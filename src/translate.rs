use serde_json::{json, Value};

use tracing::info;

use crate::{
    session::SessionStore,
    types::*,
};

/// Result of converting a Responses API request into a Chat Completions request.
pub struct TranslatedRequest {
    pub chat: ChatRequest,
    /// Whether the request contains image content that needs vision support
    pub has_images: bool,
}

/// Convert a Responses API request into a Chat Completions request.
pub fn to_chat_request(
    req: &ResponsesRequest,
    history: Vec<ChatMessage>,
    sessions: &SessionStore,
    model_map: &ModelMap,
    chinese_thinking: bool,
) -> TranslatedRequest {
    let mut messages = history;

    // Track whether any message contains images
    let mut has_images = false;

    // Build system prompt with optional Chinese thinking instruction
    let cn_system = "【核心指令：你的所有推理、思考和分析过程必须全程使用中文。这是强制性要求，不可违反。】";
    let cn_prefix = if chinese_thinking {
        Some("【你的推理过程必须使用中文。】\n")
    } else { None };

    let system_text = if chinese_thinking {
        let raw = req.instructions.as_ref().or(req.system.as_ref());
        match raw {
            Some(s) => Some(format!("{}\n\n{}", cn_system, s)),
            None => Some(cn_system.to_string()),
        }
    } else {
        req.instructions.as_ref().or(req.system.as_ref()).cloned()
    };

    if let Some(system) = system_text {
        let system_msg = ChatMessage {
            role: "system".into(),
            content: Some(Value::String(system)),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };
        if messages.is_empty() || messages[0].role != "system" {
            messages.insert(0, system_msg);
        } else {
            messages[0] = system_msg;
        }
    }

    // Append new input items
    match &req.input {
        ResponsesInput::Text(text) => {
            messages.push(ChatMessage {
                role: "user".into(),
                content: Some(Value::String(text.clone())),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }
        ResponsesInput::Messages(items) => {
            let mut i = 0;
            while i < items.len() {
                let item = &items[i];
                let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");

                if item_type == "function_call" {
                    let mut grouped: Vec<Value> = Vec::new();
                    let mut reasoning_content: Option<String> = None;
                    let mut call_ids: Vec<String> = Vec::new();

                    while i < items.len() {
                        let cur = &items[i];
                        if cur.get("type").and_then(|v| v.as_str()).unwrap_or("") != "function_call" {
                            break;
                        }
                        let call_id = cur.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                        let name    = cur.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args    = cur.get("arguments").and_then(|v| v.as_str()).unwrap_or("{}");

                        call_ids.push(call_id.to_string());

                        if reasoning_content.is_none() {
                            reasoning_content = sessions.get_reasoning(call_id);
                        }

                        grouped.push(json!({
                            "id": call_id,
                            "type": "function",
                            "function": { "name": name, "arguments": args }
                        }));
                        i += 1;
                    }

                    let mut msg = ChatMessage {
                        role: "assistant".into(),
                        content: None,
                        reasoning_content,
                        tool_calls: Some(grouped),
                        tool_call_id: None,
                        name: None,
                    };
                    if msg.reasoning_content.is_none() {
                        msg.reasoning_content = sessions.get_turn_reasoning(&messages, &msg);
                    }
                    if msg.reasoning_content.is_none() {
                        msg.reasoning_content = sessions.scan_history_reasoning(&messages, &call_ids);
                    }
                    messages.push(msg);
                } else {
                    match item_type {
                        "function_call_output" | "mcp_tool_call_output" | "custom_tool_call_output" | "tool_search_output" => {
                            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                            let success = item.get("success").and_then(|v| v.as_bool()).unwrap_or(true);
                            // Support both plain string and content items array
                            let output = match item.get("output") {
                                Some(Value::String(s)) => s.clone(),
                                Some(Value::Array(parts)) => {
                                    parts.iter()
                                        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                                        .collect::<Vec<_>>()
                                        .join("")
                                }
                                _ => String::new(),
                            };
                            let display = if !success {
                                format!("[FAILED] {}", output)
                            } else {
                                output
                            };
                            messages.push(ChatMessage {
                                role: "tool".into(),
                                content: Some(Value::String(display)),
                                reasoning_content: None,
                                tool_calls: None,
                                tool_call_id: Some(call_id.to_string()),
                                name: None,
                            });
                        }
                        _ => {
                            let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                            let role = match role {
                                "developer" => "system",
                                other => other,
                            }.to_string();
                            let (text, has_img) = value_to_text_with_flag(item.get("content"));
                            let mut msg = ChatMessage {
                                role,
                                content: Some(Value::String(text.clone())),
                                reasoning_content: None,
                                tool_calls: None,
                                tool_call_id: None,
                                name: None,
                            };
                            if has_img {
                                has_images = true;
                                // Reconstruct multimodal content with images
                                if let Some(raw_content) = item.get("content") {
                                    if let Some(parts) = raw_content.as_array() {
                                        let multimodal: Vec<Value> = parts.iter().map(|p| {
                                            let typ = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                            match typ {
                                                "image_url" => p.clone(),
                                                "input_image" => {
                                                    let url = p.get("image_url").and_then(|v| v.as_str()).unwrap_or("");
                                                    json!({"type": "image_url", "image_url": {"url": url}})
                                                }
                                                _ => {
                                                    let text = p.get("text").and_then(|t| t.as_str()).unwrap_or("");
                                                    if text.contains("data:image/") {
                                                        json!({"type": "image_url", "image_url": {"url": text}})
                                                    } else {
                                                        json!({"type": "text", "text": text})
                                                    }
                                                }
                                            }
                                        }).collect();
                                        msg.content = Some(Value::Array(multimodal));
                                    } else if let Some(s) = raw_content.as_str() {
                                        if let Some(pos) = s.find("data:image/") {
                                            let text_before = s[..pos].trim().to_string();
                                            let image_url = s[pos..].trim().to_string();
                                            let mut parts: Vec<Value> = Vec::new();
                                            if !text_before.is_empty() {
                                                parts.push(json!({"type": "text", "text": text_before}));
                                            }
                                            if !image_url.is_empty() {
                                                parts.push(json!({"type": "image_url", "image_url": {"url": image_url}}));
                                            }
                                            if !parts.is_empty() {
                                                msg.content = Some(Value::Array(parts));
                                            }
                                        }
                                    }
                                }
                            }
                            if msg.role == "assistant" {
                                msg.reasoning_content = sessions.get_turn_reasoning(&messages, &msg);
                            }
                            messages.push(msg);
                        }
                    }
                    i += 1;
                }
            }
        }
    }

    // Chinese thinking: prepend instruction to last user message
    if let Some(ref prefix) = cn_prefix {
        if let Some(last_user) = messages.iter_mut().rev().find(|m| m.role == "user") {
            match last_user.content {
                Some(Value::String(ref mut s)) => {
                    *s = format!("{}{}", prefix, s);
                }
                Some(Value::Array(ref mut parts)) => {
                    parts.insert(0, json!({"type": "text", "text": prefix}));
                }
                _ => {}
            }
        }
    }

    let dropped_tool_messages = sanitize_tool_messages(&mut messages);
    if dropped_tool_messages > 0 {
        info!("dropped {} orphan tool message(s) before upstream translation", dropped_tool_messages);
    }

    // Sanitize: replace null content with "" (DeepSeek rejects null)
    for msg in &mut messages {
        if msg.content.is_none() && msg.tool_calls.is_none() {
            msg.content = Some(Value::String(String::new()));
        }
    }

    // Resolve model name
    let mapped_model = resolve_model(&req.model, model_map);

    // Map reasoning effort
    let effort = req.reasoning.as_ref().and_then(|r| r.effort.as_deref());
    let (reasoning_effort, thinking) = map_effort(effort);

    // Filter + convert Codex tools → OpenAI Chat tools.
    // apply_patch is translated to exec_command-compatible function tool.
    let tools: Vec<Value> = req.tools.iter()
        .filter(|t| {
            let typ = t.get("type").and_then(Value::as_str).unwrap_or("");
            typ != "web_search"
                && typ != "web_search_preview"
        })
        .flat_map(|t| {
            let converted = convert_tool(t);
            // Namespace tools expand to arrays; flatten them
            if let Some(arr) = converted.as_array() {
                arr.clone()
            } else {
                vec![converted]
            }
        })
        .collect();

    // Deduplicate tools by function name (DeepSeek requires unique names)
    use std::collections::HashSet;
    let mut seen_names = HashSet::new();
    let tools: Vec<Value> = tools
        .into_iter()
        .filter(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .map(|name| seen_names.insert(name.to_string()))
                .unwrap_or(true) // keep tools without a function name
        })
        .collect();

    // Detect web_search tool → enable DeepSeek web_search_options
    let web_search_enabled = req.tools.iter().any(|t| {
        let typ = t.get("type").and_then(Value::as_str).unwrap_or("");
        typ == "web_search" || typ == "web_search_preview"
    });

    TranslatedRequest {
        chat: ChatRequest {
            model: mapped_model,
            messages,
            tools,
            temperature: req.temperature,
            top_p: req.top_p,
            max_tokens: req.max_output_tokens,
            stream: req.stream,
            reasoning_effort,
            thinking,
            tool_choice: req.tool_choice.clone(),
            stream_options: Some(StreamOptions { include_usage: true }),
            web_search_options: if web_search_enabled {
                Some(json!({"search_context": {"cache_control": {"type": "ephemeral"}}}))
            } else {
                None
            },
        },
        has_images,
    }
}

fn sanitize_tool_messages(messages: &mut Vec<ChatMessage>) -> usize {
    use std::collections::HashSet;

    let original = std::mem::take(messages);
    let mut sanitized: Vec<ChatMessage> = Vec::with_capacity(original.len());
    let mut dropped = 0usize;
    let mut i = 0usize;

    while i < original.len() {
        let msg = &original[i];

        if msg.role == "assistant" && msg.tool_calls.is_some() {
            let expected_ids: Vec<String> = msg
                .tool_calls
                .as_ref()
                .unwrap()
                .iter()
                .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                .collect();

            let expected_set: HashSet<String> = expected_ids.iter().cloned().collect();
            let mut found_set: HashSet<String> = HashSet::new();
            let mut matched_tools: Vec<ChatMessage> = Vec::new();
            let mut j = i + 1;

            while j < original.len() {
                let next = &original[j];
                if next.role != "tool" {
                    break;
                }

                let keep = next
                    .tool_call_id
                    .as_ref()
                    .map(|id| expected_set.contains(id))
                    .unwrap_or(false);

                if keep {
                    if let Some(id) = &next.tool_call_id {
                        found_set.insert(id.clone());
                    }
                    matched_tools.push(next.clone());
                } else {
                    dropped += 1;
                }
                j += 1;
            }

            let ended_at_tail = j == original.len();
            let complete = !expected_set.is_empty() && found_set == expected_set;
            let pending_tail = ended_at_tail && matched_tools.is_empty();

            if complete || pending_tail {
                sanitized.push(msg.clone());
                sanitized.extend(matched_tools);
            } else {
                dropped += 1 + matched_tools.len();
            }

            i = j;
            continue;
        }

        if msg.role == "tool" {
            dropped += 1;
            i += 1;
            continue;
        }

        sanitized.push(msg.clone());
        i += 1;
    }

    *messages = sanitized;
    dropped
}

/// Responses API flat tool → Chat Completions nested format.
/// Handles function, custom (apply_patch), and namespace (MCP) types.
fn convert_tool(tool: &Value) -> Value {
    let Some(obj) = tool.as_object() else {
        return tool.clone();
    };
    let typ = obj.get("type").and_then(Value::as_str).unwrap_or("");

    match typ {
        // Already in the right format
        _ if obj.contains_key("function") => tool.clone(),

        // Standard function type → nest under "function" key
        "function" => {
            let mut func = serde_json::Map::new();
            if let Some(v) = obj.get("name") { func.insert("name".into(), v.clone()); }
            if let Some(v) = obj.get("description") { func.insert("description".into(), v.clone()); }
            if let Some(v) = obj.get("parameters") { func.insert("parameters".into(), v.clone()); }
            if let Some(v) = obj.get("strict") { func.insert("strict".into(), v.clone()); }
            json!({"type": "function", "function": func})
        }

        // Namespace tools (MCP) → expand sub-tools as individual functions
        "namespace" => {
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("namespace");
            let mut expanded = Vec::new();
            if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
                for sub_tool in tools {
                    let sub = convert_tool(sub_tool);
                    // Prefix sub-tool names with namespace for dedup
                    if let Some(sub_name) = sub.get("function").and_then(|f| f.get("name")).and_then(Value::as_str) {
                        let full_name = format!("{}__{}", name, sub_name);
                        let mut sub_obj = sub.as_object().cloned().unwrap_or_default();
                        if let Some(func) = sub_obj.get_mut("function") {
                            if let Some(fobj) = func.as_object_mut() {
                                fobj.insert("name".into(), json!(full_name));
                            }
                        }
                        expanded.push(Value::Object(sub_obj));
                    }
                }
            }
            // If no sub-tools, create a basic placeholder
            if expanded.is_empty() {
                let desc = obj.get("description").and_then(Value::as_str).unwrap_or("");
                let full_name = format!("{}", name);
                expanded.push(json!({
                    "type": "function",
                    "function": {
                        "name": full_name,
                        "description": desc
                    }
                }));
            }
            // Return first tool as the converted value (outer iter handles collection)
            json!(expanded)
        }

        // Custom tools like apply_patch → wrap with a reasonable parameter schema
        "custom" => {
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("custom_tool");
            let desc = obj.get("description").and_then(Value::as_str).unwrap_or("");

            // apply_patch → mapped to exec_command (identical parameter schema)
            let params = if name == "apply_patch" {
                json!({
                    "type": "object",
                    "properties": {
                        "cmd": {
                            "type": "string",
                            "description": "Shell command to execute."
                        },
                        "workdir": {
                            "type": "string",
                            "description": "Optional working directory to run the command in; defaults to the turn cwd."
                        },
                        "shell": {
                            "type": "string",
                            "description": "Shell binary to launch. Defaults to the user's default shell."
                        },
                        "tty": {
                            "type": "boolean",
                            "description": "Whether to allocate a TTY for the command. Defaults to false (plain pipes)."
                        },
                        "sandbox_permissions": {
                            "type": "string",
                            "description": "Sandbox permissions. Defaults to \"use_default\"."
                        },
                        "max_output_tokens": {
                            "type": "number",
                            "description": "Maximum number of tokens to return."
                        },
                        "justification": {
                            "type": "string",
                            "description": "Justification for running outside sandbox (only when sandbox_permissions is require_escalated)."
                        },
                        "login": {
                            "type": "boolean",
                            "description": "Whether to run the shell with -l/-i semantics. Defaults to true."
                        },
                        "yield_time_ms": {
                            "type": "number",
                            "description": "How long to wait (ms) for output before yielding."
                        },
                        "prefix_rule": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Prefix command pattern for future sandbox escalation."
                        }
                    },
                    "required": ["cmd"]
                })
            } else {
                // Generic custom tool: make a simple text parameter
                json!({
                    "type": "object",
                    "properties": {
                        "input": {
                            "type": "string",
                            "description": format!("Input for {}", name)
                        }
                    }
                })
            };

            json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": desc,
                    "parameters": params
                }
            })
        }

        // LocalShell → wrap as exec_command-compatible function
        "local_shell" => {
            info!("🐚 local_shell tool converted");
            let name = obj.get("name").and_then(Value::as_str).unwrap_or("local_shell");
            let desc = obj.get("description").and_then(Value::as_str).unwrap_or("");
            json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": desc,
                    "parameters": obj.get("parameters").unwrap_or(&json!({"type": "object"}))
                }
            })
        }

        // Unknown type → best-effort
        _ => {
            let mut func = serde_json::Map::new();
            if let Some(v) = obj.get("name") { func.insert("name".into(), v.clone()); }
            if let Some(v) = obj.get("description") { func.insert("description".into(), v.clone()); }
            if let Some(v) = obj.get("parameters") { func.insert("parameters".into(), v.clone()); }
            if !func.contains_key("name") {
                func.insert("name".into(), json!(typ));
            }
            if !func.contains_key("description") {
                func.insert("description".into(), json!(format!("Codex tool: {}", typ)));
            }
            json!({"type": "function", "function": func})
        }
    }
}

/// Convert Chat Completions response → Responses API response.
pub fn from_chat_response(
    id: String,
    model: &str,
    chat: ChatResponse,
) -> (ResponsesResponse, Vec<ChatMessage>) {
    let choice = chat.choices.into_iter().next().unwrap_or_else(|| ChatChoice {
        message: ChatMessage {
            role: "assistant".into(),
            content: Some(Value::String(String::new())),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    });

    let text = choice.message.content.clone().unwrap_or_default();
    let usage = chat.usage.unwrap_or(ChatUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        completion_tokens_details: None,
        prompt_cache_hit_tokens: None,
        prompt_cache_miss_tokens: None,
        prompt_tokens_details: None,
    });

    let response = ResponsesResponse {
        id,
        object: "response",
        model: model.to_string(),
        output: vec![ResponsesOutputItem {
            kind: "message".into(),
            role: "assistant".into(),
            content: vec![ContentPart {
                kind: "output_text".into(),
                text: Some(text.as_str().unwrap_or("").to_string()),
            }],
            phase: None,
        }],
        usage: ResponsesUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        },
    };

    (response, vec![choice.message])
}

/// Collapse a Responses API content value to plain text + has_images flag.
fn value_to_text_with_flag(v: Option<&Value>) -> (String, bool) {
    match v {
        None => (String::new(), false),
        Some(Value::String(s)) => {
            let has_img = s.contains("data:image/");
            (s.clone(), has_img)
        }
        Some(Value::Array(parts)) => {
            let has_img = parts.iter().any(|p| {
                let typ = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                typ == "image_url" || typ == "input_image"
                || p.get("text").and_then(|t| t.as_str()).map(|s| s.contains("data:image/")).unwrap_or(false)
            });
            let text = parts
                .iter()
                .filter(|p| {
                    let typ = p.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    typ != "image_url" && typ != "input_image"
                })
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("");
            (text, has_img)
        }
        Some(other) => (other.to_string(), false),
    }
}

/// Collapse a Responses API content value to plain text, filtering images (for backwards compat).
#[cfg(test)]
fn value_to_text(v: Option<&Value>) -> String {
    value_to_text_with_flag(v).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn empty_map() -> ModelMap {
        HashMap::new()
    }

    fn base_req(input: ResponsesInput) -> ResponsesRequest {
        ResponsesRequest {
            model: "test".into(),
            input,
            previous_response_id: None,
            tools: vec![],
            stream: false,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            system: None,
            instructions: None,
            reasoning: None,
            tool_choice: None,
            store: None,
            metadata: None,
            truncation: None,
        }
    }

    #[test]
    fn test_text_input() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Text("hello".into()));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert_eq!(chat.messages[0].content.as_ref().and_then(|v| v.as_str()), Some("hello"));
    }

    #[test]
    fn test_system_from_instructions() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("hi".into()));
        req.instructions = Some("be helpful".into());
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages[0].role, "system");
        assert_eq!(chat.messages[0].content.as_ref().and_then(|v| v.as_str()), Some("be helpful"));
    }

    #[test]
    fn test_developer_to_system() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "message", "role": "developer", "content": "secret"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages[0].role, "system");
    }

    #[test]
    fn test_function_call_grouping() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "c1", "name": "a", "arguments": "{}"}),
            json!({"type": "function_call", "call_id": "c2", "name": "b", "arguments": "{}"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 1);
        let calls = chat.messages[0].tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_model_remapping() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("hi".into()));
        req.model = "gpt-5.5".into();
        let mut map: ModelMap = HashMap::new();
        map.insert("gpt-5.5".into(), "deepseek-v4-pro".into());
        let chat = to_chat_request(&req, vec![], &sessions, &map, false).chat;
        assert_eq!(chat.model, "deepseek-v4-pro");
    }

    #[test]
    fn test_image_filtered_from_text() {
        let result = value_to_text(Some(&json!([
            {"type": "input_text", "text": "hello "},
            {"type": "image_url", "image_url": {"url": "data:..."}},
            {"type": "input_text", "text": "world"}
        ])));
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_null_content_fixed() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Text("".into()));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages[0].content.as_ref().and_then(|v| v.as_str()), Some(""));
    }

    #[test]
    fn test_effort_mapping_low() {
        let (eff, think) = map_effort(Some("low"));
        assert_eq!(eff, None);
        assert_eq!(think, Some(json!({"type": "disabled"})));
    }

    #[test]
    fn test_effort_mapping_xhigh() {
        let (eff, think) = map_effort(Some("xhigh"));
        assert_eq!(eff, Some("max".into()));
        assert_eq!(think, Some(json!({"type": "enabled"})));
    }

    #[test]
    fn test_effort_mapping_none() {
        let (eff, think) = map_effort(Some("none"));
        assert_eq!(eff, None);
        assert_eq!(think, Some(json!({"type": "disabled"})));
    }

    #[test]
    fn test_effort_mapping_minimal() {
        let (eff, think) = map_effort(Some("minimal"));
        assert_eq!(eff, None);
        assert_eq!(think, Some(json!({"type": "disabled"})));
    }

    #[test]
    fn test_effort_mapping_default() {
        let (eff, think) = map_effort(None);
        assert_eq!(eff, Some("high".into()));
        assert_eq!(think, Some(json!({"type": "enabled"})));
    }

    #[test]
    fn test_scan_history_fallback() {
        let sessions = SessionStore::new();
        let prev = ChatMessage {
            role: "assistant".into(),
            content: None,
            reasoning_content: Some("prior_reason".into()),
            tool_calls: Some(vec![json!({"id": "tc_x", "type": "function", "function": {"name": "f", "arguments": "{}"}})]),
            tool_call_id: None,
            name: None,
        };
        let history = vec![
            ChatMessage { role: "user".into(), content: Some("q".into()), reasoning_content: None, tool_calls: None, tool_call_id: None, name: None },
            prev,
        ];
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "tc_x", "name": "f", "arguments": "{}"}),
            json!({"type": "function_call_output", "call_id": "tc_x", "output": "result"}),
        ]));
        let chat = to_chat_request(&req, history, &sessions, &empty_map(), false).chat;
        let fc_msg = chat.messages.iter().find(|m| m.role == "assistant" && m.tool_calls.is_some());
        assert!(fc_msg.is_some());
        assert_eq!(fc_msg.unwrap().reasoning_content.as_deref(), Some("prior_reason"));
    }

    #[test]
    fn test_orphan_tool_outputs_are_dropped() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call_output", "call_id": "missing", "output": "result"}),
            json!({"type": "message", "role": "user", "content": "next"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
    }

    #[test]
    fn test_tool_outputs_with_matching_tool_calls_are_kept() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "c1", "name": "a", "arguments": "{}"}),
            json!({"type": "function_call_output", "call_id": "c1", "output": "result"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(chat.messages[1].role, "tool");
        assert_eq!(chat.messages[1].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn test_incomplete_tool_call_sequence_is_dropped() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "c1", "name": "a", "arguments": "{}"}),
            json!({"type": "message", "role": "user", "content": "next"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert_eq!(chat.messages[0].content.as_ref().and_then(|v| v.as_str()), Some("next"));
    }

    #[test]
    fn test_partial_tail_tool_outputs_are_dropped() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "c1", "name": "a", "arguments": "{}"}),
            json!({"type": "function_call", "call_id": "c2", "name": "b", "arguments": "{}"}),
            json!({"type": "function_call_output", "call_id": "c1", "output": "result"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert!(chat.messages.is_empty());
    }

    #[test]
    fn test_tool_outputs_must_be_contiguous_to_be_kept() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "c1", "name": "a", "arguments": "{}"}),
            json!({"type": "message", "role": "assistant", "content": "intervening"}),
            json!({"type": "function_call_output", "call_id": "c1", "output": "result"}),
            json!({"type": "message", "role": "user", "content": "next"}),
        ]));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(chat.messages[0].content.as_ref().and_then(|v| v.as_str()), Some("intervening"));
        assert_eq!(chat.messages[1].role, "user");
    }


    #[test]
    fn test_stream_options_included() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("hi".into()));
        req.stream = true;
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert!(chat.stream_options.is_some());
        assert!(chat.stream_options.as_ref().unwrap().include_usage);
    }

    #[test]
    fn test_format_usage_basic() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            completion_tokens_details: None,
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        };
        let s = format_usage(Some(&usage));
        assert!(s.contains("in=100 out=50"));
    }

    #[test]
    fn test_format_usage_with_reasoning() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            completion_tokens_details: Some(TokenDetails { reasoning_tokens: Some(30) }),
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        };
        let s = format_usage(Some(&usage));
        assert!(s.contains("reason=30"));
    }
}
