use std::path::PathBuf;

use anyhow::{anyhow, Result};
use tracing::{info, warn};

/// 读取配置文件，自动处理 UTF-8 / UTF-16 LE / UTF-16 BE 编码。
/// Windows 上 Codex 桌面版可能写入 UTF-16 编码的 config.toml。
fn read_config_file(path: &std::path::Path) -> Result<String> {
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

fn codex_config_path() -> Option<PathBuf> {
    crate::config::home_dir().map(|home| home.join(".codex").join("config.toml"))
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
    doc["model_providers"]["deecodex"]["api_key"] = toml_edit::value(client_api_key);
    doc["model_providers"]["deecodex"]["wire_api"] = toml_edit::value("responses");

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

    // 2. 清理旧的 [model_providers.custom] 节（已迁移到 deecodex）
    {
        let current = std::fs::read_to_string(path)?;
        if let Ok(mut doc) = current.parse::<toml_edit::DocumentMut>() {
            let mut changed = false;
            if let Some(providers) = doc.get_mut("model_providers") {
                let removed = if let Some(inline) = providers.as_inline_table_mut() {
                    inline.remove("custom").is_some()
                } else if let Some(table) = providers.as_table_mut() {
                    table.remove("custom").is_some()
                } else {
                    false
                };
                if removed {
                    fixes += 1;
                    changed = true;
                    info!("Codex config.toml: 已清理旧的 [model_providers.custom] 节");
                }
            }
            if doc.get("model_provider").and_then(|v| v.as_str()) == Some("custom") {
                doc["model_provider"] = toml_edit::value("deecodex");
                fixes += 1;
                changed = true;
                info!("Codex config.toml: model_provider 已从 custom 更新为 deecodex");
            }
            if changed {
                std::fs::write(path, doc.to_string())?;
            }
        }
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
