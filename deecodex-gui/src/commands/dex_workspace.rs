use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::State;

use crate::commands::load_args;
use crate::ServerManager;

use super::dex_cli::{command_first_line, get_cli_version, get_cli_version_flexible};
use super::dex_security::{has_dangerous_shell_pattern, mask_sensitive_value};

fn canonicalize_for_allowlist(path: &Path) -> Option<PathBuf> {
    match path.canonicalize() {
        Ok(p) => Some(p),
        Err(_) => {
            let parent = path.parent()?;
            parent
                .canonicalize()
                .ok()
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
        }
    }
}

fn is_under_allowed_root(path: &Path, root: &Path) -> bool {
    let Some(root) = canonicalize_for_allowlist(root) else {
        return false;
    };
    path.starts_with(root)
}

/// 安全路径检查：只允许数据目录、常见客户端配置目录和系统临时目录。
fn is_path_allowed(path: &Path) -> bool {
    let Some(canonical) = canonicalize_for_allowlist(path) else {
        return false;
    };

    let args = load_args();
    if is_under_allowed_root(&canonical, &args.data_dir)
        || is_under_allowed_root(&canonical, &std::env::temp_dir())
        || is_under_allowed_root(&canonical, Path::new("/tmp"))
    {
        return true;
    }

    if let Some(home) = deecodex::config::home_dir() {
        return [".deecodex", ".codex", ".claude", ".openclaw", ".hermes"]
            .iter()
            .map(|dir| home.join(dir))
            .any(|root| is_under_allowed_root(&canonical, &root));
    }

    false
}

fn detect_project_type(cwd: &Path) -> Vec<String> {
    let mut types = Vec::new();
    if cwd.join("Cargo.toml").exists() {
        types.push("rust".to_string());
    }
    if cwd.join("package.json").exists() {
        types.push("node".to_string());
    }
    if cwd.join("pyproject.toml").exists() || cwd.join("requirements.txt").exists() {
        types.push("python".to_string());
    }
    if cwd.join("tauri.conf.json").exists() || cwd.join("src-tauri").exists() {
        types.push("tauri".to_string());
    }
    types
}

fn git_summary(cwd: &Path) -> Value {
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let status = std::process::Command::new("git")
        .args(["status", "--short"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .take(40)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "is_repo": branch.is_some(),
        "branch": branch,
        "dirty_count": status.len(),
        "status_preview": status,
    })
}

pub(super) async fn dex_get_workspace_context_impl(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("获取当前目录失败: {e}"))?;
    let args = load_args();
    let data_dir = manager.data_dir.lock().await.clone();
    let store = deecodex::accounts::load_accounts(&data_dir);
    let active_account = store
        .active_id
        .as_ref()
        .and_then(|id| store.accounts.iter().find(|a| &a.id == id))
        .map(|a| {
            json!({
                "id": a.id,
                "name": a.name,
                "provider": a.provider,
                "client_kind": a.client_kind,
                "target": if a.client_kind.is_codex() { "codex_proxy" } else { "client_config" },
                "upstream": a.upstream,
                "model_count": a.model_map.len(),
            })
        });
    let config_files = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "requirements.txt",
        "tauri.conf.json",
        "AGENTS.md",
        "CLAUDE.md",
        "WORKTREES.md",
    ];
    let present_files: Vec<String> = config_files
        .iter()
        .filter(|name| cwd.join(name).exists())
        .map(|s| s.to_string())
        .collect();
    let ports = super::dex::dex_detect_ports().unwrap_or_else(|e| json!({ "error": e }));
    Ok(mask_sensitive_value(json!({
        "cwd": cwd.to_string_lossy(),
        "data_dir": args.data_dir.to_string_lossy(),
        "project_types": detect_project_type(&cwd),
        "config_files": present_files,
        "git": git_summary(&cwd),
        "cli_versions": {
            "codex": get_cli_version("codex", &["--version"]),
            "claude": get_cli_version("claude", &["--version"]),
            "openclaw": get_cli_version_flexible("openclaw"),
            "hermes": get_cli_version_flexible("hermes"),
            "node": get_cli_version("node", &["--version"]),
            "cargo": command_first_line("cargo", &["--version"]),
            "git": command_first_line("git", &["--version"]),
        },
        "active_account": active_account,
        "ports": ports,
    })))
}

pub(super) fn dex_read_file_impl(path: String, max_lines: Option<usize>) -> Result<Value, String> {
    let path = PathBuf::from(&path);
    let max_lines = max_lines.unwrap_or(500);

    if !is_path_allowed(&path) {
        return Err(format!(
            "安全限制：只允许读取 deecodex 数据目录、常见客户端配置目录和系统临时目录下的文件，当前路径: {}",
            path.display()
        ));
    }

    let metadata = std::fs::metadata(&path).map_err(|e| format!("无法读取文件信息: {e}"))?;
    let size_bytes = metadata.len();

    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {e}"))?;

    let all_lines: Vec<&str> = content.lines().collect();
    let total_lines = all_lines.len();
    let truncated = total_lines > max_lines;

    let lines: Vec<String> = all_lines
        .into_iter()
        .take(max_lines)
        .map(|s| s.to_string())
        .collect();

    Ok(json!({
        "path": path.to_string_lossy(),
        "lines": lines,
        "total_lines": total_lines,
        "truncated": truncated,
        "size_bytes": size_bytes,
    }))
}

pub(super) fn dex_list_directory_impl(path: String) -> Result<Value, String> {
    let path = PathBuf::from(&path);

    if !is_path_allowed(&path) {
        return Err(format!(
            "安全限制：只允许读取 deecodex 数据目录、常见客户端配置目录和系统临时目录下的目录，当前路径: {}",
            path.display()
        ));
    }

    if !path.is_dir() {
        return Err(format!("路径不是目录: {}", path.display()));
    }

    let mut entries: Vec<Value> = Vec::new();

    let dir = std::fs::read_dir(&path).map_err(|e| format!("无法读取目录: {e}"))?;

    for entry in dir {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("获取文件类型失败: {e}"))?;
        let is_dir = file_type.is_dir();
        let size_bytes = if is_dir {
            0
        } else {
            entry.metadata().map(|m| m.len()).unwrap_or(0)
        };

        entries.push(json!({
            "name": name,
            "is_dir": is_dir,
            "size_bytes": size_bytes,
        }));
    }

    entries.sort_by(|a, b| {
        let a_dir = a["is_dir"].as_bool().unwrap_or(false);
        let b_dir = b["is_dir"].as_bool().unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        })
    });

    Ok(json!({
        "path": path.to_string_lossy(),
        "entries": entries,
    }))
}

pub(super) async fn dex_execute_shell_impl(
    command: String,
    timeout_secs: Option<u64>,
    confirmed: Option<bool>,
) -> Result<Value, String> {
    let timeout_secs = timeout_secs.unwrap_or(30);

    if confirmed != Some(true) {
        return Err("安全限制：执行 Shell 命令前必须经过用户确认".to_string());
    }

    if let Some(pattern) = has_dangerous_shell_pattern(&command) {
        return Err(format!("安全限制：禁止执行危险命令 ({pattern})"));
    }

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动命令失败: {e}"))?;

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("命令执行超时 ({timeout_secs}秒)"))?
    .map_err(|e| format!("等待命令结束失败: {e}"))?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let success = output.status.success();

    Ok(json!({
        "command": command,
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "success": success,
    }))
}

pub(super) fn dex_search_logs_impl(
    query: String,
    context_lines: Option<usize>,
) -> Result<Value, String> {
    let args = load_args();
    let log_path = args.data_dir.join("deecodex.log");

    if !log_path.exists() {
        return Ok(json!({
            "query": query,
            "matches": 0,
            "truncated": false,
            "results": [],
        }));
    }

    let content =
        std::fs::read_to_string(&log_path).map_err(|e| format!("读取日志文件失败: {e}"))?;

    let ctx = context_lines.unwrap_or(0);
    let query_lower = query.to_lowercase();
    let all_lines: Vec<&str> = content.lines().collect();
    let max_results = 50usize;

    let mut results: Vec<Value> = Vec::new();

    for (i, line) in all_lines.iter().enumerate() {
        if results.len() >= max_results {
            break;
        }
        if line.to_lowercase().contains(&query_lower) {
            let start = i.saturating_sub(ctx);
            let end = (i + ctx + 1).min(all_lines.len());
            let context: Vec<String> = all_lines[start..end]
                .iter()
                .map(|s| s.to_string())
                .collect();

            results.push(json!({
                "line_number": i + 1,
                "line": line,
                "context": context,
            }));
        }
    }

    let total_matches = all_lines
        .iter()
        .filter(|l| l.to_lowercase().contains(&query_lower))
        .count();
    let truncated = total_matches > max_results;

    Ok(json!({
        "query": query,
        "matches": total_matches,
        "truncated": truncated,
        "results": results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_allowlist_uses_path_boundaries() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("deecodex-allow-{nonce}"));
        let sibling = std::env::temp_dir().join(format!("deecodex-allow-{nonce}-sibling"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let allowed = root.join("file.txt");
        let rejected = sibling.join("file.txt");
        std::fs::write(&allowed, "ok").unwrap();
        std::fs::write(&rejected, "no").unwrap();

        let allowed = allowed.canonicalize().unwrap();
        let rejected = rejected.canonicalize().unwrap();
        assert!(is_under_allowed_root(&allowed, &root));
        assert!(!is_under_allowed_root(&rejected, &root));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(sibling);
    }
}
