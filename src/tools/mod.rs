mod builtin;
mod definitions;
mod dispatch;
pub(crate) mod selection;

use std::path::{Component, Path, PathBuf};

/// Resolves a path and verifies it stays inside `working_dir` (lexically normalized,
/// canonicalized-root prefix check).
///
/// KNOWN LIMITATION: symlinks inside working_dir that point outside are NOT detected —
/// neither for relative nor absolute paths. `starts_with` passes because the lexical
/// path stays under the working_dir prefix; the OS resolves the symlink at open-time.
/// Accepted tradeoff: catching this would require canonicalizing the target path,
/// which fails for files being CREATED (write_file). If strict isolation is needed,
/// run the agent in a chroot or container.
pub(crate) fn resolve_safe(requested: &str, working_dir: &Path) -> Result<PathBuf, String> {
    let canonical_wd = working_dir
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize working directory: {e}"))?;

    let raw = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        canonical_wd.join(requested)
    };

    let normalized = normalize_path(&raw);

    if !normalized.starts_with(&canonical_wd) {
        return Err(format!("path escapes working directory: {requested}"));
    }

    Ok(normalized)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(components.last(), Some(Component::Normal(_))) {
                    components.pop();
                }
            }
            other => components.push(other),
        }
    }
    components.iter().collect()
}

pub(crate) const PLAN_WRITE_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "replace_lines",
    "append_file",
    "delete_path",
];

pub(crate) static IGNORE_DIRS: &[&str] = &[
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

pub use definitions::{
    built_in_tool_definitions, delegate_task_def, interview_question_def, update_plan_def,
};
pub(crate) use dispatch::execute_built_in_with_mode;
pub use selection::is_flat_model;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn accepts_nested_relative() {
        let dir = tmp();
        let result = resolve_safe("src/main.rs", dir.path()).unwrap();
        assert!(result.starts_with(dir.path().canonicalize().unwrap()));
        assert!(result.ends_with("src/main.rs"));
    }

    #[test]
    fn accepts_absolute_inside() {
        let dir = tmp();
        let canonical = dir.path().canonicalize().unwrap();
        let inside = canonical.join("foo.txt");
        let result = resolve_safe(inside.to_str().unwrap(), dir.path()).unwrap();
        assert_eq!(result, inside);
    }

    #[test]
    fn normalizes_redundant_dots() {
        let dir = tmp();
        let result = resolve_safe("./src/../src/./main.rs", dir.path()).unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        assert_eq!(result, canonical.join("src/main.rs"));
    }

    #[test]
    fn escape_via_dotdot() {
        let dir = tmp();
        let err = resolve_safe("../../etc/passwd", dir.path()).unwrap_err();
        assert!(err.contains("escapes working directory"));
    }

    #[test]
    fn escape_via_absolute_path_outside() {
        let dir = tmp();
        let err = resolve_safe("/etc/passwd", dir.path()).unwrap_err();
        assert!(err.contains("escapes working directory"));
    }
}
