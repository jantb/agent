use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::mpsc;

use tracing::{debug, trace, warn};

use crate::types::{AgentEvent, Message, ToolCall, ToolDefinition, TurnOutcome};

static CALL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_call_id() -> String {
    let id = CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("call-{id}")
}

struct LineParser {
    buf: String,
}

impl LineParser {
    fn new() -> Self {
        Self { buf: String::new() }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<serde_json::Value> {
        let text = String::from_utf8_lossy(bytes);
        self.buf.push_str(&text);
        let last_newline = match self.buf.rfind('\n') {
            Some(pos) => pos,
            None => return vec![],
        };
        let complete = self.buf[..=last_newline].to_string();
        self.buf = self.buf[last_newline + 1..].to_string();
        complete
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str(l.trim()) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(line = l.trim(), error = %e, "skipping malformed JSON line");
                    None
                }
            })
            .collect()
    }
}

struct ThinkParser {
    in_think: bool,
    started: bool,
    buf: String,
}

impl ThinkParser {
    fn new() -> Self {
        Self {
            in_think: false,
            started: false,
            buf: String::new(),
        }
    }

    async fn process(&mut self, delta: &str, tx: &mpsc::Sender<AgentEvent>) -> String {
        let mut visible = String::new();
        let mut remaining = delta;

        loop {
            if self.in_think {
                if let Some(end_pos) = remaining.find("</think>") {
                    let think_content = &remaining[..end_pos];
                    if !think_content.is_empty() {
                        if let Err(e) = tx
                            .send(AgentEvent::ThinkingDelta(think_content.to_string()))
                            .await
                        {
                            tracing::error!("failed to send ThinkingDelta: {e}");
                        }
                    }
                    self.in_think = false;
                    if let Err(e) = tx.send(AgentEvent::ThinkingDone).await {
                        tracing::error!("failed to send ThinkingDone: {e}");
                    }
                    self.started = false;
                    remaining = &remaining[end_pos + "</think>".len()..];
                } else {
                    if !remaining.is_empty() {
                        if let Err(e) = tx
                            .send(AgentEvent::ThinkingDelta(remaining.to_string()))
                            .await
                        {
                            tracing::error!("failed to send ThinkingDelta: {e}");
                        }
                    }
                    break;
                }
            } else if let Some(start_pos) = remaining.find("<think>") {
                visible.push_str(&remaining[..start_pos]);
                self.in_think = true;
                if !self.started {
                    self.started = true;
                    if let Err(e) = tx.send(AgentEvent::ThinkingStarted).await {
                        tracing::error!("failed to send ThinkingStarted: {e}");
                    }
                }
                self.buf.clear();
                remaining = &remaining[start_pos + "<think>".len()..];
            } else {
                visible.push_str(remaining);
                break;
            }
        }
        visible
    }

    async fn finish(&self, tx: &mpsc::Sender<AgentEvent>) {
        if self.started {
            if let Err(e) = tx.send(AgentEvent::ThinkingDone).await {
                tracing::error!("failed to send ThinkingDone (finish): {e}");
            }
        }
    }
}

pub struct OllamaClient {
    base_url: String,
    model: String,
    tool_model: Option<String>,
    thinking: bool,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(
        base_url: &str,
        model: &str,
        tool_model: Option<String>,
        thinking: bool,
        http: reqwest::Client,
    ) -> Self {
        OllamaClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            tool_model,
            thinking,
            http,
        }
    }

    fn messages_to_json(&self, history: &[Message]) -> serde_json::Value {
        let mut msgs: Vec<serde_json::Value> = history
            .iter()
            .map(|m| {
                let role = serde_json::to_value(&m.role).unwrap_or(json!("user"));
                let mut msg = json!({ "role": role, "content": m.content });
                if !m.images.is_empty() {
                    msg["images"] = json!(m.images);
                }
                if !m.tool_calls.is_empty() {
                    msg["tool_calls"] = json!(m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments
                                }
                            })
                        })
                        .collect::<Vec<_>>());
                }
                msg
            })
            .collect();

        // Inject thinking prefix into system message if enabled
        if self.thinking {
            if let Some(first) = msgs.first_mut() {
                if first["role"] == "system" {
                    let content = first["content"].as_str().unwrap_or("").to_string();
                    first["content"] = json!(format!("<|think|>\n{content}"));
                }
            }
        }
        json!(msgs)
    }

    fn tools_to_json(tools: &[ToolDefinition]) -> serde_json::Value {
        let arr: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            })
            .collect();
        json!(arr)
    }

    pub async fn stream_turn(
        &self,
        history: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<AgentEvent>,
        use_tool_model: bool,
    ) -> anyhow::Result<TurnOutcome> {
        let url = format!("{}/api/chat", self.base_url);
        let active_model = if use_tool_model {
            self.tool_model.as_deref().unwrap_or(&self.model)
        } else {
            &self.model
        };
        let body = json!({
            "model": active_model,
            "messages": self.messages_to_json(history),
            "tools": Self::tools_to_json(tools),
            "stream": true,
            "options": {
                "temperature": 1.0,
                "top_p": 0.95,
                "top_k": 64
            }
        });

        debug!(
            model = active_model,
            messages = history.len(),
            tools = tools.len(),
            "ollama request start"
        );
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("connecting to Ollama")?
            .error_for_status()
            .context("Ollama returned error status")?;

        let mut stream = resp.bytes_stream();
        let mut lines = LineParser::new();
        let mut think = ThinkParser::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut full_text = String::new();
        let mut done_flag = false;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("stream read error")?;
            trace!(bytes = bytes.len(), "stream chunk");
            for val in lines.feed(&bytes) {
                if let Some(calls) = val["message"]["tool_calls"].as_array() {
                    for tc in calls {
                        let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                        let arguments = tc["function"]["arguments"].clone();
                        tool_calls.push(ToolCall {
                            id: next_call_id(),
                            name,
                            arguments,
                        });
                    }
                }

                if let Some(delta) = val["message"]["content"].as_str() {
                    if !delta.is_empty() {
                        let remaining = think.process(delta, &tx).await;
                        if !remaining.is_empty() {
                            full_text.push_str(&remaining);
                            if let Err(e) = tx.send(AgentEvent::TextDelta(remaining)).await {
                                tracing::error!("failed to send TextDelta: {e}");
                            }
                        }
                    }
                }

                if val["done"].as_bool() == Some(true) {
                    think.finish(&tx).await;
                    let eval_count = val["eval_count"].as_u64().unwrap_or(0);
                    let prompt_eval_count = val["prompt_eval_count"].as_u64().unwrap_or(0);
                    if eval_count > 0 || prompt_eval_count > 0 {
                        if let Err(e) = tx
                            .send(AgentEvent::TurnStats {
                                eval_count,
                                prompt_eval_count,
                            })
                            .await
                        {
                            tracing::error!("failed to send TurnStats: {e}");
                        }
                    }
                    debug!(
                        eval_count,
                        prompt_eval_count,
                        tool_calls = tool_calls.len(),
                        "ollama stream done"
                    );
                    done_flag = true;
                }
            }
        }

        if !done_flag {
            warn!("stream ended without done=true — possible Ollama hang");
        }
        if !tool_calls.is_empty() {
            Ok(TurnOutcome::ToolCalls(full_text, tool_calls))
        } else {
            Ok(TurnOutcome::Text(full_text))
        }
    }

    pub async fn list_models(&self) -> anyhow::Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url);
        let resp: serde_json::Value = self
            .http
            .get(&url)
            .send()
            .await
            .context("connecting to Ollama")?
            .error_for_status()
            .context("Ollama /api/tags returned error")?
            .json()
            .await
            .context("parsing Ollama model list")?;

        let models = resp["models"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| m["name"].as_str().map(str::to_string))
            .collect();
        Ok(models)
    }

    pub async fn fetch_context_window(&self) -> anyhow::Result<Option<u64>> {
        let url = format!("{}/api/show", self.base_url);
        let body = json!({ "model": self.model });
        let resp: serde_json::Value = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("connecting to Ollama /api/show")?
            .error_for_status()
            .context("Ollama /api/show returned error")?
            .json()
            .await
            .context("parsing /api/show response")?;
        Ok(parse_context_window(&resp))
    }
}

fn parse_context_window(resp: &serde_json::Value) -> Option<u64> {
    if let Some(n) = resp["model_info"]["llama.context_length"].as_u64() {
        return Some(n);
    }
    if let Some(n) = resp["model_info"]["general.context_length"].as_u64() {
        return Some(n);
    }
    if let Some(params) = resp["parameters"].as_str() {
        for line in params.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 && parts[0] == "num_ctx" {
                if let Ok(n) = parts[1].parse::<u64>() {
                    return Some(n);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;
    use tokio::sync::mpsc;

    async fn collect_events(delta: &str) -> (Vec<AgentEvent>, String) {
        let (tx, mut rx) = mpsc::channel(32);
        let mut parser = ThinkParser::new();
        let visible = parser.process(delta, &tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        (events, visible)
    }

    #[tokio::test]
    async fn plain_text_no_think_tags() {
        let (events, visible) = collect_events("hello world").await;
        assert!(events.is_empty());
        assert_eq!(visible, "hello world");
    }

    #[tokio::test]
    async fn think_tags_emit_events() {
        let (events, visible) = collect_events("<think>internal</think>response").await;
        assert_eq!(visible, "response");
        let has_started = events
            .iter()
            .any(|e| matches!(e, AgentEvent::ThinkingStarted));
        let has_delta = events
            .iter()
            .any(|e| matches!(e, AgentEvent::ThinkingDelta(d) if d.contains("internal")));
        let has_done = events.iter().any(|e| matches!(e, AgentEvent::ThinkingDone));
        assert!(has_started, "should have ThinkingStarted");
        assert!(has_delta, "should have ThinkingDelta with 'internal'");
        assert!(has_done, "should have ThinkingDone");
    }

    #[tokio::test]
    async fn think_before_text() {
        let (_, visible) = collect_events("<think>private</think>public").await;
        assert_eq!(visible, "public");
    }

    #[tokio::test]
    async fn text_before_and_after_think() {
        let (_, visible) = collect_events("pre<think>mid</think>post").await;
        assert_eq!(visible, "prepost");
    }

    #[tokio::test]
    async fn no_end_tag_stays_in_thinking_mode() {
        let (events, visible) = collect_events("<think>still thinking").await;
        assert!(visible.is_empty());
        let has_delta = events
            .iter()
            .any(|e| matches!(e, AgentEvent::ThinkingDelta(d) if d.contains("still thinking")));
        assert!(has_delta);
    }

    #[test]
    fn messages_to_json_system_gets_think_prefix() {
        let http = reqwest::Client::new();
        let client = OllamaClient::new("http://localhost:11434", "gemma4:26b", None, true, http);
        let history = vec![Message::new(Role::System, "You are helpful.".into())];
        let json = client.messages_to_json(&history);
        let content = json[0]["content"].as_str().unwrap();
        assert!(content.starts_with("<|think|>"));
    }

    #[test]
    fn messages_to_json_no_think_prefix_when_disabled() {
        let http = reqwest::Client::new();
        let client = OllamaClient::new("http://localhost:11434", "gemma4:26b", None, false, http);
        let history = vec![Message::new(Role::System, "You are helpful.".into())];
        let json = client.messages_to_json(&history);
        let content = json[0]["content"].as_str().unwrap();
        assert!(!content.contains("<|think|>"));
    }

    #[test]
    fn tools_to_json_format() {
        let tools = vec![ToolDefinition {
            name: "my_tool".into(),
            description: "does something".into(),
            parameters: json!({"type": "object"}),
            source: crate::types::ToolSource::BuiltIn,
        }];
        let json = OllamaClient::tools_to_json(&tools);
        assert_eq!(json[0]["type"], "function");
        assert_eq!(json[0]["function"]["name"], "my_tool");
    }

    #[test]
    fn parse_context_window_from_model_info() {
        let resp = json!({
            "model_info": { "llama.context_length": 32768 }
        });
        assert_eq!(parse_context_window(&resp), Some(32768));
    }

    #[test]
    fn parse_context_window_from_parameters_string() {
        let resp = json!({
            "parameters": "stop <|end|>\nnum_ctx 8192\ntemperature 0.7"
        });
        assert_eq!(parse_context_window(&resp), Some(8192));
    }

    #[test]
    fn parse_context_window_missing_returns_none() {
        let resp = json!({ "modelfile": "..." });
        assert_eq!(parse_context_window(&resp), None);
    }

    #[test]
    fn parse_context_window_model_info_takes_precedence() {
        let resp = json!({
            "model_info": { "llama.context_length": 32768 },
            "parameters": "num_ctx 8192"
        });
        assert_eq!(parse_context_window(&resp), Some(32768));
    }

    #[test]
    fn parse_context_window_general_context_length() {
        let resp = serde_json::json!({
            "model_info": { "general.context_length": 65536 }
        });
        assert_eq!(parse_context_window(&resp), Some(65536));
    }

    #[test]
    fn parse_context_window_llama_takes_precedence_over_general() {
        let resp = serde_json::json!({
            "model_info": {
                "llama.context_length": 32768,
                "general.context_length": 65536
            }
        });
        assert_eq!(parse_context_window(&resp), Some(32768));
    }

    #[test]
    fn line_parser_complete_line() {
        let mut p = LineParser::new();
        let vals = p.feed(b"{\"done\":true}\n");
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0]["done"], true);
    }

    #[test]
    fn line_parser_partial_buffered() {
        let mut p = LineParser::new();
        let vals = p.feed(b"{\"done\":");
        assert!(vals.is_empty());
        let vals = p.feed(b"true}\n");
        assert_eq!(vals.len(), 1);
    }

    #[test]
    fn line_parser_multiple_lines() {
        let mut p = LineParser::new();
        let vals = p.feed(b"{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(vals.len(), 2);
    }

    #[test]
    fn line_parser_invalid_json_skipped() {
        let mut p = LineParser::new();
        let vals = p.feed(b"not json\n{\"ok\":1}\n");
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0]["ok"], 1);
    }
}
