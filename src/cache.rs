use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::Mutex;

use tracing::info;

use crate::types::ChatUsage;

/// Cached response data for a completed request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<CachedToolCall>,
    pub usage: Option<CachedUsage>,
    pub created_at: u64, // unix timestamp
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub reasoning_tokens: Option<u32>,
    pub cache_hit_tokens: Option<u32>,
    pub cache_miss_tokens: Option<u32>,
}

/// Convert ChatUsage (optional ref) to CachedUsage.
pub fn usage_to_cached(u: Option<&ChatUsage>) -> Option<CachedUsage> {
    u.map(|u| CachedUsage {
        prompt_tokens: u.prompt_tokens,
        completion_tokens: u.completion_tokens,
        total_tokens: u.total_tokens,
        reasoning_tokens: u
            .completion_tokens_details
            .as_ref()
            .and_then(|d| d.reasoning_tokens),
        cache_hit_tokens: u.prompt_cache_hit_tokens,
        cache_miss_tokens: u.prompt_cache_miss_tokens,
    })
}

/// Simple request cache keyed by serialized ChatRequest hash.
#[derive(Clone)]
pub struct RequestCache {
    inner: Arc<DashMap<u64, CachedResponse>>,
    order: Arc<Mutex<VecDeque<u64>>>,
    max_entries: usize,
}

impl RequestCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            order: Arc::new(Mutex::new(VecDeque::new())),
            max_entries,
        }
    }

    /// Compute a hash for the request payload (serialize to JSON then hash).
    pub fn hash_request(req: &impl Serialize) -> u64 {
        let json = serde_json::to_string(req).unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        json.hash(&mut hasher);
        hasher.finish()
    }

    pub fn get(&self, hash: u64) -> Option<CachedResponse> {
        let result = self.inner.get(&hash).map(|v| v.clone());
        if result.is_some() {
            let mut order = self.order.lock().unwrap();
            if let Some(pos) = order.iter().position(|k| *k == hash) {
                order.remove(pos);
                order.push_back(hash);
            }
        }
        result
    }

    pub fn insert(&self, hash: u64, resp: CachedResponse) {
        let mut order = self.order.lock().unwrap();
        if self.inner.len() >= self.max_entries {
            if let Some(k) = order.pop_front() {
                self.inner.remove(&k);
            }
        }
        order.push_back(hash);
        drop(order);
        self.inner.insert(hash, resp);
        info!("request cache: stored entry (total: {})", self.inner.len());
    }

    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

impl Default for RequestCache {
    fn default() -> Self {
        Self::new(128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, ChatRequest, StreamOptions, TokenDetails};

    fn dummy_req() -> ChatRequest {
        ChatRequest {
            model: "test-model".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: Some(serde_json::json!("hello")),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: false,
            reasoning_effort: None,
            thinking: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        }
    }

    fn dummy_resp(text: &str) -> CachedResponse {
        CachedResponse {
            text: text.into(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        }
    }

    // ── hash_request ──────────────────────────────────────────────────────────

    #[test]
    fn test_hash_request_consistency() {
        let req = dummy_req();
        let h1 = RequestCache::hash_request(&req);
        let h2 = RequestCache::hash_request(&req);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_request_different_models() {
        let mut a = dummy_req();
        a.model = "model-a".into();
        let mut b = dummy_req();
        b.model = "model-b".into();
        assert_ne!(
            RequestCache::hash_request(&a),
            RequestCache::hash_request(&b),
        );
    }

    #[test]
    fn test_hash_request_different_messages() {
        let empty = dummy_req();
        let mut with_msg = dummy_req();
        with_msg.messages = vec![ChatMessage {
            role: "user".into(),
            content: Some(serde_json::json!("different")),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        assert_ne!(
            RequestCache::hash_request(&empty),
            RequestCache::hash_request(&with_msg),
        );
    }

    #[test]
    fn test_hash_request_stream_options_included_when_some() {
        let mut without = dummy_req();
        without.stream_options = None;
        let mut with = dummy_req();
        with.stream_options = Some(StreamOptions {
            include_usage: true,
        });
        // stream_options: Some(_) serializes into the JSON and changes the hash
        assert_ne!(
            RequestCache::hash_request(&without),
            RequestCache::hash_request(&with),
        );
    }

    // ── usage_to_cached ───────────────────────────────────────────────────────

    #[test]
    fn test_usage_to_cached_none() {
        assert!(usage_to_cached(None).is_none());
    }

    #[test]
    fn test_usage_to_cached_full() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            completion_tokens_details: Some(TokenDetails {
                reasoning_tokens: Some(20),
            }),
            prompt_cache_hit_tokens: Some(30),
            prompt_cache_miss_tokens: Some(10),
            prompt_tokens_details: None,
        };
        let cached = usage_to_cached(Some(&usage)).unwrap();
        assert_eq!(cached.prompt_tokens, 100);
        assert_eq!(cached.completion_tokens, 50);
        assert_eq!(cached.total_tokens, 150);
        assert_eq!(cached.reasoning_tokens, Some(20));
        assert_eq!(cached.cache_hit_tokens, Some(30));
        assert_eq!(cached.cache_miss_tokens, Some(10));
    }

    #[test]
    fn test_usage_to_cached_partial() {
        let usage = ChatUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            completion_tokens_details: None,
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        };
        let cached = usage_to_cached(Some(&usage)).unwrap();
        assert_eq!(cached.reasoning_tokens, None);
        assert_eq!(cached.cache_hit_tokens, None);
        assert_eq!(cached.cache_miss_tokens, None);
    }

    // ── RequestCache::len ─────────────────────────────────────────────────────

    #[test]
    fn test_cache_len_starts_zero() {
        let cache = RequestCache::new(10);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_len_after_inserts() {
        let cache = RequestCache::new(10);
        for i in 0..5 {
            cache.insert(i, dummy_resp(&format!("r{i}")));
        }
        assert_eq!(cache.len(), 5);
    }

    #[test]
    fn test_cache_len_with_eviction() {
        let cache = RequestCache::new(3);
        for i in 0..6 {
            cache.insert(i as u64, dummy_resp(&format!("r{i}")));
        }
        assert_eq!(cache.len(), 3);
    }

    // ── CachedResponse serde round-trip ───────────────────────────────────────

    #[test]
    fn test_cached_response_serde_roundtrip() {
        let resp = CachedResponse {
            text: "Hello!".into(),
            reasoning: "Let me think...".into(),
            tool_calls: vec![CachedToolCall {
                id: "call_abc".into(),
                name: "get_weather".into(),
                arguments: r#"{"city":"NYC"}"#.into(),
            }],
            usage: Some(CachedUsage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
                reasoning_tokens: Some(5),
                cache_hit_tokens: Some(2),
                cache_miss_tokens: Some(8),
            }),
            created_at: 1234567890,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: CachedResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.text, deserialized.text);
        assert_eq!(resp.reasoning, deserialized.reasoning);
        assert_eq!(resp.tool_calls.len(), deserialized.tool_calls.len());
        assert_eq!(resp.tool_calls[0].id, deserialized.tool_calls[0].id);
        assert_eq!(resp.tool_calls[0].name, deserialized.tool_calls[0].name);
        assert_eq!(
            resp.tool_calls[0].arguments,
            deserialized.tool_calls[0].arguments
        );
        let u = resp.usage.unwrap();
        let du = deserialized.usage.unwrap();
        assert_eq!(u.prompt_tokens, du.prompt_tokens);
        assert_eq!(u.completion_tokens, du.completion_tokens);
        assert_eq!(u.total_tokens, du.total_tokens);
        assert_eq!(u.reasoning_tokens, du.reasoning_tokens);
        assert_eq!(u.cache_hit_tokens, du.cache_hit_tokens);
        assert_eq!(u.cache_miss_tokens, du.cache_miss_tokens);
        assert_eq!(resp.created_at, deserialized.created_at);
    }

    #[test]
    fn test_cached_response_serde_roundtrip_no_usage() {
        let resp = CachedResponse {
            text: "no usage".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deserialized: CachedResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.text, deserialized.text);
        assert!(deserialized.usage.is_none());
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn test_insert_duplicate_hash_overwrites() {
        let cache = RequestCache::new(10);
        let hash = 42u64;
        cache.insert(
            hash,
            CachedResponse {
                text: "first".into(),
                reasoning: String::new(),
                tool_calls: vec![],
                usage: None,
                created_at: 0,
            },
        );
        cache.insert(
            hash,
            CachedResponse {
                text: "second".into(),
                reasoning: "updated".into(),
                tool_calls: vec![],
                usage: None,
                created_at: 1,
            },
        );
        let got = cache.get(hash).unwrap();
        assert_eq!(got.text, "second");
        assert_eq!(got.reasoning, "updated");
        assert_eq!(got.created_at, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_get_nonexistent_hash() {
        let cache = RequestCache::new(10);
        assert!(cache.get(999).is_none());
    }

    #[test]
    fn test_cache_insert_and_get() {
        let cache = RequestCache::new(10);
        let hash = 42u64;
        let resp = CachedResponse {
            text: "hello".into(),
            reasoning: String::new(),
            tool_calls: vec![],
            usage: None,
            created_at: 0,
        };
        cache.insert(hash, resp.clone());
        let got = cache.get(hash).unwrap();
        assert_eq!(got.text, "hello");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_eviction() {
        let cache = RequestCache::new(2);
        for i in 0..4 {
            cache.insert(
                i,
                CachedResponse {
                    text: format!("r{i}"),
                    reasoning: String::new(),
                    tool_calls: vec![],
                    usage: None,
                    created_at: 0,
                },
            );
        }
        assert!(cache.len() <= 2);
    }
}
