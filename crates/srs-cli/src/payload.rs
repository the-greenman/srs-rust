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
    blueprint::TypeRef,
    container::Container,
    lifecycle::Lifecycle,
    note::Note,
    record::Record,
    record_type::RecordType,
    relation::Relation,
    relation_type_definition::RelationTypeDefinition,
    term::Term,
    theme::Theme,
    view::{DocumentView, View},
    vocabulary::Vocabulary,
};
use srs_repository::{
    analysis::{FoundationNoteSet, RepoMap, TagAudit},
    container_service::ContainerSummary,
    container_view_service::ContainerView,
    discovery_service::DiscoveryResult,
    extension_service::ExtensionSummary,
    protocol_run_service::RunSummary,
    record_store::{
        AllowedLifecycleTransitionsResult, LifecycleTransitionOption, ListRecordTagsResult,
        RecordSummary, RecordTagSummary,
    },
    relation_service::RelationSummary,
    repository_navigation_service::RepositoryNavigation,
    services::{ListNoteTagsResult, NoteSummary, TagSummary},
    theme_service::ThemeSummary,
    validation::{RepositoryValidationReport, ValidationSummary},
    view_service::{DocumentViewSummary, ViewSummary},
    vocabulary_service::TagSetEntry,
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
    pub protocol_id: String,
    pub namespace: String,
    pub name: String,
    pub version: i32,
    pub stage_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_package: Option<String>,
}

/// A single entry in a protocol stages list.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolStageEntry {
    pub stage_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub order: i32,
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_criteria: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributes_to: Option<Vec<FieldRef>>,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<TypeRef>,
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

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteGraduatePayload {
    #[schemars(with = "serde_json::Value")]
    pub note: Note,
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
}

// ── Record payloads ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordListPayload {
    /// Each entry is a `RecordSummary` — the full `Record` plus its core-resolved
    /// `displayLabel` (same resolution `srs tree` uses). Embedded opaquely as JSON
    /// since `RecordSummary` lives in `srs-repository` and is not `JsonSchema`.
    #[schemars(with = "Vec<serde_json::Value>")]
    pub records: Vec<RecordSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordPayload {
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
}

/// Payload for `record get` — the full `Record` plus its core-resolved `displayLabel`.
/// Shape mirrors `RecordSummary` from srs-repository.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordGetPayload {
    pub instance_id: String,
    pub display_label: String,
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
}

impl From<RecordSummary> for RecordGetPayload {
    fn from(s: RecordSummary) -> Self {
        Self {
            instance_id: s.instance_id,
            display_label: s.display_label,
            record: s.record,
        }
    }
}

/// Payload for `record successor` — the new Record and the Relation to the predecessor.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordSuccessorPayload {
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
    #[schemars(with = "serde_json::Value")]
    pub relation: Relation,
}

/// Payload for `record transition` — the updated record and any non-fatal warnings
/// (e.g., LIFECYCLE_FINAL_STATE). Envelope `diagnostics` is reserved for `ok: false` errors;
/// non-fatal operational signals on a successful response belong in the typed payload per ADR-011.
/// When the transition was fulfilled (RFC-022), `successor` / `relation` carry the artifacts.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordTransitionPayload {
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub successor: Option<Record>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub relation: Option<Relation>,
}

/// One allowed lifecycle transition for `record allowed-transitions`.
/// `requiresRelation` (RFC-022) is the target state's relation obligation, when declared.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AllowedTransitionEntry {
    pub name: String,
    pub to: String,
    pub to_is_final: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>")]
    pub requires_relation: Option<srs_core::types::lifecycle::RequiresRelation>,
}

impl From<LifecycleTransitionOption> for AllowedTransitionEntry {
    fn from(t: LifecycleTransitionOption) -> Self {
        Self {
            name: t.name,
            to: t.to,
            to_is_final: t.to_is_final,
            requires_relation: t.requires_relation,
        }
    }
}

/// Payload for `record allowed-transitions` — current lifecycle state, permitted next
/// transitions, and whether the record is in a final (immutable) state.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordAllowedTransitionsPayload {
    pub current_state: String,
    pub transitions: Vec<AllowedTransitionEntry>,
    pub is_immutable: bool,
}

impl From<AllowedLifecycleTransitionsResult> for RecordAllowedTransitionsPayload {
    fn from(r: AllowedLifecycleTransitionsResult) -> Self {
        Self {
            current_state: r.current_state,
            transitions: r
                .transitions
                .into_iter()
                .map(AllowedTransitionEntry::from)
                .collect(),
            is_immutable: r.is_immutable,
        }
    }
}

/// Payload for `record tag add` and `record tag remove`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordTagAddPayload {
    #[schemars(with = "serde_json::Value")]
    pub record: Record,
    pub tag: String,
}

/// Per-tag count entry for `record tag list`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordTagEntry {
    pub tag: String,
    pub record_count: usize,
}

impl From<RecordTagSummary> for RecordTagEntry {
    fn from(s: RecordTagSummary) -> Self {
        Self {
            tag: s.tag,
            record_count: s.record_count,
        }
    }
}

/// Payload for `record tag list`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordTagListPayload {
    pub total_records: usize,
    pub tags: Vec<RecordTagEntry>,
}

impl From<ListRecordTagsResult> for RecordTagListPayload {
    fn from(r: ListRecordTagsResult) -> Self {
        Self {
            total_records: r.total_records,
            tags: r.tags.into_iter().map(RecordTagEntry::from).collect(),
        }
    }
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

// ── Relation-type payloads ────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationTypeListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub relation_type_definitions: Vec<RelationTypeDefinition>,
}

/// Shared by relation-type get, create, and update (identical shapes).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationTypePayload {
    #[schemars(with = "serde_json::Value")]
    pub relation_type_definition: RelationTypeDefinition,
}

/// Uses `id` (not `instanceId`) — relation-type definitions are package definitions,
/// not instance-index members (ADR-009).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RelationTypeDeletePayload {
    pub id: String,
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

/// Payload for `container resolve-view` (issue #254, #256).
///
/// Carries the structured container view: the container root record, ordered member
/// records (Tier-0, Tier-1, or Tier-2; full `Record` present only for Tier-2), the
/// DocumentView-driven column spec, and non-fatal diagnostics.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContainerViewPayload {
    #[schemars(with = "serde_json::Value")]
    pub container_view: ContainerView,
}

/// Payload for `find` — `ext:discovery` results: ordered hits, total, diagnostics.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FindPayload {
    #[schemars(with = "serde_json::Value")]
    pub result: DiscoveryResult,
}

/// Payload for `record validate` — no-write record input validation (preflight).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordValidatePayload {
    pub ok: bool,
    pub errors: Vec<String>,
}

// ── Tag payloads ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub terms: Vec<Term>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagPayload {
    #[schemars(with = "serde_json::Value")]
    pub term: Term,
}

// ── Vocabulary payloads ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub vocabularies: Vec<Vocabulary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum VocabularyGetPayload {
    #[serde(rename = "found")]
    Found {
        #[schemars(with = "serde_json::Value")]
        vocabulary: Box<Vocabulary>,
    },
    #[serde(rename = "not_found")]
    NotFound { id: String },
}

/// Payload for `vocabulary create`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyCreatePayload {
    #[schemars(with = "serde_json::Value")]
    pub vocabulary: Vocabulary,
}

/// Payload for `vocabulary term-create`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TermCreatePayload {
    #[schemars(with = "serde_json::Value")]
    pub term: Term,
    #[schemars(with = "serde_json::Value")]
    pub vocabulary: Vocabulary,
}

// ── Lifecycle payloads ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub lifecycles: Vec<Lifecycle>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum LifecycleGetPayload {
    #[serde(rename = "found")]
    Found {
        #[schemars(with = "serde_json::Value")]
        lifecycle: Box<Lifecycle>,
    },
    #[serde(rename = "not_found")]
    NotFound { id: String },
}

/// Payload for `lifecycle create`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleCreatePayload {
    #[schemars(with = "serde_json::Value")]
    pub lifecycle: Lifecycle,
}

// ── Term payloads (RFC-006) ───────────────────────────────────────────────────

/// Payload for `term list`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TermListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub terms: Vec<Term>,
}

/// Payload for `term get`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum TermGetPayload {
    #[serde(rename = "found")]
    Found {
        #[schemars(with = "serde_json::Value")]
        term: Box<Term>,
    },
    #[serde(rename = "not_found")]
    NotFound { id: String },
}

/// Payload for `vocabulary promote`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromoteVocabularyPayload {
    #[schemars(with = "serde_json::Value")]
    pub vocabulary: Vocabulary,
}

/// Error payload for `vocabulary promote` when the V10 pre-flight blocks promotion.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PromoteVocabularyBlockedPayload {
    pub vocabulary_id: String,
    pub unresolvable_keys: Vec<String>,
}

/// Payload for `vocabulary derive-tag-set`.
///
/// Lists every in-use tag key and classifies it against the vocabulary's
/// effective terms (V10 pre-flight), so an author can inspect the live usage
/// state of an open vocabulary before promoting it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyDeriveTagSetPayload {
    #[schemars(with = "serde_json::Value")]
    pub vocabulary: Vocabulary,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub entries: Vec<TagSetEntry>,
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

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldDeletePayload {
    pub id: String,
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

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeDeletePayload {
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypeSchemaPayload {
    /// A draft-07 JSON Schema describing a record's `fieldValues` for this Type.
    /// Dynamic shape; emitted opaquely (see ADR-011 for the dynamic-value convention).
    #[schemars(with = "serde_json::Value")]
    pub schema: serde_json::Value,
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
    pub protocol_id: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDeletePayload {
    pub protocol_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolFindByTargetTypePayload {
    pub protocol_id: String,
    pub protocol_name: String,
    pub stages: Vec<ProtocolStageEntry>,
    pub diagnostics: Vec<String>,
}

// ── Protocol run payloads ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRunListEntry {
    pub run_id: String,
    pub protocol_id: String,
    pub container_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_stage_id: Option<String>,
    pub started_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRunListPayload {
    pub runs: Vec<ProtocolRunListEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRunPayload {
    #[schemars(with = "serde_json::Value")]
    pub run: serde_json::Value,
}

impl From<RunSummary> for ProtocolRunListEntry {
    fn from(s: RunSummary) -> Self {
        Self {
            run_id: s.run_id,
            protocol_id: s.protocol_id,
            container_id: s.container_id,
            status: s.status,
            current_stage_id: s.current_stage_id,
            started_at: s.started_at,
        }
    }
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

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintSchemaPayload {
    /// A nested draft-07 JSON Schema for the whole multi-record document.
    #[schemars(with = "serde_json::Value")]
    pub schema: serde_json::Value,
    /// Non-fatal projection diagnostics (unresolvable types, unparseable cardinality, etc.).
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefField {
    pub field_id: String,
    pub name: String,
    pub order: u32,
    pub required: bool,
    pub value_type: String,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefType {
    pub type_id: String,
    pub namespace: String,
    pub name: String,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    pub fields: Vec<BriefField>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefRelationSpec {
    pub relation_type: String,
    pub source_type_id: String,
    pub target_type_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cardinality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Payload mirror of `srs_core::types::protocol::FieldRef`.
/// Separate struct because payload types must derive `JsonSchema`; srs-core must not
/// depend on schemars (ADR-011).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldRef {
    pub field_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefStage {
    pub stage_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub order: i32,
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_criteria: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributes_to: Option<Vec<FieldRef>>,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_type: Option<TypeRef>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BriefProtocol {
    pub protocol_id: String,
    pub protocol_name: String,
    pub stages: Vec<BriefStage>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlueprintBriefPayload {
    /// Markdown prose in AI guidance composition order. Always populated.
    pub rendered: String,
    pub blueprint_id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    #[schemars(with = "Option<serde_json::Value>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub required_types: Vec<serde_json::Value>,
    pub types: Vec<BriefType>,
    pub structure: Vec<BriefRelationSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<BriefProtocol>,
    pub diagnostics: Vec<String>,
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

/// Payload for `document-view list-for-container <container-id>`.
///
/// Returns the DocumentViews whose `rootTypeRefs` match the type bound to
/// the container's first root instance. Empty when the root instance has no
/// type binding or when no DocumentViews match.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentViewsForContainerPayload {
    pub container_id: String,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub document_views: Vec<DocumentViewSummary>,
}

// ── Theme payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThemeListPayload {
    #[schemars(with = "Vec<serde_json::Value>")]
    pub themes: Vec<ThemeSummary>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThemePayload {
    #[schemars(with = "serde_json::Value")]
    pub theme: Theme,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThemeDeletePayload {
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
    pub repository_id: String,
    pub package_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_instance_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapPayload {
    #[schemars(with = "serde_json::Value")]
    pub repo_map: RepoMap,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoNavigationPayload {
    #[schemars(with = "serde_json::Value")]
    pub navigation: RepositoryNavigation,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoCopyPayload {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffSummary {
    pub instances_added: usize,
    pub instances_removed: usize,
    pub instances_modified: usize,
    pub relations_added: usize,
    pub relations_removed: usize,
    pub relations_modified: usize,
    pub fields_added: usize,
    pub fields_removed: usize,
    pub fields_modified: usize,
    pub record_types_added: usize,
    pub record_types_removed: usize,
    pub record_types_modified: usize,
    pub blueprints_added: usize,
    pub blueprints_removed: usize,
    pub blueprints_modified: usize,
    pub document_views_added: usize,
    pub document_views_removed: usize,
    pub document_views_modified: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffManifest {
    pub namespace_changed: bool,
    pub srs_version_changed: bool,
    pub extensions_added: Vec<String>,
    pub extensions_removed: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffInstanceAdded {
    pub instance_id: String,
    pub tier: u8,
    #[schemars(with = "serde_json::Value")]
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffInstanceRemoved {
    pub instance_id: String,
    pub tier: u8,
    #[schemars(with = "serde_json::Value")]
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffInstanceModified {
    pub instance_id: String,
    pub tier: u8,
    #[schemars(with = "serde_json::Value")]
    pub from_value: serde_json::Value,
    #[schemars(with = "serde_json::Value")]
    pub to_value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffInstances {
    pub added: Vec<RepoDiffInstanceAdded>,
    pub removed: Vec<RepoDiffInstanceRemoved>,
    pub modified: Vec<RepoDiffInstanceModified>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffRelationAdded {
    pub relation_id: String,
    #[schemars(with = "serde_json::Value")]
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffRelationRemoved {
    pub relation_id: String,
    #[schemars(with = "serde_json::Value")]
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffRelationModified {
    pub relation_id: String,
    #[schemars(with = "serde_json::Value")]
    pub from_value: serde_json::Value,
    #[schemars(with = "serde_json::Value")]
    pub to_value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffRelations {
    pub added: Vec<RepoDiffRelationAdded>,
    pub removed: Vec<RepoDiffRelationRemoved>,
    pub modified: Vec<RepoDiffRelationModified>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffPackageItemAdded {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    #[schemars(with = "serde_json::Value")]
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffPackageItemRemoved {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    #[schemars(with = "serde_json::Value")]
    pub value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffPackageItemModified {
    pub id: String,
    pub namespace: String,
    pub name: String,
    #[schemars(with = "serde_json::Value")]
    pub from_value: serde_json::Value,
    #[schemars(with = "serde_json::Value")]
    pub to_value: serde_json::Value,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffPackageCategory {
    pub added: Vec<RepoDiffPackageItemAdded>,
    pub removed: Vec<RepoDiffPackageItemRemoved>,
    pub modified: Vec<RepoDiffPackageItemModified>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffPackage {
    pub fields: RepoDiffPackageCategory,
    pub record_types: RepoDiffPackageCategory,
    pub blueprints: RepoDiffPackageCategory,
    pub document_views: RepoDiffPackageCategory,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoDiffPayload {
    pub from: PathBuf,
    pub to: PathBuf,
    pub summary: RepoDiffSummary,
    pub manifest: RepoDiffManifest,
    pub instances: RepoDiffInstances,
    pub relations: RepoDiffRelations,
    pub package: RepoDiffPackage,
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

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoExtensionsConformancePayload {
    pub declared: Vec<String>,
    pub supported: Vec<String>,
    pub declared_but_unsupported: Vec<String>,
    pub used_but_undeclared: Vec<String>,
}

impl From<srs_repository::manifest_service::DeclaredExtensionsReport>
    for RepoExtensionsConformancePayload
{
    fn from(r: srs_repository::manifest_service::DeclaredExtensionsReport) -> Self {
        Self {
            declared: r.declared,
            supported: r.supported,
            declared_but_unsupported: r.declared_but_unsupported,
            used_but_undeclared: r.used_but_undeclared,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoSetRootContainerPayload {
    pub container_id: String,
    pub identity_instance_id: String,
    pub title: String,
    pub member_instance_ids: Vec<String>,
}

// ── Revision payloads ─────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevisionListPayload {
    pub instance_id: String,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub revisions: Vec<srs_core::types::revision::Revision>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPayload {
    #[schemars(with = "serde_json::Value")]
    pub revision: srs_core::types::revision::Revision,
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

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageInstallPayload {
    /// Boundary the package was installed into (or already registered at).
    pub boundary_path: String,
    /// Upstream package identity (provenance echo).
    pub package_id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    /// Provenance stamp; preserved across idempotent re-runs.
    pub installed_at: String,
    /// Total definitions written by this run.
    pub installed: usize,
    /// Identical-UUID definitions skipped because they already exist in the repo.
    pub skipped_identical: usize,
    /// Same-key/different-UUID collisions (skipped, not silently duplicated).
    pub conflicts: Vec<PackageInstallConflictEntry>,
    /// Per-kind breakdown for kinds present in the source package.
    pub kinds: Vec<PackageInstallKindEntry>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageInstallConflictEntry {
    pub kind: String,
    pub key: String,
    pub source_id: String,
    pub existing_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageInstallKindEntry {
    pub kind: String,
    pub installed: usize,
    pub skipped_identical: usize,
    pub conflicts: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageImportRecordEntry {
    pub definition_id: String,
    pub definition_type: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub mode: String,
    pub imported_at: String,
    pub source_package_id: String,
    pub source_package_name: String,
    pub source_package_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_known_upstream_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_detected_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_edited_at: Option<String>,
}

impl From<srs_core::extensions::import_tracking::ImportRecord> for PackageImportRecordEntry {
    fn from(r: srs_core::extensions::import_tracking::ImportRecord) -> Self {
        Self {
            definition_id: r.definition_id,
            definition_type: r.definition_type.to_string(),
            namespace: r.namespace,
            name: r.name,
            version: r.version,
            mode: r.mode.to_string(),
            imported_at: r.imported_at,
            source_package_id: r.source_package_id,
            source_package_name: r.source_package_name,
            source_package_version: r.source_package_version,
            latest_known_upstream_version: r.latest_known_upstream_version,
            update_available: r.update_available,
            update_checked_at: r.update_checked_at,
            conflict_state: r.conflict_state.map(|s| s.to_string()),
            conflict_detected_at: r.conflict_detected_at,
            local_version: r.local_version,
            local_edited_at: r.local_edited_at,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageImportsPayload {
    pub generated_at: String,
    pub fields: Vec<PackageImportRecordEntry>,
    pub types: Vec<PackageImportRecordEntry>,
    pub views: Vec<PackageImportRecordEntry>,
    pub blueprints: Vec<PackageImportRecordEntry>,
    pub protocols: Vec<PackageImportRecordEntry>,
    pub relation_types: Vec<PackageImportRecordEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_definitions: Vec<String>,
}

impl From<srs_core::extensions::import_tracking::ImportSummary> for PackageImportsPayload {
    fn from(s: srs_core::extensions::import_tracking::ImportSummary) -> Self {
        Self {
            generated_at: s.generated_at,
            fields: s
                .fields
                .into_iter()
                .map(PackageImportRecordEntry::from)
                .collect(),
            types: s
                .types
                .into_iter()
                .map(PackageImportRecordEntry::from)
                .collect(),
            views: s
                .views
                .into_iter()
                .map(PackageImportRecordEntry::from)
                .collect(),
            blueprints: s
                .blueprints
                .into_iter()
                .map(PackageImportRecordEntry::from)
                .collect(),
            protocols: s
                .protocols
                .into_iter()
                .map(PackageImportRecordEntry::from)
                .collect(),
            relation_types: s
                .relation_types
                .into_iter()
                .map(PackageImportRecordEntry::from)
                .collect(),
            skipped_definitions: s.skipped_definitions,
        }
    }
}

// ── Tree ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TreePayload {
    pub roots: Vec<TreeNodePayload>,
    pub text: String,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TreeNodePayload {
    pub instance_id: String,
    pub label: String,
    pub type_namespace: String,
    pub type_name: String,
    pub lifecycle_state: Option<String>,
    pub depth: u32,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub children: Vec<TreeNodePayload>,
    pub cycle_pruned: bool,
}

// ── Repo init-new payload ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoInitNewPayload {
    pub repository_id: String,
    pub namespace: String,
    pub package_id: String,
    pub package_version: String,
}

// ── Repo upgrade payload ──────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InstancePathRename {
    pub instance_id: String,
    pub from_path: String,
    pub to_path: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoUpgradePayload {
    pub renames: Vec<InstancePathRename>,
    pub total_instances: usize,
    pub already_canonical_count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoMigrateIdentityPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_identity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_identity_tier: Option<u8>,
    pub new_identity_id: String,
    pub statement: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl From<srs_repository::migrate_identity_service::MigrateIdentityResult>
    for RepoMigrateIdentityPayload
{
    fn from(r: srs_repository::migrate_identity_service::MigrateIdentityResult) -> Self {
        Self {
            old_identity_id: r.old_identity_id,
            old_identity_tier: r.old_identity_tier,
            new_identity_id: r.new_identity_id,
            statement: r.statement,
            title: r.title,
        }
    }
}

// ── migration registry payloads ───────────────────────────────────────────────

/// Status of a migration for a specific repository.
/// Contract: exactly one of the three booleans is `true`. This invariant is
/// guaranteed by the `From<MigrationStatus>` impl below and must be preserved
/// for any future `MigrationStatus` variant.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationStatusPayload {
    pub needed: bool,
    pub already_applied: bool,
    pub not_applicable: bool,
}

impl From<srs_repository::migration_registry_service::MigrationStatus> for MigrationStatusPayload {
    fn from(s: srs_repository::migration_registry_service::MigrationStatus) -> Self {
        use srs_repository::migration_registry_service::MigrationStatus;
        Self {
            needed: matches!(s, MigrationStatus::Needed),
            already_applied: matches!(s, MigrationStatus::AlreadyApplied),
            not_applicable: matches!(s, MigrationStatus::NotApplicable),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSummaryPayload {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: MigrationStatusPayload,
}

impl From<srs_repository::migration_registry_service::MigrationSummary>
    for MigrationSummaryPayload
{
    fn from(m: srs_repository::migration_registry_service::MigrationSummary) -> Self {
        Self {
            id: m.id,
            title: m.title,
            description: m.description,
            status: MigrationStatusPayload::from(m.status),
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoMigrationsPayload {
    pub migrations: Vec<MigrationSummaryPayload>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepoApplyMigrationPayload {
    pub id: String,
    #[schemars(with = "serde_json::Value")]
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod migration_payload_tests {
    use super::*;
    use srs_repository::migration_registry_service::MigrationStatus;

    #[test]
    fn migration_status_payload_always_sets_exactly_one_bool() {
        for status in [
            MigrationStatus::Needed,
            MigrationStatus::AlreadyApplied,
            MigrationStatus::NotApplicable,
        ] {
            let p = MigrationStatusPayload::from(status);
            let count = [p.needed, p.already_applied, p.not_applicable]
                .iter()
                .filter(|&&b| b)
                .count();
            assert_eq!(count, 1, "exactly one bool must be true, got {p:?}");
        }
    }
}

// ── ext:registry payloads ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntryPayload {
    pub package_id: String,
    pub package_name: String,
    pub package_version: String,
    pub publisher: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub published_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub field_count: u32,
    pub type_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_type_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

impl From<srs_core::extensions::registry::RegistryEntry> for RegistryEntryPayload {
    fn from(e: srs_core::extensions::registry::RegistryEntry) -> Self {
        Self {
            package_id: e.package_id,
            package_name: e.package_name,
            package_version: e.package_version,
            publisher: e.publisher,
            description: e.description,
            published_at: e.published_at,
            homepage: e.homepage,
            tags: e.tags,
            field_count: e.field_count,
            type_count: e.type_count,
            view_count: e.view_count,
            schema_count: e.schema_count,
            protocol_count: e.protocol_count,
            relation_type_count: e.relation_type_count,
            download_url: e.download_url,
            checksum: e.checksum,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListPayload {
    pub registry_id: String,
    pub registry_name: String,
    pub catalog_version: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub entries: Vec<RegistryEntryPayload>,
    pub total_count: usize,
    pub filtered_count: usize,
}

/// Returned by `srs registry get`; `entry` is non-optional because the service
/// returns `RegistryEntryNotFound` (propagated as an error envelope) when absent.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistryGetPayload {
    pub registry_id: String,
    pub entry: RegistryEntryPayload,
}

// ── ext:federation payloads ───────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationRegistryEntryPayload {
    pub repository_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl From<srs_core::extensions::federation::RepositoryRegistryEntry>
    for FederationRegistryEntryPayload
{
    fn from(e: srs_core::extensions::federation::RepositoryRegistryEntry) -> Self {
        Self {
            repository_id: e.repository_id,
            title: e.title,
            location: e.location,
            last_seen: e.last_seen,
            tags: e.tags,
        }
    }
}

/// Returned by `srs federation resolve`; `found: false` when the ID is absent
/// (graceful degradation — not an error).
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationResolvePayload {
    pub found: bool,
    pub registry_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<FederationRegistryEntryPayload>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationEventPayload {
    pub event_id: String,
    pub event: String,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_repository_id: Option<String>,
    pub affected_instance_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl From<srs_core::extensions::federation::FederationEvent> for FederationEventPayload {
    fn from(e: srs_core::extensions::federation::FederationEvent) -> Self {
        use srs_core::extensions::federation::FederationStrategy;
        use srs_repository::federation_service::federation_event_kind_str;
        Self {
            event_id: e.event_id,
            event: federation_event_kind_str(&e.event).to_string(),
            at: e.at,
            performed_by: e.performed_by,
            source_repository_id: e.source_repository_id,
            target_repository_id: e.target_repository_id,
            affected_instance_ids: e.affected_instance_ids,
            strategy: e.strategy.map(|s| match s {
                FederationStrategy::PreserveIds => "preserve-ids".to_string(),
                FederationStrategy::NewIdsWithLineage => "new-ids-with-lineage".to_string(),
            }),
            note: e.note,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationEventsListPayload {
    pub repository_id: String,
    pub events: Vec<FederationEventPayload>,
    pub total_count: usize,
    pub filtered_count: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FederationAppendEventPayload {
    pub event_id: String,
    pub total_events: usize,
}

// ── Context query payloads ────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextFieldPayload {
    pub record_id: String,
    pub field_id: String,
    pub field_name: Option<String>,
    pub field_namespace: Option<String>,
    pub ai_guidance: Option<serde_json::Value>,
    pub current_value: Option<serde_json::Value>,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub revisions: Vec<srs_core::types::revision::Revision>,
    pub tagged_chunks: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextRecordPayload {
    pub record_id: String,
    pub type_id: String,
    pub type_name: String,
    pub type_namespace: String,
    pub display_label: String,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub field_values: Vec<srs_core::types::record::FieldValue>,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub relations: Vec<srs_repository::relation_service::RelationSummary>,
    pub tagged_chunks: Vec<serde_json::Value>,
    pub protocol_run_history: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextRevisionTracePayload {
    pub record_id: String,
    pub field_id: String,
    #[schemars(with = "serde_json::Value")]
    pub revision: srs_core::types::revision::Revision,
    #[schemars(with = "Vec<serde_json::Value>")]
    pub prior_chain: Vec<srs_core::types::revision::Revision>,
}

// ── Attachment payloads ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentEntry {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidecar_checksum: Option<String>,
}

impl From<srs_repository::attachment_service::AttachmentEntry> for AttachmentEntry {
    fn from(e: srs_repository::attachment_service::AttachmentEntry) -> Self {
        Self {
            path: e.path,
            document_id: e.document_id,
            title: e.title,
            content_checksum: e.content_checksum,
            sidecar_checksum: e.sidecar_checksum,
        }
    }
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentListPayload {
    pub source_documents_path: String,
    pub entries: Vec<AttachmentEntry>,
}

impl From<srs_repository::attachment_service::ListAttachmentsResult> for AttachmentListPayload {
    fn from(r: srs_repository::attachment_service::ListAttachmentsResult) -> Self {
        Self {
            source_documents_path: r.source_documents_path,
            entries: r.entries.into_iter().map(AttachmentEntry::from).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_stage_none_fields_absent_from_json() {
        let stage = BriefStage {
            stage_id: "s1".to_string(),
            name: "Stage One".to_string(),
            purpose: None,
            order: 1,
            depends_on: vec![],
            question: None,
            completion_criteria: None,
            contributes_to: None,
            ai_guidance: None,
            output_type: None,
        };
        let json = serde_json::to_string(&stage).unwrap();
        assert!(
            !json.contains("purpose"),
            "purpose should be absent, got: {json}"
        );
        assert!(
            !json.contains("question"),
            "question should be absent, got: {json}"
        );
        assert!(
            !json.contains("completionCriteria"),
            "completionCriteria should be absent, got: {json}"
        );
        assert!(
            !json.contains("contributesTo"),
            "contributesTo should be absent, got: {json}"
        );
        assert!(
            !json.contains("aiGuidance"),
            "aiGuidance should be absent, got: {json}"
        );
        assert!(
            !json.contains("outputType"),
            "outputType should be absent, got: {json}"
        );
        assert!(
            !json.contains("null"),
            "no null values should appear, got: {json}"
        );
    }

    #[test]
    fn brief_field_ai_guidance_none_absent_from_json() {
        let field = BriefField {
            field_id: "f1".to_string(),
            name: "Title".to_string(),
            order: 0,
            required: true,
            value_type: "text".to_string(),
            ai_guidance: None,
        };
        let json = serde_json::to_string(&field).unwrap();
        assert!(
            !json.contains("aiGuidance"),
            "aiGuidance should be absent, got: {json}"
        );
    }

    #[test]
    fn blueprint_brief_payload_none_fields_absent_from_json() {
        let payload = BlueprintBriefPayload {
            rendered: "# Brief".to_string(),
            blueprint_id: "bp1".to_string(),
            namespace: "com.example".to_string(),
            name: "MyBlueprint".to_string(),
            version: 1,
            ai_guidance: None,
            required_types: vec![],
            types: vec![],
            structure: vec![],
            protocol: None,
            diagnostics: vec![],
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(
            !json.contains("aiGuidance"),
            "aiGuidance should be absent, got: {json}"
        );
        assert!(
            !json.contains("protocol"),
            "protocol should be absent, got: {json}"
        );
    }
}
