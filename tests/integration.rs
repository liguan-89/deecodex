use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use deecodex::handlers::{build_router, AppState};
use http_body_util::BodyExt;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;

fn test_state() -> AppState {
    AppState {
        sessions: deecodex::session::SessionStore::new(),
        client: reqwest::Client::builder().build().unwrap(),
        upstream: Arc::new(reqwest::Url::parse("https://example.com").unwrap()),
        api_key: Arc::new("test".into()),
        model_map: Arc::new(std::collections::HashMap::new()),
        vision_upstream: None,
        vision_api_key: Arc::new(String::new()),
        vision_model: Arc::new("test".into()),
        vision_endpoint: Arc::new("v1/test".into()),
        start_time: Instant::now(),
        request_cache: deecodex::cache::RequestCache::default(),
        background_tasks: Arc::new(dashmap::DashMap::new()),
        chinese_thinking: false,
    }
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
