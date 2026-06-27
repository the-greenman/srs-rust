//! # Extension Service
//!
//! Public API for extension definition operations. This module is the sole entry point
//! for all extension logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: CreateExtensionInput = serde_json::from_reader(io::stdin())?;
//! let result = extension_service::create_extension(store, input)?;
//! output::ok("extension create", result)
//! ```

use srs_core::types::record::{FieldValue, Record};

use crate::error::RepositoryError;
use crate::record_store::{
    create_record_at_dir, delete_record, get_record_by_id, list_records_by_type, update_record,
};
use crate::store::RepositoryStore;

pub(crate) const EXTENSION_RECORD_DIR: &str = "package/records";

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionSummary {
    pub instance_id: String,
    pub namespace: Option<serde_json::Value>,
    pub name: Option<serde_json::Value>,
    pub version: Option<serde_json::Value>,
    pub extension_type: String,
}

pub struct CreateExtensionInput {
    pub raw: serde_json::Value,
}

pub struct ExtensionResult {
    pub record: Record,
}

pub fn list_extensions(
    store: &dyn RepositoryStore,
) -> Result<Vec<ExtensionSummary>, RepositoryError> {
    let mut records = list_records_by_type(store, "meta", "extension")?;
    if records.is_empty() {
        records = list_records_by_type_fallback(store, "meta", "extension")?;
    }
    let summaries = records
        .into_iter()
        .map(|r| ExtensionSummary {
            namespace: r.extra.get("namespace").cloned(),
            name: r.extra.get("name").cloned(),
            version: r.extra.get("version").cloned(),
            extension_type: format!("{}/{}", r.type_namespace, r.type_name),
            instance_id: r.instance_id,
        })
        .collect();
    Ok(summaries)
}

pub fn create_extension(
    store: &dyn RepositoryStore,
    input: CreateExtensionInput,
) -> Result<ExtensionResult, RepositoryError> {
    let json_value = input.raw;

    let field_values_json = json_value
        .get("fieldValues")
        .or_else(|| json_value.get("field_values"))
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Missing fieldValues in extension JSON".to_string(),
        })?
        .as_object()
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "fieldValues must be an object".to_string(),
        })?;

    let field_values: Vec<FieldValue> = field_values_json
        .iter()
        .map(|(k, v)| FieldValue {
            field_id: k.clone(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        })
        .collect();

    let type_id = json_value
        .get("typeId")
        .or_else(|| json_value.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("meta.extension");

    let type_version = json_value
        .get("typeVersion")
        .or_else(|| json_value.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    let record = create_record_at_dir(
        store,
        type_id,
        type_version,
        field_values,
        None,
        None,
        EXTENSION_RECORD_DIR,
    )?;
    Ok(ExtensionResult { record })
}

pub fn update_extension(
    store: &dyn RepositoryStore,
    id: &str,
    input: CreateExtensionInput,
) -> Result<ExtensionResult, RepositoryError> {
    let field_values_json: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(input.raw).map_err(|e| RepositoryError::Serialize {
            path: std::path::PathBuf::from("extension"),
            source: e,
        })?;

    let field_values: Vec<FieldValue> = field_values_json
        .into_iter()
        .map(|(k, v)| FieldValue {
            field_id: k,
            value: v,
            entries: None,
            source: None,
            edited_at: None,
        })
        .collect();

    let record = update_record(store, id, field_values, None, None)?;
    Ok(ExtensionResult { record })
}

pub fn delete_extension(store: &dyn RepositoryStore, id: &str) -> Result<String, RepositoryError> {
    delete_record(store, id)
}

pub fn get_extension_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Record>, RepositoryError> {
    match get_record_by_id(store, id)? {
        Some(record) => {
            if is_extension_type(&record) {
                Ok(Some(record))
            } else {
                Ok(None)
            }
        }
        None => {
            if let Some(record) = get_record_by_id_fallback(store, id)? {
                if is_extension_type(&record) {
                    Ok(Some(record))
                } else {
                    Ok(None)
                }
            } else {
                Ok(None)
            }
        }
    }
}

fn is_extension_type(record: &Record) -> bool {
    record.type_namespace == "meta" && record.type_name == "extension"
}

fn list_records_by_type_fallback(
    store: &dyn RepositoryStore,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let paths = match store.list_instance_files(EXTENSION_RECORD_DIR) {
        Ok(p) => p,
        Err(RepositoryError::Io { .. } | RepositoryError::NotFound { .. }) => return Ok(vec![]),
        Err(e) => return Err(e),
    };

    let mut records = vec![];
    for path in &paths {
        if !path.ends_with(".json") {
            continue;
        }
        let value = match store.load_instance_json(path) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let content = serde_json::to_string(&value).unwrap_or_default();
        if let Some(record) = parse_record_compat(&content) {
            if record.type_namespace == type_namespace && record.type_name == type_name {
                records.push(record);
            }
        }
    }
    Ok(records)
}

fn get_record_by_id_fallback(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Record>, RepositoryError> {
    let path = format!("{EXTENSION_RECORD_DIR}/{id}.json");
    match store.load_instance_json(&path) {
        Ok(value) => {
            let content = serde_json::to_string(&value).unwrap_or_default();
            let record =
                parse_record_compat(&content).ok_or_else(|| RepositoryError::RecordLoad {
                    path: std::path::PathBuf::from(&path),
                    source: json_error("Failed to parse record"),
                })?;
            Ok(Some(record))
        }
        Err(RepositoryError::Io { .. } | RepositoryError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn parse_record_compat(content: &str) -> Option<Record> {
    if let Ok(record) = serde_json::from_str::<Record>(content) {
        return Some(record);
    }

    let value = serde_json::from_str::<serde_json::Value>(content).ok()?;
    let obj = value.as_object()?;
    let type_str = obj.get("type")?.as_str()?;
    let (type_namespace, type_name) = type_str.split_once('.')?;

    let field_values_obj = obj.get("fieldValues")?.as_object()?;
    let field_values = field_values_obj
        .iter()
        .map(|(k, v)| FieldValue {
            field_id: k.clone(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        })
        .collect();

    Some(Record {
        instance_id: obj.get("instanceId")?.as_str()?.to_string(),
        type_id: type_str.to_string(),
        type_version: obj.get("version").and_then(|v| v.as_u64()).unwrap_or(1) as u32,
        type_namespace: type_namespace.to_string(),
        type_name: type_name.to_string(),
        field_values,
        group_values: None,
        lifecycle_state: obj
            .get("lifecycleState")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        created_at: obj
            .get("createdAt")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        tags: obj.get("tags").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        updated_at: obj
            .get("updatedAt")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        extra: std::collections::HashMap::new(),
    })
}

fn json_error(msg: &str) -> serde_json::Error {
    serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        msg.to_string(),
    ))
}
