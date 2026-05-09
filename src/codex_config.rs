use std::path::PathBuf;

use anyhow::Result;
use tracing::{info, warn};

fn codex_config_path() -> Option<PathBuf> {
    crate::config::home_dir()
        .map(|home| home.join(".codex").join("config.toml"))
}

fn find_in_path(name: &str) -> bool {
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

fn codex_is_installed() -> bool {
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
            if std::path::Path::new(&local).join("Programs").join("codex").exists() {
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
pub fn inject(port: u16, client_api_key: &str) {
    let Some(path) = codex_config_path() else {
        info!("跳过 Codex 配置注入: 无法确定 HOME 目录");
        return;
    };
    if !path.exists() {
        if codex_is_installed() {
            // Codex 已安装但 config.toml 尚未创建（桌面版首次使用场景）
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
        } else {
            info!(
                "跳过 Codex 配置注入: 未检测到 Codex 安装 ({} 不存在)",
                path.display()
            );
            return;
        }
    }

    match do_inject(&path, port, client_api_key) {
        Ok(true) => info!("已将 deecodex 配置注入 codex config.toml (port={port})"),
        Ok(false) => info!("codex config.toml 已包含 deecodex 配置，已更新端口"),
        Err(e) => warn!("注入 codex 配置失败: {e}"),
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

fn do_inject(path: &std::path::Path, port: u16, client_api_key: &str) -> Result<bool> {
    let content = std::fs::read_to_string(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let already_exists = doc
        .get("model_providers")
        .and_then(|mp| mp.get("custom"))
        .is_some();

    doc["model_provider"] = toml_edit::value("custom");
    doc["model_providers"]["custom"]["base_url"] =
        toml_edit::value(format!("http://127.0.0.1:{}/v1", port));
    doc["model_providers"]["custom"]["name"] = toml_edit::value("custom");
    doc["model_providers"]["custom"]["requires_openai_auth"] = toml_edit::value(false);
    doc["model_providers"]["custom"]["api_key"] = toml_edit::value(client_api_key);
    doc["model_providers"]["custom"]["wire_api"] = toml_edit::value("responses");

    std::fs::write(path, doc.to_string())?;
    Ok(!already_exists)
}

fn do_remove(path: &std::path::Path) -> Result<bool> {
    let content = std::fs::read_to_string(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let mut removed = false;

    if doc.get("model_provider").and_then(|v| v.as_str()) == Some("custom") {
        doc.remove("model_provider");
        removed = true;
    }

    // 尝试从常规 table 或 inline table 中移除 custom
    if let Some(providers) = doc.get_mut("model_providers") {
        // 检查是否是 inline table
        let mut found = false;
        if let Some(inline) = providers.as_inline_table_mut() {
            found = inline.remove("custom").is_some();
            if inline.is_empty() {
                doc.remove("model_providers");
            }
        } else if let Some(table) = providers.as_table_mut() {
            found = table.remove("custom").is_some();
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
