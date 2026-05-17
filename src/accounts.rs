use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::providers::{
    get_provider_profiles, provider_options_for_slug, AuthScheme, ModelDiscovery,
    ProviderCapabilities, WireProtocol,
};

// ── 数据模型 ────────────────────────────────────────────────────────────────

pub const ACCOUNT_STORE_VERSION: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    #[default]
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
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
            Self::OpenAiResponses | Self::CustomResponses => "responses",
            Self::AnthropicMessages => "messages",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAiChat => "OpenAI Chat",
            Self::OpenAiResponses => "OpenAI Responses",
            Self::AnthropicMessages => "Anthropic Messages",
            Self::CustomChat => "自定义 Chat",
            Self::CustomResponses => "自定义 Responses",
        }
    }
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
    #[default]
    Reject,
    StripWithWarning,
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
            unsupported_image_policy: UnsupportedImagePolicy::Reject,
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
    #[serde(default)]
    pub model_profiles: HashMap<String, ModelProfile>,
    #[serde(default)]
    pub vision: VisionConfig,
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
    #[serde(default)]
    pub balance_url: String,
}

fn default_template_version() -> u32 {
    1
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub wire_protocol: WireProtocol,
    pub upstream: String,
    pub api_key: String,
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
    /// 是否启用能力补全：由耦合账号先执行多模态/工具观察，再回传主模型推理。
    #[serde(default)]
    pub capability_enabled: bool,
    /// 能力补全账号 ID，通常选择支持原生工具/多模态能力的 OpenAI/GPT 账号。
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

    pub fn normalize_v2(&mut self) {
        if self.endpoints.is_empty() {
            self.endpoints.push(endpoint_from_legacy_account(self));
        }
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
        self.model_map = endpoint.model_map.clone();
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
}

fn default_account_store_version() -> u32 {
    ACCOUNT_STORE_VERSION
}

impl Default for AccountStore {
    fn default() -> Self {
        Self {
            version: ACCOUNT_STORE_VERSION,
            accounts: Vec::new(),
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
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

        let active_exists = self
            .active_account_id
            .as_ref()
            .is_some_and(|id| self.accounts.iter().any(|account| &account.id == id));
        if !active_exists {
            self.active_account_id = self.accounts.first().map(|a| a.id.clone());
        }
        self.active_id = self.active_account_id.clone();

        let active_endpoint_valid = self
            .active_account()
            .and_then(|account| {
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
                .and_then(|account| account.endpoints.first())
                .map(|endpoint| endpoint.id.clone());
        }

        let active_account_id = self.active_account_id.clone();
        let active_endpoint_id = self.active_endpoint_id.clone();
        for account in &mut self.accounts {
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
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.active_account_id
            .as_ref()
            .or(self.active_id.as_ref())
            .and_then(|id| self.accounts.iter().find(|account| &account.id == id))
            .or_else(|| self.accounts.first())
    }

    #[allow(dead_code)]
    pub fn active_account_mut(&mut self) -> Option<&mut Account> {
        let active_id = self
            .active_account_id
            .clone()
            .or_else(|| self.active_id.clone());
        if let Some(id) = active_id {
            if let Some(pos) = self.accounts.iter().position(|account| account.id == id) {
                return self.accounts.get_mut(pos);
            }
        }
        self.accounts.first_mut()
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

    EndpointConfig {
        id: format!("endpoint_{}", account.id),
        name: if account.translate_enabled {
            "Chat Completions".into()
        } else {
            "Responses".into()
        },
        kind: if account.translate_enabled {
            EndpointKind::OpenAiChat
        } else {
            EndpointKind::OpenAiResponses
        },
        base_url: account.upstream.clone(),
        path: String::new(),
        template_id: account.provider.clone(),
        template_version: default_template_version(),
        model_map: account.model_map.clone(),
        model_profiles: HashMap::new(),
        vision,
        custom_headers: account.custom_headers.clone(),
        request_timeout_secs: account.request_timeout_secs,
        max_retries: account.max_retries,
        context_window_override: account.context_window_override,
        reasoning_effort_override: account.reasoning_effort_override.clone(),
        thinking_tokens: account.thinking_tokens,
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
                "OpenAI Chat Completions 兼容协议；OpenRouter、DeepSeek 等聚合/兼容站点通常选择它"
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
    "gpt-5",
    "codex-auto-review",
];

// ── 持久化 ─────────────────────────────────────────────────────────────────

pub fn accounts_file_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("accounts.json")
}

#[allow(dead_code)]
pub fn load_accounts(data_dir: &Path) -> AccountStore {
    let path = accounts_file_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let mut store = parse_account_store(&content).unwrap_or_default();
            hydrate_account_defaults(&mut store);
            store.normalize_v2();
            store
        }
        Err(_) => AccountStore::default(),
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
                accounts,
            }),
            Err(_) => Err(store_err.into()),
        },
    }
}

#[allow(dead_code)]
pub fn save_accounts(data_dir: &Path, store: &AccountStore) -> Result<()> {
    let path = accounts_file_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut normalized = store.clone();
    normalized.normalize_v2();
    std::fs::write(&path, serde_json::to_string_pretty(&normalized)?)?;
    Ok(())
}

pub fn hydrate_account_defaults(store: &mut AccountStore) {
    for account in &mut store.accounts {
        if account.provider.is_empty() {
            account.provider = guess_provider(&account.upstream).to_string();
        }
        if account.provider_options.is_empty() {
            account.provider_options = provider_options_for_slug(&account.provider);
        }
    }
}

#[allow(dead_code)]
pub fn validate_capability_links(store: &AccountStore) -> Result<()> {
    for account in &store.accounts {
        if !account.capability_enabled {
            continue;
        }
        let Some(helper_id) = account.capability_account_id.as_deref() else {
            anyhow::bail!("账号 '{}' 已启用能力补全但未选择能力账号", account.name);
        };
        if helper_id == account.id {
            anyhow::bail!("账号 '{}' 的能力账号不能指向自身", account.name);
        }
        if !store
            .accounts
            .iter()
            .any(|candidate| candidate.id == helper_id)
        {
            anyhow::bail!("账号 '{}' 选择的能力账号不存在", account.name);
        }
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
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{:x}", ts)
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

        assert_eq!(kimi.default_upstream, "https://api.moonshot.ai/v1");
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
                "upstream": "https://api.moonshot.ai/v1",
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
            wire_protocol: Default::default(),
            upstream: "https://example.com/v1".into(),
            api_key: String::new(),
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
        };

        assert!(validate_capability_links(&store).is_err());
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
        };

        assert!(validate_capability_links(&store).is_ok());
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
            wire_protocol: Default::default(),
            upstream: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
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
        assert_eq!(endpoint.model_map["gpt-5"], "deepseek-chat");
        assert_eq!(endpoint.vision.mode, VisionMode::Off);
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
    fn legacy_responses_account_gets_responses_endpoint() {
        let mut account = legacy_account(false);
        account.normalize_v2();
        assert_eq!(account.endpoints[0].kind, EndpointKind::OpenAiResponses);
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
        };

        store.normalize_v2();

        let active = store.active_account().unwrap();
        assert_eq!(active.upstream, "https://api.openai.com/v1");
        assert!(!active.translate_enabled);
        assert_eq!(active.model_map["gpt-5"], "gpt-5");
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
