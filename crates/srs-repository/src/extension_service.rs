use std::path::Path;

use srs_core::types::record::{FieldValue, Record};

use crate::error::RepositoryError;
use crate::record_store::{get_record_by_id, list_records_by_type};
use crate::store::FileStore;

pub fn list_extensions(repo_root: &Path) -> Result<Vec<Record>, RepositoryError> {
    let store = FileStore::new(repo_root);
    let mut records = list_records_by_type(&store, "meta", "extension")?;
    if records.is_empty() {
        records = list_records_by_type_fallback(repo_root, "meta", "extension")?;
    }
    Ok(records)
}

pub fn get_extension_by_id(repo_root: &Path, id: &str) -> Result<Option<Record>, RepositoryError> {
    let store = FileStore::new(repo_root);
    match get_record_by_id(&store, id)? {
        Some(record) => {
            if is_extension_type(&record) {
                Ok(Some(record))
            } else {
                Ok(None)
            }
        }
        None => {
            if let Some(record) = get_record_by_id_fallback(repo_root, id)? {
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
    repo_root: &Path,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let records_dir = repo_root.join("package/records");
    if !records_dir.exists() {
        return Ok(vec![]);
    }

    let mut records = vec![];
    for entry in std::fs::read_dir(&records_dir).map_err(|source| RepositoryError::Io {
        path: records_dir.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| RepositoryError::Io {
            path: records_dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path).map_err(|source| RepositoryError::Io {
            path: path.clone(),
            source,
        })?;
        if let Some(record) = parse_record_compat(&content) {
            if record.type_namespace == type_namespace && record.type_name == type_name {
                records.push(record);
            }
        }
    }
    Ok(records)
}

fn get_record_by_id_fallback(
    repo_root: &Path,
    id: &str,
) -> Result<Option<Record>, RepositoryError> {
    let path = repo_root.join("package/records").join(format!("{id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path).map_err(|source| RepositoryError::Io {
        path: path.clone(),
        source,
    })?;
    let record = parse_record_compat(&content).ok_or_else(|| RepositoryError::RecordLoad {
        path: path.clone(),
        source: json_error("Failed to parse record"),
    })?;
    Ok(Some(record))
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
        created_at: obj
            .get("createdAt")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
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
