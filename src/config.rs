use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("malformed .mcp.json: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("io error reading .mcp.json: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub servers: Vec<McpServerConfig>,
}

pub fn load_config(dir: &Path) -> Result<Option<Config>, ConfigError> {
    let path = dir.join(".mcp.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    if text.trim().is_empty() {
        tracing::warn!(".mcp.json is empty; ignoring");
        return Ok(Some(Config { servers: vec![] }));
    }
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(".mcp.json ignored: {e}");
            return Ok(Some(Config { servers: vec![] }));
        }
    };
    let servers_obj = value
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut servers = Vec::new();
    for (name, entry) in servers_obj {
        let Some(url) = entry.get("url").and_then(|v| v.as_str()) else {
            tracing::warn!(server = %name, ".mcp.json entry skipped: missing 'url'");
            continue;
        };
        let headers = entry
            .get("headers")
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect::<HashMap<String, String>>()
            })
            .unwrap_or_default();
        servers.push(McpServerConfig {
            name,
            url: url.to_string(),
            headers,
        });
    }
    Ok(Some(Config { servers }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &Path, name: &str, content: &str) {
        let mut f = std::fs::File::create(dir.join(name)).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn missing_file_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = load_config(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn valid_config_parsed() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            ".mcp.json",
            r#"{
                "mcpServers": {
                    "my-server": {
                        "url": "http://localhost:3000/sse",
                        "headers": { "Authorization": "Bearer tok" }
                    }
                }
            }"#,
        );
        let config = load_config(dir.path()).unwrap().unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].name, "my-server");
        assert_eq!(config.servers[0].url, "http://localhost:3000/sse");
        assert_eq!(
            config.servers[0].headers.get("Authorization").unwrap(),
            "Bearer tok"
        );
    }

    #[test]
    fn malformed_json_is_ignored() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "{ not valid json }");
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn empty_object_returns_empty_servers() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "{}");
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn empty_file_is_ignored() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "");
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn config_without_headers_defaults_to_empty() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            ".mcp.json",
            r#"{"mcpServers": {"s": {"url": "http://x"}}}"#,
        );
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers[0].headers.is_empty());
    }

    #[test]
    fn missing_mcp_servers_field_returns_empty() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", r#"{"other": 1}"#);
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn entry_missing_url_is_skipped() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            ".mcp.json",
            r#"{"mcpServers":{"bad":{"headers":{}},"good":{"url":"http://y"}}}"#,
        );
        let config = load_config(dir.path()).unwrap().unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].name, "good");
    }

    #[test]
    fn non_object_root_returns_empty() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "[]");
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn extra_fields_on_server_entry_are_ignored() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            ".mcp.json",
            r#"{"mcpServers":{"s":{"type":"http","url":"http://x"}}}"#,
        );
        let config = load_config(dir.path()).unwrap().unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].url, "http://x");
    }
}
