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
use crate::index::InstanceQuery;
use crate::package_service::{get_type_by_name, GetTypeResult};
use crate::record_label;
use crate::relation_service;
use crate::revision_service;
use crate::store::{RecordTier, RepositoryStore};
use crate::writer::{new_instance_id, slugify_instance_name};
use serde::{Deserialize, Serialize};
use srs_core::types::lifecycle::{RelationDirection, RequiresRelation};
use srs_core::types::record::{FieldValues, Record};
use srs_core::types::relation::Relation;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::revision::Revision;
use srs_core::types::source_reference::SourceReference;
use srs_core::validation::lifecycle::validate_type_lifecycle_v9;
use srs_core::validation::record::{validate_record, validate_record_all, validate_type_lifecycle};
use srs_core::validation::record_type::{cross_field_type_map, validate_cross_field_rules};
use srs_core::validation::relation::validate_relation_type_for_write;
use srs_schema::RECORD_SCHEMA_ID;
use std::collections::BTreeMap;

/// List all Tier 2 records in the repository, regardless of type.
pub fn list_all_records(store: &dyn RepositoryStore) -> Result<Vec<Record>, RepositoryError> {
    let refs = store.list_instances(&InstanceQuery {
        tier: Some(2),
        tag: None,
    })?;
    let mut records = Vec::with_capacity(refs.len());
    for r in refs {
        records.push(store.load_record_by_id(&r.instance_id)?);
    }

    Ok(records)
}

/// List all Tier 2 records matching the given type namespace and name.
pub fn list_records_by_type(
    store: &dyn RepositoryStore,
    type_namespace: &str,
    type_name: &str,
) -> Result<Vec<Record>, RepositoryError> {
    let refs = store.list_instances(&InstanceQuery {
        tier: Some(2),
        tag: None,
    })?;
    let mut records = Vec::new();

    for r in refs {
        let record = store.load_record_by_id(&r.instance_id)?;
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
    if store.find_instance(id)?.is_some() {
        Ok(Some(store.load_record_by_id(id)?))
    } else {
        Ok(None)
    }
}

/// An instance loaded by tier. Tier-0 notes are legal container roots/members
/// (RFC-013 models a Tier-0 identity note that can be graduated later), so code
/// that resolves arbitrary container members must be prepared for both shapes.
#[derive(Debug, Clone)]
pub enum LoadedInstance {
    /// Tier-2 (and legacy tier-1) instance loaded as a typed Record.
    Record(Record),
    /// Tier-0 instance loaded as a Note.
    Note(srs_core::types::note::Note),
}

impl LoadedInstance {
    pub fn instance_id(&self) -> &str {
        match self {
            LoadedInstance::Record(r) => &r.instance_id,
            LoadedInstance::Note(n) => &n.instance_id,
        }
    }

    pub fn created_at(&self) -> Option<&str> {
        match self {
            LoadedInstance::Record(r) => r.created_at.as_deref(),
            LoadedInstance::Note(n) => n.created_at.as_deref(),
        }
    }

    pub fn as_record(&self) -> Option<&Record> {
        match self {
            LoadedInstance::Record(r) => Some(r),
            LoadedInstance::Note(_) => None,
        }
    }

    /// Field value lookup by `Field.name` (the RFC-039 carrier key). Notes
    /// have no field values.
    pub fn get_field_value_str(&self, name: &str) -> Option<&str> {
        match self {
            LoadedInstance::Record(r) => r.value_str(name),
            LoadedInstance::Note(_) => None,
        }
    }
}

/// Get an instance by ID, dispatching on its manifest tier: Tier-0 entries load
/// through the Note shape, everything else loads as a Record. Use this instead of
/// [`get_record_by_id`] wherever an ID may legally reference a note (e.g. container
/// members and roots).
pub fn get_instance_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<LoadedInstance>, RepositoryError> {
    match store.find_instance(id)? {
        Some(r) if r.tier == 0 => Ok(Some(LoadedInstance::Note(store.load_note_by_id(id)?))),
        Some(_) => Ok(Some(LoadedInstance::Record(store.load_record_by_id(id)?))),
        None => Ok(None),
    }
}

/// RFC-039 [R18]: instance `fieldValues` keys are serialised in
/// `FieldAssignment.order`. Write paths normalise the authored map here so
/// storage order is a property of the Type, not of the input path (CLI stdin,
/// MCP's sorted map, WASM) — unknown keys (caught by validation) keep their
/// input order at the tail.
fn normalize_field_value_order(
    field_values: FieldValues,
    effective_fields: &[srs_core::validation::value_shape::EffectiveField],
) -> FieldValues {
    let mut source = field_values.0;
    let mut ordered = serde_json::Map::new();
    for ef in effective_fields {
        if let Some(v) = source.shift_remove(&ef.name) {
            ordered.insert(ef.name.clone(), v);
        }
    }
    for (k, v) in source {
        ordered.insert(k, v);
    }
    FieldValues(ordered)
}

/// Create a new Tier 2 record in the default directory (`records/tier-2`).
pub fn create_record(
    store: &dyn RepositoryStore,
    type_id: &str,
    type_version: u32,
    field_values: FieldValues,
    field_meta: Option<indexmap::IndexMap<String, srs_core::types::record::FieldMeta>>,
    tags: Option<Vec<String>>,
) -> Result<Record, RepositoryError> {
    create_record_at_dir(
        store,
        type_id,
        type_version,
        field_values,
        field_meta,
        tags,
        store.record_tier_dir(RecordTier::Tier2),
    )
}

/// Create a new Tier 2 record in a caller-specified directory.
///
/// Use `create_record` for the common case. This function exists for callers (like
/// `create_record_in_context`) that need a non-default directory, and for internal
/// module tests that verify path behaviour.
pub(crate) fn create_record_at_dir(
    store: &dyn RepositoryStore,
    type_id: &str,
    type_version: u32,
    field_values: FieldValues,
    field_meta: Option<indexmap::IndexMap<String, srs_core::types::record::FieldMeta>>,
    tags: Option<Vec<String>>,
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

    let initial_lifecycle_state = package
        .effective_lifecycle(record_type)
        .map(|lc| lc.initial_state.to_string());

    // Normalise tags: treat Some([]) as None (no tags) to keep the record body clean.
    let initial_tags = match tags {
        Some(ref v) if !v.is_empty() => tags,
        _ => None,
    };

    let effective_fields = package.resolved_effective_fields(record_type)?;
    let field_values = normalize_field_value_order(field_values, &effective_fields);
    let mut record = Record {
        instance_id: String::new(),
        type_id: type_id.to_string(),
        type_version,
        type_namespace: record_type.namespace.clone(),
        type_name: record_type.name.clone(),
        field_values,
        field_meta,
        lifecycle_state: initial_lifecycle_state,
        tags: initial_tags,
        created_at: Some(chrono::Utc::now().to_rfc3339()),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extra: BTreeMap::new(),
    };

    validate_record(&record, record_type, &effective_fields, &package).map_err(|e| {
        RepositoryError::RecordValidation {
            path: std::path::PathBuf::from(relative_dir),
            source: e,
        }
    })?;

    // ext:cross-field-validation — enforce CrossFieldRules at write time.
    // Consistent with required-field enforcement above: first violation is a hard error.
    if let Some(rules) = &record_type.validation_rules {
        if !rules.is_empty() {
            let field_type_map = cross_field_type_map(&package.fields, record_type);
            if let Some(err) = validate_cross_field_rules(&record, rules, &field_type_map)
                .into_iter()
                .next()
            {
                return Err(RepositoryError::RecordValidation {
                    path: std::path::PathBuf::from(relative_dir),
                    source: err,
                });
            }
        }
    }

    record.instance_id = new_instance_id();

    if relative_dir == store.record_tier_dir(RecordTier::Tier2) {
        store.save_record(&record)?;
    } else {
        // Extension/legacy directory — not covered by the typed logical-id surface
        // (ADR-042 only defines Tier2/Note tiers). Membership comes from the tree
        // ([R1]): write the file only, never `manifest.json` ([R22]).
        store.ensure_instance_dir(relative_dir)?;

        let type_slug = slugify_instance_name(&record.type_name);
        let id8 = &record.instance_id[..8];
        let relative_path = if type_slug.is_empty() {
            format!("{relative_dir}/{id8}.json")
        } else {
            format!("{relative_dir}/{type_slug}-{id8}.json")
        };
        write_record(store, &record, &relative_path)?;
    }

    Ok(record)
}

/// Load a record from the store.
pub(crate) fn load_record(
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
    let mut value = serde_json::to_value(record).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(relative_path),
        source: e,
    })?;
    if let serde_json::Value::Object(ref mut obj) = value {
        obj.insert(
            "$schema".to_string(),
            serde_json::Value::String(RECORD_SCHEMA_ID.to_string()),
        );
    }
    store.save_instance_json(relative_path, &value)
}

/// Update an existing Tier 2 record.
///
/// When `input.type_version` is `None` the stored version is used; when `Some(v)`
/// the record is migrated to that version (including `type_name` and
/// `type_namespace`, which are re-derived from the package at the effective
/// version). This fixes the stale-typeVersion bug: validation always runs against
/// the **effective** version, never the stored one unconditionally.
pub fn update_record(
    store: &dyn RepositoryStore,
    instance_id: &str,
    input: UpdateRecordInput,
) -> Result<Record, RepositoryError> {
    let record =
        get_record_by_id(store, instance_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let effective_type_version = input.type_version.unwrap_or(record.type_version);

    let package = store.load_package()?;
    let record_type = package
        .resolve_type(&record.type_id, effective_type_version)
        .ok_or_else(|| RepositoryError::TypeVersionNotFound {
            type_id: record.type_id.clone(),
            version: effective_type_version,
        })?;

    // Three-way tag semantics:
    //   None        → preserve existing tags (caller did not supply the field)
    //   Some([])    → clear all tags
    //   Some([...]) → replace tags with the supplied list
    let updated_tags = match input.tags {
        None => record.tags,
        Some(ref v) if v.is_empty() => None,
        Some(v) => Some(v),
    };

    // fieldMeta: None = preserve stored value; Some(m) = replace/clear.
    let new_field_meta = match input.field_meta {
        Some(m) => Some(m),
        None => record.field_meta,
    };

    let effective_fields = package.resolved_effective_fields(record_type)?;
    let updated_record = Record {
        instance_id: record.instance_id,
        type_id: record.type_id,
        type_version: effective_type_version,
        type_namespace: record_type.namespace.clone(),
        type_name: record_type.name.clone(),
        field_values: normalize_field_value_order(input.field_values, &effective_fields),
        field_meta: new_field_meta,
        lifecycle_state: record.lifecycle_state,
        tags: updated_tags,
        created_at: record.created_at,
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        extra: record.extra,
    };

    validate_record(&updated_record, record_type, &effective_fields, &package).map_err(|e| {
        RepositoryError::RecordValidation {
            path: std::path::PathBuf::from("records"),
            source: e,
        }
    })?;

    // ext:cross-field-validation — enforce CrossFieldRules at write time.
    // Consistent with required-field enforcement above: first violation is a hard error.
    if let Some(rules) = &record_type.validation_rules {
        if !rules.is_empty() {
            let field_type_map = cross_field_type_map(&package.fields, record_type);
            if let Some(err) = validate_cross_field_rules(&updated_record, rules, &field_type_map)
                .into_iter()
                .next()
            {
                return Err(RepositoryError::RecordValidation {
                    path: std::path::PathBuf::from("records"),
                    source: err,
                });
            }
        }
    }

    store.save_record(&updated_record)?;

    Ok(updated_record)
}

/// Validate a prospective record input against its resolved `typeId@typeVersion`
/// **without persisting anything**. Performs only reads; never writes a record or
/// the manifest. Intended for editor preflight (validate a whole document before
/// the per-record save loop). Runs the same checks `create_record`/
/// `update_record` run before persist (via `validate_record_all`), so a passing
/// validate guarantees a passing write — but collects **all** diagnostics rather
/// than stopping at the first, so an editor can surface every problem at once.
pub fn validate_record_input(
    store: &dyn RepositoryStore,
    input: ValidateRecordInput,
) -> Result<RecordValidateReport, RepositoryError> {
    let package = store.load_package()?;
    let record_type = match package.resolve_type(&input.type_id, input.type_version) {
        Some(t) => t,
        None => {
            return Ok(RecordValidateReport {
                ok: false,
                errors: vec![format!(
                    "type not found: {}@{}",
                    input.type_id, input.type_version
                )],
            });
        }
    };

    let record = Record {
        instance_id: String::new(),
        type_id: input.type_id.clone(),
        type_version: input.type_version,
        type_namespace: record_type.namespace.clone(),
        type_name: record_type.name.clone(),
        field_values: input.field_values,
        field_meta: input.field_meta,
        lifecycle_state: record_type
            .lifecycle
            .as_ref()
            .map(|lc| lc.initial_state.clone()),
        tags: input.tags,
        created_at: None,
        updated_at: None,
        extra: BTreeMap::new(),
    };

    let effective_fields = package.resolved_effective_fields(record_type)?;
    // Collect *all* diagnostics so a multi-record editor can show every problem
    // in one pass, not one-fix-revalidate at a time (#111).
    let mut errors: Vec<String> =
        validate_record_all(&record, record_type, &effective_fields, &package)
            .iter()
            .map(|e| e.to_string())
            .collect();

    // ext:cross-field-validation — also collect CFR diagnostics for preflight consistency.
    // A passing preflight must guarantee a passing write.
    if let Some(rules) = &record_type.validation_rules {
        if !rules.is_empty() {
            let field_type_map = cross_field_type_map(&package.fields, record_type);
            let cfr_errors = validate_cross_field_rules(&record, rules, &field_type_map);
            errors.extend(cfr_errors.iter().map(|e| e.to_string()));
        }
    }

    Ok(RecordValidateReport {
        ok: errors.is_empty(),
        errors,
    })
}

/// Delete a Tier 2 record by its instance ID.
///
/// RFC-038 Change F: the incident references are removed before the record
/// itself, so the delete never leaves a dangling reference behind — if the
/// process is interrupted mid-way, the worst case is an orphaned-but-still-valid
/// record, never a dangling reference. Two kinds of incident reference:
///
/// - **Container membership** — an *explicit container-membership operation*
///   under Change F, so it may write `manifest.json` where [R22] forbids routine
///   writes from doing so (the root container is inline, [R1]). srs-rust#834.
/// - **Relations** — the [R22] scoped cascade; incident relation files are
///   declared targets of this delete. The batch seam is a no-op on FileStore
///   until #813.
///
/// The record's own tier and locator come from one catalog snapshot ([R24]).
/// `manifest.instance_index` is never written (srs-rust#783 Phase 3).
pub fn delete_record(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<String, RepositoryError> {
    let cat = store.catalog()?;
    let path = cat
        .instances
        .iter()
        .find(|e| e.id == instance_id && e.tier == Some(2))
        .and_then(|e| e.locator.clone())
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    // Change F: container membership is an *explicit container-membership
    // operation*, outside [R22]'s routine-write prohibition. Removed before the
    // instance: there is no transaction across these writes (the batch seam is
    // a no-op until #813), so the ordering is what guarantees the repository
    // stays loadable in every interleaving. The residue it can leave is a live
    // instance short its container references — including on an `Ok` return,
    // since the file delete below is best-effort. That is repairable with
    // `container members add` / `container roots add`, and `container update`
    // for an identity; the unloadable repository it replaces was not.
    container_service::remove_instance_from_all_containers(store, instance_id)?;

    store.begin_batch();
    // [R22] cascade: incident relation files are declared targets of this delete.
    let cascade_result = relation_service::delete_relations_incident_to(store, instance_id);
    match cascade_result {
        Ok(_) => store.commit_batch()?,
        Err(e) => {
            store.abort_batch();
            return Err(e);
        }
    }
    // The record's own file, deleted only after its incident relations are gone.
    //
    // This delete propagates its failure (srs-rust#839). It was best-effort
    // until the #834 cascade above started running first: a swallowed failure now
    // means the instance survives on disk stripped of every container reference
    // and every incident relation, reported as successfully deleted — the caller
    // is told the record is gone while it sits there belonging to nothing.
    // Truthful diagnostics over convenience: no `ok: true` over damage.
    //
    // A revision sidecar delete used to run first here. Removed (rfc-decision-
    // 2a1e1590, srs-rust#866): it was unreachable anyway — any `.revisions.json`
    // left in the tree, this record's own included, already fails `store.catalog()`
    // above ([R24], whole-repository fatal), so this line could never have run
    // while there was a sidecar for it to delete.
    store.delete_instance_file(&path)?;

    Ok(instance_id.to_string())
}

/// Filter options for listing records
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordListFilter {
    pub type_namespace: Option<String>,
    pub type_name: Option<String>,
    /// If Some, only return records that are members of this container.
    pub container_id: Option<String>,
    /// If Some, only return records whose manifest tag list contains this value.
    pub tag: Option<String>,
}

/// Input for creating or updating a record.
///
/// `field_values` is the RFC-039 object carrier: keys are `Field.name`
/// verbatim, values follow the recursive Change-B rule. `field_meta`
/// semantics on update: absent/`null` → preserve stored; `{}` → clear;
/// `{...}` → replace.
///
/// `tags` semantics (both create and update):
/// - Absent or `null` in JSON → `None` → on create: no tags; on update: preserve existing tags.
/// - `[]` (empty array) → `Some(vec![])` → on create: no tags; on update: clear all tags.
/// - `["foo", ...]` → `Some(vec![...])` → on create: set tags; on update: replace tags.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordInput {
    pub field_values: FieldValues,
    #[serde(default)]
    pub field_meta: Option<indexmap::IndexMap<String, srs_core::types::record::FieldMeta>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Input for `update_record`.
///
/// `type_version` is optional: when `None`, the stored version is preserved; when
/// `Some(v)`, the record is migrated to that version (the type must exist in the
/// package at `v`).
///
/// Tag semantics match `CreateRecordInput`:
/// - `None` → preserve existing tags.
/// - `Some([])` → clear all tags.
/// - `Some([...])` → replace tags.
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecordInput {
    pub field_values: FieldValues,
    #[serde(default)]
    pub field_meta: Option<indexmap::IndexMap<String, srs_core::types::record::FieldMeta>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub type_version: Option<u32>,
}

/// Self-contained input for `validate_record_input` (no-write preflight).
///
/// Unlike `CreateRecordInput`, this carries its own type binding (`typeId`/
/// `typeVersion`) so the input is fully self-describing and resolves via
/// `package.resolve_type` — the same call the create/update paths use.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateRecordInput {
    pub type_id: String,
    pub type_version: u32,
    pub field_values: FieldValues,
    #[serde(default)]
    pub field_meta: Option<indexmap::IndexMap<String, srs_core::types::record::FieldMeta>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Result of `validate_record_input`. Mirrors the `{ ok, errors }` shape of the
/// other `*-validate` reports. `errors` is empty iff `ok` is true.
#[derive(Debug, Clone)]
pub struct RecordValidateReport {
    pub ok: bool,
    pub errors: Vec<String>,
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

/// Input for [`get_field_value_by_name`].
#[derive(Debug)]
pub struct GetFieldValueByNameInput {
    pub instance_id: String,
    pub field_name: String,
}

/// Result for [`get_field_value_by_name`].
///
/// Accessed structurally by callers (e.g. the WASM binding unpacks `.value` directly);
/// never serialized as a whole struct, so no `Serialize` derive is needed.
#[derive(Debug, Clone)]
pub struct GetFieldValueByNameResult {
    pub value: Option<serde_json::Value>,
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

    let refs = store.list_instances(&InstanceQuery {
        tier: Some(2),
        tag: None,
    })?;
    let mut records = Vec::new();

    for r in refs {
        // Container membership filter
        if let Some(ref member_set) = member_ids {
            if !member_set.contains(&r.instance_id) {
                continue;
            }
        }

        // Tag filter — resolved from the index (no file load needed)
        if let Some(ref tag_filter) = filter.tag {
            let has_tag = r.tags.iter().any(|t| t == tag_filter);
            if !has_tag {
                continue;
            }
        }

        let record = store.load_record_by_id(&r.instance_id)?;

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

/// A listed record paired with its core-resolved display label.
///
/// The `display_label` comes from [`record_label::record_display_label`] (priority
/// `title` → `name` → `label` → `type_name` fallback) — the *same* resolution
/// `srs tree` and `resolve_container_view` (#254) use. Clients render this label
/// directly and must not re-derive titles from `field_values` (capability-layering:
/// "clients add presentation, never semantics").
///
/// Shape mirrors `container_view_service::ResolvedMember` (minus `tier`, since
/// `record list` returns only Tier-2 Records): the full `Record` is nested so the
/// client can still render cells/fields against it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordSummary {
    pub instance_id: String,
    /// Core-resolved label via `record_label::record_display_label`.
    pub display_label: String,
    pub record: Record,
}

/// List records (same filter semantics as [`list_records_filtered`]), each paired
/// with its core-resolved `display_label`.
///
/// Delegates to [`list_records_filtered`] for the records, builds the
/// `field_id → field_name` index once, and resolves each label via
/// [`record_label::record_display_label`]. No filtering or label logic is duplicated.
pub fn list_record_summaries(
    store: &dyn RepositoryStore,
    filter: RecordListFilter,
) -> Result<Vec<RecordSummary>, RepositoryError> {
    let records = list_records_filtered(store, filter)?;
    let (field_name_index, identity_field_index) = record_label::build_label_indexes(store)?;
    Ok(records
        .into_iter()
        .map(|record| {
            let instance_id = record.instance_id.clone();
            let display_label = record_label::record_display_label(
                &record,
                &identity_field_index,
                &field_name_index,
            );
            RecordSummary {
                instance_id,
                display_label,
                record,
            }
        })
        .collect())
}

/// Get a single record by instance ID, paired with its core-resolved display label.
///
/// Returns `None` when no record with `id` exists (same semantics as [`get_record_by_id`]).
/// Label resolution uses the same priority as [`list_record_summaries`]: the record's Type's
/// effective `identityFieldId` > field named "title" > "name" > "label" > `type_name` fallback.
///
/// Builds the label indexes on every call — suitable for UI single-record fetches. Callers
/// fetching multiple records should use [`list_record_summaries`] to amortise the index-build
/// cost.
pub fn get_record_summary_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<RecordSummary>, RepositoryError> {
    match get_record_by_id(store, id)? {
        None => Ok(None),
        Some(record) => {
            let (field_name_index, identity_field_index) =
                record_label::build_label_indexes(store)?;
            let instance_id = record.instance_id.clone();
            let display_label = record_label::record_display_label(
                &record,
                &identity_field_index,
                &field_name_index,
            );
            Ok(Some(RecordSummary {
                instance_id,
                display_label,
                record,
            }))
        }
    }
}

/// Best-effort rollback for a failed `add_member` step.
///
/// Calls `delete_record` to remove the newly-written record from the manifest. Any error from
/// the cleanup is silently discarded via `let _ = …`.
///
/// **Failure mode:** `delete_record` follows ADR-007 index-first ordering — the manifest entry
/// is removed and written before the file is deleted. If the manifest write fails, neither the
/// file nor the manifest is changed and the record remains intact. If the manifest write succeeds
/// but the subsequent file deletion fails, the file is left as an orphan (invisible to readers,
/// recoverable by `srs repo repair`) rather than as a dangling index entry. The common case
/// (transient I/O error on `add_member`) therefore cleans up correctly. See ADR-024.
///
/// TODO: fault-injection test for this error arm pending a FailStore test double (see ADR-024).
fn attempt_rollback_delete(store: &dyn RepositoryStore, instance_id: &str) {
    let _ = delete_record(store, instance_id);
}

/// Create a record from a `namespace/name` type filter and optionally add to a container.
///
/// - Parses `type_filter` as `namespace/name`
/// - Resolves the type (with optional version pin)
/// - Creates the record
/// - If `container_id` is Some, validates the container exists and adds the record
///
/// If the `add_member` step fails, best-effort rollback via `attempt_rollback_delete`.
/// See ADR-024 for the accepted limitations of this approach.
///
/// `dir_override` lets CLI callers honour a user-supplied `--dir` flag. Pass `None`
/// to use `RecordTier::Tier2` (via `store.record_tier_dir`). Raw path strings must not appear in binding code or
/// CLI handlers — bind the user flag value here and nowhere else.
pub fn create_record_in_context(
    store: &dyn RepositoryStore,
    type_filter: &str,
    type_version: Option<u32>,
    input: CreateRecordInput,
    container_id: Option<String>,
    dir_override: Option<&str>,
) -> Result<CreateRecordResult, RepositoryError> {
    let dir = dir_override.unwrap_or(store.record_tier_dir(RecordTier::Tier2));

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

    let record = create_record_at_dir(
        store,
        &record_type.id,
        record_type.version,
        input.field_values,
        input.field_meta,
        input.tags,
        dir,
    )?;

    if let Some(ref cid) = container_id {
        if let Err(e) = container_service::add_member(store, cid, &record.instance_id) {
            attempt_rollback_delete(store, &record.instance_id);
            return Err(e);
        }
    }

    Ok(CreateRecordResult { record })
}

/// Delete a record with optional container-scoped membership check.
///
/// If `container_id` is Some, the record must be a member of that container —
/// the scope is a *guard*, not a mutation. Removing the membership is
/// `delete_record`'s own cascade (srs-rust#834), which clears every container,
/// so doing it here as well would be a second implementation of the same step.
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
    }

    let instance_id = delete_record(store, &id)?;
    Ok(DeleteRecordResult { instance_id })
}

/// Input for `create_record_in_container`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordInContainerInput {
    pub container_id: String,
    pub type_id: String,
    pub type_version: u32,
    pub field_values: FieldValues,
    #[serde(default)]
    pub field_meta: Option<indexmap::IndexMap<String, srs_core::types::record::FieldMeta>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Create a Tier-2 record and add it to a container in one call (caller-omission atomic).
///
/// Steps (in order):
///   1. Validate the container exists — returns `ContainerNotFound` if absent (pre-write).
///   2. Create the record via `create_record_at_dir` (uses `RecordTier::Tier2`).
///   3. Add the new record to the container's `memberInstanceIds` via `container_service::add_member`.
///
/// If step 3 fails, best-effort rollback via `attempt_rollback_delete`. See ADR-024 for
/// the accepted limitations of this approach.
pub fn create_record_in_container(
    store: &dyn RepositoryStore,
    input: CreateRecordInContainerInput,
) -> Result<CreateRecordResult, RepositoryError> {
    container_service::get_container(store, &input.container_id)?;

    let record = create_record_at_dir(
        store,
        &input.type_id,
        input.type_version,
        input.field_values,
        input.field_meta,
        input.tags,
        store.record_tier_dir(RecordTier::Tier2),
    )?;

    if let Err(e) = container_service::add_member(store, &input.container_id, &record.instance_id) {
        attempt_rollback_delete(store, &record.instance_id);
        return Err(e);
    }

    Ok(CreateRecordResult { record })
}

/// Input for transitioning a record's lifecycle state.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionLifecycleInput {
    /// Target state name (use either `to` or `by_transition`, not both).
    pub to: Option<String>,
    /// Named transition (e.g., "promote") — resolved to its `to` state.
    pub by_transition: Option<String>,
    /// RFC-022: how a transition into a `requiresRelation` state establishes
    /// its relation obligation. Must be absent for other target states.
    #[serde(default)]
    pub fulfillment: Option<TransitionFulfillmentInput>,
}

/// RFC-022 fulfillment for a transition into a `requiresRelation` state.
/// Exactly one of `new_record` / `existing_instance_id` when present.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionFulfillmentInput {
    /// Spawn a successor of the record's type, relate it, then transition.
    pub new_record: Option<FulfillmentNewRecord>,
    /// Relate an already-existing instance, then transition.
    pub existing_instance_id: Option<String>,
    /// Selector when the state declares an any-of relationType array.
    /// Must be one of the declared types; defaults to the first declared.
    pub relation_type: Option<String>,
}

/// Successor seed for `fulfillment.newRecord`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FulfillmentNewRecord {
    pub field_values: FieldValues,
    /// Optional type version override (defaults to the predecessor's).
    pub type_version: Option<u32>,
}

/// Result for transition_record_lifecycle — includes warnings for final-state transitions
/// and any diagnostics from the best-effort revision append step. When the transition was
/// fulfilled (RFC-022), `successor` / `relation` carry the fulfillment artifacts.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionLifecycleResult {
    pub record: Record,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub successor: Option<Record>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<Relation>,
}

/// One legal next transition for a record in its current lifecycle state.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleTransitionOption {
    /// Display name of the transition (e.g. "promote", "archive").
    pub name: String,
    /// Target state key.
    pub to: String,
    /// Whether the target state has `is_final: true`.
    pub to_is_final: bool,
    /// RFC-022: the target state's relation obligation, when it declares one.
    /// Clients route "this transition needs a successor" UX from this structure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires_relation: Option<RequiresRelation>,
}

/// Result of `get_allowed_lifecycle_transitions`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedLifecycleTransitionsResult {
    /// The record's current lifecycle state key (empty string if unset).
    pub current_state: String,
    /// Transitions the record is permitted to take from its current state.
    pub transitions: Vec<LifecycleTransitionOption>,
    /// True when the current state has `is_final: true`.
    pub is_immutable: bool,
}

/// Input for creating a successor record.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordSuccessorInput {
    /// "supersedes" or "refines"
    pub relation_type: String,
    pub field_values: FieldValues,
    /// Optional initial lifecycle state for the successor (defaults to Type.initialState).
    pub lifecycle_state: Option<String>,
    /// Optional type version override (defaults to same as predecessor).
    pub type_version: Option<u32>,
}

/// Result for create_record_successor.
#[derive(Debug, Clone, serde::Serialize)]
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

    let lifecycle = package.effective_lifecycle(record_type).ok_or_else(|| {
        RepositoryError::LifecycleNotDefined {
            id: instance_id.to_string(),
        }
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

    // Validate a transition path from current → target exists.
    //
    // srs-rust#880: a record with no lifecycleState yet — created before its Type
    // carried a lifecycle, which is later bound retroactively (lifecycleRef added
    // after the fact) — has no edge to walk: no transition is ever declared with
    // `from: ""`. This is not a new kind of write; it is the same "first entry"
    // act `create_record` already performs for a brand-new record (assigning a
    // state the record does not yet occupy), just arriving later. So it is
    // validated the same way `create_record_successor`'s explicit lifecycle_state
    // override already validates: `state_reachable_from_initial` — the record may
    // enter any state reachable from the Lifecycle's own `initialState`, not just
    // the initial state itself, since a retrofit's target is whatever state the
    // record's pre-Lifecycle data already represents (e.g. migrating a `superseded`
    // rfc_status straight to the `superseded` LifecycleState — srs#447).
    let current_state = record.lifecycle_state.clone().unwrap_or_default();
    if record.lifecycle_state.is_none() {
        if !state_reachable_from_initial(
            lifecycle.initial_state,
            lifecycle.transitions,
            &target_state,
        ) {
            return Err(RepositoryError::LifecycleStateUnreachable {
                state: target_state.clone(),
                initial: lifecycle.initial_state.to_string(),
            });
        }
    } else {
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
    }

    // Check if target state is final → emit warning
    let mut warnings = Vec::new();
    let state_def = lifecycle.states.iter().find(|s| s.key == target_state);
    if let Some(state_def) = state_def {
        if state_def.is_final == Some(true) {
            warnings.push(format!(
                "LIFECYCLE_FINAL_STATE: Target state '{}' is a final state — no further transitions are expected",
                target_state
            ));
        }
    }

    // RFC-022: enforce the target state's relation obligation, performing any
    // requested fulfillment (successor spawn / adoption) BEFORE the state flip
    // so every committed prefix is a valid repository (R7).
    let requires = state_def.and_then(|s| s.requires_relation.clone());
    let mut successor_out: Option<Record> = None;
    let mut relation_out: Option<Relation> = None;
    match (&requires, &input.fulfillment) {
        (None, None) => {}
        (None, Some(_)) => {
            return Err(RepositoryError::LifecycleFulfillmentNotApplicable {
                state: target_state,
            });
        }
        (Some(req), fulfillment) => {
            let declared: Vec<String> = req
                .relation_type
                .types()
                .iter()
                .map(|t| t.to_string())
                .collect();
            let direction = req.effective_direction();
            match fulfillment {
                None => {
                    // Bare transition: allowed iff the obligation is already satisfied (R2).
                    if !relation_obligation_satisfied(store, instance_id, &declared, direction)? {
                        return Err(RepositoryError::LifecycleRelationRequired {
                            state: target_state,
                            relation_types: declared,
                            direction: direction.to_string(),
                        });
                    }
                }
                Some(f) => {
                    let selected_type = match &f.relation_type {
                        Some(rt) if declared.iter().any(|d| d == rt) => rt.clone(),
                        Some(rt) => {
                            return Err(
                                RepositoryError::LifecycleFulfillmentRelationTypeMismatch {
                                    state: target_state,
                                    relation_type: rt.clone(),
                                    declared,
                                },
                            );
                        }
                        None => declared[0].clone(),
                    };
                    let (successor, relation) = match (&f.new_record, &f.existing_instance_id) {
                        (Some(nr), None) => {
                            let type_version = nr.type_version.unwrap_or(record.type_version);
                            package
                                .resolve_type(&record.type_id, type_version)
                                .ok_or_else(|| RepositoryError::TypeVersionNotFound {
                                    type_id: record.type_id.clone(),
                                    version: type_version,
                                })?;
                            let successor = create_record_at_dir(
                                store,
                                &record.type_id,
                                type_version,
                                nr.field_values.clone(),
                                None,
                                None,
                                store.record_tier_dir(RecordTier::Tier2),
                            )?;
                            let (source_id, target_id) = match direction {
                                RelationDirection::Incoming => {
                                    (successor.instance_id.clone(), instance_id.to_string())
                                }
                                RelationDirection::Outgoing => {
                                    (instance_id.to_string(), successor.instance_id.clone())
                                }
                            };
                            match assert_fulfillment_relation(
                                store,
                                &selected_type,
                                source_id,
                                target_id,
                            ) {
                                Ok(rel) => (Some(successor), rel),
                                Err(e) => {
                                    attempt_rollback_delete(store, &successor.instance_id);
                                    return Err(e);
                                }
                            }
                        }
                        (None, Some(existing_id)) => {
                            if existing_id == instance_id {
                                return Err(RepositoryError::InvalidInput {
                                    message: "fulfillment.existingInstanceId must not be the record being transitioned".to_string(),
                                });
                            }
                            get_record_by_id(store, existing_id)?.ok_or_else(|| {
                                RepositoryError::NotFound {
                                    path: std::path::PathBuf::from("records"),
                                }
                            })?;
                            let (source_id, target_id) = match direction {
                                RelationDirection::Incoming => {
                                    (existing_id.clone(), instance_id.to_string())
                                }
                                RelationDirection::Outgoing => {
                                    (instance_id.to_string(), existing_id.clone())
                                }
                            };
                            let rel = assert_fulfillment_relation(
                                store,
                                &selected_type,
                                source_id,
                                target_id,
                            )?;
                            (None, rel)
                        }
                        _ => {
                            return Err(RepositoryError::InvalidInput {
                                message: "fulfillment requires exactly one of 'newRecord' or 'existingInstanceId'".to_string(),
                            });
                        }
                    };
                    successor_out = successor;
                    relation_out = Some(relation);
                }
            }
        }
    }

    // Build updated record — locator from the catalog (RFC-038: no instance_index).
    let path = store
        .catalog()?
        .instances
        .iter()
        .find(|e| e.id == instance_id)
        .and_then(|e| e.locator.clone())
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let updated = Record {
        lifecycle_state: Some(target_state),
        updated_at: Some(chrono::Utc::now().to_rfc3339()),
        ..record
    };

    // The flip is committed last (R7). If it fails after fulfillment writes,
    // best-effort rollback of the fulfillment artifacts (every prefix stays valid).
    if let Err(e) = write_record(store, &updated, &path) {
        if let Some(rel) = &relation_out {
            let _ = relation_service::delete_relation(store, &rel.relation_id);
        }
        if let Some(succ) = &successor_out {
            attempt_rollback_delete(store, &succ.instance_id);
        }
        return Err(e);
    }

    // Revision sidecars removed: rfc-decision-2a1e1590 retired the per-field
    // Revision mechanism from the spec (zero corpus consumers, PascalCase wire
    // leak, no implementation exercising the return-trigger chain). This was
    // the mechanism's one production write site (srs-rust#866) — a lifecycle
    // transition no longer appends anything here.

    Ok(TransitionLifecycleResult {
        record: updated,
        warnings,
        successor: successor_out,
        relation: relation_out,
    })
}

/// RFC-022: does any relation satisfy the obligation `declared`/`direction` for `instance_id`?
fn relation_obligation_satisfied(
    store: &dyn RepositoryStore,
    instance_id: &str,
    declared: &[String],
    direction: RelationDirection,
) -> Result<bool, RepositoryError> {
    let filter = match direction {
        RelationDirection::Incoming => relation_service::ListRelationsFilter {
            target: Some(instance_id.to_string()),
            ..Default::default()
        },
        RelationDirection::Outgoing => relation_service::ListRelationsFilter {
            source: Some(instance_id.to_string()),
            ..Default::default()
        },
    };
    let relations = relation_service::list_relations(store, filter)?;
    Ok(relations
        .iter()
        .any(|r| declared.iter().any(|t| t == &r.relation_type)))
}

/// RFC-022: assert the relation that fulfils a `requiresRelation` obligation.
fn assert_fulfillment_relation(
    store: &dyn RepositoryStore,
    relation_type: &str,
    source_instance_id: String,
    target_instance_id: String,
) -> Result<Relation, RepositoryError> {
    let result = relation_service::create_relation_auto(
        store,
        Relation {
            relation_id: String::new(),
            relation_type: relation_type.to_string(),
            source_instance_id,
            target_instance_id,
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
    Ok(result.relation)
}

/// Query the allowed lifecycle transitions for a record in its current state.
///
/// Returns the current state, all transitions valid from it, and whether the record
/// is in a final (immutable) state. Returns `RepositoryError::NotFound` if the
/// instance ID does not exist, `RepositoryError::LifecycleNotDefined` if the type
/// has no lifecycle.
///
/// If the record has never been transitioned (`lifecycle_state` is `None`), `current_state`
/// is `""` and `transitions` will be empty (no transition is defined from the empty state).
pub fn get_allowed_lifecycle_transitions(
    store: &dyn RepositoryStore,
    instance_id: &str,
) -> Result<AllowedLifecycleTransitionsResult, RepositoryError> {
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
    let lifecycle = package.effective_lifecycle(record_type).ok_or_else(|| {
        RepositoryError::LifecycleNotDefined {
            id: instance_id.to_string(),
        }
    })?;

    let current_state = record.lifecycle_state.clone().unwrap_or_default();
    let is_immutable = lifecycle
        .states
        .iter()
        .any(|s| s.key == current_state && s.is_final == Some(true));
    let transitions = lifecycle
        .transitions
        .iter()
        .filter(|t| t.from == current_state)
        .map(|t| {
            let target_def = lifecycle.states.iter().find(|s| s.key == t.to);
            let to_is_final = target_def.is_some_and(|s| s.is_final == Some(true));
            LifecycleTransitionOption {
                name: t.name.clone(),
                to: t.to.clone(),
                to_is_final,
                requires_relation: target_def.and_then(|s| s.requires_relation.clone()),
            }
        })
        .collect();
    Ok(AllowedLifecycleTransitionsResult {
        current_state,
        transitions,
        is_immutable,
    })
}

/// Create a successor record (supersedes or refines an existing record).
///
/// Creates a new Record with the same typeId+typeVersion (or a specified version),
/// then automatically adds a Relation from the successor to the predecessor.
/// The successor record is written to the `RecordTier::Tier2` directory.
pub fn create_record_successor(
    store: &dyn RepositoryStore,
    predecessor_id: &str,
    input: CreateRecordSuccessorInput,
) -> Result<CreateRecordSuccessorResult, RepositoryError> {
    let predecessor =
        get_record_by_id(store, predecessor_id)?.ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;

    let type_version = input.type_version.unwrap_or(predecessor.type_version);

    // Validate the requested type version — and pre-validate any explicit
    // lifecycle-state override against the effective lifecycle — before writing
    // anything. The override must be a defined state, reachable from the initial
    // state via declared transitions (it is not a back door around transition
    // validation), and any RFC-022 relation obligation it carries is checked
    // after the successor relation is asserted below.
    let mut requires_for_explicit: Option<RequiresRelation> = None;
    // Thread definitions out of the package block so the second write (create_relation) can
    // use them directly, avoiding a second load_package() call inside create_relation_auto.
    let definitions: Vec<RelationTypeDefinition>;
    {
        let package = store.load_package()?;
        let record_type = package
            .resolve_type(&predecessor.type_id, type_version)
            .ok_or_else(|| RepositoryError::TypeVersionNotFound {
                type_id: predecessor.type_id.clone(),
                version: type_version,
            })?;
        // Validate relation_type before writing the record, so an E1 failure avoids
        // the ADR-024 best-effort rollback path (delete_record after failed create_relation).
        // relation_id is empty in the error — no relation has been created yet.
        validate_relation_type_for_write(&package.relation_type_definitions, &input.relation_type)
            .map_err(|e| RepositoryError::RelationValidation {
                relation_id: String::new(),
                message: e.message,
            })?;
        definitions = package.relation_type_definitions.clone();
        if let Some(explicit_state) = input.lifecycle_state.as_deref() {
            let lifecycle = package.effective_lifecycle(record_type).ok_or_else(|| {
                RepositoryError::LifecycleNotDefined {
                    id: predecessor_id.to_string(),
                }
            })?;
            let state_def = lifecycle
                .states
                .iter()
                .find(|s| s.key == explicit_state)
                .ok_or_else(|| RepositoryError::LifecycleStateNotDefined {
                    state: explicit_state.to_string(),
                })?;
            if !state_reachable_from_initial(
                lifecycle.initial_state,
                lifecycle.transitions,
                explicit_state,
            ) {
                return Err(RepositoryError::LifecycleStateUnreachable {
                    state: explicit_state.to_string(),
                    initial: lifecycle.initial_state.to_string(),
                });
            }
            requires_for_explicit = state_def.requires_relation.clone();
        }
    }

    // Create the successor record (lifecycle_state auto-set from Type.initialState).
    let mut successor = create_record_at_dir(
        store,
        &predecessor.type_id,
        type_version,
        input.field_values,
        None,
        None,
        store.record_tier_dir(RecordTier::Tier2),
    )?;

    // Create the relation: successor → predecessor.
    // Use create_relation directly with the definitions already loaded above — avoids a
    // second load_package() call inside create_relation_auto.
    let rel_result = match relation_service::create_relation(
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
        &definitions,
    ) {
        Ok(r) => r,
        Err(e) => {
            attempt_rollback_delete(store, &successor.instance_id);
            return Err(e);
        }
    };

    // Apply the explicit lifecycle_state AFTER the relation exists, so an RFC-022
    // obligation the successor relation itself satisfies (e.g. an outgoing
    // `supersedes`) can hold; reject — rolling back — if it does not.
    if let Some(explicit_state) = input.lifecycle_state {
        if successor.lifecycle_state.as_deref() != Some(explicit_state.as_str()) {
            if let Some(req) = &requires_for_explicit {
                let declared: Vec<String> = req
                    .relation_type
                    .types()
                    .iter()
                    .map(|t| t.to_string())
                    .collect();
                let direction = req.effective_direction();
                if !relation_obligation_satisfied(
                    store,
                    &successor.instance_id,
                    &declared,
                    direction,
                )? {
                    let _ =
                        relation_service::delete_relation(store, &rel_result.relation.relation_id);
                    attempt_rollback_delete(store, &successor.instance_id);
                    return Err(RepositoryError::LifecycleRelationRequired {
                        state: explicit_state,
                        relation_types: declared,
                        direction: direction.to_string(),
                    });
                }
            }
            successor.lifecycle_state = Some(explicit_state);
            store.save_record(&successor)?;
        }
    }

    Ok(CreateRecordSuccessorResult {
        record: successor,
        relation: rel_result.relation,
    })
}

/// Is `target` reachable from `initial` via the declared transitions (BFS)?
fn state_reachable_from_initial(
    initial: &str,
    transitions: &[srs_core::types::lifecycle::LifecycleTransition],
    target: &str,
) -> bool {
    if initial == target {
        return true;
    }
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    visited.insert(initial);
    let mut queue = vec![initial];
    while let Some(current) = queue.pop() {
        for t in transitions.iter().filter(|t| t.from == current) {
            if t.to == target {
                return true;
            }
            if visited.insert(t.to.as_str()) {
                queue.push(t.to.as_str());
            }
        }
    }
    false
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
    let path = store
        .catalog()?
        .instances
        .iter()
        .find(|e| e.id == instance_id && e.tier == Some(2))
        .and_then(|e| e.locator.clone())
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;
    revision_service::list(store, &path, instance_id, field_id, limit, offset)
}

/// Get a single revision by its revision_id, scoped to a specific record.
pub fn get_record_revision(
    store: &dyn RepositoryStore,
    instance_id: &str,
    revision_id: &str,
) -> Result<Option<Revision>, RepositoryError> {
    let path = store
        .catalog()?
        .instances
        .iter()
        .find(|e| e.id == instance_id && e.tier == Some(2))
        .and_then(|e| e.locator.clone())
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records"),
        })?;
    revision_service::get(store, &path, instance_id, revision_id)
}

/// Result of `add_record_tag`.
pub enum AddRecordTagResult {
    /// Tag was new and has been added.
    Added { record: Record, tag: String },
    /// Tag was already present; record is unchanged.
    AlreadyPresent { record: Record, tag: String },
    /// No tier-2 record with this ID exists in the manifest.
    NotFound,
}

/// Result of `remove_record_tag`.
pub enum RemoveRecordTagResult {
    /// Tag was present and has been removed.
    Removed { record: Record, tag: String },
    /// Tag was not present; record is unchanged.
    NotPresent { record: Record, tag: String },
    /// No tier-2 record with this ID exists in the manifest.
    NotFound,
}

/// Add a tag to a tier-2 record.
///
/// Writes the record body and mirrors the updated tag list into the manifest index.
/// Returns `NotFound` if no tier-2 entry with the given ID exists.
pub fn add_record_tag(
    store: &dyn RepositoryStore,
    id: &str,
    tag: &str,
) -> Result<AddRecordTagResult, RepositoryError> {
    match store.find_instance(id)? {
        Some(r) if r.tier == 2 => {
            let mut record = store.load_record_by_id(id)?;

            let tags = record.tags.get_or_insert_with(Vec::new);
            if tags.contains(&tag.to_string()) {
                return Ok(AddRecordTagResult::AlreadyPresent {
                    record,
                    tag: tag.to_string(),
                });
            }
            tags.push(tag.to_string());

            store.save_record(&record)?;

            Ok(AddRecordTagResult::Added {
                record,
                tag: tag.to_string(),
            })
        }
        _ => Ok(AddRecordTagResult::NotFound),
    }
}

/// Remove a tag from a tier-2 record.
///
/// Writes the record body and mirrors the updated tag list into the manifest index.
/// Returns `NotFound` if no tier-2 entry with the given ID exists.
pub fn remove_record_tag(
    store: &dyn RepositoryStore,
    id: &str,
    tag: &str,
) -> Result<RemoveRecordTagResult, RepositoryError> {
    match store.find_instance(id)? {
        Some(r) if r.tier == 2 => {
            let mut record = store.load_record_by_id(id)?;

            let tags = record.tags.get_or_insert_with(Vec::new);
            if !tags.contains(&tag.to_string()) {
                return Ok(RemoveRecordTagResult::NotPresent {
                    record,
                    tag: tag.to_string(),
                });
            }
            tags.retain(|t| t != tag);
            if tags.is_empty() {
                record.tags = None;
            }

            store.save_record(&record)?;

            Ok(RemoveRecordTagResult::Removed {
                record,
                tag: tag.to_string(),
            })
        }
        _ => Ok(RemoveRecordTagResult::NotFound),
    }
}

/// Per-tag count summary across tier-2 records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordTagSummary {
    pub tag: String,
    pub record_count: usize,
}

/// Result of `list_record_tags`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRecordTagsResult {
    pub total_records: usize,
    pub tags: Vec<RecordTagSummary>,
}

/// List distinct tags across all tier-2 records in the repository.
///
/// Reads only the manifest index — no per-record file loads.
/// Optionally scoped to members of a container.
pub fn list_record_tags(
    store: &dyn RepositoryStore,
    container_id: Option<&str>,
) -> Result<ListRecordTagsResult, RepositoryError> {
    let member_ids: Option<std::collections::HashSet<String>> = if let Some(cid) = container_id {
        let members = container_service::list_members(store, cid)?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    let refs = store.list_instances(&InstanceQuery {
        tier: Some(2),
        tag: None,
    })?;
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut total_records = 0;

    for r in &refs {
        if let Some(ref m) = member_ids {
            if !m.contains(&r.instance_id) {
                continue;
            }
        }
        total_records += 1;
        for tag in &r.tags {
            *counts.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    let tags = counts
        .into_iter()
        .map(|(tag, record_count)| RecordTagSummary { tag, record_count })
        .collect();

    Ok(ListRecordTagsResult {
        total_records,
        tags,
    })
}

/// Return the value of a named field on a record, by its exact package-defined name
/// (the `name` field in the field definition JSON, e.g. `"title"` or `"decision-summary"`).
/// No case normalization is performed — the caller must pass the exact name.
///
/// Returns `Ok(GetFieldValueByNameResult { value: None })` for all missing/not-found
/// conditions: record not found, type not resolvable, field name absent from schema,
/// or field has no value set on the record. Infrastructure errors (IO, JSON parse)
/// propagate as `Err`.
///
/// Uses `effective_fields` so inherited fields are also resolved.
pub fn get_field_value_by_name(
    store: &dyn RepositoryStore,
    input: GetFieldValueByNameInput,
) -> Result<GetFieldValueByNameResult, RepositoryError> {
    let record = match get_record_by_id(store, &input.instance_id)? {
        Some(r) => r,
        None => return Ok(GetFieldValueByNameResult { value: None }),
    };
    // RFC-039: the carrier is name-keyed, so the lookup is direct ([R2b]).
    Ok(GetFieldValueByNameResult {
        value: record.value(&input.field_name).cloned(),
    })
}

/// Write a new record JSON file to `dir` using the canonical `{type_name}-{id8}.json` filename
/// convention. Writes only the entity file — membership comes from the tree ([R1]),
/// never `manifest.json` ([R22]). Returns the relative path written.
pub(crate) fn write_new_record(
    store: &dyn RepositoryStore,
    record: &Record,
    dir: &str,
) -> Result<String, RepositoryError> {
    let relative_path = format!(
        "{}/{}-{}.json",
        dir,
        slugify_instance_name(&record.type_name),
        &record.instance_id[..8]
    );
    store.ensure_instance_dir(dir)?;
    write_record(store, record, &relative_path)?;
    Ok(relative_path)
}

/// Append a SourceReference to a tier-2 record by instance ID.
///
/// Path lookup is encapsulated here so service callers never handle storage paths
/// (ADR-010 storage-boundary rule). `sourceRefs` on `Record` is persisted via
/// `record.extra["sourceRefs"]`; see ADR-034 for why a typed field is not used.
///
/// When `check_duplicate` is `true`, returns `InvalidInput` if an existing ref with the
/// same `(source_type, source_id, source_role)` triple is already present. The check
/// runs on the same record load used for the write, eliminating the TOCTOU that would
/// arise if callers loaded the record separately to perform the check.
///
/// Returns the updated record (with the appended ref in extra["sourceRefs"]).
pub(crate) fn append_source_ref(
    store: &dyn RepositoryStore,
    instance_id: &str,
    source_ref: SourceReference,
    check_duplicate: bool,
) -> Result<Record, RepositoryError> {
    match store.find_instance(instance_id)? {
        Some(r) if r.tier == 2 => {}
        _ => {
            return Err(RepositoryError::NotFound {
                path: std::path::PathBuf::from("records"),
            })
        }
    }

    let mut record = store.load_record_by_id(instance_id)?;

    let mut refs: Vec<SourceReference> = match record.extra.get("sourceRefs") {
        None => Vec::new(),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| RepositoryError::Serialize {
            path: std::path::PathBuf::from(instance_id),
            source: e,
        })?,
    };

    if check_duplicate
        && refs.iter().any(|r| {
            r.source_type == source_ref.source_type
                && r.source_id == source_ref.source_id
                && r.source_role == source_ref.source_role
        })
    {
        return Err(RepositoryError::InvalidInput {
            message: format!(
                "document '{}' is already linked to record '{}'",
                source_ref.source_id, instance_id
            ),
        });
    }

    refs.push(source_ref);
    record.extra.insert(
        "sourceRefs".to_string(),
        serde_json::to_value(&refs).map_err(|e| RepositoryError::Serialize {
            path: std::path::PathBuf::from(instance_id),
            source: e,
        })?,
    );

    store.save_record(&record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use srs_core::types::record::FieldMeta;
    use std::path::PathBuf;

    fn srs_spec_repo() -> PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let vendored = manifest.join("../../tests/fixtures/spec-repo");
        if let Ok(c) = vendored.canonicalize() {
            if c.join(".srs").exists() {
                return c;
            }
        }
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
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let status_field = Field {
            schema: None,
            id: "field-status-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-status".to_string(),
            version: 1,
            field_type: FieldType::select(vec!["active".to_string(), "inactive".to_string()]),
            description: "Status field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let test_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
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
                    description: None,
                },
                FieldAssignment {
                    field_id: "field-status-001".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Status".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    /// Store whose package has a field literally named `title` (id `field-title-0001`)
    /// and a non-label field named `summary` (id `field-summary-0001`), both optional,
    /// on type `labeled-type` (id `type-labeled-0001`). Lets `list_record_summaries`
    /// tests exercise both the `title` priority and the `type_name` fallback of
    /// `record_display_label`. Identifiers are >= 8 chars so the snapshot importer used
    /// by `copy_repository` accepts the fixture.
    fn make_store_with_title_field() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let plain_field = |id: &str, name: &str| Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: String::new(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let assignment = |field_id: &str, order: u32| FieldAssignment {
            field_id: field_id.to_string(),
            order,
            required: false,
            display_label: None,
            description: None,
        };
        let labeled_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-labeled-0001".to_string(),
            namespace: "com.test".to_string(),
            name: "labeled-type".to_string(),
            version: 1,
            description: "Type with a title field".to_string(),
            fields: vec![
                assignment("field-title-0001", 0),
                assignment("field-summary-0001", 1),
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "package-labeled-0001".to_string(),
            namespace: "com.test".to_string(),
            name: "labeled-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                plain_field("field-title-0001", "title"),
                plain_field("field-summary-0001", "summary"),
            ],
            record_types: vec![labeled_type],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    fn fvs(pairs: Vec<(&str, serde_json::Value)>) -> FieldValues {
        let mut m = FieldValues::new();
        for (k, v) in pairs {
            m.insert(k, v);
        }
        m
    }

    fn fv(name: &str, value: &str) -> FieldValues {
        fvs(vec![(name, json!(value))])
    }

    #[test]
    fn list_record_summaries_attaches_title_label() {
        let store = make_store_with_title_field();
        let record = create_record(
            &store,
            "type-labeled-0001",
            1,
            fv("title", "My Title"),
            None,
            None,
        )
        .expect("create record");

        let summaries =
            list_record_summaries(&store, RecordListFilter::default()).expect("list summaries");
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.display_label, "My Title");
        assert_eq!(s.instance_id, record.instance_id);
        assert_eq!(s.record.instance_id, record.instance_id);
    }

    #[test]
    fn list_record_summaries_falls_back_to_type_name() {
        let store = make_store_with_title_field();
        // Only the non-label `summary` field is set → no title/name/label match,
        // so the label falls back to the record's type_name.
        create_record(
            &store,
            "type-labeled-0001",
            1,
            fv("summary", "just a summary"),
            None,
            None,
        )
        .expect("create record");

        let summaries =
            list_record_summaries(&store, RecordListFilter::default()).expect("list summaries");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].display_label, "labeled-type");
    }

    #[test]
    fn list_record_summaries_respects_filter() {
        let store = make_store_with_title_field();
        create_record(&store, "type-labeled-0001", 1, fv("title", "A"), None, None).unwrap();
        create_record(&store, "type-labeled-0001", 1, fv("title", "B"), None, None).unwrap();

        // Matching type filter returns both; the same instance_ids list_records_filtered yields.
        let filter = RecordListFilter {
            type_namespace: Some("com.test".to_string()),
            type_name: Some("labeled-type".to_string()),
            container_id: None,
            tag: None,
        };
        let summaries = list_record_summaries(&store, filter.clone()).unwrap();
        let raw = list_records_filtered(&store, filter).unwrap();
        assert_eq!(summaries.len(), 2);
        let summary_ids: Vec<_> = summaries.iter().map(|s| s.instance_id.clone()).collect();
        let raw_ids: Vec<_> = raw.iter().map(|r| r.instance_id.clone()).collect();
        assert_eq!(
            summary_ids, raw_ids,
            "delegates filter to list_records_filtered"
        );

        // Non-matching type filter returns nothing.
        let none = list_record_summaries(
            &store,
            RecordListFilter {
                type_namespace: Some("com.test".to_string()),
                type_name: Some("nonexistent-type".to_string()),
                container_id: None,
                tag: None,
            },
        )
        .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn list_record_summaries_roundtrip_stores() {
        // Cross-store roundtrip (memory -> file) per CLAUDE.md Storage Boundary Rules,
        // mirroring container_view_service::resolve_container_view_roundtrip_stores.
        let store = make_store_with_title_field();
        create_record(
            &store,
            "type-labeled-0001",
            1,
            fv("title", "Roundtrip One"),
            None,
            None,
        )
        .unwrap();
        create_record(
            &store,
            "type-labeled-0001",
            1,
            fv("summary", "no title here"),
            None,
            None,
        )
        .unwrap();

        let from_memory =
            list_record_summaries(&store, RecordListFilter::default()).expect("memory summaries");

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = list_record_summaries(&file_store, RecordListFilter::default())
            .expect("file summaries");

        assert_eq!(from_memory.len(), 2, "fixture sanity: two records");
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "RecordSummary list must be identical across stores (memory -> file)"
        );
    }

    #[test]
    fn get_record_summary_by_id_returns_summary_with_label() {
        let store = make_store_with_title_field();
        let record = create_record(
            &store,
            "type-labeled-0001",
            1,
            fv("title", "My Summary Title"),
            None,
            None,
        )
        .unwrap();
        let summary = get_record_summary_by_id(&store, &record.instance_id)
            .expect("should not error")
            .expect("should find record");
        assert_eq!(summary.instance_id, record.instance_id);
        assert_eq!(summary.display_label, "My Summary Title");
        assert_eq!(summary.record.instance_id, record.instance_id);
    }

    #[test]
    fn get_record_summary_by_id_returns_none_for_unknown() {
        let store = make_store_with_title_field();
        let result = get_record_summary_by_id(&store, "00000000-0000-0000-0000-000000000000")
            .expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn get_record_summary_by_id_roundtrip_stores() {
        let store = make_store_with_title_field();
        let record = create_record(
            &store,
            "type-labeled-0001",
            1,
            fv("title", "Roundtrip Title"),
            None,
            None,
        )
        .unwrap();

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();

        let from_memory = get_record_summary_by_id(&store, &record.instance_id)
            .expect("memory lookup ok")
            .expect("should find in memory");
        let from_file = get_record_summary_by_id(&file_store, &record.instance_id)
            .expect("file lookup ok")
            .expect("should find in file");

        assert_eq!(from_memory.instance_id, from_file.instance_id);
        assert_eq!(from_memory.display_label, from_file.display_label);
        assert_eq!(from_memory.display_label, "Roundtrip Title");
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
        // RFC-038: the vendored spec-repo fixture is fatally invalid under the
        // catalog for happy-path assertions (srs-rust#783 Phase 3's known trap) —
        // a minimal in-memory store is a faithful, valid substitute here.
        let store = make_store_with_package();
        let result = get_record_by_id(&store, "00000000-0000-0000-0000-000000000000")
            .expect("should not error");
        assert!(result.is_none());
    }

    #[test]
    fn create_record_in_temp_repo() {
        let store = make_store_with_package();
        let field_values = fvs(vec![
            ("test-name", json!("Test Record")),
            ("test-status", json!("active")),
        ]);

        let record = create_record(&store, "type-test-001", 1, field_values, None, None)
            .expect("should create record");

        assert!(!record.instance_id.is_empty());
        assert_eq!(record.type_id, "type-test-001");

        // Record stored under slug-id8 path in the default dir
        let key = format!("records/tier-2/test-type-{}.json", &record.instance_id[..8]);
        store
            .load_instance_json(&key)
            .expect("should find stored record");

        // Discoverable via the catalog (RFC-038: membership comes from the tree).
        let cat = store.catalog().unwrap();
        let entry = cat.instances.iter().find(|e| e.id == record.instance_id);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().tier, Some(2));
    }

    #[test]
    fn create_record_uses_default_dir() {
        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fvs(vec![("test-name", json!("Default Dir Test"))]),
            None,
            None,
        )
        .expect("should create record");

        let cat = store.catalog().unwrap();
        let entry = cat
            .instances
            .iter()
            .find(|e| e.id == record.instance_id)
            .expect("record must be discoverable via the catalog");
        let path = entry.locator.as_deref().unwrap_or_default();
        assert!(
            path.starts_with("records/tier-2"),
            "expected path under records/tier-2, got {path}"
        );
    }

    #[test]
    fn create_record_missing_required_field_fails() {
        let store = make_store_with_package();
        let field_values = fvs(vec![("test-status", json!("active"))]);

        let result = create_record(&store, "type-test-001", 1, field_values, None, None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RepositoryError::RecordValidation { .. }
        ));
    }

    #[test]
    fn create_record_optional_field_absent_succeeds() {
        let store = make_store_with_package();
        let field_values = fvs(vec![("test-name", json!("Test Record"))]);

        let record = create_record(&store, "type-test-001", 1, field_values, None, None)
            .expect("should create with only required field");
        assert_eq!(record.field_values.len(), 1);
    }

    #[test]
    fn validate_record_input_accepts_valid() {
        let store = make_store_with_package();
        let report = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-name", json!("Valid Name"))]),
                tags: None,
            },
        )
        .expect("validate should not error");
        assert!(report.ok, "expected ok, got errors: {:?}", report.errors);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn validate_record_input_rejects_missing_required() {
        let store = make_store_with_package();
        // Only the optional status field — required name field is absent.
        let report = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-status", json!("active"))]),
                tags: None,
            },
        )
        .expect("validate should not error");
        assert!(!report.ok);
        assert!(!report.errors.is_empty(), "expected a diagnostic");
    }

    #[test]
    fn validate_record_input_rejects_unknown_field() {
        let store = make_store_with_package();
        let report = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![
                    ("test-name", json!("Valid Name")),
                    ("nonexistent-field", json!("stray")),
                ]),
                tags: None,
            },
        )
        .expect("validate should not error");
        assert!(!report.ok);
        assert!(!report.errors.is_empty(), "expected a diagnostic");
    }

    #[test]
    fn validate_record_input_collects_multiple_diagnostics() {
        // Input both omits the required "field-name-001" AND carries an unknown
        // field id. validate must report BOTH problems, not just the first (#111).
        let store = make_store_with_package();
        let report = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-test-001".to_string(),
                type_version: 1,
                // required "test-name" omitted; "nonexistent-field" is unknown
                field_values: fvs(vec![
                    ("test-status", json!("active")),
                    ("nonexistent-field", json!("stray")),
                ]),
                tags: None,
            },
        )
        .expect("validate should not error");
        assert!(!report.ok);
        assert!(
            report.errors.len() >= 2,
            "expected >= 2 diagnostics, got {}: {:?}",
            report.errors.len(),
            report.errors
        );
    }

    #[test]
    fn validate_record_input_rejects_unknown_type() {
        let store = make_store_with_package();
        let report = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-does-not-exist".to_string(),
                type_version: 1,
                field_values: FieldValues::new(),
                tags: None,
            },
        )
        .expect("validate should not error");
        assert!(!report.ok);
        assert!(
            report.errors.iter().any(|e| e.contains("type not found")),
            "expected a type-not-found diagnostic, got: {:?}",
            report.errors
        );
    }

    #[test]
    fn validate_record_input_does_not_write() {
        let store = make_store_with_package();
        let index_before = store.catalog().unwrap().instances.len();

        // Run a validation that fails (missing required) — must still write nothing.
        let _ = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-status", json!("active"))]),
                tags: None,
            },
        )
        .unwrap();

        // And one that passes — also writes nothing.
        let _ = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-name", json!("Valid Name"))]),
                tags: None,
            },
        )
        .unwrap();

        let index_after = store.catalog().unwrap().instances.len();
        assert_eq!(
            index_before, index_after,
            "validate must not add any instance index entries"
        );
    }

    #[test]
    fn record_update_validates_against_type() {
        let store = make_store_with_package();
        let initial_values = fvs(vec![
            ("test-name", json!("Initial Name")),
            ("test-status", json!("active")),
        ]);

        let record = create_record(&store, "type-test-001", 1, initial_values, None, None).unwrap();
        let instance_id = record.instance_id.clone();

        let updated_values = fvs(vec![
            ("test-name", json!("Updated Name")),
            ("test-status", json!("inactive")),
        ]);

        let updated = update_record(
            &store,
            &instance_id,
            UpdateRecordInput {
                field_meta: None,
                field_values: updated_values,
                tags: None,
                type_version: None,
            },
        )
        .unwrap();
        assert_eq!(updated.value("test-name"), Some(&json!("Updated Name")));

        // Verify stored value
        let key = format!("records/tier-2/test-type-{}.json", &instance_id[..8]);
        let stored_val = store.load_instance_json(&key).unwrap();
        let stored: Record = serde_json::from_value(stored_val).unwrap();
        assert_eq!(stored.value("test-name"), Some(&json!("Updated Name")));

        // Invalid update (missing required field)
        let invalid_values = fvs(vec![("test-status", json!("active"))]);
        assert!(update_record(
            &store,
            &instance_id,
            UpdateRecordInput {
                field_meta: None,
                field_values: invalid_values,
                tags: None,
                type_version: None,
            }
        )
        .is_err());
    }

    #[test]
    fn record_delete_cascades_incident_relations() {
        // RFC-038 Change F / [R22]: deleting an instance removes the Relations
        // incident to it in the same operation (replaces the former
        // CannotDeleteInUse refusal). Covers both storage forms: one incident
        // relation in the transitional collection, one as a standalone object.
        use crate::relation_service::load_relations;

        let store = make_store_with_package();

        let record_a = create_record(
            &store,
            "type-test-001",
            1,
            fvs(vec![("test-name", json!("Record A"))]),
            None,
            None,
        )
        .unwrap();

        let record_b = create_record(
            &store,
            "type-test-001",
            1,
            fvs(vec![("test-name", json!("Record B"))]),
            None,
            None,
        )
        .unwrap();

        // One incident relation as a standalone object…
        let rel_json = json!({
            "relations": [{
                "relationId": "eeeeeeee-0000-4000-8000-000000000021",
                "relationType": "depends-on",
                "sourceInstanceId": record_a.instance_id,
                "targetInstanceId": record_b.instance_id
            }]
        });
        crate::store::write_relations_standalone_for_test(&store, &rel_json);
        // …one incident standalone object, and one untouched bystander edge.
        store
            .save_relation(&srs_core::types::relation::Relation {
                relation_id: "dc000002-0000-4000-a000-000000000002".to_string(),
                relation_type: "refines".to_string(),
                source_instance_id: record_b.instance_id.clone(),
                target_instance_id: record_a.instance_id.clone(),
                asserted_by: None,
                confidence: None,
                created_at: None,
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            })
            .unwrap();
        store
            .save_relation(&srs_core::types::relation::Relation {
                relation_id: "dc000003-0000-4000-a000-000000000003".to_string(),
                relation_type: "refines".to_string(),
                source_instance_id: record_a.instance_id.clone(),
                target_instance_id: record_a.instance_id.clone(),
                asserted_by: None,
                confidence: None,
                created_at: None,
                created_by: None,
                status: None,
                valid_from: None,
                valid_until: None,
                notes: None,
                source_refs: None,
                meta: None,
                source_repository_id: None,
                target_repository_id: None,
            })
            .unwrap();

        delete_record(&store, &record_b.instance_id).unwrap();

        // Both incident relations are gone; the bystander survives.
        let remaining = load_relations(&store).unwrap();
        assert_eq!(
            remaining
                .iter()
                .map(|r| r.relation_id.as_str())
                .collect::<Vec<_>>(),
            vec!["dc000003-0000-4000-a000-000000000003"],
            "cascade must remove exactly the incident relations"
        );

        // No remaining relation names the deleted instance (the validate-level
        // no-E2-diagnostic proof lives in validation::tests::
        // validate_no_dangling_endpoint_after_cascade_delete, on FileStore).
        assert!(
            !remaining
                .iter()
                .any(|r| r.source_instance_id == record_b.instance_id
                    || r.target_instance_id == record_b.instance_id),
            "no remaining relation may reference the deleted instance"
        );
    }

    #[test]
    fn record_delete_succeeds_when_no_relations_reference_it() {
        let store = make_store_with_package();

        let record = create_record(
            &store,
            "type-test-001",
            1,
            fvs(vec![("test-name", json!("Isolated Record"))]),
            None,
            None,
        )
        .unwrap();

        delete_record(&store, &record.instance_id).unwrap();
    }

    #[test]
    fn record_delete_removes_file_and_catalog_entry() {
        let store = make_store_with_package();
        let field_values = fvs(vec![
            ("test-name", json!("Test Name")),
            ("test-status", json!("active")),
        ]);

        let record = create_record(&store, "type-test-001", 1, field_values, None, None).unwrap();
        let instance_id = record.instance_id.clone();
        let key = format!("records/tier-2/test-type-{}.json", &instance_id[..8]);

        assert!(store.load_instance_json(&key).is_ok());

        let deleted_id = delete_record(&store, &instance_id).unwrap();
        assert_eq!(deleted_id, instance_id);

        assert!(store.load_instance_json(&key).is_err());

        // Not writing manifest.json ([R22]): the manifest never had an index
        // entry to begin with — absence from the catalog is the real check.
        let cat = store.catalog().unwrap();
        assert!(cat.instances.iter().all(|e| e.id != instance_id));
    }

    fn make_store_with_lifecycle() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{
            FieldAssignment, LifecycleState, LifecycleTransition, RecordType, TypeLifecycle,
        };
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        let title_field = Field {
            schema: None,
            id: "field-title-lc".to_string(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Title".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let lc_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
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
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
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
                        requires_relation: None,
                        meta: None,
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
                        requires_relation: None,
                        meta: None,
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
                        requires_relation: None,
                        meta: None,
                    },
                ],
                transitions: vec![
                    LifecycleTransition {
                        id: None,
                        name: "promote".to_string(),
                        from: "draft".to_string(),
                        to: "active".to_string(),
                        description: None,
                        meta: None,
                    },
                    LifecycleTransition {
                        id: None,
                        name: "archive".to_string(),
                        from: "active".to_string(),
                        to: "archived".to_string(),
                        description: None,
                        meta: None,
                    },
                ],
                initial_state: "draft".to_string(),
            }),
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
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
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
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
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };

        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            package_dependencies: vec![],
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
            fvs(vec![("title", json!("Test Item"))]),
            None,
            None,
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
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("active"));
        assert!(result.warnings.is_empty());

        // rfc-decision-2a1e1590 / srs-rust#866: a lifecycle transition used to
        // best-effort append one Revision per field here — the mechanism's
        // one production write site. It must write no `.revisions.json`
        // sidecar any more.
        assert!(
            store
                .list_files_recursive("")
                .iter()
                .all(|p| !p.ends_with(".revisions.json")),
            "a transition must not write a .revisions.json sidecar"
        );
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
                fulfillment: None,
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
                fulfillment: None,
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
                fulfillment: None,
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
                fulfillment: None,
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
                field_values: fvs(vec![("title", json!("Updated Item"))]),
                lifecycle_state: None,
                type_version: None,
            },
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
    fn create_record_successor_unknown_relation_type_rejected_no_write() {
        let store = make_store_with_lifecycle();
        let predecessor = create_lc_record(&store);
        let before = store.catalog().unwrap().instances.len();

        let result = create_record_successor(
            &store,
            &predecessor.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "not-a-real-type".to_string(),
                field_values: fvs(vec![("title", json!("Should Fail"))]),
                lifecycle_state: None,
                type_version: None,
            },
        );

        assert!(
            matches!(result, Err(RepositoryError::RelationValidation { .. })),
            "expected RelationValidation error, got: {:?}",
            result
        );
        // No orphaned record: instance index must not have grown.
        let after = store.catalog().unwrap().instances.len();
        assert_eq!(
            after, before,
            "instance index grew — successor record was written despite unknown relation type"
        );
    }

    /// Creates a store identical to `make_store_with_lifecycle` but replaces the `supersedes`
    /// relation type definition with one carrying the given status override.
    fn make_store_with_supersedes_status(
        status: Option<srs_core::types::relation_type_definition::RelationTypeStatus>,
    ) -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{
            FieldAssignment, LifecycleState, LifecycleTransition, RecordType, TypeLifecycle,
        };
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        let title_field = Field {
            schema: None,
            id: "field-title-lc".to_string(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Title".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let lc_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
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
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
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
                        requires_relation: None,
                        meta: None,
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
                        requires_relation: None,
                        meta: None,
                    },
                ],
                transitions: vec![LifecycleTransition {
                    id: None,
                    name: "promote".to_string(),
                    from: "draft".to_string(),
                    to: "active".to_string(),
                    description: None,
                    meta: None,
                }],
                initial_state: "draft".to_string(),
            }),
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
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
            require_same_type: None,
            status,
            updated_at: None,
            meta: None,
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-lc".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package-lc".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![title_field],
            record_types: vec![lc_type],
            relation_type_definitions: vec![supersedes_def],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(manifest, package)
    }

    #[test]
    fn create_record_successor_retired_relation_type_rejected_no_write() {
        use srs_core::types::relation_type_definition::RelationTypeStatus;
        let store = make_store_with_supersedes_status(Some(RelationTypeStatus::Retired));
        let predecessor = create_lc_record(&store);
        let before = store.catalog().unwrap().instances.len();

        let result = create_record_successor(
            &store,
            &predecessor.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Retired Type"))]),
                lifecycle_state: None,
                type_version: None,
            },
        );

        assert!(
            matches!(result, Err(RepositoryError::RelationValidation { .. })),
            "expected RelationValidation error for retired type, got: {:?}",
            result
        );
        let after = store.catalog().unwrap().instances.len();
        assert_eq!(
            after, before,
            "orphaned record written for retired relation type"
        );
    }

    #[test]
    fn create_record_successor_deprecated_relation_type_rejected_no_write() {
        use srs_core::types::relation_type_definition::RelationTypeStatus;
        let store = make_store_with_supersedes_status(Some(RelationTypeStatus::Deprecated));
        let predecessor = create_lc_record(&store);
        let before = store.catalog().unwrap().instances.len();

        let result = create_record_successor(
            &store,
            &predecessor.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Deprecated Type"))]),
                lifecycle_state: None,
                type_version: None,
            },
        );

        assert!(
            matches!(result, Err(RepositoryError::RelationValidation { .. })),
            "expected RelationValidation error for deprecated type, got: {:?}",
            result
        );
        let after = store.catalog().unwrap().instances.len();
        assert_eq!(
            after, before,
            "orphaned record written for deprecated relation type"
        );
    }

    #[test]
    fn create_record_successor_tombstone_relation_type_rejected_no_write() {
        use srs_core::types::relation_type_definition::RelationTypeStatus;
        let store = make_store_with_supersedes_status(Some(RelationTypeStatus::Tombstone));
        let predecessor = create_lc_record(&store);
        let before = store.catalog().unwrap().instances.len();

        let result = create_record_successor(
            &store,
            &predecessor.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Tombstone Type"))]),
                lifecycle_state: None,
                type_version: None,
            },
        );

        assert!(
            matches!(result, Err(RepositoryError::RelationValidation { .. })),
            "expected RelationValidation error for tombstone type, got: {:?}",
            result
        );
        let after = store.catalog().unwrap().instances.len();
        assert_eq!(
            after, before,
            "orphaned record written for tombstone relation type"
        );
    }

    #[test]
    fn create_record_successor_conflicting_rtds_rejected_no_write() {
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        // Two `supersedes` definitions with different UUIDs → E1Conflict
        let def_a = RelationTypeDefinition {
            schema: None,
            id: "rtd-supersedes-aaa".to_string(),
            version: 1,
            key: "supersedes".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            label: "Supersedes A".to_string(),
            description: "First definition".to_string(),
            category: RelationTypeCategory::Refinement,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: Some(true),
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };
        let def_b = RelationTypeDefinition {
            id: "rtd-supersedes-bbb".to_string(),
            ..def_a.clone()
        };

        let manifest = crate::manifest::Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        // Reuse lc_type from make_store_with_lifecycle via a fresh MemoryStore from the helper
        // then swap the package — easiest: build package inline with two conflicting defs.
        let base = make_store_with_lifecycle();
        let mut base_pkg = base.load_package().unwrap();
        base_pkg.relation_type_definitions = vec![def_a, def_b];
        let store = MemoryStore::new(manifest, base_pkg);

        let predecessor = create_lc_record(&store);
        let before = store.catalog().unwrap().instances.len();

        let result = create_record_successor(
            &store,
            &predecessor.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Conflict Type"))]),
                lifecycle_state: None,
                type_version: None,
            },
        );

        assert!(
            matches!(result, Err(RepositoryError::RelationValidation { .. })),
            "expected RelationValidation error for conflicting RTDs, got: {:?}",
            result
        );
        let after = store.catalog().unwrap().instances.len();
        assert_eq!(
            after, before,
            "orphaned record written for conflicting RTDs"
        );
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
                fulfillment: None,
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
                field_values: fvs(vec![("title", json!("Next Version"))]),
                lifecycle_state: None,
                type_version: None,
            },
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

    // fieldMeta write path tests (RFC-039 Change C — successor of the retired
    // group_values / per-value provenance write paths)

    fn meta_for(name: &str, source: &str) -> indexmap::IndexMap<String, FieldMeta> {
        let mut m = indexmap::IndexMap::new();
        m.insert(
            name.to_string(),
            FieldMeta {
                source: Some(source.to_string()),
                ..Default::default()
            },
        );
        m
    }

    #[test]
    fn create_record_with_field_meta_persists_meta() {
        let store = make_store_with_package();

        let field_values = fvs(vec![("test-name", json!("Record With Meta"))]);
        let record = create_record(
            &store,
            "type-test-001",
            1,
            field_values,
            Some(meta_for("test-name", "human")),
            None,
        )
        .expect("should create record with fieldMeta");

        let loaded = get_record_by_id(&store, &record.instance_id)
            .unwrap()
            .expect("should load record");

        let meta = loaded.field_meta.expect("fieldMeta should be persisted");
        assert_eq!(meta["test-name"].source.as_deref(), Some("human"));
    }

    #[test]
    fn update_record_with_field_meta_replaces_meta() {
        let store = make_store_with_package();

        let fv = fvs(vec![("test-name", json!("Initial"))]);
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv,
            Some(meta_for("test-name", "human")),
            None,
        )
        .expect("create");
        let id = record.instance_id.clone();

        let new_fv = fvs(vec![("test-name", json!("Updated"))]);
        update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_values: new_fv,
                field_meta: Some(meta_for("test-name", "ai")),
                tags: None,
                type_version: None,
            },
        )
        .expect("update");

        let loaded = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(loaded.value("test-name"), Some(&json!("Updated")));
        let meta = loaded.field_meta.expect("fieldMeta replaced on update");
        assert_eq!(meta["test-name"].source.as_deref(), Some("ai"));
    }

    #[test]
    fn update_record_without_field_meta_preserves_existing() {
        let store = make_store_with_package();

        let fv = fvs(vec![("test-name", json!("With Meta"))]);
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv,
            Some(meta_for("test-name", "human")),
            None,
        )
        .expect("create");
        let id = record.instance_id.clone();

        // None = not supplied, preserve existing
        let new_fv = fvs(vec![("test-name", json!("Field Only Update"))]);
        update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: new_fv,
                tags: None,
                type_version: None,
            },
        )
        .expect("update");

        let loaded = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(loaded.value("test-name"), Some(&json!("Field Only Update")));
        assert!(
            loaded.field_meta.is_some(),
            "fieldMeta preserved when not supplied"
        );
    }

    fn make_record_in_store(store: &MemoryStore) -> String {
        let fv = fvs(vec![("test-name", json!("Tagged Record"))]);
        create_record(store, "type-test-001", 1, fv, None, None)
            .expect("create")
            .instance_id
    }

    #[test]
    fn add_record_tag_adds_and_mirrors_to_catalog() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        let result = add_record_tag(&store, &id, "construct:field").expect("add tag");
        assert!(matches!(result, AddRecordTagResult::Added { .. }));

        // Record body has the tag
        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(record.tags, Some(vec!["construct:field".to_string()]));

        // Catalog-derived InstanceRef reflects the same tag (RFC-038: no index columns).
        let entry = store
            .find_instance(&id)
            .unwrap()
            .expect("found via catalog");
        assert_eq!(entry.tags, vec!["construct:field".to_string()]);
    }

    #[test]
    fn add_record_tag_idempotent() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        add_record_tag(&store, &id, "construct:field").expect("first add");
        let result = add_record_tag(&store, &id, "construct:field").expect("second add");
        assert!(matches!(result, AddRecordTagResult::AlreadyPresent { .. }));

        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(record.tags.as_deref().unwrap_or(&[]).len(), 1);
    }

    #[test]
    fn remove_record_tag_removes_and_mirrors_to_catalog() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        add_record_tag(&store, &id, "construct:field").expect("add");
        let result = remove_record_tag(&store, &id, "construct:field").expect("remove");
        assert!(matches!(result, RemoveRecordTagResult::Removed { .. }));

        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert!(record.tags.is_none());

        let entry = store
            .find_instance(&id)
            .unwrap()
            .expect("found via catalog");
        assert!(entry.tags.is_empty());
    }

    #[test]
    fn remove_record_tag_not_present() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        let result = remove_record_tag(&store, &id, "construct:field").expect("remove");
        assert!(matches!(result, RemoveRecordTagResult::NotPresent { .. }));
    }

    #[test]
    fn add_remove_record_tag_not_found() {
        let store = make_store_with_package();

        let add = add_record_tag(&store, "no-such-id", "t").expect("add");
        assert!(matches!(add, AddRecordTagResult::NotFound));

        let remove = remove_record_tag(&store, "no-such-id", "t").expect("remove");
        assert!(matches!(remove, RemoveRecordTagResult::NotFound));
    }

    #[test]
    fn update_record_preserves_tags() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        add_record_tag(&store, &id, "concern:lifecycle").expect("add tag");

        // Update field values — tags must survive
        let new_fv = fvs(vec![("test-name", json!("Updated Name"))]);
        update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: new_fv,
                tags: None,
                type_version: None,
            },
        )
        .expect("update");

        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(record.tags, Some(vec!["concern:lifecycle".to_string()]));
    }

    #[test]
    fn list_record_tags_counts_correctly() {
        let store = make_store_with_package();

        let id1 = make_record_in_store(&store);
        let id2 = make_record_in_store(&store);

        add_record_tag(&store, &id1, "construct:field").unwrap();
        add_record_tag(&store, &id1, "layer:normative").unwrap();
        add_record_tag(&store, &id2, "construct:field").unwrap();

        let result = list_record_tags(&store, None).expect("list");
        assert_eq!(result.total_records, 2);

        let construct_entry = result.tags.iter().find(|e| e.tag == "construct:field");
        assert_eq!(construct_entry.map(|e| e.record_count), Some(2));

        let layer_entry = result.tags.iter().find(|e| e.tag == "layer:normative");
        assert_eq!(layer_entry.map(|e| e.record_count), Some(1));
    }

    #[test]
    fn list_records_filtered_by_tag() {
        let store = make_store_with_package();

        let id1 = make_record_in_store(&store);
        let id2 = make_record_in_store(&store);

        add_record_tag(&store, &id1, "construct:type").unwrap();

        let tagged = list_records_filtered(
            &store,
            RecordListFilter {
                tag: Some("construct:type".to_string()),
                ..Default::default()
            },
        )
        .expect("list");

        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].instance_id, id1);

        let _ = id2; // not tagged — should not appear
    }

    #[test]
    fn create_record_with_tags_persists_tags_in_record_and_manifest() {
        let store = make_store_with_package();

        let fv = fvs(vec![("test-name", json!("Tagged on Create"))]);

        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv,
            None,
            Some(vec![
                "construct:field".to_string(),
                "layer:normative".to_string(),
            ]),
        )
        .expect("should create record with tags");

        // Tags are in the returned record
        assert_eq!(
            record.tags,
            Some(vec![
                "construct:field".to_string(),
                "layer:normative".to_string()
            ])
        );

        // Tags are persisted in the record body
        let loaded = get_record_by_id(&store, &record.instance_id)
            .unwrap()
            .expect("should load record");
        assert_eq!(
            loaded.tags,
            Some(vec![
                "construct:field".to_string(),
                "layer:normative".to_string()
            ])
        );

        // Tags are mirrored into the catalog-derived InstanceRef
        let entry = store
            .find_instance(&record.instance_id)
            .unwrap()
            .expect("found via catalog");
        assert_eq!(
            entry.tags,
            vec!["construct:field".to_string(), "layer:normative".to_string()]
        );
    }

    #[test]
    fn create_record_with_empty_tags_has_no_tags() {
        let store = make_store_with_package();

        let fv = fvs(vec![("test-name", json!("No Tags"))]);

        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv,
            None,
            Some(vec![]), // explicitly empty — normalised to None
        )
        .expect("should create record");

        assert!(record.tags.is_none());
    }

    #[test]
    fn update_record_with_none_tags_preserves_existing_tags() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        add_record_tag(&store, &id, "concern:lifecycle").expect("add tag");

        // Update with tags: None → preserve existing
        let fv = fvs(vec![("test-name", json!("Updated"))]);
        update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: fv,
                tags: None,
                type_version: None,
            },
        )
        .expect("update");

        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(record.tags, Some(vec!["concern:lifecycle".to_string()]));
    }

    #[test]
    fn update_record_with_empty_tags_clears_tags() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        add_record_tag(&store, &id, "concern:lifecycle").expect("add tag");

        // Update with tags: Some([]) → clear all tags
        let fv = fvs(vec![("test-name", json!("Updated"))]);
        update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: fv,
                tags: Some(vec![]),
                type_version: None,
            },
        )
        .expect("update");

        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert!(record.tags.is_none());

        // Catalog-derived InstanceRef also cleared
        let entry = store
            .find_instance(&id)
            .unwrap()
            .expect("found via catalog");
        assert!(entry.tags.is_empty());
    }

    #[test]
    fn update_record_with_new_tags_replaces_existing_tags() {
        let store = make_store_with_package();
        let id = make_record_in_store(&store);

        add_record_tag(&store, &id, "old-tag").expect("add old tag");

        // Update with Some([new]) → replace
        let fv = fvs(vec![("test-name", json!("Updated"))]);
        update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: fv,
                tags: Some(vec!["new-tag-1".to_string(), "new-tag-2".to_string()]),
                type_version: None,
            },
        )
        .expect("update");

        let record = get_record_by_id(&store, &id).unwrap().unwrap();
        assert_eq!(
            record.tags,
            Some(vec!["new-tag-1".to_string(), "new-tag-2".to_string()])
        );

        // Catalog-derived InstanceRef updated
        let entry = store
            .find_instance(&id)
            .unwrap()
            .expect("found via catalog");
        assert_eq!(
            entry.tags,
            vec!["new-tag-1".to_string(), "new-tag-2".to_string()]
        );
    }

    // ── lifecycleRef write-path regression tests ───────────────────────────────

    fn make_store_with_lifecycle_ref() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::lifecycle::{Lifecycle, LifecycleState, LifecycleTransition};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        let title_field = Field {
            schema: None,
            id: "field-title-lcref".to_string(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Title".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        // Standalone lifecycle referenced by UUID.
        let standalone_lc = Lifecycle {
            schema: None,
            tags: None,
            id: "lc-ref-standalone-001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "item-lifecycle".to_string(),
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
                    requires_relation: None,
                    meta: None,
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
                    requires_relation: None,
                    meta: None,
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
                    requires_relation: None,
                    meta: None,
                },
            ],
            transitions: vec![
                LifecycleTransition {
                    id: None,
                    name: "promote".to_string(),
                    from: "draft".to_string(),
                    to: "active".to_string(),
                    description: None,
                    meta: None,
                },
                LifecycleTransition {
                    id: None,
                    name: "archive".to_string(),
                    from: "active".to_string(),
                    to: "archived".to_string(),
                    description: None,
                    meta: None,
                },
            ],
            initial_state: "draft".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        // RecordType binds lifecycle via lifecycleRef; inline lifecycle is None.
        let lcref_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-lc-ref-001".to_string(),
            namespace: "com.test".to_string(),
            name: "lifecycle-ref-type".to_string(),
            version: 1,
            description: "Type with lifecycleRef".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-title-lcref".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: Some("lc-ref-standalone-001".to_string()),
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };

        let supersedes_def = RelationTypeDefinition {
            schema: None,
            id: "rtd-supersedes-lcref".to_string(),
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
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };

        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-lcref".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package-lcref".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![title_field],
            record_types: vec![lcref_type],
            relation_type_definitions: vec![supersedes_def],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![standalone_lc],
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(manifest, package)
    }

    fn create_lc_ref_record(store: &MemoryStore) -> Record {
        create_record(
            store,
            "type-lc-ref-001",
            1,
            fvs(vec![("title", json!("Test Item"))]),
            None,
            None,
        )
        .unwrap()
    }

    #[test]
    fn create_record_with_lifecycle_ref_sets_initial_state() {
        let store = make_store_with_lifecycle_ref();
        let record = create_lc_ref_record(&store);
        assert_eq!(record.lifecycle_state.as_deref(), Some("draft"));
    }

    #[test]
    fn transition_with_lifecycle_ref_succeeds() {
        let store = make_store_with_lifecycle_ref();
        let record = create_lc_ref_record(&store);
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: None,
                by_transition: Some("promote".to_string()),
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("active"));
    }

    // ── RFC-022: relational states + transition fulfillment ────────────────────

    /// Governance-style lifecycle: draft → ratified → {superseded | closed},
    /// where `superseded` declares `requiresRelation: {relationType: "supersedes"}`
    /// (incoming by default). `unreachable-state` is defined but has no path
    /// from the initial state.
    fn make_store_with_relational_state() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::lifecycle::{
            Lifecycle, LifecycleState, LifecycleTransition, RelationTypeSpec, RequiresRelation,
        };
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };

        fn state(key: &str) -> LifecycleState {
            LifecycleState {
                id: None,
                version: None,
                namespace: None,
                key: key.to_string(),
                label: None,
                description: None,
                aliases: None,
                is_initial: None,
                is_final: None,
                status: None,
                requires_relation: None,
                meta: None,
            }
        }
        fn transition(name: &str, from: &str, to: &str) -> LifecycleTransition {
            LifecycleTransition {
                id: None,
                name: name.to_string(),
                from: from.to_string(),
                to: to.to_string(),
                description: None,
                meta: None,
            }
        }

        let title_field = Field {
            schema: None,
            id: "field-title-rfc022".to_string(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Title".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let mut draft = state("draft");
        draft.is_initial = Some(true);
        let ratified = state("ratified");
        let mut superseded = state("superseded");
        superseded.is_final = Some(true);
        superseded.requires_relation = Some(RequiresRelation {
            enforcement: None,
            relation_type: RelationTypeSpec::One("supersedes".to_string()),
            direction: None, // incoming by default
        });
        let mut closed = state("closed");
        closed.is_final = Some(true);
        let unreachable = state("unreachable-state");

        let gov_lc = Lifecycle {
            schema: None,
            tags: None,
            id: "lc-rfc022-001".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "governance-lifecycle".to_string(),
            states: vec![draft, ratified, superseded, closed, unreachable],
            transitions: vec![
                transition("propose", "draft", "ratified"),
                transition("supersede", "ratified", "superseded"),
                transition("close", "ratified", "closed"),
            ],
            initial_state: "draft".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let gov_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-rfc022-001".to_string(),
            namespace: "com.test".to_string(),
            name: "decision".to_string(),
            version: 1,
            description: "Type with relational superseded state".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-title-rfc022".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: Some("lc-rfc022-001".to_string()),
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };

        let supersedes_def = RelationTypeDefinition {
            schema: None,
            id: "rtd-supersedes-rfc022".to_string(),
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
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };

        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-rfc022".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package-rfc022".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![title_field],
            record_types: vec![gov_type],
            relation_type_definitions: vec![supersedes_def],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![gov_lc],
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(manifest, package)
    }

    fn create_rfc022_record(store: &dyn RepositoryStore, title: &str) -> Record {
        create_record(
            store,
            "type-rfc022-001",
            1,
            fvs(vec![("title", json!(title))]),
            None,
            None,
        )
        .unwrap()
    }

    fn ratify(store: &dyn RepositoryStore, instance_id: &str) {
        transition_record_lifecycle(
            store,
            instance_id,
            TransitionLifecycleInput {
                to: None,
                by_transition: Some("propose".to_string()),
                fulfillment: None,
            },
        )
        .unwrap();
    }

    fn supersede_input(
        fulfillment: Option<TransitionFulfillmentInput>,
    ) -> TransitionLifecycleInput {
        TransitionLifecycleInput {
            to: None,
            by_transition: Some("supersede".to_string()),
            fulfillment,
        }
    }

    #[test]
    fn rfc022_bare_transition_into_relational_state_rejected() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let err = transition_record_lifecycle(&store, &record.instance_id, supersede_input(None))
            .unwrap_err();
        match err {
            RepositoryError::LifecycleRelationRequired {
                state,
                relation_types,
                direction,
            } => {
                assert_eq!(state, "superseded");
                assert_eq!(relation_types, vec!["supersedes".to_string()]);
                assert_eq!(direction, "incoming");
            }
            other => panic!("expected LifecycleRelationRequired, got: {other:?}"),
        }
        // The rejected flip must not have been committed.
        let reloaded = get_record_by_id(&store, &record.instance_id)
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.lifecycle_state.as_deref(), Some("ratified"));
    }

    #[test]
    fn rfc022_bare_transition_allowed_when_obligation_satisfied() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        // Two-phase workflow: draft a successor first (asserts supersedes successor→predecessor)…
        create_record_successor(
            &store,
            &record.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Decision 1 v2"))]),
                lifecycle_state: None,
                type_version: None,
            },
        )
        .unwrap();

        // …then the bare flip is legal because the obligation is already satisfied.
        let result =
            transition_record_lifecycle(&store, &record.instance_id, supersede_input(None))
                .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("superseded"));
        assert!(result.successor.is_none());
        assert!(result.relation.is_none());
    }

    #[test]
    fn rfc022_fulfillment_new_record_is_atomic_supersede() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            supersede_input(Some(TransitionFulfillmentInput {
                new_record: Some(FulfillmentNewRecord {
                    field_values: fvs(vec![("title", json!("Decision 1 v2"))]),
                    type_version: None,
                }),
                existing_instance_id: None,
                relation_type: None,
            })),
        )
        .unwrap();

        assert_eq!(result.record.lifecycle_state.as_deref(), Some("superseded"));
        let successor = result
            .successor
            .expect("fulfillment must return the successor");
        assert_eq!(successor.lifecycle_state.as_deref(), Some("draft"));
        let relation = result
            .relation
            .expect("fulfillment must return the relation");
        assert_eq!(relation.relation_type, "supersedes");
        assert_eq!(relation.source_instance_id, successor.instance_id);
        assert_eq!(relation.target_instance_id, record.instance_id);

        // The relation is persisted, not just reported.
        let rels = relation_service::list_relations(
            &store,
            relation_service::ListRelationsFilter {
                target: Some(record.instance_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rels.len(), 1);
    }

    #[test]
    fn rfc022_fulfillment_existing_instance_adopts_drafted_successor() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);
        let drafted = create_rfc022_record(&store, "Decision 1 v2");

        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            supersede_input(Some(TransitionFulfillmentInput {
                new_record: None,
                existing_instance_id: Some(drafted.instance_id.clone()),
                relation_type: None,
            })),
        )
        .unwrap();

        assert_eq!(result.record.lifecycle_state.as_deref(), Some("superseded"));
        assert!(result.successor.is_none(), "no new record was spawned");
        let relation = result.relation.expect("relation must be returned");
        assert_eq!(relation.source_instance_id, drafted.instance_id);
        assert_eq!(relation.target_instance_id, record.instance_id);
    }

    #[test]
    fn rfc022_fulfillment_rejected_on_non_relational_target() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let err = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: None,
                by_transition: Some("close".to_string()),
                fulfillment: Some(TransitionFulfillmentInput {
                    new_record: None,
                    existing_instance_id: Some("whatever".to_string()),
                    relation_type: None,
                }),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::LifecycleFulfillmentNotApplicable { ref state } if state == "closed"),
            "got: {err:?}"
        );
    }

    #[test]
    fn rfc022_fulfillment_relation_type_mismatch_rejected() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let err = transition_record_lifecycle(
            &store,
            &record.instance_id,
            supersede_input(Some(TransitionFulfillmentInput {
                new_record: None,
                existing_instance_id: Some("whatever".to_string()),
                relation_type: Some("refines".to_string()),
            })),
        )
        .unwrap_err();
        assert!(
            matches!(
                err,
                RepositoryError::LifecycleFulfillmentRelationTypeMismatch { ref relation_type, .. }
                    if relation_type == "refines"
            ),
            "got: {err:?}"
        );
    }

    #[test]
    fn rfc022_fulfillment_requires_exactly_one_mode() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let err = transition_record_lifecycle(
            &store,
            &record.instance_id,
            supersede_input(Some(TransitionFulfillmentInput {
                new_record: None,
                existing_instance_id: None,
                relation_type: None,
            })),
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn rfc022_allowed_transitions_expose_requires_relation() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let result = get_allowed_lifecycle_transitions(&store, &record.instance_id).unwrap();
        let supersede = result
            .transitions
            .iter()
            .find(|t| t.name == "supersede")
            .expect("supersede option present");
        let req = supersede
            .requires_relation
            .as_ref()
            .expect("supersede target must expose requiresRelation");
        assert_eq!(req.relation_type.types(), vec!["supersedes"]);
        let close = result
            .transitions
            .iter()
            .find(|t| t.name == "close")
            .expect("close option present");
        assert!(
            close.requires_relation.is_none(),
            "close target declares no obligation"
        );
    }

    #[test]
    fn rfc022_successor_explicit_state_undefined_rejected() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        let err = create_record_successor(
            &store,
            &record.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: Some("ghost".to_string()),
                type_version: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::LifecycleStateNotDefined { ref state } if state == "ghost"),
            "got: {err:?}"
        );
    }

    #[test]
    fn rfc022_successor_explicit_state_unreachable_rejected() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        let err = create_record_successor(
            &store,
            &record.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: FieldValues::new(),
                lifecycle_state: Some("unreachable-state".to_string()),
                type_version: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::LifecycleStateUnreachable { ref state, .. } if state == "unreachable-state"),
            "got: {err:?}"
        );
    }

    #[test]
    fn rfc022_successor_explicit_reachable_state_ok() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        let result = create_record_successor(
            &store,
            &record.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Decision 1 v2"))]),
                lifecycle_state: Some("ratified".to_string()),
                type_version: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("ratified"));
    }

    // ── srs-rust#880: retrofit entry for a pre-existing record ─────────────────
    //
    // Reproduces the exact repro from #880: a record created before its Type
    // carried any lifecycle (simulated here by clearing `lifecycle_state` back to
    // `None` after creation — a `lifecycleRef` added retroactively leaves exactly
    // this state on every pre-existing record). `record transition` must now give
    // such a record a path to its first `lifecycleState`.

    /// Simulate a record that predates its Type's lifecycle: create it normally
    /// (which auto-assigns the initial state), then strip the state back to
    /// `None`, matching what a retroactive `lifecycleRef` bind leaves behind.
    fn retrofit_record_without_lifecycle_state(store: &dyn RepositoryStore, title: &str) -> Record {
        let mut record = create_rfc022_record(store, title);
        assert!(
            record.lifecycle_state.is_some(),
            "precondition: create_record must auto-assign an initial state"
        );
        record.lifecycle_state = None;
        store.save_record(&record).unwrap();
        record
    }

    #[test]
    fn retrofit_880_bare_to_initial_state_from_absent_state_ok() {
        let store = make_store_with_relational_state();
        let record = retrofit_record_without_lifecycle_state(&store, "RFC pre-existing");

        // Exact #880 repro: `to: "draft"` (the Lifecycle's own initialState) was
        // refused because no transition is declared `from: ""`.
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("draft".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("draft"));
    }

    #[test]
    fn retrofit_880_direct_entry_to_reachable_non_initial_state_ok() {
        let store = make_store_with_relational_state();
        let record = retrofit_record_without_lifecycle_state(&store, "RFC pre-existing");

        // "ratified" is two hops from "draft" (draft -> ratified via `propose`) and
        // carries no relation obligation — a retrofit migration lands a record
        // directly on its already-known state, it does not replay the graph walk.
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("ratified".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("ratified"));
    }

    #[test]
    fn retrofit_880_direct_entry_to_reachable_non_initial_state_roundtrips_via_filestore() {
        // Cross-store roundtrip (memory -> file) per CLAUDE.md Storage Boundary Rules.
        let store = make_store_with_relational_state();
        let record = retrofit_record_without_lifecycle_state(&store, "RFC pre-existing");

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();

        // Same repro as retrofit_880_direct_entry_to_reachable_non_initial_state_ok,
        // against the file store: "ratified" is reachable-but-not-initial from
        // "draft" and carries no relation obligation.
        let result = transition_record_lifecycle(
            &file_store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("ratified".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("ratified"));
    }

    #[test]
    fn retrofit_880_direct_entry_to_unreachable_state_rejected() {
        let store = make_store_with_relational_state();
        let record = retrofit_record_without_lifecycle_state(&store, "RFC pre-existing");

        let err = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("unreachable-state".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::LifecycleStateUnreachable { ref state, .. } if state == "unreachable-state"),
            "got: {err:?}"
        );
    }

    #[test]
    fn retrofit_880_direct_entry_to_relational_state_without_fulfillment_rejected() {
        let store = make_store_with_relational_state();
        let record = retrofit_record_without_lifecycle_state(&store, "RFC pre-existing");

        // RFC-022's requiresRelation gate must still apply on retrofit entry: no
        // supersedes relation exists, so a bare jump straight to "superseded" fails.
        let err = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("superseded".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::LifecycleRelationRequired { ref state, .. } if state == "superseded"),
            "got: {err:?}"
        );
        // Rejected — the record must still have no lifecycleState.
        let reloaded = get_record_by_id(&store, &record.instance_id)
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.lifecycle_state, None);
    }

    #[test]
    fn retrofit_880_direct_entry_to_relational_state_with_fulfillment_ok() {
        // srs#447's exact residual case: RFC-004's rfc_status is already
        // "superseded" (by RFC-033) in prose, with no `supersedes` relation ever
        // asserted. The retrofit migration must be able to land the predecessor
        // record directly in "superseded" while fulfilling the RFC-022 obligation
        // in the same call — `fulfillment.existingInstanceId` pointing at the
        // successor, exactly as for an ordinary supersede transition.
        let store = make_store_with_relational_state();
        let predecessor = retrofit_record_without_lifecycle_state(&store, "RFC-004");
        let successor = create_rfc022_record(&store, "RFC-033");

        let result = transition_record_lifecycle(
            &store,
            &predecessor.instance_id,
            TransitionLifecycleInput {
                to: Some("superseded".to_string()),
                by_transition: None,
                fulfillment: Some(TransitionFulfillmentInput {
                    new_record: None,
                    existing_instance_id: Some(successor.instance_id.clone()),
                    relation_type: None,
                }),
            },
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("superseded"));

        let rels = relation_service::list_relations(
            &store,
            relation_service::ListRelationsFilter {
                source: Some(successor.instance_id.clone()),
                target: Some(predecessor.instance_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relation_type, "supersedes");
    }

    #[test]
    fn rfc022_successor_explicit_relational_state_unsatisfied_rejected_and_rolled_back() {
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        // The successor's own relation is OUTGOING supersedes; the obligation on
        // `superseded` is INCOMING — so planting the successor directly in
        // `superseded` must be rejected (the pre-RFC-022 back door, srs-rust#502).
        let err = create_record_successor(
            &store,
            &record.instance_id,
            CreateRecordSuccessorInput {
                relation_type: "supersedes".to_string(),
                field_values: fvs(vec![("title", json!("Decision 1 v2"))]),
                lifecycle_state: Some("superseded".to_string()),
                type_version: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::LifecycleRelationRequired { .. }),
            "got: {err:?}"
        );
        // Rolled back: no relation to the predecessor survives.
        let rels = relation_service::list_relations(
            &store,
            relation_service::ListRelationsFilter {
                target: Some(record.instance_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(rels.is_empty(), "fulfillment artifacts must be rolled back");
    }

    #[test]
    fn rfc022_fulfillment_roundtrip_stores() {
        // Cross-store roundtrip (memory -> file) per CLAUDE.md Storage Boundary Rules.
        let store = make_store_with_relational_state();
        let record = create_rfc022_record(&store, "Decision 1");
        ratify(&store, &record.instance_id);

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();

        // The bare flip is rejected identically on the file store…
        let err =
            transition_record_lifecycle(&file_store, &record.instance_id, supersede_input(None))
                .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::LifecycleRelationRequired { .. }
        ));

        // …and the fulfilled transition succeeds against the file store.
        let result = transition_record_lifecycle(
            &file_store,
            &record.instance_id,
            supersede_input(Some(TransitionFulfillmentInput {
                new_record: Some(FulfillmentNewRecord {
                    field_values: fvs(vec![("title", json!("Decision 1 v2"))]),
                    type_version: None,
                }),
                existing_instance_id: None,
                relation_type: None,
            })),
        )
        .unwrap();
        assert_eq!(result.record.lifecycle_state.as_deref(), Some("superseded"));
        let successor = result.successor.unwrap();
        let reloaded = get_record_by_id(&file_store, &successor.instance_id)
            .unwrap()
            .expect("successor persisted on file store");
        assert_eq!(reloaded.lifecycle_state.as_deref(), Some("draft"));
        let rels = relation_service::list_relations(
            &file_store,
            relation_service::ListRelationsFilter {
                target: Some(record.instance_id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relation_type, "supersedes");
    }

    // -------------------------------------------------------------------------
    // UpdateRecordInput / type-version migration tests
    // -------------------------------------------------------------------------

    /// Returns a store whose package contains type "type-test-001" at **both**
    /// version 1 (namespace "com.test", name "test-type") and version 2
    /// (namespace "com.test.v2", name "test-type-v2").  Both versions share the
    /// same "field-name-001" required field, so the same field_values are valid
    /// against either version.
    fn make_store_with_two_type_versions() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{FieldAssignment, RecordType};

        let name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let field_assignment = FieldAssignment {
            field_id: "field-name-001".to_string(),
            order: 0,
            required: true,
            display_label: Some("Name".to_string()),
            description: None,
        };
        let type_v1 = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type v1".to_string(),
            fields: vec![field_assignment.clone()],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let type_v2 = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-test-001".to_string(),
            namespace: "com.test.v2".to_string(),
            name: "test-type-v2".to_string(),
            version: 2,
            description: "Test type v2".to_string(),
            fields: vec![field_assignment],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field],
            record_types: vec![type_v1, type_v2],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    #[test]
    fn record_update_allows_type_version_migration() {
        let store = make_store_with_two_type_versions();
        let fv = fvs(vec![("test-name", json!("Original Name"))]);
        let record = create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let id = record.instance_id.clone();

        assert_eq!(record.type_version, 1);
        assert_eq!(record.type_namespace, "com.test");
        assert_eq!(record.type_name, "test-type");

        let new_fv = fvs(vec![("test-name", json!("Migrated Name"))]);
        let updated = update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: new_fv,
                tags: None,
                type_version: Some(2),
            },
        )
        .expect("migration to v2 should succeed");

        assert_eq!(updated.type_version, 2, "type_version must be updated");
        assert_eq!(
            updated.type_namespace, "com.test.v2",
            "type_namespace must reflect v2"
        );
        assert_eq!(
            updated.type_name, "test-type-v2",
            "type_name must reflect v2"
        );
        assert_eq!(updated.value("test-name"), Some(&json!("Migrated Name")));
    }

    #[test]
    fn record_update_preserves_version_when_not_specified() {
        let store = make_store_with_package();
        let fv = fvs(vec![
            ("test-name", json!("Original")),
            ("test-status", json!("active")),
        ]);
        let record = create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let id = record.instance_id.clone();

        let new_fv = fvs(vec![
            ("test-name", json!("Updated")),
            ("test-status", json!("inactive")),
        ]);
        let updated = update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: new_fv,
                tags: None,
                type_version: None,
            },
        )
        .expect("update without type_version should preserve stored version");

        assert_eq!(
            updated.type_version, 1,
            "type_version must be preserved when not specified"
        );
        assert_eq!(updated.type_namespace, "com.test");
        assert_eq!(updated.type_name, "test-type");
    }

    #[test]
    fn record_update_fails_on_invalid_incoming_version() {
        let store = make_store_with_package();
        let fv = fvs(vec![
            ("test-name", json!("Name")),
            ("test-status", json!("active")),
        ]);
        let record = create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let new_fv = fvs(vec![
            ("test-name", json!("Updated")),
            ("test-status", json!("active")),
        ]);
        let result = update_record(
            &store,
            &record.instance_id,
            UpdateRecordInput {
                field_meta: None,
                field_values: new_fv,
                tags: None,
                type_version: Some(99),
            },
        );

        assert!(
            matches!(
                result,
                Err(RepositoryError::TypeVersionNotFound { version: 99, .. })
            ),
            "updating with a nonexistent type version must fail with TypeVersionNotFound"
        );
    }

    #[test]
    fn record_update_type_version_migration_roundtrip_stores() {
        let store = make_store_with_two_type_versions();
        let fv = fvs(vec![("test-name", json!("Initial"))]);
        let record = create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let id = record.instance_id.clone();

        let new_fv = fvs(vec![("test-name", json!("Migrated"))]);
        let updated = update_record(
            &store,
            &id,
            UpdateRecordInput {
                field_meta: None,
                field_values: new_fv,
                tags: None,
                type_version: Some(2),
            },
        )
        .expect("migration to v2 should succeed");

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store)
            .expect("copy to FileStore");

        let reloaded = get_record_by_id(&file_store, &id)
            .expect("get_record_by_id succeeded")
            .expect("record found in FileStore");

        assert_eq!(
            reloaded.type_version, updated.type_version,
            "type_version must survive JSON round-trip"
        );
        assert_eq!(
            reloaded.type_name, updated.type_name,
            "type_name must survive JSON round-trip"
        );
        assert_eq!(
            reloaded.type_namespace, updated.type_namespace,
            "type_namespace must survive JSON round-trip"
        );
    }

    #[test]
    fn lifecycle_transition_roundtrip_stores() {
        // Cross-store roundtrip: memory → multi-step transitions → file, verify lifecycle_state
        // survives JSON serialization. Covers: initial state, happy-path transition (no warnings),
        // final-state transition (LIFECYCLE_FINAL_STATE warning), and persistence to FileStore.
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        assert_eq!(record.lifecycle_state.as_deref(), Some("draft"));

        // draft → active (non-final state, no warnings expected)
        let r1 = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("active".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(r1.record.lifecycle_state.as_deref(), Some("active"));
        assert!(
            r1.warnings.is_empty(),
            "no warnings expected for non-final transition"
        );

        // active → archived (final state — LIFECYCLE_FINAL_STATE warning expected)
        let r2 = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("archived".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        assert_eq!(r2.record.lifecycle_state.as_deref(), Some("archived"));
        assert!(
            !r2.warnings.is_empty(),
            "expected LIFECYCLE_FINAL_STATE warning for final-state transition"
        );
        assert!(
            r2.warnings[0].contains("LIFECYCLE_FINAL_STATE"),
            "warning should contain LIFECYCLE_FINAL_STATE: {}",
            r2.warnings[0]
        );

        // Copy to FileStore and verify final state survives JSON round-trip
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store)
            .expect("copy to FileStore");

        let reloaded = get_record_by_id(&file_store, &record.instance_id)
            .expect("lookup succeeded")
            .expect("record found in FileStore");
        assert_eq!(
            reloaded.lifecycle_state.as_deref(),
            Some("archived"),
            "lifecycle_state must survive JSON round-trip"
        );
    }

    #[test]
    fn transition_with_lifecycle_ref_invalid_transition_fails() {
        let store = make_store_with_lifecycle_ref();
        let record = create_lc_ref_record(&store);
        // draft → archived is not a defined transition
        let result = transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("archived".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        );
        assert!(matches!(
            result,
            Err(RepositoryError::LifecycleTransitionNotAllowed { .. })
        ));
    }

    // ── get_allowed_lifecycle_transitions ────────────────────────────────────

    #[test]
    fn allowed_transitions_from_draft_returns_correct_options() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        let result = get_allowed_lifecycle_transitions(&store, &record.instance_id).unwrap();
        assert_eq!(result.current_state, "draft");
        assert!(!result.is_immutable);
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(result.transitions[0].name, "promote");
        assert_eq!(result.transitions[0].to, "active");
        assert!(!result.transitions[0].to_is_final);
    }

    #[test]
    fn allowed_transitions_from_active_returns_correct_options() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("active".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        let result = get_allowed_lifecycle_transitions(&store, &record.instance_id).unwrap();
        assert_eq!(result.current_state, "active");
        assert!(!result.is_immutable);
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(result.transitions[0].name, "archive");
        assert_eq!(result.transitions[0].to, "archived");
        assert!(result.transitions[0].to_is_final);
    }

    #[test]
    fn allowed_transitions_from_final_state_returns_immutable_empty() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);
        transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("active".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        transition_record_lifecycle(
            &store,
            &record.instance_id,
            TransitionLifecycleInput {
                to: Some("archived".to_string()),
                by_transition: None,
                fulfillment: None,
            },
        )
        .unwrap();
        let result = get_allowed_lifecycle_transitions(&store, &record.instance_id).unwrap();
        assert_eq!(result.current_state, "archived");
        assert!(result.is_immutable);
        assert!(result.transitions.is_empty());
    }

    #[test]
    fn allowed_transitions_record_not_found_returns_error() {
        let store = make_store_with_lifecycle();
        let result =
            get_allowed_lifecycle_transitions(&store, "00000000-0000-0000-0000-000000000000");
        assert!(matches!(result, Err(RepositoryError::NotFound { .. })));
    }

    #[test]
    fn allowed_transitions_with_lifecycle_ref() {
        let store = make_store_with_lifecycle_ref();
        let record = create_lc_ref_record(&store);
        let result = get_allowed_lifecycle_transitions(&store, &record.instance_id).unwrap();
        // make_store_with_lifecycle_ref creates a type with a draft→active lifecycle via ref
        assert_eq!(result.current_state, "draft");
        assert!(!result.is_immutable);
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(result.transitions[0].name, "promote");
    }

    #[test]
    fn allowed_transitions_roundtrip_file_store() {
        let store = make_store_with_lifecycle();
        let record = create_lc_record(&store);

        // Copy to FileStore
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store)
            .expect("copy to FileStore");

        let result = get_allowed_lifecycle_transitions(&file_store, &record.instance_id).unwrap();
        assert_eq!(result.current_state, "draft");
        assert!(!result.is_immutable);
        assert_eq!(result.transitions.len(), 1);
        assert_eq!(result.transitions[0].name, "promote");
    }

    // ── create_record_in_container ────────────────────────────────────────────

    fn make_container_in_store(store: &MemoryStore) -> String {
        use crate::container_service;
        use srs_core::types::container::Container;
        let c = Container {
            container_id: "550e8400-e29b-41d4-a716-446655440001".to_string(),
            title: "Test Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        container_service::create_container(store, c)
            .expect("container created")
            .container_id
    }

    #[test]
    fn create_record_in_container_adds_to_membership() {
        let store = make_store_with_package();
        let container_id = make_container_in_store(&store);

        let result = create_record_in_container(
            &store,
            CreateRecordInContainerInput {
                field_meta: None,
                container_id: container_id.clone(),
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-name", json!("My Decision"))]),
                tags: None,
            },
        )
        .expect("should create record");

        assert!(!result.record.instance_id.is_empty());

        let members =
            crate::container_service::list_members(&store, &container_id).expect("members loaded");
        assert!(
            members.contains(&result.record.instance_id),
            "record must be a member of the container"
        );
    }

    #[test]
    fn create_record_in_container_missing_container_fails() {
        let store = make_store_with_package();
        let initial_len = store.catalog().unwrap().instances.len();

        let result = create_record_in_container(
            &store,
            CreateRecordInContainerInput {
                field_meta: None,
                container_id: "does-not-exist".to_string(),
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-name", json!("Should Not Exist"))]),
                tags: None,
            },
        );

        assert!(matches!(
            result,
            Err(RepositoryError::ContainerNotFound { .. })
        ));

        let after_len = store.catalog().unwrap().instances.len();
        assert_eq!(
            initial_len, after_len,
            "manifest index must be unchanged after early error"
        );
    }

    #[test]
    fn create_record_in_container_invalid_type_fails() {
        let store = make_store_with_package();
        let container_id = make_container_in_store(&store);
        let initial_len = store.catalog().unwrap().instances.len();

        let result = create_record_in_container(
            &store,
            CreateRecordInContainerInput {
                field_meta: None,
                container_id: container_id.clone(),
                type_id: "type-does-not-exist".to_string(),
                type_version: 1,
                field_values: FieldValues::new(),
                tags: None,
            },
        );

        assert!(matches!(result, Err(RepositoryError::TypeNotFound { .. })));

        let after_len = store.catalog().unwrap().instances.len();
        assert_eq!(
            initial_len, after_len,
            "manifest index must be unchanged after type error"
        );
    }

    #[test]
    fn create_record_in_container_roundtrip_stores() {
        let store = make_store_with_package();
        let container_id = make_container_in_store(&store);

        let result = create_record_in_container(
            &store,
            CreateRecordInContainerInput {
                field_meta: None,
                container_id: container_id.clone(),
                type_id: "type-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("test-name", json!("Roundtrip Decision"))]),
                tags: None,
            },
        )
        .expect("memory store create should succeed");

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store)
            .expect("copy to file store");

        let reloaded = get_record_by_id(&file_store, &result.record.instance_id)
            .expect("record must exist in file store")
            .expect("record must be Some");
        assert_eq!(reloaded.instance_id, result.record.instance_id);
        assert_eq!(reloaded.type_id, "type-test-001");

        let members = crate::container_service::list_members(&file_store, &container_id)
            .expect("members loaded from file store");
        assert!(
            members.contains(&result.record.instance_id),
            "record must be a member in the file store copy"
        );
    }

    // ── rollback mechanism ─────────────────────────────────────────────────────

    #[test]
    fn rollback_mechanism_delete_record_cleans_manifest() {
        // Verifies the building blocks used by the best-effort rollback in
        // create_record_in_container / create_record_in_context: that
        // create_record_at_dir followed by delete_record returns the manifest
        // instance index to its original length. This is the two-step sequence
        // the rollback error arm executes when add_member fails.
        //
        // Note: the error-path trigger (that add_member failure invokes this
        // sequence) is verified by code inspection of the match arm; fault-
        // injection integration testing is deferred (see ADR-024).
        let store = make_store_with_package();
        let initial_len = store.catalog().unwrap().instances.len();

        let record = create_record_at_dir(
            &store,
            "type-test-001",
            1,
            fvs(vec![("test-name", json!("Rollback Test"))]),
            None,
            None,
            "records/tier-2",
        )
        .expect("create should succeed");

        let after_create_len = store.catalog().unwrap().instances.len();
        assert_eq!(
            after_create_len,
            initial_len + 1,
            "manifest must have one more entry after create"
        );

        delete_record(&store, &record.instance_id).expect("delete should succeed");

        let after_delete_len = store.catalog().unwrap().instances.len();
        assert_eq!(
            after_delete_len, initial_len,
            "manifest must return to its original length after rollback delete"
        );
    }

    #[test]
    fn create_record_in_context_container_branch_success_unaffected() {
        // Regression test: the container-branch success path of create_record_in_context
        // must continue to work after the rollback error arm was added. Record is created,
        // manifest grows by one, and the record is a member of the container.
        let store = make_store_with_package();
        let container_id = make_container_in_store(&store);
        let initial_len = store.catalog().unwrap().instances.len();

        let result = create_record_in_context(
            &store,
            "com.test/test-type",
            None,
            CreateRecordInput {
                field_meta: None,
                field_values: fvs(vec![("test-name", json!("Context Success"))]),
                tags: None,
            },
            Some(container_id.clone()),
            None,
        )
        .expect("create_record_in_context should succeed on valid type and container");

        let after_len = store.catalog().unwrap().instances.len();
        assert_eq!(
            after_len,
            initial_len + 1,
            "manifest must have one more entry after successful create_record_in_context"
        );

        let members =
            crate::container_service::list_members(&store, &container_id).expect("members loaded");
        assert!(
            members.contains(&result.record.instance_id),
            "record must be a member of the container after create_record_in_context"
        );
    }

    #[test]
    fn create_record_in_context_container_branch_roundtrip_stores() {
        // Cross-store coverage for the container branch of create_record_in_context
        // (required by CLAUDE.md: "New service features need at least one cross-store roundtrip test").
        let store = make_store_with_package();
        let container_id = make_container_in_store(&store);

        let result = create_record_in_context(
            &store,
            "com.test/test-type",
            None,
            CreateRecordInput {
                field_meta: None,
                field_values: fvs(vec![("test-name", json!("Roundtrip Context"))]),
                tags: None,
            },
            Some(container_id.clone()),
            None,
        )
        .expect("create_record_in_context should succeed on MemoryStore");

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store)
            .expect("copy to file store");

        let reloaded = get_record_by_id(&file_store, &result.record.instance_id)
            .expect("record must be loadable from file store")
            .expect("record must be Some");
        assert_eq!(reloaded.instance_id, result.record.instance_id);

        let members = crate::container_service::list_members(&file_store, &container_id)
            .expect("members loaded from file store");
        assert!(
            members.contains(&result.record.instance_id),
            "record must be a member of the container in the file store copy"
        );
    }

    // ---------------------------------------------------------------------------
    // CFR (CrossFieldRule) write-boundary tests — ext:cross-field-validation
    // These tests verify that validate_cross_field_rules is enforced at write time
    // in both create_record_at_dir and update_record, and in validate_record_input.
    // ---------------------------------------------------------------------------

    /// Build a MemoryStore whose package contains:
    /// - field-trigger-001 (String) — predicate field
    /// - field-target-001  (String) — target field
    /// - cfr-test-type v1 with a ConditionalRequired rule:
    ///   when field-trigger-001 == "active", field-target-001 is required
    fn make_store_with_cfr_package() -> MemoryStore {
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use srs_core::types::record_type::{
            CrossFieldRule, CrossFieldRuleKind, FieldAssignment, RecordType,
        };

        let trigger_field = Field {
            schema: None,
            id: "field-trigger-001".to_string(),
            namespace: "com.test".to_string(),
            name: "trigger".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Trigger field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let target_field = Field {
            schema: None,
            id: "field-target-001".to_string(),
            namespace: "com.test".to_string(),
            name: "target".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Target field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let cfr_rule = CrossFieldRule {
            rule_type: CrossFieldRuleKind::ConditionalRequired,
            message: None,
            predicate_field_id: Some("field-trigger-001".to_string()),
            predicate_value: Some("active".to_string()),
            target_field_id: Some("field-target-001".to_string()),
            effect: None,
            field_ids: None,
        };
        let cfr_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-cfr-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "cfr-test-type".to_string(),
            version: 1,
            description: "Type with a ConditionalRequired CFR".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "field-trigger-001".to_string(),
                    order: 0,
                    required: false,
                    display_label: None,
                    description: None,
                },
                FieldAssignment {
                    field_id: "field-target-001".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: Some(vec![cfr_rule]),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-cfr-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "cfr-test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![trigger_field, target_field],
            record_types: vec![cfr_type],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
    }

    #[test]
    fn cfr_create_rejects_violating_record() {
        // trigger="active" but target absent → ConditionalRequired is violated → Err
        let store = make_store_with_cfr_package();
        let result = create_record_at_dir(
            &store,
            "type-cfr-test-001",
            1,
            fvs(vec![("trigger", json!("active"))]),
            None,
            None,
            "records",
        );
        assert!(
            matches!(result, Err(RepositoryError::RecordValidation { .. })),
            "create must be rejected when a CFR is violated, got: {:?}",
            result
        );
    }

    #[test]
    fn cfr_create_accepts_satisfying_record() {
        // trigger="active" AND target present → rule satisfied → Ok
        let store = make_store_with_cfr_package();
        let result = create_record_at_dir(
            &store,
            "type-cfr-test-001",
            1,
            fvs(vec![("trigger", json!("active")), ("target", json!("x"))]),
            None,
            None,
            "records",
        );
        assert!(
            result.is_ok(),
            "create must succeed when CFR is satisfied, got: {:?}",
            result
        );
    }

    #[test]
    fn cfr_create_accepts_when_predicate_not_triggered() {
        // trigger absent → ConditionalRequired predicate is not met → rule not triggered → Ok
        let store = make_store_with_cfr_package();
        let result = create_record_at_dir(
            &store,
            "type-cfr-test-001",
            1,
            FieldValues::new(),
            None,
            None,
            "records",
        );
        assert!(
            result.is_ok(),
            "create must succeed when CFR predicate is not triggered, got: {:?}",
            result
        );
    }

    #[test]
    fn cfr_update_rejects_violating_record() {
        // Create a valid record (no trigger), then update to trigger=active without target → Err
        let store = make_store_with_cfr_package();
        let created = create_record_at_dir(
            &store,
            "type-cfr-test-001",
            1,
            FieldValues::new(),
            None,
            None,
            "records",
        )
        .expect("initial create should succeed");

        let result = update_record(
            &store,
            &created.instance_id,
            UpdateRecordInput {
                field_meta: None,
                type_version: None,
                field_values: fvs(vec![("trigger", json!("active"))]),
                tags: None,
            },
        );
        assert!(
            matches!(result, Err(RepositoryError::RecordValidation { .. })),
            "update must be rejected when a CFR is violated, got: {:?}",
            result
        );
    }

    #[test]
    fn cfr_validate_input_reports_violation() {
        // validate_record_input with trigger=active and no target → report.ok == false
        let store = make_store_with_cfr_package();
        let report = validate_record_input(
            &store,
            ValidateRecordInput {
                field_meta: None,
                type_id: "type-cfr-test-001".to_string(),
                type_version: 1,
                field_values: fvs(vec![("trigger", json!("active"))]),
                tags: None,
            },
        )
        .expect("validate_record_input must not return a service error");
        assert!(
            !report.ok,
            "preflight report must be !ok when a CFR is violated"
        );
        assert!(
            !report.errors.is_empty(),
            "preflight report must include at least one error"
        );
    }

    // --- Fault-injection test: RFC-038 [R22] delete semantics ---
    //
    // ADR-007's index-first delete ordering is retired: `delete_record` no longer
    // writes `manifest.json` at all ([R22] — srs-rust#783 Phase 3), so there is no
    // index to dangle. Membership comes from the tree ([R1]); the only remaining
    // question is whether the entity file itself was actually removed.

    fn minimal_field_values() -> FieldValues {
        fvs(vec![("test-name", json!("Fault Test Record"))])
    }

    #[test]
    fn delete_record_of_a_non_member_never_touches_manifest() {
        // Arm a manifest-write fault that would have fired under the retired
        // index-first ordering. delete_record must still succeed, because it
        // no longer calls save_manifest on the routine path.
        //
        // Scope: this record is not a member of the inline root container. A
        // record that *is* one does write manifest.json — the explicit
        // container-membership operation Change F sanctions (srs-rust#834) —
        // covered by tests/delete_membership_cascade.rs.
        use crate::store::memory::FailPoint;

        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            minimal_field_values(),
            None,
            None,
        )
        .unwrap();
        let instance_id = record.instance_id.clone();

        store.arm_fail_at(FailPoint::SaveManifest);
        let result = delete_record(&store, &instance_id);
        assert!(
            result.is_ok(),
            "a routine delete must not touch manifest.json, so an armed \
             SaveManifest fault must never fire ([R22])"
        );
        assert!(store.find_instance(&instance_id).unwrap().is_none());
    }

    /// srs-rust#839: `delete_record`'s own file delete is best-effort no
    /// longer — a swallowed failure here, after the #834 cascade already ran,
    /// would leave the instance on disk stripped of every container reference
    /// and incident relation, reported as deleted. So the failure must
    /// surface, and the partial state must be re-runnable to completion.
    ///
    /// Used to also cover a revision sidecar delete that ran before this one
    /// (two best-effort deletes, two fault-injection variants). That call is
    /// removed (rfc-decision-2a1e1590, srs-rust#866): it was unreachable —
    /// any `.revisions.json` in the tree already fails `store.catalog()`
    /// above ([R24]), so `delete_record` never reached it while one existed.
    /// One delete remains; one variant is enough.
    #[test]
    fn delete_record_instance_file_fail_surfaces_and_is_re_runnable() {
        use crate::store::memory::FailPoint;

        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            minimal_field_values(),
            None,
            None,
        )
        .unwrap();
        let instance_id = record.instance_id.clone();

        let record_path = store
            .catalog()
            .unwrap()
            .instances
            .iter()
            .find(|e| e.id == instance_id)
            .and_then(|e| e.locator.clone())
            .expect("the new record has a catalog locator");

        store.arm_fail_at(FailPoint::DeleteInstanceFileAt(record_path.clone()));
        let err = delete_record(&store, &instance_id)
            .map(|ok| panic!("expected {record_path} to fail the delete, got Ok({ok})"))
            .unwrap_err();
        assert!(
            matches!(err, RepositoryError::Io { .. }),
            "expected the underlying I/O failure for {record_path}, got: {err:?}"
        );

        // The record file survived the failed delete, so the instance is
        // still discoverable ([R1]: membership is the tree), which is what
        // makes the re-run possible.
        assert!(
            store.find_instance(&instance_id).unwrap().is_some(),
            "the record must remain discoverable after a failed delete"
        );

        // Re-run: the earlier steps are idempotent no-ops and the delete completes.
        delete_record(&store, &instance_id).expect("re-running the delete must complete it");
        assert!(
            store.find_instance(&instance_id).unwrap().is_none(),
            "the record must be gone after the successful re-run"
        );
    }

    // ── get_field_value_by_name tests ─────────────────────────────────────────

    #[test]
    fn get_field_value_by_name_returns_value() {
        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv("test-name", "Hello World"),
            None,
            None,
        )
        .expect("create record");

        let result = get_field_value_by_name(
            &store,
            GetFieldValueByNameInput {
                instance_id: record.instance_id.clone(),
                field_name: "test-name".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert_eq!(
            result.value,
            Some(json!("Hello World")),
            "value must match the stored field value"
        );
    }

    #[test]
    fn get_field_value_by_name_returns_none_for_unknown_field() {
        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv("test-name", "Hello"),
            None,
            None,
        )
        .expect("create record");

        let result = get_field_value_by_name(
            &store,
            GetFieldValueByNameInput {
                instance_id: record.instance_id.clone(),
                field_name: "nonexistent-field".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert!(
            result.value.is_none(),
            "unknown field name must return None, not an error"
        );
    }

    #[test]
    fn get_field_value_by_name_returns_none_for_unknown_record() {
        let store = make_store_with_package();

        let result = get_field_value_by_name(
            &store,
            GetFieldValueByNameInput {
                instance_id: "00000000-0000-0000-0000-000000000000".to_string(),
                field_name: "test-name".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert!(
            result.value.is_none(),
            "unknown instance_id must return None, not an error"
        );
    }

    #[test]
    fn get_field_value_by_name_returns_none_for_missing_field_value() {
        // Create a record that only sets the required "test-name" field,
        // leaving the optional "test-status" field unset.
        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv("test-name", "Hello"),
            None,
            None,
        )
        .expect("create record");

        let result = get_field_value_by_name(
            &store,
            GetFieldValueByNameInput {
                instance_id: record.instance_id.clone(),
                field_name: "test-status".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert!(
            result.value.is_none(),
            "field in schema but not set on record must return None"
        );
    }

    #[test]
    fn get_field_value_by_name_cross_store_roundtrip() {
        // CLAUDE.md Storage Boundary Rules: new service features need at least one
        // cross-store roundtrip test (memory → file).
        let store = make_store_with_package();
        let record = create_record(
            &store,
            "type-test-001",
            1,
            fv("test-name", "Roundtrip Value"),
            None,
            None,
        )
        .expect("create record in MemoryStore");

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store)
            .expect("copy to FileStore");

        let from_memory = get_field_value_by_name(
            &store,
            GetFieldValueByNameInput {
                instance_id: record.instance_id.clone(),
                field_name: "test-name".to_string(),
            },
        )
        .expect("memory store lookup");

        let from_file = get_field_value_by_name(
            &file_store,
            GetFieldValueByNameInput {
                instance_id: record.instance_id.clone(),
                field_name: "test-name".to_string(),
            },
        )
        .expect("file store lookup");

        assert_eq!(
            from_memory.value, from_file.value,
            "get_field_value_by_name must return identical values across MemoryStore and FileStore"
        );
        assert_eq!(
            from_file.value,
            Some(json!("Roundtrip Value")),
            "value must match what was written"
        );
    }

    #[test]
    fn write_record_includes_schema_header() {
        use srs_core::types::record::Record;

        let store = make_store_with_package();

        let record = Record {
            field_meta: None,
            instance_id: "aaaabbbb-0000-4000-8000-000000000001".to_string(),
            type_id: "type-test-001".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "test-type".to_string(),
            field_values: fvs(vec![("test-name", json!("schema-test"))]),
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        };

        let path = "records/tier-2/test-type-aaaabbbb.json";
        write_record(&store, &record, path).expect("write_record must succeed");

        let val = store
            .load_instance_json(path)
            .expect("stored file must be loadable");

        assert_eq!(
            val.get("$schema").and_then(|v| v.as_str()),
            Some(RECORD_SCHEMA_ID),
            "write_record must stamp the $schema key (ADR-004)"
        );
    }
}
