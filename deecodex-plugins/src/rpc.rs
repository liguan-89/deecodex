use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

impl JsonRpcMessage {
    pub fn from_line(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let val: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let has_id = val.get("id").is_some();
        let has_method = val.get("method").is_some();

        match (has_id, has_method) {
            (true, true) => serde_json::from_value::<JsonRpcRequest>(val)
                .ok()
                .map(JsonRpcMessage::Request),
            (true, false) => serde_json::from_value::<JsonRpcResponse>(val)
                .ok()
                .map(JsonRpcMessage::Response),
            (false, true) => serde_json::from_value::<JsonRpcNotification>(val)
                .ok()
                .map(JsonRpcMessage::Notification),
            _ => None,
        }
    }

    pub fn to_line(&self) -> String {
        let s = match self {
            JsonRpcMessage::Request(r) => serde_json::to_string(r),
            JsonRpcMessage::Response(r) => serde_json::to_string(r),
            JsonRpcMessage::Notification(n) => serde_json::to_string(n),
        };
        s.unwrap_or_default()
    }

    pub fn request_id(&self) -> Option<u64> {
        match self {
            JsonRpcMessage::Request(r) => Some(r.id),
            JsonRpcMessage::Response(r) => Some(r.id),
            JsonRpcMessage::Notification(_) => None,
        }
    }
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }
}

impl JsonRpcResponse {
    pub fn success(id: u64, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: u64, code: i64, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}
