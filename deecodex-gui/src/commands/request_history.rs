use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

fn request_history_filter(
    client_kind: Option<String>,
    account_id: Option<String>,
) -> deecodex::request_history::HistoryFilter {
    deecodex::request_history::HistoryFilter {
        client_kind: client_kind.filter(|v| !v.trim().is_empty()),
        account_id: account_id.filter(|v| !v.trim().is_empty()),
    }
}

pub async fn list_request_history_impl(
    manager: State<'_, ServerManager>,
    limit: Option<usize>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    let filter = request_history_filter(client_kind, account_id);
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        let entries = store.list(limit.unwrap_or(3000), &filter).await;
        return Ok(serde_json::to_value(entries).unwrap_or_default());
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let entries = state
        .request_history
        .list(limit.unwrap_or(100), &filter)
        .await;
    Ok(serde_json::to_value(entries).unwrap_or_default())
}

pub async fn clear_request_history_impl(
    manager: State<'_, ServerManager>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    let filter = request_history_filter(client_kind, account_id);
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        store.clear(&filter).await?;
        return Ok(json!({ "ok": true }));
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    state.request_history.clear(&filter).await?;
    Ok(json!({ "ok": true }))
}

pub async fn get_monthly_stats_impl(
    manager: State<'_, ServerManager>,
    limit: Option<usize>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    let filter = request_history_filter(client_kind, account_id);
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        let stats = store.list_monthly_stats(limit.unwrap_or(6), &filter).await;
        return Ok(serde_json::to_value(stats).unwrap_or_default());
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let stats = state
        .request_history
        .list_monthly_stats(limit.unwrap_or(6), &filter)
        .await;
    Ok(serde_json::to_value(stats).unwrap_or_default())
}

pub async fn get_request_stats_since_impl(
    manager: State<'_, ServerManager>,
    since: Option<u64>,
    client_kind: Option<String>,
    account_id: Option<String>,
) -> Result<Value, String> {
    let since_secs = since.unwrap_or(0);
    let filter = request_history_filter(client_kind, account_id);
    let rh = manager.request_history.lock().await;
    if let Some(store) = rh.as_ref() {
        let stats = store.stats_since(since_secs, &filter).await;
        return Ok(serde_json::to_value(stats).unwrap_or_default());
    }
    drop(rh);
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let stats = state.request_history.stats_since(since_secs, &filter).await;
    Ok(serde_json::to_value(stats).unwrap_or_default())
}
