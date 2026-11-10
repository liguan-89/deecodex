// DEX AI 个人中心扩展：
//   1. 账号云同步（配置保险箱）：accounts.json + config.json 端到端加密后上传到
//      control-plane，云端只存密文；换电脑登录主账号后可一键恢复。
//   2. 偏好设置（跟随账号）：主题/语言/通知等轻量偏好随主账号跨设备同步。
//
// 加密方案：云同步密码 -> Argon2id 派生 256 位密钥 -> AES-256-GCM。
// 密码只在本机内存中存在，云端不存密码与明文。

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

use super::dex_account::{
    control_plane_url, ensure_master_session, http_client, master_status_inner,
};

const VAULT_KIND: &str = "deecodex.account-vault";
const VAULT_PLAIN_VERSION: i32 = 1;
const VAULT_SALT_LEN: usize = 16;
const VAULT_NONCE_LEN: usize = 12;
const CLOUD_STATE_FILE: &str = "dexai-cloud-state.json";

// ── 本地云同步状态 ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudState {
    enabled: bool,
    last_sync_at: Option<i64>,
    last_restore_at: Option<i64>,
    vault_version: i32,
    vault_updated_at: Option<i64>,
    vault_salt: Option<String>,
    prefs_updated_at: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudStatus {
    pub logged_in: bool,
    pub has_vault_scope: bool,
    pub enabled: bool,
    pub last_sync_at: Option<i64>,
    pub last_restore_at: Option<i64>,
    pub vault_version: i32,
    pub vault_updated_at: Option<i64>,
    pub prefs_updated_at: Option<i64>,
    pub error: Option<String>,
}

fn cloud_state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CLOUD_STATE_FILE)
}

fn load_cloud_state(data_dir: &Path) -> CloudState {
    std::fs::read_to_string(cloud_state_path(data_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_cloud_state(data_dir: &Path, state: &CloudState) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    std::fs::write(cloud_state_path(data_dir), raw)
        .map_err(|error| format!("保存云同步状态失败: {error}"))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 解析密钥材料：优先复用本地已保存的规范 salt（保证云同步密码跨会话稳定），
/// 首次开启时生成新 salt。
fn resolve_key_material(
    state: &CloudState,
    passphrase: &str,
) -> Result<(Vec<u8>, [u8; 32]), String> {
    match &state.vault_salt {
        Some(salt_b64) => {
            let salt = B64
                .decode(salt_b64)
                .map_err(|_| "本地同步状态损坏".to_string())?;
            let key = derive_key(passphrase, &salt)?;
            Ok((salt, key))
        }
        None => {
            let mut salt = [0u8; VAULT_SALT_LEN];
            rand::rngs::OsRng.fill_bytes(&mut salt);
            let key = derive_key(passphrase, &salt)?;
            Ok((salt.to_vec(), key))
        }
    }
}

/// 若云端已存在保险箱，用给定密码校验能否解密；校验失败返回错误，
/// 避免用错误密码重新加密导致原密码失效。
/// 返回云端是否已存在保险箱（404 表示不存在）。
async fn verify_vault_passphrase(
    manager: &ServerManager,
    passphrase: &str,
) -> Result<bool, String> {
    let (status, body) = oauth_get(manager, "/v1/account/vault").await?;
    if status == 404 {
        return Ok(false);
    }
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("读取云端保险箱失败")
                .to_string()
        }));
    }
    let payload = body
        .get("payload")
        .and_then(Value::as_str)
        .ok_or("云端数据缺少 payload".to_string())?;
    if decrypt_vault(passphrase, payload).is_err() {
        return Err(
            "云同步密码与云端数据不匹配（如忘记密码，请先清除云端副本再重新开启）".to_string(),
        );
    }
    Ok(true)
}

// ── 加密：云同步密码 -> Argon2id -> AES-256-GCM ────────────────

#[derive(Serialize, Deserialize)]
struct VaultEnvelope {
    kind: String,
    version: i32,
    created_at: i64,
    salt: String,
    nonce: String,
    ciphertext: String,
}

type VaultKeyMaterial = (Vec<u8>, [u8; 32]);

static VAULT_KEY_CACHE: OnceLock<Mutex<Option<VaultKeyMaterial>>> = OnceLock::new();

fn vault_key_cache() -> &'static Mutex<Option<VaultKeyMaterial>> {
    VAULT_KEY_CACHE.get_or_init(|| Mutex::new(None))
}

fn set_cached_key(salt: Vec<u8>, key: [u8; 32]) {
    if let Ok(mut slot) = vault_key_cache().lock() {
        *slot = Some((salt, key));
    }
}

fn clear_cached_key() {
    if let Ok(mut slot) = vault_key_cache().lock() {
        *slot = None;
    }
}

fn cached_key_material() -> Option<VaultKeyMaterial> {
    vault_key_cache().lock().ok().and_then(|slot| slot.clone())
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    let params = Params::new(19_456, 2, 1, Some(32)).map_err(|error| error.to_string())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| format!("密钥派生失败: {error}"))?;
    Ok(key)
}

fn encrypt_with_material(
    salt: &[u8],
    key: &[u8; 32],
    plaintext: &[u8],
) -> Result<(String, [u8; 32]), String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|error| error.to_string())?;
    let mut nonce_bytes = [0u8; VAULT_NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| "保险箱加密失败".to_string())?;
    let envelope = VaultEnvelope {
        kind: VAULT_KIND.to_string(),
        version: VAULT_PLAIN_VERSION,
        created_at: now_unix(),
        salt: B64.encode(salt),
        nonce: B64.encode(nonce_bytes),
        ciphertext: B64.encode(ciphertext),
    };
    let payload = serde_json::to_string(&envelope).map_err(|error| error.to_string())?;
    Ok((payload, *key))
}

fn decrypt_vault(passphrase: &str, payload: &str) -> Result<(Value, Vec<u8>), String> {
    let envelope: VaultEnvelope =
        serde_json::from_str(payload).map_err(|error| format!("保险箱数据损坏: {error}"))?;
    if envelope.kind != VAULT_KIND {
        return Err("保险箱类型不匹配".to_string());
    }
    let salt = B64
        .decode(&envelope.salt)
        .map_err(|_| "保险箱数据损坏".to_string())?;
    let nonce = B64
        .decode(&envelope.nonce)
        .map_err(|_| "保险箱数据损坏".to_string())?;
    let ciphertext = B64
        .decode(&envelope.ciphertext)
        .map_err(|_| "保险箱数据损坏".to_string())?;
    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|error| error.to_string())?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| "云同步密码错误或保险箱数据被篡改".to_string())?;
    let value = serde_json::from_slice(&plain).map_err(|error| error.to_string())?;
    Ok((value, salt))
}

// ── 保险箱内容组装 / 应用 ──────────────────────────────────────

fn build_vault_payload(data_dir: &Path) -> Result<(String, i32), String> {
    let accounts_raw = std::fs::read_to_string(data_dir.join("accounts.json"))
        .map_err(|error| format!("读取 accounts.json 失败: {error}"))?;
    let config_raw = std::fs::read_to_string(data_dir.join("config.json"))
        .map_err(|error| format!("读取 config.json 失败: {error}"))?;
    let accounts: Value = serde_json::from_str(&accounts_raw)
        .map_err(|error| format!("accounts.json 不是合法 JSON: {error}"))?;
    let config: Value = serde_json::from_str(&config_raw)
        .map_err(|error| format!("config.json 不是合法 JSON: {error}"))?;
    let plain = json!({
        "app": "deecodex",
        "exportedAt": now_unix(),
        "vault": {
            "accounts": accounts,
            "config": config,
        }
    });
    let plaintext = serde_json::to_string(&plain).map_err(|error| error.to_string())?;
    Ok((plaintext, VAULT_PLAIN_VERSION))
}

fn apply_vault_payload(data_dir: &Path, plain: &Value) -> Result<(), String> {
    let vault = plain
        .get("vault")
        .ok_or("保险箱缺少 vault 数据".to_string())?;
    let accounts = vault
        .get("accounts")
        .ok_or("保险箱缺少账号数据".to_string())?;
    let config = vault
        .get("config")
        .ok_or("保险箱缺少配置数据".to_string())?;
    let accounts_raw = serde_json::to_string_pretty(accounts).map_err(|error| error.to_string())?;
    let config_raw = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
    let backups = data_dir.join("backups");
    std::fs::create_dir_all(&backups).map_err(|error| format!("创建备份目录失败: {error}"))?;
    let stamp = now_unix();
    for (name, content) in [
        ("accounts.json", &accounts_raw),
        ("config.json", &config_raw),
    ] {
        let path = data_dir.join(name);
        if path.exists() {
            let backup = backups.join(format!("{name}.cloud-restore-{stamp}"));
            std::fs::copy(&path, &backup).map_err(|error| format!("备份 {name} 失败: {error}"))?;
        }
        std::fs::write(&path, content).map_err(|error| format!("写入 {name} 失败: {error}"))?;
    }
    Ok(())
}

// ── control-plane HTTP ─────────────────────────────────────────

async fn oauth_get(manager: &ServerManager, path: &str) -> Result<(u16, Value), String> {
    let session = ensure_master_session(manager).await?;
    let client = http_client();
    let url = format!("{}{}", control_plane_url(), path);
    let response = client
        .get(&url)
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("请求控制面失败: {error}"))?;
    let status = response.status().as_u16();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, body))
}

async fn oauth_put(
    manager: &ServerManager,
    path: &str,
    payload: &Value,
) -> Result<(u16, Value), String> {
    let session = ensure_master_session(manager).await?;
    let client = http_client();
    let url = format!("{}{}", control_plane_url(), path);
    let response = client
        .put(&url)
        .bearer_auth(&session.access_token)
        .json(payload)
        .send()
        .await
        .map_err(|error| format!("请求控制面失败: {error}"))?;
    let status = response.status().as_u16();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, body))
}

async fn oauth_delete(manager: &ServerManager, path: &str) -> Result<u16, String> {
    let session = ensure_master_session(manager).await?;
    let client = http_client();
    let url = format!("{}{}", control_plane_url(), path);
    let response = client
        .delete(&url)
        .bearer_auth(&session.access_token)
        .send()
        .await
        .map_err(|error| format!("请求控制面失败: {error}"))?;
    Ok(response.status().as_u16())
}

/// 读取云端保险箱的 updatedAt；云端不存在（404）返回 None。
async fn cloud_vault_updated_at(manager: &ServerManager) -> Result<Option<i64>, String> {
    let (status, body) = oauth_get(manager, "/v1/account/vault").await?;
    if status == 404 {
        return Ok(None);
    }
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("读取云端保险箱失败")
                .to_string()
        }));
    }
    Ok(body.get("updatedAt").and_then(Value::as_i64))
}

/// 云端数据是否比本机更新（本机从未同步过且云端存在 → 视为云端更新）。
fn cloud_is_newer(cloud_updated_at: Option<i64>, local_updated_at: Option<i64>) -> bool {
    match (cloud_updated_at, local_updated_at) {
        (Some(cloud), Some(local)) => cloud > local,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// 同步时是否应阻止 push。云端比本机新时阻止；但本机时间戳未知（服务端可能
/// 未返回 updatedAt）且本机已建立过保险箱（有规范 salt）时，无法证明云端更旧，
/// 放行 push 避免自我锁死；全新机器（无 salt）仍按云端更旧拦截。
fn sync_push_blocked(
    cloud_updated_at: Option<i64>,
    local_updated_at: Option<i64>,
    local_has_setup: bool,
) -> bool {
    if !cloud_is_newer(cloud_updated_at, local_updated_at) {
        return false;
    }
    local_updated_at.is_some() || !local_has_setup
}

/// 本机从未在这台电脑建立过保险箱（无规范 salt）时，若云端已有数据，
/// 直接开启会静默覆盖云端账号配置，应阻止并引导先「从云端恢复」。
fn enable_would_clobber_cloud(cloud_exists: bool, local_has_setup: bool) -> bool {
    cloud_exists && !local_has_setup
}

fn scope_error(status: u16) -> Option<String> {
    if status == 403 {
        Some("当前主账号会话缺少云同步权限，请退出后重新登录 DEX AI 主账号".to_string())
    } else {
        None
    }
}

fn cloud_status_inner(state: &CloudState) -> CloudStatus {
    CloudStatus {
        logged_in: false,
        has_vault_scope: false,
        enabled: state.enabled,
        last_sync_at: state.last_sync_at,
        last_restore_at: state.last_restore_at,
        vault_version: state.vault_version,
        vault_updated_at: state.vault_updated_at,
        prefs_updated_at: state.prefs_updated_at,
        error: None,
    }
}

// ── 命令：状态 ─────────────────────────────────────────────────

#[tauri::command]
pub async fn dexai_cloud_status(manager: State<'_, ServerManager>) -> Result<CloudStatus, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let state = load_cloud_state(&data_dir);
    let mut status = cloud_status_inner(&state);
    let master = master_status_inner(&manager).await;
    status.logged_in = master.logged_in;
    status.error = master.error;
    if let Some(scope) = master.scope {
        status.has_vault_scope = scope.split_whitespace().any(|s| s == "dexai:vault");
    }
    Ok(status)
}

// ── 命令：云同步开关与操作 ─────────────────────────────────────

#[tauri::command]
pub async fn dexai_vault_enable(
    manager: State<'_, ServerManager>,
    passphrase: String,
) -> Result<CloudStatus, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    if passphrase.len() < 8 {
        return Err("云同步密码至少 8 位".to_string());
    }
    let mut state = load_cloud_state(&data_dir);
    let cloud_exists = verify_vault_passphrase(&manager, &passphrase).await?;
    // 全新电脑（无本地 salt）直接开启会覆盖云端已有数据，改为引导恢复。
    if enable_would_clobber_cloud(cloud_exists, state.vault_salt.is_some()) {
        return Err(
            "云端已存在账号数据。首次在这台电脑开启云同步请使用「从云端恢复」；如需用本机配置覆盖云端，请先「清除云端副本」再开启"
                .to_string(),
        );
    }
    let (salt, key) = resolve_key_material(&state, &passphrase)?;
    let (plaintext, version) = build_vault_payload(&data_dir)?;
    let (payload, key) = encrypt_with_material(&salt, &key, plaintext.as_bytes())?;
    let (status, body) = oauth_put(
        &manager,
        "/v1/account/vault",
        &json!({ "version": version, "payload": payload }),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("上传失败")
                .to_string()
        }));
    }
    set_cached_key(salt.clone(), key);
    state.enabled = true;
    state.last_sync_at = Some(now_unix());
    state.vault_version = version;
    state.vault_updated_at = body.get("updatedAt").and_then(Value::as_i64);
    state.vault_salt = Some(B64.encode(&salt));
    save_cloud_state(&data_dir, &state)?;
    Ok(cloud_status_inner(&state))
}

#[tauri::command]
pub async fn dexai_vault_sync(
    manager: State<'_, ServerManager>,
    passphrase: Option<String>,
) -> Result<CloudStatus, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let mut state = load_cloud_state(&data_dir);
    // 防覆盖：云端数据比本机新时不允许 push，避免静默丢掉另一台设备/更早会话的改动。
    let cloud_updated_at = cloud_vault_updated_at(&manager).await?;
    if sync_push_blocked(
        cloud_updated_at,
        state.vault_updated_at,
        state.vault_salt.is_some(),
    ) {
        return Err(
            "云端账号数据比本机新，请先「从云端恢复」；如确认以本机为准，请先「清除云端副本」再同步"
                .to_string(),
        );
    }
    let (plaintext, version) = build_vault_payload(&data_dir)?;
    let (salt, key) = match cached_key_material() {
        Some(material) => material,
        None => {
            let pass = passphrase
                .filter(|p| !p.is_empty())
                .ok_or("请先输入云同步密码以启用同步".to_string())?;
            if pass.len() < 8 {
                return Err("云同步密码至少 8 位".to_string());
            }
            verify_vault_passphrase(&manager, &pass).await?;
            resolve_key_material(&state, &pass)?
        }
    };
    let (payload, key) = encrypt_with_material(&salt, &key, plaintext.as_bytes())?;
    let (status, body) = oauth_put(
        &manager,
        "/v1/account/vault",
        &json!({ "version": version, "payload": payload }),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("同步失败")
                .to_string()
        }));
    }
    set_cached_key(salt.clone(), key);
    state.enabled = true;
    state.last_sync_at = Some(now_unix());
    state.vault_version = version;
    state.vault_updated_at = body.get("updatedAt").and_then(Value::as_i64);
    state.vault_salt = Some(B64.encode(&salt));
    save_cloud_state(&data_dir, &state)?;
    Ok(cloud_status_inner(&state))
}

#[tauri::command]
pub async fn dexai_vault_restore(
    manager: State<'_, ServerManager>,
    passphrase: String,
) -> Result<CloudStatus, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let (status, body) = oauth_get(&manager, "/v1/account/vault").await?;
    if status == 404 {
        return Err("云端没有可恢复的账号数据".to_string());
    }
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("获取云端数据失败")
                .to_string()
        }));
    }
    let payload = body
        .get("payload")
        .and_then(Value::as_str)
        .ok_or("云端数据缺少 payload".to_string())?;
    let (plain, salt) = decrypt_vault(&passphrase, payload)?;
    let key = derive_key(&passphrase, &salt)?;
    apply_vault_payload(&data_dir, &plain)?;
    set_cached_key(salt.clone(), key);
    let mut state = load_cloud_state(&data_dir);
    state.enabled = true;
    state.last_restore_at = Some(now_unix());
    state.vault_version = body.get("version").and_then(Value::as_i64).unwrap_or(0) as i32;
    state.vault_updated_at = body.get("updatedAt").and_then(Value::as_i64);
    state.vault_salt = Some(B64.encode(&salt));
    save_cloud_state(&data_dir, &state)?;
    Ok(cloud_status_inner(&state))
}

#[tauri::command]
pub async fn dexai_vault_disable(
    manager: State<'_, ServerManager>,
    clear_remote: bool,
) -> Result<CloudStatus, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    if clear_remote {
        let status = oauth_delete(&manager, "/v1/account/vault").await?;
        if !(200..300).contains(&status) && status != 404 {
            return Err(format!("清除云端数据失败（HTTP {status}）"));
        }
    }
    clear_cached_key();
    let mut state = load_cloud_state(&data_dir);
    state.enabled = false;
    state.last_sync_at = None;
    state.vault_version = 0;
    state.vault_updated_at = None;
    save_cloud_state(&data_dir, &state)?;
    Ok(cloud_status_inner(&state))
}

#[tauri::command]
pub async fn dexai_vault_clear(manager: State<'_, ServerManager>) -> Result<CloudStatus, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let status = oauth_delete(&manager, "/v1/account/vault").await?;
    if !(200..300).contains(&status) && status != 404 {
        return Err(format!("清除云端数据失败（HTTP {status}）"));
    }
    clear_cached_key();
    let mut state = load_cloud_state(&data_dir);
    state.enabled = false;
    state.last_sync_at = None;
    state.last_restore_at = None;
    state.vault_version = 0;
    state.vault_updated_at = None;
    save_cloud_state(&data_dir, &state)?;
    Ok(cloud_status_inner(&state))
}

// ── 命令：偏好设置（跟随账号） ─────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrefsResult {
    pub prefs: Value,
    pub updated_at: Option<i64>,
}

#[tauri::command]
pub async fn dexai_prefs_get(manager: State<'_, ServerManager>) -> Result<PrefsResult, String> {
    let (status, body) = oauth_get(&manager, "/v1/account/preferences").await?;
    if status == 404 {
        return Ok(PrefsResult {
            prefs: json!({}),
            updated_at: None,
        });
    }
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("获取偏好设置失败")
                .to_string()
        }));
    }
    Ok(PrefsResult {
        prefs: body.get("prefs").cloned().unwrap_or_else(|| json!({})),
        updated_at: body.get("updatedAt").and_then(Value::as_i64),
    })
}

#[tauri::command]
pub async fn dexai_prefs_set(
    manager: State<'_, ServerManager>,
    prefs: Value,
) -> Result<PrefsResult, String> {
    if !prefs.is_object() {
        return Err("prefs 必须是 JSON 对象".to_string());
    }
    let (status, body) = oauth_put(
        &manager,
        "/v1/account/preferences",
        &json!({ "prefs": prefs }),
    )
    .await?;
    if !(200..300).contains(&status) {
        return Err(scope_error(status).unwrap_or_else(|| {
            body.get("error")
                .and_then(Value::as_str)
                .unwrap_or("保存偏好设置失败")
                .to_string()
        }));
    }
    let data_dir = manager.data_dir.lock().await.clone();
    let mut state = load_cloud_state(&data_dir);
    state.prefs_updated_at = body.get("updatedAt").and_then(Value::as_i64);
    let _ = save_cloud_state(&data_dir, &state);
    Ok(PrefsResult {
        prefs: body.get("prefs").cloned().unwrap_or(prefs),
        updated_at: state.prefs_updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_encrypt_decrypt_round_trip() {
        let plaintext = br#"{"vault":{"accounts":{"version":3},"config":{"port":8899}}}"#;
        let state = CloudState::default();
        let (salt, key) = resolve_key_material(&state, "correct-horse-battery").unwrap();
        let (payload, key2) = encrypt_with_material(&salt, &key, plaintext).unwrap();
        assert_eq!(key2, key);
        assert!(!payload.contains("8899"), "密文不得包含明文");
        let (decoded, decoded_salt) = decrypt_vault("correct-horse-battery", &payload).unwrap();
        assert_eq!(decoded["vault"]["config"]["port"], 8899);
        assert_eq!(decoded_salt, salt);

        // 同一规范 salt 重新派生 -> 同一密钥（跨会话/跨设备稳定）
        let mut state2 = CloudState::default();
        state2.vault_salt = Some(B64.encode(&salt));
        let (salt2, key3) = resolve_key_material(&state2, "correct-horse-battery").unwrap();
        assert_eq!(salt2, salt);
        assert_eq!(key3, key);
    }

    #[test]
    fn vault_wrong_passphrase_rejected() {
        let state = CloudState::default();
        let (salt, key) = resolve_key_material(&state, "right-password").unwrap();
        let (payload, _) = encrypt_with_material(&salt, &key, b"secret-config").unwrap();
        let error = decrypt_vault("wrong-password", &payload).unwrap_err();
        assert!(error.contains("密码错误") || error.contains("篡改"));
    }

    #[test]
    fn vault_tampered_payload_rejected() {
        let state = CloudState::default();
        let (salt, key) = resolve_key_material(&state, "right-password").unwrap();
        let (payload, _) = encrypt_with_material(&salt, &key, b"secret-config").unwrap();
        let mut envelope: VaultEnvelope = serde_json::from_str(&payload).unwrap();
        envelope.ciphertext.push('A');
        let tampered = serde_json::to_string(&envelope).unwrap();
        assert!(decrypt_vault("right-password", &tampered).is_err());
    }

    #[test]
    fn cloud_is_newer_guards_against_overwriting_newer_cloud_data() {
        // 本机比云端新 → 允许 push
        assert!(!cloud_is_newer(Some(100), Some(200)));
        // 时间戳相等 → 允许 push
        assert!(!cloud_is_newer(Some(200), Some(200)));
        // 云端比本机新 → 阻止 push
        assert!(cloud_is_newer(Some(300), Some(200)));
        // 本机从未同步过但云端存在 → 视为云端更新，阻止 push
        assert!(cloud_is_newer(Some(100), None));
        // 云端不存在 → 允许 push
        assert!(!cloud_is_newer(None, Some(100)));
        assert!(!cloud_is_newer(None, None));
    }

    #[test]
    fn sync_push_blocked_allows_unknown_local_timestamp_on_established_machine() {
        // 云端不存在 → 允许
        assert!(!sync_push_blocked(None, Some(100), false));
        assert!(!sync_push_blocked(None, None, false));
        // 本机时间戳已知且不更旧 → 允许
        assert!(!sync_push_blocked(Some(100), Some(200), false));
        assert!(!sync_push_blocked(Some(200), Some(200), false));
        // 云端明确比本机新 → 阻止（无论是否已建立保险箱）
        assert!(sync_push_blocked(Some(300), Some(200), true));
        assert!(sync_push_blocked(Some(300), Some(200), false));
        // 本机时间戳未知：已建立保险箱（服务端未返回 updatedAt）→ 放行，避免自我锁死
        assert!(!sync_push_blocked(Some(100), None, true));
        // 本机时间戳未知且为全新机器 → 按云端更旧拦截
        assert!(sync_push_blocked(Some(100), None, false));
    }

    #[test]
    fn enable_would_clobber_cloud_only_on_fresh_machine() {
        // 全新电脑（无本地 salt）+ 云端已有数据 → 阻止开启，防覆盖
        assert!(enable_would_clobber_cloud(true, false));
        // 本机已建立过保险箱（disable 后再开）→ 允许
        assert!(!enable_would_clobber_cloud(true, true));
        // 云端无数据 → 允许
        assert!(!enable_would_clobber_cloud(false, false));
        assert!(!enable_would_clobber_cloud(false, true));
    }
}
