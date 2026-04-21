use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::mpsc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::agent::{is_flat_model, AgentTask, AgentTaskConfig, UserAction};
use crate::app::App;
use crate::config;
use crate::mcp::McpRegistry;
use crate::memory;
use crate::ollama::OllamaClient;
use crate::prompts::{mcp_tools_prompt_section, system_prompt_for_depth};
use crate::session::{ensure_gitignore, Session};
use crate::tools::built_in_tool_definitions;
use crate::types::{AgentEvent, AgentMode, ChatMessage, ToolSource};

/// Everything produced by [`setup`] that `tui` and `headless` need.
pub struct Setup {
    pub app: App,
    pub event_rx: mpsc::Receiver<AgentEvent>,
    pub action_tx: mpsc::Sender<UserAction>,
    pub log_dir: PathBuf,
    pub working_dir: PathBuf,
    /// Held for process lifetime — dropping this stops the non-blocking log writer.
    pub _log_guard: tracing_appender::non_blocking::WorkerGuard,
}

pub struct SetupConfig {
    pub model: String,
    pub ollama_url: String,
}

/// Initialise logging, verify Ollama, build MCP registry, load session, spawn AgentTask.
pub async fn setup(cfg: SetupConfig) -> anyhow::Result<Setup> {
    let working_dir = std::env::current_dir().context("failed to get current directory")?;

    // ── Logging ──────────────────────────────────────────────────────────────
    let log_dir = working_dir.join(".agent");
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("failed to create log dir: {e}");
    }
    let _ = std::fs::remove_file(log_dir.join("agent.log"));
    let _ = std::fs::remove_file(log_dir.join("test_report.json"));
    let _ = std::fs::remove_dir_all(log_dir.join("subtasks"));

    let file_appender = tracing_appender::rolling::never(&log_dir, "agent.log");
    let (non_blocking, log_guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false),
        )
        .init();

    // ── Ollama ────────────────────────────────────────────────────────────────
    let http = reqwest::Client::new();
    let ollama = OllamaClient::new(&cfg.ollama_url, &cfg.model, http.clone());
    let models = ollama
        .list_models()
        .await
        .context("cannot connect to Ollama — is `ollama serve` running?")?;
    if !models.iter().any(|m| m == &cfg.model) {
        eprintln!(
            "warning: model '{}' not found in Ollama. Available: {}",
            cfg.model,
            models.join(", ")
        );
    }
    tracing::info!(model = %cfg.model, ollama_url = %cfg.ollama_url, "agent starting");

    // ── MCP ───────────────────────────────────────────────────────────────────
    let mcp_registry = match config::load_config(&working_dir) {
        Ok(Some(c)) => McpRegistry::from_config(&c, &http).await,
        Ok(None) => McpRegistry::empty(),
        Err(e) => {
            tracing::warn!(".mcp.json ignored: {e}");
            McpRegistry::empty()
        }
    };
    tracing::info!(connected = ?mcp_registry.connected_servers().await, "MCP registry ready");

    // ── Tools ─────────────────────────────────────────────────────────────────
    let mut all_tools = built_in_tool_definitions();
    for t in mcp_registry.all_tools().await {
        all_tools.push(t);
    }

    // ── Session ───────────────────────────────────────────────────────────────
    let memory_index = memory::build_memory_index(&working_dir);
    let flat = is_flat_model(&cfg.model);
    let mcp_only: Vec<_> = all_tools
        .iter()
        .filter(|t| t.source == ToolSource::Mcp)
        .cloned()
        .collect();
    let mcp_tools_context = mcp_tools_prompt_section(&mcp_only);
    let sys_prompt =
        system_prompt_for_depth(0, &working_dir, &memory_index, &mcp_tools_context, flat);

    let (session, resumed) = match Session::load(&working_dir)? {
        Some(s) => {
            let date = s.updated_at.format("%Y-%m-%d %H:%M").to_string();
            let msg_count = s.messages.len();
            let last_user = s.messages.iter().rev().find_map(|m| match m {
                crate::session::SessionMessage::Text {
                    role: crate::types::Role::User,
                    content,
                    ..
                } => Some(content.clone()),
                _ => None,
            });
            let banner = match last_user {
                Some(topic) => {
                    let first_line = topic.lines().next().unwrap_or("").trim();
                    let preview: String = first_line.chars().take(60).collect();
                    let ellipsis = if first_line.chars().count() > 60 {
                        "…"
                    } else {
                        ""
                    };
                    format!("{date} — {msg_count} msgs · last: {preview}{ellipsis}")
                }
                None => format!("{date} — {msg_count} msgs"),
            };
            (s, Some(banner))
        }
        None => {
            let s = Session::new(&cfg.model, &working_dir);
            (s, None)
        }
    };
    ensure_gitignore(&working_dir)?;

    // ── Capture display values before move ───────────────────────────────────
    let mcp_connected = mcp_registry.connected_servers().await;
    let mcp_failed = mcp_registry.failed_servers().await;
    let context_window = Some(crate::ollama::NUM_CTX);
    let session_messages: Vec<ChatMessage> = session
        .messages
        .iter()
        .map(|sm| sm.clone().into())
        .collect();
    let session_plan = session.plan.clone();

    // ── Channels & AgentTask ─────────────────────────────────────────────────
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(256);
    let (action_tx, action_rx) = mpsc::channel::<UserAction>(16);

    let agent_task = AgentTask::new(AgentTaskConfig {
        ollama: Arc::new(ollama),
        mcp: Arc::new(mcp_registry),
        working_dir: working_dir.clone(),
        all_tools,
        event_tx,
        action_rx,
        session,
        system_prompt: sys_prompt,
        flat,
        mode: AgentMode::default(),
        mcp_tools_context,
    });
    tokio::spawn(async move { agent_task.run().await });

    // ── Build App ─────────────────────────────────────────────────────────────
    let mut app = App::new(cfg.model.clone(), working_dir.clone());
    app.flat = flat;
    app.context_window_size = context_window;
    app.mcp_connected = mcp_connected;
    app.mcp_failed = mcp_failed;
    app.resumed_session = resumed;
    app.available_models = models;

    for chat_msg in session_messages {
        app.messages.push(chat_msg);
    }
    app.plan = session_plan;

    Ok(Setup {
        app,
        event_rx,
        action_tx,
        log_dir,
        working_dir,
        _log_guard: log_guard,
    })
}
