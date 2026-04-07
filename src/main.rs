mod agent;
mod app;
mod config;
mod input;
mod keys;
mod markdown;
mod mcp;
mod memory;
mod ollama;
mod session;
mod tools;
mod types;
mod ui;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use crossterm::{
    event::{Event, EventStream, MouseEventKind},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use agent::{system_prompt, AgentTask, AgentTaskConfig, UserAction};
use app::App;
use mcp::McpRegistry;
use ollama::OllamaClient;
use session::{ensure_gitignore, Session};
use tools::built_in_tool_definitions;
use types::AgentEvent;

#[derive(Parser)]
#[command(name = "agent", about = "Local AI agent TUI powered by Ollama")]
struct Cli {
    #[arg(long, default_value = "gemma4:26b")]
    model: String,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_url: String,
    #[arg(long)]
    no_thinking: bool,
    #[arg(long, default_value = "gemma4:e4b")]
    tool_model: Option<String>,
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::DisableBracketedPaste,
            crossterm::event::DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Logging
    let log_dir = std::env::current_dir().unwrap_or_default().join(".agent");
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("failed to create log dir: {e}");
    }
    let file_appender = tracing_appender::rolling::never(&log_dir, "agent.log");
    let (non_blocking, _log_guard) = tracing_appender::non_blocking(file_appender);
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

    let thinking = !cli.no_thinking;
    let working_dir = std::env::current_dir().context("failed to get current directory")?;
    let http = reqwest::Client::new();

    // Verify Ollama is reachable
    let ollama = OllamaClient::new(
        &cli.ollama_url,
        &cli.model,
        cli.tool_model.clone(),
        thinking,
        http.clone(),
    );
    let models = ollama
        .list_models()
        .await
        .context("cannot connect to Ollama — is `ollama serve` running?")?;
    if !models.iter().any(|m| m == &cli.model) {
        eprintln!(
            "warning: model '{}' not found in Ollama. Available: {}",
            cli.model,
            models.join(", ")
        );
    }
    tracing::info!(model = %cli.model, ollama_url = %cli.ollama_url, thinking, "agent starting");

    // Load MCP config
    let mcp_registry = match config::load_config_from_cwd()? {
        Some(cfg) => McpRegistry::from_config(&cfg, &http).await,
        None => McpRegistry::empty(),
    };
    tracing::info!(connected = ?mcp_registry.connected_servers().await, "MCP registry ready");

    // Collect all tools
    let mut all_tools = built_in_tool_definitions();
    for t in mcp_registry.all_tools().await {
        all_tools.push(t);
    }

    // Load or create session
    let memory_index = memory::build_memory_index(&working_dir);
    let sys_prompt = system_prompt(&memory_index);
    let (session, resumed) = match Session::load(&working_dir)? {
        Some(s) => {
            let date = s.updated_at.format("%Y-%m-%d %H:%M").to_string();
            (s, Some(date))
        }
        None => {
            let s = Session::new(&cli.model, &working_dir);
            (s, None)
        }
    };

    ensure_gitignore(&working_dir)?;

    // Fetch context window size
    let ctx_window = match ollama.fetch_context_window().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("failed to fetch context window: {e}");
            None
        }
    };

    // Build App
    let mut app = App::new(cli.model.clone(), working_dir.clone());
    app.context_window_size = ctx_window;
    app.mcp_connected = mcp_registry.connected_servers().await;
    app.mcp_failed = mcp_registry.failed_servers().await;
    app.resumed_session = resumed;

    // Restore messages from session into app display
    for sm in &session.messages {
        let chat_msg: app::ChatMessage = sm.clone().into();
        app.messages.push(chat_msg);
    }

    // Setup terminal
    terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    )?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Channels
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
    let (action_tx, action_rx) = mpsc::channel::<UserAction>(16);

    // Spawn agent task
    let agent_task = AgentTask::new(AgentTaskConfig {
        ollama: Arc::new(ollama),
        mcp: Arc::new(mcp_registry),
        working_dir: working_dir.clone(),
        tools: all_tools,
        event_tx,
        action_rx,
        session,
        system_prompt: sys_prompt,
    });
    tokio::spawn(async move { agent_task.run().await });

    let mut reader = EventStream::new();
    let mut tick = interval(Duration::from_millis(50));

    // Event loop
    loop {
        app.tick();
        terminal.draw(|f| ui::draw(f, &app))?;
        if let Ok(size) = terminal.size() {
            app.viewport_height = size.height.saturating_sub(5) as u32;
        }

        tokio::select! {
            maybe_event = reader.next() => {
                handle_terminal_event(maybe_event, &mut app, &action_tx).await;
            }
            agent_event = event_rx.recv() => {
                if !handle_agent_event(agent_event, &mut app) {
                    break;
                }
            }
            _ = tick.tick() => {}
        }

        if !app.running {
            break;
        }
    }

    Ok(())
}

async fn handle_terminal_event(
    event: Option<Result<Event, std::io::Error>>,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
) {
    match event {
        Some(Ok(Event::Key(key))) => {
            let cmd = keys::map_key(key, app.streaming);
            apply_command(cmd, app, action_tx).await;
        }
        Some(Ok(Event::Paste(data))) => {
            if !app.streaming {
                apply_command(keys::UiCommand::Paste(data), app, action_tx).await;
            }
        }
        Some(Ok(Event::Mouse(mouse))) => match mouse.kind {
            MouseEventKind::ScrollUp => app.scroll_up(),
            MouseEventKind::ScrollDown => app.scroll_down(),
            _ => {}
        },
        None => {
            app.running = false;
        }
        _ => {}
    }
}

async fn apply_command(cmd: keys::UiCommand, app: &mut App, action_tx: &mpsc::Sender<UserAction>) {
    use keys::UiCommand;
    match cmd {
        UiCommand::Quit => {
            if let Err(e) = action_tx.send(UserAction::Quit).await {
                tracing::error!("failed to send Quit action: {e}");
            }
            app.running = false;
        }
        UiCommand::Cancel => {
            if let Err(e) = action_tx.send(UserAction::Cancel).await {
                tracing::error!("failed to send Cancel action: {e}");
            }
        }
        UiCommand::Submit => {
            if !app.input.is_empty() {
                let text = app.input.take();
                if text.trim() == "/clear" || text.trim() == "/new" {
                    app.clear_messages();
                    if let Err(e) = action_tx.send(UserAction::ClearHistory).await {
                        tracing::error!("failed to send ClearHistory action: {e}");
                    }
                } else if text.trim() == "/help" {
                    app.messages.push(app::ChatMessage {
                        role: types::Role::Assistant,
                        content: App::help_text().to_string(),
                        kind: app::MessageKind::Text,
                    });
                } else {
                    let images = app.take_pending_images();
                    app.add_user_message(text.clone());
                    app.start_assistant_turn();
                    if let Err(e) = action_tx.send(UserAction::SendMessage(text, images)).await {
                        tracing::error!("failed to send SendMessage action: {e}");
                    }
                }
            }
        }
        UiCommand::InsertNewline => app.input.push_char('\n'),
        UiCommand::InsertChar(c) => app.input.push_char(c),
        UiCommand::Paste(data) => app.input.insert_paste(data),
        UiCommand::Backspace => app.input.pop_char(),
        UiCommand::DeleteWord => app.input.delete_word(),
        UiCommand::ClearLine => app.input.clear_line(),
        UiCommand::MoveLeft => app.input.move_left(),
        UiCommand::MoveRight => app.input.move_right(),
        UiCommand::MoveToStart => app.input.move_to_start(),
        UiCommand::MoveToEnd => app.input.move_to_end(),
        UiCommand::HistoryPrev => app.input.history_prev(),
        UiCommand::HistoryNext => app.input.history_next(),
        UiCommand::ScrollUp => app.scroll_up(),
        UiCommand::ScrollDown => app.scroll_down(),
        UiCommand::PageUp => app.scroll_page_up(),
        UiCommand::PageDown => app.scroll_page_down(),
        UiCommand::ScrollToBottom => app.scroll_to_bottom(),
        UiCommand::ClearHistory => {
            app.clear_messages();
            if let Err(e) = action_tx.send(UserAction::ClearHistory).await {
                tracing::error!("failed to send ClearHistory action: {e}");
            }
        }
        UiCommand::PasteImage => {
            let result = tokio::task::spawn_blocking(|| -> Result<Vec<u8>, String> {
                let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
                let img = clipboard.get_image().map_err(|e| e.to_string())?;
                encode_image_to_png(&img)
            })
            .await;
            if let Ok(Ok(data)) = result {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                app.add_pending_image(b64);
            }
        }
        UiCommand::Ignore => {}
    }
}

fn handle_agent_event(event: Option<AgentEvent>, app: &mut App) -> bool {
    match event {
        Some(AgentEvent::ThinkingStarted) => app.set_thinking(true),
        Some(AgentEvent::ThinkingDelta(d)) => app.append_thinking_text(&d),
        Some(AgentEvent::ThinkingDone) => app.set_thinking(false),
        Some(AgentEvent::TextDelta(d)) => app.append_streaming_text(&d),
        Some(AgentEvent::ToolRequested(c)) => app.add_tool_call(&c),
        Some(AgentEvent::ToolCompleted(r)) => app.add_tool_result(&r),
        Some(AgentEvent::TurnStats {
            eval_count,
            prompt_eval_count,
        }) => {
            app.update_turn_stats(eval_count, prompt_eval_count);
        }
        Some(AgentEvent::TurnDone) => app.finish_assistant_turn(),
        Some(AgentEvent::Error(e)) => {
            app.finish_assistant_turn();
            app.set_error(e);
        }
        None => return false,
    }
    true
}

fn encode_image_to_png(img: &arboard::ImageData) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut buf, img.width as u32, img.height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&img.bytes)
            .map_err(|e| e.to_string())?;
    }
    Ok(buf)
}
