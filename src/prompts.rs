use crate::types::{ResponsesInput, ResponsesRequest};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct PromptRegistry {
    root: PathBuf,
}

#[derive(Debug)]
pub struct PromptError {
    status: StatusCode,
    param: &'static str,
    code: &'static str,
    message: String,
}

#[derive(Debug, Deserialize, Default)]
struct PromptRef {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    variables: HashMap<String, Value>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    input_prefix: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PromptFile {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    input_prefix: Option<String>,
    #[serde(default)]
    metadata: Option<Value>,
}

#[derive(Debug)]
struct ResolvedPrompt {
    id: Option<String>,
    version: Option<String>,
    instructions: Option<String>,
    input_prefix: Option<String>,
    metadata: Option<Value>,
}

impl PromptRegistry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[allow(dead_code)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn apply_to_request(&self, req: &mut ResponsesRequest) -> Result<(), PromptError> {
        let Some(prompt_value) = req.prompt.take() else {
            return Ok(());
        };
        let resolved = self.resolve(prompt_value)?;
        if let Some(instructions) = non_empty(resolved.instructions) {
            req.instructions = Some(join_prompt_text(
                Some(instructions),
                req.instructions.take().or_else(|| req.system.take()),
            ));
        }
        if let Some(input_prefix) = non_empty(resolved.input_prefix) {
            prepend_input_prefix(&mut req.input, &input_prefix);
        }
        if resolved.id.is_some() || resolved.metadata.is_some() {
            let metadata = req.metadata.get_or_insert_with(HashMap::new);
            if let Some(prompt_metadata) = resolved.metadata {
                metadata.insert("prompt_metadata".to_string(), prompt_metadata.to_string());
            }
            let Some(id) = resolved.id else {
                return Ok(());
            };
            metadata.insert("prompt_id".to_string(), id);
            if let Some(version) = resolved.version {
                metadata.insert("prompt_version".to_string(), version);
            }
        }
        Ok(())
    }

    pub fn list_prompts(&self) -> Value {
        let mut data = Vec::new();
        let Ok(entries) = fs::read_dir(&self.root) else {
            return json!({
                "object": "list",
                "data": data,
                "root": self.root.display().to_string()
            });
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ext) = path.extension().and_then(|v| v.to_str()) else {
                continue;
            };
            if ext != "json" && ext != "md" {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|v| v.to_str()) else {
                continue;
            };
            data.push(json!({
                "id": stem,
                "object": "prompt",
                "filename": path.file_name().and_then(|v| v.to_str()).unwrap_or(stem),
                "format": ext
            }));
        }
        data.sort_by(|a, b| {
            a.get("id")
                .and_then(Value::as_str)
                .cmp(&b.get("id").and_then(Value::as_str))
        });
        json!({
            "object": "list",
            "data": data,
            "root": self.root.display().to_string()
        })
    }

    pub fn retrieve_prompt(&self, id: &str) -> Result<Value, PromptError> {
        validate_key(id, "prompt id")?;
        let path = self.find_prompt_file(id, None).ok_or_else(|| {
            PromptError::not_found("prompt", format!("No prompt found with id {id}"))
        })?;
        let filename = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(id)
            .to_string();
        let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
        let raw = fs::read_to_string(&path)
            .map_err(|e| PromptError::internal(format!("Failed to read prompt {id}: {e}")))?;
        let spec = if ext == "json" {
            let file: PromptFile = serde_json::from_str(&raw).map_err(|e| {
                PromptError::invalid(
                    "prompt",
                    format!("Prompt {id} JSON is invalid: {e}"),
                    "invalid_prompt",
                )
            })?;
            json!({
                "id": file.id.unwrap_or_else(|| id.to_string()),
                "object": "prompt",
                "version": file.version,
                "instructions": file.instructions,
                "input_prefix": file.input_prefix,
                "metadata": file.metadata.unwrap_or_else(|| json!({})),
                "filename": filename,
                "format": ext
            })
        } else {
            json!({
                "id": id,
                "object": "prompt",
                "instructions": raw,
                "filename": filename,
                "format": ext
            })
        };
        Ok(spec)
    }

    fn resolve(&self, value: Value) -> Result<ResolvedPrompt, PromptError> {
        let prompt_ref = match value {
            Value::String(id) => PromptRef {
                id: Some(id),
                ..PromptRef::default()
            },
            Value::Object(_) => serde_json::from_value::<PromptRef>(value).map_err(|e| {
                PromptError::invalid(
                    "prompt",
                    format!("Invalid prompt object: {e}"),
                    "invalid_prompt",
                )
            })?,
            _ => {
                return Err(PromptError::invalid(
                    "prompt",
                    "prompt must be a string id or an object",
                    "invalid_prompt",
                ))
            }
        };

        let mut resolved = if let Some(id) = prompt_ref.id.as_deref() {
            validate_key(id, "prompt id")?;
            if let Some(version) = prompt_ref.version.as_deref() {
                validate_key(version, "prompt version")?;
            }
            self.load_from_disk(id, prompt_ref.version.as_deref())?
        } else {
            ResolvedPrompt {
                id: None,
                version: prompt_ref.version.clone(),
                instructions: prompt_ref.instructions.clone(),
                input_prefix: prompt_ref.input_prefix.clone(),
                metadata: None,
            }
        };

        if prompt_ref.instructions.is_some() {
            resolved.instructions = prompt_ref.instructions;
        }
        if prompt_ref.input_prefix.is_some() {
            resolved.input_prefix = prompt_ref.input_prefix;
        }

        resolved.instructions = resolved
            .instructions
            .map(|text| render_variables(&text, &prompt_ref.variables));
        resolved.input_prefix = resolved
            .input_prefix
            .map(|text| render_variables(&text, &prompt_ref.variables));

        if resolved
            .instructions
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
            && resolved
                .input_prefix
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        {
            return Err(PromptError::invalid(
                "prompt",
                "prompt must resolve to instructions or input_prefix",
                "invalid_prompt",
            ));
        }
        Ok(resolved)
    }

    fn load_from_disk(
        &self,
        id: &str,
        version: Option<&str>,
    ) -> Result<ResolvedPrompt, PromptError> {
        let path = self.find_prompt_file(id, version).ok_or_else(|| {
            PromptError::not_found("prompt", format!("No prompt found with id {id}"))
        })?;
        let raw = fs::read_to_string(&path)
            .map_err(|e| PromptError::internal(format!("Failed to read prompt {id}: {e}")))?;
        match path.extension().and_then(|v| v.to_str()) {
            Some("json") => {
                let file: PromptFile = serde_json::from_str(&raw).map_err(|e| {
                    PromptError::invalid(
                        "prompt",
                        format!("Prompt {id} JSON is invalid: {e}"),
                        "invalid_prompt",
                    )
                })?;
                Ok(ResolvedPrompt {
                    id: file.id.or_else(|| Some(id.to_string())),
                    version: file.version.or_else(|| version.map(str::to_string)),
                    instructions: file.instructions,
                    input_prefix: file.input_prefix,
                    metadata: file.metadata,
                })
            }
            Some("md") => Ok(ResolvedPrompt {
                id: Some(id.to_string()),
                version: version.map(str::to_string),
                instructions: Some(raw),
                input_prefix: None,
                metadata: None,
            }),
            _ => Err(PromptError::invalid(
                "prompt",
                format!("Prompt {id} has unsupported file extension"),
                "invalid_prompt",
            )),
        }
    }

    fn find_prompt_file(&self, id: &str, version: Option<&str>) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(version) = version {
            candidates.push(self.root.join(format!("{id}.{version}.json")));
            candidates.push(self.root.join(format!("{id}.{version}.md")));
        }
        candidates.push(self.root.join(format!("{id}.json")));
        candidates.push(self.root.join(format!("{id}.md")));
        candidates.into_iter().find(|path| path.is_file())
    }
}

impl PromptError {
    fn invalid(param: &'static str, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            param,
            code,
            message: message.into(),
        }
    }

    fn not_found(param: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            param,
            code: "not_found",
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            param: "prompt",
            code: "internal_error",
            message: message.into(),
        }
    }

    pub fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": {
                    "message": self.message,
                    "type": "invalid_request_error",
                    "param": self.param,
                    "code": self.code
                }
            })),
        )
            .into_response()
    }
}

fn validate_key(value: &str, label: &str) -> Result<(), PromptError> {
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value == "."
        || value == ".."
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(PromptError::invalid(
            "prompt",
            format!("Invalid {label}: {value}"),
            "invalid_prompt",
        ));
    }
    Ok(())
}

fn render_variables(text: &str, variables: &HashMap<String, Value>) -> String {
    let mut rendered = text.to_string();
    for (key, value) in variables {
        let placeholder = format!("{{{{{key}}}}}");
        let replacement = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        rendered = rendered.replace(&placeholder, &replacement);
    }
    rendered
}

fn join_prompt_text(prefix: Option<String>, existing: Option<String>) -> String {
    match (non_empty(prefix), non_empty(existing)) {
        (Some(prefix), Some(existing)) => format!("{prefix}\n\n{existing}"),
        (Some(prefix), None) => prefix,
        (None, Some(existing)) => existing,
        (None, None) => String::new(),
    }
}

fn prepend_input_prefix(input: &mut ResponsesInput, prefix: &str) {
    match input {
        ResponsesInput::Text(text) => {
            *text = join_prompt_text(Some(prefix.to_string()), Some(text.clone()));
        }
        ResponsesInput::Messages(items) => {
            if let Some(message) = items.iter_mut().find(|item| {
                item.get("type").and_then(Value::as_str) == Some("message")
                    && item
                        .get("role")
                        .and_then(Value::as_str)
                        .is_none_or(|role| role == "user")
            }) {
                prepend_message_content(message, prefix);
            } else {
                items.insert(
                    0,
                    json!({
                        "type": "message",
                        "role": "user",
                        "content": prefix
                    }),
                );
            }
        }
    }
}

fn prepend_message_content(message: &mut Value, prefix: &str) {
    match message.get_mut("content") {
        Some(Value::String(text)) => {
            *text = join_prompt_text(Some(prefix.to_string()), Some(text.clone()));
        }
        Some(Value::Array(parts)) => {
            if let Some(part) = parts.iter_mut().find(|part| {
                matches!(
                    part.get("type").and_then(Value::as_str),
                    Some("input_text" | "text")
                )
            }) {
                if let Some(text) = part.get("text").and_then(Value::as_str).map(str::to_string) {
                    part["text"] = json!(join_prompt_text(Some(prefix.to_string()), Some(text)));
                    return;
                }
            }
            parts.insert(0, json!({"type": "input_text", "text": prefix}));
        }
        _ => {
            message["content"] = json!(prefix);
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn base_req(prompt: Value) -> ResponsesRequest {
        ResponsesRequest {
            model: "test".into(),
            input: ResponsesInput::Text("input".into()),
            previous_response_id: None,
            tools: vec![],
            stream: false,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            system: None,
            instructions: Some("existing".into()),
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
            prompt: Some(prompt),
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

    fn temp_prompt_dir() -> PathBuf {
        let suffix = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(format!("deecodex-prompts-{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inline_prompt_merges_instructions_and_prefixes_text_input() {
        let registry = PromptRegistry::new("unused");
        let mut req = base_req(json!({
            "instructions": "Act as {{role}}.",
            "input_prefix": "Context: {{topic}}",
            "variables": {"role": "reviewer", "topic": "patch"}
        }));

        registry.apply_to_request(&mut req).unwrap();

        assert_eq!(
            req.instructions.as_deref(),
            Some("Act as reviewer.\n\nexisting")
        );
        match req.input {
            ResponsesInput::Text(text) => assert_eq!(text, "Context: patch\n\ninput"),
            _ => panic!("expected text input"),
        }
        assert!(req.prompt.is_none());
    }

    #[test]
    fn disk_prompt_supports_versioned_json() {
        let dir = temp_prompt_dir();
        fs::write(
            dir.join("agent.v1.json"),
            r#"{"instructions":"Hello {{name}}","input_prefix":"Task {{n}}"}"#,
        )
        .unwrap();
        let registry = PromptRegistry::new(&dir);
        let mut req = base_req(json!({
            "id": "agent",
            "version": "v1",
            "variables": {"name": "Codex", "n": 7}
        }));

        registry.apply_to_request(&mut req).unwrap();

        assert_eq!(req.instructions.as_deref(), Some("Hello Codex\n\nexisting"));
        match req.input {
            ResponsesInput::Text(text) => assert_eq!(text, "Task 7\n\ninput"),
            _ => panic!("expected text input"),
        }
        assert_eq!(
            req.metadata
                .as_ref()
                .and_then(|m| m.get("prompt_id"))
                .map(String::as_str),
            Some("agent")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_path_traversal_prompt_ids() {
        let registry = PromptRegistry::new("unused");
        let mut req = base_req(json!({"id": "../secret"}));
        let err = registry.apply_to_request(&mut req).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn registry_new_stores_root_path() {
        let dir = temp_prompt_dir();
        let registry = PromptRegistry::new(&dir);
        assert_eq!(registry.root(), dir);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_new_handles_non_existent_directory() {
        let dir = temp_prompt_dir();
        fs::remove_dir_all(&dir).unwrap();
        let registry = PromptRegistry::new(&dir);
        assert_eq!(registry.root(), dir);
        let result = registry.list_prompts();
        assert_eq!(result["object"], "list");
        assert!(result["data"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_prompts_returns_empty_for_empty_dir() {
        let dir = temp_prompt_dir();
        let registry = PromptRegistry::new(&dir);
        let result = registry.list_prompts();
        assert_eq!(result["object"], "list");
        assert_eq!(result["data"].as_array().unwrap().len(), 0);
        assert_eq!(result["root"], dir.display().to_string());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_prompts_includes_json_and_md_files() {
        let dir = temp_prompt_dir();
        fs::write(dir.join("agent.json"), "{}").unwrap();
        fs::write(dir.join("helper.md"), "# Helper prompt").unwrap();
        fs::write(dir.join("ignore.txt"), "nope").unwrap();
        let registry = PromptRegistry::new(&dir);
        let result = registry.list_prompts();
        let data = result["data"].as_array().unwrap();
        let ids: Vec<&str> = data.iter().map(|v| v["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["agent", "helper"]);
        for entry in data {
            match entry["id"].as_str().unwrap() {
                "agent" => assert_eq!(entry["format"], "json"),
                "helper" => assert_eq!(entry["format"], "md"),
                _ => panic!("unexpected id"),
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_prompts_filters_directories_and_unsupported_extensions() {
        let dir = temp_prompt_dir();
        fs::write(dir.join("prompt.json"), "{}").unwrap();
        fs::write(dir.join("data.csv"), "a,b").unwrap();
        fs::create_dir(dir.join("subdir")).unwrap();
        let registry = PromptRegistry::new(&dir);
        let result = registry.list_prompts();
        let data = result["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["id"], "prompt");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_prompt_finds_json_prompt_by_id() {
        let dir = temp_prompt_dir();
        fs::write(
            dir.join("agent.json"),
            r#"{"instructions":"Be helpful","version":"v1"}"#,
        )
        .unwrap();
        let registry = PromptRegistry::new(&dir);
        let result = registry.retrieve_prompt("agent").unwrap();
        assert_eq!(result["id"], "agent");
        assert_eq!(result["instructions"], "Be helpful");
        assert_eq!(result["version"], "v1");
        assert_eq!(result["object"], "prompt");
        assert_eq!(result["format"], "json");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_prompt_finds_md_prompt_by_id() {
        let dir = temp_prompt_dir();
        fs::write(dir.join("helper.md"), "You are a helpful assistant.").unwrap();
        let registry = PromptRegistry::new(&dir);
        let result = registry.retrieve_prompt("helper").unwrap();
        assert_eq!(result["id"], "helper");
        assert_eq!(result["instructions"], "You are a helpful assistant.");
        assert_eq!(result["object"], "prompt");
        assert_eq!(result["format"], "md");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_prompt_returns_error_for_missing_id() {
        let dir = temp_prompt_dir();
        let registry = PromptRegistry::new(&dir);
        let err = registry.retrieve_prompt("nonexistent").unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_prompt_returns_error_for_invalid_json() {
        let dir = temp_prompt_dir();
        fs::write(dir.join("broken.json"), "not valid json").unwrap();
        let registry = PromptRegistry::new(&dir);
        let err = registry.retrieve_prompt("broken").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn retrieve_prompt_rejects_invalid_ids() {
        let registry = PromptRegistry::new("unused");
        let err = registry.retrieve_prompt("../evil").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let err = registry.retrieve_prompt("").unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }
}
