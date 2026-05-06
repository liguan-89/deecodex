use crate::types::{ResponsesInput, ResponsesRequest};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use dashmap::DashMap;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use uuid::Uuid;

const MAX_SEARCH_RESULTS: usize = 5;
const MAX_SEARCH_CHARS: usize = 12_000;

#[derive(Clone, Debug)]
pub struct FileStore {
    files: Arc<DashMap<String, StoredFile>>,
}

#[derive(Clone, Debug)]
pub struct StoredFile {
    pub id: String,
    pub filename: String,
    pub purpose: String,
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub created_at: u64,
}

#[derive(Debug)]
pub struct FileError {
    status: StatusCode,
    param: &'static str,
    code: &'static str,
    message: String,
}

impl FileStore {
    pub fn new() -> Self {
        Self {
            files: Arc::new(DashMap::new()),
        }
    }

    pub fn insert(
        &self,
        filename: impl Into<String>,
        purpose: impl Into<String>,
        content_type: impl Into<String>,
        bytes: Vec<u8>,
        created_at: u64,
    ) -> Result<Value, FileError> {
        if bytes.is_empty() {
            return Err(FileError::invalid(
                "file",
                "Uploaded file is empty",
                "empty_file",
            ));
        }
        let id = format!("file_{}", Uuid::new_v4().simple());
        let file = StoredFile {
            id: id.clone(),
            filename: filename.into(),
            purpose: purpose.into(),
            content_type: content_type.into(),
            bytes,
            created_at,
        };
        let object = file.to_object();
        self.files.insert(id, file);
        Ok(object)
    }

    pub fn list(&self) -> Value {
        let mut data: Vec<Value> = self.files.iter().map(|entry| entry.to_object()).collect();
        data.sort_by(|a, b| {
            b.get("created_at")
                .and_then(Value::as_u64)
                .cmp(&a.get("created_at").and_then(Value::as_u64))
        });
        json!({
            "object": "list",
            "data": data,
            "has_more": false
        })
    }

    pub fn get_object(&self, file_id: &str) -> Result<Value, FileError> {
        self.files
            .get(file_id)
            .map(|file| file.to_object())
            .ok_or_else(|| file_not_found(file_id))
    }

    pub fn get_content(&self, file_id: &str) -> Result<(Vec<u8>, String), FileError> {
        self.files
            .get(file_id)
            .map(|file| (file.bytes.clone(), file.content_type.clone()))
            .ok_or_else(|| file_not_found(file_id))
    }

    pub fn delete(&self, file_id: &str) -> Result<Value, FileError> {
        if self.files.remove(file_id).is_some() {
            Ok(json!({
                "id": file_id,
                "object": "file.deleted",
                "deleted": true
            }))
        } else {
            Err(file_not_found(file_id))
        }
    }

    pub fn resolve_request_files(&self, req: &mut ResponsesRequest) -> Result<(), FileError> {
        match &mut req.input {
            ResponsesInput::Text(_) => Ok(()),
            ResponsesInput::Messages(items) => {
                for item in items {
                    resolve_value_file_refs(item, self)?;
                }
                Ok(())
            }
        }
    }

    pub fn inject_file_search_context(
        &self,
        req: &mut ResponsesRequest,
        allowed_file_ids: Option<&HashSet<String>>,
    ) -> Vec<Value> {
        if !uses_file_search(&req.tools) {
            return Vec::new();
        }
        let query = request_text(req);
        let matches = self.search_text_files(&query, allowed_file_ids);
        if matches.is_empty() {
            return Vec::new();
        }
        let mut context = String::from("Local file_search results:\n");
        for (idx, result) in matches.iter().enumerate() {
            context.push_str(&format!(
                "\n[{}] file_id={} filename={} score={}\n{}\n",
                idx + 1,
                result.file_id,
                result.filename,
                result.score,
                result.snippet
            ));
        }
        let result_values: Vec<Value> = matches
            .iter()
            .map(|result| {
                json!({
                    "file_id": result.file_id,
                    "filename": result.filename,
                    "score": result.score,
                    "snippet": result.snippet
                })
            })
            .collect();
        req.instructions = Some(
            match req.instructions.take().or_else(|| req.system.take()) {
                Some(existing) => format!("{existing}\n\n{context}"),
                None => context,
            },
        );
        let metadata = req.metadata.get_or_insert_with(Default::default);
        metadata.insert(
            "local_file_search_results".to_string(),
            serde_json::to_string(&result_values).unwrap_or_default(),
        );
        result_values
    }

    fn search_text_files(
        &self,
        query: &str,
        allowed_file_ids: Option<&HashSet<String>>,
    ) -> Vec<SearchResult> {
        let terms = search_terms(query);
        let mut results = Vec::new();
        for file in self.files.iter() {
            if allowed_file_ids.is_some_and(|ids| !ids.contains(&file.id)) {
                continue;
            }
            if !is_text_file(&file) {
                continue;
            }
            let Ok(text) = String::from_utf8(file.bytes.clone()) else {
                continue;
            };
            let score = score_text(&text, &terms);
            if score == 0 && !terms.is_empty() {
                continue;
            }
            results.push(SearchResult {
                file_id: file.id.clone(),
                filename: file.filename.clone(),
                score,
                snippet: snippet(&text, &terms),
            });
        }
        results.sort_by(|a, b| {
            b.score
                .cmp(&a.score)
                .then_with(|| a.filename.cmp(&b.filename))
        });
        results.truncate(MAX_SEARCH_RESULTS);
        results
    }
}

impl Default for FileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl StoredFile {
    fn to_object(&self) -> Value {
        json!({
            "id": self.id,
            "object": "file",
            "bytes": self.bytes.len(),
            "created_at": self.created_at,
            "filename": self.filename,
            "purpose": self.purpose,
            "content_type": self.content_type
        })
    }
}

impl FileError {
    fn invalid(param: &'static str, message: impl Into<String>, code: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            param,
            code,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            param: "file_id",
            code: "not_found",
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

fn file_not_found(file_id: &str) -> FileError {
    FileError::not_found(format!("No file found with id {file_id}"))
}

fn resolve_value_file_refs(value: &mut Value, store: &FileStore) -> Result<(), FileError> {
    match value {
        Value::Object(map) => {
            let value_type = map.get("type").and_then(Value::as_str).map(str::to_string);
            if value_type.as_deref() == Some("input_image") {
                if let Some(file_id) = map
                    .get("file_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    let Some(file) = store.files.get(&file_id) else {
                        return Err(file_not_found(&file_id));
                    };
                    if !file.content_type.starts_with("image/") {
                        return Err(FileError::invalid(
                            "file_id",
                            format!("{file_id} is not an image file"),
                            "invalid_file_type",
                        ));
                    }
                    let encoded = STANDARD.encode(&file.bytes);
                    map.insert(
                        "image_url".to_string(),
                        json!(format!("data:{};base64,{encoded}", file.content_type)),
                    );
                }
            }
            if value_type.as_deref() == Some("input_file") {
                if let Some(file_id) = map
                    .get("file_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    let Some(file) = store.files.get(&file_id) else {
                        return Err(file_not_found(&file_id));
                    };
                    if !is_text_file(&file) {
                        return Err(FileError::invalid(
                            "file_id",
                            format!("{file_id} is not a text file"),
                            "invalid_file_type",
                        ));
                    }
                    let text = String::from_utf8(file.bytes.clone()).map_err(|_| {
                        FileError::invalid(
                            "file_id",
                            format!("{file_id} is not valid UTF-8 text"),
                            "invalid_file_encoding",
                        )
                    })?;
                    map.insert("type".to_string(), json!("input_text"));
                    map.insert(
                        "text".to_string(),
                        json!(format!(
                            "File {} ({}) content:\n{}",
                            file.filename, file.id, text
                        )),
                    );
                }
            }
            for child in map.values_mut() {
                resolve_value_file_refs(child, store)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                resolve_value_file_refs(child, store)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn uses_file_search(tools: &[Value]) -> bool {
    tools.iter().any(|tool| {
        matches!(
            tool.get("type").and_then(Value::as_str),
            Some("file_search" | "file_search_preview")
        )
    })
}

fn request_text(req: &ResponsesRequest) -> String {
    let mut chunks = Vec::new();
    if let Some(instructions) = req.instructions.as_deref().or(req.system.as_deref()) {
        chunks.push(instructions.to_string());
    }
    collect_text_from_input(&req.input, &mut chunks);
    chunks.join("\n")
}

fn collect_text_from_input(input: &ResponsesInput, chunks: &mut Vec<String>) {
    match input {
        ResponsesInput::Text(text) => chunks.push(text.clone()),
        ResponsesInput::Messages(items) => {
            for item in items {
                collect_text_from_value(item, chunks);
            }
        }
    }
}

fn collect_text_from_value(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::String(text) => chunks.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_text_from_value(item, chunks);
            }
        }
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                chunks.push(text.to_string());
            }
            if let Some(content) = map.get("content") {
                collect_text_from_value(content, chunks);
            }
        }
        _ => {}
    }
}

fn search_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() >= 2)
        .map(str::to_lowercase)
        .take(32)
        .collect()
}

fn score_text(text: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 1;
    }
    let haystack = text.to_lowercase();
    terms
        .iter()
        .map(|term| haystack.match_indices(term).count())
        .sum()
}

fn snippet(text: &str, terms: &[String]) -> String {
    let lower = text.to_lowercase();
    let start = terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0)
        .saturating_sub(300);
    text.chars()
        .skip(start)
        .take(MAX_SEARCH_CHARS.min(1200))
        .collect::<String>()
        .replace('\0', "")
}

fn is_text_file(file: &StoredFile) -> bool {
    file.content_type.starts_with("text/")
        || matches!(
            file.content_type.as_str(),
            "application/json" | "application/xml" | "application/javascript"
        )
        || file.filename.ends_with(".md")
        || file.filename.ends_with(".txt")
        || file.filename.ends_with(".json")
        || file.filename.ends_with(".rs")
        || file.filename.ends_with(".toml")
}

#[derive(Debug)]
struct SearchResult {
    file_id: String,
    filename: String,
    score: usize,
    snippet: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ResponsesRequest;
    use serde_json::json;

    fn base_req(input: ResponsesInput) -> ResponsesRequest {
        ResponsesRequest {
            model: "test".into(),
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

    #[test]
    fn resolves_input_image_file_id_to_data_url() {
        let store = FileStore::new();
        let file = store
            .insert("pixel.png", "vision", "image/png", vec![137, 80, 78, 71], 1)
            .unwrap();
        let file_id = file.get("id").and_then(Value::as_str).unwrap();
        let mut req = base_req(ResponsesInput::Messages(vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_image", "file_id": file_id}]
        })]));

        store.resolve_request_files(&mut req).unwrap();

        let image_url = req.input;
        let ResponsesInput::Messages(items) = image_url else {
            panic!("expected messages")
        };
        assert_eq!(
            items[0]["content"][0]["image_url"].as_str(),
            Some("data:image/png;base64,iVBORw==")
        );
    }

    #[test]
    fn file_search_injects_matching_text_file() {
        let store = FileStore::new();
        store
            .insert(
                "notes.md",
                "assistants",
                "text/markdown",
                b"alpha project has relay notes".to_vec(),
                1,
            )
            .unwrap();
        let mut req = base_req(ResponsesInput::Text("relay".into()));
        req.tools = vec![json!({"type":"file_search"})];

        let results = store.inject_file_search_context(&mut req, None);

        assert!(req
            .instructions
            .as_deref()
            .unwrap_or("")
            .contains("notes.md"));
        assert_eq!(results.len(), 1);
        assert!(req
            .metadata
            .as_ref()
            .unwrap()
            .contains_key("local_file_search_results"));
    }

    #[test]
    fn resolves_input_file_to_text_content() {
        let store = FileStore::new();
        let file = store
            .insert(
                "notes.txt",
                "assistants",
                "text/plain",
                b"hello file".to_vec(),
                1,
            )
            .unwrap();
        let file_id = file.get("id").and_then(Value::as_str).unwrap();
        let mut req = base_req(ResponsesInput::Messages(vec![json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_file", "file_id": file_id}]
        })]));

        store.resolve_request_files(&mut req).unwrap();

        let ResponsesInput::Messages(items) = req.input else {
            panic!("expected messages")
        };
        assert_eq!(items[0]["content"][0]["type"].as_str(), Some("input_text"));
        assert!(items[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello file"));
    }
}
