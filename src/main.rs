mod cache;
mod session;
mod stream;
mod translate;
mod types;

use crate::cache::RequestCache;
use anyhow::{bail, Result};
use axum::{
    extract::{Path, Query, Request, State},
    http::header,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use session::SessionStore;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};
use types::*;

#[derive(Parser, Debug)]
#[command(name = "deecodex", about = "Responses API <-> Chat Completions bridge")]
struct Args {
    #[arg(long, env = "CODEX_RELAY_PORT", default_value = "4444")]
    port: u16,

    #[arg(
        long,
        env = "CODEX_RELAY_UPSTREAM",
        default_value = "https://openrouter.ai/api/v1"
    )]
    upstream: String,

    #[arg(long, env = "CODEX_RELAY_API_KEY", default_value = "")]
    api_key: String,

    #[arg(long, env = "CODEX_RELAY_MODEL_MAP", default_value = "{}")]
    model_map: String,

    #[arg(long, env = "CODEX_RELAY_MAX_BODY_MB", default_value = "100")]
    max_body_mb: usize,

    /// Vision/multimodal API upstream (e.g. MiniMax for image support)
    #[arg(long, env = "CODEX_RELAY_VISION_UPSTREAM", default_value = "")]
    vision_upstream: String,

    /// Vision/multimodal API key
    #[arg(long, env = "CODEX_RELAY_VISION_API_KEY", default_value = "")]
    vision_api_key: String,

    /// Model name used when routing to vision upstream (e.g. MiniMax-M1)
    #[arg(long, env = "CODEX_RELAY_VISION_MODEL", default_value = "MiniMax-M1")]
    vision_model: String,

    /// Vision upstream endpoint path (e.g. "v1/coding_plan/vlm" for MiniMax)
    #[arg(
        long,
        env = "CODEX_RELAY_VISION_ENDPOINT",
        default_value = "v1/coding_plan/vlm"
    )]
    vision_endpoint: String,

    /// Append "请用中文思考。" to system prompt for Chinese reasoning
    #[arg(long, env = "CODEX_RELAY_CHINESE_THINKING", default_value = "false")]
    chinese_thinking: bool,
}

#[derive(Clone)]
struct AppState {
    sessions: SessionStore,
    client: Client,
    upstream: Arc<Url>,
    api_key: Arc<String>,
    model_map: Arc<ModelMap>,
    /// Track which non-function tool names have already been warned about.
    /// Optional vision/multimodal upstream (e.g. MiniMax for images)
    vision_upstream: Option<Arc<Url>>,
    /// Optional vision API key
    vision_api_key: Arc<String>,
    /// Model name for vision upstream (e.g. MiniMax-M1)
    vision_model: Arc<String>,
    /// Vision upstream endpoint suffix (e.g. "v1/coding_plan/vlm")
    vision_endpoint: Arc<String>,
    /// Server start time for uptime calculation
    start_time: std::time::Instant,
    /// Request cache for identical payloads
    request_cache: RequestCache,
    /// Background response tasks that can be cancelled while queued/in progress.
    background_tasks: Arc<dashmap::DashMap<String, tokio::task::JoinHandle<()>>>,
    /// Inject Chinese thinking instruction into system prompt
    chinese_thinking: bool,
}

struct BlockingArgs<'a> {
    state: AppState,
    chat_req: types::ChatRequest,
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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deecodex=info".into()),
        )
        .init();

    let args = Args::parse();

    let model_map: ModelMap = match serde_json::from_str(&args.model_map) {
        Ok(m) => m,
        Err(e) => {
            error!("Failed to parse CODEX_RELAY_MODEL_MAP: {e}");
            HashMap::new()
        }
    };

    info!("model map: {} entries", model_map.len());

    let upstream = validate_upstream(&args.upstream)?;

    let vision_upstream = if args.vision_upstream.is_empty() {
        None
    } else {
        Some(Arc::new(validate_upstream(&args.vision_upstream)?))
    };
    if vision_upstream.is_some() {
        info!("vision upstream configured: {}", args.vision_upstream);
    }

    let state = AppState {
        sessions: SessionStore::new(),
        client: Client::builder()
            .pool_idle_timeout(None)
            .pool_max_idle_per_host(4)
            .build()?,
        upstream: Arc::new(upstream),
        api_key: Arc::new(args.api_key),
        model_map: Arc::new(model_map),
        vision_upstream,
        vision_api_key: Arc::new(args.vision_api_key),
        vision_model: Arc::new(args.vision_model),
        vision_endpoint: Arc::new(args.vision_endpoint),
        start_time: std::time::Instant::now(),
        request_cache: RequestCache::default(),
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: args.chinese_thinking,
    };
    if args.chinese_thinking {
        info!("chinese thinking mode: enabled (system prompt will include Chinese instruction)");
    }

    let max_bytes = args.max_body_mb * 1024 * 1024;
    let body_limit = axum::extract::DefaultBodyLimit::max(max_bytes);

    let app = Router::new()
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
        .fallback(handle_fallback)
        .layer(body_limit)
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{}", args.port);
    info!(
        "listening {} -> {} | body:{}MB",
        addr,
        state.upstream.as_ref(),
        args.max_body_mb
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn validate_upstream(raw: &str) -> Result<Url> {
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

#[derive(Debug, Deserialize)]
struct RetrieveResponseQuery {
    #[serde(default)]
    stream: bool,
    #[serde(default)]
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
    #[serde(default)]
    include: Option<Vec<String>>,
}

async fn handle_get_response(
    State(state): State<AppState>,
    Path(response_id): Path<String>,
    Query(query): Query<RetrieveResponseQuery>,
) -> Response {
    let _ = (
        &query.include,
        query.include_obfuscation,
        query.starting_after,
    );
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
                                    );
                                }
                            }
                        }
                    }
                }
                "reasoning" => {
                    if let Some(content) = item.get("content").and_then(Value::as_array) {
                        for (content_idx, part) in content.iter().enumerate() {
                            if part.get("type").and_then(Value::as_str) == Some("reasoning_text") {
                                let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                                if !text.is_empty() {
                                    push_replay_event(
                                        &mut events,
                                        "response.reasoning_text.delta",
                                        json!({
                                            "type": "response.reasoning_text.delta",
                                            "item_id": item.get("id").and_then(Value::as_str).unwrap_or(""),
                                            "output_index": idx,
                                            "content_index": content_idx,
                                            "delta": text
                                        }),
                                        include_obfuscation,
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
    );

    let events = events
        .into_iter()
        .enumerate()
        .filter_map(move |(idx, event)| {
            if starting_after.is_some_and(|after| (idx as u64) <= after) {
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
) {
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
    let approx = match serde_json::from_slice::<ResponsesRequest>(&body) {
        Ok(req) => approximate_input_tokens(&req),
        Err(_) => match serde_json::from_slice::<Value>(&body) {
            Ok(value) => approximate_value_tokens(&value),
            Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response(),
        },
    };
    Json(json!({
        "object": "response.input_tokens",
        "input_tokens": approx
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
    let req: ResponsesRequest = match serde_json::from_slice(&body) {
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

    let original_model = req.model.clone();
    let effort = req.reasoning.as_ref().and_then(|r| r.effort.as_deref());

    info!("← {} effort={}", original_model, fmt_codex_effort(effort));

    handle_responses_inner(state, req).await
}

async fn handle_responses_inner(state: AppState, req: ResponsesRequest) -> Response {
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
    let history = if let Some(id) = req.previous_response_id.as_deref() {
        state.sessions.get_history(id)
    } else if let Some(id) = conversation_id.as_deref() {
        state.sessions.get_conversation(id)
    } else {
        Vec::new()
    };

    let original_model = req.model.clone();
    let request_input_items = response_input_items(&req);
    let mapped_model = resolve_model(&original_model, &state.model_map);
    let store_response = req.store.unwrap_or(true);
    let response_extra = response_extra_fields(&req, conversation_id.as_deref());
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

    // Route to VLM only for first-turn image requests (msgs <= 3), not review models
    let is_review_model = original_model.contains("auto-review");
    let is_first_turn = chat_req.messages.len() <= 5;
    let route_to_vision = translated.has_images
        && state.vision_upstream.is_some()
        && !is_review_model
        && is_first_turn;
    info!(
        "route_to_vision: has_images={} review={} first_turn={} msgs={} route={}",
        translated.has_images,
        is_review_model,
        is_first_turn,
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
    info!(
        "→ {} effort={} thinking={} msgs={} tools={}{}",
        mapped_model,
        fmt_effort(&reasoning_effort),
        fmt_thinking(&thinking),
        msg_count,
        chat_req.tools.len(),
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
                    let mut value =
                        response_with_extra(enrich_response_object(value, req), &response_extra);
                    value["status"] = json!("completed");
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

fn merge_response_extra(response: &mut Value, extra: &Value) {
    let Some(extra_obj) = extra.as_object() else {
        return;
    };
    for (key, value) in extra_obj {
        if response.get(key).is_none() || response.get(key) == Some(&Value::Null) {
            response[key] = value.clone();
        }
    }
}

fn response_with_extra(mut response: Value, extra: &Value) -> Value {
    merge_response_extra(&mut response, extra);
    response
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

fn limit_function_call_outputs(response: &mut Value, max_tool_calls: usize) {
    let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };
    let mut seen = 0usize;
    output.retain(|item| {
        if item.get("type").and_then(Value::as_str) == Some("function_call") {
            seen += 1;
            seen <= max_tool_calls
        } else {
            true
        }
    });
    if seen > max_tool_calls {
        response["status"] = json!("incomplete");
        response["incomplete_details"] = json!({
            "reason": "max_tool_calls"
        });
    }
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
                item
            })
            .collect(),
    }
}

fn approximate_input_tokens(req: &ResponsesRequest) -> u32 {
    let mut chars = req.model.len()
        + req.instructions.as_deref().unwrap_or("").len()
        + req.system.as_deref().unwrap_or("").len();
    match &req.input {
        ResponsesInput::Text(text) => chars += text.len(),
        ResponsesInput::Messages(items) => {
            chars += serde_json::to_string(items).unwrap_or_default().len();
        }
    }
    chars.div_ceil(4).max(1) as u32
}

fn approximate_value_tokens(value: &Value) -> u32 {
    let chars = serde_json::to_string(value).unwrap_or_default().len();
    chars.div_ceil(4).max(1) as u32
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
        "status": "completed",
        "model": &model,
        "output": [{
            "type": "message",
            "id": &msg_id,
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": &text}]
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

    use axum::response::sse::{Event, KeepAlive, Sse};
    let events: Vec<Result<Event, std::convert::Infallible>> = vec![
        Ok(Event::default().event("response.created").data(json!({
            "type": "response.created",
            "response": { "id": &response_id, "status": "in_progress", "model": &model }
        }).to_string())),
        Ok(Event::default().event("response.output_item.added").data(json!({
            "type": "response.output_item.added", "output_index": 0,
            "item": { "type": "message", "id": &msg_id, "role": "assistant", "content": [], "status": "in_progress" }
        }).to_string())),
        Ok(Event::default().event("response.output_text.delta").data(json!({
            "type": "response.output_text.delta", "item_id": &msg_id, "output_index": 0,
            "content_index": 0, "delta": &text
        }).to_string())),
        Ok(Event::default().event("response.output_item.done").data(json!({
            "type": "response.output_item.done", "output_index": 0,
            "item": { "type": "message", "id": &msg_id, "role": "assistant", "status": "completed",
                "content": [{"type": "output_text", "text": &text}] }
        }).to_string())),
        Ok(Event::default().event("response.completed").data(json!({
            "type": "response.completed",
            "response": response_obj
        }).to_string())),
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
    fn test_approximate_value_tokens_accepts_partial_body() {
        let tokens = approximate_value_tokens(&json!({"input": "hello"}));
        assert!(tokens > 0);
    }
}
