//! CDP 注入编排器。
//!
//! 在 deecodex daemon 启动时，检测 Codex 的 CDP 远程调试端口，
//! 连接并注入 JavaScript（插件解锁 + CDP 桥接）。

use std::sync::Arc;

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

            info!("CDP 注入成功 (端口 {port})：插件解锁 + 会话删除 UI 已激活");
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

    // 4. 取走 WebSocket，启动后台桥接循环
    let ws = client.take_ws()?;
    tokio::spawn(async move {
        run_bridge_loop(ws, state).await;
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

/// 桥接循环：监听 CDP Runtime.bindingCalled 事件，处理删除/撤销请求。
async fn run_bridge_loop(mut ws: crate::cdp::CdpWsStream, state: Arc<AppState>) {
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

        if parsed.get("method").and_then(|v| v.as_str()) != Some("Runtime.bindingCalled") {
            continue;
        }

        let params = match parsed.get("params") {
            Some(p) => p,
            None => continue,
        };
        let payload_str = params
            .get("payload")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let payload: serde_json::Value = match serde_json::from_str(payload_str) {
            Ok(v) => v,
            Err(e) => {
                warn!("CDP 桥接 payload 解析失败: {e}");
                continue;
            }
        };

        let request_id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let path = payload.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let data = payload.get("payload").cloned().unwrap_or_default();

        let result = handle_bridge(&state, path, data).await;

        let expression = format!(
            "window.__deecodexResolve({}, {});",
            serde_json::to_string(request_id).unwrap_or_default(),
            serde_json::to_string(&result).unwrap_or_default(),
        );
        let id = next_bridge_id();
        let resolve_msg = serde_json::json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": {
                "expression": expression,
                "awaitPromise": false,
                "allowUnsafeEvalBlockedByCSP": true,
            }
        });
        if let Ok(text) = serde_json::to_string(&resolve_msg) {
            let _ = ws
                .send(Message::Text(text))
                .await
                .inspect_err(|e| warn!("CDP 桥接响应发送失败: {e}"));
        }
    }
    info!("CDP 桥接循环已退出");
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
        _ => serde_json::json!({"status": "failed", "message": "未知桥接路径"}),
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
