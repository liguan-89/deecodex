use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

use super::dex_plugins::execute_plugin_tool;
use super::dex_registry::{is_capability_enabled, load_capability_states};
use super::dex_security::mask_sensitive_value;

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

fn opt_string_any(args: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| opt_string(args, key))
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

fn active_account_id(manager_data_dir: &std::path::Path) -> Option<String> {
    let store = deecodex::accounts::load_accounts(manager_data_dir);
    store
        .active_selection_for_dex_assistant()
        .and_then(|selection| selection.account_id.clone())
        .or(store.active_id)
}

pub(super) async fn dex_execute_tool_impl(
    manager: State<'_, ServerManager>,
    name: String,
    args: Option<Value>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    let args = args.unwrap_or_else(|| json!({}));
    let tool = super::dex::all_tool_defs(&manager)
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
            "get_active_account" => crate::commands::get_dex_assistant_account(manager).await?,
            "add_account" => {
                crate::commands::add_account(
                    manager,
                    req_string(&args, "provider")?,
                    opt_string(&args, "account_json"),
                    opt_string_any(&args, &["client_kind", "clientKind"]),
                    opt_string_any(&args, &["client_surface", "clientSurface"]),
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
                crate::commands::set_dex_assistant_account_inner(
                    &manager,
                    req_string(&args, "id")?,
                    opt_string_any(&args, &["endpoint_id", "endpointId"]),
                )
                .await?
            }
            "import_codex_config" => crate::commands::import_codex_config(manager).await?,
            "get_provider_presets" => crate::commands::get_provider_presets()?,
            "get_client_profiles" => crate::commands::get_client_profiles()?,
            "client_lifecycle_status" => {
                super::dex::dex_client_lifecycle_status(manager, req_string(&args, "kind")?).await?
            }
            "install_client" => super::dex::dex_install_client(req_string(&args, "kind")?)?,
            "launch_client" => {
                super::dex::dex_launch_client(req_string(&args, "kind")?, opt_string(&args, "cwd"))?
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
            "check_upgrade" => crate::commands::upgrade::check_upgrade_manifest_preview().await?,
            "run_upgrade" => {
                json!({ "output": crate::commands::upgrade::run_upgrade_manifest_preview()? })
            }
            "read_file" => super::dex::dex_read_file(
                req_string(&args, "path")?,
                opt_usize(&args, "max_lines"),
            )?,
            "list_directory" => super::dex::dex_list_directory(req_string(&args, "path")?)?,
            "detect_processes" => super::dex::dex_detect_processes()?,
            "detect_ports" => super::dex::dex_detect_ports()?,
            "get_env_info" => super::dex::dex_get_env_info()?,
            "execute_shell" => {
                super::dex::dex_execute_shell(
                    req_string(&args, "command")?,
                    opt_u64(&args, "timeout_secs"),
                    confirmed,
                )
                .await?
            }
            "search_logs" => super::dex::dex_search_logs(
                req_string(&args, "query")?,
                opt_usize(&args, "context_lines"),
            )?,
            "get_codex_config_raw" => super::dex::dex_get_codex_config_raw()?,
            "health_summary" => super::dex::dex_health_summary(manager).await?,
            "dex_self_check" => super::dex::dex_self_check(manager).await?,
            "analyze_requests" => super::dex::dex_analyze_requests(manager).await?,
            "config_backup" => super::dex::dex_config_backup(
                req_string(&args, "action")?,
                opt_string(&args, "name"),
                confirmed,
            )?,
            "config_restore" => super::dex::dex_config_backup(
                "restore".into(),
                opt_string(&args, "name"),
                confirmed,
            )?,
            "config_diff" => super::dex::dex_config_diff()?,
            "token_cost" => super::dex::dex_token_cost(manager).await?,
            "speed_test" => super::dex::dex_speed_test().await?,
            "thread_cleanup" => super::dex::dex_thread_cleanup(opt_bool(&args, "dry_run"))?,
            "auto_tune" => super::dex::dex_auto_tune()?,
            "claude_mcp_check" => super::dex::dex_claude_mcp_check()?,
            "claude_env_overview" => super::dex::dex_claude_env_overview()?,
            "openclaw_env_overview" => super::dex::dex_openclaw_env_overview().await?,
            "openclaw_health_check" => super::dex::dex_openclaw_health_check().await?,
            "openclaw_mcp_check" => super::dex::dex_openclaw_mcp_check().await?,
            "openclaw_gateway_overview" => super::dex::dex_openclaw_gateway_overview().await?,
            "openclaw_agents_overview" => super::dex::dex_openclaw_agents_overview().await?,
            "openclaw_models_overview" => super::dex::dex_openclaw_models_overview().await?,
            "openclaw_approvals_overview" => super::dex::dex_openclaw_approvals_overview().await?,
            "hermes_env_overview" => super::dex::dex_hermes_env_overview().await?,
            "hermes_doctor_check" => super::dex::dex_hermes_doctor_check().await?,
            "hermes_skills_overview" => super::dex::dex_hermes_skills_overview().await?,
            "hermes_config_overview" => super::dex::dex_hermes_config_overview().await?,
            "hermes_gateway_overview" => super::dex::dex_hermes_gateway_overview().await?,
            "ai_toolchain_overview" => super::dex::dex_ai_toolchain_overview(manager).await?,
            "network_topology" => super::dex::dex_network_topology()?,
            "ssl_check" => super::dex::dex_ssl_check()?,
            "export_report" => super::dex::dex_export_report()?,
            other => return Err(format!("未实现的内置工具: {other}")),
        }
    };

    Ok(mask_sensitive_value(result))
}
