use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;

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

    pub async fn execute_tool(&self, invocation: McpToolInvocation) -> McpToolOutput {
        match self.execute_tool_inner(invocation).await {
            Ok(output) => output,
            Err(err) => McpToolOutput::failed(err.to_string()),
        }
    }

    async fn execute_tool_inner(&self, invocation: McpToolInvocation) -> Result<McpToolOutput> {
        let server = self
            .get_server(&invocation.server_label)
            .ok_or_else(|| anyhow!("MCP server '{}' is not configured", invocation.server_label))?;

        if server.read_only && !read_only_tool_allowed(&invocation.tool_name) {
            anyhow::bail!(
                "MCP server '{}' is read-only and rejected tool '{}'",
                server.label,
                invocation.tool_name
            );
        }

        let deadline = Duration::from_secs(self.timeout_secs.max(1));
        timeout(deadline, execute_stdio_mcp_tool(server, &invocation))
            .await
            .map_err(|_| anyhow!("MCP tool '{}' timed out", invocation.tool_name))?
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpToolInvocation {
    pub server_label: String,
    pub tool_name: String,
    pub arguments: Value,
}

impl McpToolInvocation {
    pub fn from_response_item(item: &Value) -> Option<Self> {
        if item.get("type").and_then(Value::as_str) != Some("mcp_tool_call") {
            return None;
        }
        let server_label = item
            .get("server_label")
            .and_then(Value::as_str)
            .unwrap_or("remote_mcp")
            .to_string();
        let tool_name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let arguments = match item.get("arguments") {
            Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or_else(|_| json!(raw)),
            Some(value) => value.clone(),
            None => json!({}),
        };
        Some(Self {
            server_label,
            tool_name,
            arguments,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpToolOutput {
    pub output: Value,
    pub status: String,
}

impl McpToolOutput {
    pub fn succeeded(output: Value) -> Self {
        Self {
            output,
            status: "completed".into(),
        }
    }

    pub fn failed(message: String) -> Self {
        Self {
            output: json!({
                "error": {
                    "message": message,
                    "type": "mcp_executor_error"
                }
            }),
            status: "failed".into(),
        }
    }
}

async fn execute_stdio_mcp_tool(
    server: &McpServerConfig,
    invocation: &McpToolInvocation,
) -> Result<McpToolOutput> {
    let mut command = Command::new(&server.command);
    command.args(&server.args);
    command.envs(&server.env);
    if let Some(cwd) = &server.cwd {
        command.current_dir(cwd);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to start MCP server '{}'", server.label))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("failed to open stdin for MCP server '{}'", server.label))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to open stdout for MCP server '{}'", server.label))?;

    send_jsonrpc(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "deecodex", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
    )
    .await?;
    read_jsonrpc_response(&mut stdout, 1).await?;

    send_jsonrpc(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    )
    .await?;

    send_jsonrpc(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": invocation.tool_name,
                "arguments": invocation.arguments
            }
        }),
    )
    .await?;
    let result = read_jsonrpc_response(&mut stdout, 2).await?;
    let _ = stdin.shutdown().await;
    let _ = child.kill().await;
    let _ = child.wait().await;

    if let Some(error) = result.get("error") {
        anyhow::bail!("MCP tool call failed: {error}");
    }
    let output = result.get("result").cloned().unwrap_or_else(|| json!({}));
    let is_error = output
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if is_error {
        Ok(McpToolOutput {
            output,
            status: "failed".into(),
        })
    } else {
        Ok(McpToolOutput::succeeded(output))
    }
}

async fn send_jsonrpc<W>(writer: &mut W, value: &Value) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(value)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_jsonrpc_response<R>(reader: &mut R, expected_id: u64) -> Result<Value>
where
    R: AsyncRead + Unpin,
{
    loop {
        let message = read_framed_json(reader).await?;
        if message.get("id").and_then(Value::as_u64) == Some(expected_id) {
            return Ok(message);
        }
    }
}

async fn read_framed_json<R>(reader: &mut R) -> Result<Value>
where
    R: AsyncRead + Unpin,
{
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        reader.read_exact(&mut byte).await?;
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
        if header.len() > 8192 {
            anyhow::bail!("MCP message header too large");
        }
    }
    let header = String::from_utf8(header).context("MCP message header is not UTF-8")?;
    let content_length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("MCP message missing Content-Length"))?;
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body).context("failed to parse MCP JSON-RPC message")
}

fn read_only_tool_allowed(tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();
    let tokens: Vec<&str> = name
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect();
    let write_markers = [
        "write", "create", "delete", "remove", "rm", "move", "rename", "patch", "edit", "update",
        "insert", "replace", "append", "mkdir", "touch", "save", "upload",
    ];
    !write_markers.iter().any(|marker| {
        name == *marker
            || tokens.iter().any(|token| token == marker)
            || name.starts_with(&format!("{marker}_"))
            || name.ends_with(&format!("_{marker}"))
    })
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

    #[test]
    fn parses_invocation_from_response_item() {
        let invocation = McpToolInvocation::from_response_item(&json!({
            "type": "mcp_tool_call",
            "server_label": "filesystem",
            "name": "read_file",
            "arguments": "{\"path\":\"/tmp/a.txt\"}"
        }))
        .unwrap();

        assert_eq!(invocation.server_label, "filesystem");
        assert_eq!(invocation.tool_name, "read_file");
        assert_eq!(invocation.arguments["path"], "/tmp/a.txt");
    }

    #[test]
    fn read_only_tool_filter_blocks_mutations() {
        assert!(read_only_tool_allowed("read_file"));
        assert!(read_only_tool_allowed("search"));
        assert!(!read_only_tool_allowed("write_file"));
        assert!(!read_only_tool_allowed("delete"));
        assert!(!read_only_tool_allowed("apply_patch"));
    }

    #[tokio::test]
    async fn stdio_frame_roundtrip() {
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        let read_task = tokio::spawn(async move { read_framed_json(&mut reader).await });

        send_jsonrpc(
            &mut writer,
            &json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}}),
        )
        .await
        .unwrap();

        let message = read_task.await.unwrap().unwrap();
        assert_eq!(message["id"], 7);
        assert_eq!(message["result"]["ok"], true);
    }
}
