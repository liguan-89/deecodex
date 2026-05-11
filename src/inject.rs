//! CDP 注入编排器。
//!
//! 在 deecodex daemon 启动时，检测 Codex 的 CDP 远程调试端口，
//! 连接并注入 JavaScript（插件解锁 + 会话删除 UI + CDP 桥接）。

use std::sync::Arc;

use futures_util::SinkExt;
use tracing::{info, warn};

use crate::backup_store::BackupStore;
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
    // 1. 注册 CDP 绑定（创建 window.deecodexBridge() 函数）
    client.add_binding(BRIDGE_NAME).await?;

    // 2. 注入桥接垫片 + 主脚本（一起执行，顺序依赖）
    let inject_js = include_str!("../static/inject.js");
    let combined = format!("{}\n{}", BRIDGE_SHIM_JS, inject_js);
    client.evaluate(&combined).await?;

    // 3. 取走 WebSocket，启动后台桥接循环
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
        "/delete" => handle_delete(state, &data).await,
        "/undo" => handle_undo(state, &data).await,
        _ => serde_json::json!({"status": "failed", "message": "未知桥接路径"}),
    }
}

async fn handle_delete(state: &AppState, data: &serde_json::Value) -> serde_json::Value {
    let session_id = data
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let title = data.get("title").and_then(|v| v.as_str()).unwrap_or("");

    if session_id.is_empty() {
        return serde_json::json!({"status": "failed", "message": "缺少 session_id"});
    }

    // 尝试作为 response 删除
    if let Some((messages, response, input_items)) =
        state.sessions.delete_response_with_data(session_id)
    {
        match create_backup(
            &state.data_dir,
            session_id,
            "response",
            &messages,
            Some(&response),
            Some(&input_items),
        )
        .await
        {
            Ok(undo_token) => {
                info!("会话已删除 (response): {session_id} ({title})");
                serde_json::json!({
                    "status": "deleted",
                    "session_id": session_id,
                    "message": format!("已删除「{title}」"),
                    "undo_token": undo_token
                })
            }
            Err(e) => {
                warn!("备份失败: {e}");
                serde_json::json!({
                    "status": "deleted",
                    "session_id": session_id,
                    "message": format!("已删除「{title}」（备份失败）")
                })
            }
        }
    }
    // 尝试作为 conversation 删除
    else if let Some((messages, items)) = state.sessions.delete_conversation_with_data(session_id)
    {
        match create_backup(
            &state.data_dir,
            session_id,
            "conversation",
            &messages,
            None,
            Some(&items),
        )
        .await
        {
            Ok(undo_token) => {
                info!("会话已删除 (conversation): {session_id} ({title})");
                serde_json::json!({
                    "status": "deleted",
                    "session_id": session_id,
                    "message": format!("已删除「{title}」"),
                    "undo_token": undo_token
                })
            }
            Err(e) => {
                warn!("备份失败: {e}");
                serde_json::json!({
                    "status": "deleted",
                    "session_id": session_id,
                    "message": format!("已删除「{title}」（备份失败）")
                })
            }
        }
    }
    // 未在 session store 中找到 — 通知前端仅移除 UI
    else {
        serde_json::json!({
            "status": "local_deleted",
            "session_id": session_id,
            "message": format!("已移除「{title}」（未找到服务端记录）")
        })
    }
}

async fn handle_undo(state: &AppState, data: &serde_json::Value) -> serde_json::Value {
    let token = data
        .get("undo_token")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if token.is_empty() {
        return serde_json::json!({"status": "failed", "message": "缺少 undo_token"});
    }

    let backup_dir = state.data_dir.join("backups");
    let backup_store = match BackupStore::new(backup_dir) {
        Ok(bs) => bs,
        Err(e) => {
            return serde_json::json!({"status": "failed", "message": format!("无法访问备份目录: {e}")});
        }
    };

    let backup = match backup_store.read_backup(token) {
        Ok(b) => b,
        Err(e) => {
            return serde_json::json!({"status": "failed", "message": format!("备份不存在: {e}")});
        }
    };

    let session_id = backup
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let session_type = backup
        .get("session_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let data = &backup["data"];
    let messages: Vec<crate::types::ChatMessage> = data
        .get("messages")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    if session_type == "response" {
        let response: serde_json::Value = data.get("response").cloned().unwrap_or_default();
        let input_items: Vec<serde_json::Value> = data
            .get("input_items")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        state
            .sessions
            .undo_delete_response(session_id, messages, response, input_items);
        info!("已撤销删除 (response): {session_id}");
    } else {
        let items: Vec<serde_json::Value> = data
            .get("items")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        state
            .sessions
            .undo_delete_conversation(session_id, messages, items);
        info!("已撤销删除 (conversation): {session_id}");
    }

    let _ = backup_store.delete_backup(token);

    serde_json::json!({
        "status": "undone",
        "session_id": session_id,
        "message": "已撤销删除"
    })
}

async fn create_backup(
    data_dir: &std::path::Path,
    session_id: &str,
    session_type: &str,
    messages: &[crate::types::ChatMessage],
    response: Option<&serde_json::Value>,
    input_items: Option<&[serde_json::Value]>,
) -> anyhow::Result<String> {
    let backup_dir = data_dir.join("backups");
    let backup_store = BackupStore::new(backup_dir)?;

    let data = serde_json::json!({
        "messages": serde_json::to_value(messages).unwrap_or_default(),
        "response": response.cloned().unwrap_or_default(),
        "input_items": input_items.map(|i| serde_json::to_value(i).unwrap_or_default()).unwrap_or_default(),
        "items": input_items.map(|i| serde_json::to_value(i).unwrap_or_default()).unwrap_or_default(),
    });

    backup_store.write_backup(session_id, session_type, &data)
}
