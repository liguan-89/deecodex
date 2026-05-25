use anyhow::{Context, Result};
use dashmap::DashMap;
use futures_util::StreamExt;
use serde_json::json;
use serde_json::Value;
use std::ffi::OsStr;
use std::path::Path;
use std::path::{Component, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{broadcast, oneshot, Mutex};
use tracing::{info, warn};

static DEFAULT_MODEL_CACHE: OnceLock<String> = OnceLock::new();

use crate::manifest::PluginManifest;
use crate::protocol::{
    PluginAssetPaths, PluginEvent, METHOD_ASSETS_DELETE, METHOD_ASSETS_LIST, METHOD_ASSETS_READ,
    METHOD_ASSETS_WRITE, METHOD_CACHE_CLEAR, METHOD_CACHE_READ, METHOD_CACHE_WRITE,
    METHOD_SECRETS_DELETE, METHOD_SECRETS_GET, METHOD_SECRETS_SET,
};
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

struct PluginStdoutLoopContext {
    plugin_id: String,
    stdout: ChildStdout,
    pending: Arc<DashMap<u64, oneshot::Sender<JsonRpcResponse>>>,
    events_tx: broadcast::Sender<PluginEvent>,
    stdin: Arc<Mutex<ChildStdin>>,
    llm_base_url: String,
    asset_paths: PluginAssetPaths,
    permissions: Vec<String>,
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
    asset_paths: &PluginAssetPaths,
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
        .env("DEECODEX_PLUGIN_ID", &manifest.id)
        .env("DEECODEX_PLUGIN_INSTALL_DIR", install_dir)
        .env("DEECODEX_PLUGIN_DATA_DIR", &asset_paths.data_dir)
        .env("DEECODEX_PLUGIN_CACHE_DIR", &asset_paths.cache_dir)
        .env("DEECODEX_PLUGIN_SECRETS_DIR", &asset_paths.secrets_dir)
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
    let asset_paths_for_reader = asset_paths.clone();
    let permissions_for_reader = manifest.permissions.clone();
    let (exit_tx, exit_rx) = oneshot::channel();
    let stdout_task = tokio::spawn(async move {
        read_stdout_loop(PluginStdoutLoopContext {
            plugin_id,
            stdout,
            pending: pending_clone,
            events_tx: events_tx_clone,
            stdin: stdin_for_reader,
            llm_base_url: llm_url,
            asset_paths: asset_paths_for_reader,
            permissions: permissions_for_reader,
        })
        .await;
        let _ = exit_tx.send(());
    });

    // 读取 stderr 并记录
    let plugin_id_for_stderr = manifest.id.clone();
    let events_tx_for_stderr = events_tx.clone();
    tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            warn!(plugin_id = %plugin_id_for_stderr, "[stderr] {line}");
            let _ = events_tx_for_stderr.send(PluginEvent::Log {
                plugin_id: plugin_id_for_stderr.clone(),
                level: "warn".into(),
                message: truncate_event_message(&format!("[stderr] {line}")),
            });
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
async fn read_stdout_loop(context: PluginStdoutLoopContext) {
    let PluginStdoutLoopContext {
        plugin_id,
        stdout,
        pending,
        events_tx,
        stdin,
        llm_base_url,
        asset_paths,
        permissions,
    } = context;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    while let Ok(Some(line)) = lines.next_line().await {
        let msg = match JsonRpcMessage::from_line(&line) {
            Some(m) => m,
            None => {
                warn!(plugin_id = %plugin_id, "无法解析 JSON-RPC 消息: {line}");
                let _ = events_tx.send(PluginEvent::Log {
                    plugin_id: plugin_id.clone(),
                    level: "warn".into(),
                    message: truncate_event_message(&format!("无法解析 JSON-RPC 消息: {line}")),
                });
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
                handle_request_from_plugin(
                    &plugin_id,
                    &req,
                    &events_tx,
                    &stdin,
                    &llm_base_url,
                    &asset_paths,
                    &permissions,
                )
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
            let _ = events_tx.send(PluginEvent::Log {
                plugin_id: plugin_id.into(),
                level: "warn".into(),
                message: format!("未处理的插件通知: {}", notif.method),
            });
        }
    }
}

fn truncate_event_message(message: &str) -> String {
    const MAX_LEN: usize = 600;
    if message.chars().count() <= MAX_LEN {
        return message.to_string();
    }
    let mut text = message.chars().take(MAX_LEN).collect::<String>();
    text.push('…');
    text
}

/// 处理插件发来的请求（llm.call 等）
async fn handle_request_from_plugin(
    plugin_id: &str,
    req: &JsonRpcRequest,
    events_tx: &broadcast::Sender<PluginEvent>,
    stdin: &Arc<Mutex<ChildStdin>>,
    llm_base_url: &str,
    asset_paths: &PluginAssetPaths,
    permissions: &[String],
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
        METHOD_ASSETS_READ
        | METHOD_ASSETS_WRITE
        | METHOD_ASSETS_LIST
        | METHOD_ASSETS_DELETE
        | METHOD_CACHE_READ
        | METHOD_CACHE_WRITE
        | METHOD_CACHE_CLEAR
        | METHOD_SECRETS_SET
        | METHOD_SECRETS_GET
        | METHOD_SECRETS_DELETE => {
            let result =
                handle_storage_request(plugin_id, req, asset_paths, permissions, events_tx).await;
            match result {
                Ok(value) => send_rpc_success(stdin, req.id, value).await,
                Err(error) => send_rpc_error(stdin, req.id, -32603, error.to_string()).await,
            }
        }
        _ => {
            warn!(plugin_id = %plugin_id, method = %req.method, "不支持的插件请求方法");
            send_rpc_error(stdin, req.id, -32601, format!("方法未找到: {}", req.method)).await;
        }
    }
}

async fn send_rpc_success(stdin: &Arc<Mutex<ChildStdin>>, id: u64, result: Value) {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: Some(result),
        error: None,
    };
    let msg = JsonRpcMessage::Response(response);
    let line = msg.to_line() + "\n";
    let mut guard = stdin.lock().await;
    guard.write_all(line.as_bytes()).await.ok();
    guard.flush().await.ok();
}

async fn send_rpc_error(stdin: &Arc<Mutex<ChildStdin>>, id: u64, code: i64, message: String) {
    let response = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        id,
        result: None,
        error: Some(crate::rpc::JsonRpcError {
            code,
            message,
            data: None,
        }),
    };
    let msg = JsonRpcMessage::Response(response);
    let line = msg.to_line() + "\n";
    let mut guard = stdin.lock().await;
    guard.write_all(line.as_bytes()).await.ok();
    guard.flush().await.ok();
}

async fn handle_storage_request(
    plugin_id: &str,
    req: &JsonRpcRequest,
    asset_paths: &PluginAssetPaths,
    permissions: &[String],
    events_tx: &broadcast::Sender<PluginEvent>,
) -> anyhow::Result<Value> {
    let result = match req.method.as_str() {
        METHOD_ASSETS_READ => {
            require_permission(permissions, &["fs.read", "file.read"])?;
            read_text_file(
                &PathBuf::from(&asset_paths.data_dir),
                request_path(req.params.as_ref())?,
            )
            .map(|content| json!({ "content": content, "encoding": "utf8" }))
        }
        METHOD_ASSETS_WRITE => {
            require_permission(permissions, &["fs.write", "file.write"])?;
            write_text_file(
                &PathBuf::from(&asset_paths.data_dir),
                request_path(req.params.as_ref())?,
                request_content(req.params.as_ref())?,
                request_append(req.params.as_ref()),
            )
            .map(|bytes| json!({ "ok": true, "bytes": bytes }))
        }
        METHOD_ASSETS_LIST => {
            require_permission(permissions, &["fs.read", "file.read"])?;
            list_files(
                &PathBuf::from(&asset_paths.data_dir),
                request_path_or_empty(req.params.as_ref()),
            )
            .map(|items| json!({ "items": items }))
        }
        METHOD_ASSETS_DELETE => {
            require_permission(permissions, &["fs.write", "file.write"])?;
            delete_path(
                &PathBuf::from(&asset_paths.data_dir),
                request_path(req.params.as_ref())?,
            )
            .map(|deleted| json!({ "ok": true, "deleted": deleted }))
        }
        METHOD_CACHE_READ => {
            require_permission(permissions, &["fs.read", "file.read"])?;
            read_text_file(
                &PathBuf::from(&asset_paths.cache_dir),
                request_path(req.params.as_ref())?,
            )
            .map(|content| json!({ "content": content, "encoding": "utf8" }))
        }
        METHOD_CACHE_WRITE => {
            require_permission(permissions, &["fs.write", "file.write"])?;
            write_text_file(
                &PathBuf::from(&asset_paths.cache_dir),
                request_path(req.params.as_ref())?,
                request_content(req.params.as_ref())?,
                request_append(req.params.as_ref()),
            )
            .map(|bytes| json!({ "ok": true, "bytes": bytes }))
        }
        METHOD_CACHE_CLEAR => {
            require_permission(permissions, &["fs.write", "file.write"])?;
            clear_dir(&PathBuf::from(&asset_paths.cache_dir))
                .map(|deleted| json!({ "ok": true, "deleted": deleted }))
        }
        METHOD_SECRETS_SET => {
            require_permission(permissions, &["secrets.write", "secrets", "secret"])?;
            write_text_file(
                &PathBuf::from(&asset_paths.secrets_dir),
                request_key(req.params.as_ref())?,
                request_content(req.params.as_ref())?,
                false,
            )
            .map(|bytes| json!({ "ok": true, "bytes": bytes }))
        }
        METHOD_SECRETS_GET => {
            require_permission(permissions, &["secrets.read", "secrets", "secret"])?;
            let key = request_key(req.params.as_ref())?;
            read_text_file(&PathBuf::from(&asset_paths.secrets_dir), key.clone())
                .map(|content| json!({ "key": key, "value": content }))
        }
        METHOD_SECRETS_DELETE => {
            require_permission(permissions, &["secrets.write", "secrets", "secret"])?;
            delete_path(
                &PathBuf::from(&asset_paths.secrets_dir),
                request_key(req.params.as_ref())?,
            )
            .map(|deleted| json!({ "ok": true, "deleted": deleted }))
        }
        _ => anyhow::bail!("方法未找到: {}", req.method),
    };

    emit_asset_event(plugin_id, req, &result, events_tx);
    result
}

fn emit_asset_event(
    plugin_id: &str,
    req: &JsonRpcRequest,
    result: &anyhow::Result<Value>,
    events_tx: &broadcast::Sender<PluginEvent>,
) {
    let (scope, action) = method_scope_action(&req.method);
    let path = req
        .params
        .as_ref()
        .and_then(|params| {
            params
                .get("path")
                .or_else(|| params.get("key"))
                .and_then(|value| value.as_str())
        })
        .unwrap_or("")
        .to_string();
    let _ = events_tx.send(PluginEvent::AssetOperation {
        plugin_id: plugin_id.to_string(),
        scope: scope.to_string(),
        action: action.to_string(),
        path,
        ok: result.is_ok(),
    });
}

fn method_scope_action(method: &str) -> (&'static str, &'static str) {
    match method {
        METHOD_ASSETS_READ => ("data", "read"),
        METHOD_ASSETS_WRITE => ("data", "write"),
        METHOD_ASSETS_LIST => ("data", "list"),
        METHOD_ASSETS_DELETE => ("data", "delete"),
        METHOD_CACHE_READ => ("cache", "read"),
        METHOD_CACHE_WRITE => ("cache", "write"),
        METHOD_CACHE_CLEAR => ("cache", "clear"),
        METHOD_SECRETS_SET => ("secrets", "set"),
        METHOD_SECRETS_GET => ("secrets", "get"),
        METHOD_SECRETS_DELETE => ("secrets", "delete"),
        _ => ("unknown", "unknown"),
    }
}

fn request_path(params: Option<&Value>) -> anyhow::Result<String> {
    let path = request_path_or_empty(params);
    if path.trim().is_empty() {
        anyhow::bail!("缺少 path");
    }
    Ok(path)
}

fn request_path_or_empty(params: Option<&Value>) -> String {
    params
        .and_then(|value| value.get("path"))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn request_key(params: Option<&Value>) -> anyhow::Result<String> {
    let key = params
        .and_then(|value| value.get("key").or_else(|| value.get("path")))
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    if key.trim().is_empty() {
        anyhow::bail!("缺少 key");
    }
    Ok(key)
}

fn request_content(params: Option<&Value>) -> anyhow::Result<String> {
    if let Some(content) = params
        .and_then(|value| value.get("content").or_else(|| value.get("value")))
        .and_then(|value| value.as_str())
    {
        return Ok(content.to_string());
    }
    anyhow::bail!("缺少 content")
}

fn request_append(params: Option<&Value>) -> bool {
    params
        .and_then(|value| value.get("append"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn require_permission(permissions: &[String], accepted: &[&str]) -> anyhow::Result<()> {
    if accepted
        .iter()
        .any(|required| has_permission(permissions, required))
    {
        return Ok(());
    }
    anyhow::bail!("缺少插件权限: {}", accepted[0])
}

fn has_permission(permissions: &[String], required: &str) -> bool {
    let required_root = required.split('.').next().unwrap_or(required);
    permissions.iter().any(|permission| {
        let permission = permission.trim().to_ascii_lowercase();
        if permission == "*" || permission == required {
            return true;
        }
        if permission == format!("{required_root}.*") || permission == required_root {
            return true;
        }
        permission.strip_suffix(".*").is_some_and(|prefix| {
            required
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
        })
    })
}

fn read_text_file(root: &Path, relative: String) -> anyhow::Result<String> {
    let path = resolve_plugin_path(root, &relative)?;
    ensure_no_symlink_under(root, &path)?;
    let bytes =
        std::fs::read(&path).with_context(|| format!("无法读取文件: {}", path.display()))?;
    String::from_utf8(bytes).with_context(|| format!("文件不是 UTF-8 文本: {}", path.display()))
}

fn write_text_file(
    root: &Path,
    relative: String,
    content: String,
    append: bool,
) -> anyhow::Result<usize> {
    let path = resolve_plugin_path(root, &relative)?;
    if let Some(parent) = path.parent() {
        ensure_no_symlink_under(root, parent)?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建目录: {}", parent.display()))?;
        ensure_no_symlink_under(root, parent)?;
    }
    if path.exists() {
        ensure_no_symlink_under(root, &path)?;
    }
    if append {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("无法打开文件: {}", path.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("无法写入文件: {}", path.display()))?;
    } else {
        std::fs::write(&path, content.as_bytes())
            .with_context(|| format!("无法写入文件: {}", path.display()))?;
    }
    Ok(content.len())
}

fn list_files(root: &Path, relative: String) -> anyhow::Result<Vec<Value>> {
    let dir = resolve_plugin_path(root, &relative)?;
    ensure_no_symlink_under(root, &dir)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    if !dir.is_dir() {
        anyhow::bail!("路径不是目录: {}", relative);
    }
    let mut items = Vec::new();
    for entry in
        std::fs::read_dir(&dir).with_context(|| format!("无法读取目录: {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        ensure_no_symlink_under(root, &path)?;
        let metadata = entry.metadata()?;
        let item_path = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        items.push(json!({
            "path": item_path,
            "kind": if metadata.is_dir() { "dir" } else { "file" },
            "bytes": if metadata.is_file() { metadata.len() } else { 0 },
        }));
    }
    items.sort_by(|a, b| {
        a.get("path")
            .and_then(|value| value.as_str())
            .cmp(&b.get("path").and_then(|value| value.as_str()))
    });
    Ok(items)
}

fn delete_path(root: &Path, relative: String) -> anyhow::Result<bool> {
    let path = resolve_plugin_path(root, &relative)?;
    if !path.exists() {
        return Ok(false);
    }
    ensure_no_symlink_under(root, &path)?;
    if path.is_dir() {
        std::fs::remove_dir_all(&path)
            .with_context(|| format!("无法删除目录: {}", path.display()))?;
    } else {
        std::fs::remove_file(&path).with_context(|| format!("无法删除文件: {}", path.display()))?;
    }
    Ok(true)
}

fn clear_dir(root: &Path) -> anyhow::Result<usize> {
    std::fs::create_dir_all(root).with_context(|| format!("无法创建目录: {}", root.display()))?;
    let mut deleted = 0_usize;
    for entry in
        std::fs::read_dir(root).with_context(|| format!("无法读取目录: {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        ensure_no_symlink_under(root, &path)?;
        if path.is_dir() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("无法删除目录: {}", path.display()))?;
        } else {
            std::fs::remove_file(&path)
                .with_context(|| format!("无法删除文件: {}", path.display()))?;
        }
        deleted += 1;
    }
    Ok(deleted)
}

fn resolve_plugin_path(root: &Path, relative: &str) -> anyhow::Result<PathBuf> {
    if relative.trim().is_empty() {
        return Ok(root.to_path_buf());
    }
    let relative_path = Path::new(relative);
    if relative_path.is_absolute() {
        anyhow::bail!("路径必须是相对路径");
    }
    let mut clean = PathBuf::new();
    for component in relative_path.components() {
        match component {
            Component::Normal(part) if valid_path_component(part) => clean.push(part),
            Component::CurDir => {}
            _ => anyhow::bail!("路径不允许包含上级目录、根路径或特殊组件"),
        }
    }
    if clean.as_os_str().is_empty() {
        return Ok(root.to_path_buf());
    }
    Ok(root.join(clean))
}

fn valid_path_component(part: &OsStr) -> bool {
    let text = part.to_string_lossy();
    !text.is_empty() && !text.contains('\0')
}

fn ensure_no_symlink_under(root: &Path, path: &Path) -> anyhow::Result<()> {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    let mut cursor = root.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("路径不允许包含符号链接: {}", cursor.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
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
