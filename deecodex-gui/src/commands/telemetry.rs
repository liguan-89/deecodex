use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::State;

use crate::ServerManager;

const TELEMETRY_SETTINGS_FILE: &str = "telemetry.json";
const TELEMETRY_EVENTS_FILE: &str = "telemetry-events.jsonl";
const TELEMETRY_FAILED_FILE: &str = "telemetry-failed.jsonl";
const DEFAULT_TELEMETRY_ENDPOINT: &str = "https://api.liguan.me/telemetry/v1/events";
const LEGACY_TELEMETRY_ENDPOINTS: &[&str] = &["https://api.dexai.vip/telemetry/v1/events"];
const MAX_PROP_STRING_LEN: usize = 256;
const MAX_EVENT_NAME_LEN: usize = 64;
const MAX_OBJECT_KEYS: usize = 64;

static SESSION_ID: OnceLock<String> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySettings {
    #[serde(default = "default_telemetry_enabled")]
    pub enabled: bool,
    #[serde(default = "default_telemetry_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub install_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryPreferences {
    pub enabled: bool,
    pub endpoint: String,
    pub install_id: String,
    pub session_id: String,
    pub app_version: String,
    pub os: String,
    pub privacy_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TelemetryTrackResult {
    pub accepted: bool,
    pub queued: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TelemetryEventEnvelope {
    event: String,
    ts: u64,
    install_id: String,
    session_id: String,
    app_version: String,
    os: String,
    arch: String,
    props: Value,
}

fn default_telemetry_enabled() -> bool {
    true
}

fn default_telemetry_endpoint() -> String {
    std::env::var("DEX_AI_TELEMETRY_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_TELEMETRY_ENDPOINT.to_string())
}

fn telemetry_forced_disabled() -> bool {
    std::env::var("DEX_AI_TELEMETRY_DISABLED")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

fn session_id() -> String {
    SESSION_ID
        .get_or_init(|| format!("session_{}", deecodex::accounts::generate_id()))
        .clone()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TELEMETRY_SETTINGS_FILE)
}

fn events_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TELEMETRY_EVENTS_FILE)
}

fn failed_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TELEMETRY_FAILED_FILE)
}

fn ensure_install_id(settings: &mut TelemetrySettings) -> bool {
    if !settings.install_id.trim().is_empty() {
        return false;
    }
    settings.install_id = format!("install_{}", deecodex::accounts::generate_id());
    true
}

fn load_settings(data_dir: &Path) -> TelemetrySettings {
    let path = settings_path(data_dir);
    let mut settings = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<TelemetrySettings>(&content).ok())
        .unwrap_or_else(|| TelemetrySettings {
            enabled: default_telemetry_enabled(),
            endpoint: default_telemetry_endpoint(),
            install_id: String::new(),
        });
    let endpoint = settings.endpoint.trim();
    if endpoint.is_empty() || LEGACY_TELEMETRY_ENDPOINTS.contains(&endpoint) {
        settings.endpoint = default_telemetry_endpoint();
    }
    if telemetry_forced_disabled() {
        settings.enabled = false;
    }
    let changed = ensure_install_id(&mut settings);
    if changed {
        if let Err(err) = save_settings(data_dir, &settings) {
            tracing::warn!(%err, "保存 telemetry install_id 失败");
        }
    }
    settings
}

fn save_settings(data_dir: &Path, settings: &TelemetrySettings) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let content =
        serde_json::to_string_pretty(settings).map_err(|e| format!("序列化统计设置失败: {e}"))?;
    std::fs::write(settings_path(data_dir), content).map_err(|e| format!("保存统计设置失败: {e}"))
}

fn sanitize_event_name(event: &str) -> Option<String> {
    let cleaned: String = event
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        .take(MAX_EVENT_NAME_LEN)
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    [
        "key",
        "token",
        "secret",
        "password",
        "authorization",
        "cookie",
        "prompt",
        "message",
        "content",
        "thread_content",
        "email",
        "path",
        "file",
        "dir",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn sensitive_string(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with("sk-")
        || trimmed.starts_with("sess-")
        || trimmed.starts_with("eyJ")
        || trimmed.len() > 80
            && trimmed
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .count()
                > 70
}

fn sanitize_value(value: Value, depth: usize) -> Value {
    if depth > 4 {
        return Value::Null;
    }
    match value {
        Value::Null | Value::Bool(_) => value,
        Value::Number(_) => value,
        Value::String(text) => {
            if sensitive_string(&text) {
                Value::String("[REDACTED]".into())
            } else if text.chars().count() > MAX_PROP_STRING_LEN {
                Value::String(text.chars().take(MAX_PROP_STRING_LEN).collect())
            } else {
                Value::String(text)
            }
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .take(MAX_OBJECT_KEYS)
                .map(|item| sanitize_value(item, depth + 1))
                .collect(),
        ),
        Value::Object(object) => {
            let mut sanitized = Map::new();
            for (key, value) in object.into_iter().take(MAX_OBJECT_KEYS) {
                if sensitive_key(&key) {
                    sanitized.insert(key, Value::String("[REDACTED]".into()));
                } else {
                    sanitized.insert(key, sanitize_value(value, depth + 1));
                }
            }
            Value::Object(sanitized)
        }
    }
}

fn append_jsonl(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建事件目录失败: {e}"))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("打开事件文件失败: {e}"))?;
    writeln!(file, "{value}").map_err(|e| format!("写入事件文件失败: {e}"))
}

fn make_preferences(settings: TelemetrySettings) -> TelemetryPreferences {
    TelemetryPreferences {
        enabled: settings.enabled,
        endpoint: settings.endpoint,
        install_id: settings.install_id,
        session_id: session_id(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        privacy_note: "只采集匿名使用统计、页面 PV/UV 和广告位曝光/点击；不采集 API Key、Prompt、线程内容、账号邮箱和本地路径。".into(),
    }
}

async fn post_event(endpoint: String, event: TelemetryEventEnvelope, data_dir: PathBuf) {
    let payload = json!({ "events": [event] });
    let result = reqwest::Client::new()
        .post(endpoint)
        .header("content-type", "application/json")
        .header(
            "user-agent",
            format!("DEX-AI/{}", env!("CARGO_PKG_VERSION")),
        )
        .body(payload.to_string())
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => {
            let failed = json!({ "ts": now_secs(), "status": response.status().as_u16(), "payload": payload });
            let _ = append_jsonl(&failed_path(&data_dir), &failed);
        }
        Err(err) => {
            let failed = json!({ "ts": now_secs(), "error": err.to_string(), "payload": payload });
            let _ = append_jsonl(&failed_path(&data_dir), &failed);
        }
    }
}

#[tauri::command]
pub async fn get_telemetry_preferences(
    manager: State<'_, ServerManager>,
) -> Result<TelemetryPreferences, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    Ok(make_preferences(load_settings(&data_dir)))
}

#[tauri::command]
pub async fn set_telemetry_preferences(
    manager: State<'_, ServerManager>,
    enabled: bool,
) -> Result<TelemetryPreferences, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut settings = load_settings(&data_dir);
    settings.enabled = enabled && !telemetry_forced_disabled();
    ensure_install_id(&mut settings);
    save_settings(&data_dir, &settings)?;
    Ok(make_preferences(settings))
}

#[tauri::command]
pub async fn track_telemetry_event(
    manager: State<'_, ServerManager>,
    event: String,
    props: Option<Value>,
) -> Result<TelemetryTrackResult, String> {
    let Some(event) = sanitize_event_name(&event) else {
        return Ok(TelemetryTrackResult {
            accepted: false,
            queued: false,
            reason: Some("事件名为空".into()),
        });
    };
    let data_dir = manager.data_dir.lock().await.clone();
    let settings = load_settings(&data_dir);
    if !settings.enabled {
        return Ok(TelemetryTrackResult {
            accepted: false,
            queued: false,
            reason: Some("匿名统计已关闭".into()),
        });
    }

    let envelope = TelemetryEventEnvelope {
        event,
        ts: now_secs(),
        install_id: settings.install_id.clone(),
        session_id: session_id(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        props: sanitize_value(props.unwrap_or_else(|| json!({})), 0),
    };
    let event_value =
        serde_json::to_value(&envelope).map_err(|e| format!("序列化统计事件失败: {e}"))?;
    append_jsonl(&events_path(&data_dir), &event_value)?;

    let endpoint = settings.endpoint.clone();
    tauri::async_runtime::spawn(post_event(endpoint, envelope, data_dir));

    Ok(TelemetryTrackResult {
        accepted: true,
        queued: true,
        reason: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{sanitize_event_name, sanitize_value};
    use serde_json::json;

    #[test]
    fn telemetry_event_name_keeps_safe_chars_only() {
        assert_eq!(sanitize_event_name(" ad.click! "), Some("ad.click".into()));
        assert_eq!(sanitize_event_name("  !!! "), None);
    }

    #[test]
    fn telemetry_props_are_redacted() {
        let value = sanitize_value(
            json!({
                "page": "status",
                "api_key": "sk-secret",
                "prompt": "hello",
                "nested": { "thread_content": "private" }
            }),
            0,
        );
        assert_eq!(value["page"], "status");
        assert_eq!(value["api_key"], "[REDACTED]");
        assert_eq!(value["prompt"], "[REDACTED]");
        assert_eq!(value["nested"]["thread_content"], "[REDACTED]");
    }
}
