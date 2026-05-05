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
    cache::{CachedResponse, CachedToolCall, RequestCache, usage_to_cached},
    session::SessionStore,
    types::{ChatMessage, ChatRequest, ChatStreamChunk, ChatUsage, ModelMap, format_usage},
};

pub struct StreamArgs {
    pub client: reqwest::Client,
    pub url: String,
    pub api_key: Arc<String>,
    pub chat_req: ChatRequest,
    pub response_id: String,
    pub sessions: SessionStore,
    pub prior_messages: Vec<ChatMessage>,
    pub request_messages: Vec<ChatMessage>,
    pub model: String,
    #[allow(dead_code)]
    pub model_map: ModelMap,
    /// Optional request cache for storing completed responses
    pub cache: Option<RequestCache>,
    /// Precomputed cache key for this request
    pub cache_key: Option<u64>,
}

/// Arguments for replaying a cached response as SSE.
pub struct CachedArgs {
    pub response_id: String,
    pub model: String,
    pub cached: CachedResponse,
}

struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
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
        prior_messages,
        request_messages,
        model,
        model_map: _model_map,
        cache,
        cache_key,
    } = args;
    let msg_item_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let reasoning_item_id = format!("rsn_{}", uuid::Uuid::new_v4().simple());

    let event_stream = stream! {
        yield Ok(Event::default()
            .event("response.created")
            .data(json!({
                "type": "response.created",
                "response": { "id": &response_id, "status": "in_progress", "model": &model }
            }).to_string()));

        // Build and send the upstream request.
        // If DeepSeek rejects with "reasoning_content must be passed back"
        // (e.g. after relay restart lost in-memory reasoning state),
        // retry once with thinking disabled.
        let max_retries = 3;
        let mut attempt = 0;
        let mut delay_ms: u64 = 500;
        let upstream = loop {
            let mut builder = client.post(&url).header("Content-Type", "application/json");
            if !api_key.is_empty() {
                builder = builder.bearer_auth(api_key.as_str());
            }

            let req_to_send = if attempt > 0 && attempt == 1 {
                // First retry: disable thinking if reasoning_content error
                let mut fallback_req = chat_req.clone();
                fallback_req.thinking = Some(serde_json::json!({"type": "disabled"}));
                fallback_req.reasoning_effort = None;
                fallback_req
            } else {
                chat_req.clone()
            };

            match builder.json(&req_to_send).send().await {
                Ok(r) if r.status().is_success() => break r,
                Ok(r) => {
                    let status = r.status();
                    let status_code = status.as_u16();
                    let body = r.text().await.unwrap_or_default();

                    let retryable = matches!(status_code, 429 | 502 | 503)
                        || (status_code == 400 && body.contains("reasoning_content") && attempt == 0);

                    if retryable && attempt < max_retries {
                        attempt += 1;
                        warn!("upstream {status_code} (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms");
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                        delay_ms *= 2;
                        continue;
                    }

                    error!("upstream {}: {}", status_code, body);
                    yield Ok(Event::default().event("response.failed").data(
                        json!({"type": "response.failed", "response": {"id": &response_id, "status": "failed", "error": {"code": status_code.to_string(), "message": body}}}).to_string()
                    ));
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
                    yield Ok(Event::default().event("response.failed").data(
                        json!({"type": "response.failed", "response": {"id": &response_id, "status": "failed", "error": {"code": "connection_error", "message": e.to_string()}}}).to_string()
                    ));
                    return;
                }
            }
        };

        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: BTreeMap<usize, ToolCallAccum> = BTreeMap::new();
        let mut emitted_message_item = false;
        let mut emitted_reasoning_item = false;
        let mut injected_reasoning_prefix = false;
        let mut reasoning_separator_emitted = false;
        let mut final_usage: Option<ChatUsage> = None;
        let mut source = upstream.bytes_stream().eventsource();
        let mut stream_completed = false;

        while let Some(ev) = source.next().await {
            match ev {
                Err(e) => {
                    warn!("SSE parse error: {e}");
                    break;
                }
                Ok(ev) if ev.data.trim() == "[DONE]" => { stream_completed = true; break; }
                Ok(ev) if ev.data.is_empty() => continue,
                Ok(ev) => {
                    match serde_json::from_str::<ChatStreamChunk>(&ev.data) {
                        Err(e) => warn!("chunk parse: {} — data: {}", e, &ev.data[..ev.data.len().min(120)]),
                        Ok(chunk) => {
                            // Capture usage from final chunk (enabled via stream_options.include_usage)
                            if chunk.usage.is_some() {
                                final_usage = chunk.usage;
                            }
                            for choice in &chunk.choices {
                                if let Some(rc) = choice.delta.reasoning_content.as_deref() {
                                    if !rc.is_empty() {
                                        if !emitted_reasoning_item {
                                            yield Ok(Event::default()
                                                .event("response.output_item.added")
                                                .data(json!({
                                                    "type": "response.output_item.added",
                                                    "output_index": 0,
                                                    "item": { "type": "reasoning", "id": &reasoning_item_id, "status": "in_progress" }
                                                }).to_string()));
                                            emitted_reasoning_item = true;
                                        }
                                        accumulated_reasoning.push_str(rc);
                                        // Emit reasoning delta for the reasoning item (index 0)
                                        yield Ok(Event::default()
                                            .event("response.reasoning_text.delta")
                                            .data(json!({
                                                "type": "response.reasoning_text.delta",
                                                "item_id": &reasoning_item_id,
                                                "output_index": 0,
                                                "content_index": 0,
                                                "delta": rc
                                            }).to_string()));
                                        // Also inject reasoning into message text for Codex visibility
                                        if !injected_reasoning_prefix {
                                            if !emitted_message_item {
                                                yield Ok(Event::default()
                                                    .event("response.output_item.added")
                                                    .data(json!({
                                                        "type": "response.output_item.added",
                                                        "output_index": 1,
                                                        "item": { "type": "message", "id": &msg_item_id, "role": "assistant", "content": [], "status": "in_progress" }
                                                    }).to_string()));
                                                emitted_message_item = true;
                                            }
                                            let prefix = "💭 思考过程:\n";
                                            accumulated_text.push_str(prefix);
                                            yield Ok(Event::default()
                                                .event("response.output_text.delta")
                                                .data(json!({
                                                    "type": "response.output_text.delta",
                                                    "item_id": &msg_item_id,
                                                    "output_index": 1,
                                                    "content_index": 0,
                                                    "delta": prefix
                                                }).to_string()));
                                            injected_reasoning_prefix = true;
                                        }
                                        accumulated_text.push_str(rc);
                                        yield Ok(Event::default()
                                            .event("response.output_text.delta")
                                            .data(json!({
                                                "type": "response.output_text.delta",
                                                "item_id": &msg_item_id,
                                                "output_index": 1,
                                                "content_index": 0,
                                                "delta": rc
                                            }).to_string()));
                                    }
                                }
                                let content = choice.delta.content.as_deref().unwrap_or("");
                                if !content.is_empty() {
                                    // Insert separator between reasoning and response
                                    if emitted_reasoning_item && !reasoning_separator_emitted {
                                        let sep = "\n\n";
                                        accumulated_text.push_str(sep);
                                        yield Ok(Event::default()
                                            .event("response.output_text.delta")
                                            .data(json!({
                                                "type": "response.output_text.delta",
                                                "item_id": &msg_item_id,
                                                "output_index": 1,
                                                "content_index": 0,
                                                "delta": sep
                                            }).to_string()));
                                        reasoning_separator_emitted = true;
                                    }
                                    if !emitted_message_item {
                                        let msg_oi: usize = if emitted_reasoning_item { 1 } else { 0 };
                                        yield Ok(Event::default()
                                            .event("response.output_item.added")
                                            .data(json!({
                                                "type": "response.output_item.added",
                                                "output_index": msg_oi,
                                                "item": { "type": "message", "id": &msg_item_id, "role": "assistant", "content": [], "status": "in_progress" }
                                            }).to_string()));
                                        emitted_message_item = true;
                                    }
                                    accumulated_text.push_str(content);
                                    yield Ok(Event::default()
                                        .event("response.output_text.delta")
                                        .data(json!({
                                            "type": "response.output_text.delta",
                                            "item_id": &msg_item_id,
                                            "output_index": if emitted_reasoning_item { 1 } else { 0 },
                                            "content_index": 0,
                                            "delta": content
                                        }).to_string()));
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
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Log streaming token usage
        let usage_str = format_usage(final_usage.as_ref());
        info!("↑ done {}", usage_str);

        // Clone for cache before moving into completion_usage
        let cache_usage = final_usage.clone();

        // Build usage for response.completed
        let completion_usage = final_usage.map(|u| json!({
            "input_tokens": u.prompt_tokens,
            "output_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens
        }));

        // Close message item
        if emitted_message_item {
            let msg_output_index: usize = if emitted_reasoning_item { 1 } else { 0 };
            yield Ok(Event::default()
                .event("response.output_item.done")
                .data(json!({
                    "type": "response.output_item.done",
                    "output_index": msg_output_index,
                    "item": {
                        "type": "message",
                        "id": &msg_item_id,
                        "role": "assistant",
                        "status": "completed",
                        "content": [{"type": "output_text", "text": &accumulated_text}]
                    }
                }).to_string()));
        }

        // Emit function_call items
        let base_index: usize = if emitted_reasoning_item { 1 } else { 0 }
            + if emitted_message_item { 1 } else { 0 };
        let mut fc_items: Vec<Value> = Vec::new();

        for (rel_idx, (_, tc)) in tool_calls.iter().enumerate() {
            let fc_item_id = format!("fc_{}", uuid::Uuid::new_v4().simple());
            let output_index = base_index + rel_idx;
            let arguments = tc.arguments.clone();
            // apply_patch → exec_command transparent translation
            let tool_name = if tc.name == "apply_patch" { "exec_command" } else { tc.name.as_str() };

            yield Ok(Event::default()
                .event("response.output_item.added")
                .data(json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "type": "function_call",
                        "id": &fc_item_id,
                        "call_id": &tc.id,
                        "name": tool_name,
                        "arguments": "",
                        "status": "in_progress"
                    }
                }).to_string()));

            if !arguments.is_empty() {
                yield Ok(Event::default()
                    .event("response.function_call_arguments.delta")
                    .data(json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": &fc_item_id,
                        "output_index": output_index,
                        "delta": &arguments
                    }).to_string()));
            }

            yield Ok(Event::default()
                .event("response.output_item.done")
                .data(json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "function_call",
                        "id": &fc_item_id,
                        "call_id": &tc.id,
                        "name": tool_name,
                        "arguments": &arguments,
                        "status": "completed"
                    }
                }).to_string()));

            fc_items.push(json!({
                "type": "function_call",
                "id": fc_item_id,
                "call_id": &tc.id,
                "name": tool_name,
                "arguments": &arguments,
                "status": "completed"
            }));
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
            tool_calls: assistant_tool_calls,
            tool_call_id: None,
            name: None,
        };

        if !accumulated_reasoning.is_empty() {
            sessions.store_turn_reasoning(&request_messages, &assistant_msg, accumulated_reasoning.clone());
        }

        let mut messages = prior_messages;
        messages.push(assistant_msg);
        sessions.save_with_id(response_id.clone(), messages);

        // Build output for response.completed
        let mut output_items: Vec<Value> = Vec::new();
        if emitted_reasoning_item {
            output_items.push(json!({
                "type": "reasoning",
                "id": &reasoning_item_id,
                "status": "completed",
                "content": [{"type": "reasoning_text", "text": &accumulated_reasoning}]
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

        yield Ok(Event::default()
            .event("response.completed")
            .data(json!({
                "type": "response.completed",
                "response": response_obj
            }).to_string()));

        // Store in request cache (only if stream completed normally)
        if stream_completed {
            if let (Some(c), Some(key)) = (cache, cache_key) {
            let cached = CachedResponse {
                text: accumulated_text.clone(),
                reasoning: accumulated_reasoning.clone(),
                tool_calls: tool_calls.values().map(|tc| CachedToolCall {
                    id: tc.id.clone(),
                    name: if tc.name == "apply_patch" { "exec_command".into() } else { tc.name.clone() },
                    arguments: tc.arguments.clone(),
                }).collect(),
                usage: usage_to_cached(cache_usage.as_ref()),
                created_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            };
            c.insert(key, cached);
            }
        }
    };

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

/// Replay a cached response as a full SSE event stream.
pub fn translate_cached(
    args: CachedArgs,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let CachedArgs { response_id, model, cached } = args;
    let msg_item_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let reasoning_item_id = format!("rsn_{}", uuid::Uuid::new_v4().simple());

    let event_stream = stream! {
        yield Ok(Event::default()
            .event("response.created")
            .data(json!({
                "type": "response.created",
                "response": { "id": &response_id, "status": "in_progress", "model": &model }
            }).to_string()));

        let mut output_index: usize = 0;

        // Reasoning item
        if !cached.reasoning.is_empty() {
            yield Ok(Event::default()
                .event("response.output_item.added")
                .data(json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": { "type": "reasoning", "id": &reasoning_item_id, "status": "in_progress" }
                }).to_string()));

            yield Ok(Event::default()
                .event("response.reasoning_text.delta")
                .data(json!({
                    "type": "response.reasoning_text.delta",
                    "item_id": &reasoning_item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "delta": &cached.reasoning
                }).to_string()));

            yield Ok(Event::default()
                .event("response.output_item.done")
                .data(json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "reasoning",
                        "id": &reasoning_item_id,
                        "status": "completed",
                        "content": [{"type": "reasoning_text", "text": &cached.reasoning}]
                    }
                }).to_string()));

            output_index += 1;
        }

        // Message item
        if !cached.text.is_empty() || cached.tool_calls.is_empty() {
            yield Ok(Event::default()
                .event("response.output_item.added")
                .data(json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": { "type": "message", "id": &msg_item_id, "role": "assistant", "content": [], "status": "in_progress" }
                }).to_string()));

            if !cached.text.is_empty() {
                yield Ok(Event::default()
                    .event("response.output_text.delta")
                    .data(json!({
                        "type": "response.output_text.delta",
                        "item_id": &msg_item_id,
                        "output_index": output_index,
                        "content_index": 0,
                        "delta": &cached.text
                    }).to_string()));
            }

            yield Ok(Event::default()
                .event("response.output_item.done")
                .data(json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "type": "message",
                        "id": &msg_item_id,
                        "role": "assistant",
                        "status": "completed",
                        "content": [{"type": "output_text", "text": &cached.text}]
                    }
                }).to_string()));

            output_index += 1;
        }

        // Tool call items
        for tc in &cached.tool_calls {
            let fc_item_id = format!("fc_{}", uuid::Uuid::new_v4().simple());

            yield Ok(Event::default()
                .event("response.output_item.added")
                .data(json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": { "type": "function_call", "id": &fc_item_id, "call_id": &tc.id, "name": &tc.name, "arguments": "", "status": "in_progress" }
                }).to_string()));

            if !tc.arguments.is_empty() {
                yield Ok(Event::default()
                    .event("response.function_call_arguments.delta")
                    .data(json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": &fc_item_id,
                        "output_index": output_index,
                        "delta": &tc.arguments
                    }).to_string()));
            }

            yield Ok(Event::default()
                .event("response.output_item.done")
                .data(json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": { "type": "function_call", "id": &fc_item_id, "call_id": &tc.id, "name": &tc.name, "arguments": &tc.arguments, "status": "completed" }
                }).to_string()));

            output_index += 1;
        }

        // Build output and usage
        let mut output_items: Vec<Value> = Vec::new();
        if !cached.reasoning.is_empty() {
            output_items.push(json!({
                "type": "reasoning", "id": &reasoning_item_id, "status": "completed",
                "content": [{"type": "reasoning_text", "text": &cached.reasoning}]
            }));
        }
        if !cached.text.is_empty() || cached.tool_calls.is_empty() {
            output_items.push(json!({
                "type": "message", "id": &msg_item_id, "role": "assistant", "status": "completed",
                "content": [{"type": "output_text", "text": &cached.text}]
            }));
        }
        for tc in &cached.tool_calls {
            output_items.push(json!({
                "type": "function_call", "id": format!("fc_{}", uuid::Uuid::new_v4().simple()),
                "call_id": &tc.id, "name": &tc.name, "arguments": &tc.arguments, "status": "completed"
            }));
        }

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

        yield Ok(Event::default()
            .event("response.completed")
            .data(json!({
                "type": "response.completed",
                "response": response_obj
            }).to_string()));

        info!("request cache: replayed (text={}b reasoning={}b tools={})",
            cached.text.len(), cached.reasoning.len(), cached.tool_calls.len());
    };

    Sse::new(event_stream).keep_alive(KeepAlive::default())
}
