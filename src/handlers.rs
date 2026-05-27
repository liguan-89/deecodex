use std::collections::HashMap;

use crate::accounts::{
    account_routing_options, Account, AccountAuthMode, AccountClientKind, AccountClientSurface,
    AccountRuntimeStatus, AccountStore, EndpointConfig, EndpointKind, GlueVisionStrategy,
    UnsupportedImagePolicy, VisionMode,
};
use crate::anthropic;
use crate::cache::RequestCache;
use crate::config::Args;
use crate::executor::{ComputerActionInvocation, LocalExecutorConfig, McpToolInvocation};
use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;
use crate::request_history::{HistoryContext, HistoryRecord};
use crate::session::SessionStore;
use crate::token_anomaly::TokenTracker;
use crate::types::*;
use crate::utils::{limit_function_call_outputs, merge_response_extra};
use crate::vision::{
    build_minimax_vlm_body, handle_minimax_vlm, request_minimax_vlm_text,
    strip_images_from_chat_request, VlmArgs,
};
use crate::{
    capability, dev_pipeline, files, prompts, providers, sse::SseState, stream, translate,
    vector_stores,
};
use anyhow::{bail, Result};
use axum::{
    extract::{Multipart, Path, Query, Request, State},
    http::StatusCode,
    http::{header, HeaderMap},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use sha2::Digest;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

const LOCAL_OUTPUT_PREFIX_ITEMS_KEY: &str = "x_deecodex_local_output_prefix_items";
pub const CODEX_OFFICIAL_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
const DEFAULT_IMAGE_TOOL_MODEL: &str = "gpt-image-2";
const DEFAULT_IMAGE_MAIN_MODEL: &str = "gpt-5.4-mini";
static CODEX_OFFICIAL_POOL_CURSOR: AtomicU64 = AtomicU64::new(0);

#[allow(dead_code)]
#[derive(Clone)]
pub struct AppState {
    pub sessions: SessionStore,
    pub client: Client,
    pub upstream: Arc<tokio::sync::RwLock<Url>>,
    pub api_key: Arc<tokio::sync::RwLock<String>>,
    pub model_map: Arc<tokio::sync::RwLock<ModelMap>>,
    pub vision_upstream: Arc<tokio::sync::RwLock<Option<Url>>>,
    pub vision_api_key: Arc<tokio::sync::RwLock<String>>,
    pub vision_model: Arc<tokio::sync::RwLock<String>>,
    pub vision_endpoint: Arc<tokio::sync::RwLock<String>>,
    pub start_time: std::time::Instant,
    pub request_cache: RequestCache,
    pub prompts: Arc<prompts::PromptRegistry>,
    pub files: files::FileStore,
    pub vector_stores: vector_stores::VectorStoreRegistry,
    pub background_tasks: Arc<dashmap::DashMap<String, tokio::task::JoinHandle<()>>>,
    pub chinese_thinking: bool,
    pub codex_auto_inject: bool,
    pub codex_persistent_inject: bool,
    pub codex_launch_with_cdp: bool,
    pub cdp_port: u16,
    pub port: u16,
    pub rate_limiter: Option<Arc<RateLimiter>>,
    pub metrics: Arc<Metrics>,
    pub tool_policy: Arc<tokio::sync::RwLock<ToolPolicy>>,
    pub executors: Arc<tokio::sync::RwLock<LocalExecutorConfig>>,
    pub token_tracker: Arc<TokenTracker>,
    pub data_dir: Arc<std::path::PathBuf>,
    pub account_store: Arc<tokio::sync::RwLock<AccountStore>>,
    pub active_account: Arc<tokio::sync::RwLock<Account>>,
    /// 强制推理强度，覆盖 Codex 请求中的 effort（来自活跃账号）
    pub reasoning_effort_override: Arc<tokio::sync::RwLock<Option<String>>>,
    /// Claude Extended Thinking Token 预算（来自活跃账号）
    pub thinking_tokens: Arc<tokio::sync::RwLock<Option<u32>>>,
    /// 自定义 HTTP 头（来自活跃账号）
    pub custom_headers: Arc<tokio::sync::RwLock<HashMap<String, String>>>,
    /// 请求超时秒数（来自活跃账号）
    pub request_timeout_secs: Arc<tokio::sync::RwLock<Option<u64>>>,
    /// 请求历史持久化存储
    pub request_history: Arc<crate::request_history::RequestHistoryStore>,
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
    api_key: String,
    custom_headers: HashMap<String, String>,
    request_timeout_secs: Option<u64>,
    max_retries: Option<u32>,
    response_id: String,
    store_response: bool,
    conversation_id: Option<String>,
    response_extra: Value,
    req: &'a ResponsesRequest,
    history_context: HistoryContext,
    start: Instant,
}

struct BypassArgs {
    state: AppState,
    body: axum::body::Bytes,
    upstream_url: Url,
    endpoint_path: String,
    api_key: String,
    custom_headers: HashMap<String, String>,
    timeout_secs: Option<u64>,
    max_retries: Option<u32>,
    response_id: String,
    store_response: bool,
    model: String,
    requested_service_tier: Option<String>,
    history_context: HistoryContext,
}

struct CapabilityObservationResult {
    message: Option<ChatMessage>,
    suppress_vision_route: bool,
}

struct AnthropicArgs<'a> {
    state: AppState,
    chat_req: ChatRequest,
    url: String,
    model: String,
    api_key: String,
    auth_scheme: providers::AuthScheme,
    custom_headers: HashMap<String, String>,
    request_timeout_secs: Option<u64>,
    max_retries: Option<u32>,
    thinking_tokens: Option<u32>,
    response_id: String,
    store_response: bool,
    conversation_id: Option<String>,
    response_extra: Value,
    req: &'a ResponsesRequest,
    history_context: HistoryContext,
    start: Instant,
}

struct UpstreamFailureArgs<'a> {
    state: &'a AppState,
    response_id: String,
    model: String,
    url: String,
    store_response: bool,
    req: &'a ResponsesRequest,
    response_extra: Value,
    start: Instant,
    code: String,
    message: String,
    status: StatusCode,
    history_context: HistoryContext,
}

struct DevPipelineResponseArgs<'a> {
    state: AppState,
    req: &'a ResponsesRequest,
    output: dev_pipeline::DevPipelineOutput,
    request_input_items: Vec<Value>,
    store_response: bool,
    conversation_id: Option<String>,
    response_extra: Value,
    history_context: HistoryContext,
    start: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountRouteSurface {
    Global,
    CodexCli,
    CodexDesktop,
    DexAssistant,
}

impl AccountRouteSurface {
    fn explicit_surface(self) -> Option<AccountClientSurface> {
        match self {
            AccountRouteSurface::Global => None,
            AccountRouteSurface::CodexCli => Some(AccountClientSurface::Cli),
            AccountRouteSurface::CodexDesktop => Some(AccountClientSurface::Desktop),
            AccountRouteSurface::DexAssistant => None,
        }
    }

    fn responses_path(self) -> &'static str {
        match self {
            AccountRouteSurface::Global => "/v1/responses",
            AccountRouteSurface::CodexCli => "/codex-cli/v1/responses",
            AccountRouteSurface::CodexDesktop => "/codex-desktop/v1/responses",
            AccountRouteSurface::DexAssistant => "/dex-assistant/v1/responses",
        }
    }
}

fn infer_account_route_surface(headers: &HeaderMap) -> AccountRouteSurface {
    if let Some(surface) = headers
        .get("x-deecodex-client-surface")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase())
    {
        if surface.contains("dex") || surface.contains("assistant") {
            return AccountRouteSurface::DexAssistant;
        }
        if surface.contains("desktop") {
            return AccountRouteSurface::CodexDesktop;
        }
        if surface.contains("cli") {
            return AccountRouteSurface::CodexCli;
        }
    }

    let marker = ["user-agent", "originator", "x-codex-client"]
        .iter()
        .filter_map(|name| headers.get(*name))
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();

    if marker.contains("codex_desktop")
        || marker.contains("codex desktop")
        || marker.contains("codex-desktop")
    {
        return AccountRouteSurface::CodexDesktop;
    }
    if marker.contains("codex_cli")
        || marker.contains("codex cli")
        || marker.contains("codex-cli")
        || marker.contains("codex_cli_rs")
    {
        return AccountRouteSurface::CodexCli;
    }
    AccountRouteSurface::Global
}

async fn active_account_endpoint(state: &AppState) -> (Account, EndpointConfig) {
    let (mut account, mut endpoint) = active_account_endpoint_without_hot_overrides(state).await;
    // AppState 的热字段仍是运行时真值；测试和托盘切换都会直接更新它。
    endpoint.base_url = state.upstream.read().await.to_string();
    account.sync_legacy_from_endpoint(&endpoint);
    (account, endpoint)
}

async fn active_account_endpoint_without_hot_overrides(
    state: &AppState,
) -> (Account, EndpointConfig) {
    let mut account = state.active_account.read().await.clone();
    if account.endpoints.is_empty() {
        account.normalize_v2();
    }
    let endpoint_id = state.account_store.read().await.active_endpoint_id.clone();
    account_endpoint_or_fallback(account, endpoint_id.as_deref())
}

async fn active_account_endpoint_for_route(
    state: &AppState,
    route_surface: AccountRouteSurface,
) -> (Account, EndpointConfig) {
    if route_surface == AccountRouteSurface::DexAssistant {
        let store = state.account_store.read().await;
        if let Some(account) = store.active_account_for_dex_assistant().cloned() {
            let endpoint_id = store
                .active_endpoint_id_for_dex_assistant()
                .map(str::to_string);
            return account_endpoint_or_fallback(account, endpoint_id.as_deref());
        }
        drop(store);

        warn!("未找到 DEX 助手对应的活跃账号，回退到全局活跃账号");
        return active_account_endpoint(state).await;
    }

    let Some(surface) = route_surface.explicit_surface() else {
        return active_account_endpoint(state).await;
    };

    let store = state.account_store.read().await;
    if let Some(account) = store.active_account_for_surface(&surface).cloned() {
        let endpoint_id = store
            .active_endpoint_id_for_surface(&AccountClientKind::Codex, &surface)
            .map(str::to_string);
        return account_endpoint_or_fallback(account, endpoint_id.as_deref());
    }
    drop(store);

    warn!(
        "未找到 {:?} 对应的 Codex 活跃账号，回退到全局活跃账号",
        surface
    );
    active_account_endpoint(state).await
}

fn account_endpoint_or_fallback(
    mut account: Account,
    active_endpoint_id: Option<&str>,
) -> (Account, EndpointConfig) {
    if account.endpoints.is_empty() {
        account.normalize_v2();
    }
    let endpoint = account
        .active_endpoint(active_endpoint_id)
        .cloned()
        .or_else(|| account.endpoints.first().cloned())
        .unwrap_or_else(|| {
            tracing::warn!("活跃账号没有端点，使用默认 OpenRouter Chat 端点兜底");
            let mut fallback = crate::accounts::EndpointConfig {
                id: "fallback_openrouter".into(),
                name: "Fallback OpenRouter".into(),
                kind: crate::accounts::EndpointKind::OpenAiChat,
                base_url: "https://openrouter.ai/api/v1".into(),
                path: String::new(),
                template_id: "openrouter".into(),
                template_version: 1,
                model_map: std::collections::HashMap::new(),
                model_profiles: std::collections::HashMap::new(),
                vision: Default::default(),
                custom_headers: std::collections::HashMap::new(),
                request_timeout_secs: None,
                max_retries: None,
                context_window_override: None,
                reasoning_effort_override: None,
                thinking_tokens: None,
                fast_mode_enabled: false,
                fast_service_tier: "priority".into(),
                balance_url: String::new(),
            };
            fallback.model_map = account.model_map.clone();
            fallback
        });
    account.sync_legacy_from_endpoint(&endpoint);
    (account, endpoint)
}

async fn codex_official_account_endpoint(
    state: &AppState,
    requested_model: &str,
    route_surface: AccountRouteSurface,
) -> Option<(Account, EndpointConfig)> {
    let store = state.account_store.read().await.clone();
    let cursor = CODEX_OFFICIAL_POOL_CURSOR.fetch_add(1, Ordering::Relaxed);
    select_codex_official_account_endpoint(
        &store,
        requested_model,
        crate::accounts::now_secs(),
        cursor,
        route_surface,
    )
}

fn codex_official_pool_unavailable_response() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "error": {
                "message": "Codex 官方号池暂无可用账号：所有参与账号都在冷却、额度耗尽或已停用",
                "type": "rate_limit_error",
                "code": "codex_official_pool_unavailable"
            }
        })),
    )
        .into_response()
}

fn select_codex_official_account_endpoint(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    cursor: u64,
    route_surface: AccountRouteSurface,
) -> Option<(Account, EndpointConfig)> {
    let active_account = match route_surface {
        AccountRouteSurface::DexAssistant => store.active_account_for_dex_assistant()?,
        _ => match route_surface.explicit_surface() {
            Some(surface) => store.active_account_for_surface(&surface)?,
            None => store.active_account()?,
        },
    };
    let active_endpoint_id = active_endpoint_id_for_route(store, route_surface);
    let active_endpoint = active_official_endpoint(active_account, active_endpoint_id.as_deref())?;
    if active_endpoint.kind != EndpointKind::CodexOfficial {
        return None;
    }

    let active_pool = account_routing_options(active_account).pool;
    let active_surface = active_account.client_surface.clone();
    let mut candidates: Vec<(Account, EndpointConfig, i64, u32)> = store
        .accounts
        .iter()
        .filter(|account| account.client_kind.is_codex())
        .filter(|account| account.client_surface == active_surface)
        .filter_map(|account| {
            let routing = account_routing_options(account);
            if !routing.effective_enabled() || routing.pool != active_pool {
                return None;
            }
            let endpoint = official_endpoint_for_account(store, account, route_surface)?;
            let mapped_model = resolve_model(requested_model, &endpoint.model_map);
            if !account_runtime_ready(account, &mapped_model, now) {
                return None;
            }
            let mut account = account.clone();
            account.sync_legacy_from_endpoint(&endpoint);
            Some((account, endpoint, routing.priority, routing.weight))
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .2
            .cmp(&left.2)
            .then_with(|| left.0.id.cmp(&right.0.id))
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    let max_priority = candidates.first().map(|candidate| candidate.2)?;
    let top: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| candidate.2 == max_priority)
        .collect();
    let total_weight: u64 = top
        .iter()
        .map(|candidate| u64::from(candidate.3.max(1)))
        .sum();
    if total_weight == 0 {
        return None;
    }
    let mut slot = cursor % total_weight;
    for (account, endpoint, _priority, weight) in top {
        let weight = u64::from(weight.max(1));
        if slot < weight {
            return Some((account, endpoint));
        }
        slot = slot.saturating_sub(weight);
    }
    None
}

fn active_endpoint_id_for_route(
    store: &AccountStore,
    route_surface: AccountRouteSurface,
) -> Option<String> {
    match route_surface {
        AccountRouteSurface::DexAssistant => store
            .active_endpoint_id_for_dex_assistant()
            .map(str::to_string),
        _ => match route_surface.explicit_surface() {
            Some(surface) => store
                .active_endpoint_id_for_surface(&AccountClientKind::Codex, &surface)
                .map(str::to_string),
            None => store.active_endpoint_id.clone(),
        },
    }
}

fn active_account_id_for_route(
    store: &AccountStore,
    route_surface: AccountRouteSurface,
) -> Option<String> {
    match route_surface {
        AccountRouteSurface::DexAssistant => store
            .active_selection_for_dex_assistant()
            .and_then(|selection| selection.account_id.clone()),
        _ => match route_surface.explicit_surface() {
            Some(surface) => store
                .active_selection_for_surface(&AccountClientKind::Codex, &surface)
                .and_then(|selection| selection.account_id.clone()),
            None => store
                .active_account_id
                .clone()
                .or_else(|| store.active_id.clone()),
        },
    }
}

fn active_official_endpoint(
    account: &Account,
    active_endpoint_id: Option<&str>,
) -> Option<EndpointConfig> {
    account
        .active_endpoint(active_endpoint_id)
        .filter(|endpoint| endpoint.kind == EndpointKind::CodexOfficial)
        .cloned()
}

fn official_endpoint_for_account(
    store: &AccountStore,
    account: &Account,
    route_surface: AccountRouteSurface,
) -> Option<EndpointConfig> {
    if active_account_id_for_route(store, route_surface).as_deref() == Some(&account.id) {
        let active_endpoint_id = active_endpoint_id_for_route(store, route_surface);
        if let Some(endpoint) = active_official_endpoint(account, active_endpoint_id.as_deref()) {
            return Some(endpoint);
        }
    }
    account
        .endpoints
        .iter()
        .find(|endpoint| endpoint.kind == EndpointKind::CodexOfficial)
        .cloned()
}

fn account_runtime_ready(account: &Account, mapped_model: &str, now: u64) -> bool {
    runtime_retry_ready(account.runtime_state.next_retry_after, now)
        && account
            .runtime_state
            .model_states
            .get(mapped_model)
            .is_none_or(|state| {
                runtime_retry_ready(state.next_retry_after, now)
                    || !matches!(
                        state.status,
                        AccountRuntimeStatus::CoolingDown | AccountRuntimeStatus::QuotaExceeded
                    )
            })
}

fn runtime_retry_ready(next_retry_after: Option<u64>, now: u64) -> bool {
    next_retry_after.is_none_or(|retry_at| retry_at <= now)
}

fn codex_official_originator(account: &Account) -> &'static str {
    match account.client_surface {
        AccountClientSurface::Desktop => "codex_desktop",
        AccountClientSurface::Cli => "codex_cli_rs",
    }
}

fn account_client_kind_slug(kind: &AccountClientKind) -> &'static str {
    match kind {
        AccountClientKind::Codex => "codex",
        AccountClientKind::ClaudeCode => "claude_code",
        AccountClientKind::Openclaw => "openclaw",
        AccountClientKind::Hermes => "hermes",
        AccountClientKind::GenericClient => "generic_client",
    }
}

fn endpoint_kind_slug(kind: &EndpointKind) -> &'static str {
    match kind {
        EndpointKind::OpenAiChat => "openai_chat",
        EndpointKind::OpenAiResponses => "openai_responses",
        EndpointKind::AnthropicMessages => "anthropic_messages",
        EndpointKind::CodexOfficial => "codex_official",
        EndpointKind::CustomChat => "custom_chat",
        EndpointKind::CustomResponses => "custom_responses",
    }
}

fn history_context_for(
    account: &Account,
    endpoint: &EndpointConfig,
    request_path: &str,
) -> HistoryContext {
    let profile = providers::profile_for_account(account);
    HistoryContext {
        client_kind: account_client_kind_slug(&account.client_kind).into(),
        account_id: account.id.clone(),
        account_name: account.name.clone(),
        endpoint_kind: endpoint_kind_slug(&endpoint.kind).into(),
        request_path: request_path.into(),
        provider: account.provider.clone(),
        provider_profile: profile.slug,
    }
}

#[allow(clippy::too_many_arguments)]
fn record_from_context(
    context: &HistoryContext,
    id: String,
    model: String,
    status: String,
    input_tokens: u32,
    output_tokens: u32,
    duration_ms: u64,
    upstream_url: String,
    error_msg: String,
    cache_hit: bool,
) -> HistoryRecord {
    context.record(
        id,
        now_unix_secs(),
        model,
        status,
        input_tokens,
        output_tokens,
        duration_ms,
        upstream_url,
        error_msg,
        cache_hit,
    )
}

struct SseHistoryBodyContext {
    request_history: Arc<crate::request_history::RequestHistoryStore>,
    history_context: HistoryContext,
    response_id: String,
    model: String,
    start: Instant,
    upstream_url: String,
    http_status: StatusCode,
}

fn history_recording_sse_body<S>(
    upstream_stream: S,
    context: SseHistoryBodyContext,
) -> axum::body::Body
where
    S: futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
{
    let body_stream = async_stream::stream! {
        let mut source = Box::pin(upstream_stream);
        let mut usage = SseUsageObservation::default();
        let mut stream_error: Option<String> = None;
        while let Some(item) = source.next().await {
            match item {
                Ok(bytes) => {
                    usage.ingest(&bytes);
                    yield Ok::<Bytes, std::io::Error>(bytes);
                }
                Err(err) => {
                    let message = err.to_string();
                    stream_error = Some(message.clone());
                    yield Err(std::io::Error::other(message));
                    break;
                }
            }
        }
        usage.finish();
        let error_msg = stream_error.unwrap_or_else(|| {
            if context.http_status.is_success() {
                String::new()
            } else {
                format!("HTTP {}", context.http_status.as_u16())
            }
        });
        let status = if context.http_status.is_success() && error_msg.is_empty() {
            "completed"
        } else {
            "failed"
        };
        let _ = context
            .request_history
            .record(record_from_context(
                &context.history_context,
                context.response_id,
                context.model,
                status.into(),
                usage.input_tokens,
                usage.output_tokens,
                context.start.elapsed().as_millis() as u64,
                context.upstream_url,
                error_msg,
                usage.cache_hit,
            ))
            .await;
    };
    axum::body::Body::from_stream(body_stream)
}

fn client_proxy_enabled(account: &Account) -> bool {
    account
        .client_options
        .get("proxy_recording_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn client_proxy_token(account: &Account) -> Option<&str> {
    account
        .client_options
        .get("proxy_token")
        .and_then(Value::as_str)
        .filter(|token| !token.trim().is_empty())
}

fn proxy_token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let value = value.trim();
        if let Some(token) = value.strip_prefix("Bearer ") {
            let token = token.trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

async fn account_for_proxy_token(state: &AppState, token: &str) -> Option<Account> {
    let store = state.account_store.read().await;
    store
        .accounts
        .iter()
        .find(|account| {
            !account.client_kind.is_codex()
                && client_proxy_enabled(account)
                && client_proxy_token(account) == Some(token)
        })
        .cloned()
}

fn history_context_for_client_proxy(
    account: &Account,
    endpoint_kind: &str,
    request_path: &str,
) -> HistoryContext {
    let profile = providers::profile_for_account(account);
    HistoryContext {
        client_kind: account_client_kind_slug(&account.client_kind).into(),
        account_id: account.id.clone(),
        account_name: account.name.clone(),
        endpoint_kind: endpoint_kind.into(),
        request_path: request_path.into(),
        provider: account.provider.clone(),
        provider_profile: profile.slug,
    }
}

fn model_from_json_body(body: &[u8], fallback: &str) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn join_proxy_upstream(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

fn anthropic_messages_path(base_url: &str) -> &'static str {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        "messages"
    } else {
        "v1/messages"
    }
}

fn apply_account_custom_headers(
    mut builder: reqwest::RequestBuilder,
    account: &Account,
) -> reqwest::RequestBuilder {
    for (name, value) in &account.custom_headers {
        if let (Ok(header_name), Ok(header_value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            header::HeaderValue::from_str(value),
        ) {
            builder = builder.header(header_name, header_value);
        }
    }
    builder
}

fn apply_proxy_upstream_headers(
    mut builder: reqwest::RequestBuilder,
    account: &Account,
) -> reqwest::RequestBuilder {
    let profile = providers::profile_for_account(account);
    for (name, value) in providers::request_headers(&profile, &account.api_key) {
        builder = builder.header(name, value);
    }
    apply_account_custom_headers(builder, account)
}

fn oauth_provider_slug(account: &Account) -> Option<&str> {
    account
        .client_options
        .get("oauth")
        .and_then(Value::as_object)
        .and_then(|oauth| oauth.get("provider"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
}

fn is_claude_official_oauth_proxy_account(account: &Account) -> bool {
    account.auth_mode == AccountAuthMode::OAuth
        && account.client_kind == AccountClientKind::ClaudeCode
        && account.provider.eq_ignore_ascii_case("anthropic")
        && oauth_provider_slug(account)
            .is_some_and(|provider| provider.eq_ignore_ascii_case("claude"))
}

fn header_or_default(headers: &HeaderMap, name: &'static str, default: &'static str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn merge_claude_beta_header(headers: &HeaderMap) -> String {
    let mut betas = headers
        .get("anthropic-beta")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,context-management-2025-06-27,prompt-caching-scope-2026-01-05,structured-outputs-2025-12-15,fast-mode-2026-02-01,redact-thinking-2026-02-12,token-efficient-tools-2026-03-28")
        .to_string();
    if !betas.contains("oauth") {
        betas.push_str(",oauth-2025-04-20");
    }
    if !betas.contains("interleaved-thinking") {
        betas.push_str(",interleaved-thinking-2025-05-14");
    }
    betas
}

fn stable_claude_session_id(account: &Account, token: &str) -> String {
    let seed = format!("{}:{token}", account.id);
    let digest = sha2::Sha256::digest(seed.as_bytes());
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0],
        digest[1],
        digest[2],
        digest[3],
        digest[4],
        digest[5],
        digest[6],
        digest[7],
        digest[8],
        digest[9],
        digest[10],
        digest[11],
        digest[12],
        digest[13],
        digest[14],
        digest[15],
    )
}

fn client_proxy_body_streams(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("stream").and_then(Value::as_bool))
        .unwrap_or(false)
}

fn apply_claude_oauth_proxy_headers(
    mut builder: reqwest::RequestBuilder,
    inbound_headers: &HeaderMap,
    account: &Account,
    access_token: &str,
    stream: bool,
) -> reqwest::RequestBuilder {
    let session_id = inbound_headers
        .get("x-claude-code-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| stable_claude_session_id(account, access_token));

    builder = builder
        .bearer_auth(access_token)
        .header(
            "Anthropic-Version",
            header_or_default(inbound_headers, "anthropic-version", "2023-06-01"),
        )
        .header("Anthropic-Beta", merge_claude_beta_header(inbound_headers))
        .header("X-App", header_or_default(inbound_headers, "x-app", "cli"))
        .header(
            "X-Stainless-Retry-Count",
            header_or_default(inbound_headers, "x-stainless-retry-count", "0"),
        )
        .header(
            "X-Stainless-Runtime",
            header_or_default(inbound_headers, "x-stainless-runtime", "node"),
        )
        .header(
            "X-Stainless-Lang",
            header_or_default(inbound_headers, "x-stainless-lang", "js"),
        )
        .header(
            "X-Stainless-Timeout",
            header_or_default(inbound_headers, "x-stainless-timeout", "600"),
        )
        .header(
            "X-Stainless-Package-Version",
            header_or_default(inbound_headers, "x-stainless-package-version", "0.74.0"),
        )
        .header(
            "X-Stainless-Runtime-Version",
            header_or_default(inbound_headers, "x-stainless-runtime-version", "v24.3.0"),
        )
        .header(
            "X-Stainless-Os",
            header_or_default(inbound_headers, "x-stainless-os", "MacOS"),
        )
        .header(
            "X-Stainless-Arch",
            header_or_default(inbound_headers, "x-stainless-arch", "arm64"),
        )
        .header("X-Claude-Code-Session-Id", session_id)
        .header(
            "User-Agent",
            header_or_default(
                inbound_headers,
                "user-agent",
                "claude-cli/2.1.63 (external, cli)",
            ),
        )
        .header("Connection", "keep-alive");
    if stream {
        builder = builder
            .header("Accept", "text/event-stream")
            .header("Accept-Encoding", "identity");
    } else {
        builder = builder
            .header("Accept", "application/json")
            .header("Accept-Encoding", "gzip, deflate, br, zstd");
    }
    apply_account_custom_headers(builder, account)
}

fn usage_from_object(usage: &serde_json::Map<String, Value>) -> (Option<u32>, Option<u32>, bool) {
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(Value::as_u64)
        .map(|v| v as u32);
    (input, output, is_cache_hit(usage))
}

fn usage_from_value(value: &Value) -> (Option<u32>, Option<u32>, bool) {
    if let Some(obj) = value.get("usage").and_then(Value::as_object) {
        return usage_from_object(obj);
    }
    if let Some(obj) = value
        .get("response")
        .and_then(|r| r.get("usage"))
        .and_then(Value::as_object)
    {
        return usage_from_object(obj);
    }
    if let Some(obj) = value.as_object() {
        if obj.contains_key("input_tokens")
            || obj.contains_key("prompt_tokens")
            || obj.contains_key("output_tokens")
            || obj.contains_key("completion_tokens")
        {
            return usage_from_object(obj);
        }
    }
    (None, None, false)
}

#[derive(Default)]
struct SseUsageObservation {
    line_buffer: String,
    input_tokens: u32,
    output_tokens: u32,
    cache_hit: bool,
}

impl SseUsageObservation {
    fn ingest(&mut self, bytes: &Bytes) {
        self.line_buffer.push_str(&String::from_utf8_lossy(bytes));
        while let Some(pos) = self.line_buffer.find('\n') {
            let line: String = self.line_buffer.drain(..=pos).collect();
            self.observe_line(line.trim_end_matches(['\r', '\n']));
        }
    }

    fn finish(&mut self) {
        if self.line_buffer.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.line_buffer);
        self.observe_line(line.trim_end_matches(['\r', '\n']));
    }

    fn observe_line(&mut self, line: &str) {
        let data = line
            .strip_prefix("data:")
            .map(str::trim_start)
            .unwrap_or(line)
            .trim();
        if data.is_empty() || data == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        let (input, output, hit) = usage_from_value(&value);
        if let Some(input) = input {
            self.input_tokens = input;
        }
        if let Some(output) = output {
            self.output_tokens = output;
        }
        self.cache_hit |= hit;
    }
}

fn extract_proxy_response_usage(bytes: &[u8]) -> (u32, u32, bool) {
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut cache_hit = false;
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        let (input, output, hit) = usage_from_value(&value);
        if let Some(input) = input {
            input_tokens = input;
        }
        if let Some(output) = output {
            output_tokens = output;
        }
        cache_hit |= hit;
    }

    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let data = line.strip_prefix("data: ").unwrap_or(line).trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let (input, output, hit) = usage_from_value(&value);
        if let Some(input) = input {
            input_tokens = input;
        }
        if let Some(output) = output {
            output_tokens = output;
        }
        cache_hit |= hit;
    }
    (input_tokens, output_tokens, cache_hit)
}

async fn forward_client_proxy_request(
    state: AppState,
    headers: HeaderMap,
    body: axum::body::Bytes,
    request_path: &'static str,
    endpoint_kind: &'static str,
    upstream_path: String,
) -> Response {
    let Some(token) = proxy_token_from_headers(&headers) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "缺少 deecodex 客户端代理 token",
                    "type": "authentication_error",
                    "code": "missing_proxy_token"
                }
            })),
        )
            .into_response();
    };
    let Some(mut account) = account_for_proxy_token(&state, &token).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "客户端代理 token 无效或未启用请求记录",
                    "type": "authentication_error",
                    "code": "invalid_proxy_token"
                }
            })),
        )
            .into_response();
    };

    let body = sanitize_client_proxy_body(endpoint_kind, &account, body);

    let response_id = state.sessions.new_id();
    let start = Instant::now();
    let model = model_from_json_body(&body, &account.default_model);
    let upstream_url = join_proxy_upstream(&account.upstream, &upstream_path);
    let history_context = history_context_for_client_proxy(&account, endpoint_kind, request_path);
    let builder = state
        .client
        .post(&upstream_url)
        .header("Content-Type", "application/json");
    let mut builder = if is_claude_official_oauth_proxy_account(&account) {
        match fresh_oauth_access_token(&state, &mut account).await {
            Ok(access_token) if !access_token.trim().is_empty() => {
                apply_claude_oauth_proxy_headers(
                    builder,
                    &headers,
                    &account,
                    &access_token,
                    client_proxy_body_streams(&body),
                )
            }
            Ok(_) => {
                update_runtime_result(
                    &state,
                    &account.id,
                    &model,
                    StatusCode::UNAUTHORIZED,
                    "Claude 官方 OAuth access token 为空".into(),
                    None,
                )
                .await;
                state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id,
                        model,
                        "failed".into(),
                        0,
                        0,
                        start.elapsed().as_millis() as u64,
                        upstream_url,
                        "missing oauth token".into(),
                        false,
                    ))
                    .await;
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": {
                            "message": "Claude 官方账号缺少 OAuth access token",
                            "type": "authentication_error",
                            "code": "missing_oauth_token"
                        }
                    })),
                )
                    .into_response();
            }
            Err(err) => {
                update_runtime_result(
                    &state,
                    &account.id,
                    &model,
                    StatusCode::UNAUTHORIZED,
                    format!("Claude OAuth token refresh failed: {err}"),
                    None,
                )
                .await;
                state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id,
                        model,
                        "failed".into(),
                        0,
                        0,
                        start.elapsed().as_millis() as u64,
                        upstream_url,
                        format!("oauth refresh failed: {err}"),
                        false,
                    ))
                    .await;
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({
                        "error": {
                            "message": format!("Claude OAuth token refresh failed: {err}"),
                            "type": "authentication_error",
                            "code": "oauth_refresh_failed"
                        }
                    })),
                )
                    .into_response();
            }
        }
    } else {
        apply_proxy_upstream_headers(builder, &account)
    };
    if let Some(secs) = account.request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let result = builder.body(body.clone()).send().await;
    let resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            update_runtime_result(
                &state,
                &account.id,
                &model,
                StatusCode::BAD_GATEWAY,
                format!("upstream connection error: {err}"),
                None,
            )
            .await;
            state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id,
                    model,
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    upstream_url,
                    format!("connection error: {err}"),
                    false,
                ))
                .await;
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": format!("upstream connection error: {err}"),
                        "type": "api_error",
                        "code": "upstream_error"
                    }
                })),
            )
                .into_response();
        }
    };

    let status = resp.status();
    let retry_after = retry_after_secs(resp.headers());
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id,
                    model,
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    upstream_url,
                    format!("response read error: {err}"),
                    false,
                ))
                .await;
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": format!("upstream response read error: {err}"),
                        "type": "api_error",
                        "code": "upstream_error"
                    }
                })),
            )
                .into_response();
        }
    };

    let (input_tokens, output_tokens, cache_hit) = if status.is_success() {
        extract_proxy_response_usage(&bytes)
    } else {
        (0, 0, false)
    };
    update_runtime_result(
        &state,
        &account.id,
        &model,
        status,
        if status.is_success() {
            String::new()
        } else {
            format!("HTTP {}", status.as_u16())
        },
        retry_after,
    )
    .await;
    state
        .request_history
        .record(record_from_context(
            &history_context,
            response_id,
            model,
            if status.is_success() {
                "completed".into()
            } else {
                "failed".into()
            },
            input_tokens,
            output_tokens,
            start.elapsed().as_millis() as u64,
            upstream_url,
            if status.is_success() {
                String::new()
            } else {
                format!("HTTP {}", status.as_u16())
            },
            cache_hit,
        ))
        .await;

    let mut response = Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .body(axum::body::Body::from(bytes))
        .unwrap();
    if request_path == "/v1/chat/completions" {
        response.headers_mut().insert(
            header::HeaderName::from_static("x-deecodex-client-kind"),
            header::HeaderValue::from_str(account_client_kind_slug(&account.client_kind))
                .unwrap_or_else(|_| header::HeaderValue::from_static("client")),
        );
    }
    response
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/v1/responses", post(handle_responses))
        .route("/codex-cli/v1/responses", post(handle_responses_codex_cli))
        .route(
            "/codex-desktop/v1/responses",
            post(handle_responses_codex_desktop),
        )
        .route(
            "/dex-assistant/v1/responses",
            post(handle_responses_dex_assistant),
        )
        .route("/v1/chat/completions", post(handle_client_chat_completions))
        .route("/v1/images/generations", post(handle_images_generations))
        .route("/v1/images/edits", post(handle_images_edits))
        .route("/v1/messages", post(handle_client_anthropic_messages_v1))
        .route("/messages", post(handle_client_anthropic_messages))
        .route("/v1/responses/compact", post(handle_compact_response))
        .route("/v1/responses/input_tokens", post(handle_input_tokens))
        .route(
            "/codex-cli/v1/responses/compact",
            post(handle_compact_response),
        )
        .route(
            "/codex-cli/v1/responses/input_tokens",
            post(handle_input_tokens),
        )
        .route(
            "/codex-desktop/v1/responses/compact",
            post(handle_compact_response),
        )
        .route(
            "/codex-desktop/v1/responses/input_tokens",
            post(handle_input_tokens),
        )
        .route(
            "/v1/responses/:response_id",
            get(handle_get_response).delete(handle_delete_response),
        )
        .route(
            "/codex-cli/v1/responses/:response_id",
            get(handle_get_response).delete(handle_delete_response),
        )
        .route(
            "/codex-desktop/v1/responses/:response_id",
            get(handle_get_response).delete(handle_delete_response),
        )
        .route(
            "/v1/responses/:response_id/cancel",
            post(handle_cancel_response),
        )
        .route(
            "/codex-cli/v1/responses/:response_id/cancel",
            post(handle_cancel_response),
        )
        .route(
            "/codex-desktop/v1/responses/:response_id/cancel",
            post(handle_cancel_response),
        )
        .route(
            "/v1/responses/:response_id/input_items",
            get(handle_input_items),
        )
        .route(
            "/codex-cli/v1/responses/:response_id/input_items",
            get(handle_input_items),
        )
        .route(
            "/codex-desktop/v1/responses/:response_id/input_items",
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
        .route("/codex-cli/v1/models", get(handle_models))
        .route("/codex-desktop/v1/models", get(handle_models))
        .route("/health", get(handle_health))
        .route("/v1", get(handle_v1))
        .route("/codex-cli/v1", get(handle_v1))
        .route("/codex-desktop/v1", get(handle_v1))
        .route("/metrics", get(handle_metrics))
        // Codex 线程聚合（跨 provider）
        .route("/api/threads", get(handle_list_threads_api))
        .route("/api/threads/status", get(handle_threads_status_api))
        .route("/api/threads/unified", get(handle_list_unified_threads_api))
        .route(
            "/api/threads/unified/status",
            get(handle_unified_thread_sources_api),
        )
        .route(
            "/api/threads/unified/content",
            get(handle_unified_thread_content_api),
        )
        .route("/api/threads/migrate", post(handle_migrate_threads_api))
        .route("/api/threads/restore", post(handle_restore_threads_api))
        .fallback(handle_fallback)
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
    let model_map = state.model_map.read().await;
    if !model_map.is_empty() {
        let data: Vec<serde_json::Value> = model_map
            .keys()
            .map(|id| {
                json!({
                    "id": id,
                    "object": "model",
                    "owned_by": "deecodex"
                })
            })
            .collect();
        return Json(json!({ "object": "list", "data": data })).into_response();
    }
    // fallback: proxy to upstream
    let upstream = state.upstream.read().await;
    let url = format!("{}models", join_base(&upstream));
    let mut builder = state.client.get(&url);
    let api_key = state.api_key.read().await;
    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key.as_str());
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

async fn handle_images_generations(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle_images_api(state, headers, body, ImageAction::Generate).await
}

async fn handle_images_edits(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle_images_api(state, headers, body, ImageAction::Edit).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImageAction {
    Generate,
    Edit,
}

impl ImageAction {
    fn path(self) -> &'static str {
        match self {
            ImageAction::Generate => "images/generations",
            ImageAction::Edit => "images/edits",
        }
    }

    fn tool_action(self) -> &'static str {
        match self {
            ImageAction::Generate => "generate",
            ImageAction::Edit => "edit",
        }
    }
}

#[derive(Debug)]
struct ImageApiRequest {
    model: String,
    prompt: String,
    images: Vec<String>,
    response_format: String,
    stream: bool,
    raw: Value,
}

#[derive(Debug)]
struct ImageApiError {
    status: StatusCode,
    message: String,
    code: &'static str,
}

impl ImageApiError {
    fn new(status: StatusCode, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status,
            message: message.into(),
            code,
        }
    }
}

impl IntoResponse for ImageApiError {
    fn into_response(self) -> Response {
        image_error_response(self.status, self.message, self.code)
    }
}

struct ImageHistoryRecord {
    history_context: HistoryContext,
    response_id: String,
    model: String,
    status: &'static str,
    start: Instant,
    url: String,
    error: String,
}

async fn handle_images_api(
    state: AppState,
    headers: HeaderMap,
    body: axum::body::Bytes,
    action: ImageAction,
) -> Response {
    let (account, endpoint) = active_account_endpoint(&state).await;
    if endpoint.kind == EndpointKind::CodexOfficial
        || upstream_is_codex_official_endpoint(&endpoint)
    {
        let Some((account, endpoint)) = codex_official_account_endpoint(
            &state,
            DEFAULT_IMAGE_TOOL_MODEL,
            AccountRouteSurface::Global,
        )
        .await
        else {
            return codex_official_pool_unavailable_response();
        };
        return handle_codex_official_images(state, account, endpoint, body, action).await;
    }
    if endpoint.kind.is_responses_like() {
        return handle_responses_images(state, account, endpoint, body, action).await;
    }
    forward_images_api(state, headers, body, account, endpoint, action).await
}

fn upstream_is_codex_official_endpoint(endpoint: &EndpointConfig) -> bool {
    endpoint
        .base_url
        .to_ascii_lowercase()
        .contains("chatgpt.com/backend-api/codex")
}

async fn forward_images_api(
    state: AppState,
    headers: HeaderMap,
    mut body: axum::body::Bytes,
    account: Account,
    endpoint: EndpointConfig,
    action: ImageAction,
) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let mut model = image_request_model(&body).unwrap_or_else(|| DEFAULT_IMAGE_TOOL_MODEL.into());
    let mapped_model = resolve_model(&model, &endpoint.model_map);
    if mapped_model != model {
        body = patch_body_model_field(&body, &mapped_model).unwrap_or(body);
        model = mapped_model;
    }

    let upstream_url =
        Url::parse(&endpoint.base_url).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
    let url = format!("{}{}", join_base(&upstream_url), action.path());
    let start = Instant::now();
    let history_context =
        history_context_for(&account, &endpoint, &format!("/v1/{}", action.path()));
    let response_id = state.sessions.new_id();

    let mut builder = state.client.post(&url).header("Content-Type", content_type);
    if !account.api_key.trim().is_empty() {
        builder = builder.bearer_auth(account.api_key.trim());
    }
    for (k, v) in &endpoint.custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    if let Some(secs) = endpoint.request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let resp = match builder.body(body).send().await {
        Ok(resp) => resp,
        Err(err) => {
            record_image_history(
                &state,
                ImageHistoryRecord {
                    history_context,
                    response_id,
                    model,
                    status: "failed",
                    start,
                    url,
                    error: format!("connection error: {err}"),
                },
            )
            .await;
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("图片上游连接失败: {err}"),
                "upstream_error",
            );
        }
    };
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let bytes = match resp.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("图片上游响应读取失败: {err}"),
                "upstream_read_error",
            )
        }
    };
    record_image_history(
        &state,
        ImageHistoryRecord {
            history_context,
            response_id,
            model,
            status: if status.is_success() {
                "completed"
            } else {
                "failed"
            },
            start,
            url,
            error: if status.is_success() {
                String::new()
            } else {
                format!("HTTP {}", status.as_u16())
            },
        },
    )
    .await;
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .body(axum::body::Body::from(bytes))
        .unwrap()
}

async fn handle_codex_official_images(
    state: AppState,
    mut account: Account,
    endpoint: EndpointConfig,
    body: axum::body::Bytes,
    action: ImageAction,
) -> Response {
    let request = match parse_image_api_request(&body, action) {
        Ok(request) => request,
        Err(err) => return err.into_response(),
    };
    if request.stream {
        tracing::info!(
            "Codex 图片端点收到 stream=true，当前先收集官方 Responses 流并返回 OpenAI Images JSON"
        );
    }
    let image_model = image_model_base(&request.model);
    if image_model != DEFAULT_IMAGE_TOOL_MODEL {
        return image_error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Model {} is not supported on /v1/{}. Use {}.",
                request.model,
                action.path(),
                DEFAULT_IMAGE_TOOL_MODEL
            ),
            "invalid_request_error",
        );
    }
    let body = match build_codex_images_responses_body(&request, action) {
        Ok(body) => body,
        Err(err) => {
            return image_error_response(
                StatusCode::BAD_REQUEST,
                format!("图片请求构造失败: {err}"),
                "invalid_request_error",
            )
        }
    };

    let history_context =
        history_context_for(&account, &endpoint, &format!("/v1/{}", action.path()));
    let response_id = state.sessions.new_id();
    let start = Instant::now();
    let token = match fresh_oauth_access_token(&state, &mut account).await {
        Ok(token) if !token.trim().is_empty() => token,
        Ok(_) => account.api_key.clone(),
        Err(err) => {
            update_runtime_result(
                &state,
                &account.id,
                DEFAULT_IMAGE_TOOL_MODEL,
                StatusCode::UNAUTHORIZED,
                format!("OAuth token refresh failed: {err}"),
                None,
            )
            .await;
            return image_error_response(
                StatusCode::UNAUTHORIZED,
                format!("OAuth token refresh failed: {err}"),
                "oauth_refresh_failed",
            );
        }
    };
    if token.trim().is_empty() {
        return image_error_response(
            StatusCode::UNAUTHORIZED,
            "Codex 官方账号缺少 OAuth access token",
            "missing_oauth_token",
        );
    }

    let upstream = Url::parse(&endpoint.base_url)
        .unwrap_or_else(|_| Url::parse(CODEX_OFFICIAL_BASE_URL).unwrap());
    let url = format!("{}responses", join_base(&upstream));
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Accept-Encoding", "identity")
        .bearer_auth(token)
        .header("Originator", codex_official_originator(&account))
        .header(
            "User-Agent",
            "codex_cli_rs/0.118.0 (Mac OS 26.3.1; arm64) deecodex/3.0",
        )
        .header("Connection", "Keep-Alive");
    if let Some(account_id) = oauth_account_id(&account) {
        builder = builder.header("Chatgpt-Account-Id", account_id);
    }
    for (k, v) in &endpoint.custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    if let Some(secs) = endpoint.request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let upstream_resp = match builder.body(body).send().await {
        Ok(resp) => resp,
        Err(err) => {
            update_runtime_result(
                &state,
                &account.id,
                DEFAULT_IMAGE_TOOL_MODEL,
                StatusCode::BAD_GATEWAY,
                format!("Codex 官方图片上游连接失败: {err}"),
                None,
            )
            .await;
            record_image_history(
                &state,
                ImageHistoryRecord {
                    history_context,
                    response_id,
                    model: DEFAULT_IMAGE_TOOL_MODEL.into(),
                    status: "failed",
                    start,
                    url,
                    error: format!("connection error: {err}"),
                },
            )
            .await;
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("Codex 官方图片上游连接失败: {err}"),
                "upstream_error",
            );
        }
    };
    let status = upstream_resp.status();
    let retry_after = retry_after_secs(upstream_resp.headers());
    let bytes = match upstream_resp.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("Codex 官方图片响应读取失败: {err}"),
                "upstream_read_error",
            )
        }
    };
    let retry_after = codex_usage_retry_after_secs(status, &bytes, retry_after);
    if !status.is_success() {
        let message = codex_error_message(status, &bytes);
        update_runtime_result(
            &state,
            &account.id,
            DEFAULT_IMAGE_TOOL_MODEL,
            status,
            message.clone(),
            retry_after,
        )
        .await;
        record_image_history(
            &state,
            ImageHistoryRecord {
                history_context,
                response_id,
                model: DEFAULT_IMAGE_TOOL_MODEL.into(),
                status: "failed",
                start,
                url,
                error: message,
            },
        )
        .await;
        return Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(bytes))
            .unwrap();
    }

    let images = match images_response_from_codex_sse(&bytes, &request.response_format) {
        Ok(value) => value,
        Err(err) => {
            update_runtime_result(
                &state,
                &account.id,
                DEFAULT_IMAGE_TOOL_MODEL,
                StatusCode::BAD_GATEWAY,
                err.to_string(),
                None,
            )
            .await;
            record_image_history(
                &state,
                ImageHistoryRecord {
                    history_context,
                    response_id,
                    model: DEFAULT_IMAGE_TOOL_MODEL.into(),
                    status: "failed",
                    start,
                    url,
                    error: err.to_string(),
                },
            )
            .await;
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("Codex 官方图片响应解析失败: {err}"),
                "invalid_response",
            );
        }
    };

    update_runtime_result(
        &state,
        &account.id,
        DEFAULT_IMAGE_TOOL_MODEL,
        StatusCode::OK,
        String::new(),
        None,
    )
    .await;
    record_image_history(
        &state,
        ImageHistoryRecord {
            history_context,
            response_id,
            model: DEFAULT_IMAGE_TOOL_MODEL.into(),
            status: "completed",
            start,
            url,
            error: String::new(),
        },
    )
    .await;
    Json(images).into_response()
}

async fn handle_responses_images(
    state: AppState,
    account: Account,
    endpoint: EndpointConfig,
    body: axum::body::Bytes,
    action: ImageAction,
) -> Response {
    let request = match parse_image_api_request(&body, action) {
        Ok(request) => request,
        Err(err) => return err.into_response(),
    };
    if image_model_base(&request.model) != DEFAULT_IMAGE_TOOL_MODEL {
        return image_error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Model {} is not supported on /v1/{}. Use {}.",
                request.model,
                action.path(),
                DEFAULT_IMAGE_TOOL_MODEL
            ),
            "invalid_request_error",
        );
    }
    let body = match build_codex_images_responses_body(&request, action) {
        Ok(body) => body,
        Err(err) => {
            return image_error_response(
                StatusCode::BAD_REQUEST,
                format!("图片请求构造失败: {err}"),
                "invalid_request_error",
            )
        }
    };

    let history_context =
        history_context_for(&account, &endpoint, &format!("/v1/{}", action.path()));
    let response_id = state.sessions.new_id();
    let start = Instant::now();
    let upstream =
        Url::parse(&endpoint.base_url).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
    let path = if endpoint.path.trim().is_empty() {
        "responses"
    } else {
        endpoint.effective_path()
    };
    let url = format!("{}{}", join_base(&upstream), path);
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Accept-Encoding", "identity");
    if !account.api_key.trim().is_empty() {
        builder = builder.bearer_auth(account.api_key.trim());
    }
    for (k, v) in &endpoint.custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    if let Some(secs) = endpoint.request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let upstream_resp = match builder.body(body).send().await {
        Ok(resp) => resp,
        Err(err) => {
            record_image_history(
                &state,
                ImageHistoryRecord {
                    history_context,
                    response_id,
                    model: DEFAULT_IMAGE_TOOL_MODEL.into(),
                    status: "failed",
                    start,
                    url,
                    error: format!("connection error: {err}"),
                },
            )
            .await;
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("图片上游连接失败: {err}"),
                "upstream_error",
            );
        }
    };
    let status = upstream_resp.status();
    let bytes = match upstream_resp.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("图片上游响应读取失败: {err}"),
                "upstream_read_error",
            )
        }
    };
    if !status.is_success() {
        record_image_history(
            &state,
            ImageHistoryRecord {
                history_context,
                response_id,
                model: DEFAULT_IMAGE_TOOL_MODEL.into(),
                status: "failed",
                start,
                url,
                error: format!("HTTP {}", status.as_u16()),
            },
        )
        .await;
        return Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(axum::body::Body::from(bytes))
            .unwrap();
    }
    let images = match images_response_from_codex_sse(&bytes, &request.response_format) {
        Ok(value) => value,
        Err(err) => {
            record_image_history(
                &state,
                ImageHistoryRecord {
                    history_context,
                    response_id,
                    model: DEFAULT_IMAGE_TOOL_MODEL.into(),
                    status: "failed",
                    start,
                    url,
                    error: err.to_string(),
                },
            )
            .await;
            return image_error_response(
                StatusCode::BAD_GATEWAY,
                format!("图片响应解析失败: {err}"),
                "invalid_response",
            );
        }
    };
    record_image_history(
        &state,
        ImageHistoryRecord {
            history_context,
            response_id,
            model: DEFAULT_IMAGE_TOOL_MODEL.into(),
            status: "completed",
            start,
            url,
            error: String::new(),
        },
    )
    .await;
    Json(images).into_response()
}

fn image_request_model(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn parse_image_api_request(
    body: &[u8],
    action: ImageAction,
) -> std::result::Result<ImageApiRequest, ImageApiError> {
    let value: Value = serde_json::from_slice(body).map_err(|err| {
        ImageApiError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid request: body must be valid JSON: {err}"),
            "invalid_request_error",
        )
    })?;
    let prompt = value
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            ImageApiError::new(
                StatusCode::BAD_REQUEST,
                "Invalid request: prompt is required",
                "invalid_request_error",
            )
        })?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_IMAGE_TOOL_MODEL)
        .to_string();
    let images = collect_image_urls(&value);
    if action == ImageAction::Edit && images.is_empty() {
        return Err(ImageApiError::new(
            StatusCode::BAD_REQUEST,
            "Invalid request: images[].image_url is required",
            "invalid_request_error",
        ));
    }
    let response_format = value
        .get("response_format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("b64_json")
        .to_string();
    let stream = value
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ImageApiRequest {
        model,
        prompt,
        images,
        response_format,
        stream,
        raw: value,
    })
}

fn collect_image_urls(value: &Value) -> Vec<String> {
    let mut images = Vec::new();
    if let Some(array) = value.get("images").and_then(Value::as_array) {
        for item in array {
            if let Some(url) = image_url_from_value(item) {
                images.push(url);
            }
        }
    }
    if let Some(image) = value.get("image").and_then(image_url_from_value) {
        images.push(image);
    }
    images
}

fn image_url_from_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .or_else(|| value.get("image_url").and_then(Value::as_str))
        .or_else(|| {
            value
                .get("image_url")
                .and_then(Value::as_object)
                .and_then(|obj| obj.get("url"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn image_model_base(model: &str) -> String {
    model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .trim()
        .to_ascii_lowercase()
}

fn build_codex_images_responses_body(
    request: &ImageApiRequest,
    action: ImageAction,
) -> serde_json::Result<axum::body::Bytes> {
    let main_model = if let Some((prefix, _)) = request.model.rsplit_once('/') {
        if prefix.trim().is_empty() {
            DEFAULT_IMAGE_MAIN_MODEL.to_string()
        } else {
            format!("{}/{}", prefix.trim(), DEFAULT_IMAGE_MAIN_MODEL)
        }
    } else {
        DEFAULT_IMAGE_MAIN_MODEL.to_string()
    };
    let mut content = vec![json!({"type": "input_text", "text": request.prompt})];
    for image in &request.images {
        content.push(json!({"type": "input_image", "image_url": image}));
    }
    let mut tool = json!({
        "type": "image_generation",
        "action": action.tool_action(),
        "model": request.model,
    });
    for field in [
        "size",
        "quality",
        "background",
        "output_format",
        "input_fidelity",
        "moderation",
    ] {
        if let Some(value) = request
            .raw
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            tool[field] = json!(value);
        }
    }
    for field in ["output_compression", "partial_images"] {
        if let Some(value) = request.raw.get(field).and_then(Value::as_i64) {
            tool[field] = json!(value);
        }
    }
    if let Some(mask) = request
        .raw
        .get("mask")
        .and_then(image_url_from_value)
        .filter(|value| !value.trim().is_empty())
    {
        tool["input_image_mask"] = json!({"image_url": mask});
    }
    let body = json!({
        "instructions": "",
        "stream": true,
        "reasoning": {"effort": "medium", "summary": "auto"},
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
        "model": main_model,
        "store": false,
        "tool_choice": {"type": "image_generation"},
        "input": [{
            "type": "message",
            "role": "user",
            "content": content,
        }],
        "tools": [tool],
    });
    serde_json::to_vec(&body).map(axum::body::Bytes::from)
}

fn images_response_from_codex_sse(body: &[u8], response_format: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        if value.get("type").and_then(Value::as_str) == Some("response.completed") {
            return images_response_from_completed_event(&value, response_format);
        }
        if value.get("response").is_some() {
            return images_response_from_completed_event(
                &json!({
                    "type": "response.completed",
                    "response": value
                }),
                response_format,
            );
        }
    }
    let mut image_items = Vec::new();
    let mut created = crate::accounts::now_secs() as i64;
    let mut usage: Option<Value> = None;
    for line in String::from_utf8_lossy(body).lines() {
        let trimmed = line.trim();
        let Some(payload) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(payload)?;
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_item.done" => {
                if let Some(item) = value.get("item") {
                    if item.get("type").and_then(Value::as_str) == Some("image_generation_call") {
                        image_items.push(item.clone());
                    }
                }
            }
            "response.completed" => {
                if let Some(response) = value.get("response") {
                    created = response
                        .get("created_at")
                        .and_then(Value::as_i64)
                        .unwrap_or(created);
                    usage = response
                        .get("tool_usage")
                        .and_then(|usage| usage.get("image_gen"))
                        .filter(|usage| usage.is_object())
                        .cloned();
                }
                if let Ok(out) = images_response_from_completed_event(&value, response_format) {
                    return Ok(out);
                }
                if !image_items.is_empty() {
                    return images_response_from_image_items(
                        &image_items,
                        created,
                        usage.as_ref(),
                        response_format,
                    );
                }
            }
            _ => {}
        }
    }
    if !image_items.is_empty() {
        return images_response_from_image_items(
            &image_items,
            created,
            usage.as_ref(),
            response_format,
        );
    }
    bail!("stream disconnected before response.completed image output");
}

fn images_response_from_completed_event(event: &Value, response_format: &str) -> Result<Value> {
    let response = event.get("response").unwrap_or(event);
    let created = response
        .get("created_at")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| crate::accounts::now_secs() as i64);
    let output = response
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let usage = response
        .get("tool_usage")
        .and_then(|usage| usage.get("image_gen"))
        .filter(|usage| usage.is_object());
    images_response_from_image_items(&output, created, usage, response_format)
}

fn images_response_from_image_items(
    items: &[Value],
    created: i64,
    usage: Option<&Value>,
    response_format: &str,
) -> Result<Value> {
    let mut data = Vec::new();
    let mut first_meta: Option<&Value> = None;
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
            continue;
        }
        let Some(result) = item
            .get("result")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if first_meta.is_none() {
            first_meta = Some(item);
        }
        let mut entry = serde_json::Map::new();
        if response_format.trim().eq_ignore_ascii_case("url") {
            let mime = image_mime_type_from_output_format(
                item.get("output_format")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            entry.insert("url".into(), json!(format!("data:{mime};base64,{result}")));
        } else {
            entry.insert("b64_json".into(), json!(result));
        }
        if let Some(revised) = item
            .get("revised_prompt")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            entry.insert("revised_prompt".into(), json!(revised));
        }
        data.push(Value::Object(entry));
    }
    if data.is_empty() {
        bail!("upstream did not return image_generation_call result");
    }
    let mut out = serde_json::Map::new();
    out.insert("created".into(), json!(created));
    out.insert("data".into(), Value::Array(data));
    if let Some(meta) = first_meta {
        for field in ["background", "output_format", "quality", "size"] {
            if let Some(value) = meta
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                out.insert(field.into(), json!(value));
            }
        }
    }
    if let Some(usage) = usage {
        out.insert("usage".into(), usage.clone());
    }
    Ok(Value::Object(out))
}

fn image_mime_type_from_output_format(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

async fn record_image_history(state: &AppState, record: ImageHistoryRecord) {
    let ImageHistoryRecord {
        history_context,
        response_id,
        model,
        status,
        start,
        url,
        error,
    } = record;
    state
        .request_history
        .record(record_from_context(
            &history_context,
            response_id,
            model,
            status.into(),
            0,
            0,
            start.elapsed().as_millis() as u64,
            url,
            error,
            false,
        ))
        .await;
}

fn image_error_response(status: StatusCode, message: impl Into<String>, code: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message.into(),
                "type": "invalid_request_error",
                "code": code,
            }
        })),
    )
        .into_response()
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

// ── Codex 线程聚合 API（复用 web.rs 中的 handler 逻辑）──

async fn handle_list_threads_api(State(_state): State<AppState>) -> Response {
    match crate::codex_threads::list_all() {
        Ok(threads) => Json(serde_json::json!({ "ok": true, "threads": threads })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "message": format!("读取线程失败: {e}") })),
        )
            .into_response(),
    }
}

async fn handle_threads_status_api(State(state): State<AppState>) -> Response {
    match crate::codex_threads::status(&state.data_dir) {
        Ok(s) => Json(serde_json::json!({ "ok": true, "status": s })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "message": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn handle_list_unified_threads_api(State(state): State<AppState>) -> Response {
    let list = crate::client_threads::list_client_threads(&state.data_dir);
    Json(serde_json::json!({ "ok": true, "threads": list.threads, "sources": list.sources, "total": list.total }))
        .into_response()
}

async fn handle_unified_thread_sources_api(State(state): State<AppState>) -> Response {
    let sources = crate::client_threads::get_thread_sources(&state.data_dir);
    Json(serde_json::json!({ "ok": true, "sources": sources })).into_response()
}

#[derive(Debug, Deserialize)]
struct UnifiedThreadContentQuery {
    client_kind: String,
    native_id: String,
    #[serde(default, alias = "threadKey")]
    thread_key: Option<String>,
}

async fn handle_unified_thread_content_api(
    Query(query): Query<UnifiedThreadContentQuery>,
) -> Response {
    let Some(kind) = crate::client_threads::parse_client_kind(&query.client_kind) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "ok": false, "message": "未知客户端类型" })),
        )
            .into_response();
    };
    match crate::client_threads::get_client_thread_content(
        kind,
        &query.native_id,
        query.thread_key.as_deref(),
    ) {
        Ok(content) => Json(serde_json::json!({ "ok": true, "content": content })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "message": format!("{e}") })),
        )
            .into_response(),
    }
}

async fn handle_migrate_threads_api(State(state): State<AppState>) -> Response {
    match crate::codex_threads::migrate(&state.data_dir) {
        Ok(diff) => Json(serde_json::json!({
            "ok": true,
            "diff": diff,
            "message": format!("已迁移 {} 条线程到 deecodex", diff.changed_count),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "message": format!("迁移失败: {e}") })),
        )
            .into_response(),
    }
}

async fn handle_restore_threads_api(State(state): State<AppState>) -> Response {
    match crate::codex_threads::restore(&state.data_dir) {
        Ok(diff) => Json(serde_json::json!({
            "ok": true,
            "diff": diff,
            "message": format!("已还原 {} 条线程的 model_provider", diff.changed_count),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "message": format!("还原失败: {e}") })),
        )
            .into_response(),
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
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    handle_responses_for_route(state, body, infer_account_route_surface(&headers)).await
}

async fn handle_responses_codex_cli(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    handle_responses_for_route(state, body, AccountRouteSurface::CodexCli).await
}

async fn handle_responses_codex_desktop(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    handle_responses_for_route(state, body, AccountRouteSurface::CodexDesktop).await
}

async fn handle_responses_dex_assistant(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    handle_responses_for_route(state, body, AccountRouteSurface::DexAssistant).await
}

async fn handle_responses_for_route(
    state: AppState,
    body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
) -> Response {
    let _start = std::time::Instant::now();
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
    let model = req.model.clone();
    let (_account, endpoint) = active_account_endpoint_for_route(&state, route_surface).await;
    let mode = match endpoint.kind {
        EndpointKind::OpenAiChat | EndpointKind::CustomChat => "translate",
        EndpointKind::OpenAiResponses | EndpointKind::CustomResponses => "bypass",
        EndpointKind::AnthropicMessages => "anthropic",
        EndpointKind::CodexOfficial => "codex_official",
    };
    tracing::info!(
        "⇢ {mode} {model} → {} ({})",
        endpoint.base_url,
        endpoint.kind.label()
    );
    if let Some(response) = validate_response_include(req.include.as_deref()) {
        return response;
    }

    // 直连模式：仅做模型映射，其他全部透传
    if endpoint.kind.is_responses_like() {
        return handle_responses_bypass(state, req, body, route_surface).await;
    }
    if endpoint.kind == EndpointKind::CodexOfficial {
        return handle_codex_official(state, req, body, route_surface).await;
    }

    // ── 以下仅翻译/代理模式生效 ──
    if let Err(err) = state.prompts.apply_to_request(&mut req) {
        return err.into_response();
    }
    if let Some(response) = validate_tool_policy(&req.tools, &*state.tool_policy.read().await) {
        return response;
    }
    if let Some(ref limiter) = state.rate_limiter {
        let key = "rl_default";
        if !limiter.check(key) {
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
        body.clone(),
        local_file_search_output_items,
        local_file_search_input_items,
        route_surface,
    )
    .await;
    tracing::info!(
        "⇠ translate {} done in {}ms",
        model,
        _start.elapsed().as_millis()
    );
    let status = response.status().as_u16().to_string();
    state
        .metrics
        .http_requests_total
        .with_label_values(&["POST", &status])
        .inc();
    response
}

async fn handle_client_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    forward_client_proxy_request(
        state,
        headers,
        body,
        "/v1/chat/completions",
        "openai_chat",
        "chat/completions".into(),
    )
    .await
}

async fn handle_client_anthropic_messages_v1(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let upstream_path = {
        let token = proxy_token_from_headers(&headers);
        if let Some(token) = token.as_deref() {
            if let Some(account) = account_for_proxy_token(&state, token).await {
                anthropic_messages_path(&account.upstream).to_string()
            } else {
                "v1/messages".into()
            }
        } else {
            "v1/messages".into()
        }
    };
    forward_client_proxy_request(
        state,
        headers,
        body,
        "/v1/messages",
        "anthropic_messages",
        upstream_path,
    )
    .await
}

async fn handle_client_anthropic_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let upstream_path = {
        let token = proxy_token_from_headers(&headers);
        if let Some(token) = token.as_deref() {
            if let Some(account) = account_for_proxy_token(&state, token).await {
                anthropic_messages_path(&account.upstream).to_string()
            } else {
                "v1/messages".into()
            }
        } else {
            "v1/messages".into()
        }
    };
    forward_client_proxy_request(
        state,
        headers,
        body,
        "/messages",
        "anthropic_messages",
        upstream_path,
    )
    .await
}

/// 从 SSE 字节流中提取 usage 和上游回显信息。
#[cfg(test)]
fn extract_bypass_response_observations(bytes: &[u8]) -> (u32, u32, bool, Option<String>) {
    let text = String::from_utf8_lossy(bytes);
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut cache_hit = false;
    let mut service_tier: Option<String> = None;
    for line in text.lines() {
        let data = line.strip_prefix("data: ").unwrap_or(line);
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
            let event_type = v.get("type").and_then(Value::as_str).unwrap_or_default();
            let event_service_tier = v
                .get("response")
                .and_then(|r| r.get("service_tier"))
                .or_else(|| v.get("service_tier"))
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(tier) = event_service_tier {
                if event_type == "response.completed" || service_tier.is_none() {
                    service_tier = Some(tier);
                }
            }
            // response.completed 事件
            if let Some(resp) = v.get("response").and_then(|r| r.get("usage")) {
                input_tokens = resp
                    .get("input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(input_tokens as u64) as u32;
                output_tokens = resp
                    .get("output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(output_tokens as u64) as u32;
                if !cache_hit {
                    if let Some(obj) = resp.as_object() {
                        tracing::info!("bypass usage: {:?}", obj);
                        cache_hit = is_cache_hit(obj);
                    }
                }
            }
            // 最后一条 data chunk 直接带 usage
            if let Some(usage) = v.get("usage").and_then(|u| u.as_object()) {
                if !usage.is_empty() {
                    input_tokens = usage
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(input_tokens as u64) as u32;
                    output_tokens = usage
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(output_tokens as u64) as u32;
                    if !cache_hit {
                        tracing::info!("bypass usage: {:?}", usage);
                        cache_hit = is_cache_hit(usage);
                    }
                }
            }
        }
    }
    (input_tokens, output_tokens, cache_hit, service_tier)
}

/// 判断 usage 对象中是否包含缓存命中标记
fn is_cache_hit(usage: &serde_json::Map<String, serde_json::Value>) -> bool {
    let prompt_cache_hit = usage
        .get("prompt_cache_hit_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let prompt_cached = usage
        .get("prompt_tokens_details")
        .and_then(|v| v.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let input_cached = usage
        .get("input_tokens_details")
        .and_then(|v| v.get("cached_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    // 通过 input_tokens_details.cached_tokens 判断时，要求缓存占比 > 50%，
    // 避免系统提示词的少量缓存被误判为命中（新会话首请求也有 ~40% 缓存）。
    let input_total = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(input_cached);
    let significant_hit = input_cached > input_total / 2;
    let hit = prompt_cache_hit > 0 || prompt_cached > 0 || significant_hit;
    if hit {
        tracing::info!(
            "bypass cache_hit: prompt_cache_hit={} prompt_cached={} input_cached={}/{}",
            prompt_cache_hit,
            prompt_cached,
            input_cached,
            input_total
        );
    }
    hit
}

/// 修改原始 JSON body 中的某个字段值
fn patch_body_model_field(
    body: &axum::body::Bytes,
    model: &str,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut v: serde_json::Value = serde_json::from_slice(body)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("model".to_string(), serde_json::json!(model));
    }
    serde_json::to_vec(&v).map(axum::body::Bytes::from)
}

fn patch_body_string_field(
    body: &axum::body::Bytes,
    field: &str,
    value: &str,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut v: serde_json::Value = serde_json::from_slice(body)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert(field.to_string(), serde_json::json!(value));
    }
    serde_json::to_vec(&v).map(axum::body::Bytes::from)
}

fn sanitize_client_proxy_body(
    endpoint_kind: &str,
    account: &Account,
    body: axum::body::Bytes,
) -> axum::body::Bytes {
    if endpoint_kind != "anthropic_messages" {
        return body;
    }
    let strip_builtin_cch = claude_cch_filter_enabled(account);
    let custom_rules = claude_custom_filter_rules(account);
    strip_claude_code_anthropic_attribution(&body, strip_builtin_cch, &custom_rules).unwrap_or(body)
}

fn strip_claude_code_anthropic_attribution(
    body: &axum::body::Bytes,
    strip_builtin_cch: bool,
    custom_rules: &[String],
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut value: Value = serde_json::from_slice(body)?;
    let mut changed = false;

    if let Some(system) = value.get_mut("system") {
        match system {
            Value::String(text) => {
                changed |=
                    strip_claude_code_attribution_text(text, strip_builtin_cch, custom_rules);
            }
            Value::Array(parts) => {
                for part in parts {
                    if let Some(Value::String(text)) = part.get_mut("text") {
                        if strip_claude_code_attribution_text(text, strip_builtin_cch, custom_rules)
                        {
                            changed = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if !changed {
        return Ok(body.clone());
    }

    tracing::info!("已移除 Claude Code Anthropic attribution header");
    serde_json::to_vec(&value).map(axum::body::Bytes::from)
}

fn strip_claude_code_attribution_text(
    text: &mut String,
    strip_builtin_cch: bool,
    custom_rules: &[String],
) -> bool {
    let original = text.clone();
    let kept: Vec<&str> = original
        .lines()
        .filter(|line| !is_claude_code_filtered_line(line, strip_builtin_cch, custom_rules))
        .collect();
    let stripped = collapse_blank_lines(kept.join("\n").trim_matches('\n'));
    if stripped == original {
        return false;
    }
    *text = stripped;
    true
}

fn is_claude_code_filtered_line(
    line: &str,
    strip_builtin_cch: bool,
    custom_rules: &[String],
) -> bool {
    (strip_builtin_cch && line.contains("x-anthropic-billing-header:") && line.contains("cch="))
        || custom_rules
            .iter()
            .any(|rule| !rule.is_empty() && line.contains(rule))
}

fn claude_cch_filter_enabled(account: &Account) -> bool {
    account
        .client_options
        .get("claude_cch_filter_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn claude_custom_filter_rules(account: &Account) -> Vec<String> {
    if !account
        .client_options
        .get("claude_custom_filter_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Vec::new();
    }
    account
        .client_options
        .get("claude_custom_filter_rules")
        .and_then(Value::as_array)
        .map(|rules| {
            rules
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|rule| !rule.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = Vec::new();
    let mut previous_blank = false;
    for line in text.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && previous_blank {
            continue;
        }
        out.push(line);
        previous_blank = is_blank;
    }
    out.join("\n").trim().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FastServiceTierStatus {
    AlreadyPresent,
    Injected,
    Skipped,
}

fn normalize_fast_service_tier(tier: &str) -> Option<&str> {
    match tier.trim() {
        "" => None,
        // 兼容旧 GUI 文案/配置：用户看到的是 Fast，但 OpenAI/Codex 上游接受的是 priority。
        "fast" => Some("priority"),
        value => Some(value),
    }
}

fn apply_endpoint_fast_service_tier(
    req: &mut ResponsesRequest,
    endpoint: &EndpointConfig,
) -> FastServiceTierStatus {
    if req.service_tier.is_some() {
        return FastServiceTierStatus::AlreadyPresent;
    }
    if endpoint.kind != EndpointKind::OpenAiResponses || !endpoint.fast_mode_enabled {
        return FastServiceTierStatus::Skipped;
    }

    if let Some(tier) = normalize_fast_service_tier(&endpoint.fast_service_tier) {
        req.service_tier = Some(tier.to_string());
        return FastServiceTierStatus::Injected;
    }

    FastServiceTierStatus::Skipped
}

fn response_input_has_new_image(input: &ResponsesInput) -> bool {
    match input {
        ResponsesInput::Text(text) => text.contains("data:image/"),
        ResponsesInput::Messages(items) => items.iter().any(response_item_has_image),
    }
}

fn response_item_has_image(item: &Value) -> bool {
    if item.get("image_url").is_some() || item.get("screenshot").is_some() {
        return true;
    }
    match item.get("content") {
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            matches!(
                part.get("type").and_then(Value::as_str),
                Some("image_url" | "input_image")
            ) || part
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains("data:image/"))
        }),
        Some(Value::String(text)) => text.contains("data:image/"),
        _ => false,
    }
}

fn strip_images_from_responses_body(
    body: &axum::body::Bytes,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut value: Value = serde_json::from_slice(body)?;
    strip_images_from_value(&mut value);
    serde_json::to_vec(&value).map(axum::body::Bytes::from)
}

fn add_caption_to_responses_body(
    body: &axum::body::Bytes,
    caption: &str,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut value: Value = serde_json::from_slice(body)?;
    strip_images_from_value(&mut value);
    let caption_text =
        format!("图片内容由视觉模型识别如下，请结合上下文继续完成原始任务：\n{caption}");
    if let Some(obj) = value.as_object_mut() {
        match obj.get_mut("input") {
            Some(Value::String(text)) => {
                text.push('\n');
                text.push_str(&caption_text);
            }
            Some(Value::Array(items)) => items.push(json!({
                "role": "user",
                "content": [{"type": "input_text", "text": caption_text}]
            })),
            _ => {
                obj.insert("input".into(), Value::String(caption_text));
            }
        }
    }
    serde_json::to_vec(&value).map(axum::body::Bytes::from)
}

fn strip_images_from_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("image_url");
            map.remove("screenshot");
            if let Some(Value::Array(parts)) = map.get_mut("content") {
                parts.retain(|part| {
                    !matches!(
                        part.get("type").and_then(Value::as_str),
                        Some("image_url" | "input_image")
                    )
                });
            }
            for value in map.values_mut() {
                strip_images_from_value(value);
            }
        }
        Value::Array(items) => {
            for value in items {
                strip_images_from_value(value);
            }
        }
        Value::String(text) => {
            if let Some(pos) = text.find("data:image/") {
                *text = text[..pos].trim().to_string();
            }
        }
        _ => {}
    }
}

async fn handle_responses_vlm_final_answer(
    state: AppState,
    req: &ResponsesRequest,
    endpoint: &EndpointConfig,
    model_map: &ModelMap,
    store_response: bool,
    request_input_items: Vec<Value>,
    response_extra: Value,
) -> Response {
    let vu = match parse_glue_vision_base_url(endpoint) {
        Ok(url) => url,
        Err(response) => return *response,
    };
    if let Err(response) = ensure_minimax_glue_adapter(endpoint) {
        return *response;
    }

    let translated = translate::to_chat_request(
        req,
        Vec::new(),
        &state.sessions,
        model_map,
        state.chinese_thinking,
    );
    let url = format!("{}{}", join_base(&vu), endpoint.vision.path.as_str());
    let vlm_body = build_minimax_vlm_body(&translated.chat);
    handle_minimax_vlm(VlmArgs {
        state,
        url,
        api_key: endpoint.vision.api_key.clone(),
        vlm_body,
        model: endpoint.vision.model.clone(),
        stream_response: req.stream,
        store_response,
        request_input_items,
        response_extra,
    })
    .await
}

async fn caption_then_patch_responses_body(
    state: &AppState,
    req: &ResponsesRequest,
    body: &axum::body::Bytes,
    endpoint: &EndpointConfig,
    model_map: &ModelMap,
) -> Result<axum::body::Bytes, Response> {
    let vu = parse_glue_vision_base_url(endpoint).map_err(|response| *response)?;
    ensure_minimax_glue_adapter(endpoint).map_err(|response| *response)?;

    let translated = translate::to_chat_request(
        req,
        Vec::new(),
        &state.sessions,
        model_map,
        state.chinese_thinking,
    );
    let url = format!("{}{}", join_base(&vu), endpoint.vision.path.as_str());
    let vlm_body = build_minimax_vlm_body(&translated.chat);
    let caption =
        request_minimax_vlm_text(state, &url, &endpoint.vision.api_key, &vlm_body).await?;
    Ok(add_caption_to_responses_body(body, &caption).unwrap_or_else(|_| body.clone()))
}

fn parse_glue_vision_base_url(endpoint: &EndpointConfig) -> Result<Url, Box<Response>> {
    if endpoint.vision.base_url.trim().is_empty() {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": "当前端点启用了胶水多模态，但未配置视觉上游 URL。",
                        "type": "invalid_request_error",
                        "code": "vision_glue_not_configured"
                    }
                })),
            )
                .into_response(),
        ));
    }

    Url::parse(endpoint.vision.base_url.trim()).map_err(|_| {
        Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "message": "当前端点的视觉上游 URL 无效。",
                        "type": "invalid_request_error",
                        "code": "vision_glue_invalid_url"
                    }
                })),
            )
                .into_response(),
        )
    })
}

fn ensure_minimax_glue_adapter(endpoint: &EndpointConfig) -> Result<(), Box<Response>> {
    if endpoint.vision.adapter_id != "minimax_coding_plan_vlm" {
        return Err(Box::new(
            (
                StatusCode::NOT_IMPLEMENTED,
                Json(json!({
                    "error": {
                        "message": "当前版本仅实现 MiniMax coding_plan/vlm 胶水视觉适配器。",
                        "type": "unsupported_vision_adapter",
                        "code": "vision_adapter_reserved"
                    }
                })),
            )
                .into_response(),
        ));
    }
    Ok(())
}

async fn handle_responses_bypass(
    state: AppState,
    mut req: ResponsesRequest,
    body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
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

    // Responses 直连用于转发 Codex 原生请求，默认保留请求体里的模型名。
    // Chat 兼容端点才需要 Responses -> Chat 的模型映射。
    let (account, endpoint) = active_account_endpoint_for_route(&state, route_surface).await;
    let history_context = history_context_for(&account, &endpoint, route_surface.responses_path());
    let fast_service_tier_status = apply_endpoint_fast_service_tier(&mut req, &endpoint);
    match fast_service_tier_status {
        FastServiceTierStatus::Injected => tracing::info!(
            model = %req.model,
            endpoint_kind = ?endpoint.kind,
            service_tier = %req.service_tier.as_deref().unwrap_or(""),
            "GPT Fast service_tier 已注入"
        ),
        FastServiceTierStatus::AlreadyPresent => tracing::info!(
            model = %req.model,
            endpoint_kind = ?endpoint.kind,
            service_tier = %req.service_tier.as_deref().unwrap_or(""),
            "请求已携带 service_tier，保持原值转发"
        ),
        FastServiceTierStatus::Skipped => {}
    }
    let model_map = ModelMap::new();
    let model = req.model.clone();
    let mut body = body;
    if let Some(service_tier) = req.service_tier.as_deref() {
        body = patch_body_string_field(&body, "service_tier", service_tier).unwrap_or(body);
    }

    let conversation_id = conversation_id_from_request(&req);
    let store_response = req.store.unwrap_or(true);

    if req.background == Some(true) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": {
                    "message": "background requests are not supported in bypass mode",
                    "type": "invalid_request_error",
                    "code": "invalid_request_error"
                }
            })),
        )
            .into_response();
    }

    let has_new_image = response_input_has_new_image(&req.input);
    let vision_mode = endpoint.model_vision_mode(&model);
    if has_new_image && vision_mode == VisionMode::Off {
        match endpoint.vision.unsupported_image_policy {
            UnsupportedImagePolicy::Reject => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "message": "当前 Responses 直连端点未启用视觉能力，无法处理图片输入。请在账号端点中选择原生多模态，或改为剥离图片后继续。",
                            "type": "unsupported_image",
                            "code": "vision_disabled"
                        }
                    })),
                )
                    .into_response();
            }
            UnsupportedImagePolicy::StripWithWarning => {
                warn!("Responses 直连端点未启用视觉能力，已按配置剥离图片后继续请求");
                body = strip_images_from_responses_body(&body).unwrap_or(body);
            }
        }
    } else if has_new_image && vision_mode == VisionMode::Glue {
        match endpoint.vision.glue_strategy {
            GlueVisionStrategy::FinalAnswer => {
                return handle_responses_vlm_final_answer(
                    state,
                    &req,
                    &endpoint,
                    &model_map,
                    store_response,
                    response_input_items(&req),
                    response_extra_fields(&req, conversation_id.as_deref()),
                )
                .await;
            }
            GlueVisionStrategy::CaptionThenMain => {
                body = match caption_then_patch_responses_body(
                    &state, &req, &body, &endpoint, &model_map,
                )
                .await
                {
                    Ok(body) => body,
                    Err(resp) => return resp,
                };
            }
        }
    }

    let api_key = account.api_key.clone();
    let custom_headers = endpoint.custom_headers.clone();
    let timeout_secs = endpoint.request_timeout_secs;
    let max_retries = endpoint.max_retries;

    let upstream_url =
        Url::parse(&endpoint.base_url).unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
    let endpoint_path = endpoint.effective_path().to_string();

    // 存储 conversation items
    if store_response {
        if let Some(id) = conversation_id.as_deref() {
            let mut items = state.sessions.get_conversation_items(id);
            items.extend(response_input_items(&req));
            state
                .sessions
                .save_conversation_items(id.to_string(), items);
        }
    }

    tracing::info!(
        model = %model,
        upstream = %upstream_url,
        endpoint_path = %endpoint_path,
        stream = req.stream,
        body_bytes = body.len(),
        service_tier = %req.service_tier.as_deref().unwrap_or(""),
        has_image = has_new_image,
        vision_mode = ?vision_mode,
        "⇢ bypass 请求摘要"
    );

    let response_id = state.sessions.new_id();
    let bypass = BypassArgs {
        state: state.clone(),
        body,
        upstream_url,
        endpoint_path,
        api_key,
        custom_headers,
        timeout_secs,
        max_retries,
        response_id,
        store_response,
        model,
        requested_service_tier: req.service_tier.clone(),
        history_context,
    };

    let start = std::time::Instant::now();
    let result = if req.stream {
        bypass_stream_forward(bypass).await
    } else {
        bypass_send_request(bypass).await
    };
    tracing::info!("⇠ bypass done in {}ms", start.elapsed().as_millis());
    result
}

fn normalize_codex_official_body(
    body: &[u8],
    mapped_model: &str,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut value: Value = serde_json::from_slice(body)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("model".into(), json!(mapped_model));
        if !matches!(obj.get("instructions"), Some(Value::String(_))) {
            obj.insert("instructions".into(), Value::String(String::new()));
        }
        match obj.get_mut("input") {
            Some(input) => normalize_codex_official_input(input),
            None => {
                obj.insert("input".into(), Value::Array(Vec::new()));
            }
        }
        obj.insert("store".into(), Value::Bool(false));
        obj.entry("parallel_tool_calls")
            .or_insert_with(|| Value::Bool(true));
        obj.entry("include")
            .or_insert_with(|| json!(["reasoning.encrypted_content"]));

        for key in [
            "max_output_tokens",
            "max_completion_tokens",
            "temperature",
            "top_p",
            "truncation",
            "context_management",
            "previous_response_id",
            "prompt_cache_retention",
            "safety_identifier",
            "stream_options",
            "user",
        ] {
            obj.remove(key);
        }
        let remove_service_tier = obj
            .get("service_tier")
            .and_then(Value::as_str)
            .is_some_and(|tier| tier != "priority");
        if remove_service_tier {
            obj.remove("service_tier");
        }
    }
    serde_json::to_vec(&value).map(axum::body::Bytes::from)
}

fn normalize_codex_official_input(input: &mut Value) {
    if let Some(text) = input.as_str() {
        *input = json!([{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": text}]
        }]);
        return;
    }
    let Some(items) = input.as_array_mut() else {
        return;
    };
    for item in items {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let mut role = obj
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if role == "system" {
            obj.insert("role".into(), Value::String("developer".into()));
            role = "developer".into();
        }
        if let Some(content) = obj.get_mut("content") {
            if let Some(text) = content.as_str() {
                let part_type = if role == "assistant" {
                    "output_text"
                } else {
                    "input_text"
                };
                *content = json!([{"type": part_type, "text": text}]);
            }
        }
    }
}

async fn handle_codex_official(
    state: AppState,
    mut req: ResponsesRequest,
    mut body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
) -> Response {
    let original_model = req.model.clone();
    let Some((mut account, endpoint)) =
        codex_official_account_endpoint(&state, &original_model, route_surface).await
    else {
        return codex_official_pool_unavailable_response();
    };
    let history_context = history_context_for(&account, &endpoint, route_surface.responses_path());
    let model_map = endpoint.model_map.clone();
    let mapped_model = resolve_model(&original_model, &model_map);
    req.model = mapped_model.clone();
    match normalize_codex_official_body(&body, &mapped_model) {
        Ok(updated) => body = updated,
        Err(err) => {
            warn!("Codex 官方请求体规范化失败，继续使用原始请求体: {err}");
            if mapped_model != original_model {
                match serde_json::to_vec(&req) {
                    Ok(updated) => body = axum::body::Bytes::from(updated),
                    Err(err) => {
                        warn!("Codex 官方请求模型映射序列化失败，继续使用原始请求体: {err}");
                    }
                }
            }
        }
    }

    let token = match fresh_oauth_access_token(&state, &mut account).await {
        Ok(token) if !token.trim().is_empty() => token,
        Ok(_) => account.api_key.clone(),
        Err(err) => {
            update_runtime_result(
                &state,
                &account.id,
                &mapped_model,
                StatusCode::UNAUTHORIZED,
                format!("OAuth token refresh failed: {err}"),
                None,
            )
            .await;
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "error": {
                        "message": format!("OAuth token refresh failed: {err}"),
                        "type": "authentication_error",
                        "code": "oauth_refresh_failed"
                    }
                })),
            )
                .into_response();
        }
    };

    if token.trim().is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": {
                    "message": "Codex 官方账号缺少 OAuth access token",
                    "type": "authentication_error",
                    "code": "missing_oauth_token"
                }
            })),
        )
            .into_response();
    }

    let upstream = Url::parse(&endpoint.base_url)
        .unwrap_or_else(|_| Url::parse(CODEX_OFFICIAL_BASE_URL).unwrap());
    let path = if endpoint.path.trim().is_empty() {
        "responses"
    } else {
        endpoint.effective_path()
    };
    let url = format!("{}{}", join_base(&upstream), path);
    let response_id = state.sessions.new_id();
    let start = Instant::now();
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .bearer_auth(token)
        .header("Originator", codex_official_originator(&account))
        .header(
            "User-Agent",
            "codex_cli_rs/0.118.0 (Mac OS 26.3.1; arm64) deecodex/2.3",
        )
        .header("Connection", "Keep-Alive");
    if req.stream {
        builder = builder
            .header("Accept", "text/event-stream")
            .header("Accept-Encoding", "identity");
    } else {
        builder = builder.header("Accept", "application/json");
    }
    if let Some(session_id) = codex_session_id_from_body(&body) {
        builder = builder.header("Session_id", session_id);
    }
    if let Some(account_id) = oauth_account_id(&account) {
        builder = builder.header("Chatgpt-Account-Id", account_id);
    }
    for (k, v) in &endpoint.custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    if let Some(secs) = endpoint.request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let result = builder.body(body).send().await;
    let upstream_resp = match result {
        Ok(resp) => resp,
        Err(err) => {
            update_runtime_result(
                &state,
                &account.id,
                &mapped_model,
                StatusCode::BAD_GATEWAY,
                format!("Codex 官方上游连接失败: {err}"),
                None,
            )
            .await;
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": format!("Codex 官方上游连接失败: {err}"),
                        "type": "api_error",
                        "code": "upstream_error"
                    }
                })),
            )
                .into_response();
        }
    };
    let status = upstream_resp.status();
    let retry_after = retry_after_secs(upstream_resp.headers());

    let content_type = upstream_resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or(if req.stream {
            "text/event-stream"
        } else {
            "application/json"
        })
        .to_string();

    if req.stream {
        update_runtime_result(
            &state,
            &account.id,
            &mapped_model,
            status,
            if status.is_success() {
                String::new()
            } else {
                format!("HTTP {}", status.as_u16())
            },
            retry_after,
        )
        .await;
        let body = history_recording_sse_body(
            upstream_resp.bytes_stream(),
            SseHistoryBodyContext {
                request_history: state.request_history.clone(),
                history_context,
                response_id,
                model: mapped_model,
                start,
                upstream_url: url,
                http_status: status,
            },
        );
        return Response::builder()
            .status(status)
            .header("Content-Type", content_type)
            .body(body)
            .unwrap();
    }

    let bytes = match upstream_resp.bytes().await {
        Ok(bytes) => bytes,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": format!("Codex 官方响应读取失败: {err}"),
                        "type": "api_error",
                        "code": "upstream_read_error"
                    }
                })),
            )
                .into_response();
        }
    };
    let retry_after = codex_usage_retry_after_secs(status, &bytes, retry_after);
    update_runtime_result(
        &state,
        &account.id,
        &mapped_model,
        status,
        if status.is_success() {
            String::new()
        } else {
            codex_error_message(status, &bytes)
        },
        retry_after,
    )
    .await;
    let (input_tokens, output_tokens, cache_hit) = if status.is_success() {
        extract_proxy_response_usage(&bytes)
    } else {
        (0, 0, false)
    };
    state
        .request_history
        .record(record_from_context(
            &history_context,
            response_id,
            mapped_model,
            if status.is_success() {
                "completed".into()
            } else {
                "failed".into()
            },
            input_tokens,
            output_tokens,
            start.elapsed().as_millis() as u64,
            url,
            if status.is_success() {
                String::new()
            } else {
                format!("HTTP {}", status.as_u16())
            },
            cache_hit,
        ))
        .await;

    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .body(axum::body::Body::from(bytes))
        .unwrap()
}

fn codex_session_id_from_body(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("prompt_cache_key")
                .and_then(Value::as_str)
                .or_else(|| value.get("Session_id").and_then(Value::as_str))
                .map(str::to_string)
        })
        .filter(|value| !value.trim().is_empty())
}

fn oauth_account_id(account: &Account) -> Option<String> {
    account
        .client_options
        .get("oauth")
        .and_then(Value::as_object)
        .and_then(|oauth| oauth.get("account_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

async fn fresh_oauth_access_token(state: &AppState, account: &mut Account) -> Result<String> {
    let Some(oauth_value) = account.client_options.get("oauth").cloned() else {
        return Ok(account.api_key.clone());
    };
    let Some(token) = crate::oauth_accounts::oauth_token_from_value(&oauth_value) else {
        return Ok(account.api_key.clone());
    };
    let now = crate::oauth_accounts::now_secs();
    if token.expired_at == 0 || token.expired_at > now.saturating_add(60) {
        return Ok(token.access_token);
    }
    if token.refresh_token.trim().is_empty() {
        return Ok(token.access_token);
    }
    let provider = crate::oauth_accounts::OAuthProvider::parse(&token.provider)?;
    let refreshed =
        crate::oauth_accounts::refresh_token(&state.client, &provider, &token.refresh_token)
            .await?;
    let login_mode = oauth_value
        .get("login_mode")
        .and_then(Value::as_str)
        .unwrap_or("browser");
    account.api_key = refreshed.access_token.clone();
    account.client_options.insert(
        "oauth".into(),
        crate::oauth_accounts::oauth_token_to_value(&refreshed, login_mode),
    );
    persist_refreshed_oauth_account(state, account.clone()).await;
    Ok(refreshed.access_token)
}

async fn persist_refreshed_oauth_account(state: &AppState, account: Account) {
    {
        let mut store = state.account_store.write().await;
        if let Some(existing) = store
            .accounts
            .iter_mut()
            .find(|candidate| candidate.id == account.id)
        {
            existing.api_key = account.api_key.clone();
            existing.client_options = account.client_options.clone();
            existing.updated_at = crate::accounts::now_secs();
        }
    }
    if state.active_account.read().await.id == account.id {
        *state.active_account.write().await = account.clone();
    }
    if let Err(err) = crate::accounts::with_account_store(state.data_dir.as_ref(), |store| {
        if let Some(existing) = store
            .accounts
            .iter_mut()
            .find(|candidate| candidate.id == account.id)
        {
            existing.api_key = account.api_key.clone();
            existing.client_options = account.client_options.clone();
            existing.updated_at = crate::accounts::now_secs();
        }
        Ok(())
    }) {
        warn!("保存刷新后的 OAuth token 失败: {err}");
    }
}

async fn update_runtime_result(
    state: &AppState,
    account_id: &str,
    model: &str,
    status: StatusCode,
    message: String,
    retry_after_secs: Option<u64>,
) {
    let now = crate::accounts::now_secs();
    let message_for_persist = message.clone();
    let mut active_update = None;
    {
        let mut store = state.account_store.write().await;
        if let Some(account) = store
            .accounts
            .iter_mut()
            .find(|candidate| candidate.id == account_id)
        {
            if status.is_success() {
                account.record_runtime_success(model, now);
            } else {
                account.record_runtime_failure(
                    model,
                    status.as_u16(),
                    message.clone(),
                    retry_after_secs,
                    now,
                );
            }
            active_update = Some(account.clone());
        }
    }
    if let Some(account) = active_update {
        if state.active_account.read().await.id == account.id {
            *state.active_account.write().await = account;
        }
    }
    if let Err(err) = crate::accounts::with_account_store(state.data_dir.as_ref(), |store| {
        if let Some(account) = store
            .accounts
            .iter_mut()
            .find(|candidate| candidate.id == account_id)
        {
            if status.is_success() {
                account.record_runtime_success(model, now);
            } else {
                account.record_runtime_failure(
                    model,
                    status.as_u16(),
                    message_for_persist,
                    retry_after_secs,
                    now,
                );
            }
        }
        Ok(())
    }) {
        warn!("保存账号运行态失败: {err}");
    }
}

fn retry_after_secs(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn codex_usage_retry_after_secs(
    status: StatusCode,
    body: &[u8],
    header_retry_after: Option<u64>,
) -> Option<u64> {
    if header_retry_after.is_some() || status != StatusCode::TOO_MANY_REQUESTS || body.is_empty() {
        return header_retry_after;
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return None;
    };
    let error = value.get("error").unwrap_or(&value);
    let error_type = error
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if error_type != "usage_limit_reached" {
        return None;
    }
    let now = crate::accounts::now_secs();
    if let Some(resets_at) = error.get("resets_at").and_then(Value::as_u64) {
        if resets_at > now {
            return Some(resets_at.saturating_sub(now));
        }
    }
    error
        .get("resets_in_seconds")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
}

fn codex_error_message(status: StatusCode, body: &[u8]) -> String {
    let fallback = format!("HTTP {}", status.as_u16());
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return fallback;
    };
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .unwrap_or(fallback)
}

async fn bypass_stream_forward(
    BypassArgs {
        state,
        body,
        upstream_url,
        endpoint_path,
        api_key,
        custom_headers,
        timeout_secs,
        max_retries,
        response_id,
        store_response: _,
        model,
        requested_service_tier,
        history_context,
    }: BypassArgs,
) -> Response {
    let url = format!("{}{}", join_base(&upstream_url), endpoint_path);
    let start = std::time::Instant::now();

    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("Accept-Encoding", "identity");

    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key.as_str());
    }

    for (k, v) in &custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }

    if let Some(secs) = timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let max_retries = max_retries.unwrap_or(3) as usize;
    let mut attempt: usize = 0;
    let mut delay_ms: u64 = 500;

    let resp = loop {
        let Some(request) = builder.try_clone() else {
            let message = "failed to clone upstream request builder".to_string();
            error!("bypass stream upstream request build error: {message}");
            let _ = state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id.clone(),
                    model.clone(),
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    url.clone(),
                    message.clone(),
                    false,
                ))
                .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "code": "request_builder_clone_failed",
                        "message": message,
                        "type": "internal_error"
                    }
                })),
            )
                .into_response();
        };
        match request.body(body.clone()).send().await {
            Err(e) => {
                if attempt < max_retries {
                    attempt += 1;
                    warn!("bypass stream upstream error (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2;
                    continue;
                }
                error!("bypass stream upstream exhausted retries: {e}");
                let _ = state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id,
                        model,
                        "failed".into(),
                        0,
                        0,
                        start.elapsed().as_millis() as u64,
                        url,
                        format!("connection error: {e}"),
                        false,
                    ))
                    .await;
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("upstream connection error: {e}"),
                )
                    .into_response();
            }
            Ok(r) => break r,
        }
    };

    let status = resp.status();
    let x_request_id = resp
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("text/event-stream")
        .to_string();

    if !status.is_success() {
        let error_body = resp.text().await.unwrap_or_default();
        let _ = state
            .request_history
            .record(record_from_context(
                &history_context,
                response_id,
                model,
                "failed".into(),
                0,
                0,
                start.elapsed().as_millis() as u64,
                url,
                format!("HTTP {}", status.as_u16()),
                false,
            ))
            .await;
        // 上游可能返回 HTML，转为 JSON 错误
        let error_body = if error_body.trim_start().starts_with('<') {
            format!("upstream returned HTTP {}", status.as_u16())
        } else {
            error_body
        };
        return (
            status,
            Json(json!({
                "error": {
                    "code": status.as_u16().to_string(),
                    "message": error_body,
                    "type": "upstream_error"
                }
            })),
        )
            .into_response();
    }

    if requested_service_tier.is_some() {
        tracing::warn!(
            model = %model,
            requested_service_tier = %requested_service_tier.as_deref().unwrap_or(""),
            "Responses 直连流式透传将在流结束后记录 usage/cache_hit"
        );
    }
    let body_stream = history_recording_sse_body(
        resp.bytes_stream(),
        SseHistoryBodyContext {
            request_history: state.request_history.clone(),
            history_context,
            response_id,
            model,
            start,
            upstream_url: url,
            http_status: status,
        },
    );
    let mut resp = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", content_type)
        .body(body_stream)
        .unwrap();
    if let Some(req_id) = x_request_id {
        if let Ok(v) = header::HeaderValue::from_str(&req_id) {
            resp.headers_mut()
                .insert(header::HeaderName::from_static("x-request-id"), v);
        }
    }
    resp
}

async fn bypass_send_request(
    BypassArgs {
        state,
        body,
        upstream_url,
        endpoint_path,
        api_key,
        custom_headers,
        timeout_secs,
        max_retries,
        response_id,
        store_response,
        model,
        requested_service_tier,
        history_context,
    }: BypassArgs,
) -> Response {
    let url = format!("{}{}", join_base(&upstream_url), endpoint_path);
    let start = std::time::Instant::now();

    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json");

    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key.as_str());
    }

    for (k, v) in &custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }

    if let Some(secs) = timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let max_retries = max_retries.unwrap_or(3) as usize;
    let mut attempt: usize = 0;
    let mut delay_ms: u64 = 500;

    let result = loop {
        let Some(request) = builder.try_clone() else {
            let message = "failed to clone upstream request builder";
            error!("bypass non-stream upstream request build error: {message}");
            if store_response {
                state.sessions.save_response(
                    response_id.clone(),
                    json!({
                        "id": response_id,
                        "object": "response",
                        "status": "failed",
                        "error": {"code": "request_builder_clone_failed", "message": message}
                    }),
                );
            }
            let _ = state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id,
                    model,
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    url,
                    message.into(),
                    false,
                ))
                .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": {
                        "message": message,
                        "type": "internal_error",
                        "code": "request_builder_clone_failed"
                    }
                })),
            )
                .into_response();
        };
        match request.body(body.clone()).send().await {
            Err(e) => {
                if attempt < max_retries {
                    attempt += 1;
                    warn!("bypass non-stream upstream error (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2;
                    continue;
                }
                break Err(e);
            }
            Ok(r) => break Ok(r),
        }
    };

    match result {
        Err(e) => {
            error!("bypass non-stream upstream exhausted retries: {e}");
            if store_response {
                state.sessions.save_response(
                    response_id.clone(),
                    json!({
                        "id": response_id,
                        "object": "response",
                        "status": "failed",
                        "error": {"code": "upstream_error", "message": format!("{e}")}
                    }),
                );
            }
            let _ = state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id,
                    model,
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    url,
                    format!("connection error: {e}"),
                    false,
                ))
                .await;
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "message": format!("upstream connection error: {e}"),
                        "type": "api_error",
                        "code": "upstream_error"
                    }
                })),
            )
                .into_response()
        }
        Ok(resp) => {
            let status = resp.status();
            let response_body: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    error!("bypass non-stream JSON parse: {e}");
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(json!({
                            "error": {
                                "message": format!("failed to parse upstream response: {e}"),
                                "type": "api_error",
                                "code": "invalid_response"
                            }
                        })),
                    )
                        .into_response();
                }
            };

            if store_response {
                state
                    .sessions
                    .save_response(response_id.clone(), response_body.clone());
            }

            let upstream_service_tier = response_body
                .get("service_tier")
                .or_else(|| {
                    response_body
                        .get("response")
                        .and_then(|r| r.get("service_tier"))
                })
                .and_then(Value::as_str);
            if let Some(service_tier) = upstream_service_tier {
                tracing::info!(
                    model = %model,
                    service_tier = %service_tier,
                    "上游响应回显 service_tier"
                );
            } else if requested_service_tier.is_some() {
                tracing::warn!(
                    model = %model,
                    requested_service_tier = %requested_service_tier.as_deref().unwrap_or(""),
                    "上游响应未回显 service_tier，无法仅凭响应确认 GPT 已采用该服务层"
                );
            }

            {
                let usage = response_body.get("usage");
                let input_tokens = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let output_tokens = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let cache_hit = usage
                    .and_then(|u| u.as_object())
                    .map(is_cache_hit)
                    .unwrap_or(false);
                state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id,
                        model,
                        if status.is_success() {
                            "completed"
                        } else {
                            "failed"
                        }
                        .into(),
                        input_tokens,
                        output_tokens,
                        start.elapsed().as_millis() as u64,
                        url,
                        String::new(),
                        cache_hit,
                    ))
                    .await;
            }

            (
                if status.is_success() {
                    StatusCode::OK
                } else {
                    status
                },
                Json(response_body),
            )
                .into_response()
        }
    }
}

async fn handle_responses_inner(
    state: AppState,
    req: ResponsesRequest,
    raw_body: axum::body::Bytes,
    local_output_prefix_items: Vec<Value>,
    local_input_suffix_items: Vec<Value>,
    route_surface: AccountRouteSurface,
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
    let (account, endpoint) = active_account_endpoint_for_route(&state, route_surface).await;
    let history_context = history_context_for(&account, &endpoint, route_surface.responses_path());
    let model_map = endpoint.model_map.clone();
    let mapped_model = resolve_model(&original_model, &model_map);
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

    if let Some(trigger) = dev_pipeline::detect_trigger(&req, &account) {
        let pipeline_start = Instant::now();
        let pipeline_store = state.account_store.read().await.clone();
        let pipeline_ctx = dev_pipeline::DevPipelineContext {
            client: state.client.clone(),
            store: pipeline_store,
            active_account: account.clone(),
            active_endpoint_id: state.account_store.read().await.active_endpoint_id.clone(),
            requested_model: original_model.clone(),
            temperature: req.temperature,
            top_p: req.top_p,
            max_output_tokens: req.max_output_tokens,
            chinese_thinking: state.chinese_thinking,
        };
        match dev_pipeline::run(trigger, pipeline_ctx).await {
            Ok(output) => {
                return dev_pipeline_response(DevPipelineResponseArgs {
                    state,
                    req: &req,
                    output,
                    request_input_items,
                    store_response,
                    conversation_id,
                    response_extra,
                    history_context: history_context.clone(),
                    start: pipeline_start,
                })
                .await;
            }
            Err(err) => {
                warn!(
                    account_id = %account.id,
                    account_name = %account.name,
                    error = %err,
                    "开发协作编排失败，回退普通主模型路径"
                );
            }
        }
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
        &model_map,
        state.chinese_thinking,
    );
    let mut chat_req = translated.chat;
    if let Some(ref forced) = endpoint.reasoning_effort_override {
        chat_req.reasoning_effort = Some(forced.clone());
        chat_req.thinking = Some(serde_json::json!({"type": "enabled"}));
    }
    if let Some(budget) = endpoint.thinking_tokens {
        if let Some(ref mut thinking) = chat_req.thinking {
            thinking["budget_tokens"] = serde_json::json!(budget);
        }
    }

    let capability_observation = build_capability_observation(&state, &req, &raw_body).await;

    // Route to VLM when the current turn has new images (not just history carrying old ones)
    let is_review_model = original_model.contains("auto-review");
    let has_new_image = response_input_has_new_image(&req.input);
    let vision_mode = if is_review_model {
        VisionMode::Off
    } else {
        endpoint.model_vision_mode(&mapped_model)
    };
    let route_to_vision = translated.has_images
        && has_new_image
        && vision_mode == VisionMode::Glue
        && !capability_observation.suppress_vision_route;
    let native_vision = translated.has_images && has_new_image && vision_mode == VisionMode::Native;
    info!(
        "route_to_vision: has_images={} review={} new_image={} msgs={} mode={:?} route={}",
        translated.has_images,
        is_review_model,
        has_new_image,
        chat_req.messages.len(),
        vision_mode,
        route_to_vision
    );

    if translated.has_images && has_new_image && vision_mode == VisionMode::Off {
        match endpoint.vision.unsupported_image_policy {
            UnsupportedImagePolicy::Reject => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "message": "当前端点未启用视觉能力，无法处理图片输入。请在账号端点中选择原生多模态或胶水多模态。",
                            "type": "unsupported_image",
                            "code": "vision_disabled"
                        }
                    })),
                )
                    .into_response();
            }
            UnsupportedImagePolicy::StripWithWarning => {
                warn!("当前端点未启用视觉能力，已按配置剥离图片后继续请求");
            }
        }
    }

    // 非原生视觉端点必须剥离 image_url，否则 DeepSeek 等上游会拒绝。
    if !route_to_vision && !native_vision {
        strip_images_from_chat_request(&mut chat_req);
    }

    // 能力通道观察注入（在 strip_images 之后，保护多模态 content 不被剥离）
    if let Some(observation) = capability_observation.message {
        let insert_at = if chat_req
            .messages
            .first()
            .is_some_and(|message| message.role == "system")
        {
            1
        } else {
            0
        };
        chat_req.messages.insert(insert_at, observation);
    }

    let mut use_vision_transport = route_to_vision;
    let (url, api_key) = if route_to_vision {
        let vu = match parse_glue_vision_base_url(&endpoint) {
            Ok(url) => url,
            Err(response) => return *response,
        };
        if let Err(response) = ensure_minimax_glue_adapter(&endpoint) {
            return *response;
        }
        let url = format!("{}{}", join_base(&vu), endpoint.vision.path.as_str());
        let vmodel = endpoint.vision.model.clone();
        info!(
            "📷 routing to vision upstream: {} model={} endpoint={} adapter={}",
            vu,
            vmodel,
            endpoint.vision.path.as_str(),
            endpoint.vision.adapter_id
        );

        let vlm_body = build_minimax_vlm_body(&chat_req);
        if endpoint.vision.glue_strategy == GlueVisionStrategy::CaptionThenMain {
            match request_minimax_vlm_text(&state, &url, &endpoint.vision.api_key, &vlm_body).await
            {
                Ok(caption) => {
                    strip_images_from_chat_request(&mut chat_req);
                    chat_req.messages.push(ChatMessage {
                        role: "user".into(),
                        content: Some(Value::String(format!(
                            "图片内容由视觉模型识别如下，请结合上下文继续完成原始任务：\n{}",
                            caption
                        ))),
                        reasoning_content: None,
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });
                    use_vision_transport = false;
                }
                Err(resp) => return resp,
            }
        } else {
            return handle_minimax_vlm(VlmArgs {
                state,
                url,
                api_key: endpoint.vision.api_key.clone(),
                vlm_body,
                model: vmodel,
                stream_response: req.stream,
                store_response,
                request_input_items,
                response_extra,
            })
            .await;
        }

        if use_vision_transport {
            chat_req.model = vmodel;
            (url, endpoint.vision.api_key.clone())
        } else {
            let upstream = Url::parse(&endpoint.base_url)
                .unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
            let url = format!("{}{}", join_base(&upstream), endpoint.effective_path());
            (url, account.api_key.clone())
        }
    } else {
        let upstream = Url::parse(&endpoint.base_url)
            .unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
        let url = format!("{}{}", join_base(&upstream), endpoint.effective_path());
        (url, account.api_key.clone())
    };

    let vision_label = if use_vision_transport { " 📷" } else { "" };
    providers::adapt_chat_request(&providers::profile_for_account(&account), &mut chat_req);
    let adapted_reasoning_effort = chat_req.reasoning_effort.clone();
    let adapted_thinking = chat_req.thinking.clone();
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
        fmt_effort(&adapted_reasoning_effort),
        fmt_thinking(&adapted_thinking),
        msg_count,
        chat_req.tools.len(),
        tool_names.join(", "),
        vision_label
    );

    if endpoint.kind == EndpointKind::AnthropicMessages {
        if req.stream || req.background == Some(true) {
            return (
                StatusCode::NOT_IMPLEMENTED,
                Json(json!({
                    "error": {
                        "message": "Anthropic Messages 端点当前支持非流式请求；流式与后台模式将在后续版本接入",
                        "type": "unsupported_endpoint_mode",
                        "code": "anthropic_messages_stream_reserved"
                    }
                })),
            )
                .into_response();
        }

        let response_id = state.sessions.new_id();
        if store_response {
            state
                .sessions
                .save_input_items(response_id.clone(), request_input_items.clone());
        }
        let upstream = Url::parse(&endpoint.base_url)
            .unwrap_or_else(|_| Url::parse("http://localhost").unwrap());
        let url = format!("{}{}", join_base(&upstream), endpoint.effective_path());
        return handle_anthropic_messages(AnthropicArgs {
            state,
            chat_req,
            url,
            model: mapped_model,
            api_key: account.api_key.clone(),
            auth_scheme: providers::profile_for_account(&account).auth_scheme,
            custom_headers: endpoint.custom_headers.clone(),
            request_timeout_secs: endpoint.request_timeout_secs,
            max_retries: endpoint.max_retries,
            thinking_tokens: endpoint.thinking_tokens,
            response_id,
            store_response,
            conversation_id,
            response_extra,
            req: &req,
            history_context: history_context.clone(),
            start: Instant::now(),
        })
        .await;
    }

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
        let bg_custom_headers = endpoint.custom_headers.clone();
        let bg_timeout = endpoint.request_timeout_secs;
        let bg_max_retries = endpoint.max_retries;
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
                custom_headers: bg_custom_headers,
                request_timeout_secs: bg_timeout,
                max_retries: bg_max_retries,
                response_id: bg_id,
                store_response,
                conversation_id: bg_conversation_id,
                response_extra: response_extra.clone(),
                req: &bg_req,
                history_context: history_context.clone(),
                start: Instant::now(),
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
        let thinking_enabled = adapted_thinking
            .as_ref()
            .is_some_and(|t| t.get("type").and_then(serde_json::Value::as_str) != Some("disabled"));

        // Check request cache
        let cache_key = RequestCache::hash_request(&chat_req);
        if store_response && req.background != Some(true) && conversation_id.is_none() {
            if let Some(cached) = state.request_cache.get(cache_key) {
                info!("request cache: hit (key={})", cache_key);
                let input_tokens = cached.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0);
                let output_tokens = cached
                    .usage
                    .as_ref()
                    .map(|u| u.completion_tokens)
                    .unwrap_or(0);
                let cached_sse = stream::translate_cached(stream::CachedArgs {
                    response_id: response_id.clone(),
                    model: mapped_model.clone(),
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
                let _ = state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id.clone(),
                        mapped_model.clone(),
                        "completed".into(),
                        input_tokens,
                        output_tokens,
                        0,
                        url.clone(),
                        String::new(),
                        true,
                    ))
                    .await;
                return resp;
            }
        }

        let start = Instant::now();
        let sse = stream::translate_stream(stream::StreamArgs {
            client: state.client,
            url: url.clone(),
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
            model_map: model_map.clone(),
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
            allowed_mcp_servers: state.tool_policy.read().await.allowed_mcp_servers.clone(),
            allowed_computer_displays: state
                .tool_policy
                .read()
                .await
                .allowed_computer_displays
                .clone(),
            custom_headers: endpoint.custom_headers.clone(),
            request_timeout_secs: endpoint.request_timeout_secs,
            max_retries: endpoint.max_retries,
            request_history: state.request_history.clone(),
            history_context: history_context.clone(),
            upstream_url: url,
            allow_missing_done: providers::profile_for_account(&account)
                .capabilities
                .allow_missing_done,
            start,
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
            custom_headers: endpoint.custom_headers.clone(),
            request_timeout_secs: endpoint.request_timeout_secs,
            max_retries: endpoint.max_retries,
            response_id,
            store_response,
            conversation_id,
            response_extra,
            req: &req,
            history_context: history_context.clone(),
            start,
        })
        .await;
        let elapsed = start.elapsed();
        debug!("blocking request completed in {:.0}ms", elapsed.as_millis());
        resp
    }
}

async fn build_capability_observation(
    state: &AppState,
    req: &ResponsesRequest,
    raw_body: &[u8],
) -> CapabilityObservationResult {
    let main_account = state.active_account.read().await.clone();
    let requested = !capability::capability_observer_request(req)
        && main_account.capability_enabled
        && capability::detect_trigger(req).is_some();
    if !requested {
        return CapabilityObservationResult {
            message: None,
            suppress_vision_route: false,
        };
    }

    let helper_account = {
        let store = state.account_store.read().await;
        main_account
            .capability_account_id
            .as_ref()
            .and_then(|helper_id| store.accounts.iter().find(|a| &a.id == helper_id).cloned())
    };

    let helper_context = if let Some(helper) = helper_account.as_ref() {
        let mut normalized_helper = helper.clone();
        if normalized_helper.endpoints.is_empty() {
            normalized_helper.normalize_v2();
        }
        let Some(helper_endpoint) = normalized_helper
            .active_endpoint(None)
            .cloned()
            .or_else(|| normalized_helper.endpoints.first().cloned())
        else {
            warn!(
                account_id = %helper.id,
                account_name = %helper.name,
                "能力账号没有可用端点，Computer Use 原生能力通道不可用"
            );
            return CapabilityObservationResult {
                message: Some(capability_config_error_message(
                    "能力账号没有可用端点，无法接管 Computer Use。",
                )),
                suppress_vision_route: true,
            };
        };
        if !helper_endpoint.kind.is_responses_like() {
            warn!(
                account_id = %helper.id,
                account_name = %helper.name,
                endpoint_kind = ?helper_endpoint.kind,
                "能力账号必须配置为原生 Responses 端点，拒绝回退到 Chat 或本地桥"
            );
            return CapabilityObservationResult {
                message: Some(capability_config_error_message(
                    "能力账号必须配置为原生 Responses 端点；当前端点不是 Responses，已拒绝回退到 Chat 或本地桥。",
                )),
                suppress_vision_route: true,
            };
        }
        match validate_upstream(&helper_endpoint.base_url) {
            Ok(upstream) => Some(capability::CapabilityContext {
                client: state.client.clone(),
                upstream,
                endpoint_path: helper_endpoint.effective_path().to_string(),
                api_key: helper.api_key.clone(),
                custom_headers: helper_endpoint.custom_headers.clone(),
                timeout_secs: helper_endpoint.request_timeout_secs,
                max_retries: helper_endpoint.max_retries,
                model_map: helper_endpoint.model_map.clone(),
                executors: state.executors.read().await.clone(),
                tool_policy: state.tool_policy.read().await.clone(),
            }),
            Err(err) => {
                warn!(
                    account_id = %helper.id,
                    account_name = %helper.name,
                    error = %err,
                    "能力账号上游 URL 无效，回退主模型并跳过旧视觉路由"
                );
                return CapabilityObservationResult {
                    message: Some(capability_config_error_message(
                        "能力账号上游 URL 无效，无法接管 Computer Use。",
                    )),
                    suppress_vision_route: true,
                };
            }
        }
    } else {
        None
    };

    let Some(context) = helper_context else {
        warn!(
            account_id = %main_account.id,
            account_name = %main_account.name,
            "能力补全已触发但未配置有效能力账号，回退主模型并跳过旧视觉路由"
        );
        return CapabilityObservationResult {
            message: Some(capability_config_error_message(
                "未配置有效的原生 Responses 能力账号，无法接管 Computer Use。",
            )),
            suppress_vision_route: true,
        };
    };

    let message =
        capability::maybe_observe(req, raw_body, &main_account, helper_account, context).await;
    if message.is_none() {
        warn!(
            account_id = %main_account.id,
            account_name = %main_account.name,
            "能力补全已触发但未产生可注入观察，回退主模型并允许旧视觉路由"
        );
    }
    CapabilityObservationResult {
        suppress_vision_route: message.is_some(),
        message,
    }
}

fn capability_config_error_message(message: &str) -> ChatMessage {
    ChatMessage {
        role: "system".into(),
        content: Some(Value::String(format!(
            "【deecodex 能力账号配置错误】{message}请直接告知用户该配置问题；不要尝试由主模型、Chat fallback 或本地 MCP bridge 执行 Computer Use。"
        ))),
        reasoning_content: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }
}

async fn dev_pipeline_response(args: DevPipelineResponseArgs<'_>) -> Response {
    let DevPipelineResponseArgs {
        state,
        req,
        output,
        request_input_items,
        store_response,
        conversation_id,
        response_extra,
        history_context,
        start,
    } = args;
    let response_id = state.sessions.new_id();
    if store_response {
        state
            .sessions
            .save_input_items(response_id.clone(), request_input_items.clone());
    }

    let item_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let mut value = enrich_response_object(
        json!({
            "id": response_id,
            "object": "response",
            "status": "completed",
            "model": output.final_model,
            "output": [{
                "id": item_id,
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": output.final_text,
                    "annotations": []
                }]
            }],
            "usage": {
                "input_tokens": 0,
                "output_tokens": 0,
                "total_tokens": 0
            }
        }),
        req,
    );
    value["metadata"]["x_deecodex_dev_pipeline"] = json!("true");
    value["metadata"]["x_deecodex_dev_pipeline_elapsed_ms"] = json!(output.elapsed_ms.to_string());
    value["metadata"]["x_deecodex_dev_pipeline_stages"] = json!(output
        .traces
        .iter()
        .map(|trace| {
            json!({
                "role": trace.role,
                "account_id": trace.account_id,
                "account_name": trace.account_name,
                "model": trace.model,
                "elapsed_ms": trace.elapsed_ms
            })
        })
        .collect::<Vec<_>>());
    let value = response_with_extra(value, &response_extra);

    if store_response {
        save_response_unless_cancelled(&state.sessions, response_id.clone(), value.clone());
    }
    if let Some(id) = conversation_id.as_deref() {
        let mut items = state.sessions.get_conversation_items(id);
        if let Some(output_items) = value.get("output").and_then(Value::as_array) {
            items.extend(output_items.iter().cloned());
            state
                .sessions
                .save_conversation_items(id.to_string(), items);
        }
    }
    let _ = state
        .request_history
        .record(record_from_context(
            &history_context,
            response_id,
            value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            "completed".into(),
            0,
            0,
            start.elapsed().as_millis() as u64,
            "dev-pipeline://local".into(),
            String::new(),
            false,
        ))
        .await;

    if req.stream {
        dev_pipeline_stream_response(value)
    } else {
        Json(value).into_response()
    }
}

fn dev_pipeline_stream_response(value: Value) -> Response {
    let response_id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp_dev_pipeline");
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("dev-pipeline");
    let item = value
        .get("output")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or_else(
            || json!({"id":"msg_dev_pipeline","type":"message","role":"assistant","content":[]}),
        );
    let item_id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("msg_dev_pipeline");
    let text = item
        .get("content")
        .and_then(Value::as_array)
        .and_then(|parts| parts.first())
        .and_then(|part| part.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut state = SseState::new();
    let output_index = state.alloc_output_index();
    let events = vec![
        state.response_created(response_id, model),
        state.response_in_progress(response_id),
        state.output_item_added(
            output_index,
            item_id,
            "message",
            json!({"role": "assistant", "content": []}),
        ),
        state.content_part_added(
            item_id,
            output_index,
            0,
            json!({"type": "output_text", "text": "", "annotations": []}),
        ),
        state.output_text_delta(item_id, output_index, 0, text),
        state.output_text_done(item_id, output_index, 0, text),
        state.content_part_done(
            item_id,
            output_index,
            0,
            json!({"type": "output_text", "text": text, "annotations": []}),
        ),
        state.output_item_done(output_index, item),
        state.response_completed(&value),
    ];
    let mut ok_events: Vec<Result<Event, std::convert::Infallible>> =
        events.into_iter().filter_map(Result::ok).map(Ok).collect();
    ok_events.push(Ok(Event::default().data("[DONE]")));
    Sse::new(futures_util::stream::iter(ok_events))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn handle_blocking(args: BlockingArgs<'_>) -> Response {
    let BlockingArgs {
        state,
        chat_req,
        url,
        model,
        api_key,
        custom_headers,
        request_timeout_secs,
        max_retries,
        response_id,
        store_response,
        conversation_id,
        response_extra,
        req,
        history_context,
        start,
    } = args;
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json");

    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key.as_str());
    }

    for (k, v) in &custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }

    if let Some(secs) = request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let max_retries = max_retries.unwrap_or(3) as usize;
    let mut attempt: usize = 0;
    let mut delay_ms: u64 = 500;
    let result = loop {
        let Some(request) = builder.try_clone() else {
            return upstream_failure_response(UpstreamFailureArgs {
                state: &state,
                response_id,
                model,
                url,
                store_response,
                req,
                response_extra,
                start,
                code: "request_builder_clone_failed".into(),
                message: "failed to clone upstream request builder".into(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                history_context: history_context.clone(),
            })
            .await;
        };
        match request.json(&chat_req).send().await {
            Err(e) => {
                if attempt < max_retries {
                    attempt += 1;
                    warn!("upstream connection error (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2;
                    continue;
                }
                break Err(e);
            }
            Ok(r) => break Ok(r),
        }
    };

    match result {
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
            let _ = state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id,
                    model,
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    url,
                    e.to_string(),
                    false,
                ))
                .await;
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "code": "connection_error",
                        "message": format!("upstream connection error: {e}"),
                        "type": "upstream_error"
                    }
                })),
            )
                .into_response()
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
            let _ = state
                .request_history
                .record(record_from_context(
                    &history_context,
                    response_id,
                    model,
                    "failed".into(),
                    0,
                    0,
                    start.elapsed().as_millis() as u64,
                    url,
                    format!("HTTP {}", status.as_u16()),
                    false,
                ))
                .await;
            // 上游可能返回 HTML，Codex 期望 JSON，统一转为 JSON 错误
            let error_message = if body.trim_start().starts_with('<') {
                format!("upstream returned {}", status.as_u16())
            } else {
                body
            };
            (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                Json(json!({
                    "error": {
                        "code": status.as_u16().to_string(),
                        "message": error_message,
                        "type": "upstream_error"
                    }
                })),
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
                let _ = state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id,
                        model,
                        "failed".into(),
                        0,
                        0,
                        start.elapsed().as_millis() as u64,
                        url,
                        e.to_string(),
                        false,
                    ))
                    .await;
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

                let _ = state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id.clone(),
                        model.clone(),
                        "completed".into(),
                        chat_resp
                            .usage
                            .as_ref()
                            .map(|u| u.prompt_tokens)
                            .unwrap_or(0),
                        chat_resp
                            .usage
                            .as_ref()
                            .map(|u| u.completion_tokens)
                            .unwrap_or(0),
                        start.elapsed().as_millis() as u64,
                        url.clone(),
                        String::new(),
                        false,
                    ))
                    .await;

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

async fn handle_anthropic_messages(args: AnthropicArgs<'_>) -> Response {
    let AnthropicArgs {
        state,
        chat_req,
        url,
        model,
        api_key,
        auth_scheme,
        custom_headers,
        request_timeout_secs,
        max_retries,
        thinking_tokens,
        response_id,
        store_response,
        conversation_id,
        response_extra,
        req,
        history_context,
        start,
    } = args;

    let body = anthropic::to_messages_body(&chat_req, thinking_tokens);
    let mut builder = state
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("anthropic-version", "2023-06-01");

    let mut auth_profile = providers::profile_by_slug("custom");
    auth_profile.auth_scheme = auth_scheme;
    for (name, value) in providers::request_headers(&auth_profile, &api_key) {
        builder = builder.header(name, value);
    }
    for (k, v) in &custom_headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(k.as_bytes()),
            header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    if let Some(secs) = request_timeout_secs {
        builder = builder.timeout(std::time::Duration::from_secs(secs));
    }

    let max_retries = max_retries.unwrap_or(3) as usize;
    let mut attempt = 0;
    let mut delay_ms = 500;
    let result = loop {
        let Some(request) = builder.try_clone() else {
            return upstream_failure_response(UpstreamFailureArgs {
                state: &state,
                response_id,
                model,
                url,
                store_response,
                req,
                response_extra,
                start,
                code: "request_builder_clone_failed".into(),
                message: "failed to clone anthropic request builder".into(),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                history_context: history_context.clone(),
            })
            .await;
        };
        match request.json(&body).send().await {
            Err(e) => {
                if attempt < max_retries {
                    attempt += 1;
                    warn!("anthropic upstream connection error (attempt {attempt}/{max_retries}), retrying in {delay_ms}ms: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2;
                    continue;
                }
                break Err(e);
            }
            Ok(r) => break Ok(r),
        }
    };

    match result {
        Err(e) => {
            upstream_failure_response(UpstreamFailureArgs {
                state: &state,
                response_id,
                model,
                url,
                store_response,
                req,
                response_extra,
                start,
                code: "connection_error".into(),
                message: format!("anthropic upstream connection error: {e}"),
                status: StatusCode::BAD_GATEWAY,
                history_context: history_context.clone(),
            })
            .await
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body_text = r.text().await.unwrap_or_default();
            error!("anthropic upstream {}: {}", status.as_u16(), body_text);
            upstream_failure_response(UpstreamFailureArgs {
                state: &state,
                response_id,
                model,
                url,
                store_response,
                req,
                response_extra,
                start,
                code: status.as_u16().to_string(),
                message: body_text,
                status: StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                history_context: history_context.clone(),
            })
            .await
        }
        Ok(r) => match r.json::<Value>().await {
            Err(e) => {
                error!("anthropic parse error: {e}");
                upstream_failure_response(UpstreamFailureArgs {
                    state: &state,
                    response_id,
                    model,
                    url,
                    store_response,
                    req,
                    response_extra,
                    start,
                    code: "parse_error".into(),
                    message: e.to_string(),
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    history_context: history_context.clone(),
                })
                .await
            }
            Ok(value) => {
                let chat_resp = anthropic::response_to_chat(value);
                let usage_str = format_usage(chat_resp.usage.as_ref());
                info!("↑ anthropic done {}", usage_str);
                let assistant_msg = chat_resp
                    .choices
                    .first()
                    .map(|c| c.message.clone())
                    .unwrap_or_else(|| ChatMessage {
                        role: "assistant".into(),
                        content: Some(Value::String(String::new())),
                        reasoning_content: None,
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });

                let mut full_history = chat_req.messages.clone();
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

                let input_tokens = chat_resp
                    .usage
                    .as_ref()
                    .map(|u| u.prompt_tokens)
                    .unwrap_or(0);
                let output_tokens = chat_resp
                    .usage
                    .as_ref()
                    .map(|u| u.completion_tokens)
                    .unwrap_or(0);
                let _ = state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id.clone(),
                        model.clone(),
                        "completed".into(),
                        input_tokens,
                        output_tokens,
                        start.elapsed().as_millis() as u64,
                        url,
                        String::new(),
                        false,
                    ))
                    .await;

                let (resp, _) = translate::from_chat_response(response_id, &model, chat_resp);
                match serde_json::to_value(&resp) {
                    Ok(value) => {
                        let mut value = enrich_response_object(value, req);
                        value["status"] = json!("completed");
                        let value = response_with_extra(value, &response_extra);
                        if store_response {
                            save_response_unless_cancelled(
                                &state.sessions,
                                resp.id.clone(),
                                value.clone(),
                            );
                        }
                        Json(value).into_response()
                    }
                    Err(_) => Json(resp).into_response(),
                }
            }
        },
    }
}

async fn upstream_failure_response(args: UpstreamFailureArgs<'_>) -> Response {
    let UpstreamFailureArgs {
        state,
        response_id,
        model,
        url,
        store_response,
        req,
        response_extra,
        start,
        code,
        message,
        status,
        history_context,
    } = args;
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
                        "error": {"code": code, "message": message}
                    }),
                    req,
                ),
                &response_extra,
            ),
        );
    }
    let _ = state
        .request_history
        .record(record_from_context(
            &history_context,
            response_id,
            model,
            "failed".into(),
            0,
            0,
            start.elapsed().as_millis() as u64,
            url,
            message.clone(),
            false,
        ))
        .await;
    (
        status,
        Json(json!({
            "error": {
                "code": code,
                "message": message,
                "type": "upstream_error"
            }
        })),
    )
        .into_response()
}

fn save_response_unless_cancelled(sessions: &SessionStore, id: String, response: Value) {
    if sessions.response_status(&id).as_deref() == Some("cancelled") {
        return;
    }
    sessions.save_response(id, response);
}

async fn append_local_computer_outputs(state: &AppState, response: &mut Value) {
    let computer_config = state.executors.read().await.computer.clone();
    if !computer_config.enabled() {
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

    let allowed_displays = {
        let guard = state.tool_policy.read().await;
        guard.allowed_computer_displays.clone()
    };
    let mut outputs = Vec::new();
    for (call_id, invocation) in calls {
        let result = if !allowed_displays.is_empty()
            && !allowed_displays
                .iter()
                .any(|display| display == &invocation.display)
        {
            crate::executor::ComputerActionOutput::failed(format!(
                "computer display '{}' is not allowed by local tool policy",
                invocation.display
            ))
        } else {
            computer_config.execute_action(invocation).await
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
    let mcp_config = state.executors.read().await.mcp.clone();
    if !mcp_config.enabled() {
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

    let allowed_servers = {
        let guard = state.tool_policy.read().await;
        guard.allowed_mcp_servers.clone()
    };
    let mut outputs = Vec::new();
    for (call_id, invocation) in calls {
        let result = if !allowed_servers.is_empty()
            && !allowed_servers
                .iter()
                .any(|server| server == &invocation.server_label)
        {
            crate::executor::McpToolOutput::failed(format!(
                "MCP server '{}' is not allowed by local tool policy",
                invocation.server_label
            ))
        } else {
            mcp_config.execute_tool(invocation).await
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
        if is_supported_response_include(field) || is_ignored_response_include(field) {
            continue;
        }
        return Some(unsupported_param(
            "include",
            &format!("include field '{field}' is not supported by this relay"),
        ));
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

fn is_ignored_response_include(field: &str) -> bool {
    matches!(
        field,
        "reasoning.encrypted_content"
            | "output[*].reasoning.encrypted_content"
            | "reasoning.encrypted_content_summary"
            | "output[*].reasoning.encrypted_content_summary"
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

/// GET /api/tool-policy — 获取当前工具安全策略
#[allow(dead_code)]
pub async fn handle_get_tool_policy(State(state): State<AppState>) -> impl IntoResponse {
    let policy = state.tool_policy.read().await;
    Json(json!({
        "allowed_mcp_servers": &policy.allowed_mcp_servers,
        "allowed_computer_displays": &policy.allowed_computer_displays,
    }))
}

/// PUT /api/tool-policy — 更新工具安全策略（运行时可变）
#[allow(dead_code)]
pub async fn handle_put_tool_policy(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, Json<Value>)> {
    let mut policy = state.tool_policy.write().await;
    if let Some(servers) = body.get("allowed_mcp_servers").and_then(|v| v.as_array()) {
        policy.allowed_mcp_servers = servers
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }
    if let Some(displays) = body
        .get("allowed_computer_displays")
        .and_then(|v| v.as_array())
    {
        policy.allowed_computer_displays = displays
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }
    let allowed_mcp = policy.allowed_mcp_servers.join(",");
    let allowed_displays = policy.allowed_computer_displays.join(",");
    drop(policy);

    let config_path = Args::default_config_path(&state.data_dir);
    if let Some(mut args) = Args::load_from_file(&config_path) {
        args.allowed_mcp_servers = allowed_mcp.clone();
        args.allowed_computer_displays = allowed_displays.clone();
        if let Err(e) = args.save_to_file(&config_path) {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("保存配置失败: {}", e)})),
            ));
        }
    }

    let policy = state.tool_policy.read().await;
    Ok(Json(json!({
        "ok": true,
        "allowed_mcp_servers": &policy.allowed_mcp_servers,
        "allowed_computer_displays": &policy.allowed_computer_displays,
    })))
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

    fn official_codex_account(id: &str, priority: i64, weight: u32) -> Account {
        let mut account: Account = serde_json::from_value(json!({
            "id": id,
            "name": format!("Codex {id}"),
            "provider": "codex",
            "client_kind": "codex",
            "upstream": "https://chatgpt.com/backend-api/codex",
            "api_key": format!("token-{id}"),
            "auth_mode": "oauth",
            "endpoints": [{
                "id": format!("ep-{id}"),
                "name": "Codex 官方",
                "kind": "codex_official",
                "base_url": "https://chatgpt.com/backend-api/codex",
                "model_map": {
                    "gpt-5": format!("official-{id}")
                }
            }]
        }))
        .unwrap();
        crate::accounts::set_account_routing_options(
            &mut account,
            crate::accounts::AccountRoutingOptions {
                priority,
                weight,
                ..Default::default()
            },
        );
        account
    }

    fn official_store(accounts: Vec<Account>, active_id: &str) -> AccountStore {
        AccountStore {
            version: crate::accounts::ACCOUNT_STORE_VERSION,
            accounts,
            active_id: Some(active_id.into()),
            active_account_id: Some(active_id.into()),
            active_endpoint_id: Some(format!("ep-{active_id}")),
            active_by_surface: HashMap::new(),
        }
    }

    #[test]
    fn codex_official_body_normalizes_required_fields() {
        let body = br#"{
            "model":"gpt-5",
            "instructions":null,
            "input":"hello",
            "temperature":0.2,
            "top_p":0.9,
            "user":"u1",
            "service_tier":"default"
        }"#;

        let normalized = normalize_codex_official_body(body, "gpt-5.4").unwrap();
        let value: Value = serde_json::from_slice(&normalized).unwrap();

        assert_eq!(value["model"], "gpt-5.4");
        assert_eq!(value["instructions"], "");
        assert_eq!(value["input"][0]["role"], "user");
        assert_eq!(value["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(value["input"][0]["content"][0]["text"], "hello");
        assert_eq!(value["store"], false);
        assert_eq!(value["parallel_tool_calls"], true);
        assert!(value.get("temperature").is_none());
        assert!(value.get("top_p").is_none());
        assert!(value.get("user").is_none());
        assert!(value.get("service_tier").is_none());
    }

    #[test]
    fn codex_official_body_converts_system_role_to_developer() {
        let body = br#"{
            "model":"gpt-5",
            "input":[
                {"type":"message","role":"system","content":"rules"},
                {"type":"message","role":"assistant","content":"ok"}
            ],
            "service_tier":"priority"
        }"#;

        let normalized = normalize_codex_official_body(body, "gpt-5").unwrap();
        let value: Value = serde_json::from_slice(&normalized).unwrap();

        assert_eq!(value["input"][0]["role"], "developer");
        assert_eq!(value["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(value["input"][1]["content"][0]["type"], "output_text");
        assert_eq!(value["service_tier"], "priority");
    }

    #[test]
    fn codex_official_selector_skips_cooled_account() {
        let mut cooled = official_codex_account("cooled", 100, 1);
        cooled.runtime_state.next_retry_after = Some(2_000);
        let ready = official_codex_account("ready", 10, 1);
        let store = official_store(vec![cooled, ready], "cooled");

        let (account, _) = select_codex_official_account_endpoint(
            &store,
            "gpt-5",
            1_000,
            0,
            AccountRouteSurface::Global,
        )
        .unwrap();

        assert_eq!(account.id, "ready");
    }

    #[test]
    fn codex_official_selector_uses_priority_before_weight() {
        let low = official_codex_account("low", 0, 100);
        let high = official_codex_account("high", 10, 1);
        let store = official_store(vec![low, high], "low");

        let (account, _) = select_codex_official_account_endpoint(
            &store,
            "gpt-5",
            1_000,
            0,
            AccountRouteSurface::Global,
        )
        .unwrap();

        assert_eq!(account.id, "high");
    }

    #[test]
    fn codex_official_selector_keeps_client_surfaces_separate() {
        let mut active_desktop = official_codex_account("active-desktop", 0, 1);
        active_desktop.client_surface = AccountClientSurface::Desktop;
        let mut cli_high_priority = official_codex_account("cli-high", 100, 1);
        cli_high_priority.client_surface = AccountClientSurface::Cli;
        let mut desktop_peer = official_codex_account("desktop-peer", 10, 1);
        desktop_peer.client_surface = AccountClientSurface::Desktop;
        let store = official_store(
            vec![active_desktop, cli_high_priority, desktop_peer],
            "active-desktop",
        );

        let (account, _) = select_codex_official_account_endpoint(
            &store,
            "gpt-5",
            1_000,
            0,
            AccountRouteSurface::Global,
        )
        .unwrap();

        assert_eq!(account.id, "desktop-peer");
        assert_eq!(account.client_surface, AccountClientSurface::Desktop);
    }

    #[test]
    fn codex_official_selector_uses_requested_route_surface_anchor() {
        let mut cli = official_codex_account("cli", 100, 1);
        cli.client_surface = AccountClientSurface::Cli;
        let mut desktop = official_codex_account("desktop", 10, 1);
        desktop.client_surface = AccountClientSurface::Desktop;
        let mut store = official_store(vec![cli, desktop], "cli");
        store.set_active_for_surface(
            &AccountClientKind::Codex,
            &AccountClientSurface::Desktop,
            "desktop".into(),
            Some("ep-desktop".into()),
        );

        let (account, _) = select_codex_official_account_endpoint(
            &store,
            "gpt-5",
            1_000,
            0,
            AccountRouteSurface::CodexDesktop,
        )
        .unwrap();

        assert_eq!(account.id, "desktop");
        assert_eq!(account.client_surface, AccountClientSurface::Desktop);
    }

    #[test]
    fn codex_official_selector_returns_none_when_none_ready() {
        let mut active = official_codex_account("active", 0, 1);
        active.runtime_state.next_retry_after = Some(2_000);
        let mut other = official_codex_account("other", 0, 1);
        other.runtime_state.model_states.insert(
            "official-other".into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::QuotaExceeded,
                status_message: "HTTP 429".into(),
                next_retry_after: Some(2_000),
                quota: crate::accounts::AccountQuotaState {
                    exceeded: true,
                    reason: "quota".into(),
                    next_recover_at: Some(2_000),
                    backoff_level: 1,
                },
                updated_at: 1_000,
            },
        );
        let store = official_store(vec![active, other], "active");

        let selected = select_codex_official_account_endpoint(
            &store,
            "gpt-5",
            1_000,
            0,
            AccountRouteSurface::Global,
        );

        assert!(selected.is_none());
    }

    #[test]
    fn codex_official_originator_matches_client_surface() {
        let mut account = official_codex_account("surface", 0, 1);
        account.client_surface = AccountClientSurface::Cli;
        assert_eq!(codex_official_originator(&account), "codex_cli_rs");

        account.client_surface = AccountClientSurface::Desktop;
        assert_eq!(codex_official_originator(&account), "codex_desktop");
    }

    #[test]
    fn codex_usage_retry_after_reads_official_reset_body() {
        let retry = codex_usage_retry_after_secs(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":{"type":"usage_limit_reached","resets_in_seconds":77}}"#,
            None,
        );

        assert_eq!(retry, Some(77));
    }

    #[test]
    fn codex_usage_retry_after_prefers_header() {
        let retry = codex_usage_retry_after_secs(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":{"type":"usage_limit_reached","resets_in_seconds":77}}"#,
            Some(30),
        );

        assert_eq!(retry, Some(30));
    }

    #[test]
    fn test_fast_service_tier_injects_only_for_openai_responses() {
        let mut req = ResponsesRequest {
            model: "gpt-5.4".into(),
            input: ResponsesInput::Text("hi".into()),
            previous_response_id: None,
            tools: vec![],
            stream: false,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            system: None,
            instructions: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("high".into()),
                summary: Some("auto".into()),
            }),
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
        let endpoint = EndpointConfig {
            id: "ep".into(),
            name: "GPT".into(),
            kind: EndpointKind::OpenAiResponses,
            base_url: "https://api.openai.com/v1".into(),
            path: String::new(),
            template_id: String::new(),
            template_version: 1,
            model_map: Default::default(),
            model_profiles: Default::default(),
            vision: Default::default(),
            custom_headers: Default::default(),
            request_timeout_secs: None,
            max_retries: None,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            fast_mode_enabled: true,
            fast_service_tier: "fast".into(),
            balance_url: String::new(),
        };

        let status = apply_endpoint_fast_service_tier(&mut req, &endpoint);

        assert_eq!(status, FastServiceTierStatus::Injected);
        assert_eq!(req.service_tier.as_deref(), Some("priority"));
        assert_eq!(
            req.reasoning.as_ref().and_then(|r| r.effort.as_deref()),
            Some("high")
        );

        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hi",
                "reasoning": {"effort": "high", "summary": "auto"}
            }))
            .unwrap(),
        );
        let patched =
            patch_body_string_field(&body, "service_tier", req.service_tier.as_deref().unwrap())
                .unwrap();
        let actual: Value = serde_json::from_slice(&patched).unwrap();
        assert_eq!(actual["service_tier"], "priority");
        assert_eq!(actual["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_fast_service_tier_keeps_priority_value() {
        let mut req = ResponsesRequest {
            model: "gpt-5.4".into(),
            input: ResponsesInput::Text("hi".into()),
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
        let endpoint = EndpointConfig {
            id: "ep".into(),
            name: "GPT".into(),
            kind: EndpointKind::OpenAiResponses,
            base_url: "https://api.openai.com/v1".into(),
            path: String::new(),
            template_id: String::new(),
            template_version: 1,
            model_map: Default::default(),
            model_profiles: Default::default(),
            vision: Default::default(),
            custom_headers: Default::default(),
            request_timeout_secs: None,
            max_retries: None,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            fast_mode_enabled: true,
            fast_service_tier: "priority".into(),
            balance_url: String::new(),
        };

        let status = apply_endpoint_fast_service_tier(&mut req, &endpoint);

        assert_eq!(status, FastServiceTierStatus::Injected);
        assert_eq!(req.service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn test_fast_service_tier_does_not_inject_for_chat_endpoint() {
        let mut req = ResponsesRequest {
            model: "gpt-5.4".into(),
            input: ResponsesInput::Text("hi".into()),
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
        let endpoint = EndpointConfig {
            id: "ep".into(),
            name: "Chat".into(),
            kind: EndpointKind::OpenAiChat,
            base_url: "https://api.openai.com/v1".into(),
            path: String::new(),
            template_id: String::new(),
            template_version: 1,
            model_map: Default::default(),
            model_profiles: Default::default(),
            vision: Default::default(),
            custom_headers: Default::default(),
            request_timeout_secs: None,
            max_retries: None,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            fast_mode_enabled: true,
            fast_service_tier: "fast".into(),
            balance_url: String::new(),
        };

        let status = apply_endpoint_fast_service_tier(&mut req, &endpoint);

        assert_eq!(status, FastServiceTierStatus::Skipped);
        assert_eq!(req.service_tier, None);
    }

    #[test]
    fn test_extract_bypass_response_observations_reads_service_tier() {
        let body = br#"event: response.completed
data: {"type":"response.completed","response":{"service_tier":"priority","usage":{"input_tokens":12,"input_tokens_details":{"cached_tokens":0},"output_tokens":3,"total_tokens":15}}}

data: [DONE]
"#;

        let (input_tokens, output_tokens, cache_hit, service_tier) =
            extract_bypass_response_observations(body);

        assert_eq!(input_tokens, 12);
        assert_eq!(output_tokens, 3);
        assert!(!cache_hit);
        assert_eq!(service_tier.as_deref(), Some("priority"));
    }

    #[test]
    fn test_extract_bypass_response_observations_prefers_completed_service_tier() {
        let body = br#"event: response.created
data: {"type":"response.created","response":{"service_tier":"auto"}}

event: response.in_progress
data: {"type":"response.in_progress","response":{"service_tier":"auto"}}

event: response.completed
data: {"type":"response.completed","response":{"service_tier":"default","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}

data: [DONE]
"#;

        let (_, _, _, service_tier) = extract_bypass_response_observations(body);

        assert_eq!(service_tier.as_deref(), Some("default"));
    }

    #[test]
    fn test_proxy_token_from_headers_accepts_bearer_and_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_static("Bearer dee-token"),
        );
        headers.insert("x-api-key", header::HeaderValue::from_static("fallback"));
        assert_eq!(
            proxy_token_from_headers(&headers).as_deref(),
            Some("dee-token")
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", header::HeaderValue::from_static("dee-api-key"));
        assert_eq!(
            proxy_token_from_headers(&headers).as_deref(),
            Some("dee-api-key")
        );
    }

    #[test]
    fn test_strip_claude_code_anthropic_attribution_string_system() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "claude-sonnet-4",
                "system": "x-anthropic-billing-header: cc_version=2.1.38; cc_entrypoint=cli; cch=abc12;\n\n你是有帮助的助手。\n\n请保持简洁。",
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .unwrap(),
        );

        let stripped = strip_claude_code_anthropic_attribution(&body, true, &[]).unwrap();
        let actual: Value = serde_json::from_slice(&stripped).unwrap();

        assert_eq!(actual["system"], "你是有帮助的助手。\n\n请保持简洁。");
        assert_eq!(actual["messages"][0]["content"], "hi");
    }

    #[test]
    fn test_strip_claude_code_anthropic_attribution_array_system() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "claude-sonnet-4",
                "system": [
                    {
                        "type": "text",
                        "text": "规则 A\n\nx-anthropic-billing-header: cc_version=2.1.38; cc_entrypoint=cli; cch=fff00;\n\n规则 B"
                    },
                    {"type": "text", "text": "规则 C"},
                    {"type": "cache_control", "cache_control": {"type": "ephemeral"}}
                ],
                "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]
            }))
            .unwrap(),
        );

        let stripped = strip_claude_code_anthropic_attribution(&body, true, &[]).unwrap();
        let actual: Value = serde_json::from_slice(&stripped).unwrap();

        assert_eq!(actual["system"][0]["text"], "规则 A\n\n规则 B");
        assert_eq!(actual["system"][1]["text"], "规则 C");
        assert_eq!(
            actual["system"][2]["cache_control"]["type"].as_str(),
            Some("ephemeral")
        );
    }

    #[test]
    fn test_sanitize_client_proxy_body_skips_non_anthropic_endpoints() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5",
                "system": "x-anthropic-billing-header: cc_version=2.1.38; cch=abc12;\n真实提示词"
            }))
            .unwrap(),
        );

        let account: Account = serde_json::from_value(json!({
            "id": "cc1",
            "name": "Claude",
            "provider": "anthropic",
            "client_kind": "claude_code",
            "upstream": "https://api.anthropic.com",
            "api_key": "sk-test"
        }))
        .unwrap();
        let sanitized = sanitize_client_proxy_body("openai_chat", &account, body.clone());

        assert_eq!(sanitized, body);
    }

    #[test]
    fn test_sanitize_client_proxy_body_respects_disabled_cch_filter() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "claude-sonnet-4",
                "system": "x-anthropic-billing-header: cc_version=2.1.38; cch=abc12;\n真实提示词"
            }))
            .unwrap(),
        );

        let account: Account = serde_json::from_value(json!({
            "id": "cc1",
            "name": "Claude",
            "provider": "anthropic",
            "client_kind": "claude_code",
            "upstream": "https://api.anthropic.com",
            "api_key": "sk-test",
            "client_options": {
                "claude_cch_filter_enabled": false
            }
        }))
        .unwrap();
        let sanitized = sanitize_client_proxy_body("anthropic_messages", &account, body.clone());

        assert_eq!(sanitized, body);
    }

    #[test]
    fn test_strip_claude_code_anthropic_attribution_keeps_text_without_cch() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "claude-sonnet-4",
                "system": "x-anthropic-billing-header: cc_version=2.1.38; cc_entrypoint=cli;\n真实提示词"
            }))
            .unwrap(),
        );

        let stripped = strip_claude_code_anthropic_attribution(&body, true, &[]).unwrap();

        assert_eq!(stripped, body);
    }

    #[test]
    fn test_strip_claude_code_anthropic_attribution_respects_cch_switch() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "claude-sonnet-4",
                "system": "x-anthropic-billing-header: cc_version=2.1.38; cch=abc12;\n真实提示词"
            }))
            .unwrap(),
        );

        let stripped = strip_claude_code_anthropic_attribution(&body, false, &[]).unwrap();

        assert_eq!(stripped, body);
    }

    #[test]
    fn test_strip_claude_code_anthropic_attribution_applies_custom_rules() {
        let body = axum::body::Bytes::from(
            serde_json::to_vec(&json!({
                "model": "claude-sonnet-4",
                "system": "保留 A\nx-custom-cache-noise: random-123\n保留 B"
            }))
            .unwrap(),
        );

        let stripped =
            strip_claude_code_anthropic_attribution(&body, true, &["x-custom-cache-noise:".into()])
                .unwrap();
        let actual: Value = serde_json::from_slice(&stripped).unwrap();

        assert_eq!(actual["system"], "保留 A\n保留 B");
    }

    #[test]
    fn test_extract_proxy_response_usage_reads_json_and_sse_usage() {
        let (input, output, cache_hit) = extract_proxy_response_usage(
            br#"{"usage":{"prompt_tokens":12,"completion_tokens":4}}"#,
        );
        assert_eq!((input, output, cache_hit), (12, 4, false));

        let body = br#"event: message_delta
data: {"usage":{"input_tokens":5,"output_tokens":1}}

event: message_stop
data: {"usage":{"input_tokens":9,"output_tokens":3}}

data: [DONE]
"#;
        let (input, output, cache_hit) = extract_proxy_response_usage(body);
        assert_eq!((input, output, cache_hit), (9, 3, false));
    }

    #[test]
    fn test_sse_usage_observation_reads_chunked_cache_hit() {
        let mut observation = SseUsageObservation::default();
        observation.ingest(&Bytes::from_static(
            br#"event: response.completed
data: {"type":"response.completed","response":{"usage":{"input_tokens":120,"input_tokens_details":{"cached_tokens":"#,
        ));
        observation.ingest(&Bytes::from_static(
            br#"90},"output_tokens":8,"total_tokens":128}}}

data: [DONE]
"#,
        ));
        observation.finish();

        assert_eq!(observation.input_tokens, 120);
        assert_eq!(observation.output_tokens, 8);
        assert!(observation.cache_hit);
    }

    #[test]
    fn test_codex_images_request_uses_image_generation_tool() {
        let request = parse_image_api_request(
            br#"{
                "model": "gpt-image-2",
                "prompt": "draw a white DEX logo",
                "size": "1024x1024",
                "output_format": "png"
            }"#,
            ImageAction::Generate,
        )
        .ok()
        .unwrap();

        let body = build_codex_images_responses_body(&request, ImageAction::Generate).unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["model"], DEFAULT_IMAGE_MAIN_MODEL);
        assert_eq!(value["tool_choice"]["type"], "image_generation");
        assert_eq!(value["tools"][0]["type"], "image_generation");
        assert_eq!(value["tools"][0]["action"], "generate");
        assert_eq!(value["tools"][0]["model"], DEFAULT_IMAGE_TOOL_MODEL);
        assert_eq!(value["tools"][0]["size"], "1024x1024");
        assert_eq!(
            value["input"][0]["content"][0]["text"],
            "draw a white DEX logo"
        );
    }

    #[test]
    fn test_codex_image_sse_converts_to_images_response() {
        let body = br#"event: response.completed
data: {"type":"response.completed","response":{"created_at":1760000000,"output":[{"type":"image_generation_call","result":"QUJD","revised_prompt":"DEX mark","output_format":"png","size":"1024x1024"}],"tool_usage":{"image_gen":{"input_tokens":10,"output_tokens":1}}}}

data: [DONE]
"#;

        let value = images_response_from_codex_sse(body, "b64_json").unwrap();

        assert_eq!(value["created"], 1760000000);
        assert_eq!(value["data"][0]["b64_json"], "QUJD");
        assert_eq!(value["data"][0]["revised_prompt"], "DEX mark");
        assert_eq!(value["size"], "1024x1024");
        assert_eq!(value["usage"]["input_tokens"], 10);
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
    fn test_ignored_include_fields_do_not_trigger_error() {
        assert!(validate_response_include(Some(&[
            "reasoning.encrypted_content".to_string(),
            "output[*].reasoning.encrypted_content".to_string(),
            "reasoning.encrypted_content_summary".to_string(),
            "output[*].reasoning.encrypted_content_summary".to_string(),
        ]))
        .is_none());
    }

    #[test]
    fn test_mixed_include_accepted_with_ignore() {
        assert!(validate_response_include(Some(&[
            "file_search_call.results".to_string(),
            "reasoning.encrypted_content".to_string(),
            "usage".to_string(),
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
    fn test_count_value_tokens_accepts_partial_body() {
        let tokens = count_value_tokens(&json!({"input": "hello"}));
        assert!(tokens > 0);
    }
}
