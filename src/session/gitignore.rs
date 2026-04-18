use std::{fs, path::Path};

use anyhow::Context;

pub fn ensure_gitignore(working_dir: &Path) -> anyhow::Result<()> {
    let path = working_dir.join(".gitignore");
    let entry = ".agent/\n";
    if path.exists() {
        let contents =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        if !contents.lines().any(|l| l.trim() == ".agent/") {
            let mut updated = contents;
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(entry);
            fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
        }
    } else {
        fs::write(&path, entry).with_context(|| format!("creating {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn ensure_gitignore_creates_file() {
        let dir = tmp();
        ensure_gitignore(dir.path()).unwrap();
        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(contents.contains(".agent/"));
    }

    #[test]
    fn ensure_gitignore_appends_if_missing() {
        let dir = tmp();
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        ensure_gitignore(dir.path()).unwrap();
        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(contents.contains("target/"));
        assert!(contents.contains(".agent/"));
    }

    #[test]
    fn ensure_gitignore_no_duplicate() {
        let dir = tmp();
        fs::write(dir.path().join(".gitignore"), ".agent/\n").unwrap();
        ensure_gitignore(dir.path()).unwrap();
        let contents = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(contents.matches(".agent/").count(), 1);
    }
}
