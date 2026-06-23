use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::codex_config::{self, CodexIntegrationCheckOptions, CodexIntegrationSyncOptions};
use crate::config::{self, Args};

const DEBOUNCE_DELAY: Duration = Duration::from_millis(600);

#[derive(Debug, Clone)]
pub struct CodexConfigGuardOptions {
    pub host: String,
    pub port: u16,
    pub context_window_override: Option<u32>,
    pub data_dir: PathBuf,
    pub codex_router_mode: String,
    pub codex_config_guard: bool,
    pub codex_auto_inject: bool,
    pub codex_persistent_inject: bool,
}

impl CodexConfigGuardOptions {
    pub fn from_args(args: &Args, context_window_override: Option<u32>) -> Self {
        Self {
            host: config::normalize_host(&args.host),
            port: args.port,
            context_window_override,
            data_dir: args.data_dir.clone(),
            codex_router_mode: config::normalize_codex_router_mode(&args.codex_router_mode),
            codex_config_guard: args.codex_config_guard,
            codex_auto_inject: args.codex_auto_inject,
            codex_persistent_inject: args.codex_persistent_inject,
        }
    }
}

#[derive(Debug)]
struct GuardPolicy {
    enabled: bool,
    codex_router_mode: String,
}

pub async fn run_codex_config_guard(
    options: CodexConfigGuardOptions,
    mut shutdown: watch::Receiver<()>,
) {
    if !guard_enabled(&options) {
        debug!("跳过 Codex 配置智能守护: 未启用");
        return;
    }

    let Some(config_path) = codex_config::codex_config_path() else {
        warn!("跳过 Codex 配置智能守护: 无法确定 HOME 目录");
        return;
    };
    let Some(parent) = config_path.parent().map(Path::to_path_buf) else {
        warn!("跳过 Codex 配置智能守护: config.toml 路径异常");
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&parent) {
        warn!(path = %parent.display(), "跳过 Codex 配置智能守护: 无法创建目录: {err}");
        return;
    }

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |result| {
        let _ = event_tx.send(result);
    }) {
        Ok(watcher) => watcher,
        Err(err) => {
            warn!("启动 Codex 配置智能守护失败: {err}");
            return;
        }
    };
    if let Err(err) = watcher.watch(&parent, RecursiveMode::NonRecursive) {
        warn!(path = %parent.display(), "监听 Codex 配置目录失败: {err}");
        return;
    }

    restore_if_needed(&options, "config_guard_start");
    info!(
        path = %config_path.display(),
        mode = %options.codex_router_mode,
        "Codex 配置智能守护已启动"
    );

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() {
                    break;
                }
                break;
            }
            Some(result) = event_rx.recv() => {
                let relevant = match result {
                    Ok(event) => config_event_relevant(&event.paths, &config_path),
                    Err(err) => {
                        warn!("Codex 配置智能守护收到文件事件错误: {err}");
                        false
                    }
                };
                if !relevant {
                    continue;
                }

                tokio::time::sleep(DEBOUNCE_DELAY).await;
                while let Ok(result) = event_rx.try_recv() {
                    if let Err(err) = result {
                        warn!("Codex 配置智能守护收到文件事件错误: {err}");
                    }
                }
                restore_if_needed(&options, "config_guard_restore");
            }
            else => break,
        }
    }

    drop(watcher);
    info!("Codex 配置智能守护已停止");
}

fn guard_enabled(options: &CodexConfigGuardOptions) -> bool {
    options.codex_config_guard && (options.codex_auto_inject || options.codex_persistent_inject)
}

fn config_event_relevant(paths: &[PathBuf], config_path: &Path) -> bool {
    if paths.is_empty() {
        return true;
    }
    paths.iter().any(|path| {
        path == config_path
            || path.file_name() == config_path.file_name()
            || path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == "config.toml")
    })
}

fn current_policy(options: &CodexConfigGuardOptions) -> GuardPolicy {
    let mut enabled = guard_enabled(options);
    let mut codex_router_mode = options.codex_router_mode.clone();
    let config_path = Args::default_config_path(&options.data_dir);
    if let Some(args) = Args::load_from_file(&config_path) {
        enabled =
            args.codex_config_guard && (args.codex_auto_inject || args.codex_persistent_inject);
        codex_router_mode = config::normalize_codex_router_mode(&args.codex_router_mode);
    }
    GuardPolicy {
        enabled,
        codex_router_mode,
    }
}

fn restore_if_needed(options: &CodexConfigGuardOptions, reason: &'static str) {
    let policy = current_policy(options);
    if !policy.enabled {
        debug!(reason, "跳过 Codex 配置智能守护: 当前配置已关闭");
        return;
    }

    let reasons = codex_config::codex_integration_restore_reasons(CodexIntegrationCheckOptions {
        host: &options.host,
        port: options.port,
        codex_router_mode: &policy.codex_router_mode,
    });
    if reasons.is_empty() {
        return;
    }

    info!(
        reason,
        mode = %policy.codex_router_mode,
        restore_reasons = ?reasons,
        "检测到 Codex 配置被外部改动，开始恢复 DEX 管理字段"
    );
    codex_config::fix();
    codex_config::sync_codex_integration(CodexIntegrationSyncOptions {
        host: &options.host,
        port: options.port,
        context_window_override: options.context_window_override,
        data_dir: Some(&options.data_dir),
        codex_router_mode: &policy.codex_router_mode,
        reason,
    });
}
