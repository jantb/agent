use crate::{
    tools::PLAN_WRITE_TOOLS,
    types::{AgentMode, ToolDefinition},
};

/// Heuristic: dense models too slow for multi-level delegation.
/// Matches "31b" or "dense" in the name. Intentionally conservative —
/// broaden if a new model family needs flat mode.
pub fn is_flat_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("31b") || m.contains("dense")
}

/// Filter the full tool set to the subset appropriate for a given depth.
/// When `flat` is true, all depths get worker tools (no delegate_task).
/// Two-tier model:
/// depth 0 (orchestrator): delegate_task + update_plan + read-only file tools
/// depth 1+ (worker):      all tools except delegate_task and update_plan
pub(crate) fn tools_for_depth(
    all_tools: &[ToolDefinition],
    depth: usize,
    flat: bool,
    mode: AgentMode,
) -> Vec<ToolDefinition> {
    let mut tools: Vec<ToolDefinition> = if flat {
        all_tools
            .iter()
            .filter(|t| t.name != "delegate_task" && (depth == 0 || t.name != "update_plan"))
            .cloned()
            .collect()
    } else {
        const ORCHESTRATOR_TOOLS: &[&str] = &[
            "delegate_task",
            "update_plan",
            "read_file",
            "list_dir",
            "glob_files",
            "search_files",
            "line_count",
            "diff_files",
        ];
        match depth {
            0 => all_tools
                .iter()
                .filter(|t| ORCHESTRATOR_TOOLS.contains(&t.name.as_str()))
                .cloned()
                .collect(),
            _ => all_tools
                .iter()
                .filter(|t| t.name != "delegate_task" && t.name != "update_plan")
                .cloned()
                .collect(),
        }
    };
    if matches!(mode, AgentMode::Plan | AgentMode::Thorough) && depth == 0 {
        tools.push(crate::tools::interview_question_def());
    }
    if mode == AgentMode::Plan {
        tools.retain(|t| !PLAN_WRITE_TOOLS.contains(&t.name.as_str()));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::built_in_tool_definitions;

    #[test]
    fn tools_for_depth_orchestrator_has_reads_and_delegate() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"update_plan"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"list_dir"));
        assert!(names.contains(&"glob_files"));
        assert!(names.contains(&"search_files"));
        assert!(names.contains(&"line_count"));
        assert!(names.contains(&"diff_files"));
        assert!(!names.contains(&"write_file"));
        assert!(!names.contains(&"edit_file"));
        assert!(!names.contains(&"replace_lines"));
        assert!(!names.contains(&"append_file"));
        assert!(!names.contains(&"delete_path"));
        assert_eq!(tools.len(), 8);
    }

    #[test]
    fn tools_for_depth_worker_has_file_tools_no_delegate() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"edit_file"));
        assert!(!names.contains(&"delegate_task"));
        assert!(!names.contains(&"update_plan"));
    }

    #[test]
    fn depth_1_now_gets_worker_tools() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 1, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"edit_file"));
        assert!(!names.contains(&"delegate_task"));
        assert!(!names.contains(&"update_plan"));
    }

    #[test]
    fn tools_for_depth_worker_excludes_delegate_and_update_plan() {
        let all = built_in_tool_definitions();
        let worker_tools = tools_for_depth(&all, 2, false, AgentMode::Oneshot);
        let mut expected: std::collections::HashSet<_> =
            all.iter().map(|t| t.name.as_str()).collect();
        expected.remove("delegate_task");
        expected.remove("update_plan");
        let worker_names: std::collections::HashSet<_> =
            worker_tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(worker_names, expected);
    }

    #[test]
    fn tools_for_depth_1_2_3_all_worker() {
        let all = built_in_tool_definitions();
        let mut d1: Vec<_> = tools_for_depth(&all, 1, false, AgentMode::Oneshot)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let mut d2: Vec<_> = tools_for_depth(&all, 2, false, AgentMode::Oneshot)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        let mut d3: Vec<_> = tools_for_depth(&all, 3, false, AgentMode::Oneshot)
            .iter()
            .map(|t| t.name.clone())
            .collect();
        d1.sort();
        d2.sort();
        d3.sort();
        assert_eq!(d1, d2);
        assert_eq!(d2, d3);
    }

    #[test]
    fn flat_mode_all_depths_get_worker_tools() {
        let all = built_in_tool_definitions();
        let d0 = tools_for_depth(&all, 0, true, AgentMode::Oneshot);
        let d1 = tools_for_depth(&all, 1, true, AgentMode::Oneshot);
        let d2 = tools_for_depth(&all, 2, true, AgentMode::Oneshot);
        // depth 0 has update_plan; depth 1+ do not
        assert_eq!(d0.len(), d2.len() + 1);
        assert_eq!(d1.len(), d2.len());
        assert!(d0.iter().all(|t| t.name != "delegate_task"));
        assert!(d0.iter().any(|t| t.name == "update_plan"));
        assert!(d1.iter().all(|t| t.name != "update_plan"));
    }

    #[test]
    fn thorough_mode_adds_interview_question_at_depth_0() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Thorough);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"interview_question"));
    }

    #[test]
    fn thorough_mode_does_not_add_interview_question_at_depth_2() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Thorough);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"interview_question"));
    }

    #[test]
    fn plan_mode_adds_interview_question_at_depth_0() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"interview_question"));
    }

    #[test]
    fn plan_mode_no_interview_at_depth_2() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!names.contains(&"interview_question"));
    }

    #[test]
    fn oneshot_no_interview_any_depth() {
        let all = built_in_tool_definitions();
        for depth in 0..3 {
            let tools = tools_for_depth(&all, depth, false, AgentMode::Oneshot);
            let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
            assert!(
                !names.contains(&"interview_question"),
                "depth {depth} should not have interview_question in Oneshot"
            );
        }
    }

    #[test]
    fn plan_mode_worker_excludes_all_write_tools() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        for write_tool in PLAN_WRITE_TOOLS {
            assert!(
                !names.contains(write_tool),
                "Plan mode worker should not have {write_tool}"
            );
        }
    }

    #[test]
    fn plan_mode_depth0_has_update_plan_delegate_interview() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, false, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"update_plan"));
        assert!(names.contains(&"interview_question"));
        // No writes
        for write_tool in PLAN_WRITE_TOOLS {
            assert!(!names.contains(write_tool));
        }
    }

    #[test]
    fn plan_mode_flat_excludes_writes_includes_reads() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 0, true, AgentMode::Plan);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        for write_tool in PLAN_WRITE_TOOLS {
            assert!(
                !names.contains(write_tool),
                "Plan mode flat should not have {write_tool}"
            );
        }
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"search_files"));
    }

    #[test]
    fn oneshot_worker_still_has_writes() {
        let all = built_in_tool_definitions();
        let tools = tools_for_depth(&all, 2, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        for write_tool in PLAN_WRITE_TOOLS {
            assert!(
                names.contains(write_tool),
                "Oneshot worker should have {write_tool}"
            );
        }
    }

    #[test]
    fn is_flat_model_detects_dense() {
        assert!(is_flat_model("gemma4:31b"));
        assert!(is_flat_model("gemma4:31b-cloud"));
        assert!(!is_flat_model("gemma4:26b"));
        assert!(!is_flat_model("gemma4:e4b"));
    }

    #[test]
    fn update_plan_not_in_worker_tools() {
        let all = built_in_tool_definitions();
        for depth in 2..4 {
            let tools = tools_for_depth(&all, depth, false, AgentMode::Oneshot);
            assert!(
                tools.iter().all(|t| t.name != "update_plan"),
                "depth {depth} should not have update_plan"
            );
        }
    }

    #[test]
    fn update_plan_only_in_flat_depth0() {
        let all = built_in_tool_definitions();
        let d0 = tools_for_depth(&all, 0, true, AgentMode::Oneshot);
        let d1 = tools_for_depth(&all, 1, true, AgentMode::Oneshot);
        assert!(d0.iter().any(|t| t.name == "update_plan"));
        assert!(d1.iter().all(|t| t.name != "update_plan"));
    }
}
