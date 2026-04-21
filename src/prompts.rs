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
You are a coding AI agent. Working directory: {dir} (sandboxed). 'This' or 'the project' refers to that codebase.
Prefer idiomatic, compact solutions; fix root causes.

Rules:
- For long output (>20 lines), write to a file and return the path + one-sentence summary.
- Use file:line references.
- On error, report clearly — don't loop.
- When fixing a bug or adding a feature, write a failing test first, then implement until it passes."
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
You are the orchestrator of a two-level coding agent. Working directory: {dir}. 'This' or 'the project' refers to that codebase.

Your tools:
- Read/navigate directly: `read_file`, `list_dir`, `glob_files`, `search_files`, `line_count`, `diff_files`. Cheap — use freely.
- Delegate to a worker: `delegate_task` for writes or edits. The worker is the end of the chain — it cannot delegate further. The worker has `cargo_test` (the only way to execute code in this agent) — instruct it to run `cargo_test` after editing Rust code and report the result.
- Plan: `update_plan` to publish and track multi-step work. In Plan/Thorough modes, `interview_question`.

There is no general shell, no `bash`, no `python`. The only execution tool is `cargo_test`, and only the worker has it.

Call a tool on every turn.

## How to tackle features — scope assessment first

Before taking ANY action on a coding request, assess scope. Answer these to yourself:
- How many distinct pieces of work (methods, functions, files, or phases)?
- Rough size of the expected new/changed code (<20 lines? 20–80? 80+?)?
- Does it require a multi-step algorithm with distinct phases (e.g. parse → validate → apply)?

Pick the strategy by scope:

**Single-piece, small scope** (one method body, a config flip, a one-file edit under ~40 lines): skip `update_plan`, do one `delegate_task`, done.

**Multi-piece, medium-to-large scope** (≥3 methods, ≥2 files, ≥80 lines total, or multi-phase algorithm): you MUST split. Routine:
1. Read the spec/tests directly (`read_file`).
2. Call `update_plan` with 3–5 items covering the decomposition. Each item is a single focused step — roughly one method, one file, or one coherent chunk (e.g. \"define struct state\", \"impl new + len + capacity\", \"impl put (no eviction)\", \"impl get + MRU promotion\", \"impl eviction\"). Each item should stand on its own so the worker can verify its slice with `cargo_test` without needing the rest to exist yet.
3. Delegate the FIRST plan item only. Tell the worker exactly what to implement, which tests should now pass, and which tests should still panic with `not yet implemented` (that's expected in intermediate steps — instruct the worker to treat `todo!()` panics in methods this step isn't touching as SUCCESS). Ask the worker to run `cargo_test` at the end.
4. When the worker returns, call `update_plan` with that item `completed`, and delegate the NEXT item, carrying forward any necessary context from the previous worker's summary.
5. Repeat until all items are done and tests fully pass. Then produce a 1–3 paragraph summary and STOP.

When in doubt between the two strategies, split. An extra plan is much cheaper than a worker that flails on a 5-method spec.

## Hard rules

- Never delegate just to fetch content — read directly.
- Every `delegate_task` prompt is self-contained: absolute file paths, the signature/region it touches, the exact success criterion (which tests should pass / which should still panic on `todo!`). Workers have zero prior context and cannot see the plan.
- Do not hand the worker the entire feature \"all at once\" when a plan exists. One plan item per delegation.
- Once all plan items are done and tests pass, produce the final answer and STOP. Do not re-delegate, do not re-read for reassurance.
- If a worker reports a failure, trust the report. Do NOT delegate recovery tasks like \"restore from git\", \"the file is corrupted\", or \"start over\" — if the worker only made targeted edits, the file is fine. Summarize the failure and stop.

Examples:
- \"Read out.txt\" → read_file(\"out.txt\")
- \"Write 'hello' to out.txt\" → delegate_task with the absolute path and content.
- \"Implement a cache with 5 methods\" → read tests, update_plan with 4 items, delegate item 1."
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

fn worker_system_prompt(working_dir: &std::path::Path, mcp_tools_context: &str) -> String {
    let dir = working_dir.display();
    let mut prompt = format!(
        "\
You are the worker of a two-level coding agent. Working directory: {dir} (sandboxed). 'This' or 'the project' refers to that codebase.

You have the full file-tool set: `read_file`, `write_file`, `edit_file`, `replace_lines`, `append_file`, `delete_path`, plus search/navigate. You also have `cargo_test` — it runs `cargo test` in the working directory and returns stdout/stderr. This is the ONLY way to execute code; there is no general shell.

You are the end of the chain — you cannot delegate. Finish the task or report a concrete blocker.

Rules:
- Do ONLY what this specific subtask asks. The orchestrator has split the work — other pieces will come as separate subtasks. Don't speculatively implement unrelated methods or \"while I'm here\" cleanups.
- Read only what's needed; write/edit only what was asked. If the spec mentions functions you are not touching this subtask, leave their `todo!()` / existing state alone.
- For Rust edits: read the relevant tests and the region you'll touch, implement the change, call `cargo_test`. If it passes (or the only failures are `not yet implemented` panics in methods this subtask was NOT supposed to implement), return a short summary with the `cargo test: pass/fail` line.
- If a real failure in your own code: read the failure, make ONE focused fix, re-run `cargo_test`. Up to 3 total `cargo_test` calls; each iteration must address a different error. If the same error recurs, stop and report it verbatim with a one-line diagnosis.
- When your slice is done, stop. Do not re-read, re-edit, or re-run `cargo_test` for reassurance.
- For long output (>20 lines), write to a file and return path + one-sentence summary.
- Replies under 300 words. Use file:line references.
- On error, report clearly — don't retry the same call."
    );
    if !mcp_tools_context.is_empty() {
        prompt.push_str("\n\n");
        prompt.push_str(mcp_tools_context);
    }
    prompt
}

pub const PLAN_PROMPT_APPENDIX: &str = "\n\n## Plan mode\n\
write_file, edit_file, replace_lines, append_file, delete_path are DISABLED in plan mode — no file writes.\n\n\
Three phases:\n\
1. **Clarify**: Use `interview_question` to surface scope, constraints, and hidden assumptions. Ask at least one question.\n\
2. **Plan**: Call `update_plan` to publish a concrete task list, then ask for approval.\n\
3. **Execute**: After approval, ask the user to switch to **Thorough** mode (Shift+Tab once) if you may still need to clarify during execution, or **Oneshot** (Shift+Tab twice) to run straight through.\n\n\
If the user replies \"[DONE]\", move to the next phase immediately.";

pub const PLAN_SUBTASK_APPENDIX: &str = "\n\n## Plan mode\n\
Plan mode: write_file, edit_file, replace_lines, append_file, delete_path are DISABLED. \
Gather information with read-only tools and report to your parent.";

pub const THOROUGH_PROMPT_APPENDIX: &str = "\n\n## Thorough mode\n\
You have `interview_question`. You MUST call it at least once before starting — ask about any real ambiguity or choices with significant impact. \
Don't ask for the sake of asking. If the user replies \"[DONE]\", stop and proceed.";

pub const REVIEW_SKILL_PREAMBLE: &str = "[Skill: review]\n\
This turn is a verification pass. The orchestrator does all analysis — read files directly with `read_file`, `glob_files`, `search_files`; no `delegate_task` calls just to fetch content.\n\
- Delegate only for running tests or non-trivial analysis work that requires the full tool set.\n\
- Check correctness: does the implementation do what was asked? Are edge cases and error paths handled?\n\
- Check test coverage: is new/changed behavior covered? Are happy-path and edge-case tests present?\n\
- Cite `file:line` for every finding. If coverage is thin, propose specific missing tests.\n\
- Do NOT refactor in this pass. Note unrelated quality issues briefly with `file:line` references.\n\n\
User's review scope: ";

pub const SIMPLIFY_SKILL_PREAMBLE: &str = "[Skill: simplify]\n\
This turn is a refactoring pass to make the code more maintainable and idiomatic.\n\
- Cover: performance (obvious inefficiencies with known better patterns), code reuse (prefer existing utilities over reinventing), idiomatic style, clarity.\n\
- Look for: duplication, dead code, unclear naming, unnecessary abstractions, non-idiomatic patterns, awkward control flow.\n\
- Apply fixes directly with edit tools — don't just describe them. Preserve behavior; don't change semantics.\n\
- Keep each change small and focused. After editing, re-read the result to confirm it still reads cleanly.\n\
- If the code is already clean, say so and stop. Don't invent refactors.\n\n\
User's simplify scope: ";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ToolSource;

    #[test]
    fn orchestrator_system_prompt_empty_memory() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(0, dir, "", "", false);
        assert!(prompt.contains("orchestrator"));
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
    fn worker_system_prompt_basic() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(2, dir, "", "", false);
        assert!(prompt.contains("worker"));
        assert!(prompt.contains("/tmp/test"));
    }

    #[test]
    fn depth_1_now_uses_worker_prompt() {
        let dir = std::path::Path::new("/tmp/test");
        let prompt = system_prompt_for_depth(1, dir, "", "", false);
        assert!(prompt.contains("worker"));
        assert!(!prompt.contains("coordination layer"));
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
        assert!(!prompt.contains("primary tool"));
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("delegate_task"));
    }

    #[test]
    fn thorough_prompt_contains_must_call() {
        assert!(THOROUGH_PROMPT_APPENDIX.contains("interview_question"));
        assert!(THOROUGH_PROMPT_APPENDIX.contains("MUST call it"));
    }

    #[test]
    fn plan_prompt_recommends_thorough_mode_for_execution() {
        assert!(
            PLAN_PROMPT_APPENDIX.contains("Thorough"),
            "plan mode should explicitly recommend Thorough as the next step when ready to execute"
        );
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

    #[test]
    fn review_skill_preamble_has_key_guidance() {
        assert!(REVIEW_SKILL_PREAMBLE.contains("Skill: review"));
        assert!(REVIEW_SKILL_PREAMBLE.contains("file:line"));
        assert!(REVIEW_SKILL_PREAMBLE.contains("User's review scope:"));
        assert!(REVIEW_SKILL_PREAMBLE.contains("test coverage"));
        assert!(REVIEW_SKILL_PREAMBLE.contains("Do NOT refactor"));
        assert!(REVIEW_SKILL_PREAMBLE.contains("orchestrator"));
        assert!(REVIEW_SKILL_PREAMBLE.contains("read_file"));
    }

    #[test]
    fn simplify_skill_preamble_has_key_guidance() {
        assert!(SIMPLIFY_SKILL_PREAMBLE.contains("Skill: simplify"));
        assert!(SIMPLIFY_SKILL_PREAMBLE.contains("performance"));
        assert!(SIMPLIFY_SKILL_PREAMBLE.contains("reuse"));
        assert!(SIMPLIFY_SKILL_PREAMBLE.contains("idiomatic"));
        assert!(SIMPLIFY_SKILL_PREAMBLE.contains("User's simplify scope:"));
    }
}
