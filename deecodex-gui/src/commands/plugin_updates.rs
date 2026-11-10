use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use deecodex_plugin_host::{PluginManager, PluginManifest};
use minisign_verify::{PublicKey, Signature};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::ServerManager;

use super::plugins::{get_pm, plugin_version_parts, plugin_version_satisfies};

const MOBILE_CODEX_PLUGIN_ID: &str = "deecodex-mobile-codex";
const DESKTOP_SYNC_PLUGIN_ID: &str = "deecodex-desktop-sync";
const DEBUG_PLUGIN_ID: &str = "dexai-debug";
const MOBILE_CODEX_UPDATE_ENDPOINT: &str =
    "https://api.liguan.me/releases/dex-ai/plugins/mobile-codex/latest.json";
const DESKTOP_SYNC_UPDATE_ENDPOINT: &str =
    "https://api.liguan.me/releases/dex-ai/plugins/desktop-sync/latest.json";
const DEBUG_UPDATE_ENDPOINT: &str =
    "https://api.liguan.me/releases/dex-ai/plugins/dexai-debug/latest.json";

#[derive(Clone, Copy)]
struct FirstPartyPluginUpdateConfig {
    plugin_id: &'static str,
    display_name: &'static str,
    endpoint: &'static str,
    path_prefix: &'static str,
}

const MOBILE_CODEX_UPDATE: FirstPartyPluginUpdateConfig = FirstPartyPluginUpdateConfig {
    plugin_id: MOBILE_CODEX_PLUGIN_ID,
    display_name: "Mobile Codex",
    endpoint: MOBILE_CODEX_UPDATE_ENDPOINT,
    path_prefix: "/releases/dex-ai/plugins/mobile-codex/",
};
const DESKTOP_SYNC_UPDATE: FirstPartyPluginUpdateConfig = FirstPartyPluginUpdateConfig {
    plugin_id: DESKTOP_SYNC_PLUGIN_ID,
    display_name: "Desktop Sync",
    endpoint: DESKTOP_SYNC_UPDATE_ENDPOINT,
    path_prefix: "/releases/dex-ai/plugins/desktop-sync/",
};
const DEBUG_UPDATE: FirstPartyPluginUpdateConfig = FirstPartyPluginUpdateConfig {
    plugin_id: DEBUG_PLUGIN_ID,
    display_name: "DEXAI Debug",
    endpoint: DEBUG_UPDATE_ENDPOINT,
    path_prefix: "/releases/dex-ai/plugins/dexai-debug/",
};
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PLUGIN_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginReleaseManifest {
    schema_version: u32,
    plugin_id: String,
    version: String,
    #[serde(default)]
    min_deecodex_version: String,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    release_notes: Vec<String>,
    platforms: HashMap<String, PluginReleasePlatform>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginReleasePlatform {
    url: String,
    sha256: String,
    size_bytes: u64,
    signature: String,
}

#[derive(Debug, Clone)]
struct PluginUpdatePlan {
    release: PluginReleaseManifest,
    platform_key: String,
    platform: PluginReleasePlatform,
    installed_version: Option<String>,
    update_available: bool,
}

fn update_endpoint(config: FirstPartyPluginUpdateConfig) -> &'static str {
    if cfg!(debug_assertions) {
        let override_endpoint = match config.plugin_id {
            MOBILE_CODEX_PLUGIN_ID => option_env!("DEECODEX_MOBILE_CODEX_UPDATE_ENDPOINT"),
            DESKTOP_SYNC_PLUGIN_ID => option_env!("DEECODEX_DESKTOP_SYNC_UPDATE_ENDPOINT"),
            _ => None,
        };
        if let Some(value) = override_endpoint.filter(|value| !value.trim().is_empty()) {
            return value;
        }
    }
    config.endpoint
}

fn current_platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        other => other,
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn validate_release_url(config: FirstPartyPluginUpdateConfig, raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|error| format!("插件更新 URL 无效: {error}"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("api.liguan.me")
        || !url.path().starts_with(config.path_prefix)
    {
        return Err("插件更新 URL 必须使用受信任的 DEX AI HTTPS 发布源".into());
    }
    Ok(url)
}

fn updater_public_key() -> Result<String, String> {
    let config: Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
        .map_err(|error| format!("无法读取 DEX AI 更新公钥配置: {error}"))?;
    config
        .pointer("/plugins/updater/pubkey")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "DEX AI 更新公钥配置缺失".to_string())
}

fn verify_release_signature(bytes: &[u8], encoded_signature: &str) -> Result<(), String> {
    let decoded_key = base64::engine::general_purpose::STANDARD
        .decode(updater_public_key()?)
        .map_err(|error| format!("DEX AI 更新公钥编码无效: {error}"))?;
    let decoded_key =
        std::str::from_utf8(&decoded_key).map_err(|_| "DEX AI 更新公钥不是 UTF-8".to_string())?;
    let public_key =
        PublicKey::decode(decoded_key).map_err(|error| format!("DEX AI 更新公钥无效: {error}"))?;

    let decoded_signature = base64::engine::general_purpose::STANDARD
        .decode(encoded_signature.trim())
        .map_err(|error| format!("插件签名编码无效: {error}"))?;
    let decoded_signature =
        std::str::from_utf8(&decoded_signature).map_err(|_| "插件签名不是 UTF-8".to_string())?;
    let signature = Signature::decode(decoded_signature)
        .map_err(|error| format!("插件签名格式无效: {error}"))?;
    public_key
        .verify(bytes, &signature, true)
        .map_err(|error| format!("插件签名验证失败: {error}"))
}

fn validate_release_manifest(
    config: FirstPartyPluginUpdateConfig,
    manifest: &PluginReleaseManifest,
) -> Result<(), String> {
    if manifest.schema_version != 1 {
        return Err(format!(
            "不支持的插件更新清单版本: {}",
            manifest.schema_version
        ));
    }
    if manifest.plugin_id != config.plugin_id {
        return Err(format!("插件更新清单 ID 不匹配: {}", manifest.plugin_id));
    }
    if plugin_version_parts(&manifest.version).is_none() {
        return Err(format!("插件更新版本无效: {}", manifest.version));
    }
    if !manifest.min_deecodex_version.trim().is_empty()
        && !plugin_version_satisfies(env!("CARGO_PKG_VERSION"), &manifest.min_deecodex_version)
    {
        return Err(format!(
            "{} {} 需要 DEX AI {}+",
            config.display_name, manifest.version, manifest.min_deecodex_version
        ));
    }
    Ok(())
}

async fn fetch_update_plan(
    pm: &Arc<PluginManager>,
    config: FirstPartyPluginUpdateConfig,
) -> Result<PluginUpdatePlan, String> {
    let name = config.display_name;
    let client = reqwest::Client::builder()
        .user_agent(concat!("DEX-AI/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("无法创建插件更新客户端: {error}"))?;
    let endpoint = validate_release_url(config, update_endpoint(config))?;
    let response = client
        .get(endpoint)
        .send()
        .await
        .map_err(|error| format!("读取 {name} 插件更新失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("{name} 插件更新服务返回错误: {error}"))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MANIFEST_BYTES)
    {
        return Err(format!("{name} 插件更新清单过大"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 {name} 插件更新清单失败: {error}"))?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(format!("{name} 插件更新清单过大"));
    }
    let release: PluginReleaseManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{name} 插件更新清单格式错误: {error}"))?;
    validate_release_manifest(config, &release)?;

    let platform_key = current_platform_key();
    let platform = release
        .platforms
        .get(&platform_key)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Mobile Codex {} 暂未提供 {platform_key} 更新包",
                release.version
            )
        })?;
    validate_release_url(config, &platform.url)?;
    if platform.size_bytes == 0 || platform.size_bytes > MAX_PLUGIN_PACKAGE_BYTES {
        return Err(format!("{name} 插件更新包大小无效"));
    }
    if platform.sha256.len() != 64 || !platform.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("{name} 插件更新包 SHA-256 无效"));
    }
    if platform.signature.trim().is_empty() {
        return Err(format!("{name} 插件更新包缺少签名"));
    }

    let installed_version = installed_plugin_version(
        pm.list()
            .await
            .iter()
            .map(|plugin| (plugin.id.as_str(), plugin.version.as_str())),
        config.plugin_id,
    );
    let update_available = installed_version
        .as_deref()
        .and_then(plugin_version_parts)
        .zip(plugin_version_parts(&release.version))
        .is_none_or(|(installed, latest)| latest > installed);

    Ok(PluginUpdatePlan {
        release,
        platform_key,
        platform,
        installed_version,
        update_available,
    })
}

/// 返回指定插件 id 的已安装版本。必须按调用方配置的 `plugin_id` 查找，
/// 不能固定为 Mobile Codex，否则 Desktop Sync 会用 Mobile Codex 的版本
/// 错误判定“始终可更新”。
fn installed_plugin_version<'a>(
    plugins: impl IntoIterator<Item = (&'a str, &'a str)>,
    plugin_id: &str,
) -> Option<String> {
    plugins
        .into_iter()
        .find(|(id, _)| *id == plugin_id)
        .map(|(_, version)| version.to_string())
}

fn update_plan_value(plan: &PluginUpdatePlan) -> Value {
    json!({
        "pluginId": plan.release.plugin_id,
        "installedVersion": plan.installed_version,
        "latestVersion": plan.release.version,
        "minDeecodexVersion": plan.release.min_deecodex_version,
        "publishedAt": plan.release.published_at,
        "releaseNotes": plan.release.release_notes,
        "platform": plan.platform_key,
        "sizeBytes": plan.platform.size_bytes,
        "sha256": plan.platform.sha256,
        "updateAvailable": plan.update_available,
    })
}

async fn download_and_apply(
    pm: &Arc<PluginManager>,
    config: FirstPartyPluginUpdateConfig,
    plan: &PluginUpdatePlan,
    restart_after_update: bool,
) -> Result<PluginManifest, String> {
    let name = config.display_name;
    if !plan.update_available {
        return Err(format!("{name} 插件已经是最新版本"));
    }
    let url = validate_release_url(config, &plan.platform.url)?;
    let response = reqwest::Client::builder()
        .user_agent(concat!("DEX-AI/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("无法创建插件下载客户端: {error}"))?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("下载 Mobile Codex 插件失败: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Mobile Codex 插件下载服务返回错误: {error}"))?;
    if response
        .content_length()
        .is_some_and(|size| size != plan.platform.size_bytes)
    {
        return Err(format!("{name} 插件更新包 Content-Length 不匹配"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 {name} 插件更新包失败: {error}"))?;
    if bytes.len() as u64 != plan.platform.size_bytes {
        return Err(format!("{name} 插件更新包大小不匹配"));
    }
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    if !actual_sha.eq_ignore_ascii_case(&plan.platform.sha256) {
        return Err(format!(
            "{name} 插件更新包 SHA-256 不匹配：期望 {}，实际 {actual_sha}",
            plan.platform.sha256
        ));
    }
    verify_release_signature(&bytes, &plan.platform.signature)?;

    let mut package = tempfile::Builder::new()
        .prefix(&format!("{}-", config.plugin_id))
        .suffix(".zip")
        .tempfile()
        .map_err(|error| format!("无法创建插件更新临时文件: {error}"))?;
    package
        .write_all(&bytes)
        .and_then(|_| package.flush())
        .map_err(|error| format!("无法写入插件更新临时文件: {error}"))?;
    let preview = pm
        .preview_install(package.path())
        .await
        .map_err(|error| format!("{name} 插件包预检失败: {error}"))?;
    if preview.manifest.id != config.plugin_id
        || !preview.manifest.first_party
        || preview.manifest.version != plan.release.version
    {
        return Err(format!("{name} 插件包内容与更新清单不一致"));
    }

    let was_running = pm.is_running(config.plugin_id);
    let enabled = pm.is_enabled(config.plugin_id).await;
    let installed = plan.installed_version.is_some();
    let manifest = if installed {
        pm.update_trusted_first_party(package.path(), &preview.source_hash)
            .await
    } else {
        pm.install_trusted_first_party(package.path(), &preview.source_hash)
            .await
    }
    .map_err(|error| format!("安装 {name} 插件更新失败: {error}"))?;
    if restart_after_update
        && enabled
        && (was_running || manifest.lifecycle.autostart == "host_ready")
    {
        pm.start(config.plugin_id)
            .await
            .map_err(|error| format!("插件已更新，但重新启动失败: {error}"))?;
    }
    Ok(manifest)
}

#[tauri::command]
pub async fn check_mobile_codex_plugin_update(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    fetch_update_plan(&pm, MOBILE_CODEX_UPDATE)
        .await
        .map(|plan| update_plan_value(&plan))
}

#[tauri::command]
pub async fn update_mobile_codex_plugin(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    let plan = fetch_update_plan(&pm, MOBILE_CODEX_UPDATE).await?;
    let manifest = download_and_apply(&pm, MOBILE_CODEX_UPDATE, &plan, true).await?;
    Ok(json!({ "ok": true, "manifest": manifest, "update": update_plan_value(&plan) }))
}

#[tauri::command]
pub async fn check_desktop_sync_plugin_update(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    fetch_update_plan(&pm, DESKTOP_SYNC_UPDATE)
        .await
        .map(|plan| update_plan_value(&plan))
}

#[tauri::command]
pub async fn update_desktop_sync_plugin(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    let plan = fetch_update_plan(&pm, DESKTOP_SYNC_UPDATE).await?;
    let manifest = download_and_apply(&pm, DESKTOP_SYNC_UPDATE, &plan, true).await?;
    Ok(json!({ "ok": true, "manifest": manifest, "update": update_plan_value(&plan) }))
}

#[tauri::command]
pub async fn check_debug_plugin_update(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    fetch_update_plan(&pm, DEBUG_UPDATE)
        .await
        .map(|plan| update_plan_value(&plan))
}

#[tauri::command]
pub async fn update_debug_plugin(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    let plan = fetch_update_plan(&pm, DEBUG_UPDATE).await?;
    let manifest = download_and_apply(&pm, DEBUG_UPDATE, &plan, true).await?;
    Ok(json!({ "ok": true, "manifest": manifest, "update": update_plan_value(&plan) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_pinned_plugin_release_path() {
        assert!(validate_release_url(MOBILE_CODEX_UPDATE, MOBILE_CODEX_UPDATE_ENDPOINT).is_ok());
        assert!(validate_release_url(MOBILE_CODEX_UPDATE,
            "https://api.liguan.me/releases/dex-ai/plugins/mobile-codex/0.1.11/deecodex-mobile-codex.zip"
        )
        .is_ok());
        assert!(validate_release_url(
            MOBILE_CODEX_UPDATE,
            "http://api.liguan.me/releases/dex-ai/plugins/mobile-codex/latest.json"
        )
        .is_err());
        assert!(validate_release_url(
            MOBILE_CODEX_UPDATE,
            "https://example.com/releases/dex-ai/plugins/mobile-codex/latest.json"
        )
        .is_err());
        assert!(validate_release_url(
            MOBILE_CODEX_UPDATE,
            "https://api.liguan.me/releases/dex-ai/latest.json"
        )
        .is_err());
        assert!(validate_release_url(DESKTOP_SYNC_UPDATE, DESKTOP_SYNC_UPDATE_ENDPOINT).is_ok());
        assert!(validate_release_url(
            DESKTOP_SYNC_UPDATE,
            "https://api.liguan.me/releases/dex-ai/plugins/mobile-codex/latest.json"
        )
        .is_err());
    }

    #[test]
    fn rejects_incompatible_or_invalid_release_manifests() {
        let mut manifest = PluginReleaseManifest {
            schema_version: 1,
            plugin_id: MOBILE_CODEX_PLUGIN_ID.into(),
            version: "0.1.11".into(),
            min_deecodex_version: env!("CARGO_PKG_VERSION").into(),
            published_at: String::new(),
            release_notes: Vec::new(),
            platforms: HashMap::new(),
        };
        assert!(validate_release_manifest(MOBILE_CODEX_UPDATE, &manifest).is_ok());
        manifest.plugin_id = "other".into();
        assert!(validate_release_manifest(MOBILE_CODEX_UPDATE, &manifest).is_err());
        manifest.plugin_id = MOBILE_CODEX_PLUGIN_ID.into();
        manifest.schema_version = 2;
        assert!(validate_release_manifest(MOBILE_CODEX_UPDATE, &manifest).is_err());
    }

    #[test]
    fn installed_version_lookup_uses_the_requested_plugin_id() {
        let plugins = [
            ("deecodex-desktop-sync", "0.1.20"),
            ("deecodex-mobile-codex", "0.1.16"),
        ];
        assert_eq!(
            installed_plugin_version(plugins.into_iter(), "deecodex-desktop-sync").as_deref(),
            Some("0.1.20")
        );
        assert_eq!(
            installed_plugin_version(plugins.into_iter(), "deecodex-mobile-codex").as_deref(),
            Some("0.1.16")
        );
        assert_eq!(
            installed_plugin_version(plugins.into_iter(), "missing"),
            None
        );
    }
}
