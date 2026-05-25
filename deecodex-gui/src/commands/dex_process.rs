use std::path::Path;

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
    // 尝试 pgrep -a，失败则用 -f 查命令行参数，再降级到 pgrep -l。
    // Python/pipx 安装的 CLI（例如 Hermes）真实进程名可能是 Python，
    // 只有完整命令行里保留 hermes 入口路径。
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

fn process_probe_noise(command: &str) -> bool {
    let exe = command
        .split_whitespace()
        .next()
        .and_then(|part| Path::new(part).file_name())
        .and_then(|part| part.to_str())
        .unwrap_or("");
    matches!(exe, "pgrep" | "grep" | "rg")
        || command.contains("pgrep -af")
        || command.contains("rg -i")
        || command.contains("grep -i")
}

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
        assert!(!process_probe_noise(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python /Users/me/.local/bin/hermes"
        ));
        assert!(!process_probe_noise(
            "/Library/Frameworks/Python.framework/Versions/3.11/Resources/Python.app/Contents/MacOS/Python -m hermes_cli.main gateway run --replace"
        ));
    }
}
