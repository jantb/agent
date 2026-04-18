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
                source: ToolSource::Mcp,
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
                images: vec![],
            },
            Err(e) => ToolResult {
                call_id: call.id.clone(),
                output: e.to_string(),
                is_error: true,
                images: vec![],
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

        if let Some(err) = resp.get("error") {
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown MCP error");
            anyhow::bail!("{msg}");
        }
        if resp["result"]["isError"].as_bool() == Some(true) {
            let msg = resp["result"]["content"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|c| c["text"].as_str())
                .unwrap_or("tool reported error");
            anyhow::bail!("{msg}");
        }
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
            images: vec![],
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Spawn a minimal HTTP/JSON-RPC mock server that handles initialize,
    /// tools/list, and tools/call. Returns the bound port.
    async fn spawn_mock_mcp_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let raw = String::from_utf8_lossy(&buf[..n]);

                    // Extract JSON body (everything after the blank line)
                    let body_str = raw
                        .split("\r\n\r\n")
                        .nth(1)
                        .unwrap_or("")
                        .trim_end_matches('\0');
                    let req: serde_json::Value =
                        serde_json::from_str(body_str).unwrap_or(serde_json::Value::Null);
                    let method = req["method"].as_str().unwrap_or("");
                    let id = req["id"].clone();

                    let result = match method {
                        "initialize" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "protocolVersion": "2024-11-05",
                                "capabilities": {}
                            }
                        }),
                        "tools/list" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "tools": [{
                                    "name": "test_tool",
                                    "description": "A test tool",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {
                                            "input": { "type": "string" }
                                        },
                                        "required": ["input"]
                                    }
                                }]
                            }
                        }),
                        "tools/call" => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "content": [{ "type": "text", "text": "mock result" }]
                            }
                        }),
                        _ => serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32601, "message": "Method not found" }
                        }),
                    };

                    let body = result.to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        port
    }

    fn make_http_client() -> reqwest::Client {
        reqwest::Client::new()
    }

    fn mcp_config(port: u16) -> crate::config::McpServerConfig {
        McpServerConfig {
            name: "test-server".to_string(),
            url: format!("http://127.0.0.1:{port}"),
            headers: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn mcp_client_loads_tool_definitions() {
        let port = spawn_mock_mcp_server().await;
        let http = make_http_client();
        let cfg = mcp_config(port);
        let client = McpClient::connect(&cfg, &http).await;

        assert!(matches!(client.status(), McpStatus::Connected));
        let tools = client.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "test_tool");
        assert_eq!(tools[0].description, "A test tool");
        assert!(tools[0].parameters["properties"]["input"].is_object());
        assert!(matches!(tools[0].source, ToolSource::Mcp));
    }

    #[tokio::test]
    async fn mcp_registry_all_tools_returns_connected_tools() {
        let port = spawn_mock_mcp_server().await;
        let http = make_http_client();
        let config = crate::config::Config {
            servers: vec![mcp_config(port)],
        };
        let registry = McpRegistry::from_config(&config, &http).await;
        let tools = registry.all_tools().await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "test_tool");
    }

    fn mcp_tool_def() -> ToolDefinition {
        ToolDefinition {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "input": { "type": "string" } },
                "required": ["input"]
            }),
            source: ToolSource::Mcp,
        }
    }

    #[test]
    fn mcp_tool_included_in_flat_mode_depth_0() {
        use crate::tools::built_in_tool_definitions;
        use crate::tools::selection::tools_for_depth;
        use crate::types::AgentMode;

        let mut all = built_in_tool_definitions();
        all.push(mcp_tool_def());

        let tools = tools_for_depth(&all, 0, true, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"test_tool"),
            "MCP tool must be present in flat mode"
        );
        assert!(
            !names.contains(&"delegate_task"),
            "delegate_task must be absent in flat mode"
        );
    }

    #[test]
    fn mcp_tool_excluded_at_depth_0_hierarchical() {
        use crate::tools::built_in_tool_definitions;
        use crate::tools::selection::tools_for_depth;
        use crate::types::AgentMode;

        let mut all = built_in_tool_definitions();
        all.push(mcp_tool_def());

        let tools = tools_for_depth(&all, 0, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !names.contains(&"test_tool"),
            "MCP tools must NOT be available at depth 0 hierarchical"
        );
        assert!(names.contains(&"delegate_task"));
    }

    #[test]
    fn mcp_tool_excluded_at_depth_1_hierarchical() {
        use crate::tools::built_in_tool_definitions;
        use crate::tools::selection::tools_for_depth;
        use crate::types::AgentMode;

        let mut all = built_in_tool_definitions();
        all.push(mcp_tool_def());

        let tools = tools_for_depth(&all, 1, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !names.contains(&"test_tool"),
            "MCP tools must NOT be available at depth 1 hierarchical"
        );
        assert!(names.contains(&"delegate_task"));
    }

    #[test]
    fn mcp_tool_included_at_depth_2_hierarchical() {
        use crate::tools::built_in_tool_definitions;
        use crate::tools::selection::tools_for_depth;
        use crate::types::AgentMode;

        let mut all = built_in_tool_definitions();
        all.push(mcp_tool_def());

        let tools = tools_for_depth(&all, 2, false, AgentMode::Oneshot);
        let names: Vec<_> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.contains(&"test_tool"),
            "MCP tool must be present at depth 2 hierarchical"
        );
        assert!(!names.contains(&"delegate_task"));
    }

    #[tokio::test]
    async fn mcp_client_execute_returns_mock_result() {
        let port = spawn_mock_mcp_server().await;
        let http = make_http_client();
        let cfg = mcp_config(port);
        let client = McpClient::connect(&cfg, &http).await;

        let call = ToolCall {
            id: "call-1".to_string(),
            name: "test_tool".to_string(),
            arguments: serde_json::json!({ "input": "hello" }),
        };
        let result = client.execute(&call).await;
        assert!(!result.is_error, "execution should succeed");
        assert_eq!(result.output, "mock result");
        assert_eq!(result.call_id, "call-1");
    }

    #[tokio::test]
    async fn mcp_registry_execute_dispatches_to_correct_server() {
        let port = spawn_mock_mcp_server().await;
        let http = make_http_client();
        let config = crate::config::Config {
            servers: vec![mcp_config(port)],
        };
        let registry = McpRegistry::from_config(&config, &http).await;

        let call = ToolCall {
            id: "reg-call".to_string(),
            name: "test_tool".to_string(),
            arguments: serde_json::json!({ "input": "world" }),
        };
        let result = registry.execute(&call).await;
        assert!(!result.is_error);
        assert_eq!(result.output, "mock result");
    }

    #[tokio::test]
    async fn mcp_registry_execute_unknown_tool_returns_error() {
        let port = spawn_mock_mcp_server().await;
        let http = make_http_client();
        let config = crate::config::Config {
            servers: vec![mcp_config(port)],
        };
        let registry = McpRegistry::from_config(&config, &http).await;

        let call = ToolCall {
            id: "bad-call".to_string(),
            name: "nonexistent_tool".to_string(),
            arguments: serde_json::json!({}),
        };
        let result = registry.execute(&call).await;
        assert!(result.is_error);
        assert!(result.output.contains("nonexistent_tool"));
    }
}
