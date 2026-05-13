use serde::{Deserialize, Serialize};

// ── RPC method 名称常量 ──────────────────────────────────────────────────────

/// deecodex → 插件：握手初始化
pub const METHOD_INITIALIZE: &str = "initialize";
/// 插件 → deecodex：初始化完成
pub const METHOD_INITIALIZED: &str = "initialized";
/// deecodex → 插件：即将关闭
pub const METHOD_SHUTDOWN: &str = "shutdown";
/// deecodex → 插件：配置更新
pub const METHOD_CONFIG_UPDATE: &str = "config.update";

/// 插件 → deecodex：调用 LLM
pub const METHOD_LLM_CALL: &str = "llm.call";
/// deecodex → 插件：LLM 流式 chunk
pub const METHOD_LLM_STREAM_CHUNK: &str = "llm.stream_chunk";
/// 插件 → deecodex：取消 LLM 调用
pub const METHOD_LLM_CANCEL: &str = "llm.cancel";

/// 插件 → deecodex：下载媒体
pub const METHOD_MEDIA_DOWNLOAD: &str = "media.download";
/// 插件 → deecodex：上传媒体
pub const METHOD_MEDIA_UPLOAD: &str = "media.upload";

/// 插件 → deecodex：日志
pub const METHOD_LOG: &str = "log";
/// 插件 → deecodex：状态变更
pub const METHOD_STATUS: &str = "status";
/// 插件 → deecodex：QR 码数据
pub const METHOD_QR_CODE: &str = "qr_code";

// ── 公开类型 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginState {
    Installed,
    Starting,
    Running,
    Stopped,
    Error,
}

impl PluginState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PluginState::Installed => "installed",
            PluginState::Starting => "starting",
            PluginState::Running => "running",
            PluginState::Stopped => "stopped",
            PluginState::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Disconnected,
    Connecting,
    Connected,
    LoginExpired,
    Error,
}

impl AccountStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountStatus::Disconnected => "disconnected",
            AccountStatus::Connecting => "connecting",
            AccountStatus::Connected => "connected",
            AccountStatus::LoginExpired => "login_expired",
            AccountStatus::Error => "error",
        }
    }
}

/// 插件信息（供前端展示）
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub state: PluginState,
    pub accounts: Vec<AccountInfo>,
    pub permissions: Vec<String>,
    pub installed_at: u64,
    pub config: serde_json::Value,
    pub config_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountInfo {
    pub account_id: String,
    pub name: String,
    pub status: AccountStatus,
    pub last_active_at: Option<u64>,
}

/// 插件事件（通过 broadcast channel 发布）
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum PluginEvent {
    #[serde(rename = "log")]
    Log {
        plugin_id: String,
        level: String,
        message: String,
    },
    #[serde(rename = "status_changed")]
    StatusChanged {
        plugin_id: String,
        account_id: String,
        status: AccountStatus,
    },
    #[serde(rename = "qr_code")]
    QrCode {
        plugin_id: String,
        account_id: String,
        data_url: String,
    },
    #[serde(rename = "error")]
    Error { plugin_id: String, message: String },
}
