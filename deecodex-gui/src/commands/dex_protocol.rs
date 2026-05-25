use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};
use tauri::State;

use crate::ServerManager;

pub(super) type DexAccountInfo = (
    String,
    String,
    HashMap<String, String>,
    String,
    deecodex::providers::ProviderProfile,
    deecodex::accounts::EndpointKind,
    String,
);

pub(super) fn get_active_account_info(data_dir: &Path) -> Option<DexAccountInfo> {
    let store = deecodex::accounts::load_accounts(data_dir);
    let mut active = store
        .active_id
        .as_ref()
        .and_then(|id| store.accounts.iter().find(|a| &a.id == id))
        .cloned()?;
    active.normalize_v2();
    let endpoint = active
        .active_endpoint(store.active_endpoint_id.as_deref())
        .cloned()
        .or_else(|| active.endpoints.first().cloned())?;
    active.sync_legacy_from_endpoint(&endpoint);
    let mut profile = deecodex::providers::profile_for_account(&active);
    profile.wire_protocol = dex_wire_protocol_for_endpoint(&endpoint.kind);

    Some((
        active.upstream.clone(),
        active.api_key.clone(),
        active.model_map.clone(),
        active.provider.clone(),
        profile,
        endpoint.kind.clone(),
        endpoint.effective_path().to_string(),
    ))
}

fn dex_wire_protocol_for_endpoint(
    kind: &deecodex::accounts::EndpointKind,
) -> deecodex::providers::WireProtocol {
    match kind {
        deecodex::accounts::EndpointKind::OpenAiChat
        | deecodex::accounts::EndpointKind::CustomChat => {
            deecodex::providers::WireProtocol::ChatCompletions
        }
        deecodex::accounts::EndpointKind::OpenAiResponses
        | deecodex::accounts::EndpointKind::CustomResponses
        | deecodex::accounts::EndpointKind::CodexOfficial => {
            deecodex::providers::WireProtocol::Responses
        }
        deecodex::accounts::EndpointKind::AnthropicMessages => {
            deecodex::providers::WireProtocol::AnthropicMessages
        }
    }
}

pub(super) async fn dex_responses_request_target(
    manager: &State<'_, ServerManager>,
    endpoint_kind: &deecodex::accounts::EndpointKind,
    upstream: &str,
    endpoint_path: &str,
    chat_req: &deecodex::types::ChatRequest,
) -> Result<(String, Value, bool), String> {
    if chat_req.stream {
        return Err("DEX 助手暂不支持 Responses 端点流式请求，已降级为非流式重试".into());
    }
    let body = dex_chat_request_to_responses_body(chat_req);
    if matches!(
        endpoint_kind,
        deecodex::accounts::EndpointKind::CodexOfficial
    ) {
        let host = manager.host.lock().await.clone();
        let port = *manager.port.lock().await;
        let url_host = deecodex::config::client_url_host(&host);
        return Ok((
            format!("http://{url_host}:{port}/v1/responses"),
            body,
            false,
        ));
    }
    let path = if endpoint_path.trim().is_empty() {
        "responses"
    } else {
        endpoint_path.trim_start_matches('/')
    };
    Ok((
        format!("{}/{}", upstream.trim_end_matches('/'), path),
        body,
        true,
    ))
}

fn dex_chat_request_to_responses_body(req: &deecodex::types::ChatRequest) -> Value {
    let mut input = Vec::new();
    for message in &req.messages {
        if message.role == "tool" {
            input.push(json!({
                "type": "function_call_output",
                "call_id": message.tool_call_id.clone().unwrap_or_default(),
                "output": dex_message_content_text(message.content.as_ref()),
            }));
            continue;
        }

        let role = if message.role == "system" {
            "developer"
        } else {
            message.role.as_str()
        };
        let content = dex_responses_content_parts(message.content.as_ref(), role);
        if !content.is_empty() {
            input.push(json!({
                "type": "message",
                "role": role,
                "content": content,
            }));
        }
        if message.role == "assistant" {
            if let Some(tool_calls) = message.tool_calls.as_ref() {
                for call in tool_calls {
                    if call.get("type").and_then(Value::as_str) != Some("function") {
                        continue;
                    }
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(),
                        "arguments": call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or_default(),
                    }));
                }
            }
        }
    }

    let mut body = json!({
        "model": req.model.clone(),
        "instructions": "",
        "input": input,
        "stream": false,
        "store": false,
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
    });
    let tools = dex_chat_tools_to_responses_tools(&req.tools);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(reasoning) = req.reasoning_effort.as_deref() {
        body["reasoning"] = json!({"effort": reasoning, "summary": "auto"});
    }
    body
}

fn dex_chat_tools_to_responses_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                return Some(tool.clone());
            }
            let function = tool.get("function")?;
            let mut converted = json!({
                "type": "function",
                "name": function.get("name").and_then(Value::as_str).unwrap_or_default(),
            });
            if let Some(description) = function.get("description") {
                converted["description"] = description.clone();
            }
            if let Some(parameters) = function.get("parameters") {
                converted["parameters"] = parameters.clone();
            }
            Some(converted)
        })
        .collect()
}

fn dex_responses_content_parts(content: Option<&Value>, role: &str) -> Vec<Value> {
    let part_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({"type": part_type, "text": text})]
        }
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    return Some(json!({"type": part_type, "text": text}));
                }
                if item.get("type").and_then(Value::as_str) == Some("image_url") {
                    if let Some(url) = item.pointer("/image_url/url").and_then(Value::as_str) {
                        return Some(json!({"type": "input_image", "image_url": url}));
                    }
                }
                None
            })
            .collect(),
        Some(other) if !other.is_null() => {
            vec![json!({"type": part_type, "text": other.to_string()})]
        }
        _ => Vec::new(),
    }
}

fn dex_message_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    }
}

pub(super) fn dex_responses_to_chat_value(value: Value) -> Value {
    if value.get("choices").is_some() {
        return value;
    }
    let mut content_parts = Vec::new();
    let mut tool_calls = Vec::new();
    if let Some(output) = value.get("output").and_then(Value::as_array) {
        for item in output {
            match item.get("type").and_then(Value::as_str).unwrap_or_default() {
                "message" => {
                    if let Some(parts) = item.get("content").and_then(Value::as_array) {
                        for part in parts {
                            if let Some(text) = part
                                .get("text")
                                .or_else(|| part.get("output_text"))
                                .and_then(Value::as_str)
                            {
                                content_parts.push(text.to_string());
                            }
                        }
                    }
                }
                "function_call" => {
                    tool_calls.push(json!({
                        "id": item.get("call_id").or_else(|| item.get("id")).and_then(Value::as_str).unwrap_or_default(),
                        "type": "function",
                        "function": {
                            "name": item.get("name").and_then(Value::as_str).unwrap_or_default(),
                            "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or_default(),
                        }
                    }));
                }
                _ => {}
            }
        }
    }
    let content = content_parts.join("");
    let mut message = json!({
        "role": "assistant",
        "content": if content.is_empty() { Value::Null } else { Value::String(content) },
    });
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    json!({
        "choices": [{
            "message": message,
            "finish_reason": "stop",
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dex_chat_request_to_responses_body_maps_messages_and_tools() {
        let req = deecodex::types::ChatRequest {
            model: "gpt-5".into(),
            messages: vec![
                deecodex::types::ChatMessage {
                    role: "system".into(),
                    content: Some(json!("rules")),
                    reasoning_content: None,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                deecodex::types::ChatMessage {
                    role: "user".into(),
                    content: Some(json!("hello")),
                    reasoning_content: None,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            ],
            tools: vec![json!({
                "type": "function",
                "function": {
                    "name": "health_summary",
                    "description": "health",
                    "parameters": {"type": "object"}
                }
            })],
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

        let body = dex_chat_request_to_responses_body(&req);

        assert_eq!(body["model"], "gpt-5");
        assert_eq!(body["instructions"], "");
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][0]["content"][0]["text"], "rules");
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "health_summary");
    }

    #[test]
    fn dex_responses_to_chat_value_extracts_text_and_tool_calls() {
        let value = json!({
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "ok"}]
                },
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "health_summary",
                    "arguments": "{}"
                }
            ]
        });

        let chat = dex_responses_to_chat_value(value);

        assert_eq!(chat["choices"][0]["message"]["content"], "ok");
        assert_eq!(
            chat["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "health_summary"
        );
    }
}
