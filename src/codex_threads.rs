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
use std::path::{Path, PathBuf};

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
            search_dirs.push(PathBuf::from(&local_appdata).join("anthropic").join("Codex"));
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
    let bp = backup_path(data_dir);

    let before = get_provider_summary(&db_path)?;

    if !bp.exists() {
        return Ok(MigrationDiff {
            before: before.clone(),
            after: before,
            changed_count: 0,
        });
    }

    // 读取当前备份
    let backup_json = std::fs::read_to_string(&bp).context("读取迁移备份文件失败")?;
    let mut originals: Vec<(String, String)> =
        serde_json::from_str(&backup_json).context("解析迁移备份文件失败")?;

    // 获取当前所有线程 ID
    let conn = Connection::open(&db_path)?;
    conn.pragma_update(None, "busy_timeout", "5000")?;
    let mut stmt = conn.prepare("SELECT id FROM threads")?;
    let current_ids: std::collections::HashSet<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()?;
    drop(stmt);

    let old_len = originals.len();
    // 移除已删除的线程
    originals.retain(|(id, _)| current_ids.contains(id));

    let removed = old_len - originals.len();
    if removed > 0 {
        tracing::info!("校准: 从备份中移除 {removed} 条已删除线程");
    }

    let new_backup = serde_json::to_string_pretty(&originals).context("序列化校准备份失败")?;
    std::fs::write(&bp, new_backup).context("写入校准备份文件失败")?;

    let after = get_provider_summary(&db_path)?;

    Ok(MigrationDiff {
        before,
        after,
        changed_count: removed,
    })
}

/// 获取指定线程的完整内容（含元数据、摘要、工具）。
#[allow(dead_code)]
pub fn get_thread_content(thread_id: &str) -> Result<serde_json::Value> {
    let home = crate::config::home_dir().context("无法确定 HOME 目录")?;
    let db_path = find_state_db(&home).context("未找到 Codex state SQLite")?;
    let conn = Connection::open(&db_path)?;

    // 1. 线程元数据
    let mut stmt = conn.prepare(
        "SELECT id, title, model_provider, model, reasoning_effort,
                first_user_message, created_at_ms, updated_at_ms,
                archived, cwd, git_sha, git_branch, agent_nickname, cli_version
         FROM threads WHERE id = ?1",
    )?;
    let mut thread = stmt.query_row(rusqlite::params![thread_id], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, String>(0)?,
            "title": row.get::<_, String>(1)?,
            "model_provider": row.get::<_, String>(2)?,
            "model": row.get::<_, Option<String>>(3)?,
            "reasoning_effort": row.get::<_, Option<String>>(4)?,
            "first_user_message": row.get::<_, String>(5)?,
            "created_at_ms": row.get::<_, Option<i64>>(6)?,
            "updated_at_ms": row.get::<_, Option<i64>>(7)?,
            "archived": row.get::<_, i32>(8)? != 0,
            "cwd": row.get::<_, String>(9)?,
            "git_sha": row.get::<_, Option<String>>(10)?,
            "git_branch": row.get::<_, Option<String>>(11)?,
            "agent_nickname": row.get::<_, Option<String>>(12)?,
            "cli_version": row.get::<_, String>(13)?,
        }))
    })?;
    drop(stmt);

    let mut messages: Vec<serde_json::Value> = Vec::new();

    // 2. 首条用户消息
    if let Some(first_msg) = thread.get("first_user_message").and_then(|v| v.as_str()) {
        if !first_msg.is_empty() {
            messages.push(serde_json::json!({
                "role": "user",
                "payload": { "role": "user", "content": [{ "type": "input_text", "text": first_msg }] }
            }));
        }
    }

    // 3. stage1_outputs 摘要
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

    // 将摘要作为 assistant 消息
    if let Some(ref summary) = rollout_summary {
        if !summary.is_empty() {
            messages.push(serde_json::json!({
                "role": "assistant",
                "payload": { "role": "assistant", "content": [{ "type": "output_text", "text": summary }] }
            }));
        }
    }

    // 4. 线程关联的工具
    if let Ok(mut stmt) = conn.prepare(
        "SELECT name, description FROM thread_dynamic_tools WHERE thread_id = ?1 ORDER BY position",
    ) {
        if let Ok(tools) = stmt
            .query_map(rusqlite::params![thread_id], |row| {
                Ok(serde_json::json!({
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
