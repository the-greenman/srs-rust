use std::path::Path;

use srs_core::types::protocol::{
    Protocol, ProtocolDiagnosticSeverity, ProtocolStage, ProtocolStageSummary,
};
use srs_core::types::record::FieldValue;
use srs_core::types::record::Record;
use srs_core::validation::protocol::validate_protocol;

use crate::error::RepositoryError;
use crate::record_store::get_record_by_id;
use crate::store::FileStore;

/// Result for protocol get operation
#[derive(Debug, Clone)]
pub enum GetProtocolResult {
    Found {
        instance_id: String,
        protocol: Box<Protocol>,
    },
    NotFound,
}

/// Result for protocol validation
#[derive(Debug, Clone)]
pub struct ValidateProtocolResult {
    pub instance_id: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

/// Get a protocol definition by ID
/// Loads the record via generic get_record_by_id and deserializes the protocol fields
pub fn get_protocol_by_id(
    repo_root: &Path,
    id: &str,
) -> Result<GetProtocolResult, RepositoryError> {
    let store = FileStore::new(repo_root);
    match get_record_by_id(&store, id)? {
        Some(record) => {
            // Check if it's a protocol type
            if !is_protocol_type(&record) {
                return Ok(GetProtocolResult::NotFound);
            }

            // Extract protocol from record field values
            let protocol = record_to_protocol(&record)?;
            Ok(GetProtocolResult::Found {
                instance_id: record.instance_id,
                protocol: Box::new(protocol),
            })
        }
        None => {
            if let Some(record) = get_record_by_id_fallback(repo_root, id)? {
                if is_protocol_type(&record) {
                    let protocol = record_to_protocol(&record)?;
                    Ok(GetProtocolResult::Found {
                        instance_id: record.instance_id,
                        protocol: Box::new(protocol),
                    })
                } else {
                    Ok(GetProtocolResult::NotFound)
                }
            } else {
                Ok(GetProtocolResult::NotFound)
            }
        }
    }
}

/// List protocol definitions
/// Uses list_records_by_type with meta.protocol type
pub fn list_protocols(repo_root: &Path) -> Result<Vec<ProtocolSummary>, RepositoryError> {
    use crate::record_store::list_records_by_type;

    let store = FileStore::new(repo_root);
    let mut records = list_records_by_type(&store, "meta", "protocol")?;
    if records.is_empty() {
        records = list_records_by_type_fallback(repo_root, "meta", "protocol")?;
    }

    let mut summaries = vec![];
    for record in records {
        let summary = record_to_protocol_summary(&record)?;
        summaries.push(summary);
    }

    Ok(summaries)
}

/// List stages for a protocol
pub fn list_protocol_stages(
    repo_root: &Path,
    id: &str,
) -> Result<Vec<ProtocolStageSummary>, RepositoryError> {
    match get_protocol_by_id(repo_root, id)? {
        GetProtocolResult::Found { protocol, .. } => {
            let mut stages: Vec<ProtocolStageSummary> = protocol
                .protocol_stages
                .into_iter()
                .map(|s| ProtocolStageSummary {
                    stage_id: s.stage_id,
                    name: s.name,
                    order: s.order,
                    depends_on: s.depends_on,
                })
                .collect();

            // Sort by order
            stages.sort_by_key(|s| s.order);

            Ok(stages)
        }
        GetProtocolResult::NotFound => Err(RepositoryError::NotFound {
            path: repo_root.join("package/records"),
        }),
    }
}

/// Validate a protocol definition
pub fn validate_protocol_definition(
    repo_root: &Path,
    id: &str,
) -> Result<ValidateProtocolResult, RepositoryError> {
    match get_protocol_by_id(repo_root, id)? {
        GetProtocolResult::Found {
            instance_id,
            protocol,
        } => {
            let validation = validate_protocol(&protocol);

            let diagnostics: Vec<String> = validation
                .diagnostics
                .into_iter()
                .map(|d| {
                    let severity = match d.severity {
                        ProtocolDiagnosticSeverity::Error => "ERROR",
                        ProtocolDiagnosticSeverity::Warning => "WARNING",
                    };
                    format!("[{}] {}", severity, d.message)
                })
                .collect();

            Ok(ValidateProtocolResult {
                instance_id,
                valid: validation.valid,
                diagnostics,
            })
        }
        GetProtocolResult::NotFound => Err(RepositoryError::NotFound {
            path: repo_root.join("package/records"),
        }),
    }
}

/// Summary of a protocol for list operations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSummary {
    pub instance_id: String,
    pub protocol_id: String,
    pub protocol_namespace: String,
    pub protocol_name: String,
    pub protocol_version: i32,
    pub stage_count: usize,
}

fn is_protocol_type(record: &srs_core::types::record::Record) -> bool {
    record.type_namespace == "meta" && record.type_name == "protocol"
}

fn record_to_protocol(
    record: &srs_core::types::record::Record,
) -> Result<Protocol, RepositoryError> {
    let fv = &record.field_values;

    // Parse stages from field values
    let stages_json = find_field_value(fv, "protocol-stages")
        .or_else(|| find_field_value(fv, "stages"))
        .ok_or_else(|| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from("protocol"),
            source: json_error("Missing protocol-stages field"),
        })?;

    let protocol_stages: Vec<ProtocolStage> =
        serde_json::from_value(stages_json.clone()).map_err(|e| {
            RepositoryError::ManifestParse {
                path: std::path::PathBuf::from("protocol"),
                source: e,
            }
        })?;

    Ok(Protocol {
        protocol_id: get_string_field(fv, "protocol-id")?,
        protocol_namespace: get_string_field(fv, "protocol-namespace")?,
        protocol_name: get_string_field(fv, "protocol-name")?,
        protocol_version: get_i32_field(fv, "protocol-version")?,
        protocol_description: find_field_value(fv, "protocol-description")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        protocol_target_type: get_string_field(fv, "protocol-target-type")?,
        protocol_stages,
        protocol_tags: find_field_value(fv, "protocol-tags").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        protocol_created_at: get_string_field(fv, "protocol-created-at")?,
    })
}

fn record_to_protocol_summary(
    record: &srs_core::types::record::Record,
) -> Result<ProtocolSummary, RepositoryError> {
    let fv = &record.field_values;

    // Count stages
    let stage_count = find_field_value(fv, "protocol-stages")
        .or_else(|| find_field_value(fv, "stages"))
        .and_then(|v| v.as_array().map(|arr| arr.len()))
        .unwrap_or(0);

    Ok(ProtocolSummary {
        instance_id: record.instance_id.clone(),
        protocol_id: get_string_field(fv, "protocol-id")?,
        protocol_namespace: get_string_field(fv, "protocol-namespace")?,
        protocol_name: get_string_field(fv, "protocol-name")?,
        protocol_version: get_i32_field(fv, "protocol-version")?,
        stage_count,
    })
}

fn get_string_field(
    fv: &[srs_core::types::record::FieldValue],
    key: &str,
) -> Result<String, RepositoryError> {
    find_field_value(fv, key)
        .or_else(|| find_field_value(fv, &key.replace("-", "_")))
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from(key),
            source: json_error(&format!("Missing or invalid field: {}", key)),
        })
}

fn get_i32_field(
    fv: &[srs_core::types::record::FieldValue],
    key: &str,
) -> Result<i32, RepositoryError> {
    find_field_value(fv, key)
        .or_else(|| find_field_value(fv, &key.replace("-", "_")))
        .and_then(|v| {
            if let Some(n) = v.as_i64() {
                Some(n as i32)
            } else if let Some(s) = v.as_str() {
                s.parse::<i32>().ok()
            } else {
                None
            }
        })
        .ok_or_else(|| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from(key),
            source: json_error(&format!("Missing or invalid field: {}", key)),
        })
}

fn find_field_value<'a>(
    fv: &'a [srs_core::types::record::FieldValue],
    key: &str,
) -> Option<&'a serde_json::Value> {
    fv.iter()
        .find(|field| field.field_id == key)
        .map(|field| &field.value)
}

fn json_error(msg: &str) -> serde_json::Error {
    serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        msg.to_string(),
    ))
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
