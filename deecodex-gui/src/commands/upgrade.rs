use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use deecodex::config::Args;

use super::load_args;

pub async fn check_upgrade_impl() -> Result<Value, String> {
    let args = load_args();
    let version_path = args.data_dir.join("VERSION");
    let current = std::fs::read_to_string(&version_path)
        .or_else(|_| std::fs::read_to_string("../VERSION"))
        .unwrap_or_else(|_| format!("v{}", env!("CARGO_PKG_VERSION")))
        .trim()
        .to_string();

    let latest_tag = fetch_latest_tag().await;

    let cur_ver = parse_version(&current).unwrap_or((0, 0, 0));
    let latest_ver = parse_version(&latest_tag).unwrap_or((0, 0, 0));
    let has_update = latest_ver > cur_ver;

    Ok(json!({
        "current": current,
        "latest": latest_tag,
        "has_update": has_update,
        "changelog": "",
    }))
}

/// 获取最新版本 tag：主站 GitHub API → 兜底 jsDelivr CDN
async fn fetch_latest_tag() -> String {
    let client = match reqwest::Client::builder().user_agent("deecodex").build() {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    // 1. GitHub API
    match client
        .get("https://api.github.com/repos/liguan-89/deecodex/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(tag) = body["tag_name"].as_str() {
                    return tag.to_string();
                }
            }
        }
        _ => {}
    }

    // 2. 兜底：jsDelivr CDN 读取 VERSION 文件
    match client
        .get("https://cdn.jsdelivr.net/gh/liguan-89/deecodex@main/VERSION")
        .timeout(Duration::from_secs(10))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(tag) = resp.text().await {
                let tag = tag.trim().to_string();
                if !tag.is_empty() {
                    return tag;
                }
            }
        }
        _ => {}
    }

    String::new()
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim_start_matches('v');
    let parts: Vec<u32> = s.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() >= 3 {
        Some((parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

pub fn run_upgrade_impl() -> Result<String, String> {
    let args = load_args();
    let script_name = if cfg!(windows) {
        "deecodex.bat"
    } else {
        "deecodex.sh"
    };

    let script = find_or_download_script(script_name, &args)?;

    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .arg("/c")
            .arg(format!(
                "timeout /t 1 /nobreak >nul & \"{}\" update",
                script.display()
            ))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("启动升级失败: {e}"))?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("sleep 1 && exec sh {} update", script.display()))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("启动升级失败: {e}"))?;
    }

    Ok("升级已启动，完成后请重启服务".to_string())
}

fn find_or_download_script(script_name: &str, args: &Args) -> Result<PathBuf, String> {
    // 1. exe 所在目录（CLI .pkg 安装场景）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(script_name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    // 2. ~/.deecodex/（install.sh 场景）
    let deecodex_dir = &args.data_dir;
    let candidate = deecodex_dir.join(script_name);
    if candidate.exists() {
        return Ok(candidate);
    }
    // 3. 自动下载到 ~/.deecodex/
    download_script(script_name, deecodex_dir)
}

fn download_script(script_name: &str, dest_dir: &Path) -> Result<PathBuf, String> {
    let url = format!(
        "https://github.com/liguan-89/deecodex/releases/latest/download/{}",
        script_name
    );
    let dest = dest_dir.join(script_name);
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("创建目录失败: {e}"))?;

    let client = reqwest::blocking::Client::builder()
        .user_agent("deecodex")
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("下载 {} 失败: {e}", script_name))?;

    if !resp.status().is_success() {
        return Err(format!("下载 {} 失败，HTTP {}", script_name, resp.status()));
    }

    let bytes = resp.bytes().map_err(|e| format!("读取响应失败: {e}"))?;
    std::fs::write(&dest, &bytes).map_err(|e| format!("写入 {} 失败: {e}", script_name))?;

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)
            .map_err(|e| format!("读取权限失败: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms).map_err(|e| format!("设置权限失败: {e}"))?;
    }

    Ok(dest)
}
