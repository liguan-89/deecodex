use serde_json::{json, Value};

pub fn merge_response_extra(response: &mut Value, extra: &Value) {
    let Some(extra_obj) = extra.as_object() else {
        return;
    };
    for (key, value) in extra_obj {
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
