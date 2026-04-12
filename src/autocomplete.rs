pub struct SlashCommand {
    pub name: &'static str,
    pub desc: &'static str,
}

pub const COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/clear",
        desc: "Clear conversation",
    },
    SlashCommand {
        name: "/new",
        desc: "Clear conversation",
    },
    SlashCommand {
        name: "/help",
        desc: "Show help",
    },
    SlashCommand {
        name: "/model",
        desc: "Switch Ollama model",
    },
    SlashCommand {
        name: "/mode",
        desc: "Cycle mode: plan → thorough → oneshot",
    },
];

pub struct Autocomplete {
    pub filtered: Vec<usize>,
    pub selected: usize,
}

impl Autocomplete {
    pub fn open() -> Self {
        Autocomplete {
            filtered: (0..COMMANDS.len()).collect(),
            selected: 0,
        }
    }

    pub fn filter(&mut self, prefix: &str) {
        self.filtered = COMMANDS
            .iter()
            .enumerate()
            .filter(|(_, c)| c.name.starts_with(prefix))
            .map(|(i, _)| i)
            .collect();
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    pub fn selected_command(&self) -> Option<&'static str> {
        self.filtered.get(self.selected).map(|&i| COMMANDS[i].name)
    }

    pub fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_shows_all_commands() {
        let ac = Autocomplete::open();
        assert_eq!(ac.filtered.len(), COMMANDS.len());
        assert_eq!(ac.selected, 0);
    }

    #[test]
    fn filter_narrows() {
        let mut ac = Autocomplete::open();
        ac.filter("/cl");
        assert_eq!(ac.filtered.len(), 1);
        assert_eq!(ac.selected_command(), Some("/clear"));
    }

    #[test]
    fn filter_no_match_empties() {
        let mut ac = Autocomplete::open();
        ac.filter("/xyz");
        assert!(ac.is_empty());
    }

    #[test]
    fn move_down_up() {
        let mut ac = Autocomplete::open();
        ac.move_down();
        assert_eq!(ac.selected, 1);
        ac.move_up();
        assert_eq!(ac.selected, 0);
    }

    #[test]
    fn move_up_at_zero_stays() {
        let mut ac = Autocomplete::open();
        ac.move_up();
        assert_eq!(ac.selected, 0);
    }

    #[test]
    fn move_down_at_end_stays() {
        let mut ac = Autocomplete::open();
        for _ in 0..20 {
            ac.move_down();
        }
        assert_eq!(ac.selected, COMMANDS.len() - 1);
    }

    #[test]
    fn filter_clamps_selected() {
        let mut ac = Autocomplete::open();
        ac.selected = 2;
        ac.filter("/cl");
        assert_eq!(ac.selected, 0);
    }

    #[test]
    fn selected_command_when_empty() {
        let mut ac = Autocomplete::open();
        ac.filter("/xyz");
        assert_eq!(ac.selected_command(), None);
    }
}
