use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{Emitter, State};
use tracing;

use crate::ServerManager;

use super::dex_cli::*;
use super::dex_plugins::{execute_plugin_tool, plugin_tool_defs};
use super::dex_protocol::{
    dex_responses_request_target, dex_responses_to_chat_value, get_active_account_info,
};
use super::dex_registry::{
    builtin_tool_defs, capability_meta, is_capability_enabled, load_capability_states,
    save_capability_states, DexCapability, DexToolDef,
};
use super::dex_security::mask_sensitive_value;

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
                let thread_key =
                    opt_string(&args, "thread_key").or_else(|| opt_string(&args, "threadKey"));
                crate::commands::get_client_thread_content(
                    req_string(&args, "client_kind")?,
                    req_string(&args, "native_id")?,
                    thread_key,
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
            "list_plugin_events" => json!(
                crate::commands::list_plugin_events(
                    manager,
                    opt_string(&args, "plugin_id"),
                    opt_usize(&args, "limit"),
                )
                .await?
            ),
            "install_plugin" => {
                crate::commands::install_plugin(manager, opt_string(&args, "path"), None, None)
                    .await?
            }
            "update_plugin" => {
                crate::commands::update_plugin(manager, opt_string(&args, "path"), None, None)
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
            "set_plugin_enabled" => {
                crate::commands::set_plugin_enabled(
                    manager,
                    req_string(&args, "plugin_id")?,
                    opt_bool(&args, "enabled").ok_or_else(|| "缺少参数: enabled".to_string())?,
                )
                .await?
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
            "execute_plugin_feature" => {
                let params = if let Some(raw) = opt_string(&args, "params_json") {
                    Some(
                        serde_json::from_str(&raw)
                            .map_err(|e| format!("解析 params_json 失败: {e}"))?,
                    )
                } else {
                    args.get("params").cloned()
                };
                crate::commands::execute_plugin_feature(
                    manager,
                    req_string(&args, "plugin_id")?,
                    req_string(&args, "feature_id")?,
                    req_string(&args, "action")?,
                    params,
                    opt_bool(&args, "confirmed"),
                )
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

    let (upstream, api_key, model_map, provider, profile, endpoint_kind, endpoint_path) =
        get_active_account_info(&data_dir)
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

    let (url, body, use_provider_headers) = match profile.wire_protocol {
        deecodex::providers::WireProtocol::ChatCompletions => (
            format!("{base}/chat/completions"),
            serde_json::to_value(&chat_req).map_err(|e| format!("序列化请求失败: {e}"))?,
            true,
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
            (url, body, true)
        }
        deecodex::providers::WireProtocol::Responses => {
            dex_responses_request_target(
                &manager,
                &endpoint_kind,
                &upstream,
                &endpoint_path,
                &chat_req,
            )
            .await?
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
    if use_provider_headers {
        for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
            req = req.header(name, value);
        }
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
        if profile.wire_protocol == deecodex::providers::WireProtocol::Responses {
            return Ok(dex_responses_to_chat_value(resp_body));
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
