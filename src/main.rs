mod accounts;
mod backup_store;
mod cache;
mod cdp;
mod codex_config;
mod codex_threads;
mod config;
mod executor;
mod files;
mod handlers;
mod inject;
mod metrics;
mod prompts;
mod ratelimit;
mod session;
mod sse;
mod stream;
mod token_anomaly;
mod translate;
mod types;
mod utils;
mod validate;
mod vector_stores;

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use config::{Args, Commands};

struct FlushWriter<W: Write>(W);

impl<W: Write> Write for FlushWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.0.write(buf)?;
        self.0.flush()?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

// ── .env 加载（覆盖模式） ────────────────────────────────────────────────────

/// 手动解析 .env 并用 `std::env::set_var` 强行覆盖已有环境变量。
/// 解决 daemon 重启时父进程继承的旧值被 `dotenvy` 跳过的问题。
fn load_env_override() {
    let env_path = find_env_file();
    let path = match env_path {
        Some(p) => p,
        None => return,
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim();
            let val = trimmed[eq + 1..].trim();
            // 去掉引号包裹
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

fn find_env_file() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    // cwd
    if std::path::Path::new(".env").exists() {
        return Some(PathBuf::from(".env"));
    }
    // ~/.deecodex/.env
    if let Some(home) = crate::config::home_dir() {
        let home_env = home.join(".deecodex").join(".env");
        if home_env.exists() {
            return Some(home_env);
        }
    }
    // exe目录
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

// ── Service helpers ─────────────────────────────────────────────────────────

fn pid_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("deecodex.pid")
}

fn log_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("deecodex.log")
}

fn read_pid(data_dir: &std::path::Path) -> Option<u32> {
    let content = std::fs::read_to_string(pid_path(data_dir)).ok()?;
    content.trim().parse().ok()
}

fn is_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 通过进程名查找 deecodex daemon 的 PID（pgrep 回退）
fn find_daemon_by_name() -> Option<u32> {
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg("deecodex.*--daemon")
        .output()
        .ok()?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.lines().next()?.trim().parse().ok()
    } else {
        None
    }
}

fn stop_service(
    data_dir: &std::path::Path,
    codex_auto_inject: bool,
    codex_persistent_inject: bool,
) -> Result<()> {
    let pid = match read_pid(data_dir) {
        Some(pid) if is_running(pid) => pid,
        _ => {
            // PID 文件丢失或过期，通过进程名查找
            if let Some(pid) = find_daemon_by_name() {
                pid
            } else {
                let _ = std::fs::remove_file(pid_path(data_dir));
                bail!("未找到运行中的 deecodex daemon");
            }
        }
    };

    if !is_running(pid) {
        let _ = std::fs::remove_file(pid_path(data_dir));
        let migration_active = data_dir.join("thread_migration_backup.json").exists();
        if codex_auto_inject && !codex_persistent_inject && !migration_active {
            codex_config::remove();
        }
        bail!("PID {} 对应的进程已不存在，已清理 PID 文件", pid);
    }

    info!("正在停止 deecodex (PID: {})...", pid);
    std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()?;

    // 等待进程退出（最多 5 秒）
    for _ in 0..50 {
        if !is_running(pid) {
            let _ = std::fs::remove_file(pid_path(data_dir));
            if codex_auto_inject && !codex_persistent_inject {
                codex_config::remove();
            }
            println!("deecodex 已停止 (PID: {})", pid);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // 强制终止
    std::process::Command::new("kill")
        .arg("-9")
        .arg(pid.to_string())
        .status()?;
    let _ = std::fs::remove_file(pid_path(data_dir));
    if codex_auto_inject && !codex_persistent_inject {
        codex_config::remove();
    }
    println!("deecodex 已强制停止 (PID: {})", pid);
    Ok(())
}

fn start_service_daemon(args: &Args) -> Result<()> {
    // 检查是否已在运行（PID 文件 + pgrep 回退）
    if let Some(pid) = read_pid(&args.data_dir) {
        if is_running(pid) {
            bail!("deecodex 已在运行 (PID: {})", pid);
        }
        let _ = std::fs::remove_file(pid_path(&args.data_dir));
    }
    if let Some(pid) = find_daemon_by_name() {
        bail!("deecodex 已在运行 (PID: {}，通过进程名检测)", pid);
    }

    std::fs::create_dir_all(&args.data_dir)?;

    // 保存当前配置，确保 daemon 进程能读取到完整参数
    let config_path = config::Args::default_config_path(&args.data_dir);
    let _ = args.save_to_file(&config_path);

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon");

    // 透传当前参数（排除子命令）
    let current_args: Vec<String> = std::env::args().collect();
    for arg in &current_args[1..] {
        if arg != "start" && arg != "restart" {
            cmd.arg(arg);
        }
    }
    // config.json 已在上方保存，daemon 进程会自行加载

    let child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    let pid = child.id();
    std::fs::write(pid_path(&args.data_dir), pid.to_string())?;
    println!(
        "deecodex 已启动 (PID: {}, 日志: {})",
        pid,
        log_path(&args.data_dir).display()
    );
    Ok(())
}

// ── Main ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // 加载 .env 文件（手动解析以覆盖已有环境变量，解决 daemon 重启继承旧值的问题）
    load_env_override();

    // 向后兼容: CODEX_RELAY_* → DEECODEX_*
    for (old, new) in [
        ("CODEX_RELAY_PORT", "DEECODEX_PORT"),
        ("CODEX_RELAY_UPSTREAM", "DEECODEX_UPSTREAM"),
        ("CODEX_RELAY_API_KEY", "DEECODEX_API_KEY"),
        ("CODEX_RELAY_CLIENT_API_KEY", "DEECODEX_CLIENT_API_KEY"),
        ("CODEX_RELAY_MODEL_MAP", "DEECODEX_MODEL_MAP"),
        ("CODEX_RELAY_MAX_BODY_MB", "DEECODEX_MAX_BODY_MB"),
        ("CODEX_RELAY_VISION_UPSTREAM", "DEECODEX_VISION_UPSTREAM"),
        ("CODEX_RELAY_VISION_API_KEY", "DEECODEX_VISION_API_KEY"),
        ("CODEX_RELAY_VISION_MODEL", "DEECODEX_VISION_MODEL"),
        ("CODEX_RELAY_VISION_ENDPOINT", "DEECODEX_VISION_ENDPOINT"),
        ("CODEX_RELAY_CHINESE_THINKING", "DEECODEX_CHINESE_THINKING"),
        ("CODEX_RELAY_PROMPTS_DIR", "DEECODEX_PROMPTS_DIR"),
        ("CODEX_RELAY_DATA_DIR", "DEECODEX_DATA_DIR"),
        (
            "CODEX_RELAY_TOKEN_ANOMALY_PROMPT_MAX",
            "DEECODEX_TOKEN_ANOMALY_PROMPT_MAX",
        ),
        (
            "CODEX_RELAY_TOKEN_ANOMALY_SPIKE_RATIO",
            "DEECODEX_TOKEN_ANOMALY_SPIKE_RATIO",
        ),
        (
            "CODEX_RELAY_TOKEN_ANOMALY_BURN_WINDOW",
            "DEECODEX_TOKEN_ANOMALY_BURN_WINDOW",
        ),
        (
            "CODEX_RELAY_TOKEN_ANOMALY_BURN_RATE",
            "DEECODEX_TOKEN_ANOMALY_BURN_RATE",
        ),
        (
            "CODEX_RELAY_ALLOWED_MCP_SERVERS",
            "DEECODEX_ALLOWED_MCP_SERVERS",
        ),
        (
            "CODEX_RELAY_ALLOWED_COMPUTER_DISPLAYS",
            "DEECODEX_ALLOWED_COMPUTER_DISPLAYS",
        ),
    ] {
        if std::env::var(new).is_err() {
            if let Ok(val) = std::env::var(old) {
                if !val.is_empty() {
                    std::env::set_var(new, val);
                }
            }
        }
    }

    let mut args = Args::parse();

    // 将相对 data_dir/prompts_dir 解析为绝对路径
    if args.data_dir.is_relative() {
        if let Some(home) = crate::config::home_dir() {
            args.data_dir = home.join(&args.data_dir);
        }
    }
    if args.prompts_dir.is_relative() {
        if let Some(home) = crate::config::home_dir() {
            args.prompts_dir = home.join(&args.prompts_dir);
        }
    }

    // ── 服务管理子命令（在 tracing 初始化之前处理，不需要日志系统） ──
    match &args.command {
        Some(Commands::Start) => {
            start_service_daemon(&args)?;
            return Ok(());
        }
        Some(Commands::Stop) => {
            stop_service(
                &args.data_dir,
                args.codex_auto_inject,
                args.codex_persistent_inject,
            )?;
            return Ok(());
        }
        Some(Commands::Restart) => {
            // 尝试停止已运行的服务（PID 文件 + pgrep，忽略错误）
            let running =
                read_pid(&args.data_dir).is_some_and(is_running) || find_daemon_by_name().is_some();
            if running {
                let _ = stop_service(
                    &args.data_dir,
                    args.codex_auto_inject,
                    args.codex_persistent_inject,
                );
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            start_service_daemon(&args)?;
            return Ok(());
        }
        Some(Commands::Status) => {
            // PID 文件 + pgrep 回退
            let pid = read_pid(&args.data_dir)
                .filter(|&p| is_running(p))
                .or_else(find_daemon_by_name);
            match pid {
                Some(pid) => {
                    println!("deecodex 正在运行 (PID: {})", pid);
                    println!("数据目录: {}", args.data_dir.display());
                    println!("日志文件: {}", log_path(&args.data_dir).display());
                }
                None => {
                    println!("deecodex 未运行");
                }
            }
            return Ok(());
        }
        Some(Commands::Logs) => {
            let log = log_path(&args.data_dir);
            if !log.exists() {
                bail!("日志文件不存在: {}", log.display());
            }
            // tail -f 日志文件
            let status = std::process::Command::new("tail")
                .arg("-f")
                .arg(&log)
                .status()?;
            if !status.success() {
                bail!("tail 命令退出");
            }
            return Ok(());
        }
        Some(Commands::FixConfig) => {
            let fixed = codex_config::fix();
            if fixed > 0 {
                println!("已修复 Codex config.toml 中的 {} 处已知问题", fixed);
            } else {
                println!("Codex config.toml 未发现已知问题");
            }
            return Ok(());
        }
        _ => {}
    }

    // ── 初始化 tracing（daemon 模式写文件，否则写 stderr） ──
    if args.daemon {
        std::fs::create_dir_all(&args.data_dir)?;
        let log_path = log_path(&args.data_dir);
        let mut log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        // 新日志文件写入 UTF-8 BOM，确保 Windows 工具能正确识别编码
        if log_file.metadata().map(|m| m.len() == 0).unwrap_or(true) {
            use std::io::Write;
            let _ = log_file.write_all(&[0xEF, 0xBB, 0xBF]);
        }
        tracing_subscriber::fmt()
            .with_writer(move || FlushWriter(log_file.try_clone().expect("failed to clone log fd")))
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "deecodex=info".into()),
            )
            .init();
        // 写入 PID 文件
        let pid = std::process::id();
        std::fs::write(pid_path(&args.data_dir), pid.to_string())?;
    } else {
        tracing_subscriber::fmt()
            .with_writer(|| FlushWriter(std::io::stderr()))
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "deecodex=info".into()),
            )
            .init();
    }

    // 从 config.json 加载持久化配置并合并
    let args = {
        let merged = args.merge_with_file();
        // 启动时自动保存配置（确保 config.json 与当前 CLI/env 一致）
        let config_path = config::Args::default_config_path(&merged.data_dir);
        let _ = merged.save_to_file(&config_path);
        merged
    };

    // 启动前配置诊断
    for diag in validate::validate(&args) {
        match diag.severity {
            validate::Severity::Error => tracing::error!(
                category = diag.category,
                "配置诊断 [错误]: {}",
                diag.message
            ),
            validate::Severity::Warn => tracing::warn!(
                category = diag.category,
                "配置诊断 [警告]: {}",
                diag.message
            ),
        }
    }

    let model_map: HashMap<String, String> = if args.model_map.is_empty() {
        HashMap::new()
    } else {
        match serde_json::from_str(&args.model_map) {
            Ok(m) => m,
            Err(e) => {
                error!("Failed to parse DEECODEX_MODEL_MAP: {e}");
                HashMap::new()
            }
        }
    };

    info!("model map: {} entries", model_map.len());

    let upstream = handlers::validate_upstream(&args.upstream)?;

    let vision_upstream = if args.vision_upstream.is_empty() {
        None
    } else {
        Some(handlers::validate_upstream(&args.vision_upstream)?)
    };
    if vision_upstream.is_some() {
        info!("vision upstream configured: {}", args.vision_upstream);
    }

    let files = crate::files::FileStore::with_data_dir(&args.data_dir)?;
    let vector_stores = crate::vector_stores::VectorStoreRegistry::with_data_dir(&args.data_dir)?;
    let executors = crate::executor::LocalExecutorConfig::from_raw(
        &args.computer_executor,
        args.computer_executor_timeout_secs,
        &args.mcp_executor_config,
        args.mcp_executor_timeout_secs,
    )?;

    use crate::accounts::{generate_id, now_secs, Account, AccountStore};
    let default_account = Account {
        id: generate_id(),
        name: "默认账号".into(),
        provider: crate::accounts::guess_provider(&args.upstream).into(),
        upstream: args.upstream.clone(),
        api_key: args.api_key.clone(),
        model_map: model_map.clone(),
        vision_upstream: args.vision_upstream.clone(),
        vision_api_key: args.vision_api_key.clone(),
        vision_model: args.vision_model.clone(),
        vision_endpoint: args.vision_endpoint.clone(),
        from_codex_config: false,
        balance_url: String::new(),
        created_at: now_secs(),
        updated_at: now_secs(),
        context_window_override: None,
    };

    let state = handlers::AppState {
        sessions: crate::session::SessionStore::new(),
        client: Client::builder()
            .pool_idle_timeout(None)
            .pool_max_idle_per_host(4)
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
        upstream: Arc::new(tokio::sync::RwLock::new(upstream)),
        api_key: Arc::new(tokio::sync::RwLock::new(args.api_key.clone())),
        client_api_key: Arc::new(tokio::sync::RwLock::new(args.client_api_key)),
        model_map: Arc::new(tokio::sync::RwLock::new(model_map.clone())),
        vision_upstream: Arc::new(tokio::sync::RwLock::new(vision_upstream)),
        vision_api_key: Arc::new(tokio::sync::RwLock::new(args.vision_api_key.clone())),
        vision_model: Arc::new(tokio::sync::RwLock::new(args.vision_model.clone())),
        vision_endpoint: Arc::new(tokio::sync::RwLock::new(args.vision_endpoint.clone())),
        start_time: std::time::Instant::now(),
        request_cache: crate::cache::RequestCache::default(),
        prompts: Arc::new(crate::prompts::PromptRegistry::new(&args.prompts_dir)),
        files,
        vector_stores,
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: args.chinese_thinking,
        codex_auto_inject: args.codex_auto_inject,
        codex_persistent_inject: args.codex_persistent_inject,
        codex_launch_with_cdp: args.codex_launch_with_cdp,
        cdp_port: args.cdp_port,
        port: args.port,
        metrics: Arc::new(metrics::Metrics::new()),
        token_tracker: Arc::new(crate::token_anomaly::TokenTracker::new(
            32,
            args.token_anomaly_prompt_max,
            args.token_anomaly_spike_ratio,
            args.token_anomaly_burn_window,
            args.token_anomaly_burn_rate,
        )),
        tool_policy: Arc::new(tokio::sync::RwLock::new(handlers::ToolPolicy {
            allowed_mcp_servers: parse_csv_list(&args.allowed_mcp_servers),
            allowed_computer_displays: parse_csv_list(&args.allowed_computer_displays),
        })),
        executors: Arc::new(tokio::sync::RwLock::new(executors)),
        data_dir: Arc::new(args.data_dir.clone()),
        account_store: Arc::new(tokio::sync::RwLock::new(AccountStore {
            accounts: vec![default_account.clone()],
            active_id: Some(default_account.id.clone()),
        })),
        active_account: Arc::new(tokio::sync::RwLock::new(default_account)),
        rate_limiter: {
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
                info!("rate limiter: {} req per {}s", rate_limit, rate_window);
                Some(Arc::new(ratelimit::RateLimiter::new(
                    rate_limit,
                    rate_window,
                )))
            } else {
                info!("rate limiter: disabled");
                None
            }
        },
    };
    info!("local prompts registry: {}", args.prompts_dir.display());
    info!("local data directory: {}", args.data_dir.display());
    if args.token_anomaly_prompt_max > 0 {
        info!(
            "token anomaly detection: prompt_max={} spike_ratio={}x burn_window={}s burn_rate={}/min",
            args.token_anomaly_prompt_max,
            args.token_anomaly_spike_ratio,
            args.token_anomaly_burn_window,
            args.token_anomaly_burn_rate
        );
    } else {
        info!("token anomaly detection: disabled (prompt_max=0)");
    }
    if args.chinese_thinking {
        info!("chinese thinking mode: enabled (system prompt will include Chinese instruction)");
    }
    let tp = state.tool_policy.read().await;
    if !tp.allowed_mcp_servers.is_empty() {
        info!(
            "MCP tool policy: {} allowed server(s)",
            tp.allowed_mcp_servers.len()
        );
    }
    if !tp.allowed_computer_displays.is_empty() {
        info!(
            "computer tool policy: {} allowed display(s)",
            tp.allowed_computer_displays.len()
        );
    }
    drop(tp);
    let startup_exec = state.executors.read().await.clone();
    info!(
        "computer executor: {} (timeout={}s)",
        startup_exec.computer.backend.as_str(),
        startup_exec.computer.timeout_secs
    );
    if startup_exec.computer.enabled() {
        info!("computer executor is enabled behind local tool policy");
    }
    if startup_exec.mcp.enabled() {
        info!(
            "MCP executor: {} configured server(s), timeout={}s",
            startup_exec.mcp.servers.len(),
            startup_exec.mcp.timeout_secs
        );
        if let Some(label) = startup_exec.mcp.servers.keys().next() {
            debug!(
                "first configured MCP executor server: {}",
                startup_exec
                    .mcp
                    .get_server(label)
                    .map(|server| server.label.as_str())
                    .unwrap_or(label)
            );
        }
    } else {
        info!("MCP executor: disabled (no configured servers)");
    }
    if state.client_api_key.read().await.is_empty() {
        tracing::warn!("client auth disabled: client_api_key is empty");
    } else {
        info!("client auth enabled for /v1 API routes");
    }

    let max_bytes = args.max_body_mb * 1024 * 1024;
    let body_limit = axum::extract::DefaultBodyLimit::max(max_bytes);

    let app = handlers::build_router(state.clone()).layer(body_limit);

    let addr = format!("127.0.0.1:{}", args.port);
    info!(
        "listening {} -> {} | body:{}MB",
        addr,
        state.upstream.read().await.as_ref(),
        args.max_body_mb
    );

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // 注入 deecodex 配置到 codex 的 config.toml
    if args.codex_auto_inject && !args.codex_persistent_inject {
        codex_config::fix();
        codex_config::inject(args.port, &state.client_api_key.read().await, None);
    }

    // 如果配置了自动启动 Codex，spawn Codex.app 带 CDP 调试端口
    if args.codex_launch_with_cdp {
        let cdp_port = args.cdp_port;
        tokio::spawn(async move {
            let result = tokio::process::Command::new("open")
                .arg("-a")
                .arg("Codex.app")
                .arg("--args")
                .arg(format!("--remote-debugging-port={cdp_port}"))
                .spawn();
            match result {
                Ok(_) => info!("已启动 Codex 桌面版 (CDP 端口 {cdp_port})"),
                Err(e) => warn!("启动 Codex 桌面版失败: {e}"),
            }
        });
    }

    // 尝试 CDP 注入（插件解锁 + 会话删除 UI），异步执行，不阻塞服务启动
    let inject_state = state.clone();
    let cdp_port = args.cdp_port;
    tokio::spawn(async move {
        inject::try_inject_with_port(Arc::new(inject_state), cdp_port).await;
    });

    #[cfg(unix)]
    async fn shutdown_signal() {
        let ctrl_c = tokio::signal::ctrl_c();
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => { info!("SIGINT received, graceful shutdown..."); }
            _ = term.recv() => { info!("SIGTERM received, graceful shutdown..."); }
        }
    }

    #[cfg(windows)]
    async fn shutdown_signal() {
        let ctrl_c = tokio::signal::ctrl_c();
        let _ = ctrl_c.await;
        info!("Ctrl+C received, graceful shutdown...");
    }

    info!("服务已启动");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // 清理 codex config.toml 中的 deecodex 配置
    // 线程聚合迁移激活时不删除，否则 Codex 重启后找不到 provider 会隐藏所有会话
    let migration_active = args.data_dir.join("thread_migration_backup.json").exists();
    if args.codex_auto_inject && !args.codex_persistent_inject && !migration_active {
        codex_config::remove();
    }

    // 清理 PID 文件
    let _ = std::fs::remove_file(pid_path(&args.data_dir));

    Ok(())
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_csv_list_trims_and_drops_empty_items() {
        assert_eq!(
            parse_csv_list(" filesystem, ,github,, browser "),
            vec!["filesystem", "github", "browser"]
        );
    }

    #[test]
    fn pid_and_log_paths_live_under_data_dir() {
        let data_dir = std::path::Path::new("/tmp/deecodex-test");

        assert_eq!(pid_path(data_dir), data_dir.join("deecodex.pid"));
        assert_eq!(log_path(data_dir), data_dir.join("deecodex.log"));
    }
}
