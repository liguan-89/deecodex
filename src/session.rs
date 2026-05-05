use dashmap::DashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use uuid::Uuid;

use crate::types::ChatMessage;

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
    reasoning: Arc<DashMap<String, String>>,
    /// fingerprint → reasoning_content for turn-level recovery
    turn_reasoning: Arc<DashMap<u64, String>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            reasoning: Arc::new(DashMap::new()),
            turn_reasoning: Arc::new(DashMap::new()),
        }
    }

    pub fn store_reasoning(&self, call_id: String, reasoning: String) {
        if !reasoning.is_empty() {
            if self.reasoning.len() >= MAX_REASONING {
                // Evict oldest entry
                if let Some(key) = self.reasoning.iter().next().map(|e| e.key().clone()) {
                    self.reasoning.remove(&key);
                }
            }
            self.reasoning.insert(call_id, reasoning);
        }
    }

    pub fn get_reasoning(&self, call_id: &str) -> Option<String> {
        self.reasoning.get(call_id).map(|v| v.clone())
    }

    /// Store turn-level reasoning. Uses a combined fingerprint of:
    /// - assistant text content (if any)
    /// - tool call IDs (if any)
    /// This handles both text-only and tool-call-only assistant messages.
    pub fn store_turn_reasoning(&self, _prior: &[ChatMessage], assistant: &ChatMessage, reasoning: String) {
        if !reasoning.is_empty() {
            if self.turn_reasoning.len() >= MAX_TURN_REASONING {
                if let Some(key) = self.turn_reasoning.iter().next().map(|e| e.key().clone()) {
                    self.turn_reasoning.remove(&key);
                }
            }
            let combined_key = Self::turn_key(assistant);
            self.turn_reasoning.insert(combined_key, reasoning.clone());
            // Also store under content-only key for text-only assistant lookup
            let content = assistant.content.as_ref().and_then(|v| v.as_str()).unwrap_or("");
            if !content.is_empty() {
                self.turn_reasoning.insert(Self::content_key(content), reasoning.clone());
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
    pub fn get_turn_reasoning(&self, _prior: &[ChatMessage], assistant: &ChatMessage) -> Option<String> {
        // Try the combined key first (text + tool call IDs)
        let key = Self::turn_key(assistant);
        if let Some(v) = self.turn_reasoning.get(&key) {
            return Some(v.clone());
        }
        // Fallback: try content-only key (for text-only assistant messages)
        let content = assistant.content.as_ref().and_then(|v| v.as_str()).unwrap_or("");
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
        assistant.content.as_ref().and_then(|v| v.as_str()).unwrap_or("").hash(&mut hasher);
        if let Some(tcs) = &assistant.tool_calls {
            for tc in tcs {
                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                    id.hash(&mut hasher);
                }
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
                    && msg_ids.iter().all(|id| tool_call_ids.iter().any(|tid| tid == id))
                {
                    return msg.reasoning_content.clone();
                }
            }
        }
        None
    }

    pub fn get_history(&self, response_id: &str) -> Vec<ChatMessage> {
        self.inner
            .get(response_id)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    pub fn new_id(&self) -> String {
        format!("resp_{}", Uuid::new_v4().simple())
    }

    pub fn save_with_id(&self, id: String, messages: Vec<ChatMessage>) {
        if self.inner.len() >= MAX_SESSIONS {
            if let Some(key) = self.inner.iter().next().map(|e| e.key().clone()) {
                self.inner.remove(&key);
            }
        }
        self.inner.insert(id, messages);
    }

    pub fn save(&self, messages: Vec<ChatMessage>) -> String {
        let id = self.new_id();
        self.inner.insert(id.clone(), messages);
        id
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
        a.tool_calls = Some(vec![serde_json::json!({"id": "id_a", "type": "function", "function": {"name": "f", "arguments": "{}"}})]);
        let mut b = msg("assistant", None);
        b.tool_calls = Some(vec![serde_json::json!({"id": "id_b", "type": "function", "function": {"name": "f", "arguments": "{}"}})]);
        assert_ne!(SessionStore::turn_key(&a), SessionStore::turn_key(&b));
    }
}
