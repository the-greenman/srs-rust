use srs_core::types::relation::Relation;
use srs_repository::record_store::{self, RecordListFilter, TransitionLifecycleInput};
use srs_repository::relation_service::{self, ListRelationsFilter};
use srs_repository::services::{self, ListNotesFilter};
use srs_repository::validation;
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
}
