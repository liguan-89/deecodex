//! 执行诊断模块。
//!
//! 提供两层诊断能力：
//! 1. `run_diagnostics()` — 全链路运行时诊断（15 项检查），供 GUI/CLI 使用
//! 2. `validate()` — 启动前静态配置诊断（向后兼容），供 main.rs 启动日志使用

use std::path::{Path, PathBuf};

use crate::accounts;
use crate::codex_config;
use crate::config::{self, Args};

// ── 新诊断类型 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupHealth {
    Healthy,
    Degraded,
    Broken,
}

/// 单项诊断结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticItem {
    pub status: Status,
    pub check_name: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// 分组诊断结果
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticGroup {
    pub name: String,
    pub health: GroupHealth,
    pub items: Vec<DiagnosticItem>,
}

/// 诊断摘要
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticSummary {
    pub total: usize,
    pub pass: usize,
    pub warn: usize,
    pub fail: usize,
    pub info: usize,
    pub health: GroupHealth,
}

/// 完整诊断报告
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticReport {
    pub summary: DiagnosticSummary,
    pub groups: Vec<DiagnosticGroup>,
    pub version: String,
}

/// 诊断上下文——诊断所需的所有输入
#[derive(Debug, Clone)]
pub struct DiagnosticContext {
    pub data_dir: PathBuf,
    pub port: u16,
    pub upstream: String,
    pub api_key: String,
    pub client_api_key: String,
    pub model_map: String,
    pub codex_auto_inject: bool,
    pub codex_persistent_inject: bool,
    pub codex_launch_with_cdp: bool,
    pub cdp_port: u16,
}

impl From<&Args> for DiagnosticContext {
    fn from(args: &Args) -> Self {
        Self {
            data_dir: args.data_dir.clone(),
            port: args.port,
            upstream: args.upstream.clone(),
            api_key: args.api_key.clone(),
            client_api_key: args.client_api_key.clone(),
            model_map: args.model_map.clone(),
            codex_auto_inject: args.codex_auto_inject,
            codex_persistent_inject: args.codex_persistent_inject,
            codex_launch_with_cdp: args.codex_launch_with_cdp,
            cdp_port: args.cdp_port,
        }
    }
}

// ── DiagnosticReport 构造 ─────────────────────────────────────────────────────

impl DiagnosticReport {
    pub fn new(groups: Vec<DiagnosticGroup>) -> Self {
        let summary = Self::compute_summary(&groups);
        Self {
            summary,
            groups,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn compute_summary(groups: &[DiagnosticGroup]) -> DiagnosticSummary {
        let mut total = 0;
        let mut pass = 0;
        let mut warn = 0;
        let mut fail = 0;
        let mut info = 0;

        for g in groups {
            for item in &g.items {
                total += 1;
                match item.status {
                    Status::Pass => pass += 1,
                    Status::Warn => warn += 1,
                    Status::Fail => fail += 1,
                    Status::Info => info += 1,
                }
            }
        }

        let health = if fail > 0 {
            GroupHealth::Broken
        } else if warn > 0 {
            GroupHealth::Degraded
        } else {
            GroupHealth::Healthy
        };

        DiagnosticSummary {
            total,
            pass,
            warn,
            fail,
            info,
            health,
        }
    }

    pub fn compute_group_health(items: &[DiagnosticItem]) -> GroupHealth {
        if items.iter().any(|i| i.status == Status::Fail) {
            GroupHealth::Broken
        } else if items.iter().any(|i| i.status == Status::Warn) {
            GroupHealth::Degraded
        } else {
            GroupHealth::Healthy
        }
    }
}

// ── 诊断入口 ──────────────────────────────────────────────────────────────────

/// 同步诊断（不含网络连通性检测，该项标记为 Info 提示需异步检测）
pub fn run_diagnostics_sync(ctx: &DiagnosticContext) -> DiagnosticReport {
    DiagnosticReport::new(vec![
        DiagnosticGroup {
            name: "服务状态".into(),
            items: vec![
                check_service_running(ctx),
                check_port_conflict(ctx),
            ],
            health: GroupHealth::Healthy, // 下面会重新计算
        },
        DiagnosticGroup {
            name: "账号连通".into(),
            items: vec![
                check_deecodex_config(ctx),
                check_accounts_config(ctx),
                check_model_mapping(ctx),
                check_upstream_connectivity_sync(ctx),
            ],
            health: GroupHealth::Healthy,
        },
        DiagnosticGroup {
            name: "Codex 路由".into(),
            items: vec![
                check_codex_installed(),
                check_codex_third_party_routing(ctx),
                check_codex_deecodex_routing(ctx),
                check_codex_config_consistency(ctx),
                check_codex_startup_order(ctx),
                check_config_backups(ctx),
            ],
            health: GroupHealth::Healthy,
        },
        DiagnosticGroup {
            name: "注入状态".into(),
            items: vec![
                check_injection_status(ctx),
                check_models_cache(),
            ],
            health: GroupHealth::Healthy,
        },
        DiagnosticGroup {
            name: "运行环境".into(),
            items: vec![
                check_disk_space(ctx),
            ],
            health: GroupHealth::Healthy,
        },
    ])
    .with_computed_health()
}

impl DiagnosticReport {
    fn with_computed_health(mut self) -> Self {
        for group in &mut self.groups {
            group.health = Self::compute_group_health(&group.items);
        }
        self.summary = Self::compute_summary(&self.groups);
        self
    }
}

// ── 1. 检查服务是否运行 ──────────────────────────────────────────────────────

fn check_service_running(ctx: &DiagnosticContext) -> DiagnosticItem {
    let pid_path = ctx.data_dir.join("deecodex.pid");

    let pid_from_file = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());

    let pid_running = pid_from_file
        .filter(|&p| process_is_running(p))
        .or_else(|| find_daemon_pid());

    match pid_running {
        Some(pid) => DiagnosticItem {
            status: Status::Pass,
            check_name: "服务运行状态".into(),
            message: format!("deecodex 守护进程正在运行 (PID: {})", pid),
            detail: Some(format!("端口: {}, PID 文件: {}", ctx.port, pid_path.display())),
            suggestion: None,
        },
        None => {
            // 检查是否端口正在监听（可能进程名不匹配）
            if port_is_listening(ctx.port) {
                DiagnosticItem {
                    status: Status::Warn,
                    check_name: "服务运行状态".into(),
                    message: format!("端口 {} 正在被占用但未检测到 deecodex 进程", ctx.port),
                    detail: Some("端口处于监听状态但进程 ID 不可识别".into()),
                    suggestion: Some("请检查是否有其他程序占用了 deecodex 端口，或使用 deecodex start 启动服务".into()),
                }
            } else {
                DiagnosticItem {
                    status: Status::Fail,
                    check_name: "服务运行状态".into(),
                    message: "deecodex 服务未运行".into(),
                    detail: None,
                    suggestion: Some("请运行 deecodex start 或在控制面板中启动服务".into()),
                }
            }
        }
    }
}

fn process_is_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn find_daemon_pid() -> Option<u32> {
    let output = std::process::Command::new("pgrep")
        .arg("-f")
        .arg("deecodex.*--daemon")
        .output()
        .ok()?;
    if output.status.success() {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()
    } else {
        None
    }
}

fn port_is_listening(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

// ── 2. 账号模型连通性（同步占位） ────────────────────────────────────────────

fn check_upstream_connectivity_sync(_: &DiagnosticContext) -> DiagnosticItem {
    DiagnosticItem {
        status: Status::Info,
        check_name: "账号连通性".into(),
        message: "跳过网络检测（同步模式不支持）".into(),
        detail: None,
        suggestion: Some("请使用完整异步诊断以检测上游 API 连通性".into()),
    }
}

/// 异步连通性检测结果，由调用方在 GUI/CLI 中异步获取后回填。
pub fn connectivity_check_result(
    ok: bool,
    status_code: u16,
    latency_ms: u128,
    model_count: Option<usize>,
    endpoint: &str,
    error: Option<&str>,
) -> DiagnosticItem {
    if ok {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "账号连通性".into(),
            message: format!("上游 API 连通正常 (延迟 {}ms)", latency_ms),
            detail: Some(format!(
                "端点: {}, HTTP {}, 可用模型数: {}",
                endpoint,
                status_code,
                model_count.map_or("未知".into(), |c| c.to_string())
            )),
            suggestion: None,
        }
    } else {
        DiagnosticItem {
            status: Status::Fail,
            check_name: "账号连通性".into(),
            message: format!("无法连接上游 API ({}ms)", latency_ms),
            detail: Some(format!(
                "端点: {}, 错误: {}",
                endpoint,
                error.unwrap_or("未知错误")
            )),
            suggestion: Some("请检查网络连接、代理设置，或切换其他可用账号".into()),
        }
    }
}

// ── 3. Codex 第三方路由检测 ──────────────────────────────────────────────────

fn check_codex_third_party_routing(ctx: &DiagnosticContext) -> DiagnosticItem {
    let Some(config_path) = codex_config::codex_config_path() else {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "第三方路由检测".into(),
            message: "无法确定 Codex 配置路径".into(),
            detail: None,
            suggestion: None,
        };
    };

    if !config_path.exists() {
        return DiagnosticItem {
            status: Status::Pass,
            check_name: "第三方路由检测".into(),
            message: "未发现 Codex 配置文件，不存在第三方路由".into(),
            detail: None,
            suggestion: None,
        };
    }

    let content = match codex_config::read_config_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Warn,
                check_name: "第三方路由检测".into(),
                message: "无法读取 Codex 配置文件".into(),
                detail: Some(format!("路径: {}, 错误: {}", config_path.display(), e)),
                suggestion: None,
            };
        }
    };

    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Warn,
                check_name: "第三方路由检测".into(),
                message: "Codex 配置文件解析失败".into(),
                detail: Some(format!("{}", e)),
                suggestion: None,
            };
        }
    };

    let mut third_parties = Vec::new();
    let deecodex_base = format!("http://127.0.0.1:{}/v1", ctx.port);

    if let Some(providers) = doc.get("model_providers").and_then(|mp| mp.as_table()) {
        for (key, value) in providers.iter() {
            if key == "deecodex" {
                continue;
            }
            let base_url = value
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // 跳过本地 deecodex 地址
            if base_url.contains("127.0.0.1") || base_url.contains("localhost") {
                if base_url == deecodex_base {
                    continue;
                }
            }

            let is_local = base_url.contains("127.0.0.1") || base_url.contains("localhost");
            third_parties.push((key.to_string(), base_url, is_local));
        }
    }

    if third_parties.is_empty() {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "第三方路由检测".into(),
            message: "Codex 配置中未发现第三方路由".into(),
            detail: None,
            suggestion: None,
        }
    } else {
        let names: Vec<String> = third_parties
            .iter()
            .map(|(name, url, _)| format!("{} ({})", name, url))
            .collect();
        DiagnosticItem {
            status: Status::Fail,
            check_name: "第三方路由检测".into(),
            message: format!(
                "Codex 配置中存在 {} 个第三方路由，与 deecodex 代理冲突",
                third_parties.len()
            ),
            detail: Some(names.join("; ")),
            suggestion: Some("请关闭第三方路由工具，确保 Codex 仅通过 deecodex 代理访问上游 API".into()),
        }
    }
}

// ── 4. Codex 路由到 deecodex 检测 ────────────────────────────────────────────

fn check_codex_deecodex_routing(ctx: &DiagnosticContext) -> DiagnosticItem {
    let Some(config_path) = codex_config::codex_config_path() else {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "deecodex 路由".into(),
            message: "无法确定 Codex 配置路径".into(),
            detail: None,
            suggestion: None,
        };
    };

    if !config_path.exists() {
        return DiagnosticItem {
            status: Status::Warn,
            check_name: "deecodex 路由".into(),
            message: "Codex 配置文件不存在".into(),
            detail: None,
            suggestion: Some("请先启动一次 Codex 以生成配置文件，或手动创建 config.toml".into()),
        };
    }

    let content = match codex_config::read_config_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Warn,
                check_name: "deecodex 路由".into(),
                message: "无法读取 Codex 配置文件".into(),
                detail: Some(e.to_string()),
                suggestion: None,
            };
        }
    };

    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Warn,
                check_name: "deecodex 路由".into(),
                message: "Codex 配置文件解析失败".into(),
                detail: Some(e.to_string()),
                suggestion: None,
            };
        }
    };

    let model_provider = doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("(未设置)");
    let expected_base = format!("http://127.0.0.1:{}/v1", ctx.port);

    let has_deecodex_section = doc
        .get("model_providers")
        .and_then(|mp| mp.get("deecodex"))
        .is_some();

    let actual_base = doc
        .get("model_providers")
        .and_then(|mp| mp.get("deecodex"))
        .and_then(|d| d.get("base_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("(未设置)");

    if model_provider != "deecodex" {
        DiagnosticItem {
            status: Status::Fail,
            check_name: "deecodex 路由".into(),
            message: format!("Codex 未路由到 deecodex，当前 provider 为: {}", model_provider),
            detail: None,
            suggestion: Some("请在 deecodex 控制面板中点击「注入配置」，将 Codex 路由至 deecodex".into()),
        }
    } else if !has_deecodex_section {
        DiagnosticItem {
            status: Status::Fail,
            check_name: "deecodex 路由".into(),
            message: "model_provider=deecodex 但缺少 [model_providers.deecodex] 配置节".into(),
            detail: None,
            suggestion: Some("请在 deecodex 控制面板中点击「注入配置」以修复".into()),
        }
    } else if actual_base != expected_base {
        DiagnosticItem {
            status: Status::Warn,
            check_name: "deecodex 路由".into(),
            message: "deecodex 路由已配置但端口不匹配".into(),
            detail: Some(format!(
                "期望: {}, 实际: {}",
                expected_base, actual_base
            )),
            suggestion: Some("请在 deecodex 控制面板中点击「注入配置」以更新端口".into()),
        }
    } else {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "deecodex 路由".into(),
            message: "Codex 已正确路由到 deecodex".into(),
            detail: Some(format!("base_url: {}", actual_base)),
            suggestion: None,
        }
    }
}

// ── 5. GPT 预设模型映射缺失检查 ──────────────────────────────────────────────

/// 常见模型名称（Codex 侧）
const COMMON_MODELS: &[&str] = &[
    "gpt-4o",
    "gpt-4o-mini",
    "gpt-4.1",
    "gpt-5",
    "o3",
    "o4-mini",
    "o3-mini",
    "claude-sonnet-4-5",
    "claude-opus-4-5",
    "claude-haiku-4-5",
    "deepseek-chat",
    "deepseek-reasoner",
    "gemini-2.5-pro",
    "gemini-2.5-flash",
];

fn check_model_mapping(ctx: &DiagnosticContext) -> DiagnosticItem {
    let raw = ctx.model_map.trim();
    if raw.is_empty() || raw == "{}" {
        return DiagnosticItem {
            status: Status::Warn,
            check_name: "模型映射".into(),
            message: "模型映射为空，Codex 请求的模型名无法转换".into(),
            detail: None,
            suggestion: Some("请在「账号管理 → 模型映射」中配置模型对应关系".into()),
        };
    }

    let model_map: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Fail,
                check_name: "模型映射".into(),
                message: "模型映射 JSON 解析失败".into(),
                detail: Some(e.to_string()),
                suggestion: Some("请检查模型映射的 JSON 格式".into()),
            };
        }
    };

    let map_obj = match model_map.as_object() {
        Some(o) => o,
        None => {
            return DiagnosticItem {
                status: Status::Fail,
                check_name: "模型映射".into(),
                message: "模型映射不是有效的 JSON 对象".into(),
                detail: None,
                suggestion: Some("请使用正确的 JSON 对象格式配置模型映射".into()),
            };
        }
    };

    let mut missing: Vec<&str> = Vec::new();
    for model in COMMON_MODELS {
        if !map_obj.contains_key(*model) {
            missing.push(model);
        }
    }

    if missing.is_empty() {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "模型映射".into(),
            message: format!("模型映射完整（已覆盖 {} 个常见模型）", COMMON_MODELS.len()),
            detail: Some(format!("映射条目总数: {}", map_obj.len())),
            suggestion: None,
        }
    } else {
        let names: Vec<String> = missing.iter().map(|m| m.to_string()).collect();
        DiagnosticItem {
            status: Status::Warn,
            check_name: "模型映射".into(),
            message: format!("缺少 {} 个常见模型的映射", missing.len()),
            detail: Some(names.join(", ")),
            suggestion: Some("请在「账号管理 → 模型映射」中补全缺失的模型对应关系".into()),
        }
    }
}

// ── 6. 注入状态检查 ──────────────────────────────────────────────────────────

fn check_injection_status(ctx: &DiagnosticContext) -> DiagnosticItem {
    let auto = ctx.codex_auto_inject;
    let persistent = ctx.codex_persistent_inject;

    // 检查 Codex 配置中的实际注入状态
    let injected = codex_config::codex_config_path()
        .and_then(|p| {
            if !p.exists() {
                return None;
            }
            let content = codex_config::read_config_file(&p).ok()?;
            let doc: toml_edit::DocumentMut = content.parse().ok()?;
            let provider = doc
                .get("model_provider")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(provider == "deecodex")
        })
        .unwrap_or(false);

    let base_expected = format!("http://127.0.0.1:{}/v1", ctx.port);

    let base_matches = codex_config::codex_config_path()
        .and_then(|p| {
            if !p.exists() {
                return None;
            }
            let content = codex_config::read_config_file(&p).ok()?;
            let doc: toml_edit::DocumentMut = content.parse().ok()?;
            doc.get("model_providers")
                .and_then(|mp| mp.get("deecodex"))
                .and_then(|d| d.get("base_url"))
                .and_then(|v| v.as_str())
                .map(|u| u == base_expected)
        })
        .unwrap_or(false);

    if persistent {
        if injected && base_matches {
            DiagnosticItem {
                status: Status::Pass,
                check_name: "注入状态".into(),
                message: "持久注入已启用，Codex 配置正确".into(),
                detail: Some("deecodex 不会在启动/关闭时自动修改 Codex 配置".into()),
                suggestion: None,
            }
        } else {
            DiagnosticItem {
                status: Status::Warn,
                check_name: "注入状态".into(),
                message: "持久注入已启用但 Codex 配置未正确指向 deecodex".into(),
                detail: Some("deecodex 不会自动管理 Codex 配置，请手动注入".into()),
                suggestion: Some("请在控制面板中点击「注入配置」或关闭持久注入以启用自动管理".into()),
            }
        }
    } else if auto {
        if injected && base_matches {
            DiagnosticItem {
                status: Status::Pass,
                check_name: "注入状态".into(),
                message: "自动注入已启用，Codex 配置正确".into(),
                detail: Some("deecodex 会在启动时自动注入/退出时自动移除 Codex 配置".into()),
                suggestion: None,
            }
        } else {
            DiagnosticItem {
                status: Status::Warn,
                check_name: "注入状态".into(),
                message: "自动注入已启用但 Codex 配置未生效".into(),
                detail: Some("deecodex 启动时会自动注入，请检查服务是否正在运行".into()),
                suggestion: Some("请确保 deecodex 服务已启动，或手动点击「注入配置」".into()),
            }
        }
    } else {
        DiagnosticItem {
            status: Status::Warn,
            check_name: "注入状态".into(),
            message: "自动注入和持久注入均未开启".into(),
            detail: Some("需手动配置 Codex 路由至 deecodex".into()),
            suggestion: Some("建议开启「自动注入」以便 deecodex 自动管理 Codex 配置".into()),
        }
    }
}

// ── 7. deecodex 配置文件正确性 ────────────────────────────────────────────────

fn check_deecodex_config(ctx: &DiagnosticContext) -> DiagnosticItem {
    let config_path = Args::default_config_path(&ctx.data_dir);

    if !config_path.exists() {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "deecodex 配置".into(),
            message: "deecodex 配置文件 config.json 不存在".into(),
            detail: None,
            suggestion: Some("首次启动时将自动创建默认配置".into()),
        };
    }

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Fail,
                check_name: "deecodex 配置".into(),
                message: "无法读取 deecodex 配置文件".into(),
                detail: Some(format!("路径: {}, 错误: {}", config_path.display(), e)),
                suggestion: Some("请检查文件权限".into()),
            };
        }
    };

    if content.trim().is_empty() {
        return DiagnosticItem {
            status: Status::Warn,
            check_name: "deecodex 配置".into(),
            message: "deecodex 配置文件为空".into(),
            detail: Some(format!("路径: {}", config_path.display())),
            suggestion: Some("请在控制面板中重新保存配置".into()),
        };
    }

    let config: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            return DiagnosticItem {
                status: Status::Fail,
                check_name: "deecodex 配置".into(),
                message: "deecodex 配置文件 JSON 格式无效".into(),
                detail: Some(format!("{}", e)),
                suggestion: Some("请在控制面板中重新保存配置或手动修复 JSON 格式".into()),
            };
        }
    };

    // 检查关键字段
    let mut warnings = Vec::new();
    let obj = config.as_object();

    if obj.and_then(|o| o.get("port")).is_none() {
        warnings.push("缺少 port 字段");
    }
    if obj.and_then(|o| o.get("upstream")).is_none() {
        warnings.push("缺少 upstream 字段");
    }

    if warnings.is_empty() {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "deecodex 配置".into(),
            message: "deecodex 配置文件格式正确".into(),
            detail: Some(format!(
                "路径: {}, 端口: {}",
                config_path.display(),
                config.get("port").and_then(|v| v.as_u64()).unwrap_or(ctx.port as u64)
            )),
            suggestion: None,
        }
    } else {
        DiagnosticItem {
            status: Status::Warn,
            check_name: "deecodex 配置".into(),
            message: format!("deecodex 配置文件存在 {} 处问题", warnings.len()),
            detail: Some(warnings.join("; ")),
            suggestion: Some("请在控制面板中补全缺失的配置项".into()),
        }
    }
}

// ── 8. Codex 安装检测 ────────────────────────────────────────────────────────

fn check_codex_installed() -> DiagnosticItem {
    let home_dir = config::home_dir();

    let codex_dir = home_dir.as_ref().map(|h| h.join(".codex"));
    let dir_exists = codex_dir.as_ref().map(|d| d.exists()).unwrap_or(false);
    let in_path = codex_config::find_in_path("codex");

    // Windows 额外检测
    #[cfg(windows)]
    let in_programs = {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|local| {
                std::path::Path::new(&local)
                    .join("Programs")
                    .join("codex")
                    .exists()
            })
            .unwrap_or(false)
    };
    #[cfg(not(windows))]
    let in_programs = false;

    if dir_exists || in_path || in_programs {
        let locations: Vec<&str> = {
            let mut v = Vec::new();
            if dir_exists {
                v.push("~/.codex 目录");
            }
            if in_path {
                v.push("PATH 中可执行");
            }
            if in_programs {
                v.push("Programs 安装目录");
            }
            v
        };
        DiagnosticItem {
            status: Status::Pass,
            check_name: "Codex 安装".into(),
            message: "Codex 已安装".into(),
            detail: Some(locations.join(", ")),
            suggestion: None,
        }
    } else {
        DiagnosticItem {
            status: Status::Warn,
            check_name: "Codex 安装".into(),
            message: "未检测到 Codex 安装".into(),
            detail: None,
            suggestion: Some("请先安装 Codex CLI 或桌面版: https://github.com/openai/codex".into()),
        }
    }
}

// ── 9. 账号配置文件 ───────────────────────────────────────────────────────────

fn check_accounts_config(ctx: &DiagnosticContext) -> DiagnosticItem {
    let path = accounts::accounts_file_path(&ctx.data_dir);

    if !path.exists() {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "账号配置".into(),
            message: "账号配置文件 accounts.json 不存在".into(),
            detail: None,
            suggestion: Some("首次启动时将自动导入 Codex 配置中的账号".into()),
        };
    }

    let store = accounts::load_accounts(&ctx.data_dir);
    let active = store.active_id.clone();
    let count = store.accounts.len();

    if count == 0 {
        DiagnosticItem {
            status: Status::Warn,
            check_name: "账号配置".into(),
            message: "账号配置文件存在但无有效账号".into(),
            detail: Some(format!("路径: {}", path.display())),
            suggestion: Some("请在「账号管理」中添加至少一个上游账号".into()),
        }
    } else {
        let active_name = active
            .as_ref()
            .and_then(|id| store.accounts.iter().find(|a| a.id == *id))
            .map(|a| a.name.as_str())
            .unwrap_or("(未选择)");
        DiagnosticItem {
            status: Status::Pass,
            check_name: "账号配置".into(),
            message: format!("账号配置正常（{} 个账号）", count),
            detail: Some(format!(
                "路径: {}, 当前活跃: {}",
                path.display(),
                active_name
            )),
            suggestion: None,
        }
    }
}

// ── 10. 端口冲突检测 ─────────────────────────────────────────────────────────

fn check_port_conflict(ctx: &DiagnosticContext) -> DiagnosticItem {
    match std::net::TcpListener::bind(("127.0.0.1", ctx.port)) {
        Ok(_) => {
            // 端口可用
            DiagnosticItem {
                status: Status::Pass,
                check_name: "端口冲突".into(),
                message: format!("端口 {} 未被占用", ctx.port),
                detail: None,
                suggestion: None,
            }
        }
        Err(_) => {
            // 端口被占用，尝试找出占用者
            let occupant = find_port_occupant(ctx.port);
            DiagnosticItem {
                status: Status::Warn,
                check_name: "端口冲突".into(),
                message: format!("端口 {} 已被占用", ctx.port),
                detail: occupant.map(|p| format!("占用进程: {}", p)),
                suggestion: Some("如果占用者是 deecodex 自身则正常，否则请关闭占用进程或更换端口".into()),
            }
        }
    }
}

fn find_port_occupant(port: u16) -> Option<String> {
    let output = std::process::Command::new("lsof")
        .arg("-i")
        .arg(format!(":{}", port))
        .arg("-nP")
        .arg("-Fpc")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut name = String::new();

    for line in stdout.lines() {
        if let Some(c) = line.strip_prefix("pc") {
            name = c.to_string();
        } else if let Some(c) = line.strip_prefix("cn") {
            if name.is_empty() {
                name = c.to_string();
            }
        }
    }

    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

// ── 11. Codex 配置一致性 ─────────────────────────────────────────────────────

fn check_codex_config_consistency(_ctx: &DiagnosticContext) -> DiagnosticItem {
    let Some(config_path) = codex_config::codex_config_path() else {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "Codex 配置一致性".into(),
            message: "无法确定 Codex 配置路径".into(),
            detail: None,
            suggestion: None,
        };
    };

    if !config_path.exists() {
        return DiagnosticItem {
            status: Status::Pass,
            check_name: "Codex 配置一致性".into(),
            message: "Codex 配置文件不存在，跳过".into(),
            detail: None,
            suggestion: None,
        };
    }

    let content = match codex_config::read_config_file(&config_path) {
        Ok(c) => c,
        Err(_) => {
            return DiagnosticItem {
                status: Status::Warn,
                check_name: "Codex 配置一致性".into(),
                message: "无法读取 Codex 配置文件".into(),
                detail: None,
                suggestion: None,
            };
        }
    };

    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => {
            return DiagnosticItem {
                status: Status::Warn,
                check_name: "Codex 配置一致性".into(),
                message: "Codex 配置文件解析失败".into(),
                detail: None,
                suggestion: None,
            };
        }
    };

    let mut issues = Vec::new();
    let model_provider = doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .unwrap_or("(未设置)");

    let has_deecodex_section = doc
        .get("model_providers")
        .and_then(|mp| mp.get("deecodex"))
        .is_some();

    // 检查自相矛盾
    if model_provider == "deecodex" && !has_deecodex_section {
        issues.push("model_provider=deecodex 但缺少 [model_providers.deecodex] 节".to_string());
    }
    if model_provider != "deecodex" && has_deecodex_section {
        issues.push(format!(
            "model_provider={} 但存在 [model_providers.deecodex] 节（未被使用）",
            model_provider
        ));
    }

    // 检查 wire_api
    if let Some(wire) = doc
        .get("model_providers")
        .and_then(|mp| mp.get("deecodex"))
        .and_then(|d| d.get("wire_api"))
        .and_then(|v| v.as_str())
    {
        if wire != "responses" {
            issues.push(format!("wire_api={} (应为 responses)", wire));
        }
    }

    // 检查重复节
    let content_lines: Vec<&str> = content.lines().collect();
    let deecodex_sections: Vec<_> = content_lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == "[model_providers.deecodex]")
        .collect();
    if deecodex_sections.len() > 1 {
        issues.push(format!(
            "发现 {} 个重复的 [model_providers.deecodex] 节",
            deecodex_sections.len()
        ));
    }

    if issues.is_empty() {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "Codex 配置一致性".into(),
            message: "Codex 配置一致，未发现矛盾".into(),
            detail: None,
            suggestion: None,
        }
    } else {
        DiagnosticItem {
            status: Status::Warn,
            check_name: "Codex 配置一致性".into(),
            message: format!("Codex 配置存在 {} 处不一致", issues.len()),
            detail: Some(issues.join("; ")),
            suggestion: Some("请在控制面板中点击「注入配置」修复，或运行 deecodex fix-config".into()),
        }
    }
}

// ── 12. models_cache.json 状态 ───────────────────────────────────────────────

fn check_models_cache() -> DiagnosticItem {
    let cache_path = codex_config::codex_home_dir().map(|h| h.join("models_cache.json"));

    match cache_path {
        Some(path) if path.exists() => {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let parsed = serde_json::from_str::<serde_json::Value>(&content).ok();
                    let model_count = parsed
                        .as_ref()
                        .and_then(|v| v.get("models"))
                        .and_then(|m| m.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    DiagnosticItem {
                        status: Status::Pass,
                        check_name: "模型缓存".into(),
                        message: format!("Codex 模型缓存可用（{} 个模型）", model_count),
                        detail: Some(format!("路径: {}", path.display())),
                        suggestion: None,
                    }
                }
                Err(e) => DiagnosticItem {
                    status: Status::Warn,
                    check_name: "模型缓存".into(),
                    message: "无法读取 Codex 模型缓存".into(),
                    detail: Some(format!("路径: {}, 错误: {}", path.display(), e)),
                    suggestion: Some("请运行一次 Codex 以重新生成模型缓存".into()),
                },
            }
        }
        Some(path) => DiagnosticItem {
            status: Status::Info,
            check_name: "模型缓存".into(),
            message: "Codex 尚未生成模型缓存".into(),
            detail: Some(format!("路径: {}", path.display())),
            suggestion: Some("首次启动 Codex 后将自动生成".into()),
        },
        None => DiagnosticItem {
            status: Status::Info,
            check_name: "模型缓存".into(),
            message: "无法确定 Codex 缓存路径".into(),
            detail: None,
            suggestion: None,
        },
    }
}

// ── 13. 磁盘空间 ─────────────────────────────────────────────────────────────

fn check_disk_space(ctx: &DiagnosticContext) -> DiagnosticItem {
    // 确保 data_dir 存在以便检查
    let data_dir = &ctx.data_dir;
    let dir_to_check = if data_dir.exists() {
        data_dir.clone()
    } else if let Some(parent) = data_dir.parent() {
        parent.to_path_buf()
    } else {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "磁盘空间".into(),
            message: "无法确定数据目录，跳过磁盘空间检查".into(),
            detail: None,
            suggestion: None,
        };
    };

    let available_kb = get_disk_available_kb(&dir_to_check);

    match available_kb {
        Some(kb) if kb < 100_000 => {
            // < 100 MB
            let available_mb = kb / 1024;
            DiagnosticItem {
                status: Status::Fail,
                check_name: "磁盘空间".into(),
                message: format!("磁盘可用空间严重不足（{} MB）", available_mb),
                detail: Some(format!("检查路径: {}", dir_to_check.display())),
                suggestion: Some("请释放磁盘空间以确保 deecodex 正常运行".into()),
            }
        }
        Some(kb) if kb < 1_000_000 => {
            // < 1 GB
            let available_mb = kb / 1024;
            DiagnosticItem {
                status: Status::Warn,
                check_name: "磁盘空间".into(),
                message: format!("磁盘可用空间偏低（{} MB）", available_mb),
                detail: Some(format!("检查路径: {}", dir_to_check.display())),
                suggestion: Some("建议释放磁盘空间".into()),
            }
        }
        Some(kb) => {
            let available_gb = kb / (1024 * 1024);
            DiagnosticItem {
                status: Status::Pass,
                check_name: "磁盘空间".into(),
                message: format!("磁盘可用空间充足（{} GB）", available_gb),
                detail: Some(format!("检查路径: {}", dir_to_check.display())),
                suggestion: None,
            }
        }
        None => DiagnosticItem {
            status: Status::Info,
            check_name: "磁盘空间".into(),
            message: "无法获取磁盘空间信息".into(),
            detail: None,
            suggestion: None,
        },
    }
}

#[cfg(unix)]
fn get_disk_available_kb(path: &Path) -> Option<u64> {
    // 通过 df 命令获取指定路径的磁盘可用空间
    let _meta = std::fs::metadata(path).ok()?;

    // 从 /proc/mounts 或直接解析 df 获取该设备的可用空间
    let output = std::process::Command::new("df")
        .arg("-k")
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // df -k 输出格式: Filesystem 1024-blocks Used Available ...
    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            return parts[3].parse::<u64>().ok();
        }
    }
    None
}

#[cfg(not(unix))]
fn get_disk_available_kb(_path: &Path) -> Option<u64> {
    // Windows: 暂时跳过（后续可集成 winapi）
    None
}

// ── 14. Codex 先于 deecodex 启动检测 ─────────────────────────────────────────

fn check_codex_startup_order(ctx: &DiagnosticContext) -> DiagnosticItem {
    // 只在 auto_inject 开启时检测
    if !ctx.codex_auto_inject {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "启动顺序".into(),
            message: "自动注入未启用，跳过启动顺序检测".into(),
            detail: None,
            suggestion: None,
        };
    }

    // 检查 deecodex 是否在运行
    let deecodex_running = {
        let pid_path = ctx.data_dir.join("deecodex.pid");
        std::fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .filter(|&p| process_is_running(p))
            .or_else(find_daemon_pid)
            .is_some()
    };

    if !deecodex_running {
        return DiagnosticItem {
            status: Status::Info,
            check_name: "启动顺序".into(),
            message: "deecodex 未运行，跳过启动顺序检测".into(),
            detail: None,
            suggestion: Some("请先启动 deecodex 服务".into()),
        };
    }

    // deecodex 在运行 + auto_inject → 检查 codex config 是否被注入
    let injected = codex_config::codex_config_path()
        .and_then(|p| {
            if !p.exists() {
                return None;
            }
            let content = codex_config::read_config_file(&p).ok()?;
            let doc: toml_edit::DocumentMut = content.parse().ok()?;
            Some(
                doc.get("model_provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    == "deecodex",
            )
        })
        .unwrap_or(false);

    if injected {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "启动顺序".into(),
            message: "启动顺序正常：Codex 已正确路由到 deecodex".into(),
            detail: None,
            suggestion: None,
        }
    } else {
        DiagnosticItem {
            status: Status::Fail,
            check_name: "启动顺序".into(),
            message: "Codex 可能在 deecodex 之前启动，配置未注入".into(),
            detail: Some("deecodex 正在运行且自动注入已开启，但 Codex 配置未指向 deecodex".into()),
            suggestion: Some("请重启 Codex（deecodex 已就绪），或在控制面板中点击「注入配置」手动注入后重启 Codex".into()),
        }
    }
}

// ── 15. 配置文件备份检测 ─────────────────────────────────────────────────────

fn check_config_backups(ctx: &DiagnosticContext) -> DiagnosticItem {
    let mut backups = Vec::new();

    // 检查 Codex 配置备份
    if let Some(codex_dir) = codex_config::codex_home_dir() {
        for ext in &["bak", "backup", "old"] {
            let path = codex_dir.join(format!("config.toml.{}", ext));
            if path.exists() {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(modified) = meta.modified() {
                        if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                            let days_ago = (std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                                - dur.as_secs())
                                / 86400;
                            backups.push(format!(
                                "Codex: config.toml.{} ({} 天前)",
                                ext, days_ago
                            ));
                        }
                    }
                }
            }
        }
    }

    // 检查 deecodex 配置备份
    for ext in &["bak", "backup", "old"] {
        let path = ctx.data_dir.join(format!("config.json.{}", ext));
        if path.exists() {
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                        let days_ago = (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                            - dur.as_secs())
                            / 86400;
                        backups.push(format!(
                            "deecodex: config.json.{} ({} 天前)",
                            ext,
                            days_ago
                        ));
                    }
                }
            }
        }
    }

    // 检查 backups 目录
    let backup_dir = ctx.data_dir.join("backups");
    if backup_dir.exists() {
        match std::fs::read_dir(&backup_dir) {
            Ok(entries) => {
                let count = entries.flatten().count();
                if count > 0 {
                    backups.push(format!("会话备份: {} 条记录", count));
                }
            }
            Err(_) => {}
        }
    }

    if backups.is_empty() {
        DiagnosticItem {
            status: Status::Info,
            check_name: "配置备份".into(),
            message: "未发现配置文件备份".into(),
            detail: None,
            suggestion: Some("如需恢复误操作，可在控制台中手动备份配置".into()),
        }
    } else {
        DiagnosticItem {
            status: Status::Pass,
            check_name: "配置备份".into(),
            message: format!("发现 {} 份备份", backups.len()),
            detail: Some(backups.join("; ")),
            suggestion: None,
        }
    }
}

// ── 向后兼容：旧类型 ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
}

/// 启动前配置诊断（向后兼容）。
/// 不阻塞启动——由调用方决定哪些错误是致命的。
pub fn validate(args: &Args) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    check_data_dir(args, &mut diags);
    check_api_key(args, &mut diags);
    check_model_map(args, &mut diags);
    check_computer_executor(args, &mut diags);
    check_mcp_executor(args, &mut diags);
    check_file_search(args, &mut diags);

    diags
}

fn check_data_dir(args: &Args, diags: &mut Vec<Diagnostic>) {
    let dir = Path::new(&args.data_dir);
    match std::fs::create_dir_all(dir) {
        Ok(()) => {
            let md = match std::fs::metadata(dir) {
                Ok(md) => md,
                Err(e) => {
                    diags.push(Diagnostic {
                        severity: Severity::Error,
                        category: "data_dir",
                        message: format!("无法读取数据目录 {} 的元数据: {}", dir.display(), e),
                    });
                    return;
                }
            };
            if !md.is_dir() {
                diags.push(Diagnostic {
                    severity: Severity::Error,
                    category: "data_dir",
                    message: format!("数据目录 {} 不是目录", dir.display()),
                });
            }
        }
        Err(e) => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "data_dir",
                message: format!("无法创建数据目录 {}: {}", dir.display(), e),
            });
        }
    }
}

fn check_api_key(args: &Args, diags: &mut Vec<Diagnostic>) {
    if args.api_key.trim().is_empty() {
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "api_key",
            message: "API Key 未配置——上游请求将因认证失败而报错，请在账号管理中配置".into(),
        });
    }
}

fn check_model_map(args: &Args, diags: &mut Vec<Diagnostic>) {
    let raw = args.model_map.trim();
    if raw.is_empty() || raw == "{}" {
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "model_map",
            message: "模型映射为空——Codex 请求的模型名将无法转换为上游模型".into(),
        });
        return;
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => {
            if let Some(obj) = v.as_object() {
                if obj.is_empty() {
                    diags.push(Diagnostic {
                        severity: Severity::Warn,
                        category: "model_map",
                        message: "模型映射为空对象——Codex 请求的模型名将无法转换".into(),
                    });
                }
            }
        }
        Err(e) => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "model_map",
                message: format!("模型映射 JSON 解析失败: {}", e),
            });
        }
    }
}

fn check_computer_executor(args: &Args, diags: &mut Vec<Diagnostic>) {
    let backend = args.computer_executor.trim().to_ascii_lowercase();
    if backend.is_empty() || backend == "disabled" {
        return;
    }

    if backend == "playwright" {
        check_playwright(args, diags);
    } else if backend == "browser-use" || backend == "browser_use" || backend == "browseruse" {
        check_browser_use_bridge(args, diags);
    } else {
        diags.push(Diagnostic {
            severity: Severity::Error,
            category: "computer_executor",
            message: format!(
                "未知的 computer executor 后端 '{}'，支持: disabled / playwright / browser-use",
                args.computer_executor
            ),
        });
    }
}

fn check_playwright(args: &Args, diags: &mut Vec<Diagnostic>) {
    let node_check = std::process::Command::new("node")
        .arg("-e")
        .arg("process.exit(0)")
        .output();

    match node_check {
        Ok(output) if output.status.success() => {}
        _ => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "computer_executor",
                message: "computer executor 设为 playwright，但 node 命令不可用——Playwright 需要 Node.js 运行时".into(),
            });
            return;
        }
    }

    let import_check = std::process::Command::new("node")
        .arg("-e")
        .arg("require('playwright')")
        .output();

    match import_check {
        Ok(output) if output.status.success() => {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "computer_executor",
                message: "Playwright 可用（检测通过）".into(),
            });
        }
        _ => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "computer_executor",
                message: "computer executor 设为 playwright，但 Node.js 无法 import('playwright')——请确认 playwright 已安装 (npm install playwright)".into(),
            });
        }
    }

    if !args.playwright_state_dir.is_empty() {
        let dir = Path::new(&args.playwright_state_dir);
        match std::fs::create_dir_all(dir) {
            Ok(()) => {}
            Err(e) => {
                diags.push(Diagnostic {
                    severity: Severity::Warn,
                    category: "computer_executor",
                    message: format!(
                        "Playwright state 目录 {} 无法创建: {}——浏览器状态将不会持久化",
                        dir.display(),
                        e
                    ),
                });
            }
        }
    }
}

fn check_browser_use_bridge(_args: &Args, diags: &mut Vec<Diagnostic>) {
    let url = std::env::var("DEECODEX_BROWSER_USE_BRIDGE_URL")
        .unwrap_or_default()
        .trim()
        .to_string();
    let command = std::env::var("DEECODEX_BROWSER_USE_BRIDGE_COMMAND")
        .unwrap_or_default()
        .trim()
        .to_string();

    if url.is_empty() && command.is_empty() {
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "computer_executor",
            message: "computer executor 设为 browser-use，但未配置 DEECODEX_BROWSER_USE_BRIDGE_URL 和 DEECODEX_BROWSER_USE_BRIDGE_COMMAND——browser-use 操作将返回失败".into(),
        });
        return;
    }

    if !url.is_empty() {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "computer_executor",
                message: format!(
                    "browser-use bridge URL '{}' 不以 http:// 或 https:// 开头，可能不是有效的 HTTP 地址",
                    url
                ),
            });
        }
    }

    if !command.is_empty() {
        let cmd_name = command.split_whitespace().next().unwrap_or(&command);
        if which::which(cmd_name).is_err() {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "computer_executor",
                message: format!(
                    "browser-use bridge 命令 '{}' 不在 PATH 中——bridge 调用将失败",
                    cmd_name
                ),
            });
        }
    }
}

fn check_mcp_executor(args: &Args, diags: &mut Vec<Diagnostic>) {
    let raw = args.mcp_executor_config.trim();
    if raw.is_empty() {
        return;
    }

    let configs: Vec<serde_json::Value> = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Array(arr)) => arr,
        Ok(serde_json::Value::Object(obj)) => vec![serde_json::Value::Object(obj)],
        Ok(_) => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "mcp_executor",
                message: "MCP executor 配置必须是 JSON 对象或数组".into(),
            });
            return;
        }
        Err(e) => {
            let path = Path::new(raw);
            if path.exists() && path.extension().is_some_and(|ext| ext == "json") {
                match std::fs::read_to_string(path) {
                    Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(serde_json::Value::Array(arr)) => {
                            diags.push(Diagnostic {
                                severity: Severity::Warn,
                                category: "mcp_executor",
                                message: format!(
                                    "MCP executor 配置从文件 {} 加载（{} 个 server）",
                                    path.display(),
                                    arr.len()
                                ),
                            });
                            for item in &arr {
                                check_mcp_server_config(item, diags);
                            }
                        }
                        Ok(serde_json::Value::Object(obj)) => {
                            diags.push(Diagnostic {
                                severity: Severity::Warn,
                                category: "mcp_executor",
                                message: format!("MCP executor 配置从文件 {} 加载", path.display()),
                            });
                            check_mcp_server_config(&serde_json::Value::Object(obj), diags);
                        }
                        _ => {
                            diags.push(Diagnostic {
                                severity: Severity::Error,
                                category: "mcp_executor",
                                message: format!(
                                    "MCP executor 配置文件 {} 内容必须是 JSON 对象或数组",
                                    path.display()
                                ),
                            });
                        }
                    },
                    Err(e) => {
                        diags.push(Diagnostic {
                            severity: Severity::Error,
                            category: "mcp_executor",
                            message: format!(
                                "无法读取 MCP executor 配置文件 {}: {}",
                                path.display(),
                                e
                            ),
                        });
                    }
                }
                return;
            }
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "mcp_executor",
                message: format!(
                    "MCP executor 配置不是有效的 JSON 也不是存在的 .json 文件: {}",
                    e
                ),
            });
            return;
        }
    };

    for config in &configs {
        check_mcp_server_config(config, diags);
    }
}

fn check_mcp_server_config(config: &serde_json::Value, diags: &mut Vec<Diagnostic>) {
    let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        if let Some(obj) = config.as_object() {
            for (label, server_config) in obj {
                check_single_mcp_server(label, server_config, diags);
            }
        }
        return;
    }

    check_single_mcp_server("(未命名)", config, diags);
}

fn check_single_mcp_server(label: &str, config: &serde_json::Value, diags: &mut Vec<Diagnostic>) {
    let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        diags.push(Diagnostic {
            severity: Severity::Error,
            category: "mcp_executor",
            message: format!("MCP server '{}' 缺少 command 字段", label),
        });
        return;
    }

    if which::which(command).is_err() {
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "mcp_executor",
            message: format!(
                "MCP server '{}' 的命令 '{}' 不在 PATH 中——工具调用将失败",
                label, command
            ),
        });
    }

    let read_only = config
        .get("read_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if read_only {
        let args_count = config
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "mcp_executor",
            message: format!(
                "MCP server '{}' 以只读模式运行（read_only=true）——写入/删除类工具将被拒绝",
                label
            ),
        });
        if args_count > 0 {
            let args_str = config
                .get("args")
                .map(|v| v.to_string())
                .unwrap_or_default();
            if args_str.contains('/') || args_str.contains("root") || args_str.contains("home") {
                diags.push(Diagnostic {
                    severity: Severity::Warn,
                    category: "mcp_executor",
                    message: format!(
                        "MCP server '{}' args 中包含敏感路径——请确认只读模式下的访问范围符合预期",
                        label
                    ),
                });
            }
        }
    }
}

fn check_file_search(args: &Args, diags: &mut Vec<Diagnostic>) {
    let files_dir = Path::new(&args.data_dir).join("files");

    if !files_dir.exists() {
        return;
    }

    let Ok(entries) = std::fs::read_dir(&files_dir) else {
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "file_search",
            message: format!("无法读取 file_search 数据目录 {}", files_dir.display()),
        });
        return;
    };

    let mut json_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut bin_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut parse_errors = 0usize;
    let mut text_file_count = 0usize;
    let mut binary_file_count = 0usize;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        match path.extension().and_then(|s| s.to_str()) {
            Some("json") => {
                json_ids.insert(stem.to_string());
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&content) {
                            let ct = meta
                                .get("content_type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if is_text_content_type(ct) {
                                text_file_count += 1;
                            } else {
                                binary_file_count += 1;
                            }
                        } else {
                            parse_errors += 1;
                            diags.push(Diagnostic {
                                severity: Severity::Warn,
                                category: "file_search",
                                message: format!(
                                    "文件元数据 {} 无法解析，索引可能不完整",
                                    path.display()
                                ),
                            });
                        }
                    }
                    Err(e) => {
                        parse_errors += 1;
                        diags.push(Diagnostic {
                            severity: Severity::Warn,
                            category: "file_search",
                            message: format!("无法读取文件元数据 {}: {}", path.display(), e),
                        });
                    }
                }
            }
            Some("bin") => {
                bin_ids.insert(stem.to_string());
            }
            _ => {}
        }
    }

    let total = json_ids.len();

    for id in &json_ids {
        if !bin_ids.contains(id) {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "file_search",
                message: format!("文件 {} 缺少对应的 .bin 数据（元数据孤立）", id),
            });
        }
    }
    for id in &bin_ids {
        if !json_ids.contains(id) {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "file_search",
                message: format!("文件 {} 缺少对应的 .json 元数据（数据孤立）", id),
            });
        }
    }

    if total > 0 {
        let status = if parse_errors > 0 {
            format!(
                "file_search: {} 个文件（{} 可索引，{} 二进制，{} 个元数据异常）",
                total, text_file_count, binary_file_count, parse_errors
            )
        } else {
            format!(
                "file_search: {} 个文件（{} 可索引，{} 二进制）",
                total, text_file_count, binary_file_count
            )
        };
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "file_search",
            message: status,
        });
    }
}

fn is_text_content_type(content_type: &str) -> bool {
    let ct_lower = content_type.to_ascii_lowercase();
    ct_lower.is_empty()
        || ct_lower.starts_with("text/")
        || ct_lower.contains("json")
        || ct_lower.contains("xml")
        || ct_lower.contains("javascript")
        || ct_lower.contains("yaml")
        || ct_lower == "application/octet-stream"
}

// ── 测试 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> Args {
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
            codex_launch_with_cdp: false,
            cdp_port: 4448,
            prompts_dir: std::path::PathBuf::from("prompts"),
            data_dir: std::path::PathBuf::from(".deecodex"),
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

    // ── 旧 validate 兼容测试 ─────────────────────────────────────────────────

    #[test]
    fn data_dir_is_creatable_no_error() {
        let dir = std::env::temp_dir().join("deecodex-validate-test");
        let _ = std::fs::remove_dir_all(&dir);
        let mut args = base_args();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        assert!(!diags
            .iter()
            .any(|d| d.category == "data_dir" && d.severity == Severity::Error));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_path_is_file_errors() {
        let dir = std::env::temp_dir().join("deecodex-validate-file-test");
        let _ = std::fs::remove_file(&dir);
        std::fs::write(&dir, b"").unwrap();
        let mut args = base_args();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "data_dir" && d.severity == Severity::Error));
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn disabled_computer_executor_produces_no_diags() {
        let args = base_args();
        let diags = validate(&args);
        let computer_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "computer_executor")
            .collect();
        assert!(computer_diags.is_empty());
    }

    #[test]
    fn unknown_computer_backend_is_error() {
        let mut args = base_args();
        args.computer_executor = "unknown-backend".into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "computer_executor" && d.severity == Severity::Error));
    }

    #[test]
    fn empty_mcp_config_produces_no_diags() {
        let args = base_args();
        let diags = validate(&args);
        let mcp_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "mcp_executor")
            .collect();
        assert!(mcp_diags.is_empty());
    }

    #[test]
    fn invalid_mcp_json_is_error() {
        let mut args = base_args();
        args.mcp_executor_config = "not json".into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "mcp_executor" && d.severity == Severity::Error));
    }

    #[test]
    fn mcp_server_without_command_is_error() {
        let mut args = base_args();
        args.mcp_executor_config = r#"{"test":{"no_command":true}}"#.into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "mcp_executor" && d.severity == Severity::Error));
    }

    #[test]
    fn mcp_server_read_only_is_info() {
        let mut args = base_args();
        args.mcp_executor_config =
            r#"{"filesystem":{"command":"ls","args":["/tmp"],"read_only":true}}"#.into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "mcp_executor" && d.message.contains("只读模式")));
    }

    #[test]
    fn file_search_nonexistent_dir_is_noop() {
        let mut args = base_args();
        let dir = std::env::temp_dir().join("deecodex-validate-fs-nonexist");
        let _ = std::fs::remove_dir_all(&dir);
        args.data_dir = dir.clone();

        let diags = validate(&args);
        let fs_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "file_search")
            .collect();
        assert!(fs_diags.is_empty());
    }

    #[test]
    fn file_search_empty_dir_is_noop() {
        let mut args = base_args();
        let dir = std::env::temp_dir().join("deecodex-validate-fs-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        let fs_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "file_search")
            .collect();
        assert!(fs_diags.is_empty(), "空目录应无诊断，实际: {:?}", fs_diags);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_search_detects_orphaned_metadata() {
        let mut args = base_args();
        let dir = std::env::temp_dir().join("deecodex-validate-fs-orphan-meta");
        let _ = std::fs::remove_dir_all(&dir);
        let files_dir = dir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(
            files_dir.join("file_abc.json"),
            r#"{"id":"file_abc","filename":"test.txt","purpose":"file_search","content_type":"text/plain","created_at":1}"#,
        )
        .unwrap();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        let fs_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "file_search")
            .collect();
        assert!(
            fs_diags
                .iter()
                .any(|d| d.message.contains("缺少对应的 .bin")),
            "应检测到孤儿元数据，实际诊断: {:?}",
            fs_diags
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_search_detects_orphaned_data() {
        let mut args = base_args();
        let dir = std::env::temp_dir().join("deecodex-validate-fs-orphan-bin");
        let _ = std::fs::remove_dir_all(&dir);
        let files_dir = dir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(files_dir.join("file_xyz.bin"), b"hello world").unwrap();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        let fs_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "file_search")
            .collect();
        assert!(
            fs_diags
                .iter()
                .any(|d| d.message.contains("缺少对应的 .json")),
            "应检测到孤儿数据文件，实际诊断: {:?}",
            fs_diags
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_search_reports_valid_file_count() {
        let mut args = base_args();
        let dir = std::env::temp_dir().join("deecodex-validate-fs-valid");
        let _ = std::fs::remove_dir_all(&dir);
        let files_dir = dir.join("files");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::write(
            files_dir.join("file_001.json"),
            r#"{"id":"file_001","filename":"test.py","purpose":"file_search","content_type":"text/x-python","created_at":1}"#,
        )
        .unwrap();
        std::fs::write(files_dir.join("file_001.bin"), b"print('hello')").unwrap();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        let fs_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "file_search")
            .collect();
        assert!(
            fs_diags
                .iter()
                .any(|d| d.message.contains("1 个文件") && d.message.contains("1 可索引")),
            "应报告文件数量，实际诊断: {:?}",
            fs_diags
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_api_key_is_warn() {
        let args = base_args();
        let diags = validate(&args);
        assert!(
            diags
                .iter()
                .any(|d| d.category == "api_key" && d.severity == Severity::Warn),
            "空 API Key 应产生警告诊断，实际: {:?}",
            diags
        );
    }

    #[test]
    fn non_empty_api_key_is_silent() {
        let mut args = base_args();
        args.api_key = "sk-test-key".into();
        let diags = validate(&args);
        assert!(
            !diags.iter().any(|d| d.category == "api_key"),
            "已配置 API Key 不应产生诊断，实际: {:?}",
            diags
        );
    }

    #[test]
    fn empty_model_map_is_warn() {
        let args = base_args();
        let diags = validate(&args);
        assert!(
            diags
                .iter()
                .any(|d| d.category == "model_map" && d.severity == Severity::Warn),
            "空模型映射应产生警告诊断，实际: {:?}",
            diags
        );
    }

    #[test]
    fn invalid_model_map_json_is_error() {
        let mut args = base_args();
        args.model_map = "not json".into();
        let diags = validate(&args);
        assert!(
            diags
                .iter()
                .any(|d| d.category == "model_map" && d.severity == Severity::Error),
            "无效的模型映射 JSON 应产生错误诊断，实际: {:?}",
            diags
        );
    }

    #[test]
    fn is_text_content_type_classifies_correctly() {
        assert!(is_text_content_type(""));
        assert!(is_text_content_type("text/plain"));
        assert!(is_text_content_type("text/html; charset=utf-8"));
        assert!(is_text_content_type("text/x-python"));
        assert!(is_text_content_type("application/json"));
        assert!(is_text_content_type("application/xml"));
        assert!(is_text_content_type("application/javascript"));
        assert!(is_text_content_type("text/yaml"));
        assert!(is_text_content_type("application/octet-stream"));
        assert!(!is_text_content_type("image/png"));
        assert!(!is_text_content_type("audio/mpeg"));
        assert!(!is_text_content_type("video/mp4"));
    }

    // ── 新诊断测试 ───────────────────────────────────────────────────────────

    fn test_context() -> DiagnosticContext {
        DiagnosticContext {
            data_dir: PathBuf::from(".deecodex"),
            port: 4446,
            upstream: "https://openrouter.ai/api/v1".into(),
            api_key: "sk-test".into(),
            client_api_key: "".into(),
            model_map: "{}".into(),
            codex_auto_inject: true,
            codex_persistent_inject: false,
            codex_launch_with_cdp: false,
            cdp_port: 9222,
        }
    }

    #[test]
    fn new_diagnostic_report_has_correct_groups() {
        let ctx = test_context();
        let report = run_diagnostics_sync(&ctx);
        assert_eq!(report.groups.len(), 5);
        assert_eq!(report.groups[0].name, "服务状态");
        assert_eq!(report.groups[1].name, "账号连通");
        assert_eq!(report.groups[2].name, "Codex 路由");
        assert_eq!(report.groups[3].name, "注入状态");
        assert_eq!(report.groups[4].name, "运行环境");
    }

    #[test]
    fn empty_model_map_is_warn_new() {
        let ctx = test_context();
        let item = check_model_mapping(&ctx);
        assert_eq!(item.status, Status::Warn);
    }

    #[test]
    fn model_map_with_all_models_is_pass() {
        let mut ctx = test_context();
        let mut map = serde_json::Map::new();
        for model in COMMON_MODELS {
            map.insert(model.to_string(), format!("openai/{}", model).into());
        }
        ctx.model_map = serde_json::to_string(&map).unwrap();
        let item = check_model_mapping(&ctx);
        assert_eq!(item.status, Status::Pass);
    }

    #[test]
    fn model_map_missing_some_is_warn() {
        let mut ctx = test_context();
        let map = serde_json::json!({
            "gpt-4o": "openai/gpt-4o",
            "gpt-4o-mini": "openai/gpt-4o-mini"
        });
        ctx.model_map = serde_json::to_string(&map).unwrap();
        let item = check_model_mapping(&ctx);
        assert_eq!(item.status, Status::Warn);
        let detail = item.detail.unwrap();
        assert!(detail.contains("claude-sonnet-4-5") || detail.contains("deepseek-chat"));
    }

    #[test]
    fn port_free_is_pass() {
        // 使用一个不太可能被占用的高位端口
        let ctx = DiagnosticContext {
            port: 44460,
            ..test_context()
        };
        let item = check_port_conflict(&ctx);
        assert_eq!(item.status, Status::Pass);
    }

    #[test]
    fn both_inject_disabled_is_warn() {
        let ctx = DiagnosticContext {
            codex_auto_inject: false,
            codex_persistent_inject: false,
            ..test_context()
        };
        let item = check_injection_status(&ctx);
        assert_eq!(item.status, Status::Warn);
    }

    #[test]
    fn connectivity_check_result_ok() {
        let item = connectivity_check_result(true, 200, 150, Some(42), "https://api.example.com/models", None);
        assert_eq!(item.status, Status::Pass);
        assert!(item.message.contains("150ms"));
    }

    #[test]
    fn connectivity_check_result_fail() {
        let item = connectivity_check_result(false, 0, 5000, None, "https://api.example.com/models", Some("timeout"));
        assert_eq!(item.status, Status::Fail);
        assert!(item.message.contains("5000ms"));
    }

    #[test]
    fn report_computes_summary_correctly() {
        let groups = vec![
            DiagnosticGroup {
                name: "测试组".into(),
                health: GroupHealth::Healthy,
                items: vec![
                    DiagnosticItem {
                        status: Status::Pass,
                        check_name: "项A".into(),
                        message: "ok".into(),
                        detail: None,
                        suggestion: None,
                    },
                    DiagnosticItem {
                        status: Status::Warn,
                        check_name: "项B".into(),
                        message: "warn".into(),
                        detail: None,
                        suggestion: None,
                    },
                    DiagnosticItem {
                        status: Status::Fail,
                        check_name: "项C".into(),
                        message: "fail".into(),
                        detail: None,
                        suggestion: None,
                    },
                ],
            },
        ];
        let report = DiagnosticReport::new(groups).with_computed_health();
        assert_eq!(report.summary.total, 3);
        assert_eq!(report.summary.pass, 1);
        assert_eq!(report.summary.warn, 1);
        assert_eq!(report.summary.fail, 1);
        assert_eq!(report.summary.health, GroupHealth::Broken);
        assert_eq!(report.groups[0].health, GroupHealth::Broken);
    }

    #[test]
    fn report_json_serializes() {
        let ctx = test_context();
        let report = run_diagnostics_sync(&ctx);
        let json = serde_json::to_string_pretty(&report).unwrap();
        assert!(json.contains("\"summary\""));
        assert!(json.contains("\"groups\""));
        assert!(json.contains("\"version\""));
        assert!(json.contains("\"check_name\""));
    }
}
