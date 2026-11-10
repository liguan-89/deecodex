//! DEXAI Debug 支持端远程调用：通过 Tailnet 直连目标 DEX AI 的 7779 通道，
//! 短码授权后拉取/执行诊断。客户侧开关与短码由 dexai-debug 插件提供。
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tauri::State;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;

use crate::ServerManager;

use super::plugins::invoke_plugin_method;
use crate::commands::dex_account;

const DEFAULT_DEBUG_PORT: u16 = 17443;
/// 本机 dexai-debug netd 的 tailnet 出站代理（支持端经它 CONNECT 到目标节点）
const DEBUG_OUTBOUND_PROXY: &str = "127.0.0.1:17445";
const RPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

async fn connect_debug_outbound_proxy() -> Result<TcpStream, String> {
    let connect_fut = TcpStream::connect(DEBUG_OUTBOUND_PROXY);
    tokio::time::timeout(std::time::Duration::from_secs(5), connect_fut)
        .await
        .map_err(|_| {
            "连接本机 tailnet 出站代理超时（请确认 DEXAI Debug 插件已启用）".to_string()
        })?
        .map_err(|e| format!("连接本机 tailnet 出站代理失败: {e}"))
}

/// 系统剪贴板写入（WebView clipboard API 在 Tauri 下不可靠，走 Rust 端）
#[tauri::command]
pub fn clipboard_write(text: String) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用: {e}"))?;
    cb.set_text(text).map_err(|e| format!("写入剪贴板失败: {e}"))
}

/// 系统剪贴板读取
#[tauri::command]
pub fn clipboard_read() -> Result<String, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用: {e}"))?;
    cb.get_text().map_err(|e| format!("读取剪贴板失败: {e}"))
}

/// 自动向 control-plane 领取 headscale 一次性 auth key 并写入诊断插件数据目录。
/// 用 dexai-desktop 主账号 OAuth（Bearer）授权，调用 control-plane /v1/debug/network-credentials。
///
/// 与 Tauri command 共用的核心逻辑：GUI 启动时（插件自动启用路径）与手动"启用"都会调用，
/// 保证 netd 启动前 authkey 文件里是未过期的新 key（headscale 一次性 key 有效期仅 5 分钟）。
pub(crate) async fn refresh_debug_authkey_inner(
    manager: &ServerManager,
) -> Result<Value, String> {
    let session = dex_account::ensure_master_session(manager).await?;
    let base = dex_account::control_plane_url();
    let client = dex_account::http_client();
    let resp = client
        .post(format!("{base}/v1/debug/network-credentials"))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|e| format!("请求签发失败: {e}"))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))?;
    if !status.is_success() {
        return Err(format!("签发失败 HTTP {status}: {body}"));
    }
    let auth_key = body
        .get("authKey")
        .and_then(Value::as_str)
        .ok_or_else(|| "响应缺少 authKey".to_string())?
        .to_string();
    let login_server = body
        .get("loginServer")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 写入诊断插件数据目录 authkey（netd 启动时自动读取）
    let data_dir = manager.data_dir.lock().await.clone();
    let auth_dir = data_dir
        .join("plugins")
        .join("dexai-debug")
        .join(".diagnose-data");
    std::fs::create_dir_all(&auth_dir).map_err(|e| format!("创建插件数据目录失败: {e}"))?;
    std::fs::write(auth_dir.join("authkey"), &auth_key)
        .map_err(|e| format!("写入 authkey 失败: {e}"))?;
    Ok(json!({
        "loginServer": login_server,
        "authKey": auth_key,
        "written": true,
    }))
}

/// Tauri 命令入口：手动启用诊断服务前领取网络凭证。
#[tauri::command]
pub async fn debug_obtain_network_credentials(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    refresh_debug_authkey_inner(&manager).await
}

/// GUI 启动时刷新 dexai-debug 的 authkey（供插件自动启用路径使用）。
/// 仅在插件配置里诊断服务已开启（config.enabled == true）时执行；
/// 未登录主账号或刷新失败只告警，不阻塞启动（插件仍可走持久化身份恢复）。
pub(crate) async fn refresh_debug_authkey_on_startup(manager: &ServerManager) {
    let data_dir = manager.data_dir.lock().await.clone();
    let Some(pm) = manager.plugin_manager.lock().await.clone() else {
        return;
    };
    let debug_enabled = pm
        .list()
        .await
        .iter()
        .find(|p| p.id == "dexai-debug")
        .map(|p| {
            p.config
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if !debug_enabled {
        return;
    }
    // 插件已开启诊断服务：确保 authkey 文件里有新鲜 key，避免 netd 启动时撞上 5 分钟过期。
    match refresh_debug_authkey_inner(manager).await {
        Ok(_) => {
            tracing::info!(data_dir = %data_dir.display(), "DEXAI Debug 已开启，启动时刷新网络凭证");
        }
        Err(e) => {
            tracing::warn!(error = %e, "DEXAI Debug 启动时刷新网络凭证失败（可能未登录主账号，插件将尝试持久身份恢复）");
        }
    }
}

/// 调用本机 dexai-debug 插件的宿主方法（客户侧：开关/短码/状态/审计）。
#[tauri::command]
pub async fn debug_local_call(
    manager: State<'_, ServerManager>,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    invoke_plugin_method(manager, "dexai-debug".to_string(), method, params).await
}

/// 支持端远程调用：连目标 DEX AI 的 7779 通道。
/// - 无 token 时用 code 打开会话（session.open），自动回填 token
/// - 有 token 时直接调用 method（token 放在 params.token）
/// 返回 { session_token, result }。
#[tauri::command]
pub async fn debug_remote_call(
    manager: State<'_, ServerManager>,
    host: String,
    port: Option<u16>,
    code: Option<String>,
    token: Option<String>,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let port = port.unwrap_or(DEFAULT_DEBUG_PORT);
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("缺少目标地址".into());
    }

    let mut session_token = token.clone().unwrap_or_default();
    let needs_open = session_token.is_empty();
    if needs_open && code.as_ref().map_or(true, |c| c.trim().is_empty()) {
        return Err("需要短码".to_string());
    }

    // 支持端不直连 tailnet IP（本机可能没有 tailnet 路由），而是经本机 dexai-debug
    // netd 的 outbound-proxy（127.0.0.1:17445）CONNECT 到目标节点：netd 通过 tsnet
    // userspace 转发，目标节点 netd 再把 tailnet 入站连接转发到其 127.0.0.1:17443。
    // 先确保本机 netd 已启动（仅提供出站代理，不开启被诊断服务）。
    let _ = invoke_plugin_method(
        manager.clone(),
        "dexai-debug".to_string(),
        "debug.ensure_netd".to_string(),
        None,
    )
    .await;
    let target = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let mut stream = match connect_debug_outbound_proxy().await {
        Ok(stream) => stream,
        Err(first_err) => {
            // netd 可能因一次性 auth key 过期/被消费而没起来：刷新网络凭证后
            // 强制重新注册 netd，再重试一次连接。
            let refreshed = debug_obtain_network_credentials(manager.clone()).await.is_ok();
            let _ = invoke_plugin_method(
                manager.clone(),
                "dexai-debug".to_string(),
                "debug.ensure_netd".to_string(),
                Some(json!({ "force": true })),
            )
            .await;
            connect_debug_outbound_proxy().await.map_err(|second_err| {
                if refreshed {
                    second_err
                } else {
                    format!("{first_err}；且刷新网络凭证失败（请先在个人中心登录 DEX AI 主账号）")
                }
            })?
        }
    };
    let (read_half, mut write_half) = stream.split();
    let mut lines = BufReader::new(read_half).lines();
    // 发送 CONNECT 后立即进入 JSON-RPC；若目标不可达，netd 会关闭连接，
    // 首个请求将读到 EOF（连接被关闭）。
    write_half
        .write_all(format!("CONNECT {target}\n").as_bytes())
        .await
        .map_err(|e| format!("发送 CONNECT {target} 失败: {e}"))?;
    write_half.flush().await.map_err(|e| format!("flush 失败: {e}"))?;

    let mut last_result = Value::Null;
    // 阶段 1：无 token 时先打开会话，拿到 token 后再调用目标方法
    // （token 必须在收到 session.open 响应后才能获得，不能预先拼进请求）
    if needs_open {
        let code = code.unwrap_or_default();
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session.open",
            "params": { "code": code.trim(), "source": "gui" },
        });
        let line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        write_half
            .write_all(format!("{line}\n").as_bytes())
            .await
            .map_err(|e| format!("发送失败: {e}"))?;
        write_half.flush().await.map_err(|e| format!("flush 失败: {e}"))?;

        let resp_line = tokio::time::timeout(RPC_TIMEOUT, lines.next_line())
            .await
            .map_err(|_| "响应超时".to_string())?
            .map_err(|e| format!("读取失败: {e}"))?
            .ok_or_else(|| "连接被关闭".to_string())?;
        let resp: Value =
            serde_json::from_str(&resp_line).map_err(|e| format!("解析响应失败: {e}"))?;
        if let Some(err) = resp.get("error") {
            return Err(format!("远程错误: {err}"));
        }
        session_token = resp["result"]["token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_token.is_empty() {
            return Err("远程未返回会话 token".to_string());
        }
        last_result = resp.get("result").cloned().unwrap_or(Value::Null);
    }

    // 阶段 2：携带 token 调用目标方法
    let mut call_params = params.unwrap_or_else(|| json!({}));
    if let Value::Object(ref mut map) = call_params {
        map.insert("token".into(), json!(session_token));
    }
    let req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": method,
        "params": call_params,
    });
    let line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
    write_half
        .write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(|e| format!("发送失败: {e}"))?;
    write_half.flush().await.map_err(|e| format!("flush 失败: {e}"))?;

    let resp_line = tokio::time::timeout(RPC_TIMEOUT, lines.next_line())
        .await
        .map_err(|_| "响应超时".to_string())?
        .map_err(|e| format!("读取失败: {e}"))?
        .ok_or_else(|| "连接被关闭".to_string())?;
    let resp: Value =
        serde_json::from_str(&resp_line).map_err(|e| format!("解析响应失败: {e}"))?;
    if let Some(err) = resp.get("error") {
        return Err(format!("远程错误: {err}"));
    }
    last_result = resp.get("result").cloned().unwrap_or(Value::Null);

    Ok(json!({
        "session_token": session_token,
        "result": last_result,
    }))
}

/// 建立到目标的 SSH/TCP 隧道：本地监听 `local_port`，所有字节经 tailnet 17443
/// 通道透传到目标本地 `port`（应用层转发，不依赖 netd 开放额外端口）。
/// 返回本地端口，用户用 `ssh -p <port> user@127.0.0.1` / `scp -P <port>` 连接。
#[tauri::command]
pub async fn debug_remote_tunnel(
    manager: State<'_, ServerManager>,
    host: String,
    port: Option<u16>,
    code: Option<String>,
    token: Option<String>,
    local_port: Option<u16>,
) -> Result<Value, String> {
    let host = host.trim().to_string();
    let target_port = port.unwrap_or(22);
    let code = code.filter(|c| !c.trim().is_empty());
    let token = token.filter(|t| !t.trim().is_empty());
    if host.is_empty() {
        return Err("缺少目标地址".into());
    }
    if code.is_none() && token.is_none() {
        return Err("需要短码".to_string());
    }

    // 确保本机 netd 已启动（提供 tailnet 出站代理）
    let _ = invoke_plugin_method(
        manager.clone(),
        "dexai-debug".to_string(),
        "debug.ensure_netd".to_string(),
        None,
    )
    .await;

    // 本地监听（0 = 自动分配端口）
    let bind_addr = format!("127.0.0.1:{}", local_port.unwrap_or(0));
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|e| format!("监听本地端口失败: {e}"))?;
    let actual_port = listener
        .local_addr()
        .map_err(|e| format!("获取本地端口失败: {e}"))?
        .port();

    let host_task = host.clone();
    let code_task = code.clone();
    let token_task = token.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else { break };
            let host = host_task.clone();
            let code = code_task.clone();
            let token = token_task.clone();
            tokio::spawn(async move {
                if let Err(e) = tunnel_connection(host, target_port, code, token, client).await {
                    tracing::debug!(error = %e, "tunnel connection closed with error");
                }
            });
        }
    });

    if let Some(prev) = manager
        .debug_tunnels
        .lock()
        .unwrap()
        .insert(actual_port, handle)
    {
        prev.abort();
    }

    Ok(json!({
        "ok": true,
        "local_port": actual_port,
        "target": format!("{host}:{target_port}"),
        "command": format!("ssh -p {actual_port} user@127.0.0.1"),
    }))
}

/// 关闭本地指定端口的隧道。
#[tauri::command]
pub fn debug_remote_tunnel_close(
    manager: State<'_, ServerManager>,
    local_port: u16,
) -> Result<(), String> {
    if let Some(h) = manager
        .debug_tunnels
        .lock()
        .unwrap()
        .remove(&local_port)
    {
        h.abort();
        Ok(())
    } else {
        Err(format!("端口 {local_port} 没有活跃隧道"))
    }
}

/// 从 JSON-RPC 连接读取一行（带超时，EOF 视为关闭）。
async fn read_json_line<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<String, String> {
    let mut line = String::new();
    let n = tokio::time::timeout(RPC_TIMEOUT, reader.read_line(&mut line))
        .await
        .map_err(|_| "响应超时".to_string())?
        .map_err(|e| format!("读取失败: {e}"))?;
    if n == 0 {
        return Err("连接被关闭".to_string());
    }
    Ok(line.trim().to_string())
}

/// 建立到目标 17443 通道的连接并完成授权 + 透传方法调用（tunnel.open / shell.open）。
/// 返回已进入透传模式的 upstream，以及方法确认后可能已缓冲的字节。
async fn establish_channel(
    host: String,
    code: Option<String>,
    token: Option<String>,
    method: &str,
    params: Value,
) -> Result<(TcpStream, Vec<u8>), String> {
    let mut upstream = connect_debug_outbound_proxy().await?;
    let target = if host.contains(':') {
        format!("[{host}]:{DEFAULT_DEBUG_PORT}")
    } else {
        format!("{host}:{DEFAULT_DEBUG_PORT}")
    };
    upstream
        .write_all(format!("CONNECT {target}\n").as_bytes())
        .await
        .map_err(|e| format!("CONNECT 失败: {e}"))?;
    upstream.flush().await.map_err(|e| format!("flush 失败: {e}"))?;

    let mut reader = BufReader::new(upstream);
    let mut session_token = token.unwrap_or_default();
    if session_token.is_empty() {
        let code = code.ok_or_else(|| "需要短码".to_string())?;
        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session.open",
            "params": { "code": code, "source": "gui-chan" },
        });
        reader
            .get_mut()
            .write_all(format!("{req}\n").as_bytes())
            .await
            .map_err(|e| format!("发送 session.open 失败: {e}"))?;
        reader
            .get_mut()
            .flush()
            .await
            .map_err(|e| format!("flush 失败: {e}"))?;
        let line = read_json_line(&mut reader).await?;
        let resp: Value =
            serde_json::from_str(&line).map_err(|e| format!("解析响应失败: {e}"))?;
        if let Some(err) = resp.get("error") {
            return Err(format!("远程错误: {err}"));
        }
        session_token = resp["result"]["token"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if session_token.is_empty() {
            return Err("远程未返回会话 token".to_string());
        }
    }

    let mut call_params = params;
    if let Value::Object(ref mut map) = call_params {
        map.insert("token".into(), json!(session_token));
    }
    let req = json!({ "jsonrpc": "2.0", "id": 2, "method": method, "params": call_params });
    reader
        .get_mut()
        .write_all(format!("{req}\n").as_bytes())
        .await
        .map_err(|e| format!("发送 {method} 失败: {e}"))?;
    reader
        .get_mut()
        .flush()
        .await
        .map_err(|e| format!("flush 失败: {e}"))?;
    let line = read_json_line(&mut reader).await?;
    let resp: Value =
        serde_json::from_str(&line).map_err(|e| format!("解析响应失败: {e}"))?;
    if let Some(err) = resp.get("error") {
        return Err(format!("建立失败: {err}"));
    }

    let buffered = reader.buffer().to_vec();
    let upstream = reader.into_inner();
    Ok((upstream, buffered))
}

/// 单个隧道连接：经 tailnet 17443 通道 + 插件 tunnel.open 透传到目标本地端口。
async fn tunnel_connection(
    host: String,
    target_port: u16,
    code: Option<String>,
    token: Option<String>,
    client: TcpStream,
) -> Result<(), String> {
    let (mut upstream, buffered) =
        establish_channel(host, code, token, "tunnel.open", json!({ "port": target_port })).await?;
    let (mut rh, mut wh) = tokio::io::split(upstream);
    let (mut crh, mut cwh) = tokio::io::split(client);
    if !buffered.is_empty() {
        cwh.write_all(&buffered).await.map_err(|e| format!("转发缓冲失败: {e}"))?;
    }
    let t1 = tokio::spawn(async move { tokio::io::copy(&mut crh, &mut wh).await });
    let t2 = tokio::spawn(async move { tokio::io::copy(&mut rh, &mut cwh).await });
    let _ = tokio::try_join!(t1, t2);
    Ok(())
}

/// 内置交互式终端会话：远端 shell 输出写入 buffer（前端轮询拉取），输入写回 write_half。
pub struct TerminalSession {
    pub write_half: Arc<tokio::sync::Mutex<tokio::io::WriteHalf<TcpStream>>>,
    pub buffer: Arc<std::sync::Mutex<VecDeque<u8>>>,
    pub handle: tokio::task::JoinHandle<()>,
}

static TERMINAL_SEQ: AtomicU32 = AtomicU32::new(1);

/// 打开内置交互式终端（不依赖目标机 sshd）：tailnet 通道 + 插件 shell.open。
#[tauri::command]
pub async fn debug_terminal_open(
    manager: State<'_, ServerManager>,
    host: String,
    code: Option<String>,
    token: Option<String>,
) -> Result<Value, String> {
    let host = host.trim().to_string();
    if host.is_empty() {
        return Err("缺少目标地址".into());
    }
    let code = code.filter(|c| !c.trim().is_empty());
    let token = token.filter(|t| !t.trim().is_empty());
    if code.is_none() && token.is_none() {
        return Err("需要短码".to_string());
    }
    let _ = invoke_plugin_method(
        manager.clone(),
        "dexai-debug".to_string(),
        "debug.ensure_netd".to_string(),
        None,
    )
    .await;

    let (upstream, buffered) = establish_channel(host, code, token, "shell.open", json!({})).await?;
    let (read_half, write_half) = tokio::io::split(upstream);
    let write_arc = Arc::new(tokio::sync::Mutex::new(write_half));
    let buffer_arc: Arc<std::sync::Mutex<VecDeque<u8>>> =
        Arc::new(std::sync::Mutex::new(VecDeque::new()));
    {
        let mut b = buffer_arc.lock().unwrap();
        b.extend(buffered);
    }
    let buf_task = buffer_arc.clone();
    let handle = tokio::spawn(async move {
        let mut rh = read_half;
        let mut chunk = vec![0u8; 8192];
        loop {
            match rh.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf_task.lock().unwrap().extend(&chunk[..n]);
                }
            }
        }
    });

    let session_id = TERMINAL_SEQ.fetch_add(1, Ordering::Relaxed);
    let session = TerminalSession {
        write_half: write_arc,
        buffer: buffer_arc,
        handle,
    };
    if let Some(prev) = manager.debug_terminals.lock().unwrap().insert(session_id, session) {
        prev.handle.abort();
    }
    Ok(json!({ "ok": true, "session_id": session_id }))
}

/// 拉取终端新增输出（轮询用，返回后清空缓冲）。
/// 用 base64 传原始字节：远端 shell 输出是任意二进制/UTF-8 流，
/// 直接转 String 会在轮询切块处破坏多字节字符（乱码）。
#[tauri::command]
pub async fn debug_terminal_poll(
    manager: State<'_, ServerManager>,
    session_id: u32,
) -> Result<Value, String> {
    let guard = manager.debug_terminals.lock().unwrap();
    let session = guard.get(&session_id).ok_or("终端会话不存在")?;
    let data: Vec<u8> = session.buffer.lock().unwrap().drain(..).collect();
    Ok(json!({ "data": B64.encode(&data) }))
}

/// 向终端写入输入字节。
#[tauri::command]
pub async fn debug_terminal_write(
    manager: State<'_, ServerManager>,
    session_id: u32,
    data: String,
) -> Result<(), String> {
    let write_arc = {
        let guard = manager.debug_terminals.lock().unwrap();
        guard
            .get(&session_id)
            .map(|s| s.write_half.clone())
            .ok_or("终端会话不存在")?
    };
    let mut wh = write_arc.lock().await;
    wh.write_all(data.as_bytes())
        .await
        .map_err(|e| format!("写入失败: {e}"))?;
    wh.flush().await.map_err(|e| format!("flush 失败: {e}"))
}

/// 关闭终端会话。
#[tauri::command]
pub fn debug_terminal_close(
    manager: State<'_, ServerManager>,
    session_id: u32,
) -> Result<(), String> {
    if let Some(s) = manager.debug_terminals.lock().unwrap().remove(&session_id) {
        s.handle.abort();
        Ok(())
    } else {
        Err("终端会话不存在".to_string())
    }
}
