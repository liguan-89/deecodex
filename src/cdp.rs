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

    /// 注册脚本到所有新页面（Page.addScriptToEvaluateOnNewDocument）。
    /// 确保 SPA 导航/页面重载后注入脚本仍然生效。
    pub async fn add_script_to_new_documents(&mut self, script: &str) -> Result<()> {
        let id = self.next_id();
        let payload = serde_json::json!({
            "id": id,
            "method": "Page.addScriptToEvaluateOnNewDocument",
            "params": { "source": script }
        });
        self.send(&payload).await?;
        self.wait_response(id).await?;
        Ok(())
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

    /// 启用 Fetch 域拦截。
    ///
    /// `patterns` 形如 `["ab.chatgpt.com", "ab.chatgpt.com/*"]`，控制哪些 URL 走拦截。
    /// 调用后 Codex 发出的匹配请求会触发 `Fetch.requestPaused` 事件，
    /// 必须用 `fetch_continue_request` / `fetch_fulfill_request` / `fetch_fail_request` 之一响应。
    pub async fn fetch_enable(&mut self, patterns: &[&str]) -> Result<()> {
        let id = self.next_id();
        let payload = serde_json::json!({
            "id": id,
            "method": "Fetch.enable",
            "params": {
                "patterns": patterns.iter().map(|p| serde_json::json!({"urlPattern": p})).collect::<Vec<_>>()
            }
        });
        self.send(&payload).await?;
        self.wait_response(id).await?;
        Ok(())
    }

    /// 放行一个被暂停的请求（继续走真实网络）。
    #[allow(dead_code)]
    pub async fn fetch_continue_request(&mut self, request_id: &str) -> Result<()> {
        let payload = serde_json::json!({
            "id": self.next_id(),
            "method": "Fetch.continueRequest",
            "params": { "requestId": request_id }
        });
        self.send(&payload).await
    }

    /// 用合成响应直接回填被暂停的请求（不发出真实网络请求）。
    /// `body` 必须是 base64 编码后的字符串（CDP 协议要求）。
    #[allow(dead_code)]
    pub async fn fetch_fulfill_request(
        &mut self,
        request_id: &str,
        status: u16,
        headers: &[(&str, &str)],
        body_b64: &str,
    ) -> Result<()> {
        let response_headers: Vec<Value> = headers
            .iter()
            .map(|(k, v)| serde_json::json!({"name": k, "value": v}))
            .collect();
        let payload = serde_json::json!({
            "id": self.next_id(),
            "method": "Fetch.fulfillRequest",
            "params": {
                "requestId": request_id,
                "responseCode": status,
                "responseHeaders": response_headers,
                "body": body_b64,
            }
        });
        self.send(&payload).await
    }

    /// 让被暂停的请求失败（被 Network.aborted 之类的）。
    #[allow(dead_code)]
    pub async fn fetch_fail_request(&mut self, request_id: &str, reason: &str) -> Result<()> {
        let payload = serde_json::json!({
            "id": self.next_id(),
            "method": "Fetch.failRequest",
            "params": { "requestId": request_id, "errorReason": reason }
        });
        self.send(&payload).await
    }

    /// 通过已取走的 WebSocket 直接发送 CDP 消息（事件循环内使用）。
    /// 避免在桥接循环里再造一层 CdpClient 包装。
    pub async fn send_raw(ws: &mut CdpWsStream, payload: &Value) -> Result<()> {
        let text = serde_json::to_string(payload)?;
        ws.send(Message::Text(text))
            .await
            .context("CDP 发送消息失败")?;
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
