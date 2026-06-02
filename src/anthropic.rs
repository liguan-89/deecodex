use crate::types::{ChatMessage, ChatRequest, ChatResponse, ChatUsage};
use serde_json::{json, Value};

pub fn to_messages_body(chat_req: &ChatRequest, thinking_tokens: Option<u32>) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();

    for msg in &chat_req.messages {
        if msg.role == "system" {
            if let Some(text) = message_text(msg) {
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
            continue;
        }

        let role = match msg.role.as_str() {
            "assistant" => "assistant",
            "tool" => "user",
            _ => "user",
        };
        messages.push(json!({
            "role": role,
            "content": anthropic_content(msg),
        }));
    }

    if messages.is_empty() {
        messages.push(json!({"role": "user", "content": [{"type": "text", "text": ""}]}));
    }

    let mut body = json!({
        "model": chat_req.model,
        "messages": messages,
        "max_tokens": chat_req.max_tokens.unwrap_or(4096),
    });

    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    if let Some(temp) = chat_req.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(top_p) = chat_req.top_p {
        body["top_p"] = json!(top_p);
    }
    if let Some(budget) = thinking_tokens.or_else(|| thinking_budget(chat_req.thinking.as_ref())) {
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    }
    let tools = anthropic_tools(&chat_req.tools);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(choice) = anthropic_tool_choice(chat_req.tool_choice.as_ref()) {
        body["tool_choice"] = choice;
    }

    body
}

pub fn response_to_chat(value: Value) -> ChatResponse {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    if let Some(parts) = value.get("content").and_then(Value::as_array) {
        for part in parts {
            match part.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = part.get("text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("thinking") | Some("reasoning") => {
                    if let Some(t) = part
                        .get("thinking")
                        .or_else(|| part.get("text"))
                        .and_then(Value::as_str)
                    {
                        reasoning.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = part.get("id").and_then(Value::as_str).unwrap_or("");
                    let name = part.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = part.get("input").cloned().unwrap_or_else(|| json!({}));
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": input.to_string()
                        }
                    }));
                }
                _ => {}
            }
        }
    }

    let usage = value.get("usage").map(|usage| ChatUsage {
        prompt_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        completion_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        total_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
            + usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u32,
        completion_tokens_details: None,
        prompt_cache_hit_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        prompt_cache_miss_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        prompt_tokens_details: None,
    });

    ChatResponse {
        choices: vec![crate::types::ChatChoice {
            message: ChatMessage {
                role: "assistant".into(),
                content: Some(Value::String(text)),
                reasoning_content: if reasoning.is_empty() {
                    None
                } else {
                    Some(reasoning)
                },
                reasoning_details: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
                name: None,
            },
        }],
        usage,
    }
}

fn anthropic_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?;
            Some(json!({
                "name": name,
                "description": function
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                "input_schema": function
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}})),
            }))
        })
        .collect()
}

fn anthropic_tool_choice(choice: Option<&Value>) -> Option<Value> {
    let choice = choice?;
    if let Some(s) = choice.as_str() {
        return match s {
            "auto" => Some(json!({"type":"auto"})),
            "required" => Some(json!({"type":"any"})),
            "none" => None,
            _ => None,
        };
    }
    let typ = choice.get("type").and_then(Value::as_str).unwrap_or("");
    if typ == "function" {
        let name = choice
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)?;
        return Some(json!({"type":"tool", "name": name}));
    }
    None
}

fn anthropic_content(msg: &ChatMessage) -> Value {
    if msg.role == "tool" {
        return json!([{
            "type": "tool_result",
            "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
            "content": message_text(msg).unwrap_or_default(),
        }]);
    }

    if msg.role == "assistant" {
        let mut parts = Vec::new();
        if let Some(text) = message_text(msg) {
            if !text.is_empty() {
                parts.push(json!({"type": "text", "text": text}));
            }
        }
        if let Some(tool_calls) = &msg.tool_calls {
            for call in tool_calls {
                let id = call.get("id").and_then(Value::as_str).unwrap_or("");
                let function = call.get("function").unwrap_or(&Value::Null);
                let name = function.get("name").and_then(Value::as_str).unwrap_or("");
                let arguments = function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .unwrap_or_else(|| json!({}));
                parts.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": arguments,
                }));
            }
        }
        if !parts.is_empty() {
            return Value::Array(parts);
        }
    }

    match msg.content.as_ref() {
        Some(Value::Array(parts)) => Value::Array(parts.iter().filter_map(convert_part).collect()),
        Some(Value::String(s)) => json!([{"type": "text", "text": s}]),
        Some(other) => json!([{"type": "text", "text": other.to_string()}]),
        None => json!([{"type": "text", "text": ""}]),
    }
}

fn convert_part(part: &Value) -> Option<Value> {
    match part.get("type").and_then(Value::as_str) {
        Some("text") => Some(json!({
            "type": "text",
            "text": part.get("text").and_then(Value::as_str).unwrap_or("")
        })),
        Some("image_url") | Some("input_image") => {
            let url = part
                .get("image_url")
                .and_then(|u| u.get("url"))
                .or_else(|| part.get("image_url"))
                .and_then(Value::as_str)?;
            data_url_to_anthropic_image(url)
        }
        _ => None,
    }
}

fn data_url_to_anthropic_image(url: &str) -> Option<Value> {
    let rest = url.strip_prefix("data:")?;
    let (media_type, data) = rest.split_once(";base64,")?;
    Some(json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": media_type,
            "data": data,
        }
    }))
}

fn message_text(msg: &ChatMessage) -> Option<String> {
    match msg.content.as_ref()? {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|p| {
                    if p.get("type").and_then(Value::as_str) == Some("text") {
                        p.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(""),
        ),
        other => Some(other.to_string()),
    }
}

fn thinking_budget(thinking: Option<&Value>) -> Option<u32> {
    let thinking = thinking?;
    if thinking.get("type").and_then(Value::as_str) != Some("enabled") {
        return None;
    }
    thinking
        .get("budget_tokens")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: Value) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: Some(content),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    #[test]
    fn builds_anthropic_messages_body_with_system_and_image() {
        let chat = ChatRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![
                msg("system", Value::String("You are concise".into())),
                msg(
                    "user",
                    json!([
                        {"type":"text", "text":"describe"},
                        {"type":"image_url", "image_url":{"url":"data:image/png;base64,AAAA"}}
                    ]),
                ),
            ],
            tools: vec![],
            temperature: Some(0.2),
            top_p: None,
            max_tokens: Some(1000),
            stream: false,
            reasoning_effort: None,
            thinking: Some(json!({"type":"enabled", "budget_tokens": 2048})),
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        let body = to_messages_body(&chat, None);
        assert_eq!(body["model"], "claude-sonnet-4-5");
        assert_eq!(body["system"], "You are concise");
        assert_eq!(body["max_tokens"], 1000);
        assert_eq!(body["thinking"]["budget_tokens"], 2048);
        assert_eq!(
            body["messages"][0]["content"][1]["source"]["media_type"],
            "image/png"
        );
    }

    #[test]
    fn converts_anthropic_response_to_chat_shape() {
        let chat = response_to_chat(json!({
            "content": [
                {"type":"thinking", "thinking":"reason"},
                {"type":"text", "text":"answer"},
                {"type":"tool_use", "id":"toolu_1", "name":"read_file", "input":{"path":"a.txt"}}
            ],
            "usage": {"input_tokens": 3, "output_tokens": 4}
        }));

        assert_eq!(chat.choices[0].message.content.as_ref().unwrap(), "answer");
        assert_eq!(
            chat.choices[0].message.reasoning_content.as_deref(),
            Some("reason")
        );
        assert_eq!(
            chat.choices[0].message.tool_calls.as_ref().unwrap()[0]["function"]["name"],
            "read_file"
        );
        assert_eq!(chat.usage.unwrap().total_tokens, 7);
    }

    #[test]
    fn maps_openai_tools_to_anthropic_tools() {
        let chat = ChatRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![msg("user", Value::String("hi".into()))],
            tools: vec![json!({
                "type":"function",
                "function": {"name":"read_file", "description":"Read", "parameters":{"type":"object"}}
            })],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: false,
            reasoning_effort: None,
            thinking: None,
            tool_choice: Some(json!({"type":"function", "function":{"name":"read_file"}})),
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        let body = to_messages_body(&chat, None);
        assert_eq!(body["tools"][0]["name"], "read_file");
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "read_file");
    }

    #[test]
    fn maps_tool_history_to_anthropic_tool_use_and_result() {
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: Some(Value::String("checking".into())),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: Some(vec![json!({
                "id":"call_1",
                "type":"function",
                "function":{"name":"read_file", "arguments":"{\"path\":\"a.txt\"}"}
            })]),
            tool_call_id: None,
            name: None,
        };
        let tool = ChatMessage {
            role: "tool".into(),
            content: Some(Value::String("file text".into())),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
            name: None,
        };
        let chat = ChatRequest {
            model: "claude-sonnet-4-5".into(),
            messages: vec![assistant, tool],
            tools: vec![],
            temperature: None,
            top_p: None,
            max_tokens: None,
            stream: false,
            reasoning_effort: None,
            thinking: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        };

        let body = to_messages_body(&chat, None);
        assert_eq!(body["messages"][0]["content"][1]["type"], "tool_use");
        assert_eq!(body["messages"][0]["content"][1]["input"]["path"], "a.txt");
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][1]["content"][0]["tool_use_id"], "call_1");
    }
}
