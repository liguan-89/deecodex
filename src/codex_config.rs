use std::path::PathBuf;

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::Value;
use tracing::{info, warn};

/// deecodex 管理的模型目录文件名
const CATALOG_FILENAME: &str = "models_deecodex.json";
const DEX_ACCOUNT_MODEL_CACHE_FILENAME: &str = "codex_account_models_cache.json";
const DEX_ACCOUNT_MODEL_SLUG_PREFIX: &str = "dexacct";
const DEECODEX_PROVIDER: &str = "deecodex";
const DEECODEX_CLI_PROVIDER: &str = "deecodex_cli";
const DEECODEX_DESKTOP_PROVIDER: &str = "deecodex_desktop";
const DEX_ROUTER_PROVIDER: &str = "dex_router";
const CODEX_REGISTRY_MODEL_SLUGS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-mini",
    "gpt-5.3-codex",
    "gpt-5.2",
    "codex-auto-review",
];
const RETIRED_CODEX_REGISTRY_MODEL_SLUGS: &[&str] = &["gpt-5.3-codex-spark"];

pub(crate) fn managed_model_provider() -> &'static str {
    managed_model_provider_for_mode(&crate::config::default_codex_router_mode())
}

pub(crate) fn managed_model_provider_for_mode(codex_router_mode: &str) -> &'static str {
    if crate::config::codex_router_mode_is_smart(codex_router_mode) {
        DEX_ROUTER_PROVIDER
    } else {
        DEECODEX_PROVIDER
    }
}

pub(crate) fn is_managed_model_provider(provider: &str) -> bool {
    matches!(
        provider.trim(),
        DEECODEX_PROVIDER | DEECODEX_CLI_PROVIDER | DEECODEX_DESKTOP_PROVIDER | DEX_ROUTER_PROVIDER
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
    if crate::config::codex_router_mode_is_smart(codex_router_mode) {
        "/codex-router/v1"
    } else {
        "/v1"
    }
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
        crate::config::codex_router_mode_is_smart(options.codex_router_mode);
    let active_model = read_active_codex_model(&path).unwrap_or_else(|| "gpt-5.5".to_string());
    let active_model =
        if !include_account_models && decode_dex_account_model_slug(&active_model).is_some() {
            "gpt-5.5".to_string()
        } else {
            active_model
        };
    let catalog = match generate_context_catalog(
        options.context_window_override,
        &active_model,
        options.data_dir,
        include_account_models,
    ) {
        Ok(catalog) => Some(catalog),
        Err(e) => {
            warn!(reason = options.reason, "生成 Codex 模型目录失败: {e}");
            None
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

fn do_inject(
    path: &std::path::Path,
    url_host: &str,
    port: u16,
    context_window_override: Option<u32>,
    catalog_path: Option<&std::path::Path>,
    model_context_window: Option<i64>,
    codex_router_mode: &str,
) -> Result<bool> {
    let content = read_config_file(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let active_provider = managed_model_provider_for_mode(codex_router_mode);
    let already_exists = doc
        .get("model_providers")
        .and_then(|mp| mp.get(active_provider))
        .is_some();

    doc["model_provider"] = toml_edit::value(active_provider);
    if !crate::config::codex_router_mode_is_smart(codex_router_mode) {
        reset_account_model_selection_for_api_mode(&mut doc);
    }

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
    write_deecodex_provider(
        doc.as_table_mut(),
        DEECODEX_CLI_PROVIDER,
        url_host,
        port,
        "/codex-cli/v1",
        false,
    );
    write_deecodex_provider(
        doc.as_table_mut(),
        DEECODEX_DESKTOP_PROVIDER,
        url_host,
        port,
        "/codex-desktop/v1",
        false,
    );
    if crate::config::codex_router_mode_is_smart(codex_router_mode) {
        write_deecodex_provider(
            doc.as_table_mut(),
            DEX_ROUTER_PROVIDER,
            url_host,
            port,
            managed_model_provider_route_prefix_for_mode(codex_router_mode),
            true,
        );
    } else if let Some(providers) = doc
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
        let compact_limit = (cw as u64 * 9 / 10).min(i64::MAX as u64) as i64;
        doc["model_auto_compact_token_limit"] = toml_edit::value(compact_limit);
        info!("已启用大上下文: context_window={cw}, auto_compact_token_limit={compact_limit}");
    } else {
        doc.remove("model_auto_compact_token_limit");
    }

    std::fs::write(path, doc.to_string())?;
    Ok(!already_exists)
}

fn reset_account_model_selection_for_api_mode(doc: &mut toml_edit::DocumentMut) {
    let selected_model = doc
        .get("model")
        .and_then(|model| model.as_str())
        .unwrap_or_default();
    if decode_dex_account_model_slug(selected_model).is_some() {
        doc["model"] = toml_edit::value("gpt-5.5");
    }
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
    clear_context_catalog();

    // 尝试从常规 table 或 inline table 中移除 deecodex
    if let Some(providers) = doc.get_mut("model_providers") {
        // 检查是否是 inline table
        let mut found = false;
        if let Some(inline) = providers.as_inline_table_mut() {
            found |= inline.remove(DEECODEX_PROVIDER).is_some();
            found |= inline.remove(DEECODEX_CLI_PROVIDER).is_some();
            found |= inline.remove(DEECODEX_DESKTOP_PROVIDER).is_some();
            found |= inline.remove(DEX_ROUTER_PROVIDER).is_some();
            if inline.is_empty() {
                doc.remove("model_providers");
            }
        } else if let Some(table) = providers.as_table_mut() {
            found |= table.remove(DEECODEX_PROVIDER).is_some();
            found |= table.remove(DEECODEX_CLI_PROVIDER).is_some();
            found |= table.remove(DEECODEX_DESKTOP_PROVIDER).is_some();
            found |= table.remove(DEX_ROUTER_PROVIDER).is_some();
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
    pub reason: &'a str,
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

    let codex_model_slugs = catalog_model_slugs(&catalog);
    let account_models = if include_account_models {
        data_dir
            .map(|data_dir| dex_catalog_account_models(data_dir, &codex_model_slugs))
            .transpose()?
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let catalog_out = build_context_catalog(catalog, context_window_override, &account_models)?;
    let model_context_window = context_window_override
        .map(|window| (window as u64).min(i64::MAX as u64) as i64)
        .or_else(|| resolve_model_context_window(&catalog_out, active_model));
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

/// 清理 deecodex 管理的模型目录文件。
fn clear_context_catalog() {
    if let Some(codex_home) = codex_home_dir() {
        let catalog_path = codex_home.join(CATALOG_FILENAME);
        if catalog_path.exists() {
            if let Err(e) = std::fs::remove_file(&catalog_path) {
                warn!("删除 models_deecodex.json 失败: {e}");
            }
        }
    }
}

fn catalog_model_slugs(catalog: &Value) -> Vec<String> {
    catalog
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn catalog_native_gpt_model_slugs(codex_model_slugs: &[String]) -> Vec<String> {
    let mut models = codex_model_slugs
        .iter()
        .map(|slug| slug.trim())
        .filter(|slug| is_codex_native_text_gpt_model(slug))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    for model in crate::providers::profile_by_slug("codex").known_models {
        if is_codex_native_text_gpt_model(&model) && !models.iter().any(|seen| seen == &model) {
            models.push(model);
        }
    }
    models
}

fn is_codex_native_text_gpt_model(model: &str) -> bool {
    matches!(
        model,
        "gpt-5.5" | "gpt-5.4" | "gpt-5.4-mini" | "gpt-5.3-codex" | "gpt-5.2"
    )
}

fn dex_catalog_account_models(
    data_dir: &std::path::Path,
    codex_model_slugs: &[String],
) -> Result<Vec<DexCatalogAccountModel>> {
    let store = crate::accounts::load_accounts(data_dir);
    let cache = read_account_model_cache(data_dir);
    let native_gpt_models = catalog_native_gpt_model_slugs(codex_model_slugs);
    let mut values = Vec::new();
    for account in store
        .accounts
        .iter()
        .filter(|account| account.client_kind.is_codex())
        .filter(|account| account.client_surface == crate::accounts::AccountClientSurface::Desktop)
    {
        let profile = crate::providers::profile_for_account(account);
        for endpoint in &account.endpoints {
            let models = catalog_models_for_endpoint(
                account,
                endpoint,
                &profile,
                &cache,
                &native_gpt_models,
            );
            let models = models
                .into_iter()
                .map(|model| model.trim().to_string())
                .filter(|model| !model.is_empty())
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
}

fn one_m_context_window_for_model(model: &str) -> Option<u32> {
    model
        .trim()
        .to_ascii_lowercase()
        .ends_with("[1m]")
        .then_some(1_000_000)
}

fn catalog_models_for_endpoint(
    account: &crate::accounts::Account,
    endpoint: &crate::accounts::EndpointConfig,
    profile: &crate::providers::ProviderProfile,
    cache: &Value,
    native_gpt_models: &[String],
) -> Vec<String> {
    if native_account_owns_codex_gpt_models(account, endpoint) {
        let mut models = native_gpt_models.to_vec();
        for model in catalog_native_gpt_model_slugs(native_gpt_models) {
            if !models.iter().any(|seen| seen == &model) {
                models.push(model);
            }
        }
        if models.is_empty() && !account.default_model.trim().is_empty() {
            models.push(account.default_model.trim().to_string());
        }
        return models;
    }

    let mut models = cached_models_for(cache, &account.id, &endpoint.id);
    if models.is_empty() {
        models.extend(endpoint.known_models.iter().cloned());
    }
    if models.is_empty() && !account.default_model.trim().is_empty() {
        models.push(account.default_model.trim().to_string());
    }
    if models.is_empty() {
        models.extend(profile.known_models.iter().cloned());
    }
    models
}

fn native_account_owns_codex_gpt_models(
    account: &crate::accounts::Account,
    endpoint: &crate::accounts::EndpointConfig,
) -> bool {
    matches!(account.provider.as_str(), "openai" | "codex")
        && (endpoint.kind.is_responses_like()
            || endpoint.kind == crate::accounts::EndpointKind::CodexOfficial)
}

fn build_context_catalog(
    mut catalog: Value,
    context_window_override: Option<u32>,
    account_models: &[DexCatalogAccountModel],
) -> Result<Value> {
    let models = catalog
        .get_mut("models")
        .and_then(|m| m.as_array_mut())
        .ok_or_else(|| anyhow!("models_cache.json 格式异常: 缺少 models 数组"))?;

    prune_retired_codex_registry_models(models);
    ensure_codex_registry_models(models);

    for model in models.iter_mut() {
        if let Some(context_window) = context_window_override {
            model["context_window"] = serde_json::Value::from(context_window);
            model["max_context_window"] = serde_json::Value::from(context_window);
        } else if model.get("context_window").is_none() {
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
    hide_base_catalog_models_when_account_models_exist(models, account_models);
    prepend_dex_account_catalog_models(
        models,
        &template_models,
        account_models,
        context_window_override,
    );

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

fn hide_base_catalog_models_when_account_models_exist(
    models: &mut Vec<Value>,
    account_models: &[DexCatalogAccountModel],
) {
    if account_models.is_empty() {
        return;
    }
    models.retain(|model| {
        let slug = model.get("slug").and_then(Value::as_str).unwrap_or("");
        !is_codex_native_text_gpt_model(slug) && !CODEX_REGISTRY_MODEL_SLUGS.contains(&slug)
    });
}

fn prepend_dex_account_catalog_models(
    models: &mut Vec<Value>,
    template_models: &[Value],
    account_models: &[DexCatalogAccountModel],
    context_window_override: Option<u32>,
) {
    if account_models.is_empty() {
        return;
    }
    let fallback_template = catalog_model_template(template_models);
    let existing = models
        .iter()
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<std::collections::HashSet<_>>();
    let existing_display_names = models
        .iter()
        .filter_map(|model| model.get("display_name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect::<std::collections::HashSet<_>>();
    let mut account_display_counts = std::collections::HashMap::<String, usize>::new();
    for account_model in account_models {
        let display_name = catalog_account_model_display_name(account_model);
        *account_display_counts.entry(display_name).or_default() += 1;
    }
    let mut added = std::collections::HashSet::new();
    let mut account_entries = Vec::new();
    for account_model in account_models {
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
        let base_display_name = catalog_account_model_display_name(account_model);
        let display_name = if existing_display_names.contains(&base_display_name)
            || account_display_counts
                .get(&base_display_name)
                .is_some_and(|count| *count > 1)
        {
            catalog_account_model_disambiguated_display_name(account_model, &base_display_name)
        } else {
            base_display_name
        };
        model["slug"] = Value::String(slug);
        model["display_name"] = Value::String(display_name);
        model["description"] = Value::String(format!(
            "DEX AI 账号直选：{} · {} · {} · {}",
            account_model.account_name,
            account_model.endpoint_name,
            account_model.endpoint_kind.label(),
            account_model.model
        ));
        model["visibility"] = Value::String("list".into());
        model["supported_in_api"] = Value::Bool(true);
        model["priority"] = Value::from(3);
        model["provider"] = Value::String(DEX_ROUTER_PROVIDER.into());
        let window = context_window_override.or(account_model.context_window_override);
        if let Some(window) = window {
            model["context_window"] = Value::from(window);
            model["max_context_window"] = Value::from(window);
        } else {
            ensure_model_context_fields(&mut model);
            cap_account_model_max_context_window(&mut model);
        }
        model["availability_nux"] = Value::Null;
        account_entries.push(model);
    }
    if !account_entries.is_empty() {
        models.splice(0..0, account_entries);
    }
}

fn catalog_model_template_for_account_model(
    template_models: &[Value],
    fallback_template: &Value,
    account_model: &DexCatalogAccountModel,
) -> Value {
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
}

fn catalog_account_model_disambiguated_display_name(
    account_model: &DexCatalogAccountModel,
    model_display_name: &str,
) -> String {
    let account_name = account_model.account_name.trim();
    if !account_name.is_empty() {
        return format!("{account_name} / {model_display_name}");
    }
    format!(
        "{} / {}",
        provider_display_label(&account_model.provider),
        model_display_name
    )
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
    ) = match slug {
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
        "supported_reasoning_levels": [
            {"effort": "low", "description": "Fast responses with lighter reasoning"},
            {"effort": "medium", "description": "Balances speed and reasoning depth for everyday tasks"},
            {"effort": "high", "description": "Greater reasoning depth for complex problems"},
            {"effort": "xhigh", "description": "Extra high reasoning depth for complex problems"}
        ],
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
        )
        .unwrap();
        assert!(changed);

        let fixed = std::fs::read_to_string(&path).unwrap();
        assert!(fixed.contains("model_provider = \"deecodex\""));
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert!(doc
            .get("model_providers")
            .and_then(|providers| providers.get("deecodex"))
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
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|value| value.as_str()),
            Some("dex_router")
        );
        let router = doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_router"))
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
    fn inject_api_mode_resets_dex_account_model_selection() {
        let account_slug = encode_dex_account_model_slug("acct_1", "ep_1", "deepseek-v4-pro");
        let path = write_temp_config(&format!("model = \"{account_slug}\"\n"));
        let catalog_path = path.parent().unwrap().join(CATALOG_FILENAME);
        do_inject(
            &path,
            "127.0.0.1",
            4446,
            None,
            Some(&catalog_path),
            Some(272_000),
            crate::config::CODEX_ROUTER_MODE_API,
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert_eq!(
            doc.get("model").and_then(|value| value.as_str()),
            Some("gpt-5.5")
        );
        assert_eq!(
            doc.get("model_provider").and_then(|value| value.as_str()),
            Some("deecodex")
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
        )
        .unwrap();

        let fixed = std::fs::read_to_string(&path).unwrap();
        let doc: toml_edit::DocumentMut = fixed.parse().unwrap();
        assert_eq!(
            doc.get("model").and_then(|value| value.as_str()),
            Some("gpt-5.5")
        );
        assert_eq!(
            doc.get("model_provider").and_then(|value| value.as_str()),
            Some("deecodex")
        );
        assert!(doc
            .get("model_providers")
            .and_then(|providers| providers.get("dex_router"))
            .is_none());
        let provider = doc
            .get("model_providers")
            .and_then(|providers| providers.get("deecodex"))
            .unwrap();
        assert_eq!(
            provider.get("base_url").and_then(|value| value.as_str()),
            Some("http://127.0.0.1:4446/v1")
        );
        cleanup(&path);
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

        let output = build_context_catalog(input, None, &[]).unwrap();
        assert!(output.get("fetched_at").is_none());
        let models = output["models"].as_array().unwrap();
        assert_eq!(models[0]["context_window"], 272000);
        assert_eq!(models[0]["max_context_window"], 1000000);
        assert_eq!(models[1]["context_window"], 272000);
        assert_eq!(models[1]["max_context_window"], 272000);
        assert_eq!(
            resolve_model_context_window(&output, "gpt-5.5"),
            Some(272_000)
        );
    }

    #[test]
    fn context_catalog_overrides_cached_window_when_requested() {
        let input = serde_json::json!({
            "models": [
                {
                    "slug": "gpt-5.5",
                    "context_window": 272000,
                    "max_context_window": 1000000
                }
            ]
        });

        let output = build_context_catalog(input, Some(2_000_000), &[]).unwrap();
        let model = &output["models"][0];
        assert_eq!(model["context_window"], 2_000_000);
        assert_eq!(model["max_context_window"], 2_000_000);
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

        let output = build_context_catalog(input, None, &[]).unwrap();
        let models = output["models"].as_array().unwrap();
        assert!(!models.iter().any(|model| {
            model.get("slug").and_then(Value::as_str) == Some("gpt-5.3-codex-spark")
        }));
        let gpt_54 = models
            .iter()
            .find(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4"))
            .unwrap();
        assert_eq!(gpt_54["max_context_window"], 1_000_000);
        assert_eq!(gpt_54["default_reasoning_level"], "xhigh");
        assert_eq!(gpt_54["service_tiers"][0]["id"], "priority");
    }

    #[test]
    fn dex_account_model_slug_roundtrips_special_model_names() {
        let slug = encode_dex_account_model_slug("acct.1", "endpoint/2", "vendor/model:latest");
        let decoded = decode_dex_account_model_slug(&slug).unwrap();
        assert_eq!(decoded.account_id, "acct.1");
        assert_eq!(decoded.endpoint_id, "endpoint/2");
        assert_eq!(decoded.model, "vendor/model:latest");
    }

    #[test]
    fn context_catalog_appends_dex_account_models() {
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

        let output = build_context_catalog(input, None, &account_models).unwrap();
        let models = output["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        let appended = &models[0];
        assert_eq!(appended["display_name"], "GPT-5.5 Proxy");
        assert_eq!(appended["context_window"], 128_000);
        assert_eq!(appended["max_context_window"], 128_000);
        assert_eq!(
            decode_dex_account_model_slug(appended["slug"].as_str().unwrap())
                .unwrap()
                .model,
            "gpt-5.5-proxy"
        );
        assert_eq!(models[1]["display_name"], "DeepSeek V4 Pro");
        assert_eq!(models[1]["context_window"], 272000);
        assert_eq!(models[1]["max_context_window"], 272000);
    }

    #[test]
    fn context_catalog_has_registry_models_when_codex_cache_is_empty() {
        let output = build_context_catalog(serde_json::json!({ "models": [] }), None, &[]).unwrap();
        let models = output["models"].as_array().unwrap();
        assert!(models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.5")));
        assert!(models
            .iter()
            .any(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.4")));
        assert!(models
            .iter()
            .all(|model| model.get("visibility").and_then(Value::as_str).is_some()));
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

        let output = build_context_catalog(input, None, &account_models).unwrap();
        let models = output["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["display_name"], "OpenAI 桌面版 账号 / GPT-5.5");
        assert_eq!(models[1]["display_name"], "OpenAI 备用账号 / GPT-5.5");
        assert_eq!(models[0]["context_window"], 272000);
        assert_eq!(models[0]["max_context_window"], 272000);
        assert_eq!(models[1]["context_window"], 272000);
        assert_eq!(models[1]["max_context_window"], 272000);
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

        let output = build_context_catalog(input, None, &account_models).unwrap();
        let models = output["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["display_name"], "GPT-5.4");
        assert_eq!(models[0]["context_window"], 272_000);
        assert_eq!(models[0]["max_context_window"], 272_000);
        assert_eq!(models[0]["default_reasoning_level"], "xhigh");
    }

    #[test]
    fn chat_catalog_models_ignore_retired_model_map_values() {
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
        let profile = crate::providers::profile_for_account(&account);
        let models = catalog_models_for_endpoint(
            &account,
            &account.endpoints[0],
            &profile,
            &serde_json::json!({}),
            &["gpt-5.5".into(), "gpt-5.4-mini".into()],
        );

        assert!(models.iter().any(|model| model == "deepseek-v4-pro"));
        assert!(models.iter().any(|model| model == "deepseek-v4-flash"));
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

        let profile = crate::providers::profile_for_account(&account);
        let models = catalog_models_for_endpoint(
            &account,
            endpoint,
            &profile,
            &serde_json::json!({}),
            &["gpt-5.5".into(), "gpt-5.4-mini".into()],
        );

        assert!(models.iter().any(|model| model == "legacy-mapped-model"));
        assert!(models.iter().any(|model| model == "legacy-flash-model"));
        assert!(!models.iter().any(|model| model == "gpt-5.5"));
        assert!(!models.iter().any(|model| model == "gpt-5.4-mini"));
    }

    #[test]
    fn native_openai_catalog_models_include_owned_gpt_entries() {
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
        let profile = crate::providers::profile_for_account(&account);
        let models = catalog_models_for_endpoint(
            &account,
            &account.endpoints[0],
            &profile,
            &serde_json::json!({}),
            &["gpt-5.5".into(), "gpt-5.4-mini".into()],
        );

        assert!(models.iter().any(|model| model == "gpt-5.5"));
        assert!(models.iter().any(|model| model == "gpt-5.4"));
        assert!(models.iter().any(|model| model == "gpt-5.4-mini"));
        assert!(models.iter().any(|model| model == "gpt-5.3-codex"));
        assert!(models.iter().any(|model| model == "gpt-5.2"));
        assert!(!models.iter().any(|model| model == "gpt-5.3-codex-spark"));
        assert!(!models.iter().any(|model| model == "gpt-5"));
        assert!(!models.iter().any(|model| model == "gpt-4.1"));
    }

    #[test]
    fn native_gpt_catalog_slugs_are_not_limited_by_codex_cache() {
        let models = catalog_native_gpt_model_slugs(&["gpt-5.5".into(), "gpt-5.4-mini".into()]);

        assert!(models.iter().any(|model| model == "gpt-5.5"));
        assert!(models.iter().any(|model| model == "gpt-5.4"));
        assert!(models.iter().any(|model| model == "gpt-5.4-mini"));
        assert!(models.iter().any(|model| model == "gpt-5.3-codex"));
        assert!(models.iter().any(|model| model == "gpt-5.2"));
        assert!(!models.iter().any(|model| model == "gpt-5.3-codex-spark"));
        assert!(!models.iter().any(|model| model == "gpt-image-2"));
        assert!(!models.iter().any(|model| model == "codex-auto-review"));
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
}
