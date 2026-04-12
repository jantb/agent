use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;

#[derive(Debug, PartialEq)]
pub enum ScriptCommand {
    Send(String),
    Wait(Duration),
    ExpectFile {
        path: String,
        content: String,
    },
    ExpectNoFile(String),
    /// Assert that a named event appeared in the most recent step's event log.
    ExpectEvent(String),
    /// Assert that a named event did NOT appear in the most recent step's event log.
    ExpectNoEvent(String),
    ExpectStat {
        lhs: String,
        op: String,
        rhs: String,
    },
}

pub fn parse_script(path: &Path) -> Result<Vec<ScriptCommand>> {
    let src =
        fs::read_to_string(path).with_context(|| format!("reading script {}", path.display()))?;
    let mut cmds = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@wait ") {
            let secs: f64 = rest
                .trim()
                .parse()
                .with_context(|| format!("line {}: invalid @wait value {:?}", i + 1, rest))?;
            cmds.push(ScriptCommand::Wait(Duration::from_secs_f64(secs)));
        } else if let Some(rest) = line.strip_prefix("@expect_file ") {
            let (path, content) = rest.split_once(' ').with_context(|| {
                format!("line {}: @expect_file requires path and content", i + 1)
            })?;
            cmds.push(ScriptCommand::ExpectFile {
                path: path.to_owned(),
                content: content.to_owned(),
            });
        } else if let Some(rest) = line.strip_prefix("@expect_no_file ") {
            cmds.push(ScriptCommand::ExpectNoFile(rest.trim().to_owned()));
        } else if let Some(rest) = line.strip_prefix("@expect_no_event ") {
            cmds.push(ScriptCommand::ExpectNoEvent(rest.trim().to_owned()));
        } else if let Some(rest) = line.strip_prefix("@expect_event ") {
            cmds.push(ScriptCommand::ExpectEvent(rest.trim().to_owned()));
        } else if let Some(rest) = line.strip_prefix("@expect_stat ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() != 3 {
                bail!(
                    "line {}: @expect_stat requires '<field> <op> <field>'",
                    i + 1
                );
            }
            cmds.push(ScriptCommand::ExpectStat {
                lhs: parts[0].to_owned(),
                op: parts[1].to_owned(),
                rhs: parts[2].to_owned(),
            });
        } else if line.starts_with('@') {
            bail!("line {}: unknown directive {:?}", i + 1, line);
        } else {
            cmds.push(ScriptCommand::Send(line.to_owned()));
        }
    }
    Ok(cmds)
}

/// Per-step token usage broken down by agent depth.
/// Depths are determined by tracking SubtaskEnter/SubtaskExit events.
#[derive(Debug, Serialize, Default)]
pub struct TokenSummary {
    /// Max prompt tokens seen in any single turn at depth 0 (orchestrator).
    pub orchestrator_prompt_max: u64,
    /// Max prompt tokens seen in any single turn at depth 1+ (subtasks).
    pub subtask_prompt_max: u64,
    /// Total generated tokens across all depths.
    pub total_eval: u64,
}

impl TokenSummary {
    pub fn resolve_field(&self, name: &str) -> Option<u64> {
        match name {
            "orchestrator_prompt_max" => Some(self.orchestrator_prompt_max),
            "subtask_prompt_max" => Some(self.subtask_prompt_max),
            "total_eval" => Some(self.total_eval),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StepReport {
    pub command: String,
    pub events: Vec<String>,
    pub duration_ms: u64,
    pub status: StepStatus,
    pub token_summary: TokenSummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Completed,
    TimedOut,
    Failed(String),
}

#[derive(Debug, Serialize)]
pub struct AssertionResult {
    pub assert_type: String,
    pub path: String,
    pub expected: String,
    pub actual: String,
    pub pass: bool,
}

#[derive(Debug, Serialize)]
pub struct TestReport {
    pub model: String,
    pub steps: Vec<StepReport>,
    pub assertions: Vec<AssertionResult>,
}

impl TestReport {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_owned(),
            steps: Vec::new(),
            assertions: Vec::new(),
        }
    }

    pub fn add_step(&mut self, step: StepReport) {
        self.steps.push(step);
    }

    pub fn add_assertion(&mut self, assertion: AssertionResult) {
        self.assertions.push(assertion);
    }

    pub fn write_to_file(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json).with_context(|| format!("writing report to {}", path.display()))
    }

    pub fn print_summary(&self) {
        let passed = self.assertions.iter().filter(|a| a.pass).count();
        let failed = self.assertions.iter().filter(|a| !a.pass).count();
        let timed_out = self
            .steps
            .iter()
            .filter(|s| matches!(s.status, StepStatus::TimedOut))
            .count();
        println!(
            "assertions: {} passed, {} failed | steps timed out: {}",
            passed, failed, timed_out
        );
        for (i, step) in self.steps.iter().enumerate() {
            let t = &step.token_summary;
            if t.total_eval > 0 {
                println!(
                    "step {i} tokens: orchestrator_prompt_max={} subtask_prompt_max={} total_eval={}",
                    t.orchestrator_prompt_max, t.subtask_prompt_max, t.total_eval
                );
            }
        }
    }
}

pub fn run_assertion(cmd: &ScriptCommand, working_dir: &Path) -> Option<AssertionResult> {
    match cmd {
        ScriptCommand::ExpectFile { path, content } => {
            let full = working_dir.join(path);
            let actual = fs::read_to_string(&full).unwrap_or_default();
            let pass = actual.contains(content.trim());
            Some(AssertionResult {
                assert_type: "expect_file".into(),
                path: path.clone(),
                expected: content.clone(),
                actual,
                pass,
            })
        }
        ScriptCommand::ExpectNoFile(path) => {
            let full = working_dir.join(path);
            let exists = full.exists();
            Some(AssertionResult {
                assert_type: "expect_no_file".into(),
                path: path.clone(),
                expected: String::new(),
                actual: if exists {
                    "exists".into()
                } else {
                    String::new()
                },
                pass: !exists,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn parse_str(s: &str) -> Vec<ScriptCommand> {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(s.as_bytes()).unwrap();
        parse_script(f.path()).unwrap()
    }

    #[test]
    fn skips_comments_and_empty_lines() {
        let cmds = parse_str("# comment\n\n  \n# another");
        assert!(cmds.is_empty());
    }

    #[test]
    fn parses_send() {
        let cmds = parse_str("hello world");
        assert_eq!(cmds, vec![ScriptCommand::Send("hello world".into())]);
    }

    #[test]
    fn parses_wait() {
        let cmds = parse_str("@wait 2.5");
        assert_eq!(cmds, vec![ScriptCommand::Wait(Duration::from_millis(2500))]);
    }

    #[test]
    fn parses_expect_file() {
        let cmds = parse_str("@expect_file foo.txt hello");
        assert_eq!(
            cmds,
            vec![ScriptCommand::ExpectFile {
                path: "foo.txt".into(),
                content: "hello".into(),
            }]
        );
    }

    #[test]
    fn parses_expect_no_file() {
        let cmds = parse_str("@expect_no_file missing.txt");
        assert_eq!(
            cmds,
            vec![ScriptCommand::ExpectNoFile("missing.txt".into())]
        );
    }

    #[test]
    fn mixed_script() {
        let src = "# setup\nsend this\n\n@wait 1\n@expect_file out.txt done\n@expect_no_file tmp";
        let cmds = parse_str(src);
        assert_eq!(cmds.len(), 4);
        assert!(matches!(&cmds[0], ScriptCommand::Send(s) if s == "send this"));
        assert!(matches!(&cmds[1], ScriptCommand::Wait(d) if *d == Duration::from_secs(1)));
        assert!(
            matches!(&cmds[2], ScriptCommand::ExpectFile { path, content } if path == "out.txt" && content == "done")
        );
        assert!(matches!(&cmds[3], ScriptCommand::ExpectNoFile(p) if p == "tmp"));
    }

    #[test]
    fn parses_expect_event() {
        let cmds = parse_str("@expect_event SubtaskEnter");
        assert_eq!(
            cmds,
            vec![ScriptCommand::ExpectEvent("SubtaskEnter".into())]
        );
    }

    #[test]
    fn parses_expect_stat() {
        let cmds = parse_str("@expect_stat orchestrator_prompt_max < subtask_prompt_max");
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            &cmds[0],
            ScriptCommand::ExpectStat { lhs, op, rhs }
            if lhs == "orchestrator_prompt_max" && op == "<" && rhs == "subtask_prompt_max"
        ));
    }

    #[test]
    fn parses_expect_no_event() {
        let cmds = parse_str("@expect_no_event InterviewQuestion");
        assert_eq!(
            cmds,
            vec![ScriptCommand::ExpectNoEvent("InterviewQuestion".into())]
        );
    }

    #[test]
    fn unknown_directive_errors() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(b"@unknown foo").unwrap();
        assert!(parse_script(f.path()).is_err());
    }
}
