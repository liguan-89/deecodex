use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerExecutorBackend {
    #[default]
    Disabled,
    Playwright,
    BrowserUse,
}

impl ComputerExecutorBackend {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "disabled" | "off" | "none" => Ok(Self::Disabled),
            "playwright" => Ok(Self::Playwright),
            "browser-use" | "browser_use" | "browseruse" => Ok(Self::BrowserUse),
            other => anyhow::bail!(
                "unsupported computer executor backend '{other}', expected disabled/playwright/browser-use"
            ),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Playwright => "playwright",
            Self::BrowserUse => "browser-use",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerExecutorConfig {
    pub backend: ComputerExecutorBackend,
    pub timeout_secs: u64,
}

impl Default for ComputerExecutorConfig {
    fn default() -> Self {
        Self {
            backend: ComputerExecutorBackend::Disabled,
            timeout_secs: 30,
        }
    }
}

impl ComputerExecutorConfig {
    pub fn enabled(&self) -> bool {
        self.backend != ComputerExecutorBackend::Disabled
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub label: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default = "default_read_only")]
    pub read_only: bool,
}

fn default_read_only() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpExecutorConfig {
    pub timeout_secs: u64,
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

impl Default for McpExecutorConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            servers: BTreeMap::new(),
        }
    }
}

impl McpExecutorConfig {
    pub fn enabled(&self) -> bool {
        !self.servers.is_empty()
    }

    pub fn get_server(&self, label: &str) -> Option<&McpServerConfig> {
        self.servers.get(label)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalExecutorConfig {
    pub computer: ComputerExecutorConfig,
    pub mcp: McpExecutorConfig,
}

impl LocalExecutorConfig {
    pub fn from_raw(
        computer_backend: &str,
        computer_timeout_secs: u64,
        mcp_config: &str,
        mcp_timeout_secs: u64,
    ) -> Result<Self> {
        Ok(Self {
            computer: ComputerExecutorConfig {
                backend: ComputerExecutorBackend::parse(computer_backend)?,
                timeout_secs: computer_timeout_secs,
            },
            mcp: McpExecutorConfig {
                timeout_secs: mcp_timeout_secs,
                servers: parse_mcp_servers(mcp_config)?,
            },
        })
    }
}

fn parse_mcp_servers(raw: &str) -> Result<BTreeMap<String, McpServerConfig>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(BTreeMap::new());
    }

    let source = if raw.starts_with('{') || raw.starts_with('[') {
        raw.to_string()
    } else {
        std::fs::read_to_string(raw)
            .with_context(|| format!("failed to read MCP executor config from {raw}"))?
    };

    let value: serde_json::Value =
        serde_json::from_str(&source).context("failed to parse MCP executor config JSON")?;
    mcp_servers_from_value(value)
}

fn mcp_servers_from_value(value: serde_json::Value) -> Result<BTreeMap<String, McpServerConfig>> {
    match value {
        serde_json::Value::Array(items) => {
            let mut servers = BTreeMap::new();
            for item in items {
                let server: McpServerConfig =
                    serde_json::from_value(item).context("invalid MCP server config")?;
                validate_mcp_server(&server)?;
                servers.insert(server.label.clone(), server);
            }
            Ok(servers)
        }
        serde_json::Value::Object(map) => {
            let mut servers = BTreeMap::new();
            for (label, item) in map {
                let mut server: McpServerConfig =
                    serde_json::from_value(item).context("invalid MCP server config")?;
                if server.label.is_empty() {
                    server.label = label;
                }
                validate_mcp_server(&server)?;
                servers.insert(server.label.clone(), server);
            }
            Ok(servers)
        }
        _ => anyhow::bail!("MCP executor config must be a JSON object or array"),
    }
}

fn validate_mcp_server(server: &McpServerConfig) -> Result<()> {
    if server.label.trim().is_empty() {
        anyhow::bail!("MCP server label cannot be empty");
    }
    if server.command.trim().is_empty() {
        anyhow::bail!("MCP server '{}' command cannot be empty", server.label);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn parses_computer_backend_aliases() {
        assert_eq!(
            ComputerExecutorBackend::parse("").unwrap(),
            ComputerExecutorBackend::Disabled
        );
        assert_eq!(
            ComputerExecutorBackend::parse("browser_use").unwrap(),
            ComputerExecutorBackend::BrowserUse
        );
        assert!(ComputerExecutorBackend::parse("shell").is_err());
    }

    #[test]
    fn parses_mcp_servers_from_object() {
        let cfg = LocalExecutorConfig::from_raw(
            "playwright",
            12,
            r#"{
                "filesystem": {
                    "label": "",
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem"],
                    "env": {"ROOT": "/tmp"}
                }
            }"#,
            9,
        )
        .unwrap();

        assert!(cfg.computer.enabled());
        assert_eq!(cfg.computer.timeout_secs, 12);
        assert_eq!(cfg.mcp.timeout_secs, 9);
        let server = cfg.mcp.get_server("filesystem").unwrap();
        assert_eq!(server.label, "filesystem");
        assert_eq!(server.command, "npx");
        assert!(server.read_only);
        assert_eq!(server.env["ROOT"], "/tmp");
    }

    #[test]
    fn parses_mcp_servers_from_file() {
        let path =
            std::env::temp_dir().join(format!("deecodex-mcp-{}.json", Uuid::new_v4().simple()));
        std::fs::write(
            &path,
            r#"[{"label":"github","command":"mcp-github","read_only":false}]"#,
        )
        .unwrap();

        let cfg =
            LocalExecutorConfig::from_raw("disabled", 30, &path.to_string_lossy(), 45).unwrap();
        let server = cfg.mcp.get_server("github").unwrap();
        assert_eq!(server.command, "mcp-github");
        assert!(!server.read_only);
        std::fs::remove_file(path).unwrap();
    }
}
