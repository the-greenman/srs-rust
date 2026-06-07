use serde::Deserialize;
use srs_core::types::record::{FieldGroupValue, FieldValue};
use srs_repository::record_store::{self, RecordListFilter};
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
            "records",
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
