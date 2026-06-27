//! 请求体私有参数过滤。
//!
//! 借鉴 cc-switch `body_filter.rs`：递归剔除以 `_` 开头的字段，防止本地客户端
//! 内部追踪/调试字段（`_internal_id`、`_debug_mode`、`_session_token` 等）透传
//! 到 minimax/mimo/Anthropic 等上游造成内部信息泄露。
//!
//! 豁免 JSON Schema 的 `properties` / `patternProperties` / `definitions` / `$defs`
//! 名空间：这些 key 下面是用户工具定义，不是私有参数。
//!
//! 典型使用：入站请求解析 JSON 之后、转换协议之前调用一次。

use serde_json::Value;
use std::collections::HashSet;

/// 默认白名单：保留这些 `_` 前缀字段（用于 deecodex 自身运行时需要保留的字段）。
pub const DEFAULT_WHITELIST: &[&str] = &[];

/// 过滤私有参数（默认白名单为空）。
pub fn filter_private_params(body: Value) -> Value {
    filter_recursive(body, &mut Vec::new(), &HashSet::new())
}

/// 过滤私有参数（带白名单）。
///
/// 递归遍历 JSON 结构，移除所有以下划线开头的字段，保留白名单中指定的字段。
pub fn filter_private_params_with_whitelist(body: Value, whitelist: &[&str]) -> Value {
    let set: HashSet<&str> = whitelist.iter().copied().collect();
    filter_recursive(body, &mut Vec::new(), &set)
}

fn filter_recursive(value: Value, path: &mut Vec<String>, whitelist: &HashSet<&str>) -> Value {
    match value {
        Value::Object(map) => {
            // 上一级 key 是 JSON Schema 命名空间时，递归停止过滤（保留用户工具定义）
            let in_schema_namespace = path.last().is_some_and(|key| matches_schema_namespace(key));
            let filtered: serde_json::Map<String, Value> = map
                .into_iter()
                .filter_map(|(key, val)| {
                    if !in_schema_namespace
                        && key.starts_with('_')
                        && !whitelist.contains(key.as_str())
                    {
                        None
                    } else {
                        path.push(key.clone());
                        let filtered_value = filter_recursive(val, path, whitelist);
                        path.pop();
                        Some((key, filtered_value))
                    }
                })
                .collect();
            Value::Object(filtered)
        }
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|v| filter_recursive(v, path, whitelist))
                .collect(),
        ),
        other => other,
    }
}

/// 上一级 key 是这些时，当前层所有 `_` 前缀字段视为用户定义（不剥）。
fn matches_schema_namespace(key: &str) -> bool {
    matches!(
        key,
        "properties" | "patternProperties" | "definitions" | "$defs"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filter_top_level_private_params() {
        let input = json!({
            "model": "claude-3",
            "_internal_id": "abc123",
            "_debug": true,
            "max_tokens": 1024
        });
        let out = filter_private_params(input);
        assert!(out.get("model").is_some());
        assert!(out.get("max_tokens").is_some());
        assert!(out.get("_internal_id").is_none());
        assert!(out.get("_debug").is_none());
    }

    #[test]
    fn filter_nested_private_params_in_messages() {
        let input = json!({
            "model": "claude-3",
            "messages": [
                {
                    "role": "user",
                    "content": "hello",
                    "_session_token": "secret"
                }
            ]
        });
        let out = filter_private_params(input);
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hello");
        assert!(messages[0].get("_session_token").is_none());
    }

    #[test]
    fn filter_nested_private_params_in_metadata() {
        let input = json!({
            "metadata": {
                "user_id": "user-1",
                "_tracking_id": "track-1"
            }
        });
        let out = filter_private_params(input);
        assert_eq!(out["metadata"]["user_id"], "user-1");
        assert!(out["metadata"].get("_tracking_id").is_none());
    }

    #[test]
    fn whitelist_keeps_private_params() {
        let input = json!({
            "_internal_id": "abc",
            "_dee_session": "session-1"
        });
        let out = filter_private_params_with_whitelist(input, &["_dee_session"]);
        assert!(out.get("_internal_id").is_none());
        assert_eq!(out["_dee_session"], "session-1");
    }

    #[test]
    fn properties_namespace_keeps_user_tool_field_names() {
        // JSON Schema 的 properties 里的 _ 前缀 key 是用户工具参数名，不应过滤
        let input = json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "exec_command",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "_cmd": {"type": "string"},
                            "path": {"type": "string"}
                        }
                    }
                }
            }]
        });
        let out = filter_private_params(input);
        let props = &out["tools"][0]["function"]["parameters"]["properties"];
        // 关键：_cmd 必须保留
        assert!(props.get("_cmd").is_some());
        assert!(props.get("path").is_some());
    }

    #[test]
    fn deep_nested_arrays_are_filtered() {
        let input = json!({
            "a": [
                {"b": [{"_x": 1, "y": 2}]}
            ]
        });
        let out = filter_private_params(input);
        assert!(out["a"][0]["b"][0].get("_x").is_none());
        assert_eq!(out["a"][0]["b"][0]["y"], 2);
    }

    #[test]
    fn non_object_values_passthrough() {
        let input = json!({
            "scalar": "ok",
            "list": [1, 2, 3],
            "bool": true,
            "null": null
        });
        let out = filter_private_params(input);
        assert_eq!(out["scalar"], "ok");
        assert_eq!(out["list"], json!([1, 2, 3]));
        assert_eq!(out["bool"], true);
        assert!(out["null"].is_null());
    }
}
