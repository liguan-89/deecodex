use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── 数据模型 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub provider: String,
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
}

#[allow(dead_code)]
pub fn get_provider_presets() -> Vec<ProviderPreset> {
    vec![
        ProviderPreset {
            slug: "openrouter".into(),
            label: "OpenRouter".into(),
            description: "多模型聚合平台，按量计费，支持 Claude/OpenAI/DeepSeek 等数百种模型"
                .into(),
            default_upstream: "https://openrouter.ai/api/v1".into(),
            known_models: vec![
                "deepseek/deepseek-chat".into(),
                "deepseek/deepseek-reasoner".into(),
                "anthropic/claude-sonnet-4.5".into(),
                "anthropic/claude-opus-4.5".into(),
                "openai/gpt-5.3-codex".into(),
                "openai/gpt-5".into(),
                "meta-llama/llama-4-maverick".into(),
            ],
            default_api_key_env: "OPENROUTER_API_KEY".into(),
        },
        ProviderPreset {
            slug: "deepseek".into(),
            label: "DeepSeek".into(),
            description: "深度求索，高性价比的中国 LLM 提供商".into(),
            default_upstream: "https://api.deepseek.com/v1".into(),
            known_models: vec!["deepseek-chat".into(), "deepseek-reasoner".into()],
            default_api_key_env: "DEEPSEEK_API_KEY".into(),
        },
        ProviderPreset {
            slug: "openai".into(),
            label: "OpenAI".into(),
            description: "OpenAI 官方 API，提供 GPT 系列模型".into(),
            default_upstream: "https://api.openai.com/v1".into(),
            known_models: vec![
                "gpt-5.3-codex".into(),
                "gpt-5".into(),
                "gpt-4.1".into(),
                "gpt-4.1-mini".into(),
                "gpt-4.1-nano".into(),
            ],
            default_api_key_env: "OPENAI_API_KEY".into(),
        },
        ProviderPreset {
            slug: "anthropic".into(),
            label: "Anthropic".into(),
            description: "Anthropic 官方 API，提供 Claude 系列模型".into(),
            default_upstream: "https://api.anthropic.com/v1".into(),
            known_models: vec![
                "claude-sonnet-4-5".into(),
                "claude-opus-4-5".into(),
                "claude-haiku-4-5".into(),
                "claude-3-5-haiku".into(),
            ],
            default_api_key_env: "ANTHROPIC_API_KEY".into(),
        },
        ProviderPreset {
            slug: "google-ai".into(),
            label: "Google AI".into(),
            description: "Google AI Studio，提供 Gemini 系列模型，有免费额度".into(),
            default_upstream: "https://generativelanguage.googleapis.com/v1beta".into(),
            known_models: vec!["gemini-2.0-flash".into()],
            default_api_key_env: "GEMINI_API_KEY".into(),
        },
        ProviderPreset {
            slug: "custom".into(),
            label: "自定义".into(),
            description: "手动配置上游 URL、API Key 和模型列表".into(),
            default_upstream: String::new(),
            known_models: vec![],
            default_api_key_env: String::new(),
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
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
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
    if upstream.contains("deepseek.com") {
        "deepseek"
    } else if upstream.contains("openrouter.ai") {
        "openrouter"
    } else if upstream.contains("api.openai.com") {
        "openai"
    } else if upstream.contains("anthropic.com") {
        "anthropic"
    } else if upstream.contains("generativelanguage.googleapis.com") {
        "google-ai"
    } else {
        "custom"
    }
}

#[cfg(test)]
mod tests {
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
