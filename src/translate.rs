use std::collections::HashSet;

use serde_json::{json, Value};

use tracing::info;

use crate::{session::SessionStore, types::*, utils::normalize_apply_patch_input};

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
    let host_only_tools = collect_host_only_tools(&req.tools);

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

    let mut system_text = if chinese_thinking {
        let raw = req.instructions.as_ref().or(req.system.as_ref());
        match raw {
            Some(s) => Some(format!("{}\n\n{}", cn_system, s)),
            None => Some(cn_system.to_string()),
        }
    } else {
        req.instructions.as_ref().or(req.system.as_ref()).cloned()
    };
    if !host_only_tools.is_empty() {
        let notice = host_only_tool_notice(&host_only_tools);
        system_text = Some(match system_text {
            Some(existing) => format!("{existing}\n\n{notice}"),
            None => notice,
        });
        info!(
            count = host_only_tools.len(),
            tools = host_only_tools.join(","),
            "filtered host-only Codex tools before upstream translation"
        );
    }
    if let Some(system) = system_text {
        let system_msg = ChatMessage {
            role: "system".into(),
            content: Some(Value::String(system)),
            reasoning_content: None,
            reasoning_details: None,
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
                reasoning_details: None,
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
                    let mut reasoning_details: Option<Value> = None;
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
                        if reasoning_details.is_none() {
                            reasoning_details = sessions.get_reasoning_details(call_id);
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
                        reasoning_details,
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
                    if msg.reasoning_details.is_none() {
                        msg.reasoning_details =
                            sessions.get_turn_reasoning_details(&messages, &msg);
                    }
                    if msg.reasoning_details.is_none() {
                        msg.reasoning_details =
                            sessions.scan_history_reasoning_details(&messages, &call_ids);
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
                                        reasoning_details: None,
                                        tool_calls: Some(vec![json!({
                                            "id": call_id,
                                            "type": "function",
                                            "function": {
                                                "name": synthetic_tool_name_for_output(item_type),
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
                                reasoning_details: None,
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
                                reasoning_details: None,
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

    merge_system_messages(&mut messages);

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
    // apply_patch 保持为独立函数工具，便于 Codex 识别代码修改并展示动态 diff。
    let tools: Vec<Value> = req
        .tools
        .iter()
        .filter(|t| {
            let typ = t.get("type").and_then(Value::as_str).unwrap_or("");
            !matches!(
                typ,
                "web_search" | "web_search_preview" | "file_search" | "file_search_preview"
            ) && !is_host_only_tool(t)
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

    // Detect web_search tool and attach a provider-specific web marker.
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
            reasoning_split: None,
            tool_choice: convert_tool_choice(req.tool_choice.as_ref()),
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

/// 将多余的系统消息合并到第一条系统消息中。
/// MiniMax 等严格 API 拒绝包含多条 system 消息的请求。
fn merge_system_messages(messages: &mut Vec<ChatMessage>) {
    let first_sys = match messages.iter().position(|m| m.role == "system") {
        Some(idx) => idx,
        None => return,
    };
    for i in (first_sys + 1..messages.len()).rev() {
        if messages[i].role == "system" {
            let removed = messages.remove(i);
            if let Some(Value::String(s)) = removed.content {
                if let Some(Value::String(ref mut target)) = messages[first_sys].content {
                    target.push_str("\n\n");
                    target.push_str(&s);
                }
            }
        }
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

fn collect_host_only_tools(tools: &[Value]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut labels = Vec::new();
    for tool in tools {
        if let Some(label) = host_only_tool_label(tool) {
            if seen.insert(label.clone()) {
                labels.push(label);
            }
        }
    }
    labels
}

fn host_only_tool_notice(labels: &[String]) -> String {
    let list = labels.join(", ");
    format!(
        "【deecodex 能力边界】本代理已过滤以下 Codex 宿主侧操作工具，未转发给上游模型：{list}。这些桌面/浏览器操作和客户端插件能力只能由 Codex 宿主原生通道执行。遇到相关请求时，请明确说明当前上游代理不能直接执行这些操作，不要伪造工具调用结果。"
    )
}

fn host_only_tool_label(tool: &Value) -> Option<String> {
    let obj = tool.as_object()?;
    let typ = obj.get("type").and_then(Value::as_str).unwrap_or("");
    if matches!(
        typ,
        "computer_use" | "computer_use_preview" | "browser_use" | "browser"
    ) {
        return Some(typ.to_string());
    }

    let name = obj
        .get("name")
        .or_else(|| obj.get("namespace"))
        .or_else(|| obj.get("server_label"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if host_only_tool_name(name) {
        return Some(if name.is_empty() {
            typ.to_string()
        } else {
            name.to_string()
        });
    }

    if let Some(function_name) = obj
        .get("function")
        .and_then(|f| f.get("name"))
        .and_then(Value::as_str)
    {
        if host_only_tool_name(function_name) {
            return Some(function_name.to_string());
        }
    }

    None
}

fn is_host_only_tool(tool: &Value) -> bool {
    host_only_tool_label(tool).is_some()
}

fn host_only_tool_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("computer_use")
        || lower.contains("computer-use")
        || lower.contains("browser_use")
        || lower.contains("browser-use")
        || lower == "browser"
        || lower.starts_with("browser.")
        || lower.starts_with("browser__")
        || lower == "chrome"
        || lower.starts_with("chrome.")
        || lower.starts_with("chrome__")
}

fn is_tool_search_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "tool_search"
        || lower == "tool_search_tool"
        || lower == "tool_search.tool_search_tool"
        || lower == "tool_search__tool_search_tool"
        || lower == "functions.tool_search"
        || lower == "functions__tool_search"
}

fn normalize_tool_search_name(name: &str) -> &str {
    if is_tool_search_name(name) {
        "tool_search"
    } else {
        name
    }
}

fn split_namespace_tool_name(name: &str) -> (Option<String>, String) {
    if let Some((namespace, tool_name)) = name.rsplit_once("__") {
        if !namespace.is_empty() && !tool_name.is_empty() {
            return (
                Some(
                    namespace
                        .strip_suffix("__")
                        .unwrap_or(namespace)
                        .to_string(),
                ),
                tool_name.to_string(),
            );
        }
    }
    (None, name.to_string())
}

fn convert_tool_search_tool(tool: &Value) -> Value {
    let obj = tool.as_object();
    let description = obj
        .and_then(|o| o.get("description"))
        .or_else(|| obj.and_then(|o| o.get("function")).and_then(|f| f.get("description")))
        .cloned()
        .unwrap_or_else(|| {
            json!(
                "搜索延迟暴露的 Codex 工具元数据，并在下一轮补齐匹配工具。用于插件/连接器/Computer Use 等能力发现。"
            )
        });
    let parameters = obj
        .and_then(|o| o.get("parameters"))
        .or_else(|| {
            obj.and_then(|o| o.get("function"))
                .and_then(|f| f.get("parameters"))
        })
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "要搜索的能力或工具关键词"
                    },
                    "limit": {
                        "type": "number",
                        "description": "最多返回的工具数量"
                    }
                },
                "required": ["query"]
            })
        });

    json!({
        "type": "function",
        "function": {
            "name": "tool_search",
            "description": description,
            "parameters": parameters
        }
    })
}

fn synthetic_tool_name_for_output(item_type: &str) -> &str {
    match item_type {
        "tool_search_output" => "tool_search",
        "computer_call_output" => "local_computer",
        "mcp_tool_call_output" => "local_mcp_call",
        other => other,
    }
}

/// Responses API flat tool → Chat Completions nested format.
/// Handles function, custom (apply_patch), and namespace (MCP) types.
pub fn convert_tool(tool: &Value) -> Value {
    let Some(obj) = tool.as_object() else {
        return tool.clone();
    };
    let typ = obj.get("type").and_then(Value::as_str).unwrap_or("");

    match typ {
        // Already in the right format — ensure required fields are present
        _ if obj.contains_key("function") => {
            if obj
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .is_some_and(is_tool_search_name)
            {
                return convert_tool_search_tool(tool);
            }
            let mut t = tool.clone();
            if let Some(func) = t.get_mut("function").and_then(|f| f.as_object_mut()) {
                if !func.contains_key("parameters") {
                    func.insert(
                        "parameters".into(),
                        json!({"type": "object", "properties": {}}),
                    );
                }
                let name_empty = !func.contains_key("name")
                    || func
                        .get("name")
                        .and_then(|v| v.as_str())
                        .is_none_or(|s| s.is_empty());
                if name_empty {
                    func.insert("name".into(), json!("unnamed_tool"));
                }
            }
            t
        }

        // Standard function type → nest under "function" key
        "function" => {
            if obj
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(is_tool_search_name)
            {
                return convert_tool_search_tool(tool);
            }
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
            if is_tool_search_name(name) {
                return convert_tool_search_tool(tool);
            }
            let mut expanded = Vec::new();
            if let Some(tools) = obj.get("tools").and_then(Value::as_array) {
                for sub_tool in tools {
                    if let Some(sub_name) = sub_tool
                        .get("name")
                        .or_else(|| sub_tool.get("function").and_then(|f| f.get("name")))
                        .and_then(Value::as_str)
                    {
                        if is_tool_search_name(sub_name) {
                            expanded.push(convert_tool_search_tool(sub_tool));
                            continue;
                        }
                    }
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

            // apply_patch 是 Codex 的代码修改工具。Chat 兼容模型只能通过 JSON
            // function calling 表达参数，因此固定使用 patch 字段承载补丁文本。
            let params = if name == "apply_patch" {
                json!({
                    "type": "object",
                    "properties": {
                        "patch": {
                            "type": "string",
                            "description": "Patch text to apply to the local workspace. Use the Codex apply_patch format beginning with *** Begin Patch and ending with *** End Patch."
                        }
                    },
                    "required": ["patch"],
                    "additionalProperties": false
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
                    "description": if name == "apply_patch" && desc.trim().is_empty() {
                        "Apply a source-code patch to the local workspace so Codex can show file edit diff statistics."
                    } else {
                        desc
                    },
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

        "tool_search" => convert_tool_search_tool(tool),

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

        // Unknown type → best-effort (e.g. image_generation, view_image)
        _ => {
            let mut func = serde_json::Map::new();
            func.insert(
                "name".into(),
                obj.get("name").cloned().unwrap_or(json!(typ)),
            );
            if let Some(v) = obj.get("description") {
                func.insert("description".into(), v.clone());
            } else {
                func.insert("description".into(), json!(format!("Codex tool: {}", typ)));
            }
            func.insert(
                "parameters".into(),
                obj.get("parameters")
                    .cloned()
                    .unwrap_or(json!({"type": "object", "properties": {}})),
            );
            json!({"type": "function", "function": func})
        }
    }
}

fn convert_tool_choice(choice: Option<&Value>) -> Option<Value> {
    let choice = choice?;
    if choice.is_string() {
        return Some(choice.clone());
    }
    let obj = choice.as_object()?;
    let typ = obj.get("type").and_then(Value::as_str).unwrap_or("");

    if let Some(name) = obj
        .get("function")
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
    {
        return Some(json!({
            "type": "function",
            "function": {"name": normalize_tool_search_name(name)}
        }));
    }

    match typ {
        "function" | "custom" => obj.get("name").and_then(Value::as_str).map(|name| {
            json!({
                "type": "function",
                "function": {"name": normalize_tool_search_name(name)}
            })
        }),
        "tool_search" => Some(json!({
            "type": "function",
            "function": {"name": "tool_search"}
        })),
        _ => Some(choice.clone()),
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
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            },
        });

    let raw_reasoning_content = choice.message.reasoning_content.clone().unwrap_or_default();
    let raw_text = chat_message_content_text(choice.message.content.as_ref());
    let (tagged_reasoning_content, text_out) = split_tagged_reasoning(&raw_text);
    let reasoning_content = if raw_reasoning_content.is_empty() {
        tagged_reasoning_content
    } else {
        raw_reasoning_content
    };
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
            namespace: None,
            server_label: None,
            arguments: None,
            input: None,
            action: None,
            status: Some("completed".into()),
            phase: None,
        });
    }

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
            namespace: None,
            server_label: None,
            arguments: None,
            input: None,
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
            .map(normalize_tool_search_name)
            .unwrap_or_default();
        let arguments = function.get("arguments").map(tool_arguments_string);
        let is_computer_call = name == "local_computer";
        let is_apply_patch = name == "apply_patch";
        let apply_patch_input = is_apply_patch
            .then(|| {
                arguments
                    .as_deref()
                    .map(apply_patch_input_from_arguments)
                    .unwrap_or_default()
            })
            .filter(|input| !input.is_empty());
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
        let (namespace, function_name) = if is_computer_call || is_mcp_call || is_apply_patch {
            (None, name.to_string())
        } else {
            split_namespace_tool_name(name)
        };
        let item_id = if is_computer_call {
            format!("cc_{}", call_id)
        } else if is_mcp_call {
            format!("mcp_{}", call_id)
        } else if is_apply_patch {
            format!("ctc_{}", call_id)
        } else {
            format!("fc_{}", call_id)
        };
        output.push(ResponsesOutputItem {
            kind: if is_computer_call {
                "computer_call".into()
            } else if is_mcp_call {
                "mcp_tool_call".into()
            } else if is_apply_patch {
                "custom_tool_call".into()
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
                Some(function_name)
            },
            namespace,
            server_label: mcp.as_ref().map(|mcp| mcp.server_label.clone()),
            arguments: if is_computer_call {
                None
            } else if let Some(mcp) = &mcp {
                Some(mcp.arguments.clone())
            } else if is_apply_patch {
                None
            } else {
                arguments.clone()
            },
            input: apply_patch_input,
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

fn tool_arguments_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn apply_patch_input_from_arguments(arguments: &str) -> String {
    let input = serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| {
            value
                .get("patch")
                .or_else(|| value.get("input"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| arguments.to_string());
    normalize_apply_patch_input(&input)
}

fn chat_message_content_text(content: Option<&Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    let mut chunks = Vec::new();
    collect_chat_content_text(content, &mut chunks);
    if chunks.is_empty() {
        if content.is_null() {
            String::new()
        } else {
            content.as_str().map(str::to_string).unwrap_or_default()
        }
    } else {
        chunks.join("")
    }
}

fn collect_chat_content_text(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::String(text) => chunks.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_chat_content_text(item, chunks);
            }
        }
        Value::Object(map) => {
            for key in ["text", "output_text", "input_text", "refusal"] {
                if let Some(text) = map.get(key).and_then(Value::as_str) {
                    chunks.push(text.to_string());
                    return;
                }
            }
            if let Some(content) = map.get("content") {
                collect_chat_content_text(content, chunks);
            }
        }
        _ => {}
    }
}

fn split_tagged_reasoning(text: &str) -> (String, String) {
    let mut reasoning = String::new();
    let mut visible = String::new();
    let mut rest = text;

    while let Some(start) = rest.find("<think>") {
        visible.push_str(&rest[..start]);
        rest = &rest[start + "<think>".len()..];
        match rest.find("</think>") {
            Some(end) => {
                reasoning.push_str(&rest[..end]);
                rest = &rest[end + "</think>".len()..];
            }
            None => {
                reasoning.push_str(rest);
                return (reasoning, visible);
            }
        }
    }

    visible.push_str(rest);
    (reasoning, visible)
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
            reasoning_details: None,
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
                reasoning_details: None,
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
                    reasoning_details: None,
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
    fn test_blocking_namespace_tool_call_keeps_namespace() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: Some(vec![json!({
                        "id": "call_app_state",
                        "type": "function",
                        "function": {
                            "name": "mcp__computer_use__get_app_state",
                            "arguments": "{\"app\":\"抖音\"}"
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_ns".into(), "deepseek-v4-pro", chat_resp);
        assert_eq!(resp.output.len(), 1);
        assert_eq!(resp.output[0].kind, "function_call");
        assert_eq!(
            resp.output[0].namespace.as_deref(),
            Some("mcp__computer_use")
        );
        assert_eq!(resp.output[0].name.as_deref(), Some("get_app_state"));
        assert_eq!(
            resp.output[0].arguments.as_deref(),
            Some("{\"app\":\"抖音\"}")
        );
    }

    #[test]
    fn test_blocking_apply_patch_call_keeps_patch_tool_name() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: Some(vec![json!({
                        "id": "patch_123",
                        "type": "function",
                        "function": {
                            "name": "apply_patch",
                            "arguments": "{\"patch\":\"*** Begin Patch\\n*** End Patch\"}"
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_patch".into(), "deepseek-v4-pro", chat_resp);

        assert_eq!(resp.output.len(), 1);
        assert_eq!(resp.output[0].kind, "custom_tool_call");
        assert_eq!(resp.output[0].id.as_deref(), Some("ctc_patch_123"));
        assert_eq!(resp.output[0].call_id.as_deref(), Some("patch_123"));
        assert_eq!(resp.output[0].name.as_deref(), Some("apply_patch"));
        assert_eq!(resp.output[0].arguments, None);
        assert_eq!(
            resp.output[0].input.as_deref(),
            Some("*** Begin Patch\n*** End Patch")
        );
    }

    #[test]
    fn test_blocking_apply_patch_call_normalizes_unified_diff() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: Some(vec![json!({
                        "id": "patch_123",
                        "type": "function",
                        "function": {
                            "name": "apply_patch",
                            "arguments": json!({
                                "patch": concat!(
                                    "*** Begin Patch\n",
                                    "--- a/tmp/codex-minimax-toolchain-test/file1.txt\n",
                                    "+++ b/tmp/codex-minimax-toolchain-test/file1.txt\n",
                                    "@@ -1 +1 @@\n",
                                    "-minimax test\n",
                                    "+PATCH_OK minimax test\n",
                                    "*** End Patch"
                                )
                            })
                            .to_string()
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_patch".into(), "MiniMax-M2.7", chat_resp);
        let input = resp.output[0].input.as_deref().unwrap();

        assert!(input.contains("*** Update File: /tmp/codex-minimax-toolchain-test/file1.txt"));
        assert!(!input.contains("--- a/tmp/codex-minimax-toolchain-test/file1.txt"));
        assert!(!input.contains("+++ b/tmp/codex-minimax-toolchain-test/file1.txt"));
    }

    #[test]
    fn test_blocking_think_tags_are_returned_as_reasoning() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: Some(Value::String("<think>先分析</think>最终答案".into())),
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_think".into(), "test", chat_resp);

        assert_eq!(resp.output.len(), 2);
        assert_eq!(resp.output[0].kind, "reasoning");
        assert_eq!(resp.output[0].content[0].text.as_deref(), Some("先分析"));
        assert_eq!(resp.output[1].kind, "message");
        assert_eq!(resp.output[1].content[0].text.as_deref(), Some("最终答案"));
    }

    #[test]
    fn test_blocking_tool_call_arguments_accept_object() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: Some(vec![json!({
                        "id": "call_obj",
                        "type": "function",
                        "function": {
                            "name": "read_file",
                            "arguments": {"path": "/tmp/a.txt"}
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_args".into(), "test", chat_resp);

        assert_eq!(resp.output[0].kind, "function_call");
        assert_eq!(
            resp.output[0].arguments.as_deref(),
            Some(r#"{"path":"/tmp/a.txt"}"#)
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
                    reasoning_details: None,
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
                    reasoning_details: None,
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
    fn test_host_only_tools_are_filtered_from_chat_tools() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("打开抖音搜索露营视频".into()));
        req.tools = vec![
            json!({"type": "computer_use", "display_width": 1024, "display_height": 768}),
            json!({
                "type": "namespace",
                "name": "mcp__computer-use__",
                "tools": [{"type": "function", "name": "get_app_state"}]
            }),
            json!({
                "type": "mcp",
                "server_label": "computer-use"
            }),
            json!({"type": "function", "name": "safe_tool", "parameters": {"type": "object"}}),
        ];

        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        let tool_names: Vec<_> = chat
            .tools
            .iter()
            .filter_map(|tool| {
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
            })
            .collect();
        assert_eq!(tool_names, vec!["safe_tool"]);
        assert_eq!(chat.messages[0].role, "system");
        assert!(chat.messages[0]
            .content
            .as_ref()
            .and_then(Value::as_str)
            .unwrap()
            .contains("computer-use"));
    }

    #[test]
    fn test_web_search_preview_becomes_provider_web_marker() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("搜索今天的新闻".into()));
        req.tools = vec![
            json!({"type": "web_search_preview"}),
            json!({"type": "function", "name": "safe_tool", "parameters": {"type": "object"}}),
        ];

        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;

        assert!(chat.web_search_options.is_some());
        assert!(!chat.tools.iter().any(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .is_some_and(|typ| typ == "web_search" || typ == "web_search_preview")
        }));
        assert_eq!(chat.tools.len(), 1);
        assert_eq!(chat.tools[0]["function"]["name"], "safe_tool");
    }

    #[test]
    fn test_computer_use_plugin_mention_does_not_inject_mcp_bridge_tool() {
        let sessions = SessionStore::new();
        let req = base_req(ResponsesInput::Messages(vec![json!({
            "role": "user",
            "content": [
                {"type": "input_text", "text": "电脑打开抖音 app 播放第一个视频"},
                {"type": "input_text", "text": "plugin://computer-use@openai-bundled"}
            ]
        })]));

        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        assert!(!chat
            .tools
            .iter()
            .any(|tool| tool["function"]["name"] == "local_mcp_call"));
        assert!(!chat.messages.iter().any(|message| {
            message.role == "system"
                && message
                    .content
                    .as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains("local_mcp_call"))
        }));
    }

    #[test]
    fn test_tool_search_function_alias_is_normalized() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("补齐电脑能力".into()));
        req.tools = vec![
            json!({
                "type": "function",
                "function": {
                    "name": "tool_search",
                    "parameters": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}},
                        "required": ["query"]
                    }
                }
            }),
            json!({
                "type": "tool_search",
                "parameters": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}
            }),
            json!({
                "type": "function",
                "name": "tool_search_tool",
                "parameters": {"type": "object"}
            }),
            json!({
                "type": "namespace",
                "name": "tool_search",
                "tools": [{"type": "function", "name": "tool_search_tool"}]
            }),
        ];

        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;
        let tool_names: Vec<_> = chat
            .tools
            .iter()
            .filter_map(|tool| {
                tool.get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
            })
            .collect();
        assert_eq!(tool_names, vec!["tool_search"]);
        assert_eq!(
            chat.tools[0]["function"]["parameters"]["required"],
            json!(["query"])
        );
        assert_eq!(
            chat.tools[0]["function"]["parameters"]["properties"]["limit"]["type"],
            "integer"
        );
    }

    #[test]
    fn test_tool_search_call_alias_is_returned_as_stable_name() {
        let chat_resp = ChatResponse {
            choices: vec![ChatChoice {
                message: ChatMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    reasoning_details: None,
                    tool_calls: Some(vec![json!({
                        "id": "call_ts",
                        "type": "function",
                        "function": {
                            "name": "tool_search__tool_search_tool",
                            "arguments": "{\"query\":\"computer use\"}"
                        }
                    })]),
                    tool_call_id: None,
                    name: None,
                },
            }],
            usage: None,
        };

        let (resp, _) = from_chat_response("resp_ts".into(), "test", chat_resp);
        assert_eq!(resp.output[0].kind, "function_call");
        assert_eq!(resp.output[0].name.as_deref(), Some("tool_search"));
        assert_eq!(
            resp.output[0].arguments.as_deref(),
            Some("{\"query\":\"computer use\"}")
        );
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
        let params = &result["function"]["parameters"];
        assert_eq!(params["type"], "object");
        assert!(params["properties"]["patch"].is_object());
        assert_eq!(params["required"][0], "patch");
        assert_eq!(params["additionalProperties"], false);
        assert!(params["properties"].get("cmd").is_none());
    }

    #[test]
    fn test_tool_choice_custom_apply_patch_converts_to_chat_function_choice() {
        let sessions = SessionStore::new();
        let mut req = base_req(ResponsesInput::Text("改文件".into()));
        req.tools = vec![json!({
            "type": "custom",
            "name": "apply_patch",
            "description": "apply a patch"
        })];
        req.tool_choice = Some(json!({"type": "custom", "name": "apply_patch"}));

        let chat = to_chat_request(&req, vec![], &sessions, &empty_map(), false).chat;

        assert_eq!(
            chat.tool_choice,
            Some(json!({"type":"function","function":{"name":"apply_patch"}}))
        );
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
