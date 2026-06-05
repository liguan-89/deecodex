use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

pub const NATIVE_OBSERVE_TURNS: u8 = 1;
pub const NATIVE_OBSERVE_TTL_SECS: u64 = 10 * 60;

#[derive(Clone, Debug)]
pub struct MainModelAnchor {
    pub account_id: String,
    pub endpoint_id: String,
    pub model: String,
    pub endpoint_kind: String,
}

#[derive(Clone, Debug)]
pub struct RouteState {
    pub observe_remaining: u8,
    pub expires_at: u64,
    pub main_model_anchor: Option<MainModelAnchor>,
}

#[derive(Clone, Debug)]
pub struct ResponseFeedback {
    pub key: String,
    pub reason: &'static str,
    pub refreshed: bool,
}

pub type RouteStateMap = Arc<DashMap<String, RouteState>>;

pub fn refresh_native_track(
    sessions: &RouteStateMap,
    key: &str,
    now: u64,
    reason: &'static str,
) -> ResponseFeedback {
    sessions.insert(
        key.to_string(),
        RouteState {
            observe_remaining: NATIVE_OBSERVE_TURNS,
            expires_at: now.saturating_add(NATIVE_OBSERVE_TTL_SECS),
            main_model_anchor: sessions
                .get(key)
                .and_then(|state| state.main_model_anchor.clone()),
        },
    );
    ResponseFeedback {
        key: key.to_string(),
        reason,
        refreshed: true,
    }
}

pub fn response_has_native_signal(value: &Value) -> bool {
    value_has_type(value, &["computer_call", "computer_call_output"])
        || value_has_key(value, "screenshot")
}

pub fn maybe_refresh_from_response(
    sessions: Option<&RouteStateMap>,
    route_key: Option<&str>,
    response: &Value,
    now: u64,
) -> Option<ResponseFeedback> {
    if !response_has_native_signal(response) {
        return None;
    }
    let sessions = sessions?;
    let route_key = route_key?;
    Some(refresh_native_track(
        sessions,
        route_key,
        now,
        "response.computer_signal",
    ))
}

fn value_has_type(value: &Value, expected: &[&str]) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| value_has_type(item, expected)),
        Value::Object(map) => {
            map.get("type")
                .and_then(Value::as_str)
                .is_some_and(|typ| expected.contains(&typ))
                || map.values().any(|value| value_has_type(value, expected))
        }
        _ => false,
    }
}

fn value_has_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Array(items) => items.iter().any(|item| value_has_key(item, key)),
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|value| value_has_key(value, key))
        }
        _ => false,
    }
}
