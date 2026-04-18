use crate::types::{ChatMessage, MessageKind, PlanItem, Role};

use super::App;

impl App {
    pub fn apply_plan_update(&mut self, items: Vec<PlanItem>) {
        self.plan = items.clone();
        self.messages.push(ChatMessage {
            role: Role::Assistant,
            content: format!("plan updated: {} items", items.len()),
            kind: MessageKind::PlanUpdate { items },
        });
        if self.auto_scroll {
            self.scroll_offset = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::types::{MessageKind, PlanItem, PlanStatus};

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn apply_plan_update_stores_plan_and_appends_message() {
        let mut app = make_app();
        let items = vec![
            PlanItem {
                content: "step 1".into(),
                status: PlanStatus::Pending,
            },
            PlanItem {
                content: "step 2".into(),
                status: PlanStatus::InProgress,
            },
        ];
        app.apply_plan_update(items.clone());
        assert_eq!(app.plan, items);
        assert_eq!(app.messages.len(), 1);
        assert!(matches!(
            app.messages[0].kind,
            MessageKind::PlanUpdate { .. }
        ));
        if let MessageKind::PlanUpdate { items: stored } = &app.messages[0].kind {
            assert_eq!(stored.len(), 2);
        }
    }

    #[test]
    fn clear_messages_clears_plan() {
        let mut app = make_app();
        app.apply_plan_update(vec![PlanItem {
            content: "task".into(),
            status: PlanStatus::Pending,
        }]);
        app.clear_messages();
        assert!(app.plan.is_empty());
        assert!(app.messages.is_empty());
    }

    #[test]
    fn apply_plan_update_replaces_previous_plan() {
        let mut app = make_app();
        app.apply_plan_update(vec![PlanItem {
            content: "old".into(),
            status: PlanStatus::Pending,
        }]);
        app.apply_plan_update(vec![
            PlanItem {
                content: "new 1".into(),
                status: PlanStatus::InProgress,
            },
            PlanItem {
                content: "new 2".into(),
                status: PlanStatus::Pending,
            },
        ]);
        assert_eq!(app.plan.len(), 2);
        assert_eq!(app.plan[0].content, "new 1");
        assert_eq!(app.messages.len(), 2);
    }
}
