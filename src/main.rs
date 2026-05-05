mod session;
mod stream;
mod translate;
mod types;

use anyhow::{bail, Result};
use axum::{
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use reqwest::{Client, Url};
use session::SessionStore;
use dashmap::DashSet;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};
use types::*;

#[derive(Parser, Debug)]
#[command(name = "codex-relay", about = "Responses API <-> Chat Completions bridge")]
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
}

#[derive(Clone)]
struct AppState {
    sessions: SessionStore,
    client: Client,
    upstream: Arc<Url>,
    api_key: Arc<String>,
    model_map: Arc<ModelMap>,
    /// Track which non-function tool names have already been warned about.
    tool_drop_warned: DashSet<String>,
    /// Optional vision/multimodal upstream (e.g. MiniMax for images)
    vision_upstream: Option<Arc<Url>>,
    /// Optional vision API key
    vision_api_key: Arc<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "codex_relay=info".into()),
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
        tool_drop_warned: DashSet::new(),
        vision_upstream,
        vision_api_key: Arc::new(args.vision_api_key),
    };

    let max_bytes = args.max_body_mb * 1024 * 1024;
    let body_limit = axum::extract::DefaultBodyLimit::max(max_bytes);

    let app = Router::new()
        .route("/v1/responses", post(handle_responses))
        .route("/v1/models", get(handle_models))
        .route("/v1", get(handle_v1))
        .fallback(handle_fallback)
        .layer(body_limit)
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{}", args.port);
    info!("listening {} -> {} | body:{}MB", addr, state.upstream.as_ref(), args.max_body_mb);

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
    if s.ends_with('/') { s.to_string() } else { format!("{s}/") }
}

async fn handle_v1() -> Response {
    Json(serde_json::json!({"status": "ok"})).into_response()
}

async fn handle_models(State(state): State<AppState>) -> Response {
    debug!("GET /v1/models");
    let url = format!("{}models", join_base(&state.upstream));
    let mut builder = state.client.get(&url);
    if !state.api_key.is_empty() {
        builder = builder.bearer_auth(state.api_key.as_str());
    }
    match builder.send().await {
        Ok(r) if r.status().is_success() => {
            match r.json::<serde_json::Value>().await {
                Ok(body) => Json(body).into_response(),
                Err(e) => {
                    warn!("upstream models parse error: {e}");
                    Json(serde_json::json!({ "object": "list", "data": [] })).into_response()
                }
            }
        }
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

async fn handle_responses(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    let req: ResponsesRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            error!("JSON parse error: {e}");
            error!("body prefix: {}", String::from_utf8_lossy(&body[..body.len().min(500)]));
            return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response();
        }
    };

    let original_model = req.model.clone();
    let effort = req.reasoning.as_ref().and_then(|r| r.effort.as_deref());

    info!(
        "← {} effort={}",
        original_model, fmt_codex_effort(effort)
    );

    handle_responses_inner(state, req).await
}

async fn handle_responses_inner(
    state: AppState,
    req: ResponsesRequest,
) -> Response {
    let history = req
        .previous_response_id
        .as_deref()
        .map(|id| state.sessions.get_history(id))
        .unwrap_or_default();

    let original_model = req.model.clone();
    let mapped_model = resolve_model(&original_model, &state.model_map);
    let effort = req.reasoning.as_ref().and_then(|r| r.effort.as_deref());
    let (reasoning_effort, thinking) = map_effort(effort);
    let msg_count = match &req.input {
        ResponsesInput::Messages(ref items) => items.len(),
        _ => 1,
    };

    // Debug: log raw tool definitions to understand what Codex sends
    if !req.tools.is_empty() {
        debug!(
            "raw tools received: {}",
            serde_json::to_string(&req.tools).unwrap_or_default()
        );
    }

    // Warn about non-function tools being dropped — once per tool type, then debug
    let dropped = non_function_tool_types(&req.tools);
    if !dropped.is_empty() {
        let new_drops: Vec<&String> = dropped
            .iter()
            .filter(|name| state.tool_drop_warned.insert(name.to_string()))
            .collect();
        let known_drops: Vec<&String> = dropped
            .iter()
            .filter(|name| !new_drops.contains(name))
            .collect();
        if !new_drops.is_empty() {
            warn!(
                "dropping unsupported tool(s) (first occurrence): {:?}",
                new_drops
            );
        }
        if !known_drops.is_empty() {
            debug!(
                "dropping unsupported tool(s) (repeat): {:?}",
                known_drops
            );
        }
    }

    let translated = translate::to_chat_request(&req, history.clone(), &state.sessions, &state.model_map);
    let mut chat_req = translated.chat;

    // Route to vision API if images are present and a vision upstream is configured
    let (url, api_key) = if translated.has_images && state.vision_upstream.is_some() {
        let vu = state.vision_upstream.as_ref().unwrap();
        let url = format!("{}chat/completions", join_base(vu.as_ref()));
        info!("📷 routing to vision upstream: {}", vu.as_ref());
        (url, state.vision_api_key.clone())
    } else {
        let url = format!("{}chat/completions", join_base(&state.upstream));
        (url, state.api_key.clone())
    };

    let vision_label = if translated.has_images { " 📷" } else { "" };
    info!(
        "→ {} effort={} thinking={} msgs={}{}",
        mapped_model, fmt_effort(&reasoning_effort), fmt_thinking(&thinking), msg_count, vision_label
    );

    if req.stream {
        let response_id = state.sessions.new_id();
        chat_req.stream = true;
        let request_messages = chat_req.messages.clone();
        stream::translate_stream(stream::StreamArgs {
            client: state.client,
            url,
            api_key,
            chat_req,
            response_id,
            sessions: state.sessions,
            prior_messages: history,
            request_messages,
            model: mapped_model,
            model_map: state.model_map.as_ref().clone(),
        })
        .into_response()
    } else {
        chat_req.stream = false;
        let start = Instant::now();
        let resp = handle_blocking(state.clone(), chat_req, url, mapped_model, api_key).await;
        let elapsed = start.elapsed();
        debug!("blocking request completed in {:.0}ms", elapsed.as_millis());
        resp
    }
}

async fn handle_blocking(
    state: AppState,
    chat_req: types::ChatRequest,
    url: String,
    model: String,
    api_key: Arc<String>,
) -> Response {
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
            (StatusCode::BAD_GATEWAY, e.to_string()).into_response()
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            error!("upstream {}: {}", status.as_u16(), body);
            (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                body,
            )
                .into_response()
        }
        Ok(r) => match r.json::<ChatResponse>().await {
            Err(e) => {
                error!("parse error: {e}");
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
                full_history.push(assistant_msg);
                let response_id = state.sessions.save(full_history);

                let (resp, _) = translate::from_chat_response(response_id, &model, chat_resp);
                Json(resp).into_response()
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
