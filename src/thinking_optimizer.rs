//! 思考模式三路径自动优化。
//!
//! 借鉴 cc-switch `thinking_optimizer.rs`：根据模型名自动选择 thinking 形态：
//! - **skip**：`haiku` 系列跳过（保持原样，不启用思考）
//! - **adaptive**：`opus-4-6/7/8` / `sonnet-4-6` 走 `thinking.type=adaptive` +
//!   `output_config.effort=max` + 追加 `context-1m-2025-08-07` beta
//! - **legacy**：其他模型走 `thinking.type=enabled` + `budget_tokens = max_tokens - 1`
//!   + 追加 `interleaved-thinking-2025-05-14` beta

use serde_json::{json, Value};

const ADAPTIVE_NEEDLES: &[&str] = &["opus-4-8", "opus-4-7", "opus-4-6", "sonnet-4-6"];
const BETA_ADAPTIVE: &str = "context-1m-2025-08-07";
const BETA_LEGACY: &str = "interleaved-thinking-2025-05-14";
const DEFAULT_MAX_TOKENS: u64 = 16384;

/// 在请求体上执行三路径优化。返回应用的路径标签。
///
/// - `None`：未应用（缺 model / 路径无法判定）
/// - `Some("skip")`：haiku 跳过
/// - `Some("adaptive")`：adaptive 路径
/// - `Some("legacy")`：legacy 路径
pub fn optimize_thinking(body: &mut Value) -> Option<&'static str> {
    let model = body.get("model").and_then(Value::as_str)?;
    let normalized = model.to_ascii_lowercase().replace('.', "-");

    // 路径 A：skip
    if normalized.contains("haiku") {
        return Some("skip");
    }

    // 路径 B：adaptive
    if ADAPTIVE_NEEDLES
        .iter()
        .any(|needle| normalized.contains(needle))
    {
        body["thinking"] = json!({"type": "adaptive"});
        body["output_config"] = json!({"effort": "max"});
        append_beta(body, BETA_ADAPTIVE);
        return Some("adaptive");
    }

    // 路径 C：legacy
    let max_tokens = body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let budget_target = max_tokens.saturating_sub(1);

    let thinking_type = body
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(Value::as_str);

    match thinking_type {
        None | Some("disabled") => {
            body["thinking"] = json!({"type": "enabled", "budget_tokens": budget_target});
            append_beta(body, BETA_LEGACY);
        }
        Some("enabled") => {
            let current = body
                .get("thinking")
                .and_then(|t| t.get("budget_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if current < budget_target {
                if let Some(t) = body.get_mut("thinking").and_then(Value::as_object_mut) {
                    t.insert("budget_tokens".to_string(), json!(budget_target));
                }
            }
            append_beta(body, BETA_LEGACY);
        }
        _ => {
            // adaptive 等其他 type：不动 thinking，仅追加 beta
            append_beta(body, BETA_LEGACY);
        }
    }
    Some("legacy")
}

/// 追加 beta 标签到 body.anthropic_beta，重复则跳过。
pub fn append_beta(body: &mut Value, beta: &str) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    match obj.get("anthropic_beta") {
        Some(Value::Array(arr)) => {
            let exists = arr.iter().any(|v| v.as_str() == Some(beta));
            if !exists {
                let mut new_arr = arr.clone();
                new_arr.push(json!(beta));
                obj.insert("anthropic_beta".to_string(), Value::Array(new_arr));
            }
        }
        Some(Value::Null) | None => {
            obj.insert("anthropic_beta".to_string(), json!([beta]));
        }
        Some(_) => {
            // 字符串/数字/对象等异常类型，覆盖为单元素数组
            obj.insert("anthropic_beta".to_string(), json!([beta]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adaptive_opus_4_8_normalizes_dots() {
        let mut body = json!({
            "model": "claude-opus-4.8",
            "thinking": {"type": "enabled", "budget_tokens": 1024},
            "max_tokens": 8000
        });
        let result = optimize_thinking(&mut body);
        assert_eq!(result, Some("adaptive"));
        assert_eq!(body["thinking"]["type"], "adaptive");
        // 切到 adaptive 应清空 budget_tokens（覆盖）
        assert!(body["thinking"].get("budget_tokens").is_none());
        assert_eq!(body["output_config"]["effort"], "max");
        assert_eq!(body["anthropic_beta"], json!(["context-1m-2025-08-07"]));
    }

    #[test]
    fn adaptive_opus_4_6_with_version_suffix() {
        let mut body = json!({
            "model": "claude-opus-4-6-20250514-v1:0",
        });
        let result = optimize_thinking(&mut body);
        assert_eq!(result, Some("adaptive"));
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn adaptive_sonnet_4_6() {
        let mut body = json!({"model": "claude-sonnet-4-6-20250514-v1:0"});
        let result = optimize_thinking(&mut body);
        assert_eq!(result, Some("adaptive"));
    }

    #[test]
    fn legacy_sonnet_4_5_injects_when_null() {
        let mut body = json!({"model": "claude-sonnet-4-5-20250514-v1:0"});
        let result = optimize_thinking(&mut body);
        assert_eq!(result, Some("legacy"));
        assert_eq!(body["thinking"]["type"], "enabled");
        // 缺 max_tokens → 用默认 16384 → budget=16383
        assert_eq!(body["thinking"]["budget_tokens"], 16383);
        assert_eq!(
            body["anthropic_beta"],
            json!(["interleaved-thinking-2025-05-14"])
        );
    }

    #[test]
    fn legacy_upgrades_underbudget() {
        let mut body = json!({
            "model": "claude-sonnet-4-5-20250514-v1:0",
            "thinking": {"type": "enabled", "budget_tokens": 1024},
            "max_tokens": 8000
        });
        let result = optimize_thinking(&mut body);
        assert_eq!(result, Some("legacy"));
        // budget=1024 < 7999 → 升到 7999
        assert_eq!(body["thinking"]["budget_tokens"], 7999);
    }

    #[test]
    fn legacy_keeps_higher_budget() {
        let mut body = json!({
            "model": "claude-sonnet-4-5-20250514-v1:0",
            "thinking": {"type": "enabled", "budget_tokens": 20000},
            "max_tokens": 32000
        });
        let _ = optimize_thinking(&mut body);
        // budget=20000 >= 31999？no，max-1=31999 > 20000 → 应升到 31999
        assert_eq!(body["thinking"]["budget_tokens"], 31999);
    }

    #[test]
    fn skip_haiku_preserves_body() {
        let mut body = json!({
            "model": "claude-haiku-4-5-20250514-v1:0",
            "thinking": {"type": "enabled", "budget_tokens": 1024}
        });
        let original = body.clone();
        let result = optimize_thinking(&mut body);
        assert_eq!(result, Some("skip"));
        assert_eq!(body, original, "haiku body should be unchanged");
    }

    #[test]
    fn adaptive_dedup_beta() {
        let mut body = json!({
            "model": "claude-opus-4-6-20250514-v1:0",
            "anthropic_beta": ["context-1m-2025-08-07", "other-beta"]
        });
        let _ = optimize_thinking(&mut body);
        // 应当不重复 push
        let arr = body["anthropic_beta"].as_array().unwrap();
        assert_eq!(
            arr.iter()
                .filter(|v| v == &&json!("context-1m-2025-08-07"))
                .count(),
            1
        );
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn legacy_disabled_thinking_injected() {
        let mut body = json!({
            "model": "claude-sonnet-4-5-20250514-v1:0",
            "thinking": {"type": "disabled"}
        });
        let _ = optimize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert!(body["thinking"]["budget_tokens"].as_u64().unwrap() > 0);
    }

    #[test]
    fn legacy_default_max_tokens() {
        let mut body = json!({"model": "claude-sonnet-4-5-20250514-v1:0"});
        let _ = optimize_thinking(&mut body);
        // 缺 max_tokens → 默认 16384 → budget=16383
        assert_eq!(body["thinking"]["budget_tokens"], 16383);
    }

    #[test]
    fn append_beta_null_field() {
        let mut body = json!({
            "model": "claude-opus-4-6-20250514-v1:0",
            "anthropic_beta": null
        });
        let _ = optimize_thinking(&mut body);
        // null 应当被替换为数组
        assert_eq!(body["anthropic_beta"], json!(["context-1m-2025-08-07"]));
    }

    #[test]
    fn append_beta_missing_field() {
        let mut body = json!({"model": "claude-opus-4-6-20250514-v1:0"});
        let _ = optimize_thinking(&mut body);
        assert_eq!(body["anthropic_beta"], json!(["context-1m-2025-08-07"]));
    }

    #[test]
    fn optimize_returns_none_when_no_model() {
        let mut body = json!({});
        let result = optimize_thinking(&mut body);
        assert_eq!(result, None);
    }
}
