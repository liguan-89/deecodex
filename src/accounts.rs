use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::providers::{
    get_provider_profiles, provider_options_for_slug, AuthScheme, ModelDiscovery,
    ProviderCapabilities, WireProtocol,
};

// ── 数据模型 ────────────────────────────────────────────────────────────────

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
}

fn default_translate_enabled() -> bool {
    true
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountStore {
    pub accounts: Vec<Account>,
    pub active_id: Option<String>,
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
            let mut store: AccountStore = serde_json::from_str(&content).unwrap_or_default();
            hydrate_account_defaults(&mut store);
            store
        }
        Err(_) => AccountStore::default(),
    }
}

#[allow(dead_code)]
pub fn save_accounts(data_dir: &Path, store: &AccountStore) -> Result<()> {
    let path = accounts_file_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(store)?)?;
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
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
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
            accounts: vec![account],
            active_id: Some("a1".into()),
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
            accounts: vec![main, helper],
            active_id: Some("a1".into()),
        };

        assert!(validate_capability_links(&store).is_ok());
    }
}
