//! # Protocol Service
//!
//! Public API for Protocol definition CRUD operations. This module is the sole entry point
//! for all protocol logic. CLI handlers and future API handlers must call these functions;
//! they must not call internal helpers or store I/O methods directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes `store: &dyn RepositoryStore` and returns a typed result.
//! - All validation, orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Storage model
//!
//! Protocols are **Package definitions**, exactly parallel to Blueprints (see
//! [`crate::blueprint_service`]). Each Protocol is a JSON file under `package/protocols/`
//! whose relative path is registered in the boundary's `package.json` `protocols[]` array.
//! Protocols are identified by `protocolId`. They are **not** instance Records — this is what
//! the spec mandates (subsection 05-1-5-1, Invariant 037) and what makes a Protocol satisfy
//! both `protocol validate` and `repo validate` (the-greenman/srs-rust#169).
//!
//! ## Atomicity notes
//!
//! - **Create**: the protocol file is written first, then `package.json` is updated. If the
//!   `package.json` update fails, the orphaned file is left on disk and an error is returned
//!   including the orphaned path.
//! - **Delete**: `package.json` is updated first (entry removed), then the file is deleted. If
//!   file deletion fails after index removal, the entry is gone but the file remains as an orphan.

use srs_core::types::protocol::{Protocol, ProtocolDiagnosticSeverity, ProtocolStage};
use srs_core::validation::protocol::validate_protocol;

use crate::blueprint_service::validate_package_selector;
use crate::error::RepositoryError;
use crate::package_types::{DefinitionKind, PackageSelector};
use crate::store::RepositoryStore;

const PROTOCOLS_DIR: &str = "protocols";

// ---------------------------------------------------------------------------
// Public input/result types
// ---------------------------------------------------------------------------

pub struct ImportProtocolInput {
    pub raw: serde_json::Value,
}

pub struct CreateProtocolResult {
    /// The stored definition JSON (preserved verbatim, including stage fields beyond the
    /// `ProtocolStage` struct).
    pub protocol: serde_json::Value,
}

pub struct UpdateProtocolResult {
    pub protocol: serde_json::Value,
}

pub struct DeleteProtocolResult {
    pub protocol_id: String,
}

/// Result for protocol get / export operations.
///
/// Carries the raw stored definition JSON rather than a typed [`Protocol`], so that stage fields
/// beyond the `ProtocolStage` struct (e.g. `contributesTo`, `completionCriteria`) survive a
/// get/export round-trip.
#[derive(Debug, Clone)]
pub enum GetProtocolResult {
    Found(serde_json::Value),
    NotFound,
}

/// Result for protocol validation.
#[derive(Debug, Clone)]
pub struct ValidateProtocolResult {
    pub protocol_id: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

/// Summary of a protocol for list operations.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSummary {
    pub protocol_id: String,
    pub protocol_namespace: String,
    pub protocol_name: String,
    pub protocol_version: i32,
    pub stage_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

/// Result for finding a protocol by its target type ID.
#[derive(Debug, Clone)]
pub struct FindProtocolByTargetTypeResult {
    pub protocol_id: String,
    pub protocol_name: String,
    pub stages: Vec<ProtocolStage>,
    pub diagnostics: Vec<String>,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn slugify(name: &str) -> String {
    name.to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// Parse and structurally validate a protocol definition JSON into a typed [`Protocol`].
fn protocol_from_value(value: &serde_json::Value) -> Result<Protocol, RepositoryError> {
    serde_json::from_value(value.clone()).map_err(|e| {
        RepositoryError::InvalidRepositoryInitialization {
            message: format!("invalid protocol definition: {e}"),
        }
    })
}

/// Run semantic validation, returning the joined error messages when invalid.
///
/// Covers the field-shape rules the importer used to enforce (version >= 1, RFC 3339
/// `createdAt`, non-empty stage id/name, non-negative order) plus the stage dependency-graph
/// validation in [`validate_protocol`].
fn check_protocol(protocol: &Protocol) -> Result<(), RepositoryError> {
    let mut messages: Vec<String> = vec![];

    if protocol.protocol_version < 1 {
        messages.push(format!(
            "protocolVersion must be >= 1, got {}",
            protocol.protocol_version
        ));
    }
    if chrono::DateTime::parse_from_rfc3339(&protocol.protocol_created_at).is_err() {
        messages.push(format!(
            "protocolCreatedAt must be a valid RFC 3339 datetime, got '{}'",
            protocol.protocol_created_at
        ));
    }
    for stage in &protocol.protocol_stages {
        messages.extend(srs_core::validation::protocol::validate_protocol_stage(
            stage,
        ));
    }

    let validation = validate_protocol(protocol);
    if !validation.valid {
        messages.extend(
            validation
                .diagnostics
                .into_iter()
                .filter(|d| d.severity == ProtocolDiagnosticSeverity::Error)
                .map(|d| d.message),
        );
    }

    if messages.is_empty() {
        Ok(())
    } else {
        Err(RepositoryError::InvalidRepositoryInitialization {
            message: messages.join("; "),
        })
    }
}

/// Locate the full repo-root-relative path (and owning boundary) for a protocol by its
/// `protocolId`, scanning each boundary's `package.json` `protocols[]` array.
fn find_protocol_path(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<(String, PackageSelector)>, RepositoryError> {
    for boundary in store.list_package_boundaries()? {
        let prefix = boundary.selector.as_deref().unwrap_or("package");
        let Ok(pkg_json) = store.load_instance_json(&format!("{prefix}/package.json")) else {
            continue;
        };
        let Some(paths) = pkg_json["protocols"].as_array() else {
            continue;
        };
        for entry in paths {
            if let Some(rel) = entry.as_str() {
                let full = format!("{prefix}/{rel}");
                if let Ok(val) = store.load_instance_json(&full) {
                    if val["protocolId"].as_str() == Some(id) {
                        return Ok(Some((full, boundary.selector.clone())));
                    }
                }
            }
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Read-only service functions
// ---------------------------------------------------------------------------

/// List protocol summaries from the compiled package model.
pub fn list_protocols(
    store: &dyn RepositoryStore,
) -> Result<Vec<ProtocolSummary>, RepositoryError> {
    let package = store.load_package()?;
    let summaries = package
        .protocols
        .into_iter()
        .map(|lp| ProtocolSummary {
            protocol_id: lp.protocol.protocol_id.clone(),
            protocol_namespace: lp.protocol.protocol_namespace.clone(),
            protocol_name: lp.protocol.protocol_name.clone(),
            protocol_version: lp.protocol.protocol_version,
            stage_count: lp.protocol.protocol_stages.len(),
            source_package: lp.source_package,
        })
        .collect();
    Ok(summaries)
}

/// Get a protocol's stored definition JSON by its `protocolId`.
pub fn get_protocol_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetProtocolResult, RepositoryError> {
    let package = store.load_package()?;
    match package
        .protocols
        .into_iter()
        .find(|lp| lp.protocol.protocol_id == id)
    {
        Some(lp) => Ok(GetProtocolResult::Found(lp.raw)),
        None => Ok(GetProtocolResult::NotFound),
    }
}

/// Export a protocol's portable definition (identical to `get` — the stored definition is
/// already the canonical, instance-free import format).
pub fn export_protocol(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetProtocolResult, RepositoryError> {
    get_protocol_by_id(store, id)
}

/// List a protocol's stages, sorted by `order`.
pub fn list_protocol_stages(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Vec<ProtocolStage>, RepositoryError> {
    match get_protocol_by_id(store, id)? {
        GetProtocolResult::Found(val) => {
            let proto = protocol_from_value(&val)?;
            let mut stages = proto.protocol_stages;
            stages.sort_by_key(|s| s.order);
            Ok(stages)
        }
        GetProtocolResult::NotFound => Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("{PROTOCOLS_DIR}/{id}")),
        }),
    }
}

/// Validate a protocol definition's stage dependency graph.
pub fn validate_protocol_definition(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<ValidateProtocolResult, RepositoryError> {
    match get_protocol_by_id(store, id)? {
        GetProtocolResult::Found(val) => {
            let proto = protocol_from_value(&val)?;
            let validation = validate_protocol(&proto);
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
                protocol_id: proto.protocol_id,
                valid: validation.valid,
                diagnostics,
            })
        }
        GetProtocolResult::NotFound => Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from(format!("{PROTOCOLS_DIR}/{id}")),
        }),
    }
}

/// Find the first protocol whose `protocolTargetType` matches `target_type_id`.
///
/// Returns `None` when no protocol targets that type.
pub fn find_protocol_by_target_type(
    store: &dyn RepositoryStore,
    target_type_id: &str,
) -> Result<Option<FindProtocolByTargetTypeResult>, RepositoryError> {
    let package = store.load_package()?;
    for lp in package.protocols {
        if lp.protocol.protocol_target_type != target_type_id {
            continue;
        }
        let stages = lp.protocol.protocol_stages.clone();
        return Ok(Some(FindProtocolByTargetTypeResult {
            protocol_id: lp.protocol.protocol_id,
            protocol_name: lp.protocol.protocol_name,
            stages,
            diagnostics: vec![],
        }));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Mutating service functions
// ---------------------------------------------------------------------------

/// Create a new Protocol definition from its JSON value.
///
/// Validates the selector and the protocol, writes the definition file (verbatim) under
/// `package/protocols/`, then registers it in the boundary's `package.json` `protocols[]`.
pub fn create_protocol(
    store: &dyn RepositoryStore,
    value: serde_json::Value,
    selector: PackageSelector,
) -> Result<CreateProtocolResult, RepositoryError> {
    validate_package_selector(&selector)?;
    store.load_package_boundary(&selector)?;

    let protocol = protocol_from_value(&value)?;
    check_protocol(&protocol)?;

    let boundary_path = selector.as_deref().unwrap_or("package");
    store.ensure_instance_dir(&format!("{boundary_path}/{PROTOCOLS_DIR}"))?;

    let id_prefix = &protocol.protocol_id[..protocol.protocol_id.len().min(8)];
    let rel_filename = format!(
        "{PROTOCOLS_DIR}/{}-{}.json",
        slugify(&protocol.protocol_name),
        id_prefix
    );
    let full_path = format!("{boundary_path}/{rel_filename}");

    // Write file first (atomicity: file before index). Store the value verbatim to preserve
    // stage fields beyond the `ProtocolStage` struct.
    store.save_instance_json(&full_path, &value)?;

    // Then register in the boundary's package.json.
    if let Err(e) =
        store.add_definition_to_boundary(&selector, DefinitionKind::Protocol, &rel_filename)
    {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "protocol file written to '{full_path}' but package.json update failed: {e}; \
                 repair by deleting the orphaned file or re-running create"
            ),
        });
    }

    Ok(CreateProtocolResult { protocol: value })
}

/// Import a Protocol definition from a JSON payload.
///
/// Accepts either a bare Protocol object or `{ "protocol": { ... } }`. The payload must use the
/// canonical camelCase keys (`protocolId`, `protocolStages`, …) — the same shape `export` emits.
pub fn import_protocol(
    store: &dyn RepositoryStore,
    input: ImportProtocolInput,
    selector: PackageSelector,
) -> Result<CreateProtocolResult, RepositoryError> {
    let value = input.raw.get("protocol").cloned().unwrap_or(input.raw);
    create_protocol(store, value, selector)
}

/// Update an existing Protocol definition (full replace) from its JSON value.
///
/// Preserves the original `protocolCreatedAt` from the stored value.
pub fn update_protocol(
    store: &dyn RepositoryStore,
    id: &str,
    mut value: serde_json::Value,
) -> Result<UpdateProtocolResult, RepositoryError> {
    let (path, _owner) =
        find_protocol_path(store, id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(PROTOCOLS_DIR),
        })?;

    // Preserve the original createdAt.
    let stored = store.load_instance_json(&path)?;
    if let (Some(created), Some(obj)) =
        (stored["protocolCreatedAt"].as_str(), value.as_object_mut())
    {
        obj.insert(
            "protocolCreatedAt".to_string(),
            serde_json::Value::String(created.to_string()),
        );
    }

    let protocol = protocol_from_value(&value)?;
    check_protocol(&protocol)?;

    store.save_instance_json(&path, &value)?;
    Ok(UpdateProtocolResult { protocol: value })
}

/// Delete a Protocol by `protocolId`.
///
/// Removes the entry from `package.json` first, then deletes the file.
pub fn delete_protocol(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<DeleteProtocolResult, RepositoryError> {
    let (full_path, owner) =
        find_protocol_path(store, id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from(PROTOCOLS_DIR),
        })?;

    let boundary_prefix = owner.as_deref().unwrap_or("package");
    let rel_path = full_path
        .strip_prefix(&format!("{boundary_prefix}/"))
        .unwrap_or(&full_path)
        .to_string();

    // Remove from package.json first (atomicity: index before file).
    store.remove_definition_from_boundary(&owner, DefinitionKind::Protocol, &rel_path)?;

    if let Err(e) = store.delete_instance_file(&full_path) {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "[WARN] removed '{full_path}' from package index but file deletion failed: {e}; \
                 orphaned file may remain at '{full_path}'"
            ),
        });
    }

    Ok(DeleteProtocolResult {
        protocol_id: id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::LoadedProtocol;
    use crate::store::memory::MemoryStore;
    use srs_core::types::protocol::Protocol;

    fn make_protocol(id: &str, target_type: &str, name: &str) -> LoadedProtocol {
        let protocol = Protocol {
            protocol_id: id.to_string(),
            protocol_namespace: "com.test".to_string(),
            protocol_name: name.to_string(),
            protocol_version: 1,
            protocol_description: None,
            protocol_target_type: target_type.to_string(),
            protocol_stages: vec![],
            protocol_tags: None,
            protocol_created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let raw = serde_json::to_value(&protocol).unwrap();
        LoadedProtocol {
            protocol,
            raw,
            source_package: None,
        }
    }

    #[test]
    fn list_protocols_empty() {
        let store = MemoryStore::empty();
        let result = list_protocols(&store).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn list_protocols_returns_all() {
        let store = MemoryStore::empty()
            .with_protocol(make_protocol("proto-001", "type-a", "Alpha"))
            .with_protocol(make_protocol("proto-002", "type-b", "Beta"));
        let result = list_protocols(&store).unwrap();
        assert_eq!(result.len(), 2);
        let ids: Vec<&str> = result.iter().map(|s| s.protocol_id.as_str()).collect();
        assert!(ids.contains(&"proto-001"));
        assert!(ids.contains(&"proto-002"));
    }

    #[test]
    fn get_protocol_by_id_found() {
        let store =
            MemoryStore::empty().with_protocol(make_protocol("proto-001", "type-a", "Alpha"));
        match get_protocol_by_id(&store, "proto-001").unwrap() {
            GetProtocolResult::Found(val) => {
                assert_eq!(val["protocolId"].as_str(), Some("proto-001"));
            }
            GetProtocolResult::NotFound => panic!("expected Found"),
        }
    }

    #[test]
    fn get_protocol_by_id_not_found() {
        let store = MemoryStore::empty();
        assert!(matches!(
            get_protocol_by_id(&store, "missing-id").unwrap(),
            GetProtocolResult::NotFound
        ));
    }

    #[test]
    fn find_protocol_by_target_type_found() {
        let store = MemoryStore::empty()
            .with_protocol(make_protocol("proto-001", "type-a", "Alpha"))
            .with_protocol(make_protocol("proto-002", "type-b", "Beta"));
        let result = find_protocol_by_target_type(&store, "type-b").unwrap();
        let found = result.expect("should find protocol for type-b");
        assert_eq!(found.protocol_id, "proto-002");
        assert_eq!(found.protocol_name, "Beta");
    }

    #[test]
    fn find_protocol_by_target_type_not_found() {
        let store =
            MemoryStore::empty().with_protocol(make_protocol("proto-001", "type-a", "Alpha"));
        let result = find_protocol_by_target_type(&store, "type-x").unwrap();
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use crate::json_store::JsonStore;
    use crate::repository_lifecycle::{
        create_repository, InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    };
    use tempfile::TempDir;

    fn init_store() -> (TempDir, JsonStore) {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(
            &store,
            &InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "repo-001".to_string(),
                    namespace: "com.test".to_string(),
                    srs_version: "2.0-draft".to_string(),
                    title: None,
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "pkg-001".to_string(),
                    namespace: "com.test".to_string(),
                    name: "primary".to_string(),
                    version: "1.0.0".to_string(),
                },
            },
        )
        .unwrap();
        (tmp, store)
    }

    fn proto_json(id: &str, target_type: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "protocolId": id,
            "protocolNamespace": "com.test",
            "protocolName": name,
            "protocolVersion": 1,
            "protocolTargetType": target_type,
            "protocolCreatedAt": "2026-01-01T00:00:00Z",
            "protocolStages": [{
                "stageId": "s1",
                "name": "Gather",
                "order": 1,
                "dependsOn": [],
                "question": "What is the topic?"
            }]
        })
    }

    #[test]
    fn protocol_roundtrip_create_list_get_find() {
        let (_tmp, store) = init_store();

        // Write via create path
        create_protocol(
            &store,
            proto_json("proto-rt-001", "type-aaa", "Roundtrip Protocol"),
            None,
        )
        .unwrap();

        // list_protocols reads compiled model
        let list = list_protocols(&store).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].protocol_id, "proto-rt-001");
        assert_eq!(list[0].protocol_name, "Roundtrip Protocol");
        assert_eq!(list[0].stage_count, 1);

        // get_protocol_by_id reads compiled model
        match get_protocol_by_id(&store, "proto-rt-001").unwrap() {
            GetProtocolResult::Found(val) => {
                assert_eq!(val["protocolId"].as_str(), Some("proto-rt-001"));
            }
            GetProtocolResult::NotFound => panic!("expected Found"),
        }

        // find_protocol_by_target_type reads compiled model
        let found = find_protocol_by_target_type(&store, "type-aaa")
            .unwrap()
            .expect("should find protocol");
        assert_eq!(found.protocol_id, "proto-rt-001");
        assert_eq!(found.stages.len(), 1);
        assert_eq!(found.stages[0].stage_id, "s1");

        // Confirm a missing id and type return the right results
        assert!(matches!(
            get_protocol_by_id(&store, "missing").unwrap(),
            GetProtocolResult::NotFound
        ));
        assert!(find_protocol_by_target_type(&store, "type-zzz")
            .unwrap()
            .is_none());
    }
}
