use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};
use tauri::State;

use crate::commands::load_args;
use crate::ServerManager;

use super::dex_cli::get_cli_version;
use super::dex_protocol::get_active_account_info;
use super::dex_registry::{is_capability_enabled, load_capability_states};
use super::dex_security::mask_sensitive_value;
use deecodex::request_history::HistoryFilter;

#[cfg(windows)]
fn hide_window(command: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut std::process::Command) {}

pub(super) fn dex_detect_ports_impl() -> Result<Value, String> {
    let args = load_args();
    let mut ports_to_check = vec![4446u16, 9222, 8080, 3000, 8000, 11434];
    if !ports_to_check.contains(&args.port) {
        ports_to_check.push(args.port);
    }

    dex_detect_ports_for_platform(&ports_to_check)
}

#[cfg(windows)]
fn dex_detect_ports_for_platform(ports_to_check: &[u16]) -> Result<Value, String> {
    let ports: Vec<Value> = ports_to_check
        .iter()
        .map(|port| {
            let (pids, processes) = windows_port_processes(*port);
            json!({
                "port": port,
                "in_use": !pids.is_empty(),
                "pids": pids,
                "processes": processes,
            })
        })
        .collect();
    Ok(json!({ "ports": ports }))
}

#[cfg(not(windows))]
fn dex_detect_ports_for_platform(ports_to_check: &[u16]) -> Result<Value, String> {
    let mut port_results: Vec<Value> = Vec::new();

    for port in ports_to_check {
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

#[cfg(windows)]
fn windows_port_processes(port: u16) -> (Vec<String>, Vec<String>) {
    let script = format!(
        r#"
Get-NetTCPConnection -LocalPort {port} -State Listen -ErrorAction SilentlyContinue |
  Select-Object LocalPort,OwningProcess,@{{Name='ProcessName';Expression={{$p=Get-Process -Id $_.OwningProcess -ErrorAction SilentlyContinue; if ($p) {{$p.ProcessName}} else {{''}}}}}} |
  ConvertTo-Json -Compress
"#
    );
    let mut cmd = std::process::Command::new("powershell.exe");
    hide_window(&mut cmd);
    let output = cmd.args(["-NoProfile", "-Command", &script]).output();

    let Ok(out) = output else {
        return (Vec::new(), Vec::new());
    };
    if !out.status.success() {
        return (Vec::new(), Vec::new());
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let rows = match serde_json::from_str::<Value>(raw.trim()) {
        Ok(Value::Array(rows)) => rows,
        Ok(value) if value.is_object() => vec![value],
        _ => Vec::new(),
    };
    let mut pids = Vec::new();
    let mut processes = Vec::new();
    for row in rows {
        let Some(pid) = row
            .get("OwningProcess")
            .and_then(Value::as_i64)
            .map(|pid| pid.to_string())
        else {
            continue;
        };
        let name = row
            .get("ProcessName")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "unknown".to_string());
        pids.push(pid);
        processes.push(name);
    }
    (pids, processes)
}

pub(super) fn dex_get_env_info_impl() -> Result<Value, String> {
    let os_type = std::env::consts::OS.to_string();
    let os_version = get_os_version();
    let deecodex_version = env!("CARGO_PKG_VERSION").to_string();

    let codex_version = get_cli_version("codex", &["--version"]);
    let codex_installed = super::dex::codex_is_installed();

    let claude_version = get_cli_version("claude", &["--version"]);
    let openclaw_version = get_cli_version("openclaw", &["--version"]);
    let hermes_version = get_cli_version("hermes", &["--version"]);
    let node_version = get_cli_version("node", &["--version"]);

    let args = load_args();
    let config_json_exists = args.data_dir.join("config.json").exists();
    let accounts_json_exists = args.data_dir.join("accounts.json").exists();
    let codex_config_exists = super::dex::codex_config_path()
        .map(|p| p.exists())
        .unwrap_or(false);

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
        "openclaw": {
            "version": openclaw_version,
        },
        "hermes": {
            "version": hermes_version,
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

pub(super) async fn dex_health_summary_impl(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let args = load_args();

    let running = manager.is_running().await;
    let port = *manager.port.lock().await;

    let store = deecodex::accounts::load_accounts(&data_dir);
    let account_count = store.accounts.len();
    let client_counts = {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for account in &store.accounts {
            let key = serde_json::to_value(&account.client_kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "codex".into());
            *counts.entry(key).or_default() += 1;
        }
        counts
    };
    let (account_ok, provider, profile_slug, wire_protocol, capability_labels) =
        if let Some((upstream, api_key, _, _, provider, profile, _, _)) =
            get_active_account_info(&data_dir)
        {
            let ok = !upstream.is_empty() && !api_key.is_empty();
            let labels = deecodex::providers::capability_labels(&profile);
            (
                ok,
                provider,
                profile.slug,
                format!("{:?}", profile.wire_protocol),
                labels,
            )
        } else {
            (
                false,
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
            )
        };

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

    let codex_ok = super::dex::codex_is_installed();

    Ok(json!({
        "service": { "running": running, "port": port },
        "account": {
            "ok": account_ok,
            "provider": provider,
            "profile": profile_slug,
            "wire_protocol": wire_protocol,
            "capabilities": capability_labels,
            "count": account_count,
            "client_counts": client_counts
        },
        "codex_installed": codex_ok,
        "recent_errors": recent_errors,
        "data_dir": args.data_dir.to_string_lossy(),
    }))
}

pub(super) async fn dex_self_check_impl(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let states = load_capability_states(&data_dir);
    let tools = super::dex::all_tool_defs(&manager).await;
    let capabilities = super::dex::dex_list_capabilities(manager.clone()).await?;
    let workspace = super::dex_workspace::dex_get_workspace_context_impl(manager.clone())
        .await
        .unwrap_or_else(|e| json!({ "error": e }));

    let mut level_counts: HashMap<String, usize> = HashMap::new();
    let mut source_counts: HashMap<String, usize> = HashMap::new();
    let mut capability_counts: HashMap<String, usize> = HashMap::new();
    let mut disabled_tool_count = 0usize;
    for tool in &tools {
        *level_counts.entry(format!("L{}", tool.level)).or_default() += 1;
        *source_counts.entry(tool.source.clone()).or_default() += 1;
        *capability_counts
            .entry(tool.capability.clone())
            .or_default() += 1;
        if !is_capability_enabled(&states, &tool.capability) {
            disabled_tool_count += 1;
        }
    }
    let plugin_tools: Vec<Value> = tools
        .iter()
        .filter(|tool| tool.source == "plugin")
        .map(|tool| {
            json!({
                "name": tool.name,
                "capability": tool.capability,
                "level": tool.level,
                "plugin_id": tool.plugin_id,
            })
        })
        .collect();
    let recent_request_errors = match manager.request_history.lock().await.as_ref() {
        Some(store) => store
            .list(50, &HistoryFilter::default())
            .await
            .into_iter()
            .filter(|entry| entry.status != "completed" || !entry.error_msg.is_empty())
            .take(10)
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "status": entry.status,
                    "model": entry.model,
                    "error": entry.error_msg,
                    "duration_ms": entry.duration_ms,
                })
            })
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };

    let mut warnings = Vec::new();
    if tools.is_empty() {
        warnings.push("工具注册表为空".to_string());
    }
    if disabled_tool_count > 0 {
        warnings.push(format!("有 {disabled_tool_count} 个工具属于已停用能力包"));
    }
    if plugin_tools.is_empty() {
        warnings.push("当前没有插件向 DEX 暴露工具".to_string());
    }
    if !recent_request_errors.is_empty() {
        warnings.push(format!("最近有 {} 条请求错误", recent_request_errors.len()));
    }

    Ok(mask_sensitive_value(json!({
        "ok": warnings.is_empty(),
        "warnings": warnings,
        "tool_count": tools.len(),
        "disabled_tool_count": disabled_tool_count,
        "level_counts": level_counts,
        "source_counts": source_counts,
        "capability_counts": capability_counts,
        "capabilities": capabilities,
        "plugin_tools": plugin_tools,
        "recent_request_errors": recent_request_errors,
        "workspace": workspace,
    })))
}

pub(super) async fn dex_analyze_requests_impl(
    manager: State<'_, ServerManager>,
) -> Result<Value, String> {
    let rh = manager.request_history.lock().await;
    let store = rh
        .as_ref()
        .ok_or("请求历史不可用（服务未启动或数据库未初始化）")?;
    let entries = store.list(1000, &HistoryFilter::default()).await;

    let total = entries.len();
    if total == 0 {
        return Ok(json!({ "total": 0, "message": "暂无请求记录" }));
    }

    let mut success = 0u64;
    let mut error = 0u64;
    let mut total_tokens = 0u64;
    let mut durations: Vec<u64> = Vec::new();
    let mut models: HashMap<String, u64> = HashMap::new();

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

pub(super) fn get_total_memory_gb() -> f64 {
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
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("powershell.exe");
        hide_window(&mut cmd);
        let output = cmd
            .args([
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ])
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
    0.0
}

pub(super) fn get_disk_free_gb(path: &Path) -> f64 {
    #[cfg(target_os = "windows")]
    {
        let target = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let script = format!(
            r#"
$item = Get-Item -LiteralPath {} -ErrorAction SilentlyContinue
if ($item) {{
  $drive = Get-PSDrive -Name $item.PSDrive.Name -ErrorAction SilentlyContinue
  if ($drive) {{ [Math]::Round($drive.Free / 1GB, 3) }}
}}
"#,
            powershell_single_quote(&target.to_string_lossy())
        );
        let mut cmd = std::process::Command::new("powershell.exe");
        hide_window(&mut cmd);
        let output = cmd.args(["-NoProfile", "-Command", &script]).output();
        if let Ok(out) = output {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Ok(gb) = s.trim().parse::<f64>() {
                    return gb;
                }
            }
        }
        return 0.0;
    }
    #[cfg(not(target_os = "windows"))]
    {
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
}

#[cfg(target_os = "windows")]
fn powershell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
