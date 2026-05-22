#![allow(dead_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OAuthProvider {
    Codex,
    Claude,
}

impl OAuthProvider {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "codex" => Ok(Self::Codex),
            "claude" | "anthropic" => Ok(Self::Claude),
            other => Err(anyhow!("不支持的 OAuth provider: {other}")),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PkceCodes {
    pub code_verifier: String,
    pub code_challenge: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthToken {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub email: String,
    pub account_id: String,
    pub expired: String,
    pub expired_at: u64,
    pub last_refresh: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexDeviceUserCode {
    pub device_auth_id: String,
    pub user_code: String,
    pub interval_secs: u64,
    pub verification_url: String,
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct CodexTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    id_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct ClaudeTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    account: ClaudeTokenAccount,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeTokenAccount {
    #[serde(default)]
    uuid: String,
    #[serde(default)]
    email_address: String,
}

const CODEX_AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CODEX_DEVICE_USER_CODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const CODEX_DEVICE_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
pub const CODEX_DEVICE_VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const CODEX_DEVICE_EXCHANGE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

const CLAUDE_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_REDIRECT_URI: &str = "http://localhost:54545/callback";

pub fn generate_pkce_codes() -> Result<PkceCodes> {
    let mut bytes = [0u8; 96];
    getrandom::getrandom(&mut bytes)?;
    let code_verifier = URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(digest);
    Ok(PkceCodes {
        code_verifier,
        code_challenge,
    })
}

pub fn generate_state() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub fn auth_url(provider: &OAuthProvider, state: &str, pkce: &PkceCodes) -> String {
    let mut params = Vec::<(&str, &str)>::new();
    match provider {
        OAuthProvider::Codex => {
            params.extend([
                ("client_id", CODEX_CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", CODEX_REDIRECT_URI),
                ("scope", "openid email profile offline_access"),
                ("state", state),
                ("code_challenge", pkce.code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("prompt", "login"),
                ("id_token_add_organizations", "true"),
                ("codex_cli_simplified_flow", "true"),
            ]);
            format!("{}?{}", CODEX_AUTH_URL, form_urlencoded(&params))
        }
        OAuthProvider::Claude => {
            params.extend([
                ("code", "true"),
                ("client_id", CLAUDE_CLIENT_ID),
                ("response_type", "code"),
                ("redirect_uri", CLAUDE_REDIRECT_URI),
                (
                    "scope",
                    "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload",
                ),
                ("code_challenge", pkce.code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", state),
            ]);
            format!("{}?{}", CLAUDE_AUTH_URL, form_urlencoded(&params))
        }
    }
}

pub async fn exchange_code(
    client: &Client,
    provider: &OAuthProvider,
    code: &str,
    state: &str,
    pkce: &PkceCodes,
) -> Result<OAuthToken> {
    match provider {
        OAuthProvider::Codex => exchange_codex_code(client, code, CODEX_REDIRECT_URI, pkce).await,
        OAuthProvider::Claude => exchange_claude_code(client, code, state, pkce).await,
    }
}

pub async fn exchange_codex_device_code(
    client: &Client,
    code: &str,
    code_verifier: &str,
    code_challenge: &str,
) -> Result<OAuthToken> {
    let pkce = PkceCodes {
        code_verifier: code_verifier.to_string(),
        code_challenge: code_challenge.to_string(),
    };
    exchange_codex_code(client, code, CODEX_DEVICE_EXCHANGE_REDIRECT_URI, &pkce).await
}

pub async fn refresh_token(
    client: &Client,
    provider: &OAuthProvider,
    refresh_token: &str,
) -> Result<OAuthToken> {
    match provider {
        OAuthProvider::Codex => refresh_codex_token(client, refresh_token).await,
        OAuthProvider::Claude => refresh_claude_token(client, refresh_token).await,
    }
}

async fn exchange_codex_code(
    client: &Client,
    code: &str,
    redirect_uri: &str,
    pkce: &PkceCodes,
) -> Result<OAuthToken> {
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", CODEX_CLIENT_ID),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("code_verifier", pkce.code_verifier.as_str()),
    ];
    let response = client
        .post(CODEX_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("Codex token exchange failed {status}: {body}"));
    }
    let parsed: CodexTokenResponse = serde_json::from_str(&body)?;
    Ok(codex_token_from_response(parsed))
}

async fn refresh_codex_token(client: &Client, refresh_token: &str) -> Result<OAuthToken> {
    let params = [
        ("client_id", CODEX_CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("scope", "openid profile email"),
    ];
    let response = client
        .post(CODEX_TOKEN_URL)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("Codex token refresh failed {status}: {body}"));
    }
    let parsed: CodexTokenResponse = serde_json::from_str(&body)?;
    Ok(codex_token_from_response(parsed))
}

async fn exchange_claude_code(
    client: &Client,
    code: &str,
    state: &str,
    pkce: &PkceCodes,
) -> Result<OAuthToken> {
    let (code, callback_state) = split_claude_code_state(code);
    let token_state = callback_state.unwrap_or(state);
    let body = json!({
        "code": code,
        "state": token_state,
        "grant_type": "authorization_code",
        "client_id": CLAUDE_CLIENT_ID,
        "redirect_uri": CLAUDE_REDIRECT_URI,
        "code_verifier": pkce.code_verifier,
    });
    let response = client
        .post(CLAUDE_TOKEN_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("Claude token exchange failed {status}: {body}"));
    }
    let parsed: ClaudeTokenResponse = serde_json::from_str(&body)?;
    Ok(claude_token_from_response(parsed))
}

async fn refresh_claude_token(client: &Client, refresh_token: &str) -> Result<OAuthToken> {
    let body = json!({
        "client_id": CLAUDE_CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    });
    let response = client
        .post(CLAUDE_TOKEN_URL)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("Claude token refresh failed {status}: {body}"));
    }
    let parsed: ClaudeTokenResponse = serde_json::from_str(&body)?;
    Ok(claude_token_from_response(parsed))
}

pub async fn request_codex_device_user_code(client: &Client) -> Result<CodexDeviceUserCode> {
    let response = client
        .post(CODEX_DEVICE_USER_CODE_URL)
        .header("Accept", "application/json")
        .json(&json!({ "client_id": CODEX_CLIENT_ID }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("Codex device user code failed {status}: {body}"));
    }
    let value: Value = serde_json::from_str(&body)?;
    let user_code = value
        .get("user_code")
        .or_else(|| value.get("usercode"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let device_auth_id = value
        .get("device_auth_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if user_code.is_empty() || device_auth_id.is_empty() {
        return Err(anyhow!(
            "Codex device response missing user_code/device_auth_id"
        ));
    }
    Ok(CodexDeviceUserCode {
        device_auth_id,
        user_code,
        interval_secs: parse_interval_secs(value.get("interval")).unwrap_or(5),
        verification_url: CODEX_DEVICE_VERIFICATION_URL.into(),
        expires_at: now_secs().saturating_add(15 * 60),
    })
}

pub async fn poll_codex_device_token(
    client: &Client,
    device_auth_id: &str,
    user_code: &str,
) -> Result<Option<(String, String, String)>> {
    let response = client
        .post(CODEX_DEVICE_TOKEN_URL)
        .header("Accept", "application/json")
        .json(&json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if status.as_u16() == 403 || status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(anyhow!("Codex device poll failed {status}: {body}"));
    }
    let value: Value = serde_json::from_str(&body)?;
    let code = value
        .get("authorization_code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let verifier = value
        .get("code_verifier")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let challenge = value
        .get("code_challenge")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if code.is_empty() || verifier.is_empty() || challenge.is_empty() {
        return Err(anyhow!(
            "Codex device poll response missing token exchange fields"
        ));
    }
    Ok(Some((
        code.to_string(),
        verifier.to_string(),
        challenge.to_string(),
    )))
}

pub fn oauth_token_to_value(token: &OAuthToken, login_mode: &str) -> Value {
    json!({
        "provider": token.provider,
        "login_mode": login_mode,
        "access_token": token.access_token,
        "refresh_token": token.refresh_token,
        "id_token": token.id_token,
        "email": token.email,
        "account_id": token.account_id,
        "expired": token.expired,
        "expired_at": token.expired_at,
        "last_refresh": token.last_refresh,
    })
}

pub fn oauth_token_from_value(value: &Value) -> Option<OAuthToken> {
    let obj = value.as_object()?;
    Some(OAuthToken {
        provider: obj.get("provider")?.as_str()?.to_string(),
        access_token: obj.get("access_token")?.as_str()?.to_string(),
        refresh_token: obj
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        id_token: obj
            .get("id_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        email: obj
            .get("email")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        account_id: obj
            .get("account_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        expired: obj
            .get("expired")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        expired_at: obj.get("expired_at").and_then(Value::as_u64).unwrap_or(0),
        last_refresh: obj
            .get("last_refresh")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    })
}

fn codex_token_from_response(response: CodexTokenResponse) -> OAuthToken {
    let claims = jwt_payload(&response.id_token);
    let email = claims
        .as_ref()
        .and_then(|v| v.get("email"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let account_id = claims
        .as_ref()
        .and_then(|v| {
            v.get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
                .or_else(|| v.get("chatgpt_account_id"))
                .or_else(|| v.get("account_id"))
                .or_else(|| v.get("sub"))
        })
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    token(
        "codex",
        response.access_token,
        response.refresh_token,
        response.id_token,
        email,
        account_id,
        response.expires_in,
    )
}

fn claude_token_from_response(response: ClaudeTokenResponse) -> OAuthToken {
    token(
        "claude",
        response.access_token,
        response.refresh_token,
        String::new(),
        response.account.email_address,
        response.account.uuid,
        response.expires_in,
    )
}

fn token(
    provider: &str,
    access_token: String,
    refresh_token: String,
    id_token: String,
    email: String,
    account_id: String,
    expires_in: u64,
) -> OAuthToken {
    let now = SystemTime::now();
    let expires_in = if expires_in == 0 { 3600 } else { expires_in };
    let expired_at = now_secs().saturating_add(expires_in);
    OAuthToken {
        provider: provider.into(),
        access_token,
        refresh_token,
        id_token,
        email,
        account_id,
        expired: rfc3339_utc(now + Duration::from_secs(expires_in)),
        expired_at,
        last_refresh: rfc3339_utc(now),
    }
}

fn jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload.as_bytes()).ok()?;
    serde_json::from_slice(&decoded).ok()
}

pub fn codex_id_token_info(token: &str) -> Value {
    let Some(claims) = jwt_payload(token) else {
        return json!({});
    };
    let auth = claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object);
    let pick_str = |key: &str| -> Option<String> {
        auth.and_then(|auth| auth.get(key))
            .or_else(|| claims.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    json!({
        "email": claims.get("email").and_then(Value::as_str).unwrap_or(""),
        "chatgpt_account_id": pick_str("chatgpt_account_id")
            .or_else(|| pick_str("account_id"))
            .unwrap_or_default(),
        "plan_type": pick_str("chatgpt_plan_type").unwrap_or_default(),
        "chatgpt_subscription_active_start": auth
            .and_then(|auth| auth.get("chatgpt_subscription_active_start"))
            .or_else(|| claims.get("chatgpt_subscription_active_start"))
            .cloned()
            .unwrap_or(Value::Null),
        "chatgpt_subscription_active_until": auth
            .and_then(|auth| auth.get("chatgpt_subscription_active_until"))
            .or_else(|| claims.get("chatgpt_subscription_active_until"))
            .cloned()
            .unwrap_or(Value::Null),
    })
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn rfc3339_utc(time: SystemTime) -> String {
    time::OffsetDateTime::from(time)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| now_secs().to_string())
}

fn form_urlencoded(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                percent_encode(key.as_bytes()),
                percent_encode(value.as_bytes())
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn percent_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &byte in bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn split_claude_code_state(code: &str) -> (&str, Option<&str>) {
    code.split_once('#')
        .map(|(code, state)| (code, Some(state)))
        .unwrap_or((code, None))
}

fn parse_interval_secs(value: Option<&Value>) -> Option<u64> {
    match value? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_uses_128_char_verifier_and_sha256_challenge() {
        let pkce = generate_pkce_codes().unwrap();
        assert_eq!(pkce.code_verifier.len(), 128);
        assert!(!pkce.code_verifier.contains('='));
        assert!(!pkce.code_challenge.contains('='));
        let digest = Sha256::digest(pkce.code_verifier.as_bytes());
        assert_eq!(pkce.code_challenge, URL_SAFE_NO_PAD.encode(digest));
    }

    #[test]
    fn codex_auth_url_contains_required_flags() {
        let pkce = PkceCodes {
            code_verifier: "verifier".into(),
            code_challenge: "challenge".into(),
        };
        let url = auth_url(&OAuthProvider::Codex, "state1", &pkce);
        assert!(url.starts_with(CODEX_AUTH_URL));
        assert!(url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("id_token_add_organizations=true"));
    }

    #[test]
    fn claude_auth_url_contains_oauth_scope() {
        let pkce = PkceCodes {
            code_verifier: "verifier".into(),
            code_challenge: "challenge".into(),
        };
        let url = auth_url(&OAuthProvider::Claude, "state1", &pkce);
        assert!(url.starts_with(CLAUDE_AUTH_URL));
        assert!(url.contains("client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e"));
        assert!(url.contains("user%3Ainference"));
        assert!(url.contains("user%3Asessions%3Aclaude_code"));
    }
}
