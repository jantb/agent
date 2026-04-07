use std::sync::atomic::{AtomicU64, Ordering};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::json;

use crate::config::{Config, McpServerConfig};
use crate::types::{ToolCall, ToolDefinition, ToolResult, ToolSource};

static RPC_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_rpc_id() -> u64 {
    RPC_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug)]
pub enum McpStatus {
    Connected,
    Unavailable(String),
}

#[derive(Debug)]
pub struct McpClient {
    server_name: String,
    base_url: String,
    headers: HeaderMap,
    tools: Vec<ToolDefinition>,
    status: McpStatus,
    http: reqwest::Client,
}

impl McpClient {
    pub async fn connect(cfg: &McpServerConfig, http: &reqwest::Client) -> Self {
        let base_url = cfg.url.trim_end_matches("/sse").to_string();
        let mut headers = HeaderMap::new();
        for (k, v) in &cfg.headers {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                headers.insert(name, val);
            }
        }

        match Self::do_connect(&cfg.name, &base_url, &headers, http).await {
            Ok(tools) => McpClient {
                server_name: cfg.name.clone(),
                base_url,
                headers,
                tools,
                status: McpStatus::Connected,
                http: http.clone(),
            },
            Err(e) => {
                eprintln!("[mcp] warning: failed to connect to '{}': {e}", cfg.name);
                McpClient {
                    server_name: cfg.name.clone(),
                    base_url,
                    headers,
                    tools: vec![],
                    status: McpStatus::Unavailable(e.to_string()),
                    http: http.clone(),
                }
            }
        }
    }

    async fn do_connect(
        server_name: &str,
        base_url: &str,
        headers: &HeaderMap,
        http: &reqwest::Client,
    ) -> anyhow::Result<Vec<ToolDefinition>> {
        // Initialize
        let init_body = json!({
            "jsonrpc": "2.0",
            "id": next_rpc_id(),
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "agent", "version": "0.1.0" }
            }
        });
        http.post(base_url)
            .headers(headers.clone())
            .json(&init_body)
            .send()
            .await?
            .error_for_status()?;

        // List tools
        let list_body = json!({
            "jsonrpc": "2.0",
            "id": next_rpc_id(),
            "method": "tools/list",
            "params": {}
        });
        let resp: serde_json::Value = http
            .post(base_url)
            .headers(headers.clone())
            .json(&list_body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let tools = resp["result"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|t| ToolDefinition {
                name: t["name"].as_str().unwrap_or("").to_string(),
                description: t["description"].as_str().unwrap_or("").to_string(),
                parameters: t["inputSchema"].clone(),
                source: ToolSource::Mcp {
                    server_name: server_name.to_string(),
                },
            })
            .collect();
        Ok(tools)
    }

    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    pub fn status(&self) -> &McpStatus {
        &self.status
    }

    pub async fn execute(&self, call: &ToolCall) -> ToolResult {
        match self.do_execute(call).await {
            Ok(output) => ToolResult {
                call_id: call.id.clone(),
                output,
                is_error: false,
            },
            Err(e) => ToolResult {
                call_id: call.id.clone(),
                output: e.to_string(),
                is_error: true,
            },
        }
    }

    async fn do_execute(&self, call: &ToolCall) -> anyhow::Result<String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": next_rpc_id(),
            "method": "tools/call",
            "params": {
                "name": call.name,
                "arguments": call.arguments
            }
        });
        let http = &self.http;
        let resp: serde_json::Value = http
            .post(&self.base_url)
            .headers(self.headers.clone())
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let content = resp["result"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let text = content
            .iter()
            .filter_map(|c| {
                if c["type"].as_str() == Some("text") {
                    c["text"].as_str().map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(text)
    }
}

pub struct McpRegistry {
    clients: Vec<McpClient>,
}

impl McpRegistry {
    pub async fn from_config(config: &Config, http: &reqwest::Client) -> Self {
        let mut clients = Vec::new();
        for server in &config.servers {
            clients.push(McpClient::connect(server, http).await);
        }
        McpRegistry { clients }
    }

    pub fn empty() -> Self {
        McpRegistry { clients: vec![] }
    }

    pub fn all_tools(&self) -> Vec<&ToolDefinition> {
        self.clients
            .iter()
            .filter(|c| matches!(c.status(), McpStatus::Connected))
            .flat_map(|c| c.tools())
            .collect()
    }

    pub async fn execute(&self, call: &ToolCall) -> ToolResult {
        for client in &self.clients {
            if client.tools().iter().any(|t| t.name == call.name) {
                return client.execute(call).await;
            }
        }
        ToolResult {
            call_id: call.id.clone(),
            output: format!("no MCP server found for tool '{}'", call.name),
            is_error: true,
        }
    }

    pub fn connected_servers(&self) -> Vec<&str> {
        self.clients
            .iter()
            .filter(|c| matches!(c.status(), McpStatus::Connected))
            .map(|c| c.server_name())
            .collect()
    }

    pub fn failed_servers(&self) -> Vec<(&str, &str)> {
        self.clients
            .iter()
            .filter_map(|c| {
                if let McpStatus::Unavailable(reason) = c.status() {
                    Some((c.server_name(), reason.as_str()))
                } else {
                    None
                }
            })
            .collect()
    }
}
