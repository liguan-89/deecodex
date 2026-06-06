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
    #[serde(default)]
    pub web_search_tool: bool,
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
            web_search_tool: false,
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
            vec![
                "deepseek-v4-pro[1m]",
                "deepseek-v4-pro",
                "deepseek-v4-flash",
                "deepseek-chat",
                "deepseek-reasoner",
            ],
            "DEEPSEEK_API_KEY",
            ProviderCapabilities {
                reasoning: ReasoningMode::DeepSeek,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "kimi",
            "Kimi",
            "Moonshot AI Kimi，OpenAI Chat Completions 兼容接口",
            "https://api.moonshot.cn/v1",
            vec![
                "kimi-k2.5",
                "kimi-k2-turbo-preview",
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
            "https://api.minimaxi.com/v1",
            vec![
                "MiniMax-M3",
                "MiniMax-M2.7",
                "MiniMax-M2.7-highspeed",
                "MiniMax-M2.5",
                "MiniMax-M2.5-highspeed",
                "MiniMax-M2.1",
                "MiniMax-M2.1-highspeed",
                "MiniMax-M2",
            ],
            "MINIMAX_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::DeepSeek,
                allow_missing_done: true,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "mimo",
            "MiMo",
            "小米 MiMo，支持 Anthropic 兼容接口与 OpenAI 兼容接口",
            "https://token-plan-cn.xiaomimimo.com/v1",
            vec!["mimo-v2.5-pro", "mimo-v2.5", "mimo-v2-omni", "mimo-v2-pro"],
            "MIMO_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::DeepSeek,
                web_search_tool: true,
                vision_input: true,
                allow_missing_done: true,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "longcat",
            "LongCat",
            "美团 LongCat，支持 Anthropic 格式与 OpenAI 兼容接口",
            "https://api.longcat.chat/openai",
            vec![
                "LongCat-2.0-Preview",
                "LongCat-Flash-Lite",
                "LongCat-Flash-Chat",
                "LongCat-Flash-Thinking-2601",
                "LongCat-Flash-Omni-2603",
            ],
            "LONGCAT_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::None,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "glm",
            "GLM",
            "智谱 GLM，OpenAI SDK 兼容接口",
            "https://open.bigmodel.cn/api/paas/v4",
            vec![
                "glm-5.1",
                "glm-5",
                "glm-4.7",
                "glm-4.6",
                "glm-4.5",
                "glm-4.5-air",
                "glm-4-flash",
            ],
            "ZHIPUAI_API_KEY",
            ProviderCapabilities {
                parallel_tool_calls: false,
                reasoning: ReasoningMode::None,
                ..Default::default()
            },
        ),
        ProviderProfile::chat(
            "qwen",
            "Qwen",
            "阿里云百炼 DashScope，OpenAI Chat Completions 兼容接口",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            vec![
                "qwen-max",
                "qwen-plus",
                "qwen-turbo",
                "qwen3-coder-plus",
                "qwen3-coder-flash",
                "qwen-long",
                "qwen-vl-plus",
                "qwen-vl-max",
            ],
            "DASHSCOPE_API_KEY",
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
            "codex",
            "Codex 官方",
            "ChatGPT Codex 官方 OAuth 后端，使用账号登录而不是 API Key",
            "https://chatgpt.com/backend-api/codex",
            vec![
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex",
                "gpt-5.3-codex-spark",
                "gpt-5.2",
                "gpt-image-2",
                "codex-auto-review",
            ],
            "OPENAI_CODEX_OAUTH",
            ProviderCapabilities {
                parallel_tool_calls: true,
                reasoning: ReasoningMode::OpenAi,
                vision_input: true,
                ..Default::default()
            },
        )
        .with_wire_protocol(WireProtocol::Responses),
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
    } else if upstream.contains("xiaomimimo.com") || upstream.contains("mimo-v2.com") {
        "mimo"
    } else if upstream.contains("longcat.chat") {
        "longcat"
    } else if upstream.contains("bigmodel.cn")
        || upstream.contains("zhipu")
        || upstream.contains("z.ai")
    {
        "glm"
    } else if upstream.contains("dashscope.aliyuncs.com") || upstream.contains("bailian") {
        "qwen"
    } else if upstream.contains("chatgpt.com/backend-api/codex") {
        "codex"
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
    } else if profile.slug == "deepseek"
        && req
            .response_format
            .as_ref()
            .and_then(|format| format.get("type"))
            .and_then(serde_json::Value::as_str)
            == Some("json_schema")
    {
        req.response_format = Some(serde_json::json!({"type": "json_object"}));
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
    if profile.slug == "minimax" {
        normalize_minimax_thinking(&mut req.thinking);
        if minimax_model_supports_reasoning_split(&req.model) {
            req.reasoning_split = Some(true);
        }
    } else if profile.slug == "mimo" {
        normalize_mimo_reasoning(req);
    }

    if caps.web_search_tool {
        adapt_web_search_tool(req);
    } else if !caps.web_search_options {
        req.web_search_options = None;
    }

    if profile.system_message_policy == SystemMessagePolicy::Merge {
        merge_extra_system_messages(&mut req.messages);
    }
}

fn normalize_minimax_thinking(thinking: &mut Option<serde_json::Value>) {
    let Some(serde_json::Value::Object(map)) = thinking else {
        return;
    };
    if map.get("type").and_then(serde_json::Value::as_str) == Some("enabled") {
        map.insert("type".into(), serde_json::Value::String("adaptive".into()));
    }
}

fn normalize_mimo_reasoning(req: &mut ChatRequest) {
    if req.reasoning_effort.as_deref() == Some("max") {
        req.reasoning_effort = Some("high".into());
    }
    let Some(serde_json::Value::Object(map)) = &mut req.thinking else {
        return;
    };
    let Some(kind) = map.get("type").and_then(serde_json::Value::as_str) else {
        return;
    };
    if kind != "enabled" && kind != "disabled" {
        map.insert("type".into(), serde_json::Value::String("enabled".into()));
    }
}

fn minimax_model_supports_reasoning_split(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("MiniMax-M3")
}

fn adapt_web_search_tool(req: &mut ChatRequest) {
    if req.web_search_options.take().is_none() {
        return;
    }
    if !has_web_search_tool(req) {
        req.tools.push(serde_json::json!({
            "type": "web_search",
            "max_keyword": 3,
            "force_search": true,
            "limit": 1
        }));
    }
}

pub fn has_web_search_tool(req: &ChatRequest) -> bool {
    req.tools.iter().any(is_web_search_tool)
}

pub fn strip_web_search_tool(req: &mut ChatRequest) -> bool {
    let before = req.tools.len();
    req.tools.retain(|tool| !is_web_search_tool(tool));
    req.tools.len() != before
}

pub fn is_mimo_web_search_disabled_error(status_code: u16, body: &str) -> bool {
    status_code == 400 && body.contains("webSearchEnabled") && body.contains("false")
}

fn is_web_search_tool(tool: &serde_json::Value) -> bool {
    tool.get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|typ| typ == "web_search")
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
    if profile.capabilities.web_search_options || profile.capabilities.web_search_tool {
        labels.push("联网扩展");
    }
    if profile.capabilities.vision_input {
        labels.push("原生多模态");
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
    let models: Vec<String> = match profile.model_discovery.response_shape {
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
    };
    if profile.slug == "mimo" {
        return models
            .into_iter()
            .filter(|model| !model.to_ascii_lowercase().contains("tts"))
            .collect();
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn guess_provider_recognizes_new_cn_providers() {
        assert_eq!(guess_provider("https://api.moonshot.cn/v1"), "kimi");
        assert_eq!(guess_provider("https://api.moonshot.ai/v1"), "kimi");
        assert_eq!(guess_provider("https://api.minimaxi.com/v1"), "minimax");
        assert_eq!(
            guess_provider("https://token-plan-cn.xiaomimimo.com/v1"),
            "mimo"
        );
        assert_eq!(guess_provider("https://api.mimo-v2.com/v1"), "mimo");
        assert_eq!(
            guess_provider("https://open.bigmodel.cn/api/paas/v4"),
            "glm"
        );
        assert_eq!(
            guess_provider("https://chatgpt.com/backend-api/codex"),
            "codex"
        );
    }

    #[test]
    fn provider_capabilities_keep_missing_done_provider_scoped() {
        assert!(!profile_by_slug("deepseek").capabilities.allow_missing_done);
        assert!(!profile_by_slug("kimi").capabilities.allow_missing_done);
        assert!(profile_by_slug("minimax").capabilities.allow_missing_done);
        assert!(profile_by_slug("mimo").capabilities.allow_missing_done);
        assert!(!profile_by_slug("glm").capabilities.allow_missing_done);
        assert!(!profile_by_slug("custom").capabilities.allow_missing_done);
    }

    #[test]
    fn minimax_profile_knows_m3_and_converts_thinking_to_adaptive() {
        let minimax = profile_by_slug("minimax");
        assert!(minimax
            .known_models
            .iter()
            .any(|model| model == "MiniMax-M3"));
        assert!(minimax
            .known_models
            .iter()
            .any(|model| model == "MiniMax-M2.7-highspeed"));
        assert_eq!(minimax.capabilities.reasoning, ReasoningMode::DeepSeek);

        let mut req = ChatRequest {
            model: "MiniMax-M3".into(),
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
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: Some(true),
            response_format: Some(json!({"type":"json_object"})),
            user: None,
            stream_options: Some(crate::types::StreamOptions {
                include_usage: true,
            }),
            web_search_options: Some(json!({"search_context": {}})),
        };

        adapt_chat_request(&minimax, &mut req);

        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(req.thinking, Some(json!({"type":"adaptive"})));
        assert_eq!(req.reasoning_split, Some(true));
        assert_eq!(req.parallel_tool_calls, None);
        assert_eq!(req.web_search_options, None);
        assert!(!req.tools.is_empty());
    }

    #[test]
    fn mimo_profile_uses_xiaomi_url_and_keeps_supported_reasoning_fields() {
        let mimo = profile_by_slug("mimo");
        assert_eq!(
            mimo.default_upstream,
            "https://token-plan-cn.xiaomimimo.com/v1"
        );
        assert_eq!(
            mimo.known_models,
            vec!["mimo-v2.5-pro", "mimo-v2.5", "mimo-v2-omni", "mimo-v2-pro"]
        );
        assert_eq!(mimo.capabilities.reasoning, ReasoningMode::DeepSeek);
        assert!(mimo.capabilities.web_search_tool);
        assert!(!mimo.capabilities.web_search_options);
        assert!(mimo.capabilities.vision_input);
        assert!(mimo.capabilities.allow_missing_done);

        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
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
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: Some(true),
            response_format: Some(json!({"type":"json_object"})),
            user: None,
            stream_options: Some(crate::types::StreamOptions {
                include_usage: true,
            }),
            web_search_options: Some(json!({"search_context": {}})),
        };

        adapt_chat_request(&mimo, &mut req);

        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(req.thinking, Some(json!({"type":"enabled"})));
        assert_eq!(req.reasoning_split, None);
        assert_eq!(req.parallel_tool_calls, None);
        assert_eq!(req.web_search_options, None);
        assert!(req.tools.iter().any(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(serde_json::Value::as_str)
                == Some("x")
        }));
        let web_search = req
            .tools
            .iter()
            .find(|tool| tool.get("type").and_then(serde_json::Value::as_str) == Some("web_search"))
            .unwrap();
        assert_eq!(web_search["max_keyword"], 3);
        assert_eq!(web_search["force_search"], true);
        assert_eq!(web_search["limit"], 1);
    }

    #[test]
    fn mimo_profile_keeps_thinking_but_clamps_unsupported_max_effort() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: Some("max".into()),
            thinking: Some(json!({"type":"adaptive"})),
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: Some(true),
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(req.thinking, Some(json!({"type":"enabled"})));
        assert_eq!(req.reasoning_split, None);
    }

    #[test]
    fn mimo_web_search_disabled_error_can_strip_web_tool() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![],
            tools: vec![
                json!({"type":"web_search","max_keyword":3,"force_search":true,"limit":1}),
                json!({"type":"function","function":{"name":"x","parameters":{"type":"object"}}}),
            ],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: None,
            thinking: None,
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        assert!(is_mimo_web_search_disabled_error(
            400,
            "web search tool found in the request body, but webSearchEnabled is false"
        ));
        assert!(has_web_search_tool(&req));
        assert!(strip_web_search_tool(&mut req));
        assert!(!has_web_search_tool(&req));
        assert_eq!(
            req.tools[0]
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("x")
        );
    }

    #[test]
    fn code_patch_tool_survives_deepseek_minimax_and_mimo_adaptation() {
        for slug in ["deepseek", "minimax", "mimo"] {
            let mut req = ChatRequest {
                model: match slug {
                    "minimax" => "MiniMax-M3".into(),
                    "mimo" => "mimo-v2.5-pro".into(),
                    _ => "deepseek-v4-pro".into(),
                },
                messages: vec![],
                tools: vec![json!({
                    "type": "function",
                    "function": {
                        "name": "apply_patch",
                        "description": "Apply code patch",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "patch": {"type": "string"}
                            },
                            "required": ["patch"]
                        }
                    }
                })],
                temperature: None,
                top_p: None,
                max_tokens: None,
                stream: true,
                reasoning_effort: Some("high".into()),
                thinking: Some(json!({"type":"enabled"})),
                reasoning_split: None,
                tool_choice: Some(json!({
                    "type": "function",
                    "function": {"name": "apply_patch"}
                })),
                parallel_tool_calls: Some(true),
                response_format: None,
                user: None,
                stream_options: Some(crate::types::StreamOptions {
                    include_usage: true,
                }),
                web_search_options: None,
            };

            adapt_chat_request(&profile_by_slug(slug), &mut req);

            assert!(
                req.tools.iter().any(|tool| {
                    tool.get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(serde_json::Value::as_str)
                        == Some("apply_patch")
                }),
                "{slug} 不应清除 apply_patch"
            );
            assert_eq!(
                req.tool_choice
                    .as_ref()
                    .and_then(|choice| choice.get("function"))
                    .and_then(|function| function.get("name"))
                    .and_then(serde_json::Value::as_str),
                Some("apply_patch"),
                "{slug} 不应清除 apply_patch tool_choice"
            );
        }
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
    fn codex_official_models_match_cliproxy_registry() {
        let codex = profile_by_slug("codex");
        assert_eq!(
            codex.known_models,
            vec![
                "gpt-5.5",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex",
                "gpt-5.3-codex-spark",
                "gpt-5.2",
                "gpt-image-2",
                "codex-auto-review",
            ]
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
            model_discovery_url(&kimi, "https://api.moonshot.cn/v1/", "sk").unwrap(),
            "https://api.moonshot.cn/v1/models"
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

        let mimo = profile_by_slug("mimo");
        let models = parse_models_response(
            &mimo,
            &json!({"data":[
                {"id":"mimo-v2-omni"},
                {"id":"mimo-v2.5-pro"},
                {"id":"mimo-v2.5-tts"},
                {"id":"mimo-v2.5-tts-voiceclone"}
            ]}),
        );
        assert_eq!(models, vec!["mimo-v2-omni", "mimo-v2.5-pro"]);

        let longcat = profile_by_slug("longcat");
        assert_eq!(
            model_discovery_url(&longcat, "https://api.longcat.chat/openai", "ak").unwrap(),
            "https://api.longcat.chat/openai/models"
        );
        assert_eq!(
            request_headers(&longcat, "ak"),
            vec![("authorization", "Bearer ak".to_string())]
        );
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
            reasoning_split: None,
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
    fn deepseek_profile_keeps_reasoning_and_drops_web_search() {
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
            reasoning_split: None,
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
        assert_eq!(req.web_search_options, None);
        assert_eq!(req.parallel_tool_calls, None);
    }

    #[test]
    fn deepseek_profile_downgrades_json_schema_response_format() {
        let mut req = ChatRequest {
            model: "deepseek-chat".into(),
            messages: vec![],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: None,
            thinking: None,
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: Some(json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "response",
                    "schema": {"type": "object"},
                    "strict": true
                }
            })),
            user: None,
            stream_options: Some(crate::types::StreamOptions {
                include_usage: true,
            }),
            web_search_options: None,
        };

        adapt_chat_request(&profile_by_slug("deepseek"), &mut req);

        assert_eq!(req.response_format, Some(json!({"type": "json_object"})));
    }
}
