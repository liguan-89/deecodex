use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

#[allow(dead_code)]
#[derive(Clone, Serialize)]
pub struct HistoryEntry {
    pub id: String,
    pub created_at: u64,
    pub model: String,
    pub status: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub duration_ms: u64,
    pub upstream_url: String,
    pub error_msg: String,
}

#[allow(dead_code)]
#[derive(Clone, Serialize)]
pub struct MonthlyStats {
    pub year_month: String,
    pub total_requests: u32,
    pub success_count: u32,
    pub total_tokens: u32,
    pub avg_duration_ms: u64,
}

pub struct RequestHistoryStore {
    db: Arc<Mutex<Connection>>,
}

impl RequestHistoryStore {
    pub fn new(db_path: &Path) -> Result<Self, String> {
        let conn = Connection::open(db_path).map_err(|e| format!("打开请求历史数据库失败: {e}"))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS request_history (
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                model TEXT NOT NULL,
                status TEXT NOT NULL,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                total_tokens INTEGER DEFAULT 0,
                duration_ms INTEGER DEFAULT 0,
                upstream_url TEXT DEFAULT '',
                error_msg TEXT DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_created_at ON request_history(created_at DESC);
            CREATE TABLE IF NOT EXISTS history_monthly_stats (
                year_month TEXT PRIMARY KEY,
                total_requests INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                avg_duration_ms INTEGER NOT NULL DEFAULT 0
            );
            ",
        )
        .map_err(|e| format!("初始化请求历史表失败: {e}"))?;
        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        &self,
        id: String,
        created_at: u64,
        model: String,
        status: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
        upstream_url: String,
        error_msg: String,
    ) {
        let total = input_tokens + output_tokens;
        let db = self.db.lock().await;
        let _ = db.execute(
            "INSERT OR REPLACE INTO request_history
             (id, created_at, model, status, input_tokens, output_tokens, total_tokens, duration_ms, upstream_url, error_msg)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                created_at,
                model,
                status,
                input_tokens,
                output_tokens,
                total,
                duration_ms,
                upstream_url,
                error_msg
            ],
        );
        // 每月首条记录触发上月归档：聚合 → 写入 monthly_stats → 删除明细
        let has_old: bool = db
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM request_history
                    WHERE strftime('%Y-%m', datetime(created_at, 'unixepoch')) < strftime('%Y-%m', 'now')
                )",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if has_old {
            let _ = db.execute_batch(
                "INSERT OR REPLACE INTO history_monthly_stats
                     (year_month, total_requests, success_count, total_tokens, avg_duration_ms)
                 SELECT
                     strftime('%Y-%m', datetime(created_at, 'unixepoch')) as ym,
                     COUNT(*),
                     SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END),
                     SUM(total_tokens),
                     CAST(AVG(duration_ms) AS INTEGER)
                 FROM request_history
                 WHERE strftime('%Y-%m', datetime(created_at, 'unixepoch')) < strftime('%Y-%m', 'now')
                 GROUP BY ym;
                 DELETE FROM request_history
                 WHERE strftime('%Y-%m', datetime(created_at, 'unixepoch')) < strftime('%Y-%m', 'now');",
            );
        }
    }

    #[allow(dead_code)]
    pub async fn list(&self, limit: usize) -> Vec<HistoryEntry> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, created_at, model, status, input_tokens, output_tokens, total_tokens, duration_ms, upstream_url, error_msg
             FROM request_history ORDER BY created_at DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                created_at: row.get(1)?,
                model: row.get(2)?,
                status: row.get(3)?,
                input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                duration_ms: row.get(7)?,
                upstream_url: row.get(8)?,
                error_msg: row.get(9)?,
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    #[allow(dead_code)]
    pub async fn list_monthly_stats(&self, limit: usize) -> Vec<MonthlyStats> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT year_month, total_requests, success_count, total_tokens, avg_duration_ms
             FROM history_monthly_stats ORDER BY year_month DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(MonthlyStats {
                year_month: row.get(0)?,
                total_requests: row.get(1)?,
                success_count: row.get(2)?,
                total_tokens: row.get(3)?,
                avg_duration_ms: row.get(4)?,
            })
        });
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(_) => vec![],
        }
    }

    #[allow(dead_code)]
    pub async fn clear(&self) -> Result<(), String> {
        let db = self.db.lock().await;
        db.execute("DELETE FROM request_history", [])
            .map_err(|e| format!("清空请求历史失败: {e}"))?;
        db.execute("DELETE FROM history_monthly_stats", [])
            .map_err(|e| format!("清空月度统计失败: {e}"))?;
        Ok(())
    }
}
