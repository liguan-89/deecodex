//! Thinking budget 整流器。
//!
//! 借鉴 cc-switch `thinking_budget_rectifier.rs`：上游校验 `thinking.budget_tokens >= 1024`
//! 且要求 `max_tokens > budget_tokens` 时，本地客户端可能送出 100 或 8192 等不兼容值。
//! 检测到 3 条件错误后自动改写：
//! - `thinking.type` = `"enabled"`
//! - `thinking.budget_tokens` = `32000`
//! - `max_tokens` = `64000`（条件性：原值缺失或 < 32001）

use serde_json::{json, Value};

pub const MAX_THINKING_BUDGET: u64 = 32000;
pub const MAX_TOKENS_VALUE: u64 = 64000;
const MIN_MAX_TOKENS_FOR_BUDGET: u64 = MAX_THINKING_BUDGET + 1;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BudgetRectifySnapshot {
    pub max_tokens: Option<u64>,
    pub thinking_type: Option<String>,
    pub thinking_budget_tokens: Option<u64>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BudgetRectifyResult {
    pub applied: bool,
    pub before: BudgetRectifySnapshot,
    pub after: BudgetRectifySnapshot,
}

/// 判断上游错误消息是否触发 thinking budget 整流。
///
/// 三条件必须同时满足：
/// 1. 消息含 `budget_tokens` 或 `budget tokens`
/// 2. 消息含 `thinking`
/// 3. 消息含 1024 约束（`>= 1024` / `greater than or equal to 1024` / `1024` + `input should be`）
pub fn should_rectify_thinking_budget(error_message: Option<&str>) -> bool {
    let Some(message) = error_message else {
        return false;
    };
    let lower = message.to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }

    let has_budget_tokens = lower.contains("budget_tokens") || lower.contains("budget tokens");
    let has_thinking = lower.contains("thinking");
    let has_1024_constraint = lower.contains("greater than or equal to 1024")
        || lower.contains(">= 1024")
        || (lower.contains("1024") && lower.contains("input should be"));

    has_budget_tokens && has_thinking && has_1024_constraint
}

fn snapshot(body: &Value) -> BudgetRectifySnapshot {
    let max_tokens = body.get("max_tokens").and_then(Value::as_u64);
    let thinking_type = body
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let thinking_budget_tokens = body
        .get("thinking")
        .and_then(|t| t.get("budget_tokens"))
        .and_then(Value::as_u64);
    BudgetRectifySnapshot {
        max_tokens,
        thinking_type,
        thinking_budget_tokens,
    }
}

/// 整流 thinking budget。
///
/// - 若 `thinking.type == "adaptive"` → 跳过（不动 body）
/// - 否则强制覆盖 `thinking.type=enabled` + `thinking.budget_tokens=32000`
/// - 缺失 thinking 对象时自动建
/// - `max_tokens` 缺失或 < 32001 时设为 64000
pub fn rectify_thinking_budget(body: &mut Value) -> BudgetRectifyResult {
    let before = snapshot(body);
    if before.thinking_type.as_deref() == Some("adaptive") {
        return BudgetRectifyResult {
            applied: false,
            before: before.clone(),
            after: before,
        };
    }

    // 补建 thinking 对象
    if !body.get("thinking").map(Value::is_object).unwrap_or(false) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("thinking".to_string(), json!({}));
        }
    }

    // 强制 thinking.type=enabled + budget_tokens=32000
    if let Some(thinking) = body.get_mut("thinking").and_then(Value::as_object_mut) {
        thinking.insert("type".to_string(), json!("enabled"));
        thinking.insert("budget_tokens".to_string(), json!(MAX_THINKING_BUDGET));
    }

    // 条件性 max_tokens
    if before.max_tokens.is_none() || before.max_tokens < Some(MIN_MAX_TOKENS_FOR_BUDGET) {
        if let Some(obj) = body.as_object_mut() {
            obj.insert("max_tokens".to_string(), json!(MAX_TOKENS_VALUE));
        }
    }

    let after = snapshot(body);
    BudgetRectifyResult {
        applied: before != after,
        before,
        after,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_budget_tokens_thinking_error() {
        let msg = "thinking.budget_tokens must be greater than or equal to 1024, and max_tokens must be larger than budget_tokens";
        assert!(should_rectify_thinking_budget(Some(msg)));
    }

    #[test]
    fn no_detect_budget_tokens_without_thinking() {
        let msg = "max_tokens must be greater than or equal to 1024";
        assert!(!should_rectify_thinking_budget(Some(msg)));
    }

    #[test]
    fn no_detect_thinking_without_1024() {
        let msg = "thinking.budget_tokens is invalid";
        assert!(!should_rectify_thinking_budget(Some(msg)));
    }

    #[test]
    fn detect_gte_1024_pattern() {
        let msg = "thinking budget_tokens >= 1024 required";
        assert!(should_rectify_thinking_budget(Some(msg)));
    }

    #[test]
    fn detect_input_should_be_pattern() {
        let msg = "thinking.budget_tokens: input should be 1024 at minimum";
        assert!(should_rectify_thinking_budget(Some(msg)));
    }

    #[test]
    fn no_trigger_for_unrelated_error() {
        assert!(!should_rectify_thinking_budget(Some("connection timeout")));
        assert!(!should_rectify_thinking_budget(None));
    }

    #[test]
    fn rectify_basic() {
        let mut body = json!({
            "model": "claude",
            "thinking": {"type": "enabled", "budget_tokens": 1024},
            "max_tokens": 2048
        });
        let r = rectify_thinking_budget(&mut body);
        assert!(r.applied);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 32000);
        // max_tokens=2048 < 32001 → 应被拉升到 64000
        assert_eq!(body["max_tokens"], 64000);
    }

    #[test]
    fn rectify_skips_adaptive() {
        let mut body = json!({
            "thinking": {"type": "adaptive"}
        });
        let original = body.clone();
        let r = rectify_thinking_budget(&mut body);
        assert!(!r.applied);
        assert_eq!(body, original);
    }

    #[test]
    fn rectify_preserves_large_max_tokens() {
        let mut body = json!({
            "thinking": {"type": "enabled", "budget_tokens": 1024},
            "max_tokens": 100000
        });
        let _r = rectify_thinking_budget(&mut body);
        // max_tokens=100000 >= 32001 → 不动
        assert_eq!(body["max_tokens"], 100000);
    }

    #[test]
    fn rectify_creates_thinking_object_when_missing() {
        let mut body = json!({
            "model": "claude",
            "max_tokens": 4096
        });
        let r = rectify_thinking_budget(&mut body);
        assert!(r.applied);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 32000);
        // max_tokens=4096 < 32001 → 拉 64000
        assert_eq!(body["max_tokens"], 64000);
    }

    #[test]
    fn rectify_no_max_tokens() {
        let mut body = json!({
            "thinking": {"type": "enabled", "budget_tokens": 100}
        });
        let r = rectify_thinking_budget(&mut body);
        assert!(r.applied);
        assert_eq!(body["max_tokens"], 64000);
    }

    #[test]
    fn rectify_normalizes_non_enabled_type() {
        // type=disabled 强制改 enabled
        let mut body = json!({
            "thinking": {"type": "disabled", "budget_tokens": 0}
        });
        let r = rectify_thinking_budget(&mut body);
        assert!(r.applied);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 32000);
    }

    #[test]
    fn rectify_no_change_when_already_valid() {
        let mut body = json!({
            "thinking": {"type": "enabled", "budget_tokens": 32000},
            "max_tokens": 64000
        });
        let r = rectify_thinking_budget(&mut body);
        assert!(!r.applied);
    }
}
