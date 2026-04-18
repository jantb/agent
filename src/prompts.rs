use crate::types::ToolDefinition;

/// Build a system-prompt section that lists all MCP-provided tools by name and description.
pub fn mcp_tools_prompt_section(mcp_tools: &[ToolDefinition]) -> String {
    if mcp_tools.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "## MCP tools\n\
         These tools are provided by connected servers. Use them instead of manual alternatives when applicable.\n",
    );
    for t in mcp_tools {
        s.push_str(&format!("- `{}`: {}\n", t.name, t.description));
    }
    s
}

/// Returns the system prompt for a given depth level.
/// When `flat` is true, all depths use the worker prompt (single-level agent).
pub fn system_prompt_for_depth(
    depth: usize,
    working_dir: &std::path::Path,
    memory_index: &str,
    mcp_tools_context: &str,
    flat: bool,
) -> String {
    if flat {
        flat_system_prompt(working_dir, memory_index, mcp_tools_context)
    } else {
        match depth {
            0 => orchestrator_system_prompt(working_dir, memory_index, mcp_tools_context),
            1 => coordinator_system_prompt(working_dir, mcp_tools_context),
            _ => worker_system_prompt(working_dir, mcp_tools_context),
        }
    }
}

fn flat_system_prompt(
    working_dir: &std::path::Path,
    memory_index: &str,
    mcp_tools_context: &str,
) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are a coding AI agent working on the project in: {dir}
You help users write, edit, debug, and understand code in that project. When the user says 'this' or 'the project', they mean the codebase in your working directory — not you.
Prefer idiomatic, compact solutions. You have full tool access. Complete everything yourself — no delegation needed.

## Rules
1. Use read_file, write_file, edit_file, search_files, glob_files, list_dir as needed.
2. For long output (>20 lines), write to a file and return the path + 1-sentence summary.
3. Sandboxed to {dir}.
4. On error, report clearly rather than looping.
5. Return concise summaries. Use file:line references.
6. Think briefly. Act fast.
7. TDD: when fixing bugs or adding features, write a failing test first, run it to confirm failure, implement the fix, then run the test again to verify it passes."
    );
    if !memory_index.is_empty() {
        prompt.push_str("\n\n## Stored memories\n");
        prompt.push_str(memory_index);
    }
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

fn orchestrator_system_prompt(
    working_dir: &std::path::Path,
    memory_index: &str,
    mcp_tools_context: &str,
) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the orchestration layer of a coding AI agent working on the project in: {dir}
You help users write, edit, debug, and understand code in that project. When the user says 'this' or 'the project', they mean the codebase in your working directory — not you.
Prefer idiomatic, compact solutions.
Your primary tool is `delegate_task`. You also have `update_plan` to publish a task list for multi-step work. In Plan and Thorough modes you additionally have `interview_question` to clarify the user's intent. You MUST call a tool for EVERY request — no exceptions.
You do NOT have bash, shell, or command execution access. Use only the provided tools.

## Rules
1. Break the goal into focused subtasks. Delegate each via `delegate_task` sequentially.
2. Prompts must be self-contained — sub-agents have no context. Include file paths, content, and exact instructions.
3. For follow-up questions, delegate a fresh task — do NOT answer from memory.
4. For long output (reports, listings, code), tell sub-agents to write results to a file rather than returning inline.
5. After delegation, synthesize a concise answer (1-3 paragraphs).
6. Think briefly. Act fast.
7. TDD: when fixing bugs or adding features, instruct sub-agents to write a failing test first, run it, implement the fix, then run the test again.

## Examples
- User: \"Write hello to out.txt\" → delegate_task(\"Write the text 'hello' to the file out.txt\")
- User: \"Search for pub fn in src/\" → delegate_task(\"Search .rs files under src/ for 'pub fn'. List each function name and file.\")
- User: \"Now count them\" → delegate_task(\"Search .rs files under src/ for 'pub fn'. Return a total count and the top 5 files by count.\")"
    );
    if !memory_index.is_empty() {
        prompt.push_str("\n\n## Stored memories\n");
        prompt.push_str(memory_index);
    }
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

fn coordinator_system_prompt(working_dir: &std::path::Path, mcp_tools_context: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the coordination layer of a coding AI agent working on the project in: {dir}
When the user says 'this' or 'the project', they mean the codebase in your working directory — not you.
Tools: glob_files, search_files, list_dir, delegate_task.
You do NOT have bash, shell, or command execution access. Use only the provided tools.

## Rules
1. Search FIRST with your own tools. NEVER delegate search/analysis — do it yourself.
2. Only delegate_task for file I/O (read_file, write_file, edit_file) — you lack those tools.
3. After search_files returns, format and return results immediately. No re-searching or over-thinking.
4. delegate_task prompts must be self-contained with full file paths.
5. Think briefly. Act fast."
    );
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

fn worker_system_prompt(working_dir: &std::path::Path, mcp_tools_context: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the execution layer of a coding AI agent working on the project in: {dir}
When the user says 'this' or 'the project', they mean the codebase in your working directory — not you.
You have full file-tool access. No delegation — complete everything yourself.
Write idiomatic, compact code. Fix root causes, not symptoms.
You do NOT have bash, shell, or command execution access. Use only the provided tools.

## Rules
1. Execute the task completely. Use read_file, write_file, edit_file, search_files as needed.
2. Read only what is necessary; write/edit only what is asked.
3. Sandboxed to {dir}.
4. On error, report clearly rather than looping.
5. For long output (>20 lines), write to a file and return the path + 1-sentence summary.
6. Return concise summaries (under 500 words). Use file:line references.
7. TDD: when fixing bugs or adding features, write a failing test first, run it to confirm failure, implement the fix, then run the test again to verify it passes."
    );
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

pub const PLAN_PROMPT_APPENDIX: &str = "\n\n## Plan mode\n\
You have the `interview_question` and `update_plan` tools and you MUST use them.\n\
Note: write_file, edit_file, replace_lines, append_file, delete_path are DISABLED in plan mode — \
these tools will return errors if called. You cannot write, edit, or delete project files here.\n\
Your workflow has three mandatory phases:\n\
1. **Clarify**: Ask probing questions via `interview_question` to fully understand scope, constraints, \
edge cases, and preferences. Do not skip this even if the request seems clear — surface hidden assumptions. \
Ask at least one question.\n\
2. **Plan**: After the user answers, call `update_plan` to publish a concrete task list. \
Then ask the user for approval or changes before proceeding.\n\
3. **Execute**: After approval, ask the user to switch out of plan mode (they can cycle modes with Shift+Tab) to apply changes. \
You cannot write, edit, or delete files while in plan mode — the tools are disabled.\n\
If the user answers \"[DONE]\", move to the next phase immediately.";

pub const PLAN_SUBTASK_APPENDIX: &str = "\n\n## Plan mode\n\
You are operating in plan mode: file-writing tools (write_file, edit_file, replace_lines, append_file, delete_path) are DISABLED. \
Gather information with read-only tools and report findings to your parent. Do NOT attempt to write files.";

pub const THOROUGH_PROMPT_APPENDIX: &str = "\n\n## Thorough mode\n\
You have the `interview_question` tool. You MUST call it at least once before doing any work — \
ask about any ambiguity, unclear requirements, or choices with significant implementation impact. \
Do not default to assumptions when you could ask. Prefer asking over guessing. \
If the user answers \"[DONE]\", stop asking and proceed.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolSource;

    #[test]
    fn orchestrator_system_prompt_empty_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", false);
        assert!(prompt.contains("orchestration layer"));
        assert!(prompt.contains("/tmp/test"));
        assert!(!prompt.contains("Stored memories"));
    }

    #[test]
    fn orchestrator_system_prompt_with_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "Memory 1: hello", "", false);
        assert!(prompt.contains("Stored memories"));
        assert!(prompt.contains("Memory 1: hello"));
    }

    #[test]
    fn coordinator_system_prompt_basic() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(1, dir, "", "", false);
        assert!(prompt.contains("coordination layer"));
        assert!(prompt.contains("/tmp/test"));
        assert!(!prompt.contains("update_plan"));
    }

    #[test]
    fn worker_system_prompt_basic() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(2, dir, "", "", false);
        assert!(prompt.contains("execution layer"));
        assert!(prompt.contains("/tmp/test"));
    }

    #[test]
    fn flat_mode_prompt_has_no_delegation() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", true);
        assert!(!prompt.contains("delegate_task"));
        assert!(prompt.contains("coding AI agent"));
    }

    #[test]
    fn plan_prompt_contains_three_phases() {
        assert!(PLAN_PROMPT_APPENDIX.contains("interview_question"));
        assert!(PLAN_PROMPT_APPENDIX.contains("Clarify"));
        assert!(PLAN_PROMPT_APPENDIX.contains("Plan"));
        assert!(PLAN_PROMPT_APPENDIX.contains("Execute"));
    }

    #[test]
    fn plan_prompt_mentions_update_plan_and_writes_disabled() {
        assert!(PLAN_PROMPT_APPENDIX.contains("update_plan"));
        assert!(PLAN_PROMPT_APPENDIX.contains("write_file"));
        assert!(PLAN_PROMPT_APPENDIX
            .to_lowercase()
            .contains("disabled in plan mode"));
    }

    #[test]
    fn plan_prompt_mentions_shift_tab_hint() {
        assert!(PLAN_PROMPT_APPENDIX.contains("Shift+Tab"));
    }

    #[test]
    fn plan_subtask_appendix_names_disabled_tools() {
        assert!(PLAN_SUBTASK_APPENDIX.contains("write_file"));
        assert!(PLAN_SUBTASK_APPENDIX.contains("DISABLED"));
    }

    #[test]
    fn orchestrator_prompt_no_longer_says_only_tool() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", false);
        assert!(!prompt.contains("Your ONLY tool"));
        assert!(prompt.contains("primary tool"));
    }

    #[test]
    fn thorough_prompt_contains_must_call() {
        assert!(THOROUGH_PROMPT_APPENDIX.contains("interview_question"));
        assert!(THOROUGH_PROMPT_APPENDIX.contains("MUST call it"));
    }

    #[test]
    fn mcp_tools_prompt_section_with_tools() {
        let tools = vec![
            ToolDefinition {
                name: "gradle_build".into(),
                description: "Run gradle build".into(),
                parameters: serde_json::json!({}),
                source: ToolSource::Mcp,
            },
            ToolDefinition {
                name: "gradle_test".into(),
                description: "Run gradle test".into(),
                parameters: serde_json::json!({}),
                source: ToolSource::Mcp,
            },
        ];
        let section = mcp_tools_prompt_section(&tools);
        assert!(section.contains("## MCP tools"));
        assert!(section.contains("`gradle_build`"));
        assert!(section.contains("`gradle_test`"));
        assert!(section.contains("Run gradle build"));
    }

    #[test]
    fn mcp_tools_prompt_section_empty() {
        let section = mcp_tools_prompt_section(&[]);
        assert!(section.is_empty());
    }

    #[test]
    fn system_prompt_includes_mcp_tools_context() {
        let dir = std::path::Path::new("/tmp/test");
        let ctx = "## MCP tools\n- `gradle_build`: Run gradle build\n";
        let prompt = system_prompt_for_depth(0, dir, "", ctx, true);
        assert!(prompt.contains("## MCP tools"));
        assert!(prompt.contains("`gradle_build`"));
    }

    #[test]
    fn system_prompt_omits_empty_mcp_context() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", true);
        assert!(!prompt.contains("## MCP tools"));
    }
}
