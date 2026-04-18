pub mod commands;
pub mod events;

use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::EventStream;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

use crate::agent::UserAction;
use crate::app::App;
use crate::script::{self, ScriptCommand};
use crate::types::AgentEvent;
use crate::ui;

pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::DisableBracketedPaste,
            LeaveAlternateScreen
        );
    }
}

pub async fn run_loop(
    mut app: App,
    mut event_rx: mpsc::Receiver<AgentEvent>,
    action_tx: mpsc::Sender<UserAction>,
    script_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        EnterAlternateScreen,
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
    if let Some(path) = script_path {
        let action_tx_clone = action_tx.clone();
        let turn_done = script_turn_done.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let commands = match script::parse_script(&path) {
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
                        turn_done.notified().await;
                    }
                    ScriptCommand::Wait(dur) => {
                        tokio::time::sleep(dur).await;
                    }
                    ScriptCommand::ExpectFile { .. }
                    | ScriptCommand::ExpectNoFile(_)
                    | ScriptCommand::ExpectEvent(_)
                    | ScriptCommand::ExpectNoEvent(_)
                    | ScriptCommand::ExpectStat { .. } => {
                        tracing::info!("script assertion skipped in TUI mode");
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            action_tx_clone.send(UserAction::Quit).await.ok();
        });
    }

    let mut current_depth: usize = 0;
    loop {
        app.tick();
        terminal.draw(|f| ui::draw(f, &app))?;
        if let Ok(size) = terminal.size() {
            app.viewport_height = size.height.saturating_sub(5) as u32;
        }

        tokio::select! {
            maybe_event = reader.next() => {
                commands::handle_terminal_event(maybe_event, &mut app, &action_tx).await;
            }
            agent_event = event_rx.recv() => {
                let was_streaming = app.streaming;
                if !events::handle_agent_event(agent_event, &mut app, &action_tx, &mut current_depth).await {
                    break;
                }
                // Notify script task only on the streaming → idle transition.
                if was_streaming && !app.streaming {
                    script_turn_done_writer.notify_one();
                }
            }
            script_msg = script_rx.recv() => {
                if let Some(text) = script_msg {
                    app.add_user_message(text.clone());
                    app.start_assistant_turn();
                    action_tx.send(UserAction::SendMessage { text, images: vec![], mode: app.mode }).await.ok();
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
