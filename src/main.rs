use clap::Parser;

use agent::bootstrap::{setup, SetupConfig};

#[derive(Parser)]
#[command(name = "agent", about = "Local AI agent TUI powered by Ollama")]
struct Cli {
    #[arg(long, default_value = "qwen3.6:35b-a3b-coding-nvfp4", hide = true)]
    model: String,
    #[arg(long, default_value = "http://localhost:11434")]
    ollama_url: String,
    #[arg(long)]
    script: Option<std::path::PathBuf>,
    #[arg(long, default_value = "false")]
    headless: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let setup = setup(SetupConfig {
        model: cli.model.clone(),
        ollama_url: cli.ollama_url.clone(),
    })
    .await?;

    if let Some(script_path) = &cli.script {
        if cli.headless {
            let agent::bootstrap::Setup {
                event_rx: mut rx,
                action_tx: tx,
                log_dir,
                working_dir,
                ..
            } = setup;
            return agent::headless::run_script(
                script_path,
                &cli.model,
                &log_dir,
                &working_dir,
                &mut rx,
                &tx,
            )
            .await;
        }
    }

    let agent::bootstrap::Setup {
        app,
        event_rx,
        action_tx,
        ..
    } = setup;

    agent::tui::run_loop(app, event_rx, action_tx, cli.script).await
}
