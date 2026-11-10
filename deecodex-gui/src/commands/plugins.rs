use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::State;

use deecodex_plugin_host::{PluginManager, PluginManifest};

use crate::ServerManager;

use super::open_path_with_system_editor;

/// Windows：隐藏子进程控制台窗口（CREATE_NO_WINDOW），避免命令探测弹黑框。
#[cfg(target_os = "windows")]
fn hide_child_window(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}
#[cfg(not(target_os = "windows"))]
fn hide_child_window(_command: &mut std::process::Command) {}

/// 插件市场列表缓存：避免每次切换/刷新都全量扫描目录并生成预览（preview_install 较慢）。
/// 安装/卸载命令会主动失效；TTL 兜底防止长时间陈旧。
static MARKETPLACE_CACHE: OnceLock<Mutex<Option<(Instant, Vec<Value>)>>> = OnceLock::new();
const MARKETPLACE_CACHE_TTL: Duration = Duration::from_secs(8);

pub(crate) fn invalidate_marketplace_cache() {
    if let Some(cache) = MARKETPLACE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

#[tauri::command]
pub async fn browse_plugin_package() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .add_filter("插件包", &["zip"])
        .pick_file()
        .await
        .map(|f| f.path().to_string_lossy().to_string());
    Ok(path)
}

#[tauri::command]
pub async fn browse_plugin_directory() -> Result<Option<String>, String> {
    let path = rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|f| f.path().to_string_lossy().to_string());
    Ok(path)
}

#[tauri::command]
pub async fn create_plugin_from_template(
    template_id: String,
    plugin_id: String,
    name: String,
    destination_dir: String,
) -> Result<Value, String> {
    let plugin_id = plugin_id.trim();
    if !plugin_id_is_valid(plugin_id) {
        return Err("插件 ID 只能包含 ASCII 字母、数字、短横线、下划线或点".into());
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("插件名称不能为空".into());
    }
    let parent = PathBuf::from(destination_dir.trim());
    if parent.as_os_str().is_empty() {
        return Err("请选择目标目录".into());
    }
    std::fs::create_dir_all(&parent).map_err(|e| format!("无法创建目标目录: {e}"))?;

    let mut selected: Option<(PathBuf, PluginManifest)> = None;
    for candidate in plugin_marketplace_candidates() {
        if !candidate.template {
            continue;
        }
        let Ok(manifest) = PluginManifest::from_dir(&candidate.path) else {
            continue;
        };
        if manifest.id == template_id {
            selected = Some((candidate.path, manifest));
            break;
        }
    }
    let (template_path, _template_manifest) =
        selected.ok_or_else(|| format!("未找到插件模板: {template_id}"))?;
    let target = parent.join(plugin_id);
    copy_plugin_template_dir(&template_path, &target)?;
    update_plugin_template_manifest(&target, plugin_id, name)?;
    let manifest = PluginManifest::from_dir(&target).map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "path": target.to_string_lossy().to_string(),
        "manifest": plugin_manifest_summary(&manifest),
    }))
}

#[tauri::command]
pub async fn validate_plugin_path(
    manager: State<'_, ServerManager>,
    path: String,
) -> Result<Value, String> {
    let path = PathBuf::from(path.trim());
    if path.as_os_str().is_empty() {
        return Ok(json!({ "ok": false, "error": "请选择插件目录或插件包" }));
    }
    let pm = get_pm(&manager).await?;
    match pm.preview_install(&path).await {
        Ok(preview) => {
            let update_available = plugin_update_available(
                preview.existing_version.as_deref(),
                &preview.manifest.version,
                preview.previous_source_hash.as_deref(),
                &preview.source_hash,
            );
            let compatibility = plugin_compatibility_summary(
                &preview.manifest,
                &path,
                &preview.permission_risk,
                update_available,
            );
            Ok(json!({
                "ok": true,
                "manifest": plugin_manifest_summary(&preview.manifest),
                "preview": preview,
                "update_available": update_available,
                "compatibility": compatibility,
            }))
        }
        Err(error) => Ok(json!({
            "ok": false,
            "error": error.to_string(),
            "path": path.to_string_lossy().to_string(),
        })),
    }
}

#[tauri::command]
pub async fn package_plugin_directory(path: String) -> Result<Value, String> {
    let path = PathBuf::from(path.trim());
    if !path.is_dir() {
        return Err("请选择插件目录".into());
    }
    let manifest = PluginManifest::from_dir(&path).map_err(|e| e.to_string())?;
    let output = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}-{}.zip", manifest.id, manifest.version));
    let file = std::fs::File::create(&output).map_err(|e| format!("无法创建插件包: {e}"))?;
    let mut writer = zip::ZipWriter::new(file);
    add_plugin_dir_to_zip(&mut writer, &path, &path)?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "path": output.to_string_lossy().to_string(),
        "manifest": plugin_manifest_summary(&manifest),
    }))
}

#[tauri::command]
pub async fn open_plugin_directory(path: String) -> Result<Value, String> {
    let path = PathBuf::from(path.trim());
    if !path.exists() {
        return Err(format!("路径不存在: {}", path.display()));
    }
    open_path_with_system_editor(&path).map_err(|e| format!("打开失败: {e}"))?;
    Ok(json!({ "ok": true }))
}

fn personal_plugin_marketplace_root() -> Result<PathBuf, String> {
    deecodex::config::home_dir()
        .map(|home| home.join(".deecodex").join("plugin-marketplace"))
        .ok_or_else(|| "无法确定 HOME 目录".to_string())
}

#[tauri::command]
pub async fn open_plugin_marketplace_directory() -> Result<Value, String> {
    let root = personal_plugin_marketplace_root()?;
    std::fs::create_dir_all(root.join("plugins"))
        .map_err(|e| format!("无法创建个人插件目录: {e}"))?;
    std::fs::create_dir_all(root.join("templates"))
        .map_err(|e| format!("无法创建个人模板目录: {e}"))?;
    open_path_with_system_editor(&root).map_err(|e| format!("打开失败: {e}"))?;
    Ok(json!({
        "ok": true,
        "path": root.to_string_lossy().to_string(),
    }))
}

// ── 插件管理 ──────────────────────────────────────────────────────────────

pub(super) async fn get_pm(manager: &ServerManager) -> Result<Arc<PluginManager>, String> {
    let guard = manager.plugin_manager.lock().await;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| "插件管理器未初始化".into())
}

#[derive(Debug, Clone)]
struct PluginMarketplaceCandidate {
    path: PathBuf,
    source_type: &'static str,
    source_label: &'static str,
    template: bool,
}

fn push_plugin_marketplace_dir(
    items: &mut Vec<PluginMarketplaceCandidate>,
    path: PathBuf,
    source_type: &'static str,
    source_label: &'static str,
    template: bool,
) {
    if path.join("plugin.json").exists() {
        items.push(PluginMarketplaceCandidate {
            path,
            source_type,
            source_label,
            template,
        });
    }
}

fn push_plugin_marketplace_children(
    items: &mut Vec<PluginMarketplaceCandidate>,
    root: PathBuf,
    source_type: &'static str,
    source_label: &'static str,
    template: bool,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            push_plugin_marketplace_dir(items, path, source_type, source_label, template);
        }
    }
}

fn plugin_marketplace_candidates() -> Vec<PluginMarketplaceCandidate> {
    let mut items = Vec::new();
    let mut roots = Vec::new();

    if let Some(home) = deecodex::config::home_dir() {
        let personal = home.join(".deecodex").join("plugin-marketplace");
        push_plugin_marketplace_children(
            &mut items,
            personal.join("plugins"),
            "personal",
            "个人市场",
            false,
        );
        push_plugin_marketplace_children(
            &mut items,
            personal.join("templates"),
            "template",
            "个人模板",
            true,
        );
    }

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.to_path_buf());
            roots.push(dir.join("../Resources"));
            roots.push(dir.join("../Resources/deecodex-plugins"));
            // Tauri v2 打包把 resources 放进 _up_（升级保留目录）
            roots.push(dir.join("../Resources/_up_/deecodex-plugins"));
        }
    }

    for root in roots {
        push_plugin_marketplace_children(
            &mut items,
            root.join("deecodex-plugins/plugins"),
            "builtin",
            "内置插件",
            false,
        );
        push_plugin_marketplace_children(
            &mut items,
            root.join("deecodex-plugins/templates"),
            "template",
            "开发模板",
            true,
        );
        push_plugin_marketplace_children(
            &mut items,
            root.join("plugins"),
            "builtin",
            "内置插件",
            false,
        );
        push_plugin_marketplace_children(
            &mut items,
            root.join("templates"),
            "template",
            "开发模板",
            true,
        );
        for direct in [
            "deecodex-weixin",
            "node-tool",
            "node-automation",
            "python-datasource",
        ] {
            let template = direct != "deecodex-weixin";
            push_plugin_marketplace_dir(
                &mut items,
                root.join(direct),
                if template { "template" } else { "builtin" },
                if template {
                    "开发模板"
                } else {
                    "内置插件"
                },
                template,
            );
        }
    }

    items
}

fn plugin_manifest_summary(manifest: &PluginManifest) -> Value {
    json!({
        "id": manifest.id,
        "name": manifest.name,
        "version": manifest.version,
        "description": manifest.description,
        "author": manifest.author,
        "kind": manifest.kind,
        "tags": manifest.tags,
        "first_party": manifest.first_party,
        "lifecycle": manifest.lifecycle,
        "features": manifest.features,
        "permissions": manifest.permissions,
        "config_schema": manifest.config_schema,
        "account": manifest.account,
        "dex_tools": manifest.dex_tools,
        "min_deecodex_version": manifest.min_deecodex_version,
    })
}

fn plugin_update_available(
    existing_version: Option<&str>,
    manifest_version: &str,
    previous_source_hash: Option<&str>,
    source_hash: &str,
) -> bool {
    existing_version.is_some_and(|version| version != manifest_version)
        || previous_source_hash.is_some_and(|hash| hash != source_hash)
}

pub(super) fn plugin_version_parts(version: &str) -> Option<(u64, u64, u64)> {
    let cleaned = version.trim().trim_start_matches('v');
    let stable = cleaned.split(['-', '+']).next().unwrap_or(cleaned);
    let mut parts = stable.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub(super) fn plugin_version_satisfies(current: &str, minimum: &str) -> bool {
    match (plugin_version_parts(current), plugin_version_parts(minimum)) {
        (Some(current), Some(minimum)) => current >= minimum,
        _ => true,
    }
}

fn command_available(command: &str) -> bool {
    let mut probe = std::process::Command::new(command);
    hide_child_window(&mut probe);
    probe
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn plugin_runtime_status(runtime: &str) -> (bool, String, String) {
    match runtime {
        "node" => {
            let ok = command_available("node");
            (
                ok,
                "Node.js".to_string(),
                if ok {
                    "Node.js 可用"
                } else {
                    "缺少 Node.js"
                }
                .to_string(),
            )
        }
        "python" => {
            let ok = command_available("python3") || command_available("python");
            (
                ok,
                "Python".to_string(),
                if ok { "Python 可用" } else { "缺少 Python" }.to_string(),
            )
        }
        "binary" => (true, "Binary".to_string(), "本地二进制".to_string()),
        other => (false, other.to_string(), format!("未知运行时 {other}")),
    }
}

fn plugin_entry_script_status(
    source_path: &Path,
    manifest: &PluginManifest,
) -> (Option<bool>, String) {
    if !source_path.is_dir() {
        return (None, "压缩包安装时检查".to_string());
    }
    let exists = source_path.join(&manifest.entry.script).exists();
    (
        Some(exists),
        if exists {
            "入口脚本已找到".to_string()
        } else {
            format!("入口脚本缺失: {}", manifest.entry.script)
        },
    )
}

fn plugin_compatibility_summary(
    manifest: &PluginManifest,
    source_path: &Path,
    permission_risk: &str,
    update_available: bool,
) -> Value {
    let current_version = env!("CARGO_PKG_VERSION");
    let min_version = manifest.min_deecodex_version.as_deref().unwrap_or("");
    let version_ok =
        min_version.is_empty() || plugin_version_satisfies(current_version, min_version);
    let (runtime_ok, runtime_label, runtime_text) = plugin_runtime_status(&manifest.entry.runtime);
    let (script_ok, script_text) = plugin_entry_script_status(source_path, manifest);

    let mut reasons = Vec::new();
    if !version_ok {
        reasons.push(format!("需要 DEX AI {min_version}+"));
    }
    if !runtime_ok {
        reasons.push(runtime_text.clone());
    }
    if script_ok == Some(false) {
        reasons.push(script_text.clone());
    }
    if permission_risk == "high" {
        reasons.push("高风险权限，安装和执行需要确认".to_string());
    } else if permission_risk == "medium" {
        reasons.push("中风险权限，安装前建议检查".to_string());
    }
    if update_available {
        reasons.push("已安装旧版本，可更新".to_string());
    }

    let compatible = version_ok && runtime_ok && script_ok != Some(false);
    let needs_confirm = permission_risk == "high";
    let tone = if !compatible {
        "block"
    } else if needs_confirm || permission_risk == "medium" || update_available {
        "warn"
    } else {
        "ok"
    };
    let label = if !compatible {
        "不可安装"
    } else if update_available {
        "可更新"
    } else if needs_confirm {
        "需确认"
    } else {
        "兼容"
    };

    json!({
        "compatible": compatible,
        "needs_confirm": needs_confirm,
        "tone": tone,
        "label": label,
        "current_version": current_version,
        "min_version": manifest.min_deecodex_version,
        "runtime": manifest.entry.runtime,
        "entry_script": manifest.entry.script,
        "reasons": reasons,
        "checks": [
            {
                "label": "DEX 版本",
                "value": if min_version.is_empty() {
                    format!("当前 {current_version}")
                } else {
                    format!("当前 {current_version} / 要求 {min_version}+")
                },
                "tone": if version_ok { "ok" } else { "block" }
            },
            {
                "label": "运行时",
                "value": format!("{runtime_label} · {runtime_text}"),
                "tone": if runtime_ok { "ok" } else { "block" }
            },
            {
                "label": "入口脚本",
                "value": script_text,
                "tone": match script_ok {
                    Some(true) => "ok",
                    Some(false) => "block",
                    None => "muted",
                }
            },
            {
                "label": "权限",
                "value": format!("风险 {}", permission_risk),
                "tone": match permission_risk {
                    "high" => "warn",
                    "medium" => "warn",
                    _ => "ok",
                }
            }
        ]
    })
}

fn plugin_id_is_valid(id: &str) -> bool {
    let trimmed = id.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
}

fn copy_plugin_template_dir(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!("目标目录已存在: {}", target.display()));
    }
    std::fs::create_dir_all(target).map_err(|e| format!("无法创建目标目录: {e}"))?;
    copy_plugin_dir_contents(source, target)
}

fn copy_plugin_dir_contents(source: &Path, target: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(source)
        .map_err(|e| format!("无法读取模板目录 {}: {e}", source.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == ".DS_Store" || name == "target" {
            continue;
        }
        let dest = target.join(&name);
        if path.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| format!("无法创建目录: {e}"))?;
            copy_plugin_dir_contents(&path, &dest)?;
        } else {
            std::fs::copy(&path, &dest).map_err(|e| {
                format!("无法复制文件 {} -> {}: {e}", path.display(), dest.display())
            })?;
        }
    }
    Ok(())
}

fn update_plugin_template_manifest(
    target: &Path,
    plugin_id: &str,
    name: &str,
) -> Result<(), String> {
    let manifest_path = target.join("plugin.json");
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("无法读取 plugin.json: {e}"))?;
    let mut value: Value =
        serde_json::from_str(&content).map_err(|e| format!("plugin.json 格式错误: {e}"))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| "plugin.json 必须是对象".to_string())?;
    obj.insert("id".to_string(), Value::String(plugin_id.to_string()));
    obj.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(tags) = obj.get_mut("tags").and_then(Value::as_array_mut) {
        tags.retain(|tag| tag.as_str() != Some("template"));
        if !tags.iter().any(|tag| tag.as_str() == Some("local")) {
            tags.push(Value::String("local".to_string()));
        }
        if !tags.iter().any(|tag| tag.as_str() == Some("draft")) {
            tags.push(Value::String("draft".to_string()));
        }
    }
    let next = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    std::fs::write(&manifest_path, format!("{next}\n"))
        .map_err(|e| format!("无法写入 plugin.json: {e}"))?;
    PluginManifest::from_dir(target).map_err(|e| e.to_string())?;
    Ok(())
}

fn add_plugin_dir_to_zip(
    writer: &mut zip::ZipWriter<std::fs::File>,
    root: &Path,
    dir: &Path,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(dir)
        .map_err(|e| format!("无法读取目录 {}: {e}", dir.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    entries.sort_by_key(|entry| entry.path());

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name == ".git" || name == ".DS_Store" || name == "target" {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            writer
                .add_directory(format!("{relative}/"), options)
                .map_err(|e| e.to_string())?;
            add_plugin_dir_to_zip(writer, root, &path)?;
        } else {
            writer
                .start_file(relative, options)
                .map_err(|e| e.to_string())?;
            let mut file = std::fs::File::open(&path).map_err(|e| format!("无法读取文件: {e}"))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            writer.write_all(&buf).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn plugin_method_missing(error: &str) -> bool {
    error.contains("方法未找到")
        || error.contains("Method not found")
        || error.contains("method not found")
        || error.contains("-32601")
}

async fn send_plugin_account_request(
    pm: &Arc<PluginManager>,
    plugin_id: &str,
    account_id: &str,
    methods: Vec<String>,
) -> Result<Value, String> {
    send_plugin_request(
        pm,
        plugin_id,
        Some(json!({ "account_id": account_id })),
        methods,
    )
    .await
}

async fn send_plugin_request(
    pm: &Arc<PluginManager>,
    plugin_id: &str,
    params: Option<Value>,
    methods: Vec<String>,
) -> Result<Value, String> {
    let mut last_missing = None;
    for (index, method) in methods.iter().enumerate() {
        match pm
            .send_request(plugin_id, method.as_str(), params.clone())
            .await
        {
            Ok(value) => return Ok(value),
            Err(error)
                if index + 1 < methods.len() && plugin_method_missing(&error.to_string()) =>
            {
                last_missing = Some(error.to_string());
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err(last_missing.unwrap_or_else(|| "插件账号方法不可用".into()))
}

async fn plugin_account_methods(
    pm: &Arc<PluginManager>,
    plugin_id: &str,
    action: &str,
    defaults: &[&str],
) -> Vec<String> {
    let custom = pm
        .list()
        .await
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .and_then(|plugin| plugin.account)
        .and_then(|account| match action {
            "register" => account.methods.register,
            "verify_email" => account.methods.verify_email,
            "resend_verification" => account.methods.resend_verification,
            "login" => account.methods.login,
            "refresh" => account.methods.refresh,
            "logout" => account.methods.logout,
            "forgot_password" => account.methods.forgot_password,
            "reset_password" => account.methods.reset_password,
            "current_user" => account.methods.current_user,
            "list_devices" => account.methods.list_devices,
            "rename_device" => account.methods.rename_device,
            "revoke_device" => account.methods.revoke_device,
            "register_computer" => account.methods.register_computer,
            "rename_computer" => account.methods.rename_computer,
            "revoke_computer" => account.methods.revoke_computer,
            "cancel_login" => account.methods.cancel_login,
            "status" => account.methods.status,
            "start" => account.methods.start,
            "stop" => account.methods.stop,
            "launch_chatgpt" => account.methods.launch_chatgpt,
            _ => None,
        });

    let mut methods = Vec::new();
    if let Some(method) = custom {
        let method = method.trim();
        if !method.is_empty() {
            methods.push(method.to_string());
        }
    }
    for method in defaults {
        if !methods.iter().any(|existing| existing == method) {
            methods.push((*method).to_string());
        }
    }
    methods
}

fn default_account_methods(action: &str) -> &'static [&'static str] {
    match action {
        "register" => &["account.register"],
        "verify_email" => &["account.verify_email"],
        "resend_verification" => &["account.resend_verification"],
        "login" => &["account.login"],
        "refresh" => &["account.refresh"],
        "logout" => &["account.logout"],
        "forgot_password" => &["account.forgot_password"],
        "reset_password" => &["account.reset_password"],
        "current_user" => &["account.current_user"],
        "list_devices" => &["account.list_devices"],
        "rename_device" => &["account.rename_device"],
        "revoke_device" => &["account.revoke_device"],
        "register_computer" => &["account.register_computer"],
        "rename_computer" => &["account.rename_computer"],
        "revoke_computer" => &["account.revoke_computer"],
        "cancel_login" => &["account.cancel_login"],
        "status" => &["account.status"],
        "start" => &["account.start"],
        "stop" => &["account.stop"],
        "launch_chatgpt" => &["account.launch_chatgpt"],
        _ => &[],
    }
}

#[tauri::command]
pub async fn invoke_plugin_account_action(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    action: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let action = action.trim();
    if default_account_methods(action).is_empty() {
        return Err(format!("不支持的插件账号动作: {action}"));
    }
    let pm = get_pm(&manager).await?;
    if !pm.is_enabled(&plugin_id).await {
        return Err(format!("插件 '{plugin_id}' 已停用"));
    }
    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id)
            .await
            .map_err(|error| error.to_string())?;
    }
    let methods =
        plugin_account_methods(&pm, &plugin_id, action, default_account_methods(action)).await;
    send_plugin_request(&pm, &plugin_id, params, methods).await
}

/// 通用插件方法调用（DEXAI Debug 等第一方插件的宿主通道）：
/// 直接按 method 调插件 JSON-RPC，未启用/未运行自动拉起。
#[tauri::command]
pub async fn invoke_plugin_method(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    if !pm.is_enabled(&plugin_id).await {
        return Err(format!("插件 '{plugin_id}' 已停用"));
    }
    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id)
            .await
            .map_err(|error| error.to_string())?;
    }
    pm.send_request(&plugin_id, method.as_str(), params)
        .await
        .map_err(|error| error.to_string())
}

/// 插件市场正向操作（复制/安装/生成）的授权门禁：
/// 通过 DEX AI 主账号（个人中心）校验套餐/授权是否有效，未授权则拒绝。
pub(super) async fn require_wb_entitlement(manager: &ServerManager) -> Result<(), String> {
    const PLUGIN_ID: &str = "deecodex-mobile-codex";
    // 前置：DEX AI 主账号必须已登录（个人中心），退出后插件市场操作立即锁定。
    crate::commands::dex_account::ensure_master_session(manager)
        .await
        .map_err(|_| "该操作需要 DEX AI 账号授权：请先在个人中心登录 DEX AI 主账号".to_string())?;
    let pm = get_pm(manager).await?;
    if !pm.is_enabled(PLUGIN_ID).await {
        return Err("该操作需要 DEX AI 账号授权：Mobile Codex 插件未启用".into());
    }
    if !pm.is_running(PLUGIN_ID) {
        pm.start(PLUGIN_ID)
            .await
            .map_err(|error| error.to_string())?;
    }
    let methods = plugin_account_methods(
        &pm,
        PLUGIN_ID,
        "current_user",
        default_account_methods("current_user"),
    )
    .await;
    let value = send_plugin_request(&pm, PLUGIN_ID, None, methods).await?;
    let entitlement = value.get("entitlement").cloned().unwrap_or(Value::Null);
    let known = entitlement
        .get("known")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let authorized = entitlement
        .get("authorized")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !(known && authorized) {
        return Err("该操作需要 DEX AI 账号授权：请先登录 DEX AI 主账号并确保套餐有效".into());
    }
    Ok(())
}

pub(crate) async fn ensure_builtin_mobile_codex(
    pm: &Arc<PluginManager>,
) -> Result<Option<PluginManifest>, String> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("DEECODEX_MOBILE_CODEX_PLUGIN_DIR") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("../deecodex-plugins/plugins/deecodex-mobile-codex"),
        PathBuf::from("deecodex-plugins/plugins/deecodex-mobile-codex"),
    ]);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("deecodex-mobile-codex"));
            candidates.push(dir.join("../Resources/deecodex-mobile-codex"));
            candidates.push(dir.join("../Resources/deecodex-plugins/deecodex-mobile-codex"));
            candidates
                .push(dir.join("../Resources/_up_/deecodex-plugins/plugins/deecodex-mobile-codex"));
        }
    }
    ensure_builtin_mobile_codex_from_candidates(
        pm,
        candidates,
        option_env!("DEECODEX_MOBILE_CODEX_PLUGIN_SHA256"),
        cfg!(debug_assertions),
    )
    .await
}

pub(crate) async fn ensure_builtin_desktop_sync(
    pm: &Arc<PluginManager>,
) -> Result<Option<PluginManifest>, String> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("DEECODEX_DESKTOP_SYNC_PLUGIN_DIR") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("../deecodex-plugins/plugins/deecodex-desktop-sync"),
        PathBuf::from("deecodex-plugins/plugins/deecodex-desktop-sync"),
    ]);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("deecodex-desktop-sync"));
            candidates.push(dir.join("../Resources/deecodex-desktop-sync"));
            candidates.push(dir.join("../Resources/deecodex-plugins/deecodex-desktop-sync"));
            candidates
                .push(dir.join("../Resources/_up_/deecodex-plugins/plugins/deecodex-desktop-sync"));
        }
    }
    ensure_builtin_plugin_from_candidates(
        pm,
        candidates,
        "deecodex-desktop-sync",
        option_env!("DEECODEX_DESKTOP_SYNC_PLUGIN_SHA256"),
        cfg!(debug_assertions),
    )
    .await
}

pub(crate) async fn ensure_builtin_debug(
    pm: &Arc<PluginManager>,
) -> Result<Option<PluginManifest>, String> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("DEECODEX_DEBUG_PLUGIN_DIR") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend([
        PathBuf::from("../deecodex-plugins/plugins/deecodex-debug"),
        PathBuf::from("deecodex-plugins/plugins/deecodex-debug"),
    ]);
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("deecodex-debug"));
            candidates.push(dir.join("../Resources/deecodex-debug"));
            candidates.push(dir.join("../Resources/deecodex-plugins/deecodex-debug"));
            candidates
                .push(dir.join("../Resources/_up_/deecodex-plugins/plugins/deecodex-debug"));
        }
    }
    ensure_builtin_plugin_from_candidates(
        pm,
        candidates,
        "dexai-debug",
        option_env!("DEECODEX_DEBUG_PLUGIN_SHA256"),
        cfg!(debug_assertions),
    )
    .await
}

async fn ensure_builtin_mobile_codex_from_candidates(
    pm: &Arc<PluginManager>,
    candidates: Vec<PathBuf>,
    compiled_expected_hash: Option<&str>,
    allow_development_hash: bool,
) -> Result<Option<PluginManifest>, String> {
    ensure_builtin_plugin_from_candidates(
        pm,
        candidates,
        "deecodex-mobile-codex",
        compiled_expected_hash,
        allow_development_hash,
    )
    .await
}

async fn ensure_builtin_plugin_from_candidates(
    pm: &Arc<PluginManager>,
    candidates: Vec<PathBuf>,
    plugin_id: &str,
    compiled_expected_hash: Option<&str>,
    allow_development_hash: bool,
) -> Result<Option<PluginManifest>, String> {
    let installed = pm
        .list()
        .await
        .into_iter()
        .find(|plugin| plugin.id == plugin_id);
    for path in candidates {
        let Ok(manifest) = PluginManifest::from_dir(&path) else {
            continue;
        };
        if manifest.id != plugin_id || !manifest.first_party {
            continue;
        }
        let entry = path.join(&manifest.entry.script);
        let entry_exists =
            entry.is_file() || (cfg!(windows) && entry.with_extension("exe").is_file());
        if !entry_exists {
            tracing::warn!(
                path = %path.display(),
                entry = %manifest.entry.script,
                "Mobile Codex 第一方插件包不完整，跳过自动安装"
            );
            continue;
        }

        let preview = pm
            .preview_install(&path)
            .await
            .map_err(|error| error.to_string())?;
        let expected_hash = compiled_expected_hash
            .map(str::trim)
            .filter(|hash| !hash.is_empty())
            .map(str::to_string)
            .or_else(|| {
                if allow_development_hash {
                    tracing::warn!(
                        source_hash = %preview.source_hash,
                        "开发构建未固化 Mobile Codex 包哈希，临时信任当前开发包"
                    );
                    Some(preview.source_hash.clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                "发布构建缺少 DEECODEX_MOBILE_CODEX_PLUGIN_SHA256，拒绝安装第一方插件".to_string()
            })?;

        if installed.is_none() {
            let manifest = pm
                .install_trusted_first_party(&path, &expected_hash)
                .await
                .map_err(|error| error.to_string())?;
            return Ok(Some(manifest));
        }
        tracing::info!(
            plugin_id,
            bundled_version = %manifest.version,
            "第一方组件已安装，跳过内置包覆盖；更新必须由用户手动触发"
        );
        return Ok(Some(manifest));
    }
    Ok(None)
}

#[tauri::command]
pub async fn list_plugins(manager: State<'_, ServerManager>) -> Result<Vec<Value>, String> {
    let pm = get_pm(&manager).await?;
    let plugins = pm.list().await;
    Ok(plugins
        .iter()
        .map(|p| serde_json::to_value(p).unwrap_or_default())
        .collect())
}

#[tauri::command]
pub async fn list_plugin_events(
    manager: State<'_, ServerManager>,
    plugin_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<Value>, String> {
    let pm = get_pm(&manager).await?;
    let events = pm
        .recent_events(plugin_id.as_deref(), limit.unwrap_or(80))
        .await;
    Ok(events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap_or_default())
        .collect())
}

#[tauri::command]
pub async fn list_plugin_marketplace(
    manager: State<'_, ServerManager>,
) -> Result<Vec<Value>, String> {
    // TTL 缓存命中：直接返回克隆，避免重复全量扫描 + preview_install。
    if let Some(cache) = MARKETPLACE_CACHE.get() {
        if let Ok(guard) = cache.lock() {
            if let Some((at, items)) = guard.as_ref() {
                if at.elapsed() < MARKETPLACE_CACHE_TTL {
                    return Ok(items.clone());
                }
            }
        }
    }
    let pm = get_pm(&manager).await?;
    let installed = pm
        .list()
        .await
        .into_iter()
        .map(|plugin| (plugin.id.clone(), plugin))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for candidate in plugin_marketplace_candidates() {
        let manifest = match PluginManifest::from_dir(&candidate.path) {
            Ok(manifest) => manifest,
            Err(error) => {
                tracing::warn!(
                    path = %candidate.path.display(),
                    "跳过无效插件市场条目: {error}"
                );
                continue;
            }
        };
        if !seen.insert(manifest.id.clone()) {
            continue;
        }
        // 市场列表不计算 source_hash（preview_install_light）：更新判断按版本号即可，
        // 同版本内容变更场景罕见；source_hash 在安装/详情预览时仍精确计算。
        let preview = match pm.preview_install_light(&candidate.path).await {
            Ok(preview) => preview,
            Err(error) => {
                tracing::warn!(
                    id = %manifest.id,
                    path = %candidate.path.display(),
                    "生成插件市场预览失败: {error}"
                );
                continue;
            }
        };
        let installed_plugin = installed.get(&manifest.id);
        let installed_version = installed_plugin.map(|plugin| plugin.version.clone());
        let installed_state = installed_plugin
            .and_then(|plugin| serde_json::to_value(&plugin.state).ok())
            .unwrap_or(Value::Null);
        let update_available = plugin_update_available(
            installed_version.as_deref(),
            &manifest.version,
            None,
            "",
        );
        let status = if update_available {
            "update_available"
        } else if installed_version.is_some() {
            "installed"
        } else {
            "available"
        };
        let compatibility = plugin_compatibility_summary(
            &manifest,
            &candidate.path,
            &preview.permission_risk,
            update_available,
        );
        items.push(json!({
            "id": manifest.id,
            "name": manifest.name,
            "version": manifest.version,
            "description": manifest.description,
            "author": manifest.author,
            "kind": manifest.kind,
            "tags": manifest.tags,
            "features": manifest.features,
            "permissions": manifest.permissions,
            "config_schema": manifest.config_schema,
            "account": manifest.account,
            "dex_tools": manifest.dex_tools,
            "min_deecodex_version": manifest.min_deecodex_version,
            "manifest": plugin_manifest_summary(&manifest),
            "path": candidate.path.to_string_lossy().to_string(),
            "source_type": candidate.source_type,
            "source_label": candidate.source_label,
            "template": candidate.template,
            "status": status,
            "installed": installed_version.is_some(),
            "installed_version": installed_version,
            "installed_enabled": installed_plugin.map(|plugin| plugin.enabled),
            "installed_state": installed_state,
            "update_available": update_available,
            "permission_risk": preview.permission_risk,
            "permission_details": preview.permission_details,
            "compatibility": compatibility,
            "source_hash": preview.source_hash,
        }));
    }

    items.sort_by(|a, b| {
        let rank = |item: &Value| match item.get("source_type").and_then(Value::as_str) {
            Some("builtin") => 0,
            Some("template") => 1,
            Some("personal") => 2,
            _ => 3,
        };
        rank(a).cmp(&rank(b)).then_with(|| {
            a.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
        })
    });

    if let Some(cache) = MARKETPLACE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = Some((Instant::now(), items.clone()));
        }
    }
    Ok(items)
}

#[tauri::command]
pub async fn install_plugin(
    manager: State<'_, ServerManager>,
    path: Option<String>,
    archive_path: Option<String>,
    plugin_path: Option<String>,
) -> Result<Value, String> {
    let path = path
        .or(archive_path)
        .or(plugin_path)
        .ok_or_else(|| "缺少插件路径".to_string())?;
    let pm = get_pm(&manager).await?;
    let manifest = pm
        .install(std::path::Path::new(&path))
        .await
        .map_err(|e| e.to_string())?;
    invalidate_marketplace_cache();
    Ok(serde_json::to_value(&manifest).unwrap_or_default())
}

#[tauri::command]
pub async fn update_plugin(
    manager: State<'_, ServerManager>,
    path: Option<String>,
    archive_path: Option<String>,
    plugin_path: Option<String>,
) -> Result<Value, String> {
    let path = path
        .or(archive_path)
        .or(plugin_path)
        .ok_or_else(|| "缺少插件路径".to_string())?;
    let pm = get_pm(&manager).await?;
    let manifest = pm
        .update_package(std::path::Path::new(&path))
        .await
        .map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(&manifest).unwrap_or_default())
}

#[tauri::command]
pub async fn preview_plugin_install(
    manager: State<'_, ServerManager>,
    path: Option<String>,
    archive_path: Option<String>,
    plugin_path: Option<String>,
) -> Result<Value, String> {
    let path = path
        .or(archive_path)
        .or(plugin_path)
        .ok_or_else(|| "缺少插件路径".to_string())?;
    let pm = get_pm(&manager).await?;
    let preview = pm
        .preview_install(std::path::Path::new(&path))
        .await
        .map_err(|e| e.to_string())?;
    let source_path = std::path::Path::new(&path);
    let update_available = plugin_update_available(
        preview.existing_version.as_deref(),
        &preview.manifest.version,
        preview.previous_source_hash.as_deref(),
        &preview.source_hash,
    );
    let compatibility = plugin_compatibility_summary(
        &preview.manifest,
        source_path,
        &preview.permission_risk,
        update_available,
    );
    let mut value = serde_json::to_value(&preview).unwrap_or_default();
    if let Value::Object(ref mut map) = value {
        map.insert("compatibility".to_string(), compatibility);
        map.insert(
            "update_available".to_string(),
            Value::Bool(update_available),
        );
    }
    Ok(value)
}

#[tauri::command]
pub async fn uninstall_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.uninstall(&plugin_id).await.map_err(|e| e.to_string())?;
    invalidate_marketplace_cache();
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn start_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.start(&plugin_id).await.map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn stop_plugin(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.stop(&plugin_id).await.map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn set_plugin_enabled(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    enabled: bool,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.set_enabled(&plugin_id, enabled)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "plugin_id": plugin_id, "enabled": enabled }))
}

#[tauri::command]
pub async fn update_plugin_config(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    config: Value,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    pm.update_config(&plugin_id, config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub async fn upsert_plugin_account_asset(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
    asset: Option<Value>,
) -> Result<Value, String> {
    let account_id = account_id.trim().to_string();
    if account_id.is_empty() {
        return Err("连接 ID 不能为空".into());
    }
    let value = asset.unwrap_or_else(|| json!({ "name": account_id, "enabled": true }));
    let pm = get_pm(&manager).await?;
    pm.upsert_account_asset(&plugin_id, &account_id, value)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "plugin_id": plugin_id, "account_id": account_id }))
}

#[tauri::command]
pub async fn remove_plugin_account_asset(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let account_id = account_id.trim().to_string();
    if account_id.is_empty() {
        return Err("连接 ID 不能为空".into());
    }
    let pm = get_pm(&manager).await?;
    pm.remove_account_asset(&plugin_id, &account_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "plugin_id": plugin_id, "account_id": account_id }))
}

#[tauri::command]
pub async fn clear_plugin_cache(
    manager: State<'_, ServerManager>,
    plugin_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    let assets = pm
        .clear_cache(&plugin_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true, "plugin_id": plugin_id, "assets": assets }))
}

#[tauri::command]
pub async fn execute_plugin_feature(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    feature_id: String,
    action: String,
    params: Option<Value>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    if !pm.is_enabled(&plugin_id).await {
        return Err(format!(
            "插件 '{plugin_id}' 已停用，请先启用后再执行能力动作"
        ));
    }
    let plugin = pm
        .list()
        .await
        .into_iter()
        .find(|plugin| plugin.id == plugin_id)
        .ok_or_else(|| format!("插件 '{plugin_id}' 未安装"))?;
    if plugin.permission_risk == "high" && confirmed != Some(true) {
        return Err(format!(
            "插件 '{}' 包含高风险权限，执行能力动作前需要确认",
            plugin.name
        ));
    }
    let method = plugin
        .features
        .into_iter()
        .find(|feature| feature.id == feature_id)
        .and_then(|feature| feature.methods.get(&action).cloned())
        .ok_or_else(|| format!("插件能力未声明动作: {feature_id}/{action}"))?;

    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id).await.map_err(|e| e.to_string())?;
    }

    pm.send_request(&plugin_id, &method, params)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_plugin_qrcode(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id).await.map_err(|e| e.to_string())?;
    }
    send_plugin_account_request(
        &pm,
        &plugin_id,
        &account_id,
        plugin_account_methods(&pm, &plugin_id, "login", &["account.login", "weixin.login"]).await,
    )
    .await
}

#[tauri::command]
pub async fn plugin_login_cancel(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    send_plugin_account_request(
        &pm,
        &plugin_id,
        &account_id,
        plugin_account_methods(
            &pm,
            &plugin_id,
            "cancel_login",
            &[
                "account.cancel_login",
                "account.login_cancel",
                "weixin.login_cancel",
            ],
        )
        .await,
    )
    .await
}

#[tauri::command]
pub async fn query_plugin_status(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    send_plugin_account_request(
        &pm,
        &plugin_id,
        &account_id,
        plugin_account_methods(
            &pm,
            &plugin_id,
            "status",
            &["account.status", "weixin.status"],
        )
        .await,
    )
    .await
}

#[tauri::command]
pub async fn start_plugin_account(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    if !pm.is_running(&plugin_id) {
        pm.start(&plugin_id).await.map_err(|e| e.to_string())?;
    }
    send_plugin_account_request(
        &pm,
        &plugin_id,
        &account_id,
        plugin_account_methods(&pm, &plugin_id, "start", &["account.start", "weixin.start"]).await,
    )
    .await
}

#[tauri::command]
pub async fn stop_plugin_account(
    manager: State<'_, ServerManager>,
    plugin_id: String,
    account_id: String,
) -> Result<Value, String> {
    let pm = get_pm(&manager).await?;
    send_plugin_account_request(
        &pm,
        &plugin_id,
        &account_id,
        plugin_account_methods(&pm, &plugin_id, "stop", &["account.stop", "weixin.stop"]).await,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_update_available_requires_version_or_source_change() {
        assert!(!plugin_update_available(
            Some("1.0.0"),
            "1.0.0",
            Some("sha256:same"),
            "sha256:same"
        ));
        assert!(plugin_update_available(
            Some("1.0.0"),
            "1.0.1",
            Some("sha256:same"),
            "sha256:same"
        ));
        assert!(plugin_update_available(
            Some("1.0.0"),
            "1.0.0",
            Some("sha256:old"),
            "sha256:new"
        ));
        assert!(!plugin_update_available(None, "1.0.0", None, "sha256:new"));
    }

    #[tokio::test]
    async fn builtin_mobile_codex_requires_hash_and_uses_trusted_install() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let package = temp.path().join("deecodex-mobile-codex");
        std::fs::create_dir_all(&package).expect("创建插件包目录失败");
        std::fs::write(
            package.join("plugin.json"),
            r#"{
              "id": "deecodex-mobile-codex",
              "name": "Mobile Codex",
              "version": "0.1.0",
              "description": "test",
              "author": "DEX AI",
              "first_party": true,
              "lifecycle": {
                "autostart": "host_ready",
                "requires_gateway": false,
                "restart_policy": "on_failure",
                "shutdown_timeout_ms": 1000
              },
              "entry": { "runtime": "binary", "script": "mobile-codex-dex-plugin" }
            }"#,
        )
        .expect("写入 manifest 失败");
        std::fs::write(package.join("mobile-codex-dex-plugin"), b"test").expect("写入测试入口失败");

        let manager = Arc::new(PluginManager::new(
            temp.path().join("data"),
            "http://127.0.0.1:4446".into(),
        ));
        let preview = manager.preview_install(&package).await.expect("预览失败");

        let missing_hash = ensure_builtin_mobile_codex_from_candidates(
            &manager,
            vec![package.clone()],
            None,
            false,
        )
        .await;
        assert!(missing_hash.is_err(), "发布通道缺少固化哈希时必须拒绝");

        let installed = ensure_builtin_mobile_codex_from_candidates(
            &manager,
            vec![package],
            Some(&preview.source_hash),
            false,
        )
        .await
        .expect("受信任安装失败")
        .expect("应安装第一方插件");
        assert_eq!(installed.id, "deecodex-mobile-codex");
        assert!(installed.first_party);
    }

    #[tokio::test]
    async fn builtin_mobile_codex_preserves_newer_external_trusted_package() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let external = temp.path().join("external");
        let bundled = temp.path().join("bundled");
        for (package, version, content) in [
            (&external, "0.1.13", b"external".as_slice()),
            (&bundled, "0.1.11", b"bundled".as_slice()),
        ] {
            std::fs::create_dir_all(package).expect("创建插件包目录失败");
            std::fs::write(
                package.join("plugin.json"),
                format!(
                    r#"{{
                  "id": "deecodex-mobile-codex",
                  "name": "Mobile Codex",
                  "version": "{version}",
                  "description": "test",
                  "author": "DEX AI",
                  "first_party": true,
                  "lifecycle": {{
                    "autostart": "host_ready",
                    "requires_gateway": false,
                    "restart_policy": "on_failure",
                    "shutdown_timeout_ms": 1000
                  }},
                  "entry": {{ "runtime": "binary", "script": "mobile-codex-dex-plugin" }}
                }}"#
                ),
            )
            .expect("写入 manifest 失败");
            std::fs::write(package.join("mobile-codex-dex-plugin"), content)
                .expect("写入测试入口失败");
        }

        let manager = Arc::new(PluginManager::new(
            temp.path().join("data"),
            "http://127.0.0.1:4446".into(),
        ));
        let external_preview = manager.preview_install(&external).await.expect("预览失败");
        manager
            .install_trusted_first_party(&external, &external_preview.source_hash)
            .await
            .expect("外部受信任安装失败");

        let bundled_preview = manager.preview_install(&bundled).await.expect("预览失败");
        ensure_builtin_mobile_codex_from_candidates(
            &manager,
            vec![bundled],
            Some(&bundled_preview.source_hash),
            false,
        )
        .await
        .expect("检查内置包失败")
        .expect("应识别内置包");

        let installed = manager
            .list()
            .await
            .into_iter()
            .find(|plugin| plugin.id == "deecodex-mobile-codex")
            .expect("插件应保持已安装");
        assert_eq!(installed.version, "0.1.13");
        assert_eq!(installed.source_path, external.to_string_lossy());
        assert_eq!(installed.source_hash, external_preview.source_hash);
    }

    #[tokio::test]
    async fn builtin_desktop_sync_does_not_override_independently_installed_package() {
        let temp = tempfile::tempdir().expect("创建临时目录失败");
        let external = temp.path().join("external");
        let bundled = temp.path().join("bundled");
        for (package, version, content) in [
            (&external, "0.1.0", b"external".as_slice()),
            (&bundled, "0.1.5", b"bundled".as_slice()),
        ] {
            std::fs::create_dir_all(package).expect("创建插件包目录失败");
            std::fs::write(
                package.join("plugin.json"),
                format!(
                    r#"{{
                      "id": "deecodex-desktop-sync",
                      "name": "Mobile Codex 多机同步组件",
                      "version": "{version}",
                      "description": "test",
                      "author": "DEX AI",
                      "first_party": true,
                      "lifecycle": {{
                        "autostart": "manual",
                        "requires_gateway": false,
                        "restart_policy": "on_failure",
                        "shutdown_timeout_ms": 1000
                      }},
                      "entry": {{
                        "runtime": "binary",
                        "script": "mobile-codex-desktop-sync-plugin"
                      }}
                    }}"#
                ),
            )
            .expect("写入 manifest 失败");
            std::fs::write(package.join("mobile-codex-desktop-sync-plugin"), content)
                .expect("写入测试入口失败");
        }

        let manager = Arc::new(PluginManager::new(
            temp.path().join("data"),
            "http://127.0.0.1:4446".into(),
        ));
        let external_preview = manager.preview_install(&external).await.expect("预览失败");
        manager
            .install_trusted_first_party(&external, &external_preview.source_hash)
            .await
            .expect("旧同步组件安装失败");

        let bundled_preview = manager.preview_install(&bundled).await.expect("预览失败");
        ensure_builtin_plugin_from_candidates(
            &manager,
            vec![bundled.clone()],
            "deecodex-desktop-sync",
            Some(&bundled_preview.source_hash),
            false,
        )
        .await
        .expect("检查内置同步组件失败")
        .expect("应识别内置同步组件");

        let installed = manager
            .list()
            .await
            .into_iter()
            .find(|plugin| plugin.id == "deecodex-desktop-sync")
            .expect("同步组件应保持已安装");
        assert_eq!(installed.version, "0.1.0");
        assert_eq!(installed.source_path, external.to_string_lossy());
        assert_eq!(installed.source_hash, external_preview.source_hash);
        assert!(installed.first_party);
    }
}
