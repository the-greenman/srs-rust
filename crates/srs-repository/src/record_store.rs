//! # Record Service
//!
//! Public API for record (Tier 2) operations. This module is the sole entry point for
//! all record logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, container orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: CreateRecordInput = serde_json::from_reader(io::stdin())?;
//! let result = record_store::create_record(store, input)?;
//! output::ok("record create", result)
//! ```

use crate::container_service;
use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::manifest::Manifest;
use crate::package_service::{get_type_by_name, GetTypeResult};
use crate::relation_service;
use crate::revision_service;
use crate::store::RepositoryStore;
use crate::writer::{new_instance_id, write_manifest};
use serde::Deserialize;
use srs_core::types::record::{FieldValue, Record};
use srs_core::types::relation::Relation;
use srs_core::types::revision::{Revision, RevisionAgent, RevisionProvenance};
use srs_core::validation::lifecycle::validate_type_lifecycle_v9;
use srs_core::validation::record::{validate_record, validate_type_lifecycle};
use std::collections::HashMap;

/// List all Tier 2 records in the repository, regardless of type.
pub fn list_all_records(store: &dyn RepositoryStore) -> Result<Vec<Record>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        records.push(load_record(store, entry.path())?);
    }

    Ok(records)
}

/// List all Tier 2 records matching the given type namespace and name.
pub fn list_records_by_type(
    store: &dyn RepositoryStore,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        let record = load_record(store, entry.path())?;
        if record.type_namespace == type_namespace && record.type_name == type_name {
            records.push(record);
        }
    }

    Ok(records)
}

/// Get a record by its instance ID.
pub fn get_record_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Record>, RepositoryError> {
    let manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id);

    match entry {
        Some(entry) => {
            let record = load_record(store, entry.path())?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

/// Create a new Tier 2 record.
pub fn create_record(
    store: &dyn RepositoryStore,
    type_id: &str,
    type_version: u32,
    field_values: Vec<FieldValue>,
    group_values: Option<Vec<srs_core::types::record::FieldGroupValue>>,
    relative_dir: &str,
) -> Result<Record, RepositoryError> {
    let package = store.load_package()?;
    let record_type = package.resolve_type(type_id, type_version).ok_or_else(|| {
        RepositoryError::TypeNotFound {
            type_id: type_id.to_string(),
            version: type_version,
        }
    })?;

    // Invariants 4+5: validate Type's lifecycle definition before using it.
    if let Some(lc) = &record_type.lifecycle {
        validate_type_lifecycle(lc).map_err(|e| RepositoryError::RecordValidation {
            path: std::path::PathBuf::from(relative_dir),
            source: e,
        })?;
        // V9 invariants: final-state outgoing transitions, duplicate IDs, etc.
        let v9_diags = validate_type_lifecycle_v9(&lc.states, &lc.transitions, &record_type.name);
        if !v9_diags.is_empty() {
            let msg = v9_diags
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            return Err(RepositoryError::InvalidRepositoryInitialization { message: msg });
        }
    }

    let initial_lifecycle_state = record_type
        .lifecycle
        .as_ref()
        .map(|lc| lc.initial_state.clone());

    let mut record = Record {
        instance_id: String::new(),
        type_id: type_id.to_string(),
        type_version,
        type_namespace: record_type.namespace.clone(),
        type_name: record_type.name.clone(),
        field_values,
        group_values,
        lifecycle_state: initial_lifecycle_state,
        created_at: Some(chrono::Utc::now().to_rfc3339()),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extra: HashMap::new(),
    };

    let effective_fields = package.effective_fields(record_type)?;
    validate_record(&record, record_type, &effective_fields).map_err(|e| {
        RepositoryError::RecordValidation {
            path: std::path::PathBuf::from(relative_dir),
            source: e,
        }
    })?;

    record.instance_id = new_instance_id();

    store.ensure_instance_dir(relative_dir)?;

    let relative_path = format!("{}/{}.json", relative_dir, record.instance_id);
    write_record(store, &record, &relative_path)?;

    let mut manifest = store.load_manifest()?;
    upsert_record_index_entry(&mut manifest, &record, &relative_path);
    write_manifest(store, &manifest)?;

    Ok(record)
}

/// Load a record from the store.
fn load_record(
    store: &dyn RepositoryStore,
    relative_path: &str,
) -> Result<Record, RepositoryError> {
    let value = store.load_instance_json(relative_path)?;
    serde_json::from_value(value).map_err(|e| RepositoryError::RecordLoad {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })
}

/// Write a record to the store.
fn write_record(
    store: &dyn RepositoryStore,
    record: &Record,
    relative_path: &str,
) -> Result<(), RepositoryError> {
    let value = serde_json::to_value(record).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })?;
    store.save_instance_json(relative_path, &value)
}

/// Update an existing Tier 2 record.
pub fn update_record(
    store: &dyn RepositoryStore,
    instance_id: &str,
    field_values: Vec<FieldValue>,
    group_values: Option<Option<Vec<srs_core::types::record::FieldGroupValue>>>,
) -> Result<Record, RepositoryError> {
    let record =
        get_record_by_id(store, instance_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let package = store.load_package()?;
    let record_type = package
        .resolve_type(&record.type_id, record.type_version)
        .ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: record.type_id.clone(),
            version: record.type_version,
        })?;

    let updated_record = Record {
        instance_id: record.instance_id,
        type_id: record.type_id,
        type_version: record.type_version,
        type_namespace: record.type_namespace,
        type_name: record.type_name,
        field_values,
        // None outer = not supplied by caller → preserve existing; Some(v) = replace.
        group_values: group_values.unwrap_or(record.group_values),
        lifecycle_state: record.lifecycle_state,
        created_at: record.created_at,
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extra: record.extra,
    };

    let effective_fields = package.effective_fields(record_type)?;
    validate_record(&updated_record, record_type, &effective_fields).map_err(|e| {
        RepositoryError::RecordValidation {
            path: std::path::PathBuf::from("records"),
            source: e,
        }
    })?;

    let mut manifest = store.load_manifest()?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == instance_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    write_record(store, &updated_record, entry.path())?;
    upsert_record_index_entry(&mut manifest, &updated_record, entry.path());
    write_manifest(store, &manifest)?;

    Ok(updated_record)
}

/// Returns the IDs of any Relations that reference `instance_id` as source or target.
fn find_relations_referencing_instance(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    let refs: Vec<String> = relation_service::load_relations(store)?
        .into_iter()
        .filter(|r| r.source_instance_id == instance_id || r.target_instance_id == instance_id)
        .map(|r| r.relation_id)
        .collect();
    Ok(refs)
}

/// Delete a Tier 2 record by its instance ID.
/// Returns `CannotDeleteInUse` if any Relation references this record as source or target.
pub fn delete_record(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<String, RepositoryError> {
    let refs = find_relations_referencing_instance(store, instance_id)?;
    if !refs.is_empty() {
        return Err(RepositoryError::CannotDeleteInUse {
            entity_type: "record".to_string(),
            id: instance_id.to_string(),
            used_by: refs,
        });
    }

    let mut manifest = store.load_manifest()?;

    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == instance_id && e.tier() == 2)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let path = manifest.instance_index[entry_index].path().to_string();

    store.delete_instance_file(&path)?;
    // Best-effort: delete the revision sidecar co-located with this record.
    let _ = revision_service::delete_sidecar(store, &path);
    manifest.instance_index.remove(entry_index);
    write_manifest(store, &manifest)?;

    Ok(instance_id.to_string())
}

/// Filter options for listing records
#[derive(Debug, Clone, Default)]
pub struct RecordListFilter {
    pub type_namespace: Option<String>,
    pub type_name: Option<String>,
    /// If Some, only return records that are members of this container.
    pub container_id: Option<String>,
}

/// Input for creating or updating a record.
///
/// When used for updates via `record update`, `group_values` semantics:
/// - Field absent or `null` in JSON → field-value `None` → existing group_values preserved.
/// - `[]` (empty array) → `Some(vec![])` → group_values replaced with empty (effectively cleared).
/// - `[{...}]` (non-empty array) → `Some(vec![...])` → group_values replaced with new entries.
///
/// There is no JSON representation to distinguish "null" from "absent"; both map to `None` (preserve).
/// To clear all group_values, send `"groupValues": []`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordInput {
    pub field_values: Vec<FieldValue>,
    #[serde(default)]
    pub group_values: Option<Vec<srs_core::types::record::FieldGroupValue>>,
}

/// Result for create_record_in_context
#[derive(Debug, Clone)]
pub struct CreateRecordResult {
    pub record: Record,
}

/// Result for delete_record_in_context
#[derive(Debug, Clone)]
pub struct DeleteRecordResult {
    pub instance_id: String,
}

/// List records using a unified filter (type and/or container).
pub fn list_records_filtered(
    store: &dyn RepositoryStore,
    filter: RecordListFilter,
) -> Result<Vec<Record>, RepositoryError> {
    // Resolve container members once
    let member_ids: Option<std::collections::HashSet<String>> =
        if let Some(ref cid) = filter.container_id {
            let members = container_service::list_members(store, cid)?;
            Some(members.into_iter().collect())
        } else {
            None
        };

    let manifest = store.load_manifest()?;
    let mut records = Vec::new();

    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }

        // Container membership filter
        if let Some(ref member_set) = member_ids {
            if !member_set.contains(entry.instance_id()) {
                continue;
            }
        }

        let record = load_record(store, entry.path())?;

        // Type namespace/name filter
        if let Some(ref ns) = filter.type_namespace {
            if &record.type_namespace != ns {
                continue;
            }
        }
        if let Some(ref name) = filter.type_name {
            if &record.type_name != name {
                continue;
            }
        }

        records.push(record);
    }

    Ok(records)
}

/// Create a record from a `namespace/name` type filter and optionally add to a container.
///
/// - Parses `type_filter` as `namespace/name`
/// - Resolves the type (with optional version pin)
/// - Creates the record
/// - If `container_id` is Some, validates the container exists and adds the record
pub fn create_record_in_context(
    store: &dyn RepositoryStore,
    type_filter: &str,
    type_version: Option<u32>,
    input: CreateRecordInput,
    container_id: Option<String>,
    relative_dir: &str,
) -> Result<CreateRecordResult, RepositoryError> {
    // Parse namespace/name
    let parts: Vec<&str> = type_filter.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "Invalid type filter '{}'. Expected format: namespace/name",
                type_filter
            ),
        });
    }
    let namespace = parts[0];
    let name = parts[1];

    // Validate container exists before writing anything
    if let Some(ref cid) = container_id {
        container_service::get_container(store, cid)?;
    }

    // Resolve type
    let record_type = if let Some(version) = type_version {
        let package = store.load_package()?;
        package
            .record_types
            .iter()
            .find(|rt| rt.namespace == namespace && rt.name == name && rt.version == version)
            .cloned()
            .ok_or_else(|| RepositoryError::TypeNotFound {
                type_id: format!("{}/{}", namespace, name),
                version,
            })?
    } else {
        match get_type_by_name(store, namespace, name)? {
            GetTypeResult::Found(rt) => rt,
            GetTypeResult::NotFound => {
                return Err(RepositoryError::TypeNotFound {
                    type_id: format!("{}/{}", namespace, name),
                    version: 0,
                })
            }
        }
    };

    let record = create_record(
        store,
        &record_type.id,
        record_type.version,
        input.field_values,
        input.group_values,
        relative_dir,
    )?;

    if let Some(ref cid) = container_id {
        container_service::add_member(store, cid, &record.instance_id)?;
    }

    Ok(CreateRecordResult { record })
}

/// Delete a record with optional container-scoped membership check.
///
/// If `container_id` is Some, the record must be a member of that container;
/// membership is removed before the record is deleted.
pub fn delete_record_in_context(
    store: &dyn RepositoryStore,
    id: String,
    container_id: Option<String>,
) -> Result<DeleteRecordResult, RepositoryError> {
    if let Some(ref cid) = container_id {
        if !container_service::is_member(store, cid, &id)? {
            return Err(RepositoryError::NotFound {
                path: std::path::PathBuf::from(format!(
                    "Instance '{}' is not a member of container '{}'",
                    id, cid
                )),
            });
        }
        container_service::remove_member(store, cid, &id)?;
    }

    let instance_id = delete_record(store, &id)?;
    Ok(DeleteRecordResult { instance_id })
}

/// Input for transitioning a record's lifecycle state.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionLifecycleInput {
    /// Target state name (use either `to` or `by_transition`, not both).
    pub to: Option<String>,
    /// Named transition (e.g., "promote") — resolved to its `to` state.
    pub by_transition: Option<String>,
}

/// Result for transition_record_lifecycle — includes warnings for final-state transitions
/// and any diagnostics from the best-effort revision append step.
#[derive(Debug, Clone)]
pub struct TransitionLifecycleResult {
    pub record: Record,
    pub warnings: Vec<String>,
}

/// Input for creating a successor record.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordSuccessorInput {
    /// "supersedes" or "refines"
    pub relation_type: String,
    pub field_values: Vec<FieldValue>,
    /// Optional initial lifecycle state for the successor (defaults to Type.initialState).
    pub lifecycle_state: Option<String>,
    /// Optional type version override (defaults to same as predecessor).
    pub type_version: Option<u32>,
}

/// Result for create_record_successor.
#[derive(Debug, Clone)]
pub struct CreateRecordSuccessorResult {
    pub record: Record,
    pub relation: Relation,
}

/// Transition a record's lifecycle state.
///
/// Validates that the transition exists in the Type's lifecycle.transitions[].
/// If the target state has isFinal: true, the transition succeeds but a warning is returned.
pub fn transition_record_lifecycle(
    store: &dyn RepositoryStore,
    instance_id: &str,
    input: TransitionLifecycleInput,
) -> Result<TransitionLifecycleResult, RepositoryError> {
    let record =
        get_record_by_id(store, instance_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let package = store.load_package()?;
    let record_type = package
        .resolve_type(&record.type_id, record.type_version)
        .ok_or_else(|| RepositoryError::TypeNotFound {
            type_id: record.type_id.clone(),
            version: record.type_version,
        })?;

    let lifecycle =
        record_type
            .lifecycle
            .as_ref()
            .ok_or_else(|| RepositoryError::LifecycleNotDefined {
                id: instance_id.to_string(),
            })?;

    // Resolve target state name from either `to` or `by_transition`
    let target_state = match (&input.to, &input.by_transition) {
        (Some(to), None) => to.clone(),
        (None, Some(transition_name)) => lifecycle
            .transitions
            .iter()
            .find(|t| &t.name == transition_name)
            .map(|t| t.to.clone())
            .ok_or_else(|| RepositoryError::LifecycleTransitionNotAllowed {
                from: record.lifecycle_state.clone().unwrap_or_default(),
                to: transition_name.clone(),
            })?,
        _ => {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: "exactly one of 'to' or 'byTransition' must be provided".to_string(),
            })
        }
    };

    // Validate target state exists in lifecycle
    if !lifecycle.states.iter().any(|s| s.key == target_state) {
        return Err(RepositoryError::LifecycleStateNotDefined {
            state: target_state,
        });
    }

    // Validate a transition path from current → target exists
    let current_state = record.lifecycle_state.clone().unwrap_or_default();
    let transition_allowed = lifecycle
        .transitions
        .iter()
        .any(|t| t.from == current_state && t.to == target_state);
    if !transition_allowed {
        return Err(RepositoryError::LifecycleTransitionNotAllowed {
            from: current_state,
            to: target_state.clone(),
        });
    }

    // Check if target state is final → emit warning
    let mut warnings = Vec::new();
    if let Some(state_def) = lifecycle.states.iter().find(|s| s.key == target_state) {
        if state_def.is_final == Some(true) {
            warnings.push(format!(
                "LIFECYCLE_FINAL_STATE: Target state '{}' is a final state — no further transitions are expected",
                target_state
            ));
        }
    }

    // Build updated record
    let manifest = store.load_manifest()?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == instance_id)
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let updated = Record {
        lifecycle_state: Some(target_state),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        ..record
    };

    write_record(store, &updated, entry.path())?;

    // Best-effort: append one Revision per field value, tagged with the lifecycle transition.
    // Transition is already committed at this point — if append fails we emit a diagnostic
    // rather than returning an error (the file store has no cross-entity transactions).
    let now = updated
        .updated_at
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let provenance = RevisionProvenance {
        lifecycle_transition: Some(updated.lifecycle_state.clone().unwrap_or_default()),
        transitioned_at: Some(now.clone()),
        import_source: None,
    };
    for field_value in &updated.field_values {
        let prior_revision_id = find_latest_revision_id(
            store,
            entry.path(),
            &updated.instance_id,
            &field_value.field_id,
        );
        let revision = Revision {
            revision_id: new_instance_id(),
            record_id: updated.instance_id.clone(),
            field_id: field_value.field_id.clone(),
            value: field_value.value.clone(),
            prior_revision_id,
            agent: RevisionAgent::Ai,
            provenance: Some(provenance.clone()),
            created_at: now.clone(),
        };
        if let Err(_e) = revision_service::append(store, entry.path(), revision) {
            warnings.push(format!(
                "REVISION_APPEND_FAILED: could not append revision for field '{}'",
                field_value.field_id
            ));
        }
    }

    Ok(TransitionLifecycleResult {
        record: updated,
        warnings,
    })
}

/// Find the most recent revision_id for a (record, field) pair, if any.
fn find_latest_revision_id(
    store: &dyn RepositoryStore,
    record_path: &str,
    record_id: &str,
    field_id: &str,
) -> Option<String> {
    revision_service::list(store, record_path, record_id, Some(field_id), None, None)
        .ok()
        .and_then(|revs| revs.into_iter().last().map(|r| r.revision_id))
}

/// Create a successor record (supersedes or refines an existing record).
///
/// Creates a new Record with the same typeId+typeVersion (or a specified version),
/// then automatically adds a Relation from the successor to the predecessor.
pub fn create_record_successor(
    store: &dyn RepositoryStore,
    predecessor_id: &str,
    input: CreateRecordSuccessorInput,
    relative_dir: &str,
) -> Result<CreateRecordSuccessorResult, RepositoryError> {
    let predecessor =
        get_record_by_id(store, predecessor_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let type_version = input.type_version.unwrap_or(predecessor.type_version);

    // Validate the requested type version exists before writing anything.
    {
        let package = store.load_package()?;
        package
            .resolve_type(&predecessor.type_id, type_version)
            .ok_or_else(|| RepositoryError::TypeVersionNotFound {
                type_id: predecessor.type_id.clone(),
                version: type_version,
            })?;
    }

    // Create the successor record (lifecycle_state auto-set from Type.initialState).
    let mut successor = create_record(
        store,
        &predecessor.type_id,
        type_version,
        input.field_values,
        None,
        relative_dir,
    )?;

    // If caller supplied an explicit lifecycle_state, patch it.
    if let Some(explicit_state) = input.lifecycle_state {
        if successor.lifecycle_state.as_deref() != Some(explicit_state.as_str()) {
            let manifest = store.load_manifest()?;
            let entry = manifest
                .instance_index
                .iter()
                .find(|e| e.instance_id() == successor.instance_id)
                .cloned()
                .ok_or_else(|| RepositoryError::NotFound {
                    path: std::path::PathBuf::from("records"),
                })?;
            successor.lifecycle_state = Some(explicit_state);
            write_record(store, &successor, entry.path())?;
        }
    }

    // Create the relation: successor → predecessor
    let rel_result = relation_service::create_relation_auto(
        store,
        Relation {
            relation_id: String::new(),
            relation_type: input.relation_type,
            source_instance_id: successor.instance_id.clone(),
            target_instance_id: predecessor_id.to_string(),
            asserted_by: None,
            confidence: None,
            created_at: Some(chrono::Utc::now().to_rfc3339()),
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        },
    )?;

    Ok(CreateRecordSuccessorResult {
        record: successor,
        relation: rel_result.relation,
    })
}

/// List revisions for a record, optionally filtered by field_id.
///
/// Returns revisions in append order (oldest first).
pub fn list_record_revisions(
    store: &dyn RepositoryStore,
    instance_id: &str,
    field_id: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<Revision>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == instance_id && e.tier() == 2)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;
    revision_service::list(store, entry.path(), instance_id, field_id, limit, offset)
}

/// Get a single revision by its revision_id, scoped to a specific record.
pub fn get_record_revision(
    store: &dyn RepositoryStore,
    instance_id: &str,
    revision_id: &str,
) -> Result<Option<Revision>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == instance_id && e.tier() == 2)
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;
    revision_service::get(store, entry.path(), instance_id, revision_id)
}

/// Add or replace the manifest index entry for a Record (in memory only).
fn upsert_record_index_entry(manifest: &mut Manifest, record: &Record, relative_path: &str) {
    let entry = InstanceIndexEntry {
        instance_id: record.instance_id.clone(),
        tier: 2,
        path: relative_path.to_string(),
        title: None,
        tags: None,
    };

    if let Some(pos) = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == record.instance_id)
    {
        manifest.instance_index[pos] = entry;
    } else {
        manifest.instance_index.push(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use std::path::PathBuf;

    fn srs_spec_repo() -> PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut dir = manifest.to_path_buf();
        loop {
            let candidate = dir.join("../srs/srs");
            if let Ok(c) = candidate.canonicalize() {
                if c.join(".srs").exists() {
                    return c;
                }
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        manifest.join("../../../srs/srs")
    }

    fn make_store_with_package() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let name_field = Field {
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "Name field".to_string(),
            ai_guidance: json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let status_field = Field {
            id: "field-status-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-status".to_string(),
            version: 1,
            value_type: ValueType::Select,
            description: "Status field".to_string(),
            ai_guidance: json!(null),
            allowed_values: Some(vec!["active".to_string(), "inactive".to_string()]),
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let test_type = RecordType {
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "field-name-001".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Name".to_string()),
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
                FieldAssignment {
                    field_id: "field-status-001".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Status".to_string()),
                    repeatable: false,
                    min_items: None,
                    max_items: None,
                },
            ],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };
        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field, status_field],
            record_types: vec![test_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    // These tests mirror the existing tests that use TempDir — they still call
    // list_records_by_type / get_record_by_id against the live srs repo (read-only),
    // which is fine since they don't write.

    #[test]
    fn list_records_by_type_from_live_repo() {
        use crate::FileStore;
        let srs_repo = srs_spec_repo();
        if !srs_repo.exists() {
            println!("Skipping test: live repo not found");
            return;
        }
        let store = FileStore::new(&srs_repo);
        match list_records_by_type(&store, "com.semanticops.srs", "meta.extension") {
            Ok(records) => {
                for record in &records {
                    assert_eq!(record.type_namespace, "com.semanticops.srs");
                    assert_eq!(record.type_name, "meta.extension");
                }
            }
            Err(_) => println!("Skipping: could not list records"),
        }
    }

    #[test]
    fn get_record_by_id_returns_known_record() {
        use crate::FileStore;
        let srs_repo = srs_spec_repo();
        if !srs_repo.exists() {
            println!("Skipping test: live repo not found");
            return;
        }
        let store = FileStore::new(&srs_repo);
        let records = match list_records_by_type(&store, "com.semanticops.srs", "meta.extension") {
            Ok(r) => r,
            Err(_) => {
                println!("Skipping: could not list records");
                return;
            }
        };
        if records.is_empty() {
            println!("Skipping: no extension records");
            return;
        }
        let first_id = records[0].instance_id.clone();
        let retrieved = get_record_by_id(&store, &first_id).expect("should get record");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().instance_id, first_id);
    }

    #[test]
    fn get_record_by_id_returns_none_for_unknown() {
        use crate::FileStore;
        let srs_repo = srs_spec_repo();
        let store = FileStore::new(&srs_repo);
        let result = get_record_by_id(&store, "00000000-0000-0000-0000-000000000000")
            .expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn create_record_in_temp_repo() {
        let store = make_store_with_package();
        let field_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Test Record"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("active"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            None,
            "records/test-items",
        )
        .expect("should create record");

        assert!(!record.instance_id.is_empty());
        assert_eq!(record.type_id, "type-test-001");

        // Record stored in memory
        let key = format!("records/test-items/{}.json", record.instance_id);
        store
            .load_instance_json(&key)
            .expect("should find stored record");

        // Manifest updated
        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == record.instance_id);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().tier(), 2);
    }

    #[test]
    fn create_record_missing_required_field_fails() {
        let store = make_store_with_package();
        let field_values = vec![FieldValue {
            field_id: "field-status-001".to_string(),
            value: json!("active"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let result = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            None,
            "records/test-items",
        );
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RepositoryError::RecordValidation { .. }
        ));
    }

    #[test]
    fn create_record_optional_field_absent_succeeds() {
        let store = make_store_with_package();
        let field_values = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Test Record"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            None,
            "records/test-items",
        )
        .expect("should create with only required field");
        assert_eq!(record.field_values.len(), 1);
    }

    #[test]
    fn record_update_validates_against_type() {
        let store = make_store_with_package();
        let initial_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Initial Name"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("active"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            initial_values,
            None,
            "records/test-items",
        )
        .unwrap();
        let instance_id = record.instance_id.clone();

        let updated_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Updated Name"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("inactive"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let updated = update_record(&store, &instance_id, updated_values, None).unwrap();
        assert_eq!(updated.field_values[0].value, json!("Updated Name"));

        // Verify stored value
        let key = format!("records/test-items/{}.json", instance_id);
        let stored_val = store.load_instance_json(&key).unwrap();
        let stored: Record = serde_json::from_value(stored_val).unwrap();
        assert_eq!(stored.field_values[0].value, json!("Updated Name"));

        // Invalid update (missing required field)
        let invalid_values = vec![FieldValue {
            field_id: "field-status-001".to_string(),
            value: json!("active"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        assert!(update_record(&store, &instance_id, invalid_values, None).is_err());
    }

    #[test]
    fn record_delete_blocked_when_relation_references_it() {
        use crate::relation_service::load_relations;

        let store = make_store_with_package();

        let record_a = create_record(
            &store,
            "type-test-001",
            1,
            vec![FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Record A"),
                entries: None,
                source: None,
                edited_at: None,
            }],
            None,
            "records/test-items",
        )
        .unwrap();

        let record_b = create_record(
            &store,
            "type-test-001",
            1,
            vec![FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Record B"),
                entries: None,
                source: None,
                edited_at: None,
            }],
            None,
            "records/test-items",
        )
        .unwrap();

        // Write a relation directly to the store, bypassing type-definition validation
        // (the guard only checks existence, not type validity).
        let rel_json = json!({
            "relations": [{
                "relationId": "rel-test-001",
                "relationType": "depends-on",
                "sourceInstanceId": record_a.instance_id,
                "targetInstanceId": record_b.instance_id
            }]
        });
        store
            .save_relations_json("relations/relations-collection.json", &rel_json)
            .unwrap();

        // Deleting record_b (the target) should be blocked
        let result = delete_record(&store, &record_b.instance_id);
        match result {
            Err(RepositoryError::CannotDeleteInUse {
                entity_type,
                id,
                used_by,
            }) => {
                assert_eq!(entity_type, "record");
                assert_eq!(id, record_b.instance_id);
                assert!(used_by.contains(&"rel-test-001".to_string()));
            }
            other => panic!("expected CannotDeleteInUse, got {:?}", other),
        }

        // Relation still exists — nothing was deleted
        let remaining = load_relations(&store).unwrap();
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn record_delete_succeeds_when_no_relations_reference_it() {
        let store = make_store_with_package();

        let record = create_record(
            &store,
            "type-test-001",
            1,
            vec![FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Isolated Record"),
                entries: None,
                source: None,
                edited_at: None,
            }],
            None,
            "records/test-items",
        )
        .unwrap();

        delete_record(&store, &record.instance_id).unwrap();
    }

    #[test]
    fn record_delete_removes_file_and_manifest_entry() {
        let store = make_store_with_package();
        let field_values = vec![
            FieldValue {
                field_id: "field-name-001".to_string(),
                value: json!("Test Name"),
                entries: None,
                source: None,
                edited_at: None,
            },
            FieldValue {
                field_id: "field-status-001".to_string(),
                value: json!("active"),
                entries: None,
                source: None,
                edited_at: None,
            },
        ];

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            None,
            "records/test-items",
        )
        .unwrap();
        let instance_id = record.instance_id.clone();
        let key = format!("records/test-items/{}.json", instance_id);

        assert!(store.load_instance_json(&key).is_ok());

        let deleted_id = delete_record(&store, &instance_id).unwrap();
        assert_eq!(deleted_id, instance_id);

        assert!(store.load_instance_json(&key).is_err());

        let manifest = store.load_manifest().unwrap();
        assert!(manifest
            .instance_index
            .iter()
            .all(|e| e.instance_id() != instance_id));
    }

    fn make_store_with_lifecycle() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{Field, ValueType};
        use srs_core::types::record_type::{
            FieldAssignment, LifecycleState, LifecycleTransition, RecordType, TypeLifecycle,
        };
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        let title_field = Field {
            id: "field-title-lc".to_string(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            value_type: ValueType::String,
            description: "Title".to_string(),
            ai_guidance: json!(null),
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let lc_type = RecordType {
            id: "type-lc-001".to_string(),
            namespace: "com.test".to_string(),
            name: "lifecycle-type".to_string(),
            version: 1,
            description: "Type with lifecycle".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-title-lc".to_string(),
                order: 0,
                required: true,
                display_label: None,
                repeatable: false,
                min_items: None,
                max_items: None,
            }],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: Some(TypeLifecycle {
                states: vec![
                    LifecycleState {
                        id: None,
                        version: None,
                        namespace: None,
                        key: "draft".to_string(),
                        label: None,
                        description: None,
                        aliases: None,
                        is_initial: Some(true),
                        is_final: None,
                        status: None,
                        properties: None,
                    },
                    LifecycleState {
                        id: None,
                        version: None,
                        namespace: None,
                        key: "active".to_string(),
                        label: None,
                        description: None,
                        aliases: None,
                        is_initial: None,
                        is_final: None,
                        status: None,
                        properties: None,
                    },
                    LifecycleState {
                        id: None,
                        version: None,
                        namespace: None,
                        key: "archived".to_string(),
                        label: None,
                        description: None,
                        aliases: None,
                        is_initial: None,
                        is_final: Some(true),
                        status: None,
                        properties: None,
                    },
                ],
                transitions: vec![
                    LifecycleTransition {
                        id: None,
                        name: "promote".to_string(),
                        from: "draft".to_string(),
                        to: "active".to_string(),
                        description: None,
                        properties: None,
                    },
                    LifecycleTransition {
                        id: None,
                        name: "archive".to_string(),
                        from: "active".to_string(),
                        to: "archived".to_string(),
                        description: None,
                        properties: None,
                    },
                ],
                initial_state: "draft".to_string(),
            }),
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let supersedes_def = RelationTypeDefinition {
            schema: None,
            id: "rtd-supersedes-001".to_string(),
            version: 1,
            key: "supersedes".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            label: "Supersedes".to_string(),
            description: "The source record supersedes the target.".to_string(),
            category: RelationTypeCategory::Refinement,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: Some(true),
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: None,
            updated_at: None,
            properties: None,
        };

        let refines_def = RelationTypeDefinition {
            schema: None,
            id: "rtd-refines-001".to_string(),
            version: 1,
            key: "refines".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            label: "Refines".to_string(),
            description: "The source record refines the target.".to_string(),
            category: RelationTypeCategory::Refinement,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: Some(true),
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: None,
            updated_at: None,
            properties: None,
        };

        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-lc".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package-lc".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![title_field],
            record_types: vec![lc_type],
            relation_type_definitions: vec![supersedes_def, refines_def],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(manifest, package)
    }

    fn create_lc_record(store: &MemoryStore) -> Record {
        create_record(
            store,
            "type-lc-001",
            1,
            vec![FieldValue {
                field_id: "field-title-lc".to_string(),
                value: json!("Test Item"),
                entries: None,
                source: None,
                edited_at: None,
            }],
            None,
            "records/lc-items",
        )
        .unwrap()
    }

    #[test]
    fn create_record_sets_initial_lifecycle_state() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        assert_eq!(record.lifecycle_state.as_deref(), Some("draft"));
    }

    #[test]
    fn transition_by_state_name_succeeds() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("active".to_string()),
                by_transition: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("active"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn transition_by_named_transition_succeeds() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: None,
                by_transition: Some("promote".to_string()),
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("active"));
    }

    #[test]
    fn transition_to_final_state_emits_warning() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        // Promote to active first
        transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("active".to_string()),
                by_transition: None,
            },
        )
        .unwrap();
        // Then archive (final state)
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("archived".to_string()),
                by_transition: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("archived"));
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("LIFECYCLE_FINAL_STATE"));
    }

    #[test]
    fn transition_not_in_transitions_list_fails() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        // Attempt draft → archived (no such transition defined)
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("archived".to_string()),
                by_transition: None,
            },
        );
        assert!(matches!(
            result,
            Err(RepositoryError::LifecycleTransitionNotAllowed { .. })
        ));
    }

    #[test]
    fn create_record_successor_supersedes() {
        let store = make_store_with_lifecycle();
        let predecessor = create_lc_record(&store);

        let result = create_record_successor(
            &store,
            &predecessor.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: vec![FieldValue {
                    field_id: "field-title-lc".to_string(),
                    value: json!("Updated Item"),
                    entries: None,
                    source: None,
                    edited_at: None,
                }],
                lifecycle_state: None,
                type_version: None,
            },
            "records/lc-items",
        )
        .unwrap();

        // Successor has initial lifecycle state
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("draft"));
        // Relation points from successor to predecessor
        assert_eq!(result.relation.relation_type, "supersedes");
        assert_eq!(
            result.relation.source_instance_id,
            result.record.instance_id
        );
        assert_eq!(result.relation.target_instance_id, predecessor.instance_id);
    }

    #[test]
    fn full_lifecycle_create_transition_successor() {
        let store = make_store_with_lifecycle();

        // Create in draft
        let original = create_lc_record(&store);
        assert_eq!(original.lifecycle_state.as_deref(), Some("draft"));

        // Transition to active
        let promoted = transition_record_lifecycle(
            &store,
            &original.instance_id,
            TransitionLifecycleInput {
                to: Some("active".to_string()),
                by_transition: None,
            },
        )
        .unwrap();
        assert_eq!(promoted.record.lifecycle_state.as_deref(), Some("active"));

        // Create a superseding successor
        let result = create_record_successor(
            &store,
            &original.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: vec![FieldValue {
                    field_id: "field-title-lc".to_string(),
                    value: json!("Next Version"),
                    entries: None,
                    source: None,
                    edited_at: None,
                }],
                lifecycle_state: None,
                type_version: None,
            },
            "records/lc-items",
        )
        .unwrap();

        // Successor is in draft, original still active
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("draft"));
        let original_now = get_record_by_id(&store, &original.instance_id)
            .unwrap()
            .unwrap();
        assert_eq!(original_now.lifecycle_state.as_deref(), Some("active"));

        // Verify relation
        assert_eq!(result.relation.relation_type, "supersedes");
        assert_eq!(
            result.relation.source_instance_id,
            result.record.instance_id
        );
        assert_eq!(result.relation.target_instance_id, original.instance_id);
    }

    // group_values write path tests (Phase 1D)

    #[test]
    fn create_record_with_group_values_persists_entries() {
        use srs_core::types::record::{FieldGroupEntry, FieldGroupValue, FieldValueEntry};

        let store = make_store_with_package();

        let field_values = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Grouped Record"),
            entries: None,
            source: None,
            edited_at: None,
        }];

        let group_values = Some(vec![FieldGroupValue {
            group_id: "rows".to_string(),
            entries: vec![
                FieldGroupEntry {
                    entry_id: None,
                    field_values: vec![FieldValue {
                        field_id: "field-name-001".to_string(),
                        value: json!("Row 1"),
                        entries: Some(vec![FieldValueEntry {
                            value: serde_json::json!("Row 1"),
                            source: None,
                            edited_at: None,
                        }]),
                        source: None,
                        edited_at: None,
                    }],
                },
                FieldGroupEntry {
                    entry_id: None,
                    field_values: vec![FieldValue {
                        field_id: "field-name-001".to_string(),
                        value: json!("Row 2"),
                        entries: None,
                        source: None,
                        edited_at: None,
                    }],
                },
            ],
        }]);

        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            group_values,
            "records/test-items",
        )
        .expect("should create record with group_values");

        let loaded = get_record_by_id(&store, &record.instance_id)
            .unwrap()
            .expect("should load record");

        let gv = loaded
            .group_values
            .expect("group_values should be persisted");
        assert_eq!(gv.len(), 1);
        assert_eq!(gv[0].group_id, "rows");
        assert_eq!(gv[0].entries.len(), 2);
    }

    #[test]
    fn update_record_with_group_values_replaces_entries() {
        use srs_core::types::record::{FieldGroupEntry, FieldGroupValue};

        let store = make_store_with_package();

        let fv = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Initial"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        let record = create_record(&store, "type-test-001", 1, fv, None, "records/test-items")
            .expect("create");
        let id = record.instance_id.clone();

        let new_fv = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Updated"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        let new_gv = Some(vec![FieldGroupValue {
            group_id: "rows".to_string(),
            entries: vec![FieldGroupEntry {
                entry_id: None,
                field_values: vec![],
            }],
        }]);
        update_record(&store, &id, new_fv, Some(new_gv)).expect("update");

        let loaded = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(loaded.field_values[0].value, json!("Updated"));
        let gv = loaded
            .group_values
            .expect("group_values should exist after update");
        assert_eq!(gv[0].group_id, "rows");
    }

    #[test]
    fn update_record_without_group_values_preserves_existing() {
        use srs_core::types::record::{FieldGroupEntry, FieldGroupValue};

        let store = make_store_with_package();

        let fv = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("With Groups"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        let gv = Some(vec![FieldGroupValue {
            group_id: "rows".to_string(),
            entries: vec![FieldGroupEntry {
                entry_id: None,
                field_values: vec![],
            }],
        }]);
        let record = create_record(&store, "type-test-001", 1, fv, gv, "records/test-items")
            .expect("create");
        let id = record.instance_id.clone();

        // None outer = not supplied, preserve existing
        let new_fv = vec![FieldValue {
            field_id: "field-name-001".to_string(),
            value: json!("Field Only Update"),
            entries: None,
            source: None,
            edited_at: None,
        }];
        update_record(&store, &id, new_fv, None).expect("update");

        let loaded = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(loaded.field_values[0].value, json!("Field Only Update"));
        assert!(
            loaded.group_values.is_some(),
            "group_values preserved when not supplied"
        );
    }
}
