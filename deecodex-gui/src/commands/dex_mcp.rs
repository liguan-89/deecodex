//! DEX MCP 数据源（Streamable HTTP）
//!
//! 按能力包分组暴露 DEX 工具给 opencode：`/mcp/{group}`。
//! 传输行为已在 opencode 1.18.16 上实证：
//! - initialize / tools/list 走 JSON 响应；
//! - tools/call 必须返回 202 并把结果通过 GET SSE 推送（直接 JSON 响应会被客户端 abort）。
//!
//! 认证：`Authorization: Bearer <token>`。

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, Mutex};
use tracing::{debug, warn};

use crate::ServerManager;

use super::dex::all_tool_defs;
use super::dex_opencode_config::capability_to_group;
use super::dex_registry::{is_capability_enabled, load_capability_states};
use super::dex_security::mask_sensitive_value;
use super::dex_tool_executor::dex_execute_tool_impl;

const MCP_PROTOCOL_VERSION: &str = "2025-11-25";
/// axum 0.7 路径参数语法是 `:group`（`{group}` 是 0.8 语法，曾导致路由全部 404）。
const MCP_ROUTE: &str = "/mcp/:group";

#[derive(Clone)]
pub struct McpShared {
    token: Arc<str>,
    app: AppHandle,
    /// group -> broadcast sender（SSE data 行，JSON 字符串）
    groups: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
}

impl McpShared {
    pub fn new(token: String, app: AppHandle) -> Self {
        Self {
            token: Arc::from(token),
            app,
            groups: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn sender_for(&self, group: &str) -> broadcast::Sender<String> {
        let mut guard = self.groups.lock().await;
        if let Some(tx) = guard.get(group) {
            return tx.clone();
        }
        let (tx, _) = broadcast::channel(256);
        guard.insert(group.to_string(), tx.clone());
        tx
    }

    async fn push(&self, group: &str, data: String) {
        let sender = self.sender_for(group).await;
        if sender.receiver_count() == 0 {
            warn!(group, "MCP group 无 SSE 订阅者，结果未推送");
            return;
        }
        let _ = sender.send(data);
    }
}

pub fn mcp_router(shared: McpShared) -> Router {
    Router::new()
        .route(MCP_ROUTE, get(handle_get).post(handle_post))
        .with_state(shared)
}

fn check_auth(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == token)
        .unwrap_or(false)
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "jsonrpc": "2.0",
            "error": { "code": -32001, "message": "unauthorized" }
        })),
    )
        .into_response()
}

fn error_response(id: Value, code: i64, message: &str) -> Response {
    (
        StatusCode::OK,
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        })),
    )
        .into_response()
}

/// 把 DEX 工具映射为 MCP tools 列表（按分组过滤）。
/// 工具名用原始名（不加分组前缀）：opencode 会用自己的 server 名前缀拼出
/// 最终工具 id（`dex_ops_get_config`），若这里再加前缀会变成
/// `dex_ops_dex_ops_get_config` 并导致 permission 配置匹配不上。
fn mcp_tools_for_group(group: &str, defs: &[super::dex_registry::DexToolDef]) -> Vec<Value> {
    defs.iter()
        .filter(|tool| capability_to_group(&tool.capability) == group)
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": tool.parameters,
            })
        })
        .collect()
}

/// 去掉 `<server>_` 前缀，还原 DEX 工具名。
fn strip_group_prefix(group: &str, prefixed_name: &str) -> String {
    prefixed_name
        .strip_prefix(&format!("{group}_"))
        .unwrap_or(prefixed_name)
        .to_string()
}

/// GET /mcp/{group}：SSE 服务端推送通道（tools/call 结果 + listChanged 通知）
async fn handle_get(
    State(shared): State<McpShared>,
    Path(group): Path<String>,
    headers: HeaderMap,
) -> Response {
    if !check_auth(&headers, &shared.token) {
        return unauthorized();
    }
    let sender = shared.sender_for(&group).await;
    let rx = sender.subscribe();
    let endpoint_json = json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": { "name": "deecodex-mcp", "version": env!("CARGO_PKG_VERSION") }
    })
    .to_string();
    let stream = futures_util::stream::unfold((rx, true), move |(mut rx, first)| {
        let ep = endpoint_json.clone();
        async move {
            if first {
                // 必须用 Event::event("endpoint") 构造合法 SSE（event 行不能拼进 data）
                let evt = Event::default().event("endpoint").data(ep);
                return Some((Ok::<_, Infallible>(evt), (rx, false)));
            }
            match rx.recv().await {
                Ok(line) => Some((
                    Ok::<_, Infallible>(Event::default().data(line)),
                    (rx, false),
                )),
                Err(_) => None,
            }
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// POST /mcp/{group}：JSON-RPC 2.0
async fn handle_post(
    State(shared): State<McpShared>,
    Path(group): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    if !check_auth(&headers, &shared.token) {
        return unauthorized();
    }
    let Ok(msg) = serde_json::from_str::<Value>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": "parse error" } }),
            ),
        )
            .into_response();
    };
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let id = msg.get("id").cloned().unwrap_or(Value::Null);

    match method.as_str() {
        "initialize" => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": { "name": "deecodex-mcp", "version": env!("CARGO_PKG_VERSION") }
            }
        }))
        .into_response(),
        "notifications/initialized" | "notifications/cancelled" | "notifications/progress" => {
            StatusCode::ACCEPTED.into_response()
        }
        "ping" => Json(json!({ "jsonrpc": "2.0", "id": id, "result": {} })).into_response(),
        "tools/list" => {
            let manager = shared.app.state::<ServerManager>();
            let data_dir = manager.data_dir.lock().await.clone();
            let states = load_capability_states(&data_dir);
            let all_defs = all_tool_defs(&manager).await;
            let defs: Vec<super::dex_registry::DexToolDef> = all_defs
                .iter()
                .filter(|tool| is_capability_enabled(&states, &tool.capability))
                .cloned()
                .collect();
            let tools = mcp_tools_for_group(&group, &defs);
            debug!(group, count = tools.len(), "MCP tools/list");
            Json(json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools } }))
                .into_response()
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let shared_clone = shared.clone();
            let group_clone = group.clone();
            // 202 + SSE 推送结果
            tokio::spawn(async move {
                let result =
                    execute_dex_tool(&shared_clone.app, &group_clone, &name, arguments).await;
                let payload = match result {
                    Ok(value) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": value }],
                            "isError": false
                        }
                    }),
                    Err(message) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{ "type": "text", "text": message }],
                            "isError": true
                        }
                    }),
                };
                shared_clone.push(&group_clone, payload.to_string()).await;
            });
            StatusCode::ACCEPTED.into_response()
        }
        _ => error_response(id, -32601, "method not found"),
    }
}

/// 解析工具名（去掉 `<server>_` 前缀）并转发到 DEX 执行器。
/// L3 工具由 opencode 的 permission ask 门禁授权后到达这里，因此带 confirmed=true。
async fn execute_dex_tool(
    app: &AppHandle,
    group: &str,
    prefixed_name: &str,
    arguments: Value,
) -> Result<String, String> {
    let manager = app.state::<ServerManager>();
    let defs = all_tool_defs(&manager).await;
    let raw_name = strip_group_prefix(group, prefixed_name);
    let Some(tool) = defs.iter().find(|t| t.name == raw_name) else {
        return Err(format!("未知工具: {prefixed_name}"));
    };
    let confirmed = Some(tool.level >= 3);
    let result = dex_execute_tool_impl(manager, raw_name, Some(arguments), confirmed).await?;
    Ok(mask_sensitive_value(result).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, capability: &str, level: u8) -> super::super::dex_registry::DexToolDef {
        super::super::dex_registry::DexToolDef {
            name: name.to_string(),
            tauri_cmd: name.to_string(),
            level,
            confirm: None,
            description: format!("tool {name}"),
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
            capability: capability.to_string(),
            source: "builtin".to_string(),
            plugin_id: None,
            plugin_method: None,
        }
    }

    #[test]
    fn tools_are_grouped_with_raw_names() {
        let defs = vec![
            tool("get_config", "deecodex.ops", 0),
            tool("execute_shell", "core.workspace", 3),
            tool("run_diagnostics", "deecodex.ops", 0),
        ];
        let ops = mcp_tools_for_group("dex_ops", &defs);
        assert_eq!(ops.len(), 2);
        assert!(ops.iter().any(|t| t["name"] == "get_config"));
        assert!(ops.iter().any(|t| t["name"] == "run_diagnostics"));
        let ws = mcp_tools_for_group("dex_workspace", &defs);
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0]["name"], "execute_shell");
        assert_eq!(mcp_tools_for_group("dex_plugins", &defs).len(), 0);
    }

    #[test]
    fn strip_group_prefix_handles_unknown() {
        assert_eq!(
            strip_group_prefix("dex_ops", "dex_ops_get_config"),
            "get_config"
        );
        assert_eq!(strip_group_prefix("dex_ops", "get_config"), "get_config");
    }

    #[test]
    fn mcp_route_uses_axum07_param_syntax() {
        assert_eq!(MCP_ROUTE, "/mcp/:group");
        assert!(!MCP_ROUTE.contains('{'));
    }
}
