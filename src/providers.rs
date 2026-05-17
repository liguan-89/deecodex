#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::ChatRequest;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WireProtocol {
    #[default]
    ChatCompletions,
    Responses,
    AnthropicMessages,
    GeminiNative,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningMode {
    #[default]
    None,
    DeepSeek,
    OpenAi,
    AnthropicThinking,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StreamUsageMode {
    #[default]
    FinalChunk,
    ResponseCompleted,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SystemMessagePolicy {
    Native,
    #[default]
    Merge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthScheme {
    #[default]
    Bearer,
    GeminiApiKeyQuery,
    AnthropicApiKey,
    Header,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelsResponseShape {
    #[default]
    OpenAiDataId,
    GeminiModelsName,
    AnthropicDataId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDiscovery {
    pub enabled: bool,
    pub endpoint: String,
    pub auth_scheme: AuthScheme,
    pub response_shape: ModelsResponseShape,
}

impl Default for ModelDiscovery {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: "models".into(),
            auth_scheme: AuthScheme::Bearer,
            response_shape: ModelsResponseShape::OpenAiDataId,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub tools: bool,
    pub parallel_tool_calls: bool,
    pub response_format: bool,
    pub stream_usage: StreamUsageMode,
    pub reasoning: ReasoningMode,
    pub web_search_options: bool,
    pub vision_input: bool,
    pub allow_missing_done: bool,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            tools: true,
            parallel_tool_calls: false,
            response_format: true,
            stream_usage: StreamUsageMode::FinalChunk,
            reasoning: ReasoningMode::None,
            web_search_options: false,
            vision_input: false,
            allow_missing_done: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub slug: String,
    pub label: String,
    pub description: String,
    pub default_upstream: String,
    pub known_models: Vec<String>,
    pub default_api_key_env: String,
    pub wire_protocol: WireProtocol,
    pub system_message_policy: SystemMessagePolicy,
    pub auth_scheme: AuthScheme,
    pub model_discovery: ModelDiscovery,
    pub capabilities: ProviderCapabilities,
}

impl ProviderProfile {
    fn chat(
        slug: &str,
        label: &str,
        description: &str,
        default_upstream: &str,
        known_models: Vec<&str>,
        default_api_key_env: &str,
        capabilities: ProviderCapabilities,
    ) -> Self {
        Self {
            slug: slug.into(),
            label: label.into(),
            description: description.into(),
            default_upstream: default_upstream.into(),
            known_models: known_models.into_iter().map(str::to_string).collect(),
            default_api_key_env: default_api_key_env.into(),
            wire_protocol: WireProtocol::ChatCompletions,
            system_message_policy: SystemMessagePolicy::Merge,
            auth_scheme: AuthScheme::Bearer,
            model_discovery: ModelDiscovery::default(),
            capabilities,
        }
    }

    fn with_wire_protocol(mut self, wire_protocol: WireProtocol) -> Self {
        self.wire_protocol = wire_protocol;
        self
    }

    fn with_auth_scheme(mut self, auth_scheme: AuthScheme) -> Self {
        self.auth_scheme = auth_scheme.clone();
        self.model_discovery.auth_scheme = auth_scheme;
        self
    }

    fn with_model_discovery(mut self, endpoint: &str, response_shape: ModelsResponseShape) -> Self {
        self.model_discovery.endpoint = endpoint.into();
        self.model_discovery.response_shape = response_shape;
        self
    }
}

pub fn get_provider_profiles() -> Vec<ProviderProfile> {
    vec![
        ProviderProfile::chat(
            "openrouter",
            "OpenRouter",
            "多模型聚合平台，按量计费，支持 Claude/OpenAI/DeepSeek 等数百种模型",
            "https://openrouter.ai/api/v1",
            vec![
                "deepseek/deepseek-chat",
                "deepseek/deepseek-reasoner",
                "anthropic/claude-sonnet-4.5",
                "anthropic/claude-opus-4.5",
                "openai/gpt-5.3-codex",
                "openai/gpt-5",
                "meta-llama/llama-4-maverick",
            ],
            "OPENROUTER_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: true,
                reasoning: ReasoningMode::OpenAi,
                web_search_options: false,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "deepseek",
            "DeepSeek",
            "深度求索，高性价比的中国 LLM 提供商",
            "https://api.deepseek.com/v1",
            vec!["deepseek-chat", "deepseek-reasoner"],
            "DEEPSEEK_API_KEY",
            ProviderCapabilities {
                reasoning: ReasoningMode::DeepSeek,
                web_search_options: true,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "kimi",
            "Kimi",
            "Moonshot AI Kimi，OpenAI Chat Completions 兼容接口",
            "https://api.moonshot.ai/v1",
            vec![
                "kimi-k2-0711-preview",
                "moonshot-v1-8k",
                "moonshot-v1-32k",
                "moonshot-v1-128k",
            ],
            "MOONSHOT_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::None,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "minimax",
            "MiniMax",
            "MiniMax Chat Completions/OpenAI 兼容接口",
            "https://api.minimax.io/v1",
            vec!["MiniMax-M1", "MiniMax-Text-01", "abab6.5s-chat"],
            "MINIMAX_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::None,
                allow_missing_done: true,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "glm",
            "GLM",
            "智谱 GLM，OpenAI SDK 兼容接口",
            "https://open.bigmodel.cn/api/paas/v4",
            vec!["glm-4.6", "glm-4.5", "glm-4.5-air", "glm-4-flash"],
            "ZHIPUAI_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::None,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "openai",
            "OpenAI",
            "OpenAI 官方 API，提供 GPT 系列模型",
            "https://api.openai.com/v1",
            vec![
                "gpt-5.3-codex",
                "gpt-5",
                "gpt-4.1",
                "gpt-4.1-mini",
                "gpt-4.1-nano",
            ],
            "OPENAI_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: true,
                reasoning: ReasoningMode::OpenAi,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "anthropic",
            "Anthropic",
            "Anthropic 官方 API，支持非流式 Messages 原生协议",
            "https://api.anthropic.com/v1",
            vec!["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"],
            "ANTHROPIC_API_KEY",
            ProviderCapabilities {
                tools: false,
                response_format: false,
                stream_usage: StreamUsageMode::Unavailable,
                reasoning: ReasoningMode::None,
                ..Default::default()
            },
        )
        .with_wire_protocol(WireProtocol::AnthropicMessages)
        .with_auth_scheme(AuthScheme::AnthropicApiKey)
        .with_model_discovery("models", ModelsResponseShape::AnthropicDataId),
        ProviderProfile::chat(
            "google-ai",
            "Google AI",
            "Google AI Studio，支持非流式 Gemini 原生协议",
            "https://generativelanguage.googleapis.com/v1beta",
            vec!["gemini-2.0-flash"],
            "GEMINI_API_KEY",
            ProviderCapabilities {
                tools: false,
                response_format: false,
                stream_usage: StreamUsageMode::Unavailable,
                ..Default::default()
            },
        )
        .with_wire_protocol(WireProtocol::GeminiNative)
        .with_auth_scheme(AuthScheme::GeminiApiKeyQuery)
        .with_model_discovery("models", ModelsResponseShape::GeminiModelsName),
        ProviderProfile::chat(
            "custom",
            "自定义",
            "手动配置 OpenAI Chat Completions 兼容上游",
            "",
            vec![],
            "",
            ProviderCapabilities::default(),
        ),
    ]
}

pub fn profile_by_slug(slug: &str) -> ProviderProfile {
    get_provider_profiles()
        .into_iter()
        .find(|p| p.slug == slug)
        .unwrap_or_else(|| {
            let mut custom = get_provider_profiles()
                .into_iter()
                .find(|p| p.slug == "custom")
                .expect("custom provider profile must exist");
            custom.slug = slug.to_string();
            custom
        })
}

pub fn profile_for_account(account: &crate::accounts::Account) -> ProviderProfile {
    let mut profile = profile_by_slug(&account.provider);
    profile.wire_protocol = account.wire_protocol.clone();
    profile
}

pub fn guess_provider(upstream: &str) -> &str {
    if upstream.contains("deepseek.com") {
        "deepseek"
    } else if upstream.contains("openrouter.ai") {
        "openrouter"
    } else if upstream.contains("api.moonshot.ai") || upstream.contains("moonshot.cn") {
        "kimi"
    } else if upstream.contains("minimax") {
        "minimax"
    } else if upstream.contains("bigmodel.cn") || upstream.contains("zhipu") {
        "glm"
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

pub fn adapt_chat_request(profile: &ProviderProfile, req: &mut ChatRequest) {
    let caps = &profile.capabilities;

    if !caps.tools {
        req.tools.clear();
        req.tool_choice = None;
        req.parallel_tool_calls = None;
    } else if !caps.parallel_tool_calls {
        req.parallel_tool_calls = None;
    }

    if !caps.response_format {
        req.response_format = None;
    }

    if caps.stream_usage == StreamUsageMode::Unavailable {
        req.stream_options = None;
    }

    match caps.reasoning {
        ReasoningMode::DeepSeek => {}
        ReasoningMode::OpenAi => {
            req.thinking = None;
        }
        ReasoningMode::AnthropicThinking | ReasoningMode::None => {
            req.reasoning_effort = None;
            req.thinking = None;
        }
    }

    if !caps.web_search_options {
        req.web_search_options = None;
    }

    if profile.system_message_policy == SystemMessagePolicy::Merge {
        merge_extra_system_messages(&mut req.messages);
    }
}

fn merge_extra_system_messages(messages: &mut Vec<crate::types::ChatMessage>) {
    let Some(first) = messages.iter().position(|m| m.role == "system") else {
        return;
    };
    for idx in (first + 1..messages.len()).rev() {
        if messages[idx].role == "system" {
            let removed = messages.remove(idx);
            if let Some(serde_json::Value::String(text)) = removed.content {
                if let Some(serde_json::Value::String(target)) = &mut messages[first].content {
                    target.push_str("\n\n");
                    target.push_str(&text);
                }
            }
        }
    }
}

pub fn capability_labels(profile: &ProviderProfile) -> Vec<&'static str> {
    let mut labels = vec![match profile.wire_protocol {
        WireProtocol::ChatCompletions => "Chat 兼容",
        WireProtocol::Responses => "Responses 直连",
        WireProtocol::AnthropicMessages => "Anthropic 原生",
        WireProtocol::GeminiNative => "Gemini 原生",
    }];
    if profile.capabilities.tools {
        labels.push("工具调用");
    }
    if profile.capabilities.reasoning != ReasoningMode::None {
        labels.push("推理字段");
    }
    if profile.capabilities.web_search_options {
        labels.push("联网扩展");
    }
    if profile.capabilities.allow_missing_done {
        labels.push("流式容错");
    }
    labels
}

pub fn provider_options_for_slug(slug: &str) -> HashMap<String, serde_json::Value> {
    let profile = profile_by_slug(slug);
    HashMap::from([(
        "capability_labels".to_string(),
        serde_json::json!(capability_labels(&profile)),
    )])
}

pub fn model_discovery_url(
    profile: &ProviderProfile,
    upstream: &str,
    api_key: &str,
) -> Option<String> {
    if !profile.model_discovery.enabled {
        return None;
    }
    let base = upstream.trim_end_matches('/');
    let endpoint = profile.model_discovery.endpoint.trim_start_matches('/');
    let mut url = format!("{base}/{endpoint}");
    if profile.model_discovery.auth_scheme == AuthScheme::GeminiApiKeyQuery && !api_key.is_empty() {
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str("key=");
        url.push_str(api_key);
    }
    Some(url)
}

pub fn auth_header(profile: &ProviderProfile, api_key: &str) -> Option<(&'static str, String)> {
    if api_key.is_empty() {
        return None;
    }
    match profile.auth_scheme {
        AuthScheme::Bearer => Some(("authorization", format!("Bearer {api_key}"))),
        AuthScheme::AnthropicApiKey => Some(("x-api-key", api_key.to_string())),
        AuthScheme::Header => Some(("authorization", api_key.to_string())),
        AuthScheme::GeminiApiKeyQuery | AuthScheme::None => None,
    }
}

pub fn request_headers(profile: &ProviderProfile, api_key: &str) -> Vec<(&'static str, String)> {
    let mut headers = Vec::new();
    if let Some(header) = auth_header(profile, api_key) {
        headers.push(header);
    }
    if profile.auth_scheme == AuthScheme::AnthropicApiKey {
        headers.push(("anthropic-version", "2023-06-01".to_string()));
    }
    headers
}

pub fn parse_models_response(profile: &ProviderProfile, body: &serde_json::Value) -> Vec<String> {
    match profile.model_discovery.response_shape {
        ModelsResponseShape::OpenAiDataId | ModelsResponseShape::AnthropicDataId => body["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        ModelsResponseShape::GeminiModelsName => body["models"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m["name"].as_str())
                    .map(|name| name.strip_prefix("models/").unwrap_or(name).to_string())
                    .collect()
            })
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn guess_provider_recognizes_new_cn_providers() {
        assert_eq!(guess_provider("https://api.moonshot.ai/v1"), "kimi");
        assert_eq!(guess_provider("https://api.minimax.io/v1"), "minimax");
        assert_eq!(
            guess_provider("https://open.bigmodel.cn/api/paas/v4"),
            "glm"
        );
    }

    #[test]
    fn provider_capabilities_keep_missing_done_provider_scoped() {
        assert!(!profile_by_slug("deepseek").capabilities.allow_missing_done);
        assert!(!profile_by_slug("kimi").capabilities.allow_missing_done);
        assert!(profile_by_slug("minimax").capabilities.allow_missing_done);
        assert!(!profile_by_slug("glm").capabilities.allow_missing_done);
        assert!(!profile_by_slug("custom").capabilities.allow_missing_done);
    }

    #[test]
    fn native_protocol_profiles_are_enabled() {
        assert_eq!(
            profile_by_slug("anthropic").wire_protocol,
            WireProtocol::AnthropicMessages
        );
        assert_eq!(
            profile_by_slug("google-ai").wire_protocol,
            WireProtocol::GeminiNative
        );
    }

    #[test]
    fn model_discovery_uses_provider_auth_and_response_shape() {
        let gemini = profile_by_slug("google-ai");
        let url = model_discovery_url(
            &gemini,
            "https://generativelanguage.googleapis.com/v1beta",
            "gemini-key",
        )
        .unwrap();
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models?key=gemini-key"
        );
        assert_eq!(auth_header(&gemini, "gemini-key"), None);
        assert!(request_headers(&gemini, "gemini-key").is_empty());
        let models = parse_models_response(
            &gemini,
            &json!({"models":[{"name":"models/gemini-2.0-flash"},{"name":"models/gemini-pro"}]}),
        );
        assert_eq!(models, vec!["gemini-2.0-flash", "gemini-pro"]);

        let anthropic = profile_by_slug("anthropic");
        assert_eq!(
            request_headers(&anthropic, "anthropic-key"),
            vec![
                ("x-api-key", "anthropic-key".to_string()),
                ("anthropic-version", "2023-06-01".to_string())
            ]
        );

        let kimi = profile_by_slug("kimi");
        assert_eq!(
            model_discovery_url(&kimi, "https://api.moonshot.ai/v1/", "sk").unwrap(),
            "https://api.moonshot.ai/v1/models"
        );
        assert_eq!(
            auth_header(&kimi, "sk"),
            Some(("authorization", "Bearer sk".to_string()))
        );
        assert_eq!(
            request_headers(&kimi, "sk"),
            vec![("authorization", "Bearer sk".to_string())]
        );
        let models = parse_models_response(&kimi, &json!({"data":[{"id":"moonshot-v1-8k"}]}));
        assert_eq!(models, vec!["moonshot-v1-8k"]);
    }

    #[test]
    fn kimi_profile_strips_deepseek_only_fields() {
        let mut req = ChatRequest {
            model: "kimi-k2".into(),
            messages: vec![],
            tools: vec![
                json!({"type":"function","function":{"name":"x","parameters":{"type":"object"}}}),
            ],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: Some("high".into()),
            thinking: Some(json!({"type":"enabled"})),
            tool_choice: None,
            parallel_tool_calls: Some(true),
            response_format: Some(json!({"type":"json_object"})),
            user: None,
            stream_options: Some(crate::types::StreamOptions {
                include_usage: true,
            }),
            web_search_options: Some(json!({"search_context": {}})),
        };
        adapt_chat_request(&profile_by_slug("kimi"), &mut req);
        assert_eq!(req.reasoning_effort, None);
        assert_eq!(req.thinking, None);
        assert_eq!(req.parallel_tool_calls, None);
        assert_eq!(req.web_search_options, None);
        assert!(!req.tools.is_empty());
    }

    #[test]
    fn deepseek_profile_keeps_reasoning_and_web_search() {
        let mut req = ChatRequest {
            model: "deepseek-reasoner".into(),
            messages: vec![],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: Some("high".into()),
            thinking: Some(json!({"type":"enabled"})),
            tool_choice: None,
            parallel_tool_calls: Some(true),
            response_format: None,
            user: None,
            stream_options: Some(crate::types::StreamOptions {
                include_usage: true,
            }),
            web_search_options: Some(json!({"search_context": {}})),
        };
        adapt_chat_request(&profile_by_slug("deepseek"), &mut req);
        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
        assert!(req.thinking.is_some());
        assert!(req.web_search_options.is_some());
        assert_eq!(req.parallel_tool_calls, None);
    }
}
