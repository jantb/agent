use super::App;

impl App {
    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
        self.auto_scroll = false;
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
            if self.scroll_offset == 0 {
                self.auto_scroll = true;
            }
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.auto_scroll = true;
        self.scroll_offset = 0;
    }

    pub fn scroll_page_up(&mut self) {
        let half = (self.viewport_height / 2).max(1);
        self.scroll_offset = self.scroll_offset.saturating_add(half);
        self.auto_scroll = false;
    }

    pub fn scroll_page_down(&mut self) {
        let half = (self.viewport_height / 2).max(1);
        self.scroll_offset = self.scroll_offset.saturating_sub(half);
        if self.scroll_offset == 0 {
            self.auto_scroll = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn scroll_up_increments_disables_auto_scroll() {
        let mut app = make_app();
        app.scroll_up();
        assert_eq!(app.scroll_offset, 1);
        assert!(!app.auto_scroll);
    }

    #[test]
    fn scroll_down_decrements() {
        let mut app = make_app();
        app.scroll_offset = 5;
        app.scroll_down();
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn scroll_down_at_zero_stays_zero() {
        let mut app = make_app();
        app.scroll_down();
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn scroll_to_bottom_enables_auto_scroll() {
        let mut app = make_app();
        app.scroll_up();
        app.scroll_to_bottom();
        assert!(app.auto_scroll);
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn scroll_down_to_zero_enables_auto_scroll() {
        let mut app = make_app();
        app.scroll_offset = 1;
        app.auto_scroll = false;
        app.scroll_down();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.auto_scroll);
    }

    #[test]
    fn viewport_height_default_zero() {
        let app = make_app();
        assert_eq!(app.viewport_height, 0);
    }

    #[test]
    fn scroll_page_up_moves_half_viewport() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn scroll_page_up_disables_auto_scroll() {
        let mut app = make_app();
        app.viewport_height = 20;
        assert!(app.auto_scroll);
        app.scroll_page_up();
        assert!(!app.auto_scroll);
    }

    #[test]
    fn scroll_page_down_moves_half_viewport() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = 20;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn scroll_page_down_to_zero_enables_auto_scroll() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = 5;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.auto_scroll);
    }

    #[test]
    fn scroll_page_up_saturates_at_u32_max() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = u32::MAX - 5;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, u32::MAX);
    }

    #[test]
    fn scroll_page_down_clamps_to_zero() {
        let mut app = make_app();
        app.viewport_height = 40;
        app.scroll_offset = 3;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 0);
        assert!(app.auto_scroll);
    }

    #[test]
    fn scroll_page_up_zero_viewport_moves_by_one() {
        let mut app = make_app();
        app.viewport_height = 0;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn scroll_page_down_zero_viewport_moves_by_one() {
        let mut app = make_app();
        app.viewport_height = 0;
        app.scroll_offset = 5;
        app.auto_scroll = false;
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn scroll_page_up_odd_viewport() {
        let mut app = make_app();
        app.viewport_height = 21;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn scroll_page_up_viewport_one() {
        let mut app = make_app();
        app.viewport_height = 1;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn multiple_page_ups_accumulate() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_page_up();
        app.scroll_page_up();
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 30);
    }

    #[test]
    fn page_up_then_page_down_returns_to_original() {
        let mut app = make_app();
        app.viewport_height = 20;
        app.scroll_offset = 10;
        app.auto_scroll = false;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 20);
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 10);
    }
}
