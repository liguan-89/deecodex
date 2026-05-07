#![allow(dead_code)]

// SSE event builder helpers for Responses API.
// Each builder emits the correct event type, sequence_number, and payload shape
// matching the OpenAI Responses API spec.

use axum::response::sse::Event;
use serde_json::{json, Value};
use std::convert::Infallible;

/// Accumulates state for building a complete Responses SSE event stream.
pub struct SseState {
    seq: u32,
    next_output_index: usize,
}

impl Default for SseState {
    fn default() -> Self {
        Self::new()
    }
}

impl SseState {
    pub fn new() -> Self {
        Self {
            seq: 0,
            next_output_index: 0,
        }
    }

    pub fn alloc_output_index(&mut self) -> usize {
        let ix = self.next_output_index;
        self.next_output_index += 1;
        ix
    }

    pub fn response_created(&mut self, id: &str, model: &str) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.created").data(
            json!({
                "type": "response.created",
                "sequence_number": self.seq,
                "response": { "id": id, "status": "in_progress", "model": model }
            })
            .to_string(),
        ))
    }

    pub fn response_in_progress(&mut self, id: &str) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.in_progress").data(
            json!({
                "type": "response.in_progress",
                "sequence_number": self.seq,
                "response": { "id": id, "status": "in_progress" }
            })
            .to_string(),
        ))
    }

    pub fn output_item_added(
        &mut self,
        output_index: usize,
        item_id: &str,
        item_type: &str,
        extra: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        let mut item = json!({
            "type": item_type,
            "id": item_id,
            "status": "in_progress"
        });
        merge(&mut item, &extra);
        Ok(Event::default().event("response.output_item.added").data(
            json!({
                "type": "response.output_item.added",
                "sequence_number": self.seq,
                "output_index": output_index,
                "item": item
            })
            .to_string(),
        ))
    }

    pub fn content_part_added(
        &mut self,
        item_id: &str,
        output_index: usize,
        content_index: usize,
        part: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.content_part.added").data(
            json!({
                "type": "response.content_part.added",
                "sequence_number": self.seq,
                "item_id": item_id,
                "output_index": output_index,
                "content_index": content_index,
                "part": part
            })
            .to_string(),
        ))
    }

    pub fn output_text_delta(
        &mut self,
        item_id: &str,
        output_index: usize,
        content_index: usize,
        delta: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.output_text.delta").data(
            json!({
                "type": "response.output_text.delta",
                "sequence_number": self.seq,
                "item_id": item_id,
                "output_index": output_index,
                "content_index": content_index,
                "delta": delta
            })
            .to_string(),
        ))
    }

    pub fn output_text_done(
        &mut self,
        item_id: &str,
        output_index: usize,
        content_index: usize,
        text: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.output_text.done").data(
            json!({
                "type": "response.output_text.done",
                "sequence_number": self.seq,
                "item_id": item_id,
                "output_index": output_index,
                "content_index": content_index,
                "text": text,
                "logprobs": []
            })
            .to_string(),
        ))
    }

    pub fn content_part_done(
        &mut self,
        item_id: &str,
        output_index: usize,
        content_index: usize,
        part: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.content_part.done").data(
            json!({
                "type": "response.content_part.done",
                "sequence_number": self.seq,
                "item_id": item_id,
                "output_index": output_index,
                "content_index": content_index,
                "part": part
            })
            .to_string(),
        ))
    }

    pub fn reasoning_summary_part_added(
        &mut self,
        item_id: &str,
        output_index: usize,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default()
            .event("response.reasoning_summary_part.added")
            .data(
                json!({
                    "type": "response.reasoning_summary_part.added",
                    "sequence_number": self.seq,
                    "item_id": item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": "" }
                })
                .to_string(),
            ))
    }

    pub fn reasoning_summary_text_delta(
        &mut self,
        item_id: &str,
        output_index: usize,
        delta: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default()
            .event("response.reasoning_summary_text.delta")
            .data(
                json!({
                    "type": "response.reasoning_summary_text.delta",
                    "sequence_number": self.seq,
                    "item_id": item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "delta": delta
                })
                .to_string(),
            ))
    }

    pub fn reasoning_summary_text_done(
        &mut self,
        item_id: &str,
        output_index: usize,
        text: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default()
            .event("response.reasoning_summary_text.done")
            .data(
                json!({
                    "type": "response.reasoning_summary_text.done",
                    "sequence_number": self.seq,
                    "item_id": item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "text": text
                })
                .to_string(),
            ))
    }

    pub fn reasoning_summary_part_done(
        &mut self,
        item_id: &str,
        output_index: usize,
        text: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default()
            .event("response.reasoning_summary_part.done")
            .data(
                json!({
                    "type": "response.reasoning_summary_part.done",
                    "sequence_number": self.seq,
                    "item_id": item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": text }
                })
                .to_string(),
            ))
    }

    pub fn function_call_arguments_delta(
        &mut self,
        item_id: &str,
        output_index: usize,
        delta: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default()
            .event("response.function_call_arguments.delta")
            .data(
                json!({
                    "type": "response.function_call_arguments.delta",
                    "sequence_number": self.seq,
                    "item_id": item_id,
                    "output_index": output_index,
                    "delta": delta
                })
                .to_string(),
            ))
    }

    pub fn function_call_arguments_done(
        &mut self,
        item_id: &str,
        output_index: usize,
        arguments: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default()
            .event("response.function_call_arguments.done")
            .data(
                json!({
                    "type": "response.function_call_arguments.done",
                    "sequence_number": self.seq,
                    "item_id": item_id,
                    "output_index": output_index,
                    "arguments": arguments
                })
                .to_string(),
            ))
    }

    pub fn output_item_done(
        &mut self,
        output_index: usize,
        item: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.output_item.done").data(
            json!({
                "type": "response.output_item.done",
                "sequence_number": self.seq,
                "output_index": output_index,
                "item": item
            })
            .to_string(),
        ))
    }

    pub fn response_completed(&mut self, response: &Value) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.completed").data(
            json!({
                "type": "response.completed",
                "sequence_number": self.seq,
                "response": response
            })
            .to_string(),
        ))
    }

    pub fn response_failed(&mut self, response: &Value) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.failed").data(
            json!({
                "type": "response.failed",
                "sequence_number": self.seq,
                "response": response
            })
            .to_string(),
        ))
    }
}

fn merge(target: &mut Value, source: &Value) {
    if let (Value::Object(t), Value::Object(s)) = (target, source) {
        for (k, v) in s {
            t.insert(k.clone(), v.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use futures_util::stream;
    use serde_json::json;

    /// Serialize a single Event through the axum Sse pipeline and return
    /// the parsed SSE fields as (event_name, data_json).
    async fn parse_event(event: Event) -> (String, Value) {
        let stream = stream::once(async move { Ok::<_, Infallible>(event) });
        let sse = axum::response::sse::Sse::new(stream);
        let res = sse.into_response();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();

        let mut event_name = String::new();
        let mut data_str = String::new();
        for line in text.lines() {
            if let Some(val) = line.strip_prefix("event: ") {
                event_name = val.to_string();
            } else if let Some(val) = line.strip_prefix("data: ") {
                data_str = val.to_string();
            }
        }
        let data: Value = serde_json::from_str(&data_str).unwrap();
        (event_name, data)
    }

    #[tokio::test]
    async fn test_new_initial_state() {
        let mut state = SseState::new();
        let (name, data) = parse_event(state.response_created("r1", "m1").unwrap()).await;
        assert_eq!(data["sequence_number"], 1);
        assert_eq!(name, "response.created");
    }

    #[tokio::test]
    async fn test_default() {
        let mut state = SseState::default();
        let (name, data) = parse_event(state.response_created("r1", "m1").unwrap()).await;
        assert_eq!(data["sequence_number"], 1);
        assert_eq!(name, "response.created");
    }

    #[test]
    fn test_alloc_output_index() {
        let mut state = SseState::new();
        assert_eq!(state.alloc_output_index(), 0);
        assert_eq!(state.alloc_output_index(), 1);
        assert_eq!(state.alloc_output_index(), 2);
    }

    #[tokio::test]
    async fn test_sequence_increments_across_calls() {
        let mut s = SseState::new();

        let (_, d1) = parse_event(s.response_created("r1", "m1").unwrap()).await;
        assert_eq!(d1["sequence_number"], 1);

        let (_, d2) = parse_event(s.response_in_progress("r1").unwrap()).await;
        assert_eq!(d2["sequence_number"], 2);

        let (_, d3) =
            parse_event(s.output_item_added(0, "i1", "message", json!({})).unwrap()).await;
        assert_eq!(d3["sequence_number"], 3);

        let (_, d4) = parse_event(
            s.content_part_added("i1", 0, 0, json!({"type": "text"}))
                .unwrap(),
        )
        .await;
        assert_eq!(d4["sequence_number"], 4);

        let (_, d5) = parse_event(s.output_text_delta("i1", 0, 0, "hello").unwrap()).await;
        assert_eq!(d5["sequence_number"], 5);

        let (_, d6) = parse_event(
            s.content_part_done("i1", 0, 0, json!({"type": "text", "text": "hello"}))
                .unwrap(),
        )
        .await;
        assert_eq!(d6["sequence_number"], 6);

        let (_, d7) = parse_event(s.output_item_done(0, json!({"type": "message"})).unwrap()).await;
        assert_eq!(d7["sequence_number"], 7);

        let (_, d8) = parse_event(
            s.response_completed(&json!({"id": "r1", "status": "completed"}))
                .unwrap(),
        )
        .await;
        assert_eq!(d8["sequence_number"], 8);

        let (_, d9) = parse_event(
            s.response_failed(&json!({"id": "r1", "status": "failed"}))
                .unwrap(),
        )
        .await;
        assert_eq!(d9["sequence_number"], 9);
    }

    #[tokio::test]
    async fn test_response_created() {
        let mut s = SseState::new();
        let (name, data) =
            parse_event(s.response_created("resp_123", "deepseek-v3").unwrap()).await;
        assert_eq!(name, "response.created");
        assert_eq!(data["type"], "response.created");
        assert_eq!(data["sequence_number"], 1);
        assert_eq!(data["response"]["id"], "resp_123");
        assert_eq!(data["response"]["model"], "deepseek-v3");
        assert_eq!(data["response"]["status"], "in_progress");
    }

    #[tokio::test]
    async fn test_response_in_progress() {
        let mut s = SseState::new();
        let (name, data) = parse_event(s.response_in_progress("resp_123").unwrap()).await;
        assert_eq!(name, "response.in_progress");
        assert_eq!(data["type"], "response.in_progress");
        assert_eq!(data["response"]["id"], "resp_123");
        assert_eq!(data["response"]["status"], "in_progress");
    }

    #[tokio::test]
    async fn test_output_item_added_basic() {
        let mut s = SseState::new();
        let (name, data) = parse_event(
            s.output_item_added(0, "item_1", "message", json!({}))
                .unwrap(),
        )
        .await;
        assert_eq!(name, "response.output_item.added");
        assert_eq!(data["type"], "response.output_item.added");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["item"]["type"], "message");
        assert_eq!(data["item"]["id"], "item_1");
        assert_eq!(data["item"]["status"], "in_progress");
    }

    #[tokio::test]
    async fn test_output_item_added_with_extra_fields() {
        let mut s = SseState::new();
        let extra = json!({"name": "my_func", "call_id": "call_1"});
        let (_, data) = parse_event(
            s.output_item_added(1, "fc_1", "function_call", extra)
                .unwrap(),
        )
        .await;
        assert_eq!(data["output_index"], 1);
        assert_eq!(data["item"]["name"], "my_func");
        assert_eq!(data["item"]["call_id"], "call_1");
        assert_eq!(data["item"]["type"], "function_call");
    }

    #[tokio::test]
    async fn test_content_part_added() {
        let mut s = SseState::new();
        let part = json!({"type": "text"});
        let (name, data) =
            parse_event(s.content_part_added("item_1", 0, 0, part.clone()).unwrap()).await;
        assert_eq!(name, "response.content_part.added");
        assert_eq!(data["item_id"], "item_1");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["content_index"], 0);
        assert_eq!(data["part"], part);
    }

    #[tokio::test]
    async fn test_output_text_delta() {
        let mut s = SseState::new();
        let (name, data) =
            parse_event(s.output_text_delta("item_1", 0, 0, "Hello, ").unwrap()).await;
        assert_eq!(name, "response.output_text.delta");
        assert_eq!(data["item_id"], "item_1");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["content_index"], 0);
        assert_eq!(data["delta"], "Hello, ");
    }

    #[tokio::test]
    async fn test_output_text_done() {
        let mut s = SseState::new();
        let (name, data) =
            parse_event(s.output_text_done("item_1", 0, 0, "Hello, world!").unwrap()).await;
        assert_eq!(name, "response.output_text.done");
        assert_eq!(data["text"], "Hello, world!");
        assert_eq!(data["logprobs"], json!([]));
    }

    #[tokio::test]
    async fn test_content_part_done() {
        let mut s = SseState::new();
        let part = json!({"type": "text", "text": "Hello"});
        let (name, data) =
            parse_event(s.content_part_done("item_1", 0, 0, part.clone()).unwrap()).await;
        assert_eq!(name, "response.content_part.done");
        assert_eq!(data["item_id"], "item_1");
        assert_eq!(data["content_index"], 0);
        assert_eq!(data["part"], part);
    }

    #[tokio::test]
    async fn test_reasoning_summary_part_added() {
        let mut s = SseState::new();
        let (name, data) = parse_event(s.reasoning_summary_part_added("item_r1", 0).unwrap()).await;
        assert_eq!(name, "response.reasoning_summary_part.added");
        assert_eq!(data["item_id"], "item_r1");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["summary_index"], 0);
        assert_eq!(data["part"]["type"], "summary_text");
        assert_eq!(data["part"]["text"], "");
    }

    #[tokio::test]
    async fn test_reasoning_summary_text_delta() {
        let mut s = SseState::new();
        let (name, data) = parse_event(
            s.reasoning_summary_text_delta("item_r1", 0, "thinking...")
                .unwrap(),
        )
        .await;
        assert_eq!(name, "response.reasoning_summary_text.delta");
        assert_eq!(data["summary_index"], 0);
        assert_eq!(data["delta"], "thinking...");
    }

    #[tokio::test]
    async fn test_reasoning_summary_text_done() {
        let mut s = SseState::new();
        let (name, data) = parse_event(
            s.reasoning_summary_text_done("item_r1", 0, "final thought")
                .unwrap(),
        )
        .await;
        assert_eq!(name, "response.reasoning_summary_text.done");
        assert_eq!(data["summary_index"], 0);
        assert_eq!(data["text"], "final thought");
    }

    #[tokio::test]
    async fn test_reasoning_summary_part_done() {
        let mut s = SseState::new();
        let (name, data) = parse_event(
            s.reasoning_summary_part_done("item_r1", 0, "final thought")
                .unwrap(),
        )
        .await;
        assert_eq!(name, "response.reasoning_summary_part.done");
        assert_eq!(data["summary_index"], 0);
        assert_eq!(data["part"]["type"], "summary_text");
        assert_eq!(data["part"]["text"], "final thought");
    }

    #[tokio::test]
    async fn test_function_call_arguments_delta() {
        let mut s = SseState::new();
        let (name, data) = parse_event(
            s.function_call_arguments_delta("fc_1", 0, r#"{"path":"/tmp"}"#)
                .unwrap(),
        )
        .await;
        assert_eq!(name, "response.function_call_arguments.delta");
        assert_eq!(data["item_id"], "fc_1");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["delta"], r#"{"path":"/tmp"}"#);
    }

    #[tokio::test]
    async fn test_function_call_arguments_done() {
        let mut s = SseState::new();
        let (name, data) = parse_event(
            s.function_call_arguments_done("fc_1", 0, r#"{"path":"/tmp"}"#)
                .unwrap(),
        )
        .await;
        assert_eq!(name, "response.function_call_arguments.done");
        assert_eq!(data["item_id"], "fc_1");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["arguments"], r#"{"path":"/tmp"}"#);
    }

    #[tokio::test]
    async fn test_output_item_done() {
        let mut s = SseState::new();
        let item = json!({"type": "message", "id": "item_1", "status": "completed"});
        let (name, data) = parse_event(s.output_item_done(0, item.clone()).unwrap()).await;
        assert_eq!(name, "response.output_item.done");
        assert_eq!(data["output_index"], 0);
        assert_eq!(data["item"], item);
    }

    #[tokio::test]
    async fn test_response_completed() {
        let mut s = SseState::new();
        let resp = json!({"id": "resp_1", "status": "completed"});
        let (name, data) = parse_event(s.response_completed(&resp).unwrap()).await;
        assert_eq!(name, "response.completed");
        assert_eq!(data["response"], resp);
    }

    #[tokio::test]
    async fn test_response_failed() {
        let mut s = SseState::new();
        let resp = json!({"id": "resp_1", "status": "failed", "error": {"code": "server_error"}});
        let (name, data) = parse_event(s.response_failed(&resp).unwrap()).await;
        assert_eq!(name, "response.failed");
        assert_eq!(data["response"], resp);
    }

    #[tokio::test]
    async fn test_multiple_output_indices() {
        let mut s = SseState::new();
        assert_eq!(s.alloc_output_index(), 0);
        assert_eq!(s.alloc_output_index(), 1);

        let (_, d) = parse_event(s.output_item_added(0, "i1", "message", json!({})).unwrap()).await;
        assert_eq!(d["output_index"], 0);

        assert_eq!(s.alloc_output_index(), 2);
        let (_, d) = parse_event(s.output_item_added(2, "i2", "message", json!({})).unwrap()).await;
        assert_eq!(d["output_index"], 2);
    }
}
