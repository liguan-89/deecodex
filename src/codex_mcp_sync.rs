//! Codex MCP server 配置层同步模块。
//!
//! 借鉴 cc-switch `mcp/codex.rs`：让 Rust core / GUI 能够读写
//! `~/.codex/config.toml` 中的 `[mcp_servers.*]` 段。
//!
//! 设计为纯函数 + `toml_edit::DocumentMut` 操作：
//! - 不维护自有 `McpConfig` 类型，统一用 `serde_json::Value` 作为中间表示
//!   （与 `body_filter`、`thinking_rectifier` 等模块保持一致）
//! - stdio / http / sse 三种 transport 均支持；缺省 `type` 自动补 `stdio`
//! - 保留 `[mcp_servers]` 之外的 toml 段落不动（用 toml_edit 增量改写）
//!
//! ## 与 cc-switch 的差异
//! - 不引入 `McpConfig` 强类型（按 Karpathy "不为单一用途创建抽象"）
//! - 不实现"目录缺失时跳过写入"语义（由调用方决定是否调用）
//! - 不实现 JSON Value → TOML Item 的递归通用转换，只针对三种 transport
//!   的固定字段集做映射（避免实现复杂度爆炸）

use serde_json::{json, Value};
use std::collections::BTreeMap;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

/// MCP server 操作累计计数。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct McpSyncCount {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub invalid: usize,
}

/// 解析 Codex config.toml 文本。空字符串返回空 DocumentMut。
pub fn parse_codex_config(text: &str) -> Result<DocumentMut, String> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>()
        .map_err(|e| format!("解析 config.toml 失败: {e}"))
}

/// 序列化 Codex config.toml DocumentMut 为文本。
pub fn serialize_codex_config(doc: &DocumentMut) -> String {
    doc.to_string()
}

/// 读取 `~/.codex/config.toml`，自动处理 UTF-8 / UTF-16 LE / UTF-16 BE 编码。
///
/// 与 `codex_config::read_config_file` 等价的简化版：MCP 同步只需要字符串内容，
/// 不需要其它业务字段的漂移检测，所以单独抽一份避免与 codex_config.rs 循环依赖。
pub fn read_codex_config_file(path: &std::path::Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    if bytes.is_empty() {
        return Ok(String::new());
    }
    // UTF-16 LE BOM
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&u16s).map_err(|e| format!("UTF-16 LE 解码失败: {e}"));
    }
    // UTF-16 BE BOM
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        let u16s: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&u16s).map_err(|e| format!("UTF-16 BE 解码失败: {e}"));
    }
    String::from_utf8(bytes).map_err(|e| format!("UTF-8 解码失败: {e}"))
}

/// 原子写入 config.toml（先写临时文件再 rename）。
pub fn write_codex_config_file(path: &std::path::Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("config.toml 路径 {} 没有父目录", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("创建父目录 {} 失败: {e}", parent.display()))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, content)
        .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} → {} 失败: {e}", tmp.display(), path.display()))
}

/// 列出 [mcp_servers.*] 段下所有 server。
///
/// 返回 BTreeMap 保证输出顺序稳定（按 server id 字典序）。
/// server spec 归一为 JSON Value，缺省 `type` 自动补 `"stdio"`。
pub fn list_mcp_servers(doc: &DocumentMut) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let Some(table) = doc.get("mcp_servers").and_then(Item::as_table) else {
        return out;
    };
    for (id, item) in table.iter() {
        let Some(server_table) = item.as_table() else {
            continue;
        };
        match toml_table_to_spec(server_table) {
            Ok(spec) => {
                out.insert(id.to_string(), spec);
            }
            Err(_) => {
                // 跳过无效条目，但保证 key 存在（用 Null 占位）
                out.insert(id.to_string(), Value::Null);
            }
        }
    }
    out
}

/// 校验并归一化 server spec（JSON Value）。
///
/// 规则：
/// - `type` 缺省 → `"stdio"`
/// - `type` 不在 {stdio, http, sse} → 错误
/// - stdio 缺 `command` → 错误
/// - http/sse 缺 `url` → 错误
/// - 保留额外字段（如 enabled、startup_timeout_sec、env 等）
pub fn parse_server_spec(spec: &Value) -> Result<Value, String> {
    let Some(obj) = spec.as_object() else {
        return Err("server spec 必须是 JSON object".to_string());
    };
    let typ = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("stdio")
        .to_string();
    match typ.as_str() {
        "stdio" => {
            if obj.get("command").and_then(Value::as_str).is_none() {
                return Err("stdio server 缺少 command 字段".to_string());
            }
        }
        "http" | "sse" => {
            if obj.get("url").and_then(Value::as_str).is_none() {
                return Err(format!("{typ} server 缺少 url 字段"));
            }
        }
        other => {
            return Err(format!("未知 server type '{other}'，仅支持 stdio/http/sse"));
        }
    }
    let mut out = obj.clone();
    out.insert("type".to_string(), Value::String(typ));
    Ok(Value::Object(out))
}

/// 添加或更新单个 MCP server。返回 `(added, updated)` 二选一。
///
/// 已存在则覆盖（不算 add 也不算 invalid）；spec 解析失败返回错误且不改 doc。
pub fn add_or_update_mcp_server(
    doc: &mut DocumentMut,
    id: &str,
    spec: &Value,
) -> Result<bool, String> {
    if id.is_empty() {
        return Err("server id 不能为空".to_string());
    }
    let normalized = parse_server_spec(spec)?;
    let toml = spec_to_toml_table(&normalized)?;
    ensure_mcp_servers_table(doc);
    let mcp = doc["mcp_servers"].as_table_mut().expect("已确保存在");
    let existed = mcp.contains_key(id);
    mcp.insert(id, Item::Table(toml));
    Ok(existed)
}

/// 移除单个 MCP server。返回是否真的删除了一项。
pub fn remove_mcp_server(doc: &mut DocumentMut, id: &str) -> bool {
    let Some(mcp) = doc.get_mut("mcp_servers").and_then(Item::as_table_mut) else {
        return false;
    };
    mcp.remove(id).is_some()
}

/// 批量同步：把 callers 给定的 server map 写入 doc。
///
/// 行为：
/// - 已有 id + spec 相同 → 跳过
/// - 已有 id + spec 不同 → 覆盖（计入 updated）
/// - 没有 id → 新增（计入 added）
/// - spec 解析失败 → 跳过（计入 invalid），不影响其他项
///
/// 返回累计计数。
pub fn sync_mcp_servers(doc: &mut DocumentMut, desired: &BTreeMap<String, Value>) -> McpSyncCount {
    let mut count = McpSyncCount::default();
    for (id, spec) in desired {
        match add_or_update_mcp_server(doc, id, spec) {
            Ok(true) => count.updated += 1,
            Ok(false) => count.added += 1,
            Err(_) => count.invalid += 1,
        }
    }
    count
}

// ===== 私有工具函数 =====

fn ensure_mcp_servers_table(doc: &mut DocumentMut) {
    if !doc.get("mcp_servers").map(Item::is_table).unwrap_or(false) {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
}

fn toml_value_to_json(v: &TomlValue) -> Value {
    match v {
        TomlValue::String(s) => Value::String(s.value().to_string()),
        TomlValue::Integer(i) => json!(i.value()),
        TomlValue::Float(f) => json!(f.value()),
        TomlValue::Boolean(b) => json!(b.value()),
        TomlValue::Datetime(dt) => Value::String(dt.to_string()),
        TomlValue::Array(arr) => Value::Array(arr.iter().map(toml_value_to_json).collect()),
        TomlValue::InlineTable(t) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in t.iter() {
                obj.insert(k.to_string(), toml_value_to_json(v));
            }
            Value::Object(obj)
        }
    }
}

fn toml_table_to_spec(table: &Table) -> Result<Value, String> {
    let mut obj = serde_json::Map::new();
    for (key, item) in table.iter() {
        let value = match item {
            Item::Value(v) => toml_value_to_json(v),
            Item::Table(t) => toml_table_to_spec(t)?,
            Item::ArrayOfTables(_) | Item::None => continue,
        };
        obj.insert(key.to_string(), value);
    }
    let spec = Value::Object(obj);
    parse_server_spec(&spec)
}

fn json_to_toml_value(v: &Value) -> Result<TomlValue, String> {
    match v {
        Value::Null => Ok(TomlValue::from(String::new())),
        Value::Bool(b) => Ok(TomlValue::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(TomlValue::from(i))
            } else if let Some(f) = n.as_f64() {
                Ok(TomlValue::from(f))
            } else {
                Err(format!("无法转换 number: {n}"))
            }
        }
        Value::String(s) => Ok(TomlValue::from(s.clone())),
        Value::Array(items) => {
            let mut arr = Array::new();
            for item in items {
                arr.push(json_to_toml_value(item)?);
            }
            Ok(TomlValue::Array(arr))
        }
        Value::Object(obj) => {
            let mut t = InlineTable::new();
            for (k, v) in obj {
                t.insert(k, json_to_toml_value(v)?);
            }
            Ok(TomlValue::InlineTable(t))
        }
    }
}

/// 把 JSON server spec 转为 toml_edit Table。
///
/// 字段映射规则：
/// - `type` → string（自动补 "stdio"）
/// - `command` → string
/// - `args` → array of strings
/// - `env` → table of strings
/// - `cwd` → string
/// - `url` → string
/// - `http_headers` → table of strings
/// - 其它字段按 JSON → TOML 通用规则转换
fn spec_to_toml_table(spec: &Value) -> Result<Table, String> {
    let mut table = Table::new();
    let obj = spec
        .as_object()
        .ok_or_else(|| "spec 不是 object".to_string())?;

    if let Some(typ) = obj.get("type").and_then(Value::as_str) {
        table.insert("type", Item::Value(TomlValue::from(typ.to_string())));
    }

    match obj.get("type").and_then(Value::as_str).unwrap_or("stdio") {
        "stdio" => {
            if let Some(cmd) = obj.get("command").and_then(Value::as_str) {
                table.insert("command", Item::Value(TomlValue::from(cmd.to_string())));
            }
            if let Some(args) = obj.get("args").and_then(Value::as_array) {
                let mut arr = Array::new();
                for a in args {
                    let s = a
                        .as_str()
                        .ok_or_else(|| format!("args 元素不是 string: {a}"))?;
                    arr.push(TomlValue::from(s.to_string()));
                }
                table.insert("args", Item::Value(TomlValue::Array(arr)));
            }
            if let Some(env) = obj.get("env").and_then(Value::as_object) {
                let mut env_table = InlineTable::new();
                for (k, v) in env {
                    let s = v.as_str().ok_or_else(|| format!("env.{k} 不是 string"))?;
                    env_table.insert(k, TomlValue::from(s.to_string()));
                }
                table.insert("env", Item::Value(TomlValue::InlineTable(env_table)));
            }
            if let Some(cwd) = obj.get("cwd").and_then(Value::as_str) {
                table.insert("cwd", Item::Value(TomlValue::from(cwd.to_string())));
            }
        }
        "http" | "sse" => {
            if let Some(url) = obj.get("url").and_then(Value::as_str) {
                table.insert("url", Item::Value(TomlValue::from(url.to_string())));
            }
            if let Some(headers) = obj.get("http_headers").and_then(Value::as_object) {
                let mut header_table = InlineTable::new();
                for (k, v) in headers {
                    let s = v
                        .as_str()
                        .ok_or_else(|| format!("http_headers.{k} 不是 string"))?;
                    header_table.insert(k, TomlValue::from(s.to_string()));
                }
                table.insert(
                    "http_headers",
                    Item::Value(TomlValue::InlineTable(header_table)),
                );
            }
        }
        _ => unreachable!("parse_server_spec 已拒绝未知 type"),
    }

    // 保留额外字段（不在已知字段集内的）
    let known: &[&str] = &[
        "type",
        "command",
        "args",
        "env",
        "cwd",
        "url",
        "http_headers",
    ];
    for (k, v) in obj {
        if known.contains(&k.as_str()) {
            continue;
        }
        table.insert(k, Item::Value(json_to_toml_value(v)?));
    }

    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_config_returns_empty_doc() {
        let doc = parse_codex_config("").unwrap();
        assert!(doc.to_string().is_empty());
    }

    #[test]
    fn list_mcp_servers_on_empty_doc_returns_empty() {
        let doc = parse_codex_config("").unwrap();
        assert!(list_mcp_servers(&doc).is_empty());
    }

    #[test]
    fn list_mcp_servers_extracts_stdio_with_default_type() {
        let text = r#"
[mcp_servers.fs]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[mcp_servers.shell]
type = "stdio"
command = "bash"
"#;
        let doc = parse_codex_config(text).unwrap();
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["fs"]["type"], "stdio");
        assert_eq!(servers["fs"]["command"], "npx");
        assert_eq!(servers["fs"]["args"][0], "-y");
        assert_eq!(servers["shell"]["type"], "stdio");
        assert_eq!(servers["shell"]["command"], "bash");
    }

    #[test]
    fn list_mcp_servers_extracts_http_and_sse() {
        let text = r#"
[mcp_servers.remote_http]
type = "http"
url = "https://example.com/mcp"

[mcp_servers.remote_sse]
type = "sse"
url = "https://example.com/sse"
"#;
        let doc = parse_codex_config(text).unwrap();
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers["remote_http"]["type"], "http");
        assert_eq!(servers["remote_http"]["url"], "https://example.com/mcp");
        assert_eq!(servers["remote_sse"]["type"], "sse");
    }

    #[test]
    fn list_mcp_servers_preserves_env_and_headers() {
        let text = r#"
[mcp_servers.fs]
command = "npx"
args = ["x"]
env = { KEY = "value" }

[mcp_servers.remote]
type = "http"
url = "https://example.com"
http_headers = { Authorization = "Bearer xxx" }
"#;
        let doc = parse_codex_config(text).unwrap();
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers["fs"]["env"]["KEY"], "value");
        assert_eq!(
            servers["remote"]["http_headers"]["Authorization"],
            "Bearer xxx"
        );
    }

    #[test]
    fn parse_server_spec_defaults_type_to_stdio() {
        let spec = json!({"command": "ls"});
        let parsed = parse_server_spec(&spec).unwrap();
        assert_eq!(parsed["type"], "stdio");
    }

    #[test]
    fn parse_server_spec_rejects_unknown_type() {
        let spec = json!({"type": "ws", "url": "wss://example.com"});
        let err = parse_server_spec(&spec).unwrap_err();
        assert!(err.contains("未知 server type"));
    }

    #[test]
    fn parse_server_spec_requires_command_for_stdio() {
        let spec = json!({"type": "stdio"});
        let err = parse_server_spec(&spec).unwrap_err();
        assert!(err.contains("stdio server 缺少 command"));
    }

    #[test]
    fn parse_server_spec_requires_url_for_http() {
        let spec = json!({"type": "http"});
        let err = parse_server_spec(&spec).unwrap_err();
        assert!(err.contains("http server 缺少 url"));
    }

    #[test]
    fn parse_server_spec_requires_url_for_sse() {
        let spec = json!({"type": "sse"});
        let err = parse_server_spec(&spec).unwrap_err();
        assert!(err.contains("sse server 缺少 url"));
    }

    #[test]
    fn parse_server_spec_rejects_non_object() {
        let err = parse_server_spec(&Value::String("nope".into())).unwrap_err();
        assert!(err.contains("必须是 JSON object"));
    }

    #[test]
    fn add_mcp_server_to_empty_doc() {
        let mut doc = parse_codex_config("").unwrap();
        let existed = add_or_update_mcp_server(
            &mut doc,
            "fs",
            &json!({"type":"stdio", "command":"npx", "args":["-y"]}),
        )
        .unwrap();
        assert!(!existed);
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers["fs"]["command"], "npx");
        assert_eq!(servers["fs"]["args"][0], "-y");
    }

    #[test]
    fn add_then_update_mcp_server() {
        let mut doc = parse_codex_config("").unwrap();
        add_or_update_mcp_server(&mut doc, "fs", &json!({"command":"npx", "args":["-y"]})).unwrap();
        let existed = add_or_update_mcp_server(
            &mut doc,
            "fs",
            &json!({"command":"node", "args":["server.js"]}),
        )
        .unwrap();
        assert!(existed);
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers["fs"]["command"], "node");
        assert_eq!(servers["fs"]["args"][0], "server.js");
    }

    #[test]
    fn add_rejects_empty_id() {
        let mut doc = parse_codex_config("").unwrap();
        let err = add_or_update_mcp_server(&mut doc, "", &json!({"command":"x"})).unwrap_err();
        assert!(err.contains("不能为空"));
    }

    #[test]
    fn add_rejects_invalid_spec() {
        let mut doc = parse_codex_config("").unwrap();
        let err = add_or_update_mcp_server(&mut doc, "bad", &json!({"type":"ws"})).unwrap_err();
        assert!(err.contains("未知 server type"));
        // doc 应保持原状
        assert!(list_mcp_servers(&doc).is_empty());
    }

    #[test]
    fn remove_mcp_server_returns_true_when_existed() {
        let mut doc = parse_codex_config("").unwrap();
        add_or_update_mcp_server(&mut doc, "fs", &json!({"command":"npx"})).unwrap();
        assert!(remove_mcp_server(&mut doc, "fs"));
        assert!(list_mcp_servers(&doc).is_empty());
    }

    #[test]
    fn remove_mcp_server_returns_false_when_missing() {
        let mut doc = parse_codex_config("").unwrap();
        assert!(!remove_mcp_server(&mut doc, "nope"));
    }

    #[test]
    fn remove_on_doc_without_mcp_servers_returns_false() {
        let mut doc = parse_codex_config(
            r#"[other]
key = "value"
"#,
        )
        .unwrap();
        assert!(!remove_mcp_server(&mut doc, "anything"));
    }

    #[test]
    fn preserves_other_sections_after_edit() {
        let text = r#"
model_provider = "openai"

[mcp_servers.fs]
command = "npx"

[model_providers.openai]
name = "OpenAI"
"#;
        let mut doc = parse_codex_config(text).unwrap();
        add_or_update_mcp_server(&mut doc, "shell", &json!({"command":"bash"})).unwrap();
        let serialized = serialize_codex_config(&doc);
        assert!(serialized.contains(r#"model_provider = "openai""#));
        assert!(serialized.contains("[mcp_servers.fs]"));
        assert!(serialized.contains("[mcp_servers.shell]"));
        assert!(serialized.contains("[model_providers.openai]"));
        assert!(serialized.contains(r#"name = "OpenAI""#));
    }

    #[test]
    fn preserves_other_sections_after_remove() {
        let text = r#"
model_provider = "openai"

[mcp_servers.fs]
command = "npx"
"#;
        let mut doc = parse_codex_config(text).unwrap();
        remove_mcp_server(&mut doc, "fs");
        let serialized = serialize_codex_config(&doc);
        assert!(serialized.contains(r#"model_provider = "openai""#));
        assert!(!serialized.contains("mcp_servers"));
    }

    #[test]
    fn sync_mcp_servers_counts_added_updated_invalid() {
        let mut doc = parse_codex_config("").unwrap();
        add_or_update_mcp_server(&mut doc, "existing", &json!({"command":"old"})).unwrap();
        let mut desired = BTreeMap::new();
        desired.insert("existing".to_string(), json!({"command":"new"}));
        desired.insert("new_one".to_string(), json!({"command":"bash"}));
        desired.insert("bad".to_string(), json!({"type":"ws"}));
        let count = sync_mcp_servers(&mut doc, &desired);
        assert_eq!(count.added, 1);
        assert_eq!(count.updated, 1);
        assert_eq!(count.invalid, 1);
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers["existing"]["command"], "new");
        assert_eq!(servers["new_one"]["command"], "bash");
    }

    #[test]
    fn round_trip_preserves_all_three_transport_types() {
        let mut doc = parse_codex_config("").unwrap();
        add_or_update_mcp_server(
            &mut doc,
            "stdio_one",
            &json!({
                "type":"stdio",
                "command":"python",
                "args":["-m","server"],
                "env":{"KEY":"VAL"},
                "cwd":"/tmp"
            }),
        )
        .unwrap();
        add_or_update_mcp_server(
            &mut doc,
            "http_one",
            &json!({
                "type":"http",
                "url":"https://example.com",
                "http_headers":{"Auth":"Bearer xyz"}
            }),
        )
        .unwrap();
        add_or_update_mcp_server(
            &mut doc,
            "sse_one",
            &json!({"type":"sse","url":"https://example.com/sse"}),
        )
        .unwrap();

        let serialized = serialize_codex_config(&doc);
        let reparsed = parse_codex_config(&serialized).unwrap();
        let servers = list_mcp_servers(&reparsed);
        assert_eq!(servers.len(), 3);
        assert_eq!(servers["stdio_one"]["env"]["KEY"], "VAL");
        assert_eq!(servers["http_one"]["http_headers"]["Auth"], "Bearer xyz");
        assert_eq!(servers["sse_one"]["url"], "https://example.com/sse");
    }

    #[test]
    fn round_trip_preserves_unknown_extra_fields() {
        let mut doc = parse_codex_config("").unwrap();
        add_or_update_mcp_server(
            &mut doc,
            "fs",
            &json!({
                "command":"npx",
                "startup_timeout_sec": 30,
                "enabled": false
            }),
        )
        .unwrap();
        let serialized = serialize_codex_config(&doc);
        let reparsed = parse_codex_config(&serialized).unwrap();
        let servers = list_mcp_servers(&reparsed);
        assert_eq!(servers["fs"]["startup_timeout_sec"], 30);
        assert_eq!(servers["fs"]["enabled"], false);
    }

    #[test]
    fn file_round_trip_works() {
        let dir = std::env::temp_dir().join(format!("codex_mcp_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let initial = r#"
model_provider = "openai"

[mcp_servers.fs]
command = "npx"
"#;
        write_codex_config_file(&path, initial).unwrap();

        let text = read_codex_config_file(&path).unwrap();
        let mut doc = parse_codex_config(&text).unwrap();
        add_or_update_mcp_server(&mut doc, "shell", &json!({"command":"bash"})).unwrap();
        write_codex_config_file(&path, &serialize_codex_config(&doc)).unwrap();

        let text2 = read_codex_config_file(&path).unwrap();
        assert!(text2.contains("[mcp_servers.fs]"));
        assert!(text2.contains("[mcp_servers.shell]"));
        assert!(text2.contains(r#"model_provider = "openai""#));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_nonexistent_file_returns_error() {
        let path = std::path::Path::new("/nonexistent/config.toml");
        let err = read_codex_config_file(path).unwrap_err();
        assert!(err.contains("读取"));
    }

    #[test]
    fn list_mcp_servers_skips_invalid_entries_with_null() {
        let text = r#"
[mcp_servers.bad]
type = "ws"

[mcp_servers.good]
command = "ls"
"#;
        let doc = parse_codex_config(text).unwrap();
        let servers = list_mcp_servers(&doc);
        assert_eq!(servers.len(), 2);
        assert!(servers["bad"].is_null());
        assert_eq!(servers["good"]["command"], "ls");
    }
}
