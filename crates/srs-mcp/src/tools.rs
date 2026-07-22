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
use srs_core::types::record::{FieldGroupEntry, FieldGroupValue, FieldValue, FieldValueEntry};
use srs_core::types::relation::{AssertedBy, Relation, RelationStatus};
use srs_repository::discovery_service::{self, DiscoveryQuery};
use srs_repository::record_store::{self, CreateRecordInput};
use srs_repository::relation_service;
use srs_repository::services::{self, CreateNoteInput};
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

// ── Tool descriptions — single source (srs-usage.md MCP section mirrors these) ─

pub const DESC_REPO_VALIDATE: &str = "Validate the whole repository and return the diagnostics \
array plus a summary. Run this after every write batch — an empty diagnostics array means the \
repository is consistent. Diagnostics are data, not an error: the tool succeeds even when \
problems are found.";

pub const DESC_FIND: &str = "Deterministic discovery query (ext:discovery). All axes are \
optional and AND-combined: typeId, typeNamespace, typeName, containerId, tag (repeatable; \
instance must carry ALL), lifecycleState, excludeLifecycleStates, tier, and contentMatch \
(recall-floor substring over every searchable text field, not just the title). Returns hits \
with instanceId, label, type, lifecycleState, snippet, and matchedFields. This build serves \
Tier 2 (typed Records); other tier values return zero hits with a diagnostic.";

pub const DESC_RECORD_CREATE: &str = "Create a typed Tier-2 Record. 'type' is \
'namespace/name'; resolve the type's fieldAssignments first via the type_schema tool or the \
srs://<repositoryId>/type/{typeId} resource (each property's x-srs-field-id is the UUID to \
use here) — fieldValues entries are keyed by fieldId UUID, never by field name. Validation is enforced: missing required fields or unknown fields \
are rejected with diagnostics and nothing is written. Optional containerId adds the record to \
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
carry x-srs-field-id (the UUID to use in record_create fieldValues), x-srs-ai-guidance, \
x-srs-description, and x-srs-instructions; required fields are listed in 'required'. Read \
this before creating records of an unfamiliar type — discover typeIds from the type \
resources in resources/list.";

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
    #[serde(default)]
    pub exclude_lifecycle_states: Vec<String>,
    /// Instance tier (0=Note, 1=TypedRecord, 2=Record).
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
            exclude_lifecycle_states: input.exclude_lifecycle_states,
            tier: input.tier,
            content_match: input.content_match,
        }
    }
}

/// Mirrors `srs_core::types::record::FieldValueEntry`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldValueEntryInput {
    pub value: Value,
    pub source: Option<String>,
    pub edited_at: Option<String>,
}

impl From<FieldValueEntryInput> for FieldValueEntry {
    fn from(input: FieldValueEntryInput) -> Self {
        FieldValueEntry {
            value: input.value,
            source: input.source,
            edited_at: input.edited_at,
        }
    }
}

/// Mirrors `srs_core::types::record::FieldValue`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldValueInput {
    /// The field's UUID from the type's fieldAssignments — never a field name.
    pub field_id: String,
    #[serde(default)]
    pub value: Value,
    /// Repeatable-field entries (ext:repeatable-fields).
    pub entries: Option<Vec<FieldValueEntryInput>>,
    pub source: Option<String>,
    pub edited_at: Option<String>,
}

impl From<FieldValueInput> for FieldValue {
    fn from(input: FieldValueInput) -> Self {
        FieldValue {
            field_id: input.field_id,
            value: input.value,
            entries: input
                .entries
                .map(|es| es.into_iter().map(Into::into).collect()),
            source: input.source,
            edited_at: input.edited_at,
        }
    }
}

/// Mirrors `srs_core::types::record::FieldGroupEntry`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldGroupEntryInput {
    pub field_values: Vec<FieldValueInput>,
    pub entry_id: Option<String>,
}

impl From<FieldGroupEntryInput> for FieldGroupEntry {
    fn from(input: FieldGroupEntryInput) -> Self {
        FieldGroupEntry {
            field_values: input.field_values.into_iter().map(Into::into).collect(),
            entry_id: input.entry_id,
        }
    }
}

/// Mirrors `srs_core::types::record::FieldGroupValue`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldGroupValueInput {
    pub group_id: String,
    pub entries: Vec<FieldGroupEntryInput>,
}

impl From<FieldGroupValueInput> for FieldGroupValue {
    fn from(input: FieldGroupValueInput) -> Self {
        FieldGroupValue {
            group_id: input.group_id,
            entries: input.entries.into_iter().map(Into::into).collect(),
        }
    }
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
    pub field_values: Vec<FieldValueInput>,
    pub group_values: Option<Vec<FieldGroupValueInput>>,
    pub tags: Option<Vec<String>>,
    /// Add the new record to this container atomically.
    pub container_id: Option<String>,
}

impl From<RecordCreateToolInput> for CreateRecordInput {
    fn from(input: RecordCreateToolInput) -> Self {
        CreateRecordInput {
            field_values: input.field_values.into_iter().map(Into::into).collect(),
            group_values: input
                .group_values
                .map(|gs| gs.into_iter().map(Into::into).collect()),
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
        assert_eq!(q.exclude_lifecycle_states, vec!["superseded".to_string()]);
        assert_eq!(q.tier, Some(2));
        assert_eq!(q.content_match.as_deref(), Some("text"));

        // RecordCreate → CreateRecordInput (nested field/group values included)
        let rec = RecordCreateToolInput {
            type_filter: "ns/nm".into(),
            type_version: Some(3),
            field_values: vec![FieldValueInput {
                field_id: "f1".into(),
                value: serde_json::Value::String("v1".into()),
                entries: Some(vec![FieldValueEntryInput {
                    value: serde_json::Value::String("e1".into()),
                    source: Some("src".into()),
                    edited_at: Some("t1".into()),
                }]),
                source: Some("fsrc".into()),
                edited_at: Some("t2".into()),
            }],
            group_values: Some(vec![FieldGroupValueInput {
                group_id: "g1".into(),
                entries: vec![FieldGroupEntryInput {
                    field_values: vec![FieldValueInput {
                        field_id: "f2".into(),
                        value: serde_json::Value::Bool(true),
                        entries: None,
                        source: None,
                        edited_at: None,
                    }],
                    entry_id: Some("ge1".into()),
                }],
            }]),
            tags: Some(vec!["t".into()]),
            container_id: Some("c".into()),
        };
        assert_eq!(rec.type_filter, "ns/nm");
        assert_eq!(rec.type_version, Some(3));
        assert_eq!(rec.container_id.as_deref(), Some("c"));
        let ci: CreateRecordInput = rec.into();
        assert_eq!(ci.field_values.len(), 1);
        let fv = &ci.field_values[0];
        assert_eq!(fv.field_id, "f1");
        assert_eq!(fv.value, serde_json::Value::String("v1".into()));
        let entry = &fv.entries.as_ref().unwrap()[0];
        assert_eq!(entry.value, serde_json::Value::String("e1".into()));
        assert_eq!(entry.source.as_deref(), Some("src"));
        assert_eq!(entry.edited_at.as_deref(), Some("t1"));
        assert_eq!(fv.source.as_deref(), Some("fsrc"));
        assert_eq!(fv.edited_at.as_deref(), Some("t2"));
        let group = &ci.group_values.as_ref().unwrap()[0];
        assert_eq!(group.group_id, "g1");
        assert_eq!(group.entries[0].entry_id.as_deref(), Some("ge1"));
        assert_eq!(group.entries[0].field_values[0].field_id, "f2");
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
    fn list_tools_advertises_all_six_with_schemas() {
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
                TOOL_TYPE_SCHEMA
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
}
