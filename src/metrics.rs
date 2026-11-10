use prometheus::Encoder;
use prometheus::{
    register_histogram_vec_with_registry, register_int_counter_vec_with_registry,
    register_int_gauge_with_registry, HistogramOpts, IntCounterVec, Opts, Registry, TextEncoder,
};
use std::sync::Mutex;

pub struct Metrics {
    registry: Registry,
    pub http_requests_total: IntCounterVec,
    pub rate_limit_hits_total: IntCounterVec,
    pub token_anomalies_total: IntCounterVec,
    encoder: Mutex<TextEncoder>,
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        let http_requests_total = register_int_counter_vec_with_registry!(
            Opts::new(
                "http_requests_total",
                "Total HTTP requests by method and status"
            ),
            &["method", "status"],
            registry
        )
        .unwrap();

        let _http_request_duration_seconds = register_histogram_vec_with_registry!(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "HTTP request latency by method and path"
            )
            .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0]),
            &["method", "path"],
            registry
        )
        .unwrap();

        let _upstream_requests_total = register_int_counter_vec_with_registry!(
            Opts::new("upstream_requests_total", "Upstream API calls by status"),
            &["status"],
            registry
        )
        .unwrap();

        let _upstream_request_duration_seconds = register_histogram_vec_with_registry!(
            HistogramOpts::new(
                "upstream_request_duration_seconds",
                "Upstream API call latency"
            )
            .buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0]),
            &[],
            registry
        )
        .unwrap();

        let _cache_hits_total = register_int_counter_vec_with_registry!(
            Opts::new("cache_hits_total", "Cache hit/miss by type"),
            &["type"],
            registry
        )
        .unwrap();

        let _stream_events_total = register_int_counter_vec_with_registry!(
            Opts::new("stream_events_total", "SSE events produced by type"),
            &["event_type"],
            registry
        )
        .unwrap();

        let rate_limit_hits_total = register_int_counter_vec_with_registry!(
            Opts::new("rate_limit_hits_total", "Rate limited requests by key"),
            &["key"],
            registry
        )
        .unwrap();

        let _active_connections = register_int_gauge_with_registry!(
            Opts::new("active_connections", "Currently active connections"),
            registry
        )
        .unwrap();

        let token_anomalies_total = register_int_counter_vec_with_registry!(
            Opts::new(
                "token_anomalies_total",
                "Token usage anomaly events by type"
            ),
            &["anomaly_type"],
            registry
        )
        .unwrap();

        let _token_usage_prompt_total = register_histogram_vec_with_registry!(
            HistogramOpts::new(
                "token_usage_prompt_total",
                "Prompt token usage distribution"
            )
            .buckets(vec![
                100.0,
                500.0,
                1000.0,
                5000.0,
                10_000.0,
                50_000.0,
                100_000.0,
                200_000.0,
                500_000.0,
                1_000_000.0,
            ]),
            &["model"],
            registry
        )
        .unwrap();

        let _token_usage_completion_total = register_histogram_vec_with_registry!(
            HistogramOpts::new(
                "token_usage_completion_total",
                "Completion token usage distribution"
            )
            .buckets(vec![
                10.0, 50.0, 100.0, 500.0, 1000.0, 5000.0, 10_000.0, 50_000.0, 100_000.0,
            ]),
            &["model"],
            registry
        )
        .unwrap();

        Self {
            registry,
            http_requests_total,
            rate_limit_hits_total,
            token_anomalies_total,
            encoder: Mutex::new(TextEncoder::new()),
        }
    }

    pub fn gather(&self) -> String {
        let metric_families = self.registry.gather();
        let mut buffer = Vec::new();
        let encoder = match self.encoder.lock() {
            Ok(guard) => guard,
            Err(_) => {
                let mut buf = Vec::new();
                TextEncoder::new()
                    .encode(&metric_families, &mut buf)
                    .unwrap();
                return String::from_utf8(buf).unwrap_or_default();
            }
        };
        encoder.encode(&metric_families, &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap_or_default()
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_new() {
        let m = Metrics::new();
        let output = m.gather();
        assert!(!output.is_empty(), "gather output should not be empty");
    }

    #[test]
    fn test_http_counter_increment() {
        let m = Metrics::new();
        m.http_requests_total
            .with_label_values(&["POST", "200"])
            .inc();
        let inc = m
            .http_requests_total
            .with_label_values(&["POST", "200"])
            .get();
        assert_eq!(inc, 1);
    }
}
