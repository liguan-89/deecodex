//! Codex 线程聚合模块。
//!
//! 读取 Codex 本地 state_*.sqlite 中 threads 表，提供跨 provider 的会话聚合功能。
//! 支持「迁移」：将所有非 deecodex 线程的 model_provider 改为 "deecodex"，
//! 以及「还原」：从备份恢复原始 model_provider 值。
//!
//! 迁移前自动备份，还原后自动清理备份。全程不破坏 Codex 原有数据。

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

const MAX_ROLLOUT_MESSAGE_CHARS: usize = 24_000;
const MAX_ROLLOUT_TOTAL_CHARS: usize = 1_500_000;

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

/// 迁移/还原前后的差异对比。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationDiff {
    pub before: Vec<ProviderSummary>,
    pub after: Vec<ProviderSummary>,
    pub changed_count: usize,
}

/// 是否已有迁移备份（即迁移操作已执行过）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadStatus {
    pub summary: Vec<ProviderSummary>,
    pub total: usize,
    pub migrated: bool,
    pub non_deecodex_count: usize,
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

        let mut candidates: Vec<_> = std::fs::read_dir(codex_dir)
            .ok()?
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
                    let size = path.metadata().ok()?.len();
                    Some((path, size))
                } else {
                    None
                }
            })
            .collect();

        if !candidates.is_empty() {
            candidates.sort_by_key(|b| std::cmp::Reverse(b.1));
            let found = candidates.into_iter().next().map(|(p, _)| p);
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

/// 备份文件路径（存在 deecodex data_dir 下）。
pub fn backup_path(data_dir: &Path) -> PathBuf {
    data_dir.join("thread_migration_backup.json")
}

/// 获取当前状态：各 provider 线程数、是否已迁移。
pub fn status(data_dir: &Path) -> Result<ThreadStatus> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;

    let summary = get_provider_summary(&db_path)?;
    let total: usize = summary.iter().map(|s| s.count).sum();
    let non_deecodex_count: usize = summary
        .iter()
        .filter(|s| s.provider != "deecodex")
        .map(|s| s.count)
        .sum();
    let migrated = backup_path(data_dir).exists();

    Ok(ThreadStatus {
        summary,
        total,
        migrated,
        non_deecodex_count,
    })
}

/// 列出所有线程（不过滤 provider）。
pub fn list_all() -> Result<Vec<ThreadInfo>> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    list_threads(&db_path)
}

/// 迁移：将所有非 deecodex 线程的 model_provider 改为 "deecodex"。
/// 迁移前自动备份原始值到 `data_dir/thread_migration_backup.json`。
pub fn migrate(data_dir: &Path) -> Result<MigrationDiff> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;

    do_migrate(&db_path, &backup_path(data_dir))
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

    do_restore(&db_path, &bp)
}

/// 校准迁移备份：移除已删除的线程，追加新增的非 deecodex 线程。
#[allow(dead_code)]
pub fn calibrate(data_dir: &Path) -> Result<MigrationDiff> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    do_calibrate(&db_path, &backup_path(data_dir))
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
    let conn = Connection::open(&db_path)?;
    conn.pragma_update(None, "busy_timeout", "5000")?;

    let affected = conn
        .execute(
            "DELETE FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
        )
        .context("删除线程失败")?;

    if affected == 0 {
        anyhow::bail!("未找到线程 {thread_id}");
    }

    // 同时从迁移备份中移除
    let bp = backup_path(data_dir);
    if bp.exists() {
        if let Ok(json) = std::fs::read_to_string(&bp) {
            if let Ok(mut originals) = serde_json::from_str::<Vec<(String, String)>>(&json) {
                let before = originals.len();
                originals.retain(|(id, _)| id != thread_id);
                if originals.len() != before {
                    let new_json =
                        serde_json::to_string_pretty(&originals).context("序列化备份失败")?;
                    std::fs::write(&bp, new_json).context("写入备份文件失败")?;
                }
            }
        }
    }

    tracing::info!("已永久删除线程 {thread_id}");
    Ok(())
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

fn do_migrate(db_path: &Path, backup_path: &Path) -> Result<MigrationDiff> {
    let before = get_provider_summary(db_path)?;

    let conn = Connection::open(db_path)?;
    // 设置 busy timeout 以应对 Codex 持有的 WAL 写锁
    conn.pragma_update(None, "busy_timeout", "5000")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;

    let mut stmt =
        conn.prepare("SELECT id, model_provider FROM threads WHERE model_provider != 'deecodex'")?;
    let new_originals: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(stmt);

    if new_originals.is_empty() {
        return Ok(MigrationDiff {
            before: before.clone(),
            after: before,
            changed_count: 0,
        });
    }

    // 与已有备份合并（追加新线程，保留旧备份中的原始 provider）
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
    for (id, provider) in new_originals {
        merged.entry(id).or_insert(provider);
    }
    let merged_vec: Vec<(String, String)> = merged.into_iter().collect();

    let backup_json = serde_json::to_string_pretty(&merged_vec).context("序列化迁移备份失败")?;
    std::fs::write(backup_path, backup_json).context("写入迁移备份文件失败")?;

    // 逐条 UPDATE 以避免 WAL 锁冲突导致整批失败
    let mut changed = 0usize;
    for (id, _) in &merged_vec {
        match conn.execute(
            "UPDATE threads SET model_provider = 'deecodex' WHERE id = ?1 AND model_provider != 'deecodex'",
            rusqlite::params![id],
        ) {
            Ok(n) => changed += n,
            Err(e) => {
                tracing::warn!("迁移线程 {id} 失败: {e}");
            }
        }
    }

    let after = get_provider_summary(db_path)?;

    let skipped = merged_vec.len().saturating_sub(changed);
    if skipped > 0 {
        tracing::warn!("迁移: {changed} 条成功, {skipped} 条因锁冲突跳过，下次迁移会自动重试");
    } else {
        tracing::info!(
            "已迁移 {changed} 条线程到 deecodex，备份条目数 {}",
            merged_vec.len()
        );
    }

    Ok(MigrationDiff {
        before,
        after,
        changed_count: changed,
    })
}

fn do_calibrate(db_path: &Path, backup_path: &Path) -> Result<MigrationDiff> {
    let before = get_provider_summary(db_path)?;

    if !backup_path.exists() {
        return Ok(MigrationDiff {
            before: before.clone(),
            after: before,
            changed_count: 0,
        });
    }

    let backup_json = std::fs::read_to_string(backup_path).context("读取迁移备份文件失败")?;
    let mut originals: Vec<(String, String)> =
        serde_json::from_str(&backup_json).context("解析迁移备份文件失败")?;

    let conn = Connection::open(db_path)?;
    conn.pragma_update(None, "busy_timeout", "5000")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;

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
        .filter(|(id, provider)| provider != "deecodex" && !backed_up_ids.contains(id))
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
            "UPDATE threads SET model_provider = 'deecodex' WHERE id = ?1 AND model_provider != 'deecodex'",
            rusqlite::params![id],
        ) {
            Ok(n) => migrated += n,
            Err(e) => {
                tracing::warn!("校准线程 {id} 失败: {e}");
            }
        }
    }

    let after = get_provider_summary(db_path)?;
    let skipped = originals.len().saturating_sub(migrated);
    if migrated > 0 || removed > 0 {
        tracing::info!("已校准 {migrated} 条线程到 deecodex，清理 {removed} 条备份记录");
    }
    if skipped > 0 && after.iter().any(|s| s.provider != "deecodex") {
        tracing::warn!("校准: {migrated} 条成功，仍有线程未统一，下次校准会自动重试");
    }

    Ok(MigrationDiff {
        before,
        after,
        changed_count: migrated + removed,
    })
}

fn do_restore(db_path: &Path, backup_path: &Path) -> Result<MigrationDiff> {
    let before = get_provider_summary(db_path)?;

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

    let after = get_provider_summary(db_path)?;

    Ok(MigrationDiff {
        before,
        after,
        changed_count: restored,
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
        let conn = Connection::open(path).expect("open test db");
        conn.execute(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT NOT NULL)",
            [],
        )
        .expect("create threads table");
        for (id, provider) in threads {
            conn.execute(
                "INSERT INTO threads (id, model_provider) VALUES (?1, ?2)",
                params![id, provider],
            )
            .expect("insert thread");
        }
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
    fn calibrate_appends_new_threads_and_unifies_provider() {
        let dir = temp_test_dir("thread-calibrate");
        let db_path = dir.join("state_test.sqlite");
        let backup_path = dir.join("thread_migration_backup.json");
        create_test_threads_db(
            &db_path,
            &[
                ("kept", "deecodex"),
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

        let diff = do_calibrate(&db_path, &backup_path).expect("calibrate threads");
        assert_eq!(diff.changed_count, 3);

        let after = get_provider_summary(&db_path).expect("provider summary");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].provider, "deecodex");
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
}
