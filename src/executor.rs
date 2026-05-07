use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;
use tokio::time::timeout;

const MAX_SCREENSHOT_BYTES: usize = 1_500_000;

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

    pub async fn execute_action(
        &self,
        invocation: ComputerActionInvocation,
    ) -> ComputerActionOutput {
        let deadline = Duration::from_secs(self.timeout_secs.max(1));
        let backend = self.backend.as_str();
        let action_type = invocation
            .action
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let display_name = invocation.display.clone();
        let started = Instant::now();
        let output = match timeout(deadline, self.execute_action_inner(invocation)).await {
            Ok(Ok(output)) => output,
            Ok(Err(err)) => ComputerActionOutput::failed(err.to_string()),
            Err(_) => ComputerActionOutput::failed(format!(
                "computer action timed out after {}s",
                self.timeout_secs.max(1)
            )),
        };
        tracing::info!(
            backend = backend,
            display = display_name.as_str(),
            action_type = action_type.as_str(),
            status = output.status.as_str(),
            elapsed_ms = started.elapsed().as_millis(),
            "computer executor action finished"
        );
        output
    }

    async fn execute_action_inner(
        &self,
        invocation: ComputerActionInvocation,
    ) -> Result<ComputerActionOutput> {
        match self.backend {
            ComputerExecutorBackend::Disabled => Ok(ComputerActionOutput::failed(
                "computer executor is disabled".into(),
            )),
            ComputerExecutorBackend::BrowserUse => execute_browser_use_action(&invocation).await,
            ComputerExecutorBackend::Playwright => execute_playwright_action(&invocation).await,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputerActionInvocation {
    pub call_id: String,
    pub action: Value,
    pub display: String,
}

impl ComputerActionInvocation {
    pub fn from_response_item(item: &Value) -> Option<Self> {
        if item.get("type").and_then(Value::as_str) != Some("computer_call") {
            return None;
        }
        let call_id = item.get("call_id").and_then(Value::as_str)?.to_string();
        let action = item.get("action").cloned().unwrap_or_else(|| json!({}));
        let display = action
            .get("display")
            .or_else(|| action.get("environment"))
            .and_then(Value::as_str)
            .or_else(|| item.get("display").and_then(Value::as_str))
            .unwrap_or("default")
            .to_string();
        Some(Self {
            call_id,
            action,
            display,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputerActionOutput {
    pub output: Value,
    pub status: String,
}

impl ComputerActionOutput {
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
                    "type": "computer_executor_error"
                }
            }),
            status: "failed".into(),
        }
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
        let server_label = invocation.server_label.clone();
        let tool_name = invocation.tool_name.clone();
        let started = Instant::now();
        let output = match self.execute_tool_inner(invocation).await {
            Ok(output) => output,
            Err(err) => McpToolOutput::failed(err.to_string()),
        };
        tracing::info!(
            server_label = server_label.as_str(),
            tool_name = tool_name.as_str(),
            status = output.status.as_str(),
            elapsed_ms = started.elapsed().as_millis(),
            "MCP executor tool finished"
        );
        output
    }

    async fn execute_tool_inner(&self, invocation: McpToolInvocation) -> Result<McpToolOutput> {
        let server = self
            .get_server(&invocation.server_label)
            .ok_or_else(|| anyhow!("MCP server '{}' is not configured", invocation.server_label))?;

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
    let stderr_task = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(read_limited_stderr(stderr, 4096)));

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
    let init_response = read_jsonrpc_response(&mut stdout, 1).await?;

    send_jsonrpc(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    )
    .await?;

    let mut next_id = 2_u64;
    let can_list_tools = init_response
        .pointer("/result/capabilities/tools")
        .is_some();
    let tool_metadata = if can_list_tools {
        match list_mcp_tools(&mut stdin, &mut stdout, next_id).await {
            Ok(metadata) => {
                next_id += 1;
                metadata
            }
            Err(err) => {
                tracing::debug!(
                    "MCP server '{}' tools/list probe skipped or failed: {err}",
                    server.label
                );
                None
            }
        }
    } else {
        None
    };

    if server.read_only
        && !mcp_tool_allowed_by_metadata(&invocation.tool_name, tool_metadata.as_ref())
    {
        let reason = if tool_metadata.is_some() {
            "metadata"
        } else {
            "name heuristic"
        };
        anyhow::bail!(
            "MCP server '{}' is read-only and rejected tool '{}' by {reason}",
            server.label,
            invocation.tool_name
        );
    }

    send_jsonrpc(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": next_id,
            "method": "tools/call",
            "params": {
                "name": invocation.tool_name,
                "arguments": invocation.arguments
            }
        }),
    )
    .await?;
    let result = read_jsonrpc_response(&mut stdout, next_id).await?;
    let _ = stdin.shutdown().await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    let stderr_summary = match stderr_task {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };

    if let Some(error) = result.get("error") {
        anyhow::bail!(
            "MCP tool call failed: {}{}",
            error,
            stderr_suffix(&stderr_summary)
        );
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

async fn list_mcp_tools<W, R>(stdin: &mut W, stdout: &mut R, id: u64) -> Result<Option<Value>>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    send_jsonrpc(
        stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await?;
    let response = read_jsonrpc_response(stdout, id).await?;
    if response.get("error").is_some() {
        return Ok(None);
    }
    Ok(response.get("result").cloned())
}

fn mcp_tool_allowed_by_metadata(tool_name: &str, metadata: Option<&Value>) -> bool {
    let Some(metadata) = metadata else {
        return read_only_tool_allowed(tool_name);
    };
    let tools = metadata
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let Some(tool) = tools
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(tool_name))
    else {
        return read_only_tool_allowed(tool_name);
    };
    let annotations = tool.get("annotations").unwrap_or(&Value::Null);
    if annotations
        .get("destructiveHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    if annotations
        .get("readOnlyHint")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    if annotations.get("readOnlyHint").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    read_only_tool_allowed(tool_name)
}

async fn read_limited_stderr<R>(mut reader: R, limit: usize) -> String
where
    R: AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf).await;
    let mut text = String::from_utf8_lossy(&buf).to_string();
    if text.len() > limit {
        text.truncate(limit);
        text.push_str("...[truncated]");
    }
    text
}

fn stderr_suffix(stderr: &str) -> String {
    let clean = stderr.trim();
    if clean.is_empty() {
        String::new()
    } else {
        format!("; stderr: {clean}")
    }
}

async fn execute_playwright_action(
    invocation: &ComputerActionInvocation,
) -> Result<ComputerActionOutput> {
    let action_json = serde_json::to_string(&invocation.action)?;
    let script = r#"
(async () => {
  const action = JSON.parse(process.env.DEECODEX_COMPUTER_ACTION || "{}");
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const { chromium } = await import("playwright");
  const stateRoot = process.env.DEECODEX_PLAYWRIGHT_STATE_DIR || "";
  const display = String(action.display || action.environment || "default").replace(/[^A-Za-z0-9_.-]/g, "_");
  const displayStateDir = stateRoot ? path.join(stateRoot, display) : "";
  let browser = null;
  let context = null;
  if (displayStateDir) {
    await fs.mkdir(displayStateDir, { recursive: true });
    context = await chromium.launchPersistentContext(displayStateDir, { headless: true, viewport: { width: Number(action.display_width || 1024), height: Number(action.display_height || 768) } });
  } else {
    browser = await chromium.launch({ headless: true });
    context = await browser.newContext({ viewport: { width: Number(action.display_width || 1024), height: Number(action.display_height || 768) } });
  }
  const page = context.pages()[0] || await context.newPage();
  const lastUrlFile = displayStateDir ? path.join(displayStateDir, "deecodex-last-url.txt") : "";
  let targetUrl = action.url || "";
  if (!targetUrl && lastUrlFile) {
    targetUrl = await fs.readFile(lastUrlFile, "utf8").catch(() => "");
  }
  if (!targetUrl) targetUrl = "about:blank";
  if (targetUrl) await page.goto(targetUrl, { waitUntil: "domcontentloaded", timeout: 15000 }).catch(() => {});
  const typ = action.type || "screenshot";
  if (typ === "click") await page.mouse.click(Number(action.x || 0), Number(action.y || 0), { button: action.button || "left" });
  if (typ === "double_click") await page.mouse.dblclick(Number(action.x || 0), Number(action.y || 0), { button: action.button || "left" });
  if (typ === "scroll") await page.mouse.wheel(Number(action.scroll_x || 0), Number(action.scroll_y || action.y || 0));
  if (typ === "type") await page.keyboard.type(String(action.text || ""));
  if (typ === "keypress") {
    const keys = Array.isArray(action.keys) ? action.keys : [action.key || action.text || "Enter"];
    for (const key of keys) await page.keyboard.press(String(key));
  }
  if (typ === "wait") await page.waitForTimeout(Number(action.ms || 1000));
  if (typ === "open_url" && action.url) await page.goto(action.url, { waitUntil: "domcontentloaded", timeout: 15000 }).catch(() => {});
  const bytes = await page.screenshot({ type: "png", fullPage: false });
  if (lastUrlFile) await fs.writeFile(lastUrlFile, page.url(), "utf8").catch(() => {});
  await context.close();
  if (browser) await browser.close();
  process.stdout.write(JSON.stringify({
    backend: "playwright",
    action_type: typ,
    url: page.url(),
    screenshot: "data:image/png;base64," + bytes.toString("base64")
  }));
})().catch((err) => {
  process.stderr.write(String(err && err.stack || err));
  process.exit(1);
});
"#;
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .env("DEECODEX_COMPUTER_ACTION", action_json)
        .output()
        .await
        .context("failed to start node for Playwright computer executor")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Playwright computer action failed{}",
            stderr_suffix(&stderr)
        );
    }
    let mut value: Value =
        serde_json::from_slice(&output.stdout).context("Playwright executor returned non-JSON")?;
    value["call_id"] = json!(invocation.call_id);
    value["display"] = json!(invocation.display);
    if let Some(screenshot) = value.get("screenshot").and_then(Value::as_str) {
        let approx_bytes = screenshot
            .split_once(',')
            .and_then(|(_, raw)| STANDARD.decode(raw).ok())
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        value["screenshot_bytes"] = json!(approx_bytes);
        if approx_bytes > MAX_SCREENSHOT_BYTES {
            value["screenshot"] = json!(format!(
                "[image omitted: screenshot {}B exceeds local limit {}B]",
                approx_bytes, MAX_SCREENSHOT_BYTES
            ));
            value["screenshot_omitted"] = json!(true);
            value["screenshot_limit_bytes"] = json!(MAX_SCREENSHOT_BYTES);
        }
    }
    Ok(ComputerActionOutput::succeeded(value))
}

async fn execute_browser_use_action(
    invocation: &ComputerActionInvocation,
) -> Result<ComputerActionOutput> {
    if let Ok(url) = std::env::var("DEECODEX_BROWSER_USE_BRIDGE_URL") {
        let url = url.trim();
        if !url.is_empty() {
            return execute_browser_use_http_bridge(url, invocation).await;
        }
    }
    if let Ok(command) = std::env::var("DEECODEX_BROWSER_USE_BRIDGE_COMMAND") {
        let command = command.trim();
        if !command.is_empty() {
            return execute_browser_use_command_bridge(command, invocation).await;
        }
    }
    Ok(ComputerActionOutput::failed(
        "browser-use computer executor is configured but neither DEECODEX_BROWSER_USE_BRIDGE_URL nor DEECODEX_BROWSER_USE_BRIDGE_COMMAND is set".into(),
    ))
}

async fn execute_browser_use_http_bridge(
    url: &str,
    invocation: &ComputerActionInvocation,
) -> Result<ComputerActionOutput> {
    let payload = json!({
        "call_id": invocation.call_id,
        "display": invocation.display,
        "action": invocation.action
    });
    let response = reqwest::Client::new()
        .post(url)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("failed to call browser-use bridge at {url}"))?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("browser-use bridge returned {}: {}", status.as_u16(), body);
    }
    let value: Value =
        serde_json::from_str(&body).context("browser-use bridge returned non-JSON")?;
    Ok(normalize_browser_use_output(invocation, value))
}

async fn execute_browser_use_command_bridge(
    command: &str,
    invocation: &ComputerActionInvocation,
) -> Result<ComputerActionOutput> {
    let action_json = serde_json::to_string(&json!({
        "call_id": invocation.call_id,
        "display": invocation.display,
        "action": invocation.action
    }))?;
    let output = Command::new(command)
        .env("DEECODEX_COMPUTER_ACTION", action_json)
        .output()
        .await
        .with_context(|| format!("failed to start browser-use bridge command '{command}'"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "browser-use bridge command failed{}",
            stderr_suffix(&stderr)
        );
    }
    let value: Value =
        serde_json::from_slice(&output.stdout).context("browser-use bridge returned non-JSON")?;
    Ok(normalize_browser_use_output(invocation, value))
}

fn normalize_browser_use_output(
    invocation: &ComputerActionInvocation,
    mut value: Value,
) -> ComputerActionOutput {
    if !value.is_object() {
        value = json!({"output": value});
    }
    value["backend"] = json!("browser-use");
    value["call_id"] = json!(invocation.call_id);
    value["display"] = json!(invocation.display);
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed")
        .to_string();
    if let Some(screenshot) = value.get("screenshot").and_then(Value::as_str) {
        let approx_bytes = screenshot
            .split_once(',')
            .and_then(|(_, raw)| STANDARD.decode(raw).ok())
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        value["screenshot_bytes"] = json!(approx_bytes);
        if approx_bytes > MAX_SCREENSHOT_BYTES {
            value["screenshot"] = json!(format!(
                "[image omitted: screenshot {}B exceeds local limit {}B]",
                approx_bytes, MAX_SCREENSHOT_BYTES
            ));
            value["screenshot_omitted"] = json!(true);
            value["screenshot_limit_bytes"] = json!(MAX_SCREENSHOT_BYTES);
        }
    }
    ComputerActionOutput {
        output: value,
        status,
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
    fn parses_computer_invocation_from_response_item() {
        let invocation = ComputerActionInvocation::from_response_item(&json!({
            "type": "computer_call",
            "call_id": "call_screen",
            "action": {"type": "screenshot", "display": "browser"}
        }))
        .unwrap();

        assert_eq!(invocation.call_id, "call_screen");
        assert_eq!(invocation.display, "browser");
        assert_eq!(invocation.action["type"], "screenshot");
    }

    #[test]
    fn read_only_tool_filter_blocks_mutations() {
        assert!(read_only_tool_allowed("read_file"));
        assert!(read_only_tool_allowed("search"));
        assert!(!read_only_tool_allowed("write_file"));
        assert!(!read_only_tool_allowed("delete"));
        assert!(!read_only_tool_allowed("apply_patch"));
    }

    #[test]
    fn mcp_read_only_prefers_tool_metadata() {
        let metadata = json!({
            "tools": [
                {"name": "fetch", "annotations": {"readOnlyHint": true}},
                {"name": "remove_file", "annotations": {"destructiveHint": true}}
            ]
        });

        assert!(mcp_tool_allowed_by_metadata("fetch", Some(&metadata)));
        assert!(!mcp_tool_allowed_by_metadata(
            "remove_file",
            Some(&metadata)
        ));
        assert!(!mcp_tool_allowed_by_metadata("write_file", None));
    }

    #[tokio::test]
    async fn browser_use_executor_returns_explicit_failed_output() {
        let config = ComputerExecutorConfig {
            backend: ComputerExecutorBackend::BrowserUse,
            timeout_secs: 1,
        };
        let output = config
            .execute_action(ComputerActionInvocation {
                call_id: "call_screen".into(),
                action: json!({"type": "screenshot"}),
                display: "browser".into(),
            })
            .await;

        assert_eq!(output.status, "failed");
        assert_eq!(
            output.output["error"]["type"].as_str(),
            Some("computer_executor_error")
        );
    }

    #[test]
    fn browser_use_output_is_normalized_and_large_screenshot_is_omitted() {
        let raw = format!(
            "data:image/png;base64,{}",
            STANDARD.encode(vec![1_u8; MAX_SCREENSHOT_BYTES + 1])
        );
        let output = normalize_browser_use_output(
            &ComputerActionInvocation {
                call_id: "call_screen".into(),
                action: json!({"type": "screenshot"}),
                display: "browser".into(),
            },
            json!({"status": "completed", "screenshot": raw}),
        );

        assert_eq!(output.status, "completed");
        assert_eq!(output.output["backend"], "browser-use");
        assert_eq!(output.output["call_id"], "call_screen");
        assert_eq!(output.output["screenshot_omitted"], true);
        assert!(output.output["screenshot"]
            .as_str()
            .unwrap()
            .starts_with("[image omitted: screenshot"));
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
