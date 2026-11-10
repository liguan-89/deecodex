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

pub fn normalize_apply_patch_input(input: &str) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let has_unified_pair = lines
        .windows(2)
        .any(|pair| pair[0].starts_with("--- ") && pair[1].starts_with("+++ "));
    let has_unified_hunk_header = lines.iter().any(|line| is_unified_hunk_header(line));
    if !has_unified_pair && !has_unified_hunk_header {
        return input.to_string();
    }

    let has_patch_action = lines.iter().any(|line| {
        line.starts_with("*** Update File:")
            || line.starts_with("*** Add File:")
            || line.starts_with("*** Delete File:")
    });
    let has_begin = lines.iter().any(|line| line.trim() == "*** Begin Patch");
    let has_end = lines.iter().any(|line| line.trim() == "*** End Patch");
    let mut changed = false;
    let mut normalized = Vec::new();

    if !has_begin {
        normalized.push("*** Begin Patch".to_string());
        changed = true;
    }

    let mut idx = 0usize;
    while idx < lines.len() {
        let line = lines[idx];
        if line.starts_with("diff --git ") {
            changed = true;
            idx += 1;
            continue;
        }

        if is_unified_hunk_header(line) {
            normalized.push("@@".to_string());
            changed = true;
            idx += 1;
            continue;
        }

        if line.starts_with("--- ") && idx + 1 < lines.len() && lines[idx + 1].starts_with("+++ ") {
            if !has_patch_action {
                if let Some(path) = unified_diff_target_path(line, lines[idx + 1]) {
                    normalized.push(format!("*** Update File: {path}"));
                }
            }
            changed = true;
            idx += 2;
            continue;
        }

        normalized.push(line.to_string());
        idx += 1;
    }

    if !has_end {
        normalized.push("*** End Patch".to_string());
        changed = true;
    }

    if changed {
        let mut result = normalized.join("\n");
        if input.ends_with('\n') {
            result.push('\n');
        }
        result
    } else {
        input.to_string()
    }
}

fn is_unified_hunk_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with("@@ -") && trimmed.ends_with("@@") && trimmed.contains(" +")
}

fn unified_diff_target_path(old_header: &str, new_header: &str) -> Option<String> {
    unified_diff_header_path(new_header).or_else(|| unified_diff_header_path(old_header))
}

fn unified_diff_header_path(header: &str) -> Option<String> {
    let path = header
        .strip_prefix("--- ")
        .or_else(|| header.strip_prefix("+++ "))?
        .split_whitespace()
        .next()?;
    if path == "/dev/null" {
        return None;
    }

    let mut path = path.trim_matches('"').to_string();
    if let Some(stripped) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        path = stripped.to_string();
    }
    if path.starts_with("tmp/") {
        path.insert(0, '/');
    }
    Some(path)
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
    fn normalize_apply_patch_converts_unified_diff_headers() {
        let input = concat!(
            "*** Begin Patch\n",
            "--- a/tmp/codex-minimax-toolchain-test/file1.txt\n",
            "+++ b/tmp/codex-minimax-toolchain-test/file1.txt\n",
            "@@ -1 +1 @@\n",
            "-minimax test\n",
            "+PATCH_OK minimax test\n",
            "*** End Patch"
        );

        let normalized = normalize_apply_patch_input(input);

        assert!(normalized.contains("*** Update File: /tmp/codex-minimax-toolchain-test/file1.txt"));
        assert!(!normalized.contains("--- a/tmp/codex-minimax-toolchain-test/file1.txt"));
        assert!(!normalized.contains("+++ b/tmp/codex-minimax-toolchain-test/file1.txt"));
        assert!(normalized.contains("@@\n-minimax test"));
    }

    #[test]
    fn normalize_apply_patch_removes_unified_headers_after_update_file() {
        let input = concat!(
            "*** Begin Patch\n",
            "*** Update File: /tmp/demo.txt\n",
            "--- a/tmp/demo.txt\n",
            "+++ b/tmp/demo.txt\n",
            "@@\n",
            "-old\n",
            "+new\n",
            "*** End Patch"
        );

        let normalized = normalize_apply_patch_input(input);

        assert_eq!(
            normalized.matches("*** Update File: /tmp/demo.txt").count(),
            1
        );
        assert!(!normalized.contains("--- a/tmp/demo.txt"));
        assert!(!normalized.contains("+++ b/tmp/demo.txt"));
    }

    #[test]
    fn normalize_apply_patch_simplifies_unified_hunk_ranges() {
        let input = concat!(
            "*** Begin Patch\n",
            "*** Update File: /tmp/codex-minimax-toolchain-test/test1.txt\n",
            "@@ -1 +1 @@\n",
            "-测试文件1 - minimax工具链测试\n",
            "+测试文件1 - minimax工具链测试\n",
            "+PATCH_OK\n",
            "*** End Patch"
        );

        let normalized = normalize_apply_patch_input(input);

        assert!(normalized.contains("@@\n-测试文件1"));
        assert!(!normalized.contains("@@ -1 +1 @@"));
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
