use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::{Emitter, State};

use crate::ServerManager;

use super::dex_protocol::{
    dex_responses_request_target, dex_responses_to_chat_value, get_active_account_info,
};

pub(super) async fn dex_chat_impl(
    manager: State<'_, ServerManager>,
    messages: Vec<Value>,
    tools: Option<Vec<Value>>,
    stream: Option<bool>,
    model: Option<String>,
) -> Result<Value, String> {
    let stream_mode = stream.unwrap_or(false);

    let data_dir = manager.data_dir.lock().await.clone();

    let (upstream, api_key, model_map, provider, profile, endpoint_kind, endpoint_path) =
        get_active_account_info(&data_dir)
            .ok_or_else(|| "请先在账号管理中配置一个活跃账号".to_string())?;

    let default_model = "gpt-5.5";
    let requested = model
        .as_deref()
        .filter(|m| !m.is_empty())
        .unwrap_or(default_model);
    let mapped_model = model_map
        .get(requested)
        .cloned()
        .or_else(|| model_map.get(default_model).cloned())
        .or_else(|| model_map.values().next().cloned())
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

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&body);
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
        let err_msg = match status.as_u16() {
            401 => "API Key 无效，请检查账号配置".to_string(),
            403 => "API 访问被拒绝，请检查账号权限".to_string(),
            429 => "请求频率过高，请稍后重试".to_string(),
            code if code >= 500 => "上游服务暂时不可用，请稍后重试".to_string(),
            _ => {
                let body = resp.text().await.unwrap_or_default();
                format!("上游返回错误 ({}): {}", status, body)
            }
        };
        return Err(err_msg);
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
        let resp_body: Value = resp
            .json()
            .await
            .map_err(|e| format!("解析响应失败: {e}"))?;

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
