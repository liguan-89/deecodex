use futures_util::StreamExt;
use reqwest::header;
use serde_json::{json, Value};
use tauri::{Emitter, State};

use crate::ServerManager;

use super::dex_protocol::{
    dex_responses_request_target, dex_responses_to_chat_value, get_active_account_info,
};

fn response_snippet(text: &str) -> String {
    text.chars().take(500).collect::<String>()
}

fn parse_sse_json_events(body: &str) -> Vec<(String, Value)> {
    fn flush_event(event: &str, data_lines: &[String], events: &mut Vec<(String, Value)>) {
        if data_lines.is_empty() {
            return;
        }
        let data = data_lines.join("\n");
        let trimmed = data.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" {
            return;
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            events.push((event.to_string(), value));
        }
    }

    let mut events = Vec::new();
    let mut event_name = String::new();
    let mut data_lines = Vec::new();
    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            flush_event(&event_name, &data_lines, &mut events);
            event_name.clear();
            data_lines.clear();
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_name = value.trim().to_string();
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start().to_string());
        }
    }
    flush_event(&event_name, &data_lines, &mut events);
    events
}

fn append_chat_delta_tool_calls(tool_calls: &mut Vec<Value>, delta_tool_calls: &[Value]) {
    for delta in delta_tool_calls {
        let index = delta.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        while tool_calls.len() <= index {
            tool_calls.push(json!({
                "id": "",
                "type": "function",
                "function": {"name": "", "arguments": ""}
            }));
        }
        if let Some(id) = delta.get("id").and_then(Value::as_str) {
            if !id.is_empty() {
                tool_calls[index]["id"] = json!(id);
            }
        }
        if let Some(kind) = delta.get("type").and_then(Value::as_str) {
            if !kind.is_empty() {
                tool_calls[index]["type"] = json!(kind);
            }
        }
        if let Some(function) = delta.get("function") {
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                let current = tool_calls[index]["function"]["name"]
                    .as_str()
                    .unwrap_or_default();
                tool_calls[index]["function"]["name"] = json!(format!("{current}{name}"));
            }
            if let Some(arguments) = function.get("arguments").and_then(Value::as_str) {
                let current = tool_calls[index]["function"]["arguments"]
                    .as_str()
                    .unwrap_or_default();
                tool_calls[index]["function"]["arguments"] = json!(format!("{current}{arguments}"));
            }
        }
    }
}

fn response_part_text(part: &Value) -> Option<&str> {
    part.get("text")
        .or_else(|| part.get("output_text"))
        .and_then(Value::as_str)
}

fn response_item_has_visible_output(item: &Value) -> bool {
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "message" | "reasoning" | "reasoning_summary" => item
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .any(|part| response_part_text(part).is_some_and(|text| !text.is_empty()))
            })
            .unwrap_or(false),
        "function_call" => {
            item.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| !name.is_empty())
                || item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .is_some_and(|arguments| !arguments.is_empty())
        }
        "" => false,
        _ => true,
    }
}

fn response_has_visible_output(response: &Value) -> bool {
    response
        .get("output")
        .and_then(Value::as_array)
        .map(|items| items.iter().any(response_item_has_visible_output))
        .unwrap_or(false)
}

fn response_message_item_from_text(content: &str) -> Value {
    json!({
        "type": "message",
        "role": "assistant",
        "status": "completed",
        "content": [{"type": "output_text", "text": content}]
    })
}

fn response_sse_error_message(value: &Value) -> Option<String> {
    let error = value.get("error").or_else(|| {
        value
            .get("response")
            .and_then(|response| response.get("error"))
    })?;
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if code.is_empty() {
            return Some(message.to_string());
        }
        return Some(format!("{code}: {message}"));
    }
    if let Some(message) = error.as_str() {
        return Some(message.to_string());
    }
    Some(error.to_string())
}

fn dex_chat_value_from_sse_body(body: &str) -> Result<Option<Value>, String> {
    let events = parse_sse_json_events(body);
    if events.is_empty() {
        return Ok(None);
    }

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason = String::new();
    let mut usage = Value::Null;
    let mut tool_calls = Vec::new();
    let mut completed_response = None;
    let mut response_output_items = Vec::new();

    for (event_name, value) in events {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name.as_str());
        match event_type {
            "response.failed" | "response.incomplete" | "error" => {
                let message = response_sse_error_message(&value)
                    .unwrap_or_else(|| response_snippet(&value.to_string()));
                return Err(format!("上游 SSE 返回失败: {message}"));
            }
            "response.completed" => {
                completed_response = value
                    .get("response")
                    .cloned()
                    .or_else(|| Some(value.clone()));
            }
            "response.output_text.delta" => {
                if let Some(delta) = value
                    .get("delta")
                    .or_else(|| value.get("text"))
                    .and_then(Value::as_str)
                {
                    content.push_str(delta);
                }
            }
            "response.output_text.done" => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    content = text.to_string();
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                if let Some(delta) = value
                    .get("delta")
                    .or_else(|| value.get("text"))
                    .and_then(Value::as_str)
                {
                    reasoning.push_str(delta);
                }
            }
            "response.reasoning_text.done" | "response.reasoning_summary_text.done" => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    reasoning = text.to_string();
                }
            }
            "response.output_item.done" => {
                if let Some(item) = value.get("item").cloned() {
                    response_output_items.push(item);
                }
            }
            _ => {}
        }

        if let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
        {
            if let Some(delta) = choice.get("delta") {
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    content.push_str(text);
                }
                if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
                    reasoning.push_str(text);
                }
                if let Some(items) = delta.get("tool_calls").and_then(Value::as_array) {
                    append_chat_delta_tool_calls(&mut tool_calls, items);
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                if !reason.is_empty() {
                    finish_reason = reason.to_string();
                }
            }
        }
        if value.get("usage").is_some() {
            usage = value["usage"].clone();
        }
    }

    if let Some(response) = completed_response {
        if response_has_visible_output(&response) {
            return Ok(Some(response));
        }
    }
    if !response_output_items.is_empty() {
        let has_message = response_output_items
            .iter()
            .any(|item| item.get("type").and_then(Value::as_str) == Some("message"));
        if !content.is_empty() && !has_message {
            response_output_items.push(response_message_item_from_text(&content));
        }
        let mut response = json!({
            "object": "response",
            "status": "completed",
            "output": response_output_items,
        });
        if !usage.is_null() {
            response["usage"] = usage.clone();
        }
        return Ok(Some(response));
    }
    if content.is_empty() && reasoning.is_empty() && tool_calls.is_empty() {
        return Ok(None);
    }

    let mut message = json!({"role": "assistant", "content": content});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }
    Ok(Some(json!({
        "choices": [{
            "message": message,
            "finish_reason": if finish_reason.is_empty() { "stop" } else { finish_reason.as_str() },
        }],
        "usage": usage,
    })))
}

async fn parse_dex_json_response(resp: reqwest::Response) -> Result<Value, String> {
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let maybe_sse = content_type.contains("text/event-stream")
        || body.trim_start().starts_with("event:")
        || body.trim_start().starts_with("data:");
    if maybe_sse {
        if let Some(value) = dex_chat_value_from_sse_body(&body)? {
            tracing::info!(
                status = %status,
                content_type = %content_type,
                "dex_chat 已将 SSE 响应收敛为普通消息"
            );
            return Ok(value);
        }
    }
    serde_json::from_str::<Value>(&body).map_err(|e| {
        tracing::warn!(
            status = %status,
            content_type = %content_type,
            error = %e,
            body = %response_snippet(&body),
            "dex_chat 响应不是有效 JSON"
        );
        let hint = if body.trim().is_empty() {
            "响应体为空".to_string()
        } else {
            response_snippet(&body)
        };
        format!(
            "解析响应失败: {e}; status={status}; content-type={}; body={hint}",
            if content_type.is_empty() {
                "unknown"
            } else {
                &content_type
            }
        )
    })
}

pub(super) async fn dex_chat_impl(
    manager: State<'_, ServerManager>,
    messages: Vec<Value>,
    tools: Option<Vec<Value>>,
    stream: Option<bool>,
    model: Option<String>,
) -> Result<Value, String> {
    let stream_mode = stream.unwrap_or(false);

    let data_dir = manager.data_dir.lock().await.clone();

    let (
        upstream,
        api_key,
        model_map,
        known_models,
        provider,
        profile,
        endpoint_kind,
        endpoint_path,
    ) = get_active_account_info(&data_dir)
        .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    let explicit_model = model.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let default_model = known_models
        .first()
        .map(String::as_str)
        .or_else(|| model_map.get("gpt-5.5").map(String::as_str))
        .or_else(|| model_map.values().next().map(String::as_str))
        .unwrap_or("gpt-5.5");
    let requested = explicit_model.unwrap_or(default_model);
    let mapped_model = model_map
        .get(requested)
        .cloned()
        .or_else(|| {
            if explicit_model.is_none() {
                model_map.get("gpt-5.5").cloned()
            } else {
                None
            }
        })
        .or_else(|| {
            if explicit_model.is_none() {
                model_map.values().next().cloned()
            } else {
                None
            }
        })
        .unwrap_or_else(|| requested.to_string());

    let base = upstream.trim_end_matches('/');

    let mut chat_req = deecodex::types::ChatRequest {
        model: mapped_model.clone(),
        messages: messages
            .into_iter()
            .filter_map(|m| serde_json::from_value(m).ok())
            .collect(),
        tools: tools.unwrap_or_default(),
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: stream_mode,
        reasoning_effort: None,
        thinking: None,
        reasoning_split: None,
        tool_choice: None,
        parallel_tool_calls: None,
        response_format: None,
        user: None,
        stream_options: None,
        web_search_options: None,
    };
    deecodex::providers::adapt_chat_request(&profile, &mut chat_req);
    let msg_count = chat_req.messages.len();

    if stream_mode && profile.wire_protocol != deecodex::providers::WireProtocol::ChatCompletions {
        return Err(
            "DEX 助手暂不支持 Anthropic/Gemini 原生协议流式请求，请关闭流式或切换 Chat 兼容供应商"
                .into(),
        );
    }

    let (url, body, use_provider_headers) = match profile.wire_protocol {
        deecodex::providers::WireProtocol::ChatCompletions => (
            format!("{base}/chat/completions"),
            serde_json::to_value(&chat_req).map_err(|e| format!("序列化请求失败: {e}"))?,
            true,
        ),
        deecodex::providers::WireProtocol::AnthropicMessages
        | deecodex::providers::WireProtocol::GeminiNative => {
            let url = deecodex::native_protocols::native_endpoint(
                &profile.wire_protocol,
                &upstream,
                &chat_req.model,
                false,
                &api_key,
            )
            .ok_or_else(|| "当前供应商原生协议尚未接入 DEX 助手".to_string())?;
            let body =
                deecodex::native_protocols::to_native_request(&profile.wire_protocol, &chat_req)
                    .ok_or_else(|| "无法构造原生协议请求".to_string())?;
            (url, body, true)
        }
        deecodex::providers::WireProtocol::Responses => {
            dex_responses_request_target(
                &manager,
                &endpoint_kind,
                &upstream,
                &endpoint_path,
                &chat_req,
            )
            .await?
        }
    };

    tracing::info!(
        url = %url,
        provider = %provider,
        profile = %profile.slug,
        protocol = ?profile.wire_protocol,
        model = %mapped_model,
        msg_count,
        stream = stream_mode,
        "dex_chat 发送请求"
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))?;
    let mut req = client
        .post(&url)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::ACCEPT_ENCODING, "identity")
        .json(&body);
    if use_provider_headers {
        for (name, value) in deecodex::providers::request_headers(&profile, &api_key) {
            req = req.header(name, value);
        }
    }

    let resp = req.send().await.map_err(|e| {
        tracing::error!(error = %e, "dex_chat 请求失败");
        if e.is_timeout() || e.is_connect() {
            "连接上游超时，请检查网络或上游地址".to_string()
        } else {
            format!("请求失败: {e}")
        }
    })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let snippet = response_snippet(&body);
        let prefix = match status.as_u16() {
            401 => "API Key 无效，请检查账号配置",
            403 => "API 访问被拒绝，请检查账号权限",
            429 => "请求频率过高，请稍后重试",
            code if code >= 500 => "上游服务暂时不可用，请稍后重试",
            _ => "上游返回错误",
        };
        return Err(format!("{} ({}): {}", prefix, status, snippet));
    }

    if stream_mode {
        let app_handle = {
            let guard = manager.app_handle.lock().await;
            guard.clone()
        };

        let mut stream = resp.bytes_stream();
        let mut buffer = String::new();
        let mut finish_reason = String::new();
        let mut usage = Value::Null;

        'stream_loop: while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("流读取失败: {e}"))?;
            let text = String::from_utf8_lossy(&chunk);
            buffer.push_str(&text);

            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break 'stream_loop;
                    }
                    if let Ok(chunk_val) = serde_json::from_str::<Value>(data) {
                        if let Some(fr) = chunk_val["choices"]
                            .as_array()
                            .and_then(|arr| arr.first())
                            .and_then(|c| c["finish_reason"].as_str())
                        {
                            if !fr.is_empty() {
                                finish_reason = fr.to_string();
                            }
                        }
                        if chunk_val.get("usage").is_some() {
                            usage = chunk_val["usage"].clone();
                        }

                        if let Some(ref handle) = app_handle {
                            let _ = handle.emit(
                                "dex-chat-chunk",
                                &json!({ "chunk": chunk_val, "done": false }),
                            );
                        }
                    }
                }
            }
        }

        if let Some(ref handle) = app_handle {
            let _ = handle.emit("dex-chat-chunk", &json!({ "chunk": null, "done": true }));
        }

        Ok(json!({
            "stream": true,
            "finish_reason": finish_reason,
            "usage": usage,
        }))
    } else {
        let resp_body: Value = parse_dex_json_response(resp).await?;

        if profile.wire_protocol == deecodex::providers::WireProtocol::ChatCompletions {
            let choice = resp_body["choices"]
                .as_array()
                .and_then(|choices| choices.first())
                .ok_or_else(|| "响应中没有 choices 数据".to_string())?;

            let message = &choice["message"];
            let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

            return Ok(json!({
                "choices": [{
                    "message": message.clone(),
                    "finish_reason": finish_reason,
                }]
            }));
        }
        if profile.wire_protocol == deecodex::providers::WireProtocol::Responses {
            return Ok(dex_responses_to_chat_value(resp_body));
        }

        let chat_resp =
            deecodex::native_protocols::native_response_to_chat(&profile.wire_protocol, resp_body)
                .map_err(|e| format!("解析原生协议响应失败: {e}"))?;
        let message = chat_resp
            .choices
            .first()
            .map(|choice| serde_json::to_value(&choice.message).unwrap_or_else(|_| json!({})))
            .unwrap_or_else(|| json!({"role":"assistant","content":""}));
        Ok(json!({
            "choices": [{
                "message": message,
                "finish_reason": "stop",
            }]
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_sse_delta_survives_empty_completed_event() {
        let body = r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress","output":[]}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"你好"}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"，DEX"}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[]}}

"#;

        let value = dex_chat_value_from_sse_body(body).unwrap().unwrap();
        let chat = dex_responses_to_chat_value(value);

        assert_eq!(chat["choices"][0]["message"]["content"], "你好，DEX");
    }

    #[test]
    fn responses_sse_uses_completed_output_when_present() {
        let body = r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_2","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"完整回复"}]}]}}

"#;

        let value = dex_chat_value_from_sse_body(body).unwrap().unwrap();
        let chat = dex_responses_to_chat_value(value);

        assert_eq!(chat["choices"][0]["message"]["content"], "完整回复");
    }

    #[test]
    fn chat_sse_delta_becomes_chat_response() {
        let body = r#"data: {"choices":[{"delta":{"content":"hello"},"finish_reason":null}]}

data: {"choices":[{"delta":{"content":" world"},"finish_reason":"stop"}],"usage":{"total_tokens":3}}

data: [DONE]

"#;

        let value = dex_chat_value_from_sse_body(body).unwrap().unwrap();

        assert_eq!(value["choices"][0]["message"]["content"], "hello world");
        assert_eq!(value["choices"][0]["finish_reason"], "stop");
        assert_eq!(value["usage"]["total_tokens"], 3);
    }

    #[test]
    fn responses_sse_failed_event_returns_clear_error() {
        let body = r#"event: response.failed
data: {"type":"response.failed","response":{"status":"failed","error":{"code":"bad_request","message":"Instructions are required"}}}

"#;

        let err = dex_chat_value_from_sse_body(body).unwrap_err();

        assert!(err.contains("bad_request: Instructions are required"));
    }
}
