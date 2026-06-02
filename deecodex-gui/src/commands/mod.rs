pub mod dex;
pub mod dex_chat;
pub mod dex_cli;
pub mod dex_clients;
pub mod dex_diagnostics;
pub mod dex_ops;
pub mod dex_plugins;
pub mod dex_process;
pub mod dex_protocol;
pub mod dex_registry;
pub mod dex_security;
pub mod dex_tool_executor;
pub mod dex_toolchain;
pub mod dex_workspace;
pub mod dialogs;
pub mod logs;
pub mod plugins;
pub mod request_history;
pub mod sessions;
pub mod threads;
pub mod upgrade;

pub use plugins::*;

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};

use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{State, WebviewWindow};

use deecodex::accounts::{
    AccountClientKind, AccountClientSurface, AccountStore, DevPipelineToolMode,
    DevPipelineTriggerMode,
};
use deecodex::config::Args;
use deecodex::handlers;
use deecodex::{files, metrics, vector_stores};

use crate::ServerManager;

fn client_kind_slug(kind: &AccountClientKind) -> &'static str {
    match kind {
        AccountClientKind::Codex => "codex",
        AccountClientKind::ClaudeCode => "claude_code",
        AccountClientKind::Openclaw => "openclaw",
        AccountClientKind::Hermes => "hermes",
        AccountClientKind::GenericClient => "generic_client",
    }
}

fn client_account_counts(store: &AccountStore) -> Value {
    let mut counts: HashMap<&'static str, usize> = HashMap::new();
    for account in &store.accounts {
        *counts
            .entry(client_kind_slug(&account.client_kind))
            .or_default() += 1;
    }
    json!(counts)
}

fn mutate_account_store<R, F>(
    data_dir: &Path,
    error_prefix: &str,
    mutate: F,
) -> Result<(AccountStore, R), String>
where
    F: FnOnce(&mut AccountStore) -> Result<R, String>,
{
    deecodex::accounts::with_account_store(data_dir, |store| {
        mutate(store).map_err(anyhow::Error::msg)
    })
    .map_err(|e| format!("{error_prefix}: {e}"))
}

fn validate_endpoint_runtime_urls(
    endpoint: &deecodex::accounts::EndpointConfig,
) -> Result<(), String> {
    deecodex::handlers::validate_upstream(&endpoint.base_url)
        .map_err(|e| format!("目标账号上游 URL 无效: {e}"))?;
    if !endpoint.vision.base_url.trim().is_empty() {
        deecodex::handlers::validate_upstream(&endpoint.vision.base_url)
            .map_err(|e| format!("视觉上游 URL 无效: {e}"))?;
    }
    Ok(())
}

fn client_proxy_base_url(host: &str, port: u16, kind: &AccountClientKind) -> String {
    let url_host = deecodex::config::client_url_host(host);
    match kind {
        AccountClientKind::ClaudeCode => format!("http://{url_host}:{port}"),
        AccountClientKind::Openclaw
        | AccountClientKind::Hermes
        | AccountClientKind::GenericClient => {
            format!("http://{url_host}:{port}/v1")
        }
        AccountClientKind::Codex => format!("http://{url_host}:{port}/v1"),
    }
}

fn ensure_client_proxy_options(account: &mut deecodex::accounts::Account, host: &str, port: u16) {
    if account.client_kind.is_codex() {
        return;
    }
    let enabled = account
        .client_options
        .get("proxy_recording_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return;
    }
    if account
        .client_options
        .get("proxy_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        let token = format!("dee_{}_{}", account.id, deecodex::accounts::generate_id());
        account
            .client_options
            .insert("proxy_token".into(), Value::String(token));
    }
    account.client_options.insert(
        "proxy_base_url".into(),
        Value::String(client_proxy_base_url(host, port, &account.client_kind)),
    );
}

fn append_account_event(
    data_dir: &Path,
    account_id: &str,
    client_kind: &AccountClientKind,
    action: &str,
    ok: bool,
    message: &str,
    details: Value,
) {
    let path = data_dir.join("account-events.jsonl");
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!("创建账号事件目录失败: {err}");
            return;
        }
    }
    let event = json!({
        "ts": deecodex::accounts::now_secs(),
        "account_id": account_id,
        "client_kind": client_kind,
        "action": action,
        "ok": ok,
        "message": message,
        "details": details,
    });
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(err) = writeln!(file, "{event}") {
                tracing::warn!("写入账号事件日志失败: {err}");
            }
        }
        Err(err) => tracing::warn!("打开账号事件日志失败: {err}"),
    }
}

fn read_account_events(data_dir: &Path, account_id: Option<&str>, limit: usize) -> Vec<Value> {
    let path = data_dir.join("account-events.jsonl");
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let mut events: Vec<Value> = content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| {
            account_id.is_none_or(|id| event.get("account_id").and_then(Value::as_str) == Some(id))
        })
        .collect();
    events.sort_by_key(|event| event.get("ts").and_then(Value::as_u64).unwrap_or(0));
    events.reverse();
    events.truncate(limit.clamp(1, 100));
    events
}

fn parse_account_json(raw: &str) -> Result<deecodex::accounts::Account, String> {
    let mut value: Value =
        serde_json::from_str(raw).map_err(|e| format!("解析账号 JSON 失败: {e}"))?;
    if let Value::Object(ref mut object) = value {
        if object.contains_key("client_kind") {
            object.remove("target");
        }
    }
    serde_json::from_value(value).map_err(|e| format!("解析账号 JSON 失败: {e}"))
}

fn apply_explicit_account_client(
    account: &mut deecodex::accounts::Account,
    client_kind: Option<&AccountClientKind>,
    client_surface: Option<&str>,
) {
    let Some(kind) = client_kind else {
        return;
    };
    account.client_kind = kind.clone();
    account.client_surface = parse_account_client_surface(client_surface.unwrap_or("cli"), kind);
    account
        .client_options
        .insert("client_kind".into(), json!(client_kind_slug(kind)));
    account.client_options.insert(
        "client_surface".into(),
        json!(account.client_surface.clone()),
    );
}

#[derive(Debug, Deserialize)]
struct AuthJsonImportFile {
    #[serde(default)]
    name: String,
    content: String,
}

fn auth_json_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .or_else(|| value.get("tokens").and_then(|tokens| tokens.get(key)))
        .or_else(|| value.get("token").and_then(|token| token.get(key)))
        .or_else(|| value.get("oauth").and_then(|oauth| oauth.get(key)))
        .or_else(|| value.get("auth").and_then(|auth| auth.get(key)))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string()
}

fn auth_json_u64(value: &Value, key: &str) -> Option<u64> {
    value
        .get(key)
        .or_else(|| value.get("tokens").and_then(|tokens| tokens.get(key)))
        .or_else(|| value.get("token").and_then(|token| token.get(key)))
        .or_else(|| value.get("oauth").and_then(|oauth| oauth.get(key)))
        .and_then(|raw| {
            raw.as_u64().or_else(|| {
                raw.as_str()
                    .map(str::trim)
                    .and_then(|text| text.parse::<u64>().ok())
            })
        })
}

fn parse_auth_json_import_files(raw: &str) -> Result<Vec<AuthJsonImportFile>, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("解析认证文件列表失败: {e}"))?;
    if value.is_array() {
        return serde_json::from_value(value).map_err(|e| format!("解析认证文件列表失败: {e}"));
    }
    if value.get("content").is_some() {
        return serde_json::from_value(value)
            .map(|file| vec![file])
            .map_err(|e| format!("解析认证文件失败: {e}"));
    }
    Ok(vec![AuthJsonImportFile {
        name: "auth.json".into(),
        content: raw.to_string(),
    }])
}

fn codex_oauth_token_from_auth_json(
    value: &Value,
    now: u64,
) -> Result<deecodex::oauth_accounts::OAuthToken, String> {
    let provider = auth_json_string(value, "type");
    let provider = if provider.is_empty() {
        auth_json_string(value, "provider")
    } else {
        provider
    };
    let provider = provider.to_ascii_lowercase();
    let access_token = auth_json_string(value, "access_token");
    let refresh_token = auth_json_string(value, "refresh_token");
    let id_token = auth_json_string(value, "id_token");
    if (provider == "codex" || provider == "openai" || provider == "chatgpt")
        && access_token.is_empty()
    {
        return Err("认证文件缺少 access_token".into());
    }
    let explicit_codex = provider == "codex" || provider == "openai" || provider == "chatgpt";
    let looks_like_codex = !access_token.is_empty()
        && (explicit_codex
            || (provider.is_empty() && (!id_token.is_empty() || !refresh_token.is_empty())));
    if !looks_like_codex {
        let label = if provider.is_empty() {
            "未知".to_string()
        } else {
            provider
        };
        return Err(format!("暂不支持的认证类型: {label}"));
    }

    let token_info = deecodex::oauth_accounts::codex_id_token_info(&id_token);
    let mut email = auth_json_string(value, "email");
    if email.is_empty() {
        email = token_info
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
    }
    let mut account_id = auth_json_string(value, "account_id");
    if account_id.is_empty() {
        account_id = token_info
            .get("chatgpt_account_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
    }

    let expired_at = auth_json_u64(value, "expired_at")
        .or_else(|| auth_json_u64(value, "expires_at"))
        .or_else(|| {
            auth_json_u64(value, "expires_in").map(|expires_in| now.saturating_add(expires_in))
        })
        .unwrap_or(0);

    Ok(deecodex::oauth_accounts::OAuthToken {
        provider: "codex".into(),
        access_token,
        refresh_token,
        id_token,
        email,
        account_id,
        expired: auth_json_string(value, "expired"),
        expired_at,
        last_refresh: auth_json_string(value, "last_refresh"),
    })
}

fn codex_official_endpoint_config(account_id: &str) -> deecodex::accounts::EndpointConfig {
    deecodex::accounts::EndpointConfig {
        id: format!("endpoint_{account_id}"),
        name: "Codex 官方".into(),
        kind: deecodex::accounts::EndpointKind::CodexOfficial,
        base_url: deecodex::handlers::CODEX_OFFICIAL_BASE_URL.into(),
        path: "responses".into(),
        template_id: "codex_official".into(),
        template_version: 1,
        model_map: HashMap::new(),
        known_models: Vec::new(),
        model_profiles: HashMap::new(),
        vision: deecodex::accounts::VisionConfig {
            mode: deecodex::accounts::VisionMode::Native,
            ..Default::default()
        },
        image_generation_enabled: Some(true),
        custom_headers: HashMap::new(),
        request_timeout_secs: None,
        max_retries: None,
        context_window_override: None,
        reasoning_effort_override: None,
        thinking_tokens: None,
        fast_mode_enabled: false,
        fast_service_tier: "priority".into(),
        balance_url: String::new(),
    }
}

fn codex_account_from_imported_token(
    token: deecodex::oauth_accounts::OAuthToken,
    source_name: &str,
    client_surface: AccountClientSurface,
    now: u64,
) -> deecodex::accounts::Account {
    use deecodex::accounts::{
        Account, AccountAuthMode, AccountClientKind, DevPipelineToolMode, DevPipelineTriggerMode,
    };

    let account_id = deecodex::accounts::generate_id();
    let mut client_options = HashMap::new();
    client_options.insert(
        "oauth".into(),
        deecodex::oauth_accounts::oauth_token_to_value(&token, "import"),
    );
    client_options.insert("auth_mode".into(), json!("oauth"));
    client_options.insert(
        "routing".into(),
        json!({
            "enabled": true,
            "pool": "codex-official",
            "priority": 0,
            "weight": 1,
            "disabled": false,
        }),
    );
    if !source_name.trim().is_empty() {
        client_options.insert("auth_file_name".into(), json!(source_name.trim()));
    }

    let mut account = Account {
        id: account_id.clone(),
        name: oauth_account_name("Codex", &token.email),
        provider: "codex".into(),
        client_kind: AccountClientKind::Codex,
        client_surface,
        wire_protocol: Default::default(),
        upstream: deecodex::handlers::CODEX_OFFICIAL_BASE_URL.into(),
        api_key: token.access_token.clone(),
        auth_mode: AccountAuthMode::OAuth,
        default_model: String::new(),
        client_options,
        runtime_state: Default::default(),
        last_applied_at: None,
        last_check: None,
        model_map: HashMap::new(),
        vision_upstream: String::new(),
        vision_api_key: String::new(),
        vision_model: String::new(),
        vision_endpoint: String::new(),
        vision_enabled: false,
        from_codex_config: false,
        balance_url: String::new(),
        created_at: now,
        updated_at: now,
        context_window_override: None,
        reasoning_effort_override: None,
        thinking_tokens: None,
        custom_headers: HashMap::new(),
        provider_options: deecodex::providers::provider_options_for_slug("codex"),
        request_timeout_secs: None,
        max_retries: None,
        translate_enabled: false,
        capability_enabled: false,
        capability_account_id: None,
        dev_pipeline_enabled: false,
        dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
        dev_pipeline_command: "/dev-pipeline".into(),
        dev_pipeline_architect_account_id: None,
        dev_pipeline_implementer_account_id: None,
        dev_pipeline_reviewer_account_id: None,
        dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
        dev_pipeline_max_iterations: 3,
        dev_pipeline_show_trace: false,
        dev_pipeline_architect_instruction: String::new(),
        dev_pipeline_implementer_instruction: String::new(),
        dev_pipeline_reviewer_instruction: String::new(),
        endpoints: vec![codex_official_endpoint_config(&account_id)],
    };
    account.normalize_v2();
    account
}

fn same_imported_codex_oauth(
    account: &deecodex::accounts::Account,
    token: &deecodex::oauth_accounts::OAuthToken,
    client_surface: &AccountClientSurface,
) -> bool {
    same_oauth_account(
        account,
        token,
        "codex",
        &AccountClientKind::Codex,
        client_surface,
    )
}

fn same_oauth_account(
    account: &deecodex::accounts::Account,
    token: &deecodex::oauth_accounts::OAuthToken,
    provider: &str,
    client_kind: &AccountClientKind,
    client_surface: &AccountClientSurface,
) -> bool {
    if account.provider != provider
        || &account.client_kind != client_kind
        || &account.client_surface != client_surface
    {
        return false;
    }
    let existing = account
        .client_options
        .get("oauth")
        .and_then(deecodex::oauth_accounts::oauth_token_from_value);
    if let Some(existing) = existing {
        if !token.account_id.trim().is_empty() && existing.account_id == token.account_id {
            return true;
        }
        if !token.refresh_token.trim().is_empty() && existing.refresh_token == token.refresh_token {
            return true;
        }
        if !token.email.trim().is_empty()
            && existing.email == token.email
            && !existing.email.trim().is_empty()
        {
            return true;
        }
    }
    !token.access_token.trim().is_empty() && account.api_key == token.access_token
}

fn surface_has_active_codex_official(
    store: &deecodex::accounts::AccountStore,
    surface: &AccountClientSurface,
) -> bool {
    let Some(active) = store.active_account_for_surface(surface) else {
        return false;
    };
    active
        .active_endpoint(store.active_endpoint_id_for_surface(&AccountClientKind::Codex, surface))
        .or_else(|| active.endpoints.first())
        .is_some_and(|endpoint| endpoint.kind == deecodex::accounts::EndpointKind::CodexOfficial)
}

fn activate_codex_surface_account(
    store: &mut AccountStore,
    account: &deecodex::accounts::Account,
    endpoint_id: Option<String>,
    sync_legacy_global: bool,
) {
    if !account.client_kind.is_codex() {
        return;
    }
    activate_client_surface_account(store, account, endpoint_id, sync_legacy_global);
}

fn activate_client_surface_account(
    store: &mut AccountStore,
    account: &deecodex::accounts::Account,
    endpoint_id: Option<String>,
    sync_legacy_global: bool,
) {
    if !account.client_kind.supports_desktop_surface()
        && account.client_surface != AccountClientSurface::Cli
    {
        return;
    }
    let endpoint_id = endpoint_id.or_else(|| {
        if account.client_kind.is_codex() {
            account
                .endpoints
                .first()
                .map(|endpoint| endpoint.id.clone())
        } else {
            None
        }
    });
    store.set_active_for_surface(
        &account.client_kind,
        &account.client_surface,
        account.id.clone(),
        endpoint_id.clone(),
    );
    if account.client_kind.is_codex() && sync_legacy_global {
        store.active_id = Some(account.id.clone());
        store.active_account_id = Some(account.id.clone());
        store.active_endpoint_id = endpoint_id;
    }
}

fn should_sync_legacy_global(store: &AccountStore, account: &deecodex::accounts::Account) -> bool {
    account.client_surface == AccountClientSurface::Cli || store.active_account_id.is_none()
}

fn dex_assistant_endpoint_for_account<'a>(
    store: &deecodex::accounts::AccountStore,
    account: &'a deecodex::accounts::Account,
) -> Option<&'a deecodex::accounts::EndpointConfig> {
    let selection = store.active_selection_for_dex_assistant();
    let selected_endpoint_id = selection
        .and_then(|selection| {
            if selection.account_id.as_deref() == Some(account.id.as_str()) {
                selection.endpoint_id.as_deref()
            } else {
                None
            }
        })
        .or_else(|| {
            store.active_endpoint_id_for_surface(&AccountClientKind::Codex, &account.client_surface)
        });
    account
        .active_endpoint(selected_endpoint_id)
        .or_else(|| account.endpoints.first())
}

pub(crate) async fn set_dex_assistant_account_inner(
    manager: &ServerManager,
    account_id: String,
    endpoint_id: Option<String>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, (account, endpoint)) =
        mutate_account_store(&data_dir, "保存 DEX 助手账号失败", |store| {
            let mut account = store
                .accounts
                .iter()
                .find(|account| account.id == account_id)
                .cloned()
                .ok_or_else(|| format!("账号不存在: {account_id}"))?;
            if !account.client_kind.is_codex() {
                return Err("DEX 助手只能使用 Codex 代理账号".into());
            }
            account.normalize_v2();
            let endpoint = account
                .active_endpoint(endpoint_id.as_deref())
                .cloned()
                .or_else(|| {
                    store
                        .active_endpoint_id_for_surface(
                            &AccountClientKind::Codex,
                            &account.client_surface,
                        )
                        .and_then(|endpoint_id| account.active_endpoint(Some(endpoint_id)).cloned())
                })
                .or_else(|| account.endpoints.first().cloned())
                .ok_or_else(|| "目标账号没有可用端点".to_string())?;
            validate_endpoint_runtime_urls(&endpoint)?;
            store.set_active_for_dex_assistant(account.id.clone(), Some(endpoint.id.clone()));
            Ok((account, endpoint))
        })?;
    sync_account_store_to_running_state(manager, &store).await;
    Ok(account_to_value_with_endpoint(&account, Some(&endpoint)))
}

fn secret_is_redacted(value: &str) -> bool {
    value.contains("****")
}

fn non_empty_override(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn secret_override(value: Option<String>) -> Option<String> {
    value.filter(|value| {
        let value = value.trim();
        !value.is_empty() && !secret_is_redacted(value)
    })
}

fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("打开 macOS 剪贴板失败: {e}"))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| "剪贴板写入通道不可用".to_string())?
            .write_all(text.as_bytes())
            .map_err(|e| format!("写入剪贴板失败: {e}"))?;
        let status = child
            .wait()
            .map_err(|e| format!("等待剪贴板写入失败: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("剪贴板写入失败: pbcopy 退出状态 {status}"))
        }
    }

    #[cfg(target_os = "windows")]
    {
        let mut child = Command::new("powershell")
            .args(["-NoProfile", "-Command", "Set-Clipboard"])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("打开 Windows 剪贴板失败: {e}"))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| "剪贴板写入通道不可用".to_string())?
            .write_all(text.as_bytes())
            .map_err(|e| format!("写入剪贴板失败: {e}"))?;
        let status = child
            .wait()
            .map_err(|e| format!("等待剪贴板写入失败: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("剪贴板写入失败: Set-Clipboard 退出状态 {status}"))
        }
    }

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        for command in ["wl-copy", "xclip", "xsel"] {
            let args: &[&str] = match command {
                "xclip" => &["-selection", "clipboard"],
                "xsel" => &["--clipboard", "--input"],
                _ => &[],
            };
            let spawn = Command::new(command)
                .args(args)
                .stdin(Stdio::piped())
                .spawn();
            let Ok(mut child) = spawn else {
                continue;
            };
            child
                .stdin
                .as_mut()
                .ok_or_else(|| "剪贴板写入通道不可用".to_string())?
                .write_all(text.as_bytes())
                .map_err(|e| format!("写入剪贴板失败: {e}"))?;
            let status = child
                .wait()
                .map_err(|e| format!("等待剪贴板写入失败: {e}"))?;
            if status.success() {
                return Ok(());
            }
        }
        Err("当前系统没有可用剪贴板命令 wl-copy/xclip/xsel".into())
    }
}

#[tauri::command]
pub async fn copy_account_secret(
    manager: State<'_, ServerManager>,
    account_id: String,
    secret_kind: String,
    endpoint_id: Option<String>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;

    let secret = match secret_kind.as_str() {
        "api_key" | "primary" => account.api_key.as_str(),
        "vision_api_key" | "vision" => {
            let endpoint_secret = endpoint_id
                .as_deref()
                .and_then(|id| account.endpoints.iter().find(|endpoint| endpoint.id == id))
                .or_else(|| account.endpoints.first())
                .map(|endpoint| endpoint.vision.api_key.as_str())
                .filter(|value| !value.trim().is_empty());
            endpoint_secret.unwrap_or(account.vision_api_key.as_str())
        }
        _ => return Err("不支持复制的密钥类型".into()),
    };

    if secret.trim().is_empty() {
        return Err("这个账号没有已保存的密钥".into());
    }

    copy_text_to_clipboard(secret.trim())?;
    Ok(json!({"ok": true}))
}

fn mask_secret(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if value.len() <= 8 {
        return "****".into();
    }
    format!("{}****{}", &value[..4], &value[value.len() - 4..])
}

fn redact_client_options(mut options: HashMap<String, Value>) -> HashMap<String, Value> {
    for key in ["proxy_token"] {
        if let Some(Value::String(value)) = options.get_mut(key) {
            *value = mask_secret(value);
        }
    }
    if let Some(Value::Object(oauth)) = options.get_mut("oauth") {
        for key in ["access_token", "refresh_token", "id_token"] {
            if let Some(Value::String(value)) = oauth.get_mut(key) {
                *value = mask_secret(value);
            }
        }
        for key in ["access_token", "refresh_token", "id_token"] {
            let present_key = format!("{key}_present");
            let present = oauth
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty());
            oauth.insert(present_key, Value::Bool(present));
        }
    }
    options
}

fn redact_endpoints(
    endpoints: &[deecodex::accounts::EndpointConfig],
) -> Vec<deecodex::accounts::EndpointConfig> {
    endpoints
        .iter()
        .cloned()
        .map(|mut endpoint| {
            endpoint.vision.api_key = mask_secret(&endpoint.vision.api_key);
            endpoint
        })
        .collect()
}

fn restore_redacted_client_options(
    incoming: &mut HashMap<String, Value>,
    existing: &HashMap<String, Value>,
) {
    if incoming
        .get("proxy_token")
        .and_then(Value::as_str)
        .is_some_and(secret_is_redacted)
    {
        if let Some(value) = existing.get("proxy_token").cloned() {
            incoming.insert("proxy_token".into(), value);
        }
    }
    if let Some(incoming_oauth) = incoming.get_mut("oauth").and_then(Value::as_object_mut) {
        let existing_oauth = existing.get("oauth").and_then(Value::as_object);
        for key in ["access_token", "refresh_token", "id_token"] {
            let redacted = incoming_oauth
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(secret_is_redacted);
            if redacted {
                if let Some(value) = existing_oauth.and_then(|oauth| oauth.get(key)).cloned() {
                    incoming_oauth.insert(key.into(), value);
                }
            }
        }
        for key in [
            "access_token_present",
            "refresh_token_present",
            "id_token_present",
        ] {
            incoming_oauth.remove(key);
        }
    }
}

fn restore_redacted_account_secrets(
    incoming: &mut deecodex::accounts::Account,
    existing: &deecodex::accounts::Account,
) {
    if secret_is_redacted(&incoming.api_key) {
        incoming.api_key = existing.api_key.clone();
    }
    if secret_is_redacted(&incoming.vision_api_key) {
        incoming.vision_api_key = existing.vision_api_key.clone();
    }
    for (idx, endpoint) in incoming.endpoints.iter_mut().enumerate() {
        if secret_is_redacted(&endpoint.vision.api_key) {
            if let Some(existing_endpoint) = existing.endpoints.get(idx) {
                endpoint.vision.api_key = existing_endpoint.vision.api_key.clone();
            }
        }
    }
    restore_redacted_client_options(&mut incoming.client_options, &existing.client_options);
}

#[derive(Debug, Clone)]
struct OAuthLoginSession {
    provider: deecodex::oauth_accounts::OAuthProvider,
    client_kind: AccountClientKind,
    client_surface: AccountClientSurface,
    mode: String,
    pkce: Option<deecodex::oauth_accounts::PkceCodes>,
    auth_url: String,
    verification_url: Option<String>,
    user_code: Option<String>,
    device_auth_id: Option<String>,
    poll_interval_secs: u64,
    last_device_poll_at: u64,
    expires_at: u64,
    callback_code: Option<String>,
    callback_error: Option<String>,
    account_id: Option<String>,
    completed_account: Option<Value>,
}

type OAuthSessionMap = dashmap::DashMap<String, Arc<tokio::sync::Mutex<OAuthLoginSession>>>;

static OAUTH_SESSIONS: OnceLock<OAuthSessionMap> = OnceLock::new();
static OAUTH_CODEX_CALLBACK_STARTED: OnceLock<()> = OnceLock::new();
static OAUTH_CLAUDE_CALLBACK_STARTED: OnceLock<()> = OnceLock::new();

fn oauth_sessions() -> &'static OAuthSessionMap {
    OAUTH_SESSIONS.get_or_init(dashmap::DashMap::new)
}

fn oauth_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("创建 OAuth HTTP 客户端失败: {e}"))
}

#[tauri::command]
pub async fn start_oauth_account_login(
    provider: String,
    client_kind: Option<String>,
    client_surface: Option<String>,
    mode: Option<String>,
) -> Result<Value, String> {
    let provider =
        deecodex::oauth_accounts::OAuthProvider::parse(&provider).map_err(|e| e.to_string())?;
    let mode = mode.unwrap_or_else(|| "browser".into());
    let client_kind = client_kind
        .as_deref()
        .map(parse_account_client_kind)
        .unwrap_or_else(|| {
            if provider == deecodex::oauth_accounts::OAuthProvider::Claude {
                AccountClientKind::ClaudeCode
            } else {
                AccountClientKind::Codex
            }
        });
    let client_surface = client_surface
        .as_deref()
        .map(|value| parse_account_client_surface(value, &client_kind))
        .unwrap_or_default();
    let state = deecodex::oauth_accounts::generate_state().map_err(|e| e.to_string())?;
    let client = oauth_http_client()?;

    let session = if mode == "device" {
        if provider != deecodex::oauth_accounts::OAuthProvider::Codex {
            return Err("设备码登录仅支持 Codex".into());
        }
        let device = deecodex::oauth_accounts::request_codex_device_user_code(&client)
            .await
            .map_err(|e| format!("获取 Codex 设备码失败: {e}"))?;
        OAuthLoginSession {
            provider,
            client_kind,
            client_surface: client_surface.clone(),
            mode: mode.clone(),
            pkce: None,
            auth_url: device.verification_url.clone(),
            verification_url: Some(device.verification_url),
            user_code: Some(device.user_code),
            device_auth_id: Some(device.device_auth_id),
            poll_interval_secs: device.interval_secs,
            last_device_poll_at: 0,
            expires_at: device.expires_at,
            callback_code: None,
            callback_error: None,
            account_id: None,
            completed_account: None,
        }
    } else {
        let pkce = deecodex::oauth_accounts::generate_pkce_codes().map_err(|e| e.to_string())?;
        let auth_url = deecodex::oauth_accounts::auth_url(&provider, &state, &pkce);
        ensure_oauth_callback_server(&provider);
        OAuthLoginSession {
            provider,
            client_kind,
            client_surface: client_surface.clone(),
            mode: "browser".into(),
            pkce: Some(pkce),
            auth_url,
            verification_url: None,
            user_code: None,
            device_auth_id: None,
            poll_interval_secs: 5,
            last_device_poll_at: 0,
            expires_at: deecodex::oauth_accounts::now_secs().saturating_add(10 * 60),
            callback_code: None,
            callback_error: None,
            account_id: None,
            completed_account: None,
        }
    };

    let response = json!({
        "state": state,
        "provider": session.provider.as_str(),
        "client_surface": session.client_surface,
        "mode": session.mode,
        "url": session.auth_url,
        "verification_url": session.verification_url,
        "user_code": session.user_code,
        "expires_at": session.expires_at,
        "poll_interval_secs": session.poll_interval_secs,
    });
    oauth_sessions().insert(state, Arc::new(tokio::sync::Mutex::new(session)));
    Ok(response)
}

#[tauri::command]
pub async fn poll_oauth_account_login(
    manager: State<'_, ServerManager>,
    state: String,
) -> Result<Value, String> {
    let Some(entry) = oauth_sessions().get(&state).map(|entry| entry.clone()) else {
        return Ok(json!({"status": "expired", "message": "OAuth 登录会话不存在或已过期"}));
    };
    let now = deecodex::oauth_accounts::now_secs();
    let mut session = entry.lock().await;
    if let Some(account) = session.completed_account.clone() {
        return Ok(json!({"status": "success", "account": account}));
    }
    if now > session.expires_at {
        oauth_sessions().remove(&state);
        return Ok(json!({"status": "expired", "message": "OAuth 登录已超时"}));
    }
    if let Some(error) = session.callback_error.clone() {
        oauth_sessions().remove(&state);
        return Ok(json!({"status": "error", "message": error}));
    }

    let token = if session.mode == "device" {
        if now
            < session
                .last_device_poll_at
                .saturating_add(session.poll_interval_secs.max(1))
        {
            return Ok(json!({"status": "pending"}));
        }
        session.last_device_poll_at = now;
        let client = oauth_http_client()?;
        let device_auth_id = session.device_auth_id.clone().unwrap_or_default();
        let user_code = session.user_code.clone().unwrap_or_default();
        match deecodex::oauth_accounts::poll_codex_device_token(
            &client,
            &device_auth_id,
            &user_code,
        )
        .await
        .map_err(|e| format!("轮询 Codex 设备码失败: {e}"))?
        {
            Some((code, verifier, challenge)) => {
                deecodex::oauth_accounts::exchange_codex_device_code(
                    &client, &code, &verifier, &challenge,
                )
                .await
                .map_err(|e| format!("Codex 设备码换 token 失败: {e}"))?
            }
            None => return Ok(json!({"status": "pending"})),
        }
    } else {
        let Some(code) = session.callback_code.clone() else {
            return Ok(json!({"status": "pending"}));
        };
        let pkce = session
            .pkce
            .clone()
            .ok_or_else(|| "OAuth 登录会话缺少 PKCE 信息".to_string())?;
        let client = oauth_http_client()?;
        deecodex::oauth_accounts::exchange_code(&client, &session.provider, &code, &state, &pkce)
            .await
            .map_err(|e| format!("OAuth code 换 token 失败: {e}"))?
    };

    let account = create_oauth_account(&manager, &session, token).await?;
    session.account_id = account
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string);
    session.completed_account = Some(account.clone());
    oauth_sessions().remove(&state);
    Ok(json!({"status": "success", "account": account}))
}

#[tauri::command]
pub async fn cancel_oauth_account_login(state: String) -> Result<Value, String> {
    oauth_sessions().remove(&state);
    Ok(json!({"ok": true}))
}

fn parse_account_client_kind(value: &str) -> AccountClientKind {
    match value {
        "codex" | "codex_cli" | "codex_desktop" | "Codex" => AccountClientKind::Codex,
        "claude_cli" | "claude_desktop" => AccountClientKind::ClaudeCode,
        "claude_code" | "ClaudeCode" => AccountClientKind::ClaudeCode,
        "openclaw" | "Openclaw" => AccountClientKind::Openclaw,
        "hermes" | "Hermes" => AccountClientKind::Hermes,
        "generic_client" | "GenericClient" => AccountClientKind::GenericClient,
        _ => AccountClientKind::Codex,
    }
}

fn parse_account_client_surface(value: &str, kind: &AccountClientKind) -> AccountClientSurface {
    if !kind.supports_desktop_surface() {
        return AccountClientSurface::Cli;
    }
    match value {
        "desktop" | "Desktop" => AccountClientSurface::Desktop,
        _ => AccountClientSurface::Cli,
    }
}

fn ensure_oauth_callback_server(provider: &deecodex::oauth_accounts::OAuthProvider) {
    match provider {
        deecodex::oauth_accounts::OAuthProvider::Codex => {
            let _ = OAUTH_CODEX_CALLBACK_STARTED.get_or_init(|| {
                tokio::spawn(oauth_callback_server(1455));
            });
        }
        deecodex::oauth_accounts::OAuthProvider::Claude => {
            let _ = OAUTH_CLAUDE_CALLBACK_STARTED.get_or_init(|| {
                tokio::spawn(oauth_callback_server(54545));
            });
        }
    };
}

async fn oauth_callback_server(port: u16) {
    use axum::{extract::Query, response::Html, routing::get, Router};
    use std::collections::HashMap;

    async fn callback(Query(query): Query<HashMap<String, String>>) -> Html<&'static str> {
        let state = query.get("state").cloned().unwrap_or_default();
        let code = query.get("code").cloned();
        let error = query
            .get("error_description")
            .or_else(|| query.get("error"))
            .cloned();
        if !state.is_empty() {
            if let Some(entry) = oauth_sessions().get(&state).map(|entry| entry.clone()) {
                let mut session = entry.lock().await;
                if let Some(error) = error {
                    session.callback_error = Some(error);
                } else if let Some(code) = code {
                    session.callback_code = Some(code);
                } else {
                    session.callback_error = Some("OAuth 回调缺少 code".into());
                }
            }
        }
        Html("OAuth 登录完成，可以回到 deecodex。")
    }

    let app = Router::new()
        .route("/auth/callback", get(callback))
        .route("/callback", get(callback));
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => {
            if let Err(err) = axum::serve(listener, app).await {
                tracing::warn!("OAuth callback server stopped: {err}");
            }
        }
        Err(err) => {
            tracing::warn!("OAuth callback server bind failed on {addr}: {err}");
        }
    }
}

async fn create_oauth_account(
    manager: &ServerManager,
    session: &OAuthLoginSession,
    token: deecodex::oauth_accounts::OAuthToken,
) -> Result<Value, String> {
    use deecodex::accounts::{
        generate_id, now_secs, Account, AccountAuthMode, EndpointConfig, EndpointKind, VisionMode,
    };

    let data_dir = manager.data_dir.lock().await.clone();
    let (host, port) = service_endpoint_for_manager(manager).await;
    let now = now_secs();
    let account_id = generate_id();
    let oauth_value = deecodex::oauth_accounts::oauth_token_to_value(&token, &session.mode);
    let mut client_options = HashMap::new();
    client_options.insert("oauth".into(), oauth_value);
    client_options.insert("auth_mode".into(), json!("oauth"));

    let (name, provider, client_kind, upstream, default_model, endpoint) = match session.provider {
        deecodex::oauth_accounts::OAuthProvider::Codex => {
            let endpoint = EndpointConfig {
                id: format!("endpoint_{account_id}"),
                name: "Codex 官方".into(),
                kind: EndpointKind::CodexOfficial,
                base_url: deecodex::handlers::CODEX_OFFICIAL_BASE_URL.into(),
                path: "responses".into(),
                template_id: "codex_official".into(),
                template_version: 1,
                model_map: HashMap::new(),
                known_models: Vec::new(),
                model_profiles: HashMap::new(),
                vision: deecodex::accounts::VisionConfig {
                    mode: VisionMode::Native,
                    ..Default::default()
                },
                image_generation_enabled: Some(true),
                custom_headers: HashMap::new(),
                request_timeout_secs: None,
                max_retries: None,
                context_window_override: None,
                reasoning_effort_override: None,
                thinking_tokens: None,
                fast_mode_enabled: false,
                fast_service_tier: "priority".into(),
                balance_url: String::new(),
            };
            (
                oauth_account_name("Codex", &token.email),
                "codex".into(),
                AccountClientKind::Codex,
                deecodex::handlers::CODEX_OFFICIAL_BASE_URL.into(),
                String::new(),
                Some(endpoint),
            )
        }
        deecodex::oauth_accounts::OAuthProvider::Claude => {
            client_options.insert(
                "api_key_env".into(),
                Value::String("ANTHROPIC_AUTH_TOKEN".into()),
            );
            client_options.insert(
                "auth_env".into(),
                Value::String("ANTHROPIC_AUTH_TOKEN".into()),
            );
            client_options.insert("proxy_recording_enabled".into(), Value::Bool(true));
            client_options.insert(
                "proxy_base_url".into(),
                Value::String(client_proxy_base_url(
                    &host,
                    port,
                    &AccountClientKind::ClaudeCode,
                )),
            );
            client_options.insert(
                "proxy_token".into(),
                Value::String(format!("dee_{account_id}_{}", generate_id())),
            );
            (
                oauth_account_name("Claude", &token.email),
                "anthropic".into(),
                session.client_kind.clone(),
                "https://api.anthropic.com".into(),
                "claude-sonnet-4-5".into(),
                None,
            )
        }
    };

    let mut account = Account {
        id: account_id,
        name,
        provider,
        client_kind,
        client_surface: session.client_surface.clone(),
        wire_protocol: Default::default(),
        upstream,
        api_key: token.access_token.clone(),
        auth_mode: AccountAuthMode::OAuth,
        default_model,
        client_options,
        runtime_state: Default::default(),
        last_applied_at: None,
        last_check: None,
        model_map: HashMap::new(),
        vision_upstream: String::new(),
        vision_api_key: String::new(),
        vision_model: String::new(),
        vision_endpoint: String::new(),
        vision_enabled: false,
        from_codex_config: false,
        balance_url: String::new(),
        created_at: now,
        updated_at: now,
        context_window_override: None,
        reasoning_effort_override: None,
        thinking_tokens: None,
        custom_headers: HashMap::new(),
        provider_options: HashMap::new(),
        request_timeout_secs: None,
        max_retries: None,
        translate_enabled: true,
        capability_enabled: false,
        capability_account_id: None,
        dev_pipeline_enabled: false,
        dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
        dev_pipeline_command: "/dev-pipeline".into(),
        dev_pipeline_architect_account_id: None,
        dev_pipeline_implementer_account_id: None,
        dev_pipeline_reviewer_account_id: None,
        dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
        dev_pipeline_max_iterations: 3,
        dev_pipeline_show_trace: false,
        dev_pipeline_architect_instruction: String::new(),
        dev_pipeline_implementer_instruction: String::new(),
        dev_pipeline_reviewer_instruction: String::new(),
        endpoints: endpoint.into_iter().collect(),
    };
    account.provider_options = deecodex::providers::provider_options_for_slug(&account.provider);
    if !account.client_kind.is_codex() {
        account.translate_enabled = false;
    }
    account.normalize_v2();

    let (store, (account, became_active)) =
        mutate_account_store(&data_dir, "保存 OAuth 账号失败", |store| {
            let became_active = account.client_kind.is_codex();
            let mut saved_account = account.clone();
            if let Some(existing) = store.accounts.iter_mut().find(|existing| {
                same_oauth_account(
                    existing,
                    &token,
                    &account.provider,
                    &account.client_kind,
                    &account.client_surface,
                )
            }) {
                existing.provider = account.provider.clone();
                existing.client_kind = account.client_kind.clone();
                existing.client_surface = account.client_surface.clone();
                existing.upstream = account.upstream.clone();
                existing.api_key = token.access_token.clone();
                existing.auth_mode = AccountAuthMode::OAuth;
                existing.default_model = account.default_model.clone();
                for (key, value) in &account.client_options {
                    if key == "proxy_token" && existing.client_options.contains_key(key) {
                        continue;
                    }
                    existing.client_options.insert(key.clone(), value.clone());
                }
                if existing.endpoints.is_empty() {
                    existing.endpoints = account.endpoints.clone();
                }
                existing.provider_options = account.provider_options.clone();
                existing.updated_at = now;
                existing.normalize_v2();
                saved_account = existing.clone();
            } else {
                store.accounts.push(saved_account.clone());
            }

            if became_active {
                let endpoint_id = saved_account
                    .endpoints
                    .first()
                    .map(|endpoint| endpoint.id.clone());
                let sync_legacy_global = should_sync_legacy_global(store, &saved_account);
                activate_codex_surface_account(
                    store,
                    &saved_account,
                    endpoint_id,
                    sync_legacy_global,
                );
            }
            Ok((saved_account, became_active))
        })?;
    if became_active {
        if account.client_surface == AccountClientSurface::Cli
            || store.active_account_id.as_deref() == Some(&account.id)
        {
            sync_active_account_to_running_state(manager, &store, &account).await?;
        } else {
            sync_account_store_to_running_state(manager, &store).await;
        }
    } else {
        sync_account_store_to_running_state(manager, &store).await;
    }
    Ok(account_to_value(&account))
}

fn oauth_account_name(prefix: &str, email: &str) -> String {
    let email = email.trim();
    if email.is_empty() {
        format!("{prefix} 官方账号")
    } else {
        format!("{prefix} · {email}")
    }
}

// ── 前端返回类型 ──────────────────────────────────────────────────────────

#[tauri::command]
pub fn start_window_drag(window: WebviewWindow) -> Result<(), String> {
    window
        .start_dragging()
        .map_err(|err| format!("窗口拖动失败: {err}"))
}

#[derive(Serialize, Clone)]
pub struct ServiceInfo {
    pub running: bool,
    pub host: String,
    pub port: u16,
    pub uptime_secs: Option<u64>,
    pub version: String,
    pub cdp_port: u16,
    pub codex_launch_with_cdp: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GuiConfig {
    #[serde(default = "deecodex::config::default_host")]
    pub host: String,
    pub port: u16,
    pub upstream: String,
    pub api_key: String,
    pub model_map: String,
    pub chinese_thinking: bool,
    pub codex_auto_inject: bool,
    pub codex_persistent_inject: bool,
    pub vision_upstream: String,
    pub vision_api_key: String,
    pub vision_model: String,
    pub vision_endpoint: String,
    pub token_anomaly_prompt_max: u32,
    pub token_anomaly_spike_ratio: f64,
    pub token_anomaly_burn_window: u64,
    pub token_anomaly_burn_rate: u32,
    pub allowed_mcp_servers: String,
    pub allowed_computer_displays: String,
    pub computer_executor: String,
    pub computer_executor_timeout_secs: u64,
    pub mcp_executor_config: String,
    pub mcp_executor_timeout_secs: u64,
    pub max_body_mb: u32,
    pub prompts_dir: String,
    pub playwright_state_dir: String,
    pub browser_use_bridge_url: String,
    pub browser_use_bridge_command: String,
    pub data_dir: String,
    pub codex_launch_with_cdp: bool,
    pub cdp_port: u16,
}

impl From<Args> for GuiConfig {
    fn from(a: Args) -> Self {
        Self {
            host: deecodex::config::normalize_host(&a.host),
            port: a.port,
            upstream: a.upstream,
            api_key: a.api_key,
            model_map: a.model_map,
            chinese_thinking: a.chinese_thinking,
            codex_auto_inject: a.codex_auto_inject,
            codex_persistent_inject: a.codex_persistent_inject,
            vision_upstream: a.vision_upstream,
            vision_api_key: a.vision_api_key,
            vision_model: a.vision_model,
            vision_endpoint: a.vision_endpoint,
            token_anomaly_prompt_max: a.token_anomaly_prompt_max,
            token_anomaly_spike_ratio: a.token_anomaly_spike_ratio,
            token_anomaly_burn_window: a.token_anomaly_burn_window,
            token_anomaly_burn_rate: a.token_anomaly_burn_rate,
            allowed_mcp_servers: a.allowed_mcp_servers,
            allowed_computer_displays: a.allowed_computer_displays,
            computer_executor: a.computer_executor,
            computer_executor_timeout_secs: a.computer_executor_timeout_secs,
            mcp_executor_config: a.mcp_executor_config,
            mcp_executor_timeout_secs: a.mcp_executor_timeout_secs,
            max_body_mb: a.max_body_mb as u32,
            prompts_dir: a.prompts_dir.to_string_lossy().to_string(),
            playwright_state_dir: a.playwright_state_dir,
            browser_use_bridge_url: a.browser_use_bridge_url,
            browser_use_bridge_command: a.browser_use_bridge_command,
            data_dir: a.data_dir.to_string_lossy().to_string(),
            codex_launch_with_cdp: a.codex_launch_with_cdp,
            cdp_port: a.cdp_port,
        }
    }
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn normalize_data_dir(data_dir: impl Into<std::path::PathBuf>) -> std::path::PathBuf {
    let data_dir = data_dir.into();
    if data_dir.is_absolute() {
        return data_dir;
    }
    if let Some(home) = deecodex::config::home_dir() {
        home.join(data_dir)
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(data_dir)
    } else {
        data_dir
    }
}

fn sync_data_dir_env_file(data_dir: &Path, key: &str, value: &str) {
    let path = data_dir.join(".env");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let prefix = format!("{key}=");
    let new_line = format!("{key}={value}");
    let replaced = if content.lines().any(|line| line.starts_with(&prefix)) {
        content
            .lines()
            .map(|line| {
                if line.starts_with(&prefix) {
                    new_line.as_str()
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else if content.is_empty() {
        new_line
    } else {
        format!("{content}\n{new_line}")
    };
    let _ = std::fs::write(path, replaced);
}

pub(crate) struct RuntimeDefaults {
    pub preview: bool,
    pub port: u16,
    pub data_dir: PathBuf,
}

pub(crate) fn runtime_defaults() -> RuntimeDefaults {
    let preview = option_env!("DEX_AI_PREVIEW_BUILD")
        .is_some_and(|value| matches!(value, "1" | "true" | "yes" | "preview"));
    let data_dir_name = if preview {
        ".deecodex-preview"
    } else {
        ".deecodex"
    };
    RuntimeDefaults {
        preview,
        port: if preview { 4556 } else { 4446 },
        data_dir: deecodex::config::home_dir()
            .map(|home| home.join(data_dir_name))
            .unwrap_or_else(|| PathBuf::from(data_dir_name)),
    }
}

fn apply_runtime_data_dir_default(
    mut args: Args,
    defaults: &RuntimeDefaults,
    data_dir_env_configured: bool,
) -> Args {
    if defaults.preview
        && !data_dir_env_configured
        && args.data_dir.as_path() == Path::new(".deecodex")
    {
        args.data_dir = defaults.data_dir.clone();
    }
    args
}

fn apply_runtime_port_default(
    mut args: Args,
    defaults: &RuntimeDefaults,
    port_file_configured: bool,
    port_env_configured: bool,
) -> Args {
    if defaults.preview && !port_file_configured && !port_env_configured && args.port == 4446 {
        args.port = defaults.port;
    }
    args
}

struct AccountBackedConfig {
    upstream: String,
    api_key: String,
    model_map: String,
    vision_upstream: String,
    vision_api_key: String,
    vision_model: String,
    vision_endpoint: String,
}

fn account_backed_config(existing: Option<&Args>) -> AccountBackedConfig {
    AccountBackedConfig {
        upstream: existing.map(|a| a.upstream.clone()).unwrap_or_default(),
        api_key: existing.map(|a| a.api_key.clone()).unwrap_or_default(),
        model_map: existing.map(|a| a.model_map.clone()).unwrap_or_default(),
        vision_upstream: existing
            .map(|a| a.vision_upstream.clone())
            .unwrap_or_default(),
        vision_api_key: existing
            .map(|a| a.vision_api_key.clone())
            .unwrap_or_default(),
        vision_model: existing.map(|a| a.vision_model.clone()).unwrap_or_default(),
        vision_endpoint: existing
            .map(|a| a.vision_endpoint.clone())
            .unwrap_or_default(),
    }
}

pub(crate) fn load_args() -> Args {
    let defaults = runtime_defaults();
    let port_env_configured = std::env::var_os("DEECODEX_PORT").is_some();
    let data_dir_env_configured = std::env::var_os("DEECODEX_DATA_DIR").is_some();
    // 从环境变量 + 默认值构建 Args
    let args = match Args::try_parse_from(["deecodex-gui"]) {
        Ok(a) => a,
        Err(_) => {
            return Args::try_parse_from(["deecodex-gui"]).unwrap_or_else(|_| {
                // clap 失败时返回纯默认值
                Args {
                    command: None,
                    config: None,
                    host: deecodex::config::default_host(),
                    port: defaults.port,
                    upstream: "https://openrouter.ai/api/v1".into(),
                    api_key: String::new(),
                    model_map: "{}".into(),
                    max_body_mb: 100,
                    vision_upstream: String::new(),
                    vision_api_key: String::new(),
                    vision_model: "MiniMax-M1".into(),
                    vision_endpoint: "v1/coding_plan/vlm".into(),
                    chinese_thinking: false,
                    codex_auto_inject: true,
                    codex_persistent_inject: false,
                    prompts_dir: "prompts".into(),
                    data_dir: defaults.data_dir,
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
                    codex_launch_with_cdp: false,
                    cdp_port: 9222,
                }
            });
        }
    };
    let mut args = apply_runtime_data_dir_default(args, &defaults, data_dir_env_configured);
    // 先确保 data_dir 为绝对路径，再合并配置文件；否则 dev 模式会去
    // deecodex-gui/.deecodex 读配置，导致 GUI 保存到 HOME 后又读回默认值。
    if args.data_dir.is_relative() {
        args.data_dir = normalize_data_dir(args.data_dir);
    }
    let config_path = match &args.config {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => Args::default_config_path(&args.data_dir),
    };
    let port_file_configured = Args::load_from_file(&config_path).is_some();
    let mut args = args.merge_with_file();
    args = apply_runtime_port_default(args, &defaults, port_file_configured, port_env_configured);
    args.host = deecodex::config::normalize_host(&args.host);
    // 文件里的旧 data_dir 也可能仍是相对路径，合并后再规整一次。
    if args.data_dir.is_relative() {
        args.data_dir = normalize_data_dir(args.data_dir);
    }
    args
}

async fn service_endpoint_for_manager(manager: &ServerManager) -> (String, u16) {
    if manager.is_running().await {
        let host = manager.host.lock().await.clone();
        let port = *manager.port.lock().await;
        (host, port)
    } else {
        let args = load_args();
        (args.host, args.port)
    }
}

/// 执行首次启动迁移：如果 accounts.json 不存在，从旧配置和 Codex config 迁移账号。
/// 返回迁移后的 AccountStore（已持久化）。
fn migrate_or_load_accounts(data_dir: &std::path::Path) -> AccountStore {
    use deecodex::accounts::{
        generate_id, get_provider_presets, guess_provider, now_secs, Account, AccountStore,
    };

    let path = deecodex::accounts::accounts_file_path(data_dir);

    // 已有账号文件，直接加载
    if path.exists() {
        tracing::info!("加载已有账号文件: {}", path.display());
        let mut store = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| deecodex::accounts::parse_account_store(&content).ok())
        {
            Some(store) => store,
            None => return deecodex::accounts::load_accounts(data_dir),
        };
        store.normalize_v2();
        if let Err(e) = deecodex::accounts::save_accounts(data_dir, &store) {
            tracing::warn!("保存规范化后的账号文件失败: {e}");
        }
        return store;
    }

    tracing::info!("accounts.json 不存在，执行首次迁移");

    let mut accounts: Vec<Account> = Vec::new();

    // a. 检查 config.json 是否有自定义上游/Key
    let config_path = Args::default_config_path(data_dir);
    if let Some(file_args) = Args::load_from_file(&config_path) {
        // 上游非默认 OpenRouter 或 Key 不为空 → 迁移旧配置
        let has_custom_upstream = file_args.upstream != "https://openrouter.ai/api/v1";
        let has_api_key = !file_args.api_key.is_empty();
        if has_custom_upstream || has_api_key {
            let model_map: HashMap<String, String> =
                if file_args.model_map.is_empty() || file_args.model_map == "{}" {
                    HashMap::new()
                } else {
                    serde_json::from_str(&file_args.model_map).unwrap_or_default()
                };

            let provider = if has_custom_upstream {
                guess_provider(&file_args.upstream)
            } else {
                "openrouter"
            };

            let migrated = Account {
                id: generate_id(),
                name: "旧配置导入".into(),
                provider: provider.to_string(),
                client_kind: Default::default(),
                client_surface: Default::default(),
                wire_protocol: Default::default(),
                upstream: file_args.upstream.clone(),
                api_key: file_args.api_key.clone(),
                auth_mode: Default::default(),
                default_model: String::new(),
                client_options: HashMap::new(),
                runtime_state: Default::default(),
                last_applied_at: None,
                last_check: None,
                model_map,
                vision_upstream: file_args.vision_upstream.clone(),
                vision_api_key: file_args.vision_api_key.clone(),
                vision_model: file_args.vision_model.clone(),
                vision_endpoint: file_args.vision_endpoint.clone(),
                vision_enabled: false,
                from_codex_config: false,
                balance_url: String::new(),
                created_at: now_secs(),
                updated_at: now_secs(),
                context_window_override: None,
                reasoning_effort_override: None,
                thinking_tokens: None,
                custom_headers: HashMap::new(),
                provider_options: deecodex::providers::provider_options_for_slug(provider),
                request_timeout_secs: None,
                max_retries: None,
                translate_enabled: true,
                capability_enabled: false,
                capability_account_id: None,
                dev_pipeline_enabled: false,
                dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
                dev_pipeline_command: "/dev-pipeline".into(),
                dev_pipeline_architect_account_id: None,
                dev_pipeline_implementer_account_id: None,
                dev_pipeline_reviewer_account_id: None,
                dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
                dev_pipeline_max_iterations: 3,
                dev_pipeline_show_trace: false,
                dev_pipeline_architect_instruction: String::new(),
                dev_pipeline_implementer_instruction: String::new(),
                dev_pipeline_reviewer_instruction: String::new(),
                endpoints: Vec::new(),
            };
            tracing::info!("从 config.json 导入旧配置账号: provider={}", provider);
            accounts.push(migrated);
        }
    }

    // b. 从 Codex config.toml 导入
    if let Some(codex_account) = deecodex::codex_config::extract_account_from_codex_config() {
        // 避免重复（如果旧配置已经包含了同样的 upstream）
        let is_duplicate = accounts.iter().any(|a| {
            a.from_codex_config
                || (a.upstream == codex_account.upstream && a.api_key == codex_account.api_key)
        });
        if !is_duplicate {
            accounts.push(codex_account);
        }
    }

    // c. 都没有 → 创建默认 OpenRouter 空账号
    if accounts.is_empty() {
        let presets = get_provider_presets();
        let openrouter = presets.iter().find(|p| p.slug == "openrouter").unwrap();
        let default = Account {
            id: generate_id(),
            name: "默认账号".into(),
            provider: "openrouter".into(),
            client_kind: Default::default(),
            client_surface: Default::default(),
            wire_protocol: openrouter.wire_protocol.clone(),
            upstream: openrouter.default_upstream.clone(),
            api_key: String::new(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: now_secs(),
            updated_at: now_secs(),
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: deecodex::providers::provider_options_for_slug("openrouter"),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        };
        tracing::info!("创建默认 OpenRouter 空账号");
        accounts.push(default);
    }

    let mut store = AccountStore {
        version: deecodex::accounts::ACCOUNT_STORE_VERSION,
        active_id: Some(accounts[0].id.clone()),
        active_account_id: Some(accounts[0].id.clone()),
        active_endpoint_id: None,
        active_by_surface: HashMap::new(),
        accounts,
    };
    store.normalize_v2();

    // 持久化
    if let Err(e) = deecodex::accounts::save_accounts(data_dir, &store) {
        tracing::warn!("保存迁移后的账号文件失败: {e}");
    } else {
        tracing::info!("首次迁移完成，已保存 {} 个账号", store.accounts.len());
    }

    store
}

/// 从账号存储中读取活跃账号的上下文窗口覆盖值。
fn load_active_account_context_window(data_dir: &std::path::Path) -> Option<u32> {
    let store = deecodex::accounts::load_accounts(data_dir);
    store
        .active_endpoint()
        .and_then(|endpoint| endpoint.context_window_override)
}

fn build_app_state(args: &Args) -> anyhow::Result<handlers::AppState> {
    // 迁移/加载账号
    let account_store = migrate_or_load_accounts(&args.data_dir);

    // 解析活跃账号的配置
    let mut active_account = account_store
        .active_account_id
        .as_ref()
        .or(account_store.active_id.as_ref())
        .and_then(|id| account_store.accounts.iter().find(|a| &a.id == id))
        .filter(|account| account.client_kind.is_codex())
        .or_else(|| {
            account_store
                .accounts
                .iter()
                .find(|account| account.client_kind.is_codex())
        })
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!("没有可用于 deecodex 代理的 Codex 账号，请先创建 Codex 客户端账号")
        })?;

    let active_endpoint = active_account
        .active_endpoint(account_store.active_endpoint_id.as_deref())
        .cloned()
        .or_else(|| active_account.endpoints.first().cloned())
        .ok_or_else(|| anyhow::anyhow!("没有可用于 deecodex 代理的 Codex 账号端点"))?;
    active_account.sync_legacy_from_endpoint(&active_endpoint);

    let model_map: HashMap<String, String> = active_endpoint.model_map.clone();
    let upstream = handlers::validate_upstream(&active_endpoint.base_url).unwrap_or_else(|_| {
        tracing::warn!("活跃账号上游 URL 无效，使用默认 OpenRouter");
        handlers::validate_upstream("https://openrouter.ai/api/v1").unwrap()
    });

    let vision_upstream = if active_endpoint.vision.base_url.is_empty() {
        None
    } else {
        match handlers::validate_upstream(&active_endpoint.vision.base_url) {
            Ok(url) => Some(url),
            Err(e) => {
                tracing::warn!("视觉上游 URL 无效: {e}");
                None
            }
        }
    };

    let file_store = files::FileStore::with_data_dir(&args.data_dir)?;
    let vs_registry = vector_stores::VectorStoreRegistry::with_data_dir(&args.data_dir)?;

    let executors = deecodex::executor::LocalExecutorConfig::from_raw(
        &args.computer_executor,
        args.computer_executor_timeout_secs,
        &args.browser_use_bridge_url,
        &args.browser_use_bridge_command,
        &args.mcp_executor_config,
        args.mcp_executor_timeout_secs,
    )?;

    let rate_limiter = {
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
            Some(Arc::new(deecodex::ratelimit::RateLimiter::new(
                rate_limit,
                rate_window,
            )))
        } else {
            None
        }
    };

    let vision_api_key = active_endpoint.vision.api_key.clone();
    let vision_model = if active_endpoint.vision.model.is_empty() {
        args.vision_model.clone()
    } else {
        active_endpoint.vision.model.clone()
    };
    let vision_endpoint = if active_endpoint.vision.path.is_empty() {
        args.vision_endpoint.clone()
    } else {
        active_endpoint.vision.path.clone()
    };

    Ok(handlers::AppState {
        sessions: deecodex::session::SessionStore::new(),
        client: reqwest::Client::builder()
            .pool_idle_timeout(None)
            .pool_max_idle_per_host(4)
            .timeout(std::time::Duration::from_secs(300))
            .build()?,
        upstream: Arc::new(tokio::sync::RwLock::new(upstream)),
        api_key: Arc::new(tokio::sync::RwLock::new(active_account.api_key.clone())),
        model_map: Arc::new(tokio::sync::RwLock::new(model_map.clone())),
        vision_upstream: Arc::new(tokio::sync::RwLock::new(vision_upstream)),
        vision_api_key: Arc::new(tokio::sync::RwLock::new(vision_api_key)),
        vision_model: Arc::new(tokio::sync::RwLock::new(vision_model)),
        vision_endpoint: Arc::new(tokio::sync::RwLock::new(vision_endpoint)),
        start_time: std::time::Instant::now(),
        request_cache: deecodex::cache::RequestCache::default(),
        prompts: Arc::new(deecodex::prompts::PromptRegistry::new(&args.prompts_dir)),
        files: file_store,
        vector_stores: vs_registry,
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: args.chinese_thinking,
        codex_auto_inject: args.codex_auto_inject,
        codex_persistent_inject: args.codex_persistent_inject,
        port: args.port,
        rate_limiter,
        metrics: Arc::new(metrics::Metrics::new()),
        tool_policy: Arc::new(tokio::sync::RwLock::new(handlers::ToolPolicy {
            allowed_mcp_servers: parse_csv_list(&args.allowed_mcp_servers),
            allowed_computer_displays: parse_csv_list(&args.allowed_computer_displays),
        })),
        executors: Arc::new(tokio::sync::RwLock::new(executors)),
        token_tracker: Arc::new(deecodex::token_anomaly::TokenTracker::new(
            32,
            args.token_anomaly_prompt_max,
            args.token_anomaly_spike_ratio,
            args.token_anomaly_burn_window,
            args.token_anomaly_burn_rate,
        )),
        data_dir: Arc::new(args.data_dir.clone()),
        codex_launch_with_cdp: args.codex_launch_with_cdp,
        cdp_port: args.cdp_port,
        account_store: Arc::new(tokio::sync::RwLock::new(account_store)),
        active_account: Arc::new(tokio::sync::RwLock::new(active_account)),
        reasoning_effort_override: Arc::new(tokio::sync::RwLock::new(
            active_endpoint.reasoning_effort_override.clone(),
        )),
        thinking_tokens: Arc::new(tokio::sync::RwLock::new(active_endpoint.thinking_tokens)),
        custom_headers: Arc::new(tokio::sync::RwLock::new(
            active_endpoint.custom_headers.clone(),
        )),
        request_timeout_secs: Arc::new(tokio::sync::RwLock::new(
            active_endpoint.request_timeout_secs,
        )),
        request_history: {
            let db_path = args.data_dir.join("request_history.db");
            Arc::new(
                deecodex::request_history::RequestHistoryStore::new(&db_path).unwrap_or_else(|e| {
                    tracing::warn!("请求历史数据库初始化失败，使用内存存储: {e}");
                    deecodex::request_history::RequestHistoryStore::new(std::path::Path::new(
                        ":memory:",
                    ))
                    .unwrap()
                }),
            )
        },
        codex_router_sessions: Arc::new(dashmap::DashMap::new()),
    })
}

async fn running_app_state(manager: &ServerManager) -> Option<handlers::AppState> {
    manager.app_state.lock().await.clone()
}

async fn sync_account_store_to_running_state(manager: &ServerManager, store: &AccountStore) {
    if let Some(app_state) = running_app_state(manager).await {
        *app_state.account_store.write().await = store.clone();
    }
}

async fn sync_account_mutation_to_running_state(
    manager: &ServerManager,
    store: &AccountStore,
    account: &deecodex::accounts::Account,
) {
    if let Some(app_state) = running_app_state(manager).await {
        *app_state.account_store.write().await = store.clone();
        if app_state.active_account.read().await.id == account.id {
            *app_state.active_account.write().await = account.clone();
        }
    }
}

async fn sync_active_account_to_running_state(
    manager: &ServerManager,
    store: &AccountStore,
    target: &deecodex::accounts::Account,
) -> Result<(), String> {
    let Some(app_state) = running_app_state(manager).await else {
        return Ok(());
    };

    let mut target = target.clone();
    target.normalize_v2();
    let target_endpoint = target
        .active_endpoint(store.active_endpoint_id.as_deref())
        .cloned()
        .or_else(|| target.endpoints.first().cloned())
        .ok_or_else(|| "目标账号没有可用端点".to_string())?;
    target.sync_legacy_from_endpoint(&target_endpoint);

    let upstream_url = deecodex::handlers::validate_upstream(&target_endpoint.base_url)
        .map_err(|e| format!("目标账号上游 URL 无效: {e}"))?;
    let vision_upstream = if target_endpoint.vision.base_url.is_empty() {
        None
    } else {
        Some(
            deecodex::handlers::validate_upstream(&target_endpoint.vision.base_url)
                .map_err(|e| format!("视觉上游 URL 无效: {e}"))?,
        )
    };

    *app_state.upstream.write().await = upstream_url;
    *app_state.api_key.write().await = target.api_key.clone();
    *app_state.model_map.write().await = target_endpoint.model_map.clone();
    *app_state.vision_upstream.write().await = vision_upstream;
    *app_state.vision_api_key.write().await = target_endpoint.vision.api_key.clone();
    *app_state.vision_model.write().await = target_endpoint.vision.model.clone();
    *app_state.vision_endpoint.write().await = target_endpoint.vision.path.clone();

    *app_state.reasoning_effort_override.write().await =
        target_endpoint.reasoning_effort_override.clone();
    *app_state.thinking_tokens.write().await = target_endpoint.thinking_tokens;
    *app_state.custom_headers.write().await = target_endpoint.custom_headers.clone();
    *app_state.request_timeout_secs.write().await = target_endpoint.request_timeout_secs;

    *app_state.active_account.write().await = target.clone();
    *app_state.account_store.write().await = store.clone();

    let host = manager.host.lock().await.clone();
    let port = *manager.port.lock().await;
    let data_dir = manager.data_dir.lock().await.clone();
    deecodex::codex_config::inject_with_host_and_data_dir(
        &host,
        port,
        target.context_window_override,
        Some(&data_dir),
    );

    tracing::info!("已同步运行中账号: {} ({})", target.name, target.provider);
    Ok(())
}

// ── 内部函数（托盘和 Tauri 命令共用） ─────────────────────────────────────

pub async fn start_service_inner(manager: &ServerManager) -> Result<ServiceInfo, String> {
    if manager.is_running().await {
        let info = get_status_internal(manager).await;
        return Err(format!("服务已在运行中 (端口: {})", info.port));
    }

    let args = load_args();
    let host = args.host.clone();
    let port = args.port;

    let state = build_app_state(&args).map_err(|e| format!("构建服务状态失败: {e}"))?;

    // 将 AppState 存储到 ServerManager，供 switch_account 等命令使用
    *manager.app_state.lock().await = Some(state.clone());
    // 请求历史数据库独立保存，服务停止后仍可读取
    *manager.request_history.lock().await = Some(state.request_history.clone());

    let app = handlers::build_router(state.clone()).layer(axum::extract::DefaultBodyLimit::max(
        args.max_body_mb * 1024 * 1024,
    ));

    let addr = deecodex::config::format_host_port(&host, port);
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| format!("无法绑定服务地址 {addr}: {e}"))?;

    if args.codex_auto_inject && !args.codex_persistent_inject {
        deecodex::codex_config::fix();
        deecodex::codex_config::inject_with_host_and_data_dir(
            &host,
            port,
            load_active_account_context_window(&args.data_dir),
            Some(&args.data_dir),
        );
    }

    let (tx, mut rx) = tokio::sync::watch::channel(());
    let server = axum::serve(listener, app);

    let handle = tokio::spawn(async move {
        server
            .with_graceful_shutdown(async move {
                rx.changed().await.ok();
            })
            .await
            .ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    *manager.shutdown_tx.lock().await = Some(tx);
    *manager.handle.lock().await = Some(handle);
    *manager.host.lock().await = host.clone();
    *manager.port.lock().await = port;
    *manager.start_time.lock().await = Some(std::time::Instant::now());

    // CDP 注入：自动启动 Codex 桌面版并注入 JS
    if args.codex_launch_with_cdp {
        let cdp_port = args.cdp_port;
        tokio::spawn(async move {
            #[cfg(target_os = "macos")]
            let result = tokio::process::Command::new("open")
                .arg("-a")
                .arg("Codex.app")
                .arg("--args")
                .arg(format!("--remote-debugging-port={cdp_port}"))
                .spawn();
            #[cfg(target_os = "windows")]
            let result = tokio::process::Command::new("Codex.exe")
                .arg(format!("--remote-debugging-port={cdp_port}"))
                .spawn();
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let result: std::io::Result<tokio::process::Child> = Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "CDP 启动不支持当前平台",
            ));
            match result {
                Ok(_) => tracing::info!("已启动 Codex 桌面版 (CDP 端口 {cdp_port})"),
                Err(e) => tracing::warn!("启动 Codex 桌面版失败: {e}"),
            }
        });
    }
    let inject_state = Arc::new(state.clone());
    let cdp_port = args.cdp_port;
    tokio::spawn(async move {
        deecodex::inject::try_inject_with_port(inject_state, cdp_port).await;
    });

    // 写入 PID 文件，供诊断检测服务运行状态
    let pid = std::process::id();
    let _ = std::fs::write(args.data_dir.join("deecodex.pid"), pid.to_string());

    manager.update_tray().await;
    tracing::info!(
        "服务已启动 → http://{}:{port}",
        deecodex::config::client_url_host(&host)
    );

    Ok(get_status_internal(manager).await)
}

pub async fn stop_service_inner(manager: &ServerManager) -> Result<ServiceInfo, String> {
    if !manager.is_running().await {
        return Err("服务未在运行".to_string());
    }

    if let Some(tx) = manager.shutdown_tx.lock().await.take() {
        let _ = tx.send(());
    }

    if let Some(handle) = manager.handle.lock().await.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(35), handle).await;
    }

    let args = load_args();
    // 线程已聚合并依赖 deecodex 时才保留注入，否则安全清理。
    let needs_deecodex_injection = {
        let bp = args.data_dir.join("thread_migration_backup.json");
        if bp.exists() {
            std::fs::read_to_string(&bp)
                .ok()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
                .and_then(|v| v.get("target_provider")?.as_str().map(|s| s == "deecodex"))
                .unwrap_or(true) // 解析失败则保守保留
        } else {
            false
        }
    };
    if args.codex_auto_inject && !args.codex_persistent_inject && !needs_deecodex_injection {
        deecodex::codex_config::remove();
    }

    // 清理 PID 文件
    let _ = std::fs::remove_file(args.data_dir.join("deecodex.pid"));

    *manager.start_time.lock().await = None;
    *manager.app_state.lock().await = None;
    manager.update_tray().await;
    tracing::info!("服务已停止");

    Ok(get_status_internal(manager).await)
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn start_service(manager: State<'_, ServerManager>) -> Result<ServiceInfo, String> {
    start_service_inner(&manager).await
}

#[tauri::command]
pub async fn stop_service(manager: State<'_, ServerManager>) -> Result<ServiceInfo, String> {
    stop_service_inner(&manager).await
}

#[tauri::command]
pub async fn get_service_status(manager: State<'_, ServerManager>) -> Result<ServiceInfo, String> {
    Ok(get_status_internal(&manager).await)
}

async fn get_status_internal(manager: &ServerManager) -> ServiceInfo {
    let running = manager.is_running().await;
    let uptime = if running {
        manager
            .start_time
            .lock()
            .await
            .map(|t| t.elapsed().as_secs())
    } else {
        None
    };
    let args = load_args();
    let host = if running {
        manager.host.lock().await.clone()
    } else {
        args.host.clone()
    };
    let port = if running {
        *manager.port.lock().await
    } else {
        args.port
    };
    ServiceInfo {
        running,
        host,
        port,
        uptime_secs: uptime,
        version: env!("CARGO_PKG_VERSION").to_string(),
        cdp_port: args.cdp_port,
        codex_launch_with_cdp: args.codex_launch_with_cdp,
    }
}

#[tauri::command]
pub fn launch_codex_cdp(manager: State<'_, ServerManager>) -> Result<(), String> {
    let args = load_args();
    let cdp_port = args.cdp_port;
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-a")
        .arg("Codex.app")
        .arg("--args")
        .arg(format!("--remote-debugging-port={cdp_port}"))
        .spawn()
        .map_err(|e| format!("启动 Codex 失败: {e}"))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("Codex.exe")
        .arg(format!("--remote-debugging-port={cdp_port}"))
        .spawn()
        .map_err(|e| format!("启动 Codex 失败: {e}"))?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("CDP 启动不支持当前平台".to_string());

    // 启动 Codex 后异步触发 JS 注入
    let app_state =
        tauri::async_runtime::block_on(async { manager.app_state.lock().await.clone() });
    if let Some(state) = app_state {
        tauri::async_runtime::spawn(async move {
            deecodex::inject::try_inject_with_port(std::sync::Arc::new(state), cdp_port).await;
        });
    }

    Ok(())
}

#[tauri::command]
pub fn stop_codex_cdp() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("osascript")
        .arg("-e")
        .arg("quit app \"Codex\"")
        .spawn()
        .map_err(|e| format!("停止 Codex 失败: {e}"))?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .arg("/c")
        .arg("taskkill")
        .arg("/f")
        .arg("/im")
        .arg("Codex.exe")
        .spawn()
        .map_err(|e| format!("停止 Codex 失败: {e}"))?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("CDP 停止不支持当前平台".to_string());
    Ok(())
}

#[tauri::command]
pub fn get_config() -> Result<GuiConfig, String> {
    let mut args = load_args();

    // 用活跃账号的字段覆盖 config.json 的对应字段，保证配置面板显示的是实际运行值
    let store = deecodex::accounts::load_accounts(&args.data_dir);
    if let Some(active_id) = &store.active_id {
        if let Some(active) = store.accounts.iter().find(|a| &a.id == active_id) {
            if !active.upstream.is_empty() {
                args.upstream = active.upstream.clone();
            }
            if !active.api_key.is_empty() {
                args.api_key = active.api_key.clone();
            }
            if !active.model_map.is_empty() {
                args.model_map = serde_json::to_string(&active.model_map).unwrap_or_default();
            }
            if !active.vision_upstream.is_empty() {
                args.vision_upstream = active.vision_upstream.clone();
            }
            if !active.vision_api_key.is_empty() {
                args.vision_api_key = active.vision_api_key.clone();
            }
            if !active.vision_model.is_empty() {
                args.vision_model = active.vision_model.clone();
            }
            if !active.vision_endpoint.is_empty() {
                args.vision_endpoint = active.vision_endpoint.clone();
            }
        }
    }

    Ok(GuiConfig::from(args))
}

fn save_config_inner(
    config: GuiConfig,
    injection_endpoint: Option<(String, u16)>,
) -> Result<(), String> {
    let host = deecodex::config::normalize_host(&config.host);
    let data_dir = normalize_data_dir(&config.data_dir);
    let config_path = Args::default_config_path(&data_dir);
    let existing = Args::load_from_file(&config_path);
    let account_config = account_backed_config(existing.as_ref());

    // GUI 始终写入当前数据目录，避免 Preview 或自定义目录污染正式版配置。
    sync_data_dir_env_file(&data_dir, "DEECODEX_HOST", &host);
    sync_data_dir_env_file(&data_dir, "DEECODEX_PORT", &config.port.to_string());
    sync_data_dir_env_file(&data_dir, "DEECODEX_UPSTREAM", &account_config.upstream);
    sync_data_dir_env_file(&data_dir, "DEECODEX_API_KEY", &account_config.api_key);
    sync_data_dir_env_file(&data_dir, "DEECODEX_MODEL_MAP", &account_config.model_map);

    let args = Args {
        command: None,
        config: None,
        port: config.port,
        host,
        upstream: account_config.upstream,
        api_key: account_config.api_key,
        model_map: account_config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: account_config.vision_upstream,
        vision_api_key: account_config.vision_api_key,
        vision_model: account_config.vision_model,
        vision_endpoint: account_config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    let config_path = Args::default_config_path(&args.data_dir);
    args.save_to_file(&config_path)
        .map_err(|e| format!("保存配置失败: {e}"))?;

    // 根据更新后的 Codex 注入开关立即应用/移除 Codex config.toml 修改
    let (inject_host, inject_port) =
        injection_endpoint.unwrap_or_else(|| (args.host.clone(), args.port));
    if args.codex_auto_inject || args.codex_persistent_inject {
        deecodex::codex_config::fix();
        let cw = load_active_account_context_window(&args.data_dir);
        deecodex::codex_config::inject_with_host_and_data_dir(
            &inject_host,
            inject_port,
            cw,
            Some(&args.data_dir),
        );
    } else {
        deecodex::codex_config::remove();
    }

    tracing::info!("配置已保存 → {}", config_path.display());
    Ok(())
}

#[tauri::command]
pub async fn save_config(
    manager: State<'_, ServerManager>,
    config: GuiConfig,
) -> Result<(), String> {
    let injection_endpoint = if manager.is_running().await {
        let host = manager.host.lock().await.clone();
        let port = *manager.port.lock().await;
        Some((host, port))
    } else {
        None
    };
    save_config_inner(config, injection_endpoint)
}

pub fn save_config_without_runtime(config: GuiConfig) -> Result<(), String> {
    save_config_inner(config, None)
}

#[tauri::command]
pub fn validate_config(config: GuiConfig) -> Vec<Value> {
    let host = deecodex::config::normalize_host(&config.host);
    let data_dir = normalize_data_dir(&config.data_dir);
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        host,
        upstream: config.upstream,
        api_key: config.api_key,
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key: config.vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    deecodex::validate::validate(&args)
        .into_iter()
        .map(|d| {
            json!({
                "severity": match d.severity {
                    deecodex::validate::Severity::Error => "error",
                    deecodex::validate::Severity::Warn => "warn",
                },
                "category": d.category,
                "message": d.message,
            })
        })
        .collect()
}

/// 运行完整诊断（同步，含 14 项检查；连通性检测标记为 Info 待后续异步补全）
#[tauri::command]
pub fn run_diagnostics(config: GuiConfig) -> serde_json::Value {
    let host = deecodex::config::normalize_host(&config.host);
    let data_dir = normalize_data_dir(&config.data_dir);
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        host,
        upstream: config.upstream,
        api_key: config.api_key,
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key: config.vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    let ctx = deecodex::validate::DiagnosticContext::from(&args);
    let report = deecodex::validate::run_diagnostics_sync(&ctx);
    serde_json::to_value(report).unwrap_or_default()
}

/// 运行完整诊断（异步，包含上游 API 连通性检测）
#[tauri::command]
pub async fn run_full_diagnostics(config: GuiConfig) -> Result<serde_json::Value, String> {
    let host = deecodex::config::normalize_host(&config.host);
    let data_dir = normalize_data_dir(&config.data_dir);
    let args = Args {
        command: None,
        config: None,
        port: config.port,
        host,
        upstream: config.upstream.clone(),
        api_key: config.api_key.clone(),
        model_map: config.model_map,
        max_body_mb: config.max_body_mb as usize,
        vision_upstream: config.vision_upstream,
        vision_api_key: config.vision_api_key,
        vision_model: config.vision_model,
        vision_endpoint: config.vision_endpoint,
        chinese_thinking: config.chinese_thinking,
        codex_auto_inject: config.codex_auto_inject,
        codex_persistent_inject: config.codex_persistent_inject,
        prompts_dir: config.prompts_dir.into(),
        data_dir,
        token_anomaly_prompt_max: config.token_anomaly_prompt_max,
        token_anomaly_spike_ratio: config.token_anomaly_spike_ratio,
        token_anomaly_burn_window: config.token_anomaly_burn_window,
        token_anomaly_burn_rate: config.token_anomaly_burn_rate,
        allowed_mcp_servers: config.allowed_mcp_servers,
        allowed_computer_displays: config.allowed_computer_displays,
        computer_executor: config.computer_executor,
        computer_executor_timeout_secs: config.computer_executor_timeout_secs,
        mcp_executor_config: config.mcp_executor_config,
        mcp_executor_timeout_secs: config.mcp_executor_timeout_secs,
        playwright_state_dir: config.playwright_state_dir,
        browser_use_bridge_url: config.browser_use_bridge_url,
        browser_use_bridge_command: config.browser_use_bridge_command,
        daemon: false,
        codex_launch_with_cdp: config.codex_launch_with_cdp,
        cdp_port: config.cdp_port,
    };

    let ctx = deecodex::validate::DiagnosticContext::from(&args);
    let mut report = deecodex::validate::run_diagnostics_sync(&ctx);

    // 异步检测上游连通性
    let connectivity = do_test_connectivity(&config.upstream, &config.api_key).await;
    let conn_item = match connectivity {
        Ok(result) => deecodex::validate::connectivity_check_result(
            result.ok,
            result.status_code,
            result.latency_ms,
            result.model_count,
            &result.endpoint,
            result.error.as_deref(),
        ),
        Err(e) => deecodex::validate::connectivity_check_result(
            false,
            0,
            0,
            None,
            &config.upstream,
            Some(&e),
        ),
    };

    // 替换「账号连通」分组中的连通性检查项
    for group in &mut report.groups {
        if group.name == "账号连通" {
            if let Some(item) = group
                .items
                .iter_mut()
                .find(|i| i.check_name == "账号连通性")
            {
                *item = conn_item;
            }
            group.health = deecodex::validate::DiagnosticReport::compute_group_health(&group.items);
            break;
        }
    }

    // 重新计算摘要
    report.summary = deecodex::validate::DiagnosticReport::compute_summary(&report.groups);

    Ok(serde_json::to_value(report).unwrap_or_default())
}

#[tauri::command]
pub async fn check_upgrade() -> Result<Value, String> {
    upgrade::check_upgrade_impl().await
}

#[tauri::command]
pub fn run_upgrade() -> Result<String, String> {
    upgrade::run_upgrade_impl()
}

// ── 账号管理 Tauri 命令 ────────────────────────────────────────────────────

/// 获取账号列表，Key 字段脱敏后返回
#[tauri::command]
pub async fn list_accounts(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);

    let accounts: Vec<Value> = store
        .accounts
        .iter()
        .map(|account| account_to_value_for_store(account, &store))
        .collect();
    let client_counts = client_account_counts(&store);
    let router_now = deecodex::accounts::now_secs();
    let router_status = handlers::dex_router_status_snapshot(&store, "gpt-5.5", router_now);
    let router_status_scenarios =
        handlers::dex_router_status_scenarios(&store, "gpt-5.5", router_now);

    Ok(json!({
        "accounts": accounts,
        "active_id": store.active_id,
        "active_account_id": store.active_account_id,
        "active_endpoint_id": store.active_endpoint_id,
        "active_by_surface": store.active_by_surface,
        "client_counts": client_counts,
        "router_status": router_status,
        "router_status_scenarios": router_status_scenarios,
    }))
}

/// 获取当前活跃账号
#[tauri::command]
pub async fn get_active_account(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);

    let active = store.active_account();

    match active {
        Some(a) => Ok(account_to_value_for_store(a, &store)),
        None => Err("没有活跃账号".to_string()),
    }
}

#[tauri::command]
pub async fn get_dex_assistant_account(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .active_account_for_dex_assistant()
        .ok_or_else(|| "没有 DEX 助手活跃账号".to_string())?;
    let endpoint = dex_assistant_endpoint_for_account(&store, account);
    Ok(account_to_value_with_endpoint(account, endpoint))
}

#[tauri::command]
pub async fn set_dex_assistant_account(
    manager: State<'_, ServerManager>,
    account_id: String,
    endpoint_id: Option<String>,
) -> Result<Value, String> {
    set_dex_assistant_account_inner(&manager, account_id, endpoint_id).await
}

/// 创建新账号（支持传入完整 account_json，用于前端先编辑后保存的流程）
#[tauri::command]
pub async fn add_account(
    manager: State<'_, ServerManager>,
    provider: String,
    account_json: Option<String>,
    client_kind: Option<String>,
    client_surface: Option<String>,
) -> Result<Value, String> {
    use deecodex::accounts::{
        generate_id, get_provider_presets, guess_provider, now_secs, Account,
    };

    let data_dir = manager.data_dir.lock().await.clone();
    let (host, port) = service_endpoint_for_manager(&manager).await;
    let explicit_client_kind = client_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_account_client_kind);
    let default_client_kind = explicit_client_kind.clone().unwrap_or_default();

    let mut new_account = if let Some(json) = account_json {
        let mut a: Account = parse_account_json(&json)?;
        a.id = generate_id();
        apply_explicit_account_client(
            &mut a,
            explicit_client_kind.as_ref(),
            client_surface.as_deref(),
        );
        if a.provider.is_empty() {
            a.provider = guess_provider(&a.upstream).to_string();
        }
        if a.provider_options.is_empty() {
            a.provider_options = deecodex::providers::provider_options_for_slug(&a.provider);
        }
        a.created_at = now_secs();
        a.updated_at = now_secs();
        a
    } else {
        let presets = get_provider_presets();
        let preset = presets
            .iter()
            .find(|p| p.slug == provider)
            .ok_or_else(|| format!("未知供应商: {provider}"))?;

        Account {
            id: generate_id(),
            name: format!("{} 账号", preset.label),
            provider: provider.clone(),
            client_kind: default_client_kind.clone(),
            client_surface: parse_account_client_surface(
                client_surface.as_deref().unwrap_or("cli"),
                &default_client_kind,
            ),
            wire_protocol: preset.wire_protocol.clone(),
            upstream: preset.default_upstream.clone(),
            api_key: String::new(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: Default::default(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: now_secs(),
            updated_at: now_secs(),
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: deecodex::providers::provider_options_for_slug(&provider),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    };
    new_account.normalize_v2();
    if !new_account.client_kind.is_codex() {
        new_account.translate_enabled = false;
        new_account.endpoints.clear();
        ensure_client_proxy_options(&mut new_account, &host, port);
    }

    let (store, (new_account, became_active)) =
        mutate_account_store(&data_dir, "保存账号失败", |store| {
            let mut candidate_store = store.clone();
            candidate_store.accounts.push(new_account.clone());
            deecodex::accounts::validate_capability_links(&candidate_store)
                .map_err(|e| e.to_string())?;
            deecodex::accounts::validate_dev_pipeline_links(&candidate_store)
                .map_err(|e| e.to_string())?;

            // 如果没有活跃账号，自动设为活跃
            let became_active = store.active_id.is_none() && new_account.client_kind.is_codex();
            if became_active {
                if let Some(endpoint) = new_account.endpoints.first() {
                    validate_endpoint_runtime_urls(endpoint)?;
                }
                let sync_legacy_global = should_sync_legacy_global(store, &new_account);
                activate_codex_surface_account(
                    store,
                    &new_account,
                    new_account.endpoints.first().map(|e| e.id.clone()),
                    sync_legacy_global,
                );
            }

            store.accounts.push(new_account.clone());
            Ok((new_account.clone(), became_active))
        })?;

    if became_active
        && (new_account.client_surface == AccountClientSurface::Cli
            || store.active_account_id.as_deref() == Some(&new_account.id))
    {
        sync_active_account_to_running_state(&manager, &store, &new_account).await?;
    } else {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    Ok(account_to_value(&new_account))
}

/// 服务概览轻量接入：创建账号并立即准备对应客户端配置。
#[tauri::command]
pub async fn dex_quick_configure_client(
    manager: State<'_, ServerManager>,
    kind: String,
    surface: Option<String>,
    account_json: String,
) -> Result<Value, String> {
    use deecodex::accounts::{generate_id, guess_provider, now_secs, Account};

    let data_dir = manager.data_dir.lock().await.clone();
    let (host, port) = service_endpoint_for_manager(&manager).await;
    let mut account: Account = parse_account_json(&account_json)?;
    let client_kind = parse_account_client_kind(&kind);
    account.id = generate_id();
    account.client_kind = client_kind.clone();
    account.client_surface =
        parse_account_client_surface(surface.as_deref().unwrap_or("cli"), &client_kind);
    if account.provider.trim().is_empty() {
        account.provider = guess_provider(&account.upstream).to_string();
    }
    if account.provider_options.is_empty() {
        account.provider_options =
            deecodex::providers::provider_options_for_slug(&account.provider);
    }
    let now = now_secs();
    account.created_at = now;
    account.updated_at = now;
    account.normalize_v2();

    let mut report = None;
    let mut client_apply_ok = false;
    if account.client_kind.is_codex() {
        account.translate_enabled = true;
        if account.endpoints.is_empty() {
            account.normalize_v2();
        }
    } else {
        account.translate_enabled = false;
        account.endpoints.clear();
        ensure_client_proxy_options(&mut account, &host, port);
        let apply_report = deecodex::client_integrations::apply(&mut account, false)
            .map_err(|e| format!("写入客户端配置失败: {e}"))?;
        client_apply_ok = apply_report.ok;
        report = Some(serde_json::to_value(&apply_report).unwrap_or_default());
        append_account_event(
            &data_dir,
            &account.id,
            &account.client_kind,
            "client_account_apply",
            apply_report.ok,
            &apply_report.message,
            serde_json::to_value(&apply_report).unwrap_or_default(),
        );
    }

    let (store, (account, became_active)) =
        mutate_account_store(&data_dir, "保存账号失败", |store| {
            let mut candidate_store = store.clone();
            candidate_store.accounts.push(account.clone());
            deecodex::accounts::validate_capability_links(&candidate_store)
                .map_err(|e| e.to_string())?;
            deecodex::accounts::validate_dev_pipeline_links(&candidate_store)
                .map_err(|e| e.to_string())?;

            let became_active = account.client_kind.is_codex();
            if became_active {
                if let Some(endpoint) = account.endpoints.first() {
                    validate_endpoint_runtime_urls(endpoint)?;
                }
                let endpoint_id = account
                    .endpoints
                    .first()
                    .map(|endpoint| endpoint.id.clone());
                let sync_legacy_global = should_sync_legacy_global(store, &account);
                activate_codex_surface_account(store, &account, endpoint_id, sync_legacy_global);
            } else if client_apply_ok {
                activate_client_surface_account(store, &account, None, false);
            }
            store.accounts.push(account.clone());
            Ok((account.clone(), became_active))
        })?;

    if became_active
        && (account.client_surface == AccountClientSurface::Cli
            || store.active_account_id.as_deref() == Some(&account.id))
    {
        sync_active_account_to_running_state(&manager, &store, &account).await?;
    } else {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    Ok(json!({
        "ok": true,
        "account": account_to_value_for_store(&account, &store),
        "report": report,
    }))
}

/// 更新账号信息（从前端接收完整 JSON）
#[tauri::command]
pub async fn update_account(
    manager: State<'_, ServerManager>,
    account_json: String,
) -> Result<Value, String> {
    use deecodex::accounts::{guess_provider, now_secs, Account};

    let data_dir = manager.data_dir.lock().await.clone();
    let (host, port) = service_endpoint_for_manager(&manager).await;

    let updated: Account = parse_account_json(&account_json)?;
    let (store, (account, endpoint_for_legacy, is_active)) =
        mutate_account_store(&data_dir, "保存账号失败", |store| {
            let pos = store
                .accounts
                .iter()
                .position(|a| a.id == updated.id)
                .ok_or_else(|| format!("账号不存在: {}", updated.id))?;

            let mut account = updated.clone();
            restore_redacted_account_secrets(&mut account, &store.accounts[pos]);
            // 仅当 provider 为空时自动检测，避免覆盖用户选择
            if account.provider.is_empty() {
                account.provider = guess_provider(&account.upstream).to_string();
            }
            if account.provider_options.is_empty() {
                account.provider_options =
                    deecodex::providers::provider_options_for_slug(&account.provider);
            }
            if !account.client_kind.is_codex() {
                account.translate_enabled = false;
                account.endpoints.clear();
                ensure_client_proxy_options(&mut account, &host, port);
            }
            account.normalize_v2();
            let surface_active = account.client_kind.is_codex()
                && store
                    .active_selection_for_surface(
                        &AccountClientKind::Codex,
                        &account.client_surface,
                    )
                    .and_then(|selection| selection.account_id.as_deref())
                    == Some(account.id.as_str());
            let surface_endpoint_id = store
                .active_endpoint_id_for_surface(&AccountClientKind::Codex, &account.client_surface)
                .map(str::to_string);
            let endpoint_for_legacy = if surface_active {
                account
                    .active_endpoint(surface_endpoint_id.as_deref())
                    .cloned()
                    .or_else(|| account.endpoints.first().cloned())
            } else if store.active_account_id.as_ref() == Some(&account.id)
                || store.active_id.as_ref() == Some(&account.id)
            {
                account
                    .active_endpoint(store.active_endpoint_id.as_deref())
                    .cloned()
                    .or_else(|| account.endpoints.first().cloned())
            } else {
                account.endpoints.first().cloned()
            };
            if let Some(endpoint) = endpoint_for_legacy.as_ref() {
                account.sync_legacy_from_endpoint(endpoint);
            }
            account.updated_at = now_secs();

            let is_active = account.client_kind.is_codex()
                && (surface_active
                    || store.active_account_id.as_ref() == Some(&account.id)
                    || store.active_id.as_ref() == Some(&account.id));
            if is_active {
                if let Some(endpoint) = endpoint_for_legacy.as_ref() {
                    validate_endpoint_runtime_urls(endpoint)?;
                }
            }
            store.accounts[pos] = account.clone();
            deecodex::accounts::validate_capability_links(store).map_err(|e| e.to_string())?;
            deecodex::accounts::validate_dev_pipeline_links(store).map_err(|e| e.to_string())?;
            Ok((account, endpoint_for_legacy, is_active))
        })?;

    // 如果保存的是活跃账号，立即热更新运行中的服务状态。
    if is_active
        && (account.client_surface == AccountClientSurface::Cli
            || store.active_account_id.as_deref() == Some(&account.id))
    {
        sync_active_account_to_running_state(&manager, &store, &account).await?;
    } else {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    let selected_endpoint = endpoint_for_legacy.as_ref();
    Ok(account_to_value_with_endpoint(&account, selected_endpoint))
}

/// 删除账号（拒绝删除最后一个）
#[tauri::command]
pub async fn delete_account(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, next_active_account) =
        mutate_account_store(&data_dir, "保存账号失败", |store| {
            if store.accounts.len() <= 1 {
                return Err("不能删除最后一个账号".to_string());
            }
            let deleting = store
                .accounts
                .iter()
                .find(|account| account.id == id)
                .cloned()
                .ok_or_else(|| format!("账号不存在: {id}"))?;
            if deleting.client_kind.is_codex()
                && store
                    .accounts
                    .iter()
                    .filter(|account| account.client_kind.is_codex())
                    .count()
                    <= 1
            {
                return Err(
                    "不能删除最后一个 Codex 代理账号，否则 DEX 助手和代理服务没有可用活跃账号"
                        .into(),
                );
            }

            let was_global_active = store.active_id.as_deref() == Some(&id)
                || store.active_account_id.as_deref() == Some(&id);
            let was_surface_active = store
                .active_selection_for_surface(&deleting.client_kind, &deleting.client_surface)
                .and_then(|selection| selection.account_id.as_deref())
                == Some(id.as_str());

            store.accounts.retain(|a| a.id != id);
            for account in &mut store.accounts {
                if account.capability_account_id.as_deref() == Some(&id) {
                    account.capability_enabled = false;
                    account.capability_account_id = None;
                }
            }

            if was_surface_active {
                if let Some(next_surface_account) = store
                    .accounts
                    .iter()
                    .find(|account| {
                        account.client_kind == deleting.client_kind
                            && account.client_surface == deleting.client_surface
                    })
                    .cloned()
                {
                    activate_client_surface_account(
                        store,
                        &next_surface_account,
                        next_surface_account
                            .endpoints
                            .first()
                            .map(|endpoint| endpoint.id.clone()),
                        false,
                    );
                }
            }

            let next_active_id = if was_global_active {
                store
                    .accounts
                    .iter()
                    .find(|account| {
                        account.client_kind.is_codex()
                            && account.client_surface == AccountClientSurface::Cli
                    })
                    .or_else(|| {
                        store
                            .accounts
                            .iter()
                            .find(|account| account.client_kind.is_codex())
                    })
                    .map(|account| account.id.clone())
            } else {
                None
            };

            // 如果删除的是活跃账号，只切到剩余的 Codex 代理账号；外部客户端不参与代理热切换。
            if was_global_active {
                store.active_id = next_active_id.clone();
                store.active_account_id = store.active_id.clone();
                store.active_endpoint_id = next_active_id.as_ref().and_then(|next_id| {
                    store
                        .accounts
                        .iter()
                        .find(|account| &account.id == next_id)
                        .and_then(|account| account.endpoints.first())
                        .map(|endpoint| endpoint.id.clone())
                });
            }

            deecodex::accounts::validate_capability_links(store).map_err(|e| e.to_string())?;
            deecodex::accounts::validate_dev_pipeline_links(store).map_err(|e| e.to_string())?;
            let next_active_account = next_active_id.and_then(|next_id| {
                store
                    .accounts
                    .iter()
                    .find(|account| account.id == next_id)
                    .cloned()
            });
            Ok(next_active_account)
        })?;

    if let Some(next_active_account) = next_active_account {
        sync_active_account_to_running_state(&manager, &store, &next_active_account).await?;
    } else {
        sync_account_store_to_running_state(&manager, &store).await;
    }

    Ok(json!({"success": true}))
}

/// 切换活跃账号，同步更新运行中服务的上游/Key/模型映射等热字段
#[tauri::command]
pub(crate) async fn switch_account_inner(
    manager: &ServerManager,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, (target, target_endpoint)) =
        mutate_account_store(&data_dir, "保存账号失败", |store| {
            let mut target = store
                .accounts
                .iter()
                .find(|a| a.id == id)
                .ok_or_else(|| format!("账号不存在: {id}"))?
                .clone();
            if !target.client_kind.is_codex() {
                return Err("非 Codex 客户端账号不参与 deecodex 代理切换，请使用写入配置".into());
            }
            target.normalize_v2();
            let surface_endpoint_id = store
                .active_endpoint_id_for_surface(&AccountClientKind::Codex, &target.client_surface)
                .map(str::to_string);
            let target_endpoint = target
                .active_endpoint(surface_endpoint_id.as_deref())
                .cloned()
                .or_else(|| target.endpoints.first().cloned())
                .ok_or_else(|| "目标账号没有可用端点".to_string())?;
            validate_endpoint_runtime_urls(&target_endpoint)?;
            target.sync_legacy_from_endpoint(&target_endpoint);

            deecodex::accounts::validate_capability_links(store).map_err(|e| e.to_string())?;
            deecodex::accounts::validate_dev_pipeline_links(store).map_err(|e| e.to_string())?;

            let sync_legacy_global = should_sync_legacy_global(store, &target);
            activate_codex_surface_account(
                store,
                &target,
                Some(target_endpoint.id.clone()),
                sync_legacy_global,
            );
            Ok((target, target_endpoint))
        })?;

    if target.client_surface == AccountClientSurface::Cli
        || store.active_account_id.as_deref() == Some(&target.id)
    {
        sync_active_account_to_running_state(manager, &store, &target).await?;
    } else {
        sync_account_store_to_running_state(manager, &store).await;
    }

    Ok(account_to_value_with_endpoint(
        &target,
        Some(&target_endpoint),
    ))
}

#[tauri::command]
pub async fn switch_account(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    switch_account_inner(&manager, id).await
}

/// 清除账号当前冷却/配额等待，但保留成功失败统计和最近请求桶。
#[tauri::command]
pub async fn clear_account_cooldown(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, account) = mutate_account_store(&data_dir, "保存账号失败", |store| {
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .ok_or_else(|| format!("账号不存在: {id}"))?;
        let now = deecodex::accounts::now_secs();
        account.clear_runtime_cooldown(now);
        account.updated_at = now;
        Ok(account.clone())
    })?;
    sync_account_mutation_to_running_state(&manager, &store, &account).await;

    Ok(account_to_value_for_store(&account, &store))
}

/// 重置账号运行态，清空配额、冷却、模型状态和最近成功/失败统计。
#[tauri::command]
pub async fn reset_account_runtime_state(
    manager: State<'_, ServerManager>,
    id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, account) = mutate_account_store(&data_dir, "保存账号失败", |store| {
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .ok_or_else(|| format!("账号不存在: {id}"))?;
        account.reset_runtime_state();
        account.updated_at = deecodex::accounts::now_secs();
        Ok(account.clone())
    })?;
    sync_account_mutation_to_running_state(&manager, &store, &account).await;

    Ok(account_to_value_for_store(&account, &store))
}

/// 更新官方账号池路由参数。默认用于 Codex 官方账号池，也可预留给后续分池。
#[tauri::command]
pub async fn set_account_routing(
    manager: State<'_, ServerManager>,
    id: String,
    anchor_enabled: Option<bool>,
    execution_enabled: Option<bool>,
    pool: Option<String>,
    priority: Option<i64>,
    weight: Option<u32>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, account) = mutate_account_store(&data_dir, "保存账号失败", |store| {
        let account = store
            .accounts
            .iter_mut()
            .find(|account| account.id == id)
            .ok_or_else(|| format!("账号不存在: {id}"))?;
        let mut routing = deecodex::accounts::account_routing_options(account);
        let role_changed = anchor_enabled.is_some() || execution_enabled.is_some();
        if let Some(anchor_enabled) = anchor_enabled {
            routing.anchor_enabled = Some(anchor_enabled);
        }
        if let Some(execution_enabled) = execution_enabled {
            routing.execution_enabled = Some(execution_enabled);
        }
        if role_changed {
            routing.enabled = routing.anchor_enabled_for_account(account)
                || routing.execution_enabled_for_account(account);
            routing.disabled = !routing.enabled;
        }
        if let Some(pool) = pool {
            let pool = pool.trim();
            if !pool.is_empty() {
                routing.pool = pool.to_string();
            }
        }
        if let Some(priority) = priority {
            routing.priority = priority;
        }
        if let Some(weight) = weight {
            routing.weight = weight.clamp(1, 100);
        }
        let routing = routing.normalized();
        deecodex::accounts::set_account_routing_options(account, routing);
        account.updated_at = deecodex::accounts::now_secs();
        Ok(account.clone())
    })?;
    sync_account_mutation_to_running_state(&manager, &store, &account).await;

    Ok(account_to_value_for_store(&account, &store))
}

/// 导入 CLIProxyAPI / Codex OAuth 认证 JSON，并转成 deecodex 账号池账号。
#[tauri::command]
pub async fn import_auth_json_accounts(
    manager: State<'_, ServerManager>,
    auth_files_json: String,
    client_surface: Option<String>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let files = parse_auth_json_import_files(&auth_files_json)?;
    if files.is_empty() {
        return Err("没有选择认证 JSON 文件".into());
    }

    let now = deecodex::accounts::now_secs();
    let target_surface = client_surface
        .as_deref()
        .map(|value| parse_account_client_surface(value, &AccountClientKind::Codex))
        .unwrap_or_default();

    let (store, (imported_count, skipped, failed, activated, imported_ids, imported_events)) =
        mutate_account_store(&data_dir, "保存导入账号失败", |store| {
            let mut imported_events = Vec::new();
            let mut failed = Vec::new();
            let mut skipped = 0usize;
            let mut first_imported_id: Option<String> = None;
            let mut imported_ids = Vec::<String>::new();

            for file in files {
                let name = file.name.trim();
                if !name.is_empty() && !name.to_ascii_lowercase().ends_with(".json") {
                    failed.push(json!({
                        "name": name,
                        "error": "文件必须是 .json",
                    }));
                    continue;
                }
                let value: Value = match serde_json::from_str(&file.content) {
                    Ok(value) => value,
                    Err(err) => {
                        failed.push(json!({
                            "name": name,
                            "error": format!("JSON 无效: {err}"),
                        }));
                        continue;
                    }
                };
                let token = match codex_oauth_token_from_auth_json(&value, now) {
                    Ok(token) => token,
                    Err(err) => {
                        failed.push(json!({
                            "name": name,
                            "error": err,
                        }));
                        continue;
                    }
                };
                if store
                    .accounts
                    .iter()
                    .any(|account| same_imported_codex_oauth(account, &token, &target_surface))
                {
                    skipped += 1;
                    continue;
                }

                let account =
                    codex_account_from_imported_token(token, name, target_surface.clone(), now);
                if first_imported_id.is_none() {
                    first_imported_id = Some(account.id.clone());
                }
                imported_ids.push(account.id.clone());
                imported_events.push((
                    account.id.clone(),
                    account.client_kind.clone(),
                    json!({ "source_file": name }),
                ));
                store.accounts.push(account);
            }

            let imported_count = imported_events.len();
            let mut activated = false;
            if imported_count > 0 && !surface_has_active_codex_official(store, &target_surface) {
                if let Some(account_id) = first_imported_id.clone() {
                    let account = store
                        .accounts
                        .iter()
                        .find(|account| account.id == account_id)
                        .cloned();
                    if let Some(account) = account {
                        let endpoint_id = account
                            .endpoints
                            .first()
                            .map(|endpoint| endpoint.id.clone());
                        let sync_legacy_global = should_sync_legacy_global(store, &account);
                        activate_codex_surface_account(
                            store,
                            &account,
                            endpoint_id,
                            sync_legacy_global,
                        );
                        activated = true;
                    }
                }
            }
            Ok((
                imported_count,
                skipped,
                failed,
                activated,
                imported_ids,
                imported_events,
            ))
        })?;

    let mut imported = Vec::new();
    if imported_count > 0 {
        sync_account_store_to_running_state(&manager, &store).await;
        if activated {
            if let Some(account) = store.active_account_for_surface(&target_surface).cloned() {
                if account.client_surface == AccountClientSurface::Cli
                    || store.active_account_id.as_deref() == Some(&account.id)
                {
                    sync_active_account_to_running_state(&manager, &store, &account).await?;
                }
            }
        }
        for (account_id, client_kind, details) in imported_events {
            append_account_event(
                &data_dir,
                &account_id,
                &client_kind,
                "auth_json_import",
                true,
                "已从认证 JSON 导入 Codex 官方账号",
                details,
            );
        }
        imported = store
            .accounts
            .iter()
            .filter(|account| imported_ids.iter().any(|id| id == &account.id))
            .map(|account| account_to_value_for_store(account, &store))
            .collect();
    }

    if imported_count == 0 && !failed.is_empty() {
        let first_error = failed
            .first()
            .and_then(|item| item.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("导入失败");
        return Err(format!("认证 JSON 导入失败: {first_error}"));
    }

    let message = if imported_count > 0 {
        let active_text = if activated {
            "，已设为活跃官方账号"
        } else {
            ""
        };
        format!(
            "已导入 {imported_count} 个 Codex 官方账号到账号池，跳过 {skipped} 个已存在账号{active_text}"
        )
    } else {
        format!("未导入新账号，跳过 {skipped} 个已存在账号")
    };

    Ok(json!({
        "imported": imported_count,
        "skipped": skipped,
        "failed": failed,
        "activated": activated,
        "accounts": imported,
        "message": message,
    }))
}

/// 从 Codex 的 config.toml 导入账号
#[tauri::command]
pub async fn import_codex_config(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();

    let mut imported = deecodex::codex_config::extract_account_from_codex_config()
        .ok_or_else(|| "Codex config.toml 中未找到可导入的第三方 provider 配置".to_string())?;
    imported.normalize_v2();

    let (_store, imported) = mutate_account_store(&data_dir, "保存账号失败", |store| {
        // 检查是否已存在相同 upstream + key 的账号
        let is_duplicate = store
            .accounts
            .iter()
            .any(|a| a.upstream == imported.upstream && a.api_key == imported.api_key);

        if is_duplicate {
            return Err("已存在相同上游和 Key 的账号，跳过导入".to_string());
        }

        // 如果没有活跃账号，自动设为活跃
        if store.active_id.is_none() {
            activate_codex_surface_account(
                store,
                &imported,
                imported
                    .endpoints
                    .first()
                    .map(|endpoint| endpoint.id.clone()),
                true,
            );
        }

        store.accounts.push(imported.clone());
        Ok(imported.clone())
    })?;

    Ok(account_to_value(&imported))
}

/// 返回供应商预设列表
#[tauri::command]
pub fn get_provider_presets() -> Result<Value, String> {
    let presets = deecodex::accounts::get_provider_presets();
    let list: Vec<Value> = presets
        .iter()
        .map(|p| {
            json!({
                "slug": p.slug,
                "label": p.label,
                "description": p.description,
                "default_upstream": p.default_upstream,
                "known_models": p.known_models,
                "default_api_key_env": p.default_api_key_env,
                "wire_protocol": p.wire_protocol,
                "auth_scheme": p.auth_scheme,
                "model_discovery": p.model_discovery,
                "capabilities": p.capabilities,
                "capability_labels": p.capability_labels,
                "provider_options": p.provider_options,
            })
        })
        .collect();
    Ok(json!(list))
}

#[tauri::command]
pub fn get_client_profiles() -> Result<Value, String> {
    serde_json::to_value(deecodex::client_integrations::get_client_profiles())
        .map_err(|e| format!("序列化客户端分类失败: {e}"))
}

#[tauri::command]
pub async fn get_client_status(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    serde_json::to_value(deecodex::client_integrations::status(account))
        .map_err(|e| format!("序列化客户端状态失败: {e}"))
}

#[tauri::command]
pub async fn refresh_client_status(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, (report, client_kind)) =
        mutate_account_store(&data_dir, "保存账号状态失败", |store| {
            let pos = store
                .accounts
                .iter()
                .position(|a| a.id == account_id)
                .ok_or_else(|| "账号不存在".to_string())?;
            let report = deecodex::client_integrations::status(&store.accounts[pos]);
            store.accounts[pos].last_check = Some(deecodex::accounts::ClientCheckRecord {
                ok: report.ok,
                checked_at: deecodex::accounts::now_secs(),
                message: report.message.clone(),
                details: serde_json::to_value(&report).unwrap_or_default(),
            });
            Ok((report, store.accounts[pos].client_kind.clone()))
        })?;
    sync_account_store_to_running_state(&manager, &store).await;
    if !client_kind.is_codex() {
        append_account_event(
            &data_dir,
            &account_id,
            &client_kind,
            "client_account_status",
            report.ok,
            &report.message,
            serde_json::to_value(&report).unwrap_or_default(),
        );
    }
    serde_json::to_value(report).map_err(|e| format!("序列化客户端状态失败: {e}"))
}

#[tauri::command]
pub async fn list_client_backups(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    serde_json::to_value(deecodex::client_integrations::list_backups(account))
        .map_err(|e| format!("序列化客户端备份失败: {e}"))
}

#[tauri::command]
pub async fn restore_client_backup(
    manager: State<'_, ServerManager>,
    account_id: String,
    backup_path: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, (report, client_kind)) =
        mutate_account_store(&data_dir, "保存账号恢复状态失败", |store| {
            let pos = store
                .accounts
                .iter()
                .position(|a| a.id == account_id)
                .ok_or_else(|| "账号不存在".to_string())?;
            let report = deecodex::client_integrations::restore_backup_for_account(
                &store.accounts[pos],
                Path::new(&backup_path),
            )
            .map_err(|e| format!("恢复客户端备份失败: {e}"))?;
            store.accounts[pos].last_check = Some(deecodex::accounts::ClientCheckRecord {
                ok: report.ok,
                checked_at: deecodex::accounts::now_secs(),
                message: report.message.clone(),
                details: serde_json::to_value(&report).unwrap_or_default(),
            });
            Ok((report, store.accounts[pos].client_kind.clone()))
        })?;
    sync_account_store_to_running_state(&manager, &store).await;
    append_account_event(
        &data_dir,
        &account_id,
        &client_kind,
        "client_account_restore",
        report.ok,
        &report.message,
        serde_json::to_value(&report).unwrap_or_default(),
    );
    serde_json::to_value(report).map_err(|e| format!("序列化恢复结果失败: {e}"))
}

#[tauri::command]
pub async fn open_client_config(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    ensure_editable_account_config_file(&target.path, account)
        .map_err(|e| format!("准备客户端配置文件失败: {e}"))?;
    open_path_with_system_editor(&target.path)
        .map_err(|e| format!("打开客户端配置文件失败: {e}"))?;
    append_account_event(
        &data_dir,
        &account_id,
        &account.client_kind,
        "client_config_open",
        true,
        "已打开客户端配置文件",
        json!({"config_path": target.path.to_string_lossy(), "format": target.format}),
    );
    Ok(json!({"ok": true, "path": target.path.to_string_lossy(), "format": target.format}))
}

#[tauri::command]
pub async fn get_account_config_file(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    let exists = target.path.exists();
    let content = if exists {
        read_text_file_lossy(&target.path).map_err(|e| format!("读取配置文件失败: {e}"))?
    } else {
        initial_account_config_text(account)
    };
    let validation = validate_config_text_for_editor(target.format, &content);
    let size_bytes = if exists {
        std::fs::metadata(&target.path)
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };
    Ok(json!({
        "ok": true,
        "account_id": account_id,
        "client_kind": account.client_kind,
        "label": target.label,
        "path": target.path.to_string_lossy(),
        "format": target.format,
        "exists": exists,
        "content": content,
        "size_bytes": size_bytes,
        "validation": validation,
    }))
}

#[tauri::command]
pub async fn validate_account_config_file(
    manager: State<'_, ServerManager>,
    account_id: String,
    content: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    Ok(validate_config_text_for_editor(target.format, &content))
}

#[tauri::command]
pub async fn save_account_config_file(
    manager: State<'_, ServerManager>,
    account_id: String,
    content: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    let target = account_config_target(account).map_err(|e| format!("定位配置文件失败: {e}"))?;
    let validation = validate_config_text_for_editor(target.format, &content);
    if validation["ok"].as_bool() != Some(true) {
        return Ok(json!({
            "ok": false,
            "message": "配置文件校验未通过，未写入磁盘",
            "path": target.path.to_string_lossy(),
            "format": target.format,
            "validation": validation,
        }));
    }
    if let Some(parent) = target.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let backup_path = backup_config_file_for_editor(&target.path)
        .map_err(|e| format!("备份配置文件失败: {e}"))?;
    std::fs::write(&target.path, content).map_err(|e| format!("写入配置文件失败: {e}"))?;
    append_account_event(
        &data_dir,
        &account_id,
        &account.client_kind,
        "client_config_save",
        true,
        "配置文件已在客户端编辑并保存",
        json!({
            "config_path": target.path.to_string_lossy(),
            "backup_path": backup_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "format": target.format,
        }),
    );
    Ok(json!({
        "ok": true,
        "message": "配置文件已保存",
        "path": target.path.to_string_lossy(),
        "format": target.format,
        "backup_path": backup_path.map(|p| p.to_string_lossy().to_string()),
        "validation": validation,
    }))
}

#[tauri::command]
pub async fn get_claude_desktop_developer_mode() -> Result<Value, String> {
    read_claude_desktop_developer_mode()
}

#[tauri::command]
pub async fn set_claude_desktop_developer_mode(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    enabled: bool,
) -> Result<Value, String> {
    let result = write_claude_desktop_developer_mode(enabled)?;
    if let Some(account_id) = account_id.filter(|id| !id.trim().is_empty()) {
        let data_dir = manager.data_dir.lock().await.clone();
        let store = deecodex::accounts::load_accounts(&data_dir);
        if let Some(account) = store.accounts.iter().find(|a| a.id == account_id) {
            append_account_event(
                &data_dir,
                &account_id,
                &account.client_kind,
                "claude_desktop_developer_mode",
                true,
                if enabled {
                    "Claude 桌面版开发者模式已开启"
                } else {
                    "Claude 桌面版开发者模式已关闭"
                },
                result.clone(),
            );
        }
    }
    Ok(result)
}

struct ConfigEditorTarget {
    path: PathBuf,
    format: &'static str,
    label: &'static str,
}

fn account_config_target(
    account: &deecodex::accounts::Account,
) -> Result<ConfigEditorTarget, String> {
    if account.client_kind.is_codex() {
        let path = deecodex::config::home_dir()
            .ok_or_else(|| "无法定位用户 HOME 目录".to_string())?
            .join(".codex")
            .join("config.toml");
        return Ok(ConfigEditorTarget {
            path,
            format: "toml",
            label: "Codex config.toml",
        });
    }
    let report = deecodex::client_integrations::status(account);
    let path = report
        .config_path
        .as_deref()
        .ok_or_else(|| "客户端配置路径不可用".to_string())?;
    let (format, label) = match account.client_kind {
        AccountClientKind::ClaudeCode => {
            if account.client_surface == AccountClientSurface::Desktop {
                ("json", "Claude 桌面版 configLibrary")
            } else {
                ("json", "Claude Code settings.json")
            }
        }
        AccountClientKind::Openclaw => ("json", "OpenClaw 配置"),
        AccountClientKind::Hermes => ("yaml", "Hermes config.yaml"),
        AccountClientKind::GenericClient => ("env", "通用客户端 env"),
        AccountClientKind::Codex => unreachable!(),
    };
    Ok(ConfigEditorTarget {
        path: PathBuf::from(path),
        format,
        label,
    })
}

fn ensure_editable_account_config_file(
    path: &Path,
    account: &deecodex::accounts::Account,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        let initial = initial_account_config_text(account);
        std::fs::write(path, initial)?;
    }
    Ok(())
}

fn initial_account_config_text(account: &deecodex::accounts::Account) -> String {
    if account.client_kind.is_codex() {
        "# Codex config.toml\n".into()
    } else {
        initial_client_config_text(account)
    }
}

fn initial_client_config_text(account: &deecodex::accounts::Account) -> String {
    match account.client_kind {
        deecodex::accounts::AccountClientKind::ClaudeCode => {
            let model_map = client_model_map_for_editor(account);
            let auth_env = client_auth_env_for_editor(account);
            let mut env = serde_json::Map::new();
            env.insert(auth_env, Value::String(String::new()));
            env.insert(
                "ANTHROPIC_BASE_URL".into(),
                Value::String(account.upstream.clone()),
            );
            if let Some(model) = client_model_for_editor(account, &model_map, "default") {
                env.insert("ANTHROPIC_MODEL".into(), Value::String(model));
            }
            for (slot, key) in [
                ("sonnet", "ANTHROPIC_DEFAULT_SONNET_MODEL"),
                ("opus", "ANTHROPIC_DEFAULT_OPUS_MODEL"),
                ("haiku", "ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            ] {
                if let Some(model) = client_model_for_editor(account, &model_map, slot) {
                    env.insert(key.into(), Value::String(model));
                }
            }
            serde_json::to_string_pretty(&json!({ "env": env })).unwrap_or_else(|_| "{}".into())
                + "\n"
        }
        deecodex::accounts::AccountClientKind::Openclaw => {
            let model_map = client_model_map_for_editor(account);
            let default_model = client_model_for_editor(account, &model_map, "default")
                .unwrap_or_else(|| account.default_model.clone());
            let env_name = client_auth_env_for_editor(account);
            let mut defaults = serde_json::Map::new();
            if !default_model.trim().is_empty() {
                defaults.insert(
                    "model".into(),
                    Value::String(format!("deecodex/{}", default_model.trim())),
                );
            }
            for (slot, key) in [
                ("image", "imageModel"),
                ("image_generation", "imageGenerationModel"),
                ("video_generation", "videoGenerationModel"),
            ] {
                if let Some(model) = client_model_for_editor(account, &model_map, slot) {
                    defaults.insert(key.into(), Value::String(format!("deecodex/{model}")));
                }
            }
            let models: Vec<Value> = client_model_values_for_editor(account, &model_map)
                .into_iter()
                .map(|model| json!({ "id": model, "name": model }))
                .collect();
            serde_json::to_string_pretty(&json!({
                "models": {
                    "providers": {
                        "deecodex": {
                            "baseUrl": account.upstream,
                            "apiKey": { "provider": "default", "source": "env", "id": env_name },
                            "auth": "api-key",
                            "models": models
                        }
                    }
                },
                "agents": { "defaults": defaults }
            }))
            .unwrap_or_else(|_| "{}".into())
                + "\n"
        }
        deecodex::accounts::AccountClientKind::Hermes => {
            let model_map = client_model_map_for_editor(account);
            let default_model = client_model_for_editor(account, &model_map, "default")
                .unwrap_or_else(|| account.default_model.clone());
            let env_name = client_auth_env_for_editor(account);
            let mut out = String::new();
            out.push_str("model:\n");
            out.push_str(&format!("  default: {}\n", yaml_scalar(&default_model)));
            out.push_str(&format!("  provider: {}\n", yaml_scalar(&account.provider)));
            out.push_str(&format!("  base_url: {}\n", yaml_scalar(&account.upstream)));
            out.push_str(&format!("  api_key_env: {}\n", yaml_scalar(&env_name)));
            let mut aux_lines = Vec::new();
            for (slot, path) in [
                ("vision", "vision"),
                ("web_extract", "web_extract"),
                ("compression", "compression"),
                ("session_search", "session_search"),
                ("title_generation", "title_generation"),
            ] {
                if let Some(model) = client_model_for_editor(account, &model_map, slot) {
                    aux_lines.push(format!("  {path}:\n    model: {}\n", yaml_scalar(&model)));
                }
            }
            if !aux_lines.is_empty() {
                out.push_str("auxiliary:\n");
                out.push_str(&aux_lines.join(""));
            }
            out
        }
        deecodex::accounts::AccountClientKind::GenericClient => {
            let model_map = client_model_map_for_editor(account);
            let env_name = client_auth_env_for_editor(account);
            let mut out = String::new();
            out.push_str(&format!("OPENAI_BASE_URL={}\n", account.upstream));
            out.push_str(&format!("{env_name}=\n"));
            if let Some(model) = client_model_for_editor(account, &model_map, "default") {
                out.push_str(&format!("OPENAI_MODEL={model}\n"));
            }
            for (slot, model) in model_map {
                if slot == "default" || model.trim().is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "{}={}\n",
                    generic_model_env_name_for_editor(&slot),
                    model
                ));
            }
            out
        }
        deecodex::accounts::AccountClientKind::Codex => String::new(),
    }
}

fn read_text_file_lossy(path: &Path) -> std::io::Result<String> {
    match std::fs::read_to_string(path) {
        Ok(content) => Ok(content),
        Err(_) => std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).to_string()),
    }
}

fn backup_config_file_for_editor(path: &Path) -> std::io::Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_file_name(format!(
        "{}.deecodex.bak.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        deecodex::accounts::now_secs()
    ));
    std::fs::copy(path, &backup)?;
    Ok(Some(backup))
}

fn read_claude_desktop_developer_mode() -> Result<Value, String> {
    let candidates = claude_desktop_developer_settings_paths()?;
    let mut entries = Vec::new();
    let mut enabled_count = 0usize;
    let mut existing_count = 0usize;
    for path in &candidates {
        let exists = path.exists();
        if exists {
            existing_count += 1;
        }
        let settings = read_json_object_for_editor(path)?;
        let enabled = settings
            .get("allowDevTools")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if enabled {
            enabled_count += 1;
        }
        entries.push(json!({
            "path": path.to_string_lossy(),
            "exists": exists,
            "enabled": enabled,
        }));
    }
    let runtime = claude_desktop_runtime_info();
    let restart_required = claude_desktop_restart_required(&candidates, &runtime);
    Ok(json!({
        "ok": true,
        "enabled": enabled_count > 0,
        "enabled_count": enabled_count,
        "existing_count": existing_count,
        "entries": entries,
        "runtime": runtime,
        "restart_required": restart_required,
    }))
}

fn write_claude_desktop_developer_mode(enabled: bool) -> Result<Value, String> {
    let candidates = claude_desktop_developer_settings_paths()?;
    let mut targets: Vec<PathBuf> = candidates
        .iter()
        .filter(|path| {
            path.exists() || path.parent().map(|parent| parent.exists()).unwrap_or(false)
        })
        .cloned()
        .collect();
    if targets.is_empty() {
        if let Some(primary) = candidates.first() {
            targets.push(primary.clone());
        }
    }

    let mut changed_files = Vec::new();
    let mut backup_paths = Vec::new();
    for path in targets {
        let mut settings = read_json_object_for_editor(&path)?;
        settings["allowDevTools"] = Value::Bool(enabled);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建 Claude 设置目录失败: {e}"))?;
        }
        if let Some(backup) = backup_config_file_for_editor(&path)
            .map_err(|e| format!("备份 Claude 设置失败: {e}"))?
        {
            backup_paths.push(backup.to_string_lossy().to_string());
        }
        let content = serde_json::to_string_pretty(&settings)
            .map_err(|e| format!("序列化 Claude 设置失败: {e}"))?
            + "\n";
        std::fs::write(&path, content).map_err(|e| format!("写入 Claude 设置失败: {e}"))?;
        changed_files.push(path.to_string_lossy().to_string());
    }
    let runtime = claude_desktop_runtime_info();
    let restart_required = claude_desktop_restart_required(&candidates, &runtime);

    Ok(json!({
        "ok": true,
        "enabled": enabled,
        "message": if enabled {
            "Claude 桌面版开发者模式已开启"
        } else {
            "Claude 桌面版开发者模式已关闭"
        },
        "changed_files": changed_files,
        "backup_paths": backup_paths,
        "runtime": runtime,
        "restart_required": restart_required,
    }))
}

fn claude_desktop_developer_settings_paths() -> Result<Vec<PathBuf>, String> {
    #[cfg(target_os = "macos")]
    {
        let home =
            deecodex::config::home_dir().ok_or_else(|| "无法定位用户 HOME 目录".to_string())?;
        let app_support = home.join("Library").join("Application Support");
        let mut paths = vec![
            app_support.join("Claude").join("developer_settings.json"),
            app_support
                .join("Claude-3p")
                .join("developer_settings.json"),
        ];
        for user_data_dir in claude_desktop_runtime_user_data_dirs() {
            push_unique_path(&mut paths, user_data_dir.join("developer_settings.json"));
        }
        Ok(paths)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("Claude 桌面版开发者模式当前仅支持 macOS 配置路径".into())
    }
}

#[cfg(target_os = "macos")]
fn claude_desktop_runtime_info() -> Value {
    let user_data_dirs = claude_desktop_runtime_user_data_dirs();
    let pids = claude_desktop_process_ids();
    let oldest_started_at = claude_desktop_oldest_started_at();
    json!({
        "running": !pids.is_empty(),
        "pids": pids,
        "oldest_started_at": oldest_started_at,
        "user_data_dirs": user_data_dirs
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect::<Vec<_>>(),
    })
}

#[cfg(not(target_os = "macos"))]
fn claude_desktop_runtime_info() -> Value {
    json!({
        "running": false,
        "pids": [],
        "oldest_started_at": null,
        "user_data_dirs": [],
    })
}

#[cfg(target_os = "macos")]
fn claude_desktop_runtime_user_data_dirs() -> Vec<PathBuf> {
    claude_desktop_process_lines()
        .into_iter()
        .filter_map(|line| extract_command_arg_value(&line, "--user-data-dir="))
        .map(PathBuf::from)
        .fold(Vec::new(), |mut acc, path| {
            push_unique_path(&mut acc, path);
            acc
        })
}

#[cfg(target_os = "macos")]
fn claude_desktop_process_ids() -> Vec<u32> {
    claude_desktop_process_lines()
        .into_iter()
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .collect()
}

#[cfg(target_os = "macos")]
fn claude_desktop_oldest_started_at() -> Option<u64> {
    let now = unix_timestamp_secs();
    claude_desktop_process_lines()
        .into_iter()
        .filter_map(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(parse_ps_elapsed_secs)
        })
        .map(|elapsed| now.saturating_sub(elapsed))
        .min()
}

#[cfg(target_os = "macos")]
fn claude_desktop_process_lines() -> Vec<String> {
    let output = match std::process::Command::new("ps")
        .args(["ax", "-o", "pid=,etime=,command="])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            tracing::warn!("探测 Claude Desktop 进程失败: {err}");
            return Vec::new();
        }
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.contains("/Claude.app/Contents/")
                || line.contains("/Claude Helper.app/Contents/")
                || line.ends_with("/Claude")
        })
        .map(str::to_string)
        .collect()
}

#[cfg(target_os = "macos")]
fn extract_command_arg_value(line: &str, prefix: &str) -> Option<String> {
    let start = line.find(prefix)? + prefix.len();
    let rest = &line[start..];
    let end = rest.find(" --").unwrap_or(rest.len());
    let value = rest[..end].trim().trim_matches('"').trim_matches('\'');
    (!value.is_empty()).then(|| value.to_string())
}

fn claude_desktop_restart_required(paths: &[PathBuf], runtime: &Value) -> bool {
    if runtime.get("running").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let Some(started_at) = runtime.get("oldest_started_at").and_then(Value::as_u64) else {
        return true;
    };
    paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .filter_map(|meta| meta.modified().ok())
        .filter_map(|mtime| {
            mtime
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_secs())
        })
        .max()
        .map(|latest_mtime| latest_mtime > started_at)
        .unwrap_or(false)
}

fn parse_ps_elapsed_secs(value: &str) -> Option<u64> {
    let (day_part, time_part) = value
        .split_once('-')
        .map_or((None, value), |(days, time)| (Some(days), time));
    let days = day_part
        .and_then(|days| days.parse::<u64>().ok())
        .unwrap_or(0);
    let parts: Vec<u64> = time_part
        .split(':')
        .filter_map(|part| part.parse::<u64>().ok())
        .collect();
    let seconds = match parts.as_slice() {
        [minutes, seconds] => minutes * 60 + seconds,
        [hours, minutes, seconds] => hours * 3600 + minutes * 60 + seconds,
        _ => return None,
    };
    Some(days * 86_400 + seconds)
}

fn unix_timestamp_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn read_json_object_for_editor(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let content = read_text_file_lossy(path).map_err(|e| format!("读取 JSON 配置失败: {e}"))?;
    if content.trim().is_empty() {
        return Ok(json!({}));
    }
    let value: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 JSON 配置失败: {e}"))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err("JSON 配置根节点必须是对象".into())
    }
}

fn validate_config_text_for_editor(format: &str, content: &str) -> Value {
    let mut diagnostics = Vec::new();
    let trimmed = content.trim();
    match format {
        "toml" => {
            if let Err(err) = content.parse::<toml_edit::DocumentMut>() {
                diagnostics
                    .push(json!({"level": "error", "message": format!("TOML 解析失败: {err}")}));
            }
        }
        "json" => {
            if trimmed.is_empty() {
                diagnostics.push(json!({"level": "error", "message": "JSON 配置不能为空"}));
            } else if let Err(err) = serde_json::from_str::<Value>(content) {
                diagnostics
                    .push(json!({"level": "error", "message": format!("JSON 解析失败: {err}")}));
            }
        }
        "yaml" => {
            if !trimmed.is_empty() {
                if let Err(err) = serde_yaml::from_str::<serde_yaml::Value>(content) {
                    diagnostics.push(
                        json!({"level": "error", "message": format!("YAML 解析失败: {err}")}),
                    );
                }
            }
        }
        "env" => {
            for (idx, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((key, _)) = line.split_once('=') else {
                    diagnostics.push(
                        json!({"level": "error", "message": format!("第 {} 行缺少 '='", idx + 1)}),
                    );
                    continue;
                };
                if !key
                    .chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
                {
                    diagnostics.push(json!({"level": "warning", "message": format!("第 {} 行环境变量名建议使用大写字母、数字和下划线", idx + 1)}));
                }
            }
        }
        _ => diagnostics
            .push(json!({"level": "warning", "message": format!("未知配置格式: {format}")})),
    }
    let ok = diagnostics
        .iter()
        .all(|item| item["level"].as_str() != Some("error"));
    if ok {
        diagnostics.push(json!({"level": "info", "message": "配置语法校验通过"}));
    }
    json!({
        "ok": ok,
        "format": format,
        "diagnostics": diagnostics,
    })
}

fn client_model_map_for_editor(account: &deecodex::accounts::Account) -> HashMap<String, String> {
    account
        .client_options
        .get("model_map")
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|model| !model.is_empty())
                        .map(|model| (key.clone(), model.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn client_model_for_editor(
    account: &deecodex::accounts::Account,
    model_map: &HashMap<String, String>,
    slot: &str,
) -> Option<String> {
    model_map
        .get(slot)
        .cloned()
        .filter(|model| !model.trim().is_empty())
        .or_else(|| {
            if slot == "default" && !account.default_model.trim().is_empty() {
                Some(account.default_model.clone())
            } else {
                None
            }
        })
}

fn client_model_values_for_editor(
    account: &deecodex::accounts::Account,
    model_map: &HashMap<String, String>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(default_model) = client_model_for_editor(account, model_map, "default") {
        out.push(default_model);
    }
    for model in model_map.values() {
        if !model.trim().is_empty() && !out.contains(model) {
            out.push(model.clone());
        }
    }
    out
}

fn client_auth_env_for_editor(account: &deecodex::accounts::Account) -> String {
    account
        .client_options
        .get("auth_env")
        .or_else(|| account.client_options.get("api_key_env"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            match account.provider.as_str() {
                "anthropic" => "ANTHROPIC_API_KEY",
                "openrouter" => "OPENROUTER_API_KEY",
                "minimax" => "MINIMAX_API_KEY",
                _ => "OPENAI_API_KEY",
            }
            .into()
        })
}

fn generic_model_env_name_for_editor(slot: &str) -> String {
    let normalized: String = slot
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("OPENAI_{normalized}_MODEL")
}

fn yaml_scalar(value: &str) -> String {
    if value.is_empty()
        || value.chars().any(|ch| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    ':' | '#'
                        | '\''
                        | '"'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | ','
                        | '&'
                        | '*'
                        | '!'
                        | '|'
                        | '>'
                        | '@'
                        | '`'
                )
        })
    {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        value.into()
    }
}

pub(super) fn open_path_with_system_editor(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path.to_string_lossy()])
            .spawn()?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
        Ok(())
    }
}

#[tauri::command]
pub async fn test_client_account(
    manager: State<'_, ServerManager>,
    account_json: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (host, port) = service_endpoint_for_manager(&manager).await;
    let mut persisted_account_id = None;
    let mut account = if let Some(raw) = account_json {
        parse_account_json(&raw)?
    } else {
        let id = account_id.ok_or_else(|| "缺少 account_id 或 account_json".to_string())?;
        persisted_account_id = Some(id.clone());
        let store = deecodex::accounts::load_accounts(&data_dir);
        store
            .accounts
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or_else(|| "账号不存在".to_string())?
    };
    ensure_client_proxy_options(&mut account, &host, port);
    let mut draft = account.clone();
    let report = deecodex::client_integrations::apply(&mut draft, true)
        .map_err(|e| format!("客户端 dry-run 失败: {e}"))?;
    if let Some(id) = persisted_account_id {
        let (store, updated) =
            mutate_account_store(&data_dir, "保存账号预检状态失败", |store| {
                let mut updated = false;
                if let Some(existing) = store.accounts.iter_mut().find(|item| item.id == id) {
                    ensure_client_proxy_options(existing, &host, port);
                    existing.last_check = Some(deecodex::accounts::ClientCheckRecord {
                        ok: report.ok,
                        checked_at: deecodex::accounts::now_secs(),
                        message: report.message.clone(),
                        details: serde_json::to_value(&report).unwrap_or_default(),
                    });
                    updated = true;
                }
                Ok(updated)
            })?;
        if updated {
            sync_account_store_to_running_state(&manager, &store).await;
        }
    }
    if !account.id.trim().is_empty() && !account.client_kind.is_codex() {
        append_account_event(
            &data_dir,
            &account.id,
            &account.client_kind,
            "client_account_dry_run",
            report.ok,
            &report.message,
            serde_json::to_value(&report).unwrap_or_default(),
        );
    }
    serde_json::to_value(report).map_err(|e| format!("序列化 dry-run 结果失败: {e}"))
}

#[tauri::command]
pub async fn apply_client_account(
    manager: State<'_, ServerManager>,
    account_id: String,
    dry_run: Option<bool>,
) -> Result<Value, String> {
    let dry_run = dry_run.unwrap_or(false);
    let data_dir = manager.data_dir.lock().await.clone();
    let (host, port) = service_endpoint_for_manager(&manager).await;
    let (store, (report, client_kind)) =
        mutate_account_store(&data_dir, "保存账号状态失败", |store| {
            let pos = store
                .accounts
                .iter()
                .position(|a| a.id == account_id)
                .ok_or_else(|| "账号不存在".to_string())?;
            let mut account = store.accounts[pos].clone();
            if account.client_kind.is_codex() {
                return Err("Codex 账号请使用「应用」切换代理账号".into());
            }
            ensure_client_proxy_options(&mut account, &host, port);
            let report = deecodex::client_integrations::apply(&mut account, dry_run)
                .map_err(|e| format!("写入客户端配置失败: {e}"))?;
            let now = deecodex::accounts::now_secs();
            account.last_check = Some(deecodex::accounts::ClientCheckRecord {
                ok: report.ok,
                checked_at: now,
                message: report.message.clone(),
                details: serde_json::to_value(&report).unwrap_or_default(),
            });
            let client_kind = account.client_kind.clone();
            if !dry_run && report.ok {
                activate_client_surface_account(store, &account, None, false);
            }
            store.accounts[pos] = account;
            Ok((report, client_kind))
        })?;
    sync_account_store_to_running_state(&manager, &store).await;
    append_account_event(
        &data_dir,
        &account_id,
        &client_kind,
        if dry_run {
            "client_account_dry_run"
        } else {
            "client_account_apply"
        },
        report.ok,
        &report.message,
        serde_json::to_value(&report).unwrap_or_default(),
    );
    serde_json::to_value(report).map_err(|e| format!("序列化写入结果失败: {e}"))
}

#[tauri::command]
pub async fn get_account_events(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    Ok(json!(read_account_events(
        &data_dir,
        account_id.as_deref(),
        limit.unwrap_or(20)
    )))
}

#[tauri::command]
pub async fn import_client_accounts(manager: State<'_, ServerManager>) -> Result<Value, String> {
    use deecodex::accounts::{
        generate_id, now_secs, Account, DevPipelineToolMode, DevPipelineTriggerMode,
    };

    let data_dir = manager.data_dir.lock().await.clone();
    let candidates = deecodex::client_integrations::discover_client_accounts();
    let (store, (imported_accounts, import_events, skipped)) =
        mutate_account_store(&data_dir, "保存导入账号失败", |store| {
            let mut imported_accounts = Vec::new();
            let mut import_events = Vec::new();
            let mut skipped = 0usize;
            for candidate in candidates {
                if store
                    .accounts
                    .iter()
                    .any(|account| same_client_account(account, &candidate))
                {
                    skipped += 1;
                    continue;
                }
                let now = now_secs();
                let mut account = Account {
                    id: generate_id(),
                    name: candidate.name.clone(),
                    provider: candidate.provider.clone(),
                    client_kind: candidate.client_kind.clone(),
                    client_surface: candidate.client_surface.clone(),
                    wire_protocol: Default::default(),
                    upstream: candidate.upstream.clone(),
                    api_key: candidate.api_key.clone(),
                    auth_mode: Default::default(),
                    default_model: candidate.default_model.clone(),
                    client_options: candidate.client_options.clone(),
                    runtime_state: Default::default(),
                    last_applied_at: None,
                    last_check: None,
                    model_map: HashMap::new(),
                    vision_upstream: String::new(),
                    vision_api_key: String::new(),
                    vision_model: String::new(),
                    vision_endpoint: String::new(),
                    vision_enabled: false,
                    from_codex_config: false,
                    balance_url: String::new(),
                    created_at: now,
                    updated_at: now,
                    context_window_override: None,
                    reasoning_effort_override: None,
                    thinking_tokens: None,
                    custom_headers: HashMap::new(),
                    provider_options: deecodex::providers::provider_options_for_slug(
                        &candidate.provider,
                    ),
                    request_timeout_secs: None,
                    max_retries: None,
                    translate_enabled: false,
                    capability_enabled: false,
                    capability_account_id: None,
                    dev_pipeline_enabled: false,
                    dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
                    dev_pipeline_command: "/dev-pipeline".into(),
                    dev_pipeline_architect_account_id: None,
                    dev_pipeline_implementer_account_id: None,
                    dev_pipeline_reviewer_account_id: None,
                    dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
                    dev_pipeline_max_iterations: 3,
                    dev_pipeline_show_trace: false,
                    dev_pipeline_architect_instruction: String::new(),
                    dev_pipeline_implementer_instruction: String::new(),
                    dev_pipeline_reviewer_instruction: String::new(),
                    endpoints: Vec::new(),
                };
                account.normalize_v2();
                import_events.push((
                    account.id.clone(),
                    account.client_kind.clone(),
                    json!({
                        "source_path": candidate.source_path,
                        "client_surface": candidate.client_surface,
                        "warnings": candidate.warnings,
                    }),
                ));
                imported_accounts.push(account_to_value(&account));
                store.accounts.push(account);
            }
            Ok((imported_accounts, import_events, skipped))
        })?;

    if !imported_accounts.is_empty() {
        sync_account_store_to_running_state(&manager, &store).await;
        for (account_id, client_kind, details) in import_events {
            append_account_event(
                &data_dir,
                &account_id,
                &client_kind,
                "client_account_import",
                true,
                "已从本机客户端配置导入账号",
                details,
            );
        }
    }

    let statuses: Vec<Value> = store
        .accounts
        .iter()
        .filter(|account| !account.client_kind.is_codex())
        .map(|account| {
            serde_json::to_value(deecodex::client_integrations::status(account)).unwrap_or_default()
        })
        .collect();
    let imported_count = imported_accounts.len();
    let message = if imported_count == 0 {
        format!("客户端扫描完成，未发现新的可导入账号（已存在 {skipped} 个）")
    } else {
        format!("已导入 {imported_count} 个客户端账号，跳过 {skipped} 个已存在账号")
    };
    Ok(json!({
        "imported": imported_count,
        "skipped": skipped,
        "accounts": imported_accounts,
        "message": message,
        "statuses": statuses,
    }))
}

fn same_client_account(
    account: &deecodex::accounts::Account,
    candidate: &deecodex::client_integrations::ClientImportCandidate,
) -> bool {
    if account.client_kind != candidate.client_kind {
        return false;
    }
    if account.client_surface != candidate.client_surface {
        return false;
    }
    let existing_path = account
        .client_options
        .get("config_path")
        .and_then(Value::as_str);
    if let (Some(existing), Some(source)) = (existing_path, candidate.source_path.as_deref()) {
        if existing == source {
            return true;
        }
    }
    account.provider == candidate.provider
        && account.upstream == candidate.upstream
        && account.default_model == candidate.default_model
}

#[tauri::command]
pub fn get_endpoint_templates() -> Result<Value, String> {
    serde_json::to_value(deecodex::accounts::get_endpoint_templates())
        .map_err(|e| format!("序列化端点模板失败: {e}"))
}

#[tauri::command]
pub async fn switch_endpoint(
    manager: State<'_, ServerManager>,
    account_id: String,
    endpoint_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (store, (account, endpoint)) =
        mutate_account_store(&data_dir, "保存端点切换失败", |store| {
            let mut account = store
                .accounts
                .iter()
                .find(|a| a.id == account_id)
                .cloned()
                .ok_or_else(|| format!("账号不存在: {account_id}"))?;
            if !account.client_kind.is_codex() {
                return Err("非 Codex 客户端账号没有 deecodex 代理端点，请使用写入配置".into());
            }
            account.normalize_v2();
            let endpoint = account
                .endpoints
                .iter()
                .find(|e| e.id == endpoint_id)
                .cloned()
                .ok_or_else(|| format!("端点不存在: {endpoint_id}"))?;
            validate_endpoint_runtime_urls(&endpoint)?;

            let sync_legacy_global = should_sync_legacy_global(store, &account);
            activate_codex_surface_account(store, &account, Some(endpoint_id), sync_legacy_global);
            Ok((account, endpoint))
        })?;

    if account.client_surface == AccountClientSurface::Cli
        || store.active_account_id.as_deref() == Some(&account.id)
    {
        sync_active_account_to_running_state(&manager, &store, &account).await?;
    } else {
        sync_account_store_to_running_state(&manager, &store).await;
    }
    Ok(json!({
        "account": account_to_value_with_endpoint(&account, Some(&endpoint)),
        "endpoint": endpoint,
    }))
}

// ── 模型列表获取 ──────────────────────────────────────────────────────────

/// 从上游获取模型列表（传入 account_id 时自动查真实 Key）
#[tauri::command]
pub async fn fetch_upstream_models(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    upstream: Option<String>,
    api_key: Option<String>,
    endpoint_kind: Option<String>,
) -> Result<Vec<String>, String> {
    let mut cache_target: Option<(PathBuf, String, String)> = None;
    let (upstream, api_key, profile, endpoint_kind, oauth_account) = if let Some(id) = account_id {
        let data_dir = manager.data_dir.lock().await.clone();
        let store = deecodex::accounts::load_accounts(&data_dir);
        let account = store
            .accounts
            .iter()
            .find(|a| a.id == id)
            .ok_or_else(|| "账号不存在".to_string())?;
        let endpoint = endpoint_for_account_in_store(account, &store);
        let endpoint_id = endpoint
            .map(|endpoint| endpoint.id.clone())
            .or_else(|| {
                account
                    .endpoints
                    .first()
                    .map(|endpoint| endpoint.id.clone())
            })
            .unwrap_or_else(|| "default".into());
        cache_target = Some((data_dir, account.id.clone(), endpoint_id));
        (
            non_empty_override(upstream).unwrap_or_else(|| {
                endpoint
                    .map(|ep| ep.base_url.clone())
                    .unwrap_or_else(|| account.upstream.clone())
            }),
            secret_override(api_key).unwrap_or_else(|| account.api_key.clone()),
            deecodex::providers::profile_for_account(account),
            endpoint_kind.or_else(|| endpoint.map(|ep| format!("{:?}", ep.kind))),
            matches!(
                account.auth_mode,
                deecodex::accounts::AccountAuthMode::OAuth
            ),
        )
    } else {
        let upstream = upstream.ok_or("缺少 upstream 参数")?;
        let provider = if endpoint_kind
            .as_deref()
            .map(endpoint_kind_is_codex_official)
            .unwrap_or(false)
            || upstream_is_codex_official(&upstream)
        {
            "codex".to_string()
        } else {
            deecodex::providers::guess_provider(&upstream).to_string()
        };
        (
            upstream,
            api_key.unwrap_or_default(),
            deecodex::providers::profile_by_slug(&provider),
            endpoint_kind,
            false,
        )
    };

    if should_use_known_model_list(&profile, &upstream, endpoint_kind.as_deref(), oauth_account)
        && !profile.known_models.is_empty()
    {
        tracing::warn!(
            provider = %profile.slug,
            upstream = %upstream,
            "官方 OAuth 账号不探测真实 /models，按 CLIProxyAPI registry 模式使用内置模型列表"
        );
        persist_fetched_models(cache_target.as_ref(), &profile.known_models);
        refresh_codex_model_catalog_after_fetch(&manager, cache_target.as_ref()).await;
        return Ok(profile.known_models);
    }

    let urls = deecodex::providers::model_discovery_url(&profile, &upstream, &api_key)
        .map(|url| vec![url])
        .unwrap_or_else(|| vec![format!("{}/models", upstream.trim_end_matches('/'))]);

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("创建客户端失败: {e}"))?;
    for url in &urls {
        let req = model_probe_request(&client, url, &api_key, endpoint_kind.as_deref());
        tracing::info!(provider = %profile.slug, "获取上游模型: GET {url}");
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await.map_err(|e| format!("解析失败: {e}"))?;
                let models = deecodex::providers::parse_models_response(&profile, &body);
                if !models.is_empty() {
                    tracing::info!(provider = %profile.slug, "获取上游模型成功: {} 个模型", models.len());
                    persist_fetched_models(cache_target.as_ref(), &models);
                    refresh_codex_model_catalog_after_fetch(&manager, cache_target.as_ref()).await;
                    return Ok(models);
                }
                tracing::info!(provider = %profile.slug, "上游模型响应解析为空: {:?}", body);
            }
            Ok(resp) => {
                let status = resp.status();
                let snippet = resp.text().await.unwrap_or_default();
                tracing::info!(
                    "上游模型请求失败 HTTP {}: {}",
                    status.as_u16(),
                    snippet.chars().take(200).collect::<String>()
                );
            }
            Err(e) => {
                tracing::info!("上游模型请求错误: {url} → {e}");
            }
        }
    }
    if [
        "deepseek", "kimi", "minimax", "mimo", "longcat", "qwen", "glm",
    ]
    .contains(&profile.slug.as_str())
        && endpoint_kind
            .as_deref()
            .map(|kind| kind.to_ascii_lowercase().contains("anthropic"))
            .unwrap_or(false)
        && upstream.to_ascii_lowercase().contains("/anthropic")
        && !profile.known_models.is_empty()
    {
        tracing::warn!(
            provider = %profile.slug,
            upstream = %upstream,
            "Anthropic 兼容入口未返回可解析模型列表，使用内置模型模板"
        );
        persist_fetched_models(cache_target.as_ref(), &profile.known_models);
        refresh_codex_model_catalog_after_fetch(&manager, cache_target.as_ref()).await;
        return Ok(profile.known_models);
    }
    Err("无法从上游获取模型列表".to_string())
}

fn persist_fetched_models(target: Option<&(PathBuf, String, String)>, models: &[String]) {
    let Some((data_dir, account_id, endpoint_id)) = target else {
        return;
    };
    if let Err(err) =
        deecodex::codex_config::save_account_model_cache(data_dir, account_id, endpoint_id, models)
    {
        tracing::warn!(
            account_id = %account_id,
            endpoint_id = %endpoint_id,
            error = %err,
            "保存 Codex 账号模型缓存失败"
        );
    }
}

async fn refresh_codex_model_catalog_after_fetch(
    manager: &ServerManager,
    target: Option<&(PathBuf, String, String)>,
) {
    let Some((data_dir, _, _)) = target else {
        return;
    };
    let host = manager.host.lock().await.clone();
    let port = *manager.port.lock().await;
    let cw = load_active_account_context_window(data_dir);
    deecodex::codex_config::inject_with_host_and_data_dir(&host, port, cw, Some(data_dir));
}

fn endpoint_kind_is_codex_official(kind: &str) -> bool {
    let normalized = kind.to_ascii_lowercase();
    normalized.contains("codex_official") || normalized.contains("codexofficial")
}

fn upstream_is_codex_official(upstream: &str) -> bool {
    let normalized = upstream.to_ascii_lowercase();
    normalized.contains("chatgpt.com/backend-api/codex")
}

fn should_use_known_model_list(
    profile: &deecodex::providers::ProviderProfile,
    upstream: &str,
    endpoint_kind: Option<&str>,
    oauth_account: bool,
) -> bool {
    profile.slug == "codex"
        && (oauth_account
            || upstream_is_codex_official(upstream)
            || endpoint_kind
                .map(endpoint_kind_is_codex_official)
                .unwrap_or(false))
}

/// 查询余额/额度信息，自动探测端点与计费模式
#[derive(Serialize)]
pub struct BalanceInfo {
    pub mode: String,
    pub credit_remaining: Option<f64>,
    pub credit_limit: Option<f64>,
    pub credit_label: Option<String>,
    pub weekly_remaining: Option<String>,
    pub weekly_limit: Option<String>,
    pub hours_5_remaining: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_remains: Option<Vec<ModelRemain>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub official: Option<Value>,
}

#[derive(Serialize)]
pub struct ModelRemain {
    pub model_name: String,
    pub interval_total: f64,
    pub interval_used: f64,
    pub weekly_total: f64,
    pub weekly_used: f64,
}

const CODEX_WHAM_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

fn codex_quota_user_agent() -> String {
    format!(
        "codex_cli_rs/0.118.0 (Mac OS 26.3.1; arm64) DEXAI/{}",
        env!("CARGO_PKG_VERSION")
    )
}

#[derive(Debug, Clone)]
struct CodexUsageError {
    status: u16,
    code: String,
    message: String,
    body: String,
}

impl CodexUsageError {
    fn local(message: impl Into<String>) -> Self {
        Self {
            status: 0,
            code: "local_error".into(),
            message: message.into(),
            body: String::new(),
        }
    }

    fn from_response(status: u16, body: String) -> Self {
        let parsed = serde_json::from_str::<Value>(&body).unwrap_or_else(|_| json!({}));
        let code = parsed
            .pointer("/error/code")
            .or_else(|| parsed.pointer("/code"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let message = parsed
            .pointer("/error/message")
            .or_else(|| parsed.pointer("/message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        Self {
            status,
            code,
            message,
            body,
        }
    }

    fn is_auth_unavailable(&self) -> bool {
        let code = self.code.to_ascii_lowercase();
        let message = self.message.to_ascii_lowercase();
        self.status == 401
            || code == "auth_unavailable"
            || code == "invalid_api_key"
            || message.contains("authentication token has been invalidated")
            || message.contains("signing in again")
    }

    fn user_message(&self) -> String {
        if !self.message.trim().is_empty() {
            return format!("额度接口 HTTP {}: {}", self.status, self.message);
        }
        if !self.code.trim().is_empty() {
            return format!("额度接口 HTTP {}: {}", self.status, self.code);
        }
        if !self.body.trim().is_empty() {
            return format!("额度接口 HTTP {}: {}", self.status, self.body);
        }
        format!("额度接口 HTTP {}", self.status)
    }
}

fn is_codex_official_oauth_account(account: &deecodex::accounts::Account) -> bool {
    account.client_kind.is_codex()
        && matches!(
            &account.auth_mode,
            deecodex::accounts::AccountAuthMode::OAuth
        )
        && account.endpoints.iter().any(|endpoint| {
            matches!(
                &endpoint.kind,
                deecodex::accounts::EndpointKind::CodexOfficial
            )
        })
}

async fn request_stats_since_for_account(
    manager: &ServerManager,
    since_secs: u64,
    account_id: &str,
) -> deecodex::request_history::RequestStats {
    let filter = deecodex::request_history::HistoryFilter {
        client_kind: Some("codex".into()),
        account_id: Some(account_id.to_string()),
    };
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        return store.stats_since(since_secs, &filter).await;
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    if let Some(state) = guard.as_ref() {
        return state.request_history.stats_since(since_secs, &filter).await;
    }
    deecodex::request_history::RequestStats::default()
}

async fn refresh_oauth_token_if_needed(
    account: &mut deecodex::accounts::Account,
    client: &reqwest::Client,
) -> Option<String> {
    let oauth_value = account.client_options.get("oauth").cloned()?;
    let token = deecodex::oauth_accounts::oauth_token_from_value(&oauth_value)?;
    let now = deecodex::oauth_accounts::now_secs();
    if token.expired_at == 0 || token.expired_at > now.saturating_add(60) {
        return None;
    }
    refresh_oauth_token_for_account(account, client).await.err()
}

async fn refresh_oauth_token_for_account(
    account: &mut deecodex::accounts::Account,
    client: &reqwest::Client,
) -> Result<(), String> {
    let oauth_value = account
        .client_options
        .get("oauth")
        .cloned()
        .ok_or_else(|| "账号缺少 OAuth token".to_string())?;
    let token = deecodex::oauth_accounts::oauth_token_from_value(&oauth_value)
        .ok_or_else(|| "OAuth token 格式无效".to_string())?;
    if token.refresh_token.trim().is_empty() {
        return Err("OAuth refresh_token 为空，无法主动刷新".into());
    }
    let provider = match deecodex::oauth_accounts::OAuthProvider::parse(&token.provider) {
        Ok(provider) => provider,
        Err(err) => return Err(err.to_string()),
    };
    match deecodex::oauth_accounts::refresh_token(client, &provider, &token.refresh_token).await {
        Ok(mut refreshed) => {
            if refreshed.refresh_token.trim().is_empty() {
                refreshed.refresh_token = token.refresh_token.clone();
            }
            if refreshed.id_token.trim().is_empty() {
                refreshed.id_token = token.id_token.clone();
            }
            if refreshed.email.trim().is_empty() {
                refreshed.email = token.email.clone();
            }
            if refreshed.account_id.trim().is_empty() {
                refreshed.account_id = token.account_id.clone();
            }
            let login_mode = oauth_value
                .get("login_mode")
                .and_then(Value::as_str)
                .unwrap_or("browser");
            account.api_key = refreshed.access_token.clone();
            account.client_options.insert(
                "oauth".into(),
                deecodex::oauth_accounts::oauth_token_to_value(&refreshed, login_mode),
            );
            Ok(())
        }
        Err(err) => Err(format!("OAuth token 刷新失败: {err}")),
    }
}

async fn fetch_codex_wham_usage_once(
    client: &reqwest::Client,
    account: &deecodex::accounts::Account,
) -> Result<Value, CodexUsageError> {
    let oauth_value = account
        .client_options
        .get("oauth")
        .ok_or_else(|| CodexUsageError::local("账号缺少 OAuth token"))?;
    let oauth = deecodex::oauth_accounts::oauth_token_from_value(oauth_value)
        .ok_or_else(|| CodexUsageError::local("OAuth token 格式无效"))?;
    let access_token = if oauth.access_token.trim().is_empty() {
        account.api_key.trim()
    } else {
        oauth.access_token.trim()
    };
    if access_token.is_empty() {
        return Err(CodexUsageError::local("OAuth access token 为空"));
    }

    let mut request = client
        .get(CODEX_WHAM_USAGE_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .header("Originator", "codex_cli_rs")
        .header("User-Agent", codex_quota_user_agent())
        .header("Connection", "Keep-Alive");
    if !oauth.account_id.trim().is_empty() {
        request = request.header("Chatgpt-Account-Id", oauth.account_id.trim());
    }

    let response = request
        .send()
        .await
        .map_err(|err| CodexUsageError::local(format!("额度接口请求失败: {err}")))?;
    let status = response.status();
    let status_code = status.as_u16();
    let body = response
        .text()
        .await
        .map_err(|err| CodexUsageError::local(format!("额度接口响应读取失败: {err}")))?;
    if !status.is_success() {
        return Err(CodexUsageError::from_response(status_code, body));
    }
    serde_json::from_str::<Value>(&body)
        .map_err(|err| CodexUsageError::local(format!("额度接口 JSON 解析失败: {err}")))
}

async fn fetch_codex_wham_usage(
    client: &reqwest::Client,
    account: &mut deecodex::accounts::Account,
) -> (Option<Value>, Option<String>) {
    match fetch_codex_wham_usage_once(client, account).await {
        Ok(payload) => (Some(payload), None),
        Err(err) if err.is_auth_unavailable() => {
            tracing::warn!(
                account_id = %account.id,
                status = err.status,
                code = %err.code,
                "Codex 额度接口 token 失效，尝试刷新 OAuth token 后重试"
            );
            match refresh_oauth_token_for_account(account, client).await {
                Ok(()) => match fetch_codex_wham_usage_once(client, account).await {
                    Ok(payload) => (Some(payload), None),
                    Err(retry_err) => (None, Some(retry_err.user_message())),
                },
                Err(refresh_err) => (
                    None,
                    Some(format!(
                        "{}；自动刷新失败: {}",
                        err.user_message(),
                        refresh_err
                    )),
                ),
            }
        }
        Err(err) => (None, Some(err.user_message())),
    }
}

fn number_to_u64(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().map(|v| v.round().max(0.0) as u64)),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn used_to_remaining_percent(used_percent: Option<u64>) -> Option<u64> {
    used_percent.map(|used| 100u64.saturating_sub(used.min(100)))
}

fn codex_rate_limit_window(rate_limit: &Value, key: &str) -> Value {
    let window = rate_limit.get(key).unwrap_or(&Value::Null);
    let used_percent = number_to_u64(window.get("used_percent"));
    let remaining_percent = used_to_remaining_percent(used_percent);
    json!({
        "used_percent": used_percent,
        "remaining_percent": remaining_percent,
        "reset_at": number_to_u64(window.get("reset_at")),
        "reset_after_seconds": number_to_u64(window.get("reset_after_seconds")),
        "limit_window_seconds": number_to_u64(window.get("limit_window_seconds")),
    })
}

fn codex_window_u64(window: &Value, key: &str) -> Option<u64> {
    window.get(key).and_then(Value::as_u64)
}

fn codex_next_usage_reset(payload: &Value, now: u64) -> Option<u64> {
    let rate_limit = payload.get("rate_limit").unwrap_or(&Value::Null);
    ["primary_window", "secondary_window"]
        .iter()
        .filter_map(|key| {
            number_to_u64(
                rate_limit
                    .get(*key)
                    .and_then(|window| window.get("reset_at")),
            )
        })
        .filter(|reset_at| *reset_at > now)
        .min()
}

async fn official_oauth_balance(
    manager: &ServerManager,
    account: &mut deecodex::accounts::Account,
) -> BalanceInfo {
    let client = reqwest::Client::new();
    let refresh_error = refresh_oauth_token_if_needed(account, &client).await;
    let now = deecodex::accounts::now_secs();
    let (usage_payload, usage_error) = fetch_codex_wham_usage(&client, account).await;
    let mut primary_window = json!(null);
    let mut secondary_window = json!(null);
    let mut usage_allowed = None;
    let mut usage_limit_reached = None;
    let mut usage_next_recover_at = None;

    if let Some(payload) = usage_payload.as_ref() {
        let rate_limit = payload.get("rate_limit").unwrap_or(&Value::Null);
        primary_window = codex_rate_limit_window(rate_limit, "primary_window");
        secondary_window = codex_rate_limit_window(rate_limit, "secondary_window");
        let allowed = rate_limit
            .get("allowed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let limit_reached = rate_limit
            .get("limit_reached")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        usage_allowed = Some(allowed);
        usage_limit_reached = Some(limit_reached);
        if !allowed || limit_reached {
            usage_next_recover_at = codex_next_usage_reset(payload, now);
            account.runtime_state.status = deecodex::accounts::AccountRuntimeStatus::QuotaExceeded;
            account.runtime_state.status_message =
                "ChatGPT WHAM usage 显示 Codex 额度已触顶".into();
            account.runtime_state.next_retry_after = usage_next_recover_at;
            account.runtime_state.quota = deecodex::accounts::AccountQuotaState {
                exceeded: true,
                reason: "quota".into(),
                next_recover_at: usage_next_recover_at,
                backoff_level: account.runtime_state.quota.backoff_level,
            };
        } else if matches!(
            account.runtime_state.status,
            deecodex::accounts::AccountRuntimeStatus::CoolingDown
                | deecodex::accounts::AccountRuntimeStatus::QuotaExceeded
        ) {
            account.clear_runtime_cooldown(now);
        }
    }

    let stats_5h =
        request_stats_since_for_account(manager, now.saturating_sub(5 * 60 * 60), &account.id)
            .await;
    let stats_7d =
        request_stats_since_for_account(manager, now.saturating_sub(7 * 24 * 60 * 60), &account.id)
            .await;
    let routing = deecodex::accounts::account_routing_options(account);
    let oauth = account
        .client_options
        .get("oauth")
        .and_then(deecodex::oauth_accounts::oauth_token_from_value);
    let token_info = oauth
        .as_ref()
        .map(|token| deecodex::oauth_accounts::codex_id_token_info(&token.id_token))
        .unwrap_or_else(|| json!({}));
    let plan_type = token_info
        .get("plan_type")
        .and_then(Value::as_str)
        .or_else(|| {
            usage_payload
                .as_ref()
                .and_then(|payload| payload.get("plan_type"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .to_string();
    let usage_plan_type = usage_payload
        .as_ref()
        .and_then(|payload| payload.get("plan_type"))
        .and_then(Value::as_str)
        .unwrap_or(&plan_type)
        .to_string();
    let runtime = &account.runtime_state;
    let next_recover_at = usage_next_recover_at
        .or(runtime.quota.next_recover_at)
        .or(runtime.next_retry_after);
    let blocked = next_recover_at.is_some_and(|ts| ts > now)
        || matches!(
            &runtime.status,
            deecodex::accounts::AccountRuntimeStatus::CoolingDown
                | deecodex::accounts::AccountRuntimeStatus::QuotaExceeded
        );
    let quota_exceeded = usage_limit_reached.unwrap_or(runtime.quota.exceeded)
        || usage_allowed.is_some_and(|allowed| !allowed);
    let status_label = if usage_error.is_some() {
        "刷新失败"
    } else if !routing.effective_enabled() {
        "未参与账号池"
    } else if quota_exceeded {
        "额度耗尽"
    } else if blocked
        && matches!(
            &runtime.status,
            deecodex::accounts::AccountRuntimeStatus::QuotaExceeded
        )
    {
        "额度冷却"
    } else if blocked {
        "冷却中"
    } else if matches!(
        &runtime.status,
        deecodex::accounts::AccountRuntimeStatus::Error
    ) {
        "最近错误"
    } else {
        "可用"
    };
    let message = usage_error
        .clone()
        .or(refresh_error.clone())
        .unwrap_or_else(|| {
            if usage_payload.is_some() {
                "已从 ChatGPT WHAM usage 获取真实 Codex 5h/7d 剩余额度。".into()
            } else {
                "暂无真实额度数据；请稍后重试刷新额度。".into()
            }
        });
    let fallback_message = || {
        if matches!(
            &runtime.status,
            deecodex::accounts::AccountRuntimeStatus::QuotaExceeded
        ) {
            "官方返回额度限制，已按恢复时间暂停该账号。".into()
        } else if stats_7d.total == 0 {
            "暂无真实额度数据；先显示计划与本地窗口用量。".into()
        } else {
            "未触发官方额度限制；额度窗口按本机请求历史统计。".into()
        }
    };
    let message = if usage_payload.is_some() || usage_error.is_some() || refresh_error.is_some() {
        message
    } else {
        fallback_message()
    };
    let primary_remaining_percent = codex_window_u64(&primary_window, "remaining_percent");
    let secondary_remaining_percent = codex_window_u64(&secondary_window, "remaining_percent");
    let primary_used_percent = codex_window_u64(&primary_window, "used_percent");
    let secondary_used_percent = codex_window_u64(&secondary_window, "used_percent");
    let primary_reset_at = codex_window_u64(&primary_window, "reset_at");
    let secondary_reset_at = codex_window_u64(&secondary_window, "reset_at");
    let status = serde_json::to_value(&runtime.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "active".into());
    let effective_plan_type = if usage_plan_type.trim().is_empty() {
        plan_type
    } else {
        usage_plan_type
    };
    let source = if usage_payload.is_some() {
        "chatgpt_wham_usage"
    } else {
        "local_runtime"
    };
    let confidence_level = if usage_payload.is_some() {
        "精确"
    } else {
        "本地状态"
    };
    let is_estimated = usage_payload.is_none();
    let rate_limit_reached_type = usage_payload
        .as_ref()
        .and_then(|payload| payload.get("rate_limit_reached_type"))
        .cloned()
        .unwrap_or(Value::Null);
    let usage_account_id = usage_payload
        .as_ref()
        .and_then(|payload| payload.get("account_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let usage_user_id = usage_payload
        .as_ref()
        .and_then(|payload| payload.get("user_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let usage_email = usage_payload
        .as_ref()
        .and_then(|payload| payload.get("email"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut official_map = serde_json::Map::new();
    official_map.insert("checked_at".into(), json!(now));
    official_map.insert("provider".into(), json!("codex"));
    official_map.insert("title".into(), json!("Codex 额度"));
    official_map.insert("status".into(), json!(status));
    official_map.insert("status_label".into(), json!(status_label));
    official_map.insert(
        "status_message".into(),
        json!(runtime.status_message.clone()),
    );
    official_map.insert("quota_exceeded".into(), json!(quota_exceeded));
    official_map.insert("quota_reason".into(), json!(runtime.quota.reason.clone()));
    official_map.insert("next_recover_at".into(), json!(next_recover_at));
    official_map.insert("routing_enabled".into(), json!(routing.effective_enabled()));
    official_map.insert("pool".into(), json!(routing.pool));
    official_map.insert("priority".into(), json!(routing.priority));
    official_map.insert("weight".into(), json!(routing.weight));
    official_map.insert("plan_type".into(), json!(effective_plan_type));
    official_map.insert(
        "account_id".into(),
        json!(oauth
            .as_ref()
            .map(|token| token.account_id.clone())
            .unwrap_or_default()),
    );
    official_map.insert("usage_account_id".into(), json!(usage_account_id));
    official_map.insert("usage_user_id".into(), json!(usage_user_id));
    official_map.insert(
        "email".into(),
        json!(oauth
            .as_ref()
            .map(|token| token.email.clone())
            .unwrap_or_default()),
    );
    official_map.insert("usage_email".into(), json!(usage_email));
    official_map.insert(
        "token_expired_at".into(),
        json!(oauth
            .as_ref()
            .map(|token| token.expired_at)
            .unwrap_or_default()),
    );
    official_map.insert(
        "last_refresh".into(),
        json!(oauth
            .as_ref()
            .map(|token| token.last_refresh.clone())
            .unwrap_or_default()),
    );
    official_map.insert("allowed".into(), json!(usage_allowed));
    official_map.insert("limit_reached".into(), json!(usage_limit_reached));
    official_map.insert("rate_limit_reached_type".into(), rate_limit_reached_type);
    official_map.insert("primary_window".into(), primary_window);
    official_map.insert("secondary_window".into(), secondary_window);
    official_map.insert(
        "hours_5_remaining_percent".into(),
        json!(primary_remaining_percent),
    );
    official_map.insert("hours_5_used_percent".into(), json!(primary_used_percent));
    official_map.insert("hours_5_reset_at".into(), json!(primary_reset_at));
    official_map.insert(
        "weekly_remaining_percent".into(),
        json!(secondary_remaining_percent),
    );
    official_map.insert("weekly_used_percent".into(), json!(secondary_used_percent));
    official_map.insert("weekly_reset_at".into(), json!(secondary_reset_at));
    official_map.insert("requests_5h".into(), json!(stats_5h.total));
    official_map.insert("success_5h".into(), json!(stats_5h.success_count));
    official_map.insert(
        "failed_5h".into(),
        json!(stats_5h.total.saturating_sub(stats_5h.success_count)),
    );
    official_map.insert("tokens_5h".into(), json!(stats_5h.total_tokens));
    official_map.insert("requests_7d".into(), json!(stats_7d.total));
    official_map.insert("success_7d".into(), json!(stats_7d.success_count));
    official_map.insert(
        "failed_7d".into(),
        json!(stats_7d.total.saturating_sub(stats_7d.success_count)),
    );
    official_map.insert("tokens_7d".into(), json!(stats_7d.total_tokens));
    official_map.insert("message".into(), json!(message));
    official_map.insert("refresh_error".into(), json!(refresh_error));
    official_map.insert("usage_error".into(), json!(usage_error));
    official_map.insert("source".into(), json!(source));
    official_map.insert("confidence_level".into(), json!(confidence_level));
    official_map.insert("is_estimated".into(), json!(is_estimated));
    let official = Value::Object(official_map);
    account
        .client_options
        .insert("oauth_quota".into(), official.clone());
    account.updated_at = now;
    BalanceInfo {
        mode: "official_oauth".into(),
        credit_remaining: None,
        credit_limit: None,
        credit_label: None,
        weekly_remaining: secondary_remaining_percent.map(|percent| format!("{percent}%")),
        weekly_limit: None,
        hours_5_remaining: primary_remaining_percent.map(|percent| format!("{percent}%")),
        model_remains: None,
        official: Some(official),
    }
}

#[tauri::command]
pub async fn fetch_balance(
    manager: State<'_, ServerManager>,
    account_id: String,
) -> Result<BalanceInfo, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account_index = store
        .accounts
        .iter()
        .position(|a| a.id == account_id)
        .ok_or_else(|| "账号不存在".to_string())?;
    if is_codex_official_oauth_account(&store.accounts[account_index]) {
        let mut account = store.accounts[account_index].clone();
        let info = official_oauth_balance(&manager, &mut account).await;
        let (store, account) =
            mutate_account_store(&data_dir, "保存账号额度状态失败", |store| {
                let existing = store
                    .accounts
                    .iter_mut()
                    .find(|candidate| candidate.id == account_id)
                    .ok_or_else(|| "账号不存在".to_string())?;
                existing.api_key = account.api_key.clone();
                existing.client_options = account.client_options.clone();
                existing.runtime_state = account.runtime_state.clone();
                existing.updated_at = deecodex::accounts::now_secs();
                Ok(existing.clone())
            })?;
        sync_account_mutation_to_running_state(&manager, &store, &account).await;
        return Ok(info);
    }
    let account = &store.accounts[account_index];
    let profile = deecodex::providers::profile_for_account(account);
    let endpoint = endpoint_for_account_in_store(account, &store);
    let upstream = endpoint
        .map(|endpoint| endpoint.base_url.as_str())
        .unwrap_or(&account.upstream)
        .trim_end_matches('/')
        .to_string();
    let api_key = account.api_key.clone();

    if api_key.is_empty() {
        return Ok(BalanceInfo {
            mode: "unsupported".into(),
            credit_remaining: None,
            credit_limit: None,
            credit_label: None,
            weekly_remaining: None,
            weekly_limit: None,
            hours_5_remaining: None,
            model_remains: None,
            official: None,
        });
    }

    let client = reqwest::Client::new();

    let balance_url = endpoint
        .map(|endpoint| endpoint.balance_url.as_str())
        .filter(|url| !url.is_empty())
        .unwrap_or(&account.balance_url);

    // 如果端点/账号配置了自定义 balance_url，直接用该 URL 探测
    if !balance_url.is_empty() {
        let url = balance_url.trim_end_matches('/').to_string();
        let mut req = client.get(&url);
        for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
            req = req.header(name, value);
        }
        tracing::info!("使用自定义 balance_url 探测: {}", url);
        match req.send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(body) = resp.json::<Value>().await {
                        if let Some(info) = try_parse_balance(&body) {
                            return Ok(info);
                        }
                        tracing::info!("自定义 balance_url 解析未能匹配: {:?}", body);
                    }
                } else {
                    tracing::info!(
                        "自定义 balance_url HTTP {}: {}",
                        resp.status().as_u16(),
                        url
                    );
                }
            }
            Err(e) => tracing::info!("自定义 balance_url 请求失败: {} → {}", url, e),
        }
        return Ok(BalanceInfo {
            mode: "unsupported".into(),
            credit_remaining: None,
            credit_limit: None,
            credit_label: None,
            weekly_remaining: None,
            weekly_limit: None,
            hours_5_remaining: None,
            model_remains: None,
            official: None,
        });
    }

    // 生成基础 URL 列表：完整 upstream + 去除 /v1、/v1beta、/api/v1 的根路径
    let mut bases = vec![upstream.clone()];
    for strip in &["/v1", "/v1beta", "/api/v1"] {
        if let Some(root) = upstream.strip_suffix(strip) {
            let root = root.to_string();
            if root != upstream && !bases.contains(&root) {
                bases.push(root);
            }
        }
    }

    // 按顺序尝试各端点：(路径后缀, 是否允许返回非 200 也不放弃)
    let probes: Vec<&str> = vec![
        "/v1/coding_plan/remains",
        "/v1/api/openplatform/coding_plan/remains",
        "/user/balance",
        "/auth/key",
        "/v1/auth/key",
        "/api/v1/auth/key",
        "/v1/billing/info",
        "/v1/account/info",
        "/v1/account",
        "/v1/user/info",
        "/v1/billing",
        "/v1/dashboard/billing/credit_grants",
        "/v1/dashboard/billing/subscription",
        "/v1/subscription",
        "/v1/usage",
        "/v1/plan",
        "/v1/quota",
        "/v1/api/user/info",
    ];

    fn try_parse_balance(body: &Value) -> Option<BalanceInfo> {
        // 1. MiniMax 风格: { base_resp: { status_code: 0 }, model_remains: [...] }
        if body["base_resp"]["status_code"].as_i64() == Some(0) {
            if let Some(remains) = body["model_remains"].as_array() {
                let models: Vec<ModelRemain> = remains
                    .iter()
                    .map(|m| ModelRemain {
                        model_name: m["model_name"].as_str().unwrap_or("?").into(),
                        interval_total: m["current_interval_total_count"].as_f64().unwrap_or(0.0),
                        interval_used: m["current_interval_usage_count"].as_f64().unwrap_or(0.0),
                        weekly_total: m["current_weekly_total_count"].as_f64().unwrap_or(0.0),
                        weekly_used: m["current_weekly_usage_count"].as_f64().unwrap_or(0.0),
                    })
                    .collect();
                return Some(BalanceInfo {
                    mode: "coding_plan".into(),
                    credit_remaining: None,
                    credit_limit: None,
                    credit_label: None,
                    weekly_remaining: None,
                    weekly_limit: None,
                    hours_5_remaining: None,
                    model_remains: Some(models),
                    official: None,
                });
            }
        }

        // 2. OpenRouter 风格: { data: { limit_remaining, limit, label } }
        let data = body.get("data").unwrap_or(body);
        let cr = data["limit_remaining"].as_f64();
        let cl = data["limit"].as_f64();
        if cr.is_some() || cl.is_some() {
            return Some(BalanceInfo {
                mode: "token_credit".into(),
                credit_remaining: cr,
                credit_limit: cl,
                credit_label: data["label"].as_str().map(String::from),
                weekly_remaining: None,
                weekly_limit: None,
                hours_5_remaining: None,
                model_remains: None,
                official: None,
            });
        }

        // 3. DeepSeek 风格: { balance_infos: [{ total_balance, currency }] }
        if let Some(infos) = body["balance_infos"].as_array() {
            if let Some(first) = infos.first() {
                if let Some(total) = first["total_balance"].as_str() {
                    let cr = total.parse::<f64>().ok();
                    return Some(BalanceInfo {
                        mode: "token_credit".into(),
                        credit_remaining: cr,
                        credit_limit: None,
                        credit_label: first["currency"].as_str().map(String::from),
                        weekly_remaining: None,
                        weekly_limit: None,
                        hours_5_remaining: None,
                        model_remains: None,
                        official: None,
                    });
                }
            }
        }

        // 4. data 为数组: { data: [{ balance / credit / quota, ... }] }
        if let Some(arr) = data.as_array().and_then(|a| a.first()) {
            for key in &[
                "balance",
                "credit",
                "credit_remaining",
                "quota",
                "remaining",
            ] {
                if let Some(v) = arr[key].as_f64() {
                    return Some(BalanceInfo {
                        mode: "token_credit".into(),
                        credit_remaining: Some(v),
                        credit_limit: arr["limit"].as_f64().or(arr["credit_limit"].as_f64()),
                        credit_label: arr["currency"].as_str().map(String::from),
                        weekly_remaining: None,
                        weekly_limit: None,
                        hours_5_remaining: None,
                        model_remains: None,
                        official: None,
                    });
                }
            }
        }

        // 5. 顶层 token/credit 相关字段
        for key in &[
            "balance",
            "credit",
            "credit_remaining",
            "total_balance",
            "quota",
            "remaining_quota",
            "token_balance",
            "remaining",
        ] {
            if let Some(v) = body[key].as_f64() {
                return Some(BalanceInfo {
                    mode: "token_credit".into(),
                    credit_remaining: Some(v),
                    credit_limit: None,
                    credit_label: body["currency"].as_str().map(String::from),
                    weekly_remaining: None,
                    weekly_limit: None,
                    hours_5_remaining: None,
                    model_remains: None,
                    official: None,
                });
            }
        }

        // 6. 订阅模式: { subscription / plan: { weekly_remaining, ... } }
        if let Some(sub) = body.get("subscription").or(body.get("plan")) {
            return Some(BalanceInfo {
                mode: "subscription".into(),
                credit_remaining: None,
                credit_limit: None,
                credit_label: None,
                weekly_remaining: sub
                    .get("weekly_remaining")
                    .and_then(|v| v.as_str().or_else(|| v.as_number().map(|_| "")))
                    .map(|s| s.to_string()),
                weekly_limit: sub
                    .get("weekly_limit")
                    .and_then(|v| v.as_str().or_else(|| v.as_number().map(|_| "")))
                    .map(|s| s.to_string()),
                hours_5_remaining: sub
                    .get("5h_remaining")
                    .or(sub.get("hours_5_remaining"))
                    .and_then(|v| v.as_str().or_else(|| v.as_number().map(|_| "")))
                    .map(|s| s.to_string()),
                model_remains: None,
                official: None,
            });
        }

        None
    }

    for probe in &probes {
        for base in &bases {
            let url = format!("{}{}", base, probe);
            let mut req = client.get(&url);
            for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
                req = req.header(name, value);
            }
            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        match resp.json::<Value>().await {
                            Ok(body) => {
                                tracing::info!(
                                    "余额探测成功: {} → body keys: {:?}",
                                    url,
                                    body.as_object().map(|o| o.keys().collect::<Vec<_>>())
                                );
                                if let Some(info) = try_parse_balance(&body) {
                                    return Ok(info);
                                }
                                tracing::info!("余额解析未能匹配已知格式: {:?}", body);
                            }
                            Err(e) => tracing::info!("余额探测 JSON 解析失败: {} → {}", url, e),
                        }
                    } else {
                        tracing::info!("余额探测 HTTP {}: {}", status.as_u16(), url);
                    }
                }
                Err(e) => tracing::debug!("余额探测请求失败: {} → {}", url, e),
            }
        }
    }
    tracing::info!("余额探测全部失败: upstream={}, bases={:?}", upstream, bases);

    Ok(BalanceInfo {
        mode: "unsupported".into(),
        credit_remaining: None,
        credit_limit: None,
        credit_label: None,
        weekly_remaining: None,
        weekly_limit: None,
        hours_5_remaining: None,
        model_remains: None,
        official: None,
    })
}

// ── 会话管理 ──────────────────────────────────────────────────────────────

/// 列出所有活跃会话
#[tauri::command]
pub async fn list_sessions(manager: State<'_, ServerManager>) -> Result<Value, String> {
    sessions::list_sessions_impl(manager).await
}

/// 删除会话（先备份）
#[tauri::command]
pub async fn delete_session(
    manager: State<'_, ServerManager>,
    session_type: String,
    session_id: String,
) -> Result<Value, String> {
    sessions::delete_session_impl(manager, session_type, session_id).await
}

/// 撤销删除会话
#[tauri::command]
pub async fn undo_delete_session(
    manager: State<'_, ServerManager>,
    undo_token: String,
) -> Result<Value, String> {
    sessions::undo_delete_session_impl(manager, undo_token).await
}

// ── 辅助函数 ──────────────────────────────────────────────────────────────

fn account_to_value(a: &deecodex::accounts::Account) -> Value {
    let endpoint = if a.client_kind.is_codex() {
        a.endpoints.first()
    } else {
        None
    };
    account_to_value_with_endpoint(a, endpoint)
}

fn account_to_value_for_store(
    a: &deecodex::accounts::Account,
    store: &deecodex::accounts::AccountStore,
) -> Value {
    let endpoint = endpoint_for_account_in_store(a, store);
    account_to_value_with_endpoint(a, endpoint)
}

fn endpoint_for_account_in_store<'a>(
    account: &'a deecodex::accounts::Account,
    store: &deecodex::accounts::AccountStore,
) -> Option<&'a deecodex::accounts::EndpointConfig> {
    if !account.client_kind.is_codex() {
        return None;
    }
    if store
        .active_selection_for_surface(&AccountClientKind::Codex, &account.client_surface)
        .and_then(|selection| selection.account_id.as_deref())
        == Some(account.id.as_str())
    {
        account.active_endpoint(
            store
                .active_endpoint_id_for_surface(&AccountClientKind::Codex, &account.client_surface),
        )
    } else {
        account.endpoints.first()
    }
}

fn account_to_value_with_endpoint(
    a: &deecodex::accounts::Account,
    endpoint: Option<&deecodex::accounts::EndpointConfig>,
) -> Value {
    let upstream = endpoint
        .map(|endpoint| endpoint.base_url.as_str())
        .unwrap_or(&a.upstream);
    let model_map = endpoint
        .map(|endpoint| endpoint.model_map.clone())
        .unwrap_or_else(|| a.model_map.clone());
    let vision = endpoint.map(|endpoint| &endpoint.vision);
    let balance_url = endpoint
        .map(|endpoint| endpoint.balance_url.as_str())
        .unwrap_or(&a.balance_url);
    let raw_vision_api_key = vision
        .map(|v| v.api_key.clone())
        .unwrap_or_else(|| a.vision_api_key.clone());
    let mut value = json!({
        "id": a.id,
        "name": a.name,
        "provider": a.provider,
        "client_kind": a.client_kind,
        "target": a.client_kind,
        "wire_protocol": a.wire_protocol,
        "upstream": upstream,
        "api_key": mask_secret(&a.api_key),
        "api_key_present": !a.api_key.is_empty(),
        "auth_mode": a.auth_mode.clone(),
        "default_model": a.default_model,
        "client_options": redact_client_options(a.client_options.clone()),
        "routing": deecodex::accounts::account_routing_options(a),
        "runtime_state": a.runtime_state.clone(),
        "last_applied_at": a.last_applied_at,
        "last_check": a.last_check,
        "model_map": model_map,
        "vision_upstream": vision.map(|v| v.base_url.clone()).unwrap_or_else(|| a.vision_upstream.clone()),
        "vision_api_key": mask_secret(&raw_vision_api_key),
        "vision_api_key_present": !raw_vision_api_key.is_empty(),
        "vision_model": vision.map(|v| v.model.clone()).unwrap_or_else(|| a.vision_model.clone()),
        "vision_endpoint": vision.map(|v| v.path.clone()).unwrap_or_else(|| a.vision_endpoint.clone()),
        "vision_enabled": vision.map(|v| v.mode == deecodex::accounts::VisionMode::Glue).unwrap_or(a.vision_enabled),
        "context_window_override": endpoint.and_then(|e| e.context_window_override),
        "reasoning_effort_override": endpoint.and_then(|e| e.reasoning_effort_override.clone()),
        "thinking_tokens": endpoint.and_then(|e| e.thinking_tokens),
        "custom_headers": endpoint.map(|e| e.custom_headers.clone()).unwrap_or_else(|| a.custom_headers.clone()),
        "request_timeout_secs": endpoint.and_then(|e| e.request_timeout_secs),
        "max_retries": endpoint.and_then(|e| e.max_retries),
        "translate_enabled": endpoint.map(|e| e.kind.is_chat_like()).unwrap_or(a.translate_enabled),
        "provider_options": a.provider_options,
        "capability_enabled": a.capability_enabled,
        "capability_account_id": a.capability_account_id,
        "endpoints": redact_endpoints(&a.endpoints),
        "active_endpoint_name": endpoint.map(|e| e.name.clone()).unwrap_or_default(),
        "active_endpoint_kind": endpoint.map(|e| format!("{:?}", e.kind)).unwrap_or_default(),
        "active_vision_mode": endpoint.map(|e| format!("{:?}", e.vision.mode)).unwrap_or_default(),
        "from_codex_config": a.from_codex_config,
        "balance_url": balance_url,
        "created_at": a.created_at,
        "updated_at": a.updated_at,
    });
    value["client_surface"] = json!(a.client_surface);
    value["dev_pipeline_enabled"] = json!(a.dev_pipeline_enabled);
    value["dev_pipeline_trigger_mode"] = json!(a.dev_pipeline_trigger_mode);
    value["dev_pipeline_command"] = json!(a.dev_pipeline_command);
    value["dev_pipeline_architect_account_id"] = json!(a.dev_pipeline_architect_account_id);
    value["dev_pipeline_implementer_account_id"] = json!(a.dev_pipeline_implementer_account_id);
    value["dev_pipeline_reviewer_account_id"] = json!(a.dev_pipeline_reviewer_account_id);
    value["dev_pipeline_tool_mode"] = json!(a.dev_pipeline_tool_mode);
    value["dev_pipeline_max_iterations"] = json!(a.dev_pipeline_max_iterations);
    value["dev_pipeline_show_trace"] = json!(a.dev_pipeline_show_trace);
    value["dev_pipeline_architect_instruction"] = json!(a.dev_pipeline_architect_instruction);
    value["dev_pipeline_implementer_instruction"] = json!(a.dev_pipeline_implementer_instruction);
    value["dev_pipeline_reviewer_instruction"] = json!(a.dev_pipeline_reviewer_instruction);
    value
}

// ── 线程聚合 ──────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_threads_status(manager: State<'_, ServerManager>) -> Result<Value, String> {
    threads::get_threads_status_impl(manager).await
}

#[tauri::command]
pub async fn list_threads() -> Result<Value, String> {
    threads::list_threads_impl().await
}

#[tauri::command]
pub async fn get_thread_sources(manager: State<'_, ServerManager>) -> Result<Value, String> {
    threads::get_thread_sources_impl(manager).await
}

#[tauri::command]
pub async fn list_client_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    threads::list_client_threads_impl(manager).await
}

#[tauri::command]
pub async fn migrate_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    threads::migrate_threads_impl(manager).await
}

#[tauri::command]
pub async fn restore_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    threads::restore_threads_impl(manager).await
}

#[tauri::command]
pub async fn calibrate_threads(manager: State<'_, ServerManager>) -> Result<Value, String> {
    threads::calibrate_threads_impl(manager).await
}

#[tauri::command]
pub async fn get_thread_content(thread_id: String) -> Result<Value, String> {
    threads::get_thread_content_impl(thread_id).await
}

#[tauri::command]
pub async fn get_client_thread_content(
    client_kind: String,
    native_id: String,
    thread_key: Option<String>,
) -> Result<Value, String> {
    threads::get_client_thread_content_impl(client_kind, native_id, thread_key).await
}

#[tauri::command]
pub async fn delete_thread(
    manager: State<'_, ServerManager>,
    thread_id: String,
) -> Result<Value, String> {
    threads::delete_thread_impl(manager, thread_id).await
}

/// 连通性检测结果
struct ConnectivityResult {
    ok: bool,
    status_code: u16,
    latency_ms: u128,
    model_count: Option<usize>,
    endpoint: String,
    error: Option<String>,
}

/// 执行上游连通性检测（内部使用）
async fn do_test_connectivity(upstream: &str, api_key: &str) -> Result<ConnectivityResult, String> {
    do_test_connectivity_with_kind(upstream, api_key, None).await
}

async fn do_test_connectivity_with_kind(
    upstream: &str,
    api_key: &str,
    endpoint_kind: Option<&str>,
) -> Result<ConnectivityResult, String> {
    let provider = deecodex::providers::guess_provider(upstream);
    let profile = deecodex::providers::profile_by_slug(provider);
    let base = upstream.trim_end_matches('/');
    let url = deecodex::providers::model_discovery_url(&profile, upstream, api_key)
        .unwrap_or_else(|| format!("{base}/models"));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let req = model_probe_request(&client, &url, api_key, endpoint_kind);
    let start = std::time::Instant::now();
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency_ms = start.elapsed().as_millis();
            let body = resp.text().await.unwrap_or_default();
            let model_count = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .map(|v| deecodex::providers::parse_models_response(&profile, &v).len())
                .filter(|count| *count > 0);
            Ok(ConnectivityResult {
                ok: status < 500,
                status_code: status,
                latency_ms,
                model_count,
                endpoint: url,
                error: None,
            })
        }
        Err(e) => Ok(ConnectivityResult {
            ok: false,
            status_code: 0,
            latency_ms: start.elapsed().as_millis(),
            model_count: None,
            endpoint: url,
            error: Some(e.to_string()),
        }),
    }
}

async fn resolve_upstream_connectivity_args(
    manager: &ServerManager,
    account_id: Option<String>,
    upstream: Option<String>,
    api_key: Option<String>,
    endpoint_kind: Option<String>,
) -> Result<(String, String, Option<String>), String> {
    if let Some(account_id) = non_empty_override(account_id) {
        let data_dir = manager.data_dir.lock().await.clone();
        let store = deecodex::accounts::load_accounts(&data_dir);
        let account = store
            .accounts
            .iter()
            .find(|account| account.id == account_id)
            .ok_or_else(|| format!("账号不存在: {account_id}"))?;
        let endpoint = endpoint_for_account_in_store(account, &store);
        let upstream = non_empty_override(upstream).unwrap_or_else(|| {
            endpoint
                .map(|ep| ep.base_url.clone())
                .unwrap_or_else(|| account.upstream.clone())
        });
        let api_key = secret_override(api_key).unwrap_or_else(|| account.api_key.clone());
        let endpoint_kind = endpoint_kind.or_else(|| endpoint.map(|ep| format!("{:?}", ep.kind)));
        return Ok((upstream, api_key, endpoint_kind));
    }

    Ok((
        non_empty_override(upstream).ok_or("缺少 upstream 参数")?,
        secret_override(api_key).unwrap_or_default(),
        endpoint_kind,
    ))
}

fn build_vision_probe_url(upstream: &str, vision_path: &str) -> Result<String, String> {
    let upstream = upstream.trim().trim_end_matches('/');
    if upstream.is_empty() {
        return Err("视觉上游 URL 为空".into());
    }
    deecodex::handlers::validate_upstream(upstream)
        .map_err(|e| format!("视觉上游 URL 无效: {e}"))?;

    let path = vision_path.trim().trim_start_matches('/');
    if path.is_empty() {
        return Err("视觉端点路径为空".into());
    }

    let base = if upstream.ends_with("/v1") && path.starts_with("v1/") {
        upstream.trim_end_matches("/v1")
    } else {
        upstream
    };
    Ok(format!("{base}/{path}"))
}

fn classify_minimax_vlm_probe(status: u16, body: &Value) -> (bool, Option<String>, Option<String>) {
    let base_status = body["base_resp"]["status_code"].as_i64();
    let base_msg = body["base_resp"]["status_msg"].as_str().unwrap_or("");
    let content = body["content"].as_str().unwrap_or("");

    if status >= 500 {
        return (
            false,
            None,
            Some(format!("HTTP {status}: {}", truncate_for_ui(base_msg, 180))),
        );
    }

    if matches!(base_status, Some(2049))
        || base_msg.to_ascii_lowercase().contains("invalid api key")
    {
        return (
            false,
            base_status.map(|code| format!("MiniMax base_resp={code}")),
            Some("MiniMax API Key 无效或与当前 API Host 不匹配".into()),
        );
    }

    if !content.is_empty() || base_status == Some(0) {
        return (
            true,
            Some("MiniMax VLM 返回 content，视觉端点可用".into()),
            None,
        );
    }

    if matches!(base_status, Some(2013 | 1026)) {
        return (
            true,
            Some(format!(
                "MiniMax VLM 鉴权通过，探测图片被上游校验拒绝: {base_status:?} {base_msg}"
            )),
            None,
        );
    }

    if status < 500 {
        return (
            true,
            Some(format!(
                "MiniMax VLM 返回 HTTP {status}，base_resp={}",
                base_status
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "未知".into())
            )),
            None,
        );
    }

    (false, None, Some("视觉端点探测失败".into()))
}

fn truncate_for_ui(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            return out;
        }
        out.push(ch);
    }
    out
}

fn model_probe_request(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    endpoint_kind: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut req = client.get(url);
    if api_key.is_empty() {
        return req;
    }
    let is_longcat = url.to_ascii_lowercase().contains("longcat.chat");
    let is_anthropic = endpoint_kind
        .map(|kind| {
            let kind = kind.to_ascii_lowercase();
            kind.contains("anthropic")
        })
        .unwrap_or_else(|| url.to_ascii_lowercase().contains("anthropic.com"));
    if is_anthropic && !is_longcat {
        req = req
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01");
    } else {
        req = req.bearer_auth(api_key);
    }
    req
}

async fn resolve_vision_connectivity_args(
    manager: &ServerManager,
    account_id: Option<String>,
    endpoint_id: Option<String>,
    upstream: Option<String>,
    api_key: Option<String>,
    vision_path: Option<String>,
) -> Result<(String, String, Option<String>), String> {
    if let Some(account_id) = non_empty_override(account_id) {
        let data_dir = manager.data_dir.lock().await.clone();
        let store = deecodex::accounts::load_accounts(&data_dir);
        let account = store
            .accounts
            .iter()
            .find(|account| account.id == account_id)
            .ok_or_else(|| format!("账号不存在: {account_id}"))?;
        let endpoint = endpoint_id
            .as_deref()
            .and_then(|id| account.endpoints.iter().find(|endpoint| endpoint.id == id))
            .or_else(|| endpoint_for_account_in_store(account, &store));
        let stored_upstream = endpoint
            .map(|endpoint| endpoint.vision.base_url.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| account.vision_upstream.clone());
        let stored_api_key = endpoint
            .map(|endpoint| endpoint.vision.api_key.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| account.vision_api_key.clone());
        let stored_path = endpoint
            .map(|endpoint| endpoint.vision.path.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| account.vision_endpoint.clone());
        let upstream = non_empty_override(upstream).unwrap_or(stored_upstream);
        let api_key = secret_override(api_key).unwrap_or(stored_api_key);
        let vision_path =
            non_empty_override(vision_path).or_else(|| non_empty_override(Some(stored_path)));
        return Ok((upstream, api_key, vision_path));
    }

    Ok((
        non_empty_override(upstream).ok_or("缺少视觉上游 URL")?,
        secret_override(api_key).unwrap_or_default(),
        vision_path,
    ))
}

/// 测试胶水视觉 API 端点连通性
#[tauri::command]
pub async fn test_vision_connectivity(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    endpoint_id: Option<String>,
    upstream: Option<String>,
    api_key: Option<String>,
    vision_path: Option<String>,
    adapter_id: Option<String>,
) -> Result<Value, String> {
    let adapter = adapter_id.unwrap_or_else(|| "minimax_coding_plan_vlm".into());
    if adapter != "minimax_coding_plan_vlm" {
        return Ok(json!({
            "ok": false,
            "status": 0,
            "latency_ms": 0,
            "endpoint": "",
            "adapter": adapter,
            "detail": null,
            "error": "当前版本仅实现 MiniMax Coding Plan VLM 胶水适配器"
        }));
    }

    let (upstream, api_key, vision_path) = resolve_vision_connectivity_args(
        &manager,
        account_id,
        endpoint_id,
        upstream,
        api_key,
        vision_path,
    )
    .await?;
    let endpoint = build_vision_probe_url(
        &upstream,
        vision_path.as_deref().unwrap_or("v1/coding_plan/vlm"),
    )?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let mut req = client
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .json(&json!({
            "prompt": "deecodex vision connectivity probe",
            "image_url": "https://example.invalid/deecodex-vision-probe.png"
        }));
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key.trim());
    }

    let start = std::time::Instant::now();
    match req.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.json::<Value>().await.unwrap_or_else(|_| json!({}));
            let (ok, detail, error) = classify_minimax_vlm_probe(status, &body);
            Ok(json!({
                "ok": ok,
                "status": status,
                "latency_ms": start.elapsed().as_millis(),
                "endpoint": endpoint,
                "adapter": adapter,
                "base_status": body["base_resp"]["status_code"],
                "detail": detail,
                "error": error,
            }))
        }
        Err(e) => Ok(json!({
            "ok": false,
            "status": 0,
            "latency_ms": start.elapsed().as_millis(),
            "endpoint": endpoint,
            "adapter": adapter,
            "detail": null,
            "error": e.to_string(),
        })),
    }
}

/// 测试上游 API 端点连通性
#[tauri::command]
pub async fn test_upstream_connectivity(
    manager: State<'_, ServerManager>,
    account_id: Option<String>,
    upstream: Option<String>,
    api_key: Option<String>,
    endpoint_kind: Option<String>,
) -> Result<Value, String> {
    let (upstream, api_key, endpoint_kind) =
        resolve_upstream_connectivity_args(&manager, account_id, upstream, api_key, endpoint_kind)
            .await?;
    let r = do_test_connectivity_with_kind(&upstream, &api_key, endpoint_kind.as_deref()).await?;
    Ok(serde_json::json!({
        "ok": r.ok,
        "status": r.status_code,
        "latency_ms": r.latency_ms,
        "model_count": r.model_count,
        "endpoint": r.endpoint,
        "error": r.error,
    }))
}

#[tauri::command]
pub async fn list_request_history(
    manager: State<'_, ServerManager>,
    limit: Option<usize>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    request_history::list_request_history_impl(manager, limit, client_kind, account_id).await
}

#[tauri::command]
pub async fn clear_request_history(
    manager: State<'_, ServerManager>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    request_history::clear_request_history_impl(manager, client_kind, account_id).await
}

#[tauri::command]
pub async fn get_monthly_stats(
    manager: State<'_, ServerManager>,
    limit: Option<usize>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    request_history::get_monthly_stats_impl(manager, limit, client_kind, account_id).await
}

#[tauri::command]
pub async fn get_request_stats_since(
    manager: State<'_, ServerManager>,
    since: Option<u64>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    request_history::get_request_stats_since_impl(manager, since, client_kind, account_id).await
}

#[tauri::command]
pub async fn browse_file() -> Result<Option<String>, String> {
    dialogs::browse_file_impl().await
}

#[tauri::command]
pub async fn browse_attachment_file() -> Result<Option<String>, String> {
    dialogs::browse_attachment_file_impl().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use deecodex::accounts::Account;
    use std::path::PathBuf;

    fn test_args() -> Args {
        Args {
            command: None,
            config: None,
            port: 4446,
            host: deecodex::config::default_host(),
            upstream: "https://openrouter.ai/api/v1".into(),
            api_key: String::new(),
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
            cdp_port: 9222,
            prompts_dir: PathBuf::from("prompts"),
            data_dir: PathBuf::from(".deecodex"),
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

    fn preview_defaults() -> RuntimeDefaults {
        RuntimeDefaults {
            preview: true,
            port: 4556,
            data_dir: PathBuf::from("/tmp/.deecodex-preview"),
        }
    }

    #[test]
    fn preview_runtime_defaults_replace_core_defaults() {
        let defaults = preview_defaults();
        let args = apply_runtime_data_dir_default(test_args(), &defaults, false);
        let args = apply_runtime_port_default(args, &defaults, false, false);

        assert_eq!(args.port, 4556);
        assert_eq!(args.data_dir, defaults.data_dir);
    }

    #[test]
    fn preview_runtime_defaults_preserve_explicit_values() {
        let defaults = preview_defaults();
        let mut args = test_args();
        args.port = 4666;
        args.data_dir = PathBuf::from("/tmp/custom-preview");
        let args = apply_runtime_data_dir_default(args, &defaults, false);
        let args = apply_runtime_port_default(args, &defaults, false, false);

        assert_eq!(args.port, 4666);
        assert_eq!(args.data_dir, PathBuf::from("/tmp/custom-preview"));
    }

    #[test]
    fn preview_runtime_defaults_preserve_env_and_file_values() {
        let defaults = preview_defaults();
        let args = apply_runtime_data_dir_default(test_args(), &defaults, true);
        let args = apply_runtime_port_default(args, &defaults, true, false);

        assert_eq!(args.port, 4446);
        assert_eq!(args.data_dir, PathBuf::from(".deecodex"));
    }

    #[test]
    fn secret_override_rejects_redacted_values() {
        assert_eq!(
            secret_override(Some("sk-1234".into())).as_deref(),
            Some("sk-1234")
        );
        assert!(secret_override(Some("sk-****-abcd".into())).is_none());
        assert!(secret_override(Some("   ".into())).is_none());
    }

    #[test]
    fn codex_auth_json_import_accepts_cli_proxy_shape() {
        let value = json!({
            "type": "codex",
            "email": "alpha@example.com",
            "access_token": "access-1",
            "refresh_token": "refresh-1",
            "account_id": "acct_1",
            "expired_at": 12345u64
        });

        let token = codex_oauth_token_from_auth_json(&value, 100).unwrap();

        assert_eq!(token.provider, "codex");
        assert_eq!(token.access_token, "access-1");
        assert_eq!(token.refresh_token, "refresh-1");
        assert_eq!(token.email, "alpha@example.com");
        assert_eq!(token.account_id, "acct_1");
        assert_eq!(token.expired_at, 12345);
    }

    #[test]
    fn codex_auth_json_import_accepts_nested_tokens_shape() {
        let value = json!({
            "tokens": {
                "access_token": "access-2",
                "refresh_token": "refresh-2",
                "expires_in": 3600
            }
        });

        let token = codex_oauth_token_from_auth_json(&value, 100).unwrap();

        assert_eq!(token.access_token, "access-2");
        assert_eq!(token.refresh_token, "refresh-2");
        assert_eq!(token.expired_at, 3700);
    }

    #[test]
    fn imported_codex_account_joins_official_pool() {
        let token = deecodex::oauth_accounts::OAuthToken {
            provider: "codex".into(),
            access_token: "access-3".into(),
            refresh_token: "refresh-3".into(),
            id_token: String::new(),
            email: "pool@example.com".into(),
            account_id: "acct_pool".into(),
            expired: String::new(),
            expired_at: 0,
            last_refresh: String::new(),
        };

        let account =
            codex_account_from_imported_token(token, "pool.json", AccountClientSurface::Cli, 100);
        let routing = deecodex::accounts::account_routing_options(&account);

        assert_eq!(account.provider, "codex");
        assert_eq!(account.client_surface, AccountClientSurface::Cli);
        assert_eq!(
            account.auth_mode,
            deecodex::accounts::AccountAuthMode::OAuth
        );
        assert_eq!(routing.pool, "codex-official");
        assert!(routing.effective_enabled());
        assert!(routing.effective_anchor_enabled_for_account(&account));
        assert!(!routing.effective_execution_enabled_for_account(&account));
        assert!(account
            .endpoints
            .iter()
            .any(|endpoint| endpoint.kind == deecodex::accounts::EndpointKind::CodexOfficial));
    }

    #[test]
    fn imported_codex_oauth_duplicate_is_scoped_by_surface() {
        let token = deecodex::oauth_accounts::OAuthToken {
            provider: "codex".into(),
            access_token: "access-surface".into(),
            refresh_token: "refresh-surface".into(),
            id_token: String::new(),
            email: "surface@example.com".into(),
            account_id: "acct_surface".into(),
            expired: String::new(),
            expired_at: 0,
            last_refresh: String::new(),
        };

        let cli_account = codex_account_from_imported_token(
            token.clone(),
            "cli.json",
            AccountClientSurface::Cli,
            100,
        );
        let desktop_account = codex_account_from_imported_token(
            token.clone(),
            "desktop.json",
            AccountClientSurface::Desktop,
            100,
        );

        assert!(same_imported_codex_oauth(
            &cli_account,
            &token,
            &AccountClientSurface::Cli
        ));
        assert!(!same_imported_codex_oauth(
            &cli_account,
            &token,
            &AccountClientSurface::Desktop
        ));
        assert!(same_imported_codex_oauth(
            &desktop_account,
            &token,
            &AccountClientSurface::Desktop
        ));
    }

    #[test]
    fn activate_client_surface_account_scopes_claude_without_global_switch() {
        let mut cli = test_account("claude-cli");
        cli.client_kind = AccountClientKind::ClaudeCode;
        cli.client_surface = AccountClientSurface::Cli;
        cli.translate_enabled = false;

        let mut desktop = test_account("claude-desktop");
        desktop.client_kind = AccountClientKind::ClaudeCode;
        desktop.client_surface = AccountClientSurface::Desktop;
        desktop.translate_enabled = false;

        let mut hermes = test_account("hermes");
        hermes.client_kind = AccountClientKind::Hermes;
        hermes.client_surface = AccountClientSurface::Cli;
        hermes.translate_enabled = false;

        let mut store = AccountStore {
            version: deecodex::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![cli.clone(), desktop.clone(), hermes.clone()],
            active_id: Some("codex-global".into()),
            active_account_id: Some("codex-global".into()),
            active_endpoint_id: Some("codex-endpoint".into()),
            active_by_surface: HashMap::new(),
        };

        activate_client_surface_account(&mut store, &cli, None, true);
        activate_client_surface_account(&mut store, &desktop, None, true);
        activate_client_surface_account(&mut store, &hermes, None, true);

        assert_eq!(store.active_account_id.as_deref(), Some("codex-global"));
        assert_eq!(
            store
                .active_selection_for_surface(
                    &AccountClientKind::ClaudeCode,
                    &AccountClientSurface::Cli
                )
                .and_then(|selection| selection.account_id.as_deref()),
            Some("claude-cli")
        );
        assert_eq!(
            store
                .active_selection_for_surface(
                    &AccountClientKind::ClaudeCode,
                    &AccountClientSurface::Desktop
                )
                .and_then(|selection| selection.account_id.as_deref()),
            Some("claude-desktop")
        );
        assert_eq!(
            store
                .active_selection_for_surface(
                    &AccountClientKind::Hermes,
                    &AccountClientSurface::Cli
                )
                .and_then(|selection| selection.account_id.as_deref()),
            Some("hermes")
        );
    }

    fn test_account(id: &str) -> deecodex::accounts::Account {
        deecodex::accounts::Account {
            id: id.into(),
            name: "Test".into(),
            provider: "deepseek".into(),
            client_kind: Default::default(),
            client_surface: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://api.deepseek.com/v1".into(),
            api_key: "test-key".into(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: Default::default(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 1,
            updated_at: 1,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: Default::default(),
            provider_options: deecodex::providers::provider_options_for_slug("deepseek"),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    }

    #[test]
    fn parse_account_json_accepts_client_kind_with_legacy_target() {
        let raw = json!({
            "id": "a1",
            "name": "Hermes",
            "provider": "openrouter",
            "client_kind": "hermes",
            "target": "hermes",
            "upstream": "https://openrouter.ai/api/v1",
            "api_key": "sk-test"
        })
        .to_string();

        let account = parse_account_json(&raw).unwrap();

        assert_eq!(account.client_kind, AccountClientKind::Hermes);
    }

    #[test]
    fn parse_account_json_keeps_target_only_legacy_payloads() {
        let raw = json!({
            "id": "a1",
            "name": "Claude Code",
            "provider": "anthropic",
            "target": "claude_code",
            "upstream": "https://api.anthropic.com",
            "api_key": "sk-test"
        })
        .to_string();

        let account = parse_account_json(&raw).unwrap();

        assert_eq!(account.client_kind, AccountClientKind::ClaudeCode);
    }

    #[test]
    fn explicit_client_kind_overrides_default_codex_payload() {
        let raw = json!({
            "id": "a1",
            "name": "MiniMax",
            "provider": "minimax",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://api.minimaxi.com/v1",
            "api_key": "sk-test",
            "translate_enabled": true,
            "endpoints": [{
                "id": "ep1",
                "name": "Chat 兼容",
                "kind": "open_ai_chat",
                "base_url": "https://api.minimaxi.com/v1"
            }]
        })
        .to_string();
        let mut account = parse_account_json(&raw).unwrap();

        apply_explicit_account_client(
            &mut account,
            Some(&AccountClientKind::Hermes),
            Some("desktop"),
        );
        account.normalize_v2();

        assert_eq!(account.client_kind, AccountClientKind::Hermes);
        assert_eq!(account.client_surface, AccountClientSurface::Cli);
        assert!(!account.translate_enabled);
        assert!(account.endpoints.is_empty());
        assert_eq!(account.client_options["client_kind"], "hermes");
        assert_eq!(account.client_options["client_surface"], "cli");
    }

    #[test]
    fn account_backed_config_preserves_fields_from_existing_config() {
        let mut existing = test_args();
        existing.upstream = "https://account.example/v1".into();
        existing.api_key = "account-key".into();
        existing.model_map = r#"{"gpt-5.5":"deepseek-v4-pro"}"#.into();
        existing.vision_upstream = "https://vision.example/v1".into();
        existing.vision_api_key = "vision-key".into();
        existing.vision_model = "vision-model".into();
        existing.vision_endpoint = "v1/vision".into();

        let preserved = account_backed_config(Some(&existing));

        assert_eq!(preserved.upstream, "https://account.example/v1");
        assert_eq!(preserved.api_key, "account-key");
        assert_eq!(preserved.model_map, r#"{"gpt-5.5":"deepseek-v4-pro"}"#);
        assert_eq!(preserved.vision_upstream, "https://vision.example/v1");
        assert_eq!(preserved.vision_api_key, "vision-key");
        assert_eq!(preserved.vision_model, "vision-model");
        assert_eq!(preserved.vision_endpoint, "v1/vision");
    }

    #[test]
    fn account_backed_config_is_empty_without_existing_config() {
        let preserved = account_backed_config(None);

        assert!(preserved.upstream.is_empty());
        assert!(preserved.api_key.is_empty());
        assert!(preserved.model_map.is_empty());
        assert!(preserved.vision_upstream.is_empty());
        assert!(preserved.vision_api_key.is_empty());
        assert!(preserved.vision_model.is_empty());
        assert!(preserved.vision_endpoint.is_empty());
    }

    #[test]
    fn editable_client_config_seed_is_client_specific_and_redacted() {
        let mut account = test_account("client");
        account.client_kind = AccountClientKind::Hermes;
        account.provider = "minimax".into();
        account.upstream = "https://api.minimaxi.com/v1".into();
        account.api_key = "sk-secret-should-not-leak".into();
        account.default_model = "MiniMax-M2.7".into();
        account
            .client_options
            .insert("api_key_env".into(), json!("MINIMAX_API_KEY"));
        account.client_options.insert(
            "model_map".into(),
            json!({
                "default": "MiniMax-M2.7",
                "vision": "MiniMax-VL-01"
            }),
        );

        let text = initial_client_config_text(&account);

        assert!(text.contains("model:"));
        assert!(text.contains("api_key_env: MINIMAX_API_KEY"));
        assert!(text.contains("vision:"));
        assert!(text.contains("MiniMax-VL-01"));
        assert!(!text.contains("sk-secret-should-not-leak"));
    }

    #[test]
    fn config_editor_validates_common_config_formats() {
        assert_eq!(
            validate_config_text_for_editor("toml", "[model_providers.deecodex]\nname = \"x\"\n")
                ["ok"],
            true
        );
        assert_eq!(
            validate_config_text_for_editor("json", "{\"env\":{}}")["ok"],
            true
        );
        assert_eq!(
            validate_config_text_for_editor("yaml", "model:\n  default: MiniMax-M2.7\n")["ok"],
            true
        );
        assert_eq!(
            validate_config_text_for_editor("env", "OPENAI_MODEL=gpt-5\n")["ok"],
            true
        );
        assert_eq!(validate_config_text_for_editor("json", "{")["ok"], false);
        assert_eq!(
            validate_config_text_for_editor("env", "OPENAI_MODEL")["ok"],
            false
        );
    }

    #[test]
    fn codex_config_editor_uses_codex_toml_target() {
        let mut account = test_account("codex");
        account.client_kind = AccountClientKind::Codex;

        let target = account_config_target(&account).unwrap();

        assert_eq!(target.format, "toml");
        assert!(target.path.ends_with(".codex/config.toml"));
        assert!(initial_account_config_text(&account).contains("Codex config.toml"));
    }

    #[test]
    fn account_to_value_exposes_capability_fields() {
        let account = Account {
            id: "main".into(),
            name: "主账号".into(),
            provider: "deepseek".into(),
            client_kind: Default::default(),
            client_surface: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://api.deepseek.com/v1".into(),
            api_key: "sk-test".into(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 0,
            updated_at: 0,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: HashMap::new(),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: true,
            capability_account_id: Some("helper".into()),
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        };

        let value = account_to_value(&account);

        assert_eq!(value["capability_enabled"], true);
        assert_eq!(value["capability_account_id"], "helper");
    }

    #[test]
    fn endpoint_selection_uses_active_endpoint_only_for_active_account() {
        let mut active = test_account("active");
        active.name = "Active".into();
        active.provider = "openrouter".into();
        active.upstream = "https://active-default.example/v1".into();
        active.api_key = "active-key".into();
        active.normalize_v2();
        let mut active_second = active.endpoints[0].clone();
        active_second.id = "shared_endpoint_id".into();
        active_second.base_url = "https://active-selected.example/v1".into();
        active.endpoints.push(active_second);

        let mut other = active.clone();
        other.id = "other".into();
        other.name = "Other".into();
        other.endpoints[0].base_url = "https://other-default.example/v1".into();
        other.endpoints.push({
            let mut endpoint = other.endpoints[0].clone();
            endpoint.id = "shared_endpoint_id".into();
            endpoint.base_url = "https://other-shared.example/v1".into();
            endpoint
        });

        let store = deecodex::accounts::AccountStore {
            version: deecodex::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![active.clone(), other.clone()],
            active_id: Some(active.id.clone()),
            active_account_id: Some(active.id.clone()),
            active_endpoint_id: Some("shared_endpoint_id".into()),
            active_by_surface: HashMap::from([(
                deecodex::accounts::surface_active_key(
                    &AccountClientKind::Codex,
                    &AccountClientSurface::Cli,
                ),
                deecodex::accounts::SurfaceActiveSelection {
                    account_id: Some(active.id.clone()),
                    endpoint_id: Some("shared_endpoint_id".into()),
                },
            )]),
        };

        let active_endpoint = endpoint_for_account_in_store(&active, &store).unwrap();
        let other_endpoint = endpoint_for_account_in_store(&other, &store).unwrap();

        assert_eq!(
            active_endpoint.base_url,
            "https://active-selected.example/v1"
        );
        assert_eq!(other_endpoint.base_url, "https://other-default.example/v1");
    }

    #[test]
    fn minimax_vision_probe_url_avoids_duplicate_v1() {
        assert_eq!(
            build_vision_probe_url("https://api.minimaxi.com", "v1/coding_plan/vlm").unwrap(),
            "https://api.minimaxi.com/v1/coding_plan/vlm"
        );
        assert_eq!(
            build_vision_probe_url("https://api.minimaxi.com/v1", "v1/coding_plan/vlm").unwrap(),
            "https://api.minimaxi.com/v1/coding_plan/vlm"
        );
    }

    #[test]
    fn minimax_vlm_probe_treats_validation_response_as_connected() {
        let body = json!({
            "base_resp": {
                "status_code": 2013,
                "status_msg": "invalid params, invalid image URL"
            },
            "content": ""
        });
        let (ok, detail, error) = classify_minimax_vlm_probe(200, &body);

        assert!(ok);
        assert!(detail.unwrap().contains("鉴权通过"));
        assert!(error.is_none());
    }

    #[test]
    fn minimax_vlm_probe_rejects_invalid_api_key() {
        let body = json!({
            "base_resp": {
                "status_code": 2049,
                "status_msg": "invalid api key"
            }
        });
        let (ok, _, error) = classify_minimax_vlm_probe(200, &body);

        assert!(!ok);
        assert!(error.unwrap().contains("API Key"));
    }

    #[test]
    fn account_events_are_filtered_newest_first_and_limited() {
        let data_dir = std::env::temp_dir().join(format!(
            "deecodex-account-events-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(
            data_dir.join("account-events.jsonl"),
            [
                json!({"ts": 10, "account_id": "a", "action": "old"}).to_string(),
                "not-json".to_string(),
                json!({"ts": 30, "account_id": "b", "action": "other"}).to_string(),
                json!({"ts": 20, "account_id": "a", "action": "new"}).to_string(),
            ]
            .join("\n"),
        )
        .unwrap();

        let events = read_account_events(&data_dir, Some("a"), 1);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["action"], "new");

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_process_arg_parser_keeps_user_data_dir_with_spaces() {
        let line = "123 /Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper --type=renderer --user-data-dir=/Users/test/Library/Application Support/Claude-3p --lang=zh-CN";
        assert_eq!(
            extract_command_arg_value(line, "--user-data-dir=").as_deref(),
            Some("/Users/test/Library/Application Support/Claude-3p")
        );
    }

    #[test]
    fn same_client_account_matches_path_or_provider_tuple() {
        let mut account = test_account("client");
        account.client_kind = AccountClientKind::Hermes;
        account.provider = "openrouter".into();
        account.upstream = "https://openrouter.ai/api/v1".into();
        account.default_model = "anthropic/claude-sonnet-4".into();
        account
            .client_options
            .insert("config_path".into(), json!("/tmp/hermes.yaml"));

        let mut candidate = deecodex::client_integrations::ClientImportCandidate {
            client_kind: AccountClientKind::Hermes,
            client_surface: AccountClientSurface::Cli,
            name: "Hermes".into(),
            provider: "anthropic".into(),
            upstream: "https://api.anthropic.com".into(),
            api_key: "sk-test".into(),
            default_model: "claude-sonnet-4-5".into(),
            client_options: HashMap::new(),
            source_path: Some("/tmp/hermes.yaml".into()),
            warnings: Vec::new(),
        };
        assert!(same_client_account(&account, &candidate));

        candidate.source_path = Some("/tmp/other.yaml".into());
        candidate.provider = account.provider.clone();
        candidate.upstream = account.upstream.clone();
        candidate.default_model = account.default_model.clone();
        assert!(same_client_account(&account, &candidate));

        candidate.client_surface = AccountClientSurface::Desktop;
        assert!(!same_client_account(&account, &candidate));
        candidate.client_surface = AccountClientSurface::Cli;

        candidate.client_kind = AccountClientKind::ClaudeCode;
        assert!(!same_client_account(&account, &candidate));
    }

    #[test]
    fn migrate_existing_legacy_array_accounts_file_writes_v2() {
        let data_dir = std::env::temp_dir().join(format!(
            "deecodex-migrate-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        let path = deecodex::accounts::accounts_file_path(&data_dir);
        std::fs::write(
            &path,
            serde_json::to_string(&vec![test_account("legacy")]).unwrap(),
        )
        .unwrap();

        let store = migrate_or_load_accounts(&data_dir);
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(store.version, deecodex::accounts::ACCOUNT_STORE_VERSION);
        assert_eq!(
            saved["version"].as_u64(),
            Some(deecodex::accounts::ACCOUNT_STORE_VERSION as u64)
        );
        assert_eq!(
            saved["accounts"][0]["endpoints"].as_array().unwrap().len(),
            1
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[tokio::test]
    async fn build_app_state_uses_active_endpoint_advanced_fields() {
        let data_dir = std::env::temp_dir().join(format!(
            "deecodex-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();

        let mut account = test_account("active");
        account.name = "Active".into();
        account.provider = "custom".into();
        account.upstream = "https://legacy.example/v1".into();
        account.api_key = "account-key".into();
        account.normalize_v2();
        let endpoint = account.endpoints.first_mut().unwrap();
        endpoint.id = "selected".into();
        endpoint.base_url = "https://selected.example/v1".into();
        endpoint.kind = deecodex::accounts::EndpointKind::CustomChat;
        endpoint
            .model_map
            .insert("gpt-5".into(), "upstream-model".into());
        endpoint
            .custom_headers
            .insert("x-test".into(), "yes".into());
        endpoint.request_timeout_secs = Some(42);
        endpoint.max_retries = Some(5);
        endpoint.reasoning_effort_override = Some("high".into());
        endpoint.thinking_tokens = Some(2048);
        endpoint.vision.mode = deecodex::accounts::VisionMode::Glue;
        endpoint.vision.base_url = "https://vision.example/v1".into();
        endpoint.vision.api_key = "vision-key".into();
        endpoint.vision.model = "vision-model".into();
        endpoint.vision.path = "v1/coding_plan/vlm".into();

        let store = deecodex::accounts::AccountStore {
            version: deecodex::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![account],
            active_id: Some("active".into()),
            active_account_id: Some("active".into()),
            active_endpoint_id: Some("selected".into()),
            active_by_surface: HashMap::new(),
        };
        deecodex::accounts::save_accounts(&data_dir, &store).unwrap();

        let mut args = test_args();
        args.data_dir = data_dir.clone();
        args.prompts_dir = data_dir.join("prompts");
        let state = build_app_state(&args).unwrap();

        assert_eq!(
            state.upstream.read().await.as_str(),
            "https://selected.example/v1"
        );
        assert_eq!(state.api_key.read().await.as_str(), "account-key");
        assert_eq!(
            state
                .model_map
                .read()
                .await
                .get("gpt-5")
                .map(String::as_str),
            Some("upstream-model")
        );
        assert_eq!(
            state
                .custom_headers
                .read()
                .await
                .get("x-test")
                .map(String::as_str),
            Some("yes")
        );
        assert_eq!(*state.request_timeout_secs.read().await, Some(42));
        assert_eq!(
            state.reasoning_effort_override.read().await.as_deref(),
            Some("high")
        );
        assert_eq!(*state.thinking_tokens.read().await, Some(2048));
        assert_eq!(
            state
                .vision_upstream
                .read()
                .await
                .as_ref()
                .map(|url| url.as_str().to_string()),
            Some("https://vision.example/v1".into())
        );
        assert_eq!(state.vision_api_key.read().await.as_str(), "vision-key");
        assert_eq!(state.vision_model.read().await.as_str(), "vision-model");
        assert_eq!(
            state.vision_endpoint.read().await.as_str(),
            "v1/coding_plan/vlm"
        );

        let _ = std::fs::remove_dir_all(data_dir);
    }

    #[test]
    fn codex_usage_window_converts_used_to_remaining_percent() {
        let rate_limit = json!({
            "primary_window": {
                "used_percent": 1,
                "reset_at": 1_779_454_627u64,
                "reset_after_seconds": 18_000,
                "limit_window_seconds": 18_000
            }
        });

        let window = codex_rate_limit_window(&rate_limit, "primary_window");

        assert_eq!(window["used_percent"], 1);
        assert_eq!(window["remaining_percent"], 99);
        assert_eq!(window["reset_at"], 1_779_454_627u64);
    }

    #[test]
    fn codex_usage_auth_unavailable_detects_invalidated_token() {
        let err = CodexUsageError::from_response(
            401,
            r#"{"error":{"message":"Your authentication token has been invalidated. Please try signing in again.","type":"authentication_error","code":"auth_unavailable"}}"#.into(),
        );

        assert!(err.is_auth_unavailable());
    }
}
