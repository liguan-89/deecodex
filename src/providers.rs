#![allow(dead_code)]

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{ChatMessage, ChatRequest};

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
        append_tool_execution_guard(req, "MiniMax");
        enforce_min_tool_calls(req, "MiniMax");
    } else if profile.slug == "mimo" {
        normalize_mimo_reasoning(req);
        append_tool_execution_guard(req, "MiMo");
        enforce_min_tool_calls(req, "MiMo");
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

const TOOL_EXECUTION_GUARD_MARKER: &str = "工具调用稳定性约束";
const MIN_TOOL_CALL_GUARD_MARKER: &str = "最小工具调用计数约束";
const TOOLCHAIN_COVERAGE_GUARD_MARKER: &str = "工具链覆盖完整性约束";
const TOOLCHAIN_FINAL_REPORT_GUARD_MARKER: &str = "工具链最终报告完整性约束";
fn tool_execution_guard(label: &str) -> String {
    format!(
        "【{label} {TOOL_EXECUTION_GUARD_MARKER}】当用户请求执行工具链、测试工具、读写文件、下载/生成文件、编译运行、查看结果、修复输出或连续步骤时，如果还存在未完成的工具步骤，必须直接继续发起下一次工具调用；不要只输出阶段标题、步骤说明或总结后结束。凡是出现“让我运行/让我看看/我来修复/现在生成/接下来执行”这类承诺下一步的表述，必须在同一条 assistant 响应中携带实际 tool_calls/function_call。只有确认所有必要工具调用都已完成，并且失败命令后的恢复/验证也完成后，才可以输出最终总结。"
    )
}

fn append_tool_execution_guard(req: &mut ChatRequest, label: &str) {
    if req.tools.is_empty() {
        return;
    }
    let guard = tool_execution_guard(label);
    if req.messages.iter().any(|msg| {
        msg.role == "system"
            && msg
                .content
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains(&guard))
    }) {
        return;
    }

    if let Some(system) = req.messages.iter_mut().find(|msg| msg.role == "system") {
        match &mut system.content {
            Some(serde_json::Value::String(text)) => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(&guard);
            }
            Some(serde_json::Value::Array(parts)) => {
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": guard
                }));
            }
            _ => {
                system.content = Some(serde_json::Value::String(guard));
            }
        }
        return;
    }

    req.messages.insert(
        0,
        ChatMessage {
            role: "system".into(),
            content: Some(serde_json::Value::String(guard)),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    );
}

fn enforce_min_tool_calls(req: &mut ChatRequest, label: &str) {
    if req.tools.is_empty() {
        return;
    }
    let pending_minimum = pending_min_tool_calls(&req.messages);
    let pending_coverage = pending_toolchain_coverage(&req.messages);

    if let Some((completed, required)) = pending_minimum {
        append_min_tool_call_guard(req, label, completed, required);
        if can_override_tool_choice(&req.tool_choice) {
            req.tool_choice = Some(serde_json::Value::String("required".into()));
        }
    }
    if let Some(coverage) = pending_coverage.as_ref() {
        append_toolchain_coverage_guard(req, label, &coverage);
        if can_override_tool_choice(&req.tool_choice) {
            req.tool_choice = Some(serde_json::Value::String("required".into()));
        }
    }
    if pending_minimum.is_none()
        && pending_coverage.is_none()
        && pending_toolchain_final_report(&req.messages)
    {
        append_toolchain_final_report_guard(req, label);
    }
}

fn append_min_tool_call_guard(
    req: &mut ChatRequest,
    label: &str,
    completed: usize,
    required: usize,
) {
    let guard = format!(
        "【{label} {MIN_TOOL_CALL_GUARD_MARKER}】用户明确要求至少 {required} 次真实工具调用；当前只收到 {completed}/{required} 次工具输出。下一条 assistant 响应必须包含实际 tool_calls/function_call，不能只输出阶段标题、进度说明、测试报告或总结；未达到 {required} 次前不得结束。"
    );

    req.messages.retain(|msg| {
        !chat_message_text(msg.content.as_ref()).contains(MIN_TOOL_CALL_GUARD_MARKER)
    });
    req.messages.push(ChatMessage {
        role: "user".into(),
        content: Some(serde_json::Value::String(guard)),
        reasoning_content: None,
        reasoning_details: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
}

fn append_toolchain_coverage_guard(
    req: &mut ChatRequest,
    label: &str,
    coverage: &ToolchainCoverageStatus,
) {
    let missing = coverage.missing.join("；");
    let guard = format!(
        "【{label} {TOOLCHAIN_COVERAGE_GUARD_MARKER}】用户正在做 Codex 工具链压力测试，当前实际覆盖仍不完整：{missing}。下一条 assistant 响应必须继续发起缺失项对应的真实 tool_calls/function_call，不得输出最终报告、不得声明通过、不得结束。"
    );

    req.messages.retain(|msg| {
        !chat_message_text(msg.content.as_ref()).contains(TOOLCHAIN_COVERAGE_GUARD_MARKER)
    });
    req.messages.push(ChatMessage {
        role: "user".into(),
        content: Some(serde_json::Value::String(guard)),
        reasoning_content: None,
        reasoning_details: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
}

fn append_toolchain_final_report_guard(req: &mut ChatRequest, label: &str) {
    let guard = format!(
        "【{label} {TOOLCHAIN_FINAL_REPORT_GUARD_MARKER}】工具链测试已经进入收尾阶段，但历史消息里还没有完整最终测试报告。下一条 assistant 响应必须直接输出完整报告，至少包含：实际工具调用总数、exec_command 次数、apply_patch 次数、read_thread_terminal 次数、tool_search 次数、失败工具调用次数和原因、unsupported call 检查、interrupted/response.failed/SSE parse error 检查、最终判定。不能只说准备输出报告，不能直接结束。"
    );

    req.messages.retain(|msg| {
        !chat_message_text(msg.content.as_ref()).contains(TOOLCHAIN_FINAL_REPORT_GUARD_MARKER)
    });
    req.messages.push(ChatMessage {
        role: "user".into(),
        content: Some(serde_json::Value::String(guard)),
        reasoning_content: None,
        reasoning_details: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
}

pub fn mimo_pending_min_tool_calls(messages: &[ChatMessage]) -> Option<(usize, usize)> {
    pending_min_tool_calls(messages)
}

pub fn pending_min_tool_calls(messages: &[ChatMessage]) -> Option<(usize, usize)> {
    let (user_index, required) = min_tool_call_requirement(messages)?;
    let completed = messages
        .iter()
        .skip(user_index + 1)
        .filter(|msg| msg.role == "tool" && msg.tool_call_id.is_some())
        .count();
    (completed < required).then_some((completed, required))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainCoverageStatus {
    pub missing: Vec<String>,
    pub next_recovery_tool: Option<ToolchainRecoveryTool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainRecoveryTool {
    ReadThreadTerminal,
    ToolSearchThread,
    ToolSearchBrowser,
    ApplyPatch {
        patch: String,
    },
    ExecCommand {
        cmd: String,
        workdir: Option<String>,
    },
    ExecCommandNoop,
}

pub fn pending_toolchain_coverage(messages: &[ChatMessage]) -> Option<ToolchainCoverageStatus> {
    let requirement = toolchain_coverage_requirement(messages)?;
    let observed = observed_toolchain_coverage(messages, requirement.user_index);
    let mut missing = Vec::new();

    if let Some(required) = requirement.min_total_tools {
        if observed.total_tools < required {
            missing.push(format!("总工具调用 {}/{}", observed.total_tools, required));
        }
    }
    if requirement.needs_read_thread_terminal && observed.read_thread_terminal < 1 {
        missing.push("read_thread_terminal 0/1".into());
    }
    if let Some(required) = requirement.min_tool_search {
        if observed.tool_search < required {
            missing.push(format!("tool_search {}/{}", observed.tool_search, required));
        }
    }
    if let Some(required) = requirement.min_successful_apply_patch {
        if observed.successful_apply_patch < required {
            missing.push(format!(
                "成功 apply_patch {}/{}",
                observed.successful_apply_patch, required
            ));
        }
    }

    let prompt_text = messages
        .get(requirement.user_index)
        .map(|msg| chat_message_text(msg.content.as_ref()))
        .unwrap_or_default();
    let next_recovery_tool =
        sequential_toolchain_recovery_tool(&prompt_text, &observed).or_else(|| {
            if requirement.needs_read_thread_terminal && observed.read_thread_terminal < 1 {
                Some(ToolchainRecoveryTool::ReadThreadTerminal)
            } else if requirement.min_tool_search.is_some() && observed.tool_search == 0 {
                Some(ToolchainRecoveryTool::ToolSearchThread)
            } else if requirement
                .min_tool_search
                .is_some_and(|required| observed.tool_search < required)
            {
                Some(ToolchainRecoveryTool::ToolSearchBrowser)
            } else if requirement
                .min_successful_apply_patch
                .is_some_and(|required| observed.successful_apply_patch < required)
            {
                apply_patch_recovery_tool(&prompt_text, observed.successful_apply_patch)
            } else if requirement
                .min_total_tools
                .is_some_and(|required| observed.total_tools < required)
            {
                Some(exec_recovery_tool(
                    "pwd".into(),
                    first_tmp_dir(&prompt_text),
                ))
            } else {
                None
            }
        });

    (!missing.is_empty()).then_some(ToolchainCoverageStatus {
        missing,
        next_recovery_tool,
    })
}

pub fn min_tool_call_recovery_tool(messages: &[ChatMessage]) -> Option<ToolchainRecoveryTool> {
    let (user_index, _required) = min_tool_call_requirement(messages)?;
    let observed = observed_toolchain_coverage(messages, user_index);
    let prompt_text = messages
        .get(user_index)
        .map(|msg| chat_message_text(msg.content.as_ref()))
        .unwrap_or_default();
    sequential_toolchain_recovery_tool(&prompt_text, &observed).or_else(|| {
        Some(exec_recovery_tool(
            "pwd".into(),
            first_tmp_dir(&prompt_text),
        ))
    })
}

pub fn toolchain_final_report_required(messages: &[ChatMessage]) -> bool {
    toolchain_final_report_user_index(messages).is_some()
}

pub fn pending_toolchain_final_report(messages: &[ChatMessage]) -> bool {
    let Some(user_index) = toolchain_final_report_user_index(messages) else {
        return false;
    };
    !messages.iter().skip(user_index + 1).any(|msg| {
        msg.role == "assistant"
            && complete_toolchain_final_report_text(&chat_message_text(msg.content.as_ref()))
    })
}

pub fn complete_toolchain_final_report_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let preparing_only = text.contains("准备出报告")
        || text.contains("准备输出报告")
        || text.contains("准备生成报告")
        || text.contains("准备补")
        || lower.contains("preparing report");
    if preparing_only {
        return false;
    }

    let has_report_title = text.contains("测试报告")
        || text.contains("最终报告")
        || lower.contains("test report")
        || lower.contains("final report");
    let has_total = text.contains("实际工具调用总数")
        || text.contains("工具调用总数")
        || lower.contains("tool call");
    let has_exec = lower.contains("exec_command");
    let has_patch = lower.contains("apply_patch");
    let has_terminal =
        lower.contains("read_thread_terminal") || lower.contains("codex_app.read_thread_terminal");
    let has_search = lower.contains("tool_search");
    let has_failure = text.contains("失败工具调用")
        || text.contains("失败次数")
        || lower.contains("failed tool")
        || lower.contains("failure");
    let has_unsupported = lower.contains("unsupported call");
    let has_stream_errors = lower.contains("interrupted")
        || lower.contains("response.failed")
        || lower.contains("sse parse error");
    let has_verdict = text.contains("最终判定")
        && (text.contains("通过") || text.contains("不通过") || lower.contains("pass"));

    has_report_title
        && has_total
        && has_exec
        && has_patch
        && has_terminal
        && has_search
        && has_failure
        && has_unsupported
        && has_stream_errors
        && has_verdict
}

pub fn final_report_recovery_tool(messages: &[ChatMessage]) -> ToolchainRecoveryTool {
    let workdir = toolchain_final_report_user_index(messages)
        .and_then(|index| messages.get(index))
        .map(|msg| chat_message_text(msg.content.as_ref()))
        .and_then(|text| first_tmp_dir(&text));
    exec_recovery_tool("pwd".into(), workdir)
}

struct ToolchainCoverageRequirement {
    user_index: usize,
    min_total_tools: Option<usize>,
    needs_read_thread_terminal: bool,
    min_tool_search: Option<usize>,
    min_successful_apply_patch: Option<usize>,
}

#[derive(Default)]
struct ObservedToolchainCoverage {
    total_tools: usize,
    read_thread_terminal: usize,
    tool_search: usize,
    successful_apply_patch: usize,
}

fn toolchain_coverage_requirement(
    messages: &[ChatMessage],
) -> Option<ToolchainCoverageRequirement> {
    messages.iter().enumerate().rev().find_map(|(index, msg)| {
        if msg.role != "user" {
            return None;
        }
        let text = chat_message_text(msg.content.as_ref());
        if text.contains(MIN_TOOL_CALL_GUARD_MARKER)
            || text.contains(TOOLCHAIN_COVERAGE_GUARD_MARKER)
        {
            return None;
        }
        let lower = text.to_ascii_lowercase();
        let toolchain_test = text.contains("压力测试")
            || text.contains("稳定性测试")
            || text.contains("兼容性测试")
            || lower.contains("stress test")
            || lower.contains("stability test")
            || lower.contains("compatibility test");
        let pressure_test =
            toolchain_test && (text.contains("工具链") || lower.contains("toolchain"));
        if !pressure_test {
            return None;
        }
        let min_total_tools = min_tool_calls_from_text(&text);
        let needs_read_thread_terminal = text.contains("read_thread_terminal");
        let min_tool_search = text.contains("tool_search").then_some(2);
        let min_successful_apply_patch = (text.contains("apply_patch")
            && (text.contains("至少 2") || text.contains("至少2") || lower.contains("at least 2")))
        .then_some(2);
        if min_total_tools.is_none()
            && !needs_read_thread_terminal
            && min_tool_search.is_none()
            && min_successful_apply_patch.is_none()
        {
            return None;
        }
        Some(ToolchainCoverageRequirement {
            user_index: index,
            min_total_tools,
            needs_read_thread_terminal,
            min_tool_search,
            min_successful_apply_patch,
        })
    })
}

fn toolchain_final_report_user_index(messages: &[ChatMessage]) -> Option<usize> {
    messages.iter().enumerate().rev().find_map(|(index, msg)| {
        if msg.role != "user" {
            return None;
        }
        let text = chat_message_text(msg.content.as_ref());
        if text.contains(MIN_TOOL_CALL_GUARD_MARKER)
            || text.contains(TOOLCHAIN_COVERAGE_GUARD_MARKER)
            || text.contains(TOOLCHAIN_FINAL_REPORT_GUARD_MARKER)
        {
            return None;
        }
        let lower = text.to_ascii_lowercase();
        let asks_report = text.contains("最终报告")
            || text.contains("测试报告")
            || lower.contains("final report")
            || lower.contains("test report");
        let toolchain_test = text.contains("工具链")
            || lower.contains("toolchain")
            || text.contains("工具调用")
            || lower.contains("tool call");
        (asks_report && toolchain_test).then_some(index)
    })
}

fn sequential_toolchain_recovery_tool(
    prompt_text: &str,
    observed: &ObservedToolchainCoverage,
) -> Option<ToolchainRecoveryTool> {
    let lower = prompt_text.to_ascii_lowercase();
    let sequential = prompt_text.contains("必须按顺序执行")
        || prompt_text.contains("低并发顺序执行")
        || lower.contains("sequential");
    if !sequential {
        return None;
    }

    let workdir = first_tmp_dir(prompt_text);
    let dir = workdir
        .clone()
        .unwrap_or_else(|| "/tmp/codex-toolchain-recovery".to_string());
    let file1 = format!("{dir}/file1.txt");
    let file2 = format!("{dir}/file2.txt");
    let file3 = format!("{dir}/file3.txt");
    let probe = format!("{dir}/probe.rs");
    let file1_content = file_content_from_prompt(prompt_text, "file1.txt", "minimax m3 alpha");
    let file2_content = file_content_from_prompt(prompt_text, "file2.txt", "minimax m3 beta");
    let file3_content = file_content_from_prompt(prompt_text, "file3.txt", "minimax m3 gamma");

    match observed.total_tools {
        0 => Some(exec_recovery_tool(
            format!("mkdir -p {}", shell_quote(&dir)),
            None,
        )),
        1 => Some(exec_recovery_tool(
            format!(
                "printf '%s\\n' {} > {}",
                shell_quote(&file1_content),
                shell_quote(&file1)
            ),
            workdir,
        )),
        2 => Some(exec_recovery_tool(
            format!(
                "printf '%s\\n' {} > {}",
                shell_quote(&file2_content),
                shell_quote(&file2)
            ),
            workdir,
        )),
        3 => Some(exec_recovery_tool(
            format!(
                "printf '%s\\n' {} > {}",
                shell_quote(&file3_content),
                shell_quote(&file3)
            ),
            workdir,
        )),
        4 => Some(exec_recovery_tool(
            format!("cat {}", shell_quote(&file1)),
            workdir,
        )),
        5 => Some(exec_recovery_tool(
            format!("cat {}", shell_quote(&file2)),
            workdir,
        )),
        6 => Some(exec_recovery_tool(
            format!("cat {}", shell_quote(&file3)),
            workdir,
        )),
        7 => Some(exec_recovery_tool(
            format!("rg minimax {}", shell_quote(&dir)),
            workdir,
        )),
        8 => apply_patch_recovery_tool(prompt_text, observed.successful_apply_patch),
        9 => apply_patch_recovery_tool(prompt_text, observed.successful_apply_patch.max(1)),
        10 => Some(exec_recovery_tool(
            format!("rg PATCH_OK {}", shell_quote(&dir)),
            workdir,
        )),
        11 => Some(ToolchainRecoveryTool::ReadThreadTerminal),
        12 => Some(ToolchainRecoveryTool::ToolSearchThread),
        13 => Some(ToolchainRecoveryTool::ToolSearchBrowser),
        14 => Some(exec_recovery_tool(
            format!(
                "printf '%s\\n' {} > {}",
                shell_quote("fn main() { println!(\"MINIMAX_M3_PROBE_OK\"); }"),
                shell_quote(&probe)
            ),
            workdir,
        )),
        15 => Some(exec_recovery_tool("rustc --version".into(), workdir)),
        16 => Some(exec_recovery_tool(
            format!(
                "rustc {} -o {}",
                shell_quote(&probe),
                shell_quote(&format!("{dir}/probe"))
            ),
            workdir,
        )),
        17 => Some(exec_recovery_tool("./probe".into(), workdir)),
        18 => Some(exec_recovery_tool("bad-command".into(), workdir)),
        19 => Some(exec_recovery_tool("ls -la".into(), workdir)),
        20 => Some(exec_recovery_tool(
            "wc -l file1.txt file2.txt file3.txt".into(),
            workdir,
        )),
        21 => Some(exec_recovery_tool("grep -R PATCH_OK .".into(), workdir)),
        _ => None,
    }
}

fn apply_patch_recovery_tool(
    prompt_text: &str,
    successful_apply_patch: usize,
) -> Option<ToolchainRecoveryTool> {
    let dir =
        first_tmp_dir(prompt_text).unwrap_or_else(|| "/tmp/codex-toolchain-recovery".to_string());
    let (path, content) = if successful_apply_patch == 0 {
        (
            format!("{dir}/file1.txt"),
            file_content_from_prompt(prompt_text, "file1.txt", "minimax m3 alpha"),
        )
    } else {
        (
            format!("{dir}/file2.txt"),
            file_content_from_prompt(prompt_text, "file2.txt", "minimax m3 beta"),
        )
    };
    Some(ToolchainRecoveryTool::ApplyPatch {
        patch: format!(
            "*** Begin Patch\n*** Update File: {path}\n@@\n {content}\n+PATCH_OK\n*** End Patch\n"
        ),
    })
}

fn exec_recovery_tool(cmd: String, workdir: Option<String>) -> ToolchainRecoveryTool {
    ToolchainRecoveryTool::ExecCommand { cmd, workdir }
}

fn first_tmp_dir(text: &str) -> Option<String> {
    let start = text.find("/tmp/")?;
    let path: String = text[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-'))
        .collect();
    (!path.is_empty()).then_some(path)
}

fn file_content_from_prompt(text: &str, file_name: &str, fallback: &str) -> String {
    let Some(file_pos) = text.find(file_name) else {
        return fallback.to_string();
    };
    let after_file = &text[file_pos + file_name.len()..];
    let Some(marker_pos) = after_file.find("内容包含") else {
        return fallback.to_string();
    };
    let after_marker = after_file[marker_pos + "内容包含".len()..]
        .trim_start_matches(|ch: char| ch.is_whitespace() || ch == ':' || ch == '：');
    let content: String = after_marker
        .chars()
        .take_while(|ch| !matches!(ch, '。' | '\n' | '\r'))
        .collect::<String>()
        .trim()
        .to_string();
    if content.is_empty() {
        fallback.to_string()
    } else {
        content
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn observed_toolchain_coverage(
    messages: &[ChatMessage],
    user_index: usize,
) -> ObservedToolchainCoverage {
    let mut observed = ObservedToolchainCoverage::default();
    let mut tool_names_by_call_id = HashMap::new();

    for msg in messages.iter().skip(user_index + 1) {
        if msg.role == "assistant" {
            if let Some(calls) = &msg.tool_calls {
                for call in calls {
                    let Some(call_id) = call.get("id").and_then(serde_json::Value::as_str) else {
                        continue;
                    };
                    let name = call
                        .get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    tool_names_by_call_id.insert(call_id.to_string(), normalize_tool_name(name));
                }
            }
        } else if msg.role == "tool" {
            let Some(call_id) = msg.tool_call_id.as_deref() else {
                continue;
            };
            let Some(name) = tool_names_by_call_id.get(call_id) else {
                continue;
            };
            observed.total_tools += 1;
            match name.as_str() {
                "read_thread_terminal" => observed.read_thread_terminal += 1,
                "tool_search" => observed.tool_search += 1,
                "apply_patch" => {
                    if tool_output_success(msg.content.as_ref()) {
                        observed.successful_apply_patch += 1;
                    }
                }
                _ => {}
            }
        }
    }

    observed
}

fn normalize_tool_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if lower == "tool_search"
        || lower == "tool_search_tool"
        || lower == "tool_search.tool_search_tool"
        || lower == "tool_search__tool_search_tool"
        || lower == "functions.tool_search"
        || lower == "functions__tool_search"
    {
        "tool_search".into()
    } else if lower == "codex_app__read_thread_terminal"
        || lower == "codex_app.read_thread_terminal"
        || lower == "codex_app__codex_app__read_thread_terminal"
    {
        "read_thread_terminal".into()
    } else {
        name.to_string()
    }
}

fn tool_output_success(content: Option<&serde_json::Value>) -> bool {
    let text = chat_message_text(content);
    let lower = text.to_ascii_lowercase();
    !lower.contains("[failed]")
        && !lower.contains("failed")
        && !text.contains("verification failed")
        && !text.contains("patch rejected")
        && !text.contains("No such file")
        && (lower.contains("success") || lower.contains("exit code: 0"))
}

fn can_override_tool_choice(choice: &Option<serde_json::Value>) -> bool {
    match choice {
        None => true,
        Some(serde_json::Value::String(value)) => matches!(value.as_str(), "auto" | "none"),
        _ => false,
    }
}

fn min_tool_call_requirement(messages: &[ChatMessage]) -> Option<(usize, usize)> {
    messages.iter().enumerate().rev().find_map(|(index, msg)| {
        if msg.role != "user" {
            return None;
        }
        let text = chat_message_text(msg.content.as_ref());
        if text.contains(MIN_TOOL_CALL_GUARD_MARKER) {
            return None;
        }
        min_tool_calls_from_text(&text).map(|required| (index, required))
    })
}

fn chat_message_text(content: Option<&serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn min_tool_calls_from_text(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let has_tool_word = text.contains("工具调用")
        || lower.contains("tool call")
        || lower.contains("tool invocation");
    if !has_tool_word || !(text.contains("至少") || lower.contains("at least")) {
        return None;
    }
    ascii_numbers(text)
        .into_iter()
        .filter(|number| (1..=100).contains(number))
        .max()
}

fn ascii_numbers(text: &str) -> Vec<usize> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            if let Ok(number) = current.parse::<usize>() {
                numbers.push(number);
            }
            current.clear();
        }
    }
    if !current.is_empty() {
        if let Ok(number) = current.parse::<usize>() {
            numbers.push(number);
        }
    }
    numbers
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
    fn minimax_requires_more_tool_calls_when_minimum_not_reached() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!(
                "必须至少完成 24 次工具调用，没达到 24 次前不能总结、不能结束。"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for idx in 0..5 {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": "exec_command", "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        let mut req = ChatRequest {
            model: "MiniMax-M3".into(),
            messages,
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("minimax"), &mut req);

        assert_eq!(req.tool_choice, Some(json!("required")));
        assert!(req.messages.iter().any(|msg| {
            msg.role == "user"
                && msg
                    .content
                    .as_ref()
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|text| {
                        text.contains("MiniMax 最小工具调用计数约束")
                            && text.contains("5/24")
                            && text.contains("必须包含实际 tool_calls")
                    })
        }));
    }

    #[test]
    fn minimax_requires_missing_toolchain_coverage_after_minimum_reached() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!(
                "MiniMax 工具链兼容性压力测试。必须完成至少 24 次独立工具调用。read_thread_terminal 调用至少 1 次。tool_search 分别搜索 thread 和 browser，各 1 次。apply_patch 修改至少 2 个 txt 文件，追加 PATCH_OK。"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for idx in 0..24 {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": "exec_command", "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        let mut req = ChatRequest {
            model: "MiniMax-M2.7-highspeed".into(),
            messages,
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("minimax"), &mut req);

        assert_eq!(req.tool_choice, Some(json!("required")));
        let guard = req
            .messages
            .iter()
            .rev()
            .find_map(|msg| msg.content.as_ref().and_then(serde_json::Value::as_str))
            .expect("expected coverage guard");
        assert!(guard.contains("MiniMax 工具链覆盖完整性约束"));
        assert!(guard.contains("read_thread_terminal 0/1"));
        assert!(guard.contains("tool_search 0/2"));
        assert!(guard.contains("成功 apply_patch 0/2"));
        let coverage = pending_toolchain_coverage(&req.messages).expect("expected coverage");
        assert_eq!(
            coverage.next_recovery_tool,
            Some(ToolchainRecoveryTool::ReadThreadTerminal)
        );
    }

    #[test]
    fn minimax_toolchain_coverage_counts_successful_apply_patch_aliases() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!(
                "MiniMax 工具链兼容性压力测试。必须完成至少 4 次独立工具调用。read_thread_terminal 调用至少 1 次。tool_search 分别搜索 thread 和 browser，各 1 次。apply_patch 修改至少 2 个 txt 文件，追加 PATCH_OK。"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for (idx, name, output) in [
            (
                "0",
                "apply_patch",
                "Exit code: 0\nSuccess. Updated the following files:",
            ),
            (
                "1",
                "apply_patch",
                "Exit code: 0\nSuccess. Updated the following files:",
            ),
            ("2", "codex_app__read_thread_terminal", "ok"),
            ("3", "tool_search__tool_search_tool", "ok"),
            ("4", "tool_search", "ok"),
        ] {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": name, "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!(output)),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        assert_eq!(pending_toolchain_coverage(&messages), None);
    }

    #[test]
    fn minimax_toolchain_coverage_recovers_tool_search_in_order() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!(
                "MiniMax 工具链兼容性压力测试。必须完成至少 24 次独立工具调用。read_thread_terminal 调用至少 1 次。tool_search 分别搜索 thread 和 browser，各 1 次。"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for (idx, name) in [
            ("0", "codex_app__read_thread_terminal"),
            ("1", "tool_search"),
        ] {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": name, "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        let coverage = pending_toolchain_coverage(&messages).expect("expected coverage");
        assert_eq!(
            coverage.next_recovery_tool,
            Some(ToolchainRecoveryTool::ToolSearchBrowser)
        );
    }

    #[test]
    fn minimax_stability_test_minimum_uses_sequential_recovery() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!(
                "MiniMax-M3 工具链稳定性测试，低并发顺序执行。必须完成至少 18 次真实工具调用。必须按顺序执行：1. exec_command 创建 /tmp/codex-minimax-m3-seq-test。2. exec_command 创建 file1.txt，内容包含 minimax m3 alpha。3. exec_command 创建 file2.txt，内容包含 minimax m3 beta。"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for idx in 0..2 {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": "exec_command", "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        let coverage = pending_toolchain_coverage(&messages).expect("expected coverage");
        assert!(coverage
            .missing
            .iter()
            .any(|item| item == "总工具调用 2/18"));
        assert!(matches!(
            coverage.next_recovery_tool,
            Some(ToolchainRecoveryTool::ExecCommand { ref cmd, ref workdir })
                if cmd.contains("file2.txt")
                    && cmd.contains("minimax m3 beta")
                    && workdir.as_deref() == Some("/tmp/codex-minimax-m3-seq-test")
        ));
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
    fn mimo_tool_requests_get_execution_guard() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![ChatMessage {
                role: "system".into(),
                content: Some(json!("base system")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        let system = req.messages[0].content.as_ref().unwrap().as_str().unwrap();
        assert!(system.contains("base system"));
        assert!(system.contains("MiMo 工具调用稳定性约束"));
        assert!(system.contains("必须直接继续发起下一次工具调用"));
    }

    #[test]
    fn mimo_without_tools_does_not_get_execution_guard() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![ChatMessage {
                role: "system".into(),
                content: Some(json!("base system")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
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
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        let system = req.messages[0].content.as_ref().unwrap().as_str().unwrap();
        assert!(!system.contains("MiMo 工具调用稳定性约束"));
    }

    #[test]
    fn mimo_execution_guard_is_not_duplicated() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![ChatMessage {
                role: "system".into(),
                content: Some(json!(tool_execution_guard("MiMo"))),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        let system = req.messages[0].content.as_ref().unwrap().as_str().unwrap();
        assert_eq!(system.matches("MiMo 工具调用稳定性约束").count(), 1);
    }

    #[test]
    fn mimo_requires_more_tool_calls_when_minimum_not_reached() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!("至少执行 15 次独立工具调用，不要合并成一个大脚本。")),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for idx in 0..11 {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": "exec_command", "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages,
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        assert_eq!(req.tool_choice, Some(json!("required")));
        assert!(req.messages.iter().any(|msg| {
            msg.role == "user"
                && msg
                    .content
                    .as_ref()
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|text| {
                        text.contains("MiMo 最小工具调用计数约束")
                            && text.contains("11/15")
                            && text.contains("必须包含实际 tool_calls")
                    })
        }));
    }

    #[test]
    fn mimo_allows_summary_when_minimum_tool_calls_reached() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!("至少执行 15 次独立工具调用，不要合并成一个大脚本。")),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for idx in 0..15 {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": "exec_command", "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }

        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages,
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        assert_eq!(req.tool_choice, None);
    }

    #[test]
    fn mimo_reinforces_minimum_after_progress_title_without_tool_call() {
        let mut messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(json!(
                "必须至少完成 24 次工具调用，没达到 24 次前不能总结、不能结束。"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        for idx in 0..7 {
            messages.push(ChatMessage {
                role: "assistant".into(),
                content: None,
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: Some(vec![json!({
                    "id": format!("call_{idx}"),
                    "type": "function",
                    "function": {"name": "exec_command", "arguments": "{}"}
                })]),
                tool_call_id: None,
                name: None,
            });
            messages.push(ChatMessage {
                role: "tool".into(),
                content: Some(json!("ok")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: Some(format!("call_{idx}")),
                name: None,
            });
        }
        messages.push(ChatMessage {
            role: "assistant".into(),
            content: Some(json!(
                "**工具调用 #8: exec_command - 使用 rg 搜索关键词 mimo**"
            )),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages,
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        assert_eq!(req.tool_choice, Some(json!("required")));
        let guard = req
            .messages
            .iter()
            .rev()
            .find(|msg| {
                msg.role == "user"
                    && chat_message_text(msg.content.as_ref()).contains("MiMo 最小工具调用计数约束")
            })
            .and_then(|msg| msg.content.as_ref())
            .and_then(serde_json::Value::as_str)
            .unwrap();
        assert!(guard.contains("7/24"));
        assert!(guard.contains("不能只输出阶段标题"));
    }

    #[test]
    fn mimo_dynamic_minimum_guard_stays_near_latest_turn() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: Some(json!("base system")),
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                ChatMessage {
                    role: "user".into(),
                    content: Some(json!("至少执行 24 次工具调用。")),
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            ],
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
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

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        assert_eq!(
            req.messages
                .last()
                .and_then(|msg| msg.content.as_ref())
                .and_then(serde_json::Value::as_str)
                .map(|text| text.contains("MiMo 最小工具调用计数约束")),
            Some(true)
        );
        assert_eq!(
            req.messages.iter().filter(|msg| msg.role == "user").count(),
            2
        );
    }

    #[test]
    fn mimo_keeps_explicit_tool_choice_when_minimum_not_reached() {
        let mut req = ChatRequest {
            model: "mimo-v2.5-pro".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: Some(json!("至少执行 15 次独立工具调用。")),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![
                json!({"type":"function","function":{"name":"exec_command","parameters":{"type":"object"}}}),
            ],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: true,
            reasoning_effort: None,
            thinking: None,
            reasoning_split: None,
            tool_choice: Some(json!({
                "type": "function",
                "function": {"name": "exec_command"}
            })),
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        adapt_chat_request(&profile_by_slug("mimo"), &mut req);

        assert_eq!(
            req.tool_choice
                .as_ref()
                .and_then(|choice| choice.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("exec_command")
        );
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
