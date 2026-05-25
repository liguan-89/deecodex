use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

/// 列出所有活跃会话
pub async fn list_sessions_impl(manager: State<'_, ServerManager>) -> Result<Value, String> {
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;
    let responses = state.sessions.list_responses();
    let conversations = state.sessions.list_conversations();
    Ok(json!({
        "responses": responses.iter().map(|r| json!({"id": r.id, "status": r.status})).collect::<Vec<_>>(),
        "conversations": conversations.iter().map(|c| json!({"id": c.id, "message_count": c.message_count})).collect::<Vec<_>>(),
    }))
}

/// 删除会话（先备份）
pub async fn delete_session_impl(
    manager: State<'_, ServerManager>,
    session_type: String,
    session_id: String,
) -> Result<Value, String> {
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;

    let backup_store = deecodex::backup_store::BackupStore::new(state.data_dir.join("backups"))
        .map_err(|e| format!("备份存储初始化失败: {e}"))?;

    match session_type.as_str() {
        "responses" => {
            if let Some((messages, response, input_items)) =
                state.sessions.delete_response_with_data(&session_id)
            {
                let data =
                    json!({"messages": messages, "response": response, "input_items": input_items});
                let token = backup_store
                    .write_backup(&session_id, "response", &data)
                    .unwrap_or_default();
                Ok(
                    json!({"id": session_id, "object": "response.deleted", "deleted": true, "undo_token": token}),
                )
            } else {
                Err(format!("未找到响应: {}", session_id))
            }
        }
        "conversations" => {
            if let Some((messages, items)) =
                state.sessions.delete_conversation_with_data(&session_id)
            {
                let data = json!({"messages": messages, "items": items});
                let token = backup_store
                    .write_backup(&session_id, "conversation", &data)
                    .unwrap_or_default();
                Ok(
                    json!({"id": session_id, "object": "conversation.deleted", "deleted": true, "undo_token": token}),
                )
            } else {
                Err(format!("未找到对话: {}", session_id))
            }
        }
        _ => Err(format!("未知的会话类型: {}", session_type)),
    }
}

/// 撤销删除会话
pub async fn undo_delete_session_impl(
    manager: State<'_, ServerManager>,
    undo_token: String,
) -> Result<Value, String> {
    let guard = manager.app_state.lock().await;
    let state = guard.as_ref().ok_or("服务未启动")?;

    let backup_store = deecodex::backup_store::BackupStore::new(state.data_dir.join("backups"))
        .map_err(|e| format!("备份存储初始化失败: {e}"))?;
    let backup = backup_store
        .read_backup(&undo_token)
        .map_err(|e| format!("备份未找到: {e}"))?;

    let session_type = backup
        .get("session_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let data = &backup["data"];

    match session_type {
        "response" => {
            let response_id = backup["session_id"].as_str().unwrap_or("");
            let messages: Vec<deecodex::types::ChatMessage> =
                serde_json::from_value(data["messages"].clone())
                    .map_err(|e| format!("备份数据损坏: {e}"))?;
            let response = data["response"].clone();
            let input_items: Vec<Value> =
                serde_json::from_value(data["input_items"].clone()).unwrap_or_default();
            state
                .sessions
                .undo_delete_response(response_id, messages, response, input_items);
        }
        "conversation" => {
            let conversation_id = backup["session_id"].as_str().unwrap_or("");
            let messages: Vec<deecodex::types::ChatMessage> =
                serde_json::from_value(data["messages"].clone())
                    .map_err(|e| format!("备份数据损坏: {e}"))?;
            let items: Vec<Value> =
                serde_json::from_value(data["items"].clone()).unwrap_or_default();
            state
                .sessions
                .undo_delete_conversation(conversation_id, messages, items);
        }
        _ => return Err(format!("未知的会话类型: {}", session_type)),
    }

    let _ = backup_store.delete_backup(&undo_token);
    Ok(json!({"ok": true}))
}
