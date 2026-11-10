//! WorkBuddy 资源（专家 / 技能 / 连接器）只读清单命令。
//!
//! 数据来源：`~/.deecodex/wb-resources/`，由 `scripts/import-workbuddy-resources.sh`
//! 从 WorkBuddy 提取生成：
//!   - `connectors/` + `connectors-index.json`（连接器 + 中文索引）
//!   - `skills/`（技能包，含 `source-plugin.json` 中文描述）
//!   - `agents/` + `agents-index.json`（429 个专家：中文元数据 + prompt + 头像）
//!
//! 本模块只做扫描与摘要，不做安装/写入。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};

use deecodex::config;

/// WorkBuddy 资源 CDN 根（api.liguan.me / Mobile Codex 服务器）
const WB_CDN_BASE: &str = "https://api.liguan.me/releases/dex-ai/wb-resources";

/// 启动时同步的清单文件（体积小，先拉清单即可浏览）
const WB_CORE_MANIFESTS: [&str; 4] = [
    "skills-cache.json",
    "skills-icons.json",
    "agents-index.json",
    "connectors-index.json",
];

struct WbCache {
    fingerprint: String,
    data: Value,
}

fn wb_cache() -> &'static Mutex<Option<WbCache>> {
    static CACHE: OnceLock<Mutex<Option<WbCache>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn mtime_nanos(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 计算 wb-resources 的指纹（关键文件/目录 mtime），用于缓存失效。
fn wb_resources_fingerprint(root: &Path) -> String {
    let mut parts = Vec::new();
    for rel in [
        "skills",
        "connectors",
        "agents",
        "skills-icons",
        "connectors-icons",
    ] {
        parts.push(format!("{}={}", rel, mtime_nanos(&root.join(rel))));
    }
    for rel in [
        "skills-cache.json",
        "skills-icons.json",
        "agents-index.json",
        "connectors-index.json",
    ] {
        parts.push(format!("{}={}", rel, mtime_nanos(&root.join(rel))));
    }
    parts.join("|")
}

/// WorkBuddy 资源根目录：`~/.deecodex/wb-resources`
fn wb_resources_root() -> Option<PathBuf> {
    config::home_dir().map(|home| home.join(".deecodex").join("wb-resources"))
}

/// 从 CDN 下载单个资源到本地（幂等：本地已存在且非空则跳过）。
async fn download_wb_asset(rel: &str) -> Result<PathBuf, String> {
    let Some(root) = wb_resources_root() else {
        return Err("无法确定 HOME 目录".to_string());
    };
    let rel_trim = rel.trim_start_matches('/');
    let dest = root.join(rel_trim);
    if dest.is_file() {
        if let Ok(meta) = fs::metadata(&dest) {
            if meta.len() > 0 {
                return Ok(dest);
            }
        }
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let url = format!("{WB_CDN_BASE}/{rel_trim}");
    let resp = reqwest::get(&url)
        .await
        .map_err(|e| format!("下载失败 {rel_trim}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败 {rel_trim}: HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败 {rel_trim}: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("下载为空 {rel_trim}"));
    }
    fs::write(&dest, &bytes).map_err(|e| format!("写入失败 {rel_trim}: {e}"))?;
    Ok(dest)
}

/// 同步核心清单（skills-cache / skills-icons / agents-index / connectors-index）。
#[tauri::command]
pub async fn sync_wb_resources() -> Result<Value, String> {
    let mut downloaded = Vec::new();
    let mut existing = Vec::new();
    for rel in WB_CORE_MANIFESTS {
        let Some(root) = wb_resources_root() else {
            return Err("无法确定 HOME 目录".to_string());
        };
        let dest = root.join(rel);
        let had = dest.is_file();
        match download_wb_asset(rel).await {
            Ok(_) => {
                if had {
                    existing.push(rel.to_string());
                } else {
                    downloaded.push(rel.to_string());
                }
            }
            Err(e) => {
                tracing::warn!(rel, "同步 WorkBuddy 资源失败: {e}");
            }
        }
    }
    Ok(json!({
        "downloaded": downloaded,
        "existing": existing,
        "cdn": WB_CDN_BASE,
    }))
}

/// 按需下载单个资源（图标/头像/mcp.json 等），返回本地路径。
#[tauri::command]
pub async fn fetch_wb_asset(rel: String) -> Result<Value, String> {
    let path = download_wb_asset(&rel).await?;
    Ok(json!({ "rel": rel, "path": path.to_string_lossy().to_string() }))
}

/// 按 manifest 下载一个资源目录的全部文件（Skill/MCP/Agent 本体），供复制/安装前落地。
#[tauri::command]
pub async fn fetch_wb_resource(kind: String, id: String) -> Result<Value, String> {
    let Some(root) = wb_resources_root() else {
        return Err("无法确定 HOME 目录".to_string());
    };
    let kind_key = if kind == "skill" {
        "skills"
    } else if kind == "connector" {
        "connectors"
    } else {
        "agents"
    };
    let manifest_path = download_wb_asset("wb-resources-manifest.json").await?;
    let content =
        fs::read_to_string(&manifest_path).map_err(|e| format!("读取 manifest 失败: {e}"))?;
    let manifest: Value =
        serde_json::from_str(&content).map_err(|e| format!("解析 manifest 失败: {e}"))?;
    let files = manifest
        .get(kind_key)
        .and_then(|m| m.get(&id))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if files.is_empty() {
        return Err(format!("清单中未找到 {kind_key}/{id} 的文件"));
    }
    let mut downloaded = Vec::new();
    let mut skipped = 0usize;
    for file in &files {
        let Some(rel) = file.as_str() else { continue };
        let dest = root.join(rel);
        if dest.is_file() {
            skipped += 1;
            continue;
        }
        match download_wb_asset(rel).await {
            Ok(_) => downloaded.push(rel.to_string()),
            Err(e) => tracing::warn!(rel, "下载资源文件失败: {e}"),
        }
    }
    Ok(json!({
        "kind": kind,
        "id": id,
        "downloaded": downloaded,
        "skipped": skipped,
        "dir": root.join(kind_key).join(&id).to_string_lossy().to_string(),
    }))
}

/// 读取 JSON 文件，失败返回 None 并打日志。
fn read_json(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&content) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(path = %path.display(), "解析 JSON 失败: {error}");
            None
        }
    }
}

/// 读取 SKILL.md 的 YAML frontmatter，返回 Value（解析失败返回空对象）。
fn skill_frontmatter(path: &Path) -> Value {
    let Ok(content) = fs::read_to_string(path) else {
        return json!({});
    };
    let content = content.trim_start();
    if !content.starts_with("---") {
        return json!({});
    }
    let Some(end) = content[3..].find("\n---") else {
        return json!({});
    };
    let yaml_block = &content[3..3 + end];
    let Ok(value) = serde_yaml::from_str::<Value>(yaml_block) else {
        return json!({});
    };
    value
}

/// 从 YAML 字符串值或 Value 取字符串。
fn yaml_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// 判断字符串是否"像英文"（无 CJK 字符）。
fn is_mostly_ascii(text: &str) -> bool {
    !text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 从中文描述推导技能显示名：
/// - 描述无中文时回退到技术名；
/// - 优先取「X：」中的 X（中文短语）；
/// - 否则取开头一段中文（到标点或 12 字以内）。
fn derive_skill_display_name(name: &str, description: &str) -> String {
    if is_mostly_ascii(description) || description.trim().is_empty() {
        return name.to_string();
    }
    // 「X：」形式
    if let Some(idx) = description.find('：') {
        let head = &description[..idx];
        let cn_count = head
            .chars()
            .filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c))
            .count();
        if cn_count >= 2 && head.chars().count() <= 14 {
            return head.trim().to_string();
        }
    }
    // 取开头中文短语：到中文标点为止；若无标点则取前 12 字
    let mut out = String::new();
    for ch in description.chars() {
        if out.chars().count() >= 12 {
            break;
        }
        if "，。、；！？：,!?;".contains(ch) {
            if !out.trim().is_empty() {
                break;
            }
            continue;
        }
        out.push(ch);
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        name.to_string()
    } else {
        out
    }
}

/// 从 frontmatter metadata.keywords 提取中文关键词，拼接为描述补充。
fn metadata_zh_keywords(front: &Value) -> String {
    let Some(meta) = front.get("metadata") else {
        return String::new();
    };
    let keywords = meta
        .get("keywords")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let zh: Vec<String> = keywords
        .iter()
        .filter_map(|k| k.as_str().map(str::to_string))
        .filter(|k| !is_mostly_ascii(k))
        .take(6)
        .collect();
    zh.join("、")
}

/// 加载 WorkBuddy 技能中心缓存（skills-cache.json），按技能 name 建索引。
fn load_skill_cache(root: &Path) -> HashMap<String, Value> {
    let mut index = HashMap::new();
    let Some(value) = read_json(&root.join("skills-cache.json")) else {
        return index;
    };
    let Some(skills) = value
        .get("skills-cache-builtin-market")
        .and_then(Value::as_array)
    else {
        return index;
    };
    for skill in skills {
        let Some(name) = skill.get("name").and_then(Value::as_str) else {
            continue;
        };
        index.insert(name.to_string(), skill.clone());
    }
    index
}

/// 加载技能图标映射（skills-icons.json：name -> 本地文件名）。
fn load_skill_icons(root: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(value) = read_json(&root.join("skills-icons.json")) else {
        return map;
    };
    let Some(object) = value.as_object() else {
        return map;
    };
    for (name, file) in object {
        if let Some(file) = file.as_str() {
            map.insert(name.clone(), file.to_string());
        }
    }
    map
}

/// 扫描一个技能目录，返回摘要或 None。
/// 描述优先级：source-plugin.json（插件中文描述）> SKILL.md description > metadata 中文关键词。
fn scan_skill_dir(dir: &Path) -> Option<Value> {
    let skill_md = dir.join("SKILL.md");
    if !skill_md.is_file() {
        return None;
    }
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let front = skill_frontmatter(&skill_md);
    let name = {
        let n = yaml_str(&front, "name");
        if n.is_empty() {
            id.clone()
        } else {
            n
        }
    };
    let mut description = yaml_str(&front, "description");
    // 插件级中文描述
    let plugin_json = dir.join("source-plugin.json");
    if let Some(value) = read_json(&plugin_json) {
        let plugin_desc = yaml_str(&value, "description");
        if !plugin_desc.is_empty() && (description.is_empty() || is_mostly_ascii(&description)) {
            description = plugin_desc;
        }
    }
    // metadata 中文关键词兜底
    if description.is_empty() || is_mostly_ascii(&description) {
        let zh = metadata_zh_keywords(&front);
        if !zh.is_empty() {
            description = format!("{}。{}", zh, description);
        }
    }
    let display_name = derive_skill_display_name(&name, &description);
    let source = if id.starts_with("builtin-") {
        "builtin"
    } else if id.starts_with("market-") {
        "market"
    } else if id.starts_with("ext-") {
        "ext"
    } else if id.starts_with("teams-") {
        "teams"
    } else {
        "other"
    };
    Some(json!({
        "id": id,
        "name": name,
        "display_name": display_name,
        "description": description,
        "source": source,
        "path": dir.to_string_lossy().to_string(),
    }))
}

/// 扫描 connectors 目录下的技能子目录（skill/ 或 skills/）。
fn scan_connector_skills(conn_dir: &Path) -> Vec<Value> {
    let mut items = Vec::new();
    for skills_root in [conn_dir.join("skill"), conn_dir.join("skills")] {
        if !skills_root.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&skills_root).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(item) = scan_skill_dir(&path) {
                    items.push(item);
                }
            } else if path.file_name().is_some_and(|n| n == "SKILL.md") {
                if let Some(item) = scan_skill_dir(&skills_root) {
                    items.push(item);
                }
            }
        }
    }
    items
}

/// 从 skills-cache.json 生成技能条目（含中文名/介绍/图标/示例）。
fn skill_entry_from_cache(skill: &Value, icons: &HashMap<String, String>) -> Option<Value> {
    let name = skill.get("name").and_then(Value::as_str)?.to_string();
    let display_name = skill
        .get("display_name_zh")
        .or_else(|| skill.get("display_name_en"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| name.clone());
    let description = skill
        .get("description_zh")
        .or_else(|| skill.get("description_en"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let examples: Vec<String> = skill
        .get("examples_zh")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .take(3)
                .collect()
        })
        .unwrap_or_default();
    let categories: Vec<String> = skill
        .get("categories")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let featured = skill.get("featured").and_then(Value::as_u64).unwrap_or(0);
    let sort_order = skill.get("sort_order").and_then(Value::as_u64);
    let featured_order = skill.get("featured_order").and_then(Value::as_u64);
    let updated_at = skill
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let icon = icons
        .get(&name)
        .cloned()
        .unwrap_or_else(|| "letter".to_string());
    Some(json!({
        "id": format!("wb-{name}"),
        "name": name,
        "display_name": display_name,
        "description": description,
        "source": skill.get("source").and_then(Value::as_str).unwrap_or("official"),
        "icon": icon,
        "examples": examples,
        "categories": categories,
        "featured": featured,
        "sort_order": sort_order.unwrap_or(u64::MAX),
        "featured_order": featured_order.unwrap_or(u64::MAX),
        "updated_at": updated_at,
        "path": "",
        "from_market": true,
    }))
}

/// 加载连接器中文索引（connectors-index.json）。
fn load_connector_index(root: &Path) -> HashMap<String, Value> {
    let mut index = HashMap::new();
    let Some(value) = read_json(&root.join("connectors-index.json")) else {
        return index;
    };
    let Some(connectors) = value.get("connectors").and_then(Value::as_array) else {
        return index;
    };
    for connector in connectors {
        let Some(id) = connector.get("id").and_then(Value::as_str) else {
            continue;
        };
        let name = connector
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let description = connector
            .get("description_zh")
            .or_else(|| connector.get("description"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let examples: Vec<String> = connector
            .get("examples_zh")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        index.insert(
            id.to_string(),
            json!({ "name": name, "description": description, "examples": examples }),
        );
    }
    index
}

/// 扫描一个连接器目录，返回摘要或 None（含中文索引合并）。
fn scan_connector_dir(dir: &Path, index: &HashMap<String, Value>) -> Option<Value> {
    let id = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if id.is_empty() {
        return None;
    }

    let mut mcp_servers = Vec::new();
    let mcp_path = dir.join("mcp.json");
    if let Ok(content) = fs::read_to_string(&mcp_path) {
        if let Ok(value) = serde_json::from_str::<Value>(&content) {
            if let Some(servers) = value.get("mcpServers").and_then(Value::as_object) {
                for (name, spec) in servers {
                    mcp_servers.push(json!({
                        "name": name,
                        "type": if spec.get("command").is_some() { "stdio" } else { "http" },
                        "url": spec.get("url").and_then(Value::as_str).unwrap_or(""),
                        "command": spec.get("command").and_then(Value::as_str).unwrap_or(""),
                        "args": spec.get("args").cloned().unwrap_or_else(|| json!([])),
                    }));
                }
            }
        } else {
            tracing::warn!(path = %mcp_path.display(), "解析连接器 mcp.json 失败");
        }
    }

    let mut cli = Value::Null;
    let cli_path = dir.join("cli.json");
    if cli_path.is_file() {
        if let Ok(content) = fs::read_to_string(&cli_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&content) {
                cli = json!({
                    "runtime": value.get("runtime").cloned().unwrap_or_else(|| json!({})),
                    "init": value.get("init").cloned().unwrap_or_else(|| json!({})),
                });
            }
        }
    }

    let skills = scan_connector_skills(dir);
    let kind = if !mcp_servers.is_empty() && !cli.is_null() {
        "mixed"
    } else if !mcp_servers.is_empty() {
        "mcp"
    } else if !cli.is_null() {
        "cli"
    } else {
        "unknown"
    };

    let meta = index.get(&id).cloned().unwrap_or_else(|| json!({}));
    let name = meta
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());
    let description = meta
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let examples = meta.get("examples").cloned().unwrap_or_else(|| json!([]));
    // 连接器图标（connectors-icons/<id>.svg|png）
    let mut icon = "letter".to_string();
    if let Some(root) = dir.parent().and_then(|p| p.parent()) {
        let icons_dir = root.join("connectors-icons");
        for ext in ["svg", "png", "jpg"] {
            if icons_dir.join(format!("{id}.{ext}")).is_file() {
                icon = format!("{id}.{ext}");
                break;
            }
        }
    }

    Some(json!({
        "id": id,
        "name": name,
        "description": description,
        "examples": examples,
        "icon": icon,
        "kind": kind,
        "path": dir.to_string_lossy().to_string(),
        "mcp_servers": mcp_servers,
        "cli": cli,
        "skills": skills,
        "skill_count": skills.len(),
    }))
}

/// 从 connectors-index.json 生成连接器条目（本地无 mcp.json 时的索引条目）。
/// 让 MCP 专区在本地连接器目录未同步时也能列出全部连接器，点击安装时再按需下载。
fn connector_entry_from_index(id: &str, meta: &Value) -> Option<Value> {
    let name = meta
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| id.to_string());
    let description = meta
        .get("description_zh")
        .or_else(|| meta.get("description"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let examples: Vec<Value> = meta
        .get("examples_zh")
        .or_else(|| meta.get("examples"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(json!({
        "id": id,
        "name": name,
        "description": description,
        "examples": examples,
        "icon": "letter",
        "kind": "unknown",
        "path": "",
        "mcp_servers": [],
        "cli": null,
        "skills": [],
        "skill_count": 0,
        "source": "index",
    }))
}

/// 解析本地专家 plugin.json（如 we-chat-mini-program-developer），返回摘要或 None。
fn scan_agent_dir(dir: &Path) -> Option<Value> {
    let plugin_json = dir.join(".codebuddy-plugin").join("plugin.json");
    let content = fs::read_to_string(&plugin_json).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;
    let id = value
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });
    let pick = |key: &str| -> String {
        value
            .get(key)
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("zh").or_else(|| obj.get("en")))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_default()
    };
    let description = {
        let display = pick("displayDescription");
        if !display.is_empty() {
            display
        } else {
            value
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_default()
        }
    };
    let tags: Vec<String> = value
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|tag| {
                    tag.get("zh")
                        .or_else(|| tag.get("en"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let quick_prompts: Vec<String> = value
        .get("quickPrompts")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|p| {
                    p.get("zh")
                        .or_else(|| p.get("en"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut agent_files = Vec::new();
    if let Some(agents) = value.get("agents").and_then(Value::as_array) {
        for agent in agents {
            if let Some(path) = agent.as_str() {
                agent_files.push(path.to_string());
            }
        }
    }
    let mut skills = Vec::new();
    if let Some(skill_paths) = value.get("skills").and_then(Value::as_array) {
        for skill in skill_paths {
            if let Some(path) = skill.as_str() {
                skills.push(path.to_string());
            }
        }
    }
    Some(json!({
        "id": id,
        "name": id,
        "expert_type": value.get("expertType").and_then(Value::as_str).unwrap_or("agent"),
        "display_name": pick("displayName"),
        "profession": pick("profession"),
        "description": description,
        "category": value.get("categoryId").and_then(Value::as_str).unwrap_or(""),
        "avatar": value.get("avatar").and_then(Value::as_str).unwrap_or(""),
        "tags": tags,
        "quick_prompts": quick_prompts,
        "skills": skills,
        "agent_files": agent_files,
        "version": value.get("version").and_then(Value::as_str).unwrap_or(""),
        "source": "workbuddy-local",
        "path": dir.to_string_lossy().to_string(),
    }))
}

/// 从 agents-index.json（WorkBuddy 专家市场）生成专家条目。
fn agents_from_index(root: &Path) -> Vec<Value> {
    let mut items = Vec::new();
    let Some(value) = read_json(&root.join("agents-index.json")) else {
        return items;
    };
    let Some(experts) = value.get("experts").and_then(Value::as_array) else {
        return items;
    };
    for expert in experts {
        let Some(id) = expert.get("id").and_then(Value::as_str) else {
            continue;
        };
        let plugin = expert
            .get("plugin")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let pick = |key: &str| -> String {
            expert
                .get(key)
                .and_then(Value::as_object)
                .and_then(|obj| obj.get("zh").or_else(|| obj.get("en")))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_default()
        };
        let tags: Vec<String> = expert
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|tag| {
                        tag.get("zh")
                            .or_else(|| tag.get("en"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let quick_prompts: Vec<String> = expert
            .get("quickPrompts")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        p.get("zh")
                            .or_else(|| p.get("en"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let prompt_file = expert
            .get("promptFile")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let avatar = expert
            .get("avatar")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // 本地 prompt 文件（如果已下载）
        let agent_dir = root.join("agents").join(&plugin).join("agents");
        let prompt_name = Path::new(&prompt_file)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let prompt_path = if !prompt_name.is_empty() {
            agent_dir.join(&prompt_name).to_string_lossy().to_string()
        } else {
            String::new()
        };
        items.push(json!({
            "id": plugin,
            "name": id.to_string(),
            "expert_type": expert.get("expertType").and_then(Value::as_str).unwrap_or("agent"),
            "display_name": pick("displayName"),
            "profession": pick("profession"),
            "description": pick("description"),
            "category": expert.get("categoryId").and_then(Value::as_str).unwrap_or(""),
            "avatar": avatar,
            "tags": tags,
            "quick_prompts": quick_prompts,
            "default_init_prompt": pick("defaultInitPrompt"),
            "skills": json!([]),
            "agent_files": if prompt_path.is_empty() { json!([]) } else { json!([prompt_path]) },
            "source": "workbuddy",
            "path": agent_dir.to_string_lossy().to_string(),
        }));
    }
    items
}

/// Tauri 命令：返回 WorkBuddy 资源（连接器 / 技能 / 专家）清单。
#[tauri::command]
pub async fn list_wb_resources() -> Result<Value, String> {
    let Some(root) = wb_resources_root() else {
        return Ok(
            json!({ "root": "", "connectors": [], "skills": [], "agents": [], "stats": {} }),
        );
    };
    let fingerprint = wb_resources_fingerprint(&root);
    if let Ok(guard) = wb_cache().lock() {
        if let Some(cache) = guard.as_ref() {
            if cache.fingerprint == fingerprint {
                return Ok(cache.data.clone());
            }
        }
    }

    let connector_index = load_connector_index(&root);
    let mut connectors = Vec::new();
    let connectors_root = root.join("connectors");
    if let Ok(entries) = fs::read_dir(&connectors_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(item) = scan_connector_dir(&path, &connector_index) {
                    connectors.push(item);
                }
            }
        }
    } else {
        tracing::warn!(path = %connectors_root.display(), "WorkBuddy 连接器目录不可读");
    }
    // 补充 connectors-index 中本地未提取的连接器（索引驱动，本地无 mcp.json 也可浏览）
    for (id, meta) in &connector_index {
        let exists = connectors
            .iter()
            .any(|c| c.get("id").and_then(Value::as_str) == Some(id.as_str()));
        if !exists {
            if let Some(entry) = connector_entry_from_index(id, meta) {
                connectors.push(entry);
            }
        }
    }

    let skill_cache = load_skill_cache(&root);
    let skill_icons = load_skill_icons(&root);
    let mut skills = Vec::new();
    let skills_root = root.join("skills");
    if let Ok(entries) = fs::read_dir(&skills_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(item) = scan_skill_dir(&path) {
                    let mut item = item;
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if let Some(cached) = skill_cache.get(&name) {
                        // 用 WorkBuddy 技能中心的权威中文数据覆盖
                        if let Some(zh) = cached
                            .get("display_name_zh")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            item["display_name"] = json!(zh);
                        }
                        if let Some(zh) = cached
                            .get("description_zh")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                        {
                            item["description"] = json!(zh);
                        }
                        if let Some(v) = cached.get("featured") {
                            item["featured"] = v.clone();
                        }
                        if let Some(v) = cached.get("sort_order") {
                            item["sort_order"] = v.clone();
                        }
                        if let Some(v) = cached.get("featured_order") {
                            item["featured_order"] = v.clone();
                        }
                        if let Some(v) = cached.get("updated_at") {
                            item["updated_at"] = v.clone();
                        }
                        if let Some(examples) = cached
                            .get("examples_zh")
                            .and_then(Value::as_array)
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .take(3)
                                    .collect::<Vec<_>>()
                            })
                        {
                            item["examples"] = json!(examples);
                        }
                    }
                    if item.get("examples").is_none() {
                        item["examples"] = json!([]);
                    }
                    // 所有技能统一用 skills-icons 映射设置图标（本地技能同样生效）
                    item["icon"] = skill_icons
                        .get(&name)
                        .map(|icon| json!(icon))
                        .unwrap_or_else(|| json!("letter"));
                    skills.push(item);
                }
            }
        }
    } else {
        tracing::warn!(path = %skills_root.display(), "WorkBuddy 技能目录不可读");
    }
    // 补充 skills-cache 中本地未提取的技能（WorkBuddy 技能中心清单）
    for cached in skill_cache.values() {
        let name = cached.get("name").and_then(Value::as_str).unwrap_or("");
        let exists = skills
            .iter()
            .any(|s| s.get("name").and_then(Value::as_str) == Some(name));
        if !exists {
            if let Some(entry) = skill_entry_from_cache(cached, &skill_icons) {
                skills.push(entry);
            }
        }
    }

    // 专家：WorkBuddy 专家市场 manifest（中文）+ 本地额外专家包
    let mut agents = agents_from_index(&root);
    let agents_root = root.join("agents");
    if let Ok(entries) = fs::read_dir(&agents_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // 已由 manifest 覆盖的跳过
            let dir_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if agents
                .iter()
                .any(|a| a.get("id").and_then(Value::as_str) == Some(dir_name.as_str()))
            {
                continue;
            }
            if let Some(item) = scan_agent_dir(&path) {
                agents.push(item);
            }
        }
    }

    // 连接器按官方索引顺序（WorkBuddy 推荐顺序），未收录的放最后按名称
    let connector_order = {
        let mut order = HashMap::new();
        if let Some(value) = read_json(&root.join("connectors-index.json")) {
            if let Some(arr) = value.get("connectors").and_then(Value::as_array) {
                for (i, c) in arr.iter().enumerate() {
                    if let Some(id) = c.get("id").and_then(Value::as_str) {
                        order.insert(id.to_string(), i);
                    }
                }
            }
        }
        order
    };
    connectors.sort_by(|a, b| {
        let oa = a
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| connector_order.get(id))
            .copied()
            .unwrap_or(usize::MAX);
        let ob = b
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| connector_order.get(id))
            .copied()
            .unwrap_or(usize::MAX);
        oa.cmp(&ob).then_with(|| {
            a.get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(b.get("name").and_then(Value::as_str).unwrap_or(""))
        })
    });
    // Skill 专区按官方热门排序：精选(featured)最前 -> sort_order -> featured_order -> 更新时间 -> 名称
    skills.sort_by(|a, b| {
        fn hot_key(s: &Value) -> (u8, u64, u64, std::cmp::Reverse<String>, String) {
            let featured = if s.get("featured").and_then(Value::as_u64).unwrap_or(0) > 0 {
                0
            } else {
                1
            };
            let sort_order = s
                .get("sort_order")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            let featured_order = s
                .get("featured_order")
                .and_then(Value::as_u64)
                .unwrap_or(u64::MAX);
            let updated_at = s
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = s
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            (
                featured,
                sort_order,
                featured_order,
                std::cmp::Reverse(updated_at),
                name,
            )
        }
        hot_key(a).cmp(&hot_key(b))
    });
    let result = json!({
        "root": root.to_string_lossy().to_string(),
        "connectors": connectors,
        "skills": skills,
        "agents": agents,
        "stats": {
            "connectors": connectors.len(),
            "skills": skills.len(),
            "agents": agents.len(),
        },
    });
    if let Ok(mut guard) = wb_cache().lock() {
        *guard = Some(WbCache {
            fingerprint,
            data: result.clone(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "wb-resource-test-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn skill_frontmatter_parses_name_and_description() {
        let dir = tmp_dir("frontmatter");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: my-skill\ndescription: does things\n---\n\nbody\n"),
        )
        .unwrap();
        let meta = skill_frontmatter(&dir.join("SKILL.md"));
        assert_eq!(meta["name"], "my-skill");
        assert_eq!(meta["description"], "does things");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn skill_description_prefers_chinese_plugin_description() {
        let dir = tmp_dir("plugin-desc");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: pdf\ndescription: Use this skill for PDF files\n---\n\nbody\n"),
        )
        .unwrap();
        fs::write(
            dir.join("source-plugin.json"),
            r#"{"name":"pdf","description":"全面的 PDF 处理工具包"}"#,
        )
        .unwrap();
        let item = scan_skill_dir(&dir).unwrap();
        assert_eq!(item["name"], "pdf");
        assert_eq!(item["description"], "全面的 PDF 处理工具包");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn connector_merges_chinese_index() {
        let root = tmp_dir("conn-index");
        let dir = root.join("connectors").join("feishu");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("cli.json"), r#"{"runtime":{"type":"node"}}"#).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("connectors-index.json"),
            r#"{"connectors":[{"id":"feishu","name":"飞书","description_zh":"飞书办公套件","examples_zh":["发消息"]}]}"#,
        )
        .unwrap();
        let index = load_connector_index(&root);
        let item = scan_connector_dir(&dir, &index).unwrap();
        assert_eq!(item["name"], "飞书");
        assert_eq!(item["description"], "飞书办公套件");
        assert_eq!(item["kind"], "cli");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn agents_index_produces_chinese_entries() {
        let root = tmp_dir("agents-index");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("agents-index.json"),
            r#"{"experts":[{"id":"ContentCreator","plugin":"content-creator","expertType":"agent","displayName":{"zh":"文博凯","en":"Kai"},"profession":{"zh":"内容创作专家"},"description":{"zh":"擅长创作内容"},"categoryId":"06","tags":[{"zh":"内容策略"}],"quickPrompts":[{"zh":"帮我写内容"}],"defaultInitPrompt":{"zh":"默认"},"promptFile":"/plugins/content-creator/agents/content-creator.md","avatar":"/avatars/ContentCreator.png"}]}"#,
        )
        .unwrap();
        let items = agents_from_index(&root);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["display_name"], "文博凯");
        assert_eq!(items[0]["profession"], "内容创作专家");
        assert_eq!(items[0]["tags"][0], "内容策略");
        assert_eq!(items[0]["source"], "workbuddy");
        let _ = fs::remove_dir_all(&root);
    }
}
