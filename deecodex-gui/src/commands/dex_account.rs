// DEX AI 主账号：个人中心唯一登录入口 + OAuth 授权中心客户端（Phase 1a）。
// 主会话（refresh token）由本模块持有并落盘到 data_dir（0600），
// 用于调用 control-plane 的授权与授权管理接口。
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

const CONTROL_PLANE_DEFAULT_URL: &str = "https://api.liguan.me/mobile-codex";
const MASTER_CLIENT_ID: &str = "dexai-desktop";
const MASTER_SESSION_FILE: &str = "dexai-master-session.json";
// 桌面端 scope：基础（profile/consent/grants）+ 账号云同步（vault）+ 偏好设置（prefs）。
// 需要 control-plane 侧 dexai-desktop 客户端在 oauth_clients.scopes 中允许这些 scope。
const MASTER_SCOPES: &str = "dexai:profile.read dexai:consent dexai:grants dexai:vault dexai:prefs";

pub(crate) fn control_plane_url() -> String {
    std::env::var("DEX_AI_CONTROL_PLANE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| CONTROL_PLANE_DEFAULT_URL.to_string())
}

pub(crate) fn http_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .expect("构建 DEX AI 主账号 HTTP 客户端失败")
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MasterSession {
    pub(crate) client_id: String,
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_in: i64,
    pub(crate) scope: String,
    pub(crate) account_id: Option<String>,
    pub(crate) email: Option<String>,
}

fn session_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MASTER_SESSION_FILE)
}

pub(crate) fn load_session(data_dir: &Path) -> Option<MasterSession> {
    let raw = std::fs::read_to_string(session_path(data_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_session(data_dir: &Path, session: &MasterSession) -> Result<(), String> {
    let path = session_path(data_dir);
    let raw = serde_json::to_string_pretty(session).map_err(|error| error.to_string())?;
    // 防御性确保数据目录存在（写入不创建父目录）
    if let Err(error) = std::fs::create_dir_all(data_dir) {
        return Err(format!(
            "保存主账号会话失败：无法访问数据目录 {}：{error}",
            data_dir.display()
        ));
    }
    // 已有会话文件若因属主/权限异常不可写，尝试恢复可写后重写；
    // 若目录本身由其它用户拥有，这里会失败并给出明确提示。
    if path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.permissions().readonly() {
                    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
    std::fs::write(&path, raw).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            format!(
                "保存主账号会话失败：无权限写入 {}。若数据目录属主异常，请先运行：\
                 sudo chown -R \"$(whoami)\" {} && sudo chmod -R u+rwX {}",
                path.display(),
                data_dir.display(),
                data_dir.display()
            )
        } else {
            format!("保存主账号会话失败: {error}")
        }
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn clear_session(data_dir: &Path) {
    let path = session_path(data_dir);
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

async fn userinfo(client: &Client, base: &str, access_token: &str) -> Result<Value, String> {
    let response = client
        .get(format!("{base}/v1/oauth/userinfo"))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| format!("请求控制面失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if status.is_success() {
        Ok(body)
    } else if status == reqwest::StatusCode::UNAUTHORIZED {
        Err("登录状态已失效".to_string())
    } else {
        Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("请求控制面失败")
            .to_string())
    }
}

async fn refresh_session(
    client: &Client,
    base: &str,
    session: &MasterSession,
) -> Result<MasterSession, String> {
    let response = client
        .post(format!("{base}/v1/oauth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", &session.client_id),
            ("refresh_token", &session.refresh_token),
        ])
        .send()
        .await
        .map_err(|error| format!("刷新主账号会话失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(body
            .get("error_description")
            .and_then(Value::as_str)
            .or_else(|| body.get("error").and_then(Value::as_str))
            .unwrap_or("刷新主账号会话失败")
            .to_string());
    }
    Ok(MasterSession {
        client_id: session.client_id.clone(),
        access_token: body["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        refresh_token: body["refresh_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        expires_in: body["expires_in"].as_i64().unwrap_or(0),
        scope: body["scope"].as_str().unwrap_or_default().to_string(),
        account_id: session.account_id.clone(),
        email: session.email.clone(),
    })
}

/// 读取主会话并保证 access token 有效（必要时用 refresh token 刷新）。
pub(crate) async fn ensure_master_session(
    manager: &ServerManager,
) -> Result<MasterSession, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let Some(mut session) = load_session(&data_dir) else {
        return Err("请先在个人中心登录 DEX AI 主账号".to_string());
    };
    let client = http_client();
    let base = control_plane_url();
    match userinfo(client, &base, &session.access_token).await {
        Ok(_) => Ok(session),
        Err(error) if error == "登录状态已失效" => {
            let refreshed = refresh_session(client, &base, &session).await?;
            session = refreshed;
            save_session(&data_dir, &session)?;
            Ok(session)
        }
        Err(error) => Err(error),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MasterStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn dexai_master_register(
    invitation_code: String,
    email: String,
    password: String,
) -> Result<Value, String> {
    let email = email.trim().to_ascii_lowercase();
    let invitation_code = invitation_code.trim();
    if invitation_code.is_empty() {
        return Err("请填写邀请码".to_string());
    }
    if !email.contains('@') || !email.contains('.') {
        return Err("请输入有效的邮箱地址".to_string());
    }
    if password.len() < 12 {
        return Err("密码至少需要 12 个字符".to_string());
    }
    let client = http_client();
    let base = control_plane_url();
    let response = client
        .post(format!("{base}/v1/auth/register"))
        .json(&json!({
            "invitationCode": invitation_code,
            "email": email,
            "password": password,
        }))
        .send()
        .await
        .map_err(|error| format!("注册请求失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(Value::as_str)
            .or_else(|| body.get("error_description").and_then(Value::as_str))
            .unwrap_or("注册失败")
            .to_string());
    }
    // 保留服务端返回（开发环境可能带 developmentToken / token），由前端决定后续动作。
    Ok(body)
}

#[tauri::command]
pub async fn dexai_master_login(
    manager: State<'_, ServerManager>,
    email: String,
    password: String,
) -> Result<Value, String> {
    let email = email.trim().to_ascii_lowercase();
    if !email.contains('@') || password.is_empty() {
        return Err("请填写邮箱和密码".to_string());
    }
    let client = http_client();
    let base = control_plane_url();
    let response = client
        .post(format!("{base}/v1/oauth/token"))
        .form(&[
            ("grant_type", "password"),
            ("client_id", MASTER_CLIENT_ID),
            ("username", &email),
            ("password", &password),
            ("scope", MASTER_SCOPES),
        ])
        .send()
        .await
        .map_err(|error| format!("登录请求失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(body
            .get("error_description")
            .and_then(Value::as_str)
            .or_else(|| body.get("error").and_then(Value::as_str))
            .unwrap_or("登录失败")
            .to_string());
    }
    let info = userinfo(
        client,
        &base,
        body["access_token"].as_str().unwrap_or_default(),
    )
    .await
    .unwrap_or(Value::Null);
    let data_dir = manager.data_dir.lock().await.clone();
    let session = MasterSession {
        client_id: MASTER_CLIENT_ID.to_string(),
        access_token: body["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        refresh_token: body["refresh_token"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        expires_in: body["expires_in"].as_i64().unwrap_or(0),
        scope: body["scope"].as_str().unwrap_or_default().to_string(),
        account_id: info.get("sub").and_then(Value::as_str).map(str::to_string),
        email: info
            .get("email")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    save_session(&data_dir, &session)?;
    Ok(json!({
        "loggedIn": true,
        "email": session.email,
        "accountId": session.account_id,
        "scope": session.scope,
    }))
}

#[tauri::command]
pub async fn dexai_master_status(
    manager: State<'_, ServerManager>,
) -> Result<MasterStatus, String> {
    Ok(master_status_inner(&manager).await)
}

pub(crate) async fn master_status_inner(manager: &ServerManager) -> MasterStatus {
    let data_dir = manager.data_dir.lock().await.clone();
    let Some(session) = load_session(&data_dir) else {
        return MasterStatus {
            logged_in: false,
            email: None,
            account_id: None,
            scope: None,
            error: None,
        };
    };
    match ensure_master_session(manager).await {
        Ok(session) => MasterStatus {
            logged_in: true,
            email: session.email,
            account_id: session.account_id,
            scope: Some(session.scope),
            error: None,
        },
        Err(error) => MasterStatus {
            logged_in: false,
            email: session.email,
            account_id: session.account_id,
            scope: None,
            error: Some(error),
        },
    }
}

#[tauri::command]
pub async fn dexai_master_logout(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    if let Some(session) = load_session(&data_dir) {
        let _ = http_client()
            .post(format!("{}/v1/oauth/revoke", control_plane_url()))
            .json(&json!({ "token": session.refresh_token, "token_type_hint": "refresh_token" }))
            .send()
            .await;
    }
    clear_session(&data_dir);
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn dexai_oauth_authorize(
    manager: State<'_, ServerManager>,
    client_id: String,
    scope: String,
    redirect_uri: String,
    code_challenge: String,
    code_challenge_method: String,
) -> Result<Value, String> {
    let session = ensure_master_session(&manager).await?;
    let response = http_client()
        .post(format!("{}/v1/oauth/authorize", control_plane_url()))
        .bearer_auth(&session.access_token)
        .json(&json!({
            "clientId": client_id,
            "scope": scope,
            "redirectUri": redirect_uri,
            "codeChallenge": code_challenge,
            "codeChallengeMethod": code_challenge_method,
        }))
        .send()
        .await
        .map_err(|error| format!("发起授权失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(body
            .get("error_description")
            .and_then(Value::as_str)
            .or_else(|| body.get("error").and_then(Value::as_str))
            .unwrap_or("授权失败")
            .to_string());
    }
    Ok(body)
}

#[tauri::command]
pub async fn dexai_oauth_grants(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let session = ensure_master_session(&manager).await?;
    let response = http_client()
        .get(format!("{}/v1/oauth/grants", control_plane_url()))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("读取授权列表失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("读取授权列表失败")
            .to_string());
    }
    Ok(body)
}

#[tauri::command]
pub async fn dexai_oauth_revoke_grant(
    manager: State<'_, ServerManager>,
    grant_id: String,
) -> Result<Value, String> {
    let session = ensure_master_session(&manager).await?;
    let response = http_client()
        .post(format!(
            "{}/v1/oauth/grants/{grant_id}/revoke",
            control_plane_url()
        ))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("撤销授权失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("撤销授权失败: HTTP {}", response.status().as_u16()));
    }
    Ok(json!({ "ok": true }))
}

/// 读取当前主账号的授权审计记录（写入/刷新/撤销等动作）。
#[tauri::command]
pub async fn dexai_oauth_audit(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let session = ensure_master_session(&manager).await?;
    let response = http_client()
        .get(format!("{}/v1/oauth/audit", control_plane_url()))
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("读取授权记录失败: {error}"))?;
    let status = response.status();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("读取授权记录失败")
            .to_string());
    }
    Ok(body)
}

/// 通用插件 JSON-RPC 透传：个人中心授权闭环中把授权码/verifier 交给插件换取令牌。
#[tauri::command]
pub async fn invoke_plugin_rpc(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let pm = manager
        .plugin_manager
        .lock()
        .await
        .clone()
        .ok_or_else(|| "插件管理器未就绪".to_string())?;
    if !pm.is_enabled(&plugin_id).await {
        return Err(format!("插件 '{plugin_id}' 已停用"));
    }
    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id)
            .await
            .map_err(|error| error.to_string())?;
    }
    pm.send_request(&plugin_id, &method, params)
        .await
        .map_err(|error| error.to_string())
}

/// 读取已安装插件声明的 OAuth 授权（plugin.json `auth` 块），
/// 供个人中心生成通用授权入口（Phase 2 SDK 化）。
#[tauri::command]
pub async fn dexai_plugin_auth_manifest(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = manager
        .plugin_manager
        .lock()
        .await
        .clone()
        .ok_or_else(|| "插件管理器未就绪".to_string())?;
    let plugins = pm.list().await;
    let mut items = Vec::new();
    for plugin in plugins {
        let Some(auth) = plugin.auth else { continue };
        items.push(json!({
            "pluginId": plugin.id,
            "name": plugin.name,
            "version": plugin.version,
            "enabled": plugin.enabled,
            "clientId": auth.client_id,
            "scopes": auth.scopes,
            "reason": auth.reason,
            "redirectUri": auth.redirect_uri,
            "firstParty": auth.first_party,
            "methods": {
                "exchange": auth.methods.exchange,
                "status": auth.methods.status,
                "logout": auth.methods.logout,
            },
        }));
    }
    Ok(json!({ "plugins": items }))
}
