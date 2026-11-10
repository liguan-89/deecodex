//! opencode 引擎生命周期 + Tauri 命令（DEX 助手 v1）
//!
//! - 子进程管理：解析 pin 版本二进制、`opencode serve` 启动/健康检查/停止/崩溃重启；
//! - MCP 数据源：启动本进程内的 MCP Streamable HTTP server（见 `dex_mcp`）；
//! - 事件转发：把 opencode `/event` SSE 归一化后通过 Tauri 事件推给前端；
//! - 会话代理：把前端需要的会话 API 通过 Tauri 命令暴露（前端不直连 opencode HTTP）。

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use base64::Engine as _;
use futures_util::StreamExt;
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};
use tokio::process::Command;
use tracing::{info, warn};

use crate::ServerManager;

use super::dex::all_tool_defs;
use super::dex_mcp::{mcp_router, McpShared};
use super::dex_opencode_config::{generate_opencode_config, OPENCODE_PIN_VERSION};
use super::dex_protocol::get_active_account_info;
use super::dex_registry::{is_capability_enabled, load_capability_states};

/// Windows：隐藏 opencode 子进程控制台窗口（CREATE_NO_WINDOW），
/// 否则 `opencode serve` 长驻进程会一直挂着黑框终端。
#[cfg(target_os = "windows")]
fn hide_child_window(command: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}
#[cfg(not(target_os = "windows"))]
fn hide_child_window(_command: &mut tokio::process::Command) {}

const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(30);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(200);
const EVENT_RECONNECT_INTERVAL: Duration = Duration::from_secs(2);
const EVENT_NAME: &str = "dex-opencode-event";

/// opencode 1.18.16 `/doc`（OpenAPI）契约基线 SHA256（本机与 Mac mini 双机实测一致）。
pub const OPENCODE_DOC_SHA256: &str =
    "c3a9f94af0c3324d97b482b14c692e810ce7ccac3136319ba46334de972b4cf1";

/// opencode 1.18.16 二进制 SHA256（macOS arm64 / Windows x64，来自官方 release 实测）。
#[cfg(target_os = "macos")]
const OPENCODE_BINARY_SHA256: &str =
    "a41776bf64c75786d6baf531b840ffb873c090d7c44793ae2dd4b1896de56a1f";
#[cfg(target_os = "windows")]
const OPENCODE_BINARY_SHA256: &str =
    "dadee463adc9eaeeab9b79d5c5b4557a372a33af70b2742fff76d5507fccc0ac";

/// opencode 运行时句柄（存在 ServerManager.opencode）
pub struct OpencodeRuntime {
    pub url: String,
    pub port: u16,
    pub mcp_url: String,
    pub mcp_port: u16,
    pub mcp_token: String,
    pub config_dir: PathBuf,
    pub running: AtomicBool,
    http: reqwest::Client,
    child: tokio::sync::Mutex<Option<tokio::process::Child>>,
    mcp_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    event_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl OpencodeRuntime {
    async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.url, path);
        let req = match method {
            "GET" => self.http.get(&url),
            "POST" => self
                .http
                .post(&url)
                .json(&body.unwrap_or_else(|| json!({}))),
            "PATCH" => self
                .http
                .patch(&url)
                .json(&body.unwrap_or_else(|| json!({}))),
            _ => return Err(format!("unsupported method {method}")),
        };
        let resp = req
            .send()
            .await
            .map_err(|e| format!("opencode 请求失败: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "opencode {method} {path} -> {status}: {}",
                text.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&text).map_err(|_| text)
    }

    async fn session_create(&self, title: &str) -> Result<Value, String> {
        self.request("POST", "/session", Some(json!({ "title": title })))
            .await
    }

    async fn prompt(&self, session_id: &str, text: &str) -> Result<Value, String> {
        self.request(
            "POST",
            &format!("/session/{session_id}/prompt_async"),
            Some(json!({ "parts": [{ "type": "text", "text": text }] })),
        )
        .await
    }

    async fn abort(&self, session_id: &str) -> Result<Value, String> {
        self.request("POST", &format!("/session/{session_id}/abort"), None)
            .await
    }

    async fn permission_reply(
        &self,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<Value, String> {
        self.request(
            "POST",
            &format!("/session/{session_id}/permissions/{permission_id}"),
            Some(json!({ "response": response })),
        )
        .await
    }
}

/// 二进制解析：环境变量 > 捆绑资源目录 > 本地开发路径
fn resolve_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("DEECODEX_OPENCODE_BIN") {
        if !path.trim().is_empty() {
            let pb = PathBuf::from(path.trim());
            if pb.exists() {
                return Some(pb);
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            #[cfg(target_os = "macos")]
            let candidates = [
                dir.join("opencode"),
                dir.join("bin").join("opencode"),
                // Tauri macOS 捆绑资源目录
                dir.join("../Resources/opencode"),
                // Tauri resources glob 保留相对路径：resources/opencode/*
                dir.join("../Resources/resources/opencode/opencode-darwin-arm64"),
            ];
            #[cfg(target_os = "windows")]
            let candidates = [
                dir.join("opencode.exe"),
                dir.join("bin").join("opencode.exe"),
                // Tauri Windows 资源目录
                dir.join("opencode"),
                // Tauri resources glob 保留相对路径：resources/opencode/*
                dir.join("resources/opencode/opencode-windows-x64.exe"),
            ];
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let candidates: [PathBuf; 0] = [];
            for candidate in candidates {
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    deecodex::config::home_dir()
        .map(|home| home.join(format!(".local/opencode/{OPENCODE_PIN_VERSION}/opencode")))
        .filter(|p| p.exists())
}

async fn verify_version(binary: &Path) -> Result<(), String> {
    let mut command = tokio::process::Command::new(binary);
    hide_child_window(&mut command);
    let output = command
        .arg("--version")
        .output()
        .await
        .map_err(|e| format!("执行 opencode --version 失败: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim().trim_start_matches('v');
    if version == OPENCODE_PIN_VERSION {
        Ok(())
    } else {
        Err(format!(
            "opencode 版本不匹配：需要 {}，实际 {}（{}）。请用 DEECODEX_OPENCODE_BIN 指定正确二进制",
            OPENCODE_PIN_VERSION,
            version,
            binary.display()
        ))
    }
}

/// 校验二进制 SHA256（pin 版本防篡改/防漂移）。
fn verify_binary_sha256(binary: &Path) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(binary).map_err(|e| format!("读取 opencode 二进制失败: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual == OPENCODE_BINARY_SHA256 {
        Ok(())
    } else {
        Err(format!(
            "opencode 二进制 SHA256 校验失败（{}）：期望 {}，实际 {}。请通过 scripts/fetch-opencode.sh 获取官方二进制",
            binary.display(),
            OPENCODE_BINARY_SHA256,
            actual
        ))
    }
}

fn pick_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("分配端口失败: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取端口失败: {e}"))?
        .port();
    drop(listener);
    Ok(port)
}

fn basic_auth_header(username: &str, password: &str) -> Result<HeaderValue, String> {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
    HeaderValue::from_str(&format!("Basic {token}")).map_err(|e| format!("构造认证头失败: {e}"))
}

fn write_secret_file(path: &Path, content: &str) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
        .map_err(|e| format!("创建配置目录失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, content).map_err(|e| format!("写入配置失败: {e}"))?;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, content).map_err(|e| format!("写入配置失败: {e}"))?;
    }
    Ok(())
}

/// 启动 opencode serve + MCP server，返回运行时句柄。
pub async fn opencode_start(manager: &ServerManager) -> Result<Arc<OpencodeRuntime>, String> {
    let app = manager
        .app_handle
        .lock()
        .await
        .clone()
        .ok_or_else(|| "应用句柄未就绪".to_string())?;

    let binary = resolve_binary().ok_or_else(|| {
        format!(
            "未找到 opencode {OPENCODE_PIN_VERSION} 二进制。请设置 DEECODEX_OPENCODE_BIN 或安装到 ~/.local/opencode/{OPENCODE_PIN_VERSION}/"
        )
    })?;
    verify_version(&binary).await?;
    verify_binary_sha256(&binary)?;

    let data_dir = manager.data_dir.lock().await.clone();
    let base_dir = data_dir.join("opencode");
    let config_dir = base_dir.join("config");
    let data_home = base_dir.join("data");
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("创建 opencode 目录失败: {e}"))?;
    std::fs::create_dir_all(&data_home).map_err(|e| format!("创建 opencode 数据目录失败: {e}"))?;

    // 账号信息（活跃账号直连上游）
    let (upstream, api_key, model_map, known_models, provider, ..) =
        get_active_account_info(&data_dir)
            .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    // 只取启用能力包的工具
    let states = load_capability_states(&data_dir);
    let tool_defs: Vec<_> = all_tool_defs(manager)
        .await
        .into_iter()
        .filter(|t| is_capability_enabled(&states, &t.capability))
        .collect();

    let mcp_port = pick_port()?;
    let mcp_token = format!(
        "dee_{}_{}",
        std::process::id(),
        deecodex::accounts::generate_id()
    );
    let mcp_base_url = format!("http://127.0.0.1:{mcp_port}");

    let cfg = generate_opencode_config(
        &upstream,
        &api_key,
        &model_map,
        &known_models,
        &provider,
        &mcp_base_url,
        &mcp_token,
        &tool_defs,
    );
    write_secret_file(
        &config_dir.join("opencode.json"),
        &serde_json::to_string_pretty(&cfg).map_err(|e| format!("序列化配置失败: {e}"))?,
    )?;

    // MCP server
    let mcp_shared = McpShared::new(mcp_token.clone(), app.clone());
    let mcp_listener = tokio::net::TcpListener::bind(("127.0.0.1", mcp_port))
        .await
        .map_err(|e| format!("绑定 MCP 端口失败: {e}"))?;
    let mcp_app = mcp_router(mcp_shared);
    let mcp_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(mcp_listener, mcp_app).await {
            warn!(%e, "MCP server 退出");
        }
    });

    // opencode serve 子进程
    let port = pick_port()?;
    let username = "opencode";
    let password = deecodex::accounts::generate_id();
    let url = format!("http://127.0.0.1:{port}");
    let mut command = Command::new(&binary);
    hide_child_window(&mut command);
    let mut child = command
        .current_dir(&data_dir)
        .arg("serve")
        .arg("--hostname")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--cors")
        .arg("*")
        .env("OPENCODE_SERVER_USERNAME", username)
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .env("OPENCODE_CONFIG_DIR", &config_dir)
        .env("XDG_DATA_HOME", &data_home)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 opencode serve 失败: {e}"))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Some(mut out) = stdout {
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(&mut out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    info!(stream = "opencode.stdout", %line);
                }
            }
        });
    }
    if let Some(mut err) = stderr {
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = tokio::io::BufReader::new(&mut err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    warn!(stream = "opencode.stderr", %line);
                }
            }
        });
    }

    let auth_header = basic_auth_header(username, &password)?;
    let http = reqwest::Client::builder()
        .default_headers([(AUTHORIZATION, auth_header)].into_iter().collect())
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;

    // 等待健康
    let deadline = tokio::time::Instant::now() + SERVER_READY_TIMEOUT;
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("opencode serve 在 30s 内未就绪".to_string());
        }
        match http.get(format!("{url}/api/health")).send().await {
            Ok(resp) if resp.status().is_success() => break,
            _ => tokio::time::sleep(HEALTH_POLL_INTERVAL).await,
        }
    }

    let w_binary = binary.clone();
    let w_data_dir = data_dir.clone();
    let w_config_dir = config_dir.clone();
    let w_data_home = data_home.clone();
    let w_password = password.clone();
    let w_port = port;
    let runtime = Arc::new(OpencodeRuntime {
        url,
        port,
        mcp_url: format!("{mcp_base_url}/mcp"),
        mcp_port,
        mcp_token,
        config_dir,
        running: AtomicBool::new(true),
        http,
        child: tokio::sync::Mutex::new(Some(child)),
        mcp_task: tokio::sync::Mutex::new(Some(mcp_task)),
        event_task: tokio::sync::Mutex::new(None),
    });

    // 崩溃监控 + 事件转发
    let watcher = runtime.clone();
    let watcher_app = app.clone();
    tokio::spawn(async move {
        let mut restarts = 0u32;
        loop {
            let exited = {
                let mut guard = watcher.child.lock().await;
                match guard.as_mut() {
                    Some(child) => child.try_wait().ok().flatten().is_some(),
                    None => true,
                }
            };
            if exited {
                watcher.running.store(false, Ordering::Relaxed);
                if restarts < 5 {
                    restarts += 1;
                    warn!(restarts, "opencode serve 异常退出，5s 后重启");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if let Some(new_child) = spawn_opencode_serve(
                        &w_binary,
                        &w_data_dir,
                        w_port,
                        username,
                        &w_password,
                        &w_config_dir,
                        &w_data_home,
                    )
                    .await
                    {
                        *watcher.child.lock().await = Some(new_child);
                        watcher.running.store(true, Ordering::Relaxed);
                        continue;
                    }
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
        let _ = watcher_app;
    });
    spawn_event_forwarder(&runtime, &app).await;

    info!(url = %runtime.url, mcp = %runtime.mcp_url, "opencode 引擎已就绪");
    Ok(runtime)
}

async fn spawn_opencode_serve(
    binary: &Path,
    data_dir: &Path,
    port: u16,
    username: &str,
    password: &str,
    config_dir: &Path,
    data_home: &Path,
) -> Option<tokio::process::Child> {
    let mut command = Command::new(binary);
    hide_child_window(&mut command);
    let mut child = command
        .current_dir(data_dir)
        .arg("serve")
        .arg("--hostname")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--cors")
        .arg("*")
        .env("OPENCODE_SERVER_USERNAME", username)
        .env("OPENCODE_SERVER_PASSWORD", password)
        .env("OPENCODE_CONFIG_DIR", config_dir)
        .env("XDG_DATA_HOME", data_home)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // 等待健康
    let url = format!("http://127.0.0.1:{port}");
    let auth = basic_auth_header(username, password).ok()?;
    let client = reqwest::Client::builder()
        .default_headers([(AUTHORIZATION, auth)].into_iter().collect())
        .build()
        .ok()?;
    for _ in 0..150 {
        if client
            .get(format!("{url}/api/health"))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return Some(child);
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
    let _ = child.kill().await;
    None
}

async fn spawn_event_forwarder(runtime: &Arc<OpencodeRuntime>, app: &AppHandle) {
    let mut guard = runtime.event_task.lock().await;
    if guard.is_some() {
        return;
    }
    let url = runtime.url.clone();
    let http = runtime.http.clone();
    let app = app.clone();
    let handle = tokio::spawn(async move {
        loop {
            match subscribe_events_once(&http, &url, &app).await {
                Ok(()) => info!("opencode SSE 连接结束，准备重连"),
                Err(e) => warn!(%e, "opencode SSE 订阅失败，准备重连"),
            }
            tokio::time::sleep(EVENT_RECONNECT_INTERVAL).await;
        }
    });
    *guard = Some(handle);
}

async fn subscribe_events_once(
    http: &reqwest::Client,
    url: &str,
    app: &AppHandle,
) -> Result<(), String> {
    let response = http
        .get(format!("{url}/event"))
        .header(ACCEPT, "text/event-stream")
        .send()
        .await
        .map_err(|e| format!("连接 opencode SSE 失败: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("opencode SSE 返回 {}", response.status()));
    }
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取 SSE 分块失败: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer.drain(..=pos);
            if let Some(data) = line.strip_prefix("data:") {
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    let _ = app.emit(EVENT_NAME, json!({ "event": value }));
                }
            }
        }
    }
    Ok(())
}

pub async fn opencode_stop(manager: &ServerManager) -> Result<Value, String> {
    let runtime = manager.opencode.lock().await.take();
    let Some(runtime) = runtime else {
        return Ok(json!({ "ok": true, "running": false }));
    };
    runtime.running.store(false, Ordering::Relaxed);
    if let Some(mut child) = runtime.child.lock().await.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    if let Some(task) = runtime.mcp_task.lock().await.take() {
        task.abort();
    }
    if let Some(task) = runtime.event_task.lock().await.take() {
        task.abort();
    }
    Ok(json!({ "ok": true, "running": false }))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 拉取 opencode `/doc` 并与 pin 基线比对。
pub async fn verify_doc_contract(runtime: &OpencodeRuntime) -> Result<bool, String> {
    let resp = runtime
        .http
        .get(format!("{}/doc", runtime.url))
        .send()
        .await
        .map_err(|e| format!("请求 /doc 失败: {e}"))?;
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取 /doc 失败: {e}"))?;
    let live = sha256_hex(&bytes);
    Ok(live == OPENCODE_DOC_SHA256)
}

fn opencode_start_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// ── Tauri 命令 ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn dex_opencode_status(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let runtime = manager.opencode.lock().await.clone();
    let Some(runtime) = runtime else {
        return Ok(json!({
            "running": false,
            "available": resolve_binary().is_some(),
            "pin": OPENCODE_PIN_VERSION,
        }));
    };
    Ok(json!({
        "running": runtime.running.load(Ordering::Relaxed),
        "url": runtime.url,
        "port": runtime.port,
        "mcp_url": runtime.mcp_url,
        "mcp_port": runtime.mcp_port,
        "config_dir": runtime.config_dir.to_string_lossy(),
        "pin": OPENCODE_PIN_VERSION,
    }))
}

#[tauri::command]
pub async fn dex_opencode_start(manager: State<'_, ServerManager>) -> Result<Value, String> {
    // 串行化启动：面板懒启动与发消息时的再次启动可能并发调用，
    // 不加锁会导致二次拉起引擎或竞态回退。
    let _guard = opencode_start_lock().lock().await;
    if let Some(runtime) = manager.opencode.lock().await.as_ref() {
        if runtime.running.load(Ordering::Relaxed) {
            return Ok(json!({ "ok": true, "already_running": true }));
        }
    }
    let runtime = opencode_start(manager.inner()).await?;
    *manager.opencode.lock().await = Some(runtime);
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn dex_opencode_stop(manager: State<'_, ServerManager>) -> Result<Value, String> {
    opencode_stop(manager.inner()).await
}

#[tauri::command]
pub async fn dex_opencode_session_create(
    manager: State<'_, ServerManager>,
    title: Option<String>,
) -> Result<Value, String> {
    let runtime = manager
        .opencode
        .lock()
        .await
        .clone()
        .ok_or_else(|| "opencode 引擎未启动".to_string())?;
    runtime
        .session_create(&title.unwrap_or_else(|| "DEX 助手".to_string()))
        .await
}

#[tauri::command]
pub async fn dex_opencode_prompt(
    manager: State<'_, ServerManager>,
    session_id: String,
    text: String,
) -> Result<Value, String> {
    let runtime = manager
        .opencode
        .lock()
        .await
        .clone()
        .ok_or_else(|| "opencode 引擎未启动".to_string())?;
    runtime.prompt(&session_id, &text).await
}

#[tauri::command]
pub async fn dex_opencode_abort(
    manager: State<'_, ServerManager>,
    session_id: String,
) -> Result<Value, String> {
    let runtime = manager
        .opencode
        .lock()
        .await
        .clone()
        .ok_or_else(|| "opencode 引擎未启动".to_string())?;
    runtime.abort(&session_id).await
}

#[tauri::command]
pub async fn dex_opencode_permission_reply(
    manager: State<'_, ServerManager>,
    session_id: String,
    permission_id: String,
    response: String,
) -> Result<Value, String> {
    let runtime = manager
        .opencode
        .lock()
        .await
        .clone()
        .ok_or_else(|| "opencode 引擎未启动".to_string())?;
    runtime
        .permission_reply(&session_id, &permission_id, &response)
        .await
}

#[tauri::command]
pub async fn dex_opencode_events_subscribe(
    manager: State<'_, ServerManager>,
) -> Result<bool, String> {
    let app = manager
        .app_handle
        .lock()
        .await
        .clone()
        .ok_or_else(|| "应用句柄未就绪".to_string())?;
    let runtime = manager
        .opencode
        .lock()
        .await
        .clone()
        .ok_or_else(|| "opencode 引擎未启动".to_string())?;
    spawn_event_forwarder(&runtime, &app).await;
    Ok(true)
}

#[tauri::command]
pub async fn dex_opencode_doc_contract(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let Some(runtime) = manager.opencode.lock().await.clone() else {
        return Ok(json!({ "checked": false, "reason": "引擎未启动" }));
    };
    match verify_doc_contract(&runtime).await {
        Ok(matched) => Ok(json!({
            "checked": true,
            "matched": matched,
            "baseline": OPENCODE_DOC_SHA256,
            "pin": OPENCODE_PIN_VERSION,
        })),
        Err(e) => Ok(json!({ "checked": false, "error": e })),
    }
}

/// 前端探针：webview 加载最新前端时调用，把运行状态写到
/// `<data_dir>/opencode/frontend-probe.json`，用于远程定位前端是否更新。
#[tauri::command]
pub async fn dex_opencode_probe_frontend(
    manager: State<'_, ServerManager>,
    has_opencode: Option<bool>,
    ts: Option<i64>,
    reason: Option<String>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let dir = data_dir.join("opencode");
    let _ = std::fs::create_dir_all(&dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let payload = json!({
        "hasOpencode": has_opencode.unwrap_or(false),
        "ts": ts.unwrap_or(now),
        "now": now,
        "reason": reason.unwrap_or_default(),
        "version": env!("CARGO_PKG_VERSION"),
        "pin": OPENCODE_PIN_VERSION,
    });
    let _ = std::fs::write(
        dir.join("frontend-probe.json"),
        serde_json::to_string_pretty(&payload).unwrap_or_default(),
    );
    Ok(payload)
}

#[tauri::command]
pub async fn dex_opencode_config_info(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let states = load_capability_states(&data_dir);
    let tools = all_tool_defs(manager.inner()).await;
    let enabled: Vec<Value> = tools
        .iter()
        .filter(|t| is_capability_enabled(&states, &t.capability))
        .map(|t| {
            json!({
                "name": t.name,
                "level": t.level,
                "capability": t.capability,
                "group": super::dex_opencode_config::capability_to_group(&t.capability),
            })
        })
        .collect();
    let account = get_active_account_info(&data_dir)
        .map(|(upstream, _key, _map, models, provider, ..)| {
            json!({
                "upstream": upstream,
                "provider": provider,
                "models": models,
            })
        })
        .unwrap_or_else(|| json!(null));
    Ok(json!({
        "enabled_tool_count": enabled.len(),
        "tools": enabled,
        "account": account,
        "binary": resolve_binary().map(|p| p.to_string_lossy().to_string()),
        "pin": OPENCODE_PIN_VERSION,
    }))
}
