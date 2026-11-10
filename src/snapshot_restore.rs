//! Codex 线程快照恢复模块（macOS 专属）。
//!
//! 背景：Codex（ChatGPT 客户端）升级时会重建 `~/.codex/state_*.sqlite`，
//! 导致历史线程从本地库消失。若用户开启了 Time Machine（含本地快照），
//! 升级前的完整数据保存在 APFS 本地快照中。
//!
//! 本模块提供：
//! - `list_snapshots()`：列出可用的 Time Machine 本地快照。
//! - `preview_restore()`：挂载快照、统计差异（仅读，不写入）。
//! - `execute_restore()`：备份当前库 → 合并历史线程 → 复制 rollout 文件
//!   → 合并 session_index → 校验完整性。
//!
//! 合并策略：快照库（旧版表结构，无 `project_id` 列）→ 当前库（新版结构），
//! 只搬数据、保留当前库表骨架；`INSERT OR IGNORE` 按主键去重，零冲突。
//!
//! 非 macOS 平台该模块为空壳（所有函数返回"平台不支持"错误）。

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Time Machine 本地快照信息。
#[derive(Debug, Clone, Serialize)]
pub struct TmSnapshot {
    /// 快照名，如 `com.apple.TimeMachine.2026-08-20-125305.local`。
    pub name: String,
    /// 快照时间（本地时间，来自快照名解析）。
    pub created_at: String,
    /// 快照对应的数据卷设备（如 `disk3s5`）。
    pub device: String,
}

/// 快照挂载点（临时目录）。
fn mount_root() -> PathBuf {
    std::env::temp_dir().join("deecodex-snapshot-restore")
}

/// 解析 `tmutil listlocalsnapshots <path>` 的输出，返回快照名列表。
///
/// 输出格式：
/// ```text
/// Snapshots for disk /:
/// com.apple.TimeMachine.2026-08-20-071301.local
/// com.apple.TimeMachine.2026-08-20-125305.local
/// ```
fn parse_tmutil_snapshots(output: &str, device: &str) -> Vec<TmSnapshot> {
    let mut snaps = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Snapshots for disk") {
            continue;
        }
        // 兼容带 `@/dev/...` 后缀的格式：`name@/dev/disk3s5`
        let name = line.split('@').next().unwrap_or(line).trim().to_string();
        if !name.starts_with("com.apple.TimeMachine.") || !name.ends_with(".local") {
            continue;
        }
        let created_at = name
            .trim_start_matches("com.apple.TimeMachine.")
            .trim_end_matches(".local")
            .to_string();
        // 格式 YYYY-MM-DD-HHMMSS → YYYY-MM-DD HH:MM
        let created_at = if created_at.len() == 17 {
            format!(
                "{}-{}-{} {}:{}",
                &created_at[0..4],
                &created_at[5..7],
                &created_at[8..10],
                &created_at[11..13],
                &created_at[13..15]
            )
        } else {
            created_at
        };
        snaps.push(TmSnapshot {
            name,
            created_at,
            device: device.to_string(),
        });
    }
    snaps.sort_by(|a, b| b.name.cmp(&a.name));
    snaps
}

/// 列出 Time Machine 本地快照。
///
/// 扫描数据卷（`/System/Volumes/Data`）与根卷的快照。
#[cfg(target_os = "macos")]
pub fn list_snapshots() -> Result<Vec<TmSnapshot>> {
    let mut all = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // 数据卷：Codex 的 `~/.codex` 位于数据卷。
    for mount in ["/System/Volumes/Data", "/"] {
        let output = Command::new("tmutil")
            .arg("listlocalsnapshots")
            .arg(mount)
            .output()
            .with_context(|| format!("执行 tmutil listlocalsnapshots {mount} 失败"))?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let device = if mount == "/System/Volumes/Data" {
            "disk3s5"
        } else {
            "disk3s1"
        };
        for snap in parse_tmutil_snapshots(&text, device) {
            if seen.insert(snap.name.clone()) {
                all.push(snap);
            }
        }
    }
    if all.is_empty() {
        bail!("未找到 Time Machine 本地快照（可能未开启 Time Machine 或尚无快照）");
    }
    Ok(all)
}

/// 非 macOS：不支持。
#[cfg(not(target_os = "macos"))]
pub fn list_snapshots() -> Result<Vec<TmSnapshot>> {
    bail!("快照恢复仅支持 macOS")
}

/// 生成快照挂载点目录：`<temp>/deecodex-snapshot-restore/<name>`。
fn snapshot_mountpoint(snap: &TmSnapshot) -> PathBuf {
    mount_root().join(&snap.name)
}

/// 挂载快照（只读）到临时目录。
///
/// 使用 `mount_apfs -o ro -s <name> <device> <mountpoint>`。
/// 已挂载时幂等返回（目录内容可用即视为成功）。
#[cfg(target_os = "macos")]
pub fn mount_snapshot(snap: &TmSnapshot) -> Result<PathBuf> {
    let mountpoint = snapshot_mountpoint(snap);
    if !mountpoint.exists() {
        std::fs::create_dir_all(&mountpoint)
            .with_context(|| format!("创建挂载点 {} 失败", mountpoint.display()))?;
    }
    let output = Command::new("mount_apfs")
        .args(["-o", "ro", "-s", &snap.name, &snap.device])
        .arg(&mountpoint)
        .output()
        .with_context(|| format!("执行 mount_apfs 失败（快照 {}", snap.name))?;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        tracing::info!(snapshot = %snap.name, mountpoint = %mountpoint.display(), "快照挂载成功");
        return Ok(mountpoint);
    }
    // 已挂载（Resource busy）时视为成功：检查挂载点内容是否可读。
    if stderr.contains("Resource busy") || stderr.contains("already mounted") {
        let probe = mountpoint.join("Users");
        if probe.is_dir() {
            tracing::info!(snapshot = %snap.name, mountpoint = %mountpoint.display(), "快照已挂载，复用现有挂载");
            return Ok(mountpoint);
        }
    }
    bail!("挂载快照失败: {stderr}")
}

/// 卸载快照挂载点。
#[cfg(target_os = "macos")]
pub fn unmount_snapshot(mountpoint: &Path) {
    let output = Command::new("umount").arg(mountpoint).output();
    match output {
        Ok(o) if o.status.success() => {
            tracing::info!(mountpoint = %mountpoint.display(), "快照挂载已卸载");
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr).to_string();
            tracing::warn!(mountpoint = %mountpoint.display(), "卸载快照失败: {stderr}");
        }
        Err(e) => {
            tracing::warn!(mountpoint = %mountpoint.display(), "卸载快照失败: {e}");
        }
    }
}

/// 快照恢复预览（只读统计，不写入）。
#[derive(Debug, Clone, Serialize)]
pub struct RestorePreview {
    /// 快照库线程总数。
    pub snapshot_threads: usize,
    /// 快照中归档线程数。
    pub snapshot_archived: usize,
    /// 快照 sessions 目录 rollout 文件数。
    pub snapshot_rollouts: usize,
    /// 快照 archived_sessions 文件数。
    pub snapshot_archived_rollouts: usize,
    /// 快照 rollout 总体积（字节）。
    pub snapshot_rollout_bytes: u64,
    /// 将新增线程数（快照有、当前库没有）。
    pub new_threads: usize,
    /// 将新增 rollout 文件数（快照有、当前没有）。
    pub new_rollouts: usize,
    /// 将新增索引条数。
    pub new_index_entries: usize,
    /// 快照库中 `~/.codex` 路径（挂载点下）。
    pub snapshot_codex_path: String,
}

/// 定位快照挂载点下的 `~/.codex` 目录。
fn snapshot_codex_dir(mountpoint: &Path) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME 环境变量未设置，无法定位快照 codex 目录");
    // 快照内容是数据卷根，HOME 相对路径为 `Users/<user>`。
    let rel = home.trim_start_matches('/');
    mountpoint.join(rel)
}

/// 快照库路径候选（state_*.sqlite）。
fn find_snapshot_state_db(codex_dir: &Path) -> Result<PathBuf> {
    let entries = std::fs::read_dir(codex_dir)
        .with_context(|| format!("读取快照目录 {} 失败", codex_dir.display()))?;
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.starts_with("state_") && name.ends_with(".sqlite")).then_some(name)
        })
        .collect();
    names.sort();
    names
        .pop()
        .map(|n| codex_dir.join(n))
        .with_context(|| format!("快照 {} 中未找到 state_*.sqlite", codex_dir.display()))
}

/// 打开只读 SQLite（复制 db+wal+shm 到可写临时目录，避免只读卷 WAL 问题）。
fn open_snapshot_db_ro(db_path: &Path) -> Result<(PathBuf, rusqlite::Connection)> {
    let tmp = std::env::temp_dir().join(format!(
        "deecodex-snap-db-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&tmp)?;
    let copy = tmp.join("state.sqlite");
    std::fs::copy(db_path, &copy)
        .with_context(|| format!("复制快照库 {} 失败", db_path.display()))?;
    // 复制 WAL/SHM（若存在），保证数据完整性。
    for suffix in ["-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        if src.exists() {
            let _ = std::fs::copy(&src, tmp.join(format!("state.sqlite{suffix}")));
        }
    }
    let conn = rusqlite::Connection::open_with_flags(
        &copy,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("打开快照库副本 {} 失败", copy.display()))?;
    Ok((tmp, conn))
}

/// 统计快照库线程数与归档数。
fn snapshot_thread_stats(conn: &rusqlite::Connection) -> Result<(usize, usize)> {
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))?;
    let archived: i64 = conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE archived = 1",
        [],
        |r| r.get(0),
    )?;
    Ok((total as usize, archived as usize))
}

/// 统计目录下文件数与总体积。
fn dir_file_stats(dir: &Path) -> Result<(usize, u64)> {
    if !dir.exists() {
        return Ok((0, 0));
    }
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in walkdir_files(dir) {
        count += 1;
        if let Ok(meta) = std::fs::metadata(&entry) {
            bytes += meta.len();
        }
    }
    Ok((count, bytes))
}

/// 递归收集文件路径（不依赖 walkdir crate）。
fn walkdir_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

/// 当前库已存在的线程 id 集合。
fn current_thread_ids(home: &Path) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    let Some(db) = find_current_state_db(home) else {
        return ids;
    };
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return ids;
    };
    let Ok(mut stmt) = conn.prepare("SELECT id FROM threads") else {
        return ids;
    };
    if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
        for row in rows.flatten() {
            ids.insert(row);
        }
    }
    ids
}

/// 查找当前 `~/.codex` 的 state_*.sqlite。
fn find_current_state_db(home: &Path) -> Option<PathBuf> {
    let codex_dir = home.join(".codex");
    let Ok(entries) = std::fs::read_dir(&codex_dir) else {
        return None;
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.starts_with("state_") && name.ends_with(".sqlite")).then_some(name)
        })
        .collect();
    names.sort();
    names.pop().map(|n| codex_dir.join(n))
}

/// 当前 rollout 文件集合（sessions/ + archived_sessions/ 的文件名）。
fn current_rollout_names(home: &Path) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    let codex_dir = home.join(".codex");
    for sub in ["sessions", "archived_sessions"] {
        let dir = codex_dir.join(sub);
        if !dir.exists() {
            continue;
        }
        for path in walkdir_files(&dir) {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.insert(name.to_string());
            }
        }
    }
    names
}

/// 当前 session_index.jsonl 的 id 集合。
fn current_index_ids(home: &Path) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    let path = home.join(".codex/session_index.jsonl");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return ids;
    };
    for line in content.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

/// 预览从快照恢复的效果（只读，不写入任何数据）。
#[cfg(target_os = "macos")]
pub fn preview_restore(snap: &TmSnapshot) -> Result<RestorePreview> {
    let mountpoint = mount_snapshot(snap)?;
    let codex_dir = snapshot_codex_dir(&mountpoint);
    let db_path = find_snapshot_state_db(&codex_dir)
        .with_context(|| format!("快照 {} 中未找到 state 库", snap.name))?;
    let (tmp, conn) = open_snapshot_db_ro(&db_path)?;
    let (total, archived) = snapshot_thread_stats(&conn)?;
    drop(conn);
    let _ = std::fs::remove_dir_all(&tmp);

    let (rollouts, rollout_bytes) = dir_file_stats(&codex_dir.join("sessions"))?;
    let (archived_rollouts, _) = dir_file_stats(&codex_dir.join("archived_sessions"))?;

    let home = std::env::var("HOME").expect("HOME 环境变量未设置，无法定位快照数据目录");
    let home_path = Path::new(&home);
    let cur_ids = current_thread_ids(home_path);
    let cur_rollouts = current_rollout_names(home_path);
    let cur_index = current_index_ids(home_path);

    let new_threads = count_new_thread_ids(&db_path, &cur_ids)?;
    let new_rollouts = count_new_rollouts(&codex_dir, &cur_rollouts);
    let new_index_entries = count_new_index_entries(&codex_dir, &cur_index);

    // 卸载前先完成统计；unmount 失败仅告警。
    unmount_snapshot(&mountpoint);

    Ok(RestorePreview {
        snapshot_threads: total,
        snapshot_archived: archived,
        snapshot_rollouts: rollouts,
        snapshot_archived_rollouts: archived_rollouts,
        snapshot_rollout_bytes: rollout_bytes,
        new_threads,
        new_rollouts,
        new_index_entries,
        snapshot_codex_path: codex_dir.display().to_string(),
    })
}

/// 非 macOS：不支持。
#[cfg(not(target_os = "macos"))]
pub fn preview_restore(_snap: &TmSnapshot) -> Result<RestorePreview> {
    bail!("快照恢复仅支持 macOS")
}

/// 统计快照库中当前库没有的线程 id 数量。
fn count_new_thread_ids(db_path: &Path, cur_ids: &std::collections::HashSet<String>) -> Result<usize> {
    let (tmp, conn) = open_snapshot_db_ro(db_path)?;
    let mut count = 0usize;
    {
        let mut stmt = conn.prepare("SELECT id FROM threads")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows.flatten() {
            if !cur_ids.contains(&row) {
                count += 1;
            }
        }
    }
    drop(conn);
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(count)
}

/// 统计快照 rollout 中当前缺失的文件数。
fn count_new_rollouts(codex_dir: &Path, cur_rollouts: &std::collections::HashSet<String>) -> usize {
    let mut count = 0usize;
    for sub in ["sessions", "archived_sessions"] {
        let dir = codex_dir.join(sub);
        if !dir.exists() {
            continue;
        }
        for path in walkdir_files(&dir) {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if !cur_rollouts.contains(name) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// 统计快照 session_index 中当前缺失的条目数。
fn count_new_index_entries(codex_dir: &Path, cur_index: &std::collections::HashSet<String>) -> usize {
    let path = codex_dir.join("session_index.jsonl");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return 0;
    };
    let mut count = 0usize;
    for line in content.lines() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                if !cur_index.contains(id) {
                    count += 1;
                }
            }
        }
    }
    count
}

/// 恢复结果。
#[derive(Debug, Clone, Serialize)]
pub struct RestoreResult {
    /// 当前库线程总数（恢复后）。
    pub threads_total: usize,
    /// 本次新增线程数。
    pub new_threads: usize,
    /// 本次新增 rollout 文件数。
    pub new_rollouts: usize,
    /// 本次新增索引条目数。
    pub new_index_entries: usize,
    /// 恢复后的归档线程数。
    pub archived_total: usize,
    /// 备份路径（恢复前当前库备份）。
    pub backup_path: String,
    /// rollout 文件引用完整性校验（缺失数，0 = 全部存在）。
    pub missing_rollouts: usize,
}

/// 执行快照恢复：备份 → 合并库 → 复制 rollout → 合并索引 → 校验。
#[cfg(target_os = "macos")]
pub fn execute_restore(snap: &TmSnapshot) -> Result<RestoreResult> {
    let mountpoint = mount_snapshot(snap)?;
    let codex_dir = snapshot_codex_dir(&mountpoint);
    let db_path = find_snapshot_state_db(&codex_dir)?;

    let home = std::env::var("HOME").expect("HOME 环境变量未设置，无法定位快照数据目录");
    let home_path = Path::new(&home);
    let codex_home = home_path.join(".codex");

    // 1. 备份当前库到 recovery。
    let backup_dir = home_path
        .join(".deecodex/recovery")
        .join(format!("snapshot-restore-{}", snap.created_at.replace([' ', ':'], "-")));
    std::fs::create_dir_all(&backup_dir)?;
    let cur_db = find_current_state_db(home_path)
        .with_context(|| "未找到当前 ~/.codex/state_*.sqlite")?;
    let backup_path = backup_dir.join("state_5.sqlite.pre-restore");
    backup_current_db(&cur_db, &backup_path)?;
    tracing::info!(backup = %backup_path.display(), "恢复前已备份当前库");

    // 2. 合并快照库线程 → 当前库。
    let (merged_threads, new_threads, archived_total) = merge_snapshot_db(&db_path, &cur_db)?;

    // 3. 复制 rollout 文件（跳过已存在）。
    let new_rollouts = copy_rollouts(&codex_dir, &codex_home)?;

    // 4. 合并 session_index.jsonl。
    let new_index_entries = merge_session_index(&codex_dir, &codex_home)?;

    // 5. 校验 rollout 引用完整性。
    let missing_rollouts = verify_rollout_refs(&cur_db)?;

    unmount_snapshot(&mountpoint);

    Ok(RestoreResult {
        threads_total: merged_threads,
        new_threads,
        new_rollouts,
        new_index_entries,
        archived_total,
        backup_path: backup_path.display().to_string(),
        missing_rollouts,
    })
}

/// 非 macOS：不支持。
#[cfg(not(target_os = "macos"))]
pub fn execute_restore(_snap: &TmSnapshot) -> Result<RestoreResult> {
    bail!("快照恢复仅支持 macOS")
}

/// 备份当前库（在线 .backup，避免 WAL 不一致）。
fn backup_current_db(db_path: &Path, backup_path: &Path) -> Result<()> {
    let conn = rusqlite::Connection::open(db_path)?;
    conn.backup(rusqlite::DatabaseName::Main, backup_path, None)
        .with_context(|| format!("备份 {} 到 {} 失败", db_path.display(), backup_path.display()))?;
    Ok(())
}

/// 合并快照库数据到当前库。
///
/// 返回 `(合并后线程总数, 新增线程数, 归档线程总数)`。
fn merge_snapshot_db(snapshot_db: &Path, current_db: &Path) -> Result<(usize, usize, usize)> {
    let mut conn = rusqlite::Connection::open(current_db)?;
    conn.pragma_update(None, "busy_timeout", "15000")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;

    let before_total: i64 = conn.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))?;

    // 只读打开快照库副本，再 ATTACH 到当前库连接。
    let (tmp, _snap_conn) = open_snapshot_db_ro(snapshot_db)?;
    let snap_copy = tmp.join("state.sqlite");
    conn.execute(&format!("ATTACH DATABASE '{}' AS snap", snap_copy.display()), [])?;

    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // thread_sections（先插父表，满足外键）。
    tx.execute(
        "INSERT OR IGNORE INTO thread_sections (id, name) SELECT id, name FROM snap.thread_sections",
        [],
    )?;

    // threads：快照库旧版结构无 project_id 列，显式列清单。
    tx.execute(
        "INSERT OR IGNORE INTO threads (
            id, rollout_path, created_at, updated_at, source, model_provider, cwd, title,
            sandbox_policy, approval_mode, tokens_used, has_user_event, archived, archived_at,
            git_sha, git_branch, git_origin_url, cli_version, first_user_message,
            agent_nickname, agent_role, memory_mode, model, reasoning_effort, agent_path,
            created_at_ms, updated_at_ms, thread_source, preview, recency_at, recency_at_ms,
            history_mode, name, is_pinned, thread_section_id, section_position, section_entered_at_ms
        )
        SELECT
            id, rollout_path, created_at, updated_at, source, model_provider, cwd, title,
            sandbox_policy, approval_mode, tokens_used, has_user_event, archived, archived_at,
            git_sha, git_branch, git_origin_url, cli_version, first_user_message,
            agent_nickname, agent_role, memory_mode, model, reasoning_effort, agent_path,
            created_at_ms, updated_at_ms, thread_source, preview, recency_at, recency_at_ms,
            history_mode, name, is_pinned, thread_section_id, section_position, section_entered_at_ms
        FROM snap.threads",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_dynamic_tools (thread_id, position, name, description, input_schema, defer_loading, namespace)
         SELECT thread_id, position, name, description, input_schema, defer_loading, namespace FROM snap.thread_dynamic_tools",
        [],
    )?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
         SELECT parent_thread_id, child_thread_id, status FROM snap.thread_spawn_edges",
        [],
    )?;

    tx.commit()?;

    let after_total: i64 = conn.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))?;
    let archived_total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE archived = 1",
        [],
        |r| r.get(0),
    )?;

    drop(_snap_conn);
    let _ = conn.execute("DETACH DATABASE snap", []);
    let _ = std::fs::remove_dir_all(&tmp);

    Ok((
        after_total as usize,
        (after_total - before_total) as usize,
        archived_total as usize,
    ))
}

/// 复制快照 rollout 文件到当前目录（跳过已存在）。
fn copy_rollouts(snapshot_codex: &Path, current_codex: &Path) -> Result<usize> {
    let mut copied = 0usize;
    for sub in ["sessions", "archived_sessions"] {
        let src = snapshot_codex.join(sub);
        if !src.exists() {
            continue;
        }
        let dst = current_codex.join(sub);
        std::fs::create_dir_all(&dst)?;
        for path in walkdir_files(&src) {
            let rel = path.strip_prefix(&src)?;
            let target = dst.join(rel);
            if target.exists() {
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &target)
                .with_context(|| format!("复制 {} 失败", path.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

/// 合并 session_index.jsonl（按 id 去重，保留时间序）。
fn merge_session_index(snapshot_codex: &Path, current_codex: &Path) -> Result<usize> {
    let snap_path = snapshot_codex.join("session_index.jsonl");
    let cur_path = current_codex.join("session_index.jsonl");
    if !snap_path.exists() {
        return Ok(0);
    }
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for path in [&cur_path, &snap_path] {
        if !path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(path)?;
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(id) = v.get("id").and_then(|i| i.as_str()) else {
                continue;
            };
            if ids.insert(id.to_string()) {
                entries.push(v);
            }
        }
    }
    let before = if cur_path.exists() {
        std::fs::read_to_string(&cur_path)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count()
    } else {
        0
    };
    entries.sort_by(|a, b| {
        a.get("updated_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""))
    });
    let mut out = String::new();
    for e in &entries {
        out.push_str(&serde_json::to_string(e)?);
        out.push('\n');
    }
    std::fs::write(&cur_path, out)?;
    Ok(entries.len().saturating_sub(before))
}

/// 校验当前库全部线程的 rollout_path 文件存在性，返回缺失数。
fn verify_rollout_refs(current_db: &Path) -> Result<usize> {
    let conn = rusqlite::Connection::open_with_flags(
        current_db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut missing = 0usize;
    let mut stmt = conn.prepare("SELECT rollout_path FROM threads")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows.flatten() {
        if !Path::new(&row).exists() {
            tracing::warn!(rollout = %row, "线程 rollout 文件缺失");
            missing += 1;
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 构造一个旧版结构（无 project_id 列）的快照库，含历史线程。
    fn create_snapshot_db(path: &Path) {
        let conn = rusqlite::Connection::open(path).expect("open snapshot db");
        conn.execute_batch(
            "CREATE TABLE thread_sections (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                source TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT NOT NULL,
                sandbox_policy TEXT NOT NULL,
                approval_mode TEXT NOT NULL,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                has_user_event INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0,
                archived_at INTEGER,
                git_sha TEXT,
                git_branch TEXT,
                git_origin_url TEXT,
                cli_version TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                agent_nickname TEXT, agent_role TEXT,
                memory_mode TEXT NOT NULL DEFAULT 'enabled',
                model TEXT, reasoning_effort TEXT, agent_path TEXT,
                created_at_ms INTEGER, updated_at_ms INTEGER,
                thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
                recency_at INTEGER NOT NULL DEFAULT 0, recency_at_ms INTEGER NOT NULL DEFAULT 0,
                history_mode TEXT NOT NULL DEFAULT 'legacy',
                name TEXT, is_pinned INTEGER NOT NULL DEFAULT 0,
                thread_section_id TEXT REFERENCES thread_sections(id) ON DELETE SET NULL,
                section_position INTEGER, section_entered_at_ms INTEGER);
             CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                input_schema TEXT NOT NULL,
                defer_loading INTEGER NOT NULL DEFAULT 0,
                namespace TEXT,
                PRIMARY KEY(thread_id, position));
             CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT NOT NULL PRIMARY KEY,
                status TEXT NOT NULL);
             INSERT INTO thread_sections (id, name) VALUES ('sec-1', 'Pinned');
             INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider,
                cwd, title, sandbox_policy, approval_mode, archived)
             VALUES
                ('hist-0001', '/Users/t/.codex/sessions/2026/07/01/rollout-a.jsonl',
                 1787000000, 1787000001, 'vscode', 'dex_ai', '/Users/t/proj', '历史线程1',
                 'default', 'default', 0),
                ('hist-0002', '/Users/t/.codex/archived_sessions/rollout-b.jsonl',
                 1786000000, 1786000001, 'vscode', 'dex_ai', '/Users/t/proj', '历史线程2',
                 'default', 'default', 1),
                ('hist-0003', '/Users/t/.codex/sessions/2026/07/02/rollout-c.jsonl',
                 1788000000, 1788000001, 'vscode', 'dex_ai', '/Users/t/proj', '历史线程3',
                 'default', 'default', 0);
             INSERT INTO thread_dynamic_tools (thread_id, position, name, description, input_schema)
             VALUES ('hist-0001', 0, 'web', '浏览', '{}');
             INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
             VALUES ('hist-0001', 'hist-0002', 'completed');",
        )
        .expect("create snapshot schema");
    }

    /// 构造新版结构（含 project_id 列）的当前库，含 1 条新线程。
    fn create_current_db(path: &Path) {
        let conn = rusqlite::Connection::open(path).expect("open current db");
        conn.execute_batch(
            "CREATE TABLE thread_sections (id TEXT PRIMARY KEY, name TEXT NOT NULL);
             CREATE TABLE projects (id TEXT PRIMARY KEY);
             CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                rollout_path TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                source TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT NOT NULL,
                sandbox_policy TEXT NOT NULL,
                approval_mode TEXT NOT NULL,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                has_user_event INTEGER NOT NULL DEFAULT 0,
                archived INTEGER NOT NULL DEFAULT 0,
                archived_at INTEGER,
                git_sha TEXT, git_branch TEXT, git_origin_url TEXT,
                cli_version TEXT NOT NULL DEFAULT '',
                first_user_message TEXT NOT NULL DEFAULT '',
                agent_nickname TEXT, agent_role TEXT,
                memory_mode TEXT NOT NULL DEFAULT 'enabled',
                model TEXT, reasoning_effort TEXT, agent_path TEXT,
                created_at_ms INTEGER, updated_at_ms INTEGER,
                thread_source TEXT, preview TEXT NOT NULL DEFAULT '',
                recency_at INTEGER NOT NULL DEFAULT 0, recency_at_ms INTEGER NOT NULL DEFAULT 0,
                history_mode TEXT NOT NULL DEFAULT 'legacy',
                name TEXT, is_pinned INTEGER NOT NULL DEFAULT 0,
                thread_section_id TEXT REFERENCES thread_sections(id) ON DELETE SET NULL,
                section_position INTEGER, section_entered_at_ms INTEGER,
                project_id TEXT REFERENCES projects(id) ON DELETE SET NULL);
             CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                name TEXT NOT NULL,
                description TEXT NOT NULL,
                input_schema TEXT NOT NULL,
                defer_loading INTEGER NOT NULL DEFAULT 0,
                namespace TEXT,
                PRIMARY KEY(thread_id, position));
             CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT NOT NULL PRIMARY KEY,
                status TEXT NOT NULL);
             INSERT INTO threads (id, rollout_path, created_at, updated_at, source, model_provider,
                cwd, title, sandbox_policy, approval_mode, archived)
             VALUES
                ('new-0001', '/Users/t/.codex/sessions/2026/08/20/rollout-new.jsonl',
                 1789000000, 1789000001, 'vscode', 'dex_ai', '/Users/t/proj', '新线程',
                 'default', 'default', 0);",
        )
        .expect("create current schema");
    }

    #[test]
    fn parse_tmutil_output_extracts_snapshot_names() {
        let out = "Snapshots for disk /:\n\
                   com.apple.TimeMachine.2026-08-20-071301.local\n\
                   com.apple.TimeMachine.2026-08-20-125305.local\n";
        let snaps = parse_tmutil_snapshots(out, "disk3s5");
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].name, "com.apple.TimeMachine.2026-08-20-125305.local");
        assert_eq!(snaps[0].created_at, "2026-08-20 12:53");
        assert_eq!(snaps[1].name, "com.apple.TimeMachine.2026-08-20-071301.local");
        assert_eq!(snaps[1].created_at, "2026-08-20 07:13");
        // 非法行忽略
        let out2 = "Snapshots for disk /:\nfoo\nnot-a-snapshot\n";
        assert!(parse_tmutil_snapshots(out2, "disk3s5").is_empty());
    }

    #[test]
    fn snapshot_stats_counts_threads_and_archived() {
        let dir = std::env::temp_dir().join(format!("snap-test-stats-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state_5.sqlite");
        create_snapshot_db(&db);
        let (tmp, conn) = open_snapshot_db_ro(&db).expect("open snapshot");
        let (total, archived) = snapshot_thread_stats(&conn).unwrap();
        assert_eq!(total, 3);
        assert_eq!(archived, 1);
        drop(conn);
        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_snapshot_db_merges_history_keeping_new_rows() {
        let dir = std::env::temp_dir().join(format!("snap-test-merge-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let snap_db = dir.join("snapshot.sqlite");
        let cur_db = dir.join("current.sqlite");
        create_snapshot_db(&snap_db);
        create_current_db(&cur_db);

        let (total, new_threads, archived) = merge_snapshot_db(&snap_db, &cur_db).unwrap();
        assert_eq!(new_threads, 3, "应新增 3 条历史线程");
        assert_eq!(total, 4, "合并后共 4 条（3 历史 + 1 当前）");
        assert_eq!(archived, 1);

        // 幂等：重复合并不产生重复。
        let (total2, new2, _) = merge_snapshot_db(&snap_db, &cur_db).unwrap();
        assert_eq!(total2, 4);
        assert_eq!(new2, 0, "重复合并新增数应为 0");

        // 关联表也已合并。
        let conn = rusqlite::Connection::open(&cur_db).unwrap();
        let tools: i64 = conn
            .query_row("SELECT COUNT(*) FROM thread_dynamic_tools", [], |r| r.get(0))
            .unwrap();
        let edges: i64 = conn
            .query_row("SELECT COUNT(*) FROM thread_spawn_edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tools, 1);
        assert_eq!(edges, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_rollouts_skips_existing_files() {
        let dir = std::env::temp_dir().join(format!("snap-test-copy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir.join("src/sessions/2026/07/01")).unwrap();
        fs::create_dir_all(&dir.join("dst/sessions/2026/07/01")).unwrap();
        fs::write(
            dir.join("src/sessions/2026/07/01/rollout-a.jsonl"),
            "a",
        )
        .unwrap();
        fs::write(
            dir.join("src/sessions/2026/07/01/rollout-b.jsonl"),
            "b",
        )
        .unwrap();
        fs::write(
            dir.join("dst/sessions/2026/07/01/rollout-a.jsonl"),
            "existing",
        )
        .unwrap();

        let copied = copy_rollouts(&dir.join("src"), &dir.join("dst")).unwrap();
        assert_eq!(copied, 1, "只复制缺失的 rollout-b");
        let content = fs::read_to_string(dir.join("dst/sessions/2026/07/01/rollout-a.jsonl")).unwrap();
        assert_eq!(content, "existing", "已存在文件不得覆盖");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_session_index_dedupes_by_id() {
        let dir = std::env::temp_dir().join(format!("snap-test-index-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let snap_dir = dir.join("snap");
        let cur_dir = dir.join("cur");
        fs::create_dir_all(&snap_dir).unwrap();
        fs::create_dir_all(&cur_dir).unwrap();
        fs::write(
            snap_dir.join("session_index.jsonl"),
            "{\"id\":\"a\",\"updated_at\":\"2026-07-01T00:00:00Z\"}\n\
             {\"id\":\"b\",\"updated_at\":\"2026-07-02T00:00:00Z\"}\n",
        )
        .unwrap();
        fs::write(
            cur_dir.join("session_index.jsonl"),
            "{\"id\":\"b\",\"updated_at\":\"2026-07-02T00:00:00Z\"}\n\
             {\"id\":\"c\",\"updated_at\":\"2026-08-01T00:00:00Z\"}\n",
        )
        .unwrap();

        let added = merge_session_index(&snap_dir, &cur_dir).unwrap();
        assert_eq!(added, 1, "只新增快照独有的 a");
        let merged = fs::read_to_string(cur_dir.join("session_index.jsonl")).unwrap();
        let lines: Vec<&str> = merged.lines().collect();
        assert_eq!(lines.len(), 3, "合并后 3 条（a+b+c）");
        assert!(lines[0].contains("\"a\""), "按 updated_at 排序");
        let _ = fs::remove_dir_all(&dir);
    }
}
