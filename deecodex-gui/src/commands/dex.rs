use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{Emitter, State};
use tracing;

type DexAccountInfo = (
    String,
    String,
    HashMap<String, String>,
    String,
    deecodex::providers::ProviderProfile,
);

use crate::commands::load_args;
use crate::ServerManager;

use super::dex_plugins::{execute_plugin_tool, plugin_tool_defs};
use super::dex_registry::{
    builtin_tool_defs, capability_meta, is_capability_enabled, load_capability_states,
    save_capability_states, DexCapability, DexToolDef,
};
use super::dex_security::{has_dangerous_shell_pattern, mask_sensitive_value};
use deecodex::request_history::HistoryFilter;

// ── 执行辅助函数 ──────────────────────────────────────────────────────────

/// 获取 Codex config.toml 的路径（等效于 deecodex::codex_config::codex_config_path）
fn codex_config_path() -> Option<PathBuf> {
    deecodex::config::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

/// 检测 Codex 是否已安装（等效于 deecodex::codex_config::codex_is_installed）
fn codex_is_installed() -> bool {
    if let Some(home) = deecodex::config::home_dir() {
        if home.join(".codex").exists() {
            return true;
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let exe = dir.join("codex");
            if exe.exists() {
                return true;
            }
        }
    }
    false
}

/// 获取活跃账号及 provider profile 信息
fn get_active_account_info(data_dir: &std::path::Path) -> Option<DexAccountInfo> {
    let store = deecodex::accounts::load_accounts(data_dir);
    let active = store
        .active_id
        .as_ref()
        .and_then(|id| store.accounts.iter().find(|a| &a.id == id))?;
    let profile = deecodex::providers::profile_for_account(active);

    Some((
        active.upstream.clone(),
        active.api_key.clone(),
        active.model_map.clone(),
        active.provider.clone(),
        profile,
    ))
}

fn canonicalize_for_allowlist(path: &std::path::Path) -> Option<PathBuf> {
    match path.canonicalize() {
        Ok(p) => Some(p),
        Err(_) => {
            let parent = path.parent()?;
            parent
                .canonicalize()
                .ok()
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
        }
    }
}

fn is_under_allowed_root(path: &std::path::Path, root: &std::path::Path) -> bool {
    let Some(root) = canonicalize_for_allowlist(root) else {
        return false;
    };
    path.starts_with(root)
}

/// 安全路径检查：只允许数据目录、常见客户端配置目录和系统临时目录。
fn is_path_allowed(path: &std::path::Path) -> bool {
    let Some(canonical) = canonicalize_for_allowlist(path) else {
        return false;
    };

    let args = load_args();
    if is_under_allowed_root(&canonical, &args.data_dir)
        || is_under_allowed_root(&canonical, &std::env::temp_dir())
        || is_under_allowed_root(&canonical, Path::new("/tmp"))
    {
        return true;
    }

    if let Some(home) = deecodex::config::home_dir() {
        return [".deecodex", ".codex", ".claude", ".openclaw", ".hermes"]
            .iter()
            .map(|dir| home.join(dir))
            .any(|root| is_under_allowed_root(&canonical, &root));
    }

    false
}

fn req_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("缺少参数: {key}"))
}

fn opt_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn opt_usize(args: &Value, key: &str) -> Option<usize> {
    args.get(key).and_then(Value::as_u64).map(|v| v as usize)
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(Value::as_u64)
}

fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn parse_gui_config(value: &Value) -> Result<crate::commands::GuiConfig, String> {
    if let Some(raw) = value.get("config_json").and_then(Value::as_str) {
        serde_json::from_str(raw).map_err(|e| format!("解析 config_json 失败: {e}"))
    } else if let Some(config) = value.get("config") {
        serde_json::from_value(config.clone()).map_err(|e| format!("解析 config 失败: {e}"))
    } else {
        Err("缺少参数: config_json".to_string())
    }
}

async fn all_tool_defs(manager: &ServerManager) -> Vec<DexToolDef> {
    let mut defs = builtin_tool_defs();
    defs.extend(plugin_tool_defs(manager).await);
    defs
}

/// DEX 2.0：列出能力包。
#[tauri::command]
pub async fn dex_list_capabilities(
    manager: State<'_, ServerManager>,
) -> Result<Vec<DexCapability>, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let states = load_capability_states(&data_dir);
    let tools = all_tool_defs(&manager).await;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for tool in &tools {
        *counts.entry(tool.capability.clone()).or_insert(0) += 1;
    }

    let ordered = [
        "core.system",
        "core.workspace",
        "ai.codex",
        "ai.claude",
        "ai.openclaw",
        "ai.hermes",
        "ai.mcp",
        "deecodex.ops",
        "plugins.dynamic",
    ];
    let mut ids: Vec<String> = ordered.iter().map(|s| s.to_string()).collect();
    for id in counts.keys() {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }

    Ok(ids
        .into_iter()
        .map(|id| {
            let (label, description) = capability_meta(&id);
            DexCapability {
                enabled: is_capability_enabled(&states, &id),
                tool_count: counts.get(&id).copied().unwrap_or(0),
                id,
                label: label.to_string(),
                description: description.to_string(),
            }
        })
        .collect())
}

/// DEX 2.0：列出可暴露给 LLM 的工具定义。
#[tauri::command]
pub async fn dex_list_tools(
    manager: State<'_, ServerManager>,
    capability_ids: Option<Vec<String>>,
) -> Result<Vec<DexToolDef>, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let states = load_capability_states(&data_dir);
    let selected: Option<HashSet<String>> = capability_ids.map(|ids| ids.into_iter().collect());
    let mut tools = all_tool_defs(&manager).await;
    tools.retain(|tool| {
        let selected_match = selected
            .as_ref()
            .map(|ids| ids.contains(&tool.capability))
            .unwrap_or(true);
        selected_match && is_capability_enabled(&states, &tool.capability)
    });
    Ok(tools)
}

/// DEX 2.0：启停能力包。
#[tauri::command]
pub async fn dex_update_capability_state(
    manager: State<'_, ServerManager>,
    capability_id: String,
    enabled: bool,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut states = load_capability_states(&data_dir);
    states.insert(capability_id.clone(), enabled);
    save_capability_states(&data_dir, &states)?;
    Ok(json!({ "ok": true, "capability_id": capability_id, "enabled": enabled }))
}

fn command_first_line(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let text = if stdout.trim().is_empty() {
                stderr.as_ref()
            } else {
                stdout.as_ref()
            };
            text.lines().next().unwrap_or("").trim().to_string()
        })
        .filter(|s| !s.is_empty())
}

const DEX_CLI_TIMEOUT_SECS: u64 = 8;
const DEX_CLI_OUTPUT_LIMIT: usize = 12_000;

#[derive(Debug, Clone)]
struct ReadonlyCliResult {
    binary: String,
    args: Vec<String>,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    spawn_error: Option<String>,
}

impl ReadonlyCliResult {
    fn unavailable(binary: &str, args: &[&str], error: String) -> Self {
        Self {
            binary: binary.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            spawn_error: Some(error),
        }
    }

    fn timed_out(binary: &str, args: &[&str]) -> Self {
        Self {
            binary: binary.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            spawn_error: None,
        }
    }

    fn command_line(&self) -> String {
        let mut parts = vec![self.binary.clone()];
        parts.extend(self.args.clone());
        parts.join(" ")
    }

    fn json_output(&self) -> Option<Value> {
        parse_json_output(&self.stdout)
    }

    fn to_value(&self) -> Value {
        json!({
            "command": self.command_line(),
            "success": self.success,
            "exit_code": self.exit_code,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "timed_out": self.timed_out,
            "error": self.spawn_error,
            "json": self.json_output(),
        })
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("\n…(已截断)");
            return out;
        }
        out.push(ch);
    }
    out
}

fn parse_json_output(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    for (start_idx, ch) in trimmed.char_indices() {
        let end_idx = match ch {
            '{' => trimmed.rfind('}'),
            '[' => trimmed.rfind(']'),
            _ => None,
        };
        if let Some(end_idx) = end_idx {
            if end_idx > start_idx {
                if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start_idx..=end_idx]) {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn command_exists(cmd: &str) -> bool {
    if cmd.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(cmd).exists();
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(cmd);
        if candidate.is_file() {
            return true;
        }
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                if dir.join(format!("{cmd}.{ext}")).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

fn get_cli_version_flexible(cmd: &str) -> Option<String> {
    get_cli_version(cmd, &["--version"])
        .or_else(|| get_cli_version(cmd, &["-V"]))
        .or_else(|| get_cli_version(cmd, &["version"]))
}

async fn run_readonly_cli_command(binary: &str, args: &[&str]) -> ReadonlyCliResult {
    let mut command = tokio::process::Command::new(binary);
    command.args(args);
    let command_future = command.output();
    match tokio::time::timeout(Duration::from_secs(DEX_CLI_TIMEOUT_SECS), command_future).await {
        Ok(Ok(output)) => ReadonlyCliResult {
            binary: binary.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: truncate_text(
                &String::from_utf8_lossy(&output.stdout),
                DEX_CLI_OUTPUT_LIMIT,
            ),
            stderr: truncate_text(
                &String::from_utf8_lossy(&output.stderr),
                DEX_CLI_OUTPUT_LIMIT,
            ),
            timed_out: false,
            spawn_error: None,
        },
        Ok(Err(e)) => {
            tracing::warn!(binary, args = ?args, error = %e, "DEX 只读 CLI 命令启动失败");
            ReadonlyCliResult::unavailable(binary, args, e.to_string())
        }
        Err(_) => {
            tracing::warn!(
                binary,
                args = ?args,
                timeout_secs = DEX_CLI_TIMEOUT_SECS,
                "DEX 只读 CLI 命令超时"
            );
            ReadonlyCliResult::timed_out(binary, args)
        }
    }
}

async fn run_first_successful_readonly_command(
    binary: &str,
    command_sets: &[Vec<&str>],
) -> ReadonlyCliResult {
    let mut last: Option<ReadonlyCliResult> = None;
    for args in command_sets {
        let result = run_readonly_cli_command(binary, args).await;
        if result.success {
            return result;
        }
        last = Some(result);
    }
    last.unwrap_or_else(|| ReadonlyCliResult::unavailable(binary, &[], "没有可执行命令".into()))
}

fn inspect_json_config_files(paths: &[PathBuf]) -> (Vec<Value>, Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut servers = HashSet::new();
    let mut issues = Vec::new();

    for path in paths {
        if !path.exists() {
            continue;
        }
        let mut file = json!({
            "path": path.to_string_lossy(),
            "exists": true,
            "valid_json": false,
        });
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(value) => {
                    let file_servers = extract_mcp_servers(&value);
                    for server in &file_servers {
                        servers.insert(server.clone());
                    }
                    file["valid_json"] = json!(true);
                    file["top_level_keys"] = json!(top_level_keys(&value));
                    file["mcp_servers"] = json!(file_servers);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "DEX AI 工具配置 JSON 解析失败");
                    issues.push(format!("{} 格式无效", path.display()));
                    file["error"] = json!(e.to_string());
                }
            },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "DEX AI 工具配置读取失败");
                issues.push(format!("{} 读取失败", path.display()));
                file["error"] = json!(e.to_string());
            }
        }
        files.push(file);
    }

    let mut server_list: Vec<String> = servers.into_iter().collect();
    server_list.sort();
    (files, server_list, issues)
}

fn top_level_keys(value: &Value) -> Vec<String> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

fn extract_mcp_servers(value: &Value) -> Vec<String> {
    let mut servers = Vec::new();
    if let Some(map) = value.get("mcpServers").and_then(Value::as_object) {
        servers.extend(map.keys().cloned());
    }
    if let Some(map) = value
        .get("mcp")
        .and_then(|v| v.get("servers"))
        .and_then(Value::as_object)
    {
        servers.extend(map.keys().cloned());
    }
    if let Some(arr) = value
        .get("mcp")
        .and_then(|v| v.get("servers"))
        .and_then(Value::as_array)
    {
        for item in arr {
            if let Some(name) = item
                .get("name")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                servers.push(name.to_string());
            }
        }
    }
    servers.sort();
    servers.dedup();
    servers
}

fn config_dir_summary(dir: &Path, expected_files: &[&str]) -> Value {
    let files: Vec<Value> = expected_files
        .iter()
        .map(|name| {
            let path = dir.join(name);
            json!({
                "name": name,
                "path": path.to_string_lossy(),
                "exists": path.exists(),
            })
        })
        .collect();
    json!({
        "path": dir.to_string_lossy(),
        "exists": dir.exists(),
        "files": files,
    })
}

fn issue_if_missing_binary(tool_name: &str, binary: &str, issues: &mut Vec<String>) {
    if !command_exists(binary) {
        tracing::warn!(
            tool = tool_name,
            binary,
            "DEX AI 工具 CLI 未安装或不在 PATH"
        );
        issues.push(format!("{tool_name} CLI 未安装或不在 PATH: {binary}"));
    }
}

fn command_problem_summary(result: &ReadonlyCliResult) -> String {
    if result.timed_out {
        return "命令超时".to_string();
    }
    if let Some(error) = &result.spawn_error {
        return error.clone();
    }
    let stderr = result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.lines().next().unwrap_or(stderr).to_string();
    }
    format!("退出码 {:?}", result.exit_code)
}

fn add_failed_command_issue(
    tool_name: &str,
    label: &str,
    result: &ReadonlyCliResult,
    issues: &mut Vec<String>,
) {
    if result.success {
        return;
    }
    let summary = command_problem_summary(result);
    tracing::warn!(
        tool = tool_name,
        command = %result.command_line(),
        error = %summary,
        "DEX AI 工具只读命令未返回成功状态"
    );
    issues.push(format!("{label} 获取失败: {summary}"));
}

fn mcp_servers_from_command(result: &ReadonlyCliResult) -> Vec<String> {
    let Some(value) = result.json_output() else {
        return Vec::new();
    };
    extract_mcp_servers(&value)
}

fn detect_project_type(cwd: &std::path::Path) -> Vec<String> {
    let mut types = Vec::new();
    if cwd.join("Cargo.toml").exists() {
        types.push("rust".to_string());
    }
    if cwd.join("package.json").exists() {
        types.push("node".to_string());
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        types.push("python".to_string());
    }
    if cwd.join("tauri.conf.json").exists() || cwd.join("src-tauri").exists() {
        types.push("tauri".to_string());
    }
    types
}

fn git_summary(cwd: &std::path::Path) -> Value {
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let status = std::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .take(40)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "is_repo": branch.is_some(),
        "branch": branch,
        "dirty_count": status.len(),
        "status_preview": status,
    })
}

/// DEX 2.0：获取 AI 工具工作台上下文摘要。
#[tauri::command]
pub async fn dex_get_workspace_context(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("获取当前目录失败: {e}"))?;
    let args = load_args();
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
                "client_kind": a.client_kind,
                "target": if a.client_kind.is_codex() { "codex_proxy" } else { "client_config" },
                "upstream": a.upstream,
                "model_count": a.model_map.len(),
            })
        });
    let config_files = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        "tauri.conf.json",
        "AGENTS.md",
        "CLAUDE.md",
        "WORKTREES.md",
    ];
    let present_files: Vec<String> = config_files
        .iter()
        .filter(|name| cwd.join(name).exists())
        .map(|s| s.to_string())
        .collect();
    let ports = dex_detect_ports().unwrap_or_else(|e| json!({ "error": e }));
    Ok(mask_sensitive_value(json!({
        "cwd": cwd.to_string_lossy(),
        "data_dir": args.data_dir.to_string_lossy(),
        "project_types": detect_project_type(&cwd),
        "config_files": present_files,
        "git": git_summary(&cwd),
        "cli_versions": {
            "codex": get_cli_version("codex", &["--version"]),
            "claude": get_cli_version("claude", &["--version"]),
            "openclaw": get_cli_version_flexible("openclaw"),
            "hermes": get_cli_version_flexible("hermes"),
            "node": get_cli_version("node", &["--version"]),
            "cargo": command_first_line("cargo", &["--version"]),
            "git": command_first_line("git", &["--version"]),
        },
        "active_account": active_account,
        "ports": ports,
    })))
}

fn active_account_id(manager_data_dir: &std::path::Path) -> Option<String> {
    let store = deecodex::accounts::load_accounts(manager_data_dir);
    store.active_id
}

/// DEX 2.0：统一工具执行入口。前端只负责确认与展示，后端负责路由、策略和脱敏。
#[tauri::command]
pub async fn dex_execute_tool(
    manager: State<'_, ServerManager>,
    name: String,
    args: Option<Value>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    let args = args.unwrap_or_else(|| json!({}));
    let tool = all_tool_defs(&manager)
        .await
        .into_iter()
        .find(|t| t.name == name)
        .ok_or_else(|| format!("未知工具: {name}"))?;

    let data_dir = manager.data_dir.lock().await.clone();
    let states = load_capability_states(&data_dir);
    if !is_capability_enabled(&states, &tool.capability) {
        return Err(format!(
            "能力包 '{}' 已停用，拒绝执行工具 {}",
            tool.capability, tool.name
        ));
    }

    if tool.level >= 3 && confirmed != Some(true) {
        return Err(format!(
            "安全限制：{} 是 L{} 操作，必须先确认",
            tool.name, tool.level
        ));
    }

    let result = if tool.source == "plugin" {
        execute_plugin_tool(&manager, &tool, args.clone()).await?
    } else {
        match tool.name.as_str() {
            "get_service_status" => {
                serde_json::to_value(crate::commands::get_service_status(manager).await?)
                    .unwrap_or_default()
            }
            "start_service" => {
                serde_json::to_value(crate::commands::start_service_inner(&manager).await?)
                    .unwrap_or_default()
            }
            "stop_service" => {
                serde_json::to_value(crate::commands::stop_service_inner(&manager).await?)
                    .unwrap_or_default()
            }
            "launch_codex_cdp" => {
                crate::commands::launch_codex_cdp(manager)?;
                json!({ "ok": true })
            }
            "stop_codex_cdp" => {
                crate::commands::stop_codex_cdp()?;
                json!({ "ok": true })
            }
            "get_config" => {
                serde_json::to_value(crate::commands::get_config()?).unwrap_or_default()
            }
            "save_config" => {
                crate::commands::save_config_without_runtime(parse_gui_config(&args)?)?;
                json!({ "ok": true })
            }
            "validate_config" => json!(crate::commands::validate_config(parse_gui_config(&args)?)),
            "run_diagnostics" => {
                let cfg = args
                    .get("config")
                    .map(|v| {
                        serde_json::from_value(v.clone())
                            .map_err(|e| format!("解析 config 失败: {e}"))
                    })
                    .transpose()?
                    .unwrap_or(crate::commands::get_config()?);
                crate::commands::run_diagnostics(cfg)
            }
            "run_full_diagnostics" => {
                let cfg = args
                    .get("config")
                    .map(|v| {
                        serde_json::from_value(v.clone())
                            .map_err(|e| format!("解析 config 失败: {e}"))
                    })
                    .transpose()?
                    .unwrap_or(crate::commands::get_config()?);
                crate::commands::run_full_diagnostics(cfg).await?
            }
            "list_accounts" => crate::commands::list_accounts(manager).await?,
            "get_active_account" => crate::commands::get_active_account(manager).await?,
            "add_account" => {
                crate::commands::add_account(
                    manager,
                    req_string(&args, "provider")?,
                    opt_string(&args, "account_json"),
                )
                .await?
            }
            "update_account" => {
                crate::commands::update_account(manager, req_string(&args, "account_json")?).await?
            }
            "delete_account" => {
                crate::commands::delete_account(manager, req_string(&args, "id")?).await?
            }
            "switch_account" => {
                crate::commands::switch_account_inner(&manager, req_string(&args, "id")?).await?
            }
            "import_codex_config" => crate::commands::import_codex_config(manager).await?,
            "get_provider_presets" => crate::commands::get_provider_presets()?,
            "get_client_profiles" => crate::commands::get_client_profiles()?,
            "client_lifecycle_status" => {
                dex_client_lifecycle_status(manager, req_string(&args, "kind")?).await?
            }
            "install_client" => dex_install_client(req_string(&args, "kind")?)?,
            "launch_client" => {
                dex_launch_client(req_string(&args, "kind")?, opt_string(&args, "cwd"))?
            }
            "quick_configure_client" => {
                crate::commands::dex_quick_configure_client(
                    manager,
                    req_string(&args, "kind")?,
                    opt_string(&args, "surface"),
                    req_string(&args, "account_json")?,
                )
                .await?
            }
            "get_thread_sources" => crate::commands::get_thread_sources(manager).await?,
            "list_client_threads" => crate::commands::list_client_threads(manager).await?,
            "get_client_thread_content" => {
                crate::commands::get_client_thread_content(
                    req_string(&args, "client_kind")?,
                    req_string(&args, "native_id")?,
                )
                .await?
            }
            "get_client_status" => {
                crate::commands::get_client_status(manager, req_string(&args, "account_id")?)
                    .await?
            }
            "refresh_client_status" => {
                crate::commands::refresh_client_status(manager, req_string(&args, "account_id")?)
                    .await?
            }
            "list_client_backups" => {
                crate::commands::list_client_backups(manager, req_string(&args, "account_id")?)
                    .await?
            }
            "restore_client_backup" => {
                crate::commands::restore_client_backup(
                    manager,
                    req_string(&args, "account_id")?,
                    req_string(&args, "backup_path")?,
                )
                .await?
            }
            "open_client_config" => {
                crate::commands::open_client_config(manager, req_string(&args, "account_id")?)
                    .await?
            }
            "get_account_config_file" => {
                crate::commands::get_account_config_file(manager, req_string(&args, "account_id")?)
                    .await?
            }
            "validate_account_config_file" => {
                crate::commands::validate_account_config_file(
                    manager,
                    req_string(&args, "account_id")?,
                    req_string(&args, "content")?,
                )
                .await?
            }
            "save_account_config_file" => {
                crate::commands::save_account_config_file(
                    manager,
                    req_string(&args, "account_id")?,
                    req_string(&args, "content")?,
                )
                .await?
            }
            "test_client_account" => {
                crate::commands::test_client_account(
                    manager,
                    opt_string(&args, "account_json"),
                    opt_string(&args, "account_id"),
                )
                .await?
            }
            "apply_client_account" => {
                crate::commands::apply_client_account(
                    manager,
                    req_string(&args, "account_id")?,
                    args.get("dry_run").and_then(|v| v.as_bool()),
                )
                .await?
            }
            "get_account_events" => {
                crate::commands::get_account_events(
                    manager,
                    opt_string(&args, "account_id"),
                    args.get("limit")
                        .and_then(|v| v.as_u64())
                        .map(|value| value as usize),
                )
                .await?
            }
            "import_client_accounts" => crate::commands::import_client_accounts(manager).await?,
            "fetch_upstream_models" => {
                let data_dir = manager.data_dir.lock().await.clone();
                let account_id =
                    opt_string(&args, "account_id").or_else(|| active_account_id(&data_dir));
                json!(
                    crate::commands::fetch_upstream_models(
                        manager,
                        account_id,
                        opt_string(&args, "upstream"),
                        opt_string(&args, "api_key"),
                        opt_string(&args, "endpoint_kind"),
                    )
                    .await?
                )
            }
            "fetch_balance" => {
                let data_dir = manager.data_dir.lock().await.clone();
                let account_id = opt_string(&args, "account_id")
                    .or_else(|| active_account_id(&data_dir))
                    .ok_or("缺少参数: account_id，且没有活跃账号")?;
                serde_json::to_value(crate::commands::fetch_balance(manager, account_id).await?)
                    .unwrap_or_default()
            }
            "test_upstream_connectivity" => {
                let account_id = if args.get("upstream").is_some() {
                    opt_string(&args, "account_id")
                } else {
                    let data_dir = manager.data_dir.lock().await.clone();
                    opt_string(&args, "account_id").or_else(|| active_account_id(&data_dir))
                };
                crate::commands::test_upstream_connectivity(
                    manager,
                    account_id,
                    opt_string(&args, "upstream"),
                    opt_string(&args, "api_key"),
                    opt_string(&args, "endpoint_kind"),
                )
                .await?
            }
            "list_sessions" => crate::commands::list_sessions(manager).await?,
            "delete_session" => {
                let id = opt_string(&args, "session_id")
                    .or_else(|| opt_string(&args, "id"))
                    .ok_or("缺少参数: session_id")?;
                let session_type =
                    opt_string(&args, "session_type").unwrap_or_else(|| "responses".to_string());
                crate::commands::delete_session(manager, session_type, id).await?
            }
            "undo_delete_session" => {
                let token = opt_string(&args, "undo_token")
                    .or_else(|| opt_string(&args, "id"))
                    .ok_or("缺少参数: undo_token")?;
                crate::commands::undo_delete_session(manager, token).await?
            }
            "get_threads_status" => crate::commands::get_threads_status(manager).await?,
            "list_threads" => crate::commands::list_threads().await?,
            "get_thread_content" => {
                crate::commands::get_thread_content(req_string(&args, "thread_id")?).await?
            }
            "migrate_threads" => crate::commands::migrate_threads(manager).await?,
            "restore_threads" => crate::commands::restore_threads(manager).await?,
            "calibrate_threads" => crate::commands::calibrate_threads(manager).await?,
            "delete_thread" => {
                crate::commands::delete_thread(manager, req_string(&args, "thread_id")?).await?
            }
            "list_request_history" => {
                crate::commands::list_request_history(
                    manager,
                    opt_usize(&args, "limit"),
                    opt_string(&args, "client_kind"),
                    opt_string(&args, "account_id"),
                )
                .await?
            }
            "clear_request_history" => {
                crate::commands::clear_request_history(
                    manager,
                    opt_string(&args, "client_kind"),
                    opt_string(&args, "account_id"),
                )
                .await?
            }
            "get_monthly_stats" => {
                crate::commands::get_monthly_stats(
                    manager,
                    opt_usize(&args, "limit"),
                    opt_string(&args, "client_kind"),
                    opt_string(&args, "account_id"),
                )
                .await?
            }
            "get_request_stats_since" => {
                crate::commands::get_request_stats_since(
                    manager,
                    opt_u64(&args, "since"),
                    opt_string(&args, "client_kind"),
                    opt_string(&args, "account_id"),
                )
                .await?
            }
            "list_plugins" => json!(crate::commands::list_plugins(manager).await?),
            "install_plugin" => {
                crate::commands::install_plugin(manager, opt_string(&args, "path"), None, None)
                    .await?
            }
            "uninstall_plugin" => {
                crate::commands::uninstall_plugin(manager, req_string(&args, "plugin_id")?).await?
            }
            "start_plugin" => {
                crate::commands::start_plugin(manager, req_string(&args, "plugin_id")?).await?
            }
            "stop_plugin" => {
                crate::commands::stop_plugin(manager, req_string(&args, "plugin_id")?).await?
            }
            "update_plugin_config" => {
                let cfg = if let Some(raw) = opt_string(&args, "config_json") {
                    serde_json::from_str(&raw).map_err(|e| format!("解析 config_json 失败: {e}"))?
                } else {
                    args.get("config").cloned().unwrap_or_else(|| json!({}))
                };
                crate::commands::update_plugin_config(manager, req_string(&args, "plugin_id")?, cfg)
                    .await?
            }
            "get_plugin_qrcode" => {
                crate::commands::get_plugin_qrcode(
                    manager,
                    req_string(&args, "plugin_id")?,
                    req_string(&args, "account_id")?,
                )
                .await?
            }
            "plugin_login_cancel" => {
                crate::commands::plugin_login_cancel(
                    manager,
                    req_string(&args, "plugin_id")?,
                    req_string(&args, "account_id")?,
                )
                .await?
            }
            "query_plugin_status" => {
                crate::commands::query_plugin_status(
                    manager,
                    req_string(&args, "plugin_id")?,
                    req_string(&args, "account_id")?,
                )
                .await?
            }
            "start_plugin_account" => {
                crate::commands::start_plugin_account(
                    manager,
                    req_string(&args, "plugin_id")?,
                    req_string(&args, "account_id")?,
                )
                .await?
            }
            "stop_plugin_account" => {
                crate::commands::stop_plugin_account(
                    manager,
                    req_string(&args, "plugin_id")?,
                    req_string(&args, "account_id")?,
                )
                .await?
            }
            "get_logs" => json!(crate::commands::logs::get_logs()),
            "clear_logs" => {
                crate::commands::logs::clear_logs()?;
                json!({ "ok": true })
            }
            "debug_gui_state" => crate::commands::logs::debug_gui_state(),
            "browse_file" => json!(crate::commands::browse_file().await?),
            "check_upgrade" => crate::commands::check_upgrade().await?,
            "run_upgrade" => json!({ "output": crate::commands::run_upgrade()? }),
            "read_file" => {
                dex_read_file(req_string(&args, "path")?, opt_usize(&args, "max_lines"))?
            }
            "list_directory" => dex_list_directory(req_string(&args, "path")?)?,
            "detect_processes" => dex_detect_processes()?,
            "detect_ports" => dex_detect_ports()?,
            "get_env_info" => dex_get_env_info()?,
            "execute_shell" => {
                dex_execute_shell(
                    req_string(&args, "command")?,
                    opt_u64(&args, "timeout_secs"),
                    confirmed,
                )
                .await?
            }
            "search_logs" => dex_search_logs(
                req_string(&args, "query")?,
                opt_usize(&args, "context_lines"),
            )?,
            "get_codex_config_raw" => dex_get_codex_config_raw()?,
            "health_summary" => dex_health_summary(manager).await?,
            "dex_self_check" => dex_self_check(manager).await?,
            "analyze_requests" => dex_analyze_requests(manager).await?,
            "config_backup" => dex_config_backup(
                req_string(&args, "action")?,
                opt_string(&args, "name"),
                confirmed,
            )?,
            "config_restore" => {
                dex_config_backup("restore".into(), opt_string(&args, "name"), confirmed)?
            }
            "config_diff" => dex_config_diff()?,
            "token_cost" => dex_token_cost(manager).await?,
            "speed_test" => dex_speed_test().await?,
            "thread_cleanup" => dex_thread_cleanup(opt_bool(&args, "dry_run"))?,
            "auto_tune" => dex_auto_tune()?,
            "claude_mcp_check" => dex_claude_mcp_check()?,
            "claude_env_overview" => dex_claude_env_overview()?,
            "openclaw_env_overview" => dex_openclaw_env_overview().await?,
            "openclaw_health_check" => dex_openclaw_health_check().await?,
            "openclaw_mcp_check" => dex_openclaw_mcp_check().await?,
            "openclaw_gateway_overview" => dex_openclaw_gateway_overview().await?,
            "openclaw_agents_overview" => dex_openclaw_agents_overview().await?,
            "openclaw_models_overview" => dex_openclaw_models_overview().await?,
            "openclaw_approvals_overview" => dex_openclaw_approvals_overview().await?,
            "hermes_env_overview" => dex_hermes_env_overview().await?,
            "hermes_doctor_check" => dex_hermes_doctor_check().await?,
            "hermes_skills_overview" => dex_hermes_skills_overview().await?,
            "hermes_config_overview" => dex_hermes_config_overview().await?,
            "hermes_gateway_overview" => dex_hermes_gateway_overview().await?,
            "ai_toolchain_overview" => dex_ai_toolchain_overview(manager).await?,
            "network_topology" => dex_network_topology()?,
            "ssl_check" => dex_ssl_check()?,
            "export_report" => dex_export_report()?,
            other => return Err(format!("未实现的内置工具: {other}")),
        }
    };

    Ok(mask_sensitive_value(result))
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────

/// DEX 助手：发送聊天补全请求
/// 当 stream == true 时启用 SSE 流式传输，通过 dex-chat-chunk 事件实时推送每个 chunk。
#[tauri::command]
pub async fn dex_chat(
    manager: State<'_, ServerManager>,
    messages: Vec<Value>,
    tools: Option<Vec<Value>>,
    stream: Option<bool>,
    model: Option<String>,
) -> Result<Value, String> {
    let stream_mode = stream.unwrap_or(false);

    let data_dir = manager.data_dir.lock().await.clone();

    let (upstream, api_key, model_map, provider, profile) = get_active_account_info(&data_dir)
        .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    // 模型映射：优先使用传入的 model，否则用默认 "gpt-5.5"
    let default_model = "gpt-5.5";
    let requested = model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or(default_model);
    let mapped_model = model_map
        .get(requested)
        .cloned()
        .or_else(|| model_map.get(default_model).cloned())
        .or_else(|| model_map.values().next().cloned())
        .unwrap_or_else(|| requested.to_string());

    let base = upstream.trim_end_matches('/');

    let mut chat_req = deecodex::types::ChatRequest {
        model: mapped_model.clone(),
        messages: messages
            .into_iter()
            .filter_map(|m| serde_json::from_value(m).ok())
            .collect(),
        tools: tools.unwrap_or_default(),
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: stream_mode,
        reasoning_effort: None,
        thinking: None,
        tool_choice: None,
        parallel_tool_calls: None,
        response_format: None,
        user: None,
        stream_options: None,
        web_search_options: None,
    };
    deecodex::providers::adapt_chat_request(&profile, &mut chat_req);
    let msg_count = chat_req.messages.len();

    if stream_mode && profile.wire_protocol != deecodex::providers::WireProtocol::ChatCompletions {
        return Err(
            "DEX 助手暂不支持 Anthropic/Gemini 原生协议流式请求，请关闭流式或切换 Chat 兼容供应商"
                .into(),
        );
    }

    let (url, body) = match profile.wire_protocol {
        deecodex::providers::WireProtocol::ChatCompletions => (
            format!("{base}/chat/completions"),
            serde_json::to_value(&chat_req).map_err(|e| format!("序列化请求失败: {e}"))?,
        ),
        deecodex::providers::WireProtocol::AnthropicMessages
        | deecodex::providers::WireProtocol::GeminiNative => {
            let url = deecodex::native_protocols::native_endpoint(
                &profile.wire_protocol,
                &upstream,
                &chat_req.model,
                false,
                &api_key,
            )
            .ok_or_else(|| "当前供应商原生协议尚未接入 DEX 助手".to_string())?;
            let body =
                deecodex::native_protocols::to_native_request(&profile.wire_protocol, &chat_req)
                    .ok_or_else(|| "无法构造原生协议请求".to_string())?;
            (url, body)
        }
        deecodex::providers::WireProtocol::Responses => {
            return Err("DEX 助手不支持 Responses 直连账号，请切换 Chat 兼容或原生供应商".into());
        }
    };

    tracing::info!(
        url = %url,
        provider = %provider,
        profile = %profile.slug,
        protocol = ?profile.wire_protocol,
        model = %mapped_model,
        msg_count,
        stream = stream_mode,
        "dex_chat 发送请求"
    );

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&body);
    for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
        req = req.header(name, value);
    }

    let resp = req.send().await.map_err(|e| {
        tracing::error!(error = %e, "dex_chat 请求失败");
        if e.is_timeout() || e.is_connect() {
            "连接上游超时，请检查网络或上游地址".to_string()
        } else {
            format!("请求失败: {e}")
        }
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_msg = match status.as_u16() {
            401 => "API Key 无效，请检查账号配置".to_string(),
            403 => "API 访问被拒绝，请检查账号权限".to_string(),
            429 => "请求频率过高，请稍后重试".to_string(),
            code if code >= 500 => "上游服务暂时不可用，请稍后重试".to_string(),
            _ => {
                let body = resp.text().await.unwrap_or_default();
                format!("上游返回错误 ({}): {}", status, body)
            }
        };
        return Err(err_msg);
    }

    if stream_mode {
        // ── 流式模式：通过 SSE 逐 chunk 推送到前端 ──
        // 克隆 AppHandle 以便在流循环中使用（避免长期持有锁）
        let app_handle = {
            let guard = manager.app_handle.lock().await;
            guard.clone()
        };

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut finish_reason = String::new();
        let mut usage = Value::Null;

        'stream_loop: while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("流读取失败: {e}"))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break 'stream_loop;
                    }
                    if let Ok(chunk_val) = serde_json::from_str::<Value>(data) {
                        // 收集 usage 和 finish_reason（可能分散在多个 chunk 中）
                        if let Some(fr) = chunk_val["choices"]
                            .as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|c| c["finish_reason"].as_str())
                        {
                            if !fr.is_empty() {
                                finish_reason = fr.to_string();
                            }
                        }
                        if chunk_val.get("usage").is_some() {
                            usage = chunk_val["usage"].clone();
                        }

                        // 推送单个 chunk 到前端
                        if let Some(ref handle) = app_handle {
                            let _ = handle.emit(
                                "dex-chat-chunk",
                                &json!({ "chunk": chunk_val, "done": false }),
                            );
                        }
                    }
                }
            }
        }

        // 发送流结束事件
        if let Some(ref handle) = app_handle {
            let _ = handle.emit("dex-chat-chunk", &json!({ "chunk": null, "done": true }));
        }

        Ok(json!({
            "stream": true,
            "finish_reason": finish_reason,
            "usage": usage,
        }))
    } else {
        let resp_body: Value = resp
            .json()
            .await
            .map_err(|e| format!("解析响应失败: {e}"))?;

        if profile.wire_protocol == deecodex::providers::WireProtocol::ChatCompletions {
            let choice = resp_body["choices"]
                .as_array()
                .and_then(|choices| choices.first())
                .ok_or_else(|| "响应中没有 choices 数据".to_string())?;

            let message = &choice["message"];
            let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

            return Ok(json!({
                "choices": [{
                    "message": message.clone(),
                    "finish_reason": finish_reason,
                }]
            }));
        }

        let chat_resp =
            deecodex::native_protocols::native_response_to_chat(&profile.wire_protocol, resp_body)
                .map_err(|e| format!("解析原生协议响应失败: {e}"))?;
        let message = chat_resp
            .choices
            .first()
            .map(|choice| serde_json::to_value(&choice.message).unwrap_or_else(|_| json!({})))
            .unwrap_or_else(|| json!({"role":"assistant","content":""}));
        Ok(json!({
            "choices": [{
                "message": message,
                "finish_reason": "stop",
            }]
        }))
    }
}

/// DEX 助手：安全读取文件
#[tauri::command]
pub fn dex_read_file(path: String, max_lines: Option<usize>) -> Result<Value, String> {
    let path = PathBuf::from(&path);
    let max_lines = max_lines.unwrap_or(500);

    if !is_path_allowed(&path) {
        return Err(format!(
            "安全限制：只允许读取 deecodex 数据目录、常见客户端配置目录和系统临时目录下的文件，当前路径: {}",
            path.display()
        ));
    }

    let metadata = std::fs::metadata(&path).map_err(|e| format!("无法读取文件信息: {e}"))?;
    let size_bytes = metadata.len();

    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;

    let all_lines: Vec<&str> = content.lines().collect();
    let total_lines = all_lines.len();
    let truncated = total_lines > max_lines;

    let lines: Vec<String> = all_lines
        .into_iter()
        .take(max_lines)
        .map(|s| s.to_string())
        .collect();

    Ok(json!({
        "path": path.to_string_lossy(),
        "lines": lines,
        "total_lines": total_lines,
        "truncated": truncated,
        "size_bytes": size_bytes,
    }))
}

/// DEX 助手：列出目录内容
#[tauri::command]
pub fn dex_list_directory(path: String) -> Result<Value, String> {
    let path = PathBuf::from(&path);

    if !is_path_allowed(&path) {
        return Err(format!(
            "安全限制：只允许读取 deecodex 数据目录、常见客户端配置目录和系统临时目录下的目录，当前路径: {}",
            path.display()
        ));
    }

    if !path.is_dir() {
        return Err(format!("路径不是目录: {}", path.display()));
    }

    let mut entries: Vec<Value> = Vec::new();

    let dir = std::fs::read_dir(&path).map_err(|e| format!("无法读取目录: {e}"))?;

    for entry in dir {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("获取文件类型失败: {e}"))?;
        let is_dir = file_type.is_dir();
        let size_bytes = if is_dir {
            0
        } else {
            entry.metadata().map(|m| m.len()).unwrap_or(0)
        };

        entries.push(json!({
            "name": name,
            "is_dir": is_dir,
            "size_bytes": size_bytes,
        }));
    }

    // 按目录优先、名称排序
    entries.sort_by(|a, b| {
        let a_dir = a["is_dir"].as_bool().unwrap_or(false);
        let b_dir = b["is_dir"].as_bool().unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        })
    });

    Ok(json!({
        "path": path.to_string_lossy(),
        "entries": entries,
    }))
}

/// DEX 助手：检测运行中的进程
#[tauri::command]
pub fn dex_detect_processes() -> Result<Value, String> {
    let targets = [
        "codex", "Codex", "deecodex", "claude", "Claude", "openclaw", "hermes", "node",
    ];
    let mut processes: Vec<Value> = Vec::new();

    for target in &targets {
        let instances = detect_process_instances(target);

        processes.push(json!({
            "process": target,
            "running": !instances.is_empty(),
            "instances": instances,
        }));
    }

    Ok(json!({ "processes": processes }))
}

#[derive(Clone, Copy)]
struct ClientAppSpec {
    kind: &'static str,
    label: &'static str,
    account_kind: &'static str,
    surface: &'static str,
    command: Option<&'static str>,
    launch_command: Option<&'static str>,
    launch_requires_cwd: bool,
    desktop_app: Option<&'static str>,
    desktop_exe: Option<&'static str>,
    process_names: &'static [&'static str],
    mac_desktop_paths: &'static [&'static str],
    download_url: Option<&'static str>,
    mac_install_command: Option<&'static str>,
    windows_install_command: Option<&'static str>,
}

const CLIENT_APP_SPECS: &[ClientAppSpec] = &[
    ClientAppSpec {
        kind: "codex_cli",
        label: "Codex CLI",
        account_kind: "codex",
        surface: "cli",
        command: Some("codex"),
        launch_command: Some("codex"),
        launch_requires_cwd: true,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["codex"],
        mac_desktop_paths: &[],
        download_url: None,
        mac_install_command: Some("npm install -g @openai/codex"),
        windows_install_command: Some("npm install -g @openai/codex"),
    },
    ClientAppSpec {
        kind: "codex_desktop",
        label: "Codex 桌面",
        account_kind: "codex",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        desktop_app: Some("Codex"),
        desktop_exe: Some("Codex.exe"),
        process_names: &["Codex", "codex"],
        mac_desktop_paths: &["/Applications/Codex.app", "~/Applications/Codex.app"],
        download_url: Some("https://developers.openai.com/codex/app"),
        mac_install_command: None,
        windows_install_command: None,
    },
    ClientAppSpec {
        kind: "claude_cli",
        label: "Claude CLI",
        account_kind: "claude_code",
        surface: "cli",
        command: Some("claude"),
        launch_command: Some("claude"),
        launch_requires_cwd: true,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["claude"],
        mac_desktop_paths: &[],
        download_url: None,
        mac_install_command: Some("npm install -g @anthropic-ai/claude-code"),
        windows_install_command: Some(
            "winget install Anthropic.ClaudeCode || npm install -g @anthropic-ai/claude-code",
        ),
    },
    ClientAppSpec {
        kind: "claude_desktop",
        label: "Claude 桌面",
        account_kind: "claude_code",
        surface: "desktop",
        command: None,
        launch_command: None,
        launch_requires_cwd: false,
        desktop_app: Some("Claude"),
        desktop_exe: Some("Claude.exe"),
        process_names: &["Claude", "claude"],
        mac_desktop_paths: &["/Applications/Claude.app", "~/Applications/Claude.app"],
        download_url: Some("https://claude.ai/download"),
        mac_install_command: None,
        windows_install_command: None,
    },
    ClientAppSpec {
        kind: "openclaw",
        label: "OpenClaw",
        account_kind: "openclaw",
        surface: "cli",
        command: Some("openclaw"),
        launch_command: Some("openclaw tui"),
        launch_requires_cwd: false,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["openclaw"],
        mac_desktop_paths: &[],
        download_url: Some("https://docs.openclaw.ai/install"),
        mac_install_command: Some(
            "curl -fsSL https://openclaw.ai/install.sh | bash -s -- --no-onboard",
        ),
        windows_install_command: Some(
            "& ([scriptblock]::Create((iwr -useb https://openclaw.ai/install.ps1))) -NoOnboard",
        ),
    },
    ClientAppSpec {
        kind: "hermes",
        label: "Hermes",
        account_kind: "hermes",
        surface: "cli",
        command: Some("hermes"),
        launch_command: Some("hermes"),
        launch_requires_cwd: false,
        desktop_app: None,
        desktop_exe: None,
        process_names: &["hermes"],
        mac_desktop_paths: &[],
        download_url: Some("https://www.hermes-ai.net/docs/installation/"),
        mac_install_command: Some("curl -fsSL https://www.hermes-ai.net/install.sh | bash"),
        windows_install_command: Some(
            "python -m pip install --user pipx; python -m pipx install hermes-agent",
        ),
    },
];

fn client_app_spec(kind: &str) -> Option<&'static ClientAppSpec> {
    CLIENT_APP_SPECS.iter().find(|spec| spec.kind == kind)
}

fn client_app_spec_or_err(kind: &str) -> Result<&'static ClientAppSpec, String> {
    client_app_spec(kind).ok_or_else(|| format!("未知客户端: {kind}"))
}

fn client_account_kind_from_slug(kind: &str) -> deecodex::accounts::AccountClientKind {
    match kind {
        "claude_code" => deecodex::accounts::AccountClientKind::ClaudeCode,
        "openclaw" => deecodex::accounts::AccountClientKind::Openclaw,
        "hermes" => deecodex::accounts::AccountClientKind::Hermes,
        "generic_client" => deecodex::accounts::AccountClientKind::GenericClient,
        _ => deecodex::accounts::AccountClientKind::Codex,
    }
}

fn client_surface_from_slug(surface: &str) -> deecodex::accounts::AccountClientSurface {
    match surface {
        "desktop" => deecodex::accounts::AccountClientSurface::Desktop,
        _ => deecodex::accounts::AccountClientSurface::Cli,
    }
}

fn client_spec_is_cli(spec: &ClientAppSpec) -> bool {
    spec.surface == "cli"
}

fn client_cli_status(spec: &ClientAppSpec) -> Value {
    let Some(command) = spec.command else {
        return json!({"installed": false, "command": null, "version": null, "error": "该客户端不是 CLI"});
    };
    let installed = command_exists(command);
    let version = if installed {
        get_cli_version_flexible(command)
            .or_else(|| command_first_line(command, &["--version"]))
            .or_else(|| Some("已安装".into()))
    } else {
        None
    };
    json!({
        "installed": installed,
        "command": command,
        "version": version,
        "error": if installed { Value::Null } else { json!("未在 PATH 中检测到命令") },
    })
}

fn expand_home_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = deecodex::config::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

fn mac_desktop_app_path(spec: &ClientAppSpec) -> Option<PathBuf> {
    spec.mac_desktop_paths
        .iter()
        .map(|path| expand_home_path(path))
        .find(|path| path.exists())
}

fn windows_command_output(cmd: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn windows_desktop_app_candidates(spec: &ClientAppSpec) -> Vec<PathBuf> {
    let Some(exe) = spec.desktop_exe else {
        return Vec::new();
    };
    let app = spec.desktop_app.unwrap_or("").trim();
    let mut out = Vec::new();
    for key in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Ok(base) = std::env::var(key) {
            let base = PathBuf::from(base);
            out.push(base.join("Programs").join(app).join(exe));
            out.push(base.join(app).join(exe));
            out.push(base.join("Anthropic").join(app).join(exe));
            out.push(base.join("OpenAI").join(app).join(exe));
        }
    }
    out
}

fn windows_desktop_app_path(spec: &ClientAppSpec) -> Option<PathBuf> {
    for candidate in windows_desktop_app_candidates(spec) {
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Some(exe) = spec.desktop_exe {
        if let Some(path) = windows_command_output("where", &[exe])
            .and_then(|text| text.lines().next().map(str::trim).map(PathBuf::from))
            .filter(|path| path.exists())
        {
            return Some(path);
        }
        let reg_key = format!(r"HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
        if let Some(text) = windows_command_output("reg", &["query", &reg_key, "/ve"]) {
            for line in text.lines() {
                if let Some((_, value)) = line.split_once("REG_SZ") {
                    let path = PathBuf::from(value.trim().trim_matches('"'));
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }
    }
    None
}

fn client_desktop_status(spec: &ClientAppSpec) -> Value {
    let (installed, path) = if cfg!(target_os = "macos") {
        let path = mac_desktop_app_path(spec);
        (path.is_some(), path)
    } else if cfg!(target_os = "windows") {
        let path = windows_desktop_app_path(spec);
        (path.is_some(), path)
    } else {
        (false, None)
    };
    json!({
        "installed": installed,
        "app": spec.desktop_app,
        "path": path.map(|p| p.to_string_lossy().to_string()),
        "download_url": spec.download_url,
        "error": if installed { Value::Null } else { json!("未检测到桌面版应用") },
    })
}

fn client_process_instances_for_spec(spec: &ClientAppSpec) -> Vec<Value> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in spec.process_names {
        for instance in detect_process_instances(name) {
            let pid = instance
                .get("pid")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !pid.is_empty() && !seen.insert(pid) {
                continue;
            }
            let command = instance
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if spec.kind == "codex_desktop" && !status_command_is_codex_desktop(&command) {
                continue;
            }
            if spec.kind == "codex_cli" && status_command_is_codex_desktop(&command) {
                continue;
            }
            if spec.kind == "claude_desktop" && !status_command_is_claude_desktop(&command) {
                continue;
            }
            if spec.kind == "claude_cli" && status_command_is_claude_desktop(&command) {
                continue;
            }
            out.push(instance);
        }
    }
    out
}

fn status_command_is_codex_desktop(command: &str) -> bool {
    command.contains("/Codex.app/")
        || command.contains("com.openai.codex")
        || command.contains("Codex Helper")
        || command.contains("Application Support/Codex")
}

fn status_command_is_claude_desktop(command: &str) -> bool {
    command.contains("/Claude.app/")
        || command.contains("Claude Helper")
        || command.contains("Application Support/Claude")
}

fn install_command_for_current_os(spec: &ClientAppSpec) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        spec.mac_install_command
    } else if cfg!(target_os = "windows") {
        spec.windows_install_command
    } else {
        None
    }
}

fn install_command_for_status(spec: &ClientAppSpec) -> Option<&'static str> {
    install_command_for_current_os(spec)
        .or(spec.mac_install_command)
        .or(spec.windows_install_command)
}

fn client_primary_account(
    store: &deecodex::accounts::AccountStore,
    spec: &ClientAppSpec,
) -> Option<deecodex::accounts::Account> {
    let kind = client_account_kind_from_slug(spec.account_kind);
    let surface = client_surface_from_slug(spec.surface);
    let mut matches: Vec<_> = store
        .accounts
        .iter()
        .filter(|account| account.client_kind == kind)
        .filter(|account| {
            if !kind.supports_desktop_surface() {
                return true;
            }
            account.client_surface == surface
        })
        .cloned()
        .collect();
    if matches.is_empty() {
        return None;
    }
    if kind.is_codex() {
        let active_id = store
            .active_account_id
            .as_ref()
            .or(store.active_id.as_ref());
        if let Some(account) = active_id
            .and_then(|id| matches.iter().find(|account| &account.id == id))
            .cloned()
        {
            return Some(account);
        }
        return matches.into_iter().next();
    }
    matches.sort_by_key(|account| account.last_applied_at.unwrap_or(0));
    matches.pop()
}

fn lifecycle_next_action(
    installed: bool,
    account_exists: bool,
    process_running: bool,
) -> &'static str {
    if !installed {
        "install"
    } else if !account_exists {
        "configure"
    } else if process_running {
        "running"
    } else {
        "launch"
    }
}

/// 状态页客户端 Dock：读取单个客户端的一键接入生命周期状态。
#[tauri::command]
pub async fn dex_client_lifecycle_status(
    manager: State<'_, ServerManager>,
    kind: String,
) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    let install = if client_spec_is_cli(spec) {
        client_cli_status(spec)
    } else {
        client_desktop_status(spec)
    };
    let installed = install
        .get("installed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let process_instances = client_process_instances_for_spec(spec);
    let process_running = !process_instances.is_empty();

    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = client_primary_account(&store, spec);
    let status_report = account
        .as_ref()
        .map(deecodex::client_integrations::status)
        .and_then(|report| serde_json::to_value(report).ok());
    let account_exists = account.is_some();
    let account_configured = account
        .as_ref()
        .is_some_and(|account| account.client_kind.is_codex() || account.last_applied_at.is_some());
    let config_ok = status_report
        .as_ref()
        .and_then(|report| report.get("ok"))
        .and_then(Value::as_bool)
        .unwrap_or(account_exists);
    let next_action = lifecycle_next_action(installed, account_exists, process_running);

    Ok(json!({
        "kind": spec.kind,
        "label": spec.label,
        "account_kind": spec.account_kind,
        "surface": spec.surface,
        "installed": installed,
        "install": {
            "command": install_command_for_status(spec),
            "download_url": spec.download_url,
            "mode": if client_spec_is_cli(spec) { "terminal" } else { "download_page" },
        },
        "runtime": {
            "running": process_running,
            "instances": process_instances,
        },
        "launch": {
            "mode": if client_spec_is_cli(spec) { "terminal" } else { "desktop" },
            "requires_cwd": spec.launch_requires_cwd,
        },
        "account": account.as_ref().map(|account| json!({
            "id": account.id,
            "name": account.name,
            "provider": account.provider,
            "client_kind": account.client_kind,
            "client_surface": account.client_surface,
            "last_applied_at": account.last_applied_at,
        })),
        "account_exists": account_exists,
        "account_configured": account_configured,
        "config_ok": config_ok,
        "status_report": status_report,
        "next_action": next_action,
        "cli": client_spec_is_cli(spec),
        "command": spec.command,
        "launch_command": spec.launch_command,
        "desktop_app": spec.desktop_app,
        "installed_detail": install,
    }))
}

/// 状态页客户端 Dock：安装或打开官方下载页。
#[tauri::command]
pub fn dex_install_client(kind: String) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    if client_spec_is_cli(spec) {
        let command = install_command_for_current_os(spec)
            .ok_or_else(|| format!("当前平台暂不支持自动安装 {}", spec.label))?;
        spawn_terminal_command(command, None)?;
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
        }));
    }

    let url = spec
        .download_url
        .ok_or_else(|| format!("{} 没有配置下载地址", spec.label))?;
    open_url_with_system(url)?;
    Ok(json!({
        "ok": true,
        "kind": spec.kind,
        "label": spec.label,
        "mode": "download_page",
        "url": url,
    }))
}

/// 状态页客户端 Dock：启动桌面版或在终端中启动 CLI。
#[tauri::command]
pub fn dex_launch_client(kind: String, cwd: Option<String>) -> Result<Value, String> {
    let spec = client_app_spec_or_err(&kind)?;
    if client_spec_is_cli(spec) {
        let command = spec
            .launch_command
            .or(spec.command)
            .ok_or_else(|| format!("{} 没有 CLI 启动命令", spec.label))?;
        let cwd = if spec.launch_requires_cwd {
            let cwd = cwd
                .map(PathBuf::from)
                .ok_or_else(|| format!("{} 需要先选择启动目录", spec.label))?;
            if !cwd.exists() || !cwd.is_dir() {
                return Err(format!("启动目录不存在或不是目录: {}", cwd.display()));
            }
            Some(cwd)
        } else {
            None
        };
        spawn_terminal_command(command, cwd.as_deref())?;
        return Ok(json!({
            "ok": true,
            "kind": spec.kind,
            "label": spec.label,
            "mode": "terminal",
            "command": command,
            "cwd": cwd.as_ref().map(|path| path.to_string_lossy().to_string()),
            "requires_cwd": spec.launch_requires_cwd,
        }));
    }

    launch_desktop_app(spec)?;
    Ok(json!({
        "ok": true,
        "kind": spec.kind,
        "label": spec.label,
        "mode": "desktop",
        "app": spec.desktop_app,
    }))
}

/// 状态页客户端 Dock：选择 CLI 启动目录。
#[tauri::command]
pub async fn dex_pick_client_launch_dir() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .set_title("选择客户端启动目录")
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().to_string());
    Ok(path)
}

fn open_url_with_system(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("打开下载页失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| format!("打开下载页失败: {e}"))?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| format!("打开下载页失败: {e}"))?;
        Ok(())
    }
}

fn launch_desktop_app(spec: &ClientAppSpec) -> Result<(), String> {
    let app_name = spec
        .desktop_app
        .ok_or_else(|| format!("{} 不是桌面版客户端", spec.label))?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-a")
            .arg(app_name)
            .spawn()
            .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(path) = windows_desktop_app_path(spec) {
            std::process::Command::new(path)
                .spawn()
                .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
            return Ok(());
        }
        std::process::Command::new("cmd")
            .args(["/C", "start", "", app_name])
            .spawn()
            .map_err(|e| format!("打开 {app_name} 失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app_name;
        Err("桌面客户端启动暂只支持 macOS 与 Windows".to_string())
    }
}

fn shell_quote_posix(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(any(target_os = "windows", test))]
fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn mac_terminal_script(command: &str, cwd: Option<&Path>) -> String {
    let shell = if let Some(cwd) = cwd {
        format!(
            "cd {} && {command}",
            shell_quote_posix(&cwd.to_string_lossy())
        )
    } else {
        command.to_string()
    };
    format!(
        "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
        escape_applescript_string(&shell)
    )
}

#[cfg(any(target_os = "windows", test))]
fn windows_terminal_script(command: &str, cwd: Option<&Path>) -> String {
    if let Some(cwd) = cwd {
        format!(
            "Set-Location -LiteralPath {}; {}",
            powershell_single_quote(&cwd.to_string_lossy()),
            command
        )
    } else {
        command.to_string()
    }
}

fn spawn_terminal_command(command: &str, cwd: Option<&Path>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let script = mac_terminal_script(command, cwd);
        std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn()
            .map_err(|e| format!("打开终端失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let script = windows_terminal_script(command, cwd);
        let result = if command_exists("wt") {
            std::process::Command::new("wt")
                .args(["powershell.exe", "-NoExit", "-Command", &script])
                .spawn()
        } else {
            std::process::Command::new("powershell.exe")
                .args(["-NoExit", "-Command", &script])
                .spawn()
        };
        result.map_err(|e| format!("打开终端失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (command, cwd);
        Err("CLI 终端启动暂只支持 macOS 与 Windows".to_string())
    }
}

/// 状态页客户端 Dock：打开或退出桌面版客户端。
#[tauri::command]
pub fn dex_toggle_desktop_client(kind: String, running: bool) -> Result<Value, String> {
    let app_name = match kind.as_str() {
        "codex_desktop" => "Codex",
        "claude_desktop" => "Claude",
        _ => return Err(format!("不支持的桌面客户端: {kind}")),
    };
    let action = if running { "quit" } else { "open" };

    #[cfg(target_os = "macos")]
    {
        let result = if running {
            std::process::Command::new("osascript")
                .arg("-e")
                .arg(format!("quit app \"{app_name}\""))
                .spawn()
        } else {
            std::process::Command::new("open")
                .arg("-a")
                .arg(app_name)
                .spawn()
        };
        result.map_err(|e| {
            format!(
                "{} {} 失败: {e}",
                if running { "退出" } else { "打开" },
                app_name
            )
        })?;
        Ok(json!({
            "ok": true,
            "kind": kind,
            "app": app_name,
            "action": action,
        }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app_name, action);
        Err("桌面客户端开关暂只支持 macOS".to_string())
    }
}

fn detect_process_instances(target: &str) -> Vec<Value> {
    // 尝试 pgrep -a，失败则降级到 pgrep -l
    let output = std::process::Command::new("pgrep")
        .arg("-a")
        .arg(target)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_pgrep_output(&stdout)
        }
        _ => {
            let output = std::process::Command::new("pgrep")
                .arg("-l")
                .arg(target)
                .output();

            match output {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    parse_pgrep_l_output(&stdout)
                }
                _ => Vec::new(),
            }
        }
    }
}

fn parse_pgrep_output(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let pid = parts.next()?.to_string();
            let command = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| process_command_for_pid(&pid))
                .unwrap_or_default();
            Some(json!({ "pid": pid, "command": command }))
        })
        .collect()
}

fn parse_pgrep_l_output(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.to_string();
            let command = process_command_for_pid(&pid)
                .or_else(|| parts.next().map(str::to_string))
                .unwrap_or_default();
            Some(json!({ "pid": pid, "command": command }))
        })
        .collect()
}

fn process_command_for_pid(pid: &str) -> Option<String> {
    let output = std::process::Command::new("ps")
        .arg("-p")
        .arg(pid)
        .arg("-o")
        .arg("command=")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

/// DEX 助手：检测端口占用
#[tauri::command]
pub fn dex_detect_ports() -> Result<Value, String> {
    let args = load_args();
    let mut ports_to_check = vec![4446u16, 9222, 8080, 3000, 8000, 11434];
    // 也检测当前配置的端口
    if !ports_to_check.contains(&args.port) {
        ports_to_check.push(args.port);
    }

    let mut port_results: Vec<Value> = Vec::new();

    for port in &ports_to_check {
        let output = std::process::Command::new("lsof")
            .arg("-i")
            .arg(format!(":{port}"))
            .arg("-P")
            .arg("-n")
            .arg("-t")
            .output();

        let pids: Vec<String> = match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .collect()
            }
            _ => Vec::new(),
        };

        let in_use = !pids.is_empty();

        let processes: Vec<String> = pids
            .iter()
            .map(|pid| {
                let output = std::process::Command::new("ps")
                    .arg("-p")
                    .arg(pid)
                    .arg("-o")
                    .arg("comm=")
                    .output()
                    .ok();
                output
                    .and_then(|o| {
                        if o.status.success() {
                            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "未知".to_string())
            })
            .collect();

        port_results.push(json!({
            "port": port,
            "in_use": in_use,
            "pids": pids,
            "processes": processes,
        }));
    }

    Ok(json!({ "ports": port_results }))
}

/// DEX 助手：收集环境信息
#[tauri::command]
pub fn dex_get_env_info() -> Result<Value, String> {
    let os_type = std::env::consts::OS.to_string();
    let os_version = get_os_version();

    let deecodex_version = env!("CARGO_PKG_VERSION").to_string();

    // Codex 版本
    let codex_version = get_cli_version("codex", &["--version"]);
    let codex_installed = codex_is_installed();

    // Claude 版本
    let claude_version = get_cli_version("claude", &["--version"]);
    let openclaw_version = get_cli_version("openclaw", &["--version"]);
    let hermes_version = get_cli_version("hermes", &["--version"]);

    // Node 版本
    let node_version = get_cli_version("node", &["--version"]);

    // 配置文件存在性
    let args = load_args();
    let config_json_exists = args.data_dir.join("config.json").exists();
    let accounts_json_exists = args.data_dir.join("accounts.json").exists();
    let codex_config_exists = codex_config_path().map(|p| p.exists()).unwrap_or(false);

    Ok(json!({
        "os": {
            "type": os_type,
            "version": os_version,
        },
        "deecodex": {
            "version": deecodex_version,
            "data_dir": args.data_dir.to_string_lossy(),
        },
        "codex": {
            "installed": codex_installed,
            "version": codex_version,
        },
        "claude": {
            "version": claude_version,
        },
        "openclaw": {
            "version": openclaw_version,
        },
        "hermes": {
            "version": hermes_version,
        },
        "node": {
            "version": node_version,
        },
        "config_files": {
            "config_json": config_json_exists,
            "accounts_json": accounts_json_exists,
            "codex_config_toml": codex_config_exists,
        },
    }))
}

fn get_os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "未知".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
            })
            .unwrap_or_else(|| "未知".to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "未知".to_string()
    }
}

fn get_cli_version(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                let text = if stdout.trim().is_empty() {
                    stderr.as_ref()
                } else {
                    stdout.as_ref()
                };
                let version = text.lines().next().unwrap_or("").trim().to_string();
                if version.is_empty() {
                    None
                } else {
                    Some(version)
                }
            } else {
                None
            }
        })
}

/// DEX 助手：安全执行 Shell 命令
#[tauri::command]
pub async fn dex_execute_shell(
    command: String,
    timeout_secs: Option<u64>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    let timeout_secs = timeout_secs.unwrap_or(30);

    if confirmed != Some(true) {
        return Err("安全限制：执行 Shell 命令前必须经过用户确认".to_string());
    }

    if let Some(pattern) = has_dangerous_shell_pattern(&command) {
        return Err(format!("安全限制：禁止执行危险命令 ({pattern})"));
    }

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动命令失败: {e}"))?;

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("命令执行超时 ({timeout_secs}秒)"))?
    .map_err(|e| format!("等待命令结束失败: {e}"))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();

    Ok(json!({
        "command": command,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "success": success,
    }))
}

/// DEX 助手：搜索日志
#[tauri::command]
pub fn dex_search_logs(query: String, context_lines: Option<usize>) -> Result<Value, String> {
    let args = load_args();
    let log_path = args.data_dir.join("deecodex.log");

    if !log_path.exists() {
        return Ok(json!({
            "query": query,
            "matches": 0,
            "truncated": false,
            "results": [],
        }));
    }

    let content =
        std::fs::read_to_string(&log_path).map_err(|e| format!("读取日志文件失败: {e}"))?;

    let ctx = context_lines.unwrap_or(0);
    let query_lower = query.to_lowercase();
    let all_lines: Vec<&str> = content.lines().collect();
    let max_results = 50usize;

    let mut results: Vec<Value> = Vec::new();

    for (i, line) in all_lines.iter().enumerate() {
        if results.len() >= max_results {
            break;
        }
        if line.to_lowercase().contains(&query_lower) {
            // 收集上下文
            let start = i.saturating_sub(ctx);
            let end = (i + ctx + 1).min(all_lines.len());
            let context: Vec<String> = all_lines[start..end]
                .iter()
                .map(|s| s.to_string())
                .collect();

            results.push(json!({
                "line_number": i + 1,
                "line": line,
                "context": context,
            }));
        }
    }

    let total_matches = all_lines
        .iter()
        .filter(|l| l.to_lowercase().contains(&query_lower))
        .count();
    let truncated = total_matches > max_results;

    Ok(json!({
        "query": query,
        "matches": total_matches,
        "truncated": truncated,
        "results": results,
    }))
}

/// DEX 助手：获取 Codex 原始配置
#[tauri::command]
pub fn dex_get_codex_config_raw() -> Result<Value, String> {
    let config_path = codex_config_path();
    let exists = config_path.as_ref().map(|p| p.exists()).unwrap_or(false);

    let (content, size_bytes) = if let Some(ref path) = config_path {
        if exists {
            match std::fs::read_to_string(path) {
                Ok(c) => {
                    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    (c, size)
                }
                Err(_) => {
                    // 尝试处理编码（简化版，主要处理 UTF-8）
                    match std::fs::read(path) {
                        Ok(bytes) => {
                            let size = bytes.len() as u64;
                            let content = String::from_utf8_lossy(&bytes).to_string();
                            (content, size)
                        }
                        Err(e) => return Err(format!("读取配置文件失败: {e}")),
                    }
                }
            }
        } else {
            (String::new(), 0)
        }
    } else {
        (String::new(), 0)
    };

    Ok(json!({
        "exists": exists,
        "path": config_path.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "content": content,
        "size_bytes": size_bytes,
    }))
}

/// DEX 助手：一键健康概览（合并服务状态+账号+异常）
#[tauri::command]
pub async fn dex_health_summary(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let args = load_args();

    // 服务状态
    let running = manager.is_running().await;
    let port = *manager.port.lock().await;

    // 账号信息
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account_count = store.accounts.len();
    let client_counts = {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for account in &store.accounts {
            let key = serde_json::to_value(&account.client_kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "codex".into());
            *counts.entry(key).or_default() += 1;
        }
        counts
    };
    let (account_ok, provider, profile_slug, wire_protocol, capability_labels) =
        if let Some((upstream, api_key, _, provider, profile)) = get_active_account_info(&data_dir)
        {
            let ok = !upstream.is_empty() && !api_key.is_empty();
            let labels = deecodex::providers::capability_labels(&profile);
            (
                ok,
                provider,
                profile.slug,
                format!("{:?}", profile.wire_protocol),
                labels,
            )
        } else {
            (
                false,
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
            )
        };

    // 最近错误计数
    let log_path = args.data_dir.join("deecodex.log");
    let recent_errors = if log_path.exists() {
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.to_lowercase().contains("error") || l.to_lowercase().contains("warn"))
            .count()
    } else {
        0
    };

    // Codex 安装状态
    let codex_ok = codex_is_installed();

    Ok(json!({
        "service": { "running": running, "port": port },
        "account": {
            "ok": account_ok,
            "provider": provider,
            "profile": profile_slug,
            "wire_protocol": wire_protocol,
            "capabilities": capability_labels,
            "count": account_count,
            "client_counts": client_counts
        },
        "codex_installed": codex_ok,
        "recent_errors": recent_errors,
        "data_dir": args.data_dir.to_string_lossy(),
    }))
}

/// DEX 助手：自检 DEX 注册表、能力包、插件工具和最近错误。
#[tauri::command]
pub async fn dex_self_check(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let states = load_capability_states(&data_dir);
    let tools = all_tool_defs(&manager).await;
    let capabilities = dex_list_capabilities(manager.clone()).await?;
    let workspace = dex_get_workspace_context(manager.clone())
        .await
        .unwrap_or_else(|e| json!({ "error": e }));

    let mut level_counts: HashMap<String, usize> = HashMap::new();
    let mut source_counts: HashMap<String, usize> = HashMap::new();
    let mut capability_counts: HashMap<String, usize> = HashMap::new();
    let mut disabled_tool_count = 0usize;
    for tool in &tools {
        *level_counts.entry(format!("L{}", tool.level)).or_default() += 1;
        *source_counts.entry(tool.source.clone()).or_default() += 1;
        *capability_counts
            .entry(tool.capability.clone())
            .or_default() += 1;
        if !is_capability_enabled(&states, &tool.capability) {
            disabled_tool_count += 1;
        }
    }
    let plugin_tools: Vec<Value> = tools
        .iter()
        .filter(|tool| tool.source == "plugin")
        .map(|tool| {
            json!({
                "name": tool.name,
                "capability": tool.capability,
                "level": tool.level,
                "plugin_id": tool.plugin_id,
            })
        })
        .collect();
    let recent_request_errors = match manager.request_history.lock().await.as_ref() {
        Some(store) => store
            .list(50, &HistoryFilter::default())
            .await
            .into_iter()
            .filter(|entry| entry.status != "completed" || !entry.error_msg.is_empty())
            .take(10)
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "status": entry.status,
                    "model": entry.model,
                    "error": entry.error_msg,
                    "duration_ms": entry.duration_ms,
                })
            })
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };

    let mut warnings = Vec::new();
    if tools.is_empty() {
        warnings.push("工具注册表为空".to_string());
    }
    if disabled_tool_count > 0 {
        warnings.push(format!("有 {disabled_tool_count} 个工具属于已停用能力包"));
    }
    if plugin_tools.is_empty() {
        warnings.push("当前没有插件向 DEX 暴露工具".to_string());
    }
    if !recent_request_errors.is_empty() {
        warnings.push(format!("最近有 {} 条请求错误", recent_request_errors.len()));
    }

    Ok(mask_sensitive_value(json!({
        "ok": warnings.is_empty(),
        "warnings": warnings,
        "tool_count": tools.len(),
        "disabled_tool_count": disabled_tool_count,
        "level_counts": level_counts,
        "source_counts": source_counts,
        "capability_counts": capability_counts,
        "capabilities": capabilities,
        "plugin_tools": plugin_tools,
        "recent_request_errors": recent_request_errors,
        "workspace": workspace,
    })))
}

/// DEX 助手：请求历史分析（最近请求统计）
#[tauri::command]
pub async fn dex_analyze_requests(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    let store = rh
        .as_ref()
        .ok_or("请求历史不可用（服务未启动或数据库未初始化）")?;
    let entries = store.list(1000, &HistoryFilter::default()).await;

    let total = entries.len();
    if total == 0 {
        return Ok(json!({ "total": 0, "message": "暂无请求记录" }));
    }

    let mut success = 0u64;
    let mut error = 0u64;
    let mut total_tokens = 0u64;
    let mut durations: Vec<u64> = Vec::new();
    let mut models: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for e in &entries {
        if e.error_msg.is_empty() {
            success += 1;
        } else {
            error += 1;
        }
        total_tokens += e.total_tokens as u64;
        if e.duration_ms > 0 {
            durations.push(e.duration_ms);
        }
        if !e.model.is_empty() {
            *models.entry(e.model.clone()).or_default() += 1;
        }
    }

    let avg_latency = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<u64>() as f64 / durations.len() as f64
    };
    durations.sort();
    let p50 = durations.get(durations.len() / 2).copied().unwrap_or(0);
    let p99 = durations
        .get((durations.len() as f64 * 0.99) as usize)
        .copied()
        .unwrap_or(0);

    let mut top_models: Vec<Value> = models
        .into_iter()
        .map(|(m, c)| json!({ "model": m, "count": c }))
        .collect();
    top_models.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));

    Ok(json!({
        "total": total,
        "success_rate": if total > 0 { (success as f64 / total as f64 * 100.0).round() } else { 0.0 },
        "errors": error,
        "total_tokens": total_tokens,
        "avg_latency_ms": avg_latency.round() as u64,
        "p50_latency_ms": p50,
        "p99_latency_ms": p99,
        "top_models": top_models,
    }))
}

// ── 辅助函数（配置对比/系统检测）────────────────────────────────────────────

/// 从 TOML 文本中提取简单键值
fn extract_toml_value(content: &str, key: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{} = ", key)) {
            return rest.trim().trim_matches('"').to_string();
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{}=", key)) {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    String::new()
}

/// 从 URL 中提取端口号
fn extract_port_from_url(url: &str) -> Option<u16> {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    if let Some(port_str) = host.rsplit(':').next() {
        if port_str != host {
            return port_str.parse().ok();
        }
    }
    None
}

/// 获取系统总内存（GB）
fn get_total_memory_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Ok(bytes) = s.trim().parse::<f64>() {
                    return (bytes / (1024.0 * 1024.0 * 1024.0) * 10.0).round() / 10.0;
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let kb: Vec<&str> = line.split_whitespace().collect();
                    if kb.len() >= 2 {
                        if let Ok(kb_val) = kb[1].parse::<f64>() {
                            return (kb_val / (1024.0 * 1024.0) * 10.0).round() / 10.0;
                        }
                    }
                }
            }
        }
    }
    0.0
}

/// 获取指定路径所在磁盘的剩余空间（GB）
fn get_disk_free_gb(path: &std::path::Path) -> f64 {
    let output = std::process::Command::new("df")
        .arg(path.to_string_lossy().as_ref())
        .arg("-k")
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().nth(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    if let Ok(kb) = cols[3].parse::<f64>() {
                        return kb / (1024.0 * 1024.0);
                    }
                }
            }
        }
    }
    0.0
}

// ── DEX 助手命令（第二部分）─────────────────────────────────────────────────

/// DEX 助手：配置备份/恢复
#[tauri::command]
pub fn dex_config_backup(
    action: String,
    name: Option<String>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    if action == "restore" && confirmed != Some(true) {
        return Err("安全限制：恢复配置会覆盖当前 config.json/accounts.json，必须先确认".into());
    }

    let args = load_args();
    let data_dir = &args.data_dir;
    let backup_dir = data_dir.join("backups");

    match action.as_str() {
        "backup" => {
            let name = name.ok_or("备份名称不能为空")?;
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let dir_name = format!("{}_{}", name, ts);
            let target = backup_dir.join(&dir_name);
            std::fs::create_dir_all(&target).map_err(|e| format!("创建备份目录失败: {e}"))?;

            let mut files = Vec::new();
            for fname in &["config.json", "accounts.json"] {
                let src = data_dir.join(fname);
                if src.exists() {
                    std::fs::copy(&src, target.join(fname))
                        .map_err(|e| format!("备份 {} 失败: {}", fname, e))?;
                    files.push(*fname);
                }
            }

            Ok(json!({
                "ok": true,
                "backup_name": dir_name,
                "files": files,
            }))
        }
        "restore" => {
            let name = name.ok_or("备份名称不能为空")?;
            let source = backup_dir.join(&name);
            if !source.exists() {
                return Err(format!("备份不存在: {}", name));
            }

            for fname in &["config.json", "accounts.json"] {
                let src = source.join(fname);
                if src.exists() {
                    std::fs::copy(&src, data_dir.join(fname))
                        .map_err(|e| format!("恢复 {} 失败: {}", fname, e))?;
                }
            }

            Ok(json!({ "ok": true }))
        }
        "list" => {
            if !backup_dir.exists() {
                return Ok(json!({ "backups": [] }));
            }

            let mut backups = Vec::new();
            let dir =
                std::fs::read_dir(&backup_dir).map_err(|e| format!("读取备份目录失败: {e}"))?;

            for entry in dir {
                let entry = entry.map_err(|e| format!("读取备份条目失败: {e}"))?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let full_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let (base_name, time_secs) = if let Some(pos) = full_name.rfind('_') {
                    let ts = full_name[pos + 1..].parse::<u64>().unwrap_or(0);
                    (full_name[..pos].to_string(), ts)
                } else {
                    (full_name.clone(), 0u64)
                };

                let mut files = Vec::new();
                if path.join("config.json").exists() {
                    files.push("config.json");
                }
                if path.join("accounts.json").exists() {
                    files.push("accounts.json");
                }

                backups.push(json!({
                    "name": full_name,
                    "base_name": base_name,
                    "time": time_secs,
                    "files": files,
                }));
            }

            backups.sort_by_key(|b| std::cmp::Reverse(b["time"].as_u64().unwrap_or(0)));

            Ok(json!({ "backups": backups }))
        }
        _ => Err(format!("未知操作: {}，支持: backup, restore, list", action)),
    }
}

/// DEX 助手：配置差异对比
#[tauri::command]
pub fn dex_config_diff() -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;

    let deecodex_port = args.port;
    let (deecodex_upstream, deecodex_model_count, deecodex_provider) =
        if let Some((up, _, mm, provider, _)) = get_active_account_info(data_dir) {
            (up, mm.len(), provider)
        } else {
            (String::new(), 0, String::new())
        };

    // Codex config.toml 内容
    let codex_toml = codex_config_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .unwrap_or_default();

    let codex_base_url = extract_toml_value(&codex_toml, "base_url");
    let codex_model_provider = extract_toml_value(&codex_toml, "model_provider");
    let codex_port = extract_port_from_url(&codex_base_url);

    let mut diffs = Vec::new();

    if codex_model_provider != "deecodex" {
        let severity = if codex_model_provider.is_empty() {
            "critical"
        } else {
            "warning"
        };
        diffs.push(json!({
            "field": "model_provider",
            "deecodex_value": "deecodex",
            "codex_value": if codex_model_provider.is_empty() {
                "(未设置)"
            } else {
                &codex_model_provider
            },
            "severity": severity,
        }));
    }

    if let Some(cp) = codex_port {
        if cp != deecodex_port {
            diffs.push(json!({
                "field": "port",
                "deecodex_value": deecodex_port,
                "codex_value": cp,
                "severity": "warning",
            }));
        }
    } else if codex_model_provider == "deecodex" && !codex_base_url.is_empty() {
        diffs.push(json!({
            "field": "port",
            "deecodex_value": deecodex_port,
            "codex_value": "(无法解析)",
            "severity": "warning",
        }));
    }

    Ok(json!({
        "deecodex": {
            "port": deecodex_port,
            "upstream": deecodex_upstream,
            "model_count": deecodex_model_count,
            "provider": deecodex_provider,
        },
        "codex": {
            "base_url": codex_base_url,
            "model_provider": codex_model_provider,
            "port": codex_port,
        },
        "diffs": diffs,
    }))
}

/// DEX 助手：Token 费用估算
#[tauri::command]
pub async fn dex_token_cost(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    let store = rh
        .as_ref()
        .ok_or("请求历史不可用（服务未启动或数据库未初始化）")?;
    let entries = store.list(1000, &HistoryFilter::default()).await;

    let total = entries.len();
    if total == 0 {
        return Ok(json!({
            "total_tokens": 0,
            "total_input": 0,
            "total_output": 0,
            "estimated_cost_usd": 0.0,
            "by_model": [],
        }));
    }

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut by_model: HashMap<String, (u64, u64)> = HashMap::new();

    for e in &entries {
        total_input += e.input_tokens as u64;
        total_output += e.output_tokens as u64;
        let entry = by_model.entry(e.model.clone()).or_insert((0, 0));
        entry.0 += e.input_tokens as u64;
        entry.1 += e.output_tokens as u64;
    }

    // 通用定价: input $0.5/1M tokens, output $2/1M tokens
    let input_cost = total_input as f64 / 1_000_000.0 * 0.5;
    let output_cost = total_output as f64 / 1_000_000.0 * 2.0;
    let total_cost = input_cost + output_cost;

    let mut models: Vec<Value> = by_model
        .into_iter()
        .map(|(model, (input, output))| {
            let cost = input as f64 / 1_000_000.0 * 0.5 + output as f64 / 1_000_000.0 * 2.0;
            json!({
                "model": model,
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": input + output,
                "estimated_cost_usd": (cost * 10000.0).round() / 10000.0,
            })
        })
        .collect();
    models.sort_by(|a, b| {
        b["estimated_cost_usd"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["estimated_cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "total_tokens": total_input + total_output,
        "total_input": total_input,
        "total_output": total_output,
        "estimated_cost_usd": (total_cost * 10000.0).round() / 10000.0,
        "by_model": models,
    }))
}

/// DEX 助手：模型速度测试
#[tauri::command]
pub async fn dex_speed_test() -> Result<Value, String> {
    let args = load_args();
    let (upstream, api_key, model_map, provider, profile) = get_active_account_info(&args.data_dir)
        .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    let base = upstream.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let provider_profile = profile.slug.clone();
    let wire_protocol = format!("{:?}", profile.wire_protocol);
    let mut results = Vec::new();
    // 最多测 5 个模型，减少 API 开销
    let model_pairs: Vec<(&String, &String)> = model_map.iter().take(5).collect();

    for (deecodex_model, upstream_model) in &model_pairs {
        let mut chat_req = deecodex::types::ChatRequest {
            model: (*upstream_model).clone(),
            messages: vec![deecodex::types::ChatMessage {
                role: "user".into(),
                content: Some(json!("hi")),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: Some(1),
            stream: false,
            reasoning_effort: None,
            thinking: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };
        deecodex::providers::adapt_chat_request(&profile, &mut chat_req);
        let (url, body) = match profile.wire_protocol {
            deecodex::providers::WireProtocol::ChatCompletions => (
                format!("{base}/chat/completions"),
                serde_json::to_value(&chat_req).unwrap_or_else(|_| json!({})),
            ),
            deecodex::providers::WireProtocol::AnthropicMessages
            | deecodex::providers::WireProtocol::GeminiNative => {
                let url = deecodex::native_protocols::native_endpoint(
                    &profile.wire_protocol,
                    &upstream,
                    &chat_req.model,
                    false,
                    &api_key,
                )
                .unwrap_or_else(|| format!("{base}/chat/completions"));
                let body = deecodex::native_protocols::to_native_request(
                    &profile.wire_protocol,
                    &chat_req,
                )
                .unwrap_or_else(|| json!({}));
                (url, body)
            }
            deecodex::providers::WireProtocol::Responses => (
                format!("{base}/responses"),
                serde_json::to_value(&chat_req).unwrap_or_else(|_| json!({})),
            ),
        };

        let start = std::time::Instant::now();
        let mut req = client.post(&url).json(&body);
        for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
            req = req.header(name, value);
        }

        match req.send().await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let code = resp.status().as_u16();
                results.push(json!({
                    "model": deecodex_model,
                    "upstream_model": upstream_model,
                    "provider": &provider,
                    "provider_profile": &provider_profile,
                    "latency_ms": latency_ms,
                    "status": if code == 200 { "ok".to_string() } else { format!("http_{}", code) },
                }));
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                results.push(json!({
                    "model": deecodex_model,
                    "upstream_model": upstream_model,
                    "provider": &provider,
                    "provider_profile": &provider_profile,
                    "latency_ms": latency_ms,
                    "status": format!("error: {}", e),
                }));
            }
        }
    }

    Ok(json!({
        "provider": provider,
        "provider_profile": provider_profile,
        "wire_protocol": wire_protocol,
        "results": results,
        "upstream": upstream,
    }))
}

/// DEX 助手：线程清理分析
#[tauri::command]
pub fn dex_thread_cleanup(dry_run: Option<bool>) -> Result<Value, String> {
    let dry_run = dry_run.unwrap_or(true);

    let threads =
        deecodex::codex_threads::list_all().map_err(|e| format!("获取线程列表失败: {e}"))?;

    let mut empty_count = 0u64;
    let mut orphan_count = 0u64;
    let mut duplicate_count = 0u64;
    let mut seen_titles: HashSet<String> = HashSet::new();

    for t in &threads {
        // 空线程检测：内容无消息
        if let Ok(content) = deecodex::codex_threads::get_thread_content(&t.id) {
            let msgs = content
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if msgs == 0 {
                empty_count += 1;
            }
        } else {
            orphan_count += 1;
        }

        // 重复标题检测
        if !t.title.is_empty() && !seen_titles.insert(t.title.clone()) {
            duplicate_count += 1;
        }
    }

    Ok(json!({
        "dry_run": dry_run,
        "total_threads": threads.len(),
        "empty_count": empty_count,
        "orphan_count": orphan_count,
        "duplicate_count": duplicate_count,
        "total_removable": empty_count + orphan_count + duplicate_count,
    }))
}

/// DEX 助手：自动调优建议
#[tauri::command]
pub fn dex_auto_tune() -> Result<Value, String> {
    let args = load_args();

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let total_memory_gb = get_total_memory_gb();
    let disk_free_gb = get_disk_free_gb(&args.data_dir);

    let mut recommendations = Vec::new();

    // max_body_mb 建议（基于内存）
    let recommended_body_mb = if total_memory_gb >= 16.0 {
        200
    } else if total_memory_gb >= 8.0 {
        100
    } else {
        50
    };
    if (args.max_body_mb as u32) < recommended_body_mb {
        recommendations.push(json!({
            "param": "max_body_mb",
            "current": args.max_body_mb,
            "recommended": recommended_body_mb,
            "reason": format!("基于 {:.0}GB 内存推荐", total_memory_gb),
        }));
    }

    // 磁盘空间告警
    if disk_free_gb < 10.0 {
        recommendations.push(json!({
            "param": "disk_space",
            "current": format!("{:.1}GB", disk_free_gb),
            "recommended": "清理 data_dir 中的日志和备份",
            "reason": "磁盘剩余空间不足 10GB",
        }));
    }

    // CPU 并发建议
    if cpu_cores >= 8 {
        recommendations.push(json!({
            "param": "concurrency",
            "current": "默认",
            "recommended": "可适当提高请求并发限制",
            "reason": format!("{} 核 CPU 有充足并行能力", cpu_cores),
        }));
    }

    Ok(json!({
        "system": {
            "cpu_cores": cpu_cores,
            "total_memory_gb": total_memory_gb,
            "disk_free_gb": (disk_free_gb * 10.0).round() / 10.0,
        },
        "recommendations": recommendations,
    }))
}

/// DEX 助手：Claude Code MCP 配置检查
#[tauri::command]
pub fn dex_claude_mcp_check() -> Result<Value, String> {
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
#[tauri::command]
pub fn dex_claude_env_overview() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let claude_dir = home.join(".claude");
    let mcp = dex_claude_mcp_check()?;
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
#[tauri::command]
pub async fn dex_openclaw_env_overview() -> Result<Value, String> {
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
    let mcp = dex_openclaw_mcp_check().await?;
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
#[tauri::command]
pub async fn dex_openclaw_health_check() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_openclaw_mcp_check() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_openclaw_gateway_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_openclaw_agents_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_openclaw_models_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_openclaw_approvals_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_hermes_env_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_hermes_doctor_check() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_hermes_skills_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_hermes_config_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_hermes_gateway_overview() -> Result<Value, String> {
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
#[tauri::command]
pub async fn dex_ai_toolchain_overview(manager: State<'_, ServerManager>) -> Result<Value, String> {
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
    let tools = all_tool_defs(&manager).await;
    let plugin_tool_count = tools.iter().filter(|tool| tool.source == "plugin").count();
    let claude = dex_claude_env_overview()?;
    let openclaw = dex_openclaw_env_overview().await?;
    let hermes = dex_hermes_env_overview().await?;
    let hermes_config = dex_hermes_config_overview().await?;
    let codex = json!({
        "installed": codex_is_installed(),
        "version": get_cli_version_flexible("codex"),
        "config_path": codex_config_path().map(|p| p.to_string_lossy().to_string()),
    });
    let processes = dex_detect_processes().unwrap_or_else(|e| json!({ "error": e }));
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

/// DEX 助手：网络拓扑检测
#[tauri::command]
pub fn dex_network_topology() -> Result<Value, String> {
    let args = load_args();
    let upstream = args.upstream.clone();

    let host = upstream
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    // DNS 检测
    let dns_servers = get_dns_servers();

    // Ping 检测
    #[cfg(target_os = "macos")]
    let ping_args: &[&str] = &["-c", "1", "-t", "1"];
    #[cfg(not(target_os = "macos"))]
    let ping_args: &[&str] = &["-c", "1", "-W", "1"];

    let (upstream_reachable, latency_ms) = if let Ok(out) = std::process::Command::new("ping")
        .args(ping_args)
        .arg(host)
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let ms = parse_ping_latency(&s);
            (true, ms)
        } else {
            (false, None)
        }
    } else {
        (false, None)
    };

    Ok(json!({
        "dns_servers": dns_servers,
        "upstream_host": host,
        "upstream_reachable": upstream_reachable,
        "latency_ms": latency_ms,
    }))
}

fn get_dns_servers() -> Vec<String> {
    let mut servers = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil")
            .args(["--dns"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                for line in s.lines() {
                    if let Some(ip) = line
                        .trim()
                        .strip_prefix("nameserver[")
                        .and_then(|rest| rest.split("] : ").nth(1))
                    {
                        let ip = ip.trim();
                        if !servers.contains(&ip.to_string()) {
                            servers.push(ip.to_string());
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in content.lines() {
                if let Some(ip) = line.trim().strip_prefix("nameserver ") {
                    let ip = ip.trim();
                    if !servers.contains(&ip.to_string()) {
                        servers.push(ip.to_string());
                    }
                }
            }
        }
    }
    servers
}

fn parse_ping_latency(stdout: &str) -> Option<u64> {
    for part in stdout.split_whitespace() {
        if let Some(time_str) = part.strip_prefix("time=") {
            if let Ok(ms) = time_str.trim_end_matches("ms").trim().parse::<f64>() {
                return Some(ms as u64);
            }
        }
    }
    None
}

/// DEX 助手：SSL 证书检查
#[tauri::command]
pub fn dex_ssl_check() -> Result<Value, String> {
    let args = load_args();
    let upstream = if args.upstream.is_empty() {
        "https://api.openai.com".to_string()
    } else {
        args.upstream.clone()
    };

    let provider = deecodex::providers::guess_provider(&upstream);
    let profile = deecodex::providers::profile_by_slug(provider);
    let check_url = deecodex::providers::model_discovery_url(&profile, &upstream, "")
        .unwrap_or_else(|| upstream.trim_end_matches('/').to_string());

    let output = std::process::Command::new("curl")
        .args(["-sI", "--max-time", "10", &check_url])
        .output();

    let (https_ok, status) = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if out.status.success() {
                if let Some(line) = stdout.lines().next() {
                    if line.contains("200") || line.contains("401") || line.contains("403") {
                        (
                            true,
                            format!(
                                "HTTPS 连接正常 (HTTP {})",
                                line.split_whitespace().nth(1).unwrap_or("?")
                            ),
                        )
                    } else if line.contains("301") || line.contains("302") {
                        (true, "HTTPS 连接正常 (重定向)".to_string())
                    } else {
                        (false, format!("异常响应: {}", line))
                    }
                } else {
                    (false, "无响应".to_string())
                }
            } else {
                let err = if !stderr.is_empty() { stderr } else { stdout };
                (false, format!("连接失败: {}", err.trim()))
            }
        }
        Err(e) => (false, format!("执行 curl 失败: {}", e)),
    };

    Ok(json!({
        "url": check_url,
        "https_ok": https_ok,
        "status": status,
    }))
}

/// DEX 助手：导出诊断报告
#[tauri::command]
pub fn dex_export_report() -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut report = String::new();
    report.push_str("# deecodex 诊断报告\n\n");
    report.push_str(&format!("生成时间: {}\n", now));

    // 环境信息
    report.push_str("\n## 环境信息\n\n");
    report.push_str(&format!("- 操作系统: {}\n", std::env::consts::OS));
    report.push_str(&format!("- deecodex 版本: {}\n", env!("CARGO_PKG_VERSION")));
    report.push_str(&format!("- 数据目录: {}\n", data_dir.display()));

    // 配置文件状态
    report.push_str("\n## 配置文件\n\n");
    let config_exists = data_dir.join("config.json").exists();
    let accounts_exists = data_dir.join("accounts.json").exists();
    let codex_exists = codex_config_path().is_some_and(|p| p.exists());
    report.push_str(&format!(
        "- config.json: {}\n",
        if config_exists { "存在" } else { "缺失" }
    ));
    report.push_str(&format!(
        "- accounts.json: {}\n",
        if accounts_exists { "存在" } else { "缺失" }
    ));
    report.push_str(&format!(
        "- Codex config.toml: {}\n",
        if codex_exists { "存在" } else { "缺失" }
    ));

    // 账号信息
    report.push_str("\n## 账号信息\n\n");
    let store = deecodex::accounts::load_accounts(data_dir);
    report.push_str(&format!("- 账号总数: {}\n", store.accounts.len()));
    report.push_str(&format!(
        "- 活跃账号: {}\n",
        store.active_id.as_deref().unwrap_or("无")
    ));
    for acc in &store.accounts {
        let active = if Some(&acc.id) == store.active_id.as_ref() {
            " [活跃]"
        } else {
            ""
        };
        let target = if acc.client_kind.is_codex() {
            "Codex 代理账号"
        } else {
            "客户端配置账号"
        };
        report.push_str(&format!(
            "  - {}{} ({}, {}), 模型数: {}, 最近检查: {}\n",
            acc.name,
            active,
            target,
            acc.provider,
            acc.model_map.len(),
            acc.last_check
                .as_ref()
                .map(|check| check.message.as_str())
                .unwrap_or("无")
        ));
    }

    // 线程状态
    report.push_str("\n## 线程状态\n\n");
    match deecodex::codex_threads::status(data_dir) {
        Ok(s) => {
            report.push_str(&format!("- 线程总数: {}\n", s.total));
            report.push_str(&format!("- 已迁移: {}\n", s.migrated));
            report.push_str(&format!("- 非 deecodex 线程: {}\n", s.non_deecodex_count));
        }
        Err(e) => report.push_str(&format!("- 获取失败: {}\n", e)),
    }

    // Codex 状态
    report.push_str("\n## Codex 状态\n\n");
    report.push_str(&format!("- 已安装: {}\n", codex_is_installed()));

    // 保存报告
    let report_path = data_dir.join("diagnostic_report.md");
    let saved_to = report_path.to_string_lossy().to_string();
    std::fs::write(&report_path, &report).map_err(|e| format!("保存报告失败: {e}"))?;

    Ok(json!({
        "markdown": report,
        "saved_to": saved_to,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::dex_registry::default_capability_enabled;

    #[test]
    fn builtin_tools_keep_legacy_names_and_capabilities() {
        let tools = builtin_tool_defs();
        let health = tools
            .iter()
            .find(|tool| tool.name == "health_summary")
            .expect("health_summary 工具应保留");
        assert_eq!(health.capability, "deecodex.ops");
        assert_eq!(health.level, 0);

        let shell = tools
            .iter()
            .find(|tool| tool.name == "execute_shell")
            .expect("execute_shell 工具应保留");
        assert_eq!(shell.capability, "core.workspace");
        assert_eq!(shell.level, 3);
        assert!(shell.confirm.is_some());

        let claude = tools
            .iter()
            .find(|tool| tool.name == "claude_env_overview")
            .expect("claude_env_overview 工具应存在");
        assert_eq!(claude.capability, "ai.claude");
        assert_eq!(claude.level, 0);

        let openclaw = tools
            .iter()
            .find(|tool| tool.name == "openclaw_env_overview")
            .expect("openclaw_env_overview 工具应存在");
        assert_eq!(openclaw.capability, "ai.openclaw");
        assert_eq!(openclaw.level, 0);

        let hermes = tools
            .iter()
            .find(|tool| tool.name == "hermes_env_overview")
            .expect("hermes_env_overview 工具应存在");
        assert_eq!(hermes.capability, "ai.hermes");
        assert_eq!(hermes.level, 0);

        let toolchain = tools
            .iter()
            .find(|tool| tool.name == "ai_toolchain_overview")
            .expect("ai_toolchain_overview 工具应存在");
        assert_eq!(toolchain.capability, "core.system");
        assert_eq!(toolchain.level, 0);

        let self_check = tools
            .iter()
            .find(|tool| tool.name == "dex_self_check")
            .expect("dex_self_check 工具应存在");
        assert_eq!(self_check.capability, "core.system");
        assert_eq!(self_check.level, 0);
    }

    #[test]
    fn capability_defaults_keep_workspace_mutations_opt_in() {
        assert!(default_capability_enabled("deecodex.ops"));
        assert!(default_capability_enabled("plugins.dynamic"));
        assert!(default_capability_enabled("core.workspace"));

        let shell = builtin_tool_defs()
            .into_iter()
            .find(|tool| tool.name == "execute_shell")
            .expect("execute_shell 工具应保留");
        assert_eq!(shell.level, 3);
    }

    #[test]
    fn path_allowlist_uses_path_boundaries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("deecodex-allow-{nonce}"));
        let sibling = std::env::temp_dir().join(format!("deecodex-allow-{nonce}-sibling"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let allowed = root.join("file.txt");
        let rejected = sibling.join("file.txt");
        std::fs::write(&allowed, "ok").unwrap();
        std::fs::write(&rejected, "no").unwrap();

        let allowed = allowed.canonicalize().unwrap();
        let rejected = rejected.canonicalize().unwrap();
        assert!(is_under_allowed_root(&allowed, &root));
        assert!(!is_under_allowed_root(&rejected, &root));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(sibling);
    }

    #[test]
    fn config_restore_requires_confirmation() {
        let err = dex_config_backup("restore".into(), Some("backup".into()), None).unwrap_err();
        assert!(err.contains("必须先确认"));
    }

    #[test]
    fn all_builtin_tool_names_are_function_call_safe() {
        for tool in builtin_tool_defs() {
            assert!(
                tool.name.len() <= 64,
                "{} 超过 function calling 名称长度上限",
                tool.name
            );
            assert!(
                tool.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "{} 包含不安全字符",
                tool.name
            );
        }
    }

    #[test]
    fn cli_json_parser_accepts_prefixed_json_output() {
        let parsed = parse_json_output("OpenClaw status\n{\"ok\":true,\"items\":[1]}").unwrap();
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["items"][0], 1);

        let parsed = parse_json_output("Hermes skills\n[{\"name\":\"a\"}]").unwrap();
        assert_eq!(parsed[0]["name"], "a");
    }

    #[test]
    fn client_lifecycle_specs_cover_status_dock_entries() {
        let kinds: Vec<_> = CLIENT_APP_SPECS.iter().map(|spec| spec.kind).collect();
        assert_eq!(
            kinds,
            vec![
                "codex_cli",
                "codex_desktop",
                "claude_cli",
                "claude_desktop",
                "openclaw",
                "hermes"
            ]
        );
        assert!(client_app_spec_or_err("codex_cli").unwrap().command == Some("codex"));
        assert_eq!(
            client_app_spec_or_err("openclaw").unwrap().launch_command,
            Some("openclaw tui")
        );
        assert!(
            client_app_spec_or_err("codex_cli")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(
            client_app_spec_or_err("claude_cli")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(
            !client_app_spec_or_err("openclaw")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(
            !client_app_spec_or_err("hermes")
                .unwrap()
                .launch_requires_cwd
        );
        assert!(client_app_spec("missing").is_none());
    }

    #[test]
    fn client_lifecycle_install_commands_are_whitelisted() {
        let commands: Vec<_> = CLIENT_APP_SPECS
            .iter()
            .filter_map(install_command_for_status)
            .collect();
        assert!(commands.iter().any(|cmd| cmd.contains("@openai/codex")));
        assert!(commands
            .iter()
            .any(|cmd| cmd.contains("@anthropic-ai/claude-code")));
        assert!(commands.iter().any(|cmd| cmd.contains("openclaw.ai")));
        assert!(commands.iter().any(|cmd| cmd.contains("hermes")));
    }

    #[test]
    fn terminal_scripts_quote_launch_directories() {
        let cwd = Path::new("/tmp/dee codex/it's ok");
        let mac = mac_terminal_script("codex", Some(cwd));
        assert!(mac.contains("cd '/tmp/dee codex/it'\\\\''s ok' && codex"));

        let win = windows_terminal_script("claude", Some(Path::new(r"C:\Users\A B\repo")));
        assert!(win.contains("Set-Location -LiteralPath 'C:\\Users\\A B\\repo'; claude"));
    }

    #[test]
    fn terminal_scripts_without_launch_directory_do_not_cd() {
        assert_eq!(
            mac_terminal_script("openclaw", None),
            "tell application \"Terminal\"\nactivate\ndo script \"openclaw\"\nend tell"
        );
        assert_eq!(windows_terminal_script("hermes", None), "hermes");
    }
}
