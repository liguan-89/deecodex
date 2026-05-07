use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

use tracing::warn;

use crate::types::ChatUsage;

#[derive(Debug, Clone)]
pub struct TokenSnapshot {
    timestamp: Instant,
    pub prompt_tokens: u32,
    #[allow(dead_code)]
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub model: String,
}

pub struct TokenTracker {
    recent: Mutex<VecDeque<TokenSnapshot>>,
    #[allow(dead_code)]
    window_size: usize,
    prompt_max: u32,
    prompt_spike_ratio: f64,
    burn_window_secs: u64,
    burn_rate_warn_per_min: u32,
}

impl TokenTracker {
    pub fn new(
        window_size: usize,
        prompt_max: u32,
        prompt_spike_ratio: f64,
        burn_window_secs: u64,
        burn_rate_warn_per_min: u32,
    ) -> Self {
        Self {
            recent: Mutex::new(VecDeque::with_capacity(window_size)),
            window_size,
            prompt_max,
            prompt_spike_ratio,
            burn_window_secs,
            burn_rate_warn_per_min,
        }
    }

    pub fn record(
        &self,
        usage: &ChatUsage,
        model: &str,
        response_id: &str,
    ) -> Vec<String> {
        let mut anomalies = Vec::new();

        let now = Instant::now();
        let snapshot = TokenSnapshot {
            timestamp: now,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            model: model.to_string(),
        };

        // 1. Absolute prompt token explosion
        if usage.prompt_tokens > self.prompt_max {
            let msg = format!(
                "[token_anomaly] prompt_explosion: prompt_tokens={} exceeds max={} model={} response_id={}",
                usage.prompt_tokens, self.prompt_max, model, response_id
            );
            warn!("{}", msg);
            anomalies.push("prompt_explosion".into());
        }

        // 2. Zero completion tokens (model produced nothing)
        if usage.completion_tokens == 0 && usage.total_tokens > 0 {
            let msg = format!(
                "[token_anomaly] zero_completion: prompt={} completion=0 model={} response_id={}",
                usage.prompt_tokens, model, response_id
            );
            warn!("{}", msg);
            anomalies.push("zero_completion".into());
        }

        // 3. Prompt spike vs recent average (if enough history)
        {
            let mut recent = self.recent.lock().unwrap();
            let same_model: Vec<&TokenSnapshot> = recent
                .iter()
                .filter(|s| s.model == model)
                .collect();

            if !same_model.is_empty() {
                let avg_prompt: f64 = same_model.iter().map(|s| s.prompt_tokens as f64).sum::<f64>()
                    / same_model.len() as f64;
                if avg_prompt > 0.0 {
                    let ratio = usage.prompt_tokens as f64 / avg_prompt;
                    if ratio > self.prompt_spike_ratio && usage.prompt_tokens > 10_000 {
                        let msg = format!(
                            "[token_anomaly] prompt_spike: prompt_tokens={} vs_avg={:.0} ratio={:.1}x model={} response_id={}",
                            usage.prompt_tokens, avg_prompt, ratio, model, response_id
                        );
                        warn!("{}", msg);
                        if !anomalies.contains(&"prompt_explosion".to_string()) {
                            anomalies.push("prompt_spike".into());
                        }
                    }
                }
            }

            // 4. Token burn rate over window
            let cutoff = now - std::time::Duration::from_secs(self.burn_window_secs);
            while recent.front().is_some_and(|s| s.timestamp < cutoff) {
                recent.pop_front();
            }

            let burn_total: u32 = recent
                .iter()
                .map(|s| s.total_tokens)
                .sum::<u32>()
                + usage.total_tokens;

            let burn_per_min = if recent.is_empty() {
                usage.total_tokens
            } else {
                let elapsed_secs = (now.duration_since(recent.front().unwrap().timestamp)).as_secs().max(1);
                ((burn_total as f64 / elapsed_secs as f64) * 60.0) as u32
            };

            if burn_per_min > self.burn_rate_warn_per_min {
                let msg = format!(
                    "[token_anomaly] high_burn_rate: ~{}/min total={} window={}s model={} response_id={}",
                    burn_per_min, burn_total, self.burn_window_secs, model, response_id
                );
                warn!("{}", msg);
                anomalies.push("high_burn_rate".into());
            }

            recent.push_back(snapshot);
        }

        anomalies
    }
}

impl Default for TokenTracker {
    fn default() -> Self {
        Self::new(32, 200_000, 5.0, 120, 500_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatUsage;

    fn usage(prompt: u32, completion: u32) -> ChatUsage {
        ChatUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: prompt + completion,
            completion_tokens_details: None,
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        }
    }

    #[test]
    fn test_prompt_explosion_detected() {
        let tracker = TokenTracker::new(32, 100_000, 5.0, 120, 500_000);
        let anomalies = tracker.record(&usage(200_001, 1000), "test-model", "rid_1");
        assert!(anomalies.contains(&"prompt_explosion".to_string()));
    }

    #[test]
    fn test_normal_usage_no_anomaly() {
        let tracker = TokenTracker::new(32, 200_000, 5.0, 120, 500_000);
        let anomalies = tracker.record(&usage(10_000, 500), "test-model", "rid_1");
        assert!(anomalies.is_empty());
    }

    #[test]
    fn test_zero_completion_detected() {
        let tracker = TokenTracker::new(32, 200_000, 5.0, 120, 500_000);
        let anomalies = tracker.record(&usage(5000, 0), "test-model", "rid_2");
        assert!(anomalies.contains(&"zero_completion".to_string()));
    }

    #[test]
    fn test_zero_completion_with_zero_total_not_detected() {
        let tracker = TokenTracker::new(32, 200_000, 5.0, 120, 500_000);
        let anomalies = tracker.record(&usage(0, 0), "test-model", "rid_3");
        assert!(!anomalies.contains(&"zero_completion".to_string()));
    }

    #[test]
    fn test_prompt_spike_detected() {
        let tracker = TokenTracker::new(32, 200_000, 5.0, 120, 500_000);
        tracker.record(&usage(5000, 200), "test-model", "rid_1");
        tracker.record(&usage(6000, 300), "test-model", "rid_2");
        // 3rd request has 10x prompt tokens (55k vs 5.5k avg) → spike
        let anomalies = tracker.record(&usage(55_000, 500), "test-model", "rid_3");
        assert!(anomalies.contains(&"prompt_spike".to_string()));
    }
}
