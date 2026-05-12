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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPreset {
    pub slug: String,
    pub label: String,
    pub description: String,
    pub default_upstream: String,
    pub known_models: Vec<String>,
    pub default_api_key_env: String,
}

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
#[allow(dead_code)]
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

pub fn save_accounts(data_dir: &Path, store: &AccountStore) -> Result<()> {
    let path = accounts_file_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(store)?)?;
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
