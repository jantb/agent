use crossterm::event::{Event, MouseEventKind};
use tokio::sync::mpsc;

use crate::agent::UserAction;
use crate::app::{App, ModelPickerState};
use crate::autocomplete;
use crate::keys;
use crate::types::{ChatMessage, MessageKind, Role};

pub async fn handle_terminal_event(
    event: Option<Result<Event, std::io::Error>>,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
) {
    match event {
        Some(Ok(Event::Key(key))) => {
            let picker_active = app.interview_picker.is_some() || app.model_picker.is_some();
            let cmd = keys::map_key(key, app.streaming && !picker_active);
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

pub async fn apply_command(
    cmd: keys::UiCommand,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
) {
    use keys::UiCommand;

    // Interview picker active: intercept keys
    if app.interview_picker.is_some() {
        match cmd {
            UiCommand::ScrollUp | UiCommand::HistoryPrev => {
                app.interview_picker.as_mut().unwrap().move_up();
                return;
            }
            UiCommand::ScrollDown | UiCommand::HistoryNext => {
                app.interview_picker.as_mut().unwrap().move_down();
                return;
            }
            UiCommand::Tab => {
                let picker = app.interview_picker.as_mut().unwrap();
                picker.custom_mode = !picker.custom_mode;
                return;
            }
            UiCommand::InsertChar(c) => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.push_char(c);
                }
                return;
            }
            UiCommand::Backspace => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.pop_char();
                }
                return;
            }
            UiCommand::DeleteWord => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.delete_word();
                }
                return;
            }
            UiCommand::ClearLine => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.clear_line();
                }
                return;
            }
            UiCommand::MoveLeft => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.move_left();
                }
                return;
            }
            UiCommand::MoveRight => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.move_right();
                }
                return;
            }
            UiCommand::MoveToStart => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.move_to_start();
                }
                return;
            }
            UiCommand::MoveToEnd => {
                let picker = app.interview_picker.as_mut().unwrap();
                if picker.custom_mode {
                    picker.custom_input.move_to_end();
                }
                return;
            }
            UiCommand::Submit => {
                app.interview_picker.as_mut().and_then(|p| p.submit());
                app.interview_picker = None;
                return;
            }
            UiCommand::Cancel => {
                if let Some(mut picker) = app.interview_picker.take() {
                    if let Some(tx) = picker.answer_tx.take() {
                        let _ = tx.send("[DONE]".into());
                    }
                }
                return;
            }
            UiCommand::Quit => {} // fall through
            _ => {
                return;
            }
        }
    }

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
            UiCommand::InsertChar(c) => {
                app.model_picker.as_mut().unwrap().push_filter(c);
                return;
            }
            UiCommand::Backspace => {
                app.model_picker.as_mut().unwrap().pop_filter();
                return;
            }
            UiCommand::Cancel => {
                app.model_picker = None;
                return;
            }
            UiCommand::Quit => {} // fall through
            _ => {
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
            } else if !app.input.is_empty() {
                app.input.clear_line();
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
        UiCommand::CycleMode => {
            app.mode = app.mode.cycle();
            app.messages.push(ChatMessage {
                role: Role::Assistant,
                content: format!("Mode: {}", app.mode.label()),
                kind: MessageKind::Text,
                rendered: std::cell::RefCell::new(None),
            });
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

pub async fn handle_slash_or_send(
    text: String,
    app: &mut App,
    action_tx: &mpsc::Sender<UserAction>,
) {
    if text.trim() == "/model" {
        if !app.available_models.is_empty() {
            let sel = app
                .available_models
                .iter()
                .position(|m| m == &app.model_name)
                .unwrap_or(0);
            app.model_picker = Some(ModelPickerState::new(app.available_models.clone(), sel));
        }
    } else if text.trim() == "/clear" || text.trim() == "/new" {
        app.clear_messages();
        if let Err(e) = action_tx.send(UserAction::ClearHistory).await {
            tracing::error!("failed to send ClearHistory action: {e}");
        }
    } else if text.trim() == "/help" {
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            content: App::help_text().to_string(),
            kind: MessageKind::Text,
            rendered: std::cell::RefCell::new(None),
        });
    } else if let Some(arg) = text
        .trim()
        .strip_prefix("/mode")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
        .map(str::trim)
    {
        use crate::types::AgentMode;
        let parsed = match arg {
            "" => Some(app.mode.cycle()),
            "plan" => Some(AgentMode::Plan),
            "thorough" => Some(AgentMode::Thorough),
            "oneshot" => Some(AgentMode::Oneshot),
            _ => None,
        };
        match parsed {
            Some(m) => {
                app.mode = m;
                app.messages.push(ChatMessage {
                    role: Role::Assistant,
                    content: format!("Mode: {}", app.mode.label()),
                    kind: MessageKind::Text,
                    rendered: std::cell::RefCell::new(None),
                });
            }
            None => app.set_error(format!(
                "unknown mode '{arg}' — expected plan, thorough, or oneshot"
            )),
        }
    } else if let Some(arg) = text
        .trim()
        .strip_prefix("/memory")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
        .map(str::trim)
    {
        let working_dir = app.working_dir.clone();
        let result = if arg.is_empty() {
            crate::memory::list_memories(&working_dir)
        } else {
            crate::memory::recall_memories(&working_dir, arg)
        };
        match result {
            Ok(body) => app.messages.push(ChatMessage {
                role: Role::Assistant,
                content: body,
                kind: MessageKind::Text,
                rendered: std::cell::RefCell::new(None),
            }),
            Err(e) => app.set_error(format!("memory: {e}")),
        }
    } else if text.trim() == "/mcp" {
        let mut lines = Vec::new();
        if app.mcp_connected.is_empty() && app.mcp_failed.is_empty() {
            lines.push("No MCP servers configured.".to_string());
        } else {
            for name in &app.mcp_connected {
                lines.push(format!("● {name}: connected"));
            }
            for (name, reason) in &app.mcp_failed {
                lines.push(format!("✗ {name}: {reason}"));
            }
        }
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            content: lines.join("\n"),
            kind: MessageKind::Text,
            rendered: std::cell::RefCell::new(None),
        });
    } else if text.trim() == "/show last" || text.trim() == "/show" {
        match app
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.kind, MessageKind::ToolResult { .. }))
            .map(|m| m.content.clone())
        {
            Some(body) => app.messages.push(ChatMessage {
                role: Role::Assistant,
                content: format!("```\n{body}\n```"),
                kind: MessageKind::Text,
                rendered: std::cell::RefCell::new(None),
            }),
            None => app.set_error("no tool result to show".into()),
        }
    } else if text.trim() == "/flat" {
        app.flat = !app.flat;
        let label = if app.flat {
            "flat (single-level)"
        } else {
            "hierarchical (multi-level)"
        };
        app.messages.push(ChatMessage {
            role: Role::Assistant,
            content: format!("Mode: {label}"),
            kind: MessageKind::Text,
            rendered: std::cell::RefCell::new(None),
        });
        if let Err(e) = action_tx.send(UserAction::ToggleFlat(app.flat)).await {
            tracing::error!("failed to send ToggleFlat action: {e}");
        }
    } else if let Some(rest) = text
        .trim()
        .strip_prefix("/review")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    {
        let scope = rest.trim_start();
        let body = format!("{}{}", crate::prompts::REVIEW_SKILL_PREAMBLE, scope);
        let images = app.take_pending_images();
        app.add_user_message(text.clone());
        app.start_assistant_turn();
        if let Err(e) = action_tx
            .send(UserAction::SendMessage {
                text: body,
                images,
                mode: app.mode,
            })
            .await
        {
            tracing::error!("failed to send SendMessage action: {e}");
        }
    } else if let Some(rest) = text
        .trim()
        .strip_prefix("/simplify")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    {
        let scope = rest.trim_start();
        let body = format!("{}{}", crate::prompts::SIMPLIFY_SKILL_PREAMBLE, scope);
        let images = app.take_pending_images();
        app.add_user_message(text.clone());
        app.start_assistant_turn();
        if let Err(e) = action_tx
            .send(UserAction::SendMessage {
                text: body,
                images,
                mode: app.mode,
            })
            .await
        {
            tracing::error!("failed to send SendMessage action: {e}");
        }
    } else {
        let images = app.take_pending_images();
        app.add_user_message(text.clone());
        app.start_assistant_turn();
        if let Err(e) = action_tx
            .send(UserAction::SendMessage {
                text,
                images,
                mode: app.mode,
            })
            .await
        {
            tracing::error!("failed to send SendMessage action: {e}");
        }
    }
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
