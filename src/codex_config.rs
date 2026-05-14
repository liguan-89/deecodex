use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::Value;
use tracing::{info, warn};

/// deecodex 管理的模型目录文件名
const CATALOG_FILENAME: &str = "models_deecodex.json";

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
/// `context_window_override`: Some(size) 时生成 models_deecodex.json 并设置 model_catalog_json，
/// 同时按 90% 设置 model_auto_compact_token_limit。None 时清除相关配置。
pub fn inject(port: u16, context_window_override: Option<u32>) {
    let Some(path) = codex_config_path() else {
        info!("跳过 Codex 配置注入: 无法确定 HOME 目录");
        return;
    };
    if !path.exists() {
        if codex_is_installed() {
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

    if let Some(cw) = context_window_override {
        if let Err(e) = generate_context_catalog(cw) {
            warn!("生成上下文模型目录失败: {e}");
        }
    } else {
        clear_context_catalog();
    }

    match do_inject(&path, port, context_window_override) {
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

fn do_inject(
    path: &std::path::Path,
    port: u16,
    context_window_override: Option<u32>,
) -> Result<bool> {
    let content = read_config_file(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let already_exists = doc
        .get("model_providers")
        .and_then(|mp| mp.get("deecodex"))
        .is_some();

    doc["model_provider"] = toml_edit::value("deecodex");
    doc["model_providers"]["deecodex"]["base_url"] =
        toml_edit::value(format!("http://127.0.0.1:{}/v1", port));
    doc["model_providers"]["deecodex"]["name"] = toml_edit::value("deecodex");
    doc["model_providers"]["deecodex"]["requires_openai_auth"] = toml_edit::value(false);
    doc["model_providers"]["deecodex"]["api_key"] = toml_edit::value("");
    doc["model_providers"]["deecodex"]["wire_api"] = toml_edit::value("responses");

    // 大上下文窗口覆盖
    if let Some(cw) = context_window_override {
        if let Some(codex_home) = codex_home_dir() {
            doc["model_catalog_json"] = toml_edit::value(
                codex_home
                    .join(CATALOG_FILENAME)
                    .to_string_lossy()
                    .to_string(),
            );
        }
        let compact_limit = (cw as u64 * 9 / 10).min(i64::MAX as u64) as i64;
        doc["model_auto_compact_token_limit"] = toml_edit::value(compact_limit);
        info!("已启用大上下文: context_window={cw}, auto_compact_token_limit={compact_limit}");
    } else {
        doc.remove("model_catalog_json");
        doc.remove("model_auto_compact_token_limit");
    }

    std::fs::write(path, doc.to_string())?;
    Ok(!already_exists)
}

fn do_remove(path: &std::path::Path) -> Result<bool> {
    let content = read_config_file(path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;

    let mut removed = false;

    if doc.get("model_provider").and_then(|v| v.as_str()) == Some("deecodex") {
        doc.remove("model_provider");
        removed = true;
    }

    // 清理大上下文相关配置
    if doc.remove("model_catalog_json").is_some() {
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
            found = inline.remove("deecodex").is_some();
            if inline.is_empty() {
                doc.remove("model_providers");
            }
        } else if let Some(table) = providers.as_table_mut() {
            found = table.remove("deecodex").is_some();
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

/// 从 models_cache.json 生成带有覆盖上下文窗口的模型目录，
/// 写入 ~/.codex/models_deecodex.json。
fn generate_context_catalog(context_window: u32) -> Result<()> {
    let Some(codex_home) = codex_home_dir() else {
        return Err(anyhow!("无法确定 HOME 目录"));
    };

    let cache_path = codex_home.join("models_cache.json");
    let catalog_path = codex_home.join(CATALOG_FILENAME);

    let mut catalog: Value = if cache_path.exists() {
        let content = std::fs::read_to_string(&cache_path)
            .map_err(|e| anyhow!("读取 models_cache.json 失败: {e}"))?;
        serde_json::from_str(&content).map_err(|e| anyhow!("解析 models_cache.json 失败: {e}"))?
    } else {
        return Err(anyhow!("models_cache.json 不存在，请先运行一次 Codex"));
    };

    let models = catalog
        .get_mut("models")
        .and_then(|m| m.as_array_mut())
        .ok_or_else(|| anyhow!("models_cache.json 格式异常: 缺少 models 数组"))?;

    for model in models.iter_mut() {
        model["context_window"] = serde_json::Value::from(context_window);
        model["max_context_window"] = serde_json::Value::from(context_window);
    }

    // model_catalog_json 只接受 {"models": [...]}，去掉缓存中的额外字段
    let catalog_out = serde_json::json!({ "models": models });
    let json = serde_json::to_string_pretty(&catalog_out)
        .map_err(|e| anyhow!("序列化模型目录失败: {e}"))?;
    std::fs::write(&catalog_path, json)
        .map_err(|e| anyhow!("写入 models_deecodex.json 失败: {e}"))?;
    info!(
        "已生成大上下文模型目录: {} (context_window={})",
        catalog_path.display(),
        context_window
    );
    Ok(())
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
        // 跳过 deecodex 自身（本地代理）
        if key == "deecodex" {
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
            upstream: base_url,
            api_key,
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
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
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

    // 1. 检测重复的 [model_providers.deecodex] 节（行级修复，先于 toml_edit）
    let custom_sections = find_section_ranges(&lines, "model_providers.deecodex");
    let mut remove_line_indices: std::collections::BTreeSet<usize> =
        std::collections::BTreeSet::new();

    if custom_sections.len() > 1 {
        for (start, end) in &custom_sections[..custom_sections.len() - 1] {
            for i in *start..*end {
                remove_line_indices.insert(i);
            }
        }
        fixes += (custom_sections.len() - 1) as u32;
        warn!(
            "Codex config.toml: 发现 {} 个重复的 [model_providers.deecodex] 节，保留最后一份",
            custom_sections.len()
        );
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
