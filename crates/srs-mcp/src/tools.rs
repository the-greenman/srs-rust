//! MCP tool handlers — discovery and the validated write workflows.
//!
//! Input structs here are deliberate *shadows* of the canonical service inputs
//! (ADR-011 forbids schemars on library crates, so the service types cannot
//! derive `JsonSchema`). Drift guard (ADR-037): handlers may only reach the
//! services through the `From` conversions below, and
//! `tool_input_conversion_exercises_every_field` pins every field.
//!
//! Tool description strings are single-source `pub const` items; the
//! `srs-usage.md` MCP section is written from these constants.

use std::sync::Arc;

use rmcp::model::{CallToolResult, ContentBlock, JsonObject, ListToolsResult, Tool};
use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use srs_core::types::note::{Note, NoteSection};
use srs_core::types::record::{FieldMeta, FieldValues};
use srs_core::types::relation::{AssertedBy, Relation, RelationStatus};
use srs_repository::container_service;
use srs_repository::discovery_service::{self, DiscoveryQuery};
use srs_repository::record_store::{
    self, CreateRecordInput, CreateRecordSuccessorInput, FulfillmentNewRecord,
    TransitionFulfillmentInput, TransitionLifecycleInput, UpdateRecordInput,
};
use srs_repository::relation_service;
use srs_repository::services::{self, CreateNoteInput, GraduateNoteInput};
use srs_repository::type_schema_service::{self, TypeSchemaInput};
use srs_repository::validation::validate_repository;

use crate::server::SrsMcpServer;

// ── Tool names ────────────────────────────────────────────────────────────────

pub const TOOL_REPO_VALIDATE: &str = "repo_validate";
pub const TOOL_FIND: &str = "find";
pub const TOOL_RECORD_CREATE: &str = "record_create";
pub const TOOL_RELATION_CREATE: &str = "relation_create";
pub const TOOL_NOTE_CREATE: &str = "note_create";
pub const TOOL_TYPE_SCHEMA: &str = "type_schema";
// Second-wave write tools (#680)
pub const TOOL_RECORD_UPDATE: &str = "record_update";
pub const TOOL_RECORD_TRANSITION: &str = "record_transition";
pub const TOOL_RECORD_ALLOWED_TRANSITIONS: &str = "record_allowed_transitions";
pub const TOOL_RECORD_SUCCESSOR: &str = "record_successor";
pub const TOOL_NOTE_GRADUATE: &str = "note_graduate";
pub const TOOL_CONTAINER_MEMBER_ADD: &str = "container_member_add";
pub const TOOL_CONTAINER_MEMBER_REMOVE: &str = "container_member_remove";

// ── Tool descriptions — single source (srs-usage.md MCP section mirrors these) ─

pub const DESC_REPO_VALIDATE: &str = "Validate the whole repository and return the diagnostics \
array plus a summary. Run this after every write batch. summary.errors == 0 (equivalently, no \
error diagnostics) means the repository is consistent. Warnings are non-blocking, but review \
them. An empty diagnostics array means the repository is completely clean. Diagnostics are \
data, not a tool error: the tool succeeds even when problems are found.";

pub const DESC_FIND: &str = "Deterministic discovery query (ext:discovery). All axes are \
optional and AND-combined: typeId, typeNamespace, typeName, containerId, tag (repeatable; \
instance must carry ALL), lifecycleState, excludeLifecycleStates, tier, and contentMatch \
(recall-floor substring over every searchable text field, not just the title). Returns hits \
with instanceId, label, type, lifecycleState, snippet, and matchedFields. This build serves \
Tier 2 (typed Records); other tier values return zero hits with a diagnostic.";

pub const DESC_RECORD_CREATE: &str = "Create a typed Tier-2 Record. 'type' is \
'namespace/name'; read the type's schema first via the type_schema tool or the \
srs://<repositoryId>/type/{typeId} resource — fieldValues is an OBJECT keyed by \
Field.name verbatim (RFC-039), exactly the schema's property keys; a composite \
(inline-ref) value is itself such an object, or an array of them for a list. \
Validation is enforced: missing required fields or unknown keys are rejected with \
diagnostics and nothing is written. Optional containerId adds the record to \
a container atomically.";

pub const DESC_RELATION_CREATE: &str = "Assert a typed binary relation between two instance \
UUIDs: source [relationType] target, stored in the canonical forward form only (supersedes = \
newer→older, contains = whole→part, depends-on = dependent→needed, precedes = earlier→later). \
The relationType must resolve to an installed RelationTypeDefinition — an unknown type is a \
validation error, not a soft convention. Relations are semantic claims: neither endpoint's \
lifecycle state changes. relationId is assigned when omitted.";

pub const DESC_NOTE_CREATE: &str = "Create a Tier-0 Note (free-text sections, no type \
binding). Each section has a name, content, and optional label. Optional containerId adds the \
note to a container atomically. Notes are the capture tier — graduate one to a typed Record \
later when its structure stabilises.";

pub const DESC_TYPE_SCHEMA: &str = "Get the authoring schema for a type by its UUID \
(typeVersion optional; latest when omitted). The result is a JSON Schema whose properties \
are keyed by Field.name — the same keys record_create fieldValues uses (RFC-039) — \
and carry x-srs-ai-guidance, \
x-srs-description, and x-srs-instructions; required fields are listed in 'required'. Read \
this before creating records of an unfamiliar type — discover typeIds from the type \
resources in resources/list.";

// Second-wave write tool descriptions (#680)
pub const DESC_RECORD_UPDATE: &str = "Replace the fieldValues of an existing Tier-2 Record \
(full replace, not a patch). Provide the complete set of field values you want stored. \
Optional typeVersion migrates the record to a different type version; omit to keep the \
stored version. Tag semantics: omit=preserve, []=clear, [...]=replace. Returns the updated \
Record. Run repo_validate after to confirm consistency.";

pub const DESC_RECORD_TRANSITION: &str = "Transition a record's lifecycle state as defined \
in its Type's lifecycle. Use record_allowed_transitions first to see which transitions are \
available. Supply either 'to' (target state key) or 'byTransition' (named transition, e.g. \
'promote'), not both. RFC-022: when the target state has a requiresRelation obligation, \
supply 'fulfillment.newRecord' (spawn a successor) or 'fulfillment.existingInstanceId' \
(adopt an existing instance). Returns the updated record, any warnings (e.g. final-state \
notice), and the fulfillment artifacts if spawned.";

pub const DESC_RECORD_ALLOWED_TRANSITIONS: &str = "Return the allowed next lifecycle \
transitions from a record's current state. Returns currentState (empty string if never \
transitioned), a list of transitions each with name/to/toIsFinal/requiresRelation, and \
isImmutable. Read this before calling record_transition — an unknown transition is rejected.";

pub const DESC_RECORD_SUCCESSOR: &str = "Create a successor Record and the linking relation \
in one atomic operation. The successor inherits the predecessor's typeId (and optionally a \
pinned typeVersion). relationType must be 'supersedes' or 'refines'. Validation is enforced \
before any write. Returns both the new Record and the linking Relation.";

pub const DESC_NOTE_GRADUATE: &str = "Promote a Tier-0 Note to a typed Tier-2 Record in \
one atomic step. A new Record is created from the supplied type and fieldValues, and a \
derived-from Relation (Record -> Note) is asserted atomically as the graduation's sole \
provenance record. Optional containerId adds the Record to a container. The Note is \
preserved unchanged — it is not deleted, and its graduatedAt field is never stamped. Returns \
both the Note and the new Record.";

pub const DESC_CONTAINER_MEMBER_ADD: &str = "Add an instance to a container's \
memberInstanceIds. This changes membership only; the returned memberInstanceIds array has no \
semantic or presentation-order authority. Use a precedes relation when order is a semantic claim. \
For display or curation order, author a container-subset Composition's ordering.memberOrder via \
the definition-authoring or CLI surface; MCP currently has no definition/view update tool. \
Idempotent — adding an already-present member is not an error. Returns the updated \
memberInstanceIds list.";

pub const DESC_CONTAINER_MEMBER_REMOVE: &str = "Remove an instance from a container's \
memberInstanceIds. This changes membership only; the returned memberInstanceIds array has no \
semantic or presentation-order authority. Use a precedes relation when order is a semantic claim. \
For display or curation order, author a container-subset Composition's ordering.memberOrder via \
the definition-authoring or CLI surface; MCP currently has no definition/view update tool. Returns \
the updated memberInstanceIds list. No-op if the instance is not a member.";

// ── Shadow input structs (see module docs) ────────────────────────────────────

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EmptyToolInput {}

/// Mirrors `discovery_service::DiscoveryQuery` field-for-field.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindToolInput {
    pub type_id: Option<String>,
    pub type_namespace: Option<String>,
    pub type_name: Option<String>,
    pub container_id: Option<String>,
    /// AND-conjunction: the instance's tags must contain ALL specified values.
    #[serde(default)]
    pub tag: Vec<String>,
    pub lifecycle_state: Option<String>,
    /// Inclusive multi-value lifecycleState filter (OR semantics — RFC-012 Rev 11).
    /// Independent of lifecycleState; do not combine the two.
    #[serde(default)]
    pub lifecycle_states: Vec<String>,
    #[serde(default)]
    pub exclude_lifecycle_states: Vec<String>,
    /// Instance tier (0=Note, 2=Record — Tier 1/TypedRecord is retired,
    /// srs#448/rfc-decision-53635966, srs-rust#888).
    pub tier: Option<u8>,
    /// Content substring match (the CLI's --text flag).
    pub content_match: Option<String>,
}

impl From<FindToolInput> for DiscoveryQuery {
    fn from(input: FindToolInput) -> Self {
        DiscoveryQuery {
            type_id: input.type_id,
            type_namespace: input.type_namespace,
            type_name: input.type_name,
            container_id: input.container_id,
            tag: input.tag,
            lifecycle_state: input.lifecycle_state,
            lifecycle_states: input.lifecycle_states,
            exclude_lifecycle_states: input.exclude_lifecycle_states,
            tier: input.tier,
            content_match: input.content_match,
        }
    }
}

/// Per-field provenance, mirroring `srs_core::types::record::FieldMeta`
/// (RFC-039 Change C). Keys of the enclosing `fieldMeta` map MUST be a subset
/// of the sibling `fieldValues` keys ([R6]).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldMetaInput {
    pub source: Option<String>,
    pub edited_at: Option<String>,
    pub source_refs: Option<Vec<Value>>,
}

impl From<FieldMetaInput> for FieldMeta {
    fn from(input: FieldMetaInput) -> Self {
        FieldMeta {
            source: input.source,
            edited_at: input.edited_at,
            source_refs: input.source_refs,
        }
    }
}

fn field_meta_map(
    input: Option<std::collections::BTreeMap<String, FieldMetaInput>>,
) -> Option<indexmap::IndexMap<String, FieldMeta>> {
    input.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())
}

/// Envelope: type binding + container scope around a `CreateRecordInput` mirror.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordCreateToolInput {
    /// Type in "namespace/name" form ("type" on the wire; reserved word in Rust).
    #[serde(rename = "type")]
    pub type_filter: String,
    /// Pin a specific type version (default: latest).
    pub type_version: Option<u32>,
    /// RFC-039 carrier: an object keyed by `Field.name` verbatim. Values
    /// follow the recursive Change-B rule — an inline-composite value is
    /// itself a fieldValues object (or an array of them for a list).
    pub field_values: serde_json::Map<String, Value>,
    /// Per-field provenance keyed identically to `fieldValues` ([R6]).
    pub field_meta: Option<std::collections::BTreeMap<String, FieldMetaInput>>,
    pub tags: Option<Vec<String>>,
    /// Add the new record to this container atomically.
    pub container_id: Option<String>,
}

impl From<RecordCreateToolInput> for CreateRecordInput {
    fn from(input: RecordCreateToolInput) -> Self {
        CreateRecordInput {
            field_values: FieldValues(input.field_values),
            field_meta: field_meta_map(input.field_meta),
            tags: input.tags,
        }
    }
}

/// Provenance agent ("human" | "ai" | "imported") — mirrors `AssertedBy`.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AssertedByInput {
    Human,
    Ai,
    Imported,
}

impl From<AssertedByInput> for AssertedBy {
    fn from(input: AssertedByInput) -> Self {
        match input {
            AssertedByInput::Human => AssertedBy::Human,
            AssertedByInput::Ai => AssertedBy::Ai,
            AssertedByInput::Imported => AssertedBy::Imported,
        }
    }
}

/// Relation status — mirrors `RelationStatus`.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RelationStatusInput {
    Proposed,
    Active,
    Rejected,
    Superseded,
}

impl From<RelationStatusInput> for RelationStatus {
    fn from(input: RelationStatusInput) -> Self {
        match input {
            RelationStatusInput::Proposed => RelationStatus::Proposed,
            RelationStatusInput::Active => RelationStatus::Active,
            RelationStatusInput::Rejected => RelationStatus::Rejected,
            RelationStatusInput::Superseded => RelationStatus::Superseded,
        }
    }
}

/// Mirrors the authoring surface of `srs_core::types::relation::Relation`.
///
/// First-cut narrowing (tracked in srs-rust#680): `sourceRefs` and the
/// federation fields (`sourceRepositoryId`/`targetRepositoryId`) are not
/// exposed — they belong to the sourceRef-authoring / federation waves.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationCreateToolInput {
    /// Assigned automatically when omitted.
    pub relation_id: Option<String>,
    pub relation_type: String,
    pub source_instance_id: String,
    pub target_instance_id: String,
    pub asserted_by: Option<AssertedByInput>,
    pub confidence: Option<f64>,
    pub created_at: Option<String>,
    pub created_by: Option<String>,
    pub status: Option<RelationStatusInput>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub notes: Option<String>,
    pub meta: Option<Value>,
}

impl From<RelationCreateToolInput> for Relation {
    fn from(input: RelationCreateToolInput) -> Self {
        Relation {
            relation_id: input.relation_id.unwrap_or_default(),
            relation_type: input.relation_type,
            source_instance_id: input.source_instance_id,
            target_instance_id: input.target_instance_id,
            asserted_by: input.asserted_by.map(Into::into),
            confidence: input.confidence,
            created_at: input.created_at,
            created_by: input.created_by,
            status: input.status.map(Into::into),
            valid_from: input.valid_from,
            valid_until: input.valid_until,
            notes: input.notes,
            source_refs: None,
            meta: input.meta,
            source_repository_id: None,
            target_repository_id: None,
        }
    }
}

/// Mirrors `srs_core::types::note::NoteSection` (contentHint deferred with #680).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteSectionInput {
    pub name: String,
    pub label: Option<String>,
    pub content: String,
    pub tags: Option<Vec<String>>,
}

impl From<NoteSectionInput> for NoteSection {
    fn from(input: NoteSectionInput) -> Self {
        NoteSection {
            name: input.name,
            label: input.label,
            content: input.content,
            content_hint: None,
            tags: input.tags,
        }
    }
}

/// Mirrors the authoring surface of `services::CreateNoteInput` (a flattened
/// Note plus containerId). The provenance/lifecycle fields (`graduatedAt`,
/// `sourceRefs`, `updatedAt`, `meta`) are service-managed and not exposed.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteCreateToolInput {
    /// Assigned automatically when omitted.
    pub instance_id: Option<String>,
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub sections: Vec<NoteSectionInput>,
    pub created_at: Option<String>,
    /// Add the new note to this container atomically.
    pub container_id: Option<String>,
}

impl From<NoteCreateToolInput> for CreateNoteInput {
    fn from(input: NoteCreateToolInput) -> Self {
        CreateNoteInput {
            note: Note {
                instance_id: input.instance_id.unwrap_or_default(),
                title: input.title,
                tags: input.tags,
                sections: input.sections.into_iter().map(Into::into).collect(),
                graduated_at: None,
                source_refs: None,
                created_at: input.created_at,
                updated_at: None,
                meta: None,
            },
            container_id: input.container_id,
        }
    }
}

/// Mirrors `type_schema_service::TypeSchemaInput` field-for-field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeSchemaToolInput {
    /// The type's UUID (discover via the type resources in resources/list).
    pub type_id: String,
    /// Pin a specific type version (default: latest).
    pub type_version: Option<u32>,
}

impl From<TypeSchemaToolInput> for TypeSchemaInput {
    fn from(input: TypeSchemaToolInput) -> Self {
        TypeSchemaInput {
            type_id: input.type_id,
            type_version: input.type_version,
        }
    }
}

// ── Second-wave shadow input structs (#680) ───────────────────────────────────

/// Mirrors `record_store::UpdateRecordInput` (plus `instanceId` as a separate
/// param). Tag semantics: omit=preserve, []=clear, [...]=replace.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordUpdateToolInput {
    pub instance_id: String,
    /// RFC-039 carrier: an object keyed by `Field.name` verbatim.
    pub field_values: serde_json::Map<String, Value>,
    /// Per-field provenance: omit = preserve stored, {} = clear, {...} = replace.
    #[serde(default)]
    pub field_meta: Option<std::collections::BTreeMap<String, FieldMetaInput>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub type_version: Option<u32>,
}

impl From<RecordUpdateToolInput> for UpdateRecordInput {
    fn from(input: RecordUpdateToolInput) -> Self {
        UpdateRecordInput {
            field_values: FieldValues(input.field_values),
            field_meta: field_meta_map(input.field_meta),
            tags: input.tags,
            type_version: input.type_version,
        }
    }
}

/// Mirrors `record_store::FulfillmentNewRecord`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FulfillmentNewRecordInput {
    /// RFC-039 carrier: an object keyed by `Field.name` verbatim.
    pub field_values: serde_json::Map<String, Value>,
    pub type_version: Option<u32>,
}

impl From<FulfillmentNewRecordInput> for FulfillmentNewRecord {
    fn from(input: FulfillmentNewRecordInput) -> Self {
        FulfillmentNewRecord {
            field_values: FieldValues(input.field_values),
            type_version: input.type_version,
        }
    }
}

/// Mirrors `record_store::TransitionFulfillmentInput`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransitionFulfillmentToolInput {
    pub new_record: Option<FulfillmentNewRecordInput>,
    pub existing_instance_id: Option<String>,
    pub relation_type: Option<String>,
}

impl From<TransitionFulfillmentToolInput> for TransitionFulfillmentInput {
    fn from(input: TransitionFulfillmentToolInput) -> Self {
        TransitionFulfillmentInput {
            new_record: input.new_record.map(Into::into),
            existing_instance_id: input.existing_instance_id,
            relation_type: input.relation_type,
        }
    }
}

/// Mirrors `record_store::TransitionLifecycleInput` (plus `instanceId` as a
/// separate param). Supply either `to` or `byTransition`, not both.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordTransitionToolInput {
    pub instance_id: String,
    pub to: Option<String>,
    pub by_transition: Option<String>,
    pub fulfillment: Option<TransitionFulfillmentToolInput>,
}

impl From<RecordTransitionToolInput> for TransitionLifecycleInput {
    fn from(input: RecordTransitionToolInput) -> Self {
        TransitionLifecycleInput {
            to: input.to,
            by_transition: input.by_transition,
            fulfillment: input.fulfillment.map(Into::into),
        }
    }
}

/// Read-only companion to `record_transition` — no service conversion needed.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordAllowedTransitionsToolInput {
    pub instance_id: String,
}

/// Mirrors `record_store::CreateRecordSuccessorInput` (plus `predecessorId` as
/// a separate param).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordSuccessorToolInput {
    pub predecessor_id: String,
    pub relation_type: String,
    /// RFC-039 carrier: an object keyed by `Field.name` verbatim.
    pub field_values: serde_json::Map<String, Value>,
    pub lifecycle_state: Option<String>,
    pub type_version: Option<u32>,
}

impl From<RecordSuccessorToolInput> for CreateRecordSuccessorInput {
    fn from(input: RecordSuccessorToolInput) -> Self {
        CreateRecordSuccessorInput {
            relation_type: input.relation_type,
            field_values: FieldValues(input.field_values),
            lifecycle_state: input.lifecycle_state,
            type_version: input.type_version,
        }
    }
}

/// Mirrors `services::GraduateNoteInput`. `field_values`, `field_meta`, and
/// `tags` are forwarded into `record_input: CreateRecordInput` on conversion.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteGraduateToolInput {
    pub note_id: String,
    /// Type in "namespace/name" form (same as `record_create`'s `type` field).
    #[serde(rename = "type")]
    pub type_ref: String,
    pub type_version: Option<u32>,
    /// RFC-039 carrier: an object keyed by `Field.name` verbatim.
    pub field_values: serde_json::Map<String, Value>,
    #[serde(default)]
    pub field_meta: Option<std::collections::BTreeMap<String, FieldMetaInput>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub container_id: Option<String>,
}

impl From<NoteGraduateToolInput> for GraduateNoteInput {
    fn from(input: NoteGraduateToolInput) -> Self {
        GraduateNoteInput {
            note_id: input.note_id,
            type_ref: input.type_ref,
            type_version: input.type_version,
            container_id: input.container_id,
            record_input: CreateRecordInput {
                field_values: FieldValues(input.field_values),
                field_meta: field_meta_map(input.field_meta),
                tags: input.tags,
            },
        }
    }
}

/// Shared by `container_member_add` and `container_member_remove` — fields are
/// passed directly to the service; no service struct conversion needed (follows
/// the `EmptyToolInput` / `repo_validate` pattern).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerMemberToolInput {
    pub container_id: String,
    pub instance_id: String,
}

/// Wraps the `Vec<String>` returned by container membership writes so the MCP
/// response is a named object (`{memberInstanceIds: [...]}`) rather than a bare
/// JSON array.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMembersToolResult {
    pub member_instance_ids: Vec<String>,
}

// ── Tool listing ──────────────────────────────────────────────────────────────

fn input_schema<T: JsonSchema>() -> Arc<JsonObject> {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    match serde_json::to_value(schema) {
        Ok(Value::Object(map)) => Arc::new(map),
        _ => Arc::new(JsonObject::default()),
    }
}

pub(crate) fn list_tools() -> ListToolsResult {
    ListToolsResult::with_all_items(vec![
        Tool::new(
            TOOL_REPO_VALIDATE,
            DESC_REPO_VALIDATE,
            input_schema::<EmptyToolInput>(),
        ),
        Tool::new(TOOL_FIND, DESC_FIND, input_schema::<FindToolInput>()),
        Tool::new(
            TOOL_RECORD_CREATE,
            DESC_RECORD_CREATE,
            input_schema::<RecordCreateToolInput>(),
        ),
        Tool::new(
            TOOL_RELATION_CREATE,
            DESC_RELATION_CREATE,
            input_schema::<RelationCreateToolInput>(),
        ),
        Tool::new(
            TOOL_NOTE_CREATE,
            DESC_NOTE_CREATE,
            input_schema::<NoteCreateToolInput>(),
        ),
        Tool::new(
            TOOL_TYPE_SCHEMA,
            DESC_TYPE_SCHEMA,
            input_schema::<TypeSchemaToolInput>(),
        ),
        // Second-wave write tools (#680)
        Tool::new(
            TOOL_RECORD_UPDATE,
            DESC_RECORD_UPDATE,
            input_schema::<RecordUpdateToolInput>(),
        ),
        Tool::new(
            TOOL_RECORD_TRANSITION,
            DESC_RECORD_TRANSITION,
            input_schema::<RecordTransitionToolInput>(),
        ),
        Tool::new(
            TOOL_RECORD_ALLOWED_TRANSITIONS,
            DESC_RECORD_ALLOWED_TRANSITIONS,
            input_schema::<RecordAllowedTransitionsToolInput>(),
        ),
        Tool::new(
            TOOL_RECORD_SUCCESSOR,
            DESC_RECORD_SUCCESSOR,
            input_schema::<RecordSuccessorToolInput>(),
        ),
        Tool::new(
            TOOL_NOTE_GRADUATE,
            DESC_NOTE_GRADUATE,
            input_schema::<NoteGraduateToolInput>(),
        ),
        Tool::new(
            TOOL_CONTAINER_MEMBER_ADD,
            DESC_CONTAINER_MEMBER_ADD,
            input_schema::<ContainerMemberToolInput>(),
        ),
        Tool::new(
            TOOL_CONTAINER_MEMBER_REMOVE,
            DESC_CONTAINER_MEMBER_REMOVE,
            input_schema::<ContainerMemberToolInput>(),
        ),
    ])
}

// ── Tool dispatch ─────────────────────────────────────────────────────────────

fn parse_args<T: for<'de> Deserialize<'de>>(arguments: Option<JsonObject>) -> Result<T, McpError> {
    serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
        .map_err(|e| McpError::invalid_params(e.to_string(), None))
}

/// Success result: the service struct serialized as JSON text + structured content.
fn tool_ok<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let structured =
        serde_json::to_value(value).map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let text = serde_json::to_string_pretty(&structured)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    Ok(result)
}

/// Service rejection → tool-level error the model can read (not a protocol error).
fn tool_err(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

pub(crate) fn call_tool(
    server: &SrsMcpServer,
    name: &str,
    arguments: Option<JsonObject>,
) -> Result<CallToolResult, McpError> {
    let store = server.open_store();
    match name {
        TOOL_REPO_VALIDATE => {
            let _: EmptyToolInput = parse_args(arguments)?;
            match validate_repository(&store) {
                Ok(report) => tool_ok(&report),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_FIND => {
            let input: FindToolInput = parse_args(arguments)?;
            match discovery_service::find(&store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_RECORD_CREATE => {
            let input: RecordCreateToolInput = parse_args(arguments)?;
            let type_filter = input.type_filter.clone();
            let type_version = input.type_version;
            let container_id = input.container_id.clone();
            match record_store::create_record_in_context(
                &store,
                &type_filter,
                type_version,
                input.into(),
                container_id,
                None,
            ) {
                Ok(result) => tool_ok(&result.record),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_RELATION_CREATE => {
            let input: RelationCreateToolInput = parse_args(arguments)?;
            match relation_service::create_relation_auto(&store, input.into()) {
                Ok(result) => tool_ok(&result.relation),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_NOTE_CREATE => {
            let input: NoteCreateToolInput = parse_args(arguments)?;
            match services::create_note_in_context(&store, input.into()) {
                Ok(result) => tool_ok(&result.note),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_TYPE_SCHEMA => {
            let input: TypeSchemaToolInput = parse_args(arguments)?;
            match type_schema_service::type_schema(&store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        // Second-wave write tools (#680)
        TOOL_RECORD_UPDATE => {
            let input: RecordUpdateToolInput = parse_args(arguments)?;
            let instance_id = input.instance_id.clone();
            match record_store::update_record(&store, &instance_id, input.into()) {
                Ok(record) => tool_ok(&record),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_RECORD_TRANSITION => {
            let input: RecordTransitionToolInput = parse_args(arguments)?;
            let instance_id = input.instance_id.clone();
            match record_store::transition_record_lifecycle(&store, &instance_id, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_RECORD_ALLOWED_TRANSITIONS => {
            let input: RecordAllowedTransitionsToolInput = parse_args(arguments)?;
            match record_store::get_allowed_lifecycle_transitions(&store, &input.instance_id) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_RECORD_SUCCESSOR => {
            let input: RecordSuccessorToolInput = parse_args(arguments)?;
            let predecessor_id = input.predecessor_id.clone();
            match record_store::create_record_successor(&store, &predecessor_id, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_NOTE_GRADUATE => {
            let input: NoteGraduateToolInput = parse_args(arguments)?;
            match services::graduate_note(&store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_CONTAINER_MEMBER_ADD => {
            let input: ContainerMemberToolInput = parse_args(arguments)?;
            match container_service::add_container_member(
                &store,
                &input.container_id,
                &input.instance_id,
            ) {
                Ok(members) => tool_ok(&ContainerMembersToolResult {
                    member_instance_ids: members,
                }),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        TOOL_CONTAINER_MEMBER_REMOVE => {
            let input: ContainerMemberToolInput = parse_args(arguments)?;
            match container_service::remove_container_member(
                &store,
                &input.container_id,
                &input.instance_id,
            ) {
                Ok(members) => tool_ok(&ContainerMembersToolResult {
                    member_instance_ids: members,
                }),
                Err(e) => Ok(tool_err(e.to_string())),
            }
        }
        other => Err(McpError::invalid_params(
            format!("unknown tool '{other}'"),
            None,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: populate EVERY field of each shadow input and assert the
    /// conversion carries all of them into the service type (plan review AR-1).
    #[test]
    fn tool_input_conversion_exercises_every_field() {
        // Find → DiscoveryQuery
        let find = FindToolInput {
            type_id: Some("tid".into()),
            type_namespace: Some("ns".into()),
            type_name: Some("nm".into()),
            container_id: Some("cid".into()),
            tag: vec!["a".into(), "b".into()],
            lifecycle_state: Some("active".into()),
            lifecycle_states: vec!["active".into(), "draft".into()],
            exclude_lifecycle_states: vec!["superseded".into()],
            tier: Some(2),
            content_match: Some("text".into()),
        };
        let q: DiscoveryQuery = find.into();
        assert_eq!(q.type_id.as_deref(), Some("tid"));
        assert_eq!(q.type_namespace.as_deref(), Some("ns"));
        assert_eq!(q.type_name.as_deref(), Some("nm"));
        assert_eq!(q.container_id.as_deref(), Some("cid"));
        assert_eq!(q.tag, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(q.lifecycle_state.as_deref(), Some("active"));
        assert_eq!(
            q.lifecycle_states,
            vec!["active".to_string(), "draft".to_string()]
        );
        assert_eq!(q.exclude_lifecycle_states, vec!["superseded".to_string()]);
        assert_eq!(q.tier, Some(2));
        assert_eq!(q.content_match.as_deref(), Some("text"));

        // RecordCreate → CreateRecordInput (name-keyed carrier + fieldMeta)
        let rec = RecordCreateToolInput {
            type_filter: "ns/nm".into(),
            type_version: Some(3),
            field_values: [
                ("title".to_string(), serde_json::json!("v1")),
                (
                    "rows".to_string(),
                    serde_json::json!([{"cells": ["a", "b"]}]),
                ),
            ]
            .into_iter()
            .collect(),
            field_meta: Some(
                [(
                    "title".to_string(),
                    FieldMetaInput {
                        source: Some("fsrc".into()),
                        edited_at: Some("t2".into()),
                        source_refs: Some(vec![serde_json::json!({"kind": "url"})]),
                    },
                )]
                .into_iter()
                .collect(),
            ),
            tags: Some(vec!["t".into()]),
            container_id: Some("c".into()),
        };
        assert_eq!(rec.type_filter, "ns/nm");
        assert_eq!(rec.type_version, Some(3));
        assert_eq!(rec.container_id.as_deref(), Some("c"));
        let ci: CreateRecordInput = rec.into();
        assert_eq!(ci.field_values.len(), 2);
        assert_eq!(ci.field_values.get("title"), Some(&serde_json::json!("v1")));
        assert_eq!(
            ci.field_values.get("rows"),
            Some(&serde_json::json!([{"cells": ["a", "b"]}]))
        );
        let meta = &ci.field_meta.as_ref().unwrap()["title"];
        assert_eq!(meta.source.as_deref(), Some("fsrc"));
        assert_eq!(meta.edited_at.as_deref(), Some("t2"));
        assert_eq!(
            meta.source_refs,
            Some(vec![serde_json::json!({"kind": "url"})])
        );
        assert_eq!(ci.tags, Some(vec!["t".to_string()]));

        // RelationCreate → Relation
        let rel = RelationCreateToolInput {
            relation_id: Some("rid".into()),
            relation_type: "depends-on".into(),
            source_instance_id: "s".into(),
            target_instance_id: "t".into(),
            asserted_by: Some(AssertedByInput::Ai),
            confidence: Some(0.9),
            created_at: Some("now".into()),
            created_by: Some("me".into()),
            status: Some(RelationStatusInput::Active),
            valid_from: Some("vf".into()),
            valid_until: Some("vu".into()),
            notes: Some("n".into()),
            meta: Some(serde_json::Value::Bool(true)),
        };
        let r: Relation = rel.into();
        assert_eq!(r.relation_id, "rid");
        assert_eq!(r.relation_type, "depends-on");
        assert_eq!(r.source_instance_id, "s");
        assert_eq!(r.target_instance_id, "t");
        assert_eq!(r.asserted_by, Some(AssertedBy::Ai));
        assert_eq!(r.confidence, Some(0.9));
        assert_eq!(r.created_at.as_deref(), Some("now"));
        assert_eq!(r.created_by.as_deref(), Some("me"));
        assert_eq!(r.status, Some(RelationStatus::Active));
        assert_eq!(r.valid_from.as_deref(), Some("vf"));
        assert_eq!(r.valid_until.as_deref(), Some("vu"));
        assert_eq!(r.notes.as_deref(), Some("n"));
        assert_eq!(r.meta, Some(serde_json::Value::Bool(true)));

        // NoteCreate → CreateNoteInput
        let note = NoteCreateToolInput {
            instance_id: Some("iid".into()),
            title: Some("T".into()),
            tags: Some(vec!["x".into()]),
            sections: vec![NoteSectionInput {
                name: "body".into(),
                label: Some("Body".into()),
                content: "hello".into(),
                tags: Some(vec!["s".into()]),
            }],
            created_at: Some("now".into()),
            container_id: Some("cid".into()),
        };
        let ni: CreateNoteInput = note.into();
        assert_eq!(ni.note.instance_id, "iid");
        assert_eq!(ni.note.title.as_deref(), Some("T"));
        assert_eq!(ni.note.tags, Some(vec!["x".to_string()]));
        assert_eq!(ni.note.sections[0].name, "body");
        assert_eq!(ni.note.sections[0].label.as_deref(), Some("Body"));
        assert_eq!(ni.note.sections[0].content, "hello");
        assert_eq!(ni.note.sections[0].tags, Some(vec!["s".to_string()]));
        assert_eq!(ni.note.created_at.as_deref(), Some("now"));
        assert_eq!(ni.container_id.as_deref(), Some("cid"));

        // TypeSchema → TypeSchemaInput
        let ts = TypeSchemaToolInput {
            type_id: "tid".into(),
            type_version: Some(4),
        };
        let tsi: TypeSchemaInput = ts.into();
        assert_eq!(tsi.type_id, "tid");
        assert_eq!(tsi.type_version, Some(4));
    }

    #[test]
    fn list_tools_advertises_all_thirteen_with_schemas() {
        let tools = list_tools().tools;
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(
            names,
            vec![
                TOOL_REPO_VALIDATE,
                TOOL_FIND,
                TOOL_RECORD_CREATE,
                TOOL_RELATION_CREATE,
                TOOL_NOTE_CREATE,
                TOOL_TYPE_SCHEMA,
                TOOL_RECORD_UPDATE,
                TOOL_RECORD_TRANSITION,
                TOOL_RECORD_ALLOWED_TRANSITIONS,
                TOOL_RECORD_SUCCESSOR,
                TOOL_NOTE_GRADUATE,
                TOOL_CONTAINER_MEMBER_ADD,
                TOOL_CONTAINER_MEMBER_REMOVE,
            ]
        );
        for tool in &tools {
            assert!(tool.description.is_some());
            assert!(
                !tool.input_schema.is_empty(),
                "tool {} has an empty input schema",
                tool.name
            );
        }
    }

    #[test]
    fn membership_tool_descriptions_distinguish_membership_semantics_and_presentation_order() {
        let tools = list_tools().tools;

        for name in [TOOL_CONTAINER_MEMBER_ADD, TOOL_CONTAINER_MEMBER_REMOVE] {
            let description = tools
                .iter()
                .find(|tool| tool.name.as_ref() == name)
                .and_then(|tool| tool.description.as_deref())
                .expect("membership tool must advertise a description");

            assert!(description.contains("changes membership only"));
            assert!(description.contains("no semantic or presentation-order authority"));
            assert!(description.contains("precedes relation when order is a semantic claim"));
            assert!(description.contains("container-subset Composition's ordering.memberOrder"));
            assert!(description.contains("MCP currently has no definition/view update tool"));
        }
    }

    #[test]
    fn tool_input_conversion_second_wave_exercises_every_field() {
        // RecordUpdateToolInput → UpdateRecordInput
        let upd = RecordUpdateToolInput {
            instance_id: "iid".into(),
            field_values: [("title".to_string(), serde_json::json!("v1"))]
                .into_iter()
                .collect(),
            field_meta: Some(
                [(
                    "title".to_string(),
                    FieldMetaInput {
                        source: Some("src".into()),
                        edited_at: Some("t".into()),
                        source_refs: None,
                    },
                )]
                .into_iter()
                .collect(),
            ),
            tags: Some(vec!["tag1".into()]),
            type_version: Some(2),
        };
        assert_eq!(upd.instance_id, "iid");
        let ui: UpdateRecordInput = upd.into();
        assert_eq!(ui.field_values.get("title"), Some(&serde_json::json!("v1")));
        let meta = &ui.field_meta.as_ref().unwrap()["title"];
        assert_eq!(meta.source.as_deref(), Some("src"));
        assert_eq!(meta.edited_at.as_deref(), Some("t"));
        assert_eq!(ui.tags, Some(vec!["tag1".to_string()]));
        assert_eq!(ui.type_version, Some(2));

        // FulfillmentNewRecordInput → FulfillmentNewRecord
        let fnr = FulfillmentNewRecordInput {
            field_values: [("fx".to_string(), serde_json::Value::Null)]
                .into_iter()
                .collect(),
            type_version: Some(1),
        };
        let fr: FulfillmentNewRecord = fnr.into();
        assert!(fr.field_values.contains_key("fx"));
        assert_eq!(fr.type_version, Some(1));

        // TransitionFulfillmentToolInput → TransitionFulfillmentInput
        let tfi = TransitionFulfillmentToolInput {
            new_record: Some(FulfillmentNewRecordInput {
                field_values: Default::default(),
                type_version: None,
            }),
            existing_instance_id: Some("eid".into()),
            relation_type: Some("supersedes".into()),
        };
        let tf: TransitionFulfillmentInput = tfi.into();
        assert!(tf.new_record.is_some());
        assert_eq!(tf.existing_instance_id.as_deref(), Some("eid"));
        assert_eq!(tf.relation_type.as_deref(), Some("supersedes"));

        // RecordTransitionToolInput → TransitionLifecycleInput (instance_id extracted)
        let trans = RecordTransitionToolInput {
            instance_id: "rid".into(),
            to: Some("active".into()),
            by_transition: None,
            fulfillment: Some(TransitionFulfillmentToolInput {
                new_record: None,
                existing_instance_id: None,
                relation_type: None,
            }),
        };
        assert_eq!(trans.instance_id, "rid");
        let ti: TransitionLifecycleInput = trans.into();
        assert_eq!(ti.to.as_deref(), Some("active"));
        assert!(ti.by_transition.is_none());
        assert!(ti.fulfillment.is_some());

        // RecordAllowedTransitionsToolInput (no conversion — field passed directly)
        let rat = RecordAllowedTransitionsToolInput {
            instance_id: "x".into(),
        };
        assert_eq!(rat.instance_id, "x");

        // RecordSuccessorToolInput → CreateRecordSuccessorInput (predecessor_id extracted)
        let succ = RecordSuccessorToolInput {
            predecessor_id: "pid".into(),
            relation_type: "supersedes".into(),
            field_values: [("f3".to_string(), serde_json::json!("v3"))]
                .into_iter()
                .collect(),
            lifecycle_state: Some("draft".into()),
            type_version: Some(5),
        };
        assert_eq!(succ.predecessor_id, "pid");
        let si: CreateRecordSuccessorInput = succ.into();
        assert_eq!(si.relation_type, "supersedes");
        assert_eq!(si.field_values.get("f3"), Some(&serde_json::json!("v3")));
        assert_eq!(si.lifecycle_state.as_deref(), Some("draft"));
        assert_eq!(si.type_version, Some(5));

        // NoteGraduateToolInput → GraduateNoteInput
        // Key: field_values/field_meta/tags land in result.record_input, not top-level.
        let grad = NoteGraduateToolInput {
            note_id: "nid".into(),
            type_ref: "ns/nm".into(),
            type_version: Some(3),
            field_values: [("fg1".to_string(), serde_json::json!("grad"))]
                .into_iter()
                .collect(),
            field_meta: Some(
                [(
                    "fg1".to_string(),
                    FieldMetaInput {
                        source: Some("gsrc".into()),
                        edited_at: None,
                        source_refs: None,
                    },
                )]
                .into_iter()
                .collect(),
            ),
            tags: Some(vec!["gtag".into()]),
            container_id: Some("cid".into()),
        };
        let gi: GraduateNoteInput = grad.into();
        assert_eq!(gi.note_id, "nid");
        assert_eq!(gi.type_ref, "ns/nm");
        assert_eq!(gi.type_version, Some(3));
        assert_eq!(gi.container_id.as_deref(), Some("cid"));
        // The three forwarded fields must be inside record_input, NOT on GraduateNoteInput:
        assert_eq!(
            gi.record_input.field_values.get("fg1"),
            Some(&serde_json::json!("grad"))
        );
        assert_eq!(
            gi.record_input.field_meta.as_ref().unwrap()["fg1"]
                .source
                .as_deref(),
            Some("gsrc")
        );
        assert_eq!(gi.record_input.tags, Some(vec!["gtag".to_string()]));

        // ContainerMemberToolInput (no conversion — fields passed directly)
        let cm = ContainerMemberToolInput {
            container_id: "c1".into(),
            instance_id: "i1".into(),
        };
        assert_eq!(cm.container_id, "c1");
        assert_eq!(cm.instance_id, "i1");
    }
}
