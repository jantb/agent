use std::{
    fs,
    path::{Path, PathBuf},
};

use tracing::{debug, warn};

#[allow(dead_code)]
pub struct MemoryEntry {
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub content: String,
}

pub fn slugify(name: &str) -> String {
    let raw: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    // dedup consecutive '-'
    let mut slug = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for c in raw.chars() {
        if c == '-' {
            if !prev_dash {
                slug.push(c);
            }
            prev_dash = true;
        } else {
            slug.push(c);
            prev_dash = false;
        }
    }
    slug.trim_matches('-').to_string()
}

pub fn memory_dir(working_dir: &Path) -> PathBuf {
    working_dir.join(".agent").join("memory")
}

pub fn write_memory(
    working_dir: &Path,
    name: &str,
    description: &str,
    tags: &[String],
    content: &str,
) -> Result<String, String> {
    let dir = memory_dir(working_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("create memory dir: {e}"))?;
    let slug = slugify(name);
    let path = dir.join(format!("{slug}.md"));
    let tags_str = tags.join(", ");
    let file = format!(
        "---\nname: {name}\ndescription: {description}\ntags: [{tags_str}]\n---\n\n{content}"
    );
    fs::write(&path, file).map_err(|e| format!("write memory: {e}"))?;
    Ok(format!("Memory '{name}' saved to {}", path.display()))
}

pub fn read_memory_file(path: &Path) -> Result<MemoryEntry, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    parse_memory(&raw)
}

fn parse_memory(raw: &str) -> Result<MemoryEntry, String> {
    // Expect file to start with "---\n"
    let rest = raw
        .strip_prefix("---\n")
        .ok_or("missing frontmatter opening ---")?;
    let (front, body) = rest
        .split_once("\n---\n")
        .ok_or("missing frontmatter closing ---")?;

    let mut name = String::new();
    let mut description = String::new();
    let mut tags = Vec::new();

    for line in front.lines() {
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("tags:") {
            let inner = v.trim().trim_matches(|c| c == '[' || c == ']');
            tags = inner
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
        }
    }

    let content = body.trim_start_matches('\n').to_string();
    Ok(MemoryEntry {
        name,
        description,
        tags,
        content,
    })
}

pub fn recall_memories(working_dir: &Path, keyword: &str) -> Result<String, String> {
    let dir = memory_dir(working_dir);
    if !dir.exists() {
        return Ok(format!("No memories found matching '{keyword}'."));
    }
    let kw = keyword.to_lowercase();
    let mut results = Vec::new();

    for entry in fs::read_dir(&dir).map_err(|e| format!("read dir: {e}"))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if raw.to_lowercase().contains(&kw) {
            if let Ok(mem) = parse_memory(&raw) {
                results.push(format!(
                    "**{}**\n{}\n\n{}",
                    mem.name, mem.description, mem.content
                ));
            }
        }
    }

    if results.is_empty() {
        Ok(format!("No memories found matching '{keyword}'."))
    } else {
        Ok(results.join("\n\n---\n\n"))
    }
}

pub fn forget_memory(working_dir: &Path, name: &str) -> Result<String, String> {
    let path = memory_dir(working_dir).join(format!("{}.md", slugify(name)));
    if !path.exists() {
        return Err(format!("Memory '{name}' not found."));
    }
    fs::remove_file(&path).map_err(|e| format!("delete memory: {e}"))?;
    Ok(format!("Memory '{name}' deleted."))
}

pub fn list_memories(working_dir: &Path) -> Result<String, String> {
    let dir = memory_dir(working_dir);
    if !dir.exists() {
        return Ok("No memories stored yet.".into());
    }
    let mut lines = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| format!("read dir: {e}"))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if let Ok(mem) = read_memory_file(&path) {
            lines.push(format!("- **{}**: {}", mem.name, mem.description));
        }
    }
    if lines.is_empty() {
        Ok("No memories stored yet.".into())
    } else {
        lines.sort();
        Ok(lines.join("\n"))
    }
}

pub fn build_memory_index(working_dir: &Path) -> String {
    let dir = memory_dir(working_dir);
    if !dir.exists() {
        debug!(dir = %dir.display(), "memory dir does not exist yet, skipping index");
        return String::new();
    }
    let rd = match fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => {
            warn!(dir = %dir.display(), error = %e, "failed to read memory dir");
            return String::new();
        }
    };
    let mut lines: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
        .take(50)
        .filter_map(|e| {
            let path = e.path();
            match read_memory_file(&path) {
                Ok(m) => Some(m),
                Err(err) => {
                    warn!(path = %path.display(), error = %err, "failed to parse memory file");
                    None
                }
            }
        })
        .map(|m| format!("- **{}**: {}", m.name, m.description))
        .collect();
    lines.sort();
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Rust async tips!"), "rust-async-tips");
    }

    #[test]
    fn slugify_dedup_dashes() {
        assert_eq!(slugify("foo  bar"), "foo-bar");
    }

    #[test]
    fn slugify_trim_edges() {
        assert_eq!(slugify("  hello "), "hello");
    }

    #[test]
    fn write_and_read_roundtrip() {
        let dir = tmp();
        let tags = vec!["rust".into(), "async".into()];
        write_memory(dir.path(), "Rust tips", "Tips for Rust", &tags, "body here").unwrap();
        let path = memory_dir(dir.path()).join("rust-tips.md");
        let mem = read_memory_file(&path).unwrap();
        assert_eq!(mem.name, "Rust tips");
        assert_eq!(mem.description, "Tips for Rust");
        assert_eq!(mem.tags, vec!["rust", "async"]);
        assert_eq!(mem.content, "body here");
    }

    #[test]
    fn recall_finds_match() {
        let dir = tmp();
        let tags = vec!["rust".into()];
        write_memory(dir.path(), "Rust tips", "desc", &tags, "tokio patterns").unwrap();
        let result = recall_memories(dir.path(), "tokio").unwrap();
        assert!(result.contains("Rust tips"));
    }

    #[test]
    fn recall_no_match() {
        let dir = tmp();
        let tags: Vec<String> = vec![];
        write_memory(dir.path(), "Go tips", "desc", &tags, "goroutines").unwrap();
        let result = recall_memories(dir.path(), "tokio").unwrap();
        assert!(result.contains("No memories found"));
    }

    #[test]
    fn forget_removes_file() {
        let dir = tmp();
        write_memory(dir.path(), "Old tip", "x", &[], "content").unwrap();
        forget_memory(dir.path(), "Old tip").unwrap();
        let path = memory_dir(dir.path()).join("old-tip.md");
        assert!(!path.exists());
    }

    #[test]
    fn list_memories_empty() {
        let dir = tmp();
        assert_eq!(
            list_memories(dir.path()).unwrap(),
            "No memories stored yet."
        );
    }

    #[test]
    fn list_memories_entries() {
        let dir = tmp();
        write_memory(dir.path(), "Alpha", "first", &[], "a").unwrap();
        write_memory(dir.path(), "Beta", "second", &[], "b").unwrap();
        let result = list_memories(dir.path()).unwrap();
        assert!(result.contains("**Alpha**"));
        assert!(result.contains("**Beta**"));
    }

    #[test]
    fn build_index_empty_when_no_dir() {
        let dir = tmp();
        assert_eq!(build_memory_index(dir.path()), "");
    }

    #[test]
    fn build_index_lists_entries() {
        let dir = tmp();
        write_memory(dir.path(), "Tip A", "desc a", &[], "body").unwrap();
        let idx = build_memory_index(dir.path());
        assert!(idx.contains("**Tip A**"));
    }

    #[test]
    fn recall_missing_dir_returns_no_matches() {
        let dir = tmp();
        let result = recall_memories(dir.path(), "anything").unwrap();
        assert!(result.contains("No memories found"));
    }

    #[test]
    fn parse_tags_with_spaces() {
        let dir = tmp();
        let tags = vec!["a".into(), "b c".into(), "d".into()];
        write_memory(dir.path(), "t", "d", &tags, "body").unwrap();
        let path = memory_dir(dir.path()).join("t.md");
        let mem = read_memory_file(&path).unwrap();
        assert_eq!(mem.tags, vec!["a", "b c", "d"]);
    }
}
