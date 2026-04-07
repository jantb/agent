use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::json;

use crate::memory;
use crate::types::{ToolCall, ToolDefinition, ToolResult, ToolSource};

pub fn built_in_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the UTF-8 content of a file. Returns numbered lines. Use start_line/end_line to read specific ranges for large files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative or absolute path to read" },
                    "start_line": { "type": "integer", "description": "First line to read (1-based, inclusive). Omit to start from beginning." },
                    "end_line": { "type": "integer", "description": "Last line to read (1-based, inclusive). Omit to read to end." }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file, creating parent directories as needed.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to write" },
                    "content": { "type": "string", "description": "File content" }
                },
                "required": ["path", "content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List contents of a directory. depth=0 (default) lists immediate children only; depth>0 recurses that many levels. Max depth 10. Shows file size and modified time.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list" },
                    "depth": { "type": "integer", "description": "How many levels to recurse (0 = immediate only, max 10)" }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "edit_file".into(),
            description: "Edit a file by replacing an exact substring match. By default old_string must match exactly once. Set replace_all=true to replace every occurrence.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to edit" },
                    "old_string": { "type": "string", "description": "Exact substring to find" },
                    "new_string": { "type": "string", "description": "Replacement string" },
                    "replace_all": { "type": "boolean", "description": "If true, replace all occurrences (default false)" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "search_files".into(),
            description: "Recursively search files for a pattern. Set is_regex=true to use a regular expression. Returns filename:line:content, max 100 matches.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "String or regex pattern to search for" },
                    "path": { "type": "string", "description": "Directory to search in" },
                    "is_regex": { "type": "boolean", "description": "Treat pattern as a regex (default false)" }
                },
                "required": ["pattern", "path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "replace_lines".into(),
            description: "Replace a range of lines in a file with new content. Lines are 1-based, inclusive.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "start_line": { "type": "integer", "description": "First line to replace (1-based)" },
                    "end_line": { "type": "integer", "description": "Last line to replace (1-based, inclusive)" },
                    "new_content": { "type": "string", "description": "Replacement text (replaces the specified line range)" }
                },
                "required": ["path", "start_line", "end_line", "new_content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "diff_files".into(),
            description: "Show a unified diff between two files.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path_a": { "type": "string", "description": "First file" },
                    "path_b": { "type": "string", "description": "Second file" }
                },
                "required": ["path_a", "path_b"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "append_file".into(),
            description: "Append content to the end of a file (creates the file if it doesn't exist).".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "content": { "type": "string", "description": "Content to append" }
                },
                "required": ["path", "content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "delete_path".into(),
            description: "Delete a file or empty directory.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to delete" }
                },
                "required": ["path"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "glob_files".into(),
            description: "Find files matching a glob pattern (e.g. '**/*.rs', 'src/*.txt'). Returns matching paths relative to the working directory. Max 200 results.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern to match" },
                    "path": { "type": "string", "description": "Base directory for the search (default '.')" }
                },
                "required": ["pattern"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "remember".into(),
            description: "Store or update a persistent memory. Use to save knowledge across sessions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Short name for this memory" },
                    "content": { "type": "string", "description": "The knowledge to remember" },
                    "description": { "type": "string", "description": "One-line description of what this memory contains" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tags for categorization" }
                },
                "required": ["name", "content"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "recall".into(),
            description: "Search stored memories by keyword. Returns matching memories with their content.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "keyword": { "type": "string", "description": "Search term to find in memory names, descriptions, tags, and content" }
                },
                "required": ["keyword"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "forget".into(),
            description: "Delete a stored memory by name.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the memory to delete" }
                },
                "required": ["name"]
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "list_memories".into(),
            description: "List all stored memories with their names and descriptions.".into(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
            source: ToolSource::BuiltIn,
        },
        ToolDefinition {
            name: "git_diff".into(),
            description: "Show git diff output for the working directory or a specific path.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Scope diff to a specific file or directory" },
                    "staged": { "type": "boolean", "description": "If true, show staged changes (--cached)" },
                    "ref": { "type": "string", "description": "Diff against a ref (commit hash, branch, tag, e.g. HEAD~3, main)" }
                }
            }),
            source: ToolSource::BuiltIn,
        },
    ]
}

fn resolve_safe(requested: &str, working_dir: &Path) -> Result<PathBuf, String> {
    let joined = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        working_dir.join(requested)
    };

    // For paths that don't exist yet (write_file), canonicalize the parent
    let canonical = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|e| format!("cannot resolve path: {e}"))?
    } else {
        // Canonicalize parent, then append filename
        let parent = joined.parent().unwrap_or(Path::new("."));
        let file_name = joined.file_name().ok_or("path has no filename")?;
        let canonical_parent = if parent.exists() {
            parent
                .canonicalize()
                .map_err(|e| format!("cannot resolve parent: {e}"))?
        } else {
            // parent doesn't exist either — resolve working_dir and build path
            working_dir
                .canonicalize()
                .map_err(|e| format!("cannot resolve working dir: {e}"))?
                .join(parent.strip_prefix(working_dir).unwrap_or(parent))
        };
        canonical_parent.join(file_name)
    };

    let wd = working_dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve working dir: {e}"))?;

    if !canonical.starts_with(&wd) {
        return Err(format!(
            "access denied: path '{}' is outside working directory",
            requested
        ));
    }
    Ok(canonical)
}

pub async fn execute_built_in(call: &ToolCall, working_dir: &Path) -> ToolResult {
    let output = match call.name.as_str() {
        "read_file" => run_read_file(call, working_dir).await,
        "write_file" => run_write_file(call, working_dir).await,
        "list_dir" => run_list_dir(call, working_dir).await,
        "edit_file" => run_edit_file(call, working_dir).await,
        "search_files" => run_search_files(call, working_dir).await,
        "replace_lines" => run_replace_lines(call, working_dir).await,
        "diff_files" => run_diff_files(call, working_dir).await,
        "append_file" => run_append_file(call, working_dir).await,
        "delete_path" => run_delete_path(call, working_dir).await,
        "glob_files" => run_glob_files(call, working_dir).await,
        "remember" => run_remember(call, working_dir).await,
        "recall" => run_recall(call, working_dir).await,
        "forget" => run_forget(call, working_dir).await,
        "list_memories" => run_list_memories(call, working_dir).await,
        "git_diff" => run_git_diff(call, working_dir).await,
        other => Err(format!("unknown built-in tool: {other}")),
    };

    match output {
        Ok(text) => ToolResult {
            call_id: call.id.clone(),
            output: text,
            is_error: false,
        },
        Err(msg) => ToolResult {
            call_id: call.id.clone(),
            output: msg,
            is_error: true,
        },
    }
}

async fn run_read_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let start = call.arguments["start_line"]
        .as_u64()
        .map(|n| (n as usize).saturating_sub(1));
    let end = call.arguments["end_line"].as_u64().map(|n| n as usize);
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_safe(&path_str, &working_dir)?;
        let content = std::fs::read_to_string(&path).map_err(|e| format!("read error: {e}"))?;

        let all_lines: Vec<&str> = content.lines().collect();
        let total = all_lines.len();

        let start = start.unwrap_or(0);
        let end = end.map(|n| n.min(total)).unwrap_or(total);

        if start >= total {
            return Err(format!("start_line {} exceeds file length ({} lines)", start + 1, total));
        }

        let slice = &all_lines[start..end.min(total)];
        let mut out = String::new();
        for (i, line) in slice.iter().enumerate() {
            let lineno = start + i + 1;
            out.push_str(&format!("{lineno:>6}\t{line}\n"));
        }

        const MAX_CHARS: usize = 50_000;
        if out.len() > MAX_CHARS {
            out.truncate(MAX_CHARS);
            out.push_str(&format!("\n[... truncated at {MAX_CHARS} chars, use start_line/end_line to read specific ranges ...]"));
        }

        if start > 0 || end < total {
            out.push_str(&format!("\n[showing lines {}-{} of {}]", start + 1, end.min(total), total));
        }

        Ok(out)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_write_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?;
    let content = call.arguments["content"]
        .as_str()
        .ok_or("missing 'content' argument")?;
    let path = resolve_safe(path_str, working_dir)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create dirs error: {e}"))?;
    }
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("write error: {e}"))?;
    Ok(format!(
        "wrote {} bytes to {}",
        content.len(),
        path.display()
    ))
}

async fn run_list_dir(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let max_depth = call.arguments["depth"]
        .as_u64()
        .map(|d| (d as usize).min(10))
        .unwrap_or(0);
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_safe(&path_str, &working_dir)?;
        let mut out = String::new();
        list_tree(&path, &mut out, 0, max_depth)?;
        Ok(out)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

fn fmt_modified(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .ok()
        .map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.format("%Y-%m-%d %H:%M").to_string()
        })
        .unwrap_or_default()
}

fn list_tree(path: &Path, out: &mut String, depth: usize, max_depth: usize) -> Result<(), String> {
    if depth > max_depth {
        return Ok(());
    }
    let entries = std::fs::read_dir(path).map_err(|e| format!("list error: {e}"))?;
    let mut items: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| !s.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    items.sort_by_key(|e| e.file_name());

    for entry in items {
        let indent = "  ".repeat(depth);
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if IGNORE_DIRS.contains(&name_str.as_ref()) {
            continue;
        }
        let meta = entry.metadata().map_err(|e| format!("stat error: {e}"))?;
        if meta.is_dir() {
            out.push_str(&format!("{indent}{name_str}/\n"));
            list_tree(&entry.path(), out, depth + 1, max_depth)?;
        } else {
            let size = meta.len();
            let modified = fmt_modified(&meta);
            out.push_str(&format!("{indent}{name_str} ({size} bytes, {modified})\n"));
        }
    }
    Ok(())
}

async fn run_edit_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let old_string = call.arguments["old_string"]
        .as_str()
        .ok_or("missing 'old_string' argument")?
        .to_string();
    let new_string = call.arguments["new_string"]
        .as_str()
        .ok_or("missing 'new_string' argument")?
        .to_string();
    let replace_all = call.arguments["replace_all"].as_bool().unwrap_or(false);

    if old_string.is_empty() {
        return Err("old_string must not be empty".into());
    }
    if old_string == new_string {
        return Err("old_string and new_string are identical — nothing to change".into());
    }

    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_safe(&path_str, &working_dir)?;
        let content = std::fs::read_to_string(&path).map_err(|e| format!("read error: {e}"))?;

        let match_count = content.matches(old_string.as_str()).count();
        match match_count {
            0 => {
                let first_line = old_string.lines().next().unwrap_or(&old_string);
                let partial: Vec<(usize, &str)> = content
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.contains(first_line.trim()))
                    .take(3)
                    .collect();
                if partial.is_empty() {
                    Err(format!(
                        "old_string not found in {}. Make sure whitespace and indentation match exactly.",
                        path_str
                    ))
                } else {
                    let hints: Vec<String> = partial
                        .iter()
                        .map(|(n, l)| format!("  line {}: {}", n + 1, l))
                        .collect();
                    Err(format!(
                        "old_string not found in {}. Partial matches on first line:\n{}",
                        path_str,
                        hints.join("\n")
                    ))
                }
            }
            1 => {
                let new_content = content.replacen(old_string.as_str(), new_string.as_str(), 1);
                std::fs::write(&path, &new_content).map_err(|e| format!("write error: {e}"))?;
                let old_lines = old_string.lines().count();
                let new_lines = new_string.lines().count();
                Ok(format!(
                    "edited {}: replaced {old_lines} line(s) with {new_lines} line(s)",
                    path_str
                ))
            }
            n if replace_all => {
                let new_content = content.replace(old_string.as_str(), new_string.as_str());
                std::fs::write(&path, &new_content).map_err(|e| format!("write error: {e}"))?;
                Ok(format!("edited {}: replaced {n} occurrence(s)", path_str))
            }
            n => Err(format!(
                "old_string matched {n} times in {} — must be unique. Add more surrounding context to old_string.",
                path_str
            )),
        }
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_search_files(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let pattern = call.arguments["pattern"]
        .as_str()
        .ok_or("missing 'pattern' argument")?
        .to_string();
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let is_regex = call.arguments["is_regex"].as_bool().unwrap_or(false);
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let root = resolve_safe(&path_str, &working_dir)?;
        let matcher: Box<dyn Fn(&str) -> bool + Send> = if is_regex {
            let re = Regex::new(&pattern).map_err(|e| format!("invalid regex: {e}"))?;
            Box::new(move |line: &str| re.is_match(line))
        } else {
            Box::new(move |line: &str| line.contains(pattern.as_str()))
        };
        let mut matches = Vec::new();
        search_recursive(&root, &matcher, &mut matches, 0)?;
        let count = matches.len();
        if matches.is_empty() {
            Ok("no matches found".into())
        } else {
            let truncated = if count >= 100 { " (truncated)" } else { "" };
            matches.push(format!("\n[{count} matches{truncated}]"));
            Ok(matches.join("\n"))
        }
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_replace_lines(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let start_line = call.arguments["start_line"]
        .as_u64()
        .ok_or("missing 'start_line' argument")? as usize;
    let end_line = call.arguments["end_line"]
        .as_u64()
        .ok_or("missing 'end_line' argument")? as usize;
    let new_content = call.arguments["new_content"]
        .as_str()
        .ok_or("missing 'new_content' argument")?
        .to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_safe(&path_str, &working_dir)?;
        let content = std::fs::read_to_string(&path).map_err(|e| format!("read error: {e}"))?;
        let mut lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if start_line < 1 || start_line > total {
            return Err(format!("start_line {start_line} out of range (file has {total} lines)"));
        }
        if end_line < start_line || end_line > total {
            return Err(format!("end_line {end_line} out of range (start={start_line}, total={total})"));
        }
        let old_count = end_line - start_line + 1;
        let new_lines: Vec<&str> = new_content.lines().collect();
        let new_count = new_lines.len();
        lines.splice((start_line - 1)..end_line, new_lines);
        let result = lines.join("\n");
        // preserve trailing newline if original had one
        let result = if content.ends_with('\n') {
            format!("{result}\n")
        } else {
            result
        };
        std::fs::write(&path, &result).map_err(|e| format!("write error: {e}"))?;
        Ok(format!(
            "replaced lines {start_line}-{end_line} ({old_count} lines) with {new_count} lines in {path_str}"
        ))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

enum DiffLine<'a> {
    Context(&'a str),
    Add(&'a str),
    Remove(&'a str),
}

fn lcs_diff<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<DiffLine<'a>> {
    let m = a.len();
    let n = b.len();
    // Build LCS table
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut result = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m || j < n {
        if i < m && j < n && a[i] == b[j] {
            result.push(DiffLine::Context(a[i]));
            i += 1;
            j += 1;
        } else if j < n && (i >= m || dp[i + 1][j] >= dp[i][j + 1]) {
            result.push(DiffLine::Add(b[j]));
            j += 1;
        } else {
            result.push(DiffLine::Remove(a[i]));
            i += 1;
        }
    }
    result
}

async fn run_diff_files(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_a = call.arguments["path_a"]
        .as_str()
        .ok_or("missing 'path_a' argument")?
        .to_string();
    let path_b = call.arguments["path_b"]
        .as_str()
        .ok_or("missing 'path_b' argument")?
        .to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let pa = resolve_safe(&path_a, &working_dir)?;
        let pb = resolve_safe(&path_b, &working_dir)?;
        let ca = std::fs::read_to_string(&pa).map_err(|e| format!("read error {path_a}: {e}"))?;
        let cb = std::fs::read_to_string(&pb).map_err(|e| format!("read error {path_b}: {e}"))?;
        let la: Vec<&str> = ca.lines().collect();
        let lb: Vec<&str> = cb.lines().collect();
        let diff = lcs_diff(&la, &lb);
        let mut out = format!("--- {path_a}\n+++ {path_b}\n");
        for (count, d) in diff.iter().enumerate() {
            if count >= 500 {
                out.push_str("[diff truncated at 500 lines]\n");
                break;
            }
            match d {
                DiffLine::Context(l) => {
                    out.push(' ');
                    out.push_str(l);
                    out.push('\n');
                }
                DiffLine::Add(l) => {
                    out.push('+');
                    out.push_str(l);
                    out.push('\n');
                }
                DiffLine::Remove(l) => {
                    out.push('-');
                    out.push_str(l);
                    out.push('\n');
                }
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_append_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let content = call.arguments["content"]
        .as_str()
        .ok_or("missing 'content' argument")?
        .to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_safe(&path_str, &working_dir)?;
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .map_err(|e| format!("open error: {e}"))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("write error: {e}"))?;
        Ok(format!("appended {} bytes to {path_str}", content.len()))
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_delete_path(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_safe(&path_str, &working_dir)?;
        let meta = std::fs::metadata(&path).map_err(|e| format!("stat error: {e}"))?;
        if meta.is_dir() {
            std::fs::remove_dir(&path)
                .map_err(|e| format!("delete error (directory must be empty): {e}"))?;
            Ok(format!("deleted directory {path_str}"))
        } else {
            std::fs::remove_file(&path).map_err(|e| format!("delete error: {e}"))?;
            Ok(format!("deleted file {path_str}"))
        }
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_glob_files(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let pattern = call.arguments["pattern"]
        .as_str()
        .ok_or("missing 'pattern' argument")?
        .to_string();
    let base_str = call.arguments["path"].as_str().unwrap_or(".").to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let base = resolve_safe(&base_str, &working_dir)?;
        let full_pattern = base.join(&pattern);
        let full_pattern_str = full_pattern.to_string_lossy().to_string();
        let mut results = Vec::new();
        let wd = working_dir
            .canonicalize()
            .map_err(|e| format!("cannot resolve working dir: {e}"))?;
        for entry in glob::glob(&full_pattern_str)
            .map_err(|e| format!("invalid glob pattern: {e}"))?
            .filter_map(|e| e.ok())
        {
            if results.len() >= 200 {
                break;
            }
            let rel = entry.strip_prefix(&wd).unwrap_or(&entry);
            results.push(rel.to_string_lossy().to_string());
        }
        if results.is_empty() {
            Ok("no matches found".into())
        } else {
            let count = results.len();
            let truncated = if count >= 200 {
                " (truncated at 200)"
            } else {
                ""
            };
            results.push(format!("[{count} file(s){truncated}]"));
            Ok(results.join("\n"))
        }
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_remember(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let name = call.arguments["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let content = call.arguments["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let description = call.arguments["description"]
        .as_str()
        .unwrap_or(&name)
        .to_string();
    let tags: Vec<String> = call.arguments["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        memory::write_memory(&wd, &name, &description, &tags, &content)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

async fn run_recall(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let keyword = call.arguments["keyword"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::recall_memories(&wd, &keyword))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

async fn run_forget(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let name = call.arguments["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::forget_memory(&wd, &name))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

async fn run_list_memories(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let wd = working_dir.to_path_buf();
    let _ = call;
    tokio::task::spawn_blocking(move || memory::list_memories(&wd))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

async fn run_git_diff(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path = call.arguments["path"].as_str().map(str::to_string);
    let staged = call.arguments["staged"].as_bool().unwrap_or(false);
    let git_ref = call.arguments["ref"].as_str().map(str::to_string);

    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("diff").current_dir(working_dir);
    if staged {
        cmd.arg("--cached");
    }
    if let Some(r) = git_ref {
        cmd.arg(r);
    }
    if let Some(p) = path {
        cmd.arg("--").arg(p);
    }

    let out = cmd
        .output()
        .await
        .map_err(|e| format!("failed to run git diff: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !stderr.is_empty() {
            return Err(stderr);
        }
    }

    let mut output = String::from_utf8_lossy(&out.stdout).to_string();
    const MAX: usize = 100_000;
    if output.len() > MAX {
        output.truncate(MAX);
        output.push_str("\n... [output truncated]");
    }
    if output.is_empty() {
        Ok("No differences found.".into())
    } else {
        Ok(output)
    }
}

static IGNORE_DIRS: &[&str] = &[
    "target",
    ".git",
    "node_modules",
    ".cache",
    "dist",
    "build",
    "__pycache__",
    ".idea",
    ".vscode",
];

fn search_recursive(
    path: &Path,
    matcher: &dyn Fn(&str) -> bool,
    matches: &mut Vec<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 20 || matches.len() >= 100 {
        return Ok(());
    }
    let entries = std::fs::read_dir(path).map_err(|e| format!("read dir error: {e}"))?;
    for entry in entries.filter_map(|e| e.ok()) {
        if matches.len() >= 100 {
            break;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }
        if IGNORE_DIRS.contains(&name_str.as_ref()) {
            continue;
        }
        let entry_path = entry.path();
        let meta = entry.metadata().map_err(|e| format!("stat error: {e}"))?;
        if meta.is_dir() {
            search_recursive(&entry_path, matcher, matches, depth + 1)?;
        } else if meta.is_file() {
            if let Ok(content) = std::fs::read_to_string(&entry_path) {
                for (lineno, line) in content.lines().enumerate() {
                    if matches.len() >= 100 {
                        break;
                    }
                    if matcher(line) {
                        matches.push(format!("{}:{}:{}", entry_path.display(), lineno + 1, line));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

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
    async fn read_file_sandbox_escape_blocked() {
        let dir = setup_dir();
        let call = make_call("read_file", json!({"path": "../../etc/passwd"}));
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error, "expected error, got: {}", result.output);
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
    async fn write_file_sandbox_escape_blocked() {
        let dir = setup_dir();
        let call = make_call(
            "write_file",
            json!({"path": "../../evil.txt", "content": "bad"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
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
    async fn search_files_sandbox_blocked() {
        let dir = setup_dir();
        let call = make_call(
            "search_files",
            json!({"pattern": "root", "path": "../../etc"}),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
    }

    #[test]
    fn built_in_tool_definitions_count() {
        let defs = built_in_tool_definitions();
        assert_eq!(defs.len(), 15);
        let names: Vec<_> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"write_file"));
        assert!(names.contains(&"list_dir"));
        assert!(names.contains(&"edit_file"));
        assert!(names.contains(&"search_files"));
        assert!(names.contains(&"replace_lines"));
        assert!(names.contains(&"diff_files"));
        assert!(names.contains(&"append_file"));
        assert!(names.contains(&"delete_path"));
        assert!(names.contains(&"glob_files"));
        assert!(names.contains(&"remember"));
        assert!(names.contains(&"recall"));
        assert!(names.contains(&"forget"));
        assert!(names.contains(&"list_memories"));
    }

    // --- edit_file tests ---
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
            json!({
                "path": "test.txt",
                "old_string": "goodbye",
                "new_string": "hi"
            }),
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
            json!({
                "path": "test.txt",
                "old_string": "aaa",
                "new_string": "bbb"
            }),
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
            json!({
                "path": "test.txt",
                "old_string": "aaa",
                "new_string": "bbb",
                "replace_all": true
            }),
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
            json!({
                "path": "test.txt",
                "old_string": "",
                "new_string": "x"
            }),
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
            json!({
                "path": "test.txt",
                "old_string": "hello",
                "new_string": "hello"
            }),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("identical"));
    }

    #[tokio::test]
    async fn edit_file_sandbox_escape_blocked() {
        let dir = setup_dir();
        let call = make_call(
            "edit_file",
            json!({
                "path": "../../etc/passwd",
                "old_string": "root",
                "new_string": "evil"
            }),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn edit_file_multiline_match() {
        let dir = setup_dir();
        let file = dir.path().join("test.rs");
        std::fs::write(&file, "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n").unwrap();
        let call = make_call(
            "edit_file",
            json!({
                "path": "test.rs",
                "old_string": "fn a() {\n    1\n}",
                "new_string": "fn a() {\n    42\n}"
            }),
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
            json!({
                "path": "test.rs",
                "old_string": "fn hello() {\n    wrong_content\n}",
                "new_string": "replaced"
            }),
        );
        let result = execute_built_in(&call, dir.path()).await;
        assert!(result.is_error);
        assert!(result.output.contains("Partial matches"));
        assert!(result.output.contains("fn hello()"));
    }

    // --- read_file line range tests ---
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
        // No "showing lines" footer when reading entire file
        assert!(!result.output.contains("[showing lines"));
    }

    // --- search_files match count ---
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

    // --- replace_lines tests ---
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

    // --- append_file tests ---
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

    // --- delete_path tests ---
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

    // --- glob_files tests ---
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

    // --- diff_files tests ---
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
}
