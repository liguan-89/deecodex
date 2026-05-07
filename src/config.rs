use std::path::PathBuf;

use clap::Parser;
use serde::{Deserialize, Serialize};

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

    #[arg(long, env = "DEECODEX_PORT", default_value = "4444")]
    pub port: u16,

    #[arg(
        long,
        env = "DEECODEX_UPSTREAM",
        default_value = "https://openrouter.ai/api/v1"
    )]
    pub upstream: String,

    #[arg(long, env = "DEECODEX_API_KEY", default_value = "")]
    pub api_key: String,

    /// 客户端调用所需的 Bearer token。为空则禁用本地认证。
    #[arg(long, env = "DEECODEX_CLIENT_API_KEY", default_value = "")]
    pub client_api_key: String,

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
    #[arg(
        long,
        env = "DEECODEX_ALLOWED_COMPUTER_DISPLAYS",
        default_value = ""
    )]
    pub allowed_computer_displays: String,

    /// 后台守护模式（内部使用）
    #[arg(long, hide = true)]
    #[serde(skip)]
    pub daemon: bool,
}

#[derive(Parser, Debug, Clone)]
pub enum Commands {
    /// 启动中文 TUI 交互配置菜单
    Tui,
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
                port: pick(self.port, 4444, file.port),
                upstream: pick_str(&self.upstream, "https://openrouter.ai/api/v1", &file.upstream),
                api_key: pick_str(&self.api_key, "", &file.api_key),
                client_api_key: pick_str(&self.client_api_key, "", &file.client_api_key),
                model_map: pick_str(&self.model_map, "{}", &file.model_map),
                max_body_mb: pick(self.max_body_mb, 100, file.max_body_mb),
                vision_upstream: pick_str(&self.vision_upstream, "", &file.vision_upstream),
                vision_api_key: pick_str(&self.vision_api_key, "", &file.vision_api_key),
                vision_model: pick_str(&self.vision_model, "MiniMax-M1", &file.vision_model),
                vision_endpoint: pick_str(&self.vision_endpoint, "v1/coding_plan/vlm", &file.vision_endpoint),
                chinese_thinking: self.chinese_thinking || file.chinese_thinking,
                prompts_dir: if self.prompts_dir == PathBuf::from("prompts") { file.prompts_dir } else { self.prompts_dir },
                data_dir: if self.data_dir == PathBuf::from(".deecodex") { file.data_dir } else { self.data_dir },
                token_anomaly_prompt_max: pick(self.token_anomaly_prompt_max, 200000, file.token_anomaly_prompt_max),
                token_anomaly_spike_ratio: pick_f64(self.token_anomaly_spike_ratio, 5.0, file.token_anomaly_spike_ratio),
                token_anomaly_burn_window: pick(self.token_anomaly_burn_window, 120, file.token_anomaly_burn_window),
                token_anomaly_burn_rate: pick(self.token_anomaly_burn_rate, 500000, file.token_anomaly_burn_rate),
                allowed_mcp_servers: pick_str(&self.allowed_mcp_servers, "", &file.allowed_mcp_servers),
                allowed_computer_displays: pick_str(&self.allowed_computer_displays, "", &file.allowed_computer_displays),
                daemon: self.daemon,
            }
        } else {
            self
        }
    }
}

/// cli 值非默认时用 cli（env 优先），否则用文件值
fn pick<T: PartialEq>(cli_val: T, default: T, file_val: T) -> T {
    if cli_val == default { file_val } else { cli_val }
}

fn pick_f64(cli_val: f64, default: f64, file_val: f64) -> f64 {
    if (cli_val - default).abs() < f64::EPSILON { file_val } else { cli_val }
}

fn pick_str(cli_val: &str, default: &str, file_val: &str) -> String {
    if cli_val == default { file_val.to_string() } else { cli_val.to_string() }
}
