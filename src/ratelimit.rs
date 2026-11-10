use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_MAX_REQUESTS: u32 = 120;
const DEFAULT_WINDOW_SECS: u64 = 60;

pub struct RateLimiter {
    buckets: Arc<DashMap<String, Vec<Instant>>>,
    max_requests: u32,
    window: Duration,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_secs: u64) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            max_requests: if max_requests == 0 {
                DEFAULT_MAX_REQUESTS
            } else {
                max_requests
            },
            window: Duration::from_secs(if window_secs == 0 {
                DEFAULT_WINDOW_SECS
            } else {
                window_secs
            }),
        }
    }

    /// Check if a request from `key` is allowed.
    /// Returns `true` if within limit, `false` if rate limited.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut entry = self.buckets.entry(key.to_string()).or_default();
        entry.retain(|t| now.duration_since(*t) < self.window);
        if entry.len() >= self.max_requests as usize {
            false
        } else {
            entry.push(now);
            true
        }
    }

    pub fn max_requests(&self) -> u32 {
        self.max_requests
    }

    pub fn window_secs(&self) -> u64 {
        self.window.as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allows_within_limit() {
        let limiter = RateLimiter::new(5, 60);
        for _ in 0..5 {
            assert!(limiter.check("test"));
        }
    }

    #[test]
    fn test_blocks_over_limit() {
        let limiter = RateLimiter::new(3, 60);
        for _ in 0..3 {
            assert!(limiter.check("test"));
        }
        assert!(!limiter.check("test"));
    }

    #[test]
    fn test_different_keys_independent() {
        let limiter = RateLimiter::new(2, 60);
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
        assert!(!limiter.check("a"));
        assert!(limiter.check("b"));
    }

    #[test]
    fn test_default_values() {
        let limiter = RateLimiter::new(0, 0);
        assert_eq!(limiter.max_requests(), 120);
        assert_eq!(limiter.window_secs(), 60);
    }
}
