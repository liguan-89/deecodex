use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

/// 跨平台获取用户 HOME 目录
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            return Some(PathBuf::from(home));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return Some(PathBuf::from(home));
    }
    None
}

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(name = "deecodex", about = "Responses API <-> Chat Completions bridge")]
pub struct Args {
    #[command(subcommand)]
    #[serde(skip)]
    pub command: Option<Commands>,

    /// 配置文件路径 (JSON 格式)，TUI 模式下自动加载
    #[arg(long, env = "DEECODEX_CONFIG")]
    #[serde(skip)]
    pub config: Option<String>,

    #[arg(long, env = "DEECODEX_PORT", default_value = "4446")]
    pub port: u16,

    #[arg(
        long,
        env = "DEECODEX_UPSTREAM",
        default_value = "https://openrouter.ai/api/v1"
    )]
    pub upstream: String,

    #[arg(long, env = "DEECODEX_API_KEY", default_value = "")]
    pub api_key: String,

    #[arg(long, env = "DEECODEX_MODEL_MAP", default_value = "{}")]
    pub model_map: String,

    #[arg(long, env = "DEECODEX_MAX_BODY_MB", default_value = "100")]
    pub max_body_mb: usize,

    #[arg(long, env = "DEECODEX_VISION_UPSTREAM", default_value = "")]
    pub vision_upstream: String,

    #[arg(long, env = "DEECODEX_VISION_API_KEY", default_value = "")]
    pub vision_api_key: String,

    #[arg(long, env = "DEECODEX_VISION_MODEL", default_value = "MiniMax-M1")]
    pub vision_model: String,

    #[arg(
        long,
        env = "DEECODEX_VISION_ENDPOINT",
        default_value = "v1/coding_plan/vlm"
    )]
    pub vision_endpoint: String,

    #[arg(long, env = "DEECODEX_CHINESE_THINKING", default_value = "false")]
    pub chinese_thinking: bool,

    /// 启动/关闭时自动注入/移除 codex 配置（默认开启）
    #[arg(long, env = "DEECODEX_CODEX_AUTO_INJECT", default_value = "true")]
    pub codex_auto_inject: bool,

    /// 持久注入 codex 配置，开启后不再自动注入/移除（默认关闭）
    #[arg(
        long,
        env = "DEECODEX_CODEX_PERSISTENT_INJECT",
        default_value = "false"
    )]
    pub codex_persistent_inject: bool,

    /// 启动 deecodex 时自动启动 Codex 桌面版（带 CDP 调试端口）。
    #[arg(
        long = "cdp",
        visible_alias = "codex-launch-with-cdp",
        env = "DEECODEX_CODEX_LAUNCH_WITH_CDP",
        default_value = "false",
        global = true
    )]
    pub codex_launch_with_cdp: bool,

    /// Codex CDP 调试端口。
    #[arg(
        long = "cdp-port",
        env = "DEECODEX_CDP_PORT",
        default_value = "9222",
        global = true
    )]
    pub cdp_port: u16,

    #[arg(long, env = "DEECODEX_PROMPTS_DIR", default_value = "prompts")]
    pub prompts_dir: PathBuf,

    #[arg(long, env = "DEECODEX_DATA_DIR", default_value = ".deecodex")]
    pub data_dir: PathBuf,

    /// Token 异常检测：最大提示词 token 数 (0 禁用)。
    #[arg(
        long,
        env = "DEECODEX_TOKEN_ANOMALY_PROMPT_MAX",
        default_value = "200000"
    )]
    pub token_anomaly_prompt_max: u32,

    /// Token 异常检测：相对滑动平均的飙升比率阈值 (0 禁用)。
    #[arg(
        long,
        env = "DEECODEX_TOKEN_ANOMALY_SPIKE_RATIO",
        default_value = "5.0"
    )]
    pub token_anomaly_spike_ratio: f64,

    /// Token 异常检测：燃烧速率统计窗口 (秒)。
    #[arg(
        long,
        env = "DEECODEX_TOKEN_ANOMALY_BURN_WINDOW",
        default_value = "120"
    )]
    pub token_anomaly_burn_window: u64,

    /// Token 异常检测：燃烧速率告警阈值 (tokens/分钟, 0 禁用)。
    #[arg(
        long,
        env = "DEECODEX_TOKEN_ANOMALY_BURN_RATE",
        default_value = "500000"
    )]
    pub token_anomaly_burn_rate: u32,

    /// 可选的 MCP 服务器白名单 (逗号分隔: server_label/server_url/name)。
    #[arg(long, env = "DEECODEX_ALLOWED_MCP_SERVERS", default_value = "")]
    pub allowed_mcp_servers: String,

    /// 可选的 computer_use 显示器/环境白名单 (逗号分隔)。
    #[arg(long, env = "DEECODEX_ALLOWED_COMPUTER_DISPLAYS", default_value = "")]
    pub allowed_computer_displays: String,

    /// computer_use 本地执行器后端：disabled/playwright/browser-use。
    #[arg(long, env = "DEECODEX_COMPUTER_EXECUTOR", default_value = "disabled")]
    pub computer_executor: String,

    /// computer_use 本地执行器单步超时秒数。
    #[arg(
        long,
        env = "DEECODEX_COMPUTER_EXECUTOR_TIMEOUT_SECS",
        default_value = "30"
    )]
    pub computer_executor_timeout_secs: u64,

    /// MCP 本地执行器配置。可传 JSON 对象/数组，或 JSON 文件路径。
    #[arg(long, env = "DEECODEX_MCP_EXECUTOR_CONFIG", default_value = "")]
    pub mcp_executor_config: String,

    /// MCP 本地执行器单次工具调用超时秒数。
    #[arg(long, env = "DEECODEX_MCP_EXECUTOR_TIMEOUT_SECS", default_value = "30")]
    pub mcp_executor_timeout_secs: u64,

    /// Playwright 后端的持久化浏览器状态目录；设置后按 display 复用 cookies/localStorage 和上次 URL。
    #[arg(long, env = "DEECODEX_PLAYWRIGHT_STATE_DIR", default_value = "")]
    pub playwright_state_dir: String,

    /// browser-use 后端 HTTP bridge 地址；接收 {call_id, display, action} 并返回 JSON output。
    #[arg(long, env = "DEECODEX_BROWSER_USE_BRIDGE_URL", default_value = "")]
    pub browser_use_bridge_url: String,

    /// browser-use 后端命令 bridge；通过 DEECODEX_COMPUTER_ACTION 环境变量接收 JSON 并向 stdout 输出 JSON。
    #[arg(long, env = "DEECODEX_BROWSER_USE_BRIDGE_COMMAND", default_value = "")]
    pub browser_use_bridge_command: String,

    /// 后台守护模式（内部使用）
    #[arg(long, hide = true)]
    #[serde(skip)]
    pub daemon: bool,
}

#[derive(Parser, Debug, Clone)]
pub enum Commands {
    /// 后台启动服务
    Start,
    /// 停止后台服务
    Stop,
    /// 重启后台服务
    Restart,
    /// 查看服务运行状态
    Status,
    /// 查看服务日志
    Logs,
    /// 检测并修复 Codex config.toml 中的已知错误值
    FixConfig,
    /// 运行全链路执行诊断
    Diagnose,
}

impl Args {
    /// 将当前配置保存为 JSON 文件
    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// 从 JSON 文件加载配置
    pub fn load_from_file(path: &std::path::Path) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<Args>(&content).ok()
    }

    /// 默认配置文件路径
    pub fn default_config_path(data_dir: &std::path::Path) -> PathBuf {
        data_dir.join("config.json")
    }

    /// 将指定键值同步写入 .env 文件（按 main.rs 加载顺序查找）
    #[allow(dead_code)]
    pub fn sync_to_env_file(data_dir: &std::path::Path, key: &str, value: &str) {
        let env_path = Self::find_env_file(data_dir);
        let path = match &env_path {
            Some(p) => p.clone(),
            None => {
                // 默认写入 ~/.deecodex/.env
                home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".deecodex")
                    .join(".env")
            }
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let prefix = format!("{}=", key);
        let new_line = format!("{}={}", key, value);
        let replaced = if content.lines().any(|l| l.starts_with(&prefix)) {
            // 替换已有行
            content
                .lines()
                .map(|l| {
                    if l.starts_with(&prefix) {
                        new_line.as_str()
                    } else {
                        l
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            // 追加新行
            if content.is_empty() {
                new_line
            } else {
                format!("{}\n{}", content, new_line)
            }
        };
        let _ = std::fs::write(&path, replaced);
    }

    fn find_env_file(data_dir: &std::path::Path) -> Option<PathBuf> {
        // 按 main.rs 加载顺序：cwd → ~/.deecodex/ → exe目录
        if std::path::Path::new(".env").exists() {
            return Some(PathBuf::from(".env"));
        }
        let home = home_dir()?;
        let home_env = home.join(".deecodex").join(".env");
        if home_env.exists() {
            return Some(home_env);
        }
        let exe_dir = std::env::current_exe()
            .ok()?
            .parent()
            .map(|d| d.join(".env"))?;
        if exe_dir.exists() {
            return Some(exe_dir);
        }
        Some(data_dir.join(".env"))
    }

    /// 遮蔽 API key 等敏感字段：前4字符 + *** + 后4字符。
    /// 空字符串返回空字符串。
    #[allow(dead_code)]
    pub fn mask_sensitive(value: &str) -> String {
        if value.is_empty() {
            return String::new();
        }
        if value.len() <= 8 {
            return "****".to_string();
        }
        let prefix = &value[..4];
        let suffix = &value[value.len() - 4..];
        format!("{}***{}", prefix, suffix)
    }

    /// 完全遮蔽敏感字段
    #[allow(dead_code)]
    pub fn mask_full(value: &str) -> String {
        if value.is_empty() {
            String::new()
        } else {
            "********".to_string()
        }
    }

    /// 从文件加载配置并与当前 CLI/env 值合并。
    /// 规则：CLI/env 值若非默认值则优先，否则使用文件中的值。
    pub fn merge_with_file(self) -> Self {
        let config_path = match &self.config {
            Some(path) if !path.is_empty() => std::path::PathBuf::from(path),
            _ => Self::default_config_path(&self.data_dir),
        };

        if let Some(file) = Self::load_from_file(&config_path) {
            Args {
                command: self.command,
                config: self.config,
                port: pick(self.port, 4446, file.port),
                upstream: pick_str(
                    &self.upstream,
                    "https://openrouter.ai/api/v1",
                    &file.upstream,
                ),
                api_key: pick_str(&self.api_key, "", &file.api_key),
                model_map: pick_str(&self.model_map, "{}", &file.model_map),
                max_body_mb: pick(self.max_body_mb, 100, file.max_body_mb),
                vision_upstream: pick_str(&self.vision_upstream, "", &file.vision_upstream),
                vision_api_key: pick_str(&self.vision_api_key, "", &file.vision_api_key),
                vision_model: pick_str(&self.vision_model, "MiniMax-M1", &file.vision_model),
                vision_endpoint: pick_str(
                    &self.vision_endpoint,
                    "v1/coding_plan/vlm",
                    &file.vision_endpoint,
                ),
                chinese_thinking: pick(self.chinese_thinking, false, file.chinese_thinking),
                codex_auto_inject: pick(self.codex_auto_inject, true, file.codex_auto_inject),
                codex_persistent_inject: pick(
                    self.codex_persistent_inject,
                    false,
                    file.codex_persistent_inject,
                ),
                codex_launch_with_cdp: pick(
                    self.codex_launch_with_cdp,
                    false,
                    file.codex_launch_with_cdp,
                ),
                cdp_port: pick(self.cdp_port, 9222, file.cdp_port),
                prompts_dir: if self.prompts_dir.as_path() == std::path::Path::new("prompts") {
                    file.prompts_dir
                } else {
                    self.prompts_dir
                },
                data_dir: if self.data_dir.as_path() == std::path::Path::new(".deecodex") {
                    file.data_dir
                } else {
                    self.data_dir
                },
                token_anomaly_prompt_max: pick(
                    self.token_anomaly_prompt_max,
                    200000,
                    file.token_anomaly_prompt_max,
                ),
                token_anomaly_spike_ratio: pick_f64(
                    self.token_anomaly_spike_ratio,
                    5.0,
                    file.token_anomaly_spike_ratio,
                ),
                token_anomaly_burn_window: pick(
                    self.token_anomaly_burn_window,
                    120,
                    file.token_anomaly_burn_window,
                ),
                token_anomaly_burn_rate: pick(
                    self.token_anomaly_burn_rate,
                    500000,
                    file.token_anomaly_burn_rate,
                ),
                allowed_mcp_servers: pick_str(
                    &self.allowed_mcp_servers,
                    "",
                    &file.allowed_mcp_servers,
                ),
                allowed_computer_displays: pick_str(
                    &self.allowed_computer_displays,
                    "",
                    &file.allowed_computer_displays,
                ),
                computer_executor: pick_str(
                    &self.computer_executor,
                    "disabled",
                    &file.computer_executor,
                ),
                computer_executor_timeout_secs: pick(
                    self.computer_executor_timeout_secs,
                    30,
                    file.computer_executor_timeout_secs,
                ),
                mcp_executor_config: pick_str(
                    &self.mcp_executor_config,
                    "",
                    &file.mcp_executor_config,
                ),
                mcp_executor_timeout_secs: pick(
                    self.mcp_executor_timeout_secs,
                    30,
                    file.mcp_executor_timeout_secs,
                ),
                playwright_state_dir: pick_str(
                    &self.playwright_state_dir,
                    "",
                    &file.playwright_state_dir,
                ),
                browser_use_bridge_url: pick_str(
                    &self.browser_use_bridge_url,
                    "",
                    &file.browser_use_bridge_url,
                ),
                browser_use_bridge_command: pick_str(
                    &self.browser_use_bridge_command,
                    "",
                    &file.browser_use_bridge_command,
                ),
                daemon: self.daemon,
            }
        } else {
            self
        }
    }
}

/// cli 值非默认时用 cli（env 优先），否则用文件值
fn pick<T: PartialEq>(cli_val: T, default: T, file_val: T) -> T {
    if cli_val == default {
        file_val
    } else {
        cli_val
    }
}

fn pick_f64(cli_val: f64, default: f64, file_val: f64) -> f64 {
    if (cli_val - default).abs() < f64::EPSILON {
        file_val
    } else {
        cli_val
    }
}

fn pick_str(cli_val: &str, default: &str, file_val: &str) -> String {
    if cli_val == default {
        file_val.to_string()
    } else {
        cli_val.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_args(data_dir: PathBuf) -> Args {
        Args {
            command: None,
            config: None,
            port: 4446,
            upstream: "https://openrouter.ai/api/v1".into(),
            api_key: String::new(),
            model_map: "{}".into(),
            max_body_mb: 100,
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: "MiniMax-M1".into(),
            vision_endpoint: "v1/coding_plan/vlm".into(),
            chinese_thinking: false,
            codex_auto_inject: true,
            codex_persistent_inject: false,
            codex_launch_with_cdp: false,
            cdp_port: 9222,
            prompts_dir: PathBuf::from("prompts"),
            data_dir,
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
    fn config_merge_loads_tool_policy_from_file_when_cli_is_default() {
        let dir = std::env::temp_dir().join(format!("deecodex-config-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");
        let file_args = Args {
            command: None,
            config: None,
            port: 5555,
            upstream: "https://example.com/api/v1".into(),
            api_key: "upstream-key".into(),
            model_map: "{}".into(),
            max_body_mb: 100,
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: "MiniMax-M1".into(),
            vision_endpoint: "v1/coding_plan/vlm".into(),
            chinese_thinking: false,
            codex_auto_inject: true,
            codex_persistent_inject: false,
            codex_launch_with_cdp: false,
            cdp_port: 9222,
            prompts_dir: PathBuf::from("prompts"),
            data_dir: dir.clone(),
            token_anomaly_prompt_max: 200000,
            token_anomaly_spike_ratio: 5.0,
            token_anomaly_burn_window: 120,
            token_anomaly_burn_rate: 500000,
            allowed_mcp_servers: "filesystem,github".into(),
            allowed_computer_displays: "browser".into(),
            computer_executor: "playwright".into(),
            computer_executor_timeout_secs: 15,
            mcp_executor_config:
                r#"{"filesystem":{"label":"","command":"mcp-filesystem","args":["/tmp"]}}"#.into(),
            mcp_executor_timeout_secs: 12,
            playwright_state_dir: String::new(),
            browser_use_bridge_url: String::new(),
            browser_use_bridge_command: String::new(),
            daemon: false,
        };
        file_args.save_to_file(&config_path).unwrap();

        let cli_args = Args {
            command: None,
            config: Some(config_path.to_string_lossy().to_string()),
            port: 4446,
            upstream: "https://openrouter.ai/api/v1".into(),
            api_key: String::new(),
            model_map: "{}".into(),
            max_body_mb: 100,
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: "MiniMax-M1".into(),
            vision_endpoint: "v1/coding_plan/vlm".into(),
            chinese_thinking: false,
            codex_auto_inject: true,
            codex_persistent_inject: false,
            codex_launch_with_cdp: false,
            cdp_port: 9222,
            prompts_dir: PathBuf::from("prompts"),
            data_dir: PathBuf::from(".deecodex"),
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
        };

        let merged = cli_args.merge_with_file();

        assert_eq!(merged.allowed_mcp_servers, "filesystem,github");
        assert_eq!(merged.allowed_computer_displays, "browser");
        assert_eq!(merged.computer_executor, "playwright");
        assert_eq!(merged.computer_executor_timeout_secs, 15);
        assert!(merged.mcp_executor_config.contains("mcp-filesystem"));
        assert_eq!(merged.mcp_executor_timeout_secs, 12);
        assert_eq!(merged.port, 5555);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_merge_loads_checkbox_values_from_file_when_cli_is_default() {
        let dir = std::env::temp_dir().join(format!("deecodex-config-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");
        let mut file_args = test_args(dir.clone());
        file_args.chinese_thinking = true;
        file_args.codex_auto_inject = false;
        file_args.codex_persistent_inject = true;
        file_args.codex_launch_with_cdp = true;
        file_args.cdp_port = 9333;
        file_args.save_to_file(&config_path).unwrap();

        let mut cli_args = test_args(PathBuf::from(".deecodex"));
        cli_args.config = Some(config_path.to_string_lossy().to_string());

        let merged = cli_args.merge_with_file();

        assert!(merged.chinese_thinking);
        assert!(!merged.codex_auto_inject);
        assert!(merged.codex_persistent_inject);
        assert!(merged.codex_launch_with_cdp);
        assert_eq!(merged.cdp_port, 9333);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_merge_keeps_explicit_checkbox_over_file_value() {
        let dir = std::env::temp_dir().join(format!("deecodex-config-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.json");
        let mut file_args = test_args(dir.clone());
        file_args.chinese_thinking = false;
        file_args.codex_auto_inject = true;
        file_args.codex_persistent_inject = false;
        file_args.codex_launch_with_cdp = false;
        file_args.save_to_file(&config_path).unwrap();

        let mut cli_args = test_args(PathBuf::from(".deecodex"));
        cli_args.config = Some(config_path.to_string_lossy().to_string());
        cli_args.chinese_thinking = true;
        cli_args.codex_auto_inject = false;
        cli_args.codex_persistent_inject = true;
        cli_args.codex_launch_with_cdp = true;

        let merged = cli_args.merge_with_file();

        assert!(merged.chinese_thinking);
        assert!(!merged.codex_auto_inject);
        assert!(merged.codex_persistent_inject);
        assert!(merged.codex_launch_with_cdp);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
