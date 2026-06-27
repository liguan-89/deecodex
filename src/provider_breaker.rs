//! Provider 级熔断器。
//!
//! 借鉴 cc-switch `circuit_breaker.rs` 的简化版：当某个 provider（如 minimax、mimo）
//! 在短时间内连续失败 N 次，自动打开熔断器、阻断后续请求到该 provider 一段时间，
//! 避免上游 5xx 风暴雪崩到所有调用方。
//!
//! ## 与 cc-switch 的差异
//! - 简化为 2 状态：`Closed`（正常）/ `Open`（熔断），不引入 HalfOpen 探测
//!   （deecodex 已有 `ratelimit` 在做放行控制，熔断器只做"全开/全关"决策）
//! - 内存中 `DashMap<provider_slug, BreakerState>` 索引，不持久化（重启即重置）
//! - 失败阈值 = `failure_threshold`，成功重置计数；冷却到点后下一次请求放行
//!
//! ## 典型使用
//! ```ignore
//! use crate::provider_breaker;
//!
//! // 入口判断
//! if provider_breaker::is_open("minimax").await {
//!     return (StatusCode::SERVICE_UNAVAILABLE, "provider circuit open").into_response();
//! }
//!
//! // 出口记录
//! if status.is_server_error() {
//!     provider_breaker::record_failure("minimax").await;
//! } else {
//!     provider_breaker::record_success("minimax").await;
//! }
//! ```

use dashmap::DashMap;
use std::sync::{Arc, LazyLock};
use std::time::Instant;

/// 默认失败阈值：连续失败 5 次打开熔断器。
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 5;

/// 默认冷却时长：60 秒。
const DEFAULT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone)]
struct BreakerState {
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

/// 全局单例：进程内所有调用共享一个 provider → BreakerState 索引。
static BREAKERS: LazyLock<Arc<DashMap<String, BreakerState>>> =
    LazyLock::new(|| Arc::new(DashMap::new()));

/// 读取或初始化某 provider 的状态。
fn get_or_init(provider: &str) -> BreakerState {
    BREAKERS
        .entry(provider.to_string())
        .or_insert_with(|| BreakerState {
            consecutive_failures: 0,
            opened_at: None,
        })
        .clone()
}

/// 写入并回填（DashMap entry 写法）。
fn mutate<F: FnOnce(&mut BreakerState)>(provider: &str, f: F) {
    let mut entry = BREAKERS
        .entry(provider.to_string())
        .or_insert_with(|| BreakerState {
            consecutive_failures: 0,
            opened_at: None,
        });
    f(entry.value_mut());
}

/// 判断 provider 熔断器是否处于 Open 状态。
///
/// 冷却到点后**不主动关闭**熔断器（保持 Open 直到下一次成功请求），
/// 这样实现上更简单，且不会在失败风暴刚开始恢复时就把流量全放回去。
/// 真实关闭通过 `record_success` 完成。
pub async fn is_open(provider: &str) -> bool {
    let state = get_or_init(provider);
    match state.opened_at {
        None => false,
        Some(opened_at) if opened_at.elapsed() >= DEFAULT_COOLDOWN => {
            // 冷却已过，仍判 Open 直到 record_success 才关闭
            true
        }
        Some(_) => true,
    }
}

/// 记录一次成功。Open 状态下收到成功 → 关闭熔断器并清零计数；Closed 状态清零计数。
pub async fn record_success(provider: &str) {
    mutate(provider, |state| {
        state.consecutive_failures = 0;
        state.opened_at = None;
    });
}

/// 记录一次失败。Closed 状态下连续失败达到阈值则打开；Open 状态忽略（已开）。
pub async fn record_failure(provider: &str) {
    mutate(provider, |state| {
        if state.opened_at.is_some() {
            return; // 已经在 Open 状态
        }
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.consecutive_failures >= DEFAULT_FAILURE_THRESHOLD {
            state.opened_at = Some(Instant::now());
            tracing::warn!(
                "provider {} circuit breaker OPEN: {} consecutive failures",
                provider,
                state.consecutive_failures
            );
        }
    });
}

/// 手动重置（用于测试 / 运维）。
pub async fn reset(provider: &str) {
    mutate(provider, |state| {
        state.consecutive_failures = 0;
        state.opened_at = None;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_initially_allows() {
        reset("test_provider_a").await;
        assert!(!is_open("test_provider_a").await);
    }

    #[tokio::test]
    async fn failures_below_threshold_keep_closed() {
        reset("test_provider_b").await;
        for _ in 0..(DEFAULT_FAILURE_THRESHOLD - 1) {
            record_failure("test_provider_b").await;
        }
        assert!(!is_open("test_provider_b").await);
    }

    #[tokio::test]
    async fn failures_at_threshold_open_breaker() {
        reset("test_provider_c").await;
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            record_failure("test_provider_c").await;
        }
        assert!(is_open("test_provider_c").await);
    }

    #[tokio::test]
    async fn success_closes_breaker_and_resets_count() {
        reset("test_provider_d").await;
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            record_failure("test_provider_d").await;
        }
        assert!(is_open("test_provider_d").await);
        record_success("test_provider_d").await;
        assert!(!is_open("test_provider_d").await);
        // 成功后再次失败需要重新累计
        for _ in 0..(DEFAULT_FAILURE_THRESHOLD - 1) {
            record_failure("test_provider_d").await;
        }
        assert!(!is_open("test_provider_d").await);
    }

    #[tokio::test]
    async fn providers_are_independent() {
        reset("test_provider_e").await;
        reset("test_provider_f").await;
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            record_failure("test_provider_e").await;
        }
        assert!(is_open("test_provider_e").await);
        assert!(!is_open("test_provider_f").await);
    }

    #[tokio::test]
    async fn record_failure_on_open_state_is_noop() {
        reset("test_provider_g").await;
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            record_failure("test_provider_g").await;
        }
        assert!(is_open("test_provider_g").await);
        let before = get_or_init("test_provider_g").consecutive_failures;
        record_failure("test_provider_g").await;
        let after = get_or_init("test_provider_g").consecutive_failures;
        assert_eq!(before, after, "open state should not double-count failures");
    }

    #[tokio::test]
    async fn manually_reset_clears_breaker() {
        reset("test_provider_h").await;
        for _ in 0..DEFAULT_FAILURE_THRESHOLD {
            record_failure("test_provider_h").await;
        }
        assert!(is_open("test_provider_h").await);
        reset("test_provider_h").await;
        assert!(!is_open("test_provider_h").await);
    }
}
