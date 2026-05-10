use crate::backup_store::BackupStore;
use crate::config::Args;
use crate::handlers::{
    handle_get_tool_policy, handle_put_tool_policy, AppState,
};
use crate::types::ChatMessage;
use crate::validate;

use axum::{
    extract::{Path, State},
    http::header,
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde_json::{json, Value};

/// 构建 Web 配置面板路由（不需要 client_auth）
pub fn build_web_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(handle_web_panel))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/config/validate", get(validate_config))
        .route("/api/status", get(get_status))
        .route("/api/restart", post(post_restart))
        .route("/api/stop", post(post_stop))
        .route("/api/logs", get(get_logs))
        .route("/api/update", post(post_update))
        .route("/api/sessions", get(handle_list_sessions))
        .route("/api/sessions/undo", post(handle_undo_delete))
        .route("/api/sessions/responses/:response_id", delete(handle_delete_response_with_backup))
        .route("/api/sessions/conversations/:conversation_id", delete(handle_delete_conversation_with_backup))
        .route("/api/tool-policy", get(handle_get_tool_policy).put(handle_put_tool_policy))
        .with_state(state)
}

/// GET / — 返回 Web 配置面板 HTML
pub async fn handle_web_panel() -> impl IntoResponse {
    let html = if std::env::var("DEECODEX_DEV").as_deref() == Ok("1") {
        std::fs::read_to_string("static/config.html")
            .unwrap_or_else(|_| include_str!("../static/config.html").to_string())
    } else {
        include_str!("../static/config.html").to_string()
    };
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html)
}

/// 从配置文件加载 Args，若文件不存在则返回 None
fn load_args(data_dir: &std::path::Path) -> Option<Args> {
    Args::load_from_file(&Args::default_config_path(data_dir))
}

/// 从 AppState 构建 fallback Args（配置文件中不存在时使用）
fn fallback_args(state: &AppState) -> Args {
    Args {
        command: None,
        config: None,
        port: 4446,
        upstream: state.upstream.as_ref().to_string(),
        api_key: state.api_key.as_ref().to_string(),
        client_api_key: state.client_api_key.try_read().map(|g| g.clone()).unwrap_or_default(),
        model_map: if state.model_map.is_empty() {
            "{}".into()
        } else {
            serde_json::to_string(&*state.model_map).unwrap_or_else(|_| "{}".into())
        },
        max_body_mb: 100,
        vision_upstream: state
            .vision_upstream
            .as_ref()
            .map(|u| u.as_ref().to_string())
            .unwrap_or_default(),
        vision_api_key: state.vision_api_key.as_ref().to_string(),
        vision_model: state.vision_model.as_ref().to_string(),
        vision_endpoint: state.vision_endpoint.as_ref().to_string(),
        chinese_thinking: state.chinese_thinking,
        codex_auto_inject: state.codex_auto_inject,
        codex_persistent_inject: state.codex_persistent_inject,
        codex_launch_with_cdp: state.codex_launch_with_cdp,
        cdp_port: state.cdp_port,
        data_dir: state.data_dir.as_ref().clone(),
        prompts_dir: state.data_dir.join("prompts"),
        token_anomaly_prompt_max: 200000,
        token_anomaly_spike_ratio: 5.0,
        token_anomaly_burn_window: 120,
        token_anomaly_burn_rate: 500000,
        allowed_mcp_servers: String::new(),
        allowed_computer_displays: String::new(),
        computer_executor: "disabled".into(),
        computer_executor_timeout_secs: 30,
        mcp_executor_config: String::new(),
        mcp_executor_timeout_secs: 30,
        playwright_state_dir: String::new(),
        browser_use_bridge_url: String::new(),
        browser_use_bridge_command: String::new(),
        daemon: false,
    }
}

/// GET /api/config — 获取当前配置（敏感字段遮蔽）
pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let args = load_args(&state.data_dir).unwrap_or_else(|| fallback_args(&state));
    Json(json!({
        "port": args.port,
        "upstream": args.upstream,
        "api_key": Args::mask_sensitive(&args.api_key),
        "client_api_key": Args::mask_full(&args.client_api_key),
        "model_map": args.model_map,
        "max_body_mb": args.max_body_mb,
        "vision_upstream": args.vision_upstream,
        "vision_api_key": Args::mask_sensitive(&args.vision_api_key),
        "vision_model": args.vision_model,
        "vision_endpoint": args.vision_endpoint,
        "chinese_thinking": args.chinese_thinking,
        "codex_auto_inject": args.codex_auto_inject,
        "codex_persistent_inject": args.codex_persistent_inject,
        "data_dir": args.data_dir.to_string_lossy(),
        "prompts_dir": args.prompts_dir.to_string_lossy(),
        "token_anomaly_prompt_max": args.token_anomaly_prompt_max,
        "token_anomaly_spike_ratio": args.token_anomaly_spike_ratio,
        "token_anomaly_burn_window": args.token_anomaly_burn_window,
        "token_anomaly_burn_rate": args.token_anomaly_burn_rate,
        "allowed_mcp_servers": args.allowed_mcp_servers,
        "allowed_computer_displays": args.allowed_computer_displays,
        "computer_executor": args.computer_executor,
        "computer_executor_timeout_secs": args.computer_executor_timeout_secs,
        "mcp_executor_config": args.mcp_executor_config,
        "mcp_executor_timeout_secs": args.mcp_executor_timeout_secs,
        "playwright_state_dir": args.playwright_state_dir,
        "browser_use_bridge_url": args.browser_use_bridge_url,
        "browser_use_bridge_command": args.browser_use_bridge_command,
    }))
}

/// PUT /api/config — 更新并保存配置
pub async fn put_config(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let mut updated = load_args(&state.data_dir).unwrap_or_else(|| fallback_args(&state));

    if let Some(v) = body.get("port").and_then(|v| v.as_u64()) {
        updated.port = v as u16;
    }
    if let Some(v) = body.get("upstream").and_then(|v| v.as_str()) {
        updated.upstream = v.to_string();
    }
    if let Some(v) = body.get("api_key").and_then(|v| v.as_str()) {
        if v != Args::mask_sensitive(&updated.api_key) && v != "********" {
            updated.api_key = v.to_string();
        }
    }
    if let Some(v) = body.get("client_api_key").and_then(|v| v.as_str()) {
        if v != "********" {
            updated.client_api_key = v.to_string();
        }
    }
    if let Some(v) = body.get("model_map").and_then(|v| v.as_str()) {
        if serde_json::from_str::<Value>(v).is_ok() {
            updated.model_map = v.to_string();
        }
    }
    if let Some(v) = body.get("max_body_mb").and_then(|v| v.as_u64()) {
        updated.max_body_mb = v as usize;
    }
    if let Some(v) = body.get("vision_upstream").and_then(|v| v.as_str()) {
        updated.vision_upstream = v.to_string();
    }
    if let Some(v) = body.get("vision_api_key").and_then(|v| v.as_str()) {
        if v != Args::mask_sensitive(&updated.vision_api_key) && v != "********" {
            updated.vision_api_key = v.to_string();
        }
    }
    if let Some(v) = body.get("vision_model").and_then(|v| v.as_str()) {
        updated.vision_model = v.to_string();
    }
    if let Some(v) = body.get("vision_endpoint").and_then(|v| v.as_str()) {
        updated.vision_endpoint = v.to_string();
    }
    if let Some(v) = body.get("chinese_thinking").and_then(|v| v.as_bool()) {
        updated.chinese_thinking = v;
    }
    if let Some(v) = body.get("codex_auto_inject").and_then(|v| v.as_bool()) {
        updated.codex_auto_inject = v;
    }
    if let Some(v) = body
        .get("codex_persistent_inject")
        .and_then(|v| v.as_bool())
    {
        updated.codex_persistent_inject = v;
    }
    if let Some(v) = body.get("data_dir").and_then(|v| v.as_str()) {
        updated.data_dir = std::path::PathBuf::from(v);
    }
    if let Some(v) = body.get("prompts_dir").and_then(|v| v.as_str()) {
        updated.prompts_dir = std::path::PathBuf::from(v);
    }
    if let Some(v) = body
        .get("token_anomaly_prompt_max")
        .and_then(|v| v.as_u64())
    {
        updated.token_anomaly_prompt_max = v as u32;
    }
    if let Some(v) = body
        .get("token_anomaly_spike_ratio")
        .and_then(|v| v.as_f64())
    {
        updated.token_anomaly_spike_ratio = v;
    }
    if let Some(v) = body
        .get("token_anomaly_burn_window")
        .and_then(|v| v.as_u64())
    {
        updated.token_anomaly_burn_window = v;
    }
    if let Some(v) = body.get("token_anomaly_burn_rate").and_then(|v| v.as_u64()) {
        updated.token_anomaly_burn_rate = v as u32;
    }
    if let Some(v) = body.get("allowed_mcp_servers").and_then(|v| v.as_str()) {
        updated.allowed_mcp_servers = v.to_string();
    }
    if let Some(v) = body
        .get("allowed_computer_displays")
        .and_then(|v| v.as_str())
    {
        updated.allowed_computer_displays = v.to_string();
    }
    if let Some(v) = body.get("computer_executor").and_then(|v| v.as_str()) {
        updated.computer_executor = v.to_string();
    }
    if let Some(v) = body
        .get("computer_executor_timeout_secs")
        .and_then(|v| v.as_u64())
    {
        updated.computer_executor_timeout_secs = v;
    }
    if let Some(v) = body.get("mcp_executor_config").and_then(|v| v.as_str()) {
        updated.mcp_executor_config = v.to_string();
    }
    if let Some(v) = body
        .get("mcp_executor_timeout_secs")
        .and_then(|v| v.as_u64())
    {
        updated.mcp_executor_timeout_secs = v;
    }
    if let Some(v) = body.get("playwright_state_dir").and_then(|v| v.as_str()) {
        updated.playwright_state_dir = v.to_string();
    }
    if let Some(v) = body.get("browser_use_bridge_url").and_then(|v| v.as_str()) {
        updated.browser_use_bridge_url = v.to_string();
    }
    if let Some(v) = body
        .get("browser_use_bridge_command")
        .and_then(|v| v.as_str())
    {
        updated.browser_use_bridge_command = v.to_string();
    }

    let diags = validate::validate(&updated);
    let config_path = Args::default_config_path(&updated.data_dir);

    updated.save_to_file(&config_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": format!("保存配置失败: {}", e) })),
        )
    })?;

    // 同步关键配置到 .env，避免重启后 .env 占位符覆盖 config.json 的真值
    Args::sync_to_env_file(&updated.data_dir, "DEECODEX_API_KEY", &updated.api_key);
    Args::sync_to_env_file(&updated.data_dir, "DEECODEX_UPSTREAM", &updated.upstream);
    Args::sync_to_env_file(
        &updated.data_dir,
        "DEECODEX_PORT",
        &updated.port.to_string(),
    );
    Args::sync_to_env_file(
        &updated.data_dir,
        "DEECODEX_MODEL_MAP",
        &updated.model_map,
    );

    let saved_to = config_path.to_string_lossy().to_string();

    // Codex 配置注入/移除（根据更新后的开关立即执行）
    if updated.codex_auto_inject || updated.codex_persistent_inject {
        crate::codex_config::inject(updated.port, &updated.client_api_key);
    } else {
        crate::codex_config::remove();
    }

    // 运行时更新 executor 配置和 client_api_key（无需重启）
    match crate::executor::LocalExecutorConfig::from_raw(
        &updated.computer_executor,
        updated.computer_executor_timeout_secs,
        &updated.mcp_executor_config,
        updated.mcp_executor_timeout_secs,
    ) {
        Ok(exec) => *state.executors.write().await = exec,
        Err(e) => tracing::warn!("运行时更新 executor 配置失败: {e}"),
    }
    *state.client_api_key.write().await = updated.client_api_key;

    let diag_msgs: Vec<Value> = diags
        .iter()
        .map(|d| {
            json!({
                "severity": match d.severity { validate::Severity::Error => "error", validate::Severity::Warn => "warn" },
                "category": d.category,
                "message": d.message
            })
        })
        .collect();
    Ok(Json(
        json!({ "ok": true, "diagnostics": diag_msgs, "saved_to": saved_to }),
    ))
}

/// POST /api/config/validate — 仅诊断不保存
pub async fn validate_config(State(state): State<AppState>) -> impl IntoResponse {
    let args_to_check = load_args(&state.data_dir).unwrap_or_else(|| fallback_args(&state));
    let diags = validate::validate(&args_to_check);
    let diag_msgs: Vec<Value> = diags
        .iter()
        .map(|d| {
            json!({
                "severity": match d.severity { validate::Severity::Error => "error", validate::Severity::Warn => "warn" },
                "category": d.category,
                "message": d.message
            })
        })
        .collect();
    Json(json!({ "ok": true, "diagnostics": diag_msgs }))
}

/// GET /api/status — 服务运行状态
pub async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
    let uptime = state.start_time.elapsed().as_secs();
    let exec = state.executors.read().await;
    let client_auth_enabled = !state.client_api_key.read().await.is_empty();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime,
        "port": state.port,
        "upstream": state.upstream.as_str(),
        "vision_enabled": state.vision_upstream.is_some(),
        "mcp_enabled": exec.mcp.enabled(),
        "computer_executor": exec.computer.backend.as_str(),
        "chinese_thinking": state.chinese_thinking,
        "codex_auto_inject": state.codex_auto_inject,
        "codex_persistent_inject": state.codex_persistent_inject,
        "codex_launch_with_cdp": state.codex_launch_with_cdp,
        "cdp_port": state.cdp_port,
        "client_auth_enabled": client_auth_enabled,
    }))
}

/// POST /api/restart — 后台重启服务（1 秒延迟确保响应先返回）
pub async fn post_restart() -> impl IntoResponse {
    let result = spawn_mgmt_cmd("restart");
    match result {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok": true, "message": "正在重启，请稍后刷新页面"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": format!("无法执行重启: {}", e)})),
        ),
    }
}

/// POST /api/stop — 后台停止服务（1 秒延迟确保响应先返回）
pub async fn post_stop() -> impl IntoResponse {
    let result = spawn_mgmt_cmd("stop");
    match result {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok": true, "message": "服务正在停止"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": format!("无法执行停止: {}", e)})),
        ),
    }
}

/// 跨平台 spawn 管理命令（restart / stop），延迟 1 秒确保 HTTP 响应先返回
fn spawn_mgmt_cmd(action: &str) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .arg("/c")
            .arg(format!("timeout /t 1 /nobreak >nul & deecodex {}", action))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("sleep 1 && exec deecodex {}", action))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }
}

/// GET /api/logs — 返回结构化日志（最近 200 行）
pub async fn get_logs(State(state): State<AppState>) -> impl IntoResponse {
    let log_path = state.data_dir.join("deecodex.log");
    match std::fs::read_to_string(&log_path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
            let start = if lines.len() > 200 { lines.len() - 200 } else { 0 };
            let entries: Vec<Value> = lines[start..].iter().map(|l| parse_log_line(l)).collect();
            Json(json!({"ok": true, "entries": entries}))
        }
        Err(e) => Json(json!({"ok": false, "error": format!("无法读取日志: {}", e), "entries": []})),
    }
}

/// 解析单行日志：去 ANSI，提取 timestamp / level / target / message / fields
/// 日志格式: "2026-05-07T17:26:48.845500Z  INFO deecodex::handlers: message key=val..."
fn parse_log_line(raw: &str) -> Value {
    let clean = strip_ansi(raw);
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return json!({"level": "unknown", "time": "", "message": raw});
    }

    // ISO 8601 时间戳固定 27 字符: "2026-05-07T17:26:48.845500Z "
    let timestamp = if trimmed.len() >= 27 && trimmed.as_bytes()[4] == b'-' {
        trimmed[..27].to_string()
    } else {
        return json!({"level": "unknown", "time": "", "message": raw});
    };

    let time_short = timestamp[11..19].to_string();
    let rest = trimmed[27..].trim();

    // rest: "LEVEL target: message key=val..."
    let mut parts = rest.splitn(2, |c: char| c.is_whitespace());
    let level = parts.next().unwrap_or("UNKNOWN").to_uppercase();
    let after_level = parts.next().unwrap_or("").trim();

    // after_level: "target: message key=val..." 或 "message"
    let (target, body) = if let Some(pos) = after_level.find(": ") {
        let t = after_level[..pos].to_string();
        let b = after_level[pos + 2..].to_string();
        (t, b)
    } else if let Some(pos) = after_level.find(':') {
        let t = after_level[..pos].to_string();
        let b = after_level[pos + 1..].to_string();
        (t, b)
    } else {
        (String::new(), after_level.to_string())
    };

    let (message, fields) = extract_fields(&body);

    json!({
        "time": time_short,
        "timestamp": timestamp,
        "level": level.to_lowercase(),
        "target": target,
        "message": message.trim().to_string(),
        "fields": fields,
    })
}

fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next(); // consume '['
            while let Some(&c) = chars.peek() {
                chars.next();
                if c == 'm' {
                    break;
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn extract_fields(body: &str) -> (String, Value) {
    let mut fields = serde_json::Map::new();
    let words: Vec<&str> = body.split_whitespace().collect();
    let mut msg_end = words.len();

    for i in (0..words.len()).rev() {
        if words[i].contains('=') {
            let kv: Vec<&str> = words[i].splitn(2, '=').collect();
            if kv.len() == 2 {
                let key = kv[0].to_string();
                let val = kv[1].trim_matches('"').to_string();
                fields.insert(key, Value::String(val));
                msg_end = i;
            }
        } else {
            break;
        }
    }

    let message = words[..msg_end].join(" ");
    (message, Value::Object(fields))
}

/// POST /api/update — 下载最新版本并重启
pub async fn post_update(State(state): State<AppState>) -> impl IntoResponse {
    let script_name = if cfg!(windows) {
        "deecodex.bat"
    } else {
        "deecodex.sh"
    };
    let script = state.data_dir.join(script_name);
    if !script.exists() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": format!("管理脚本 {} 不存在，请重新运行安装脚本", script_name)})),
        );
    }
    let result = spawn_update_cmd(&script);
    match result {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({"ok": true, "message": "正在升级，完成后将自动重启"})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"ok": false, "error": format!("无法执行升级: {}", e)})),
        ),
    }
}

fn spawn_update_cmd(script: &std::path::Path) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .arg("/c")
            .arg(format!(
                "timeout /t 1 /nobreak >nul & \"{}\" update",
                script.display()
            ))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "sleep 1 && exec sh {} update",
                script.display()
            ))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }
}

// ── 会话管理 ──────────────────────────────────────────────────

/// GET /api/sessions — 列出所有活跃的响应和对话
async fn handle_list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let responses = state.sessions.list_responses();
    let conversations = state.sessions.list_conversations();
    Json(json!({
        "responses": responses.iter().map(|r| json!({"id": r.id, "status": r.status})).collect::<Vec<_>>(),
        "conversations": conversations.iter().map(|c| json!({"id": c.id, "message_count": c.message_count})).collect::<Vec<_>>(),
    }))
}

/// POST /api/sessions/undo — 根据备份 token 撤销删除
async fn handle_undo_delete(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let token = body.get("undo_token").and_then(|v| v.as_str()).ok_or_else(|| {
        (StatusCode::BAD_REQUEST, Json(json!({"error": "缺少 undo_token"})))
    })?;

    let backup_store = BackupStore::new(state.data_dir.join("backups")).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("备份存储初始化失败: {}", e)})))
    })?;
    let backup = backup_store.read_backup(token).map_err(|e| {
        (StatusCode::NOT_FOUND, Json(json!({"error": format!("备份未找到: {}", e)})))
    })?;

    let session_type = backup.get("session_type").and_then(|v| v.as_str()).unwrap_or("");
    let data = &backup["data"];

    match session_type {
        "response" => {
            let response_id = backup["session_id"].as_str().unwrap_or("");
            let messages: Vec<ChatMessage> = serde_json::from_value(data["messages"].clone()).map_err(|e| {
                (StatusCode::BAD_REQUEST, Json(json!({"error": format!("备份数据损坏: {}", e)})))
            })?;
            let response = data["response"].clone();
            let input_items: Vec<Value> = serde_json::from_value(data["input_items"].clone()).unwrap_or_default();
            state.sessions.undo_delete_response(response_id, messages, response, input_items);
        }
        "conversation" => {
            let conversation_id = backup["session_id"].as_str().unwrap_or("");
            let messages: Vec<ChatMessage> = serde_json::from_value(data["messages"].clone()).map_err(|e| {
                (StatusCode::BAD_REQUEST, Json(json!({"error": format!("备份数据损坏: {}", e)})))
            })?;
            let items: Vec<Value> = serde_json::from_value(data["items"].clone()).unwrap_or_default();
            state.sessions.undo_delete_conversation(conversation_id, messages, items);
        }
        _ => {
            return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "未知的会话类型"}))));
        }
    }

    let _ = backup_store.delete_backup(token);
    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/sessions/responses/:response_id — 删除响应（先备份）
async fn handle_delete_response_with_backup(
    State(state): State<AppState>,
    Path(response_id): Path<String>,
) -> impl IntoResponse {
    if let Some((messages, response, input_items)) = state.sessions.delete_response_with_data(&response_id) {
        let backup_store = match BackupStore::new(state.data_dir.join("backups")) {
            Ok(store) => store,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "备份存储初始化失败"}))).into_response(),
        };
        let data = json!({
            "messages": messages,
            "response": response,
            "input_items": input_items,
        });
        let token = backup_store.write_backup(&response_id, "response", &data).unwrap_or_default();
        Json(json!({
            "id": response_id,
            "object": "response.deleted",
            "deleted": true,
            "undo_token": token,
        }))
        .into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": format!("No response found with id {}", response_id)}))).into_response()
    }
}

/// DELETE /api/sessions/conversations/:conversation_id — 删除对话（先备份）
async fn handle_delete_conversation_with_backup(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
) -> impl IntoResponse {
    if let Some((messages, items)) = state.sessions.delete_conversation_with_data(&conversation_id) {
        let backup_store = match BackupStore::new(state.data_dir.join("backups")) {
            Ok(store) => store,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": "备份存储初始化失败"}))).into_response(),
        };
        let data = json!({
            "messages": messages,
            "items": items,
        });
        let token = backup_store.write_backup(&conversation_id, "conversation", &data).unwrap_or_default();
        Json(json!({
            "id": conversation_id,
            "object": "conversation.deleted",
            "deleted": true,
            "undo_token": token,
        }))
        .into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(json!({"error": format!("No conversation found with id {}", conversation_id)}))).into_response()
    }
}

// ── 工具策略 ──────────────────────────────────────────────────
// handle_get_tool_policy / handle_put_tool_policy 已移至 handlers.rs，路由注册在上方 build_web_router() 中
