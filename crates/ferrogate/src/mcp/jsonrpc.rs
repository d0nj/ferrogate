use serde_json::{json, Value};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

#[derive(Debug, Clone)]
pub struct Reply {
    pub id: Value,
    pub outcome: Result<Value, RpcError>,
}

impl Reply {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            id,
            outcome: Ok(result),
        }
    }

    pub fn error(id: Value, error: RpcError) -> Self {
        Self {
            id,
            outcome: Err(error),
        }
    }

    pub fn into_value(self) -> Value {
        match self.outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": self.id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": self.id, "error": error.into_value() }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(code: i64, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    pub fn into_value(self) -> Value {
        let mut object = json!({ "code": self.code, "message": self.message });
        if let Some(data) = self.data {
            object["data"] = data;
        }
        object
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: Option<Value>,
    pub method: String,
    pub params: Value,
}

impl Request {
    fn from_value(value: &Value) -> Result<Self, RpcError> {
        let object = value
            .as_object()
            .ok_or_else(|| RpcError::new(INVALID_REQUEST, "request must be a JSON object"))?;

        if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(RpcError::new(INVALID_REQUEST, "jsonrpc must be \"2.0\""));
        }

        let id = match object.get("id") {
            None | Some(Value::Null) => None,
            Some(Value::String(_)) | Some(Value::Number(_)) => Some(object["id"].clone()),
            Some(_) => {
                return Err(RpcError::new(
                    INVALID_REQUEST,
                    "id must be a string, number or null",
                ))
            }
        };

        let method = object
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(INVALID_REQUEST, "method must be a string"))?
            .to_string();

        let params = object.get("params").cloned().unwrap_or(Value::Null);
        if !params.is_null() && !params.is_object() && !params.is_array() {
            return Err(RpcError::new(
                INVALID_PARAMS,
                "params must be an object or array",
            ));
        }

        Ok(Self { id, method, params })
    }
}

#[derive(Debug)]
pub enum Envelope {
    Single(Request),
    Batch(Vec<Request>),
}

const UNPARSED_ID: Value = Value::Null;

pub fn parse_body(raw: &[u8]) -> Result<Envelope, Reply> {
    let value: Value = serde_json::from_slice(raw)
        .map_err(|err| Reply::error(UNPARSED_ID, RpcError::new(PARSE_ERROR, err.to_string())))?;

    match value {
        Value::Array(items) => {
            if items.is_empty() {
                return Err(Reply::error(
                    UNPARSED_ID,
                    RpcError::new(INVALID_REQUEST, "empty batch"),
                ));
            }

            let mut requests = Vec::with_capacity(items.len());
            for item in &items {
                let request = Request::from_value(item).map_err(|error| {
                    let id = item.get("id").cloned().unwrap_or(UNPARSED_ID);
                    Reply::error(id, error)
                })?;
                requests.push(request);
            }
            Ok(Envelope::Batch(requests))
        }
        other => Request::from_value(&other)
            .map(Envelope::Single)
            .map_err(|error| {
                let id = other.get("id").cloned().unwrap_or(UNPARSED_ID);
                Reply::error(id, error)
            }),
    }
}

pub fn tool_result(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

pub fn string_param(params: &Value, key: &str) -> Option<String> {
    params.get(key).and_then(Value::as_str).map(str::to_string)
}
