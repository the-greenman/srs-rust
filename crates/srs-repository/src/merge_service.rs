//! Bring new items in from another copy of the same repository (srs-rust#1580).
//!
//! Two-way and additive: an item the target lacks can come in, unchanged; an item both copies
//! hold is never written, and is reported when the two differ (`ext:repository`, I-54). There
//! is no base (srs#387), so a record deleted from the target looks new in the source: the
//! preview is the only protection.
//!
//! Carried items keep their `createdBy` and everything else (RFC-046 [R6], Revision 7). The
//! operation originates nothing, so it stamps nothing and needs no session actor. It writes no
//! relation of its own and never writes the manifest. Package definitions stay as the target
//! holds them. Not exposed over MCP: Change C promises an agent cannot place another actor's
//! `createdBy` through that write API.
//!
//! Preview and apply share one plan, so a client can check a selection without writing. Every
//! check runs before the first write, because no shipped store can roll back
//! (`supports_batch_rollback`). An I/O failure partway leaves the items already written, and a
//! rerun brings in the rest.

use crate::actor_service;
use crate::container_store::{self, head_path};
use crate::diff::diff_keyed;
use crate::error::RepositoryError;
use crate::field_type_migration_service::{
    data_model_revision, CURRENT_DATA_MODEL_REVISION, RFC046_ACTOR_PROVENANCE_REVISION,
};
use crate::repository_portability::{
    export_repository_snapshot_with_options, ExportSnapshotOptions, RepositorySnapshot,
    SourceDocumentSnapshot,
};
use crate::store::{RecordTier, RepositoryStore};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::Value;
use srs_core::container_decode::StoredEntry;
use srs_core::types::container::Container;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};

const DEFAULT_SOURCE_DOCUMENTS: &str = "source-documents";

/// The items a client chose, by stable id. Attachments are not chosen: one comes in with the
/// items that reference it.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeSelection {
    #[serde(default)]
    pub instance_ids: Vec<String>,
    #[serde(default)]
    pub relation_ids: Vec<String>,
    #[serde(default)]
    pub container_ids: Vec<String>,
    #[serde(default)]
    pub container_entries: Vec<MergeEntryId>,
    /// Never accepted: present only so a request carrying it is refused as `actor-supplied`
    /// (RFC-046 [R6], [R4]) and not as an unknown field.
    #[serde(default, deserialize_with = "present")]
    pub created_by: Option<Value>,
}

fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    <Value as serde::Deserialize>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MergeEntryId {
    pub container_id: String,
    pub instance_id: String,
}

/// Which kind of keyed object a report line is about: `instance`, `relation`, `container`,
/// `container-entry`, `attachment` or `definition`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeItemRef {
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
}

/// An item that will not come in, and the stable kebab-case reason why.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeHeldBack {
    #[serde(flatten)]
    pub item: MergeItemRef,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeInstance {
    pub instance_id: String,
    pub tier: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Value>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRelation {
    pub relation_id: String,
    pub relation_type: String,
    pub source_instance_id: String,
    pub target_instance_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeContainer {
    pub container_id: String,
    pub title: String,
}

/// One outline entry, decoded: RFC-051 [R5] keeps `parent` and `key` out of every payload.
/// `depth` is its depth in the source; an entry in `orphanedEntries` decodes higher in the target.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeEntry {
    pub container_id: String,
    pub instance_id: String,
    pub depth: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeAttachment {
    pub document_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// What comes in.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeItems {
    pub instances: Vec<MergeInstance>,
    pub relations: Vec<MergeRelation>,
    pub containers: Vec<MergeContainer>,
    pub container_entries: Vec<MergeEntry>,
    pub attachments: Vec<MergeAttachment>,
}

/// The one result of preview and apply. An item only the source holds is always in exactly one
/// of `bring`, `heldBack` and `notChosen`, so nothing is silently discarded.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReport {
    pub repository_id: String,
    /// False for a preview, true once the items in `bring` are written.
    pub applied: bool,
    /// Registry migrations the source archive went through to open. "Unchanged" means
    /// unchanged after these (RFC-051 [R13]).
    pub source_migrations: Vec<String>,
    pub bring: MergeItems,
    /// Same key, different content: the target's copy stands.
    pub differ: Vec<MergeItemRef>,
    pub held_back: Vec<MergeHeldBack>,
    /// Bringable, but left out of the selection.
    pub not_chosen: Vec<MergeItemRef>,
    /// Chosen, but not bringable. An apply refuses while this is not empty.
    pub unavailable: Vec<MergeItemRef>,
    /// Coming in, but its parent entry is not in the container: legal, shown as an orphan
    /// (RFC-051 [R6]).
    pub orphaned_entries: Vec<MergeEntryId>,
}

fn refuse(code: &'static str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::MergeRefused {
        code,
        message: message.into(),
    }
}

fn item(kind: &str, id: &str) -> MergeItemRef {
    MergeItemRef {
        kind: kind.to_string(),
        id: id.to_string(),
        container_id: None,
    }
}

fn entry_item(container_id: &str, instance_id: &str) -> MergeItemRef {
    MergeItemRef {
        container_id: Some(container_id.to_string()),
        ..item("container-entry", instance_id)
    }
}

/// Report what bringing in from the archive would do. Writes nothing. With a selection, the
/// report answers for exactly that choice.
pub fn merge_preview(
    target: &dyn RepositoryStore,
    archive: impl Read + Seek,
    selection: Option<&MergeSelection>,
) -> Result<MergeReport, RepositoryError> {
    let source = crate::archive::archive_to_tree(archive)?;
    Ok(build_plan(target, &source, selection)?.report)
}

/// Bring the chosen items in, or refuse and write nothing.
pub fn merge_apply(
    target: &dyn RepositoryStore,
    archive: impl Read + Seek,
    selection: &MergeSelection,
) -> Result<MergeReport, RepositoryError> {
    let source = crate::archive::archive_to_tree(archive)?;
    let plan = build_plan(target, &source, Some(selection))?;
    if !plan.report.unavailable.is_empty() {
        let ids: Vec<&str> = plan
            .report
            .unavailable
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        return Err(refuse(
            "merge-selection-invalid",
            format!(
                "{} chosen item(s) cannot come in, so nothing was written: {}. Preview this \
                 selection for the reasons.",
                ids.len(),
                ids.join(", ")
            ),
        ));
    }
    target.begin_batch();
    match write_plan(target, &plan) {
        Ok(()) => target.commit_batch()?,
        Err(e) => {
            let _ = target.abort_batch();
            return Err(e);
        }
    }
    let mut report = plan.report;
    report.applied = true;
    Ok(report)
}

/// The items to write, as the source holds them.
struct Plan {
    report: MergeReport,
    instances: Vec<(u8, Value)>,
    relations: Vec<srs_core::types::relation::Relation>,
    /// Raw heads of the containers the target lacks.
    heads: Vec<(String, Value)>,
    entries: Vec<(String, StoredEntry)>,
    attachments: Vec<SourceDocumentSnapshot>,
    source_documents_base: String,
}

fn check_revisions(
    target: &dyn RepositoryStore,
    source: &dyn RepositoryStore,
    source_snap: &RepositorySnapshot,
    chosen: &Chosen<'_>,
) -> Result<(), RepositoryError> {
    let target_revision = data_model_revision(target)?;
    let source_revision = data_model_revision(source)?;
    // [R13] comes first: it names the rule the target's older revision breaks.
    if target_revision < RFC046_ACTOR_PROVENANCE_REVISION {
        let in_instances = source_snap.instances.iter().any(|i| {
            chosen.instance(&i.instance_id) && i.value.get(actor_service::CREATED_BY_KEY).is_some()
        });
        let in_relations = source_snap
            .relations
            .iter()
            .any(|r| chosen.relation(&r.relation_id) && r.created_by.is_some());
        if in_instances || in_relations {
            return Err(actor_service::refuse(
                "revision-too-old",
                format!(
                    "this would place a createdBy into a corpus at dataModelRevision \
                     {target_revision} (< {RFC046_ACTOR_PROVENANCE_REVISION}); migrate the \
                     target first (RFC-046 [R13])"
                ),
            ));
        }
    }
    if target_revision != source_revision {
        return Err(refuse(
            "merge-revision-mismatch",
            format!(
                "the source opens at dataModelRevision {source_revision} but the target is at \
                 {target_revision}; migrate the target first"
            ),
        ));
    }
    Ok(())
}

/// The client's choice; `None` means everything, for a whole preview.
struct Chosen<'a> {
    selection: Option<&'a MergeSelection>,
}

impl Chosen<'_> {
    fn instance(&self, id: &str) -> bool {
        self.selection
            .is_none_or(|s| s.instance_ids.iter().any(|i| i == id))
    }
    fn relation(&self, id: &str) -> bool {
        self.selection
            .is_none_or(|s| s.relation_ids.iter().any(|i| i == id))
    }
    fn container(&self, id: &str) -> bool {
        self.selection
            .is_none_or(|s| s.container_ids.iter().any(|i| i == id))
    }
    fn entry(&self, container_id: &str, instance_id: &str) -> bool {
        self.selection.is_none_or(|s| {
            s.container_entries
                .iter()
                .any(|e| e.container_id == container_id && e.instance_id == instance_id)
        })
    }
}

fn snapshot_with_blobs(store: &dyn RepositoryStore) -> Result<RepositorySnapshot, RepositoryError> {
    export_repository_snapshot_with_options(
        store,
        ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )
}

fn containers(snap: &RepositorySnapshot) -> BTreeMap<&str, &Container> {
    snap.containers
        .iter()
        .chain(snap.root_container.iter())
        .map(|c| (c.container_id.as_str(), c))
        .collect()
}

fn to_value<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Every package definition on one side, as `<kind>/<id>@<version>` (a new version of a
/// definition is a new key, so only a same-version edit reads as differing).
fn definitions(snap: &RepositorySnapshot) -> BTreeMap<String, Value> {
    fn add<T: serde::Serialize>(out: &mut BTreeMap<String, Value>, kind: &str, items: &[T]) {
        for item in items {
            let value = to_value(item);
            let key = format!(
                "{kind}/{}@{}",
                value["id"].as_str().unwrap_or_default(),
                value["version"]
            );
            out.insert(key, value);
        }
    }
    let mut out = BTreeMap::new();
    for p in &snap.packages {
        add(&mut out, "field", &p.fields);
        add(&mut out, "type", &p.record_types);
        add(&mut out, "relation-type", &p.relation_type_definitions);
        add(&mut out, "view", &p.views);
        add(&mut out, "composition", &p.compositions);
        add(&mut out, "blueprint", &p.blueprints);
        add(&mut out, "theme", &p.themes);
        add(&mut out, "vocabulary", &p.vocabularies);
        add(&mut out, "lifecycle", &p.lifecycles);
        add(&mut out, "protocol", &p.protocols);
    }
    out
}

/// Every attachment id the value's `sourceRefs` names.
fn referenced_documents(value: &Value, out: &mut BTreeSet<String>) {
    let refs = value.get("sourceRefs").and_then(Value::as_array);
    for r in refs.into_iter().flatten() {
        if let Some(id) = r.get("sourceId").and_then(Value::as_str) {
            out.insert(id.to_string());
        }
    }
}

fn build_plan(
    target: &dyn RepositoryStore,
    source: &dyn RepositoryStore,
    selection: Option<&MergeSelection>,
) -> Result<Plan, RepositoryError> {
    if selection.is_some_and(|s| s.created_by.is_some()) {
        return Err(actor_service::refuse(
            "actor-supplied",
            "a request may not supply createdBy: a carried instance keeps its own (RFC-046 [R6])",
        ));
    }
    if !target.is_file_tree_store() {
        return Err(refuse(
            "merge-store-unsupported",
            "bringing items in needs a target stored as a file tree",
        ));
    }
    if target.migration_notice().is_some() {
        return Err(refuse(
            "merge-target-migration-needed",
            "the target was opened through a migration and refuses writes until that is saved",
        ));
    }
    let source_revision = data_model_revision(source)?;
    if source_revision > CURRENT_DATA_MODEL_REVISION {
        return Err(refuse(
            "merge-revision-unsupported",
            format!(
                "the source is at dataModelRevision {source_revision}; this build reads up to \
                 {CURRENT_DATA_MODEL_REVISION}"
            ),
        ));
    }
    let chosen = Chosen { selection };
    let src = snapshot_with_blobs(source)?;
    check_revisions(target, source, &src, &chosen)?;
    let tgt = snapshot_with_blobs(target)?;
    if src.repository.repository_id != tgt.repository.repository_id {
        return Err(refuse(
            "merge-repository-mismatch",
            format!(
                "the source is repository {} and the target is {}: only copies of the same \
                 repository can be merged",
                src.repository.repository_id, tgt.repository.repository_id
            ),
        ));
    }

    let mut report = MergeReport {
        repository_id: tgt.repository.repository_id.clone(),
        applied: false,
        source_migrations: source
            .migration_notice()
            .map(|n| n.migrations)
            .unwrap_or_default(),
        bring: MergeItems::default(),
        differ: Vec::new(),
        held_back: Vec::new(),
        not_chosen: Vec::new(),
        unavailable: Vec::new(),
        orphaned_entries: Vec::new(),
    };
    let hold = |report: &mut MergeReport, item: MergeItemRef, reason: &str| {
        report.held_back.push(MergeHeldBack {
            item,
            reason: reason.to_string(),
        })
    };

    // Package definitions never come in.
    let defs = diff_keyed(&definitions(&tgt), &definitions(&src));
    for id in &defs.differ {
        report.differ.push(item("definition", id));
    }
    for id in &defs.only_in_to {
        hold(
            &mut report,
            item("definition", id),
            "package-definition-not-brought",
        );
    }

    // Attachments come in with what references them, and never over a file the target holds.
    // Decided first: an item whose attachment cannot come in cannot come in either (RFC-017 R2).
    let source_documents_base = target
        .load_manifest()?
        .source_documents_path
        .unwrap_or_else(|| DEFAULT_SOURCE_DOCUMENTS.to_string());
    let document_values = |s: &RepositorySnapshot| -> BTreeMap<String, Value> {
        s.source_documents
            .iter()
            .map(|d| {
                (
                    d.document_id.clone(),
                    serde_json::json!([d.sidecar, d.content_base64]),
                )
            })
            .collect()
    };
    let documents = diff_keyed(&document_values(&tgt), &document_values(&src));
    for id in &documents.differ {
        report.differ.push(item("attachment", id));
    }
    let taken = target.list_files_recursive(&source_documents_base);
    let is_taken = |rel: &str| {
        let full = format!("{source_documents_base}/{rel}");
        taken.contains(&full)
    };
    let source_document = |id: &str| src.source_documents.iter().find(|d| d.document_id == id);
    let target_documents: BTreeSet<&str> = tgt
        .source_documents
        .iter()
        .map(|d| d.document_id.as_str())
        .collect();
    let document_missing = |value: &Value| {
        let refs = value.get("sourceRefs").and_then(Value::as_array);
        refs.into_iter().flatten().any(|r| {
            let Some(id) = r.get("sourceId").and_then(Value::as_str) else {
                return false;
            };
            let found = source_document(id);
            let attaches = r.get("sourceRole").and_then(Value::as_str) == Some("attaches");
            (attaches || found.is_some())
                && !target_documents.contains(id)
                && found.is_none_or(|d| is_taken(&d.sidecar_path) || is_taken(&d.content_path))
        })
    };

    // Instances.
    let package = target.load_package()?;
    let instance_values = |s: &RepositorySnapshot| -> BTreeMap<String, Value> {
        s.instances
            .iter()
            .map(|i| (i.instance_id.clone(), i.value.clone()))
            .collect()
    };
    let target_instances = instance_values(&tgt);
    let instances = diff_keyed(&target_instances, &instance_values(&src));
    for id in &instances.differ {
        report.differ.push(item("instance", id));
    }
    let new_instances: BTreeSet<&str> = instances.only_in_to.iter().map(String::as_str).collect();
    let mut plan_instances = Vec::new();
    let mut documents_needed = BTreeSet::new();
    for i in src
        .instances
        .iter()
        .filter(|i| new_instances.contains(i.instance_id.as_str()))
    {
        let id = i.instance_id.as_str();
        if !chosen.instance(id) {
            report.not_chosen.push(item("instance", id));
            continue;
        }
        if crate::store::require_canonical_instance_id(id).is_err() {
            hold(&mut report, item("instance", id), "id-not-canonical");
            continue;
        }
        if document_missing(&i.value) {
            hold(&mut report, item("instance", id), "attachment-unavailable");
            continue;
        }
        if i.tier != 0 {
            let type_id = i.value["typeId"].as_str().unwrap_or_default();
            let version = i.value["typeVersion"].as_u64();
            let resolves = package
                .record_types
                .iter()
                .any(|t| t.id == type_id && Some(u64::from(t.version)) == version);
            if !resolves {
                hold(&mut report, item("instance", id), "type-not-resolved");
                continue;
            }
        }
        referenced_documents(&i.value, &mut documents_needed);
        report.bring.instances.push(MergeInstance {
            instance_id: i.instance_id.clone(),
            tier: i.tier,
            title: i.title.as_ref().and_then(Value::as_str).map(str::to_string),
            created_by: i.value.get(actor_service::CREATED_BY_KEY).cloned(),
        });
        plan_instances.push((i.tier, i.value.clone()));
    }
    let brought_instances: BTreeSet<String> = report
        .bring
        .instances
        .iter()
        .map(|i| i.instance_id.clone())
        .collect();
    let exists = |id: &str| target_instances.contains_key(id) || brought_instances.contains(id);

    // Relations.
    let relation_values = |s: &RepositorySnapshot| -> BTreeMap<String, Value> {
        s.relations
            .iter()
            .map(|r| (r.relation_id.clone(), to_value(r)))
            .collect()
    };
    let relations = diff_keyed(&relation_values(&tgt), &relation_values(&src));
    for id in &relations.differ {
        report.differ.push(item("relation", id));
    }
    let new_relations: BTreeSet<&str> = relations.only_in_to.iter().map(String::as_str).collect();
    let mut plan_relations = Vec::new();
    for r in src
        .relations
        .iter()
        .filter(|r| new_relations.contains(r.relation_id.as_str()))
    {
        let id = r.relation_id.as_str();
        if !chosen.relation(id) {
            report.not_chosen.push(item("relation", id));
            continue;
        }
        let def = package
            .relation_type_definitions
            .iter()
            .find(|d| d.key == r.relation_type && d.resolves_for_reads());
        let reason = match def {
            _ if crate::store::require_canonical_relation_id(id).is_err() => {
                Some("id-not-canonical")
            }
            _ if document_missing(&to_value(r)) => Some("attachment-unavailable"),
            None => Some("relation-type-not-resolved"),
            Some(d) if !d.accepts_new_relations() => Some("relation-type-closed"),
            Some(_) if !exists(&r.source_instance_id) || !exists(&r.target_instance_id) => {
                Some("relation-endpoint-missing")
            }
            Some(_) => None,
        };
        if let Some(reason) = reason {
            hold(&mut report, item("relation", id), reason);
            continue;
        }
        referenced_documents(&to_value(r), &mut documents_needed);
        report.bring.relations.push(MergeRelation {
            relation_id: r.relation_id.clone(),
            relation_type: r.relation_type.clone(),
            source_instance_id: r.source_instance_id.clone(),
            target_instance_id: r.target_instance_id.clone(),
        });
        plan_relations.push(r.clone());
    }

    // Containers: the root is matched by id and its head is never written (RFC-052).
    let target_containers = containers(&tgt);
    let source_containers = containers(&src);
    let target_root = tgt.root_container.as_ref();
    let source_root_id = src.root_container.as_ref().map(|c| c.container_id.as_str());
    let foreign_root = source_root_id
        .filter(|id| target_root.is_none_or(|t| t.container_id != *id))
        .filter(|id| !target_containers.contains_key(id));
    let head = |store: &dyn RepositoryStore, id: &str| store.load_instance_json(&head_path(id));

    let mut stored: BTreeMap<&str, Vec<StoredEntry>> = BTreeMap::new();
    for id in source_containers.keys() {
        stored.insert(id, container_store::read_entries(source, id)?);
    }

    let mut new_containers: BTreeSet<&str> = BTreeSet::new();
    let mut held_containers: BTreeMap<&str, &str> = BTreeMap::new();
    for id in source_containers.keys() {
        if Some(*id) == foreign_root {
            held_containers.insert(id, "root-container-differs");
        } else if target_containers.contains_key(id) {
            if head(source, id)? != head(target, id)? {
                report.differ.push(item("container", id));
            }
        } else if chosen.container(id) {
            new_containers.insert(id);
        } else {
            report.not_chosen.push(item("container", id));
        }
    }

    // The entries the target lacks that can come in, given their container is here.
    let mut candidate: BTreeMap<&str, Vec<&StoredEntry>> = BTreeMap::new();
    let mut entry_reasons: Vec<(MergeItemRef, &str)> = Vec::new();
    let mut target_entry_ids: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (id, entries) in &stored {
        let in_target: BTreeMap<String, StoredEntry> = if target_containers.contains_key(id) {
            container_store::read_entries(target, id)?
                .into_iter()
                .map(|e| (e.instance_id.clone(), e))
                .collect()
        } else {
            BTreeMap::new()
        };
        target_entry_ids.insert(id, in_target.keys().cloned().collect());
        let is_root = target_root.is_some_and(|t| t.container_id == *id);
        let root_identity = target_root
            .filter(|_| is_root)
            .and_then(|t| t.identity_instance_id.as_deref());
        for e in entries {
            if let Some(existing) = in_target.get(&e.instance_id) {
                if existing != e {
                    report.differ.push(entry_item(id, &e.instance_id));
                }
                continue;
            }
            if !chosen.entry(id, &e.instance_id) {
                report.not_chosen.push(entry_item(id, &e.instance_id));
                continue;
            }
            let reason = if is_root
                && source_containers[id].identity_instance_id.as_deref()
                    == Some(e.instance_id.as_str())
            {
                Some("root-identity-entry")
            } else if root_identity.is_some() && e.parent.as_deref() == root_identity {
                Some("parent-is-root-identity")
            } else if !exists(&e.instance_id) {
                Some("entry-record-missing")
            } else {
                None
            };
            match reason {
                Some(r) => entry_reasons.push((entry_item(id, &e.instance_id), r)),
                None => candidate.entry(id).or_default().push(e),
            }
        }
    }

    // A new container needs its identity and anchor entries and its child containers here too
    // (I-81, I-145, RFC-038 [R13]). Holding one back can strand its parent, so run to a fixpoint.
    loop {
        let mut newly_held: Vec<(&str, &str)> = Vec::new();
        for id in &new_containers {
            let c = source_containers[id];
            let has_entry = |wanted: &Option<String>| {
                wanted.as_deref().is_none_or(|w| {
                    candidate
                        .get(id)
                        .is_some_and(|es| es.iter().any(|e| e.instance_id == w))
                })
            };
            let children_here = c.child_container_ids.iter().flatten().all(|child| {
                target_containers.contains_key(child.as_str())
                    || new_containers.contains(child.as_str())
            });
            if !has_entry(&c.identity_instance_id) || !has_entry(&c.anchor_instance_id) {
                newly_held.push((id, "container-anchor-missing"));
            } else if !children_here {
                newly_held.push((id, "container-child-missing"));
            }
        }
        if newly_held.is_empty() {
            break;
        }
        for (id, reason) in newly_held {
            new_containers.remove(id);
            held_containers.insert(id, reason);
        }
    }
    for (id, reason) in &held_containers {
        hold(&mut report, item("container", id), reason);
    }

    let mut plan_heads = Vec::new();
    for id in &new_containers {
        plan_heads.push((id.to_string(), head(source, id)?));
        report.bring.containers.push(MergeContainer {
            container_id: id.to_string(),
            title: source_containers[id].title.clone(),
        });
    }
    for (entry, reason) in entry_reasons {
        hold(&mut report, entry, reason);
    }
    let mut plan_entries = Vec::new();
    for (id, entries) in &candidate {
        if !new_containers.contains(id) && !target_containers.contains_key(id) {
            let reason = if held_containers.contains_key(id) {
                "container-held-back"
            } else {
                "container-not-chosen"
            };
            for e in entries {
                hold(&mut report, entry_item(id, &e.instance_id), reason);
            }
            continue;
        }
        let in_target = &target_entry_ids[id];
        let here: BTreeSet<&str> = entries.iter().map(|e| e.instance_id.as_str()).collect();
        // Listed in the source's outline order, so a preview reads as the document does.
        let outline = source_containers[id]
            .member_instance_ids
            .as_deref()
            .unwrap_or_default();
        let position = |iid: &str| outline.iter().position(|o| o.instance_id == iid);
        let mut ordered = entries.clone();
        ordered.sort_by_key(|e| position(&e.instance_id).unwrap_or(usize::MAX));
        for e in ordered {
            if let Some(p) = e.parent.as_deref() {
                if !here.contains(p) && !in_target.contains(p) {
                    report.orphaned_entries.push(MergeEntryId {
                        container_id: id.to_string(),
                        instance_id: e.instance_id.clone(),
                    });
                }
            }
            report.bring.container_entries.push(MergeEntry {
                container_id: id.to_string(),
                instance_id: e.instance_id.clone(),
                depth: position(&e.instance_id)
                    .map(|p| outline[p].depth())
                    .unwrap_or(0),
            });
            plan_entries.push((id.to_string(), e.clone()));
        }
    }

    let mut plan_attachments = Vec::new();
    for id in &documents.only_in_to {
        let Some(d) = source_document(id) else {
            continue;
        };
        if !documents_needed.contains(id) {
            hold(
                &mut report,
                item("attachment", id),
                "attachment-not-referenced",
            );
        } else if is_taken(&d.sidecar_path) || is_taken(&d.content_path) {
            hold(&mut report, item("attachment", id), "attachment-path-taken");
        } else {
            report.bring.attachments.push(MergeAttachment {
                document_id: id.clone(),
                title: d.title.clone(),
            });
            plan_attachments.push(d.clone());
        }
    }

    // What was chosen and cannot come in.
    if let Some(s) = selection {
        let unavailable = |wanted: &[String], kind: &str, brought: &BTreeSet<&str>| {
            wanted
                .iter()
                .filter(|id| !brought.contains(id.as_str()))
                .map(|id| item(kind, id))
                .collect::<Vec<_>>()
        };
        let instance_ids: BTreeSet<&str> = brought_instances.iter().map(String::as_str).collect();
        let relation_ids: BTreeSet<&str> = report
            .bring
            .relations
            .iter()
            .map(|r| r.relation_id.as_str())
            .collect();
        let mut out = unavailable(&s.instance_ids, "instance", &instance_ids);
        out.extend(unavailable(&s.relation_ids, "relation", &relation_ids));
        out.extend(unavailable(&s.container_ids, "container", &new_containers));
        out.extend(
            s.container_entries
                .iter()
                .filter(|e| {
                    !report
                        .bring
                        .container_entries
                        .iter()
                        .any(|b| b.container_id == e.container_id && b.instance_id == e.instance_id)
                })
                .map(|e| entry_item(&e.container_id, &e.instance_id)),
        );
        report.unavailable = out;
    }

    report.differ.sort();
    report.not_chosen.sort();
    report.held_back.sort_by(|a, b| a.item.cmp(&b.item));
    Ok(Plan {
        report,
        instances: plan_instances,
        relations: plan_relations,
        heads: plan_heads,
        entries: plan_entries,
        attachments: plan_attachments,
        source_documents_base,
    })
}

/// Writes in dependency order: attachments, instances, relations, then containers.
fn write_plan(target: &dyn RepositoryStore, plan: &Plan) -> Result<(), RepositoryError> {
    let base = &plan.source_documents_base;
    for d in &plan.attachments {
        let sidecar_full = format!("{base}/{}", d.sidecar_path);
        let sidecar = serde_json::to_string_pretty(&d.sidecar).map_err(|source| {
            RepositoryError::Serialize {
                path: sidecar_full.clone().into(),
                source,
            }
        })?;
        target.save_text_file(&sidecar_full, &sidecar)?;
        if let Some(b64) = &d.content_base64 {
            let bytes = BASE64
                .decode(b64)
                .map_err(|e| RepositoryError::InvalidSnapshotData {
                    message: format!("base64 decode failed for '{}': {e}", d.content_path),
                })?;
            target.save_binary_file(&format!("{base}/{}", d.content_path), &bytes)?;
        }
    }
    for (tier, value) in &plan.instances {
        let id = value
            .get("instanceId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (tier_dir, slug) = if *tier == 0 {
            (
                target.record_tier_dir(RecordTier::Note),
                value.get("title").and_then(Value::as_str),
            )
        } else {
            (
                target.record_tier_dir(RecordTier::Tier2),
                value.get("typeName").and_then(Value::as_str),
            )
        };
        crate::store::save_new_instance(target, id, value, tier_dir, slug.unwrap_or_default())?;
    }
    for relation in &plan.relations {
        target.save_relation(relation)?;
    }
    for (id, head) in &plan.heads {
        container_store::write_carried_head(target, id, head)?;
    }
    for (id, entry) in &plan.entries {
        container_store::write_carried_entry(target, id, entry)?;
    }
    Ok(())
}
