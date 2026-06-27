use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::providers::{
    get_provider_profiles, provider_options_for_slug, AuthScheme, ModelDiscovery,
    ProviderCapabilities, WireProtocol,
};

// ── 数据模型 ────────────────────────────────────────────────────────────────

pub const ACCOUNT_STORE_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountAuthMode {
    #[default]
    ApiKey,
    #[serde(rename = "oauth", alias = "o_auth")]
    OAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountClientKind {
    #[default]
    Codex,
    ClaudeCode,
    Openclaw,
    Hermes,
    GenericClient,
}

impl AccountClientKind {
    pub fn is_codex(&self) -> bool {
        matches!(self, Self::Codex)
    }

    pub fn supports_desktop_surface(&self) -> bool {
        matches!(self, Self::Codex | Self::ClaudeCode)
    }

    pub fn active_surfaces(&self) -> Vec<AccountClientSurface> {
        if self.supports_desktop_surface() {
            vec![AccountClientSurface::Cli, AccountClientSurface::Desktop]
        } else {
            vec![AccountClientSurface::Cli]
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountClientSurface {
    #[default]
    Cli,
    Desktop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SurfaceActiveSelection {
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub endpoint_id: Option<String>,
}

pub fn account_client_kind_key(kind: &AccountClientKind) -> &'static str {
    match kind {
        AccountClientKind::Codex => "codex",
        AccountClientKind::ClaudeCode => "claude_code",
        AccountClientKind::Openclaw => "openclaw",
        AccountClientKind::Hermes => "hermes",
        AccountClientKind::GenericClient => "generic_client",
    }
}

pub fn account_client_surface_key(surface: &AccountClientSurface) -> &'static str {
    match surface {
        AccountClientSurface::Cli => "cli",
        AccountClientSurface::Desktop => "desktop",
    }
}

pub fn surface_active_key(kind: &AccountClientKind, surface: &AccountClientSurface) -> String {
    format!(
        "{}:{}",
        account_client_kind_key(kind),
        account_client_surface_key(surface)
    )
}

pub const DEX_ASSISTANT_ACTIVE_KEY: &str = "dex:assistant";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClientCheckRecord {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub checked_at: u64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountRuntimeStatus {
    #[default]
    Active,
    Error,
    CoolingDown,
    QuotaExceeded,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountQuotaState {
    #[serde(default)]
    pub exceeded: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub next_recover_at: Option<u64>,
    #[serde(default)]
    pub backoff_level: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountModelRuntimeState {
    #[serde(default)]
    pub status: AccountRuntimeStatus,
    #[serde(default)]
    pub status_message: String,
    #[serde(default)]
    pub next_retry_after: Option<u64>,
    #[serde(default)]
    pub quota: AccountQuotaState,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountRecentRequestBucket {
    #[serde(default)]
    pub bucket_start: u64,
    #[serde(default)]
    pub success: u64,
    #[serde(default)]
    pub failed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountRuntimeState {
    #[serde(default)]
    pub status: AccountRuntimeStatus,
    #[serde(default)]
    pub status_message: String,
    #[serde(default)]
    pub next_retry_after: Option<u64>,
    #[serde(default)]
    pub quota: AccountQuotaState,
    #[serde(default)]
    pub model_states: HashMap<String, AccountModelRuntimeState>,
    #[serde(default)]
    pub success: u64,
    #[serde(default)]
    pub failed: u64,
    #[serde(default)]
    pub recent_requests: Vec<AccountRecentRequestBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountRoutingOptions {
    #[serde(default = "default_routing_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_enabled: Option<bool>,
    #[serde(default = "default_routing_pool")]
    pub pool: String,
    #[serde(default)]
    pub priority: i64,
    #[serde(default = "default_routing_weight")]
    pub weight: u32,
    #[serde(default = "default_native_computer_policy")]
    pub native_computer_policy: String,
    #[serde(default)]
    pub disabled: bool,
}

fn default_routing_enabled() -> bool {
    true
}

fn default_routing_pool() -> String {
    "codex-official".into()
}

fn default_routing_weight() -> u32 {
    1
}

fn default_native_computer_policy() -> String {
    "helper_required".into()
}

impl Default for AccountRoutingOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            anchor_enabled: None,
            execution_enabled: None,
            pool: default_routing_pool(),
            priority: 0,
            weight: 1,
            native_computer_policy: default_native_computer_policy(),
            disabled: false,
        }
    }
}

impl AccountRoutingOptions {
    pub fn effective_enabled(&self) -> bool {
        self.enabled && !self.disabled && self.weight > 0
    }

    pub fn anchor_enabled_for_account(&self, account: &Account) -> bool {
        self.anchor_enabled
            .unwrap_or_else(|| account.is_codex_official_account())
    }

    pub fn execution_enabled_for_account(&self, account: &Account) -> bool {
        self.execution_enabled
            .unwrap_or_else(|| !account.is_codex_official_account())
    }

    pub fn effective_anchor_enabled_for_account(&self, account: &Account) -> bool {
        self.enabled && !self.disabled && self.anchor_enabled_for_account(account)
    }

    pub fn effective_execution_enabled_for_account(&self, account: &Account) -> bool {
        self.enabled
            && !self.disabled
            && self.weight > 0
            && self.execution_enabled_for_account(account)
    }

    pub fn normalized(mut self) -> Self {
        if self.pool.trim().is_empty() {
            self.pool = default_routing_pool();
        } else {
            self.pool = self.pool.trim().to_string();
        }
        self.weight = self.weight.clamp(1, 100);
        self.native_computer_policy = match self.native_computer_policy.trim() {
            "strip_and_continue" => "strip_and_continue".into(),
            _ => default_native_computer_policy(),
        };
        self
    }

    pub fn strip_native_computer_toolchain(&self) -> bool {
        self.native_computer_policy == "strip_and_continue"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    #[default]
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    CodexOfficial,
    CustomChat,
    CustomResponses,
}

impl EndpointKind {
    pub fn is_chat_like(&self) -> bool {
        matches!(self, Self::OpenAiChat | Self::CustomChat)
    }

    pub fn is_responses_like(&self) -> bool {
        matches!(self, Self::OpenAiResponses | Self::CustomResponses)
    }

    pub fn default_path(&self) -> &'static str {
        match self {
            Self::OpenAiChat | Self::CustomChat => "chat/completions",
            Self::OpenAiResponses | Self::CustomResponses | Self::CodexOfficial => "responses",
            Self::AnthropicMessages => "messages",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "OpenAI Chat",
            Self::OpenAiResponses => "OpenAI Responses",
            Self::AnthropicMessages => "Anthropic Messages",
            Self::CodexOfficial => "Codex 官方",
            Self::CustomChat => "自定义 Chat",
            Self::CustomResponses => "自定义 Responses",
        }
    }
}

fn normalize_responses_base_url(url: &str) -> String {
    let mut normalized = url.trim().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/responses"] {
        if normalized
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase())
        {
            let new_len = normalized.len().saturating_sub(suffix.len());
            normalized.truncate(new_len);
            normalized = normalized.trim_end_matches('/').to_string();
            break;
        }
    }
    normalized
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisionMode {
    #[default]
    Off,
    Native,
    Glue,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelVisionMode {
    #[default]
    Inherit,
    Off,
    Native,
    Glue,
}

impl ModelVisionMode {
    pub fn resolve(&self, inherited: &VisionMode) -> VisionMode {
        match self {
            Self::Inherit => inherited.clone(),
            Self::Off => VisionMode::Off,
            Self::Native => VisionMode::Native,
            Self::Glue => VisionMode::Glue,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedImagePolicy {
    Reject,
    #[default]
    StripWithWarning,
    OcrThenStrip,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GlueVisionStrategy {
    #[default]
    FinalAnswer,
    CaptionThenMain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionConfig {
    #[serde(default)]
    pub mode: VisionMode,
    #[serde(default)]
    pub unsupported_image_policy: UnsupportedImagePolicy,
    #[serde(default)]
    pub glue_strategy: GlueVisionStrategy,
    #[serde(default = "default_minimax_adapter")]
    pub adapter_id: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_minimax_vlm_path")]
    pub path: String,
}

fn default_minimax_adapter() -> String {
    "minimax_coding_plan_vlm".into()
}

fn default_minimax_vlm_path() -> String {
    "v1/coding_plan/vlm".into()
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            mode: VisionMode::Off,
            unsupported_image_policy: UnsupportedImagePolicy::StripWithWarning,
            glue_strategy: GlueVisionStrategy::FinalAnswer,
            adapter_id: default_minimax_adapter(),
            base_url: String::new(),
            api_key: String::new(),
            model: String::new(),
            path: default_minimax_vlm_path(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelProfile {
    #[serde(default)]
    pub vision_mode: ModelVisionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: EndpointKind,
    pub base_url: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub template_id: String,
    #[serde(default = "default_template_version")]
    pub template_version: u32,
    #[serde(default)]
    pub model_map: HashMap<String, String>,
    /// Codex 模型直选使用的上游模型列表。旧 model_map 的 value 只迁移到这里一次。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_models: Vec<String>,
    #[serde(default)]
    pub model_profiles: HashMap<String, ModelProfile>,
    #[serde(default)]
    pub vision: VisionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_generation_enabled: Option<bool>,
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
    #[serde(default)]
    pub request_timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
    #[serde(default)]
    pub context_window_override: Option<u32>,
    #[serde(default)]
    pub reasoning_effort_override: Option<String>,
    #[serde(default)]
    pub thinking_tokens: Option<u32>,
    /// Fast 模式服务层。开启后仅注入 service_tier，不降低 reasoning / thinking。
    #[serde(default)]
    pub fast_mode_enabled: bool,
    #[serde(default = "default_fast_service_tier")]
    pub fast_service_tier: String,
    #[serde(default)]
    pub balance_url: String,
}

fn default_template_version() -> u32 {
    1
}

fn default_fast_service_tier() -> String {
    "priority".into()
}

fn default_image_generation_enabled_for(provider: &str, kind: &EndpointKind) -> bool {
    matches!(kind, EndpointKind::CodexOfficial)
        || (matches!(kind, EndpointKind::OpenAiResponses)
            && provider.trim().eq_ignore_ascii_case("openai"))
}

fn legacy_model_map_values(model_map: &HashMap<String, String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut values = Vec::new();
    for model in model_map.values() {
        let model = model.trim();
        if model.is_empty() || !seen.insert(model.to_string()) {
            continue;
        }
        values.push(model.to_string());
    }
    values
}

fn append_known_models(known_models: &mut Vec<String>, models: impl IntoIterator<Item = String>) {
    let mut seen = known_models
        .iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect::<HashSet<_>>();
    known_models.retain(|model| !model.trim().is_empty());
    for model in models {
        let model = model.trim().to_string();
        if model.is_empty() || !seen.insert(model.clone()) {
            continue;
        }
        known_models.push(model);
    }
}

impl EndpointConfig {
    pub fn effective_path(&self) -> &str {
        if self.path.trim().is_empty() {
            self.kind.default_path()
        } else {
            self.path.trim()
        }
    }

    pub fn model_vision_mode(&self, model: &str) -> VisionMode {
        self.model_profiles
            .get(model)
            .map(|profile| profile.vision_mode.resolve(&self.vision.mode))
            .unwrap_or_else(|| self.vision.mode.clone())
    }

    pub fn effective_image_generation_enabled(&self, account: &Account) -> bool {
        self.image_generation_enabled
            .unwrap_or_else(|| default_image_generation_enabled_for(&account.provider, &self.kind))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub provider: String,
    /// 账号面向的 AI 客户端。Codex 账号走 deecodex 代理；其他客户端只管理本地配置。
    #[serde(default, alias = "target")]
    pub client_kind: AccountClientKind,
    /// 同一客户端家族下的使用形态。仅 Codex/Claude 区分 CLI 与桌面版，其他客户端固定为 CLI。
    #[serde(default, alias = "surface")]
    pub client_surface: AccountClientSurface,
    #[serde(default)]
    pub wire_protocol: WireProtocol,
    pub upstream: String,
    pub api_key: String,
    #[serde(default)]
    pub auth_mode: AccountAuthMode,
    /// 非 Codex 客户端直接使用的默认模型名。Codex 账号使用模型直选，旧 model_map 仅保留解析兼容。
    #[serde(default)]
    pub default_model: String,
    /// 客户端侧扩展配置，例如 env 覆盖、profile 名称、配置路径等。
    #[serde(default)]
    pub client_options: HashMap<String, Value>,
    /// 账号运行态：最近错误、配额冷却、单模型可用性等。
    #[serde(default)]
    pub runtime_state: AccountRuntimeState,
    /// 外部客户端配置最近一次成功写入时间。
    #[serde(default)]
    pub last_applied_at: Option<u64>,
    /// 外部客户端最近一次检查结果。
    #[serde(default)]
    pub last_check: Option<ClientCheckRecord>,
    #[serde(default)]
    pub model_map: HashMap<String, String>,
    #[serde(default)]
    pub vision_upstream: String,
    #[serde(default)]
    pub vision_api_key: String,
    #[serde(default)]
    pub vision_model: String,
    #[serde(default)]
    pub vision_endpoint: String,
    /// 是否启用多模态视觉路由，勾选后显式路由图片请求至视觉模型
    #[serde(default)]
    pub vision_enabled: bool,
    #[serde(default)]
    pub from_codex_config: bool,
    #[serde(default)]
    pub balance_url: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    /// 覆盖 Codex 模型上下文窗口大小（token），None 表示不覆盖。
    /// 注入 codex config 时写入 model_catalog_json。
    #[serde(default)]
    pub context_window_override: Option<u32>,
    /// 强制推理强度，覆盖 Codex 请求中的 effort 值。
    /// "low" / "medium" / "high" / "max"，None 则不覆盖
    #[serde(default)]
    pub reasoning_effort_override: Option<String>,
    /// Claude Extended Thinking Token 预算，注入 thinking.budget_tokens
    #[serde(default)]
    pub thinking_tokens: Option<u32>,
    /// 自定义 HTTP 头，发送上游请求时附加
    #[serde(default)]
    pub custom_headers: HashMap<String, String>,
    /// 供应商扩展选项，给 GUI/诊断展示 provider 能力和后续协议参数。
    #[serde(default)]
    pub provider_options: HashMap<String, serde_json::Value>,
    /// 请求超时（秒），None 则使用全局默认 300s
    #[serde(default)]
    pub request_timeout_secs: Option<u64>,
    /// 上游请求失败时的最大重试次数，None 使用默认值 3
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// 是否启用请求翻译（Responses → Chat Completions）。
    /// 关闭时请求直接透传至上游 Responses API 端点。
    #[serde(default = "default_translate_enabled")]
    pub translate_enabled: bool,
    /// 旧版能力补全开关。当前已废弃，规范化时会关闭，仅保留字段兼容旧账号文件。
    #[serde(default)]
    pub capability_enabled: bool,
    /// 旧版能力补全账号 ID。当前已废弃，仅保留字段兼容旧账号文件。
    #[serde(default)]
    pub capability_account_id: Option<String>,
    /// 是否启用开发协作编排。触发后按角色账号依次完成方案、实现草稿和验收收口。
    #[serde(default)]
    pub dev_pipeline_enabled: bool,
    /// 开发协作编排触发方式：手动命令或始终触发。
    #[serde(default)]
    pub dev_pipeline_trigger_mode: DevPipelineTriggerMode,
    /// 手动触发命令，默认 /dev-pipeline。
    #[serde(default = "default_dev_pipeline_command")]
    pub dev_pipeline_command: String,
    /// 方案设计角色账号 ID。None 或 "active" 表示使用当前活跃账号。
    #[serde(default)]
    pub dev_pipeline_architect_account_id: Option<String>,
    /// 代码/内容实现角色账号 ID。None 或 "active" 表示使用当前活跃账号。
    #[serde(default)]
    pub dev_pipeline_implementer_account_id: Option<String>,
    /// 验收收口角色账号 ID。None 或 "active" 表示使用当前活跃账号。
    #[serde(default)]
    pub dev_pipeline_reviewer_account_id: Option<String>,
    /// 实现阶段工具能力模式，先进入配置与提示词，后续可扩为多轮工具 agent。
    #[serde(default)]
    pub dev_pipeline_tool_mode: DevPipelineToolMode,
    /// 最大修正轮数，避免开发编排无限循环。
    #[serde(default = "default_dev_pipeline_max_iterations")]
    pub dev_pipeline_max_iterations: u32,
    /// 是否在最终回答中显示阶段摘要。
    #[serde(default)]
    pub dev_pipeline_show_trace: bool,
    /// 方案设计角色的附加指令。
    #[serde(default)]
    pub dev_pipeline_architect_instruction: String,
    /// 实现角色的附加指令。
    #[serde(default)]
    pub dev_pipeline_implementer_instruction: String,
    /// 验收角色的附加指令。
    #[serde(default)]
    pub dev_pipeline_reviewer_instruction: String,
    /// v2: 同一账号下的端点配置。旧字段仍保留用于兼容迁移和老 GUI。
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
}

fn default_translate_enabled() -> bool {
    true
}

fn default_dev_pipeline_command() -> String {
    "/dev-pipeline".into()
}

fn default_dev_pipeline_max_iterations() -> u32 {
    3
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevPipelineTriggerMode {
    #[default]
    Manual,
    Always,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DevPipelineToolMode {
    PatchOnly,
    #[default]
    ControlledTools,
    FullAgent,
}

impl Account {
    #[allow(dead_code)]
    pub fn mask_key(&self) -> String {
        if self.api_key.len() <= 8 {
            return "****".to_string();
        }
        let prefix = &self.api_key[..4];
        let suffix = &self.api_key[self.api_key.len() - 4..];
        format!("{}****{}", prefix, suffix)
    }

    fn is_codex_native_responses_provider(&self) -> bool {
        self.client_kind.is_codex()
            && matches!(
                self.provider.to_ascii_lowercase().as_str(),
                "openai" | "minimax" | "mimo"
            )
    }

    fn is_responses_direct_account(&self) -> bool {
        self.is_codex_native_responses_provider()
            || self.endpoints.iter().any(|endpoint| {
                endpoint.kind.is_responses_like() || endpoint.kind == EndpointKind::CodexOfficial
            })
    }

    pub fn is_codex_official_account(&self) -> bool {
        self.client_kind.is_codex()
            && self
                .endpoints
                .iter()
                .any(|endpoint| endpoint.kind == EndpointKind::CodexOfficial)
    }

    fn normalize_responses_direct(&mut self) {
        self.normalize_deepseek_codex_chat_endpoint();
        let force_native_responses = self.is_codex_native_responses_provider();
        if !self.is_responses_direct_account() {
            return;
        }

        self.translate_enabled = false;
        self.model_map.clear();
        self.context_window_override = None;
        self.reasoning_effort_override = None;
        self.thinking_tokens = None;
        self.vision_enabled = false;
        self.vision_upstream.clear();
        self.vision_api_key.clear();
        self.vision_model.clear();
        self.vision_endpoint = default_minimax_vlm_path();
        self.capability_enabled = false;
        self.capability_account_id = None;
        self.dev_pipeline_enabled = false;
        self.dev_pipeline_trigger_mode = DevPipelineTriggerMode::Manual;
        self.dev_pipeline_command = default_dev_pipeline_command();
        self.dev_pipeline_architect_account_id = None;
        self.dev_pipeline_implementer_account_id = None;
        self.dev_pipeline_reviewer_account_id = None;
        self.dev_pipeline_tool_mode = DevPipelineToolMode::ControlledTools;
        self.dev_pipeline_max_iterations = default_dev_pipeline_max_iterations();
        self.dev_pipeline_show_trace = false;
        self.dev_pipeline_architect_instruction.clear();
        self.dev_pipeline_implementer_instruction.clear();
        self.dev_pipeline_reviewer_instruction.clear();

        let provider = self.provider.clone();
        for endpoint in &mut self.endpoints {
            if !force_native_responses
                && !endpoint.kind.is_responses_like()
                && endpoint.kind != EndpointKind::CodexOfficial
            {
                continue;
            }
            if force_native_responses {
                endpoint.kind = EndpointKind::OpenAiResponses;
            }
            endpoint.name = endpoint.kind.label().into();
            endpoint.template_id = match endpoint.kind {
                EndpointKind::CodexOfficial => "codex_official".into(),
                EndpointKind::CustomResponses => {
                    if endpoint.template_id.trim().is_empty() {
                        "custom_responses".into()
                    } else {
                        endpoint.template_id.clone()
                    }
                }
                _ => "responses_direct".into(),
            };
            if endpoint.kind != EndpointKind::CustomResponses {
                endpoint.base_url = normalize_responses_base_url(&endpoint.base_url);
                endpoint.path.clear();
            }
            endpoint.model_map.clear();
            endpoint.model_profiles.clear();
            endpoint.vision = VisionConfig {
                mode: VisionMode::Native,
                ..VisionConfig::default()
            };
            if endpoint.image_generation_enabled.is_none() {
                endpoint.image_generation_enabled = Some(default_image_generation_enabled_for(
                    &provider,
                    &endpoint.kind,
                ));
            }
            endpoint.context_window_override = None;
            endpoint.reasoning_effort_override = None;
            endpoint.thinking_tokens = None;
        }
    }

    fn normalize_deepseek_codex_chat_endpoint(&mut self) {
        if !self.client_kind.is_codex() || !self.provider.eq_ignore_ascii_case("deepseek") {
            return;
        }
        self.upstream = normalize_responses_base_url(&self.upstream);
        for endpoint in &mut self.endpoints {
            endpoint.base_url = normalize_responses_base_url(&endpoint.base_url);
            if !endpoint.kind.is_responses_like() {
                continue;
            }
            endpoint.kind = EndpointKind::OpenAiChat;
            endpoint.name = endpoint.kind.label().into();
            endpoint.template_id = "deepseek".into();
            endpoint.path = "chat/completions".into();
            endpoint.model_map.clear();
            endpoint.model_profiles.clear();
            endpoint.vision = VisionConfig {
                mode: VisionMode::Off,
                ..VisionConfig::default()
            };
            endpoint.image_generation_enabled = Some(false);
            endpoint.context_window_override = None;
            endpoint.reasoning_effort_override = None;
            endpoint.thinking_tokens = None;
        }
    }

    fn retire_codex_model_map(&mut self) {
        if !self.client_kind.is_codex() {
            return;
        }
        let account_models = legacy_model_map_values(&self.model_map);
        for endpoint in &mut self.endpoints {
            let mut models = legacy_model_map_values(&endpoint.model_map);
            if models.is_empty() {
                models.extend(account_models.clone());
            }
            append_known_models(&mut endpoint.known_models, models);
        }
        self.model_map.clear();
        for endpoint in &mut self.endpoints {
            endpoint.model_map.clear();
        }
    }

    fn normalize_mimo_codex_model_profiles(&mut self) {
        if !self.client_kind.is_codex() || !self.provider.eq_ignore_ascii_case("mimo") {
            return;
        }
        for endpoint in &mut self.endpoints {
            if !endpoint.kind.is_chat_like() {
                continue;
            }
            for model in ["mimo-v2.5[1m]", "mimo-v2.5", "mimo-v2-omni"] {
                endpoint.model_profiles.insert(
                    model.into(),
                    ModelProfile {
                        vision_mode: ModelVisionMode::Native,
                    },
                );
            }
            for model in [
                "mimo-v2.5-pro[1m]",
                "mimo-v2.5-pro",
                "mimo-v2-pro[1m]",
                "mimo-v2-pro",
            ] {
                endpoint.model_profiles.insert(
                    model.into(),
                    ModelProfile {
                        vision_mode: ModelVisionMode::Off,
                    },
                );
            }
        }
    }

    fn normalize_runtime_image_capability_failures(&mut self) {
        if runtime_message_is_image_capability_error(&self.runtime_state.status_message) {
            self.runtime_state.status = AccountRuntimeStatus::Active;
            self.runtime_state.next_retry_after = None;
            self.runtime_state.quota = AccountQuotaState::default();
        }
        for state in self.runtime_state.model_states.values_mut() {
            if runtime_message_is_image_capability_error(&state.status_message) {
                state.status = AccountRuntimeStatus::Active;
                state.next_retry_after = None;
                state.quota = AccountQuotaState::default();
            }
        }
    }

    fn normalize_non_quota_runtime_cooldowns(&mut self) {
        if matches!(self.runtime_state.status, AccountRuntimeStatus::CoolingDown)
            && !self.runtime_state.quota.exceeded
        {
            self.runtime_state.status = AccountRuntimeStatus::Error;
            self.runtime_state.next_retry_after = None;
            self.runtime_state.quota = AccountQuotaState::default();
        }
        for state in self.runtime_state.model_states.values_mut() {
            if matches!(state.status, AccountRuntimeStatus::CoolingDown) && !state.quota.exceeded {
                state.status = AccountRuntimeStatus::Error;
                state.next_retry_after = None;
                state.quota = AccountQuotaState::default();
            }
        }
    }

    fn normalize_unsupported_image_policy_default(&mut self) {
        for endpoint in &mut self.endpoints {
            if endpoint.vision.unsupported_image_policy == UnsupportedImagePolicy::Reject {
                endpoint.vision.unsupported_image_policy = UnsupportedImagePolicy::StripWithWarning;
            }
        }
    }

    pub fn normalize_v2(&mut self) {
        self.capability_enabled = false;
        self.capability_account_id = None;
        if !self.client_kind.supports_desktop_surface() {
            self.client_surface = AccountClientSurface::Cli;
        }
        if !self.client_kind.is_codex() {
            self.translate_enabled = false;
            self.endpoints.clear();
            return;
        }
        if self.endpoints.is_empty() {
            self.endpoints.push(endpoint_from_legacy_account(self));
        }
        self.normalize_responses_direct();
        self.retire_codex_model_map();
        self.normalize_mimo_codex_model_profiles();
        self.normalize_unsupported_image_policy_default();
        self.normalize_runtime_image_capability_failures();
        self.normalize_non_quota_runtime_cooldowns();
        if let Some(first) = self.endpoints.first().cloned() {
            self.sync_legacy_from_endpoint(&first);
        }
    }

    pub fn active_endpoint<'a>(
        &'a self,
        active_endpoint_id: Option<&str>,
    ) -> Option<&'a EndpointConfig> {
        active_endpoint_id
            .and_then(|id| self.endpoints.iter().find(|endpoint| endpoint.id == id))
            .or_else(|| self.endpoints.first())
    }

    pub fn sync_legacy_from_endpoint(&mut self, endpoint: &EndpointConfig) {
        self.upstream = endpoint.base_url.clone();
        if self.client_kind.is_codex() {
            self.model_map.clear();
        } else {
            self.model_map = endpoint.model_map.clone();
        }
        self.balance_url = endpoint.balance_url.clone();
        self.context_window_override = endpoint.context_window_override;
        self.reasoning_effort_override = endpoint.reasoning_effort_override.clone();
        self.thinking_tokens = endpoint.thinking_tokens;
        self.custom_headers = endpoint.custom_headers.clone();
        self.request_timeout_secs = endpoint.request_timeout_secs;
        self.max_retries = endpoint.max_retries;
        self.translate_enabled = endpoint.kind.is_chat_like();
        self.vision_enabled = endpoint.vision.mode == VisionMode::Glue;
        self.vision_upstream = endpoint.vision.base_url.clone();
        self.vision_api_key = endpoint.vision.api_key.clone();
        self.vision_model = endpoint.vision.model.clone();
        self.vision_endpoint = endpoint.vision.path.clone();
    }

    pub fn record_runtime_success(&mut self, model: &str, now: u64) {
        self.runtime_state.record_request(now, true);
        self.runtime_state.success = self.runtime_state.success.saturating_add(1);
        self.runtime_state.status = AccountRuntimeStatus::Active;
        self.runtime_state.status_message.clear();
        self.runtime_state.next_retry_after = None;
        self.runtime_state.quota = AccountQuotaState::default();
        if !model.trim().is_empty() {
            let model_key = model.trim();
            let keep_active_cooldown = self
                .runtime_state
                .model_states
                .get(model_key)
                .is_some_and(|state| state.next_retry_after.is_some_and(|retry| retry > now));
            if !keep_active_cooldown {
                self.runtime_state.model_states.remove(model_key);
            }
        }
    }

    pub fn record_runtime_failure(
        &mut self,
        model: &str,
        status_code: u16,
        message: String,
        retry_after_secs: Option<u64>,
        now: u64,
    ) {
        self.runtime_state.record_request(now, false);
        self.runtime_state.failed = self.runtime_state.failed.saturating_add(1);

        let model_key = model.trim().to_string();
        if runtime_message_is_image_capability_error(&message) {
            self.runtime_state.status = AccountRuntimeStatus::Active;
            self.runtime_state.status_message = message.clone();
            self.runtime_state.next_retry_after = None;
            self.runtime_state.quota = AccountQuotaState::default();
            if !model_key.is_empty() {
                self.runtime_state.model_states.insert(
                    model_key,
                    AccountModelRuntimeState {
                        status: AccountRuntimeStatus::Active,
                        status_message: message,
                        next_retry_after: None,
                        quota: AccountQuotaState::default(),
                        updated_at: now,
                    },
                );
            }
            return;
        }
        let previous_backoff = if model_key.is_empty() {
            self.runtime_state.quota.backoff_level
        } else {
            self.runtime_state
                .model_states
                .get(&model_key)
                .map(|state| state.quota.backoff_level)
                .unwrap_or(0)
        };
        let cooldown = runtime_cooldown_for_status(status_code, retry_after_secs, previous_backoff);

        self.runtime_state.status_message = message.clone();
        self.runtime_state.next_retry_after = cooldown.next_retry_after;
        self.runtime_state.status = cooldown.status.clone();
        self.runtime_state.quota = cooldown.quota.clone();

        if !model_key.is_empty() {
            self.runtime_state.model_states.insert(
                model_key,
                AccountModelRuntimeState {
                    status: cooldown.status,
                    status_message: message,
                    next_retry_after: cooldown.next_retry_after,
                    quota: cooldown.quota,
                    updated_at: now,
                },
            );
        }
    }

    pub fn record_runtime_failure_observation(&mut self, model: &str, message: String, now: u64) {
        self.runtime_state.record_request(now, false);
        self.runtime_state.failed = self.runtime_state.failed.saturating_add(1);

        let model_key = model.trim().to_string();
        let status = if runtime_message_is_image_capability_error(&message) {
            AccountRuntimeStatus::Active
        } else {
            AccountRuntimeStatus::Error
        };
        self.runtime_state.status = status.clone();
        self.runtime_state.status_message = message.clone();
        self.runtime_state.next_retry_after = None;
        self.runtime_state.quota = AccountQuotaState::default();

        if !model_key.is_empty() {
            self.runtime_state.model_states.insert(
                model_key,
                AccountModelRuntimeState {
                    status,
                    status_message: message,
                    next_retry_after: None,
                    quota: AccountQuotaState::default(),
                    updated_at: now,
                },
            );
        }
    }

    #[allow(dead_code)]
    pub fn clear_runtime_cooldown(&mut self, now: u64) {
        self.runtime_state.status = AccountRuntimeStatus::Active;
        self.runtime_state.status_message.clear();
        self.runtime_state.next_retry_after = None;
        self.runtime_state.quota = AccountQuotaState::default();
        self.runtime_state.model_states.retain(|_, state| {
            if state.next_retry_after.is_some_and(|retry| retry > now) {
                return false;
            }
            if matches!(
                state.status,
                AccountRuntimeStatus::CoolingDown | AccountRuntimeStatus::QuotaExceeded
            ) {
                return false;
            }
            true
        });
    }

    #[allow(dead_code)]
    pub fn reset_runtime_state(&mut self) {
        self.runtime_state = AccountRuntimeState::default();
    }
}

pub fn account_routing_options(account: &Account) -> AccountRoutingOptions {
    let mut options = AccountRoutingOptions::default();
    if let Some(Value::Object(routing)) = account.client_options.get("routing") {
        if let Some(enabled) = routing.get("enabled").and_then(Value::as_bool) {
            options.enabled = enabled;
        }
        if let Some(anchor_enabled) = routing.get("anchor_enabled").and_then(Value::as_bool) {
            options.anchor_enabled = Some(anchor_enabled);
        }
        if let Some(execution_enabled) = routing.get("execution_enabled").and_then(Value::as_bool) {
            options.execution_enabled = Some(execution_enabled);
        }
        if let Some(pool) = routing.get("pool").and_then(Value::as_str) {
            options.pool = pool.to_string();
        }
        if let Some(priority) = routing.get("priority").and_then(Value::as_i64) {
            options.priority = priority;
        }
        if let Some(weight) = routing.get("weight").and_then(Value::as_u64) {
            options.weight = weight.min(u32::MAX as u64) as u32;
        }
        if let Some(policy) = routing
            .get("native_computer_policy")
            .and_then(Value::as_str)
        {
            options.native_computer_policy = policy.to_string();
        }
        if let Some(disabled) = routing.get("disabled").and_then(Value::as_bool) {
            options.disabled = disabled;
        }
    }
    options.normalized()
}

#[allow(dead_code)]
pub fn set_account_routing_options(account: &mut Account, options: AccountRoutingOptions) {
    let options = options.normalized();
    account.client_options.insert(
        "routing".into(),
        serde_json::json!({
            "enabled": options.enabled,
            "anchor_enabled": options.anchor_enabled,
            "execution_enabled": options.execution_enabled,
            "pool": options.pool,
            "priority": options.priority,
            "weight": options.weight,
            "native_computer_policy": options.native_computer_policy,
            "disabled": options.disabled,
        }),
    );
}

#[derive(Debug, Clone)]
pub struct RuntimeCooldown {
    pub status: AccountRuntimeStatus,
    pub next_retry_after: Option<u64>,
    pub quota: AccountQuotaState,
}

pub fn runtime_cooldown_for_status(
    status_code: u16,
    retry_after_secs: Option<u64>,
    previous_backoff_level: u32,
) -> RuntimeCooldown {
    let now = now_secs();
    let mut quota = AccountQuotaState::default();
    let (status, wait_secs, backoff_level) = match status_code {
        401..=404 => (AccountRuntimeStatus::Error, None, previous_backoff_level),
        429 => {
            let (wait, next_level) = match retry_after_secs.filter(|v| *v > 0) {
                Some(wait) => (wait, previous_backoff_level),
                None => next_quota_backoff(previous_backoff_level),
            };
            quota.exceeded = true;
            quota.reason = "quota".into();
            quota.backoff_level = next_level;
            (AccountRuntimeStatus::QuotaExceeded, Some(wait), next_level)
        }
        408 | 500 | 502 | 503 | 504 => {
            let (default_wait, next_level) =
                next_transient_upstream_backoff(previous_backoff_level);
            let wait = retry_after_secs
                .filter(|wait| *wait > 0)
                .map(|wait| wait.max(default_wait))
                .unwrap_or(default_wait);
            quota.backoff_level = next_level;
            (AccountRuntimeStatus::Error, Some(wait), next_level)
        }
        _ => (AccountRuntimeStatus::Error, None, previous_backoff_level),
    };
    if status_code == 429 {
        quota.backoff_level = backoff_level;
    }
    let next_retry_after = wait_secs.map(|wait| now.saturating_add(wait));
    if quota.exceeded {
        quota.next_recover_at = next_retry_after;
    }
    RuntimeCooldown {
        status,
        next_retry_after,
        quota,
    }
}

fn runtime_message_is_image_capability_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "no endpoints found that support image input",
        "unsupported_image",
        "vision_disabled",
        "image input",
        "images are not supported",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn next_quota_backoff(previous_level: u32) -> (u64, u32) {
    let level = previous_level.min(31);
    let wait = 1u64.checked_shl(level).unwrap_or(30 * 60).clamp(1, 30 * 60);
    let next_level = if wait >= 30 * 60 {
        previous_level
    } else {
        previous_level.saturating_add(1)
    };
    (wait, next_level)
}

fn next_transient_upstream_backoff(previous_level: u32) -> (u64, u32) {
    let level = previous_level.min(3);
    let wait = 60u64
        .saturating_mul(1u64.checked_shl(level).unwrap_or(8))
        .min(5 * 60);
    let next_level = if wait >= 5 * 60 {
        previous_level
    } else {
        previous_level.saturating_add(1)
    };
    (wait, next_level)
}

impl AccountRuntimeState {
    pub fn record_request(&mut self, now: u64, success: bool) {
        let bucket_start = now / 600 * 600;
        if let Some(bucket) = self
            .recent_requests
            .iter_mut()
            .find(|bucket| bucket.bucket_start == bucket_start)
        {
            if success {
                bucket.success = bucket.success.saturating_add(1);
            } else {
                bucket.failed = bucket.failed.saturating_add(1);
            }
        } else {
            self.recent_requests.push(AccountRecentRequestBucket {
                bucket_start,
                success: u64::from(success),
                failed: u64::from(!success),
            });
        }
        self.recent_requests
            .sort_by_key(|bucket| bucket.bucket_start);
        if self.recent_requests.len() > 20 {
            let keep_from = self.recent_requests.len() - 20;
            self.recent_requests.drain(0..keep_from);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStore {
    #[serde(default = "default_account_store_version")]
    pub version: u32,
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub active_id: Option<String>,
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub active_endpoint_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_default_on_null")]
    pub active_by_surface: HashMap<String, SurfaceActiveSelection>,
}

fn default_account_store_version() -> u32 {
    ACCOUNT_STORE_VERSION
}

fn deserialize_default_on_null<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl Default for AccountStore {
    fn default() -> Self {
        Self {
            version: ACCOUNT_STORE_VERSION,
            accounts: Vec::new(),
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        }
    }
}

impl AccountStore {
    pub fn normalize_v2(&mut self) {
        self.version = ACCOUNT_STORE_VERSION;
        if self.active_account_id.is_none() {
            self.active_account_id = self.active_id.clone();
        }
        if self.active_id.is_none() {
            self.active_id = self.active_account_id.clone();
        }
        for account in &mut self.accounts {
            account.normalize_v2();
        }
        self.repair_duplicate_account_ids();

        let active_exists = self.active_account_id.as_ref().is_some_and(|id| {
            self.accounts
                .iter()
                .any(|account| &account.id == id && account.client_kind.is_codex())
        });
        if !active_exists {
            self.active_account_id = self
                .accounts
                .iter()
                .find(|account| account.client_kind.is_codex())
                .map(|a| a.id.clone());
        }
        self.active_id = self.active_account_id.clone();

        let active_endpoint_valid = self
            .active_account()
            .and_then(|account| {
                if !account.client_kind.is_codex() {
                    return None;
                }
                self.active_endpoint_id.as_ref().map(|endpoint_id| {
                    account
                        .endpoints
                        .iter()
                        .any(|endpoint| &endpoint.id == endpoint_id)
                })
            })
            .unwrap_or(false);
        if !active_endpoint_valid {
            self.active_endpoint_id = self
                .active_account()
                .filter(|account| account.client_kind.is_codex())
                .and_then(|account| account.endpoints.first())
                .map(|endpoint| endpoint.id.clone());
        }
        let active_account_id = self.active_account_id.clone();
        let active_endpoint_id = self.active_endpoint_id.clone();
        for account in &mut self.accounts {
            if !account.client_kind.is_codex() {
                account.translate_enabled = false;
                account.endpoints.clear();
                continue;
            }
            let endpoint = if Some(&account.id) == active_account_id.as_ref() {
                account
                    .active_endpoint(active_endpoint_id.as_deref())
                    .cloned()
            } else {
                account.endpoints.first().cloned()
            };
            if let Some(endpoint) = endpoint {
                account.sync_legacy_from_endpoint(&endpoint);
            }
        }
        self.repair_surface_active();
        self.repair_dex_assistant_active();
    }

    fn repair_surface_active(&mut self) {
        for kind in [
            AccountClientKind::Codex,
            AccountClientKind::ClaudeCode,
            AccountClientKind::Openclaw,
            AccountClientKind::Hermes,
            AccountClientKind::GenericClient,
        ] {
            self.repair_surface_active_for_kind(kind);
        }
    }

    fn repair_surface_active_for_kind(&mut self, kind: AccountClientKind) {
        for surface in kind.active_surfaces() {
            let key = surface_active_key(&kind, &surface);
            let selected_account_id = self
                .active_by_surface
                .get(&key)
                .and_then(|selection| selection.account_id.as_ref())
                .filter(|account_id| {
                    self.accounts.iter().any(|account| {
                        &account.id == *account_id
                            && account.client_kind == kind
                            && account.client_surface == surface
                    })
                })
                .cloned()
                .or_else(|| {
                    self.recently_applied_surface_account(&kind, &surface)
                        .map(|account| account.id.clone())
                })
                .or_else(|| {
                    if kind.is_codex() || kind.supports_desktop_surface() {
                        self.accounts
                            .iter()
                            .find(|account| {
                                account.client_kind == kind && account.client_surface == surface
                            })
                            .map(|account| account.id.clone())
                    } else {
                        None
                    }
                });

            let Some(account_id) = selected_account_id else {
                self.active_by_surface.remove(&key);
                continue;
            };

            let endpoint_id = self
                .active_by_surface
                .get(&key)
                .and_then(|selection| selection.endpoint_id.as_ref())
                .filter(|endpoint_id| {
                    self.accounts
                        .iter()
                        .find(|account| account.id == account_id)
                        .is_some_and(|account| {
                            account
                                .endpoints
                                .iter()
                                .any(|endpoint| &endpoint.id == *endpoint_id)
                        })
                })
                .cloned()
                .or_else(|| {
                    self.accounts
                        .iter()
                        .find(|account| account.id == account_id)
                        .and_then(|account| account.endpoints.first())
                        .map(|endpoint| endpoint.id.clone())
                });

            self.active_by_surface.insert(
                key,
                SurfaceActiveSelection {
                    account_id: Some(account_id),
                    endpoint_id,
                },
            );
        }
    }

    fn recently_applied_surface_account(
        &self,
        kind: &AccountClientKind,
        surface: &AccountClientSurface,
    ) -> Option<&Account> {
        if kind.is_codex() {
            return None;
        }
        self.accounts
            .iter()
            .filter(|account| &account.client_kind == kind && &account.client_surface == surface)
            .filter(|account| account.last_applied_at.is_some())
            .max_by_key(|account| account.last_applied_at.unwrap_or_default())
    }

    fn repair_dex_assistant_active(&mut self) {
        let selected_account_id = self
            .active_by_surface
            .get(DEX_ASSISTANT_ACTIVE_KEY)
            .and_then(|selection| selection.account_id.as_ref())
            .filter(|account_id| {
                self.accounts
                    .iter()
                    .any(|account| &account.id == *account_id && account.client_kind.is_codex())
            })
            .cloned()
            .or_else(|| {
                self.active_account_id
                    .as_ref()
                    .filter(|account_id| {
                        self.accounts.iter().any(|account| {
                            &account.id == *account_id && account.client_kind.is_codex()
                        })
                    })
                    .cloned()
            })
            .or_else(|| {
                self.active_account_for_surface(&AccountClientSurface::Cli)
                    .map(|account| account.id.clone())
            })
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|account| account.client_kind.is_codex())
                    .map(|account| account.id.clone())
            });

        let Some(account_id) = selected_account_id else {
            self.active_by_surface.remove(DEX_ASSISTANT_ACTIVE_KEY);
            return;
        };

        let endpoint_id = self
            .active_by_surface
            .get(DEX_ASSISTANT_ACTIVE_KEY)
            .and_then(|selection| selection.endpoint_id.as_ref())
            .filter(|endpoint_id| {
                self.accounts
                    .iter()
                    .find(|account| account.id == account_id)
                    .is_some_and(|account| {
                        account
                            .endpoints
                            .iter()
                            .any(|endpoint| &endpoint.id == *endpoint_id)
                    })
            })
            .cloned()
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|account| account.id == account_id)
                    .and_then(|account| {
                        if self.active_account_id.as_deref() == Some(account.id.as_str()) {
                            account.active_endpoint(self.active_endpoint_id.as_deref())
                        } else {
                            None
                        }
                        .or_else(|| {
                            self.active_endpoint_id_for_surface(
                                &AccountClientKind::Codex,
                                &account.client_surface,
                            )
                            .and_then(|endpoint_id| account.active_endpoint(Some(endpoint_id)))
                        })
                        .or_else(|| account.endpoints.first())
                    })
                    .map(|endpoint| endpoint.id.clone())
            });

        self.active_by_surface.insert(
            DEX_ASSISTANT_ACTIVE_KEY.into(),
            SurfaceActiveSelection {
                account_id: Some(account_id),
                endpoint_id,
            },
        );
    }

    #[allow(dead_code)]
    pub fn set_active_for_surface(
        &mut self,
        kind: &AccountClientKind,
        surface: &AccountClientSurface,
        account_id: String,
        endpoint_id: Option<String>,
    ) {
        let key = surface_active_key(kind, surface);
        self.active_by_surface.insert(
            key,
            SurfaceActiveSelection {
                account_id: Some(account_id),
                endpoint_id,
            },
        );
    }

    pub fn active_selection_for_surface(
        &self,
        kind: &AccountClientKind,
        surface: &AccountClientSurface,
    ) -> Option<&SurfaceActiveSelection> {
        self.active_by_surface
            .get(&surface_active_key(kind, surface))
    }

    pub fn active_endpoint_id_for_surface(
        &self,
        kind: &AccountClientKind,
        surface: &AccountClientSurface,
    ) -> Option<&str> {
        self.active_selection_for_surface(kind, surface)
            .and_then(|selection| selection.endpoint_id.as_deref())
    }

    #[allow(dead_code)]
    pub fn set_active_for_dex_assistant(
        &mut self,
        account_id: String,
        endpoint_id: Option<String>,
    ) {
        self.active_by_surface.insert(
            DEX_ASSISTANT_ACTIVE_KEY.into(),
            SurfaceActiveSelection {
                account_id: Some(account_id),
                endpoint_id,
            },
        );
    }

    pub fn active_selection_for_dex_assistant(&self) -> Option<&SurfaceActiveSelection> {
        self.active_by_surface.get(DEX_ASSISTANT_ACTIVE_KEY)
    }

    pub fn active_endpoint_id_for_dex_assistant(&self) -> Option<&str> {
        self.active_selection_for_dex_assistant()
            .and_then(|selection| selection.endpoint_id.as_deref())
    }

    pub fn active_account_for_surface(&self, surface: &AccountClientSurface) -> Option<&Account> {
        self.active_account_for_kind_surface(&AccountClientKind::Codex, surface)
    }

    pub fn active_account_for_kind_surface(
        &self,
        kind: &AccountClientKind,
        surface: &AccountClientSurface,
    ) -> Option<&Account> {
        let selection = self.active_selection_for_surface(kind, surface);
        selection
            .and_then(|selection| selection.account_id.as_ref())
            .and_then(|id| {
                self.accounts.iter().find(|account| {
                    &account.id == id
                        && &account.client_kind == kind
                        && &account.client_surface == surface
                })
            })
            .or_else(|| {
                if kind.is_codex() || kind.supports_desktop_surface() {
                    self.accounts.iter().find(|account| {
                        &account.client_kind == kind && &account.client_surface == surface
                    })
                } else {
                    None
                }
            })
    }

    pub fn active_account_for_dex_assistant(&self) -> Option<&Account> {
        self.active_selection_for_dex_assistant()
            .and_then(|selection| selection.account_id.as_ref())
            .and_then(|id| {
                self.accounts
                    .iter()
                    .find(|account| &account.id == id && account.client_kind.is_codex())
            })
            .or_else(|| {
                self.active_account()
                    .filter(|account| account.client_kind.is_codex())
            })
            .or_else(|| self.active_account_for_surface(&AccountClientSurface::Cli))
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|account| account.client_kind.is_codex())
            })
    }

    #[allow(dead_code)]
    pub fn active_endpoint_for_dex_assistant(&self) -> Option<&EndpointConfig> {
        let endpoint_id = self.active_endpoint_id_for_dex_assistant();
        self.active_account_for_dex_assistant()
            .and_then(|account| account.active_endpoint(endpoint_id))
    }

    #[allow(dead_code)]
    pub fn active_endpoint_for_surface(
        &self,
        surface: &AccountClientSurface,
    ) -> Option<&EndpointConfig> {
        self.active_endpoint_for_kind_surface(&AccountClientKind::Codex, surface)
    }

    #[allow(dead_code)]
    pub fn active_endpoint_for_kind_surface(
        &self,
        kind: &AccountClientKind,
        surface: &AccountClientSurface,
    ) -> Option<&EndpointConfig> {
        let endpoint_id = self.active_endpoint_id_for_surface(kind, surface);
        self.active_account_for_kind_surface(kind, surface)
            .and_then(|account| account.active_endpoint(endpoint_id))
    }

    fn repair_duplicate_account_ids(&mut self) {
        let mut seen = HashSet::new();
        for account in &mut self.accounts {
            let original_id = account.id.clone();
            if seen.insert(original_id.clone()) {
                continue;
            }

            let mut repaired_id = generate_id();
            while seen.contains(&repaired_id) {
                repaired_id = generate_id();
            }
            seen.insert(repaired_id.clone());
            account.id = repaired_id.clone();

            for endpoint in &mut account.endpoints {
                if endpoint.id == original_id || endpoint.id == format!("endpoint_{original_id}") {
                    endpoint.id = format!("endpoint_{repaired_id}");
                }
            }
        }
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.active_account_id
            .as_ref()
            .or(self.active_id.as_ref())
            .and_then(|id| self.accounts.iter().find(|account| &account.id == id))
            .filter(|account| account.client_kind.is_codex())
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|account| account.client_kind.is_codex())
            })
    }

    #[allow(dead_code)]
    pub fn active_account_mut(&mut self) -> Option<&mut Account> {
        let active_id = self
            .active_account_id
            .clone()
            .or_else(|| self.active_id.clone());
        if let Some(id) = active_id {
            if let Some(pos) = self
                .accounts
                .iter()
                .position(|account| account.id == id && account.client_kind.is_codex())
            {
                return self.accounts.get_mut(pos);
            }
        }
        self.accounts
            .iter_mut()
            .find(|account| account.client_kind.is_codex())
    }

    #[allow(dead_code)]
    pub fn active_endpoint(&self) -> Option<&EndpointConfig> {
        self.active_account()
            .and_then(|account| account.active_endpoint(self.active_endpoint_id.as_deref()))
    }
}

fn endpoint_from_legacy_account(account: &Account) -> EndpointConfig {
    let mut vision = VisionConfig::default();
    if account.vision_enabled || !account.vision_upstream.is_empty() {
        vision.mode = VisionMode::Glue;
        vision.base_url = account.vision_upstream.clone();
        vision.api_key = account.vision_api_key.clone();
        vision.model = account.vision_model.clone();
        vision.path = if account.vision_endpoint.is_empty() {
            default_minimax_vlm_path()
        } else {
            account.vision_endpoint.clone()
        };
    }

    let kind = if account.translate_enabled {
        EndpointKind::OpenAiChat
    } else {
        EndpointKind::OpenAiResponses
    };

    EndpointConfig {
        id: format!("endpoint_{}", account.id),
        name: if account.translate_enabled {
            "Chat Completions".into()
        } else {
            "Responses".into()
        },
        kind: kind.clone(),
        base_url: account.upstream.clone(),
        path: String::new(),
        template_id: account.provider.clone(),
        template_version: default_template_version(),
        model_map: account.model_map.clone(),
        known_models: legacy_model_map_values(&account.model_map),
        model_profiles: HashMap::new(),
        vision,
        image_generation_enabled: Some(default_image_generation_enabled_for(
            &account.provider,
            &kind,
        )),
        custom_headers: account.custom_headers.clone(),
        request_timeout_secs: account.request_timeout_secs,
        max_retries: account.max_retries,
        context_window_override: account.context_window_override,
        reasoning_effort_override: account.reasoning_effort_override.clone(),
        thinking_tokens: account.thinking_tokens,
        fast_mode_enabled: false,
        fast_service_tier: default_fast_service_tier(),
        balance_url: account.balance_url.clone(),
    }
}

// ── 供应商预设 ──────────────────────────────────────────────────────────────

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPreset {
    pub slug: String,
    pub label: String,
    pub description: String,
    pub default_upstream: String,
    pub known_models: Vec<String>,
    pub default_api_key_env: String,
    pub wire_protocol: WireProtocol,
    pub auth_scheme: AuthScheme,
    pub model_discovery: ModelDiscovery,
    pub capabilities: ProviderCapabilities,
    pub capability_labels: Vec<String>,
    pub provider_options: HashMap<String, serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointTemplate {
    pub id: String,
    pub label: String,
    pub provider: String,
    pub kind: EndpointKind,
    pub default_base_url: String,
    pub default_path: String,
    pub default_vision_mode: VisionMode,
    pub description: String,
}

#[allow(dead_code)]
pub fn get_provider_presets() -> Vec<ProviderPreset> {
    get_provider_profiles()
        .into_iter()
        .map(|p| {
            let capability_labels = crate::providers::capability_labels(&p)
                .into_iter()
                .map(str::to_string)
                .collect();
            let provider_options = provider_options_for_slug(&p.slug);
            ProviderPreset {
                slug: p.slug,
                label: p.label,
                description: p.description,
                default_upstream: p.default_upstream,
                known_models: p.known_models,
                default_api_key_env: p.default_api_key_env,
                wire_protocol: p.wire_protocol,
                auth_scheme: p.auth_scheme,
                model_discovery: p.model_discovery,
                capabilities: p.capabilities,
                capability_labels,
                provider_options,
            }
        })
        .collect()
}

#[allow(dead_code)]
pub fn get_endpoint_templates() -> Vec<EndpointTemplate> {
    vec![
        EndpointTemplate {
            id: "chat_compatible".into(),
            label: "Chat 兼容端点".into(),
            provider: "protocol".into(),
            kind: EndpointKind::OpenAiChat,
            default_base_url: String::new(),
            default_path: "chat/completions".into(),
            default_vision_mode: VisionMode::Off,
            description:
                "OpenAI Chat Completions 兼容协议；OpenRouter 或未确认 Responses 支持的兼容站点通常选择它"
                    .into(),
        },
        EndpointTemplate {
            id: "responses_direct".into(),
            label: "Responses 直连端点".into(),
            provider: "protocol".into(),
            kind: EndpointKind::OpenAiResponses,
            default_base_url: String::new(),
            default_path: "responses".into(),
            default_vision_mode: VisionMode::Native,
            description: "OpenAI Responses API 直连协议，图片默认按原生多模态透传".into(),
        },
        EndpointTemplate {
            id: "anthropic_messages".into(),
            label: "Anthropic Messages".into(),
            provider: "anthropic".into(),
            kind: EndpointKind::AnthropicMessages,
            default_base_url: "https://api.anthropic.com/v1".into(),
            default_path: "messages".into(),
            default_vision_mode: VisionMode::Native,
            description: "Claude Messages API，支持非流式请求，流式与后台模式预留".into(),
        },
        EndpointTemplate {
            id: "codex_official".into(),
            label: "Codex 官方".into(),
            provider: "codex".into(),
            kind: EndpointKind::CodexOfficial,
            default_base_url: "https://chatgpt.com/backend-api/codex".into(),
            default_path: "responses".into(),
            default_vision_mode: VisionMode::Native,
            description: "ChatGPT Codex 官方 OAuth 后端，使用 Codex CLI 风格鉴权和请求头".into(),
        },
        EndpointTemplate {
            id: "custom_chat".into(),
            label: "自定义 Chat 端点".into(),
            provider: "custom".into(),
            kind: EndpointKind::CustomChat,
            default_base_url: String::new(),
            default_path: "chat/completions".into(),
            default_vision_mode: VisionMode::Off,
            description: "自定义 OpenAI Chat 兼容端点，可按模型覆盖视觉能力".into(),
        },
        EndpointTemplate {
            id: "custom_responses".into(),
            label: "自定义 Responses 端点".into(),
            provider: "custom".into(),
            kind: EndpointKind::CustomResponses,
            default_base_url: String::new(),
            default_path: "responses".into(),
            default_vision_mode: VisionMode::Off,
            description: "自定义 OpenAI Responses 兼容端点，可按模型覆盖视觉能力".into(),
        },
    ]
}

/// Codex 端可能请求的模型名列表（映射表左侧）
pub const CODEX_MODEL_LIST: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.2",
    "codex-auto-review",
];

// ── 持久化 ─────────────────────────────────────────────────────────────────

pub fn accounts_file_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("accounts.json")
}

pub fn accounts_backup_file_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("accounts.json.bak")
}

pub fn accounts_lock_file_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("accounts.json.lock")
}

fn account_store_file_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct AccountStoreFileGuard {
    _thread: MutexGuard<'static, ()>,
    _process_file: File,
}

fn lock_account_store_file(data_dir: &Path) -> Result<AccountStoreFileGuard> {
    let thread = account_store_file_lock().lock().unwrap_or_else(|poisoned| {
        warn!("账号文件锁已被污染，继续接管锁以避免中断账号操作");
        poisoned.into_inner()
    });
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("创建账号目录失败: {}", data_dir.display()))?;
    let lock_path = accounts_lock_file_path(data_dir);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("打开账号锁文件失败: {}", lock_path.display()))?;
    lock_file_exclusive(&file, &lock_path)?;
    Ok(AccountStoreFileGuard {
        _thread: thread,
        _process_file: file,
    })
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File, path: &Path) -> Result<()> {
    const LOCK_EX: i32 = 2;
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    let result = unsafe { flock(file.as_raw_fd(), LOCK_EX) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
            .with_context(|| format!("锁定账号文件失败: {}", path.display()))
    }
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &File, _path: &Path) -> Result<()> {
    Ok(())
}

fn load_accounts_from_path(path: &Path) -> Result<Option<AccountStore>> {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let mut store = parse_account_store(&content)
                .with_context(|| format!("解析账号文件失败: {}", path.display()))?;
            hydrate_account_defaults(&mut store);
            store.normalize_v2();
            Ok(Some(store))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("读取账号文件失败: {}", path.display())),
    }
}

pub fn load_accounts_checked(data_dir: &Path) -> Result<AccountStore> {
    let _guard = lock_account_store_file(data_dir)?;
    load_accounts_checked_inner(data_dir, true)
}

fn load_accounts_checked_inner(
    data_dir: &Path,
    repair_primary_from_backup: bool,
) -> Result<AccountStore> {
    let path = accounts_file_path(data_dir);
    match load_accounts_from_path(&path) {
        Ok(Some(store)) => Ok(store),
        Ok(None) => Ok(AccountStore::default()),
        Err(primary_err) => {
            let backup_path = accounts_backup_file_path(data_dir);
            match load_accounts_from_path(&backup_path) {
                Ok(Some(store)) => {
                    warn!(
                        "账号主文件不可用，已从备份恢复读取: primary={}, backup={}, error={primary_err:#}",
                        path.display(),
                        backup_path.display()
                    );
                    if repair_primary_from_backup {
                        if let Err(err) = save_accounts_unlocked(data_dir, &store) {
                            warn!("从账号备份修复主文件失败: {err:#}");
                        }
                    }
                    Ok(store)
                }
                Ok(None) => Err(primary_err),
                Err(backup_err) => Err(primary_err).with_context(|| {
                    format!(
                        "备份账号文件也不可用: {}, error={backup_err:#}",
                        backup_path.display()
                    )
                }),
            }
        }
    }
}

#[allow(dead_code)]
pub fn load_accounts(data_dir: &Path) -> AccountStore {
    match load_accounts_checked(data_dir) {
        Ok(store) => store,
        Err(err) => {
            warn!("账号文件读取失败，返回空账号库以保持界面可打开: {err:#}");
            AccountStore::default()
        }
    }
}

pub fn parse_account_store(content: &str) -> Result<AccountStore> {
    match serde_json::from_str::<AccountStore>(content) {
        Ok(store) => Ok(store),
        Err(store_err) => match serde_json::from_str::<Vec<Account>>(content) {
            Ok(accounts) => Ok(AccountStore {
                version: 1,
                active_id: accounts.first().map(|account| account.id.clone()),
                active_account_id: accounts.first().map(|account| account.id.clone()),
                active_endpoint_id: None,
                active_by_surface: HashMap::new(),
                accounts,
            }),
            Err(_) => Err(store_err.into()),
        },
    }
}

#[allow(dead_code)]
pub fn save_accounts(data_dir: &Path, store: &AccountStore) -> Result<()> {
    let _guard = lock_account_store_file(data_dir)?;
    save_accounts_unlocked(data_dir, store)
}

pub fn with_account_store<R, F>(data_dir: &Path, mutate: F) -> Result<(AccountStore, R)>
where
    F: FnOnce(&mut AccountStore) -> Result<R>,
{
    let _guard = lock_account_store_file(data_dir)?;
    let mut store = load_accounts_checked_inner(data_dir, false)?;
    let result = mutate(&mut store)?;
    save_accounts_unlocked(data_dir, &store)?;
    store.normalize_v2();
    Ok((store, result))
}

fn save_accounts_unlocked(data_dir: &Path, store: &AccountStore) -> Result<()> {
    let path = accounts_file_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut normalized = store.clone();
    normalized.normalize_v2();
    let content = serde_json::to_string_pretty(&normalized)?;
    parse_account_store(&content).context("账号文件序列化后无法重新解析，已停止写入")?;

    let temp_path = account_temp_file_path(&path);
    {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| format!("创建临时账号文件失败: {}", temp_path.display()))?;
        use std::io::Write;
        file.write_all(content.as_bytes())
            .with_context(|| format!("写入临时账号文件失败: {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("同步临时账号文件失败: {}", temp_path.display()))?;
    }

    if path.exists() {
        if load_accounts_from_path(&path).ok().flatten().is_some() {
            std::fs::copy(&path, accounts_backup_file_path(data_dir))
                .with_context(|| format!("备份账号文件失败: {}", path.display()))?;
        } else {
            warn!("跳过账号备份：当前主文件不可解析: {}", path.display());
        }
    }

    if let Err(err) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err).with_context(|| format!("替换账号文件失败: {}", path.display()));
    }
    Ok(())
}

fn account_temp_file_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("accounts.json");
    path.with_file_name(format!(".{file_name}.{}.tmp", generate_id()))
}

pub fn hydrate_account_defaults(store: &mut AccountStore) {
    for account in &mut store.accounts {
        if account.provider.is_empty() {
            account.provider = guess_provider(&account.upstream).to_string();
        }
        if account.provider_options.is_empty() {
            account.provider_options = provider_options_for_slug(&account.provider);
        }
        if !account.client_kind.is_codex() {
            account.translate_enabled = false;
            account.endpoints.clear();
        }
        if !account.client_kind.supports_desktop_surface() {
            account.client_surface = AccountClientSurface::Cli;
        }
    }
}

#[allow(dead_code)]
pub fn validate_capability_links(store: &AccountStore) -> Result<()> {
    for account in &store.accounts {
        if !account.capability_enabled {
            continue;
        }
        tracing::warn!(
            account_id = %account.id,
            account_name = %account.name,
            "能力补全已废弃，忽略旧配置"
        );
    }
    Ok(())
}

#[allow(dead_code)]
pub fn validate_dev_pipeline_links(store: &AccountStore) -> Result<()> {
    for account in &store.accounts {
        if !account.dev_pipeline_enabled {
            continue;
        }
        for (role, maybe_id) in [
            (
                "方案设计",
                account.dev_pipeline_architect_account_id.as_deref(),
            ),
            (
                "实现",
                account.dev_pipeline_implementer_account_id.as_deref(),
            ),
            ("验收", account.dev_pipeline_reviewer_account_id.as_deref()),
        ] {
            let Some(id) = maybe_id else {
                continue;
            };
            if id.trim().is_empty() || id == "active" {
                continue;
            }
            if !store.accounts.iter().any(|candidate| candidate.id == id) {
                anyhow::bail!("账号 '{}' 的开发协作编排 {} 账号不存在", account.name, role);
            }
        }
        if account.dev_pipeline_command.trim().is_empty()
            && account.dev_pipeline_trigger_mode == DevPipelineTriggerMode::Manual
        {
            anyhow::bail!("账号 '{}' 的开发协作编排触发命令不能为空", account.name);
        }
    }
    Ok(())
}

// ── 工具函数 ────────────────────────────────────────────────────────────────

pub fn generate_id() -> String {
    static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff;
    format!("{ts:x}{seq:04x}")
}

pub fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// 根据上游 URL 猜测供应商
pub fn guess_provider(upstream: &str) -> &str {
    crate::providers::guess_provider(upstream)
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::providers::{
        AuthScheme, ModelsResponseShape, ReasoningMode, StreamUsageMode, WireProtocol,
    };
    use serde_json::json;

    #[test]
    fn provider_presets_include_v1_profiles_and_capabilities() {
        let presets = get_provider_presets();
        let kimi = presets.iter().find(|p| p.slug == "kimi").unwrap();
        let minimax = presets.iter().find(|p| p.slug == "minimax").unwrap();
        let glm = presets.iter().find(|p| p.slug == "glm").unwrap();

        assert_eq!(kimi.default_upstream, "https://api.moonshot.cn/v1");
        assert_eq!(glm.default_upstream, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(kimi.wire_protocol, WireProtocol::ChatCompletions);
        assert_eq!(kimi.model_discovery.endpoint, "models");
        assert_eq!(kimi.auth_scheme, AuthScheme::Bearer);
        assert_eq!(glm.capabilities.reasoning, ReasoningMode::None);
        assert!(minimax.capabilities.allow_missing_done);
        let gemini = presets.iter().find(|p| p.slug == "google-ai").unwrap();
        assert_eq!(gemini.auth_scheme, AuthScheme::GeminiApiKeyQuery);
        assert_eq!(
            gemini.model_discovery.response_shape,
            ModelsResponseShape::GeminiModelsName
        );
        assert_eq!(
            minimax.capabilities.stream_usage,
            StreamUsageMode::FinalChunk
        );
        assert!(minimax
            .provider_options
            .get("capability_labels")
            .and_then(|v| v.as_array())
            .is_some_and(|labels| labels.iter().any(|v| v == "流式容错")));
    }

    #[test]
    fn legacy_account_json_deserializes_and_hydrates_defaults() {
        let raw = json!({
            "accounts": [{
                "id": "old-1",
                "name": "旧 Kimi 账号",
                "provider": "",
                "upstream": "https://api.moonshot.cn/v1",
                "api_key": "sk-old",
                "model_map": {"gpt-5": "moonshot-v1-8k"}
            }],
            "active_id": "old-1"
        });
        let mut store: AccountStore = serde_json::from_value(raw).unwrap();
        hydrate_account_defaults(&mut store);
        let account = &store.accounts[0];

        assert_eq!(account.provider, "kimi");
        assert_eq!(account.wire_protocol, WireProtocol::ChatCompletions);
        assert!(account.translate_enabled);
        assert!(account.provider_options.contains_key("capability_labels"));
        assert!(account.custom_headers.is_empty());
        assert!(!account.dev_pipeline_enabled);
        assert_eq!(
            account.dev_pipeline_trigger_mode,
            DevPipelineTriggerMode::Manual
        );
        assert_eq!(account.dev_pipeline_command, "/dev-pipeline");
        assert_eq!(account.dev_pipeline_architect_account_id, None);
        assert_eq!(account.dev_pipeline_implementer_account_id, None);
        assert_eq!(account.dev_pipeline_reviewer_account_id, None);
        assert_eq!(
            account.dev_pipeline_tool_mode,
            DevPipelineToolMode::ControlledTools
        );
        assert_eq!(account.dev_pipeline_max_iterations, 3);
    }

    #[test]
    fn v3_non_codex_account_does_not_keep_proxy_endpoint() {
        let raw = json!({
            "version": 2,
            "accounts": [{
                "id": "claude-1",
                "name": "Claude Code",
                "provider": "anthropic",
                "client_kind": "claude_code",
                "upstream": "https://api.anthropic.com",
                "api_key": "sk-test",
                "default_model": "claude-sonnet-4-5",
                "translate_enabled": true,
                "endpoints": [{
                    "id": "endpoint_bad",
                    "name": "Chat",
                    "kind": "open_ai_chat",
                    "base_url": "https://api.anthropic.com",
                    "path": ""
                }]
            }],
            "active_id": "claude-1"
        });
        let mut store: AccountStore = serde_json::from_value(raw).unwrap();
        hydrate_account_defaults(&mut store);
        store.normalize_v2();
        let account = &store.accounts[0];
        assert_eq!(store.version, ACCOUNT_STORE_VERSION);
        assert_eq!(account.client_kind, AccountClientKind::ClaudeCode);
        assert!(!account.translate_enabled);
        assert!(account.endpoints.is_empty());
        assert!(store.active_account_id.is_none());
        assert!(store.active_id.is_none());
        assert!(store.active_account().is_none());
        assert!(store.active_endpoint_id.is_none());
    }

    #[test]
    fn openai_codex_account_is_normalized_to_native_responses() {
        let raw = json!({
            "version": 3,
            "accounts": [{
                "id": "openai-1",
                "name": "OpenAI",
                "provider": "openai",
                "client_kind": "codex",
                "upstream": "https://api.openai.com/v1",
                "api_key": "sk-test",
                "model_map": {"gpt-5.5": "other-model"},
                "vision_enabled": true,
                "vision_upstream": "https://vision.example.com",
                "context_window_override": 1000000,
                "reasoning_effort_override": "high",
                "thinking_tokens": 16000,
                "translate_enabled": true,
                "capability_enabled": true,
                "capability_account_id": "helper",
                "dev_pipeline_enabled": true,
                "dev_pipeline_architect_account_id": "helper",
                "endpoints": [{
                    "id": "endpoint_openai",
                    "name": "Chat",
                    "kind": "open_ai_chat",
                    "base_url": "https://api.openai.com/v1",
                    "path": "chat/completions",
                    "model_map": {"gpt-5.5": "other-model"},
                    "model_profiles": {"other-model": {"vision_mode": "glue"}},
                    "vision": {
                        "mode": "glue",
                        "base_url": "https://vision.example.com",
                        "api_key": "sk-vision",
                        "model": "vision-model"
                    },
                    "context_window_override": 1000000,
                    "reasoning_effort_override": "high",
                    "thinking_tokens": 16000,
                    "fast_mode_enabled": true,
                    "request_timeout_secs": 42,
                    "max_retries": 4
                }]
            }],
            "active_id": "openai-1"
        });
        let mut store: AccountStore = serde_json::from_value(raw).unwrap();
        store.normalize_v2();
        let account = &store.accounts[0];
        let endpoint = &account.endpoints[0];

        assert!(!account.translate_enabled);
        assert!(account.model_map.is_empty());
        assert!(!account.vision_enabled);
        assert!(account.vision_upstream.is_empty());
        assert_eq!(account.context_window_override, None);
        assert_eq!(account.reasoning_effort_override, None);
        assert_eq!(account.thinking_tokens, None);
        assert!(!account.capability_enabled);
        assert_eq!(account.capability_account_id, None);
        assert!(!account.dev_pipeline_enabled);
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        assert!(endpoint.path.is_empty());
        assert!(endpoint.model_map.is_empty());
        assert!(endpoint.model_profiles.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Native);
        assert!(endpoint.vision.base_url.is_empty());
        assert_eq!(endpoint.context_window_override, None);
        assert_eq!(endpoint.reasoning_effort_override, None);
        assert_eq!(endpoint.thinking_tokens, None);
        assert_eq!(endpoint.image_generation_enabled, Some(true));
        assert!(endpoint.fast_mode_enabled);
        assert_eq!(endpoint.request_timeout_secs, Some(42));
        assert_eq!(endpoint.max_retries, Some(4));
    }

    #[test]
    fn codex_responses_normalization_trims_full_endpoint_url_to_base_url() {
        let cases = [
            (
                "openai",
                "https://api.openai.com/v1/chat/completions",
                "https://api.openai.com/v1",
            ),
            (
                "openai",
                "https://api.openai.com/v1/responses",
                "https://api.openai.com/v1",
            ),
            (
                "deepseek",
                "https://api.deepseek.com/v1/chat/completions",
                "https://api.deepseek.com/v1",
            ),
            (
                "deepseek",
                "https://api.deepseek.com/v1/responses",
                "https://api.deepseek.com/v1",
            ),
            (
                "minimax",
                "https://api.minimaxi.com/v1/chat/completions",
                "https://api.minimaxi.com/v1",
            ),
            (
                "minimax",
                "https://api.minimaxi.com/v1/responses",
                "https://api.minimaxi.com/v1",
            ),
            (
                "mimo",
                "https://token-plan-cn.xiaomimimo.com/v1/chat/completions",
                "https://token-plan-cn.xiaomimimo.com/v1",
            ),
            (
                "mimo",
                "https://token-plan-cn.xiaomimimo.com/v1/responses",
                "https://token-plan-cn.xiaomimimo.com/v1",
            ),
        ];

        for (provider, full_url, expected_base_url) in cases {
            let raw = json!({
                "version": 3,
                "accounts": [{
                    "id": format!("{provider}-1"),
                    "name": provider,
                    "provider": provider,
                    "client_kind": "codex",
                    "upstream": full_url,
                    "api_key": "sk-test",
                    "translate_enabled": true,
                    "endpoints": [{
                        "id": format!("endpoint_{provider}"),
                        "name": "Chat",
                        "kind": "open_ai_chat",
                        "base_url": full_url,
                        "path": "chat/completions"
                    }]
                }],
                "active_id": format!("{provider}-1")
            });
            let mut store: AccountStore = serde_json::from_value(raw).unwrap();
            store.normalize_v2();
            let account = &store.accounts[0];
            let endpoint = &account.endpoints[0];

            let expected_kind = if provider == "deepseek" {
                EndpointKind::OpenAiChat
            } else {
                EndpointKind::OpenAiResponses
            };
            assert_eq!(endpoint.kind, expected_kind, "{provider}");
            assert_eq!(account.upstream, expected_base_url, "{provider}");
            assert_eq!(endpoint.base_url, expected_base_url, "{provider}");
            if provider == "deepseek" {
                assert_eq!(endpoint.path, "chat/completions", "{provider}");
            } else {
                assert!(endpoint.path.is_empty(), "{provider}");
            }
        }
    }

    #[test]
    fn custom_responses_endpoint_is_normalized_without_losing_custom_path() {
        let raw = json!({
            "version": 3,
            "accounts": [{
                "id": "responses-1",
                "name": "Responses Direct",
                "provider": "custom",
                "client_kind": "codex",
                "upstream": "https://gateway.example.com",
                "api_key": "sk-test",
                "translate_enabled": true,
                "capability_enabled": true,
                "dev_pipeline_enabled": true,
                "context_window_override": 1000000,
                "reasoning_effort_override": "high",
                "thinking_tokens": 16000,
                "model_map": {"gpt-5.5": "mapped-model"},
                "endpoints": [{
                    "id": "endpoint_custom_responses",
                    "name": "Custom Responses",
                    "kind": "custom_responses",
                    "base_url": "https://gateway.example.com",
                    "path": "v2/responses",
                    "model_map": {"gpt-5.5": "mapped-model"},
                    "model_profiles": {"mapped-model": {"vision_mode": "glue"}},
                    "vision": {
                        "mode": "glue",
                        "base_url": "https://vision.example.com",
                        "api_key": "sk-vision",
                        "model": "vision-model"
                    },
                    "context_window_override": 1000000,
                    "reasoning_effort_override": "high",
                    "thinking_tokens": 16000
                }]
            }],
            "active_id": "responses-1"
        });
        let mut store: AccountStore = serde_json::from_value(raw).unwrap();
        store.normalize_v2();
        let account = &store.accounts[0];
        let endpoint = &account.endpoints[0];

        assert!(!account.translate_enabled);
        assert!(!account.capability_enabled);
        assert!(!account.dev_pipeline_enabled);
        assert_eq!(account.context_window_override, None);
        assert_eq!(account.reasoning_effort_override, None);
        assert_eq!(account.thinking_tokens, None);
        assert!(account.model_map.is_empty());
        assert_eq!(endpoint.kind, EndpointKind::CustomResponses);
        assert_eq!(endpoint.path, "v2/responses");
        assert!(endpoint.model_map.is_empty());
        assert!(endpoint.model_profiles.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Native);
        assert!(endpoint.vision.base_url.is_empty());
        assert_eq!(endpoint.context_window_override, None);
        assert_eq!(endpoint.reasoning_effort_override, None);
        assert_eq!(endpoint.thinking_tokens, None);
        assert_eq!(endpoint.image_generation_enabled, Some(false));
    }

    #[test]
    fn codex_official_endpoint_is_normalized_like_native_responses() {
        let raw = json!({
            "version": 3,
            "accounts": [{
                "id": "official-1",
                "name": "Codex 官方",
                "provider": "codex",
                "client_kind": "codex",
                "upstream": "https://chatgpt.com/backend-api/codex",
                "api_key": "sk-oauth",
                "translate_enabled": true,
                "capability_enabled": true,
                "capability_account_id": "helper",
                "dev_pipeline_enabled": true,
                "dev_pipeline_architect_account_id": "helper",
                "context_window_override": 1000000,
                "reasoning_effort_override": "high",
                "thinking_tokens": 16000,
                "model_map": {"gpt-5": "gpt-5"},
                "endpoints": [{
                    "id": "endpoint_official",
                    "name": "Codex 官方",
                    "kind": "codex_official",
                    "base_url": "https://chatgpt.com/backend-api/codex",
                    "path": "responses",
                    "template_id": "codex_official",
                    "model_map": {"gpt-5": "gpt-5"},
                    "model_profiles": {"gpt-5": {"vision_mode": "glue"}},
                    "vision": {
                        "mode": "glue",
                        "base_url": "https://vision.example.com",
                        "api_key": "sk-vision",
                        "model": "vision-model"
                    },
                    "context_window_override": 1000000,
                    "reasoning_effort_override": "high",
                    "thinking_tokens": 16000
                }]
            }],
            "active_id": "official-1"
        });
        let mut store: AccountStore = serde_json::from_value(raw).unwrap();
        store.normalize_v2();
        let account = &store.accounts[0];
        let endpoint = &account.endpoints[0];

        assert!(!account.translate_enabled);
        assert!(!account.capability_enabled);
        assert!(!account.dev_pipeline_enabled);
        assert_eq!(account.context_window_override, None);
        assert_eq!(account.reasoning_effort_override, None);
        assert_eq!(account.thinking_tokens, None);
        assert!(account.model_map.is_empty());
        assert_eq!(endpoint.kind, EndpointKind::CodexOfficial);
        assert!(endpoint.path.is_empty());
        assert_eq!(endpoint.template_id, "codex_official");
        assert!(endpoint.model_map.is_empty());
        assert!(endpoint.model_profiles.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Native);
        assert!(endpoint.vision.base_url.is_empty());
        assert_eq!(endpoint.context_window_override, None);
        assert_eq!(endpoint.reasoning_effort_override, None);
        assert_eq!(endpoint.thinking_tokens, None);
        assert_eq!(endpoint.image_generation_enabled, Some(true));
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use serde_json::json;

    fn base_account(id: &str) -> Account {
        Account {
            id: id.into(),
            name: format!("账号 {id}"),
            provider: "custom".into(),
            client_kind: AccountClientKind::Codex,
            client_surface: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://example.com/v1".into(),
            api_key: String::new(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 0,
            updated_at: 0,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: HashMap::new(),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: default_dev_pipeline_command(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: default_dev_pipeline_max_iterations(),
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn legacy_account_defaults_capability_disabled() {
        let account: Account = serde_json::from_value(json!({
            "id": "a1",
            "name": "legacy",
            "provider": "deepseek",
            "upstream": "https://api.deepseek.com/v1",
            "api_key": "sk-test"
        }))
        .unwrap();

        assert!(!account.capability_enabled);
        assert_eq!(account.capability_account_id, None);
    }

    #[test]
    fn validate_capability_links_rejects_self_reference() {
        let mut account = base_account("a1");
        account.capability_enabled = true;
        account.capability_account_id = Some("a1".into());
        let store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            accounts: vec![account],
            active_id: Some("a1".into()),
            active_account_id: Some("a1".into()),
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };

        assert!(validate_capability_links(&store).is_ok());
    }

    #[test]
    fn validate_capability_links_allows_existing_helper() {
        let mut main = base_account("a1");
        main.capability_enabled = true;
        main.capability_account_id = Some("a2".into());
        let helper = base_account("a2");
        let store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            accounts: vec![main, helper],
            active_id: Some("a1".into()),
            active_account_id: Some("a1".into()),
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };

        assert!(validate_capability_links(&store).is_ok());
    }

    #[test]
    fn normalize_disables_legacy_capability_completion() {
        let mut account = base_account("a1");
        account.capability_enabled = true;
        account.capability_account_id = Some("a2".into());

        account.normalize_v2();

        assert!(!account.capability_enabled);
        assert_eq!(account.capability_account_id, None);
    }

    #[test]
    fn legacy_account_defaults_dev_pipeline_disabled() {
        let account: Account = serde_json::from_value(json!({
            "id": "a1",
            "name": "legacy",
            "provider": "deepseek",
            "upstream": "https://api.deepseek.com/v1",
            "api_key": "sk-test"
        }))
        .unwrap();

        assert!(!account.dev_pipeline_enabled);
        assert_eq!(
            account.dev_pipeline_trigger_mode,
            DevPipelineTriggerMode::Manual
        );
        assert_eq!(account.dev_pipeline_command, "/dev-pipeline");
        assert_eq!(
            account.dev_pipeline_tool_mode,
            DevPipelineToolMode::ControlledTools
        );
        assert_eq!(account.dev_pipeline_max_iterations, 3);
    }

    #[test]
    fn validate_dev_pipeline_links_rejects_missing_role_account() {
        let mut account = base_account("a1");
        account.dev_pipeline_enabled = true;
        account.dev_pipeline_architect_account_id = Some("missing".into());
        let store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            accounts: vec![account],
            active_id: Some("a1".into()),
            active_account_id: Some("a1".into()),
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };

        assert!(validate_dev_pipeline_links(&store).is_err());
    }

    #[test]
    fn validate_dev_pipeline_links_allows_active_and_existing_role_accounts() {
        let mut main = base_account("a1");
        main.dev_pipeline_enabled = true;
        main.dev_pipeline_architect_account_id = Some("active".into());
        main.dev_pipeline_implementer_account_id = None;
        main.dev_pipeline_reviewer_account_id = Some("a2".into());
        let helper = base_account("a2");
        let store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            accounts: vec![main, helper],
            active_id: Some("a1".into()),
            active_account_id: Some("a1".into()),
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };

        assert!(validate_dev_pipeline_links(&store).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_account(translate_enabled: bool) -> Account {
        Account {
            id: "acc_1".into(),
            name: "测试账号".into(),
            provider: "deepseek".into(),
            client_kind: AccountClientKind::Codex,
            client_surface: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::from([("gpt-5".into(), "deepseek-chat".into())]),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 1,
            updated_at: 1,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: HashMap::new(),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: default_dev_pipeline_command(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: default_dev_pipeline_max_iterations(),
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn legacy_chat_account_gets_openai_chat_endpoint() {
        let mut account = legacy_account(true);
        account.normalize_v2();
        let endpoint = account.endpoints.first().unwrap();
        assert_eq!(endpoint.kind, EndpointKind::OpenAiChat);
        assert!(endpoint.model_map.is_empty());
        assert_eq!(endpoint.known_models, vec!["deepseek-chat"]);
        assert_eq!(endpoint.vision.mode, VisionMode::Off);
    }

    #[test]
    fn legacy_minimax_codex_account_is_normalized_to_responses() {
        let mut account = legacy_account(true);
        account.provider = "minimax".into();
        account.upstream = "https://api.minimaxi.com/v1".into();
        account.normalize_v2();

        let endpoint = account.endpoints.first().unwrap();
        assert!(!account.translate_enabled);
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        assert_eq!(endpoint.name, "OpenAI Responses");
        assert!(endpoint.path.is_empty());
        assert!(endpoint.model_map.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Native);
    }

    #[test]
    fn legacy_deepseek_codex_account_stays_on_chat_completions() {
        let mut account = legacy_account(true);
        account.provider = "deepseek".into();
        account.upstream = "https://api.deepseek.com/v1".into();
        account.normalize_v2();

        let endpoint = account.endpoints.first().unwrap();
        assert_eq!(endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(endpoint.name, "Chat Completions");
        assert!(endpoint.path.is_empty());
        assert!(endpoint.model_map.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Off);
    }

    #[test]
    fn existing_deepseek_responses_endpoint_is_migrated_back_to_chat() {
        let mut account = legacy_account(true);
        account.provider = "deepseek".into();
        account.upstream = "https://api.deepseek.com/v1".into();
        account.endpoints = vec![EndpointConfig {
            id: "endpoint_deepseek_responses".into(),
            name: "OpenAI Responses".into(),
            kind: EndpointKind::OpenAiResponses,
            base_url: "https://api.deepseek.com/v1/responses".into(),
            path: String::new(),
            template_id: "responses_direct".into(),
            template_version: 1,
            model_map: HashMap::from([("gpt-5.5".into(), "deepseek-v4-pro".into())]),
            known_models: vec!["deepseek-v4-pro".into(), "deepseek-v4-flash".into()],
            model_profiles: HashMap::new(),
            vision: VisionConfig {
                mode: VisionMode::Native,
                ..VisionConfig::default()
            },
            image_generation_enabled: Some(true),
            custom_headers: HashMap::new(),
            request_timeout_secs: None,
            max_retries: None,
            context_window_override: Some(1000000),
            reasoning_effort_override: Some("high".into()),
            thinking_tokens: Some(16000),
            fast_mode_enabled: false,
            fast_service_tier: "auto".into(),
            balance_url: String::new(),
        }];
        account.normalize_v2();

        let endpoint = account.endpoints.first().unwrap();
        assert_eq!(endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(endpoint.name, "OpenAI Chat");
        assert_eq!(endpoint.base_url, "https://api.deepseek.com/v1");
        assert_eq!(endpoint.path, "chat/completions");
        assert_eq!(endpoint.template_id, "deepseek");
        assert!(endpoint.model_map.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Off);
        assert_eq!(endpoint.image_generation_enabled, Some(false));
        assert_eq!(endpoint.context_window_override, None);
        assert_eq!(endpoint.reasoning_effort_override, None);
        assert_eq!(endpoint.thinking_tokens, None);
    }

    #[test]
    fn legacy_array_file_migrates_to_v2_store() {
        let account = legacy_account(true);
        let content = serde_json::to_string(&vec![account]).unwrap();

        let mut store = parse_account_store(&content).unwrap();
        store.normalize_v2();

        assert_eq!(store.version, ACCOUNT_STORE_VERSION);
        assert_eq!(store.accounts.len(), 1);
        assert_eq!(store.active_account_id.as_deref(), Some("acc_1"));
        assert_eq!(
            store.accounts[0].endpoints[0].kind,
            EndpointKind::OpenAiChat
        );
    }

    #[test]
    fn store_deserializes_null_active_by_surface_as_empty() {
        let content = r#"{
            "version": 3,
            "accounts": [],
            "active_id": null,
            "active_account_id": null,
            "active_endpoint_id": null,
            "active_by_surface": null
        }"#;

        let mut store = parse_account_store(content).unwrap();
        store.normalize_v2();

        assert!(store.active_by_surface.is_empty());
    }

    #[test]
    fn legacy_deepseek_account_gets_chat_endpoint() {
        // 4e198efd 时代 deepseek 默认被切到 OpenAiResponses；5633e4e2 后
        // deepseek codex 切回 Chat Completions（DeepSeek 原生 API 是 Chat），
        // 此处反映最新行为。
        let mut account = legacy_account(false);
        account.normalize_v2();
        assert_eq!(account.endpoints[0].kind, EndpointKind::OpenAiChat);
        assert_eq!(account.endpoints[0].image_generation_enabled, Some(false));
    }

    #[test]
    fn legacy_minimax_vision_becomes_glue_mode() {
        let mut account = legacy_account(true);
        account.vision_enabled = true;
        account.vision_upstream = "https://api.minimax.chat".into();
        account.vision_api_key = "vision-key".into();
        account.vision_model = "MiniMax-M1".into();
        account.normalize_v2();

        let vision = &account.endpoints[0].vision;
        assert_eq!(vision.mode, VisionMode::Glue);
        assert_eq!(vision.adapter_id, "minimax_coding_plan_vlm");
        assert_eq!(vision.base_url, "https://api.minimax.chat");
        assert_eq!(vision.model, "MiniMax-M1");
    }

    #[test]
    fn routing_options_default_and_update_roundtrip() {
        let mut account = legacy_account(true);
        let defaults = account_routing_options(&account);
        assert!(defaults.effective_enabled());
        assert_eq!(defaults.pool, "codex-official");
        assert_eq!(defaults.weight, 1);

        set_account_routing_options(
            &mut account,
            AccountRoutingOptions {
                enabled: false,
                anchor_enabled: Some(false),
                execution_enabled: Some(false),
                pool: " official-main ".into(),
                priority: 30,
                weight: 4,
                native_computer_policy: "helper_required".into(),
                disabled: true,
            },
        );
        let routing = account_routing_options(&account);
        assert!(!routing.effective_enabled());
        assert_eq!(routing.pool, "official-main");
        assert_eq!(routing.priority, 30);
        assert_eq!(routing.weight, 4);
    }

    #[test]
    fn clear_runtime_cooldown_keeps_request_counters() {
        let mut account = legacy_account(true);
        account.runtime_state.success = 3;
        account.runtime_state.failed = 2;
        account.runtime_state.status = AccountRuntimeStatus::QuotaExceeded;
        account.runtime_state.next_retry_after = Some(2_000);
        account.runtime_state.quota.exceeded = true;
        account.runtime_state.model_states.insert(
            "deepseek-chat".into(),
            AccountModelRuntimeState {
                status: AccountRuntimeStatus::CoolingDown,
                status_message: "HTTP 429".into(),
                next_retry_after: Some(2_000),
                quota: AccountQuotaState::default(),
                updated_at: 1_000,
            },
        );

        account.clear_runtime_cooldown(1_000);

        assert_eq!(account.runtime_state.status, AccountRuntimeStatus::Active);
        assert_eq!(account.runtime_state.success, 3);
        assert_eq!(account.runtime_state.failed, 2);
        assert!(account.runtime_state.next_retry_after.is_none());
        assert!(account.runtime_state.model_states.is_empty());
    }

    #[test]
    fn transient_upstream_5xx_uses_default_cooldown_without_retry_after() {
        let cooldown = runtime_cooldown_for_status(502, None, 0);

        assert_eq!(cooldown.status, AccountRuntimeStatus::Error);
        assert!(cooldown.next_retry_after.is_some());
        assert!(!cooldown.quota.exceeded);
        assert_eq!(cooldown.quota.backoff_level, 1);
    }

    #[test]
    fn transient_upstream_5xx_honors_retry_after() {
        let cooldown = runtime_cooldown_for_status(503, Some(30), 0);

        assert_eq!(cooldown.status, AccountRuntimeStatus::Error);
        assert!(cooldown.next_retry_after.is_some());
        assert!(!cooldown.quota.exceeded);
        assert_eq!(cooldown.quota.backoff_level, 1);
    }

    #[test]
    fn transient_upstream_5xx_retry_after_still_increases_backoff() {
        let first = runtime_cooldown_for_status(502, Some(60), 0);
        let second = runtime_cooldown_for_status(502, Some(60), first.quota.backoff_level);

        assert_eq!(first.quota.backoff_level, 1);
        assert_eq!(second.quota.backoff_level, 2);
        assert!(second.next_retry_after.unwrap() > first.next_retry_after.unwrap());
    }

    #[test]
    fn runtime_success_keeps_active_model_cooldown() {
        let mut account = legacy_account(true);
        account.normalize_v2();

        account.record_runtime_failure("gpt-5.5", 502, "HTTP 502".into(), None, 1_000);
        let retry_after = account
            .runtime_state
            .model_states
            .get("gpt-5.5")
            .and_then(|state| state.next_retry_after)
            .unwrap();
        account.record_runtime_success("gpt-5.5", 1_001);

        let model = account.runtime_state.model_states.get("gpt-5.5").unwrap();
        assert_eq!(model.next_retry_after, Some(retry_after));
        assert_eq!(model.status, AccountRuntimeStatus::Error);
    }

    #[test]
    fn auth_and_not_found_errors_do_not_create_local_cooldown() {
        for status in [401, 403, 404] {
            let cooldown = runtime_cooldown_for_status(status, Some(30), 0);

            assert_eq!(cooldown.status, AccountRuntimeStatus::Error);
            assert!(cooldown.next_retry_after.is_none());
            assert!(!cooldown.quota.exceeded);
        }
    }

    #[test]
    fn normalize_clears_legacy_non_quota_runtime_cooldown() {
        let mut account = legacy_account(true);
        account.runtime_state.status = AccountRuntimeStatus::CoolingDown;
        account.runtime_state.status_message = "HTTP 401".into();
        account.runtime_state.next_retry_after = Some(2_000);
        account.runtime_state.model_states.insert(
            "gpt-5.5".into(),
            AccountModelRuntimeState {
                status: AccountRuntimeStatus::CoolingDown,
                status_message: "HTTP 404".into(),
                next_retry_after: Some(2_000),
                quota: AccountQuotaState::default(),
                updated_at: 1_000,
            },
        );

        account.normalize_v2();

        assert_eq!(account.runtime_state.status, AccountRuntimeStatus::Error);
        assert!(account.runtime_state.next_retry_after.is_none());
        let model = account.runtime_state.model_states.get("gpt-5.5").unwrap();
        assert_eq!(model.status, AccountRuntimeStatus::Error);
        assert!(model.next_retry_after.is_none());
    }

    #[test]
    fn image_capability_failure_does_not_cool_down_account() {
        let mut account = legacy_account(true);
        account.normalize_v2();

        account.record_runtime_failure(
            "mimo-v2.5-pro",
            404,
            r#"{"error":{"message":"No endpoints found that support image input"}}"#.into(),
            None,
            1_000,
        );

        assert_eq!(account.runtime_state.status, AccountRuntimeStatus::Active);
        assert!(account.runtime_state.next_retry_after.is_none());
        let model = account
            .runtime_state
            .model_states
            .get("mimo-v2.5-pro")
            .unwrap();
        assert_eq!(model.status, AccountRuntimeStatus::Active);
        assert!(model.next_retry_after.is_none());
    }

    #[test]
    fn model_profile_overrides_endpoint_vision_mode() {
        let mut account = legacy_account(true);
        account.normalize_v2();
        let endpoint = account.endpoints.first_mut().unwrap();
        endpoint.vision.mode = VisionMode::Glue;
        endpoint.model_profiles.insert(
            "deepseek-chat".into(),
            ModelProfile {
                vision_mode: ModelVisionMode::Off,
            },
        );

        assert_eq!(endpoint.model_vision_mode("deepseek-chat"), VisionMode::Off);
        assert_eq!(endpoint.model_vision_mode("other"), VisionMode::Glue);
    }

    #[test]
    fn normalize_migrates_legacy_reject_image_policy_to_strip() {
        let mut account = legacy_account(true);
        account.normalize_v2();
        account.endpoints[0].vision.unsupported_image_policy = UnsupportedImagePolicy::Reject;

        account.normalize_v2();

        assert_eq!(
            account.endpoints[0].vision.unsupported_image_policy,
            UnsupportedImagePolicy::StripWithWarning
        );
    }

    #[test]
    fn mimo_codex_normalize_clears_model_map_and_uses_native_responses_vision() {
        let mut account = legacy_account(true);
        account.provider = "mimo".into();
        account.upstream = "https://token-plan-cn.xiaomimimo.com/v1".into();
        account.model_map = HashMap::from([("gpt-5.5".into(), "old-mapped-model".into())]);
        account.normalize_v2();
        account.endpoints[0].model_profiles.insert(
            "mimo-v2.5-pro".into(),
            ModelProfile {
                vision_mode: ModelVisionMode::Native,
            },
        );

        account.normalize_v2();

        let endpoint = account.endpoints.first().unwrap();
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        assert!(endpoint.model_map.is_empty());
        assert_eq!(endpoint.known_models, vec!["old-mapped-model"]);
        assert!(endpoint.model_profiles.is_empty());
        assert_eq!(endpoint.vision.mode, VisionMode::Native);
        assert_eq!(
            endpoint.model_vision_mode("mimo-v2.5-pro"),
            VisionMode::Native
        );
        assert_eq!(endpoint.model_vision_mode("mimo-v2.5"), VisionMode::Native);
        assert!(account.model_map.is_empty());
    }

    #[test]
    fn store_normalize_repairs_invalid_active_account_and_endpoint() {
        let mut account = legacy_account(true);
        account.normalize_v2();
        let expected_account_id = account.id.clone();
        let expected_endpoint_id = account.endpoints[0].id.clone();
        let mut store = AccountStore {
            version: 1,
            accounts: vec![account],
            active_id: Some("missing_account".into()),
            active_account_id: Some("missing_account".into()),
            active_endpoint_id: Some("missing_endpoint".into()),
            active_by_surface: HashMap::new(),
        };

        store.normalize_v2();

        assert_eq!(store.version, ACCOUNT_STORE_VERSION);
        assert_eq!(
            store.active_account_id.as_deref(),
            Some(expected_account_id.as_str())
        );
        assert_eq!(
            store.active_id.as_deref(),
            Some(expected_account_id.as_str())
        );
        assert_eq!(
            store.active_endpoint_id.as_deref(),
            Some(expected_endpoint_id.as_str())
        );
    }

    #[test]
    fn store_normalize_repairs_codex_surface_active_independently() {
        let mut cli = legacy_account(true);
        cli.id = "cli".into();
        cli.client_surface = AccountClientSurface::Cli;
        cli.normalize_v2();
        cli.endpoints[0].id = "cli-endpoint".into();

        let mut desktop = legacy_account(true);
        desktop.id = "desktop".into();
        desktop.client_surface = AccountClientSurface::Desktop;
        desktop.normalize_v2();
        desktop.endpoints[0].id = "desktop-endpoint".into();

        let mut store = AccountStore {
            version: 2,
            accounts: vec![cli, desktop],
            active_id: Some("cli".into()),
            active_account_id: Some("cli".into()),
            active_endpoint_id: Some("cli-endpoint".into()),
            active_by_surface: HashMap::from([(
                surface_active_key(&AccountClientKind::Codex, &AccountClientSurface::Desktop),
                SurfaceActiveSelection {
                    account_id: Some("desktop".into()),
                    endpoint_id: Some("missing-endpoint".into()),
                },
            )]),
        };

        store.normalize_v2();

        assert_eq!(
            store
                .active_account_for_surface(&AccountClientSurface::Cli)
                .map(|account| account.id.as_str()),
            Some("cli")
        );
        assert_eq!(
            store.active_endpoint_id_for_surface(
                &AccountClientKind::Codex,
                &AccountClientSurface::Cli
            ),
            Some("cli-endpoint")
        );
        assert_eq!(
            store
                .active_account_for_surface(&AccountClientSurface::Desktop)
                .map(|account| account.id.as_str()),
            Some("desktop")
        );
        assert_eq!(
            store.active_endpoint_id_for_surface(
                &AccountClientKind::Codex,
                &AccountClientSurface::Desktop
            ),
            Some("desktop-endpoint")
        );
    }

    #[test]
    fn store_normalize_repairs_claude_surface_active_independently() {
        let mut cli = legacy_account(false);
        cli.id = "claude-cli".into();
        cli.client_kind = AccountClientKind::ClaudeCode;
        cli.client_surface = AccountClientSurface::Cli;
        cli.provider = "anthropic".into();
        cli.normalize_v2();
        cli.last_applied_at = Some(20);

        let mut old_cli = cli.clone();
        old_cli.id = "claude-cli-old".into();
        old_cli.last_applied_at = Some(1);

        let mut desktop = legacy_account(false);
        desktop.id = "claude-desktop".into();
        desktop.client_kind = AccountClientKind::ClaudeCode;
        desktop.client_surface = AccountClientSurface::Desktop;
        desktop.provider = "anthropic".into();
        desktop.normalize_v2();

        let mut store = AccountStore {
            version: 2,
            accounts: vec![old_cli, cli, desktop],
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
            active_by_surface: HashMap::from([(
                surface_active_key(
                    &AccountClientKind::ClaudeCode,
                    &AccountClientSurface::Desktop,
                ),
                SurfaceActiveSelection {
                    account_id: Some("claude-desktop".into()),
                    endpoint_id: Some("stale-endpoint".into()),
                },
            )]),
        };

        store.normalize_v2();

        assert_eq!(
            store
                .active_account_for_kind_surface(
                    &AccountClientKind::ClaudeCode,
                    &AccountClientSurface::Cli
                )
                .map(|account| account.id.as_str()),
            Some("claude-cli")
        );
        assert_eq!(
            store.active_endpoint_id_for_surface(
                &AccountClientKind::ClaudeCode,
                &AccountClientSurface::Cli
            ),
            None
        );
        assert_eq!(
            store
                .active_account_for_kind_surface(
                    &AccountClientKind::ClaudeCode,
                    &AccountClientSurface::Desktop
                )
                .map(|account| account.id.as_str()),
            Some("claude-desktop")
        );
        assert_eq!(
            store.active_endpoint_id_for_surface(
                &AccountClientKind::ClaudeCode,
                &AccountClientSurface::Desktop
            ),
            None
        );
        assert!(store.active_account_id.is_none());
    }

    #[test]
    fn store_normalize_repairs_cli_client_active_from_last_applied() {
        let mut old_hermes = legacy_account(false);
        old_hermes.id = "hermes-old".into();
        old_hermes.client_kind = AccountClientKind::Hermes;
        old_hermes.client_surface = AccountClientSurface::Cli;
        old_hermes.provider = "minimax".into();
        old_hermes.last_applied_at = Some(1);
        old_hermes.normalize_v2();

        let mut hermes = old_hermes.clone();
        hermes.id = "hermes-new".into();
        hermes.last_applied_at = Some(20);

        let mut openclaw = legacy_account(false);
        openclaw.id = "openclaw-active".into();
        openclaw.client_kind = AccountClientKind::Openclaw;
        openclaw.client_surface = AccountClientSurface::Cli;
        openclaw.provider = "openclaw".into();
        openclaw.last_applied_at = Some(10);
        openclaw.normalize_v2();

        let mut store = AccountStore {
            version: 2,
            accounts: vec![old_hermes, hermes, openclaw],
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };

        store.normalize_v2();

        assert_eq!(
            store
                .active_account_for_kind_surface(
                    &AccountClientKind::Hermes,
                    &AccountClientSurface::Cli
                )
                .map(|account| account.id.as_str()),
            Some("hermes-new")
        );
        assert_eq!(
            store
                .active_account_for_kind_surface(
                    &AccountClientKind::Openclaw,
                    &AccountClientSurface::Cli
                )
                .map(|account| account.id.as_str()),
            Some("openclaw-active")
        );
        assert!(store.active_account_id.is_none());
    }

    #[test]
    fn store_normalize_does_not_guess_unapplied_cli_client_active() {
        let mut hermes = legacy_account(false);
        hermes.id = "hermes-unapplied".into();
        hermes.client_kind = AccountClientKind::Hermes;
        hermes.client_surface = AccountClientSurface::Cli;
        hermes.provider = "minimax".into();
        hermes.normalize_v2();

        let mut store = AccountStore {
            version: 2,
            accounts: vec![hermes],
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };

        store.normalize_v2();

        assert!(store
            .active_selection_for_surface(&AccountClientKind::Hermes, &AccountClientSurface::Cli)
            .is_none());
        assert!(store
            .active_account_for_kind_surface(&AccountClientKind::Hermes, &AccountClientSurface::Cli)
            .is_none());
    }

    #[test]
    fn store_normalize_repairs_dex_assistant_active_independently() {
        let mut cli = legacy_account(true);
        cli.id = "cli".into();
        cli.client_surface = AccountClientSurface::Cli;
        cli.normalize_v2();
        cli.endpoints[0].id = "cli-endpoint".into();

        let mut assistant = legacy_account(true);
        assistant.id = "assistant".into();
        assistant.client_surface = AccountClientSurface::Desktop;
        assistant.normalize_v2();
        assistant.endpoints[0].id = "assistant-endpoint".into();

        let mut store = AccountStore {
            version: 2,
            accounts: vec![cli, assistant],
            active_id: Some("cli".into()),
            active_account_id: Some("cli".into()),
            active_endpoint_id: Some("cli-endpoint".into()),
            active_by_surface: HashMap::from([(
                DEX_ASSISTANT_ACTIVE_KEY.into(),
                SurfaceActiveSelection {
                    account_id: Some("assistant".into()),
                    endpoint_id: Some("missing-endpoint".into()),
                },
            )]),
        };

        store.normalize_v2();

        assert_eq!(
            store
                .active_account_for_dex_assistant()
                .map(|account| account.id.as_str()),
            Some("assistant")
        );
        assert_eq!(
            store.active_endpoint_id_for_dex_assistant(),
            Some("assistant-endpoint")
        );
        assert_eq!(store.active_account_id.as_deref(), Some("cli"));
    }

    #[test]
    fn store_normalize_repairs_duplicate_account_ids() {
        let mut first = legacy_account(true);
        first.normalize_v2();
        first.id = "duplicate".into();
        first.endpoints[0].id = "endpoint_duplicate".into();
        let mut second = first.clone();
        second.name = "Second".into();
        second.api_key = "second-key".into();
        let mut store = AccountStore {
            version: 2,
            accounts: vec![first, second],
            active_id: Some("duplicate".into()),
            active_account_id: Some("duplicate".into()),
            active_endpoint_id: Some("endpoint_duplicate".into()),
            active_by_surface: HashMap::new(),
        };

        store.normalize_v2();

        assert_eq!(store.accounts[0].id, "duplicate");
        assert_ne!(store.accounts[1].id, "duplicate");
        assert_ne!(store.accounts[0].id, store.accounts[1].id);
        assert_eq!(
            store.accounts[1].endpoints[0].id,
            format!("endpoint_{}", store.accounts[1].id)
        );
        assert_eq!(store.active_account_id.as_deref(), Some("duplicate"));
        assert_eq!(
            store.active_endpoint_id.as_deref(),
            Some("endpoint_duplicate")
        );
    }

    #[test]
    fn generate_id_is_unique_for_batch_calls() {
        let mut ids = HashSet::new();
        for _ in 0..256 {
            assert!(ids.insert(generate_id()));
        }
    }

    fn test_data_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("deecodex-{label}-{}", generate_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_accounts_writes_atomically_and_keeps_backup() {
        let dir = test_data_dir("accounts-save");
        let mut first = legacy_account(true);
        first.id = "first".into();
        let first_store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            active_id: Some(first.id.clone()),
            active_account_id: Some(first.id.clone()),
            active_endpoint_id: None,
            accounts: vec![first],
            active_by_surface: HashMap::new(),
        };
        save_accounts(&dir, &first_store).unwrap();

        let mut second = legacy_account(true);
        second.id = "second".into();
        let second_store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            active_id: Some(second.id.clone()),
            active_account_id: Some(second.id.clone()),
            active_endpoint_id: None,
            accounts: vec![second],
            active_by_surface: HashMap::new(),
        };
        save_accounts(&dir, &second_store).unwrap();

        let loaded = load_accounts_checked(&dir).unwrap();
        assert_eq!(loaded.active_account_id.as_deref(), Some("second"));
        let backup = std::fs::read_to_string(accounts_backup_file_path(&dir)).unwrap();
        let backup_store = parse_account_store(&backup).unwrap();
        assert_eq!(backup_store.active_account_id.as_deref(), Some("first"));

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn load_accounts_checked_recovers_from_backup_when_primary_is_invalid() {
        let dir = test_data_dir("accounts-backup");
        let mut account = legacy_account(true);
        account.id = "backup-account".into();
        let store = AccountStore {
            version: ACCOUNT_STORE_VERSION,
            active_id: Some(account.id.clone()),
            active_account_id: Some(account.id.clone()),
            active_endpoint_id: None,
            accounts: vec![account],
            active_by_surface: HashMap::new(),
        };
        std::fs::write(
            accounts_backup_file_path(&dir),
            serde_json::to_string_pretty(&store).unwrap(),
        )
        .unwrap();
        std::fs::write(accounts_file_path(&dir), "{broken").unwrap();

        let loaded = load_accounts_checked(&dir).unwrap();
        assert_eq!(loaded.active_account_id.as_deref(), Some("backup-account"));
        let repaired = std::fs::read_to_string(accounts_file_path(&dir)).unwrap();
        let repaired_store = parse_account_store(&repaired).unwrap();
        assert_eq!(
            repaired_store.active_account_id.as_deref(),
            Some("backup-account")
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn store_normalize_syncs_legacy_fields_from_active_endpoint() {
        let mut account = legacy_account(true);
        account.normalize_v2();
        let mut second = account.endpoints[0].clone();
        second.id = "endpoint_responses".into();
        second.base_url = "https://api.openai.com/v1".into();
        second.kind = EndpointKind::OpenAiResponses;
        second.model_map = HashMap::from([("gpt-5".into(), "gpt-5".into())]);
        account.endpoints.push(second.clone());

        let mut store = AccountStore {
            version: 2,
            accounts: vec![account],
            active_id: Some("acc_1".into()),
            active_account_id: Some("acc_1".into()),
            active_endpoint_id: Some(second.id.clone()),
            active_by_surface: HashMap::new(),
        };

        store.normalize_v2();

        let active = store.active_account().unwrap();
        assert_eq!(active.upstream, "https://api.openai.com/v1");
        // 4e198efd 之前 deepseek 默认被切到 OpenAiResponses 时 translate
        // 会被关闭；5633e4e2 后 deepseek 走 Chat Completions，需要
        // translate 开启才能正常转发。
        assert!(active.translate_enabled);
        assert!(active.model_map.is_empty());
    }

    #[test]
    fn endpoint_templates_cover_all_endpoint_kinds() {
        let templates = get_endpoint_templates();
        let kinds: Vec<_> = templates
            .iter()
            .map(|template| template.kind.clone())
            .collect();

        assert!(kinds.contains(&EndpointKind::OpenAiChat));
        assert!(kinds.contains(&EndpointKind::OpenAiResponses));
        assert!(kinds.contains(&EndpointKind::AnthropicMessages));
        assert!(kinds.contains(&EndpointKind::CustomChat));
        assert!(kinds.contains(&EndpointKind::CustomResponses));

        let custom_chat = templates
            .iter()
            .find(|template| template.id == "custom_chat")
            .unwrap();
        assert!(custom_chat.default_base_url.is_empty());
    }
}
