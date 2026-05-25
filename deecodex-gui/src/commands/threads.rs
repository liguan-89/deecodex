use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

pub async fn get_threads_status_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let status =
        deecodex::codex_threads::status(&data_dir).map_err(|e| format!("获取线程状态失败: {e}"))?;

    // 活跃 provider：迁移后为 "deecodex"，否则取数量最多的 provider
    let active_provider = if status.migrated {
        "deecodex"
    } else {
        status
            .summary
            .iter()
            .max_by_key(|s| s.count)
            .map(|s| s.provider.as_str())
            .unwrap_or("(空)")
    };

    Ok(json!({
        "summary": status.summary,
        "total": status.total,
        "non_deecodex_count": status.non_deecodex_count,
        "non_unified_count": status.non_deecodex_count,
        "provider_unified_count": status.provider_unified_count,
        "codex_visible_count": status.codex_visible_count,
        "missing_preview_count": status.missing_preview_count,
        "missing_user_event_count": status.missing_user_event_count,
        "current_cwd_visible_count": status.current_cwd_visible_count,
        "desktop_project_indexed_count": status.desktop_project_indexed_count,
        "desktop_project_pending_count": status.desktop_project_pending_count,
        "desktop_project_repair_blocked": status.desktop_project_repair_blocked,
        "desktop_recent_visible_count": status.desktop_recent_visible_count,
        "desktop_recent_pending_count": status.desktop_recent_pending_count,
        "desktop_recent_repair_blocked": status.desktop_recent_repair_blocked,
        "source_summary": status.source_summary,
        "context_window": status.context_window,
        "migrated": status.migrated,
        "calibration_needed": false,
        "active_provider": active_provider,
    }))
}

pub async fn list_threads_impl() -> Result<Value, String> {
    let threads =
        deecodex::codex_threads::list_all().map_err(|e| format!("获取线程列表失败: {e}"))?;
    serde_json::to_value(threads).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn get_thread_sources_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let sources = deecodex::client_threads::get_thread_sources(&data_dir);
    serde_json::to_value(sources).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn list_client_threads_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let list = deecodex::client_threads::list_client_threads(&data_dir);
    serde_json::to_value(list).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn migrate_threads_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let diff = deecodex::codex_threads::migrate(&data_dir).map_err(|e| format!("迁移失败: {e}"))?;
    serde_json::to_value(diff).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn restore_threads_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let diff = deecodex::codex_threads::restore(&data_dir).map_err(|e| format!("还原失败: {e}"))?;
    // 还原后若服务未运行，清理 Codex config.toml 中的 deecodex 注入
    if !manager.is_running().await {
        deecodex::codex_config::remove();
    }
    serde_json::to_value(diff).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn calibrate_threads_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    let diff =
        deecodex::codex_threads::calibrate(&data_dir).map_err(|e| format!("校准失败: {e}"))?;
    serde_json::to_value(diff).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn get_thread_content_impl(thread_id: String) -> Result<Value, String> {
    let content = deecodex::codex_threads::get_thread_content(&thread_id)
        .map_err(|e| format!("获取线程内容失败: {e}"))?;
    serde_json::to_value(content).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn get_client_thread_content_impl(
    client_kind: String,
    native_id: String,
    thread_key: Option<String>,
) -> Result<Value, String> {
    let kind = deecodex::client_threads::parse_client_kind(&client_kind)
        .ok_or_else(|| format!("未知客户端类型: {client_kind}"))?;
    let content = deecodex::client_threads::get_client_thread_content(
        kind,
        &native_id,
        thread_key.as_deref(),
    )
    .map_err(|e| format!("获取线程内容失败: {e}"))?;
    serde_json::to_value(content).map_err(|e| format!("序列化失败: {e}"))
}

pub async fn delete_thread_impl(
    manager: State<'_, ServerManager>,
    thread_id: String,
) -> Result<Value, String> {
    let data_dir = manager.data_dir.lock().await.clone();
    deecodex::codex_threads::delete_thread(&data_dir, &thread_id)
        .map_err(|e| format!("删除线程失败: {e}"))?;
    Ok(json!({ "ok": true, "message": "线程已永久删除" }))
}
