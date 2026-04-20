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

#[derive(Deserialize)]
struct RawConfig {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, RawServerEntry>,
}

#[derive(Deserialize)]
struct RawServerEntry {
    url: String,
    #[serde(default)]
    headers: HashMap<String, String>,
}

pub fn load_config(dir: &Path) -> Result<Option<Config>, ConfigError> {
    let path = dir.join(".mcp.json");
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)?;
    let raw: RawConfig = serde_json::from_str(&text)?;
    let servers = raw
        .mcp_servers
        .into_iter()
        .map(|(name, entry)| McpServerConfig {
            name,
            url: entry.url,
            headers: entry.headers,
        })
        .collect();
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
    fn malformed_config_returns_error() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "{ not valid json }");
        let result = load_config(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn empty_object_returns_empty_servers() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "{}");
        let config = load_config(dir.path()).unwrap().unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn empty_file_is_malformed() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), ".mcp.json", "");
        let result = load_config(dir.path());
        assert!(result.is_err());
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
}
