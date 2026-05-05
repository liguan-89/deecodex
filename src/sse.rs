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

impl SseState {
    pub fn new() -> Self {
        Self { seq: 0, next_output_index: 0 }
    }

    pub fn alloc_output_index(&mut self) -> usize {
        let ix = self.next_output_index;
        self.next_output_index += 1;
        ix
    }

    pub fn response_created(&mut self, id: &str, model: &str) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.created").data(json!({
            "type": "response.created",
            "sequence_number": self.seq,
            "response": { "id": id, "status": "in_progress", "model": model }
        }).to_string()))
    }

    pub fn response_in_progress(&mut self, id: &str) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.in_progress").data(json!({
            "type": "response.in_progress",
            "sequence_number": self.seq,
            "response": { "id": id, "status": "in_progress" }
        }).to_string()))
    }

    pub fn output_item_added(
        &mut self, output_index: usize, item_id: &str,
        item_type: &str, extra: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        let mut item = json!({
            "type": item_type,
            "id": item_id,
            "status": "in_progress"
        });
        merge(&mut item, &extra);
        Ok(Event::default().event("response.output_item.added").data(json!({
            "type": "response.output_item.added",
            "sequence_number": self.seq,
            "output_index": output_index,
            "item": item
        }).to_string()))
    }

    pub fn content_part_added(
        &mut self, item_id: &str, output_index: usize,
        content_index: usize, part: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.content_part.added").data(json!({
            "type": "response.content_part.added",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "content_index": content_index,
            "part": part
        }).to_string()))
    }

    pub fn output_text_delta(
        &mut self, item_id: &str, output_index: usize,
        content_index: usize, delta: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.output_text.delta").data(json!({
            "type": "response.output_text.delta",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "content_index": content_index,
            "delta": delta
        }).to_string()))
    }

    pub fn output_text_done(
        &mut self, item_id: &str, output_index: usize,
        content_index: usize, text: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.output_text.done").data(json!({
            "type": "response.output_text.done",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "content_index": content_index,
            "text": text,
            "logprobs": []
        }).to_string()))
    }

    pub fn content_part_done(
        &mut self, item_id: &str, output_index: usize,
        content_index: usize, part: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.content_part.done").data(json!({
            "type": "response.content_part.done",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "content_index": content_index,
            "part": part
        }).to_string()))
    }

    pub fn reasoning_summary_part_added(
        &mut self, item_id: &str, output_index: usize,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.reasoning_summary_part.added").data(json!({
            "type": "response.reasoning_summary_part.added",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "summary_index": 0,
            "part": { "type": "summary_text", "text": "" }
        }).to_string()))
    }

    pub fn reasoning_summary_text_delta(
        &mut self, item_id: &str, output_index: usize, delta: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.reasoning_summary_text.delta").data(json!({
            "type": "response.reasoning_summary_text.delta",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "summary_index": 0,
            "delta": delta
        }).to_string()))
    }

    pub fn reasoning_summary_text_done(
        &mut self, item_id: &str, output_index: usize, text: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.reasoning_summary_text.done").data(json!({
            "type": "response.reasoning_summary_text.done",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "summary_index": 0,
            "text": text
        }).to_string()))
    }

    pub fn reasoning_summary_part_done(
        &mut self, item_id: &str, output_index: usize, text: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.reasoning_summary_part.done").data(json!({
            "type": "response.reasoning_summary_part.done",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "summary_index": 0,
            "part": { "type": "summary_text", "text": text }
        }).to_string()))
    }

    pub fn function_call_arguments_delta(
        &mut self, item_id: &str, output_index: usize, delta: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.function_call_arguments.delta").data(json!({
            "type": "response.function_call_arguments.delta",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "delta": delta
        }).to_string()))
    }

    pub fn function_call_arguments_done(
        &mut self, item_id: &str, output_index: usize, arguments: &str,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.function_call_arguments.done").data(json!({
            "type": "response.function_call_arguments.done",
            "sequence_number": self.seq,
            "item_id": item_id,
            "output_index": output_index,
            "arguments": arguments
        }).to_string()))
    }

    pub fn output_item_done(
        &mut self, output_index: usize, item: Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.output_item.done").data(json!({
            "type": "response.output_item.done",
            "sequence_number": self.seq,
            "output_index": output_index,
            "item": item
        }).to_string()))
    }

    pub fn response_completed(
        &mut self, response: &Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.completed").data(json!({
            "type": "response.completed",
            "sequence_number": self.seq,
            "response": response
        }).to_string()))
    }

    pub fn response_failed(
        &mut self, response: &Value,
    ) -> Result<Event, Infallible> {
        self.seq += 1;
        Ok(Event::default().event("response.failed").data(json!({
            "type": "response.failed",
            "sequence_number": self.seq,
            "response": response
        }).to_string()))
    }
}

fn merge(target: &mut Value, source: &Value) {
    match (target, source) {
        (Value::Object(t), Value::Object(s)) => {
            for (k, v) in s {
                t.insert(k.clone(), v.clone());
            }
        }
        _ => {}
    }
}
