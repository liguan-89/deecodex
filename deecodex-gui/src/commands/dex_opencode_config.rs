//! opencode 引擎配置生成（DEX 助手 v1）
//!
//! 纯函数模块：把 deecodex 账号信息 + DEX 工具注册表 + MCP 端点信息
//! 生成 opencode 1.18.x 的 `opencode.json`。不依赖 Tauri/网络，便于单元测试。
//! 与前端 `dex-assistant-registry.js` 的模式定义保持一致（6 种模式）。
//!
//! 实证结论（opencode 1.18.16）：agent 的 `tools` 映射只对内置工具生效，
//! MCP server 一旦 enabled，其全部工具对所有 agent 可见（true 白名单与显式 false
//! 都无法按 agent 过滤 MCP 工具）。因此 v1 全量暴露 115 个 DEX 工具，
//! 但把 bash/edit/read 等 opencode 内置工具在 agent 层显式禁用（已验证生效），
//! 避免模型直接操作宿主 shell/文件；按模式收敛上下文留作后续扩展。

use serde_json::{json, Value};

use super::dex_registry::DexToolDef;

/// 目标 opencode 版本（pin）
pub const OPENCODE_PIN_VERSION: &str = "1.18.16";

/// MCP server 分组（按能力包聚合，避免 115 个工具全量进单个上下文）
pub const MCP_GROUPS: [&str; 4] = ["dex_diag", "dex_workspace", "dex_ops", "dex_plugins"];

/// 能力包 -> 分组
pub fn capability_to_group(capability: &str) -> &'static str {
    match capability {
        "core.system" | "ai.codex" | "ai.claude" | "ai.hermes" | "ai.mcp" => "dex_diag",
        "core.workspace" => "dex_workspace",
        "deecodex.ops" | "relay.channels" => "dex_ops",
        "plugins.dynamic" => "dex_plugins",
        _ => "dex_diag",
    }
}

/// 模式 -> (id, 系统提示, 启用的 MCP 分组)
pub const MODES: [(&str, &str, &[&str]); 6] = [
    (
        "diag",
        "优先使用只读诊断工具定位问题，先给风险概览，再建议修复路径。",
        &["dex_diag", "dex_ops", "dex_workspace", "dex_plugins"],
    ),
    (
        "fix",
        "先诊断，再执行 L2 修复；L3 必须通过确认流程。每次修复后复核结果。",
        &["dex_diag", "dex_ops", "dex_workspace", "dex_plugins"],
    ),
    (
        "workspace",
        "聚焦当前项目上下文、Git 状态、配置文件、CLI 版本、端口和受限文件读取。",
        &["dex_workspace", "dex_ops"],
    ),
    (
        "plugins",
        "聚焦插件状态、插件账号、插件暴露给 DEX 的工具和插件 JSON-RPC 错误。",
        &["dex_plugins", "dex_ops"],
    ),
    (
        "cost",
        "聚焦请求历史、Token 消耗、缓存命中、模型分布、失败率和成本估算。",
        &["dex_ops"],
    ),
    (
        "logs",
        "聚焦日志异常、最近错误、关键栈信息和可复现线索。",
        &["dex_diag", "dex_ops"],
    ),
];

/// 内置工具（opencode 自带）在 DEX 助手模式下默认禁用，避免模型直接操作宿主 shell/文件。
const DISABLED_BUILTIN_TOOLS: &[&str] = &[
    "bash",
    "edit",
    "write",
    "read",
    "glob",
    "grep",
    "list",
    "webfetch",
    "websearch",
    "task",
    "todowrite",
    "apply_patch",
    "skill",
    "vision",
];

/// MCP 工具在 opencode 上下文中的命名：`<server>_<tool>`（已验证 1.18.16）
pub fn mcp_tool_id(server: &str, tool_name: &str) -> String {
    format!(
        "{}_{}",
        sanitize_part(server, "dex"),
        sanitize_part(tool_name, "tool")
    )
}

/// 找出所有未进入 permission ask 表的 L3 工具（防御漏配）。
/// 返回工具名列表；空列表表示覆盖完整。生成配置后调用，发现漏配应打 warn。
pub fn missing_l3_permissions(
    tool_defs: &[DexToolDef],
    permission: &serde_json::Map<String, Value>,
) -> Vec<String> {
    let mut missing = Vec::new();
    for tool in tool_defs {
        if tool.level >= 3 {
            let group = capability_to_group(&tool.capability);
            let key = mcp_tool_id(group, &tool.name);
            if permission.get(&key) != Some(&Value::String("ask".into())) {
                missing.push(tool.name.clone());
            }
        }
    }
    missing
}

fn sanitize_part(value: &str, fallback: &str) -> String {
    let out: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

/// 归一化上游 baseURL：opencode 的 openai provider 会在 baseURL 后追加 /chat/completions，
/// 因此这里保证以 `/v1` 结尾。
pub fn normalize_base_url(upstream: &str) -> String {
    let mut trimmed = upstream.trim().trim_end_matches('/').to_string();
    if let Some(stripped) = trimmed.strip_suffix("/chat/completions") {
        trimmed = stripped.trim_end_matches('/').to_string();
    }
    if trimmed.ends_with("/v1") {
        trimmed
    } else {
        format!("{trimmed}/v1")
    }
}

/// 挑选默认模型：优先 model_map 的 gpt-5.5 映射，否则用 known_models 第一个。
pub fn pick_default_model(
    model_map: &std::collections::HashMap<String, String>,
    known_models: &[String],
) -> String {
    if let Some(model) = model_map.get("gpt-5.5").filter(|m| !m.trim().is_empty()) {
        return model.clone();
    }
    known_models
        .first()
        .filter(|m| !m.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "gpt-5.5".to_string())
}

/// provider id 必须加 `dex_` 前缀：避免与 opencode 内置 provider（如 minimax/deepseek）
/// 冲突导致请求被内置模型目录覆盖（实证：minimax 会被发到 /v1/messages）。
fn provider_id(provider_slug: &str) -> String {
    format!("dex_{}", sanitize_part(provider_slug, "dex"))
}

/// 生成 opencode.json（Value）。工具按传入的注册表动态归类到分组。
#[allow(clippy::too_many_arguments)]
pub fn generate_opencode_config(
    upstream: &str,
    api_key: &str,
    model_map: &std::collections::HashMap<String, String>,
    known_models: &[String],
    provider_slug: &str,
    mcp_base_url: &str,
    mcp_token: &str,
    tool_defs: &[DexToolDef],
) -> Value {
    let pid = provider_id(provider_slug);
    let model = pick_default_model(model_map, known_models);
    let base_url = normalize_base_url(upstream);

    let mut groups: std::collections::HashMap<&'static str, Vec<&DexToolDef>> =
        std::collections::HashMap::new();
    for tool in tool_defs {
        groups
            .entry(capability_to_group(&tool.capability))
            .or_default()
            .push(tool);
    }

    // mcp servers（每个分组一个 remote server，指向 deecodex 的 MCP 端点）
    let mut mcp = serde_json::Map::new();
    for group in MCP_GROUPS {
        mcp.insert(
            group.to_string(),
            json!({
                "type": "remote",
                "url": format!("{mcp_base_url}/mcp/{group}"),
                "enabled": true,
                "oauth": false,
                "timeout": 10000,
                "headers": { "Authorization": format!("Bearer {mcp_token}") }
            }),
        );
    }

    // permission：L3 工具 -> ask（已验证 opencode 会据此发 permission.asked）
    let mut permission = serde_json::Map::new();
    for tool in tool_defs {
        if tool.level >= 3 {
            let server = capability_to_group(&tool.capability);
            permission.insert(mcp_tool_id(server, &tool.name), Value::String("ask".into()));
        }
    }
    let missing = missing_l3_permissions(tool_defs, &permission);
    if !missing.is_empty() {
        tracing::warn!(
            tools = ?missing,
            "L3 工具未进入 opencode permission ask 表，将无确认执行"
        );
    }

    // agents（模式映射）
    let mut agents = serde_json::Map::new();
    for (mode, prompt, groups_enabled) in MODES {
        let mut tools = serde_json::Map::new();
        for builtin in DISABLED_BUILTIN_TOOLS {
            tools.insert((*builtin).to_string(), Value::Bool(false));
        }
        tools.insert("question".to_string(), Value::Bool(true));
        for group in groups_enabled {
            if let Some(defs) = groups.get(group) {
                for tool in defs.iter() {
                    tools.insert(mcp_tool_id(group, &tool.name), Value::Bool(true));
                }
            }
        }
        agents.insert(
            mode.to_string(),
            json!({
                "description": format!("DEX 助手 {} 模式", mode),
                "mode": "primary",
                "prompt": prompt,
                "tools": Value::Object(tools),
            }),
        );
    }

    json!({
        "$schema": "https://opencode.ai/config.json",
        "provider": {
            pid.clone(): {
                "api": "openai",
                "name": provider_slug,
                "options": {
                    "apiKey": api_key,
                    "baseURL": base_url,
                    "timeout": 300000
                },
                "models": {
                    model.clone(): {
                        "name": model.clone(),
                        "limit": { "context": 64000, "output": 8192 }
                    }
                }
            }
        },
        "model": format!("{pid}/{model}"),
        "small_model": format!("{pid}/{model}"),
        "enabled_providers": [pid],
        "default_agent": "diag",
        "agent": Value::Object(agents),
        "mcp": Value::Object(mcp),
        "permission": Value::Object(permission),
        "autoupdate": false,
        "snapshot": false,
        "share": "disabled"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, capability: &str, level: u8) -> DexToolDef {
        DexToolDef {
            name: name.to_string(),
            tauri_cmd: name.to_string(),
            level,
            confirm: None,
            description: format!("tool {name}"),
            parameters: json!({ "type": "object", "properties": {}, "required": [] }),
            capability: capability.to_string(),
            source: "builtin".to_string(),
            plugin_id: None,
            plugin_method: None,
        }
    }

    #[test]
    fn base_url_normalization() {
        assert_eq!(
            normalize_base_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            normalize_base_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1"
        );
        assert_eq!(
            normalize_base_url("https://opencode.ai/zen/go/v1"),
            "https://opencode.ai/zen/go/v1"
        );
        assert_eq!(
            normalize_base_url("https://x.com/v1/chat/completions"),
            "https://x.com/v1"
        );
    }

    #[test]
    fn capability_group_mapping_covers_all() {
        for cap in [
            "core.system",
            "ai.codex",
            "ai.claude",
            "ai.hermes",
            "ai.mcp",
            "core.workspace",
            "deecodex.ops",
            "relay.channels",
            "plugins.dynamic",
        ] {
            let g = capability_to_group(cap);
            assert!(MCP_GROUPS.contains(&g), "{cap} -> {g}");
        }
        assert_eq!(capability_to_group("unknown.extra"), "dex_diag");
    }

    #[test]
    fn mcp_tool_id_sanitizes() {
        assert_eq!(mcp_tool_id("dex_ops", "get_config"), "dex_ops_get_config");
        assert_eq!(
            mcp_tool_id("dex_diag", "plugin__a__b__123"),
            "dex_diag_plugin__a__b__123"
        );
    }

    #[test]
    fn config_has_provider_mcp_permission_agents_and_isolations() {
        let mut model_map = std::collections::HashMap::new();
        model_map.insert("gpt-5.5".to_string(), "deepseek-chat".to_string());
        let defs = vec![
            tool("get_config", "deecodex.ops", 0),
            tool("execute_shell", "core.workspace", 3),
            tool("run_diagnostics", "deecodex.ops", 0),
            tool("list_plugins", "plugins.dynamic", 0),
        ];
        let cfg = generate_opencode_config(
            "https://api.deepseek.com",
            "sk-test",
            &model_map,
            &["deepseek-chat".to_string()],
            "deepseek",
            "http://127.0.0.1:19090",
            "tok123",
            &defs,
        );
        assert_eq!(cfg["model"], "dex_deepseek/deepseek-chat");
        assert_eq!(
            cfg["provider"]["dex_deepseek"]["options"]["apiKey"],
            "sk-test"
        );
        assert_eq!(
            cfg["provider"]["dex_deepseek"]["options"]["baseURL"],
            "https://api.deepseek.com/v1"
        );
        assert_eq!(cfg["enabled_providers"], json!(["dex_deepseek"]));
        assert_eq!(cfg["autoupdate"], false);
        assert_eq!(cfg["snapshot"], false);
        assert_eq!(
            cfg["mcp"]["dex_ops"]["url"],
            "http://127.0.0.1:19090/mcp/dex_ops"
        );
        assert_eq!(
            cfg["mcp"]["dex_ops"]["headers"]["Authorization"],
            "Bearer tok123"
        );
        assert_eq!(cfg["permission"]["dex_workspace_execute_shell"], "ask");
        assert!(cfg["permission"].get("dex_ops_get_config").is_none());
        assert_eq!(cfg["agent"]["cost"]["tools"]["dex_ops_get_config"], true);
        assert_eq!(
            cfg["agent"]["cost"]["tools"]["dex_ops_run_diagnostics"],
            true
        );
        assert_eq!(
            cfg["agent"]["cost"]["tools"]["dex_plugins_list_plugins"],
            Value::Null
        );
        assert_eq!(cfg["agent"]["cost"]["tools"]["bash"], false);
        assert_eq!(
            cfg["agent"]["diag"]["tools"]["dex_workspace_execute_shell"],
            true
        );
        assert_eq!(cfg["default_agent"], "diag");
    }

    #[test]
    fn all_l3_tools_are_covered_by_permission_ask() {
        // 覆盖真实注册表（src/commands/dex_registry.rs）的全部 L3 工具，
        // 防止新增 L3 工具时漏配 permission 导致无确认执行。
        let l3_tools: [(&str, &str); 14] = [
            ("stop_service", "deecodex.ops"),
            ("stop_codex_cdp", "ai.codex"),
            ("delete_account", "core.system"),
            ("migrate_threads", "ai.codex"),
            ("restore_threads", "ai.codex"),
            ("delete_thread", "ai.codex"),
            ("clear_request_history", "deecodex.ops"),
            ("delete_relay_channel", "relay.channels"),
            ("update_plugin", "plugins.dynamic"),
            ("uninstall_plugin", "plugins.dynamic"),
            ("clear_logs", "deecodex.ops"),
            ("run_upgrade", "deecodex.ops"),
            ("execute_shell", "core.workspace"),
            ("config_restore", "deecodex.ops"),
        ];
        let defs: Vec<DexToolDef> = l3_tools
            .iter()
            .map(|(name, cap)| tool(name, cap, 3))
            .collect();
        let mut model_map = std::collections::HashMap::new();
        model_map.insert("gpt-5.5".to_string(), "deepseek-chat".to_string());
        let cfg = generate_opencode_config(
            "https://api.deepseek.com",
            "sk-test",
            &model_map,
            &["deepseek-chat".to_string()],
            "deepseek",
            "http://127.0.0.1:19090",
            "tok123",
            &defs,
        );
        let permission = cfg["permission"].as_object().unwrap();
        let missing = missing_l3_permissions(&defs, permission);
        assert!(
            missing.is_empty(),
            "以下 L3 工具未覆盖 permission ask: {missing:?}"
        );
        // 每个 L3 工具都应生成对应的 ask 条目
        for (name, _) in l3_tools.iter() {
            let group =
                capability_to_group(&defs.iter().find(|t| &t.name == name).unwrap().capability);
            let key = mcp_tool_id(group, name);
            assert_eq!(permission.get(&key), Some(&json!("ask")), "{key}");
        }
    }
}
