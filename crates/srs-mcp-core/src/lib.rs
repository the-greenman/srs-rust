//! Transport- and runtime-independent MCP JSON-RPC dispatch.
//!
//! This crate deliberately owns no HTTP, WebSocket, stdio, Tokio, rmcp, or
//! provider code. It owns the generic SRS contract over an injected
//! [`srs_repository::store::RepositoryStore`], so native and browser adapters
//! can execute the same application semantics behind [`McpApplication`].

use serde_json::{json, Value};

/// SRS-specific MCP metadata shared by native and browser adapters.
///
/// This is JSON rather than an `rmcp` model deliberately: the browser is an
/// equal MCP host, and the native adapter is responsible only for converting
/// this contract to its transport library's types.
pub mod srs_metadata {
    use serde_json::{json, Value};

    const BUILD_NUMBER: Option<&str> = option_env!("SRS_BUILD_NUMBER");
    const MIN_SUPPORTED_DATA_MODEL_REVISION: u32 = 2;

    const INSTRUCTIONS: &str = "This server exposes one SRS (Semantic Record System) repository. \
Orient before writing: read srs://<repositoryId>/map for counts and package info, and \
srs://<repositoryId>/navigation for the document structure. srs://<repositoryId>/agent-index is \
the one-page AI orientation index (identity, counts, types, sections, entry points). \
srs://<repositoryId>/tree is the recursive contains-tree from every root, and \
srs://<repositoryId>/tree/{instanceId} the subtree under one instance — descend from a \
navigation section or container member by its instanceId; container members also carry \
sectionContainerId when they root a sub-container. Read individual records via the \
srs://<repositoryId>/record/{instanceId} resource template, containers via \
srs://<repositoryId>/container/<containerId>, and rendered document views via \
srs://<repositoryId>/view/<compositionId>. Type schemas live at \
srs://<repositoryId>/type/{typeId} (also via the type_schema tool): read one before \
authoring records of an unfamiliar type — its properties are keyed by Field.name (the \
same keys record_create fieldValues uses, RFC-039) and carry aiGuidance. Protocols (staged \
processes) live at srs://<repositoryId>/protocol (list) and \
srs://<repositoryId>/protocol/{protocolId} (definition plus stages in order). Use the find tool for structured discovery \
(type, tag, lifecycle, tier, container, content match). Writes are validated: record_create, \
relation_create, and note_create enforce the repository's type and relation contracts and \
return diagnostics on rejection. Run repo_validate after a write batch and check its \
summary: summary.errors == 0 means the repository is consistent. Warnings are non-blocking, \
but review them. An empty diagnostics array means the repository is completely clean. \
Prompts: this server exposes one MCP prompt per installed blueprint. Call prompts/list \
to discover available blueprints; call prompts/get with a blueprint UUID to retrieve its \
full brief as rendered markdown — AI guidance, required types, structure, and protocol.";

    pub fn release_version() -> String {
        let base = env!("CARGO_PKG_VERSION");
        match BUILD_NUMBER {
            Some(number) if !number.is_empty() => format!("{base}+build.{number}"),
            _ => format!("{base}+dev"),
        }
    }

    pub fn release_generation_description() -> String {
        format!(
            "supports data-model generation >= {MIN_SUPPORTED_DATA_MODEL_REVISION} (RFC-038 [R21])"
        )
    }

    pub const fn instructions() -> &'static str {
        INSTRUCTIONS
    }

    /// The JSON result returned from the MCP `initialize` request.
    pub fn initialize_result() -> Value {
        json!({
            "protocolVersion": super::MCP_PROTOCOL_VERSION,
            "capabilities": {
                "prompts": {},
                "resources": {},
                "tools": {}
            },
            "serverInfo": {
                "name": "srs-mcp",
                "version": release_version(),
                "description": release_generation_description()
            },
            "instructions": INSTRUCTIONS
        })
    }
}

pub mod uri;

/// JSON-native portions of the generic SRS resource contract.
///
/// Dynamic resource enumeration and reads will move here with their repository
/// service calls. Templates are intentionally extracted first because they
/// have no runtime dependency and make a precise parity boundary.
pub mod srs_resources {
    use serde_json::{json, Value};
    use srs_repository::container_service::{list_containers, ContainerListFilter};
    use srs_repository::package_service::{list_types_filtered, TypeListFilter};
    use srs_repository::protocol_service::list_protocols;
    use srs_repository::store::RepositoryStore;
    use srs_repository::view_service::{list_compositions_summary, CompositionListFilter};

    use crate::uri;

    const MIME_JSON: &str = "application/json";
    const MIME_MARKDOWN: &str = "text/markdown";

    fn resource(
        uri: String,
        name: String,
        title: Option<String>,
        description: Option<String>,
        mime_type: &str,
    ) -> Value {
        let mut value = json!({ "uri": uri, "name": name, "mimeType": mime_type });
        let object = value.as_object_mut().expect("resource is an object");
        if let Some(title) = title {
            object.insert("title".into(), Value::String(title));
        }
        if let Some(description) = description {
            object.insert("description".into(), Value::String(description));
        }
        value
    }

    /// Enumerate the generic SRS resource catalogue without an MCP model dependency.
    pub fn list_resources(
        store: &dyn RepositoryStore,
        repository_id: &str,
    ) -> Result<Value, String> {
        let mut resources = vec![
            resource(uri::format(&uri::SrsUri::Map, repository_id), "map".into(), Some("Repository map".into()), Some("Counts, package info, relation summary and description for this repository — read this first to orient.".into()), MIME_JSON),
            resource(uri::format(&uri::SrsUri::Navigation, repository_id), "navigation".into(), Some("Repository navigation".into()), Some("The repository's identity record and ordered navigation sections (root container structure).".into()), MIME_JSON),
            resource(uri::format(&uri::SrsUri::Tree, repository_id), "tree".into(), Some("Repository tree".into()), Some("Recursive `contains` tree from every auto-detected root (records not targeted by a contains edge), with depth and cycle pruning — the same result as `srs tree`. Subtrees: srs://<repositoryId>/tree/{instanceId}.".into()), MIME_JSON),
            resource(uri::format(&uri::SrsUri::AgentIndex, repository_id), "agent-index".into(), Some("Agent index".into()), Some("AI orientation index: repository identity, counts, installed types, top-level sections and suggested entry points — same as `srs repo agent-index`.".into()), MIME_JSON),
        ];
        for c in
            list_containers(store, &ContainerListFilter::default()).map_err(|e| e.to_string())?
        {
            resources.push(resource(
                uri::format(&uri::SrsUri::Container(c.container_id), repository_id),
                c.title.clone(),
                Some(c.title),
                Some("Container: authored columns and ordered members (resolve-view).".into()),
                MIME_JSON,
            ));
        }
        for v in list_compositions_summary(store, &CompositionListFilter::default())
            .map_err(|e| e.to_string())?
        {
            resources.push(resource(
                uri::format(&uri::SrsUri::Composition(v.id), repository_id),
                format!("{}/{}", v.namespace, v.name),
                None,
                Some(v.description),
                MIME_MARKDOWN,
            ));
        }
        resources.push(resource(uri::format(&uri::SrsUri::ProtocolList, repository_id), "protocol".into(), Some("Installed protocols".into()), Some("Every installed Protocol definition: id, namespace/name@version, targetType, stageCount. Read one via srs://<repositoryId>/protocol/{protocolId}.".into()), MIME_JSON));
        for p in list_protocols(store).map_err(|e| e.to_string())? {
            resources.push(resource(
                uri::format(&uri::SrsUri::Protocol(p.protocol_id), repository_id),
                format!("{}/{}", p.protocol_namespace, p.protocol_name),
                None,
                Some("Protocol definition with its stages in dependsOn order.".into()),
                MIME_JSON,
            ));
        }
        for t in list_types_filtered(store, TypeListFilter::default()).map_err(|e| e.to_string())? {
            resources.push(resource(
                uri::format(&uri::SrsUri::Type(t.id), repository_id),
                format!("{}/{}", t.namespace, t.name),
                None,
                Some(t.description.unwrap_or_else(|| {
                    "Type schema: fieldAssignments + aiGuidance for authoring".into()
                })),
                MIME_JSON,
            ));
        }
        Ok(json!({ "resources": resources }))
    }

    pub fn list_resource_templates(repository_id: &str) -> Value {
        json!({ "resourceTemplates": [
            {
                "uriTemplate": uri::record_template(repository_id),
                "name": "record",
                "title": "Record by instance id",
                "description": "Read a single record (any tier) as typed JSON by its instanceId. Discover instanceIds via the find tool or container resources.",
                "mimeType": MIME_JSON
            },
            {
                "uriTemplate": uri::type_template(repository_id),
                "name": "type",
                "title": "Type authoring schema by type id",
                "description": "Authoring schema for a type: fieldIds, required flags, and aiGuidance — read before record_create on an unfamiliar type.",
                "mimeType": MIME_JSON
            },
            {
                "uriTemplate": uri::protocol_template(repository_id),
                "name": "protocol",
                "title": "Protocol definition by protocol id",
                "description": "A Protocol definition (same shape as `srs protocol get`) plus its stages sorted by order — the dependsOn walk an agent follows.",
                "mimeType": MIME_JSON
            },
            {
                "uriTemplate": uri::tree_template(repository_id),
                "name": "tree",
                "title": "Subtree by root instance id",
                "description": "Recursive `contains` tree rooted at one instance — descend from any navigation section or container member by its instanceId.",
                "mimeType": MIME_JSON
            }
        ] })
    }
}

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
