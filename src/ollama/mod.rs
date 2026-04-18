mod parse;
mod stream;

use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use futures::StreamExt;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::types::{AgentEvent, Message, Role, ToolCall, ToolDefinition, TurnOutcome};
use parse::parse_context_window;
use stream::{LineParser, ThinkTagFilter};

pub const NUM_CTX: u64 = 65_536;

static CALL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_call_id() -> String {
    let id = CALL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("call-{id}")
}

pub struct OllamaClient {
    base_url: String,
    model: std::sync::Mutex<String>,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(base_url: &str, model: &str, http: reqwest::Client) -> Self {
        OllamaClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: std::sync::Mutex::new(model.to_string()),
            http,
        }
    }

    pub fn set_model(&self, m: String) {
        *self.model.lock().unwrap() = m;
    }

    fn current_model(&self) -> String {
        self.model.lock().unwrap().clone()
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
            "model": self.current_model(),
            "messages": self.messages_to_json(history),
            "tools": Self::tools_to_json(tools),
            "stream": true,
            "think": true,
            "options": {
                "temperature": 1.0,
                "top_p": 0.95,
                "top_k": 64,
                "num_predict": 8192,
                "num_ctx": NUM_CTX
            }
        });

        debug!(
            model = %self.current_model(),
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
        let body = json!({ "model": self.current_model() });
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
}
