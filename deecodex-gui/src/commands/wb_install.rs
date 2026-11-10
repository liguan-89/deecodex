//! WorkBuddy 资源运行时接入（多客户端）。
//!
//! 目标客户端：
//!   - codex    ：MCP → `~/.codex/config.toml [mcp_servers]`；技能 → `~/.codex/skills`；专家 → `~/.codex/agents/*.toml`
//!   - claude   ：MCP → `~/.claude.json mcpServers`；技能 → `~/.claude/skills`；专家 → `~/.claude/agents/*.md`（subagents）
//!   - opencode ：MCP/agent → `~/.config/opencode/opencode.json`；技能 → `~/.config/opencode/skills`
//!
//! 原则：只写客户端侧用户可控的配置/文件；WorkBuddy 侧 OAuth 凭据加密存储无法迁移，
//! 运行时授权由客户端/对应 MCP server 处理；CLI 型连接器不自动安装依赖，仅提示。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::State;

use deecodex::config;

use crate::ServerManager;

const WB_AGENT_MARKER: &str = "# deecodex-workbuddy";
const WB_AGENT_PREFIX: &str = "wb-";

#[derive(Clone, Copy, PartialEq, Debug)]
enum WbTarget {
    Codex,
    Claude,
    Opencode,
}

impl WbTarget {
    fn slug(self) -> &'static str {
        match self {
            WbTarget::Codex => "codex",
            WbTarget::Claude => "claude",
            WbTarget::Opencode => "opencode",
        }
    }

    fn all() -> Vec<WbTarget> {
        vec![WbTarget::Codex, WbTarget::Claude, WbTarget::Opencode]
    }
}

fn parse_targets(targets: Option<Vec<String>>) -> Vec<WbTarget> {
    match targets {
        None => WbTarget::all(),
        Some(list) => WbTarget::all()
            .into_iter()
            .filter(|t| list.iter().any(|s| s == t.slug()))
            .collect(),
    }
}

fn wb_resources_root() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".deecodex").join("wb-resources"))
}

fn codex_config_path() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

fn codex_skills_dir() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".codex").join("skills"))
}

fn codex_agents_dir() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".codex").join("agents"))
}

fn claude_skills_dir() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".claude").join("skills"))
}

fn claude_agents_dir() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".claude").join("agents"))
}

fn claude_json_path() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".claude.json"))
}

fn opencode_config_path() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".config").join("opencode").join("opencode.json"))
}

fn opencode_skills_dir() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".config").join("opencode").join("skills"))
}

fn read_json(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// 连接器的 MCP server 定义（name -> spec）。
fn connector_mcp_servers(id: &str) -> Vec<(String, Value)> {
    let Some(root) = wb_resources_root() else {
        return Vec::new();
    };
    let dir = root.join("connectors").join(id);
    let Some(value) = read_json(&dir.join("mcp.json")) else {
        return Vec::new();
    };
    let Some(servers) = value.get("mcpServers").and_then(Value::as_object) else {
        return Vec::new();
    };
    servers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn connector_has_cli(id: &str) -> bool {
    let Some(root) = wb_resources_root() else {
        return false;
    };
    root.join("connectors").join(id).join("cli.json").is_file()
}

// ---------- Codex config.toml ----------

fn read_codex_config() -> (Option<PathBuf>, toml_edit::DocumentMut) {
    let Some(path) = codex_config_path() else {
        return (None, toml_edit::DocumentMut::new());
    };
    let content = fs::read_to_string(&path).unwrap_or_default();
    let doc: toml_edit::DocumentMut = content.parse().unwrap_or_default();
    (Some(path), doc)
}

fn save_codex_config(path: &Path, doc: &toml_edit::DocumentMut) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    fs::write(path, doc.to_string()).map_err(|e| format!("写入 config.toml 失败: {e}"))
}

fn installed_mcp_server_names(doc: &toml_edit::DocumentMut) -> Vec<String> {
    doc.get("mcp_servers")
        .and_then(|v| v.as_table())
        .map(|t| t.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default()
}

fn write_mcp_server(doc: &mut toml_edit::DocumentMut, name: &str, spec: &Value) {
    let table = doc
        .entry("mcp_servers")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let servers = table.as_table_mut().expect("mcp_servers 应为表");
    let server = servers
        .entry(name)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let server_table = server.as_table_mut().expect("server 应为表");
    if let Some(url) = spec.get("url").and_then(Value::as_str) {
        server_table["url"] = toml_edit::value(url);
    }
    if let Some(command) = spec.get("command").and_then(Value::as_str) {
        server_table["command"] = toml_edit::value(command);
    }
    if let Some(args) = spec.get("args").and_then(Value::as_array) {
        let mut array = toml_edit::Array::new();
        for arg in args {
            if let Some(s) = arg.as_str() {
                array.push(s);
            }
        }
        server_table["args"] = toml_edit::Item::Value(toml_edit::Value::Array(array));
    }
    if let Some(cwd) = spec.get("cwd").and_then(Value::as_str) {
        server_table["cwd"] = toml_edit::value(cwd);
    }
}

// ---------- Claude / opencode JSON ----------

fn read_claude_json() -> Result<Value, String> {
    let path = claude_json_path().ok_or_else(|| "无法确定 Claude 配置路径".to_string())?;
    if !path.exists() {
        return Ok(json!({ "mcpServers": {} }));
    }
    let content =
        fs::read_to_string(&path).map_err(|e| format!("读取 ~/.claude.json 失败: {e}"))?;
    let mut value: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 ~/.claude.json 失败: {e}"))?;
    if value.get("mcpServers").is_none() {
        value["mcpServers"] = json!({});
    }
    Ok(value)
}

fn write_claude_json(path: &Path, value: &Value) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| format!("序列化 ~/.claude.json 失败: {e}"))?;
    fs::write(path, content).map_err(|e| format!("写入 ~/.claude.json 失败: {e}"))
}

fn read_opencode_json() -> Result<Value, String> {
    let path = opencode_config_path().ok_or_else(|| "无法确定 opencode 配置路径".to_string())?;
    if !path.exists() {
        return Ok(json!({ "$schema": "https://opencode.ai/config.json", "mcp": {}, "agent": {} }));
    }
    let content = fs::read_to_string(&path).map_err(|e| format!("读取 opencode.json 失败: {e}"))?;
    let mut value: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 opencode.json 失败: {e}"))?;
    if value.get("mcp").is_none() {
        value["mcp"] = json!({});
    }
    if value.get("agent").is_none() {
        value["agent"] = json!({});
    }
    Ok(value)
}

fn write_opencode_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 opencode 目录失败: {e}"))?;
    }
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| format!("序列化 opencode.json 失败: {e}"))?;
    fs::write(path, content).map_err(|e| format!("写入 opencode.json 失败: {e}"))
}

fn claude_mcp_spec(spec: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(command) = spec.get("command").and_then(Value::as_str) {
        out.insert("type".into(), json!("stdio"));
        out.insert("command".into(), json!(command));
        if let Some(args) = spec.get("args") {
            out.insert("args".into(), args.clone());
        }
        if let Some(env) = spec.get("env") {
            out.insert("env".into(), env.clone());
        }
    } else {
        out.insert("type".into(), json!("http"));
        if let Some(url) = spec.get("url").and_then(Value::as_str) {
            out.insert("url".into(), json!(url));
        }
    }
    Value::Object(out)
}

fn opencode_mcp_spec(spec: &Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(command) = spec.get("command").and_then(Value::as_str) {
        out.insert("type".into(), json!("stdio"));
        out.insert("command".into(), json!(command));
        if let Some(args) = spec.get("args") {
            out.insert("args".into(), args.clone());
        }
    } else {
        out.insert("type".into(), json!("http"));
        if let Some(url) = spec.get("url").and_then(Value::as_str) {
            out.insert("url".into(), json!(url));
        }
    }
    Value::Object(out)
}

// ---------- 连接器：安装 / 卸载 ----------

/// 安装指定连接器的 MCP server 到目标客户端。
#[tauri::command]
pub async fn install_wb_connectors(
    manager: State<'_, ServerManager>,
    ids: Vec<String>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    super::plugins::require_wb_entitlement(&manager).await?;
    let targets = parse_targets(targets);
    let mut results = Vec::new();
    for target in targets {
        let (installed, conflicts, missing, cli_only) =
            install_connectors_for_target(target, &ids)?;
        results.push(json!({
            "target": target.slug(),
            "installed": installed,
            "conflicts": conflicts,
            "missing": missing,
            "cli_only": cli_only,
        }));
    }
    Ok(json!({
        "results": results,
        "targets": results.iter().filter_map(|r| r.get("target").cloned()).collect::<Vec<_>>(),
    }))
}

#[allow(clippy::type_complexity)]
fn install_connectors_for_target(
    target: WbTarget,
    ids: &[String],
) -> Result<(Vec<Value>, Vec<Value>, Vec<String>, Vec<Value>), String> {
    let mut installed = Vec::new();
    let mut conflicts = Vec::new();
    let mut missing = Vec::new();
    let mut cli_only = Vec::new();

    // 已存在名字（按目标读取）
    let existing: Vec<String> = match target {
        WbTarget::Codex => {
            let (_, doc) = read_codex_config();
            installed_mcp_server_names(&doc)
        }
        WbTarget::Claude => read_claude_json()
            .ok()
            .and_then(|v| v.get("mcpServers").cloned())
            .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
            .unwrap_or_default(),
        WbTarget::Opencode => read_opencode_json()
            .ok()
            .and_then(|v| v.get("mcp").cloned())
            .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
            .unwrap_or_default(),
    };

    for id in ids {
        let servers = connector_mcp_servers(id);
        if servers.is_empty() {
            if connector_has_cli(id) {
                cli_only.push(json!({"id": id, "reason": "仅提供 CLI 定义，需手动安装命令行工具"}));
            } else {
                missing.push(id.clone());
            }
            continue;
        }
        for (server_name, spec) in &servers {
            if existing.iter().any(|n| n == server_name) {
                conflicts.push(json!({
                    "connector": id,
                    "server": server_name,
                    "reason": "目标客户端已存在同名 MCP server",
                }));
                continue;
            }
            match target {
                WbTarget::Codex => {
                    let (path, mut doc) = read_codex_config();
                    write_mcp_server(&mut doc, server_name, spec);
                    if let Some(path) = path {
                        save_codex_config(&path, &doc)?;
                    }
                }
                WbTarget::Claude => {
                    let path =
                        claude_json_path().ok_or_else(|| "无法确定 Claude 配置路径".to_string())?;
                    let mut json = read_claude_json()?;
                    if let Some(servers) = json.get_mut("mcpServers").and_then(Value::as_object_mut)
                    {
                        servers.insert(server_name.clone(), claude_mcp_spec(spec));
                    }
                    write_claude_json(&path, &json)?;
                }
                WbTarget::Opencode => {
                    let path = opencode_config_path()
                        .ok_or_else(|| "无法确定 opencode 配置路径".to_string())?;
                    let mut json = read_opencode_json()?;
                    if let Some(mcp) = json.get_mut("mcp").and_then(Value::as_object_mut) {
                        mcp.insert(server_name.clone(), opencode_mcp_spec(spec));
                    }
                    write_opencode_json(&path, &json)?;
                }
            }
            installed.push(json!({
                "connector": id,
                "server": server_name,
                "type": if spec.get("command").is_some() { "stdio" } else { "http" },
                "url": spec.get("url").and_then(Value::as_str).unwrap_or(""),
                "command": spec.get("command").and_then(Value::as_str).unwrap_or(""),
            }));
        }
    }
    Ok((installed, conflicts, missing, cli_only))
}

/// 移除指定连接器的 MCP server。
#[tauri::command]
pub async fn uninstall_wb_connectors(
    ids: Vec<String>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    let targets = parse_targets(targets);
    let mut results = Vec::new();
    for target in targets {
        let (removed, not_found) = uninstall_connectors_for_target(target, &ids)?;
        results
            .push(json!({ "target": target.slug(), "removed": removed, "not_found": not_found }));
    }
    Ok(json!({ "results": results }))
}

fn uninstall_connectors_for_target(
    target: WbTarget,
    ids: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    let servers_to_remove: Vec<String> = ids
        .iter()
        .flat_map(|id| {
            connector_mcp_servers(id)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        })
        .collect();
    let mut removed = Vec::new();
    let mut not_found = Vec::new();
    match target {
        WbTarget::Codex => {
            let (path, mut doc) = read_codex_config();
            if let Some(table) = doc.get_mut("mcp_servers").and_then(|v| v.as_table_mut()) {
                for name in &servers_to_remove {
                    if table.contains_key(name) {
                        table.remove(name);
                        removed.push(name.clone());
                    } else {
                        not_found.push(name.clone());
                    }
                }
            }
            if let Some(path) = path {
                if !removed.is_empty() {
                    save_codex_config(&path, &doc)?;
                }
            }
        }
        WbTarget::Claude => {
            if let Some(path) = claude_json_path() {
                if path.exists() {
                    let mut json = read_claude_json()?;
                    if let Some(servers) = json.get_mut("mcpServers").and_then(Value::as_object_mut)
                    {
                        for name in &servers_to_remove {
                            if servers.contains_key(name) {
                                servers.remove(name);
                                removed.push(name.clone());
                            } else {
                                not_found.push(name.clone());
                            }
                        }
                    }
                    if !removed.is_empty() {
                        write_claude_json(&path, &json)?;
                    }
                }
            }
        }
        WbTarget::Opencode => {
            if let Some(path) = opencode_config_path() {
                if path.exists() {
                    let mut json = read_opencode_json()?;
                    if let Some(mcp) = json.get_mut("mcp").and_then(Value::as_object_mut) {
                        for name in &servers_to_remove {
                            if mcp.contains_key(name) {
                                mcp.remove(name);
                                removed.push(name.clone());
                            } else {
                                not_found.push(name.clone());
                            }
                        }
                    }
                    if !removed.is_empty() {
                        write_opencode_json(&path, &json)?;
                    }
                }
            }
        }
    }
    Ok((removed, not_found))
}

// ---------- 技能：安装 / 卸载 ----------

fn skills_dest_for(target: WbTarget) -> Option<PathBuf> {
    match target {
        WbTarget::Codex => codex_skills_dir(),
        WbTarget::Claude => claude_skills_dir(),
        WbTarget::Opencode => opencode_skills_dir(),
    }
}

/// 安装技能到目标客户端。
#[tauri::command]
pub async fn install_wb_skills(
    manager: State<'_, ServerManager>,
    ids: Vec<String>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    super::plugins::require_wb_entitlement(&manager).await?;
    let targets = parse_targets(targets);
    let Some(root) = wb_resources_root() else {
        return Err("无法确定 HOME 目录".into());
    };
    let mut results = Vec::new();
    for target in targets {
        let Some(dest_root) = skills_dest_for(target) else {
            return Err("无法确定客户端技能目录".into());
        };
        fs::create_dir_all(&dest_root).map_err(|e| format!("创建技能目录失败: {e}"))?;
        let mut installed = Vec::new();
        let mut missing = Vec::new();
        for id in &ids {
            let src = root.join("skills").join(id);
            if !src.join("SKILL.md").is_file() {
                missing.push(id.clone());
                continue;
            }
            let dest = dest_root.join(id);
            if dest.exists() {
                installed.push(json!({"id": id, "already": true}));
                continue;
            }
            copy_skill_dir(&src, &dest)?;
            installed.push(json!({"id": id, "already": false}));
        }
        results.push(json!({
            "target": target.slug(),
            "installed": installed,
            "missing": missing,
            "dir": dest_root.to_string_lossy().to_string(),
        }));
    }
    Ok(json!({ "results": results }))
}

fn copy_skill_dir(src: &Path, dest: &Path) -> Result<(), String> {
    fn copy_rec(from: &Path, to: &Path) -> Result<(), String> {
        fs::create_dir_all(to).map_err(|e| format!("创建目录失败: {e}"))?;
        for entry in fs::read_dir(from).map_err(|e| format!("读取目录失败: {e}"))? {
            let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
            let name = entry.file_name();
            if name == "source-plugin.json" {
                continue;
            }
            let src_path = entry.path();
            let dest_path = to.join(&name);
            if src_path.is_dir() {
                copy_rec(&src_path, &dest_path)?;
            } else {
                fs::copy(&src_path, &dest_path).map_err(|e| format!("复制失败: {e}"))?;
            }
        }
        Ok(())
    }
    copy_rec(src, dest)
}

/// 卸载技能。
#[tauri::command]
pub async fn uninstall_wb_skills(
    ids: Vec<String>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    let targets = parse_targets(targets);
    let mut results = Vec::new();
    for target in targets {
        let Some(dest_root) = skills_dest_for(target) else {
            return Err("无法确定客户端技能目录".into());
        };
        let mut removed = Vec::new();
        let mut not_found = Vec::new();
        for id in &ids {
            let dest = dest_root.join(id);
            if dest.is_dir() {
                fs::remove_dir_all(&dest).map_err(|e| format!("删除技能失败: {e}"))?;
                removed.push(id.clone());
            } else {
                not_found.push(id.clone());
            }
        }
        results
            .push(json!({ "target": target.slug(), "removed": removed, "not_found": not_found }));
    }
    Ok(json!({ "results": results }))
}

// ---------- 专家：Custom Agents / subagents / opencode agents ----------

fn pick_zh(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get("zh").or_else(|| obj.get("en")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default()
}

fn sanitize_agent_name(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn toml_escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 组装一个专家的展示信息（中文名/职业/描述/默认提示）。
fn agent_info(expert: &Value) -> (String, String, String, String) {
    let display_name = pick_zh(expert, "displayName");
    let profession = pick_zh(expert, "profession");
    let description = pick_zh(expert, "description");
    let default_init = pick_zh(expert, "defaultInitPrompt");
    (display_name, profession, description, default_init)
}

/// 读取专家本地 prompt 正文。
fn agent_prompt_text(root: &Path, plugin: &str, prompt_file: &str) -> String {
    let prompt_name = Path::new(prompt_file)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if prompt_name.is_empty() {
        return String::new();
    }
    fs::read_to_string(
        root.join("agents")
            .join(plugin)
            .join("agents")
            .join(&prompt_name),
    )
    .unwrap_or_default()
}

/// 生成专家（Custom Agent）到目标客户端。
#[tauri::command]
pub async fn install_wb_agents(
    manager: State<'_, ServerManager>,
    ids: Option<Vec<String>>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    super::plugins::require_wb_entitlement(&manager).await?;
    let targets = parse_targets(targets);
    let Some(root) = wb_resources_root() else {
        return Err("无法确定 HOME 目录".into());
    };
    let Some(index) = read_json(&root.join("agents-index.json")) else {
        return Err("未找到 agents-index.json，请先运行提取脚本".into());
    };
    let Some(experts) = index.get("experts").and_then(Value::as_array) else {
        return Err("agents-index.json 缺少 experts".into());
    };
    let filter = ids;
    let mut results = Vec::new();
    for target in targets {
        let mut installed = Vec::new();
        let mut skipped = Vec::new();
        for expert in experts {
            let Some(id) = expert.get("id").and_then(Value::as_str) else {
                continue;
            };
            let plugin = expert
                .get("plugin")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_string();
            if let Some(filter) = &filter {
                if !filter.contains(&plugin) && !filter.iter().any(|f| f == id) {
                    continue;
                }
            }
            let (display_name, profession, description, default_init) = agent_info(expert);
            let prompt_file = expert
                .get("promptFile")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let prompt_text = agent_prompt_text(&root, &plugin, &prompt_file);
            let instructions = if !prompt_text.trim().is_empty() {
                prompt_text
            } else if !default_init.is_empty() {
                format!(
                    "你是{}（{}）。{}\n\n{}",
                    display_name, profession, description, default_init
                )
            } else {
                format!("你是{}（{}）。{}", display_name, profession, description)
            };
            let agent_name = sanitize_agent_name(&plugin);
            if agent_name.is_empty() {
                skipped.push(json!({"id": plugin, "reason": "专家名不合法"}));
                continue;
            }
            let agent_desc = format!(
                "WorkBuddy 专家：{}（{}）。{}",
                display_name, profession, description
            );
            match target {
                WbTarget::Codex => write_codex_agent(&agent_name, &agent_desc, &instructions)?,
                WbTarget::Claude => write_claude_agent(&agent_name, &agent_desc, &instructions)?,
                WbTarget::Opencode => {
                    write_opencode_agent(&agent_name, &agent_desc, &instructions)?
                }
            }
            installed.push(json!({
                "id": plugin,
                "name": agent_name,
                "display_name": display_name,
                "target": target.slug(),
            }));
        }
        results
            .push(json!({ "target": target.slug(), "installed": installed, "skipped": skipped }));
    }
    Ok(json!({ "results": results }))
}

fn write_codex_agent(name: &str, description: &str, instructions: &str) -> Result<(), String> {
    let Some(agents_dir) = codex_agents_dir() else {
        return Err("无法确定 Codex agents 目录".into());
    };
    fs::create_dir_all(&agents_dir).map_err(|e| format!("创建 agents 目录失败: {e}"))?;
    let toml = format!(
        "{marker}\nname = \"{name}\"\ndescription = \"{desc}\"\n\ndeveloper_instructions = \"\"\"\n{instr}\n\"\"\"\n",
        marker = WB_AGENT_MARKER,
        name = name,
        desc = toml_escape(description),
        instr = instructions.trim_end(),
    );
    fs::write(agents_dir.join(format!("{name}.toml")), toml)
        .map_err(|e| format!("写入 Codex agent 失败: {e}"))
}

fn write_claude_agent(name: &str, description: &str, instructions: &str) -> Result<(), String> {
    let Some(agents_dir) = claude_agents_dir() else {
        return Err("无法确定 Claude agents 目录".into());
    };
    fs::create_dir_all(&agents_dir).map_err(|e| format!("创建 Claude agents 目录失败: {e}"))?;
    let md = format!(
        "---\nname: {name}\ndescription: \"{desc}\"\n---\n\n{instr}\n",
        name = name,
        desc = description.replace('"', "\\\""),
        instr = instructions.trim_end(),
    );
    fs::write(agents_dir.join(format!("{WB_AGENT_PREFIX}{name}.md")), md)
        .map_err(|e| format!("写入 Claude subagent 失败: {e}"))
}

fn write_opencode_agent(name: &str, description: &str, instructions: &str) -> Result<(), String> {
    let path = opencode_config_path().ok_or_else(|| "无法确定 opencode 配置路径".to_string())?;
    let mut json = read_opencode_json()?;
    let key = format!("{WB_AGENT_PREFIX}{name}");
    if let Some(agents) = json.get_mut("agent").and_then(Value::as_object_mut) {
        agents.insert(
            key.clone(),
            json!({
                "description": description,
                "mode": "subagent",
                "prompt": instructions,
                "tools": { "bash": false, "edit": false, "read": false },
            }),
        );
    }
    write_opencode_json(&path, &json)
}

/// 卸载 WorkBuddy 生成的 agents（只删带标记/前缀的）。
#[tauri::command]
pub async fn uninstall_wb_agents(
    ids: Option<Vec<String>>,
    targets: Option<Vec<String>>,
) -> Result<Value, String> {
    let targets = parse_targets(targets);
    let filter = ids;
    let mut results = Vec::new();
    for target in targets {
        let (removed, not_found) = match target {
            WbTarget::Codex => uninstall_codex_agents(&filter)?,
            WbTarget::Claude => uninstall_claude_agents(&filter)?,
            WbTarget::Opencode => uninstall_opencode_agents(&filter)?,
        };
        results
            .push(json!({ "target": target.slug(), "removed": removed, "not_found": not_found }));
    }
    Ok(json!({ "results": results }))
}

fn uninstall_codex_agents(
    filter: &Option<Vec<String>>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let Some(agents_dir) = codex_agents_dir() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut removed = Vec::new();
    let mut not_found = Vec::new();
    if !agents_dir.is_dir() {
        return Ok((removed, not_found));
    }
    for entry in fs::read_dir(&agents_dir).map_err(|e| format!("读取 agents 目录失败: {e}"))?
    {
        let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let content = fs::read_to_string(&path).unwrap_or_default();
        if !content.contains(WB_AGENT_MARKER) {
            continue;
        }
        let name = path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Some(filter) = filter {
            if !filter.contains(&name) {
                continue;
            }
        }
        if fs::remove_file(&path).is_ok() {
            removed.push(name);
        } else {
            not_found.push(name);
        }
    }
    Ok((removed, not_found))
}

fn uninstall_claude_agents(
    filter: &Option<Vec<String>>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let Some(agents_dir) = claude_agents_dir() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let mut removed = Vec::new();
    let mut not_found = Vec::new();
    if !agents_dir.is_dir() {
        return Ok((removed, not_found));
    }
    for entry in fs::read_dir(&agents_dir).map_err(|e| format!("读取 agents 目录失败: {e}"))?
    {
        let entry = entry.map_err(|e| format!("读取条目失败: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.starts_with(WB_AGENT_PREFIX) {
            continue;
        }
        let name = file_name
            .trim_start_matches(WB_AGENT_PREFIX)
            .trim_end_matches(".md")
            .to_string();
        if let Some(filter) = filter {
            if !filter.contains(&name) {
                continue;
            }
        }
        if fs::remove_file(&path).is_ok() {
            removed.push(name);
        } else {
            not_found.push(name);
        }
    }
    Ok((removed, not_found))
}

fn uninstall_opencode_agents(
    filter: &Option<Vec<String>>,
) -> Result<(Vec<String>, Vec<String>), String> {
    let Some(path) = opencode_config_path() else {
        return Ok((Vec::new(), Vec::new()));
    };
    if !path.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut json = read_opencode_json()?;
    let mut removed = Vec::new();
    if let Some(agents) = json.get_mut("agent").and_then(Value::as_object_mut) {
        let keys: Vec<String> = agents.keys().cloned().collect();
        for key in keys {
            if !key.starts_with(WB_AGENT_PREFIX) {
                continue;
            }
            let name = key.trim_start_matches(WB_AGENT_PREFIX).to_string();
            if let Some(filter) = filter {
                if !filter.contains(&name) {
                    continue;
                }
            }
            agents.remove(&key);
            removed.push(name);
        }
    }
    if !removed.is_empty() {
        write_opencode_json(&path, &json)?;
    }
    Ok((removed, Vec::new()))
}

// ---------- 已安装状态 ----------

/// 已安装状态（按目标客户端）。
#[tauri::command]
pub async fn list_wb_installed() -> Result<Value, String> {
    let Some(root) = wb_resources_root() else {
        return Ok(json!({ "targets": {} }));
    };
    let mut targets = serde_json::Map::new();
    for target in WbTarget::all() {
        // 连接器
        let mut connectors = Vec::new();
        let existing: Vec<String> = match target {
            WbTarget::Codex => {
                let (_, doc) = read_codex_config();
                installed_mcp_server_names(&doc)
            }
            WbTarget::Claude => read_claude_json()
                .ok()
                .and_then(|v| v.get("mcpServers").cloned())
                .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
                .unwrap_or_default(),
            WbTarget::Opencode => read_opencode_json()
                .ok()
                .and_then(|v| v.get("mcp").cloned())
                .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
                .unwrap_or_default(),
        };
        if let Ok(entries) = fs::read_dir(root.join("connectors")) {
            for entry in entries.flatten() {
                let id = entry.file_name().to_string_lossy().to_string();
                let servers: Vec<String> = connector_mcp_servers(&id)
                    .into_iter()
                    .map(|(n, _)| n)
                    .filter(|n| existing.contains(n))
                    .collect();
                if !servers.is_empty() {
                    connectors.push(json!({"id": id, "servers": servers}));
                }
            }
        }
        // 技能
        let mut skills = Vec::new();
        if let Some(dir) = skills_dest_for(target) {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let id = entry.file_name().to_string_lossy().to_string();
                    if root.join("skills").join(&id).join("SKILL.md").is_file() {
                        skills.push(id);
                    }
                }
            }
        }
        // agents
        let mut agents = Vec::new();
        match target {
            WbTarget::Codex => {
                if let Some(dir) = codex_agents_dir() {
                    if let Ok(entries) = fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                                continue;
                            }
                            if fs::read_to_string(&path)
                                .unwrap_or_default()
                                .contains(WB_AGENT_MARKER)
                            {
                                agents.push(
                                    path.file_stem()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_default(),
                                );
                            }
                        }
                    }
                }
            }
            WbTarget::Claude => {
                if let Some(dir) = claude_agents_dir() {
                    if let Ok(entries) = fs::read_dir(&dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with(WB_AGENT_PREFIX) && name.ends_with(".md") {
                                agents.push(
                                    name.trim_start_matches(WB_AGENT_PREFIX)
                                        .trim_end_matches(".md")
                                        .to_string(),
                                );
                            }
                        }
                    }
                }
            }
            WbTarget::Opencode => {
                if let Ok(json) = read_opencode_json() {
                    if let Some(agents_map) = json.get("agent").and_then(Value::as_object) {
                        for key in agents_map.keys() {
                            if key.starts_with(WB_AGENT_PREFIX) {
                                agents.push(key.trim_start_matches(WB_AGENT_PREFIX).to_string());
                            }
                        }
                    }
                }
            }
        }
        targets.insert(
            target.slug().to_string(),
            json!({ "connectors": connectors, "skills": skills, "agents": agents }),
        );
    }
    Ok(json!({ "targets": Value::Object(targets) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_agent_name_keeps_kebab_ascii() {
        assert_eq!(sanitize_agent_name("content-creator"), "content-creator");
        assert_eq!(
            sanitize_agent_name("ChinaEcommerceOps"),
            "chinaecommerceops"
        );
        assert_eq!(sanitize_agent_name("_x_y-"), "x-y");
        assert_eq!(sanitize_agent_name("抖音"), "");
    }

    #[test]
    fn parse_targets_defaults_to_all() {
        let all = parse_targets(None);
        assert_eq!(all.len(), 3);
        let sub = parse_targets(Some(vec!["claude".into()]));
        assert_eq!(sub, vec![WbTarget::Claude]);
        let empty = parse_targets(Some(vec!["nope".into()]));
        assert!(empty.is_empty());
    }

    #[test]
    fn claude_mcp_spec_http_and_stdio() {
        let http = claude_mcp_spec(&json!({"url": "https://mcp.linear.app/mcp"}));
        assert_eq!(http["type"], "http");
        assert_eq!(http["url"], "https://mcp.linear.app/mcp");
        let stdio = claude_mcp_spec(&json!({"command": "npx", "args": ["-y", "x"]}));
        assert_eq!(stdio["type"], "stdio");
        assert_eq!(stdio["args"][0], "-y");
    }

    #[test]
    fn opencode_mcp_spec_shape() {
        let http = opencode_mcp_spec(&json!({"url": "https://x/mcp"}));
        assert_eq!(http["type"], "http");
        assert_eq!(http["url"], "https://x/mcp");
    }

    #[test]
    fn write_mcp_server_http_toml() {
        let mut doc = toml_edit::DocumentMut::new();
        write_mcp_server(
            &mut doc,
            "github",
            &json!({"url": "https://api.githubcopilot.com/mcp/"}),
        );
        let parsed: toml_edit::DocumentMut = doc.to_string().parse().unwrap();
        assert_eq!(
            parsed["mcp_servers"]["github"]["url"].as_str(),
            Some("https://api.githubcopilot.com/mcp/")
        );
    }
}
