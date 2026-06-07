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

    /// Serialise the current repository state to a `.srsj` JSON string.
    /// The browser caller can use this to offer a download of the edited repo.
    #[wasm_bindgen]
    pub fn export_srsj(&self) -> Result<String, JsValue> {
        self.store.to_srsj_string().map_err(js_err)
    }
}
