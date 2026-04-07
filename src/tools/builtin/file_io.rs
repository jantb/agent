use std::path::Path;

use crate::tools::IGNORE_DIRS;
use crate::types::ToolCall;

use super::resolve_path;

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
            return Err(format!(
                "start_line {} exceeds file length ({} lines)",
                start + 1,
                total
            ));
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
            out.push_str(&format!(
                "\n[... truncated at {MAX_CHARS} chars, use start_line/end_line to read specific ranges ...]"
            ));
        }

        if start > 0 || end < total {
            out.push_str(&format!(
                "\n[showing lines {}-{} of {}]",
                start + 1,
                end.min(total),
                total
            ));
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
