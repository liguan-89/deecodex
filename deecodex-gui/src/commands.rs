use std::collections::HashMap;
use std::sync::Arc;

use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use deecodex::accounts::AccountStore;
use deecodex::config::Args;
use deecodex::handlers;
use deecodex::{files, metrics, vector_stores};

use crate::ServerManager;

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
    pub client_api_key: String,
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
            client_api_key: a.client_api_key,
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
                    client_api_key: String::new(),
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
                    data_dir: ".deecodex".into(),
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
    args.merge_with_file()
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
        return deecodex::accounts::load_accounts(data_dir);
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
                upstream: file_args.upstream.clone(),
                api_key: file_args.api_key.clone(),
                model_map,
                vision_upstream: file_args.vision_upstream.clone(),
                vision_api_key: file_args.vision_api_key.clone(),
                vision_model: file_args.vision_model.clone(),
                vision_endpoint: file_args.vision_endpoint.clone(),
                from_codex_config: false,
                created_at: now_secs(),
                updated_at: now_secs(),
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
            upstream: openrouter.default_upstream.clone(),
            api_key: String::new(),
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            from_codex_config: false,
            created_at: now_secs(),
            updated_at: now_secs(),
        };
        tracing::info!("创建默认 OpenRouter 空账号");
        accounts.push(default);
    }

    let store = AccountStore {
        active_id: Some(accounts[0].id.clone()),
        accounts,
    };

    // 持久化
    if let Err(e) = deecodex::accounts::save_accounts(data_dir, &store) {
        tracing::warn!("保存迁移后的账号文件失败: {e}");
    } else {
        tracing::info!("首次迁移完成，已保存 {} 个账号", store.accounts.len());
    }

    store
}

fn build_app_state(args: &Args) -> anyhow::Result<handlers::AppState> {
    // 迁移/加载账号
    let account_store = migrate_or_load_accounts(&args.data_dir);

    // 解析活跃账号的配置
    let active_account = account_store
        .active_id
        .as_ref()
        .and_then(|id| account_store.accounts.iter().find(|a| &a.id == id))
        .cloned()
        .unwrap_or_else(|| account_store.accounts[0].clone());

    let model_map: HashMap<String, String> = active_account.model_map.clone();
    let upstream = handlers::validate_upstream(&active_account.upstream).unwrap_or_else(|_| {
        tracing::warn!("活跃账号上游 URL 无效，使用默认 OpenRouter");
        handlers::validate_upstream("https://openrouter.ai/api/v1").unwrap()
    });

    let vision_upstream = if active_account.vision_upstream.is_empty() {
        None
    } else {
        match handlers::validate_upstream(&active_account.vision_upstream) {
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

    let vision_api_key = active_account.vision_api_key.clone();
    let vision_model = if active_account.vision_model.is_empty() {
        args.vision_model.clone()
    } else {
        active_account.vision_model.clone()
    };
    let vision_endpoint = if active_account.vision_endpoint.is_empty() {
        args.vision_endpoint.clone()
    } else {
        active_account.vision_endpoint.clone()
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
        client_api_key: Arc::new(tokio::sync::RwLock::new(args.client_api_key.clone())),
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
    })
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

    let app = handlers::build_router(state.clone())
        .merge(deecodex::web::build_web_router(state.clone()))
        .layer(axum::extract::DefaultBodyLimit::max(
            args.max_body_mb * 1024 * 1024,
        ));

    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("无法绑定端口 {port}: {e}"))?;

    if args.codex_auto_inject && !args.codex_persistent_inject {
        deecodex::codex_config::inject(port, &state.client_api_key.read().await);
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
    if args.codex_auto_inject && !args.codex_persistent_inject {
        deecodex::codex_config::remove();
    }

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
    let data_dir: std::path::PathBuf = std::path::PathBuf::from(&config.data_dir);
    let config_path = Args::default_config_path(&data_dir);
    let existing = Args::load_from_file(&config_path);

    // 掩码保护：若前端传回掩码值，保留原有明文 Key
    let api_key = if config.api_key.contains("****") || config.api_key == "********" {
        existing
            .as_ref()
            .map(|a| a.api_key.clone())
            .unwrap_or_default()
    } else {
        config.api_key.clone()
    };
    let client_api_key =
        if config.client_api_key.contains("****") || config.client_api_key == "********" {
            existing
                .as_ref()
                .map(|a| a.client_api_key.clone())
                .unwrap_or_default()
        } else {
            config.client_api_key.clone()
        };
    let vision_api_key =
        if config.vision_api_key.contains("****") || config.vision_api_key == "********" {
            existing
                .as_ref()
                .map(|a| a.vision_api_key.clone())
                .unwrap_or_default()
        } else {
            config.vision_api_key.clone()
        };

    // 同步关键字段到 .env（始终写入，空值会清除 .env 中的旧条目）
    Args::sync_to_env_file(&data_dir, "DEECODEX_PORT", &config.port.to_string());
    Args::sync_to_env_file(&data_dir, "DEECODEX_UPSTREAM", &config.upstream);
    Args::sync_to_env_file(&data_dir, "DEECODEX_API_KEY", &api_key);
    Args::sync_to_env_file(&data_dir, "DEECODEX_MODEL_MAP", &config.model_map);

    let args = Args {
        command: None,
        config: None,
        port: config.port,
        upstream: config.upstream,
        api_key,
        client_api_key,
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir: config.data_dir.into(),
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
    let ca_key = args.client_api_key.clone();
    if args.codex_auto_inject || args.codex_persistent_inject {
        deecodex::codex_config::inject(port, &ca_key);
    } else {
        deecodex::codex_config::remove();
    }

    tracing::info!("配置已保存 → {}", config_path.display());
    Ok(())
}

#[tauri::command]
pub fn get_logs() -> Vec<String> {
    let args = load_args();
    let log_path = args.data_dir.join("deecodex.log");
    match std::fs::read_to_string(&log_path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
            let start = if lines.len() > 200 {
                lines.len() - 200
            } else {
                0
            };
            lines[start..].iter().map(|s| s.to_string()).collect()
        }
        Err(_) => vec!["(暂无日志)".to_string()],
    }
}

#[tauri::command]
pub fn validate_config(config: GuiConfig) -> Vec<Value> {
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        upstream: config.upstream,
        api_key: config.api_key,
        client_api_key: config.client_api_key,
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
        data_dir: config.data_dir.into(),
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

#[tauri::command]
pub fn update_service() -> Result<String, String> {
    let args = load_args();
    let script_name = if cfg!(windows) {
        "deecodex.bat"
    } else {
        "deecodex.sh"
    };
    let script = args.data_dir.join(script_name);
    if !script.exists() {
        return Err(format!("管理脚本 {} 不存在，请先运行安装脚本", script_name));
    }

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

// ── 账号管理 Tauri 命令 ────────────────────────────────────────────────────

/// 获取账号列表，Key 字段脱敏后返回
#[tauri::command]
pub async fn list_accounts(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);

    let accounts: Vec<Value> = store
        .accounts
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "provider": a.provider,
                "upstream": a.upstream,
                "api_key": a.mask_key(),
                "model_map": a.model_map,
                "vision_upstream": a.vision_upstream,
                "vision_api_key": a.vision_api_key,
                "vision_model": a.vision_model,
                "vision_endpoint": a.vision_endpoint,
                "from_codex_config": a.from_codex_config,
                "created_at": a.created_at,
                "updated_at": a.updated_at,
            })
        })
        .collect();

    Ok(json!({
        "accounts": accounts,
        "active_id": store.active_id,
    }))
}

/// 获取当前活跃账号
#[tauri::command]
pub async fn get_active_account(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);

    let active = store
        .active_id
        .as_ref()
        .and_then(|id| store.accounts.iter().find(|a| &a.id == id));

    match active {
        Some(a) => Ok(json!({
            "id": a.id,
            "name": a.name,
            "provider": a.provider,
            "upstream": a.upstream,
            "api_key": a.mask_key(),
            "model_map": a.model_map,
            "vision_upstream": a.vision_upstream,
            "vision_api_key": a.vision_api_key,
            "vision_model": a.vision_model,
            "vision_endpoint": a.vision_endpoint,
            "from_codex_config": a.from_codex_config,
            "created_at": a.created_at,
            "updated_at": a.updated_at,
        })),
        None => Err("没有活跃账号".to_string()),
    }
}

/// 根据供应商类型创建新账号
#[tauri::command]
pub async fn add_account(
    manager: State<'_, ServerManager>,
    provider: String,
) -> Result<Value, String> {
    use deecodex::accounts::{generate_id, get_provider_presets, now_secs, Account};

    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let presets = get_provider_presets();
    let preset = presets
        .iter()
        .find(|p| p.slug == provider)
        .ok_or_else(|| format!("未知供应商: {provider}"))?;

    let new_account = Account {
        id: generate_id(),
        name: format!("{} 账号", preset.label),
        provider: provider.clone(),
        upstream: preset.default_upstream.clone(),
        api_key: String::new(),
        model_map: Default::default(),
        vision_upstream: String::new(),
        vision_api_key: String::new(),
        vision_model: String::new(),
        vision_endpoint: String::new(),
        from_codex_config: false,
        created_at: now_secs(),
        updated_at: now_secs(),
    };

    // 如果没有活跃账号，自动设为活跃
    if store.active_id.is_none() {
        store.active_id = Some(new_account.id.clone());
    }

    store.accounts.push(new_account.clone());

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

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

    let updated: Account =
        serde_json::from_str(&account_json).map_err(|e| format!("解析账号 JSON 失败: {e}"))?;

    let pos = store
        .accounts
        .iter()
        .position(|a| a.id == updated.id)
        .ok_or_else(|| format!("账号不存在: {}", updated.id))?;

    // 保留原有 api_key 若前端传过来的是脱敏值或空值
    let mut account = updated.clone();
    if account.api_key.is_empty() || account.api_key.contains("****") {
        account.api_key = store.accounts[pos].api_key.clone();
    }
    // 仅当 provider 为空时自动检测，避免覆盖用户选择
    if account.provider.is_empty() {
        account.provider = guess_provider(&account.upstream).to_string();
    }
    account.updated_at = now_secs();

    store.accounts[pos] = account.clone();

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    Ok(account_to_value(&account))
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

    let was_active = store.active_id.as_deref() == Some(&id);

    store.accounts.retain(|a| a.id != id);

    // 如果删除的是活跃账号，切换到第一个
    if was_active {
        store.active_id = Some(store.accounts[0].id.clone());
    }

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    Ok(json!({"success": true}))
}

/// 切换活跃账号，同步更新运行中服务的上游/Key/模型映射等热字段
#[tauri::command]
pub async fn switch_account(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let target = store
        .accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("账号不存在: {id}"))?
        .clone();

    store.active_id = Some(id);

    deecodex::accounts::save_accounts(&data_dir, &store)
        .map_err(|e| format!("保存账号失败: {e}"))?;

    // 如果服务在运行，同步更新 AppState 热字段
    if let Some(app_state) = manager.app_state.lock().await.as_ref() {
        // 更新上游 URL
        let upstream_url = deecodex::handlers::validate_upstream(&target.upstream)
            .map_err(|e| format!("目标账号上游 URL 无效: {e}"))?;
        *app_state.upstream.write().await = upstream_url;

        // 更新 API Key
        *app_state.api_key.write().await = target.api_key.clone();

        // 更新模型映射
        *app_state.model_map.write().await = target.model_map.clone();

        // 更新视觉配置
        let vision_upstream = if target.vision_upstream.is_empty() {
            None
        } else {
            Some(
                deecodex::handlers::validate_upstream(&target.vision_upstream)
                    .map_err(|e| format!("视觉上游 URL 无效: {e}"))?,
            )
        };
        *app_state.vision_upstream.write().await = vision_upstream;
        *app_state.vision_api_key.write().await = target.vision_api_key.clone();
        *app_state.vision_model.write().await = target.vision_model.clone();
        *app_state.vision_endpoint.write().await = target.vision_endpoint.clone();

        // 更新 active_account
        *app_state.active_account.write().await = target.clone();

        // 同步更新 account_store
        *app_state.account_store.write().await = store;

        tracing::info!("已切换活跃账号: {} ({})", target.name, target.provider);
    }

    Ok(account_to_value(&target))
}

/// 从 Codex 的 config.toml 导入账号
#[tauri::command]
pub async fn import_codex_config(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut store = deecodex::accounts::load_accounts(&data_dir);

    let imported = deecodex::codex_config::extract_account_from_codex_config()
        .ok_or_else(|| "Codex config.toml 中未找到可导入的第三方 provider 配置".to_string())?;

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
            })
        })
        .collect();
    Ok(json!(list))
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────

/// 将 Account 转为前端安全的 Value（Key 脱敏）
fn account_to_value(a: &deecodex::accounts::Account) -> Value {
    json!({
        "id": a.id,
        "name": a.name,
        "provider": a.provider,
        "upstream": a.upstream,
        "api_key": a.mask_key(),
        "model_map": a.model_map,
        "vision_upstream": a.vision_upstream,
        "vision_api_key": a.vision_api_key,
        "vision_model": a.vision_model,
        "vision_endpoint": a.vision_endpoint,
        "from_codex_config": a.from_codex_config,
        "created_at": a.created_at,
        "updated_at": a.updated_at,
    })
}
