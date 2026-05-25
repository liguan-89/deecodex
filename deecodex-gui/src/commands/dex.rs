use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

use super::dex_cli::*;
use super::dex_plugins::plugin_tool_defs;
use super::dex_registry::{
    builtin_tool_defs, capability_meta, is_capability_enabled, load_capability_states,
    save_capability_states, DexCapability, DexToolDef,
};

// ── 执行辅助函数 ──────────────────────────────────────────────────────────

/// 获取 Codex config.toml 的路径（等效于 deecodex::codex_config::codex_config_path）
pub(super) fn codex_config_path() -> Option<PathBuf> {
    deecodex::config::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

/// 检测 Codex 是否已安装（等效于 deecodex::codex_config::codex_is_installed）
pub(super) fn codex_is_installed() -> bool {
    if let Some(home) = deecodex::config::home_dir() {
        if home.join(".codex").exists() {
            return true;
        }
    }
    command_exists("codex")
}

pub(super) async fn all_tool_defs(manager: &ServerManager) -> Vec<DexToolDef> {
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

/// DEX 2.0：获取 AI 工具工作台上下文摘要。
#[tauri::command]
pub async fn dex_get_workspace_context(manager: State<'_, ServerManager>) -> Result<Value, String> {
    super::dex_workspace::dex_get_workspace_context_impl(manager).await
}

/// DEX 2.0：统一工具执行入口。前端只负责确认与展示，后端负责路由、策略和脱敏。
#[tauri::command]
pub async fn dex_execute_tool(
    manager: State<'_, ServerManager>,
    name: String,
    args: Option<Value>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    super::dex_tool_executor::dex_execute_tool_impl(manager, name, args, confirmed).await
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
    super::dex_chat::dex_chat_impl(manager, messages, tools, stream, model).await
}

/// DEX 助手：安全读取文件
#[tauri::command]
pub fn dex_read_file(path: String, max_lines: Option<usize>) -> Result<Value, String> {
    super::dex_workspace::dex_read_file_impl(path, max_lines)
}

/// DEX 助手：列出目录内容
#[tauri::command]
pub fn dex_list_directory(path: String) -> Result<Value, String> {
    super::dex_workspace::dex_list_directory_impl(path)
}

/// DEX 助手：检测运行中的进程
#[tauri::command]
pub fn dex_detect_processes() -> Result<Value, String> {
    super::dex_process::dex_detect_processes_impl()
}

/// 状态页客户端 Dock：读取单个客户端的一键接入生命周期状态。
#[tauri::command]
pub async fn dex_client_lifecycle_status(
    manager: State<'_, ServerManager>,
    kind: String,
) -> Result<Value, String> {
    super::dex_clients::dex_client_lifecycle_status_impl(manager, kind).await
}

/// 状态页客户端 Dock：安装或打开官方下载页。
#[tauri::command]
pub fn dex_install_client(kind: String) -> Result<Value, String> {
    super::dex_clients::dex_install_client_impl(kind)
}

/// 状态页客户端 Dock：启动桌面版或在终端中启动 CLI。
#[tauri::command]
pub fn dex_launch_client(kind: String, cwd: Option<String>) -> Result<Value, String> {
    super::dex_clients::dex_launch_client_impl(kind, cwd)
}

/// 状态页客户端 Dock：选择 CLI 启动目录。
#[tauri::command]
pub async fn dex_pick_client_launch_dir() -> Result<Option<String>, String> {
    super::dex_clients::dex_pick_client_launch_dir_impl().await
}

/// 状态页客户端 Dock：打开或退出桌面版客户端。
#[tauri::command]
pub fn dex_toggle_desktop_client(kind: String, running: bool) -> Result<Value, String> {
    super::dex_clients::dex_toggle_desktop_client_impl(kind, running)
}

/// DEX 助手：检测端口占用
#[tauri::command]
pub fn dex_detect_ports() -> Result<Value, String> {
    super::dex_diagnostics::dex_detect_ports_impl()
}

/// DEX 助手：收集环境信息
#[tauri::command]
pub fn dex_get_env_info() -> Result<Value, String> {
    super::dex_diagnostics::dex_get_env_info_impl()
}

/// DEX 助手：安全执行 Shell 命令
#[tauri::command]
pub async fn dex_execute_shell(
    command: String,
    timeout_secs: Option<u64>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    super::dex_workspace::dex_execute_shell_impl(command, timeout_secs, confirmed).await
}

/// DEX 助手：搜索日志
#[tauri::command]
pub fn dex_search_logs(query: String, context_lines: Option<usize>) -> Result<Value, String> {
    super::dex_workspace::dex_search_logs_impl(query, context_lines)
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
    super::dex_diagnostics::dex_health_summary_impl(manager).await
}

/// DEX 助手：自检 DEX 注册表、能力包、插件工具和最近错误。
#[tauri::command]
pub async fn dex_self_check(manager: State<'_, ServerManager>) -> Result<Value, String> {
    super::dex_diagnostics::dex_self_check_impl(manager).await
}

/// DEX 助手：请求历史分析（最近请求统计）
#[tauri::command]
pub async fn dex_analyze_requests(manager: State<'_, ServerManager>) -> Result<Value, String> {
    super::dex_diagnostics::dex_analyze_requests_impl(manager).await
}

/// DEX 助手：配置备份/恢复
#[tauri::command]
pub fn dex_config_backup(
    action: String,
    name: Option<String>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    super::dex_ops::dex_config_backup_impl(action, name, confirmed)
}

/// DEX 助手：配置差异对比
#[tauri::command]
pub fn dex_config_diff() -> Result<Value, String> {
    super::dex_ops::dex_config_diff_impl()
}

/// DEX 助手：Token 费用估算
#[tauri::command]
pub async fn dex_token_cost(manager: State<'_, ServerManager>) -> Result<Value, String> {
    super::dex_ops::dex_token_cost_impl(manager).await
}

/// DEX 助手：模型速度测试
#[tauri::command]
pub async fn dex_speed_test() -> Result<Value, String> {
    super::dex_ops::dex_speed_test_impl().await
}

/// DEX 助手：线程清理分析
#[tauri::command]
pub fn dex_thread_cleanup(dry_run: Option<bool>) -> Result<Value, String> {
    super::dex_ops::dex_thread_cleanup_impl(dry_run)
}

/// DEX 助手：自动调优建议
#[tauri::command]
pub fn dex_auto_tune() -> Result<Value, String> {
    super::dex_ops::dex_auto_tune_impl()
}

/// DEX 助手：Claude Code MCP 配置检查
#[tauri::command]
pub fn dex_claude_mcp_check() -> Result<Value, String> {
    super::dex_toolchain::dex_claude_mcp_check_impl()
}

/// DEX 助手：Claude Code 环境概览
#[tauri::command]
pub fn dex_claude_env_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_claude_env_overview_impl()
}

/// DEX 助手：OpenClaw 环境概览
#[tauri::command]
pub async fn dex_openclaw_env_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_env_overview_impl().await
}

/// DEX 助手：OpenClaw 健康检查
#[tauri::command]
pub async fn dex_openclaw_health_check() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_health_check_impl().await
}

/// DEX 助手：OpenClaw MCP 检查
#[tauri::command]
pub async fn dex_openclaw_mcp_check() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_mcp_check_impl().await
}

/// DEX 助手：OpenClaw Gateway 概览
#[tauri::command]
pub async fn dex_openclaw_gateway_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_gateway_overview_impl().await
}

/// DEX 助手：OpenClaw agents 概览
#[tauri::command]
pub async fn dex_openclaw_agents_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_agents_overview_impl().await
}

/// DEX 助手：OpenClaw models 概览
#[tauri::command]
pub async fn dex_openclaw_models_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_models_overview_impl().await
}

/// DEX 助手：OpenClaw approvals 概览
#[tauri::command]
pub async fn dex_openclaw_approvals_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_openclaw_approvals_overview_impl().await
}

/// DEX 助手：Hermes 环境概览
#[tauri::command]
pub async fn dex_hermes_env_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_hermes_env_overview_impl().await
}

/// DEX 助手：Hermes doctor 检查
#[tauri::command]
pub async fn dex_hermes_doctor_check() -> Result<Value, String> {
    super::dex_toolchain::dex_hermes_doctor_check_impl().await
}

/// DEX 助手：Hermes skills 概览
#[tauri::command]
pub async fn dex_hermes_skills_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_hermes_skills_overview_impl().await
}

/// DEX 助手：Hermes 配置概览
#[tauri::command]
pub async fn dex_hermes_config_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_hermes_config_overview_impl().await
}

/// DEX 助手：Hermes Gateway 概览
#[tauri::command]
pub async fn dex_hermes_gateway_overview() -> Result<Value, String> {
    super::dex_toolchain::dex_hermes_gateway_overview_impl().await
}

/// DEX 助手：AI 工具链总览
#[tauri::command]
pub async fn dex_ai_toolchain_overview(manager: State<'_, ServerManager>) -> Result<Value, String> {
    super::dex_toolchain::dex_ai_toolchain_overview_impl(manager).await
}

/// DEX 助手：网络拓扑检测
#[tauri::command]
pub fn dex_network_topology() -> Result<Value, String> {
    super::dex_ops::dex_network_topology_impl()
}

/// DEX 助手：SSL 证书检查
#[tauri::command]
pub fn dex_ssl_check() -> Result<Value, String> {
    super::dex_ops::dex_ssl_check_impl()
}

/// DEX 助手：导出诊断报告
#[tauri::command]
pub fn dex_export_report() -> Result<Value, String> {
    super::dex_ops::dex_export_report_impl()
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
}
