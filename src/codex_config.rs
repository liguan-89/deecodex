use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::Value;
use tracing::{info, warn};

/// deecodex 管理的模型目录文件名
const CATALOG_FILENAME: &str = "models_deecodex.json";
const DEX_ACCOUNT_MODEL_CACHE_FILENAME: &str = "codex_account_models_cache.json";
const DEX_ACCOUNT_MODEL_SLUG_PREFIX: &str = "dexacct";
/// 分组占位条目的 slug 前缀：视觉分组标题，选中不执行实际调用。
const DEX_ACCOUNT_GROUP_PLACEHOLDER_PREFIX: &str = "dexacct.group";
const DEECODEX_PROVIDER: &str = "deecodex";
const DEECODEX_CLI_PROVIDER: &str = "deecodex_cli";
const DEECODEX_DESKTOP_PROVIDER: &str = "deecodex_desktop";
const DEX_ROUTER_PROVIDER: &str = "dex_router";
/// 两种模式（API / 智能路由）共用的 provider key。
/// 用 `dex_ai` 而不是 `openai`，因为 Codex 把 `openai` 视为 reserved built-in provider，
/// 任何 `[model_providers.openai]` 块都会被 Codex 拒绝加载。
/// `dex_ai` 是非保留名，对外是 DEX AI 的稳定标识，对内统一两个模式。
const DEX_AI_PROVIDER: &str = "dex_ai";
const DEX_ACCOUNT_MODEL_BASE_INSTRUCTIONS: &str =
    "You are Codex, a coding agent. Follow the active Codex Desktop instructions and complete the user request safely.";
const CODEX_REGISTRY_MODEL_SLUGS: &[&str] = &[
    "gpt-5.6-sol",
    "gpt-5.6-terra",
    "gpt-5.6-luna",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "codex-auto-review",
];
/// 已退役的 Codex 官方模型。无论来源（账号 known_models、上游刷新缓存、
/// provider 内置列表）都不允许进入模型选择器。
const RETIRED_CODEX_REGISTRY_MODEL_SLUGS: &[&str] =
    &["gpt-5.2", "gpt-5.3-codex", "gpt-5.3-codex-spark"];

fn is_retired_codex_model(model: &str) -> bool {
    RETIRED_CODEX_REGISTRY_MODEL_SLUGS.contains(&model.trim())
}

pub(crate) fn managed_model_provider() -> &'static str {
    managed_model_provider_for_mode(&crate::config::default_codex_router_mode())
}

pub(crate) fn managed_model_provider_for_mode(_codex_router_mode: &str) -> &'static str {
    // 两种模式（API / 智能路由）共用 `dex_ai` provider key，
    // 让 `config.toml.model_provider` 稳定、线程切换模式时不被改写。
    // 路由差异由 `[model_providers.dex_ai].base_url` 承载。
    DEX_AI_PROVIDER
}

pub(crate) fn is_managed_model_provider(provider: &str) -> bool {
    // 旧 provider 名称保留在白名单里：migrate 用它们识别需要收敛成 `dex_ai` 的历史线程。
    matches!(
        provider.trim(),
        DEECODEX_PROVIDER
            | DEECODEX_CLI_PROVIDER
            | DEECODEX_DESKTOP_PROVIDER
            | DEX_ROUTER_PROVIDER
            | DEX_AI_PROVIDER
    )
}

pub(crate) fn active_managed_model_provider() -> String {
    let fallback = managed_model_provider().to_string();
    let Some(path) = codex_config_path() else {
        return fallback;
    };
    if !path.exists() {
        return fallback;
    }

    match read_config_file(&path).and_then(|content| managed_model_provider_from_config(&content)) {
        Ok(Some(provider)) => provider,
        Ok(None) => fallback,
        Err(err) => {
            warn!(
                path = %path.display(),
                "读取 Codex 当前 provider 失败，回退到默认 DEX provider: {err}"
            );
            fallback
        }
    }
}

fn managed_model_provider_from_config(content: &str) -> Result<Option<String>> {
    let doc: toml_edit::DocumentMut = content.parse()?;
    Ok(doc
        .get("model_provider")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|provider| is_managed_model_provider(provider))
        .map(ToString::to_string))
}

pub(crate) fn managed_model_provider_route_prefix_for_mode(
    codex_router_mode: &str,
) -> &'static str {
    let mode = crate::config::normalize_codex_router_mode(codex_router_mode);
    if mode == crate::config::CODEX_ROUTER_MODE_SMART {
        "/codex-router/v1"
    } else if mode == crate::config::CODEX_ROUTER_MODE_RELAY {
        "/codex-relay/desktop/v1"
    } else {
        "/codex-desktop/v1"
    }
}

fn expected_requires_openai_auth(
    codex_router_mode: &str,
    relay_requires_openai_auth: bool,
) -> bool {
    crate::config::codex_router_mode_is_smart(codex_router_mode)
        || (crate::config::codex_router_mode_is_relay(codex_router_mode)
            && relay_requires_openai_auth)
}

fn expected_provider_base_url(host: &str, port: u16, codex_router_mode: &str) -> String {
    format!(
        "http://{}:{}{}",
        crate::config::client_url_host(host),
        port,
        managed_model_provider_route_prefix_for_mode(codex_router_mode)
    )
}

fn auth_json_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            value
                .get("tokens")
                .and_then(|tokens| tokens.get(key))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("")
        .trim()
        .to_string()
}

fn codex_auth_json_has_login_state(value: &serde_json::Value) -> bool {
    let provider = ["type", "provider", "auth_mode"]
        .iter()
        .map(|key| auth_json_string(value, key).to_ascii_lowercase())
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    let explicit_codex = matches!(provider.as_str(), "codex" | "openai" | "chatgpt");
    let unsupported_provider = !provider.is_empty() && !explicit_codex;
    if unsupported_provider {
        return false;
    }

    let access_token = auth_json_string(value, "access_token");
    if access_token.is_empty() {
        return false;
    }
    explicit_codex
        || !auth_json_string(value, "refresh_token").is_empty()
        || !auth_json_string(value, "id_token").is_empty()
        || value.get("tokens").is_some()
}

fn default_codex_auth_file_has_login_state() -> bool {
    let Some(path) = crate::config::home_dir().map(|home| home.join(".codex").join("auth.json"))
    else {
        return false;
    };
    if !path.is_file() {
        return false;
    }
    // 复用 read_config_file：兼容 UTF-8 BOM / UTF-16 LE/BE，
    // 避免 auth.json 被 Windows 编辑器以 BOM 保存后误判为未登录。
    let content = match read_config_file(&path) {
        Ok(content) => content,
        Err(err) => {
            warn!(path = %path.display(), error = %err, "读取 Codex 登录态文件失败");
            return false;
        }
    };
    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(err) => {
            warn!(path = %path.display(), error = %err, "解析 Codex 登录态文件失败");
            return false;
        }
    };
    codex_auth_json_has_login_state(&value)
}

fn account_has_codex_official_endpoint(account: &crate::accounts::Account) -> bool {
    account
        .endpoints
        .iter()
        .any(|endpoint| endpoint.kind == crate::accounts::EndpointKind::CodexOfficial)
}

fn store_has_desktop_codex_official(store: &crate::accounts::AccountStore) -> bool {
    let desktop = crate::accounts::AccountClientSurface::Desktop;
    store.accounts.iter().any(|account| {
        account.client_kind == crate::accounts::AccountClientKind::Codex
            && account.client_surface == desktop
            && account_has_codex_official_endpoint(account)
    }) || store
        .active_account_for_surface(&desktop)
        .is_some_and(|active| {
            active
                .active_endpoint(store.active_endpoint_id_for_surface(
                    &crate::accounts::AccountClientKind::Codex,
                    &desktop,
                ))
                .or_else(|| active.endpoints.first())
                .is_some_and(|endpoint| {
                    endpoint.kind == crate::accounts::EndpointKind::CodexOfficial
                })
        })
}

pub fn relay_requires_openai_auth_for_data_dir(data_dir: &Path) -> bool {
    let store = crate::accounts::load_accounts(data_dir);
    store_has_desktop_codex_official(&store) || default_codex_auth_file_has_login_state()
}

pub(crate) fn codex_home_dir() -> Option<PathBuf> {
    crate::config::home_dir().map(|home| home.join(".codex"))
}

/// 读取配置文件，自动处理 UTF-8 / UTF-16 LE / UTF-16 BE 编码。
/// Windows 上 Codex 桌面版可能写入 UTF-16 编码的 config.toml。
pub(crate) fn read_config_file(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Ok(String::new());
    }
    // UTF-16 LE BOM
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&u16s).map_err(|e| anyhow!("UTF-16 LE 解码失败: {e}"));
    }
    // UTF-16 BE BOM
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&u16s).map_err(|e| anyhow!("UTF-16 BE 解码失败: {e}"));
    }
    // UTF-8 BOM
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        return String::from_utf8(bytes[3..].to_vec())
            .map_err(|e| anyhow!("UTF-8 (BOM) 解码失败: {e}"));
    }
    // 无 BOM — 优先 UTF-8，失败后尝试 UTF-16 LE
    match String::from_utf8(bytes.clone()) {
        Ok(s) => Ok(s),
        Err(_) => {
            if bytes.len() % 2 == 0 {
                let u16s: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                if let Ok(s) = String::from_utf16(&u16s) {
                    return Ok(s);
                }
            }
            Err(anyhow!("无法解码配置文件（不支持的文件编码）"))
        }
    }
}

pub(crate) fn codex_config_path() -> Option<PathBuf> {
    crate::config::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

pub(crate) fn find_in_path(name: &str) -> bool {
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let exe = dir.join(name);
            if exe.exists() {
                return true;
            }
            // Windows: 也检查 .exe / .cmd / .bat 后缀
            for ext in [".exe", ".cmd", ".bat"] {
                if exe.with_extension(ext).exists() {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn codex_is_installed() -> bool {
    // 1. ~/.codex 目录存在（CLI 或桌面版都可能创建）
    if let Some(home) = crate::config::home_dir() {
        if home.join(".codex").exists() {
            return true;
        }
    }
    // 2. codex 在 PATH 中
    if find_in_path("codex") {
        return true;
    }
    #[cfg(windows)]
    {
        // 3. 桌面版/MSI 安装目录
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if std::path::Path::new(&local)
                .join("Programs")
                .join("codex")
                .exists()
            {
                return true;
            }
        }
        // 4. Microsoft Store 版本
        let store = std::path::Path::new(r"C:\Program Files\WindowsApps");
        if store.exists() {
            if let Ok(entries) = std::fs::read_dir(store) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("OpenAI.Codex") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 将 deecodex 代理配置注入 codex 的 config.toml。
/// 始终尝试生成 models_deecodex.json 并设置 model_catalog_json，让 Codex 自定义 provider
/// 也能显示模型上下文窗口信息。
/// `context_window_override`: Some(size) 时覆盖目录里的上下文窗口，并按 90% 设置
/// model_auto_compact_token_limit；None 时保留 Codex 缓存中的原始上下文窗口，不设置压缩阈值。
/// 同时写入原始 model_context_window，Codex 会再按 effective_context_window_percent
/// 计算最终可用窗口。
#[allow(dead_code)]
pub fn inject(port: u16, context_window_override: Option<u32>) {
    inject_with_host(crate::config::DEFAULT_HOST, port, context_window_override);
}

pub fn inject_with_host(host: &str, port: u16, context_window_override: Option<u32>) {
    inject_with_host_and_data_dir(host, port, context_window_override, None);
}

pub fn inject_with_host_and_data_dir(
    host: &str,
    port: u16,
    context_window_override: Option<u32>,
    data_dir: Option<&std::path::Path>,
) {
    inject_with_host_and_data_dir_for_mode(
        host,
        port,
        context_window_override,
        data_dir,
        &crate::config::default_codex_router_mode(),
    );
}

pub fn inject_with_host_and_data_dir_for_mode(
    host: &str,
    port: u16,
    context_window_override: Option<u32>,
    data_dir: Option<&std::path::Path>,
    codex_router_mode: &str,
) {
    sync_codex_integration(CodexIntegrationSyncOptions {
        host,
        port,
        context_window_override,
        data_dir,
        codex_router_mode,
        relay_requires_openai_auth: false,
        reason: "legacy_inject",
    });
}

pub fn sync_codex_integration(options: CodexIntegrationSyncOptions<'_>) {
    let Some(path) = codex_config_path() else {
        info!(
            reason = options.reason,
            "跳过 Codex 集成同步: 无法确定 HOME 目录"
        );
        return;
    };
    if !path.exists() {
        if codex_is_installed() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
        } else {
            info!(
                reason = options.reason,
                "跳过 Codex 集成同步: 未检测到 Codex 安装 ({} 不存在)",
                path.display()
            );
            return;
        }
    }

    let include_account_models =
        include_account_models_in_catalog_for_mode(options.codex_router_mode);
    let active_provider = managed_model_provider_for_mode(options.codex_router_mode);
    let active_model = read_active_codex_model(&path).unwrap_or_else(|| "gpt-5.5".to_string());
    let catalog = match generate_context_catalog(
        options.context_window_override,
        &active_model,
        options.data_dir,
        include_account_models,
        active_provider,
    ) {
        Ok(catalog) => Some(catalog),
        Err(e) => {
            warn!(reason = options.reason, "生成 Codex 模型目录失败: {e}");
            let existing =
                existing_generated_catalog(options.context_window_override, &active_model);
            if existing.is_some() {
                warn!(
                    reason = options.reason,
                    "保留已有 Codex 模型目录，避免本次同步失败清空模型直选入口"
                );
            }
            existing
        }
    };

    let url_host = crate::config::client_url_host(options.host);
    let catalog_path = catalog.as_ref().map(|catalog| catalog.path.as_path());
    let model_context_window = catalog
        .as_ref()
        .and_then(|catalog| catalog.model_context_window);
    match do_inject(
        &path,
        &url_host,
        options.port,
        options.context_window_override,
        catalog_path,
        model_context_window,
        options.codex_router_mode,
        options.relay_requires_openai_auth,
    ) {
        Ok(true) => {
            if let Some(catalog) = catalog.as_ref() {
                info!(
                    reason = options.reason,
                    model_count = catalog.model_count,
                    account_model_count = catalog.account_model_count,
                    "已同步 Codex 集成并写入 DEX provider ({url_host}:{})",
                    options.port
                );
            } else {
                info!(
                    reason = options.reason,
                    "已同步 Codex 集成并写入 DEX provider ({url_host}:{})", options.port
                );
            }
        }
        Ok(false) => {
            if let Some(catalog) = catalog.as_ref() {
                info!(
                    reason = options.reason,
                    model_count = catalog.model_count,
                    account_model_count = catalog.account_model_count,
                    "已同步 Codex 集成并更新服务地址 ({url_host}:{})",
                    options.port
                );
            } else {
                info!(
                    reason = options.reason,
                    "已同步 Codex 集成并更新服务地址 ({url_host}:{})", options.port
                );
            }
        }
        Err(e) => warn!(reason = options.reason, "同步 Codex 集成失败: {e}"),
    }
}

pub fn codex_integration_restore_reasons(options: CodexIntegrationCheckOptions<'_>) -> Vec<String> {
    let Some(path) = codex_config_path() else {
        return vec!["home_dir_missing".into()];
    };
    let content = match read_config_file(&path) {
        Ok(content) => content,
        Err(err) => return vec![format!("config_unreadable:{err}")],
    };
    codex_integration_restore_reasons_for_content(
        &content,
        options.host,
        options.port,
        options.codex_router_mode,
        options.relay_requires_openai_auth,
    )
}

fn codex_integration_restore_reasons_for_content(
    content: &str,
    host: &str,
    port: u16,
    codex_router_mode: &str,
    relay_requires_openai_auth: bool,
) -> Vec<String> {
    let doc: toml_edit::DocumentMut = match content.parse() {
        Ok(doc) => doc,
        Err(err) => return vec![format!("config_parse_failed:{err}")],
    };
    let mut reasons = Vec::new();
    let expected_provider = managed_model_provider_for_mode(codex_router_mode);
    let actual_provider = doc
        .get("model_provider")
        .and_then(|value| value.as_str())
        .map(str::trim);
    if actual_provider != Some(expected_provider) {
        reasons.push(format!(
            "model_provider:{}",
            actual_provider.unwrap_or("<missing>")
        ));
    }

    let expected_base_url = expected_provider_base_url(host, port, codex_router_mode);
    let provider = doc
        .get("model_providers")
        .and_then(|providers| providers.get(expected_provider));
    match provider {
        Some(provider) => {
            let actual_base_url = provider
                .get("base_url")
                .and_then(|value| value.as_str())
                .map(str::trim);
            if actual_base_url != Some(expected_base_url.as_str()) {
                reasons.push(format!(
                    "provider_base_url:{}",
                    actual_base_url.unwrap_or("<missing>")
                ));
            }
            let expected_auth =
                expected_requires_openai_auth(codex_router_mode, relay_requires_openai_auth);
            if provider
                .get("requires_openai_auth")
                .and_then(|value| value.as_bool())
                != Some(expected_auth)
            {
                reasons.push("provider_auth_mismatch".into());
            }
            if provider
                .get("wire_api")
                .and_then(|value| value.as_str())
                .map(str::trim)
                != Some("responses")
            {
                reasons.push("provider_wire_api_mismatch".into());
            }
        }
        None => reasons.push(format!("provider_missing:{expected_provider}")),
    }

    let catalog_matches = doc
        .get("model_catalog_json")
        .and_then(|value| value.as_str())
        .and_then(|value| std::path::Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .is_some_and(|file_name| file_name == CATALOG_FILENAME);
    if !catalog_matches {
        reasons.push("model_catalog_json_missing_or_external".into());
    }

    reasons
}

fn include_account_models_in_catalog_for_mode(codex_router_mode: &str) -> bool {
    !crate::config::codex_router_mode_is_relay(codex_router_mode)
}

/// 从 codex 的 config.toml 中移除 deecodex 代理配置。
pub fn remove() {
    let Some(path) = codex_config_path() else {
        return;
    };
    if !path.exists() {
        return;
    }

    match do_remove(&path) {
        Ok(true) => info!("已从 codex config.toml 移除 deecodex 配置"),
        Ok(false) => {} // 本来就没有 deecodex 配置
        Err(e) => warn!("移除 codex 配置失败: {e}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn do_inject(
    path: &std::path::Path,
    url_host: &str,
    port: u16,
    context_window_override: Option<u32>,
    catalog_path: Option<&std::path::Path>,
    model_context_window: Option<i64>,
    codex_router_mode: &str,
    relay_requires_openai_auth: bool,
) -> Result<bool> {
    let content = read_config_file(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let active_provider = managed_model_provider_for_mode(codex_router_mode);
    let already_exists = doc
        .get("model_providers")
        .and_then(|mp| mp.get(active_provider))
        .is_some();

    doc["model_provider"] = toml_edit::value(active_provider);

    // 确保 model_providers 是常规表（非内联表），避免与用户自定义 provider 冲突
    if doc.get("model_providers").is_none() {
        doc.insert(
            "model_providers",
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }
    write_deecodex_provider(
        doc.as_table_mut(),
        DEECODEX_PROVIDER,
        url_host,
        port,
        "/v1",
        false,
    );
    let cli_route_prefix = if crate::config::codex_router_mode_is_relay(codex_router_mode) {
        "/codex-relay/cli/v1"
    } else {
        "/codex-cli/v1"
    };
    let desktop_route_prefix = if crate::config::codex_router_mode_is_relay(codex_router_mode) {
        "/codex-relay/desktop/v1"
    } else {
        "/codex-desktop/v1"
    };
    write_deecodex_provider(
        doc.as_table_mut(),
        DEECODEX_CLI_PROVIDER,
        url_host,
        port,
        cli_route_prefix,
        false,
    );
    write_deecodex_provider(
        doc.as_table_mut(),
        DEECODEX_DESKTOP_PROVIDER,
        url_host,
        port,
        desktop_route_prefix,
        false,
    );
    // 活动 provider 统一用 `dex_ai` key（Codex 把 `openai` 当成 reserved built-in，
    // 不能写在 [model_providers] 里）。路由差异（直连 / 智能路由）通过 base_url 区分：
    //   API 模式    → /codex-desktop/v1, requires_openai_auth = false
    //   中转模式    → /codex-relay/desktop/v1, requires_openai_auth = 是否检测到官方登录态
    //   智能路由    → /codex-router/v1,  requires_openai_auth = true
    write_deecodex_provider(
        doc.as_table_mut(),
        DEX_AI_PROVIDER,
        url_host,
        port,
        managed_model_provider_route_prefix_for_mode(codex_router_mode),
        expected_requires_openai_auth(codex_router_mode, relay_requires_openai_auth),
    );
    // 旧版智能路由模式曾写入 `dex_router` provider 块；现在改用 `dex_ai` 后该块不再被引用，主动清理掉。
    if let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(|providers| providers.as_table_mut())
    {
        providers.remove(DEX_ROUTER_PROVIDER);
    }

    if let Some(catalog_path) = catalog_path {
        doc["model_catalog_json"] = toml_edit::value(catalog_path.to_string_lossy().to_string());
    } else {
        doc.remove("model_catalog_json");
    }

    if let Some(model_context_window) = model_context_window {
        doc["model_context_window"] = toml_edit::value(model_context_window);
    } else {
        doc.remove("model_context_window");
    }

    // 只有显式覆盖上下文窗口时才调整自动压缩阈值。
    if let Some(cw) = context_window_override {
        // 压缩阈值按当前模型的有效窗口（catalog per-model 解析值优先）计算，
        // 避免切换小窗口模型后仍按 1M 的 90% 阈值不触发压缩。
        let effective = model_context_window.map(|w| w as u64).unwrap_or(cw as u64);
        let compact_limit = (effective * 9 / 10).min(i64::MAX as u64) as i64;
        doc["model_auto_compact_token_limit"] = toml_edit::value(compact_limit);
        info!(
            "已启用大上下文: context_window={effective}, auto_compact_token_limit={compact_limit}"
        );
    } else {
        doc.remove("model_auto_compact_token_limit");
    }

    std::fs::write(path, doc.to_string())?;
    Ok(!already_exists)
}

fn write_deecodex_provider(
    doc: &mut toml_edit::Table,
    provider: &str,
    url_host: &str,
    port: u16,
    route_prefix: &str,
    requires_openai_auth: bool,
) {
    doc["model_providers"][provider]["base_url"] =
        toml_edit::value(format!("http://{}:{}{}", url_host, port, route_prefix));
    doc["model_providers"][provider]["name"] = toml_edit::value(provider);
    doc["model_providers"][provider]["requires_openai_auth"] =
        toml_edit::value(requires_openai_auth);
    doc["model_providers"][provider]["api_key"] = toml_edit::value("");
    doc["model_providers"][provider]["wire_api"] = toml_edit::value("responses");
}

fn do_remove(path: &std::path::Path) -> Result<bool> {
    let content = read_config_file(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let mut removed = false;

    if doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .is_some_and(is_managed_model_provider)
    {
        doc.remove("model_provider");
        removed = true;
    }

    // 顶层 model 若是 dex 账号模型（dexacct.*，含分组占位），一并移除，
    // 避免停止服务/关闭自动注入后 Codex 仍残留 dex 模型引用。
    if doc
        .get("model")
        .and_then(|v| v.as_str())
        .is_some_and(|model| model.starts_with(DEX_ACCOUNT_MODEL_SLUG_PREFIX))
    {
        doc.remove("model");
        removed = true;
    }

    // 清理大上下文相关配置
    if doc.remove("model_catalog_json").is_some() {
        removed = true;
    }
    if doc.remove("model_context_window").is_some() {
        removed = true;
    }
    if doc.remove("model_auto_compact_token_limit").is_some() {
        removed = true;
    }
    // 不删除 models_deecodex.json。Codex 的下拉直选列表依赖这个目录文件；
    // 停止服务或关闭自动注入只应移除 config.toml 绑定，不应把可恢复目录物理清空。

    // 尝试从常规 table 或 inline table 中移除 deecodex
    if let Some(providers) = doc.get_mut("model_providers") {
        // 检查是否是 inline table
        let mut found = false;
        if let Some(inline) = providers.as_inline_table_mut() {
            found |= inline.remove(DEECODEX_PROVIDER).is_some();
            found |= inline.remove(DEECODEX_CLI_PROVIDER).is_some();
            found |= inline.remove(DEECODEX_DESKTOP_PROVIDER).is_some();
            found |= inline.remove(DEX_ROUTER_PROVIDER).is_some();
            found |= inline.remove(DEX_AI_PROVIDER).is_some();
            if inline.is_empty() {
                doc.remove("model_providers");
            }
        } else if let Some(table) = providers.as_table_mut() {
            found |= table.remove(DEECODEX_PROVIDER).is_some();
            found |= table.remove(DEECODEX_CLI_PROVIDER).is_some();
            found |= table.remove(DEECODEX_DESKTOP_PROVIDER).is_some();
            found |= table.remove(DEX_ROUTER_PROVIDER).is_some();
            found |= table.remove(DEX_AI_PROVIDER).is_some();
            if table.is_empty() {
                doc.remove("model_providers");
            }
        }
        // 兜底：如果以上方法都不行，直接删除整个 model_providers
        if !found {
            doc.remove("model_providers");
            removed = true;
        } else {
            removed = true;
        }
    }

    if removed {
        std::fs::write(path, doc.to_string())?;
    }
    Ok(removed)
}

struct GeneratedCatalog {
    path: std::path::PathBuf,
    model_context_window: Option<i64>,
    model_count: usize,
    account_model_count: usize,
}

pub struct CodexIntegrationSyncOptions<'a> {
    pub host: &'a str,
    pub port: u16,
    pub context_window_override: Option<u32>,
    pub data_dir: Option<&'a std::path::Path>,
    pub codex_router_mode: &'a str,
    pub relay_requires_openai_auth: bool,
    pub reason: &'a str,
}

pub struct CodexIntegrationCheckOptions<'a> {
    pub host: &'a str,
    pub port: u16,
    pub codex_router_mode: &'a str,
    pub relay_requires_openai_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexAccountModelRef {
    pub account_id: String,
    pub endpoint_id: String,
    pub model: String,
}

#[derive(Debug, Clone)]
struct DexCatalogAccountModel {
    account_id: String,
    endpoint_id: String,
    account_name: String,
    endpoint_name: String,
    endpoint_kind: crate::accounts::EndpointKind,
    provider: String,
    model: String,
    context_window_override: Option<u32>,
}

pub fn encode_dex_account_model_slug(account_id: &str, endpoint_id: &str, model: &str) -> String {
    format!(
        "{}.{}.{}.{}",
        DEX_ACCOUNT_MODEL_SLUG_PREFIX,
        encode_slug_part(account_id),
        encode_slug_part(endpoint_id),
        encode_slug_part(model)
    )
}

/// 分组占位条目 slug：`dexacct.group.<base64(account_id)>`。
/// 该 id 不是真实模型，deecodex 请求层遇到它会直接返回错误提示，
/// 不会发起任何实际调用。
pub fn encode_dex_account_group_placeholder_slug(account_id: &str) -> String {
    format!(
        "{DEX_ACCOUNT_GROUP_PLACEHOLDER_PREFIX}.{}",
        encode_slug_part(account_id)
    )
}

/// 判断 slug 是否为分组占位条目。
pub fn is_dex_account_group_placeholder_slug(slug: &str) -> bool {
    slug.starts_with(DEX_ACCOUNT_GROUP_PLACEHOLDER_PREFIX)
}

pub fn decode_dex_account_model_slug(slug: &str) -> Option<DexAccountModelRef> {
    let mut parts = slug.split('.');
    if parts.next()? != DEX_ACCOUNT_MODEL_SLUG_PREFIX {
        return None;
    }
    let account_id = decode_slug_part(parts.next()?)?;
    let endpoint_id = decode_slug_part(parts.next()?)?;
    let model = decode_slug_part(parts.next()?)?;
    if parts.next().is_some() || account_id.is_empty() || endpoint_id.is_empty() || model.is_empty()
    {
        return None;
    }
    Some(DexAccountModelRef {
        account_id,
        endpoint_id,
        model,
    })
}

fn encode_slug_part(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(value.as_bytes())
}

fn decode_slug_part(value: &str) -> Option<String> {
    let bytes = URL_SAFE_NO_PAD.decode(value.as_bytes()).ok()?;
    String::from_utf8(bytes).ok()
}

fn account_model_cache_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(DEX_ACCOUNT_MODEL_CACHE_FILENAME)
}

#[allow(dead_code)]
pub fn save_account_model_cache(
    data_dir: &std::path::Path,
    account_id: &str,
    endpoint_id: &str,
    models: &[String],
) -> Result<()> {
    let mut cache = read_account_model_cache(data_dir);
    let account_entry = cache
        .as_object_mut()
        .ok_or_else(|| anyhow!("账号模型缓存格式异常"))?
        .entry(account_id.to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !account_entry.is_object() {
        *account_entry = serde_json::json!({});
    }
    let models = models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|model| Value::String(model.to_string()))
        .collect::<Vec<_>>();
    account_entry[endpoint_id] = serde_json::json!({
        "updated_at": crate::accounts::now_secs(),
        "models": models,
    });
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(
        account_model_cache_path(data_dir),
        serde_json::to_string_pretty(&cache)?,
    )?;
    Ok(())
}

fn read_account_model_cache(data_dir: &std::path::Path) -> Value {
    let path = account_model_cache_path(data_dir);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}

fn cached_models_for(cache: &Value, account_id: &str, endpoint_id: &str) -> Vec<String> {
    cache
        .get(account_id)
        .and_then(|account| account.get(endpoint_id))
        .and_then(|entry| entry.get("models"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[allow(dead_code)]
pub fn account_model_cache_for(
    data_dir: &std::path::Path,
    account_id: &str,
    endpoint_id: &str,
) -> Vec<String> {
    let cache = read_account_model_cache(data_dir);
    cached_models_for(&cache, account_id, endpoint_id)
}

/// 从 models_cache.json 生成 deecodex 模型目录，写入 ~/.codex/models_deecodex.json。
fn generate_context_catalog(
    context_window_override: Option<u32>,
    active_model: &str,
    data_dir: Option<&std::path::Path>,
    include_account_models: bool,
    catalog_account_provider: &str,
) -> Result<GeneratedCatalog> {
    let Some(codex_home) = codex_home_dir() else {
        return Err(anyhow!("无法确定 HOME 目录"));
    };

    let cache_path = codex_home.join("models_cache.json");
    let catalog_path = codex_home.join(CATALOG_FILENAME);

    let catalog: Value = if cache_path.exists() {
        match std::fs::read_to_string(&cache_path)
            .map_err(|e| anyhow!("读取 models_cache.json 失败: {e}"))
            .and_then(|content| {
                serde_json::from_str(&content)
                    .map_err(|e| anyhow!("解析 models_cache.json 失败: {e}"))
            }) {
            Ok(catalog) => catalog,
            Err(err) => {
                warn!("Codex models_cache.json 不可用，使用 DEX 内置模型目录兜底: {err}");
                fallback_codex_model_catalog()
            }
        }
    } else {
        warn!(
            path = %cache_path.display(),
            "Codex models_cache.json 不存在，使用 DEX 内置模型目录兜底"
        );
        fallback_codex_model_catalog()
    };

    let account_models = if include_account_models {
        data_dir
            .map(dex_catalog_account_models)
            .transpose()?
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let preserve_existing_account_entries = include_account_models
        && account_models.is_empty()
        && should_preserve_existing_account_catalog(data_dir, &catalog_path);
    let preserved_account_entries = if preserve_existing_account_entries {
        let entries = existing_catalog_account_entries(&catalog_path);
        if !entries.is_empty() {
            warn!(
                path = %catalog_path.display(),
                data_dir = data_dir.map(|path| path.display().to_string()).unwrap_or_default(),
                count = entries.len(),
                "本次未生成账号模型，保留已有 Codex 账号模型目录项"
            );
        }
        entries
    } else {
        Vec::new()
    };
    let catalog_out = build_context_catalog(
        catalog,
        context_window_override,
        &account_models,
        &preserved_account_entries,
        catalog_account_provider,
    )?;
    // config.toml 的 model_context_window 跟随当前模型：优先从 catalog 解析
    // 当前模型的窗口（per-model），解析不到才回退全局 override。这样切换到
    // 256k 等小窗口模型时不会残留 1M 窗口值。
    let model_context_window =
        resolve_model_context_window(&catalog_out, active_model).or_else(|| {
            context_window_override.map(|window| (window as u64).min(i64::MAX as u64) as i64)
        });
    let model_count = catalog_out
        .get("models")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let account_model_count = catalog_out
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| {
            model
                .get("slug")
                .and_then(Value::as_str)
                .is_some_and(|slug| slug.starts_with(DEX_ACCOUNT_MODEL_SLUG_PREFIX))
        })
        .count();
    let json = serde_json::to_string_pretty(&catalog_out)
        .map_err(|e| anyhow!("序列化模型目录失败: {e}"))?;
    std::fs::write(&catalog_path, json)
        .map_err(|e| anyhow!("写入 models_deecodex.json 失败: {e}"))?;
    if let Some(context_window) = context_window_override {
        info!(
            "已生成大上下文模型目录: {} (context_window={})",
            catalog_path.display(),
            context_window
        );
    } else {
        info!(
            "已生成 Codex 模型目录: {} (保留原始上下文窗口)",
            catalog_path.display()
        );
    }
    Ok(GeneratedCatalog {
        path: catalog_path,
        model_context_window,
        model_count,
        account_model_count,
    })
}

fn fallback_codex_model_catalog() -> Value {
    serde_json::json!({ "models": [] })
}

fn existing_generated_catalog(
    context_window_override: Option<u32>,
    active_model: &str,
) -> Option<GeneratedCatalog> {
    if let Some(codex_home) = codex_home_dir() {
        let catalog_path = codex_home.join(CATALOG_FILENAME);
        let content = std::fs::read_to_string(&catalog_path).ok()?;
        let catalog = serde_json::from_str::<Value>(&content).ok()?;
        let model_count = catalog_model_count(&catalog);
        if model_count == 0 {
            return None;
        }
        let account_model_count = catalog_account_model_count(&catalog);
        let model_context_window =
            resolve_model_context_window(&catalog, active_model).or_else(|| {
                context_window_override.map(|window| (window as u64).min(i64::MAX as u64) as i64)
            });
        return Some(GeneratedCatalog {
            path: catalog_path,
            model_context_window,
            model_count,
            account_model_count,
        });
    }
    None
}

fn catalog_model_count(catalog: &Value) -> usize {
    catalog
        .get("models")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn catalog_account_model_count(catalog: &Value) -> usize {
    catalog
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|model| model_is_dex_account_model(model))
        .count()
}

fn is_codex_native_text_gpt_model(model: &str) -> bool {
    matches!(
        model,
        "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" | "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini"
    )
}

fn dex_catalog_account_models(data_dir: &std::path::Path) -> Result<Vec<DexCatalogAccountModel>> {
    let store = crate::accounts::load_accounts_checked(data_dir)?;
    let cache = read_account_model_cache(data_dir);
    let mut values = Vec::new();
    for account in store
        .accounts
        .iter()
        .filter(|account| account.client_kind.is_codex())
        .filter(|account| account.client_surface == crate::accounts::AccountClientSurface::Desktop)
    {
        for endpoint in &account.endpoints {
            let models = catalog_models_for_endpoint(account, endpoint, &cache);
            let models = models
                .into_iter()
                .map(|model| model.trim().to_string())
                .filter(|model| {
                    !model.is_empty() && !crate::providers::model_is_image_generation_only(model)
                })
                .collect::<std::collections::BTreeSet<_>>();
            for model in models {
                let context_window_override =
                    account_model_context_window_override(account, endpoint, &model);
                values.push(DexCatalogAccountModel {
                    account_id: account.id.clone(),
                    endpoint_id: endpoint.id.clone(),
                    account_name: account.name.clone(),
                    endpoint_name: endpoint.name.clone(),
                    endpoint_kind: endpoint.kind.clone(),
                    provider: account.provider.clone(),
                    model,
                    context_window_override,
                });
            }
        }
    }
    Ok(values)
}

fn account_model_context_window_override(
    account: &crate::accounts::Account,
    endpoint: &crate::accounts::EndpointConfig,
    model: &str,
) -> Option<u32> {
    endpoint
        .context_window_override
        .or(account.context_window_override)
        .or_else(|| one_m_context_window_for_model(model))
        .or_else(|| known_model_context_window(model))
}

fn one_m_context_window_for_model(model: &str) -> Option<u32> {
    model
        .trim()
        .to_ascii_lowercase()
        .ends_with("[1m]")
        .then_some(1_000_000)
}

/// 已知第三方模型的真实上下文窗口（token），取值依据各厂商官方文档。
///
/// - DeepSeek V4-Pro / V4-Flash：官方 1M 上下文；
/// - MiniMax：M3 / M1 为 1M，M2.7 / M2.5 / M2.1 / M2 为 204,800；
/// - Kimi：k3 为 1M，k2.7-code / k2.6 / k2.5 为 256K；
/// - Qwen3.5-Plus：1M；
/// - MiniMax MiMo v2.5 / v2.5-pro：1M。
///
/// 未列出的模型返回 None，调用方不应再回退继承 gpt-5.5 的 272000 模板值，
/// 以免 Codex 按错误的大窗口跳过自动压缩、导致上游 context 超限报错。
fn known_model_context_window(model: &str) -> Option<u32> {
    let slug = model.trim().to_ascii_lowercase();
    let base = slug.strip_suffix("[1m]").unwrap_or(&slug);
    match base {
        // DeepSeek V4 全系：官方 1M 上下文。
        "deepseek-v4-pro" | "deepseek-v4-flash" => Some(1_000_000),
        // MiniMax：M3 / M1 为 1M。
        "minimax-m3" | "minimax-m1" => Some(1_000_000),
        // MiniMax M2 系列：官方 204,800。
        "minimax-m2.7"
        | "minimax-m2.7-highspeed"
        | "minimax-m2.5"
        | "minimax-m2.5-highspeed"
        | "minimax-m2.1"
        | "minimax-m2.1-highspeed"
        | "minimax-m2" => Some(204_800),
        // Kimi：k3 为 1M；K2 系列为 256K。
        "kimi-k3" => Some(1_000_000),
        "kimi-k2.7-code"
        | "kimi-k2.6"
        | "kimi-k2.5"
        | "kimi-k2-turbo-preview"
        | "kimi-k2-0711-preview" => Some(256_000),
        // Qwen3.5-Plus：官方 1M。
        "qwen3.5-plus" => Some(1_000_000),
        // MiniMax MiMo v2.5 / v2.5-pro：官方 1M。
        "mimo-v2.5" | "mimo-v2.5-pro" => Some(1_000_000),
        _ => None,
    }
}

fn catalog_models_for_endpoint(
    account: &crate::accounts::Account,
    endpoint: &crate::accounts::EndpointConfig,
    cache: &Value,
) -> Vec<String> {
    // 只保留账号真实获取/配置的模型：优先上游刷新缓存，其次账号 known_models，
    // 最后账号 default_model。不再注入硬编码的 Codex 官方 GPT 模板或 provider
    // 兜底模型——那些不是账号从上游获取到的，不应出现在模型选择器里。
    let mut models = cached_models_for(cache, &account.id, &endpoint.id);
    if models.is_empty() {
        models.extend(endpoint.known_models.iter().cloned());
    }
    if models.is_empty() && !account.default_model.trim().is_empty() {
        models.push(account.default_model.trim().to_string());
    }
    // 退役/图像模型一律不进选择器，无论账号数据里是否残留。
    models.retain(|model| {
        let model = model.trim();
        // 账号停用的模型不注入 Codex 模型选择器（客户端看不到也调不到）。
        let disabled = endpoint
            .model_profiles
            .get(model)
            .map(|profile| !profile.enabled)
            .unwrap_or(false);
        !model.is_empty()
            && !disabled
            && !is_retired_codex_model(model)
            && !crate::providers::model_is_image_generation_only(model)
    });
    models
}

fn build_context_catalog(
    mut catalog: Value,
    context_window_override: Option<u32>,
    account_models: &[DexCatalogAccountModel],
    preserved_account_entries: &[Value],
    catalog_account_provider: &str,
) -> Result<Value> {
    let models = catalog
        .get_mut("models")
        .and_then(|m| m.as_array_mut())
        .ok_or_else(|| anyhow!("models_cache.json 格式异常: 缺少 models 数组"))?;

    prune_retired_codex_registry_models(models);
    ensure_codex_registry_models(models);

    for model in models.iter_mut() {
        // 上下文窗口按模型各自维护：全局 context_window_override 只作用于
        // 对应账号的直选模型（见 prepend_dex_account_catalog_models），不再
        // 强制覆盖缓存里的原生 Codex 模型，避免把 256k 等小窗口模型误标为 1M。
        if model.get("context_window").is_none() {
            if let Some(max_context_window) = model.get("max_context_window").cloned() {
                model["context_window"] = max_context_window;
            }
        } else if model.get("max_context_window").is_none() {
            if let Some(context_window) = model.get("context_window").cloned() {
                model["max_context_window"] = context_window;
            }
        }
    }

    let template_models = models.clone();
    // 彻底移除 UI 可见的 Codex 官方 GPT 模板：它们无账号归属、无 provider，
    // 会在 Codex 模型选择器里形成无分组的“官方模型”区（如 GPT-5.6 Sol/Terra）。
    // 所需 GPT 模型均由账号直选分组（dexacct.*）提供；codex-auto-review 是
    // Codex 内部自动审查模型，保留（本来就 hidden，不进选择器）。
    // 注意必须在 template_models 之后执行：账号直选模型仍要借官方模板生成元数据。
    prune_native_gpt_models(models);
    prepend_dex_account_catalog_models(
        models,
        &template_models,
        account_models,
        context_window_override,
        catalog_account_provider,
    );
    prepend_preserved_account_catalog_models(
        models,
        preserved_account_entries,
        catalog_account_provider,
    );
    ensure_model_catalog_schema(models, &catalog_model_template(&template_models));

    // model_catalog_json 只接受 {"models": [...]}，去掉缓存中的额外字段。
    let models = catalog
        .get("models")
        .cloned()
        .ok_or_else(|| anyhow!("models_cache.json 格式异常: 缺少 models 数组"))?;
    Ok(serde_json::json!({ "models": models }))
}

fn prune_retired_codex_registry_models(models: &mut Vec<Value>) {
    models.retain(|model| {
        let slug = model.get("slug").and_then(Value::as_str).unwrap_or("");
        !RETIRED_CODEX_REGISTRY_MODEL_SLUGS.contains(&slug)
    });
}

/// 从目录中移除所有 UI 可见的 Codex 官方 GPT 模板（is_codex_native_text_gpt_model），
/// 保留 codex-auto-review 等内部辅助模型。账号直选模型（dexacct.*）不在匹配范围。
fn prune_native_gpt_models(models: &mut Vec<Value>) {
    models.retain(|model| {
        let slug = model.get("slug").and_then(Value::as_str).unwrap_or("");
        !is_codex_native_text_gpt_model(slug)
    });
}

fn ensure_codex_registry_models(models: &mut Vec<Value>) {
    for slug in CODEX_REGISTRY_MODEL_SLUGS {
        let Some(registry_model) = codex_registry_model_template(slug) else {
            continue;
        };
        if let Some(model) = models
            .iter_mut()
            .find(|model| model.get("slug").and_then(Value::as_str) == Some(*slug))
        {
            merge_codex_registry_model_metadata(model, &registry_model);
        } else {
            models.push(registry_model);
        }
    }
}

fn merge_codex_registry_model_metadata(model: &mut Value, registry_model: &Value) {
    for key in [
        "display_name",
        "description",
        "auto_compact_token_limit",
        "default_reasoning_level",
        "shell_type",
        "visibility",
        "minimal_client_version",
        "supported_in_api",
        "priority",
        "support_verbosity",
        "default_verbosity",
        "supports_image_detail_original",
        "supports_parallel_tool_calls",
        "reasoning_summary_format",
        "default_reasoning_summary",
        "prefer_websockets",
        "apply_patch_tool_type",
        "web_search_tool_type",
        "truncation_policy",
    ] {
        if model.get(key).is_none() || model.get(key).is_some_and(Value::is_null) {
            if let Some(value) = registry_model.get(key) {
                model[key] = value.clone();
            }
        }
    }

    for key in [
        "supported_reasoning_levels",
        "service_tiers",
        "input_modalities",
    ] {
        let should_fill = model
            .get(key)
            .and_then(Value::as_array)
            .is_none_or(|values| values.is_empty());
        if should_fill {
            if let Some(value) = registry_model.get(key) {
                model[key] = value.clone();
            }
        }
    }

    for key in ["context_window", "max_context_window"] {
        let current = model.get(key).and_then(Value::as_u64).unwrap_or_default();
        let Some(registry) = registry_model.get(key).and_then(Value::as_u64) else {
            continue;
        };
        if current < registry {
            model[key] = Value::from(registry);
        }
    }
}

/// 展开 1M 变体：支持 1M 的账号模型同时生成 `模型[1m]` 与不带后缀两个条目，
/// 让 Codex 桌面模型选择器里能显式区分「1M / 非 1M」两个可选模型。
/// - 模型本身已知 1M 窗口（如 DeepSeek V4、MiniMax M3、Kimi K3 等）→ 补一个 `[1m]` 后缀条目；
/// - 源列表已带 `[1m]` 后缀 → 补回不带后缀的基础条目；
/// - 已知窗口不是 1M 的模型（如 MiniMax M2.7 = 204800）→ 不生成 `[1m]` 条目。
fn expand_account_model_one_m_variants(
    account_models: &[DexCatalogAccountModel],
) -> Vec<DexCatalogAccountModel> {
    let mut expanded = Vec::with_capacity(account_models.len());
    let mut seen = std::collections::HashSet::new();
    let mut push = |model: DexCatalogAccountModel| {
        let key = (
            model.account_id.clone(),
            model.endpoint_id.clone(),
            model.model.clone(),
        );
        if seen.insert(key) {
            expanded.push(model);
        }
    };
    for model in account_models {
        let trimmed = model.model.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.to_ascii_lowercase().ends_with("[1m]") {
            push(model.clone());
            let base = trimmed[..trimmed.len() - "[1m]".len()].to_string();
            if !base.trim().is_empty() {
                let mut twin = model.clone();
                twin.model = base;
                push(twin);
            }
        } else {
            push(model.clone());
            if known_model_context_window(&trimmed) == Some(1_000_000) {
                let mut twin = model.clone();
                twin.model = format!("{trimmed}[1m]");
                push(twin);
            }
        }
    }
    expanded
}

fn prepend_dex_account_catalog_models(
    models: &mut Vec<Value>,
    template_models: &[Value],
    account_models: &[DexCatalogAccountModel],
    context_window_override: Option<u32>,
    catalog_account_provider: &str,
) {
    if account_models.is_empty() {
        return;
    }
    let account_models = expand_account_model_one_m_variants(account_models);
    let fallback_template = catalog_model_template(template_models);
    let existing = models
        .iter()
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<std::collections::HashSet<_>>();
    let mut added = std::collections::HashSet::new();
    let mut account_entries = Vec::new();
    let mut last_account_id: Option<&str> = None;
    for account_model in &account_models {
        // 退役 Codex 模型（gpt-5.2 / gpt-5.3-codex 等）即使账号获取到也
        // 不允许进入模型选择器，避免上游/缓存残留再次显示。
        if is_retired_codex_model(&account_model.model) {
            continue;
        }
        if crate::providers::model_is_image_generation_only(&account_model.model) {
            continue;
        }
        // 每个账号只插一个分组占位条目（视觉分组标题，选中返回错误、不执行调用）
        if last_account_id != Some(account_model.account_id.as_str()) {
            let placeholder_slug =
                encode_dex_account_group_placeholder_slug(&account_model.account_id);
            if !existing.contains(&placeholder_slug) && added.insert(placeholder_slug.clone()) {
                let mut placeholder =
                    build_group_placeholder_entry(account_model, catalog_account_provider);
                placeholder["slug"] = Value::String(placeholder_slug.clone());
                placeholder["model"] = Value::String(placeholder_slug);
                account_entries.push(placeholder);
            }
            last_account_id = Some(account_model.account_id.as_str());
        }
        let slug = encode_dex_account_model_slug(
            &account_model.account_id,
            &account_model.endpoint_id,
            &account_model.model,
        );
        if existing.contains(&slug) || !added.insert(slug.clone()) {
            continue;
        }
        let mut model = catalog_model_template_for_account_model(
            template_models,
            &fallback_template,
            account_model,
        );
        // 分组占位条目（display_name=账号名）承担分组标题；组内模型只显示纯模型名，
        // 避免原生显示出现 “账号 / 模型” 与分组标题重复。
        let display_name = catalog_account_model_display_name(account_model);
        model["slug"] = Value::String(slug);
        model["display_name"] = Value::String(display_name);
        let is_one_m = account_model
            .model
            .trim()
            .to_ascii_lowercase()
            .ends_with("[1m]");
        // 端点名默认与协议标签相同（accounts.rs 里 endpoint.name =
        // endpoint.kind.label()），描述里避免重复显示。
        let endpoint_label = if account_model.endpoint_name.trim().is_empty()
            || account_model.endpoint_name.trim() == account_model.endpoint_kind.label()
        {
            account_model.endpoint_kind.label().to_string()
        } else {
            format!(
                "{} · {}",
                account_model.endpoint_name.trim(),
                account_model.endpoint_kind.label()
            )
        };
        let context_marker = if is_one_m { " · 1M 上下文" } else { "" };
        model["description"] = Value::String(format!(
            "DEX AI 账号直选：{} · {} · {}{}",
            account_model.account_name, endpoint_label, account_model.model, context_marker
        ));
        // codex-auto-review 是 Codex 内部自动审查模型：即使账号 known_models /
        // 获取上游模型回填了它，也必须在模型选择器里隐藏，不能变成可见模型。
        let is_auto_review = account_model.model.trim() == "codex-auto-review";
        model["visibility"] = Value::String(if is_auto_review { "hide" } else { "list" }.into());
        model["supported_in_api"] = Value::Bool(true);
        model["priority"] = Value::from(3);
        model["provider"] = Value::String(catalog_account_provider.to_string());
        // 账号模型窗口已按 (账号, endpoint, 模型) 计算好（含 endpoint/账号
        // 级 override、[1m] 后缀与已知窗口表），per-model 值优先；全局值仅作
        // 兜底，避免把同 catalog 里其他账号的小窗口模型一并覆盖成 1M。
        let window = account_model
            .context_window_override
            .or(context_window_override);
        if let Some(window) = window {
            model["context_window"] = Value::from(window);
            model["max_context_window"] = Value::from(window);
        } else {
            ensure_model_context_fields(&mut model);
            cap_account_model_max_context_window(&mut model);
        }
        prune_account_catalog_entry(&mut model);
        account_entries.push(model);
    }
    if !account_entries.is_empty() {
        models.splice(0..0, account_entries);
    }
}

/// 分组占位条目：display_name 即分组名（账号名），可选但选中时 deecodex
/// 返回错误提示，不执行实际调用。用于无 CDP 注入时 Codex 原生按 catalog
/// 显示分组标题。
fn build_group_placeholder_entry(
    account_model: &DexCatalogAccountModel,
    catalog_account_provider: &str,
) -> Value {
    serde_json::json!({
        "display_name": account_model.account_name,
        "displayName": account_model.account_name,
        "description": "模型分组占位项：请选择下方具体模型",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 3,
        "provider": catalog_account_provider,
    })
}

fn prepend_preserved_account_catalog_models(
    models: &mut Vec<Value>,
    preserved: &[Value],
    catalog_account_provider: &str,
) {
    if preserved.is_empty() {
        return;
    }
    let mut existing = models
        .iter()
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<std::collections::HashSet<_>>();
    let mut account_entries = Vec::new();
    for model in preserved {
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            continue;
        };
        if !slug.starts_with(DEX_ACCOUNT_MODEL_SLUG_PREFIX) || !existing.insert(slug.to_string()) {
            continue;
        }
        let mut model = model.clone();
        model["provider"] = Value::String(catalog_account_provider.to_string());
        account_entries.push(model);
    }
    if !account_entries.is_empty() {
        models.splice(0..0, account_entries);
    }
}

fn should_preserve_existing_account_catalog(
    data_dir: Option<&std::path::Path>,
    catalog_path: &std::path::Path,
) -> bool {
    if existing_catalog_account_entries(catalog_path).is_empty() {
        return false;
    }
    if let Some(data_dir) = data_dir {
        let accounts_path = crate::accounts::accounts_file_path(data_dir);
        if accounts_path.exists() {
            warn!(
                path = %catalog_path.display(),
                accounts_path = %accounts_path.display(),
                "本次账号库未生成 Codex Desktop 账号模型，继续保留已有模型目录直选项"
            );
        }
    }
    true
}

fn existing_catalog_account_entries(catalog_path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(catalog_path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|catalog| catalog.get("models").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(model_is_dex_account_model)
        .collect()
}

fn model_is_dex_account_model(model: &Value) -> bool {
    model
        .get("slug")
        .and_then(Value::as_str)
        .is_some_and(|slug| slug.starts_with(DEX_ACCOUNT_MODEL_SLUG_PREFIX))
}

fn prune_account_catalog_entry(model: &mut Value) {
    let Some(object) = model.as_object_mut() else {
        return;
    };
    object.retain(|key, _| account_catalog_field_is_supported(key));
}

fn account_catalog_field_is_supported(key: &str) -> bool {
    matches!(
        key,
        "slug"
            | "display_name"
            | "displayName"
            | "model"
            | "description"
            | "provider"
            | "hidden"
            | "visibility"
            | "supported_in_api"
            | "priority"
            | "context_window"
            | "max_context_window"
            | "auto_compact_token_limit"
            | "effective_context_window_percent"
            | "minimal_client_version"
            | "default_reasoning_level"
            | "defaultReasoningEffort"
            | "supported_reasoning_levels"
            | "supportedReasoningEfforts"
            | "shell_type"
            | "input_modalities"
            | "supports_image_detail_original"
            | "supports_parallel_tool_calls"
            | "supports_reasoning_summaries"
            | "supports_search_tool"
            | "support_verbosity"
            | "default_verbosity"
            | "default_reasoning_summary"
            | "reasoning_summary_format"
            | "apply_patch_tool_type"
            | "web_search_tool_type"
            | "truncation_policy"
            | "service_tiers"
            | "additional_speed_tiers"
            | "use_responses_lite"
            | "experimental_supported_tools"
            | "prefer_websockets"
            | "base_instructions"
            | "model_messages"
    )
}

fn ensure_model_catalog_schema(models: &mut [Value], schema_template: &Value) {
    for model in models {
        let is_account_model = model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| slug.starts_with(DEX_ACCOUNT_MODEL_SLUG_PREFIX));
        ensure_model_catalog_entry_schema(model, schema_template, is_account_model);
        // Codex 解析 model_catalog_json 要求每个条目带 shell_type，
        // 缺字段会导致整个 catalog 加载失败（dex 模型全部不显示）。
        if model.get("shell_type").is_none_or(Value::is_null) {
            model["shell_type"] = Value::String("shell_command".into());
        }
    }
}

fn ensure_model_catalog_entry_schema(
    model: &mut Value,
    schema_template: &Value,
    is_account_model: bool,
) {
    normalize_codex_desktop_model_catalog_aliases(model);

    if is_account_model {
        model["base_instructions"] = Value::String(DEX_ACCOUNT_MODEL_BASE_INSTRUCTIONS.into());
        model["model_messages"] = compact_dex_account_model_messages(schema_template);
    } else {
        fill_catalog_field_from_template(
            model,
            schema_template,
            "base_instructions",
            Value::String(DEX_ACCOUNT_MODEL_BASE_INSTRUCTIONS.into()),
        );
        fill_catalog_field_from_template(
            model,
            schema_template,
            "model_messages",
            compact_dex_account_model_messages(schema_template),
        );
    }

    fill_catalog_field_from_template(
        model,
        schema_template,
        "supports_reasoning_summaries",
        Value::Bool(true),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "default_reasoning_summary",
        Value::String("none".into()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "support_verbosity",
        Value::Bool(true),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "default_verbosity",
        Value::String("low".into()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "apply_patch_tool_type",
        Value::String("freeform".into()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "web_search_tool_type",
        Value::String("text_and_image".into()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "truncation_policy",
        serde_json::json!({ "mode": "tokens", "limit": 10000 }),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "supports_parallel_tool_calls",
        Value::Bool(true),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "supports_image_detail_original",
        Value::Bool(true),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "effective_context_window_percent",
        Value::from(95),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "experimental_supported_tools",
        Value::Array(Vec::new()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "supports_search_tool",
        Value::Bool(true),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "use_responses_lite",
        Value::Bool(false),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "input_modalities",
        serde_json::json!(["text", "image"]),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "additional_speed_tiers",
        Value::Array(Vec::new()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "service_tiers",
        Value::Array(Vec::new()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "reasoning_summary_format",
        Value::String("experimental".into()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "default_reasoning_level",
        Value::String("medium".into()),
    );
    fill_catalog_field_from_template(
        model,
        schema_template,
        "supported_reasoning_levels",
        serde_json::json!([
            {"effort": "low", "description": "Fast responses with lighter reasoning"},
            {"effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks"},
            {"effort": "high", "description": "Greater reasoning depth for complex problems"}
        ]),
    );
    ensure_codex_desktop_model_catalog_fields(model);
}

fn normalize_codex_desktop_model_catalog_aliases(model: &mut Value) {
    if model.get("slug").is_none_or(Value::is_null) {
        if let Some(slug) = model.get("model").and_then(Value::as_str) {
            model["slug"] = Value::String(slug.to_string());
        }
    }
    if model.get("display_name").is_none_or(Value::is_null) {
        if let Some(display_name) = model.get("displayName").and_then(Value::as_str) {
            model["display_name"] = Value::String(display_name.to_string());
        }
    }
    if model.get("visibility").is_none_or(Value::is_null) {
        if let Some(hidden) = model.get("hidden").and_then(Value::as_bool) {
            model["visibility"] = Value::String(if hidden { "hide" } else { "list" }.into());
        }
    }
    if model
        .get("default_reasoning_level")
        .is_none_or(Value::is_null)
    {
        if let Some(effort) = model.get("defaultReasoningEffort").and_then(Value::as_str) {
            model["default_reasoning_level"] = Value::String(effort.to_string());
        }
    }
    if model
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .is_none_or(|values| values.is_empty())
    {
        let levels = model
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .map(|efforts| {
                efforts
                    .iter()
                    .filter_map(|effort| {
                        let level = effort
                            .get("reasoningEffort")
                            .or_else(|| effort.get("effort"))
                            .and_then(Value::as_str)?
                            .trim();
                        if level.is_empty() {
                            return None;
                        }
                        let description = effort
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or(level);
                        Some(serde_json::json!({
                            "effort": level,
                            "description": description
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !levels.is_empty() {
            model["supported_reasoning_levels"] = Value::Array(levels);
        }
    }
}

fn ensure_codex_desktop_model_catalog_fields(model: &mut Value) {
    if model.get("slug").is_none_or(Value::is_null) {
        if let Some(slug) = model.get("model").and_then(Value::as_str) {
            model["slug"] = Value::String(slug.to_string());
        }
    }
    if model.get("model").is_none_or(Value::is_null) {
        if let Some(slug) = model.get("slug").and_then(Value::as_str) {
            model["model"] = Value::String(slug.to_string());
        }
    }
    if model.get("displayName").is_none_or(Value::is_null) {
        if let Some(display_name) = model.get("display_name").and_then(Value::as_str) {
            model["displayName"] = Value::String(display_name.to_string());
        }
    }
    if model.get("hidden").is_none_or(Value::is_null) {
        let hidden = model
            .get("visibility")
            .and_then(Value::as_str)
            .is_some_and(|visibility| visibility != "list");
        model["hidden"] = Value::Bool(hidden);
    }
    if model
        .get("defaultReasoningEffort")
        .is_none_or(Value::is_null)
    {
        if let Some(effort) = model.get("default_reasoning_level").and_then(Value::as_str) {
            model["defaultReasoningEffort"] = Value::String(effort.to_string());
        }
    }
    if model
        .get("supportedReasoningEfforts")
        .and_then(Value::as_array)
        .is_none_or(|values| values.is_empty())
    {
        let efforts = model
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| {
                        let effort = level
                            .get("effort")
                            .or_else(|| level.get("reasoningEffort"))
                            .and_then(Value::as_str)?
                            .trim();
                        if effort.is_empty() {
                            return None;
                        }
                        let description = level
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or(effort);
                        Some(serde_json::json!({
                            "reasoningEffort": effort,
                            "description": description
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !efforts.is_empty() {
            model["supportedReasoningEfforts"] = Value::Array(efforts);
        }
    }
}

fn fill_catalog_field_from_template(
    model: &mut Value,
    schema_template: &Value,
    key: &str,
    fallback: Value,
) {
    let should_fill = model.get(key).is_none_or(Value::is_null);
    if !should_fill {
        return;
    }
    model[key] = schema_template
        .get(key)
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(fallback);
}

fn compact_dex_account_model_messages(schema_template: &Value) -> Value {
    let instructions_variables = schema_template
        .get("model_messages")
        .and_then(|messages| messages.get("instructions_variables"))
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "personality_default": null,
                "personality_friendly": null,
                "personality_pragmatic": null
            })
        });
    serde_json::json!({
        "instructions_template": "{{ base_instructions }}",
        "instructions_variables": instructions_variables
    })
}

fn catalog_model_template_for_account_model(
    template_models: &[Value],
    fallback_template: &Value,
    account_model: &DexCatalogAccountModel,
) -> Value {
    let matched = template_models.iter().any(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| slug == account_model.model)
    }) || codex_registry_model_template(&account_model.model).is_some();
    let mut template = if matched {
        template_models
            .iter()
            .find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| slug == account_model.model)
            })
            .cloned()
            .or_else(|| codex_registry_model_template(&account_model.model))
            .unwrap_or_else(|| fallback_template.clone())
    } else {
        // 未知模型不继承 fallback 模板的上下文窗口（fallback 通常是 gpt-5.5 的
        // 272000）：把第三方模型误标成大窗口会让 Codex 跳过自动压缩，长上下文
        // 直接打到上游被 400 拒绝。窗口由 known_model_context_window 或调用方
        // 的显式 override 决定，都没有时保持缺失（Codex 用自身默认窗口）。
        let mut t = fallback_template.clone();
        if let Some(obj) = t.as_object_mut() {
            obj.remove("context_window");
            obj.remove("max_context_window");
        }
        t
    };
    if let Some(window) = known_model_context_window(&account_model.model) {
        template["context_window"] = Value::from(window);
        template["max_context_window"] = Value::from(window);
    }
    template
}

fn catalog_account_model_display_name(account_model: &DexCatalogAccountModel) -> String {
    let model = account_model.model.trim();
    if model.is_empty() {
        return provider_display_label(&account_model.provider);
    }
    friendly_upstream_model_name(&account_model.provider, model)
}

fn friendly_upstream_model_name(provider: &str, model: &str) -> String {
    let model = model.rsplit('/').next().unwrap_or(model).trim();
    let normalized = if let Some(rest) = model.strip_prefix("gpt-") {
        format!(
            "GPT-{}",
            rest.replace(['_', '-'], " ")
                .split_whitespace()
                .map(friendly_model_word)
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else {
        model
            .trim()
            .replace(['_', '-'], " ")
            .split_whitespace()
            .map(friendly_model_word)
            .collect::<Vec<_>>()
            .join(" ")
    };
    let normalized = normalize_model_brand(provider, &normalized);
    if normalized.is_empty() {
        provider_display_label(provider)
    } else {
        normalized
    }
}

fn friendly_model_word(word: &str) -> String {
    let lower = word.to_ascii_lowercase();
    match lower.as_str() {
        "gpt" => "GPT".into(),
        "glm" => "GLM".into(),
        "qwen" => "Qwen".into(),
        "kimi" => "Kimi".into(),
        "mimo" => "MiMo".into(),
        "minimax" => "MiniMax".into(),
        "deepseek" => "DeepSeek".into(),
        "longcat" => "LongCat".into(),
        "claude" => "Claude".into(),
        "gemini" => "Gemini".into(),
        "codex" => "Codex".into(),
        "vl" => "VL".into(),
        "omni" => "Omni".into(),
        "chat" => "Chat".into(),
        "reasoner" => "Reasoner".into(),
        "pro" => "Pro".into(),
        "flash" => "Flash".into(),
        "lite" => "Lite".into(),
        "preview" => "Preview".into(),
        _ if lower.starts_with('v')
            && lower.chars().nth(1).is_some_and(|ch| ch.is_ascii_digit()) =>
        {
            lower.to_ascii_uppercase()
        }
        _ if word.chars().all(|ch| ch.is_ascii_digit() || ch == '.') => word.into(),
        _ => {
            let mut chars = lower.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

fn normalize_model_brand(provider: &str, normalized: &str) -> String {
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return String::new();
    }
    let lower = normalized.to_ascii_lowercase();
    match provider {
        "deepseek" if !lower.starts_with("deepseek") => format!("DeepSeek {normalized}"),
        "longcat" if !lower.starts_with("longcat") => format!("LongCat {normalized}"),
        "minimax" if !lower.starts_with("minimax") => format!("MiniMax {normalized}"),
        "mimo" if !lower.starts_with("mimo") => format!("MiMo {normalized}"),
        "kimi" if !lower.starts_with("kimi") && !lower.starts_with("moonshot") => {
            format!("Kimi {normalized}")
        }
        "qwen" if !lower.starts_with("qwen") => format!("Qwen {normalized}"),
        "glm" if !lower.starts_with("glm") => format!("GLM {normalized}"),
        "codex" if lower.starts_with("gpt") => normalized.to_string(),
        "openai" if lower.starts_with("gpt") => normalized.to_string(),
        _ => normalized.to_string(),
    }
}

fn provider_display_label(provider: &str) -> String {
    let profile = crate::providers::profile_by_slug(provider);
    if profile.label.trim().is_empty() {
        provider.to_string()
    } else {
        profile.label
    }
}

fn catalog_model_template(models: &[Value]) -> Value {
    models
        .iter()
        .find(|model| {
            model
                .get("slug")
                .and_then(Value::as_str)
                .is_some_and(|slug| slug == "gpt-5.5")
        })
        .or_else(|| models.first())
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "slug": "gpt-5.5",
                "display_name": "GPT-5.5",
                "description": "DEX AI managed model",
                "default_reasoning_level": "medium",
                "supported_reasoning_levels": [
                    {"effort": "low", "description": "Fast responses with lighter reasoning"},
                    {"effort": "medium", "description": "Balances speed and reasoning depth"},
                    {"effort": "high", "description": "Greater reasoning depth"}
                ],
                "shell_type": "shell_command",
                "visibility": "list",
                "supported_in_api": true,
                "priority": 9,
                "context_window": 272000,
                "max_context_window": 272000
            })
        })
}

fn codex_registry_model_template(slug: &str) -> Option<Value> {
    let (
        display_name,
        description,
        context_window,
        max_context_window,
        default_reasoning_level,
        default_verbosity,
        supports_image_detail_original,
        reasoning_summary_format,
        default_reasoning_summary,
        web_search_tool_type,
        truncation_mode,
        minimal_client_version,
        priority,
        visibility,
        has_priority_service_tier,
        supports_max_effort,
        supports_ultra_effort,
    ) = match slug {
        "gpt-5.6-sol" => (
            "GPT-5.6-Sol",
            "Frontier agentic coding model for complex software work.",
            272_000,
            272_000,
            "medium",
            "low",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.145.0",
            0,
            "list",
            true,
            true,
            true,
        ),
        "gpt-5.6-terra" => (
            "GPT-5.6-Terra",
            "Balanced agentic coding model for everyday work.",
            272_000,
            272_000,
            "medium",
            "low",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.145.0",
            2,
            "list",
            true,
            true,
            true,
        ),
        "gpt-5.6-luna" => (
            "GPT-5.6-Luna",
            "Fast and affordable agentic coding model.",
            272_000,
            272_000,
            "medium",
            "low",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.145.0",
            3,
            "list",
            false,
            true,
            false,
        ),
        "gpt-5.5" => (
            "GPT-5.5",
            "Frontier model for complex coding, research, and real-world work.",
            272_000,
            272_000,
            "medium",
            "low",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.124.0",
            0,
            "list",
            true,
            false,
            false,
        ),
        "gpt-5.4" => (
            "gpt-5.4",
            "Strong model for everyday coding.",
            272_000,
            1_000_000,
            "xhigh",
            "low",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.98.0",
            2,
            "list",
            true,
            false,
            false,
        ),
        "gpt-5.4-mini" => (
            "GPT-5.4-Mini",
            "Small, fast, and cost-efficient model for simpler coding tasks.",
            272_000,
            272_000,
            "medium",
            "medium",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.98.0",
            4,
            "list",
            false,
            false,
            false,
        ),
        "gpt-5.3-codex" => (
            "gpt-5.3-codex",
            "Coding-optimized model.",
            272_000,
            272_000,
            "medium",
            "low",
            true,
            "experimental",
            "none",
            "text",
            "tokens",
            "0.98.0",
            6,
            "list",
            false,
            false,
            false,
        ),
        "gpt-5.2" => (
            "gpt-5.2",
            "Optimized for professional work and long-running agents.",
            272_000,
            272_000,
            "medium",
            "low",
            false,
            "none",
            "auto",
            "text",
            "bytes",
            "0.0.1",
            10,
            "list",
            false,
            false,
            false,
        ),
        "codex-auto-review" => (
            "Codex Auto Review",
            "Automatic approval review model for Codex.",
            272_000,
            1_000_000,
            "medium",
            "low",
            true,
            "experimental",
            "none",
            "text_and_image",
            "tokens",
            "0.98.0",
            29,
            "hide",
            false,
            false,
            false,
        ),
        _ => return None,
    };

    let service_tiers = if has_priority_service_tier {
        serde_json::json!([{
            "id": "priority",
            "name": "Fast",
            "description": "1.5x speed, increased usage"
        }])
    } else {
        serde_json::json!([])
    };
    let mut supported_reasoning_levels = vec![
        serde_json::json!({
            "effort": "low",
            "description": "Fast responses with lighter reasoning"
        }),
        serde_json::json!({
            "effort": "medium",
            "description": "Balances speed and reasoning depth for everyday tasks"
        }),
        serde_json::json!({
            "effort": "high",
            "description": "Greater reasoning depth for complex problems"
        }),
        serde_json::json!({
            "effort": "xhigh",
            "description": "Extra high reasoning depth for complex problems"
        }),
    ];
    if supports_max_effort {
        supported_reasoning_levels.push(serde_json::json!({
            "effort": "max",
            "description": "Maximum reasoning depth for demanding agentic work"
        }));
    }
    if supports_ultra_effort {
        supported_reasoning_levels.push(serde_json::json!({
            "effort": "ultra",
            "description": "Ultra reasoning depth for the hardest long-running tasks"
        }));
    }

    Some(serde_json::json!({
        "prefer_websockets": true,
        "support_verbosity": true,
        "default_verbosity": default_verbosity,
        "apply_patch_tool_type": "freeform",
        "web_search_tool_type": web_search_tool_type,
        "input_modalities": ["text", "image"],
        "supports_image_detail_original": supports_image_detail_original,
        "truncation_policy": {
            "mode": truncation_mode,
            "limit": 10000
        },
        "supports_parallel_tool_calls": true,
        "context_window": context_window,
        "max_context_window": max_context_window,
        "auto_compact_token_limit": null,
        "reasoning_summary_format": reasoning_summary_format,
        "default_reasoning_summary": default_reasoning_summary,
        "slug": slug,
        "display_name": display_name,
        "description": description,
        "default_reasoning_level": default_reasoning_level,
        "supported_reasoning_levels": supported_reasoning_levels,
        "shell_type": "shell_command",
        "visibility": visibility,
        "minimal_client_version": minimal_client_version,
        "supported_in_api": true,
        "upgrade": null,
        "priority": priority,
        "service_tiers": service_tiers
    }))
}

fn ensure_model_context_fields(model: &mut Value) {
    if model.get("context_window").is_none() {
        if let Some(max_context_window) = model.get("max_context_window").cloned() {
            model["context_window"] = max_context_window;
        }
    }
    if model.get("max_context_window").is_none() {
        if let Some(context_window) = model.get("context_window").cloned() {
            model["max_context_window"] = context_window;
        }
    }
}

fn cap_account_model_max_context_window(model: &mut Value) {
    let Some(context_window) = model.get("context_window").and_then(Value::as_u64) else {
        return;
    };
    let max_context_window = model
        .get("max_context_window")
        .and_then(Value::as_u64)
        .unwrap_or(context_window);
    if max_context_window > context_window {
        model["max_context_window"] = Value::from(context_window);
    }
}

fn resolve_model_context_window(catalog: &Value, active_model: &str) -> Option<i64> {
    let models = catalog.get("models")?.as_array()?;
    let active_model_tail = active_model.rsplit('/').next().unwrap_or(active_model);
    let model = models.iter().find(|model| {
        model
            .get("slug")
            .and_then(|slug| slug.as_str())
            .is_some_and(|slug| slug == active_model || slug == active_model_tail)
    })?;
    let context_window = model
        .get("context_window")
        .or_else(|| model.get("max_context_window"))
        .and_then(|window| window.as_u64())?;
    Some(context_window.min(i64::MAX as u64) as i64)
}

fn read_active_codex_model(path: &std::path::Path) -> Option<String> {
    let content = read_config_file(path).ok()?;
    let doc: toml_edit::DocumentMut = content.parse().ok()?;
    doc.get("model")
        .and_then(|model| model.as_str())
        .map(ToString::to_string)
}

/// 读取当前 Codex config.toml 中的主模型（dex 账号直选 slug）。
///
/// 仅当 config.toml 的 `model` 是 dex 账号直选 slug（如
/// `dexacct.<account>/<endpoint>/<model>`）时返回，否则返回 None，
/// 避免把用户手动填写的普通模型名当作主模型用于改写。
pub fn read_active_codex_model_slug() -> Option<String> {
    let path = codex_config_path()?;
    let slug = read_active_codex_model(&path)?;
    decode_dex_account_model_slug(&slug)?;
    Some(slug)
}

/// 从 Codex 的 config.toml 中提取非 deecodex 的 provider 配置，
/// 用于首次启动时将 Codex 原有账号迁移到 deecodex。
/// 返回 None 表示没有找到可导入的账号。
#[allow(dead_code)]
pub fn extract_account_from_codex_config() -> Option<crate::accounts::Account> {
    use crate::accounts::{generate_id, guess_provider, now_secs, Account};
    use std::collections::HashMap;

    let path = codex_config_path()?;
    if !path.exists() {
        tracing::info!("Codex config.toml 不存在，跳过账号导入");
        return None;
    }

    let content = read_config_file(&path).ok()?;
    let doc: toml_edit::DocumentMut = content.parse().ok()?;

    let providers = doc.get("model_providers")?.as_table()?;

    for (key, value) in providers.iter() {
        // 跳过 DEX AI 自身管理的本地代理 provider。
        if [
            DEECODEX_PROVIDER,
            DEECODEX_CLI_PROVIDER,
            DEECODEX_DESKTOP_PROVIDER,
            DEX_ROUTER_PROVIDER,
        ]
        .contains(&key)
        {
            continue;
        }

        let base_url = value.get("base_url")?.as_str()?.to_string();

        // 跳过本地地址
        if base_url.contains("127.0.0.1") || base_url.contains("localhost") {
            continue;
        }

        let api_key = value
            .get("api_key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let provider = guess_provider(&base_url).to_string();

        let name = if key == provider {
            format!("Codex 导入 - {}", provider)
        } else {
            format!("Codex 导入 - {}", key)
        };

        let account = Account {
            id: generate_id(),
            name,
            provider: provider.to_string(),
            client_kind: Default::default(),
            client_surface: Default::default(),
            wire_protocol: Default::default(),
            upstream: base_url,
            api_key,
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
            from_codex_config: true,
            balance_url: String::new(),
            created_at: now_secs(),
            updated_at: now_secs(),
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: crate::providers::provider_options_for_slug(&provider),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: false,
            dev_pipeline_trigger_mode: Default::default(),
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: Default::default(),
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        };

        tracing::info!(
            "从 Codex config.toml 导入账号: provider={}, base_url={}",
            provider,
            account.upstream
        );
        return Some(account);
    }

    // 只有 deecodex 自身，没有第三方 provider
    tracing::info!("Codex config.toml 中未找到可导入的第三方 provider");
    None
}

/// 检测并修复 Codex config.toml 中的已知错误值。
/// 返回修复的问题数量。0 表示没有发现问题。
pub fn fix() -> u32 {
    let Some(path) = codex_config_path() else {
        return 0;
    };
    if !path.exists() {
        return 0;
    }

    match do_fix(&path) {
        Ok(count) => {
            if count > 0 {
                info!(
                    "已修复 Codex config.toml 中的 {} 处已知问题 (路径: {})",
                    count,
                    path.display()
                );
            }
            count
        }
        Err(e) => {
            warn!(
                "修复 Codex config.toml 失败 (路径: {}): {e}",
                path.display()
            );
            0
        }
    }
}

fn do_fix(path: &std::path::Path) -> Result<u32> {
    let content = read_config_file(path)?;
    let lines: Vec<&str> = content.lines().collect();
    let mut fixes = 0u32;

    let mut remove_line_indices: std::collections::BTreeSet<usize> =
        std::collections::BTreeSet::new();

    // 1. 检测重复的 DEX 管理 provider 节（行级修复，先于 toml_edit）
    for section_name in ["model_providers.deecodex", "model_providers.dex_router"] {
        let custom_sections = find_section_ranges(&lines, section_name);
        if custom_sections.len() > 1 {
            for (start, end) in &custom_sections[..custom_sections.len() - 1] {
                for i in *start..*end {
                    remove_line_indices.insert(i);
                }
            }
            fixes += (custom_sections.len() - 1) as u32;
            warn!(
                "Codex config.toml: 发现 {} 个重复的 [{}] 节，保留最后一份",
                custom_sections.len(),
                section_name
            );
        }
    }

    // 3. 检测 [windows] sandbox 问题
    let windows_section = find_section_range(&lines, "windows");
    if let Some((ws_start, ws_end)) = windows_section {
        for (i, line) in lines
            .iter()
            .enumerate()
            .skip(ws_start)
            .take(ws_end - ws_start)
        {
            let trimmed = line.trim();
            if trimmed == "sandbox = \"unelevated\"" || trimmed == "sandbox = 'unelevated'" {
                remove_line_indices.insert(i);
                fixes += 1;
                warn!(
                    "Codex config.toml: 删除 [windows] sandbox = \"unelevated\" (恢复默认 elevated)"
                );
            } else if trimmed == "sandbox = \"off\"" || trimmed == "sandbox = 'off'" {
                warn!(
                    "Codex config.toml: [windows] sandbox = \"off\" — 沙盒已完全禁用，如需启用请手动修改"
                );
            }
        }
    }

    if !remove_line_indices.is_empty() {
        let mut new_content = String::new();
        for (i, line) in lines.iter().enumerate() {
            if remove_line_indices.contains(&i) {
                continue;
            }
            new_content.push_str(line);
            new_content.push('\n');
        }
        // 清理末尾多余空行（保留一个）
        while new_content.ends_with("\n\n") {
            new_content.pop();
        }
        std::fs::write(path, new_content)?;
    }

    Ok(fixes)
}

/// 查找 TOML 文件中指定 section 的所有出现位置及其范围。
/// 返回 Vec<(start_line, end_line)>，end_line 为排他边界。
fn find_section_ranges(lines: &[&str], section_name: &str) -> Vec<(usize, usize)> {
    let header = format!("[{}]", section_name);
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == header.as_str() {
            let start = i;
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if t.starts_with('[') && !t.starts_with("[[") {
                    break;
                }
                i += 1;
            }
            ranges.push((start, i));
        } else {
            i += 1;
        }
    }
    ranges
}

/// 查找单个 section 的范围。返回 None 如果未找到。
fn find_section_range(lines: &[&str], section_name: &str) -> Option<(usize, usize)> {
    find_section_ranges(lines, section_name).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn write_temp_config(content: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("deecodex-codex-config-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn cleanup(path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn group_placeholder_slug_roundtrip_and_detection() {
        let slug = encode_dex_account_group_placeholder_slug("acct_123");
        assert!(slug.starts_with("dexacct.group."), "占位 slug 前缀",);
        assert!(is_dex_account_group_placeholder_slug(&slug));
        assert!(is_dex_account_group_placeholder_slug("dexacct.group.abc"));
        assert!(!is_dex_account_group_placeholder_slug(
            "dexacct.abc.def.ghi"
        ));
        assert!(!is_dex_account_group_placeholder_slug("gpt-5.5"));
        // 占位 slug 不应被当作真实账号模型 slug 解析
        assert!(decode_dex_account_model_slug(&slug).is_none());
    }

    #[test]
    fn do_remove_clears_dex_model_and_provider_config() {
        let content = r#"
model_provider = "dex_ai"
model = "dexacct.abc.def.ghi"
model_catalog_json = "/Users/test/.codex/models_deecodex.json"
model_context_window = 1000000

[model_providers.dex_ai]
base_url = "http://127.0.0.1:8860/codex-router/v1"
name = "dex_ai"
requires_openai_auth = true
api_key = ""
wire_api = "responses"
"#;
        let path = write_temp_config(content);
        let changed = do_remove(&path).unwrap();
        assert!(changed, "应检测到 deecodex 配置并清理");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            !after.contains("dexacct"),
            "model = dexacct.* 应被移除: {after}"
        );
        assert!(
            !after.contains("model_provider"),
            "model_provider 应被移除: {after}"
        );
        assert!(
            !after.contains("model_catalog_json"),
            "model_catalog_json 应被移除: {after}"
        );
        assert!(
            !after.contains("model_providers"),
            "model_providers 应被移除: {after}"
        );
        assert!(
            !after.contains("model_context_window"),
            "model_context_window 应被移除: {after}"
        );
        cleanup(&path);
    }

    #[test]
    fn ensure_model_catalog_schema_fills_missing_shell_type() {
        let schema = catalog_model_template(&[]);
        let mut models = vec![
            serde_json::json!({ "slug": "dexacct.abc", "display_name": "A / B" }),
            serde_json::json!({ "slug": "gpt-5.5", "display_name": "GPT-5.5", "shell_type": "shell_command" }),
        ];
        ensure_model_catalog_schema(&mut models, &schema);
        for model in &models {
            assert_eq!(
                model.get("shell_type").and_then(Value::as_str),
                Some("shell_command"),
                "每个 catalog 条目都必须有 shell_type: {model}"
            );
        }
    }

    #[test]
    fn inject_without_override_still_writes_model_catalog() {
        let path = write_temp_config("");
        let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
        let changed = do_inject(
            &path,
            "127.0.0.1",
            4446,
            None,
            Some(&catalog_path),
            Some(272_000),
            crate::config::CODEX_ROUTER_MODE_API,
            false,
        )
        .unwrap();
        assert!(changed);

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(fixed.contains("model_provider = \"dex_ai\""));
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert!(doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_ai"))
            .is_some());
        assert!(doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_router"))
            .is_none());
        assert!(fixed.contains("model_catalog_json"));
        assert!(fixed.contains(&catalog_path.to_string_lossy().to_string()));
        assert!(fixed.contains("model_context_window = 272000"));
        assert!(!fixed.contains("model_auto_compact_token_limit"));
        cleanup(&path);
    }

    #[test]
    fn inject_with_override_writes_catalog_and_compact_limit() {
        let path = write_temp_config("");
        let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
        do_inject(
            &path,
            "127.0.0.1",
            4446,
            Some(1_000_000),
            Some(&catalog_path),
            Some(1_000_000),
            crate::config::CODEX_ROUTER_MODE_API,
            false,
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(fixed.contains("model_catalog_json"));
        assert!(fixed.contains("model_context_window = 1000000"));
        assert!(fixed.contains("model_auto_compact_token_limit = 900000"));
        cleanup(&path);
    }

    #[test]
    fn inject_smart_mode_writes_dex_router_provider() {
        let path = write_temp_config("");
        let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
        do_inject(
            &path,
            "127.0.0.1",
            4446,
            None,
            Some(&catalog_path),
            Some(272_000),
            crate::config::CODEX_ROUTER_MODE_SMART,
            false,
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        // 智能路由模式：model_provider 统一是 `dex_ai`，
        // 差异落在 `[model_providers.dex_ai]` 块的 base_url 上（走 /codex-router/v1）。
        assert_eq!(
            doc.get("model_provider").and_then(|value| value.as_str()),
            Some("dex_ai")
        );
        let router = doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_ai"))
            .unwrap();
        assert_eq!(
            router.get("base_url").and_then(|value| value.as_str()),
            Some("http://127.0.0.1:4446/codex-router/v1")
        );
        assert_eq!(
            router
                .get("requires_openai_auth")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
        cleanup(&path);
    }

    #[test]
    fn inject_relay_mode_respects_login_state_auth_requirement() {
        for (relay_requires_openai_auth, expected_auth) in [(false, false), (true, true)] {
            let path = write_temp_config("");
            let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
            do_inject(
                &path,
                "127.0.0.1",
                4446,
                None,
                Some(&catalog_path),
                Some(272_000),
                crate::config::CODEX_ROUTER_MODE_RELAY,
                relay_requires_openai_auth,
            )
            .unwrap();

            let fixed = std::fs::read_to_string(&path).unwrap();
            let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
            let provider = doc
                .get("model_providers")
                .and_then(|providers| providers.get("dex_ai"))
                .unwrap();
            assert_eq!(
                provider.get("base_url").and_then(|value| value.as_str()),
                Some("http://127.0.0.1:4446/codex-relay/desktop/v1")
            );
            assert_eq!(
                provider
                    .get("requires_openai_auth")
                    .and_then(|value| value.as_bool()),
                Some(expected_auth)
            );
            cleanup(&path);
        }
    }

    #[test]
    fn inject_api_mode_preserves_dex_account_model_selection() {
        let selected = encode_dex_account_model_slug("acct_1", "endpoint_1", "deepseek-v4-pro");
        let path = write_temp_config(&format!("model = \"{selected}\"\n"));
        let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
        do_inject(
            &path,
            "127.0.0.1",
            4446,
            None,
            Some(&catalog_path),
            Some(272_000),
            crate::config::CODEX_ROUTER_MODE_API,
            false,
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert_eq!(
            doc.get("model").and_then(|value| value.as_str()),
            Some(selected.as_str())
        );
        assert_eq!(
            doc.get("model_provider").and_then(|value| value.as_str()),
            Some("dex_ai")
        );
        cleanup(&path);
    }

    #[test]
    fn inject_api_mode_removes_smart_router_provider() {
        let account_slug = encode_dex_account_model_slug("acct_1", "ep_1", "deepseek-v4-pro");
        let path = write_temp_config(&format!(
            r#"model = "{account_slug}"
model_provider = "dex_router"

[model_providers.dex_router]
base_url = "http://127.0.0.1:4446/codex-router/v1"
name = "dex_router"
requires_openai_auth = true
wire_api = "responses"
"#
        ));
        let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
        do_inject(
            &path,
            "127.0.0.1",
            4446,
            None,
            Some(&catalog_path),
            Some(272_000),
            crate::config::CODEX_ROUTER_MODE_API,
            false,
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert_eq!(
            doc.get("model").and_then(|value| value.as_str()),
            Some(account_slug.as_str())
        );
        // API 模式下注入后：`model_provider` 收敛到 `dex_ai`，
        // 旧的 `dex_router` provider 块被移除，`[model_providers.dex_ai]` 接管路由。
        assert_eq!(
            doc.get("model_provider").and_then(|value| value.as_str()),
            Some("dex_ai")
        );
        assert!(doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_router"))
            .is_none());
        let provider = doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_ai"))
            .unwrap();
        assert_eq!(
            provider.get("base_url").and_then(|value| value.as_str()),
            Some("http://127.0.0.1:4446/codex-desktop/v1")
        );
        assert_eq!(
            provider
                .get("requires_openai_auth")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        cleanup(&path);
    }

    #[test]
    fn guard_check_accepts_api_mode_integration() {
        let content = r#"
model_provider = "dex_ai"
model_catalog_json = "/Users/test/.codex/models_deecodex.json"

[model_providers.dex_ai]
base_url = "http://127.0.0.1:4446/codex-desktop/v1"
requires_openai_auth = false
wire_api = "responses"
"#;
        let reasons = codex_integration_restore_reasons_for_content(
            content,
            "127.0.0.1",
            4446,
            crate::config::CODEX_ROUTER_MODE_API,
            false,
        );
        assert!(reasons.is_empty(), "{reasons:?}");
    }

    #[test]
    fn guard_check_accepts_smart_mode_integration() {
        let content = r#"
model_provider = "dex_ai"
model_catalog_json = "/Users/test/.codex/models_deecodex.json"

[model_providers.dex_ai]
base_url = "http://127.0.0.1:4556/codex-router/v1"
requires_openai_auth = true
wire_api = "responses"
"#;
        let reasons = codex_integration_restore_reasons_for_content(
            content,
            "127.0.0.1",
            4556,
            crate::config::CODEX_ROUTER_MODE_SMART,
            false,
        );
        assert!(reasons.is_empty(), "{reasons:?}");
    }

    #[test]
    fn guard_check_respects_relay_login_state_auth_requirement() {
        let content = r#"
model_provider = "dex_ai"
model_catalog_json = "/Users/test/.codex/models_deecodex.json"

[model_providers.dex_ai]
base_url = "http://127.0.0.1:4446/codex-relay/desktop/v1"
requires_openai_auth = true
wire_api = "responses"
"#;
        let logged_in_reasons = codex_integration_restore_reasons_for_content(
            content,
            "127.0.0.1",
            4446,
            crate::config::CODEX_ROUTER_MODE_RELAY,
            true,
        );
        assert!(logged_in_reasons.is_empty(), "{logged_in_reasons:?}");

        let api_mode_reasons = codex_integration_restore_reasons_for_content(
            content,
            "127.0.0.1",
            4446,
            crate::config::CODEX_ROUTER_MODE_RELAY,
            false,
        );
        assert!(
            api_mode_reasons
                .iter()
                .any(|reason| reason == "provider_auth_mismatch"),
            "{api_mode_reasons:?}"
        );
    }

    #[test]
    fn relay_auth_detection_uses_saved_desktop_official_account() {
        let dir =
            std::env::temp_dir().join(format!("deecodex-relay-auth-{}", Uuid::new_v4().simple()));
        let data_dir = dir.join(".deecodex");
        std::fs::create_dir_all(&data_dir).unwrap();
        let account: crate::accounts::Account = serde_json::from_value(serde_json::json!({
            "id": "acct_codex_official",
            "name": "Codex 官方",
            "provider": "codex",
            "client_kind": "codex",
            "client_surface": "desktop",
            "wire_protocol": "responses",
            "upstream": "https://chatgpt.com/backend-api/codex",
            "api_key": "access-token",
            "auth_mode": "oauth",
            "endpoints": [{
                "id": "endpoint_codex_official",
                "name": "Codex 官方",
                "kind": "codex_official",
                "base_url": "https://chatgpt.com/backend-api/codex"
            }]
        }))
        .unwrap();
        let store = crate::accounts::AccountStore {
            version: crate::accounts::ACCOUNT_STORE_VERSION,
            accounts: vec![account],
            active_id: None,
            active_account_id: None,
            active_endpoint_id: None,
            active_by_surface: std::collections::HashMap::new(),
        };
        crate::accounts::save_accounts(&data_dir, &store).unwrap();

        assert!(relay_requires_openai_auth_for_data_dir(&data_dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codex_auth_json_login_state_accepts_default_auth_shapes() {
        assert!(codex_auth_json_has_login_state(&serde_json::json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "access_token": "access-file",
                "refresh_token": "refresh-file"
            }
        })));
        assert!(!codex_auth_json_has_login_state(&serde_json::json!({
            "type": "other",
            "access_token": "access-file"
        })));
    }

    #[test]
    fn guard_check_detects_legacy_provider_name() {
        // 历史版本的 `model_provider = "deecodex_desktop"` 在新版本下应被识别为不匹配。
        let content = r#"
model_provider = "deecodex_desktop"
model_catalog_json = "/Users/test/.codex/models_deecodex.json"

[model_providers.dex_ai]
base_url = "http://127.0.0.1:4446/codex-desktop/v1"
requires_openai_auth = false
wire_api = "responses"
"#;
        let reasons = codex_integration_restore_reasons_for_content(
            content,
            "127.0.0.1",
            4446,
            crate::config::CODEX_ROUTER_MODE_API,
            false,
        );
        assert!(reasons
            .iter()
            .any(|reason| reason == "model_provider:deecodex_desktop"));
    }

    #[test]
    fn guard_check_distinguishes_api_and_smart_modes() {
        // API 模式配置文件（走 /codex-desktop/v1）在智能路由模式下应被识别为不匹配：
        // base_url 不一致会导致 requires_openai_auth 与 wire_api 校验失败。
        let content = r#"
model_provider = "dex_ai"
model_catalog_json = "/Users/test/.codex/models_deecodex.json"

[model_providers.dex_ai]
base_url = "http://127.0.0.1:4446/codex-desktop/v1"
requires_openai_auth = false
wire_api = "responses"
"#;
        let reasons = codex_integration_restore_reasons_for_content(
            content,
            "127.0.0.1",
            4446,
            crate::config::CODEX_ROUTER_MODE_SMART,
            false,
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason == "provider_auth_mismatch"),
            "{reasons:?}"
        );
    }

    #[test]
    fn api_and_smart_modes_include_account_models_in_catalog() {
        assert!(include_account_models_in_catalog_for_mode(
            crate::config::CODEX_ROUTER_MODE_API
        ));
        assert!(include_account_models_in_catalog_for_mode(
            crate::config::CODEX_ROUTER_MODE_SMART
        ));
        assert!(!include_account_models_in_catalog_for_mode(
            crate::config::CODEX_ROUTER_MODE_RELAY
        ));
    }

    #[test]
    fn context_catalog_preserves_cached_window_and_applies_registry_floor() {
        let input = serde_json::json!({
            "fetched_at": "ignored",
            "models": [
                {
                    "slug": "gpt-5.5",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "effective_context_window_percent": 95
                },
                {
                    "slug": "gpt-5.4-mini",
                    "max_context_window": 128000
                }
            ]
        });

        let output = build_context_catalog(input, None, &[], &[], DEX_ROUTER_PROVIDER).unwrap();
        assert!(output.get("fetched_at").is_none());
        let models = output["models"].as_array().unwrap();
        // 官方 GPT 模板被彻底移除：输出目录不含 gpt-5.5 / gpt-5.4-mini，
        // 仅保留 codex-auto-review 内部辅助模型。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4-mini")));
        assert!(models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("codex-auto-review")));
        assert_eq!(resolve_model_context_window(&output, "gpt-5.5"), None);
    }

    #[test]
    fn context_catalog_does_not_override_native_models_with_global_window() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "context_window": 272000,
                    "max_context_window": 1000000
                }
            ]
        });

        let output =
            build_context_catalog(input, Some(2_000_000), &[], &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        // 官方 GPT 模板已彻底移除：目录不再含 gpt-5.5，全局 override 不再作用于任何原生模型。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
    }

    #[test]
    fn context_catalog_account_model_uses_endpoint_override_window() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![DexCatalogAccountModel {
            account_id: "acct_1".into(),
            endpoint_id: "ep_1".into(),
            account_name: "测试账号".into(),
            endpoint_name: "OpenAI Responses".into(),
            endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
            provider: "deepseek".into(),
            model: "deepseek-v4-pro".into(),
            context_window_override: Some(1_000_000),
        }];

        let output = build_context_catalog(
            input,
            Some(1_000_000),
            &account_models,
            &[],
            DEX_ROUTER_PROVIDER,
        )
        .unwrap();
        let models = output["models"].as_array().unwrap();
        let account_entry = models
            .iter()
            .find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == "deepseek-v4-pro")
            })
            .expect("missing deepseek account model");
        assert_eq!(account_entry["context_window"], 1_000_000);
        assert_eq!(account_entry["max_context_window"], 1_000_000);
        // 官方 gpt-5.5 模板已从目录移除。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
    }

    #[test]
    fn context_catalog_applies_cliproxy_codex_registry_metadata() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.4",
                    "display_name": "gpt-5.4",
                    "context_window": 272000,
                    "max_context_window": 272000
                },
                {
                    "slug": "gpt-5.3-codex-spark",
                    "display_name": "gpt-5.3-codex-spark",
                    "context_window": 272000,
                    "max_context_window": 272000
                }
            ]
        });

        let output = build_context_catalog(input, None, &[], &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        assert!(!models.iter().any(|model| {
            model.get("slug").and_then(Value::as_str) == Some("gpt-5.3-codex-spark")
        }));
        // 官方 GPT 模板（含 gpt-5.4）已从输出目录移除，仅保留内部辅助模型。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
        assert!(models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("codex-auto-review")));
    }

    #[test]
    fn dex_account_model_slug_roundtrips_special_model_names() {
        let slug = encode_dex_account_model_slug("acct.1", "endpoint/2", "vendor/model:latest");
        let decoded = decode_dex_account_model_slug(&slug).unwrap();
        assert_eq!(decoded.account_id, "acct.1");
        assert_eq!(decoded.endpoint_id, "endpoint/2");
        assert_eq!(decoded.model, "vendor/model:latest");
    }

    fn basic_catalog_input() -> Value {
        serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        })
    }

    fn deepseek_catalog_account_model() -> DexCatalogAccountModel {
        DexCatalogAccountModel {
            account_id: "acct_deepseek".into(),
            endpoint_id: "ep_deepseek".into(),
            account_name: "DeepSeek 桌面版账号".into(),
            endpoint_name: "OpenAI Chat".into(),
            endpoint_kind: crate::accounts::EndpointKind::OpenAiChat,
            provider: "deepseek".into(),
            model: "deepseek-v4-pro".into(),
            context_window_override: None,
        }
    }

    #[test]
    fn context_catalog_account_provider_uses_openai_for_both_modes() {
        // 两种模式现在共用 Codex 默认 provider key `openai`，
        // 因此 catalog 中账号模型的 provider 字段在两种模式下应该一致。
        let account_models = vec![deepseek_catalog_account_model()];
        let api_catalog = build_context_catalog(
            basic_catalog_input(),
            None,
            &account_models,
            &[],
            managed_model_provider_for_mode(crate::config::CODEX_ROUTER_MODE_API),
        )
        .unwrap();
        let smart_catalog = build_context_catalog(
            basic_catalog_input(),
            None,
            &account_models,
            &[],
            managed_model_provider_for_mode(crate::config::CODEX_ROUTER_MODE_SMART),
        )
        .unwrap();

        let account_provider = |catalog: &Value| {
            catalog["models"]
                .as_array()
                .unwrap()
                .iter()
                .find(|model| model_is_dex_account_model(model))
                .and_then(|model| model.get("provider"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        };
        assert_eq!(account_provider(&api_catalog).as_deref(), Some("dex_ai"));
        assert_eq!(account_provider(&smart_catalog).as_deref(), Some("dex_ai"));
    }

    #[test]
    fn context_catalog_appends_dex_account_models() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "base_instructions": "large prompt that should not be copied into account entries",
                    "model_messages": [{"role": "system", "content": "large prompt"}],
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "acct_1".into(),
                endpoint_id: "ep_1".into(),
                account_name: "测试账号".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "openai".into(),
                model: "gpt-5.5-proxy".into(),
                context_window_override: Some(128_000),
            },
            DexCatalogAccountModel {
                account_id: "acct_2".into(),
                endpoint_id: "ep_2".into(),
                account_name: "DeepSeek 账号".into(),
                endpoint_name: "OpenAI Chat".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiChat,
                provider: "deepseek".into(),
                model: "deepseek-v4-pro".into(),
                context_window_override: None,
            },
        ];

        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        assert!(models.len() > 2);
        let account_entry = models
            .iter()
            .find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == "gpt-5.5-proxy")
            })
            .expect("missing account model");
        assert_eq!(account_entry["display_name"], "GPT-5.5 Proxy");
        assert_eq!(account_entry["model"], account_entry["slug"]);
        assert_eq!(account_entry["displayName"], "GPT-5.5 Proxy");
        assert_eq!(account_entry["hidden"], false);
        assert_eq!(account_entry["defaultReasoningEffort"], "medium");
        assert!(account_entry["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|effort| effort["reasoningEffort"] == "medium"));
        assert_eq!(account_entry["context_window"], 128_000);
        assert_eq!(account_entry["max_context_window"], 128_000);
        assert_eq!(
            account_entry["base_instructions"],
            DEX_ACCOUNT_MODEL_BASE_INSTRUCTIONS
        );
        assert_eq!(
            account_entry["model_messages"]["instructions_template"],
            "{{ base_instructions }}"
        );
        assert_eq!(
            decode_dex_account_model_slug(account_entry["slug"].as_str().unwrap())
                .unwrap()
                .model,
            "gpt-5.5-proxy"
        );
        let deepseek_entry = models
            .iter()
            .find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == "deepseek-v4-pro")
            })
            .expect("missing deepseek account model");
        assert_eq!(deepseek_entry["display_name"], "DeepSeek V4 Pro");
        assert_eq!(deepseek_entry["model"], deepseek_entry["slug"]);
        assert_eq!(deepseek_entry["displayName"], "DeepSeek V4 Pro");
        assert_eq!(deepseek_entry["hidden"], false);
        // deepseek-v4-pro 真实上下文窗口为 1M（官方），不再继承模板 272000。
        assert_eq!(deepseek_entry["context_window"], 1_000_000);
        assert_eq!(deepseek_entry["max_context_window"], 1_000_000);
        assert_eq!(
            deepseek_entry["model_messages"]["instructions_template"],
            "{{ base_instructions }}"
        );
        // 官方 GPT 模板已从目录移除，账号直选条目保留。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
    }

    #[test]
    fn catalog_account_models_emit_one_m_and_base_variants() {
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "acct_deepseek".into(),
                endpoint_id: "ep_deepseek".into(),
                account_name: "DeepSeek".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_minimax".into(),
                endpoint_id: "ep_minimax".into(),
                account_name: "MMX".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "minimax".into(),
                model: "MiniMax-M2.7".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_minimax".into(),
                endpoint_id: "ep_minimax".into(),
                account_name: "MMX".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "minimax".into(),
                model: "MiniMax-M3[1m]".into(),
                context_window_override: None,
            },
        ];

        let output = build_context_catalog(
            basic_catalog_input(),
            None,
            &account_models,
            &[],
            DEX_ROUTER_PROVIDER,
        )
        .unwrap();
        let models = output["models"].as_array().unwrap();
        let find = |model_name: &str| {
            models.iter().find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == model_name)
            })
        };

        // 天生 1M 的 deepseek-v4-flash：基础版与 [1m] 版两条并存，名字可区分。
        let base = find("deepseek-v4-flash").expect("missing base variant");
        let one_m = find("deepseek-v4-flash[1m]").expect("missing [1m] variant");
        assert_eq!(base["display_name"], "DeepSeek V4 Flash");
        assert_eq!(one_m["display_name"], "DeepSeek V4 Flash[1m]");
        assert_eq!(base["context_window"], 1_000_000);
        assert_eq!(one_m["context_window"], 1_000_000);
        assert_eq!(one_m["max_context_window"], 1_000_000);
        let base_desc = base["description"].as_str().unwrap();
        let one_m_desc = one_m["description"].as_str().unwrap();
        assert!(one_m_desc.contains("1M 上下文"));
        assert!(!base_desc.contains("1M 上下文"));
        // 描述不再重复协议段（端点名默认等于协议标签）。
        assert!(!one_m_desc.contains("OpenAI Responses · OpenAI Responses"));

        // 非 1M 模型（M2.7 = 204800）不生成 [1m] 变体。
        assert!(find("MiniMax-M2.7").is_some());
        assert!(find("MiniMax-M2.7[1m]").is_none());

        // 源列表自带 [1m] 后缀（M3）时，补回不带后缀的基础条目。
        let m3_base = find("MiniMax-M3").expect("missing M3 base variant");
        let m3_one_m = find("MiniMax-M3[1m]").expect("missing M3 [1m] variant");
        assert_eq!(m3_base["display_name"], "MiniMax M3");
        assert_eq!(m3_one_m["display_name"], "MiniMax M3[1m]");
        assert_eq!(m3_one_m["context_window"], 1_000_000);
    }

    #[test]
    fn account_gpt_model_gets_account_prefix_when_clashing_with_native() {
        // 原生 catalog 里 GPT-5.6-Sol 用连字符、GPT-5.5 用空格；账号直选经
        // friendly 格式化后分别是 GPT-5.6 Sol（空格）与 GPT-5.5。归一化后应与
        // 原生同名，必须加账号前缀，让用户能看出来自账号 88。
        let catalog_input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "display_name": "GPT-5.6-Sol",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                },
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "acct_88".into(),
                endpoint_id: "ep_88".into(),
                account_name: "88".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "openai".into(),
                model: "gpt-5.6-sol".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_88".into(),
                endpoint_id: "ep_88".into(),
                account_name: "88".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "openai".into(),
                model: "gpt-5.5".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_deepseek".into(),
                endpoint_id: "ep_deepseek".into(),
                account_name: "DeepSeek".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "deepseek".into(),
                model: "deepseek-v4-pro".into(),
                context_window_override: None,
            },
        ];

        let output = build_context_catalog(
            catalog_input,
            None,
            &account_models,
            &[],
            DEX_ROUTER_PROVIDER,
        )
        .unwrap();
        let models = output["models"].as_array().unwrap();
        let find = |model_name: &str| {
            models.iter().find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == model_name)
            })
        };

        // 与原生语义同名 → 加账号前缀。
        let gpt_sol = find("gpt-5.6-sol").expect("missing account gpt-5.6-sol");
        assert_eq!(gpt_sol["display_name"], "GPT-5.6 Sol");
        let gpt_55 = find("gpt-5.5").expect("missing account gpt-5.5");
        assert_eq!(gpt_55["display_name"], "GPT-5.5");
        // 账号直选模型一律带账号前缀，即使无同名冲突也标出所属账号。
        let deepseek = find("deepseek-v4-pro").expect("missing deepseek-v4-pro");
        assert_eq!(deepseek["display_name"], "DeepSeek V4 Pro");
    }

    #[test]
    fn context_catalog_skips_image_generation_only_account_models() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "acct_1".into(),
                endpoint_id: "ep_1".into(),
                account_name: "日日新".into(),
                endpoint_name: "OpenAI Chat".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiChat,
                provider: "deepseek".into(),
                model: "sensenova-6.7-flash-lite".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_1".into(),
                endpoint_id: "ep_1".into(),
                account_name: "日日新".into(),
                endpoint_name: "OpenAI Chat".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiChat,
                provider: "deepseek".into(),
                model: "sensenova-u1-fast".into(),
                context_window_override: None,
            },
        ];

        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let decoded_models = models
            .iter()
            .filter_map(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .map(|slug| slug.model)
            })
            .collect::<Vec<_>>();

        assert!(decoded_models
            .iter()
            .any(|model| model == "sensenova-6.7-flash-lite"));
        assert!(!decoded_models
            .iter()
            .any(|model| model == "sensenova-u1-fast"));
    }

    #[test]
    fn context_catalog_has_registry_models_when_codex_cache_is_empty() {
        let output = build_context_catalog(
            serde_json::json!({ "models": [] }),
            None,
            &[],
            &[],
            DEX_ROUTER_PROVIDER,
        )
        .unwrap();
        let models = output["models"].as_array().unwrap();
        // 缓存为空时不再补齐官方 GPT 模板，仅保留 codex-auto-review 内部辅助模型。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
        assert!(models
            .iter()
            .all(|model| model.get("visibility").and_then(Value::as_str).is_some()));
        assert!(models
            .iter()
            .all(|model| model.get("model").and_then(Value::as_str).is_some()));
        assert!(models
            .iter()
            .all(|model| model.get("hidden").and_then(Value::as_bool).is_some()));
        let auto_review = models
            .iter()
            .find(|model| model.get("slug").and_then(Value::as_str) == Some("codex-auto-review"))
            .unwrap();
        assert_eq!(auto_review["hidden"], true);
    }

    #[test]
    fn context_catalog_preserves_existing_account_models_when_generation_is_empty() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let preserved_slug =
            encode_dex_account_model_slug("acct_old", "endpoint_old", "deepseek-v4-pro");
        let preserved = vec![serde_json::json!({
            "slug": preserved_slug,
            "display_name": "DeepSeek V4 Pro",
            "visibility": "list",
            "provider": DEX_ROUTER_PROVIDER
        })];

        let output =
            build_context_catalog(input, None, &[], &preserved, DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let restored = models
            .iter()
            .find(|model| {
                model.get("slug").and_then(Value::as_str) == Some(preserved_slug.as_str())
            })
            .expect("missing preserved account model");
        assert_eq!(restored["model"], preserved_slug);
        assert_eq!(restored["displayName"], "DeepSeek V4 Pro");
        assert_eq!(restored["hidden"], false);
        assert!(restored
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .is_some_and(|efforts| !efforts.is_empty()));
    }

    #[test]
    fn preserve_existing_account_catalog_even_when_accounts_file_exists() {
        let dir = std::env::temp_dir().join(format!(
            "deecodex-preserve-catalog-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let catalog_path = dir.join(CATALOG_FILENAME);
        let accounts_dir = dir.join("data");
        std::fs::create_dir_all(&accounts_dir).unwrap();
        std::fs::write(
            crate::accounts::accounts_file_path(&accounts_dir),
            serde_json::json!({
                "version": 3,
                "accounts": []
            })
            .to_string(),
        )
        .unwrap();
        let preserved_slug =
            encode_dex_account_model_slug("acct_old", "endpoint_old", "deepseek-v4-pro");
        std::fs::write(
            &catalog_path,
            serde_json::json!({
                "models": [
                    {
                        "slug": preserved_slug,
                        "model": preserved_slug,
                        "display_name": "DeepSeek V4 Pro",
                        "provider": DEECODEX_DESKTOP_PROVIDER
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        assert!(should_preserve_existing_account_catalog(
            Some(&accounts_dir),
            &catalog_path
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn context_catalog_rewrites_preserved_account_provider_for_current_mode() {
        let preserved_slug =
            encode_dex_account_model_slug("acct_old", "endpoint_old", "deepseek-v4-pro");
        let preserved = vec![serde_json::json!({
            "slug": preserved_slug,
            "display_name": "DeepSeek V4 Pro",
            "visibility": "list",
            "provider": DEX_ROUTER_PROVIDER
        })];

        let output = build_context_catalog(
            basic_catalog_input(),
            None,
            &[],
            &preserved,
            DEECODEX_DESKTOP_PROVIDER,
        )
        .unwrap();
        let restored = output["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| {
                model.get("slug").and_then(Value::as_str) == Some(preserved_slug.as_str())
            })
            .expect("missing preserved account model");
        assert_eq!(restored["provider"], DEECODEX_DESKTOP_PROVIDER);
    }

    #[test]
    fn context_catalog_accepts_codex_desktop_camel_case_cache_entries() {
        let input = serde_json::json!({
            "models": [
                {
                    "model": "vendor-model",
                    "displayName": "Vendor Model",
                    "hidden": false,
                    "defaultReasoningEffort": "high",
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "low", "description": "low effort"},
                        {"reasoningEffort": "high", "description": "high effort"}
                    ],
                    "context_window": 128000,
                    "max_context_window": 128000
                }
            ]
        });

        let output = build_context_catalog(input, None, &[], &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let vendor = models
            .iter()
            .find(|model| model.get("model").and_then(Value::as_str) == Some("vendor-model"))
            .expect("missing vendor model");
        assert_eq!(vendor["slug"], "vendor-model");
        assert_eq!(vendor["display_name"], "Vendor Model");
        assert_eq!(vendor["visibility"], "list");
        assert_eq!(vendor["default_reasoning_level"], "high");
        assert!(vendor["supported_reasoning_levels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|level| level["effort"] == "high"));
        assert!(vendor["supportedReasoningEfforts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|effort| effort["reasoningEffort"] == "high"));
    }

    #[test]
    fn account_model_with_one_m_suffix_keeps_one_m_context_window() {
        assert_eq!(
            one_m_context_window_for_model("mimo-v2.5-pro[1m]"),
            Some(1_000_000)
        );
        assert_eq!(
            one_m_context_window_for_model("MiniMax-M3[1m]"),
            Some(1_000_000)
        );
        assert_eq!(one_m_context_window_for_model("deepseek-v4-pro"), None);
    }

    #[test]
    fn known_model_context_window_matches_official_windows() {
        assert_eq!(
            known_model_context_window("deepseek-v4-pro"),
            Some(1_000_000)
        );
        assert_eq!(
            known_model_context_window("deepseek-v4-flash"),
            Some(1_000_000)
        );
        assert_eq!(known_model_context_window("MiniMax-M3"), Some(1_000_000));
        assert_eq!(known_model_context_window("MiniMax-M2.7"), Some(204_800));
        assert_eq!(
            known_model_context_window("minimax-m2.7-highspeed"),
            Some(204_800)
        );
        assert_eq!(known_model_context_window("MiniMax-M2.5"), Some(204_800));
        assert_eq!(known_model_context_window("kimi-k2.5"), Some(256_000));
        assert_eq!(known_model_context_window("kimi-k3"), Some(1_000_000));
        assert_eq!(known_model_context_window("qwen3.5-plus"), Some(1_000_000));
        assert_eq!(known_model_context_window("mimo-v2.5"), Some(1_000_000));
        // [1m] 后缀不应影响已知窗口匹配。
        assert_eq!(
            known_model_context_window("deepseek-v4-pro[1m]"),
            Some(1_000_000)
        );
        // 未收录的模型返回 None，不伪造窗口。
        assert_eq!(known_model_context_window("glm-5"), None);
        assert_eq!(known_model_context_window("hy3"), None);
    }

    #[test]
    fn unknown_account_model_does_not_inherit_template_window() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![DexCatalogAccountModel {
            account_id: "acct_1".into(),
            endpoint_id: "ep_1".into(),
            account_name: "测试账号".into(),
            endpoint_name: "OpenAI Responses".into(),
            endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
            provider: "custom".into(),
            model: "some-unknown-model".into(),
            context_window_override: None,
        }];

        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let account_entry = models
            .iter()
            .find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == "some-unknown-model")
            })
            .expect("missing unknown account model");
        // 未知模型不应继承 gpt-5.5 模板的 272000 窗口。
        assert!(account_entry.get("context_window").is_none());
        assert!(account_entry.get("max_context_window").is_none());
    }

    #[test]
    fn context_catalog_disambiguates_duplicate_account_model_names() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "openai_1".into(),
                endpoint_id: "ep_1".into(),
                account_name: "OpenAI 桌面版 账号".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "openai".into(),
                model: "gpt-5.5".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "openai_2".into(),
                endpoint_id: "ep_2".into(),
                account_name: "OpenAI 备用账号".into(),
                endpoint_name: "OpenAI Responses".into(),
                endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
                provider: "openai".into(),
                model: "gpt-5.5".into(),
                context_window_override: None,
            },
        ];

        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let account_entries = models
            .iter()
            .filter(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == "gpt-5.5")
            })
            .collect::<Vec<_>>();
        assert_eq!(account_entries.len(), 2);
        assert_eq!(account_entries[0]["display_name"], "GPT-5.5");
        assert_eq!(account_entries[1]["display_name"], "GPT-5.5");
        assert_eq!(account_entries[0]["context_window"], 272000);
        assert_eq!(account_entries[0]["max_context_window"], 272000);
        assert_eq!(account_entries[1]["context_window"], 272000);
        assert_eq!(account_entries[1]["max_context_window"], 272000);
        // 官方 gpt-5.5 模板已移除，目录里只剩两个账号直选条目。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
    }

    #[test]
    fn dex_account_model_uses_registry_template_but_caps_default_max_context() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "context_window": 272000,
                    "max_context_window": 272000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![DexCatalogAccountModel {
            account_id: "openai_1".into(),
            endpoint_id: "ep_1".into(),
            account_name: "OpenAI 桌面版账号".into(),
            endpoint_name: "OpenAI Responses".into(),
            endpoint_kind: crate::accounts::EndpointKind::OpenAiResponses,
            provider: "openai".into(),
            model: "gpt-5.4".into(),
            context_window_override: None,
        }];

        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let account_entry = models
            .iter()
            .find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .and_then(decode_dex_account_model_slug)
                    .is_some_and(|slug| slug.model == "gpt-5.4")
            })
            .expect("missing account GPT-5.4");
        // 原生目录里有 gpt-5.4（小写），账号直选经归一化判定同名 → 加账号前缀区分。
        assert_eq!(account_entry["display_name"], "GPT-5.4");
        assert_eq!(account_entry["context_window"], 272_000);
        assert_eq!(account_entry["max_context_window"], 272_000);
        assert_eq!(account_entry["default_reasoning_level"], "xhigh");
        // 官方 gpt-5.4 模板已移除，仅保留账号直选条目。
        assert!(!models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
    }

    #[test]
    fn chat_catalog_models_do_not_inject_unfetched_models() {
        let account: crate::accounts::Account = serde_json::from_value(serde_json::json!({
            "id": "deepseek_1",
            "name": "DeepSeek 桌面版账号",
            "provider": "deepseek",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://api.deepseek.com/v1",
            "api_key": "token",
            "endpoints": [{
                "id": "ep_deepseek",
                "name": "Chat",
                "kind": "open_ai_chat",
                "base_url": "https://api.deepseek.com/v1",
                    "model_map": {
                        "gpt-5.5": "legacy-mapped-model",
                        "gpt-5.4-mini": "legacy-flash-model"
                    }
            }]
        }))
        .unwrap();
        let models =
            catalog_models_for_endpoint(&account, &account.endpoints[0], &serde_json::json!({}));

        // 账号没有上游缓存、known_models、default_model 时，不注入任何
        // provider 兜底模型（那些不是账号获取到的）。
        assert!(models.is_empty());
        assert!(!models.iter().any(|model| model == "legacy-mapped-model"));
        assert!(!models.iter().any(|model| model == "legacy-flash-model"));
        assert!(!models.iter().any(|model| model == "gpt-5.5"));
        assert!(!models.iter().any(|model| model == "gpt-5.4-mini"));
    }

    #[test]
    fn chat_catalog_models_include_migrated_known_models() {
        let mut account: crate::accounts::Account = serde_json::from_value(serde_json::json!({
            "id": "deepseek_1",
            "name": "DeepSeek 桌面版账号",
            "provider": "deepseek",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://api.deepseek.com/v1",
            "api_key": "token",
            "endpoints": [{
                "id": "ep_deepseek",
                "name": "Chat",
                "kind": "open_ai_chat",
                "base_url": "https://api.deepseek.com/v1",
                "model_map": {
                    "gpt-5.5": "legacy-mapped-model",
                    "gpt-5.4-mini": "legacy-flash-model"
                }
            }]
        }))
        .unwrap();
        account.normalize_v2();
        let endpoint = &account.endpoints[0];
        assert!(account.model_map.is_empty());
        assert!(endpoint.model_map.is_empty());
        assert!(endpoint
            .known_models
            .iter()
            .any(|model| model == "legacy-mapped-model"));

        let models = catalog_models_for_endpoint(&account, endpoint, &serde_json::json!({}));

        assert!(models.iter().any(|model| model == "legacy-mapped-model"));
        assert!(models.iter().any(|model| model == "legacy-flash-model"));
        assert!(!models.iter().any(|model| model == "gpt-5.5"));
        assert!(!models.iter().any(|model| model == "gpt-5.4-mini"));
    }

    #[test]
    fn native_openai_catalog_models_do_not_inject_unfetched_gpt() {
        let account: crate::accounts::Account = serde_json::from_value(serde_json::json!({
            "id": "openai_1",
            "name": "OpenAI 桌面版账号",
            "provider": "openai",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://api.openai.com/v1",
            "api_key": "token",
            "endpoints": [{
                "id": "ep_openai",
                "name": "OpenAI Responses",
                "kind": "open_ai_responses",
                "base_url": "https://api.openai.com/v1"
            }]
        }))
        .unwrap();
        let models =
            catalog_models_for_endpoint(&account, &account.endpoints[0], &serde_json::json!({}));

        // Codex 官方账号没有上游刷新缓存、known_models、default_model 时，
        // 不再注入硬编码的官方 GPT 模板（那些不是账号获取到的）。
        assert!(models.is_empty());
    }

    #[test]
    fn dex_catalog_auto_review_account_model_stays_hidden() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "display_name": "GPT-5.6 Sol",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "acct_official".into(),
                endpoint_id: "ep_official".into(),
                account_name: "Codex 官方".into(),
                endpoint_name: "Codex 官方".into(),
                endpoint_kind: crate::accounts::EndpointKind::CodexOfficial,
                provider: "codex".into(),
                model: "gpt-5.6-sol".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_official".into(),
                endpoint_id: "ep_official".into(),
                account_name: "Codex 官方".into(),
                endpoint_name: "Codex 官方".into(),
                endpoint_kind: crate::accounts::EndpointKind::CodexOfficial,
                provider: "codex".into(),
                model: "codex-auto-review".into(),
                context_window_override: None,
            },
        ];
        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let dex_model_name = |model: &Value| -> Option<String> {
            let slug = model.get("slug").and_then(Value::as_str)?;
            decode_dex_account_model_slug(slug).map(|r| r.model)
        };
        let auto_review = models
            .iter()
            .find(|model| dex_model_name(model).as_deref() == Some("codex-auto-review"))
            .expect("账号 codex-auto-review 条目应存在");
        assert_eq!(auto_review["hidden"], Value::Bool(true));
        assert_eq!(auto_review["visibility"], Value::String("hide".into()));
        let sol = models
            .iter()
            .find(|model| dex_model_name(model).as_deref() == Some("gpt-5.6-sol"))
            .expect("账号 gpt-5.6-sol 条目应存在");
        assert_eq!(sol["hidden"], Value::Bool(false));
        assert_eq!(sol["visibility"], Value::String("list".into()));
    }

    #[test]
    fn dex_catalog_filters_retired_and_image_models_from_accounts() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.6-sol",
                    "display_name": "GPT-5.6 Sol",
                    "context_window": 272000,
                    "max_context_window": 1000000,
                    "visibility": "list",
                    "supported_in_api": true
                }
            ]
        });
        let account_models = vec![
            DexCatalogAccountModel {
                account_id: "acct_official".into(),
                endpoint_id: "ep_official".into(),
                account_name: "Codex 官方".into(),
                endpoint_name: "Codex 官方".into(),
                endpoint_kind: crate::accounts::EndpointKind::CodexOfficial,
                provider: "codex".into(),
                model: "gpt-5.6-sol".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_official".into(),
                endpoint_id: "ep_official".into(),
                account_name: "Codex 官方".into(),
                endpoint_name: "Codex 官方".into(),
                endpoint_kind: crate::accounts::EndpointKind::CodexOfficial,
                provider: "codex".into(),
                model: "gpt-5.2".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_official".into(),
                endpoint_id: "ep_official".into(),
                account_name: "Codex 官方".into(),
                endpoint_name: "Codex 官方".into(),
                endpoint_kind: crate::accounts::EndpointKind::CodexOfficial,
                provider: "codex".into(),
                model: "gpt-5.3-codex".into(),
                context_window_override: None,
            },
            DexCatalogAccountModel {
                account_id: "acct_official".into(),
                endpoint_id: "ep_official".into(),
                account_name: "Codex 官方".into(),
                endpoint_name: "Codex 官方".into(),
                endpoint_kind: crate::accounts::EndpointKind::CodexOfficial,
                provider: "codex".into(),
                model: "gpt-image-2".into(),
                context_window_override: None,
            },
        ];
        let output =
            build_context_catalog(input, None, &account_models, &[], DEX_ROUTER_PROVIDER).unwrap();
        let models = output["models"].as_array().unwrap();
        let dex_models = models
            .iter()
            .filter_map(|model| {
                let slug = model.get("slug").and_then(Value::as_str)?;
                decode_dex_account_model_slug(slug).map(|r| r.model)
            })
            .collect::<Vec<_>>();
        assert!(dex_models.contains(&"gpt-5.6-sol".to_string()));
        assert!(!dex_models.iter().any(|m| m == "gpt-5.2"));
        assert!(!dex_models.iter().any(|m| m == "gpt-5.3-codex"));
        assert!(!dex_models.iter().any(|m| m == "gpt-image-2"));
    }

    #[test]
    fn catalog_models_for_endpoint_filters_retired_and_image_models() {
        let account: crate::accounts::Account = serde_json::from_value(serde_json::json!({
            "id": "official_1",
            "name": "Codex 官方",
            "provider": "codex",
            "client_kind": "codex",
            "client_surface": "desktop",
            "upstream": "https://chatgpt.com/backend-api/codex",
            "api_key": "token",
            "endpoints": [{
                "id": "ep_official",
                "name": "Codex 官方",
                "kind": "codex_official",
                "base_url": "https://chatgpt.com/backend-api/codex",
                "known_models": [
                    "gpt-5.6-sol",
                    "gpt-5.2",
                    "gpt-5.3-codex",
                    "gpt-image-2",
                    "codex-auto-review"
                ],
                "model_profiles": {
                    "gpt-5.6-sol": { "enabled": false },
                    "codex-auto-review": { "enabled": true }
                }
            }]
        }))
        .unwrap();
        let models =
            catalog_models_for_endpoint(&account, &account.endpoints[0], &serde_json::json!({}));
        // 停用的 gpt-5.6-sol 不应进入模型选择器
        assert!(!models.iter().any(|m| m == "gpt-5.6-sol"));
        assert!(!models.iter().any(|m| m == "gpt-5.2"));
        assert!(!models.iter().any(|m| m == "gpt-5.3-codex"));
        assert!(!models.iter().any(|m| m == "gpt-image-2"));
        // codex-auto-review 未停用，保留在模型列表（catalog 层单独隐藏），
        // 但退役/图像模型必须被剔除。
        assert!(models.iter().any(|m| m == "codex-auto-review"));
    }

    #[test]
    fn fix_clean_config_returns_zero() {
        let content = "[model_provider]\nkey = \"value\"\n\n[windows]\nsandbox = \"elevated\"\n";
        let path = write_temp_config(content);
        let result = do_fix(&path).unwrap();
        assert_eq!(result, 0);
        cleanup(&path);
    }

    #[test]
    fn fix_duplicate_custom_sections_keeps_last() {
        let content = "\
[model_providers.deecodex]
base_url = \"http://127.0.0.1:4446/v1\"
name = \"custom\"
wire_api = \"responses\"

[other]
key = \"value\"

[model_providers.deecodex]
base_url = \"http://127.0.0.1:5555/v1\"
name = \"custom\"
wire_api = \"responses\"
";
        let path = write_temp_config(content);
        let result = do_fix(&path).unwrap();
        assert_eq!(result, 1, "should fix 1 duplicate section");

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(
            !fixed.contains("4446"),
            "first section should be removed, got: {fixed}"
        );
        assert!(
            fixed.contains("5555"),
            "last section should remain, got: {fixed}"
        );
        assert!(
            fixed.contains("[other]"),
            "[other] section should remain, got: {fixed}"
        );
        assert_eq!(
            fixed
                .lines()
                .filter(|l| l.trim() == "[model_providers.deecodex]")
                .count(),
            1,
            "should have exactly one custom section header"
        );
        cleanup(&path);
    }

    #[test]
    fn fix_removes_sandbox_unelevated() {
        let content = "[windows]\nsandbox = \"unelevated\"\nother_key = \"value\"\n";
        let path = write_temp_config(content);
        let result = do_fix(&path).unwrap();
        assert_eq!(result, 1);

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(
            !fixed.contains("unelevated"),
            "unelevated should be removed, got: {fixed}"
        );
        assert!(
            fixed.contains("other_key"),
            "other_key should remain, got: {fixed}"
        );
        cleanup(&path);
    }

    #[test]
    fn fix_warns_but_does_not_remove_sandbox_off() {
        let content = "[windows]\nsandbox = \"off\"\n";
        let path = write_temp_config(content);
        let result = do_fix(&path).unwrap();
        assert_eq!(result, 0, "sandbox=off should NOT be fixed, only warned");

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(
            fixed.contains("sandbox = \"off\""),
            "sandbox=off should remain unchanged"
        );
        cleanup(&path);
    }

    #[test]
    fn fix_both_duplicate_and_sandbox() {
        let content = "\
[windows]
sandbox = \"unelevated\"

[model_providers.deecodex]
base_url = \"http://old/v1\"

[other]
key = \"value\"

[model_providers.deecodex]
base_url = \"http://new/v1\"
";
        let path = write_temp_config(content);
        let result = do_fix(&path).unwrap();
        assert_eq!(result, 2, "should fix 2 issues (duplicate + sandbox)");

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(!fixed.contains("unelevated"));
        assert!(!fixed.contains("old/v1"));
        assert!(fixed.contains("new/v1"));
        assert_eq!(
            fixed
                .lines()
                .filter(|l| l.trim() == "[model_providers.deecodex]")
                .count(),
            1
        );
        cleanup(&path);
    }

    #[test]
    fn find_section_ranges_single() {
        let lines: Vec<&str> = vec!["[a]", "k=v", "[b]", "x=y"];
        let ranges = find_section_ranges(&lines, "a");
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (0, 2));

        let range_b = find_section_range(&lines, "b");
        assert_eq!(range_b, Some((2, 4)));
    }

    #[test]
    fn find_section_ranges_multiple() {
        let lines: Vec<&str> = vec!["[x]", "a=1", "[x]", "a=2", "[x]", "a=3"];
        let ranges = find_section_ranges(&lines, "x");
        assert_eq!(ranges.len(), 3);
        assert_eq!(ranges[0], (0, 2));
        assert_eq!(ranges[1], (2, 4));
        assert_eq!(ranges[2], (4, 6));
    }

    #[test]
    fn find_section_range_not_found() {
        let lines: Vec<&str> = vec!["[a]", "k=v"];
        assert_eq!(find_section_range(&lines, "nonexistent"), None);
    }

    #[test]
    fn find_section_range_at_eof() {
        let lines: Vec<&str> = vec!["[a]", "k=v"];
        let range = find_section_range(&lines, "a");
        assert_eq!(range, Some((0, 2)));
    }

    #[test]
    fn fix_empty_file_returns_zero() {
        let content = "";
        let path = write_temp_config(content);
        let result = do_fix(&path).unwrap();
        assert_eq!(result, 0);
        cleanup(&path);
    }

    #[test]
    fn fix_nonexistent_config_path_returns_error() {
        let path =
            std::env::temp_dir().join(format!("deecodex-nonexistent-{}", Uuid::new_v4().simple()));
        let result = do_fix(&path);
        assert!(result.is_err());
    }

    #[test]
    fn gpt_5_6_registry_templates_expose_current_reasoning_levels() {
        let sol = codex_registry_model_template("gpt-5.6-sol").unwrap();
        let terra = codex_registry_model_template("gpt-5.6-terra").unwrap();
        let luna = codex_registry_model_template("gpt-5.6-luna").unwrap();

        let efforts = |model: &Value| {
            model["supported_reasoning_levels"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|level| level["effort"].as_str())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };

        assert!(efforts(&sol).contains(&"ultra".to_string()));
        assert!(efforts(&terra).contains(&"ultra".to_string()));
        assert!(efforts(&luna).contains(&"max".to_string()));
        assert!(!efforts(&luna).contains(&"ultra".to_string()));
        assert_eq!(sol["context_window"], 272_000);
        assert_eq!(terra["max_context_window"], 272_000);
        assert_eq!(luna["context_window"], 272_000);
    }
}
