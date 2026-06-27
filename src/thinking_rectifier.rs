//! Thinking 签名整流器。
//!
//! 借鉴 cc-switch `thinking_rectifier.rs`：当第三方渠道把 Anthropic thinking 的
//! signature 透传失败时，Anthropic 官方会拒绝整轮请求。本模块：
//! 1. 识别 7 类触发场景的错误消息
//! 2. 自动从 messages 里删掉 `thinking` / `redacted_thinking` block + 移除其他 block 的 `signature` 字段
//! 3. 必要时移除顶层 `thinking`（仅当 `type=enabled` 且最后一条 assistant 消息含 tool_use）
//!
//! ## 与 cc-switch 的差异
//! - 简化为无 `RectifierConfig` 开关（按 Karpathy "简洁优先"，签名整流始终启用）
//! - normalize_thinking_type 移除（cc-switch 实现为空）

use serde_json::{json, Value};

/// 整流结果统计。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RectifyResult {
    pub removed_thinking_blocks: usize,
    pub removed_redacted_thinking_blocks: usize,
    pub removed_signature_fields: usize,
    pub removed_top_level_thinking: bool,
}

impl RectifyResult {
    pub fn is_changed(&self) -> bool {
        self.removed_thinking_blocks > 0
            || self.removed_redacted_thinking_blocks > 0
            || self.removed_signature_fields > 0
            || self.removed_top_level_thinking
    }
}

/// 判断上游错误消息是否触发 thinking 签名整流。
///
/// 覆盖 7 类场景：
/// 1. `Invalid signature in thinking block`
/// 1b. `Thought signature is not valid` / `invalid`
/// 2. `must start with a thinking block`
/// 3. `expected ... thinking/redacted_thinking ... found ... tool_use`
/// 4. `signature: field required`
/// 5. `signature: extra inputs are not permitted`
/// 6. `thinking/redacted_thinking ... cannot be modified`
/// 7. `非法请求` / `illegal request` / `invalid request`
pub fn should_rectify_thinking_signature(error_message: Option<&str>) -> bool {
    let Some(message) = error_message else {
        return false;
    };
    let lower = message.to_ascii_lowercase();
    if lower.is_empty() {
        return false;
    }

    // 场景 1
    if lower.contains("invalid")
        && lower.contains("signature")
        && lower.contains("thinking")
        && lower.contains("block")
    {
        return true;
    }
    // 场景 1b
    if lower.contains("thought signature")
        && (lower.contains("not valid") || lower.contains("invalid"))
    {
        return true;
    }
    // 场景 2
    if lower.contains("must start with a thinking block") {
        return true;
    }
    // 场景 3：expected ... (thinking|redacted_thinking) ... found ... tool_use
    if lower.contains("expected")
        && (lower.contains("thinking") || lower.contains("redacted_thinking"))
        && lower.contains("found")
        && lower.contains("tool_use")
    {
        return true;
    }
    // 场景 4
    if lower.contains("signature") && lower.contains("field required") {
        return true;
    }
    // 场景 5
    if lower.contains("signature") && lower.contains("extra inputs are not permitted") {
        return true;
    }
    // 场景 6
    if (lower.contains("thinking") || lower.contains("redacted_thinking"))
        && lower.contains("cannot be modified")
    {
        return true;
    }
    // 场景 7：中文/英文非法请求
    if message.contains("非法请求")
        || lower.contains("illegal request")
        || lower.contains("invalid request")
    {
        return true;
    }
    false
}

/// 整流 Anthropic 请求体。
///
/// - 遍历 `body.messages[*].content[*]`，删除 `type=thinking` / `type=redacted_thinking` 块，
///   从其他块移除 `signature` 字段
/// - 若 `body.thinking.type == "enabled"` 且最后一条 assistant 消息含 tool_use 块，
///   移除顶层 `body.thinking`
///
/// 不会改动 `type=adaptive` 顶层 thinking。
pub fn rectify_anthropic_request(body: &mut Value) -> RectifyResult {
    let mut result = RectifyResult::default();

    // 顶层 thinking 移除决策要在整流前用原始 body 判定（避免循环依赖）
    let top_level_remove = should_remove_top_level_thinking(body);

    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
                continue;
            };
            let mut kept: Vec<Value> = Vec::with_capacity(blocks.len());
            for block in blocks.drain(..) {
                let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
                match block_type {
                    "thinking" => {
                        result.removed_thinking_blocks += 1;
                        // drop whole block
                    }
                    "redacted_thinking" => {
                        result.removed_redacted_thinking_blocks += 1;
                        // drop whole block
                    }
                    _ => {
                        // 其他 block：移除 signature 字段（如有）
                        let mut block = block;
                        if let Some(obj) = block.as_object_mut() {
                            if obj.remove("signature").is_some() {
                                result.removed_signature_fields += 1;
                            }
                        }
                        kept.push(block);
                    }
                }
            }
            *blocks = kept;
        }
    }

    if top_level_remove {
        if let Some(obj) = body.as_object_mut() {
            if obj.remove("thinking").is_some() {
                result.removed_top_level_thinking = true;
            }
        }
    }

    result
}

fn should_remove_top_level_thinking(body: &Value) -> bool {
    // 条件 1：body.thinking.type == "enabled"（仅 enabled；adaptive 不删）
    let thinking_type = body
        .get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(Value::as_str);
    if thinking_type != Some("enabled") {
        return false;
    }
    // 条件 2：最后一条 assistant 消息且 content 非空
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return false;
    };
    let Some(last_assistant) = messages
        .iter()
        .rev()
        .find(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
    else {
        return false;
    };
    let Some(blocks) = last_assistant.get("content").and_then(Value::as_array) else {
        return false;
    };
    if blocks.is_empty() {
        return false;
    }
    // 条件 3：首块既不是 thinking 也不是 redacted_thinking
    let first_type = blocks[0].get("type").and_then(Value::as_str).unwrap_or("");
    if first_type == "thinking" || first_type == "redacted_thinking" {
        return false;
    }
    // 条件 4：content 中存在任意 tool_use 块
    blocks
        .iter()
        .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_invalid_signature() {
        let msg = "Invalid `signature` in `thinking` block";
        assert!(should_rectify_thinking_signature(Some(msg)));
    }

    #[test]
    fn detect_invalid_signature_no_backticks() {
        let msg = "INVALID SIGNATURE IN THINKING BLOCK";
        assert!(should_rectify_thinking_signature(Some(msg)));
    }

    #[test]
    fn detect_invalid_thought_signature_message() {
        assert!(should_rectify_thinking_signature(Some(
            "Thought signature is not valid"
        )));
    }

    #[test]
    fn detect_thinking_expected_tool_use() {
        let msg = "expected thinking or redacted_thinking, found tool_use";
        assert!(should_rectify_thinking_signature(Some(msg)));
    }

    #[test]
    fn no_detect_thinking_expected_when_no_tool_use() {
        let msg = "expected thinking, found text";
        assert!(!should_rectify_thinking_signature(Some(msg)));
    }

    #[test]
    fn detect_must_start_with_thinking() {
        assert!(should_rectify_thinking_signature(Some(
            "request must start with a thinking block"
        )));
    }

    #[test]
    fn detect_signature_field_required() {
        assert!(should_rectify_thinking_signature(Some(
            "thinking.signature: Field required"
        )));
    }

    #[test]
    fn detect_signature_extra_inputs() {
        assert!(should_rectify_thinking_signature(Some(
            "signature: Extra inputs are not permitted"
        )));
    }

    #[test]
    fn detect_thinking_cannot_be_modified() {
        assert!(should_rectify_thinking_signature(Some(
            "thinking cannot be modified"
        )));
    }

    #[test]
    fn detect_invalid_request_chinese() {
        assert!(should_rectify_thinking_signature(Some("非法请求")));
    }

    #[test]
    fn no_trigger_for_unrelated_error() {
        assert!(!should_rectify_thinking_signature(Some(
            "connection timeout"
        )));
        assert!(!should_rectify_thinking_signature(Some(
            "connection refused"
        )));
        assert!(!should_rectify_thinking_signature(None));
    }

    #[test]
    fn rectify_removes_thinking_blocks() {
        let mut body = json!({
            "model": "claude-3",
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "...", "signature": "sig1"},
                    {"type": "redacted_thinking", "data": "..."},
                    {"type": "text", "text": "hi", "signature": "sig2"}
                ]
            }]
        });
        let result = rectify_anthropic_request(&mut body);
        assert_eq!(result.removed_thinking_blocks, 1);
        assert_eq!(result.removed_redacted_thinking_blocks, 1);
        assert_eq!(result.removed_signature_fields, 1);
        let content = &body["messages"][0]["content"];
        assert_eq!(content.as_array().unwrap().len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert!(content[0].get("signature").is_none());
    }

    #[test]
    fn rectify_removes_top_level_thinking_when_enabled_and_tool_use_present() {
        let mut body = json!({
            "thinking": {"type": "enabled", "budget_tokens": 1000},
            "messages": [
                {"role": "user", "content": "do something"},
                {"role": "assistant", "content": [
                    {"type": "text", "text": "calling tool"},
                    {"type": "tool_use", "id": "t1", "name": "exec"}
                ]}
            ]
        });
        let result = rectify_anthropic_request(&mut body);
        assert!(result.removed_top_level_thinking);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn rectify_keeps_adaptive_top_level_thinking() {
        let mut body = json!({
            "thinking": {"type": "adaptive"},
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "text", "text": "x"},
                    {"type": "tool_use", "id": "t1", "name": "exec"}
                ]}
            ]
        });
        let result = rectify_anthropic_request(&mut body);
        assert!(!result.removed_top_level_thinking);
        assert!(body.get("thinking").is_some());
    }

    #[test]
    fn rectify_keeps_top_level_when_assistant_starts_with_thinking() {
        let mut body = json!({
            "thinking": {"type": "enabled", "budget_tokens": 1000},
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "first", "signature": "s"},
                    {"type": "tool_use", "id": "t1", "name": "exec"}
                ]}
            ]
        });
        let result = rectify_anthropic_request(&mut body);
        // 首块是 thinking 时不删顶层（避免破坏首块签名链路）
        assert!(!result.removed_top_level_thinking);
    }

    #[test]
    fn rectify_no_change_when_clean() {
        let mut body = json!({
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let original = body.clone();
        let result = rectify_anthropic_request(&mut body);
        assert!(!result.is_changed());
        assert_eq!(body, original);
    }

    #[test]
    fn should_remove_thinking_signature_helper() {
        // direct coverage for the helper's 4-condition matrix
        // 1. adaptive → not removed
        let mut body = json!({
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "assistant", "content": [
                {"type": "text"}, {"type": "tool_use"}
            ]}]
        });
        assert!(!should_remove_top_level_thinking(&body));
        // 2. enabled + no tool_use → not removed
        let mut body = json!({
            "thinking": {"type": "enabled"},
            "messages": [{"role": "assistant", "content": [
                {"type": "text", "text": "no tool"}
            ]}]
        });
        assert!(!should_remove_top_level_thinking(&body));
        // 3. enabled + tool_use + no messages → not removed
        let body = json!({"thinking": {"type": "enabled"}});
        assert!(!should_remove_top_level_thinking(&body));
        // 4. missing thinking object
        let body = json!({"messages": []});
        assert!(!should_remove_top_level_thinking(&body));
    }
}
