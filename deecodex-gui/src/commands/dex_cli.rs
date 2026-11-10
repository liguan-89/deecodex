use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

#[cfg(windows)]
fn hide_window(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut std::process::Command) {}

#[cfg(windows)]
fn hide_tokio_window(command: &mut tokio::process::Command) {
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
fn hide_tokio_window(_command: &mut tokio::process::Command) {}

pub(super) fn command_first_line(cmd: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(command_path_for_spawn(cmd));
    hide_window(&mut command);
    if let Some(path) = cli_child_path_env() {
        command.env("PATH", path);
    }
    command
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let text = if stdout.trim().is_empty() {
                stderr.as_ref()
            } else {
                stdout.as_ref()
            };
            text.lines().next().unwrap_or("").trim().to_string()
        })
        .filter(|s| !s.is_empty())
}

const DEX_CLI_TIMEOUT_SECS: u64 = 8;
const DEX_CLI_OUTPUT_LIMIT: usize = 12_000;

#[derive(Debug, Clone)]
pub(super) struct ReadonlyCliResult {
    pub(super) binary: String,
    pub(super) args: Vec<String>,
    pub(super) success: bool,
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) timed_out: bool,
    pub(super) spawn_error: Option<String>,
}

impl ReadonlyCliResult {
    pub(super) fn unavailable(binary: &str, args: &[&str], error: String) -> Self {
        Self {
            binary: binary.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            spawn_error: Some(error),
        }
    }

    fn timed_out(binary: &str, args: &[&str]) -> Self {
        Self {
            binary: binary.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            spawn_error: None,
        }
    }

    pub(super) fn command_line(&self) -> String {
        let mut parts = vec![self.binary.clone()];
        parts.extend(self.args.clone());
        parts.join(" ")
    }

    pub(super) fn json_output(&self) -> Option<Value> {
        parse_json_output(&self.stdout)
    }

    pub(super) fn to_value(&self) -> Value {
        json!({
            "command": self.command_line(),
            "success": self.success,
            "exit_code": self.exit_code,
            "stdout": self.stdout,
            "stderr": self.stderr,
            "timed_out": self.timed_out,
            "error": self.spawn_error,
            "json": self.json_output(),
        })
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("\n…(已截断)");
            return out;
        }
        out.push(ch);
    }
    out
}

fn parse_json_output(stdout: &str) -> Option<Value> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    for (start_idx, ch) in trimmed.char_indices() {
        let end_idx = match ch {
            '{' => trimmed.rfind('}'),
            '[' => trimmed.rfind(']'),
            _ => None,
        };
        if let Some(end_idx) = end_idx {
            if end_idx > start_idx {
                if let Ok(value) = serde_json::from_str::<Value>(&trimmed[start_idx..=end_idx]) {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn command_candidate_is_file(path: &Path) -> bool {
    path.is_file()
}

fn push_unique_path(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

/// Codex 桌面内置 CLI（`codex`）所在目录：同时覆盖旧版 `Codex.app` 与新版 `ChatGPT.app`，
/// 兼容 `/Applications` 与 `~/Applications` 两个常见安装位置。
/// 仅在 macOS 上有意义——其它平台用 `#[cfg]` gate 剔除，避免 Windows/Linux 路径污染。
#[cfg(target_os = "macos")]
fn codex_desktop_cli_search_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for app_dir in [
        "/Applications/Codex.app",
        "/Applications/ChatGPT.app",
        "~/Applications/Codex.app",
        "~/Applications/ChatGPT.app",
    ] {
        // 与 `dex_clients::expand_home_path` 语义对齐：home 缺失时保留字面量，
        // 让后续 `path.exists()` 自然返回 false 而非整条目录被静默丢弃。
        let path = if let Some(rest) = app_dir.strip_prefix("~/") {
            match deecodex::config::home_dir() {
                Some(home) => home.join(rest),
                None => PathBuf::from(app_dir),
            }
        } else {
            PathBuf::from(app_dir)
        };
        push_unique_path(&mut out, path.join("Contents").join("Resources"));
    }
    out
}

pub(super) fn command_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            push_unique_path(&mut dirs, dir);
        }
    }

    if let Some(home) = deecodex::config::home_dir() {
        for rel in [
            ".local/bin",
            ".npm-global/bin",
            ".opencode/bin",
            ".n/bin",
            ".cargo/bin",
            ".volta/bin",
            ".bun/bin",
            "homebrew/bin",
            "homebrew/sbin",
        ] {
            push_unique_path(&mut dirs, home.join(rel));
        }

        let nvm_versions = home.join(".nvm").join("versions").join("node");
        if let Ok(entries) = std::fs::read_dir(nvm_versions) {
            for entry in entries.flatten() {
                push_unique_path(&mut dirs, entry.path().join("bin"));
            }
        }
    }

    for dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/opt/local/bin",
    ] {
        push_unique_path(&mut dirs, PathBuf::from(dir));
    }

    #[cfg(target_os = "macos")]
    for dir in codex_desktop_cli_search_dirs() {
        push_unique_path(&mut dirs, dir);
    }

    dirs
}

/// GUI 发布版进程（从 Finder/Dock 启动的 .app）PATH 往往只有系统目录，
/// 不含 `~/.npm-global/bin`、`~/.n/bin` 等用户安装目录。spawn CLI 时若不补充
/// PATH，npm 全局安装的 CLI（脚本 shebang 走 `env node`，如 dsh）会因找不到
/// node 而退出，导致「已安装却被误判为未安装 / 下载失败」。
/// 这里把 `command_search_dirs()` 的完整目录合并为子进程 PATH（保持原 PATH 顺序）。
fn cli_child_path_env() -> Option<String> {
    let dirs = command_search_dirs();
    if dirs.is_empty() {
        return None;
    }
    std::env::join_paths(dirs)
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

pub(super) fn find_command_path(cmd: &str) -> Option<PathBuf> {
    if cmd.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(cmd);
        return command_candidate_is_file(&path).then_some(path);
    }
    for dir in command_search_dirs() {
        let candidate = dir.join(cmd);
        if command_candidate_is_file(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for ext in ["exe", "cmd", "bat"] {
                let candidate = dir.join(format!("{cmd}.{ext}"));
                if command_candidate_is_file(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub(super) fn command_path_for_spawn(cmd: &str) -> PathBuf {
    find_command_path(cmd).unwrap_or_else(|| PathBuf::from(cmd))
}

pub(super) fn command_exists(cmd: &str) -> bool {
    find_command_path(cmd).is_some()
}

pub(super) fn get_cli_version_flexible(cmd: &str) -> Option<String> {
    get_cli_version(cmd, &["--version"])
        .or_else(|| get_cli_version(cmd, &["-V"]))
        .or_else(|| get_cli_version(cmd, &["version"]))
}

pub(super) fn get_cli_version(cmd: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(command_path_for_spawn(cmd));
    hide_window(&mut command);
    if let Some(path) = cli_child_path_env() {
        command.env("PATH", path);
    }
    command.args(args).output().ok().and_then(|o| {
        if o.status.success() {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            let text = if stdout.trim().is_empty() {
                stderr.as_ref()
            } else {
                stdout.as_ref()
            };
            let version = text.lines().next().unwrap_or("").trim().to_string();
            if version.is_empty() {
                None
            } else {
                Some(version)
            }
        } else {
            None
        }
    })
}

pub(super) async fn run_readonly_cli_command(binary: &str, args: &[&str]) -> ReadonlyCliResult {
    let mut command = tokio::process::Command::new(command_path_for_spawn(binary));
    hide_tokio_window(&mut command);
    if let Some(path) = cli_child_path_env() {
        command.env("PATH", path);
    }
    command.args(args);
    let command_future = command.output();
    match tokio::time::timeout(Duration::from_secs(DEX_CLI_TIMEOUT_SECS), command_future).await {
        Ok(Ok(output)) => ReadonlyCliResult {
            binary: binary.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: truncate_text(
                &String::from_utf8_lossy(&output.stdout),
                DEX_CLI_OUTPUT_LIMIT,
            ),
            stderr: truncate_text(
                &String::from_utf8_lossy(&output.stderr),
                DEX_CLI_OUTPUT_LIMIT,
            ),
            timed_out: false,
            spawn_error: None,
        },
        Ok(Err(e)) => {
            tracing::warn!(binary, args = ?args, error = %e, "DEX 只读 CLI 命令启动失败");
            ReadonlyCliResult::unavailable(binary, args, e.to_string())
        }
        Err(_) => {
            tracing::warn!(
                binary,
                args = ?args,
                timeout_secs = DEX_CLI_TIMEOUT_SECS,
                "DEX 只读 CLI 命令超时"
            );
            ReadonlyCliResult::timed_out(binary, args)
        }
    }
}

pub(super) async fn run_first_successful_readonly_command(
    binary: &str,
    command_sets: &[Vec<&str>],
) -> ReadonlyCliResult {
    let mut last: Option<ReadonlyCliResult> = None;
    for args in command_sets {
        let result = run_readonly_cli_command(binary, args).await;
        if result.success {
            return result;
        }
        last = Some(result);
    }
    last.unwrap_or_else(|| ReadonlyCliResult::unavailable(binary, &[], "没有可执行命令".into()))
}

pub(super) fn inspect_json_config_files(
    paths: &[PathBuf],
) -> (Vec<Value>, Vec<String>, Vec<String>) {
    let mut files = Vec::new();
    let mut servers = HashSet::new();
    let mut issues = Vec::new();

    for path in paths {
        if !path.exists() {
            continue;
        }
        let mut file = json!({
            "path": path.to_string_lossy(),
            "exists": true,
            "valid_json": false,
        });
        match std::fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(value) => {
                    let file_servers = extract_mcp_servers(&value);
                    for server in &file_servers {
                        servers.insert(server.clone());
                    }
                    file["valid_json"] = json!(true);
                    file["top_level_keys"] = json!(top_level_keys(&value));
                    file["mcp_servers"] = json!(file_servers);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "DEX AI 工具配置 JSON 解析失败");
                    issues.push(format!("{} 格式无效", path.display()));
                    file["error"] = json!(e.to_string());
                }
            },
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "DEX AI 工具配置读取失败");
                issues.push(format!("{} 读取失败", path.display()));
                file["error"] = json!(e.to_string());
            }
        }
        files.push(file);
    }

    let mut server_list: Vec<String> = servers.into_iter().collect();
    server_list.sort();
    (files, server_list, issues)
}

pub(super) fn top_level_keys(value: &Value) -> Vec<String> {
    let Some(map) = value.as_object() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort();
    keys
}

pub(super) fn extract_mcp_servers(value: &Value) -> Vec<String> {
    let mut servers = Vec::new();
    if let Some(map) = value.get("mcpServers").and_then(Value::as_object) {
        servers.extend(map.keys().cloned());
    }
    if let Some(map) = value
        .get("mcp")
        .and_then(|v| v.get("servers"))
        .and_then(Value::as_object)
    {
        servers.extend(map.keys().cloned());
    }
    if let Some(arr) = value
        .get("mcp")
        .and_then(|v| v.get("servers"))
        .and_then(Value::as_array)
    {
        for item in arr {
            if let Some(name) = item
                .get("name")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
            {
                servers.push(name.to_string());
            }
        }
    }
    servers.sort();
    servers.dedup();
    servers
}

pub(super) fn config_dir_summary(dir: &Path, expected_files: &[&str]) -> Value {
    let files: Vec<Value> = expected_files
        .iter()
        .map(|name| {
            let path = dir.join(name);
            json!({
                "name": name,
                "path": path.to_string_lossy(),
                "exists": path.exists(),
            })
        })
        .collect();
    json!({
        "path": dir.to_string_lossy(),
        "exists": dir.exists(),
        "files": files,
    })
}

pub(super) fn issue_if_missing_binary(tool_name: &str, binary: &str, issues: &mut Vec<String>) {
    if !command_exists(binary) {
        tracing::warn!(
            tool = tool_name,
            binary,
            "DEX AI 工具 CLI 未安装或不在 PATH"
        );
        issues.push(format!("{tool_name} CLI 未安装或不在 PATH: {binary}"));
    }
}

fn command_problem_summary(result: &ReadonlyCliResult) -> String {
    if result.timed_out {
        return "命令超时".to_string();
    }
    if let Some(error) = &result.spawn_error {
        return error.clone();
    }
    let stderr = result.stderr.trim();
    if !stderr.is_empty() {
        return stderr.lines().next().unwrap_or(stderr).to_string();
    }
    format!("退出码 {:?}", result.exit_code)
}

pub(super) fn add_failed_command_issue(
    tool_name: &str,
    label: &str,
    result: &ReadonlyCliResult,
    issues: &mut Vec<String>,
) {
    if result.success {
        return;
    }
    let summary = command_problem_summary(result);
    tracing::warn!(
        tool = tool_name,
        command = %result.command_line(),
        error = %summary,
        "DEX AI 工具只读命令未返回成功状态"
    );
    issues.push(format!("{label} 获取失败: {summary}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_json_parser_accepts_prefixed_json_output() {
        let parsed = parse_json_output("Hermes status\n{\"ok\":true,\"items\":[1]}").unwrap();
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["items"][0], 1);

        let parsed = parse_json_output("Hermes skills\n[{\"name\":\"a\"}]").unwrap();
        assert_eq!(parsed[0]["name"], "a");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn command_search_dirs_cover_gui_launch_cli_locations() {
        let dirs = command_search_dirs();
        assert!(dirs.iter().any(|path| path.ends_with(".npm-global/bin")));
        assert!(dirs.iter().any(|path| path.ends_with(".local/bin")));
        assert!(dirs.iter().any(|path| path.ends_with(".opencode/bin")));
        assert!(dirs.iter().any(|path| path.ends_with("homebrew/bin")));
        assert!(dirs
            .iter()
            .any(|path| path == Path::new("/opt/homebrew/bin")));
        // 同时覆盖旧版 Codex 桌面（`Codex.app`）与新版（`ChatGPT.app`）的内置 CLI 路径。
        assert!(dirs
            .iter()
            .any(|path| path == Path::new("/Applications/Codex.app/Contents/Resources")));
        assert!(dirs
            .iter()
            .any(|path| path == Path::new("/Applications/ChatGPT.app/Contents/Resources")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cli_child_path_env_covers_user_bin_dirs() {
        // GUI 进程从 Dock 启动时 PATH 常不含用户目录，补充后必须能覆盖
        // npm 全局 bin / n 管理的 node bin，否则 dsh 等 `env node` 脚本会误判未安装。
        let path = cli_child_path_env().expect("补充 PATH 不应为空");
        for needle in [".npm-global/bin", ".n/bin", ".local/bin", ".opencode/bin"] {
            assert!(
                path.split(':').any(|dir| dir.ends_with(needle)),
                "补充 PATH 应包含 {needle}，实际: {path}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn codex_desktop_cli_search_dirs_cover_all_app_layouts() {
        let dirs = codex_desktop_cli_search_dirs();
        assert!(dirs
            .iter()
            .any(|path| path == Path::new("/Applications/Codex.app/Contents/Resources")));
        assert!(dirs
            .iter()
            .any(|path| path == Path::new("/Applications/ChatGPT.app/Contents/Resources")));
        assert!(dirs.iter().any(|path| {
            path.ends_with("Applications/Codex.app/Contents/Resources")
                && !path.starts_with("/Applications/")
        }));
        assert!(dirs.iter().any(|path| {
            path.ends_with("Applications/ChatGPT.app/Contents/Resources")
                && !path.starts_with("/Applications/")
        }));
    }
}
