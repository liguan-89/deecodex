#![allow(dead_code)]

use serde_json::{json, Value};

use crate::providers::WireProtocol;
use crate::types::{ChatChoice, ChatMessage, ChatRequest, ChatResponse, ChatUsage};

pub fn to_native_request(protocol: &WireProtocol, req: &ChatRequest) -> Option<Value> {
    match protocol {
        WireProtocol::AnthropicMessages => Some(to_anthropic_messages_request(req)),
        WireProtocol::GeminiNative => Some(to_gemini_generate_content_request(req)),
        _ => None,
    }
}

pub fn native_endpoint(
    protocol: &WireProtocol,
    upstream: &str,
    model: &str,
    stream: bool,
    api_key: &str,
) -> Option<String> {
    let base = upstream.trim_end_matches('/');
    match protocol {
        WireProtocol::AnthropicMessages => Some(format!("{base}/messages")),
        WireProtocol::GeminiNative => {
            let action = if stream {
                "streamGenerateContent"
            } else {
                "generateContent"
            };
            let mut url = format!("{base}/models/{model}:{action}");
            if !api_key.is_empty() {
                let sep = if url.contains('?') { '&' } else { '?' };
                url.push(sep);
                url.push_str("key=");
                url.push_str(api_key);
            }
            Some(url)
        }
        _ => None,
    }
}

pub fn native_response_to_chat(
    protocol: &WireProtocol,
    body: Value,
) -> Result<ChatResponse, String> {
    match protocol {
        WireProtocol::AnthropicMessages => anthropic_response_to_chat(body),
        WireProtocol::GeminiNative => gemini_response_to_chat(body),
        other => Err(format!("unsupported native wire protocol: {other:?}")),
    }
}

pub fn to_anthropic_messages_request(req: &ChatRequest) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();

    for msg in &req.messages {
        if msg.role == "system" {
            if let Some(text) = text_content(msg) {
                system_parts.push(text);
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
            "content": text_content(msg).unwrap_or_default(),
        }));
    }

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": req.stream,
    });
    if let Some(max_tokens) = req.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    } else {
        body["max_tokens"] = json!(4096);
    }
    if let Some(temp) = req.temperature {
        body["temperature"] = json!(temp);
    }
    if !system_parts.is_empty() {
        body["system"] = json!(system_parts.join("\n\n"));
    }
    body
}

pub fn to_gemini_generate_content_request(req: &ChatRequest) -> Value {
    let mut system_parts = Vec::new();
    let mut contents = Vec::new();

    for msg in &req.messages {
        if msg.role == "system" {
            if let Some(text) = text_content(msg) {
                system_parts.push(text);
            }
            continue;
        }
        let role = if msg.role == "assistant" {
            "model"
        } else {
            "user"
        };
        contents.push(json!({
            "role": role,
            "parts": [{"text": text_content(msg).unwrap_or_default()}],
        }));
    }

    let mut body = json!({
        "contents": contents,
    });
    if !system_parts.is_empty() {
        body["systemInstruction"] = json!({
            "parts": [{"text": system_parts.join("\n\n")}]
        });
    }
    let mut generation_config = serde_json::Map::new();
    if let Some(max_tokens) = req.max_tokens {
        generation_config.insert("maxOutputTokens".into(), json!(max_tokens));
    }
    if let Some(temp) = req.temperature {
        generation_config.insert("temperature".into(), json!(temp));
    }
    if let Some(top_p) = req.top_p {
        generation_config.insert("topP".into(), json!(top_p));
    }
    if !generation_config.is_empty() {
        body["generationConfig"] = Value::Object(generation_config);
    }
    body
}

fn anthropic_response_to_chat(body: Value) -> Result<ChatResponse, String> {
    let content = body
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| {
                    let typ = part.get("type").and_then(Value::as_str).unwrap_or("text");
                    if typ == "text" {
                        part.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let usage = body.get("usage").map(|usage| ChatUsage {
        prompt_tokens: u32_field(usage, "input_tokens"),
        completion_tokens: u32_field(usage, "output_tokens"),
        total_tokens: u32_field(usage, "input_tokens") + u32_field(usage, "output_tokens"),
        completion_tokens_details: None,
        prompt_cache_hit_tokens: None,
        prompt_cache_miss_tokens: None,
        prompt_tokens_details: None,
    });
    Ok(chat_response_from_text(content, usage))
}

fn gemini_response_to_chat(body: Value) -> Result<ChatResponse, String> {
    let content = body
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let usage = body.get("usageMetadata").map(|usage| {
        let prompt = u32_field(usage, "promptTokenCount");
        let completion = u32_field(usage, "candidatesTokenCount");
        ChatUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: u32_field(usage, "totalTokenCount").max(prompt + completion),
            completion_tokens_details: None,
            prompt_cache_hit_tokens: None,
            prompt_cache_miss_tokens: None,
            prompt_tokens_details: None,
        }
    });
    Ok(chat_response_from_text(content, usage))
}

fn chat_response_from_text(content: String, usage: Option<ChatUsage>) -> ChatResponse {
    ChatResponse {
        choices: vec![ChatChoice {
            message: ChatMessage {
                role: "assistant".into(),
                content: Some(Value::String(content)),
                reasoning_content: None,
                reasoning_details: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,

                ..Default::default()
            },
        }],
        usage,
    }
}

fn u32_field(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn text_content(msg: &ChatMessage) -> Option<String> {
    match msg.content.as_ref()? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| {
                    part.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| part.get("content").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join(""),
        ),
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;

    fn msg(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: Some(json!(content)),
            reasoning_content: None,
            reasoning: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn chat_req() -> ChatRequest {
        ChatRequest {
            model: "native-model".into(),
            messages: vec![
                msg("system", "be concise"),
                msg("user", "hello"),
                msg("assistant", "hi"),
            ],
            tools: vec![],
            temperature: Some(0.2),
            top_p: Some(0.9),
            max_tokens: Some(128),
            stream: false,
            reasoning_effort: None,
            thinking: None,
            reasoning_split: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            user: None,
            stream_options: None,
            web_search_options: None,
        }
    }

    #[test]
    fn anthropic_request_maps_system_and_messages() {
        let body = to_anthropic_messages_request(&chat_req());
        assert_eq!(body["model"], "native-model");
        assert_eq!(body["system"], "be concise");
        assert_eq!(body["max_tokens"], 128);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][1]["role"], "assistant");
    }

    #[test]
    fn gemini_request_maps_roles_and_generation_config() {
        let body = to_gemini_generate_content_request(&chat_req());
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "be concise");
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][1]["role"], "model");
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 128);
        assert_eq!(body["generationConfig"]["topP"], 0.9);
    }

    #[test]
    fn native_endpoint_maps_provider_paths() {
        assert_eq!(
            native_endpoint(
                &WireProtocol::AnthropicMessages,
                "https://api.anthropic.com/v1/",
                "claude",
                false,
                "sk"
            )
            .unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            native_endpoint(
                &WireProtocol::GeminiNative,
                "https://generativelanguage.googleapis.com/v1beta",
                "gemini-2.0-flash",
                false,
                "gemini-key"
            )
            .unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key=gemini-key"
        );
    }

    #[test]
    fn native_responses_normalize_to_chat_response() {
        let anthropic = native_response_to_chat(
            &WireProtocol::AnthropicMessages,
            json!({
                "content": [{"type":"text","text":"hello"}],
                "usage": {"input_tokens": 3, "output_tokens": 4}
            }),
        )
        .unwrap();
        assert_eq!(anthropic.choices[0].message.content, Some(json!("hello")));
        assert_eq!(anthropic.usage.unwrap().total_tokens, 7);

        let gemini = native_response_to_chat(
            &WireProtocol::GeminiNative,
            json!({
                "candidates": [{"content": {"parts": [{"text":"hi"}]}}],
                "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 6, "totalTokenCount": 11}
            }),
        )
        .unwrap();
        assert_eq!(gemini.choices[0].message.content, Some(json!("hi")));
        assert_eq!(gemini.usage.unwrap().completion_tokens, 6);
    }
}
