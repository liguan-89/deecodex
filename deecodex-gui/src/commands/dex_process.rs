use std::path::Path;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub(super) fn dex_detect_processes_impl() -> Result<Value, String> {
    let targets = [
        "codex", "Codex", "deecodex", "claude", "Claude", "openclaw", "hermes", "node",
    ];
    let mut processes: Vec<Value> = Vec::new();

    for target in &targets {
        let instances = detect_process_instances(target);

        processes.push(json!({
            "process": target,
            "running": !instances.is_empty(),
            "instances": instances,
        }));
    }

    Ok(json!({ "processes": processes }))
}

pub(super) fn detect_process_instances(target: &str) -> Vec<Value> {
    #[cfg(windows)]
    {
        detect_process_instances_windows(target)
    }
    #[cfg(not(windows))]
    {
        detect_process_instances_unix(target)
    }
}

#[cfg(not(windows))]
fn detect_process_instances_unix(target: &str) -> Vec<Value> {
    let output = std::process::Command::new("pgrep")
        .arg("-a")
        .arg(target)
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_pgrep_output(&stdout)
        }
        _ => {
            let output = std::process::Command::new("pgrep")
                .arg("-af")
                .arg(target)
                .output();
            if let Ok(out) = output {
                if out.status.success() {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let instances = parse_pgrep_output(&stdout)
                        .into_iter()
                        .filter(|instance| {
                            let command = instance
                                .get("command")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            !process_probe_noise(command)
                        })
                        .collect::<Vec<_>>();
                    if !instances.is_empty() {
                        return instances;
                    }
                }
            }

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
    }
}

#[cfg(windows)]
fn detect_process_instances_windows(target: &str) -> Vec<Value> {
    let target_lower = target.to_ascii_lowercase();
    let rows = windows_process_rows_cached();

    rows.into_iter()
        .filter_map(|row| {
            let pid = row
                .get("ProcessId")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())?;
            let name = row.get("Name").and_then(Value::as_str).unwrap_or("");
            let command_line = row
                .get("CommandLine")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(name);
            let haystack = format!("{name} {command_line}").to_ascii_lowercase();
            if !haystack.contains(&target_lower) || process_probe_noise(command_line) {
                return None;
            }
            Some(json!({ "pid": pid, "command": command_line }))
        })
        .collect()
}

#[cfg(windows)]
fn windows_process_rows_cached() -> Vec<Value> {
    static CACHE: OnceLock<Mutex<Option<(Instant, Vec<Value>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = match cache.lock() {
        Ok(guard) => guard,
        Err(_) => return windows_process_rows_uncached(),
    };
    if let Some((created_at, rows)) = guard.as_ref() {
        if created_at.elapsed() < Duration::from_millis(1500) {
            return rows.clone();
        }
    }
    let rows = windows_process_rows_uncached();
    *guard = Some((Instant::now(), rows.clone()));
    rows
}

#[cfg(windows)]
fn windows_process_rows_uncached() -> Vec<Value> {
    let mut cmd = std::process::Command::new("powershell.exe");
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000);
    let output = cmd
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Select-Object ProcessId,Name,CommandLine | ConvertTo-Json -Compress",
        ])
        .output();

    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(rows)) => rows,
        Ok(value) if value.is_object() => vec![value],
        _ => Vec::new(),
    }
}

fn process_probe_noise(command: &str) -> bool {
    let exe = command
        .split_whitespace()
        .next()
        .and_then(|part| Path::new(part).file_name())
        .and_then(|part| part.to_str())
        .unwrap_or("");
    matches!(
        exe,
        "pgrep" | "grep" | "rg" | "powershell" | "powershell.exe"
    ) || command.contains("pgrep -af")
        || command.contains("rg -i")
        || command.contains("grep -i")
        || command.contains("Get-CimInstance Win32_Process")
}

#[cfg(not(windows))]
fn parse_pgrep_output(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let pid = parts.next()?.to_string();
            let command = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| process_command_for_pid(&pid))
                .unwrap_or_default();
            Some(json!({ "pid": pid, "command": command }))
        })
        .collect()
}

#[cfg(not(windows))]
fn parse_pgrep_l_output(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.to_string();
            let command = process_command_for_pid(&pid)
                .or_else(|| parts.next().map(str::to_string))
                .unwrap_or_default();
            Some(json!({ "pid": pid, "command": command }))
        })
        .collect()
}

#[cfg(not(windows))]
fn process_command_for_pid(pid: &str) -> Option<String> {
    let output = std::process::Command::new("ps")
        .arg("-p")
        .arg(pid)
        .arg("-o")
        .arg("command=")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let command = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if command.is_empty() {
        None
    } else {
        Some(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_probe_noise_filters_shell_search_commands() {
        assert!(process_probe_noise("pgrep -af hermes"));
        assert!(process_probe_noise("rg -i hermes"));
        assert!(process_probe_noise("/opt/homebrew/bin/rg -i hermes"));
        assert!(process_probe_noise(
            "powershell.exe -NoProfile -Command Get-CimInstance Win32_Process"
        ));
        assert!(!process_probe_noise(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python /Users/me/.local/bin/hermes"
        ));
        assert!(!process_probe_noise(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python -m hermes_cli.main gateway run --replace"
        ));
    }
}
