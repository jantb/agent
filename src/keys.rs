use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, PartialEq, Eq)]
pub enum UiCommand {
    Quit,
    Cancel,
    Submit,
    InsertNewline,
    InsertChar(char),
    Backspace,
    DeleteWord,
    ClearLine,
    MoveLeft,
    MoveRight,
    MoveToStart,
    MoveToEnd,
    HistoryPrev,
    HistoryNext,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    ScrollToBottom,
    ClearHistory,
    Tab,
    PasteImage,
    Paste(String),
    Ignore,
}

pub fn map_key(key: KeyEvent, streaming: bool) -> UiCommand {
    match (key.code, key.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => UiCommand::Quit,
        (KeyCode::Esc, _) => UiCommand::Cancel,
        (KeyCode::Char('w'), KeyModifiers::CONTROL) => UiCommand::DeleteWord,
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => UiCommand::ClearLine,
        (KeyCode::Char('a'), KeyModifiers::CONTROL) => UiCommand::MoveToStart,
        (KeyCode::Char('e'), KeyModifiers::CONTROL) => UiCommand::MoveToEnd,
        (KeyCode::Char('l'), KeyModifiers::CONTROL) => UiCommand::ClearHistory,
        (KeyCode::Char('v'), KeyModifiers::CONTROL) => UiCommand::PasteImage,
        (KeyCode::Enter, KeyModifiers::SHIFT) => UiCommand::InsertNewline,
        (KeyCode::Enter, KeyModifiers::NONE) => UiCommand::Submit,
        (KeyCode::Backspace, _) => UiCommand::Backspace,
        (KeyCode::Left, _) => UiCommand::MoveLeft,
        (KeyCode::Right, _) => UiCommand::MoveRight,
        (KeyCode::Up, KeyModifiers::SHIFT) => UiCommand::ScrollUp,
        (KeyCode::Down, KeyModifiers::SHIFT) => UiCommand::ScrollDown,
        (KeyCode::Up, _) if !streaming => UiCommand::HistoryPrev,
        (KeyCode::Down, _) if !streaming => UiCommand::HistoryNext,
        (KeyCode::PageUp, _) => UiCommand::PageUp,
        (KeyCode::PageDown, _) => UiCommand::PageDown,
        (KeyCode::End, _) => UiCommand::ScrollToBottom,
        (KeyCode::Tab, _) if !streaming => UiCommand::Tab,
        (KeyCode::Char(c), _) => UiCommand::InsertChar(c),
        _ => UiCommand::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn ctrl_c_always_quits() {
        assert_eq!(
            map_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            UiCommand::Quit
        );
        assert_eq!(
            map_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), true),
            UiCommand::Quit
        );
    }

    #[test]
    fn esc_cancels_only_when_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Esc, KeyModifiers::NONE), true),
            UiCommand::Cancel
        );
        assert_eq!(
            map_key(key(KeyCode::Esc, KeyModifiers::NONE), false),
            UiCommand::Cancel
        );
    }

    #[test]
    fn enter_submits_when_not_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Enter, KeyModifiers::NONE), false),
            UiCommand::Submit
        );
    }

    #[test]
    fn enter_ignored_when_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Enter, KeyModifiers::NONE), true),
            UiCommand::Submit
        );
    }

    #[test]
    fn shift_enter_inserts_newline() {
        assert_eq!(
            map_key(key(KeyCode::Enter, KeyModifiers::SHIFT), false),
            UiCommand::InsertNewline
        );
    }

    #[test]
    fn char_input_ignored_when_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Char('a'), KeyModifiers::NONE), true),
            UiCommand::InsertChar('a')
        );
    }

    #[test]
    fn char_input_when_not_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Char('x'), KeyModifiers::NONE), false),
            UiCommand::InsertChar('x')
        );
    }

    #[test]
    fn scroll_works_during_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Up, KeyModifiers::SHIFT), true),
            UiCommand::ScrollUp
        );
        assert_eq!(
            map_key(key(KeyCode::PageUp, KeyModifiers::NONE), true),
            UiCommand::PageUp
        );
    }

    #[test]
    fn ctrl_shortcuts_ignored_when_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Char('w'), KeyModifiers::CONTROL), true),
            UiCommand::DeleteWord
        );
        assert_eq!(
            map_key(key(KeyCode::Char('u'), KeyModifiers::CONTROL), true),
            UiCommand::ClearLine
        );
    }

    #[test]
    fn backspace_ignored_when_streaming() {
        assert_eq!(
            map_key(key(KeyCode::Backspace, KeyModifiers::NONE), true),
            UiCommand::Backspace
        );
    }
}
