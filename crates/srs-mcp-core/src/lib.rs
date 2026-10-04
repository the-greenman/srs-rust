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
Clients that cannot read resources: call the read tool with any srs:// uri below (same result; output capped, with a pointer to bounded reads). Orient before writing: read srs://<repositoryId>/map for counts and package info, and \
srs://<repositoryId>/navigation for the document structure. srs://<repositoryId>/agent-index is \
the one-page AI orientation index (identity, counts, types, sections, entry points). \
srs://<repositoryId>/tree is the recursive contains-tree from every root, and \
srs://<repositoryId>/tree/{instanceId} the subtree under one instance (both take ?maxDepth=N, ?relationType=<key>, ?typeFilter=<namespace/name> to bound it; maxDepth=0 is roots only) — descend from a \
navigation section or container member by its instanceId; container members also carry \
sectionContainerId when they root a sub-container. Read individual records via the \
srs://<repositoryId>/record/{instanceId} resource template (or everything about one record, relations and \
arrangement subtree included, via srs://<repositoryId>/context/{containerId}/{instanceId}, optionally with ?excludeRelationCategories=composition,sequence to drop structural edges; for a hub record with many edges use the bounded `neighbours` tool instead), containers via \
srs://<repositoryId>/container/<containerId>, and rendered document views via \
srs://<repositoryId>/composition/<compositionId> (markdown; append ?containerId=<id> to render the composition for that container, required when its container-subset section names none, and, repeatable, ?excludeInstanceId=<id> to drop that member from the rendering, its arranged descendants moving up one level; containerId at most once). Type schemas live at \
srs://<repositoryId>/type/{typeId} (also via the type_schema tool): read one before \
authoring records of an unfamiliar type — its properties are keyed by Field.name (the \
same keys record_create fieldValues uses, RFC-039) and carry aiGuidance. Installed relation types (every one, used or not — the valid relationType keys) live at srs://<repositoryId>/relation-types. Protocols (staged \
processes) live at srs://<repositoryId>/protocol (list) and \
srs://<repositoryId>/protocol/{protocolId} (definition plus stages in order). Use the find tool for structured discovery \
(type, tag, lifecycle, tier, container, content match). Writes are validated: record_create, \
relation_create, and note_create enforce the repository's type and relation contracts and \
return diagnostics on rejection. Run repo_validate after a write batch and check its \
summary: summary.errors == 0 means the repository is consistent. Warnings are non-blocking, \
but review them; info diagnostics are informational. An empty diagnostics array means the repository is completely clean. \
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

pub mod guard;
pub mod tools;
pub mod uri;

/// JSON-native portions of the generic SRS resource contract.
///
/// Dynamic resource enumeration and reads will move here with their repository
/// service calls. Templates are intentionally extracted first because they
/// have no runtime dependency and make a precise parity boundary.
pub mod srs_resources {
    use crate::McpApplicationError;
    use serde_json::{json, Value};
    use srs_repository::agent_index_service::build_agent_index;
    use srs_repository::analysis::build_repo_map;
    use srs_repository::container_service::{list_containers, ContainerListFilter};
    use srs_repository::container_view_service::{
        resolve_container_view, ResolveContainerViewInput,
    };
    use srs_repository::context_query_service::{get_record_context, RecordContextQuery};
    use srs_repository::error::RepositoryError;
    use srs_repository::package_service::{
        list_relation_types_filtered, list_types_filtered, RelationTypeListFilter, TypeListFilter,
    };
    use srs_repository::protocol_service::{
        get_protocol_by_id, list_protocol_stages, list_protocols, GetProtocolResult,
    };
    use srs_repository::record_store::{get_instance_by_id, LoadedInstance};
    use srs_repository::render_service::{render_composition, RenderCompositionOptions};
    use srs_repository::repository_navigation_service::repository_navigation;
    use srs_repository::store::RepositoryStore;
    use srs_repository::tree_service::{build_tree, TreeOptions};
    use srs_repository::type_schema_service::{type_schema, TypeSchemaInput};
    use srs_repository::view_service::{list_compositions_summary, CompositionListFilter};

    use crate::uri;

    const MIME_JSON: &str = "application/json";
    const MIME_MARKDOWN: &str = "text/markdown";

    /// `srs tree` options for a tree URI (`root` = the `tree/{instanceId}` form).
    fn tree_options(root: Option<String>, q: uri::TreeQuery) -> TreeOptions {
        let d = TreeOptions::default();
        TreeOptions {
            root_ids: root.map(|id| vec![id]),
            relation_type: q.relation_type.unwrap_or(d.relation_type),
            max_depth: q.max_depth,
            type_filter: q.type_filter,
            ..d
        }
    }

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
            resource(uri::format(&uri::SrsUri::Tree(uri::TreeQuery::default()), repository_id), "tree".into(), Some("Repository tree".into()), Some("Recursive `contains` tree from every auto-detected root (records not targeted by a contains edge), with depth and cycle pruning — the same result as `srs tree`. Subtrees: srs://<repositoryId>/tree/{instanceId}. Append ?maxDepth=N (0 = roots only), ?relationType=<key> (default contains) and ?typeFilter=<namespace/name> to bound it, e.g. tree?maxDepth=1.".into()), MIME_JSON),
            resource(uri::format(&uri::SrsUri::AgentIndex, repository_id), "agent-index".into(), Some("Agent index".into()), Some("AI orientation index: repository identity, counts, installed types, top-level sections and suggested entry points — same as `srs repo agent-index`.".into()), MIME_JSON),
            resource(uri::format(&uri::SrsUri::RelationTypes, repository_id), "relation-types".into(), Some("Installed relation types".into()), Some("Every installed RelationTypeDefinition in the effective package set, used or not: key (`namespace/name` or canonical short name), label, category, description, direction and constraints. Use these keys as relationType in relation_create — same as `srs relation-type list`.".into()), MIME_JSON),
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
                uri::format(
                    &uri::SrsUri::Composition(v.id, Default::default()),
                    repository_id,
                ),
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

    fn service_err(e: RepositoryError) -> McpApplicationError {
        McpApplicationError::internal(e.to_string())
    }

    fn contents(uri: &str, mime_type: &str, text: String) -> Value {
        json!({ "contents": [{ "uri": uri, "mimeType": mime_type, "text": text }] })
    }

    fn json_contents<T: serde::Serialize>(
        value: &T,
        uri: &str,
    ) -> Result<Value, McpApplicationError> {
        let text = serde_json::to_string_pretty(value)
            .map_err(|e| McpApplicationError::internal(e.to_string()))?;
        Ok(contents(uri, MIME_JSON, text))
    }

    /// The `resources/read` result. Each arm is one repository-service call
    /// whose typed result is serialized verbatim (ADR-010/ADR-037).
    pub fn read_resource(
        store: &dyn RepositoryStore,
        repository_id: &str,
        raw_uri: &str,
    ) -> Result<Value, McpApplicationError> {
        let not_found =
            || McpApplicationError::resource_not_found(format!("resource not found: {raw_uri}"));
        let parsed = uri::parse(raw_uri, repository_id)
            .map_err(|e| McpApplicationError::invalid_params(e.to_string()))?;
        match parsed {
            uri::SrsUri::Map => {
                json_contents(&build_repo_map(store).map_err(service_err)?, raw_uri)
            }
            uri::SrsUri::Navigation => {
                json_contents(&repository_navigation(store).map_err(service_err)?, raw_uri)
            }
            uri::SrsUri::Tree(q) => json_contents(
                &build_tree(store, tree_options(None, q)).map_err(service_err)?,
                raw_uri,
            ),
            uri::SrsUri::TreeFrom(id, q) => json_contents(
                &build_tree(store, tree_options(Some(id), q)).map_err(service_err)?,
                raw_uri,
            ),
            uri::SrsUri::AgentIndex => {
                json_contents(&build_agent_index(store).map_err(service_err)?, raw_uri)
            }
            uri::SrsUri::RelationTypes => json_contents(
                &json!({ "relationTypes": list_relation_types_filtered(store, RelationTypeListFilter::default()).map_err(service_err)? }),
                raw_uri,
            ),
            // `srs context record` (+ global `--container`) as one read (#1134).
            uri::SrsUri::Context {
                container_id,
                instance_id,
                exclude_relation_categories,
            } => json_contents(
                &get_record_context(
                    store,
                    RecordContextQuery {
                        record_id: instance_id,
                        container_id,
                        exclude_relation_categories: exclude_relation_categories
                            .iter()
                            .map(|c| c.parse())
                            .collect::<Result<_, String>>()
                            .map_err(McpApplicationError::invalid_params)?,
                    },
                )
                .map_err(service_err)?,
                raw_uri,
            ),
            // `Ok(None)` is not a service error, so the not-found text is adapter-authored.
            // Any tier: a Tier-0 note is a legal record target (#1227).
            uri::SrsUri::Record(id) => match get_instance_by_id(store, &id).map_err(service_err)? {
                None => Err(not_found()),
                Some(LoadedInstance::Record(record)) => json_contents(&record, raw_uri),
                Some(LoadedInstance::Note(note)) => json_contents(&note, raw_uri),
            },
            uri::SrsUri::Container(id) => json_contents(
                &resolve_container_view(
                    store,
                    ResolveContainerViewInput {
                        container_id: id,
                        view_id: None,
                    },
                )
                .map_err(service_err)?,
                raw_uri,
            ),
            uri::SrsUri::Composition(id, q) => {
                let result = render_composition(RenderCompositionOptions {
                    store,
                    view_id: &id,
                    format: Some("markdown"),
                    theme_variant: None,
                    container_id: q.container_id.as_deref(),
                    instance_id_filter: None,
                    exclude_instance_ids: &q.exclude_instance_ids,
                })
                .map_err(service_err)?;
                Ok(contents(raw_uri, MIME_MARKDOWN, result.rendered))
            }
            uri::SrsUri::Type(id) => json_contents(
                &type_schema(
                    store,
                    TypeSchemaInput {
                        type_id: id,
                        type_version: None,
                    },
                )
                .map_err(service_err)?,
                raw_uri,
            ),
            uri::SrsUri::ProtocolList => json_contents(
                &json!({ "protocols": list_protocols(store).map_err(service_err)? }),
                raw_uri,
            ),
            // `srs protocol get` + `srs protocol stages` in one read.
            uri::SrsUri::Protocol(id) => {
                match get_protocol_by_id(store, &id).map_err(service_err)? {
                    GetProtocolResult::NotFound => Err(not_found()),
                    GetProtocolResult::Found(protocol) => {
                        let stages = list_protocol_stages(store, &id).map_err(service_err)?;
                        json_contents(&json!({ "protocol": protocol, "stages": stages }), raw_uri)
                    }
                }
            }
        }
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
                "uriTemplate": uri::context_template(repository_id),
                "name": "context",
                "title": "Record context in a container",
                "description": "Everything about one record in one read: field values, every relation in both directions with the other endpoint inline (comments, notes, sources), and its arrangement subtree in the container. Drop the containerId segment (srs://<repositoryId>/context/{instanceId}) for the record and its relations only. Append ?excludeRelationCategories=composition,sequence to drop edges by relation-type category (e.g. structural contains/precedes).",
                "mimeType": MIME_JSON
            },
            {
                "uriTemplate": uri::tree_template(repository_id),
                "name": "tree",
                "title": "Subtree by root instance id",
                "description": "Recursive `contains` tree rooted at one instance — descend from any navigation section or container member by its instanceId. Optional ?maxDepth (0 = roots only), ?relationType, ?typeFilter bound it.",
                "mimeType": MIME_JSON
            }
        ] })
    }
}

/// The MCP prompts surface — one prompt per package blueprint. Prompt `name`
/// is the blueprint UUID so `prompts/get` passes it straight to the service.
pub mod srs_prompts {
    use serde_json::{json, Map, Value};
    use srs_repository::blueprint_brief_service::{
        blueprint_brief, render_brief_markdown, BlueprintBriefInput,
    };
    use srs_repository::blueprint_service::list_blueprints_summary;
    use srs_repository::error::RepositoryError;
    use srs_repository::store::RepositoryStore;

    use crate::McpApplicationError;

    fn service_err(e: RepositoryError) -> McpApplicationError {
        McpApplicationError::internal(e.to_string())
    }

    /// The `prompts/list` result. Non-fatal blueprint diagnostics are not
    /// surfaced: `prompts/list` has no warnings channel.
    pub fn list_prompts(store: &dyn RepositoryStore) -> Result<Value, McpApplicationError> {
        let result = list_blueprints_summary(store).map_err(service_err)?;
        let prompts: Vec<Value> = result
            .summaries
            .into_iter()
            .map(|s| {
                json!({
                    "name": s.id,
                    "description": format!(
                        "{}/{} v{}: {}", s.namespace, s.name, s.version, s.description
                    )
                })
            })
            .collect();
        Ok(json!({ "prompts": prompts }))
    }

    /// The `prompts/get` result. Role is `user`: a blueprint brief is guidance
    /// the agent consumes as user-context, not pre-authored assistant output.
    pub fn get_prompt(
        store: &dyn RepositoryStore,
        name: &str,
        arguments: Option<&Map<String, Value>>,
    ) -> Result<Value, McpApplicationError> {
        if arguments.is_some_and(|a| !a.is_empty()) {
            return Err(McpApplicationError::invalid_params(format!(
                "prompt '{name}' takes no arguments"
            )));
        }
        let result = blueprint_brief(
            store,
            BlueprintBriefInput {
                blueprint_id: name.to_string(),
            },
        )
        .map_err(|e| match e {
            RepositoryError::BlueprintNotFound { .. } => {
                McpApplicationError::invalid_params(format!("prompt not found: {name}"))
            }
            other => service_err(other),
        })?;
        Ok(json!({
            "messages": [{
                "role": "user",
                "content": { "type": "text", "text": render_brief_markdown(&result) }
            }]
        }))
    }
}

/// Length cap (chars) on the client-supplied handle used as `Actor.name`.
pub const CLIENT_HANDLE_MAX_CHARS: usize = 120;

/// What one `tools/call` changed (ADR-049): the tool and the entities it wrote,
/// as recorded by the store. Session telemetry for the host; never part of the
/// MCP response and never persisted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WriteSummary {
    pub tool: String,
    pub changed: Vec<srs_repository::ChangeEntry>,
}

/// The complete SRS MCP application over one repository store: resources,
/// prompts and tools, as JSON-native MCP results. Owns no transport, runtime or
/// filesystem; the caller decides the store's lifetime and persistence.
pub struct SrsMcpApplication<S> {
    store: S,
    repository_id: String,
    write_guard: Option<guard::WriteGuard>,
    last_summary: Option<WriteSummary>,
    /// The client handle this application last stamped (so a repeat `initialize` can replace it).
    applied_handle: Option<String>,
}

impl<S: srs_repository::store::RepositoryStore> SrsMcpApplication<S> {
    pub fn new(store: S, repository_id: impl Into<String>) -> Self {
        store.set_change_recording(true);
        Self {
            store,
            repository_id: repository_id.into(),
            write_guard: None,
            last_summary: None,
            applied_handle: None,
        }
    }

    /// The last request's write summary, once (ADR-049): `None` if it wrote
    /// nothing (reads, guard rejections, failures before any write).
    pub fn take_write_summary(&mut self) -> Option<WriteSummary> {
        self.last_summary.take()
    }

    /// Replace the session's write guard (srs-rust#1165); `None` clears it.
    pub fn set_write_guard(&mut self, guard: Option<guard::WriteGuard>) {
        self.write_guard = guard;
    }

    /// Set (or clear) the RFC-046 session actor stamped on everything this
    /// session creates. Host-supplied only — never read from tool arguments.
    pub fn set_session_actor(&self, actor: Option<Value>) {
        self.store.set_session_actor(actor);
    }

    /// Client handle (srs-rust#1177): fill the session actor's `name` from
    /// `params.clientInfo.name` (control characters stripped, trimmed, capped at [`CLIENT_HANDLE_MAX_CHARS`]; empty or
    /// missing leaves `name` absent). Display-only (RFC-046 `Actor.name` is a hint). The
    /// host owns `kind`/`id` and any `name` it set (a non-empty string `name` wins; null or "" does not count as fixed); the client
    /// can never change them. A repeat `initialize` refreshes a handle this method set; a host-fixed `name` is never replaced.
    fn apply_client_handle(&mut self, params: &Value) {
        let Some(Value::Object(mut actor)) = self.store.session_actor() else {
            return;
        };
        // Host-fixed = a non-empty string `name`; null/"" count as absent.
        if actor
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| !n.is_empty() && Some(n) != self.applied_handle.as_deref())
        {
            return;
        }
        let handle: String = params["clientInfo"]["name"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .chars()
            .take(CLIENT_HANDLE_MAX_CHARS)
            .collect();
        let handle = handle.trim_end();
        if handle.is_empty() {
            if self.applied_handle.take().is_some() {
                actor.remove("name");
                self.store.set_session_actor(Some(Value::Object(actor)));
            }
            return;
        }
        self.applied_handle = Some(handle.to_string());
        actor.insert("name".into(), json!(handle));
        self.store.set_session_actor(Some(Value::Object(actor)));
    }

    /// Read `repositoryId` from the store's manifest (`"unknown"` when absent).
    pub fn open(store: S) -> Result<Self, McpApplicationError> {
        let manifest = store
            .load_manifest()
            .map_err(|e| McpApplicationError::internal(e.to_string()))?;
        let repository_id = manifest
            .extra
            .get("repositoryId")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        Ok(Self::new(store, repository_id))
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn store(&self) -> &S {
        &self.store
    }
}

fn arguments(
    fields: &mut serde_json::Map<String, Value>,
) -> Result<Option<serde_json::Map<String, Value>>, McpApplicationError> {
    match fields.remove("arguments") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(arguments)) => Ok(Some(arguments)),
        Some(_) => Err(McpApplicationError::invalid_params(
            "arguments must be an object",
        )),
    }
}

impl<S: srs_repository::store::RepositoryStore> McpApplication for SrsMcpApplication<S> {
    fn initialize(&mut self, params: &Value) -> Result<Value, McpApplicationError> {
        self.apply_client_handle(params);
        Ok(srs_metadata::initialize_result())
    }

    fn call(&mut self, method: &str, params: Option<&Value>) -> Result<Value, McpApplicationError> {
        let mut fields = match params {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(object)) => object.clone(),
            Some(_) => {
                return Err(McpApplicationError::invalid_params(
                    "params must be an object",
                ))
            }
        };
        self.last_summary = None;
        let store: &dyn srs_repository::store::RepositoryStore = &self.store;
        match method {
            "resources/list" => srs_resources::list_resources(store, &self.repository_id)
                .map_err(McpApplicationError::internal),
            "resources/templates/list" => {
                Ok(srs_resources::list_resource_templates(&self.repository_id))
            }
            "resources/read" => {
                let uri = fields
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| McpApplicationError::invalid_params("uri must be a string"))?;
                srs_resources::read_resource(store, &self.repository_id, uri)
            }
            "tools/list" => Ok(tools::list_tools()),
            "tools/call" => {
                let name = fields
                    .remove("name")
                    .and_then(|v| v.as_str().map(ToString::to_string))
                    .ok_or_else(|| McpApplicationError::invalid_params("name must be a string"))?;
                let arguments = arguments(&mut fields)?;
                if name == tools::TOOL_READ {
                    return tools::read_tool(store, &self.repository_id, arguments);
                }
                if let Some(guard) = &self.write_guard {
                    if let Err(message) = guard.check(store, &name, arguments.as_ref()) {
                        return Ok(tools::tool_err(message));
                    }
                }
                // Writes made through other handles since the last call (the UI)
                // are not this request's; assumes synchronous dispatch (ADR-049).
                store.drain_changes();
                let result = tools::call_tool(store, &name, arguments);
                let changed = store.drain_changes();
                if !changed.is_empty() {
                    self.last_summary = Some(WriteSummary {
                        tool: name,
                        changed,
                    });
                }
                result
            }
            "prompts/list" => srs_prompts::list_prompts(store),
            "prompts/get" => {
                let name = fields
                    .remove("name")
                    .and_then(|v| v.as_str().map(ToString::to_string))
                    .ok_or_else(|| McpApplicationError::invalid_params("name must be a string"))?;
                srs_prompts::get_prompt(store, &name, arguments(&mut fields)?.as_ref())
            }
            _ => Err(McpApplicationError::method_not_found(method)),
        }
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

    /// MCP `-32002` resource-not-found.
    pub fn resource_not_found(message: impl Into<String>) -> Self {
        Self {
            code: -32002,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {method}"),
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

    /// Dispatch one JSON-RPC message given as text — the shape a browser or
    /// HTTP body arrives in. Unparseable text is a `-32700` parse error.
    pub fn dispatch_str(&mut self, message: &str) -> Option<String> {
        let response = match serde_json::from_str::<Value>(message) {
            Ok(message) => self.dispatch(message),
            Err(_) => Some(error(Value::Null, -32700, "Parse error", None)),
        };
        response.map(|response| response.to_string())
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
            // A repeat initialize re-runs initialization (the relay shares one session across
            // clients); an unsupported requested version is answered with ours (MCP lifecycle).
            let result = self.application.initialize(params.unwrap_or(&Value::Null));
            if result.is_ok() {
                self.initialized = true;
            }
            result
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
    fn dispatch_str_reports_parse_errors_and_swallows_notifications() {
        let mut dispatcher = McpDispatcher::new(EchoApplication);
        let parsed: Value =
            serde_json::from_str(&dispatcher.dispatch_str("{not json").unwrap()).unwrap();
        assert_eq!(parsed["error"]["code"], -32700);
        assert_eq!(parsed["id"], Value::Null);
        dispatcher
            .dispatch_str(
                &json!({"jsonrpc":"2.0","id":1,"method":"initialize",
                        "params":{"protocolVersion": MCP_PROTOCOL_VERSION}})
                .to_string(),
            )
            .unwrap();
        assert_eq!(
            dispatcher.dispatch_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            None
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
    fn unsupported_version_gets_ours_and_repeat_initialize_succeeds() {
        let mut dispatcher = McpDispatcher::new(EchoApplication);
        for params in [
            json!({ "protocolVersion": "2024-11-05" }),
            json!({}),
            json!({ "protocolVersion": MCP_PROTOCOL_VERSION }),
        ] {
            let r = dispatcher
                .dispatch(request(json!(1), "initialize", params))
                .unwrap();
            assert_eq!(r["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
            assert!(dispatcher.is_initialized());
        }
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
