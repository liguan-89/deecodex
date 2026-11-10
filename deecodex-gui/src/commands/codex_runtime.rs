use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{broadcast, mpsc, oneshot, watch, Mutex},
};

use crate::ServerManager;

const RUNTIME_EVENT_NAME: &str = "codex-runtime-event";

enum OutboundMessage {
    Json(Value),
    Shutdown,
}

#[derive(Clone)]
pub struct CodexRuntimeBridge {
    inner: Arc<BridgeInner>,
}

struct BridgeInner {
    endpoint: PathBuf,
    outbound: mpsc::Sender<OutboundMessage>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>,
    events: broadcast::Sender<CodexRuntimeEvent>,
    next_id: AtomicU64,
    next_event_sequence: AtomicU64,
    connected: AtomicBool,
    event_forwarder_started: AtomicBool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexRuntimeEvent {
    pub sequence: u64,
    pub kind: &'static str,
    pub message: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexRuntimeStatus {
    pub connected: bool,
    pub endpoint: String,
    pub transport: &'static str,
    pub attach_mode: &'static str,
    pub shared_runtime: bool,
}

impl CodexRuntimeBridge {
    /// 只附着已经存在的共享 Runtime Socket。这里绝不启动第二个 Codex app-server。
    pub async fn connect_existing() -> Result<Self> {
        let endpoint =
            discover_shared_runtime_socket().context("未发现可附着的 Codex 共享 Runtime Socket")?;
        Self::connect_at(&endpoint).await
    }

    #[cfg(unix)]
    async fn connect_at(endpoint: &Path) -> Result<Self> {
        let stream = tokio::net::UnixStream::connect(endpoint)
            .await
            .with_context(|| format!("连接 Codex 共享 Runtime 失败: {}", endpoint.display()))?;
        let (read_half, mut write_half) = stream.into_split();
        let (outbound, mut outbound_rx) = mpsc::channel(256);
        let (events, _) = broadcast::channel(1024);
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let inner = Arc::new(BridgeInner {
            endpoint: endpoint.to_path_buf(),
            outbound,
            pending: Mutex::new(HashMap::new()),
            events,
            next_id: AtomicU64::new(1),
            next_event_sequence: AtomicU64::new(1),
            connected: AtomicBool::new(true),
            event_forwarder_started: AtomicBool::new(false),
        });

        let writer_inner = inner.clone();
        tokio::spawn(async move {
            while let Some(message) = outbound_rx.recv().await {
                match message {
                    OutboundMessage::Json(value) => {
                        let Ok(line) = serde_json::to_string(&value) else {
                            continue;
                        };
                        if write_half.write_all(line.as_bytes()).await.is_err()
                            || write_half.write_all(b"\n").await.is_err()
                            || write_half.flush().await.is_err()
                        {
                            break;
                        }
                    }
                    OutboundMessage::Shutdown => break,
                }
            }
            writer_inner.connected.store(false, Ordering::Release);
            let _ = shutdown_tx.send(true);
        });

        let reader_inner = inner.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(read_half).lines();
            loop {
                tokio::select! {
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    line = lines.next_line() => {
                        let Ok(Some(line)) = line else {
                            break;
                        };
                        let Ok(message) = serde_json::from_str::<Value>(&line) else {
                            tracing::warn!("Codex Runtime 返回了无法解析的 JSON 行");
                            continue;
                        };
                        dispatch_runtime_message(&reader_inner, message).await;
                    }
                }
            }
            mark_disconnected(&reader_inner, "Codex 共享 Runtime 连接已关闭").await;
        });

        Ok(Self { inner })
    }

    #[cfg(not(unix))]
    async fn connect_at(_endpoint: &Path) -> Result<Self> {
        bail!("当前 Phase 0 PoC 仅实现 Unix Socket 附着；Windows Named Pipe 尚未实现")
    }

    pub fn status(&self) -> CodexRuntimeStatus {
        CodexRuntimeStatus {
            connected: self.is_connected(),
            endpoint: self.inner.endpoint.to_string_lossy().to_string(),
            transport: "unix_socket",
            attach_mode: "existing_shared_runtime",
            shared_runtime: true,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.inner.connected.load(Ordering::Acquire)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<CodexRuntimeEvent> {
        self.inner.events.subscribe()
    }

    pub fn start_tauri_event_forwarder(&self, app: AppHandle) -> bool {
        if self
            .inner
            .event_forwarder_started
            .swap(true, Ordering::AcqRel)
        {
            return false;
        }
        let mut receiver = self.subscribe_events();
        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let _ = app.emit(RUNTIME_EVENT_NAME, event);
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!(count, "Codex Runtime Tauri 事件转发落后");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        true
    }

    pub async fn list_threads(&self, limit: u32) -> Result<Value> {
        self.request(
            "thread/list",
            json!({
                "limit": limit.clamp(1, 200),
                "sortKey": "recency_at",
                "sortDirection": "desc",
                "useStateDbOnly": true
            }),
        )
        .await
    }

    pub async fn resume_thread(&self, thread_id: &str) -> Result<Value> {
        if thread_id.trim().is_empty() {
            bail!("thread_id 不能为空");
        }
        self.request("thread/resume", json!({ "threadId": thread_id }))
            .await
    }

    pub async fn shutdown(&self) {
        let _ = self.inner.outbound.send(OutboundMessage::Shutdown).await;
        mark_disconnected(&self.inner, "DEX AI 已断开 Codex Runtime Bridge").await;
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        if !self.is_connected() {
            bail!("Codex Runtime Bridge 未连接");
        }
        // 当前透明 Shim 会隐藏 mobile-codex-* 响应，避免附加客户端响应进入桌面主连接。
        let id = format!(
            "mobile-codex-deecodex-{}-{}",
            std::process::id(),
            self.inner.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id.clone(), tx);
        let message = json!({ "id": id, "method": method, "params": params });
        if self
            .inner
            .outbound
            .send(OutboundMessage::Json(message))
            .await
            .is_err()
        {
            self.inner.pending.lock().await.remove(&id);
            bail!("Codex Runtime 写入通道已关闭");
        }

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(message))) => Err(anyhow!(message)),
            Ok(Err(_)) => Err(anyhow!("Codex Runtime 响应通道已关闭")),
            Err(_) => {
                self.inner.pending.lock().await.remove(&id);
                bail!("Codex Runtime 请求超时: {method}")
            }
        }
    }
}

async fn dispatch_runtime_message(inner: &Arc<BridgeInner>, message: Value) {
    let kind = if message.get("method").is_some() {
        if message.get("id").is_some() {
            "request"
        } else {
            "notification"
        }
    } else {
        "response"
    };
    let sequence = inner.next_event_sequence.fetch_add(1, Ordering::Relaxed);
    let _ = inner.events.send(CodexRuntimeEvent {
        sequence,
        kind,
        message: message.clone(),
    });

    let Some(id) = message.get("id").and_then(request_id_key) else {
        return;
    };
    let Some((_, pending)) = inner.pending.lock().await.remove_entry(&id) else {
        return;
    };
    if let Some(error) = message.get("error") {
        let text = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        let _ = pending.send(Err(text));
    } else {
        let _ = pending.send(Ok(message.get("result").cloned().unwrap_or(Value::Null)));
    }
}

async fn mark_disconnected(inner: &Arc<BridgeInner>, reason: &str) {
    if !inner.connected.swap(false, Ordering::AcqRel) {
        return;
    }
    let mut pending = inner.pending.lock().await;
    for (_, sender) in pending.drain() {
        let _ = sender.send(Err(reason.to_string()));
    }
}

fn request_id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn discover_shared_runtime_socket() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DEECODEX_CODEX_RUNTIME_SOCKET") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let path = home
            .join("Library")
            .join("Application Support")
            .join("dev.mobilecodex.MobileCodex")
            .join("shim.sock");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

pub async fn ensure_bridge(manager: &ServerManager) -> Result<Arc<CodexRuntimeBridge>, String> {
    let mut slot = manager.codex_runtime_bridge.lock().await;
    if let Some(bridge) = slot.as_ref() {
        if bridge.is_connected() {
            return Ok(bridge.clone());
        }
    }

    let bridge = Arc::new(
        CodexRuntimeBridge::connect_existing()
            .await
            .map_err(|error| error.to_string())?,
    );
    if let Some(app) = manager.app_handle.lock().await.as_ref().cloned() {
        bridge.start_tauri_event_forwarder(app);
    }
    *slot = Some(bridge.clone());
    Ok(bridge)
}

pub async fn attach_on_gateway_start(
    manager: &ServerManager,
) -> Result<CodexRuntimeStatus, String> {
    Ok(ensure_bridge(manager).await?.status())
}

pub async fn detach_on_gateway_stop(manager: &ServerManager) {
    if let Some(bridge) = manager.codex_runtime_bridge.lock().await.take() {
        bridge.shutdown().await;
    }
}

#[tauri::command]
pub async fn codex_runtime_status(
    manager: State<'_, ServerManager>,
) -> Result<CodexRuntimeStatus, String> {
    Ok(ensure_bridge(&manager).await?.status())
}

#[tauri::command]
pub async fn codex_runtime_threads_list(
    manager: State<'_, ServerManager>,
    limit: Option<u32>,
) -> Result<Value, String> {
    ensure_bridge(&manager)
        .await?
        .list_threads(limit.unwrap_or(50))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn codex_runtime_thread_resume(
    manager: State<'_, ServerManager>,
    thread_id: String,
) -> Result<Value, String> {
    ensure_bridge(&manager)
        .await?
        .resume_thread(&thread_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn codex_runtime_events_subscribe(
    app: AppHandle,
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let bridge = ensure_bridge(&manager).await?;
    let started = bridge.start_tauri_event_forwarder(app);
    Ok(json!({
        "event": RUNTIME_EVENT_NAME,
        "connected": bridge.is_connected(),
        "forwarder_started": started
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::net::UnixListener;

    async fn fake_runtime(socket_path: PathBuf) -> tokio::task::JoinHandle<()> {
        let listener = UnixListener::bind(socket_path).expect("绑定测试 Runtime Socket 失败");
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("接受测试连接失败");
            let (read_half, mut write_half) = stream.into_split();
            let mut lines = BufReader::new(read_half).lines();
            while let Some(line) = lines.next_line().await.expect("读取测试请求失败") {
                let request: Value = serde_json::from_str(&line).expect("请求 JSON 无效");
                assert_ne!(
                    request.get("method").and_then(Value::as_str),
                    Some("initialize"),
                    "附着已有 Runtime 时不得重复 initialize"
                );
                let id = request.get("id").cloned().expect("请求缺少 id");
                let method = request
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("请求缺少 method");
                let notification = json!({
                    "method": "thread/status/changed",
                    "params": { "threadId": "shared-thread", "status": "idle" }
                });
                write_half
                    .write_all(
                        format!("{}\n", serde_json::to_string(&notification).unwrap()).as_bytes(),
                    )
                    .await
                    .unwrap();
                let result = match method {
                    "thread/list" => json!({
                        "id": id,
                        "result": { "data": [{ "id": "shared-thread" }], "nextCursor": null }
                    }),
                    "thread/resume" => json!({
                        "id": id,
                        "result": { "thread": { "id": "shared-thread" } }
                    }),
                    _ => json!({
                        "id": id,
                        "error": { "code": -32601, "message": "method not found" }
                    }),
                };
                write_half
                    .write_all(format!("{}\n", serde_json::to_string(&result).unwrap()).as_bytes())
                    .await
                    .unwrap();
                write_half.flush().await.unwrap();
            }
        })
    }

    #[tokio::test]
    async fn shared_runtime_supports_thread_list_resume_and_events() {
        let dir = TempDir::new().unwrap();
        let socket_path = dir.path().join("runtime.sock");
        let server = fake_runtime(socket_path.clone()).await;
        let bridge = CodexRuntimeBridge::connect_at(&socket_path)
            .await
            .expect("连接测试 Runtime 失败");
        let mut events = bridge.subscribe_events();

        let list = bridge.list_threads(10).await.expect("thread/list 失败");
        assert_eq!(list["data"][0]["id"], "shared-thread");

        let first_event = events.recv().await.expect("未收到 Runtime 事件");
        assert_eq!(first_event.kind, "notification");
        assert_eq!(first_event.message["method"], "thread/status/changed");
        assert_eq!(first_event.message["params"]["threadId"], "shared-thread");

        let list_response_event = events.recv().await.expect("未收到 thread/list 响应事件");
        assert_eq!(list_response_event.kind, "response");
        assert_eq!(
            list_response_event.message["result"]["data"][0]["id"],
            "shared-thread"
        );

        let resumed = bridge
            .resume_thread("shared-thread")
            .await
            .expect("thread/resume 失败");
        assert_eq!(resumed["thread"]["id"], "shared-thread");

        bridge.shutdown().await;
        server.abort();
    }

    #[tokio::test]
    #[ignore = "需要当前 ChatGPT 通过透明 Shim 启动同一个 Codex app-server"]
    async fn live_shared_runtime_thread_list_and_resume() {
        let bridge = CodexRuntimeBridge::connect_existing()
            .await
            .expect("无法附着当前共享 Runtime");
        let list = bridge.list_threads(3).await.expect("实时 thread/list 失败");
        let threads = list["data"].as_array().expect("thread/list 缺少 data");
        assert!(!threads.is_empty(), "当前 Runtime 没有返回线程");
        let thread_id = threads[0]["id"].as_str().expect("线程缺少 id");
        let resumed = bridge
            .resume_thread(thread_id)
            .await
            .expect("实时 thread/resume 失败");
        assert_eq!(resumed["thread"]["id"], thread_id);
        bridge.shutdown().await;
    }
}
