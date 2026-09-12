//! Transport- and runtime-independent MCP JSON-RPC dispatch.
//!
//! This crate deliberately owns no HTTP, WebSocket, stdio, Tokio, rmcp,
//! repository, or provider code. A native adapter and a browser binding can
//! both place their application implementation behind [`McpApplication`].

use serde_json::{json, Value};

pub const JSON_RPC_VERSION: &str = "2.0";
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Application semantics behind an MCP transport.
///
/// The dispatcher owns JSON-RPC framing and initialization state. Implementors
/// own only MCP application methods and return JSON-compatible MCP result
/// values; errors are converted to JSON-RPC error responses consistently.
pub trait McpApplication {
    fn initialize(&mut self, params: &Value) -> Result<Value, McpApplicationError>;

    fn call(&mut self, method: &str, params: Option<&Value>) -> Result<Value, McpApplicationError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpApplicationError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl McpApplicationError {
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }
}

/// A single-client MCP session. Construct one for each stdio connection or
/// browser executor lifetime; it intentionally has no persistence.
pub struct McpDispatcher<A> {
    application: A,
    initialized: bool,
}

impl<A: McpApplication> McpDispatcher<A> {
    pub fn new(application: A) -> Self {
        Self {
            application,
            initialized: false,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn application(&self) -> &A {
        &self.application
    }

    pub fn application_mut(&mut self) -> &mut A {
        &mut self.application
    }

    /// Dispatch one JSON-RPC message. Notifications yield `None`; callers are
    /// responsible for translating that to their transport's empty response.
    /// JSON-RPC batches are deliberately rejected by the pinned profile.
    pub fn dispatch(&mut self, message: Value) -> Option<Value> {
        if message.is_array() {
            return Some(error(
                Value::Null,
                -32600,
                "JSON-RPC batches are not supported",
                None,
            ));
        }
        let Some(object) = message.as_object() else {
            return Some(error(Value::Null, -32600, "Invalid Request", None));
        };
        if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
            return Some(error(Value::Null, -32600, "Invalid Request", None));
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(error(Value::Null, -32600, "Invalid Request", None));
        };
        if object
            .get("params")
            .is_some_and(|params| !params.is_object() && !params.is_array())
        {
            return Some(error(Value::Null, -32600, "Invalid Request", None));
        }
        let id = object.get("id").cloned();
        if id
            .as_ref()
            .is_some_and(|id| !id.is_null() && !id.is_string() && !id.is_number())
        {
            return Some(error(Value::Null, -32600, "Invalid Request", None));
        }
        let notification = id.is_none();
        if method == "initialize" && notification {
            return Some(error(
                Value::Null,
                -32600,
                "initialize must be a request",
                None,
            ));
        }
        if method == "notifications/initialized" && !notification {
            return Some(error(
                id.unwrap_or(Value::Null),
                -32600,
                "notifications/initialized must be a notification",
                None,
            ));
        }
        let id = id.unwrap_or(Value::Null);
        let params = object.get("params");

        let result = if method == "initialize" {
            if self.initialized {
                Err(McpApplicationError::invalid_params(
                    "MCP session is already initialized",
                ))
            } else if params
                .and_then(|value| value.get("protocolVersion"))
                .and_then(Value::as_str)
                != Some(MCP_PROTOCOL_VERSION)
            {
                Err(McpApplicationError::invalid_params(format!(
                    "unsupported MCP protocol version; expected {MCP_PROTOCOL_VERSION}"
                )))
            } else {
                let result = self.application.initialize(params.unwrap_or(&Value::Null));
                if result.is_ok() {
                    self.initialized = true;
                }
                result
            }
        } else if method == "notifications/initialized" {
            if self.initialized {
                Ok(Value::Null)
            } else {
                Err(McpApplicationError::invalid_params(
                    "MCP session is not initialized",
                ))
            }
        } else if !self.initialized {
            Err(McpApplicationError::invalid_params(
                "MCP session is not initialized",
            ))
        } else {
            self.application.call(method, params)
        };

        if notification {
            return None;
        }
        Some(match result {
            Ok(result) => json!({ "jsonrpc": JSON_RPC_VERSION, "id": id, "result": result }),
            Err(error_data) => error(id, error_data.code, &error_data.message, error_data.data),
        })
    }
}

fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = serde_json::Map::from_iter([
        ("code".to_string(), Value::Number(code.into())),
        ("message".to_string(), Value::String(message.to_string())),
    ]);
    if let Some(data) = data {
        error.insert("data".to_string(), data);
    }
    json!({ "jsonrpc": JSON_RPC_VERSION, "id": id, "error": error })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct EchoApplication;

    impl McpApplication for EchoApplication {
        fn initialize(&mut self, _params: &Value) -> Result<Value, McpApplicationError> {
            Ok(json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "test", "version": "0" }
            }))
        }

        fn call(
            &mut self,
            method: &str,
            params: Option<&Value>,
        ) -> Result<Value, McpApplicationError> {
            match method {
                "tools/list" => Ok(json!({ "tools": [] })),
                "echo" => Ok(params.cloned().unwrap_or(Value::Null)),
                _ => Err(McpApplicationError {
                    code: -32601,
                    message: "Method not found".to_string(),
                    data: None,
                }),
            }
        }
    }

    fn request(id: Value, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_notification_and_tool_round_trip() {
        let mut dispatcher = McpDispatcher::new(EchoApplication);
        let initialized = dispatcher.dispatch(request(
            json!(1),
            "initialize",
            json!({ "protocolVersion": MCP_PROTOCOL_VERSION }),
        ));
        assert_eq!(
            initialized.unwrap()["result"]["protocolVersion"],
            MCP_PROTOCOL_VERSION
        );
        assert!(dispatcher.is_initialized());
        assert_eq!(
            dispatcher.dispatch(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
            None
        );
        assert_eq!(
            dispatcher
                .dispatch(request(json!(2), "echo", json!({ "value": "browser" })))
                .unwrap()["result"],
            json!({ "value": "browser" })
        );
    }

    #[test]
    fn rejects_batches_and_calls_before_initialization() {
        let mut dispatcher = McpDispatcher::new(EchoApplication);
        assert_eq!(
            dispatcher.dispatch(json!([])).unwrap()["error"]["code"],
            -32600
        );
        assert_eq!(
            dispatcher
                .dispatch(request(json!(1), "tools/list", json!({})))
                .unwrap()["error"]["code"],
            -32602
        );
    }

    #[test]
    fn invalid_initialize_version_does_not_open_session() {
        let mut dispatcher = McpDispatcher::new(EchoApplication);
        assert_eq!(
            dispatcher
                .dispatch(request(
                    json!(1),
                    "initialize",
                    json!({ "protocolVersion": "2024-11-05" })
                ))
                .unwrap()["error"]["code"],
            -32602
        );
        assert!(!dispatcher.is_initialized());
    }

    #[test]
    fn rejects_invalid_request_ids_params_and_initialize_notifications() {
        let mut dispatcher = McpDispatcher::new(EchoApplication);
        for invalid in [
            json!({ "jsonrpc": "2.0", "id": {}, "method": "initialize", "params": {} }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": true }),
            json!({ "jsonrpc": "2.0", "method": "initialize", "params": { "protocolVersion": MCP_PROTOCOL_VERSION } }),
        ] {
            assert_eq!(
                dispatcher.dispatch(invalid).unwrap()["error"]["code"],
                -32600
            );
        }
        assert!(!dispatcher.is_initialized());
    }
}
