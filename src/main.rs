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

use agent::agent::{system_prompt_for_depth, AgentTask, AgentTaskConfig, UserAction};
use agent::app::{App, ModelPickerState};
use agent::autocomplete;
use agent::config;
use agent::keys;
use agent::mcp::McpRegistry;
use agent::memory;
use agent::ollama::OllamaClient;
use agent::script::{self, ScriptCommand, StepReport, StepStatus, TestReport};
use agent::session::{ensure_gitignore, Session};
use agent::tools::built_in_tool_definitions;
use agent::types::{AgentEvent, ChatMessage, MessageKind};
use agent::ui;

#[derive(Parser)]
#[command(name = "agent", about = "Local AI agent TUI powered by Ollama")]
struct Cli {
    #[arg(long, default_value = "gemma4:26b", hide = true)]
    model: String,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_url: String,
    #[arg(long)]
    script: Option<std::path::PathBuf>,
    #[arg(long, default_value = "false")]
    headless: bool,
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
    // Clear old log on startup
    let _ = std::fs::remove_file(log_dir.join("agent.log"));
    let _ = std::fs::remove_file(log_dir.join("test_report.json"));
    let _ = std::fs::remove_dir_all(log_dir.join("subtasks"));
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

    let working_dir = std::env::current_dir().context("failed to get current directory")?;
    let http = reqwest::Client::new();

    // Verify Ollama is reachable
    let ollama = OllamaClient::new(&cli.ollama_url, &cli.model, http.clone());
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
    tracing::info!(model = %cli.model, ollama_url = %cli.ollama_url, "agent starting");

    // Load MCP config
    let mcp_registry = match config::load_config(&working_dir).map_err(anyhow::Error::from)? {
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
    let sys_prompt = system_prompt_for_depth(0, &working_dir, &memory_index);
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

    // Capture display values before moving into agent task
    let mcp_connected = mcp_registry.connected_servers().await;
    let mcp_failed = mcp_registry.failed_servers().await;
    let context_window = Some(agent::ollama::NUM_CTX);
    let session_messages: Vec<ChatMessage> = session
        .messages
        .iter()
        .map(|sm| sm.clone().into())
        .collect();

    // Channels
    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(256);
    let (action_tx, action_rx) = mpsc::channel::<UserAction>(16);

    // Spawn agent task
    let agent_task = AgentTask::new(AgentTaskConfig {
        ollama: Arc::new(ollama),
        mcp: Arc::new(mcp_registry),
        working_dir: working_dir.clone(),
        all_tools,
        event_tx,
        action_rx,
        session,
        system_prompt: sys_prompt,
    });
    tokio::spawn(async move { agent_task.run().await });

    // --- Script: headless mode ---
    if let Some(script_path) = &cli.script {
        if cli.headless {
            let commands = script::parse_script(script_path)?;
            let mut report = TestReport::new(&cli.model);

            for cmd in &commands {
                match cmd {
                    ScriptCommand::Send(text) => {
                        let step_start = std::time::Instant::now();

                        action_tx
                            .send(UserAction::SendMessage(text.clone(), vec![]))
                            .await
                            .ok();
                        println!("[send] {text}");

                        let result = tokio::time::timeout(
                            Duration::from_secs(240),
                            async {
                                let mut log: Vec<String> = Vec::new();
                                let mut current_depth: usize = 0;
                                let mut orch_prompt_max: u64 = 0;
                                let mut subtask_prompt_max: u64 = 0;
                                let mut total_eval: u64 = 0;
                                loop {
                                    match event_rx.recv().await {
                                        Some(AgentEvent::ThinkingStarted) => {
                                            println!("[event] ThinkingStarted");
                                            log.push("ThinkingStarted".into());
                                        }
                                        Some(AgentEvent::ThinkingDelta(d)) => {
                                            print!("[think] {d}");
                                            log.push(format!("ThinkingDelta: {d}"));
                                        }
                                        Some(AgentEvent::ThinkingDone) => {
                                            println!("\n[event] ThinkingDone");
                                            log.push("ThinkingDone".into());
                                        }
                                        Some(AgentEvent::TextDelta(d)) => {
                                            print!("{d}");
                                            log.push(format!("TextDelta: {d}"));
                                        }
                                        Some(AgentEvent::ToolRequested(c)) => {
                                            println!("\n[tool] {}({})", c.name, c.arguments);
                                            log.push(format!("ToolRequested: {}({})", c.name, c.arguments));
                                        }
                                        Some(AgentEvent::ToolCompleted(r)) => {
                                            println!("[tool_done] {}", r.call_id);
                                            log.push(format!("ToolCompleted: {}", r.call_id));
                                        }
                                        Some(AgentEvent::TurnStats { eval_count, eval_duration_ns, prompt_eval_count }) => {
                                            log.push(format!("TurnStats: depth={current_depth} eval={eval_count} prompt={prompt_eval_count} ns={eval_duration_ns}"));
                                            total_eval += eval_count;
                                            if current_depth == 0 {
                                                orch_prompt_max = orch_prompt_max.max(prompt_eval_count);
                                            } else {
                                                subtask_prompt_max = subtask_prompt_max.max(prompt_eval_count);
                                            }
                                        }
                                        Some(AgentEvent::TurnDone) => {
                                            println!("\n[done]");
                                            log.push("TurnDone".into());
                                            return (StepStatus::Completed, log, orch_prompt_max, subtask_prompt_max, total_eval);
                                        }
                                        Some(AgentEvent::Error(e)) => {
                                            println!("\n[error] {e}");
                                            log.push(format!("Error: {e}"));
                                            return (StepStatus::Failed(e), log, orch_prompt_max, subtask_prompt_max, total_eval);
                                        }
                                        Some(AgentEvent::LoopDetected) => {
                                            let msg = "Loop detected";
                                            println!("\n[error] {msg}");
                                            log.push(format!("Error: {msg}"));
                                            return (StepStatus::Failed(msg.into()), log, orch_prompt_max, subtask_prompt_max, total_eval);
                                        }
                                        Some(AgentEvent::SubtaskEnter { depth, label }) => {
                                            println!("[subtask_enter] depth={depth} {label}");
                                            log.push(format!("SubtaskEnter: depth={depth} {label}"));
                                            current_depth = depth;
                                        }
                                        Some(AgentEvent::SubtaskExit { depth }) => {
                                            println!("[subtask_exit] depth={depth}");
                                            log.push(format!("SubtaskExit: depth={depth}"));
                                            current_depth = depth.saturating_sub(1);
                                        }
                                        None => {
                                            return (StepStatus::Failed("channel closed".into()), log, orch_prompt_max, subtask_prompt_max, total_eval);
                                        }
                                    }
                                }
                            },
                        )
                        .await;

                        let (status, events_log, orch_prompt_max, subtask_prompt_max, total_eval) =
                            match result {
                                Ok(tuple) => tuple,
                                Err(_) => {
                                    println!("[timeout] step exceeded 240s");
                                    (StepStatus::TimedOut, vec![], 0, 0, 0)
                                }
                            };

                        report.add_step(StepReport {
                            command: text.clone(),
                            events: events_log,
                            duration_ms: step_start.elapsed().as_millis() as u64,
                            status,
                            token_summary: script::TokenSummary {
                                orchestrator_prompt_max: orch_prompt_max,
                                subtask_prompt_max,
                                total_eval,
                            },
                        });
                    }
                    ScriptCommand::Wait(dur) => {
                        println!("[wait] {}ms", dur.as_millis());
                        tokio::time::sleep(*dur).await;
                    }
                    ScriptCommand::ExpectFile { .. } | ScriptCommand::ExpectNoFile(_) => {
                        if let Some(result) = script::run_assertion(cmd, &working_dir) {
                            let pass_str = if result.pass { "PASS" } else { "FAIL" };
                            println!("[assert] {pass_str} {} {}", result.assert_type, result.path);
                            report.add_assertion(result);
                        }
                    }
                    ScriptCommand::ExpectEvent(event_name) => {
                        let Some(last_step) = report.steps.last() else {
                            println!("[assert] FAIL expect_event {event_name} — no Send step preceding this assertion");
                            report.add_assertion(script::AssertionResult {
                                assert_type: "expect_event".into(),
                                path: event_name.clone(),
                                expected: event_name.clone(),
                                actual: "no prior step".into(),
                                pass: false,
                            });
                            continue;
                        };
                        let pass = last_step
                            .events
                            .iter()
                            .any(|e| e.contains(event_name.as_str()));
                        let pass_str = if pass { "PASS" } else { "FAIL" };
                        println!("[assert] {pass_str} expect_event {event_name}");
                        report.add_assertion(script::AssertionResult {
                            assert_type: "expect_event".into(),
                            path: event_name.clone(),
                            expected: event_name.clone(),
                            actual: if pass {
                                event_name.clone()
                            } else {
                                String::new()
                            },
                            pass,
                        });
                    }
                    ScriptCommand::ExpectStat { lhs, op, rhs } => {
                        let Some(last_step) = report.steps.last() else {
                            println!(
                                "[assert] FAIL expect_stat — no Send step preceding this assertion"
                            );
                            report.add_assertion(script::AssertionResult {
                                assert_type: "expect_stat".into(),
                                path: format!("{lhs} {op} {rhs}"),
                                expected: format!("{lhs} {op} {rhs}"),
                                actual: "no prior step".into(),
                                pass: false,
                            });
                            continue;
                        };
                        let ts = &last_step.token_summary;
                        let lhs_val = ts.resolve_field(lhs);
                        let rhs_val = ts.resolve_field(rhs);
                        let (pass, actual_str) = match (lhs_val, rhs_val) {
                            (Some(l), Some(r)) => {
                                let result = match op.as_str() {
                                    "<" => l < r,
                                    ">" => l > r,
                                    "==" => l == r,
                                    _ => false,
                                };
                                (result, format!("{l} {op} {r}"))
                            }
                            _ => (false, format!("unknown field(s): {lhs}, {rhs}")),
                        };
                        let pass_str = if pass { "PASS" } else { "FAIL" };
                        println!("[assert] {pass_str} expect_stat {lhs} {op} {rhs} ({actual_str})");
                        report.add_assertion(script::AssertionResult {
                            assert_type: "expect_stat".into(),
                            path: format!("{lhs} {op} {rhs}"),
                            expected: format!("{lhs} {op} {rhs}"),
                            actual: actual_str,
                            pass,
                        });
                    }
                }
            }

            action_tx.send(UserAction::Quit).await.ok();

            let report_path = log_dir.join("test_report.json");
            report.write_to_file(&report_path)?;
            println!("[report] written to {}", report_path.display());
            report.print_summary();

            // Clean up subtask context dumps — they've been summarised in the report
            let _ = std::fs::remove_dir_all(log_dir.join("subtasks"));

            return Ok(());
        }
    }

    // --- Build App (TUI modes) ---
    let mut app = App::new(cli.model.clone(), working_dir.clone());
    app.context_window_size = context_window;
    app.mcp_connected = mcp_connected;
    app.mcp_failed = mcp_failed;
    app.resumed_session = resumed;
    app.available_models = models;

    for chat_msg in session_messages {
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

    let mut reader = EventStream::new();
    let mut tick = interval(Duration::from_millis(50));

    // Script TUI mode: feed commands via channel, with turn-done sync
    let (script_tx, mut script_rx) = mpsc::channel::<String>(8);
    let script_turn_done = Arc::new(tokio::sync::Notify::new());
    let script_turn_done_writer = script_turn_done.clone();
    if let Some(script_path) = cli.script.clone() {
        let action_tx_clone = action_tx.clone();
        let turn_done = script_turn_done.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let commands = match script::parse_script(&script_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("script parse error: {e}");
                    return;
                }
            };
            for cmd in commands {
                match cmd {
                    ScriptCommand::Send(text) => {
                        script_tx.send(text).await.ok();
                        // Wait for turn to complete before sending next
                        turn_done.notified().await;
                    }
                    ScriptCommand::Wait(dur) => {
                        tokio::time::sleep(dur).await;
                    }
                    ScriptCommand::ExpectFile { .. }
                    | ScriptCommand::ExpectNoFile(_)
                    | ScriptCommand::ExpectEvent(_)
                    | ScriptCommand::ExpectStat { .. } => {
                        tracing::info!("script assertion skipped in TUI mode");
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            action_tx_clone.send(UserAction::Quit).await.ok();
        });
    }

    // Event loop
    let mut current_depth: usize = 0;
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
                if !handle_agent_event(agent_event, &mut app, &action_tx, &mut current_depth).await {
                    break;
                }
                // Notify script task when turn ends
                if !app.streaming {
                    script_turn_done_writer.notify_one();
                }
            }
            script_msg = script_rx.recv() => {
                if let Some(text) = script_msg {
                    app.add_user_message(text.clone());
                    app.start_assistant_turn();
                    action_tx.send(UserAction::SendMessage(text, vec![])).await.ok();
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
            apply_command(keys::UiCommand::Paste(data), app, action_tx).await;
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

    // Model picker active: intercept keys
    if app.model_picker.is_some() {
        match cmd {
            UiCommand::ScrollUp | UiCommand::HistoryPrev => {
                app.model_picker.as_mut().unwrap().move_up();
                return;
            }
            UiCommand::ScrollDown | UiCommand::HistoryNext => {
                app.model_picker.as_mut().unwrap().move_down();
                return;
            }
            UiCommand::Submit | UiCommand::Tab => {
                let model = app
                    .model_picker
                    .as_ref()
                    .unwrap()
                    .selected()
                    .map(str::to_string);
                app.model_picker = None;
                if let Some(model) = model {
                    app.model_name = model.clone();
                    action_tx.send(UserAction::ChangeModel(model)).await.ok();
                }
                return;
            }
            UiCommand::Quit => {} // fall through
            _ => {
                app.model_picker = None;
                return;
            }
        }
    }

    // Autocomplete active: intercept keys
    if let Some(ac) = &mut app.autocomplete {
        match cmd {
            UiCommand::Tab | UiCommand::Submit => {
                if let Some(name) = ac.selected_command() {
                    app.input.text = name.to_string();
                    app.input.cursor_pos = app.input.text.len();
                }
                app.autocomplete = None;
                if matches!(cmd, UiCommand::Submit) {
                    let text = app.input.take();
                    handle_slash_or_send(text, app, action_tx).await;
                }
                return;
            }
            UiCommand::HistoryPrev | UiCommand::ScrollUp => {
                ac.move_up();
                return;
            }
            UiCommand::HistoryNext | UiCommand::ScrollDown => {
                ac.move_down();
                return;
            }
            UiCommand::Cancel => {
                app.autocomplete = None;
                return;
            }
            UiCommand::InsertChar(c) => {
                app.input.push_char(c);
                ac.filter(&app.input.text);
                if ac.is_empty() {
                    app.autocomplete = None;
                }
                return;
            }
            UiCommand::Backspace => {
                app.input.pop_char();
                if app.input.text.is_empty() || !app.input.text.starts_with('/') {
                    app.autocomplete = None;
                } else {
                    ac.filter(&app.input.text);
                    if ac.is_empty() {
                        app.autocomplete = None;
                    }
                }
                return;
            }
            UiCommand::Quit => {} // fall through
            _ => {
                app.autocomplete = None;
            }
        }
    }

    // Trigger autocomplete on '/' as first char
    if let UiCommand::InsertChar('/') = &cmd {
        if app.input.text.is_empty() {
            app.input.push_char('/');
            app.autocomplete = Some(autocomplete::Autocomplete::open());
            return;
        }
    }

    match cmd {
        UiCommand::Quit => {
            if let Err(e) = action_tx.send(UserAction::Quit).await {
                tracing::error!("failed to send Quit action: {e}");
            }
            app.running = false;
        }
        UiCommand::Cancel => {
            if app.streaming {
                if let Err(e) = action_tx.send(UserAction::Cancel).await {
                    tracing::error!("failed to send Cancel action: {e}");
                }
            }
        }
        UiCommand::Submit => {
            if !app.input.is_empty() {
                if app.streaming {
                    let text = app.input.take();
                    let images = app.take_pending_images();
                    app.enqueue_message(text, images);
                } else {
                    let text = app.input.take();
                    handle_slash_or_send(text, app, action_tx).await;
                }
            }
        }
        UiCommand::InsertNewline => app.input.push_char('\n'),
        UiCommand::InsertChar(c) => app.input.push_char(c),
        UiCommand::Tab => {}
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
            match result {
                Ok(Ok(data)) => {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                    app.add_pending_image(b64);
                }
                Ok(Err(e)) => app.set_error(format!("clipboard error: {e}")),
                Err(e) => app.set_error(format!("paste task error: {e}")),
            }
        }
        UiCommand::Ignore => {}
    }
}

async fn handle_slash_or_send(text: String, app: &mut App, action_tx: &mpsc::Sender<UserAction>) {
    if text.trim() == "/model" {
        if !app.available_models.is_empty() {
            let sel = app
                .available_models
                .iter()
                .position(|m| m == &app.model_name)
                .unwrap_or(0);
            app.model_picker = Some(ModelPickerState {
                models: app.available_models.clone(),
                selected: sel,
            });
        }
    } else if text.trim() == "/clear" || text.trim() == "/new" {
        app.clear_messages();
        if let Err(e) = action_tx.send(UserAction::ClearHistory).await {
            tracing::error!("failed to send ClearHistory action: {e}");
        }
    } else if text.trim() == "/help" {
        app.messages.push(ChatMessage {
            role: agent::types::Role::Assistant,
            content: App::help_text().to_string(),
            kind: MessageKind::Text,
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

async fn handle_agent_event(
    event: Option<AgentEvent>,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
    current_depth: &mut usize,
) -> bool {
    match event {
        Some(AgentEvent::ThinkingStarted) => app.set_thinking(true),
        Some(AgentEvent::ThinkingDelta(d)) => app.append_thinking_text(&d),
        Some(AgentEvent::ThinkingDone) => app.flush_thinking(),
        Some(AgentEvent::TextDelta(d)) => app.append_streaming_text(&d),
        Some(AgentEvent::ToolRequested(c)) => app.add_tool_call(&c),
        Some(AgentEvent::ToolCompleted(r)) => app.add_tool_result(&r),
        Some(AgentEvent::TurnStats {
            eval_count,
            eval_duration_ns,
            prompt_eval_count,
        }) => {
            app.update_turn_stats(
                eval_count,
                eval_duration_ns,
                prompt_eval_count,
                *current_depth,
            );
        }
        Some(AgentEvent::TurnDone) => {
            app.finish_assistant_turn();
            if let Some((text, images)) = app.dequeue_message() {
                app.start_assistant_turn();
                if let Err(e) = action_tx.send(UserAction::SendMessage(text, images)).await {
                    tracing::error!("failed to send queued message: {e}");
                }
            }
        }
        Some(AgentEvent::Error(e)) => {
            app.fail_active_node();
            app.finish_assistant_turn();
            app.message_queue.clear();
            app.set_error(e);
        }
        Some(AgentEvent::LoopDetected) => {
            app.fail_active_node();
            app.finish_assistant_turn();
            app.message_queue.clear();
            app.set_error("Loop detected — model was repeating itself".into());
        }
        Some(AgentEvent::SubtaskEnter { depth, label }) => {
            *current_depth = depth;
            app.enter_subtask(depth, label);
        }
        Some(AgentEvent::SubtaskExit { depth }) => {
            *current_depth = depth.saturating_sub(1);
            app.exit_subtask(depth);
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
