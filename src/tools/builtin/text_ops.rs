use std::path::Path;

use crate::types::ToolCall;

use super::resolve_path;

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
            return Err(format!(
                "start_line {start_line} out of range (file has {total} lines)"
            ));
        }
        if end_line < start_line || end_line > total {
            return Err(format!(
                "end_line {end_line} out of range (start={start_line}, total={total})"
            ));
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
