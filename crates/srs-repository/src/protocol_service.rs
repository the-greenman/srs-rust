//! # Protocol Service
//!
//! Public API for protocol definition operations. This module is the sole entry point
//! for all protocol logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, field-ID mapping, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: ProtocolImportInput = serde_json::from_reader(io::stdin())?;
//! let result = protocol_service::import_protocol(store, input)?;
//! output::ok("protocol import", result)
//! ```

use srs_core::types::protocol::{
    Protocol, ProtocolDiagnosticSeverity, ProtocolStage, ProtocolStageSummary,
};
use srs_core::types::record::FieldValue;
use srs_core::types::record::Record;
use srs_core::validation::protocol::validate_protocol;

use crate::error::RepositoryError;
use crate::record_store::{create_record, get_record_by_id};
use crate::store::RepositoryStore;

pub struct ImportProtocolInput {
    pub raw: serde_json::Value,
}

pub struct ImportProtocolResult {
    pub instance_id: String,
    pub record: Record,
}

/// Result for protocol get operation
#[derive(Debug, Clone)]
pub enum GetProtocolResult {
    Found {
        instance_id: String,
        protocol: serde_json::Value,
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
pub fn get_protocol_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetProtocolResult, RepositoryError> {
    match get_record_by_id(store, id)? {
        Some(record) => {
            if !is_protocol_type(&record) {
                return Ok(GetProtocolResult::NotFound);
            }
            let instance_id = record.instance_id.clone();
            let protocol = record_to_protocol(&record)?;
            let mut protocol_json =
                serde_json::to_value(&protocol).map_err(|e| RepositoryError::Serialize {
                    path: std::path::PathBuf::from("protocol"),
                    source: e,
                })?;
            if let Some(obj) = protocol_json.as_object_mut() {
                obj.insert(
                    "instanceId".to_string(),
                    serde_json::Value::String(instance_id.clone()),
                );
            }
            Ok(GetProtocolResult::Found {
                instance_id,
                protocol: protocol_json,
            })
        }
        None => {
            if let Some(record) = get_record_by_id_fallback(store, id)? {
                if is_protocol_type(&record) {
                    let instance_id = record.instance_id.clone();
                    let protocol = record_to_protocol(&record)?;
                    let mut protocol_json = serde_json::to_value(&protocol).map_err(|e| {
                        RepositoryError::Serialize {
                            path: std::path::PathBuf::from("protocol"),
                            source: e,
                        }
                    })?;
                    if let Some(obj) = protocol_json.as_object_mut() {
                        obj.insert(
                            "instanceId".to_string(),
                            serde_json::Value::String(instance_id.clone()),
                        );
                    }
                    Ok(GetProtocolResult::Found {
                        instance_id,
                        protocol: protocol_json,
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

/// Import a protocol definition from a JSON payload
pub fn import_protocol(
    store: &dyn RepositoryStore,
    input: ImportProtocolInput,
) -> Result<ImportProtocolResult, RepositoryError> {
    let json_value = input.raw;
    let protocol_json = json_value.get("protocol").unwrap_or(&json_value);

    protocol_json
        .get("fieldValues")
        .or_else(|| protocol_json.get("protocolStages").map(|_| protocol_json))
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Missing protocol fields in JSON".to_string(),
        })?
        .as_object()
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Protocol fields must be an object".to_string(),
        })?;

    let mut field_values: Vec<FieldValue> = vec![];

    if let Some(v) = protocol_json
        .get("protocolId")
        .or_else(|| protocol_json.get("protocol-id"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-id".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolNamespace")
        .or_else(|| protocol_json.get("protocol-namespace"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-namespace".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolName")
        .or_else(|| protocol_json.get("protocol-name"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-name".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolVersion")
        .or_else(|| protocol_json.get("protocol-version"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-version".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolTargetType")
        .or_else(|| protocol_json.get("protocol-target-type"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-target-type".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolStages")
        .or_else(|| protocol_json.get("protocol-stages"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-stages".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }
    if let Some(v) = protocol_json
        .get("protocolCreatedAt")
        .or_else(|| protocol_json.get("protocol-created-at"))
    {
        field_values.push(FieldValue {
            field_id: "protocol-created-at".to_string(),
            value: v.clone(),
            entries: None,
            source: None,
            edited_at: None,
        });
    }

    let record = create_record(store, "meta.protocol", 1, field_values, "package/records")?;
    let instance_id = record.instance_id.clone();
    Ok(ImportProtocolResult {
        instance_id,
        record,
    })
}

/// List protocol definitions
pub fn list_protocols(
    store: &dyn RepositoryStore,
) -> Result<Vec<ProtocolSummary>, RepositoryError> {
    use crate::record_store::list_records_by_type;

    let mut records = list_records_by_type(store, "meta", "protocol")?;
    if records.is_empty() {
        records = list_records_by_type_fallback(store, "meta", "protocol")?;
    }

    let mut summaries = vec![];
    for record in records {
        let summary = record_to_protocol_summary(&record)?;
        summaries.push(summary);
    }

    Ok(summaries)
}

/// Internal helper: get a protocol as a typed struct (not yet enriched with instanceId).
fn get_protocol_struct_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, Box<Protocol>)>, RepositoryError> {
    let record = match get_record_by_id(store, id)? {
        Some(r) if is_protocol_type(&r) => r,
        Some(_) => return Ok(None),
        None => match get_record_by_id_fallback(store, id)? {
            Some(r) if is_protocol_type(&r) => r,
            _ => return Ok(None),
        },
    };
    let instance_id = record.instance_id.clone();
    let protocol = record_to_protocol(&record)?;
    Ok(Some((instance_id, Box::new(protocol))))
}

/// List stages for a protocol
pub fn list_protocol_stages(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Vec<ProtocolStageSummary>, RepositoryError> {
    match get_protocol_struct_by_id(store, id)? {
        Some((_instance_id, protocol)) => {
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

            stages.sort_by_key(|s| s.order);

            Ok(stages)
        }
        None => Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from("package/records"),
        }),
    }
}

/// Validate a protocol definition
pub fn validate_protocol_definition(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<ValidateProtocolResult, RepositoryError> {
    match get_protocol_struct_by_id(store, id)? {
        Some((instance_id, protocol)) => {
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
        None => Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from("package/records"),
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
    store: &dyn RepositoryStore,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let paths = match store.list_instance_files("package/records") {
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
    let path = format!("package/records/{id}.json");
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
        created_at: obj
            .get("createdAt")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        updated_at: obj
            .get("updatedAt")
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        extra: std::collections::HashMap::new(),
    })
}
