use std::collections::HashSet;

use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

use super::dex_cli::*;
use super::dex_process::detect_process_instances;
use super::dex_security::mask_sensitive_value;

/// DEX 助手：Claude Code MCP 配置检查
pub(super) fn dex_claude_mcp_check_impl() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let claude_dir = home.join(".claude");

    let mcp_path = claude_dir.join("mcp.json");
    let desktop_path = claude_dir.join("claude_desktop_config.json");

    let mut has_mcp_config = false;
    let mut has_deecodex_entry = false;
    let mut config_path = String::new();
    let mut issues = Vec::new();
    let mut mcp_servers = HashSet::new();
    let mut config_files = Vec::new();

    for path in &[&mcp_path, &desktop_path] {
        if !path.exists() {
            continue;
        }
        has_mcp_config = true;
        if config_path.is_empty() {
            config_path = path.to_string_lossy().to_string();
        }
        let mut file = json!({
            "path": path.to_string_lossy(),
            "exists": true,
            "valid_json": false,
            "mcp_servers": [],
        });

        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(json_val) = serde_json::from_str::<Value>(&content) {
                let servers = extract_mcp_servers(&json_val);
                for server in &servers {
                    mcp_servers.insert(server.clone());
                }
                if json_val
                    .get("mcpServers")
                    .and_then(|s| s.get("deecodex"))
                    .is_some()
                {
                    has_deecodex_entry = true;
                }
                file["valid_json"] = json!(true);
                file["top_level_keys"] = json!(top_level_keys(&json_val));
                file["mcp_servers"] = json!(servers);
            } else {
                tracing::warn!(path = %path.display(), "Claude MCP 配置 JSON 解析失败");
                issues.push(format!(
                    "{} 格式无效",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
                file["error"] = json!("JSON 格式无效");
            }
        } else {
            tracing::warn!(path = %path.display(), "Claude MCP 配置读取失败");
            issues.push(format!(
                "{} 读取失败",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            file["error"] = json!("读取失败");
        }
        config_files.push(file);
    }

    if !has_mcp_config {
        tracing::warn!("未找到 Claude MCP 配置文件");
        issues
            .push("未找到 ~/.claude/mcp.json 或 ~/.claude/claude_desktop_config.json".to_string());
    }
    if has_mcp_config && !has_deecodex_entry {
        issues.push("MCP 配置文件中未找到 deecodex 条目".to_string());
    }
    let mut mcp_servers: Vec<String> = mcp_servers.into_iter().collect();
    mcp_servers.sort();

    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty(),
        "has_mcp_config": has_mcp_config,
        "has_deecodex_entry": has_deecodex_entry,
        "config_path": config_path,
        "config_files": config_files,
        "mcp_servers": mcp_servers,
        "issues": issues,
        "recommendations": if has_mcp_config {
            vec!["确认 Claude Code 使用的 MCP 配置文件与当前项目一致".to_string()]
        } else {
            vec!["先创建 Claude MCP 配置，再添加 deecodex 或其他 MCP server".to_string()]
        },
    })))
}

/// DEX 助手：Claude Code 环境概览
pub(super) fn dex_claude_env_overview_impl() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let claude_dir = home.join(".claude");
    let mcp = dex_claude_mcp_check_impl()?;
    let installed =
        command_exists("claude") || command_first_line("claude", &["--version"]).is_some();
    let mut issues = Vec::new();
    if !installed {
        tracing::warn!("Claude CLI 未安装或不在 PATH");
        issues.push("Claude CLI 未安装或不在 PATH".to_string());
    }
    if !claude_dir.exists() {
        issues.push("~/.claude 配置目录不存在".to_string());
    }
    if mcp
        .get("ok")
        .and_then(Value::as_bool)
        .map(|ok| !ok)
        .unwrap_or(false)
    {
        issues.push("Claude MCP 配置需要关注".to_string());
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty(),
        "installed": installed,
        "version": get_cli_version_flexible("claude"),
        "config_dir": claude_dir.to_string_lossy(),
        "config_dir_exists": claude_dir.exists(),
        "config": config_dir_summary(
            &claude_dir,
            &["mcp.json", "claude_desktop_config.json", "settings.json"]
        ),
        "mcp": mcp,
        "issues": issues,
        "recommendations": [
            "优先确认 Claude Code CLI 版本、~/.claude 配置目录和 MCP server 条目",
            "如果 Claude 可以运行但 MCP 失败，先检查 mcp.json 的 JSON 格式和 server 名称",
        ],
    })))
}

/// DEX 助手：OpenClaw 环境概览
pub(super) async fn dex_openclaw_env_overview_impl() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let config_dir = home.join(".openclaw");
    let mut issues = Vec::new();
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    if !config_dir.exists() {
        tracing::warn!(path = %config_dir.display(), "OpenClaw 配置目录不存在");
        issues.push("~/.openclaw 配置目录不存在".to_string());
    }
    let status = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[vec!["status", "--json"], vec!["status"]],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("openclaw", &["status"], "openclaw 不在 PATH".into())
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw status", &status, &mut issues);
    }
    let mcp = dex_openclaw_mcp_check_impl().await?;
    let process_instances = detect_process_instances("openclaw");
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty(),
        "installed": command_exists("openclaw"),
        "version": get_cli_version_flexible("openclaw"),
        "config_dir": config_dir.to_string_lossy(),
        "config_dir_exists": config_dir.exists(),
        "config": config_dir_summary(
            &config_dir,
            &["config.json", "settings.json", "mcp.json", "agents.json", "models.json"]
        ),
        "status": status.to_value(),
        "mcp": mcp,
        "process": {
            "running": !process_instances.is_empty(),
            "instances": process_instances,
        },
        "issues": issues,
    })))
}

/// DEX 助手：OpenClaw 健康检查
pub(super) async fn dex_openclaw_health_check_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    let health = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[
                vec!["health", "--json"],
                vec!["health"],
                vec!["doctor", "--json"],
                vec!["doctor"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("openclaw", &["health"], "openclaw 不在 PATH".into())
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw health", &health, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && health.success,
        "health": health.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：OpenClaw MCP 检查
pub(super) async fn dex_openclaw_mcp_check_impl() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let config_dir = home.join(".openclaw");
    let config_paths = [
        config_dir.join("mcp.json"),
        config_dir.join("config.json"),
        config_dir.join("settings.json"),
    ];
    let (config_files, mut config_servers, mut issues) = inspect_json_config_files(&config_paths);
    if config_files.is_empty() {
        tracing::warn!(path = %config_dir.display(), "未找到 OpenClaw MCP 相关 JSON 配置");
        issues.push("未找到 OpenClaw MCP 相关 JSON 配置".to_string());
    }
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    let command = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[
                vec!["mcp", "list", "--json"],
                vec!["mcp", "list"],
                vec!["mcp", "show", "--json"],
                vec!["mcp", "show"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("openclaw", &["mcp", "list"], "openclaw 不在 PATH".into())
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw MCP", &command, &mut issues);
    }
    config_servers.extend(mcp_servers_from_command(&command));
    config_servers.sort();
    config_servers.dedup();
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty(),
        "config_dir": config_dir.to_string_lossy(),
        "config_files": config_files,
        "mcp_servers": config_servers,
        "command": command.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：OpenClaw Gateway 概览
pub(super) async fn dex_openclaw_gateway_overview_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    let command = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[
                vec!["gateway", "status", "--json"],
                vec!["gateway", "status"],
                vec!["gateway", "health", "--json"],
                vec!["gateway", "health"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable(
            "openclaw",
            &["gateway", "status"],
            "openclaw 不在 PATH".into(),
        )
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw Gateway", &command, &mut issues);
    }
    let instances = detect_process_instances("openclaw");
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && (command.success || !instances.is_empty()),
        "gateway": command.to_value(),
        "process": {
            "running": !instances.is_empty(),
            "instances": instances,
        },
        "issues": issues,
    })))
}

/// DEX 助手：OpenClaw agents 概览
pub(super) async fn dex_openclaw_agents_overview_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    let command = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[
                vec!["agents", "list", "--json"],
                vec!["agents", "list"],
                vec!["agent", "agents", "list", "--json"],
                vec!["agent", "agents", "list"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("openclaw", &["agents", "list"], "openclaw 不在 PATH".into())
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw agents", &command, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && command.success,
        "agents": command.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：OpenClaw models 概览
pub(super) async fn dex_openclaw_models_overview_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    let status = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[
                vec!["models", "status", "--json"],
                vec!["models", "status"],
                vec!["models", "list", "--json"],
                vec!["models", "list"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable(
            "openclaw",
            &["models", "status"],
            "openclaw 不在 PATH".into(),
        )
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw models", &status, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && status.success,
        "models": status.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：OpenClaw approvals 概览
pub(super) async fn dex_openclaw_approvals_overview_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("OpenClaw", "openclaw", &mut issues);
    let command = if command_exists("openclaw") {
        run_first_successful_readonly_command(
            "openclaw",
            &[
                vec!["approvals", "get", "--json"],
                vec!["approvals", "get"],
                vec!["exec-policy", "show", "--json"],
                vec!["exec-policy", "show"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable(
            "openclaw",
            &["approvals", "get"],
            "openclaw 不在 PATH".into(),
        )
    };
    if command_exists("openclaw") {
        add_failed_command_issue("OpenClaw", "OpenClaw approvals", &command, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && command.success,
        "approvals": command.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：Hermes 环境概览
pub(super) async fn dex_hermes_env_overview_impl() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let config_dir = home.join(".hermes");
    let mut issues = Vec::new();
    issue_if_missing_binary("Hermes", "hermes", &mut issues);
    if !config_dir.exists() {
        tracing::warn!(path = %config_dir.display(), "Hermes 配置目录不存在");
        issues.push("~/.hermes 配置目录不存在".to_string());
    }
    let doctor = if command_exists("hermes") {
        run_first_successful_readonly_command("hermes", &[vec!["doctor", "--json"], vec!["doctor"]])
            .await
    } else {
        ReadonlyCliResult::unavailable("hermes", &["doctor"], "hermes 不在 PATH".into())
    };
    if command_exists("hermes") {
        add_failed_command_issue("Hermes", "Hermes doctor", &doctor, &mut issues);
    }
    let instances = detect_process_instances("hermes");
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty(),
        "installed": command_exists("hermes"),
        "version": get_cli_version_flexible("hermes"),
        "config_dir": config_dir.to_string_lossy(),
        "config_dir_exists": config_dir.exists(),
        "config": config_dir_summary(
            &config_dir,
            &["config.json", "settings.json", "mcp.json", "skills.json", "gateway.json"]
        ),
        "doctor": doctor.to_value(),
        "process": {
            "running": !instances.is_empty(),
            "instances": instances,
        },
        "issues": issues,
    })))
}

/// DEX 助手：Hermes doctor 检查
pub(super) async fn dex_hermes_doctor_check_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("Hermes", "hermes", &mut issues);
    let command = if command_exists("hermes") {
        run_first_successful_readonly_command("hermes", &[vec!["doctor", "--json"], vec!["doctor"]])
            .await
    } else {
        ReadonlyCliResult::unavailable("hermes", &["doctor"], "hermes 不在 PATH".into())
    };
    if command_exists("hermes") {
        add_failed_command_issue("Hermes", "Hermes doctor", &command, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && command.success,
        "doctor": command.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：Hermes skills 概览
pub(super) async fn dex_hermes_skills_overview_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("Hermes", "hermes", &mut issues);
    let command = if command_exists("hermes") {
        run_first_successful_readonly_command(
            "hermes",
            &[vec!["skills", "list", "--json"], vec!["skills", "list"]],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("hermes", &["skills", "list"], "hermes 不在 PATH".into())
    };
    if command_exists("hermes") {
        add_failed_command_issue("Hermes", "Hermes skills", &command, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && command.success,
        "skills": command.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：Hermes 配置概览
pub(super) async fn dex_hermes_config_overview_impl() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let config_dir = home.join(".hermes");
    let config_paths = [
        config_dir.join("config.json"),
        config_dir.join("settings.json"),
        config_dir.join("mcp.json"),
        config_dir.join("gateway.json"),
    ];
    let (config_files, mcp_servers, mut issues) = inspect_json_config_files(&config_paths);
    if config_files.is_empty() {
        tracing::warn!(path = %config_dir.display(), "未找到 Hermes JSON 配置");
        issues.push("未找到 Hermes JSON 配置".to_string());
    }
    issue_if_missing_binary("Hermes", "hermes", &mut issues);
    let command = if command_exists("hermes") {
        run_first_successful_readonly_command(
            "hermes",
            &[vec!["config", "get", "--json"], vec!["config", "get"]],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("hermes", &["config", "get"], "hermes 不在 PATH".into())
    };
    if command_exists("hermes") {
        add_failed_command_issue("Hermes", "Hermes config", &command, &mut issues);
    }
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty(),
        "config_dir": config_dir.to_string_lossy(),
        "config_files": config_files,
        "mcp_servers": mcp_servers,
        "command": command.to_value(),
        "issues": issues,
    })))
}

/// DEX 助手：Hermes Gateway 概览
pub(super) async fn dex_hermes_gateway_overview_impl() -> Result<Value, String> {
    let mut issues = Vec::new();
    issue_if_missing_binary("Hermes", "hermes", &mut issues);
    let command = if command_exists("hermes") {
        run_first_successful_readonly_command(
            "hermes",
            &[
                vec!["gateway", "status", "--json"],
                vec!["gateway", "status"],
                vec!["gateway", "health", "--json"],
                vec!["gateway", "health"],
            ],
        )
        .await
    } else {
        ReadonlyCliResult::unavailable("hermes", &["gateway", "status"], "hermes 不在 PATH".into())
    };
    if command_exists("hermes") {
        add_failed_command_issue("Hermes", "Hermes Gateway", &command, &mut issues);
    }
    let instances = detect_process_instances("hermes");
    Ok(mask_sensitive_value(json!({
        "ok": issues.is_empty() && (command.success || !instances.is_empty()),
        "gateway": command.to_value(),
        "process": {
            "running": !instances.is_empty(),
            "instances": instances,
        },
        "issues": issues,
    })))
}

/// DEX 助手：AI 工具链总览
pub(super) async fn dex_ai_toolchain_overview_impl(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let active_account = store
        .active_id
        .as_ref()
        .and_then(|id| store.accounts.iter().find(|a| &a.id == id))
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "provider": a.provider,
                "upstream": a.upstream,
                "model_count": a.model_map.len(),
            })
        });
    let tools = super::dex::all_tool_defs(&manager).await;
    let plugin_tool_count = tools.iter().filter(|tool| tool.source == "plugin").count();
    let claude = dex_claude_env_overview_impl()?;
    let openclaw = dex_openclaw_env_overview_impl().await?;
    let hermes = dex_hermes_env_overview_impl().await?;
    let hermes_config = dex_hermes_config_overview_impl().await?;
    let codex = json!({
        "installed": super::dex::codex_is_installed(),
        "version": get_cli_version_flexible("codex"),
        "config_path": super::dex::codex_config_path().map(|p| p.to_string_lossy().to_string()),
    });
    let processes =
        super::dex_process::dex_detect_processes_impl().unwrap_or_else(|e| json!({ "error": e }));
    let mut blockers = Vec::new();
    if active_account.is_none() {
        blockers.push("未配置活跃模型账号".to_string());
    }
    for (label, value) in [
        ("Claude", &claude),
        ("OpenClaw", &openclaw),
        ("Hermes", &hermes),
    ] {
        if value
            .get("installed")
            .and_then(Value::as_bool)
            .map(|v| !v)
            .unwrap_or(false)
        {
            blockers.push(format!("{label} CLI 未安装或不在 PATH"));
        }
    }
    let claude_servers = claude
        .pointer("/mcp/mcp_servers")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let openclaw_servers = openclaw
        .pointer("/mcp/mcp_servers")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let hermes_servers = hermes
        .pointer("/config/mcp_servers")
        .cloned()
        .or_else(|| hermes_config.pointer("/mcp_servers").cloned())
        .unwrap_or_else(|| json!([]));

    Ok(mask_sensitive_value(json!({
        "ok": blockers.is_empty(),
        "codex": codex,
        "claude": claude,
        "openclaw": openclaw,
        "hermes": hermes,
        "hermes_config": hermes_config,
        "mcp": {
            "claude_servers": claude_servers,
            "openclaw_servers": openclaw_servers,
            "hermes_servers": hermes_servers,
        },
        "accounts": {
            "count": store.accounts.len(),
            "active": active_account,
        },
        "plugins": {
            "dex_tool_count": plugin_tool_count,
        },
        "processes": processes,
        "blockers": blockers,
    })))
}
