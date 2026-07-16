use serde::Deserialize;
use srs_core::types::record::{FieldGroupValue, FieldValue};
use srs_core::types::relation::Relation;
use srs_repository::blueprint_schema_service::{self, BlueprintSchemaInput};
use srs_repository::blueprint_service;
use srs_repository::container_service::{self, ContainerListFilter};
use srs_repository::container_view_service::{self, ResolveContainerViewInput};
use srs_repository::discovery_service::{self, DiscoveryQuery};
use srs_repository::federation_service::{
    append_federation_event, filter_federation_events, list_federation_events,
    parse_federation_registry_json, resolve_repository, AppendFederationEventInput,
    ListFederationEventsFilter, ListFederationEventsInput, ResolveRepositoryInput,
};
use srs_repository::governance_scaffold_service::{self, CreateGovernanceRepositoryInput};
use srs_repository::manifest_service;
use srs_repository::migrate_identity_service;
use srs_repository::package_service::{
    self, FieldListFilter, GetFieldResult, GetTypeResult, RelationTypeListFilter, TypeListFilter,
};
use srs_repository::protocol_service::{self, GetProtocolResult};
use srs_repository::record_store::{
    self, CreateRecordInput, RecordListFilter, TransitionLifecycleInput,
};
use srs_repository::registry_service::{
    filter_registry_entries, parse_registry_json, RegistryListFilter,
};
use srs_repository::relation_service::{self, ListRelationsFilter, OrderByPrecedesInput};
use srs_repository::render_service::{self, RenderDocumentViewOptions};
use srs_repository::repository_lifecycle::{self, InitNewRepositoryInput};
use srs_repository::repository_navigation_service;
use srs_repository::services::{
    self, graduate_note as graduate_note_service, GraduateNoteInput, ListNotesFilter,
};
use srs_repository::tag_service;
use srs_repository::type_schema_service::{self, TypeSchemaInput};
use srs_repository::validation;
use srs_repository::view_service::{self, DocumentViewListFilter, GetViewResult};
use srs_repository::JsonStore;
use wasm_bindgen::prelude::*;

/// Serialise `value` to a JSON string via serde_json (which respects all serde attributes
/// including `rename_all` and `flatten`), then parse it as a JS value via the browser's
/// native JSON.parse. This is more reliable than serde_wasm_bindgen::to_value for structs
/// that use #[serde(flatten)] or complex serde transformations.
fn to_js<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let json = serde_json::to_string(value).map_err(|e| js_err(e.to_string()))?;
    js_sys::JSON::parse(&json).map_err(|e| js_err(format!("{e:?}")))
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub struct SrsRepository {
    store: JsonStore,
}

#[wasm_bindgen]
impl SrsRepository {
    /// Load a repository from a `.srsj` JSON string.
    ///
    /// Applies RFC-014 migration before loading (idempotent on already-migrated bundles),
    /// so callers do not need to migrate the seed separately.
    pub fn load(srsj: &str) -> Result<SrsRepository, JsValue> {
        let store = srs_repository::srsj_migration_service::load_from_srsj(srsj).map_err(js_err)?;
        Ok(SrsRepository { store })
    }

    /// Validate the repository. Returns a `RepositoryValidationReport` as a JS value.
    pub fn validate(&self) -> Result<JsValue, JsValue> {
        let report = validation::validate_repository(&self.store).map_err(js_err)?;
        to_js(&report)
    }

    /// Return a conformance report comparing the manifest's `declaredExtensions` against the
    /// implementation's supported set and detected content usage.
    /// Returns a `DeclaredExtensionsReport` as a JS value with four camelCase keys:
    /// `declared`, `supported`, `declaredButUnsupported`, `usedButUndeclared`.
    pub fn declared_extensions_conformance(&self) -> Result<JsValue, JsValue> {
        let report =
            manifest_service::declared_extensions_conformance(&self.store).map_err(js_err)?;
        to_js(&report)
    }

    /// List records. `filter_json` is a JSON string matching `RecordListFilter`
    /// (`{"typeNamespace":"...","typeName":"...","containerId":"..."}`); pass `"{}"` for all records.
    /// Returns a JS array of `RecordSummary` objects — each `{ instanceId, displayLabel, record }`,
    /// where `displayLabel` is the core-resolved label (same resolution `srs tree` uses) and
    /// `record` is the full `Record`. Clients render `displayLabel` directly and must not
    /// re-derive titles from `fieldValues`.
    pub fn list_records(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RecordListFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let summaries = record_store::list_record_summaries(&self.store, filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Run a discovery query against the repository.
    /// `query_json` is a JSON object matching `DiscoveryQuery` (camelCase fields;
    /// all optional — omit or pass `"{}"` for "return all").
    /// Returns a `DiscoveryResult` as a JS value.
    pub fn find(&self, query_json: &str) -> Result<JsValue, JsValue> {
        let query: DiscoveryQuery =
            serde_json::from_str(query_json).map_err(|e| js_err(format!("invalid query: {e}")))?;
        let result = discovery_service::find(&self.store, query).map_err(js_err)?;
        to_js(&result)
    }

    /// Get a single record by instance ID. Returns a `RecordSummary` (`{ instanceId, displayLabel, record }`)
    /// as a JS value, or `null` if not found.
    pub fn get_record(&self, id: &str) -> Result<JsValue, JsValue> {
        match record_store::get_record_summary_by_id(&self.store, id).map_err(js_err)? {
            Some(summary) => to_js(&summary),
            None => Ok(JsValue::NULL),
        }
    }

    /// List notes. Returns a `ListNotesResult` as a JS value.
    pub fn list_notes(&self) -> Result<JsValue, JsValue> {
        let result =
            services::list_notes(&self.store, ListNotesFilter::default()).map_err(js_err)?;
        to_js(&result)
    }

    /// Graduate a Note to a typed Record in one atomic step.
    ///
    /// `input_json` is a `CreateRecordInput` JSON object
    /// (`fieldValues`, `groupValues?`, `tags?`). Returns `{ note, record }` where
    /// `note` has `graduatedAt` stamped. `container_id` is optional; when supplied,
    /// the new Record is added to that container atomically.
    #[wasm_bindgen]
    pub fn graduate_note(
        &self,
        note_id: &str,
        type_ref: &str,
        type_version: Option<u32>,
        container_id: Option<String>,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let record_input: CreateRecordInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = graduate_note_service(
            &self.store,
            GraduateNoteInput {
                note_id: note_id.to_string(),
                type_ref: type_ref.to_string(),
                type_version,
                record_input,
                container_id,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Serialise the current repository state to a `.srsj` JSON string.
    /// The browser caller can use this to offer a download of the edited repo.
    #[wasm_bindgen]
    pub fn export_srsj(&self) -> Result<String, JsValue> {
        self.store.to_srsj_string().map_err(js_err)
    }

    /// Create a record. `input_json` is a JSON object with fields:
    /// `fieldValues` (array of `{fieldId, value}`), `groupValues` (optional array),
    /// and `tags` (optional array of strings).
    /// Returns the created `Record` as a JS value.
    pub fn create_record(
        &self,
        type_id: &str,
        type_version: u32,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: CreateRecordBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let record = record_store::create_record(
            &self.store,
            type_id,
            type_version,
            input.field_values,
            input.group_values,
            input.tags,
        )
        .map_err(js_err)?;
        to_js(&record)
    }

    /// Create a Tier-2 record and add it to a container in one call.
    ///
    /// `container_id` is the UUID of the container to add the record to.
    /// `type_id` is the UUID of the type; `type_version` is the version number.
    /// `input_json` is a JSON object with `fieldValues` (required), `groupValues` (optional),
    /// and `tags` (optional) — the same shape as `create_record`.
    ///
    /// Returns the created `Record` as a JS value.
    /// Returns a JS error if the container does not exist, the type is not found,
    /// or field validation fails.
    pub fn create_record_in_container(
        &self,
        container_id: &str,
        type_id: &str,
        type_version: u32,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: CreateRecordBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = record_store::create_record_in_container(
            &self.store,
            record_store::CreateRecordInContainerInput {
                container_id: container_id.to_string(),
                type_id: type_id.to_string(),
                type_version,
                field_values: input.field_values,
                group_values: input.group_values,
                tags: input.tags,
            },
        )
        .map_err(js_err)?;
        to_js(&result.record)
    }

    /// Update a record. `input_json` is a JSON object with fields:
    /// `fieldValues` (array), `groupValues` (optional), `tags` (optional),
    /// `typeVersion` (optional u32 — omit to keep the stored version).
    /// Returns the updated `Record` as a JS value.
    pub fn update_record(&self, instance_id: &str, input_json: &str) -> Result<JsValue, JsValue> {
        let input: record_store::UpdateRecordInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let record =
            record_store::update_record(&self.store, instance_id, input).map_err(js_err)?;
        to_js(&record)
    }

    /// Delete a record by instance ID. Returns nothing on success.
    pub fn delete_record(&self, instance_id: &str) -> Result<(), JsValue> {
        record_store::delete_record(&self.store, instance_id).map_err(js_err)?;
        Ok(())
    }

    /// List relations. `filter_json` is a JSON object with optional camelCase fields:
    /// `{ "source": "uuid", "target": "uuid", "relationType": "...", "containerId": "uuid" }`
    /// Pass `"{}"` for all relations.
    /// Returns a JS array of `RelationSummary` objects.
    pub fn list_relations(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct FilterInput {
            source: Option<String>,
            target: Option<String>,
            relation_type: Option<String>,
            container_id: Option<String>,
        }
        let input: FilterInput = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = ListRelationsFilter {
            source: input.source,
            target: input.target,
            relation_type: input.relation_type,
            container_id: input.container_id,
        };
        let summaries = relation_service::list_relations(&self.store, filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Create a relation. `input_json` is a JSON object whose fields match the `Relation` struct
    /// (camelCase: `relationType`, `sourceInstanceId`, `targetInstanceId`; `relationId` is
    /// auto-generated if absent or empty).
    /// Returns the created `Relation` as a JS value.
    pub fn create_relation(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let relation: Relation = serde_json::from_str(input_json)
            .map_err(|e| js_err(format!("invalid relation input: {e}")))?;
        let result =
            relation_service::create_relation_auto(&self.store, relation).map_err(js_err)?;
        to_js(&result.relation)
    }

    /// Delete a relation by its `relation_id`. Returns `undefined` on success.
    pub fn delete_relation(&self, relation_id: &str) -> Result<(), JsValue> {
        relation_service::delete_relation(&self.store, relation_id).map_err(js_err)?;
        Ok(())
    }

    /// Order a set of instance IDs by following the `precedes` relation chain.
    ///
    /// `input_json` is `{ "instanceIds": ["uuid1", "uuid2", ...] }`.
    /// Returns `{ "orderedIds": [...] }` — same IDs, reordered by the `precedes`
    /// chain. Falls back to `created_at` ascending, then `instanceId` ascending,
    /// for records not connected by a `precedes` edge. Handles cycles.
    pub fn order_by_precedes(&self, input_json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            instance_ids: Vec<String>,
        }
        let parsed: Input = serde_json::from_str(input_json)
            .map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = relation_service::order_by_precedes(
            &self.store,
            OrderByPrecedesInput {
                instance_ids: parsed.instance_ids,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Transition a record's lifecycle state.
    /// `state` is the target state name (e.g. `"ratified"`).
    /// Returns `{ "record": <Record>, "warnings": ["LIFECYCLE_FINAL_STATE: ..."] }` as a JS value.
    /// `warnings` is empty for non-final transitions; contains a `LIFECYCLE_FINAL_STATE` entry
    /// when the target state has `isFinal: true`.
    /// Note: a bare flip into a state declaring `requiresRelation` (RFC-022) is rejected unless
    /// the obligation is already satisfied — use `transition_record` with a `fulfillment` input.
    pub fn set_lifecycle_state(&self, instance_id: &str, state: &str) -> Result<JsValue, JsValue> {
        let input = TransitionLifecycleInput {
            to: Some(state.to_string()),
            by_transition: None,
            fulfillment: None,
        };
        let result = record_store::transition_record_lifecycle(&self.store, instance_id, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Transition a record's lifecycle state with the full RFC-022 input surface.
    /// `input_json` matches the CLI `record transition` stdin contract:
    /// `{ "to"?: string, "byTransition"?: string, "fulfillment"?: {
    ///    "newRecord"?: { "fieldValues": [...], "typeVersion"?: N },
    ///    "existingInstanceId"?: "<uuid>", "relationType"?: "supersedes" } }`.
    /// Returns `{ "record", "warnings", "successor"?, "relation"? }` as a JS value —
    /// `successor`/`relation` are present when the transition was fulfilled.
    pub fn transition_record(
        &self,
        instance_id: &str,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: TransitionLifecycleInput = serde_json::from_str(input_json)
            .map_err(|e| js_err(format!("invalid transition input: {e}")))?;
        let result = record_store::transition_record_lifecycle(&self.store, instance_id, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Query the allowed lifecycle transitions for a record (ext:lifecycle).
    /// Returns `{ "currentState": string, "transitions": [{ "name": string, "to": string,
    /// "toIsFinal": bool, "requiresRelation"?: { "relationType": string|string[],
    /// "direction"?: "incoming"|"outgoing" } }], "isImmutable": bool }` as a JS value.
    /// `requiresRelation` (RFC-022) is the target state's relation obligation — clients route
    /// successor-flow UX from it instead of string-matching state names.
    pub fn get_allowed_lifecycle_transitions(&self, instance_id: &str) -> Result<JsValue, JsValue> {
        let result = record_store::get_allowed_lifecycle_transitions(&self.store, instance_id)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Create a successor record that supersedes or refines an existing record.
    /// `predecessor_id` is the instance ID of the record being superseded/refined.
    /// `input_json` is a JSON object:
    ///   `{ "relationType": "supersedes"|"refines", "fieldValues": [...], "lifecycleState"?: "...", "typeVersion"?: N }`.
    /// Returns `{ "record": <Record>, "relation": <Relation> }` as a JS value.
    /// The relation runs from the successor (source) to the predecessor (target).
    pub fn create_record_successor(
        &self,
        predecessor_id: &str,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: record_store::CreateRecordSuccessorInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = record_store::create_record_successor(&self.store, predecessor_id, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Project a blueprint into a nested draft-07 JSON Schema describing the whole
    /// multi-record document it declares. `blueprint_id` is the blueprint's UUID.
    /// Returns `{ "schema": <json-schema>, "diagnostics": [<string>, ...] }` as a JS value;
    /// non-fatal projection problems surface in `diagnostics`.
    pub fn blueprint_schema(&self, blueprint_id: &str) -> Result<JsValue, JsValue> {
        let result = blueprint_schema_service::blueprint_schema(
            &self.store,
            BlueprintSchemaInput {
                blueprint_id: blueprint_id.to_string(),
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Render a document view. `view_id` is the view's UUID; `format` is `"json"` or `"markdown"`;
    /// `container_id` optionally scopes TypeQuery sections to a container's membership;
    /// `instance_id_filter` optionally scopes ContainerSubset sections to a single record,
    /// producing a per-record export document.
    /// Returns `{ "rendered": <string>, "diagnostics": [...], "projection": <json|null> }`.
    /// When `format == "json"`, `projection` is a `DocumentViewProjection` object; otherwise `null`.
    pub fn render_document_view(
        &self,
        view_id: &str,
        format: &str,
        container_id: Option<String>,
        instance_id_filter: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let result = render_service::render_document_view(RenderDocumentViewOptions {
            store: &self.store,
            view_id,
            format: Some(format),
            theme_variant: None,
            container_id: container_id.as_deref(),
            instance_id_filter: instance_id_filter.as_deref(),
        })
        .map_err(js_err)?;
        to_js(&result)
    }

    /// List document-view (L2) summaries. `filter_json` is a JSON string matching
    /// `{ "namespace"?: string, "name"?: string, "containerType"?: string, "rootTypeId"?: string }`;
    /// pass `"{}"` for all document views. `name` filters to an exact view name. `rootTypeId` keeps only views whose
    /// `rootTypeRefs` include that Type UUID (RFC-009). Returns a JS array of objects
    /// `{ id, namespace, name, version, description, containerType?, rootTypeRefs?, sourcePackage? }`.
    pub fn list_document_views(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let parsed: DocumentViewListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = DocumentViewListFilter {
            namespace: parsed.namespace,
            name: parsed.name,
            container_type: parsed.container_type,
            root_type_id: parsed.root_type_id,
        };
        let summaries =
            view_service::list_document_views_summary(&self.store, &filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// List container summaries. `filter_json` is a JSON string matching
    /// `{ "containerType"?: string, "memberInstanceId"?: string, "rootInstanceId"?: string }`;
    /// pass `"{}"` for all containers. Returns a JS array of `ContainerSummary` objects.
    pub fn list_containers(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let parsed: ContainerListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = ContainerListFilter {
            container_type: parsed.container_type,
            member_instance_id: parsed.member_instance_id,
            root_instance_id: parsed.root_instance_id,
        };
        let summaries = container_service::list_containers(&self.store, &filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Get a single container by ID, including its `rootInstanceIds` and `memberInstanceIds`.
    /// Returns the `Container` as a JS value.
    pub fn get_container(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let container =
            container_service::get_container(&self.store, container_id).map_err(js_err)?;
        to_js(&container)
    }

    /// Add an instance to a container's `memberInstanceIds` (idempotent).
    /// Returns the updated member-id list as a JS array of strings.
    pub fn add_container_member(
        &self,
        container_id: &str,
        instance_id: &str,
    ) -> Result<JsValue, JsValue> {
        let members =
            container_service::add_container_member(&self.store, container_id, instance_id)
                .map_err(js_err)?;
        to_js(&members)
    }

    /// Remove an instance from a container's `memberInstanceIds`.
    /// Returns the updated member-id list as a JS array of strings.
    pub fn remove_container_member(
        &self,
        container_id: &str,
        instance_id: &str,
    ) -> Result<JsValue, JsValue> {
        let members =
            container_service::remove_container_member(&self.store, container_id, instance_id)
                .map_err(js_err)?;
        to_js(&members)
    }

    /// List the containers an instance belongs to — every container whose `memberInstanceIds`
    /// includes `instance_id`. Returns a JS array of `ContainerSummary` objects (same shape as
    /// `list_containers`). Equivalent to `list_containers('{"memberInstanceId": instance_id}')`,
    /// exposed by name for the web client (issue #181).
    pub fn containers_for_instance(&self, instance_id: &str) -> Result<JsValue, JsValue> {
        let summaries =
            container_service::containers_for_instance(&self.store, instance_id).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Project a Type into a draft-07 JSON Schema describing a single record's `fieldValues`,
    /// keyed by field `name`. `type_id` is the Type's UUID; `type_version` selects a version —
    /// pass `undefined` (omit the argument) to resolve the latest version.
    /// Returns `{ "schema": <json-schema>, "diagnostics": [<string>, ...] }` as a JS value;
    /// non-fatal projection problems (a dangling `fieldId`, a select field with no
    /// `allowedValues`) surface in `diagnostics`. An unresolvable Type is an error.
    pub fn type_schema(
        &self,
        type_id: &str,
        type_version: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let result = type_schema_service::type_schema(
            &self.store,
            TypeSchemaInput {
                type_id: type_id.to_string(),
                type_version,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// List blueprint summaries across all package boundaries.
    /// Returns a JS value matching `BlueprintListResult`; WARN-level
    /// provenance issues (missing files, duplicate IDs) surface in `diagnostics`.
    pub fn list_blueprints(&self) -> Result<JsValue, JsValue> {
        let result = blueprint_service::list_blueprints_summary(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// List protocol summaries from the compiled package model.
    /// Returns a JS array of `{ protocolId, protocolNamespace, protocolName, protocolVersion,
    /// stageCount, sourcePackage? }` objects.
    pub fn list_protocols(&self) -> Result<JsValue, JsValue> {
        let summaries = protocol_service::list_protocols(&self.store).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Get a protocol's stored definition JSON by its `protocolId`.
    /// Returns the full protocol definition as a JS value, or `null` if not found.
    pub fn get_protocol_by_id(&self, id: &str) -> Result<JsValue, JsValue> {
        match protocol_service::get_protocol_by_id(&self.store, id).map_err(js_err)? {
            GetProtocolResult::Found(val) => to_js(&val),
            GetProtocolResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// Find the first protocol whose `protocolTargetType` matches `target_type_id`.
    /// Returns `{ protocolId, protocolName, stages, diagnostics }` as a JS value,
    /// or `null` if no protocol targets that type.
    pub fn find_protocol_by_target_type(&self, target_type_id: &str) -> Result<JsValue, JsValue> {
        match protocol_service::find_protocol_by_target_type(&self.store, target_type_id)
            .map_err(js_err)?
        {
            Some(result) => to_js(&result),
            None => Ok(JsValue::NULL),
        }
    }

    // ── Definition browse (fields, types, L1 views, packages) ────────────────────

    /// List field definitions from the compiled package, optionally filtered by namespace or
    /// package boundary path. `filter_json` is `{}` or `{"namespace":"...","package":"..."}`.
    /// Returns a JS array of `FieldSummary` objects.
    pub fn list_fields(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: FieldListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let fields = package_service::list_fields_filtered(
            &self.store,
            FieldListFilter {
                namespace: filter.namespace,
                package: filter.package.map(Some),
            },
        )
        .map_err(js_err)?;
        to_js(&fields)
    }

    /// Get a field definition by its id. Returns the full `Field` object, or `null` if not found.
    pub fn get_field(&self, id: &str) -> Result<JsValue, JsValue> {
        match package_service::get_field_by_id(&self.store, id).map_err(js_err)? {
            GetFieldResult::Found(field) => to_js(&*field), // GetFieldResult wraps Box<Field>
            GetFieldResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List type definitions from the compiled package, optionally filtered by namespace or
    /// package boundary path. `filter_json` is `{}` or `{"namespace":"...","package":"..."}`.
    /// Returns a JS array of `TypeSummary` objects.
    pub fn list_types(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: TypeListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let types = package_service::list_types_filtered(
            &self.store,
            TypeListFilter {
                namespace: filter.namespace,
                package: filter.package.map(Some),
            },
        )
        .map_err(js_err)?;
        to_js(&types)
    }

    /// List relation type definitions from the compiled package.
    /// `filter_json` is `{}` or `{"status":"active"}` to filter by status.
    /// Returns a JS array of `RelationTypeDefinition` objects.
    pub fn list_relation_types(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RelationTypeListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let relation_types = package_service::list_relation_types_filtered(
            &self.store,
            RelationTypeListFilter {
                status: filter.status,
            },
        )
        .map_err(js_err)?;
        to_js(&relation_types)
    }

    /// Get a type definition by its id (latest version). Returns the full `RecordType` object,
    /// or `null` if not found.
    pub fn get_type(&self, id: &str) -> Result<JsValue, JsValue> {
        match package_service::get_type_by_id_latest(&self.store, id).map_err(js_err)? {
            GetTypeResult::Found(record_type) => to_js(&record_type),
            GetTypeResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List L1 view definitions from the compiled package. Returns a JS array of `ViewSummary`
    /// objects (`{id, namespace, name, version, description, compatibleTypes?, sourcePackage?}`).
    pub fn list_views(&self) -> Result<JsValue, JsValue> {
        let views = view_service::list_views_summary(&self.store).map_err(js_err)?;
        to_js(&views)
    }

    /// Get an L1 view definition by its id. Returns the full `View` object, or `null` if not found.
    pub fn get_view(&self, id: &str) -> Result<JsValue, JsValue> {
        match view_service::get_view_by_id(&self.store, id).map_err(js_err)? {
            GetViewResult::Found(view) => to_js(&*view),
            GetViewResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List all package boundaries (primary + sub-packages) from the repository manifest.
    /// Returns a JS array of objects with shape `{id, namespace, name, version,
    /// boundaryPath: string | null, fieldCount, typeCount}`.
    /// `boundaryPath` is `null` for the primary package and the boundary path string for sub-packages.
    pub fn list_packages(&self) -> Result<JsValue, JsValue> {
        let packages = package_service::list_packages(&self.store).map_err(js_err)?;
        to_js(&packages)
    }

    /// List the document views (L2) bound to a container's root type. Resolves the container's
    /// first root instance's `typeId`/`typeVersion`, then returns every `DocumentView` whose
    /// `rootTypeRefs` includes that exact type binding (RFC-009). Returns an empty array — not an
    /// error — when the container has no root instance, the root carries no type binding (Tier 0/1),
    /// or no view matches. Returns a JS array of **full** `DocumentView` objects (including
    /// `sections`) — not the lighter summaries that `list_document_views` returns — because the
    /// caller needs the section definitions to render the view.
    pub fn document_views_for_container(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let views = view_service::document_views_for_container(&self.store, container_id)
            .map_err(js_err)?;
        to_js(&views)
    }

    /// Resolve a structured container view for an editor member list (issue #254):
    /// the container root record, the ordered Tier-2 member records (full `Record` +
    /// core-resolved display label), the DocumentView-driven column spec, and
    /// diagnostics. `view_id` optionally overrides the DocumentView; when omitted it is
    /// matched from the container's root type binding. Returns a `ContainerView` object.
    pub fn resolve_container_view(
        &self,
        container_id: &str,
        view_id: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let result = container_view_service::resolve_container_view(
            &self.store,
            ResolveContainerViewInput {
                container_id: container_id.to_string(),
                view_id,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Return the repository's root identity and precedes-ordered section navigation.
    /// Equivalent to `srs repo navigation`.
    ///
    /// Returns `{ rootContainerId, identity, sections, diagnostics }`.
    /// - `identity`: `NavigationNode` for the root identity record (`instanceId`, `typeId`,
    ///   `typeVersion`, `typeNamespace`, `typeName`, `displayLabel`).
    /// - `sections`: precedes-ordered `NavigationNode` array; each carries an optional
    ///   `sectionContainerId` pointing to the section's own container.
    /// - `diagnostics`: non-empty when `manifest.container` is absent (pre-RFC-013 repo) or a
    ///   member id fails to resolve; in that case `rootContainerId` is `""`, `identity` has
    ///   all-empty-string/zero fields (it is a present object, not `null`), and `sections` is `[]`.
    pub fn repository_navigation(&self) -> Result<JsValue, JsValue> {
        let result =
            repository_navigation_service::repository_navigation(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// List all vocabulary Terms (RFC-006) defined in the package.
    /// Returns a JS array of `Term` objects — the same terms returned by `srs term list`.
    /// srs-web uses this to populate the tag picker / tag cloud.
    pub fn list_terms(&self) -> Result<JsValue, JsValue> {
        let terms = tag_service::list_terms(&self.store).map_err(js_err)?;
        to_js(&terms)
    }

    /// Scaffold a governance repository from a seeded `.srsj` store.
    ///
    /// `input_json` is a JSON string matching `CreateGovernanceRepositoryInput`
    /// (`{"title":"...","purpose":"...","repositoryId":"..."}`).
    /// `namespace` is optional — when omitted or `null`, the service derives
    /// `"com.example.<slug>"` from the title (e.g. `"My Org"` → `"com.example.my-org"`).
    /// Pass an explicit `namespace` (`{"namespace":"com.example.myorg","title":"..."}`)
    /// when a different organisational prefix is required.
    ///
    /// Stamps manifest identity (repositoryId, namespace, title) and creates the
    /// governance/article identity record, Decision Log container + root record, and
    /// root container — all in one call. After this returns, call `export_srsj()` to get
    /// the final bundle for download.
    pub fn scaffold_new_repository(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: CreateGovernanceRepositoryInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = governance_scaffold_service::create_governance_repository(&self.store, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Re-stamp a seed repository's identity. `input_json` is a JSON string matching
    /// `InitNewRepositoryInput` (`{"namespace":"...","title":"...",...}`).
    /// Updates `repositoryId`, `namespace`, `title`, `description`, and
    /// `installedAt` on the upstream package reference — at top-level
    /// `upstreamPackage` for RFC-014-migrated seeds, or `meta.upstreamPackage`
    /// for legacy seeds. Returns an
    /// `InitNewRepositoryResult` as a JS value.
    pub fn init_new_repository(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: InitNewRepositoryInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            repository_lifecycle::init_new_repository(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Migrate the repository's identity instance to a `com.semanticops.core/purpose` record.
    ///
    /// Converts a Tier-0 note identity (or a container with no identity pointer) to a typed
    /// purpose record. Returns a `MigrateIdentityResult` as a JS value. Errors if the identity
    /// is already a purpose record or the container has no title/description to derive a
    /// statement from. After this returns, call `export_srsj()` to get the updated bundle.
    pub fn migrate_identity(&self) -> Result<JsValue, JsValue> {
        let result = migrate_identity_service::migrate_identity(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// Return the value of a named field on a record, by its exact package-defined name
    /// (the `name` field in the field definition JSON, e.g. `"title"` or `"decision-summary"`).
    /// No case normalization is performed — the caller must pass the exact name.
    ///
    /// Returns the field value as a JS value, or `null` if the field is absent from
    /// the record, the field name is not part of the type schema, or the record is
    /// not found. Never errors on a missing/unknown field — callers treat `null` as
    /// a graceful no-op.
    pub fn get_field_value_by_name(
        &self,
        instance_id: &str,
        field_name: &str,
    ) -> Result<JsValue, JsValue> {
        let result = record_store::get_field_value_by_name(
            &self.store,
            record_store::GetFieldValueByNameInput {
                instance_id: instance_id.to_string(),
                field_name: field_name.to_string(),
            },
        )
        .map_err(js_err)?;
        match result.value {
            Some(v) => to_js(&v),
            None => Ok(JsValue::NULL),
        }
    }

    /// Resolve a repository by ID via DFS through the local federation registry.
    ///
    /// `input_json` is `{"repositoryId": "<id>"}`. Returns a `ResolveRepositoryResult`
    /// with `found`, `registryId`, and `entry` (null when not found).
    /// Errors when the federation registry file is absent.
    pub fn federation_resolve(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: ResolveRepositoryInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = resolve_repository(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// List federation events from the repository's configured events file.
    ///
    /// `filter_json` is a JSON object with optional `sourceRepositoryId`, `targetRepositoryId`,
    /// and `kind` keys; pass `"{}"` to return all events. Returns a `ListFederationEventsResult`
    /// with `repositoryId`, `events`, `totalCount`, and `filteredCount`.
    /// Returns an empty result (not an error) when the events file does not yet exist.
    pub fn federation_events_list(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: ListFederationEventsFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let result = list_federation_events(&self.store, ListFederationEventsInput { filter })
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Append a federation event to the repository's events file.
    ///
    /// `input_json` is `{"repositoryId": "<id>", "event": <FederationEvent>}`.
    /// Returns `{"eventId": "<id>", "totalEvents": N}`.
    pub fn federation_events_append(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: AppendFederationEventInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = append_federation_event(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }
}

// ── Repo-independent free functions (ADR-013 addendum) ───────────────────────

/// Parse a registry catalog JSON string into a `Registry` object.
///
/// `catalog_json` is the raw text of a `.json` registry catalog file
/// (the `ext:registry` schema). Returns the parsed `Registry` as a JS value,
/// or a JS error string on parse failure.
///
/// This is a repo-independent free function (ADR-013 addendum): it does not
/// require a loaded SRS repository and operates on a caller-supplied payload.
#[wasm_bindgen]
pub fn parse_registry(catalog_json: &str) -> Result<JsValue, JsValue> {
    let registry = parse_registry_json(catalog_json).map_err(js_err)?;
    to_js(&registry)
}

/// Parse a registry catalog JSON string and apply an optional filter.
///
/// `catalog_json` is the raw registry catalog text. `filter_json` is a JSON
/// object with optional `publisher` (string) and `tag` (string) keys; pass
/// `"{}"` to return all entries.
///
/// Returns a `Registry` JS value whose `entries` array contains only the
/// matching entries (all entries if no filter criteria are set).
///
/// This is a repo-independent free function (ADR-013 addendum).
#[wasm_bindgen]
pub fn list_registry_entries(catalog_json: &str, filter_json: &str) -> Result<JsValue, JsValue> {
    let registry = parse_registry_json(catalog_json).map_err(js_err)?;
    let filter: RegistryListFilter =
        serde_json::from_str(filter_json).map_err(|e| js_err(format!("invalid filter: {e}")))?;
    let filtered = filter_registry_entries(registry, &filter);
    to_js(&filtered)
}

/// Parse a federation registry JSON string into a `RepositoryRegistry` object.
///
/// `registry_json` is the raw text of a `federation/registry.json` file.
/// Returns the parsed `RepositoryRegistry` as a JS value, or a JS error on parse failure.
///
/// This is a repo-independent free function (ADR-013 addendum).
#[wasm_bindgen]
pub fn parse_federation_registry(registry_json: &str) -> Result<JsValue, JsValue> {
    let registry = parse_federation_registry_json(registry_json).map_err(js_err)?;
    to_js(&registry)
}

/// Parse a federation registry JSON string and apply an optional filter.
///
/// `registry_json` is the raw `federation/registry.json` text. `filter_json` is a JSON
/// object with optional `sourceRepositoryId`, `targetRepositoryId`, and `kind` keys;
/// pass `"{}"` to return all events.
///
/// Returns a `FederationEventsFile` JS value whose `events` array contains only the
/// matching events (all events if no filter criteria are set).
///
/// This is a repo-independent free function (ADR-013 addendum).
#[wasm_bindgen]
pub fn filter_federation_events_json(
    events_file_json: &str,
    filter_json: &str,
) -> Result<JsValue, JsValue> {
    use srs_core::extensions::federation::FederationEventsFile;
    let events_file: FederationEventsFile = serde_json::from_str(events_file_json)
        .map_err(|e| js_err(format!("invalid events file: {e}")))?;
    let filter: ListFederationEventsFilter =
        serde_json::from_str(filter_json).map_err(|e| js_err(format!("invalid filter: {e}")))?;
    let filtered = filter_federation_events(events_file, &filter);
    to_js(&filtered)
}

/// Input shape for `list_document_views` — parsed from caller-supplied JSON.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DocumentViewListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    container_type: Option<String>,
    #[serde(default)]
    root_type_id: Option<String>,
}

/// Input shape for `list_containers` — parsed from caller-supplied JSON.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ContainerListBindingFilter {
    #[serde(default)]
    container_type: Option<String>,
    #[serde(default)]
    member_instance_id: Option<String>,
    #[serde(default)]
    root_instance_id: Option<String>,
}

/// Input shape for `list_fields` — parsed from caller-supplied JSON.
/// `package: null` means no package filter (returns all); a specific sub-package path
/// narrows to that boundary. The primary-package-only filter (`Some(None)` in the service)
/// is not expressible through this binding — omit `package` to get all fields.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FieldListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

/// Input shape for `list_types` — parsed from caller-supplied JSON.
/// Same package-filter semantics as `FieldListBindingFilter`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TypeListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

/// Input shape for `list_relation_types` — parsed from caller-supplied JSON.
/// `status`: None returns all relation type definitions; Some(s) returns only those
/// whose serialized status string equals s (e.g. "active").
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RelationTypeListBindingFilter {
    #[serde(default)]
    status: Option<String>,
}

/// Input shape for `create_record` — parsed from caller-supplied JSON.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRecordBindingInput {
    field_values: Vec<FieldValue>,
    #[serde(default)]
    group_values: Option<Vec<FieldGroupValue>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use srs_repository::record_store::CreateRecordInput;
    use srs_repository::services::{graduate_note as graduate_note_service, GraduateNoteInput};
    use srs_repository::JsonStore;

    /// Minimal `.srsj` with one note (tier-0) and one optional-field type `com.test/bind-type`.
    fn srsj_with_note_and_type() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "test-repo-bindings-graduate",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [{
                    "instanceId": "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                    "tier": 0,
                    "path": "records/notes/binding-test-note.json"
                }],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-bindings-001",
                    "namespace": "com.test",
                    "name": "bind-package",
                    "version": "1.0.0",
                    "fields": ["fields/body.json"],
                    "types": ["types/bind-type.json"],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                },
                "package/fields/body.json": {
                    "id": "field-bind-00001",
                    "namespace": "com.test",
                    "name": "body",
                    "version": 1,
                    "valueType": "string",
                    "description": "Body",
                    "aiGuidance": null,
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/bind-type.json": {
                    "id": "type-bind-00001",
                    "namespace": "com.test",
                    "name": "bind-type",
                    "version": 1,
                    "description": "Binding test type",
                    "fields": [{
                        "fieldId": "field-bind-00001",
                        "order": 0,
                        "required": false,
                        "repeatable": false
                    }],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "records/notes/binding-test-note.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
                    "instanceId": "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                    "sections": [{"name": "body", "content": "test content"}]
                }
            }
        })
        .to_string()
    }

    #[test]
    fn graduate_note_service_result_serialises() {
        let store = JsonStore::from_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let result = graduate_note_service(
            &store,
            GraduateNoteInput {
                note_id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string(),
                type_ref: "com.test/bind-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .expect("graduate_note should succeed");

        let json = serde_json::to_value(&result).expect("result must serialize");
        assert!(
            json["note"]["graduatedAt"].is_string(),
            "note.graduatedAt must be present as a string"
        );
        assert!(
            json["record"].is_object(),
            "record must be present as an object"
        );
        assert!(
            json["record"]["instanceId"].is_string(),
            "record.instanceId must be present"
        );
    }

    /// Minimal `.srsj` with one type `com.test/container-item-type` (one required string field)
    /// and one container. Used by `create_record_in_container_result_serialises`.
    fn srsj_with_container_and_type() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "test-repo-bindings-container",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [],
                "containerIndex": [{
                    "containerId": "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                    "title": "Test Container"
                }],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-container-001",
                    "namespace": "com.test",
                    "name": "container-package",
                    "version": "1.0.0",
                    "fields": ["fields/title.json"],
                    "types": ["types/container-item-type.json"],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                },
                "package/fields/title.json": {
                    "id": "field-title-00001",
                    "namespace": "com.test",
                    "name": "title",
                    "version": 1,
                    "valueType": "string",
                    "description": "Title",
                    "aiGuidance": null,
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/container-item-type.json": {
                    "id": "type-container-001",
                    "namespace": "com.test",
                    "name": "container-item-type",
                    "version": 1,
                    "description": "Binding test type for container membership",
                    "fields": [{
                        "fieldId": "field-title-00001",
                        "order": 0,
                        "required": true,
                        "repeatable": false
                    }],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "containers/cccccccc-cccc-4ccc-8ccc-cccccccccccc.json": {
                    "containerId": "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                    "title": "Test Container",
                    "memberInstanceIds": []
                }
            }
        })
        .to_string()
    }

    #[test]
    fn create_record_in_container_result_serialises() {
        use srs_repository::container_service;
        use srs_repository::record_store::{self, CreateRecordInContainerInput};

        let srsj = srsj_with_container_and_type();
        let store = srs_repository::JsonStore::from_srsj(&srsj).expect("load srsj");

        let result = record_store::create_record_in_container(
            &store,
            CreateRecordInContainerInput {
                container_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".to_string(),
                type_id: "type-container-001".to_string(),
                type_version: 1,
                field_values: vec![srs_core::types::record::FieldValue {
                    field_id: "field-title-00001".to_string(),
                    value: serde_json::json!("My Decision"),
                    entries: None,
                    source: None,
                    edited_at: None,
                }],
                group_values: None,
                tags: None,
            },
        )
        .expect("create_record_in_container should succeed");

        let json = serde_json::to_value(&result.record).expect("record must serialize");
        let instance_id = json["instanceId"]
            .as_str()
            .expect("instanceId must be string");
        assert!(!instance_id.is_empty(), "instanceId must be non-empty");

        let container =
            container_service::get_container(&store, "cccccccc-cccc-4ccc-8ccc-cccccccccccc")
                .expect("container loaded");
        let members = container.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&instance_id.to_string()),
            "instanceId must appear in container memberInstanceIds"
        );
    }

    #[test]
    fn blueprint_schema_result_serialises() {
        use srs_repository::blueprint_schema_service::BlueprintSchemaResult;
        let result = BlueprintSchemaResult {
            schema: serde_json::Value::Null,
            diagnostics: vec![],
        };
        let json = serde_json::to_value(&result).expect("BlueprintSchemaResult must serialize");
        assert!(json["schema"].is_null(), "schema field must be present");
        assert!(
            json["diagnostics"].is_array(),
            "diagnostics field must be present as array"
        );
    }

    #[test]
    fn get_record_summary_by_id_smoke() {
        use srs_repository::record_store::{get_record_summary_by_id, CreateRecordInput};
        let store = JsonStore::from_srsj(&srsj_with_note_and_type()).expect("load srsj");
        // Graduate the note to get a typed record we can look up
        let note_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let graduated = srs_repository::services::graduate_note(
            &store,
            srs_repository::services::GraduateNoteInput {
                note_id: note_id.to_string(),
                type_ref: "com.test/bind-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: vec![],
                    group_values: None,
                    tags: None,
                },
                container_id: None,
            },
        )
        .expect("graduate should succeed");
        let record_id = &graduated.record.instance_id;
        let summary = get_record_summary_by_id(&store, record_id)
            .expect("should not error")
            .expect("should find record");
        assert_eq!(summary.instance_id, *record_id);
        // No title/name/label fields → falls back to type_name
        assert!(
            !summary.display_label.is_empty(),
            "display_label must be non-empty"
        );
        assert_eq!(summary.record.instance_id, *record_id);
    }

    #[test]
    fn declared_extensions_conformance_report_serialises() {
        let store = JsonStore::from_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let report = srs_repository::manifest_service::declared_extensions_conformance(&store)
            .expect("conformance report should succeed");
        let json = serde_json::to_value(&report).expect("report must serialize");
        assert!(json["declared"].is_array(), "declared must be a JSON array");
        assert!(
            json["supported"].is_array(),
            "supported must be a JSON array"
        );
        assert!(
            json["declaredButUnsupported"].is_array(),
            "declaredButUnsupported must be a JSON array"
        );
        assert!(
            json["usedButUndeclared"].is_array(),
            "usedButUndeclared must be a JSON array"
        );
        // A minimal repo with no declaredExtensions has an empty declared list
        assert!(
            json["declared"]
                .as_array()
                .expect("declared must be an array")
                .is_empty(),
            "no extensions declared in a minimal srsj repo"
        );
        // The supported list must include the known extensions
        let supported: Vec<String> = json["supported"]
            .as_array()
            .expect("supported must be an array")
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("supported entry must be a string")
                    .to_string()
            })
            .collect();
        assert!(
            supported.contains(&"ext:lifecycle".to_string()),
            "supported must include ext:lifecycle"
        );
    }

    // ── get_field_value_by_name service integration tests ────────────────────
    //
    // These tests call `record_store::get_field_value_by_name` directly because
    // `JsValue` is not available in a native target. Their purpose is to confirm
    // the service function is reachable from the srs-bindings crate imports.
    // The binding's `to_js`/`JsValue::NULL` branches are validated exclusively
    // by the `cargo build --target wasm32-unknown-unknown -p srs-bindings` gate.

    /// Minimal `.srsj` with one Tier-2 record whose `"title"` field is set to `"My Title"`.
    fn srsj_with_titled_record() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "test-repo-get-field-value",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [{
                    "instanceId": "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
                    "tier": 2,
                    "path": "records/titled-record.json"
                }],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-gfvbn-001",
                    "namespace": "com.test",
                    "name": "gfvbn-package",
                    "version": "1.0.0",
                    "fields": ["fields/title.json"],
                    "types": ["types/titled-type.json"],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                },
                "package/fields/title.json": {
                    "id": "field-gfvbn-title",
                    "namespace": "com.test",
                    "name": "title",
                    "version": 1,
                    "valueType": "string",
                    "description": "Title",
                    "aiGuidance": null,
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/titled-type.json": {
                    "id": "type-gfvbn-001",
                    "namespace": "com.test",
                    "name": "titled-type",
                    "version": 1,
                    "description": "Type with a title field",
                    "fields": [{
                        "fieldId": "field-gfvbn-title",
                        "order": 0,
                        "required": true,
                        "repeatable": false
                    }],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "records/titled-record.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
                    "typeId": "type-gfvbn-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "titled-type",
                    "fieldValues": [{
                        "fieldId": "field-gfvbn-title",
                        "value": "My Title"
                    }]
                }
            }
        })
        .to_string()
    }

    #[test]
    fn get_field_value_by_name_returns_value_for_known_field() {
        use srs_repository::record_store;
        let store = JsonStore::from_srsj(&srsj_with_titled_record()).expect("load srsj");

        let result = record_store::get_field_value_by_name(
            &store,
            record_store::GetFieldValueByNameInput {
                instance_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
                field_name: "title".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert_eq!(
            result.value,
            Some(serde_json::json!("My Title")),
            "value must match the stored field value"
        );
    }

    #[test]
    fn get_field_value_by_name_returns_none_for_unknown_field() {
        use srs_repository::record_store;
        let store = JsonStore::from_srsj(&srsj_with_titled_record()).expect("load srsj");

        let result = record_store::get_field_value_by_name(
            &store,
            record_store::GetFieldValueByNameInput {
                instance_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
                field_name: "nonexistent".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert!(
            result.value.is_none(),
            "unknown field name must return None, not an error"
        );
    }

    // ── federation binding smoke tests ───────────────────────────────────────
    //
    // These tests call the service functions directly (not through JsValue) because
    // `JsValue` is not available on a native target. They confirm the service functions
    // are reachable from srs-bindings imports and that results serialize correctly.

    fn srsj_with_federation_events() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "test-repo-federation",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-fed-001",
                    "namespace": "com.test",
                    "name": "fed-package",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                },
                "federation/events.json": {
                    "repositoryId": "test-repo-federation",
                    "events": [{
                        "eventId": "evt-0001",
                        "event": "merge",
                        "sourceRepositoryId": "repo-source-01",
                        "targetRepositoryId": "test-repo-federation",
                        "affectedInstanceIds": ["iiii-1111"],
                        "at": "2026-01-01T00:00:00Z"
                    }]
                }
            }
        })
        .to_string()
    }

    #[test]
    fn federation_events_list_service_result_serialises() {
        use srs_repository::federation_service::{
            list_federation_events, ListFederationEventsFilter, ListFederationEventsInput,
        };
        let store = JsonStore::from_srsj(&srsj_with_federation_events()).expect("load srsj");
        let result = list_federation_events(
            &store,
            ListFederationEventsInput {
                filter: ListFederationEventsFilter::default(),
            },
        )
        .expect("list_federation_events should succeed");

        assert_eq!(result.total_count, 1, "should find one event");
        assert_eq!(result.filtered_count, 1, "filter is empty so count stays 1");
        assert_eq!(result.events[0].event_id, "evt-0001");

        let json = serde_json::to_value(&result).expect("result must serialize");
        assert!(
            json["repositoryId"].is_string(),
            "repositoryId must be present"
        );
        assert!(json["events"].is_array(), "events must be an array");
        assert_eq!(json["totalCount"].as_u64(), Some(1));
        assert_eq!(json["filteredCount"].as_u64(), Some(1));
    }

    #[test]
    fn federation_events_list_empty_when_no_file() {
        use srs_repository::federation_service::{
            list_federation_events, ListFederationEventsFilter, ListFederationEventsInput,
        };
        let store = JsonStore::from_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let result = list_federation_events(
            &store,
            ListFederationEventsInput {
                filter: ListFederationEventsFilter::default(),
            },
        )
        .expect("list_federation_events should not error when events file is absent");

        assert_eq!(result.total_count, 0, "empty repo should have 0 events");
        let json = serde_json::to_value(&result).expect("result must serialize");
        assert_eq!(json["totalCount"].as_u64(), Some(0));
    }

    #[test]
    fn federation_events_append_service_result_serialises() {
        use srs_core::extensions::federation::FederationEvent;
        use srs_repository::federation_service::{
            append_federation_event, AppendFederationEventInput,
        };
        let store = JsonStore::from_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let result = append_federation_event(
            &store,
            AppendFederationEventInput {
                repository_id: "test-repo-bindings-graduate".to_string(),
                event: FederationEvent {
                    event_id: "evt-bind-0001".to_string(),
                    event: srs_core::extensions::federation::FederationEventKind::Import,
                    at: "2026-01-01T00:00:00Z".to_string(),
                    performed_by: None,
                    source_repository_id: Some("repo-source-01".to_string()),
                    target_repository_id: Some("test-repo-bindings-graduate".to_string()),
                    affected_instance_ids: vec!["iiii-0001".to_string()],
                    strategy: None,
                    note: None,
                },
            },
        )
        .expect("append_federation_event should succeed");

        assert_eq!(result.event_id, "evt-bind-0001");
        assert_eq!(result.total_events, 1);

        let json = serde_json::to_value(&result).expect("result must serialize");
        assert_eq!(json["eventId"].as_str(), Some("evt-bind-0001"));
        assert_eq!(json["totalEvents"].as_u64(), Some(1));
    }
}
