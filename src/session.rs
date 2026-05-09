use dashmap::DashMap;
use serde_json::Value;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::types::ChatMessage;

/// 响应摘要信息
pub struct ResponseInfo {
    pub id: String,
    pub status: String,
}

/// 对话摘要信息
pub struct ConversationInfo {
    pub id: String,
    pub message_count: usize,
}

/// In-memory session store: response_id → messages, call_id → reasoning_content,
/// and turn-level reasoning fingerprints.
///
/// Maximum number of sessions to retain. Oldest entries are evicted
/// when this limit is exceeded.
const MAX_SESSIONS: usize = 256;

/// Maximum number of reasoning entries to retain.
const MAX_REASONING: usize = 512;

/// Maximum number of turn-reasoning entries to retain.
const MAX_TURN_REASONING: usize = 256;

/// NOTE: All data is in-memory and lost on restart. For conversations in
/// progress, Codex replays the full conversation in the `input` array,
/// so the relay reconstructs history from the replay. The reasoning_content
/// indices help recover thinking content that DeepSeek requires to be
/// passed back in subsequent requests.
#[derive(Clone)]
pub struct SessionStore {
    inner: Arc<DashMap<String, Vec<ChatMessage>>>,
    responses: Arc<DashMap<String, Value>>,
    input_items: Arc<DashMap<String, Vec<Value>>>,
    conversations: Arc<DashMap<String, Vec<ChatMessage>>>,
    conversation_items: Arc<DashMap<String, Vec<Value>>>,
    reasoning: Arc<DashMap<String, String>>,
    /// fingerprint → reasoning_content for turn-level recovery
    turn_reasoning: Arc<DashMap<u64, String>>,
    response_order: Arc<Mutex<VecDeque<String>>>,
    conversation_order: Arc<Mutex<VecDeque<String>>>,
    reasoning_order: Arc<Mutex<VecDeque<String>>>,
    turn_reasoning_order: Arc<Mutex<VecDeque<u64>>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            responses: Arc::new(DashMap::new()),
            input_items: Arc::new(DashMap::new()),
            conversations: Arc::new(DashMap::new()),
            conversation_items: Arc::new(DashMap::new()),
            reasoning: Arc::new(DashMap::new()),
            turn_reasoning: Arc::new(DashMap::new()),
            response_order: Arc::new(Mutex::new(VecDeque::new())),
            conversation_order: Arc::new(Mutex::new(VecDeque::new())),
            reasoning_order: Arc::new(Mutex::new(VecDeque::new())),
            turn_reasoning_order: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn store_reasoning(&self, call_id: String, reasoning: String) {
        if !reasoning.is_empty() {
            let is_new = self.reasoning.insert(call_id.clone(), reasoning).is_none();
            if is_new {
                self.reasoning_order.lock().unwrap().push_back(call_id);
            }
            if self.reasoning.len() >= MAX_REASONING {
                if let Some(oldest) = self.reasoning_order.lock().unwrap().pop_front() {
                    self.reasoning.remove(&oldest);
                }
            }
        }
    }

    pub fn get_reasoning(&self, call_id: &str) -> Option<String> {
        self.reasoning.get(call_id).map(|v| v.clone())
    }

    /// Store turn-level reasoning. Uses a combined fingerprint of:
    /// - assistant text content (if any)
    /// - tool call IDs (if any)
    ///
    /// This handles both text-only and tool-call-only assistant messages.
    pub fn store_turn_reasoning(
        &self,
        _prior: &[ChatMessage],
        assistant: &ChatMessage,
        reasoning: String,
    ) {
        if !reasoning.is_empty() {
            if self.turn_reasoning.len() >= MAX_TURN_REASONING {
                if let Some(oldest) = self.turn_reasoning_order.lock().unwrap().pop_front() {
                    self.turn_reasoning.remove(&oldest);
                }
            }
            let combined_key = Self::turn_key(assistant);
            if self
                .turn_reasoning
                .insert(combined_key, reasoning.clone())
                .is_none()
            {
                self.turn_reasoning_order
                    .lock()
                    .unwrap()
                    .push_back(combined_key);
            }
            // Also store under content-only key for text-only assistant lookup
            let content = assistant
                .content
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !content.is_empty() {
                let content_key = Self::content_key(content);
                if self
                    .turn_reasoning
                    .insert(content_key, reasoning.clone())
                    .is_none()
                {
                    self.turn_reasoning_order
                        .lock()
                        .unwrap()
                        .push_back(content_key);
                }
            }
            // Also store under each individual tool call_id for direct lookup
            if let Some(tcs) = &assistant.tool_calls {
                for tc in tcs {
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            self.store_reasoning(id.to_string(), reasoning.clone());
                        }
                    }
                }
            }
        }
    }

    /// Look up reasoning_content for an assistant turn.
    /// Uses a combined key of text content + tool call IDs.
    pub fn get_turn_reasoning(
        &self,
        _prior: &[ChatMessage],
        assistant: &ChatMessage,
    ) -> Option<String> {
        // Try the combined key first (text + tool call IDs)
        let key = Self::turn_key(assistant);
        if let Some(v) = self.turn_reasoning.get(&key) {
            return Some(v.clone());
        }
        // Fallback: try content-only key (for text-only assistant messages)
        let content = assistant
            .content
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !content.is_empty() {
            let content_key = Self::content_key(content);
            if let Some(v) = self.turn_reasoning.get(&content_key) {
                return Some(v.clone());
            }
        }
        None
    }

    /// Combined fingerprint: text content + sorted tool call IDs.
    fn turn_key(assistant: &ChatMessage) -> u64 {
        let mut hasher = DefaultHasher::new();
        assistant
            .content
            .as_ref()
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .hash(&mut hasher);
        if let Some(tcs) = &assistant.tool_calls {
            let mut ids: Vec<&str> = tcs
                .iter()
                .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()))
                .collect();
            ids.sort_unstable();
            for id in ids {
                id.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Hash text content only (fallback for text-only lookups).
    fn content_key(content: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        hasher.finish()
    }

    /// Scan previously reconstructed messages for reasoning_content.
    /// Used as a fallback when per-call_id and turn_reasoning lookups fail
    /// (e.g. after relay restart). Walks messages in reverse to find the
    /// most recent assistant message whose tool_calls match.
    pub fn scan_history_reasoning(
        &self,
        messages: &[ChatMessage],
        tool_call_ids: &[String],
    ) -> Option<String> {
        if tool_call_ids.is_empty() {
            return None;
        }
        for msg in messages.iter().rev() {
            if msg.role != "assistant" {
                continue;
            }
            if let Some(ref tcs) = msg.tool_calls {
                let msg_ids: Vec<&str> = tcs
                    .iter()
                    .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()))
                    .collect();
                if msg_ids.len() == tool_call_ids.len()
                    && msg_ids
                        .iter()
                        .all(|id| tool_call_ids.iter().any(|tid| tid == id))
                {
                    return msg.reasoning_content.clone();
                }
            }
        }
        None
    }

    pub fn new_id(&self) -> String {
        format!("resp_{}", Uuid::new_v4().simple())
    }

    pub fn save_with_id(&self, id: String, messages: Vec<ChatMessage>) {
        let is_new = self.inner.insert(id.clone(), messages).is_none();
        if is_new {
            self.response_order.lock().unwrap().push_back(id);
        }
        if self.inner.len() >= MAX_SESSIONS {
            if let Some(oldest) = self.response_order.lock().unwrap().pop_front() {
                self.inner.remove(&oldest);
                self.responses.remove(&oldest);
                self.input_items.remove(&oldest);
            }
        }
    }

    pub fn save_response(&self, id: String, response: Value) {
        let is_new = self.responses.insert(id.clone(), response).is_none();
        if is_new {
            self.response_order.lock().unwrap().push_back(id);
        }
        if self.responses.len() >= MAX_SESSIONS {
            if let Some(oldest) = self.response_order.lock().unwrap().pop_front() {
                self.responses.remove(&oldest);
                self.inner.remove(&oldest);
                self.input_items.remove(&oldest);
            }
        }
    }

    pub fn get_response(&self, response_id: &str) -> Option<Value> {
        self.responses.get(response_id).map(|v| v.clone())
    }

    pub fn response_status(&self, response_id: &str) -> Option<String> {
        self.responses
            .get(response_id)
            .and_then(|v| v.get("status").and_then(Value::as_str).map(str::to_string))
    }

    pub fn delete_response(&self, response_id: &str) -> bool {
        let removed = self.responses.remove(response_id).is_some();
        self.inner.remove(response_id);
        self.input_items.remove(response_id);
        removed
    }

    pub fn save_input_items(&self, response_id: String, items: Vec<Value>) {
        let is_new = self
            .input_items
            .insert(response_id.clone(), items)
            .is_none();
        if is_new {
            self.response_order.lock().unwrap().push_back(response_id);
        }
        if self.input_items.len() >= MAX_SESSIONS {
            if let Some(oldest) = self.response_order.lock().unwrap().pop_front() {
                self.input_items.remove(&oldest);
                self.inner.remove(&oldest);
                self.responses.remove(&oldest);
            }
        }
    }

    pub fn get_input_items(&self, response_id: &str) -> Option<Vec<Value>> {
        self.input_items.get(response_id).map(|v| v.clone())
    }

    pub fn save_conversation(&self, conversation_id: String, messages: Vec<ChatMessage>) {
        let is_new = self
            .conversations
            .insert(conversation_id.clone(), messages)
            .is_none();
        if is_new {
            self.conversation_order
                .lock()
                .unwrap()
                .push_back(conversation_id);
        }
        if self.conversations.len() >= MAX_SESSIONS {
            if let Some(oldest) = self.conversation_order.lock().unwrap().pop_front() {
                self.conversations.remove(&oldest);
                self.conversation_items.remove(&oldest);
            }
        }
    }

    pub fn save_conversation_items(&self, conversation_id: String, items: Vec<Value>) {
        let is_new = self
            .conversation_items
            .insert(conversation_id.clone(), items)
            .is_none();
        if is_new {
            self.conversation_order
                .lock()
                .unwrap()
                .push_back(conversation_id);
        }
        if self.conversation_items.len() >= MAX_SESSIONS {
            if let Some(oldest) = self.conversation_order.lock().unwrap().pop_front() {
                self.conversation_items.remove(&oldest);
                self.conversations.remove(&oldest);
            }
        }
    }

    pub fn get_conversation_items(&self, conversation_id: &str) -> Vec<Value> {
        self.conversation_items
            .get(conversation_id)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn conversation_exists(&self, conversation_id: &str) -> bool {
        self.conversations.contains_key(conversation_id)
            || self.conversation_items.contains_key(conversation_id)
    }

    pub fn delete_conversation(&self, conversation_id: &str) -> bool {
        let removed_messages = self.conversations.remove(conversation_id).is_some();
        let removed_items = self.conversation_items.remove(conversation_id).is_some();
        removed_messages || removed_items
    }

    // ── 列表查询 ──────────────────────────────────────────────

    /// 列出所有响应及其状态
    pub fn list_responses(&self) -> Vec<ResponseInfo> {
        self.responses
            .iter()
            .map(|entry| {
                let status = entry
                    .value()
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                ResponseInfo {
                    id: entry.key().clone(),
                    status,
                }
            })
            .collect()
    }

    /// 列出所有对话及其消息数量
    pub fn list_conversations(&self) -> Vec<ConversationInfo> {
        self.conversations
            .iter()
            .map(|entry| ConversationInfo {
                id: entry.key().clone(),
                message_count: entry.value().len(),
            })
            .collect()
    }

    // ── 带数据提取的删除 ──────────────────────────────────────

    /// 删除响应，同时返回删除前的数据用于备份。
    /// 返回 (消息列表, 完整响应, 输入项列表)
    pub fn delete_response_with_data(
        &self,
        response_id: &str,
    ) -> Option<(Vec<ChatMessage>, Value, Vec<Value>)> {
        let messages = self.inner.get(response_id).map(|v| v.clone());
        let response = self.responses.get(response_id).map(|v| v.clone());
        let items = self.input_items.get(response_id).map(|v| v.clone());

        // 提取完毕后执行删除
        self.inner.remove(response_id);
        self.responses.remove(response_id);
        self.input_items.remove(response_id);

        match (messages, response, items) {
            (Some(m), Some(r), Some(i)) => Some((m, r, i)),
            _ => None,
        }
    }

    /// 删除对话，同时返回删除前的数据用于备份。
    /// 返回 (消息列表, 对话项列表)
    pub fn delete_conversation_with_data(
        &self,
        conversation_id: &str,
    ) -> Option<(Vec<ChatMessage>, Vec<Value>)> {
        let messages = self.conversations.get(conversation_id).map(|v| v.clone());
        let items = self
            .conversation_items
            .get(conversation_id)
            .map(|v| v.clone());

        self.conversations.remove(conversation_id);
        self.conversation_items.remove(conversation_id);

        match (messages, items) {
            (Some(m), Some(i)) => Some((m, i)),
            _ => None,
        }
    }

    // ── 撤销删除 ──────────────────────────────────────────────

    /// 恢复被删除的响应
    pub fn undo_delete_response(
        &self,
        response_id: &str,
        messages: Vec<ChatMessage>,
        response: Value,
        input_items: Vec<Value>,
    ) {
        let rid = response_id.to_string();
        let is_new = self
            .inner
            .insert(rid.clone(), messages)
            .is_none()
            || self
                .responses
                .insert(rid.clone(), response)
                .is_none()
            || self
                .input_items
                .insert(rid.clone(), input_items)
                .is_none();
        if is_new {
            self.response_order.lock().unwrap().push_back(rid);
        }
    }

    /// 恢复被删除的对话
    pub fn undo_delete_conversation(
        &self,
        conversation_id: &str,
        messages: Vec<ChatMessage>,
        items: Vec<Value>,
    ) {
        let cid = conversation_id.to_string();
        let is_new = self
            .conversations
            .insert(cid.clone(), messages)
            .is_none()
            || self
                .conversation_items
                .insert(cid.clone(), items)
                .is_none();
        if is_new {
            self.conversation_order.lock().unwrap().push_back(cid);
        }
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;

    fn msg(role: &str, content: Option<&str>) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: content.map(|s| serde_json::Value::String(s.to_string())),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    #[test]
    fn test_store_and_get_reasoning() {
        let store = SessionStore::new();
        store.store_reasoning("call_1".into(), "think".into());
        assert_eq!(store.get_reasoning("call_1"), Some("think".into()));
    }

    #[test]
    fn test_turn_reasoning_with_tool_calls_only() {
        let store = SessionStore::new();
        let mut assistant = msg("assistant", None);
        assistant.tool_calls = Some(vec![serde_json::json!({
            "id": "tc_a",
            "type": "function",
            "function": {"name": "exec", "arguments": "{}"}
        })]);
        store.store_turn_reasoning(&[], &assistant, "tool_reason".into());
        let recovered = store.get_turn_reasoning(&[], &assistant);
        assert_eq!(recovered, Some("tool_reason".into()));
    }

    #[test]
    fn test_turn_reasoning_with_text_and_tools() {
        let store = SessionStore::new();
        let mut assistant = msg("assistant", Some("hello"));
        assistant.tool_calls = Some(vec![serde_json::json!({
            "id": "tc_b",
            "type": "function",
            "function": {"name": "read", "arguments": "{}"}
        })]);
        store.store_turn_reasoning(&[], &assistant, "mixed_reason".into());
        let recovered = store.get_turn_reasoning(&[], &assistant);
        assert_eq!(recovered, Some("mixed_reason".into()));
    }

    #[test]
    fn test_scan_history_reasoning_finds_match() {
        let store = SessionStore::new();
        let mut assistant = msg("assistant", None);
        assistant.tool_calls = Some(vec![serde_json::json!({
            "id": "scan_1",
            "type": "function",
            "function": {"name": "x", "arguments": "{}"}
        })]);
        assistant.reasoning_content = Some("found_it".into());
        let messages = vec![msg("user", Some("q")), assistant.clone()];
        let result = store.scan_history_reasoning(&messages, &["scan_1".into()]);
        assert_eq!(result, Some("found_it".into()));
    }

    #[test]
    fn test_scan_history_reasoning_no_match() {
        let store = SessionStore::new();
        let messages = vec![msg("user", Some("q")), msg("assistant", Some("a"))];
        let result = store.scan_history_reasoning(&messages, &["nope".into()]);
        assert_eq!(result, None);
    }

    #[test]
    fn test_turn_key_different_for_different_tool_ids() {
        let mut a = msg("assistant", None);
        a.tool_calls = Some(vec![
            serde_json::json!({"id": "id_a", "type": "function", "function": {"name": "f", "arguments": "{}"}}),
        ]);
        let mut b = msg("assistant", None);
        b.tool_calls = Some(vec![
            serde_json::json!({"id": "id_b", "type": "function", "function": {"name": "f", "arguments": "{}"}}),
        ]);
        assert_ne!(SessionStore::turn_key(&a), SessionStore::turn_key(&b));
    }

    #[test]
    fn test_turn_key_same_for_same_tool_ids_different_order() {
        let mut a = msg("assistant", None);
        a.tool_calls = Some(vec![
            serde_json::json!({"id": "id_a", "type": "function", "function": {"name": "f", "arguments": "{}"}}),
            serde_json::json!({"id": "id_b", "type": "function", "function": {"name": "g", "arguments": "{}"}}),
        ]);
        let mut b = msg("assistant", None);
        b.tool_calls = Some(vec![
            serde_json::json!({"id": "id_b", "type": "function", "function": {"name": "g", "arguments": "{}"}}),
            serde_json::json!({"id": "id_a", "type": "function", "function": {"name": "f", "arguments": "{}"}}),
        ]);
        assert_eq!(SessionStore::turn_key(&a), SessionStore::turn_key(&b));
    }

    // ------------------------------------------------------------------
    //  new_id()
    // ------------------------------------------------------------------

    #[test]
    fn test_new_id_not_empty() {
        let store = SessionStore::new();
        let id = store.new_id();
        assert!(!id.is_empty());
        assert!(id.starts_with("resp_"));
    }

    #[test]
    fn test_new_id_unique() {
        let store = SessionStore::new();
        let id1 = store.new_id();
        let id2 = store.new_id();
        assert_ne!(id1, id2);
    }

    // ------------------------------------------------------------------
    //  save_with_id()  (stores messages into `inner`; no public getter)
    // ------------------------------------------------------------------

    #[test]
    fn test_save_with_id_no_panic() {
        let store = SessionStore::new();
        store.save_with_id(store.new_id(), vec![msg("user", Some("hello"))]);
    }

    #[test]
    fn test_save_with_id_eviction() {
        let store = SessionStore::new();
        for i in 0..MAX_SESSIONS {
            store.save_with_id(format!("key_{}", i), vec![msg("user", Some("data"))]);
        }
        store.save_with_id("overflow".into(), vec![msg("user", Some("data"))]);
    }

    // ------------------------------------------------------------------
    //  save_response() / get_response() / response_status()
    // ------------------------------------------------------------------

    #[test]
    fn test_save_response_and_get_response() {
        let store = SessionStore::new();
        let id = store.new_id();
        let response = serde_json::json!({"choices": [{"text": "hi"}]});
        store.save_response(id.clone(), response.clone());
        assert_eq!(store.get_response(&id), Some(response));
    }

    #[test]
    fn test_get_response_missing() {
        let store = SessionStore::new();
        assert_eq!(store.get_response("nonexistent"), None);
    }

    #[test]
    fn test_response_status() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_response(id.clone(), serde_json::json!({"status": "completed"}));
        assert_eq!(store.response_status(&id), Some("completed".into()));
    }

    #[test]
    fn test_response_status_no_status_field() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_response(id.clone(), serde_json::json!({"unrelated": 42}));
        assert_eq!(store.response_status(&id), None);
    }

    #[test]
    fn test_response_status_missing() {
        let store = SessionStore::new();
        assert_eq!(store.response_status("nonexistent"), None);
    }

    #[test]
    fn test_delete_response_existing() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_response(id.clone(), serde_json::json!({"status": "done"}));
        assert!(store.delete_response(&id));
        assert_eq!(store.get_response(&id), None);
    }

    #[test]
    fn test_delete_response_nonexistent_is_noop() {
        let store = SessionStore::new();
        assert!(!store.delete_response("nonexistent"));
    }

    // ------------------------------------------------------------------
    //  save_input_items() / get_input_items()
    // ------------------------------------------------------------------

    #[test]
    fn test_save_and_get_input_items() {
        let store = SessionStore::new();
        let id = store.new_id();
        let items = vec![serde_json::json!({"role": "user"})];
        store.save_input_items(id.clone(), items.clone());
        assert_eq!(store.get_input_items(&id), Some(items));
    }

    #[test]
    fn test_get_input_items_missing() {
        let store = SessionStore::new();
        assert_eq!(store.get_input_items("nonexistent"), None);
    }

    #[test]
    fn test_save_input_items_eviction() {
        let store = SessionStore::new();
        for i in 0..MAX_SESSIONS {
            store.save_input_items(format!("key_{}", i), vec![serde_json::json!({"n": i})]);
        }
        store.save_input_items("overflow".into(), vec![serde_json::json!({"n": -1})]);
        assert_eq!(
            store.get_input_items("overflow"),
            Some(vec![serde_json::json!({"n": -1})])
        );
    }

    // ------------------------------------------------------------------
    //  save_conversation() / conversation_exists()
    // ------------------------------------------------------------------

    #[test]
    fn test_save_conversation_and_exists() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_conversation(id.clone(), vec![msg("user", Some("hi"))]);
        assert!(store.conversation_exists(&id));
    }

    #[test]
    fn test_conversation_exists_from_items() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_conversation_items(id.clone(), vec![serde_json::json!({"type": "text"})]);
        assert!(store.conversation_exists(&id));
    }

    #[test]
    fn test_conversation_exists_missing() {
        let store = SessionStore::new();
        assert!(!store.conversation_exists("nonexistent"));
    }

    // ------------------------------------------------------------------
    //  save_conversation_items() / get_conversation_items()
    // ------------------------------------------------------------------

    #[test]
    fn test_save_and_get_conversation_items() {
        let store = SessionStore::new();
        let id = store.new_id();
        let items = vec![serde_json::json!({"type": "text"})];
        store.save_conversation_items(id.clone(), items.clone());
        assert_eq!(store.get_conversation_items(&id), items);
    }

    #[test]
    fn test_get_conversation_items_missing_returns_empty() {
        let store = SessionStore::new();
        assert!(store.get_conversation_items("nonexistent").is_empty());
    }

    // ------------------------------------------------------------------
    //  delete_conversation()
    // ------------------------------------------------------------------

    #[test]
    fn test_delete_conversation_existing() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_conversation(id.clone(), vec![msg("user", Some("bye"))]);
        assert!(store.delete_conversation(&id));
        assert!(!store.conversation_exists(&id));
    }

    #[test]
    fn test_delete_conversation_with_items_only() {
        let store = SessionStore::new();
        let id = store.new_id();
        store.save_conversation_items(id.clone(), vec![serde_json::json!({"type": "text"})]);
        assert!(store.delete_conversation(&id));
        assert!(!store.conversation_exists(&id));
    }

    #[test]
    fn test_delete_conversation_nonexistent() {
        let store = SessionStore::new();
        assert!(!store.delete_conversation("nonexistent"));
    }
}
