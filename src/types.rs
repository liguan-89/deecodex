use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ── Responses API (inbound from Codex CLI) ──────────────────────────────────

#[allow(dead_code)] // reserved fields for API compat
#[derive(Debug, Deserialize, Clone)]
pub struct ResponsesRequest {
    pub model: String,
    #[serde(default)]
    pub input: ResponsesInput,
    #[serde(default)]
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(default)]
    pub tool_choice: Option<Value>,
    #[serde(default)]
    pub store: Option<bool>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, String>>,
    #[serde(default)]
    pub truncation: Option<String>,
    #[serde(default)]
    pub background: Option<bool>,
    #[serde(default)]
    pub conversation: Option<Value>,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub include_obfuscation: Option<bool>,
    #[serde(default)]
    pub max_tool_calls: Option<u32>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub prompt: Option<Value>,
    #[serde(default)]
    pub prompt_cache_key: Option<String>,
    #[serde(default)]
    pub prompt_cache_retention: Option<String>,
    #[serde(default)]
    pub safety_identifier: Option<String>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub stream_options: Option<Value>,
    #[serde(default)]
    pub text: Option<Value>,
    #[serde(default)]
    pub top_logprobs: Option<u32>,
    #[serde(default)]
    pub user: Option<String>,
}

#[allow(dead_code)] // reserved fields for API compat
#[derive(Debug, Deserialize, Clone)]
pub struct ReasoningConfig {
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum ResponsesInput {
    Text(String),
    Messages(Vec<Value>),
}

impl Default for ResponsesInput {
    fn default() -> Self {
        Self::Messages(Vec::new())
    }
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResponsesResponse {
    pub id: String,
    pub object: &'static str,
    pub model: String,
    pub output: Vec<ResponsesOutputItem>,
    pub usage: ResponsesUsage,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResponsesOutputItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ContentPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Debug, Serialize, Default, Clone)]
pub struct ResponsesUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

// ── Chat Completions (outbound to provider) ──────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Request token usage stats in the final streaming chunk (DeepSeek)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,
    /// DeepSeek web_search activation via web_search_options (non-standard extension)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_options: Option<Value>,
}

#[derive(Debug, Serialize, Clone)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    /// Can be a plain string or a multimodal array
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ChatMessage,
}

#[allow(dead_code)] // reserved fields for upstream API compat
#[derive(Debug, Deserialize, Clone)]
pub struct ChatUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    /// DeepSeek returns nested token breakdown inside completion_tokens_details
    #[serde(default)]
    pub completion_tokens_details: Option<TokenDetails>,
    /// DeepSeek context caching stats
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_cache_miss_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_tokens_details: Option<CachedTokenDetails>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TokenDetails {
    #[serde(default)]
    pub reasoning_tokens: Option<u32>,
}

#[allow(dead_code)] // reserved for upstream API compat
#[derive(Debug, Deserialize, Clone)]
pub struct CachedTokenDetails {
    #[serde(default)]
    pub cached_tokens: Option<u32>,
}

// ── SSE streaming types ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChatStreamChunk {
    pub choices: Vec<ChatStreamChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ChatStreamChoice {
    pub delta: ChatDelta,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ChatDelta {
    #[allow(dead_code)]
    pub role: Option<String>,
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DeltaToolCall {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DeltaFunction {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

// ── Model mapping ────────────────────────────────────────────────────────────

pub type ModelMap = HashMap<String, String>;

pub fn resolve_model(model: &str, map: &ModelMap) -> String {
    map.get(model).cloned().unwrap_or_else(|| model.to_string())
}

/// Map Codex reasoning.effort to DeepSeek reasoning_effort + thinking params.
///
/// | Codex effort | reasoning_effort | thinking            |
/// |-------------|-----------------|---------------------|
/// | none        | (none)          | {"type":"disabled"} |
/// | minimal     | (none)          | {"type":"disabled"} |
/// | low         | (none)          | {"type":"disabled"} |
/// | medium      | "high"          | {"type":"disabled"} |
/// | high        | "high"          | {"type":"enabled"}  |
/// | xhigh       | "max"           | {"type":"enabled"}  |
/// | default     | "high"          | {"type":"disabled"} |
pub fn map_effort(effort: Option<&str>) -> (Option<String>, Option<Value>) {
    match effort.unwrap_or("medium") {
        "none" => (None, Some(serde_json::json!({"type": "disabled"}))),
        "minimal" => (None, Some(serde_json::json!({"type": "disabled"}))),
        "low" => (None, Some(serde_json::json!({"type": "disabled"}))),
        "medium" => (
            None,
            Some(serde_json::json!({"type": "disabled"})),
        ),
        "high" => (
            Some("high".into()),
            Some(serde_json::json!({"type": "enabled"})),
        ),
        "xhigh" => (
            Some("max".into()),
            Some(serde_json::json!({"type": "enabled"})),
        ),
        _ => (None, Some(serde_json::json!({"type": "disabled"}))),
    }
}

/// Format token usage for logging: "in→out [reason=N] [hit=N] [miss=N]"
pub fn format_usage(usage: Option<&ChatUsage>) -> String {
    match usage {
        None => "?".into(),
        Some(u) => {
            let mut s = format!("in={} out={}", u.prompt_tokens, u.completion_tokens);
            if let Some(ref det) = u.completion_tokens_details {
                if let Some(rt) = det.reasoning_tokens {
                    if rt > 0 {
                        s.push_str(&format!(" reason={rt}"));
                    }
                }
            }
            if let Some(hit) = u.prompt_cache_hit_tokens {
                if hit > 0 {
                    s.push_str(&format!(" hit={hit}"));
                }
            }
            if let Some(miss) = u.prompt_cache_miss_tokens {
                if miss > 0 {
                    s.push_str(&format!(" miss={miss}"));
                }
            }
            s
        }
    }
}

/// Format optional thinking state: "on" | "off" | "-"
pub fn fmt_thinking(t: &Option<serde_json::Value>) -> &str {
    match t {
        Some(v) if v.get("type").and_then(|t| t.as_str()) == Some("disabled") => "off",
        Some(_) => "on",
        None => "-",
    }
}

/// Format optional effort: "xhigh" | "high" | "-" etc
pub fn fmt_effort(e: &Option<String>) -> &str {
    match e {
        Some(s) => s.as_str(),
        None => "-",
    }
}

/// Format optional reasoning effort from Codex: "xhigh" | "medium" | "low" | "-"
pub fn fmt_codex_effort(e: Option<&str>) -> &str {
    match e {
        Some(s) => s,
        None => "-",
    }
}
