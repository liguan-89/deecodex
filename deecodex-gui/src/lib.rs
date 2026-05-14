mod commands;

use std::io::Write;
use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Manager,
};
use tokio::sync::Mutex;
use tracing;

struct FlushWriter<W: Write>(W);

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

pub struct ServerManager {
    pub shutdown_tx: Mutex<Option<tokio::sync::watch::Sender<()>>>,
    pub handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
}

impl ServerManager {
    fn new() -> Self {
        Self {
            shutdown_tx: Mutex::new(None),
            handle: Mutex::new(None),
            port: Mutex::new(4446),
            start_time: Mutex::new(None),
            tray: Mutex::new(None),
            app_handle: Mutex::new(None),
            data_dir: Mutex::new(std::path::PathBuf::from(".deecodex")),
            app_state: Mutex::new(None),
            plugin_manager: Mutex::new(None),
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
            if let Ok(menu) = build_tray_menu(app, running) {
                let _ = tray.set_menu(Some(menu));
            }
            let _ = tray.set_tooltip(Some(&format!("deecodex · {label}")));
        }
    }
}

fn build_tray_menu(
    app: &tauri::AppHandle,
    running: bool,
) -> Result<tauri::menu::Menu<tauri::Wry>, tauri::Error> {
    let label = if running { "运行中" } else { "已停止" };
    let status_item = MenuItemBuilder::with_id("status", format!("deecodex · {label}"))
        .enabled(false)
        .build(app)?;
    let start_item = MenuItemBuilder::with_id("start", "启动服务")
        .accelerator("CmdOrCtrl+Shift+S")
        .build(app)?;
    let stop_item = MenuItemBuilder::with_id("stop", "停止服务").build(app)?;
    let open_item = MenuItemBuilder::with_id("open", "打开控制面板").build(app)?;
    let quit_item = MenuItemBuilder::with_id("quit", "退出 deecodex")
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&status_item)
        .separator()
        .item(&start_item)
        .item(&stop_item)
        .separator()
        .item(&open_item)
        .item(&quit_item)
        .build()?;

    Ok(menu)
}

fn find_env_file() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if std::path::Path::new(".env").exists() {
        return Some(PathBuf::from(".env"));
    }
    if let Some(home) = deecodex::config::home_dir() {
        let home_env = home.join(".deecodex").join(".env");
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
                    std::env::set_var(key, val);
                }
            }
        }
    }
}

pub fn run() {
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

    tauri::Builder::default()
        .setup(|app| {
            // 保留 Dock 图标，用户可在 Dock 看到运行状态

            let menu = build_tray_menu(app.handle(), false)?;

            let icon = {
                // 生成 48x48 青色菱形托盘图标（RGBA），高分辨率让形状更清晰
                let w = 48u32;
                let h = 48u32;
                let center = 24.0;
                let outer_r = 22.0; // 外层菱形半对角线
                let inner_r = 7.0; // 内层挖空半对角线（接近 CSS logo 比例）
                let feather = 1.2; // 边缘羽化
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                for y in 0..h {
                    for x in 0..w {
                        let dx = (x as f32 - center + 0.5).abs();
                        let dy = (y as f32 - center + 0.5).abs();
                        let d = dx + dy; // 曼哈顿距离 → 菱形
                        if d <= inner_r || d >= outer_r + feather {
                            rgba.extend_from_slice(&[0, 0, 0, 0]);
                        } else if d >= outer_r {
                            let a = (255.0 * (1.0 - (d - outer_r) / feather)) as u8;
                            rgba.extend_from_slice(&[0, 200, 232, a]);
                        } else if d <= inner_r + feather {
                            let a = (255.0 * ((d - inner_r) / feather)) as u8;
                            rgba.extend_from_slice(&[0, 200, 232, a]);
                        } else {
                            rgba.extend_from_slice(&[0, 200, 232, 255]);
                        }
                    }
                }
                tauri::image::Image::new_owned(rgba, w, h)
            };

            let tray = TrayIconBuilder::new()
                .icon(icon)
                .icon_as_template(false)
                .menu(&menu)
                .tooltip("deecodex · 已停止")
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
                        "quit" => {
                            let handle = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let manager = handle.state::<ServerManager>();
                                let _ = commands::stop_service_inner(&manager).await;
                                handle.exit(0);
                            });
                        }
                        _ => {}
                    }
                })
                .build(app)?;

            // 托盘启动时也显示主窗口
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }

            // 存储托盘引用、AppHandle 和 data_dir 到状态管理器
            let manager = app.state::<ServerManager>();
            let app_handle = app.handle().clone();
            let data_dir = crate::commands::load_args().data_dir.clone();
            tauri::async_runtime::block_on(async {
                // 初始化插件管理器
                let pm = Arc::new(deecodex_plugin_host::PluginManager::new(
                    data_dir.clone(),
                    "http://127.0.0.1:4446".to_string(),
                ));
                tracing::info!("插件管理器已初始化");

                // 自动安装内置微信插件
                let mut weixin_paths: Vec<std::path::PathBuf> = vec![
                    // 开发环境 — 项目树内
                    std::path::PathBuf::from("../deecodex-plugins/plugins/deecodex-weixin"),
                    std::path::PathBuf::from("deecodex-plugins/plugins/deecodex-weixin"),
                    // macOS 开发机绝对路径
                    std::path::PathBuf::from(
                        "/Users/liguan/projects/deecodex-gui-worktree/deecodex-plugins/plugins/deecodex-weixin",
                    ),
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

                // 自启动已安装的插件
                let plugins = pm.list().await;
                for p in &plugins {
                    tracing::info!(plugin_id = %p.id, "自启动插件");
                    if let Err(e) = pm.start(&p.id).await {
                        tracing::warn!(plugin_id = %p.id, error = %e, "插件自启动失败");
                    }
                }

                *manager.plugin_manager.lock().await = Some(pm);
                *manager.tray.lock().await = Some(tray);
                *manager.app_handle.lock().await = Some(app_handle);
                *manager.data_dir.lock().await = data_dir;
            });

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
            commands::stop_service,
            commands::get_service_status,
            commands::get_config,
            commands::save_config,
            commands::get_logs,
            commands::validate_config,
            commands::run_diagnostics,
            commands::run_full_diagnostics,
            commands::update_service,
            commands::launch_codex_cdp,
            commands::stop_codex_cdp,
            commands::list_accounts,
            commands::get_active_account,
            commands::add_account,
            commands::update_account,
            commands::delete_account,
            commands::switch_account,
            commands::import_codex_config,
            commands::get_provider_presets,
            commands::fetch_upstream_models,
            commands::fetch_balance,
            commands::test_upstream_connectivity,
            commands::list_sessions,
            commands::delete_session,
            commands::undo_delete_session,
            commands::list_request_history,
            commands::clear_request_history,
            commands::get_monthly_stats,
            commands::get_threads_status,
            commands::list_threads,
            commands::migrate_threads,
            commands::restore_threads,
            commands::calibrate_threads,
            commands::get_thread_content,
            commands::delete_thread,
            commands::browse_file,
            commands::list_plugins,
            commands::install_plugin,
            commands::uninstall_plugin,
            commands::start_plugin,
            commands::stop_plugin,
            commands::update_plugin_config,
            commands::get_plugin_qrcode,
            commands::plugin_login_cancel,
            commands::query_plugin_status,
            commands::start_plugin_account,
            commands::stop_plugin_account,
        ])
        .run(tauri::generate_context!())
        .expect("启动 deecodex GUI 失败");
}
