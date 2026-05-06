use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use dashmap::DashMap;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct VectorStoreRegistry {
    stores: Arc<DashMap<String, StoredVectorStore>>,
    batches: Arc<DashMap<String, StoredFileBatch>>,
}

#[derive(Clone, Debug)]
struct StoredVectorStore {
    id: String,
    name: String,
    created_at: u64,
    file_ids: Vec<String>,
    metadata: Value,
}

#[derive(Clone, Debug)]
struct StoredFileBatch {
    id: String,
    vector_store_id: String,
    created_at: u64,
    file_ids: Vec<String>,
    status: String,
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
        }
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
        Ok(vector_store_file_object(
            vector_store_id,
            &file_id,
            store.created_at,
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
        Ok(json!({
            "id": file_id,
            "object": "vector_store.file.deleted",
            "deleted": before != store.file_ids.len(),
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
        Ok(batch.to_object())
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
}
