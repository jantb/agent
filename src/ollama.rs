use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::mpsc;

use tracing::{debug, trace, warn};

use crate::types::{AgentEvent, Message, Role, ToolCall, ToolDefinition, TurnOutcome};

static CALL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_call_id() -> String {
    let id = CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("call-{id}")
}

struct LineParser {
    buf: Vec<u8>,
}

impl LineParser {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<serde_json::Value> {
        self.buf.extend_from_slice(bytes);
        let last_newline = match self.buf.iter().rposition(|&b| b == b'\n') {
            Some(pos) => pos,
            None => return vec![],
        };
        let complete = self.buf[..=last_newline].to_vec();
        self.buf = self.buf[last_newline + 1..].to_vec();
        complete
            .split(|&b| b == b'\n')
            .filter(|l| !l.iter().all(|b| b.is_ascii_whitespace()))
            .filter_map(|l| {
                let s = match std::str::from_utf8(l) {
                    Ok(s) => s.trim(),
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping line with invalid UTF-8");
                        return None;
                    }
                };
                if s.is_empty() {
                    return None;
                }
                match serde_json::from_str(s) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        tracing::warn!(line = s, error = %e, "skipping malformed JSON line");
                        None
                    }
                }
            })
            .collect()
    }
}

const ALL_TAGS: &[&str] = &[
    "<|channel>thought",
    "<|channel>text",
    "<think>",
    "<channel|>",
    "</think>",
];

fn tag_open(s: &str) -> Option<FilterState> {
    match s {
        "<|channel>thought" => Some(FilterState::InThinkTag),
        "<|channel>text" => Some(FilterState::InTextTag),
        "<think>" => Some(FilterState::InThinkTag),
        _ => None,
    }
}

fn is_close_tag(s: &str, prior: &PriorState) -> bool {
    match prior {
        PriorState::InThinkTag => s == "<channel|>" || s == "</think>",
        PriorState::InTextTag => s == "<channel|>",
        PriorState::Text => false,
    }
}

#[derive(Clone)]
enum PriorState {
    Text,
    InThinkTag,
    InTextTag,
}

enum FilterState {
    Text,
    InThinkTag,
    InTextTag,
    Pending { buf: String, prior: PriorState },
}

#[derive(Default)]
pub struct FilterOutput {
    pub text: String,
    pub thinking: String,
}

pub struct ThinkTagFilter {
    state: FilterState,
}

impl Default for ThinkTagFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkTagFilter {
    pub fn new() -> Self {
        Self {
            state: FilterState::Text,
        }
    }

    pub fn feed(&mut self, delta: &str) -> FilterOutput {
        let mut out = FilterOutput::default();
        for ch in delta.chars() {
            self.push_char(ch, &mut out);
        }
        out
    }

    pub fn flush(&mut self) -> FilterOutput {
        let mut out = FilterOutput::default();
        if let FilterState::Pending { buf, prior } = &self.state {
            let buf = buf.clone();
            let prior = prior.clone();
            match &prior {
                PriorState::InThinkTag => out.thinking.push_str(&buf),
                _ => out.text.push_str(&buf),
            }
            self.state = match prior {
                PriorState::Text => FilterState::Text,
                PriorState::InThinkTag => FilterState::InThinkTag,
                PriorState::InTextTag => FilterState::InTextTag,
            };
        }
        out
    }

    fn push_char(&mut self, ch: char, out: &mut FilterOutput) {
        match &mut self.state {
            FilterState::Text => {
                if ch == '<' {
                    self.state = FilterState::Pending {
                        buf: String::from('<'),
                        prior: PriorState::Text,
                    };
                } else {
                    out.text.push(ch);
                }
            }
            FilterState::InThinkTag => {
                if ch == '<' {
                    self.state = FilterState::Pending {
                        buf: String::from('<'),
                        prior: PriorState::InThinkTag,
                    };
                } else {
                    out.thinking.push(ch);
                }
            }
            FilterState::InTextTag => {
                if ch == '<' {
                    self.state = FilterState::Pending {
                        buf: String::from('<'),
                        prior: PriorState::InTextTag,
                    };
                } else {
                    out.text.push(ch);
                }
            }
            FilterState::Pending { buf, prior } => {
                buf.push(ch);
                let buf_str = buf.as_str();
                if let Some(new_state) = tag_open(buf_str) {
                    self.state = new_state;
                } else if is_close_tag(buf_str, prior) {
                    self.state = FilterState::Text;
                } else if ALL_TAGS.iter().any(|t| t.starts_with(buf_str)) {
                    // still a valid prefix, stay pending
                } else {
                    // no match — flush buf to appropriate output and return to prior
                    let buf = buf.clone();
                    let prior = prior.clone();
                    match &prior {
                        PriorState::InThinkTag => out.thinking.push_str(&buf),
                        _ => out.text.push_str(&buf),
                    }
                    self.state = match prior {
                        PriorState::Text => FilterState::Text,
                        PriorState::InThinkTag => FilterState::InThinkTag,
                        PriorState::InTextTag => FilterState::InTextTag,
                    };
                }
            }
        }
    }
}

pub struct OllamaClient {
    base_url: String,
    model: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(base_url: &str, model: &str, http: reqwest::Client) -> Self {
        OllamaClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            http,
        }
    }

    fn messages_to_json(&self, history: &[Message]) -> serde_json::Value {
        // Workaround: Ollama suppresses gemma4 thinking when a system role message
        // is combined with tools. We extract the system prompt and prepend it to the
        // first user message as <system>...</system> instead.
        let mut system_content: Option<&str> = None;
        let mut msgs: Vec<serde_json::Value> = Vec::with_capacity(history.len());

        for m in history {
            if m.role == Role::System {
                system_content = Some(&m.content);
                continue;
            }
            let role = serde_json::to_value(&m.role).unwrap_or(json!("user"));
            let content = if let Some(sys) = system_content.take() {
                format!("<system>\n{sys}\n</system>\n\n{}", m.content)
            } else {
                m.content.clone()
            };
            let mut msg = json!({ "role": role, "content": content });
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
            msgs.push(msg);
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
    ) -> anyhow::Result<TurnOutcome> {
        let url = format!("{}/api/chat", self.base_url);
        let body = json!({
            "model": &self.model,
            "messages": self.messages_to_json(history),
            "tools": Self::tools_to_json(tools),
            "stream": true,
            "think": true,
            "options": {
                "temperature": 1.0,
                "top_p": 0.95,
                "top_k": 64,
                "num_predict": 8192
            }
        });

        debug!(
            model = %self.model,
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
        let mut tag_filter = ThinkTagFilter::new();
        let mut in_thinking = false;
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut full_text = String::new();
        let mut done_flag = false;

        while let Some(chunk) = stream.next().await {
            let bytes = chunk.context("stream read error")?;
            trace!(bytes = bytes.len(), "stream chunk");
            for val in lines.feed(&bytes) {
                if let Some(calls) = val["message"]["tool_calls"].as_array() {
                    for tc in calls {
                        let name = tc["function"]["name"].as_str().unwrap_or("");
                        if name.is_empty() {
                            warn!("skipping tool call with empty name");
                            continue;
                        }
                        let arguments = tc["function"]["arguments"].clone();
                        tool_calls.push(ToolCall {
                            id: next_call_id(),
                            name: name.to_string(),
                            arguments,
                        });
                    }
                }

                if let Some(think_delta) = val["message"]["thinking"].as_str() {
                    if !think_delta.is_empty() {
                        if !in_thinking {
                            in_thinking = true;
                            if tx.send(AgentEvent::ThinkingStarted).await.is_err() {
                                return Ok(TurnOutcome::Text(full_text));
                            }
                        }
                        if tx
                            .send(AgentEvent::ThinkingDelta(think_delta.to_string()))
                            .await
                            .is_err()
                        {
                            return Ok(TurnOutcome::Text(full_text));
                        }
                    }
                }

                if let Some(delta) = val["message"]["content"].as_str() {
                    if !delta.is_empty() {
                        let filtered = tag_filter.feed(delta);
                        if !filtered.thinking.is_empty() {
                            if !in_thinking {
                                in_thinking = true;
                                if tx.send(AgentEvent::ThinkingStarted).await.is_err() {
                                    return Ok(TurnOutcome::Text(full_text));
                                }
                            }
                            if tx
                                .send(AgentEvent::ThinkingDelta(filtered.thinking))
                                .await
                                .is_err()
                            {
                                return Ok(TurnOutcome::Text(full_text));
                            }
                        }
                        if !filtered.text.is_empty() {
                            if in_thinking {
                                in_thinking = false;
                                if tx.send(AgentEvent::ThinkingDone).await.is_err() {
                                    return Ok(TurnOutcome::Text(full_text));
                                }
                            }
                            full_text.push_str(&filtered.text);
                            if tx.send(AgentEvent::TextDelta(filtered.text)).await.is_err() {
                                return Ok(TurnOutcome::Text(full_text));
                            }
                        }
                    }
                }

                if val["done"].as_bool() == Some(true) {
                    if in_thinking {
                        in_thinking = false;
                        if tx.send(AgentEvent::ThinkingDone).await.is_err() {
                            return Ok(TurnOutcome::Text(full_text));
                        }
                    }
                    let eval_count = val["eval_count"].as_u64().unwrap_or(0);
                    let eval_duration_ns = val["eval_duration"].as_u64().unwrap_or(0);
                    let prompt_eval_count = val["prompt_eval_count"].as_u64().unwrap_or(0);
                    if (eval_count > 0 || prompt_eval_count > 0)
                        && tx
                            .send(AgentEvent::TurnStats {
                                eval_count,
                                eval_duration_ns,
                                prompt_eval_count,
                            })
                            .await
                            .is_err()
                    {
                        return Ok(TurnOutcome::Text(full_text));
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

        // Flush partial tag content buffered by the filter (e.g. incomplete tags at stream end)
        let flushed = tag_filter.flush();
        if !flushed.thinking.is_empty() {
            if !in_thinking {
                in_thinking = true;
                let _ = tx.send(AgentEvent::ThinkingStarted).await;
            }
            let _ = tx.send(AgentEvent::ThinkingDelta(flushed.thinking)).await;
        }
        if !flushed.text.is_empty() {
            full_text.push_str(&flushed.text);
            let _ = tx.send(AgentEvent::TextDelta(flushed.text)).await;
        }

        // Close any open thinking block (stream may end before done=true)
        if in_thinking {
            let _ = tx.send(AgentEvent::ThinkingDone).await;
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
    let mi = &resp["model_info"];
    if let Some(arch) = mi["general.architecture"].as_str() {
        let key = format!("{arch}.context_length");
        if let Some(n) = mi[&key].as_u64() {
            return Some(n);
        }
    }
    if let Some(n) = mi["llama.context_length"].as_u64() {
        return Some(n);
    }
    if let Some(n) = mi["general.context_length"].as_u64() {
        return Some(n);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;

    #[test]
    fn messages_to_json_system_merged_into_first_user() {
        let http = reqwest::Client::new();
        let client = OllamaClient::new("http://localhost:11434", "gemma4:26b", http);
        let history = vec![
            Message::new(Role::System, "You are helpful.".into()),
            Message::new(Role::User, "hello".into()),
        ];
        let json = client.messages_to_json(&history);
        // System message is merged into the first user message
        assert_eq!(json.as_array().unwrap().len(), 1);
        assert_eq!(json[0]["role"], "user");
        let content = json[0]["content"].as_str().unwrap();
        assert!(content.contains("<system>"));
        assert!(content.contains("You are helpful."));
        assert!(content.contains("hello"));
    }

    #[test]
    fn messages_to_json_includes_images() {
        let http = reqwest::Client::new();
        let client = OllamaClient::new("http://localhost:11434", "gemma4:26b", http);
        let mut msg = Message::new(Role::User, "describe this".into());
        msg.images = vec!["base64data".into()];
        let json = client.messages_to_json(&[msg]);
        assert_eq!(json[0]["images"][0], "base64data");
    }

    #[test]
    fn messages_to_json_no_images_field_when_empty() {
        let http = reqwest::Client::new();
        let client = OllamaClient::new("http://localhost:11434", "gemma4:26b", http);
        let msg = Message::new(Role::User, "hello".into());
        let json = client.messages_to_json(&[msg]);
        assert!(json[0].get("images").is_none());
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
    fn parse_context_window_num_ctx_takes_precedence() {
        let resp = json!({
            "model_info": { "llama.context_length": 32768 },
            "parameters": "num_ctx 8192"
        });
        assert_eq!(parse_context_window(&resp), Some(8192));
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
    fn parse_context_window_dynamic_arch() {
        let resp = serde_json::json!({
            "model_info": {
                "general.architecture": "gemma4",
                "gemma4.context_length": 131072
            }
        });
        assert_eq!(parse_context_window(&resp), Some(131072));
    }

    #[test]
    fn parse_context_window_dynamic_arch_takes_precedence() {
        let resp = serde_json::json!({
            "model_info": {
                "general.architecture": "gemma4",
                "gemma4.context_length": 131072,
                "llama.context_length": 32768
            }
        });
        assert_eq!(parse_context_window(&resp), Some(131072));
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

    #[test]
    fn filter_strips_channel_think_tag() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>thoughtsome thinking<channel|>real text");
        assert_eq!(out.thinking, "some thinking");
        assert_eq!(out.text, "real text");
    }

    #[test]
    fn filter_handles_split_open_tag() {
        let mut f = ThinkTagFilter::new();
        let out1 = f.feed("<|channel>th");
        assert!(out1.text.is_empty() && out1.thinking.is_empty());
        let out2 = f.feed("oughtsome thought<channel|>");
        assert_eq!(out2.thinking, "some thought");
    }

    #[test]
    fn filter_handles_split_close_tag() {
        let mut f = ThinkTagFilter::new();
        f.feed("<|channel>thoughtthinking<chan");
        let out = f.feed("nel|>after");
        assert!(out.text.contains("after"));
    }

    #[test]
    fn filter_strips_legacy_think_tags() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<think>internal</think>visible");
        assert_eq!(out.thinking, "internal");
        assert_eq!(out.text, "visible");
    }

    #[test]
    fn filter_passes_non_tag_angle_bracket() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("x < y is true");
        assert_eq!(out.text, "x < y is true");
    }

    #[test]
    fn filter_passthrough_no_tags() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("hello world");
        assert_eq!(out.text, "hello world");
        assert_eq!(out.thinking, "");
    }

    #[test]
    fn filter_text_channel_transparent() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>textvisible content<channel|>");
        assert_eq!(out.text, "visible content");
    }

    #[test]
    fn filter_text_channel_with_trailing() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>textvisible<channel|> more text");
        assert_eq!(out.text, "visible more text");
    }

    #[test]
    fn filter_think_then_text_sequence() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<|channel>thoughtmy plan<channel|><|channel>textmy response<channel|>");
        assert_eq!(out.thinking, "my plan");
        assert_eq!(out.text, "my response");
    }

    #[test]
    fn filter_flush_pending() {
        let mut f = ThinkTagFilter::new();
        let out1 = f.feed("hello<|ch");
        assert_eq!(out1.text, "hello");
        let out2 = f.flush();
        assert_eq!(out2.text, "<|ch");
    }

    #[test]
    fn line_parser_multibyte_utf8_split_across_feeds() {
        // ü = [0xc3, 0xbc]; full line: {"content":"über"}\n
        let full = b"{\"content\":\"\xc3\xbcber\"}\n";
        // split between the two bytes of ü (after the opening quote)
        let split = full.iter().position(|&b| b == 0xc3).unwrap();
        let mut p = LineParser::new();
        let v1 = p.feed(&full[..split + 1]); // includes 0xc3
        assert!(v1.is_empty());
        let v2 = p.feed(&full[split + 1..]); // 0xbc + rest + \n
        assert_eq!(v2.len(), 1);
        assert_eq!(v2[0]["content"].as_str().unwrap(), "über");
    }

    #[test]
    fn line_parser_invalid_utf8_skipped() {
        let mut p = LineParser::new();
        // invalid UTF-8 bytes followed by newline, then a valid JSON line
        let mut bytes = vec![0xff, 0xfe, b'\n'];
        bytes.extend_from_slice(b"{\"ok\":1}\n");
        let vals = p.feed(&bytes);
        assert_eq!(vals.len(), 1);
        assert_eq!(vals[0]["ok"], 1);
    }

    #[test]
    fn filter_flush_returns_thinking_from_pending_think_state() {
        // Pending buffer accumulated while in InThinkTag state should flush as thinking
        let mut f = ThinkTagFilter::new();
        f.feed("<think>partial thought</think>");
        // Now feed content that starts a new pending inside think (after re-entering via another <think>)
        let mut f2 = ThinkTagFilter::new();
        let _ = f2.feed("<think>deep<");
        // <  puts us in Pending{prior: InThinkTag}; flush should emit "<" as thinking
        let out = f2.flush();
        assert_eq!(out.thinking, "<");
    }

    #[test]
    fn filter_flush_returns_thinking_no_close_tag() {
        let mut f = ThinkTagFilter::new();
        let _ = f.feed("<think>my reasoning");
        // state is now InThinkTag (no pending, just mid-think)
        // flush should not panic and thinking already emitted via feed
        // test that a subsequent flush is safe (no content)
        let out = f.flush();
        assert_eq!(out.thinking, "");
        assert_eq!(out.text, "");
    }

    #[test]
    fn filter_think_no_close_tag_content_via_feed() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<think>my reasoning");
        assert_eq!(out.thinking, "my reasoning");
        assert_eq!(out.text, "");
    }

    #[test]
    fn filter_nested_angle_bracket_in_think_block() {
        let mut f = ThinkTagFilter::new();
        let out = f.feed("<think>if x < 10 then y</think>visible");
        assert_eq!(out.thinking, "if x < 10 then y");
        assert_eq!(out.text, "visible");
    }
}
