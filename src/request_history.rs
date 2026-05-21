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
    pub provider: String,
    pub provider_profile: String,
    pub error_msg: String,
    pub cache_hit: bool,
    pub client_kind: String,
    pub account_id: String,
    pub account_name: String,
    pub endpoint_kind: String,
    pub request_path: String,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryFilter {
    pub client_kind: Option<String>,
    pub account_id: Option<String>,
}

impl HistoryFilter {
    fn client_kind_param(&self) -> &str {
        self.client_kind.as_deref().unwrap_or("").trim()
    }

    fn account_id_param(&self) -> &str {
        self.account_id.as_deref().unwrap_or("").trim()
    }
}

#[derive(Clone, Debug)]
pub struct HistoryRecord {
    pub id: String,
    pub created_at: u64,
    pub model: String,
    pub status: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub duration_ms: u64,
    pub upstream_url: String,
    pub provider: String,
    pub provider_profile: String,
    pub error_msg: String,
    pub cache_hit: bool,
    pub client_kind: String,
    pub account_id: String,
    pub account_name: String,
    pub endpoint_kind: String,
    pub request_path: String,
}

#[derive(Clone, Debug)]
pub struct HistoryContext {
    pub client_kind: String,
    pub account_id: String,
    pub account_name: String,
    pub endpoint_kind: String,
    pub request_path: String,
    pub provider: String,
    pub provider_profile: String,
}

impl Default for HistoryContext {
    fn default() -> Self {
        Self {
            client_kind: "codex".into(),
            account_id: String::new(),
            account_name: String::new(),
            endpoint_kind: "openai_responses".into(),
            request_path: "/v1/responses".into(),
            provider: String::new(),
            provider_profile: String::new(),
        }
    }
}

impl HistoryContext {
    #[allow(clippy::too_many_arguments)]
    pub fn record(
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
        cache_hit: bool,
    ) -> HistoryRecord {
        HistoryRecord {
            id,
            created_at,
            model,
            status,
            input_tokens,
            output_tokens,
            duration_ms,
            upstream_url,
            provider: self.provider.clone(),
            provider_profile: self.provider_profile.clone(),
            error_msg,
            cache_hit,
            client_kind: self.client_kind.clone(),
            account_id: self.account_id.clone(),
            account_name: self.account_name.clone(),
            endpoint_kind: self.endpoint_kind.clone(),
            request_path: self.request_path.clone(),
        }
    }
}

impl HistoryRecord {
    #[allow(dead_code, clippy::too_many_arguments)]
    pub fn codex(
        id: String,
        created_at: u64,
        model: String,
        status: String,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
        upstream_url: String,
        error_msg: String,
        cache_hit: bool,
    ) -> Self {
        Self {
            id,
            created_at,
            model,
            status,
            input_tokens,
            output_tokens,
            duration_ms,
            upstream_url,
            provider: String::new(),
            provider_profile: String::new(),
            error_msg,
            cache_hit,
            client_kind: "codex".into(),
            account_id: String::new(),
            account_name: String::new(),
            endpoint_kind: "openai_responses".into(),
            request_path: "/v1/responses".into(),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Serialize)]
pub struct MonthlyStats {
    pub year_month: String,
    pub client_kind: String,
    pub account_id: String,
    pub account_name: String,
    pub total_requests: u32,
    pub success_count: u32,
    pub total_tokens: u32,
    pub avg_duration_ms: u64,
}

#[allow(dead_code)]
#[derive(Clone, Serialize)]
pub struct RequestStats {
    pub total: u64,
    pub success_count: u64,
    pub cache_hit_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub total_duration_ms: u64,
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
                provider TEXT DEFAULT '',
                provider_profile TEXT DEFAULT '',
                error_msg TEXT DEFAULT '',
                cache_hit INTEGER DEFAULT 0,
                client_kind TEXT DEFAULT 'codex',
                account_id TEXT DEFAULT '',
                account_name TEXT DEFAULT '',
                endpoint_kind TEXT DEFAULT 'openai_responses',
                request_path TEXT DEFAULT '/v1/responses'
            );
            CREATE INDEX IF NOT EXISTS idx_created_at ON request_history(created_at DESC);
            CREATE TABLE IF NOT EXISTS history_monthly_stats (
                year_month TEXT NOT NULL,
                client_kind TEXT NOT NULL DEFAULT 'codex',
                account_id TEXT NOT NULL DEFAULT '',
                account_name TEXT NOT NULL DEFAULT '',
                total_requests INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                avg_duration_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (year_month, client_kind, account_id)
            );
            ",
        )
        .map_err(|e| format!("初始化请求历史表失败: {e}"))?;

        // 迁移：为已有数据库添加新增列（列已存在时忽略错误）。
        for ddl in [
            "ALTER TABLE request_history ADD COLUMN cache_hit INTEGER DEFAULT 0;",
            "ALTER TABLE request_history ADD COLUMN provider TEXT DEFAULT '';",
            "ALTER TABLE request_history ADD COLUMN provider_profile TEXT DEFAULT '';",
            "ALTER TABLE request_history ADD COLUMN client_kind TEXT DEFAULT 'codex';",
            "ALTER TABLE request_history ADD COLUMN account_id TEXT DEFAULT '';",
            "ALTER TABLE request_history ADD COLUMN account_name TEXT DEFAULT '';",
            "ALTER TABLE request_history ADD COLUMN endpoint_kind TEXT DEFAULT 'openai_responses';",
            "ALTER TABLE request_history ADD COLUMN request_path TEXT DEFAULT '/v1/responses';",
        ] {
            let _ = conn.execute_batch(ddl);
        }
        migrate_monthly_stats_table(&conn)?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_history_client_created ON request_history(client_kind, created_at DESC);
             CREATE INDEX IF NOT EXISTS idx_history_account_created ON request_history(account_id, created_at DESC);",
        )
        .map_err(|e| format!("初始化请求历史索引失败: {e}"))?;

        Ok(Self {
            db: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn record(&self, mut record: HistoryRecord) {
        if record.client_kind.trim().is_empty() {
            record.client_kind = "codex".into();
        }
        if record.endpoint_kind.trim().is_empty() {
            record.endpoint_kind = "openai_responses".into();
        }
        if record.request_path.trim().is_empty() {
            record.request_path = "/v1/responses".into();
        }
        if record.provider.trim().is_empty() {
            record.provider = crate::providers::guess_provider(&record.upstream_url).to_string();
        }
        if record.provider_profile.trim().is_empty() {
            record.provider_profile = crate::providers::profile_by_slug(&record.provider).slug;
        }

        let total = record.input_tokens + record.output_tokens;
        let db = self.db.lock().await;
        let _ = db.execute(
            "INSERT OR REPLACE INTO request_history
             (id, created_at, model, status, input_tokens, output_tokens, total_tokens, duration_ms, upstream_url, provider, provider_profile, error_msg, cache_hit, client_kind, account_id, account_name, endpoint_kind, request_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                record.id,
                record.created_at,
                record.model,
                record.status,
                record.input_tokens,
                record.output_tokens,
                total,
                record.duration_ms,
                record.upstream_url,
                record.provider,
                record.provider_profile,
                record.error_msg,
                record.cache_hit,
                record.client_kind,
                record.account_id,
                record.account_name,
                record.endpoint_kind,
                record.request_path,
            ],
        );
        archive_previous_months(&db);
    }

    #[allow(dead_code)]
    pub async fn list(&self, limit: usize, filter: &HistoryFilter) -> Vec<HistoryEntry> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT id, created_at, model, status, input_tokens, output_tokens, total_tokens, duration_ms, upstream_url, provider, provider_profile, error_msg, cache_hit, client_kind, account_id, account_name, endpoint_kind, request_path
             FROM request_history
             WHERE (?2 = '' OR client_kind = ?2)
               AND (?3 = '' OR account_id = ?3)
             ORDER BY created_at DESC LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("查询请求历史失败: {err}");
                return vec![];
            }
        };
        let rows = stmt.query_map(
            params![
                limit as i64,
                filter.client_kind_param(),
                filter.account_id_param()
            ],
            |row| {
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
                    provider: row.get(9)?,
                    provider_profile: row.get(10)?,
                    error_msg: row.get(11)?,
                    cache_hit: row.get(12)?,
                    client_kind: row.get(13)?,
                    account_id: row.get(14)?,
                    account_name: row.get(15)?,
                    endpoint_kind: row.get(16)?,
                    request_path: row.get(17)?,
                })
            },
        );
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(err) => {
                tracing::warn!("读取请求历史行失败: {err}");
                vec![]
            }
        }
    }

    #[allow(dead_code)]
    pub async fn list_monthly_stats(
        &self,
        limit: usize,
        filter: &HistoryFilter,
    ) -> Vec<MonthlyStats> {
        let db = self.db.lock().await;
        let mut stmt = match db.prepare(
            "SELECT year_month, client_kind, account_id, account_name, total_requests, success_count, total_tokens, avg_duration_ms
             FROM history_monthly_stats
             WHERE (?2 = '' OR client_kind = ?2)
               AND (?3 = '' OR account_id = ?3)
             ORDER BY year_month DESC, client_kind ASC, account_name ASC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("查询月度请求统计失败: {err}");
                return vec![];
            }
        };
        let rows = stmt.query_map(
            params![
                limit as i64,
                filter.client_kind_param(),
                filter.account_id_param()
            ],
            |row| {
                Ok(MonthlyStats {
                    year_month: row.get(0)?,
                    client_kind: row.get(1)?,
                    account_id: row.get(2)?,
                    account_name: row.get(3)?,
                    total_requests: row.get(4)?,
                    success_count: row.get(5)?,
                    total_tokens: row.get(6)?,
                    avg_duration_ms: row.get(7)?,
                })
            },
        );
        match rows {
            Ok(iter) => iter.filter_map(|r| r.ok()).collect(),
            Err(err) => {
                tracing::warn!("读取月度请求统计失败: {err}");
                vec![]
            }
        }
    }

    #[allow(dead_code)]
    pub async fn stats_since(&self, since_secs: u64, filter: &HistoryFilter) -> RequestStats {
        let db = self.db.lock().await;
        let result = db.query_row(
            "SELECT
                 COUNT(*) as total,
                 COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0) as success_count,
                 COALESCE(SUM(CASE WHEN cache_hit != 0 THEN 1 ELSE 0 END), 0) as cache_hit_count,
                 COALESCE(SUM(input_tokens), 0) as input_tokens,
                 COALESCE(SUM(output_tokens), 0) as output_tokens,
                 COALESCE(SUM(total_tokens), 0) as total_tokens,
                 COALESCE(SUM(duration_ms), 0) as total_duration_ms,
                 COALESCE(CAST(AVG(duration_ms) AS INTEGER), 0) as avg_duration_ms
             FROM request_history
             WHERE created_at >= ?1
               AND (?2 = '' OR client_kind = ?2)
               AND (?3 = '' OR account_id = ?3)",
            params![
                since_secs as i64,
                filter.client_kind_param(),
                filter.account_id_param()
            ],
            |row| {
                Ok(RequestStats {
                    total: row.get::<_, i64>(0)?.max(0) as u64,
                    success_count: row.get::<_, i64>(1)?.max(0) as u64,
                    cache_hit_count: row.get::<_, i64>(2)?.max(0) as u64,
                    input_tokens: row.get::<_, i64>(3)?.max(0) as u64,
                    output_tokens: row.get::<_, i64>(4)?.max(0) as u64,
                    total_tokens: row.get::<_, i64>(5)?.max(0) as u64,
                    total_duration_ms: row.get::<_, i64>(6)?.max(0) as u64,
                    avg_duration_ms: row.get::<_, i64>(7)?.max(0) as u64,
                })
            },
        );
        result.unwrap_or(RequestStats {
            total: 0,
            success_count: 0,
            cache_hit_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
            total_duration_ms: 0,
            avg_duration_ms: 0,
        })
    }

    #[allow(dead_code)]
    pub async fn clear(&self, filter: &HistoryFilter) -> Result<(), String> {
        let db = self.db.lock().await;
        db.execute(
            "DELETE FROM request_history
             WHERE (?1 = '' OR client_kind = ?1)
               AND (?2 = '' OR account_id = ?2)",
            params![filter.client_kind_param(), filter.account_id_param()],
        )
        .map_err(|e| format!("清空请求历史失败: {e}"))?;
        db.execute(
            "DELETE FROM history_monthly_stats
             WHERE (?1 = '' OR client_kind = ?1)
               AND (?2 = '' OR account_id = ?2)",
            params![filter.client_kind_param(), filter.account_id_param()],
        )
        .map_err(|e| format!("清空月度统计失败: {e}"))?;
        Ok(())
    }
}

fn migrate_monthly_stats_table(conn: &Connection) -> Result<(), String> {
    if table_has_column(conn, "history_monthly_stats", "client_kind") {
        return Ok(());
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS history_monthly_stats_new (
            year_month TEXT NOT NULL,
            client_kind TEXT NOT NULL DEFAULT 'codex',
            account_id TEXT NOT NULL DEFAULT '',
            account_name TEXT NOT NULL DEFAULT '',
            total_requests INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            avg_duration_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (year_month, client_kind, account_id)
        );
        INSERT OR REPLACE INTO history_monthly_stats_new
            (year_month, client_kind, account_id, account_name, total_requests, success_count, total_tokens, avg_duration_ms)
        SELECT year_month, 'codex', '', '', total_requests, success_count, total_tokens, avg_duration_ms
        FROM history_monthly_stats;
        DROP TABLE history_monthly_stats;
        ALTER TABLE history_monthly_stats_new RENAME TO history_monthly_stats;",
    )
    .map_err(|e| format!("迁移月度统计表失败: {e}"))
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let mut stmt = match conn.prepare(&format!("PRAGMA table_info({table})")) {
        Ok(stmt) => stmt,
        Err(_) => return false,
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(1)) {
        Ok(rows) => rows,
        Err(_) => return false,
    };
    for name in rows.filter_map(Result::ok) {
        if name == column {
            return true;
        }
    }
    false
}

fn archive_previous_months(db: &Connection) {
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
    if !has_old {
        return;
    }

    if let Err(err) = db.execute_batch(
        "INSERT INTO history_monthly_stats
             (year_month, client_kind, account_id, account_name, total_requests, success_count, total_tokens, avg_duration_ms)
         SELECT
             strftime('%Y-%m', datetime(created_at, 'unixepoch')) as ym,
             COALESCE(NULLIF(client_kind, ''), 'codex'),
             COALESCE(account_id, ''),
             COALESCE(MAX(account_name), ''),
             COUNT(*),
             SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END),
             SUM(total_tokens),
             CAST(AVG(duration_ms) AS INTEGER)
         FROM request_history
         WHERE strftime('%Y-%m', datetime(created_at, 'unixepoch')) < strftime('%Y-%m', 'now')
         GROUP BY ym, COALESCE(NULLIF(client_kind, ''), 'codex'), COALESCE(account_id, '')
         ON CONFLICT(year_month, client_kind, account_id) DO UPDATE SET
             account_name = excluded.account_name,
             total_requests = history_monthly_stats.total_requests + excluded.total_requests,
             success_count = history_monthly_stats.success_count + excluded.success_count,
             total_tokens = history_monthly_stats.total_tokens + excluded.total_tokens,
             avg_duration_ms = CASE
                 WHEN history_monthly_stats.total_requests + excluded.total_requests = 0 THEN 0
                 ELSE CAST(((history_monthly_stats.avg_duration_ms * history_monthly_stats.total_requests) + (excluded.avg_duration_ms * excluded.total_requests)) / (history_monthly_stats.total_requests + excluded.total_requests) AS INTEGER)
             END;
         DELETE FROM request_history
         WHERE strftime('%Y-%m', datetime(created_at, 'unixepoch')) < strftime('%Y-%m', 'now');",
    ) {
        tracing::warn!("归档请求历史失败: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::{HistoryFilter, HistoryRecord, RequestHistoryStore};
    use rusqlite::Connection;
    use std::path::Path;

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn record(id: &str, ts: u64, client_kind: &str, account_id: &str) -> HistoryRecord {
        HistoryRecord {
            id: id.into(),
            created_at: ts,
            model: "deepseek-chat".into(),
            status: "completed".into(),
            input_tokens: 10,
            output_tokens: 20,
            duration_ms: 100,
            upstream_url: "https://api.example.test/v1/chat/completions".into(),
            provider: String::new(),
            provider_profile: String::new(),
            error_msg: String::new(),
            cache_hit: false,
            client_kind: client_kind.into(),
            account_id: account_id.into(),
            account_name: format!("账号 {account_id}"),
            endpoint_kind: "openai_chat".into(),
            request_path: "/v1/chat/completions".into(),
        }
    }

    #[tokio::test]
    async fn records_and_lists_recent_entries() {
        let store = RequestHistoryStore::new(Path::new(":memory:")).unwrap();
        let now = now_secs();

        store
            .record(HistoryRecord::codex(
                "old".into(),
                now,
                "deepseek-chat".into(),
                "completed".into(),
                12,
                34,
                250,
                "https://api.example.test/v1/chat/completions".into(),
                String::new(),
                false,
            ))
            .await;
        store
            .record(HistoryRecord::codex(
                "new".into(),
                now + 1,
                "deepseek-reasoner".into(),
                "failed".into(),
                5,
                0,
                1_500,
                "https://api.example.test/v1/chat/completions".into(),
                "HTTP 429".into(),
                true,
            ))
            .await;

        let entries = store.list(1, &HistoryFilter::default()).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "new");
        assert_eq!(entries[0].model, "deepseek-reasoner");
        assert_eq!(entries[0].status, "failed");
        assert_eq!(entries[0].total_tokens, 5);
        assert_eq!(entries[0].duration_ms, 1_500);
        assert_eq!(entries[0].provider, "custom");
        assert_eq!(entries[0].provider_profile, "custom");
        assert_eq!(entries[0].client_kind, "codex");
        assert_eq!(entries[0].endpoint_kind, "openai_responses");
        assert_eq!(entries[0].request_path, "/v1/responses");
        assert_eq!(entries[0].error_msg, "HTTP 429");
        assert!(entries[0].cache_hit);
    }

    #[tokio::test]
    async fn records_provider_profile_from_upstream() {
        let store = RequestHistoryStore::new(Path::new(":memory:")).unwrap();
        store
            .record(HistoryRecord::codex(
                "kimi".into(),
                now_secs(),
                "kimi-k2".into(),
                "completed".into(),
                1,
                2,
                3,
                "https://api.moonshot.cn/v1/chat/completions".into(),
                String::new(),
                false,
            ))
            .await;

        let entries = store.list(10, &HistoryFilter::default()).await;
        assert_eq!(entries[0].provider, "kimi");
        assert_eq!(entries[0].provider_profile, "kimi");
    }

    #[tokio::test]
    async fn filters_by_client_kind_and_account_id() {
        let store = RequestHistoryStore::new(Path::new(":memory:")).unwrap();
        let now = now_secs();
        store.record(record("codex", now, "codex", "")).await;
        store
            .record(record("claude", now + 1, "claude_code", "acct-a"))
            .await;
        store
            .record(record("hermes", now + 2, "hermes", "acct-b"))
            .await;

        let claude = store
            .list(
                10,
                &HistoryFilter {
                    client_kind: Some("claude_code".into()),
                    account_id: None,
                },
            )
            .await;
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].id, "claude");

        let acct_b = store
            .stats_since(
                now,
                &HistoryFilter {
                    client_kind: None,
                    account_id: Some("acct-b".into()),
                },
            )
            .await;
        assert_eq!(acct_b.total, 1);
        assert_eq!(acct_b.total_tokens, 30);
    }

    #[tokio::test]
    async fn stats_since_aggregates_without_list_limit() {
        let store = RequestHistoryStore::new(Path::new(":memory:")).unwrap();
        let now = now_secs();

        for i in 0..5 {
            let mut item = record(&format!("req-{i}"), now + i, "codex", "");
            if i == 4 {
                item.status = "failed".into();
            }
            item.duration_ms = 100 + i;
            item.cache_hit = i == 1;
            store.record(item).await;
        }

        let recent = store.list(3, &HistoryFilter::default()).await;
        assert_eq!(recent.len(), 3);

        let stats = store.stats_since(now, &HistoryFilter::default()).await;
        assert_eq!(stats.total, 5);
        assert_eq!(stats.success_count, 4);
        assert_eq!(stats.cache_hit_count, 1);
        assert_eq!(stats.input_tokens, 50);
        assert_eq!(stats.output_tokens, 100);
        assert_eq!(stats.total_tokens, 150);
        assert_eq!(stats.total_duration_ms, 510);
        assert_eq!(stats.avg_duration_ms, 102);
    }

    #[tokio::test]
    async fn monthly_archive_is_grouped_by_client_and_account() {
        let store = RequestHistoryStore::new(Path::new(":memory:")).unwrap();
        store.record(record("old-codex", 0, "codex", "")).await;
        store
            .record(record("old-claude", 1, "claude_code", "acct-a"))
            .await;

        let all = store
            .list_monthly_stats(10, &HistoryFilter::default())
            .await;
        assert_eq!(all.len(), 2);
        assert!(all.iter().any(|s| s.client_kind == "codex"));
        assert!(all
            .iter()
            .any(|s| s.client_kind == "claude_code" && s.account_id == "acct-a"));

        let claude = store
            .list_monthly_stats(
                10,
                &HistoryFilter {
                    client_kind: Some("claude_code".into()),
                    account_id: None,
                },
            )
            .await;
        assert_eq!(claude.len(), 1);
        assert_eq!(claude[0].total_tokens, 30);
    }

    #[tokio::test]
    async fn clear_can_target_one_client() {
        let store = RequestHistoryStore::new(Path::new(":memory:")).unwrap();
        let now = now_secs();
        store.record(record("codex", now, "codex", "")).await;
        store
            .record(record("claude", now + 1, "claude_code", "acct-a"))
            .await;

        store
            .clear(&HistoryFilter {
                client_kind: Some("claude_code".into()),
                account_id: None,
            })
            .await
            .unwrap();

        let entries = store.list(10, &HistoryFilter::default()).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "codex");
    }

    #[test]
    fn migrates_legacy_monthly_table() {
        let path = std::env::temp_dir().join(format!("deecodex-history-migrate-{}.db", now_secs()));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE request_history (
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
            CREATE TABLE history_monthly_stats (
                year_month TEXT PRIMARY KEY,
                total_requests INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                avg_duration_ms INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO history_monthly_stats VALUES ('2025-01', 2, 1, 30, 100);",
        )
        .unwrap();
        drop(conn);

        let store = RequestHistoryStore::new(&path).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let stats = rt.block_on(store.list_monthly_stats(10, &HistoryFilter::default()));
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].client_kind, "codex");
        assert_eq!(stats[0].total_requests, 2);
        let _ = std::fs::remove_file(path);
    }
}
