use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
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

#[derive(Clone, Debug)]
pub struct VectorStoreRegistry {
    stores: Arc<DashMap<String, StoredVectorStore>>,
    batches: Arc<DashMap<String, StoredFileBatch>>,
    data_dir: Option<Arc<PathBuf>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredVectorStore {
    id: String,
    name: String,
    created_at: u64,
    file_ids: Vec<String>,
    metadata: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredFileBatch {
    id: String,
    vector_store_id: String,
    created_at: u64,
    file_ids: Vec<String>,
    status: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistrySnapshot {
    stores: Vec<StoredVectorStore>,
    batches: Vec<StoredFileBatch>,
}

#[derive(Debug)]
pub struct VectorStoreError {
    status: StatusCode,
    param: &'static str,
    code: &'static str,
    message: String,
}

impl VectorStoreRegistry {
    pub fn new() -> Self {
        Self {
            stores: Arc::new(DashMap::new()),
            batches: Arc::new(DashMap::new()),
            data_dir: None,
        }
    }

    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> io::Result<Self> {
        let data_dir = data_dir.into();
        fs::create_dir_all(&data_dir)?;
        let registry = Self {
            stores: Arc::new(DashMap::new()),
            batches: Arc::new(DashMap::new()),
            data_dir: Some(Arc::new(data_dir)),
        };
        registry.load_from_disk()?;
        Ok(registry)
    }

    pub fn create(
        &self,
        name: Option<String>,
        file_ids: Vec<String>,
        metadata: Value,
        created_at: u64,
    ) -> Value {
        let id = format!("vs_{}", Uuid::new_v4().simple());
        let store = StoredVectorStore {
            id: id.clone(),
            name: name.unwrap_or_else(|| id.clone()),
            created_at,
            file_ids,
            metadata,
        };
        let object = store.to_object();
        self.stores.insert(id, store);
        self.persist_or_warn();
        object
    }

    pub fn list(&self) -> Value {
        let mut data: Vec<Value> = self.stores.iter().map(|entry| entry.to_object()).collect();
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

    pub fn get(&self, vector_store_id: &str) -> Result<Value, VectorStoreError> {
        self.stores
            .get(vector_store_id)
            .map(|store| store.to_object())
            .ok_or_else(|| not_found(vector_store_id))
    }

    pub fn delete(&self, vector_store_id: &str) -> Result<Value, VectorStoreError> {
        if self.stores.remove(vector_store_id).is_some() {
            let batch_ids: Vec<String> = self
                .batches
                .iter()
                .filter(|entry| entry.vector_store_id == vector_store_id)
                .map(|entry| entry.id.clone())
                .collect();
            for batch_id in batch_ids {
                self.batches.remove(&batch_id);
            }
            self.persist_or_warn();
            Ok(json!({
                "id": vector_store_id,
                "object": "vector_store.deleted",
                "deleted": true
            }))
        } else {
            Err(not_found(vector_store_id))
        }
    }

    pub fn add_file(
        &self,
        vector_store_id: &str,
        file_id: String,
    ) -> Result<Value, VectorStoreError> {
        let Some(mut store) = self.stores.get_mut(vector_store_id) else {
            return Err(not_found(vector_store_id));
        };
        if !store.file_ids.contains(&file_id) {
            store.file_ids.push(file_id.clone());
        }
        let created_at = store.created_at;
        drop(store);
        self.persist_or_warn();
        Ok(vector_store_file_object(
            vector_store_id,
            &file_id,
            created_at,
        ))
    }

    pub fn list_files(&self, vector_store_id: &str) -> Result<Value, VectorStoreError> {
        let Some(store) = self.stores.get(vector_store_id) else {
            return Err(not_found(vector_store_id));
        };
        let data: Vec<Value> = store
            .file_ids
            .iter()
            .map(|file_id| vector_store_file_object(vector_store_id, file_id, store.created_at))
            .collect();
        Ok(json!({
            "object": "list",
            "data": data,
            "has_more": false
        }))
    }

    pub fn get_file(
        &self,
        vector_store_id: &str,
        file_id: &str,
    ) -> Result<Value, VectorStoreError> {
        let Some(store) = self.stores.get(vector_store_id) else {
            return Err(not_found(vector_store_id));
        };
        if store.file_ids.iter().any(|id| id == file_id) {
            Ok(vector_store_file_object(
                vector_store_id,
                file_id,
                store.created_at,
            ))
        } else {
            Err(VectorStoreError::not_found(format!(
                "No vector store file found with id {file_id}"
            )))
        }
    }

    pub fn delete_file(
        &self,
        vector_store_id: &str,
        file_id: &str,
    ) -> Result<Value, VectorStoreError> {
        let Some(mut store) = self.stores.get_mut(vector_store_id) else {
            return Err(not_found(vector_store_id));
        };
        let before = store.file_ids.len();
        store.file_ids.retain(|id| id != file_id);
        let deleted = before != store.file_ids.len();
        drop(store);
        self.persist_or_warn();
        Ok(json!({
            "id": file_id,
            "object": "vector_store.file.deleted",
            "deleted": deleted,
            "vector_store_id": vector_store_id
        }))
    }

    pub fn create_batch(
        &self,
        vector_store_id: &str,
        file_ids: Vec<String>,
        created_at: u64,
    ) -> Result<Value, VectorStoreError> {
        let Some(mut store) = self.stores.get_mut(vector_store_id) else {
            return Err(not_found(vector_store_id));
        };
        for file_id in &file_ids {
            if !store.file_ids.contains(file_id) {
                store.file_ids.push(file_id.clone());
            }
        }
        let batch = StoredFileBatch {
            id: format!("vsfb_{}", Uuid::new_v4().simple()),
            vector_store_id: vector_store_id.to_string(),
            created_at,
            file_ids,
            status: "completed".to_string(),
        };
        let object = batch.to_object();
        self.batches.insert(batch.id.clone(), batch);
        drop(store);
        self.persist_or_warn();
        Ok(object)
    }

    pub fn get_batch(
        &self,
        vector_store_id: &str,
        batch_id: &str,
    ) -> Result<Value, VectorStoreError> {
        let Some(batch) = self.batches.get(batch_id) else {
            return Err(batch_not_found(batch_id));
        };
        if batch.vector_store_id != vector_store_id {
            return Err(batch_not_found(batch_id));
        }
        Ok(batch.to_object())
    }

    pub fn cancel_batch(
        &self,
        vector_store_id: &str,
        batch_id: &str,
    ) -> Result<Value, VectorStoreError> {
        let Some(mut batch) = self.batches.get_mut(batch_id) else {
            return Err(batch_not_found(batch_id));
        };
        if batch.vector_store_id != vector_store_id {
            return Err(batch_not_found(batch_id));
        }
        if batch.status != "completed" {
            batch.status = "cancelled".to_string();
        }
        let object = batch.to_object();
        drop(batch);
        self.persist_or_warn();
        Ok(object)
    }

    pub fn list_batch_files(
        &self,
        vector_store_id: &str,
        batch_id: &str,
    ) -> Result<Value, VectorStoreError> {
        let Some(batch) = self.batches.get(batch_id) else {
            return Err(batch_not_found(batch_id));
        };
        if batch.vector_store_id != vector_store_id {
            return Err(batch_not_found(batch_id));
        }
        let data: Vec<Value> = batch
            .file_ids
            .iter()
            .map(|file_id| vector_store_file_object(vector_store_id, file_id, batch.created_at))
            .collect();
        Ok(json!({
            "object": "list",
            "data": data,
            "has_more": false
        }))
    }

    pub fn file_ids_for_tools(&self, tools: &[Value]) -> Option<HashSet<String>> {
        let vector_store_ids: Vec<&str> = tools
            .iter()
            .filter(|tool| {
                matches!(
                    tool.get("type").and_then(Value::as_str),
                    Some("file_search" | "file_search_preview")
                )
            })
            .flat_map(|tool| {
                tool.get("vector_store_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
            })
            .collect();
        if vector_store_ids.is_empty() {
            return None;
        }
        let mut file_ids = HashSet::new();
        for store_id in vector_store_ids {
            if let Some(store) = self.stores.get(store_id) {
                file_ids.extend(store.file_ids.iter().cloned());
            }
        }
        Some(file_ids)
    }

    fn load_from_disk(&self) -> io::Result<()> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(());
        };
        let path = snapshot_path(data_dir.as_ref());
        if !path.exists() {
            return Ok(());
        }
        let snapshot: RegistrySnapshot =
            serde_json::from_slice(&fs::read(&path)?).map_err(invalid_data)?;
        for store in snapshot.stores {
            self.stores.insert(store.id.clone(), store);
        }
        for batch in snapshot.batches {
            self.batches.insert(batch.id.clone(), batch);
        }
        Ok(())
    }

    fn persist_or_warn(&self) {
        if let Err(err) = self.persist() {
            warn!("failed to persist vector stores: {err}");
        }
    }

    fn persist(&self) -> io::Result<()> {
        let Some(data_dir) = self.data_dir.as_ref() else {
            return Ok(());
        };
        fs::create_dir_all(data_dir.as_ref())?;
        let snapshot = RegistrySnapshot {
            stores: self.stores.iter().map(|entry| entry.clone()).collect(),
            batches: self.batches.iter().map(|entry| entry.clone()).collect(),
        };
        let bytes = serde_json::to_vec_pretty(&snapshot).map_err(invalid_data)?;
        write_atomic(&snapshot_path(data_dir.as_ref()), &bytes)
    }
}

impl Default for VectorStoreRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StoredVectorStore {
    fn to_object(&self) -> Value {
        json!({
            "id": self.id,
            "object": "vector_store",
            "created_at": self.created_at,
            "name": self.name,
            "status": "completed",
            "usage_bytes": 0,
            "file_counts": {
                "in_progress": 0,
                "completed": self.file_ids.len(),
                "failed": 0,
                "cancelled": 0,
                "total": self.file_ids.len()
            },
            "metadata": self.metadata
        })
    }
}

impl StoredFileBatch {
    fn to_object(&self) -> Value {
        let completed = if self.status == "completed" {
            self.file_ids.len()
        } else {
            0
        };
        let cancelled = if self.status == "cancelled" {
            self.file_ids.len()
        } else {
            0
        };
        json!({
            "id": self.id,
            "object": "vector_store.file_batch",
            "created_at": self.created_at,
            "vector_store_id": self.vector_store_id,
            "status": self.status,
            "file_counts": {
                "in_progress": 0,
                "completed": completed,
                "failed": 0,
                "cancelled": cancelled,
                "total": self.file_ids.len()
            }
        })
    }
}

impl VectorStoreError {
    #[allow(dead_code)]
    pub fn invalid(param: &'static str, message: impl Into<String>, code: &'static str) -> Self {
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
            param: "vector_store_id",
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

fn not_found(vector_store_id: &str) -> VectorStoreError {
    VectorStoreError::not_found(format!("No vector store found with id {vector_store_id}"))
}

fn batch_not_found(batch_id: &str) -> VectorStoreError {
    VectorStoreError::not_found(format!(
        "No vector store file batch found with id {batch_id}"
    ))
}

fn vector_store_file_object(vector_store_id: &str, file_id: &str, created_at: u64) -> Value {
    json!({
        "id": file_id,
        "object": "vector_store.file",
        "created_at": created_at,
        "vector_store_id": vector_store_id,
        "status": "completed"
    })
}

fn snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join("vector_stores.json")
}

fn invalid_data(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp_path = path.with_extension(format!("tmp-{}", Uuid::new_v4().simple()));
    fs::write(&tmp_path, bytes)?;
    fs::rename(tmp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_store_filters_file_search_ids() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec!["file_a".into()], json!({}), 1);
        let store_id = store.get("id").and_then(Value::as_str).unwrap();
        let filter = registry.file_ids_for_tools(&[json!({
            "type": "file_search",
            "vector_store_ids": [store_id]
        })]);

        assert!(filter.as_ref().unwrap().contains("file_a"));
    }

    #[test]
    fn vector_store_file_batch_can_be_retrieved() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let store_id = store.get("id").and_then(Value::as_str).unwrap();
        let batch = registry
            .create_batch(store_id, vec!["file_a".into(), "file_b".into()], 2)
            .unwrap();
        let batch_id = batch.get("id").and_then(Value::as_str).unwrap();

        let retrieved = registry.get_batch(store_id, batch_id).unwrap();

        assert_eq!(retrieved["file_counts"]["completed"].as_u64(), Some(2));
        assert_eq!(
            registry
                .list_batch_files(store_id, batch_id)
                .unwrap()
                .get("data")
                .and_then(Value::as_array)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn persistent_registry_restores_stores_and_batches() {
        let dir = std::env::temp_dir().join(format!("deecodex-vector-{}", Uuid::new_v4().simple()));
        let (store_id, batch_id) = {
            let registry = VectorStoreRegistry::with_data_dir(&dir).unwrap();
            let store = registry.create(Some("docs".into()), vec!["file_a".into()], json!({}), 1);
            let store_id = store.get("id").and_then(Value::as_str).unwrap().to_string();
            let batch = registry
                .create_batch(&store_id, vec!["file_b".into()], 2)
                .unwrap();
            let batch_id = batch.get("id").and_then(Value::as_str).unwrap().to_string();
            (store_id, batch_id)
        };

        let restored = VectorStoreRegistry::with_data_dir(&dir).unwrap();

        assert!(restored
            .file_ids_for_tools(&[json!({
                "type": "file_search",
                "vector_store_ids": [&store_id]
            })])
            .unwrap()
            .contains("file_b"));
        assert_eq!(
            restored.get_batch(&store_id, &batch_id).unwrap()["file_counts"]["completed"].as_u64(),
            Some(1)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_creates_empty_registry() {
        let registry = VectorStoreRegistry::new();
        let list = registry.list();
        assert_eq!(list["object"], "list");
        assert!(list["data"].as_array().unwrap().is_empty());
        assert_eq!(list["has_more"], false);
    }

    #[test]
    fn list_returns_all_stores() {
        let registry = VectorStoreRegistry::new();
        registry.create(Some("a".into()), vec![], json!({}), 1);
        registry.create(Some("b".into()), vec![], json!({}), 2);

        let data = registry.list()["data"].as_array().unwrap().clone();
        assert_eq!(data.len(), 2);
    }

    #[test]
    fn get_returns_existing_store() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let id = store["id"].as_str().unwrap();

        let result = registry.get(id).unwrap();
        assert_eq!(result["id"], id);
        assert_eq!(result["object"], "vector_store");
    }

    #[test]
    fn get_returns_error_for_missing_store() {
        let registry = VectorStoreRegistry::new();
        let err = registry.get("vs_nonexistent").unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn delete_removes_existing_store() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let id = store["id"].as_str().unwrap().to_string();

        let result = registry.delete(&id).unwrap();
        assert_eq!(result["id"], id);
        assert_eq!(result["deleted"], true);
        assert!(registry.get(&id).is_err());
    }

    #[test]
    fn delete_non_existent_returns_error() {
        let registry = VectorStoreRegistry::new();
        let err = registry.delete("vs_nonexistent").unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn add_file_adds_to_store() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let id = store["id"].as_str().unwrap();

        let result = registry.add_file(id, "file_x".into()).unwrap();
        assert_eq!(result["id"], "file_x");
        assert_eq!(result["object"], "vector_store.file");

        let files = registry.list_files(id).unwrap();
        assert_eq!(files["data"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn add_file_errors_for_non_existent_store() {
        let registry = VectorStoreRegistry::new();
        let err = registry
            .add_file("vs_nonexistent", "file_x".into())
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn get_file_returns_file_from_store() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let id = store["id"].as_str().unwrap();
        registry.add_file(id, "file_y".into()).unwrap();

        let result = registry.get_file(id, "file_y").unwrap();
        assert_eq!(result["id"], "file_y");
        assert_eq!(result["vector_store_id"], id);
    }

    #[test]
    fn get_file_errors_for_missing_file() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let id = store["id"].as_str().unwrap();

        let err = registry.get_file(id, "file_nonexistent").unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn delete_file_removes_file_from_store() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec!["file_z".into()], json!({}), 1);
        let id = store["id"].as_str().unwrap();

        let result = registry.delete_file(id, "file_z").unwrap();
        assert_eq!(result["id"], "file_z");
        assert_eq!(result["deleted"], true);

        assert!(registry.get_file(id, "file_z").is_err());
    }

    #[test]
    fn cancel_batch_returns_batch_object() {
        let registry = VectorStoreRegistry::new();
        let store = registry.create(Some("docs".into()), vec![], json!({}), 1);
        let store_id = store["id"].as_str().unwrap();
        let batch = registry
            .create_batch(store_id, vec!["file_a".into()], 2)
            .unwrap();
        let batch_id = batch["id"].as_str().unwrap();

        let result = registry.cancel_batch(store_id, batch_id).unwrap();
        assert_eq!(result["id"], batch_id);
        assert_eq!(result["object"], "vector_store.file_batch");
    }

    #[test]
    fn cancel_batch_errors_for_non_existent_batch() {
        let registry = VectorStoreRegistry::new();
        let err = registry
            .cancel_batch("vs_x", "vsfb_nonexistent")
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }
}
