use crate::accounts::Account;
use crate::types::{ChatMessage, ResponsesInput, ResponsesRequest};
use reqwest::{Client, Url};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const CAPABILITY_METADATA_KEY: &str = "x_deecodex_capability_observer";
const CAPABILITY_MAX_OBSERVATION_CHARS: usize = 12_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityTrigger {
    pub reasons: Vec<String>,
}

impl CapabilityTrigger {
    pub fn label(&self) -> String {
        self.reasons.join(",")
    }
}

#[derive(Clone)]
pub struct CapabilityContext {
    pub client: Client,
    pub upstream: Url,
    pub endpoint_path: String,
    pub api_key: String,
    pub custom_headers: HashMap<String, String>,
    pub timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    pub model_map: HashMap<String, String>,
}

pub async fn maybe_observe(
    req: &ResponsesRequest,
    raw_body: &[u8],
    main_account: &Account,
    helper_account: Option<Account>,
    context: CapabilityContext,
) -> Option<ChatMessage> {
    let trigger = detect_trigger(req)?;
    if capability_observer_request(req) {
        return None;
    }
    if !main_account.capability_enabled {
        return None;
    }

    let Some(helper) = helper_account else {
        warn!(
            account_id = %main_account.id,
            account_name = %main_account.name,
            reason = %trigger.label(),
            "能力补全已触发，但未找到能力账号，回退主模型"
        );
        return None;
    };

    if helper.id == main_account.id {
        warn!(
            account_id = %main_account.id,
            account_name = %main_account.name,
            "能力补全账号指向自身，回退主模型"
        );
        return None;
    }

    match observe_with_helper(req, raw_body, main_account, &helper, &trigger, context).await {
        Ok(Some(text)) => {
            let guidance = main_model_capability_guidance(&trigger);
            info!(
                main_account = %main_account.name,
                helper_account = %helper.name,
                reason = %trigger.label(),
                chars = text.len(),
                "能力补全观察已注入主模型"
            );
            Some(ChatMessage {
                role: "system".into(),
                content: Some(Value::String(format!(
                    "【deecodex 能力通道观察结果】\n{text}{guidance}\n\n请把以上观察作为当前会话上下文的一部分。最终回答仍由你完成；不要声称自己直接执行了这些工具。"
                ))),
                reasoning_content: None,
                tool_calls: None,
                tool_call_id: None,
                name: None,
            })
        }
        Ok(None) => None,
        Err(err) => {
            warn!(
                main_account = %main_account.name,
                helper_account = %helper.name,
                reason = %trigger.label(),
                error = %err,
                "能力补全观察失败，回退主模型"
            );
            None
        }
    }
}

pub fn detect_trigger(req: &ResponsesRequest) -> Option<CapabilityTrigger> {
    let mut reasons = Vec::new();
    if request_has_images(req) {
        reasons.push("image".to_string());
    }
    collect_input_reasons(&req.input, &mut reasons);
    for tool in &req.tools {
        collect_tool_reasons(tool, &mut reasons);
    }
    reasons.sort();
    reasons.dedup();
    if reasons.is_empty() {
        None
    } else {
        Some(CapabilityTrigger { reasons })
    }
}

pub fn capability_observer_request(req: &ResponsesRequest) -> bool {
    req.metadata
        .as_ref()
        .and_then(|metadata| metadata.get(CAPABILITY_METADATA_KEY))
        .is_some_and(|value| value == "true")
}

async fn observe_with_helper(
    req: &ResponsesRequest,
    raw_body: &[u8],
    main_account: &Account,
    _helper: &Account,
    trigger: &CapabilityTrigger,
    context: CapabilityContext,
) -> anyhow::Result<Option<String>> {
    let model = context
        .model_map
        .get(&req.model)
        .cloned()
        .unwrap_or_else(|| req.model.clone());
    let body = build_native_responses_body(
        raw_body,
        &model,
        &observer_instructions(main_account, trigger),
    )?;

    let url = format!(
        "{}{}",
        join_base(&context.upstream),
        context.endpoint_path.trim_start_matches('/')
    );
    let start = Instant::now();
    let response = send_responses_request(
        &context.client,
        &url,
        &context.api_key,
        &context.custom_headers,
        context.timeout_secs,
        context.max_retries.unwrap_or(1) as usize,
        &body,
    )
    .await?;
    let elapsed_ms = start.elapsed().as_millis();

    let observation = summarize_native_response(&response, elapsed_ms);
    if observation.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(observation))
    }
}

async fn send_responses_request(
    client: &Client,
    url: &str,
    api_key: &str,
    custom_headers: &HashMap<String, String>,
    timeout_secs: Option<u64>,
    max_retries: usize,
    body: &Value,
) -> anyhow::Result<Value> {
    let mut builder = client.post(url).header("Content-Type", "application/json");
    if !api_key.is_empty() {
        builder = builder.bearer_auth(api_key);
    }
    for (k, v) in custom_headers {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v),
        ) {
            builder = builder.header(name, value);
        }
    }
    if let Some(secs) = timeout_secs {
        builder = builder.timeout(Duration::from_secs(secs.max(1)));
    }

    let mut attempt = 0;
    let mut delay_ms = 300;
    loop {
        let response = builder
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("无法克隆能力通道请求"))?
            .json(body)
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => return Ok(resp.json::<Value>().await?),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("能力账号上游返回 HTTP {}: {}", status.as_u16(), body);
            }
            Err(err) if attempt < max_retries => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                delay_ms *= 2;
                warn!(attempt, max_retries, error = %err, "能力通道请求失败，准备重试");
            }
            Err(err) => return Err(err.into()),
        }
    }
}

fn build_native_responses_body(
    raw_body: &[u8],
    model: &str,
    instructions: &str,
) -> anyhow::Result<Value> {
    let mut body: Value = serde_json::from_slice(raw_body)?;
    let Some(map) = body.as_object_mut() else {
        anyhow::bail!("能力通道原始 Responses 请求不是 JSON object");
    };
    map.insert("model".into(), Value::String(model.to_string()));
    map.insert(
        "instructions".into(),
        Value::String(instructions.to_string()),
    );
    if map.get("stream").and_then(Value::as_bool) == Some(true) {
        map.insert("stream".into(), Value::Bool(false));
    }
    map.entry("max_output_tokens")
        .or_insert_with(|| Value::from(2048));
    map.remove("system");
    if let Some(metadata) = map.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove(CAPABILITY_METADATA_KEY);
        if metadata.is_empty() {
            map.remove("metadata");
        }
    }
    Ok(body)
}

fn observer_instructions(main_account: &Account, trigger: &CapabilityTrigger) -> String {
    format!(
        "你是 deecodex 的原生 Responses 能力账号。主推理账号是 '{}'，触发原因是 {}。请使用 OpenAI/Codex 原生能力完成必要的 computer_use、MCP、浏览器或多模态观察，只返回可供主模型继续回答的事实结果。不要展开完整推理。",
        main_account.name,
        trigger.label()
    )
}

fn summarize_native_response(response: &Value, elapsed_ms: u128) -> String {
    let mut lines = vec![format!("能力通道耗时: {elapsed_ms}ms")];
    if let Some(id) = response.get("id").and_then(Value::as_str) {
        lines.push(format!("原生 Responses ID: {id}"));
    }
    if let Some(status) = response.get("status").and_then(Value::as_str) {
        lines.push(format!("原生 Responses 状态: {status}"));
    }
    if let Some(items) = response.get("output").and_then(Value::as_array) {
        for item in items {
            let item_type = item
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            match item_type {
                "message" => {
                    let text = response_message_text(item);
                    if !text.is_empty() {
                        lines.push(format!("观察说明: {text}"));
                    }
                }
                "mcp_tool_call" => lines.push(format!(
                    "MCP 调用: server={} tool={} args={}",
                    item.get("server_label")
                        .and_then(Value::as_str)
                        .unwrap_or("remote_mcp"),
                    item.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    compact_json(item.get("arguments").unwrap_or(&Value::Null))
                )),
                "mcp_tool_call_output" => lines.push(format!(
                    "MCP 结果: status={} output={}",
                    item.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    compact_json(item.get("output").unwrap_or(&Value::Null))
                )),
                "computer_call" => lines.push(format!(
                    "Computer 调用: action={}",
                    compact_json(item.get("action").unwrap_or(&Value::Null))
                )),
                "computer_call_output" => lines.push(format!(
                    "Computer 结果: status={} output={}",
                    item.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    compact_json(item.get("output").unwrap_or(&Value::Null))
                )),
                "function_call" => lines.push(format!(
                    "函数调用: name={} args={}",
                    item.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    compact_json(item.get("arguments").unwrap_or(&Value::Null))
                )),
                _ => {}
            }
        }
    } else {
        lines.push(format!("原生 Responses 返回: {}", compact_json(response)));
    }
    truncate_observation(lines.join("\n"))
}

fn response_message_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn compact_json(value: &Value) -> String {
    let mut text = if let Some(s) = value.as_str() {
        s.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    };
    text = text.replace("data:image/", "[image:data:image/");
    if text.len() > 2000 {
        text.truncate(2000);
        text.push_str("...[truncated]");
    }
    text
}

fn truncate_observation(mut text: String) -> String {
    if text.len() > CAPABILITY_MAX_OBSERVATION_CHARS {
        text.truncate(CAPABILITY_MAX_OBSERVATION_CHARS);
        text.push_str("\n...[能力通道观察过长，已截断]");
    }
    text
}

fn join_base(url: &Url) -> String {
    let s = url.as_str();
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

fn collect_tool_reasons(tool: &Value, reasons: &mut Vec<String>) {
    let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or("");
    match tool_type {
        "computer_use" | "computer_use_preview" => reasons.push("computer".into()),
        "mcp" | "remote_mcp" => reasons.push("mcp".into()),
        "browser" | "browser_use" | "web_browser" => reasons.push("browser".into()),
        "function" => {
            let name = tool
                .get("name")
                .or_else(|| tool.get("function").and_then(|f| f.get("name")))
                .and_then(Value::as_str)
                .unwrap_or("");
            if looks_like_plugin_or_browser_tool(name) {
                reasons.push("plugin".into());
            }
        }
        "multi_tool_use.parallel" => reasons.push("plugin".into()),
        _ => {
            let name = tool
                .get("name")
                .or_else(|| tool.get("function").and_then(|f| f.get("name")))
                .and_then(Value::as_str)
                .unwrap_or("");
            if looks_like_plugin_or_browser_tool(name) {
                reasons.push("plugin".into());
            }
        }
    }
}

fn collect_input_reasons(input: &ResponsesInput, reasons: &mut Vec<String>) {
    match input {
        ResponsesInput::Text(text) => collect_text_reasons(text, reasons),
        ResponsesInput::Messages(items) => {
            for item in items {
                collect_value_text_reasons(item, reasons);
            }
        }
    }
}

fn collect_value_text_reasons(value: &Value, reasons: &mut Vec<String>) {
    match value {
        Value::String(text) => collect_text_reasons(text, reasons),
        Value::Array(items) => {
            for item in items {
                collect_value_text_reasons(item, reasons);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_value_text_reasons(value, reasons);
            }
        }
        _ => {}
    }
}

fn collect_text_reasons(text: &str, reasons: &mut Vec<String>) {
    let lower = text.to_ascii_lowercase();
    let has_plugin_uri = lower.contains("plugin://") || lower.contains("app://");
    let mentions_computer = lower.contains("computer-use")
        || lower.contains("computer_use")
        || lower.contains("computer use")
        || text.contains("@电脑")
        || text.contains("电脑");
    let mentions_browser = lower.contains("browser")
        || lower.contains("chrome")
        || text.contains("@浏览器")
        || text.contains("浏览器");
    let mentions_mcp = lower.contains("mcp://") || lower.contains("mcp__") || lower.contains("mcp");

    if has_plugin_uri {
        reasons.push("plugin".into());
    }
    if mentions_computer {
        reasons.push("computer".into());
        if has_plugin_uri {
            reasons.push("plugin".into());
        }
    }
    if mentions_browser {
        reasons.push("browser".into());
        if has_plugin_uri {
            reasons.push("plugin".into());
        }
    }
    if mentions_mcp {
        reasons.push("mcp".into());
    }
}

fn main_model_capability_guidance(trigger: &CapabilityTrigger) -> String {
    let has_computer_plugin = trigger.reasons.iter().any(|reason| reason == "computer")
        && trigger.reasons.iter().any(|reason| reason == "plugin");
    if !has_computer_plugin {
        return String::new();
    }

    "\n\n【deecodex Computer Use 提醒】Computer Use 已由原生 Responses 能力账号接管。主模型不要调用 computer-use、local_mcp_call、list_mcp_resources、read_mcp_resource 或 resources/list；只基于能力通道返回的观察结果继续回答。".into()
}

fn looks_like_plugin_or_browser_tool(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("browser")
        || lower.contains("chrome")
        || lower.contains("computer")
        || lower.contains("plugin")
        || lower.contains("mcp")
        || lower.contains("app__")
}

fn request_has_images(req: &ResponsesRequest) -> bool {
    match &req.input {
        ResponsesInput::Text(text) => text.contains("data:image/"),
        ResponsesInput::Messages(items) => items.iter().any(value_has_image),
    }
}

fn value_has_image(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains("data:image/"),
        Value::Array(items) => items.iter().any(value_has_image),
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            typ == "image_url"
                || typ == "input_image"
                || map.get("image_url").is_some()
                || map.values().any(value_has_image)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_req() -> ResponsesRequest {
        ResponsesRequest {
            model: "gpt-5.5".into(),
            input: ResponsesInput::Text("hello".into()),
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
    fn plain_text_does_not_trigger_capability() {
        assert_eq!(detect_trigger(&base_req()), None);
    }

    #[test]
    fn image_triggers_capability() {
        let mut req = base_req();
        req.input = ResponsesInput::Messages(vec![json!({
            "role": "user",
            "content": [{"type":"input_image","image_url":"data:image/png;base64,abc"}]
        })]);
        assert_eq!(detect_trigger(&req).unwrap().reasons, vec!["image"]);
    }

    #[test]
    fn computer_and_mcp_tools_trigger_capability() {
        let mut req = base_req();
        req.tools = vec![
            json!({"type":"computer_use", "display":"browser"}),
            json!({"type":"mcp", "server_label":"filesystem"}),
        ];
        assert_eq!(
            detect_trigger(&req).unwrap().reasons,
            vec!["computer", "mcp"]
        );
    }

    #[test]
    fn browser_and_plugin_tools_trigger_capability() {
        let mut req = base_req();
        req.tools = vec![
            json!({"type":"browser"}),
            json!({"type":"function", "name":"app__gmail_search"}),
            json!({"type":"multi_tool_use.parallel"}),
        ];
        assert_eq!(
            detect_trigger(&req).unwrap().reasons,
            vec!["browser", "plugin"]
        );
    }

    #[test]
    fn computer_plugin_mention_triggers_capability() {
        let mut req = base_req();
        req.input =
            ResponsesInput::Text("[@电脑](plugin://computer-use@openai-bundled) 打开抖音".into());

        assert_eq!(
            detect_trigger(&req).unwrap().reasons,
            vec!["computer", "plugin"]
        );
    }

    #[test]
    fn computer_plugin_mention_inside_message_triggers_capability() {
        let mut req = base_req();
        req.input = ResponsesInput::Messages(vec![json!({
            "role": "user",
            "content": [{"type":"input_text","text":"[@电脑](plugin://computer-use@openai-bundled) 打开抖音"}]
        })]);

        assert_eq!(
            detect_trigger(&req).unwrap().reasons,
            vec!["computer", "plugin"]
        );
    }

    #[test]
    fn native_observer_keeps_computer_plugin_request_unchanged() {
        let mut req = base_req();
        req.input =
            ResponsesInput::Text("[@电脑](plugin://computer-use@openai-bundled) 打开抖音".into());
        req.tools = vec![json!({"type":"computer_use_preview", "display_width":1024})];
        req.metadata
            .get_or_insert_with(Default::default)
            .insert(CAPABILITY_METADATA_KEY.into(), "true".into());

        assert!(capability_observer_request(&req));
        assert_eq!(req.tools.len(), 1);
        assert_eq!(
            req.tools[0].get("type").and_then(Value::as_str),
            Some("computer_use_preview")
        );
    }

    #[test]
    fn summarize_native_response_extracts_message_and_tool_items() {
        let response = json!({
            "id": "resp_capability",
            "status": "completed",
            "output": [
                {"type":"computer_call", "action":{"type":"click","x":1,"y":2}},
                {"type":"message", "content":[{"type":"output_text","text":"已打开抖音并播放第一个视频"}]}
            ]
        });

        let summary = summarize_native_response(&response, 42);

        assert!(summary.contains("原生 Responses ID: resp_capability"));
        assert!(summary.contains("Computer 调用"));
        assert!(summary.contains("已打开抖音并播放第一个视频"));
    }

    #[test]
    fn build_native_responses_body_patches_raw_body() {
        let raw_body = json!({
            "model": "gpt-5",
            "input": "[@电脑](plugin://computer-use@openai-bundled) 打开抖音",
            "tools": [{"type":"computer_use_preview", "display_width":1024}],
            "stream": true,
            "system": "不要透传",
            "metadata": {CAPABILITY_METADATA_KEY: "true"}
        })
        .to_string();

        let body = build_native_responses_body(raw_body.as_bytes(), "gpt-4.1", "observe").unwrap();

        assert_eq!(body["model"], "gpt-4.1");
        assert_eq!(body["instructions"], "observe");
        assert_eq!(body["tools"][0]["type"], "computer_use_preview");
        assert_eq!(body["max_output_tokens"], 2048);
        assert_eq!(body["stream"], false);
        assert!(body.get("system").is_none());
        assert!(body.get("metadata").is_none());
        assert!(body.get("background").is_none());
        assert!(body.get("store").is_none());
    }

    #[test]
    fn observer_metadata_prevents_recursion() {
        let mut req = base_req();
        req.metadata = Some(HashMap::from([(
            CAPABILITY_METADATA_KEY.into(),
            "true".into(),
        )]));
        assert!(capability_observer_request(&req));
    }
}
