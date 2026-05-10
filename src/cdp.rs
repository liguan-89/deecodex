//! CDP (Chrome DevTools Protocol) 客户端。
//!
//! 用于连接到 Codex Electron 渲染进程的远程调试端口，注入 JavaScript。

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

/// CDP WebSocket 类型别名。
pub type CdpWsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// CDP WebSocket 客户端。
pub struct CdpClient {
    ws: Option<CdpWsStream>,
    next_id: u64,
}

impl CdpClient {
    /// 连接到 CDP WebSocket URL。
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws, _) = connect_async(ws_url)
            .await
            .with_context(|| format!("CDP WebSocket 连接失败: {ws_url}"))?;
        Ok(Self {
            ws: Some(ws),
            next_id: 1,
        })
    }

    fn ws_mut(&mut self) -> Result<&mut CdpWsStream> {
        self.ws.as_mut().context("CDP WebSocket 已被取走")
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// 取走内部 WebSocket（用于转移到后台桥接循环）。
    pub fn take_ws(&mut self) -> Result<CdpWsStream> {
        self.ws.take().context("CDP WebSocket 已被取走")
    }

    /// 执行 JavaScript 表达式（绕过 CSP）。
    pub async fn evaluate(&mut self, expression: &str) -> Result<Value> {
        let id = self.next_id();
        let payload = serde_json::json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "awaitPromise": false,
                "allowUnsafeEvalBlockedByCSP": true,
            }
        });
        self.send(&payload).await?;
        self.wait_response(id).await
    }

    /// 注册 Runtime.addBinding（用于 JS → Rust 通信）。
    pub async fn add_binding(&mut self, name: &str) -> Result<()> {
        let id = self.next_id();
        let payload = serde_json::json!({
            "id": id,
            "method": "Runtime.addBinding",
            "params": { "name": name }
        });
        self.send(&payload).await?;
        self.wait_response(id).await?;
        Ok(())
    }

    /// 发送 CDP 消息。
    async fn send(&mut self, payload: &Value) -> Result<()> {
        let text = serde_json::to_string(payload)?;
        self.ws_mut()?
            .send(Message::Text(text))
            .await
            .context("CDP 发送消息失败")?;
        Ok(())
    }

    /// 等待指定 id 的响应。
    async fn wait_response(&mut self, expected_id: u64) -> Result<Value> {
        loop {
            let msg = self.recv().await?;
            if msg.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                if let Some(err) = msg.get("error") {
                    anyhow::bail!("CDP 错误: {err}");
                }
                return Ok(msg);
            }
        }
    }

    /// 读取下一条 CDP 消息。
    pub async fn recv(&mut self) -> Result<Value> {
        loop {
            let msg = self.ws_mut()?.next().await;
            match msg {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(&text).context("解析 CDP 消息失败");
                }
                Some(Ok(Message::Close(_))) | None => {
                    anyhow::bail!("CDP WebSocket 连接已关闭");
                }
                Some(Err(e)) => {
                    anyhow::bail!("CDP WebSocket 读取错误: {e}");
                }
                _ => continue,
            }
        }
    }
}

/// 列出 CDP 调试目标。
pub async fn list_targets(port: u16) -> Result<Vec<Value>> {
    let url = format!("http://127.0.0.1:{port}/json");
    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("CDP 目标列表请求失败: {url}"))?;
    let targets: Vec<Value> = resp.json().await.context("解析 CDP 目标列表失败")?;
    Ok(targets)
}

/// 在目标列表中查找 Codex 页面目标。
/// 优先匹配 title/url 包含 "codex" 的页面。
pub fn find_codex_page(targets: &[Value]) -> Option<String> {
    let pages: Vec<&Value> = targets
        .iter()
        .filter(|t| {
            t.get("type").and_then(|v| v.as_str()) == Some("page")
                && t.get("webSocketDebuggerUrl").is_some()
        })
        .collect();

    // 优先匹配包含 "codex" 的页面
    for page in &pages {
        let title = page
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let url = page
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if title.contains("codex") || url.contains("codex") {
            return page
                .get("webSocketDebuggerUrl")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    }

    // 回退到第一个页面
    pages.first().and_then(|p| {
        p.get("webSocketDebuggerUrl")
            .and_then(|v| v.as_str())
            .map(String::from)
    })
}
