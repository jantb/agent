use std::path::{Path, PathBuf};

use regex::Regex;

use crate::memory;
use crate::types::ToolCall;

use super::IGNORE_DIRS;

fn resolve_path(requested: &str, working_dir: &Path) -> PathBuf {
    let p = Path::new(requested);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        working_dir.join(p)
    }
}

pub async fn run_read_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let path = resolve_path(&path_str, &working_dir);
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

pub async fn run_write_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?;
    let content = call.arguments["content"]
        .as_str()
        .ok_or("missing 'content' argument")?;
    let path = resolve_path(path_str, working_dir);
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

pub async fn run_list_dir(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let path = resolve_path(&path_str, &working_dir);
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

pub async fn run_edit_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let path = resolve_path(&path_str, &working_dir);
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

pub async fn run_search_files(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let root = resolve_path(&path_str, &working_dir);
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

pub async fn run_replace_lines(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let path = resolve_path(&path_str, &working_dir);
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

pub async fn run_diff_files(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let pa = resolve_path(&path_a, &working_dir);
        let pb = resolve_path(&path_b, &working_dir);
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

pub async fn run_append_file(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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
        let path = resolve_path(&path_str, &working_dir);
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

pub async fn run_delete_path(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let path_str = call.arguments["path"]
        .as_str()
        .ok_or("missing 'path' argument")?
        .to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let path = resolve_path(&path_str, &working_dir);
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

pub async fn run_glob_files(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let pattern = call.arguments["pattern"]
        .as_str()
        .ok_or("missing 'pattern' argument")?
        .to_string();
    let base_str = call.arguments["path"].as_str().unwrap_or(".").to_string();
    let working_dir = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let base = resolve_path(&base_str, &working_dir);
        let full_pattern = base.join(&pattern);
        let full_pattern_str = full_pattern.to_string_lossy().to_string();
        let mut results = Vec::new();
        let wd = working_dir
            .canonicalize()
            .unwrap_or_else(|_| working_dir.clone());
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

pub async fn run_remember(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
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

pub async fn run_recall(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let keyword = call.arguments["keyword"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::recall_memories(&wd, &keyword))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub async fn run_forget(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let name = call.arguments["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::forget_memory(&wd, &name))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub async fn run_list_memories(_call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::list_memories(&wd))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

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
