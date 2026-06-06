use async_stream::stream;
use axum::response::{
    sse::{Event, KeepAlive},
    Sse,
};
use eventsource_stream::Eventsource as EventsourceExt;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::{
    cache::{usage_to_cached, CachedResponse, CachedToolCall, RequestCache},
    executor::{
        ComputerActionInvocation, ComputerActionOutput, LocalExecutorConfig, McpToolInvocation,
        McpToolOutput,
    },
    metrics::Metrics,
    providers,
    request_history::{HistoryContext, RequestHistoryStore},
    runtime_feedback::RuntimeFeedbackSink,
    session::SessionStore,
    token_anomaly::TokenTracker,
    types::{format_usage, ChatMessage, ChatRequest, ChatStreamChunk, ChatUsage, ModelMap},
    utils::merge_response_extra,
};

fn chat_usage_cache_hit(usage: &ChatUsage) -> bool {
    let prompt_cache_hit = usage.prompt_cache_hit_tokens.unwrap_or(0);
    let prompt_cached = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens)
        .unwrap_or(0);
    prompt_cache_hit > 0 || prompt_cached > 0
}

fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

pub struct StreamArgs {
    pub client: reqwest::Client,
    pub url: String,
    pub api_key: String,
    pub chat_req: ChatRequest,
    pub response_id: String,
    pub sessions: SessionStore,
    pub prior_messages: Vec<ChatMessage>,
    pub request_messages: Vec<ChatMessage>,
    pub request_input_items: Vec<Value>,
    pub store_response: bool,
    pub conversation_id: Option<String>,
    pub response_extra: Value,
    pub model: String,
    #[allow(dead_code)]
    pub model_map: ModelMap,
    /// Optional request cache for storing completed responses
    pub cache: Option<RequestCache>,
    /// Precomputed cache key for this request
    pub cache_key: Option<u64>,
    pub token_tracker: Arc<TokenTracker>,
    pub metrics: Arc<Metrics>,
    pub executors: Arc<tokio::sync::RwLock<LocalExecutorConfig>>,
    pub allowed_mcp_servers: Vec<String>,
    pub allowed_computer_displays: Vec<String>,
    pub custom_headers: std::collections::HashMap<String, String>,
    pub request_timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    pub request_history: Arc<RequestHistoryStore>,
    pub history_context: HistoryContext,
    pub codex_router_sessions: Option<crate::codex_router_session::RouteStateMap>,
    pub upstream_url: String,
    pub allow_missing_done: bool,
    pub runtime_feedback: RuntimeFeedbackSink,
    pub start: std::time::Instant,
}

/// Arguments for replaying a cached response as SSE.
pub struct CachedArgs {
    pub response_id: String,
    pub model: String,
    pub cached: CachedResponse,
    pub sessions: SessionStore,
    pub request_input_items: Vec<Value>,
    pub store_response: bool,
    pub conversation_id: Option<String>,
    pub response_extra: Value,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

struct ResponseToolCallItem {
    item_type: &'static str,
    item_id: String,
    name: Option<String>,
    namespace: Option<String>,
    server_label: Option<String>,
    arguments: Option<String>,
    input: Option<String>,
    action: Option<Value>,
}

fn event_with_sequence(
    seq: &mut u32,
    name: &'static str,
    mut payload: Value,
) -> Result<Event, std::convert::Infallible> {
    *seq += 1;
    if let Value::Object(ref mut obj) = payload {
        obj.insert("sequence_number".to_string(), json!(*seq));
    }
    Ok(Event::default().event(name).data(payload.to_string()))
}

fn reasoning_segment_events(
    seq: &mut u32,
    emitted_reasoning_item: &mut bool,
    accumulated_reasoning: &mut String,
    reasoning_item_id: &str,
    text: &str,
) -> Vec<Result<Event, std::convert::Infallible>> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut events = Vec::new();
    if !*emitted_reasoning_item {
        events.push(event_with_sequence(
            seq,
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": { "type": "reasoning_summary", "id": reasoning_item_id, "status": "in_progress", "summary_index": 0 }
            }),
        ));
        *emitted_reasoning_item = true;
    }
    accumulated_reasoning.push_str(text);
    events.push(event_with_sequence(
        seq,
        "response.reasoning_summary_text.delta",
        json!({
            "type": "response.reasoning_summary_text.delta",
            "item_id": reasoning_item_id,
            "output_index": 0,
            "content_index": 0,
            "delta": text
        }),
    ));
    events
}

fn text_segment_events(
    seq: &mut u32,
    emitted_message_item: &mut bool,
    emitted_reasoning_item: bool,
    accumulated_text: &mut String,
    msg_item_id: &str,
    text: &str,
) -> Vec<Result<Event, std::convert::Infallible>> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut events = Vec::new();
    let output_index: usize = if emitted_reasoning_item { 1 } else { 0 };
    if !*emitted_message_item {
        events.push(event_with_sequence(
            seq,
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": { "type": "message", "id": msg_item_id, "role": "assistant", "content": [], "status": "in_progress" }
            }),
        ));
        *emitted_message_item = true;
    }
    accumulated_text.push_str(text);
    events.push(event_with_sequence(
        seq,
        "response.output_text.delta",
        json!({
            "type": "response.output_text.delta",
            "item_id": msg_item_id,
            "output_index": output_index,
            "content_index": 0,
            "delta": text
        }),
    ));
    events
}

enum ContentSegment {
    Text(String),
    Reasoning(String),
}

struct ThinkTagParser {
    in_think_tag: bool,
    pending: String,
}

impl ThinkTagParser {
    fn new() -> Self {
        Self {
            in_think_tag: false,
            pending: String::new(),
        }
    }

    fn push(&mut self, chunk: &str) -> Vec<ContentSegment> {
        if chunk.is_empty() {
            return Vec::new();
        }
        let mut input = String::new();
        input.push_str(&self.pending);
        input.push_str(chunk);
        self.pending.clear();
        self.consume(input)
    }

    fn finish(&mut self) -> Vec<ContentSegment> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let pending = std::mem::take(&mut self.pending);
        vec![self.segment(pending)]
    }

    fn consume(&mut self, mut input: String) -> Vec<ContentSegment> {
        let mut segments = Vec::new();
        while !input.is_empty() {
            let marker = if self.in_think_tag {
                "</think>"
            } else {
                "<think>"
            };
            if let Some(pos) = input.find(marker) {
                let before = input[..pos].to_string();
                if !before.is_empty() {
                    segments.push(self.segment(before));
                }
                self.in_think_tag = !self.in_think_tag;
                input = input[pos + marker.len()..].to_string();
                continue;
            }

            let keep = partial_marker_suffix_len(&input, marker);
            let emit_len = input.len().saturating_sub(keep);
            if emit_len > 0 {
                segments.push(self.segment(input[..emit_len].to_string()));
            }
            self.pending = input[emit_len..].to_string();
            break;
        }
        segments
    }

    fn segment(&self, text: String) -> ContentSegment {
        if self.in_think_tag {
            ContentSegment::Reasoning(text)
        } else {
            ContentSegment::Text(text)
        }
    }
}

fn partial_marker_suffix_len(input: &str, marker: &str) -> usize {
    let max = input.len().min(marker.len().saturating_sub(1));
    (1..=max)
        .rev()
        .find(|len| input.ends_with(&marker[..*len]))
        .unwrap_or(0)
}

fn is_tool_search_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "tool_search"
        || lower == "tool_search_tool"
        || lower == "tool_search.tool_search_tool"
        || lower == "tool_search__tool_search_tool"
        || lower == "functions.tool_search"
        || lower == "functions__tool_search"
}

fn normalize_tool_search_name(name: &str) -> &str {
    if is_tool_search_name(name) {
        "tool_search"
    } else {
        name
    }
}

fn split_namespace_tool_name(name: &str) -> (Option<String>, String) {
    if let Some((namespace, tool_name)) = name.rsplit_once("__") {
        if !namespace.is_empty() && !tool_name.is_empty() {
            return (
                Some(
                    namespace
                        .strip_suffix("__")
                        .unwrap_or(namespace)
                        .to_string(),
                ),
                tool_name.to_string(),
            );
        }
    }
    (None, name.to_string())
}

fn response_tool_call_item(call_id: &str, name: &str, arguments: &str) -> ResponseToolCallItem {
    if name == "local_computer" {
        ResponseToolCallItem {
            item_type: "computer_call",
            item_id: format!("cc_{call_id}"),
            name: None,
            namespace: None,
            server_label: None,
            arguments: None,
            input: None,
            action: serde_json::from_str::<Value>(arguments)
                .ok()
                .or_else(|| Some(json!({"type": "unknown"}))),
        }
    } else if name == "local_mcp_call" {
        let mcp = parse_local_mcp_arguments(arguments);
        ResponseToolCallItem {
            item_type: "mcp_tool_call",
            item_id: format!("mcp_{call_id}"),
            name: Some(mcp.tool),
            namespace: None,
            server_label: Some(mcp.server_label),
            arguments: Some(mcp.arguments),
            input: None,
            action: None,
        }
    } else if name == "apply_patch" {
        ResponseToolCallItem {
            item_type: "custom_tool_call",
            item_id: format!("ctc_{call_id}"),
            name: Some("apply_patch".into()),
            namespace: None,
            server_label: None,
            arguments: None,
            input: Some(apply_patch_input_from_arguments(arguments)),
            action: None,
        }
    } else {
        let tool_name = normalize_tool_search_name(name).to_string();
        let (namespace, tool_name) = split_namespace_tool_name(&tool_name);
        ResponseToolCallItem {
            item_type: "function_call",
            item_id: format!("fc_{call_id}"),
            name: Some(tool_name),
            namespace,
            server_label: None,
            arguments: Some(arguments.to_string()),
            input: None,
            action: None,
        }
    }
}

fn apply_patch_input_from_arguments(arguments: &str) -> String {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("patch")
                .or_else(|| value.get("input"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| arguments.to_string())
}

fn response_tool_call_json(call_id: &str, spec: &ResponseToolCallItem, in_progress: bool) -> Value {
    let mut item = json!({
        "type": spec.item_type,
        "id": spec.item_id,
        "call_id": call_id,
        "status": if in_progress { "in_progress" } else { "completed" }
    });
    if let Some(name) = &spec.name {
        item["name"] = json!(name);
    }
    if let Some(namespace) = &spec.namespace {
        item["namespace"] = json!(namespace);
    }
    if let Some(server_label) = &spec.server_label {
        item["server_label"] = json!(server_label);
    }
    if let Some(arguments) = &spec.arguments {
        item["arguments"] = json!(if in_progress { "" } else { arguments.as_str() });
    }
    if let Some(input) = &spec.input {
        item["input"] = json!(if in_progress { "" } else { input.as_str() });
    }
    if let Some(action) = &spec.action {
        item["action"] = action.clone();
    }
    item
}

fn mcp_tool_output_json(call_id: &str, result: &McpToolOutput, in_progress: bool) -> Value {
    json!({
        "type": "mcp_tool_call_output",
        "id": format!("mcpout_{call_id}"),
        "call_id": call_id,
        "status": if in_progress { "in_progress" } else { result.status.as_str() },
        "output": if in_progress { Value::Null } else { result.output.clone() }
    })
}

fn computer_call_output_json(
    call_id: &str,
    result: &ComputerActionOutput,
    in_progress: bool,
) -> Value {
    json!({
        "type": "computer_call_output",
        "id": format!("ccout_{call_id}"),
        "call_id": call_id,
        "status": if in_progress { "in_progress" } else { result.status.as_str() },
        "output": if in_progress { Value::Null } else { result.output.clone() }
    })
}

struct LocalMcpCall {
    server_label: String,
    tool: String,
    arguments: String,
}

fn parse_local_mcp_arguments(raw: &str) -> LocalMcpCall {
    let value = serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({}));
    let server_label = value
        .get("server_label")
        .or_else(|| value.get("server_url"))
        .or_else(|| value.get("server"))
        .and_then(Value::as_str)
        .unwrap_or("remote_mcp")
        .to_string();
    let tool = value
        .get("tool")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let arguments = value
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string();
    LocalMcpCall {
        server_label,
        tool,
        arguments,
    }
}

fn reasoning_delta_text(delta: &crate::types::ChatDelta) -> Option<String> {
    if let Some(text) = delta.reasoning_content.as_deref().filter(|s| !s.is_empty()) {
        return Some(text.to_string());
    }
    let details = delta.reasoning_details.as_ref()?;
    let mut chunks = Vec::new();
    collect_reasoning_detail_text(details, &mut chunks);
    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join(""))
    }
}

fn collect_reasoning_detail_text(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::String(s) => chunks.push(s.clone()),
        Value::Array(items) => {
            for item in items {
                collect_reasoning_detail_text(item, chunks);
            }
        }
        Value::Object(map) => {
            for key in ["text", "content", "summary", "reasoning", "delta"] {
                if let Some(s) = map.get(key).and_then(Value::as_str) {
                    chunks.push(s.to_string());
                    return;
                }
            }
            for value in map.values() {
                collect_reasoning_detail_text(value, chunks);
            }
        }
        _ => {}
    }
}

fn push_reasoning_detail_delta(accumulated: &mut Vec<Value>, value: &Value) {
    if let Some(items) = value.as_array() {
        accumulated.extend(items.iter().cloned());
    } else {
        accumulated.push(value.clone());
    }
}

fn reasoning_details_message_value(accumulated: &[Value]) -> Option<Value> {
    (!accumulated.is_empty()).then(|| Value::Array(accumulated.to_vec()))
}

pub fn translate_stream(
    args: StreamArgs,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let StreamArgs {
        client,
        url,
        api_key,
        chat_req,
        response_id,
        sessions,
        prior_messages: _prior_messages,
        request_messages,
        request_input_items,
        store_response,
        conversation_id,
        response_extra,
        model,
        model_map: _model_map,
        cache,
        cache_key,
        token_tracker,
        metrics,
        executors,
        allowed_mcp_servers,
        allowed_computer_displays,
        custom_headers,
        request_timeout_secs,
        max_retries: account_max_retries,
        request_history,
        history_context,
        codex_router_sessions,
        upstream_url,
        allow_missing_done,
        runtime_feedback,
        start,
    } = args;
    let msg_item_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let reasoning_item_id = format!("rsn_{}", uuid::Uuid::new_v4().simple());

    let event_stream = stream! {
        let executors = executors.read().await.clone();
        let mut seq = 0_u32;
        yield event_with_sequence(
            &mut seq,
            "response.created",
            json!({
                "type": "response.created",
                "response": { "id": &response_id, "status": "in_progress", "model": &model }
            }),
        );
        if store_response {
            let mut created_response = json!({
                "id": &response_id,
                "object": "response",
                "status": "in_progress",
                "model": &model,
                "output": []
            });
            merge_response_extra(&mut created_response, &response_extra);
            sessions.save_response(response_id.clone(), created_response);
            sessions.save_input_items(response_id.clone(), request_input_items.clone());
        }

        // Build and send the upstream request.
        // If DeepSeek rejects with "reasoning_content must be passed back"
        // (e.g. after relay restart lost in-memory reasoning state),
        // retry once with thinking disabled.
        let max_retries = account_max_retries.unwrap_or(3) as usize;
        let mut attempt = 0;
        let mut delay_ms: u64 = 500;
        let mut disable_thinking_retry = false;
        let mut disable_web_search_retry = false;
        let upstream = loop {
            let mut builder = client.post(&url).header("Content-Type", "application/json");
            if !api_key.is_empty() {
                builder = builder.bearer_auth(api_key.as_str());
            }
            // 注入账号级自定义 HTTP 头
            for (k, v) in &custom_headers {
                if let (Ok(name), Ok(value)) = (
                    axum::http::header::HeaderName::from_bytes(k.as_bytes()),
                    axum::http::header::HeaderValue::from_str(v),
                ) {
                    builder = builder.header(name, value);
                }
            }
            // 账号级请求超时
            if let Some(secs) = request_timeout_secs {
                builder = builder.timeout(std::time::Duration::from_secs(secs));
            }

            let req_to_send = if disable_thinking_retry || disable_web_search_retry {
                let mut fallback_req = chat_req.clone();
                if disable_web_search_retry {
                    providers::strip_web_search_tool(&mut fallback_req);
                }
                if disable_thinking_retry {
                    fallback_req.thinking = Some(serde_json::json!({"type": "disabled"}));
                    fallback_req.reasoning_effort = None;
                }
                fallback_req
            } else {
                chat_req.clone()
            };

            match builder.json(&req_to_send).send().await {
                Ok(r) if r.status().is_success() => break r,
                Ok(r) => {
                    let status = r.status();
                    let status_code = status.as_u16();
                    let retry_after = retry_after_secs(r.headers());
                    let body = r.text().await.unwrap_or_default();

                    let reasoning_content_error =
                        status_code == 400 && body.contains("reasoning_content");
                    let web_search_disabled_error =
                        providers::is_mimo_web_search_disabled_error(status_code, &body);
                    let retryable = matches!(status_code, 401 | 429 | 502 | 503)
                        || (reasoning_content_error && !disable_thinking_retry)
                        || (web_search_disabled_error
                            && !disable_web_search_retry
                            && providers::has_web_search_tool(&chat_req));

                    if retryable && attempt < max_retries {
                        attempt += 1;
                        if reasoning_content_error {
                            disable_thinking_retry = true;
                        }
                        if web_search_disabled_error {
                            disable_web_search_retry = true;
                        }
                        warn!("upstream {status_code} (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms");
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        delay_ms *= 2;
                        continue;
                    }

                    let error_msg = if body.trim_start().starts_with('<') {
                        format!("upstream HTTP {}", status_code)
                    } else {
                        body.clone()
                    };
                    runtime_feedback
                        .failure(&model, status_code, error_msg.clone(), retry_after)
                        .await;
                    error!("upstream {}: {}", status_code, body.chars().take(300).collect::<String>());
                    if store_response {
                        let mut failed = json!({
                            "id": &response_id,
                            "object": "response",
                            "status": "failed",
                            "model": &model,
                            "output": [],
                            "error": {"code": status_code.to_string(), "message": error_msg.clone()}
                        });
                        merge_response_extra(&mut failed, &response_extra);
                        sessions.save_response(response_id.clone(), failed);
                    }
                    yield event_with_sequence(
                        &mut seq,
                        "response.failed",
                        json!({"type": "response.failed", "response": {"id": &response_id, "status": "failed", "error": {"code": status_code.to_string(), "message": error_msg}}}),
                    );
                    let _ = request_history.record(history_context.record(
                        response_id,
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                        model,
                        "failed".into(),
                        0, 0,
                        start.elapsed().as_millis() as u64,
                        upstream_url,
                        format!("HTTP {}", status_code),
                        false,
                    )).await;
                    return;
                }
                Err(e) => {
                    if attempt < max_retries {
                        attempt += 1;
                        warn!("upstream connection error (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        delay_ms *= 2;
                        continue;
                    }
                    error!("upstream request failed: {e}");
                    runtime_feedback
                        .failure(
                            &model,
                            reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                            e.to_string(),
                            None,
                        )
                        .await;
                    if store_response {
                        let mut failed = json!({
                            "id": &response_id,
                            "object": "response",
                            "status": "failed",
                            "model": &model,
                            "output": [],
                            "error": {"code": "connection_error", "message": e.to_string()}
                        });
                        merge_response_extra(&mut failed, &response_extra);
                        sessions.save_response(response_id.clone(), failed);
                    }
                    yield event_with_sequence(
                        &mut seq,
                        "response.failed",
                        json!({"type": "response.failed", "response": {"id": &response_id, "status": "failed", "error": {"code": "connection_error", "message": e.to_string()}}}),
                    );
                    let _ = request_history.record(history_context.record(
                        response_id,
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                        model,
                        "failed".into(),
                        0, 0,
                        start.elapsed().as_millis() as u64,
                        upstream_url,
                        e.to_string(),
                        false,
                    )).await;
                    return;
                }
            }
        };

        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut accumulated_reasoning_details: Vec<Value> = Vec::new();
        let mut tool_calls: BTreeMap<usize, ToolCallAccum> = BTreeMap::new();
        let mut emitted_message_item = false;
        let mut emitted_reasoning_item = false;
        let mut final_usage: Option<ChatUsage> = None;
        let mut response_native_feedback_sent = false;
        let mut source = upstream.bytes_stream().eventsource();
        let mut stream_completed = false;
        let mut stream_error: Option<String> = None;
        let mut think_parser = ThinkTagParser::new();

        while let Some(ev) = source.next().await {
            match ev {
                Err(e) => {
                    warn!("SSE parse error: {e}");
                    stream_error = Some(e.to_string());
                    break;
                }
                Ok(ev) if ev.data.trim() == "[DONE]" => { stream_completed = true; break; }
                Ok(ev) if ev.data.is_empty() => continue,
                Ok(ev) => {
                    match serde_json::from_str::<ChatStreamChunk>(&ev.data) {
                        Err(e) => {
                            warn!("chunk parse: {} — data prefix: {}", e, &ev.data[..ev.data.len().min(120)]);
                            stream_error = Some(format!("invalid upstream stream chunk: {e}"));
                            break;
                        }
                        Ok(chunk) => {
                            // Capture usage from final chunk (enabled via stream_options.include_usage)
                            if chunk.usage.is_some() {
                                final_usage = chunk.usage;
                            }
                            for choice in &chunk.choices {
                                if let Some(details) = &choice.delta.reasoning_details {
                                    push_reasoning_detail_delta(
                                        &mut accumulated_reasoning_details,
                                        details,
                                    );
                                }
                                if let Some(rc) = reasoning_delta_text(&choice.delta) {
                                    for event in reasoning_segment_events(
                                        &mut seq,
                                        &mut emitted_reasoning_item,
                                        &mut accumulated_reasoning,
                                        &reasoning_item_id,
                                        &rc,
                                    ) {
                                        yield event;
                                    }
                                }
                                let content = choice.delta.content.as_deref().unwrap_or("");
                                if !content.is_empty() {
                                    for segment in think_parser.push(content) {
                                        match segment {
                                            ContentSegment::Reasoning(text) => {
                                                for event in reasoning_segment_events(
                                                    &mut seq,
                                                    &mut emitted_reasoning_item,
                                                    &mut accumulated_reasoning,
                                                    &reasoning_item_id,
                                                    &text,
                                                ) {
                                                    yield event;
                                                }
                                            }
                                            ContentSegment::Text(text) => {
                                                for event in text_segment_events(
                                                    &mut seq,
                                                    &mut emitted_message_item,
                                                    emitted_reasoning_item,
                                                    &mut accumulated_text,
                                                    &msg_item_id,
                                                    &text,
                                                ) {
                                                    yield event;
                                                }
                                            }
                                        }
                                    }
                                }
                                if let Some(delta_calls) = &choice.delta.tool_calls {
                                    for dc in delta_calls {
                                        let entry = tool_calls.entry(dc.index).or_insert(ToolCallAccum {
                                            id: String::new(),
                                            name: String::new(),
                                            arguments: String::new(),
                                        });
                                        if let Some(id) = &dc.id {
                                            if !id.is_empty() { entry.id.clone_from(id); }
                                        }
                                        if let Some(func) = &dc.function {
                                            if let Some(n) = &func.name {
                                                if !n.is_empty() { entry.name.push_str(n); }
                                            }
                                            if let Some(a) = &func.arguments {
                                                entry.arguments.push_str(a);
                                            }
                                        }
                                        if !response_native_feedback_sent
                                            && entry.name == "local_computer"
                                        {
                                            if let (Some(sessions), Some(route_key)) = (
                                                codex_router_sessions.as_ref(),
                                                history_context.codex_router_session_key.as_deref(),
                                            ) {
                                                crate::codex_router_session::refresh_native_track(
                                                    sessions,
                                                    route_key,
                                                    std::time::SystemTime::now()
                                                        .duration_since(std::time::UNIX_EPOCH)
                                                        .unwrap_or_default()
                                                        .as_secs(),
                                                    "response.local_computer_tool_call",
                                                );
                                                response_native_feedback_sent = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !stream_completed {
            if let Some(message) = stream_error {
                error!("upstream stream incomplete: {message}");
                runtime_feedback
                    .failure(
                        &model,
                        reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                        message.clone(),
                        None,
                    )
                    .await;
                if store_response {
                    let mut failed = json!({
                        "id": &response_id,
                        "object": "response",
                        "status": "failed",
                        "model": &model,
                        "output": [],
                        "error": {"code": "stream_incomplete", "message": message.clone()}
                    });
                    merge_response_extra(&mut failed, &response_extra);
                    sessions.save_response(response_id.clone(), failed);
                }
                yield event_with_sequence(
                    &mut seq,
                    "response.failed",
                    json!({"type": "response.failed", "response": {"id": &response_id, "status": "failed", "error": {"code": "stream_incomplete", "message": message}}}),
                );
                let _ = request_history.record(history_context.record(
                    response_id,
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                    model,
                    "failed".into(),
                    0, 0,
                    start.elapsed().as_millis() as u64,
                    upstream_url,
                    message,
                    false,
                )).await;
                return;
            }
            if allow_missing_done {
                // 部分兼容接口（如 MiniMax/MiMo）会干净结束 SSE 但不发送 [DONE]。
                stream_completed = true;
            } else {
                let message = "upstream stream ended without [DONE]".to_string();
                error!("upstream stream incomplete: {message}");
                runtime_feedback
                    .failure(
                        &model,
                        reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                        message.clone(),
                        None,
                    )
                    .await;
                if store_response {
                    let mut failed = json!({
                        "id": &response_id,
                        "object": "response",
                        "status": "failed",
                        "model": &model,
                        "output": [],
                        "error": {"code": "stream_incomplete", "message": message.clone()}
                    });
                    merge_response_extra(&mut failed, &response_extra);
                    sessions.save_response(response_id.clone(), failed);
                }
                yield event_with_sequence(
                    &mut seq,
                    "response.failed",
                    json!({"type": "response.failed", "response": {"id": &response_id, "status": "failed", "error": {"code": "stream_incomplete", "message": message}}}),
                );
                let _ = request_history.record(history_context.record(
                    response_id,
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
                    model,
                    "failed".into(),
                    0, 0,
                    start.elapsed().as_millis() as u64,
                    upstream_url,
                    message,
                    false,
                )).await;
                return;
            }
        }

        for segment in think_parser.finish() {
            match segment {
                ContentSegment::Reasoning(text) => {
                    for event in reasoning_segment_events(
                        &mut seq,
                        &mut emitted_reasoning_item,
                        &mut accumulated_reasoning,
                        &reasoning_item_id,
                        &text,
                    ) {
                        yield event;
                    }
                }
                ContentSegment::Text(text) => {
                    for event in text_segment_events(
                        &mut seq,
                        &mut emitted_message_item,
                        emitted_reasoning_item,
                        &mut accumulated_text,
                        &msg_item_id,
                        &text,
                    ) {
                        yield event;
                    }
                }
            }
        }

        // Log streaming token usage
        let usage_str = format_usage(final_usage.as_ref());
        info!("↑ done {}", usage_str);

        if let Some(ref usage) = final_usage {
            let anomalies = token_tracker.record(usage, &model, &response_id);
            for atype in &anomalies {
                metrics
                    .token_anomalies_total
                    .with_label_values(&[atype])
                    .inc();
            }
        }

        // Clone for cache before moving into completion_usage
        let cache_usage = final_usage.clone();
        let history_cache_hit = cache_usage.as_ref().is_some_and(chat_usage_cache_hit);

        // Build usage for response.completed
        let completion_usage = final_usage.map(|u| json!({
            "input_tokens": u.prompt_tokens,
            "output_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens
        }));

        // Close reasoning item
        if emitted_reasoning_item {
            yield event_with_sequence(
                &mut seq,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": 0,
                    "item": {
                        "type": "reasoning",
                        "id": &reasoning_item_id,
                        "status": "completed",
                        "content": [{"type": "summary_text", "text": &accumulated_reasoning}]
                    }
                }),
            );
        }

        // Close message item
        if emitted_message_item {
            let msg_output_index: usize = if emitted_reasoning_item { 1 } else { 0 };
            yield event_with_sequence(
                &mut seq,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": msg_output_index,
                    "item": {
                        "type": "message",
                        "id": &msg_item_id,
                        "role": "assistant",
                        "status": "completed",
                        "content": [{"type": "output_text", "text": &accumulated_text}]
                    }
                }),
            );
        }

        // Emit tool call items.
        let base_index: usize = if emitted_reasoning_item { 1 } else { 0 }
            + if emitted_message_item { 1 } else { 0 };
        let mut fc_items: Vec<Value> = Vec::new();
        let mut executable_mcp_calls: Vec<(String, McpToolInvocation)> = Vec::new();
        let mut executable_computer_calls: Vec<(String, ComputerActionInvocation)> = Vec::new();

        for (rel_idx, (_, tc)) in tool_calls.iter().enumerate() {
            let call_id = if tc.id.is_empty() {
                format!("{}_{}", response_id, base_index + rel_idx)
            } else {
                tc.id.clone()
            };
            let output_index = base_index + rel_idx;
            let arguments = tc.arguments.clone();
            let item_spec = response_tool_call_item(&call_id, &tc.name, &arguments);

            yield event_with_sequence(
                &mut seq,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": response_tool_call_json(&call_id, &item_spec, true)
                }),
            );

            if item_spec.item_type == "function_call" && !arguments.is_empty() {
                yield event_with_sequence(
                    &mut seq,
                    "response.function_call_arguments.delta",
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": &item_spec.item_id,
                        "output_index": output_index,
                        "delta": &arguments
                    }),
                );
            }

            yield event_with_sequence(
                &mut seq,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": response_tool_call_json(&call_id, &item_spec, false)
                }),
            );

            let final_item = response_tool_call_json(&call_id, &item_spec, false);
            if item_spec.item_type == "mcp_tool_call" {
                if let Some(invocation) = McpToolInvocation::from_response_item(&final_item) {
                    executable_mcp_calls.push((call_id.clone(), invocation));
                }
            } else if item_spec.item_type == "computer_call" {
                if let Some(invocation) = ComputerActionInvocation::from_response_item(&final_item) {
                    executable_computer_calls.push((call_id.clone(), invocation));
                }
            }
            fc_items.push(final_item);
        }

        let mut local_mcp_output_items: Vec<Value> = Vec::new();
        let mut local_computer_output_items: Vec<Value> = Vec::new();
        if executors.mcp.enabled() {
            for (rel_idx, (call_id, invocation)) in executable_mcp_calls.into_iter().enumerate() {
                let output_index = base_index + tool_calls.len() + rel_idx;
                let result = if !allowed_mcp_servers.is_empty()
                    && !allowed_mcp_servers.iter().any(|server| server == &invocation.server_label)
                {
                    McpToolOutput::failed(format!(
                        "MCP server '{}' is not allowed by local tool policy",
                        invocation.server_label
                    ))
                } else {
                    executors.mcp.execute_tool(invocation).await
                };
                let final_item = mcp_tool_output_json(&call_id, &result, false);

                yield event_with_sequence(
                    &mut seq,
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": output_index,
                        "item": mcp_tool_output_json(&call_id, &result, true)
                    }),
                );
                yield event_with_sequence(
                    &mut seq,
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": final_item
                    }),
                );
                local_mcp_output_items.push(final_item);
            }
        }
        if executors.computer.enabled() {
            let start_index = base_index + tool_calls.len() + local_mcp_output_items.len();
            for (rel_idx, (call_id, invocation)) in executable_computer_calls.into_iter().enumerate() {
                let output_index = start_index + rel_idx;
                let result = if !allowed_computer_displays.is_empty()
                    && !allowed_computer_displays.iter().any(|display| display == &invocation.display)
                {
                    ComputerActionOutput::failed(format!(
                        "computer display '{}' is not allowed by local tool policy",
                        invocation.display
                    ))
                } else {
                    executors.computer.execute_action(invocation).await
                };
                let final_item = computer_call_output_json(&call_id, &result, false);

                yield event_with_sequence(
                    &mut seq,
                    "response.output_item.added",
                    json!({
                        "type": "response.output_item.added",
                        "output_index": output_index,
                        "item": computer_call_output_json(&call_id, &result, true)
                    }),
                );
                yield event_with_sequence(
                    &mut seq,
                    "response.output_item.done",
                    json!({
                        "type": "response.output_item.done",
                        "output_index": output_index,
                        "item": final_item
                    }),
                );
                local_computer_output_items.push(final_item);
            }
        }

        // Persist reasoning_content
        for tc in tool_calls.values() {
            if !tc.id.is_empty() {
                sessions.store_reasoning(tc.id.clone(), accumulated_reasoning.clone());
            }
        }

        let assistant_tool_calls: Option<Vec<Value>> = if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls.values().map(|tc| json!({
                "id": &tc.id,
                "type": "function",
                "function": { "name": &tc.name, "arguments": &tc.arguments }
            })).collect())
        };
        let assistant_msg = ChatMessage {
            role: "assistant".into(),
            content: if accumulated_text.is_empty() { None } else { Some(serde_json::Value::String(accumulated_text.clone())) },
            reasoning_content: if accumulated_reasoning.is_empty() { None } else { Some(accumulated_reasoning.clone()) },
            reasoning_details: reasoning_details_message_value(&accumulated_reasoning_details),
            tool_calls: assistant_tool_calls,
            tool_call_id: None,
            name: None,
        };

        if !accumulated_reasoning.is_empty() {
            sessions.store_turn_reasoning(&request_messages, &assistant_msg, accumulated_reasoning.clone());
        }
        if assistant_msg.reasoning_details.is_some() {
            sessions.store_turn_reasoning_details(&request_messages, &assistant_msg);
        }

        let mut messages = request_messages.clone();
        messages.push(assistant_msg);
        if store_response {
            sessions.save_with_id(response_id.clone(), messages);
        }
        if let Some(id) = conversation_id.clone() {
            let mut conversation_messages = request_messages;
            conversation_messages.push(ChatMessage {
                role: "assistant".into(),
                content: if accumulated_text.is_empty() { None } else { Some(serde_json::Value::String(accumulated_text.clone())) },
                reasoning_content: if accumulated_reasoning.is_empty() { None } else { Some(accumulated_reasoning.clone()) },
                reasoning_details: reasoning_details_message_value(&accumulated_reasoning_details),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls.values().map(|tc| json!({
                        "id": &tc.id,
                        "type": "function",
                        "function": { "name": &tc.name, "arguments": &tc.arguments }
                    })).collect())
                },
                tool_call_id: None,
                name: None,
            });
            sessions.save_conversation(id, conversation_messages);
        }

        // Build output for response.completed
        let mut output_items: Vec<Value> = Vec::new();
        if emitted_reasoning_item {
            output_items.push(json!({
                "type": "reasoning",
                "id": &reasoning_item_id,
                "status": "completed",
                "content": [{"type": "summary_text", "text": &accumulated_reasoning}]
            }));
        }
        if emitted_message_item {
            output_items.push(json!({
                "type": "message",
                "id": &msg_item_id,
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": &accumulated_text}]
            }));
        }
        output_items.extend(fc_items);
        output_items.extend(local_mcp_output_items);
        output_items.extend(local_computer_output_items);
        if let Some(id) = conversation_id {
            let mut conversation_items = sessions.get_conversation_items(&id);
            conversation_items.extend(output_items.iter().cloned());
            sessions.save_conversation_items(id, conversation_items);
        }

        // Include usage in response.completed
        let mut response_obj = json!({
            "id": &response_id,
            "status": "completed",
            "model": &model,
            "output": output_items
        });
        if let Some(ref u) = completion_usage {
            response_obj["usage"] = u.clone();
        }
        response_obj["object"] = json!("response");
        merge_response_extra(&mut response_obj, &response_extra);
        if store_response {
            sessions.save_response(response_id.clone(), response_obj.clone());
        }

        yield event_with_sequence(
            &mut seq,
            "response.completed",
            json!({
                "type": "response.completed",
                "response": response_obj
            }),
        );

        // Store in request cache (only if stream completed normally)
        if stream_completed && store_response {
            if let (Some(c), Some(key)) = (cache, cache_key) {
                let cached = CachedResponse {
                    text: accumulated_text.clone(),
                    reasoning: accumulated_reasoning.clone(),
                    tool_calls: tool_calls
                        .values()
                        .map(|tc| CachedToolCall {
                            id: tc.id.clone(),
                            name: normalize_tool_search_name(&tc.name).to_string(),
                            arguments: tc.arguments.clone(),
                        })
                        .collect(),
                    usage: usage_to_cached(cache_usage.as_ref()),
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                c.insert(key, cached);
            }
        }

        runtime_feedback.success(&model).await;
        let _ = request_history.record(history_context.record(
            response_id,
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
            model,
            "completed".into(),
            completion_usage.as_ref().and_then(|u| u["input_tokens"].as_u64()).unwrap_or(0) as u32,
            completion_usage.as_ref().and_then(|u| u["output_tokens"].as_u64()).unwrap_or(0) as u32,
            start.elapsed().as_millis() as u64,
            upstream_url,
            String::new(),
            history_cache_hit,
        )).await;
    };

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

/// Replay a cached response as a full SSE event stream.
pub fn translate_cached(
    args: CachedArgs,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let CachedArgs {
        response_id,
        model,
        cached,
        sessions,
        request_input_items,
        store_response,
        conversation_id: _conversation_id,
        response_extra,
    } = args;
    let msg_item_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let reasoning_item_id = format!("rsn_{}", uuid::Uuid::new_v4().simple());

    let event_stream = stream! {
        let mut seq = 0_u32;
        yield event_with_sequence(
            &mut seq,
            "response.created",
            json!({
                "type": "response.created",
                "response": { "id": &response_id, "status": "in_progress", "model": &model }
            }),
        );
        if store_response {
            sessions.save_input_items(response_id.clone(), request_input_items);
        }

        let mut output_index: usize = 0;

        // Reasoning item
        if !cached.reasoning.is_empty() {
            yield event_with_sequence(
                &mut seq,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": { "type": "reasoning_summary", "id": &reasoning_item_id, "status": "in_progress", "summary_index": 0 }
                }),
            );

            yield event_with_sequence(
                &mut seq,
                "response.reasoning_summary_text.delta",
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": &reasoning_item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "delta": &cached.reasoning
                }),
            );

            yield event_with_sequence(
                &mut seq,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "reasoning",
                        "id": &reasoning_item_id,
                        "status": "completed",
                        "content": [{"type": "summary_text", "text": &cached.reasoning}]
                    }
                }),
            );

            output_index += 1;
        }

        // Message item
        if !cached.text.is_empty() || cached.tool_calls.is_empty() {
            yield event_with_sequence(
                &mut seq,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": { "type": "message", "id": &msg_item_id, "role": "assistant", "content": [], "status": "in_progress" }
                }),
            );

            if !cached.text.is_empty() {
                yield event_with_sequence(
                    &mut seq,
                    "response.output_text.delta",
                    json!({
                        "type": "response.output_text.delta",
                        "item_id": &msg_item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "delta": &cached.text
                    }),
                );
            }

            yield event_with_sequence(
                &mut seq,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "message",
                        "id": &msg_item_id,
                        "role": "assistant",
                        "status": "completed",
                        "content": [{"type": "output_text", "text": &cached.text}]
                    }
                }),
            );

            output_index += 1;
        }

        // Tool call items
        let mut cached_fc_items: Vec<Value> = Vec::new();
        for tc in &cached.tool_calls {
            let item_spec = response_tool_call_item(&tc.id, &tc.name, &tc.arguments);

            yield event_with_sequence(
                &mut seq,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": response_tool_call_json(&tc.id, &item_spec, true)
                }),
            );

            if item_spec.item_type == "function_call" && !tc.arguments.is_empty() {
                yield event_with_sequence(
                    &mut seq,
                    "response.function_call_arguments.delta",
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": &item_spec.item_id,
                        "output_index": output_index,
                        "delta": &tc.arguments
                    }),
                );
            }

            yield event_with_sequence(
                &mut seq,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": response_tool_call_json(&tc.id, &item_spec, false)
                }),
            );

            cached_fc_items.push(response_tool_call_json(&tc.id, &item_spec, false));
            output_index += 1;
        }

        // Build output and usage
        let mut output_items: Vec<Value> = Vec::new();
        if !cached.reasoning.is_empty() {
            output_items.push(json!({
                "type": "reasoning", "id": &reasoning_item_id, "status": "completed",
                "content": [{"type": "summary_text", "text": &cached.reasoning}]
            }));
        }
        if !cached.text.is_empty() || cached.tool_calls.is_empty() {
            output_items.push(json!({
                "type": "message", "id": &msg_item_id, "role": "assistant", "status": "completed",
                "content": [{"type": "output_text", "text": &cached.text}]
            }));
        }
        output_items.extend(cached_fc_items);

        let mut response_obj = json!({
            "id": &response_id, "status": "completed", "model": &model, "output": output_items
        });
        if let Some(ref u) = cached.usage {
            response_obj["usage"] = json!({
                "input_tokens": u.prompt_tokens,
                "output_tokens": u.completion_tokens,
                "total_tokens": u.total_tokens
            });
        }
        response_obj["object"] = json!("response");
        merge_response_extra(&mut response_obj, &response_extra);
        if store_response {
            sessions.save_response(response_id.clone(), response_obj.clone());
        }

        yield event_with_sequence(
            &mut seq,
            "response.completed",
            json!({
                "type": "response.completed",
                "response": response_obj
            }),
        );

        info!("request cache: replayed (text={}b reasoning={}b tools={})",
            cached.text.len(), cached.reasoning.len(), cached.tool_calls.len());
    };

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachedUsage;
    use crate::types::CachedTokenDetails;
    use axum::response::IntoResponse;

    fn parse_sse_events(body: &[u8]) -> Vec<(String, serde_json::Value)> {
        let text = std::str::from_utf8(body).unwrap();
        text.split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .map(|block| {
                let mut event_type = String::new();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(val) = line.strip_prefix("event: ") {
                        event_type = val.to_string();
                    } else if let Some(val) = line.strip_prefix("data: ") {
                        data = val.to_string();
                    }
                }
                let data_value: serde_json::Value = serde_json::from_str(&data).unwrap_or_default();
                (event_type, data_value)
            })
            .collect()
    }

    fn assert_sequence_numbers(events: &[(String, serde_json::Value)]) {
        let mut last = 0_u64;
        for (event_type, payload) in events {
            let seq = payload["sequence_number"]
                .as_u64()
                .unwrap_or_else(|| panic!("{event_type} missing sequence_number"));
            assert!(
                seq > last,
                "{event_type} sequence_number {seq} did not increase after {last}"
            );
            last = seq;
        }
    }

    #[test]
    fn detects_stream_usage_cache_hit() {
        let mut usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
            completion_tokens_details: None,
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        };
        assert!(!chat_usage_cache_hit(&usage));

        usage.prompt_tokens_details = Some(CachedTokenDetails {
            cached_tokens: Some(12),
        });
        assert!(chat_usage_cache_hit(&usage));

        usage.prompt_tokens_details = None;
        usage.prompt_cache_hit_tokens = Some(1);
        assert!(chat_usage_cache_hit(&usage));
    }

    #[test]
    fn think_tag_parser_handles_split_markers() {
        let mut parser = ThinkTagParser::new();
        let mut segments = Vec::new();
        segments.extend(parser.push("<thi"));
        segments.extend(parser.push("nk>先"));
        segments.extend(parser.push("分析</thi"));
        segments.extend(parser.push("nk>答案"));
        segments.extend(parser.finish());

        let mut reasoning = String::new();
        let mut text = String::new();
        for segment in segments {
            match segment {
                ContentSegment::Reasoning(chunk) => reasoning.push_str(&chunk),
                ContentSegment::Text(chunk) => text.push_str(&chunk),
            }
        }
        assert_eq!(reasoning, "先分析");
        assert_eq!(text, "答案");
    }

    #[test]
    fn think_tag_parser_does_not_panic_on_non_ascii_suffix() {
        let mut parser = ThinkTagParser::new();
        let segments = parser.push("答案");
        assert_eq!(segments.len(), 1);
        match &segments[0] {
            ContentSegment::Text(text) => assert_eq!(text, "答案"),
            ContentSegment::Reasoning(_) => panic!("expected text segment"),
        }
    }

    #[tokio::test]
    async fn test_cached_text_only() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: "Hello, world!".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        };
        let args = CachedArgs {
            response_id: "test_resp_1".into(),
            model: "test-model".into(),
            cached,
            sessions,
            request_input_items: vec![],
            store_response: false,
            conversation_id: None,
            response_extra: json!({}),
        };
        let sse = translate_cached(args);
        let res = sse.into_response();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let events = parse_sse_events(&bytes);

        assert_eq!(events.len(), 5);
        assert_sequence_numbers(&events);
        assert_eq!(events[0].0, "response.created");
        assert_eq!(events[0].1["type"], "response.created");
        assert_eq!(events[0].1["response"]["id"], "test_resp_1");
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "message");
        assert_eq!(events[2].0, "response.output_text.delta");
        assert_eq!(events[2].1["delta"], "Hello, world!");
        assert_eq!(events[3].0, "response.output_item.done");
        assert_eq!(events[3].1["item"]["type"], "message");
        assert_eq!(events[3].1["item"]["content"][0]["text"], "Hello, world!");
        assert_eq!(events[4].0, "response.completed");
        assert_eq!(events[4].1["response"]["status"], "completed");
    }

    #[tokio::test]
    async fn test_cached_text_and_reasoning() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: "Hello".into(),
            reasoning: "Let me think...".into(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        };
        let args = CachedArgs {
            response_id: "test_resp_2".into(),
            model: "test-model".into(),
            cached,
            sessions,
            request_input_items: vec![],
            store_response: false,
            conversation_id: None,
            response_extra: json!({}),
        };
        let sse = translate_cached(args);
        let res = sse.into_response();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let events = parse_sse_events(&bytes);

        assert_eq!(events.len(), 8);
        assert_eq!(events[0].0, "response.created");
        // reasoning comes first
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "reasoning_summary");
        assert_eq!(events[2].0, "response.reasoning_summary_text.delta");
        assert_eq!(events[2].1["delta"], "Let me think...");
        assert_eq!(events[3].0, "response.output_item.done");
        assert_eq!(events[3].1["item"]["type"], "reasoning");
        assert_eq!(events[3].1["item"]["content"][0]["text"], "Let me think...");
        // then message
        assert_eq!(events[4].0, "response.output_item.added");
        assert_eq!(events[4].1["item"]["type"], "message");
        assert_eq!(events[5].0, "response.output_text.delta");
        assert_eq!(events[5].1["delta"], "Hello");
        assert_eq!(events[6].0, "response.output_item.done");
        assert_eq!(events[6].1["item"]["type"], "message");
        assert_eq!(events[6].1["item"]["content"][0]["text"], "Hello");
        assert_eq!(events[7].0, "response.completed");
        assert_eq!(
            events[7].1["response"]["output"].as_array().unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn test_cached_tool_calls() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: vec![CachedToolCall {
                id: "call_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"/tmp/test.txt"}"#.into(),
            }],
            usage: None,
            created_at: 0,
        };
        let args = CachedArgs {
            response_id: "test_resp_3".into(),
            model: "test-model".into(),
            cached,
            sessions,
            request_input_items: vec![],
            store_response: false,
            conversation_id: None,
            response_extra: json!({}),
        };
        let sse = translate_cached(args);
        let res = sse.into_response();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let events = parse_sse_events(&bytes);

        assert_eq!(events.len(), 5);
        assert_eq!(events[0].0, "response.created");
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "function_call");
        assert_eq!(events[1].1["item"]["name"], "read_file");
        assert_eq!(events[1].1["item"]["call_id"], "call_1");
        assert_eq!(events[2].0, "response.function_call_arguments.delta");
        assert_eq!(events[3].0, "response.output_item.done");
        assert_eq!(events[3].1["item"]["type"], "function_call");
        assert_eq!(events[3].1["item"]["name"], "read_file");
        assert_eq!(events[3].1["item"]["call_id"], "call_1");
        assert!(events[3].1["item"]["arguments"]
            .as_str()
            .unwrap()
            .contains("/tmp/test.txt"));
        assert_eq!(events[4].0, "response.completed");
    }

    #[tokio::test]
    async fn test_cached_empty_response() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        };
        let args = CachedArgs {
            response_id: "test_resp_4".into(),
            model: "test-model".into(),
            cached,
            sessions,
            request_input_items: vec![],
            store_response: false,
            conversation_id: None,
            response_extra: json!({}),
        };
        let sse = translate_cached(args);
        let res = sse.into_response();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let events = parse_sse_events(&bytes);

        assert_eq!(events.len(), 4);
        assert_eq!(events[0].0, "response.created");
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "message");
        assert_eq!(events[2].0, "response.output_item.done");
        assert_eq!(events[2].1["item"]["type"], "message");
        assert_eq!(events[2].1["item"]["content"][0]["text"], "");
        assert_eq!(events[3].0, "response.completed");
    }

    #[tokio::test]
    async fn test_cached_apply_patch_translation() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: vec![CachedToolCall {
                id: "patch_1".into(),
                name: "apply_patch".into(),
                arguments: r#"{"patch":"diff..."}"#.into(),
            }],
            usage: None,
            created_at: 0,
        };
        let args = CachedArgs {
            response_id: "test_resp_5".into(),
            model: "test-model".into(),
            cached,
            sessions,
            request_input_items: vec![],
            store_response: false,
            conversation_id: None,
            response_extra: json!({}),
        };
        let sse = translate_cached(args);
        let res = sse.into_response();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let events = parse_sse_events(&bytes);

        assert_eq!(events.len(), 4);
        assert_eq!(events[0].0, "response.created");
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "custom_tool_call");
        assert_eq!(events[1].1["item"]["name"], "apply_patch");
        assert_eq!(events[1].1["item"]["input"], "");
        assert_eq!(events[2].0, "response.output_item.done");
        assert_eq!(events[2].1["item"]["type"], "custom_tool_call");
        assert_eq!(events[2].1["item"]["name"], "apply_patch");
        assert_eq!(events[2].1["item"]["input"], "diff...");
        assert_eq!(events[3].0, "response.completed");
    }

    // --- Pure function tests ---

    #[test]
    fn test_event_with_sequence_increments() {
        let mut seq = 0_u32;
        let _event = event_with_sequence(&mut seq, "test.event", json!({"key": "val"})).unwrap();
        assert_eq!(seq, 1);
    }

    #[test]
    fn test_event_with_sequence_multiple() {
        let mut seq = 5_u32;
        let _ = event_with_sequence(&mut seq, "e1", json!({})).unwrap();
        assert_eq!(seq, 6);
        let _ = event_with_sequence(&mut seq, "e2", json!({})).unwrap();
        assert_eq!(seq, 7);
    }

    #[test]
    fn test_response_tool_call_item_normal() {
        let item = response_tool_call_item("call_1", "exec_command", r#"{"cmd":"ls"}"#);
        assert_eq!(item.item_type, "function_call");
        assert_eq!(item.item_id, "fc_call_1");
        assert_eq!(item.name.as_deref(), Some("exec_command"));
        assert_eq!(item.arguments.as_deref(), Some(r#"{"cmd":"ls"}"#));
        assert!(item.action.is_none());
    }

    #[test]
    fn test_response_tool_call_item_apply_patch() {
        let item = response_tool_call_item("p1", "apply_patch", r#"{"patch":"diff"}"#);
        assert_eq!(item.item_type, "custom_tool_call");
        assert_eq!(item.item_id, "ctc_p1");
        assert_eq!(item.name.as_deref(), Some("apply_patch"));
        assert_eq!(item.arguments, None);
        assert_eq!(item.input.as_deref(), Some("diff"));
    }

    #[test]
    fn test_response_tool_call_item_tool_search_alias() {
        let item = response_tool_call_item(
            "ts1",
            "tool_search__tool_search_tool",
            r#"{"query":"电脑"}"#,
        );
        assert_eq!(item.item_type, "function_call");
        assert_eq!(item.name.as_deref(), Some("tool_search"));
        assert_eq!(item.arguments.as_deref(), Some(r#"{"query":"电脑"}"#));
    }

    #[test]
    fn test_response_tool_call_item_namespace_function() {
        let item = response_tool_call_item(
            "ns1",
            "mcp__computer_use__get_app_state",
            r#"{"app":"抖音"}"#,
        );
        assert_eq!(item.item_type, "function_call");
        assert_eq!(item.name.as_deref(), Some("get_app_state"));
        assert_eq!(item.namespace.as_deref(), Some("mcp__computer_use"));
        let json = response_tool_call_json("ns1", &item, false);
        assert_eq!(json["name"], "get_app_state");
        assert_eq!(json["namespace"], "mcp__computer_use");
    }

    #[test]
    fn test_response_tool_call_item_computer() {
        let item = response_tool_call_item("scr_1", "local_computer", r#"{"type":"screenshot"}"#);
        assert_eq!(item.item_type, "computer_call");
        assert_eq!(item.item_id, "cc_scr_1");
        assert!(item.name.is_none());
        assert!(item.arguments.is_none());
        assert_eq!(
            item.action
                .as_ref()
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str()),
            Some("screenshot")
        );
    }

    #[test]
    fn test_response_tool_call_item_computer_invalid_json() {
        let item = response_tool_call_item("scr_2", "local_computer", "not-json");
        assert_eq!(item.item_type, "computer_call");
        assert_eq!(
            item.action
                .as_ref()
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str()),
            Some("unknown")
        );
    }

    #[test]
    fn test_response_tool_call_item_local_mcp() {
        let item = response_tool_call_item(
            "m1",
            "local_mcp_call",
            r#"{"server_label":"filesystem","tool":"read_file","arguments":{"path":"README.md"}}"#,
        );

        assert_eq!(item.item_type, "mcp_tool_call");
        assert_eq!(item.item_id, "mcp_m1");
        assert_eq!(item.server_label.as_deref(), Some("filesystem"));
        assert_eq!(item.name.as_deref(), Some("read_file"));
        assert_eq!(item.arguments.as_deref(), Some(r#"{"path":"README.md"}"#));
    }

    #[test]
    fn test_response_tool_call_json_function_in_progress() {
        let spec = response_tool_call_item("c1", "exec_command", r#"{"cmd":"ls"}"#);
        let json = response_tool_call_json("c1", &spec, true);
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["id"], "fc_c1");
        assert_eq!(json["call_id"], "c1");
        assert_eq!(json["status"], "in_progress");
        assert_eq!(json["arguments"], ""); // in_progress → empty
        assert_eq!(json["name"], "exec_command");
    }

    #[test]
    fn test_response_tool_call_json_function_completed() {
        let spec = response_tool_call_item("c1", "exec_command", r#"{"cmd":"ls"}"#);
        let json = response_tool_call_json("c1", &spec, false);
        assert_eq!(json["status"], "completed");
        assert_eq!(json["arguments"], r#"{"cmd":"ls"}"#);
    }

    #[test]
    fn test_response_tool_call_json_local_mcp_completed() {
        let spec = response_tool_call_item(
            "m1",
            "local_mcp_call",
            r#"{"server_label":"filesystem","tool":"read_file","arguments":{"path":"README.md"}}"#,
        );
        let json = response_tool_call_json("m1", &spec, false);

        assert_eq!(json["type"], "mcp_tool_call");
        assert_eq!(json["id"], "mcp_m1");
        assert_eq!(json["call_id"], "m1");
        assert_eq!(json["status"], "completed");
        assert_eq!(json["server_label"], "filesystem");
        assert_eq!(json["name"], "read_file");
        assert_eq!(json["arguments"], r#"{"path":"README.md"}"#);
    }

    #[test]
    fn test_mcp_tool_output_json_completed() {
        let output = McpToolOutput::succeeded(json!({
            "content": [{"type": "text", "text": "ok"}]
        }));
        let json = mcp_tool_output_json("m1", &output, false);

        assert_eq!(json["type"], "mcp_tool_call_output");
        assert_eq!(json["id"], "mcpout_m1");
        assert_eq!(json["call_id"], "m1");
        assert_eq!(json["status"], "completed");
        assert_eq!(json["output"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_response_tool_call_json_computer() {
        let spec = response_tool_call_item("scr_1", "local_computer", r#"{"type":"click"}"#);
        let json = response_tool_call_json("scr_1", &spec, true);
        assert_eq!(json["type"], "computer_call");
        assert_eq!(json["id"], "cc_scr_1");
        assert_eq!(json["status"], "in_progress");
        assert!(json.get("name").is_none());
        assert_eq!(json["action"]["type"], "click");
    }

    // --- translate_cached edge cases ---

    #[tokio::test]
    async fn test_cached_text_reasoning_and_tools() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: "Done.".into(),
            reasoning: "Computing...".into(),
            tool_calls: vec![CachedToolCall {
                id: "tc_1".into(),
                name: "read_file".into(),
                arguments: r#"{"path":"/"}"#.into(),
            }],
            usage: None,
            created_at: 0,
        };
        let args = CachedArgs {
            response_id: "r_combined".into(),
            model: "test".into(),
            cached,
            sessions,
            request_input_items: vec![],
            store_response: false,
            conversation_id: None,
            response_extra: json!({}),
        };
        let bytes = axum::body::to_bytes(
            translate_cached(args).into_response().into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let events = parse_sse_events(&bytes);

        // reasoning (3) + message (3) + tool item (3) = 9 + created + completed = 11
        assert_eq!(events.len(), 11);
        assert_eq!(events[0].0, "response.created");
        // reasoning
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "reasoning_summary");
        assert_eq!(events[3].0, "response.output_item.done");
        assert_eq!(events[3].1["item"]["type"], "reasoning");
        // message
        assert_eq!(events[4].0, "response.output_item.added");
        assert_eq!(events[4].1["item"]["type"], "message");
        assert_eq!(events[6].0, "response.output_item.done");
        assert_eq!(events[6].1["item"]["type"], "message");
        assert_eq!(events[6].1["item"]["content"][0]["text"], "Done.");
        // tool call
        assert_eq!(events[7].0, "response.output_item.added");
        assert_eq!(events[7].1["item"]["type"], "function_call");
        assert_eq!(events[7].1["item"]["name"], "read_file");
        assert_eq!(events[9].0, "response.output_item.done");
        assert_eq!(events[9].1["item"]["type"], "function_call");
        // completed
        assert_eq!(events[10].0, "response.completed");
        assert_eq!(
            events[10].1["response"]["output"].as_array().unwrap().len(),
            3
        );
    }

    #[tokio::test]
    async fn test_cached_multiple_tool_calls() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: vec![
                CachedToolCall {
                    id: "c1".into(),
                    name: "exec_command".into(),
                    arguments: r#"{"cmd":"ls"}"#.into(),
                },
                CachedToolCall {
                    id: "c2".into(),
                    name: "read_file".into(),
                    arguments: r#"{"path":"/tmp"}"#.into(),
                },
                CachedToolCall {
                    id: "c3".into(),
                    name: "write_file".into(),
                    arguments: r#"{"path":"/tmp/x","content":"hi"}"#.into(),
                },
            ],
            usage: None,
            created_at: 0,
        };
        let bytes = axum::body::to_bytes(
            translate_cached(CachedArgs {
                response_id: "r_multitool".into(),
                model: "test".into(),
                cached,
                sessions,
                request_input_items: vec![],
                store_response: false,
                conversation_id: None,
                response_extra: json!({}),
            })
            .into_response()
            .into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let events = parse_sse_events(&bytes);

        // created(1) + 3 tools × (added+delta+done) + completed(1) = 11
        assert_eq!(events.len(), 11);
        assert_eq!(events[0].0, "response.created");
        // Verify each tool call
        for i in 0..3 {
            let base = 1 + i * 3;
            assert_eq!(events[base].0, "response.output_item.added");
            assert_eq!(events[base].1["item"]["type"], "function_call");
            assert_eq!(events[base + 2].0, "response.output_item.done");
        }
        assert_eq!(events[10].0, "response.completed");
        assert_eq!(
            events[10].1["response"]["output"].as_array().unwrap().len(),
            3
        );
    }

    #[tokio::test]
    async fn test_cached_with_usage() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: "Hi".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: Some(CachedUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                reasoning_tokens: None,
                cache_hit_tokens: None,
                cache_miss_tokens: None,
            }),
            created_at: 0,
        };
        let bytes = axum::body::to_bytes(
            translate_cached(CachedArgs {
                response_id: "r_usage".into(),
                model: "test".into(),
                cached,
                sessions,
                request_input_items: vec![],
                store_response: false,
                conversation_id: None,
                response_extra: json!({}),
            })
            .into_response()
            .into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let events = parse_sse_events(&bytes);
        let completed = &events.last().unwrap().1;
        assert_eq!(completed["response"]["usage"]["input_tokens"], 100);
        assert_eq!(completed["response"]["usage"]["output_tokens"], 50);
        assert_eq!(completed["response"]["usage"]["total_tokens"], 150);
    }

    #[tokio::test]
    async fn test_cached_with_store_response_and_conversation() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: "Stored!".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        };
        let _bytes = axum::body::to_bytes(
            translate_cached(CachedArgs {
                response_id: "r_store".into(),
                model: "test".into(),
                cached,
                sessions: sessions.clone(),
                request_input_items: vec![
                    json!({"type": "message", "role": "user", "content": "hi"}),
                ],
                store_response: true,
                conversation_id: Some("conv_1".into()),
                response_extra: json!({"custom_field": "val"}),
            })
            .into_response()
            .into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let saved = sessions.get_response("r_store");
        assert!(saved.is_some());
        let saved = saved.unwrap();
        assert_eq!(saved["id"], "r_store");
        assert_eq!(saved["custom_field"], "val");
        assert_eq!(saved["status"], "completed");
        let input_items = sessions.get_input_items("r_store");
        assert!(input_items.is_some());
        assert_eq!(input_items.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_cached_local_computer_tool() {
        let sessions = SessionStore::new();
        let cached = CachedResponse {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: vec![CachedToolCall {
                id: "comp_1".into(),
                name: "local_computer".into(),
                arguments: r#"{"type":"screenshot","target":"screen"}"#.into(),
            }],
            usage: None,
            created_at: 0,
        };
        let bytes = axum::body::to_bytes(
            translate_cached(CachedArgs {
                response_id: "r_comp".into(),
                model: "test".into(),
                cached,
                sessions,
                request_input_items: vec![],
                store_response: false,
                conversation_id: None,
                response_extra: json!({}),
            })
            .into_response()
            .into_body(),
            usize::MAX,
        )
        .await
        .unwrap();
        let events = parse_sse_events(&bytes);

        assert_eq!(events.len(), 4); // created + added + done + completed = 4 (no arguments.delta for computer_call)
        assert_eq!(events[0].0, "response.created");
        assert_eq!(events[1].0, "response.output_item.added");
        assert_eq!(events[1].1["item"]["type"], "computer_call");
        assert_eq!(events[1].1["item"]["id"], "cc_comp_1");
        assert_eq!(events[1].1["item"]["action"]["type"], "screenshot");
        assert_eq!(events[2].0, "response.output_item.done");
        assert_eq!(events[2].1["item"]["type"], "computer_call");
        assert_eq!(events[2].1["item"]["action"]["type"], "screenshot");
        assert_eq!(events[3].0, "response.completed");
    }
}
