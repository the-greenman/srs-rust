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
//! ## Storage model (ADR-006)
//!
//! Protocol definitions are generic Tier 2 Records bound to the spec type
//! `com.semanticops.srs/meta.protocol@1` (UUID `48a03f5d-4f27-42f4-b791-999f6c22f8d2`).
//! Field values are stored by UUID field ID, not by human-readable name.
//! Records are indexed in `manifest.json` `instanceIndex` under `records/protocols/`.

use srs_core::types::protocol::{
    Protocol, ProtocolDiagnosticSeverity, ProtocolStage, ProtocolStageSummary,
};
use srs_core::types::record::FieldValue;
use srs_core::types::record::Record;
use srs_core::validation::protocol::validate_protocol;

use crate::error::RepositoryError;
use crate::record_store::{create_record, get_record_by_id};
use crate::store::RepositoryStore;

// ---------------------------------------------------------------------------
// Field ID constants — authoritative UUIDs from srs/srs/package/types/meta.protocol.json
// ---------------------------------------------------------------------------

const FIELD_PROTOCOL_ID: &str = "6c66d06c-3f95-4d17-8ecf-e1046a6f2ec1";
const FIELD_PROTOCOL_NAMESPACE: &str = "8d0f55f9-80e3-4dd6-a05c-10c4b6b6cc87";
const FIELD_PROTOCOL_NAME: &str = "09c5e389-cf6c-4f72-aad6-8cf26bce0b78";
const FIELD_PROTOCOL_VERSION: &str = "f7d28d9d-f90c-4a01-a3eb-2ff4cad54ff6";
const FIELD_PROTOCOL_DESCRIPTION: &str = "7d1d2f86-b5b6-4f95-82c9-dd8f820b1d04";
const FIELD_PROTOCOL_TARGET_TYPE: &str = "4939a29b-7f70-481f-bf6b-bf693f8bd67f";
const FIELD_PROTOCOL_STAGES: &str = "0f1232c6-0db5-4383-b91d-64d81195f1c4";
const FIELD_PROTOCOL_TAGS: &str = "0eafae91-91a8-4115-a95f-fde3d22a87af";
const FIELD_PROTOCOL_CREATED_AT: &str = "b953f716-383a-4218-bebf-96e93c4747a4";

const PROTOCOL_TYPE_ID: &str = "48a03f5d-4f27-42f4-b791-999f6c22f8d2";
const PROTOCOL_TYPE_VERSION: u32 = 1;
const PROTOCOL_STORAGE_DIR: &str = "records/protocols";

// ---------------------------------------------------------------------------
// Public input/result types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Public service functions
// ---------------------------------------------------------------------------

/// List protocol definitions
pub fn list_protocols(
    store: &dyn RepositoryStore,
) -> Result<Vec<ProtocolSummary>, RepositoryError> {
    use crate::record_store::list_records_by_type;

    let records = list_records_by_type(store, "com.semanticops.srs", "meta.protocol")?;

    let mut summaries = vec![];
    for record in records {
        let summary = record_to_protocol_summary(&record)?;
        summaries.push(summary);
    }

    Ok(summaries)
}

/// Get a protocol definition by ID
pub fn get_protocol_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetProtocolResult, RepositoryError> {
    match get_record_by_id(store, id)? {
        Some(record) if is_protocol_type(&record) => {
            let instance_id = record.instance_id.clone();
            let protocol = record_to_protocol(&record)?;
            let protocol_json =
                serde_json::to_value(&protocol).map_err(|e| RepositoryError::Serialize {
                    path: std::path::PathBuf::from("protocol"),
                    source: e,
                })?;
            Ok(GetProtocolResult::Found {
                instance_id,
                protocol: protocol_json,
            })
        }
        Some(_) | None => Ok(GetProtocolResult::NotFound),
    }
}

/// Import a protocol definition from a JSON payload
pub fn import_protocol(
    store: &dyn RepositoryStore,
    input: ImportProtocolInput,
) -> Result<ImportProtocolResult, RepositoryError> {
    let json_value = input.raw;
    let pj = json_value.get("protocol").unwrap_or(&json_value);

    // Validate and extract required fields
    let protocol_id = require_string(pj, "protocolId", "protocol-id")?;
    let protocol_namespace = require_string(pj, "protocolNamespace", "protocol-namespace")?;
    let protocol_name = require_string(pj, "protocolName", "protocol-name")?;
    let protocol_version = require_version(pj)?;
    let protocol_target_type = require_string(pj, "protocolTargetType", "protocol-target-type")?;
    let protocol_created_at = require_created_at(pj)?;
    let stages_value = require_stages(pj)?;

    let mut field_values = vec![
        fv(FIELD_PROTOCOL_ID, serde_json::Value::String(protocol_id)),
        fv(
            FIELD_PROTOCOL_NAMESPACE,
            serde_json::Value::String(protocol_namespace),
        ),
        fv(FIELD_PROTOCOL_NAME, serde_json::Value::String(protocol_name)),
        fv(
            FIELD_PROTOCOL_VERSION,
            serde_json::Value::Number(protocol_version.into()),
        ),
        fv(
            FIELD_PROTOCOL_TARGET_TYPE,
            serde_json::Value::String(protocol_target_type),
        ),
        fv(FIELD_PROTOCOL_STAGES, stages_value),
        fv(
            FIELD_PROTOCOL_CREATED_AT,
            serde_json::Value::String(protocol_created_at),
        ),
    ];

    // Optional fields
    if let Some(desc) = pj
        .get("protocolDescription")
        .or_else(|| pj.get("protocol-description"))
    {
        field_values.push(fv(FIELD_PROTOCOL_DESCRIPTION, desc.clone()));
    }
    if let Some(tags) = pj
        .get("protocolTags")
        .or_else(|| pj.get("protocol-tags"))
    {
        field_values.push(fv(FIELD_PROTOCOL_TAGS, tags.clone()));
    }

    let record = create_record(
        store,
        PROTOCOL_TYPE_ID,
        PROTOCOL_TYPE_VERSION,
        field_values,
        PROTOCOL_STORAGE_DIR,
    )
    .map_err(|e| match e {
        RepositoryError::TypeNotFound { .. } => {
            RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "Repository package does not declare type \
                     'com.semanticops.srs/meta.protocol@1' (UUID {}). \
                     Add it to your package before importing protocols.",
                    PROTOCOL_TYPE_ID
                ),
            }
        }
        other => other,
    })?;

    let instance_id = record.instance_id.clone();
    Ok(ImportProtocolResult {
        instance_id,
        record,
    })
}

/// List stages for a protocol, sorted by order
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
            path: std::path::PathBuf::from(PROTOCOL_STORAGE_DIR),
        }),
    }
}

/// Validate a protocol definition's stage dependency graph
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
                    let sev = match d.severity {
                        ProtocolDiagnosticSeverity::Error => "ERROR",
                        ProtocolDiagnosticSeverity::Warning => "WARNING",
                    };
                    format!("[{}] {}", sev, d.message)
                })
                .collect();
            Ok(ValidateProtocolResult {
                instance_id,
                valid: validation.valid,
                diagnostics,
            })
        }
        None => Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from(PROTOCOL_STORAGE_DIR),
        }),
    }
}

/// Get the portable export representation of a protocol (no instanceId — suitable for import)
pub fn export_protocol(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetProtocolResult, RepositoryError> {
    match get_record_by_id(store, id)? {
        Some(record) if is_protocol_type(&record) => {
            let instance_id = record.instance_id.clone();
            let protocol = record_to_protocol(&record)?;
            // Serialize without instanceId — this is the canonical import format
            let protocol_json =
                serde_json::to_value(&protocol).map_err(|e| RepositoryError::Serialize {
                    path: std::path::PathBuf::from("protocol"),
                    source: e,
                })?;
            Ok(GetProtocolResult::Found {
                instance_id,
                protocol: protocol_json,
            })
        }
        Some(_) | None => Ok(GetProtocolResult::NotFound),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn get_protocol_struct_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, Box<Protocol>)>, RepositoryError> {
    match get_record_by_id(store, id)? {
        Some(r) if is_protocol_type(&r) => {
            let instance_id = r.instance_id.clone();
            let protocol = record_to_protocol(&r)?;
            Ok(Some((instance_id, Box::new(protocol))))
        }
        Some(_) | None => Ok(None),
    }
}

fn is_protocol_type(record: &Record) -> bool {
    record.type_namespace == "com.semanticops.srs" && record.type_name == "meta.protocol"
}

fn record_to_protocol(record: &Record) -> Result<Protocol, RepositoryError> {
    let fv = &record.field_values;

    let stages_json = find_fv(fv, FIELD_PROTOCOL_STAGES).ok_or_else(|| {
        RepositoryError::ManifestParse {
            path: std::path::PathBuf::from("protocol"),
            source: json_error("Missing protocol-stages field"),
        }
    })?;

    let protocol_stages: Vec<ProtocolStage> =
        serde_json::from_value(stages_json.clone()).map_err(|e| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from("protocol"),
            source: e,
        })?;

    Ok(Protocol {
        protocol_id: get_string_fv(fv, FIELD_PROTOCOL_ID, "protocol-id")?,
        protocol_namespace: get_string_fv(fv, FIELD_PROTOCOL_NAMESPACE, "protocol-namespace")?,
        protocol_name: get_string_fv(fv, FIELD_PROTOCOL_NAME, "protocol-name")?,
        protocol_version: get_i32_fv(fv, FIELD_PROTOCOL_VERSION, "protocol-version")?,
        protocol_description: find_fv(fv, FIELD_PROTOCOL_DESCRIPTION)
            .and_then(|v| v.as_str().map(|s| s.to_string())),
        protocol_target_type: get_string_fv(
            fv,
            FIELD_PROTOCOL_TARGET_TYPE,
            "protocol-target-type",
        )?,
        protocol_stages,
        protocol_tags: find_fv(fv, FIELD_PROTOCOL_TAGS).and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
        }),
        protocol_created_at: get_string_fv(fv, FIELD_PROTOCOL_CREATED_AT, "protocol-created-at")?,
    })
}

fn record_to_protocol_summary(record: &Record) -> Result<ProtocolSummary, RepositoryError> {
    let fv = &record.field_values;

    let stage_count = find_fv(fv, FIELD_PROTOCOL_STAGES)
        .and_then(|v| v.as_array().map(|arr| arr.len()))
        .unwrap_or(0);

    Ok(ProtocolSummary {
        instance_id: record.instance_id.clone(),
        protocol_id: get_string_fv(fv, FIELD_PROTOCOL_ID, "protocol-id")?,
        protocol_namespace: get_string_fv(fv, FIELD_PROTOCOL_NAMESPACE, "protocol-namespace")?,
        protocol_name: get_string_fv(fv, FIELD_PROTOCOL_NAME, "protocol-name")?,
        protocol_version: get_i32_fv(fv, FIELD_PROTOCOL_VERSION, "protocol-version")?,
        stage_count,
    })
}

// ---------------------------------------------------------------------------
// Field extraction helpers
// ---------------------------------------------------------------------------

fn find_fv<'a>(fv: &'a [FieldValue], uuid: &str) -> Option<&'a serde_json::Value> {
    fv.iter()
        .find(|f| f.field_id == uuid)
        .map(|f| &f.value)
}

fn get_string_fv(fv: &[FieldValue], uuid: &str, label: &str) -> Result<String, RepositoryError> {
    find_fv(fv, uuid)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from(label),
            source: json_error(&format!("Missing or invalid field: {}", label)),
        })
}

fn get_i32_fv(fv: &[FieldValue], uuid: &str, label: &str) -> Result<i32, RepositoryError> {
    find_fv(fv, uuid)
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
            path: std::path::PathBuf::from(label),
            source: json_error(&format!("Missing or invalid field: {}", label)),
        })
}

fn fv(field_id: &str, value: serde_json::Value) -> FieldValue {
    FieldValue {
        field_id: field_id.to_string(),
        value,
        entries: None,
        source: None,
        edited_at: None,
    }
}

// ---------------------------------------------------------------------------
// Input validation helpers (used by import_protocol)
// ---------------------------------------------------------------------------

fn require_string(
    pj: &serde_json::Value,
    camel: &str,
    kebab: &str,
) -> Result<String, RepositoryError> {
    pj.get(camel)
        .or_else(|| pj.get(kebab))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "Missing or invalid required field '{}' in protocol input",
                camel
            ),
        })
}

fn require_version(pj: &serde_json::Value) -> Result<i64, RepositoryError> {
    let v = pj
        .get("protocolVersion")
        .or_else(|| pj.get("protocol-version"))
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Missing required field 'protocolVersion' in protocol input".to_string(),
        })?;

    let n = if let Some(n) = v.as_i64() {
        n
    } else if let Some(s) = v.as_str() {
        s.parse::<i64>().map_err(|_| {
            RepositoryError::InvalidRepositoryInitialization {
                message: "Field 'protocolVersion' must be a positive integer".to_string(),
            }
        })?
    } else {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "Field 'protocolVersion' must be a positive integer".to_string(),
        });
    };

    if n < 1 {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "Field 'protocolVersion' must be >= 1, got {}",
                n
            ),
        });
    }
    Ok(n)
}

fn require_created_at(pj: &serde_json::Value) -> Result<String, RepositoryError> {
    let s = pj
        .get("protocolCreatedAt")
        .or_else(|| pj.get("protocol-created-at"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Missing required field 'protocolCreatedAt' in protocol input".to_string(),
        })?;

    chrono::DateTime::parse_from_rfc3339(s).map_err(|_| {
        RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "Field 'protocolCreatedAt' must be a valid RFC 3339 datetime, got '{}'",
                s
            ),
        }
    })?;

    Ok(s.to_string())
}

fn require_stages(pj: &serde_json::Value) -> Result<serde_json::Value, RepositoryError> {
    let stages = pj
        .get("protocolStages")
        .or_else(|| pj.get("protocol-stages"))
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Missing required field 'protocolStages' in protocol input".to_string(),
        })?;

    let arr = stages
        .as_array()
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "Field 'protocolStages' must be an array".to_string(),
        })?;

    // Collect all stage IDs for dependsOn validation
    let mut stage_ids = std::collections::HashSet::new();
    for (i, stage) in arr.iter().enumerate() {
        let obj = stage.as_object().ok_or_else(|| {
            RepositoryError::InvalidRepositoryInitialization {
                message: format!("protocolStages[{}] must be an object", i),
            }
        })?;

        let stage_id = obj
            .get("stageId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "protocolStages[{}].stageId must be a non-empty string",
                    i
                ),
            })?;
        stage_ids.insert(stage_id.to_string());

        // Validate name
        let _ = obj
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "protocolStages[{}] ('{}') .name must be a non-empty string",
                    i, stage_id
                ),
            })?;

        // Validate order
        let order = obj
            .get("order")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "protocolStages[{}] ('{}') .order must be a non-negative integer",
                    i, stage_id
                ),
            })?;
        if order < 0 {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "protocolStages[{}] ('{}') .order must be >= 0, got {}",
                    i, stage_id, order
                ),
            });
        }
    }

    // Second pass: validate dependsOn references
    for (i, stage) in arr.iter().enumerate() {
        let obj = stage.as_object().unwrap();
        let stage_id = obj.get("stageId").and_then(|v| v.as_str()).unwrap();
        if let Some(deps) = obj.get("dependsOn") {
            let dep_arr = deps
                .as_array()
                .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
                    message: format!(
                        "protocolStages[{}] ('{}') .dependsOn must be an array",
                        i, stage_id
                    ),
                })?;
            for dep in dep_arr {
                let dep_id = dep.as_str().ok_or_else(|| {
                    RepositoryError::InvalidRepositoryInitialization {
                        message: format!(
                            "protocolStages[{}] ('{}') .dependsOn entries must be strings",
                            i, stage_id
                        ),
                    }
                })?;
                if !stage_ids.contains(dep_id) {
                    return Err(RepositoryError::InvalidRepositoryInitialization {
                        message: format!(
                            "protocolStages[{}] ('{}') .dependsOn references unknown stageId '{}'",
                            i, stage_id, dep_id
                        ),
                    });
                }
            }
        }
    }

    Ok(stages.clone())
}

fn json_error(msg: &str) -> serde_json::Error {
    serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        msg.to_string(),
    ))
}
