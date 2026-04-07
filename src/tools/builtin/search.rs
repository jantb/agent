use std::path::Path;

use regex::Regex;

use crate::tools::IGNORE_DIRS;
use crate::types::ToolCall;

use super::resolve_path;

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
