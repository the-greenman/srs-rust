//! JSON-RPC-facing adapter for the complete SRS MCP surface.
//!
//! This is a transitional parity adapter: it maps the existing, tested rmcp
//! result models to JSON values for `srs-mcp-core`. The next move transfers
//! these service calls into the wasm-safe crate; keeping this adapter explicit
//! prevents the native stdio transport from becoming the browser contract.

use rmcp::model::JsonObject;
use rmcp::ErrorData as McpError;
use serde_json::Value;
use srs_mcp_core::{McpApplication as JsonApplication, McpApplicationError};

use crate::McpApplication;

pub struct JsonSrsApplication<'store> {
    application: McpApplication<'store>,
}

impl<'store> JsonSrsApplication<'store> {
    pub fn new(application: McpApplication<'store>) -> Self {
        Self { application }
    }
}

fn object(params: Option<&Value>) -> Result<JsonObject, McpApplicationError> {
    match params {
        None | Some(Value::Null) => Ok(JsonObject::default()),
        Some(Value::Object(object)) => Ok(object.clone()),
        Some(_) => Err(McpApplicationError::invalid_params(
            "params must be an object",
        )),
    }
}

fn required_string(params: Option<&Value>, name: &str) -> Result<String, McpApplicationError> {
    object(params)?
        .remove(name)
        .and_then(|value| value.as_str().map(ToString::to_string))
        .ok_or_else(|| McpApplicationError::invalid_params(format!("{name} must be a string")))
}

fn result(value: impl serde::Serialize) -> Result<Value, McpApplicationError> {
    serde_json::to_value(value).map_err(|error| McpApplicationError::internal(error.to_string()))
}

fn mcp_error(error: McpError) -> McpApplicationError {
    let encoded = serde_json::to_value(&error).unwrap_or(Value::Null);
    McpApplicationError {
        code: encoded
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or(-32603),
        message: encoded
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Internal error")
            .to_string(),
        data: encoded.get("data").cloned().filter(|data| !data.is_null()),
    }
}

impl JsonApplication for JsonSrsApplication<'_> {
    fn initialize(&mut self, _params: &Value) -> Result<Value, McpApplicationError> {
        result(McpApplication::server_info())
    }

    fn call(&mut self, method: &str, params: Option<&Value>) -> Result<Value, McpApplicationError> {
        let response = match method {
            "resources/list" => result(self.application.list_resources().map_err(mcp_error)?)?,
            "resources/templates/list" => result(self.application.list_resource_templates())?,
            "resources/read" => {
                let uri = required_string(params, "uri")?;
                result(self.application.read_resource(&uri).map_err(mcp_error)?)?
            }
            "tools/list" => result(self.application.list_tools())?,
            "tools/call" => {
                let mut fields = object(params)?;
                let name = fields
                    .remove("name")
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| McpApplicationError::invalid_params("name must be a string"))?;
                let arguments = match fields.remove("arguments") {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(arguments)) => Some(arguments),
                    Some(_) => {
                        return Err(McpApplicationError::invalid_params(
                            "arguments must be an object",
                        ))
                    }
                };
                result(
                    self.application
                        .call_tool(&name, arguments)
                        .map_err(mcp_error)?,
                )?
            }
            "prompts/list" => result(self.application.list_prompts().map_err(mcp_error)?)?,
            "prompts/get" => {
                let mut fields = object(params)?;
                let name = fields
                    .remove("name")
                    .and_then(|value| value.as_str().map(ToString::to_string))
                    .ok_or_else(|| McpApplicationError::invalid_params("name must be a string"))?;
                let arguments = match fields.remove("arguments") {
                    None | Some(Value::Null) => None,
                    Some(Value::Object(arguments)) => Some(arguments),
                    Some(_) => {
                        return Err(McpApplicationError::invalid_params(
                            "arguments must be an object",
                        ))
                    }
                };
                result(
                    self.application
                        .get_prompt(&name, arguments.as_ref())
                        .map_err(mcp_error)?,
                )?
            }
            _ => {
                return Err(McpApplicationError {
                    code: -32601,
                    message: format!("Method not found: {method}"),
                    data: None,
                })
            }
        };
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_mcp_core::{McpDispatcher, MCP_PROTOCOL_VERSION};
    use srs_repository::repository_lifecycle::{
        create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    };
    use srs_repository::store::FileStore;

    fn message(id: i64, method: &str, params: Value) -> Value {
        serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn raw_json_dispatcher_runs_catalogue_read_and_validated_write() {
        let directory = tempfile::tempdir().unwrap();
        let store = FileStore::new(directory.path());
        create_repository(
            &store,
            &InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "json-dispatch-test".to_string(),
                    namespace: "com.example.jsondispatch".to_string(),
                    srs_version: "2.0".to_string(),
                    title: Some("JSON dispatch fixture".to_string()),
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "test-package".to_string(),
                    namespace: "com.example.jsondispatch".to_string(),
                    name: "fixture".to_string(),
                    version: "1.0.0".to_string(),
                },
            },
        )
        .unwrap();
        let application = McpApplication::new(&store, "json-dispatch-test");
        let mut dispatcher = McpDispatcher::new(JsonSrsApplication::new(application));

        let initialized = dispatcher
            .dispatch(message(
                1,
                "initialize",
                serde_json::json!({ "protocolVersion": MCP_PROTOCOL_VERSION }),
            ))
            .unwrap();
        assert_eq!(initialized["result"]["serverInfo"]["name"], "srs-mcp");
        assert!(
            dispatcher
                .dispatch(message(2, "tools/list", serde_json::json!({})))
                .unwrap()["result"]["tools"]
                .as_array()
                .unwrap()
                .len()
                >= 19
        );
        assert!(dispatcher
            .dispatch(message(3, "resources/list", serde_json::json!({})))
            .unwrap()["result"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["uri"] == "srs://json-dispatch-test/map"));

        let created = dispatcher
            .dispatch(message(
                4,
                "tools/call",
                serde_json::json!({
                    "name": "note_create",
                    "arguments": {
                        "title": "Browser-dispatched note",
                        "sections": [{ "name": "body", "content": "unsaved browser mutation" }]
                    }
                }),
            ))
            .unwrap();
        assert_eq!(created["result"]["isError"], false, "{created}");
        assert!(created["result"]["structuredContent"]["instanceId"].is_string());
    }
}
