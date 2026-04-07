use std::path::Path;

use crate::memory;
use crate::types::ToolCall;

pub async fn run_remember(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let name = call.arguments["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let content = call.arguments["content"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let description = call.arguments["description"]
        .as_str()
        .unwrap_or(&name)
        .to_string();
    let tags: Vec<String> = call.arguments["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        memory::write_memory(&wd, &name, &description, &tags, &content)
    })
    .await
    .map_err(|e| format!("task join error: {e}"))?
}

pub async fn run_recall(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let keyword = call.arguments["keyword"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::recall_memories(&wd, &keyword))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub async fn run_forget(call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let name = call.arguments["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::forget_memory(&wd, &name))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub async fn run_list_memories(_call: &ToolCall, working_dir: &Path) -> Result<String, String> {
    let wd = working_dir.to_path_buf();
    tokio::task::spawn_blocking(move || memory::list_memories(&wd))
        .await
        .map_err(|e| format!("task join error: {e}"))?
}
