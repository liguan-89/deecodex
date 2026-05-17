use axum::{
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde_json::{json, Value};
use tracing::{error, info};

use crate::handlers::AppState;
use crate::types::ChatRequest;
use crate::utils::merge_response_extra;

pub struct VlmArgs {
    pub state: AppState,
    pub url: String,
    pub api_key: String,
    pub vlm_body: Value,
    pub model: String,
    pub stream_response: bool,
    pub store_response: bool,
    pub request_input_items: Vec<Value>,
    pub response_extra: Value,
}

pub fn strip_images_from_chat_request(chat_req: &mut ChatRequest) {
    for msg in &mut chat_req.messages {
        if let Some(ref content) = msg.content {
            if let Some(parts) = content.as_array() {
                let text_parts: Vec<&str> = parts
                    .iter()
                    .filter_map(|p| {
                        if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                            p.get("text").and_then(|t| t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                msg.content = Some(Value::String(text_parts.join("")));
            } else if let Some(s) = content.as_str() {
                if let Some(pos) = s.find("data:image/") {
                    let stripped = s[..pos].trim().to_string();
                    msg.content = Some(Value::String(stripped));
                }
            }
        }
    }
}

/// 提取 MiniMax coding_plan/vlm 所需的 prompt 与 image_url。
pub fn build_minimax_vlm_body(chat_req: &ChatRequest) -> Value {
    let mut prompt = String::new();
    let mut image_url = String::new();

    for msg in chat_req.messages.iter().rev() {
        if msg.role != "user" {
            continue;
        }
        if let Some(ref content) = msg.content {
            match content {
                Value::String(s) => {
                    if let Some(pos) = s.find("data:image/") {
                        if image_url.is_empty() {
                            image_url = s[pos..].trim().to_string();
                        }
                        let text = s[..pos].trim();
                        if !text.is_empty() && prompt.is_empty() {
                            prompt = text.to_string();
                        }
                    } else if prompt.is_empty() && !s.starts_with("data:") {
                        prompt = s.clone();
                    }
                }
                Value::Array(parts) => {
                    for p in parts {
                        match p.get("type").and_then(|t| t.as_str()) {
                            Some("text") if prompt.is_empty() => {
                                if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                                    prompt = t.to_string();
                                }
                            }
                            Some("image_url") if image_url.is_empty() => {
                                image_url = p
                                    .get("image_url")
                                    .and_then(|u| u.get("url"))
                                    .and_then(|u| u.as_str())
                                    .unwrap_or("")
                                    .to_string();
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if prompt.is_empty() {
        prompt = "Describe the image.".into();
    }

    json!({ "prompt": prompt, "image_url": image_url })
}

pub async fn request_minimax_vlm_text(
    state: &AppState,
    url: &str,
    api_key: &str,
    vlm_body: &Value,
) -> Result<String, Response> {
    let mut builder = state
        .client
        .post(url)
        .header("Content-Type", "application/json");
    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key);
    }

    let vlm_result = match builder.json(vlm_body).send().await {
        Err(e) => {
            error!("vlm upstream error: {e}");
            return Err((StatusCode::BAD_GATEWAY, e.to_string()).into_response());
        }
        Ok(r) if !r.status().is_success() => {
            let status = r.status();
            let body = r.text().await.unwrap_or_default();
            error!("vlm upstream {}: {}", status.as_u16(), body);
            return Err((
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                body,
            )
                .into_response());
        }
        Ok(r) => match r.json::<Value>().await {
            Err(e) => {
                error!("vlm parse error: {e}");
                return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response());
            }
            Ok(resp) => resp,
        },
    };

    Ok(vlm_result
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// 调用 MiniMax VLM，并把返回文本包装成 Responses API 响应。
pub async fn handle_minimax_vlm(args: VlmArgs) -> Response {
    let VlmArgs {
        state,
        url,
        api_key,
        vlm_body,
        model,
        stream_response,
        store_response,
        request_input_items,
        response_extra,
    } = args;
    let text = match request_minimax_vlm_text(&state, &url, &api_key, &vlm_body).await {
        Ok(text) => text,
        Err(resp) => return resp,
    };
    info!("↑ vlm done text_len={}", text.len());

    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());
    let msg_id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    if store_response {
        state
            .sessions
            .save_input_items(response_id.clone(), request_input_items);
    }

    let mut response_obj = json!({
        "id": &response_id,
        "object": "response",
        "created_at": now_unix_secs(),
        "status": "completed",
        "background": false,
        "error": null,
        "incomplete_details": null,
        "model": &model,
        "output": [{
            "type": "message",
            "id": &msg_id,
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": &text, "annotations": [], "logprobs": []}]
        }],
        "usage": {"input_tokens": 0, "output_tokens": 0, "total_tokens": 0}
    });
    merge_response_extra(&mut response_obj, &response_extra);
    if store_response {
        state
            .sessions
            .save_response(response_id.clone(), response_obj.clone());
    }

    if !stream_response {
        return Json(response_obj).into_response();
    }

    use crate::sse::SseState;
    let mut vss = SseState::new();
    let vlm_oi = vss.alloc_output_index();

    let events: Vec<Result<Event, std::convert::Infallible>> = vec![
        vss.response_created(&response_id, &model),
        vss.response_in_progress(&response_id),
        vss.output_item_added(
            vlm_oi,
            &msg_id,
            "message",
            json!({"role": "assistant", "content": []}),
        ),
        vss.content_part_added(
            &msg_id,
            vlm_oi,
            0,
            json!({"type": "output_text", "text": "", "annotations": [], "logprobs": []}),
        ),
        vss.output_text_delta(&msg_id, vlm_oi, 0, &text),
        vss.output_text_done(&msg_id, vlm_oi, 0, &text),
        vss.content_part_done(
            &msg_id,
            vlm_oi,
            0,
            json!({
                "type": "output_text",
                "text": &text,
                "annotations": [],
                "logprobs": []
            }),
        ),
        vss.output_item_done(
            vlm_oi,
            json!({
                "type": "message",
                "id": &msg_id,
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": &text, "annotations": [], "logprobs": []}]
            }),
        ),
        vss.response_completed(&response_obj),
    ];

    Sse::new(futures_util::stream::iter(events))
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatMessage;

    #[test]
    fn test_build_minimax_vlm_body_extracts_prompt_and_image() {
        let chat_req = ChatRequest {
            model: "vision".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: Some(json!([
                    {"type": "text", "text": "请描述这张图"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
                ])),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
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

        let body = build_minimax_vlm_body(&chat_req);
        assert_eq!(body["prompt"], "请描述这张图");
        assert_eq!(body["image_url"], "data:image/png;base64,abc");
    }

    #[test]
    fn test_strip_images_from_chat_request_keeps_text_only() {
        let mut chat_req = ChatRequest {
            model: "text".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: Some(json!([
                    {"type": "text", "text": "只保留文本"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
                ])),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
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

        strip_images_from_chat_request(&mut chat_req);
        assert_eq!(
            chat_req.messages[0].content.as_ref().unwrap(),
            &Value::String("只保留文本".into())
        );
    }
}
