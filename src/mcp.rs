use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::json;

use tracing::{debug, warn};

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
    async fn do_connect_inner(
        url: &str,
        server_name: &str,
        headers: &HeaderMap,
        http: &reqwest::Client,
    ) -> anyhow::Result<(Vec<ToolDefinition>, String)> {
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
        debug!(server = server_name, url = url, "MCP connect start");
        http.post(url)
            .headers(headers.clone())
            .json(&init_body)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?;

        let list_body = json!({
            "jsonrpc": "2.0",
            "id": next_rpc_id(),
            "method": "tools/list",
            "params": {}
        });
        let resp: serde_json::Value = http
            .post(url)
            .headers(headers.clone())
            .json(&list_body)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let tools_raw = match resp["result"]["tools"].as_array().cloned() {
            Some(arr) => arr,
            None => {
                tracing::warn!(
                    server = server_name,
                    "MCP tools/list response missing result.tools array, assuming empty"
                );
                vec![]
            }
        };
        let tools: Vec<ToolDefinition> = tools_raw
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
        debug!(
            server = server_name,
            tool_count = tools.len(),
            "MCP connect ok"
        );
        Ok((tools, url.to_string()))
    }

    pub async fn connect(cfg: &McpServerConfig, http: &reqwest::Client) -> Self {
        let base_url = cfg.url.trim_end_matches("/sse").to_string();
        let mut headers = HeaderMap::new();
        for (k, v) in &cfg.headers {
            match (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                (Ok(name), Ok(val)) => {
                    headers.insert(name, val);
                }
                (Err(e), _) => {
                    tracing::warn!(header = %k, error = %e, "invalid MCP header name, skipping")
                }
                (_, Err(e)) => {
                    tracing::warn!(header = %k, error = %e, "invalid MCP header value, skipping")
                }
            }
        }

        match Self::do_connect_inner(&base_url, &cfg.name, &headers, http).await {
            Ok((tools, resolved_url)) => McpClient {
                server_name: cfg.name.clone(),
                base_url: resolved_url,
                headers,
                tools,
                status: McpStatus::Connected,
                http: http.clone(),
            },
            Err(e) => {
                warn!(server = %cfg.name, error = %e, "MCP connect failed");
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

    async fn reconnect(&mut self) {
        match Self::do_connect_inner(&self.base_url, &self.server_name, &self.headers, &self.http)
            .await
        {
            Ok((tools, base_url)) => {
                self.tools = tools;
                self.base_url = base_url;
                self.status = McpStatus::Connected;
                tracing::info!(server = %self.server_name, "MCP reconnected");
            }
            Err(e) => {
                self.status = McpStatus::Unavailable(e.to_string());
                tracing::warn!(server = %self.server_name, error = %e, "MCP reconnect failed");
            }
        }
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
        debug!(tool = %call.name, server = %self.server_name, "MCP execute start");
        let resp: serde_json::Value = self
            .http
            .post(&self.base_url)
            .headers(self.headers.clone())
            .json(&body)
            .timeout(Duration::from_secs(60))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        debug!(tool = %call.name, "MCP execute done");

        let content = match resp["result"]["content"].as_array().cloned() {
            Some(arr) => arr,
            None => {
                tracing::warn!(tool = %call.name, server = %self.server_name, "MCP tool response missing result.content array, assuming empty");
                vec![]
            }
        };
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
    clients: Vec<tokio::sync::Mutex<McpClient>>,
}

impl McpRegistry {
    pub async fn from_config(config: &Config, http: &reqwest::Client) -> Self {
        let clients = futures::future::join_all(
            config
                .servers
                .iter()
                .map(|server| McpClient::connect(server, http)),
        )
        .await;
        let clients = clients.into_iter().map(tokio::sync::Mutex::new).collect();
        McpRegistry { clients }
    }

    pub fn empty() -> Self {
        McpRegistry { clients: vec![] }
    }

    pub async fn all_tools(&self) -> Vec<ToolDefinition> {
        let mut tools = Vec::new();
        for client in &self.clients {
            let client = client.lock().await;
            if matches!(client.status(), McpStatus::Connected) {
                tools.extend(client.tools().iter().cloned());
            }
        }
        tools
    }

    pub async fn execute(&self, call: &ToolCall) -> ToolResult {
        for client_mutex in &self.clients {
            let mut client = client_mutex.lock().await;
            if !client.tools().iter().any(|t| t.name == call.name) {
                continue;
            }
            if matches!(client.status(), McpStatus::Unavailable(_)) {
                client.reconnect().await;
                if matches!(client.status(), McpStatus::Unavailable(_)) {
                    continue;
                }
            }
            return client.execute(call).await;
        }
        ToolResult {
            call_id: call.id.clone(),
            output: format!("no MCP server handles tool: {}", call.name),
            is_error: true,
        }
    }

    pub async fn connected_servers(&self) -> Vec<String> {
        let mut servers = Vec::new();
        for client in &self.clients {
            let client = client.lock().await;
            if matches!(client.status(), McpStatus::Connected) {
                servers.push(client.server_name().to_string());
            }
        }
        servers
    }

    pub async fn failed_servers(&self) -> Vec<(String, String)> {
        let mut servers = Vec::new();
        for client in &self.clients {
            let client = client.lock().await;
            if let McpStatus::Unavailable(reason) = client.status() {
                servers.push((client.server_name().to_string(), reason.clone()));
            }
        }
        servers
    }
}
