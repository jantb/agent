// NOTE: AgentEvent handling is intentionally duplicated across headless.rs and tui/events.rs.
// When adding a new AgentEvent variant, update BOTH.

use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::agent::{is_flat_model, UserAction};
use crate::script::{self, ScriptCommand, StepReport, StepStatus, TestReport, TokenSummary};
use crate::types::AgentEvent;

/// Run the headless script mode.  Consumes `event_rx` / `action_tx` from bootstrap.
pub async fn run_script(
    script_path: &Path,
    model: &str,
    log_dir: &Path,
    working_dir: &Path,
    event_rx: &mut mpsc::Receiver<AgentEvent>,
    action_tx: &mpsc::Sender<UserAction>,
) -> anyhow::Result<()> {
    let commands = script::parse_script(script_path)?;
    let mut report = TestReport::new(model);
    let mut mode = crate::types::AgentMode::default();
    let mut flat = is_flat_model(model);

    for cmd in &commands {
        match cmd {
            ScriptCommand::Send(text) => {
                let step_start = std::time::Instant::now();

                if text.trim() == "/mode" {
                    mode = mode.cycle();
                    println!("[mode] {}", mode.label());
                    continue;
                }
                if text.trim() == "/flat" {
                    flat = !flat;
                    println!("[flat] {}", if flat { "on" } else { "off" });
                    action_tx.send(UserAction::ToggleFlat(flat)).await.ok();
                    loop {
                        match event_rx.recv().await {
                            Some(AgentEvent::TurnDone) | None => break,
                            _ => {}
                        }
                    }
                    continue;
                }
                if text.trim() == "/clear" || text.trim() == "/new" {
                    println!("[clear]");
                    action_tx.send(UserAction::ClearHistory).await.ok();
                    loop {
                        match event_rx.recv().await {
                            Some(AgentEvent::TurnDone) | None => break,
                            _ => {}
                        }
                    }
                    continue;
                }

                let action = UserAction::SendMessage {
                    text: text.clone(),
                    images: vec![],
                    mode,
                };
                action_tx.send(action).await.ok();
                println!("[send] {text}");

                let result = tokio::time::timeout(Duration::from_secs(240), async {
                    let mut log: Vec<String> = Vec::new();
                    let mut current_depth: usize = 0;
                    let mut orch_prompt_max: u64 = 0;
                    let mut subtask_prompt_max: u64 = 0;
                    let mut total_eval: u64 = 0;
                    loop {
                        match event_rx.recv().await {
                            Some(AgentEvent::ThinkingStarted) => {
                                println!("[event] ThinkingStarted");
                                log.push("ThinkingStarted".into());
                            }
                            Some(AgentEvent::ThinkingDelta(d)) => {
                                print!("[think] {d}");
                                log.push(format!("ThinkingDelta: {d}"));
                            }
                            Some(AgentEvent::ThinkingDone) => {
                                println!("\n[event] ThinkingDone");
                                log.push("ThinkingDone".into());
                            }
                            Some(AgentEvent::TextDelta(d)) => {
                                print!("{d}");
                                log.push(format!("TextDelta: {d}"));
                            }
                            Some(AgentEvent::ToolRequested(c)) => {
                                println!("\n[tool] {}({})", c.name, c.arguments);
                                log.push(format!("ToolRequested: {}({})", c.name, c.arguments));
                            }
                            Some(AgentEvent::ToolCompleted(r)) => {
                                println!("[tool_done] {}", r.call_id);
                                log.push(format!("ToolCompleted: {}", r.call_id));
                            }
                            Some(AgentEvent::TurnStats {
                                eval_count,
                                eval_duration_ns,
                                prompt_eval_count,
                            }) => {
                                log.push(format!(
                                    "TurnStats: depth={current_depth} eval={eval_count} prompt={prompt_eval_count} ns={eval_duration_ns}"
                                ));
                                total_eval += eval_count;
                                if current_depth == 0 {
                                    orch_prompt_max = orch_prompt_max.max(prompt_eval_count);
                                } else {
                                    subtask_prompt_max = subtask_prompt_max.max(prompt_eval_count);
                                }
                            }
                            Some(AgentEvent::TurnDone) => {
                                println!("\n[done]");
                                log.push("TurnDone".into());
                                return (
                                    StepStatus::Completed,
                                    log,
                                    orch_prompt_max,
                                    subtask_prompt_max,
                                    total_eval,
                                );
                            }
                            Some(AgentEvent::Error(e)) => {
                                println!("\n[error] {e}");
                                log.push(format!("Error: {e}"));
                                return (
                                    StepStatus::Failed(e),
                                    log,
                                    orch_prompt_max,
                                    subtask_prompt_max,
                                    total_eval,
                                );
                            }
                            Some(AgentEvent::LoopDetected) => {
                                let msg = "Loop detected";
                                println!("\n[error] {msg}");
                                log.push(format!("Error: {msg}"));
                                return (
                                    StepStatus::Failed(msg.into()),
                                    log,
                                    orch_prompt_max,
                                    subtask_prompt_max,
                                    total_eval,
                                );
                            }
                            Some(AgentEvent::SubtaskEnter { depth, label }) => {
                                println!("[subtask_enter] depth={depth} {label}");
                                log.push(format!("SubtaskEnter: depth={depth} {label}"));
                                current_depth = depth;
                            }
                            Some(AgentEvent::SubtaskExit { depth }) => {
                                println!("[subtask_exit] depth={depth}");
                                log.push(format!("SubtaskExit: depth={depth}"));
                                current_depth = depth.saturating_sub(1);
                            }
                            Some(AgentEvent::InterviewQuestion {
                                question,
                                suggestions,
                                answer_tx,
                            }) => {
                                let answer =
                                    suggestions.into_iter().next().unwrap_or_else(|| "yes".into());
                                println!("[clarify] Q: {question} -> A: {answer}");
                                log.push(format!("InterviewQuestion: {question}"));
                                let _ = answer_tx.0.send(answer);
                            }
                            Some(AgentEvent::PlanUpdated(items)) => {
                                println!("[plan_updated] {} items", items.len());
                                log.push(format!("PlanUpdated: {} items", items.len()));
                            }
                            None => {
                                return (
                                    StepStatus::Failed("channel closed".into()),
                                    log,
                                    orch_prompt_max,
                                    subtask_prompt_max,
                                    total_eval,
                                );
                            }
                        }
                    }
                })
                .await;

                let (status, events_log, orch_prompt_max, subtask_prompt_max, total_eval) =
                    match result {
                        Ok(tuple) => tuple,
                        Err(_) => {
                            println!("[timeout] step exceeded 240s");
                            (StepStatus::TimedOut, vec![], 0, 0, 0)
                        }
                    };

                report.add_step(StepReport {
                    command: text.clone(),
                    events: events_log,
                    duration_ms: step_start.elapsed().as_millis() as u64,
                    status,
                    token_summary: TokenSummary {
                        orchestrator_prompt_max: orch_prompt_max,
                        subtask_prompt_max,
                        total_eval,
                    },
                });
            }
            ScriptCommand::Wait(dur) => {
                println!("[wait] {}ms", dur.as_millis());
                tokio::time::sleep(*dur).await;
            }
            ScriptCommand::ExpectFile { .. } | ScriptCommand::ExpectNoFile(_) => {
                if let Some(result) = script::run_assertion(cmd, working_dir) {
                    let pass_str = if result.pass { "PASS" } else { "FAIL" };
                    println!("[assert] {pass_str} {} {}", result.assert_type, result.path);
                    report.add_assertion(result);
                }
            }
            ScriptCommand::ExpectNoEvent(event_name) => {
                let Some(last_step) = report.steps.last() else {
                    report.add_assertion(script::AssertionResult {
                        assert_type: "expect_no_event".into(),
                        path: event_name.clone(),
                        expected: format!("no {event_name}"),
                        actual: "no prior step".into(),
                        pass: true,
                    });
                    continue;
                };
                let found = last_step
                    .events
                    .iter()
                    .any(|e| e.contains(event_name.as_str()));
                let pass = !found;
                let pass_str = if pass { "PASS" } else { "FAIL" };
                println!("[assert] {pass_str} expect_no_event {event_name}");
                report.add_assertion(script::AssertionResult {
                    assert_type: "expect_no_event".into(),
                    path: event_name.clone(),
                    expected: format!("no {event_name}"),
                    actual: if found {
                        event_name.clone()
                    } else {
                        String::new()
                    },
                    pass,
                });
            }
            ScriptCommand::ExpectEvent(event_name) => {
                let Some(last_step) = report.steps.last() else {
                    println!(
                        "[assert] FAIL expect_event {event_name} — no Send step preceding this assertion"
                    );
                    report.add_assertion(script::AssertionResult {
                        assert_type: "expect_event".into(),
                        path: event_name.clone(),
                        expected: event_name.clone(),
                        actual: "no prior step".into(),
                        pass: false,
                    });
                    continue;
                };
                let pass = last_step
                    .events
                    .iter()
                    .any(|e| e.contains(event_name.as_str()));
                let pass_str = if pass { "PASS" } else { "FAIL" };
                println!("[assert] {pass_str} expect_event {event_name}");
                report.add_assertion(script::AssertionResult {
                    assert_type: "expect_event".into(),
                    path: event_name.clone(),
                    expected: event_name.clone(),
                    actual: if pass {
                        event_name.clone()
                    } else {
                        String::new()
                    },
                    pass,
                });
            }
            ScriptCommand::ExpectStat { lhs, op, rhs } => {
                let Some(last_step) = report.steps.last() else {
                    println!("[assert] FAIL expect_stat — no Send step preceding this assertion");
                    report.add_assertion(script::AssertionResult {
                        assert_type: "expect_stat".into(),
                        path: format!("{lhs} {op} {rhs}"),
                        expected: format!("{lhs} {op} {rhs}"),
                        actual: "no prior step".into(),
                        pass: false,
                    });
                    continue;
                };
                let ts = &last_step.token_summary;
                let lhs_val = ts.resolve_field(lhs);
                let rhs_val = ts.resolve_field(rhs);
                let (pass, actual_str) = match (lhs_val, rhs_val) {
                    (Some(l), Some(r)) => {
                        let result = match op.as_str() {
                            "<" => l < r,
                            ">" => l > r,
                            "==" => l == r,
                            _ => false,
                        };
                        (result, format!("{l} {op} {r}"))
                    }
                    _ => (false, format!("unknown field(s): {lhs}, {rhs}")),
                };
                let pass_str = if pass { "PASS" } else { "FAIL" };
                println!("[assert] {pass_str} expect_stat {lhs} {op} {rhs} ({actual_str})");
                report.add_assertion(script::AssertionResult {
                    assert_type: "expect_stat".into(),
                    path: format!("{lhs} {op} {rhs}"),
                    expected: format!("{lhs} {op} {rhs}"),
                    actual: actual_str,
                    pass,
                });
            }
        }
    }

    action_tx.send(UserAction::Quit).await.ok();

    let report_path = log_dir.join("test_report.json");
    report.write_to_file(&report_path)?;
    println!("[report] written to {}", report_path.display());
    report.print_summary();

    // Clean up subtask context dumps — summarised in the report
    let _ = std::fs::remove_dir_all(log_dir.join("subtasks"));

    Ok(())
}
