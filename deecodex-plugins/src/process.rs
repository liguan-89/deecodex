use anyhow::{Context, Result};
use dashmap::DashMap;
use futures_util::StreamExt;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::{info, warn};

static DEFAULT_MODEL_CACHE: OnceLock<String> = OnceLock::new();

use crate::manifest::PluginManifest;
use crate::protocol::PluginEvent;
use crate::rpc::{JsonRpcMessage, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// 运行时插件实例的通信手柄
pub struct PluginProcessHandle {
    pub child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pub pending: Arc<DashMap<u64, oneshot::Sender<JsonRpcResponse>>>,
    pub request_id: std::sync::atomic::AtomicU64,
    pub stdout_task: tokio::task::JoinHandle<()>,
    /// 当 stdout 读取循环退出时触发 (进程已退出)
    pub exit_rx: Option<oneshot::Receiver<()>>,
}

impl PluginProcessHandle {
    /// 发送 JSON-RPC 请求并等待响应（30s 超时）
    pub async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse> {
        let id = self
            .request_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = JsonRpcRequest::new(id, method, params);
        let msg = JsonRpcMessage::Request(req);
        let line = msg.to_line() + "\n";

        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);

        {
            let mut stdin = self.stdin.lock().await;
            stdin
                .write_all(line.as_bytes())
                .await
                .context("写入 stdin 失败")?;
            stdin.flush().await.context("刷新 stdin 失败")?;
        }

        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                self.pending.remove(&id);
                anyhow::bail!("请求 channel 已关闭")
            }
            Err(_) => {
                self.pending.remove(&id);
                anyhow::bail!("请求超时 (30s)")
            }
        }
    }

    /// 返回 stdin 写入器的 Arc 克隆，用于在释放 DashMap Ref 后写入
    pub fn stdin_clone(&self) -> Arc<Mutex<ChildStdin>> {
        self.stdin.clone()
    }

    /// 发送 JSON-RPC 通知（无响应）
    pub async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notif = JsonRpcNotification::new(method, params);
        let msg = JsonRpcMessage::Notification(notif);
        let line = msg.to_line() + "\n";

        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .context("写入 stdin 失败")?;
        stdin.flush().await.context("刷新 stdin 失败")?;
        Ok(())
    }
}

/// 自动获取 deecodex 第一个可用模型，缓存结果避免重复请求
pub(crate) async fn resolve_default_model(llm_base_url: &str) -> String {
    if let Some(cached) = DEFAULT_MODEL_CACHE.get() {
        return cached.clone();
    }
    let url = format!("{}/v1/models", llm_base_url);
    let fallback = "deepseek-v4-pro".to_string();
    let client = reqwest::Client::new();
    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) => {
            if let Ok(json) = resp.json::<Value>().await {
                if let Some(models) = json["data"].as_array() {
                    if let Some(first) = models.first() {
                        if let Some(id) = first["id"].as_str() {
                            let model = id.to_string();
                            let _ = DEFAULT_MODEL_CACHE.set(model.clone());
                            return model;
                        }
                    }
                }
            }
            let _ = DEFAULT_MODEL_CACHE.set(fallback.clone());
            fallback
        }
        Err(_) => {
            let _ = DEFAULT_MODEL_CACHE.set(fallback.clone());
            fallback
        }
    }
}

/// 启动插件子进程，建立 stdio 管道，启动 stdout 读取任务
pub async fn spawn_plugin(
    manifest: &PluginManifest,
    install_dir: &Path,
    _data_dir: &Path,
    llm_base_url: &str,
    _config: &Value,
    events_tx: broadcast::Sender<PluginEvent>,
) -> Result<PluginProcessHandle> {
    let script_path = install_dir.join(&manifest.entry.script);

    let (program, args) = match manifest.entry.runtime.as_str() {
        "node" => {
            let node = find_executable("node").unwrap_or_else(|| "node".into());
            let mut args = vec![script_path.to_string_lossy().to_string()];
            args.extend(manifest.entry.args.iter().cloned());
            (node, args)
        }
        "python" => {
            let python = find_executable("python3")
                .or_else(|| find_executable("python"))
                .unwrap_or_else(|| "python3".into());
            let mut args = vec![script_path.to_string_lossy().to_string()];
            args.extend(manifest.entry.args.iter().cloned());
            (python, args)
        }
        "binary" => {
            let bin = script_path.to_string_lossy().to_string();
            let args = manifest.entry.args.clone();
            (bin, args)
        }
        _ => anyhow::bail!("不支持的运行时: {}", manifest.entry.runtime),
    };

    let mut cmd = Command::new(&program);
    cmd.args(&args)
        .current_dir(install_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("无法启动插件进程: {program} {}", args.join(" ")))?;

    let stdin = child.stdin.take().context("无法获取 stdin")?;
    let stdout = child.stdout.take().context("无法获取 stdout")?;
    let stderr = child.stderr.take().context("无法获取 stderr")?;

    let stdin_arc = Arc::new(Mutex::new(stdin));

    let pending: Arc<DashMap<u64, oneshot::Sender<JsonRpcResponse>>> = Arc::new(DashMap::new());

    let pending_clone = pending.clone();
    let events_tx_clone = events_tx.clone();
    let plugin_id = manifest.id.clone();
    let stdin_for_reader = stdin_arc.clone();
    let llm_url = llm_base_url.to_string();
    let (exit_tx, exit_rx) = oneshot::channel();
    let stdout_task = tokio::spawn(async move {
        read_stdout_loop(
            plugin_id,
            stdout,
            pending_clone,
            events_tx_clone,
            stdin_for_reader,
            llm_url,
        )
        .await;
        let _ = exit_tx.send(());
    });

    // 读取 stderr 并记录
    let plugin_id_for_stderr = manifest.id.clone();
    tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            warn!(plugin_id = %plugin_id_for_stderr, "[stderr] {line}");
        }
    });

    Ok(PluginProcessHandle {
        child,
        stdin: stdin_arc,
        pending,
        request_id: std::sync::atomic::AtomicU64::new(1),
        stdout_task,
        exit_rx: Some(exit_rx),
    })
}

/// 持续读取插件 stdout，解析 JSON-RPC 消息并分发
async fn read_stdout_loop(
    plugin_id: String,
    stdout: ChildStdout,
    pending: Arc<DashMap<u64, oneshot::Sender<JsonRpcResponse>>>,
    events_tx: broadcast::Sender<PluginEvent>,
    stdin: Arc<Mutex<ChildStdin>>,
    llm_base_url: String,
) {
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let msg = match JsonRpcMessage::from_line(&line) {
            Some(m) => m,
            None => {
                warn!(plugin_id = %plugin_id, "无法解析 JSON-RPC 消息: {line}");
                continue;
            }
        };

        match msg {
            JsonRpcMessage::Response(resp) => {
                if let Some((_, tx)) = pending.remove(&resp.id) {
                    let _ = tx.send(resp);
                }
            }
            JsonRpcMessage::Notification(notif) => {
                handle_notification(&plugin_id, &notif, &events_tx);
            }
            JsonRpcMessage::Request(req) => {
                handle_request_from_plugin(&plugin_id, &req, &events_tx, &stdin, &llm_base_url)
                    .await;
            }
        }
    }

    info!(plugin_id = %plugin_id, "插件 stdout 已关闭");
}

/// 处理插件发来的通知（状态、日志、QR码等）
fn handle_notification(
    plugin_id: &str,
    notif: &JsonRpcNotification,
    events_tx: &broadcast::Sender<PluginEvent>,
) {
    match notif.method.as_str() {
        "log" => {
            if let Some(params) = &notif.params {
                let level = params["level"].as_str().unwrap_or("info");
                let message = params["message"].as_str().unwrap_or("");
                match level {
                    "error" => tracing::error!(plugin_id = %plugin_id, "[plugin] {message}"),
                    "warn" => tracing::warn!(plugin_id = %plugin_id, "[plugin] {message}"),
                    _ => tracing::info!(plugin_id = %plugin_id, "[plugin] {message}"),
                }
                let _ = events_tx.send(PluginEvent::Log {
                    plugin_id: plugin_id.into(),
                    level: level.into(),
                    message: message.into(),
                });
            }
        }
        "status" => {
            if let Some(params) = &notif.params {
                let account_id = params["account_id"].as_str().unwrap_or("");
                let status_str = params["status"].as_str().unwrap_or("disconnected");
                let status = match status_str {
                    "connected" => crate::protocol::AccountStatus::Connected,
                    "connecting" => crate::protocol::AccountStatus::Connecting,
                    "login_expired" => crate::protocol::AccountStatus::LoginExpired,
                    "error" => crate::protocol::AccountStatus::Error,
                    _ => crate::protocol::AccountStatus::Disconnected,
                };
                let _ = events_tx.send(PluginEvent::StatusChanged {
                    plugin_id: plugin_id.into(),
                    account_id: account_id.into(),
                    status,
                });
            }
        }
        "qr_code" => {
            if let Some(params) = &notif.params {
                let account_id = params["account_id"].as_str().unwrap_or("");
                let data_url = params["data_url"].as_str().unwrap_or("");
                let _ = events_tx.send(PluginEvent::QrCode {
                    plugin_id: plugin_id.into(),
                    account_id: account_id.into(),
                    data_url: data_url.into(),
                });
            }
        }
        _ => {
            info!(plugin_id = %plugin_id, method = %notif.method, "未处理的插件通知");
        }
    }
}

/// 处理插件发来的请求（llm.call 等）
async fn handle_request_from_plugin(
    plugin_id: &str,
    req: &JsonRpcRequest,
    _events_tx: &broadcast::Sender<PluginEvent>,
    stdin: &Arc<Mutex<ChildStdin>>,
    llm_base_url: &str,
) {
    match req.method.as_str() {
        "llm.call" => {
            let params = req.params.as_ref();
            let messages = params
                .and_then(|p| p.get("messages").cloned())
                .unwrap_or(Value::Null);
            let msg_count = messages.as_array().map(|a| a.len()).unwrap_or(0);
            let model = match params.and_then(|p| p.get("model").and_then(|v| v.as_str())) {
                Some(m) if !m.is_empty() && m != "auto" => m.to_string(),
                _ => resolve_default_model(llm_base_url).await,
            };
            let system_prompt = params
                .and_then(|p| p.get("system_prompt").and_then(|v| v.as_str()))
                .map(|s| s.to_string());

            tracing::info!(
                plugin_id = %plugin_id,
                request_id = %req.id,
                model = %model,
                msg_count = %msg_count,
                llm_base_url = %llm_base_url,
                "[llm.call] 收到 LLM 请求"
            );

            let result = proxy_llm_call(
                &messages,
                &model,
                system_prompt.as_deref(),
                llm_base_url,
                stdin,
                req.id,
            )
            .await;

            match &result {
                Ok(content) => tracing::info!(
                    plugin_id = %plugin_id,
                    request_id = %req.id,
                    content_len = %content.len(),
                    "[llm.call] LLM 调用成功"
                ),
                Err(err) => tracing::error!(
                    plugin_id = %plugin_id,
                    request_id = %req.id,
                    error = %err,
                    "[llm.call] LLM 调用失败"
                ),
            }

            let response = match result {
                Ok(content) => JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: Some(serde_json::json!({ "content": content })),
                    error: None,
                },
                Err(err) => JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: req.id,
                    result: None,
                    error: Some(crate::rpc::JsonRpcError {
                        code: -32603,
                        message: format!("LLM 调用失败: {err}"),
                        data: None,
                    }),
                },
            };

            let msg = JsonRpcMessage::Response(response);
            let line = msg.to_line() + "\n";
            let mut guard = stdin.lock().await;
            guard.write_all(line.as_bytes()).await.ok();
            guard.flush().await.ok();
        }
        _ => {
            warn!(plugin_id = %plugin_id, method = %req.method, "不支持的插件请求方法");
            // 返回错误响应
            let response = JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id,
                result: None,
                error: Some(crate::rpc::JsonRpcError {
                    code: -32601,
                    message: format!("方法未找到: {}", req.method),
                    data: None,
                }),
            };
            let msg = JsonRpcMessage::Response(response);
            let line = msg.to_line() + "\n";
            let mut guard = stdin.lock().await;
            guard.write_all(line.as_bytes()).await.ok();
            guard.flush().await.ok();
        }
    }
}

/// 代理 LLM 调用到 deecodex HTTP API
async fn proxy_llm_call(
    messages: &Value,
    model: &str,
    system_prompt: Option<&str>,
    llm_base_url: &str,
    stdin: &Arc<Mutex<ChildStdin>>,
    request_id: u64,
) -> anyhow::Result<String> {
    let mut payload = serde_json::json!({
        "model": model,
        "input": messages,
        "stream": true,
    });
    if let Some(sp) = system_prompt {
        payload["instructions"] = serde_json::Value::String(sp.to_string());
    }

    let url = format!("{}/v1/responses", llm_base_url);
    tracing::info!(%url, model = %model, msg_count = %messages.as_array().map(|a| a.len()).unwrap_or(0), "[proxy_llm_call] 发起 HTTP 请求");

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .json(&payload)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .with_context(|| format!("LLM 请求失败: {url}"))?;

    let status = response.status();
    tracing::info!(%status, %url, "[proxy_llm_call] 收到响应");
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        tracing::error!(%status, %body, "[proxy_llm_call] deecodex 返回错误");
        anyhow::bail!("deecodex 返回错误 {status}: {body}");
    }

    let mut stream = response.bytes_stream();
    let mut full_text = String::new();
    let mut stream_idx: u64 = 0;

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.with_context(|| "读取 SSE 流失败")?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        for line in chunk_str.lines() {
            let data = line.strip_prefix("data: ").unwrap_or(line);
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            if let Ok(val) = serde_json::from_str::<Value>(data) {
                if let Some(delta) = val["delta"].as_str() {
                    full_text.push_str(delta);
                }
                if let Some(content) = val["output_text"].as_str() {
                    full_text.push_str(content);
                }
                // 流式发送 chunk 给插件
                let notif = JsonRpcNotification::new(
                    "llm.stream_chunk",
                    Some(serde_json::json!({
                        "id": request_id,
                        "index": stream_idx,
                        "chunk": data,
                    })),
                );
                let msg = JsonRpcMessage::Notification(notif);
                let line = msg.to_line() + "\n";
                let mut guard = stdin.lock().await;
                guard.write_all(line.as_bytes()).await.ok();
                guard.flush().await.ok();
                stream_idx += 1;
            }
        }
    }

    Ok(full_text)
}

/// 停止插件：发 shutdown 通知 → 等待5秒 → 强杀
pub async fn shutdown_plugin(mut handle: PluginProcessHandle) -> Result<()> {
    let _ = handle.send_notification("shutdown", None).await;

    // 等待进程自行退出
    match tokio::time::timeout(std::time::Duration::from_secs(5), handle.child.wait()).await {
        Ok(Ok(status)) => {
            info!("插件进程已退出: {status}");
        }
        Ok(Err(e)) => {
            warn!("等待插件退出时出错: {e}");
        }
        Err(_) => {
            warn!("插件未在 5 秒内退出，强制终止");
            let _ = handle.child.kill().await;
        }
    }

    // 取消 stdout 读取任务
    handle.stdout_task.abort();
    Ok(())
}

/// 查找可执行文件路径
fn find_executable(name: &str) -> Option<String> {
    // 先检查 PATH
    if let Ok(paths) = std::env::var("PATH") {
        for dir in paths.split(':') {
            let full = std::path::Path::new(dir).join(name);
            if full.exists() {
                return Some(full.to_string_lossy().to_string());
            }
        }
    }
    // 系统路径
    for prefix in ["/opt/homebrew/bin", "/usr/local/bin"] {
        let full = std::path::Path::new(prefix).join(name);
        if full.exists() {
            return Some(full.to_string_lossy().to_string());
        }
    }
    // 用户目录下的版本管理器路径
    if let Some(home) = dirs_fallback() {
        for sub in [".n/bin", ".local/bin", ".volta/bin"] {
            let full = home.join(sub).join(name);
            if full.exists() {
                return Some(full.to_string_lossy().to_string());
            }
        }
        // nvm: 扫描已安装版本
        let nvm_dir = home.join(".nvm/versions/node");
        if let Ok(entries) = std::fs::read_dir(&nvm_dir) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin").join(name);
                if bin.exists() {
                    return Some(bin.to_string_lossy().to_string());
                }
            }
        }
        // fnm
        let fnm_dir = home.join(".fnm/node-versions");
        if let Ok(entries) = std::fs::read_dir(&fnm_dir) {
            for entry in entries.flatten() {
                let bin = entry.path().join("installation/bin").join(name);
                if bin.exists() {
                    return Some(bin.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}

fn dirs_fallback() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(std::path::PathBuf::from)
}
