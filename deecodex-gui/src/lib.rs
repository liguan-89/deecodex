pub mod commands;

use std::io::Write;
use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder},
    LogicalSize, Manager, Size,
};
use tokio::sync::Mutex;

struct FlushWriter<W: Write>(W);

use tauri::Emitter;

impl<W: Write> Write for FlushWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.0.write(buf)?;
        self.0.flush()?;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn env_flag_enabled(value: Option<&str>) -> bool {
    value
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn preview_build() -> bool {
    commands::runtime_defaults().preview
}

fn product_name() -> &'static str {
    if preview_build() {
        "DEX AI Preview"
    } else {
        "DEX AI"
    }
}

const BETA_EXPIRES_AT_UNIX: u64 = 1_780_243_199; // 2026-05-31 23:59:59 Asia/Shanghai

fn beta_trial_expired() -> bool {
    if !env!("CARGO_PKG_VERSION").contains("beta") {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(u64::MAX);
    now > BETA_EXPIRES_AT_UNIX
}

fn show_startup_blocking_alert(title: &str, message: &str) {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display dialog {:?} with title {:?} buttons {{\"退出\"}} default button \"退出\" with icon stop",
            message, title
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("{title}: {message}");
    }
}

pub struct ServerManager {
    pub shutdown_tx: Mutex<Option<tokio::sync::watch::Sender<()>>>,
    pub handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub host: Mutex<String>,
    pub port: Mutex<u16>,
    pub start_time: Mutex<Option<std::time::Instant>>,
    pub tray: Mutex<Option<tauri::tray::TrayIcon>>,
    pub app_handle: Mutex<Option<tauri::AppHandle>>,
    /// 数据目录路径，用于账号文件等持久化存储
    pub data_dir: Mutex<std::path::PathBuf>,
    /// 运行中的 AppState，供 switch_account 等命令更新热字段
    pub app_state: Mutex<Option<deecodex::handlers::AppState>>,
    /// 插件管理器
    pub plugin_manager: Mutex<Option<Arc<deecodex_plugin_host::PluginManager>>>,
    /// 请求历史数据库（独立于 AppState，服务停止后仍可读取）
    pub request_history: Mutex<Option<Arc<deecodex::request_history::RequestHistoryStore>>>,
    /// 应用更新状态，用于托盘菜单提示
    pub update_info: Mutex<Option<UpdateTrayInfo>>,
}

#[derive(Clone, Debug)]
pub struct UpdateTrayInfo {
    pub latest: String,
}

impl ServerManager {
    fn new() -> Self {
        let defaults = commands::runtime_defaults();
        Self {
            shutdown_tx: Mutex::new(None),
            handle: Mutex::new(None),
            host: Mutex::new(deecodex::config::default_host()),
            port: Mutex::new(defaults.port),
            start_time: Mutex::new(None),
            tray: Mutex::new(None),
            app_handle: Mutex::new(None),
            data_dir: Mutex::new(defaults.data_dir),
            app_state: Mutex::new(None),
            plugin_manager: Mutex::new(None),
            request_history: Mutex::new(None),
            update_info: Mutex::new(None),
        }
    }

    async fn is_running(&self) -> bool {
        self.handle
            .lock()
            .await
            .as_ref()
            .is_some_and(|j| !j.is_finished())
    }

    async fn update_tray(&self) {
        let running = self.is_running().await;
        let label = if running { "运行中" } else { "已停止" };
        let app_guard = self.app_handle.lock().await;
        let tray_guard = self.tray.lock().await;
        if let (Some(app), Some(tray)) = (app_guard.as_ref(), tray_guard.as_ref()) {
            let data_dir = self.data_dir.lock().await;
            let update_info = self.update_info.lock().await.clone();
            if let Ok(menu) = build_tray_menu(app, running, &data_dir, update_info.as_ref()) {
                let _ = tray.set_menu(Some(menu));
            }
            let _ = tray.set_tooltip(Some(&format!("{} · {label}", product_name())));
        }
    }
}

fn build_tray_menu(
    app: &tauri::AppHandle,
    running: bool,
    data_dir: &std::path::Path,
    update_info: Option<&UpdateTrayInfo>,
) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    let label = if running { "运行中" } else { "已停止" };
    let status_item = MenuItemBuilder::with_id("status", format!("{} · {label}", product_name()))
        .enabled(false)
        .build(app)?;
    let start_item = MenuItemBuilder::with_id("start", "启动服务")
        .accelerator("CmdOrCtrl+Shift+S")
        .build(app)?;
    let stop_item = MenuItemBuilder::with_id("stop", "停止服务").build(app)?;
    let open_item = MenuItemBuilder::with_id("open", "打开控制面板").build(app)?;
    let update_item = update_info
        .map(|info| {
            MenuItemBuilder::with_id("check_update", format!("发现新版本 {}", info.latest))
                .build(app)
        })
        .transpose()?;
    let quit_item = MenuItemBuilder::with_id("quit", format!("退出 {}", product_name()))
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;

    // 构建账号切换子菜单
    let account_submenu = build_account_submenu(app, data_dir)?;

    let mut menu_builder = MenuBuilder::new(app)
        .item(&status_item)
        .separator()
        .item(&start_item)
        .item(&stop_item)
        .separator()
        .item(&open_item);

    if let Some(update_item) = update_item {
        menu_builder = menu_builder.item(&update_item);
    }

    // 插入账号切换子菜单（如果有账号）
    if let Some(sub) = account_submenu {
        menu_builder = menu_builder.separator().item(&sub);
    }

    let menu = menu_builder.item(&quit_item).build()?;

    Ok(menu)
}

fn build_account_submenu(
    app: &tauri::AppHandle,
    data_dir: &std::path::Path,
) -> Result<Option<tauri::menu::Submenu<tauri::Wry>>, tauri::Error> {
    let store = deecodex::accounts::load_accounts(data_dir);
    tracing::info!(count = %store.accounts.len(), data_dir = %data_dir.display(), "构建账号切换子菜单");
    if store.accounts.is_empty() {
        tracing::warn!(data_dir = %data_dir.display(), "账号列表为空，跳过子菜单");
        return Ok(None);
    }

    let mut submenu = SubmenuBuilder::new(app, "切换账号");
    for acc in &store.accounts {
        let label = if Some(&acc.id) == store.active_id.as_ref() {
            format!("✓ {} · {}", acc.name, acc.provider)
        } else {
            format!("  {} · {}", acc.name, acc.provider)
        };
        let item = MenuItemBuilder::with_id(format!("switch_to_{}", acc.id), label).build(app)?;
        submenu = submenu.item(&item);
    }
    Ok(Some(submenu.build()?))
}

fn make_tray_icon() -> tauri::image::Image<'static> {
    tauri::image::Image::new(include_bytes!("../icons/tray-dex.rgba"), 48, 48)
}

fn find_env_file() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if std::path::Path::new(".env").exists() {
        return Some(PathBuf::from(".env"));
    }
    if let Ok(data_dir) = std::env::var("DEECODEX_DATA_DIR") {
        if !data_dir.trim().is_empty() {
            let data_env = PathBuf::from(data_dir).join(".env");
            if data_env.exists() {
                return Some(data_env);
            }
        }
    }
    if let Some(home) = deecodex::config::home_dir() {
        let data_dir_name = if preview_build() {
            ".deecodex-preview"
        } else {
            ".deecodex"
        };
        let home_env = home.join(data_dir_name).join(".env");
        if home_env.exists() {
            return Some(home_env);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let exe_env = dir.join(".env");
            if exe_env.exists() {
                return Some(exe_env);
            }
        }
    }
    None
}

fn load_env() {
    if let Some(path) = find_env_file() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if let Some(eq) = trimmed.find('=') {
                    let key = trimmed[..eq].trim();
                    let val = trimmed[eq + 1..].trim();
                    let val = if (val.starts_with('"') && val.ends_with('"'))
                        || (val.starts_with('\'') && val.ends_with('\''))
                    {
                        &val[1..val.len() - 1]
                    } else {
                        val
                    };
                    // GUI 配置保存后以数据目录 .env 为准，避免更新重启或父进程继承旧环境值。
                    std::env::set_var(key, val);
                }
            }
        }
    }
}

fn sync_codex_integration_on_gui_start(args: &deecodex::config::Args) {
    if !(args.codex_auto_inject || args.codex_persistent_inject) {
        return;
    }
    deecodex::codex_config::fix();
    deecodex::codex_config::sync_codex_integration(
        deecodex::codex_config::CodexIntegrationSyncOptions {
            host: &args.host,
            port: args.port,
            context_window_override: commands::load_active_account_context_window(&args.data_dir),
            data_dir: Some(&args.data_dir),
            codex_router_mode: &args.codex_router_mode,
            reason: "gui_start",
        },
    );
}

const STARTUP_DESKTOP_INDEX_STABILIZE_ATTEMPTS: usize = 4;
const STARTUP_DESKTOP_INDEX_STABILIZE_DELAY_MS: u64 = 1_500;
const STARTUP_DESKTOP_INDEX_GUARD_ATTEMPTS: usize = 40;
const STARTUP_DESKTOP_INDEX_GUARD_INTERVAL_SECS: u64 = 15;

fn normalize_codex_desktop_threads_once(
    data_dir: &std::path::Path,
    phase: &str,
) -> Option<deecodex::codex_threads::MigrationDiff> {
    match deecodex::codex_threads::normalize_desktop_threads(data_dir) {
        Ok(diff) => {
            if diff.changed_count > 0
                || diff.rollout_metadata_fixed_count > 0
                || diff.remaining_non_unified_count > 0
                || diff.desktop_project_fixed_count > 0
                || diff.desktop_project_pending_count > 0
            {
                tracing::info!(
                    target_provider = %diff.target_provider,
                    changed = diff.changed_count,
                    rollout_metadata_fixed = diff.rollout_metadata_fixed_count,
                    remaining = diff.remaining_non_unified_count,
                    desktop_project_fixed = diff.desktop_project_fixed_count,
                    desktop_project_pending = diff.desktop_project_pending_count,
                    phase,
                    "Codex Desktop 线程启动归一完成"
                );
            } else {
                tracing::debug!(
                    target_provider = %diff.target_provider,
                    phase,
                    "Codex Desktop 线程启动归一无需变更"
                );
            }
            Some(diff)
        }
        Err(err) => {
            tracing::warn!(phase, "Codex Desktop 线程启动归一失败: {err}");
            None
        }
    }
}

fn normalize_codex_desktop_threads_on_startup(data_dir: &std::path::Path) {
    let _ = normalize_codex_desktop_threads_once(data_dir, "startup");
}

fn schedule_codex_desktop_thread_normalization(data_dir: std::path::PathBuf) {
    for (phase, delay_secs) in [
        ("startup-delay-2s", 2_u64),
        ("startup-delay-5s", 5_u64),
        ("startup-delay-20s", 20),
        ("startup-delay-60s", 60),
        ("startup-delay-180s", 180),
    ] {
        let data_dir = data_dir.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            let _ = normalize_codex_desktop_threads_once(&data_dir, phase);
        });
    }

    let stabilize_data_dir = data_dir.clone();
    tauri::async_runtime::spawn(async move {
        for attempt in 1..=STARTUP_DESKTOP_INDEX_STABILIZE_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(
                STARTUP_DESKTOP_INDEX_STABILIZE_DELAY_MS,
            ))
            .await;
            let Ok(status) = deecodex::codex_threads::status(&stabilize_data_dir) else {
                continue;
            };
            if status.desktop_project_pending_count == 0 {
                break;
            }
            tracing::warn!(
                attempt,
                pending = status.desktop_project_pending_count,
                "Codex Desktop 项目索引启动复查仍有待补齐项，重新归一"
            );
            let _ = normalize_codex_desktop_threads_once(&stabilize_data_dir, "startup-stabilize");
        }
    });

    let guard_data_dir = data_dir;
    tauri::async_runtime::spawn(async move {
        for attempt in 1..=STARTUP_DESKTOP_INDEX_GUARD_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_secs(
                STARTUP_DESKTOP_INDEX_GUARD_INTERVAL_SECS,
            ))
            .await;
            let Ok(status) = deecodex::codex_threads::status(&guard_data_dir) else {
                continue;
            };
            if status.desktop_project_pending_count == 0 {
                continue;
            }
            tracing::warn!(
                attempt,
                pending = status.desktop_project_pending_count,
                "Codex Desktop 项目索引后台守护发现待补齐项，重新归一"
            );
            let _ = normalize_codex_desktop_threads_once(&guard_data_dir, "startup-guard");
        }
    });
}

pub fn run() {
    if beta_trial_expired() {
        show_startup_blocking_alert(
            "DEX AI 测试版已过期",
            "此测试版的 7 天使用期限已结束，请安装新的正式版或更新的测试版。",
        );
        std::process::exit(1);
    }

    // 单实例控制：检测已有 GUI 实例，避免重复启动。
    // 开发调试可通过环境变量允许并行窗口，避免影响已安装版本。
    let allow_multi_instance = cfg!(debug_assertions)
        && (env_flag_enabled(
            std::env::var("DEECODEX_GUI_ALLOW_MULTI_INSTANCE")
                .ok()
                .as_deref(),
        ) || env_flag_enabled(std::env::var("DEECODEX_GUI_ALLOW_MULTIPLE").ok().as_deref()));
    if !allow_multi_instance && !preview_build() {
        let current_pid = std::process::id();
        let process_name = "deecodex-gui";
        // 扫描同名进程（排除自身）
        let output = std::process::Command::new("pgrep")
            .arg("-x")
            .arg(process_name)
            .output();
        if let Ok(out) = output {
            let pids: Vec<&str> = std::str::from_utf8(&out.stdout)
                .unwrap_or("")
                .lines()
                .filter(|l| !l.is_empty())
                .collect();
            if pids.len() > 1 || (pids.len() == 1 && pids[0] != current_pid.to_string()) {
                eprintln!(
                    "deecodex-gui 已在运行中 (pid: {}), 本次启动取消",
                    pids.join(", ")
                );
                std::process::exit(1);
            }
        }
    }

    load_env();

    let args = crate::commands::load_args();
    let _ = std::fs::create_dir_all(&args.data_dir);
    let log_path = args.data_dir.join("deecodex.log");
    let mut log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|e| {
            eprintln!("无法打开日志文件 {}: {e}", log_path.display());
            std::process::exit(1);
        });
    // 新文件写入 UTF-8 BOM
    if log_file.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        let _ = log_file.write_all(&[0xEF, 0xBB, 0xBF]);
    }

    tracing_subscriber::fmt()
        .with_writer(move || FlushWriter(log_file.try_clone().expect("日志文件描述符克隆失败")))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "deecodex=info".into()),
        )
        .init();

    normalize_codex_desktop_threads_on_startup(&args.data_dir);
    schedule_codex_desktop_thread_normalization(args.data_dir.clone());
    sync_codex_integration_on_gui_start(&args);

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // 保留 Dock 图标，用户可在 Dock 看到运行状态

            let args = crate::commands::load_args();
            let menu = build_tray_menu(app.handle(), false, &args.data_dir, None)?;

            let icon = make_tray_icon();

            let tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(false)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip(format!("{} · 已停止", product_name()))
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    match id {
                        "start" => {
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let manager = handle.state::<ServerManager>();
                                let _ = commands::start_service_inner(&manager).await;
                            });
                        }
                        "stop" => {
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let manager = handle.state::<ServerManager>();
                                let _ = commands::stop_service_inner(&manager).await;
                            });
                        }
                        "open" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                        "check_update" => {
                            if let Some(window) = app.get_webview_window("main") {
                                let _ = window.show();
                                let _ = window.set_focus();
                                let _ = window.emit("show-update", ());
                            }
                        }
                        "quit" => {
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let manager = handle.state::<ServerManager>();
                                let _ = commands::stop_service_inner(&manager).await;
                                handle.exit(0);
                            });
                        }
                        id if id.starts_with("switch_to_") => {
                            let account_id = id.strip_prefix("switch_to_").unwrap_or("").to_string();
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let manager = handle.state::<ServerManager>();
                                match commands::switch_account_inner(&manager, account_id).await {
                                    Ok(v) => {
                                        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                        tracing::info!(name = %name, "托盘切换账号成功");
                                        // 通知前端刷新
                                        let _ = handle.emit("account-switched", &v);
                                    }
                                    Err(e) => {
                                        tracing::error!(error = %e, "托盘切换账号失败");
                                    }
                                }
                                manager.update_tray().await;
                            });
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // 托盘启动时也显示主窗口
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_size(Size::Logical(LogicalSize {
                    width: 1000.0,
                    height: 650.0,
                }));
                let _ = window.show();
            }

            // 存储托盘引用、AppHandle 和 data_dir 到状态管理器
            let manager = app.state::<ServerManager>();
            let app_handle = app.handle().clone();
            let args = crate::commands::load_args();
            let data_dir = args.data_dir.clone();
            let llm_base_url = format!("http://127.0.0.1:{}", args.port);
            tauri::async_runtime::block_on(async {
                // 初始化插件管理器
                let pm = Arc::new(deecodex_plugin_host::PluginManager::new(
                    data_dir.clone(),
                    llm_base_url.clone(),
                ));
                tracing::info!(llm_base_url = %llm_base_url, "插件管理器已初始化");

                // 自动安装内置微信插件
                let mut weixin_paths: Vec<std::path::PathBuf> = vec![
                    // 开发环境 — 项目树内
                    std::path::PathBuf::from("../deecodex-plugins/plugins/deecodex-weixin"),
                    std::path::PathBuf::from("deecodex-plugins/plugins/deecodex-weixin"),
                ];
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(dir) = exe.parent() {
                        weixin_paths.push(dir.join("deecodex-weixin"));
                        weixin_paths.push(dir.join("../Resources/deecodex-weixin"));
                    }
                }
                let weixin_id = "deecodex-weixin";
                let already_installed = pm.list().await.iter().any(|p| p.id == weixin_id);
                if !already_installed {
                    for p in &weixin_paths {
                        if p.join("plugin.json").exists() {
                            tracing::info!(path = %p.display(), "发现内置微信插件，自动安装");
                            match pm.install(p).await {
                                Ok(manifest) => {
                                    tracing::info!(id = %manifest.id, version = %manifest.version, "内置微信插件安装成功");
                                    break;
                                }
                                Err(e) => {
                                    tracing::warn!(path = %p.display(), error = %e, "内置微信插件安装失败");
                                }
                            }
                        }
                    }
                }

                *manager.plugin_manager.lock().await = Some(pm);
                *manager.tray.lock().await = Some(tray);
                *manager.app_handle.lock().await = Some(app_handle);
                *manager.data_dir.lock().await = data_dir;
            });

            if args.codex_auto_inject || args.codex_persistent_inject {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let manager = handle.state::<ServerManager>();
                    match commands::start_service_inner(&manager).await {
                        Ok(info) => {
                            tracing::info!(
                                host = %info.host,
                                port = info.port,
                                "DEX AI 启动时已自动启动本地代理服务"
                            );
                        }
                        Err(error) if error.contains("服务已在运行中") => {
                            tracing::info!(%error, "DEX AI 启动时检测到本地代理已运行");
                        }
                        Err(error) => {
                            tracing::warn!(%error, "DEX AI 启动时自动启动本地代理失败");
                        }
                    }
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .manage(ServerManager::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_service,
            commands::start_window_drag,
            commands::stop_service,
            commands::get_service_status,
            commands::get_config,
            commands::save_config,
            commands::logs::get_logs,
            commands::logs::clear_logs,
            commands::logs::debug_gui_state,
            commands::validate_config,
            commands::check_upgrade,
            commands::run_upgrade,
            commands::restart_app,
            commands::exit_app,
            commands::run_diagnostics,
            commands::run_full_diagnostics,
            commands::launch_codex_cdp,
            commands::stop_codex_cdp,
            commands::cdp_debug::cdp_debug_snapshot,
            commands::cdp_debug::cdp_threads_probe,
            commands::list_accounts,
            commands::get_active_account,
            commands::get_dex_assistant_account,
            commands::set_dex_assistant_account,
            commands::copy_account_secret,
            commands::add_account,
            commands::update_account,
            commands::start_oauth_account_login,
            commands::poll_oauth_account_login,
            commands::cancel_oauth_account_login,
            commands::open_external_url,
            commands::delete_account,
            commands::switch_account,
            commands::clear_account_cooldown,
            commands::reset_account_runtime_state,
            commands::set_account_routing,
            commands::import_auth_json_accounts,
            commands::import_codex_config,
            commands::get_provider_presets,
            commands::get_client_profiles,
            commands::get_client_status,
            commands::refresh_client_status,
            commands::list_client_backups,
            commands::restore_client_backup,
            commands::dex_quick_configure_client,
            commands::get_codex_quick_start_status,
            commands::apply_codex_quick_start,
            commands::open_client_config,
            commands::get_account_config_file,
            commands::validate_account_config_file,
            commands::save_account_config_file,
            commands::get_claude_desktop_developer_mode,
            commands::set_claude_desktop_developer_mode,
            commands::test_client_account,
            commands::apply_client_account,
            commands::get_account_events,
            commands::import_client_accounts,
            commands::get_endpoint_templates,
            commands::switch_endpoint,
            commands::fetch_upstream_models,
            commands::fetch_balance,
            commands::test_upstream_connectivity,
            commands::test_vision_connectivity,
            commands::list_sessions,
            commands::list_request_history,
            commands::clear_request_history,
            commands::get_monthly_stats,
            commands::get_request_stats_since,
            commands::get_threads_status,
            commands::list_threads,
            commands::get_thread_sources,
            commands::list_client_threads,
            commands::migrate_threads,
            commands::normalize_threads,
            commands::restore_threads,
            commands::calibrate_threads,
            commands::get_thread_content,
            commands::get_client_thread_content,
            commands::delete_thread,
            commands::pin_thread,
            commands::archive_thread,
            commands::browse_file,
            commands::browse_plugin_package,
            commands::browse_plugin_directory,
            commands::create_plugin_from_template,
            commands::validate_plugin_path,
            commands::package_plugin_directory,
            commands::open_plugin_directory,
            commands::open_plugin_marketplace_directory,
            commands::browse_attachment_file,
            commands::list_plugins,
            commands::list_plugin_events,
            commands::list_plugin_marketplace,
            commands::preview_plugin_install,
            commands::install_plugin,
            commands::update_plugin,
            commands::uninstall_plugin,
            commands::start_plugin,
            commands::stop_plugin,
            commands::set_plugin_enabled,
            commands::update_plugin_config,
            commands::upsert_plugin_account_asset,
            commands::remove_plugin_account_asset,
            commands::clear_plugin_cache,
            commands::execute_plugin_feature,
            commands::get_plugin_qrcode,
            commands::plugin_login_cancel,
            commands::dex::dex_chat,
            commands::dex::dex_list_capabilities,
            commands::dex::dex_list_tools,
            commands::dex::dex_execute_tool,
            commands::dex::dex_get_workspace_context,
            commands::dex::dex_update_capability_state,
            commands::dex::dex_read_file,
            commands::dex::dex_list_directory,
            commands::dex::dex_detect_processes,
            commands::dex::dex_client_lifecycle_status,
            commands::dex::dex_install_client,
            commands::dex::dex_launch_client,
            commands::dex::dex_pick_client_launch_dir,
            commands::dex::dex_toggle_desktop_client,
            commands::dex::dex_force_quit_client,
            commands::dex::dex_detect_ports,
            commands::dex::dex_get_env_info,
            commands::dex::dex_execute_shell,
            commands::dex::dex_search_logs,
            commands::dex::dex_get_codex_config_raw,
            commands::dex::dex_health_summary,
            commands::dex::dex_analyze_requests,
            commands::dex::dex_config_backup,
            commands::dex::dex_config_diff,
            commands::dex::dex_token_cost,
            commands::dex::dex_speed_test,
            commands::dex::dex_thread_cleanup,
            commands::dex::dex_auto_tune,
            commands::dex::dex_claude_mcp_check,
            commands::dex::dex_claude_env_overview,
            commands::dex::dex_openclaw_env_overview,
            commands::dex::dex_openclaw_health_check,
            commands::dex::dex_openclaw_mcp_check,
            commands::dex::dex_openclaw_gateway_overview,
            commands::dex::dex_openclaw_agents_overview,
            commands::dex::dex_openclaw_models_overview,
            commands::dex::dex_openclaw_approvals_overview,
            commands::dex::dex_hermes_env_overview,
            commands::dex::dex_hermes_doctor_check,
            commands::dex::dex_hermes_skills_overview,
            commands::dex::dex_hermes_config_overview,
            commands::dex::dex_hermes_gateway_overview,
            commands::dex::dex_ai_toolchain_overview,
            commands::dex::dex_network_topology,
            commands::dex::dex_ssl_check,
            commands::dex::dex_export_report,
            commands::query_plugin_status,
            commands::start_plugin_account,
            commands::stop_plugin_account,
        ])
        .build(tauri::generate_context!())
        .expect("启动 DEX AI GUI 失败")
        .run(|app_handle, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::{beta_trial_expired, env_flag_enabled, BETA_EXPIRES_AT_UNIX};

    #[test]
    fn gui_allow_multiple_flag_accepts_common_truthy_values() {
        assert!(env_flag_enabled(Some("1")));
        assert!(env_flag_enabled(Some("true")));
        assert!(env_flag_enabled(Some(" YES ")));
    }

    #[test]
    fn gui_allow_multiple_flag_rejects_empty_and_falsey_values() {
        assert!(!env_flag_enabled(None));
        assert!(!env_flag_enabled(Some("")));
        assert!(!env_flag_enabled(Some("false")));
        assert!(!env_flag_enabled(Some("0")));
    }

    #[test]
    fn beta_expiry_timestamp_is_seven_day_window() {
        assert_eq!(BETA_EXPIRES_AT_UNIX, 1_780_243_199);
    }

    #[test]
    fn beta_expiry_gate_is_version_scoped() {
        if !env!("CARGO_PKG_VERSION").contains("beta") {
            assert!(!beta_trial_expired());
        }
    }
}
