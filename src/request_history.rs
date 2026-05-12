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
            params![id, created_at, model, status, input_tokens, output_tokens, total, duration_ms, upstream_url, error_msg],
        );
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
    pub async fn clear(&self) -> Result<(), String> {
        let db = self.db.lock().await;
        db.execute("DELETE FROM request_history", [])
            .map_err(|e| format!("清空请求历史失败: {e}"))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_older_than(&self, max_entries: usize) {
        let db = self.db.lock().await;
        let _ = db.execute(
            "DELETE FROM request_history WHERE id NOT IN (
                SELECT id FROM request_history ORDER BY created_at DESC LIMIT ?1
            )",
            params![max_entries as i64],
        );
    }
}
