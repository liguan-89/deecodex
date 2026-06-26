//! CDP 注入编排器。
//!
//! 在 deecodex daemon 启动时，检测 Codex 的 CDP 远程调试端口，
//! 连接并注入 JavaScript（插件解锁 + 强制安装 + 模型选择器扩展 + Statsig 离线回退）。

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine;
use futures_util::SinkExt;
use tracing::{info, warn};

use crate::cdp::{self, CdpClient};
use crate::handlers::AppState;

/// CDP 桥接绑定名。
const BRIDGE_NAME: &str = "deecodexBridge";

/// 尝试连接到 Codex CDP 端口并注入 JS。
/// 优先尝试配置的 cdp_port，再扫描 9222–9230 端口，找到 Codex 页面目标后注入。
/// 注入失败不影响代理功能。
#[allow(dead_code)]
pub async fn try_inject(state: Arc<AppState>) {
    try_inject_with_port(state, 0).await
}

/// 带优先端口的注入逻辑。若 priority_port > 0 则优先尝试。
pub async fn try_inject_with_port(state: Arc<AppState>, priority_port: u16) {
    // 重试 30 次，每次间隔 1 秒，给 Codex 启动留出时间
    for attempt in 0..30 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        // 优先尝试配置的端口
        let mut ports: Vec<u16> = Vec::new();
        if priority_port > 0 {
            ports.push(priority_port);
        }
        ports.extend(9222..=9250_u16);

        for port in ports {
            let targets = match cdp::list_targets(port).await {
                Ok(t) => t,
                Err(_) => continue,
            };

            let Some(ws_url) = cdp::find_codex_page(&targets) else {
                continue;
            };

            info!("检测到 Codex CDP 页面 (端口 {port})，开始注入...");

            let mut client = match CdpClient::connect(&ws_url).await {
                Ok(c) => c,
                Err(e) => {
                    warn!("CDP WebSocket 连接失败 (端口 {port}): {e}");
                    continue;
                }
            };

            if let Err(e) = do_inject(&mut client, Arc::clone(&state)).await {
                warn!("CDP 注入失败 (端口 {port}): {e}");
                continue;
            }

            info!("CDP 注入成功 (端口 {port})：插件解锁 + 模型选择器扩展 + Statsig 离线回退已激活");
            return;
        }
    }

    let port_hint = if priority_port > 0 {
        format!("优先端口 {priority_port}，")
    } else {
        String::new()
    };
    info!("未检测到 Codex CDP 调试端口 ({port_hint}扫描范围 9222–9230，已重试 30 次)，跳过注入。");
    info!("如需使用插件解锁和会话删除 UI，请以 --remote-debugging-port=9222 启动 Codex 桌面版，或在控制面板点击“启动 Codex CDP”。");
}

/// 执行注入流程。
async fn do_inject(client: &mut CdpClient, state: Arc<AppState>) -> anyhow::Result<()> {
    let inject_js = include_str!("../static/inject.js");
    let combined = format!("{}\n{}", BRIDGE_SHIM_JS, inject_js);

    // 1. 注册脚本到所有新页面（SPA 导航/重载后自动重新注入）
    client.add_script_to_new_documents(&combined).await?;

    // 2. 注册 CDP 绑定（创建 window.deecodexBridge() 函数）
    client.add_binding(BRIDGE_NAME).await?;

    // 3. 在当前页面立即执行
    client.evaluate(&combined).await?;

    // 4. 加载本地缓存的 Statsig 初始化响应（如果存在），启用 Fetch 拦截
    // Codex 实际用的 Statsig 端点是 https://ab.chatgpt.com/v1/initialize
    // （app.asar 里 protocol-relative `//ab.chatgpt.com` + 路径 `/v1/initialize`）。
    // CSP `connect-src` 只允许 `https://ab.chatgpt.com`，所以 Statsig 必须用它。
    // 额外拦截 Statsig 官方 CDN 作为兜底（防止 Codex 改端点）。
    const STATSIG_PATTERNS: &[&str] = &[
        "*://ab.chatgpt.com/*",
        "*://api.statsigcdn.com/*",
        "*://statsigapi.net/*",
    ];
    let cached_init = load_cached_statsig_init(&state);
    if let Some(ref cache) = cached_init {
        if let Err(e) = client.fetch_enable(STATSIG_PATTERNS).await {
            warn!("Fetch.enable 失败（Statsig 离线回退不可用）: {e}");
        } else {
            info!(
                "已加载本地 Statsig 配置（{} 字节），Statsig 请求将由 CDP 直接回填",
                cache.body.len()
            );
        }
    } else {
        info!(
            "未找到本地 Statsig 配置（{}），首次启动会通过注入脚本自动捕获",
            statsig_init_path(&state).display()
        );
        // 即使没有缓存也启用拦截，配合注入脚本的捕获路径
        if let Err(e) = client.fetch_enable(STATSIG_PATTERNS).await {
            warn!("Fetch.enable 失败（Statsig 捕获路径不可用）: {e}");
        }
    }

    // 5. 取走 WebSocket，启动后台桥接循环
    let ws = client.take_ws()?;
    tokio::spawn(async move {
        run_bridge_loop(ws, state, cached_init).await;
    });

    Ok(())
}

/// CDP 桥接垫片 JS — 在页面中创建 window.__deecodexBridge(path, payload) → Promise。
const BRIDGE_SHIM_JS: &str = r#"
(function(){
    const cbs = new Map();
    let seq = 0;
    window.__deecodexResolve = function(id, result) {
        const cb = cbs.get(String(id));
        if (cb) { cbs.delete(String(id)); cb.resolve(result); }
    };
    window.__deecodexBridge = function(path, payload) {
        return new Promise(function(resolve) {
            const id = String(++seq);
            cbs.set(id, { resolve: resolve });
            window.deecodexBridge(JSON.stringify({ id: id, path: path, payload: payload }));
        });
    };
})();
"#;

/// 桥接循环：监听 CDP Runtime.bindingCalled 与 Fetch.requestPaused 事件。
///
/// `cached_init` 为 `Some` 时，对 `api.statsigcdn.com/v1/initialize` 请求
/// 直接用本地缓存回填（Statsig 离线回退）；其他请求透传。
async fn run_bridge_loop(
    mut ws: crate::cdp::CdpWsStream,
    state: Arc<AppState>,
    cached_init: Option<CachedStatsigInit>,
) {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    while let Some(msg) = ws.next().await {
        let text = match msg {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(_)) => break,
            Err(e) => {
                warn!("CDP 桥接 WebSocket 错误: {e}");
                break;
            }
            _ => continue,
        };

        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match parsed.get("method").and_then(|v| v.as_str()) {
            Some("Runtime.bindingCalled") => {
                if let Err(e) = handle_binding_called(&mut ws, &parsed, state.as_ref()).await {
                    warn!("CDP 桥接 binding 处理失败: {e}");
                }
            }
            Some("Fetch.requestPaused") => {
                if let Err(e) = handle_fetch_request_paused(&mut ws, &parsed, &cached_init).await {
                    warn!("CDP Fetch 事件处理失败: {e}");
                }
            }
            _ => continue,
        }
    }
    info!("CDP 桥接循环已退出");
}

/// 处理 Runtime.bindingCalled：读取 payload，调用对应桥接端点，把结果以
/// `__deecodexResolve(id, result)` 形式回传给页面。
async fn handle_binding_called(
    ws: &mut crate::cdp::CdpWsStream,
    event: &serde_json::Value,
    state: &AppState,
) -> anyhow::Result<()> {
    use tokio_tungstenite::tungstenite::Message;

    let params = event
        .get("params")
        .ok_or_else(|| anyhow::anyhow!("bindingCalled 缺少 params"))?;
    let payload_str = params
        .get("payload")
        .and_then(|v| v.as_str())
        .unwrap_or("{}");
    let payload: serde_json::Value = serde_json::from_str(payload_str)
        .map_err(|e| anyhow::anyhow!("bindingCalled payload 解析失败: {e}"))?;

    let request_id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let data = payload.get("payload").cloned().unwrap_or_default();

    let result = handle_bridge(state, path, data).await;

    let expression = format!(
        "window.__deecodexResolve({}, {});",
        serde_json::to_string(request_id).unwrap_or_default(),
        serde_json::to_string(&result).unwrap_or_default(),
    );
    let resolve_msg = serde_json::json!({
        "id": next_bridge_id(),
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "awaitPromise": false,
            "allowUnsafeEvalBlockedByCSP": true,
        }
    });
    let text = serde_json::to_string(&resolve_msg)?;
    ws.send(Message::Text(text))
        .await
        .map_err(|e| anyhow::anyhow!("CDP 桥接响应发送失败: {e}"))?;
    Ok(())
}

/// 处理 Fetch.requestPaused 事件。
///
/// - 命中 `api.statsigcdn.com/v1/initialize` 且存在本地缓存：用缓存回填（Statsig 离线回退）
/// - 命中 `ab.chatgpt.com/v1/initialize` 但无缓存：放行（让真实请求通过，由注入脚本捕获）
/// - 其他请求：放行
async fn handle_fetch_request_paused(
    ws: &mut crate::cdp::CdpWsStream,
    event: &serde_json::Value,
    cached_init: &Option<CachedStatsigInit>,
) -> anyhow::Result<()> {
    let params = event
        .get("params")
        .ok_or_else(|| anyhow::anyhow!("requestPaused 缺少 params"))?;
    let request_id = params
        .get("requestId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("requestPaused 缺少 requestId"))?;
    let url = params
        .pointer("/request/url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // 响应阶段的 paused 事件（responseStatusCode 已存在）目前不需要干预，全部放行
    if params.get("responseStatusCode").is_some() {
        return send_continue_response(ws, request_id).await;
    }

    // Codex 的 Statsig 客户端用的是 `/v1/rgstr`（register）端点，
    // 不是标准 Statsig SDK 的 `/v1/initialize`。同时拦截两者做兼容。
    let is_statsig = url.contains("ab.chatgpt.com")
        || url.contains("api.statsigcdn.com")
        || url.contains("statsigapi.net");
    let is_init =
        url.contains("/v1/initialize") || url.contains("/v1/rgstr") || url.contains("/v1/evaluate");
    if is_statsig && is_init {
        if let Some(cache) = cached_init {
            info!(
                "Statsig 离线回退：使用本地缓存回填 {url}（{} 字节）",
                cache.body.len()
            );
            return send_fulfill(ws, request_id, &cache.content_type, &cache.body).await;
        }
        info!("Statsig 捕获路径：放行 {url}，等待注入脚本捕获响应");
    }

    send_continue_request(ws, request_id).await
}

async fn send_continue_request(
    ws: &mut crate::cdp::CdpWsStream,
    request_id: &str,
) -> anyhow::Result<()> {
    let msg = serde_json::json!({
        "id": next_bridge_id(),
        "method": "Fetch.continueRequest",
        "params": { "requestId": request_id }
    });
    cdp::CdpClient::send_raw(ws, &msg).await
}

async fn send_continue_response(
    ws: &mut crate::cdp::CdpWsStream,
    request_id: &str,
) -> anyhow::Result<()> {
    let msg = serde_json::json!({
        "id": next_bridge_id(),
        "method": "Fetch.continueResponse",
        "params": { "requestId": request_id }
    });
    cdp::CdpClient::send_raw(ws, &msg).await
}

async fn send_fulfill(
    ws: &mut crate::cdp::CdpWsStream,
    request_id: &str,
    content_type: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    use base64::engine::general_purpose::STANDARD;
    let b64 = STANDARD.encode(body);
    let msg = serde_json::json!({
        "id": next_bridge_id(),
        "method": "Fetch.fulfillRequest",
        "params": {
            "requestId": request_id,
            "responseCode": 200,
            "responseHeaders": [
                { "name": "Content-Type", "value": content_type },
                { "name": "Access-Control-Allow-Origin", "value": "*" },
                { "name": "Access-Control-Allow-Headers", "value": "*" },
            ],
            "body": b64,
        }
    });
    cdp::CdpClient::send_raw(ws, &msg).await
}

fn next_bridge_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(1000);
    ID.fetch_add(1, Ordering::Relaxed)
}

async fn handle_bridge(state: &AppState, path: &str, data: serde_json::Value) -> serde_json::Value {
    match path {
        "/idb-report" => handle_idb_report(state, &data).await,
        "/models" => handle_models(state).await,
        "/statsig-init" => handle_statsig_init(state, &data).await,
        _ => serde_json::json!({"status": "failed", "message": "未知桥接路径"}),
    }
}

/// 注入脚本使用的本地 Statsig 初始化响应缓存。
/// 加载自 `data_dir/statsig_init_zh.json`（首次启动时由注入脚本捕获写入）。
struct CachedStatsigInit {
    content_type: String,
    body: Vec<u8>,
}

fn statsig_init_path(state: &AppState) -> PathBuf {
    state.data_dir.join("statsig_init_zh.json")
}

fn load_cached_statsig_init(state: &AppState) -> Option<CachedStatsigInit> {
    let path = statsig_init_path(state);
    let body = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return None,
    };
    if body.is_empty() {
        return None;
    }
    Some(CachedStatsigInit {
        content_type: "application/json; charset=utf-8".to_string(),
        body,
    })
}

/// Statsig `/v1/initialize` 响应缓存的桥接端点。
///
/// 注入脚本在启动时调用 GET 检查缓存是否存在；
/// 捕获路径在收到真实响应后通过 POST 把响应体写回磁盘。
async fn handle_statsig_init(state: &AppState, data: &serde_json::Value) -> serde_json::Value {
    use base64::engine::general_purpose::STANDARD;
    let method = data.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
    let path = statsig_init_path(state);
    match method {
        "GET" => match std::fs::read(&path) {
            Ok(body) => serde_json::json!({
                "status": "ok",
                "size": body.len(),
                "body_b64": STANDARD.encode(&body),
                "content_type": "application/json; charset=utf-8",
            }),
            Err(_) => serde_json::json!({"status": "empty"}),
        },
        "POST" => {
            let b64 = data.get("body_b64").and_then(|v| v.as_str()).unwrap_or("");
            if b64.is_empty() {
                return serde_json::json!({"status": "failed", "message": "缺少 body_b64"});
            }
            let body = match STANDARD.decode(b64) {
                Ok(b) => b,
                Err(e) => {
                    return serde_json::json!({
                        "status": "failed",
                        "message": format!("base64 解码失败: {e}"),
                    });
                }
            };
            match std::fs::write(&path, &body) {
                Ok(_) => {
                    info!(
                        "Statsig 初始化响应已保存到 {}（{} 字节）",
                        path.display(),
                        body.len()
                    );
                    serde_json::json!({"status": "ok", "size": body.len()})
                }
                Err(e) => serde_json::json!({
                    "status": "failed",
                    "message": format!("写入失败: {e}"),
                }),
            }
        }
        _ => serde_json::json!({"status": "failed", "message": "未知方法"}),
    }
}

/// 返回 ~/.codex/models_deecodex.json 中的所有模型条目。
/// Codex 桌面版 UI 通过桥接调用这个路径来扩展模型选择器。
async fn handle_models(_state: &AppState) -> serde_json::Value {
    let Some(home) = crate::codex_config::codex_home_dir() else {
        return serde_json::json!({"status": "failed", "message": "无法定位 codex home"});
    };
    let path = home.join("models_deecodex.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            return serde_json::json!({"status": "failed", "message": format!("读取模型目录失败: {e}")});
        }
    };
    let parsed: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({"status": "failed", "message": format!("解析失败: {e}")});
        }
    };
    let items = if parsed.is_array() {
        parsed
    } else if let Some(arr) = parsed.get("models").and_then(|v| v.as_array()) {
        serde_json::Value::Array(arr.clone())
    } else {
        return serde_json::json!({"status": "failed", "message": "模型目录格式异常"});
    };
    serde_json::json!({"status": "ok", "models": items})
}

/// 处理 IndexedDB 探查报告：写入文件并记录日志。
async fn handle_idb_report(state: &AppState, data: &serde_json::Value) -> serde_json::Value {
    // 写入 idb_report.json
    let report_path = state.data_dir.join("idb_report.json");
    let report_json = serde_json::to_string_pretty(data).unwrap_or_default();
    match std::fs::write(&report_path, &report_json) {
        Ok(_) => {
            tracing::info!(
                "IndexedDB 探查报告已保存到 {} ({} 字节)",
                report_path.display(),
                report_json.len()
            );
            // 同时输出到日志，方便直接查看
            tracing::info!("IndexedDB 探查报告:\n{report_json}");
        }
        Err(e) => {
            tracing::warn!("IndexedDB 探查报告写入失败: {e}");
            // 至少输出到日志
            tracing::info!("IndexedDB 探查报告 (未持久化):\n{report_json}");
        }
    }
    serde_json::json!({"status": "ok", "message": "探查报告已保存"})
}
