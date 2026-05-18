pub mod dex;
pub mod dex_plugins;
pub mod dex_registry;
pub mod dex_security;
pub mod logs;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use deecodex::accounts::{
    AccountClientKind, AccountStore, DevPipelineToolMode, DevPipelineTriggerMode,
};
use deecodex::config::Args;
use deecodex::handlers;
use deecodex::{files, metrics, vector_stores};

use deecodex_plugin_host::PluginManager;

use crate::ServerManager;

fn client_kind_slug(kind: &AccountClientKind) -> &'static str {
    match kind {
        AccountClientKind::Codex => "codex",
        AccountClientKind::ClaudeCode => "claude_code",
        AccountClientKind::Openclaw => "openclaw",
        AccountClientKind::Hermes => "hermes",
        AccountClientKind::GenericClient => "generic_client",
    }
}

fn client_account_counts(store: &AccountStore) -> Value {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for account in &store.accounts {
        *counts
            .entry(client_kind_slug(&account.client_kind))
            .or_default() += 1;
    }
    json!(counts)
}

fn append_account_event(
    data_dir: &Path,
    account_id: &str,
    client_kind: &AccountClientKind,
    action: &str,
    ok: bool,
    message: &str,
    details: Value,
) {
    let path = data_dir.join("account-events.jsonl");
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!("创建账号事件目录失败: {err}");
            return;
        }
    }
    let event = json!({
        "ts": deecodex::accounts::now_secs(),
        "account_id": account_id,
        "client_kind": client_kind,
        "action": action,
        "ok": ok,
        "message": message,
        "details": details,
    });
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(err) = writeln!(file, "{event}") {
                tracing::warn!("写入账号事件日志失败: {err}");
            }
        }
        Err(err) => tracing::warn!("打开账号事件日志失败: {err}"),
    }
}

fn read_account_events(data_dir: &Path, account_id: Option<&str>, limit: usize) -> Vec<Value> {
    let path = data_dir.join("account-events.jsonl");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let mut events: Vec<Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| {
            account_id.is_none_or(|id| event.get("account_id").and_then(Value::as_str) == Some(id))
        })
        .collect();
    events.sort_by_key(|event| event.get("ts").and_then(Value::as_u64).unwrap_or(0));
    events.reverse();
    events.truncate(limit.clamp(1, 100));
    events
}

fn parse_account_json(raw: &str) -> Result<deecodex::accounts::Account, String> {
    let mut value: Value =
        serde_json::from_str(raw).map_err(|e| format!("解析账号 JSON 失败: {e}"))?;
    if let Value::Object(ref mut object) = value {
        if object.contains_key("client_kind") {
            object.remove("target");
        }
    }
    serde_json::from_value(value).map_err(|e| format!("解析账号 JSON 失败: {e}"))
}

// ── 前端返回类型 ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ServiceInfo {
    pub running: bool,
    pub port: u16,
    pub uptime_secs: Option<u64>,
    pub version: String,
    pub cdp_port: u16,
    pub codex_launch_with_cdp: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GuiConfig {
    pub port: u16,
    pub upstream: String,
    pub api_key: String,
    pub model_map: String,
    pub chinese_thinking: bool,
    pub codex_auto_inject: bool,
    pub codex_persistent_inject: bool,
    pub vision_upstream: String,
    pub vision_api_key: String,
    pub vision_model: String,
    pub vision_endpoint: String,
    pub token_anomaly_prompt_max: u32,
    pub token_anomaly_spike_ratio: f64,
    pub token_anomaly_burn_window: u64,
    pub token_anomaly_burn_rate: u32,
    pub allowed_mcp_servers: String,
    pub allowed_computer_displays: String,
    pub computer_executor: String,
    pub computer_executor_timeout_secs: u64,
    pub mcp_executor_config: String,
    pub mcp_executor_timeout_secs: u64,
    pub max_body_mb: u32,
    pub prompts_dir: String,
    pub playwright_state_dir: String,
    pub browser_use_bridge_url: String,
    pub browser_use_bridge_command: String,
    pub data_dir: String,
    pub codex_launch_with_cdp: bool,
    pub cdp_port: u16,
}

impl From<Args> for GuiConfig {
    fn from(a: Args) -> Self {
        Self {
            port: a.port,
            upstream: a.upstream,
            api_key: a.api_key,
            model_map: a.model_map,
            chinese_thinking: a.chinese_thinking,
            codex_auto_inject: a.codex_auto_inject,
            codex_persistent_inject: a.codex_persistent_inject,
            vision_upstream: a.vision_upstream,
            vision_api_key: a.vision_api_key,
            vision_model: a.vision_model,
            vision_endpoint: a.vision_endpoint,
            token_anomaly_prompt_max: a.token_anomaly_prompt_max,
            token_anomaly_spike_ratio: a.token_anomaly_spike_ratio,
            token_anomaly_burn_window: a.token_anomaly_burn_window,
            token_anomaly_burn_rate: a.token_anomaly_burn_rate,
            allowed_mcp_servers: a.allowed_mcp_servers,
            allowed_computer_displays: a.allowed_computer_displays,
            computer_executor: a.computer_executor,
            computer_executor_timeout_secs: a.computer_executor_timeout_secs,
            mcp_executor_config: a.mcp_executor_config,
            mcp_executor_timeout_secs: a.mcp_executor_timeout_secs,
            max_body_mb: a.max_body_mb as u32,
            prompts_dir: a.prompts_dir.to_string_lossy().to_string(),
            playwright_state_dir: a.playwright_state_dir,
            browser_use_bridge_url: a.browser_use_bridge_url,
            browser_use_bridge_command: a.browser_use_bridge_command,
            data_dir: a.data_dir.to_string_lossy().to_string(),
            codex_launch_with_cdp: a.codex_launch_with_cdp,
            cdp_port: a.cdp_port,
        }
    }
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_data_dir(data_dir: impl Into<std::path::PathBuf>) -> std::path::PathBuf {
    let data_dir = data_dir.into();
    if data_dir.is_absolute() {
        return data_dir;
    }
    if let Some(home) = deecodex::config::home_dir() {
        home.join(data_dir)
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(data_dir)
    } else {
        data_dir
    }
}

struct AccountBackedConfig {
    upstream: String,
    api_key: String,
    model_map: String,
    vision_upstream: String,
    vision_api_key: String,
    vision_model: String,
    vision_endpoint: String,
}

fn account_backed_config(existing: Option<&Args>) -> AccountBackedConfig {
    AccountBackedConfig {
        upstream: existing.map(|a| a.upstream.clone()).unwrap_or_default(),
        api_key: existing.map(|a| a.api_key.clone()).unwrap_or_default(),
        model_map: existing.map(|a| a.model_map.clone()).unwrap_or_default(),
        vision_upstream: existing
            .map(|a| a.vision_upstream.clone())
            .unwrap_or_default(),
        vision_api_key: existing
            .map(|a| a.vision_api_key.clone())
            .unwrap_or_default(),
        vision_model: existing.map(|a| a.vision_model.clone()).unwrap_or_default(),
        vision_endpoint: existing
            .map(|a| a.vision_endpoint.clone())
            .unwrap_or_default(),
    }
}

pub(crate) fn load_args() -> Args {
    // 从环境变量 + 默认值构建 Args
    let args = match Args::try_parse_from(["deecodex-gui"]) {
        Ok(a) => a,
        Err(_) => {
            return Args::try_parse_from(["deecodex-gui"]).unwrap_or_else(|_| {
                // clap 失败时返回纯默认值
                Args {
                    command: None,
                    config: None,
                    port: 4446,
                    upstream: "https://openrouter.ai/api/v1".into(),
                    api_key: String::new(),
                    model_map: "{}".into(),
                    max_body_mb: 100,
                    vision_upstream: String::new(),
                    vision_api_key: String::new(),
                    vision_model: "MiniMax-M1".into(),
                    vision_endpoint: "v1/coding_plan/vlm".into(),
                    chinese_thinking: false,
                    codex_auto_inject: true,
                    codex_persistent_inject: false,
                    prompts_dir: "prompts".into(),
                    data_dir: deecodex::config::home_dir()
                        .map(|h| h.join(".deecodex"))
                        .unwrap_or_else(|| std::path::PathBuf::from(".deecodex")),
                    token_anomaly_prompt_max: 200000,
                    token_anomaly_spike_ratio: 5.0,
                    token_anomaly_burn_window: 120,
                    token_anomaly_burn_rate: 500000,
                    allowed_mcp_servers: String::new(),
                    allowed_computer_displays: String::new(),
                    computer_executor: "disabled".into(),
                    computer_executor_timeout_secs: 30,
                    mcp_executor_config: String::new(),
                    mcp_executor_timeout_secs: 30,
                    playwright_state_dir: String::new(),
                    browser_use_bridge_url: String::new(),
                    browser_use_bridge_command: String::new(),
                    daemon: false,
                    codex_launch_with_cdp: false,
                    cdp_port: 9222,
                }
            });
        }
    };
    let mut args = args;
    // 先确保 data_dir 为绝对路径，再合并配置文件；否则 dev 模式会去
    // deecodex-gui/.deecodex 读配置，导致 GUI 保存到 HOME 后又读回默认值。
    if args.data_dir.is_relative() {
        args.data_dir = normalize_data_dir(args.data_dir);
    }
    let mut args = args.merge_with_file();
    // 文件里的旧 data_dir 也可能仍是相对路径，合并后再规整一次。
    if args.data_dir.is_relative() {
        args.data_dir = normalize_data_dir(args.data_dir);
    }
    args
}

/// 执行首次启动迁移：如果 accounts.json 不存在，从旧配置和 Codex config 迁移账号。
/// 返回迁移后的 AccountStore（已持久化）。
fn migrate_or_load_accounts(data_dir: &std::path::Path) -> AccountStore {
    use deecodex::accounts::{
        generate_id, get_provider_presets, guess_provider, now_secs, Account, AccountStore,
    };

    let path = deecodex::accounts::accounts_file_path(data_dir);

    // 已有账号文件，直接加载
    if path.exists() {
        tracing::info!("加载已有账号文件: {}", path.display());
        let mut store = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| deecodex::accounts::parse_account_store(&content).ok())
        {
            Some(store) => store,
            None => return deecodex::accounts::load_accounts(data_dir),
        };
        store.normalize_v2();
        if let Err(e) = deecodex::accounts::save_accounts(data_dir, &store) {
            tracing::warn!("保存规范化后的账号文件失败: {e}");
        }
        return store;
    }

    tracing::info!("accounts.json 不存在，执行首次迁移");

    let mut accounts: Vec<Account> = Vec::new();

    // a. 检查 config.json 是否有自定义上游/Key
    let config_path = Args::default_config_path(data_dir);
    if let Some(file_args) = Args::load_from_file(&config_path) {
        // 上游非默认 OpenRouter 或 Key 不为空 → 迁移旧配置
        let has_custom_upstream = file_args.upstream != "https://openrouter.ai/api/v1";
        let has_api_key = !file_args.api_key.is_empty();
        if has_custom_upstream || has_api_key {
            let model_map: HashMap<String, String> =
                if file_args.model_map.is_empty() || file_args.model_map == "{}" {
                    HashMap::new()
                } else {
                    serde_json::from_str(&file_args.model_map).unwrap_or_default()
                };

            let provider = if has_custom_upstream {
                guess_provider(&file_args.upstream)
            } else {
                "openrouter"
            };

            let migrated = Account {
                id: generate_id(),
                name: "旧配置导入".into(),
                provider: provider.to_string(),
                client_kind: Default::default(),
                wire_protocol: Default::default(),
                upstream: file_args.upstream.clone(),
                api_key: file_args.api_key.clone(),
                default_model: String::new(),
                client_options: HashMap::new(),
                last_applied_at: None,
                last_check: None,
                model_map,
                vision_upstream: file_args.vision_upstream.clone(),
                vision_api_key: file_args.vision_api_key.clone(),
                vision_model: file_args.vision_model.clone(),
                vision_endpoint: file_args.vision_endpoint.clone(),
                vision_enabled: false,
                from_codex_config: false,
                balance_url: String::new(),
                created_at: now_secs(),
                updated_at: now_secs(),
                context_window_override: None,
                reasoning_effort_override: None,
                thinking_tokens: None,
                custom_headers: HashMap::new(),
                provider_options: deecodex::providers::provider_options_for_slug(provider),
                request_timeout_secs: None,
                max_retries: None,
                translate_enabled: true,
                capability_enabled: false,
                capability_account_id: None,
                dev_pipeline_enabled: false,
                dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
                dev_pipeline_command: "/dev-pipeline".into(),
                dev_pipeline_architect_account_id: None,
                dev_pipeline_implementer_account_id: None,
                dev_pipeline_reviewer_account_id: None,
                dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
                dev_pipeline_max_iterations: 3,
                dev_pipeline_show_trace: false,
                dev_pipeline_architect_instruction: String::new(),
                dev_pipeline_implementer_instruction: String::new(),
                dev_pipeline_reviewer_instruction: String::new(),
                endpoints: Vec::new(),
            };
            tracing::info!("从 config.json 导入旧配置账号: provider={}", provider);
            accounts.push(migrated);
        }
    }

    // b. 从 Codex config.toml 导入
    if let Some(codex_account) = deecodex::codex_config::extract_account_from_codex_config() {
        // 避免重复（如果旧配置已经包含了同样的 upstream）
        let is_duplicate = accounts.iter().any(|a| {
            a.from_codex_config
                || (a.upstream == codex_account.upstream && a.api_key == codex_account.api_key)
        });
        if !is_duplicate {
            accounts.push(codex_account);
        }
    }

    // c. 都没有 → 创建默认 OpenRouter 空账号
    if accounts.is_empty() {
        let presets = get_provider_presets();
        let openrouter = presets.iter().find(|p| p.slug == "openrouter").unwrap();
        let default = Account {
            id: generate_id(),
            name: "默认账号".into(),
            provider: "openrouter".into(),
            client_kind: Default::default(),
            wire_protocol: openrouter.wire_protocol.clone(),
            upstream: openrouter.default_upstream.clone(),
            api_key: String::new(),
            default_model: String::new(),
            client_options: HashMap::new(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: now_secs(),
            updated_at: now_secs(),
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: deecodex::providers::provider_options_for_slug("openrouter"),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        };
        tracing::info!("创建默认 OpenRouter 空账号");
        accounts.push(default);
    }

    let mut store = AccountStore {
        version: deecodex::accounts::ACCOUNT_STORE_VERSION,
        active_id: Some(accounts[0].id.clone()),
        active_account_id: Some(accounts[0].id.clone()),
        active_endpoint_id: None,
        accounts,
    };
    store.normalize_v2();

    // 持久化
    if let Err(e) = deecodex::accounts::save_accounts(data_dir, &store) {
        tracing::warn!("保存迁移后的账号文件失败: {e}");
    } else {
        tracing::info!("首次迁移完成，已保存 {} 个账号", store.accounts.len());
    }

    store
}

/// 从账号存储中读取活跃账号的上下文窗口覆盖值。
fn load_active_account_context_window(data_dir: &std::path::Path) -> Option<u32> {
    let store = deecodex::accounts::load_accounts(data_dir);
    store
        .active_endpoint()
        .and_then(|endpoint| endpoint.context_window_override)
}

fn build_app_state(args: &Args) -> anyhow::Result<handlers::AppState> {
    // 迁移/加载账号
    let account_store = migrate_or_load_accounts(&args.data_dir);

    // 解析活跃账号的配置
    let mut active_account = account_store
        .active_account_id
        .as_ref()
        .or(account_store.active_id.as_ref())
        .and_then(|id| account_store.accounts.iter().find(|a| &a.id == id))
        .filter(|account| account.client_kind.is_codex())
        .or_else(|| {
            account_store
                .accounts
                .iter()
                .find(|account| account.client_kind.is_codex())
        })
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!("没有可用于 deecodex 代理的 Codex 账号，请先创建 Codex 客户端账号")
        })?;

    let active_endpoint = active_account
        .active_endpoint(account_store.active_endpoint_id.as_deref())
        .cloned()
        .or_else(|| active_account.endpoints.first().cloned())
        .ok_or_else(|| anyhow::anyhow!("没有可用于 deecodex 代理的 Codex 账号端点"))?;
    active_account.sync_legacy_from_endpoint(&active_endpoint);

    let model_map: HashMap<String, String> = active_endpoint.model_map.clone();
    let upstream = handlers::validate_upstream(&active_endpoint.base_url).unwrap_or_else(|_| {
        tracing::warn!("活跃账号上游 URL 无效，使用默认 OpenRouter");
        handlers::validate_upstream("https://openrouter.ai/api/v1").unwrap()
    });

    let vision_upstream = if active_endpoint.vision.base_url.is_empty() {
        None
    } else {
        match handlers::validate_upstream(&active_endpoint.vision.base_url) {
            Ok(url) => Some(url),
            Err(e) => {
                tracing::warn!("视觉上游 URL 无效: {e}");
                None
            }
        }
    };

    let file_store = files::FileStore::with_data_dir(&args.data_dir)?;
    let vs_registry = vector_stores::VectorStoreRegistry::with_data_dir(&args.data_dir)?;

    let executors = deecodex::executor::LocalExecutorConfig::from_raw(
        &args.computer_executor,
        args.computer_executor_timeout_secs,
        &args.mcp_executor_config,
        args.mcp_executor_timeout_secs,
    )?;

    let rate_limiter = {
        let rate_limit = std::env::var("DEECODEX_RATE_LIMIT")
            .or_else(|_| std::env::var("CODEX_RELAY_RATE_LIMIT"))
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(120);
        let rate_window = std::env::var("DEECODEX_RATE_WINDOW")
            .or_else(|_| std::env::var("CODEX_RELAY_RATE_WINDOW"))
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);
        if rate_limit > 0 {
            Some(Arc::new(deecodex::ratelimit::RateLimiter::new(
                rate_limit,
                rate_window,
            )))
        } else {
            None
        }
    };

    let vision_api_key = active_endpoint.vision.api_key.clone();
    let vision_model = if active_endpoint.vision.model.is_empty() {
        args.vision_model.clone()
    } else {
        active_endpoint.vision.model.clone()
    };
    let vision_endpoint = if active_endpoint.vision.path.is_empty() {
        args.vision_endpoint.clone()
    } else {
        active_endpoint.vision.path.clone()
    };

    Ok(handlers::AppState {
        sessions: deecodex::session::SessionStore::new(),
        client: reqwest::Client::builder()
            .pool_idle_timeout(None)
            .pool_max_idle_per_host(4)
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
        upstream: Arc::new(tokio::sync::RwLock::new(upstream)),
        api_key: Arc::new(tokio::sync::RwLock::new(active_account.api_key.clone())),
        model_map: Arc::new(tokio::sync::RwLock::new(model_map.clone())),
        vision_upstream: Arc::new(tokio::sync::RwLock::new(vision_upstream)),
        vision_api_key: Arc::new(tokio::sync::RwLock::new(vision_api_key)),
        vision_model: Arc::new(tokio::sync::RwLock::new(vision_model)),
        vision_endpoint: Arc::new(tokio::sync::RwLock::new(vision_endpoint)),
        start_time: std::time::Instant::now(),
        request_cache: deecodex::cache::RequestCache::default(),
        prompts: Arc::new(deecodex::prompts::PromptRegistry::new(&args.prompts_dir)),
        files: file_store,
        vector_stores: vs_registry,
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: args.chinese_thinking,
        codex_auto_inject: args.codex_auto_inject,
        codex_persistent_inject: args.codex_persistent_inject,
        port: args.port,
        rate_limiter,
        metrics: Arc::new(metrics::Metrics::new()),
        tool_policy: Arc::new(tokio::sync::RwLock::new(handlers::ToolPolicy {
            allowed_mcp_servers: parse_csv_list(&args.allowed_mcp_servers),
            allowed_computer_displays: parse_csv_list(&args.allowed_computer_displays),
        })),
        executors: Arc::new(tokio::sync::RwLock::new(executors)),
        token_tracker: Arc::new(deecodex::token_anomaly::TokenTracker::new(
            32,
            args.token_anomaly_prompt_max,
            args.token_anomaly_spike_ratio,
            args.token_anomaly_burn_window,
            args.token_anomaly_burn_rate,
        )),
        data_dir: Arc::new(args.data_dir.clone()),
        codex_launch_with_cdp: args.codex_launch_with_cdp,
        cdp_port: args.cdp_port,
        account_store: Arc::new(tokio::sync::RwLock::new(account_store)),
        active_account: Arc::new(tokio::sync::RwLock::new(active_account)),
        reasoning_effort_override: Arc::new(tokio::sync::RwLock::new(
            active_endpoint.reasoning_effort_override.clone(),
        )),
        thinking_tokens: Arc::new(tokio::sync::RwLock::new(active_endpoint.thinking_tokens)),
        custom_headers: Arc::new(tokio::sync::RwLock::new(
            active_endpoint.custom_headers.clone(),
        )),
        request_timeout_secs: Arc::new(tokio::sync::RwLock::new(
            active_endpoint.request_timeout_secs,
        )),
        request_history: {
            let db_path = args.data_dir.join("request_history.db");
            Arc::new(
                deecodex::request_history::RequestHistoryStore::new(&db_path).unwrap_or_else(|e| {
                    tracing::warn!("请求历史数据库初始化失败，使用内存存储: {e}");
                    deecodex::request_history::RequestHistoryStore::new(std::path::Path::new(
                        ":memory:",
                    ))
                    .unwrap()
                }),
            )
        },
    })
}

async fn running_app_state(manager: &ServerManager) -> Option<handlers::AppState> {
    manager.app_state.lock().await.clone()
}

async fn sync_account_store_to_running_state(manager: &ServerManager, store: &AccountStore) {
    if let Some(app_state) = running_app_state(manager).await {
        *app_state.account_store.write().await = store.clone();
    }
}

async fn sync_active_account_to_running_state(
    manager: &ServerManager,
    store: &AccountStore,
    target: &deecodex::accounts::Account,
) -> Result<(), String> {
    let Some(app_state) = running_app_state(manager).await else {
        return Ok(());
    };

    let upstream_url = deecodex::handlers::validate_upstream(&target.upstream)
        .map_err(|e| format!("目标账号上游 URL 无效: {e}"))?;
    let vision_upstream = if target.vision_upstream.is_empty() {
        None
    } else {
        Some(
            deecodex::handlers::validate_upstream(&target.vision_upstream)
                .map_err(|e| format!("视觉上游 URL 无效: {e}"))?,
        )
    };

    *app_state.upstream.write().await = upstream_url;
    *app_state.api_key.write().await = target.api_key.clone();
    *app_state.model_map.write().await = target.model_map.clone();
    *app_state.vision_upstream.write().await = vision_upstream;
    *app_state.vision_api_key.write().await = target.vision_api_key.clone();
    *app_state.vision_model.write().await = target.vision_model.clone();
    *app_state.vision_endpoint.write().await = target.vision_endpoint.clone();

    *app_state.reasoning_effort_override.write().await = target.reasoning_effort_override.clone();
    *app_state.thinking_tokens.write().await = target.thinking_tokens;
    *app_state.custom_headers.write().await = target.custom_headers.clone();
    *app_state.request_timeout_secs.write().await = target.request_timeout_secs;

    *app_state.active_account.write().await = target.clone();
    *app_state.account_store.write().await = store.clone();

    let port = *manager.port.lock().await;
    deecodex::codex_config::inject(port, target.context_window_override);

    tracing::info!("已同步运行中账号: {} ({})", target.name, target.provider);
    Ok(())
}

// ── 内部函数（托盘和 Tauri 命令共用） ─────────────────────────────────────

pub async fn start_service_inner(manager: &ServerManager) -> Result<ServiceInfo, String> {
    if manager.is_running().await {
        let info = get_status_internal(manager).await;
        return Err(format!("服务已在运行中 (端口: {})", info.port));
    }

    let args = load_args();
    let port = args.port;

    let state = build_app_state(&args).map_err(|e| format!("构建服务状态失败: {e}"))?;

    // 将 AppState 存储到 ServerManager，供 switch_account 等命令使用
    *manager.app_state.lock().await = Some(state.clone());
    // 请求历史数据库独立保存，服务停止后仍可读取
    *manager.request_history.lock().await = Some(state.request_history.clone());

    let app = handlers::build_router(state.clone()).layer(axum::extract::DefaultBodyLimit::max(
        args.max_body_mb * 1024 * 1024,
    ));

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("无法绑定端口 {port}: {e}"))?;

    if args.codex_auto_inject && !args.codex_persistent_inject {
        deecodex::codex_config::fix();
        deecodex::codex_config::inject(port, load_active_account_context_window(&args.data_dir));
    }

    let (tx, mut rx) = tokio::sync::watch::channel(());
    let server = axum::serve(listener, app);

    let handle = tokio::spawn(async move {
        server
            .with_graceful_shutdown(async move {
                rx.changed().await.ok();
            })
            .await
            .ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    *manager.shutdown_tx.lock().await = Some(tx);
    *manager.handle.lock().await = Some(handle);
    *manager.port.lock().await = port;
    *manager.start_time.lock().await = Some(std::time::Instant::now());

    // CDP 注入：自动启动 Codex 桌面版并注入 JS
    if args.codex_launch_with_cdp {
        let cdp_port = args.cdp_port;
        tokio::spawn(async move {
            #[cfg(target_os = "macos")]
            let result = tokio::process::Command::new("open")
                .arg("-a")
                .arg("Codex.app")
                .arg("--args")
                .arg(format!("--remote-debugging-port={cdp_port}"))
                .spawn();
            #[cfg(target_os = "windows")]
            let result = tokio::process::Command::new("Codex.exe")
                .arg(format!("--remote-debugging-port={cdp_port}"))
                .spawn();
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let result: std::io::Result<tokio::process::Child> = Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "CDP 启动不支持当前平台",
            ));
            match result {
                Ok(_) => tracing::info!("已启动 Codex 桌面版 (CDP 端口 {cdp_port})"),
                Err(e) => tracing::warn!("启动 Codex 桌面版失败: {e}"),
            }
        });
    }
    let inject_state = Arc::new(state.clone());
    let cdp_port = args.cdp_port;
    tokio::spawn(async move {
        deecodex::inject::try_inject_with_port(inject_state, cdp_port).await;
    });

    // 写入 PID 文件，供诊断检测服务运行状态
    let pid = std::process::id();
    let _ = std::fs::write(args.data_dir.join("deecodex.pid"), pid.to_string());

    manager.update_tray().await;
    tracing::info!("服务已启动 → http://127.0.0.1:{port}");

    Ok(get_status_internal(manager).await)
}

pub async fn stop_service_inner(manager: &ServerManager) -> Result<ServiceInfo, String> {
    if !manager.is_running().await {
        return Err("服务未在运行".to_string());
    }

    if let Some(tx) = manager.shutdown_tx.lock().await.take() {
        let _ = tx.send(());
    }

    if let Some(handle) = manager.handle.lock().await.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(35), handle).await;
    }

    let args = load_args();
    // 线程已聚合并依赖 deecodex 时才保留注入，否则安全清理。
    let needs_deecodex_injection = {
        let bp = args.data_dir.join("thread_migration_backup.json");
        if bp.exists() {
            std::fs::read_to_string(&bp)
                .ok()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                .and_then(|v| v.get("target_provider")?.as_str().map(|s| s == "deecodex"))
                .unwrap_or(true) // 解析失败则保守保留
        } else {
            false
        }
    };
    if args.codex_auto_inject && !args.codex_persistent_inject && !needs_deecodex_injection {
        deecodex::codex_config::remove();
    }

    // 清理 PID 文件
    let _ = std::fs::remove_file(args.data_dir.join("deecodex.pid"));

    *manager.start_time.lock().await = None;
    *manager.app_state.lock().await = None;
    manager.update_tray().await;
    tracing::info!("服务已停止");

    Ok(get_status_internal(manager).await)
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_service(manager: State<'_, ServerManager>) -> Result<ServiceInfo, String> {
    start_service_inner(&manager).await
}

#[tauri::command]
pub async fn stop_service(manager: State<'_, ServerManager>) -> Result<ServiceInfo, String> {
    stop_service_inner(&manager).await
}

#[tauri::command]
pub async fn get_service_status(manager: State<'_, ServerManager>) -> Result<ServiceInfo, String> {
    Ok(get_status_internal(&manager).await)
}

async fn get_status_internal(manager: &ServerManager) -> ServiceInfo {
    let running = manager.is_running().await;
    let port = *manager.port.lock().await;
    let uptime = if running {
        manager
            .start_time
            .lock()
            .await
            .map(|t| t.elapsed().as_secs())
    } else {
        None
    };
    let args = load_args();
    ServiceInfo {
        running,
        port,
        uptime_secs: uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
        cdp_port: args.cdp_port,
        codex_launch_with_cdp: args.codex_launch_with_cdp,
    }
}

#[tauri::command]
pub fn launch_codex_cdp(manager: State<'_, ServerManager>) -> Result<(), String> {
    let args = load_args();
    let cdp_port = args.cdp_port;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-a")
        .arg("Codex.app")
        .arg("--args")
        .arg(format!("--remote-debugging-port={cdp_port}"))
        .spawn()
        .map_err(|e| format!("启动 Codex 失败: {e}"))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("Codex.exe")
        .arg(format!("--remote-debugging-port={cdp_port}"))
        .spawn()
        .map_err(|e| format!("启动 Codex 失败: {e}"))?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("CDP 启动不支持当前平台".to_string());

    // 启动 Codex 后异步触发 JS 注入
    let app_state =
        tauri::async_runtime::block_on(async { manager.app_state.lock().await.clone() });
    if let Some(state) = app_state {
        tauri::async_runtime::spawn(async move {
            deecodex::inject::try_inject_with_port(std::sync::Arc::new(state), cdp_port).await;
        });
    }

    Ok(())
}

#[tauri::command]
pub fn stop_codex_cdp() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("osascript")
        .arg("-e")
        .arg("quit app \"Codex\"")
        .spawn()
        .map_err(|e| format!("停止 Codex 失败: {e}"))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .arg("/c")
        .arg("taskkill")
        .arg("/f")
        .arg("/im")
        .arg("Codex.exe")
        .spawn()
        .map_err(|e| format!("停止 Codex 失败: {e}"))?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("CDP 停止不支持当前平台".to_string());
    Ok(())
}

#[tauri::command]
pub fn get_config() -> Result<GuiConfig, String> {
    let mut args = load_args();

    // 用活跃账号的字段覆盖 config.json 的对应字段，保证配置面板显示的是实际运行值
    let store = deecodex::accounts::load_accounts(&args.data_dir);
    if let Some(active_id) = &store.active_id {
        if let Some(active) = store.accounts.iter().find(|a| &a.id == active_id) {
            if !active.upstream.is_empty() {
                args.upstream = active.upstream.clone();
            }
            if !active.api_key.is_empty() {
                args.api_key = active.api_key.clone();
            }
            if !active.model_map.is_empty() {
                args.model_map = serde_json::to_string(&active.model_map).unwrap_or_default();
            }
            if !active.vision_upstream.is_empty() {
                args.vision_upstream = active.vision_upstream.clone();
            }
            if !active.vision_api_key.is_empty() {
                args.vision_api_key = active.vision_api_key.clone();
            }
            if !active.vision_model.is_empty() {
                args.vision_model = active.vision_model.clone();
            }
            if !active.vision_endpoint.is_empty() {
                args.vision_endpoint = active.vision_endpoint.clone();
            }
        }
    }

    Ok(GuiConfig::from(args))
}

#[tauri::command]
pub fn save_config(config: GuiConfig) -> Result<(), String> {
    let data_dir = normalize_data_dir(&config.data_dir);
    let config_path = Args::default_config_path(&data_dir);
    let existing = Args::load_from_file(&config_path);
    let account_config = account_backed_config(existing.as_ref());

    // 同步关键字段到 .env（始终写入，空值会清除 .env 中的旧条目）
    Args::sync_to_env_file(&data_dir, "DEECODEX_PORT", &config.port.to_string());
    Args::sync_to_env_file(&data_dir, "DEECODEX_UPSTREAM", &account_config.upstream);
    Args::sync_to_env_file(&data_dir, "DEECODEX_API_KEY", &account_config.api_key);
    Args::sync_to_env_file(&data_dir, "DEECODEX_MODEL_MAP", &account_config.model_map);

    let args = Args {
        command: None,
        config: None,
        port: config.port,
        upstream: account_config.upstream,
        api_key: account_config.api_key,
        model_map: account_config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: account_config.vision_upstream,
        vision_api_key: account_config.vision_api_key,
        vision_model: account_config.vision_model,
        vision_endpoint: account_config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    let config_path = Args::default_config_path(&args.data_dir);
    args.save_to_file(&config_path)
        .map_err(|e| format!("保存配置失败: {e}"))?;

    // 根据更新后的 Codex 注入开关立即应用/移除 Codex config.toml 修改
    let port = args.port;
    if args.codex_auto_inject || args.codex_persistent_inject {
        deecodex::codex_config::fix();
        let cw = load_active_account_context_window(&args.data_dir);
        deecodex::codex_config::inject(port, cw);
    } else {
        deecodex::codex_config::remove();
    }

    tracing::info!("配置已保存 → {}", config_path.display());
    Ok(())
}

#[tauri::command]
pub fn validate_config(config: GuiConfig) -> Vec<Value> {
    let data_dir = normalize_data_dir(&config.data_dir);
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        upstream: config.upstream,
        api_key: config.api_key,
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key: config.vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    deecodex::validate::validate(&args)
        .into_iter()
        .map(|d| {
            json!({
                "severity": match d.severity {
                    deecodex::validate::Severity::Error => "error",
                    deecodex::validate::Severity::Warn => "warn",
                },
                "category": d.category,
                "message": d.message,
            })
        })
        .collect()
}

/// 运行完整诊断（同步，含 14 项检查；连通性检测标记为 Info 待后续异步补全）
#[tauri::command]
pub fn run_diagnostics(config: GuiConfig) -> serde_json::Value {
    let data_dir = normalize_data_dir(&config.data_dir);
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        upstream: config.upstream,
        api_key: config.api_key,
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key: config.vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    let ctx = deecodex::validate::DiagnosticContext::from(&args);
    let report = deecodex::validate::run_diagnostics_sync(&ctx);
    serde_json::to_value(report).unwrap_or_default()
}

/// 运行完整诊断（异步，包含上游 API 连通性检测）
#[tauri::command]
pub async fn run_full_diagnostics(config: GuiConfig) -> Result<serde_json::Value, String> {
    let data_dir = normalize_data_dir(&config.data_dir);
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        upstream: config.upstream.clone(),
        api_key: config.api_key.clone(),
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key: config.vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    let ctx = deecodex::validate::DiagnosticContext::from(&args);
    let mut report = deecodex::validate::run_diagnostics_sync(&ctx);

    // 异步检测上游连通性
    let connectivity = do_test_connectivity(&config.upstream, &config.api_key).await;
    let conn_item = match connectivity {
        Ok(result) => deecodex::validate::connectivity_check_result(
            result.ok,
            result.status_code,
            result.latency_ms,
            result.model_count,
            &result.endpoint,
            result.error.as_deref(),
        ),
        Err(e) => deecodex::validate::connectivity_check_result(
            false,
            0,
            0,
            None,
            &config.upstream,
            Some(&e),
        ),
    };

    // 替换「账号连通」分组中的连通性检查项
    for group in &mut report.groups {
        if group.name == "账号连通" {
            if let Some(item) = group
                .items
                .iter_mut()
                .find(|i| i.check_name == "账号连通性")
            {
                *item = conn_item;
            }
            group.health = deecodex::validate::DiagnosticReport::compute_group_health(&group.items);
            break;
        }
    }

    // 重新计算摘要
    report.summary = deecodex::validate::DiagnosticReport::compute_summary(&report.groups);

    Ok(serde_json::to_value(report).unwrap_or_default())
}

#[tauri::command]
pub async fn check_upgrade() -> Result<Value, String> {
    let args = load_args();
    let version_path = args.data_dir.join("VERSION");
    let current = std::fs::read_to_string(&version_path)
        .or_else(|_| std::fs::read_to_string("../VERSION"))
        .unwrap_or_else(|_| format!("v{}", env!("CARGO_PKG_VERSION")))
        .trim()
        .to_string();

    let latest_tag = fetch_latest_tag().await;

    let cur_ver = parse_version(&current).unwrap_or((0, 0, 0));
    let latest_ver = parse_version(&latest_tag).unwrap_or((0, 0, 0));
    let has_update = latest_ver > cur_ver;

    Ok(json!({
        "current": current,
        "latest": latest_tag,
        "has_update": has_update,
        "changelog": "",
    }))
}

/// 获取最新版本 tag：主站 GitHub API → 兜底 jsDelivr CDN
async fn fetch_latest_tag() -> String {
    let client = match reqwest::Client::builder().user_agent("deecodex").build() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    // 1. GitHub API
    match client
        .get("https://api.github.com/repos/liguan-89/deecodex/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(tag) = body["tag_name"].as_str() {
                    return tag.to_string();
                }
            }
        }
        _ => {}
    }

    // 2. 兜底：jsDelivr CDN 读取 VERSION 文件
    match client
        .get("https://cdn.jsdelivr.net/gh/liguan-89/deecodex@main/VERSION")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(tag) = resp.text().await {
                let tag = tag.trim().to_string();
                if !tag.is_empty() {
                    return tag;
                }
            }
        }
        _ => {}
    }

    String::new()
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim_start_matches('v');
    let parts: Vec<u32> = s.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() >= 3 {
        Some((parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

#[tauri::command]
pub fn run_upgrade() -> Result<String, String> {
    let args = load_args();
    let script_name = if cfg!(windows) {
        "deecodex.bat"
    } else {
        "deecodex.sh"
    };

    let script = find_or_download_script(script_name, &args)?;

    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .arg("/c")
            .arg(format!(
                "timeout /t 1 /nobreak >nul & \"{}\" update",
                script.display()
            ))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("启动升级失败: {e}"))?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("sleep 1 && exec sh {} update", script.display()))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("启动升级失败: {e}"))?;
    }

    Ok("升级已启动，完成后请重启服务".to_string())
}

fn find_or_download_script(script_name: &str, args: &Args) -> Result<std::path::PathBuf, String> {
    // 1. exe 所在目录（CLI .pkg 安装场景）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(script_name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    // 2. ~/.deecodex/（install.sh 场景）
    let deecodex_dir = &args.data_dir;
    let candidate = deecodex_dir.join(script_name);
    if candidate.exists() {
        return Ok(candidate);
    }
    // 3. 自动下载到 ~/.deecodex/
    download_script(script_name, deecodex_dir)
}

fn download_script(
    script_name: &str,
    dest_dir: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let url = format!(
        "https://github.com/liguan-89/deecodex/releases/latest/download/{}",
        script_name
    );
    let dest = dest_dir.join(script_name);
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("创建目录失败: {e}"))?;

    let client = reqwest::blocking::Client::builder()
        .user_agent("deecodex")
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("下载 {} 失败: {e}", script_name))?;

    if !resp.status().is_success() {
        return Err(format!("下载 {} 失败，HTTP {}", script_name, resp.status()));
    }

    let bytes = resp.bytes().map_err(|e| format!("读取响应失败: {e}"))?;
    std::fs::write(&dest, &bytes).map_err(|e| format!("写入 {} 失败: {e}", script_name))?;

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)
            .map_err(|e| format!("读取权限失败: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).map_err(|e| format!("设置权限失败: {e}"))?;
    }

    Ok(dest)
}

// ── 账号管理 Tauri 命令 ────────────────────────────────────────────────────

/// 获取账号列表，Key 字段脱敏后返回
#[tauri::command]
pub async fn list_accounts(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);

    let accounts: Vec<Value> = store
        .accounts
        .iter()
        .map(|account| account_to_value_for_store(account, &store))
        .collect();
    let client_counts = client_account_counts(&store);

    Ok(json!({
        "accounts": accounts,
        "active_id": store.active_id,
        "active_account_id": store.active_account_id,
        "active_endpoint_id": store.active_endpoint_id,
        "client_counts": client_counts,
    }))
}

/// 获取当前活跃账号
#[tauri::command]
pub async fn get_active_account(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);

    let active = store.active_account();

    match active {
        Some(a) => Ok(account_to_value_for_store(a, &store)),
        None => Err("没有活跃账号".to_string()),
    }
}

/// 创建新账号（支持传入完整 account_json，用于前端先编辑后保存的流程）
#[tauri::command]
pub async fn add_account(
    manager: State<'_, ServerManager>,
    provider: String,
    account_json: Option<String>,
) -> Result<Value, String> {
    use deecodex::accounts::{
        generate_id, get_provider_presets, guess_provider, now_secs, Account,
    };

    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let mut new_account = if let Some(json) = account_json {
        let mut a: Account = parse_account_json(&json)?;
        a.id = generate_id();
        if a.provider.is_empty() {
            a.provider = guess_provider(&a.upstream).to_string();
        }
        if a.provider_options.is_empty() {
            a.provider_options = deecodex::providers::provider_options_for_slug(&a.provider);
        }
        a.created_at = now_secs();
        a.updated_at = now_secs();
        a
    } else {
        let presets = get_provider_presets();
        let preset = presets
            .iter()
            .find(|p| p.slug == provider)
            .ok_or_else(|| format!("未知供应商: {provider}"))?;

        Account {
            id: generate_id(),
            name: format!("{} 账号", preset.label),
            provider: provider.clone(),
            client_kind: Default::default(),
            wire_protocol: preset.wire_protocol.clone(),
            upstream: preset.default_upstream.clone(),
            api_key: String::new(),
            default_model: String::new(),
            client_options: HashMap::new(),
            last_applied_at: None,
            last_check: None,
            model_map: Default::default(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: now_secs(),
            updated_at: now_secs(),
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: deecodex::providers::provider_options_for_slug(&provider),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    };
    new_account.normalize_v2();
    if !new_account.client_kind.is_codex() {
        new_account.translate_enabled = false;
        new_account.endpoints.clear();
    }

    let mut candidate_store = store.clone();
    candidate_store.accounts.push(new_account.clone());
    deecodex::accounts::validate_capability_links(&candidate_store).map_err(|e| e.to_string())?;
    deecodex::accounts::validate_dev_pipeline_links(&candidate_store).map_err(|e| e.to_string())?;

    // 如果没有活跃账号，自动设为活跃
    let became_active = store.active_id.is_none() && new_account.client_kind.is_codex();
    if became_active {
        store.active_id = Some(new_account.id.clone());
        store.active_account_id = Some(new_account.id.clone());
        store.active_endpoint_id = new_account.endpoints.first().map(|e| e.id.clone());
    }

    store.accounts.push(new_account.clone());

    if became_active {
        sync_active_account_to_running_state(&manager, &store, &new_account).await?;
    }

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    if !became_active {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    Ok(account_to_value(&new_account))
}

/// 更新账号信息（从前端接收完整 JSON）
#[tauri::command]
pub async fn update_account(
    manager: State<'_, ServerManager>,
    account_json: String,
) -> Result<Value, String> {
    use deecodex::accounts::{guess_provider, now_secs, Account};

    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let updated: Account = parse_account_json(&account_json)?;

    let pos = store
        .accounts
        .iter()
        .position(|a| a.id == updated.id)
        .ok_or_else(|| format!("账号不存在: {}", updated.id))?;

    let mut account = updated.clone();
    // 仅当 provider 为空时自动检测，避免覆盖用户选择
    if account.provider.is_empty() {
        account.provider = guess_provider(&account.upstream).to_string();
    }
    if account.provider_options.is_empty() {
        account.provider_options =
            deecodex::providers::provider_options_for_slug(&account.provider);
    }
    if !account.client_kind.is_codex() {
        account.translate_enabled = false;
        account.endpoints.clear();
    }
    account.normalize_v2();
    let endpoint_for_legacy = if store.active_account_id.as_ref() == Some(&account.id)
        || store.active_id.as_ref() == Some(&account.id)
    {
        account
            .active_endpoint(store.active_endpoint_id.as_deref())
            .cloned()
            .or_else(|| account.endpoints.first().cloned())
    } else {
        account.endpoints.first().cloned()
    };
    if let Some(endpoint) = endpoint_for_legacy.as_ref() {
        account.sync_legacy_from_endpoint(endpoint);
    }
    account.updated_at = now_secs();

    let is_active = account.client_kind.is_codex()
        && (store.active_account_id.as_ref() == Some(&account.id)
            || store.active_id.as_ref() == Some(&account.id));
    store.accounts[pos] = account.clone();
    deecodex::accounts::validate_capability_links(&store).map_err(|e| e.to_string())?;
    deecodex::accounts::validate_dev_pipeline_links(&store).map_err(|e| e.to_string())?;

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    // 如果保存的是活跃账号，立即热更新运行中的服务状态。
    if is_active && manager.app_state.lock().await.is_some() {
        switch_account_inner(&manager, account.id.clone()).await?;
    } else if !is_active {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    let selected_endpoint = endpoint_for_legacy.as_ref();
    Ok(account_to_value_with_endpoint(&account, selected_endpoint))
}

/// 删除账号（拒绝删除最后一个）
#[tauri::command]
pub async fn delete_account(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    if store.accounts.len() <= 1 {
        return Err("不能删除最后一个账号".to_string());
    }

    let was_active =
        store.active_id.as_deref() == Some(&id) || store.active_account_id.as_deref() == Some(&id);

    store.accounts.retain(|a| a.id != id);
    for account in &mut store.accounts {
        if account.capability_account_id.as_deref() == Some(&id) {
            account.capability_enabled = false;
            account.capability_account_id = None;
        }
    }

    let next_active_id = if was_active {
        store
            .accounts
            .iter()
            .find(|account| account.client_kind.is_codex())
            .map(|account| account.id.clone())
    } else {
        None
    };

    // 如果删除的是活跃账号，只切到剩余的 Codex 代理账号；外部客户端不参与代理热切换。
    if was_active {
        store.active_id = next_active_id.clone();
        store.active_account_id = store.active_id.clone();
        store.active_endpoint_id = next_active_id.as_ref().and_then(|next_id| {
            store
                .accounts
                .iter()
                .find(|account| &account.id == next_id)
                .and_then(|account| account.endpoints.first())
                .map(|endpoint| endpoint.id.clone())
        });
    }

    deecodex::accounts::validate_capability_links(&store).map_err(|e| e.to_string())?;
    deecodex::accounts::validate_dev_pipeline_links(&store).map_err(|e| e.to_string())?;

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    if !was_active {
        sync_account_store_to_running_state(&manager, &store).await;
    } else if let Some(next_id) = next_active_id {
        switch_account_inner(&manager, next_id).await?;
    } else {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    Ok(json!({"success": true}))
}

/// 切换活跃账号，同步更新运行中服务的上游/Key/模型映射等热字段
#[tauri::command]
pub(crate) async fn switch_account_inner(
    manager: &ServerManager,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let mut target = store
        .accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("账号不存在: {id}"))?
        .clone();
    if !target.client_kind.is_codex() {
        return Err("非 Codex 客户端账号不参与 deecodex 代理切换，请使用写入配置".into());
    }
    target.normalize_v2();
    let target_endpoint = target
        .active_endpoint(store.active_endpoint_id.as_deref())
        .cloned()
        .or_else(|| target.endpoints.first().cloned())
        .ok_or_else(|| "目标账号没有可用端点".to_string())?;
    target.sync_legacy_from_endpoint(&target_endpoint);

    deecodex::accounts::validate_capability_links(&store).map_err(|e| e.to_string())?;
    deecodex::accounts::validate_dev_pipeline_links(&store).map_err(|e| e.to_string())?;

    store.active_id = Some(id.clone());
    store.active_account_id = Some(id.clone());
    store.active_endpoint_id = Some(target_endpoint.id.clone());

    // 如果服务在运行，先同步更新 AppState 热字段，再写文件。
    // 避免文件已切但 AppState 更新失败导致的不一致。
    sync_active_account_to_running_state(manager, &store, &target).await?;

    // 持久化到文件（无论服务是否运行）
    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    Ok(account_to_value_with_endpoint(
        &target,
        Some(&target_endpoint),
    ))
}

#[tauri::command]
pub async fn switch_account(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    switch_account_inner(&manager, id).await
}

/// 从 Codex 的 config.toml 导入账号
#[tauri::command]
pub async fn import_codex_config(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let mut imported = deecodex::codex_config::extract_account_from_codex_config()
        .ok_or_else(|| "Codex config.toml 中未找到可导入的第三方 provider 配置".to_string())?;
    imported.normalize_v2();

    // 检查是否已存在相同 upstream + key 的账号
    let is_duplicate = store
        .accounts
        .iter()
        .any(|a| a.upstream == imported.upstream && a.api_key == imported.api_key);

    if is_duplicate {
        return Err("已存在相同上游和 Key 的账号，跳过导入".to_string());
    }

    // 如果没有活跃账号，自动设为活跃
    if store.active_id.is_none() {
        store.active_id = Some(imported.id.clone());
        store.active_account_id = Some(imported.id.clone());
        store.active_endpoint_id = imported
            .endpoints
            .first()
            .map(|endpoint| endpoint.id.clone());
    }

    store.accounts.push(imported.clone());

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    Ok(account_to_value(&imported))
}

/// 返回供应商预设列表
#[tauri::command]
pub fn get_provider_presets() -> Result<Value, String> {
    let presets = deecodex::accounts::get_provider_presets();
    let list: Vec<Value> = presets
        .iter()
        .map(|p| {
            json!({
                "slug": p.slug,
                "label": p.label,
                "description": p.description,
                "default_upstream": p.default_upstream,
                "known_models": p.known_models,
                "default_api_key_env": p.default_api_key_env,
                "wire_protocol": p.wire_protocol,
                "auth_scheme": p.auth_scheme,
                "model_discovery": p.model_discovery,
                "capabilities": p.capabilities,
                "capability_labels": p.capability_labels,
                "provider_options": p.provider_options,
            })
        })
        .collect();
    Ok(json!(list))
}

#[tauri::command]
pub fn get_client_profiles() -> Result<Value, String> {
    serde_json::to_value(deecodex::client_integrations::get_client_profiles())
        .map_err(|e| format!("序列化客户端分类失败: {e}"))
}

#[tauri::command]
pub async fn get_client_status(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    serde_json::to_value(deecodex::client_integrations::status(account))
        .map_err(|e| format!("序列化客户端状态失败: {e}"))
}

#[tauri::command]
pub async fn refresh_client_status(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);
    let pos = store
        .accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let report = deecodex::client_integrations::status(&store.accounts[pos]);
    store.accounts[pos].last_check = Some(deecodex::accounts::ClientCheckRecord {
        ok: report.ok,
        checked_at: deecodex::accounts::now_secs(),
        message: report.message.clone(),
        details: serde_json::to_value(&report).unwrap_or_default(),
    });
    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号状态失败: {e}"))?;
    sync_account_store_to_running_state(&manager, &store).await;
    if !store.accounts[pos].client_kind.is_codex() {
        append_account_event(
            &data_dir,
            &account_id,
            &store.accounts[pos].client_kind,
            "client_account_status",
            report.ok,
            &report.message,
            serde_json::to_value(&report).unwrap_or_default(),
        );
    }
    serde_json::to_value(report).map_err(|e| format!("序列化客户端状态失败: {e}"))
}

#[tauri::command]
pub async fn list_client_backups(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    serde_json::to_value(deecodex::client_integrations::list_backups(account))
        .map_err(|e| format!("序列化客户端备份失败: {e}"))
}

#[tauri::command]
pub async fn restore_client_backup(
    manager: State<'_, ServerManager>,
    account_id: String,
    backup_path: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);
    let pos = store
        .accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let report = deecodex::client_integrations::restore_backup_for_account(
        &store.accounts[pos],
        Path::new(&backup_path),
    )
    .map_err(|e| format!("恢复客户端备份失败: {e}"))?;
    store.accounts[pos].last_check = Some(deecodex::accounts::ClientCheckRecord {
        ok: report.ok,
        checked_at: deecodex::accounts::now_secs(),
        message: report.message.clone(),
        details: serde_json::to_value(&report).unwrap_or_default(),
    });
    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号恢复状态失败: {e}"))?;
    sync_account_store_to_running_state(&manager, &store).await;
    append_account_event(
        &data_dir,
        &account_id,
        &store.accounts[pos].client_kind,
        "client_account_restore",
        report.ok,
        &report.message,
        serde_json::to_value(&report).unwrap_or_default(),
    );
    serde_json::to_value(report).map_err(|e| format!("序列化恢复结果失败: {e}"))
}

#[tauri::command]
pub async fn open_client_config(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    ensure_editable_account_config_file(&target.path, account)
        .map_err(|e| format!("准备客户端配置文件失败: {e}"))?;
    open_path_with_system_editor(&target.path)
        .map_err(|e| format!("打开客户端配置文件失败: {e}"))?;
    append_account_event(
        &data_dir,
        &account_id,
        &account.client_kind,
        "client_config_open",
        true,
        "已打开客户端配置文件",
        json!({"config_path": target.path.to_string_lossy(), "format": target.format}),
    );
    Ok(json!({"ok": true, "path": target.path.to_string_lossy(), "format": target.format}))
}

#[tauri::command]
pub async fn get_account_config_file(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    let exists = target.path.exists();
    let content = if exists {
        read_text_file_lossy(&target.path).map_err(|e| format!("读取配置文件失败: {e}"))?
    } else {
        initial_account_config_text(account)
    };
    let validation = validate_config_text_for_editor(target.format, &content);
    let size_bytes = if exists {
        std::fs::metadata(&target.path)
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };
    Ok(json!({
        "ok": true,
        "account_id": account_id,
        "client_kind": account.client_kind,
        "label": target.label,
        "path": target.path.to_string_lossy(),
        "format": target.format,
        "exists": exists,
        "content": content,
        "size_bytes": size_bytes,
        "validation": validation,
    }))
}

#[tauri::command]
pub async fn validate_account_config_file(
    manager: State<'_, ServerManager>,
    account_id: String,
    content: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    Ok(validate_config_text_for_editor(target.format, &content))
}

#[tauri::command]
pub async fn save_account_config_file(
    manager: State<'_, ServerManager>,
    account_id: String,
    content: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    let validation = validate_config_text_for_editor(target.format, &content);
    if validation["ok"].as_bool() != Some(true) {
        return Ok(json!({
            "ok": false,
            "message": "配置文件校验未通过，未写入磁盘",
            "path": target.path.to_string_lossy(),
            "format": target.format,
            "validation": validation,
        }));
    }
    if let Some(parent) = target.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let backup_path = backup_config_file_for_editor(&target.path)
        .map_err(|e| format!("备份配置文件失败: {e}"))?;
    std::fs::write(&target.path, content).map_err(|e| format!("写入配置文件失败: {e}"))?;
    append_account_event(
        &data_dir,
        &account_id,
        &account.client_kind,
        "client_config_save",
        true,
        "配置文件已在客户端编辑并保存",
        json!({
            "config_path": target.path.to_string_lossy(),
            "backup_path": backup_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "format": target.format,
        }),
    );
    Ok(json!({
        "ok": true,
        "message": "配置文件已保存",
        "path": target.path.to_string_lossy(),
        "format": target.format,
        "backup_path": backup_path.map(|p| p.to_string_lossy().to_string()),
        "validation": validation,
    }))
}

struct ConfigEditorTarget {
    path: PathBuf,
    format: &'static str,
    label: &'static str,
}

fn account_config_target(
    account: &deecodex::accounts::Account,
) -> Result<ConfigEditorTarget, String> {
    if account.client_kind.is_codex() {
        let path = deecodex::config::home_dir()
            .ok_or_else(|| "无法定位用户 HOME 目录".to_string())?
            .join(".codex")
            .join("config.toml");
        return Ok(ConfigEditorTarget {
            path,
            format: "toml",
            label: "Codex config.toml",
        });
    }
    let report = deecodex::client_integrations::status(account);
    let path = report
        .config_path
        .as_deref()
        .ok_or_else(|| "客户端配置路径不可用".to_string())?;
    let (format, label) = match account.client_kind {
        AccountClientKind::ClaudeCode => ("json", "Claude Code settings.json"),
        AccountClientKind::Openclaw => ("json", "OpenClaw 配置"),
        AccountClientKind::Hermes => ("yaml", "Hermes config.yaml"),
        AccountClientKind::GenericClient => ("env", "通用客户端 env"),
        AccountClientKind::Codex => unreachable!(),
    };
    Ok(ConfigEditorTarget {
        path: PathBuf::from(path),
        format,
        label,
    })
}

fn ensure_editable_account_config_file(
    path: &Path,
    account: &deecodex::accounts::Account,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let initial = initial_account_config_text(account);
        std::fs::write(path, initial)?;
    }
    Ok(())
}

fn initial_account_config_text(account: &deecodex::accounts::Account) -> String {
    if account.client_kind.is_codex() {
        "# Codex config.toml\n".into()
    } else {
        initial_client_config_text(account)
    }
}

fn initial_client_config_text(account: &deecodex::accounts::Account) -> String {
    match account.client_kind {
        deecodex::accounts::AccountClientKind::ClaudeCode => {
            let model_map = client_model_map_for_editor(account);
            let auth_env = client_auth_env_for_editor(account);
            let mut env = serde_json::Map::new();
            env.insert(auth_env, Value::String(String::new()));
            env.insert(
                "ANTHROPIC_BASE_URL".into(),
                Value::String(account.upstream.clone()),
            );
            if let Some(model) = client_model_for_editor(account, &model_map, "default") {
                env.insert("ANTHROPIC_MODEL".into(), Value::String(model));
            }
            for (slot, key) in [
                ("sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
                ("opus", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
                ("haiku", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            ] {
                if let Some(model) = client_model_for_editor(account, &model_map, slot) {
                    env.insert(key.into(), Value::String(model));
                }
            }
            serde_json::to_string_pretty(&json!({ "env": env })).unwrap_or_else(|_| "{}".into())
                + "\n"
        }
        deecodex::accounts::AccountClientKind::Openclaw => {
            let model_map = client_model_map_for_editor(account);
            let default_model = client_model_for_editor(account, &model_map, "default")
                .unwrap_or_else(|| account.default_model.clone());
            let env_name = client_auth_env_for_editor(account);
            let mut defaults = serde_json::Map::new();
            if !default_model.trim().is_empty() {
                defaults.insert(
                    "model".into(),
                    Value::String(format!("deecodex/{}", default_model.trim())),
                );
            }
            for (slot, key) in [
                ("image", "imageModel"),
                ("image_generation", "imageGenerationModel"),
                ("video_generation", "videoGenerationModel"),
            ] {
                if let Some(model) = client_model_for_editor(account, &model_map, slot) {
                    defaults.insert(key.into(), Value::String(format!("deecodex/{model}")));
                }
            }
            let models: Vec<Value> = client_model_values_for_editor(account, &model_map)
                .into_iter()
                .map(|model| json!({ "id": model, "name": model }))
                .collect();
            serde_json::to_string_pretty(&json!({
                "models": {
                    "providers": {
                        "deecodex": {
                            "baseUrl": account.upstream,
                            "apiKey": { "provider": "default", "source": "env", "id": env_name },
                            "auth": "api-key",
                            "models": models
                        }
                    }
                },
                "agents": { "defaults": defaults }
            }))
            .unwrap_or_else(|_| "{}".into())
                + "\n"
        }
        deecodex::accounts::AccountClientKind::Hermes => {
            let model_map = client_model_map_for_editor(account);
            let default_model = client_model_for_editor(account, &model_map, "default")
                .unwrap_or_else(|| account.default_model.clone());
            let env_name = client_auth_env_for_editor(account);
            let mut out = String::new();
            out.push_str("model:\n");
            out.push_str(&format!("  default: {}\n", yaml_scalar(&default_model)));
            out.push_str(&format!("  provider: {}\n", yaml_scalar(&account.provider)));
            out.push_str(&format!("  base_url: {}\n", yaml_scalar(&account.upstream)));
            out.push_str(&format!("  api_key_env: {}\n", yaml_scalar(&env_name)));
            let mut aux_lines = Vec::new();
            for (slot, path) in [
                ("vision", "vision"),
                ("web_extract", "web_extract"),
                ("compression", "compression"),
                ("session_search", "session_search"),
                ("title_generation", "title_generation"),
            ] {
                if let Some(model) = client_model_for_editor(account, &model_map, slot) {
                    aux_lines.push(format!("  {path}:\n    model: {}\n", yaml_scalar(&model)));
                }
            }
            if !aux_lines.is_empty() {
                out.push_str("auxiliary:\n");
                out.push_str(&aux_lines.join(""));
            }
            out
        }
        deecodex::accounts::AccountClientKind::GenericClient => {
            let model_map = client_model_map_for_editor(account);
            let env_name = client_auth_env_for_editor(account);
            let mut out = String::new();
            out.push_str(&format!("OPENAI_BASE_URL={}\n", account.upstream));
            out.push_str(&format!("{env_name}=\n"));
            if let Some(model) = client_model_for_editor(account, &model_map, "default") {
                out.push_str(&format!("OPENAI_MODEL={model}\n"));
            }
            for (slot, model) in model_map {
                if slot == "default" || model.trim().is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "{}={}\n",
                    generic_model_env_name_for_editor(&slot),
                    model
                ));
            }
            out
        }
        deecodex::accounts::AccountClientKind::Codex => String::new(),
    }
}

fn read_text_file_lossy(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(_) => std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
    }
}

fn backup_config_file_for_editor(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_file_name(format!(
        "{}.deecodex.bak.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        deecodex::accounts::now_secs()
    ));
    std::fs::copy(path, &backup)?;
    Ok(Some(backup))
}

fn validate_config_text_for_editor(format: &str, content: &str) -> Value {
    let mut diagnostics = Vec::new();
    let trimmed = content.trim();
    match format {
        "toml" => {
            if let Err(err) = content.parse::<toml_edit::DocumentMut>() {
                diagnostics
                    .push(json!({"level": "error", "message": format!("TOML 解析失败: {err}")}));
            }
        }
        "json" => {
            if trimmed.is_empty() {
                diagnostics.push(json!({"level": "error", "message": "JSON 配置不能为空"}));
            } else if let Err(err) = serde_json::from_str::<Value>(content) {
                diagnostics
                    .push(json!({"level": "error", "message": format!("JSON 解析失败: {err}")}));
            }
        }
        "yaml" => {
            if !trimmed.is_empty() {
                if let Err(err) = serde_yaml::from_str::<serde_yaml::Value>(content) {
                    diagnostics.push(
                        json!({"level": "error", "message": format!("YAML 解析失败: {err}")}),
                    );
                }
            }
        }
        "env" => {
            for (idx, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((key, _)) = line.split_once('=') else {
                    diagnostics.push(
                        json!({"level": "error", "message": format!("第 {} 行缺少 '='", idx + 1)}),
                    );
                    continue;
                };
                if !key
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
                {
                    diagnostics.push(json!({"level": "warning", "message": format!("第 {} 行环境变量名建议使用大写字母、数字和下划线", idx + 1)}));
                }
            }
        }
        _ => diagnostics
            .push(json!({"level": "warning", "message": format!("未知配置格式: {format}")})),
    }
    let ok = diagnostics
        .iter()
        .all(|item| item["level"].as_str() != Some("error"));
    if ok {
        diagnostics.push(json!({"level": "info", "message": "配置语法校验通过"}));
    }
    json!({
        "ok": ok,
        "format": format,
        "diagnostics": diagnostics,
    })
}

fn client_model_map_for_editor(account: &deecodex::accounts::Account) -> HashMap<String, String> {
    account
        .client_options
        .get("model_map")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|model| !model.is_empty())
                        .map(|model| (key.clone(), model.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn client_model_for_editor(
    account: &deecodex::accounts::Account,
    model_map: &HashMap<String, String>,
    slot: &str,
) -> Option<String> {
    model_map
        .get(slot)
        .cloned()
        .filter(|model| !model.trim().is_empty())
        .or_else(|| {
            if slot == "default" && !account.default_model.trim().is_empty() {
                Some(account.default_model.clone())
            } else {
                None
            }
        })
}

fn client_model_values_for_editor(
    account: &deecodex::accounts::Account,
    model_map: &HashMap<String, String>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(default_model) = client_model_for_editor(account, model_map, "default") {
        out.push(default_model);
    }
    for model in model_map.values() {
        if !model.trim().is_empty() && !out.contains(model) {
            out.push(model.clone());
        }
    }
    out
}

fn client_auth_env_for_editor(account: &deecodex::accounts::Account) -> String {
    account
        .client_options
        .get("auth_env")
        .or_else(|| account.client_options.get("api_key_env"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            match account.provider.as_str() {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openrouter" => "OPENROUTER_API_KEY",
                "minimax" => "MINIMAX_API_KEY",
                _ => "OPENAI_API_KEY",
            }
            .into()
        })
}

fn generic_model_env_name_for_editor(slot: &str) -> String {
    let normalized: String = slot
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("OPENAI_{normalized}_MODEL")
}

fn yaml_scalar(value: &str) -> String {
    if value.is_empty()
        || value.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ':' | '#'
                        | '\''
                        | '"'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | ','
                        | '&'
                        | '*'
                        | '!'
                        | '|'
                        | '>'
                        | '@'
                        | '`'
                )
        })
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.into()
    }
}

fn open_path_with_system_editor(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
}

#[tauri::command]
pub async fn test_client_account(
    manager: State<'_, ServerManager>,
    account_json: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut persisted_account_id = None;
    let account = if let Some(raw) = account_json {
        parse_account_json(&raw)?
    } else {
        let id = account_id.ok_or_else(|| "缺少 account_id 或 account_json".to_string())?;
        persisted_account_id = Some(id.clone());
        let store = deecodex::accounts::load_accounts(&data_dir);
        store
            .accounts
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "账号不存在".to_string())?
    };
    let mut draft = account.clone();
    let report = deecodex::client_integrations::apply(&mut draft, true)
        .map_err(|e| format!("客户端 dry-run 失败: {e}"))?;
    if let Some(id) = persisted_account_id {
        let mut store = deecodex::accounts::load_accounts(&data_dir);
        if let Some(existing) = store.accounts.iter_mut().find(|item| item.id == id) {
            existing.last_check = Some(deecodex::accounts::ClientCheckRecord {
                ok: report.ok,
                checked_at: deecodex::accounts::now_secs(),
                message: report.message.clone(),
                details: serde_json::to_value(&report).unwrap_or_default(),
            });
            deecodex::accounts::save_accounts(&data_dir, &store)
                .map_err(|e| format!("保存账号预检状态失败: {e}"))?;
            sync_account_store_to_running_state(&manager, &store).await;
        }
    }
    if !account.id.trim().is_empty() && !account.client_kind.is_codex() {
        append_account_event(
            &data_dir,
            &account.id,
            &account.client_kind,
            "client_account_dry_run",
            report.ok,
            &report.message,
            serde_json::to_value(&report).unwrap_or_default(),
        );
    }
    serde_json::to_value(report).map_err(|e| format!("序列化 dry-run 结果失败: {e}"))
}

#[tauri::command]
pub async fn apply_client_account(
    manager: State<'_, ServerManager>,
    account_id: String,
    dry_run: Option<bool>,
) -> Result<Value, String> {
    let dry_run = dry_run.unwrap_or(false);
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);
    let pos = store
        .accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let mut account = store.accounts[pos].clone();
    if account.client_kind.is_codex() {
        return Err("Codex 账号请使用「应用」切换代理账号".into());
    }
    let report = deecodex::client_integrations::apply(&mut account, dry_run)
        .map_err(|e| format!("写入客户端配置失败: {e}"))?;
    let now = deecodex::accounts::now_secs();
    account.last_check = Some(deecodex::accounts::ClientCheckRecord {
        ok: report.ok,
        checked_at: now,
        message: report.message.clone(),
        details: serde_json::to_value(&report).unwrap_or_default(),
    });
    store.accounts[pos] = account;
    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号状态失败: {e}"))?;
    sync_account_store_to_running_state(&manager, &store).await;
    append_account_event(
        &data_dir,
        &account_id,
        &store.accounts[pos].client_kind,
        if dry_run {
            "client_account_dry_run"
        } else {
            "client_account_apply"
        },
        report.ok,
        &report.message,
        serde_json::to_value(&report).unwrap_or_default(),
    );
    serde_json::to_value(report).map_err(|e| format!("序列化写入结果失败: {e}"))
}

#[tauri::command]
pub async fn get_account_events(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    Ok(json!(read_account_events(
        &data_dir,
        account_id.as_deref(),
        limit.unwrap_or(20)
    )))
}

#[tauri::command]
pub async fn import_client_accounts(manager: State<'_, ServerManager>) -> Result<Value, String> {
    use deecodex::accounts::{
        generate_id, now_secs, Account, DevPipelineToolMode, DevPipelineTriggerMode,
    };

    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);
    let candidates = deecodex::client_integrations::discover_client_accounts();
    let mut imported_accounts = Vec::new();
    let mut import_events = Vec::new();
    let mut skipped = 0usize;
    for candidate in candidates {
        if store
            .accounts
            .iter()
            .any(|account| same_client_account(account, &candidate))
        {
            skipped += 1;
            continue;
        }
        let now = now_secs();
        let mut account = Account {
            id: generate_id(),
            name: candidate.name.clone(),
            provider: candidate.provider.clone(),
            client_kind: candidate.client_kind.clone(),
            wire_protocol: Default::default(),
            upstream: candidate.upstream.clone(),
            api_key: candidate.api_key.clone(),
            default_model: candidate.default_model.clone(),
            client_options: candidate.client_options.clone(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: now,
            updated_at: now,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: deecodex::providers::provider_options_for_slug(&candidate.provider),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: false,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        };
        account.normalize_v2();
        import_events.push((
            account.id.clone(),
            account.client_kind.clone(),
            json!({
                "source_path": candidate.source_path,
                "warnings": candidate.warnings,
            }),
        ));
        imported_accounts.push(account_to_value(&account));
        store.accounts.push(account);
    }

    if !imported_accounts.is_empty() {
        deecodex::accounts::save_accounts(&data_dir, &store)
            .map_err(|e| format!("保存导入账号失败: {e}"))?;
        sync_account_store_to_running_state(&manager, &store).await;
        for (account_id, client_kind, details) in import_events {
            append_account_event(
                &data_dir,
                &account_id,
                &client_kind,
                "client_account_import",
                true,
                "已从本机客户端配置导入账号",
                details,
            );
        }
    }

    let statuses: Vec<Value> = store
        .accounts
        .iter()
        .filter(|account| !account.client_kind.is_codex())
        .map(|account| {
            serde_json::to_value(deecodex::client_integrations::status(account)).unwrap_or_default()
        })
        .collect();
    let imported_count = imported_accounts.len();
    let message = if imported_count == 0 {
        format!("客户端扫描完成，未发现新的可导入账号（已存在 {skipped} 个）")
    } else {
        format!("已导入 {imported_count} 个客户端账号，跳过 {skipped} 个已存在账号")
    };
    Ok(json!({
        "imported": imported_count,
        "skipped": skipped,
        "accounts": imported_accounts,
        "message": message,
        "statuses": statuses,
    }))
}

fn same_client_account(
    account: &deecodex::accounts::Account,
    candidate: &deecodex::client_integrations::ClientImportCandidate,
) -> bool {
    if account.client_kind != candidate.client_kind {
        return false;
    }
    let existing_path = account
        .client_options
        .get("config_path")
        .and_then(Value::as_str);
    if let (Some(existing), Some(source)) = (existing_path, candidate.source_path.as_deref()) {
        if existing == source {
            return true;
        }
    }
    account.provider == candidate.provider
        && account.upstream == candidate.upstream
        && account.default_model == candidate.default_model
}

#[tauri::command]
pub fn get_endpoint_templates() -> Result<Value, String> {
    serde_json::to_value(deecodex::accounts::get_endpoint_templates())
        .map_err(|e| format!("序列化端点模板失败: {e}"))
}

#[tauri::command]
pub async fn switch_endpoint(
    manager: State<'_, ServerManager>,
    account_id: String,
    endpoint_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);
    let mut account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .cloned()
        .ok_or_else(|| format!("账号不存在: {account_id}"))?;
    if !account.client_kind.is_codex() {
        return Err("非 Codex 客户端账号没有 deecodex 代理端点，请使用写入配置".into());
    }
    account.normalize_v2();
    let endpoint = account
        .endpoints
        .iter()
        .find(|e| e.id == endpoint_id)
        .cloned()
        .ok_or_else(|| format!("端点不存在: {endpoint_id}"))?;

    store.active_id = Some(account_id.clone());
    store.active_account_id = Some(account_id.clone());
    store.active_endpoint_id = Some(endpoint_id);

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存端点切换失败: {e}"))?;

    // 复用账号切换热更新逻辑，将 active_endpoint_id 一并同步进 AppState。
    switch_account_inner(&manager, account_id).await?;
    Ok(json!({
        "account": account_to_value_with_endpoint(&account, Some(&endpoint)),
        "endpoint": endpoint,
    }))
}

// ── 模型列表获取 ──────────────────────────────────────────────────────────

/// 从上游获取模型列表（传入 account_id 时自动查真实 Key）
#[tauri::command]
pub async fn fetch_upstream_models(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    upstream: Option<String>,
    api_key: Option<String>,
    endpoint_kind: Option<String>,
) -> Result<Vec<String>, String> {
    let (upstream, api_key, profile, endpoint_kind) = if let Some(id) = account_id {
        let data_dir = manager.data_dir.lock().await.clone();
        let store = deecodex::accounts::load_accounts(&data_dir);
        let account = store
            .accounts
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| "账号不存在".to_string())?;
        let endpoint = endpoint_for_account_in_store(account, &store);
        (
            endpoint
                .map(|ep| ep.base_url.clone())
                .unwrap_or_else(|| account.upstream.clone()),
            account.api_key.clone(),
            deecodex::providers::profile_for_account(account),
            endpoint.map(|ep| format!("{:?}", ep.kind)),
        )
    } else {
        let upstream = upstream.ok_or("缺少 upstream 参数")?;
        let provider = deecodex::providers::guess_provider(&upstream).to_string();
        (
            upstream,
            api_key.unwrap_or_default(),
            deecodex::providers::profile_by_slug(&provider),
            endpoint_kind,
        )
    };

    let urls = deecodex::providers::model_discovery_url(&profile, &upstream, &api_key)
        .map(|url| vec![url])
        .unwrap_or_else(|| vec![format!("{}/models", upstream.trim_end_matches('/'))]);

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建客户端失败: {e}"))?;
    for url in &urls {
        let req = model_probe_request(&client, url, &api_key, endpoint_kind.as_deref());
        tracing::info!(provider = %profile.slug, "获取上游模型: GET {url}");
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;
                let models = deecodex::providers::parse_models_response(&profile, &body);
                if !models.is_empty() {
                    tracing::info!(provider = %profile.slug, "获取上游模型成功: {} 个模型", models.len());
                    return Ok(models);
                }
                tracing::info!(provider = %profile.slug, "上游模型响应解析为空: {:?}", body);
            }
            Ok(resp) => {
                let status = resp.status();
                let snippet = resp.text().await.unwrap_or_default();
                tracing::info!(
                    "上游模型请求失败 HTTP {}: {}",
                    status.as_u16(),
                    snippet.chars().take(200).collect::<String>()
                );
            }
            Err(e) => {
                tracing::info!("上游模型请求错误: {url} → {e}");
            }
        }
    }
    Err("无法从上游获取模型列表".to_string())
}

/// 查询余额/额度信息，自动探测端点与计费模式
#[derive(Serialize)]
pub struct BalanceInfo {
    pub mode: String,
    pub credit_remaining: Option<f64>,
    pub credit_limit: Option<f64>,
    pub credit_label: Option<String>,
    pub weekly_remaining: Option<String>,
    pub weekly_limit: Option<String>,
    pub hours_5_remaining: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_remains: Option<Vec<ModelRemain>>,
}

#[derive(Serialize)]
pub struct ModelRemain {
    pub model_name: String,
    pub interval_total: f64,
    pub interval_used: f64,
    pub weekly_total: f64,
    pub weekly_used: f64,
}

#[tauri::command]
pub async fn fetch_balance(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<BalanceInfo, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let profile = deecodex::providers::profile_for_account(account);
    let endpoint = endpoint_for_account_in_store(account, &store);
    let upstream = endpoint
        .map(|endpoint| endpoint.base_url.as_str())
        .unwrap_or(&account.upstream)
        .trim_end_matches('/')
        .to_string();
    let api_key = account.api_key.clone();

    if api_key.is_empty() {
        return Ok(BalanceInfo {
            mode: "unsupported".into(),
            credit_remaining: None,
            credit_limit: None,
            credit_label: None,
            weekly_remaining: None,
            weekly_limit: None,
            hours_5_remaining: None,
            model_remains: None,
        });
    }

    let client = reqwest::Client::new();

    let balance_url = endpoint
        .map(|endpoint| endpoint.balance_url.as_str())
        .filter(|url| !url.is_empty())
        .unwrap_or(&account.balance_url);

    // 如果端点/账号配置了自定义 balance_url，直接用该 URL 探测
    if !balance_url.is_empty() {
        let url = balance_url.trim_end_matches('/').to_string();
        let mut req = client.get(&url);
        for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
            req = req.header(name, value);
        }
        tracing::info!("使用自定义 balance_url 探测: {}", url);
        match req.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(body) = resp.json::<Value>().await {
                        if let Some(info) = try_parse_balance(&body) {
                            return Ok(info);
                        }
                        tracing::info!("自定义 balance_url 解析未能匹配: {:?}", body);
                    }
                } else {
                    tracing::info!(
                        "自定义 balance_url HTTP {}: {}",
                        resp.status().as_u16(),
                        url
                    );
                }
            }
            Err(e) => tracing::info!("自定义 balance_url 请求失败: {} → {}", url, e),
        }
        return Ok(BalanceInfo {
            mode: "unsupported".into(),
            credit_remaining: None,
            credit_limit: None,
            credit_label: None,
            weekly_remaining: None,
            weekly_limit: None,
            hours_5_remaining: None,
            model_remains: None,
        });
    }

    // 生成基础 URL 列表：完整 upstream + 去除 /v1、/v1beta、/api/v1 的根路径
    let mut bases = vec![upstream.clone()];
    for strip in &["/v1", "/v1beta", "/api/v1"] {
        if let Some(root) = upstream.strip_suffix(strip) {
            let root = root.to_string();
            if root != upstream && !bases.contains(&root) {
                bases.push(root);
            }
        }
    }

    // 按顺序尝试各端点：(路径后缀, 是否允许返回非 200 也不放弃)
    let probes: Vec<&str> = vec![
        "/v1/coding_plan/remains",
        "/v1/api/openplatform/coding_plan/remains",
        "/user/balance",
        "/auth/key",
        "/v1/auth/key",
        "/api/v1/auth/key",
        "/v1/billing/info",
        "/v1/account/info",
        "/v1/account",
        "/v1/user/info",
        "/v1/billing",
        "/v1/dashboard/billing/credit_grants",
        "/v1/dashboard/billing/subscription",
        "/v1/subscription",
        "/v1/usage",
        "/v1/plan",
        "/v1/quota",
        "/v1/api/user/info",
    ];

    fn try_parse_balance(body: &Value) -> Option<BalanceInfo> {
        // 1. MiniMax 风格: { base_resp: { status_code: 0 }, model_remains: [...] }
        if body["base_resp"]["status_code"].as_i64() == Some(0) {
            if let Some(remains) = body["model_remains"].as_array() {
                let models: Vec<ModelRemain> = remains
                    .iter()
                    .map(|m| ModelRemain {
                        model_name: m["model_name"].as_str().unwrap_or("?").into(),
                        interval_total: m["current_interval_total_count"].as_f64().unwrap_or(0.0),
                        interval_used: m["current_interval_usage_count"].as_f64().unwrap_or(0.0),
                        weekly_total: m["current_weekly_total_count"].as_f64().unwrap_or(0.0),
                        weekly_used: m["current_weekly_usage_count"].as_f64().unwrap_or(0.0),
                    })
                    .collect();
                return Some(BalanceInfo {
                    mode: "coding_plan".into(),
                    credit_remaining: None,
                    credit_limit: None,
                    credit_label: None,
                    weekly_remaining: None,
                    weekly_limit: None,
                    hours_5_remaining: None,
                    model_remains: Some(models),
                });
            }
        }

        // 2. OpenRouter 风格: { data: { limit_remaining, limit, label } }
        let data = body.get("data").unwrap_or(body);
        let cr = data["limit_remaining"].as_f64();
        let cl = data["limit"].as_f64();
        if cr.is_some() || cl.is_some() {
            return Some(BalanceInfo {
                mode: "token_credit".into(),
                credit_remaining: cr,
                credit_limit: cl,
                credit_label: data["label"].as_str().map(String::from),
                weekly_remaining: None,
                weekly_limit: None,
                hours_5_remaining: None,
                model_remains: None,
            });
        }

        // 3. DeepSeek 风格: { balance_infos: [{ total_balance, currency }] }
        if let Some(infos) = body["balance_infos"].as_array() {
            if let Some(first) = infos.first() {
                if let Some(total) = first["total_balance"].as_str() {
                    let cr = total.parse::<f64>().ok();
                    return Some(BalanceInfo {
                        mode: "token_credit".into(),
                        credit_remaining: cr,
                        credit_limit: None,
                        credit_label: first["currency"].as_str().map(String::from),
                        weekly_remaining: None,
                        weekly_limit: None,
                        hours_5_remaining: None,
                        model_remains: None,
                    });
                }
            }
        }

        // 4. data 为数组: { data: [{ balance / credit / quota, ... }] }
        if let Some(arr) = data.as_array().and_then(|a| a.first()) {
            for key in &[
                "balance",
                "credit",
                "credit_remaining",
                "quota",
                "remaining",
            ] {
                if let Some(v) = arr[key].as_f64() {
                    return Some(BalanceInfo {
                        mode: "token_credit".into(),
                        credit_remaining: Some(v),
                        credit_limit: arr["limit"].as_f64().or(arr["credit_limit"].as_f64()),
                        credit_label: arr["currency"].as_str().map(String::from),
                        weekly_remaining: None,
                        weekly_limit: None,
                        hours_5_remaining: None,
                        model_remains: None,
                    });
                }
            }
        }

        // 5. 顶层 token/credit 相关字段
        for key in &[
            "balance",
            "credit",
            "credit_remaining",
            "total_balance",
            "quota",
            "remaining_quota",
            "token_balance",
            "remaining",
        ] {
            if let Some(v) = body[key].as_f64() {
                return Some(BalanceInfo {
                    mode: "token_credit".into(),
                    credit_remaining: Some(v),
                    credit_limit: None,
                    credit_label: body["currency"].as_str().map(String::from),
                    weekly_remaining: None,
                    weekly_limit: None,
                    hours_5_remaining: None,
                    model_remains: None,
                });
            }
        }

        // 6. 订阅模式: { subscription / plan: { weekly_remaining, ... } }
        if let Some(sub) = body.get("subscription").or(body.get("plan")) {
            return Some(BalanceInfo {
                mode: "subscription".into(),
                credit_remaining: None,
                credit_limit: None,
                credit_label: None,
                weekly_remaining: sub
                    .get("weekly_remaining")
                    .and_then(|v| v.as_str().or_else(|| v.as_number().map(|_| "")))
                    .map(|s| s.to_string()),
                weekly_limit: sub
                    .get("weekly_limit")
                    .and_then(|v| v.as_str().or_else(|| v.as_number().map(|_| "")))
                    .map(|s| s.to_string()),
                hours_5_remaining: sub
                    .get("5h_remaining")
                    .or(sub.get("hours_5_remaining"))
                    .and_then(|v| v.as_str().or_else(|| v.as_number().map(|_| "")))
                    .map(|s| s.to_string()),
                model_remains: None,
            });
        }

        None
    }

    for probe in &probes {
        for base in &bases {
            let url = format!("{}{}", base, probe);
            let mut req = client.get(&url);
            for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
                req = req.header(name, value);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        match resp.json::<Value>().await {
                            Ok(body) => {
                                tracing::info!(
                                    "余额探测成功: {} → body keys: {:?}",
                                    url,
                                    body.as_object().map(|o| o.keys().collect::<Vec<_>>())
                                );
                                if let Some(info) = try_parse_balance(&body) {
                                    return Ok(info);
                                }
                                tracing::info!("余额解析未能匹配已知格式: {:?}", body);
                            }
                            Err(e) => tracing::info!("余额探测 JSON 解析失败: {} → {}", url, e),
                        }
                    } else {
                        tracing::info!("余额探测 HTTP {}: {}", status.as_u16(), url);
                    }
                }
                Err(e) => tracing::debug!("余额探测请求失败: {} → {}", url, e),
            }
        }
    }
    tracing::info!("余额探测全部失败: upstream={}, bases={:?}", upstream, bases);

    Ok(BalanceInfo {
        mode: "unsupported".into(),
        credit_remaining: None,
        credit_limit: None,
        credit_label: None,
        weekly_remaining: None,
        weekly_limit: None,
        hours_5_remaining: None,
        model_remains: None,
    })
}

// ── 会话管理 ──────────────────────────────────────────────────────────────

/// 列出所有活跃会话
#[tauri::command]
pub async fn list_sessions(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let responses = state.sessions.list_responses();
    let conversations = state.sessions.list_conversations();
    Ok(json!({
        "responses": responses.iter().map(|r| json!({"id": r.id, "status": r.status})).collect::<Vec<_>>(),
        "conversations": conversations.iter().map(|c| json!({"id": c.id, "message_count": c.message_count})).collect::<Vec<_>>(),
    }))
}

/// 删除会话（先备份）
#[tauri::command]
pub async fn delete_session(
    manager: State<'_, ServerManager>,
    session_type: String,
    session_id: String,
) -> Result<Value, String> {
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;

    let backup_store = deecodex::backup_store::BackupStore::new(state.data_dir.join("backups"))
        .map_err(|e| format!("备份存储初始化失败: {e}"))?;

    match session_type.as_str() {
        "responses" => {
            if let Some((messages, response, input_items)) =
                state.sessions.delete_response_with_data(&session_id)
            {
                let data =
                    json!({"messages": messages, "response": response, "input_items": input_items});
                let token = backup_store
                    .write_backup(&session_id, "response", &data)
                    .unwrap_or_default();
                Ok(
                    json!({"id": session_id, "object": "response.deleted", "deleted": true, "undo_token": token}),
                )
            } else {
                Err(format!("未找到响应: {}", session_id))
            }
        }
        "conversations" => {
            if let Some((messages, items)) =
                state.sessions.delete_conversation_with_data(&session_id)
            {
                let data = json!({"messages": messages, "items": items});
                let token = backup_store
                    .write_backup(&session_id, "conversation", &data)
                    .unwrap_or_default();
                Ok(
                    json!({"id": session_id, "object": "conversation.deleted", "deleted": true, "undo_token": token}),
                )
            } else {
                Err(format!("未找到对话: {}", session_id))
            }
        }
        _ => Err(format!("未知的会话类型: {}", session_type)),
    }
}

/// 撤销删除会话
#[tauri::command]
pub async fn undo_delete_session(
    manager: State<'_, ServerManager>,
    undo_token: String,
) -> Result<Value, String> {
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;

    let backup_store = deecodex::backup_store::BackupStore::new(state.data_dir.join("backups"))
        .map_err(|e| format!("备份存储初始化失败: {e}"))?;
    let backup = backup_store
        .read_backup(&undo_token)
        .map_err(|e| format!("备份未找到: {e}"))?;

    let session_type = backup
        .get("session_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let data = &backup["data"];

    match session_type {
        "response" => {
            let response_id = backup["session_id"].as_str().unwrap_or("");
            let messages: Vec<deecodex::types::ChatMessage> =
                serde_json::from_value(data["messages"].clone())
                    .map_err(|e| format!("备份数据损坏: {e}"))?;
            let response = data["response"].clone();
            let input_items: Vec<Value> =
                serde_json::from_value(data["input_items"].clone()).unwrap_or_default();
            state
                .sessions
                .undo_delete_response(response_id, messages, response, input_items);
        }
        "conversation" => {
            let conversation_id = backup["session_id"].as_str().unwrap_or("");
            let messages: Vec<deecodex::types::ChatMessage> =
                serde_json::from_value(data["messages"].clone())
                    .map_err(|e| format!("备份数据损坏: {e}"))?;
            let items: Vec<Value> =
                serde_json::from_value(data["items"].clone()).unwrap_or_default();
            state
                .sessions
                .undo_delete_conversation(conversation_id, messages, items);
        }
        _ => return Err(format!("未知的会话类型: {}", session_type)),
    }

    let _ = backup_store.delete_backup(&undo_token);
    Ok(json!({"ok": true}))
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────

fn account_to_value(a: &deecodex::accounts::Account) -> Value {
    let endpoint = if a.client_kind.is_codex() {
        a.endpoints.first()
    } else {
        None
    };
    account_to_value_with_endpoint(a, endpoint)
}

fn account_to_value_for_store(
    a: &deecodex::accounts::Account,
    store: &deecodex::accounts::AccountStore,
) -> Value {
    let endpoint = endpoint_for_account_in_store(a, store);
    account_to_value_with_endpoint(a, endpoint)
}

fn endpoint_for_account_in_store<'a>(
    account: &'a deecodex::accounts::Account,
    store: &deecodex::accounts::AccountStore,
) -> Option<&'a deecodex::accounts::EndpointConfig> {
    if !account.client_kind.is_codex() {
        return None;
    }
    if store.active_account_id.as_deref() == Some(&account.id)
        || store.active_id.as_deref() == Some(&account.id)
    {
        account.active_endpoint(store.active_endpoint_id.as_deref())
    } else {
        account.endpoints.first()
    }
}

fn account_to_value_with_endpoint(
    a: &deecodex::accounts::Account,
    endpoint: Option<&deecodex::accounts::EndpointConfig>,
) -> Value {
    let upstream = endpoint
        .map(|endpoint| endpoint.base_url.as_str())
        .unwrap_or(&a.upstream);
    let model_map = endpoint
        .map(|endpoint| endpoint.model_map.clone())
        .unwrap_or_else(|| a.model_map.clone());
    let vision = endpoint.map(|endpoint| &endpoint.vision);
    let balance_url = endpoint
        .map(|endpoint| endpoint.balance_url.as_str())
        .unwrap_or(&a.balance_url);
    let mut value = json!({
        "id": a.id,
        "name": a.name,
        "provider": a.provider,
        "client_kind": a.client_kind,
        "target": a.client_kind,
        "wire_protocol": a.wire_protocol,
        "upstream": upstream,
        "api_key": a.api_key.clone(),
        "default_model": a.default_model,
        "client_options": a.client_options,
        "last_applied_at": a.last_applied_at,
        "last_check": a.last_check,
        "model_map": model_map,
        "vision_upstream": vision.map(|v| v.base_url.clone()).unwrap_or_else(|| a.vision_upstream.clone()),
        "vision_api_key": vision.map(|v| v.api_key.clone()).unwrap_or_else(|| a.vision_api_key.clone()),
        "vision_model": vision.map(|v| v.model.clone()).unwrap_or_else(|| a.vision_model.clone()),
        "vision_endpoint": vision.map(|v| v.path.clone()).unwrap_or_else(|| a.vision_endpoint.clone()),
        "vision_enabled": vision.map(|v| v.mode == deecodex::accounts::VisionMode::Glue).unwrap_or(a.vision_enabled),
        "context_window_override": endpoint.and_then(|e| e.context_window_override),
        "reasoning_effort_override": endpoint.and_then(|e| e.reasoning_effort_override.clone()),
        "thinking_tokens": endpoint.and_then(|e| e.thinking_tokens),
        "custom_headers": endpoint.map(|e| e.custom_headers.clone()).unwrap_or_else(|| a.custom_headers.clone()),
        "request_timeout_secs": endpoint.and_then(|e| e.request_timeout_secs),
        "max_retries": endpoint.and_then(|e| e.max_retries),
        "translate_enabled": endpoint.map(|e| e.kind.is_chat_like()).unwrap_or(a.translate_enabled),
        "provider_options": a.provider_options,
        "capability_enabled": a.capability_enabled,
        "capability_account_id": a.capability_account_id,
        "endpoints": a.endpoints,
        "active_endpoint_name": endpoint.map(|e| e.name.clone()).unwrap_or_default(),
        "active_endpoint_kind": endpoint.map(|e| format!("{:?}", e.kind)).unwrap_or_default(),
        "active_vision_mode": endpoint.map(|e| format!("{:?}", e.vision.mode)).unwrap_or_default(),
        "from_codex_config": a.from_codex_config,
        "balance_url": balance_url,
        "created_at": a.created_at,
        "updated_at": a.updated_at,
    });
    value["dev_pipeline_enabled"] = json!(a.dev_pipeline_enabled);
    value["dev_pipeline_trigger_mode"] = json!(a.dev_pipeline_trigger_mode);
    value["dev_pipeline_command"] = json!(a.dev_pipeline_command);
    value["dev_pipeline_architect_account_id"] = json!(a.dev_pipeline_architect_account_id);
    value["dev_pipeline_implementer_account_id"] = json!(a.dev_pipeline_implementer_account_id);
    value["dev_pipeline_reviewer_account_id"] = json!(a.dev_pipeline_reviewer_account_id);
    value["dev_pipeline_tool_mode"] = json!(a.dev_pipeline_tool_mode);
    value["dev_pipeline_max_iterations"] = json!(a.dev_pipeline_max_iterations);
    value["dev_pipeline_show_trace"] = json!(a.dev_pipeline_show_trace);
    value["dev_pipeline_architect_instruction"] = json!(a.dev_pipeline_architect_instruction);
    value["dev_pipeline_implementer_instruction"] = json!(a.dev_pipeline_implementer_instruction);
    value["dev_pipeline_reviewer_instruction"] = json!(a.dev_pipeline_reviewer_instruction);
    value
}

// ── 线程聚合 ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_threads_status(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let status =
        deecodex::codex_threads::status(&data_dir).map_err(|e| format!("获取线程状态失败: {e}"))?;

    // 校准需求：已迁移但仍有非 deecodex 线程（备份可能过时）
    let calibration_needed = status.migrated && status.non_deecodex_count > 0;

    // 活跃 provider：迁移后为 "deecodex"，否则取数量最多的 provider
    let active_provider = if status.migrated {
        "deecodex"
    } else {
        status
            .summary
            .iter()
            .max_by_key(|s| s.count)
            .map(|s| s.provider.as_str())
            .unwrap_or("(空)")
    };

    Ok(serde_json::json!({
        "summary": status.summary,
        "total": status.total,
        "non_unified_count": status.non_deecodex_count,
        "migrated": status.migrated,
        "calibration_needed": calibration_needed,
        "active_provider": active_provider,
    }))
}

#[tauri::command]
pub async fn list_threads() -> Result<Value, String> {
    let threads =
        deecodex::codex_threads::list_all().map_err(|e| format!("获取线程列表失败: {e}"))?;
    serde_json::to_value(threads).map_err(|e| format!("序列化失败: {e}"))
}

#[tauri::command]
pub async fn migrate_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let diff = deecodex::codex_threads::migrate(&data_dir).map_err(|e| format!("迁移失败: {e}"))?;
    serde_json::to_value(diff).map_err(|e| format!("序列化失败: {e}"))
}

#[tauri::command]
pub async fn restore_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let diff = deecodex::codex_threads::restore(&data_dir).map_err(|e| format!("还原失败: {e}"))?;
    // 还原后若服务未运行，清理 Codex config.toml 中的 deecodex 注入
    if !manager.is_running().await {
        deecodex::codex_config::remove();
    }
    serde_json::to_value(diff).map_err(|e| format!("序列化失败: {e}"))
}

#[tauri::command]
pub async fn calibrate_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let diff =
        deecodex::codex_threads::calibrate(&data_dir).map_err(|e| format!("校准失败: {e}"))?;
    serde_json::to_value(diff).map_err(|e| format!("序列化失败: {e}"))
}

#[tauri::command]
pub async fn get_thread_content(thread_id: String) -> Result<Value, String> {
    let content = deecodex::codex_threads::get_thread_content(&thread_id)
        .map_err(|e| format!("获取线程内容失败: {e}"))?;
    serde_json::to_value(content).map_err(|e| format!("序列化失败: {e}"))
}

#[tauri::command]
pub async fn delete_thread(
    manager: State<'_, ServerManager>,
    thread_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    deecodex::codex_threads::delete_thread(&data_dir, &thread_id)
        .map_err(|e| format!("删除线程失败: {e}"))?;
    Ok(serde_json::json!({ "ok": true, "message": "线程已永久删除" }))
}

/// 连通性检测结果
struct ConnectivityResult {
    ok: bool,
    status_code: u16,
    latency_ms: u128,
    model_count: Option<usize>,
    endpoint: String,
    error: Option<String>,
}

/// 执行上游连通性检测（内部使用）
async fn do_test_connectivity(upstream: &str, api_key: &str) -> Result<ConnectivityResult, String> {
    do_test_connectivity_with_kind(upstream, api_key, None).await
}

async fn do_test_connectivity_with_kind(
    upstream: &str,
    api_key: &str,
    endpoint_kind: Option<&str>,
) -> Result<ConnectivityResult, String> {
    let provider = deecodex::providers::guess_provider(upstream);
    let profile = deecodex::providers::profile_by_slug(provider);
    let base = upstream.trim_end_matches('/');
    let url = deecodex::providers::model_discovery_url(&profile, upstream, api_key)
        .unwrap_or_else(|| format!("{base}/models"));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let req = model_probe_request(&client, &url, api_key, endpoint_kind);
    let start = std::time::Instant::now();
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency_ms = start.elapsed().as_millis();
            let body = resp.text().await.unwrap_or_default();
            let model_count = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .map(|v| deecodex::providers::parse_models_response(&profile, &v).len())
                .filter(|count| *count > 0);
            Ok(ConnectivityResult {
                ok: status < 500,
                status_code: status,
                latency_ms,
                model_count,
                endpoint: url,
                error: None,
            })
        }
        Err(e) => Ok(ConnectivityResult {
            ok: false,
            status_code: 0,
            latency_ms: start.elapsed().as_millis(),
            model_count: None,
            endpoint: url,
            error: Some(e.to_string()),
        }),
    }
}

fn build_vision_probe_url(upstream: &str, vision_path: &str) -> Result<String, String> {
    let upstream = upstream.trim().trim_end_matches('/');
    if upstream.is_empty() {
        return Err("视觉上游 URL 为空".into());
    }
    deecodex::handlers::validate_upstream(upstream)
        .map_err(|e| format!("视觉上游 URL 无效: {e}"))?;

    let path = vision_path.trim().trim_start_matches('/');
    if path.is_empty() {
        return Err("视觉端点路径为空".into());
    }

    let base = if upstream.ends_with("/v1") && path.starts_with("v1/") {
        upstream.trim_end_matches("/v1")
    } else {
        upstream
    };
    Ok(format!("{base}/{path}"))
}

fn classify_minimax_vlm_probe(status: u16, body: &Value) -> (bool, Option<String>, Option<String>) {
    let base_status = body["base_resp"]["status_code"].as_i64();
    let base_msg = body["base_resp"]["status_msg"].as_str().unwrap_or("");
    let content = body["content"].as_str().unwrap_or("");

    if status >= 500 {
        return (
            false,
            None,
            Some(format!("HTTP {status}: {}", truncate_for_ui(base_msg, 180))),
        );
    }

    if matches!(base_status, Some(2049))
        || base_msg.to_ascii_lowercase().contains("invalid api key")
    {
        return (
            false,
            base_status.map(|code| format!("MiniMax base_resp={code}")),
            Some("MiniMax API Key 无效或与当前 API Host 不匹配".into()),
        );
    }

    if !content.is_empty() || base_status == Some(0) {
        return (
            true,
            Some("MiniMax VLM 返回 content，视觉端点可用".into()),
            None,
        );
    }

    if matches!(base_status, Some(2013 | 1026)) {
        return (
            true,
            Some(format!(
                "MiniMax VLM 鉴权通过，探测图片被上游校验拒绝: {base_status:?} {base_msg}"
            )),
            None,
        );
    }

    if status < 500 {
        return (
            true,
            Some(format!(
                "MiniMax VLM 返回 HTTP {status}，base_resp={}",
                base_status
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "未知".into())
            )),
            None,
        );
    }

    (false, None, Some("视觉端点探测失败".into()))
}

fn truncate_for_ui(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

fn model_probe_request(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    endpoint_kind: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut req = client.get(url);
    if api_key.is_empty() {
        return req;
    }
    let is_anthropic = endpoint_kind
        .map(|kind| {
            let kind = kind.to_ascii_lowercase();
            kind.contains("anthropic")
        })
        .unwrap_or_else(|| url.to_ascii_lowercase().contains("anthropic.com"));
    if is_anthropic {
        req = req
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01");
    } else {
        req = req.bearer_auth(api_key);
    }
    req
}

/// 测试胶水视觉 API 端点连通性
#[tauri::command]
pub async fn test_vision_connectivity(
    upstream: String,
    api_key: String,
    vision_path: Option<String>,
    adapter_id: Option<String>,
) -> Result<Value, String> {
    let adapter = adapter_id.unwrap_or_else(|| "minimax_coding_plan_vlm".into());
    if adapter != "minimax_coding_plan_vlm" {
        return Ok(json!({
            "ok": false,
            "status": 0,
            "latency_ms": 0,
            "endpoint": "",
            "adapter": adapter,
            "detail": null,
            "error": "当前版本仅实现 MiniMax Coding Plan VLM 胶水适配器"
        }));
    }

    let endpoint = build_vision_probe_url(
        &upstream,
        vision_path.as_deref().unwrap_or("v1/coding_plan/vlm"),
    )?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let mut req = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&json!({
            "prompt": "deecodex vision connectivity probe",
            "image_url": "https://example.invalid/deecodex-vision-probe.png"
        }));
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }

    let start = std::time::Instant::now();
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.json::<Value>().await.unwrap_or_else(|_| json!({}));
            let (ok, detail, error) = classify_minimax_vlm_probe(status, &body);
            Ok(json!({
                "ok": ok,
                "status": status,
                "latency_ms": start.elapsed().as_millis(),
                "endpoint": endpoint,
                "adapter": adapter,
                "base_status": body["base_resp"]["status_code"],
                "detail": detail,
                "error": error,
            }))
        }
        Err(e) => Ok(json!({
            "ok": false,
            "status": 0,
            "latency_ms": start.elapsed().as_millis(),
            "endpoint": endpoint,
            "adapter": adapter,
            "detail": null,
            "error": e.to_string(),
        })),
    }
}

/// 测试上游 API 端点连通性
#[tauri::command]
pub async fn test_upstream_connectivity(
    upstream: String,
    api_key: String,
    endpoint_kind: Option<String>,
) -> Result<Value, String> {
    let r = do_test_connectivity_with_kind(&upstream, &api_key, endpoint_kind.as_deref()).await?;
    Ok(serde_json::json!({
        "ok": r.ok,
        "status": r.status_code,
        "latency_ms": r.latency_ms,
        "model_count": r.model_count,
        "endpoint": r.endpoint,
        "error": r.error,
    }))
}

// ── 请求历史 ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_request_history(
    manager: State<'_, ServerManager>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        let entries = store.list(limit.unwrap_or(3000)).await;
        return Ok(serde_json::to_value(entries).unwrap_or_default());
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let entries = state.request_history.list(limit.unwrap_or(100)).await;
    Ok(serde_json::to_value(entries).unwrap_or_default())
}

#[tauri::command]
pub async fn clear_request_history(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        store.clear().await?;
        return Ok(json!({ "ok": true }));
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    state.request_history.clear().await?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn get_monthly_stats(
    manager: State<'_, ServerManager>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        let stats = store.list_monthly_stats(limit.unwrap_or(6)).await;
        return Ok(serde_json::to_value(stats).unwrap_or_default());
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let stats = state
        .request_history
        .list_monthly_stats(limit.unwrap_or(6))
        .await;
    Ok(serde_json::to_value(stats).unwrap_or_default())
}

#[tauri::command]
pub async fn get_request_stats_since(
    manager: State<'_, ServerManager>,
    since: Option<u64>,
) -> Result<Value, String> {
    let since_secs = since.unwrap_or(0);
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        let stats = store.stats_since(since_secs).await;
        return Ok(serde_json::to_value(stats).unwrap_or_default());
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let stats = state.request_history.stats_since(since_secs).await;
    Ok(serde_json::to_value(stats).unwrap_or_default())
}

#[tauri::command]
pub async fn browse_file() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .add_filter("插件包", &["zip"])
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().to_string());
    Ok(path)
}

// ── 插件管理 ──────────────────────────────────────────────────────────────

async fn get_pm(manager: &ServerManager) -> Result<Arc<PluginManager>, String> {
    let guard = manager.plugin_manager.lock().await;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "插件管理器未初始化".into())
}

#[tauri::command]
pub async fn list_plugins(manager: State<'_, ServerManager>) -> Result<Vec<Value>, String> {
    let pm = get_pm(&manager).await?;
    let plugins = pm.list().await;
    Ok(plugins
        .iter()
        .map(|p| serde_json::to_value(p).unwrap_or_default())
        .collect())
}

#[tauri::command]
pub async fn install_plugin(
    manager: State<'_, ServerManager>,
    path: Option<String>,
    archive_path: Option<String>,
    plugin_path: Option<String>,
) -> Result<Value, String> {
    let path = path
        .or(archive_path)
        .or(plugin_path)
        .ok_or_else(|| "缺少插件路径".to_string())?;
    let pm = get_pm(&manager).await?;
    let manifest = pm
        .install(std::path::Path::new(&path))
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(&manifest).unwrap_or_default())
}

#[tauri::command]
pub async fn uninstall_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.uninstall(&plugin_id).await.map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn start_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.start(&plugin_id).await.map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn stop_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.stop(&plugin_id).await.map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn update_plugin_config(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    config: Value,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.update_config(&plugin_id, config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn get_plugin_qrcode(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id).await.map_err(|e| e.to_string())?;
    }
    pm.send_request(
        &plugin_id,
        "weixin.login",
        Some(json!({ "account_id": account_id })),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugin_login_cancel(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.send_request(
        &plugin_id,
        "weixin.login_cancel",
        Some(json!({ "account_id": account_id })),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn query_plugin_status(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.send_request(
        &plugin_id,
        "weixin.status",
        Some(json!({ "account_id": account_id })),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_plugin_account(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.send_request(
        &plugin_id,
        "weixin.start",
        Some(json!({ "account_id": account_id })),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn stop_plugin_account(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.send_request(
        &plugin_id,
        "weixin.stop",
        Some(json!({ "account_id": account_id })),
    )
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use deecodex::accounts::Account;
    use std::path::PathBuf;

    fn test_args() -> Args {
        Args {
            command: None,
            config: None,
            port: 4446,
            upstream: "https://openrouter.ai/api/v1".into(),
            api_key: String::new(),
            model_map: "{}".into(),
            max_body_mb: 100,
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: "MiniMax-M1".into(),
            vision_endpoint: "v1/coding_plan/vlm".into(),
            chinese_thinking: false,
            codex_auto_inject: true,
            codex_persistent_inject: false,
            codex_launch_with_cdp: false,
            cdp_port: 9222,
            prompts_dir: PathBuf::from("prompts"),
            data_dir: PathBuf::from(".deecodex"),
            token_anomaly_prompt_max: 200000,
            token_anomaly_spike_ratio: 5.0,
            token_anomaly_burn_window: 120,
            token_anomaly_burn_rate: 500000,
            allowed_mcp_servers: String::new(),
            allowed_computer_displays: String::new(),
            computer_executor: "disabled".into(),
            computer_executor_timeout_secs: 30,
            mcp_executor_config: String::new(),
            mcp_executor_timeout_secs: 30,
            playwright_state_dir: String::new(),
            browser_use_bridge_url: String::new(),
            browser_use_bridge_command: String::new(),
            daemon: false,
        }
    }

    fn test_account(id: &str) -> deecodex::accounts::Account {
        deecodex::accounts::Account {
            id: id.into(),
            name: "Test".into(),
            provider: "deepseek".into(),
            client_kind: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://api.deepseek.com/v1".into(),
            api_key: "test-key".into(),
            default_model: String::new(),
            client_options: HashMap::new(),
            last_applied_at: None,
            last_check: None,
            model_map: Default::default(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 1,
            updated_at: 1,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: Default::default(),
            provider_options: deecodex::providers::provider_options_for_slug("deepseek"),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn parse_account_json_accepts_client_kind_with_legacy_target() {
        let raw = json!({
            "id": "a1",
            "name": "Hermes",
            "provider": "openrouter",
            "client_kind": "hermes",
            "target": "hermes",
            "upstream": "https://openrouter.ai/api/v1",
            "api_key": "sk-test"
        })
        .to_string();

        let account = parse_account_json(&raw).unwrap();

        assert_eq!(account.client_kind, AccountClientKind::Hermes);
    }

    #[test]
    fn parse_account_json_keeps_target_only_legacy_payloads() {
        let raw = json!({
            "id": "a1",
            "name": "Claude Code",
            "provider": "anthropic",
            "target": "claude_code",
            "upstream": "https://api.anthropic.com",
            "api_key": "sk-test"
        })
        .to_string();

        let account = parse_account_json(&raw).unwrap();

        assert_eq!(account.client_kind, AccountClientKind::ClaudeCode);
    }

    #[test]
    fn account_backed_config_preserves_fields_from_existing_config() {
        let mut existing = test_args();
        existing.upstream = "https://account.example/v1".into();
        existing.api_key = "account-key".into();
        existing.model_map = r#"{"gpt-5.5":"deepseek-v4-pro"}"#.into();
        existing.vision_upstream = "https://vision.example/v1".into();
        existing.vision_api_key = "vision-key".into();
        existing.vision_model = "vision-model".into();
        existing.vision_endpoint = "v1/vision".into();

        let preserved = account_backed_config(Some(&existing));

        assert_eq!(preserved.upstream, "https://account.example/v1");
        assert_eq!(preserved.api_key, "account-key");
        assert_eq!(preserved.model_map, r#"{"gpt-5.5":"deepseek-v4-pro"}"#);
        assert_eq!(preserved.vision_upstream, "https://vision.example/v1");
        assert_eq!(preserved.vision_api_key, "vision-key");
        assert_eq!(preserved.vision_model, "vision-model");
        assert_eq!(preserved.vision_endpoint, "v1/vision");
    }

    #[test]
    fn account_backed_config_is_empty_without_existing_config() {
        let preserved = account_backed_config(None);

        assert!(preserved.upstream.is_empty());
        assert!(preserved.api_key.is_empty());
        assert!(preserved.model_map.is_empty());
        assert!(preserved.vision_upstream.is_empty());
        assert!(preserved.vision_api_key.is_empty());
        assert!(preserved.vision_model.is_empty());
        assert!(preserved.vision_endpoint.is_empty());
    }

    #[test]
    fn editable_client_config_seed_is_client_specific_and_redacted() {
        let mut account = test_account("client");
        account.client_kind = AccountClientKind::Hermes;
        account.provider = "minimax".into();
        account.upstream = "https://api.minimaxi.com/v1".into();
        account.api_key = "sk-secret-should-not-leak".into();
        account.default_model = "MiniMax-M2.7".into();
        account
            .client_options
            .insert("api_key_env".into(), json!("MINIMAX_API_KEY"));
        account.client_options.insert(
            "model_map".into(),
            json!({
                "default": "MiniMax-M2.7",
                "vision": "MiniMax-VL-01"
            }),
        );

        let text = initial_client_config_text(&account);

        assert!(text.contains("model:"));
        assert!(text.contains("api_key_env: MINIMAX_API_KEY"));
        assert!(text.contains("vision:"));
        assert!(text.contains("MiniMax-VL-01"));
        assert!(!text.contains("sk-secret-should-not-leak"));
    }

    #[test]
    fn config_editor_validates_common_config_formats() {
        assert_eq!(
            validate_config_text_for_editor("toml", "[model_providers.deecodex]\nname = \"x\"\n")
                ["ok"],
            true
        );
        assert_eq!(
            validate_config_text_for_editor("json", "{\"env\":{}}")["ok"],
            true
        );
        assert_eq!(
            validate_config_text_for_editor("yaml", "model:\n  default: MiniMax-M2.7\n")["ok"],
            true
        );
        assert_eq!(
            validate_config_text_for_editor("env", "OPENAI_MODEL=gpt-5\n")["ok"],
            true
        );
        assert_eq!(validate_config_text_for_editor("json", "{")["ok"], false);
        assert_eq!(
            validate_config_text_for_editor("env", "OPENAI_MODEL")["ok"],
            false
        );
    }

    #[test]
    fn codex_config_editor_uses_codex_toml_target() {
        let mut account = test_account("codex");
        account.client_kind = AccountClientKind::Codex;

        let target = account_config_target(&account).unwrap();

        assert_eq!(target.format, "toml");
        assert!(target.path.ends_with(".codex/config.toml"));
        assert!(initial_account_config_text(&account).contains("Codex config.toml"));
    }

    #[test]
    fn account_to_value_exposes_capability_fields() {
        let account = Account {
            id: "main".into(),
            name: "主账号".into(),
            provider: "deepseek".into(),
            client_kind: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            default_model: String::new(),
            client_options: HashMap::new(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 0,
            updated_at: 0,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: HashMap::new(),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: true,
            capability_account_id: Some("helper".into()),
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        };

        let value = account_to_value(&account);

        assert_eq!(value["capability_enabled"], true);
        assert_eq!(value["capability_account_id"], "helper");
    }

    #[test]
    fn endpoint_selection_uses_active_endpoint_only_for_active_account() {
        let mut active = test_account("active");
        active.name = "Active".into();
        active.provider = "openrouter".into();
        active.upstream = "https://active-default.example/v1".into();
        active.api_key = "active-key".into();
        active.normalize_v2();
        let mut active_second = active.endpoints[0].clone();
        active_second.id = "shared_endpoint_id".into();
        active_second.base_url = "https://active-selected.example/v1".into();
        active.endpoints.push(active_second);

        let mut other = active.clone();
        other.id = "other".into();
        other.name = "Other".into();
        other.endpoints[0].base_url = "https://other-default.example/v1".into();
        other.endpoints.push({
            let mut endpoint = other.endpoints[0].clone();
            endpoint.id = "shared_endpoint_id".into();
            endpoint.base_url = "https://other-shared.example/v1".into();
            endpoint
        });

        let store = deecodex::accounts::AccountStore {
            version: deecodex::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![active.clone(), other.clone()],
            active_id: Some(active.id.clone()),
            active_account_id: Some(active.id.clone()),
            active_endpoint_id: Some("shared_endpoint_id".into()),
        };

        let active_endpoint = endpoint_for_account_in_store(&active, &store).unwrap();
        let other_endpoint = endpoint_for_account_in_store(&other, &store).unwrap();

        assert_eq!(
            active_endpoint.base_url,
            "https://active-selected.example/v1"
        );
        assert_eq!(other_endpoint.base_url, "https://other-default.example/v1");
    }

    #[test]
    fn minimax_vision_probe_url_avoids_duplicate_v1() {
        assert_eq!(
            build_vision_probe_url("https://api.minimaxi.com", "v1/coding_plan/vlm").unwrap(),
            "https://api.minimaxi.com/v1/coding_plan/vlm"
        );
        assert_eq!(
            build_vision_probe_url("https://api.minimaxi.com/v1", "v1/coding_plan/vlm").unwrap(),
            "https://api.minimaxi.com/v1/coding_plan/vlm"
        );
    }

    #[test]
    fn minimax_vlm_probe_treats_validation_response_as_connected() {
        let body = json!({
            "base_resp": {
                "status_code": 2013,
                "status_msg": "invalid params, invalid image URL"
            },
            "content": ""
        });
        let (ok, detail, error) = classify_minimax_vlm_probe(200, &body);

        assert!(ok);
        assert!(detail.unwrap().contains("鉴权通过"));
        assert!(error.is_none());
    }

    #[test]
    fn minimax_vlm_probe_rejects_invalid_api_key() {
        let body = json!({
            "base_resp": {
                "status_code": 2049,
                "status_msg": "invalid api key"
            }
        });
        let (ok, _, error) = classify_minimax_vlm_probe(200, &body);

        assert!(!ok);
        assert!(error.unwrap().contains("API Key"));
    }

    #[test]
    fn account_events_are_filtered_newest_first_and_limited() {
        let data_dir = std::env::temp_dir().join(format!(
            "deecodex-account-events-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(
            data_dir.join("account-events.jsonl"),
            [
                json!({"ts": 10, "account_id": "a", "action": "old"}).to_string(),
                "not-json".to_string(),
                json!({"ts": 30, "account_id": "b", "action": "other"}).to_string(),
                json!({"ts": 20, "account_id": "a", "action": "new"}).to_string(),
            ]
            .join("\n"),
        )
        .unwrap();

        let events = read_account_events(&data_dir, Some("a"), 1);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["action"], "new");

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn same_client_account_matches_path_or_provider_tuple() {
        let mut account = test_account("client");
        account.client_kind = AccountClientKind::Hermes;
        account.provider = "openrouter".into();
        account.upstream = "https://openrouter.ai/api/v1".into();
        account.default_model = "anthropic/claude-sonnet-4".into();
        account
            .client_options
            .insert("config_path".into(), json!("/tmp/hermes.yaml"));

        let mut candidate = deecodex::client_integrations::ClientImportCandidate {
            client_kind: AccountClientKind::Hermes,
            name: "Hermes".into(),
            provider: "anthropic".into(),
            upstream: "https://api.anthropic.com".into(),
            api_key: "sk-test".into(),
            default_model: "claude-sonnet-4-5".into(),
            client_options: HashMap::new(),
            source_path: Some("/tmp/hermes.yaml".into()),
            warnings: Vec::new(),
        };
        assert!(same_client_account(&account, &candidate));

        candidate.source_path = Some("/tmp/other.yaml".into());
        candidate.provider = account.provider.clone();
        candidate.upstream = account.upstream.clone();
        candidate.default_model = account.default_model.clone();
        assert!(same_client_account(&account, &candidate));

        candidate.client_kind = AccountClientKind::ClaudeCode;
        assert!(!same_client_account(&account, &candidate));
    }

    #[test]
    fn migrate_existing_legacy_array_accounts_file_writes_v2() {
        let data_dir = std::env::temp_dir().join(format!(
            "deecodex-migrate-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        let path = deecodex::accounts::accounts_file_path(&data_dir);
        std::fs::write(
            &path,
            serde_json::to_string(&vec![test_account("legacy")]).unwrap(),
        )
        .unwrap();

        let store = migrate_or_load_accounts(&data_dir);
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(store.version, deecodex::accounts::ACCOUNT_STORE_VERSION);
        assert_eq!(
            saved["version"].as_u64(),
            Some(deecodex::accounts::ACCOUNT_STORE_VERSION as u64)
        );
        assert_eq!(
            saved["accounts"][0]["endpoints"].as_array().unwrap().len(),
            1
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn build_app_state_uses_active_endpoint_advanced_fields() {
        let data_dir = std::env::temp_dir().join(format!(
            "deecodex-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut account = test_account("active");
        account.name = "Active".into();
        account.provider = "custom".into();
        account.upstream = "https://legacy.example/v1".into();
        account.api_key = "account-key".into();
        account.normalize_v2();
        let endpoint = account.endpoints.first_mut().unwrap();
        endpoint.id = "selected".into();
        endpoint.base_url = "https://selected.example/v1".into();
        endpoint.kind = deecodex::accounts::EndpointKind::CustomChat;
        endpoint
            .model_map
            .insert("gpt-5".into(), "upstream-model".into());
        endpoint
            .custom_headers
            .insert("x-test".into(), "yes".into());
        endpoint.request_timeout_secs = Some(42);
        endpoint.max_retries = Some(5);
        endpoint.reasoning_effort_override = Some("high".into());
        endpoint.thinking_tokens = Some(2048);
        endpoint.vision.mode = deecodex::accounts::VisionMode::Glue;
        endpoint.vision.base_url = "https://vision.example/v1".into();
        endpoint.vision.api_key = "vision-key".into();
        endpoint.vision.model = "vision-model".into();
        endpoint.vision.path = "v1/coding_plan/vlm".into();

        let store = deecodex::accounts::AccountStore {
            version: deecodex::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![account],
            active_id: Some("active".into()),
            active_account_id: Some("active".into()),
            active_endpoint_id: Some("selected".into()),
        };
        deecodex::accounts::save_accounts(&data_dir, &store).unwrap();

        let mut args = test_args();
        args.data_dir = data_dir.clone();
        args.prompts_dir = data_dir.join("prompts");
        let state = build_app_state(&args).unwrap();

        assert_eq!(
            state.upstream.read().await.as_str(),
            "https://selected.example/v1"
        );
        assert_eq!(state.api_key.read().await.as_str(), "account-key");
        assert_eq!(
            state
                .model_map
                .read()
                .await
                .get("gpt-5")
                .map(String::as_str),
            Some("upstream-model")
        );
        assert_eq!(
            state
                .custom_headers
                .read()
                .await
                .get("x-test")
                .map(String::as_str),
            Some("yes")
        );
        assert_eq!(*state.request_timeout_secs.read().await, Some(42));
        assert_eq!(
            state.reasoning_effort_override.read().await.as_deref(),
            Some("high")
        );
        assert_eq!(*state.thinking_tokens.read().await, Some(2048));
        assert_eq!(
            state
                .vision_upstream
                .read()
                .await
                .as_ref()
                .map(|url| url.as_str().to_string()),
            Some("https://vision.example/v1".into())
        );
        assert_eq!(state.vision_api_key.read().await.as_str(), "vision-key");
        assert_eq!(state.vision_model.read().await.as_str(), "vision-model");
        assert_eq!(
            state.vision_endpoint.read().await.as_str(),
            "v1/coding_plan/vlm"
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }
}
