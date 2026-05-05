use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

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
        reasoning_tokens: u.completion_tokens_details.as_ref().and_then(|d| d.reasoning_tokens),
        cache_hit_tokens: u.prompt_cache_hit_tokens,
        cache_miss_tokens: u.prompt_cache_miss_tokens,
    })
}

/// Simple request cache keyed by serialized ChatRequest hash.
#[derive(Clone)]
pub struct RequestCache {
    inner: Arc<DashMap<u64, CachedResponse>>,
    max_entries: usize,
}

impl RequestCache {
    pub fn new(max_entries: usize) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
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
        self.inner.get(&hash).map(|v| v.clone())
    }

    pub fn insert(&self, hash: u64, resp: CachedResponse) {
        // Evict oldest if at capacity (simple: just remove a random entry)
        if self.inner.len() >= self.max_entries {
            if let Some(key) = self.inner.iter().next().map(|e| *e.key()) {
                self.inner.remove(&key);
            }
        }
        self.inner.insert(hash, resp);
        info!("request cache: stored entry (total: {})", self.inner.len());
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn stats(&self) -> (usize, usize) {
        (self.inner.len(), self.max_entries)
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
            cache.insert(i, CachedResponse {
                text: format!("r{i}"),
                reasoning: String::new(),
                tool_calls: vec![],
                usage: None,
                created_at: 0,
            });
        }
        assert!(cache.len() <= 2);
    }
}
