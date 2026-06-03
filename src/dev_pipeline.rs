use crate::accounts::{
    Account, AccountStore, DevPipelineToolMode, DevPipelineTriggerMode, EndpointConfig,
    EndpointKind,
};
use crate::anthropic;
use crate::handlers::validate_upstream;
use crate::types::{
    resolve_model, ChatMessage, ChatRequest, ChatResponse, ResponsesInput, ResponsesRequest,
};
use anyhow::{anyhow, bail, Result};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const DEV_PIPELINE_INTERNAL_METADATA: &str = "x_deecodex_dev_pipeline_internal";

#[derive(Clone, Debug)]
pub struct DevPipelineTrigger {
    pub command: String,
    pub task: String,
}

#[derive(Clone, Debug)]
pub struct DevPipelineStageTrace {
    pub role: &'static str,
    pub account_id: String,
    pub account_name: String,
    pub model: String,
    pub elapsed_ms: u128,
    pub output: String,
}

#[derive(Clone, Debug)]
pub struct DevPipelineOutput {
    pub final_text: String,
    pub final_model: String,
    pub traces: Vec<DevPipelineStageTrace>,
    pub elapsed_ms: u128,
}

#[derive(Clone)]
pub struct DevPipelineContext {
    pub client: Client,
    pub store: AccountStore,
    pub active_account: Account,
    pub active_endpoint_id: Option<String>,
    pub requested_model: String,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub chinese_thinking: bool,
}

pub fn internal_request(req: &ResponsesRequest) -> bool {
    req.metadata
        .as_ref()
        .and_then(|metadata| metadata.get(DEV_PIPELINE_INTERNAL_METADATA))
        .is_some_and(|value| value == "true")
}

pub fn detect_trigger(req: &ResponsesRequest, account: &Account) -> Option<DevPipelineTrigger> {
    if internal_request(req) || !account.dev_pipeline_enabled {
        return None;
    }

    let command = normalized_command(account);
    match account.dev_pipeline_trigger_mode {
        DevPipelineTriggerMode::Always => Some(DevPipelineTrigger {
            command,
            task: collect_request_text(&req.input),
        }),
        DevPipelineTriggerMode::Manual => {
            let text = collect_request_text(&req.input);
            let stripped = strip_manual_command(&text, &command)?;
            Some(DevPipelineTrigger {
                command,
                task: stripped,
            })
        }
    }
}

pub async fn run(
    trigger: DevPipelineTrigger,
    ctx: DevPipelineContext,
) -> Result<DevPipelineOutput> {
    let started = Instant::now();
    let active = ctx.active_account.clone();
    let plan = run_stage(
        &ctx,
        "architect",
        "方案设计",
        select_role_account(
            &ctx.store,
            &active,
            active.dev_pipeline_architect_account_id.as_deref(),
        )?,
        architect_prompt(&active, &trigger.task),
    )
    .await?;

    let implementation = run_stage(
        &ctx,
        "implementer",
        "实现填充",
        select_role_account(
            &ctx.store,
            &active,
            active.dev_pipeline_implementer_account_id.as_deref(),
        )?,
        implementer_prompt(&active, &trigger.task, &plan.output),
    )
    .await?;

    let reviewer = run_stage(
        &ctx,
        "reviewer",
        "验收收口",
        select_role_account(
            &ctx.store,
            &active,
            active.dev_pipeline_reviewer_account_id.as_deref(),
        )?,
        reviewer_prompt(&active, &trigger.task, &plan.output, &implementation.output),
    )
    .await?;

    let traces = vec![plan, implementation, reviewer.clone()];
    let mut final_text = reviewer.output;
    if active.dev_pipeline_show_trace {
        final_text = format!(
            "{}\n\n---\n开发协作编排摘要:\n{}",
            final_text,
            traces
                .iter()
                .map(|trace| format!(
                    "- {}: {} / {} / {}ms",
                    trace.role, trace.account_name, trace.model, trace.elapsed_ms
                ))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    info!(
        command = %trigger.command,
        active_account = %active.name,
        elapsed_ms = started.elapsed().as_millis(),
        "开发协作编排完成"
    );

    Ok(DevPipelineOutput {
        final_text,
        final_model: traces
            .last()
            .map(|trace| trace.model.clone())
            .unwrap_or_else(|| ctx.requested_model.clone()),
        traces,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

async fn run_stage(
    ctx: &DevPipelineContext,
    role: &'static str,
    label: &'static str,
    account: Account,
    prompt: String,
) -> Result<DevPipelineStageTrace> {
    let endpoint = select_endpoint(
        &account,
        &ctx.active_account,
        ctx.active_endpoint_id.as_deref(),
    )
    .ok_or_else(|| anyhow!("账号 '{}' 未配置可用端点", account.name))?;
    let model = stage_model(&ctx.requested_model, &account, &endpoint);
    let started = Instant::now();
    let output = match endpoint.kind {
        EndpointKind::OpenAiChat | EndpointKind::CustomChat => {
            call_chat_stage(ctx, &account, &endpoint, &model, label, &prompt).await?
        }
        EndpointKind::AnthropicMessages => {
            call_anthropic_stage(ctx, &account, &endpoint, &model, label, &prompt).await?
        }
        EndpointKind::OpenAiResponses
        | EndpointKind::CustomResponses
        | EndpointKind::CodexOfficial => {
            call_responses_stage(ctx, &account, &endpoint, &model, label, &prompt).await?
        }
    };

    let elapsed_ms = started.elapsed().as_millis();
    info!(
        role,
        label,
        account_id = %account.id,
        account_name = %account.name,
        model,
        elapsed_ms,
        chars = output.len(),
        "开发协作阶段完成"
    );
    Ok(DevPipelineStageTrace {
        role,
        account_id: account.id,
        account_name: account.name,
        model,
        elapsed_ms,
        output,
    })
}

fn stage_model(requested_model: &str, account: &Account, endpoint: &EndpointConfig) -> String {
    let endpoint_model = resolve_model(requested_model, &endpoint.model_map);
    if endpoint_model != requested_model {
        return endpoint_model;
    }
    let account_model = resolve_model(requested_model, &account.model_map);
    if account_model != requested_model {
        return account_model;
    }
    if endpoint
        .known_models
        .iter()
        .any(|model| model.trim() == requested_model)
    {
        return requested_model.to_string();
    }
    endpoint
        .known_models
        .iter()
        .map(|model| model.trim())
        .find(|model| !model.is_empty())
        .unwrap_or(requested_model)
        .to_string()
}

async fn call_chat_stage(
    ctx: &DevPipelineContext,
    account: &Account,
    endpoint: &EndpointConfig,
    model: &str,
    label: &str,
    prompt: &str,
) -> Result<String> {
    let upstream = validate_upstream(&endpoint.base_url)?;
    let url = format!("{}{}", join_base(&upstream), endpoint.effective_path());
    let chat_req = ChatRequest {
        model: model.to_string(),
        messages: stage_messages(ctx, label, prompt),
        tools: Vec::new(),
        temperature: ctx.temperature,
        top_p: ctx.top_p,
        max_tokens: ctx.max_output_tokens,
        stream: false,
        reasoning_effort: endpoint.reasoning_effort_override.clone(),
        thinking: endpoint.thinking_tokens.map(|budget| {
            json!({
                "type": "enabled",
                "budget_tokens": budget
            })
        }),
        reasoning_split: None,
        tool_choice: None,
        parallel_tool_calls: None,
        response_format: None,
        user: None,
        stream_options: None,
        web_search_options: None,
    };
    let value = send_json(
        &ctx.client,
        &url,
        Auth::Bearer(&account.api_key),
        &endpoint.custom_headers,
        endpoint.request_timeout_secs,
        endpoint.max_retries.unwrap_or(2) as usize,
        &chat_req,
    )
    .await?;
    let response: ChatResponse = serde_json::from_value(value)?;
    Ok(response
        .choices
        .first()
        .map(|choice| chat_message_text(&choice.message))
        .unwrap_or_default())
}

async fn call_anthropic_stage(
    ctx: &DevPipelineContext,
    account: &Account,
    endpoint: &EndpointConfig,
    model: &str,
    label: &str,
    prompt: &str,
) -> Result<String> {
    let upstream = validate_upstream(&endpoint.base_url)?;
    let url = format!("{}{}", join_base(&upstream), endpoint.effective_path());
    let chat_req = ChatRequest {
        model: model.to_string(),
        messages: stage_messages(ctx, label, prompt),
        tools: Vec::new(),
        temperature: ctx.temperature,
        top_p: ctx.top_p,
        max_tokens: ctx.max_output_tokens,
        stream: false,
        reasoning_effort: endpoint.reasoning_effort_override.clone(),
        thinking: None,
        reasoning_split: None,
        tool_choice: None,
        parallel_tool_calls: None,
        response_format: None,
        user: None,
        stream_options: None,
        web_search_options: None,
    };
    let body = anthropic::to_messages_body(&chat_req, endpoint.thinking_tokens);
    let value = send_json(
        &ctx.client,
        &url,
        Auth::Anthropic(&account.api_key),
        &endpoint.custom_headers,
        endpoint.request_timeout_secs,
        endpoint.max_retries.unwrap_or(2) as usize,
        &body,
    )
    .await?;
    let response = anthropic::response_to_chat(value);
    Ok(response
        .choices
        .first()
        .map(|choice| chat_message_text(&choice.message))
        .unwrap_or_default())
}

async fn call_responses_stage(
    ctx: &DevPipelineContext,
    account: &Account,
    endpoint: &EndpointConfig,
    model: &str,
    label: &str,
    prompt: &str,
) -> Result<String> {
    let upstream = validate_upstream(&endpoint.base_url)?;
    let url = format!("{}{}", join_base(&upstream), endpoint.effective_path());
    let mut body = json!({
        "model": model,
        "instructions": stage_system(ctx, label),
        "input": [{
            "role": "user",
            "content": [{"type": "input_text", "text": prompt}]
        }],
        "stream": false,
        "temperature": ctx.temperature,
        "top_p": ctx.top_p,
        "max_output_tokens": ctx.max_output_tokens,
        "metadata": {}
    });
    body["metadata"][DEV_PIPELINE_INTERNAL_METADATA] = json!("true");
    let value = send_json(
        &ctx.client,
        &url,
        Auth::Bearer(&account.api_key),
        &endpoint.custom_headers,
        endpoint.request_timeout_secs,
        endpoint.max_retries.unwrap_or(2) as usize,
        &body,
    )
    .await?;
    Ok(responses_output_text(&value))
}

enum Auth<'a> {
    Bearer(&'a str),
    Anthropic(&'a str),
}

async fn send_json<T: serde::Serialize + ?Sized>(
    client: &Client,
    url: &str,
    auth: Auth<'_>,
    custom_headers: &HashMap<String, String>,
    timeout_secs: Option<u64>,
    max_retries: usize,
    body: &T,
) -> Result<Value> {
    let mut builder = client.post(url).header("Content-Type", "application/json");
    match auth {
        Auth::Bearer(key) if !key.is_empty() => {
            builder = builder.bearer_auth(key);
        }
        Auth::Anthropic(key) if !key.is_empty() => {
            builder = builder
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        }
        Auth::Anthropic(_) => {
            builder = builder.header("anthropic-version", "2023-06-01");
        }
        Auth::Bearer(_) => {}
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
            .ok_or_else(|| anyhow!("无法克隆开发协作阶段请求"))?
            .json(body)
            .send()
            .await;
        match response {
            Ok(resp) if resp.status().is_success() => return Ok(resp.json::<Value>().await?),
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                bail!("开发协作阶段上游返回 HTTP {}: {}", status.as_u16(), text);
            }
            Err(err) if attempt < max_retries => {
                attempt += 1;
                warn!(attempt, max_retries, error = %err, "开发协作阶段请求失败，准备重试");
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                delay_ms *= 2;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

fn select_role_account(
    store: &AccountStore,
    active: &Account,
    role_id: Option<&str>,
) -> Result<Account> {
    let Some(id) = role_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(active.clone());
    };
    if id == "active" {
        return Ok(active.clone());
    }
    store
        .accounts
        .iter()
        .find(|account| account.id == id)
        .cloned()
        .ok_or_else(|| anyhow!("开发协作角色账号不存在: {id}"))
}

fn select_endpoint(
    account: &Account,
    active: &Account,
    active_endpoint_id: Option<&str>,
) -> Option<EndpointConfig> {
    if account.id == active.id {
        account.active_endpoint(active_endpoint_id).cloned()
    } else {
        account.endpoints.first().cloned()
    }
    .or_else(|| account.endpoints.first().cloned())
}

fn stage_messages(ctx: &DevPipelineContext, label: &str, prompt: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "system".into(),
            content: Some(Value::String(stage_system(ctx, label))),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
        ChatMessage {
            role: "user".into(),
            content: Some(Value::String(prompt.to_string())),
            reasoning_content: None,
            reasoning_details: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        },
    ]
}

fn stage_system(ctx: &DevPipelineContext, label: &str) -> String {
    let cn = if ctx.chinese_thinking {
        "所有分析、推理和输出必须使用简体中文。\n"
    } else {
        ""
    };
    format!(
        "{cn}你是 deecodex 开发协作编排的「{label}」阶段。你只处理当前阶段职责，输出要结构化、可交接、可验收。不要声称已经执行未被提供的工具或命令。"
    )
}

fn architect_prompt(active: &Account, task: &str) -> String {
    let extra = active.dev_pipeline_architect_instruction.trim();
    format!(
        "用户开发任务:\n{task}\n\n请完成方案设计阶段：给出目标拆解、影响范围、建议文件/模块、实现步骤、风险点和验收标准。不要写完整代码。\n{}",
        optional_extra(extra)
    )
}

fn implementer_prompt(active: &Account, task: &str, plan: &str) -> String {
    let extra = active.dev_pipeline_implementer_instruction.trim();
    format!(
        "用户开发任务:\n{task}\n\n方案设计阶段输出:\n{plan}\n\n请完成实现填充阶段。根据工具能力模式「{}」输出可落地的实现内容、补丁建议、关键代码片段、测试命令和自检结果。若缺少真实仓库上下文，请明确列出需要读取的文件或命令，不要编造已经修改文件。\n{}",
        tool_mode_label(&active.dev_pipeline_tool_mode),
        optional_extra(extra)
    )
}

fn reviewer_prompt(active: &Account, task: &str, plan: &str, implementation: &str) -> String {
    let extra = active.dev_pipeline_reviewer_instruction.trim();
    format!(
        "用户开发任务:\n{task}\n\n方案设计阶段输出:\n{plan}\n\n实现填充阶段输出:\n{implementation}\n\n请完成验收收口阶段：审查实现是否满足任务，指出必须修正的问题，给出最终交付说明、验证清单和剩余风险。最终答案要直接面向用户。\n{}",
        optional_extra(extra)
    )
}

fn optional_extra(extra: &str) -> String {
    if extra.is_empty() {
        String::new()
    } else {
        format!("\n角色附加指令:\n{extra}")
    }
}

fn tool_mode_label(mode: &DevPipelineToolMode) -> &'static str {
    match mode {
        DevPipelineToolMode::PatchOnly => "仅生成补丁",
        DevPipelineToolMode::ControlledTools => "受控工具执行",
        DevPipelineToolMode::FullAgent => "完整 Codex 执行能力",
    }
}

fn normalized_command(account: &Account) -> String {
    let command = account.dev_pipeline_command.trim();
    if command.is_empty() {
        "/dev-pipeline".into()
    } else {
        command.to_string()
    }
}

fn strip_manual_command(text: &str, command: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed == command {
        return Some(String::new());
    }
    trimmed.strip_prefix(command).map(|rest| {
        rest.trim_start_matches([' ', '\n', '\t', ':', '：'])
            .to_string()
    })
}

fn collect_request_text(input: &ResponsesInput) -> String {
    let mut chunks = Vec::new();
    match input {
        ResponsesInput::Text(text) => chunks.push(text.clone()),
        ResponsesInput::Messages(items) => {
            for item in items {
                collect_value_text(item, &mut chunks);
            }
        }
    }
    chunks.join("\n")
}

fn collect_value_text(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::String(text) => chunks.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_value_text(item, chunks);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                chunks.push(text.to_string());
            }
            if let Some(content) = map.get("content") {
                collect_value_text(content, chunks);
            }
        }
        _ => {}
    }
}

fn chat_message_text(message: &ChatMessage) -> String {
    match message.content.as_ref() {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn responses_output_text(value: &Value) -> String {
    value
        .get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .flat_map(|item| {
                    item.get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn join_base(url: &Url) -> String {
    let s = url.as_str();
    if s.ends_with('/') {
        s.to_string()
    } else {
        format!("{s}/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{DevPipelineToolMode, DevPipelineTriggerMode};

    fn account() -> Account {
        Account {
            id: "a1".into(),
            name: "主账号".into(),
            provider: "custom".into(),
            client_kind: Default::default(),
            client_surface: Default::default(),
            wire_protocol: Default::default(),
            upstream: "https://example.com/v1".into(),
            api_key: String::new(),
            auth_mode: Default::default(),
            default_model: String::new(),
            client_options: HashMap::new(),
            runtime_state: Default::default(),
            last_applied_at: None,
            last_check: None,
            model_map: HashMap::new(),
            vision_upstream: String::new(),
            vision_api_key: String::new(),
            vision_model: String::new(),
            vision_endpoint: String::new(),
            vision_enabled: false,
            from_codex_config: false,
            balance_url: String::new(),
            created_at: 0,
            updated_at: 0,
            context_window_override: None,
            reasoning_effort_override: None,
            thinking_tokens: None,
            custom_headers: HashMap::new(),
            provider_options: HashMap::new(),
            request_timeout_secs: None,
            max_retries: None,
            translate_enabled: true,
            capability_enabled: false,
            capability_account_id: None,
            dev_pipeline_enabled: true,
            dev_pipeline_trigger_mode: DevPipelineTriggerMode::Manual,
            dev_pipeline_command: "/dev-pipeline".into(),
            dev_pipeline_architect_account_id: None,
            dev_pipeline_implementer_account_id: None,
            dev_pipeline_reviewer_account_id: None,
            dev_pipeline_tool_mode: DevPipelineToolMode::ControlledTools,
            dev_pipeline_max_iterations: 3,
            dev_pipeline_show_trace: false,
            dev_pipeline_architect_instruction: String::new(),
            dev_pipeline_implementer_instruction: String::new(),
            dev_pipeline_reviewer_instruction: String::new(),
            endpoints: Vec::new(),
        }
    }

    fn req(text: &str) -> ResponsesRequest {
        ResponsesRequest {
            model: "gpt-5".into(),
            input: ResponsesInput::Text(text.into()),
            previous_response_id: None,
            tools: Vec::new(),
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
    fn manual_command_triggers_and_strips_prefix() {
        let trigger = detect_trigger(&req("/dev-pipeline 实现功能"), &account()).unwrap();
        assert_eq!(trigger.task, "实现功能");
    }

    #[test]
    fn plain_text_does_not_trigger_manual_pipeline() {
        assert!(detect_trigger(&req("实现功能"), &account()).is_none());
    }

    #[test]
    fn always_mode_triggers_without_command() {
        let mut account = account();
        account.dev_pipeline_trigger_mode = DevPipelineTriggerMode::Always;
        let trigger = detect_trigger(&req("实现功能"), &account).unwrap();
        assert_eq!(trigger.command, "/dev-pipeline");
        assert_eq!(trigger.task, "实现功能");
    }

    #[test]
    fn internal_metadata_prevents_recursive_pipeline() {
        let mut request = req("/dev-pipeline 实现功能");
        request.metadata = Some(HashMap::from([(
            DEV_PIPELINE_INTERNAL_METADATA.to_string(),
            "true".to_string(),
        )]));

        assert!(detect_trigger(&request, &account()).is_none());
    }
}
