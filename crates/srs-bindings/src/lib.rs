use serde::Deserialize;
use srs_core::types::record::{FieldGroupValue, FieldValue};
use srs_core::types::relation::Relation;
use srs_repository::blueprint_schema_service::{self, BlueprintSchemaInput};
use srs_repository::container_service;
use srs_repository::record_store::{self, RecordListFilter, TransitionLifecycleInput};
use srs_repository::relation_service::{self, ListRelationsFilter};
use srs_repository::render_service::{self, RenderDocumentViewOptions};
use srs_repository::services::{self, ListNotesFilter};
use srs_repository::validation;
use srs_repository::view_service::{self, DocumentViewListFilter};
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
    pub fn load(srsj: &str) -> Result<SrsRepository, JsValue> {
        let store = JsonStore::from_srsj(srsj).map_err(js_err)?;
        Ok(SrsRepository { store })
    }

    /// Validate the repository. Returns a `RepositoryValidationReport` as a JS value.
    pub fn validate(&self) -> Result<JsValue, JsValue> {
        let report = validation::validate_repository(&self.store).map_err(js_err)?;
        to_js(&report)
    }

    /// List records. `filter_json` is a JSON string matching `RecordListFilter`
    /// (`{"typeNamespace":"...","typeName":"...","containerId":"..."}`); pass `"{}"` for all records.
    /// Returns a JS array of `Record` objects.
    pub fn list_records(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RecordListFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let records = record_store::list_records_filtered(&self.store, filter).map_err(js_err)?;
        to_js(&records)
    }

    /// Get a single record by instance ID. Returns the `Record` as a JS value, or `null` if not found.
    pub fn get_record(&self, id: &str) -> Result<JsValue, JsValue> {
        match record_store::get_record_by_id(&self.store, id).map_err(js_err)? {
            Some(record) => to_js(&record),
            None => Ok(JsValue::NULL),
        }
    }

    /// List notes. Returns a `ListNotesResult` as a JS value.
    pub fn list_notes(&self) -> Result<JsValue, JsValue> {
        let result =
            services::list_notes(&self.store, ListNotesFilter::default()).map_err(js_err)?;
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
            "records/tier-2",
        )
        .map_err(js_err)?;
        to_js(&record)
    }

    /// Update a record. `input_json` is a JSON object with fields:
    /// `fieldValues` (array), `groupValues` (optional), `tags` (optional).
    /// Returns the updated `Record` as a JS value.
    pub fn update_record(&self, instance_id: &str, input_json: &str) -> Result<JsValue, JsValue> {
        let input: UpdateRecordBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let record = record_store::update_record(
            &self.store,
            instance_id,
            input.field_values,
            input.group_values,
            input.tags,
        )
        .map_err(js_err)?;
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

    /// Transition a record's lifecycle state.
    /// `state` is the target state name (e.g. `"ratified"`).
    /// Returns the updated `Record` as a JS value.
    pub fn set_lifecycle_state(&self, instance_id: &str, state: &str) -> Result<JsValue, JsValue> {
        let input = TransitionLifecycleInput {
            to: Some(state.to_string()),
            by_transition: None,
        };
        let result = record_store::transition_record_lifecycle(&self.store, instance_id, input)
            .map_err(js_err)?;
        to_js(&result.record)
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
        to_js(&serde_json::json!({
            "schema": result.schema,
            "diagnostics": result.diagnostics,
        }))
    }

    /// Render a document view. `view_id` is the view's UUID; `format` is `"json"` or `"markdown"`;
    /// `container_id` optionally scopes TypeQuery sections to a container's membership.
    /// Returns `{ "rendered": <string>, "diagnostics": [...], "projection": <json|null> }`.
    /// When `format == "json"`, `projection` is a `DocumentViewProjection` object; otherwise `null`.
    pub fn render_document_view(
        &self,
        view_id: &str,
        format: &str,
        container_id: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let result = render_service::render_document_view(RenderDocumentViewOptions {
            store: &self.store,
            view_id,
            format: Some(format),
            theme_variant: None,
            container_id: container_id.as_deref(),
        })
        .map_err(js_err)?;
        to_js(&serde_json::json!({
            "rendered": result.rendered,
            "diagnostics": result.diagnostics,
            "projection": result.projection,
        }))
    }

    /// List document-view (L2) summaries. `filter_json` is a JSON string matching
    /// `{ "namespace"?: string, "containerType"?: string, "rootTypeId"?: string }`;
    /// pass `"{}"` for all document views. `rootTypeId` keeps only views whose
    /// `rootTypeRefs` include that Type UUID (RFC-009). Returns a JS array of objects
    /// `{ id, namespace, name, version, description, containerType?, rootTypeRefs?, sourcePackage? }`.
    pub fn list_document_views(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let parsed: DocumentViewListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = DocumentViewListFilter {
            namespace: parsed.namespace,
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
        let filter: ContainerListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let summaries = container_service::list_containers(
            &self.store,
            filter.container_type.as_deref(),
            filter.member_instance_id.as_deref(),
            filter.root_instance_id.as_deref(),
        )
        .map_err(js_err)?;
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
        let members = container_service::add_member(&self.store, container_id, instance_id)
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
        let members = container_service::remove_member(&self.store, container_id, instance_id)
            .map_err(js_err)?;
        to_js(&members)
    }
}

/// Input shape for `list_document_views` — parsed from caller-supplied JSON.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DocumentViewListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
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

/// Input shape for `update_record` — parsed from caller-supplied JSON.
///
/// `group_values` semantics mirror `record_store::update_record`:
/// - absent/null  → `None`  → preserve existing group values
/// - `[]`         → `Some(Some([]))` → clear group values
/// - `[{...}]`    → `Some(Some([...]))` → replace group values
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRecordBindingInput {
    field_values: Vec<FieldValue>,
    /// Double-wrapped so absence and null both become `None` (preserve existing),
    /// while an array (including empty) becomes `Some(Some(...))` (replace/clear).
    #[serde(default, deserialize_with = "deserialize_optional_optional")]
    group_values: Option<Option<Vec<FieldGroupValue>>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

/// Deserialise a field that may be absent, null, or a value into `Option<Option<T>>`:
/// - absent  → `None`
/// - `null`  → `None`  (same as absent; there is no JSON representation to distinguish)
/// - value   → `Some(Some(v))`
fn deserialize_optional_optional<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    let opt: Option<T> = Option::deserialize(deserializer)?;
    Ok(opt.map(Some))
}
