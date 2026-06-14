//! Codex 桌面线程归一模块。
//!
//! 读取 Codex 本地 state_*.sqlite 中 threads 表，将 Codex Desktop 主线程
//! (`source = 'vscode'`) 以及历史 DEX 管理线程的 model_provider 归一到当前 Codex 配置里的 DEX provider。
//! 旧迁移备份、还原和校准入口仅保留兼容，不属于启动自动归一主路径。

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_ROLLOUT_MESSAGE_CHARS: usize = 24_000;
const MAX_ROLLOUT_TOTAL_CHARS: usize = 1_500_000;
const DESKTOP_RECENT_LOAD_WINDOW: usize = 20;
const MAX_ROLLOUT_TOKEN_USAGE_SCAN_FILES: usize = 400;
const DESKTOP_PROJECT_INDEX_REPAIR_ATTEMPTS: usize = 3;

/// 线程信息（只读）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInfo {
    pub id: String,
    pub title: String,
    pub model_provider: String,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub archived: bool,
}

/// 各 provider 的线程数量。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub count: usize,
}

/// 线程归一/还原前后的差异对比。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationDiff {
    pub before: Vec<ProviderSummary>,
    pub after: Vec<ProviderSummary>,
    pub target_provider: String,
    pub changed_count: usize,
    pub rollout_metadata_fixed_count: usize,
    pub remaining_non_unified_count: usize,
    pub visibility_fixed_count: usize,
    pub desktop_project_fixed_count: usize,
    pub desktop_recent_fixed_count: usize,
    pub desktop_project_pending_count: usize,
    pub desktop_recent_pending_count: usize,
    pub desktop_project_repair_blocked: bool,
    pub desktop_recent_repair_blocked: bool,
    pub codex_desktop_running: bool,
    pub cwd_aligned_count: usize,
}

/// 是否已有旧迁移备份。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadStatus {
    pub summary: Vec<ProviderSummary>,
    pub total: usize,
    pub managed_provider: String,
    pub migrated: bool,
    pub codex_desktop_running: bool,
    pub non_deecodex_count: usize,
    pub provider_unified_count: usize,
    pub codex_visible_count: usize,
    pub missing_preview_count: usize,
    pub missing_user_event_count: usize,
    pub current_cwd_visible_count: usize,
    pub desktop_project_indexed_count: usize,
    pub desktop_project_pending_count: usize,
    pub desktop_project_repair_blocked: bool,
    pub desktop_recent_visible_count: usize,
    pub desktop_recent_pending_count: usize,
    pub desktop_recent_repair_blocked: bool,
    pub source_summary: Vec<ThreadSourceSummary>,
    pub context_window: CodexContextWindowStatus,
}

/// Codex 首页来源分布。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSourceSummary {
    pub source: String,
    pub count: usize,
}

/// Codex Desktop 上下文窗口诊断。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexContextWindowStatus {
    pub active_model: Option<String>,
    pub configured_model_context_window: Option<i64>,
    pub catalog_model_context_window: Option<i64>,
    pub effective_model_context_window: Option<i64>,
    pub latest_rollout_model_context_window: Option<i64>,
    pub latest_rollout_last_total_tokens: Option<i64>,
    pub latest_rollout_token_usage_found: bool,
    pub latest_rollout_path: Option<String>,
}

#[derive(Debug, Clone)]
struct ThreadVisibilityStatus {
    provider_unified_count: usize,
    codex_visible_count: usize,
    missing_preview_count: usize,
    missing_user_event_count: usize,
    current_cwd_visible_count: usize,
    desktop_project_indexed_count: usize,
    desktop_project_pending_count: usize,
    desktop_project_repair_blocked: bool,
    desktop_recent_visible_count: usize,
    desktop_recent_pending_count: usize,
    desktop_recent_repair_blocked: bool,
    source_summary: Vec<ThreadSourceSummary>,
}

#[derive(Debug, Clone)]
struct RolloutTokenUsage {
    model_context_window: Option<i64>,
    last_total_tokens: Option<i64>,
    path: PathBuf,
}

fn current_thread_provider() -> String {
    crate::codex_config::active_managed_model_provider()
}

fn desktop_thread_filter_sql() -> &'static str {
    "source = 'vscode'"
}

/// 查找 Codex 的 state SQLite 数据库。
///
/// 依次搜索多个可能的 Codex 数据目录，优先找版本号最大的 `state_*.sqlite`（不含 -wal/-shm 后缀）。
fn find_state_db(home: &Path) -> Option<PathBuf> {
    // 可能的 Codex 数据目录列表（按优先级）
    #[allow(unused_mut)]
    let mut search_dirs: Vec<PathBuf> = vec![home.join(".codex")];

    // Windows：Codex Desktop 可能把数据放在不同位置
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            search_dirs.push(PathBuf::from(&appdata).join("Codex"));
            search_dirs.push(PathBuf::from(&appdata).join("codex"));
            search_dirs.push(PathBuf::from(&appdata).join("anthropic").join("Codex"));
        }
        if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
            search_dirs.push(PathBuf::from(&local_appdata).join("Codex"));
            search_dirs.push(PathBuf::from(&local_appdata).join("codex"));
            search_dirs.push(
                PathBuf::from(&local_appdata)
                    .join("anthropic")
                    .join("Codex"),
            );
        }
    }

    tracing::debug!(dirs = ?search_dirs, "搜索 Codex state 数据库");

    for codex_dir in &search_dirs {
        if !codex_dir.is_dir() {
            tracing::debug!(dir = %codex_dir.display(), "Codex 目录不存在，跳过");
            continue;
        }

        let entries = match std::fs::read_dir(codex_dir) {
            Ok(entries) => entries,
            Err(err) => {
                tracing::warn!(dir = %codex_dir.display(), "读取 Codex 目录失败，跳过: {err}");
                continue;
            }
        };
        let mut candidates: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("state_")
                    && name.ends_with(".sqlite")
                    && !name.ends_with("-wal")
                    && !name.ends_with("-shm")
                {
                    let path = e.path();
                    let version = state_db_version(&name)?;
                    let metadata = path.metadata().ok()?;
                    let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
                    let size = metadata.len();
                    Some((path, version, modified, size))
                } else {
                    None
                }
            })
            .collect();

        if !candidates.is_empty() {
            candidates.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| b.2.cmp(&a.2))
                    .then_with(|| b.3.cmp(&a.3))
            });
            let found = candidates.into_iter().next().map(|(p, _, _, _)| p);
            if let Some(ref db_path) = found {
                tracing::info!(db = %db_path.display(), "找到 Codex state 数据库");
            }
            return found;
        } else {
            tracing::debug!(dir = %codex_dir.display(), "目录下未找到 state_*.sqlite");
        }
    }

    tracing::warn!(home = %home.display(), "未找到 Codex state SQLite 数据库");
    None
}

fn state_db_version(name: &str) -> Option<u64> {
    name.strip_prefix("state_")?
        .strip_suffix(".sqlite")?
        .parse()
        .ok()
}

/// 备份文件路径（存在 deecodex data_dir 下）。
pub fn backup_path(data_dir: &Path) -> PathBuf {
    data_dir.join("thread_migration_backup.json")
}

/// 线程 cwd 可见性备份路径。
pub fn cwd_backup_path(data_dir: &Path) -> PathBuf {
    data_dir.join("thread_cwd_visibility_backup.json")
}

/// Codex Desktop recent 可见性时间戳备份路径。
pub fn desktop_recent_backup_path(data_dir: &Path) -> PathBuf {
    data_dir.join("thread_desktop_recent_backup.json")
}

/// 获取当前状态：各 provider 线程数、待归一 DEX 管理线程数、旧迁移备份状态。
pub fn status(data_dir: &Path) -> Result<ThreadStatus> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    let target_provider = current_thread_provider();

    let summary = get_provider_summary(&db_path)?;
    let total: usize = summary.iter().map(|s| s.count).sum();
    let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let non_deecodex_count = count_non_current_managed_threads(&conn, &target_provider)?;
    let visibility = get_visibility_status(&db_path)?;
    let migrated = backup_path(data_dir).exists();
    let context_window = get_context_window_status(&home);
    let codex_desktop_running = is_codex_desktop_running_for_db(&db_path);

    Ok(ThreadStatus {
        summary,
        total,
        managed_provider: target_provider,
        migrated,
        codex_desktop_running,
        non_deecodex_count,
        provider_unified_count: visibility.provider_unified_count,
        codex_visible_count: visibility.codex_visible_count,
        missing_preview_count: visibility.missing_preview_count,
        missing_user_event_count: visibility.missing_user_event_count,
        current_cwd_visible_count: visibility.current_cwd_visible_count,
        desktop_project_indexed_count: visibility.desktop_project_indexed_count,
        desktop_project_pending_count: visibility.desktop_project_pending_count,
        desktop_project_repair_blocked: visibility.desktop_project_repair_blocked,
        desktop_recent_visible_count: visibility.desktop_recent_visible_count,
        desktop_recent_pending_count: visibility.desktop_recent_pending_count,
        desktop_recent_repair_blocked: visibility.desktop_recent_repair_blocked,
        source_summary: visibility.source_summary,
        context_window,
    })
}

/// 列出所有线程（不过滤 provider）。
pub fn list_all() -> Result<Vec<ThreadInfo>> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    list_threads(&db_path)
}

/// 归一：将 Codex Desktop 主线程和历史 DEX 管理线程的 model_provider 改为当前 Codex 配置里的 provider。
pub fn migrate(data_dir: &Path) -> Result<MigrationDiff> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;

    do_normalize_desktop_threads(&db_path, &desktop_recent_backup_path(data_dir))
}

/// 打开 DEX 时静默执行的幂等归一：统一 Codex Desktop 主线程和历史 DEX 管理线程。
pub fn normalize_desktop_threads(data_dir: &Path) -> Result<MigrationDiff> {
    migrate(data_dir)
}

/// 还原：从备份恢复原始 model_provider 值。
/// 还原后自动删除备份文件。
pub fn restore(data_dir: &Path) -> Result<MigrationDiff> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    let bp = backup_path(data_dir);

    if !bp.exists() {
        anyhow::bail!("没有迁移备份，无需还原");
    }

    do_restore(
        &db_path,
        &bp,
        &cwd_backup_path(data_dir),
        &desktop_recent_backup_path(data_dir),
    )
}

/// 校准迁移备份：移除已删除的线程，追加新增的非当前 DEX provider 线程。
#[allow(dead_code)]
pub fn calibrate(data_dir: &Path) -> Result<MigrationDiff> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    do_calibrate(
        &db_path,
        &backup_path(data_dir),
        &cwd_backup_path(data_dir),
        &desktop_recent_backup_path(data_dir),
    )
}

/// 获取指定线程的完整内容（含元数据、摘要、工具）。
#[allow(dead_code)]
pub fn get_thread_content(thread_id: &str) -> Result<serde_json::Value> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    let conn = Connection::open(&db_path)?;

    // 1. 线程元数据
    let mut stmt = conn.prepare(
        "SELECT id, title, model_provider, model, reasoning_effort, rollout_path,
                first_user_message, created_at_ms, updated_at_ms,
                archived, cwd, git_sha, git_branch, agent_nickname, cli_version
         FROM threads WHERE id = ?1",
    )?;
    let (mut thread, rollout_path) = stmt.query_row(rusqlite::params![thread_id], |row| {
        let rollout_path = row.get::<_, String>(5)?;
        Ok((
            json!({
                "id": row.get::<_, String>(0)?,
                "title": row.get::<_, String>(1)?,
                "model_provider": row.get::<_, String>(2)?,
                "model": row.get::<_, Option<String>>(3)?,
                "reasoning_effort": row.get::<_, Option<String>>(4)?,
                "rollout_path": rollout_path.clone(),
                "first_user_message": row.get::<_, String>(6)?,
                "created_at_ms": row.get::<_, Option<i64>>(7)?,
                "updated_at_ms": row.get::<_, Option<i64>>(8)?,
                "archived": row.get::<_, i32>(9)? != 0,
                "cwd": row.get::<_, String>(10)?,
                "git_sha": row.get::<_, Option<String>>(11)?,
                "git_branch": row.get::<_, Option<String>>(12)?,
                "agent_nickname": row.get::<_, Option<String>>(13)?,
                "cli_version": row.get::<_, String>(14)?,
            }),
            rollout_path,
        ))
    })?;
    drop(stmt);

    let mut messages = read_rollout_messages(Path::new(&rollout_path)).unwrap_or_else(|err| {
        tracing::warn!(
            thread_id = thread_id,
            rollout_path = %rollout_path,
            "读取线程 rollout 内容失败: {err}"
        );
        Vec::new()
    });

    // 2. stage1_outputs 摘要
    let mut rollout_summary = None;
    if let Ok(mut stmt) = conn
        .prepare("SELECT rollout_summary, rollout_slug FROM stage1_outputs WHERE thread_id = ?1")
    {
        if let Ok((summary, slug)) = stmt.query_row(rusqlite::params![thread_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        }) {
            rollout_summary = Some(summary.clone());
            thread["rollout_summary"] = serde_json::Value::String(summary);
            if let Some(s) = slug {
                thread["rollout_slug"] = serde_json::Value::String(s);
            }
        }
    }

    if messages.is_empty() {
        // 兼容极旧或缺失 rollout 文件的线程，至少保留列表里能看到的首问和摘要。
        if let Some(first_msg) = thread.get("first_user_message").and_then(|v| v.as_str()) {
            if !first_msg.is_empty() {
                messages.push(json!({
                    "role": "user",
                    "payload": { "role": "user", "content": [{ "type": "input_text", "text": first_msg }] }
                }));
            }
        }
        if let Some(ref summary) = rollout_summary {
            if !summary.is_empty() {
                messages.push(json!({
                    "role": "assistant",
                    "payload": { "role": "assistant", "content": [{ "type": "output_text", "text": summary }] }
                }));
            }
        }
    }
    thread["message_count"] = json!(messages.len());

    // 3. 线程关联的工具
    if let Ok(mut stmt) = conn.prepare(
        "SELECT name, description FROM thread_dynamic_tools WHERE thread_id = ?1 ORDER BY position",
    ) {
        if let Ok(tools) = stmt
            .query_map(rusqlite::params![thread_id], |row| {
                Ok(json!({
                    "name": row.get::<_, String>(0)?,
                    "description": row.get::<_, String>(1)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            if !tools.is_empty() {
                thread["tools"] = serde_json::Value::Array(tools);
            }
        }
    }

    Ok(serde_json::json!({
        "thread": thread,
        "messages": messages
    }))
}

fn read_rollout_messages(path: &Path) -> Result<Vec<Value>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("打开 rollout 文件失败: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    let mut stopped_for_size = false;

    for line in reader.lines() {
        let line = line.context("读取 rollout 行失败")?;
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(path = %path.display(), "跳过无法解析的 rollout 行: {err}");
                continue;
            }
        };
        let message = match event.get("type").and_then(Value::as_str) {
            Some("response_item") => rollout_response_item_to_message(event.get("payload")),
            Some("compacted") => event
                .get("payload")
                .and_then(|p| p.get("message"))
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(|text| text_message("system", format!("上下文摘要\n\n{text}"))),
            _ => None,
        };

        if let Some(message) = message {
            let message_chars = value_text_len(&message);
            if total_chars.saturating_add(message_chars) > MAX_ROLLOUT_TOTAL_CHARS {
                stopped_for_size = true;
                break;
            }
            total_chars += message_chars;
            messages.push(message);
        }
    }

    if stopped_for_size {
        messages.push(text_message(
            "system",
            "线程内容过大，后续内容已停止加载。可继续收窄查看范围或导出原始 rollout 文件。"
                .to_string(),
        ));
    }

    Ok(messages)
}

fn rollout_response_item_to_message(payload: Option<&Value>) -> Option<Value> {
    let payload = payload?;
    let item_type = payload.get("type").and_then(Value::as_str)?;
    match item_type {
        "message" => {
            let role = payload
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("system");
            let text = content_to_text(payload.get("content"));
            if text.trim().is_empty() {
                return None;
            }
            Some(text_message(role, text))
        }
        "reasoning" => {
            let mut text = content_to_text(payload.get("summary"));
            if text.trim().is_empty() {
                text = content_to_text(payload.get("content"));
            }
            if text.trim().is_empty() {
                return None;
            }
            Some(text_message("assistant", format!("推理摘要\n\n{text}")))
        }
        "function_call" | "custom_tool_call" | "tool_search_call" | "web_search_call" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(item_type);
            let arguments = payload
                .get("arguments")
                .and_then(Value::as_str)
                .map(truncate_rollout_text)
                .or_else(|| payload.get("input").map(value_to_display_string))
                .or_else(|| payload.get("query").map(value_to_display_string))
                .unwrap_or_else(|| value_to_display_string(payload));
            let text = if arguments.trim().is_empty() {
                format!("调用工具: {name}")
            } else {
                format!("调用工具: {name}\n{arguments}")
            };
            Some(text_message("tool", text))
        }
        "function_call_output" | "custom_tool_call_output" | "tool_search_output" => {
            let output = payload
                .get("output")
                .or_else(|| payload.get("result"))
                .or_else(|| payload.get("content"))
                .map(value_to_display_string)
                .unwrap_or_default();
            if output.trim().is_empty() {
                return None;
            }
            Some(text_message("tool", output))
        }
        _ => None,
    }
}

fn text_message(role: &str, text: String) -> Value {
    json!({
        "role": role,
        "payload": {
            "role": role,
            "content": [{ "type": "text", "text": truncate_rollout_text(&text) }]
        }
    })
}

fn content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => truncate_rollout_text(text),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(content_item_to_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) if !value.is_null() => value_to_display_string(value),
        _ => String::new(),
    }
}

fn content_item_to_text(item: &Value) -> Option<String> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    match item_type {
        "input_text" | "output_text" | "text" => item
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| item.get("content").and_then(Value::as_str))
            .map(truncate_rollout_text),
        "input_image" | "image_url" => Some(describe_image_item(item)),
        "input_file" => Some(describe_file_item(item)),
        _ => {
            if item.get("image_url").is_some() {
                Some(describe_image_item(item))
            } else if item.get("file_id").is_some() || item.get("filename").is_some() {
                Some(describe_file_item(item))
            } else {
                item.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("content").and_then(Value::as_str))
                    .map(truncate_rollout_text)
                    .or_else(|| Some(value_to_display_string(item)).filter(|text| !text.is_empty()))
            }
        }
    }
}

fn describe_image_item(item: &Value) -> String {
    let image_url = item
        .get("image_url")
        .and_then(Value::as_str)
        .or_else(|| item.get("url").and_then(Value::as_str))
        .unwrap_or_default();
    if image_url.starts_with("data:image/") {
        let mime = image_url
            .split(';')
            .next()
            .unwrap_or("data:image")
            .trim_start_matches("data:");
        format!(
            "[图片内容: {mime}，{} 字节 data URL，已省略原始数据]",
            image_url.len()
        )
    } else if image_url.is_empty() {
        "[图片内容已省略]".to_string()
    } else {
        format!(
            "[图片内容: {}]",
            truncate_text(image_url, MAX_ROLLOUT_MESSAGE_CHARS.min(240))
        )
    }
}

fn describe_file_item(item: &Value) -> String {
    let name = item
        .get("filename")
        .and_then(Value::as_str)
        .or_else(|| item.get("file_id").and_then(Value::as_str))
        .unwrap_or("未知文件");
    format!("[文件内容: {}]", truncate_text(name, 240))
}

fn value_to_display_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => truncate_rollout_text(text),
        Value::Array(items) => {
            let lines = items
                .iter()
                .filter_map(content_item_to_text)
                .collect::<Vec<_>>();
            if lines.is_empty() {
                truncate_rollout_text(&pretty_json(value))
            } else {
                truncate_rollout_text(&lines.join("\n"))
            }
        }
        Value::Object(_) => {
            if value.get("image_url").is_some() {
                describe_image_item(value)
            } else if value.get("file_id").is_some() || value.get("filename").is_some() {
                describe_file_item(value)
            } else {
                truncate_rollout_text(&pretty_json(value))
            }
        }
        _ => truncate_rollout_text(&value.to_string()),
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn truncate_rollout_text(text: &str) -> String {
    truncate_text(text, MAX_ROLLOUT_MESSAGE_CHARS)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut end = text.len();
    for (count, (idx, _)) in text.char_indices().enumerate() {
        if count >= max_chars {
            end = idx;
            break;
        }
    }
    if end == text.len() {
        return text.to_string();
    }

    let mut out = text[..end].to_string();
    out.push_str(&format!(
        "\n\n[内容过长，已截断，原始长度约 {} 字节]",
        text.len()
    ));
    out
}

fn value_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(value_text_len).sum(),
        Value::Object(map) => map.values().map(value_text_len).sum(),
        _ => 0,
    }
}

/// 永久删除指定线程。
#[allow(dead_code)]
pub fn delete_thread(data_dir: &Path, thread_id: &str) -> Result<()> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    delete_thread_from_db(data_dir, &home, &db_path, thread_id)
}

fn delete_thread_from_db(
    data_dir: &Path,
    home: &Path,
    db_path: &Path,
    thread_id: &str,
) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "busy_timeout", "5000")?;

    let rollout_path: Option<String> = conn
        .query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
            |row| row.get(0),
        )
        .optional()
        .context("读取线程 rollout 路径失败")?;

    let Some(rollout_path) = rollout_path else {
        anyhow::bail!("未找到线程 {thread_id}");
    };

    let tx = conn.unchecked_transaction()?;
    delete_if_table_exists(&tx, "stage1_outputs", "thread_id", thread_id)?;
    delete_if_table_exists(&tx, "thread_dynamic_tools", "thread_id", thread_id)?;
    let affected = tx
        .execute(
            "DELETE FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
        )
        .context("删除线程失败")?;
    tx.commit().context("提交线程删除失败")?;

    // 同时从迁移备份中移除
    remove_thread_from_migration_backup(&backup_path(data_dir), thread_id)?;
    if let Err(err) = remove_deleted_thread_from_desktop_state(db_path, thread_id) {
        tracing::warn!("清理 Codex Desktop 线程索引失败: {err}");
    }
    remove_thread_rollout_file(home, &rollout_path)?;

    tracing::info!(affected, "已永久删除线程 {thread_id}");
    Ok(())
}

fn delete_if_table_exists(
    conn: &Connection,
    table: &str,
    id_column: &str,
    thread_id: &str,
) -> Result<usize> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            rusqlite::params![table],
            |row| row.get(0),
        )
        .with_context(|| format!("检查线程关联表 {table} 失败"))?;
    if !exists {
        return Ok(0);
    }
    let sql = format!("DELETE FROM {table} WHERE {id_column} = ?1");
    conn.execute(&sql, rusqlite::params![thread_id])
        .with_context(|| format!("删除线程关联表 {table} 记录失败"))
}

fn remove_thread_from_migration_backup(path: &Path, thread_id: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let Ok(json) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(mut originals) = serde_json::from_str::<Vec<(String, String)>>(&json) else {
        return Ok(());
    };
    let before = originals.len();
    originals.retain(|(id, _)| id != thread_id);
    if originals.len() == before {
        return Ok(());
    }
    let new_json = serde_json::to_string_pretty(&originals).context("序列化备份失败")?;
    std::fs::write(path, new_json).context("写入备份文件失败")
}

fn remove_thread_rollout_file(home: &Path, rollout_path: &str) -> Result<()> {
    let path = PathBuf::from(rollout_path);
    if !path.exists() {
        return Ok(());
    }
    if !is_safe_codex_rollout_path(home, &path) {
        tracing::warn!(
            thread_rollout_path = %path.display(),
            "跳过删除不在 Codex sessions 目录内的 rollout 文件"
        );
        return Ok(());
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("删除线程 rollout 文件失败: {}", path.display()))
}

fn is_safe_codex_rollout_path(home: &Path, path: &Path) -> bool {
    let sessions_dir = home.join(".codex").join("sessions");
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(sessions_dir) = sessions_dir.canonicalize() else {
        return false;
    };
    path.starts_with(sessions_dir)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
}

// ── 内部函数 ──

fn list_threads(db_path: &Path) -> Result<Vec<ThreadInfo>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT id, title, model_provider, created_at_ms, updated_at_ms, archived
         FROM threads
         ORDER BY COALESCE(updated_at_ms, updated_at) DESC",
    )?;
    let threads = stmt
        .query_map([], |row| {
            Ok(ThreadInfo {
                id: row.get(0)?,
                title: row.get(1)?,
                model_provider: row.get(2)?,
                created_at_ms: row.get(3)?,
                updated_at_ms: row.get(4)?,
                archived: row.get::<_, i32>(5)? != 0,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(threads)
}

fn get_provider_summary(db_path: &Path) -> Result<Vec<ProviderSummary>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(TRIM(model_provider), ''), '(空)'), COUNT(*)
         FROM threads
         GROUP BY model_provider
         ORDER BY COUNT(*) DESC",
    )?;
    let summary = stmt
        .query_map([], |row| {
            Ok(ProviderSummary {
                provider: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(summary)
}

fn get_visibility_status(db_path: &Path) -> Result<ThreadVisibilityStatus> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let provider = current_thread_provider();
    let provider_unified_count = query_count(
        &conn,
        "SELECT COUNT(*) FROM threads WHERE model_provider = ?1",
        rusqlite::params![provider],
    )?;
    let codex_visible_count = query_count(
        &conn,
        "SELECT COUNT(*) FROM threads
         WHERE archived = 0 AND model_provider = ?1 AND TRIM(preview) <> ''",
        rusqlite::params![provider],
    )?;
    let missing_preview_count = query_count(
        &conn,
        "SELECT COUNT(*) FROM threads
         WHERE archived = 0 AND model_provider = ?1 AND TRIM(preview) = ''",
        rusqlite::params![provider],
    )?;
    let missing_user_event_count = query_count(
        &conn,
        "SELECT COUNT(*) FROM threads
         WHERE model_provider = ?1
           AND archived = 0
           AND has_user_event = 0
           AND (TRIM(first_user_message) <> '' OR thread_source = 'user')",
        rusqlite::params![provider],
    )?;
    let current_cwd = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();
    let current_cwd_visible_count = if current_cwd.is_empty() {
        0
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM threads
             WHERE archived = 0
               AND model_provider = ?1
               AND TRIM(preview) <> ''
               AND cwd = ?2",
            rusqlite::params![provider, current_cwd],
            |row| row.get::<_, usize>(0),
        )?
    };

    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(TRIM(source), ''), '(空)'), COUNT(*)
         FROM threads
         WHERE archived = 0 AND model_provider = ?1 AND TRIM(preview) <> ''
         GROUP BY source
         ORDER BY COUNT(*) DESC",
    )?;
    let source_summary = stmt
        .query_map(rusqlite::params![provider], |row| {
            Ok(ThreadSourceSummary {
                source: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let desktop_project_status = get_desktop_project_index_status(db_path, &conn)?;

    Ok(ThreadVisibilityStatus {
        provider_unified_count,
        codex_visible_count,
        missing_preview_count,
        missing_user_event_count,
        current_cwd_visible_count,
        desktop_project_indexed_count: desktop_project_status.indexed_count,
        desktop_project_pending_count: desktop_project_status.pending_count,
        desktop_project_repair_blocked: desktop_project_status.repair_blocked,
        desktop_recent_visible_count: desktop_project_status.recent_visible_count,
        desktop_recent_pending_count: desktop_project_status.recent_pending_count,
        desktop_recent_repair_blocked: desktop_project_status.recent_repair_blocked,
        source_summary,
    })
}

fn query_count<P>(conn: &Connection, sql: &str, params: P) -> Result<usize>
where
    P: rusqlite::Params,
{
    conn.query_row(sql, params, |row| row.get::<_, usize>(0))
        .with_context(|| format!("执行统计失败: {sql}"))
}

fn get_context_window_status(home: &Path) -> CodexContextWindowStatus {
    let mut status = match read_codex_context_window_status(home) {
        Ok(status) => status,
        Err(err) => {
            tracing::warn!("读取 Codex 上下文窗口配置失败: {err}");
            CodexContextWindowStatus::default()
        }
    };

    match find_latest_rollout_token_usage(home) {
        Ok(Some(usage)) => {
            status.latest_rollout_model_context_window = usage.model_context_window;
            status.latest_rollout_last_total_tokens = usage.last_total_tokens;
            status.latest_rollout_token_usage_found = true;
            status.latest_rollout_path = Some(usage.path.to_string_lossy().to_string());
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!("读取 Codex rollout token_count 失败: {err}");
        }
    }

    status
}

fn read_codex_context_window_status(home: &Path) -> Result<CodexContextWindowStatus> {
    let config_path = home.join(".codex").join("config.toml");
    if !config_path.exists() {
        return Ok(CodexContextWindowStatus::default());
    }

    let content = crate::codex_config::read_config_file(&config_path)
        .with_context(|| format!("读取 Codex config.toml 失败: {}", config_path.display()))?;
    let doc: toml_edit::DocumentMut = content
        .parse()
        .with_context(|| format!("解析 Codex config.toml 失败: {}", config_path.display()))?;

    let active_model = doc
        .get("model")
        .and_then(|model| model.as_str())
        .map(ToString::to_string);
    let configured_model_context_window =
        toml_item_i64(doc.get("model_context_window")).filter(|value| *value > 0);
    let catalog_path = doc
        .get("model_catalog_json")
        .and_then(|path| path.as_str())
        .map(|path| expand_codex_path(home, path))
        .or_else(|| {
            let fallback = home.join(".codex").join("models_deecodex.json");
            fallback.exists().then_some(fallback)
        });

    let (catalog_model_context_window, effective_model_context_window) =
        if let Some(catalog_path) = catalog_path {
            read_catalog_context_window(
                &catalog_path,
                active_model.as_deref(),
                configured_model_context_window,
            )
            .unwrap_or_else(|err| {
                tracing::warn!(
                    path = %catalog_path.display(),
                    "读取 Codex 模型目录上下文窗口失败: {err}"
                );
                (None, configured_model_context_window)
            })
        } else {
            (None, configured_model_context_window)
        };

    Ok(CodexContextWindowStatus {
        active_model,
        configured_model_context_window,
        catalog_model_context_window,
        effective_model_context_window,
        latest_rollout_model_context_window: None,
        latest_rollout_last_total_tokens: None,
        latest_rollout_token_usage_found: false,
        latest_rollout_path: None,
    })
}

fn expand_codex_path(home: &Path, raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        return home.join(rest);
    }
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        home.join(".codex").join(path)
    }
}

fn toml_item_i64(item: Option<&toml_edit::Item>) -> Option<i64> {
    item.and_then(|item| item.as_value())
        .and_then(|value| value.as_integer())
}

fn read_catalog_context_window(
    catalog_path: &Path,
    active_model: Option<&str>,
    configured_model_context_window: Option<i64>,
) -> Result<(Option<i64>, Option<i64>)> {
    let raw = std::fs::read_to_string(catalog_path)
        .with_context(|| format!("读取模型目录失败: {}", catalog_path.display()))?;
    let catalog: Value = serde_json::from_str(&raw)
        .with_context(|| format!("解析模型目录失败: {}", catalog_path.display()))?;
    let Some(models) = catalog.get("models").and_then(Value::as_array) else {
        return Ok((None, configured_model_context_window));
    };
    let model = active_model
        .and_then(|active| {
            models.iter().find(|model| {
                ["slug", "id", "name", "model"]
                    .iter()
                    .any(|key| model.get(*key).and_then(Value::as_str) == Some(active))
            })
        })
        .or_else(|| models.first());

    let Some(model) = model else {
        return Ok((None, configured_model_context_window));
    };

    let catalog_model_context_window = json_i64(model.get("context_window"))
        .or_else(|| json_i64(model.get("max_context_window")))
        .filter(|value| *value > 0);
    let percent = json_i64(model.get("effective_context_window_percent"))
        .filter(|value| *value > 0)
        .unwrap_or(100);
    let base = configured_model_context_window.or(catalog_model_context_window);
    let effective_model_context_window = base.map(|value| value.saturating_mul(percent) / 100);

    Ok((catalog_model_context_window, effective_model_context_window))
}

fn json_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_u64()
                .map(|value| value.min(i64::MAX as u64) as i64)
        }),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn json_i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    json_i64(Some(current))
}

fn find_latest_rollout_token_usage(home: &Path) -> Result<Option<RolloutTokenUsage>> {
    let sessions_dir = home.join(".codex").join("sessions");
    if !sessions_dir.is_dir() {
        return Ok(None);
    }

    let mut files = Vec::new();
    collect_rollout_files(&sessions_dir, &mut files);
    files.sort_by_key(|item| std::cmp::Reverse(item.1));

    for (path, _) in files.into_iter().take(MAX_ROLLOUT_TOKEN_USAGE_SCAN_FILES) {
        if let Some(usage) = read_latest_rollout_token_usage(&path)? {
            return Ok(Some(usage));
        }
    }

    Ok(None)
}

fn collect_rollout_files(dir: &Path, files: &mut Vec<(PathBuf, SystemTime)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(dir = %dir.display(), "读取 Codex sessions 目录失败: {err}");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_dir() {
            collect_rollout_files(&path, files);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("rollout-") && name.ends_with(".jsonl") {
            files.push((path, metadata.modified().unwrap_or(UNIX_EPOCH)));
        }
    }
}

fn read_latest_rollout_token_usage(path: &Path) -> Result<Option<RolloutTokenUsage>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("打开 rollout 文件失败: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut latest = None;

    for line in reader.lines() {
        let line = line.context("读取 rollout 行失败")?;
        if !line.contains("\"token_count\"") {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(info) = token_count_info(&event) else {
            continue;
        };
        latest = Some(RolloutTokenUsage {
            model_context_window: json_i64(info.get("model_context_window")),
            last_total_tokens: json_i64_at(info, &["last_token_usage", "total_tokens"])
                .or_else(|| json_i64_at(info, &["last", "totalTokens"]))
                .or_else(|| json_i64_at(info, &["last", "total_tokens"])),
            path: path.to_path_buf(),
        });
    }

    Ok(latest)
}

fn token_count_info(event: &Value) -> Option<&Value> {
    let payload = event.get("payload").unwrap_or(event);
    let is_token_count = event.get("type").and_then(Value::as_str) == Some("token_count")
        || payload.get("type").and_then(Value::as_str) == Some("token_count");
    if !is_token_count {
        return None;
    }
    payload
        .get("info")
        .or_else(|| event.get("info"))
        .or(Some(payload))
}

fn repair_thread_visibility(conn: &Connection) -> Result<usize> {
    let provider = current_thread_provider();
    let columns = thread_table_columns(conn)?;
    let has_thread_source = columns.contains("thread_source");
    let mut fixed = 0usize;
    fixed += conn
        .execute(
            "UPDATE threads
             SET preview = first_user_message
             WHERE model_provider = ?1
               AND archived = 0
               AND TRIM(preview) = ''
               AND TRIM(first_user_message) <> ''",
            rusqlite::params![provider],
        )
        .context("补齐 Codex 线程 preview(first_user_message) 失败")?;
    fixed += conn
        .execute(
            "UPDATE threads
             SET preview = title
             WHERE model_provider = ?1
               AND archived = 0
               AND TRIM(preview) = ''
               AND TRIM(title) <> ''",
            rusqlite::params![provider],
        )
        .context("补齐 Codex 线程 preview(title) 失败")?;
    if has_thread_source {
        fixed += conn
            .execute(
                "UPDATE threads
                 SET has_user_event = 1
                 WHERE model_provider = ?1
                   AND archived = 0
                   AND has_user_event = 0
                   AND (TRIM(first_user_message) <> '' OR thread_source = 'user')",
                rusqlite::params![provider],
            )
            .context("补齐 Codex 线程 has_user_event 失败")?;
    } else {
        fixed += conn
            .execute(
                "UPDATE threads
                 SET has_user_event = 1
                 WHERE model_provider = ?1
                   AND archived = 0
                   AND has_user_event = 0
                   AND TRIM(first_user_message) <> ''",
                rusqlite::params![provider],
            )
            .context("补齐 Codex 线程 has_user_event 失败")?;
    }

    if fixed > 0 {
        tracing::info!("已修复 {fixed} 个 Codex 线程首页可见性字段");
    }
    Ok(fixed)
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn repair_desktop_project_metadata(conn: &Connection) -> Result<usize> {
    let provider = current_thread_provider();
    let cwd_rows: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT cwd
             FROM threads
             WHERE model_provider = ?1
               AND archived = 0
               AND TRIM(preview) <> ''
               AND TRIM(cwd) <> ''",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![provider], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };

    let mut fixed = 0usize;
    for cwd in cwd_rows {
        let cwd_path = Path::new(&cwd);
        if !cwd_path.is_dir() {
            continue;
        }

        let Some(git_sha) = git_output(cwd_path, &["rev-parse", "HEAD"]) else {
            continue;
        };
        let git_branch = git_output(cwd_path, &["branch", "--show-current"]);
        let git_origin_url = git_output(cwd_path, &["config", "--get", "remote.origin.url"]);

        let changed = conn
            .execute(
                "UPDATE threads
                 SET git_sha = ?1,
                     git_branch = COALESCE(?2, git_branch),
                     git_origin_url = COALESCE(?3, git_origin_url)
                 WHERE model_provider = ?4
                   AND archived = 0
                   AND TRIM(preview) <> ''
                   AND cwd = ?5
                   AND (
                     COALESCE(git_sha, '') != ?1
                     OR (?2 IS NOT NULL AND COALESCE(git_branch, '') != ?2)
                     OR (?3 IS NOT NULL AND COALESCE(git_origin_url, '') != ?3)
                   )",
                rusqlite::params![git_sha, git_branch, git_origin_url, provider, cwd],
            )
            .with_context(|| format!("修复 Codex Desktop 项目元数据失败: {cwd}"))?;
        fixed += changed;
    }

    if fixed > 0 {
        tracing::info!("已修复 {fixed} 个 Codex Desktop 项目会话元数据");
    }
    Ok(fixed)
}

#[derive(Debug, Clone)]
struct DesktopThreadRow {
    id: String,
    cwd: String,
}

#[derive(Debug, Clone, Copy, Default)]
struct DesktopProjectIndexStatus {
    indexed_count: usize,
    pending_count: usize,
    repair_blocked: bool,
    recent_visible_count: usize,
    recent_pending_count: usize,
    recent_repair_blocked: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct DesktopProjectRepairResult {
    fixed_count: usize,
    pending_count: usize,
    blocked: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct DesktopRecentRepairResult {
    fixed_count: usize,
    pending_count: usize,
    blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopRecentTimestampBackup {
    id: String,
    updated_at: i64,
    updated_at_ms: Option<i64>,
}

fn codex_global_state_path(db_path: &Path) -> Option<PathBuf> {
    db_path
        .parent()
        .map(|codex_home| codex_home.join(".codex-global-state.json"))
}

fn value_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn value_string_object(value: Option<&Value>) -> Map<String, Value> {
    value
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn append_unique(items: &mut Vec<String>, value: &str) -> bool {
    if items.iter().any(|item| item == value) {
        return false;
    }
    items.push(value.to_string());
    true
}

fn remove_value(items: &mut Vec<String>, value: &str) -> bool {
    let before = items.len();
    items.retain(|item| item != value);
    before != items.len()
}

fn remove_deleted_thread_from_desktop_state(db_path: &Path, thread_id: &str) -> Result<usize> {
    let Some(global_path) = codex_global_state_path(db_path) else {
        return Ok(0);
    };
    if !global_path.exists() {
        return Ok(0);
    }

    let raw = std::fs::read_to_string(&global_path).context("读取 Codex Desktop 全局状态失败")?;
    let mut state: Value = serde_json::from_str(&raw).context("解析 Codex Desktop 全局状态失败")?;
    let Some(object) = state.as_object_mut() else {
        return Ok(0);
    };

    let mut changed = 0usize;
    for key in [
        "thread-project-assignments",
        "thread-workspace-root-hints",
        "thread-projectless-output-directories",
    ] {
        if let Some(map) = object.get_mut(key).and_then(Value::as_object_mut) {
            if map.remove(thread_id).is_some() {
                changed += 1;
            }
        }
    }

    if let Some(items) = object
        .get_mut("projectless-thread-ids")
        .and_then(Value::as_array_mut)
    {
        let before = items.len();
        items.retain(|item| item.as_str() != Some(thread_id));
        changed += before.saturating_sub(items.len());
    }

    if let Some(orders) = object
        .get_mut("sidebar-project-thread-orders")
        .and_then(Value::as_object_mut)
    {
        let mut empty_projects = Vec::new();
        for (project, order) in orders.iter_mut() {
            let Some(ids) = order.get_mut("threadIds").and_then(Value::as_array_mut) else {
                continue;
            };
            let before = ids.len();
            ids.retain(|item| item.as_str() != Some(thread_id));
            changed += before.saturating_sub(ids.len());
            if ids.is_empty() {
                empty_projects.push(project.clone());
            }
        }
        for project in empty_projects {
            if orders.remove(&project).is_some() {
                changed += 1;
            }
        }
    }

    if changed == 0 {
        return Ok(0);
    }

    let backup_path = global_path.with_file_name(format!(
        ".codex-global-state.json.deecodex-delete-thread-{}.bak",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0)
    ));
    std::fs::write(&backup_path, raw).context("备份 Codex Desktop 全局状态失败")?;
    write_json_atomically(&global_path, &state).context("写入 Codex Desktop 全局状态失败")?;
    Ok(changed)
}

fn file_signature(path: &Path) -> Result<(u64, Option<SystemTime>)> {
    let meta =
        std::fs::metadata(path).with_context(|| format!("读取文件状态失败: {}", path.display()))?;
    Ok((meta.len(), meta.modified().ok()))
}

fn write_raw_atomically(path: &Path, raw: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_path = path.with_file_name(format!(
        ".{file_name}.deecodex-tmp-{}-{suffix}",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        std::fs::write(&tmp_path, raw).context("写入 Codex Desktop 临时状态失败")?;
        std::fs::rename(&tmp_path, path).context("替换 Codex Desktop 全局状态失败")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn write_json_atomically(path: &Path, value: &Value) -> Result<()> {
    let raw = serde_json::to_string(value).context("序列化 Codex Desktop 全局状态失败")? + "\n";
    write_raw_atomically(path, &raw)?;
    if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
        let backup_path = path.with_file_name(format!("{file_name}.bak"));
        write_raw_atomically(&backup_path, &raw)?;
    }
    Ok(())
}

fn path_starts_with(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let path = path.replace('\\', "/");
    let root = root.trim_end_matches('/').replace('\\', "/");
    path == root || path.starts_with(&format!("{root}/"))
}

fn is_codex_worktree_path(path: &str) -> bool {
    let marker = "/.codex/worktrees/";
    path.contains(marker) && path.ends_with("/deecodex/deecodex-gui")
}

fn project_label(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(path)
        .to_string()
}

fn project_for_thread(cwd: &str, known_roots: &BTreeSet<String>) -> Option<String> {
    let mut best: Option<&String> = None;
    for root in known_roots {
        if path_starts_with(cwd, root) && best.is_none_or(|current| root.len() > current.len()) {
            best = Some(root);
        }
    }
    best.cloned().or_else(|| {
        if Path::new(cwd).is_dir() {
            Some(cwd.to_string())
        } else {
            None
        }
    })
}

fn read_desktop_thread_rows(conn: &Connection) -> Result<Vec<DesktopThreadRow>> {
    let provider = current_thread_provider();
    let mut column_stmt = conn.prepare("PRAGMA table_info(threads)")?;
    let columns: HashSet<String> = column_stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    let order_columns = ["updated_at_ms", "updated_at", "created_at_ms", "created_at"]
        .into_iter()
        .filter(|column| columns.contains(*column))
        .map(|column| format!("COALESCE({column}, 0) DESC"))
        .collect::<Vec<_>>();
    let order_by = if order_columns.is_empty() {
        "rowid DESC".to_string()
    } else {
        order_columns.join(", ")
    };
    let sql = format!(
        "SELECT id, cwd
         FROM threads
         WHERE model_provider = ?1
           AND source = 'vscode'
           AND archived = 0
           AND TRIM(preview) <> ''
           AND TRIM(cwd) <> ''
         ORDER BY {order_by}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params![provider], |row| {
            Ok(DesktopThreadRow {
                id: row.get(0)?,
                cwd: row.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn desktop_recent_thread_ids(conn: &Connection, limit: usize) -> Result<HashSet<String>> {
    let mut column_stmt = conn.prepare("PRAGMA table_info(threads)")?;
    let columns: HashSet<String> = column_stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    let order_columns = ["updated_at", "created_at", "updated_at_ms", "created_at_ms"]
        .into_iter()
        .filter(|column| columns.contains(*column))
        .map(|column| format!("COALESCE({column}, 0) DESC"))
        .collect::<Vec<_>>();
    let order_by = if order_columns.is_empty() {
        "rowid DESC".to_string()
    } else {
        order_columns.join(", ")
    };
    let sql = format!(
        "SELECT id
         FROM threads
         WHERE archived = 0
           AND source = 'vscode'
           AND TRIM(preview) <> ''
         ORDER BY {order_by}
         LIMIT ?1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(ids)
}

fn desktop_recent_repair_thread_ids(
    project_threads: &BTreeMap<String, Vec<String>>,
) -> HashSet<String> {
    project_threads
        .iter()
        .filter(|(project, _)| is_codex_worktree_path(project))
        .flat_map(|(_, ids)| ids.iter().cloned())
        .collect()
}

fn desktop_recent_repair_order(project_threads: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for (project, thread_ids) in project_threads {
        if !is_codex_worktree_path(project) {
            continue;
        }
        for id in thread_ids {
            if seen.insert(id.clone()) {
                ids.push(id.clone());
            }
        }
    }
    ids
}

fn is_live_codex_desktop_state(global_path: &Path) -> bool {
    let Some(home) = crate::config::home_dir() else {
        return false;
    };
    let expected = home.join(".codex/.codex-global-state.json");
    if global_path != expected {
        return false;
    }

    let output = Command::new("pgrep")
        .arg("-f")
        .arg(
            r"/Applications/Codex\.app/Contents/(MacOS/Codex($| )|Frameworks/.*/Helpers/Codex \((Service|Renderer)\)\.app/Contents/MacOS/Codex \((Service|Renderer)\)|Resources/codex app-server($| ))",
        )
        .output();
    output
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

fn is_codex_desktop_running_for_db(db_path: &Path) -> bool {
    codex_global_state_path(db_path)
        .as_deref()
        .is_some_and(is_live_codex_desktop_state)
}

fn count_non_unified_provider(conn: &Connection, target_provider: &str) -> Result<usize> {
    query_count(
        conn,
        "SELECT COUNT(*) FROM threads WHERE model_provider != ?1",
        rusqlite::params![target_provider],
    )
}

fn managed_provider_filter_sql() -> &'static str {
    "TRIM(model_provider) IN ('deecodex', 'deecodex_cli', 'deecodex_desktop', 'dex_router')"
}

fn count_non_current_managed_threads(conn: &Connection, target_provider: &str) -> Result<usize> {
    let sql = format!(
        "SELECT COUNT(*) FROM threads
         WHERE model_provider != ?1
           AND {}",
        managed_provider_filter_sql()
    );
    query_count(conn, &sql, rusqlite::params![target_provider])
}

fn normalize_desktop_thread_providers(conn: &Connection, target_provider: &str) -> Result<usize> {
    let sql = format!(
        "UPDATE threads SET model_provider = ?1 WHERE model_provider != ?1 AND {}",
        desktop_thread_filter_sql()
    );
    conn.execute(&sql, rusqlite::params![target_provider])
        .context("归一 Codex Desktop 线程 provider 失败")
}

fn normalize_managed_thread_providers(conn: &Connection, target_provider: &str) -> Result<usize> {
    let sql = format!(
        "UPDATE threads
         SET model_provider = ?1
         WHERE model_provider != ?1
           AND {}",
        managed_provider_filter_sql()
    );
    conn.execute(&sql, rusqlite::params![target_provider])
        .context("归一历史 DEX 管理线程 provider 失败")
}

fn normalize_desktop_rollout_metadata(
    db_path: &Path,
    conn: &Connection,
    target_provider: &str,
) -> Result<usize> {
    let columns = thread_table_columns(conn)?;
    if !columns.contains("rollout_path") {
        return Ok(0);
    }

    let Some(home) = state_db_home(db_path) else {
        tracing::warn!(
            db = %db_path.display(),
            "无法确定 Codex HOME，跳过 rollout 元数据归一"
        );
        return Ok(0);
    };

    let mut stmt = conn.prepare(
        "SELECT id, rollout_path
         FROM threads
         WHERE source = 'vscode'
           AND TRIM(COALESCE(rollout_path, '')) <> ''",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut fixed = 0usize;
    for (thread_id, rollout_path) in rows {
        let path = PathBuf::from(&rollout_path);
        match normalize_rollout_session_meta_provider(&home, &thread_id, &path, target_provider) {
            Ok(true) => fixed += 1,
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(
                    thread_id,
                    rollout_path = %path.display(),
                    "归一 Codex rollout 元数据失败: {err}"
                );
            }
        }
    }
    Ok(fixed)
}

fn state_db_home(db_path: &Path) -> Option<PathBuf> {
    let codex_dir = db_path.parent()?;
    if codex_dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".codex")
    {
        return codex_dir.parent().map(Path::to_path_buf);
    }
    crate::config::home_dir()
}

fn thread_table_columns(conn: &Connection) -> Result<HashSet<String>> {
    let mut column_stmt = conn.prepare("PRAGMA table_info(threads)")?;
    let columns = column_stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    Ok(columns)
}

fn normalize_rollout_session_meta_provider(
    home: &Path,
    thread_id: &str,
    path: &Path,
    target_provider: &str,
) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    if !is_safe_codex_rollout_path(home, path) {
        tracing::warn!(
            thread_id,
            rollout_path = %path.display(),
            "跳过不在 Codex sessions 目录内的 rollout 元数据归一"
        );
        return Ok(false);
    }

    let initial_meta = std::fs::metadata(path)
        .with_context(|| format!("读取 rollout 元数据失败: {}", path.display()))?;
    let file = std::fs::File::open(path)
        .with_context(|| format!("打开 rollout 文件失败: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line)? == 0 {
        return Ok(false);
    }

    let line_ending = if first_line.ends_with("\r\n") {
        "\r\n"
    } else if first_line.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let json_line = first_line.trim_end_matches(['\r', '\n']);
    let mut meta: Value = serde_json::from_str(json_line)
        .with_context(|| format!("解析 rollout 首行 JSON 失败: {}", path.display()))?;

    if meta.get("type").and_then(Value::as_str) != Some("session_meta") {
        return Ok(false);
    }
    let Some(payload) = meta.get_mut("payload").and_then(Value::as_object_mut) else {
        return Ok(false);
    };
    if payload
        .get("source")
        .and_then(Value::as_str)
        .is_some_and(|source| source != "vscode")
    {
        return Ok(false);
    }
    if payload
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| id != thread_id)
    {
        tracing::warn!(
            thread_id,
            rollout_path = %path.display(),
            "rollout 首行线程 ID 不匹配，跳过元数据归一"
        );
        return Ok(false);
    }
    if payload
        .get("model_provider")
        .and_then(Value::as_str)
        .is_some_and(|provider| provider == target_provider)
    {
        return Ok(false);
    }

    payload.insert(
        "model_provider".to_string(),
        Value::String(target_provider.to_string()),
    );

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_path = path.with_file_name(format!(
        ".{}.deecodex-normalize-{}-{nonce}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("rollout.jsonl"),
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut tmp = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| format!("创建 rollout 临时文件失败: {}", tmp_path.display()))?;
        tmp.set_permissions(initial_meta.permissions())?;
        write!(
            tmp,
            "{}{}",
            serde_json::to_string(&meta).context("序列化 rollout 首行失败")?,
            line_ending
        )?;
        std::io::copy(&mut reader, &mut tmp)?;
        tmp.flush()?;

        let current_meta = std::fs::metadata(path)
            .with_context(|| format!("重新读取 rollout 元数据失败: {}", path.display()))?;
        if current_meta.len() != initial_meta.len()
            || current_meta.modified().ok() != initial_meta.modified().ok()
        {
            anyhow::bail!("rollout 文件写入期间发生变化，跳过本次归一");
        }

        std::fs::rename(&tmp_path, path)
            .with_context(|| format!("替换 rollout 文件失败: {}", path.display()))?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result.map(|_| true)
}

fn desktop_project_candidates(
    state: &Value,
    rows: Vec<DesktopThreadRow>,
) -> (BTreeMap<String, Vec<String>>, HashMap<String, String>) {
    let saved_roots = value_string_array(state.get("electron-saved-workspace-roots"));
    let active_roots = value_string_array(state.get("active-workspace-roots"));
    let project_order = value_string_array(state.get("project-order"));
    let labels = value_string_object(state.get("electron-workspace-root-labels"));

    let mut known_roots: BTreeSet<String> = saved_roots.iter().cloned().collect();
    known_roots.extend(active_roots.iter().cloned());
    known_roots.extend(project_order.iter().cloned());
    known_roots.extend(labels.keys().cloned());

    let mut order_project_by_thread: HashMap<String, String> = HashMap::new();
    if let Some(orders) = state
        .get("sidebar-project-thread-orders")
        .and_then(Value::as_object)
    {
        for (project, order) in orders {
            known_roots.insert(project.clone());
            let Some(thread_ids) = order.get("threadIds").and_then(Value::as_array) else {
                continue;
            };
            for thread_id in thread_ids.iter().filter_map(Value::as_str) {
                order_project_by_thread
                    .entry(thread_id.to_string())
                    .or_insert_with(|| project.clone());
            }
        }
    }

    let mut project_threads: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut assignments_by_thread: HashMap<String, String> = HashMap::new();
    for row in rows {
        let Some(project) = order_project_by_thread
            .get(&row.id)
            .cloned()
            .or_else(|| project_for_thread(&row.cwd, &known_roots))
        else {
            continue;
        };
        project_threads
            .entry(project.clone())
            .or_default()
            .push(row.id.clone());
        assignments_by_thread.insert(row.id, project);
    }
    (project_threads, assignments_by_thread)
}

fn get_desktop_project_index_status(
    db_path: &Path,
    conn: &Connection,
) -> Result<DesktopProjectIndexStatus> {
    let Some(global_path) = codex_global_state_path(db_path) else {
        return Ok(DesktopProjectIndexStatus::default());
    };
    if !global_path.exists() {
        return Ok(DesktopProjectIndexStatus::default());
    }

    let raw = std::fs::read_to_string(&global_path).context("读取 Codex Desktop 全局状态失败")?;
    let state: Value = serde_json::from_str(&raw).context("解析 Codex Desktop 全局状态失败")?;
    let rows = read_desktop_thread_rows(conn)?;
    let (project_threads, assignments_by_thread) = desktop_project_candidates(&state, rows);
    let recent_ids = desktop_recent_thread_ids(conn, DESKTOP_RECENT_LOAD_WINDOW)?;
    let candidate_ids = desktop_recent_repair_thread_ids(&project_threads);
    let recent_visible_count = candidate_ids
        .iter()
        .filter(|id| recent_ids.contains(*id))
        .count();
    let recent_pending_count = candidate_ids.len().saturating_sub(recent_visible_count);
    let assignments = state
        .get("thread-project-assignments")
        .and_then(Value::as_object);
    let orders = state
        .get("sidebar-project-thread-orders")
        .and_then(Value::as_object);
    let projectless: HashSet<String> = value_string_array(state.get("projectless-thread-ids"))
        .into_iter()
        .collect();

    let mut indexed_count = 0usize;
    let mut pending_count = 0usize;
    for (thread_id, project) in &assignments_by_thread {
        let assigned = assignments
            .and_then(|items| items.get(thread_id))
            .and_then(|item| item.get("projectId"))
            .and_then(Value::as_str)
            == Some(project.as_str());
        let ordered = orders
            .and_then(|items| items.get(project))
            .and_then(|item| item.get("threadIds"))
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(thread_id)));
        if assigned && ordered && !projectless.contains(thread_id) {
            indexed_count += 1;
        } else {
            pending_count += 1;
        }
    }

    if project_threads.is_empty() {
        pending_count = 0;
    }

    Ok(DesktopProjectIndexStatus {
        indexed_count,
        pending_count,
        repair_blocked: false,
        recent_visible_count,
        recent_pending_count,
        recent_repair_blocked: false,
    })
}

fn repair_desktop_project_index(
    db_path: &Path,
    conn: &Connection,
) -> Result<DesktopProjectRepairResult> {
    let Some(global_path) = codex_global_state_path(db_path) else {
        return Ok(DesktopProjectRepairResult::default());
    };
    if !global_path.exists() {
        tracing::debug!(
            path = %global_path.display(),
            "Codex Desktop 全局状态不存在，跳过项目索引修复"
        );
        return Ok(DesktopProjectRepairResult::default());
    }

    let rows = read_desktop_thread_rows(conn)?;
    if rows.is_empty() {
        return Ok(DesktopProjectRepairResult::default());
    }

    let mut total_changed = 0usize;
    let mut last_pending = get_desktop_project_index_status(db_path, conn)?.pending_count;
    let mut backup_written = false;

    for attempt in 1..=DESKTOP_PROJECT_INDEX_REPAIR_ATTEMPTS {
        let initial_signature = file_signature(&global_path)?;
        let raw =
            std::fs::read_to_string(&global_path).context("读取 Codex Desktop 全局状态失败")?;
        let mut state: Value =
            serde_json::from_str(&raw).context("解析 Codex Desktop 全局状态失败")?;

        let (project_threads, assignments_by_thread) =
            desktop_project_candidates(&state, rows.clone());
        if project_threads.is_empty() {
            return Ok(DesktopProjectRepairResult {
                fixed_count: total_changed,
                pending_count: 0,
                blocked: false,
            });
        }

        let mut changed = 0usize;
        let saved_roots = value_string_array(state.get("electron-saved-workspace-roots"));
        let active_roots = value_string_array(state.get("active-workspace-roots"));
        let project_order = value_string_array(state.get("project-order"));
        let labels = value_string_object(state.get("electron-workspace-root-labels"));
        let mut next_saved_roots = saved_roots;
        let mut next_active_roots = active_roots;
        let mut next_project_order = project_order;
        let mut next_labels = labels;
        let mut next_assignments = value_string_object(state.get("thread-project-assignments"));
        let mut next_projectless = value_string_array(state.get("projectless-thread-ids"));
        let mut next_hints = value_string_object(state.get("thread-workspace-root-hints"));
        let mut next_orders = value_string_object(state.get("sidebar-project-thread-orders"));

        for project in project_threads.keys() {
            changed += append_unique(&mut next_saved_roots, project) as usize;
            changed += append_unique(&mut next_active_roots, project) as usize;
            changed += append_unique(&mut next_project_order, project) as usize;
            if !next_labels.contains_key(project) {
                next_labels.insert(project.clone(), Value::String(project_label(project)));
                changed += 1;
            }
        }

        for (thread_id, project) in &assignments_by_thread {
            let assignment = json!({
                "projectKind": "local",
                "projectId": project,
                "path": project,
                "pendingCoreUpdate": false,
            });
            if next_assignments.get(thread_id) != Some(&assignment) {
                next_assignments.insert(thread_id.clone(), assignment);
                changed += 1;
            }
            changed += remove_value(&mut next_projectless, thread_id) as usize;
            if next_hints.get(thread_id).and_then(Value::as_str) != Some(project.as_str()) {
                next_hints.insert(thread_id.clone(), Value::String(project.clone()));
                changed += 1;
            }
        }

        for (project, thread_ids) in &project_threads {
            let mut merged = Vec::new();
            let mut seen = HashSet::new();
            for id in thread_ids {
                if seen.insert(id.clone()) {
                    merged.push(Value::String(id.clone()));
                }
            }
            let sort_key = next_orders
                .get(project)
                .and_then(Value::as_object)
                .and_then(|object| object.get("sortKey"))
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(existing_ids) = next_orders
                .get(project)
                .and_then(Value::as_object)
                .and_then(|object| object.get("threadIds"))
                .and_then(Value::as_array)
            {
                for id in existing_ids.iter().filter_map(Value::as_str) {
                    if seen.insert(id.to_string()) {
                        merged.push(Value::String(id.to_string()));
                    }
                }
            }

            let mut order = Map::new();
            order.insert("threadIds".to_string(), Value::Array(merged));
            if let Some(sort_key) = sort_key {
                order.insert("sortKey".to_string(), Value::String(sort_key));
            }
            let next = Value::Object(order);
            if next_orders.get(project) != Some(&next) {
                next_orders.insert(project.clone(), next);
                changed += 1;
            }
        }

        if changed == 0 {
            let status = get_desktop_project_index_status(db_path, conn)?;
            return Ok(DesktopProjectRepairResult {
                fixed_count: total_changed,
                pending_count: status.pending_count,
                blocked: false,
            });
        }

        state["electron-saved-workspace-roots"] = json!(next_saved_roots);
        state["active-workspace-roots"] = json!(next_active_roots);
        state["project-order"] = json!(next_project_order);
        state["electron-workspace-root-labels"] = Value::Object(next_labels);
        state["thread-project-assignments"] = Value::Object(next_assignments);
        state["projectless-thread-ids"] = json!(next_projectless);
        state["thread-workspace-root-hints"] = Value::Object(next_hints);
        state["sidebar-project-thread-orders"] = Value::Object(next_orders);

        if file_signature(&global_path)? != initial_signature {
            tracing::warn!(
                attempt,
                path = %global_path.display(),
                "Codex Desktop 全局状态在项目索引修复期间发生变化，重读后重试"
            );
            continue;
        }

        if !backup_written {
            let backup_path = global_path.with_file_name(format!(
                ".codex-global-state.json.deecodex-desktop-index-{}.bak",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::write(&backup_path, &raw).context("备份 Codex Desktop 全局状态失败")?;
            backup_written = true;
        }

        write_json_atomically(&global_path, &state)?;
        total_changed += changed;

        let after_status = get_desktop_project_index_status(db_path, conn)?;
        last_pending = after_status.pending_count;
        if last_pending == 0 {
            tracing::info!(
                fixed = total_changed,
                projects = project_threads.len(),
                path = %global_path.display(),
                "已修复 Codex Desktop 项目线程索引"
            );
            return Ok(DesktopProjectRepairResult {
                fixed_count: total_changed,
                pending_count: 0,
                blocked: false,
            });
        }

        tracing::warn!(
            attempt,
            pending = last_pending,
            path = %global_path.display(),
            "Codex Desktop 项目索引写入后仍有待补齐项，重读状态后重试"
        );
    }

    tracing::warn!(
        pending = last_pending,
        path = %global_path.display(),
        "Codex Desktop 项目索引修复已重试，仍有待补齐项"
    );
    Ok(DesktopProjectRepairResult {
        fixed_count: total_changed,
        pending_count: last_pending,
        blocked: false,
    })
}

fn load_desktop_recent_backup(
    backup_path: &Path,
) -> Result<HashMap<String, DesktopRecentTimestampBackup>> {
    if !backup_path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(backup_path).context("读取 Desktop recent 备份失败")?;
    let rows: Vec<DesktopRecentTimestampBackup> =
        serde_json::from_str(&raw).context("解析 Desktop recent 备份失败")?;
    Ok(rows.into_iter().map(|row| (row.id.clone(), row)).collect())
}

fn write_desktop_recent_backup(
    backup_path: &Path,
    backups: &HashMap<String, DesktopRecentTimestampBackup>,
) -> Result<()> {
    if backups.is_empty() {
        if backup_path.exists() {
            std::fs::remove_file(backup_path).context("删除 Desktop recent 备份失败")?;
        }
        return Ok(());
    }
    if let Some(parent) = backup_path.parent() {
        std::fs::create_dir_all(parent).context("创建 Desktop recent 备份目录失败")?;
    }
    let mut rows = backups.values().cloned().collect::<Vec<_>>();
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    let raw = serde_json::to_string_pretty(&rows).context("序列化 Desktop recent 备份失败")?;
    std::fs::write(backup_path, raw).context("写入 Desktop recent 备份失败")
}

fn repair_desktop_recent_visibility(
    db_path: &Path,
    conn: &Connection,
    _backup_path: &Path,
) -> Result<DesktopRecentRepairResult> {
    let Some(global_path) = codex_global_state_path(db_path) else {
        return Ok(DesktopRecentRepairResult::default());
    };
    if !global_path.exists() {
        return Ok(DesktopRecentRepairResult::default());
    }

    let raw = std::fs::read_to_string(&global_path).context("读取 Codex Desktop 全局状态失败")?;
    let state: Value = serde_json::from_str(&raw).context("解析 Codex Desktop 全局状态失败")?;
    let rows = read_desktop_thread_rows(conn)?;
    let (project_threads, _) = desktop_project_candidates(&state, rows);
    let repair_ids = desktop_recent_repair_order(&project_threads);
    let candidate_ids: HashSet<String> = repair_ids.iter().cloned().collect();
    if candidate_ids.is_empty() {
        return Ok(DesktopRecentRepairResult::default());
    }

    let recent_ids = desktop_recent_thread_ids(conn, DESKTOP_RECENT_LOAD_WINDOW)?;
    let mut pending_ids = candidate_ids
        .into_iter()
        .filter(|id| !recent_ids.contains(id))
        .collect::<Vec<_>>();
    if pending_ids.is_empty() {
        return Ok(DesktopRecentRepairResult::default());
    }
    pending_ids.sort();

    // Recent 的排序是用户的真实使用时间线。旧实现会把项目线程的
    // updated_at/updated_at_ms 临时抬到当前时间，虽然能挤进首屏，
    // 但会把其他真实最近线程顶出去，看起来像“线程丢失”。这里改为只
    // 报告待显示数量，项目索引仍可修复，但不再改写线程时间戳。
    if is_live_codex_desktop_state(&global_path) {
        tracing::info!(
            pending = pending_ids.len(),
            path = %global_path.display(),
            "Codex Desktop 正在运行，Recent 仍仅做只读诊断"
        );
    } else {
        tracing::info!(
            pending = pending_ids.len(),
            path = %global_path.display(),
            "Recent 仅做只读诊断，不改写线程时间戳"
        );
    }

    Ok(DesktopRecentRepairResult {
        fixed_count: 0,
        pending_count: pending_ids.len(),
        blocked: false,
    })
}

fn restore_desktop_recent_timestamps(conn: &Connection, backup_path: &Path) -> Result<usize> {
    let backups = load_desktop_recent_backup(backup_path)?;
    if backups.is_empty() {
        return Ok(0);
    }

    let mut restored = 0usize;
    let mut failed: HashMap<String, DesktopRecentTimestampBackup> = HashMap::new();
    for backup in backups.values() {
        match conn.execute(
            "UPDATE threads
             SET updated_at = ?1,
                 updated_at_ms = ?2
             WHERE id = ?3",
            rusqlite::params![backup.updated_at, backup.updated_at_ms, backup.id],
        ) {
            Ok(n) => restored += n,
            Err(err) => {
                tracing::warn!("还原线程 {} Desktop recent 时间戳失败: {err}", backup.id);
                failed.insert(backup.id.clone(), backup.clone());
            }
        }
    }
    write_desktop_recent_backup(backup_path, &failed)?;
    Ok(restored)
}

fn backup_non_deecodex_threads(
    conn: &Connection,
    backup_path: &Path,
) -> Result<Vec<(String, String)>> {
    let target_provider = current_thread_provider();
    let mut stmt =
        conn.prepare("SELECT id, model_provider FROM threads WHERE model_provider != ?1")?;
    let non_unified: Vec<(String, String)> = stmt
        .query_map(rusqlite::params![target_provider.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);

    if non_unified.is_empty() {
        return Ok(Vec::new());
    }

    let mut merged: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if backup_path.exists() {
        if let Ok(existing_json) = std::fs::read_to_string(backup_path) {
            if let Ok(existing) = serde_json::from_str::<Vec<(String, String)>>(&existing_json) {
                for (id, provider) in existing {
                    merged.insert(id, provider);
                }
            }
        }
    }
    for (id, provider) in &non_unified {
        merged.entry(id.clone()).or_insert_with(|| provider.clone());
    }
    let merged_vec: Vec<(String, String)> = merged.into_iter().collect();
    let backup_json = serde_json::to_string_pretty(&merged_vec).context("序列化迁移备份失败")?;
    std::fs::write(backup_path, backup_json).context("写入迁移备份文件失败")?;

    Ok(non_unified)
}

fn unify_remaining_non_deecodex(conn: &Connection, backup_path: &Path) -> Result<usize> {
    let target_provider = current_thread_provider();
    let non_unified = backup_non_deecodex_threads(conn, backup_path)?;
    if non_unified.is_empty() {
        return Ok(0);
    }

    let changed = conn
        .execute(
            "UPDATE threads SET model_provider = ?1 WHERE model_provider != ?1",
            rusqlite::params![target_provider.as_str()],
        )
        .context("兜底统一 Codex 线程 provider 失败")?;
    if changed < non_unified.len() {
        tracing::warn!(
            target_provider,
            changed,
            expected = non_unified.len(),
            "兜底统一 Codex 线程 provider 未完全成功"
        );
    } else {
        tracing::info!(target_provider, changed, "兜底统一 Codex 线程 provider");
    }
    Ok(changed)
}

fn restore_thread_cwds(conn: &Connection, cwd_backup_path: &Path) -> Result<usize> {
    if !cwd_backup_path.exists() {
        return Ok(0);
    }

    let backup_json =
        std::fs::read_to_string(cwd_backup_path).context("读取 cwd 可见性备份失败")?;
    let originals: Vec<(String, String)> =
        serde_json::from_str(&backup_json).context("解析 cwd 可见性备份失败")?;

    let mut restored = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    for (id, original_cwd) in &originals {
        match conn.execute(
            "UPDATE threads SET cwd = ?1 WHERE id = ?2",
            rusqlite::params![original_cwd, id],
        ) {
            Ok(n) => restored += n,
            Err(err) => {
                tracing::warn!("还原线程 {id} cwd 失败: {err}");
                failed.push((id.clone(), original_cwd.clone()));
            }
        }
    }

    if failed.is_empty() {
        std::fs::remove_file(cwd_backup_path).context("删除 cwd 可见性备份失败")?;
    } else {
        let failed_json =
            serde_json::to_string_pretty(&failed).context("序列化剩余 cwd 备份失败")?;
        std::fs::write(cwd_backup_path, failed_json).context("写入剩余 cwd 备份失败")?;
    }
    Ok(restored)
}

fn do_normalize_desktop_threads(
    db_path: &Path,
    desktop_recent_backup_path: &Path,
) -> Result<MigrationDiff> {
    let target_provider = current_thread_provider();
    let before = get_provider_summary(db_path)?;
    let codex_desktop_running = is_codex_desktop_running_for_db(db_path);

    let conn = Connection::open(db_path)?;
    // 设置 busy timeout 以应对 Codex 持有的写锁；不切换 journal_mode，避免无变更时改动数据库模式。
    conn.pragma_update(None, "busy_timeout", "5000")?;

    let remaining_before = count_non_current_managed_threads(&conn, &target_provider)?;
    let desktop_changed = normalize_desktop_thread_providers(&conn, &target_provider)?;
    let managed_changed = normalize_managed_thread_providers(&conn, &target_provider)?;
    let changed = desktop_changed + managed_changed;
    let rollout_metadata_fixed_count =
        normalize_desktop_rollout_metadata(db_path, &conn, &target_provider)?;
    let visibility_fixed_count = repair_thread_visibility(&conn)?;
    let metadata_fixed_count = repair_desktop_project_metadata(&conn)?;
    let desktop_project_repair = repair_desktop_project_index(db_path, &conn)?;
    let desktop_recent_repair =
        repair_desktop_recent_visibility(db_path, &conn, desktop_recent_backup_path)?;
    let desktop_project_fixed_count = metadata_fixed_count + desktop_project_repair.fixed_count;
    let after = get_provider_summary(db_path)?;
    let remaining_non_unified_count = count_non_current_managed_threads(&conn, &target_provider)?;

    if remaining_non_unified_count > 0 {
        tracing::warn!(
            target_provider,
            changed,
            desktop_changed,
            managed_changed,
            rollout_metadata_fixed_count,
            remaining_non_unified_count,
            "DEX 管理线程归一后仍有未统一线程"
        );
    } else if changed > 0 || rollout_metadata_fixed_count > 0 {
        tracing::info!(
            target_provider,
            changed,
            desktop_changed,
            managed_changed,
            rollout_metadata_fixed_count,
            before = remaining_before,
            "已归一 DEX 管理线程到当前 provider"
        );
    } else {
        tracing::debug!(target_provider, "DEX 管理线程已处于当前 provider");
    }

    Ok(MigrationDiff {
        before,
        after,
        target_provider,
        changed_count: changed,
        rollout_metadata_fixed_count,
        remaining_non_unified_count,
        visibility_fixed_count,
        desktop_project_fixed_count,
        desktop_recent_fixed_count: desktop_recent_repair.fixed_count,
        desktop_project_pending_count: desktop_project_repair.pending_count,
        desktop_recent_pending_count: desktop_recent_repair.pending_count,
        desktop_project_repair_blocked: desktop_project_repair.blocked,
        desktop_recent_repair_blocked: desktop_recent_repair.blocked,
        codex_desktop_running,
        cwd_aligned_count: 0,
    })
}

fn do_calibrate(
    db_path: &Path,
    backup_path: &Path,
    _cwd_backup_path: &Path,
    desktop_recent_backup_path: &Path,
) -> Result<MigrationDiff> {
    let target_provider = current_thread_provider();
    let before = get_provider_summary(db_path)?;
    let codex_desktop_running = is_codex_desktop_running_for_db(db_path);

    if !backup_path.exists() {
        let conn = Connection::open(db_path)?;
        conn.pragma_update(None, "busy_timeout", "5000")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let remaining_before = count_non_unified_provider(&conn, &target_provider)?;
        if codex_desktop_running && remaining_before > 0 {
            tracing::info!(
                remaining_before,
                "Codex Desktop 正在运行，继续执行线程校准；如运行态稍后回写，下次校准会自动重试"
            );
        }
        let migrated = unify_remaining_non_deecodex(&conn, backup_path)?;
        let visibility_fixed_count = repair_thread_visibility(&conn)?;
        let metadata_fixed_count = repair_desktop_project_metadata(&conn)?;
        let desktop_project_repair = repair_desktop_project_index(db_path, &conn)?;
        let desktop_recent_repair =
            repair_desktop_recent_visibility(db_path, &conn, desktop_recent_backup_path)?;
        let desktop_project_fixed_count = metadata_fixed_count + desktop_project_repair.fixed_count;
        let after = get_provider_summary(db_path)?;
        let remaining_non_unified_count = count_non_unified_provider(&conn, &target_provider)?;
        return Ok(MigrationDiff {
            before,
            after,
            target_provider,
            changed_count: migrated,
            rollout_metadata_fixed_count: 0,
            remaining_non_unified_count,
            visibility_fixed_count,
            desktop_project_fixed_count,
            desktop_recent_fixed_count: desktop_recent_repair.fixed_count,
            desktop_project_pending_count: desktop_project_repair.pending_count,
            desktop_recent_pending_count: desktop_recent_repair.pending_count,
            desktop_project_repair_blocked: desktop_project_repair.blocked,
            desktop_recent_repair_blocked: desktop_recent_repair.blocked,
            codex_desktop_running,
            cwd_aligned_count: 0,
        });
    }

    let backup_json = std::fs::read_to_string(backup_path).context("读取迁移备份文件失败")?;
    let mut originals: Vec<(String, String)> =
        serde_json::from_str(&backup_json).context("解析迁移备份文件失败")?;

    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "busy_timeout", "5000")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    let remaining_before = count_non_unified_provider(&conn, &target_provider)?;
    if codex_desktop_running && remaining_before > 0 {
        tracing::info!(
            remaining_before,
            "Codex Desktop 正在运行，继续执行线程校准；如运行态稍后回写，下次校准会自动重试"
        );
    }

    let current_threads: Vec<(String, String)> = {
        let mut stmt = conn.prepare("SELECT id, model_provider FROM threads")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let current_ids: std::collections::HashSet<String> =
        current_threads.iter().map(|(id, _)| id.clone()).collect();

    let old_len = originals.len();
    originals.retain(|(id, _)| current_ids.contains(id));
    let removed = old_len - originals.len();
    if removed > 0 {
        tracing::info!("校准: 从备份中移除 {removed} 条已删除线程");
    }

    let backed_up_ids: std::collections::HashSet<String> =
        originals.iter().map(|(id, _)| id.clone()).collect();
    let new_originals: Vec<(String, String)> = current_threads
        .into_iter()
        .filter(|(id, provider)| provider != &target_provider && !backed_up_ids.contains(id))
        .collect();
    let added = new_originals.len();
    if added > 0 {
        tracing::info!("校准: 追加 {added} 条新增线程到迁移备份");
        originals.extend(new_originals);
    }

    let backup_json = serde_json::to_string_pretty(&originals).context("序列化校准备份失败")?;
    std::fs::write(backup_path, backup_json).context("写入校准备份文件失败")?;

    let mut migrated = 0usize;
    for (id, _) in &originals {
        match conn.execute(
            "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND model_provider != ?1",
            rusqlite::params![target_provider.as_str(), id],
        ) {
            Ok(n) => migrated += n,
            Err(e) => {
                tracing::warn!("校准线程 {id} 失败: {e}");
            }
        }
    }
    migrated += unify_remaining_non_deecodex(&conn, backup_path)?;
    let visibility_fixed_count = repair_thread_visibility(&conn)?;
    let metadata_fixed_count = repair_desktop_project_metadata(&conn)?;
    let desktop_project_repair = repair_desktop_project_index(db_path, &conn)?;
    let desktop_recent_repair =
        repair_desktop_recent_visibility(db_path, &conn, desktop_recent_backup_path)?;
    let desktop_project_fixed_count = metadata_fixed_count + desktop_project_repair.fixed_count;

    let after = get_provider_summary(db_path)?;
    let remaining_non_unified_count = count_non_unified_provider(&conn, &target_provider)?;
    let skipped = originals.len().saturating_sub(migrated);
    if migrated > 0 || removed > 0 {
        tracing::info!(
            target_provider,
            "已校准 {migrated} 条线程到当前 DEX provider，清理 {removed} 条备份记录"
        );
    }
    if skipped > 0 && after.iter().any(|s| s.provider != target_provider) {
        tracing::warn!("校准: {migrated} 条成功，仍有线程未统一，下次校准会自动重试");
    }

    Ok(MigrationDiff {
        before,
        after,
        target_provider,
        changed_count: migrated + removed,
        rollout_metadata_fixed_count: 0,
        remaining_non_unified_count,
        visibility_fixed_count,
        desktop_project_fixed_count,
        desktop_recent_fixed_count: desktop_recent_repair.fixed_count,
        desktop_project_pending_count: desktop_project_repair.pending_count,
        desktop_recent_pending_count: desktop_recent_repair.pending_count,
        desktop_project_repair_blocked: desktop_project_repair.blocked,
        desktop_recent_repair_blocked: desktop_recent_repair.blocked,
        codex_desktop_running,
        cwd_aligned_count: 0,
    })
}

fn do_restore(
    db_path: &Path,
    backup_path: &Path,
    cwd_backup_path: &Path,
    desktop_recent_backup_path: &Path,
) -> Result<MigrationDiff> {
    let before = get_provider_summary(db_path)?;
    let target_provider = current_thread_provider();
    let codex_desktop_running = is_codex_desktop_running_for_db(db_path);

    let backup_json = std::fs::read_to_string(backup_path).context("读取迁移备份文件失败")?;
    let originals: Vec<(String, String)> =
        serde_json::from_str(&backup_json).context("解析迁移备份文件失败")?;

    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "busy_timeout", "5000")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;

    let mut restored = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    for (id, original_provider) in &originals {
        match conn.execute(
            "UPDATE threads SET model_provider = ?1 WHERE id = ?2",
            rusqlite::params![original_provider, id],
        ) {
            Ok(n) => restored += n,
            Err(e) => {
                tracing::warn!("还原线程 {id} 失败: {e}");
                failed.push((id.clone(), original_provider.clone()));
            }
        }
    }

    if failed.is_empty() {
        std::fs::remove_file(backup_path).context("删除迁移备份文件失败")?;
        tracing::info!("已还原 {restored} 条线程的 model_provider，备份已删除");
    } else {
        // 保留未成功还原的线程在备份中
        let failed_json = serde_json::to_string_pretty(&failed).context("序列化剩余备份失败")?;
        std::fs::write(backup_path, failed_json).context("写入剩余备份文件失败")?;
        tracing::warn!(
            "已还原 {restored}/{total} 条线程，{failed} 条因锁冲突保留在备份中",
            total = originals.len(),
            failed = failed.len()
        );
    }
    let cwd_restored = restore_thread_cwds(&conn, cwd_backup_path)?;
    let desktop_recent_fixed_count =
        restore_desktop_recent_timestamps(&conn, desktop_recent_backup_path)?;

    let after = get_provider_summary(db_path)?;
    let remaining_non_unified_count = count_non_unified_provider(&conn, &target_provider)?;

    Ok(MigrationDiff {
        before,
        after,
        target_provider,
        changed_count: restored,
        rollout_metadata_fixed_count: 0,
        remaining_non_unified_count,
        visibility_fixed_count: 0,
        desktop_project_fixed_count: 0,
        desktop_recent_fixed_count,
        desktop_project_pending_count: 0,
        desktop_recent_pending_count: 0,
        desktop_project_repair_blocked: false,
        desktop_recent_repair_blocked: false,
        codex_desktop_running,
        cwd_aligned_count: cwd_restored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::io::Write;

    fn temp_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("deecodex-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn create_test_threads_db(path: &Path, threads: &[(&str, &str)]) {
        create_test_threads_db_with_rows(
            path,
            &threads
                .iter()
                .map(|(id, provider)| (*id, *provider, "", "", "", "vscode", "/tmp/project", 0))
                .collect::<Vec<_>>(),
        );
    }

    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn create_test_threads_db_with_rows(
        path: &Path,
        threads: &[(&str, &str, &str, &str, &str, &str, &str, i32)],
    ) {
        let conn = Connection::open(path).expect("open test db");
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                thread_source TEXT,
                source TEXT NOT NULL DEFAULT 'vscode',
                cwd TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                has_user_event INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .expect("create threads table");
        for (id, provider, title, preview, first_user_message, source, cwd, archived) in threads {
            conn.execute(
                "INSERT INTO threads (
                    id, model_provider, title, preview, first_user_message,
                    thread_source, source, cwd, archived
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    provider,
                    title,
                    preview,
                    first_user_message,
                    "user",
                    source,
                    cwd,
                    archived
                ],
            )
            .expect("insert thread");
        }
        conn.execute(
            "CREATE TABLE stage1_outputs (
                thread_id TEXT PRIMARY KEY,
                source_updated_at INTEGER NOT NULL,
                raw_memory TEXT NOT NULL,
                rollout_summary TEXT NOT NULL,
                generated_at INTEGER NOT NULL
            )",
            [],
        )
        .expect("create stage1_outputs table");
    }

    fn create_test_threads_db_with_rollout(path: &Path, thread_id: &str, rollout_path: &Path) {
        let conn = Connection::open(path).expect("open test db");
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                rollout_path TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL DEFAULT 'vscode',
                cwd TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .expect("create threads table");
        conn.execute(
            "INSERT INTO threads (
                id, model_provider, title, preview, first_user_message,
                rollout_path, source, cwd, archived
            ) VALUES (?1, 'deecodex', '待删除', '预览', '首问', ?2, 'vscode', '/tmp/project', 0)",
            params![thread_id, rollout_path.to_string_lossy().to_string()],
        )
        .expect("insert thread");
        conn.execute(
            "CREATE TABLE stage1_outputs (
                thread_id TEXT PRIMARY KEY,
                source_updated_at INTEGER NOT NULL,
                raw_memory TEXT NOT NULL,
                rollout_summary TEXT NOT NULL,
                generated_at INTEGER NOT NULL
            )",
            [],
        )
        .expect("create stage1_outputs table");
        conn.execute(
            "INSERT INTO stage1_outputs
             (thread_id, source_updated_at, raw_memory, rollout_summary, generated_at)
             VALUES (?1, 1, '{}', 'summary', 2)",
            params![thread_id],
        )
        .expect("insert stage1");
        conn.execute(
            "CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                position INTEGER NOT NULL
            )",
            [],
        )
        .expect("create dynamic tools table");
        conn.execute(
            "INSERT INTO thread_dynamic_tools
             (thread_id, name, description, position)
             VALUES (?1, 'tool', 'desc', 1)",
            params![thread_id],
        )
        .expect("insert dynamic tool");
    }

    #[test]
    fn find_state_db_prefers_highest_version_over_largest_file() {
        let dir = temp_test_dir("state-db-version");
        let codex_dir = dir.join(".codex");
        std::fs::create_dir_all(&codex_dir).expect("create codex dir");
        std::fs::write(codex_dir.join("state_5.sqlite"), vec![0_u8; 4096])
            .expect("write older larger db");
        std::fs::write(codex_dir.join("state_10.sqlite"), [1_u8]).expect("write newer smaller db");
        std::fs::write(codex_dir.join("state_10.sqlite-wal"), [2_u8]).expect("write wal");

        let found = find_state_db(&dir).expect("find state db");
        assert_eq!(
            found.file_name().and_then(|name| name.to_str()),
            Some("state_10.sqlite")
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn delete_thread_removes_db_rows_rollout_backup_and_desktop_index() {
        let dir = temp_test_dir("thread-delete");
        let codex_dir = dir.join(".codex");
        let sessions_dir = codex_dir
            .join("sessions")
            .join("2026")
            .join("06")
            .join("03");
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let rollout_path = sessions_dir.join("rollout-delete-me.jsonl");
        std::fs::write(&rollout_path, "{\"type\":\"response_item\"}\n").expect("write rollout");

        let db_path = codex_dir.join("state_10.sqlite");
        create_test_threads_db_with_rollout(&db_path, "delete-me", &rollout_path);
        let backup_path = backup_path(&dir);
        std::fs::write(
            &backup_path,
            serde_json::to_string(&vec![
                ("delete-me".to_string(), "codex".to_string()),
                ("keep-me".to_string(), "codex".to_string()),
            ])
            .unwrap(),
        )
        .expect("write migration backup");
        let global_path = codex_dir.join(".codex-global-state.json");
        std::fs::write(
            &global_path,
            serde_json::to_string(&json!({
                "thread-project-assignments": {
                    "delete-me": {"projectId": "/tmp/project"},
                    "keep-me": {"projectId": "/tmp/project"}
                },
                "thread-workspace-root-hints": {
                    "delete-me": "/tmp/project",
                    "keep-me": "/tmp/project"
                },
                "thread-projectless-output-directories": {
                    "delete-me": "/tmp/output"
                },
                "projectless-thread-ids": ["delete-me", "keep-me"],
                "sidebar-project-thread-orders": {
                    "/tmp/project": {"threadIds": ["delete-me", "keep-me"], "sortKey": "abc"},
                    "/tmp/empty": {"threadIds": ["delete-me"]}
                }
            }))
            .unwrap(),
        )
        .expect("write global state");

        delete_thread_from_db(&dir, &dir, &db_path, "delete-me").expect("delete thread");

        let conn = Connection::open(&db_path).expect("open db after delete");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM threads WHERE id = 'delete-me'",
                [],
                |row| row.get(0),
            )
            .expect("count thread");
        assert_eq!(count, 0);
        let stage1_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM stage1_outputs WHERE thread_id = 'delete-me'",
                [],
                |row| row.get(0),
            )
            .expect("count stage1");
        assert_eq!(stage1_count, 0);
        let tools_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM thread_dynamic_tools WHERE thread_id = 'delete-me'",
                [],
                |row| row.get(0),
            )
            .expect("count dynamic tools");
        assert_eq!(tools_count, 0);
        assert!(!rollout_path.exists());

        let backup_json = std::fs::read_to_string(&backup_path).expect("read backup");
        assert!(!backup_json.contains("delete-me"));
        assert!(backup_json.contains("keep-me"));

        let state: Value = serde_json::from_str(
            &std::fs::read_to_string(&global_path).expect("read global state"),
        )
        .expect("parse global state");
        assert!(state["thread-project-assignments"]
            .get("delete-me")
            .is_none());
        assert!(state["thread-workspace-root-hints"]
            .get("delete-me")
            .is_none());
        assert!(state["thread-projectless-output-directories"]
            .get("delete-me")
            .is_none());
        assert!(state["projectless-thread-ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value.as_str() != Some("delete-me")));
        let order_ids = state["sidebar-project-thread-orders"]["/tmp/project"]["threadIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(order_ids, vec!["keep-me"]);
        assert!(state["sidebar-project-thread-orders"]
            .get("/tmp/empty")
            .is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_rollout_messages_collects_full_thread_items() {
        let path = std::env::temp_dir().join(format!(
            "deecodex-rollout-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut file = std::fs::File::create(&path).expect("create temp rollout");
        writeln!(
            file,
            r#"{{"type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"第一问"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"response_item","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"第一答"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"response_item","payload":{{"type":"function_call_output","output":"工具结果"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"compacted","payload":{{"message":"压缩摘要"}}}}"#
        )
        .unwrap();

        let messages = read_rollout_messages(&path).expect("read rollout messages");
        std::fs::remove_file(&path).ok();

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[3]["role"], "system");
        assert!(messages[0]["payload"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("第一问"));
        assert!(messages[3]["payload"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("压缩摘要"));
    }

    #[test]
    fn read_rollout_messages_summarizes_structured_tool_outputs() {
        let path = std::env::temp_dir().join(format!(
            "deecodex-rollout-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut file = std::fs::File::create(&path).expect("create temp rollout");
        writeln!(
            file,
            r#"{{"type":"response_item","payload":{{"type":"function_call_output","output":[{{"type":"output_text","text":"工具文本"}},{{"type":"input_image","image_url":"data:image/png;base64,AAAAAAAAAAAAAAAA"}}]}}}}"#
        )
        .unwrap();

        let messages = read_rollout_messages(&path).expect("read rollout messages");
        std::fs::remove_file(&path).ok();
        let text = messages[0]["payload"]["content"][0]["text"]
            .as_str()
            .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "tool");
        assert!(text.contains("工具文本"));
        assert!(text.contains("图片内容"));
        assert!(!text.contains("AAAAAAAAAAAAAAAA"));
    }

    #[test]
    fn read_latest_rollout_token_usage_reads_context_window() {
        let path = std::env::temp_dir().join(format!(
            "deecodex-rollout-token-test-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let mut file = std::fs::File::create(&path).expect("create temp rollout");
        writeln!(
            file,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"total_tokens":1200}},"model_context_window":258400}}}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"total_tokens":2400}},"model_context_window":258400}}}}}}"#
        )
        .unwrap();

        let usage = read_latest_rollout_token_usage(&path)
            .expect("read rollout token usage")
            .expect("token usage");
        std::fs::remove_file(&path).ok();

        assert_eq!(usage.model_context_window, Some(258400));
        assert_eq!(usage.last_total_tokens, Some(2400));
    }

    #[test]
    fn read_catalog_context_window_applies_effective_percent() {
        let dir = temp_test_dir("codex-context-catalog");
        let path = dir.join("models_deecodex.json");
        std::fs::write(
            &path,
            r#"{"models":[{"slug":"gpt-5.5","context_window":272000,"effective_context_window_percent":95}]}"#,
        )
        .expect("write catalog");

        let (catalog_window, effective_window) =
            read_catalog_context_window(&path, Some("gpt-5.5"), Some(272000))
                .expect("read catalog");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(catalog_window, Some(272000));
        assert_eq!(effective_window, Some(258400));
    }

    #[test]
    fn migrate_normalizes_desktop_rollout_session_meta_provider() {
        let dir = temp_test_dir("thread-rollout-metadata");
        let codex_dir = dir.join(".codex");
        let sessions_dir = codex_dir.join("sessions/2026/06/06");
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let rollout_path = sessions_dir.join("rollout-2026-06-06T00-00-00-desktop.jsonl");
        std::fs::write(
            &rollout_path,
            concat!(
                r#"{"type":"session_meta","payload":{"id":"desktop","source":"vscode","model_provider":"deecodex"}}"#,
                "\n",
                r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#,
                "\n"
            ),
        )
        .expect("write desktop rollout");
        let cli_rollout_path = sessions_dir.join("rollout-2026-06-06T00-00-00-cli.jsonl");
        std::fs::write(
            &cli_rollout_path,
            r#"{"type":"session_meta","payload":{"id":"cli","source":"cli","model_provider":"deecodex"}}"#,
        )
        .expect("write cli rollout");

        let db_path = codex_dir.join("state_10.sqlite");
        let conn = Connection::open(&db_path).expect("open db");
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                rollout_path TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL DEFAULT 'vscode',
                cwd TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                has_user_event INTEGER NOT NULL DEFAULT 0
            )",
            [],
        )
        .expect("create threads");
        conn.execute(
            "INSERT INTO threads (id, model_provider, rollout_path, source, preview, cwd)
             VALUES (?1, 'deecodex', ?2, 'vscode', '桌面预览', '/tmp/desktop')",
            params!["desktop", rollout_path.to_string_lossy().to_string()],
        )
        .expect("insert desktop thread");
        conn.execute(
            "INSERT INTO threads (id, model_provider, rollout_path, source, preview, cwd)
             VALUES (?1, 'deecodex', ?2, 'cli', 'CLI 预览', '/tmp/cli')",
            params!["cli", cli_rollout_path.to_string_lossy().to_string()],
        )
        .expect("insert cli thread");
        drop(conn);

        let target_provider = current_thread_provider();
        let diff = do_normalize_desktop_threads(&db_path, &desktop_recent_backup_path(&dir))
            .expect("normalize desktop threads");
        assert_eq!(
            diff.changed_count,
            if target_provider == "deecodex" { 0 } else { 2 }
        );
        assert_eq!(
            diff.rollout_metadata_fixed_count,
            if target_provider == "deecodex" { 0 } else { 1 }
        );
        assert_eq!(diff.remaining_non_unified_count, 0);

        let conn = Connection::open(&db_path).expect("reopen db");
        let desktop_provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'desktop'",
                [],
                |row| row.get(0),
            )
            .expect("read desktop provider");
        let cli_provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'cli'",
                [],
                |row| row.get(0),
            )
            .expect("read cli provider");
        assert_eq!(desktop_provider, target_provider);
        assert_eq!(cli_provider, target_provider);

        let first_line = std::fs::read_to_string(&rollout_path)
            .expect("read desktop rollout")
            .lines()
            .next()
            .unwrap()
            .to_string();
        let meta: Value = serde_json::from_str(&first_line).expect("parse desktop meta");
        assert_eq!(
            meta["payload"]["model_provider"].as_str(),
            Some(target_provider.as_str())
        );
        let cli_meta: Value =
            serde_json::from_str(&std::fs::read_to_string(&cli_rollout_path).unwrap())
                .expect("parse cli meta");
        assert_eq!(
            cli_meta["payload"]["model_provider"].as_str(),
            Some("deecodex")
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn migrate_normalizes_legacy_managed_non_desktop_threads() {
        let dir = temp_test_dir("thread-managed-provider");
        let db_path = dir.join("state_test.sqlite");
        let target_provider = current_thread_provider();
        create_test_threads_db_with_rows(
            &db_path,
            &[
                (
                    "legacy-cli",
                    "deecodex",
                    "CLI",
                    "CLI 预览",
                    "用户消息",
                    "cli",
                    "/tmp/cli",
                    0,
                ),
                (
                    "legacy-subagent",
                    "deecodex_desktop",
                    "Subagent",
                    "Subagent 预览",
                    "用户消息",
                    r#"{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}}"#,
                    "/tmp/subagent",
                    0,
                ),
                (
                    "native-codex",
                    "codex",
                    "Codex",
                    "Codex 预览",
                    "用户消息",
                    "cli",
                    "/tmp/codex",
                    0,
                ),
            ],
        );

        let diff = do_normalize_desktop_threads(&db_path, &desktop_recent_backup_path(&dir))
            .expect("normalize managed threads");
        let expected_changed = if target_provider == "deecodex" { 1 } else { 2 };
        assert_eq!(diff.changed_count, expected_changed);
        assert_eq!(diff.remaining_non_unified_count, 0);

        let conn = Connection::open(&db_path).expect("open db");
        let legacy_count = query_count(
            &conn,
            "SELECT COUNT(*) FROM threads WHERE id IN ('legacy-cli', 'legacy-subagent') AND model_provider = ?1",
            rusqlite::params![target_provider],
        )
        .expect("count legacy");
        let codex_provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'native-codex'",
                [],
                |row| row.get(0),
            )
            .expect("read codex provider");
        assert_eq!(legacy_count, 2);
        assert_eq!(codex_provider, "codex");

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn calibrate_appends_new_threads_and_unifies_provider() {
        let dir = temp_test_dir("thread-calibrate");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let target_provider = current_thread_provider();
        create_test_threads_db(
            &db_path,
            &[
                ("kept", &target_provider),
                ("new-claude", "claude"),
                ("new-codex", "codex"),
            ],
        );
        std::fs::write(
            &backup_path,
            serde_json::to_string(&vec![
                ("kept".to_string(), "codex".to_string()),
                ("deleted".to_string(), "codex".to_string()),
            ])
            .unwrap(),
        )
        .unwrap();

        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");
        assert_eq!(diff.changed_count, 3);

        let after = get_provider_summary(&db_path).expect("provider summary");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].provider, target_provider);
        assert_eq!(after[0].count, 3);

        let backup_json = std::fs::read_to_string(&backup_path).expect("read backup");
        let mut backup: Vec<(String, String)> =
            serde_json::from_str(&backup_json).expect("parse backup");
        backup.sort();
        assert_eq!(
            backup,
            vec![
                ("kept".to_string(), "codex".to_string()),
                ("new-claude".to_string(), "claude".to_string()),
                ("new-codex".to_string(), "codex".to_string()),
            ]
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn migrate_normalizes_desktop_and_repairs_visibility() {
        let dir = temp_test_dir("thread-migrate-visibility");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let target_provider = current_thread_provider();
        let other_provider = if target_provider == "deecodex" {
            "dex_router"
        } else {
            "deecodex"
        };
        create_test_threads_db_with_rows(
            &db_path,
            &[
                (
                    "needs-preview",
                    other_provider,
                    "标题兜底",
                    "",
                    "首条用户消息",
                    "vscode",
                    "/tmp/a",
                    0,
                ),
                (
                    "keeps-source-cwd",
                    other_provider,
                    "已有标题",
                    "",
                    "",
                    "cli",
                    "/tmp/b",
                    0,
                ),
                (
                    "archived-preview",
                    other_provider,
                    "归档标题",
                    "",
                    "归档消息",
                    "vscode",
                    "/tmp/c",
                    1,
                ),
            ],
        );

        let diff = do_normalize_desktop_threads(&db_path, &desktop_recent_backup_path(&dir))
            .expect("migrate threads");
        assert_eq!(diff.changed_count, 3);
        assert_eq!(diff.visibility_fixed_count, 4);
        assert_eq!(diff.desktop_project_fixed_count, 0);
        assert_eq!(diff.desktop_recent_fixed_count, 0);
        assert_eq!(diff.remaining_non_unified_count, 0);
        assert_eq!(diff.cwd_aligned_count, 0);

        let conn = Connection::open(&db_path).expect("open db");
        let row: (String, String, i32, String, String) = conn
            .query_row(
                "SELECT model_provider, preview, has_user_event, source, cwd FROM threads WHERE id = 'needs-preview'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .expect("read row");
        assert_eq!(
            row,
            (
                target_provider.clone(),
                "首条用户消息".into(),
                1_i32,
                "vscode".into(),
                "/tmp/a".into()
            )
        );

        let keeps: (String, String, String) = conn
            .query_row(
                "SELECT model_provider, source, cwd FROM threads WHERE id = 'keeps-source-cwd'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read source cwd");
        assert_eq!(
            keeps,
            (target_provider.clone(), "cli".into(), "/tmp/b".into())
        );

        let archived: (String, String) = conn
            .query_row(
                "SELECT model_provider, cwd FROM threads WHERE id = 'archived-preview'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read archived cwd");
        assert_eq!(archived, (target_provider, "/tmp/c".into()));

        let stage1_count: usize = conn
            .query_row("SELECT COUNT(*) FROM stage1_outputs", [], |row| row.get(0))
            .expect("stage1 count");
        assert_eq!(stage1_count, 0);
        assert!(!backup_path.exists());
        assert!(!cwd_backup_path.exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn calibrate_without_backup_still_repairs_visibility() {
        let dir = temp_test_dir("thread-calibrate-visibility");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let target_provider = current_thread_provider();
        create_test_threads_db_with_rows(
            &db_path,
            &[(
                "already-unified",
                &target_provider,
                "标题兜底",
                "",
                "",
                "vscode",
                "/tmp/a",
                0,
            )],
        );

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");
        assert_eq!(diff.changed_count, 0);
        assert_eq!(diff.visibility_fixed_count, 2);
        assert_eq!(diff.cwd_aligned_count, 0);

        let conn = Connection::open(&db_path).expect("open db");
        let row: (String, i32, String) = conn
            .query_row(
                "SELECT preview, has_user_event, cwd FROM threads WHERE id = 'already-unified'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read row");
        assert_eq!(row, ("标题兜底".into(), 1, "/tmp/a".into()));
        assert!(!backup_path.exists());
        assert!(!cwd_backup_path.exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn calibrate_without_backup_unifies_remaining_provider() {
        let dir = temp_test_dir("thread-calibrate-provider");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let target_provider = current_thread_provider();
        create_test_threads_db_with_rows(
            &db_path,
            &[(
                "needs-unify",
                "openai",
                "已有预览",
                "已有预览",
                "用户消息",
                "vscode",
                "/tmp/a",
                1,
            )],
        );

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");
        assert_eq!(diff.changed_count, 1);

        let conn = Connection::open(&db_path).expect("open db");
        let provider: String = conn
            .query_row(
                "SELECT model_provider FROM threads WHERE id = 'needs-unify'",
                [],
                |row| row.get(0),
            )
            .expect("read provider");
        assert_eq!(provider, target_provider);
        assert!(backup_path.exists());

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn calibrate_preview_unifies_legacy_deecodex_to_current_provider() {
        let dir = temp_test_dir("thread-calibrate-preview-provider");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let target_provider = current_thread_provider();
        create_test_threads_db(
            &db_path,
            &[
                ("legacy-stable", "deecodex"),
                ("codex-native", "codex"),
                ("current", &target_provider),
            ],
        );

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");

        let after = get_provider_summary(&db_path).expect("provider summary");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].provider, target_provider);
        assert_eq!(after[0].count, 3);
        assert_eq!(
            diff.changed_count,
            if target_provider == "deecodex" { 1 } else { 2 }
        );

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn calibrate_repairs_codex_desktop_project_index() {
        let dir = temp_test_dir("thread-desktop-project-index");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let target_provider = current_thread_provider();
        create_test_threads_db_with_rows(
            &db_path,
            &[
                (
                    "thread-a",
                    &target_provider,
                    "标题 A",
                    "预览 A",
                    "",
                    "vscode",
                    "/tmp/project-a",
                    0,
                ),
                (
                    "thread-b",
                    &target_provider,
                    "标题 B",
                    "预览 B",
                    "",
                    "vscode",
                    "/tmp/project-a/sub",
                    0,
                ),
                (
                    "projectless",
                    &target_provider,
                    "标题 C",
                    "预览 C",
                    "",
                    "vscode",
                    "/tmp/other",
                    0,
                ),
            ],
        );
        let global_path = dir.join(".codex-global-state.json");
        std::fs::write(
            &global_path,
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": ["/tmp/project-a"],
                "project-order": [],
                "electron-workspace-root-labels": {},
                "projectless-thread-ids": ["thread-a", "projectless"]
            }))
            .unwrap(),
        )
        .unwrap();

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");
        assert!(diff.desktop_project_fixed_count >= 1);

        let state: Value = serde_json::from_str(
            &std::fs::read_to_string(&global_path).expect("read global state"),
        )
        .expect("parse global state");
        assert!(state["project-order"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some("/tmp/project-a")));
        assert_eq!(
            state["thread-project-assignments"]["thread-a"]["projectId"].as_str(),
            Some("/tmp/project-a")
        );
        assert_eq!(
            state["thread-project-assignments"]["thread-b"]["projectId"].as_str(),
            Some("/tmp/project-a")
        );
        assert!(state["projectless-thread-ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value.as_str() != Some("thread-a")));
        let order_ids = state["sidebar-project-thread-orders"]["/tmp/project-a"]["threadIds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(order_ids.contains(&"thread-a"));
        assert!(order_ids.contains(&"thread-b"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn desktop_project_index_uses_existing_sidebar_order_as_project_hint() {
        let dir = temp_test_dir("thread-desktop-project-order-hint");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let project = "/tmp/order-project";
        let target_provider = current_thread_provider();
        create_test_threads_db_with_rows(
            &db_path,
            &[(
                "ordered-thread",
                &target_provider,
                "标题",
                "预览",
                "",
                "vscode",
                "/tmp/moved-project/subdir",
                0,
            )],
        );
        let global_path = dir.join(".codex-global-state.json");
        let mut state = json!({
            "electron-saved-workspace-roots": [],
            "project-order": [],
            "electron-workspace-root-labels": {},
            "projectless-thread-ids": [],
            "sidebar-project-thread-orders": {}
        });
        state["sidebar-project-thread-orders"][project] =
            json!({ "threadIds": ["ordered-thread"] });
        std::fs::write(&global_path, serde_json::to_string(&state).unwrap())
            .expect("write global state");

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");
        assert!(diff.desktop_project_fixed_count >= 1);

        let state: Value = serde_json::from_str(
            &std::fs::read_to_string(&global_path).expect("read global state"),
        )
        .expect("parse global state");
        assert_eq!(
            state["thread-project-assignments"]["ordered-thread"]["projectId"].as_str(),
            Some(project)
        );
        assert!(state["active-workspace-roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some(project)));

        let conn = Connection::open(&db_path).expect("open db");
        let status = get_desktop_project_index_status(&db_path, &conn).expect("project status");
        assert_eq!(status.indexed_count, 1);
        assert_eq!(status.pending_count, 0);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn desktop_project_index_uses_existing_thread_cwd_as_project_fallback() {
        let dir = temp_test_dir("thread-desktop-project-cwd-fallback");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let project_dir = dir.join("real-project");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = project_dir.to_string_lossy().to_string();
        let target_provider = current_thread_provider();
        create_test_threads_db_with_rows(
            &db_path,
            &[(
                "cwd-thread",
                &target_provider,
                "标题",
                "预览",
                "",
                "vscode",
                &project,
                0,
            )],
        );
        let global_path = dir.join(".codex-global-state.json");
        std::fs::write(
            &global_path,
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [],
                "active-workspace-roots": [],
                "project-order": [],
                "electron-workspace-root-labels": {},
                "projectless-thread-ids": ["cwd-thread"],
                "sidebar-project-thread-orders": {}
            }))
            .unwrap(),
        )
        .expect("write global state");

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &desktop_recent_backup_path(&dir),
        )
        .expect("calibrate threads");
        assert_eq!(diff.desktop_project_pending_count, 0);

        let state: Value = serde_json::from_str(
            &std::fs::read_to_string(&global_path).expect("read global state"),
        )
        .expect("parse global state");
        assert_eq!(
            state["thread-project-assignments"]["cwd-thread"]["projectId"].as_str(),
            Some(project.as_str())
        );
        assert!(state["active-workspace-roots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some(project.as_str())));
        assert!(
            state["sidebar-project-thread-orders"][&project]["threadIds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str() == Some("cwd-thread"))
        );
        assert!(state["projectless-thread-ids"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value.as_str() != Some("cwd-thread")));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn calibrate_reports_desktop_project_threads_outside_recent_without_mutating_timestamps() {
        let dir = temp_test_dir("thread-desktop-recent");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        let cwd_backup_path = dir.join("thread_cwd_visibility_backup.json");
        let recent_backup_path = desktop_recent_backup_path(&dir);
        let project_dir = dir
            .join(".codex")
            .join("worktrees")
            .join("c7e2")
            .join("deecodex")
            .join("deecodex-gui");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let project = project_dir.to_string_lossy().to_string();

        let conn = Connection::open(&db_path).expect("open test db");
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                preview TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                thread_source TEXT,
                source TEXT NOT NULL DEFAULT 'vscode',
                cwd TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                has_user_event INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER,
                created_at INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER,
                git_sha TEXT,
                git_branch TEXT,
                git_origin_url TEXT
            )",
            [],
        )
        .expect("create threads");
        conn.execute(
            "CREATE TABLE stage1_outputs (thread_id TEXT, rollout_summary TEXT, rollout_slug TEXT)",
            [],
        )
        .expect("create stage1");
        for idx in 0..55 {
            let id = format!("recent-{idx:02}");
            conn.execute(
                "INSERT INTO threads
                 (id, model_provider, title, preview, source, cwd, updated_at, updated_at_ms, created_at, created_at_ms)
                 VALUES (?1, 'deecodex', ?2, '预览', 'vscode', '/tmp/other', ?3, ?4, ?3, ?4)",
                params![id, format!("最近 {idx}"), 2_000_i64 - idx, (2_000_i64 - idx) * 1000],
            )
            .expect("insert recent");
        }
        conn.execute(
            "INSERT INTO threads
             (id, model_provider, title, preview, source, cwd, updated_at, updated_at_ms, created_at, created_at_ms)
             VALUES ('project-old', 'deecodex', '旧项目线程', '预览', 'vscode', ?1, 100, 100000, 100, 100000)",
            params![project],
        )
        .expect("insert project thread");
        conn.execute(
            "INSERT INTO threads
             (id, model_provider, title, preview, source, cwd, updated_at, updated_at_ms, created_at, created_at_ms)
             VALUES ('project-new', 'deecodex', '新项目线程', '预览', 'vscode', ?1, 1995, 1995000, 1995, 1995000)",
            params![project],
        )
        .expect("insert visible project thread");
        drop(conn);

        std::fs::write(
            dir.join(".codex-global-state.json"),
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [project],
                "project-order": [],
                "electron-workspace-root-labels": {},
                "projectless-thread-ids": []
            }))
            .unwrap(),
        )
        .expect("write global state");

        let conn = Connection::open(&db_path).expect("open db");
        assert!(
            !desktop_recent_thread_ids(&conn, DESKTOP_RECENT_LOAD_WINDOW)
                .expect("recent before")
                .contains("project-old")
        );
        drop(conn);

        let diff = do_calibrate(
            &db_path,
            &backup_path,
            &cwd_backup_path,
            &recent_backup_path,
        )
        .expect("calibrate threads");
        assert_eq!(diff.desktop_recent_fixed_count, 0);
        assert_eq!(diff.desktop_recent_pending_count, 1);

        let conn = Connection::open(&db_path).expect("open db after");
        let recent_after =
            desktop_recent_thread_ids(&conn, DESKTOP_RECENT_LOAD_WINDOW).expect("recent after");
        assert!(!recent_after.contains("project-old"));
        assert!(recent_after.contains("project-new"));
        let updated_at: i64 = conn
            .query_row(
                "SELECT updated_at FROM threads WHERE id = 'project-old'",
                [],
                |row| row.get(0),
            )
            .expect("read updated");
        assert_eq!(updated_at, 100);
        assert!(!recent_backup_path.exists());

        std::fs::remove_dir_all(dir).ok();
    }
}
