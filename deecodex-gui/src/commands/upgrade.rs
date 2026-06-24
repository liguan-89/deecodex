use std::time::Duration;

use serde_json::{json, Value};
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

const UPDATE_ENDPOINT: &str = "https://api.liguan.me/releases/dex-ai/latest.json";

fn current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn normalize_version(version: &str) -> String {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        current_version()
    } else if trimmed.starts_with('v') || trimmed.starts_with('V') {
        trimmed.to_string()
    } else {
        format!("v{trimmed}")
    }
}

fn update_to_value(update: tauri_plugin_updater::Update) -> Value {
    json!({
        "current": normalize_version(&update.current_version),
        "latest": normalize_version(&update.version),
        "has_update": true,
        "changelog": update.body.unwrap_or_default(),
        "endpoint": UPDATE_ENDPOINT,
        "download_url": update.download_url.to_string(),
        "target": update.target,
        "source": "tauri_updater",
    })
}

pub async fn check_upgrade_with_app(app: AppHandle) -> Result<Value, String> {
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))?;

    match updater.check().await {
        Ok(Some(update)) => Ok(update_to_value(update)),
        Ok(None) => Ok(json!({
            "current": current_version(),
            "latest": current_version(),
            "has_update": false,
            "changelog": "",
            "endpoint": UPDATE_ENDPOINT,
            "source": "tauri_updater",
        })),
        Err(e) => Err(format!("检查更新失败: {e}")),
    }
}

pub async fn run_upgrade_with_app(app: AppHandle) -> Result<Value, String> {
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("初始化更新器失败: {e}"))?;

    let update = updater
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?
        .ok_or_else(|| "当前已经是最新版本".to_string())?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| format!("下载或安装更新失败: {e}"))?;

    Ok(json!({
        "installed": true,
        "restart_required": true,
        "message": "更新已安装。请重启 DEX AI 完成切换。",
    }))
}

pub fn restart_app(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        restart_installed_macos_app(&app)?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        app.request_restart();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn restart_installed_macos_app(app: &AppHandle) -> Result<(), String> {
    use std::process::Command;

    let installed_app = std::path::Path::new("/Applications/DEX AI.app");
    if installed_app.is_dir() {
        Command::new("open")
            .arg("-n")
            .arg(installed_app)
            .spawn()
            .map_err(|e| format!("打开已安装的 DEX AI 失败: {e}"))?;
        app.exit(0);
        return Ok(());
    }

    app.request_restart();
    Ok(())
}

/// DEX 助手工具链没有 AppHandle，不能执行真实安装；这里只做远端 manifest 预览。
pub async fn check_upgrade_manifest_preview() -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .user_agent(format!("DEX AI/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let resp = client
        .get(UPDATE_ENDPOINT)
        .send()
        .await
        .map_err(|e| format!("获取更新清单失败: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("获取更新清单失败，HTTP {}", resp.status()));
    }

    let body = resp
        .json::<Value>()
        .await
        .map_err(|e| format!("解析更新清单失败: {e}"))?;

    let latest = body
        .get("version")
        .and_then(Value::as_str)
        .map(normalize_version)
        .unwrap_or_else(current_version);

    Ok(json!({
        "current": current_version(),
        "latest": latest,
        "has_update": false,
        "changelog": body.get("notes").and_then(Value::as_str).unwrap_or_default(),
        "endpoint": UPDATE_ENDPOINT,
        "manifest": body,
        "source": "manifest_preview",
    }))
}

pub fn run_upgrade_manifest_preview() -> Result<String, String> {
    Err("请在服务概览页使用“检查更新/立即升级”，DEX 助手只支持查看更新清单。".to_string())
}
