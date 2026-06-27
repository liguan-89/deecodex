use std::collections::{HashMap, HashSet};

use crate::accounts::{
    account_routing_options, Account, AccountAuthMode, AccountClientKind, AccountClientSurface,
    AccountRoutingOptions, AccountRuntimeStatus, AccountStore, EndpointConfig, EndpointKind,
    GlueVisionStrategy, UnsupportedImagePolicy, VisionMode,
};
use crate::anthropic;
use crate::cache::RequestCache;
use crate::config::Args;
use crate::executor::{ComputerActionInvocation, LocalExecutorConfig, McpToolInvocation};
use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;
use crate::request_history::{HistoryContext, HistoryRecord};
use crate::runtime_feedback::RuntimeFeedbackSink;
use crate::session::SessionStore;
use crate::token_anomaly::TokenTracker;
use crate::types::*;
use crate::utils::{limit_function_call_outputs, merge_response_extra};
use crate::vision::{
    build_minimax_vlm_body, handle_minimax_vlm, request_minimax_vlm_text,
    strip_images_from_chat_request, VlmArgs,
};
use crate::{
    codex_config, dev_pipeline, files, local_ocr, prompts, providers, sse::SseState, stream,
    translate, vector_stores,
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
const DEFAULT_NATIVE_HELPER_MODEL: &str = "gpt-5.5";
const GPT54_MODEL: &str = "gpt-5.4";
const GPT54_COMPUTER_FALLBACK_MODEL: &str = "gpt-5.5";
static CODEX_OFFICIAL_POOL_CURSOR: AtomicU64 = AtomicU64::new(0);
static DEX_ROUTER_POOL_CURSOR: AtomicU64 = AtomicU64::new(0);
static CODEX_DESKTOP_THREAD_NORMALIZE_AT: AtomicU64 = AtomicU64::new(0);
const DEX_ROUTER_MAX_NON_STREAM_ATTEMPTS: usize = 3;
const CODEX_DESKTOP_THREAD_NORMALIZE_INTERVAL_SECS: u64 = 15;

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
    /// DEX Router 会话级原生 Responses 轨道状态
    pub codex_router_sessions: crate::codex_router_session::RouteStateMap,
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
    runtime_feedback: RuntimeFeedbackSink,
    task_loop_guard_label: Option<String>,
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
    runtime_feedback: RuntimeFeedbackSink,
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
    runtime_feedback: RuntimeFeedbackSink,
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
    runtime_feedback: RuntimeFeedbackSink,
    retry_after_secs: Option<u64>,
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
    CodexRouter,
    DexAssistant,
}

#[derive(Clone)]
struct AccountRouteSelection {
    account: Account,
    endpoint: EndpointConfig,
    route_trace: Option<String>,
    requires_computer: bool,
    strip_native_computer_toolchain: bool,
    explicit_model: Option<String>,
    explicit_account_model: bool,
    session_main_model_anchor: bool,
    main_model_anchor_to_record: Option<crate::codex_router_session::MainModelAnchor>,
}

#[derive(Clone, Debug)]
struct CodexRouterExternalAnchor {
    account_id: Option<String>,
}

#[derive(Clone, Debug)]
struct DexRouterSessionRouteDecision {
    key: String,
    state: &'static str,
    reason: String,
    observe_remaining: u8,
    expires_at: Option<u64>,
    force_native_responses: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum NativeRouteSignal {
    #[default]
    None,
    WeakContinuation,
    TextIntent,
    StrongNative,
}

impl NativeRouteSignal {
    fn as_str(self) -> &'static str {
        match self {
            NativeRouteSignal::None => "none",
            NativeRouteSignal::WeakContinuation => "weak_continuation",
            NativeRouteSignal::TextIntent => "text_intent",
            NativeRouteSignal::StrongNative => "strong_native",
        }
    }
}

struct DexRouterCandidate {
    account: Account,
    endpoint: EndpointConfig,
    priority: i64,
    weight: u32,
    mapped_model: String,
}

struct RouterAttemptFailure {
    status: StatusCode,
    code: String,
    message: String,
    body: Bytes,
    parts: axum::http::response::Parts,
}

#[derive(Clone, Copy)]
struct DexRouterTraceExclusion<'a> {
    account_ids: &'a [String],
    reason: &'static str,
}

impl<'a> DexRouterTraceExclusion<'a> {
    fn none() -> Self {
        Self {
            account_ids: &[],
            reason: "excluded",
        }
    }

    fn new(account_ids: &'a [String], reason: &'static str) -> Self {
        Self {
            account_ids,
            reason,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct RouterToolRequirements {
    tool_count: usize,
    has_function_tools: bool,
    has_web_search: bool,
    has_file_search: bool,
    has_mcp: bool,
    has_computer: bool,
    has_image_generation: bool,
    requires_function_tools: bool,
    requires_web_search: bool,
    requires_file_search: bool,
    requires_mcp: bool,
    requires_computer: bool,
    requires_image_generation: bool,
    requires_unknown_tools: bool,
    has_unknown_tools: bool,
    native_signal: NativeRouteSignal,
    labels: Vec<String>,
}

impl RouterToolRequirements {
    fn has_tools(&self) -> bool {
        self.tool_count > 0 || self.has_required_tools()
    }

    fn has_required_tools(&self) -> bool {
        self.requires_function_tools
            || self.requires_web_search
            || self.requires_file_search
            || self.requires_mcp
            || self.requires_computer
            || self.requires_image_generation
            || self.requires_unknown_tools
    }

    fn add_label(&mut self, label: impl Into<String>) {
        let label = label.into();
        if !self.labels.iter().any(|seen| seen == &label) {
            self.labels.push(label);
        }
    }

    fn set_native_signal(&mut self, signal: NativeRouteSignal, label: impl Into<String>) {
        self.native_signal = self.native_signal.max(signal);
        self.has_computer = true;
        if signal >= NativeRouteSignal::StrongNative {
            self.requires_computer = true;
        }
        self.add_label(label);
    }

    fn force_native_observe(&mut self) {
        self.native_signal = self.native_signal.max(NativeRouteSignal::WeakContinuation);
        self.has_computer = true;
        self.requires_computer = true;
        self.add_label("session.native_observe");
    }

    fn is_weak_computer_intent(&self) -> bool {
        self.has_computer && self.native_signal == NativeRouteSignal::TextIntent
    }
}

impl AccountRouteSurface {
    fn explicit_surface(self) -> Option<AccountClientSurface> {
        match self {
            AccountRouteSurface::Global => None,
            AccountRouteSurface::CodexCli => Some(AccountClientSurface::Cli),
            AccountRouteSurface::CodexDesktop => Some(AccountClientSurface::Desktop),
            AccountRouteSurface::CodexRouter => Some(AccountClientSurface::Desktop),
            AccountRouteSurface::DexAssistant => None,
        }
    }

    fn responses_path(self) -> &'static str {
        match self {
            AccountRouteSurface::Global => "/v1/responses",
            AccountRouteSurface::CodexCli => "/codex-cli/v1/responses",
            AccountRouteSurface::CodexDesktop => "/codex-desktop/v1/responses",
            AccountRouteSurface::CodexRouter => "/codex-router/v1/responses",
            AccountRouteSurface::DexAssistant => "/dex-assistant/v1/responses",
        }
    }

    fn uses_codex_direct_models(self) -> bool {
        matches!(
            self,
            AccountRouteSurface::CodexCli
                | AccountRouteSurface::CodexDesktop
                | AccountRouteSurface::CodexRouter
        )
    }
}

fn is_sensitive_router_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "authorization"
        || name == "cookie"
        || name == "set-cookie"
        || name == "x-api-key"
        || name == "chatgpt-account-id"
        || name == "session_id"
        || name.contains("token")
        || name.contains("secret")
        || name.contains("credential")
}

fn truncate_router_header_value(value: &str) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(160).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn masked_router_headers(headers: &HeaderMap) -> Value {
    let mut out = serde_json::Map::new();
    for (name, value) in headers {
        let name = name.as_str();
        let value = if is_sensitive_router_header(name) {
            "<redacted>".to_string()
        } else {
            value
                .to_str()
                .map(truncate_router_header_value)
                .unwrap_or_else(|_| "<non-utf8>".to_string())
        };
        out.insert(name.to_string(), Value::String(value));
    }
    Value::Object(out)
}

fn codex_router_external_anchor_from_headers(
    headers: &HeaderMap,
) -> Option<CodexRouterExternalAnchor> {
    let account_id = headers
        .get("chatgpt-account-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let has_bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .is_some_and(|value| {
            let lower = value.to_ascii_lowercase();
            lower.starts_with("bearer ") && value.len() > "bearer ".len()
        });
    (has_bearer || account_id.is_some()).then_some(CodexRouterExternalAnchor { account_id })
}

fn codex_router_pool_unavailable_response(
    message: impl Into<String>,
    code: &'static str,
) -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": {
                "message": message.into(),
                "type": "configuration_error",
                "code": code
            }
        })),
    )
        .into_response()
}

fn effective_codex_router_anchor(store: &AccountStore) -> Option<&Account> {
    store
        .active_account_for_surface(&AccountClientSurface::Desktop)
        .filter(|account| {
            account_routing_options(account).effective_anchor_enabled_for_account(account)
        })
}

fn codex_router_store_with_external_anchor(
    mut store: AccountStore,
    external_anchor: Option<&CodexRouterExternalAnchor>,
) -> AccountStore {
    if effective_codex_router_anchor(&store).is_some() || external_anchor.is_none() {
        return store;
    }

    let external_anchor = external_anchor.expect("checked above");
    let anchor_id = "__codex_desktop_login_anchor__";
    let display_suffix = external_anchor
        .account_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map(|id| format!(" · {id}"))
        .unwrap_or_default();
    let mut account: Account = serde_json::from_value(json!({
        "id": anchor_id,
        "name": format!("Codex Desktop 登录态{}", display_suffix),
        "provider": "codex",
        "client_kind": "codex",
        "client_surface": "desktop",
        "upstream": CODEX_OFFICIAL_BASE_URL,
        "api_key": "",
        "auth_mode": "oauth",
        "endpoints": []
    }))
    .expect("synthetic external Codex anchor should deserialize");
    crate::accounts::set_account_routing_options(
        &mut account,
        AccountRoutingOptions {
            enabled: true,
            anchor_enabled: Some(true),
            execution_enabled: Some(false),
            pool: "codex-official".into(),
            disabled: false,
            ..Default::default()
        },
    );

    store.accounts.retain(|account| account.id != anchor_id);
    store.accounts.push(account);
    store.set_active_for_surface(
        &AccountClientKind::Codex,
        &AccountClientSurface::Desktop,
        anchor_id.to_string(),
        None,
    );
    store
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

async fn refresh_account_store_from_disk(state: &AppState) {
    let store = match crate::accounts::load_accounts_checked(state.data_dir.as_ref()) {
        Ok(store) => store,
        Err(err) => {
            warn!("刷新运行态账号库失败，继续使用内存账号库: {err:#}");
            return;
        }
    };
    if !store
        .accounts
        .iter()
        .any(|account| account.client_kind.is_codex())
    {
        return;
    }

    let mut active_account = store.active_account().cloned();
    let active_endpoint = active_account.as_ref().and_then(|account| {
        account
            .active_endpoint(store.active_endpoint_id.as_deref())
            .or_else(|| account.endpoints.first())
            .cloned()
    });
    if let (Some(account), Some(endpoint)) = (active_account.as_mut(), active_endpoint.as_ref()) {
        account.sync_legacy_from_endpoint(endpoint);
    }

    *state.account_store.write().await = store;
    if let Some(account) = active_account {
        *state.active_account.write().await = account.clone();
        *state.api_key.write().await = account.api_key.clone();
    }
    if let Some(endpoint) = active_endpoint {
        if let Ok(upstream) = validate_upstream(&endpoint.base_url) {
            *state.upstream.write().await = upstream;
        }
        *state.model_map.write().await = endpoint.model_map.clone();
        *state.vision_upstream.write().await = if endpoint.vision.base_url.trim().is_empty() {
            None
        } else {
            match validate_upstream(&endpoint.vision.base_url) {
                Ok(url) => Some(url),
                Err(err) => {
                    warn!("刷新账号视觉上游失败，忽略视觉上游: {err}");
                    None
                }
            }
        };
        *state.vision_api_key.write().await = endpoint.vision.api_key.clone();
        if !endpoint.vision.model.trim().is_empty() {
            *state.vision_model.write().await = endpoint.vision.model.clone();
        }
        if !endpoint.vision.path.trim().is_empty() {
            *state.vision_endpoint.write().await = endpoint.vision.path.clone();
        }
        *state.reasoning_effort_override.write().await = endpoint.reasoning_effort_override.clone();
        *state.thinking_tokens.write().await = endpoint.thinking_tokens;
        *state.custom_headers.write().await = endpoint.custom_headers.clone();
        *state.request_timeout_secs.write().await = endpoint.request_timeout_secs;
    }
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
    requested_model: Option<&str>,
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

    if route_surface == AccountRouteSurface::CodexRouter {
        let store = state.account_store.read().await.clone();
        let cursor = DEX_ROUTER_POOL_CURSOR.fetch_add(1, Ordering::Relaxed);
        if let Some((account, endpoint)) = select_dex_router_account_endpoint(
            &store,
            requested_model.unwrap_or(""),
            crate::accounts::now_secs(),
            cursor,
            None,
        ) {
            tracing::info!(
                account_id = %account.id,
                account_name = %account.name,
                endpoint_id = %endpoint.id,
                endpoint_kind = ?endpoint.kind,
                requested_model = %requested_model.unwrap_or(""),
                "DEX Router 已选择执行账号"
            );
            return (account, endpoint);
        }
        drop(store);

        warn!("DEX Router 未找到可用执行账号，回退到 Codex 桌面版活跃账号");
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

async fn resolve_account_endpoint_for_response(
    state: &AppState,
    route_surface: AccountRouteSurface,
    requested_model: &str,
    tool_requirements: Option<&RouterToolRequirements>,
    external_anchor: Option<&CodexRouterExternalAnchor>,
    session_main_model_anchor: Option<&crate::codex_router_session::MainModelAnchor>,
) -> Result<AccountRouteSelection, Response> {
    refresh_account_store_from_disk(state).await;

    if codex_config::decode_dex_account_model_slug(requested_model).is_some() {
        let store = state.account_store.read().await.clone();
        let store = if route_surface == AccountRouteSurface::CodexRouter {
            codex_router_store_with_external_anchor(store, external_anchor)
        } else {
            store
        };
        if let Some(selection) =
            resolve_explicit_dex_account_model_selection(&store, requested_model, tool_requirements)
                .map_err(|response| *response)?
        {
            return Ok(selection);
        }
    }

    if route_surface == AccountRouteSurface::CodexRouter
        && codex_router_native_direct_model(requested_model)
        && !tool_requirements.is_some_and(|requirements| {
            requirements.native_signal >= NativeRouteSignal::StrongNative
        })
    {
        if let Some(anchor) = session_main_model_anchor {
            let store = state.account_store.read().await.clone();
            let store = codex_router_store_with_external_anchor(store, external_anchor);
            return resolve_session_main_model_anchor_selection(
                &store,
                requested_model,
                anchor,
                tool_requirements,
            )
            .map_err(|response| *response);
        }
    }

    if route_surface != AccountRouteSurface::CodexRouter {
        let (account, endpoint) =
            active_account_endpoint_for_route(state, route_surface, Some(requested_model)).await;
        return Ok(AccountRouteSelection {
            account,
            endpoint,
            route_trace: None,
            requires_computer: false,
            strip_native_computer_toolchain: false,
            explicit_model: None,
            explicit_account_model: false,
            session_main_model_anchor: false,
            main_model_anchor_to_record: None,
        });
    }

    let store = state.account_store.read().await.clone();
    let store = codex_router_store_with_external_anchor(store, external_anchor);
    let Some(anchor) = store.active_account_for_surface(&AccountClientSurface::Desktop) else {
        return Err(codex_router_pool_unavailable_response(
            "DEX Router 未配置 Codex 桌面版锚点账号，请先在账号管理中为 Codex 桌面版选择一个账号",
            "dex_router_missing_desktop_anchor",
        ));
    };
    let anchor_routing = account_routing_options(anchor);
    if !anchor_routing.effective_anchor_enabled_for_account(anchor) {
        return Err(codex_router_pool_unavailable_response(
            "DEX Router 的 Codex 桌面版锚点账号已关闭登录态锚定，请在账号管理中重新开启",
            "dex_router_anchor_disabled",
        ));
    }
    let anchor_pool = anchor_routing.pool;
    let cursor = DEX_ROUTER_POOL_CURSOR.fetch_add(1, Ordering::Relaxed);
    let now = crate::accounts::now_secs();
    let (selection, route_trace) =
        dex_router_trace_for_selection(&store, requested_model, now, cursor, tool_requirements);
    match selection {
        Some((account, endpoint)) => {
            let effective_model = router_effective_model_for_account(&account, requested_model, &endpoint);
            Ok(AccountRouteSelection {
                account,
                endpoint,
                route_trace: Some(route_trace),
                requires_computer: tool_requirements.is_some_and(|requirements| {
                    requirements.requires_computer
                }),
                strip_native_computer_toolchain: false,
                explicit_model: (effective_model != requested_model).then_some(effective_model),
                explicit_account_model: false,
                session_main_model_anchor: false,
                main_model_anchor_to_record: None,
            })
        }
        None => Err(codex_router_pool_unavailable_response(
            format!(
                "DEX Router 在账号池「{}」中没有找到可用于模型 {} 的执行账号：请检查账号池启用状态、模型选择和冷却状态",
                anchor_pool, requested_model
            ),
            "dex_router_pool_unavailable",
        )),
    }
}

fn router_effective_model_for_account(
    account: &Account,
    requested_model: &str,
    endpoint: &EndpointConfig,
) -> String {
    effective_model_for_chat_account(account, requested_model, endpoint, true)
}

fn translated_chat_effective_model_for_account(
    account: &Account,
    requested_model: &str,
    endpoint: &EndpointConfig,
) -> String {
    effective_model_for_chat_account(account, requested_model, endpoint, false)
}

fn effective_model_for_chat_account(
    account: &Account,
    requested_model: &str,
    endpoint: &EndpointConfig,
    keep_native_gpt_models: bool,
) -> String {
    let requested_model = requested_model.trim();
    if !endpoint.kind.is_chat_like() {
        return requested_model.to_string();
    }
    if keep_native_gpt_models && codex_router_native_direct_model(requested_model) {
        return requested_model.to_string();
    }
    if let Some(mapped) = endpoint
        .model_map
        .get(requested_model)
        .or_else(|| account.model_map.get(requested_model))
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
    {
        return mapped.to_string();
    }
    if endpoint
        .known_models
        .iter()
        .any(|model| model.trim() == requested_model)
    {
        return requested_model.to_string();
    }
    let default_model = account.default_model.trim();
    if !default_model.is_empty() {
        // 兜底前先校验 default_model 是否适配当前 endpoint：
        // 第三方上游不识别 Codex 默认 gpt-5.4-mini 等模型，会 400 杀流。
        // 若 default_model 不在 endpoint.known_models 且 known_models 非空，
        // 强制走 known_models[0]（该上游真实支持的能力）而非 default_model。
        if endpoint
            .known_models
            .iter()
            .any(|model| model.trim() == default_model)
        {
            return default_model.to_string();
        }
    }
    endpoint
        .known_models
        .iter()
        .map(|model| model.trim())
        .find(|model| !model.is_empty())
        .unwrap_or(requested_model)
        .to_string()
}

fn router_effective_model(requested_model: &str, endpoint: &EndpointConfig) -> String {
    let requested_model = requested_model.trim();
    if !endpoint.kind.is_chat_like() {
        return requested_model.to_string();
    }
    if endpoint
        .known_models
        .iter()
        .any(|model| model.trim() == requested_model)
    {
        return requested_model.to_string();
    }
    endpoint
        .known_models
        .iter()
        .map(|model| model.trim())
        .find(|model| !model.is_empty())
        .unwrap_or(requested_model)
        .to_string()
}

fn model_recent_transient_upstream_error(
    account: &Account,
    endpoint: &EndpointConfig,
    model: &str,
    now: u64,
) -> bool {
    if !endpoint_uses_dex_managed_runtime_cooldown(account, endpoint) {
        return false;
    }
    account
        .runtime_state
        .model_states
        .get(model)
        .is_some_and(|state| {
            state.updated_at.saturating_add(10 * 60) >= now
                && runtime_state_is_transient_upstream_error(&state.status, &state.status_message)
        })
}

fn native_helper_model_candidates(selected_model: &str) -> Vec<String> {
    let mut models = Vec::new();
    let push_model = |models: &mut Vec<String>, model: String| {
        if !models.iter().any(|seen| seen == &model) {
            models.push(model);
        }
    };
    push_model(&mut models, explicit_native_helper_model(selected_model));
    if selected_model != GPT54_MODEL && codex_router_native_direct_model(selected_model) {
        push_model(&mut models, selected_model.to_string());
    }
    push_model(&mut models, DEFAULT_NATIVE_HELPER_MODEL.into());
    models
}

fn gpt54_fallback_trace_event(
    requested_model: &str,
    fallback_model: &str,
    account: &Account,
    endpoint: &EndpointConfig,
    reason: &'static str,
) -> Value {
    json!({
        "action": "model_fallback",
        "reason": reason,
        "from_model": requested_model,
        "to_model": fallback_model,
        "account_id": account.id,
        "account_name": account.name,
        "endpoint_id": endpoint.id,
        "endpoint_kind": endpoint.kind.label(),
    })
}

fn patch_router_model_fallback_trace(route_trace: Option<String>, event: Value) -> Option<String> {
    let mut trace = route_trace
        .and_then(|trace| serde_json::from_str::<Value>(&trace).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = trace.as_object_mut() {
        obj.insert("model_fallback".into(), event);
    }
    serde_json::to_string(&trace).ok()
}

fn patch_route_trace_field(route_trace: Option<String>, key: &str, value: Value) -> Option<String> {
    let mut trace = route_trace
        .and_then(|trace| serde_json::from_str::<Value>(&trace).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = trace.as_object_mut() {
        obj.insert(key.into(), value);
    }
    serde_json::to_string(&trace).ok()
}

fn apply_gpt54_fallback_to_selection(
    mut selection: AccountRouteSelection,
    requested_model: &str,
    fallback_model: &str,
    reason: &'static str,
) -> AccountRouteSelection {
    let event = gpt54_fallback_trace_event(
        requested_model,
        fallback_model,
        &selection.account,
        &selection.endpoint,
        reason,
    );
    selection.route_trace = patch_router_model_fallback_trace(selection.route_trace, event);
    selection.explicit_model = Some(fallback_model.to_string());
    selection
}

fn resolve_explicit_dex_account_model_selection(
    store: &AccountStore,
    requested_model: &str,
    tool_requirements: Option<&RouterToolRequirements>,
) -> Result<Option<AccountRouteSelection>, Box<Response>> {
    let Some(model_ref) = codex_config::decode_dex_account_model_slug(requested_model) else {
        return Ok(None);
    };
    let Some(account) = store.accounts.iter().find(|account| {
        account.id == model_ref.account_id
            && account.client_kind.is_codex()
            && account.client_surface == AccountClientSurface::Desktop
    }) else {
        return Err(Box::new(codex_router_pool_unavailable_response(
            "DEX Router 直选账号不存在或不是 Codex 桌面版账号，请重新同步账号模型目录",
            "dex_router_explicit_account_missing",
        )));
    };
    let Some(endpoint) = account
        .endpoints
        .iter()
        .find(|endpoint| endpoint.id == model_ref.endpoint_id)
        .cloned()
    else {
        return Err(Box::new(codex_router_pool_unavailable_response(
            "DEX Router 直选端点不存在，请重新同步账号模型目录",
            "dex_router_explicit_endpoint_missing",
        )));
    };
    let now = crate::accounts::now_secs();
    let upstream_model = model_ref.model.clone();
    if tool_requirements.is_some_and(|requirements| {
        requirements.requires_computer
            && requirements.native_signal >= NativeRouteSignal::StrongNative
    }) && !endpoint_is_native_router_executor(&endpoint)
    {
        let routing = account_routing_options(account);
        if routing.strip_native_computer_toolchain() {
            return explicit_chat_model_strip_native_toolchain_selection(
                account,
                &endpoint,
                &model_ref.model,
                tool_requirements,
                now,
                requested_model,
            )
            .map(Some);
        }
        return resolve_explicit_chat_model_native_helper_selection(
            store,
            account,
            &endpoint,
            &model_ref.model,
            tool_requirements,
            now,
            requested_model,
        )
        .map(Some);
    }
    if let Some(block) =
        account_runtime_block_for_endpoint(account, &endpoint, &upstream_model, now)
    {
        return Err(Box::new(codex_router_pool_unavailable_response(
            format!(
                "已选择「{} / {}」，但该模型当前不可用：{}",
                account.name, upstream_model, block.reason
            ),
            "dex_router_explicit_model_runtime_blocked",
        )));
    }
    let mut account = account.clone();
    account.sync_legacy_from_endpoint(&endpoint);
    let trace = json!({
        "requested_model": requested_model,
        "explicit_model_selection": true,
        "selected_account_id": account.id,
        "selected_account_name": account.name,
        "selected_endpoint_id": endpoint.id,
        "selected_endpoint_kind": endpoint_kind_slug(&endpoint.kind),
        "selected_model": model_ref.model,
        "upstream_model": upstream_model,
    });
    let main_model_anchor_to_record = Some(crate::codex_router_session::MainModelAnchor {
        account_id: account.id.clone(),
        endpoint_id: endpoint.id.clone(),
        model: model_ref.model.clone(),
        endpoint_kind: endpoint_kind_slug(&endpoint.kind).to_string(),
    });
    Ok(Some(AccountRouteSelection {
        account,
        endpoint,
        route_trace: serde_json::to_string(&trace).ok(),
        requires_computer: tool_requirements
            .is_some_and(|requirements| requirements.requires_computer),
        strip_native_computer_toolchain: false,
        explicit_model: Some(model_ref.model),
        explicit_account_model: true,
        session_main_model_anchor: false,
        main_model_anchor_to_record,
    }))
}

fn resolve_session_main_model_anchor_selection(
    store: &AccountStore,
    requested_model: &str,
    anchor: &crate::codex_router_session::MainModelAnchor,
    tool_requirements: Option<&RouterToolRequirements>,
) -> Result<AccountRouteSelection, Box<Response>> {
    let Some(account) = store.accounts.iter().find(|account| {
        account.id == anchor.account_id
            && account.client_kind.is_codex()
            && account.client_surface == AccountClientSurface::Desktop
    }) else {
        return Err(Box::new(codex_router_pool_unavailable_response(
            "DEX Router 会话主模型账号不存在，请在 Codex 模型列表重新选择账号模型",
            "dex_router_session_anchor_account_missing",
        )));
    };
    let Some(endpoint) = account
        .endpoints
        .iter()
        .find(|endpoint| endpoint.id == anchor.endpoint_id)
        .cloned()
    else {
        return Err(Box::new(codex_router_pool_unavailable_response(
            "DEX Router 会话主模型端点不存在，请在 Codex 模型列表重新选择账号模型",
            "dex_router_session_anchor_endpoint_missing",
        )));
    };
    let now = crate::accounts::now_secs();
    if let Some(block) = account_runtime_block_for_endpoint(account, &endpoint, &anchor.model, now)
    {
        return Err(Box::new(codex_router_pool_unavailable_response(
            format!(
                "会话主模型「{} / {}」当前不可用：{}",
                account.name, anchor.model, block.reason
            ),
            "dex_router_session_anchor_runtime_blocked",
        )));
    }

    let mut account = account.clone();
    account.sync_legacy_from_endpoint(&endpoint);
    let trace = json!({
        "requested_model": requested_model,
        "session_main_model_anchor": true,
        "selected_account_id": account.id,
        "selected_account_name": account.name,
        "selected_endpoint_id": endpoint.id,
        "selected_endpoint_kind": endpoint_kind_slug(&endpoint.kind),
        "anchor_endpoint_kind": anchor.endpoint_kind,
        "selected_model": anchor.model,
        "upstream_model": anchor.model,
        "tool_requirements": router_tool_requirements_value(tool_requirements),
    });
    Ok(AccountRouteSelection {
        account,
        endpoint,
        route_trace: serde_json::to_string(&trace).ok(),
        requires_computer: false,
        strip_native_computer_toolchain: false,
        explicit_model: Some(anchor.model.clone()),
        explicit_account_model: false,
        session_main_model_anchor: true,
        main_model_anchor_to_record: None,
    })
}

fn explicit_native_helper_model(selected_model: &str) -> String {
    // 子调用跟随 Codex 桌面版当前选中的模型（包括第三方上游如 minimax-m3），
    // 不再回退到 Codex 默认 gpt-5.5（第三方上游不识别会 400）。
    // Codex 官方 gpt-5.5 路径由调用点的 native_direct_model 分支独立处理，不受此处影响。
    selected_model.to_string()
}

struct ExplicitChatFallbackContext<'a> {
    main_account: &'a Account,
    main_endpoint: &'a EndpointConfig,
    selected_model: &'a str,
    requested_model: &'a str,
    reason: &'static str,
    helper: Option<(&'a Account, &'a EndpointConfig, &'a str)>,
    skipped_helpers: Vec<Value>,
    now: u64,
}

fn resolve_explicit_chat_model_native_helper_selection(
    store: &AccountStore,
    main_account: &Account,
    main_endpoint: &EndpointConfig,
    selected_model: &str,
    tool_requirements: Option<&RouterToolRequirements>,
    now: u64,
    requested_model: &str,
) -> Result<AccountRouteSelection, Box<Response>> {
    let routing = account_routing_options(main_account);
    let cursor = DEX_ROUTER_POOL_CURSOR.fetch_add(1, Ordering::Relaxed);
    let weak_text_intent =
        tool_requirements.is_some_and(|requirements| requirements.is_weak_computer_intent());
    let mut skipped_helpers = Vec::new();
    let mut helper_selection = None;
    let mut helper_model = String::new();
    let mut last_resort_helper = None;
    let mut helper_last_resort = false;
    for candidate_model in native_helper_model_candidates(selected_model) {
        let Some((mut account, endpoint)) = select_dex_router_native_executor_in_pool(
            store,
            &routing.pool,
            &candidate_model,
            now,
            cursor,
            tool_requirements,
            Some(main_account.id.as_str()),
        ) else {
            continue;
        };
        if model_recent_transient_upstream_error(&account, &endpoint, &candidate_model, now) {
            skipped_helpers.push(json!({
                "account_id": account.id,
                "account_name": account.name,
                "endpoint_id": endpoint.id,
                "endpoint_kind": endpoint_kind_slug(&endpoint.kind),
                "model": candidate_model,
                "reason": "recent_transient_upstream_error",
            }));
            if !weak_text_intent && last_resort_helper.is_none() {
                last_resort_helper = Some((account, endpoint, candidate_model));
            }
            continue;
        }
        helper_model = candidate_model;
        account.sync_legacy_from_endpoint(&endpoint);
        helper_selection = Some((account, endpoint));
        break;
    }

    if helper_selection.is_none() {
        if let Some((mut account, endpoint, candidate_model)) = last_resort_helper {
            helper_model = candidate_model;
            helper_last_resort = true;
            account.sync_legacy_from_endpoint(&endpoint);
            helper_selection = Some((account, endpoint));
        }
    }

    let Some((helper_account, helper_endpoint)) = helper_selection else {
        if !weak_text_intent {
            return Err(Box::new(codex_router_pool_unavailable_response(
                format!(
                    "已选择「{} / {}」，但本轮包含 Computer Use 原生工具链信号，账号池「{}」中没有可用的 Responses helper；请切换到 Responses 直连或等待 helper 恢复",
                    main_account.name, selected_model, routing.pool
                ),
                "dex_router_explicit_model_native_helper_unavailable",
            )));
        }
        tracing::warn!(
            main_account_id = %main_account.id,
            main_account_name = %main_account.name,
            selected_model = %selected_model,
            pool = %routing.pool,
            "DEX Router 弱 Computer Use 意图未找到原生 helper，降级回 Chat 兼容账号"
        );
        return explicit_chat_model_native_helper_fallback_selection(ExplicitChatFallbackContext {
            main_account,
            main_endpoint,
            selected_model,
            requested_model,
            reason: "weak_intent_native_helper_unavailable",
            helper: None,
            skipped_helpers,
            now,
        });
    };
    let trace = json!({
        "requested_model": requested_model,
        "explicit_model_selection": true,
        "native_helper_reroute": true,
        "native_helper_reason": if weak_text_intent { "weak_computer_intent" } else { "strong_computer_signal" },
        "native_helper_skipped": skipped_helpers,
        "native_helper_last_resort": helper_last_resort,
        "main_account_id": main_account.id,
        "main_account_name": main_account.name,
        "main_endpoint_id": main_endpoint.id,
        "main_endpoint_kind": endpoint_kind_slug(&main_endpoint.kind),
        "main_selected_model": selected_model,
        "selected_account_id": helper_account.id,
        "selected_account_name": helper_account.name,
        "selected_endpoint_id": helper_endpoint.id,
        "selected_endpoint_kind": endpoint_kind_slug(&helper_endpoint.kind),
        "upstream_model": helper_model,
    });
    Ok(AccountRouteSelection {
        account: helper_account,
        endpoint: helper_endpoint,
        route_trace: serde_json::to_string(&trace).ok(),
        requires_computer: true,
        strip_native_computer_toolchain: false,
        explicit_model: Some(helper_model),
        explicit_account_model: true,
        session_main_model_anchor: false,
        main_model_anchor_to_record: Some(crate::codex_router_session::MainModelAnchor {
            account_id: main_account.id.clone(),
            endpoint_id: main_endpoint.id.clone(),
            model: selected_model.to_string(),
            endpoint_kind: endpoint_kind_slug(&main_endpoint.kind).to_string(),
        }),
    })
}

fn explicit_chat_model_native_helper_fallback_selection(
    context: ExplicitChatFallbackContext<'_>,
) -> Result<AccountRouteSelection, Box<Response>> {
    if let Some(block) = account_runtime_block_for_endpoint(
        context.main_account,
        context.main_endpoint,
        context.selected_model,
        context.now,
    ) {
        return Err(Box::new(codex_router_pool_unavailable_response(
            format!(
                "已选择「{} / {}」作为主账号模型，但原生 Computer Use helper 不可用，且主账号当前不可降级使用：{}",
                context.main_account.name, context.selected_model, block.reason
            ),
            "dex_router_explicit_model_native_helper_fallback_blocked",
        )));
    }
    let mut account = context.main_account.clone();
    account.sync_legacy_from_endpoint(context.main_endpoint);
    let helper = context.helper.map(|(account, endpoint, model)| {
        json!({
            "account_id": account.id,
            "account_name": account.name,
            "endpoint_id": endpoint.id,
            "endpoint_kind": endpoint_kind_slug(&endpoint.kind),
            "model": model,
        })
    });
    let trace = json!({
        "requested_model": context.requested_model,
        "explicit_model_selection": true,
        "native_helper_fallback_to_chat": true,
        "native_helper_fallback_reason": context.reason,
        "native_helper_reroute": false,
        "native_helper": helper,
        "native_helper_skipped": context.skipped_helpers,
        "main_account_id": context.main_account.id,
        "main_account_name": context.main_account.name,
        "main_endpoint_id": context.main_endpoint.id,
        "main_endpoint_kind": endpoint_kind_slug(&context.main_endpoint.kind),
        "main_selected_model": context.selected_model,
        "selected_account_id": account.id,
        "selected_account_name": account.name,
        "selected_endpoint_id": context.main_endpoint.id,
        "selected_endpoint_kind": endpoint_kind_slug(&context.main_endpoint.kind),
        "selected_model": context.selected_model,
        "upstream_model": context.selected_model,
    });
    Ok(AccountRouteSelection {
        account,
        endpoint: context.main_endpoint.clone(),
        route_trace: serde_json::to_string(&trace).ok(),
        requires_computer: false,
        strip_native_computer_toolchain: false,
        explicit_model: Some(context.selected_model.to_string()),
        explicit_account_model: true,
        session_main_model_anchor: false,
        main_model_anchor_to_record: Some(crate::codex_router_session::MainModelAnchor {
            account_id: context.main_account.id.clone(),
            endpoint_id: context.main_endpoint.id.clone(),
            model: context.selected_model.to_string(),
            endpoint_kind: endpoint_kind_slug(&context.main_endpoint.kind).to_string(),
        }),
    })
}

fn explicit_chat_model_strip_native_toolchain_selection(
    main_account: &Account,
    main_endpoint: &EndpointConfig,
    selected_model: &str,
    tool_requirements: Option<&RouterToolRequirements>,
    now: u64,
    requested_model: &str,
) -> Result<AccountRouteSelection, Box<Response>> {
    if let Some(block) =
        account_runtime_block_for_endpoint(main_account, main_endpoint, selected_model, now)
    {
        return Err(Box::new(codex_router_pool_unavailable_response(
            format!(
                "已选择「{} / {}」，但该模型当前不可用：{}",
                main_account.name, selected_model, block.reason
            ),
            "dex_router_explicit_model_runtime_blocked",
        )));
    }
    let mut account = main_account.clone();
    account.sync_legacy_from_endpoint(main_endpoint);
    let trace = json!({
        "requested_model": requested_model,
        "explicit_model_selection": true,
        "native_computer_policy": "strip_and_continue",
        "native_toolchain_stripped": true,
        "native_helper_reroute": false,
        "native_helper_fallback_reason": "account_policy_strip_and_continue",
        "selected_account_id": account.id,
        "selected_account_name": account.name,
        "selected_endpoint_id": main_endpoint.id,
        "selected_endpoint_kind": endpoint_kind_slug(&main_endpoint.kind),
        "selected_model": selected_model,
        "upstream_model": selected_model,
        "tool_requirements": router_tool_requirements_value(tool_requirements),
    });
    Ok(AccountRouteSelection {
        account,
        endpoint: main_endpoint.clone(),
        route_trace: serde_json::to_string(&trace).ok(),
        requires_computer: false,
        strip_native_computer_toolchain: true,
        explicit_model: Some(selected_model.to_string()),
        explicit_account_model: true,
        session_main_model_anchor: false,
        main_model_anchor_to_record: Some(crate::codex_router_session::MainModelAnchor {
            account_id: main_account.id.clone(),
            endpoint_id: main_endpoint.id.clone(),
            model: selected_model.to_string(),
            endpoint_kind: endpoint_kind_slug(&main_endpoint.kind).to_string(),
        }),
    })
}

fn clean_identity_label(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.into()
    } else {
        value.into()
    }
}

fn maybe_inject_explicit_model_identity(
    messages: &mut Vec<ChatMessage>,
    account: &Account,
    endpoint: &EndpointConfig,
    explicit_model: Option<&str>,
) {
    let Some(model) = explicit_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    else {
        return;
    };
    if !endpoint.kind.is_chat_like() {
        return;
    }

    let account_name = clean_identity_label(&account.name, "未命名账号");
    let endpoint_name = clean_identity_label(&endpoint.name, endpoint.kind.label());
    let provider = clean_identity_label(&account.provider, "未知供应商");
    let prompt = format!(
        "真实模型身份说明：当前请求由 DEX AI 代理到账号「{account_name}」的端点「{endpoint_name}」，真实上游模型是「{model}」（供应商：{provider}）。DEX AI 只是本地代理层。回答身份、模型名称、供应商相关问题时，请以该真实上游模型身份为准；不要自称 Codex、GPT-5 或 OpenAI 官方模型，除非真实上游模型本身就是对应官方模型。保持 Codex 的编码、调试和项目协作行为规范，但不要伪造模型身份。"
    );
    let message = ChatMessage {
        role: "system".into(),
        content: Some(Value::String(prompt)),
        reasoning_content: None,
        reasoning_details: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    };
    let insert_at = if messages
        .first()
        .is_some_and(|message| message.role == "system")
    {
        1
    } else {
        0
    };
    messages.insert(insert_at, message);
}

fn select_dex_router_native_executor_in_pool(
    store: &AccountStore,
    pool: &str,
    requested_model: &str,
    now: u64,
    cursor: u64,
    tool_requirements: Option<&RouterToolRequirements>,
    excluded_account_id: Option<&str>,
) -> Option<(Account, EndpointConfig)> {
    let mut candidates: Vec<DexRouterCandidate> = store
        .accounts
        .iter()
        .filter(|account| account.client_kind.is_codex())
        .filter(|account| account.client_surface == AccountClientSurface::Desktop)
        .filter(|account| excluded_account_id != Some(account.id.as_str()))
        .filter_map(|account| {
            let routing = account_routing_options(account);
            if !routing.effective_execution_enabled_for_account(account) || routing.pool != pool {
                return None;
            }
            let endpoint = router_endpoint_for_account(store, account, requested_model)?;
            if !endpoint_is_native_router_executor(&endpoint) {
                return None;
            }
            let mapped_model =
                router_effective_model_for_account(account, requested_model, &endpoint);
            if !account_runtime_ready_for_endpoint(account, &endpoint, &mapped_model, now) {
                return None;
            }
            let capabilities = dex_router_capability_summary(account, &endpoint, &mapped_model);
            if !dex_router_capability_gaps(&capabilities, tool_requirements).is_empty() {
                return None;
            }
            let mut account = account.clone();
            account.sync_legacy_from_endpoint(&endpoint);
            Some(DexRouterCandidate {
                account,
                endpoint,
                priority: routing.priority,
                weight: routing.weight,
                mapped_model,
            })
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.account.id.cmp(&right.account.id))
            .then_with(|| left.endpoint.id.cmp(&right.endpoint.id))
    });
    let best_priority = candidates.first().map(|candidate| candidate.priority)?;
    let top: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| candidate.priority == best_priority)
        .collect();
    let total_weight: u64 = top
        .iter()
        .map(|candidate| u64::from(candidate.weight.max(1)))
        .sum();
    if total_weight == 0 {
        return None;
    }
    let mut slot = cursor % total_weight;
    for candidate in top {
        let weight = u64::from(candidate.weight.max(1));
        if slot < weight {
            return Some((candidate.account, candidate.endpoint));
        }
        slot = slot.saturating_sub(weight);
    }
    None
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
                known_models: Vec::new(),
                model_profiles: std::collections::HashMap::new(),
                vision: Default::default(),
                image_generation_enabled: None,
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

fn select_dex_router_account_endpoint(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    cursor: u64,
    tool_requirements: Option<&RouterToolRequirements>,
) -> Option<(Account, EndpointConfig)> {
    select_dex_router_account_endpoint_excluding(
        store,
        requested_model,
        now,
        cursor,
        tool_requirements,
        &[],
    )
}

fn select_dex_router_account_endpoint_excluding(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    cursor: u64,
    tool_requirements: Option<&RouterToolRequirements>,
    excluded_account_ids: &[String],
) -> Option<(Account, EndpointConfig)> {
    let anchor = store.active_account_for_surface(&AccountClientSurface::Desktop)?;
    let anchor_routing = account_routing_options(anchor);
    if !anchor_routing.effective_anchor_enabled_for_account(anchor) {
        return None;
    }
    let anchor_pool = anchor_routing.pool;
    let native_direct_model = codex_router_native_direct_model(requested_model);
    let mut candidates: Vec<DexRouterCandidate> = store
        .accounts
        .iter()
        .filter(|account| account.client_kind.is_codex())
        .filter(|account| account.client_surface == AccountClientSurface::Desktop)
        .filter(|account| !excluded_account_ids.iter().any(|id| id == &account.id))
        .filter_map(|account| {
            let routing = account_routing_options(account);
            if !routing.effective_execution_enabled_for_account(account)
                || routing.pool != anchor_pool
            {
                return None;
            }
            let endpoint = router_endpoint_for_account(store, account, requested_model)?;
            if native_direct_model
                && !endpoint_is_native_router_executor(&endpoint)
                && router_effective_model_for_account(account, requested_model, &endpoint)
                    == requested_model
            {
                return None;
            }
            let mapped_model =
                router_effective_model_for_account(account, requested_model, &endpoint);
            if !account_runtime_ready_for_endpoint(account, &endpoint, &mapped_model, now) {
                return None;
            }
            let capabilities = dex_router_capability_summary(account, &endpoint, &mapped_model);
            if !dex_router_capability_gaps(&capabilities, tool_requirements).is_empty() {
                return None;
            }
            let mut account = account.clone();
            account.sync_legacy_from_endpoint(&endpoint);
            Some(DexRouterCandidate {
                account,
                endpoint,
                priority: routing.priority,
                weight: routing.weight,
                mapped_model,
            })
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.account.id.cmp(&right.account.id))
            .then_with(|| left.endpoint.id.cmp(&right.endpoint.id))
    });
    let best_priority = candidates.first().map(|candidate| candidate.priority)?;
    let top: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| candidate.priority == best_priority)
        .collect();
    let total_weight: u64 = top
        .iter()
        .map(|candidate| u64::from(candidate.weight.max(1)))
        .sum();
    if total_weight == 0 {
        return None;
    }
    let mut slot = cursor % total_weight;
    for candidate in top {
        let weight = u64::from(candidate.weight.max(1));
        if slot < weight {
            tracing::info!(
                account_id = %candidate.account.id,
                account_name = %candidate.account.name,
                endpoint_id = %candidate.endpoint.id,
                endpoint_kind = ?candidate.endpoint.kind,
                requested_model = %requested_model,
                mapped_model = %candidate.mapped_model,
                priority = candidate.priority,
                "DEX Router 命中账号池候选"
            );
            return Some((candidate.account, candidate.endpoint));
        }
        slot = slot.saturating_sub(weight);
    }
    None
}

#[allow(dead_code)]
pub fn dex_router_status_snapshot(store: &AccountStore, requested_model: &str, now: u64) -> Value {
    let selected = select_dex_router_account_endpoint(store, requested_model, now, 0, None);
    dex_router_snapshot_value(
        store,
        requested_model,
        now,
        0,
        selected.as_ref(),
        None,
        DexRouterTraceExclusion::none(),
    )
}

pub fn dex_router_status_snapshot_for_tools(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    tools: &[Value],
) -> Value {
    let requirements = router_tool_requirements_for_diagnostic_tools(tools);
    let requirements = requirements.has_tools().then_some(requirements);
    let requirements_ref = requirements.as_ref();
    let selected =
        select_dex_router_account_endpoint(store, requested_model, now, 0, requirements_ref);
    dex_router_snapshot_value(
        store,
        requested_model,
        now,
        0,
        selected.as_ref(),
        requirements_ref,
        DexRouterTraceExclusion::none(),
    )
}

pub fn dex_router_status_scenarios(store: &AccountStore, requested_model: &str, now: u64) -> Value {
    let scenarios: [(&str, &str, Vec<Value>); 5] = [
        ("text", "文本", Vec::new()),
        ("web", "Web", vec![json!({"type": "web_search_preview"})]),
        ("file", "文件", vec![json!({"type": "file_search"})]),
        (
            "mcp",
            "MCP",
            vec![json!({"type": "remote_mcp", "server_label": "diagnostic"})],
        ),
        (
            "native",
            "图片/电脑",
            vec![
                json!({"type": "image_generation"}),
                json!({"type": "computer_use_preview"}),
            ],
        ),
    ];
    let values = scenarios
        .into_iter()
        .map(|(id, label, tools)| {
            let mut snapshot =
                dex_router_status_snapshot_for_tools(store, requested_model, now, &tools);
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert("scenario_id".into(), json!(id));
                obj.insert("scenario_label".into(), json!(label));
            }
            snapshot
        })
        .collect::<Vec<_>>();
    json!(values)
}

fn dex_router_trace_for_selection(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    cursor: u64,
    tool_requirements: Option<&RouterToolRequirements>,
) -> (Option<(Account, EndpointConfig)>, String) {
    dex_router_trace_for_selection_excluding(
        store,
        requested_model,
        now,
        cursor,
        tool_requirements,
        &[],
        "excluded",
    )
}

fn dex_router_trace_for_selection_excluding(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    cursor: u64,
    tool_requirements: Option<&RouterToolRequirements>,
    excluded_account_ids: &[String],
    excluded_reason: &'static str,
) -> (Option<(Account, EndpointConfig)>, String) {
    let selected = select_dex_router_account_endpoint_excluding(
        store,
        requested_model,
        now,
        cursor,
        tool_requirements,
        excluded_account_ids,
    );
    let snapshot = dex_router_snapshot_value(
        store,
        requested_model,
        now,
        cursor,
        selected.as_ref(),
        tool_requirements,
        DexRouterTraceExclusion::new(excluded_account_ids, excluded_reason),
    );
    let trace = serde_json::to_string(&snapshot).unwrap_or_default();
    (selected, trace)
}

fn dex_router_snapshot_value(
    store: &AccountStore,
    requested_model: &str,
    now: u64,
    cursor: u64,
    selected: Option<&(Account, EndpointConfig)>,
    tool_requirements: Option<&RouterToolRequirements>,
    exclusion: DexRouterTraceExclusion<'_>,
) -> Value {
    let anchor = store.active_account_for_surface(&AccountClientSurface::Desktop);
    let anchor_routing = anchor.map(account_routing_options);
    let anchor_disabled = anchor
        .zip(anchor_routing.as_ref())
        .is_some_and(|(account, routing)| !routing.effective_anchor_enabled_for_account(account));
    let anchor_pool = anchor
        .zip(anchor_routing.as_ref())
        .and_then(|(account, routing)| {
            routing
                .effective_anchor_enabled_for_account(account)
                .then(|| routing.pool.clone())
        });
    let native_direct_model = codex_router_native_direct_model(requested_model);
    let candidates: Vec<Value> = store
        .accounts
        .iter()
        .filter(|account| account.client_kind.is_codex())
        .filter(|account| account.client_surface == AccountClientSurface::Desktop)
        .map(|account| {
            let routing = account_routing_options(account);
            let endpoint = router_endpoint_for_account(store, account, requested_model);
            let mapped_model = endpoint
                .as_ref()
                .map(|endpoint| router_effective_model_for_account(account, requested_model, endpoint))
                .unwrap_or_else(|| requested_model.to_string());
            let runtime_block = endpoint
                .as_ref()
                .and_then(|endpoint| {
                    account_runtime_block_for_endpoint(account, endpoint, &mapped_model, now)
                });
            let capabilities = endpoint
                .as_ref()
                .map(|endpoint| dex_router_capability_summary(account, endpoint, &mapped_model));
            let capability_gaps =
                dex_router_capability_gaps_value(capabilities.as_ref(), tool_requirements);
            let native_direct_blocked = endpoint.as_ref().is_some_and(|endpoint| {
                native_direct_model && !endpoint_is_native_router_executor(endpoint)
            });
            let reason = if anchor.is_none() {
                "no_anchor"
            } else if anchor_disabled {
                "anchor_disabled"
            } else if !routing.effective_enabled() {
                "routing_disabled"
            } else if !routing.effective_execution_enabled_for_account(account) {
                "execution_disabled"
            } else if Some(routing.pool.as_str()) != anchor_pool.as_deref() {
                "pool_mismatch"
            } else if exclusion.account_ids.iter().any(|id| id == &account.id) {
                exclusion.reason
            } else if endpoint.is_none() {
                "no_supported_endpoint"
            } else if native_direct_blocked {
                "native_direct_requires_gpt_account"
            } else if let Some(block) = runtime_block.as_ref() {
                block.reason
            } else if capability_gaps
                .as_array()
                .is_some_and(|gaps| !gaps.is_empty())
            {
                "capability_mismatch"
            } else {
                "ready"
            };
            let model_state = account.runtime_state.model_states.get(&mapped_model);
            json!({
                "account_id": account.id,
                "account_name": account.name,
                "provider": account.provider,
                "pool": routing.pool,
                "priority": routing.priority,
                "weight": routing.weight,
                "routing_enabled": routing.effective_enabled(),
                "anchor_enabled": routing.effective_anchor_enabled_for_account(account),
                "execution_enabled": routing.effective_execution_enabled_for_account(account),
                "endpoint_id": endpoint.as_ref().map(|endpoint| endpoint.id.clone()),
                "endpoint_name": endpoint.as_ref().map(|endpoint| endpoint.name.clone()),
                "endpoint_kind": endpoint.as_ref().map(|endpoint| endpoint.kind.label()),
                "mapped_model": mapped_model,
                "effective_model": mapped_model,
                "capabilities": capabilities,
                "capability_gaps": capability_gaps,
                "eligible": reason == "ready",
                "reason": reason,
                "runtime_status": account.runtime_state.status,
                "runtime_message": account.runtime_state.status_message,
                "runtime_next_retry_after": account.runtime_state.next_retry_after,
                "runtime_quota_exceeded": account.runtime_state.quota.exceeded,
                "model_runtime_status": model_state.map(|state| state.status.clone()),
                "model_runtime_message": model_state.map(|state| state.status_message.clone()),
                "model_runtime_next_retry_after": model_state.and_then(|state| state.next_retry_after),
                "model_runtime_quota_exceeded": model_state.is_some_and(|state| state.quota.exceeded),
            })
        })
        .collect();
    let eligible_count = candidates
        .iter()
        .filter(|candidate| candidate["eligible"].as_bool().unwrap_or(false))
        .count();
    let selected = selected.map(|(account, endpoint)| {
        let candidate = candidates.iter().find(|candidate| {
            candidate["account_id"].as_str() == Some(account.id.as_str())
                && candidate["endpoint_id"].as_str() == Some(endpoint.id.as_str())
        });
        json!({
            "account_id": account.id,
            "account_name": account.name,
            "endpoint_id": endpoint.id,
            "endpoint_name": endpoint.name,
            "endpoint_kind": endpoint.kind.label(),
            "mapped_model": router_effective_model_for_account(account, requested_model, endpoint),
            "effective_model": router_effective_model_for_account(account, requested_model, endpoint),
            "priority": candidate.and_then(|candidate| candidate.get("priority")).cloned(),
            "weight": candidate.and_then(|candidate| candidate.get("weight")).cloned(),
            "capabilities": candidate.and_then(|candidate| candidate.get("capabilities")).cloned(),
            "tool_decisions": candidate
                .and_then(|candidate| candidate.get("capabilities"))
                .map(|capabilities| dex_router_tool_decisions(capabilities, tool_requirements)),
        })
    });
    let selected_tool_decisions = selected
        .as_ref()
        .and_then(|selected| selected.get("tool_decisions").cloned());

    json!({
        "trace_version": 1,
        "route_surface": "codex_router",
        "requested_model": requested_model,
        "cursor": cursor,
        "tool_requirements": router_tool_requirements_value(tool_requirements),
        "tool_decisions": selected_tool_decisions,
        "anchor": anchor.map(|account| {
            let routing = account_routing_options(account);
            json!({
                "account_id": account.id,
                "account_name": account.name,
                "pool": routing.pool,
                "anchor_enabled": routing.effective_anchor_enabled_for_account(account),
                "execution_enabled": routing.effective_execution_enabled_for_account(account),
            })
        }),
        "selected": selected,
        "candidate_count": candidates.len(),
        "eligible_count": eligible_count,
        "skipped_count": candidates.len().saturating_sub(eligible_count),
        "candidates": candidates,
    })
}

fn router_endpoint_for_account(
    store: &AccountStore,
    account: &Account,
    requested_model: &str,
) -> Option<EndpointConfig> {
    let active_endpoint_id = store
        .active_endpoint_id_for_surface(&AccountClientKind::Codex, &AccountClientSurface::Desktop)
        .filter(|_| {
            store
                .active_account_for_surface(&AccountClientSurface::Desktop)
                .is_some_and(|active| active.id == account.id)
        });
    account
        .active_endpoint(active_endpoint_id)
        .filter(|endpoint| endpoint_supports_router_model(endpoint, requested_model))
        .cloned()
        .or_else(|| {
            account
                .endpoints
                .iter()
                .find(|endpoint| {
                    Some(endpoint.id.as_str()) != active_endpoint_id
                        && endpoint_supports_router_model(endpoint, requested_model)
                })
                .cloned()
        })
}

fn endpoint_supports_router_model(endpoint: &EndpointConfig, _requested_model: &str) -> bool {
    matches!(
        endpoint.kind,
        EndpointKind::OpenAiChat
            | EndpointKind::CustomChat
            | EndpointKind::OpenAiResponses
            | EndpointKind::CustomResponses
            | EndpointKind::CodexOfficial
    )
}

fn endpoint_is_native_router_executor(endpoint: &EndpointConfig) -> bool {
    endpoint.kind.is_responses_like() || endpoint.kind == EndpointKind::CodexOfficial
}

fn codex_router_native_direct_model(requested_model: &str) -> bool {
    matches!(
        requested_model.trim(),
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini"
    )
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
            let mapped_model = router_effective_model(requested_model, &endpoint);
            if !account_runtime_ready_for_endpoint(account, &endpoint, &mapped_model, now) {
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

fn endpoint_uses_dex_managed_runtime_cooldown(
    account: &Account,
    endpoint: &EndpointConfig,
) -> bool {
    endpoint.kind == EndpointKind::CodexOfficial
        || matches!(account.auth_mode, AccountAuthMode::OAuth)
}

fn account_runtime_ready_for_endpoint(
    account: &Account,
    endpoint: &EndpointConfig,
    mapped_model: &str,
    now: u64,
) -> bool {
    account_runtime_block_for_endpoint(account, endpoint, mapped_model, now).is_none()
}

fn dex_router_capability_summary(
    account: &Account,
    endpoint: &EndpointConfig,
    mapped_model: &str,
) -> Value {
    let profile = providers::profile_for_account(account);
    let provider_caps = &profile.capabilities;
    let protocol = match endpoint.kind {
        EndpointKind::OpenAiChat | EndpointKind::CustomChat => "chat_translate",
        EndpointKind::OpenAiResponses | EndpointKind::CustomResponses => "responses_direct",
        EndpointKind::AnthropicMessages => "anthropic_messages",
        EndpointKind::CodexOfficial => "codex_official",
    };
    let tool_mode = match endpoint.kind {
        EndpointKind::OpenAiChat | EndpointKind::CustomChat => {
            if provider_caps.tools {
                "translated"
            } else {
                "none"
            }
        }
        EndpointKind::OpenAiResponses
        | EndpointKind::CustomResponses
        | EndpointKind::CodexOfficial => "native",
        EndpointKind::AnthropicMessages => "anthropic",
    };
    let reasoning = match endpoint.kind {
        EndpointKind::OpenAiResponses
        | EndpointKind::CustomResponses
        | EndpointKind::CodexOfficial => "native",
        EndpointKind::AnthropicMessages => "anthropic",
        EndpointKind::OpenAiChat | EndpointKind::CustomChat => match provider_caps.reasoning {
            providers::ReasoningMode::None => "none",
            providers::ReasoningMode::DeepSeek => "deepseek",
            providers::ReasoningMode::OpenAi => "openai",
            providers::ReasoningMode::AnthropicThinking => "anthropic",
        },
    };
    let vision = match endpoint.model_vision_mode(mapped_model) {
        VisionMode::Off => "off",
        VisionMode::Native => "native",
        VisionMode::Glue => "glue",
    };
    let stream_usage = match provider_caps.stream_usage {
        providers::StreamUsageMode::FinalChunk => "final_chunk",
        providers::StreamUsageMode::ResponseCompleted => "response_completed",
        providers::StreamUsageMode::Unavailable => "unavailable",
    };
    let native_responses = matches!(
        endpoint.kind,
        EndpointKind::OpenAiResponses | EndpointKind::CustomResponses | EndpointKind::CodexOfficial
    );
    let web_mode = if native_responses {
        "native"
    } else if provider_caps.web_search_tool {
        "tool"
    } else if provider_caps.web_search_options {
        "options"
    } else {
        "none"
    };
    let supports_web = web_mode != "none";
    let supports_image_generation = endpoint.effective_image_generation_enabled(account);

    json!({
        "protocol": protocol,
        "tool_mode": tool_mode,
        "tools": tool_mode != "none",
        "web": supports_web,
        "web_mode": web_mode,
        "vision": vision,
        "reasoning": reasoning,
        "image_generation": supports_image_generation,
        "stream_usage": stream_usage,
        "allow_missing_done": provider_caps.allow_missing_done,
    })
}

fn router_tool_requirements(req: &ResponsesRequest) -> RouterToolRequirements {
    let mut requirements = router_tool_requirements_for_tools(&req.tools, req.tool_choice.as_ref());
    router_tool_requirements_from_input(&req.input, &mut requirements);
    requirements
}

fn router_tool_requirements_for_tools(
    tools: &[Value],
    tool_choice: Option<&Value>,
) -> RouterToolRequirements {
    let mut requirements = RouterToolRequirements {
        tool_count: tools.len(),
        ..Default::default()
    };

    for tool in tools {
        let typ = tool
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let label = router_tool_label(tool, &typ);
        match typ.as_str() {
            "web_search" | "web_search_preview" | "web_fetch" | "web_fetch_preview" => {
                requirements.has_web_search = true;
            }
            "file_search" | "file_search_preview" => {
                requirements.has_file_search = true;
            }
            "mcp" | "remote_mcp" => {
                requirements.has_mcp = true;
            }
            "computer_use" | "computer_use_preview" | "browser_use" | "browser" => {
                requirements.has_computer = true;
            }
            "image_generation" | "image_generation_preview" | "image2" => {
                requirements.has_image_generation = true;
            }
            "function" | "custom" | "namespace" | "local_shell" | "tool_search" => {
                requirements.has_function_tools = true;
            }
            "" if tool.get("function").is_some() => {
                requirements.has_function_tools = true;
            }
            "" => {
                requirements.has_unknown_tools = true;
            }
            _ => {
                if tool.get("function").is_some() {
                    requirements.has_function_tools = true;
                } else {
                    requirements.has_unknown_tools = true;
                }
            }
        }
        if !requirements.labels.iter().any(|seen| seen == &label) {
            requirements.labels.push(label);
        }
    }

    if requirements.has_web_search {
        requirements.requires_web_search = true;
    }
    match router_tool_choice_type(tool_choice) {
        Some("function" | "custom" | "namespace" | "local_shell" | "tool_search") => {
            requirements.requires_function_tools = true;
            requirements.has_function_tools = true;
        }
        Some("web_search" | "web_search_preview" | "web_fetch" | "web_fetch_preview") => {
            requirements.requires_web_search = true;
            requirements.has_web_search = true;
        }
        Some("file_search" | "file_search_preview") => {
            requirements.requires_file_search = true;
            requirements.has_file_search = true;
        }
        Some("mcp" | "remote_mcp") => {
            requirements.requires_mcp = true;
            requirements.has_mcp = true;
        }
        Some("computer_use" | "computer_use_preview" | "browser_use" | "browser") => {
            requirements.set_native_signal(NativeRouteSignal::StrongNative, "tool_choice.computer");
        }
        Some("image_generation" | "image_generation_preview" | "image2") => {
            requirements.requires_image_generation = true;
            requirements.has_image_generation = true;
        }
        Some(_) => {
            requirements.requires_unknown_tools = true;
            requirements.has_unknown_tools = true;
        }
        None => {}
    }

    requirements
}

fn router_tool_requirements_for_diagnostic_tools(tools: &[Value]) -> RouterToolRequirements {
    let mut requirements = router_tool_requirements_for_tools(tools, None);
    requirements.require_all_available_tools();
    requirements
}

fn router_tool_requirements_from_input(
    input: &ResponsesInput,
    requirements: &mut RouterToolRequirements,
) {
    let mut signal = None;
    match input {
        ResponsesInput::Text(text) => {
            if text.contains("computer_call_output") || text.contains("\"screenshot\"") {
                signal = Some((
                    NativeRouteSignal::StrongNative,
                    "input.text_computer_signal",
                ));
            } else if router_text_computer_intent_signal(text) {
                signal = Some((NativeRouteSignal::TextIntent, "input.computer_intent"));
            }
        }
        ResponsesInput::Messages(items) => {
            signal = router_latest_input_computer_signal(items);
        }
    }

    if let Some((signal, label)) = signal {
        let signal = if signal == NativeRouteSignal::TextIntent && requirements.has_computer {
            NativeRouteSignal::StrongNative
        } else {
            signal
        };
        requirements.set_native_signal(signal, label);
    }
}

fn router_latest_input_computer_signal(
    items: &[Value],
) -> Option<(NativeRouteSignal, &'static str)> {
    for item in items.iter().rev() {
        if let Some(signal) = router_explicit_computer_output_signal(item) {
            return Some(signal);
        }
    }
    for item in items.iter().rev() {
        if router_input_item_is_user_turn(item) {
            return router_input_computer_signal_for_message_item(item);
        }
    }
    None
}

fn router_input_item_is_user_turn(value: &Value) -> bool {
    let Value::Object(map) = value else {
        return true;
    };
    map.get("role")
        .and_then(Value::as_str)
        .is_none_or(|role| role == "user")
}

fn router_explicit_computer_output_signal(
    value: &Value,
) -> Option<(NativeRouteSignal, &'static str)> {
    match value {
        Value::Array(items) => items
            .iter()
            .rev()
            .find_map(router_explicit_computer_output_signal),
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(typ, "computer_call" | "computer_call_output") {
                return Some((
                    NativeRouteSignal::StrongNative,
                    "input.computer_call_output",
                ));
            }
            if map.get("screenshot").is_some() {
                return Some((NativeRouteSignal::StrongNative, "input.screenshot"));
            }
            for key in ["output", "content"] {
                if let Some(value) = map.get(key) {
                    if let Some(signal) = router_explicit_computer_output_signal(value) {
                        return Some(signal);
                    }
                }
            }
            None
        }
        Value::String(text) => {
            if text.contains("computer_call_output") || text.contains("\"screenshot\"") {
                Some((
                    NativeRouteSignal::StrongNative,
                    "input.text_computer_signal",
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn router_input_computer_signal_for_message_item(
    value: &Value,
) -> Option<(NativeRouteSignal, &'static str)> {
    let allow_text_intent = value
        .get("role")
        .and_then(Value::as_str)
        .is_none_or(|role| role == "user");
    router_input_computer_signal(value, allow_text_intent)
}

fn router_input_computer_signal(
    value: &Value,
    allow_text_intent: bool,
) -> Option<(NativeRouteSignal, &'static str)> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| router_input_computer_signal(item, allow_text_intent)),
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(typ, "computer_call" | "computer_call_output") {
                return Some((
                    NativeRouteSignal::StrongNative,
                    "input.computer_call_output",
                ));
            }
            if map.get("screenshot").is_some() {
                return Some((NativeRouteSignal::StrongNative, "input.screenshot"));
            }
            let child_text_intent = map
                .get("role")
                .and_then(Value::as_str)
                .map(|role| role == "user")
                .unwrap_or(allow_text_intent);
            for key in ["output", "content", "image", "image_url", "text"] {
                if let Some(value) = map.get(key) {
                    if let Some(signal) = router_input_computer_signal(value, child_text_intent) {
                        return Some(signal);
                    }
                }
            }
            None
        }
        Value::String(text) => {
            if allow_text_intent
                && (text.contains("computer_call_output") || text.contains("\"screenshot\""))
            {
                Some((
                    NativeRouteSignal::StrongNative,
                    "input.text_computer_signal",
                ))
            } else if allow_text_intent && router_text_computer_intent_signal(text) {
                Some((NativeRouteSignal::TextIntent, "input.computer_intent"))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn router_text_computer_intent_signal(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    if text.contains("computer use")
        || text.contains("computer-use")
        || text.contains("computer plugin")
        || text.contains("computer tool")
        || text.contains("plugin://computer-use")
        || text.contains("app://computer-use")
        || text.contains("browser-use")
        || text.contains("[@电脑]")
        || text.contains("电脑插件")
        || text.contains("电脑操作")
        || text.contains("操作电脑")
        || text.contains("使用电脑")
    {
        return true;
    }

    let app_action = text.contains("打开")
        || text.contains("点击")
        || text.contains("播放")
        || text.contains("登录")
        || text.contains("切到")
        || text.contains("切换到");
    let app_target = text.contains(" app")
        || text.contains("应用")
        || text.contains("抖音")
        || text.contains("浏览器")
        || text.contains("视频")
        || text.contains("屏幕")
        || text.contains("窗口");
    app_action && app_target
}

fn router_input_weak_continuation_signal(input: &ResponsesInput) -> Option<&'static str> {
    let text = router_latest_user_text(input)?;
    router_text_weak_continuation_signal(&text).then_some("input.weak_continuation")
}

fn router_latest_user_text(input: &ResponsesInput) -> Option<String> {
    match input {
        ResponsesInput::Text(text) => Some(text.clone()),
        ResponsesInput::Messages(items) => items.iter().rev().find_map(router_user_text_from_value),
    }
}

fn router_user_text_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(router_user_text_from_value)
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Object(map) => {
            if map
                .get("role")
                .and_then(Value::as_str)
                .is_some_and(|role| role != "user")
            {
                return None;
            }
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return Some(text.to_string());
            }
            map.get("content")
                .or_else(|| map.get("input"))
                .and_then(router_user_text_from_value)
        }
        _ => None,
    }
}

fn router_text_weak_continuation_signal(text: &str) -> bool {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return false;
    }
    let weak = [
        "继续",
        "下一步",
        "接着",
        "然后呢",
        "点第一个",
        "点击第一个",
        "打开第一个",
        "播放第一个",
        "刚才那个",
        "上一个",
        "这个页面",
        "当前页面",
    ];
    weak.iter().any(|needle| text.contains(needle))
}

impl RouterToolRequirements {
    fn require_all_available_tools(&mut self) {
        self.requires_function_tools = self.has_function_tools;
        self.requires_web_search = self.has_web_search;
        self.requires_file_search = self.has_file_search;
        self.requires_mcp = self.has_mcp;
        self.requires_computer = self.has_computer;
        self.requires_image_generation = self.has_image_generation;
        self.requires_unknown_tools = self.has_unknown_tools;
    }
}

fn router_tool_choice_type(tool_choice: Option<&Value>) -> Option<&str> {
    let choice = tool_choice?;
    match choice {
        Value::String(choice) => {
            let choice = choice.trim();
            (!matches!(choice, "" | "auto" | "none" | "required")).then_some(choice)
        }
        Value::Object(obj) => obj.get("type").and_then(Value::as_str),
        _ => None,
    }
}

fn router_tool_label(tool: &Value, typ: &str) -> String {
    tool.get("name")
        .or_else(|| tool.get("namespace"))
        .or_else(|| tool.get("server_label"))
        .or_else(|| tool.get("server_url"))
        .or_else(|| {
            tool.get("function")
                .and_then(|function| function.get("name"))
        })
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if typ.is_empty() {
                "unknown_tool".into()
            } else {
                typ.to_string()
            }
        })
}

fn router_tool_requirements_value(requirements: Option<&RouterToolRequirements>) -> Value {
    let Some(requirements) = requirements else {
        return Value::Null;
    };
    json!({
        "tool_count": requirements.tool_count,
        "labels": requirements.labels.clone(),
        "function_tools": requirements.has_function_tools,
        "web_search": requirements.has_web_search,
        "file_search": requirements.has_file_search,
        "mcp": requirements.has_mcp,
        "computer": requirements.has_computer,
        "image_generation": requirements.has_image_generation,
        "requires_function_tools": requirements.requires_function_tools,
        "requires_web_search": requirements.requires_web_search,
        "requires_file_search": requirements.requires_file_search,
        "requires_mcp": requirements.requires_mcp,
        "requires_computer": requirements.requires_computer,
        "requires_image_generation": requirements.requires_image_generation,
        "requires_unknown_tools": requirements.requires_unknown_tools,
        "unknown_tools": requirements.has_unknown_tools,
        "native_signal": requirements.native_signal.as_str(),
    })
}

fn codex_router_session_route_key(headers: &HeaderMap) -> Option<String> {
    for name in ["thread-id", "session-id"] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(format!("{name}:{value}"));
            }
        }
    }
    None
}

fn update_codex_router_session_route(
    state: &AppState,
    route_key: Option<String>,
    mut requirements: Option<&mut RouterToolRequirements>,
    input: Option<&ResponsesInput>,
) -> Option<DexRouterSessionRouteDecision> {
    let route_key = route_key?;
    if let (Some(requirements), Some(input)) = (requirements.as_deref_mut(), input) {
        if requirements.native_signal == NativeRouteSignal::None
            && state
                .codex_router_sessions
                .get(&route_key)
                .is_some_and(|state| state.observe_remaining > 0)
        {
            if let Some(label) = router_input_weak_continuation_signal(input) {
                requirements.native_signal = NativeRouteSignal::WeakContinuation;
                requirements.add_label(label);
            }
        }
    }
    update_codex_router_session_route_state(
        &state.codex_router_sessions,
        Some(route_key),
        requirements,
        crate::accounts::now_secs(),
    )
}

fn codex_router_session_main_model_anchor(
    sessions: &crate::codex_router_session::RouteStateMap,
    route_key: Option<&str>,
) -> Option<crate::codex_router_session::MainModelAnchor> {
    let route_key = route_key?;
    sessions
        .get(route_key)
        .and_then(|state| state.main_model_anchor.clone())
}

fn record_codex_router_session_main_model_anchor(
    sessions: &crate::codex_router_session::RouteStateMap,
    route_key: Option<&str>,
    selection: &AccountRouteSelection,
) {
    let Some(route_key) = route_key else {
        return;
    };
    let Some(anchor) = selection.main_model_anchor_to_record.clone() else {
        return;
    };
    let mut state = sessions
        .get(route_key)
        .map(|state| state.clone())
        .unwrap_or(crate::codex_router_session::RouteState {
            observe_remaining: 0,
            expires_at: 0,
            main_model_anchor: None,
        });
    state.main_model_anchor = Some(anchor.clone());
    sessions.insert(route_key.to_string(), state);
    tracing::info!(
        session_route_key = %route_key,
        account_id = %anchor.account_id,
        endpoint_id = %anchor.endpoint_id,
        model = %anchor.model,
        "DEX Router 已记录会话主模型锚点"
    );
}

fn update_codex_router_session_route_state(
    sessions: &crate::codex_router_session::RouteStateMap,
    route_key: Option<String>,
    requirements: Option<&mut RouterToolRequirements>,
    now: u64,
) -> Option<DexRouterSessionRouteDecision> {
    let route_key = route_key?;
    let native_signal = requirements
        .as_ref()
        .map(|requirements| requirements.native_signal)
        .unwrap_or(NativeRouteSignal::None);

    if native_signal >= NativeRouteSignal::StrongNative {
        let reason = requirements
            .as_ref()
            .and_then(|requirements| {
                requirements
                    .labels
                    .iter()
                    .find(|label| label.contains("computer") || label.contains("screenshot"))
                    .cloned()
            })
            .unwrap_or_else(|| "computer_use".into());
        let route_state = crate::codex_router_session::RouteState {
            observe_remaining: crate::codex_router_session::NATIVE_OBSERVE_TURNS,
            expires_at: now.saturating_add(crate::codex_router_session::NATIVE_OBSERVE_TTL_SECS),
            main_model_anchor: sessions
                .get(&route_key)
                .and_then(|state| state.main_model_anchor.clone()),
        };
        sessions.insert(route_key.clone(), route_state.clone());
        return Some(DexRouterSessionRouteDecision {
            key: route_key,
            state: "native_active",
            reason,
            observe_remaining: route_state.observe_remaining,
            expires_at: Some(route_state.expires_at),
            force_native_responses: true,
        });
    }

    let weak_continuation = native_signal == NativeRouteSignal::WeakContinuation;
    let Some(entry) = sessions.get(&route_key) else {
        return Some(DexRouterSessionRouteDecision {
            key: route_key,
            state: "free",
            reason: "no_native_session".into(),
            observe_remaining: 0,
            expires_at: None,
            force_native_responses: false,
        });
    };

    let existing_anchor = entry.main_model_anchor.clone();
    if existing_anchor.is_some() && native_signal < NativeRouteSignal::StrongNative {
        drop(entry);
        sessions.insert(
            route_key.clone(),
            crate::codex_router_session::RouteState {
                observe_remaining: 0,
                expires_at: 0,
                main_model_anchor: existing_anchor,
            },
        );
        return Some(DexRouterSessionRouteDecision {
            key: route_key,
            state: "free",
            reason: "main_model_anchor_plain_turn".into(),
            observe_remaining: 0,
            expires_at: None,
            force_native_responses: false,
        });
    }

    if entry.observe_remaining > 0 && entry.expires_at <= now {
        drop(entry);
        if let Some(anchor) = existing_anchor {
            sessions.insert(
                route_key.clone(),
                crate::codex_router_session::RouteState {
                    observe_remaining: 0,
                    expires_at: 0,
                    main_model_anchor: Some(anchor),
                },
            );
        } else {
            sessions.remove(&route_key);
        }
        return Some(DexRouterSessionRouteDecision {
            key: route_key,
            state: "free",
            reason: "native_observe_expired".into(),
            observe_remaining: 0,
            expires_at: None,
            force_native_responses: false,
        });
    }

    if entry.observe_remaining == 0 {
        if weak_continuation {
            let route_state = crate::codex_router_session::RouteState {
                observe_remaining: crate::codex_router_session::NATIVE_OBSERVE_TURNS,
                expires_at: now
                    .saturating_add(crate::codex_router_session::NATIVE_OBSERVE_TTL_SECS),
                main_model_anchor: sessions
                    .get(&route_key)
                    .and_then(|state| state.main_model_anchor.clone()),
            };
            drop(entry);
            if let Some(requirements) = requirements {
                requirements.force_native_observe();
                requirements.add_label("session.weak_continuation");
            }
            sessions.insert(route_key.clone(), route_state.clone());
            return Some(DexRouterSessionRouteDecision {
                key: route_key,
                state: "native_observe",
                reason: "weak_continuation_keep_native".into(),
                observe_remaining: route_state.observe_remaining,
                expires_at: Some(route_state.expires_at),
                force_native_responses: true,
            });
        }
        let existing_anchor = entry.main_model_anchor.clone();
        if entry.expires_at == 0 && existing_anchor.is_some() {
            drop(entry);
            return Some(DexRouterSessionRouteDecision {
                key: route_key,
                state: "free",
                reason: "main_model_anchor_only".into(),
                observe_remaining: 0,
                expires_at: None,
                force_native_responses: false,
            });
        }
        drop(entry);
        if let Some(anchor) = existing_anchor {
            sessions.insert(
                route_key.clone(),
                crate::codex_router_session::RouteState {
                    observe_remaining: 0,
                    expires_at: 0,
                    main_model_anchor: Some(anchor),
                },
            );
        } else {
            sessions.remove(&route_key);
        }
        return Some(DexRouterSessionRouteDecision {
            key: route_key,
            state: "native_released",
            reason: "plain_turn_release_to_chat".into(),
            observe_remaining: 0,
            expires_at: None,
            force_native_responses: false,
        });
    }

    let expires_at = entry.expires_at;
    let observe_remaining = entry.observe_remaining.saturating_sub(1);
    drop(entry);

    if let Some(requirements) = requirements {
        requirements.force_native_observe();
        if weak_continuation {
            requirements.add_label("session.weak_continuation");
        }
    }

    sessions.insert(
        route_key.clone(),
        crate::codex_router_session::RouteState {
            observe_remaining,
            expires_at,
            main_model_anchor: sessions
                .get(&route_key)
                .and_then(|state| state.main_model_anchor.clone()),
        },
    );
    Some(DexRouterSessionRouteDecision {
        key: route_key,
        state: "native_observe",
        reason: "plain_turn_keep_native_once".into(),
        observe_remaining,
        expires_at: Some(expires_at),
        force_native_responses: true,
    })
}

fn patch_selection_session_route_trace(
    mut selection: AccountRouteSelection,
    decision: Option<&DexRouterSessionRouteDecision>,
) -> AccountRouteSelection {
    selection.route_trace = patch_router_session_route_trace(selection.route_trace, decision);
    selection
}

fn patch_router_session_route_trace(
    route_trace: Option<String>,
    decision: Option<&DexRouterSessionRouteDecision>,
) -> Option<String> {
    let Some(decision) = decision else {
        return route_trace;
    };
    let mut trace = route_trace
        .and_then(|trace| serde_json::from_str::<Value>(&trace).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = trace.as_object_mut() {
        obj.insert("session_route_key".into(), json!(decision.key));
        obj.insert("session_route_state".into(), json!(decision.state));
        obj.insert("session_route_reason".into(), json!(decision.reason));
        obj.insert(
            "session_route_observe_remaining".into(),
            json!(decision.observe_remaining),
        );
        obj.insert(
            "session_route_expires_at".into(),
            json!(decision.expires_at),
        );
        obj.insert(
            "session_route_force_native_responses".into(),
            json!(decision.force_native_responses),
        );
    }
    serde_json::to_string(&trace).ok()
}

fn attach_codex_router_session_key(
    history_context: &mut HistoryContext,
    decision: Option<&DexRouterSessionRouteDecision>,
) {
    history_context.codex_router_session_key = decision.map(|decision| decision.key.clone());
}

fn refresh_codex_router_session_from_response(
    state: &AppState,
    history_context: &HistoryContext,
    response: &Value,
) {
    let feedback = crate::codex_router_session::maybe_refresh_from_response(
        Some(&state.codex_router_sessions),
        history_context.codex_router_session_key.as_deref(),
        response,
        crate::accounts::now_secs(),
    );
    if let Some(feedback) = feedback {
        tracing::info!(
            session_route_key = %feedback.key,
            reason = feedback.reason,
            refreshed = feedback.refreshed,
            "DEX Router 响应反查刷新 Computer Use 原生轨道"
        );
    }
}

fn dex_router_capability_gaps(
    capabilities: &Value,
    requirements: Option<&RouterToolRequirements>,
) -> Vec<String> {
    let Some(requirements) = requirements else {
        return Vec::new();
    };
    if !requirements.has_tools() {
        return Vec::new();
    }

    let protocol = capabilities
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_mode = capabilities
        .get("tool_mode")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let tools = capabilities
        .get("tools")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let web = capabilities
        .get("web")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let image_generation = capabilities
        .get("image_generation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let native_tools = tool_mode == "native";
    let translated_tools = matches!(tool_mode, "translated" | "anthropic");
    let local_file_search = matches!(protocol, "chat_translate" | "anthropic_messages");

    let mut gaps = Vec::new();
    if requirements.requires_function_tools && !tools {
        gaps.push("tools".to_string());
    }
    if requirements.requires_web_search && !web {
        gaps.push("web_search".to_string());
    }
    if requirements.requires_file_search && !(native_tools || local_file_search) {
        gaps.push("file_search".to_string());
    }
    if requirements.requires_mcp && !(native_tools || tool_mode == "translated") {
        gaps.push("mcp".to_string());
    }
    if requirements.requires_computer && !native_tools {
        gaps.push("computer".to_string());
    }
    if requirements.requires_image_generation && !image_generation {
        gaps.push("image_generation".to_string());
    }
    if requirements.requires_unknown_tools && !(native_tools || translated_tools) {
        gaps.push("unknown_tools".to_string());
    }
    gaps
}

fn dex_router_capability_gaps_value(
    capabilities: Option<&Value>,
    requirements: Option<&RouterToolRequirements>,
) -> Value {
    match capabilities {
        Some(capabilities) => json!(dex_router_capability_gaps(capabilities, requirements)),
        None => json!([]),
    }
}

fn dex_router_tool_decisions(
    capabilities: &Value,
    requirements: Option<&RouterToolRequirements>,
) -> Value {
    let Some(requirements) = requirements else {
        return Value::Null;
    };
    let protocol = capabilities
        .get("protocol")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_mode = capabilities
        .get("tool_mode")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let web_mode = capabilities
        .get("web_mode")
        .and_then(Value::as_str)
        .unwrap_or("none");
    let native_tools = tool_mode == "native";
    let translated_tools = tool_mode == "translated";
    let anthropic_tools = tool_mode == "anthropic";

    let mut kept = Vec::new();
    let mut translated = Vec::new();
    let mut local = Vec::new();
    let mut filtered = dex_router_capability_gaps(capabilities, Some(requirements));

    if requirements.has_function_tools {
        if native_tools {
            kept.push("function_tools");
        } else if translated_tools || anthropic_tools {
            translated.push("function_tools");
        }
    }
    if requirements.has_web_search {
        if native_tools {
            kept.push("web_search");
        } else if capabilities
            .get("web")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            translated.push(match web_mode {
                "tool" => "web_search_tool",
                "options" => "web_search_options",
                _ => "web_search",
            });
        }
    }
    if requirements.has_file_search {
        if native_tools {
            kept.push("file_search");
        } else if matches!(protocol, "chat_translate" | "anthropic_messages") {
            local.push("file_search");
        }
    }
    if requirements.has_mcp {
        if native_tools {
            kept.push("mcp");
        } else if translated_tools {
            translated.push("local_mcp_call");
        }
    }
    if requirements.has_computer && native_tools {
        kept.push("computer_use");
    }
    if requirements.has_image_generation
        && native_tools
        && capabilities
            .get("image_generation")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        kept.push("image_generation");
    }
    if requirements.has_unknown_tools {
        if native_tools {
            kept.push("unknown_tools");
        } else if translated_tools || anthropic_tools {
            translated.push("unknown_tools");
        }
    }
    filtered.sort();
    filtered.dedup();

    json!({
        "kept": kept,
        "translated": translated,
        "local": local,
        "filtered": filtered,
        "labels": requirements.labels.clone(),
    })
}

fn tool_type_is_image_generation(typ: &str) -> bool {
    matches!(
        typ,
        "image_generation" | "image_generation_preview" | "image2"
    )
}

fn request_has_image_generation_tool(req: &ResponsesRequest) -> bool {
    req.tools.iter().any(|tool| {
        tool.get("type")
            .and_then(Value::as_str)
            .is_some_and(tool_type_is_image_generation)
    })
}

fn strip_image_generation_tools_from_body(
    body: &axum::body::Bytes,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut value: Value = serde_json::from_slice(body)?;
    if let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| {
            !tool
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(tool_type_is_image_generation)
        });
    }
    serde_json::to_vec(&value).map(axum::body::Bytes::from)
}

fn apply_endpoint_image_generation_declaration(
    account: &Account,
    endpoint: &EndpointConfig,
    req: &mut ResponsesRequest,
    body: &mut axum::body::Bytes,
) -> Result<(), Box<Response>> {
    if endpoint.effective_image_generation_enabled(account)
        || !request_has_image_generation_tool(req)
    {
        return Ok(());
    }

    let requirements = router_tool_requirements(req);
    if requirements.requires_image_generation {
        return Err(Box::new((
            StatusCode::CONFLICT,
            Json(json!({
                "error": {
                    "message": format!(
                        "账号「{}」的端点「{}」未声明支持 image_generation，无法执行图片生成；请在账号编辑里开启“图片生成”，或切换到支持生图的 Responses 端点。",
                        account.name,
                        endpoint.name
                    ),
                    "type": "capability_error",
                    "code": "image_generation_not_enabled"
                }
            })),
        )
            .into_response()));
    }

    req.tools.retain(|tool| {
        !tool
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(tool_type_is_image_generation)
    });
    match strip_image_generation_tools_from_body(body) {
        Ok(updated) => *body = updated,
        Err(err) => warn!(
            account_id = %account.id,
            endpoint_id = %endpoint.id,
            "image_generation 工具剥离失败，继续使用解析后的请求体: {err}"
        ),
    }
    info!(
        account_id = %account.id,
        endpoint_id = %endpoint.id,
        "端点未声明支持 image_generation，已从本轮直连请求中移除可选图片生成工具"
    );
    Ok(())
}

fn native_computer_tool_type(typ: &str) -> bool {
    matches!(
        typ,
        "computer_use" | "computer_use_preview" | "browser_use" | "browser"
    )
}

fn native_computer_item_type(typ: &str) -> bool {
    matches!(typ, "computer_call" | "computer_call_output")
}

fn native_computer_tool_name(name: &str) -> bool {
    matches!(
        name,
        "get_app_state"
            | "screenshot"
            | "click"
            | "double_click"
            | "scroll"
            | "press_key"
            | "type"
            | "type_text"
            | "drag"
            | "move"
            | "wait"
            | "open_url"
    )
}

fn tool_value_is_native_computer(tool: &Value) -> bool {
    let Value::Object(map) = tool else {
        return false;
    };
    let typ = map
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if native_computer_tool_type(&typ) {
        return true;
    }
    let label = map
        .get("name")
        .or_else(|| map.get("namespace"))
        .or_else(|| map.get("server_label"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if label.contains("computer") || label.contains("browser-use") {
        return true;
    }
    if matches!(typ.as_str(), "function" | "custom") {
        let name = map
            .get("name")
            .or_else(|| {
                map.get("function")
                    .and_then(|function| function.get("name"))
            })
            .and_then(Value::as_str)
            .unwrap_or("");
        return native_computer_tool_name(name);
    }
    false
}

fn strip_native_computer_from_value(value: &mut Value) -> bool {
    match value {
        Value::Array(items) => {
            let mut changed = false;
            items.retain_mut(|item| {
                let drop = strip_native_computer_value_or_drop(item);
                changed |= drop;
                !drop
            });
            for item in items {
                changed |= strip_native_computer_from_value(item);
            }
            changed
        }
        Value::Object(map) => {
            let typ = map
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if native_computer_item_type(&typ) {
                *value = Value::Null;
                return true;
            }
            let mut changed = false;
            for key in ["screenshot", "action"] {
                changed |= map.remove(key).is_some();
            }
            for key in ["content", "output", "image", "text"] {
                if let Some(child) = map.get_mut(key) {
                    let drop_child = child
                        .get("type")
                        .and_then(Value::as_str)
                        .map(|typ| native_computer_item_type(&typ.to_ascii_lowercase()))
                        .unwrap_or(false);
                    if drop_child {
                        *child = Value::Null;
                        changed = true;
                    } else {
                        changed |= strip_native_computer_from_value(child);
                    }
                }
            }
            changed
        }
        Value::String(text)
            if text.contains("computer_call_output") || text.contains("\"screenshot\"") =>
        {
            *text = "[Computer Use 原生工具链残留已按账号设置剥离]".into();
            true
        }
        Value::String(_) => false,
        _ => false,
    }
}

fn strip_native_computer_value_or_drop(value: &mut Value) -> bool {
    let should_drop = value
        .get("type")
        .and_then(Value::as_str)
        .map(|typ| native_computer_item_type(&typ.to_ascii_lowercase()))
        .unwrap_or(false);
    if should_drop {
        return true;
    }
    strip_native_computer_from_value(value);
    false
}

fn strip_native_computer_toolchain_from_request(req: &mut ResponsesRequest) -> bool {
    let mut changed = false;
    let before_tools = req.tools.len();
    req.tools
        .retain(|tool| !tool_value_is_native_computer(tool));
    changed |= before_tools != req.tools.len();

    if req
        .tool_choice
        .as_ref()
        .is_some_and(tool_value_is_native_computer)
    {
        req.tool_choice = None;
        changed = true;
    }

    match &mut req.input {
        ResponsesInput::Text(text) => {
            if text.contains("computer_call_output") || text.contains("\"screenshot\"") {
                *text = "[Computer Use 原生工具链残留已按账号设置剥离]".into();
                changed = true;
            }
        }
        ResponsesInput::Messages(items) => {
            let before = items.len();
            items.retain_mut(|item| !strip_native_computer_value_or_drop(item));
            changed |= before != items.len();
            for item in items {
                changed |= strip_native_computer_from_value(item);
            }
        }
    }
    changed
}

fn patch_native_computer_strip_trace(route_trace: Option<String>, changed: bool) -> Option<String> {
    let mut trace = route_trace
        .and_then(|trace| serde_json::from_str::<Value>(&trace).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = trace.as_object_mut() {
        obj.insert("native_computer_policy".into(), json!("strip_and_continue"));
        obj.insert("native_toolchain_stripped".into(), json!(true));
        obj.insert("native_toolchain_request_changed".into(), json!(changed));
    }
    serde_json::to_string(&trace).ok()
}

struct RuntimeBlock {
    reason: &'static str,
}

fn account_runtime_block_for_endpoint(
    account: &Account,
    endpoint: &EndpointConfig,
    mapped_model: &str,
    now: u64,
) -> Option<RuntimeBlock> {
    if !endpoint_uses_dex_managed_runtime_cooldown(account, endpoint) {
        return None;
    }
    account_runtime_block(account, mapped_model, now)
}

fn account_runtime_block(account: &Account, mapped_model: &str, now: u64) -> Option<RuntimeBlock> {
    if matches!(
        account.runtime_state.status,
        AccountRuntimeStatus::QuotaExceeded
    ) && !runtime_retry_ready(account.runtime_state.next_retry_after, now)
    {
        return Some(RuntimeBlock {
            reason: "account_quota_cooling",
        });
    }
    if runtime_state_blocks_for_upstream_cooldown(
        &account.runtime_state.status,
        &account.runtime_state.status_message,
        account.runtime_state.next_retry_after,
        now,
    ) {
        return Some(RuntimeBlock {
            reason: "account_upstream_cooling",
        });
    }

    account
        .runtime_state
        .model_states
        .get(mapped_model)
        .and_then(|state| {
            if matches!(state.status, AccountRuntimeStatus::QuotaExceeded)
                && !runtime_retry_ready(state.next_retry_after, now)
            {
                Some(RuntimeBlock {
                    reason: "model_quota_cooling",
                })
            } else if runtime_state_blocks_for_upstream_cooldown(
                &state.status,
                &state.status_message,
                state.next_retry_after,
                now,
            ) {
                Some(RuntimeBlock {
                    reason: "model_upstream_cooling",
                })
            } else {
                None
            }
        })
}

fn runtime_state_blocks_for_upstream_cooldown(
    status: &AccountRuntimeStatus,
    status_message: &str,
    next_retry_after: Option<u64>,
    now: u64,
) -> bool {
    !runtime_retry_ready(next_retry_after, now)
        && (matches!(status, AccountRuntimeStatus::Error)
            || runtime_state_is_transient_upstream_error(status, status_message))
}

fn runtime_state_is_transient_upstream_error(
    status: &AccountRuntimeStatus,
    status_message: &str,
) -> bool {
    if !matches!(
        status,
        AccountRuntimeStatus::CoolingDown | AccountRuntimeStatus::Error
    ) {
        return false;
    }
    let message = status_message.to_ascii_lowercase();
    [
        "http 408",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
        "service temporarily unavailable",
        "upstream request failed",
        "response.failed",
    ]
    .iter()
    .any(|needle| message.contains(needle))
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
        route_trace: String::new(),
        codex_router_session_key: None,
    }
}

fn runtime_feedback_for_account_endpoint(
    state: &AppState,
    account: &Account,
    endpoint: &EndpointConfig,
) -> RuntimeFeedbackSink {
    RuntimeFeedbackSink::new(
        state.data_dir.clone(),
        state.account_store.clone(),
        state.active_account.clone(),
        account.id.clone(),
        endpoint_uses_dex_managed_runtime_cooldown(account, endpoint),
    )
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
    codex_router_sessions: Option<crate::codex_router_session::RouteStateMap>,
    response_id: String,
    model: String,
    start: Instant,
    upstream_url: String,
    http_status: StatusCode,
    retry_after_secs: Option<u64>,
    runtime_feedback: Option<RuntimeFeedbackSink>,
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
        let mut native_feedback_sent = false;
        while let Some(item) = source.next().await {
            match item {
                Ok(bytes) => {
                    usage.ingest(&bytes);
                    if !native_feedback_sent && sse_bytes_have_native_signal(&bytes) {
                        if let (Some(sessions), Some(route_key)) = (
                            context.codex_router_sessions.as_ref(),
                            context.history_context.codex_router_session_key.as_deref(),
                        ) {
                            crate::codex_router_session::refresh_native_track(
                                sessions,
                                route_key,
                                crate::accounts::now_secs(),
                                "response.stream_computer_signal",
                            );
                            native_feedback_sent = true;
                        }
                    }
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
        let had_stream_error = stream_error.is_some();
        let had_event_error = usage.saw_error_event;
        let error_msg = stream_error
            .or_else(|| usage.event_error_message.clone())
            .unwrap_or_else(|| {
                if context.http_status.is_success() {
                    String::new()
                } else {
                    format!("HTTP {}", context.http_status.as_u16())
                }
            });
        let status = if context.http_status.is_success() && error_msg.is_empty() && !had_event_error {
            "completed"
        } else {
            "failed"
        };
        if let Some(runtime_feedback) = context.runtime_feedback.as_ref() {
            if status == "completed" {
                runtime_feedback.success(&context.model).await;
            } else {
                let status_code = if had_stream_error || (context.http_status.is_success() && had_event_error) {
                    StatusCode::BAD_GATEWAY.as_u16()
                } else {
                    context.http_status.as_u16()
                };
                runtime_feedback
                    .failure(
                        &context.model,
                        status_code,
                        error_msg.clone(),
                        context.retry_after_secs,
                    )
                    .await;
            }
        }
        let native_failure_track_refreshed = maybe_refresh_failed_native_track(
            context.codex_router_sessions.as_ref(),
            context.history_context.codex_router_session_key.as_deref(),
            &context.history_context.route_trace,
            status == "failed",
            crate::accounts::now_secs(),
        );
        if native_failure_track_refreshed {
            tracing::warn!(
                route_key = %context.history_context.codex_router_session_key.as_deref().unwrap_or(""),
                model = %context.model,
                "DEX Router 原生 Responses 失败，保持 Computer Use 原生轨道"
            );
        }
        let mut observed_context =
            history_context_with_sse_observation(&context.history_context, &usage);
        if native_failure_track_refreshed {
            let original_trace = observed_context.route_trace.clone();
            observed_context.route_trace = patch_route_trace_field(
                Some(original_trace.clone()),
                "native_failure_keep_native",
                json!(true),
            )
            .unwrap_or(original_trace);
        }
        let _ = context
            .request_history
            .record(record_from_context(
                &observed_context,
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

fn maybe_refresh_failed_native_track(
    sessions: Option<&crate::codex_router_session::RouteStateMap>,
    route_key: Option<&str>,
    route_trace: &str,
    failed: bool,
    now: u64,
) -> bool {
    if !failed || !route_trace_force_native_responses(route_trace) {
        return false;
    }
    let (Some(sessions), Some(route_key)) = (sessions, route_key) else {
        return false;
    };
    crate::codex_router_session::refresh_native_track(
        sessions,
        route_key,
        now,
        "response.stream_failed_keep_native",
    );
    true
}

fn history_context_with_sse_observation(
    context: &HistoryContext,
    usage: &SseUsageObservation,
) -> HistoryContext {
    let mut context = context.clone();
    let force_native = route_trace_force_native_responses(&context.route_trace);
    let native_tool_not_emitted = force_native && !usage.saw_computer_call;
    let observation = usage.observation_value(native_tool_not_emitted);
    let original_trace = context.route_trace.clone();
    context.route_trace = patch_route_trace_field(
        Some(original_trace.clone()),
        "bypass_observation",
        observation,
    )
    .unwrap_or(original_trace);
    if native_tool_not_emitted {
        warn!(
            route_key = %context.codex_router_session_key.as_deref().unwrap_or(""),
            "DEX Router 原生 Responses 请求完成，但响应流未产出 computer_call"
        );
    }
    context
}

fn route_trace_force_native_responses(route_trace: &str) -> bool {
    serde_json::from_str::<Value>(route_trace)
        .ok()
        .and_then(|trace| {
            trace
                .get("session_route_force_native_responses")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false)
}

fn sse_bytes_have_native_signal(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            if crate::codex_router_session::response_has_native_signal(&value) {
                return true;
            }
        }
    }
    false
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
        route_trace: String::new(),
        codex_router_session_key: None,
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
    saw_native_signal: bool,
    saw_computer_call: bool,
    saw_computer_call_output: bool,
    saw_screenshot: bool,
    saw_error_event: bool,
    event_error_message: Option<String>,
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
        self.observe_native_value(&value);
    }

    fn observe_native_value(&mut self, value: &Value) {
        self.saw_computer_call |= json_value_has_type(value, &["computer_call"]);
        self.saw_computer_call_output |= json_value_has_type(value, &["computer_call_output"]);
        self.saw_screenshot |= json_value_has_key(value, "screenshot");
        self.saw_native_signal |= self.saw_computer_call
            || self.saw_computer_call_output
            || self.saw_screenshot
            || crate::codex_router_session::response_has_native_signal(value);
        if let Some(message) = sse_error_message_from_value(value) {
            self.saw_error_event = true;
            if self.event_error_message.is_none() {
                self.event_error_message = Some(truncate_router_failure_message(&message));
            }
        }
    }

    fn observation_value(&self, native_tool_not_emitted: bool) -> Value {
        json!({
            "native_signal": self.saw_native_signal,
            "computer_call": self.saw_computer_call,
            "computer_call_output": self.saw_computer_call_output,
            "screenshot": self.saw_screenshot,
            "error_event": self.saw_error_event,
            "error_event_message": self.event_error_message,
            "native_tool_not_emitted": native_tool_not_emitted,
        })
    }
}

fn json_value_has_type(value: &Value, expected: &[&str]) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| json_value_has_type(item, expected)),
        Value::Object(map) => {
            map.get("type")
                .and_then(Value::as_str)
                .is_some_and(|typ| expected.contains(&typ))
                || map
                    .values()
                    .any(|value| json_value_has_type(value, expected))
        }
        _ => false,
    }
}

fn json_value_has_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| json_value_has_key(item, key)),
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|value| json_value_has_key(value, key))
        }
        _ => false,
    }
}

fn sse_error_message_from_value(value: &Value) -> Option<String> {
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    let status_failed = value
        .get("status")
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("status"))
        })
        .and_then(Value::as_str)
        .is_some_and(|status| status == "failed");
    let error = value
        .get("error")
        .filter(|error| !error.is_null())
        .or_else(|| {
            value
                .get("response")
                .and_then(|response| response.get("error"))
                .filter(|error| !error.is_null())
        });
    if !event_type.contains("error")
        && !event_type.contains("failed")
        && !status_failed
        && error.is_none()
    {
        return None;
    }
    let error = error.unwrap_or(value);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string);
    message.or_else(|| {
        (!event_type.trim().is_empty())
            .then(|| event_type.trim().to_string())
            .or_else(|| Some("response failed".to_string()))
    })
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
            "/codex-router/v1/responses",
            post(handle_responses_codex_router),
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
            "/codex-router/v1/responses/compact",
            post(handle_compact_response),
        )
        .route(
            "/codex-desktop/v1/responses/input_tokens",
            post(handle_input_tokens),
        )
        .route(
            "/codex-router/v1/responses/input_tokens",
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
            "/codex-router/v1/responses/:response_id",
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
            "/codex-router/v1/responses/:response_id/cancel",
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
        .route(
            "/codex-router/v1/responses/:response_id/input_items",
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
        .route("/codex-router/v1/models", get(handle_models))
        .route("/health", get(handle_health))
        .route("/v1", get(handle_v1))
        .route("/codex-cli/v1", get(handle_v1))
        .route("/codex-desktop/v1", get(handle_v1))
        .route("/codex-router/v1", get(handle_v1))
        .route("/metrics", get(handle_metrics))
        .route("/api/router/status", get(handle_router_status_api))
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
    drop(model_map);
    if let Some(data) = codex_model_catalog_response_data() {
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

fn codex_model_catalog_response_data() -> Option<Vec<Value>> {
    let catalog_path = codex_config::codex_home_dir()?.join("models_deecodex.json");
    codex_model_catalog_response_data_from_path(&catalog_path)
}

fn codex_model_catalog_response_data_from_path(path: &std::path::Path) -> Option<Vec<Value>> {
    let content = std::fs::read_to_string(path).ok()?;
    let catalog: Value = serde_json::from_str(&content).ok()?;
    let mut data = Vec::new();
    let mut seen = HashSet::new();
    for model in catalog.get("models")?.as_array()? {
        if model
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        if model.get("visibility").and_then(Value::as_str) == Some("hidden") {
            continue;
        }
        let Some(id) = model
            .get("model")
            .or_else(|| model.get("slug"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
        else {
            continue;
        };
        if !seen.insert(id.to_string()) {
            continue;
        }
        data.push(json!({
            "id": id,
            "object": "model",
            "owned_by": model
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("deecodex")
        }));
    }
    (!data.is_empty()).then_some(data)
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

#[derive(Debug, Deserialize)]
struct RouterStatusQuery {
    #[serde(default = "default_router_status_model")]
    model: String,
    #[serde(default)]
    tools: Option<String>,
}

fn default_router_status_model() -> String {
    "gpt-5.5".into()
}

async fn handle_router_status_api(
    State(state): State<AppState>,
    Query(query): Query<RouterStatusQuery>,
) -> Response {
    refresh_account_store_from_disk(&state).await;

    let model = if query.model.trim().is_empty() {
        default_router_status_model()
    } else {
        query.model.trim().to_string()
    };
    let tools = router_status_tools_from_query(query.tools.as_deref());
    let store = state.account_store.read().await.clone();
    let now = crate::accounts::now_secs();
    Json(json!({
        "ok": true,
        "router": dex_router_status_snapshot_for_tools(&store, &model, now, &tools),
        "scenarios": dex_router_status_scenarios(&store, &model, now),
    }))
    .into_response()
}

fn router_status_tools_from_query(raw: Option<&str>) -> Vec<Value> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .filter_map(router_status_tool_for_slug)
        .collect()
}

fn router_status_tool_for_slug(slug: &str) -> Option<Value> {
    let slug = slug.trim().to_ascii_lowercase();
    match slug.as_str() {
        "web" | "web_search" | "web_search_preview" => Some(json!({"type": "web_search_preview"})),
        "file" | "file_search" | "file_search_preview" => Some(json!({"type": "file_search"})),
        "mcp" | "remote_mcp" => Some(json!({"type": "remote_mcp", "server_label": "diagnostic"})),
        "computer" | "computer_use" | "computer_use_preview" => {
            Some(json!({"type": "computer_use_preview"}))
        }
        "image" | "image_generation" | "image2" => Some(json!({"type": "image_generation"})),
        "function" | "tool" | "tools" => {
            Some(json!({"type": "function", "name": "diagnostic_tool"}))
        }
        _ => None,
    }
}

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
            "message": format!(
                "已归一 {} 条 Codex Desktop 线程到 {}",
                diff.changed_count,
                diff.target_provider
            ),
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "ok": false, "message": format!("归一失败: {e}") })),
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

async fn handle_responses_codex_router(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    tracing::info!(
        headers = %masked_router_headers(&headers),
        body_bytes = body.len(),
        "DEX Router 收到 Codex 请求，已打码官方登录态相关请求头"
    );
    let external_anchor = codex_router_external_anchor_from_headers(&headers);
    handle_responses_for_route_with_router_anchor(
        state,
        headers,
        body,
        AccountRouteSurface::CodexRouter,
        external_anchor,
    )
    .await
}

async fn handle_responses_dex_assistant(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Response {
    handle_responses_for_route(state, body, AccountRouteSurface::DexAssistant).await
}

fn maybe_schedule_codex_desktop_thread_normalization(
    state: &AppState,
    route_surface: AccountRouteSurface,
) {
    if !matches!(
        route_surface,
        AccountRouteSurface::CodexDesktop | AccountRouteSurface::CodexRouter
    ) {
        return;
    }

    let now = crate::accounts::now_secs();
    let last = CODEX_DESKTOP_THREAD_NORMALIZE_AT.load(Ordering::Relaxed);
    if now.saturating_sub(last) < CODEX_DESKTOP_THREAD_NORMALIZE_INTERVAL_SECS {
        return;
    }
    if CODEX_DESKTOP_THREAD_NORMALIZE_AT
        .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    let data_dir = state.data_dir.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(move || {
            crate::codex_threads::normalize_desktop_threads(data_dir.as_ref())
        })
        .await;
        match result {
            Ok(Ok(diff)) => {
                if diff.changed_count > 0
                    || diff.rollout_metadata_fixed_count > 0
                    || diff.remaining_non_unified_count > 0
                {
                    tracing::info!(
                        target_provider = %diff.target_provider,
                        changed = diff.changed_count,
                        rollout_metadata_fixed = diff.rollout_metadata_fixed_count,
                        remaining = diff.remaining_non_unified_count,
                        "Codex Desktop 请求触发线程归一完成"
                    );
                }
            }
            Ok(Err(err)) => {
                tracing::warn!("Codex Desktop 请求触发线程归一失败: {err}");
            }
            Err(err) => {
                tracing::warn!("Codex Desktop 请求触发线程归一任务失败: {err}");
            }
        }
    });
}

async fn handle_responses_for_route(
    state: AppState,
    body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
) -> Response {
    handle_responses_for_route_with_router_anchor(
        state,
        HeaderMap::new(),
        body,
        route_surface,
        None,
    )
    .await
}

async fn handle_responses_for_route_with_router_anchor(
    state: AppState,
    headers: HeaderMap,
    body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
    external_anchor: Option<CodexRouterExternalAnchor>,
) -> Response {
    maybe_schedule_codex_desktop_thread_normalization(&state, route_surface);
    let _start = std::time::Instant::now();
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
    let model = req.model.clone();
    let explicit_dex_account_model = codex_config::decode_dex_account_model_slug(&model).is_some();
    let codex_router_route_key = (route_surface == AccountRouteSurface::CodexRouter)
        .then(|| codex_router_session_route_key(&headers))
        .flatten();
    let mut router_tool_requirements = (route_surface == AccountRouteSurface::CodexRouter
        || explicit_dex_account_model)
        .then(|| router_tool_requirements(&req));
    let session_route_decision = if route_surface == AccountRouteSurface::CodexRouter {
        update_codex_router_session_route(
            &state,
            codex_router_route_key.clone(),
            router_tool_requirements.as_mut(),
            Some(&req.input),
        )
    } else {
        None
    };
    let session_main_model_anchor = codex_router_session_main_model_anchor(
        &state.codex_router_sessions,
        codex_router_route_key.as_deref(),
    );
    if let Some(requirements) = router_tool_requirements.as_ref() {
        let input_labels: Vec<&str> = requirements
            .labels
            .iter()
            .map(String::as_str)
            .filter(|label| label.starts_with("input."))
            .collect();
        if requirements.requires_computer && !input_labels.is_empty() {
            tracing::info!(
                requested_model = %model,
                input_signals = ?input_labels,
                "DEX Router 检测到 Computer Use 输入，强制要求 Responses 原生执行账号"
            );
        }
    }
    let mut route_selection = match resolve_account_endpoint_for_response(
        &state,
        route_surface,
        &model,
        router_tool_requirements.as_ref(),
        external_anchor.as_ref(),
        session_main_model_anchor.as_ref(),
    )
    .await
    {
        Ok(selection) => {
            record_codex_router_session_main_model_anchor(
                &state.codex_router_sessions,
                codex_router_route_key.as_deref(),
                &selection,
            );
            patch_selection_session_route_trace(selection, session_route_decision.as_ref())
        }
        Err(response) => return response,
    };
    if route_surface == AccountRouteSurface::CodexRouter && req.background != Some(true) {
        route_selection = apply_codex_router_preflight_model_fallback(&model, route_selection);
    }
    if route_surface == AccountRouteSurface::CodexRouter
        && !req.stream
        && req.background != Some(true)
    {
        return handle_codex_router_non_stream_with_fallback(
            state,
            req,
            body,
            route_selection,
            router_tool_requirements,
            session_route_decision,
        )
        .await;
    }

    handle_responses_for_selection(
        state,
        req,
        body,
        route_surface,
        route_selection,
        _start,
        session_route_decision.as_ref(),
    )
    .await
}

async fn handle_responses_for_selection(
    state: AppState,
    mut req: ResponsesRequest,
    mut body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
    mut route_selection: AccountRouteSelection,
    start: Instant,
    session_route_decision: Option<&DexRouterSessionRouteDecision>,
) -> Response {
    let endpoint = route_selection.endpoint.clone();
    if let Some(model) = route_selection.explicit_model.as_deref() {
        req.model = model.to_string();
        match patch_body_model_field(&body, model) {
            Ok(updated) => body = updated,
            Err(err) => {
                warn!("DEX 账号模型直选请求体模型替换失败，继续使用解析后的请求模型: {err}")
            }
        }
    }
    // 默认保留 Codex Desktop 原生工具链；只有账号显式选择“剥离并继续”时才移除历史残留。
    let model = req.model.clone();
    if endpoint.kind.is_responses_like() || endpoint.kind == EndpointKind::CodexOfficial {
        if let Err(response) = apply_endpoint_image_generation_declaration(
            &route_selection.account,
            &endpoint,
            &mut req,
            &mut body,
        ) {
            return *response;
        }
    }
    if route_selection.strip_native_computer_toolchain && endpoint.kind.is_chat_like() {
        let changed = strip_native_computer_toolchain_from_request(&mut req);
        route_selection.route_trace =
            patch_native_computer_strip_trace(route_selection.route_trace.take(), changed);
        match serde_json::to_vec(&req) {
            Ok(updated) => body = axum::body::Bytes::from(updated),
            Err(err) => {
                warn!("Computer Use 残留剥离后请求体序列化失败，继续使用解析后的请求: {err}")
            }
        }
        info!(
            account_id = %route_selection.account.id,
            endpoint_id = %endpoint.id,
            request_changed = changed,
            "已按账号设置剥离 Computer Use 原生工具链残留并继续 Chat 兼容请求"
        );
    }
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
        return handle_responses_bypass(
            state,
            req,
            body,
            route_surface,
            Some(route_selection),
            session_route_decision,
        )
        .await;
    }
    if endpoint.kind == EndpointKind::CodexOfficial {
        return handle_codex_official(
            state,
            req,
            body,
            route_surface,
            Some(route_selection),
            session_route_decision,
        )
        .await;
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
        Some(route_selection),
        session_route_decision,
    )
    .await;
    tracing::info!(
        "⇠ translate {} done in {}ms",
        model,
        start.elapsed().as_millis()
    );
    let status = response.status().as_u16().to_string();
    state
        .metrics
        .http_requests_total
        .with_label_values(&["POST", &status])
        .inc();
    response
}

async fn handle_codex_router_non_stream_with_fallback(
    state: AppState,
    req: ResponsesRequest,
    body: axum::body::Bytes,
    initial_selection: AccountRouteSelection,
    tool_requirements: Option<RouterToolRequirements>,
    session_route_decision: Option<DexRouterSessionRouteDecision>,
) -> Response {
    let mut selection = initial_selection;
    let mut excluded_account_ids: Vec<String> = Vec::new();
    let mut fallback_attempts: Vec<Value> = Vec::new();

    for attempt_index in 1..=DEX_ROUTER_MAX_NON_STREAM_ATTEMPTS {
        let response = handle_responses_for_selection(
            state.clone(),
            req.clone(),
            body.clone(),
            AccountRouteSurface::CodexRouter,
            selection.clone(),
            Instant::now(),
            session_route_decision.as_ref(),
        )
        .await;

        let status = response.status();
        if status.is_success()
            || !dex_router_retryable_status(status)
            || selection.explicit_account_model
            || selection.session_main_model_anchor
            || attempt_index >= DEX_ROUTER_MAX_NON_STREAM_ATTEMPTS
        {
            return response;
        }

        let failure = collect_router_attempt_failure(response).await;
        let retryable = dex_router_retryable_failure(&failure);
        if !retryable {
            return rebuild_response(failure.parts, failure.body);
        }
        let failed_account_id = selection.account.id.clone();
        excluded_account_ids.push(failed_account_id.clone());
        fallback_attempts.push(router_fallback_attempt_value(
            attempt_index,
            &selection,
            &req.model,
            &failure,
            retryable,
        ));

        let Some(next_selection) = resolve_dex_router_retry_selection(
            &state,
            &req.model,
            tool_requirements.as_ref(),
            &excluded_account_ids,
            &fallback_attempts,
            session_route_decision.as_ref(),
        )
        .await
        else {
            tracing::warn!(
                account_id = %failed_account_id,
                status = failure.status.as_u16(),
                "DEX Router 非流式降级无后续候选，返回最后一次上游失败"
            );
            return rebuild_response(failure.parts, failure.body);
        };

        tracing::warn!(
            from_account_id = %failed_account_id,
            to_account_id = %next_selection.account.id,
            status = failure.status.as_u16(),
            "DEX Router 非流式请求触发同请求降级"
        );
        selection = next_selection;
    }

    unreachable!("DEX Router fallback loop always returns");
}

fn apply_codex_router_preflight_model_fallback(
    requested_model: &str,
    selection: AccountRouteSelection,
) -> AccountRouteSelection {
    if selection.explicit_account_model || selection.session_main_model_anchor {
        return selection;
    }
    let now = crate::accounts::now_secs();
    if requested_model == GPT54_MODEL && endpoint_is_native_router_executor(&selection.endpoint) {
        let fallback = if selection.requires_computer {
            Some((
                GPT54_COMPUTER_FALLBACK_MODEL,
                "computer_use_gpt54_native_helper",
            ))
        } else {
            None
        };
        if let Some((fallback_model, reason)) = fallback {
            if account_runtime_ready_for_endpoint(
                &selection.account,
                &selection.endpoint,
                fallback_model,
                now,
            ) {
                tracing::warn!(
                    account_id = %selection.account.id,
                    account_name = %selection.account.name,
                    from_model = GPT54_MODEL,
                    to_model = fallback_model,
                    reason = reason,
                    "DEX Router 发送前切换 gpt-5.4 原生执行模型"
                );
                return apply_gpt54_fallback_to_selection(
                    selection,
                    requested_model,
                    fallback_model,
                    reason,
                );
            }
        }
    }
    selection
}

async fn resolve_dex_router_retry_selection(
    state: &AppState,
    requested_model: &str,
    tool_requirements: Option<&RouterToolRequirements>,
    excluded_account_ids: &[String],
    fallback_attempts: &[Value],
    session_route_decision: Option<&DexRouterSessionRouteDecision>,
) -> Option<AccountRouteSelection> {
    let store = state.account_store.read().await.clone();
    let cursor = DEX_ROUTER_POOL_CURSOR.fetch_add(1, Ordering::Relaxed);
    let (selection, route_trace) = dex_router_trace_for_selection_excluding(
        &store,
        requested_model,
        crate::accounts::now_secs(),
        cursor,
        tool_requirements,
        excluded_account_ids,
        "attempt_failed",
    );
    selection.map(|(account, endpoint)| {
        let effective_model =
            router_effective_model_for_account(&account, requested_model, &endpoint);
        AccountRouteSelection {
            account,
            endpoint,
            route_trace: patch_router_session_route_trace(
                patch_router_fallback_trace(Some(route_trace), fallback_attempts),
                session_route_decision,
            ),
            requires_computer: tool_requirements
                .is_some_and(|requirements| requirements.requires_computer),
            strip_native_computer_toolchain: false,
            explicit_model: (effective_model != requested_model).then_some(effective_model),
            explicit_account_model: false,
            session_main_model_anchor: false,
            main_model_anchor_to_record: None,
        }
    })
}

fn chat_message_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn task_loop_guard_active(model: &str, guard_label: Option<&str>) -> bool {
    guard_label.is_some() || providers::task_loop_guard_applies_to_identifier(model)
}

fn synthetic_task_loop_recovery_tool_call(response_id: &str) -> Value {
    json!({
        "id": format!("call_{response_id}_task_loop_recovery"),
        "type": "function",
        "function": {
            "name": "exec_command",
            "arguments": json!({
                "cmd": "pwd",
                "yield_time_ms": 1000,
                "max_output_tokens": 2000
            }).to_string()
        }
    })
}

fn maybe_inject_blocking_task_loop_recovery(
    chat_resp: &mut ChatResponse,
    response_id: &str,
    model: &str,
    guard_label: Option<&str>,
) -> bool {
    if !task_loop_guard_active(model, guard_label) {
        return false;
    }
    let Some(choice) = chat_resp.choices.first_mut() else {
        return false;
    };
    if choice
        .message
        .tool_calls
        .as_ref()
        .is_some_and(|calls| !calls.is_empty())
    {
        return false;
    }
    let text = chat_message_text(choice.message.content.as_ref());
    if !providers::should_recover_promised_tool_call_text(&text) {
        return false;
    }
    choice.message.tool_calls = Some(vec![synthetic_task_loop_recovery_tool_call(response_id)]);
    true
}

fn patch_router_fallback_trace(
    route_trace: Option<String>,
    fallback_attempts: &[Value],
) -> Option<String> {
    let mut trace = route_trace
        .and_then(|trace| serde_json::from_str::<Value>(&trace).ok())
        .unwrap_or_else(|| json!({}));
    if fallback_attempts.is_empty() {
        return serde_json::to_string(&trace).ok();
    }
    if let Some(obj) = trace.as_object_mut() {
        obj.insert("fallback_count".into(), json!(fallback_attempts.len()));
        obj.insert("fallback_attempts".into(), json!(fallback_attempts));
    }
    serde_json::to_string(&trace).ok()
}

fn router_fallback_attempt_value(
    attempt_index: usize,
    selection: &AccountRouteSelection,
    requested_model: &str,
    failure: &RouterAttemptFailure,
    retryable: bool,
) -> Value {
    json!({
        "attempt": attempt_index,
        "account_id": selection.account.id.clone(),
        "account_name": selection.account.name.clone(),
        "endpoint_id": selection.endpoint.id.clone(),
        "endpoint_kind": selection.endpoint.kind.label(),
        "mapped_model": router_effective_model(requested_model, &selection.endpoint),
        "effective_model": router_effective_model(requested_model, &selection.endpoint),
        "status": failure.status.as_u16(),
        "code": failure.code.clone(),
        "retryable": retryable,
        "message": failure.message.clone(),
    })
}

async fn collect_router_attempt_failure(response: Response) -> RouterAttemptFailure {
    let (parts, body) = response.into_parts();
    let status = parts.status;
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_else(|err| Bytes::from(format!("failed to collect response body: {err}")));
    let (code, message) = router_failure_details(status, &body);
    RouterAttemptFailure {
        status,
        code,
        message,
        body,
        parts,
    }
}

fn rebuild_response(parts: axum::http::response::Parts, body: Bytes) -> Response {
    Response::from_parts(parts, axum::body::Body::from(body))
}

fn dex_router_retryable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) || status.is_server_error()
}

fn dex_router_retryable_failure(failure: &RouterAttemptFailure) -> bool {
    if matches!(
        failure.code.as_str(),
        "rate_limited"
            | "invalid_request_error"
            | "unsupported_feature"
            | "unsupported_endpoint_mode"
            | "vision_disabled"
            | "request_builder_clone_failed"
    ) {
        return false;
    }
    dex_router_retryable_status(failure.status)
}

fn router_failure_details(status: StatusCode, body: &[u8]) -> (String, String) {
    let fallback_code = status.as_u16().to_string();
    let fallback_message = format!("HTTP {}", status.as_u16());
    if body.is_empty() {
        return (fallback_code, fallback_message);
    }
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        let error = value.get("error").unwrap_or(&value);
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|code| !code.is_empty())
            .unwrap_or(fallback_code.as_str())
            .to_string();
        if let Some(message) = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .map(str::trim)
            .filter(|message| !message.is_empty())
        {
            return (code, truncate_router_failure_message(message));
        }
    }
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        (fallback_code, fallback_message)
    } else {
        (fallback_code, truncate_router_failure_message(text))
    }
}

fn truncate_router_failure_message(message: &str) -> String {
    let mut chars = message.chars();
    let head: String = chars.by_ref().take(500).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
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

fn patch_body_input_and_remove_previous_response_id(
    body: &axum::body::Bytes,
    input_items: Vec<Value>,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let mut v: serde_json::Value = serde_json::from_slice(body)?;
    if let Some(obj) = v.as_object_mut() {
        obj.insert("input".to_string(), Value::Array(input_items));
        obj.remove("previous_response_id");
    }
    serde_json::to_vec(&v).map(axum::body::Bytes::from)
}

fn patch_router_native_bridge_trace(
    route_trace: &mut String,
    previous_response_id: &str,
    replayed_items: usize,
) {
    let mut trace = serde_json::from_str::<Value>(route_trace).unwrap_or_else(|_| json!({}));
    if let Some(obj) = trace.as_object_mut() {
        obj.insert(
            "native_bridge".into(),
            json!({
                "action": "replay_local_input_items",
                "previous_response_id": previous_response_id,
                "replayed_items": replayed_items,
                "removed_previous_response_id": true,
            }),
        );
    }
    if let Ok(next) = serde_json::to_string(&trace) {
        *route_trace = next;
    }
}

fn bridge_previous_response_for_native_turn(
    state: &AppState,
    req: &mut ResponsesRequest,
    body: &mut axum::body::Bytes,
    requires_computer: bool,
    route_trace: &mut Option<String>,
) {
    if !requires_computer {
        return;
    }
    let Some(previous_response_id) = req.previous_response_id.clone() else {
        return;
    };
    let Some(previous_items) = state.sessions.get_input_items(&previous_response_id) else {
        return;
    };
    if previous_items.is_empty() {
        return;
    }
    let mut input_items = previous_items;
    input_items.extend(response_input_items(req));
    match patch_body_input_and_remove_previous_response_id(body, input_items.clone()) {
        Ok(updated) => {
            *body = updated;
            req.previous_response_id = None;
            if let Some(trace) = route_trace.as_mut() {
                patch_router_native_bridge_trace(trace, &previous_response_id, input_items.len());
            }
        }
        Err(err) => {
            warn!(
                previous_response_id = %previous_response_id,
                error = %err,
                "无法桥接本地 previous_response_id 到原生 Responses，继续使用原请求体"
            );
        }
    }
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

fn patch_missing_function_call_namespaces(
    body: &axum::body::Bytes,
    req: &mut ResponsesRequest,
) -> Result<axum::body::Bytes, serde_json::Error> {
    let namespace_by_tool = namespace_tools_index(&req.tools);
    if namespace_by_tool.is_empty() {
        return Ok(body.clone());
    }

    let mut value: Value = serde_json::from_slice(body)?;
    let patched = value
        .get_mut("input")
        .map(|input| patch_missing_namespaces_in_value(input, &namespace_by_tool))
        .unwrap_or(false);
    if !patched {
        return Ok(body.clone());
    }

    if let Ok(updated_req) = serde_json::from_value::<ResponsesRequest>(value.clone()) {
        *req = updated_req;
    }
    serde_json::to_vec(&value).map(axum::body::Bytes::from)
}

fn namespace_tools_index(tools: &[Value]) -> HashMap<String, String> {
    let mut namespaces_by_tool: HashMap<String, HashSet<String>> = HashMap::new();
    for tool in tools {
        let Some(obj) = tool.as_object() else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("namespace") {
            continue;
        }
        let Some(namespace) = obj.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(sub_tools) = obj.get("tools").and_then(Value::as_array) else {
            continue;
        };
        for sub_tool in sub_tools {
            let Some(name) = sub_tool
                .get("name")
                .or_else(|| sub_tool.get("function").and_then(|f| f.get("name")))
                .and_then(Value::as_str)
            else {
                continue;
            };
            namespaces_by_tool
                .entry(name.to_string())
                .or_default()
                .insert(namespace.to_string());
        }
    }

    namespaces_by_tool
        .into_iter()
        .filter_map(|(tool, namespaces)| {
            if namespaces.len() == 1 {
                namespaces
                    .into_iter()
                    .next()
                    .map(|namespace| (tool, namespace))
            } else {
                None
            }
        })
        .collect()
}

fn patch_missing_namespaces_in_value(
    value: &mut Value,
    namespace_by_tool: &HashMap<String, String>,
) -> bool {
    match value {
        Value::Object(map) => {
            let mut patched = false;
            let is_function_call = map
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|typ| typ == "function_call");
            let missing_namespace = !map.contains_key("namespace");
            if is_function_call && missing_namespace {
                if let Some(namespace) = map
                    .get("name")
                    .and_then(Value::as_str)
                    .and_then(|name| namespace_by_tool.get(name))
                {
                    map.insert("namespace".into(), Value::String(namespace.clone()));
                    patched = true;
                }
            }
            for child in map.values_mut() {
                patched |= patch_missing_namespaces_in_value(child, namespace_by_tool);
            }
            patched
        }
        Value::Array(items) => {
            let mut patched = false;
            for item in items {
                patched |= patch_missing_namespaces_in_value(item, namespace_by_tool);
            }
            patched
        }
        _ => false,
    }
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

fn append_ocr_fallback_message(chat_req: &mut ChatRequest, ocr: local_ocr::OcrFallbackReport) {
    if ocr.image_count == 0 {
        return;
    }
    let content = if ocr.text.trim().is_empty() {
        format!(
            "用户上传了 {} 张图片。当前 Chat 兼容模型不支持图片输入，DEX AI 已剥离图片并尝试本机 OCR，但未识别到可用文字。请基于现有文本继续；不要声称已经看到图片细节。",
            ocr.image_count
        )
    } else {
        format!(
            "用户上传了 {} 张图片。当前 Chat 兼容模型不支持图片输入，DEX AI 已剥离图片，并用本机 OCR 提取到以下文字。OCR 仅代表图片中的可识别文字，不包含颜色、布局、物体或图形关系：\n\n{}",
            ocr.image_count,
            ocr.text.trim()
        )
    };
    chat_req.messages.push(ChatMessage {
        role: "user".into(),
        content: Some(Value::String(content)),
        reasoning_content: None,
        reasoning_details: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
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
    mut body: axum::body::Bytes,
    route_surface: AccountRouteSurface,
    selected_endpoint: Option<AccountRouteSelection>,
    session_route_decision: Option<&DexRouterSessionRouteDecision>,
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
    let mut route_trace = selected_endpoint
        .as_ref()
        .and_then(|selection| selection.route_trace.clone());
    let requires_computer = selected_endpoint
        .as_ref()
        .is_some_and(|selection| selection.requires_computer);
    let (account, endpoint) = match selected_endpoint {
        Some(selection) => (selection.account, selection.endpoint),
        None => active_account_endpoint_for_route(&state, route_surface, Some(&req.model)).await,
    };
    let mut history_context =
        history_context_for(&account, &endpoint, route_surface.responses_path());
    attach_codex_router_session_key(&mut history_context, session_route_decision);
    if let Some(trace) = route_trace.clone() {
        history_context.route_trace = trace;
    }
    bridge_previous_response_for_native_turn(
        &state,
        &mut req,
        &mut body,
        requires_computer,
        &mut route_trace,
    );
    if let Some(trace) = route_trace {
        history_context.route_trace = trace;
    }
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
    body = patch_missing_function_call_namespaces(&body, &mut req).unwrap_or(body);
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
            UnsupportedImagePolicy::OcrThenStrip => {
                warn!("Responses 直连端点不执行本机 OCR，已按配置剥离图片后继续请求");
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
    let runtime_feedback = runtime_feedback_for_account_endpoint(&state, &account, &endpoint);
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
        runtime_feedback,
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
    selected_endpoint: Option<AccountRouteSelection>,
    session_route_decision: Option<&DexRouterSessionRouteDecision>,
) -> Response {
    let original_model = req.model.clone();
    let route_trace = selected_endpoint
        .as_ref()
        .and_then(|selection| selection.route_trace.clone());
    let explicit_model = selected_endpoint
        .as_ref()
        .and_then(|selection| selection.explicit_model.clone());
    let (mut account, endpoint) = match selected_endpoint {
        Some(selection) => (selection.account, selection.endpoint),
        None => {
            let Some(selection) =
                codex_official_account_endpoint(&state, &original_model, route_surface).await
            else {
                return codex_official_pool_unavailable_response();
            };
            selection
        }
    };
    let mut history_context =
        history_context_for(&account, &endpoint, route_surface.responses_path());
    attach_codex_router_session_key(&mut history_context, session_route_decision);
    if let Some(trace) = route_trace {
        history_context.route_trace = trace;
    }
    let mapped_model = explicit_model.unwrap_or_else(|| original_model.clone());
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
                codex_router_sessions: Some(state.codex_router_sessions.clone()),
                response_id,
                model: mapped_model,
                start,
                upstream_url: url,
                http_status: status,
                retry_after_secs: retry_after,
                runtime_feedback: None,
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
    if status.is_success() {
        if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
            refresh_codex_router_session_from_response(&state, &history_context, &value);
        }
    }
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
    crate::runtime_feedback::record_runtime_result(
        state.data_dir.clone(),
        state.account_store.clone(),
        state.active_account.clone(),
        crate::runtime_feedback::RuntimeFeedbackRecord {
            account_id: account_id.to_string(),
            model: model.to_string(),
            status_code: status.as_u16(),
            message,
            retry_after_secs,
            cooldown_managed: true,
        },
    )
    .await;
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

fn runtime_message_from_response_body(status: StatusCode, body: &Value) -> String {
    let fallback = format!("HTTP {}", status.as_u16());
    body.get("error")
        .and_then(|error| error.get("message").or(Some(error)))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(str::to_string)
        .unwrap_or(fallback)
}

fn runtime_message_from_response_text(status: StatusCode, body: &str) -> String {
    let fallback = format!("HTTP {}", status.as_u16());
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return fallback;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return runtime_message_from_response_body(status, &value);
    }
    if trimmed.starts_with('<') {
        return fallback;
    }
    truncate_router_failure_message(trimmed)
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
        runtime_feedback,
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
            runtime_feedback
                .failure(
                    &model,
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    message.clone(),
                    None,
                )
                .await;
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
                runtime_feedback
                    .failure(
                        &model,
                        StatusCode::BAD_GATEWAY.as_u16(),
                        format!("connection error: {e}"),
                        None,
                    )
                    .await;
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
    let retry_after = retry_after_secs(resp.headers());
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
        let retry_after = retry_after_secs(resp.headers());
        let raw_error_body = resp.text().await.unwrap_or_default();
        let history_error_msg = runtime_message_from_response_text(status, &raw_error_body);
        runtime_feedback
            .failure(
                &model,
                status.as_u16(),
                history_error_msg.clone(),
                retry_after,
            )
            .await;
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
                history_error_msg,
                false,
            ))
            .await;
        // 上游可能返回 HTML，转为 JSON 错误
        let error_body = if raw_error_body.trim_start().starts_with('<') {
            format!("upstream returned HTTP {}", status.as_u16())
        } else {
            raw_error_body
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
            codex_router_sessions: Some(state.codex_router_sessions.clone()),
            response_id,
            model,
            start,
            upstream_url: url,
            http_status: status,
            retry_after_secs: retry_after,
            runtime_feedback: Some(runtime_feedback),
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
        runtime_feedback,
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
            runtime_feedback
                .failure(
                    &model,
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    message,
                    None,
                )
                .await;
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
            runtime_feedback
                .failure(
                    &model,
                    StatusCode::BAD_GATEWAY.as_u16(),
                    format!("connection error: {e}"),
                    None,
                )
                .await;
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
            let retry_after = retry_after_secs(resp.headers());
            let response_body: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    error!("bypass non-stream JSON parse: {e}");
                    runtime_feedback
                        .failure(
                            &model,
                            StatusCode::BAD_GATEWAY.as_u16(),
                            format!("failed to parse upstream response: {e}"),
                            None,
                        )
                        .await;
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
                let history_error_msg = if status.is_success() {
                    String::new()
                } else {
                    runtime_message_from_response_body(status, &response_body)
                };
                state
                    .request_history
                    .record(record_from_context(
                        &history_context,
                        response_id,
                        model.clone(),
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
                        history_error_msg.clone(),
                        cache_hit,
                    ))
                    .await;
                if status.is_success() {
                    refresh_codex_router_session_from_response(
                        &state,
                        &history_context,
                        &response_body,
                    );
                    runtime_feedback.success(&model).await;
                } else {
                    runtime_feedback
                        .failure(&model, status.as_u16(), history_error_msg, retry_after)
                        .await;
                }
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

#[allow(clippy::too_many_arguments)]
async fn handle_responses_inner(
    state: AppState,
    req: ResponsesRequest,
    raw_body: axum::body::Bytes,
    local_output_prefix_items: Vec<Value>,
    local_input_suffix_items: Vec<Value>,
    route_surface: AccountRouteSurface,
    selected_endpoint: Option<AccountRouteSelection>,
    session_route_decision: Option<&DexRouterSessionRouteDecision>,
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
    let route_trace = selected_endpoint
        .as_ref()
        .and_then(|selection| selection.route_trace.clone());
    let explicit_model = selected_endpoint
        .as_ref()
        .and_then(|selection| selection.explicit_model.clone());
    let (account, endpoint) = match selected_endpoint {
        Some(selection) => (selection.account, selection.endpoint),
        None => {
            active_account_endpoint_for_route(&state, route_surface, Some(&original_model)).await
        }
    };
    let mut history_context =
        history_context_for(&account, &endpoint, route_surface.responses_path());
    attach_codex_router_session_key(&mut history_context, session_route_decision);
    if let Some(trace) = route_trace {
        history_context.route_trace = trace;
    }
    let runtime_feedback = runtime_feedback_for_account_endpoint(&state, &account, &endpoint);
    let model_map = if route_surface.uses_codex_direct_models() {
        ModelMap::new()
    } else {
        endpoint.model_map.clone()
    };
    let explicit_model_for_identity = explicit_model.clone();
    let mapped_model = explicit_model.unwrap_or_else(|| {
        if route_surface.uses_codex_direct_models() {
            original_model.clone()
        } else if endpoint.kind.is_chat_like() {
            translated_chat_effective_model_for_account(&account, &original_model, &endpoint)
        } else {
            resolve_model(&original_model, &model_map)
        }
    });
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
    chat_req.model = mapped_model.clone();
    if let Some(ref forced) = endpoint.reasoning_effort_override {
        chat_req.reasoning_effort = Some(forced.clone());
        chat_req.thinking = Some(serde_json::json!({"type": "enabled"}));
    }
    if let Some(budget) = endpoint.thinking_tokens {
        if let Some(ref mut thinking) = chat_req.thinking {
            thinking["budget_tokens"] = serde_json::json!(budget);
        }
    }

    // Route to VLM when the current turn has new images (not just history carrying old ones)
    let is_review_model = original_model.contains("auto-review");
    let has_new_image = response_input_has_new_image(&req.input);
    let vision_mode = if is_review_model {
        VisionMode::Off
    } else {
        endpoint.model_vision_mode(&mapped_model)
    };
    let route_to_vision = translated.has_images && has_new_image && vision_mode == VisionMode::Glue;
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
            UnsupportedImagePolicy::OcrThenStrip => {
                warn!("当前 Chat 兼容端点未启用视觉能力，已使用本机 OCR 降级后继续请求");
            }
        }
    }

    // 非原生视觉端点必须剥离 image_url，否则 DeepSeek 等上游会拒绝。
    if !route_to_vision && !native_vision {
        let should_ocr = translated.has_images
            && has_new_image
            && vision_mode == VisionMode::Off
            && endpoint.vision.unsupported_image_policy == UnsupportedImagePolicy::OcrThenStrip
            && endpoint.kind.is_chat_like();
        let ocr_report = if should_ocr {
            match serde_json::from_slice::<Value>(&raw_body) {
                Ok(value) => Some(local_ocr::recognize_images_from_value(&value).await),
                Err(err) => {
                    warn!(error = %err, "解析原始请求失败，本机 OCR 降级跳过并改为剥离图片");
                    None
                }
            }
        } else {
            None
        };
        strip_images_from_chat_request(&mut chat_req);
        if let Some(report) = ocr_report {
            append_ocr_fallback_message(&mut chat_req, report);
        }
    }

    maybe_inject_explicit_model_identity(
        &mut chat_req.messages,
        &account,
        &endpoint,
        explicit_model_for_identity.as_deref(),
    );

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
                        reasoning_details: None,
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
    let provider_profile = providers::profile_for_account(&account);
    let task_loop_guard_label =
        providers::task_loop_guard_label(&provider_profile).map(str::to_string);
    providers::adapt_chat_request(&provider_profile, &mut chat_req);
    let adapted_reasoning_effort = chat_req.reasoning_effort.clone();
    let adapted_thinking = chat_req.thinking.clone();
    let adapted_reasoning_split = chat_req.reasoning_split;
    let adapted_stream_options = chat_req.stream_options.clone();
    // minimax 协议调试：打印翻译后发往上游的关键字段，便于对照上游 OpenAPI schema
    if provider_profile.slug == "minimax" {
        let last_msg = chat_req.messages.last();
        let last_role = last_msg.map(|m| m.role.as_str()).unwrap_or("");
        let last_content = last_msg
            .and_then(|m| m.content.as_ref())
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(200).collect::<String>())
            .unwrap_or_default();
        info!(
            "minimax upstream request fields: model={} reasoning_effort={:?} \
             thinking={:?} reasoning_split={:?} stream_options={:?} msg_count={} \
             tool_count={} last_role={} last_content_head={:?}",
            chat_req.model,
            adapted_reasoning_effort,
            adapted_thinking,
            adapted_reasoning_split,
            adapted_stream_options.as_ref().map(|o| o.include_usage),
            chat_req.messages.len(),
            chat_req.tools.len(),
            last_role,
            last_content,
        );
    }
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
            runtime_feedback: runtime_feedback.clone(),
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
        let bg_runtime_feedback = runtime_feedback.clone();
        let bg_task_loop_guard_label = task_loop_guard_label.clone();
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
                runtime_feedback: bg_runtime_feedback,
                task_loop_guard_label: bg_task_loop_guard_label,
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
            codex_router_sessions: Some(state.codex_router_sessions.clone()),
            upstream_url: url,
            allow_missing_done: providers::profile_for_account(&account)
                .capabilities
                .allow_missing_done,
            task_loop_guard_label: task_loop_guard_label.clone(),
            runtime_feedback: runtime_feedback.clone(),
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
            runtime_feedback,
            task_loop_guard_label,
            start,
        })
        .await;
        let elapsed = start.elapsed();
        debug!("blocking request completed in {:.0}ms", elapsed.as_millis());
        resp
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
    enum BlockingUpstreamResponse {
        Success(reqwest::Response),
        Error {
            status: StatusCode,
            retry_after: Option<u64>,
            body: String,
        },
    }

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
        runtime_feedback,
        task_loop_guard_label,
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
    let mut disable_web_search_retry = false;
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
                runtime_feedback: runtime_feedback.clone(),
                retry_after_secs: None,
            })
            .await;
        };
        let req_to_send = if disable_web_search_retry {
            let mut fallback_req = chat_req.clone();
            providers::strip_web_search_tool(&mut fallback_req);
            fallback_req
        } else {
            chat_req.clone()
        };
        match request.json(&req_to_send).send().await {
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
            Ok(r) if !r.status().is_success() => {
                let status = r.status();
                let retry_after = retry_after_secs(r.headers());
                let body = r.text().await.unwrap_or_default();
                let web_search_disabled_error =
                    providers::is_mimo_web_search_disabled_error(status.as_u16(), &body);
                if web_search_disabled_error
                    && !disable_web_search_retry
                    && providers::has_web_search_tool(&chat_req)
                    && attempt < max_retries
                {
                    attempt += 1;
                    disable_web_search_retry = true;
                    warn!(
                        "upstream {} rejected MiMo web_search, retrying without web_search tool in {delay_ms}ms",
                        status.as_u16()
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    delay_ms *= 2;
                    continue;
                }
                break Ok(BlockingUpstreamResponse::Error {
                    status,
                    retry_after,
                    body,
                });
            }
            Ok(r) => break Ok(BlockingUpstreamResponse::Success(r)),
        }
    };

    match result {
        Err(e) => {
            error!("upstream error: {e}");
            runtime_feedback
                .failure(
                    &model,
                    StatusCode::BAD_GATEWAY.as_u16(),
                    e.to_string(),
                    None,
                )
                .await;
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
        Ok(BlockingUpstreamResponse::Error {
            status,
            retry_after,
            body,
        }) => {
            error!("upstream {}: {}", status.as_u16(), body);
            runtime_feedback
                .failure(&model, status.as_u16(), body.clone(), retry_after)
                .await;
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
        Ok(BlockingUpstreamResponse::Success(r)) => match r.json::<ChatResponse>().await {
            Err(e) => {
                error!("parse error: {e}");
                runtime_feedback
                    .failure(
                        &model,
                        StatusCode::BAD_GATEWAY.as_u16(),
                        format!("failed to parse upstream response: {e}"),
                        None,
                    )
                    .await;
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
            Ok(mut chat_resp) => {
                runtime_feedback.success(&model).await;
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

                if maybe_inject_blocking_task_loop_recovery(
                    &mut chat_resp,
                    &response_id,
                    &model,
                    task_loop_guard_label.as_deref(),
                ) {
                    warn!(
                        "{} promised a follow-up tool action without tool calls in blocking mode; injecting recovery tool call.",
                        task_loop_guard_label
                            .as_deref()
                            .unwrap_or("upstream model")
                    );
                }

                let assistant_msg = chat_resp
                    .choices
                    .first()
                    .map(|c| c.message.clone())
                    .unwrap_or_else(|| ChatMessage {
                        role: "assistant".into(),
                        content: Some(serde_json::Value::String(String::new())),
                        reasoning_content: None,
                        reasoning_details: None,
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
                if assistant_msg.reasoning_details.is_some() {
                    state
                        .sessions
                        .store_turn_reasoning_details(&chat_req.messages, &assistant_msg);
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
                    refresh_codex_router_session_from_response(&state, &history_context, &value);
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
        runtime_feedback,
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
                runtime_feedback: runtime_feedback.clone(),
                retry_after_secs: None,
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
                runtime_feedback: runtime_feedback.clone(),
                retry_after_secs: None,
            })
            .await
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let retry_after = retry_after_secs(r.headers());
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
                runtime_feedback: runtime_feedback.clone(),
                retry_after_secs: retry_after,
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
                    runtime_feedback: runtime_feedback.clone(),
                    retry_after_secs: None,
                })
                .await
            }
            Ok(value) => {
                runtime_feedback.success(&model).await;
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
                        reasoning_details: None,
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
        runtime_feedback,
        retry_after_secs,
    } = args;
    runtime_feedback
        .failure(&model, status.as_u16(), message.clone(), retry_after_secs)
        .await;
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
    fn codex_models_endpoint_uses_generated_catalog_entries() {
        let dir = std::env::temp_dir().join(format!(
            "deecodex-model-catalog-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("models_deecodex.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "models": [
                    {
                        "slug": "dexacct.account.endpoint.deepseek",
                        "model": "dexacct.account.endpoint.deepseek",
                        "provider": "deecodex",
                        "visibility": "list",
                        "hidden": false
                    },
                    {
                        "slug": "gpt-5.5",
                        "provider": "dex_router",
                        "visibility": "list",
                        "hidden": false
                    },
                    {
                        "slug": "hidden-model",
                        "model": "hidden-model",
                        "provider": "dex_router",
                        "visibility": "hidden",
                        "hidden": false
                    },
                    {
                        "slug": "also-hidden",
                        "model": "also-hidden",
                        "provider": "dex_router",
                        "visibility": "list",
                        "hidden": true
                    }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let data = codex_model_catalog_response_data_from_path(&path).unwrap();
        let ids = data
            .iter()
            .filter_map(|model| model.get("id").and_then(Value::as_str))
            .collect::<Vec<_>>();

        assert!(ids.contains(&"dexacct.account.endpoint.deepseek"));
        assert!(ids.contains(&"gpt-5.5"));
        assert!(!ids.contains(&"hidden-model"));
        assert!(!ids.contains(&"also-hidden"));
        assert_eq!(data[0]["owned_by"], "deecodex");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dex_router_masks_sensitive_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_static("Bearer secret-token"),
        );
        headers.insert(
            header::HeaderName::from_static("chatgpt-account-id"),
            header::HeaderValue::from_static("acct-secret"),
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("codex_desktop/1.0"),
        );

        let masked = masked_router_headers(&headers);

        assert_eq!(masked["authorization"], "<redacted>");
        assert_eq!(masked["chatgpt-account-id"], "<redacted>");
        assert_eq!(masked["user-agent"], "codex_desktop/1.0");
    }

    #[test]
    fn dex_router_uses_desktop_surface() {
        assert_eq!(
            AccountRouteSurface::CodexRouter.explicit_surface(),
            Some(AccountClientSurface::Desktop)
        );
        assert_eq!(
            AccountRouteSurface::CodexRouter.responses_path(),
            "/codex-router/v1/responses"
        );
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
                "base_url": "https://chatgpt.com/backend-api/codex"
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

    fn router_chat_account(
        id: &str,
        pool: &str,
        priority: i64,
        weight: u32,
        mapped_model: Option<&str>,
    ) -> Account {
        let model_map = mapped_model
            .map(|model| json!({"gpt-5": model}))
            .unwrap_or_else(|| json!({}));
        let mut account: Account = serde_json::from_value(json!({
            "id": id,
            "name": format!("Router {id}"),
            "provider": "openrouter",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://openrouter.ai/api/v1",
            "api_key": format!("token-{id}"),
            "endpoints": [{
                "id": format!("ep-{id}"),
                "name": "Chat",
                "kind": "open_ai_chat",
                "base_url": "https://openrouter.ai/api/v1",
                "model_map": model_map
            }]
        }))
        .unwrap();
        crate::accounts::set_account_routing_options(
            &mut account,
            crate::accounts::AccountRoutingOptions {
                pool: pool.into(),
                priority,
                weight,
                ..Default::default()
            },
        );
        account
    }

    fn router_responses_account(id: &str, pool: &str, priority: i64, weight: u32) -> Account {
        let mut account: Account = serde_json::from_value(json!({
            "id": id,
            "name": format!("Router {id}"),
            "provider": "openai",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://api.example.com/v1",
            "api_key": format!("token-{id}"),
            "endpoints": [{
                "id": format!("ep-{id}"),
                "name": "Responses",
                "kind": "open_ai_responses",
                "base_url": "https://api.example.com/v1"
            }]
        }))
        .unwrap();
        crate::accounts::set_account_routing_options(
            &mut account,
            crate::accounts::AccountRoutingOptions {
                pool: pool.into(),
                priority,
                weight,
                ..Default::default()
            },
        );
        account
    }

    fn router_official_anchor_account(id: &str, pool: &str, priority: i64, weight: u32) -> Account {
        let mut account: Account = serde_json::from_value(json!({
            "id": id,
            "name": format!("Official {id}"),
            "provider": "codex",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://chatgpt.com/backend-api/codex",
            "api_key": format!("token-{id}"),
            "auth_mode": "oauth",
            "endpoints": [{
                "id": format!("ep-{id}"),
                "name": "Codex 官方",
                "kind": "codex_official",
                "base_url": "https://chatgpt.com/backend-api/codex"
            }]
        }))
        .unwrap();
        crate::accounts::set_account_routing_options(
            &mut account,
            crate::accounts::AccountRoutingOptions {
                pool: pool.into(),
                priority,
                weight,
                ..Default::default()
            },
        );
        account
    }

    fn enable_official_execution(account: &mut Account) {
        let mut routing = crate::accounts::account_routing_options(account);
        routing.execution_enabled = Some(true);
        crate::accounts::set_account_routing_options(account, routing);
    }

    fn router_store(accounts: Vec<Account>, active_id: &str) -> AccountStore {
        let mut accounts = accounts;
        for account in &mut accounts {
            if account.id == active_id {
                let mut routing = crate::accounts::account_routing_options(account);
                routing.anchor_enabled = Some(true);
                crate::accounts::set_account_routing_options(account, routing);
            }
        }
        let mut store = AccountStore {
            version: crate::accounts::ACCOUNT_STORE_VERSION,
            accounts,
            active_id: Some(active_id.into()),
            active_account_id: Some(active_id.into()),
            active_endpoint_id: Some(format!("ep-{active_id}")),
            active_by_surface: HashMap::new(),
        };
        store.set_active_for_surface(
            &AccountClientKind::Codex,
            &AccountClientSurface::Desktop,
            active_id.into(),
            Some(format!("ep-{active_id}")),
        );
        store
    }

    fn responses_request_with_tools(tools: Vec<Value>) -> ResponsesRequest {
        ResponsesRequest {
            model: "gpt-5".into(),
            input: ResponsesInput::Messages(vec![]),
            previous_response_id: None,
            tools,
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
        }
    }

    fn responses_request_with_tool_choice(
        tools: Vec<Value>,
        tool_choice: Value,
    ) -> ResponsesRequest {
        let mut req = responses_request_with_tools(tools);
        req.tool_choice = Some(tool_choice);
        req
    }

    #[test]
    fn dex_router_selects_highest_priority_account_in_active_pool() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let high = router_responses_account("high", "pool-a", 10, 1);
        let other_pool = router_responses_account("other", "pool-b", 100, 1);
        let store = router_store(vec![active, high, other_pool], "active");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "high");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
    }

    #[test]
    fn dex_router_uses_official_account_as_anchor_only_by_default() {
        let official = router_official_anchor_account("official", "pool-a", 100, 1);
        let executor = router_responses_account("executor", "pool-a", 10, 1);
        let store = router_store(vec![official, executor], "official");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "executor");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);

        let snapshot = dex_router_status_snapshot(&store, "gpt-5", 1_000);
        assert_eq!(snapshot["anchor"]["account_id"], "official");
        assert_eq!(snapshot["anchor"]["anchor_enabled"], true);
        assert_eq!(snapshot["anchor"]["execution_enabled"], false);
        assert_eq!(snapshot["selected"]["account_id"], "executor");
        let official_candidate = snapshot["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["account_id"] == "official")
            .unwrap();
        assert_eq!(official_candidate["reason"], "execution_disabled");
        assert_eq!(official_candidate["eligible"], false);
    }

    #[test]
    fn dex_router_reuses_external_codex_login_headers_as_anchor() {
        let executor = router_responses_account("executor", "codex-official", 10, 1);
        let store = AccountStore {
            version: crate::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![executor],
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
            active_by_surface: HashMap::new(),
        };
        assert!(select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).is_none());

        let external_anchor = CodexRouterExternalAnchor {
            account_id: Some("acct_external".into()),
        };
        let store = codex_router_store_with_external_anchor(store, Some(&external_anchor));
        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "executor");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        let snapshot = dex_router_status_snapshot(&store, "gpt-5", 1_000);
        assert_eq!(
            snapshot["anchor"]["account_id"],
            "__codex_desktop_login_anchor__"
        );
        assert_eq!(snapshot["anchor"]["execution_enabled"], false);
        assert_eq!(snapshot["selected"]["account_id"], "executor");
    }

    #[test]
    fn dex_router_uses_weighted_round_robin_at_same_priority() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let alpha = router_responses_account("alpha", "pool-a", 20, 1);
        let beta = router_responses_account("beta", "pool-a", 20, 3);
        let store = router_store(vec![active, alpha, beta], "active");

        let (first, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_200, 0, None).unwrap();
        let (second, _) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_200, 1, None).unwrap();

        assert_eq!(first.id, "alpha");
        assert_eq!(second.id, "beta");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
    }

    #[test]
    fn dex_router_keeps_non_quota_retry_account_eligible() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut cooled = router_responses_account("cooled", "pool-a", 100, 1);
        cooled.runtime_state.status = AccountRuntimeStatus::CoolingDown;
        cooled.runtime_state.status_message = "HTTP 401".into();
        cooled.runtime_state.next_retry_after = Some(2_000);
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, cooled, ready], "active");

        let (account, _) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "cooled");
    }

    #[test]
    fn dex_router_keeps_unmanaged_direct_quota_error_eligible() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut quota = router_responses_account("quota", "pool-a", 100, 1);
        quota.runtime_state.status = AccountRuntimeStatus::QuotaExceeded;
        quota.runtime_state.status_message = "HTTP 429".into();
        quota.runtime_state.next_retry_after = Some(2_000);
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, quota, ready], "active");

        let (account, _) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "quota");
    }

    #[test]
    fn dex_router_skips_managed_official_quota_exceeded_account() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut quota = router_official_anchor_account("quota", "pool-a", 100, 1);
        enable_official_execution(&mut quota);
        quota.runtime_state.status = AccountRuntimeStatus::QuotaExceeded;
        quota.runtime_state.status_message = "HTTP 429".into();
        quota.runtime_state.next_retry_after = Some(2_000);
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, quota, ready], "active");

        let (account, _) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "ready");
    }

    #[test]
    fn dex_router_keeps_unmanaged_direct_transient_5xx_eligible() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut transient = router_responses_account("transient", "pool-a", 100, 1);
        transient.runtime_state.status = AccountRuntimeStatus::CoolingDown;
        transient.runtime_state.status_message = "HTTP 502".into();
        transient.runtime_state.next_retry_after = Some(2_000);
        transient.runtime_state.model_states.insert(
            "model-transient".into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::CoolingDown,
                status_message: "HTTP 502".into(),
                next_retry_after: Some(2_000),
                quota: Default::default(),
                updated_at: 1_000,
            },
        );
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, transient, ready], "active");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "transient");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 2_000, 0, None).unwrap();

        assert_eq!(account.id, "transient");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
    }

    #[test]
    fn dex_router_keeps_non_quota_model_retry_eligible() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut model_cooled = router_responses_account("model-cooled", "pool-a", 100, 1);
        model_cooled.runtime_state.model_states.insert(
            "gpt-5".into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::CoolingDown,
                status_message: "模型冷却".into(),
                next_retry_after: Some(2_000),
                quota: Default::default(),
                updated_at: 1_000,
            },
        );
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, model_cooled, ready], "active");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "model-cooled");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
    }

    #[test]
    fn dex_router_keeps_unmanaged_direct_model_level_quota_eligible() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut model_quota = router_responses_account("model-quota", "pool-a", 100, 1);
        model_quota.runtime_state.model_states.insert(
            "gpt-5".into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::QuotaExceeded,
                status_message: "HTTP 429".into(),
                next_retry_after: Some(2_000),
                quota: Default::default(),
                updated_at: 1_000,
            },
        );
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, model_quota, ready], "active");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "model-quota");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
    }

    #[test]
    fn dex_router_skips_managed_official_model_level_quota() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut model_quota = router_official_anchor_account("model-quota", "pool-a", 100, 1);
        enable_official_execution(&mut model_quota);
        model_quota.runtime_state.model_states.insert(
            "gpt-5".into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::QuotaExceeded,
                status_message: "HTTP 429".into(),
                next_retry_after: Some(2_000),
                quota: Default::default(),
                updated_at: 1_000,
            },
        );
        let ready = router_responses_account("ready", "pool-a", 10, 1);
        let store = router_store(vec![active, model_quota, ready], "active");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "ready");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
    }

    #[test]
    fn dex_router_ignores_chat_mapping_for_plain_codex_models() {
        let active = router_chat_account("active", "pool-a", 0, 1, Some("model-active"));
        let unmapped = router_chat_account("unmapped", "pool-a", 100, 1, None);
        let responses = router_responses_account("responses", "pool-a", 50, 1);
        let store = router_store(vec![active, unmapped, responses], "active");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();

        assert_eq!(account.id, "unmapped");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiChat);
    }

    #[test]
    fn dex_router_pool_chat_selection_sets_internal_upstream_model() {
        let chat = router_chat_account("chat", "pool-a", 100, 1, Some("deepseek-v4-pro"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![chat, responses], "chat");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, None).unwrap();
        let effective_model = router_effective_model_for_account(&account, "gpt-5", &endpoint);

        assert_eq!(account.id, "chat");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(effective_model, "deepseek-v4-pro");
    }

    #[test]
    fn dex_router_native_direct_gpt_model_skips_chat_mapping() {
        let mut chat = router_chat_account("deepseek", "pool-a", 100, 1, Some("model-chat"));
        chat.provider = "deepseek".into();
        chat.endpoints[0]
            .model_map
            .insert("gpt-5.5".into(), "deepseek-v4-pro".into());
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![chat, responses], "deepseek");

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5.5", 1_000, 0, None).unwrap();
        assert_eq!(account.id, "responses");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);

        let snapshot = dex_router_status_snapshot(&store, "gpt-5.5", 1_000);
        let chat = snapshot["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["account_id"] == "deepseek")
            .unwrap();
        assert_eq!(chat["reason"], "native_direct_requires_gpt_account");
    }

    #[test]
    fn gpt54_computer_use_preflight_falls_back_to_gpt55() {
        let account = router_responses_account("responses", "pool-a", 10, 1);
        let endpoint = account.endpoints[0].clone();
        let selection = AccountRouteSelection {
            account,
            endpoint,
            route_trace: Some(json!({"requested_model": GPT54_MODEL}).to_string()),
            requires_computer: true,
            strip_native_computer_toolchain: false,
            explicit_model: None,
            explicit_account_model: false,
            session_main_model_anchor: false,
            main_model_anchor_to_record: None,
        };

        let selection = apply_codex_router_preflight_model_fallback(GPT54_MODEL, selection);
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(
            selection.explicit_model.as_deref(),
            Some(GPT54_COMPUTER_FALLBACK_MODEL)
        );
        assert_eq!(
            trace["model_fallback"]["to_model"],
            GPT54_COMPUTER_FALLBACK_MODEL
        );
        assert_eq!(
            trace["model_fallback"]["reason"],
            "computer_use_gpt54_native_helper"
        );
    }

    #[test]
    fn gpt54_plain_preflight_keeps_requested_model_even_after_recent_error() {
        let mut account = router_responses_account("responses", "pool-a", 10, 1);
        account.runtime_state.model_states.insert(
            GPT54_MODEL.into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::Error,
                status_message: "HTTP 503".into(),
                next_retry_after: None,
                quota: Default::default(),
                updated_at: crate::accounts::now_secs(),
            },
        );
        let endpoint = account.endpoints[0].clone();
        let selection = AccountRouteSelection {
            account,
            endpoint,
            route_trace: Some(json!({"requested_model": GPT54_MODEL}).to_string()),
            requires_computer: false,
            strip_native_computer_toolchain: false,
            explicit_model: None,
            explicit_account_model: false,
            session_main_model_anchor: false,
            main_model_anchor_to_record: None,
        };

        let selection = apply_codex_router_preflight_model_fallback(GPT54_MODEL, selection);

        assert!(selection.explicit_model.is_none());
        assert_eq!(
            selection.route_trace.as_deref(),
            Some(json!({"requested_model": GPT54_MODEL}).to_string().as_str())
        );
    }

    #[test]
    fn dex_router_explicit_account_model_selects_exact_account_without_model_map() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let target = router_chat_account("target", "pool-b", 0, 1, Some("mapped-target"));
        let slug = codex_config::encode_dex_account_model_slug("target", "ep-target", "real-model");
        let store = router_store(vec![active, target], "active");

        let selection = resolve_explicit_dex_account_model_selection(&store, &slug, None)
            .unwrap()
            .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "target");
        assert_eq!(selection.endpoint.id, "ep-target");
        assert_eq!(selection.explicit_model.as_deref(), Some("real-model"));
        assert_eq!(trace["explicit_model_selection"], true);
        assert_eq!(trace["upstream_model"], "real-model");
    }

    #[test]
    fn dex_router_explicit_chat_model_uses_native_helper_for_computer_use() {
        let mut active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        active.endpoints[0]
            .model_map
            .insert("gpt-5.5".into(), "deepseek-v4-pro".into());
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active, responses], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "responses");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiResponses);
        assert_eq!(selection.explicit_model.as_deref(), Some("gpt-5.5"));
        assert_eq!(trace["native_helper_reroute"], true);
        assert_eq!(trace["main_account_id"], "active");
        assert_eq!(trace["main_selected_model"], "deepseek-v4-pro");
        assert_eq!(trace["upstream_model"], "gpt-5.5");
    }

    #[test]
    fn dex_router_explicit_chat_model_weak_intent_falls_back_without_native_helper() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: false,
            native_signal: NativeRouteSignal::TextIntent,
            labels: vec!["input.computer_intent".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "active");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(selection.explicit_model.as_deref(), Some("deepseek-v4-pro"));
        assert!(!selection.requires_computer);
        assert_eq!(trace["explicit_model_selection"], true);
        assert!(trace.get("native_helper_fallback_to_chat").is_none());
    }

    #[test]
    fn dex_router_explicit_chat_model_rejects_strong_computer_signal_without_native_helper() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let err = match resolve_explicit_dex_account_model_selection(
            &store,
            &slug,
            Some(&requirements),
        ) {
            Ok(_) => panic!("强 Computer Use 信号缺少原生 helper 时不应回退到 Chat"),
            Err(err) => err,
        };
        assert_eq!(err.status(), StatusCode::CONFLICT);
    }

    #[test]
    fn dex_router_explicit_chat_model_strips_native_toolchain_when_enabled() {
        let mut active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let mut routing = crate::accounts::account_routing_options(&active);
        routing.native_computer_policy = "strip_and_continue".into();
        crate::accounts::set_account_routing_options(&mut active, routing);
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "active");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(selection.explicit_model.as_deref(), Some("deepseek-v4-pro"));
        assert!(!selection.requires_computer);
        assert!(selection.strip_native_computer_toolchain);
        assert_eq!(trace["native_computer_policy"], "strip_and_continue");
        assert_eq!(trace["native_toolchain_stripped"], true);
        assert_eq!(trace["upstream_model"], "deepseek-v4-pro");
    }

    #[test]
    fn dex_router_explicit_chat_model_uses_native_helper_despite_recent_failures() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let mut responses = router_responses_account("responses", "pool-a", 10, 1);
        let now = crate::accounts::now_secs();
        responses
            .runtime_state
            .recent_requests
            .push(crate::accounts::AccountRecentRequestBucket {
                bucket_start: now,
                success: 0,
                failed: 3,
            });
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active, responses], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "responses");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiResponses);
        assert_eq!(selection.explicit_model.as_deref(), Some("gpt-5.5"));
        assert!(selection.requires_computer);
        assert_eq!(trace["native_helper_reroute"], true);
        assert_eq!(trace["main_account_id"], "active");
        assert_eq!(trace["selected_account_id"], "responses");
    }

    #[test]
    fn dex_router_explicit_chat_model_weak_intent_keeps_main_model() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let mut responses = router_responses_account("responses", "pool-a", 10, 1);
        let now = crate::accounts::now_secs();
        responses.runtime_state.model_states.insert(
            DEFAULT_NATIVE_HELPER_MODEL.into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::Error,
                status_message: "HTTP 502: upstream returned response.failed".into(),
                next_retry_after: None,
                quota: Default::default(),
                updated_at: now,
            },
        );
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active, responses], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: false,
            native_signal: NativeRouteSignal::TextIntent,
            labels: vec!["input.computer_intent".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "active");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(selection.explicit_model.as_deref(), Some("deepseek-v4-pro"));
        assert!(!selection.requires_computer);
        assert_eq!(trace["upstream_model"], "deepseek-v4-pro");
        assert!(trace.get("native_helper_reroute").is_none());
    }

    #[test]
    fn dex_router_explicit_chat_model_weak_intent_ignores_failed_helpers() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let mut responses = router_responses_account("responses", "pool-a", 10, 1);
        let now = crate::accounts::now_secs();
        responses.runtime_state.model_states.insert(
            DEFAULT_NATIVE_HELPER_MODEL.into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::Error,
                status_message: "HTTP 503: Service temporarily unavailable".into(),
                next_retry_after: None,
                quota: Default::default(),
                updated_at: now,
            },
        );
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active, responses], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: false,
            native_signal: NativeRouteSignal::TextIntent,
            labels: vec!["input.computer_intent".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "active");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(selection.explicit_model.as_deref(), Some("deepseek-v4-pro"));
        assert!(!selection.requires_computer);
        assert_eq!(trace["upstream_model"], "deepseek-v4-pro");
        assert!(trace.get("native_helper_skipped").is_none());
    }

    #[test]
    fn dex_router_session_main_model_anchor_routes_gpt_helper_name_back_to_real_model() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, responses], "active");
        let anchor = crate::codex_router_session::MainModelAnchor {
            account_id: "active".into(),
            endpoint_id: "ep-active".into(),
            model: "deepseek-v4-pro".into(),
            endpoint_kind: "openai_chat".into(),
        };
        let selection =
            resolve_session_main_model_anchor_selection(&store, "gpt-5.4-mini", &anchor, None)
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "active");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiChat);
        assert_eq!(selection.explicit_model.as_deref(), Some("deepseek-v4-pro"));
        assert!(selection.session_main_model_anchor);
        assert_eq!(trace["session_main_model_anchor"], true);
        assert_eq!(trace["requested_model"], "gpt-5.4-mini");
        assert_eq!(trace["upstream_model"], "deepseek-v4-pro");
    }

    #[test]
    fn dex_router_explicit_chat_model_strong_signal_uses_managed_recent_failed_helper_as_last_resort(
    ) {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let mut responses = router_official_anchor_account("responses", "pool-a", 10, 1);
        enable_official_execution(&mut responses);
        let now = crate::accounts::now_secs();
        responses.runtime_state.model_states.insert(
            DEFAULT_NATIVE_HELPER_MODEL.into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::Error,
                status_message: "HTTP 503: Service temporarily unavailable".into(),
                next_retry_after: None,
                quota: Default::default(),
                updated_at: now,
            },
        );
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active, responses], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "responses");
        assert_eq!(selection.endpoint.kind, EndpointKind::CodexOfficial);
        assert_eq!(selection.explicit_model.as_deref(), Some("gpt-5.5"));
        assert!(selection.requires_computer);
        assert_eq!(trace["native_helper_reroute"], true);
        assert_eq!(trace["native_helper_reason"], "strong_computer_signal");
        assert_eq!(trace["native_helper_skipped"][0]["model"], "gpt-5.5");
        assert_eq!(
            trace["native_helper_skipped"][0]["reason"],
            "recent_transient_upstream_error"
        );
        assert_eq!(trace["native_helper_last_resort"], true);
    }

    #[test]
    fn dex_router_explicit_chat_model_strong_signal_does_not_auto_use_gpt54_mini() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let mut responses = router_responses_account("responses", "pool-a", 10, 1);
        let now = crate::accounts::now_secs();
        responses.runtime_state.model_states.insert(
            DEFAULT_NATIVE_HELPER_MODEL.into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::Error,
                status_message: "HTTP 503: Service temporarily unavailable".into(),
                next_retry_after: None,
                quota: Default::default(),
                updated_at: now,
            },
        );
        let slug =
            codex_config::encode_dex_account_model_slug("active", "ep-active", "deepseek-v4-pro");
        let store = router_store(vec![active, responses], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();
        let trace: Value = serde_json::from_str(selection.route_trace.as_deref().unwrap()).unwrap();

        assert_eq!(selection.account.id, "responses");
        assert_eq!(selection.endpoint.kind, EndpointKind::OpenAiResponses);
        assert_eq!(selection.explicit_model.as_deref(), Some("gpt-5.5"));
        assert!(selection.requires_computer);
        assert_eq!(trace["native_helper_reroute"], true);
        assert_eq!(trace["native_helper_reason"], "strong_computer_signal");
        assert_eq!(trace["native_helper_skipped"].as_array().unwrap().len(), 0);
        assert_eq!(trace["native_helper_last_resort"], false);
    }

    #[test]
    fn dex_router_explicit_chat_model_allows_available_computer_tool_for_plain_text() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("mapped-active"));
        let slug = codex_config::encode_dex_account_model_slug("active", "ep-active", "real-model");
        let store = router_store(vec![active], "active");
        let requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: false,
            ..Default::default()
        };

        let selection =
            resolve_explicit_dex_account_model_selection(&store, &slug, Some(&requirements))
                .unwrap()
                .unwrap();

        assert_eq!(selection.account.id, "active");
        assert_eq!(selection.explicit_model.as_deref(), Some("real-model"));
    }

    #[test]
    fn explicit_chat_model_injects_real_model_identity() {
        let mut account = router_chat_account("deepseek", "pool-a", 100, 1, None);
        account.name = "DeepSeek 桌面版".into();
        account.provider = "deepseek".into();
        let endpoint = account.endpoints[0].clone();
        let mut messages = vec![
            ChatMessage {
                role: "system".into(),
                content: Some(Value::String("你是 Codex。".into())),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            ChatMessage {
                role: "user".into(),
                content: Some(Value::String("你是谁？".into())),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        ];

        maybe_inject_explicit_model_identity(
            &mut messages,
            &account,
            &endpoint,
            Some("deepseek-v4-pro"),
        );

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1].role, "system");
        let identity = messages[1]
            .content
            .as_ref()
            .and_then(Value::as_str)
            .unwrap();
        assert!(identity.contains("真实上游模型是「deepseek-v4-pro」"));
        assert!(identity.contains("供应商：deepseek"));
        assert!(identity.contains("不要自称 Codex、GPT-5 或 OpenAI 官方模型"));
    }

    #[test]
    fn explicit_model_identity_skips_non_explicit_or_responses_endpoint() {
        let chat = router_chat_account("chat", "pool-a", 100, 1, None);
        let responses = router_responses_account("responses", "pool-a", 100, 1);
        let base_message = ChatMessage {
            role: "user".into(),
            content: Some(Value::String("hi".into())),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };
        let mut non_explicit_messages = vec![base_message.clone()];
        let mut responses_messages = vec![base_message];

        maybe_inject_explicit_model_identity(
            &mut non_explicit_messages,
            &chat,
            &chat.endpoints[0],
            None,
        );
        maybe_inject_explicit_model_identity(
            &mut responses_messages,
            &responses,
            &responses.endpoints[0],
            Some("gpt-5.5"),
        );

        assert_eq!(non_explicit_messages.len(), 1);
        assert_eq!(responses_messages.len(), 1);
    }

    #[test]
    fn dex_router_status_reports_skip_reasons() {
        let active = router_chat_account("active", "pool-a", 0, 1, Some("model-active"));
        let unmapped = router_chat_account("unmapped", "pool-a", 100, 1, None);
        let other_pool = router_chat_account("other", "pool-b", 100, 1, Some("model-other"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, unmapped, other_pool, responses], "active");

        let snapshot = dex_router_status_snapshot(&store, "gpt-5", 1_000);
        let candidates = snapshot["candidates"].as_array().unwrap();

        assert_eq!(snapshot["anchor"]["pool"], "pool-a");
        assert_eq!(snapshot["selected"]["account_id"], "unmapped");
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate["account_id"] == "active"
                    && candidate["reason"] == "ready")
        );
        assert!(candidates.iter().any(
            |candidate| candidate["account_id"] == "unmapped" && candidate["reason"] == "ready"
        ));
        assert!(candidates
            .iter()
            .any(|candidate| candidate["account_id"] == "other"
                && candidate["reason"] == "pool_mismatch"));
    }

    #[test]
    fn dex_router_trace_records_selection_context() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let high = router_responses_account("high", "pool-a", 10, 2);
        let other_pool = router_responses_account("other", "pool-b", 100, 1);
        let store = router_store(vec![active, high, other_pool], "active");

        let (selection, trace) = dex_router_trace_for_selection(&store, "gpt-5", 1_000, 7, None);
        let (account, endpoint) = selection.unwrap();
        let value: Value = serde_json::from_str(&trace).unwrap();

        assert_eq!(account.id, "high");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        assert_eq!(value["trace_version"], 1);
        assert_eq!(value["route_surface"], "codex_router");
        assert_eq!(value["cursor"], 7);
        assert_eq!(value["anchor"]["account_id"], "active");
        assert_eq!(value["selected"]["account_id"], "high");
        assert_eq!(value["selected"]["mapped_model"], "gpt-5");
        assert_eq!(value["selected"]["effective_model"], "gpt-5");
        assert_eq!(
            value["selected"]["capabilities"]["protocol"],
            "responses_direct"
        );
        assert_eq!(value["selected"]["capabilities"]["tool_mode"], "native");
        assert_eq!(value["selected"]["capabilities"]["image_generation"], true);
        assert_eq!(value["candidate_count"], 3);
        assert_eq!(value["eligible_count"], 2);
        assert!(value["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["account_id"] == "other"
                && candidate["reason"] == "pool_mismatch"));
    }

    #[test]
    fn dex_router_status_reports_runtime_skip_details() {
        let active = router_responses_account("active", "pool-a", 0, 1);
        let mut account_cooled = router_official_anchor_account("account-cooled", "pool-a", 100, 1);
        enable_official_execution(&mut account_cooled);
        account_cooled.runtime_state.status = AccountRuntimeStatus::QuotaExceeded;
        account_cooled.runtime_state.status_message = "账号额度耗尽".into();
        account_cooled.runtime_state.next_retry_after = Some(2_000);

        let mut model_cooled = router_official_anchor_account("model-cooled", "pool-a", 90, 1);
        enable_official_execution(&mut model_cooled);
        model_cooled.runtime_state.model_states.insert(
            "gpt-5".into(),
            crate::accounts::AccountModelRuntimeState {
                status: AccountRuntimeStatus::QuotaExceeded,
                status_message: "模型额度耗尽".into(),
                next_retry_after: Some(1_800),
                quota: Default::default(),
                updated_at: 1_000,
            },
        );
        let store = router_store(vec![active, account_cooled, model_cooled], "active");

        let snapshot = dex_router_status_snapshot(&store, "gpt-5", 1_000);
        let candidates = snapshot["candidates"].as_array().unwrap();

        let account = candidates
            .iter()
            .find(|candidate| candidate["account_id"] == "account-cooled")
            .unwrap();
        assert_eq!(account["reason"], "account_quota_cooling");
        assert_eq!(account["runtime_message"], "账号额度耗尽");
        assert_eq!(account["runtime_next_retry_after"], 2_000);

        let model = candidates
            .iter()
            .find(|candidate| candidate["account_id"] == "model-cooled")
            .unwrap();
        assert_eq!(model["reason"], "model_quota_cooling");
        assert_eq!(model["model_runtime_message"], "模型额度耗尽");
        assert_eq!(model["model_runtime_next_retry_after"], 1_800);
    }

    #[test]
    fn dex_router_status_reports_capability_matrix() {
        let active = router_chat_account("active", "pool-a", 0, 1, Some("model-active"));
        let responses = router_responses_account("responses", "pool-a", 20, 1);
        let mut mimo = router_chat_account("mimo", "pool-a", 5, 1, Some("mimo-v2.5-pro"));
        mimo.provider = "mimo".into();
        mimo.upstream = "https://token-plan-cn.xiaomimimo.com/v1".into();
        mimo.endpoints[0].base_url = "https://token-plan-cn.xiaomimimo.com/v1".into();
        let mimo_capabilities =
            dex_router_capability_summary(&mimo, &mimo.endpoints[0], "mimo-v2.5-pro");
        let mut custom = router_responses_account("custom", "pool-a", 15, 1);
        custom.provider = "custom".into();
        custom.endpoints[0].kind = EndpointKind::CustomResponses;
        custom.endpoints[0].image_generation_enabled = None;
        let mut custom_enabled = router_responses_account("custom-enabled", "pool-a", 10, 1);
        custom_enabled.provider = "custom".into();
        custom_enabled.endpoints[0].kind = EndpointKind::CustomResponses;
        custom_enabled.endpoints[0].image_generation_enabled = Some(true);
        let store = router_store(vec![active, responses, custom, custom_enabled], "active");

        let snapshot = dex_router_status_snapshot(&store, "gpt-5", 1_000);
        let candidates = snapshot["candidates"].as_array().unwrap();
        let responses = candidates
            .iter()
            .find(|candidate| candidate["account_id"] == "responses")
            .unwrap();
        let custom = candidates
            .iter()
            .find(|candidate| candidate["account_id"] == "custom")
            .unwrap();
        let custom_enabled = candidates
            .iter()
            .find(|candidate| candidate["account_id"] == "custom-enabled")
            .unwrap();
        assert_eq!(responses["capabilities"]["protocol"], "responses_direct");
        assert_eq!(responses["capabilities"]["tool_mode"], "native");
        assert_eq!(responses["capabilities"]["image_generation"], true);
        assert_eq!(responses["capabilities"]["vision"], "off");
        assert_eq!(mimo_capabilities["protocol"], "chat_translate");
        assert_eq!(mimo_capabilities["web"], true);
        assert_eq!(mimo_capabilities["web_mode"], "tool");
        assert_eq!(custom["capabilities"]["image_generation"], false);
        assert_eq!(custom_enabled["capabilities"]["image_generation"], true);
    }

    #[test]
    fn dex_router_tool_decisions_name_provider_web_adapter() {
        let requirements =
            router_tool_requirements_for_tools(&[json!({"type": "web_search_preview"})], None);

        let mimo = dex_router_tool_decisions(
            &json!({
                "protocol": "chat_translate",
                "tool_mode": "translated",
                "tools": true,
                "web": true,
                "web_mode": "tool",
                "image_generation": false
            }),
            Some(&requirements),
        );
        let options_adapter = dex_router_tool_decisions(
            &json!({
                "protocol": "chat_translate",
                "tool_mode": "translated",
                "tools": true,
                "web": true,
                "web_mode": "options",
                "image_generation": false
            }),
            Some(&requirements),
        );

        assert_eq!(mimo["translated"][0], "web_search_tool");
        assert_eq!(options_adapter["translated"][0], "web_search_options");
    }

    #[test]
    fn dex_router_keeps_one_plain_turn_after_computer_use() {
        let sessions = Arc::new(dashmap::DashMap::new());
        let mut native_requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };

        let active = update_codex_router_session_route_state(
            &sessions,
            Some("thread-id:test-thread".into()),
            Some(&mut native_requirements),
            1_000,
        )
        .unwrap();
        assert_eq!(active.state, "native_active");
        assert!(active.force_native_responses);
        assert!(sessions.contains_key("thread-id:test-thread"));

        let mut plain_requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: false,
            ..Default::default()
        };
        let observed = update_codex_router_session_route_state(
            &sessions,
            Some("thread-id:test-thread".into()),
            Some(&mut plain_requirements),
            1_030,
        )
        .unwrap();

        assert_eq!(observed.state, "native_observe");
        assert_eq!(observed.reason, "plain_turn_keep_native_once");
        assert!(observed.force_native_responses);
        assert!(plain_requirements.requires_computer);
        assert!(plain_requirements
            .labels
            .iter()
            .any(|label| label == "session.native_observe"));
        assert!(sessions.contains_key("thread-id:test-thread"));

        let mut next_plain_requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: false,
            ..Default::default()
        };
        let released = update_codex_router_session_route_state(
            &sessions,
            Some("thread-id:test-thread".into()),
            Some(&mut next_plain_requirements),
            1_060,
        )
        .unwrap();

        assert_eq!(released.state, "native_released");
        assert_eq!(released.reason, "plain_turn_release_to_chat");
        assert!(!released.force_native_responses);
        assert!(!next_plain_requirements.requires_computer);
        assert!(!next_plain_requirements
            .labels
            .iter()
            .any(|label| label == "session.native_observe"));
        assert!(!sessions.contains_key("thread-id:test-thread"));
    }

    #[test]
    fn dex_router_refreshes_native_track_on_computer_output() {
        let sessions = Arc::new(dashmap::DashMap::new());
        let mut native_requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::TextIntent,
            labels: vec!["input.computer_intent".into()],
            ..Default::default()
        };
        update_codex_router_session_route_state(
            &sessions,
            Some("thread-id:test-thread".into()),
            Some(&mut native_requirements),
            1_000,
        )
        .unwrap();

        let mut output_requirements = RouterToolRequirements {
            has_computer: true,
            requires_computer: true,
            native_signal: NativeRouteSignal::StrongNative,
            labels: vec!["input.computer_call_output".into()],
            ..Default::default()
        };
        let refreshed = update_codex_router_session_route_state(
            &sessions,
            Some("thread-id:test-thread".into()),
            Some(&mut output_requirements),
            1_010,
        )
        .unwrap();

        assert_eq!(refreshed.state, "native_active");
        assert_eq!(refreshed.reason, "input.computer_call_output");
        assert!(refreshed.force_native_responses);
        assert!(sessions.contains_key("thread-id:test-thread"));
    }

    #[test]
    fn dex_router_weak_continuation_keeps_existing_native_track() {
        let sessions = Arc::new(dashmap::DashMap::new());
        sessions.insert(
            "thread-id:test-thread".into(),
            crate::codex_router_session::RouteState {
                observe_remaining: 0,
                expires_at: 1_600,
                main_model_anchor: None,
            },
        );
        let mut requirements = RouterToolRequirements {
            native_signal: NativeRouteSignal::WeakContinuation,
            ..Default::default()
        };
        requirements.add_label("input.weak_continuation");

        let decision = update_codex_router_session_route_state(
            &sessions,
            Some("thread-id:test-thread".into()),
            Some(&mut requirements),
            1_030,
        )
        .unwrap();

        assert_eq!(decision.state, "native_observe");
        assert_eq!(decision.reason, "weak_continuation_keep_native");
        assert!(decision.force_native_responses);
        assert!(requirements.requires_computer);
        assert!(requirements
            .labels
            .iter()
            .any(|label| label == "session.weak_continuation"));
        assert!(sessions.contains_key("thread-id:test-thread"));
    }

    #[test]
    fn dex_router_failed_native_stream_keeps_native_track() {
        let sessions = Arc::new(dashmap::DashMap::new());
        let route_trace = json!({
            "session_route_force_native_responses": true
        })
        .to_string();

        let refreshed = maybe_refresh_failed_native_track(
            Some(&sessions),
            Some("thread-id:test-thread"),
            &route_trace,
            true,
            1_000,
        );

        assert!(refreshed);
        let state = sessions.get("thread-id:test-thread").unwrap();
        assert_eq!(
            state.observe_remaining,
            crate::codex_router_session::NATIVE_OBSERVE_TURNS
        );
        assert_eq!(
            state.expires_at,
            1_000 + crate::codex_router_session::NATIVE_OBSERVE_TTL_SECS
        );
    }

    #[test]
    fn dex_router_plain_failed_stream_does_not_keep_native_track() {
        let sessions = Arc::new(dashmap::DashMap::new());
        let route_trace = json!({
            "session_route_force_native_responses": false
        })
        .to_string();

        let refreshed = maybe_refresh_failed_native_track(
            Some(&sessions),
            Some("thread-id:test-thread"),
            &route_trace,
            true,
            1_000,
        );

        assert!(!refreshed);
        assert!(!sessions.contains_key("thread-id:test-thread"));
    }

    #[test]
    fn dex_router_response_feedback_refreshes_native_track() {
        let sessions = Arc::new(dashmap::DashMap::new());
        let response = json!({
            "id": "resp_test",
            "output": [{"type": "computer_call", "call_id": "call_1"}]
        });

        let feedback = crate::codex_router_session::maybe_refresh_from_response(
            Some(&sessions),
            Some("thread-id:test-thread"),
            &response,
            1_000,
        )
        .unwrap();

        assert_eq!(feedback.reason, "response.computer_signal");
        let state = sessions.get("thread-id:test-thread").unwrap();
        assert_eq!(
            state.observe_remaining,
            crate::codex_router_session::NATIVE_OBSERVE_TURNS
        );
        assert!(state.expires_at > 1_000);
    }

    #[test]
    fn dex_router_response_feedback_ignores_plain_function_calls() {
        let sessions = Arc::new(dashmap::DashMap::new());
        let response = json!({
            "id": "resp_test",
            "output": [{"type": "function_call", "name": "read_file"}]
        });

        let feedback = crate::codex_router_session::maybe_refresh_from_response(
            Some(&sessions),
            Some("thread-id:test-thread"),
            &response,
            1_000,
        );

        assert!(feedback.is_none());
        assert!(!sessions.contains_key("thread-id:test-thread"));
    }

    #[test]
    fn dex_router_status_scenarios_report_tool_specific_selection() {
        let mut active = router_chat_account("active", "pool-a", 100, 1, Some("model-active"));
        active.provider = "deepseek".into();
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, responses], "active");

        let scenarios = dex_router_status_scenarios(&store, "gpt-5", 1_000);
        let scenarios = scenarios.as_array().unwrap();
        let web = scenarios
            .iter()
            .find(|scenario| scenario["scenario_id"] == "web")
            .unwrap();
        let native = scenarios
            .iter()
            .find(|scenario| scenario["scenario_id"] == "native")
            .unwrap();

        assert_eq!(web["selected"]["account_id"], "responses");
        assert_eq!(web["tool_requirements"]["web_search"], true);
        assert_eq!(native["selected"]["account_id"], "responses");
        assert_eq!(native["tool_requirements"]["image_generation"], true);
        assert_eq!(native["tool_requirements"]["computer"], true);
        let chat = native["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["account_id"] == "active")
            .unwrap();
        assert_eq!(chat["reason"], "capability_mismatch");
    }

    #[test]
    fn dex_router_status_ignores_recent_failures_for_selection() {
        let mut risky = router_responses_account("risky", "pool-a", 100, 1);
        risky
            .runtime_state
            .recent_requests
            .push(crate::accounts::AccountRecentRequestBucket {
                bucket_start: 1_200,
                success: 0,
                failed: 2,
            });
        let healthy = router_responses_account("healthy", "pool-a", 10, 1);
        let store = router_store(vec![risky, healthy], "risky");

        let snapshot = dex_router_status_snapshot(&store, "gpt-5", 1_200);

        assert_eq!(snapshot["selected"]["account_id"], "risky");
        let risky_candidate = snapshot["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["account_id"] == "risky")
            .unwrap();
        assert_eq!(risky_candidate["reason"], "ready");
    }

    #[test]
    fn router_status_tools_from_query_maps_known_slugs() {
        let tools =
            router_status_tools_from_query(Some("web,image,computer,mcp,file,function,unknown"));
        let requirements = router_tool_requirements_for_diagnostic_tools(&tools);

        assert_eq!(tools.len(), 6);
        assert!(requirements.has_web_search);
        assert!(requirements.has_image_generation);
        assert!(requirements.has_computer);
        assert!(requirements.has_mcp);
        assert!(requirements.has_file_search);
        assert!(requirements.has_function_tools);
        assert!(requirements.requires_function_tools);
        assert!(requirements.requires_web_search);
        assert!(requirements.requires_file_search);
        assert!(requirements.requires_mcp);
        assert!(requirements.requires_image_generation);
        assert!(requirements.requires_computer);
    }

    #[test]
    fn dex_router_tool_requirements_detects_codex_tool_families() {
        let req = responses_request_with_tools(vec![
            json!({"type": "function", "name": "read_file"}),
            json!({"type": "web_search_preview"}),
            json!({"type": "file_search"}),
            json!({"type": "remote_mcp", "server_label": "github"}),
            json!({"type": "computer_use_preview"}),
            json!({"type": "image_generation"}),
            json!({"type": "future_tool", "name": "mystery"}),
        ]);

        let requirements = router_tool_requirements(&req);

        assert_eq!(requirements.tool_count, 7);
        assert!(requirements.has_function_tools);
        assert!(requirements.has_web_search);
        assert!(requirements.has_file_search);
        assert!(requirements.has_mcp);
        assert!(requirements.has_computer);
        assert!(requirements.has_image_generation);
        assert!(!requirements.requires_function_tools);
        assert!(requirements.requires_web_search);
        assert!(!requirements.requires_file_search);
        assert!(!requirements.requires_mcp);
        assert!(!requirements.requires_computer);
        assert!(!requirements.requires_image_generation);
        assert!(requirements.has_unknown_tools);
        assert!(requirements.labels.iter().any(|label| label == "github"));
    }

    #[test]
    fn dex_router_tool_requirements_detects_computer_signal_in_input() {
        let mut req = responses_request_with_tools(vec![json!({"type": "image_generation"})]);
        req.input = ResponsesInput::Messages(vec![json!({
            "type": "computer_call_output",
            "call_id": "call_screen",
            "screenshot": "data:image/png;base64,abc"
        })]);

        let requirements = router_tool_requirements(&req);

        assert!(requirements.has_computer);
        assert!(requirements.requires_computer);
        assert!(requirements.has_image_generation);
        assert!(!requirements.requires_image_generation);
        assert!(requirements
            .labels
            .iter()
            .any(|label| label == "input.computer_call_output"));
    }

    #[test]
    fn dex_router_tool_requirements_ignores_system_computer_text() {
        let mut req = responses_request_with_tools(vec![json!({"type": "computer_use_preview"})]);
        req.input = ResponsesInput::Messages(vec![
            json!({
                "type": "message",
                "role": "developer",
                "content": "Computer Use tools are available when the user asks for desktop control."
            }),
            json!({
                "type": "message",
                "role": "system",
                "content": "Do not use Computer Use unless needed."
            }),
            json!({
                "type": "message",
                "role": "user",
                "content": "普通文本测试 DeepSeek"
            }),
        ]);

        let requirements = router_tool_requirements(&req);

        assert!(requirements.has_computer);
        assert!(!requirements.requires_computer);
        assert!(!requirements
            .labels
            .iter()
            .any(|label| label == "input.computer_intent"));
    }

    #[test]
    fn dex_router_tool_requirements_ignores_historical_computer_intent() {
        let mut req = responses_request_with_tools(vec![json!({"type": "computer_use_preview"})]);
        req.input = ResponsesInput::Messages(vec![
            json!({
                "type": "message",
                "role": "user",
                "content": "使用 Computer Use 打开抖音 app，播放第一个视频"
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": "已经完成。"
            }),
            json!({
                "type": "message",
                "role": "user",
                "content": "刚才为什么失败？"
            }),
        ]);

        let requirements = router_tool_requirements(&req);

        assert!(requirements.has_computer);
        assert!(!requirements.requires_computer);
        assert!(!requirements
            .labels
            .iter()
            .any(|label| label == "input.computer_intent"));
    }

    #[test]
    fn dex_router_tool_requirements_keeps_latest_computer_output_native() {
        let mut req = responses_request_with_tools(vec![json!({"type": "computer_use_preview"})]);
        req.input = ResponsesInput::Messages(vec![
            json!({
                "type": "message",
                "role": "user",
                "content": "普通文本"
            }),
            json!({
                "type": "computer_call_output",
                "call_id": "call_screen",
                "screenshot": "data:image/png;base64,abc"
            }),
        ]);

        let requirements = router_tool_requirements(&req);

        assert!(requirements.has_computer);
        assert!(requirements.requires_computer);
        assert!(requirements
            .labels
            .iter()
            .any(|label| label == "input.computer_call_output"));
    }

    #[test]
    fn dex_router_tool_requirements_ignores_plain_input_image() {
        let mut req = responses_request_with_tools(vec![]);
        req.input = ResponsesInput::Messages(vec![json!({
            "type": "message",
            "role": "user",
            "content": [
                {
                    "type": "input_image",
                    "image_url": "data:image/png;base64,abc"
                }
            ]
        })]);

        let requirements = router_tool_requirements(&req);

        assert!(!requirements.has_computer);
        assert!(!requirements.requires_computer);
        assert!(!requirements
            .labels
            .iter()
            .any(|label| label == "input.screenshot"));
    }

    #[test]
    fn strip_native_computer_toolchain_preserves_regular_tools_and_text() {
        let mut req = responses_request_with_tools(vec![
            json!({"type": "computer_use_preview"}),
            json!({"type": "function", "name": "get_app_state"}),
            json!({"type": "function", "name": "read_file"}),
            json!({"type": "web_search_preview"}),
        ]);
        req.tool_choice = Some(json!({"type": "function", "function": {"name": "get_app_state"}}));
        req.input = ResponsesInput::Messages(vec![
            json!({
                "type": "message",
                "role": "user",
                "content": "保留这条普通文本"
            }),
            json!({
                "type": "computer_call_output",
                "call_id": "call_screen",
                "screenshot": "data:image/png;base64,abc"
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "{\"screenshot\":\"raw\"}"}]
            }),
            json!({
                "type": "message",
                "role": "assistant",
                "output": {
                    "type": "computer_call_output",
                    "call_id": "nested",
                    "screenshot": "data:image/png;base64,def"
                }
            }),
        ]);

        assert!(strip_native_computer_toolchain_from_request(&mut req));

        assert_eq!(req.tools.len(), 2);
        assert!(req.tools.iter().any(|tool| tool["type"] == "function"));
        assert!(req
            .tools
            .iter()
            .any(|tool| tool["type"] == "web_search_preview"));
        assert!(req.tool_choice.is_none());
        let ResponsesInput::Messages(items) = &req.input else {
            panic!("input should stay as message array");
        };
        assert_eq!(items.len(), 3);
        assert_eq!(items[0]["content"], "保留这条普通文本");
        assert_eq!(
            items[1]["content"][0]["text"],
            "[Computer Use 原生工具链残留已按账号设置剥离]"
        );
        assert_eq!(items[2]["output"], Value::Null);
    }

    #[test]
    fn dex_router_tool_requirements_detects_computer_intent_in_text() {
        let mut req = responses_request_with_tools(vec![
            json!({"type": "function", "name": "exec_command"}),
            json!({"type": "image_generation"}),
        ]);
        req.input = ResponsesInput::Text("使用 Computer Use 打开抖音 app，播放第一个视频".into());

        let requirements = router_tool_requirements(&req);

        assert!(requirements.has_computer);
        assert!(!requirements.requires_computer);
        assert!(requirements.has_function_tools);
        assert!(requirements.has_image_generation);
        assert!(requirements
            .labels
            .iter()
            .any(|label| label == "input.computer_intent"));
    }

    #[test]
    fn dex_router_computer_intent_does_not_synthesize_native_tool() {
        let mut req = responses_request_with_tools(vec![
            json!({"type": "function", "name": "mcp__computer_use"}),
            json!({"type": "image_generation"}),
        ]);
        let original_tools = req.tools.clone();
        req.input = ResponsesInput::Text("使用 Computer Use 打开抖音 app，播放第一个视频".into());

        let requirements = router_tool_requirements(&req);

        assert!(!requirements.requires_computer);
        assert_eq!(requirements.native_signal, NativeRouteSignal::TextIntent);
        assert_eq!(req.tools, original_tools);
        assert!(!req.tools.iter().any(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .is_some_and(|typ| typ == "computer_use_preview")
        }));
    }

    #[test]
    fn dex_router_uses_responses_candidate_when_tools_are_only_available() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("model-active"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, responses], "active");
        let req = responses_request_with_tools(vec![
            json!({"type": "function", "name": "read_file"}),
            json!({"type": "web_search_preview"}),
            json!({"type": "file_search"}),
            json!({"type": "remote_mcp", "server_label": "github"}),
            json!({"type": "computer_use_preview"}),
            json!({"type": "image_generation"}),
        ]);
        let requirements = router_tool_requirements(&req);

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, Some(&requirements))
                .unwrap();

        assert_eq!(account.id, "responses");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        assert!(requirements.has_computer);
        assert!(!requirements.requires_computer);
    }

    #[test]
    fn dex_router_diagnostic_tools_still_report_capability_gaps() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("model-active"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, responses], "active");
        let tools = vec![
            json!({"type": "image_generation"}),
            json!({"type": "computer_use_preview"}),
        ];

        let snapshot = dex_router_status_snapshot_for_tools(&store, "gpt-5", 1_000, &tools);
        let chat = snapshot["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["account_id"] == "active")
            .unwrap();

        assert_eq!(snapshot["selected"]["account_id"], "responses");
        assert_eq!(
            snapshot["tool_requirements"]["requires_image_generation"],
            true
        );
        assert_eq!(snapshot["tool_requirements"]["requires_computer"], true);
        assert_eq!(chat["reason"], "capability_mismatch");
    }

    #[test]
    fn dex_router_skips_candidate_without_explicit_image_tool_choice() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("model-active"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, responses], "active");
        let req = responses_request_with_tool_choice(
            vec![json!({"type": "image_generation"})],
            json!({"type": "image_generation"}),
        );
        let requirements = router_tool_requirements(&req);

        let (account, endpoint) =
            select_dex_router_account_endpoint(&store, "gpt-5", 1_000, 0, Some(&requirements))
                .unwrap();

        assert_eq!(account.id, "responses");
        assert_eq!(endpoint.kind, EndpointKind::OpenAiResponses);
        assert!(requirements.requires_image_generation);
    }

    #[test]
    fn dex_router_trace_records_capability_gaps_and_tool_decisions() {
        let active = router_chat_account("active", "pool-a", 100, 1, Some("model-active"));
        let responses = router_responses_account("responses", "pool-a", 10, 1);
        let store = router_store(vec![active, responses], "active");
        let req = responses_request_with_tool_choice(
            vec![json!({"type": "image_generation"})],
            json!({"type": "image_generation"}),
        );
        let requirements = router_tool_requirements(&req);

        let (selection, trace) =
            dex_router_trace_for_selection(&store, "gpt-5", 1_000, 0, Some(&requirements));
        let value: Value = serde_json::from_str(&trace).unwrap();

        assert_eq!(selection.unwrap().0.id, "responses");
        assert_eq!(value["tool_requirements"]["image_generation"], true);
        assert_eq!(
            value["tool_requirements"]["requires_image_generation"],
            true
        );
        assert_eq!(
            value["selected"]["tool_decisions"]["kept"][0],
            "image_generation"
        );
        assert_eq!(value["tool_decisions"]["kept"][0], "image_generation");
        let chat = value["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["account_id"] == "active")
            .unwrap();
        assert_eq!(chat["reason"], "capability_mismatch");
    }

    #[test]
    fn disabled_image_generation_strips_optional_tool_before_direct_forward() {
        let mut account = router_responses_account("custom", "pool-a", 10, 1);
        account.provider = "custom".into();
        account.endpoints[0].kind = EndpointKind::CustomResponses;
        account.endpoints[0].image_generation_enabled = Some(false);
        let endpoint = account.endpoints[0].clone();
        let mut req = responses_request_with_tools(vec![
            json!({"type": "image_generation"}),
            json!({"type": "web_search_preview"}),
        ]);
        let mut body = axum::body::Bytes::from(serde_json::to_vec(&req).unwrap());

        apply_endpoint_image_generation_declaration(&account, &endpoint, &mut req, &mut body)
            .unwrap();

        assert!(!request_has_image_generation_tool(&req));
        let value: Value = serde_json::from_slice(&body).unwrap();
        let tools = value["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "web_search_preview");
    }

    #[test]
    fn disabled_image_generation_rejects_explicit_tool_choice() {
        let mut account = router_responses_account("custom", "pool-a", 10, 1);
        account.provider = "custom".into();
        account.endpoints[0].kind = EndpointKind::CustomResponses;
        account.endpoints[0].image_generation_enabled = Some(false);
        let endpoint = account.endpoints[0].clone();
        let mut req = responses_request_with_tool_choice(
            vec![json!({"type": "image_generation"})],
            json!({"type": "image_generation"}),
        );
        let mut body = axum::body::Bytes::from(serde_json::to_vec(&req).unwrap());

        let err =
            apply_endpoint_image_generation_declaration(&account, &endpoint, &mut req, &mut body)
                .unwrap_err();

        assert_eq!(err.status(), StatusCode::CONFLICT);
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
    fn codex_official_selector_skips_quota_exceeded_account() {
        let mut quota = official_codex_account("quota", 100, 1);
        quota.runtime_state.status = AccountRuntimeStatus::QuotaExceeded;
        quota.runtime_state.next_retry_after = Some(2_000);
        let ready = official_codex_account("ready", 10, 1);
        let store = official_store(vec![quota, ready], "quota");

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
        active.runtime_state.status = AccountRuntimeStatus::QuotaExceeded;
        active.runtime_state.next_retry_after = Some(2_000);
        let mut other = official_codex_account("other", 0, 1);
        other.runtime_state.model_states.insert(
            "gpt-5".into(),
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
            known_models: Default::default(),
            model_profiles: Default::default(),
            vision: Default::default(),
            image_generation_enabled: Some(true),
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
            known_models: Default::default(),
            model_profiles: Default::default(),
            vision: Default::default(),
            image_generation_enabled: Some(true),
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
            known_models: Default::default(),
            model_profiles: Default::default(),
            vision: Default::default(),
            image_generation_enabled: None,
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
    fn test_sse_usage_observation_marks_response_failed_event() {
        let mut observation = SseUsageObservation::default();
        observation.ingest(&Bytes::from_static(
            br#"event: response.created
data: {"type":"response.created","response":{"status":"in_progress","error":null}}

"#,
        ));
        observation.ingest(&Bytes::from_static(
            br#"event: response.failed
data: {"type":"response.failed","response":{"status":"failed","error":{"code":"upstream_error","message":"Upstream request failed"}}}

data: [DONE]
"#,
        ));
        observation.finish();

        assert!(observation.saw_error_event);
        assert_eq!(
            observation.event_error_message.as_deref(),
            Some("Upstream request failed")
        );
        assert_eq!(
            sse_error_message_from_value(&json!({
                "type": "response.created",
                "response": {
                    "status": "in_progress",
                    "error": null
                }
            })),
            None
        );
        assert_eq!(
            sse_error_message_from_value(&json!({
                "type": "response.failed",
                "response": {
                    "status": "failed",
                    "error": {"message": "Service temporarily unavailable"}
                }
            }))
            .as_deref(),
            Some("Service temporarily unavailable")
        );
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
