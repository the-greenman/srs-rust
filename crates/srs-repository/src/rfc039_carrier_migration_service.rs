//! RFC-039 carrier migration — data-model migration #2 (revision 1 → 2).
//!
//! Rewrites every Tier-2 `fieldValues` array into the name-keyed object
//! carrier, converts each `FieldGroup` into an inline-composite Field over a
//! minted range Type (Change E.2), strips the deprecated
//! `FieldAssignment.{repeatable,minItems,maxItems}` trio, replaces Tier-1
//! `TypedField.valueType` with an inline `fieldType` ([R8]), carries theme
//! `groupFieldRowTemplates` keys to `compositeFieldRowTemplates` ([R12]), and
//! stamps `dataModelRevision: 2`.
//!
//! Boundary rules (ADR-043 §3): the transform operates on **raw
//! `serde_json::Value` documents** through `RepositoryStore`'s generic-JSON
//! methods — never `Vfs`, never typed entities (the revision-2 typed layer
//! rejects revision-1 documents by [R9], and would silently drop the trio the
//! transform must read). The whole run is wrapped in the ADR-021 batch seam:
//! any abort rolls the store back, so no repository is ever half-migrated
//! ([R13]).
//!
//! Every schema-legal input has an explicit disposition (RFC-039 "the
//! transform must be total"): abort rather than skip, with two logged-notice
//! exceptions (valueless pairs; `FieldGroupEntry.entryId`), and the [R20]
//! dual-write rule (take `value`, assert the `entries` projection agrees).

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Revision this migration produces. RFC-033 numbering: RFC-032 was #1.
pub const CARRIER_REVISION: u64 = 2;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CarrierMigrationResult {
    pub from_revision: u64,
    pub to_revision: u64,
    pub instances_migrated: usize,
    pub tier2_records: usize,
    pub tier1_records: usize,
    pub values_rewritten: usize,
    /// Minted definition entities (Change E.2), `"kind name uuid"` per line —
    /// the audit log RFC-039 requires for reproducibility.
    pub minted: Vec<String>,
    /// Keys omitted for valueless pairs — the first declared
    /// non-round-trippable class.
    pub valueless_pairs_omitted: Vec<String>,
    /// Dual-written `value`/`entries` pairs collapsed ([R20]) — the second
    /// declared non-round-trippable class.
    pub dual_writes_collapsed: usize,
    /// `FieldGroupEntry.entryId`s dropped with a logged notice.
    pub entry_ids_dropped: Vec<String>,
    /// Theme keys carried `groupFieldRowTemplates` → `compositeFieldRowTemplates` ([R12]).
    pub theme_keys_carried: usize,
}

/// Abort with a diagnostic naming the document and the reason ([R10]/[R13]:
/// never skip, never coerce, never partially migrate).
fn abort(path: &str, reason: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::InvalidSnapshotData {
        message: format!("carrier migration aborted at {path}: {reason}"),
    }
}

/// Definition index built from the **raw** post-Phase-0 package files.
struct DefinitionIndex {
    /// field id → (name, raw fieldType)
    fields: HashMap<String, (String, Value)>,
    /// (type id, version) → ordered assignments (field_id, required)
    types: HashMap<(String, u64), Vec<(String, bool)>>,
    /// (type id, old version) → new version, for Types bumped by Change E.2.
    version_bumps: HashMap<(String, u64), u64>,
}

impl DefinitionIndex {
    fn field_name(&self, field_id: &str) -> Option<&str> {
        self.fields.get(field_id).map(|(n, _)| n.as_str())
    }
    fn field_type(&self, field_id: &str) -> Option<&Value> {
        self.fields.get(field_id).map(|(_, ft)| ft)
    }
}

/// Is this migration needed? Structural [R9] test over the manifest revision.
pub fn migration_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    Ok(crate::field_type_migration_service::data_model_revision(store)? < CARRIER_REVISION)
}

pub fn migrate_carrier(
    store: &dyn RepositoryStore,
) -> Result<CarrierMigrationResult, RepositoryError> {
    let from_revision = crate::field_type_migration_service::data_model_revision(store)?;

    // Sequencing guard (srs-rust#809): the carrier transform keys every step
    // on `Field.fieldType`; a revision-0 repository's definitions still carry
    // `valueType`, so running here would stamp revision 2 over pre-RFC-032
    // definitions — the inconsistent artifact RFC-039's migration ordering
    // exists to prevent. RFC-032's migration must run first.
    if from_revision < crate::field_type_migration_service::FIELD_TYPE_REVISION {
        return Err(RepositoryError::InvalidSnapshotData {
            message: format!(
                "carrier migration requires data-model revision >= 1 (found {from_revision}):                  run `srs repo apply-migration --id field-type` first (RFC-032, migration #1)"
            ),
        });
    }

    store.begin_batch();
    let result = run_migration(store, from_revision);
    match result {
        Ok(r) => {
            store.commit_batch()?;
            Ok(r)
        }
        Err(e) => {
            // ADR-021: an abort leaves no store half-migrated ([R13]).
            store.abort_batch();
            Err(e)
        }
    }
}

fn run_migration(
    store: &dyn RepositoryStore,
    from_revision: u64,
) -> Result<CarrierMigrationResult, RepositoryError> {
    let mut result = CarrierMigrationResult {
        from_revision,
        to_revision: CARRIER_REVISION,
        instances_migrated: 0,
        tier2_records: 0,
        tier1_records: 0,
        values_rewritten: 0,
        minted: Vec::new(),
        valueless_pairs_omitted: Vec::new(),
        dual_writes_collapsed: 0,
        entry_ids_dropped: Vec::new(),
        theme_keys_carried: 0,
    };

    // ── Phase 0 — definitions (whole repository, before any instance) ──
    let index = migrate_definitions(store, &mut result)?;

    // ── Phase 1 — instances, enumerated from instanceIndex ([R13]) ──
    let manifest = store.load_manifest()?;
    let index_count = manifest.instance_index.len();
    for entry in &manifest.instance_index {
        let path = entry.path().to_string();
        let doc = store.load_instance_json(&path)?;
        let migrated = match entry.tier() {
            2 => {
                result.tier2_records += 1;
                migrate_tier2(&path, doc, &index, &mut result)?
            }
            1 => {
                result.tier1_records += 1;
                migrate_tier1(&path, doc)?
            }
            _ => doc, // Tier-0 Notes carry no field values.
        };
        store.save_instance_json(&path, &migrated)?;
        result.instances_migrated += 1;
    }

    // [R13]: migrated count equals the index count, or the migration fails.
    if result.instances_migrated != index_count {
        return Err(abort(
            "manifest.json",
            format!(
                "migrated {} instances but instanceIndex lists {index_count} ([R13])",
                result.instances_migrated
            ),
        ));
    }

    // ── Phase 2 — repository level ──
    migrate_themes(store, &mut result)?;
    crate::field_type_migration_service::stamp_data_model_revision(store, CARRIER_REVISION)?;
    stamp_package_manifests(store)?;
    delete_zero_referent_versions(store, &index)?;

    Ok(result)
}

/// Phase 0 — Change E.2 minting, trio strip, and the definition index build.
fn migrate_definitions(
    store: &dyn RepositoryStore,
    result: &mut CarrierMigrationResult,
) -> Result<DefinitionIndex, RepositoryError> {
    // Seed from the embedded core package (ADR-025 implicit merge) so records
    // typed by installed core Types — every repo-create repository's purpose
    // record — resolve. Core definitions are already post-RFC-032/039 and are
    // never rewritten here.
    let mut fields: HashMap<String, (String, Value)> = HashMap::new();
    let mut types: HashMap<(String, u64), Vec<(String, bool)>> = HashMap::new();
    {
        let core = crate::core_package::core_package();
        for f in &core.fields {
            let ft = serde_json::to_value(&f.field_type).unwrap_or(Value::Null);
            fields.insert(f.id.clone(), (f.name.clone(), ft));
        }
        for rt in &core.record_types {
            let mut assignments: Vec<(String, bool)> = rt
                .fields
                .iter()
                .map(|fa| (fa.field_id.clone(), fa.required))
                .collect();
            assignments.sort_by_key(|(id, _)| {
                rt.fields
                    .iter()
                    .find(|fa| &fa.field_id == id)
                    .map(|fa| fa.order)
                    .unwrap_or(0)
            });
            types.insert((rt.id.clone(), u64::from(rt.version)), assignments);
        }
    }

    // Package roots: the primary "package" plus every local manifest
    // packageRef (srs-rust#809 — the spec repo declares five sub-packages;
    // resolving types only from the primary root aborted its migration).
    let mut package_roots: Vec<String> = vec!["package".to_string()];
    if let Ok(manifest) = store.load_manifest() {
        for r in manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if r.get("mode").and_then(|m| m.as_str()) == Some("local") {
                if let Some(path) = r.get("path").and_then(|p| p.as_str()) {
                    package_roots.push(path.to_string());
                }
            }
        }
    }
    let mut version_bumps: HashMap<(String, u64), u64> = HashMap::new();
    for root in &package_roots {
        migrate_package_root(
            store,
            root,
            &mut fields,
            &mut types,
            &mut version_bumps,
            result,
        )?;
    }

    Ok(DefinitionIndex {
        fields,
        types,
        version_bumps,
    })
}

/// Load, transform (Change E.2 minting + trio strip) and index one package
/// root's definitions. `root` is `"package"` or a `manifest.packageRefs`
/// local path (srs-rust#809 — sub-package Types must resolve too).
fn migrate_package_root(
    store: &dyn RepositoryStore,
    root: &str,
    fields: &mut HashMap<String, (String, Value)>,
    types: &mut HashMap<(String, u64), Vec<(String, bool)>>,
    version_bumps: &mut HashMap<(String, u64), u64>,
    result: &mut CarrierMigrationResult,
) -> Result<(), RepositoryError> {
    let pkg_index_path = format!("{root}/package.json");
    let mut pkg_index = match store.load_instance_json(&pkg_index_path) {
        Ok(v) => v,
        // A missing root (packageless repository) has no definitions here.
        Err(_) => return Ok(()),
    };

    let field_paths: Vec<String> = list_of_strings(&pkg_index, "fields");
    let type_paths: Vec<String> = list_of_strings(&pkg_index, "types");

    for rel in &field_paths {
        let path = format!("{root}/{rel}");
        let doc = store.load_instance_json(&path)?;
        let id = str_of(&doc, "id", &path)?;
        let name = str_of(&doc, "name", &path)?;
        let ft = doc.get("fieldType").cloned().unwrap_or(Value::Null);
        fields.insert(id, (name, ft));
    }

    let mut new_field_files: Vec<String> = Vec::new();
    let mut new_type_files: Vec<String> = Vec::new();

    for rel in &type_paths {
        let path = format!("{root}/{rel}");
        let mut doc = store.load_instance_json(&path)?;
        let type_id = str_of(&doc, "id", &path)?;
        let type_version = doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
        let type_name = str_of(&doc, "name", &path)?;
        let namespace = str_of(&doc, "namespace", &path)?;

        // 0b. Strip the trio from every assignment ([R7] — missing one leaves
        // the Type schema-invalid at the revision Phase 2 stamps).
        if let Some(fas) = doc.get_mut("fields").and_then(|f| f.as_array_mut()) {
            for fa in fas.iter_mut() {
                if let Some(obj) = fa.as_object_mut() {
                    obj.remove("repeatable");
                    obj.remove("minItems");
                    obj.remove("maxItems");
                }
            }
        }

        // 0a. Change E.2 — each FieldGroup becomes an inline-composite Field
        // over a minted range Type; the owning Type's version is bumped.
        let groups = doc
            .get("fieldGroups")
            .and_then(|g| g.as_array())
            .cloned()
            .unwrap_or_default();
        if !groups.is_empty() {
            let created_at = doc
                .get("createdAt")
                .and_then(|v| v.as_str())
                .unwrap_or("1970-01-01T00:00:00Z")
                .to_string();
            for group in &groups {
                let group_id = str_of(group, "groupId", &path)?;
                let range_name = format!("{}_{}", type_name, group_id).replace('-', "_");
                let range_type_id = uuid::Uuid::new_v4().to_string();
                let new_field_id = uuid::Uuid::new_v4().to_string();

                // The minted range Type R: the group's assignments, trio
                // stripped ([R7] would reject a freshly-minted carrier).
                let mut range_fields: Vec<Value> = Vec::new();
                for fa in group
                    .get("fields")
                    .and_then(|f| f.as_array())
                    .into_iter()
                    .flatten()
                {
                    let mut fa = fa.clone();
                    if let Some(obj) = fa.as_object_mut() {
                        obj.remove("repeatable");
                        obj.remove("minItems");
                        obj.remove("maxItems");
                    }
                    range_fields.push(fa);
                }
                let range_type = json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": range_type_id,
                    "namespace": namespace,
                    "name": range_name,
                    "version": 1,
                    "description": group.get("label").and_then(|v| v.as_str())
                        .map(|l| l.to_string())
                        .unwrap_or_else(|| format!("Range type minted from FieldGroup '{group_id}' by the RFC-039 carrier migration")),
                    "fields": range_fields,
                    "createdAt": created_at,
                });
                let range_rel = format!("types/{range_name}.json");
                store.save_instance_json(&format!("{root}/{range_rel}"), &range_type)?;
                new_type_files.push(range_rel.clone());
                result
                    .minted
                    .push(format!("type {namespace}/{range_name}@1 {range_type_id}"));

                // The minted composite Field.
                let group_repeatable = group
                    .get("repeatable")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut field_type = json!({
                    "datatype": "ref",
                    "mode": "inline",
                    "rangeType": { "typeId": range_type_id, "typeVersion": 1 },
                });
                if group_repeatable {
                    field_type["cardinality"] = json!("list");
                }
                if let Some(min) = group.get("minItems") {
                    field_type["minItems"] = min.clone();
                }
                if let Some(max) = group.get("maxItems") {
                    field_type["maxItems"] = max.clone();
                }
                let purpose = group
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(|l| l.to_string())
                    .unwrap_or_else(|| {
                        format!("Composite field minted from FieldGroup '{group_id}'")
                    });
                let new_field = json!({
                    "id": new_field_id,
                    "namespace": namespace,
                    "name": group_id,
                    "version": 1,
                    "description": purpose,
                    "aiGuidance": { "purpose": purpose },
                    "fieldType": field_type,
                    "createdAt": created_at,
                });
                let field_rel = format!("fields/{}.json", group_id.replace('/', "_"));
                store.save_instance_json(&format!("{root}/{field_rel}"), &new_field)?;
                new_field_files.push(field_rel.clone());
                result
                    .minted
                    .push(format!("field {namespace}/{group_id}@1 {new_field_id}"));
                fields.insert(
                    new_field_id.clone(),
                    (group_id.clone(), new_field["fieldType"].clone()),
                );

                // Assignment on the owning Type: required = minItems >= 1
                // (substantive — E.2), order/displayLabel carried over.
                let required = group
                    .get("minItems")
                    .and_then(|v| v.as_u64())
                    .is_some_and(|m| m >= 1);
                let mut assignment = json!({
                    "fieldId": new_field_id,
                    "order": group.get("order").cloned().unwrap_or(json!(0)),
                    "required": required,
                });
                if let Some(label) = group.get("label") {
                    assignment["displayLabel"] = label.clone();
                }
                if let Some(fas) = doc.get_mut("fields").and_then(|f| f.as_array_mut()) {
                    fas.push(assignment);
                }
            }
            // Version bump: replacing a group with a composite Field changes
            // validation, so the catch-all requires it (Change E.2).
            let new_version = type_version + 1;
            doc["version"] = json!(new_version);
            version_bumps.insert((type_id.clone(), type_version), new_version);
            if let Some(obj) = doc.as_object_mut() {
                obj.remove("fieldGroups");
            }
        }

        store.save_instance_json(&path, &doc)?;

        let assignments: Vec<(String, bool)> = doc
            .get("fields")
            .and_then(|f| f.as_array())
            .map(|fas| {
                let mut sorted: Vec<&Value> = fas.iter().collect();
                sorted.sort_by_key(|fa| fa.get("order").and_then(|o| o.as_u64()).unwrap_or(0));
                sorted
                    .into_iter()
                    .filter_map(|fa| {
                        Some((
                            fa.get("fieldId")?.as_str()?.to_string(),
                            fa.get("required")
                                .and_then(|r| r.as_bool())
                                .unwrap_or(false),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let effective_version = doc.get("version").and_then(|v| v.as_u64()).unwrap_or(1);
        types.insert((type_id, effective_version), assignments);
    }

    // Register minted files in the package index.
    if !new_field_files.is_empty() || !new_type_files.is_empty() {
        if let Some(arr) = pkg_index.get_mut("fields").and_then(|v| v.as_array_mut()) {
            arr.extend(new_field_files.iter().map(|p| json!(p)));
        }
        if let Some(arr) = pkg_index.get_mut("types").and_then(|v| v.as_array_mut()) {
            arr.extend(new_type_files.iter().map(|p| json!(p)));
        }
        store.save_instance_json(&pkg_index_path, &pkg_index)?;
    }
    Ok(())
}

/// Phase 1, Tier 2 — the pair→key transform (steps 1–5).
fn migrate_tier2(
    path: &str,
    mut doc: Value,
    index: &DefinitionIndex,
    result: &mut CarrierMigrationResult,
) -> Result<Value, RepositoryError> {
    // Idempotency FIRST: an already-object carrier (revision 2) is left
    // untouched — before any type resolution, so a record typed by an
    // installed package (e.g. the repo-create purpose record) re-runs clean.
    if matches!(doc.get("fieldValues"), Some(Value::Object(_))) && doc.get("groupValues").is_none()
    {
        return Ok(doc);
    }

    let type_id = str_of(&doc, "typeId", path)?;
    let mut type_version = doc.get("typeVersion").and_then(|v| v.as_u64()).unwrap_or(1);

    // Step 1: re-pin to the post-0a version where 0a bumped it, then resolve
    // the effective field set against the post-transform Type.
    if let Some(new_version) = index.version_bumps.get(&(type_id.clone(), type_version)) {
        type_version = *new_version;
        doc["typeVersion"] = json!(type_version);
    }
    let assignments = index
        .types
        .get(&(type_id.clone(), type_version))
        .ok_or_else(|| {
            abort(
                path,
                format!("unresolvable typeId@typeVersion {type_id}@{type_version}"),
            )
        })?;

    let legacy_pairs = match doc.get("fieldValues") {
        Some(Value::Array(pairs)) => pairs.clone(),
        _ => Vec::new(),
    };

    let mut carrier = Map::new();
    let mut meta = Map::new();
    let mut seen_field_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let assignment_ids: std::collections::HashSet<&str> =
        assignments.iter().map(|(id, _)| id.as_str()).collect();

    for pair in &legacy_pairs {
        let field_id = str_of(pair, "fieldId", path)?;
        // Totality row 1: duplicate fieldId collapses to one key — abort.
        if !seen_field_ids.insert(field_id.clone()) {
            return Err(abort(
                path,
                format!("duplicate fieldId {field_id} in fieldValues"),
            ));
        }
        // Totality row 2: out-of-set fieldId would mint a key [R1] rejects.
        if !assignment_ids.contains(field_id.as_str()) {
            return Err(abort(
                path,
                format!("fieldId {field_id} is not in the Type's effective field set"),
            ));
        }
        // [R10]: every fieldId must resolve to a Field.name.
        let name = index
            .field_name(&field_id)
            .ok_or_else(|| abort(path, format!("unresolvable fieldId {field_id} ([R10])")))?
            .to_string();

        let (value, collapsed) =
            resolve_pair_value(path, pair, index.field_type(&field_id), &name)?;
        result.dual_writes_collapsed += usize::from(collapsed);
        match value {
            Some(v) => {
                result.values_rewritten += 1;
                carrier.insert(name.clone(), v);
            }
            None => {
                // Valueless pair: key omitted with a logged notice — the first
                // declared non-round-trippable class ([R5]: null is not a value).
                result
                    .valueless_pairs_omitted
                    .push(format!("{path}:{name}"));
                continue;
            }
        }

        // Step 5 (depth 0 only): per-value provenance → fieldMeta.
        let mut m = Map::new();
        for k in ["source", "editedAt", "sourceRefs"] {
            if let Some(v) = pair.get(k) {
                m.insert(k.to_string(), v.clone());
            }
        }
        if !m.is_empty() {
            meta.insert(name, Value::Object(m));
        }
    }

    // Step 4: groupValues → the group's minted Field key → array of
    // recursively-transformed fieldValues objects (steps 2–3 at every depth).
    if let Some(groups) = doc.get("groupValues").and_then(|g| g.as_array()).cloned() {
        for group in &groups {
            let group_id = str_of(group, "groupId", path)?;
            // The minted Field's name is the groupId verbatim (E.2).
            let entries = group
                .get("entries")
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            let mut rows: Vec<Value> = Vec::new();
            for entry in &entries {
                if let Some(entry_id) = entry.get("entryId").and_then(|v| v.as_str()) {
                    // Totality row 5: dropped with a logged notice — position
                    // is carried by the list index, and no entryId is
                    // referenced anywhere in the corpus.
                    result
                        .entry_ids_dropped
                        .push(format!("{path}:{group_id}:{entry_id}"));
                }
                let mut row = Map::new();
                for pair in entry
                    .get("fieldValues")
                    .and_then(|f| f.as_array())
                    .into_iter()
                    .flatten()
                {
                    // Totality rows 3–4: per-value/entry-level metadata inside
                    // a composite has no destination ([R6]) — abort.
                    for k in ["source", "editedAt", "sourceRefs"] {
                        if pair.get(k).is_some() {
                            return Err(abort(
                                path,
                                format!(
                                    "per-value metadata '{k}' inside groupValues entry has no \
                                     destination ([R6] forbids fieldMeta inside a composite)"
                                ),
                            ));
                        }
                    }
                    let field_id = str_of(pair, "fieldId", path)?;
                    let name = index
                        .field_name(&field_id)
                        .ok_or_else(|| {
                            abort(path, format!("unresolvable fieldId {field_id} ([R10])"))
                        })?
                        .to_string();
                    let (value, collapsed) =
                        resolve_pair_value(path, pair, index.field_type(&field_id), &name)?;
                    result.dual_writes_collapsed += usize::from(collapsed);
                    if let Some(v) = value {
                        result.values_rewritten += 1;
                        row.insert(name, v);
                    } else {
                        result
                            .valueless_pairs_omitted
                            .push(format!("{path}:{group_id}.{name}"));
                    }
                }
                rows.push(Value::Object(row));
            }
            carrier.insert(group_id, Value::Array(rows));
        }
        if let Some(obj) = doc.as_object_mut() {
            obj.remove("groupValues");
        }
    }

    doc["fieldValues"] = Value::Object(carrier);
    if !meta.is_empty() {
        doc["fieldMeta"] = Value::Object(meta);
    }
    Ok(doc)
}

/// Steps 2–3 for one legacy pair: the value, honouring [R20] (dual-written
/// `value`/`entries` — take `value`, assert the projections agree, abort on
/// divergence) and [R5] (`null` aborts; key absence is the sole unset).
/// Returns `(value, dual_write_collapsed)`.
fn resolve_pair_value(
    path: &str,
    pair: &Value,
    field_type: Option<&Value>,
    name: &str,
) -> Result<(Option<Value>, bool), RepositoryError> {
    let raw_value = pair.get("value");
    let entries = pair.get("entries").and_then(|e| e.as_array());

    let is_list = field_type
        .and_then(|ft| ft.get("cardinality"))
        .and_then(|c| c.as_str())
        == Some("list");

    match (raw_value, entries) {
        (Some(Value::Null), _) | (None, None) => {
            // Explicit null (totality row 8) aborts — except that a bare
            // valueless pair (neither value nor entries) is the one known
            // non-zero row: key omitted, logged by the caller. serde makes the
            // two indistinguishable here (`value: null` and absent both read
            // as no value), so distinguish on the raw JSON:
            if pair.get("value").is_some_and(|v| v.is_null()) {
                return Err(abort(
                    path,
                    format!("explicit null at '{name}' — [R5]: omit the key instead"),
                ));
            }
            Ok((None, false))
        }
        (Some(value), Some(entries)) => {
            // [R20]: the entire entries population is dual-written; take
            // `value`, assert agreement, abort on divergence.
            if !is_list {
                return Err(abort(
                    path,
                    format!("'{name}' carries entries but its Field is not cardinality list"),
                ));
            }
            let projection: Vec<&Value> = entries.iter().filter_map(|e| e.get("value")).collect();
            let value_items: Vec<&Value> = match value {
                Value::Array(items) => items.iter().collect(),
                other => vec![other],
            };
            if projection != value_items {
                return Err(abort(
                    path,
                    format!("'{name}': entries projection diverges from sibling value ([R20])"),
                ));
            }
            // Entry-level provenance has no destination (totality row 3).
            for e in entries {
                for k in ["source", "editedAt"] {
                    if e.get(k).is_some() {
                        return Err(abort(
                            path,
                            format!("entry-level '{k}' on '{name}' has no destination"),
                        ));
                    }
                }
            }
            Ok((Some(value.clone()), true))
        }
        (Some(value), None) => Ok((Some(value.clone()), false)),
        (None, Some(entries)) => {
            // Pure-entries form: 0 occurrences in the audited corpus, but
            // schema-legal — convert entries → array (Change D), list-only.
            if !is_list {
                return Err(abort(
                    path,
                    format!("'{name}' carries entries but its Field is not cardinality list"),
                ));
            }
            for e in entries {
                for k in ["source", "editedAt"] {
                    if e.get(k).is_some() {
                        return Err(abort(
                            path,
                            format!("entry-level '{k}' on '{name}' has no destination"),
                        ));
                    }
                }
            }
            let items: Vec<Value> = entries
                .iter()
                .map(|e| e.get("value").cloned().unwrap_or(Value::Null))
                .collect();
            if items.iter().any(|v| v.is_null()) {
                return Err(abort(path, format!("null entry value at '{name}' ([R5])")));
            }
            Ok((Some(Value::Array(items)), false))
        }
    }
}

/// Phase 1, Tier 1 — `TypedField.valueType`/`selectOptions` → inline
/// `fieldType` ([R8]), through the same RFC-032 Change-H mapping the
/// definition migration uses (`FieldType::from_legacy`).
fn migrate_tier1(path: &str, mut doc: Value) -> Result<Value, RepositoryError> {
    use srs_core::types::field_type::{FieldType, LegacyFieldFacets};

    let Some(fields) = doc.get_mut("fields").and_then(|f| f.as_array_mut()) else {
        return Ok(doc);
    };
    for field in fields.iter_mut() {
        let Some(obj) = field.as_object_mut() else {
            continue;
        };
        if obj.contains_key("fieldType") {
            obj.remove("valueType");
            obj.remove("selectOptions");
            continue; // already revision 2 — idempotent.
        }
        let Some(value_type) = obj.get("valueType").and_then(|v| v.as_str()) else {
            // A TypedField with neither facet is schema-legal revision ≤ 1;
            // its value shape is self-evident — treat as plain string/typed by
            // the stored value. [R8] requires a fieldType, so derive one from
            // the stored value's JSON type.
            let derived = match obj.get("value") {
                Some(Value::Number(_)) => json!({"datatype": "number"}),
                Some(Value::Bool(_)) => json!({"datatype": "boolean"}),
                Some(Value::Array(_)) => {
                    json!({"datatype": "string", "cardinality": "list"})
                }
                _ => json!({"datatype": "string"}),
            };
            obj.insert("fieldType".to_string(), derived);
            continue;
        };
        let legacy = match value_type {
            "string" => srs_core::types::field_type::LegacyValueType::String,
            "text" => srs_core::types::field_type::LegacyValueType::Text,
            "number" => srs_core::types::field_type::LegacyValueType::Number,
            "boolean" => srs_core::types::field_type::LegacyValueType::Boolean,
            "date" => srs_core::types::field_type::LegacyValueType::Date,
            "url" => srs_core::types::field_type::LegacyValueType::Url,
            "select" => srs_core::types::field_type::LegacyValueType::Select,
            "multiselect" => srs_core::types::field_type::LegacyValueType::Multiselect,
            other => {
                return Err(abort(path, format!("unknown Tier-1 valueType '{other}'")));
            }
        };
        let facets = LegacyFieldFacets {
            allowed_values: obj.get("selectOptions").and_then(|v| {
                v.as_array().map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
            }),
            ..Default::default()
        };
        let ft = FieldType::from_legacy(legacy, &facets);
        let ft_value = serde_json::to_value(&ft).map_err(|e| abort(path, e))?;
        obj.insert("fieldType".to_string(), ft_value);
        obj.remove("valueType");
        obj.remove("selectOptions");
    }
    Ok(doc)
}

/// Phase 2 step 6 — theme `groupFieldRowTemplates` → `compositeFieldRowTemplates`
/// ([R12]: carry every key; abort on nothing here because key→field matching is
/// name-identity under the carrier — a template keyed by a name that matches no
/// field is unreachable but not droppable, so it is carried verbatim).
fn migrate_themes(
    store: &dyn RepositoryStore,
    result: &mut CarrierMigrationResult,
) -> Result<(), RepositoryError> {
    let pkg_index = match store.load_instance_json("package/package.json") {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    for rel in list_of_strings(&pkg_index, "themes") {
        let path = format!("package/{rel}");
        let mut doc = store.load_instance_json(&path)?;
        let Some(et) = doc
            .get_mut("elementTemplates")
            .and_then(|v| v.as_object_mut())
        else {
            continue;
        };
        if let Some(templates) = et.remove("groupFieldRowTemplates") {
            let target = et
                .entry("compositeFieldRowTemplates")
                .or_insert_with(|| json!({}));
            if let (Some(target_map), Value::Object(source)) = (target.as_object_mut(), templates) {
                for (k, v) in source {
                    result.theme_keys_carried += 1;
                    target_map.entry(k).or_insert(v);
                }
            }
            store.save_instance_json(&path, &doc)?;
        }
    }
    Ok(())
}

/// Phase 2 step 8 — stamp every first-party package manifest: the primary
/// root and every local manifest packageRef (RFC-039 Change H names all of
/// them; srs-rust#809).
fn stamp_package_manifests(store: &dyn RepositoryStore) -> Result<(), RepositoryError> {
    let mut roots: Vec<String> = vec!["package".to_string()];
    if let Ok(manifest) = store.load_manifest() {
        for r in manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if r.get("mode").and_then(|m| m.as_str()) == Some("local") {
                if let Some(path) = r.get("path").and_then(|p| p.as_str()) {
                    roots.push(path.to_string());
                }
            }
        }
    }
    for root in roots {
        let path = format!("{root}/package.json");
        if let Ok(mut pkg_index) = store.load_instance_json(&path) {
            if let Some(obj) = pkg_index.as_object_mut() {
                obj.insert("dataModelRevision".to_string(), json!(CARRIER_REVISION));
            }
            store.save_instance_json(&path, &pkg_index)?;
        }
    }
    Ok(())
}

/// Phase 2 step 10 — delete every Type version left with zero referents by
/// step 1's re-pin (Change E.2: the superseded pre-bump versions). With
/// single-file-per-Type storage the bump already rewrote the file in place, so
/// there is no stale version file to remove; this is the assertion that none
/// survives.
fn delete_zero_referent_versions(
    store: &dyn RepositoryStore,
    index: &DefinitionIndex,
) -> Result<(), RepositoryError> {
    // Single-file-per-Type: the version bump overwrote the definition file, so
    // the superseded version is already gone from the tree. Verify no instance
    // still pins a bumped-away version ([R19] scope check).
    let manifest = store.load_manifest()?;
    for entry in &manifest.instance_index {
        if entry.tier() != 2 {
            continue;
        }
        let doc = store.load_instance_json(entry.path())?;
        let type_id = doc.get("typeId").and_then(|v| v.as_str()).unwrap_or("");
        let type_version = doc.get("typeVersion").and_then(|v| v.as_u64()).unwrap_or(1);
        if index
            .version_bumps
            .contains_key(&(type_id.to_string(), type_version))
        {
            return Err(abort(
                entry.path(),
                format!("instance still pins superseded {type_id}@{type_version} after migration"),
            ));
        }
    }
    Ok(())
}

// ── small raw-JSON helpers ──────────────────────────────────────────────────

fn list_of_strings(doc: &Value, key: &str) -> Vec<String> {
    doc.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn str_of(doc: &Value, key: &str, path: &str) -> Result<String, RepositoryError> {
    doc.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| abort(path, format!("missing '{key}'")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_store::JsonStore;
    use serde_json::json;

    const F_TITLE: &str = "aa000001-0000-4000-a000-000000000001";
    const F_TAGS: &str = "aa000002-0000-4000-a000-000000000002";
    const F_NAME: &str = "aa000003-0000-4000-a000-000000000003";
    const F_ROLE: &str = "aa000004-0000-4000-a000-000000000004";
    const F_ALIASES: &str = "aa000005-0000-4000-a000-000000000005";
    const T_ITEM: &str = "bb000001-0000-4000-b000-000000000001";
    const T_PLAIN: &str = "bb000002-0000-4000-b000-000000000002";
    const R_ITEM: &str = "cc000001-0000-4000-c000-000000000001";
    const R_PLAIN: &str = "cc000002-0000-4000-c000-000000000002";

    #[derive(Default)]
    struct FixtureKnobs {
        /// `labels` entries projection diverges from the sibling value ([R20]).
        divergent_entries: bool,
        /// The item Type carries an assignment whose fieldId has no Field
        /// definition, and the record carries a pair for it ([R10]).
        ghost_field: bool,
    }

    /// A revision-1 repository: legacy pair-array `fieldValues`, a dual-written
    /// list, `groupValues` with a valueless pair and a nested entries-only
    /// list, the FieldAssignment trio, and one FieldGroup.
    fn fixture_srsj(knobs: &FixtureKnobs) -> String {
        let field = |id: &str, name: &str, field_type: Value| {
            json!({
                "id": id, "namespace": "com.test", "name": name, "version": 1,
                "description": name, "aiGuidance": {"purpose": name},
                "fieldType": field_type, "createdAt": "2026-01-01T00:00:00Z"
            })
        };

        let mut item_assignments = vec![
            json!({"fieldId": F_TITLE, "order": 0, "required": true, "repeatable": false}),
            json!({"fieldId": F_TAGS, "order": 1, "required": false, "repeatable": true, "minItems": 1, "maxItems": 5}),
        ];
        if knobs.ghost_field {
            item_assignments.push(json!({"fieldId": "aa00dead-0000-4000-a000-00000000dead", "order": 3, "required": false}));
        }

        let item_type = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
            "id": T_ITEM, "namespace": "com.test", "name": "item", "version": 1,
            "description": "Item with a FieldGroup",
            "fields": item_assignments,
            "fieldGroups": [{
                "groupId": "people",
                "order": 2,
                "label": "People",
                "repeatable": true,
                "minItems": 1,
                "fields": [
                    {"fieldId": F_NAME, "order": 0, "required": true, "repeatable": false},
                    {"fieldId": F_ROLE, "order": 1, "required": false},
                    {"fieldId": F_ALIASES, "order": 2, "required": false, "repeatable": true}
                ]
            }],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let plain_type = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
            "id": T_PLAIN, "namespace": "com.test", "name": "plain", "version": 1,
            "description": "Type with the trio but no groups",
            "fields": [
                {"fieldId": F_TITLE, "order": 0, "required": true, "repeatable": false, "minItems": 0, "maxItems": 1}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        let tags_entries = if knobs.divergent_entries {
            json!([{"value": "a"}, {"value": "z"}])
        } else {
            json!([{"value": "a"}, {"value": "b"}])
        };
        let mut item_pairs = vec![
            json!({"fieldId": F_TITLE, "value": "Hello"}),
            json!({"fieldId": F_TAGS, "value": ["a", "b"], "entries": tags_entries}),
        ];
        if knobs.ghost_field {
            item_pairs
                .push(json!({"fieldId": "aa00dead-0000-4000-a000-00000000dead", "value": "x"}));
        }

        let item_record = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": R_ITEM, "typeId": T_ITEM, "typeVersion": 1,
            "typeNamespace": "com.test", "typeName": "item",
            "fieldValues": item_pairs,
            "groupValues": [{
                "groupId": "people",
                "entries": [
                    {
                        "entryId": "e1",
                        "fieldValues": [
                            {"fieldId": F_NAME, "value": "alice"},
                            {"fieldId": F_ROLE},
                            {"fieldId": F_ALIASES, "entries": [{"value": "x"}, {"value": "y"}]}
                        ]
                    },
                    {"fieldValues": [{"fieldId": F_NAME, "value": "bob"}]}
                ]
            }],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let plain_record = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": R_PLAIN, "typeId": T_PLAIN, "typeVersion": 1,
            "typeNamespace": "com.test", "typeName": "plain",
            "fieldValues": [{"fieldId": F_TITLE, "value": "Plain"}],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        json!({
            "srsj": "1",
            "manifest": {
                "srsVersion": "2.0",
                "repositoryId": "00000000-0000-4000-8000-0000000000ff",
                "title": "Carrier Migration Fixture",
                "dataModelRevision": 1,
                "instanceIndex": [
                    {"instanceId": R_ITEM, "tier": 2, "path": "records/r-item.json"},
                    {"instanceId": R_PLAIN, "tier": 2, "path": "records/r-plain.json"}
                ],
                "packageRef": {"mode": "local", "path": "package"},
                "dataModelRevision": 1,
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "data": {
                "records/r-item.json": item_record,
                "records/r-plain.json": plain_record,
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "00000000-0000-4000-8000-0000000000aa",
                    "namespace": "com.test", "name": "fixture", "title": "Fixture",
                    "description": "carrier migration fixture", "status": "active",
                    "version": "1.0.0", "createdAt": "2026-01-01T00:00:00Z",
                    "fields": [
                        "fields/title.json", "fields/labels.json", "fields/person_name.json",
                        "fields/role.json", "fields/aliases.json"
                    ],
                    "types": ["types/item.json", "types/plain.json"],
                    "views": [], "documentViews": []
                },
                "package/fields/title.json": field(F_TITLE, "title", json!({"datatype": "string"})),
                "package/fields/labels.json":
                    field(F_TAGS, "labels", json!({"datatype": "string", "cardinality": "list"})),
                "package/fields/person_name.json":
                    field(F_NAME, "person_name", json!({"datatype": "string"})),
                "package/fields/role.json": field(F_ROLE, "role", json!({"datatype": "string"})),
                "package/fields/aliases.json":
                    field(F_ALIASES, "aliases", json!({"datatype": "string", "cardinality": "list"})),
                "package/types/item.json": item_type,
                "package/types/plain.json": plain_type
            }
        })
        .to_string()
    }

    fn migrated_store() -> (JsonStore, CarrierMigrationResult) {
        let store = JsonStore::from_srsj(&fixture_srsj(&FixtureKnobs::default())).unwrap();
        let result = migrate_carrier(&store).expect("migration must succeed");
        (store, result)
    }

    #[test]
    fn field_group_minted_as_composite_with_version_bump() {
        let (store, result) = migrated_store();

        // The owning Type bumped 1 → 2 and lost its fieldGroups.
        let item_type = store.load_instance_json("package/types/item.json").unwrap();
        assert_eq!(item_type["version"], json!(2));
        assert!(item_type.get("fieldGroups").is_none());

        // A composite Field named after the groupId was minted and assigned.
        let minted_field = store
            .load_instance_json("package/fields/people.json")
            .expect("minted composite field file");
        assert_eq!(minted_field["name"], json!("people"));
        assert_eq!(minted_field["fieldType"]["datatype"], json!("ref"));
        assert_eq!(minted_field["fieldType"]["mode"], json!("inline"));
        assert_eq!(minted_field["fieldType"]["cardinality"], json!("list"));
        assert_eq!(minted_field["fieldType"]["minItems"], json!(1));
        let range = &minted_field["fieldType"]["rangeType"];

        // The minted range Type carries the group's fields, trio stripped.
        let range_type = store
            .load_instance_json("package/types/item_people.json")
            .expect("minted range type file");
        assert_eq!(range_type["id"], range["typeId"]);
        assert_eq!(range_type["version"], range["typeVersion"]);
        let range_fields = range_type["fields"].as_array().unwrap();
        assert_eq!(range_fields.len(), 3);
        for fa in range_fields {
            assert!(
                fa.get("repeatable").is_none(),
                "trio must be stripped: {fa}"
            );
        }

        // Both minted definitions are registered in the package index.
        let pkg = store.load_instance_json("package/package.json").unwrap();
        assert!(pkg["fields"]
            .as_array()
            .unwrap()
            .contains(&json!("fields/people.json")));
        assert!(pkg["types"]
            .as_array()
            .unwrap()
            .contains(&json!("types/item_people.json")));
        assert_eq!(result.minted.len(), 2, "one field + one range type minted");

        // The record re-pinned to the bumped version and carries the composite
        // under the group's key.
        let record = store.load_instance_json("records/r-item.json").unwrap();
        assert_eq!(record["typeVersion"], json!(2));
        assert_eq!(
            record["fieldValues"]["people"][0]["person_name"],
            json!("alice")
        );
        assert_eq!(
            record["fieldValues"]["people"][1]["person_name"],
            json!("bob")
        );
        assert!(record.get("groupValues").is_none());
    }

    #[test]
    fn trio_stripped_from_types_without_groups() {
        let (store, _) = migrated_store();
        let plain = store
            .load_instance_json("package/types/plain.json")
            .unwrap();
        // No group → no version bump.
        assert_eq!(plain["version"], json!(1));
        for fa in plain["fields"].as_array().unwrap() {
            assert!(fa.get("repeatable").is_none(), "{fa}");
            assert!(fa.get("minItems").is_none(), "{fa}");
            assert!(fa.get("maxItems").is_none(), "{fa}");
        }
    }

    #[test]
    fn dual_written_entries_taken_from_value_and_asserted() {
        let (store, result) = migrated_store();
        let record = store.load_instance_json("records/r-item.json").unwrap();
        // [R20]: the `value` array wins; the agreeing entries projection is
        // collapsed and counted.
        assert_eq!(record["fieldValues"]["labels"], json!(["a", "b"]));
        assert!(result.dual_writes_collapsed >= 1);
    }

    #[test]
    fn divergent_entries_abort() {
        let store = JsonStore::from_srsj(&fixture_srsj(&FixtureKnobs {
            divergent_entries: true,
            ..Default::default()
        }))
        .unwrap();
        let err = migrate_carrier(&store).expect_err("divergent dual write must abort");
        let msg = err.to_string();
        assert!(msg.contains("[R20]"), "diagnostic cites [R20]: {msg}");
        assert!(msg.contains("labels"), "diagnostic names the field: {msg}");
    }

    #[test]
    fn unresolvable_field_id_aborts_and_rolls_back() {
        // Disk-backed store: JsonStore's ADR-021 abort restores from disk, so
        // the batch seam is actually exercised.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.srsj");
        let srsj = fixture_srsj(&FixtureKnobs {
            ghost_field: true,
            ..Default::default()
        });
        std::fs::write(&path, &srsj).unwrap();
        let before = std::fs::read(&path).unwrap();

        let store = JsonStore::open(&path).unwrap();
        let err = migrate_carrier(&store).expect_err("unresolvable fieldId must abort");
        assert!(
            err.to_string().contains("[R10]"),
            "diagnostic cites [R10]: {err}"
        );

        // Batch rollback: nothing was flushed, the file is byte-identical, and
        // a fresh open still sees the legacy revision.
        let after = std::fs::read(&path).unwrap();
        assert_eq!(before, after, "abort must leave the store unchanged");
        let reopened = JsonStore::open(&path).unwrap();
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&reopened).unwrap(),
            1
        );
        let record = reopened.load_instance_json("records/r-item.json").unwrap();
        assert!(
            record["fieldValues"].is_array(),
            "records must be untouched"
        );
    }

    #[test]
    fn nested_group_values_recurse_depth() {
        // Steps 2–3 apply at every depth: the entries-only list inside a group
        // entry converts to a plain array under its Field.name.
        let (store, _) = migrated_store();
        let record = store.load_instance_json("records/r-item.json").unwrap();
        assert_eq!(
            record["fieldValues"]["people"][0]["aliases"],
            json!(["x", "y"])
        );
    }

    #[test]
    fn valueless_pair_omitted_and_logged() {
        let (store, result) = migrated_store();
        let record = store.load_instance_json("records/r-item.json").unwrap();
        // The valueless `role` pair: key omitted ([R5]), logged as the first
        // declared non-round-trippable class.
        assert!(record["fieldValues"]["people"][0].get("role").is_none());
        assert!(
            result
                .valueless_pairs_omitted
                .iter()
                .any(|p| p.contains("role")),
            "omitted pair must be logged: {:?}",
            result.valueless_pairs_omitted
        );
        // The dropped entryId is logged too.
        assert!(
            result.entry_ids_dropped.iter().any(|e| e.contains("e1")),
            "{:?}",
            result.entry_ids_dropped
        );
    }

    #[test]
    fn rerun_is_byte_idempotent() {
        let (store, _) = migrated_store();
        let first = store.to_srsj_string().unwrap();
        migrate_carrier(&store).expect("re-run must succeed");
        let second = store.to_srsj_string().unwrap();
        assert_eq!(
            first, second,
            "re-running the migration must be a byte no-op"
        );
    }

    #[test]
    fn instance_count_asserted_against_index() {
        let (store, result) = migrated_store();
        let index_len = store.load_manifest().unwrap().instance_index.len();
        assert_eq!(result.instances_migrated, index_len);
        assert_eq!(result.tier2_records, 2);
        assert_eq!(
            crate::field_type_migration_service::data_model_revision(&store).unwrap(),
            CARRIER_REVISION
        );
    }

    #[test]
    fn migrated_output_passes_value_shape_validation() {
        let (store, _) = migrated_store();
        let package = store.load_package().unwrap();
        for (path, type_id, version) in [
            ("records/r-item.json", T_ITEM, 2),
            ("records/r-plain.json", T_PLAIN, 1),
        ] {
            let record: srs_core::types::record::Record =
                serde_json::from_value(store.load_instance_json(path).unwrap())
                    .unwrap_or_else(|e| panic!("{path} must parse as a revision-2 Record: {e}"));
            let rt = package
                .resolve_type(type_id, version)
                .unwrap_or_else(|| panic!("{type_id}@{version} must resolve"));
            let effective = package.resolved_effective_fields(rt).unwrap();
            let diagnostics = srs_core::validation::record::validate_record_all(
                &record, rt, &effective, &package,
            );
            assert!(
                diagnostics.is_empty(),
                "{path} must pass value-shape validation, got: {diagnostics:?}"
            );
        }
    }

    /// srs-rust#809: a record typed by a `manifest.packageRefs` sub-package
    /// Type must migrate — the definition index reads every local root, and
    /// Phase 2 stamps every root's package manifest.
    #[test]
    fn sub_package_types_resolve_and_all_manifests_stamped() {
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "instanceIndex": [
                    {"instanceId": "00000000-0000-4000-8000-000000000901", "tier": 2,
                     "path": "records/r-sub.json"}
                ],
                "packageRef": {"mode": "local", "path": "package"},
                "packageRefs": [{"mode": "local", "path": "package/sub"}],
                "dataModelRevision": 1,
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "data": {
                "records/r-sub.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": "00000000-0000-4000-8000-000000000901",
                    "typeId": "00000000-0000-4000-8000-000000000922",
                    "typeVersion": 1,
                    "typeNamespace": "com.test.sub", "typeName": "subthing",
                    "fieldValues": [
                        {"fieldId": "00000000-0000-4000-8000-000000000921", "value": "hello"}
                    ]
                },
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "00000000-0000-4000-8000-0000000000aa",
                    "namespace": "com.test", "name": "root-pkg", "title": "Root",
                    "description": "root", "status": "active", "version": "1.0.0",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": [], "types": [], "views": [], "documentViews": []
                },
                "package/sub/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "00000000-0000-4000-8000-0000000000ab",
                    "namespace": "com.test.sub", "name": "sub-pkg", "title": "Sub",
                    "description": "sub", "status": "active", "version": "1.0.0",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": ["fields/subtitle.json"], "types": ["types/subthing.json"],
                    "views": [], "documentViews": []
                },
                "package/sub/fields/subtitle.json": {
                    "id": "00000000-0000-4000-8000-000000000921",
                    "namespace": "com.test.sub", "name": "subtitle", "version": 1,
                    "description": "subtitle", "aiGuidance": {"purpose": "subtitle"},
                    "fieldType": {"datatype": "string"},
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/sub/types/subthing.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": "00000000-0000-4000-8000-000000000922",
                    "namespace": "com.test.sub", "name": "subthing", "version": 1,
                    "description": "sub-package type",
                    "fields": [{"fieldId": "00000000-0000-4000-8000-000000000921",
                                "order": 0, "required": true}],
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string();
        let store = JsonStore::from_srsj(&srsj).unwrap();
        let result = migrate_carrier(&store).expect("sub-package type must resolve (srs-rust#809)");
        assert_eq!(result.instances_migrated, 1);

        let record = store.load_instance_json("records/r-sub.json").unwrap();
        assert_eq!(record["fieldValues"]["subtitle"], "hello");

        let root_pkg = store.load_instance_json("package/package.json").unwrap();
        assert_eq!(root_pkg["dataModelRevision"], 2);
        let sub_pkg = store
            .load_instance_json("package/sub/package.json")
            .unwrap();
        assert_eq!(
            sub_pkg["dataModelRevision"], 2,
            "packageRefs manifests are stamped too (Change H)"
        );
    }

    /// srs-rust#809 (guard): a revision-0 repository must be refused — its
    /// definitions still carry `valueType`, so stamping revision 2 over them
    /// would produce an inconsistent artifact. The diagnostic names the
    /// prerequisite migration.
    #[test]
    fn revision_zero_repository_refused_with_field_type_pointer() {
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "data": {}
        })
        .to_string();
        let store = JsonStore::from_srsj(&srsj).unwrap();
        let err = migrate_carrier(&store).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--id field-type"), "{msg}");
        assert!(msg.contains("revision >= 1"), "{msg}");
    }
}
