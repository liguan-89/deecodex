use crate::cache::RequestCache;
use crate::executor::{ComputerActionInvocation, LocalExecutorConfig, McpToolInvocation};
use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;
use crate::session::SessionStore;
use crate::token_anomaly::TokenTracker;
use crate::types::*;
use crate::utils::{limit_function_call_outputs, merge_response_extra};
use crate::{files, prompts, stream, translate, vector_stores};
use anyhow::{bail, Result};
use axum::{
    extract::{Multipart, Path, Query, Request, State},
    http::header,
    http::StatusCode,
    middleware::{from_fn_with_state, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use reqwest::{Client, Url};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

const LOCAL_OUTPUT_PREFIX_ITEMS_KEY: &str = "x_deecodex_local_output_prefix_items";

#[derive(Clone)]
pub struct AppState {
    pub sessions: SessionStore,
    pub client: Client,
    pub upstream: Arc<Url>,
    pub api_key: Arc<String>,
    pub client_api_key: Arc<String>,
    pub model_map: Arc<ModelMap>,
    pub vision_upstream: Option<Arc<Url>>,
    pub vision_api_key: Arc<String>,
    pub vision_model: Arc<String>,
    pub vision_endpoint: Arc<String>,
    pub start_time: std::time::Instant,
    pub request_cache: RequestCache,
    pub prompts: Arc<prompts::PromptRegistry>,
    pub files: files::FileStore,
    pub vector_stores: vector_stores::VectorStoreRegistry,
    pub background_tasks: Arc<dashmap::DashMap<String, tokio::task::JoinHandle<()>>>,
    pub chinese_thinking: bool,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub metrics: Arc<Metrics>,
    pub tool_policy: ToolPolicy,
    pub executors: Arc<LocalExecutorConfig>,
    pub token_tracker: Arc<TokenTracker>,
}

#[derive(Clone, Debug, Default)]
pub struct ToolPolicy {
    pub allowed_mcp_servers: Vec<String>,
    pub allowed_computer_displays: Vec<String>,
}

struct BlockingArgs<'a> {
    state: AppState,
    chat_req: ChatRequest,
    url: String,
    model: String,
    api_key: Arc<String>,
    response_id: String,
    store_response: bool,
    conversation_id: Option<String>,
    response_extra: Value,
    req: &'a ResponsesRequest,
}

struct VlmArgs {
    state: AppState,
    url: String,
    api_key: Arc<String>,
    vlm_body: Value,
    model: String,
    stream_response: bool,
    store_response: bool,
    request_input_items: Vec<Value>,
    response_extra: Value,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/responses", post(handle_responses))
        .route("/v1/responses/compact", post(handle_compact_response))
        .route("/v1/responses/input_tokens", post(handle_input_tokens))
        .route(
            "/v1/responses/:response_id",
            get(handle_get_response).delete(handle_delete_response),
        )
        .route(
            "/v1/responses/:response_id/cancel",
            post(handle_cancel_response),
        )
        .route(
            "/v1/responses/:response_id/input_items",
            get(handle_input_items),
        )
        .route("/v1/prompts", get(handle_list_prompts))
        .route("/v1/prompts/:prompt_id", get(handle_get_prompt))
        .route("/v1/files", get(handle_list_files).post(handle_create_file))
        .route(
            "/v1/files/:file_id",
            get(handle_get_file).delete(handle_delete_file),
        )
        .route("/v1/files/:file_id/content", get(handle_get_file_content))
        .route(
            "/v1/vector_stores",
            get(handle_list_vector_stores).post(handle_create_vector_store),
        )
        .route(
            "/v1/vector_stores/:vector_store_id",
            get(handle_get_vector_store).delete(handle_delete_vector_store),
        )
        .route(
            "/v1/vector_stores/:vector_store_id/files",
            get(handle_list_vector_store_files).post(handle_create_vector_store_file),
        )
        .route(
            "/v1/vector_stores/:vector_store_id/files/:file_id",
            get(handle_get_vector_store_file).delete(handle_delete_vector_store_file),
        )
        .route(
            "/v1/vector_stores/:vector_store_id/file_batches",
            post(handle_create_vector_store_file_batch),
        )
        .route(
            "/v1/vector_stores/:vector_store_id/file_batches/:batch_id",
            get(handle_get_vector_store_file_batch),
        )
        .route(
            "/v1/vector_stores/:vector_store_id/file_batches/:batch_id/cancel",
            post(handle_cancel_vector_store_file_batch),
        )
        .route(
            "/v1/vector_stores/:vector_store_id/file_batches/:batch_id/files",
            get(handle_list_vector_store_file_batch_files),
        )
        .route("/v1/conversations", post(handle_create_conversation))
        .route(
            "/v1/conversations/:conversation_id",
            get(handle_get_conversation).delete(handle_delete_conversation),
        )
        .route(
            "/v1/conversations/:conversation_id/items",
            get(handle_conversation_items),
        )
        .route("/v1/models", get(handle_models))
        .route("/health", get(handle_health))
        .route("/v1", get(handle_v1))
        .route("/metrics", get(handle_metrics))
        .fallback(handle_fallback)
        .layer(from_fn_with_state(state.clone(), require_client_auth))
        .with_state(state)
}

pub fn validate_upstream(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim_end_matches('/'))?;
    match url.scheme() {
        "http" | "https" => {}
        s => bail!("upstream URL scheme must be http or https, got: {s}"),
    }
    if url.host_str().is_none() {
        bail!("upstream URL must have a host");
    }
    Ok(url)
}

fn join_base(url: &Url) -> String {
    let s = url.as_str();
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

async fn handle_health(State(state): State<AppState>) -> Response {
    let uptime = state.start_time.elapsed().as_secs();
    Json(serde_json::json!({
        "status": "ok",
        "uptime_secs": uptime,
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn handle_v1() -> Response {
    Json(serde_json::json!({"status": "ok"})).into_response()
}

async fn handle_metrics(State(state): State<AppState>) -> Response {
    let body = state.metrics.gather();
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}

async fn require_client_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/health" || path == "/v1" {
        return next.run(req).await;
    }
    if state.client_api_key.is_empty() {
        return next.run(req).await;
    }
    let authorized = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| authorization_matches(value, state.client_api_key.as_str()));
    if authorized {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "Missing or invalid Authorization header",
                    "type": "invalid_request_error",
                    "code": "invalid_api_key"
                }
            })),
        )
            .into_response()
    }
}

fn authorization_matches(header_value: &str, expected: &str) -> bool {
    header_value
        .strip_prefix("Bearer ")
        .or_else(|| header_value.strip_prefix("bearer "))
        .unwrap_or(header_value)
        .trim()
        == expected
}

#[derive(Debug, Deserialize)]
struct RetrieveResponseQuery {
    #[serde(default)]
    stream: bool,
    #[serde(default, deserialize_with = "deserialize_optional_include")]
    include: Option<Vec<String>>,
    #[serde(default)]
    include_obfuscation: Option<bool>,
    #[serde(default)]
    starting_after: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct InputItemsQuery {
    #[serde(default)]
    after: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    order: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_include")]
    include: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IncludeQueryValue {
    One(String),
    Many(Vec<String>),
}

fn deserialize_optional_include<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<IncludeQueryValue>::deserialize(deserializer)?;
    Ok(value.map(|value| match value {
        IncludeQueryValue::One(item) => vec![item],
        IncludeQueryValue::Many(items) => items,
    }))
}

#[derive(Debug, Deserialize, Default)]
struct CreateVectorStoreRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    file_ids: Vec<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct CreateVectorStoreFileRequest {
    file_id: String,
}

#[derive(Debug, Deserialize, Default)]
struct CreateVectorStoreFileBatchRequest {
    #[serde(default)]
    file_ids: Vec<String>,
}

async fn handle_get_response(
    State(state): State<AppState>,
    Path(response_id): Path<String>,
    Query(query): Query<RetrieveResponseQuery>,
) -> Response {
    if let Some(response) = validate_response_include(query.include.as_deref()) {
        return response;
    }
    let _ = query.starting_after;
    let Some(response) = state.sessions.get_response(&response_id) else {
        return response_not_found(&response_id);
    };
    if query.stream {
        return replay_response_stream(
            response,
            query.starting_after,
            query.include_obfuscation.unwrap_or(false),
        )
        .into_response();
    }
    Json(response).into_response()
}

fn replay_response_stream(
    response: Value,
    starting_after: Option<u64>,
    include_obfuscation: bool,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let mut events = Vec::new();
    let mut sequence_number = 0_u64;
    let response_id = response
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp_unknown");
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown");

    push_replay_event(
        &mut events,
        "response.created",
        json!({
            "type": "response.created",
            "response": {"id": response_id, "object": "response", "status": "in_progress", "model": model}
        }),
        include_obfuscation,
        &mut sequence_number,
    );

    if let Some(output) = response.get("output").and_then(Value::as_array) {
        for (idx, item) in output.iter().enumerate() {
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
            let mut added_item = item.clone();
            added_item["status"] = json!("in_progress");
            if item_type == "message" {
                added_item["content"] = json!([]);
            } else if item_type == "function_call" {
                added_item["arguments"] = json!("");
            }
            push_replay_event(
                &mut events,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": idx,
                    "item": added_item
                }),
                include_obfuscation,
                &mut sequence_number,
            );

            match item_type {
                "message" => {
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for (content_idx, part) in content.iter().enumerate() {
                            if part.get("type").and_then(Value::as_str) == Some("output_text") {
                                let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                                if !text.is_empty() {
                                    push_replay_event(
                                        &mut events,
                                        "response.output_text.delta",
                                        json!({
                                            "type": "response.output_text.delta",
                                            "item_id": item.get("id").and_then(Value::as_str).unwrap_or(""),
                                            "output_index": idx,
                                            "content_index": content_idx,
                                            "delta": text
                                        }),
                                        include_obfuscation,
                                        &mut sequence_number,
                                    );
                                }
                            }
                        }
                    }
                }
                "reasoning" => {
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for (content_idx, part) in content.iter().enumerate() {
                            if part
                                .get("type")
                                .and_then(Value::as_str)
                                .is_some_and(|s| s == "reasoning_text" || s == "summary_text")
                            {
                                let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                                if !text.is_empty() {
                                    push_replay_event(
                                        &mut events,
                                        "response.reasoning_summary_text.delta",
                                        json!({
                                            "type": "response.reasoning_summary_text.delta",
                                            "item_id": item.get("id").and_then(Value::as_str).unwrap_or(""),
                                            "output_index": idx,
                                            "content_index": content_idx,
                                            "delta": text
                                        }),
                                        include_obfuscation,
                                        &mut sequence_number,
                                    );
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                    if !arguments.is_empty() {
                        push_replay_event(
                            &mut events,
                            "response.function_call_arguments.delta",
                            json!({
                                "type": "response.function_call_arguments.delta",
                                "item_id": item.get("id").and_then(Value::as_str).unwrap_or(""),
                                "output_index": idx,
                                "delta": arguments
                            }),
                            include_obfuscation,
                            &mut sequence_number,
                        );
                    }
                }
                _ => {}
            }

            push_replay_event(
                &mut events,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": idx,
                    "item": item
                }),
                include_obfuscation,
                &mut sequence_number,
            );
        }
    }

    push_replay_event(
        &mut events,
        "response.completed",
        json!({
            "type": "response.completed",
            "response": response
        }),
        include_obfuscation,
        &mut sequence_number,
    );

    let events = events
        .into_iter()
        .enumerate()
        .filter_map(move |(idx, event)| {
            let sequence_number = idx as u64 + 1;
            if starting_after.is_some_and(|after| sequence_number <= after) {
                None
            } else {
                Some(Ok(event))
            }
        });
    Sse::new(futures_util::stream::iter(events)).keep_alive(KeepAlive::default())
}

fn push_replay_event(
    events: &mut Vec<Event>,
    name: &'static str,
    mut payload: Value,
    include_obfuscation: bool,
    sequence_number: &mut u64,
) {
    *sequence_number += 1;
    payload["sequence_number"] = json!(*sequence_number);
    if include_obfuscation {
        payload["obfuscation"] = json!("relay_replay");
    }
    events.push(Event::default().event(name).data(payload.to_string()));
}

async fn handle_delete_response(
    State(state): State<AppState>,
    Path(response_id): Path<String>,
) -> Response {
    if state.sessions.delete_response(&response_id) {
        Json(json!({
            "id": response_id,
            "object": "response.deleted",
            "deleted": true
        }))
        .into_response()
    } else {
        response_not_found(&response_id)
    }
}

async fn handle_cancel_response(
    State(state): State<AppState>,
    Path(response_id): Path<String>,
) -> Response {
    let Some(mut response) = state.sessions.get_response(&response_id) else {
        return response_not_found(&response_id);
    };
    let status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    if status == "in_progress" || status == "queued" {
        response["status"] = json!("cancelled");
        if let Some((_, task)) = state.background_tasks.remove(&response_id) {
            task.abort();
        }
        state
            .sessions
            .save_response(response_id.to_string(), response.clone());
        Json(response).into_response()
    } else {
        (
            StatusCode::CONFLICT,
            Json(json!({
                "error": {
                    "message": format!("Response {response_id} cannot be cancelled because it is {status}"),
                    "type": "invalid_request_error",
                    "code": "response_not_cancellable"
                }
            })),
        )
            .into_response()
    }
}

async fn handle_input_items(
    State(state): State<AppState>,
    Path(response_id): Path<String>,
    Query(query): Query<InputItemsQuery>,
) -> Response {
    let Some(items) = state.sessions.get_input_items(&response_id) else {
        return response_not_found(&response_id);
    };
    list_items_response(items, query, "input item")
}

fn list_items_response(
    mut items: Vec<Value>,
    query: InputItemsQuery,
    item_label: &str,
) -> Response {
    let _ = &query.include;
    match query.order.as_deref().unwrap_or("desc") {
        "asc" => {}
        "desc" => items.reverse(),
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": format!("Invalid order: {other}"),
                        "type": "invalid_request_error",
                        "param": "order",
                        "code": "invalid_order"
                    }
                })),
            )
                .into_response();
        }
    }
    if let Some(after) = query.after {
        if let Some(pos) = items
            .iter()
            .position(|item| item.get("id").and_then(Value::as_str) == Some(after.as_str()))
        {
            items = items.into_iter().skip(pos + 1).collect();
        } else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": format!("No {item_label} found with id {after}"),
                        "type": "invalid_request_error",
                        "param": "after",
                        "code": "cursor_not_found"
                    }
                })),
            )
                .into_response();
        }
    }
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let has_more = items.len() > limit;
    items.truncate(limit);
    let first_id = items
        .first()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let last_id = items
        .last()
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Json(json!({
        "object": "list",
        "data": items,
        "first_id": first_id,
        "last_id": last_id,
        "has_more": has_more
    }))
    .into_response()
}

async fn handle_input_tokens(body: axum::body::Bytes) -> Response {
    let tokens = match serde_json::from_slice::<ResponsesRequest>(&body) {
        Ok(req) => count_input_tokens(&req),
        Err(_) => match serde_json::from_slice::<Value>(&body) {
            Ok(value) => count_value_tokens(&value),
            Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
        },
    };
    Json(json!({
        "object": "response.input_tokens",
        "input_tokens": tokens
    }))
    .into_response()
}

async fn handle_compact_response(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    let req: ResponsesRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
    };
    let response_id = state.sessions.new_id();
    let mut input_items = if let Some(id) = req.previous_response_id.as_deref() {
        state.sessions.get_input_items(id).unwrap_or_default()
    } else if let Some(id) = conversation_id_from_request(&req) {
        state.sessions.get_conversation_items(&id)
    } else {
        Vec::new()
    };
    input_items.extend(response_input_items(&req));
    let compacted = json!({
        "id": response_id,
        "object": "response.compacted",
        "model": req.model,
        "input": input_items.clone(),
        "instructions": req.instructions.or(req.system),
        "status": "completed",
        "created_at": now_unix_secs()
    });
    if req.store.unwrap_or(true) {
        state
            .sessions
            .save_input_items(response_id.clone(), input_items.clone());
        state
            .sessions
            .save_response(response_id.clone(), compacted.clone());
    }
    Json(compacted).into_response()
}

async fn handle_create_conversation(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    let body_value = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| json!({}));
    let conversation_id = body_value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("conv_{}", uuid::Uuid::new_v4().simple()));
    state
        .sessions
        .save_conversation(conversation_id.clone(), Vec::new());
    state
        .sessions
        .save_conversation_items(conversation_id.clone(), Vec::new());
    Json(json!({
        "id": conversation_id,
        "object": "conversation",
        "created_at": now_unix_secs(),
        "metadata": body_value.get("metadata").cloned().unwrap_or_else(|| json!({}))
    }))
    .into_response()
}

async fn handle_get_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
) -> Response {
    if !state.sessions.conversation_exists(&conversation_id) {
        return conversation_not_found(&conversation_id);
    }
    Json(json!({
        "id": conversation_id,
        "object": "conversation"
    }))
    .into_response()
}

async fn handle_delete_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
) -> Response {
    if state.sessions.delete_conversation(&conversation_id) {
        Json(json!({
            "id": conversation_id,
            "object": "conversation.deleted",
            "deleted": true
        }))
        .into_response()
    } else {
        conversation_not_found(&conversation_id)
    }
}

async fn handle_conversation_items(
    State(state): State<AppState>,
    Path(conversation_id): Path<String>,
    Query(query): Query<InputItemsQuery>,
) -> Response {
    if !state.sessions.conversation_exists(&conversation_id) {
        return conversation_not_found(&conversation_id);
    }
    list_items_response(
        state.sessions.get_conversation_items(&conversation_id),
        query,
        "conversation item",
    )
}

fn response_not_found(response_id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": {
                "message": format!("No response found with id {response_id}"),
                "type": "invalid_request_error",
                "param": "response_id",
                "code": "not_found"
            }
        })),
    )
        .into_response()
}

fn conversation_not_found(conversation_id: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": {
                "message": format!("No conversation found with id {conversation_id}"),
                "type": "invalid_request_error",
                "param": "conversation_id",
                "code": "not_found"
            }
        })),
    )
        .into_response()
}

fn unsupported_param(param: &str, message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "message": message,
                "type": "unsupported_feature",
                "param": param,
                "code": "unsupported_feature"
            }
        })),
    )
        .into_response()
}

async fn handle_models(State(state): State<AppState>) -> Response {
    debug!("GET /v1/models");
    let url = format!("{}models", join_base(&state.upstream));
    let mut builder = state.client.get(&url);
    if !state.api_key.is_empty() {
        builder = builder.bearer_auth(state.api_key.as_str());
    }
    match builder.send().await {
        Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>().await {
            Ok(body) => Json(body).into_response(),
            Err(e) => {
                warn!("upstream models parse error: {e}");
                Json(serde_json::json!({ "object": "list", "data": [] })).into_response()
            }
        },
        Ok(r) => {
            warn!("upstream models: status {}", r.status());
            Json(serde_json::json!({ "object": "list", "data": [] })).into_response()
        }
        Err(e) => {
            warn!("upstream models request error: {e}");
            Json(serde_json::json!({ "object": "list", "data": [] })).into_response()
        }
    }
}

async fn handle_list_prompts(State(state): State<AppState>) -> Response {
    Json(state.prompts.list_prompts()).into_response()
}

async fn handle_get_prompt(
    State(state): State<AppState>,
    Path(prompt_id): Path<String>,
) -> Response {
    match state.prompts.retrieve_prompt(&prompt_id) {
        Ok(prompt) => Json(prompt).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_list_files(State(state): State<AppState>) -> Response {
    Json(state.files.list()).into_response()
}

async fn handle_get_file(State(state): State<AppState>, Path(file_id): Path<String>) -> Response {
    match state.files.get_object(&file_id) {
        Ok(file) => Json(file).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_get_file_content(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> Response {
    match state.files.get_content(&file_id) {
        Ok((bytes, content_type)) => {
            let headers = [(header::CONTENT_TYPE, content_type)];
            (headers, bytes).into_response()
        }
        Err(err) => err.into_response(),
    }
}

async fn handle_delete_file(
    State(state): State<AppState>,
    Path(file_id): Path<String>,
) -> Response {
    match state.files.delete(&file_id) {
        Ok(deleted) => Json(deleted).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_create_file(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let mut purpose = "assistants".to_string();
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename = "file".to_string();
    let mut content_type = "application/octet-stream".to_string();

    while let Some(field) = match multipart.next_field().await {
        Ok(field) => field,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": format!("Invalid multipart upload: {e}"),
                        "type": "invalid_request_error",
                        "param": "file",
                        "code": "invalid_multipart"
                    }
                })),
            )
                .into_response()
        }
    } {
        let name = field.name().unwrap_or("").to_string();
        if name == "purpose" {
            purpose = field.text().await.unwrap_or_else(|_| purpose.clone());
            continue;
        }
        if name == "file" {
            filename = field.file_name().unwrap_or("file").to_string();
            content_type = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();
            match field.bytes().await {
                Ok(bytes) => file_bytes = Some(bytes.to_vec()),
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": {
                                "message": format!("Failed to read uploaded file: {e}"),
                                "type": "invalid_request_error",
                                "param": "file",
                                "code": "invalid_file"
                            }
                        })),
                    )
                        .into_response()
                }
            }
        }
    }

    let Some(bytes) = file_bytes else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "multipart field `file` is required",
                    "type": "invalid_request_error",
                    "param": "file",
                    "code": "missing_file"
                }
            })),
        )
            .into_response();
    };

    match state
        .files
        .insert(filename, purpose, content_type, bytes, now_unix_secs())
    {
        Ok(file) => Json(file).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_list_vector_stores(State(state): State<AppState>) -> Response {
    Json(state.vector_stores.list()).into_response()
}

async fn handle_create_vector_store(
    State(state): State<AppState>,
    Json(req): Json<CreateVectorStoreRequest>,
) -> Response {
    Json(state.vector_stores.create(
        req.name,
        req.file_ids,
        req.metadata.unwrap_or_else(|| json!({})),
        now_unix_secs(),
    ))
    .into_response()
}

async fn handle_get_vector_store(
    State(state): State<AppState>,
    Path(vector_store_id): Path<String>,
) -> Response {
    match state.vector_stores.get(&vector_store_id) {
        Ok(store) => Json(store).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_delete_vector_store(
    State(state): State<AppState>,
    Path(vector_store_id): Path<String>,
) -> Response {
    match state.vector_stores.delete(&vector_store_id) {
        Ok(deleted) => Json(deleted).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_create_vector_store_file(
    State(state): State<AppState>,
    Path(vector_store_id): Path<String>,
    Json(req): Json<CreateVectorStoreFileRequest>,
) -> Response {
    match state.vector_stores.add_file(&vector_store_id, req.file_id) {
        Ok(file) => Json(file).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_list_vector_store_files(
    State(state): State<AppState>,
    Path(vector_store_id): Path<String>,
) -> Response {
    match state.vector_stores.list_files(&vector_store_id) {
        Ok(files) => Json(files).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_get_vector_store_file(
    State(state): State<AppState>,
    Path((vector_store_id, file_id)): Path<(String, String)>,
) -> Response {
    match state.vector_stores.get_file(&vector_store_id, &file_id) {
        Ok(file) => Json(file).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_delete_vector_store_file(
    State(state): State<AppState>,
    Path((vector_store_id, file_id)): Path<(String, String)>,
) -> Response {
    match state.vector_stores.delete_file(&vector_store_id, &file_id) {
        Ok(deleted) => Json(deleted).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_create_vector_store_file_batch(
    State(state): State<AppState>,
    Path(vector_store_id): Path<String>,
    Json(req): Json<CreateVectorStoreFileBatchRequest>,
) -> Response {
    match state
        .vector_stores
        .create_batch(&vector_store_id, req.file_ids, now_unix_secs())
    {
        Ok(batch) => Json(batch).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_get_vector_store_file_batch(
    State(state): State<AppState>,
    Path((vector_store_id, batch_id)): Path<(String, String)>,
) -> Response {
    match state.vector_stores.get_batch(&vector_store_id, &batch_id) {
        Ok(batch) => Json(batch).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_cancel_vector_store_file_batch(
    State(state): State<AppState>,
    Path((vector_store_id, batch_id)): Path<(String, String)>,
) -> Response {
    match state
        .vector_stores
        .cancel_batch(&vector_store_id, &batch_id)
    {
        Ok(batch) => Json(batch).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_list_vector_store_file_batch_files(
    State(state): State<AppState>,
    Path((vector_store_id, batch_id)): Path<(String, String)>,
) -> Response {
    match state
        .vector_stores
        .list_batch_files(&vector_store_id, &batch_id)
    {
        Ok(files) => Json(files).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn handle_fallback(req: Request) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    if path == "/v1" && method == "GET" {
        return Json(serde_json::json!({"status": "ok"})).into_response();
    }
    warn!("unhandled {} {}", method, path);
    (StatusCode::NOT_FOUND, "not found").into_response()
}

async fn handle_responses(State(state): State<AppState>, body: axum::body::Bytes) -> Response {
    let mut req: ResponsesRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let mut hasher = DefaultHasher::new();
            body.hash(&mut hasher);
            error!("JSON parse error: {e}");
            error!(
                "invalid request body: bytes={} hash={:016x}",
                body.len(),
                hasher.finish()
            );
            return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response();
        }
    };
    if let Some(response) = validate_response_include(req.include.as_deref()) {
        return response;
    }
    if let Err(err) = state.prompts.apply_to_request(&mut req) {
        return err.into_response();
    }
    if let Some(response) = validate_tool_policy(&req.tools, &state.tool_policy) {
        return response;
    }
    if let Some(ref limiter) = state.rate_limiter {
        let key = if !state.client_api_key.is_empty() {
            format!(
                "rl_{}",
                &state.client_api_key[..4.min(state.client_api_key.len())]
            )
        } else {
            "rl_default".into()
        };
        if !limiter.check(&key) {
            state
                .metrics
                .rate_limit_hits_total
                .with_label_values(&[&key])
                .inc();
            return (StatusCode::TOO_MANY_REQUESTS, Json(json!({
                "error": {
                    "message": format!("rate limit exceeded: {} req per {}s", limiter.max_requests(), limiter.window_secs()),
                    "type": "rate_limit_error",
                    "code": "rate_limited"
                }
            }))).into_response();
        }
    }

    if let Err(err) = state.files.resolve_request_files(&mut req) {
        return err.into_response();
    }
    let file_search_filter = state.vector_stores.file_ids_for_tools(&req.tools);
    let file_search_query = local_file_search_query(&req);
    let file_search_vector_store_ids =
        crate::vector_stores::VectorStoreRegistry::vector_store_ids_for_tools(&req.tools);
    let file_search_max_results = local_file_search_max_results(&req.tools);
    let file_search_score_threshold = local_file_search_score_threshold(&req.tools);
    let file_search_ranker = local_file_search_ranker(&req.tools);
    let local_file_search_results = state.files.inject_file_search_context(
        &mut req,
        file_search_filter.as_ref(),
        file_search_max_results,
        file_search_score_threshold,
    );
    if !local_file_search_results.is_empty() {
        let metadata = req.metadata.get_or_insert_with(Default::default);
        metadata.insert(
            "local_file_search_vector_store_ids".to_string(),
            serde_json::to_string(&file_search_vector_store_ids).unwrap_or_default(),
        );
        if let Some(max_results) = file_search_max_results {
            metadata.insert(
                "local_file_search_max_num_results".to_string(),
                max_results.to_string(),
            );
        }
        if let Some(score_threshold) = file_search_score_threshold {
            metadata.insert(
                "local_file_search_score_threshold".to_string(),
                score_threshold.to_string(),
            );
        }
        if let Some(ranker) = file_search_ranker {
            metadata.insert("local_file_search_requested_ranker".to_string(), ranker);
        }
        metadata.insert("local_file_search_ranker".to_string(), "local_bm25".into());
        metadata.insert(
            "local_file_search_ranking_options".to_string(),
            "ranker=local_bm25; embeddings unavailable; reranking unavailable".to_string(),
        );
    }
    let local_file_search_output_items = local_file_search_call_output_item(
        &local_file_search_results,
        &file_search_query,
        &file_search_vector_store_ids,
    )
    .into_iter()
    .collect();
    let local_file_search_input_items = local_file_search_input_item(
        &local_file_search_results,
        &file_search_query,
        &file_search_vector_store_ids,
    )
    .into_iter()
    .collect();

    let original_model = req.model.clone();
    let effort = req.reasoning.as_ref().and_then(|r| r.effort.as_deref());

    info!("← {} effort={}", original_model, fmt_codex_effort(effort));

    let response = handle_responses_inner(
        state.clone(),
        req,
        local_file_search_output_items,
        local_file_search_input_items,
    )
    .await;
    let status = response.status().as_u16().to_string();
    state
        .metrics
        .http_requests_total
        .with_label_values(&["POST", &status])
        .inc();
    response
}

async fn handle_responses_inner(
    state: AppState,
    req: ResponsesRequest,
    local_output_prefix_items: Vec<Value>,
    local_input_suffix_items: Vec<Value>,
) -> Response {
    if req.previous_response_id.is_some() && req.conversation.is_some() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "previous_response_id and conversation cannot be used together",
                    "type": "invalid_request_error",
                    "param": "conversation",
                    "code": "invalid_request_error"
                }
            })),
        )
            .into_response();
    }

    let conversation_id = conversation_id_from_request(&req);
    let history = Vec::new();

    let original_model = req.model.clone();
    let mut request_input_items = response_input_items(&req);
    request_input_items.extend(local_input_suffix_items);
    let mapped_model = resolve_model(&original_model, &state.model_map);
    let store_response = req.store.unwrap_or(true);
    let mut response_extra = response_extra_fields(&req, conversation_id.as_deref());
    if !local_output_prefix_items.is_empty() {
        response_extra[LOCAL_OUTPUT_PREFIX_ITEMS_KEY] = json!(local_output_prefix_items);
    }
    if store_response {
        if let Some(id) = conversation_id.as_deref() {
            let mut items = state.sessions.get_conversation_items(id);
            items.extend(request_input_items.clone());
            state
                .sessions
                .save_conversation_items(id.to_string(), items);
        }
    }
    let effort = req.reasoning.as_ref().and_then(|r| r.effort.as_deref());
    let (reasoning_effort, thinking) = map_effort(effort);
    let msg_count = match &req.input {
        ResponsesInput::Messages(ref items) => items.len(),
        _ => 1,
    };

    if req.background == Some(true) && !store_response {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "background responses require store=true so they can be retrieved or cancelled",
                    "type": "invalid_request_error",
                    "param": "store",
                    "code": "invalid_request_error"
                }
            })),
        )
            .into_response();
    }

    if req.top_logprobs.is_some() {
        return unsupported_param(
            "top_logprobs",
            "top_logprobs is not supported by this relay",
        );
    }

    // Debug: log raw tool definitions to understand what Codex sends
    if !req.tools.is_empty() {
        debug!(
            "raw tools received: {}",
            serde_json::to_string(&req.tools).unwrap_or_default()
        );
    }

    let translated = translate::to_chat_request(
        &req,
        history.clone(),
        &state.sessions,
        &state.model_map,
        state.chinese_thinking,
    );
    let mut chat_req = translated.chat;

    // Route to VLM when the current turn has new images (not just history carrying old ones)
    let is_review_model = original_model.contains("auto-review");
    let has_new_image = match &req.input {
        ResponsesInput::Text(t) => t.contains("data:image/"),
        ResponsesInput::Messages(items) => items.last().is_some_and(|item| {
            let content = match item.get("content") {
                Some(c) => c,
                None => return item.get("image_url").is_some(),
            };
            match content {
                Value::Array(parts) => parts.iter().any(|p| {
                    let typ = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    typ == "image_url" || typ == "input_image"
                }),
                Value::String(s) => s.contains("data:image/"),
                _ => false,
            }
        }),
    };
    let route_to_vision = translated.has_images
        && state.vision_upstream.is_some()
        && !is_review_model
        && has_new_image;
    info!(
        "route_to_vision: has_images={} review={} new_image={} msgs={} route={}",
        translated.has_images,
        is_review_model,
        has_new_image,
        chat_req.messages.len(),
        route_to_vision
    );

    // Strip image_url content when NOT routing to VLM (DeepSeek rejects it)
    if !route_to_vision {
        for msg in &mut chat_req.messages {
            if let Some(ref content) = msg.content {
                if let Some(parts) = content.as_array() {
                    let text_parts: Vec<&str> = parts
                        .iter()
                        .filter_map(|p| {
                            if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                                p.get("text").and_then(|t| t.as_str())
                            } else {
                                None
                            }
                        })
                        .collect();
                    msg.content = Some(Value::String(text_parts.join("")));
                } else if let Some(s) = content.as_str() {
                    if let Some(pos) = s.find("data:image/") {
                        let stripped = s[..pos].trim().to_string();
                        msg.content = Some(Value::String(stripped));
                    }
                }
            }
        }
    }

    let (url, api_key) = if route_to_vision {
        let vu = state
            .vision_upstream
            .as_ref()
            .expect("vision_upstream must be set");
        let use_vlm = state.vision_endpoint.contains("vlm");
        let url = format!(
            "{}{}",
            join_base(vu.as_ref()),
            state.vision_endpoint.as_str()
        );
        let vmodel = state.vision_model.as_ref().clone();
        info!(
            "📷 routing to vision upstream: {} model={} endpoint={} vlm={}",
            vu.as_ref(),
            vmodel,
            state.vision_endpoint.as_str(),
            use_vlm
        );

        if use_vlm {
            let vlm_body = build_vlm_body(&chat_req);
            let api_key = state.vision_api_key.clone();
            return handle_vlm(VlmArgs {
                state,
                url,
                api_key,
                vlm_body,
                model: vmodel,
                stream_response: req.stream,
                store_response,
                request_input_items,
                response_extra,
            })
            .await;
        }

        chat_req.model = vmodel;
        (url, state.vision_api_key.clone())
    } else {
        let url = format!("{}chat/completions", join_base(&state.upstream));
        (url, state.api_key.clone())
    };

    let vision_label = if route_to_vision { " 📷" } else { "" };
    let tool_names: Vec<&str> = chat_req
        .tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
        })
        .collect();
    info!(
        "→ {} effort={} thinking={} msgs={} tools={} names=[{}]{}",
        mapped_model,
        fmt_effort(&reasoning_effort),
        fmt_thinking(&thinking),
        msg_count,
        chat_req.tools.len(),
        tool_names.join(", "),
        vision_label
    );

    if req.background == Some(true) {
        if req.stream {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": "background streaming is not supported by this relay",
                        "type": "unsupported_feature",
                        "param": "background",
                        "code": "unsupported_feature"
                    }
                })),
            )
                .into_response();
        }

        let response_id = state.sessions.new_id();
        if store_response {
            state
                .sessions
                .save_input_items(response_id.clone(), request_input_items);
        }
        chat_req.stream = false;

        let mut queued = enrich_response_object(
            json!({
                "id": response_id,
                "object": "response",
                "status": "queued",
                "model": mapped_model,
                "output": []
            }),
            &req,
        );
        if let Some(id) = conversation_id.as_deref() {
            queued["conversation"] = json!({
                "id": id,
                "object": "conversation"
            });
        }
        queued["background"] = json!(true);
        if store_response {
            state
                .sessions
                .save_response(response_id.clone(), queued.clone());
        }

        let bg_state = state.clone();
        let bg_req = req.clone();
        let bg_id = response_id.clone();
        let background_tasks = state.background_tasks.clone();
        let task_cleanup = background_tasks.clone();
        let task_id = response_id.clone();
        let cleanup_id = task_id.clone();
        let bg_conversation_id = conversation_id.clone();
        let handle = tokio::spawn(async move {
            if store_response {
                if let Some(mut in_progress) = bg_state.sessions.get_response(&bg_id) {
                    in_progress["status"] = json!("in_progress");
                    bg_state.sessions.save_response(bg_id.clone(), in_progress);
                }
            }
            let _ = handle_blocking(BlockingArgs {
                state: bg_state,
                chat_req,
                url,
                model: mapped_model,
                api_key,
                response_id: bg_id,
                store_response,
                conversation_id: bg_conversation_id,
                response_extra: response_extra.clone(),
                req: &bg_req,
            })
            .await;
            task_cleanup.remove(&cleanup_id);
        });
        background_tasks.insert(task_id, handle);

        return Json(queued).into_response();
    }

    if req.stream {
        let response_id = state.sessions.new_id();
        chat_req.stream = true;
        let request_messages = chat_req.messages.clone();
        let thinking_enabled = thinking
            .as_ref()
            .is_some_and(|t| t.get("type").and_then(serde_json::Value::as_str) != Some("disabled"));

        // Check request cache
        let cache_key = RequestCache::hash_request(&chat_req);
        if store_response && req.background != Some(true) && conversation_id.is_none() {
            if let Some(cached) = state.request_cache.get(cache_key) {
                info!("request cache: hit (key={})", cache_key);
                let cached_sse = stream::translate_cached(stream::CachedArgs {
                    response_id: response_id.clone(),
                    model: mapped_model,
                    cached,
                    sessions: state.sessions.clone(),
                    request_input_items,
                    store_response,
                    conversation_id: conversation_id.clone(),
                    response_extra: response_extra.clone(),
                });
                let mut resp = cached_sse.into_response();
                if thinking_enabled {
                    resp.headers_mut().insert(
                        header::HeaderName::from_static("x-reasoning-included"),
                        header::HeaderValue::from_static("true"),
                    );
                }
                return resp;
            }
        }

        let sse = stream::translate_stream(stream::StreamArgs {
            client: state.client,
            url,
            api_key,
            chat_req,
            response_id,
            sessions: state.sessions,
            prior_messages: history,
            request_messages,
            request_input_items,
            store_response,
            conversation_id: conversation_id.clone(),
            response_extra,
            model: mapped_model,
            model_map: state.model_map.as_ref().clone(),
            cache: if store_response && conversation_id.is_none() {
                Some(state.request_cache.clone())
            } else {
                None
            },
            cache_key: if store_response && conversation_id.is_none() {
                Some(cache_key)
            } else {
                None
            },
            token_tracker: state.token_tracker.clone(),
            metrics: state.metrics.clone(),
            executors: state.executors.clone(),
            allowed_mcp_servers: state.tool_policy.allowed_mcp_servers.clone(),
            allowed_computer_displays: state.tool_policy.allowed_computer_displays.clone(),
        });
        let mut resp = sse.into_response();
        if thinking_enabled {
            resp.headers_mut().insert(
                header::HeaderName::from_static("x-reasoning-included"),
                header::HeaderValue::from_static("true"),
            );
        }
        resp
    } else {
        let response_id = state.sessions.new_id();
        if store_response {
            state
                .sessions
                .save_input_items(response_id.clone(), request_input_items);
        }
        chat_req.stream = false;
        let start = Instant::now();
        let resp = handle_blocking(BlockingArgs {
            state: state.clone(),
            chat_req,
            url,
            model: mapped_model,
            api_key,
            response_id,
            store_response,
            conversation_id,
            response_extra,
            req: &req,
        })
        .await;
        let elapsed = start.elapsed();
        debug!("blocking request completed in {:.0}ms", elapsed.as_millis());
        resp
    }
}

async fn handle_blocking(args: BlockingArgs<'_>) -> Response {
    let BlockingArgs {
        state,
        chat_req,
        url,
        model,
        api_key,
        response_id,
        store_response,
        conversation_id,
        response_extra,
        req,
    } = args;
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json");

    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key.as_str());
    }

    match builder.json(&chat_req).send().await {
        Err(e) => {
            error!("upstream error: {e}");
            if store_response {
                save_response_unless_cancelled(
                    &state.sessions,
                    response_id.clone(),
                    response_with_extra(
                        enrich_response_object(
                            json!({
                            "id": response_id,
                            "object": "response",
                            "status": "failed",
                            "model": model,
                            "output": [],
                            "error": {"code": "connection_error", "message": e.to_string()}
                            }),
                            req,
                        ),
                        &response_extra,
                    ),
                );
            }
            (StatusCode::BAD_GATEWAY, e.to_string()).into_response()
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            error!("upstream {}: {}", status.as_u16(), body);
            if store_response {
                save_response_unless_cancelled(
                    &state.sessions,
                    response_id.clone(),
                    response_with_extra(
                        enrich_response_object(
                            json!({
                            "id": response_id,
                            "object": "response",
                            "status": "failed",
                            "model": model,
                            "output": [],
                            "error": {"code": status.as_u16().to_string(), "message": body.clone()}
                            }),
                            req,
                        ),
                        &response_extra,
                    ),
                );
            }
            (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                body,
            )
                .into_response()
        }
        Ok(r) => match r.json::<ChatResponse>().await {
            Err(e) => {
                error!("parse error: {e}");
                if store_response {
                    save_response_unless_cancelled(
                        &state.sessions,
                        response_id.clone(),
                        response_with_extra(
                            enrich_response_object(
                                json!({
                                "id": response_id,
                                "object": "response",
                                "status": "failed",
                                "model": model,
                                "output": [],
                                "error": {"code": "parse_error", "message": e.to_string()}
                                }),
                                req,
                            ),
                            &response_extra,
                        ),
                    );
                }
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
            Ok(chat_resp) => {
                // Log token usage including reasoning and cache stats
                let usage_str = format_usage(chat_resp.usage.as_ref());
                info!("↑ done {}", usage_str);

                if let Some(ref usage) = chat_resp.usage {
                    let anomalies = state.token_tracker.record(usage, &model, &response_id);
                    for atype in &anomalies {
                        state
                            .metrics
                            .token_anomalies_total
                            .with_label_values(&[atype])
                            .inc();
                    }
                }

                let assistant_msg = chat_resp
                    .choices
                    .first()
                    .map(|c| c.message.clone())
                    .unwrap_or_else(|| ChatMessage {
                        role: "assistant".into(),
                        content: Some(serde_json::Value::String(String::new())),
                        reasoning_content: None,
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });

                let mut full_history = chat_req.messages.clone();
                if let Some(reasoning) = assistant_msg.reasoning_content.clone() {
                    if !reasoning.is_empty() {
                        state.sessions.store_turn_reasoning(
                            &chat_req.messages,
                            &assistant_msg,
                            reasoning.clone(),
                        );
                        if let Some(tool_calls) = &assistant_msg.tool_calls {
                            for tc in tool_calls {
                                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                    state
                                        .sessions
                                        .store_reasoning(id.to_string(), reasoning.clone());
                                }
                            }
                        }
                    }
                }
                full_history.push(assistant_msg);
                if store_response {
                    state
                        .sessions
                        .save_with_id(response_id.clone(), full_history.clone());
                }
                if let Some(id) = conversation_id.as_deref() {
                    state
                        .sessions
                        .save_conversation(id.to_string(), full_history);
                }

                let (resp, _) = translate::from_chat_response(response_id, &model, chat_resp);
                if let Ok(value) = serde_json::to_value(&resp) {
                    let mut value = enrich_response_object(value, req);
                    append_local_mcp_outputs(&state, &mut value).await;
                    append_local_computer_outputs(&state, &mut value).await;
                    value["status"] = json!("completed");
                    let value = response_with_extra(value, &response_extra);
                    if store_response {
                        save_response_unless_cancelled(
                            &state.sessions,
                            resp.id.clone(),
                            value.clone(),
                        );
                    }
                    if let Some(id) = conversation_id.as_deref() {
                        let mut items = state.sessions.get_conversation_items(id);
                        if let Some(output) = value.get("output").and_then(Value::as_array) {
                            items.extend(output.iter().cloned());
                            state
                                .sessions
                                .save_conversation_items(id.to_string(), items);
                        }
                    }
                    return Json(value).into_response();
                }
                Json(resp).into_response()
            }
        },
    }
}

fn save_response_unless_cancelled(sessions: &SessionStore, id: String, response: Value) {
    if sessions.response_status(&id).as_deref() == Some("cancelled") {
        return;
    }
    sessions.save_response(id, response);
}

async fn append_local_computer_outputs(state: &AppState, response: &mut Value) {
    if !state.executors.computer.enabled() {
        return;
    }

    let calls = response
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let invocation = ComputerActionInvocation::from_response_item(item)?;
                    Some((invocation.call_id.clone(), invocation))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if calls.is_empty() {
        return;
    }

    let mut outputs = Vec::new();
    for (call_id, invocation) in calls {
        let result = if !state.tool_policy.allowed_computer_displays.is_empty()
            && !state
                .tool_policy
                .allowed_computer_displays
                .iter()
                .any(|display| display == &invocation.display)
        {
            crate::executor::ComputerActionOutput::failed(format!(
                "computer display '{}' is not allowed by local tool policy",
                invocation.display
            ))
        } else {
            state.executors.computer.execute_action(invocation).await
        };
        outputs.push(local_computer_call_output_item(&call_id, result));
    }

    if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
        items.extend(outputs);
    }
}

fn local_computer_call_output_item(
    call_id: &str,
    result: crate::executor::ComputerActionOutput,
) -> Value {
    json!({
        "type": "computer_call_output",
        "id": format!("ccout_{call_id}"),
        "call_id": call_id,
        "status": result.status,
        "output": result.output
    })
}

async fn append_local_mcp_outputs(state: &AppState, response: &mut Value) {
    if !state.executors.mcp.enabled() {
        return;
    }

    let calls = response
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let call_id = item.get("call_id").and_then(Value::as_str)?.to_string();
                    let invocation = McpToolInvocation::from_response_item(item)?;
                    Some((call_id, invocation))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if calls.is_empty() {
        return;
    }

    let mut outputs = Vec::new();
    for (call_id, invocation) in calls {
        let result = if !state.tool_policy.allowed_mcp_servers.is_empty()
            && !state
                .tool_policy
                .allowed_mcp_servers
                .iter()
                .any(|server| server == &invocation.server_label)
        {
            crate::executor::McpToolOutput::failed(format!(
                "MCP server '{}' is not allowed by local tool policy",
                invocation.server_label
            ))
        } else {
            state.executors.mcp.execute_tool(invocation).await
        };
        outputs.push(local_mcp_tool_output_item(&call_id, result));
    }

    if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
        items.extend(outputs);
    }
}

fn local_mcp_tool_output_item(call_id: &str, result: crate::executor::McpToolOutput) -> Value {
    json!({
        "type": "mcp_tool_call_output",
        "id": format!("mcpout_{call_id}"),
        "call_id": call_id,
        "status": result.status,
        "output": result.output
    })
}

fn conversation_id_from_request(req: &ResponsesRequest) -> Option<String> {
    let conversation = req.conversation.as_ref()?;
    match conversation {
        Value::String(id) if !id.is_empty() => Some(id.clone()),
        Value::Object(obj) => obj
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| Some(format!("conv_{}", uuid::Uuid::new_v4().simple()))),
        _ => None,
    }
}

fn response_extra_fields(req: &ResponsesRequest, conversation_id: Option<&str>) -> Value {
    let mut extra = enrich_response_object(json!({}), req);
    if let Some(id) = conversation_id {
        extra["conversation"] = json!({
            "id": id,
            "object": "conversation"
        });
    }
    extra
}

fn response_with_extra(mut response: Value, extra: &Value) -> Value {
    merge_response_extra(&mut response, extra);
    response
}

fn validate_response_include(include: Option<&[String]>) -> Option<Response> {
    let include = include?;
    for field in include {
        if !is_supported_response_include(field) {
            return Some(unsupported_param(
                "include",
                &format!("include field '{field}' is not supported by this relay"),
            ));
        }
    }
    None
}

fn validate_tool_policy(tools: &[Value], policy: &ToolPolicy) -> Option<Response> {
    for tool in tools {
        let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(tool_type, "mcp" | "remote_mcp") && !policy.allowed_mcp_servers.is_empty() {
            let server = tool
                .get("server_label")
                .or_else(|| tool.get("server_url"))
                .or_else(|| tool.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if !policy
                .allowed_mcp_servers
                .iter()
                .any(|allowed| allowed == server)
            {
                return Some(unsupported_param(
                    "tools",
                    &format!("MCP server '{server}' is not allowed by local tool policy"),
                ));
            }
        }
        if matches!(tool_type, "computer_use" | "computer_use_preview")
            && !policy.allowed_computer_displays.is_empty()
        {
            let display = tool
                .get("display")
                .or_else(|| tool.get("environment"))
                .and_then(Value::as_str)
                .unwrap_or("default");
            if !policy
                .allowed_computer_displays
                .iter()
                .any(|allowed| allowed == display)
            {
                return Some(unsupported_param(
                    "tools",
                    &format!("computer display '{display}' is not allowed by local tool policy"),
                ));
            }
        }
    }
    None
}

fn is_supported_response_include(field: &str) -> bool {
    matches!(
        field,
        "file_search_call.results" | "output[*].file_search_call.results" | "usage" | "input_items"
    )
}

fn local_file_search_call_output_item(
    results: &[Value],
    query: &str,
    vector_store_ids: &[String],
) -> Option<Value> {
    if results.is_empty() {
        return None;
    }
    let normalized_results: Vec<Value> = results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            json!({
                "file_id": result.get("file_id").cloned().unwrap_or(Value::Null),
                "filename": result.get("filename").cloned().unwrap_or(Value::Null),
                "chunk_id": result.get("chunk_id").cloned().unwrap_or(Value::Null),
                "start_char": result.get("start_char").cloned().unwrap_or(Value::Null),
                "end_char": result.get("end_char").cloned().unwrap_or(Value::Null),
                "score": result.get("score").cloned().unwrap_or_else(|| json!(0)),
                "text": result.get("snippet").cloned().unwrap_or_else(|| json!("")),
                "index": idx
            })
        })
        .collect();
    Some(json!({
        "type": "file_search_call",
        "id": stable_file_search_item_id("fs", query, vector_store_ids, results),
        "status": "completed",
        "queries": [{"query": query}],
        "vector_store_ids": vector_store_ids,
        "results": normalized_results
    }))
}

fn local_file_search_input_item(
    results: &[Value],
    query: &str,
    vector_store_ids: &[String],
) -> Option<Value> {
    if results.is_empty() {
        return None;
    }
    Some(json!({
        "id": stable_file_search_item_id("item_fs", query, vector_store_ids, results),
        "type": "file_search_context",
        "query": query,
        "vector_store_ids": vector_store_ids,
        "results": results
    }))
}

fn stable_file_search_item_id(
    prefix: &str,
    query: &str,
    vector_store_ids: &[String],
    results: &[Value],
) -> String {
    let mut hasher = DefaultHasher::new();
    query.hash(&mut hasher);
    vector_store_ids.hash(&mut hasher);
    for result in results {
        result.to_string().hash(&mut hasher);
    }
    format!("{prefix}_{:016x}", hasher.finish())
}

fn local_file_search_query(req: &ResponsesRequest) -> String {
    let mut chunks = Vec::new();
    if let Some(instructions) = req.instructions.as_deref().or(req.system.as_deref()) {
        chunks.push(instructions.to_string());
    }
    collect_request_text(&req.input, &mut chunks);
    chunks.join("\n")
}

fn collect_request_text(input: &ResponsesInput, chunks: &mut Vec<String>) {
    match input {
        ResponsesInput::Text(text) => chunks.push(text.clone()),
        ResponsesInput::Messages(items) => {
            for item in items {
                collect_value_text(item, chunks);
            }
        }
    }
}

fn collect_value_text(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::String(text) => chunks.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_value_text(item, chunks);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                chunks.push(text.to_string());
            }
            if let Some(content) = map.get("content") {
                collect_value_text(content, chunks);
            }
        }
        _ => {}
    }
}

fn local_file_search_max_results(tools: &[Value]) -> Option<usize> {
    tools
        .iter()
        .filter(|tool| {
            matches!(
                tool.get("type").and_then(Value::as_str),
                Some("file_search" | "file_search_preview")
            )
        })
        .filter_map(|tool| {
            tool.get("max_num_results")
                .and_then(Value::as_u64)
                .or_else(|| {
                    tool.get("ranking_options")
                        .and_then(|opts| opts.get("max_num_results"))
                        .and_then(Value::as_u64)
                })
        })
        .min()
        .map(|value| value as usize)
}

fn local_file_search_score_threshold(tools: &[Value]) -> Option<f64> {
    tools
        .iter()
        .filter(|tool| {
            matches!(
                tool.get("type").and_then(Value::as_str),
                Some("file_search" | "file_search_preview")
            )
        })
        .filter_map(|tool| {
            tool.get("ranking_options")
                .and_then(|opts| opts.get("score_threshold"))
                .and_then(Value::as_f64)
        })
        .max_by(|a, b| a.total_cmp(b))
}

fn local_file_search_ranker(tools: &[Value]) -> Option<String> {
    tools
        .iter()
        .filter(|tool| {
            matches!(
                tool.get("type").and_then(Value::as_str),
                Some("file_search" | "file_search_preview")
            )
        })
        .filter_map(|tool| {
            tool.get("ranking_options")
                .and_then(|opts| opts.get("ranker"))
                .and_then(Value::as_str)
                .or_else(|| tool.get("ranker").and_then(Value::as_str))
        })
        .find(|ranker| !ranker.is_empty())
        .map(str::to_string)
}

fn enrich_response_object(mut value: Value, req: &ResponsesRequest) -> Value {
    value["created_at"] = json!(now_unix_secs());
    value["background"] = json!(req.background.unwrap_or(false));
    value["parallel_tool_calls"] = json!(req.parallel_tool_calls.unwrap_or(true));
    value["store"] = json!(req.store.unwrap_or(true));
    value["truncation"] = json!(req.truncation.as_deref().unwrap_or("disabled"));
    value["tools"] = json!(req.tools);
    value["tool_choice"] = req.tool_choice.clone().unwrap_or_else(|| json!("auto"));
    value["temperature"] = req.temperature.map_or(Value::Null, Value::from);
    value["top_p"] = req.top_p.map_or(Value::Null, Value::from);
    value["max_output_tokens"] = req.max_output_tokens.map_or(Value::Null, Value::from);
    value["max_tool_calls"] = req.max_tool_calls.map_or(Value::Null, Value::from);
    if let Some(max) = req.max_tool_calls {
        limit_function_call_outputs(&mut value, max as usize);
    }
    value["metadata"] = req
        .metadata
        .clone()
        .and_then(|m| serde_json::to_value(m).ok())
        .unwrap_or_else(|| json!({}));
    value["instructions"] = req
        .instructions
        .clone()
        .or_else(|| req.system.clone())
        .map_or(Value::Null, Value::from);
    value["reasoning"] = req
        .reasoning
        .as_ref()
        .map(|r| {
            json!({
                "effort": r.effort,
                "summary": r.summary
            })
        })
        .unwrap_or(Value::Null);
    value["text"] = req
        .text
        .clone()
        .unwrap_or_else(|| json!({"format": {"type": "text"}}));
    if let Some(v) = &req.prompt_cache_key {
        value["prompt_cache_key"] = json!(v);
    }
    if let Some(v) = &req.prompt_cache_retention {
        value["prompt_cache_retention"] = json!(v);
    }
    if let Some(v) = &req.safety_identifier {
        value["safety_identifier"] = json!(v);
    }
    if let Some(v) = &req.service_tier {
        value["service_tier"] = json!(v);
    }
    value
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn response_input_items(req: &ResponsesRequest) -> Vec<Value> {
    match &req.input {
        ResponsesInput::Text(text) => vec![json!({
            "id": format!("item_{}", uuid::Uuid::new_v4().simple()),
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}]
        })],
        ResponsesInput::Messages(items) => items
            .iter()
            .enumerate()
            .map(|(idx, item)| {
                let mut item = item.clone();
                if item.get("id").is_none() {
                    item["id"] = json!(format!("item_{idx}"));
                }
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
                if is_tool_output_input_item(item_type) {
                    if item.get("status").is_none() {
                        item["status"] = json!("completed");
                    }
                    if item.get("call_id").is_some() && item.get("output").is_none() {
                        item["output"] = Value::Null;
                    }
                }
                item
            })
            .collect(),
    }
}

fn is_tool_output_input_item(item_type: &str) -> bool {
    matches!(
        item_type,
        "function_call_output"
            | "mcp_tool_call_output"
            | "custom_tool_call_output"
            | "tool_search_output"
            | "computer_call_output"
    )
}

fn count_input_tokens(req: &ResponsesRequest) -> u32 {
    count_tokens(&req.model, &input_token_text(req))
}

fn count_value_tokens(value: &Value) -> u32 {
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("gpt-4o");
    let text = serde_json::to_string(value).unwrap_or_default();
    count_tokens(model, &text)
}

fn count_tokens(model: &str, text: &str) -> u32 {
    if let Ok(bpe) = tiktoken_rs::bpe_for_model(model) {
        return bpe.encode_with_special_tokens(text).len().max(1) as u32;
    }
    match tiktoken_rs::cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len().max(1) as u32,
        Err(_) => text.chars().count().div_ceil(4).max(1) as u32,
    }
}

fn input_token_text(req: &ResponsesRequest) -> String {
    let mut text = String::new();
    if let Some(instructions) = req.instructions.as_deref().or(req.system.as_deref()) {
        text.push_str(instructions);
        text.push('\n');
    }
    match &req.input {
        ResponsesInput::Text(input) => text.push_str(input),
        ResponsesInput::Messages(items) => {
            text.push_str(&serde_json::to_string(items).unwrap_or_default());
        }
    }
    text
}

/// Extract prompt text and image data URL for MiniMax VLM endpoint.
fn build_vlm_body(chat_req: &ChatRequest) -> Value {
    let mut prompt = String::new();
    let mut image_url = String::new();

    for msg in chat_req.messages.iter().rev() {
        if msg.role != "user" {
            continue;
        }
        if let Some(ref content) = msg.content {
            match content {
                Value::String(s) => {
                    if let Some(pos) = s.find("data:image/") {
                        if image_url.is_empty() {
                            image_url = s[pos..].trim().to_string();
                        }
                        let text = s[..pos].trim();
                        if !text.is_empty() && prompt.is_empty() {
                            prompt = text.to_string();
                        }
                    } else if prompt.is_empty() && !s.starts_with("data:") {
                        prompt = s.clone();
                    }
                }
                Value::Array(parts) => {
                    for p in parts {
                        match p.get("type").and_then(|t| t.as_str()) {
                            Some("text") if prompt.is_empty() => {
                                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                                    prompt = t.to_string();
                                }
                            }
                            Some("image_url") if image_url.is_empty() => {
                                image_url = p
                                    .get("image_url")
                                    .and_then(|u| u.get("url"))
                                    .and_then(|u| u.as_str())
                                    .unwrap_or("")
                                    .to_string();
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if prompt.is_empty() {
        prompt = "Describe the image.".into();
    }

    json!({ "prompt": prompt, "image_url": image_url })
}

/// Handle MiniMax VLM request: send prompt + image_url, return SSE stream.
async fn handle_vlm(args: VlmArgs) -> Response {
    let VlmArgs {
        state,
        url,
        api_key,
        vlm_body,
        model,
        stream_response,
        store_response,
        request_input_items,
        response_extra,
    } = args;
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json");
    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key.as_str());
    }

    let vlm_result = match builder.json(&vlm_body).send().await {
        Err(e) => {
            error!("vlm upstream error: {e}");
            return (StatusCode::BAD_GATEWAY, e.to_string()).into_response();
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            error!("vlm upstream {}: {}", status.as_u16(), body);
            return (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                body,
            )
                .into_response();
        }
        Ok(r) => match r.json::<Value>().await {
            Err(e) => {
                error!("vlm parse error: {e}");
                return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
            }
            Ok(resp) => resp,
        },
    };

    let text = vlm_result
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    info!("↑ vlm done text_len={}", text.len());

    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let msg_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    if store_response {
        state
            .sessions
            .save_input_items(response_id.clone(), request_input_items);
    }

    let mut response_obj = json!({
        "id": &response_id,
        "object": "response",
        "created_at": now_unix_secs(),
        "status": "completed",
        "background": false,
        "error": null,
        "incomplete_details": null,
        "model": &model,
        "output": [{
            "type": "message",
            "id": &msg_id,
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": &text, "annotations": [], "logprobs": []}]
        }],
        "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}
    });
    merge_response_extra(&mut response_obj, &response_extra);
    if store_response {
        state
            .sessions
            .save_response(response_id.clone(), response_obj.clone());
    }

    if !stream_response {
        return Json(response_obj).into_response();
    }

    use crate::sse::SseState;
    use axum::response::sse::{Event, KeepAlive, Sse};
    let mut vss = SseState::new();
    let vlm_oi = vss.alloc_output_index();

    let events: Vec<Result<Event, std::convert::Infallible>> = vec![
        vss.response_created(&response_id, &model),
        vss.response_in_progress(&response_id),
        vss.output_item_added(
            vlm_oi, &msg_id, "message",
            json!({"role": "assistant", "content": []})
        ),
        vss.content_part_added(
            &msg_id, vlm_oi, 0,
            json!({"type": "output_text", "text": "", "annotations": [], "logprobs": []})
        ),
        vss.output_text_delta(&msg_id, vlm_oi, 0, &text),
        vss.output_text_done(&msg_id, vlm_oi, 0, &text),
        vss.content_part_done(&msg_id, vlm_oi, 0, json!({
            "type": "output_text",
            "text": &text,
            "annotations": [],
            "logprobs": []
        })),
        vss.output_item_done(vlm_oi, json!({
            "type": "message",
            "id": &msg_id,
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": &text, "annotations": [], "logprobs": []}]
        })),
        vss.response_completed(&response_obj),
    ];

    Sse::new(futures_util::stream::iter(events))
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_upstream_https() {
        let url = validate_upstream("https://openrouter.ai/api/v1").unwrap();
        assert_eq!(url.scheme(), "https");
    }

    #[test]
    fn test_validate_upstream_rejects_ftp() {
        assert!(validate_upstream("ftp://evil.com").is_err());
    }

    #[test]
    fn test_join_base_adds_slash() {
        let url = Url::parse("https://api.example.com/v1").unwrap();
        assert_eq!(join_base(&url), "https://api.example.com/v1/");
    }

    #[test]
    fn test_join_base_preserves_slash() {
        let url = Url::parse("https://api.example.com/v1/").unwrap();
        assert_eq!(join_base(&url), "https://api.example.com/v1/");
    }

    #[test]
    fn test_response_input_items_defaults_ids() {
        let req = ResponsesRequest {
            model: "gpt-5".into(),
            input: ResponsesInput::Messages(vec![json!({
                "type": "message",
                "role": "user",
                "content": "hi"
            })]),
            previous_response_id: None,
            tools: vec![],
            stream: false,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            system: None,
            instructions: None,
            reasoning: None,
            tool_choice: None,
            store: None,
            metadata: None,
            truncation: None,
            background: None,
            conversation: None,
            include: None,
            include_obfuscation: None,
            max_tool_calls: None,
            parallel_tool_calls: None,
            prompt: None,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            safety_identifier: None,
            service_tier: None,
            stream_options: None,
            text: None,
            top_logprobs: None,
            user: None,
        };

        let items = response_input_items(&req);
        assert_eq!(items[0].get("id").and_then(Value::as_str), Some("item_0"));
    }

    #[test]
    fn test_response_input_items_marks_tool_outputs_completed() {
        let req = ResponsesRequest {
            model: "gpt-5".into(),
            input: ResponsesInput::Messages(vec![json!({
                "type": "computer_call_output",
                "call_id": "call_screen",
                "screenshot": "data:image/png;base64,abc"
            })]),
            previous_response_id: None,
            tools: vec![],
            stream: false,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            system: None,
            instructions: None,
            reasoning: None,
            tool_choice: None,
            store: None,
            metadata: None,
            truncation: None,
            background: None,
            conversation: None,
            include: None,
            include_obfuscation: None,
            max_tool_calls: None,
            parallel_tool_calls: None,
            prompt: None,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            safety_identifier: None,
            service_tier: None,
            stream_options: None,
            text: None,
            top_logprobs: None,
            user: None,
        };

        let items = response_input_items(&req);

        assert_eq!(items[0]["id"], "item_0");
        assert_eq!(items[0]["status"], "completed");
        assert_eq!(items[0]["output"], Value::Null);
    }

    #[test]
    fn test_unsupported_include_returns_unsupported_feature() {
        let response =
            validate_response_include(Some(&["code_interpreter_call.outputs".to_string()]))
                .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_supported_include_accepts_local_fields() {
        assert!(validate_response_include(Some(&[
            "file_search_call.results".to_string(),
            "output[*].file_search_call.results".to_string(),
            "usage".to_string(),
            "input_items".to_string(),
        ]))
        .is_none());
    }

    #[test]
    fn test_file_search_call_output_item_contains_local_results() {
        let item = local_file_search_call_output_item(
            &[json!({
                "file_id": "file_1",
                "filename": "notes.md",
                "score": 2,
                "snippet": "relay notes"
            })],
            "relay",
            &["vs_1".to_string()],
        )
        .unwrap();

        assert_eq!(item["type"].as_str(), Some("file_search_call"));
        assert_eq!(item["status"].as_str(), Some("completed"));
        assert_eq!(item["queries"][0]["query"].as_str(), Some("relay"));
        assert_eq!(item["vector_store_ids"][0].as_str(), Some("vs_1"));
        assert_eq!(item["results"][0]["file_id"].as_str(), Some("file_1"));
        assert_eq!(item["results"][0]["text"].as_str(), Some("relay notes"));
    }

    #[test]
    fn test_file_search_call_output_item_uses_stable_id() {
        let results = vec![json!({
            "file_id": "file_1",
            "filename": "notes.md",
            "chunk_id": "file_1:0",
            "score": 2,
            "snippet": "relay notes"
        })];

        let first =
            local_file_search_call_output_item(&results, "relay", &["vs_1".to_string()]).unwrap();
        let second =
            local_file_search_call_output_item(&results, "relay", &["vs_1".to_string()]).unwrap();

        assert_eq!(first["id"], second["id"]);
        assert_eq!(first["results"][0]["chunk_id"], "file_1:0");
    }

    #[test]
    fn test_local_mcp_tool_output_item_preserves_status_and_output() {
        let item = local_mcp_tool_output_item(
            "call_mcp",
            crate::executor::McpToolOutput {
                status: "completed".into(),
                output: json!({"content": [{"type": "text", "text": "ok"}]}),
            },
        );

        assert_eq!(item["type"], "mcp_tool_call_output");
        assert_eq!(item["id"], "mcpout_call_mcp");
        assert_eq!(item["call_id"], "call_mcp");
        assert_eq!(item["status"], "completed");
        assert_eq!(item["output"]["content"][0]["text"], "ok");
    }

    #[test]
    fn test_local_file_search_max_results_prefers_smallest_tool_limit() {
        assert_eq!(
            local_file_search_max_results(&[
                json!({"type": "file_search", "max_num_results": 4}),
                json!({"type": "file_search", "ranking_options": {"max_num_results": 2}})
            ]),
            Some(2)
        );
    }

    #[test]
    fn test_local_file_search_score_threshold_prefers_strictest_limit() {
        assert_eq!(
            local_file_search_score_threshold(&[
                json!({"type": "file_search", "ranking_options": {"score_threshold": 1.5}}),
                json!({"type": "file_search", "ranking_options": {"score_threshold": 3.0}})
            ]),
            Some(3.0)
        );
    }

    #[test]
    fn test_tool_policy_rejects_unlisted_mcp_server() {
        let policy = ToolPolicy {
            allowed_mcp_servers: vec!["safe".into()],
            allowed_computer_displays: vec![],
        };
        let response =
            validate_tool_policy(&[json!({"type": "mcp", "server_label": "unsafe"})], &policy)
                .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_tool_policy_allows_listed_computer_display() {
        let policy = ToolPolicy {
            allowed_mcp_servers: vec![],
            allowed_computer_displays: vec!["browser".into()],
        };

        assert!(validate_tool_policy(
            &[json!({"type": "computer_use", "display": "browser"})],
            &policy,
        )
        .is_none());
    }

    #[test]
    fn test_authorization_matches_bearer_token() {
        assert!(authorization_matches("Bearer abc", "abc"));
        assert!(authorization_matches("bearer abc", "abc"));
        assert!(authorization_matches("abc", "abc"));
        assert!(!authorization_matches("Bearer abc", "def"));
    }

    #[test]
    fn test_count_value_tokens_accepts_partial_body() {
        let tokens = count_value_tokens(&json!({"input": "hello"}));
        assert!(tokens > 0);
    }
}
