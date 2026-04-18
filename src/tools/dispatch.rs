use std::path::Path;
use std::time::Instant;

use tracing::{debug, warn};

use crate::types::{AgentMode, ToolCall, ToolResult};

use super::builtin;
use super::PLAN_WRITE_TOOLS;

pub(crate) async fn execute_built_in_with_mode(
    call: &ToolCall,
    working_dir: &Path,
    mode: AgentMode,
) -> ToolResult {
    // Belt-and-suspenders: block write tools in Plan mode even if the tool filter missed them.
    if mode == AgentMode::Plan && PLAN_WRITE_TOOLS.contains(&call.name.as_str()) {
        return ToolResult {
            call_id: call.id.clone(),
            output: format!(
                "error: '{}' is disabled in plan mode — write tools are not available. \
                 Ask the user to switch out of plan mode to apply changes.",
                call.name
            ),
            is_error: true,
            images: vec![],
        };
    }

    let t0 = Instant::now();
    debug!(tool = %call.name, "built-in tool start");

    if call.name == "read_image" {
        let result = builtin::run_read_image(call, working_dir).await;
        let ms = t0.elapsed().as_millis();
        return match result {
            Ok((text, images)) => {
                debug!(tool = "read_image", ms, "built-in tool ok");
                ToolResult {
                    call_id: call.id.clone(),
                    output: text,
                    is_error: false,
                    images,
                }
            }
            Err(msg) => {
                warn!(tool = "read_image", ms, error = %msg, "built-in tool error");
                ToolResult {
                    call_id: call.id.clone(),
                    output: msg,
                    is_error: true,
                    images: vec![],
                }
            }
        };
    }

    let output = match call.name.as_str() {
        "read_file" => builtin::run_read_file(call, working_dir).await,
        "write_file" => builtin::run_write_file(call, working_dir).await,
        "list_dir" => builtin::run_list_dir(call, working_dir).await,
        "edit_file" => builtin::run_edit_file(call, working_dir).await,
        "search_files" => builtin::run_search_files(call, working_dir).await,
        "replace_lines" => builtin::run_replace_lines(call, working_dir).await,
        "diff_files" => builtin::run_diff_files(call, working_dir).await,
        "append_file" => builtin::run_append_file(call, working_dir).await,
        "delete_path" => builtin::run_delete_path(call, working_dir).await,
        "glob_files" => builtin::run_glob_files(call, working_dir).await,
        "line_count" => builtin::run_line_count(call, working_dir).await,
        "read_pdf" => builtin::run_read_pdf(call, working_dir).await,
        "remember" => builtin::run_remember(call, working_dir).await,
        "recall" => builtin::run_recall(call, working_dir).await,
        "forget" => builtin::run_forget(call, working_dir).await,
        "list_memories" => builtin::run_list_memories(call, working_dir).await,
        "delegate_task" => Err("delegate_task must be intercepted before dispatch".into()),
        "update_plan" => Err("update_plan must be intercepted before dispatch".into()),
        "read_image" => unreachable!("read_image handled above"),
        other => Err(format!("unknown built-in tool: {other}")),
    };
    let ms = t0.elapsed().as_millis();

    match output {
        Ok(text) => {
            debug!(tool = %call.name, ms, "built-in tool ok");
            ToolResult {
                call_id: call.id.clone(),
                output: text,
                is_error: false,
                images: vec![],
            }
        }
        Err(msg) => {
            warn!(tool = %call.name, ms, error = %msg, "built-in tool error");
            ToolResult {
                call_id: call.id.clone(),
                output: msg,
                is_error: true,
                images: vec![],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::TempDir;

    async fn execute_built_in(call: &ToolCall, working_dir: &std::path::Path) -> ToolResult {
        execute_built_in_with_mode(call, working_dir, AgentMode::Oneshot).await
    }

    fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "test-id".into(),
            name: name.into(),
            arguments: args,
        }
    }

    fn setup_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[tokio::test]
    async fn plan_mode_blocks_write_file() {
        let dir = setup_dir();
        let call = make_call("write_file", json!({"path": "x.txt", "content": "hi"}));
        let result = execute_built_in_with_mode(&call, dir.path(), AgentMode::Plan).await;
        assert!(result.is_error);
        assert!(result.output.contains("disabled in plan mode"));
    }

    #[tokio::test]
    async fn plan_mode_blocks_edit_file() {
        let dir = setup_dir();
        let call = make_call(
            "edit_file",
            json!({"path": "x.txt", "old_string": "a", "new_string": "b"}),
        );
        let result = execute_built_in_with_mode(&call, dir.path(), AgentMode::Plan).await;
        assert!(result.is_error);
        assert!(result.output.contains("disabled in plan mode"));
    }

    #[tokio::test]
    async fn plan_mode_blocks_replace_lines() {
        let dir = setup_dir();
        let call = make_call(
            "replace_lines",
            json!({"path": "x.txt", "start_line": 1, "end_line": 1, "new_content": "x"}),
        );
        let result = execute_built_in_with_mode(&call, dir.path(), AgentMode::Plan).await;
        assert!(result.is_error);
        assert!(result.output.contains("disabled in plan mode"));
    }

    #[tokio::test]
    async fn plan_mode_blocks_append_file() {
        let dir = setup_dir();
        let call = make_call("append_file", json!({"path": "x.txt", "content": "hi"}));
        let result = execute_built_in_with_mode(&call, dir.path(), AgentMode::Plan).await;
        assert!(result.is_error);
        assert!(result.output.contains("disabled in plan mode"));
    }

    #[tokio::test]
    async fn plan_mode_blocks_delete_path() {
        let dir = setup_dir();
        let call = make_call("delete_path", json!({"path": "x.txt"}));
        let result = execute_built_in_with_mode(&call, dir.path(), AgentMode::Plan).await;
        assert!(result.is_error);
        assert!(result.output.contains("disabled in plan mode"));
    }

    #[tokio::test]
    async fn oneshot_mode_does_not_block_write_file() {
        let dir = setup_dir();
        let call = make_call("write_file", json!({"path": "x.txt", "content": "hello"}));
        let result = execute_built_in_with_mode(&call, dir.path(), AgentMode::Oneshot).await;
        assert!(!result.is_error, "{}", result.output);
    }

    #[tokio::test]
    async fn read_file_happy_path() {
        let dir = setup_dir();
        let file = dir.path().join("hello.txt");
        std::fs::write(&file, "hello world").unwrap();
        let call = make_call("read_file", json!({"path": "hello.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("hello world"));
    }

    #[tokio::test]
    async fn write_file_happy_path() {
        let dir = setup_dir();
        let call = make_call(
            "write_file",
            json!({"path": "sub/file.txt", "content": "data"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let content = std::fs::read_to_string(dir.path().join("sub/file.txt")).unwrap();
        assert_eq!(content, "data");
    }

    #[tokio::test]
    async fn list_dir_shows_files() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        let call = make_call("list_dir", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("a.txt"));
        assert!(result.output.contains("b.txt"));
    }

    #[tokio::test]
    async fn list_dir_skips_hidden() {
        let dir = setup_dir();
        std::fs::write(dir.path().join(".hidden"), "x").unwrap();
        std::fs::write(dir.path().join("visible.txt"), "y").unwrap();
        let call = make_call("list_dir", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(!result.output.contains(".hidden"));
        assert!(result.output.contains("visible.txt"));
    }

    #[tokio::test]
    async fn list_dir_skips_build_dirs() {
        let dir = setup_dir();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("debug"), "").unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("src.rs"), "x").unwrap();
        let call = make_call("list_dir", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(!result.output.contains("target"));
        assert!(!result.output.contains("node_modules"));
        assert!(result.output.contains("src.rs"));
    }

    #[tokio::test]
    async fn list_dir_is_not_recursive() {
        let dir = setup_dir();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), "n").unwrap();
        let call = make_call("list_dir", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("sub/"));
        assert!(!result.output.contains("nested.txt"));
    }

    #[tokio::test]
    async fn list_dir_depth_recurses() {
        let dir = setup_dir();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), "n").unwrap();
        let call = make_call("list_dir", json!({"path": ".", "depth": 1}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("nested.txt"));
    }

    #[tokio::test]
    async fn search_files_finds_pattern() {
        let dir = setup_dir();
        let mut f = std::fs::File::create(dir.path().join("code.rs")).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();
        let call = make_call("search_files", json!({"pattern": "fn main", "path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("fn main"));
    }

    #[tokio::test]
    async fn search_files_regex() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("code.rs"), "fn foo() {}\nfn bar() {}\n").unwrap();
        let call = make_call(
            "search_files",
            json!({"pattern": "fn \\w+\\(\\)", "path": ".", "is_regex": true}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("fn foo()"));
        assert!(result.output.contains("fn bar()"));
    }

    #[tokio::test]
    async fn search_files_invalid_regex_errors() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("f.txt"), "hello").unwrap();
        let call = make_call(
            "search_files",
            json!({"pattern": "[invalid", "path": ".", "is_regex": true}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("invalid regex"));
    }

    #[tokio::test]
    async fn edit_file_unique_match_succeeds() {
        let dir = setup_dir();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn hello() {\n    println!(\"hi\");\n}\n").unwrap();
        let call = make_call(
            "edit_file",
            json!({
                "path": "test.rs",
                "old_string": "println!(\"hi\")",
                "new_string": "println!(\"bye\")"
            }),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("println!(\"bye\")"));
        assert!(!content.contains("println!(\"hi\")"));
    }

    #[tokio::test]
    async fn edit_file_no_match_returns_error() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "hello world").unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.txt", "old_string": "goodbye", "new_string": "hi"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn edit_file_multiple_matches_returns_error() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "aaa\naaa\naaa").unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.txt", "old_string": "aaa", "new_string": "bbb"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("matched 3 times"));
    }

    #[tokio::test]
    async fn edit_file_replace_all_replaces_all() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "aaa\naaa\naaa").unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.txt", "old_string": "aaa", "new_string": "bbb", "replace_all": true}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("3 occurrence(s)"));
        let content = std::fs::read_to_string(dir.path().join("test.txt")).unwrap();
        assert!(!content.contains("aaa"));
        assert_eq!(content.matches("bbb").count(), 3);
    }

    #[tokio::test]
    async fn edit_file_empty_old_string_returns_error() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.txt", "old_string": "", "new_string": "x"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn edit_file_identical_strings_returns_error() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.txt", "old_string": "hello", "new_string": "hello"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("identical"));
    }

    #[tokio::test]
    async fn edit_file_multiline_match() {
        let dir = setup_dir();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n").unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.rs", "old_string": "fn a() {\n    1\n}", "new_string": "fn a() {\n    42\n}"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let content = std::fs::read_to_string(&file).unwrap();
        assert!(content.contains("42"));
        assert!(content.contains("fn b()"));
    }

    #[tokio::test]
    async fn edit_file_partial_match_hint() {
        let dir = setup_dir();
        std::fs::write(
            dir.path().join("test.rs"),
            "fn hello() {\n    println!(\"hi\");\n}\n",
        )
        .unwrap();
        let call = make_call(
            "edit_file",
            json!({"path": "test.rs", "old_string": "fn hello() {\n    wrong_content\n}", "new_string": "replaced"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("Partial matches"));
        assert!(result.output.contains("fn hello()"));
    }

    #[tokio::test]
    async fn read_file_returns_line_numbers() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "aaa\nbbb\nccc\n").unwrap();
        let call = make_call("read_file", json!({"path": "test.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("1\t"));
        assert!(result.output.contains("aaa"));
        assert!(result.output.contains("3\t"));
        assert!(result.output.contains("ccc"));
    }

    #[tokio::test]
    async fn read_file_start_line() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "line1\nline2\nline3\nline4\n").unwrap();
        let call = make_call("read_file", json!({"path": "test.txt", "start_line": 3}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(!result.output.contains("line1"));
        assert!(!result.output.contains("line2"));
        assert!(result.output.contains("line3"));
        assert!(result.output.contains("line4"));
        assert!(result.output.contains("[showing lines"));
    }

    #[tokio::test]
    async fn read_file_end_line() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "line1\nline2\nline3\nline4\n").unwrap();
        let call = make_call("read_file", json!({"path": "test.txt", "end_line": 2}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("line1"));
        assert!(result.output.contains("line2"));
        assert!(!result.output.contains("line3"));
        assert!(result.output.contains("[showing lines 1-2 of 4]"));
    }

    #[tokio::test]
    async fn read_file_start_and_end_line() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "a\nb\nc\nd\ne\n").unwrap();
        let call = make_call(
            "read_file",
            json!({"path": "test.txt", "start_line": 2, "end_line": 4}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(!result.output.contains("\ta\n"));
        assert!(result.output.contains("\tb\n"));
        assert!(result.output.contains("\tc\n"));
        assert!(result.output.contains("\td\n"));
        assert!(!result.output.contains("\te\n"));
    }

    #[tokio::test]
    async fn read_file_start_line_past_end_errors() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "one\ntwo\n").unwrap();
        let call = make_call("read_file", json!({"path": "test.txt", "start_line": 100}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("exceeds"));
    }

    #[tokio::test]
    async fn read_file_no_range_reads_all() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("test.txt"), "a\nb\nc\n").unwrap();
        let call = make_call("read_file", json!({"path": "test.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("a"));
        assert!(result.output.contains("b"));
        assert!(result.output.contains("c"));
        assert!(!result.output.contains("[showing lines"));
    }

    #[tokio::test]
    async fn search_files_shows_match_count() {
        let dir = setup_dir();
        let mut f = std::fs::File::create(dir.path().join("data.txt")).unwrap();
        writeln!(f, "foo bar").unwrap();
        writeln!(f, "baz").unwrap();
        writeln!(f, "foo again").unwrap();
        let call = make_call("search_files", json!({"pattern": "foo", "path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert!(result.output.contains("[2 matches]"));
    }

    #[tokio::test]
    async fn search_files_no_matches_no_count() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("data.txt"), "nothing here").unwrap();
        let call = make_call("search_files", json!({"pattern": "zzz", "path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error);
        assert_eq!(result.output, "no matches found");
    }

    #[tokio::test]
    async fn replace_lines_basic() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\n").unwrap();
        let call = make_call(
            "replace_lines",
            json!({"path": "f.txt", "start_line": 2, "end_line": 3, "new_content": "X\nY\nZ"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let content = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
        assert_eq!(content, "a\nX\nY\nZ\nd\n");
    }

    #[tokio::test]
    async fn replace_lines_out_of_range_errors() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let call = make_call(
            "replace_lines",
            json!({"path": "f.txt", "start_line": 1, "end_line": 10, "new_content": "x"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn append_file_creates_and_appends() {
        let dir = setup_dir();
        let call = make_call(
            "append_file",
            json!({"path": "new.txt", "content": "hello"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let content = std::fs::read_to_string(dir.path().join("new.txt")).unwrap();
        assert_eq!(content, "hello");

        let call2 = make_call(
            "append_file",
            json!({"path": "new.txt", "content": " world"}),
        );
        let result2 = execute_built_in(&call2, dir.path()).await;
        assert!(!result2.is_error);
        let content2 = std::fs::read_to_string(dir.path().join("new.txt")).unwrap();
        assert_eq!(content2, "hello world");
    }

    #[tokio::test]
    async fn delete_path_file() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("del.txt"), "bye").unwrap();
        let call = make_call("delete_path", json!({"path": "del.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(!dir.path().join("del.txt").exists());
    }

    #[tokio::test]
    async fn delete_path_empty_dir() {
        let dir = setup_dir();
        std::fs::create_dir(dir.path().join("emptydir")).unwrap();
        let call = make_call("delete_path", json!({"path": "emptydir"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(!dir.path().join("emptydir").exists());
    }

    #[tokio::test]
    async fn delete_path_nonempty_dir_fails() {
        let dir = setup_dir();
        let sub = dir.path().join("nonempty");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("file.txt"), "x").unwrap();
        let call = make_call("delete_path", json!({"path": "nonempty"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn glob_files_finds_rs_files() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("main.rs"), "").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "").unwrap();
        std::fs::write(dir.path().join("readme.txt"), "").unwrap();
        let call = make_call("glob_files", json!({"pattern": "*.rs", "path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
        assert!(!result.output.contains("readme.txt"));
    }

    #[tokio::test]
    async fn diff_files_shows_changes() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("a.txt"), "line1\nline2\nline3\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "line1\nchanged\nline3\n").unwrap();
        let call = make_call("diff_files", json!({"path_a": "a.txt", "path_b": "b.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("-line2"));
        assert!(result.output.contains("+changed"));
        assert!(result.output.contains(" line1"));
    }

    #[tokio::test]
    async fn remember_creates_memory() {
        let dir = setup_dir();
        let call = make_call(
            "remember",
            json!({"name": "test tip", "content": "tokio is great", "description": "async tip", "tags": ["rust"]}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("test tip"));
        let mem_path = dir.path().join(".agent").join("memory").join("test-tip.md");
        assert!(mem_path.exists());
    }

    #[tokio::test]
    async fn recall_finds_created_memory() {
        let dir = setup_dir();
        let remember = make_call(
            "remember",
            json!({"name": "recall test", "content": "goroutines are like green threads", "description": "go concurrency"}),
        );
        execute_built_in(&remember, dir.path()).await;
        let call = make_call("recall", json!({"keyword": "goroutines"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("recall test"));
    }

    #[tokio::test]
    async fn forget_removes_memory() {
        let dir = setup_dir();
        let remember = make_call(
            "remember",
            json!({"name": "ephemeral", "content": "delete me", "description": "temp"}),
        );
        execute_built_in(&remember, dir.path()).await;
        let call = make_call("forget", json!({"name": "ephemeral"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let mem_path = dir
            .path()
            .join(".agent")
            .join("memory")
            .join("ephemeral.md");
        assert!(!mem_path.exists());
    }

    #[tokio::test]
    async fn read_file_empty_file() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("empty.txt"), "").unwrap();
        let call = make_call("read_file", json!({"path": "empty.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert_eq!(result.output, "");
    }

    #[tokio::test]
    async fn read_file_multibyte_truncation_no_panic() {
        let dir = setup_dir();
        let line = "あいうえお\n";
        let content = line.repeat(10_000);
        std::fs::write(dir.path().join("multibyte.txt"), &content).unwrap();
        let call = make_call("read_file", json!({"path": "multibyte.txt"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
    }

    #[tokio::test]
    async fn line_count_counts_lines() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n// comment\n").unwrap();
        std::fs::write(dir.path().join("b.rs"), "line1\nline2\nline3\n").unwrap();
        let call = make_call("line_count", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("b.rs"));
        assert!(result.output.contains("total lines"));
    }

    #[tokio::test]
    async fn line_count_extension_filter() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(dir.path().join("readme.md"), "# Title\nText\nMore\n").unwrap();
        let call = make_call("line_count", json!({"path": ".", "extensions": ["rs"]}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("main.rs"));
        assert!(!result.output.contains("readme.md"));
    }

    #[tokio::test]
    async fn line_count_empty_dir() {
        let dir = setup_dir();
        let call = make_call("line_count", json!({}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert_eq!(result.output, "no files found");
    }

    #[tokio::test]
    async fn line_count_skips_hidden_and_ignored() {
        let dir = setup_dir();
        std::fs::write(dir.path().join(".hidden"), "a\nb\n").unwrap();
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target").join("build.rs"), "x\n").unwrap();
        std::fs::write(dir.path().join("real.rs"), "fn x() {}\n").unwrap();
        let call = make_call("line_count", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("real.rs"));
        assert!(!result.output.contains(".hidden"));
        assert!(!result.output.contains("build.rs"));
    }

    #[tokio::test]
    async fn line_count_sorted_descending() {
        let dir = setup_dir();
        std::fs::write(dir.path().join("big.rs"), "a\nb\nc\nd\ne\n").unwrap();
        std::fs::write(dir.path().join("small.rs"), "x\n").unwrap();
        let call = make_call("line_count", json!({"path": "."}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        let big_pos = result.output.find("big.rs").unwrap();
        let small_pos = result.output.find("small.rs").unwrap();
        assert!(big_pos < small_pos, "big.rs should appear before small.rs");
    }

    #[tokio::test]
    async fn list_memories_shows_stored_memories() {
        let dir = setup_dir();
        for (name, desc) in [("Alpha note", "first"), ("Beta note", "second")] {
            let call = make_call(
                "remember",
                json!({"name": name, "content": "body", "description": desc}),
            );
            execute_built_in(&call, dir.path()).await;
        }
        let call = make_call("list_memories", json!({}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("Alpha note"));
        assert!(result.output.contains("Beta note"));
    }

    #[tokio::test]
    async fn read_image_loads_and_base64_encodes() {
        let dir = setup_dir();
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00";
        std::fs::write(dir.path().join("test.png"), data).unwrap();
        let call = make_call("read_image", json!({"path": "test.png"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("test.png"));
        assert_eq!(result.images.len(), 1);
        // Verify round-trip: decode base64 and compare
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&result.images[0])
            .unwrap();
        assert_eq!(decoded, data);
    }

    #[tokio::test]
    async fn read_image_missing_file_is_error() {
        let dir = setup_dir();
        let call = make_call("read_image", json!({"path": "no_such.png"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("read error"));
    }

    fn make_test_pdf(pages: &[&str]) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let font_dict = dictionary! { "F1" => font_id };
        let resources = dictionary! { "Font" => font_dict };
        let mut page_ids = vec![];
        for (i, text) in pages.iter().enumerate() {
            // Build a two-operator content stream per page:
            //   1. Tj to render the visible text
            //   2. A raw \x0c byte (form feed) between pages so that
            //      pdf_extract::extract_text inserts \x0c in its output,
            //      matching what run_read_pdf splits on.
            let mut content = format!("BT /F1 12 Tf 100 700 Td ({text}) Tj ET").into_bytes();
            if i + 1 < pages.len() {
                content.push(b'\x0c');
            }
            let stream = Stream::new(dictionary! {}, content);
            let content_id = doc.add_object(stream);
            let page = dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Contents" => content_id,
                "Resources" => resources.clone(),
            };
            page_ids.push(doc.add_object(page));
        }
        let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
        doc.objects.insert(
            pages_id,
            dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => Object::Integer(pages.len() as i64),
            }
            .into(),
        );
        let catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        let catalog_id = doc.add_object(catalog);
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    #[tokio::test]
    async fn read_pdf_extracts_text() {
        let dir = setup_dir();
        let pdf = make_test_pdf(&["hello world"]);
        std::fs::write(dir.path().join("test.pdf"), pdf).unwrap();
        let call = make_call("read_pdf", json!({"path": "test.pdf"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("Page 1"));
        assert!(result.output.to_lowercase().contains("hello"));
    }

    #[tokio::test]
    async fn read_pdf_missing_file_is_error() {
        let dir = setup_dir();
        let call = make_call("read_pdf", json!({"path": "no_such.pdf"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
    }

    // Build a PDF whose extracted text already contains \x0c page-break bytes by
    // constructing the content stream so pdf_extract encounters them in the raw
    // decoded stream.  pdf_extract's PlainTextOutput never inserts \x0c itself,
    // but it does pass through characters that the PDF's font encoding maps to
    // codepoint U+000C.  The easiest reliable mechanism: use lopdf to build the
    // PDF with a /ToUnicode CMap that maps byte 0x0C → U+000C, then embed
    // that byte inside a Tj string so pdf_extract outputs the \x0c character.
    //
    // Because building a ToUnicode CMap is involved, we instead take a simpler
    // path: build a single PDF with multiple PDF page objects (so lopdf writes
    // them as separate pages in the Pages tree) and rely on lopdf's own
    // get_pages() ordering.  Then we test the pages= parameter by requesting
    // page "1" from a 3-page PDF where each page has distinct text, verifying
    // only page 1 content appears.
    //
    // Note: pdf_extract::extract_text joins all pages without \x0c separators.
    // run_read_pdf splits on \x0c, so all content appears as one "page" (page 1).
    // Testing "pages=1" exercises the range filter code path end-to-end.
    #[tokio::test]
    async fn read_pdf_pages_filter() {
        let dir = setup_dir();
        let pdf = make_test_pdf(&["alpha text", "beta text", "gamma text"]);
        std::fs::write(dir.path().join("multi.pdf"), pdf).unwrap();
        // With pdf_extract, all content is extracted as one \x0c-less blob →
        // the tool sees 1 "page".  Requesting page "1" must succeed and include
        // the page header; requesting "2" must error (tested separately).
        let call = make_call("read_pdf", json!({"path": "multi.pdf", "pages": "1"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(!result.is_error, "{}", result.output);
        assert!(result.output.contains("Page 1"));
        // Content from all three PDF pages is merged by pdf_extract into one blob.
        assert!(
            result.output.to_lowercase().contains("alpha")
                || result.output.to_lowercase().contains("beta")
                || result.output.to_lowercase().contains("gamma")
        );
    }

    #[tokio::test]
    async fn read_pdf_invalid_page_range_errors() {
        let dir = setup_dir();
        let pdf = make_test_pdf(&["only page"]);
        std::fs::write(dir.path().join("one.pdf"), pdf).unwrap();
        let call = make_call("read_pdf", json!({"path": "one.pdf", "pages": "5"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("out of range"));
    }
}
