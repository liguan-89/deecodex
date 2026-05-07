use serde_json::{json, Value};

const LOCAL_OUTPUT_PREFIX_ITEMS_KEY: &str = "x_deecodex_local_output_prefix_items";

pub fn merge_response_extra(response: &mut Value, extra: &Value) {
    let Some(extra_obj) = extra.as_object() else {
        return;
    };
    let response_completed = response
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "completed");
    if response_completed {
        if let Some(prefix_items) = extra_obj
            .get(LOCAL_OUTPUT_PREFIX_ITEMS_KEY)
            .and_then(Value::as_array)
        {
            if let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) {
                let mut merged = prefix_items.clone();
                merged.append(output);
                *output = merged;
            }
        }
    }
    for (key, value) in extra_obj {
        if key == LOCAL_OUTPUT_PREFIX_ITEMS_KEY {
            continue;
        }
        if response.get(key).is_none() || response.get(key) == Some(&Value::Null) {
            response[key] = value.clone();
        }
    }
    if let Some(max) = extra
        .get("max_tool_calls")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
    {
        limit_function_call_outputs(response, max);
    }
}

pub fn limit_function_call_outputs(response: &mut Value, max_tool_calls: usize) {
    let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };
    let mut seen = 0usize;
    output.retain(|item| {
        if item.get("type").and_then(Value::as_str) == Some("function_call") {
            seen += 1;
            seen <= max_tool_calls
        } else {
            true
        }
    });
    if seen > max_tool_calls {
        response["status"] = json!("incomplete");
        response["incomplete_details"] = json!({"reason": "max_tool_calls"});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_merge_empty_extra() {
        let mut response =
            json!({"status": "completed", "output": [{"type": "text", "text": "hello"}]});
        let extra = json!({});
        merge_response_extra(&mut response, &extra);
        assert_eq!(
            response,
            json!({"status": "completed", "output": [{"type": "text", "text": "hello"}]})
        );
    }

    #[test]
    fn test_merge_extra_fields_into_response() {
        let mut response = json!({"status": "completed", "output": []});
        let extra = json!({"custom_field": "custom_value"});
        merge_response_extra(&mut response, &extra);
        assert_eq!(response["custom_field"], "custom_value");
    }

    #[test]
    fn test_merge_prefix_items_injected_when_completed() {
        let mut response =
            json!({"status": "completed", "output": [{"type": "text", "text": "world"}]});
        let extra =
            json!({"x_deecodex_local_output_prefix_items": [{"type": "text", "text": "hello "}]});
        merge_response_extra(&mut response, &extra);
        assert_eq!(
            response["output"],
            json!([
                {"type": "text", "text": "hello "},
                {"type": "text", "text": "world"}
            ])
        );
    }

    #[test]
    fn test_merge_prefix_items_not_injected_when_not_completed() {
        let mut response =
            json!({"status": "in_progress", "output": [{"type": "text", "text": "world"}]});
        let extra =
            json!({"x_deecodex_local_output_prefix_items": [{"type": "text", "text": "hello "}]});
        merge_response_extra(&mut response, &extra);
        assert_eq!(
            response["output"],
            json!([{"type": "text", "text": "world"}])
        );
    }

    #[test]
    fn test_merge_extra_overrides_null_field() {
        let mut response = json!({"status": "completed", "output": [], "extra_info": null});
        let extra = json!({"extra_info": "not null anymore"});
        merge_response_extra(&mut response, &extra);
        assert_eq!(response["extra_info"], "not null anymore");
    }

    #[test]
    fn test_merge_extra_does_not_override_existing_field() {
        let mut response = json!({"status": "completed", "output": [], "existing": "original"});
        let extra = json!({"existing": "overridden?"});
        merge_response_extra(&mut response, &extra);
        assert_eq!(response["existing"], "original");
    }

    #[test]
    fn test_merge_prefix_key_not_leaked() {
        let mut response =
            json!({"status": "completed", "output": [{"type": "text", "text": "test"}]});
        let extra = json!({"x_deecodex_local_output_prefix_items": [{"type": "text", "text": "prefix"}], "normal_field": "value"});
        merge_response_extra(&mut response, &extra);
        assert!(response
            .get("x_deecodex_local_output_prefix_items")
            .is_none());
        assert_eq!(response["normal_field"], "value");
    }

    #[test]
    fn test_merge_non_object_extra_is_noop() {
        let mut response = json!({"status": "completed", "output": []});
        let extra = json!("string_value");
        merge_response_extra(&mut response, &extra);
        assert_eq!(response, json!({"status": "completed", "output": []}));
    }

    #[test]
    fn test_limit_function_call_outputs_no_output() {
        let mut response = json!({"status": "completed"});
        limit_function_call_outputs(&mut response, 5);
        assert_eq!(response, json!({"status": "completed"}));
    }

    #[test]
    fn test_limit_function_call_outputs_within_limit() {
        let mut response = json!({"status": "completed", "output": [
            {"type": "function_call", "name": "test"},
            {"type": "text", "text": "hello"}
        ]});
        limit_function_call_outputs(&mut response, 5);
        assert_eq!(
            response["output"],
            json!([
                {"type": "function_call", "name": "test"},
                {"type": "text", "text": "hello"}
            ])
        );
        assert_eq!(response["status"], "completed");
    }

    #[test]
    fn test_limit_function_call_outputs_exceeds_limit() {
        let mut response = json!({"status": "completed", "output": [
            {"type": "function_call", "name": "a"},
            {"type": "function_call", "name": "b"},
            {"type": "text", "text": "hello"}
        ]});
        limit_function_call_outputs(&mut response, 1);
        assert_eq!(
            response["output"],
            json!([
                {"type": "function_call", "name": "a"},
                {"type": "text", "text": "hello"}
            ])
        );
        assert_eq!(response["status"], "incomplete");
        assert_eq!(
            response["incomplete_details"],
            json!({"reason": "max_tool_calls"})
        );
    }

    #[test]
    fn test_limit_function_call_outputs_no_function_calls() {
        let mut response = json!({"status": "completed", "output": [
            {"type": "text", "text": "only text"}
        ]});
        limit_function_call_outputs(&mut response, 0);
        assert_eq!(
            response["output"],
            json!([
                {"type": "text", "text": "only text"}
            ])
        );
        assert_eq!(response["status"], "completed");
    }

    #[test]
    fn test_merge_max_tool_calls_triggers_limiting() {
        let mut response = json!({"status": "completed", "output": [
            {"type": "function_call", "name": "a"},
            {"type": "function_call", "name": "b"},
            {"type": "function_call", "name": "c"},
        ]});
        let extra = json!({"max_tool_calls": 2});
        merge_response_extra(&mut response, &extra);
        assert_eq!(response["output"].as_array().unwrap().len(), 2);
        assert_eq!(response["status"], "incomplete");
    }
}
