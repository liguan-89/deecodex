use crate::types::{ResponsesInput, ResponsesRequest};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tracing::warn;
use uuid::Uuid;

const MAX_SEARCH_RESULTS: usize = 5;
const MAX_SEARCH_CHARS: usize = 12_000;

#[derive(Clone, Debug)]
pub struct FileStore {
    files: Arc<DashMap<String, StoredFile>>,
    data_dir: Option<Arc<PathBuf>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredFile {
    pub id: String,
    pub filename: String,
    pub purpose: String,
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub created_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredFileMetadata {
    id: String,
    filename: String,
    purpose: String,
    content_type: String,
    created_at: u64,
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
            data_dir: None,
        }
    }

    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> io::Result<Self> {
        let data_dir = data_dir.into().join("files");
        fs::create_dir_all(&data_dir)?;
        let store = Self {
            files: Arc::new(DashMap::new()),
            data_dir: Some(Arc::new(data_dir)),
        };
        store.load_from_disk()?;
        Ok(store)
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
        self.persist_file(&file)?;
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
            self.remove_persisted_file(file_id);
            Ok(json!({
                "id": file_id,
                "object": "file.deleted",
                "deleted": true
            }))
        } else {
            Err(file_not_found(file_id))
        }
    }

    fn load_from_disk(&self) -> io::Result<()> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(());
        };
        for entry in fs::read_dir(data_dir.as_ref())? {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(err) => {
                    warn!("failed to read persisted file entry: {err}");
                    continue;
                }
            };
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            match Self::load_file(data_dir.as_ref(), &path) {
                Ok(file) => {
                    self.files.insert(file.id.clone(), file);
                }
                Err(err) => warn!("failed to load persisted file {}: {err}", path.display()),
            }
        }
        Ok(())
    }

    fn load_file(data_dir: &Path, metadata_path: &Path) -> io::Result<StoredFile> {
        let metadata: StoredFileMetadata =
            serde_json::from_slice(&fs::read(metadata_path)?).map_err(invalid_data)?;
        let bytes = fs::read(data_dir.join(format!("{}.bin", metadata.id)))?;
        Ok(StoredFile {
            id: metadata.id,
            filename: metadata.filename,
            purpose: metadata.purpose,
            bytes,
            content_type: metadata.content_type,
            created_at: metadata.created_at,
        })
    }

    fn persist_file(&self, file: &StoredFile) -> Result<(), FileError> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(());
        };
        fs::create_dir_all(data_dir.as_ref()).map_err(FileError::persistence)?;
        write_atomic(&data_dir.join(format!("{}.bin", file.id)), &file.bytes)
            .map_err(FileError::persistence)?;
        let metadata = StoredFileMetadata {
            id: file.id.clone(),
            filename: file.filename.clone(),
            purpose: file.purpose.clone(),
            content_type: file.content_type.clone(),
            created_at: file.created_at,
        };
        let metadata_bytes =
            serde_json::to_vec_pretty(&metadata).map_err(FileError::persistence)?;
        write_atomic(&data_dir.join(format!("{}.json", file.id)), &metadata_bytes)
            .map_err(FileError::persistence)?;
        Ok(())
    }

    fn remove_persisted_file(&self, file_id: &str) {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return;
        };
        for path in [
            data_dir.join(format!("{file_id}.json")),
            data_dir.join(format!("{file_id}.bin")),
        ] {
            if let Err(err) = fs::remove_file(&path) {
                if err.kind() != io::ErrorKind::NotFound {
                    warn!("failed to remove persisted file {}: {err}", path.display());
                }
            }
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
        max_results: Option<usize>,
    ) -> Vec<Value> {
        if !uses_file_search(&req.tools) && !requests_file_search_include(req) {
            return Vec::new();
        }
        let query = request_text(req);
        let matches = self.search_text_files(&query, allowed_file_ids, max_results);
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
        metadata.insert("local_file_search_query".to_string(), query);
        result_values
    }

    fn search_text_files(
        &self,
        query: &str,
        allowed_file_ids: Option<&HashSet<String>>,
        max_results: Option<usize>,
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
        results.truncate(
            max_results
                .unwrap_or(MAX_SEARCH_RESULTS)
                .clamp(1, MAX_SEARCH_RESULTS),
        );
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

    fn persistence(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            param: "file",
            code: "persistence_error",
            message: format!("Failed to persist uploaded file: {error}"),
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

fn invalid_data(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp_path = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    fs::write(&tmp_path, bytes)?;
    fs::rename(tmp_path, path)
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

fn requests_file_search_include(req: &ResponsesRequest) -> bool {
    req.include.as_ref().is_some_and(|include| {
        include
            .iter()
            .any(|field| field.contains("file_search_call"))
    })
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

        let results = store.inject_file_search_context(&mut req, None, None);

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
        assert!(req
            .metadata
            .as_ref()
            .unwrap()
            .contains_key("local_file_search_query"));
    }

    #[test]
    fn file_search_include_triggers_matching_text_file() {
        let store = FileStore::new();
        store
            .insert(
                "notes.md",
                "assistants",
                "text/markdown",
                b"local include should find relay notes".to_vec(),
                1,
            )
            .unwrap();
        let mut req = base_req(ResponsesInput::Text("relay".into()));
        req.include = Some(vec!["file_search_call.results".into()]);

        let results = store.inject_file_search_context(&mut req, None, None);

        assert_eq!(results.len(), 1);
        assert!(req
            .metadata
            .as_ref()
            .unwrap()
            .contains_key("local_file_search_results"));
    }

    #[test]
    fn file_search_respects_max_results() {
        let store = FileStore::new();
        store
            .insert(
                "a.md",
                "assistants",
                "text/markdown",
                b"relay one".to_vec(),
                1,
            )
            .unwrap();
        store
            .insert(
                "b.md",
                "assistants",
                "text/markdown",
                b"relay two".to_vec(),
                1,
            )
            .unwrap();
        let mut req = base_req(ResponsesInput::Text("relay".into()));
        req.tools = vec![json!({"type":"file_search", "max_num_results": 1})];

        let results = store.inject_file_search_context(&mut req, None, Some(1));

        assert_eq!(results.len(), 1);
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

    #[test]
    fn persistent_store_restores_metadata_and_content() {
        let dir = std::env::temp_dir().join(format!("deecodex-files-{}", Uuid::new_v4().simple()));
        let file_id = {
            let store = FileStore::with_data_dir(&dir).unwrap();
            let file = store
                .insert(
                    "notes.txt",
                    "assistants",
                    "text/plain",
                    b"persistent hello".to_vec(),
                    7,
                )
                .unwrap();
            file.get("id").and_then(Value::as_str).unwrap().to_string()
        };

        let restored = FileStore::with_data_dir(&dir).unwrap();

        assert_eq!(
            restored.get_object(&file_id).unwrap()["filename"].as_str(),
            Some("notes.txt")
        );
        assert_eq!(
            restored.get_content(&file_id).unwrap().0,
            b"persistent hello".to_vec()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn list_returns_files_sorted_by_created_at_desc() {
        let store = FileStore::new();
        store
            .insert("a.txt", "test", "text/plain", b"aaa".to_vec(), 100)
            .unwrap();
        store
            .insert("b.txt", "test", "text/plain", b"bbb".to_vec(), 200)
            .unwrap();
        store
            .insert("c.txt", "test", "text/plain", b"ccc".to_vec(), 10)
            .unwrap();

        let result = store.list();
        assert_eq!(result["object"].as_str(), Some("list"));
        assert_eq!(result["has_more"].as_bool(), Some(false));
        let data = result["data"].as_array().unwrap();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0]["created_at"].as_u64(), Some(200));
        assert_eq!(data[1]["created_at"].as_u64(), Some(100));
        assert_eq!(data[2]["created_at"].as_u64(), Some(10));
    }

    #[test]
    fn list_returns_empty_for_empty_store() {
        let store = FileStore::new();
        let result = store.list();
        let data = result["data"].as_array().unwrap();
        assert!(data.is_empty());
    }

    #[test]
    fn delete_removes_file_and_returns_success() {
        let store = FileStore::new();
        let obj = store
            .insert("a.txt", "test", "text/plain", b"aaa".to_vec(), 1)
            .unwrap();
        let file_id = obj["id"].as_str().unwrap().to_string();

        let result = store.delete(&file_id).unwrap();
        assert_eq!(result["id"].as_str(), Some(file_id.as_str()));
        assert_eq!(result["object"].as_str(), Some("file.deleted"));
        assert_eq!(result["deleted"].as_bool(), Some(true));
        assert!(store.get_object(&file_id).is_err());
    }

    #[test]
    fn delete_non_existent_returns_error() {
        let store = FileStore::new();
        let err = store.delete("nonexistent").unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn delete_removes_persisted_files_from_disk() {
        let dir = std::env::temp_dir().join(format!("deecodex-files-{}", Uuid::new_v4().simple()));
        let store = FileStore::with_data_dir(&dir).unwrap();
        let obj = store
            .insert("a.txt", "test", "text/plain", b"aaa".to_vec(), 1)
            .unwrap();
        let file_id = obj["id"].as_str().unwrap().to_string();

        let files_dir = dir.join("files");
        assert!(files_dir.join(format!("{file_id}.json")).exists());
        assert!(files_dir.join(format!("{file_id}.bin")).exists());

        store.delete(&file_id).unwrap();

        assert!(!files_dir.join(format!("{file_id}.json")).exists());
        assert!(!files_dir.join(format!("{file_id}.bin")).exists());

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn search_terms_extracts_alphanumeric_terms() {
        assert_eq!(
            search_terms("hello world test"),
            vec!["hello", "world", "test"]
        );
    }

    #[test]
    fn search_terms_empty_query() {
        assert!(search_terms("").is_empty());
    }

    #[test]
    fn search_terms_filters_short_terms() {
        assert_eq!(search_terms("a an the"), vec!["an", "the"]);
    }

    #[test]
    fn search_terms_special_characters() {
        assert_eq!(
            search_terms("hello-world_foo+bar"),
            vec!["hello", "world", "foo", "bar"]
        );
    }

    #[test]
    fn search_terms_converts_to_lowercase() {
        assert_eq!(search_terms("Hello WORLD"), vec!["hello", "world"]);
    }

    #[test]
    fn search_terms_limits_to_32() {
        let input: String = (0..40)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(search_terms(&input).len(), 32);
    }

    #[test]
    fn score_text_counts_matches() {
        let terms = vec!["hello".to_string(), "world".to_string()];
        assert_eq!(score_text("hello world hello", &terms), 3);
    }

    #[test]
    fn score_text_no_matches_returns_zero() {
        assert_eq!(score_text("hello world", &["xyz".to_string()]), 0);
    }

    #[test]
    fn score_text_empty_terms_returns_one() {
        assert_eq!(score_text("anything", &[]), 1);
    }

    #[test]
    fn score_text_empty_text_returns_zero() {
        assert_eq!(score_text("", &["hello".to_string()]), 0);
    }

    #[test]
    fn snippet_finds_match_and_truncates() {
        let long_text = "x".repeat(500) + "target_word" + &"x".repeat(500);
        let terms = vec!["target_word".to_string()];
        let result = snippet(&long_text, &terms);
        assert!(result.contains("target_word"));
        assert!(result.len() <= 1200);
    }

    #[test]
    fn snippet_no_match_returns_start() {
        assert_eq!(snippet("hello world", &["xyz".to_string()]), "hello world");
    }

    #[test]
    fn snippet_handles_null_bytes() {
        let result = snippet("hello\0world", &["hello".to_string()]);
        assert!(!result.contains('\0'));
        assert!(result.contains("helloworld"));
    }

    #[test]
    fn is_text_file_text_mime_returns_true() {
        let file = StoredFile {
            id: "t".into(),
            filename: "f.bin".into(),
            purpose: "t".into(),
            bytes: vec![],
            content_type: "text/plain".into(),
            created_at: 0,
        };
        assert!(is_text_file(&file));
    }

    #[test]
    fn is_text_file_application_json_returns_true() {
        let file = StoredFile {
            id: "t".into(),
            filename: "f.bin".into(),
            purpose: "t".into(),
            bytes: vec![],
            content_type: "application/json".into(),
            created_at: 0,
        };
        assert!(is_text_file(&file));
    }

    #[test]
    fn is_text_file_image_returns_false() {
        let file = StoredFile {
            id: "t".into(),
            filename: "f.png".into(),
            purpose: "t".into(),
            bytes: vec![],
            content_type: "image/png".into(),
            created_at: 0,
        };
        assert!(!is_text_file(&file));
    }

    #[test]
    fn is_text_file_by_extension() {
        for (name, expected) in [
            ("readme.md", true),
            ("notes.txt", true),
            ("package.json", true),
            ("main.rs", true),
            ("Cargo.toml", true),
            ("image.png", false),
            ("data.csv", false),
        ] {
            let file = StoredFile {
                id: "t".into(),
                filename: name.into(),
                purpose: "t".into(),
                bytes: vec![],
                content_type: "".into(),
                created_at: 0,
            };
            assert_eq!(is_text_file(&file), expected, "failed for {name}");
        }
    }

    #[test]
    fn is_text_file_empty_returns_false() {
        let file = StoredFile {
            id: "t".into(),
            filename: "f.bin".into(),
            purpose: "t".into(),
            bytes: vec![],
            content_type: "".into(),
            created_at: 0,
        };
        assert!(!is_text_file(&file));
    }

    #[test]
    fn stored_file_to_object_structure() {
        let file = StoredFile {
            id: "file_abc123".into(),
            filename: "test.txt".into(),
            purpose: "assistants".into(),
            bytes: b"hello world".to_vec(),
            content_type: "text/plain".into(),
            created_at: 42,
        };
        let obj = file.to_object();
        assert_eq!(obj["id"].as_str(), Some("file_abc123"));
        assert_eq!(obj["object"].as_str(), Some("file"));
        assert_eq!(obj["bytes"].as_u64(), Some(11));
        assert_eq!(obj["created_at"].as_u64(), Some(42));
        assert_eq!(obj["filename"].as_str(), Some("test.txt"));
        assert_eq!(obj["purpose"].as_str(), Some("assistants"));
        assert_eq!(obj["content_type"].as_str(), Some("text/plain"));
    }

    #[test]
    fn stored_file_content_type_preserved_in_insert_output() {
        let store = FileStore::new();
        let file = store
            .insert("test.txt", "test", "text/plain", b"hello".to_vec(), 1)
            .unwrap();
        assert_eq!(file["content_type"].as_str(), Some("text/plain"));
        assert_eq!(file["filename"].as_str(), Some("test.txt"));
        assert_eq!(file["object"].as_str(), Some("file"));
        assert_eq!(file["bytes"].as_u64(), Some(5));
    }
}
