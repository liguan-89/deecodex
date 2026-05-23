//! 多客户端线程聚合模块。
//!
//! 这里做只读聚合：Codex 继续复用 `codex_threads`，其他客户端只读取本地历史，
//! 不改写非 Codex 客户端文件。

use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::accounts::AccountClientKind;
use crate::client_integrations::profile_for_kind;

const MAX_MESSAGE_CHARS: usize = 24_000;
const MAX_TOTAL_CHARS: usize = 1_500_000;
const MAX_SCAN_FILES: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedThreadInfo {
    pub thread_key: String,
    pub client_kind: AccountClientKind,
    pub client_label: String,
    pub native_id: String,
    pub title: String,
    pub model: String,
    pub provider: String,
    pub cwd: String,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub message_count: usize,
    pub preview: String,
    pub source_path: String,
    pub detail_available: bool,
    pub delete_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSourceStatus {
    pub client_kind: AccountClientKind,
    pub client_label: String,
    pub available: bool,
    pub scan_paths: Vec<String>,
    pub count: usize,
    pub diagnostics: Vec<String>,
    pub detail_available: bool,
    pub delete_available: bool,
    pub migrate_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedThreadContent {
    pub thread: UnifiedThreadInfo,
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedThreadList {
    pub sources: Vec<ThreadSourceStatus>,
    pub threads: Vec<UnifiedThreadInfo>,
    pub total: usize,
}

#[derive(Debug, Clone)]
struct ThreadScanResult {
    status: ThreadSourceStatus,
    threads: Vec<UnifiedThreadInfo>,
}

#[derive(Debug, Clone)]
struct ClaudeThreadRecord {
    info: UnifiedThreadInfo,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct ClaudeFileSummary {
    thread: UnifiedThreadInfo,
    first_user_title: Option<String>,
}

struct ClaudeThreadAggregate {
    info: UnifiedThreadInfo,
    paths: Vec<PathBuf>,
    first_user_title: Option<String>,
}

impl ClaudeThreadAggregate {
    fn new(summary: ClaudeFileSummary, path: PathBuf) -> Self {
        let mut info = summary.thread;
        if let Some(title) = &summary.first_user_title {
            info.title = title.clone();
        }
        Self {
            info,
            paths: vec![path],
            first_user_title: summary.first_user_title,
        }
    }

    fn push(&mut self, summary: ClaudeFileSummary, path: PathBuf) {
        let thread = summary.thread;
        self.info.message_count += thread.message_count;
        if self.first_user_title.is_none() {
            if let Some(title) = summary.first_user_title {
                self.info.title = title.clone();
                self.first_user_title = Some(title);
            }
        }
        if self.info.preview.trim().is_empty() && !thread.preview.trim().is_empty() {
            self.info.preview = thread.preview.clone();
        }
        if self.info.model.trim().is_empty() && !thread.model.trim().is_empty() {
            self.info.model = thread.model.clone();
        }
        if self.info.cwd.trim().is_empty() && !thread.cwd.trim().is_empty() {
            self.info.cwd = thread.cwd.clone();
        }
        self.info.created_at_ms = min_time(self.info.created_at_ms, thread.created_at_ms);
        if max_time(self.info.updated_at_ms, thread.updated_at_ms) == thread.updated_at_ms {
            self.info.source_path = thread.source_path.clone();
        }
        self.info.updated_at_ms = max_time(self.info.updated_at_ms, thread.updated_at_ms);
        self.paths.push(path);
    }

    fn finish(mut self) -> ClaudeThreadRecord {
        sort_paths_by_time(&mut self.paths);
        self.info.thread_key =
            grouped_thread_key(&self.info.client_kind, &self.info.native_id, &self.paths);
        ClaudeThreadRecord {
            info: self.info,
            paths: self.paths,
        }
    }
}

pub fn parse_client_kind(raw: &str) -> Option<AccountClientKind> {
    match raw.trim() {
        "codex" | "Codex" => Some(AccountClientKind::Codex),
        "claude_code" | "ClaudeCode" | "claude" => Some(AccountClientKind::ClaudeCode),
        "openclaw" | "Openclaw" => Some(AccountClientKind::Openclaw),
        "hermes" | "Hermes" => Some(AccountClientKind::Hermes),
        "generic_client" | "GenericClient" | "generic" => Some(AccountClientKind::GenericClient),
        _ => None,
    }
}

pub fn get_thread_sources(data_dir: &Path) -> Vec<ThreadSourceStatus> {
    scan_all(data_dir)
        .into_iter()
        .map(|result| result.status)
        .collect()
}

pub fn list_client_threads(data_dir: &Path) -> UnifiedThreadList {
    let mut sources = Vec::new();
    let mut threads = Vec::new();
    for result in scan_all(data_dir) {
        sources.push(result.status);
        threads.extend(result.threads);
    }
    threads.sort_by(|a, b| {
        b.updated_at_ms
            .or(b.created_at_ms)
            .cmp(&a.updated_at_ms.or(a.created_at_ms))
            .then_with(|| a.title.cmp(&b.title))
    });
    let total = threads.len();
    UnifiedThreadList {
        sources,
        threads,
        total,
    }
}

pub fn get_client_thread_content(
    client_kind: AccountClientKind,
    native_id: &str,
    thread_key: Option<&str>,
) -> Result<UnifiedThreadContent> {
    match client_kind {
        AccountClientKind::Codex => get_codex_content(native_id),
        AccountClientKind::ClaudeCode => {
            let root = home_path(&[".claude", "projects"])?;
            let record = resolve_claude_thread(&root, native_id, thread_key)?;
            read_claude_content_from_paths(&record.info, &record.paths)
        }
        AccountClientKind::Hermes => {
            let root = home_path(&[".hermes", "sessions"])?;
            let threads = scan_hermes_threads(&root).threads;
            let info = resolve_thread_info(threads, native_id, thread_key)
                .ok_or_else(|| anyhow!("未找到 Hermes 线程 {native_id}"))?;
            read_hermes_content(&info)
        }
        AccountClientKind::Openclaw | AccountClientKind::GenericClient => {
            Err(anyhow!("该客户端暂不支持线程详情"))
        }
    }
}

fn scan_all(data_dir: &Path) -> Vec<ThreadScanResult> {
    vec![
        scan_codex_threads(data_dir),
        home_path(&[".claude", "projects"])
            .map(|path| scan_claude_threads(&path))
            .unwrap_or_else(|err| error_source(AccountClientKind::ClaudeCode, err.to_string())),
        placeholder_source(
            AccountClientKind::Openclaw,
            vec![home_path_lossy(&[".openclaw"])],
            "暂未发现可读线程源，OpenClaw 线程聚合将在后续适配。",
        ),
        home_path(&[".hermes", "sessions"])
            .map(|path| scan_hermes_threads(&path))
            .unwrap_or_else(|err| error_source(AccountClientKind::Hermes, err.to_string())),
        placeholder_source(
            AccountClientKind::GenericClient,
            vec![home_path_lossy(&[".deecodex", "client-env"])],
            "通用客户端没有统一历史格式，当前仅展示配置状态占位。",
        ),
    ]
}

fn scan_codex_threads(data_dir: &Path) -> ThreadScanResult {
    let client_kind = AccountClientKind::Codex;
    let client_label = client_label(&client_kind);
    let mut diagnostics = Vec::new();
    let status_result = crate::codex_threads::status(data_dir);
    let mut migrated = false;
    if let Ok(status) = &status_result {
        migrated = status.migrated;
    } else if let Err(err) = &status_result {
        diagnostics.push(format!("Codex 状态读取失败: {err}"));
        tracing::warn!("读取 Codex 线程状态失败: {err}");
    }

    let mut list_readable = false;
    let threads = match crate::codex_threads::list_all() {
        Ok(items) => {
            list_readable = true;
            items.into_iter().map(codex_thread_to_unified).collect()
        }
        Err(err) => {
            diagnostics.push(format!("Codex 线程读取失败: {err}"));
            tracing::warn!("读取 Codex 线程失败: {err}");
            Vec::new()
        }
    };

    let mut scan_paths = Vec::new();
    if let Ok(path) = home_path(&[".codex"]) {
        scan_paths.push(path.to_string_lossy().to_string());
    }

    ThreadScanResult {
        status: ThreadSourceStatus {
            client_kind,
            client_label,
            available: list_readable,
            scan_paths,
            count: threads.len(),
            diagnostics,
            detail_available: true,
            delete_available: true,
            migrate_available: status_result.is_ok() && !migrated,
        },
        threads,
    }
}

fn scan_claude_threads(root: &Path) -> ThreadScanResult {
    let client_kind = AccountClientKind::ClaudeCode;
    let mut diagnostics = Vec::new();

    if !root.exists() {
        diagnostics.push("Claude Code 项目历史目录不存在".into());
        return source_result(
            client_kind,
            false,
            vec![root],
            diagnostics,
            Vec::new(),
            true,
            false,
        );
    }

    let records = collect_claude_thread_records(root, &mut diagnostics);
    let threads = records
        .into_iter()
        .map(|record| record.info)
        .collect::<Vec<_>>();
    let available = diagnostics.is_empty() || !threads.is_empty();
    source_result(
        client_kind,
        available,
        vec![root],
        diagnostics,
        threads,
        true,
        false,
    )
}

fn collect_claude_thread_records(
    root: &Path,
    diagnostics: &mut Vec<String>,
) -> Vec<ClaudeThreadRecord> {
    let mut threads_by_id: HashMap<String, ClaudeThreadAggregate> = HashMap::new();
    let mut files = find_files_with_extensions(root, &["jsonl"], diagnostics);
    sort_paths_by_time(&mut files);

    for path in files {
        match summarize_claude_file(&path) {
            Ok(Some(summary)) => {
                threads_by_id
                    .entry(summary.thread.native_id.clone())
                    .and_modify(|aggregate| aggregate.push(summary.clone(), path.clone()))
                    .or_insert_with(|| ClaudeThreadAggregate::new(summary, path));
            }
            Ok(None) => {}
            Err(err) => {
                diagnostics.push(format!("{}: {err}", path.display()));
                tracing::warn!(path = %path.display(), "解析 Claude Code 线程失败: {err}");
            }
        }
    }

    let mut records = threads_by_id
        .into_values()
        .map(ClaudeThreadAggregate::finish)
        .collect::<Vec<_>>();
    records.sort_by_key(|record| Reverse(record.info.updated_at_ms));
    records
}

fn resolve_claude_thread(
    root: &Path,
    native_id: &str,
    thread_key: Option<&str>,
) -> Result<ClaudeThreadRecord> {
    let mut diagnostics = Vec::new();
    let records = collect_claude_thread_records(root, &mut diagnostics);
    records
        .iter()
        .find(|record| thread_key_matches(&record.info, thread_key))
        .or_else(|| record_by_native_id(&records, native_id))
        .cloned()
        .ok_or_else(|| {
            let suffix = if diagnostics.is_empty() {
                String::new()
            } else {
                format!("；诊断: {}", diagnostics.join("；"))
            };
            anyhow!("未找到 Claude Code 线程 {native_id}{suffix}")
        })
}

fn scan_hermes_threads(root: &Path) -> ThreadScanResult {
    let client_kind = AccountClientKind::Hermes;
    let mut diagnostics = Vec::new();

    if !root.exists() {
        diagnostics.push("Hermes sessions 目录不存在".into());
        return source_result(
            client_kind,
            false,
            vec![root],
            diagnostics,
            Vec::new(),
            true,
            false,
        );
    }

    let files = find_files_with_extensions(root, &["json", "jsonl"], &mut diagnostics);
    let mut threads = Vec::new();
    for path in files {
        let result = match path.extension().and_then(|ext| ext.to_str()) {
            Some("json") => summarize_hermes_json_file(&path),
            Some("jsonl") => summarize_hermes_jsonl_file(&path),
            _ => Ok(None),
        };
        match result {
            Ok(Some(thread)) => threads.push(thread),
            Ok(None) => {}
            Err(err) => {
                diagnostics.push(format!("{}: {err}", path.display()));
                tracing::warn!(path = %path.display(), "解析 Hermes 线程失败: {err}");
            }
        }
    }

    threads.sort_by_key(|thread| Reverse(thread.updated_at_ms));
    let available = diagnostics.is_empty() || !threads.is_empty();
    source_result(
        client_kind,
        available,
        vec![root],
        diagnostics,
        threads,
        true,
        false,
    )
}

fn summarize_claude_file(path: &Path) -> Result<Option<ClaudeFileSummary>> {
    let file = fs::File::open(path).with_context(|| format!("打开文件失败: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut native_id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();
    let mut title = String::new();
    let mut preview = String::new();
    let mut first_user_title = None;
    let mut model = String::new();
    let mut cwd = String::new();
    let mut message_count = 0usize;
    let mut bad_lines = 0usize;

    for line in reader.lines() {
        let line = line.context("读取 JSONL 行失败")?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                bad_lines += 1;
                continue;
            }
        };
        if let Some(session_id) = value.get("sessionId").and_then(Value::as_str) {
            native_id = session_id.to_string();
        }
        if cwd.is_empty() {
            cwd = value
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        if model.is_empty() {
            model = value
                .get("message")
                .and_then(|m| m.get("model"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        let role = claude_role(&value);
        if !matches!(role.as_deref(), Some("user" | "assistant" | "system")) {
            continue;
        }
        let text = claude_text(&value);
        if text.trim().is_empty() {
            continue;
        }
        message_count += 1;
        if preview.is_empty() {
            preview = truncate_text(text.trim(), 240);
        }
        if title.is_empty() && role.as_deref() == Some("user") {
            title = truncate_text(text.trim(), 80);
            first_user_title = Some(title.clone());
        }
    }

    if message_count == 0 {
        return Ok(None);
    }
    if title.is_empty() {
        title = if preview.is_empty() {
            format!("Claude Code 会话 {native_id}")
        } else {
            truncate_text(&preview, 80)
        };
    }

    let mut thread = file_thread(
        FileThreadInput {
            client_kind: AccountClientKind::ClaudeCode,
            native_id,
            title,
            model,
            provider: String::new(),
            cwd,
            message_count,
            preview,
        },
        path,
    );
    if bad_lines > 0 {
        thread.preview = format!("{}（跳过 {bad_lines} 行损坏记录）", thread.preview);
    }
    Ok(Some(ClaudeFileSummary {
        thread,
        first_user_title,
    }))
}

fn summarize_hermes_json_file(path: &Path) -> Result<Option<UnifiedThreadInfo>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("读取 Hermes 会话失败: {}", path.display()))?;
    let value: Value = serde_json::from_str(&text).context("解析 Hermes JSON 失败")?;
    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if messages.is_empty() {
        return Ok(None);
    }

    let native_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| file_stem(path));
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let provider = value
        .get("base_url")
        .and_then(Value::as_str)
        .or_else(|| value.get("platform").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let preview = messages
        .iter()
        .find_map(message_text)
        .map(|text| truncate_text(text.trim(), 240))
        .unwrap_or_default();
    let title = messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .and_then(message_text)
        .map(|text| truncate_text(text.trim(), 80))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("Hermes 会话 {native_id}"));

    Ok(Some(file_thread(
        FileThreadInput {
            client_kind: AccountClientKind::Hermes,
            native_id,
            title,
            model,
            provider,
            cwd: String::new(),
            message_count: messages.len(),
            preview,
        },
        path,
    )))
}

fn summarize_hermes_jsonl_file(path: &Path) -> Result<Option<UnifiedThreadInfo>> {
    let file = fs::File::open(path).with_context(|| format!("打开文件失败: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut native_id = file_stem(path);
    let mut model = String::new();
    let mut bad_lines = 0usize;

    for line in reader.lines() {
        let line = line.context("读取 JSONL 行失败")?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                bad_lines += 1;
                continue;
            }
        };
        if let Some(id) = value.get("session_id").and_then(Value::as_str) {
            native_id = id.to_string();
        }
        if model.is_empty() {
            model = value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
        }
        if hermes_jsonl_role(&value).is_some() && !hermes_jsonl_text(&value).is_empty() {
            messages.push(value);
        }
    }
    if messages.is_empty() {
        return Ok(None);
    }

    let preview = messages
        .iter()
        .find_map(|message| {
            let text = hermes_jsonl_text(message);
            (!text.trim().is_empty()).then(|| truncate_text(text.trim(), 240))
        })
        .unwrap_or_default();
    let title = messages
        .iter()
        .find(|message| hermes_jsonl_role(message).as_deref() == Some("user"))
        .map(hermes_jsonl_text)
        .map(|text| truncate_text(text.trim(), 80))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| format!("Hermes 会话 {native_id}"));
    let mut thread = file_thread(
        FileThreadInput {
            client_kind: AccountClientKind::Hermes,
            native_id,
            title,
            model,
            provider: String::new(),
            cwd: String::new(),
            message_count: messages.len(),
            preview,
        },
        path,
    );
    if bad_lines > 0 {
        thread.preview = format!("{}（跳过 {bad_lines} 行损坏记录）", thread.preview);
    }
    Ok(Some(thread))
}

fn read_claude_content_from_paths(
    info: &UnifiedThreadInfo,
    paths: &[PathBuf],
) -> Result<UnifiedThreadContent> {
    let mut messages = Vec::new();
    let mut total_chars = 0usize;

    for path in paths {
        let file =
            fs::File::open(path).with_context(|| format!("打开文件失败: {}", path.display()))?;
        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line.context("读取 JSONL 行失败")?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(path = %path.display(), "跳过无法解析的 Claude Code 行: {err}");
                    continue;
                }
            };
            let role = match claude_role(&value) {
                Some(role) if matches!(role.as_str(), "user" | "assistant" | "system") => role,
                _ => continue,
            };
            let text = claude_text(&value);
            if text.trim().is_empty() {
                continue;
            }
            if push_limited_message(&mut messages, &mut total_chars, &role, &text) {
                return Ok(UnifiedThreadContent {
                    thread: info.clone(),
                    messages,
                });
            }
        }
    }

    Ok(UnifiedThreadContent {
        thread: info.clone(),
        messages,
    })
}

fn read_hermes_content(info: &UnifiedThreadInfo) -> Result<UnifiedThreadContent> {
    let path = PathBuf::from(&info.source_path);
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let messages = if ext == "json" {
        read_hermes_json_messages(&path)?
    } else {
        read_hermes_jsonl_messages(&path)?
    };
    Ok(UnifiedThreadContent {
        thread: info.clone(),
        messages,
    })
}

fn read_hermes_json_messages(path: &Path) -> Result<Vec<Value>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("读取 Hermes 会话失败: {}", path.display()))?;
    let value: Value = serde_json::from_str(&text).context("解析 Hermes JSON 失败")?;
    let mut out = Vec::new();
    let mut total_chars = 0usize;
    for message in value
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("system");
        let text = message_text(&message).unwrap_or_default();
        if text.trim().is_empty() {
            continue;
        }
        if push_limited_message(&mut out, &mut total_chars, role, &text) {
            break;
        }
    }
    Ok(out)
}

fn read_hermes_jsonl_messages(path: &Path) -> Result<Vec<Value>> {
    let file = fs::File::open(path).with_context(|| format!("打开文件失败: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    let mut total_chars = 0usize;
    for line in reader.lines() {
        let line = line.context("读取 JSONL 行失败")?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(path = %path.display(), "跳过无法解析的 Hermes JSONL 行: {err}");
                continue;
            }
        };
        let role = hermes_jsonl_role(&value).unwrap_or_else(|| "system".into());
        let text = hermes_jsonl_text(&value);
        if text.trim().is_empty() {
            continue;
        }
        if push_limited_message(&mut out, &mut total_chars, &role, &text) {
            break;
        }
    }
    Ok(out)
}

fn get_codex_content(native_id: &str) -> Result<UnifiedThreadContent> {
    let thread = crate::codex_threads::list_all()
        .context("读取 Codex 线程列表失败")?
        .into_iter()
        .find(|thread| thread.id == native_id)
        .map(codex_thread_to_unified)
        .ok_or_else(|| anyhow!("未找到 Codex 线程 {native_id}"))?;
    let content =
        crate::codex_threads::get_thread_content(native_id).context("读取 Codex 线程内容失败")?;
    let messages = content
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(UnifiedThreadContent { thread, messages })
}

fn codex_thread_to_unified(thread: crate::codex_threads::ThreadInfo) -> UnifiedThreadInfo {
    let title = if thread.title.trim().is_empty() {
        "(无标题)".into()
    } else {
        thread.title.clone()
    };
    UnifiedThreadInfo {
        thread_key: format!("codex:{}", thread.id),
        client_kind: AccountClientKind::Codex,
        client_label: client_label(&AccountClientKind::Codex),
        native_id: thread.id,
        title,
        model: String::new(),
        provider: thread.model_provider,
        cwd: String::new(),
        created_at_ms: thread.created_at_ms,
        updated_at_ms: thread.updated_at_ms,
        message_count: 0,
        preview: String::new(),
        source_path: String::new(),
        detail_available: true,
        delete_available: true,
    }
}

fn source_result(
    client_kind: AccountClientKind,
    available: bool,
    scan_paths: Vec<&Path>,
    diagnostics: Vec<String>,
    threads: Vec<UnifiedThreadInfo>,
    detail_available: bool,
    delete_available: bool,
) -> ThreadScanResult {
    let client_label = client_label(&client_kind);
    ThreadScanResult {
        status: ThreadSourceStatus {
            client_kind,
            client_label,
            available,
            scan_paths: scan_paths
                .into_iter()
                .map(|path| path.to_string_lossy().to_string())
                .collect(),
            count: threads.len(),
            diagnostics,
            detail_available,
            delete_available,
            migrate_available: false,
        },
        threads,
    }
}

fn placeholder_source(
    client_kind: AccountClientKind,
    scan_paths: Vec<String>,
    diagnostic: &str,
) -> ThreadScanResult {
    let client_label = client_label(&client_kind);
    ThreadScanResult {
        status: ThreadSourceStatus {
            client_kind,
            client_label,
            available: false,
            scan_paths,
            count: 0,
            diagnostics: vec![diagnostic.into()],
            detail_available: false,
            delete_available: false,
            migrate_available: false,
        },
        threads: Vec::new(),
    }
}

fn error_source(client_kind: AccountClientKind, diagnostic: String) -> ThreadScanResult {
    let client_label = client_label(&client_kind);
    ThreadScanResult {
        status: ThreadSourceStatus {
            client_kind,
            client_label,
            available: false,
            scan_paths: Vec::new(),
            count: 0,
            diagnostics: vec![diagnostic],
            detail_available: false,
            delete_available: false,
            migrate_available: false,
        },
        threads: Vec::new(),
    }
}

struct FileThreadInput {
    client_kind: AccountClientKind,
    native_id: String,
    title: String,
    model: String,
    provider: String,
    cwd: String,
    message_count: usize,
    preview: String,
}

fn file_thread(input: FileThreadInput, path: &Path) -> UnifiedThreadInfo {
    let (created_at_ms, updated_at_ms) = file_times(path);
    let slug = client_slug(&input.client_kind);
    let client_label = client_label(&input.client_kind);
    UnifiedThreadInfo {
        thread_key: format!("{slug}:{}:{}", input.native_id, stable_path_hash(path)),
        client_kind: input.client_kind,
        client_label,
        native_id: input.native_id,
        title: input.title,
        model: input.model,
        provider: input.provider,
        cwd: input.cwd,
        created_at_ms,
        updated_at_ms,
        message_count: input.message_count,
        preview: input.preview,
        source_path: path.to_string_lossy().to_string(),
        detail_available: true,
        delete_available: false,
    }
}

fn resolve_thread_info(
    threads: Vec<UnifiedThreadInfo>,
    native_id: &str,
    thread_key: Option<&str>,
) -> Option<UnifiedThreadInfo> {
    if let Some(key) = normalized_thread_key(thread_key) {
        if let Some(thread) = threads.iter().find(|thread| thread.thread_key == key) {
            return Some(thread.clone());
        }
    }
    threads
        .into_iter()
        .find(|thread| thread.native_id == native_id)
}

fn thread_key_matches(thread: &UnifiedThreadInfo, thread_key: Option<&str>) -> bool {
    normalized_thread_key(thread_key)
        .map(|key| thread.thread_key == key)
        .unwrap_or(false)
}

fn normalized_thread_key(thread_key: Option<&str>) -> Option<&str> {
    thread_key.map(str::trim).filter(|key| !key.is_empty())
}

fn record_by_native_id<'a>(
    records: &'a [ClaudeThreadRecord],
    native_id: &str,
) -> Option<&'a ClaudeThreadRecord> {
    records
        .iter()
        .find(|record| record.info.native_id == native_id)
}

fn grouped_thread_key(
    client_kind: &AccountClientKind,
    native_id: &str,
    paths: &[PathBuf],
) -> String {
    format!(
        "{}:{}:{}",
        client_slug(client_kind),
        native_id,
        stable_paths_hash(paths)
    )
}

fn sort_paths_by_time(paths: &mut [PathBuf]) {
    paths.sort_by_cached_key(|path| {
        let (created_at_ms, updated_at_ms) = file_times(path);
        (
            created_at_ms.or(updated_at_ms).unwrap_or(0),
            path.to_string_lossy().to_string(),
        )
    });
}

fn min_time(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn max_time(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn stable_path_hash(path: &Path) -> String {
    stable_hash_hex(path.to_string_lossy().as_bytes())
}

fn stable_paths_hash(paths: &[PathBuf]) -> String {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path.to_string_lossy().as_bytes());
        bytes.push(0);
    }
    stable_hash_hex(&bytes)
}

fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn client_label(client_kind: &AccountClientKind) -> String {
    profile_for_kind(client_kind).label
}

fn client_slug(client_kind: &AccountClientKind) -> &'static str {
    match client_kind {
        AccountClientKind::Codex => "codex",
        AccountClientKind::ClaudeCode => "claude_code",
        AccountClientKind::Openclaw => "openclaw",
        AccountClientKind::Hermes => "hermes",
        AccountClientKind::GenericClient => "generic_client",
    }
}

fn claude_role(value: &Value) -> Option<String> {
    value
        .get("message")
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        .or_else(|| value.get("role").and_then(Value::as_str))
        .or_else(|| value.get("type").and_then(Value::as_str))
        .map(str::to_string)
}

fn claude_text(value: &Value) -> String {
    value
        .get("message")
        .and_then(|message| message.get("content"))
        .map(content_to_text)
        .or_else(|| value.get("content").map(content_to_text))
        .unwrap_or_default()
}

fn hermes_jsonl_role(value: &Value) -> Option<String> {
    value
        .get("role")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

fn hermes_jsonl_text(value: &Value) -> String {
    value
        .get("content")
        .map(content_to_text)
        .or_else(|| {
            value
                .get("message")
                .and_then(|message| message.get("content"))
                .map(content_to_text)
        })
        .unwrap_or_default()
}

fn message_text(value: &Value) -> Option<String> {
    value
        .get("content")
        .map(content_to_text)
        .filter(|text| !text.trim().is_empty())
}

fn content_to_text(value: &Value) -> String {
    match value {
        Value::String(text) => truncate_text(text, MAX_MESSAGE_CHARS),
        Value::Array(items) => {
            let mut lines = Vec::new();
            for item in items {
                if let Some(text) = content_item_text(item) {
                    lines.push(text);
                }
            }
            truncate_text(&lines.join("\n"), MAX_MESSAGE_CHARS)
        }
        Value::Object(_) => content_item_text(value).unwrap_or_default(),
        _ => String::new(),
    }
}

fn content_item_text(item: &Value) -> Option<String> {
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    match item_type {
        "text" | "input_text" | "output_text" => item
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| item.get("content").and_then(Value::as_str))
            .map(|text| truncate_text(text, MAX_MESSAGE_CHARS)),
        _ => item
            .get("text")
            .and_then(Value::as_str)
            .map(|text| truncate_text(text, MAX_MESSAGE_CHARS)),
    }
}

fn push_limited_message(
    messages: &mut Vec<Value>,
    total_chars: &mut usize,
    role: &str,
    text: &str,
) -> bool {
    let truncated = truncate_text(text, MAX_MESSAGE_CHARS);
    let chars = truncated.len();
    if total_chars.saturating_add(chars) > MAX_TOTAL_CHARS {
        messages.push(text_message(
            "system",
            "线程内容过大，后续内容已停止加载。".to_string(),
        ));
        return true;
    }
    *total_chars += chars;
    messages.push(text_message(role, truncated));
    false
}

fn text_message(role: &str, text: String) -> Value {
    json!({
        "role": role,
        "payload": {
            "role": role,
            "content": [{ "type": "text", "text": text }]
        }
    })
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

fn find_files_with_extensions(
    root: &Path,
    extensions: &[&str],
    diagnostics: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_files(root, extensions, &mut out, diagnostics);
    out
}

fn collect_files(
    dir: &Path,
    extensions: &[&str],
    out: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<String>,
) {
    if out.len() >= MAX_SCAN_FILES {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            diagnostics.push(format!("读取目录失败 {}: {err}", dir.display()));
            tracing::warn!(dir = %dir.display(), "读取线程目录失败: {err}");
            return;
        }
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        if out.len() >= MAX_SCAN_FILES {
            diagnostics.push(format!("扫描文件数超过上限 {MAX_SCAN_FILES}，已停止"));
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, extensions, out, diagnostics);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if extensions.contains(&ext) {
            out.push(path);
        }
    }
}

fn file_times(path: &Path) -> (Option<i64>, Option<i64>) {
    match fs::metadata(path) {
        Ok(meta) => (
            meta.created().ok().and_then(system_time_ms),
            meta.modified().ok().and_then(system_time_ms),
        ),
        Err(_) => (None, None),
    }
}

fn system_time_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn home_path(parts: &[&str]) -> Result<PathBuf> {
    let mut path = crate::config::home_dir().context("无法确定 HOME 目录")?;
    for part in parts {
        path.push(part);
    }
    Ok(path)
}

fn home_path_lossy(parts: &[&str]) -> String {
    home_path(parts)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "deecodex-client-threads-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn claude_jsonl_threads_are_grouped_and_detail_is_text_only() {
        let dir = temp_dir("claude");
        let project = dir.join("project");
        fs::create_dir_all(&project).unwrap();
        let path = project.join("session.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, r#"{{"type":"permission-mode","sessionId":"s1"}}"#).unwrap();
        writeln!(
            file,
            r#"{{"type":"user","sessionId":"s1","cwd":"/tmp/demo","message":{{"role":"user","content":"第一问"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"assistant","sessionId":"s1","message":{{"role":"assistant","model":"claude-test","content":[{{"type":"text","text":"第一答"}}]}}}}"#
        )
        .unwrap();

        let result = scan_claude_threads(&dir);
        assert_eq!(result.status.count, 1);
        assert_eq!(result.threads[0].native_id, "s1");
        assert_eq!(result.threads[0].title, "第一问");
        assert_eq!(result.threads[0].message_count, 2);

        let content =
            read_claude_content_from_paths(&result.threads[0], std::slice::from_ref(&path))
                .unwrap();
        assert_eq!(content.messages.len(), 2);
        assert_eq!(content.messages[0]["role"], "user");
    }

    #[test]
    fn claude_jsonl_same_session_merges_files_and_detail() {
        let dir = temp_dir("claude-merge");
        let project = dir.join("project");
        fs::create_dir_all(&project).unwrap();
        let first = project.join("first.jsonl");
        let second = project.join("second.jsonl");
        fs::write(
            &first,
            "{\"type\":\"user\",\"sessionId\":\"same\",\"message\":{\"role\":\"user\",\"content\":\"第一问\"}}\n",
        )
        .unwrap();
        fs::write(
            &second,
            "{\"type\":\"assistant\",\"sessionId\":\"same\",\"message\":{\"role\":\"assistant\",\"content\":\"第二答\"}}\n",
        )
        .unwrap();

        let mut diagnostics = Vec::new();
        let records = collect_claude_thread_records(&dir, &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].info.message_count, 2);
        assert_eq!(records[0].info.title, "第一问");
        assert!(records[0].info.thread_key.starts_with("claude_code:same:"));

        let content = read_claude_content_from_paths(&records[0].info, &records[0].paths).unwrap();
        assert_eq!(content.messages.len(), 2);
        assert_eq!(
            content.messages[0]["payload"]["content"][0]["text"],
            "第一问"
        );
        assert_eq!(
            content.messages[1]["payload"]["content"][0]["text"],
            "第二答"
        );
    }

    #[test]
    fn claude_merged_title_uses_first_user_text() {
        let dir = temp_dir("claude-title");
        let first = dir.join("a.jsonl");
        let second = dir.join("b.jsonl");
        fs::write(
            &first,
            "{\"type\":\"assistant\",\"sessionId\":\"title\",\"message\":{\"role\":\"assistant\",\"content\":\"不是标题\"}}\n",
        )
        .unwrap();
        fs::write(
            &second,
            "{\"type\":\"user\",\"sessionId\":\"title\",\"message\":{\"role\":\"user\",\"content\":\"真正标题\"}}\n",
        )
        .unwrap();

        let result = scan_claude_threads(&dir);
        assert_eq!(result.status.count, 1);
        assert_eq!(result.threads[0].title, "真正标题");
    }

    #[test]
    fn claude_jsonl_keeps_valid_lines_when_some_lines_are_broken() {
        let dir = temp_dir("claude-broken");
        let path = dir.join("broken.jsonl");
        fs::write(
            &path,
            "not json\n{\"type\":\"user\",\"sessionId\":\"s2\",\"message\":{\"role\":\"user\",\"content\":\"可用\"}}\n",
        )
        .unwrap();

        let result = scan_claude_threads(&dir);
        assert_eq!(result.status.count, 1);
        assert!(result.threads[0].preview.contains("跳过 1 行损坏记录"));
    }

    #[test]
    fn hermes_json_threads_read_messages() {
        let dir = temp_dir("hermes-json");
        let path = dir.join("session_1.json");
        fs::write(
            &path,
            r#"{"session_id":"h1","model":"MiniMax","base_url":"https://example.test","messages":[{"role":"user","content":"你好"},{"role":"assistant","content":"你好呀"}]}"#,
        )
        .unwrap();

        let result = scan_hermes_threads(&dir);
        assert_eq!(result.status.count, 1);
        assert_eq!(result.threads[0].native_id, "h1");
        assert_eq!(result.threads[0].title, "你好");

        let content = read_hermes_content(&result.threads[0]).unwrap();
        assert_eq!(content.messages.len(), 2);
    }

    #[test]
    fn hermes_jsonl_threads_are_supported() {
        let dir = temp_dir("hermes-jsonl");
        let path = dir.join("session_2.jsonl");
        fs::write(
            &path,
            "{\"session_id\":\"h2\",\"role\":\"user\",\"content\":\"问题\"}\n{\"session_id\":\"h2\",\"role\":\"assistant\",\"content\":\"答案\"}\n",
        )
        .unwrap();

        let result = scan_hermes_threads(&dir);
        assert_eq!(result.status.count, 1);
        assert_eq!(result.threads[0].native_id, "h2");
        assert_eq!(result.threads[0].message_count, 2);
    }

    #[test]
    fn hermes_duplicate_native_id_uses_thread_key_first() {
        let dir = temp_dir("hermes-dup");
        let first = dir.join("first.json");
        let second = dir.join("second.json");
        fs::write(
            &first,
            r#"{"session_id":"dup","messages":[{"role":"user","content":"第一条"}]}"#,
        )
        .unwrap();
        fs::write(
            &second,
            r#"{"session_id":"dup","messages":[{"role":"user","content":"第二条"}]}"#,
        )
        .unwrap();

        let result = scan_hermes_threads(&dir);
        assert_eq!(result.status.count, 2);
        let second_info = result
            .threads
            .iter()
            .find(|thread| thread.source_path == second.to_string_lossy())
            .unwrap();
        let resolved =
            resolve_thread_info(result.threads.clone(), "dup", Some(&second_info.thread_key))
                .unwrap();
        assert_eq!(resolved.source_path, second.to_string_lossy());

        let content = read_hermes_content(&resolved).unwrap();
        assert_eq!(
            content.messages[0]["payload"]["content"][0]["text"],
            "第二条"
        );
    }

    #[test]
    fn missing_directory_reports_unavailable_source() {
        let dir = temp_dir("missing").join("none");
        let result = scan_claude_threads(&dir);
        assert!(!result.status.available);
        assert_eq!(result.status.count, 0);
        assert!(!result.status.diagnostics.is_empty());
    }

    #[test]
    fn placeholders_are_status_only() {
        let result = placeholder_source(
            AccountClientKind::Openclaw,
            vec!["/tmp/openclaw".into()],
            "暂未适配",
        );
        assert!(!result.status.available);
        assert!(!result.status.detail_available);
        assert!(result.threads.is_empty());
    }

    #[test]
    fn long_messages_are_truncated_in_detail() {
        let dir = temp_dir("long");
        let path = dir.join("long.json");
        let long = "a".repeat(MAX_MESSAGE_CHARS + 10);
        fs::write(
            &path,
            json!({
                "session_id": "h-long",
                "messages": [{ "role": "user", "content": long }]
            })
            .to_string(),
        )
        .unwrap();

        let result = scan_hermes_threads(&dir);
        let content = read_hermes_content(&result.threads[0]).unwrap();
        let text = content.messages[0]["payload"]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(text.contains("已截断"));
    }
}
