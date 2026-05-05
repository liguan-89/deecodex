//! Vendor compatibility tests for reasoning_content round-trip.
//!
//! These tests simulate the exact request patterns observed from Codex CLI
//! when talking to DeepSeek V4 Pro (and similar thinking models) through the
//! relay.

use deecodex::session::SessionStore;
use deecodex::translate::to_chat_request;
use deecodex::types::*;
use serde_json::json;
use std::collections::HashMap;

fn empty_map() -> ModelMap {
    HashMap::new()
}

fn base_req(input: ResponsesInput) -> ResponsesRequest {
    ResponsesRequest {
        model: "deepseek-v4-pro".into(),
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

fn assistant_msg(content: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: Some(content.into()),
        reasoning_content: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
    }
}

fn assistant_msg_with_tool_calls(content: &str, tool_calls: Vec<serde_json::Value>) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: Some(content.into()),
        reasoning_content: None,
        tool_calls: Some(tool_calls),
        tool_call_id: None,
        name: None,
    }
}

#[test]
fn test_deepseek_v4_pro_reasoning_roundtrip_text_only() {
    let store = SessionStore::new();
    let assistant = assistant_msg("Let me analyze this");
    store.store_turn_reasoning(
        &[],
        &assistant,
        "<think>analyzing the problem...</think>".into(),
    );

    let req = base_req(ResponsesInput::Messages(vec![
        json!({"type": "message", "role": "user", "content": "Research task prompt"}),
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Let me analyze this"}]}),
        json!({"type": "message", "role": "user", "content": "Continue"}),
    ]));

    let chat = to_chat_request(&req, vec![], &store, &empty_map(), false);

    assert_eq!(chat.chat.messages.len(), 3);
    assert_eq!(chat.chat.messages[1].role, "assistant");
    assert_eq!(
        chat.chat.messages[1]
            .content
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("Let me analyze this")
    );
    assert_eq!(
        chat.chat.messages[1].reasoning_content.as_deref(),
        Some("<think>analyzing the problem...</think>"),
        "assistant text message should have reasoning_content recovered"
    );
}

#[test]
fn test_deepseek_v4_pro_reasoning_roundtrip_with_tool_calls() {
    let store = SessionStore::new();

    let assistant = assistant_msg_with_tool_calls(
        "Let me check",
        vec![json!({
            "id": "call_abc",
            "type": "function",
            "function": {"name": "exec_command", "arguments": "{\"cmd\": \"ls\"}"}
        })],
    );
    store.store_turn_reasoning(&[], &assistant, "<think>need to read files</think>".into());

    let req = base_req(ResponsesInput::Messages(vec![
        json!({"type": "message", "role": "user", "content": "Prompt"}),
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Let me check"}]}),
        json!({"type": "function_call", "call_id": "call_abc", "name": "exec_command", "arguments": "{\"cmd\": \"ls\"}"}),
        json!({"type": "function_call_output", "call_id": "call_abc", "output": "file1.py\nfile2.py"}),
        json!({"type": "message", "role": "user", "content": "What next?"}),
    ]));

    let chat = to_chat_request(&req, vec![], &store, &empty_map(), false);

    assert_eq!(chat.chat.messages.len(), 5);

    assert_eq!(chat.chat.messages[1].role, "assistant");
    assert_eq!(
        chat.chat.messages[1]
            .content
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("Let me check")
    );
    assert_eq!(
        chat.chat.messages[1].reasoning_content.as_deref(),
        Some("<think>need to read files</think>"),
        "assistant text message should have reasoning_content"
    );

    assert_eq!(chat.chat.messages[2].role, "assistant");
    assert!(chat.chat.messages[2].tool_calls.is_some());
    assert_eq!(
        chat.chat.messages[2].reasoning_content.as_deref(),
        Some("<think>need to read files</think>"),
        "assistant tool-call message should have reasoning_content via call_id fallback"
    );
}

#[test]
fn test_deepseek_v4_pro_multi_turn_reasoning() {
    let store = SessionStore::new();

    let assistant1 = assistant_msg("Step 1 analysis");
    store.store_turn_reasoning(
        &[],
        &assistant1,
        "<think>first pass thinking</think>".into(),
    );

    let assistant2 = assistant_msg("Step 2 deeper look");
    store.store_turn_reasoning(
        &[],
        &assistant2,
        "<think>second pass thinking</think>".into(),
    );

    let req = base_req(ResponsesInput::Messages(vec![
        json!({"type": "message", "role": "user", "content": "Start research"}),
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Step 1 analysis"}]}),
        json!({"type": "message", "role": "user", "content": "Go deeper"}),
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Step 2 deeper look"}]}),
        json!({"type": "message", "role": "user", "content": "Finalize"}),
    ]));

    let chat = to_chat_request(&req, vec![], &store, &empty_map(), false);

    assert_eq!(chat.chat.messages.len(), 5);

    assert_eq!(chat.chat.messages[1].role, "assistant");
    assert_eq!(
        chat.chat.messages[1]
            .content
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("Step 1 analysis")
    );
    assert_eq!(
        chat.chat.messages[1].reasoning_content.as_deref(),
        Some("<think>first pass thinking</think>"),
    );

    assert_eq!(chat.chat.messages[3].role, "assistant");
    assert_eq!(
        chat.chat.messages[3]
            .content
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("Step 2 deeper look")
    );
    assert_eq!(
        chat.chat.messages[3].reasoning_content.as_deref(),
        Some("<think>second pass thinking</think>"),
    );
}

#[test]
fn test_non_thinking_model_no_reasoning_content() {
    let store = SessionStore::new();

    let req = base_req(ResponsesInput::Messages(vec![
        json!({"type": "message", "role": "user", "content": "Hello"}),
        json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "Hi there!"}]}),
        json!({"type": "message", "role": "user", "content": "Thanks"}),
    ]));

    let chat = to_chat_request(&req, vec![], &store, &empty_map(), false);

    assert_eq!(chat.chat.messages.len(), 3);
    assert_eq!(chat.chat.messages[1].role, "assistant");
    assert_eq!(
        chat.chat.messages[1]
            .content
            .as_ref()
            .and_then(|v| v.as_str()),
        Some("Hi there!")
    );
    assert!(chat.chat.messages[1].reasoning_content.is_none());
}

#[test]
fn test_kimi_k2_6_reasoning_via_call_id() {
    let store = SessionStore::new();

    store.store_reasoning("call_xyz".into(), "<think>kimi is thinking</think>".into());

    let req = base_req(ResponsesInput::Messages(vec![
        json!({"type": "message", "role": "user", "content": "Do something"}),
        json!({"type": "function_call", "call_id": "call_xyz", "name": "run_cmd", "arguments": "{\"cmd\": \"pwd\"}"}),
        json!({"type": "function_call_output", "call_id": "call_xyz", "output": "/home/user"}),
        json!({"type": "message", "role": "user", "content": "Continue"}),
    ]));

    let chat = to_chat_request(&req, vec![], &store, &empty_map(), false);

    assert_eq!(chat.chat.messages.len(), 4);
    assert_eq!(chat.chat.messages[1].role, "assistant");
    assert!(chat.chat.messages[1].tool_calls.is_some());
    assert_eq!(
        chat.chat.messages[1].reasoning_content.as_deref(),
        Some("<think>kimi is thinking</think>"),
    );
}
