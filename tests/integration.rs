use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, Method, Request, Response, StatusCode},
    response::IntoResponse,
    routing::post,
    Router,
};
use deecodex::{
    cache::RequestCache,
    handlers::{build_router, AppState},
    session::SessionStore,
    stream::{translate_cached, translate_stream, CachedArgs, StreamArgs},
    types::{ChatMessage, ChatRequest},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        sessions: deecodex::session::SessionStore::new(),
        client: reqwest::Client::builder().build().unwrap(),
        upstream: Arc::new(reqwest::Url::parse("https://example.com").unwrap()),
        api_key: Arc::new("test".into()),
        client_api_key: Arc::new(String::new()),
        model_map: Arc::new(std::collections::HashMap::new()),
        vision_upstream: None,
        vision_api_key: Arc::new(String::new()),
        vision_model: Arc::new("test".into()),
        vision_endpoint: Arc::new("v1/test".into()),
        start_time: Instant::now(),
        request_cache: deecodex::cache::RequestCache::default(),
        prompts: Arc::new(deecodex::prompts::PromptRegistry::new("prompts")),
        files: deecodex::files::FileStore::new(),
        vector_stores: deecodex::vector_stores::VectorStoreRegistry::new(),
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: false,
        rate_limiter: None,
        metrics: Arc::new(deecodex::metrics::Metrics::new()),
        token_tracker: Arc::new(deecodex::token_anomaly::TokenTracker::default()),
        tool_policy: deecodex::handlers::ToolPolicy::default(),
        executors: Arc::new(deecodex::executor::LocalExecutorConfig::default()),
    }
}

async fn one_shot_upstream(response_body: &'static str) -> reqwest::Url {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0_u8; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    reqwest::Url::parse(&format!("http://{addr}/v1")).unwrap()
}

#[tokio::test]
async fn test_health_returns_ok() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert!(json["uptime_secs"].as_u64().is_some());
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn test_v1_returns_ok() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
}

// ── Prompt integration tests ────────────────────────────────────────────

#[tokio::test]
async fn test_list_prompts_returns_empty() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/prompts")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    assert!(json["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_get_prompt_nonexistent() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/prompts/nonexistent")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

// ── Vector Store integration tests ─────────────────────────────────────

#[tokio::test]
async fn test_vector_stores_create() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/vector_stores")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name": "test-store"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let store: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(store["object"], "vector_store");
    assert_eq!(store["name"], "test-store");
    assert!(store["id"].as_str().unwrap().starts_with("vs_"));
    assert!(store["created_at"].as_u64().is_some());
    assert_eq!(store["status"], "completed");
}

#[tokio::test]
async fn test_vector_stores_list_empty() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/vector_stores")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    assert!(json["data"].as_array().unwrap().is_empty());
    assert_eq!(json["has_more"], false);
}

#[tokio::test]
async fn test_vector_stores_list_with_stores() {
    let state = test_state();
    state
        .vector_stores
        .create(Some("alpha".into()), vec![], json!({}), 1);
    state
        .vector_stores
        .create(Some("beta".into()), vec![], json!({}), 2);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/vector_stores")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    // Most recent first
    assert_eq!(data[0]["name"], "beta");
}

#[tokio::test]
async fn test_vector_stores_get() {
    let state = test_state();
    let store = state
        .vector_stores
        .create(Some("docs".into()), vec![], json!({"key": "val"}), 100);
    let id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{id}"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["id"], id);
    assert_eq!(result["name"], "docs");
    assert_eq!(result["object"], "vector_store");
    assert_eq!(result["metadata"]["key"], "val");
}

#[tokio::test]
async fn test_vector_stores_get_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/vector_stores/vs_nonexistent")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_vector_stores_delete() {
    let state = test_state();
    let store = state
        .vector_stores
        .create(Some("delete-me".into()), vec![], json!({}), 1);
    let id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{id}"))
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["id"], id);
    assert_eq!(result["object"], "vector_store.deleted");
    assert_eq!(result["deleted"], true);
}

#[tokio::test]
async fn test_vector_stores_delete_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/vector_stores/vs_nonexistent")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_vector_store_add_file() {
    let state = test_state();
    let store =
        state
            .vector_stores
            .create(Some("docs".into()), vec!["existing".into()], json!({}), 1);
    let store_id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/files"))
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"file_id": "new_file"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let file: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(file["id"], "new_file");
    assert_eq!(file["object"], "vector_store.file");
    assert_eq!(file["vector_store_id"], store_id);
    assert_eq!(file["status"], "completed");
}

#[tokio::test]
async fn test_vector_store_add_file_to_nonexistent_store() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/vector_stores/vs_nonexistent/files")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"file_id": "x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_vector_store_list_and_get_files() {
    let state = test_state();
    let store = state.vector_stores.create(
        Some("docs".into()),
        vec!["f1".into(), "f2".into()],
        json!({}),
        1,
    );
    let store_id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    // List files
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/files"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list["object"], "list");
    assert_eq!(list["data"].as_array().unwrap().len(), 2);

    // Get existing file
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/files/f1"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let file: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(file["id"], "f1");
    assert_eq!(file["vector_store_id"], store_id);

    // Get non-existent file
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/files/nonexistent"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_vector_store_delete_file() {
    let state = test_state();
    let store = state.vector_stores.create(
        Some("docs".into()),
        vec!["f1".into(), "f2".into()],
        json!({}),
        1,
    );
    let store_id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    // Delete file
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/files/f1"))
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["id"], "f1");
    assert_eq!(result["object"], "vector_store.file.deleted");
    assert_eq!(result["deleted"], true);
    assert_eq!(result["vector_store_id"], store_id);

    // Verify file count decreased
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/files"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_vector_store_file_batches_create_and_get() {
    let state = test_state();
    let store = state
        .vector_stores
        .create(Some("docs".into()), vec![], json!({}), 1);
    let store_id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    // Create batch
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/vector_stores/{store_id}/file_batches"))
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"file_ids": ["fa", "fb"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let batch: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(batch["object"], "vector_store.file_batch");
    assert_eq!(batch["vector_store_id"], store_id);
    assert_eq!(batch["status"], "completed");
    assert_eq!(batch["file_counts"]["completed"], 2);
    let batch_id = batch["id"].as_str().unwrap().to_string();

    // Get batch
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/vector_stores/{store_id}/file_batches/{batch_id}"
                ))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let retrieved: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(retrieved["id"], batch_id);

    // List batch files
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/vector_stores/{store_id}/file_batches/{batch_id}/files"
                ))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let files: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(files["object"], "list");
    assert_eq!(files["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_vector_store_file_batches_get_not_found() {
    let state = test_state();
    let store = state
        .vector_stores
        .create(Some("docs".into()), vec![], json!({}), 1);
    let store_id = store["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/vector_stores/{store_id}/file_batches/vsfb_nonexistent"
                ))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_vector_store_file_batches_cancel() {
    let state = test_state();
    let store = state
        .vector_stores
        .create(Some("docs".into()), vec![], json!({}), 1);
    let store_id = store["id"].as_str().unwrap().to_string();
    let batch = state
        .vector_stores
        .create_batch(&store_id, vec!["f1".into()], 2)
        .unwrap();
    let batch_id = batch["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/vector_stores/{store_id}/file_batches/{batch_id}/cancel"
                ))
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["id"], batch_id);
    assert_eq!(result["object"], "vector_store.file_batch");
}

#[tokio::test]
async fn test_responses_parse_error() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from("this is not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_responses_unsupported_include_returns_400() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "input": "hi",
                        "include": ["code_interpreter_call.outputs"]
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "unsupported_feature");
    assert_eq!(json["error"]["code"], "unsupported_feature");
    assert_eq!(json["error"]["param"], "include");
}

#[tokio::test]
async fn test_responses_file_search_outputs_local_call() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [
                    {"message": {"role": "assistant", "content": "done"}}
                ],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }"#,
        )
        .await,
    );
    state
        .files
        .insert(
            "notes.md",
            "assistants",
            "text/markdown",
            b"relay integration notes".to_vec(),
            1,
        )
        .unwrap();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "input": "relay",
                        "tools": [{"type": "file_search"}]
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["output"][0]["type"], "file_search_call");
    assert_eq!(json["output"][0]["results"][0]["filename"], "notes.md");
    assert_eq!(json["output"][1]["type"], "message");
    assert!(json["metadata"]["local_file_search_results"].is_string());
}

#[tokio::test]
async fn test_responses_file_search_evidence_survives_retrieve_and_input_items() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [
                    {"message": {"role": "assistant", "content": "used local docs"}}
                ],
                "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
            }"#,
        )
        .await,
    );
    let first_file = state
        .files
        .insert(
            "relay.md",
            "assistants",
            "text/markdown",
            b"relay integration notes".to_vec(),
            1,
        )
        .unwrap();
    let first_file_id = first_file["id"].as_str().unwrap().to_string();
    state
        .files
        .insert(
            "other.md",
            "assistants",
            "text/markdown",
            b"relay integration notes outside vector store".to_vec(),
            1,
        )
        .unwrap();
    let store = state.vector_stores.create(
        Some("docs".into()),
        vec![first_file_id.clone()],
        json!({}),
        1,
    );
    let store_id = store["id"].as_str().unwrap().to_string();
    let sessions = state.sessions.clone();
    let app = build_router(state);

    let body = format!(
        r#"{{
            "model": "gpt-5",
            "input": "relay",
            "include": ["output[*].file_search_call.results"],
            "tools": [{{
                "type": "file_search",
                "vector_store_ids": ["{store_id}"],
                "max_num_results": 1,
                "ranking_options": {{"score_threshold": 1.0}}
            }}]
        }}"#
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let response_id = json["id"].as_str().unwrap().to_string();

    assert_eq!(json["output"][0]["type"], "file_search_call");
    assert_eq!(json["output"][0]["queries"][0]["query"], "relay");
    assert_eq!(json["output"][0]["vector_store_ids"][0], store_id);
    assert_eq!(json["output"][0]["results"].as_array().unwrap().len(), 1);
    assert_eq!(json["output"][0]["results"][0]["file_id"], first_file_id);
    assert_eq!(json["metadata"]["local_file_search_query"], "relay");
    let metadata_store_ids: Vec<String> = serde_json::from_str(
        json["metadata"]["local_file_search_vector_store_ids"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata_store_ids, vec![store_id.clone()]);
    assert_eq!(json["metadata"]["local_file_search_max_num_results"], "1");
    assert_eq!(json["metadata"]["local_file_search_score_threshold"], "1");

    let retrieve = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/responses/{response_id}?include=file_search_call.results"
                ))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retrieve.status(), StatusCode::OK);
    let retrieve_body = retrieve.into_body().collect().await.unwrap().to_bytes();
    let retrieved: Value = serde_json::from_slice(&retrieve_body).unwrap();
    assert_eq!(retrieved["output"][0]["type"], "file_search_call");
    assert_eq!(
        retrieved["output"][0]["results"][0]["file_id"],
        first_file_id
    );
    assert_eq!(retrieved["metadata"], json["metadata"]);

    let input_items = sessions
        .get_input_items(&response_id)
        .expect("input items should be stored");
    assert_eq!(input_items[0]["type"], "message");
    assert_eq!(input_items[1]["type"], "file_search_context");
    assert_eq!(input_items[1]["query"], "relay");
    assert_eq!(input_items[1]["vector_store_ids"][0], store_id);
    assert_eq!(input_items[1]["results"][0]["file_id"], first_file_id);
}

#[tokio::test]
async fn test_tool_policy_rejects_unlisted_mcp_server() {
    let mut state = test_state();
    state.tool_policy.allowed_mcp_servers = vec!["safe_server".into()];
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "input": "hi",
                        "tools": [{"type": "mcp", "server_label": "unsafe_server"}]
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["param"], "tools");
    assert_eq!(json["error"]["code"], "unsupported_feature");
}

#[tokio::test]
async fn test_get_response_unsupported_include_returns_400() {
    let state = test_state();
    state.sessions.save_response(
        "resp_include_check".into(),
        json!({
            "id": "resp_include_check",
            "object": "response",
            "status": "completed",
            "model": "gpt-5",
            "output": []
        }),
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/resp_include_check?include=code_interpreter_call.outputs")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["param"], "include");
    assert_eq!(json["error"]["code"], "unsupported_feature");
}

#[tokio::test]
async fn test_responses_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/nonexistent")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

// ── file API integration tests ──────────────────────────────────────────

fn multipart_body(
    purpose: &str,
    filename: &str,
    content_type: &str,
    data: &[u8],
) -> (String, Vec<u8>) {
    let boundary = "TESTBOUNDARY";
    let mut body = Vec::new();

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"purpose\"\r\n");
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(purpose.as_bytes());
    body.extend_from_slice(b"\r\n");

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(data);
    body.extend_from_slice(b"\r\n");

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    (format!("multipart/form-data; boundary={boundary}"), body)
}

#[tokio::test]
async fn test_files_upload() {
    let app = build_router(test_state());
    let (content_type, body) =
        multipart_body("assistants", "test.txt", "text/plain", b"Hello, World!");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files")
                .method(Method::POST)
                .header("content-type", &content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(json["id"].as_str().unwrap().starts_with("file_"));
    assert_eq!(json["object"], "file");
    assert_eq!(json["filename"], "test.txt");
    assert_eq!(json["bytes"], 13);
    assert_eq!(json["purpose"], "assistants");
    assert_eq!(json["content_type"], "text/plain");
    assert!(json["created_at"].as_u64().is_some());
}

#[tokio::test]
async fn test_files_upload_empty_file() {
    let app = build_router(test_state());
    let (content_type, body) = multipart_body("assistants", "empty.txt", "text/plain", b"");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files")
                .method(Method::POST)
                .header("content-type", &content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"]["code"], "empty_file");
}

#[tokio::test]
async fn test_files_list_empty() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["object"], "list");
    assert!(json["data"].as_array().unwrap().is_empty());
    assert_eq!(json["has_more"], false);
}

#[tokio::test]
async fn test_files_list_with_files() {
    let state = test_state();
    state
        .files
        .insert("a.txt", "assistants", "text/plain", b"aaa".to_vec(), 100)
        .unwrap();
    state
        .files
        .insert("b.txt", "assistants", "text/plain", b"bbb".to_vec(), 200)
        .unwrap();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["object"], "list");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["created_at"].as_u64(), Some(200));
    assert_eq!(data[1]["created_at"].as_u64(), Some(100));
}

#[tokio::test]
async fn test_files_get() {
    let state = test_state();
    let file = state
        .files
        .insert(
            "notes.md",
            "assistants",
            "text/markdown",
            b"hello".to_vec(),
            1,
        )
        .unwrap();
    let file_id = file["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["id"], file_id);
    assert_eq!(json["filename"], "notes.md");
    assert_eq!(json["bytes"], 5);
    assert_eq!(json["purpose"], "assistants");
    assert_eq!(json["content_type"], "text/markdown");
}

#[tokio::test]
async fn test_files_get_nonexistent() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files/nonexistent")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_files_get_content() {
    let state = test_state();
    let file = state
        .files
        .insert(
            "data.bin",
            "assistants",
            "application/octet-stream",
            b"\x00\x01\x02\xff".to_vec(),
            1,
        )
        .unwrap();
    let file_id = file["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}/content"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/octet-stream"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_bytes[..], b"\x00\x01\x02\xff");
}

#[tokio::test]
async fn test_files_get_content_nonexistent() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files/nonexistent/content")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_files_delete() {
    let state = test_state();
    let file = state
        .files
        .insert(
            "delete-me.txt",
            "assistants",
            "text/plain",
            b"bye".to_vec(),
            1,
        )
        .unwrap();
    let file_id = file["id"].as_str().unwrap().to_string();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/files/{file_id}"))
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["id"], file_id);
    assert_eq!(json["object"], "file.deleted");
    assert_eq!(json["deleted"], true);
}

#[tokio::test]
async fn test_files_delete_nonexistent() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/files/nonexistent")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

// ── translate_stream integration tests ──────────────────────────────────

type SseBody = Arc<String>;

fn parse_sse_events(body: &[u8]) -> Vec<(String, Value)> {
    let text = std::str::from_utf8(body).unwrap();
    text.split("\n\n")
        .filter(|s| !s.trim().is_empty())
        .map(|block| {
            let mut event_type = String::new();
            let mut data = String::new();
            for line in block.lines() {
                if let Some(val) = line.strip_prefix("event: ") {
                    event_type = val.to_string();
                } else if let Some(val) = line.strip_prefix("data: ") {
                    data = val.to_string();
                }
            }
            let data_value: Value = serde_json::from_str(&data).unwrap_or_default();
            (event_type, data_value)
        })
        .collect()
}

fn assert_sequence_numbers(events: &[(String, Value)]) {
    let mut last = 0_u64;
    for (event_type, payload) in events {
        let seq = payload["sequence_number"]
            .as_u64()
            .unwrap_or_else(|| panic!("{event_type} missing sequence_number"));
        assert!(
            seq > last,
            "{event_type} sequence_number {seq} did not increase after {last}"
        );
        last = seq;
    }
}

async fn mock_sse_handler(State(body): State<SseBody>) -> Response<Body> {
    let mut resp = Response::new(Body::from(body.as_str().to_string()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    resp
}

async fn start_mock_sse(body: String) -> reqwest::Url {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state: SseBody = Arc::new(body);
    tokio::spawn(async move {
        let app = Router::new()
            .route("/chat/completions", post(mock_sse_handler))
            .with_state(state);
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    reqwest::Url::parse(&format!("http://{addr}/chat/completions")).unwrap()
}

#[derive(Clone)]
struct RetryState {
    sse_body: String,
    call_count: Arc<AtomicUsize>,
}

async fn retry_handler(State(state): State<RetryState>) -> Response<Body> {
    let call = state.call_count.fetch_add(1, Ordering::SeqCst);
    if call == 0 {
        let mut resp = Response::new(Body::from("reasoning_content must be passed back"));
        *resp.status_mut() = StatusCode::BAD_REQUEST;
        resp
    } else {
        let mut resp = Response::new(Body::from(state.sse_body.clone()));
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        resp
    }
}

async fn start_mock_retry(sse_body: String) -> (reqwest::Url, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let call_count = Arc::new(AtomicUsize::new(0));
    let state = RetryState {
        sse_body,
        call_count: call_count.clone(),
    };
    tokio::spawn(async move {
        let app = Router::new()
            .route("/chat/completions", post(retry_handler))
            .with_state(state);
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    (
        reqwest::Url::parse(&format!("http://{addr}/chat/completions")).unwrap(),
        call_count,
    )
}

async fn unrecoverable_handler() -> Response<Body> {
    let mut resp = Response::new(Body::from("rate limit exceeded"));
    *resp.status_mut() = StatusCode::BAD_REQUEST;
    resp
}

async fn start_mock_unrecoverable() -> reqwest::Url {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let app = Router::new().route("/chat/completions", post(unrecoverable_handler));
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    reqwest::Url::parse(&format!("http://{addr}/chat/completions")).unwrap()
}

#[derive(Clone)]
struct RetryInspectState {
    sse_body: String,
    call_count: Arc<AtomicUsize>,
    captured_bodies: Arc<Mutex<Vec<String>>>,
}

async fn retry_inspect_handler(
    State(state): State<RetryInspectState>,
    body: Body,
) -> Response<Body> {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let body_str = String::from_utf8_lossy(&bytes).to_string();
    let call = state.call_count.fetch_add(1, Ordering::SeqCst);
    state.captured_bodies.lock().unwrap().push(body_str);
    if call == 0 {
        let mut resp = Response::new(Body::from("reasoning_content must be passed back"));
        *resp.status_mut() = StatusCode::BAD_REQUEST;
        resp
    } else {
        let mut resp = Response::new(Body::from(state.sse_body.clone()));
        resp.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        resp
    }
}

fn make_chat_req() -> ChatRequest {
    ChatRequest {
        model: "test-model".into(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: Some(json!("hello")),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        tools: vec![],
        temperature: None,
        top_p: None,
        max_tokens: None,
        stream: true,
        reasoning_effort: None,
        thinking: None,
        tool_choice: None,
        parallel_tool_calls: None,
        response_format: None,
        user: None,
        stream_options: None,
        web_search_options: None,
    }
}

fn make_stream_args(
    client: reqwest::Client,
    url: &str,
    store_response: bool,
    cache: Option<RequestCache>,
    cache_key: Option<u64>,
) -> StreamArgs {
    StreamArgs {
        client,
        url: url.to_string(),
        api_key: Arc::new(String::new()),
        chat_req: make_chat_req(),
        response_id: "test_resp_stream".into(),
        sessions: SessionStore::new(),
        prior_messages: vec![],
        request_messages: vec![ChatMessage {
            role: "user".into(),
            content: Some(json!("hello")),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        request_input_items: vec![],
        store_response,
        conversation_id: None,
        response_extra: json!({}),
        model: "test-model".into(),
        model_map: std::collections::HashMap::new(),
        cache,
        cache_key,
        token_tracker: Arc::new(deecodex::token_anomaly::TokenTracker::default()),
        metrics: Arc::new(deecodex::metrics::Metrics::new()),
    }
}

fn make_stream_args_custom(
    client: reqwest::Client,
    url: &str,
    store_response: bool,
    cache: Option<RequestCache>,
    cache_key: Option<u64>,
    chat_req: ChatRequest,
) -> StreamArgs {
    StreamArgs {
        client,
        url: url.to_string(),
        api_key: Arc::new(String::new()),
        chat_req,
        response_id: "test_resp_stream".into(),
        sessions: SessionStore::new(),
        prior_messages: vec![],
        request_messages: vec![ChatMessage {
            role: "user".into(),
            content: Some(json!("hello")),
            reasoning_content: None,
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }],
        request_input_items: vec![],
        store_response,
        conversation_id: None,
        response_extra: json!({}),
        model: "test-model".into(),
        model_map: std::collections::HashMap::new(),
        cache,
        cache_key,
        token_tracker: Arc::new(deecodex::token_anomaly::TokenTracker::default()),
        metrics: Arc::new(deecodex::metrics::Metrics::new()),
    }
}

fn build_sse_body(chunks: Vec<&str>) -> String {
    chunks.join("\n\n") + "\n\n"
}

#[tokio::test]
async fn test_translate_stream_simple_text() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" world"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    assert_eq!(events.len(), 6);
    assert_sequence_numbers(&events);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "message");
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["delta"], "Hello");
    assert_eq!(events[3].0, "response.output_text.delta");
    assert_eq!(events[3].1["delta"], " world");
    assert_eq!(events[4].0, "response.output_item.done");
    assert_eq!(events[4].1["item"]["type"], "message");
    assert_eq!(events[4].1["item"]["content"][0]["text"], "Hello world");
    assert_eq!(events[5].0, "response.completed");
    assert_eq!(events[5].1["response"]["status"], "completed");
}

#[tokio::test]
async fn test_translate_stream_tool_call() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"NYC\"}"}}]},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // created + (added + delta + done) + completed = 5
    assert_eq!(events.len(), 5);
    assert_sequence_numbers(&events);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "function_call");
    assert_eq!(events[1].1["item"]["name"], "get_weather");
    assert_eq!(events[1].1["item"]["call_id"], "call_abc");
    assert_eq!(events[1].1["item"]["status"], "in_progress");
    assert_eq!(events[2].0, "response.function_call_arguments.delta");
    assert_eq!(events[2].1["delta"], r#"{"city":"NYC"}"#);
    assert_eq!(events[3].0, "response.output_item.done");
    assert_eq!(events[3].1["item"]["type"], "function_call");
    assert_eq!(events[3].1["item"]["name"], "get_weather");
    assert_eq!(events[3].1["item"]["call_id"], "call_abc");
    assert_eq!(events[3].1["item"]["status"], "completed");
    assert_eq!(events[4].0, "response.completed");
}

#[tokio::test]
async fn test_translate_stream_reasoning() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"reasoning_content":"Let me think"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"Answer"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // created + reason(added+delta+done + msg(added+delta+done) + completed = 8
    assert_eq!(events.len(), 8);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "reasoning_summary");
    assert_eq!(events[2].0, "response.reasoning_summary_text.delta");
    assert_eq!(events[2].1["delta"], "Let me think");
    assert_eq!(events[3].0, "response.output_item.added");
    assert_eq!(events[3].1["item"]["type"], "message");
    assert_eq!(events[4].0, "response.output_text.delta");
    assert_eq!(events[4].1["delta"], "Answer");
    assert_eq!(events[5].0, "response.output_item.done");
    assert_eq!(events[5].1["item"]["type"], "reasoning");
    assert_eq!(events[5].1["item"]["content"][0]["text"], "Let me think");
    assert_eq!(events[6].0, "response.output_item.done");
    assert_eq!(events[6].1["item"]["type"], "message");
    assert_eq!(events[6].1["item"]["content"][0]["text"], "Answer");
    assert_eq!(events[7].0, "response.completed");
    assert_eq!(
        events[7].1["response"]["output"].as_array().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn test_translate_stream_error_recovery() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"Recovered"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let (url, call_count) = start_mock_retry(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // Should have made 2 calls (first = 400, second = success)
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    // Should have received a successful response
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["delta"], "Recovered");
    assert_eq!(events[events.len() - 1].0, "response.completed");
}

#[tokio::test]
async fn test_translate_stream_error_passthrough() {
    let url = start_mock_unrecoverable().await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // Only created + failed = 2
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.failed");
    assert_eq!(events[1].1["response"]["error"]["code"], "400");
    assert!(events[1].1["response"]["error"]["message"]
        .as_str()
        .unwrap()
        .contains("rate limit exceeded"));
}

#[tokio::test]
async fn test_translate_stream_cache_store_and_replay() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"Cached response"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let cache = RequestCache::new(128);
    let cache_key = RequestCache::hash_request(&make_chat_req());

    let args = make_stream_args(
        client,
        url.as_str(),
        true,
        Some(cache.clone()),
        Some(cache_key),
    );
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);
    assert_eq!(events.last().unwrap().0, "response.completed");

    // Verify cache has the entry
    let cached = cache.get(cache_key).expect("expected cached response");
    assert_eq!(cached.text, "Cached response");
    assert!(cached.usage.is_some());
    let usage = cached.usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 5);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 7);

    // Replay the cached response and verify
    let replay_sse = translate_cached(CachedArgs {
        response_id: "replay_resp".into(),
        model: "test-model".into(),
        cached: cached.clone(),
        sessions: SessionStore::new(),
        request_input_items: vec![],
        store_response: false,
        conversation_id: None,
        response_extra: json!({}),
    });
    let replay_bytes = axum::body::to_bytes(replay_sse.into_response().into_body(), usize::MAX)
        .await
        .unwrap();
    let replay_events = parse_sse_events(&replay_bytes);

    assert!(replay_events.len() > 3);
    assert_eq!(replay_events[0].0, "response.created");
    assert_eq!(replay_events[1].0, "response.output_item.added");
    assert_eq!(replay_events[1].1["item"]["type"], "message");
    assert_eq!(replay_events[2].0, "response.output_text.delta");
    assert_eq!(replay_events[2].1["delta"], "Cached response");
    assert_eq!(replay_events.last().unwrap().0, "response.completed");
}

#[tokio::test]
async fn test_translate_cached_reasoning_completed_output_matches_live_shape() {
    let replay_sse = translate_cached(CachedArgs {
        response_id: "replay_reasoning".into(),
        model: "test-model".into(),
        cached: deecodex::cache::CachedResponse {
            text: "Final answer".into(),
            reasoning: "Reasoned locally".into(),
            tool_calls: vec![],
            usage: None,
            created_at: 1,
        },
        sessions: SessionStore::new(),
        request_input_items: vec![],
        store_response: false,
        conversation_id: None,
        response_extra: json!({}),
    });
    let replay_bytes = axum::body::to_bytes(replay_sse.into_response().into_body(), usize::MAX)
        .await
        .unwrap();
    let events = parse_sse_events(&replay_bytes);
    let completed = events
        .iter()
        .find(|(name, _)| name == "response.completed")
        .expect("completed event");

    assert_sequence_numbers(&events);
    assert_eq!(events[1].1["item"]["type"], "reasoning_summary");
    assert_eq!(events[3].1["item"]["type"], "reasoning");
    assert_eq!(completed.1["response"]["output"][0]["type"], "reasoning");
    assert_eq!(
        completed.1["response"]["output"][0]["content"][0]["text"],
        "Reasoned locally"
    );
}

// ── Blocking (non-streaming) response tests ──────────────────────────────

#[tokio::test]
async fn test_responses_blocking_text() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [
                    {"message": {"role": "assistant", "content": "Hello, world!"}}
                ],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
            }"#,
        )
        .await,
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-5","input":"Hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["id"].as_str().unwrap().starts_with("resp_"));
    assert_eq!(json["status"], "completed");
    assert_eq!(json["model"], "gpt-5");
    assert_eq!(json["output"][0]["type"], "message");
    assert_eq!(json["output"][0]["content"][0]["text"], "Hello, world!");
    assert_eq!(json["usage"]["input_tokens"], 10);
    assert_eq!(json["usage"]["output_tokens"], 20);
    assert_eq!(json["usage"]["total_tokens"], 30);
}

#[tokio::test]
async fn test_responses_blocking_tool_call() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_abc",
                            "type": "function",
                            "function": {
                                "name": "get_weather",
                                "arguments": "{\"city\": \"NYC\"}"
                            }
                        }]
                    }
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }"#,
        )
        .await,
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-5","input":"weather?"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["output"][0]["type"], "function_call");
    assert_eq!(json["output"][0]["name"], "get_weather");
    assert!(json["output"][0]["call_id"]
        .as_str()
        .unwrap()
        .contains("call_abc"));
    assert_eq!(json["output"][0]["arguments"], r#"{"city": "NYC"}"#);
    assert_eq!(json["output"][0]["status"], "completed");
}

#[tokio::test]
async fn test_responses_blocking_reasoning() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "The answer is 42.",
                        "reasoning_content": "Let me think step by step."
                    }
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
            }"#,
        )
        .await,
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-5","input":"think"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["output"][0]["type"], "reasoning");
    assert_eq!(
        json["output"][0]["content"][0]["text"],
        "Let me think step by step."
    );
    assert_eq!(json["output"][0]["status"], "completed");
    assert_eq!(json["output"][1]["type"], "message");
    assert_eq!(json["output"][1]["content"][0]["text"], "The answer is 42.");
}

#[derive(Clone)]
struct CaptureJsonState {
    response_body: &'static str,
    captured: Arc<Mutex<Vec<Value>>>,
}

async fn capture_json_handler(State(state): State<CaptureJsonState>, body: Body) -> Response<Body> {
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    state.captured.lock().unwrap().push(value);
    let mut response = Response::new(Body::from(state.response_body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

async fn capture_json_upstream(
    response_body: &'static str,
) -> (reqwest::Url, Arc<Mutex<Vec<Value>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let state = CaptureJsonState {
        response_body,
        captured: captured.clone(),
    };
    tokio::spawn(async move {
        let app = Router::new()
            .route("/chat/completions", post(capture_json_handler))
            .with_state(state);
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    (
        reqwest::Url::parse(&format!("http://{addr}/")).unwrap(),
        captured,
    )
}

#[tokio::test]
async fn test_tool_call_outputs_are_normalized_for_upstream_and_input_items() {
    let (upstream, captured) = capture_json_upstream(
        r#"{
            "choices": [
                {"message": {"role": "assistant", "content": "continued"}}
            ],
            "usage": {"prompt_tokens": 3, "completion_tokens": 4, "total_tokens": 7}
        }"#,
    )
    .await;
    let mut state = test_state();
    state.upstream = Arc::new(upstream);
    let sessions = state.sessions.clone();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "input": [
                            {
                                "type": "computer_call_output",
                                "call_id": "call_screen",
                                "screenshot": "data:image/png;base64,abc",
                                "output": [{"type": "output_text", "text": "clicked button"}]
                            },
                            {
                                "type": "mcp_tool_call_output",
                                "call_id": "call_mcp",
                                "output": {"files": ["a.rs"], "ok": true}
                            }
                        ]
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let response_id = json["id"].as_str().unwrap().to_string();

    let captured = captured.lock().unwrap();
    let messages = captured[0]["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "tool");
    assert_eq!(messages[0]["tool_call_id"], "call_screen");
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("[image omitted: image/png base64 3B]"));
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains("clicked button"));
    assert_eq!(messages[1]["role"], "tool");
    assert_eq!(messages[1]["tool_call_id"], "call_mcp");
    assert_eq!(messages[1]["content"], r#"{"files":["a.rs"],"ok":true}"#);
    drop(captured);

    let input_items = sessions
        .get_input_items(&response_id)
        .expect("input items should be stored");
    assert_eq!(input_items[0]["type"], "computer_call_output");
    assert_eq!(input_items[0]["status"], "completed");
    assert_eq!(input_items[0]["output"][0]["text"], "clicked button");
    assert_eq!(input_items[1]["type"], "mcp_tool_call_output");
    assert_eq!(input_items[1]["status"], "completed");
    assert_eq!(input_items[1]["output"]["files"][0], "a.rs");
}

// ── Streaming edge case tests ────────────────────────────────────────────

#[tokio::test]
async fn test_responses_background_queued() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [
                    {"message": {"role": "assistant", "content": "done"}}
                ],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }"#,
        )
        .await,
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-5","input":"hi","background":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["id"].as_str().unwrap().starts_with("resp_"));
    assert_eq!(json["status"], "queued");
    assert_eq!(json["background"], true);
    assert_eq!(json["model"], "gpt-5");
}

#[tokio::test]
async fn test_responses_stream_store_and_retrieve() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"Stored stream"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let mut mock_url = start_mock_sse(sse_body).await;
    mock_url.set_path("");

    let mut state = test_state();
    state.upstream = Arc::new(mock_url);
    let sessions = state.sessions.clone();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-5","input":"store me","stream":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let events = parse_sse_events(&body);
    assert!(!events.is_empty());
    let response_id = events[0].1["response"]["id"].as_str().unwrap().to_string();
    assert!(response_id.starts_with("resp_"));
    assert_eq!(events.last().unwrap().0, "response.completed");

    // Retrieve the saved response via GET
    let mut get_state = test_state();
    get_state.sessions = sessions;
    let get_app = build_router(get_state);

    let get_resp = get_app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/responses/{}", response_id))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_body = get_resp.into_body().collect().await.unwrap().to_bytes();
    let saved: Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(saved["id"], response_id);
    assert_eq!(saved["status"], "completed");
    assert_eq!(saved["model"], "gpt-5");
}

#[tokio::test]
async fn test_get_response_stream_replay_preserves_echo_ids_and_sequence_cursor() {
    let state = test_state();
    let response_id = "resp_replay_contract";
    let item_id = "msg_replay_contract";
    state.sessions.save_response(
        response_id.into(),
        json!({
            "id": response_id,
            "object": "response",
            "status": "completed",
            "model": "gpt-5",
            "metadata": {"source": "test"},
            "output": [{
                "type": "message",
                "id": item_id,
                "role": "assistant",
                "status": "completed",
                "content": [{"type": "output_text", "text": "Replay me"}]
            }],
            "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}
        }),
    );
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/responses/{response_id}?stream=true"))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let events = parse_sse_events(&body);
    assert_sequence_numbers(&events);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["id"], item_id);
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["item_id"], item_id);
    assert_eq!(events[3].0, "response.output_item.done");
    assert_eq!(events[3].1["item"]["id"], item_id);
    assert_eq!(events[4].0, "response.completed");
    assert_eq!(events[4].1["response"]["id"], response_id);
    assert_eq!(events[4].1["response"]["output"][0]["id"], item_id);
    assert_eq!(events[4].1["response"]["metadata"]["source"], "test");
    assert_eq!(events[4].1["response"]["usage"]["total_tokens"], 3);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/responses/{response_id}?stream=true&starting_after=1"
                ))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let events_after = parse_sse_events(&body);
    assert_eq!(events_after[0].0, "response.output_item.added");
    assert_eq!(events_after[0].1["sequence_number"], 2);
}

#[tokio::test]
async fn test_response_cancel_queued_response() {
    let state = test_state();
    state.sessions.save_response(
        "resp_cancel_me".into(),
        json!({
            "id": "resp_cancel_me",
            "object": "response",
            "status": "queued",
            "model": "gpt-5",
            "output": []
        }),
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/resp_cancel_me/cancel")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["id"], "resp_cancel_me");
    assert_eq!(json["status"], "cancelled");
}

#[tokio::test]
async fn test_response_cancel_completed_conflict() {
    let state = test_state();
    state.sessions.save_response(
        "resp_done".into(),
        json!({
            "id": "resp_done",
            "object": "response",
            "status": "completed",
            "model": "gpt-5",
            "output": []
        }),
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/resp_done/cancel")
                .method(Method::POST)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "response_not_cancellable");
}

#[tokio::test]
async fn test_response_compact_uses_previous_input_items() {
    let state = test_state();
    state.sessions.save_input_items(
        "resp_prev".into(),
        vec![json!({
            "id": "item_prev",
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "previous"}]
        })],
    );
    let sessions = state.sessions.clone();
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/compact")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "previous_response_id": "resp_prev",
                        "input": "current",
                        "instructions": "compress"
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let response_id = json["id"].as_str().unwrap();
    assert_eq!(json["object"], "response.compacted");
    assert_eq!(json["status"], "completed");
    assert_eq!(json["instructions"], "compress");
    assert_eq!(json["input"][0]["id"], "item_prev");
    assert_eq!(json["input"][1]["content"][0]["text"], "current");

    let stored = sessions
        .get_response(response_id)
        .expect("compacted response should be stored");
    assert_eq!(stored["id"], response_id);
    assert_eq!(stored["input"].as_array().unwrap().len(), 2);
}

// ── Handler validation edge cases ────────────────────────────────────────

#[tokio::test]
async fn test_responses_simple_text_input() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [
                    {"message": {"role": "assistant", "content": "Yes"}}
                ],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }"#,
        )
        .await,
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-5","input":"is this valid?"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(json["id"].as_str().unwrap().starts_with("resp_"));
    assert_eq!(json["status"], "completed");
}

#[tokio::test]
async fn test_responses_previous_response_id_with_conversation() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "input": "hi",
                        "previous_response_id": "resp_abc",
                        "conversation": "conv_123"
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "invalid_request_error");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("previous_response_id and conversation cannot be used together"));
}

#[tokio::test]
async fn test_responses_top_logprobs_unsupported() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "model": "gpt-5",
                        "input": "hi",
                        "top_logprobs": 5
                    }"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["type"], "unsupported_feature");
    assert_eq!(json["error"]["code"], "unsupported_feature");
    assert_eq!(json["error"]["param"], "top_logprobs");
}

// ── Session/Response/Conversation CRUD integration tests ──────────────

#[tokio::test]
async fn test_delete_response_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/nonexistent")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_get_response_after_create() {
    let mut state = test_state();
    state.upstream = Arc::new(
        one_shot_upstream(
            r#"{
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "Hello, how can I help?"
                    }
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            }"#,
        )
        .await,
    );
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model": "gpt-5", "input": "hello"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let create_json: Value = serde_json::from_slice(&body).unwrap();
    let response_id = create_json["id"].as_str().unwrap().to_string();
    assert_eq!(create_json["object"], "response");
    assert_eq!(create_json["status"], "completed");
    assert_eq!(create_json["model"], "gpt-5");

    let app2 = build_router(state.clone());
    let get_response = app2
        .oneshot(
            Request::builder()
                .uri(format!("/v1/responses/{}", response_id))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(get_json["id"], response_id);
    assert_eq!(get_json["object"], "response");
    assert_eq!(get_json["status"], "completed");
    assert_eq!(get_json["model"], "gpt-5");
}

#[tokio::test]
async fn test_delete_existing_response() {
    let state = test_state();
    state.sessions.save_response(
        "resp_del_1".into(),
        json!({
            "id": "resp_del_1",
            "object": "response",
            "status": "completed",
            "model": "test-model",
            "output": []
        }),
    );
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/resp_del_1")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["id"], "resp_del_1");
    assert_eq!(json["object"], "response.deleted");
    assert_eq!(json["deleted"], true);
}

#[tokio::test]
async fn test_input_items() {
    let state = test_state();
    state.sessions.save_input_items("resp_input_1".into(), vec![
        json!({"id": "item_1", "role": "user", "content": [{"type": "input_text", "text": "hello"}]}),
        json!({"id": "item_2", "role": "user", "content": [{"type": "input_text", "text": "world"}]}),
    ]);
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/resp_input_1/input_items")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    // default order is "desc" — items are reversed
    assert_eq!(data[0]["id"], "item_2");
    assert_eq!(data[1]["id"], "item_1");
    assert_eq!(json["first_id"], "item_2");
    assert_eq!(json["last_id"], "item_1");
    assert_eq!(json["has_more"], false);
}

#[tokio::test]
async fn test_input_items_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/responses/nonexistent/input_items")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_create_conversation() {
    let state = test_state();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations")
                .method(Method::POST)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"metadata": {"key": "val"}}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let conv_id = json["id"].as_str().unwrap().to_string();
    assert!(conv_id.starts_with("conv_"));
    assert_eq!(json["object"], "conversation");
    assert_eq!(json["metadata"]["key"], "val");
    assert!(json["created_at"].as_u64().is_some());

    // Verify GET returns the conversation
    let app2 = build_router(state.clone());
    let get_response = app2
        .oneshot(
            Request::builder()
                .uri(format!("/v1/conversations/{}", conv_id))
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(get_response.status(), StatusCode::OK);
    let get_body = get_response.into_body().collect().await.unwrap().to_bytes();
    let get_json: Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(get_json["id"], conv_id);
    assert_eq!(get_json["object"], "conversation");
}

#[tokio::test]
async fn test_get_conversation() {
    let state = test_state();
    state
        .sessions
        .save_conversation("conv_get_1".into(), Vec::new());
    state
        .sessions
        .save_conversation_items("conv_get_1".into(), Vec::new());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations/conv_get_1")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["id"], "conv_get_1");
    assert_eq!(json["object"], "conversation");
}

#[tokio::test]
async fn test_get_conversation_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations/nonexistent")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_delete_conversation() {
    let state = test_state();
    state
        .sessions
        .save_conversation("conv_del_1".into(), Vec::new());
    state
        .sessions
        .save_conversation_items("conv_del_1".into(), Vec::new());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations/conv_del_1")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["id"], "conv_del_1");
    assert_eq!(json["object"], "conversation.deleted");
    assert_eq!(json["deleted"], true);
}

#[tokio::test]
async fn test_delete_conversation_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations/nonexistent")
                .method(Method::DELETE)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_conversation_items() {
    let state = test_state();
    state.sessions.save_conversation_items("conv_items_1".into(), vec![
        json!({"id": "item_1", "type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}),
        json!({"id": "item_2", "type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]}),
    ]);
    state
        .sessions
        .save_conversation("conv_items_1".into(), Vec::new());
    let app = build_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations/conv_items_1/items")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["object"], "list");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    // default order is "desc" — items are reversed
    assert_eq!(data[0]["id"], "item_2");
    assert_eq!(data[1]["id"], "item_1");
    assert_eq!(json["first_id"], "item_2");
    assert_eq!(json["last_id"], "item_1");
    assert_eq!(json["has_more"], false);
}

#[tokio::test]
async fn test_conversation_items_not_found() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/conversations/nonexistent/items")
                .method(Method::GET)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "not_found");
}

// ── Advanced translate_stream integration tests ──────────────────────────

#[tokio::test]
async fn test_translate_stream_long_text() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"The"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" quick"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" brown"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" fox"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" jumps"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" over"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" the"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" lazy"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" sleepy"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":" dog"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    let words = [
        "The", " quick", " brown", " fox", " jumps", " over", " the", " lazy", " sleepy", " dog",
    ];
    // created(1) + added(1) + 10 deltas(10) + done(1) + completed(1) = 14
    assert_eq!(events.len(), 14);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "message");
    for (i, word) in words.iter().enumerate() {
        assert_eq!(events[2 + i].0, "response.output_text.delta");
        assert_eq!(events[2 + i].1["delta"], *word);
    }
    assert_eq!(events[12].0, "response.output_item.done");
    assert_eq!(
        events[12].1["item"]["content"][0]["text"],
        "The quick brown fox jumps over the lazy sleepy dog"
    );
    assert_eq!(events[13].0, "response.completed");
    assert_eq!(events[13].1["response"]["status"], "completed");
}

#[tokio::test]
async fn test_translate_stream_multi_tool_interleaved() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"get_weather","arguments":""}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_xyz","function":{"name":"get_time","arguments":""}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"NYC\"}"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"zone\":"}}]},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"EST\"}"}}]},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // created(1) + tool0(added+delta+done=3) + tool1(added+delta+done=3) + completed(1) = 8
    assert_eq!(events.len(), 8);
    assert_eq!(events[0].0, "response.created");

    // Tool call 0
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "function_call");
    assert_eq!(events[1].1["item"]["name"], "get_weather");
    assert_eq!(events[1].1["item"]["call_id"], "call_abc");
    assert_eq!(events[1].1["item"]["status"], "in_progress");
    assert_eq!(events[2].0, "response.function_call_arguments.delta");
    assert_eq!(events[2].1["delta"], r#"{"city":"NYC"}"#);
    assert_eq!(events[3].0, "response.output_item.done");
    assert_eq!(events[3].1["item"]["type"], "function_call");
    assert_eq!(events[3].1["item"]["name"], "get_weather");
    assert_eq!(events[3].1["item"]["call_id"], "call_abc");
    assert_eq!(events[3].1["item"]["status"], "completed");
    assert_eq!(events[3].1["item"]["arguments"], r#"{"city":"NYC"}"#);

    // Tool call 1
    assert_eq!(events[4].0, "response.output_item.added");
    assert_eq!(events[4].1["item"]["type"], "function_call");
    assert_eq!(events[4].1["item"]["name"], "get_time");
    assert_eq!(events[4].1["item"]["call_id"], "call_xyz");
    assert_eq!(events[4].1["item"]["status"], "in_progress");
    assert_eq!(events[5].0, "response.function_call_arguments.delta");
    assert_eq!(events[5].1["delta"], r#"{"zone":"EST"}"#);
    assert_eq!(events[6].0, "response.output_item.done");
    assert_eq!(events[6].1["item"]["type"], "function_call");
    assert_eq!(events[6].1["item"]["name"], "get_time");
    assert_eq!(events[6].1["item"]["call_id"], "call_xyz");
    assert_eq!(events[6].1["item"]["status"], "completed");
    assert_eq!(events[6].1["item"]["arguments"], r#"{"zone":"EST"}"#);

    assert_eq!(events[7].0, "response.completed");
}

#[tokio::test]
async fn test_translate_stream_thinking_enabled() {
    let chat_req = ChatRequest {
        thinking: Some(json!({"type": "enabled"})),
        ..make_chat_req()
    };

    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"reasoning_content":"Let me think"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"Answer"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args_custom(client, url.as_str(), false, None, None, chat_req);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // created + reason(added+delta+done) + msg(added+delta+done) + completed = 8
    assert_eq!(events.len(), 8);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "reasoning_summary");
    assert_eq!(events[2].0, "response.reasoning_summary_text.delta");
    assert_eq!(events[2].1["delta"], "Let me think");
    assert_eq!(events[3].0, "response.output_item.added");
    assert_eq!(events[3].1["item"]["type"], "message");
    assert_eq!(events[4].0, "response.output_text.delta");
    assert_eq!(events[4].1["delta"], "Answer");
    assert_eq!(events[5].0, "response.output_item.done");
    assert_eq!(events[5].1["item"]["type"], "reasoning");
    assert_eq!(events[5].1["item"]["content"][0]["text"], "Let me think");
    assert_eq!(events[6].0, "response.output_item.done");
    assert_eq!(events[6].1["item"]["type"], "message");
    assert_eq!(events[6].1["item"]["content"][0]["text"], "Answer");
    assert_eq!(events[7].0, "response.completed");
    assert_eq!(
        events[7].1["response"]["output"].as_array().unwrap().len(),
        2
    );
}

#[tokio::test]
async fn test_translate_stream_thinking_disabled() {
    let chat_req = ChatRequest {
        thinking: Some(json!({"type": "disabled"})),
        ..make_chat_req()
    };

    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"No reasoning"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args_custom(client, url.as_str(), false, None, None, chat_req);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // created + added + delta + done + completed = 5
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "message");
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["delta"], "No reasoning");
    assert_eq!(events[3].0, "response.output_item.done");
    assert_eq!(events[4].0, "response.completed");
}

#[tokio::test]
async fn test_translate_stream_empty_content_delta() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":""},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":"Real content"},"finish_reason":null}]}"#,
        r#"data: {"choices":[{"delta":{"content":null},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args(client, url.as_str(), false, None, None);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // Only the "Real content" chunk should produce a delta event.
    // created + added + delta + done + completed = 5
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "message");
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["delta"], "Real content");
    assert_eq!(events[3].0, "response.output_item.done");
    assert_eq!(events[3].1["item"]["content"][0]["text"], "Real content");
    assert_eq!(events[4].0, "response.completed");
}

#[tokio::test]
async fn test_translate_stream_thinking_disabled_retry() {
    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"Recovered after retry"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let call_count = Arc::new(AtomicUsize::new(0));
    let captured_bodies: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let state = RetryInspectState {
        sse_body: sse_body.clone(),
        call_count: call_count.clone(),
        captured_bodies: captured_bodies.clone(),
    };
    tokio::spawn(async move {
        let app = Router::new()
            .route("/chat/completions", post(retry_inspect_handler))
            .with_state(state);
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    });
    let url = reqwest::Url::parse(&format!("http://{addr}/chat/completions")).unwrap();

    let chat_req = ChatRequest {
        thinking: Some(json!({"type": "enabled"})),
        ..make_chat_req()
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap();

    let args = make_stream_args_custom(client, url.as_str(), false, None, None, chat_req);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    let bodies = captured_bodies.lock().unwrap();
    let second_body = &bodies[1];
    let req_json: Value = serde_json::from_str(second_body).unwrap();
    assert_eq!(req_json["thinking"]["type"], "disabled");
    assert!(
        req_json.get("reasoning_effort").is_none(),
        "reasoning_effort should be omitted in retry"
    );

    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["delta"], "Recovered after retry");
    assert_eq!(events[events.len() - 1].0, "response.completed");
}

#[tokio::test]
async fn test_translate_stream_web_search() {
    let chat_req = ChatRequest {
        web_search_options: Some(json!({"search_context_size": "high"})),
        ..make_chat_req()
    };

    let sse_body = build_sse_body(vec![
        r#"data: {"choices":[{"delta":{"content":"Web search result with [citation]"},"finish_reason":null}]}"#,
        "data: [DONE]",
    ]);

    let url = start_mock_sse(sse_body).await;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();

    let args = make_stream_args_custom(client, url.as_str(), false, None, None, chat_req);
    let bytes = axum::body::to_bytes(
        translate_stream(args).into_response().into_body(),
        usize::MAX,
    )
    .await
    .unwrap();
    let events = parse_sse_events(&bytes);

    // created + added + delta + done + completed = 5
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[1].0, "response.output_item.added");
    assert_eq!(events[1].1["item"]["type"], "message");
    assert_eq!(events[2].0, "response.output_text.delta");
    assert_eq!(events[2].1["delta"], "Web search result with [citation]");
    assert_eq!(events[3].0, "response.output_item.done");
    assert_eq!(
        events[3].1["item"]["content"][0]["text"],
        "Web search result with [citation]"
    );
    assert_eq!(events[4].0, "response.completed");
}
