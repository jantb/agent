use std::path::Path;

use crate::types::ToolCall;

const MAX_OUTPUT: usize = 16_000;

pub async fn run_cargo_test(_call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let output = tokio::process::Command::new("cargo")
        .arg("test")
        .current_dir(working_dir)
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .await
        .map_err(|e| format!("failed to spawn cargo: {e}"))?;

    let status = if output.status.success() {
        "pass"
    } else {
        "fail"
    };
    let code = output.status.code().unwrap_or(-1);
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let truncated = if combined.len() > MAX_OUTPUT {
        let cut = combined.len() - MAX_OUTPUT;
        format!(
            "[truncated: {cut} chars elided from head]\n{}",
            &combined[combined.len() - MAX_OUTPUT..]
        )
    } else {
        combined
    };

    Ok(format!(
        "cargo test: {status} (exit {code})\n---\n{truncated}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn sample_crate(dir: &Path, passing: bool) {
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname=\"t\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\npath=\"src/lib.rs\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let body = if passing {
            "#[test] fn ok() { assert_eq!(1, 1); }"
        } else {
            "#[test] fn bad() { assert_eq!(1, 2); }"
        };
        std::fs::write(dir.join("src/lib.rs"), body).unwrap();
    }

    #[tokio::test]
    async fn pass_case_reports_pass() {
        let dir = TempDir::new().unwrap();
        sample_crate(dir.path(), true);
        let call = ToolCall {
            id: "1".into(),
            name: "cargo_test".into(),
            arguments: json!({}),
        };
        let out = run_cargo_test(&call, dir.path()).await.unwrap();
        assert!(out.contains("cargo test: pass"), "{out}");
    }

    #[tokio::test]
    async fn fail_case_reports_fail() {
        let dir = TempDir::new().unwrap();
        sample_crate(dir.path(), false);
        let call = ToolCall {
            id: "1".into(),
            name: "cargo_test".into(),
            arguments: json!({}),
        };
        let out = run_cargo_test(&call, dir.path()).await.unwrap();
        assert!(out.contains("cargo test: fail"), "{out}");
    }
}
