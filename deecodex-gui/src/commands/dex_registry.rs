use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
pub struct DexCapability {
    pub id: String,
    pub label: String,
    pub description: String,
    pub enabled: bool,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexToolDef {
    pub name: String,
    #[serde(rename = "tauriCmd")]
    pub tauri_cmd: String,
    pub level: u8,
    pub confirm: Option<String>,
    pub description: String,
    pub parameters: Value,
    pub capability: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_method: Option<String>,
}

#[derive(Debug, Clone)]
struct BuiltinToolSpec {
    name: &'static str,
    tauri_cmd: &'static str,
    level: u8,
    confirm: Option<&'static str>,
    description: &'static str,
    capability: &'static str,
    parameters: Value,
}

fn empty_params() -> Value {
    json!({ "type": "object", "properties": {}, "required": [] })
}

fn params(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn sanitize_tool_name_part(value: &str, fallback: &str) -> String {
    let sanitized = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

pub(crate) fn plugin_function_name(plugin_id: &str, tool_name: &str) -> String {
    let safe_plugin_id = sanitize_tool_name_part(plugin_id, "plugin");
    let safe_tool_name = sanitize_tool_name_part(tool_name, "tool");
    let hash = format!("{:08x}", stable_tool_name_hash(plugin_id, tool_name) as u32);
    let name = format!("plugin__{safe_plugin_id}__{safe_tool_name}__{hash}");
    if name.len() <= 64 {
        return name;
    }

    let plugin_part = truncate_ascii(&safe_plugin_id, 22);
    let tool_part = truncate_ascii(&safe_tool_name, 22);
    format!("plugin__{}__{}__{}", plugin_part, tool_part, hash)
}

fn stable_tool_name_hash(plugin_id: &str, tool_name: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in plugin_id.bytes().chain([0]).chain(tool_name.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn truncate_ascii(value: &str, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}

fn builtin_tool_specs() -> Vec<BuiltinToolSpec> {
    vec![
        BuiltinToolSpec {
            name: "get_service_status",
            tauri_cmd: "get_service_status",
            level: 0,
            confirm: None,
            description: "获取 deecodex 服务运行状态",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "start_service",
            tauri_cmd: "start_service",
            level: 2,
            confirm: None,
            description: "启动 deecodex 服务",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "stop_service",
            tauri_cmd: "stop_service",
            level: 3,
            confirm: Some("确定要停止 deecodex 服务吗？"),
            description: "停止 deecodex 服务",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "launch_codex_cdp",
            tauri_cmd: "launch_codex_cdp",
            level: 2,
            confirm: None,
            description: "启动 Codex 桌面应用（CDP 模式）",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "stop_codex_cdp",
            tauri_cmd: "stop_codex_cdp",
            level: 3,
            confirm: Some("确定要关闭 Codex 桌面应用吗？"),
            description: "关闭 Codex 桌面应用（CDP 模式）",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_config",
            tauri_cmd: "get_config",
            level: 0,
            confirm: None,
            description: "获取当前 deecodex 配置",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "save_config",
            tauri_cmd: "save_config",
            level: 2,
            confirm: None,
            description: "保存 deecodex 配置",
            capability: "deecodex.ops",
            parameters: params(
                json!({"config_json":{"type":"string","description":"JSON 格式的配置内容"}}),
                &["config_json"],
            ),
        },
        BuiltinToolSpec {
            name: "validate_config",
            tauri_cmd: "validate_config",
            level: 0,
            confirm: None,
            description: "校验 deecodex 配置",
            capability: "deecodex.ops",
            parameters: params(
                json!({"config_json":{"type":"string","description":"待校验的 JSON 配置"}}),
                &["config_json"],
            ),
        },
        BuiltinToolSpec {
            name: "run_diagnostics",
            tauri_cmd: "run_diagnostics",
            level: 0,
            confirm: None,
            description: "运行标准诊断检查",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "run_full_diagnostics",
            tauri_cmd: "run_full_diagnostics",
            level: 1,
            confirm: None,
            description: "运行完整诊断检查（含网络测试）",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "list_accounts",
            tauri_cmd: "list_accounts",
            level: 0,
            confirm: None,
            description: "列出所有账号配置",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_active_account",
            tauri_cmd: "get_active_account",
            level: 0,
            confirm: None,
            description: "获取当前活跃账号信息",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "add_account",
            tauri_cmd: "add_account",
            level: 2,
            confirm: None,
            description: "添加新账号",
            capability: "core.system",
            parameters: params(
                json!({
                    "provider":{"type":"string","description":"供应商标识"},
                    "account_json":{"type":"string","description":"JSON 格式的账号配置"},
                    "client_kind":{"type":"string","description":"账号所属客户端，如 codex/claude_code/openclaw/hermes/generic_client"},
                    "client_surface":{"type":"string","description":"客户端形态，如 cli/desktop"}
                }),
                &["provider", "account_json"],
            ),
        },
        BuiltinToolSpec {
            name: "update_account",
            tauri_cmd: "update_account",
            level: 2,
            confirm: None,
            description: "更新账号配置",
            capability: "core.system",
            parameters: params(
                json!({"account_json":{"type":"string","description":"JSON 格式的更新内容"}}),
                &["account_json"],
            ),
        },
        BuiltinToolSpec {
            name: "delete_account",
            tauri_cmd: "delete_account",
            level: 3,
            confirm: Some("确定要删除该账号吗？此操作不可撤销。"),
            description: "删除账号",
            capability: "core.system",
            parameters: params(
                json!({"id":{"type":"string","description":"账号 ID"}}),
                &["id"],
            ),
        },
        BuiltinToolSpec {
            name: "switch_account",
            tauri_cmd: "switch_account",
            level: 2,
            confirm: None,
            description: "切换活跃账号",
            capability: "core.system",
            parameters: params(
                json!({"id":{"type":"string","description":"目标账号 ID"}}),
                &["id"],
            ),
        },
        BuiltinToolSpec {
            name: "import_codex_config",
            tauri_cmd: "import_codex_config",
            level: 2,
            confirm: None,
            description: "从 Codex 配置文件导入账号",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_provider_presets",
            tauri_cmd: "get_provider_presets",
            level: 0,
            confirm: None,
            description: "获取供应商预设列表",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_client_profiles",
            tauri_cmd: "get_client_profiles",
            level: 0,
            confirm: None,
            description: "获取账号客户端分类列表",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "client_lifecycle_status",
            tauri_cmd: "dex_client_lifecycle_status",
            level: 0,
            confirm: None,
            description: "读取服务概览客户端一键接入状态",
            capability: "core.system",
            parameters: params(
                json!({"kind":{"type":"string","description":"客户端入口，如 codex_cli/codex_desktop/claude_cli/claude_desktop/openclaw/hermes"}}),
                &["kind"],
            ),
        },
        BuiltinToolSpec {
            name: "install_client",
            tauri_cmd: "dex_install_client",
            level: 2,
            confirm: Some("确定要启动该客户端的安装或下载流程吗？"),
            description: "按白名单安装 CLI 客户端或打开桌面客户端下载页",
            capability: "core.system",
            parameters: params(
                json!({"kind":{"type":"string","description":"客户端入口"}}),
                &["kind"],
            ),
        },
        BuiltinToolSpec {
            name: "launch_client",
            tauri_cmd: "dex_launch_client",
            level: 2,
            confirm: Some("确定要启动该客户端吗？"),
            description: "打开桌面客户端或在终端中启动 CLI 客户端",
            capability: "core.system",
            parameters: params(
                json!({"kind":{"type":"string","description":"客户端入口"},"cwd":{"type":"string","description":"CLI 启动目录，可选"}}),
                &["kind"],
            ),
        },
        BuiltinToolSpec {
            name: "quick_configure_client",
            tauri_cmd: "dex_quick_configure_client",
            level: 2,
            confirm: Some("确定要保存账号并写入客户端配置吗？"),
            description: "服务概览轻量配置客户端账号",
            capability: "core.system",
            parameters: params(
                json!({"kind":{"type":"string","description":"客户端入口"},"surface":{"type":"string","description":"cli 或 desktop"},"account_json":{"type":"string","description":"账号 JSON"}}),
                &["kind", "account_json"],
            ),
        },
        BuiltinToolSpec {
            name: "get_client_status",
            tauri_cmd: "get_client_status",
            level: 0,
            confirm: None,
            description: "检查外部 AI 客户端账号配置状态",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "refresh_client_status",
            tauri_cmd: "refresh_client_status",
            level: 1,
            confirm: None,
            description: "刷新并持久化外部客户端账号配置状态",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "list_client_backups",
            tauri_cmd: "list_client_backups",
            level: 0,
            confirm: None,
            description: "列出外部客户端账号最近配置备份",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "restore_client_backup",
            tauri_cmd: "restore_client_backup",
            level: 2,
            confirm: Some("确定要恢复该客户端配置备份吗？当前配置会先再次备份。"),
            description: "恢复 Claude/OpenClaw/Hermes/通用客户端配置备份",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"},"backup_path":{"type":"string","description":"备份文件路径"}}),
                &["account_id", "backup_path"],
            ),
        },
        BuiltinToolSpec {
            name: "open_client_config",
            tauri_cmd: "open_client_config",
            level: 2,
            confirm: Some("确定要用系统默认编辑器打开该配置文件吗？"),
            description: "用系统默认编辑器打开 Codex/客户端配置文件",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "get_account_config_file",
            tauri_cmd: "get_account_config_file",
            level: 0,
            confirm: None,
            description: "读取 Codex/客户端配置文件内容，供内置编辑器展示",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "validate_account_config_file",
            tauri_cmd: "validate_account_config_file",
            level: 0,
            confirm: None,
            description: "校验 Codex/客户端配置文件文本语法",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"},"content":{"type":"string","description":"配置文件文本内容"}}),
                &["account_id", "content"],
            ),
        },
        BuiltinToolSpec {
            name: "save_account_config_file",
            tauri_cmd: "save_account_config_file",
            level: 2,
            confirm: Some("确定要保存配置文件吗？保存前会自动备份当前文件。"),
            description: "在内置编辑器中保存 Codex/客户端配置文件",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"},"content":{"type":"string","description":"配置文件文本内容"}}),
                &["account_id", "content"],
            ),
        },
        BuiltinToolSpec {
            name: "test_client_account",
            tauri_cmd: "test_client_account",
            level: 1,
            confirm: None,
            description: "对客户端账号执行 dry-run 配置验证",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"},"account_json":{"type":"string","description":"未保存账号 JSON"}}),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "apply_client_account",
            tauri_cmd: "apply_client_account",
            level: 2,
            confirm: Some("确定要写入外部客户端配置吗？写入前会自动备份。"),
            description: "写入 Claude/OpenClaw/Hermes/通用客户端配置",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"},"dry_run":{"type":"boolean","description":"仅预检不写入"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "get_account_events",
            tauri_cmd: "get_account_events",
            level: 0,
            confirm: None,
            description: "读取账号配置事件日志",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID，可选"},"limit":{"type":"number","description":"返回条数，默认 20"}}),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "import_client_accounts",
            tauri_cmd: "import_client_accounts",
            level: 1,
            confirm: None,
            description: "扫描外部 AI 客户端配置状态",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "fetch_upstream_models",
            tauri_cmd: "fetch_upstream_models",
            level: 1,
            confirm: None,
            description: "从上游获取可用模型列表",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"},"upstream":{"type":"string","description":"上游 API 地址"},"api_key":{"type":"string","description":"API 密钥"}}),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "fetch_balance",
            tauri_cmd: "fetch_balance",
            level: 1,
            confirm: None,
            description: "查询账号余额/额度",
            capability: "core.system",
            parameters: params(
                json!({"account_id":{"type":"string","description":"账号 ID"}}),
                &["account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "test_upstream_connectivity",
            tauri_cmd: "test_upstream_connectivity",
            level: 1,
            confirm: None,
            description: "测试上游连通性",
            capability: "core.system",
            parameters: params(
                json!({"upstream":{"type":"string","description":"上游 API 地址"},"api_key":{"type":"string","description":"API 密钥"}}),
                &["upstream", "api_key"],
            ),
        },
        BuiltinToolSpec {
            name: "list_sessions",
            tauri_cmd: "list_sessions",
            level: 0,
            confirm: None,
            description: "列出历史会话",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "delete_session",
            tauri_cmd: "delete_session",
            level: 3,
            confirm: Some("确定要删除该会话吗？已备份，可撤销。"),
            description: "删除指定会话",
            capability: "deecodex.ops",
            parameters: params(
                json!({"session_type":{"type":"string","description":"responses 或 conversations"},"session_id":{"type":"string","description":"会话 ID"},"id":{"type":"string","description":"会话 ID 兼容字段"}}),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "undo_delete_session",
            tauri_cmd: "undo_delete_session",
            level: 2,
            confirm: None,
            description: "撤销删除会话",
            capability: "deecodex.ops",
            parameters: params(
                json!({"undo_token":{"type":"string","description":"撤销令牌"},"id":{"type":"string","description":"撤销令牌兼容字段"}}),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "get_threads_status",
            tauri_cmd: "get_threads_status",
            level: 0,
            confirm: None,
            description: "获取线程聚合状态",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "list_threads",
            tauri_cmd: "list_threads",
            level: 0,
            confirm: None,
            description: "列出所有线程",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_thread_sources",
            tauri_cmd: "get_thread_sources",
            level: 0,
            confirm: None,
            description: "获取多客户端线程源状态",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "list_client_threads",
            tauri_cmd: "list_client_threads",
            level: 0,
            confirm: None,
            description: "列出多客户端聚合线程",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_thread_content",
            tauri_cmd: "get_thread_content",
            level: 0,
            confirm: None,
            description: "获取指定线程的完整内容",
            capability: "ai.codex",
            parameters: params(
                json!({"thread_id":{"type":"string","description":"线程 ID"}}),
                &["thread_id"],
            ),
        },
        BuiltinToolSpec {
            name: "get_client_thread_content",
            tauri_cmd: "get_client_thread_content",
            level: 0,
            confirm: None,
            description: "获取多客户端聚合线程详情",
            capability: "deecodex.ops",
            parameters: params(
                json!({"client_kind":{"type":"string","description":"客户端类型"},"native_id":{"type":"string","description":"客户端原生线程 ID"},"thread_key":{"type":"string","description":"跨客户端唯一线程键，可选"}}),
                &["client_kind", "native_id"],
            ),
        },
        BuiltinToolSpec {
            name: "migrate_threads",
            tauri_cmd: "migrate_threads",
            level: 3,
            confirm: Some("确定要迁移所有线程到 deecodex 格式吗？"),
            description: "迁移所有线程到 deecodex 格式",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "restore_threads",
            tauri_cmd: "restore_threads",
            level: 3,
            confirm: Some("确定要还原线程迁移操作吗？"),
            description: "还原线程迁移操作",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "calibrate_threads",
            tauri_cmd: "calibrate_threads",
            level: 2,
            confirm: None,
            description: "校准线程索引",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "delete_thread",
            tauri_cmd: "delete_thread",
            level: 3,
            confirm: Some("确定要永久删除该线程吗？此操作不可撤销。"),
            description: "永久删除指定线程",
            capability: "ai.codex",
            parameters: params(
                json!({"thread_id":{"type":"string","description":"线程 ID"}}),
                &["thread_id"],
            ),
        },
        BuiltinToolSpec {
            name: "list_request_history",
            tauri_cmd: "list_request_history",
            level: 0,
            confirm: None,
            description: "列出请求历史记录",
            capability: "deecodex.ops",
            parameters: params(
                json!({
                    "limit":{"type":"number","description":"返回数量上限"},
                    "client_kind":{"type":"string","description":"客户端类型过滤，如 codex/claude_code/openclaw/hermes/generic_client"},
                    "account_id":{"type":"string","description":"账号 ID 过滤"}
                }),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "clear_request_history",
            tauri_cmd: "clear_request_history",
            level: 3,
            confirm: Some("确定要清空所有请求历史记录吗？"),
            description: "清空所有请求历史记录",
            capability: "deecodex.ops",
            parameters: params(
                json!({
                    "client_kind":{"type":"string","description":"客户端类型过滤，省略则清空全部"},
                    "account_id":{"type":"string","description":"账号 ID 过滤，省略则清空匹配客户端的全部"}
                }),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "get_monthly_stats",
            tauri_cmd: "get_monthly_stats",
            level: 0,
            confirm: None,
            description: "获取月度统计信息",
            capability: "deecodex.ops",
            parameters: params(
                json!({
                    "limit":{"type":"number","description":"返回月份数量"},
                    "client_kind":{"type":"string","description":"客户端类型过滤"},
                    "account_id":{"type":"string","description":"账号 ID 过滤"}
                }),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "get_request_stats_since",
            tauri_cmd: "get_request_stats_since",
            level: 0,
            confirm: None,
            description: "按指定 Unix 秒时间点之后聚合请求统计",
            capability: "deecodex.ops",
            parameters: params(
                json!({
                    "since":{"type":"number","description":"起始 Unix 秒时间戳，省略则统计当前明细表全部记录"},
                    "client_kind":{"type":"string","description":"客户端类型过滤"},
                    "account_id":{"type":"string","description":"账号 ID 过滤"}
                }),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "list_plugins",
            tauri_cmd: "list_plugins",
            level: 0,
            confirm: None,
            description: "列出所有已安装插件",
            capability: "plugins.dynamic",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "list_plugin_events",
            tauri_cmd: "list_plugin_events",
            level: 0,
            confirm: None,
            description: "列出最近插件日志和状态事件",
            capability: "plugins.dynamic",
            parameters: params(
                json!({
                    "plugin_id":{"type":"string","description":"插件 ID，可省略"},
                    "limit":{"type":"number","description":"返回数量，默认 80"}
                }),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "install_plugin",
            tauri_cmd: "install_plugin",
            level: 2,
            confirm: None,
            description: "安装插件",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"path":{"type":"string","description":"插件文件或目录路径"}}),
                &["path"],
            ),
        },
        BuiltinToolSpec {
            name: "update_plugin",
            tauri_cmd: "update_plugin",
            level: 3,
            confirm: Some("确定要更新该插件包吗？配置会保留，但插件文件会被替换。"),
            description: "更新已安装插件，保留配置、启用状态和连接资产",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"path":{"type":"string","description":"插件文件或目录路径"}}),
                &["path"],
            ),
        },
        BuiltinToolSpec {
            name: "uninstall_plugin",
            tauri_cmd: "uninstall_plugin",
            level: 3,
            confirm: Some("确定要卸载该插件吗？"),
            description: "卸载指定插件",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"}}),
                &["plugin_id"],
            ),
        },
        BuiltinToolSpec {
            name: "start_plugin",
            tauri_cmd: "start_plugin",
            level: 2,
            confirm: None,
            description: "启动指定插件",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"}}),
                &["plugin_id"],
            ),
        },
        BuiltinToolSpec {
            name: "stop_plugin",
            tauri_cmd: "stop_plugin",
            level: 2,
            confirm: None,
            description: "停止指定插件",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"}}),
                &["plugin_id"],
            ),
        },
        BuiltinToolSpec {
            name: "set_plugin_enabled",
            tauri_cmd: "set_plugin_enabled",
            level: 2,
            confirm: None,
            description: "启用或停用指定插件。停用会停止运行中的插件，并阻止动态工具自动拉起。",
            capability: "plugins.dynamic",
            parameters: params(
                json!({
                    "plugin_id":{"type":"string","description":"插件 ID"},
                    "enabled":{"type":"boolean","description":"true 启用，false 停用"}
                }),
                &["plugin_id", "enabled"],
            ),
        },
        BuiltinToolSpec {
            name: "update_plugin_config",
            tauri_cmd: "update_plugin_config",
            level: 2,
            confirm: None,
            description: "更新插件配置",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"},"config_json":{"type":"string","description":"JSON 格式的插件配置"}}),
                &["plugin_id", "config_json"],
            ),
        },
        BuiltinToolSpec {
            name: "execute_plugin_feature",
            tauri_cmd: "execute_plugin_feature",
            level: 2,
            confirm: None,
            description: "执行插件 features.methods 中声明的通用动作",
            capability: "plugins.dynamic",
            parameters: params(
                json!({
                    "plugin_id":{"type":"string","description":"插件 ID"},
                    "feature_id":{"type":"string","description":"插件能力 ID"},
                    "action":{"type":"string","description":"能力动作名"},
                    "params_json":{"type":"string","description":"传给插件方法的 JSON 参数，可省略"},
                    "confirmed":{"type":"boolean","description":"高风险插件执行确认"}
                }),
                &["plugin_id", "feature_id", "action"],
            ),
        },
        BuiltinToolSpec {
            name: "get_plugin_qrcode",
            tauri_cmd: "get_plugin_qrcode",
            level: 1,
            confirm: None,
            description: "获取插件扫码登录二维码",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"},"account_id":{"type":"string","description":"插件账号 ID"}}),
                &["plugin_id", "account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "plugin_login_cancel",
            tauri_cmd: "plugin_login_cancel",
            level: 2,
            confirm: None,
            description: "取消插件扫码登录",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"},"account_id":{"type":"string","description":"插件账号 ID"}}),
                &["plugin_id", "account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "query_plugin_status",
            tauri_cmd: "query_plugin_status",
            level: 0,
            confirm: None,
            description: "查询插件运行状态",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"},"account_id":{"type":"string","description":"插件账号 ID"}}),
                &["plugin_id", "account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "start_plugin_account",
            tauri_cmd: "start_plugin_account",
            level: 2,
            confirm: None,
            description: "启动插件账号服务",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"},"account_id":{"type":"string","description":"插件账号 ID"}}),
                &["plugin_id", "account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "stop_plugin_account",
            tauri_cmd: "stop_plugin_account",
            level: 2,
            confirm: None,
            description: "停止插件账号服务",
            capability: "plugins.dynamic",
            parameters: params(
                json!({"plugin_id":{"type":"string","description":"插件 ID"},"account_id":{"type":"string","description":"插件账号 ID"}}),
                &["plugin_id", "account_id"],
            ),
        },
        BuiltinToolSpec {
            name: "get_logs",
            tauri_cmd: "get_logs",
            level: 0,
            confirm: None,
            description: "获取最近日志",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "clear_logs",
            tauri_cmd: "clear_logs",
            level: 3,
            confirm: Some("确定要清空所有日志吗？"),
            description: "清空所有日志",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "debug_gui_state",
            tauri_cmd: "debug_gui_state",
            level: 0,
            confirm: None,
            description: "获取 GUI 调试状态信息",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "browse_file",
            tauri_cmd: "browse_file",
            level: 0,
            confirm: None,
            description: "打开文件选择器浏览文件",
            capability: "core.workspace",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "check_upgrade",
            tauri_cmd: "check_upgrade",
            level: 0,
            confirm: None,
            description: "检查是否有可用更新",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "run_upgrade",
            tauri_cmd: "run_upgrade",
            level: 3,
            confirm: Some("确定要执行一键升级吗？"),
            description: "执行一键升级",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "read_file",
            tauri_cmd: "dex_read_file",
            level: 0,
            confirm: None,
            description: "读取文件内容（支持行数限制）",
            capability: "core.workspace",
            parameters: params(
                json!({"path":{"type":"string","description":"文件路径"},"max_lines":{"type":"number","description":"最大读取行数"}}),
                &["path"],
            ),
        },
        BuiltinToolSpec {
            name: "list_directory",
            tauri_cmd: "dex_list_directory",
            level: 0,
            confirm: None,
            description: "列出目录内容",
            capability: "core.workspace",
            parameters: params(
                json!({"path":{"type":"string","description":"目录路径"}}),
                &["path"],
            ),
        },
        BuiltinToolSpec {
            name: "detect_processes",
            tauri_cmd: "dex_detect_processes",
            level: 0,
            confirm: None,
            description: "检测系统进程信息",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "detect_ports",
            tauri_cmd: "dex_detect_ports",
            level: 0,
            confirm: None,
            description: "检测网络端口使用情况",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "get_env_info",
            tauri_cmd: "dex_get_env_info",
            level: 0,
            confirm: None,
            description: "获取系统环境信息",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "execute_shell",
            tauri_cmd: "dex_execute_shell",
            level: 3,
            confirm: Some("确定要执行这条 Shell 命令吗？"),
            description: "执行 Shell 命令",
            capability: "core.workspace",
            parameters: params(
                json!({"command":{"type":"string","description":"要执行的命令"},"timeout_secs":{"type":"number","description":"超时秒数"}}),
                &["command"],
            ),
        },
        BuiltinToolSpec {
            name: "search_logs",
            tauri_cmd: "dex_search_logs",
            level: 0,
            confirm: None,
            description: "搜索日志内容",
            capability: "deecodex.ops",
            parameters: params(
                json!({"query":{"type":"string","description":"搜索关键词"},"context_lines":{"type":"number","description":"上下文行数"}}),
                &["query"],
            ),
        },
        BuiltinToolSpec {
            name: "get_codex_config_raw",
            tauri_cmd: "dex_get_codex_config_raw",
            level: 0,
            confirm: None,
            description: "获取 Codex 原始配置文件内容",
            capability: "ai.codex",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "health_summary",
            tauri_cmd: "dex_health_summary",
            level: 0,
            confirm: None,
            description: "一键健康概览：服务状态+账号状态+Codex安装+最近错误数",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "dex_self_check",
            tauri_cmd: "dex_self_check",
            level: 0,
            confirm: None,
            description: "检查 DEX助手自身状态、能力包、工具注册表、插件工具和最近请求错误",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "analyze_requests",
            tauri_cmd: "dex_analyze_requests",
            level: 0,
            confirm: None,
            description: "分析最近请求：成功率、延迟P50/P99、Token消耗、模型分布",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "config_backup",
            tauri_cmd: "dex_config_backup",
            level: 1,
            confirm: None,
            description: "备份/列出配置文件；恢复请使用 config_restore",
            capability: "deecodex.ops",
            parameters: params(
                json!({"action":{"type":"string","description":"backup|list"},"name":{"type":"string","description":"备份名称"}}),
                &["action"],
            ),
        },
        BuiltinToolSpec {
            name: "config_restore",
            tauri_cmd: "dex_config_backup",
            level: 3,
            confirm: Some("确定要恢复配置备份吗？当前 config.json/accounts.json 会被覆盖。"),
            description: "恢复 deecodex 配置备份",
            capability: "deecodex.ops",
            parameters: params(
                json!({"name":{"type":"string","description":"备份名称"}}),
                &["name"],
            ),
        },
        BuiltinToolSpec {
            name: "config_diff",
            tauri_cmd: "dex_config_diff",
            level: 0,
            confirm: None,
            description: "对比当前配置与历史版本的差异",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "token_cost",
            tauri_cmd: "dex_token_cost",
            level: 0,
            confirm: None,
            description: "分析 Token 消耗与成本",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "speed_test",
            tauri_cmd: "dex_speed_test",
            level: 0,
            confirm: None,
            description: "测试 API 响应速度与延迟",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "thread_cleanup",
            tauri_cmd: "dex_thread_cleanup",
            level: 0,
            confirm: None,
            description: "清理无用线程数据",
            capability: "ai.codex",
            parameters: params(
                json!({"dry_run":{"type":"boolean","description":"是否为演练模式（不实际删除）"}}),
                &[],
            ),
        },
        BuiltinToolSpec {
            name: "auto_tune",
            tauri_cmd: "dex_auto_tune",
            level: 0,
            confirm: None,
            description: "自动调优 deecodex 配置参数",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "claude_mcp_check",
            tauri_cmd: "dex_claude_mcp_check",
            level: 0,
            confirm: None,
            description: "检查 Claude Code MCP 集成状态",
            capability: "ai.mcp",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "claude_env_overview",
            tauri_cmd: "dex_claude_env_overview",
            level: 0,
            confirm: None,
            description: "检查 Claude Code 安装、版本与 MCP 配置概览",
            capability: "ai.claude",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_env_overview",
            tauri_cmd: "dex_openclaw_env_overview",
            level: 0,
            confirm: None,
            description: "检查 OpenClaw CLI、配置目录与状态概览",
            capability: "ai.openclaw",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_health_check",
            tauri_cmd: "dex_openclaw_health_check",
            level: 0,
            confirm: None,
            description: "运行 OpenClaw 只读健康检查",
            capability: "ai.openclaw",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_mcp_check",
            tauri_cmd: "dex_openclaw_mcp_check",
            level: 0,
            confirm: None,
            description: "检查 OpenClaw MCP 配置与工具暴露状态",
            capability: "ai.mcp",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_gateway_overview",
            tauri_cmd: "dex_openclaw_gateway_overview",
            level: 0,
            confirm: None,
            description: "检查 OpenClaw Gateway 只读状态",
            capability: "ai.openclaw",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_agents_overview",
            tauri_cmd: "dex_openclaw_agents_overview",
            level: 0,
            confirm: None,
            description: "列出 OpenClaw agents 概览",
            capability: "ai.openclaw",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_models_overview",
            tauri_cmd: "dex_openclaw_models_overview",
            level: 0,
            confirm: None,
            description: "列出 OpenClaw 模型配置与状态概览",
            capability: "ai.openclaw",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "openclaw_approvals_overview",
            tauri_cmd: "dex_openclaw_approvals_overview",
            level: 0,
            confirm: None,
            description: "读取 OpenClaw approvals 策略概览",
            capability: "ai.openclaw",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "hermes_env_overview",
            tauri_cmd: "dex_hermes_env_overview",
            level: 0,
            confirm: None,
            description: "检查 Hermes CLI、配置目录与环境概览",
            capability: "ai.hermes",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "hermes_doctor_check",
            tauri_cmd: "dex_hermes_doctor_check",
            level: 0,
            confirm: None,
            description: "运行 Hermes doctor 只读检查",
            capability: "ai.hermes",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "hermes_skills_overview",
            tauri_cmd: "dex_hermes_skills_overview",
            level: 0,
            confirm: None,
            description: "列出 Hermes skills 概览",
            capability: "ai.hermes",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "hermes_config_overview",
            tauri_cmd: "dex_hermes_config_overview",
            level: 0,
            confirm: None,
            description: "读取 Hermes 配置摘要",
            capability: "ai.hermes",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "hermes_gateway_overview",
            tauri_cmd: "dex_hermes_gateway_overview",
            level: 0,
            confirm: None,
            description: "检查 Hermes Gateway 只读状态",
            capability: "ai.hermes",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "ai_toolchain_overview",
            tauri_cmd: "dex_ai_toolchain_overview",
            level: 0,
            confirm: None,
            description: "汇总 Codex、Claude、OpenClaw、Hermes、MCP 与插件工具链状态",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "network_topology",
            tauri_cmd: "dex_network_topology",
            level: 0,
            confirm: None,
            description: "分析网络拓扑与连通性",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "ssl_check",
            tauri_cmd: "dex_ssl_check",
            level: 0,
            confirm: None,
            description: "检查 SSL/TLS 证书状态",
            capability: "core.system",
            parameters: empty_params(),
        },
        BuiltinToolSpec {
            name: "export_report",
            tauri_cmd: "dex_export_report",
            level: 0,
            confirm: None,
            description: "导出系统诊断与健康报告",
            capability: "deecodex.ops",
            parameters: empty_params(),
        },
    ]
}

pub(crate) fn builtin_tool_defs() -> Vec<DexToolDef> {
    builtin_tool_specs()
        .into_iter()
        .map(|t| DexToolDef {
            name: t.name.to_string(),
            tauri_cmd: t.tauri_cmd.to_string(),
            level: t.level,
            confirm: t.confirm.map(str::to_string),
            description: t.description.to_string(),
            parameters: t.parameters,
            capability: t.capability.to_string(),
            source: "builtin".to_string(),
            plugin_id: None,
            plugin_method: None,
        })
        .collect()
}

pub(crate) fn capability_meta(id: &str) -> (&'static str, &'static str) {
    match id {
        "core.system" => ("系统环境", "进程、端口、CLI 版本、模型账号等只读系统信息"),
        "core.workspace" => ("工作区", "当前项目上下文、受限文件读取和受确认 Shell 执行"),
        "ai.codex" => ("Codex", "Codex 配置、线程聚合、CDP 启动和线程维护"),
        "ai.claude" => ("Claude", "Claude Code 安装、配置、MCP 和常见异常检查"),
        "ai.openclaw" => (
            "OpenClaw",
            "OpenClaw CLI、Gateway、Agents、Models、Approvals 和健康状态检查",
        ),
        "ai.hermes" => (
            "Hermes",
            "Hermes CLI、doctor、skills、config 和 gateway 检查",
        ),
        "ai.mcp" => (
            "MCP",
            "跨 Codex、Claude、OpenClaw、Hermes 的 MCP 配置和连通性诊断",
        ),
        "deecodex.ops" => ("deecodex 运维", "服务、配置、诊断、日志、请求历史和报告"),
        "plugins.dynamic" => ("插件", "插件管理和插件声明的动态 DEX 工具"),
        _ => ("扩展能力", "第三方插件或未来扩展能力"),
    }
}

pub(crate) fn default_capability_enabled(id: &str) -> bool {
    let _ = id;
    true
}

pub(crate) fn capability_state_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("dex_capabilities.json")
}

pub(crate) fn load_capability_states(data_dir: &std::path::Path) -> HashMap<String, bool> {
    let path = capability_state_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "DEX 能力包状态读取失败，使用默认值");
            HashMap::new()
        }),
        Err(_) => HashMap::new(),
    }
}

pub(crate) fn save_capability_states(
    data_dir: &std::path::Path,
    states: &HashMap<String, bool>,
) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let path = capability_state_path(data_dir);
    let content =
        serde_json::to_string_pretty(states).map_err(|e| format!("序列化能力包状态失败: {e}"))?;
    std::fs::write(&path, content).map_err(|e| format!("保存能力包状态失败: {e}"))
}

pub(crate) fn is_capability_enabled(states: &HashMap<String, bool>, id: &str) -> bool {
    states
        .get(id)
        .copied()
        .unwrap_or_else(|| default_capability_enabled(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_state_uses_defaults_and_explicit_overrides() {
        let mut states = HashMap::new();
        assert!(is_capability_enabled(&states, "core.workspace"));
        assert!(is_capability_enabled(&states, "ai.openclaw"));
        assert!(is_capability_enabled(&states, "ai.hermes"));

        states.insert("core.workspace".to_string(), false);
        assert!(!is_capability_enabled(&states, "core.workspace"));

        states.insert("plugins.dynamic".to_string(), true);
        assert!(is_capability_enabled(&states, "plugins.dynamic"));
    }

    #[test]
    fn ai_toolchain_capabilities_have_metadata() {
        assert_eq!(capability_meta("ai.openclaw").0, "OpenClaw");
        assert_eq!(capability_meta("ai.hermes").0, "Hermes");
        assert!(capability_meta("ai.mcp").1.contains("OpenClaw"));
    }

    #[test]
    fn plugin_function_names_are_safe_for_llm_tools() {
        let name = plugin_function_name("deecodex-weixin.dev", "check-status");
        assert_eq!(name, "plugin__deecodex_weixin_dev__check_status__fb386cb9");
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        assert!(name.len() <= 64);
    }

    #[test]
    fn long_plugin_function_names_fall_back_to_stable_short_name() {
        let name = plugin_function_name(
            "very-long-plugin-id-with-many-segments-and-extra-characters",
            "very-long-tool-name-with-many-segments-and-extra-characters",
        );
        assert_eq!(
            name,
            "plugin__very_long_plugin_id_wi__very_long_tool_name_wi__93ca6d12"
        );
        assert!(name.len() <= 64);
    }

    #[test]
    fn plugin_function_names_avoid_sanitization_collisions() {
        let a = plugin_function_name("plugin-a", "tool");
        let b = plugin_function_name("plugin_a", "tool");
        assert_ne!(a, b);
    }
}
