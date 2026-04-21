use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use agent::agent::UserAction;
use agent::bootstrap::{setup, SetupConfig};
use agent::tui::events::handle_agent_event;
use agent::types::{AgentEvent, AgentMode};
use agent::ui;
use ratatui::backend::TestBackend;
use ratatui::layout::Position;
use ratatui::Terminal;

const PROMPT: &str = "Implement the `LruCache` struct and its methods in src/lib.rs so that \
`cargo test` passes. Read src/lib.rs first for the full spec — struct signature, method \
signatures, doc comments, and all test cases. Do not modify the tests or public signatures.";

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let dest = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

fn describe_event(e: &AgentEvent, elapsed_ms: u128) -> String {
    match e {
        AgentEvent::ThinkingStarted => format!("{elapsed_ms} ThinkingStarted"),
        AgentEvent::ThinkingDelta(d) => format!("{elapsed_ms} ThinkingDelta({} chars)", d.len()),
        AgentEvent::ThinkingDone => format!("{elapsed_ms} ThinkingDone"),
        AgentEvent::TextDelta(d) => format!("{elapsed_ms} TextDelta({} chars)", d.len()),
        AgentEvent::ToolRequested(c) => {
            let args = c.arguments.to_string();
            let truncated = &args[..args.len().min(200)];
            format!("{elapsed_ms} ToolRequested {} {truncated}", c.name)
        }
        AgentEvent::ToolCompleted(r) => format!("{elapsed_ms} ToolCompleted {}", r.call_id),
        AgentEvent::TurnStats {
            eval_count,
            prompt_eval_count,
            ..
        } => format!("{elapsed_ms} TurnStats eval={eval_count} prompt={prompt_eval_count}"),
        AgentEvent::TurnDone => format!("{elapsed_ms} TurnDone"),
        AgentEvent::Error(e) => format!("{elapsed_ms} Error {e}"),
        AgentEvent::LoopDetected => format!("{elapsed_ms} LoopDetected"),
        AgentEvent::SubtaskEnter { depth, label } => {
            format!("{elapsed_ms} SubtaskEnter depth={depth} {label}")
        }
        AgentEvent::SubtaskExit { depth } => format!("{elapsed_ms} SubtaskExit depth={depth}"),
        AgentEvent::InterviewQuestion { question, .. } => {
            format!("{elapsed_ms} InterviewQuestion {question}")
        }
        AgentEvent::PlanUpdated(items) => format!("{elapsed_ms} PlanUpdated {} items", items.len()),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start = Instant::now();
    let repo_root = std::env::current_dir()?;

    let template_src = repo_root.join("tests/harness/sandbox_template");
    let sandbox: PathBuf = repo_root.join("target/harness_sandbox");
    let out_dir: PathBuf = repo_root.join("target/harness_out");

    // Reset sandbox and out_dir
    let _ = fs::remove_dir_all(&sandbox);
    copy_dir_recursive(&template_src, &sandbox)?;
    let _ = fs::remove_dir_all(&out_dir);
    fs::create_dir_all(&out_dir)?;

    // Move into sandbox so bootstrap uses it as working_dir
    std::env::set_current_dir(&sandbox)?;

    let model =
        std::env::var("HARNESS_MODEL").unwrap_or_else(|_| "qwen3.6:35b-a3b-coding-nvfp4".into());
    let ollama_url =
        std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());

    let setup = setup(SetupConfig {
        model: model.clone(),
        ollama_url,
    })
    .await?;

    let agent::bootstrap::Setup {
        mut app,
        mut event_rx,
        action_tx,
        ..
    } = setup;

    // TUI
    let backend = TestBackend::new(140, 40);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|f| ui::draw(f, &app))?;

    // Open transcript files
    let mut orch_file = fs::File::create(out_dir.join("orchestrator.txt"))?;
    let mut worker_file = fs::File::create(out_dir.join("worker.txt"))?;

    let mut orch_events: usize = 0;
    let mut worker_events: usize = 0;
    let mut current_depth: usize = 0;
    let mut ui_depth: usize = 0;
    let mut orch_prompt_max: u64 = 0;
    let mut worker_prompt_max: u64 = 0;
    let mut total_eval: u64 = 0;
    let mut subtasks_spawned: usize = 0;
    let mut plan_updates: usize = 0;
    let mut last_plan_size: usize = 0;

    // Send the probe prompt
    action_tx
        .send(UserAction::SendMessage {
            text: PROMPT.into(),
            images: vec![],
            mode: AgentMode::Oneshot,
        })
        .await?;

    let timeout_secs: u64 = std::env::var("HARNESS_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(480);
    let timeout_result =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
            loop {
                let event = match event_rx.recv().await {
                    Some(e) => e,
                    None => return false,
                };

                let elapsed_ms = start.elapsed().as_millis();
                let line = describe_event(&event, elapsed_ms);

                // Route to transcript file; handle InterviewQuestion without clone
                match event {
                    AgentEvent::InterviewQuestion {
                        ref question,
                        ref suggestions,
                        ..
                    } => {
                        let answer = suggestions.first().cloned().unwrap_or_else(|| "yes".into());
                        let log_line = format!("{elapsed_ms} InterviewQuestion {question}");
                        if current_depth == 0 {
                            orch_events += 1;
                            writeln!(orch_file, "{log_line}").ok();
                        } else {
                            worker_events += 1;
                            writeln!(worker_file, "{log_line}").ok();
                        }
                        // Answer and consume event (can't clone due to oneshot sender)
                        if let AgentEvent::InterviewQuestion { answer_tx, .. } = event {
                            let _ = answer_tx.0.send(answer);
                        }
                        // Skip UI replay — oneshot sender consumed
                        continue;
                    }
                    AgentEvent::SubtaskEnter { depth, .. } => {
                        current_depth = depth;
                        subtasks_spawned += 1;
                    }
                    AgentEvent::SubtaskExit { depth } => {
                        current_depth = depth.saturating_sub(1);
                    }
                    AgentEvent::TurnStats {
                        eval_count,
                        prompt_eval_count,
                        ..
                    } => {
                        total_eval += eval_count;
                        if ui_depth == 0 {
                            orch_prompt_max = orch_prompt_max.max(prompt_eval_count);
                        } else {
                            worker_prompt_max = worker_prompt_max.max(prompt_eval_count);
                        }
                    }
                    AgentEvent::PlanUpdated(ref items) => {
                        plan_updates += 1;
                        last_plan_size = items.len();
                    }
                    _ => {}
                }

                // Log line
                if current_depth == 0 {
                    orch_events += 1;
                    writeln!(orch_file, "{line}").ok();
                } else {
                    worker_events += 1;
                    writeln!(worker_file, "{line}").ok();
                }

                // Check terminal conditions before UI relay
                let is_done = matches!(event, AgentEvent::TurnDone)
                    || matches!(event, AgentEvent::Error(_))
                    || matches!(event, AgentEvent::LoopDetected);

                // Drive App state
                handle_agent_event(Some(event), &mut app, &action_tx, &mut ui_depth).await;
                terminal.draw(|f| ui::draw(f, &app)).ok();

                if is_done && ui_depth == 0 {
                    return true;
                }
            }
        })
        .await;

    let timed_out = timeout_result.is_err();
    if timed_out {
        println!("[timeout]");
    }

    // Capture TUI buffer
    let (width, height) = {
        let b = terminal.backend().buffer();
        let area = b.area();
        (area.width, area.height)
    };
    let mut tui_rows: Vec<String> = Vec::with_capacity(height as usize);
    {
        let buf = terminal.backend().buffer();
        for row in 0..height {
            let line: String = (0..width)
                .map(|col| {
                    buf.cell(Position::new(col, row))
                        .map(|c| c.symbol().to_owned())
                        .unwrap_or_else(|| " ".into())
                })
                .collect();
            tui_rows.push(line.trim_end().to_owned());
        }
    }
    let mut tui_file = fs::File::create(out_dir.join("tui_final.txt"))?;
    for row in &tui_rows {
        writeln!(tui_file, "{row}")?;
    }

    // Run cargo test in sandbox
    let cargo_out = tokio::process::Command::new("cargo")
        .arg("test")
        .arg("--manifest-path")
        .arg(sandbox.join("Cargo.toml"))
        .output()
        .await?;
    let pass = cargo_out.status.success();
    let combined = [cargo_out.stdout.as_slice(), cargo_out.stderr.as_slice()].concat();
    fs::write(out_dir.join("cargo_test.txt"), &combined)?;

    let wall_secs = start.elapsed().as_secs_f64();

    // Summary
    println!("=== harness report ===");
    println!("result:        {}", if pass { "PASS" } else { "FAIL" });
    println!("wall time:     {wall_secs:.1}s");
    println!("orchestrator:  {orch_events} events, prompt_max={orch_prompt_max} tokens");
    println!("worker:        {worker_events} events, prompt_max={worker_prompt_max} tokens");
    println!("subtasks:      {subtasks_spawned}");
    println!("plan updates:  {plan_updates} (last size: {last_plan_size})");
    println!("total_eval:    {total_eval} tokens");
    println!("transcripts:");
    println!("  target/harness_out/orchestrator.txt");
    println!("  target/harness_out/worker.txt");
    println!("  target/harness_out/tui_final.txt");
    println!("  target/harness_out/cargo_test.txt");
    println!();
    println!("=== TUI preview (first 20 rows) ===");
    for row in tui_rows.iter().take(20) {
        println!("{row}");
    }

    if !pass {
        println!("\n--- cargo test (last 30 lines) ---");
        let text = String::from_utf8_lossy(&combined);
        let lines: Vec<&str> = text.lines().collect();
        let skip = lines.len().saturating_sub(30);
        for line in &lines[skip..] {
            println!("{line}");
        }
    }

    std::process::exit(if pass { 0 } else { 1 });
}
