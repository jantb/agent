use crate::types::{ChatMessage, MessageKind, NodeInfo, NodeStatus, Role};

use super::App;

impl App {
    pub fn enter_subtask(&mut self, depth: usize, label: String) {
        for node in &mut self.tree {
            if node.status == NodeStatus::Active {
                node.status = NodeStatus::Suspended;
            }
        }
        self.tree.push(NodeInfo {
            depth,
            label: label.clone(),
            status: NodeStatus::Active,
            context_used: 0,
        });
        self.messages.push(ChatMessage {
            role: Role::Assistant,
            content: format!("depth {depth}: {label}"),
            kind: MessageKind::SubtaskEnter { depth, label },
        });
        self.subtask_tool_calls = 0;
    }

    /// Mark the currently active node (at any depth) as Failed.
    pub fn fail_active_node(&mut self) {
        if let Some(node) = self
            .tree
            .iter_mut()
            .rfind(|n| n.status == NodeStatus::Active)
        {
            node.status = NodeStatus::Failed;
        }
    }

    pub fn exit_subtask(&mut self, depth: usize) {
        if let Some(node) = self.tree.iter_mut().rfind(|n| n.depth == depth) {
            node.status = NodeStatus::Done;
        }
        if depth > 0 {
            if let Some(parent) = self
                .tree
                .iter_mut()
                .rfind(|n| n.depth == depth - 1 && n.status == NodeStatus::Suspended)
            {
                parent.status = NodeStatus::Active;
            }
        }
        self.messages.push(ChatMessage {
            role: Role::Assistant,
            content: format!("depth {depth} done"),
            kind: MessageKind::SubtaskExit { depth },
        });
        self.subtask_tool_calls = 0;
    }

    /// True when the tree has at least one subtask node (should show tree panel).
    pub fn has_tree(&self) -> bool {
        self.tree.len() > 1
    }

    pub fn clear_tree(&mut self) {
        self.tree.clear();
        self.subtask_tool_calls = 0;
    }
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::types::NodeStatus;

    fn make_app() -> App {
        App::new("test-model".into(), std::path::PathBuf::from("/tmp"))
    }

    #[test]
    fn start_assistant_turn_seeds_root_node() {
        let mut app = make_app();
        app.start_assistant_turn();
        assert_eq!(app.tree.len(), 1);
        assert_eq!(app.tree[0].depth, 0);
        assert_eq!(app.tree[0].label, "orchestrator");
        assert_eq!(app.tree[0].status, NodeStatus::Active);
    }

    #[test]
    fn enter_subtask_suspends_parent_and_adds_child() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "search files".into());
        assert_eq!(app.tree.len(), 2);
        assert_eq!(app.tree[0].status, NodeStatus::Suspended);
        assert_eq!(app.tree[1].depth, 1);
        assert_eq!(app.tree[1].label, "search files");
        assert_eq!(app.tree[1].status, NodeStatus::Active);
    }

    #[test]
    fn exit_subtask_marks_done_and_reactivates_parent() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "task".into());
        app.exit_subtask(1);
        assert_eq!(app.tree[0].status, NodeStatus::Active);
        assert_eq!(app.tree[1].status, NodeStatus::Done);
    }

    #[test]
    fn enter_exit_resets_tool_call_counter() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.subtask_tool_calls = 5;
        app.enter_subtask(1, "task".into());
        assert_eq!(app.subtask_tool_calls, 0);
        app.subtask_tool_calls = 3;
        app.exit_subtask(1);
        assert_eq!(app.subtask_tool_calls, 0);
    }

    #[test]
    fn nested_subtasks_suspend_and_resume_correctly() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "coordinator".into());
        app.enter_subtask(2, "worker".into());
        assert_eq!(app.tree[0].status, NodeStatus::Suspended);
        assert_eq!(app.tree[1].status, NodeStatus::Suspended);
        assert_eq!(app.tree[2].status, NodeStatus::Active);
        app.exit_subtask(2);
        assert_eq!(app.tree[2].status, NodeStatus::Done);
        assert_eq!(app.tree[1].status, NodeStatus::Active);
        assert_eq!(app.tree[0].status, NodeStatus::Suspended);
        app.exit_subtask(1);
        assert_eq!(app.tree[1].status, NodeStatus::Done);
        assert_eq!(app.tree[0].status, NodeStatus::Active);
    }

    #[test]
    fn fail_active_node_marks_active_as_failed() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "failing task".into());
        app.fail_active_node();
        assert_eq!(app.tree[1].status, NodeStatus::Failed);
        assert_eq!(app.tree[0].status, NodeStatus::Suspended);
    }

    #[test]
    fn has_tree_false_until_subtask_entered() {
        let mut app = make_app();
        app.start_assistant_turn();
        assert!(!app.has_tree());
        app.enter_subtask(1, "task".into());
        assert!(app.has_tree());
    }

    #[test]
    fn finish_assistant_turn_clears_tree() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "task".into());
        app.append_streaming_text("done");
        app.finish_assistant_turn();
        assert!(app.tree.is_empty());
        assert_eq!(app.subtask_tool_calls, 0);
    }

    #[test]
    fn multiple_subtasks_at_same_depth() {
        let mut app = make_app();
        app.start_assistant_turn();
        app.enter_subtask(1, "first".into());
        app.exit_subtask(1);
        app.enter_subtask(1, "second".into());
        app.exit_subtask(1);
        assert_eq!(app.tree.len(), 3);
        assert_eq!(app.tree[1].status, NodeStatus::Done);
        assert_eq!(app.tree[2].status, NodeStatus::Done);
        assert_eq!(app.tree[0].status, NodeStatus::Active);
    }
}
