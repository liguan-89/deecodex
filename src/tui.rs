use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use tui_textarea::{Input, TextArea};
use unicode_width::UnicodeWidthStr;

use crate::config::Args;

// ── Screen ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
enum Screen {
    MainMenu,
    BasicSettings,
    UpstreamSettings,
    VisionSettings,
    TokenAnomaly,
    ToolPolicy,
    HealthCheck,
    ReviewConfirm,
}

impl Screen {
    fn title(&self) -> &'static str {
        match self {
            Screen::MainMenu => "主菜单",
            Screen::BasicSettings => "基础设置",
            Screen::UpstreamSettings => "上游设置",
            Screen::VisionSettings => "视觉模型设置",
            Screen::TokenAnomaly => "Token 异常检测",
            Screen::ToolPolicy => "工具安全策略",
            Screen::HealthCheck => "快速检测",
            Screen::ReviewConfirm => "确认并启动",
        }
    }
}

// ── Field Definition ────────────────────────────────────────────────────────

#[derive(Clone)]
enum FieldKind {
    Text,
    Password,
    Number,
    Bool,
    JsonText,
    CsvList,
    Path,
    Action { target: Screen },
}

#[derive(Clone)]
struct FieldDef {
    label: &'static str,
    key: &'static str,
    kind: FieldKind,
    help: &'static str,
}

// ── TUI App State ───────────────────────────────────────────────────────────

#[derive(Clone)]
struct TuiAppState {
    // Basic
    port: String,
    max_body_mb: String,
    data_dir: String,
    prompts_dir: String,
    chinese_thinking: bool,

    // Upstream
    upstream: String,
    api_key: String,
    client_api_key: String,
    model_map: String,

    // Vision
    vision_upstream: String,
    vision_api_key: String,
    vision_model: String,
    vision_endpoint: String,

    // Token anomaly
    token_anomaly_prompt_max: String,
    token_anomaly_spike_ratio: String,
    token_anomaly_burn_window: String,
    token_anomaly_burn_rate: String,

    // Tool policy
    allowed_mcp_servers: String,
    allowed_computer_displays: String,
    computer_executor: String,
    computer_executor_timeout_secs: String,
    mcp_executor_config: String,
    mcp_executor_timeout_secs: String,
    playwright_state_dir: String,
    browser_use_bridge_url: String,
    browser_use_bridge_command: String,

    // Navigation
    config_path: String, // 当前使用的配置文件路径
    current_screen: Screen,
    selection_index: usize,
    editing_textarea: Option<(String, TextArea<'static>)>, // field_key, editor
    status_message: Option<String>,
    should_launch: bool,
    should_quit: bool,
}

impl TuiAppState {
    fn from_args(args: &Args) -> Self {
        TuiAppState {
            port: args.port.to_string(),
            max_body_mb: args.max_body_mb.to_string(),
            data_dir: args.data_dir.display().to_string(),
            prompts_dir: args.prompts_dir.display().to_string(),
            chinese_thinking: args.chinese_thinking,

            upstream: args.upstream.clone(),
            api_key: args.api_key.clone(),
            client_api_key: args.client_api_key.clone(),
            model_map: if args.model_map == "{}" {
                String::new()
            } else {
                args.model_map.clone()
            },

            vision_upstream: args.vision_upstream.clone(),
            vision_api_key: args.vision_api_key.clone(),
            vision_model: args.vision_model.clone(),
            vision_endpoint: args.vision_endpoint.clone(),

            token_anomaly_prompt_max: args.token_anomaly_prompt_max.to_string(),
            token_anomaly_spike_ratio: args.token_anomaly_spike_ratio.to_string(),
            token_anomaly_burn_window: args.token_anomaly_burn_window.to_string(),
            token_anomaly_burn_rate: args.token_anomaly_burn_rate.to_string(),

            allowed_mcp_servers: args.allowed_mcp_servers.clone(),
            allowed_computer_displays: args.allowed_computer_displays.clone(),
            computer_executor: args.computer_executor.clone(),
            computer_executor_timeout_secs: args.computer_executor_timeout_secs.to_string(),
            mcp_executor_config: args.mcp_executor_config.clone(),
            mcp_executor_timeout_secs: args.mcp_executor_timeout_secs.to_string(),
            playwright_state_dir: args.playwright_state_dir.clone(),
            browser_use_bridge_url: args.browser_use_bridge_url.clone(),
            browser_use_bridge_command: args.browser_use_bridge_command.clone(),

            config_path: String::new(),
            current_screen: Screen::MainMenu,
            selection_index: 0,
            editing_textarea: None,
            status_message: None,
            should_launch: false,
            should_quit: false,
        }
    }

    fn into_args(self) -> Result<Args> {
        let model_map = if self.model_map.trim().is_empty() {
            "{}".to_string()
        } else {
            self.model_map
        };

        Ok(Args {
            command: None,
            config: None,
            port: self
                .port
                .parse()
                .map_err(|_| anyhow::anyhow!("端口号无效: {}", self.port))?,
            upstream: self.upstream,
            api_key: self.api_key,
            client_api_key: self.client_api_key,
            model_map,
            max_body_mb: self
                .max_body_mb
                .parse()
                .map_err(|_| anyhow::anyhow!("最大请求体大小无效"))?,
            vision_upstream: self.vision_upstream,
            vision_api_key: self.vision_api_key,
            vision_model: self.vision_model,
            vision_endpoint: self.vision_endpoint,
            chinese_thinking: self.chinese_thinking,
            prompts_dir: PathBuf::from(self.prompts_dir),
            data_dir: PathBuf::from(self.data_dir),
            token_anomaly_prompt_max: self
                .token_anomaly_prompt_max
                .parse()
                .map_err(|_| anyhow::anyhow!("Token 提示词最大值无效"))?,
            token_anomaly_spike_ratio: self
                .token_anomaly_spike_ratio
                .parse()
                .map_err(|_| anyhow::anyhow!("Token 飙升比率无效"))?,
            token_anomaly_burn_window: self
                .token_anomaly_burn_window
                .parse()
                .map_err(|_| anyhow::anyhow!("Token 燃烧窗口无效"))?,
            token_anomaly_burn_rate: self
                .token_anomaly_burn_rate
                .parse()
                .map_err(|_| anyhow::anyhow!("Token 燃烧速率无效"))?,
            allowed_mcp_servers: self.allowed_mcp_servers,
            allowed_computer_displays: self.allowed_computer_displays,
            computer_executor: self.computer_executor,
            computer_executor_timeout_secs: self
                .computer_executor_timeout_secs
                .parse()
                .map_err(|_| anyhow::anyhow!("computer executor 超时无效"))?,
            mcp_executor_config: self.mcp_executor_config,
            mcp_executor_timeout_secs: self
                .mcp_executor_timeout_secs
                .parse()
                .map_err(|_| anyhow::anyhow!("MCP executor 超时无效"))?,
            playwright_state_dir: self.playwright_state_dir,
            browser_use_bridge_url: self.browser_use_bridge_url,
            browser_use_bridge_command: self.browser_use_bridge_command,
            daemon: false,
        })
    }

    /// 获取原始值（用于编辑），不返回显示占位符
    fn get_raw_value(&self, key: &str) -> String {
        match key {
            "port" => self.port.clone(),
            "max_body_mb" => self.max_body_mb.clone(),
            "data_dir" => self.data_dir.clone(),
            "prompts_dir" => self.prompts_dir.clone(),
            "chinese_thinking" => self.chinese_thinking.to_string(),
            "upstream" => self.upstream.clone(),
            "api_key" => self.api_key.clone(),
            "client_api_key" => self.client_api_key.clone(),
            "model_map" => self.model_map.clone(),
            "vision_upstream" => self.vision_upstream.clone(),
            "vision_api_key" => self.vision_api_key.clone(),
            "vision_model" => self.vision_model.clone(),
            "vision_endpoint" => self.vision_endpoint.clone(),
            "token_anomaly_prompt_max" => self.token_anomaly_prompt_max.clone(),
            "token_anomaly_spike_ratio" => self.token_anomaly_spike_ratio.clone(),
            "token_anomaly_burn_window" => self.token_anomaly_burn_window.clone(),
            "token_anomaly_burn_rate" => self.token_anomaly_burn_rate.clone(),
            "allowed_mcp_servers" => self.allowed_mcp_servers.clone(),
            "allowed_computer_displays" => self.allowed_computer_displays.clone(),
            "computer_executor" => self.computer_executor.clone(),
            "computer_executor_timeout_secs" => self.computer_executor_timeout_secs.clone(),
            "mcp_executor_config" => self.mcp_executor_config.clone(),
            "mcp_executor_timeout_secs" => self.mcp_executor_timeout_secs.clone(),
            "playwright_state_dir" => self.playwright_state_dir.clone(),
            "browser_use_bridge_url" => self.browser_use_bridge_url.clone(),
            "browser_use_bridge_command" => self.browser_use_bridge_command.clone(),
            _ => String::new(),
        }
    }

    fn get_field_value(&self, field: &FieldDef) -> String {
        match field.key {
            "config_path" => self.config_path.clone(),
            "port" => self.port.clone(),
            "max_body_mb" => self.max_body_mb.clone(),
            "data_dir" => self.data_dir.clone(),
            "prompts_dir" => self.prompts_dir.clone(),
            "chinese_thinking" => {
                if self.chinese_thinking {
                    "是".into()
                } else {
                    "否".into()
                }
            }
            "upstream" => self.upstream.clone(),
            "api_key" => self.api_key.clone(),
            "client_api_key" => self.client_api_key.clone(),
            "model_map" => {
                if self.model_map.is_empty() {
                    "(无)".into()
                } else {
                    self.model_map.clone()
                }
            }
            "vision_upstream" => {
                if self.vision_upstream.is_empty() {
                    "(未设置)".into()
                } else {
                    self.vision_upstream.clone()
                }
            }
            "vision_api_key" => self.vision_api_key.clone(),
            "vision_model" => self.vision_model.clone(),
            "vision_endpoint" => self.vision_endpoint.clone(),
            "token_anomaly_prompt_max" => self.token_anomaly_prompt_max.clone(),
            "token_anomaly_spike_ratio" => self.token_anomaly_spike_ratio.clone(),
            "token_anomaly_burn_window" => self.token_anomaly_burn_window.clone(),
            "token_anomaly_burn_rate" => self.token_anomaly_burn_rate.clone(),
            "allowed_mcp_servers" => {
                if self.allowed_mcp_servers.is_empty() {
                    "(无)".into()
                } else {
                    self.allowed_mcp_servers.clone()
                }
            }
            "allowed_computer_displays" => {
                if self.allowed_computer_displays.is_empty() {
                    "(无)".into()
                } else {
                    self.allowed_computer_displays.clone()
                }
            }
            "computer_executor" => self.computer_executor.clone(),
            "computer_executor_timeout_secs" => self.computer_executor_timeout_secs.clone(),
            "mcp_executor_config" => {
                if self.mcp_executor_config.is_empty() {
                    "(未配置)".into()
                } else {
                    self.mcp_executor_config.clone()
                }
            }
            "mcp_executor_timeout_secs" => self.mcp_executor_timeout_secs.clone(),
            "playwright_state_dir" => {
                if self.playwright_state_dir.is_empty() {
                    "(未设置)".into()
                } else {
                    self.playwright_state_dir.clone()
                }
            }
            "browser_use_bridge_url" => {
                if self.browser_use_bridge_url.is_empty() {
                    "(未设置)".into()
                } else {
                    self.browser_use_bridge_url.clone()
                }
            }
            "browser_use_bridge_command" => {
                if self.browser_use_bridge_command.is_empty() {
                    "(未设置)".into()
                } else {
                    self.browser_use_bridge_command.clone()
                }
            }
            _ => String::new(),
        }
    }

    fn set_field_value(&mut self, field: &FieldDef, value: &str) {
        match field.key {
            "port" => self.port = value.to_string(),
            "max_body_mb" => self.max_body_mb = value.to_string(),
            "data_dir" => self.data_dir = value.to_string(),
            "prompts_dir" => self.prompts_dir = value.to_string(),
            "upstream" => self.upstream = value.to_string(),
            "api_key" => self.api_key = value.to_string(),
            "client_api_key" => self.client_api_key = value.to_string(),
            "model_map" => self.model_map = value.to_string(),
            "vision_upstream" => self.vision_upstream = value.to_string(),
            "vision_api_key" => self.vision_api_key = value.to_string(),
            "vision_model" => self.vision_model = value.to_string(),
            "vision_endpoint" => self.vision_endpoint = value.to_string(),
            "token_anomaly_prompt_max" => self.token_anomaly_prompt_max = value.to_string(),
            "token_anomaly_spike_ratio" => self.token_anomaly_spike_ratio = value.to_string(),
            "token_anomaly_burn_window" => self.token_anomaly_burn_window = value.to_string(),
            "token_anomaly_burn_rate" => self.token_anomaly_burn_rate = value.to_string(),
            "allowed_mcp_servers" => self.allowed_mcp_servers = value.to_string(),
            "allowed_computer_displays" => self.allowed_computer_displays = value.to_string(),
            "computer_executor" => self.computer_executor = value.to_string(),
            "computer_executor_timeout_secs" => {
                self.computer_executor_timeout_secs = value.to_string()
            }
            "mcp_executor_config" => self.mcp_executor_config = value.to_string(),
            "mcp_executor_timeout_secs" => self.mcp_executor_timeout_secs = value.to_string(),
            "playwright_state_dir" => self.playwright_state_dir = value.to_string(),
            "browser_use_bridge_url" => self.browser_use_bridge_url = value.to_string(),
            "browser_use_bridge_command" => self.browser_use_bridge_command = value.to_string(),
            _ => {}
        }
    }
}

// ── Field Lists per Screen ──────────────────────────────────────────────────

fn main_menu_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            label: "快速检测",
            key: "",
            kind: FieldKind::Action {
                target: Screen::HealthCheck,
            },
            help: "检查当前配置是否有效、上游是否可达",
        },
        FieldDef {
            label: "基础设置",
            key: "",
            kind: FieldKind::Action {
                target: Screen::BasicSettings,
            },
            help: "端口、数据目录、中文思考模式等",
        },
        FieldDef {
            label: "上游设置",
            key: "",
            kind: FieldKind::Action {
                target: Screen::UpstreamSettings,
            },
            help: "API 地址、密钥、模型映射",
        },
        FieldDef {
            label: "视觉模型设置",
            key: "",
            kind: FieldKind::Action {
                target: Screen::VisionSettings,
            },
            help: "视觉模型上游、密钥、端点",
        },
        FieldDef {
            label: "Token 异常检测",
            key: "",
            kind: FieldKind::Action {
                target: Screen::TokenAnomaly,
            },
            help: "Token 异常检测阈值配置",
        },
        FieldDef {
            label: "工具安全策略",
            key: "",
            kind: FieldKind::Action {
                target: Screen::ToolPolicy,
            },
            help: "MCP 服务器与计算机显示器白名单",
        },
        FieldDef {
            label: "保存当前配置",
            key: "",
            kind: FieldKind::Action {
                target: Screen::MainMenu,
            },
            help: "将当前设置保存到配置文件",
        },
        FieldDef {
            label: "确认并启动服务",
            key: "",
            kind: FieldKind::Action {
                target: Screen::ReviewConfirm,
            },
            help: "复查所有配置并启动服务",
        },
    ]
}

fn basic_settings_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            label: "服务端口",
            key: "port",
            kind: FieldKind::Number,
            help: "本地监听端口 (默认: 4444)",
        },
        FieldDef {
            label: "最大请求体(MB)",
            key: "max_body_mb",
            kind: FieldKind::Number,
            help: "最大请求体大小/兆字节 (默认: 100)",
        },
        FieldDef {
            label: "数据目录",
            key: "data_dir",
            kind: FieldKind::Path,
            help: "本地文件与数据存储目录 (默认: .deecodex)",
        },
        FieldDef {
            label: "提示词目录",
            key: "prompts_dir",
            kind: FieldKind::Path,
            help: "提示词模板加载目录 (默认: prompts)",
        },
        FieldDef {
            label: "中文思考模式",
            key: "chinese_thinking",
            kind: FieldKind::Bool,
            help: "开启后系统提示词将注入中文思考指令",
        },
    ]
}

fn upstream_settings_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            label: "上游 API 地址",
            key: "upstream",
            kind: FieldKind::Text,
            help: "Chat Completions API 地址 (默认: https://openrouter.ai/api/v1)",
        },
        FieldDef {
            label: "API 密钥",
            key: "api_key",
            kind: FieldKind::Password,
            help: "上游 API 访问密钥",
        },
        FieldDef {
            label: "客户端认证密钥",
            key: "client_api_key",
            kind: FieldKind::Password,
            help: "本地调用方所需的 Bearer token，为空则不验证",
        },
        FieldDef {
            label: "模型映射(JSON)",
            key: "model_map",
            kind: FieldKind::JsonText,
            help: r#"例: {"codex-model": "deepseek-model"}"#,
        },
    ]
}

fn vision_settings_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            label: "视觉上游地址",
            key: "vision_upstream",
            kind: FieldKind::Text,
            help: "视觉/截图处理 API 地址，为空则不启用视觉路由",
        },
        FieldDef {
            label: "视觉 API 密钥",
            key: "vision_api_key",
            kind: FieldKind::Password,
            help: "视觉 API 访问密钥",
        },
        FieldDef {
            label: "视觉模型名称",
            key: "vision_model",
            kind: FieldKind::Text,
            help: "视觉模型名 (默认: MiniMax-M1)",
        },
        FieldDef {
            label: "视觉接口路径",
            key: "vision_endpoint",
            kind: FieldKind::Text,
            help: "视觉接口端点路径 (默认: v1/coding_plan/vlm)",
        },
    ]
}

fn token_anomaly_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            label: "提示词 Token 上限",
            key: "token_anomaly_prompt_max",
            kind: FieldKind::Number,
            help: "单次请求提示词最大 Token 数，0 禁用 (默认: 200000)",
        },
        FieldDef {
            label: "飙升比率阈值",
            key: "token_anomaly_spike_ratio",
            kind: FieldKind::Number,
            help: "相对滑动平均的飙升比率，0 禁用 (默认: 5.0)",
        },
        FieldDef {
            label: "燃烧速率窗口(秒)",
            key: "token_anomaly_burn_window",
            kind: FieldKind::Number,
            help: "燃烧速率统计窗口/秒 (默认: 120)",
        },
        FieldDef {
            label: "燃烧速率阈值(tok/分钟)",
            key: "token_anomaly_burn_rate",
            kind: FieldKind::Number,
            help: "燃烧速率告警阈值 token/分钟，0 禁用 (默认: 500000)",
        },
    ]
}

fn tool_policy_fields() -> Vec<FieldDef> {
    vec![
        FieldDef {
            label: "MCP 服务器白名单",
            key: "allowed_mcp_servers",
            kind: FieldKind::CsvList,
            help: "允许的 MCP server_label/server_url/name，逗号分隔，为空不限制",
        },
        FieldDef {
            label: "计算机显示器白名单",
            key: "allowed_computer_displays",
            kind: FieldKind::CsvList,
            help: "允许的 computer_use 显示器/环境，逗号分隔，为空不限制",
        },
        FieldDef {
            label: "computer 执行器",
            key: "computer_executor",
            kind: FieldKind::Text,
            help: "disabled/playwright/browser-use，默认 disabled",
        },
        FieldDef {
            label: "computer 超时(秒)",
            key: "computer_executor_timeout_secs",
            kind: FieldKind::Number,
            help: "computer_use 单步执行超时，默认 30 秒",
        },
        FieldDef {
            label: "MCP 执行器配置",
            key: "mcp_executor_config",
            kind: FieldKind::JsonText,
            help: "MCP server JSON 对象/数组，或 JSON 文件路径",
        },
        FieldDef {
            label: "MCP 超时(秒)",
            key: "mcp_executor_timeout_secs",
            kind: FieldKind::Number,
            help: "MCP 单次工具调用超时，默认 30 秒",
        },
        FieldDef {
            label: "Playwright 状态目录",
            key: "playwright_state_dir",
            kind: FieldKind::Text,
            help: "Playwright 浏览器状态持久化目录（可选）；设置后按 display 复用 cookies/localStorage 和上次 URL",
        },
        FieldDef {
            label: "browser-use Bridge URL",
            key: "browser_use_bridge_url",
            kind: FieldKind::Text,
            help: "browser-use 后端 HTTP bridge 地址；与 Bridge 命令二选一",
        },
        FieldDef {
            label: "browser-use Bridge 命令",
            key: "browser_use_bridge_command",
            kind: FieldKind::Text,
            help: "browser-use 后端命令 bridge；与 Bridge URL 二选一",
        },
    ]
}

fn fields_for_screen(screen: &Screen) -> Vec<FieldDef> {
    match screen {
        Screen::MainMenu => main_menu_fields(),
        Screen::BasicSettings => basic_settings_fields(),
        Screen::UpstreamSettings => upstream_settings_fields(),
        Screen::VisionSettings => vision_settings_fields(),
        Screen::TokenAnomaly => token_anomaly_fields(),
        Screen::ToolPolicy => tool_policy_fields(),
        Screen::HealthCheck => vec![],
        Screen::ReviewConfirm => vec![],
    }
}

// ── Public Entry Point ──────────────────────────────────────────────────────

pub async fn run(initial_args: Args) -> Option<Args> {
    let mut terminal = ratatui::init();

    // 尝试加载配置文件
    let state = load_config_and_merge(&initial_args);

    let result = run_app(&mut terminal, state).await;
    ratatui::restore();

    // 如果用户确认启动，自动保存配置
    if let Some(ref args) = result {
        let config_path = Args::default_config_path(&args.data_dir);
        if let Err(e) = args.save_to_file(&config_path) {
            tracing::warn!("保存配置文件失败: {}", e);
        }
    }

    result
}

fn load_config_and_merge(cli_args: &Args) -> TuiAppState {
    let cli = cli_args.clone();
    let merged = cli.merge_with_file();

    let config_path = match &merged.config {
        Some(path) if !path.is_empty() => std::path::PathBuf::from(path),
        _ => Args::default_config_path(&merged.data_dir),
    };

    let mut state = TuiAppState::from_args(&merged);
    state.config_path = config_path.display().to_string();
    if Args::load_from_file(&config_path).is_some() {
        state.status_message = Some(format!("已加载: {}", state.config_path));
    }
    state
}

// ── Event Loop ──────────────────────────────────────────────────────────────

async fn run_app(terminal: &mut ratatui::DefaultTerminal, mut state: TuiAppState) -> Option<Args> {
    loop {
        terminal
            .draw(|f| render(f, &mut state))
            .expect("terminal draw failed");

        // 清除一次性状态消息
        state.status_message = None;

        if state.should_launch {
            return state.into_args().ok();
        }
        if state.should_quit {
            return None;
        }

        // 等待事件
        if let Ok(Event::Key(key)) = event::read() {
            if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                handle_key_event(key, &mut state);
            }
        }
    }
}

// ── Key Event Handling ──────────────────────────────────────────────────────

fn handle_key_event(key: event::KeyEvent, state: &mut TuiAppState) {
    // 如果在文本编辑模式，优先处理
    if let Some(ref mut edit) = state.editing_textarea {
        match key.code {
            KeyCode::Esc => {
                state.editing_textarea = None;
            }
            KeyCode::Enter => {
                // 确认编辑
                let value = edit.1.lines().join("");
                let field_key = &edit.0;
                let fields = fields_for_screen(&state.current_screen);
                if let Some(field) = fields.iter().find(|f| f.key == *field_key) {
                    state.set_field_value(field, &value);
                }
                state.editing_textarea = None;
            }
            _ => {
                edit.1.input(Input::from(key));
            }
        }
        return;
    }

    // 导航模式
    match state.current_screen {
        Screen::ReviewConfirm => handle_review_key(key, state),
        _ => handle_navigation(key, state),
    }
}

fn handle_navigation(key: event::KeyEvent, state: &mut TuiAppState) {
    let fields = fields_for_screen(&state.current_screen);

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.selection_index = state.selection_index.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') if state.selection_index + 1 < fields.len() => {
            state.selection_index += 1;
        }
        KeyCode::Enter => {
            if let Some(field) = fields.get(state.selection_index) {
                activate_field(field, state);
            }
        }
        KeyCode::Esc => {
            if state.current_screen == Screen::MainMenu {
                state.should_quit = true;
            } else {
                state.current_screen = Screen::MainMenu;
                state.selection_index = 0;
            }
        }
        KeyCode::Char('q') if state.current_screen == Screen::MainMenu => {
            state.should_quit = true;
        }
        _ => {}
    }
}

fn handle_review_key(key: event::KeyEvent, state: &mut TuiAppState) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            state.should_launch = true;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.current_screen = Screen::MainMenu;
            state.selection_index = 0;
        }
        _ => {}
    }
}

fn activate_field(field: &FieldDef, state: &mut TuiAppState) {
    match &field.kind {
        FieldKind::Action { target } => {
            if field.label == "保存当前配置" {
                save_current_config(state);
                return;
            }
            state.current_screen = target.clone();
            state.selection_index = 0;
        }
        FieldKind::Bool => {
            if field.key == "chinese_thinking" {
                state.chinese_thinking = !state.chinese_thinking;
            }
        }
        _ => {
            // 进入文本编辑模式 — 使用原始值(非显示占位符)
            let raw_value = state.get_raw_value(field.key);
            let edit_value = match &field.kind {
                FieldKind::Password if !raw_value.is_empty() => String::new(), // 编辑密码时显示空
                _ => raw_value,
            };
            let mut textarea = TextArea::default();
            textarea.insert_str(&edit_value);
            textarea.move_cursor(tui_textarea::CursorMove::End);
            state.editing_textarea = Some((field.key.to_string(), textarea));
        }
    }
}

fn save_current_config(state: &mut TuiAppState) {
    if let Ok(args) = state.clone().into_args() {
        let config_path = Args::default_config_path(&args.data_dir);
        match args.save_to_file(&config_path) {
            Ok(()) => {
                state.status_message = Some(format!("配置已保存到: {}", config_path.display()));
            }
            Err(e) => {
                state.status_message = Some(format!("保存失败: {}", e));
            }
        }
    } else {
        state.status_message = Some("配置验证失败，请检查参数".into());
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn render(frame: &mut Frame, state: &mut TuiAppState) {
    let area = frame.area();

    // 垂直布局: 标题栏 | 主体 | 底部状态栏
    let vertical = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(3),
    ]);
    let [header_area, body_area, footer_area] = vertical.areas(area);

    render_header(frame, header_area, state);
    render_footer(frame, footer_area, state);

    match state.current_screen {
        Screen::ReviewConfirm => render_review_screen(frame, body_area, state),
        Screen::HealthCheck => render_health_check(frame, body_area, state),
        _ => render_form_screen(frame, body_area, state),
    }

    // 文本编辑浮层
    if state.editing_textarea.is_some() {
        render_edit_popup(frame, state);
    }
}

fn render_header(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " deecodex ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  中文参数配置"),
        Span::raw("    配置: "),
        Span::styled(&state.config_path, Style::default().fg(Color::DarkGray)),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, area);
}

fn render_footer(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let (left_text, right_text) = if let Some(ref msg) = state.status_message {
        (msg.as_str(), "")
    } else if state.editing_textarea.is_some() {
        ("Enter 确认  Esc 取消", "编辑中...")
    } else if state.current_screen == Screen::MainMenu {
        ("↑↓ 导航  Enter 选择  q/Esc 退出", "主菜单")
    } else if state.current_screen == Screen::ReviewConfirm {
        ("Y 确认启动  N/Esc 返回", "配置摘要")
    } else {
        (
            "↑↓ 导航  Enter 编辑  Esc 返回",
            state.current_screen.title(),
        )
    };

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(left_text, Style::default().fg(Color::DarkGray)),
        Span::raw("     "),
        Span::styled(right_text, Style::default().fg(Color::Cyan)),
    ]))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(footer, area);
}

fn render_form_screen(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let fields = fields_for_screen(&state.current_screen);
    let title = state.current_screen.title();

    // 计算最长标签显示宽度，用于统一对齐
    let label_width = fields
        .iter()
        .map(|f| UnicodeWidthStr::width(f.label))
        .max()
        .unwrap_or(20);

    let items: Vec<ListItem> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let value = state.get_field_value(f);
            let display_value = match &f.kind {
                FieldKind::Password if !value.is_empty() => "********".to_string(),
                FieldKind::Action { .. } => "→".to_string(),
                _ => value,
            };

            let is_selected = i == state.selection_index;
            let arrow = if is_selected { "▸" } else { " " };

            // 使用显示宽度填充标签
            let padded_label = pad_display_width(f.label, label_width + 2);

            let line = Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    arrow,
                    Style::default().fg(if is_selected {
                        Color::Cyan
                    } else {
                        Color::White
                    }),
                ),
                Span::raw(" "),
                Span::styled(padded_label, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(
                    display_value,
                    if is_selected {
                        Style::default().fg(Color::Cyan)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
            ]);

            if is_selected {
                ListItem::new(line).style(Style::default().bg(Color::Rgb(50, 50, 60)))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    // 帮助文本
    let help_text = if let Some(field) = fields.get(state.selection_index) {
        format!("💡 {}", field.help)
    } else {
        String::new()
    };

    let inner = Layout::vertical([Constraint::Fill(1), Constraint::Length(2)]);
    let [list_area, help_area] = inner.areas(area);

    let mut list_state = ListState::default();
    list_state.select(Some(state.selection_index));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .highlight_style(Style::default());

    frame.render_stateful_widget(list, list_area, &mut list_state);

    let help = Paragraph::new(help_text).style(Style::default().fg(Color::Gray));
    frame.render_widget(help, help_area);
}

fn render_review_screen(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let mut lines = vec![
        Line::from(Span::styled(
            "══════════ 配置摘要 ══════════",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    // 分组展示
    let groups: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "基础设置",
            vec![
                ("端口", state.port.clone()),
                ("最大请求体", format!("{} MB", state.max_body_mb)),
                ("数据目录", state.data_dir.clone()),
                ("提示词目录", state.prompts_dir.clone()),
                (
                    "中文思考",
                    if state.chinese_thinking {
                        "开启".into()
                    } else {
                        "关闭".into()
                    },
                ),
            ],
        ),
        (
            "上游设置",
            vec![
                ("上游地址", state.upstream.clone()),
                ("API 密钥", mask_value(&state.api_key)),
                ("客户端密钥", mask_value(&state.client_api_key)),
                (
                    "模型映射",
                    if state.model_map.is_empty() {
                        "(无)".into()
                    } else {
                        state.model_map.clone()
                    },
                ),
            ],
        ),
        (
            "视觉模型",
            vec![
                (
                    "视觉上游",
                    if state.vision_upstream.is_empty() {
                        "(未启用)".into()
                    } else {
                        state.vision_upstream.clone()
                    },
                ),
                ("视觉密钥", mask_value(&state.vision_api_key)),
                ("视觉模型", state.vision_model.clone()),
                ("视觉路径", state.vision_endpoint.clone()),
            ],
        ),
        (
            "Token 异常检测",
            vec![
                ("提示词上限", state.token_anomaly_prompt_max.clone()),
                ("飙升比率", state.token_anomaly_spike_ratio.clone()),
                (
                    "燃烧窗口",
                    format!("{} 秒", state.token_anomaly_burn_window),
                ),
                (
                    "燃烧速率",
                    format!("{} tok/分钟", state.token_anomaly_burn_rate),
                ),
            ],
        ),
        (
            "工具安全策略",
            vec![
                (
                    "MCP 白名单",
                    if state.allowed_mcp_servers.is_empty() {
                        "(不限制)".into()
                    } else {
                        state.allowed_mcp_servers.clone()
                    },
                ),
                (
                    "显示器白名单",
                    if state.allowed_computer_displays.is_empty() {
                        "(不限制)".into()
                    } else {
                        state.allowed_computer_displays.clone()
                    },
                ),
                ("computer 执行器", state.computer_executor.clone()),
                (
                    "Playwright 状态目录",
                    if state.playwright_state_dir.is_empty() {
                        "(未设置)".into()
                    } else {
                        state.playwright_state_dir.clone()
                    },
                ),
                (
                    "browser-use Bridge",
                    if state.browser_use_bridge_url.is_empty()
                        && state.browser_use_bridge_command.is_empty()
                    {
                        "(未设置)".into()
                    } else if !state.browser_use_bridge_url.is_empty() {
                        format!("URL: {}", state.browser_use_bridge_url)
                    } else {
                        format!("Cmd: {}", state.browser_use_bridge_command)
                    },
                ),
            ],
        ),
    ];

    for (group_name, items) in &groups {
        lines.push(Line::from(Span::styled(
            format!("▸ {}", group_name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for (label, value) in items {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{:<12}", *label), Style::default().fg(Color::Gray)),
                Span::raw("  "),
                Span::styled(value.clone(), Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "──────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  [Y] 确认并启动服务    [N/Esc] 返回主菜单  ",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("确认并启动")
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .wrap(Wrap { trim: true });

    // 如果内容超出，需要滚动
    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

fn render_health_check(frame: &mut Frame, area: Rect, state: &TuiAppState) {
    let checks = run_health_checks(state);

    let mut lines = vec![
        Line::from(Span::styled(
            "════════ 配置检测报告 ════════",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    let mut ok_count = 0;
    let mut warn_count = 0;
    let mut fail_count = 0;

    for check in &checks {
        let (icon, color) = match check.status {
            CheckStatus::Ok => {
                ok_count += 1;
                ("✓", Color::Green)
            }
            CheckStatus::Warn => {
                warn_count += 1;
                ("⚠", Color::Yellow)
            }
            CheckStatus::Fail => {
                fail_count += 1;
                ("✗", Color::Red)
            }
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", icon),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(&check.label, Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("    {}", check.detail),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "──────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    let summary = if fail_count > 0 {
        format!(
            "检测结果: {} 通过, {} 警告, {} 失败 — 服务可能无法正常启动",
            ok_count, warn_count, fail_count
        )
    } else if warn_count > 0 {
        format!(
            "检测结果: {} 通过, {} 警告 — 建议检查后启动",
            ok_count, warn_count
        )
    } else {
        format!("检测结果: {} 项全部通过 — 配置正常", ok_count)
    };

    let summary_color = if fail_count > 0 {
        Color::Red
    } else if warn_count > 0 {
        Color::Yellow
    } else {
        Color::Green
    };

    lines.push(Line::from(Span::styled(
        summary,
        Style::default()
            .fg(summary_color)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  按 Esc 返回主菜单",
        Style::default().fg(Color::Gray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("快速检测")
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

#[derive(Clone, PartialEq)]
enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

struct CheckResult {
    label: String,
    detail: String,
    status: CheckStatus,
}

fn run_health_checks(state: &TuiAppState) -> Vec<CheckResult> {
    let mut results = Vec::new();

    // 1. 上游地址检查
    let upstream_empty = state.upstream.is_empty();
    results.push(CheckResult {
        label: "上游 API 地址".into(),
        detail: if upstream_empty {
            "未设置上游地址，服务无法启动".into()
        } else {
            state.upstream.to_string()
        },
        status: if upstream_empty {
            CheckStatus::Fail
        } else if state.upstream.starts_with("https://") {
            CheckStatus::Ok
        } else if state.upstream.starts_with("http://") {
            CheckStatus::Warn
        } else {
            CheckStatus::Fail
        },
    });

    // 2. API 密钥检查
    let api_key_set = !state.api_key.is_empty();
    results.push(CheckResult {
        label: "API 密钥".into(),
        detail: if api_key_set {
            "已设置".into()
        } else {
            "未设置，将无法通过上游认证".into()
        },
        status: if api_key_set {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
    });

    // 3. 端口检查
    let port: Result<u16, _> = state.port.parse();
    results.push(CheckResult {
        label: "服务端口".into(),
        detail: match &port {
            Ok(p) if *p < 1024 => format!("端口 {} 需要 root 权限", p),
            Ok(p) => format!("端口 {} 正常", p),
            Err(_) => format!("无效端口号: {}", state.port),
        },
        status: match &port {
            Ok(p) if *p < 1024 => CheckStatus::Warn,
            Ok(_) => CheckStatus::Ok,
            Err(_) => CheckStatus::Fail,
        },
    });

    // 4. 数据目录检查
    let data_dir = std::path::Path::new(&state.data_dir);
    let dir_status = if data_dir.exists() {
        if data_dir.is_dir() {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        }
    } else {
        CheckStatus::Warn // 会自动创建
    };
    results.push(CheckResult {
        label: "数据目录".into(),
        detail: if data_dir.exists() {
            if data_dir.is_dir() {
                format!("{} (已存在)", state.data_dir)
            } else {
                format!("{} 不是目录", state.data_dir)
            }
        } else {
            format!("{} (不存在，将自动创建)", state.data_dir)
        },
        status: dir_status,
    });

    // 5. 提示词目录
    let prompts_dir = std::path::Path::new(&state.prompts_dir);
    results.push(CheckResult {
        label: "提示词目录".into(),
        detail: if prompts_dir.exists() {
            if prompts_dir.is_dir() {
                format!("{} (已存在)", state.prompts_dir)
            } else {
                format!("{} 不是目录", state.prompts_dir)
            }
        } else {
            format!("{} (不存在)", state.prompts_dir)
        },
        status: if prompts_dir.exists() && prompts_dir.is_dir() {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
    });

    // 6. 模型映射 JSON 检查
    let model_map_empty = state.model_map.trim().is_empty();
    results.push(CheckResult {
        label: "模型映射 (JSON)".into(),
        detail: if model_map_empty {
            "未设置模型映射".into()
        } else {
            match serde_json::from_str::<serde_json::Value>(&state.model_map) {
                Ok(_) => "JSON 格式正确".into(),
                Err(e) => format!("JSON 解析错误: {}", e),
            }
        },
        status: if model_map_empty {
            CheckStatus::Warn
        } else if serde_json::from_str::<serde_json::Value>(&state.model_map).is_ok() {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
    });

    // 7. Token 异常检测参数
    let prompt_max: Result<u32, _> = state.token_anomaly_prompt_max.parse();
    let spike: Result<f64, _> = state.token_anomaly_spike_ratio.parse();
    let burn_window: Result<u64, _> = state.token_anomaly_burn_window.parse();
    let burn_rate: Result<u32, _> = state.token_anomaly_burn_rate.parse();

    let anomaly_ok =
        prompt_max.is_ok() && spike.is_ok() && burn_window.is_ok() && burn_rate.is_ok();
    results.push(CheckResult {
        label: "Token 异常检测参数".into(),
        detail: if anomaly_ok {
            let pm: u32 = prompt_max.unwrap();
            if pm == 0 {
                "检测已关闭 (prompt_max=0)".into()
            } else {
                format!(
                    "prompt_max={} spike={}x burn_window={}s burn_rate={}/min",
                    pm,
                    spike.unwrap(),
                    burn_window.unwrap(),
                    burn_rate.unwrap()
                )
            }
        } else {
            "参数值格式错误".into()
        },
        status: if anomaly_ok {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
    });

    // 8. 视觉模型
    let vision_enabled = !state.vision_upstream.is_empty();
    results.push(CheckResult {
        label: "视觉模型".into(),
        detail: if vision_enabled {
            format!(
                "已启用: {} ({}/{})",
                state.vision_model, state.vision_upstream, state.vision_endpoint
            )
        } else {
            "未启用 (vision_upstream 为空)".into()
        },
        status: if vision_enabled {
            if state.vision_api_key.is_empty() {
                CheckStatus::Warn
            } else if state.vision_upstream.starts_with("https://") {
                CheckStatus::Ok
            } else {
                CheckStatus::Warn
            }
        } else {
            CheckStatus::Ok
        },
    });

    // 9. 最大请求体
    let max_body: Result<usize, _> = state.max_body_mb.parse();
    results.push(CheckResult {
        label: "最大请求体".into(),
        detail: match &max_body {
            Ok(mb) => format!("{} MB", mb),
            Err(_) => format!("无效值: {}", state.max_body_mb),
        },
        status: if max_body.is_ok() {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        },
    });

    // 10. 中文思考模式
    results.push(CheckResult {
        label: "中文思考模式".into(),
        detail: if state.chinese_thinking {
            "已开启，系统提示词将注入中文思考指令".into()
        } else {
            "未开启".into()
        },
        status: CheckStatus::Ok,
    });

    // 11. Computer Executor 状态
    let computer_backend = state.computer_executor.trim().to_ascii_lowercase();
    if computer_backend.is_empty() || computer_backend == "disabled" {
        results.push(CheckResult {
            label: "Computer Executor".into(),
            detail: "未启用（disabled）".into(),
            status: CheckStatus::Ok,
        });
    } else if computer_backend == "playwright" {
        let playwright_ok = std::process::Command::new("node")
            .arg("-e")
            .arg("require('playwright')")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        results.push(CheckResult {
            label: "Computer Executor".into(),
            detail: if playwright_ok {
                "已启用: playwright (Playwright 检测通过)".to_string()
            } else {
                "已启用: playwright (⚠ Playwright 不可用 — 请确认 npm install playwright)".into()
            },
            status: if playwright_ok {
                CheckStatus::Ok
            } else {
                CheckStatus::Fail
            },
        });
    } else if computer_backend == "browser-use"
        || computer_backend == "browser_use"
        || computer_backend == "browseruse"
    {
        let has_url = !state.browser_use_bridge_url.trim().is_empty();
        let has_cmd = !state.browser_use_bridge_command.trim().is_empty();
        results.push(CheckResult {
            label: "Computer Executor".into(),
            detail: if has_url || has_cmd {
                format!(
                    "已启用: browser-use ({}配置)",
                    if has_url && has_cmd {
                        "URL + 命令 "
                    } else if has_url {
                        "URL "
                    } else {
                        "命令 "
                    }
                )
            } else {
                "已启用: browser-use (⚠ 未配置 bridge URL 或命令 — 操作将返回失败)".into()
            },
            status: if has_url || has_cmd {
                CheckStatus::Ok
            } else {
                CheckStatus::Warn
            },
        });
    } else {
        results.push(CheckResult {
            label: "Computer Executor".into(),
            detail: format!(
                "未知后端: {} — 支持 playwright / browser-use",
                computer_backend
            ),
            status: CheckStatus::Fail,
        });
    }

    // 12. MCP Executor 配置
    let mcp_config = state.mcp_executor_config.trim();
    if mcp_config.is_empty() {
        results.push(CheckResult {
            label: "MCP Executor".into(),
            detail: "未配置（无 MCP server）".into(),
            status: CheckStatus::Ok,
        });
    } else {
        let parse_result = serde_json::from_str::<serde_json::Value>(mcp_config);
        match parse_result {
            Ok(serde_json::Value::Array(arr)) => {
                let valid_count = arr
                    .iter()
                    .filter(|v| {
                        !v.get("command")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .is_empty()
                    })
                    .count();
                let fail_count = arr.len() - valid_count;
                results.push(CheckResult {
                    label: "MCP Executor".into(),
                    detail: if fail_count > 0 {
                        format!(
                            "{} 个 server ({} 有效，{} 缺少 command 字段)",
                            arr.len(),
                            valid_count,
                            fail_count
                        )
                    } else {
                        format!("{} 个 server 已配置", arr.len())
                    },
                    status: if fail_count > 0 {
                        CheckStatus::Warn
                    } else {
                        CheckStatus::Ok
                    },
                });
            }
            Ok(serde_json::Value::Object(_)) => {
                let has_cmd = serde_json::from_str::<serde_json::Value>(mcp_config)
                    .ok()
                    .and_then(|v| {
                        v.get("command")
                            .and_then(|c| c.as_str())
                            .map(|s| !s.is_empty())
                    })
                    .unwrap_or(false);
                results.push(CheckResult {
                    label: "MCP Executor".into(),
                    detail: if has_cmd {
                        "1 个 server 已配置".into()
                    } else {
                        "配置格式错误：对象需包含 command 字段".into()
                    },
                    status: if has_cmd {
                        CheckStatus::Ok
                    } else {
                        CheckStatus::Fail
                    },
                });
            }
            Ok(_) => {
                results.push(CheckResult {
                    label: "MCP Executor".into(),
                    detail: "配置必须是 JSON 对象或数组".into(),
                    status: CheckStatus::Fail,
                });
            }
            Err(e) => {
                // 可能是文件路径
                let path = std::path::Path::new(mcp_config);
                if mcp_config.ends_with(".json") && path.exists() {
                    results.push(CheckResult {
                        label: "MCP Executor".into(),
                        detail: format!("从文件加载: {}", mcp_config),
                        status: CheckStatus::Ok,
                    });
                } else {
                    results.push(CheckResult {
                        label: "MCP Executor".into(),
                        detail: format!("配置解析失败: {}", e),
                        status: CheckStatus::Fail,
                    });
                }
            }
        }
    }

    // 13. File Search 状态
    let files_dir = std::path::Path::new(&state.data_dir).join("files");
    if files_dir.exists() && files_dir.is_dir() {
        let file_count = std::fs::read_dir(&files_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        e.path()
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext == "json")
                            .unwrap_or(false)
                    })
                    .count()
            })
            .unwrap_or(0);
        results.push(CheckResult {
            label: "File Search".into(),
            detail: if file_count > 0 {
                format!("{} 个文件已索引", file_count)
            } else {
                "数据目录存在但无已上传文件".into()
            },
            status: CheckStatus::Ok,
        });
    } else {
        results.push(CheckResult {
            label: "File Search".into(),
            detail: "未启用（无已上传文件）".into(),
            status: CheckStatus::Ok,
        });
    }

    results
}

fn render_edit_popup(frame: &mut Frame, state: &mut TuiAppState) {
    if let Some((ref field_key, ref mut textarea)) = &mut state.editing_textarea {
        let fields = fields_for_screen(&state.current_screen);
        let field_label = fields
            .iter()
            .find(|f| f.key == field_key)
            .map(|f| f.label)
            .unwrap_or("编辑");

        let popup_area = centered_rect(60, 20, frame.area());

        // 清空背景
        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", field_label))
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Rgb(30, 30, 40)));

        let inner = block.inner(popup_area);

        // 提示文本
        let hint = Paragraph::new("Enter 确认  Esc 取消  Ctrl+U 清空行")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);

        let chunks = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).split(inner);

        textarea.set_block(Block::default().style(Style::default().bg(Color::Rgb(30, 30, 40))));
        frame.render_widget(&*textarea, chunks[0]);

        frame.render_widget(hint, chunks[1]);
        frame.render_widget(block, popup_area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

fn mask_value(value: &str) -> String {
    if value.is_empty() {
        "(未设置)".into()
    } else {
        "********".into()
    }
}

/// 按显示宽度填充字符串，确保中英文混排对齐
fn pad_display_width(s: &str, target_width: usize) -> String {
    let current = UnicodeWidthStr::width(s);
    if current >= target_width {
        s.to_string()
    } else {
        let padding = target_width - current;
        format!("{}{}", s, " ".repeat(padding))
    }
}
