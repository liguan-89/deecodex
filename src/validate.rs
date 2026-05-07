use crate::config::Args;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub category: &'static str,
    pub message: String,
}

/// 对合并后的配置做启动前诊断，返回所有告警和错误。
/// 不阻塞启动——由调用方决定哪些错误是致命的。
pub fn validate(args: &Args) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    check_data_dir(args, &mut diags);
    check_computer_executor(args, &mut diags);
    check_mcp_executor(args, &mut diags);

    diags
}

fn check_data_dir(args: &Args, diags: &mut Vec<Diagnostic>) {
    let dir = Path::new(&args.data_dir);
    match std::fs::create_dir_all(dir) {
        Ok(()) => {
            let md = match std::fs::metadata(dir) {
                Ok(md) => md,
                Err(e) => {
                    diags.push(Diagnostic {
                        severity: Severity::Error,
                        category: "data_dir",
                        message: format!("无法读取数据目录 {} 的元数据: {}", dir.display(), e),
                    });
                    return;
                }
            };
            if !md.is_dir() {
                diags.push(Diagnostic {
                    severity: Severity::Error,
                    category: "data_dir",
                    message: format!("数据目录 {} 不是目录", dir.display()),
                });
            }
        }
        Err(e) => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "data_dir",
                message: format!("无法创建数据目录 {}: {}", dir.display(), e),
            });
        }
    }
}

fn check_computer_executor(args: &Args, diags: &mut Vec<Diagnostic>) {
    let backend = args.computer_executor.trim().to_ascii_lowercase();
    if backend.is_empty() || backend == "disabled" {
        return;
    }

    if backend == "playwright" {
        check_playwright(args, diags);
    } else if backend == "browser-use" || backend == "browser_use" || backend == "browseruse" {
        check_browser_use_bridge(args, diags);
    } else {
        diags.push(Diagnostic {
            severity: Severity::Error,
            category: "computer_executor",
            message: format!(
                "未知的 computer executor 后端 '{}'，支持: disabled / playwright / browser-use",
                args.computer_executor
            ),
        });
    }
}

fn check_playwright(args: &Args, diags: &mut Vec<Diagnostic>) {
    // 检查 Node.js 是否可用
    let node_check = std::process::Command::new("node")
        .arg("-e")
        .arg("process.exit(0)")
        .output();

    match node_check {
        Ok(output) if output.status.success() => {}
        _ => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "computer_executor",
                message: "computer executor 设为 playwright，但 node 命令不可用——Playwright 需要 Node.js 运行时".into(),
            });
            return;
        }
    }

    // 检查 playwright 模块是否可 import
    let import_check = std::process::Command::new("node")
        .arg("-e")
        .arg("require('playwright')")
        .output();

    match import_check {
        Ok(output) if output.status.success() => {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "computer_executor",
                message: "Playwright 可用（检测通过）".into(),
            });
        }
        _ => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "computer_executor",
                message: "computer executor 设为 playwright，但 Node.js 无法 import('playwright')——请确认 playwright 已安装 (npm install playwright)".into(),
            });
        }
    }

    // 检查 state_dir（如果设置了）
    if !args.playwright_state_dir.is_empty() {
        let dir = Path::new(&args.playwright_state_dir);
        match std::fs::create_dir_all(dir) {
            Ok(()) => {}
            Err(e) => {
                diags.push(Diagnostic {
                    severity: Severity::Warn,
                    category: "computer_executor",
                    message: format!(
                        "Playwright state 目录 {} 无法创建: {}——浏览器状态将不会持久化",
                        dir.display(),
                        e
                    ),
                });
            }
        }
    }
}

fn check_browser_use_bridge(_args: &Args, diags: &mut Vec<Diagnostic>) {
    let url = std::env::var("DEECODEX_BROWSER_USE_BRIDGE_URL")
        .unwrap_or_default()
        .trim()
        .to_string();
    let command = std::env::var("DEECODEX_BROWSER_USE_BRIDGE_COMMAND")
        .unwrap_or_default()
        .trim()
        .to_string();

    if url.is_empty() && command.is_empty() {
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "computer_executor",
            message: "computer executor 设为 browser-use，但未配置 DEECODEX_BROWSER_USE_BRIDGE_URL 和 DEECODEX_BROWSER_USE_BRIDGE_COMMAND——browser-use 操作将返回失败".into(),
        });
        return;
    }

    if !url.is_empty() {
        // HTTP bridge 不做在线连通性检查（可能在另一台机器上），只校验格式
        if !url.starts_with("http://") && !url.starts_with("https://") {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "computer_executor",
                message: format!(
                    "browser-use bridge URL '{}' 不以 http:// 或 https:// 开头，可能不是有效的 HTTP 地址",
                    url
                ),
            });
        }
    }

    if !command.is_empty() {
        // 检查命令是否在 PATH 中
        let cmd_name = command.split_whitespace().next().unwrap_or(&command);
        let which = std::process::Command::new("which").arg(cmd_name).output();
        match which {
            Ok(output) if output.status.success() => {}
            _ => {
                diags.push(Diagnostic {
                    severity: Severity::Warn,
                    category: "computer_executor",
                    message: format!(
                        "browser-use bridge 命令 '{}' 不在 PATH 中——bridge 调用将失败",
                        cmd_name
                    ),
                });
            }
        }
    }
}

fn check_mcp_executor(args: &Args, diags: &mut Vec<Diagnostic>) {
    let raw = args.mcp_executor_config.trim();
    if raw.is_empty() {
        return;
    }

    // 尝试解析为 JSON
    let configs: Vec<serde_json::Value> = match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::Array(arr)) => arr,
        Ok(serde_json::Value::Object(obj)) => vec![serde_json::Value::Object(obj)],
        Ok(_) => {
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "mcp_executor",
                message: "MCP executor 配置必须是 JSON 对象或数组".into(),
            });
            return;
        }
        Err(e) => {
            // 可能是文件路径
            let path = Path::new(raw);
            if path.exists() && path.extension().is_some_and(|ext| ext == "json") {
                match std::fs::read_to_string(path) {
                    Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(serde_json::Value::Array(arr)) => {
                            diags.push(Diagnostic {
                                severity: Severity::Warn,
                                category: "mcp_executor",
                                message: format!(
                                    "MCP executor 配置从文件 {} 加载（{} 个 server）",
                                    path.display(),
                                    arr.len()
                                ),
                            });
                            for item in &arr {
                                check_mcp_server_config(item, diags);
                            }
                        }
                        Ok(serde_json::Value::Object(obj)) => {
                            diags.push(Diagnostic {
                                severity: Severity::Warn,
                                category: "mcp_executor",
                                message: format!("MCP executor 配置从文件 {} 加载", path.display()),
                            });
                            check_mcp_server_config(&serde_json::Value::Object(obj), diags);
                        }
                        _ => {
                            diags.push(Diagnostic {
                                severity: Severity::Error,
                                category: "mcp_executor",
                                message: format!(
                                    "MCP executor 配置文件 {} 内容必须是 JSON 对象或数组",
                                    path.display()
                                ),
                            });
                        }
                    },
                    Err(e) => {
                        diags.push(Diagnostic {
                            severity: Severity::Error,
                            category: "mcp_executor",
                            message: format!(
                                "无法读取 MCP executor 配置文件 {}: {}",
                                path.display(),
                                e
                            ),
                        });
                    }
                }
                return;
            }
            diags.push(Diagnostic {
                severity: Severity::Error,
                category: "mcp_executor",
                message: format!(
                    "MCP executor 配置不是有效的 JSON 也不是存在的 .json 文件: {}",
                    e
                ),
            });
            return;
        }
    };

    for config in &configs {
        check_mcp_server_config(config, diags);
    }
}

fn check_mcp_server_config(config: &serde_json::Value, diags: &mut Vec<Diagnostic>) {
    let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        // 可能是以 server label 为 key 的嵌套对象
        if let Some(obj) = config.as_object() {
            for (label, server_config) in obj {
                check_single_mcp_server(label, server_config, diags);
            }
        }
        return;
    }

    check_single_mcp_server("(未命名)", config, diags);
}

fn check_single_mcp_server(label: &str, config: &serde_json::Value, diags: &mut Vec<Diagnostic>) {
    let command = config.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if command.is_empty() {
        diags.push(Diagnostic {
            severity: Severity::Error,
            category: "mcp_executor",
            message: format!("MCP server '{}' 缺少 command 字段", label),
        });
        return;
    }

    // 检查命令是否在 PATH 中
    let which = std::process::Command::new("which").arg(command).output();

    match which {
        Ok(output) if output.status.success() => {}
        _ => {
            diags.push(Diagnostic {
                severity: Severity::Warn,
                category: "mcp_executor",
                message: format!(
                    "MCP server '{}' 的命令 '{}' 不在 PATH 中——工具调用将失败",
                    label, command
                ),
            });
        }
    }

    // 检查 read_only 标记
    let read_only = config
        .get("read_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if read_only {
        let args_count = config
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        diags.push(Diagnostic {
            severity: Severity::Warn,
            category: "mcp_executor",
            message: format!(
                "MCP server '{}' 以只读模式运行（read_only=true）——写入/删除类工具将被拒绝",
                label
            ),
        });
        if args_count > 0 {
            let args_str = config
                .get("args")
                .map(|v| v.to_string())
                .unwrap_or_default();
            if args_str.contains('/') || args_str.contains("root") || args_str.contains("home") {
                diags.push(Diagnostic {
                    severity: Severity::Warn,
                    category: "mcp_executor",
                    message: format!(
                        "MCP server '{}' args 中包含敏感路径——请确认只读模式下的访问范围符合预期",
                        label
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> Args {
        Args {
            command: None,
            config: None,
            port: 4444,
            upstream: "https://openrouter.ai/api/v1".into(),
            api_key: String::new(),
            client_api_key: String::new(),
            model_map: "{}".into(),
            max_body_mb: 100,
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: "MiniMax-M1".into(),
            vision_endpoint: "v1/coding_plan/vlm".into(),
            chinese_thinking: false,
            prompts_dir: std::path::PathBuf::from("prompts"),
            data_dir: std::path::PathBuf::from(".deecodex"),
            token_anomaly_prompt_max: 200000,
            token_anomaly_spike_ratio: 5.0,
            token_anomaly_burn_window: 120,
            token_anomaly_burn_rate: 500000,
            allowed_mcp_servers: String::new(),
            allowed_computer_displays: String::new(),
            computer_executor: "disabled".into(),
            computer_executor_timeout_secs: 30,
            mcp_executor_config: String::new(),
            mcp_executor_timeout_secs: 30,
            playwright_state_dir: String::new(),
            browser_use_bridge_url: String::new(),
            browser_use_bridge_command: String::new(),
            daemon: false,
        }
    }

    #[test]
    fn data_dir_is_creatable_no_error() {
        let dir = std::env::temp_dir().join("deecodex-validate-test");
        let _ = std::fs::remove_dir_all(&dir);
        let mut args = base_args();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        // 临时目录应成功创建，无错误
        assert!(!diags
            .iter()
            .any(|d| d.category == "data_dir" && d.severity == Severity::Error));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_path_is_file_errors() {
        let dir = std::env::temp_dir().join("deecodex-validate-file-test");
        let _ = std::fs::remove_file(&dir);
        std::fs::write(&dir, b"").unwrap();
        let mut args = base_args();
        args.data_dir = dir.clone();

        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "data_dir" && d.severity == Severity::Error));
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn disabled_computer_executor_produces_no_diags() {
        let args = base_args();
        let diags = validate(&args);
        let computer_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "computer_executor")
            .collect();
        assert!(computer_diags.is_empty());
    }

    #[test]
    fn unknown_computer_backend_is_error() {
        let mut args = base_args();
        args.computer_executor = "unknown-backend".into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "computer_executor" && d.severity == Severity::Error));
    }

    #[test]
    fn empty_mcp_config_produces_no_diags() {
        let args = base_args();
        let diags = validate(&args);
        let mcp_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.category == "mcp_executor")
            .collect();
        assert!(mcp_diags.is_empty());
    }

    #[test]
    fn invalid_mcp_json_is_error() {
        let mut args = base_args();
        args.mcp_executor_config = "not json".into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "mcp_executor" && d.severity == Severity::Error));
    }

    #[test]
    fn mcp_server_without_command_is_error() {
        let mut args = base_args();
        args.mcp_executor_config = r#"{"test":{"no_command":true}}"#.into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "mcp_executor" && d.severity == Severity::Error));
    }

    #[test]
    fn mcp_server_read_only_is_info() {
        let mut args = base_args();
        args.mcp_executor_config =
            r#"{"filesystem":{"command":"ls","args":["/tmp"],"read_only":true}}"#.into();
        let diags = validate(&args);
        assert!(diags
            .iter()
            .any(|d| d.category == "mcp_executor" && d.message.contains("只读模式")));
    }
}
