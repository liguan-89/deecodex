use serde::{Deserialize, Serialize};

use crate::manifest::{DexToolManifest, PluginAccountManifest, PluginFeatureManifest};

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
/// 插件 → deecodex：读取插件长期数据
pub const METHOD_ASSETS_READ: &str = "assets.read";
/// 插件 → deecodex：写入插件长期数据
pub const METHOD_ASSETS_WRITE: &str = "assets.write";
/// 插件 → deecodex：列出插件长期数据
pub const METHOD_ASSETS_LIST: &str = "assets.list";
/// 插件 → deecodex：删除插件长期数据
pub const METHOD_ASSETS_DELETE: &str = "assets.delete";
/// 插件 → deecodex：读取插件缓存
pub const METHOD_CACHE_READ: &str = "cache.read";
/// 插件 → deecodex：写入插件缓存
pub const METHOD_CACHE_WRITE: &str = "cache.write";
/// 插件 → deecodex：清空插件缓存
pub const METHOD_CACHE_CLEAR: &str = "cache.clear";
/// 插件 → deecodex：写入插件密钥
pub const METHOD_SECRETS_SET: &str = "secrets.set";
/// 插件 → deecodex：读取插件密钥
pub const METHOD_SECRETS_GET: &str = "secrets.get";
/// 插件 → deecodex：删除插件密钥
pub const METHOD_SECRETS_DELETE: &str = "secrets.delete";

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
    pub kind: String,
    pub tags: Vec<String>,
    pub features: Vec<PluginFeatureManifest>,
    pub state: PluginState,
    pub enabled: bool,
    pub accounts: Vec<AccountInfo>,
    pub account: Option<PluginAccountManifest>,
    pub permissions: Vec<String>,
    pub permission_risk: String,
    pub permission_details: Vec<PluginPermissionInfo>,
    pub installed_at: u64,
    pub source_path: String,
    pub source_hash: String,
    pub config: serde_json::Value,
    pub config_schema: Option<serde_json::Value>,
    pub dex_tools: Vec<DexToolManifest>,
    pub assets: PluginAssetInfo,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginInstallPreview {
    pub manifest: crate::manifest::PluginManifest,
    pub already_installed: bool,
    pub existing_version: Option<String>,
    pub previous_source_hash: Option<String>,
    pub install_dir: String,
    pub asset_dir: String,
    pub source_path: String,
    pub source_hash: String,
    pub permission_risk: String,
    pub permission_details: Vec<PluginPermissionInfo>,
    pub permission_changes: Vec<PluginPermissionChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginAssetPaths {
    pub install_dir: String,
    pub data_dir: String,
    pub cache_dir: String,
    pub secrets_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginAssetInfo {
    pub paths: PluginAssetPaths,
    pub data_bytes: u64,
    pub cache_bytes: u64,
    pub secrets_bytes: u64,
    pub total_bytes: u64,
    pub secret_count: usize,
    pub account_count: usize,
    pub lifecycle: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPermissionInfo {
    pub permission: String,
    pub risk: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginPermissionChange {
    pub permission: String,
    pub change: String,
    pub risk: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountInfo {
    pub account_id: String,
    pub name: String,
    pub status: AccountStatus,
    pub last_active_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginEventRecord {
    pub seq: u64,
    pub ts: u64,
    pub plugin_id: String,
    pub event: PluginEvent,
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
    #[serde(rename = "asset_operation")]
    AssetOperation {
        plugin_id: String,
        scope: String,
        action: String,
        path: String,
        ok: bool,
    },
}

impl PluginEvent {
    pub fn plugin_id(&self) -> &str {
        match self {
            PluginEvent::Log { plugin_id, .. }
            | PluginEvent::StatusChanged { plugin_id, .. }
            | PluginEvent::QrCode { plugin_id, .. }
            | PluginEvent::Error { plugin_id, .. }
            | PluginEvent::AssetOperation { plugin_id, .. } => plugin_id,
        }
    }
}
