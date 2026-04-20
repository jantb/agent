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

    let is_absolute = Path::new(requested).is_absolute();
    let raw = if is_absolute {
        PathBuf::from(requested)
    } else {
        canonical_wd.join(requested)
    };

    let normalized = normalize_path(&raw);

    if normalized.starts_with(&canonical_wd) {
        return Ok(normalized);
    }

    if is_absolute {
        if let Some(recovered) = suffix_match_fallback(&normalized, &canonical_wd) {
            return Ok(recovered);
        }
    }

    Err(format!(
        "path escapes working directory '{}': {requested}",
        canonical_wd.display()
    ))
}

/// When an absolute path has the wrong prefix but the WD basename appears in it,
/// recover by taking everything after the last occurrence of the WD basename.
fn suffix_match_fallback(abs: &Path, canonical_wd: &Path) -> Option<PathBuf> {
    let wd_base = canonical_wd.file_name()?;
    let components: Vec<Component> = abs.components().collect();
    // Find last Normal component equal to wd_base
    let idx = components
        .iter()
        .rposition(|c| matches!(c, Component::Normal(n) if *n == wd_base))?;
    let suffix: PathBuf = components[idx + 1..].iter().collect();
    let candidate = normalize_path(&canonical_wd.join(suffix));
    if candidate.starts_with(canonical_wd) {
        Some(candidate)
    } else {
        None
    }
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

/// Returns true if any component in `rel` (relative path) starts with '.'
/// or is in IGNORE_DIRS. Used to skip hidden/ignored directories in listings.
pub(crate) fn is_ignored_path(rel: &Path) -> bool {
    rel.components().any(|c| {
        if let Component::Normal(s) = c {
            let name = s.to_string_lossy();
            name.starts_with('.') || IGNORE_DIRS.contains(&name.as_ref())
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn is_ignored_path_detects_dot_dirs() {
        assert!(is_ignored_path(Path::new(".git/config")));
        assert!(is_ignored_path(Path::new("src/.cache/foo")));
        assert!(is_ignored_path(Path::new(".DS_Store")));
    }

    #[test]
    fn is_ignored_path_detects_ignore_dirs() {
        assert!(is_ignored_path(Path::new("target/debug/foo")));
        assert!(is_ignored_path(Path::new("node_modules/x")));
    }

    #[test]
    fn is_ignored_path_passes_normal() {
        assert!(!is_ignored_path(Path::new("src/main.rs")));
        assert!(!is_ignored_path(Path::new("Cargo.toml")));
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

    #[test]
    fn recovers_absolute_path_with_matching_wd_basename() {
        let dir = tmp();
        let canonical = dir.path().canonicalize().unwrap();
        let wd_base = canonical.file_name().unwrap().to_str().unwrap();
        let bogus = format!("/bogus/prefix/{wd_base}/Cargo.toml");
        let result = resolve_safe(&bogus, dir.path()).unwrap();
        assert_eq!(result, canonical.join("Cargo.toml"));
    }

    #[test]
    fn recovers_bare_wd_via_basename() {
        let dir = tmp();
        let canonical = dir.path().canonicalize().unwrap();
        let wd_base = canonical.file_name().unwrap().to_str().unwrap();
        let bogus = format!("/bogus/{wd_base}");
        let result = resolve_safe(&bogus, dir.path()).unwrap();
        assert_eq!(result, canonical);
    }

    #[test]
    fn rejects_unrecoverable_absolute_path() {
        let dir = tmp();
        let err = resolve_safe("/etc/passwd", dir.path()).unwrap_err();
        assert!(err.contains("escapes working directory"));
    }

    #[test]
    fn rejects_escape_via_dotdot_even_with_basename_in_suffix() {
        let dir = tmp();
        let canonical = dir.path().canonicalize().unwrap();
        let wd_base = canonical.file_name().unwrap().to_str().unwrap();
        let bogus = format!("../../{wd_base}/evil");
        let err = resolve_safe(&bogus, dir.path()).unwrap_err();
        assert!(err.contains("escapes working directory"));
    }
}
