use srs_repository::record_store::{self, RecordListFilter};
use srs_repository::services::{self, ListNotesFilter};
use srs_repository::validation;
use srs_repository::JsonStore;
use wasm_bindgen::prelude::*;

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
        serde_wasm_bindgen::to_value(&report).map_err(js_err)
    }

    /// List records. `filter_json` is a JSON string matching `RecordListFilter`
    /// (`{"typeNamespace":"...","typeName":"...","containerId":"..."}`); pass `"{}"` for all records.
    /// Returns a JS array of `Record` objects.
    pub fn list_records(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RecordListFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let records = record_store::list_records_filtered(&self.store, filter).map_err(js_err)?;
        serde_wasm_bindgen::to_value(&records).map_err(js_err)
    }

    /// Get a single record by instance ID. Returns the `Record` as a JS value, or `null` if not found.
    pub fn get_record(&self, id: &str) -> Result<JsValue, JsValue> {
        match record_store::get_record_by_id(&self.store, id).map_err(js_err)? {
            Some(record) => serde_wasm_bindgen::to_value(&record).map_err(js_err),
            None => Ok(JsValue::NULL),
        }
    }

    /// List notes. Returns a `ListNotesResult` as a JS value.
    pub fn list_notes(&self) -> Result<JsValue, JsValue> {
        let result =
            services::list_notes(&self.store, ListNotesFilter::default()).map_err(js_err)?;
        serde_wasm_bindgen::to_value(&result).map_err(js_err)
    }
}
