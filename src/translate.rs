use std::collections::HashSet;

use serde_json::{json, Value};

use tracing::info;

use crate::{session::SessionStore, types::*};

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
    let cn_system =
        "【核心指令：你的所有推理、思考和分析过程必须全程使用中文。这是强制性要求，不可违反。】";
    let cn_prefix = if chinese_thinking {
        Some("【你的推理过程必须使用中文。】\n")
    } else {
        None
    };

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
                        if cur.get("type").and_then(|v| v.as_str()).unwrap_or("") != "function_call"
                        {
                            break;
                        }
                        let call_id = cur.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                        let name = cur.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let args = cur
                            .get("arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");

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
                        msg.reasoning_content =
                            sessions.scan_history_reasoning(&messages, &call_ids);
                    }
                    messages.push(msg);
                } else {
                    match item_type {
                        "function_call_output"
                        | "mcp_tool_call_output"
                        | "custom_tool_call_output"
                        | "tool_search_output"
                        | "computer_call_output" => {
                            let call_id =
                                item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                            // 为非 function_call_output 的 tool output 合成 assistant，
                            // 确保 sanitize_tool_messages 不会将其当作孤儿丢弃。
                            if item_type != "function_call_output" {
                                let has_matching = messages.last().is_some_and(|m| {
                                    m.role == "assistant"
                                        && m.tool_calls.as_ref().is_some_and(|tc| {
                                            tc.iter().any(|t| {
                                                t.get("id").and_then(|v| v.as_str())
                                                    == Some(call_id)
                                            })
                                        })
                                });
                                if !has_matching {
                                    messages.push(ChatMessage {
                                        role: "assistant".into(),
                                        content: None,
                                        reasoning_content: None,
                                        tool_calls: Some(vec![json!({
                                            "id": call_id,
                                            "type": "function",
                                            "function": {
                                                "name": item_type,
                                                "arguments": "{}"
                                            }
                                        })]),
                                        tool_call_id: None,
                                        name: None,
                                    });
                                }
                            }
                            let success = item
                                .get("success")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);
                            let output = tool_output_text(item_type, item);
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
                            }
                            .to_string();
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
                                                parts.push(
                                                    json!({"type": "text", "text": text_before}),
                                                );
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
                                msg.reasoning_content =
                                    sessions.get_turn_reasoning(&messages, &msg);
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
        info!(
            "dropped {} orphan tool message(s) before upstream translation",
            dropped_tool_messages
        );
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
    let tools: Vec<Value> = req
        .tools
        .iter()
        .filter(|t| {
            let typ = t.get("type").and_then(Value::as_str).unwrap_or("");
            !matches!(
                typ,
                "web_search" | "web_search_preview" | "file_search" | "file_search_preview"
            )
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
            parallel_tool_calls: req.parallel_tool_calls,
            response_format: response_format_from_text(req.text.as_ref()),
            user: req
                .user
                .clone()
                .or_else(|| req.safety_identifier.clone())
                .or_else(|| req.prompt_cache_key.clone()),
            stream_options: if req.stream {
                Some(StreamOptions {
                    include_usage: true,
                })
            } else {
                None
            },
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
    let original = std::mem::take(messages);
    let mut sanitized: Vec<ChatMessage> = Vec::with_capacity(original.len());
    let mut dropped = 0usize;
    let mut i = 0usize;

    while i < original.len() {
        let msg = &original[i];

        if msg.role == "assistant" {
            if let Some(tool_calls) = &msg.tool_calls {
                let expected_ids: Vec<String> = tool_calls
                    .iter()
                    .filter_map(|tc| {
                        tc.get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
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

fn tool_output_text(_item_type: &str, item: &Value) -> String {
    let mut chunks = Vec::new();
    for key in ["screenshot", "image_url"] {
        if let Some(value) = item.get(key) {
            collect_tool_output_value(value, &mut chunks);
        }
    }
    if let Some(output) = item.get("output") {
        collect_tool_output_value(output, &mut chunks);
    }
    if let Some(content) = item.get("content") {
        collect_tool_output_value(content, &mut chunks);
    }
    if chunks.is_empty() {
        serde_json::to_string(item).unwrap_or_default()
    } else {
        chunks.join("\n")
    }
}

fn collect_tool_output_value(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::Null => {}
        Value::String(text) => {
            if text.starts_with("data:image/") {
                chunks.push(format_image_url(text));
            } else {
                chunks.push(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_tool_output_value(item, chunks);
            }
        }
        Value::Object(map) => {
            let before = chunks.len();
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                chunks.push(text.to_string());
            }
            if let Some(url) = map.get("image_url").and_then(Value::as_str) {
                chunks.push(format_image_url(url));
            }
            if let Some(url) = map
                .get("image_url")
                .and_then(|v| v.get("url"))
                .and_then(Value::as_str)
            {
                chunks.push(format_image_url(url));
            }
            if let Some(screenshot) = map.get("screenshot") {
                collect_tool_output_value(screenshot, chunks);
            }
            if chunks.len() == before {
                chunks.push(serialize_output_object(value, map));
            }
        }
        other => chunks.push(other.to_string()),
    }
}

fn serialize_output_object(value: &Value, map: &serde_json::Map<String, Value>) -> String {
    let has_image = map
        .values()
        .any(|v| v.as_str().is_some_and(|s| s.starts_with("data:image/")));
    if has_image {
        let mut cleaned = map.clone();
        for (_, v) in cleaned.iter_mut() {
            if let Some(s) = v.as_str().filter(|s| s.starts_with("data:image/")) {
                *v = Value::String(format_image_url(s));
            }
        }
        serde_json::to_string(&cleaned).unwrap_or_default()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    }
}

fn format_image_url(url: &str) -> String {
    if url.starts_with("data:image/") {
        if let Some(semi) = url.find(';') {
            let mime = &url[5..semi];
            let encoded_start = semi + ";base64,".len();
            let encoded_len = url.len().saturating_sub(encoded_start);
            return format!("[image omitted: {mime} base64 {encoded_len}B]");
        }
        return "[image omitted]".to_string();
    }
    format!("[image_url] {url}")
}

fn response_format_from_text(text: Option<&Value>) -> Option<Value> {
    let format = text?.get("format")?;
    let format_type = format.get("type").and_then(Value::as_str)?;
    match format_type {
        "json_object" => Some(json!({"type": "json_object"})),
        "json_schema" => Some(json!({
            "type": "json_schema",
            "json_schema": {
                "name": format.get("name").and_then(Value::as_str).unwrap_or("response"),
                "schema": format.get("schema").cloned().unwrap_or_else(|| json!({})),
                "strict": format.get("strict").and_then(Value::as_bool).unwrap_or(false)
            }
        })),
        _ => None,
    }
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
            if let Some(v) = obj.get("name") {
                func.insert("name".into(), v.clone());
            }
            if let Some(v) = obj.get("description") {
                func.insert("description".into(), v.clone());
            }
            if let Some(v) = obj.get("parameters") {
                func.insert("parameters".into(), v.clone());
            }
            if let Some(v) = obj.get("strict") {
                func.insert("strict".into(), v.clone());
            }
            json!({"type": "function", "function": func})
        }

        // Namespace tools (MCP) → expand sub-tools as individual functions
        "namespace" => {
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("namespace");
            let mut expanded = Vec::new();
            if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
                for sub_tool in tools {
                    let sub = convert_tool(sub_tool);
                    // Prefix sub-tool names with namespace for dedup
                    if let Some(sub_name) = sub
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(Value::as_str)
                    {
                        let prefix = name.strip_suffix("__").unwrap_or(name);
                        let full_name = format!("{}__{}", prefix, sub_name);
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
                let full_name = name.to_string();
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
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("custom_tool");
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
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("local_shell");
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

        "computer_use" | "computer_use_preview" => {
            let display_width = obj
                .get("display_width")
                .and_then(Value::as_u64)
                .unwrap_or(1024);
            let display_height = obj
                .get("display_height")
                .and_then(Value::as_u64)
                .unwrap_or(768);
            json!({
                "type": "function",
                "function": {
                    "name": "local_computer",
                    "description": format!(
                        "Bridge for Responses computer_use. Request one local browser/computer action for a {display_width}x{display_height} display and wait for a computer_call_output screenshot before continuing."
                    ),
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["screenshot", "click", "double_click", "scroll", "keypress", "type", "wait", "open_url"],
                                "description": "Computer action type."
                            },
                            "x": {"type": "number"},
                            "y": {"type": "number"},
                            "button": {"type": "string"},
                            "scroll_x": {"type": "number"},
                            "scroll_y": {"type": "number"},
                            "keys": {"type": "array", "items": {"type": "string"}},
                            "text": {"type": "string"},
                            "url": {"type": "string"},
                            "display": {"type": "string"},
                            "environment": {"type": "string"}
                        },
                        "required": ["type"]
                    }
                }
            })
        }

        "mcp" | "remote_mcp" => {
            let server_label = obj
                .get("server_label")
                .or_else(|| obj.get("server_url"))
                .and_then(Value::as_str)
                .unwrap_or("remote_mcp");
            json!({
                "type": "function",
                "function": {
                    "name": "local_mcp_call",
                    "description": format!(
                        "Bridge for Responses remote MCP tool calls against {server_label}. Return a tool name and JSON arguments for the local MCP executor."
                    ),
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "server_label": {
                                "type": "string",
                                "description": "MCP server label or URL."
                            },
                            "tool": {
                                "type": "string",
                                "description": "MCP tool name to call."
                            },
                            "arguments": {
                                "type": "object",
                                "description": "JSON arguments for the MCP tool."
                            }
                        },
                        "required": ["tool", "arguments"]
                    }
                }
            })
        }

        // Unknown type → best-effort
        _ => {
            let mut func = serde_json::Map::new();
            if let Some(v) = obj.get("name") {
                func.insert("name".into(), v.clone());
            }
            if let Some(v) = obj.get("description") {
                func.insert("description".into(), v.clone());
            }
            if let Some(v) = obj.get("parameters") {
                func.insert("parameters".into(), v.clone());
            }
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
    let choice = chat
        .choices
        .into_iter()
        .next()
        .unwrap_or_else(|| ChatChoice {
            message: ChatMessage {
                role: "assistant".into(),
                content: Some(Value::String(String::new())),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        });

    let reasoning_content = choice.message.reasoning_content.clone().unwrap_or_default();
    let text = choice.message.content.clone().unwrap_or_default();
    let tool_calls = choice.message.tool_calls.clone().unwrap_or_default();
    let usage = chat.usage.unwrap_or(ChatUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        completion_tokens_details: None,
        prompt_cache_hit_tokens: None,
        prompt_cache_miss_tokens: None,
        prompt_tokens_details: None,
    });

    let mut output = Vec::new();

    // Reasoning as independent output item (matches Responses API format)
    if !reasoning_content.is_empty() {
        let item_id = response_output_item_id("rs", &id, output.len());
        output.push(ResponsesOutputItem {
            kind: "reasoning".into(),
            role: None,
            content: vec![ContentPart {
                kind: "summary_text".into(),
                text: Some(reasoning_content.clone()),
            }],
            id: Some(item_id),
            call_id: None,
            name: None,
            server_label: None,
            arguments: None,
            action: None,
            status: Some("completed".into()),
            phase: None,
        });
    }

    let text_out = text.as_str().unwrap_or("").to_string();
    if !text_out.is_empty() || tool_calls.is_empty() {
        let item_id = response_output_item_id("msg", &id, output.len());
        output.push(ResponsesOutputItem {
            kind: "message".into(),
            role: Some("assistant".into()),
            content: vec![ContentPart {
                kind: "output_text".into(),
                text: Some(text_out),
            }],
            id: Some(item_id),
            call_id: None,
            name: None,
            server_label: None,
            arguments: None,
            action: None,
            status: None,
            phase: None,
        });
    }

    for tool_call in &tool_calls {
        let function = tool_call.get("function").unwrap_or(&Value::Null);
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let arguments = function
            .get("arguments")
            .and_then(Value::as_str)
            .map(str::to_string);
        let is_computer_call = name == "local_computer";
        let call_id = tool_call
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}_{}", id, output.len()));
        let is_mcp_call = name == "local_mcp_call";
        let mcp = if is_mcp_call {
            parse_local_mcp_arguments(arguments.as_deref().unwrap_or("{}"))
        } else {
            None
        };
        let item_id = if is_computer_call {
            format!("cc_{}", call_id)
        } else if is_mcp_call {
            format!("mcp_{}", call_id)
        } else {
            format!("fc_{}", call_id)
        };
        output.push(ResponsesOutputItem {
            kind: if is_computer_call {
                "computer_call".into()
            } else if is_mcp_call {
                "mcp_tool_call".into()
            } else {
                "function_call".into()
            },
            role: None,
            content: Vec::new(),
            id: Some(item_id),
            call_id: Some(call_id),
            name: if is_computer_call {
                None
            } else if let Some(mcp) = &mcp {
                Some(mcp.tool.clone())
            } else {
                Some(name.to_string())
            },
            server_label: mcp.as_ref().map(|mcp| mcp.server_label.clone()),
            arguments: if is_computer_call {
                None
            } else if let Some(mcp) = &mcp {
                Some(mcp.arguments.clone())
            } else {
                arguments.clone()
            },
            action: if is_computer_call {
                arguments
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .or_else(|| Some(json!({"type": "unknown"})))
            } else {
                None
            },
            status: Some("completed".into()),
            phase: None,
        });
    }

    let response = ResponsesResponse {
        id,
        object: "response",
        model: model.to_string(),
        output,
        usage: ResponsesUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        },
    };

    (response, vec![choice.message])
}

struct LocalMcpCall {
    server_label: String,
    tool: String,
    arguments: String,
}

fn parse_local_mcp_arguments(raw: &str) -> Option<LocalMcpCall> {
    let value = serde_json::from_str::<Value>(raw).ok()?;
    let server_label = value
        .get("server_label")
        .or_else(|| value.get("server_url"))
        .or_else(|| value.get("server"))
        .and_then(Value::as_str)
        .unwrap_or("remote_mcp")
        .to_string();
    let tool = value
        .get("tool")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let arguments = value
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}))
        .to_string();
    Some(LocalMcpCall {
        server_label,
        tool,
        arguments,
    })
}

fn response_output_item_id(prefix: &str, response_id: &str, index: usize) -> String {
    format!("{}_{}_{}", prefix, response_id, index)
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
                typ == "image_url"
                    || typ == "input_image"
                    || p.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.contains("data:image/"))
                        .unwrap_or(false)
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
            background: None,
            conversation: None,
            include: None,
            include_obfuscation: None,
            max_tool_calls: None,
            parallel_tool_calls: None,
            prompt: None,
            prompt_cache_key: None,
            prompt_cache_retention: None,
            safety_identifier: None,
            service_tier: None,
            stream_options: None,
            text: None,
            top_logprobs: None,
            user: None,
        }
    }

    #[test]
    fn test_text_input() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Text("hello".into()));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].role, "user");
        assert_eq!(
            chat.messages[0].content.as_ref().and_then(|v| v.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn test_system_from_instructions() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("hi".into()));
        req.instructions = Some("be helpful".into());
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages[0].role, "system");
        assert_eq!(
            chat.messages[0].content.as_ref().and_then(|v| v.as_str()),
            Some("be helpful")
        );
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
        assert_eq!(
            chat.messages[0].content.as_ref().and_then(|v| v.as_str()),
            Some("")
        );
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
        assert_eq!(eff, None);
        assert_eq!(think, Some(json!({"type": "disabled"})));
    }

    #[test]
    fn test_scan_history_fallback() {
        let sessions = SessionStore::new();
        let prev = ChatMessage {
            role: "assistant".into(),
            content: None,
            reasoning_content: Some("prior_reason".into()),
            tool_calls: Some(vec![
                json!({"id": "tc_x", "type": "function", "function": {"name": "f", "arguments": "{}"}}),
            ]),
            tool_call_id: None,
            name: None,
        };
        let history = vec![
            ChatMessage {
                role: "user".into(),
                content: Some("q".into()),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
            prev,
        ];
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "tc_x", "name": "f", "arguments": "{}"}),
            json!({"type": "function_call_output", "call_id": "tc_x", "output": "result"}),
        ]));
        let chat = to_chat_request(&req, history, &sessions, &empty_map(), false).chat;
        let fc_msg = chat
            .messages
            .iter()
            .find(|m| m.role == "assistant" && m.tool_calls.is_some());
        assert!(fc_msg.is_some());
        assert_eq!(
            fc_msg.unwrap().reasoning_content.as_deref(),
            Some("prior_reason")
        );
    }

    #[test]
    fn test_mcp_output_top_level_screenshot_stripped() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![
            json!({"type": "function_call", "call_id": "call_mcp", "name": "mcp__test", "arguments": "{}"}),
            json!({
                "type": "mcp_tool_call_output",
                "call_id": "call_mcp",
                "screenshot": "data:image/png;base64,xyz",
                "output": {"status": "ok"}
            }),
        ]));

        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[0].role, "assistant");
        assert_eq!(chat.messages[1].role, "tool");
        let content = chat.messages[1].content.as_ref().unwrap().as_str().unwrap();

        assert!(!content.contains("data:image/"));
        assert!(content.contains("[image omitted: image/png base64 3B]"));
        assert!(content.contains("ok"));
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
        assert_eq!(
            chat.messages[0].content.as_ref().and_then(|v| v.as_str()),
            Some("next")
        );
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
        assert_eq!(
            chat.messages[0].content.as_ref().and_then(|v| v.as_str()),
            Some("intervening")
        );
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
    fn test_stream_options_omitted_for_blocking() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Text("hi".into()));
        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert!(chat.stream_options.is_none());
    }

    #[test]
    fn test_blocking_tool_calls_are_returned() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(vec![json!({
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "exec_command",
                            "arguments": "{\"cmd\":\"pwd\"}"
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_1".into(), "deepseek-v4-pro", chat_resp);
        assert_eq!(resp.output.len(), 1);
        assert_eq!(resp.output[0].kind, "function_call");
        assert_eq!(resp.output[0].id.as_deref(), Some("fc_call_123"));
        assert_eq!(resp.output[0].call_id.as_deref(), Some("call_123"));
        assert_eq!(resp.output[0].name.as_deref(), Some("exec_command"));
        assert_eq!(
            resp.output[0].arguments.as_deref(),
            Some("{\"cmd\":\"pwd\"}")
        );
    }

    #[test]
    fn test_blocking_local_mcp_call_is_returned_as_mcp_tool_call() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(vec![json!({
                        "id": "call_mcp",
                        "type": "function",
                        "function": {
                            "name": "local_mcp_call",
                            "arguments": "{\"server_label\":\"filesystem\",\"tool\":\"read_file\",\"arguments\":{\"path\":\"README.md\"}}"
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_mcp".into(), "deepseek-v4-pro", chat_resp);

        assert_eq!(resp.output.len(), 1);
        assert_eq!(resp.output[0].kind, "mcp_tool_call");
        assert_eq!(resp.output[0].id.as_deref(), Some("mcp_call_mcp"));
        assert_eq!(resp.output[0].call_id.as_deref(), Some("call_mcp"));
        assert_eq!(resp.output[0].server_label.as_deref(), Some("filesystem"));
        assert_eq!(resp.output[0].name.as_deref(), Some("read_file"));
        assert_eq!(
            resp.output[0].arguments.as_deref(),
            Some("{\"path\":\"README.md\"}")
        );
    }

    #[test]
    fn test_blocking_response_output_item_ids() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: Some(Value::String("done".into())),
                    reasoning_content: Some("thinking".into()),
                    tool_calls: Some(vec![
                        json!({
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "exec_command",
                                "arguments": "{\"cmd\":\"pwd\"}"
                            }
                        }),
                        json!({
                            "id": "call_screen",
                            "type": "function",
                            "function": {
                                "name": "local_computer",
                                "arguments": "{\"type\":\"screenshot\"}"
                            }
                        }),
                        json!({
                            "type": "function",
                            "function": {
                                "name": "missing_id",
                                "arguments": "{}"
                            }
                        }),
                    ]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_42".into(), "deepseek-v4-pro", chat_resp);

        assert_eq!(resp.output[0].kind, "reasoning");
        assert_eq!(resp.output[0].id.as_deref(), Some("rs_resp_42_0"));
        assert_eq!(resp.output[1].kind, "message");
        assert_eq!(resp.output[1].id.as_deref(), Some("msg_resp_42_1"));
        assert_eq!(resp.output[2].kind, "function_call");
        assert_eq!(resp.output[2].id.as_deref(), Some("fc_call_abc"));
        assert_eq!(resp.output[2].call_id.as_deref(), Some("call_abc"));
        assert_eq!(resp.output[3].kind, "computer_call");
        assert_eq!(resp.output[3].id.as_deref(), Some("cc_call_screen"));
        assert_eq!(resp.output[3].call_id.as_deref(), Some("call_screen"));
        assert_eq!(resp.output[4].kind, "function_call");
        assert_eq!(resp.output[4].id.as_deref(), Some("fc_resp_42_4"));
        assert_eq!(resp.output[4].call_id.as_deref(), Some("resp_42_4"));
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
            completion_tokens_details: Some(TokenDetails {
                reasoning_tokens: Some(30),
            }),
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        };
        let s = format_usage(Some(&usage));
        assert!(s.contains("reason=30"));
    }

    // ── convert_tool tests ──

    #[test]
    fn test_convert_tool_non_object() {
        let result = convert_tool(&json!("string_value"));
        assert_eq!(result, json!("string_value"));

        let result = convert_tool(&json!(42));
        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_convert_tool_already_function_format() {
        let tool = json!({
            "type": "function",
            "function": {
                "name": "my_func",
                "parameters": {"type": "object"}
            },
            "extra_field": "preserved"
        });
        let result = convert_tool(&tool);
        assert_eq!(result, tool);
    }

    #[test]
    fn test_convert_tool_function_type() {
        let tool = json!({
            "type": "function",
            "name": "my_func",
            "description": "does something",
            "parameters": {"type": "object", "properties": {}},
            "strict": true
        });
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "my_func");
        assert_eq!(result["function"]["description"], "does something");
        assert_eq!(result["function"]["strict"], true);
        assert_eq!(result["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn test_convert_tool_function_minimal() {
        let tool = json!({"type": "function", "name": "minimal"});
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "minimal");
        assert!(result["function"].get("description").is_none());
        assert!(result["function"].get("parameters").is_none());
    }

    #[test]
    fn test_convert_tool_namespace() {
        let tool = json!({
            "type": "namespace",
            "name": "fs__",
            "description": "File system tools",
            "tools": [
                {"type": "function", "name": "read", "description": "read file"},
                {"type": "function", "name": "write", "description": "write file"}
            ]
        });
        let result = convert_tool(&tool);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["function"]["name"], "fs__read");
        assert_eq!(arr[1]["function"]["name"], "fs__write");
    }

    #[test]
    fn test_convert_tool_namespace_no_suffix_strip() {
        let tool = json!({
            "type": "namespace",
            "name": "mcp_tools",
            "tools": [
                {"type": "function", "name": "list"}
            ]
        });
        let result = convert_tool(&tool);
        let arr = result.as_array().unwrap();
        assert_eq!(arr[0]["function"]["name"], "mcp_tools__list");
    }

    #[test]
    fn test_convert_tool_namespace_empty_tools() {
        let tool = json!({
            "type": "namespace",
            "name": "empty_ns",
            "description": "no tools available",
            "tools": []
        });
        let result = convert_tool(&tool);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "function");
        assert_eq!(arr[0]["function"]["name"], "empty_ns");
        assert_eq!(arr[0]["function"]["description"], "no tools available");
    }

    #[test]
    fn test_convert_tool_namespace_no_tools_key() {
        let tool = json!({
            "type": "namespace",
            "name": "bare_ns"
        });
        let result = convert_tool(&tool);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["function"]["name"], "bare_ns");
    }

    #[test]
    fn test_convert_tool_custom_apply_patch() {
        let tool = json!({
            "type": "custom",
            "name": "apply_patch",
            "description": "apply a patch"
        });
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "apply_patch");
        // Should be mapped to exec_command-compatible schema with cmd required
        let params = &result["function"]["parameters"];
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["cmd"].is_object());
        assert_eq!(params["required"][0], "cmd");
        // Has exec_command-style fields
        assert!(params["properties"].get("workdir").is_some());
        assert!(params["properties"].get("shell").is_some());
    }

    #[test]
    fn test_convert_tool_custom_generic() {
        let tool = json!({
            "type": "custom",
            "name": "my_api",
            "description": "call my api"
        });
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "my_api");
        let params = &result["function"]["parameters"];
        assert_eq!(params["properties"]["input"]["type"], "string");
        assert_eq!(
            params["properties"]["input"]["description"],
            "Input for my_api"
        );
    }

    #[test]
    fn test_convert_tool_custom_no_name() {
        let tool = json!({"type": "custom"});
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "custom_tool");
    }

    #[test]
    fn test_convert_tool_local_shell() {
        let tool = json!({
            "type": "local_shell",
            "name": "my_shell",
            "description": "run a shell command",
            "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}
        });
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "my_shell");
        assert_eq!(result["function"]["description"], "run a shell command");
        assert_eq!(
            result["function"]["parameters"]["properties"]["cmd"]["type"],
            "string"
        );
    }

    #[test]
    fn test_convert_tool_local_shell_default_params() {
        let tool = json!({"type": "local_shell", "name": "bare_shell"});
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "bare_shell");
        assert_eq!(result["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn test_convert_tool_computer_use() {
        let tool = json!({
            "type": "computer_use",
            "display_width": 1920,
            "display_height": 1080
        });
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "local_computer");
        assert_eq!(result["function"]["parameters"]["required"][0], "type");
        assert_eq!(
            result["function"]["parameters"]["properties"]["type"]["enum"][0],
            "screenshot"
        );
        assert_eq!(
            result["function"]["description"],
            "Bridge for Responses computer_use. Request one local browser/computer action for a 1920x1080 display and wait for a computer_call_output screenshot before continuing."
        );
    }

    #[test]
    fn test_convert_tool_computer_use_default_dimensions() {
        let tool = json!({"type": "computer_use"});
        let result = convert_tool(&tool);
        assert_eq!(
            result["function"]["description"],
            "Bridge for Responses computer_use. Request one local browser/computer action for a 1024x768 display and wait for a computer_call_output screenshot before continuing."
        );
    }

    #[test]
    fn test_convert_tool_computer_use_preview() {
        let tool =
            json!({"type": "computer_use_preview", "display_width": 800, "display_height": 600});
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "local_computer");
        assert!(result["function"]["description"]
            .as_str()
            .unwrap()
            .contains("800x600"));
    }

    #[test]
    fn test_convert_tool_mcp() {
        let tool = json!({
            "type": "mcp",
            "server_label": "my-server"
        });
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "local_mcp_call");
        assert!(result["function"]["description"]
            .as_str()
            .unwrap()
            .contains("my-server"));
        assert_eq!(
            result["function"]["parameters"]["required"],
            json!(["tool", "arguments"])
        );
    }

    #[test]
    fn test_convert_tool_remote_mcp() {
        let tool = json!({
            "type": "remote_mcp",
            "server_url": "https://mcp.example.com"
        });
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "local_mcp_call");
        assert!(result["function"]["description"]
            .as_str()
            .unwrap()
            .contains("https://mcp.example.com"));
    }

    #[test]
    fn test_convert_tool_mcp_no_label() {
        let tool = json!({"type": "mcp"});
        let result = convert_tool(&tool);
        assert!(result["function"]["description"]
            .as_str()
            .unwrap()
            .contains("remote_mcp"));
    }

    #[test]
    fn test_convert_tool_unknown_type() {
        let tool = json!({
            "type": "weird_tool",
            "name": "w",
            "description": "a weird tool",
            "parameters": {"type": "object"}
        });
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        assert_eq!(result["function"]["name"], "w");
        assert_eq!(result["function"]["description"], "a weird tool");
        assert_eq!(result["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn test_convert_tool_unknown_type_fallback_name() {
        let tool = json!({"type": "mystery"});
        let result = convert_tool(&tool);
        assert_eq!(result["function"]["name"], "mystery");
        assert_eq!(result["function"]["description"], "Codex tool: mystery");
    }

    #[test]
    fn test_convert_tool_missing_type() {
        let tool = json!({"name": "orphan"});
        let result = convert_tool(&tool);
        assert_eq!(result["type"], "function");
        // typ is "" so fallback name uses that
        assert_eq!(result["function"]["name"], "orphan");
    }

    #[test]
    fn test_convert_tool_namespace_subtool_no_function_skipped() {
        let tool = json!({
            "type": "namespace",
            "name": "ns",
            "tools": [
                42
            ]
        });
        let result = convert_tool(&tool);
        let arr = result.as_array().unwrap();
        // 42 is not an object → convert_tool returns it unchanged (no "function" key)
        // so the if-let check fails and it's skipped → empty → placeholder
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["function"]["name"], "ns");
    }
}
