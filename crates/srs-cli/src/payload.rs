//! # CLI Payload Types
//!
//! This module defines the authoritative payload shapes for every CLI command output.
//! All command handlers must serialize their results through these types rather than
//! constructing anonymous `json!({...})` literals.
//!
//! ## Contract
//!
//! - Each struct is the single source of truth for what a command's `payload` field contains.
//! - `#[serde(rename_all = "camelCase")]` on every struct ensures JSON keys are camelCase.
//! - Structs that wrap existing service types (e.g. `NotePayload`) produce identical JSON
//!   to the previous `json!({ "note": note })` literals.
//! - Structs with explicit sub-types (e.g. `NoteListEntry`) preserve the exact field subset
//!   that was previously emitted; they do NOT expose internal service fields not in the
//!   previous output.
//! - `#[derive(JsonSchema)]` on every struct powers Phase 2 golden schema generation and CI.
//!   External embedded types that do not implement `JsonSchema` are annotated with
//!   `#[schemars(with = "serde_json::Value")]` so the outer wrapper schema is still generated.

use schemars::JsonSchema;
use serde::Serialize;
use srs_core::types::{
    container::Container,
    note::Note,
    record::Record,
    record_type::RecordType,
    relation::Relation,
    view::{DocumentView, View},
};
use srs_repository::{
    analysis::{FoundationNoteSet, RepoMap, TagAudit},
    container_service::ContainerSummary,
    extension_service::ExtensionSummary,
    relation_service::RelationSummary,
    services::{ListNoteTagsResult, NoteSummary, TagSummary},
    tag_service::TagDefinitionSummary,
    validation::{RepositoryValidationReport, ValidationSummary},
    view_service::{DocumentViewSummary, ViewSummary},
};
use std::path::PathBuf;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// A single entry in a note list — only `instanceId` and `title` are exposed.
/// The full `NoteSummary` type is an internal service detail.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteListEntry {
    pub instance_id: String,
    pub title: String,
}

impl From<NoteSummary> for NoteListEntry {
    fn from(s: NoteSummary) -> Self {
        Self {
            instance_id: s.instance_id,
            title: s.title.unwrap_or_default(),
        }
    }
}

/// A single entry in a field list — the subset of `FieldSummary` exposed by the CLI.
/// (Omits `valueType` and `description` which were never in the prior output.)
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldListEntry {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub source_package: Option<String>,
}

/// A single entry in a type list — the subset of `TypeSummary` exposed by the CLI.
/// (Omits `description` which was never in the prior output.)
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeListEntry {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub field_count: usize,
    pub source_package: Option<String>,
}

/// A single entry in a protocol list.
/// Maps `ProtocolSummary` fields with the renaming that the prior handler applied
/// (e.g. `protocol_namespace` → `namespace`, `protocol_version` → `version`).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolListEntry {
    pub instance_id: String,
    pub protocol_id: String,
    pub namespace: String,
    pub name: String,
    pub version: i32,
    pub stage_count: usize,
}

/// A single entry in a protocol stages list.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStageEntry {
    pub stage_id: String,
    pub name: String,
    pub order: i32,
    pub depends_on: Vec<String>,
}

/// A single entry in a blueprint list.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintListEntry {
    pub blueprint_id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub root_type_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

/// A single entry in a blueprint structure list (RelationSpec).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationSpecEntry {
    pub relation_type: String,
    pub source_type_id: String,
    pub target_type_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// A single entry in a package list.
/// Maps `PackageBoundaryInfo` with `boundary_path` renamed to `boundaryPath`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageListEntry {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub boundary_path: Option<String>,
    pub field_count: usize,
    pub type_count: usize,
}

/// A single entry in a package refs list (enable/disable output).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageRefEntry {
    pub mode: String,
    pub path: String,
}

/// A single tag entry in a note-tag list.
/// Mirrors `TagSummary` from srs-repository with a local type for JsonSchema derivation.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteTagEntry {
    pub tag: String,
    pub note_count: usize,
}

impl From<TagSummary> for NoteTagEntry {
    fn from(t: TagSummary) -> Self {
        Self {
            tag: t.tag,
            note_count: t.note_count,
        }
    }
}

/// Summary row in a repo validate payload.
/// Mirrors `ValidationSummary` from srs-repository with a local type for JsonSchema.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoValidateSummary {
    pub checked: usize,
    pub errors: usize,
    pub warnings: usize,
}

impl From<ValidationSummary> for RepoValidateSummary {
    fn from(s: ValidationSummary) -> Self {
        Self {
            checked: s.checked,
            errors: s.errors,
            warnings: s.warnings,
        }
    }
}

// ── Note payloads ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteListPayload {
    pub notes: Vec<NoteListEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotePayload {
    #[schemars(with = "serde_json::Value")]
    pub note: Note,
}

/// Shared by note/record/tag/extension delete (all use `instanceId`).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeletedPayload {
    pub instance_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteTagAddPayload {
    #[schemars(with = "serde_json::Value")]
    pub note: Note,
    pub tag: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteTagRemovePayload {
    #[schemars(with = "serde_json::Value")]
    pub note: Note,
    pub tag: String,
    pub removed: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteTagListPayload {
    pub total_notes: usize,
    pub tags: Vec<NoteTagEntry>,
}

impl From<ListNoteTagsResult> for NoteTagListPayload {
    fn from(r: ListNoteTagsResult) -> Self {
        Self {
            total_notes: r.total_notes,
            tags: r.tags.into_iter().map(NoteTagEntry::from).collect(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteTagMapPayload {
    #[schemars(with = "serde_json::Value")]
    pub tag_audit: TagAudit,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteFoundationsPayload {
    #[schemars(with = "serde_json::Value")]
    pub foundation_notes: FoundationNoteSet,
}

// ── Record payloads ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub records: Vec<Record>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordPayload {
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
}

// ── Relation payloads ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationListPayload {
    /// Uses `RelationSummary` directly — its `serde(rename_all = "camelCase")` produces
    /// `{ "relationId", "relationType", "sourceId", "targetId" }` which matches the
    /// previous hand-rolled `json!()` output exactly.
    #[schemars(with = "Vec<serde_json::Value>")]
    pub relations: Vec<RelationSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationPayload {
    #[schemars(with = "serde_json::Value")]
    pub relation: Relation,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationDeletePayload {
    pub relation_id: String,
    pub path: String,
}

// ── Container payloads ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub containers: Vec<ContainerSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerPayload {
    #[schemars(with = "serde_json::Value")]
    pub container: Container,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerDeletePayload {
    pub container_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMembersPayload {
    pub container_id: String,
    pub member_instance_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMembersMutatePayload {
    pub container_id: String,
    pub instance_id: String,
    pub member_instance_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerRootsPayload {
    pub container_id: String,
    pub root_instance_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerRootsMutatePayload {
    pub container_id: String,
    pub instance_id: String,
    pub root_instance_ids: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerValidatePayload {
    pub ok: bool,
    pub errors: Vec<String>,
}

// ── Tag payloads ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub tag_definitions: Vec<TagDefinitionSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagPayload {
    #[schemars(with = "serde_json::Value")]
    pub tag_definition: srs_core::types::tag_definition::TagDefinition,
}

// ── Field payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldListPayload {
    pub fields: Vec<FieldListEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldPayload {
    #[schemars(with = "serde_json::Value")]
    pub field: srs_core::types::field::Field,
}

// ── Type payloads ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeListPayload {
    pub types: Vec<TypeListEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypePayload {
    /// Serialized as `"type"` in JSON.
    #[serde(rename = "type")]
    #[schemars(rename = "type")]
    #[schemars(with = "serde_json::Value")]
    pub record_type: RecordType,
}

// ── Extension payloads ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub extensions: Vec<ExtensionSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPayload {
    #[schemars(with = "serde_json::Value")]
    pub extension: Record,
}

// ── Protocol payloads ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolListPayload {
    pub protocols: Vec<ProtocolListEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolPayload {
    pub protocol: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStagesPayload {
    pub stages: Vec<ProtocolStageEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolValidatePayload {
    pub instance_id: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDeletePayload {
    pub instance_id: String,
}

// ── Blueprint payloads ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintListPayload {
    pub blueprints: Vec<BlueprintListEntry>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintPayload {
    #[schemars(with = "serde_json::Value")]
    pub blueprint: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintDeletePayload {
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintValidatePayload {
    pub id: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintStructurePayload {
    pub relation_specs: Vec<RelationSpecEntry>,
}

// ── View payloads ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub views: Vec<ViewSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewPayload {
    #[schemars(with = "serde_json::Value")]
    pub view: View,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ViewDeletePayload {
    pub id: String,
}

// ── Document-view payloads ────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub document_views: Vec<DocumentViewSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewPayload {
    #[schemars(with = "serde_json::Value")]
    pub document_view: DocumentView,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewDeletePayload {
    pub id: String,
}

// ── Render payloads ───────────────────────────────────────────────────────────

/// A single field-group entry in a JSON projection record.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedGroupEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    pub fields: serde_json::Value,
}

/// A projected field group (one group definition + its record data).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedFieldGroup {
    pub group_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub entries: Vec<ProjectedGroupEntry>,
}

/// A single record in a JSON projection section.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedRecord {
    pub instance_id: String,
    pub type_id: String,
    pub type_namespace: String,
    pub type_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    pub fields: serde_json::Value,
    pub ordered_field_keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_groups: Option<Vec<ProjectedFieldGroup>>,
}

/// A single section in a JSON projection document.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedSection {
    pub section_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub order: i32,
    pub records: Vec<ProjectedRecord>,
}

/// The top-level JSON projection object for a rendered document view.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewProjection {
    #[serde(rename = "$schema")]
    #[schemars(rename = "$schema")]
    pub schema: String,
    pub document_view_id: String,
    pub container_id: Option<String>,
    pub generated_at: String,
    pub container_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    pub sections: Vec<ProjectedSection>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RenderDocumentViewPayload {
    pub rendered: String,
    pub diagnostics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projection: Option<DocumentViewProjection>,
}

// ── Repo payloads ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoCreatePayload {
    pub repo_root: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapPayload {
    #[schemars(with = "serde_json::Value")]
    pub repo_map: RepoMap,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoCopyPayload {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoValidatePayload {
    /// Diagnostic entries serialized from `ValidationDiagnostic` objects.
    /// Each entry contains `severity`, `path`, `schemaId?`, and `message`.
    pub diagnostics: Vec<serde_json::Value>,
    pub summary: RepoValidateSummary,
}

impl From<RepositoryValidationReport> for RepoValidatePayload {
    fn from(r: RepositoryValidationReport) -> Self {
        let diagnostics = r
            .diagnostics
            .into_iter()
            .map(|d| serde_json::to_value(d).unwrap_or(serde_json::Value::Null))
            .collect();
        Self {
            diagnostics,
            summary: r.summary.into(),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoExtensionsPayload {
    pub extensions: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoExtensionsMutatePayload {
    pub extension_id: String,
    pub extensions: Vec<String>,
}

// ── Package payloads ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageListPayload {
    pub packages: Vec<PackageListEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageCreatePayload {
    pub id: String,
    pub boundary_path: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageImportPayload {
    pub selector: Option<String>,
    pub id: String,
    pub namespace: String,
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageUpdatePayload {
    pub selector: Option<String>,
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageRefPayload {
    pub path: String,
    pub packages: Vec<PackageRefEntry>,
}
