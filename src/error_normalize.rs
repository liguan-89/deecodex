//! 上游错误体归一化。
//!
//! 借鉴 cc-switch `chat_error_to_response_error`：把 Chat Completions 上游五花八门
//! 的错误体形态统一成 OpenAI Responses API 风格的 `{"error": {message, type, code, param}}`，
//! 保证 Codex 桌面 / CLI 的错误提示链路只走一种格式。
//!
//! 兼容形态：
//! 1. 标准 OpenAI `{"error": {"message", "type", "code", "param"}}`
//! 2. minimax/mimo 非标 `{"base_resp": {"status_code", "status_msg"}}`
//! 3. 中转层常见 `{"detail": "..."}` 形态
//! 4. 顶层 `{"message": "..."}` 形态
//! 5. 裸 JSON 字符串 `"Upstream timeout"`
//! 6. 缺 body

use serde_json::{json, Value};

/// 把上游 Chat 错误体归一化为 OpenAI Responses API 错误形状。
///
/// 始终返回 `{"error": {message, type, code, param}}`；任何字段无法提取时回落到
/// `"Upstream error"` / `"upstream_error"` / `null` / `null`，不返回 `Value::Null`。
pub fn chat_error_to_response_error(body: Option<&Value>) -> Value {
    let Some(value) = body else {
        return empty_error();
    };

    if let Some(text) = value.as_str() {
        return text_error(text);
    }

    let source = value.get("error").unwrap_or(value);

    let message = source
        .get("message")
        .or_else(|| source.get("detail"))
        .or_else(|| source.get("status_msg"))
        .or_else(|| source.pointer("/base_resp/status_msg"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            // 极端情况：source 本身就是字符串（少见但 cc-switch 也兜底）
            source.as_str().map(ToString::to_string)
        })
        .unwrap_or_else(|| {
            // 实在没拿到任何文本字段，就把 source 序列化回 message 方便排查
            serde_json::to_string(source).unwrap_or_else(|_| "Upstream error".to_string())
        });

    let error_type = source
        .get("type")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| "upstream_error".to_string());

    let code = source
        .get("code")
        .cloned()
        .or_else(|| source.pointer("/base_resp/status_code").cloned())
        .unwrap_or(Value::Null);

    let param = source.get("param").cloned().unwrap_or(Value::Null);

    json!({
        "error": {
            "message": message,
            "type": error_type,
            "code": code,
            "param": param,
        }
    })
}

/// 兜底：上游返回空 body 时使用。
fn empty_error() -> Value {
    json!({
        "error": {
            "message": "Upstream returned an empty error response",
            "type": "upstream_error",
            "code": Value::Null,
            "param": Value::Null,
        }
    })
}

/// 兜底：上游返回裸字符串 / HTML 时使用。
fn text_error(text: &str) -> Value {
    json!({
        "error": {
            "message": text,
            "type": "upstream_error",
            "code": Value::Null,
            "param": Value::Null,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_standard_openai_shape() {
        let input = json!({
            "error": {
                "message": "Invalid API key",
                "type": "invalid_request_error",
                "code": "invalid_api_key",
                "param": "api_key"
            }
        });
        let out = chat_error_to_response_error(Some(&input));
        assert_eq!(out["error"]["message"], "Invalid API key");
        assert_eq!(out["error"]["type"], "invalid_request_error");
        assert_eq!(out["error"]["code"], "invalid_api_key");
        assert_eq!(out["error"]["param"], "api_key");
    }

    #[test]
    fn normalizes_minimax_base_resp() {
        // minimax 把错误塞在 base_resp 里，code 是数字而不是字符串
        let input = json!({
            "base_resp": {
                "status_code": 2013,
                "status_msg": "invalid params, chat content has invalid message role: system"
            }
        });
        let out = chat_error_to_response_error(Some(&input));
        assert_eq!(
            out["error"]["message"],
            "invalid params, chat content has invalid message role: system"
        );
        assert_eq!(out["error"]["code"], 2013);
        assert_eq!(out["error"]["type"], "upstream_error");
    }

    #[test]
    fn normalizes_top_level_detail_field() {
        // OpenAI 兼容层常见：只有 detail 字段
        let input = json!({"detail": "rate limit exceeded"});
        let out = chat_error_to_response_error(Some(&input));
        assert_eq!(out["error"]["message"], "rate limit exceeded");
        assert_eq!(out["error"]["type"], "upstream_error");
    }

    #[test]
    fn normalizes_top_level_message_field() {
        let input = json!({"message": "internal server error"});
        let out = chat_error_to_response_error(Some(&input));
        assert_eq!(out["error"]["message"], "internal server error");
    }

    #[test]
    fn normalizes_plain_string_body() {
        let input = json!("Upstream timeout");
        let out = chat_error_to_response_error(Some(&input));
        assert_eq!(out["error"]["message"], "Upstream timeout");
        assert_eq!(out["error"]["type"], "upstream_error");
        assert!(out["error"]["code"].is_null());
    }

    #[test]
    fn handles_missing_body() {
        let out = chat_error_to_response_error(None);
        assert_eq!(
            out["error"]["message"],
            "Upstream returned an empty error response"
        );
        assert_eq!(out["error"]["type"], "upstream_error");
    }

    #[test]
    fn falls_back_to_serializing_unrecognized_shape() {
        // 没有任何已知字段的对象，应当把整个 source 序列化回 message
        let input = json!({"weird_field": "x", "another": 42});
        let out = chat_error_to_response_error(Some(&input));
        let message = out["error"]["message"].as_str().unwrap();
        assert!(message.contains("weird_field"));
        assert!(message.contains("another"));
    }
}
