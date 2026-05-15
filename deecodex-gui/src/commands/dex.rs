use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{Emitter, State};
use tracing;

use crate::commands::load_args;
use crate::ServerManager;

// ── 辅助函数 ──────────────────────────────────────────────────────────────

/// 获取 Codex config.toml 的路径（等效于 deecodex::codex_config::codex_config_path）
fn codex_config_path() -> Option<PathBuf> {
    deecodex::config::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

/// 检测 Codex 是否已安装（等效于 deecodex::codex_config::codex_is_installed）
fn codex_is_installed() -> bool {
    if let Some(home) = deecodex::config::home_dir() {
        if home.join(".codex").exists() {
            return true;
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let exe = dir.join("codex");
            if exe.exists() {
                return true;
            }
        }
    }
    false
}

/// 获取活跃账号的 (upstream, api_key, model_map)
fn get_active_account_info(
    data_dir: &std::path::Path,
) -> Option<(String, String, HashMap<String, String>)> {
    let store = deecodex::accounts::load_accounts(data_dir);
    let active = store
        .active_id
        .as_ref()
        .and_then(|id| store.accounts.iter().find(|a| &a.id == id))?;

    Some((
        active.upstream.clone(),
        active.api_key.clone(),
        active.model_map.clone(),
    ))
}

/// 安全路径检查：只允许 ~/.deecodex/、~/.codex/、/tmp/ 下的文件
fn is_path_allowed(path: &std::path::Path) -> bool {
    // 规范化路径
    let canonical = match path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // 路径不存在时，尝试规范化父目录
            if let Some(parent) = path.parent() {
                match parent.canonicalize() {
                    Ok(p) => p.join(path.file_name().unwrap_or_default()),
                    Err(_) => return false,
                }
            } else {
                return false;
            }
        }
    };

    let path_str = canonical.to_string_lossy();

    // 允许 /tmp/
    if path_str.starts_with("/tmp/") || path_str == "/tmp" {
        return true;
    }

    // 允许 ~/.deecodex/
    if let Some(home) = deecodex::config::home_dir() {
        let deecodex_dir = home.join(".deecodex");
        let codex_dir = home.join(".codex");
        if path_str.starts_with(deecodex_dir.to_string_lossy().as_ref())
            || path_str.starts_with(codex_dir.to_string_lossy().as_ref())
        {
            return true;
        }
    }

    false
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────

/// DEX 助手：发送聊天补全请求
/// 当 stream == true 时启用 SSE 流式传输，通过 dex-chat-chunk 事件实时推送每个 chunk。
#[tauri::command]
pub async fn dex_chat(
    manager: State<'_, ServerManager>,
    messages: Vec<Value>,
    tools: Option<Vec<Value>>,
    stream: Option<bool>,
    model: Option<String>,
) -> Result<Value, String> {
    let stream_mode = stream.unwrap_or(false);

    let data_dir = manager.data_dir.lock().await.clone();

    let (upstream, api_key, model_map) = get_active_account_info(&data_dir)
        .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    // 模型映射：优先使用传入的 model，否则用默认 "gpt-5.5"
    let default_model = "gpt-5.5";
    let requested = model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or(default_model);
    let mapped_model = model_map
        .get(requested)
        .cloned()
        .or_else(|| model_map.get(default_model).cloned())
        .or_else(|| model_map.values().next().cloned())
        .unwrap_or_else(|| requested.to_string());

    let base = upstream.trim_end_matches('/');
    let url = format!("{base}/chat/completions");

    let body = json!({
        "model": mapped_model,
        "messages": messages,
        "tools": tools,
        "stream": stream_mode,
    });

    tracing::info!(url = %url, model = %mapped_model, msg_count = messages.len(), stream = stream_mode, "dex_chat 发送请求");

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&body);
    if !api_key.is_empty() {
        req = req.bearer_auth(&api_key);
    }

    let resp = req.send().await.map_err(|e| {
        tracing::error!(error = %e, "dex_chat 请求失败");
        if e.is_timeout() || e.is_connect() {
            "连接上游超时，请检查网络或上游地址".to_string()
        } else {
            format!("请求失败: {e}")
        }
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_msg = match status.as_u16() {
            401 => "API Key 无效，请检查账号配置".to_string(),
            403 => "API 访问被拒绝，请检查账号权限".to_string(),
            429 => "请求频率过高，请稍后重试".to_string(),
            code if code >= 500 => "上游服务暂时不可用，请稍后重试".to_string(),
            _ => {
                let body = resp.text().await.unwrap_or_default();
                format!("上游返回错误 ({}): {}", status, body)
            }
        };
        return Err(err_msg);
    }

    if stream_mode {
        // ── 流式模式：通过 SSE 逐 chunk 推送到前端 ──
        // 克隆 AppHandle 以便在流循环中使用（避免长期持有锁）
        let app_handle = {
            let guard = manager.app_handle.lock().await;
            guard.clone()
        };

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut finish_reason = String::new();
        let mut usage = Value::Null;

        'stream_loop: while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("流读取失败: {e}"))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if line.starts_with("data: ") {
                    let data = &line[6..];
                    if data == "[DONE]" {
                        break 'stream_loop;
                    }
                    if let Ok(chunk_val) = serde_json::from_str::<Value>(data) {
                        // 收集 usage 和 finish_reason（可能分散在多个 chunk 中）
                        if let Some(fr) = chunk_val["choices"]
                            .as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|c| c["finish_reason"].as_str())
                        {
                            if !fr.is_empty() {
                                finish_reason = fr.to_string();
                            }
                        }
                        if chunk_val.get("usage").is_some() {
                            usage = chunk_val["usage"].clone();
                        }

                        // 推送单个 chunk 到前端
                        if let Some(ref handle) = app_handle {
                            let _ = handle.emit(
                                "dex-chat-chunk",
                                &json!({ "chunk": chunk_val, "done": false }),
                            );
                        }
                    }
                }
            }
        }

        // 发送流结束事件
        if let Some(ref handle) = app_handle {
            let _ = handle.emit("dex-chat-chunk", &json!({ "chunk": null, "done": true }));
        }

        Ok(json!({
            "stream": true,
            "finish_reason": finish_reason,
            "usage": usage,
        }))
    } else {
        // ── 非流式模式（保持原有行为）──
        let resp_body: Value = resp
            .json()
            .await
            .map_err(|e| format!("解析响应失败: {e}"))?;

        let choice = resp_body["choices"]
            .as_array()
            .and_then(|choices| choices.first())
            .ok_or_else(|| "响应中没有 choices 数据".to_string())?;

        let message = &choice["message"];
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        // 透传完整 message 对象，保留 reasoning_content 等上游特有字段
        Ok(json!({
            "choices": [{
                "message": message.clone(),
                "finish_reason": finish_reason,
            }]
        }))
    }
}

/// DEX 助手：安全读取文件
#[tauri::command]
pub fn dex_read_file(path: String, max_lines: Option<usize>) -> Result<Value, String> {
    let path = PathBuf::from(&path);
    let max_lines = max_lines.unwrap_or(500);

    if !is_path_allowed(&path) {
        return Err(format!(
            "安全限制：只允许读取 ~/.deecodex/、~/.codex/、/tmp/ 下的文件，当前路径: {}",
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

/// DEX 助手：列出目录内容
#[tauri::command]
pub fn dex_list_directory(path: String) -> Result<Value, String> {
    let path = PathBuf::from(&path);

    if !is_path_allowed(&path) {
        return Err(format!(
            "安全限制：只允许读取 ~/.deecodex/、~/.codex/、/tmp/ 下的目录，当前路径: {}",
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

    // 按目录优先、名称排序
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

/// DEX 助手：检测运行中的进程
#[tauri::command]
pub fn dex_detect_processes() -> Result<Value, String> {
    let targets = ["codex", "deecodex", "claude", "node"];
    let mut processes: Vec<Value> = Vec::new();

    for target in &targets {
        // 尝试 pgrep -a，失败则降级到 pgrep -l
        let output = std::process::Command::new("pgrep")
            .arg("-a")
            .arg(target)
            .output();

        let instances = match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                parse_pgrep_output(&stdout)
            }
            _ => {
                // 降级到 pgrep -l
                let output = std::process::Command::new("pgrep")
                    .arg("-l")
                    .arg(target)
                    .output();

                match output {
                    Ok(out) if out.status.success() => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        parse_pgrep_l_output(&stdout)
                    }
                    _ => Vec::new(),
                }
            }
        };

        processes.push(json!({
            "process": target,
            "running": !instances.is_empty(),
            "instances": instances,
        }));
    }

    Ok(json!({ "processes": processes }))
}

fn parse_pgrep_output(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let pid = parts.next()?.to_string();
            let command = parts.next().unwrap_or("").to_string();
            Some(json!({ "pid": pid, "command": command }))
        })
        .collect()
}

fn parse_pgrep_l_output(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.to_string();
            let command = parts.next().unwrap_or("").to_string();
            Some(json!({ "pid": pid, "command": command }))
        })
        .collect()
}

/// DEX 助手：检测端口占用
#[tauri::command]
pub fn dex_detect_ports() -> Result<Value, String> {
    let args = load_args();
    let mut ports_to_check = vec![4446u16, 9222, 8080, 3000, 8000, 11434];
    // 也检测当前配置的端口
    if !ports_to_check.contains(&args.port) {
        ports_to_check.push(args.port);
    }

    let mut port_results: Vec<Value> = Vec::new();

    for port in &ports_to_check {
        let output = std::process::Command::new("lsof")
            .arg("-i")
            .arg(format!(":{port}"))
            .arg("-P")
            .arg("-n")
            .arg("-t")
            .output();

        let pids: Vec<String> = match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                stdout
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .collect()
            }
            _ => Vec::new(),
        };

        let in_use = !pids.is_empty();

        let processes: Vec<String> = pids
            .iter()
            .map(|pid| {
                let output = std::process::Command::new("ps")
                    .arg("-p")
                    .arg(pid)
                    .arg("-o")
                    .arg("comm=")
                    .output()
                    .ok();
                output
                    .and_then(|o| {
                        if o.status.success() {
                            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "未知".to_string())
            })
            .collect();

        port_results.push(json!({
            "port": port,
            "in_use": in_use,
            "pids": pids,
            "processes": processes,
        }));
    }

    Ok(json!({ "ports": port_results }))
}

/// DEX 助手：收集环境信息
#[tauri::command]
pub fn dex_get_env_info() -> Result<Value, String> {
    let os_type = std::env::consts::OS.to_string();
    let os_version = get_os_version();

    let deecodex_version = env!("CARGO_PKG_VERSION").to_string();

    // Codex 版本
    let codex_version = get_cli_version("codex", &["--version"]);
    let codex_installed = codex_is_installed();

    // Claude 版本
    let claude_version = get_cli_version("claude", &["--version"]);

    // Node 版本
    let node_version = get_cli_version("node", &["--version"]);

    // 配置文件存在性
    let args = load_args();
    let config_json_exists = args.data_dir.join("config.json").exists();
    let accounts_json_exists = args.data_dir.join("accounts.json").exists();
    let codex_config_exists = codex_config_path().map(|p| p.exists()).unwrap_or(false);

    Ok(json!({
        "os": {
            "type": os_type,
            "version": os_version,
        },
        "deecodex": {
            "version": deecodex_version,
            "data_dir": args.data_dir.to_string_lossy(),
        },
        "codex": {
            "installed": codex_installed,
            "version": codex_version,
        },
        "claude": {
            "version": claude_version,
        },
        "node": {
            "version": node_version,
        },
        "config_files": {
            "config_json": config_json_exists,
            "accounts_json": accounts_json_exists,
            "codex_config_toml": codex_config_exists,
        },
    }))
}

fn get_os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "未知".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
            })
            .unwrap_or_else(|| "未知".to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "未知".to_string()
    }
}

fn get_cli_version(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let version = stdout.lines().next().unwrap_or("").trim().to_string();
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

/// 检测命令中是否包含危险模式
fn has_dangerous_pattern(cmd: &str) -> Option<&'static str> {
    let lower = cmd.to_lowercase();
    if lower.contains("rm -rf") || lower.contains("rm  -rf") {
        return Some("rm -rf");
    }
    if lower.contains("mkfs") {
        return Some("mkfs");
    }
    if lower.contains("dd if=") {
        return Some("dd if=");
    }
    if cmd.contains("> /dev/") {
        return Some("写入设备文件");
    }
    if lower.contains("curl") && cmd.contains('|') && lower.contains("sh") {
        return Some("curl | sh");
    }
    None
}

/// DEX 助手：安全执行 Shell 命令
#[tauri::command]
pub async fn dex_execute_shell(
    command: String,
    timeout_secs: Option<u64>,
) -> Result<Value, String> {
    let timeout_secs = timeout_secs.unwrap_or(30);

    if let Some(pattern) = has_dangerous_pattern(&command) {
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

/// DEX 助手：搜索日志
#[tauri::command]
pub fn dex_search_logs(query: String, context_lines: Option<usize>) -> Result<Value, String> {
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
            // 收集上下文
            let start = if i >= ctx { i - ctx } else { 0 };
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

/// DEX 助手：获取 Codex 原始配置
#[tauri::command]
pub fn dex_get_codex_config_raw() -> Result<Value, String> {
    let config_path = codex_config_path();
    let exists = config_path.as_ref().map(|p| p.exists()).unwrap_or(false);

    let (content, size_bytes) = if let Some(ref path) = config_path {
        if exists {
            match std::fs::read_to_string(path) {
                Ok(c) => {
                    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    (c, size)
                }
                Err(_) => {
                    // 尝试处理编码（简化版，主要处理 UTF-8）
                    match std::fs::read(path) {
                        Ok(bytes) => {
                            let size = bytes.len() as u64;
                            let content = String::from_utf8_lossy(&bytes).to_string();
                            (content, size)
                        }
                        Err(e) => return Err(format!("读取配置文件失败: {e}")),
                    }
                }
            }
        } else {
            (String::new(), 0)
        }
    } else {
        (String::new(), 0)
    };

    Ok(json!({
        "exists": exists,
        "path": config_path.map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "content": content,
        "size_bytes": size_bytes,
    }))
}

/// DEX 助手：一键健康概览（合并服务状态+账号+异常）
#[tauri::command]
pub async fn dex_health_summary(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let args = load_args();

    // 服务状态
    let running = manager.is_running().await;
    let port = *manager.port.lock().await;

    // 账号信息
    let store = deecodex::accounts::load_accounts(&data_dir);
    let account_count = store.accounts.len();
    let (account_ok, provider) =
        if let Some((upstream, api_key, _)) = get_active_account_info(&data_dir) {
            let ok = !upstream.is_empty() && !api_key.is_empty();
            let prov = store
                .active_id
                .as_ref()
                .and_then(|id| store.accounts.iter().find(|a| &a.id == id))
                .map(|a| a.provider.clone())
                .unwrap_or_default();
            (ok, prov)
        } else {
            (false, String::new())
        };

    // 最近错误计数
    let log_path = args.data_dir.join("deecodex.log");
    let recent_errors = if log_path.exists() {
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .lines()
            .filter(|l| l.to_lowercase().contains("error") || l.to_lowercase().contains("warn"))
            .count()
    } else {
        0
    };

    // Codex 安装状态
    let codex_ok = codex_is_installed();

    Ok(json!({
        "service": { "running": running, "port": port },
        "account": { "ok": account_ok, "provider": provider, "count": account_count },
        "codex_installed": codex_ok,
        "recent_errors": recent_errors,
        "data_dir": args.data_dir.to_string_lossy(),
    }))
}

/// DEX 助手：请求历史分析（最近请求统计）
#[tauri::command]
pub async fn dex_analyze_requests(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    let store = rh
        .as_ref()
        .ok_or("请求历史不可用（服务未启动或数据库未初始化）")?;
    let entries = store.list(1000).await;

    let total = entries.len();
    if total == 0 {
        return Ok(json!({ "total": 0, "message": "暂无请求记录" }));
    }

    let mut success = 0u64;
    let mut error = 0u64;
    let mut total_tokens = 0u64;
    let mut durations: Vec<u64> = Vec::new();
    let mut models: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

    for e in &entries {
        if e.error_msg.is_empty() {
            success += 1;
        } else {
            error += 1;
        }
        total_tokens += e.total_tokens as u64;
        if e.duration_ms > 0 {
            durations.push(e.duration_ms);
        }
        if !e.model.is_empty() {
            *models.entry(e.model.clone()).or_default() += 1;
        }
    }

    let avg_latency = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<u64>() as f64 / durations.len() as f64
    };
    durations.sort();
    let p50 = durations.get(durations.len() / 2).copied().unwrap_or(0);
    let p99 = durations
        .get((durations.len() as f64 * 0.99) as usize)
        .copied()
        .unwrap_or(0);

    let mut top_models: Vec<Value> = models
        .into_iter()
        .map(|(m, c)| json!({ "model": m, "count": c }))
        .collect();
    top_models.sort_by(|a, b| b["count"].as_u64().cmp(&a["count"].as_u64()));

    Ok(json!({
        "total": total,
        "success_rate": if total > 0 { (success as f64 / total as f64 * 100.0).round() } else { 0.0 },
        "errors": error,
        "total_tokens": total_tokens,
        "avg_latency_ms": avg_latency.round() as u64,
        "p50_latency_ms": p50,
        "p99_latency_ms": p99,
        "top_models": top_models,
    }))
}

// ── 辅助函数（配置对比/系统检测）────────────────────────────────────────────

/// 从 TOML 文本中提取简单键值
fn extract_toml_value(content: &str, key: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{} = ", key)) {
            return rest.trim().trim_matches('"').to_string();
        }
        if let Some(rest) = trimmed.strip_prefix(&format!("{}=", key)) {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    String::new()
}

/// 从 URL 中提取端口号
fn extract_port_from_url(url: &str) -> Option<u16> {
    let host = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("");
    if let Some(port_str) = host.rsplit(':').next() {
        if port_str != host {
            return port_str.parse().ok();
        }
    }
    None
}

/// 获取系统总内存（GB）
fn get_total_memory_gb() -> f64 {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Ok(bytes) = s.trim().parse::<f64>() {
                    return (bytes / (1024.0 * 1024.0 * 1024.0) * 10.0).round() / 10.0;
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let kb: Vec<&str> = line.split_whitespace().collect();
                    if kb.len() >= 2 {
                        if let Ok(kb_val) = kb[1].parse::<f64>() {
                            return (kb_val / (1024.0 * 1024.0) * 10.0).round() / 10.0;
                        }
                    }
                }
            }
        }
    }
    0.0
}

/// 获取指定路径所在磁盘的剩余空间（GB）
fn get_disk_free_gb(path: &std::path::Path) -> f64 {
    let output = std::process::Command::new("df")
        .arg(path.to_string_lossy().as_ref())
        .arg("-k")
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().nth(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 4 {
                    if let Ok(kb) = cols[3].parse::<f64>() {
                        return kb / (1024.0 * 1024.0);
                    }
                }
            }
        }
    }
    0.0
}

// ── DEX 助手命令（第二部分）─────────────────────────────────────────────────

/// DEX 助手：配置备份/恢复
#[tauri::command]
pub fn dex_config_backup(action: String, name: Option<String>) -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;
    let backup_dir = data_dir.join("backups");

    match action.as_str() {
        "backup" => {
            let name = name.ok_or("备份名称不能为空")?;
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let dir_name = format!("{}_{}", name, ts);
            let target = backup_dir.join(&dir_name);
            std::fs::create_dir_all(&target).map_err(|e| format!("创建备份目录失败: {e}"))?;

            let mut files = Vec::new();
            for fname in &["config.json", "accounts.json"] {
                let src = data_dir.join(fname);
                if src.exists() {
                    std::fs::copy(&src, target.join(fname))
                        .map_err(|e| format!("备份 {} 失败: {}", fname, e))?;
                    files.push(*fname);
                }
            }

            Ok(json!({
                "ok": true,
                "backup_name": dir_name,
                "files": files,
            }))
        }
        "restore" => {
            let name = name.ok_or("备份名称不能为空")?;
            let source = backup_dir.join(&name);
            if !source.exists() {
                return Err(format!("备份不存在: {}", name));
            }

            for fname in &["config.json", "accounts.json"] {
                let src = source.join(fname);
                if src.exists() {
                    std::fs::copy(&src, data_dir.join(fname))
                        .map_err(|e| format!("恢复 {} 失败: {}", fname, e))?;
                }
            }

            Ok(json!({ "ok": true }))
        }
        "list" => {
            if !backup_dir.exists() {
                return Ok(json!({ "backups": [] }));
            }

            let mut backups = Vec::new();
            let dir =
                std::fs::read_dir(&backup_dir).map_err(|e| format!("读取备份目录失败: {e}"))?;

            for entry in dir {
                let entry = entry.map_err(|e| format!("读取备份条目失败: {e}"))?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let full_name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                let (base_name, time_secs) = if let Some(pos) = full_name.rfind('_') {
                    let ts = full_name[pos + 1..].parse::<u64>().unwrap_or(0);
                    (full_name[..pos].to_string(), ts)
                } else {
                    (full_name.clone(), 0u64)
                };

                let mut files = Vec::new();
                if path.join("config.json").exists() {
                    files.push("config.json");
                }
                if path.join("accounts.json").exists() {
                    files.push("accounts.json");
                }

                backups.push(json!({
                    "name": full_name,
                    "base_name": base_name,
                    "time": time_secs,
                    "files": files,
                }));
            }

            backups.sort_by_key(|b| std::cmp::Reverse(b["time"].as_u64().unwrap_or(0)));

            Ok(json!({ "backups": backups }))
        }
        _ => Err(format!("未知操作: {}，支持: backup, restore, list", action)),
    }
}

/// DEX 助手：配置差异对比
#[tauri::command]
pub fn dex_config_diff() -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;

    let deecodex_port = args.port;
    let store = deecodex::accounts::load_accounts(data_dir);
    let (deecodex_upstream, deecodex_model_count, deecodex_provider) =
        if let Some((up, _, mm)) = get_active_account_info(data_dir) {
            let prov = store
                .active_id
                .as_ref()
                .and_then(|id| store.accounts.iter().find(|a| &a.id == id))
                .map(|a| a.provider.clone())
                .unwrap_or_default();
            (up, mm.len(), prov)
        } else {
            (String::new(), 0, String::new())
        };

    // Codex config.toml 内容
    let codex_toml = codex_config_path()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .unwrap_or_default();

    let codex_base_url = extract_toml_value(&codex_toml, "base_url");
    let codex_model_provider = extract_toml_value(&codex_toml, "model_provider");
    let codex_port = extract_port_from_url(&codex_base_url);

    let mut diffs = Vec::new();

    if codex_model_provider != "deecodex" {
        let severity = if codex_model_provider.is_empty() {
            "critical"
        } else {
            "warning"
        };
        diffs.push(json!({
            "field": "model_provider",
            "deecodex_value": "deecodex",
            "codex_value": if codex_model_provider.is_empty() {
                "(未设置)"
            } else {
                &codex_model_provider
            },
            "severity": severity,
        }));
    }

    if let Some(cp) = codex_port {
        if cp != deecodex_port {
            diffs.push(json!({
                "field": "port",
                "deecodex_value": deecodex_port,
                "codex_value": cp,
                "severity": "warning",
            }));
        }
    } else if codex_model_provider == "deecodex" && !codex_base_url.is_empty() {
        diffs.push(json!({
            "field": "port",
            "deecodex_value": deecodex_port,
            "codex_value": "(无法解析)",
            "severity": "warning",
        }));
    }

    Ok(json!({
        "deecodex": {
            "port": deecodex_port,
            "upstream": deecodex_upstream,
            "model_count": deecodex_model_count,
            "provider": deecodex_provider,
        },
        "codex": {
            "base_url": codex_base_url,
            "model_provider": codex_model_provider,
            "port": codex_port,
        },
        "diffs": diffs,
    }))
}

/// DEX 助手：Token 费用估算
#[tauri::command]
pub async fn dex_token_cost(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    let store = rh
        .as_ref()
        .ok_or("请求历史不可用（服务未启动或数据库未初始化）")?;
    let entries = store.list(1000).await;

    let total = entries.len();
    if total == 0 {
        return Ok(json!({
            "total_tokens": 0,
            "total_input": 0,
            "total_output": 0,
            "estimated_cost_usd": 0.0,
            "by_model": [],
        }));
    }

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut by_model: HashMap<String, (u64, u64)> = HashMap::new();

    for e in &entries {
        total_input += e.input_tokens as u64;
        total_output += e.output_tokens as u64;
        let entry = by_model.entry(e.model.clone()).or_insert((0, 0));
        entry.0 += e.input_tokens as u64;
        entry.1 += e.output_tokens as u64;
    }

    // 通用定价: input $0.5/1M tokens, output $2/1M tokens
    let input_cost = total_input as f64 / 1_000_000.0 * 0.5;
    let output_cost = total_output as f64 / 1_000_000.0 * 2.0;
    let total_cost = input_cost + output_cost;

    let mut models: Vec<Value> = by_model
        .into_iter()
        .map(|(model, (input, output))| {
            let cost = input as f64 / 1_000_000.0 * 0.5 + output as f64 / 1_000_000.0 * 2.0;
            json!({
                "model": model,
                "input_tokens": input,
                "output_tokens": output,
                "total_tokens": input + output,
                "estimated_cost_usd": (cost * 10000.0).round() / 10000.0,
            })
        })
        .collect();
    models.sort_by(|a, b| {
        b["estimated_cost_usd"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["estimated_cost_usd"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(json!({
        "total_tokens": total_input + total_output,
        "total_input": total_input,
        "total_output": total_output,
        "estimated_cost_usd": (total_cost * 10000.0).round() / 10000.0,
        "by_model": models,
    }))
}

/// DEX 助手：模型速度测试
#[tauri::command]
pub async fn dex_speed_test() -> Result<Value, String> {
    let args = load_args();
    let (upstream, api_key, model_map) = get_active_account_info(&args.data_dir)
        .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    let base = upstream.trim_end_matches('/');
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;

    let mut results = Vec::new();
    // 最多测 5 个模型，减少 API 开销
    let model_pairs: Vec<(&String, &String)> = model_map.iter().take(5).collect();

    for (deecodex_model, upstream_model) in &model_pairs {
        let url = format!("{base}/chat/completions");
        let body = json!({
            "model": upstream_model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
            "stream": false,
        });

        let start = std::time::Instant::now();
        let mut req = client.post(&url).json(&body);
        if !api_key.is_empty() {
            req = req.bearer_auth(&api_key);
        }

        match req.send().await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let code = resp.status().as_u16();
                results.push(json!({
                    "model": deecodex_model,
                    "upstream_model": upstream_model,
                    "latency_ms": latency_ms,
                    "status": if code == 200 { "ok".to_string() } else { format!("http_{}", code) },
                }));
            }
            Err(e) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                results.push(json!({
                    "model": deecodex_model,
                    "upstream_model": upstream_model,
                    "latency_ms": latency_ms,
                    "status": format!("error: {}", e),
                }));
            }
        }
    }

    Ok(json!({
        "results": results,
        "upstream": upstream,
    }))
}

/// DEX 助手：线程清理分析
#[tauri::command]
pub fn dex_thread_cleanup(dry_run: Option<bool>) -> Result<Value, String> {
    let dry_run = dry_run.unwrap_or(true);

    let threads =
        deecodex::codex_threads::list_all().map_err(|e| format!("获取线程列表失败: {e}"))?;

    let mut empty_count = 0u64;
    let mut orphan_count = 0u64;
    let mut duplicate_count = 0u64;
    let mut seen_titles: HashSet<String> = HashSet::new();

    for t in &threads {
        // 空线程检测：内容无消息
        if let Ok(content) = deecodex::codex_threads::get_thread_content(&t.id) {
            let msgs = content
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            if msgs == 0 {
                empty_count += 1;
            }
        } else {
            orphan_count += 1;
        }

        // 重复标题检测
        if !t.title.is_empty() && !seen_titles.insert(t.title.clone()) {
            duplicate_count += 1;
        }
    }

    Ok(json!({
        "dry_run": dry_run,
        "total_threads": threads.len(),
        "empty_count": empty_count,
        "orphan_count": orphan_count,
        "duplicate_count": duplicate_count,
        "total_removable": empty_count + orphan_count + duplicate_count,
    }))
}

/// DEX 助手：自动调优建议
#[tauri::command]
pub fn dex_auto_tune() -> Result<Value, String> {
    let args = load_args();

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(4);
    let total_memory_gb = get_total_memory_gb();
    let disk_free_gb = get_disk_free_gb(&args.data_dir);

    let mut recommendations = Vec::new();

    // max_body_mb 建议（基于内存）
    let recommended_body_mb = if total_memory_gb >= 16.0 {
        200
    } else if total_memory_gb >= 8.0 {
        100
    } else {
        50
    };
    if (args.max_body_mb as u32) < recommended_body_mb {
        recommendations.push(json!({
            "param": "max_body_mb",
            "current": args.max_body_mb,
            "recommended": recommended_body_mb,
            "reason": format!("基于 {:.0}GB 内存推荐", total_memory_gb),
        }));
    }

    // 磁盘空间告警
    if disk_free_gb < 10.0 {
        recommendations.push(json!({
            "param": "disk_space",
            "current": format!("{:.1}GB", disk_free_gb),
            "recommended": "清理 data_dir 中的日志和备份",
            "reason": "磁盘剩余空间不足 10GB",
        }));
    }

    // CPU 并发建议
    if cpu_cores >= 8 {
        recommendations.push(json!({
            "param": "concurrency",
            "current": "默认",
            "recommended": "可适当提高请求并发限制",
            "reason": format!("{} 核 CPU 有充足并行能力", cpu_cores),
        }));
    }

    Ok(json!({
        "system": {
            "cpu_cores": cpu_cores,
            "total_memory_gb": total_memory_gb,
            "disk_free_gb": (disk_free_gb * 10.0).round() / 10.0,
        },
        "recommendations": recommendations,
    }))
}

/// DEX 助手：Claude Code MCP 配置检查
#[tauri::command]
pub fn dex_claude_mcp_check() -> Result<Value, String> {
    let home = deecodex::config::home_dir().unwrap_or_default();
    let claude_dir = home.join(".claude");

    let mcp_path = claude_dir.join("mcp.json");
    let desktop_path = claude_dir.join("claude_desktop_config.json");

    let mut has_mcp_config = false;
    let mut has_deecodex_entry = false;
    let mut config_path = String::new();
    let mut issues = Vec::new();

    for path in &[&mcp_path, &desktop_path] {
        if !path.exists() {
            continue;
        }
        has_mcp_config = true;
        if config_path.is_empty() {
            config_path = path.to_string_lossy().to_string();
        }

        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(json_val) = serde_json::from_str::<Value>(&content) {
                if json_val
                    .get("mcpServers")
                    .and_then(|s| s.get("deecodex"))
                    .is_some()
                {
                    has_deecodex_entry = true;
                }
            } else {
                issues.push(format!(
                    "{} 格式无效",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
    }

    if !has_mcp_config {
        issues
            .push("未找到 ~/.claude/mcp.json 或 ~/.claude/claude_desktop_config.json".to_string());
    }
    if has_mcp_config && !has_deecodex_entry {
        issues.push("MCP 配置文件中未找到 deecodex 条目".to_string());
    }

    Ok(json!({
        "has_mcp_config": has_mcp_config,
        "has_deecodex_entry": has_deecodex_entry,
        "config_path": config_path,
        "issues": issues,
    }))
}

/// DEX 助手：网络拓扑检测
#[tauri::command]
pub fn dex_network_topology() -> Result<Value, String> {
    let args = load_args();
    let upstream = args.upstream.clone();

    let host = upstream
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("");

    // DNS 检测
    let dns_servers = get_dns_servers();

    // Ping 检测
    #[cfg(target_os = "macos")]
    let ping_args: &[&str] = &["-c", "1", "-t", "1"];
    #[cfg(not(target_os = "macos"))]
    let ping_args: &[&str] = &["-c", "1", "-W", "1"];

    let (upstream_reachable, latency_ms) = if let Ok(out) = std::process::Command::new("ping")
        .args(ping_args)
        .arg(host)
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let ms = parse_ping_latency(&s);
            (true, ms)
        } else {
            (false, None)
        }
    } else {
        (false, None)
    };

    Ok(json!({
        "dns_servers": dns_servers,
        "upstream_host": host,
        "upstream_reachable": upstream_reachable,
        "latency_ms": latency_ms,
    }))
}

fn get_dns_servers() -> Vec<String> {
    let mut servers = Vec::new();
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("scutil")
            .args(["--dns"])
            .output()
        {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                for line in s.lines() {
                    if let Some(ip) = line
                        .trim()
                        .strip_prefix("nameserver[")
                        .and_then(|rest| rest.split("] : ").nth(1))
                    {
                        let ip = ip.trim();
                        if !servers.contains(&ip.to_string()) {
                            servers.push(ip.to_string());
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
            for line in content.lines() {
                if let Some(ip) = line.trim().strip_prefix("nameserver ") {
                    let ip = ip.trim();
                    if !servers.contains(&ip.to_string()) {
                        servers.push(ip.to_string());
                    }
                }
            }
        }
    }
    servers
}

fn parse_ping_latency(stdout: &str) -> Option<u64> {
    for part in stdout.split_whitespace() {
        if part.starts_with("time=") {
            let time_str = &part[5..];
            if let Ok(ms) = time_str.trim_end_matches("ms").trim().parse::<f64>() {
                return Some(ms as u64);
            }
        }
    }
    None
}

/// DEX 助手：SSL 证书检查
#[tauri::command]
pub fn dex_ssl_check() -> Result<Value, String> {
    let args = load_args();
    let upstream = if args.upstream.is_empty() {
        "https://api.openai.com".to_string()
    } else {
        args.upstream.clone()
    };

    let base = upstream.trim_end_matches('/');
    let check_url = format!("{base}/models");

    let output = std::process::Command::new("curl")
        .args(["-sI", "--max-time", "10", &check_url])
        .output();

    let (https_ok, status) = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);

            if out.status.success() {
                if let Some(line) = stdout.lines().next() {
                    if line.contains("200") || line.contains("401") || line.contains("403") {
                        (
                            true,
                            format!(
                                "HTTPS 连接正常 (HTTP {})",
                                line.split_whitespace().nth(1).unwrap_or("?")
                            ),
                        )
                    } else if line.contains("301") || line.contains("302") {
                        (true, "HTTPS 连接正常 (重定向)".to_string())
                    } else {
                        (false, format!("异常响应: {}", line))
                    }
                } else {
                    (false, "无响应".to_string())
                }
            } else {
                let err = if !stderr.is_empty() { stderr } else { stdout };
                (false, format!("连接失败: {}", err.trim()))
            }
        }
        Err(e) => (false, format!("执行 curl 失败: {}", e)),
    };

    Ok(json!({
        "url": check_url,
        "https_ok": https_ok,
        "status": status,
    }))
}

/// DEX 助手：导出诊断报告
#[tauri::command]
pub fn dex_export_report() -> Result<Value, String> {
    let args = load_args();
    let data_dir = &args.data_dir;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut report = String::new();
    report.push_str("# deecodex 诊断报告\n\n");
    report.push_str(&format!("生成时间: {}\n", now));

    // 环境信息
    report.push_str("\n## 环境信息\n\n");
    report.push_str(&format!("- 操作系统: {}\n", std::env::consts::OS));
    report.push_str(&format!("- deecodex 版本: {}\n", env!("CARGO_PKG_VERSION")));
    report.push_str(&format!("- 数据目录: {}\n", data_dir.display()));

    // 配置文件状态
    report.push_str("\n## 配置文件\n\n");
    let config_exists = data_dir.join("config.json").exists();
    let accounts_exists = data_dir.join("accounts.json").exists();
    let codex_exists = codex_config_path().map_or(false, |p| p.exists());
    report.push_str(&format!(
        "- config.json: {}\n",
        if config_exists { "存在" } else { "缺失" }
    ));
    report.push_str(&format!(
        "- accounts.json: {}\n",
        if accounts_exists { "存在" } else { "缺失" }
    ));
    report.push_str(&format!(
        "- Codex config.toml: {}\n",
        if codex_exists { "存在" } else { "缺失" }
    ));

    // 账号信息
    report.push_str("\n## 账号信息\n\n");
    let store = deecodex::accounts::load_accounts(data_dir);
    report.push_str(&format!("- 账号总数: {}\n", store.accounts.len()));
    report.push_str(&format!(
        "- 活跃账号: {}\n",
        store.active_id.as_deref().unwrap_or("无")
    ));
    for acc in &store.accounts {
        let active = if Some(&acc.id) == store.active_id.as_ref() {
            " [活跃]"
        } else {
            ""
        };
        report.push_str(&format!(
            "  - {}{} ({}), 模型数: {}\n",
            acc.name,
            active,
            acc.provider,
            acc.model_map.len()
        ));
    }

    // 线程状态
    report.push_str("\n## 线程状态\n\n");
    match deecodex::codex_threads::status(data_dir) {
        Ok(s) => {
            report.push_str(&format!("- 线程总数: {}\n", s.total));
            report.push_str(&format!("- 已迁移: {}\n", s.migrated));
            report.push_str(&format!("- 非 deecodex 线程: {}\n", s.non_deecodex_count));
        }
        Err(e) => report.push_str(&format!("- 获取失败: {}\n", e)),
    }

    // Codex 状态
    report.push_str("\n## Codex 状态\n\n");
    report.push_str(&format!("- 已安装: {}\n", codex_is_installed()));

    // 保存报告
    let report_path = data_dir.join("diagnostic_report.md");
    let saved_to = report_path.to_string_lossy().to_string();
    std::fs::write(&report_path, &report).map_err(|e| format!("保存报告失败: {e}"))?;

    Ok(json!({
        "markdown": report,
        "saved_to": saved_to,
    }))
}
