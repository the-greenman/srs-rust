//! MCP tool catalogue and handlers — discovery and the validated write workflows.
//!
//! This module is the single owner of tool names, descriptions, input schemas
//! and dispatch; native rmcp and browser adapters only translate transport.
//! Results are JSON-native MCP values (no `rmcp` models).
//!
//! Input structs here are deliberate *shadows* of the canonical service inputs
//! (ADR-011 forbids schemars on library crates, so the service types cannot
//! derive `JsonSchema`). Drift guard (ADR-037): handlers may only reach the
//! services through the `From` conversions below, and
//! `tool_input_conversion_exercises_every_field` pins every field.
//!
//! Tool description strings are single-source `pub const` items; the
//! `srs-usage.md` MCP section is written from these constants.

use crate::McpApplicationError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use srs_core::arrangement::RelativeMove;
use srs_core::types::note::{Note, NoteSection};
use srs_core::types::record::{FieldMeta, FieldValues};
use srs_core::types::relation::Relation;
use srs_repository::attachment_service::{self, AddAttachmentInput, LinkAttachmentInput};
use srs_repository::container_service::{self, ContainerCreateInput};
use srs_repository::context_query_service::{
    list_neighbours, EdgeDirection, NeighboursPage, NeighboursQuery,
};
use srs_repository::discovery_service::{self, DiscoveryQuery, FindPage, MatchMode};
use srs_repository::package_dependency_service::{
    self, AddPackageDependencyInput, RemovePackageDependencyInput,
};
use srs_repository::package_install_service::{self, UpgradeOptions};
use srs_repository::projection::Projection;
use srs_repository::protocol_run_service::{
    self, AdvanceStageInput, CreateRunInput, GetRunResult, RunListFilter, RunSummary,
};
use srs_repository::record_store::{
    self, CreateRecordInput, CreateRecordSuccessorInput, FulfillmentNewRecord,
    TransitionFulfillmentInput, TransitionLifecycleInput, UpdateRecordInput,
};
use srs_repository::relation_service;
use srs_repository::services::{self, CreateNoteInput, GraduateNoteInput, UpdateNoteContentInput};
use srs_repository::store::RepositoryStore;
use srs_repository::type_schema_service::{self, TypeSchemaInput};
use srs_repository::validation::validate_repository;

// ── Tool names ────────────────────────────────────────────────────────────────

pub const TOOL_REPO_VALIDATE: &str = "repo_validate";
pub const TOOL_FIND: &str = "find";
/// Agent-facing replies are size-capped; omitted `limit` on the MCP `find` tool.
const FIND_DEFAULT_LIMIT: usize = 25;
pub const TOOL_RECORD_CREATE: &str = "record_create";
pub const TOOL_RELATION_CREATE: &str = "relation_create";
pub const TOOL_RELATION_DELETE: &str = "relation_delete";
pub const TOOL_NOTE_CREATE: &str = "note_create";
pub const TOOL_NOTE_UPDATE: &str = "note_update";
pub const TOOL_TYPE_SCHEMA: &str = "type_schema";
// Issue #1220: resources for clients that only call tools (claude.ai relay)
pub const TOOL_READ: &str = "read";
// Second-wave write tools (#680)
pub const TOOL_RECORD_UPDATE: &str = "record_update";
pub const TOOL_RECORD_TRANSITION: &str = "record_transition";
pub const TOOL_RECORD_ALLOWED_TRANSITIONS: &str = "record_allowed_transitions";
pub const TOOL_RECORD_SUCCESSOR: &str = "record_successor";
pub const TOOL_NOTE_GRADUATE: &str = "note_graduate";
pub const TOOL_CONTAINER_MEMBER_ADD: &str = "container_member_add";
pub const TOOL_CONTAINER_MEMBER_REMOVE: &str = "container_member_remove";
// Container creation (#1133) — one core service, `container_service::create_container`
pub const TOOL_CONTAINER_CREATE: &str = "container_create";
// RFC-043 Change D: arrangement operations
pub const TOOL_CONTAINER_MEMBER_MOVE: &str = "container_member_move";
pub const TOOL_CONTAINER_MEMBER_REPAIR: &str = "container_member_repair";
// Issue #1156: the structured outline read
pub const TOOL_CONTAINER_OUTLINE: &str = "container_outline";
pub const TOOL_CONTAINER_COPY: &str = "container_copy";
pub const TOOL_RECORD_FORK: &str = "record_fork";
// Protocol run execution tools (#977 — follow-up to #955)
pub const TOOL_PROTOCOL_RUN_CREATE: &str = "protocol_run_create";
pub const TOOL_PROTOCOL_RUN_ADVANCE: &str = "protocol_run_advance";
pub const TOOL_PROTOCOL_RUN_GET: &str = "protocol_run_get";
pub const TOOL_PROTOCOL_RUN_LIST: &str = "protocol_run_list";
pub const TOOL_PROTOCOL_RUN_COMPLETE: &str = "protocol_run_complete";
pub const TOOL_PROTOCOL_RUN_ABANDON: &str = "protocol_run_abandon";
// RFC-044 package requirements (srs-rust#1168) — one core service,
// `package_dependency_service`
pub const TOOL_PACKAGE_DEPENDENCY_LIST: &str = "package_dependency_list";
pub const TOOL_PACKAGE_DEPENDENCY_SET: &str = "package_dependency_set";
pub const TOOL_PACKAGE_DEPENDENCY_REMOVE: &str = "package_dependency_remove";
// Package upgrade (srs-rust#1152) — one core service, `package_install_service`
pub const TOOL_PACKAGE_UPGRADE: &str = "package_upgrade";
pub const TOOL_NEIGHBOURS: &str = "neighbours";
pub const TOOL_SIMILAR: &str = "similar";
pub const TOOL_ATTACHMENT_ADD: &str = "attachment_add";
pub const TOOL_ATTACHMENT_LINK: &str = "attachment_link";
/// Agent-facing replies are size-capped: omitted `limit` on `neighbours`, and its ceiling.
const NEIGHBOURS_DEFAULT_LIMIT: usize = 25;
const NEIGHBOURS_MAX_LIMIT: usize = 100;

// ── Tool profiles (srs-rust#1287) ─────────────────────────────────────────────

/// Which tools a session advertises and accepts. `tools/list` is paid for in context on every
/// session, so a client that needs a few tools should not carry all of them. `full` is the
/// default and the whole catalogue; the others are fixed subsets defined here, once, for every
/// transport. A tool outside the profile is refused exactly like an unknown tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolProfile {
    #[default]
    Full,
    /// Agent memory: discover, read, validate, and the writes that capture knowledge (create,
    /// update, transition, supersede, note, relate, file into a container).
    Context,
    /// Read-only: discovery, reads, outlines, validation; no tool that writes.
    Read,
}

const CONTEXT_TOOLS: &[&str] = &[
    TOOL_REPO_VALIDATE,
    TOOL_FIND,
    TOOL_READ,
    TOOL_TYPE_SCHEMA,
    TOOL_RECORD_CREATE,
    TOOL_RECORD_UPDATE,
    TOOL_RECORD_ALLOWED_TRANSITIONS,
    TOOL_RECORD_TRANSITION,
    TOOL_RECORD_SUCCESSOR,
    TOOL_NOTE_CREATE,
    TOOL_NOTE_UPDATE,
    TOOL_RELATION_CREATE,
    TOOL_CONTAINER_MEMBER_ADD,
];

const READ_TOOLS: &[&str] = &[
    TOOL_FIND,
    TOOL_READ,
    TOOL_SIMILAR,
    TOOL_NEIGHBOURS,
    TOOL_TYPE_SCHEMA,
    TOOL_CONTAINER_OUTLINE,
    TOOL_RECORD_ALLOWED_TRANSITIONS,
    TOOL_PROTOCOL_RUN_GET,
    TOOL_PROTOCOL_RUN_LIST,
    TOOL_PACKAGE_DEPENDENCY_LIST,
    TOOL_REPO_VALIDATE,
];

impl ToolProfile {
    /// Whether this profile advertises and accepts the tool `name`.
    pub fn allows(self, name: &str) -> bool {
        match self {
            ToolProfile::Full => true,
            _ => self.tool_names().is_some_and(|names| names.contains(&name)),
        }
    }

    /// The tool names this profile allows, in catalogue order; `None` for `full` (every tool).
    pub fn tool_names(self) -> Option<&'static [&'static str]> {
        match self {
            ToolProfile::Full => None,
            ToolProfile::Context => Some(CONTEXT_TOOLS),
            ToolProfile::Read => Some(READ_TOOLS),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ToolProfile::Full => "full",
            ToolProfile::Context => "context",
            ToolProfile::Read => "read",
        }
    }
}

impl std::str::FromStr for ToolProfile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "full" => Ok(ToolProfile::Full),
            "context" => Ok(ToolProfile::Context),
            "read" => Ok(ToolProfile::Read),
            other => Err(format!(
                "invalid tool profile '{other}' (expected full|context|read)"
            )),
        }
    }
}

// ── Tool descriptions — single source (srs-usage.md MCP section mirrors these) ─

pub const DESC_REPO_VALIDATE: &str = "Validate the whole repository and return the diagnostics \
array plus a summary. Run this after every write batch. summary.errors == 0 (equivalently, no \
error diagnostics) means the repository is consistent. Warnings are non-blocking, but review \
them; info diagnostics are informational and counted in neither total. An empty diagnostics array means the repository is completely clean. Diagnostics are \
data, not a tool error: the tool succeeds even when problems are found.";

pub const DESC_PACKAGE_DEPENDENCY_LIST: &str = "List a package's packageDependencies \
(RFC-044 package requirements) with each entry's check outcome: satisfied, or the reason \
(no-package-id, self-requirement, missing, version-unknown, incompatible, prerelease-excluded, \
version-too-low), the installed candidate versions, and stale labels. selector is the package \
boundary path (omit for the primary package).";

pub const DESC_PACKAGE_DEPENDENCY_SET: &str = "Require another installed package, keyed by its \
packageId (UUID), at a SemVer 2.0.0 version (satisfied by the same compatibility band at an \
equal or higher version; below 1.0 the MINOR acts as the major). The namespace/name labels are \
filled from the installed package, never guessed: an id that resolves to no installed package \
is refused. An existing entry with that packageId is replaced. A legacy entry without packageId \
whose labels equal the installed package's labels is replaced only when repairLegacy is true \
(without it the call is refused, since a missing packageId is never supplied by matching labels); \
other entries are kept verbatim. selector is the requiring package boundary path (omit for the primary package).";

pub const DESC_PACKAGE_DEPENDENCY_REMOVE: &str = "Remove a package's requirement on a packageId \
(every packageDependencies entry with that id). Refused when there is none. selector is the \
requiring package boundary path (omit for the primary package).";

pub const DESC_PACKAGE_UPGRADE: &str = "Upgrade an installed package boundary in place from a newer \
.srspkg Package Bundle (RFC-014 R2/R3/R6). bundle is the file's JSON text. The package must already \
be installed with the bundle's packageId (otherwise use install, which has no MCP tool); a lower \
bundle version is refused, an equal one is a content sync, a higher one also bumps the boundary's \
version. ALWAYS call with dryRun true first and read the plan: added (new UUIDs), newVersions (new \
versions of an installed UUID, installed alongside the old ones), updated (same uuid+version, upstream \
changed, local copy clean: overwritten; provenBy = the priorBundles version that proved an \
installed definition with no reference copy unedited), adopted (unproven no-reference-copy definitions \
overwritten because you listed their ids in adopt: ANY LOCAL CHANGE TO THEM IS LOST; list an id only \
after the user consents per definition), unchanged, repaired (content current but reference copy / \
import record rewritten), conflicts (NOT written: local-edit = you edited a definition that upstream \
also changed, the local file is kept; no-reference-copy; key-collision = same name, different UUID), \
removedUpstream (installed from this package but absent from the bundle: reported, never deleted) and \
dependencyWarnings (unsatisfied RFC-044 requirements; never blocking). Records are never touched. \
priorBundles (JSON texts of earlier published bundles of the same package) are proof only: they must \
be published artifacts you verified (e.g. by sha256), never a bundle exported from the user's own \
edited boundary, which would prove edits clean; one not older than the new bundle or newer than the \
installed version is refused. adopt matches by id and covers every version of that id; a conflict is \
adoptable exactly when its conflictKind is no-reference-copy. A local-edit \
or key-collision is never adoptable (a note says so). Then repeat with dryRun false to apply, and run \
repo_validate. boundaryPath picks the boundary when \
the package is installed at several.";

pub const DESC_ATTACHMENT_ADD: &str = "Store a file as a source-document attachment (content file + \
    .meta.json sidecar) and return its documentId. Give `content` (UTF-8 text) or `contentBase64` \
    (binary), not both. The repository's attachment_policy (size / MIME / total limits) is enforced: \
    a violating file is rejected and nothing is written. Follow with attachment_link to attach it \
    to a record.";
pub const DESC_ATTACHMENT_LINK: &str = "Attach a stored source document to a record (appends a \
    sourceRef with sourceRole 'attaches'). The document must already exist (attachment_add); \
    linking the same pair twice is an error.";
pub const DESC_NEIGHBOURS: &str = "Bounded read of one instance's relation neighbours (Record or \
Note). Returns total (every matching edge, before paging) and a page of edges, each with direction \
(out = the instance is the source, in = it is the target), relationId, relationType and the \
neighbour's instanceId, label and type; never the neighbour record (read srs://<repositoryId>/record/{id} \
for that). Optional relationType and direction (out|in; default both) filter; limit (default 25, max \
100) and offset page. Prefer this over the context resource for hub records with many edges.";

pub const DESC_READ: &str = "Read any srs:// resource and return exactly what resources/read \
returns for it (map, navigation, agent-index, tree, tree/{instanceId}, record/{instanceId}, \
context/{containerId}/{instanceId}, container/{id}, composition/{compositionId}[?containerId=&excludeInstanceId=], type/{typeId}, protocol, \
protocol/{id}, relation-types). For clients that cannot read resources. Output text is capped at 96000 bytes (before the notice); a longer \
resource is cut with a trailing notice and structuredContent.truncated = true — then use find \
{limit}, tree/{instanceId}, container_outline or record/{id} for a bounded read.";

pub const DESC_SIMILAR: &str = "More like this: instances whose text overlaps the most \
characteristic terms of one instance (instanceId), ranked by the same BM25 index as find. Use it \
after find to ask what else in the repository is about a record, including content that uses \
different wording from a query you would have guessed. Returns find-shaped hits (instanceId, uri, \
label, type, score; projection and facets as in find), best first, never the source itself. The structured filters of find (typeId, \
typeNamespace, typeName, containerId, tag, lifecycleState, tier, ...) narrow the candidates; \
there is no contentMatch. Deterministic and lexical: no embeddings. limit defaults to 25.";

pub const DESC_FIND: &str = "Deterministic discovery query (ext:discovery). All axes are \
optional and AND-combined: typeId, typeNamespace, typeName, containerId, tag (repeatable; \
instance must carry ALL), lifecycleState, excludeLifecycleStates, tier, and contentMatch \
(recall floor: matches records containing every whitespace-separated word, in any field and any \
order, not just the title; a phrase match is always included). match: \"any\" instead matches records \
containing any significant query word as a whole word (words found in most records, like \"the\" or \
\"what\", are ignored), so a question typed as a sentence still finds what it is about; any-mode is \
always ranked (BM25 puts records matching the most and rarest words first). Hits are ranked by BM25 \
relevance (score) unless rank is false, which orders by instanceId. Types are written \
'namespace/name'. Returns hits (projection \"full\", the default) with instanceId, uri, label, \
typeId, typeNamespace/typeName, containerIds, lifecycleState, score, snippet and matchedFields; projection \"card\" keeps instanceId, uri, label, type, lifecycleState, score and \
snippet, and \"label\" drops score and snippet too: use them to scan before you read. facets: true \
adds counts over the WHOLE match set, before limit/offset (byType, tags, notes, and one entry per \
closed string field (at most 25), each the top 20 values plus an other count). find with limit 0 \
returns facets by default and no hits: with no filters it is the cheap map of the repository; add a \
type filter for that type's keyword map. Serves Tier 2 (Records) and Tier 0 \
(Notes; type and lifecycle filters exclude them). A typeId, type, or containerId that names nothing returns zero hits with a warning \
diagnostic.";

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

pub const DESC_RELATION_DELETE: &str = "Delete a relation by relationId (the same service as \
`srs relation delete`). Only the edge is removed; neither endpoint is touched. In a guarded \
agent session you may delete only relations you created (createdBy equals your session actor); \
any other relation is refused. The context read shows each relation's createdAt and createdBy.";

pub const DESC_NOTE_CREATE: &str = "Create a Tier-0 Note (free-text sections, no type \
binding). Each section has a name, content, and optional label. Optional containerId adds the \
note to a container atomically. Notes are the capture tier — graduate one to a typed Record \
later when its structure stabilises.";

pub const DESC_NOTE_UPDATE: &str = "Update a Tier-0 Note by instanceId. Whole-object over the \
authoring fields: title, tags and sections are replaced by what you send (omit title/tags to \
clear them); createdBy, createdAt and other provenance are preserved. To read a note first, use \
find or the read tool on its record URI.";

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
pinned typeVersion). relationType is 'supersedes' or 'refines'; omit it and the core derives it from the predecessor's lifecycle requiresRelation (RFC-022), or errors naming the candidates. The returned relation is authoritative. Validation is enforced \
before any write. Returns both the new Record and the linking Relation.";

pub const DESC_NOTE_GRADUATE: &str = "Promote a Tier-0 Note to a typed Tier-2 Record in \
one atomic step. A new Record is created from the supplied type and fieldValues, and a \
derived-from Relation (Record -> Note) is asserted atomically as the graduation's sole \
provenance record. Optional containerId adds the Record to a container. The Note is \
preserved unchanged — it is not deleted, and its graduatedAt field is never stamped. Returns \
both the Note and the new Record.";

pub const DESC_CONTAINER_CREATE: &str = "Create a container (an essay, a section, a document \
boundary). `title` is required; `containerId` is minted when omitted. `memberInstanceIds` is the \
initial ordered outline of `{instanceId, depth?}` entries (each must resolve to an existing \
instance; first entry depth 0, depth rises by at most one per entry). `anchorInstanceId` (the \
typing anchor) and `identityInstanceId` (the container's identity/purpose record) should each \
name an entry (an anchor outside the members is reported by repo_validate, I-145). Invalid input \
is rejected whole with the validation message. \
Returns the created container.";

pub const DESC_CONTAINER_MEMBER_ADD: &str = "Add an instance to a container's ordered \
memberInstanceIds outline (RFC-043). With no position it appends at depth 0; `position` (0-based) \
inserts there and `depth` (default 0) sets the nesting level (an entry's parent is the nearest \
preceding entry with a smaller depth; depth may rise by at most one per entry). Order and depth are \
layout only: use a precedes relation when order is a semantic claim. Idempotent — adding an \
already-present member with no position or depth is not an error. Instead of position/depth, \
`relativeTo` + `placement` (before | after | into) adds it beside or as the last child of another \
entry, in one write. Returns the container's entries ({instanceId, depth?}) in order.";

pub const DESC_CONTAINER_MEMBER_REMOVE: &str = "Remove an instance from a container's outline. \
Its descendants are promoted one level (RFC-043 promoting removal) and reported in `promoted`. \
Rejected (arrangement-pointer) when the entry is the container's identityInstanceId or \
anchorInstanceId. No-op if the instance is not a member. Returns the container's entries in order.";

pub const DESC_CONTAINER_MEMBER_MOVE: &str = "Move an entry (with its descendants, which keep \
their relative depths) to `position` (0-based, against the list without that run; default: where \
it is) and/or give it `depth` (default: its current depth; setting only `depth` indents or outdents \
the run). Rejected whole, changing nothing, if the result would break the outline rules (first \
entry depth 0, depth rises by at most one). Instead of position/depth, give `relativeTo` + `placement` \
(before | after = beside that entry's run; into = its last child), or `shift`: indent (depth + 1, \
clamped to the previous entry's depth + 1), outdent (depth - 1), up / down (swap with the previous / \
next sibling; a no-op at the first / last). Placing an entry relative to itself or its own descendant \
is rejected, as is any result that nests or gives children to the root identity entry. Returns the container's entries in order.";

pub const DESC_CONTAINER_OUTLINE: &str = "Read a container's outline: `entries` (the whole \
arrangement) and `body` (the document body: entries without the container's anchor and identity \
entries, whose descendants are promoted), each as {instanceId, depth, parentInstanceId, hasChildren, \
runSize, runEnd} (runEnd = exclusive index of the entry's run in the same list). Use this instead \
of deriving parents or run ends from the flat entries.";

pub const DESC_CONTAINER_MEMBER_REPAIR: &str = "Remove every outline entry whose instanceId no \
longer resolves to an instance, promoting descendants, and report each in `removed`. Idempotent. \
Never changes identityInstanceId or anchorInstanceId (a dangling pointer stays a validation error).";

pub const DESC_CONTAINER_COPY: &str =
    "Copy a container (a document): the new container SHARES the \
source's member records (outline copied verbatim, childContainerIds shared), so no member record \
is duplicated. Only the anchor (title) record is forked - a new record linked `derived-from` the \
original - and the copy's anchor/identity/outline entry point at the fork. `title` defaults to \
\"<title> (copy)\"; `containerId` is minted when omitted (an existing id is refused). The \
repository root container cannot be copied. Returns `{container, forks, relations}`.";

pub const DESC_RECORD_FORK: &str =
    "Make a local copy: fork the record `instanceId` and its nested \
children (its outline subtree in `containerId`) into new records linked `derived-from` the \
originals, swapped into THAT container only, in place (same order and depth). Every other \
container keeps the originals. Same type and field values; the forks are attributed to the \
session actor; only fieldValues carry over (not tags/meta). The container's anchor/identity entries cannot be forked. \
`targetContainerId` forks into that container instead (the record is appended to its outline first if not a member; \
`containerId` may then be omitted). `carryRelations` (`none` default | `outgoing` | `all`) re-creates the original's \
relations on the fork (never `derived-from`; the original's are untouched; attributed to the session actor). Returns \
`{containerId, forks: [{originalId, forkId, carriedRelationIds?}], relations, carriedRelations?}`.";

// Protocol run execution tool descriptions (#977 — follow-up to #955)

pub const DESC_PROTOCOL_RUN_CREATE: &str = "Start a new run of an installed Protocol against a \
container (and optionally a target record). protocolVersion must match the installed Protocol's \
version. Supplying initialStageId opens that stage as Active immediately; omit it to create the \
run with no stage yet active. Returns the created run, including its assigned runId.";

pub const DESC_PROTOCOL_RUN_ADVANCE: &str = "Move an Active run to a new stage, appending it as \
Active to stageStates. completeCurrent controls whether the run's currently-Active stage(s) are \
marked Completed first (set true when the current stage is actually finished; false to hold \
multiple stages Active at once). Only a run whose status is Active can be advanced — advancing a \
Completed or Abandoned run is rejected. Returns the updated run.";

pub const DESC_PROTOCOL_RUN_GET: &str = "Get a single protocol run by its runId, including its \
full stageStates history and current attentionState. Returns an error if no run has that id.";

pub const DESC_PROTOCOL_RUN_LIST: &str = "List protocol run summaries, optionally filtered by \
protocolId, containerId, and/or status (one of \"Active\", \"Completed\", \"Abandoned\" — \
AND-combined with the other filters). Omit all filters to list every run. Each summary carries \
runId, protocolId, containerId, status, currentStageId, and startedAt; call protocol_run_get for \
the full stageStates history.";

pub const DESC_PROTOCOL_RUN_COMPLETE: &str = "Mark an Active run as Completed, and its \
currently-Active stage(s) as Completed alongside it. Only a run whose status is Active can be \
completed — completing an already-Completed or Abandoned run is rejected. Returns the updated \
run.";

pub const DESC_PROTOCOL_RUN_ABANDON: &str = "Mark an Active run as Abandoned. Only a run whose \
status is Active can be abandoned — abandoning an already-Completed or Abandoned run is \
rejected. Returns the updated run.";

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
    /// Maximum hits to return. Defaults to 25 when omitted; `total` in the
    /// result always gives the full match count, so page with `offset`.
    pub limit: Option<usize>,
    /// Number of hits to skip (default 0), after the deterministic sort.
    pub offset: Option<usize>,
    /// Order hits by BM25 relevance (fills `score`) instead of by instanceId.
    /// Defaults to true; the set of hits is the same either way.
    pub rank: Option<bool>,
    /// How contentMatch words combine: "all" (default) — every word must occur;
    /// "any" — any significant word, as a whole word (for natural-language queries; always ranked).
    #[serde(rename = "match")]
    pub match_mode: Option<MatchMode>,
    /// Cap on `facets.byType` values (default 20; the rest are summed into `other`).
    /// 0 returns every type, each with its `typeId`.
    pub by_type_limit: Option<usize>,
    /// Include facets. Default: only when limit is 0 (the repository map).
    pub facets: Option<bool>,
    /// How much of each hit: "full" (default), "card" (instanceId, uri, label, type,
    /// lifecycleState, score, snippet) or "label" (as card, without score and snippet).
    pub projection: Option<Projection>,
}

/// Input of the `similar` tool: the source `instanceId` plus find's structured filters
/// (no `contentMatch`: the source is the query).
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SimilarToolInput {
    /// The instance (Record or Note) to find neighbours of.
    pub instance_id: String,
    pub type_id: Option<String>,
    pub type_namespace: Option<String>,
    pub type_name: Option<String>,
    pub container_id: Option<String>,
    /// AND-conjunction: the instance's tags must contain ALL specified values.
    #[serde(default)]
    pub tag: Vec<String>,
    pub lifecycle_state: Option<String>,
    #[serde(default)]
    pub lifecycle_states: Vec<String>,
    #[serde(default)]
    pub exclude_lifecycle_states: Vec<String>,
    /// Instance tier (0=Note, 2=Record).
    pub tier: Option<u8>,
    /// Maximum hits to return (default 25); `total` is the full count of similar instances.
    pub limit: Option<usize>,
    /// Number of hits to skip (default 0).
    pub offset: Option<usize>,
    /// How much of each hit: "full" (default), "card" or "label", as in find.
    pub projection: Option<Projection>,
    /// Include facets over the similar set, as in find (default: only when limit is 0).
    pub facets: Option<bool>,
}

impl SimilarToolInput {
    fn into_parts(self) -> (String, DiscoveryQuery, FindPage) {
        let page = FindPage {
            limit: Some(self.limit.unwrap_or(FIND_DEFAULT_LIMIT)),
            offset: self.offset.unwrap_or(0),
            rank: true,
            projection: self.projection.unwrap_or_default(),
            facets: self.facets,
            ..Default::default()
        };
        let query = DiscoveryQuery {
            type_id: self.type_id,
            type_namespace: self.type_namespace,
            type_name: self.type_name,
            container_id: self.container_id,
            tag: self.tag,
            lifecycle_state: self.lifecycle_state,
            lifecycle_states: self.lifecycle_states,
            exclude_lifecycle_states: self.exclude_lifecycle_states,
            tier: self.tier,
            content_match: None,
        };
        (self.instance_id, query, page)
    }
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

pub(crate) fn field_meta_map(
    input: Option<std::collections::BTreeMap<String, FieldMetaInput>>,
) -> Option<indexmap::IndexMap<String, FieldMeta>> {
    input.map(|m| m.into_iter().map(|(k, v)| (k, v.into())).collect())
}

/// Builds the `extra` envelope-extras bag (srs-rust#1031) from the tool
/// surface's typed `meta` field, so it round-trips onto `Record.extra`.
fn meta_extra(meta: Option<Value>) -> std::collections::BTreeMap<String, Value> {
    let mut extra = std::collections::BTreeMap::new();
    if let Some(m) = meta {
        extra.insert("meta".to_string(), m);
    }
    extra
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
    /// Optional initial lifecycle state, overriding the effective Lifecycle's
    /// `initialState` — must be reachable from it via declared transitions
    /// (srs-rust#960). Mirrors `record_successor`'s `lifecycleState`.
    pub lifecycle_state: Option<String>,
    /// Envelope extra — round-trips onto `Record.extra["meta"]` (srs-rust#1031).
    pub meta: Option<Value>,
}

impl From<RecordCreateToolInput> for CreateRecordInput {
    fn from(input: RecordCreateToolInput) -> Self {
        CreateRecordInput {
            field_values: FieldValues(input.field_values),
            field_meta: field_meta_map(input.field_meta),
            tags: input.tags,
            lifecycle_state: input.lifecycle_state,
            extra: meta_extra(input.meta),
        }
    }
}

/// Mirrors the authoring surface of `srs_core::types::relation::Relation`.
///
/// First-cut narrowing (tracked in srs-rust#680): `sourceRefs` is not
/// exposed — it belongs to the sourceRef-authoring wave. `assertedBy`,
/// `confidence`, `createdBy`, `status`, `validFrom`, `validUntil`,
/// `sourceRepositoryId`, and `targetRepositoryId` are not exposed either —
/// srs#441 removed them from the canonical schema and `Relation` no longer
/// has fields for them (srs-rust#1022).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationCreateToolInput {
    /// Assigned automatically when omitted.
    pub relation_id: Option<String>,
    pub relation_type: String,
    pub source_instance_id: String,
    pub target_instance_id: String,
    pub created_at: Option<String>,
    pub notes: Option<String>,
    pub meta: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationDeleteToolInput {
    pub relation_id: String,
}

impl From<RelationCreateToolInput> for Relation {
    fn from(input: RelationCreateToolInput) -> Self {
        Relation {
            created_by: None,
            relation_id: input.relation_id.unwrap_or_default(),
            relation_type: input.relation_type,
            source_instance_id: input.source_instance_id,
            target_instance_id: input.target_instance_id,
            created_at: input.created_at,
            notes: input.notes,
            source_refs: None,
            meta: input.meta,
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
                created_by: None,
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

/// Mirrors `services::UpdateNoteContentInput` (plus `instanceId`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteUpdateToolInput {
    pub instance_id: String,
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
    pub sections: Vec<NoteSectionInput>,
}

impl From<NoteUpdateToolInput> for UpdateNoteContentInput {
    fn from(input: NoteUpdateToolInput) -> Self {
        UpdateNoteContentInput {
            title: input.title,
            tags: input.tags,
            sections: input.sections.into_iter().map(Into::into).collect(),
        }
    }
}

/// The whole input of `read`: an `srs://<repositoryId>/…` resource address.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadToolInput {
    /// The resource address, e.g. `srs://<repositoryId>/map`.
    pub uri: String,
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
    /// Envelope extra — omit to preserve the stored value, present to replace
    /// it (srs-rust#1031). Round-trips onto `Record.extra["meta"]`.
    #[serde(default)]
    pub meta: Option<Value>,
}

impl From<RecordUpdateToolInput> for UpdateRecordInput {
    fn from(input: RecordUpdateToolInput) -> Self {
        UpdateRecordInput {
            field_values: FieldValues(input.field_values),
            field_meta: field_meta_map(input.field_meta),
            tags: input.tags,
            type_version: input.type_version,
            extra: meta_extra(input.meta),
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
    /// Optional: omitted, the core derives it from the predecessor's lifecycle (RFC-022 R6).
    pub relation_type: Option<String>,
    /// RFC-039 carrier: an object keyed by `Field.name` verbatim.
    pub field_values: serde_json::Map<String, Value>,
    pub lifecycle_state: Option<String>,
    pub type_version: Option<u32>,
    /// Envelope extra, e.g. `meta.derivedFrom` — round-trips onto the new
    /// successor's `Record.extra["meta"]` (srs-rust#1031).
    pub meta: Option<Value>,
}

impl From<RecordSuccessorToolInput> for CreateRecordSuccessorInput {
    fn from(input: RecordSuccessorToolInput) -> Self {
        CreateRecordSuccessorInput {
            relation_type: input.relation_type,
            field_values: FieldValues(input.field_values),
            lifecycle_state: input.lifecycle_state,
            type_version: input.type_version,
            extra: meta_extra(input.meta),
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
                lifecycle_state: None,
                extra: std::collections::BTreeMap::new(),
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

/// `container_member_add`: `position` (0-based; default append) and `depth` (default 0) per
/// RFC-043 Change D.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerMemberAddToolInput {
    pub container_id: String,
    pub instance_id: String,
    pub position: Option<usize>,
    pub depth: Option<u32>,
    /// With `placement`: add beside / inside this entry instead of by `position` / `depth`.
    pub relative_to: Option<String>,
    /// `before` | `after` | `into` (with `relativeTo`).
    pub placement: Option<String>,
}

/// `container_member_move`: move (position) and/or set depth (depth) of an entry's whole run.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerMemberMoveToolInput {
    pub container_id: String,
    pub instance_id: String,
    pub position: Option<usize>,
    pub depth: Option<u32>,
    /// With `placement`: move beside / inside this entry (replaces `position` / `depth`).
    pub relative_to: Option<String>,
    /// `before` | `after` | `into` (with `relativeTo`).
    pub placement: Option<String>,
    /// `indent` | `outdent` | `up` | `down` (replaces `position` / `depth` / `relativeTo`).
    pub shift: Option<String>,
}

/// `package_upgrade`: mirrors the WASM `upgrade_package_bundle(bundle_json, options_json)`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageUpgradeToolInput {
    /// The `.srspkg` Package Bundle file's JSON text.
    pub bundle: String,
    /// Compute the plan without writing anything. Default false.
    #[serde(default)]
    pub dry_run: bool,
    /// The installed boundary to upgrade; default = the one installed with the bundle's packageId.
    pub boundary_path: Option<String>,
    /// Earlier published `.srspkg` JSON texts of the same package (same packageId), used only to
    /// prove an installed definition without a reference copy unedited.
    #[serde(default)]
    pub prior_bundles: Vec<String>,
    /// Definition ids to replace although nothing proves them clean (no-reference-copy only).
    #[serde(default)]
    pub adopt: Vec<String>,
}

/// `package_dependency_list`: the requiring boundary.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencyListToolInput {
    /// Package boundary path; omit for the primary package.
    pub selector: Option<String>,
}

/// Mirrors `package_dependency_service::AddPackageDependencyInput` field-for-field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencySetToolInput {
    /// Requiring package boundary path; omit for the primary package.
    pub selector: Option<String>,
    /// The required package's `id` (UUID).
    pub package_id: String,
    /// SemVer 2.0.0 requirement version.
    pub version: String,
    /// Replace the legacy entry (no packageId) whose namespace/name equal the
    /// installed package's labels exactly. Default false.
    #[serde(default)]
    pub repair_legacy: bool,
}

impl From<PackageDependencySetToolInput> for AddPackageDependencyInput {
    fn from(input: PackageDependencySetToolInput) -> Self {
        AddPackageDependencyInput {
            selector: input.selector,
            package_id: input.package_id,
            version: input.version,
            repair_legacy: input.repair_legacy,
        }
    }
}

/// Mirrors `package_dependency_service::RemovePackageDependencyInput` field-for-field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageDependencyRemoveToolInput {
    /// Requiring package boundary path; omit for the primary package.
    pub selector: Option<String>,
    /// The required package's `id` (UUID).
    pub package_id: String,
}

impl From<PackageDependencyRemoveToolInput> for RemovePackageDependencyInput {
    fn from(input: PackageDependencyRemoveToolInput) -> Self {
        RemovePackageDependencyInput {
            selector: input.selector,
            package_id: input.package_id,
        }
    }
}

/// `neighbours`: `NeighboursQuery` plus the `NeighboursPage` paging (srs-rust#1229).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NeighboursToolInput {
    /// The Record or Note whose edges to list.
    pub instance_id: String,
    /// Only edges of this relation type (e.g. `contains`).
    pub relation_type: Option<String>,
    /// `out` (instance is the source) or `in` (instance is the target); omit for both.
    #[schemars(with = "Option<String>")]
    pub direction: Option<EdgeDirection>,
    /// Maximum edges to return. Defaults to 25, capped at 100; `total` always counts every match.
    pub limit: Option<usize>,
    /// Edges to skip (default 0), after the deterministic sort.
    pub offset: Option<usize>,
}

impl NeighboursToolInput {
    fn into_parts(self) -> (NeighboursQuery, NeighboursPage) {
        (
            NeighboursQuery {
                instance_id: self.instance_id,
                relation_type: self.relation_type,
                direction: self.direction,
            },
            NeighboursPage {
                limit: Some(
                    self.limit
                        .unwrap_or(NEIGHBOURS_DEFAULT_LIMIT)
                        .min(NEIGHBOURS_MAX_LIMIT),
                ),
                offset: self.offset.unwrap_or(0),
            },
        )
    }
}

/// `container_member_repair` / `container_outline`: the container alone.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerIdToolInput {
    pub container_id: String,
}

/// `container_copy`: the source container plus the optional `ContainerCopyInput` keys.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerCopyToolInput {
    pub source_container_id: String,
    pub title: Option<String>,
    pub container_id: Option<String>,
}

/// `record_fork`: fork `instance_id` (and its nested children) inside `container_id`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordForkToolInput {
    /// Container the fork is swapped into. Optional when `targetContainerId` is given.
    pub container_id: Option<String>,
    pub instance_id: String,
    /// Fork into this container, appending the record to its outline first when it is not a
    /// member. Wins over `containerId`.
    pub target_container_id: Option<String>,
    /// `none` (default) | `outgoing` | `all`: re-create the original's relations on the fork.
    #[serde(default)]
    pub carry_relations: srs_repository::fork_service::CarryRelations,
}

// ── Protocol run shadow input structs (#977) ──────────────────────────────────

/// Mirrors `protocol_run_service::CreateRunInput` field-for-field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolRunCreateToolInput {
    pub protocol_id: String,
    pub protocol_version: i32,
    pub container_id: String,
    pub target_record_id: Option<String>,
    pub initial_stage_id: Option<String>,
}

impl From<ProtocolRunCreateToolInput> for CreateRunInput {
    fn from(input: ProtocolRunCreateToolInput) -> Self {
        CreateRunInput {
            protocol_id: input.protocol_id,
            protocol_version: input.protocol_version,
            container_id: input.container_id,
            target_record_id: input.target_record_id,
            initial_stage_id: input.initial_stage_id,
        }
    }
}

/// Mirrors `protocol_run_service::AdvanceStageInput` field-for-field.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolRunAdvanceToolInput {
    pub run_id: String,
    pub stage_id: String,
    pub complete_current: bool,
}

impl From<ProtocolRunAdvanceToolInput> for AdvanceStageInput {
    fn from(input: ProtocolRunAdvanceToolInput) -> Self {
        AdvanceStageInput {
            run_id: input.run_id,
            stage_id: input.stage_id,
            complete_current: input.complete_current,
        }
    }
}

/// No service struct conversion needed — `run_id` is passed directly to
/// `protocol_run_service::get_run`/`complete_run`/`abandon_run` (follows the
/// `RecordAllowedTransitionsToolInput` pattern).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolRunIdToolInput {
    pub run_id: String,
}

/// Mirrors `protocol_run_service::RunListFilter` field-for-field.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolRunListToolInput {
    pub protocol_id: Option<String>,
    pub container_id: Option<String>,
    /// "Active" | "Completed" | "Abandoned"
    pub status: Option<String>,
}

impl From<ProtocolRunListToolInput> for RunListFilter {
    fn from(input: ProtocolRunListToolInput) -> Self {
        RunListFilter {
            protocol_id: input.protocol_id,
            container_id: input.container_id,
            status: input.status,
        }
    }
}

/// Wraps the `Vec<RunSummary>` returned by `protocol_run_service::list_runs` so
/// the MCP response is a named object (`{runs: [...]}`) rather than a bare JSON
/// array (follows the `ContainerMembersToolResult` pattern).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRunListToolResult {
    pub runs: Vec<RunSummary>,
}

// ── Tool listing ──────────────────────────────────────────────────────────────

fn input_schema<T: JsonSchema>() -> Value {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    match serde_json::to_value(schema) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => json!({}),
    }
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": input_schema })
}

/// The `tools/list` result for the whole catalogue.
pub fn list_tools() -> Value {
    list_tools_for(ToolProfile::Full)
}

/// The `tools/list` result for one [`ToolProfile`]: the catalogue, filtered.
pub fn list_tools_for(profile: ToolProfile) -> Value {
    let mut tools = all_tools();
    tools.retain(|t| t["name"].as_str().is_some_and(|n| profile.allows(n)));
    json!({ "tools": tools })
}

fn all_tools() -> Vec<Value> {
    vec![
        tool(
            TOOL_REPO_VALIDATE,
            DESC_REPO_VALIDATE,
            input_schema::<EmptyToolInput>(),
        ),
        tool(TOOL_FIND, DESC_FIND, input_schema::<FindToolInput>()),
        tool(
            TOOL_RECORD_CREATE,
            DESC_RECORD_CREATE,
            input_schema::<RecordCreateToolInput>(),
        ),
        tool(
            TOOL_RELATION_CREATE,
            DESC_RELATION_CREATE,
            input_schema::<RelationCreateToolInput>(),
        ),
        tool(
            TOOL_RELATION_DELETE,
            DESC_RELATION_DELETE,
            input_schema::<RelationDeleteToolInput>(),
        ),
        tool(
            TOOL_NOTE_CREATE,
            DESC_NOTE_CREATE,
            input_schema::<NoteCreateToolInput>(),
        ),
        tool(
            TOOL_NOTE_UPDATE,
            DESC_NOTE_UPDATE,
            input_schema::<NoteUpdateToolInput>(),
        ),
        tool(
            TOOL_TYPE_SCHEMA,
            DESC_TYPE_SCHEMA,
            input_schema::<TypeSchemaToolInput>(),
        ),
        tool(TOOL_READ, DESC_READ, input_schema::<ReadToolInput>()),
        // Second-wave write tools (#680)
        tool(
            TOOL_RECORD_UPDATE,
            DESC_RECORD_UPDATE,
            input_schema::<RecordUpdateToolInput>(),
        ),
        tool(
            TOOL_RECORD_TRANSITION,
            DESC_RECORD_TRANSITION,
            input_schema::<RecordTransitionToolInput>(),
        ),
        tool(
            TOOL_RECORD_ALLOWED_TRANSITIONS,
            DESC_RECORD_ALLOWED_TRANSITIONS,
            input_schema::<RecordAllowedTransitionsToolInput>(),
        ),
        tool(
            TOOL_RECORD_SUCCESSOR,
            DESC_RECORD_SUCCESSOR,
            input_schema::<RecordSuccessorToolInput>(),
        ),
        tool(
            TOOL_NOTE_GRADUATE,
            DESC_NOTE_GRADUATE,
            input_schema::<NoteGraduateToolInput>(),
        ),
        tool(
            TOOL_CONTAINER_CREATE,
            DESC_CONTAINER_CREATE,
            input_schema::<ContainerCreateInput>(),
        ),
        tool(
            TOOL_CONTAINER_MEMBER_ADD,
            DESC_CONTAINER_MEMBER_ADD,
            input_schema::<ContainerMemberAddToolInput>(),
        ),
        tool(
            TOOL_CONTAINER_MEMBER_REMOVE,
            DESC_CONTAINER_MEMBER_REMOVE,
            input_schema::<ContainerMemberToolInput>(),
        ),
        tool(
            TOOL_CONTAINER_MEMBER_MOVE,
            DESC_CONTAINER_MEMBER_MOVE,
            input_schema::<ContainerMemberMoveToolInput>(),
        ),
        tool(
            TOOL_CONTAINER_MEMBER_REPAIR,
            DESC_CONTAINER_MEMBER_REPAIR,
            input_schema::<ContainerIdToolInput>(),
        ),
        tool(
            TOOL_CONTAINER_OUTLINE,
            DESC_CONTAINER_OUTLINE,
            input_schema::<ContainerIdToolInput>(),
        ),
        tool(
            TOOL_CONTAINER_COPY,
            DESC_CONTAINER_COPY,
            input_schema::<ContainerCopyToolInput>(),
        ),
        tool(
            TOOL_RECORD_FORK,
            DESC_RECORD_FORK,
            input_schema::<RecordForkToolInput>(),
        ),
        // Protocol run execution tools (#977)
        tool(
            TOOL_PROTOCOL_RUN_CREATE,
            DESC_PROTOCOL_RUN_CREATE,
            input_schema::<ProtocolRunCreateToolInput>(),
        ),
        tool(
            TOOL_PROTOCOL_RUN_ADVANCE,
            DESC_PROTOCOL_RUN_ADVANCE,
            input_schema::<ProtocolRunAdvanceToolInput>(),
        ),
        tool(
            TOOL_PROTOCOL_RUN_GET,
            DESC_PROTOCOL_RUN_GET,
            input_schema::<ProtocolRunIdToolInput>(),
        ),
        tool(
            TOOL_PROTOCOL_RUN_LIST,
            DESC_PROTOCOL_RUN_LIST,
            input_schema::<ProtocolRunListToolInput>(),
        ),
        tool(
            TOOL_PROTOCOL_RUN_COMPLETE,
            DESC_PROTOCOL_RUN_COMPLETE,
            input_schema::<ProtocolRunIdToolInput>(),
        ),
        tool(
            TOOL_PROTOCOL_RUN_ABANDON,
            DESC_PROTOCOL_RUN_ABANDON,
            input_schema::<ProtocolRunIdToolInput>(),
        ),
        // RFC-044 package requirements (srs-rust#1168)
        tool(
            TOOL_PACKAGE_DEPENDENCY_LIST,
            DESC_PACKAGE_DEPENDENCY_LIST,
            input_schema::<PackageDependencyListToolInput>(),
        ),
        tool(
            TOOL_PACKAGE_DEPENDENCY_SET,
            DESC_PACKAGE_DEPENDENCY_SET,
            input_schema::<PackageDependencySetToolInput>(),
        ),
        tool(
            TOOL_PACKAGE_DEPENDENCY_REMOVE,
            DESC_PACKAGE_DEPENDENCY_REMOVE,
            input_schema::<PackageDependencyRemoveToolInput>(),
        ),
        tool(
            TOOL_PACKAGE_UPGRADE,
            DESC_PACKAGE_UPGRADE,
            input_schema::<PackageUpgradeToolInput>(),
        ),
        tool(
            TOOL_NEIGHBOURS,
            DESC_NEIGHBOURS,
            input_schema::<NeighboursToolInput>(),
        ),
        tool(
            TOOL_SIMILAR,
            DESC_SIMILAR,
            input_schema::<SimilarToolInput>(),
        ),
        tool(
            TOOL_ATTACHMENT_ADD,
            DESC_ATTACHMENT_ADD,
            input_schema::<AttachmentAddToolInput>(),
        ),
        tool(
            TOOL_ATTACHMENT_LINK,
            DESC_ATTACHMENT_LINK,
            input_schema::<AttachmentLinkToolInput>(),
        ),
    ]
}

// ── Attachment tools (srs-rust#1327) ──────────────────────────────────────────

/// Mirrors `attachment_service::AddAttachmentInput`; exactly one of `content` /
/// `content_base64` carries the bytes. Always policy-enforcing for agents.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentAddToolInput {
    /// File name without path separators, e.g. "brief.pdf".
    pub file_name: String,
    /// UTF-8 text content. Mutually exclusive with `contentBase64`.
    pub content: Option<String>,
    /// Base64 (standard alphabet) content for binary files. Mutually exclusive with `content`.
    pub content_base64: Option<String>,
    pub title: Option<String>,
    /// Subdirectory under the source-documents directory.
    pub subdir: Option<String>,
    /// MIME type; inferred from the file extension when omitted.
    pub content_type: Option<String>,
}

impl TryFrom<AttachmentAddToolInput> for AddAttachmentInput {
    type Error = McpApplicationError;
    fn try_from(input: AttachmentAddToolInput) -> Result<Self, Self::Error> {
        use base64::Engine as _;
        let content = match (input.content, input.content_base64) {
            (Some(text), None) => text.into_bytes(),
            (None, Some(b64)) => base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| McpApplicationError::invalid_params(format!("contentBase64: {e}")))?,
            _ => {
                return Err(McpApplicationError::invalid_params(
                    "give exactly one of content or contentBase64".to_string(),
                ))
            }
        };
        Ok(AddAttachmentInput {
            file_name: input.file_name,
            content,
            subdir: input.subdir,
            title: input.title,
            content_type: input.content_type,
            enforce_policy: true,
        })
    }
}

/// Mirrors `attachment_service::LinkAttachmentInput`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentLinkToolInput {
    pub instance_id: String,
    pub document_id: String,
}

impl From<AttachmentLinkToolInput> for LinkAttachmentInput {
    fn from(input: AttachmentLinkToolInput) -> Self {
        LinkAttachmentInput {
            instance_id: input.instance_id,
            document_id: input.document_id,
        }
    }
}

// ── Tool dispatch ─────────────────────────────────────────────────────────────

fn parse_args<T: for<'de> Deserialize<'de>>(
    arguments: Option<Map<String, Value>>,
) -> Result<T, McpApplicationError> {
    serde_json::from_value(Value::Object(arguments.unwrap_or_default()))
        .map_err(|e| McpApplicationError::invalid_params(e.to_string()))
}

/// Parse the relative-placement arguments; mixing them with absolute `position` / `depth` is
/// an invalid-params error (one way per goal).
fn relative_move(
    relative_to: Option<&str>,
    placement: Option<&str>,
    shift: Option<&str>,
    has_absolute: bool,
) -> Result<Option<RelativeMove>, McpApplicationError> {
    let mv = RelativeMove::parse(relative_to, placement, shift)
        .map_err(McpApplicationError::invalid_params)?;
    if mv.is_some() && has_absolute {
        return Err(McpApplicationError::invalid_params(
            "relativeTo / placement / shift cannot be combined with position or depth".to_string(),
        ));
    }
    Ok(mv)
}

/// Success result: the service struct serialized as JSON text + structured content.
fn tool_ok<T: serde::Serialize>(value: &T) -> Result<Value, McpApplicationError> {
    let structured =
        serde_json::to_value(value).map_err(|e| McpApplicationError::internal(e.to_string()))?;
    let text = crate::json_text(&structured)?;
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": structured,
        "isError": false
    }))
}

/// Service rejection → tool-level error the model can read (not a protocol error).
pub(crate) fn tool_err(report: srs_repository::ErrorReport) -> Value {
    json!({ "content": [{ "type": "text", "text": report.message }], "structuredContent": report, "isError": true })
}

/// `read` result cap: under the ~128 KB browser-relay limit, with headroom for JSON framing.
pub const MAX_READ_BYTES: usize = 96_000;

/// Largest char boundary of `s` at or below `max`.
fn utf8_floor(s: &str, max: usize) -> usize {
    let mut cut = max.min(s.len());
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// `read {uri}` (#1220): the `resources/read` dispatch verbatim (one path, errors as there),
/// returned as a tool result and capped at [`MAX_READ_BYTES`] (policy, not protocol). Needs the
/// repository id, so `SrsMcpApplication` routes it here instead of through [`call_tool`]; it
/// is read-only and deliberately bypasses the write guard and change drain (ADR-049).
/// `structuredContent` carries the same text as `content[0].text` (for clients that surface only
/// structured content) plus metadata. A truncated
/// JSON text is no longer valid JSON: `structuredContent.truncated` is the contract.
pub fn read_tool(
    store: &dyn RepositoryStore,
    repository_id: &str,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, McpApplicationError> {
    read_tool_capped(store, repository_id, arguments, MAX_READ_BYTES)
}

/// [`read_tool`] with an explicit cap (so tests need not build a 96 KB resource).
pub fn read_tool_capped(
    store: &dyn RepositoryStore,
    repository_id: &str,
    arguments: Option<Map<String, Value>>,
    max_bytes: usize,
) -> Result<Value, McpApplicationError> {
    let input: ReadToolInput = parse_args(arguments)?;
    let result = crate::srs_resources::read_resource(store, repository_id, &input.uri)?;
    let part = &result["contents"][0];
    let mut text = part["text"].as_str().unwrap_or_default().to_string();
    let total = text.len();
    let truncated = total > max_bytes;
    let mut shown = total;
    if truncated {
        let cut = utf8_floor(&text, max_bytes);
        text.truncate(cut);
        shown = cut;
        text.push_str(&format!(
            "\n\n[truncated: showing {cut} of {total} bytes. Read a bounded part instead: find with limit/offset, \
srs://{repository_id}/tree/{{instanceId}}, container_outline, or srs://{repository_id}/record/{{instanceId}}.]"
        ));
    }
    Ok(json!({
        "content": [{ "type": "text", "text": &text }],
        "structuredContent": {
            "uri": input.uri,
            "text": text,
            "mimeType": part["mimeType"],
            "truncated": truncated,
            "totalBytes": total,
            "shownBytes": shown,
        },
        "isError": false
    }))
}

/// The `tools/call` result. Validated service rejections are tool results with
/// `isError: true`; only malformed calls are protocol (`McpApplicationError`) errors.
pub fn call_tool(
    store: &dyn RepositoryStore,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, McpApplicationError> {
    // RFC-046 [R4]/[R5]: the actor is host-set, never a tool argument. A `createdBy` in the
    // arguments of a write tool is lifted out before parsing (the input structs deny unknown
    // fields) so it gets the specified diagnostic: `actor-supplied` on a creating tool,
    // `actor-changed` (unless identical) on `record_update`.
    let mut arguments = arguments;
    let mut supplied_created_by = None;
    match name {
        TOOL_RECORD_CREATE
        | TOOL_RELATION_CREATE
        | TOOL_NOTE_CREATE
        | TOOL_RECORD_SUCCESSOR
        | TOOL_NOTE_GRADUATE => {
            if let Some(args) = &arguments {
                if let Err(e) =
                    srs_repository::actor_service::reject_supplied_created_by(store, args)
                {
                    return Ok(tool_err(e.report()));
                }
            }
        }
        TOOL_RECORD_UPDATE => {
            supplied_created_by = arguments
                .as_mut()
                .and_then(|a| a.remove(srs_repository::actor_service::CREATED_BY_KEY));
        }
        _ => {}
    }
    match name {
        TOOL_REPO_VALIDATE => {
            let _: EmptyToolInput = parse_args(arguments)?;
            match validate_repository(store) {
                Ok(report) => tool_ok(&report),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_FIND => {
            let input: FindToolInput = parse_args(arguments)?;
            let page = FindPage {
                limit: Some(input.limit.unwrap_or(FIND_DEFAULT_LIMIT)),
                offset: input.offset.unwrap_or(0),
                rank: input.rank.unwrap_or(true),
                match_mode: input.match_mode.unwrap_or_default(),
                by_type_limit: input.by_type_limit,
                facets: input.facets,
                projection: input.projection.unwrap_or_default(),
            };
            match discovery_service::find(store, input.into(), page) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_ATTACHMENT_ADD => {
            let input: AddAttachmentInput =
                parse_args::<AttachmentAddToolInput>(arguments)?.try_into()?;
            match attachment_service::add_attachment(store, input) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_ATTACHMENT_LINK => {
            let input: AttachmentLinkToolInput = parse_args(arguments)?;
            match attachment_service::link_attachment(store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_SIMILAR => {
            let (id, query, page) = parse_args::<SimilarToolInput>(arguments)?.into_parts();
            match discovery_service::similar(store, &id, query, page) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RECORD_CREATE => {
            let input: RecordCreateToolInput = parse_args(arguments)?;
            let type_filter = input.type_filter.clone();
            let type_version = input.type_version;
            let container_id = input.container_id.clone();
            match record_store::create_record_in_context(
                store,
                &type_filter,
                type_version,
                input.into(),
                container_id,
                None,
            ) {
                Ok(result) => tool_ok(&result.record),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RELATION_CREATE => {
            let input: RelationCreateToolInput = parse_args(arguments)?;
            match relation_service::create_relation_auto(store, input.into()) {
                Ok(result) => tool_ok(&result.relation),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RELATION_DELETE => {
            let input: RelationDeleteToolInput = parse_args(arguments)?;
            match relation_service::delete_relation(store, &input.relation_id) {
                Ok(r) => tool_ok(&json!({ "relationId": r.relation_id, "path": r.path })),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_NOTE_CREATE => {
            let input: NoteCreateToolInput = parse_args(arguments)?;
            match services::create_note_in_context(store, input.into()) {
                Ok(result) => tool_ok(&result.note),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_NOTE_UPDATE => {
            let input: NoteUpdateToolInput = parse_args(arguments)?;
            let id = input.instance_id.clone();
            match services::update_note_content(store, &id, input.into()) {
                Ok(result) => tool_ok(&result.note),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        // `read` is routed by `SrsMcpApplication` (it needs the repository id): see `read_tool`.
        TOOL_TYPE_SCHEMA => {
            let input: TypeSchemaToolInput = parse_args(arguments)?;
            match type_schema_service::type_schema(store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        // Second-wave write tools (#680)
        TOOL_RECORD_UPDATE => {
            let input: RecordUpdateToolInput = parse_args(arguments)?;
            let instance_id = input.instance_id.clone();
            let mut update: UpdateRecordInput = input.into();
            if let Some(v) = supplied_created_by {
                update
                    .extra
                    .insert(srs_repository::actor_service::CREATED_BY_KEY.to_string(), v);
            }
            match record_store::update_record(store, &instance_id, update) {
                Ok(record) => tool_ok(&record),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RECORD_TRANSITION => {
            let input: RecordTransitionToolInput = parse_args(arguments)?;
            let instance_id = input.instance_id.clone();
            match record_store::transition_record_lifecycle(store, &instance_id, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RECORD_ALLOWED_TRANSITIONS => {
            let input: RecordAllowedTransitionsToolInput = parse_args(arguments)?;
            match record_store::get_allowed_lifecycle_transitions(store, &input.instance_id) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RECORD_SUCCESSOR => {
            let input: RecordSuccessorToolInput = parse_args(arguments)?;
            let predecessor_id = input.predecessor_id.clone();
            match record_store::create_record_successor(store, &predecessor_id, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_NOTE_GRADUATE => {
            let input: NoteGraduateToolInput = parse_args(arguments)?;
            match services::graduate_note(store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_CREATE => {
            let input: ContainerCreateInput = parse_args(arguments)?;
            match container_service::create_container(store, input.into()) {
                Ok(container) => tool_ok(&container),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_MEMBER_ADD => {
            let input: ContainerMemberAddToolInput = parse_args(arguments)?;
            let relative = relative_move(
                input.relative_to.as_deref(),
                input.placement.as_deref(),
                None,
                input.position.is_some() || input.depth.is_some(),
            )?;
            let result = match relative {
                Some(RelativeMove::Place { target, placement }) => {
                    container_service::add_member_relative(
                        store,
                        &input.container_id,
                        &input.instance_id,
                        &target,
                        placement,
                    )
                }
                _ => container_service::add_member(
                    store,
                    &input.container_id,
                    &input.instance_id,
                    input.position,
                    input.depth,
                ),
            };
            match result {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PACKAGE_DEPENDENCY_LIST => {
            let input: PackageDependencyListToolInput = parse_args(arguments)?;
            match package_dependency_service::list_package_dependencies(store, input.selector) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PACKAGE_DEPENDENCY_SET => {
            let input: PackageDependencySetToolInput = parse_args(arguments)?;
            match package_dependency_service::add_package_dependency(store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PACKAGE_DEPENDENCY_REMOVE => {
            let input: PackageDependencyRemoveToolInput = parse_args(arguments)?;
            match package_dependency_service::remove_package_dependency(store, input.into()) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PACKAGE_UPGRADE => {
            let input: PackageUpgradeToolInput = parse_args(arguments)?;
            let options = UpgradeOptions {
                dry_run: input.dry_run,
                boundary_path: input.boundary_path,
                prior_bundles: input.prior_bundles,
                adopt: input.adopt,
            };
            match package_install_service::upgrade_package_bundle(
                store,
                input.bundle.as_bytes(),
                options,
            ) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_NEIGHBOURS => {
            let (query, page) = parse_args::<NeighboursToolInput>(arguments)?.into_parts();
            match list_neighbours(store, query, page) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_MEMBER_REMOVE => {
            let input: ContainerMemberToolInput = parse_args(arguments)?;
            match container_service::remove_member(store, &input.container_id, &input.instance_id) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_MEMBER_MOVE => {
            let input: ContainerMemberMoveToolInput = parse_args(arguments)?;
            let relative = relative_move(
                input.relative_to.as_deref(),
                input.placement.as_deref(),
                input.shift.as_deref(),
                input.position.is_some() || input.depth.is_some(),
            )?;
            let result = match relative {
                Some(mv) => container_service::move_member_relative(
                    store,
                    &input.container_id,
                    &input.instance_id,
                    &mv,
                ),
                None => container_service::move_member(
                    store,
                    &input.container_id,
                    &input.instance_id,
                    input.position,
                    input.depth,
                ),
            };
            match result {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_MEMBER_REPAIR => {
            let input: ContainerIdToolInput = parse_args(arguments)?;
            match container_service::repair_members(store, &input.container_id) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_OUTLINE => {
            let input: ContainerIdToolInput = parse_args(arguments)?;
            match container_service::get_outline(store, &input.container_id) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_CONTAINER_COPY => {
            let input: ContainerCopyToolInput = parse_args(arguments)?;
            let copy = container_service::ContainerCopyInput {
                title: input.title,
                container_id: input.container_id,
            };
            match container_service::copy_container(store, &input.source_container_id, copy) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_RECORD_FORK => {
            let input: RecordForkToolInput = parse_args(arguments)?;
            let opts = srs_repository::fork_service::ForkOptions {
                target_container: input.target_container_id.clone(),
                carry_relations: input.carry_relations,
            };
            match srs_repository::fork_service::fork_subtree(
                store,
                input.container_id.as_deref(),
                &input.instance_id,
                &opts,
            ) {
                Ok(result) => tool_ok(&result),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        // Protocol run execution tools (#977)
        TOOL_PROTOCOL_RUN_CREATE => {
            let input: ProtocolRunCreateToolInput = parse_args(arguments)?;
            match protocol_run_service::create_run(store, input.into()) {
                Ok(result) => tool_ok(&result.run),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PROTOCOL_RUN_ADVANCE => {
            let input: ProtocolRunAdvanceToolInput = parse_args(arguments)?;
            match protocol_run_service::advance_stage(store, input.into()) {
                Ok(result) => tool_ok(&result.run),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PROTOCOL_RUN_GET => {
            let input: ProtocolRunIdToolInput = parse_args(arguments)?;
            match protocol_run_service::get_run(store, &input.run_id) {
                Ok(GetRunResult::Found(run)) => tool_ok(&*run),
                Ok(GetRunResult::NotFound) => Ok(tool_err(
                    srs_repository::error::RepositoryError::RunNotFound {
                        run_id: input.run_id,
                    }
                    .report(),
                )),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PROTOCOL_RUN_LIST => {
            let input: ProtocolRunListToolInput = parse_args(arguments)?;
            match protocol_run_service::list_runs(store, input.into()) {
                Ok(runs) => tool_ok(&ProtocolRunListToolResult { runs }),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PROTOCOL_RUN_COMPLETE => {
            let input: ProtocolRunIdToolInput = parse_args(arguments)?;
            match protocol_run_service::complete_run(store, &input.run_id) {
                Ok(result) => tool_ok(&result.run),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        TOOL_PROTOCOL_RUN_ABANDON => {
            let input: ProtocolRunIdToolInput = parse_args(arguments)?;
            match protocol_run_service::abandon_run(store, &input.run_id) {
                Ok(result) => tool_ok(&result.run),
                Err(e) => Ok(tool_err(e.report())),
            }
        }
        other => Err(McpApplicationError::invalid_params(format!(
            "unknown tool '{other}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift guard: populate EVERY field of each shadow input and assert the
    /// conversion carries all of them into the service type (plan review AR-1).
    #[test]
    fn package_dependency_tool_inputs_convert_every_field() {
        let set: AddPackageDependencyInput = PackageDependencySetToolInput {
            selector: Some("packages/a".into()),
            package_id: "pid".into(),
            version: "1.2.0".into(),
            repair_legacy: true,
        }
        .into();
        assert!(set.repair_legacy);
        assert_eq!(set.selector.as_deref(), Some("packages/a"));
        assert_eq!(set.package_id, "pid");
        assert_eq!(set.version, "1.2.0");
        let rm: RemovePackageDependencyInput = PackageDependencyRemoveToolInput {
            selector: None,
            package_id: "pid".into(),
        }
        .into();
        assert_eq!(rm.selector, None);
        assert_eq!(rm.package_id, "pid");
    }

    #[test]
    fn neighbours_input_conversion_exercises_every_field_and_bounds_limit() {
        let input = |limit| NeighboursToolInput {
            instance_id: "iid".into(),
            relation_type: Some("contains".into()),
            direction: Some(EdgeDirection::In),
            limit,
            offset: Some(7),
        };
        let (q, p) = input(Some(10)).into_parts();
        assert_eq!(q.instance_id, "iid");
        assert_eq!(q.relation_type.as_deref(), Some("contains"));
        assert_eq!(q.direction, Some(EdgeDirection::In));
        assert_eq!((p.limit, p.offset), (Some(10), 7));
        assert_eq!(
            input(None).into_parts().1.limit,
            Some(NEIGHBOURS_DEFAULT_LIMIT)
        );
        assert_eq!(
            input(Some(100_000)).into_parts().1.limit,
            Some(NEIGHBOURS_MAX_LIMIT)
        );
    }

    #[test]
    fn find_input_reads_match_mode_under_the_json_key_match() {
        let any: FindToolInput =
            serde_json::from_value(json!({"contentMatch": "x", "match": "any"})).unwrap();
        assert_eq!(any.match_mode, Some(MatchMode::Any));
        let absent: FindToolInput = serde_json::from_value(json!({"contentMatch": "x"})).unwrap();
        assert_eq!(absent.match_mode, None);
        assert!(serde_json::from_value::<FindToolInput>(json!({"match": "some"})).is_err());
    }

    #[test]
    fn find_input_parses_projection_and_facets() {
        let input: FindToolInput =
            serde_json::from_value(json!({"projection": "label", "facets": false})).unwrap();
        assert_eq!(input.projection, Some(Projection::Label));
        assert_eq!(input.facets, Some(false));
        assert!(serde_json::from_value::<FindToolInput>(json!({"projection": "tiny"})).is_err());
        let similar: SimilarToolInput =
            serde_json::from_value(json!({"instanceId": "i", "projection": "card"})).unwrap();
        let page = similar.into_parts().2;
        assert_eq!((page.projection, page.facets), (Projection::Card, None));
    }

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
            limit: None,
            by_type_limit: None,
            offset: None,
            rank: None,
            match_mode: Some(MatchMode::Any),
            facets: Some(true),
            projection: Some(Projection::Card),
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
            lifecycle_state: Some("proposed".into()),
            meta: Some(serde_json::json!({"derivedFrom": "src-id"})),
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
        assert_eq!(ci.lifecycle_state.as_deref(), Some("proposed"));
        assert_eq!(
            ci.extra.get("meta"),
            Some(&serde_json::json!({"derivedFrom": "src-id"}))
        );

        // RelationCreate → Relation
        let rel = RelationCreateToolInput {
            relation_id: Some("rid".into()),
            relation_type: "depends-on".into(),
            source_instance_id: "s".into(),
            target_instance_id: "t".into(),
            created_at: Some("now".into()),
            notes: Some("n".into()),
            meta: Some(serde_json::Value::Bool(true)),
        };
        let r: Relation = rel.into();
        assert_eq!(r.relation_id, "rid");
        assert_eq!(r.relation_type, "depends-on");
        assert_eq!(r.source_instance_id, "s");
        assert_eq!(r.target_instance_id, "t");
        assert_eq!(r.created_at.as_deref(), Some("now"));
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
    fn attachment_tool_inputs_convert_every_field() {
        let add: AttachmentAddToolInput = serde_json::from_value(json!({
            "fileName": "f.bin", "contentBase64": "AAEC", "title": "T",
            "subdir": "s", "contentType": "application/x-test"
        }))
        .unwrap();
        let add = AddAttachmentInput::try_from(add).unwrap();
        assert_eq!(add.file_name, "f.bin");
        assert_eq!(add.content, vec![0, 1, 2]);
        assert_eq!(add.title.as_deref(), Some("T"));
        assert_eq!(add.subdir.as_deref(), Some("s"));
        assert_eq!(add.content_type.as_deref(), Some("application/x-test"));
        assert!(add.enforce_policy, "agent writes always enforce the policy");
        let link: AttachmentLinkToolInput =
            serde_json::from_value(json!({ "instanceId": "i", "documentId": "d" })).unwrap();
        let link = LinkAttachmentInput::from(link);
        assert_eq!(
            (link.instance_id.as_str(), link.document_id.as_str()),
            ("i", "d")
        );
    }

    #[test]
    fn utf8_floor_never_splits_a_char() {
        let s = "aé€😀"; // 1 + 2 + 3 + 4 bytes
        let cuts: Vec<usize> = (0..=s.len()).map(|n| utf8_floor(s, n)).collect();
        assert_eq!(cuts, vec![0, 1, 1, 3, 3, 3, 6, 6, 6, 6, 10]);
        assert_eq!(utf8_floor(s, 99), 10);
    }

    #[test]
    fn tool_ok_text_is_compact_json_equal_to_structured_content() {
        let r = tool_ok(&json!({"hits": [{"id": "a", "tags": ["x", "y"]}], "total": 1})).unwrap();
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains('\n') && !text.contains(": "), "{text}");
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed, r["structuredContent"]);
    }

    #[test]
    fn tool_profiles_are_subsets_of_the_catalogue() {
        let names = |p: ToolProfile| -> Vec<String> {
            list_tools_for(p)["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["name"].as_str().unwrap().to_string())
                .collect()
        };
        let full = names(ToolProfile::Full);
        assert_eq!(full.len(), all_tools().len());
        // Every profile entry names a real tool (a typo would silently drop it).
        for listed in [CONTEXT_TOOLS, READ_TOOLS] {
            for n in listed {
                assert!(full.iter().any(|f| f == n), "{n} is not a tool");
            }
        }
        assert_eq!(names(ToolProfile::Context).len(), CONTEXT_TOOLS.len());
        assert_eq!(names(ToolProfile::Read).len(), READ_TOOLS.len());
        // The read profile carries nothing that writes.
        const WRITE_VERBS: &[&str] = &[
            "create",
            "update",
            "delete",
            "add",
            "remove",
            "move",
            "repair",
            "copy",
            "fork",
            "transition",
            "successor",
            "graduate",
            "advance",
            "complete",
            "abandon",
            "set",
            "upgrade",
        ];
        for n in READ_TOOLS {
            assert!(
                *n == TOOL_RECORD_ALLOWED_TRANSITIONS
                    || !WRITE_VERBS.iter().any(|v| n.ends_with(v)),
                "{n} looks like a write"
            );
        }
        for p in ["full", "context", "read"] {
            assert_eq!(p.parse::<ToolProfile>().unwrap().as_str(), p);
        }
        assert!("all".parse::<ToolProfile>().is_err());
    }

    #[test]
    fn list_tools_advertises_every_tool_with_schemas() {
        let tools = list_tools()["tools"].as_array().unwrap().clone();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec![
                TOOL_REPO_VALIDATE,
                TOOL_FIND,
                TOOL_RECORD_CREATE,
                TOOL_RELATION_CREATE,
                TOOL_RELATION_DELETE,
                TOOL_NOTE_CREATE,
                TOOL_NOTE_UPDATE,
                TOOL_TYPE_SCHEMA,
                TOOL_READ,
                TOOL_RECORD_UPDATE,
                TOOL_RECORD_TRANSITION,
                TOOL_RECORD_ALLOWED_TRANSITIONS,
                TOOL_RECORD_SUCCESSOR,
                TOOL_NOTE_GRADUATE,
                TOOL_CONTAINER_CREATE,
                TOOL_CONTAINER_MEMBER_ADD,
                TOOL_CONTAINER_MEMBER_REMOVE,
                TOOL_CONTAINER_MEMBER_MOVE,
                TOOL_CONTAINER_MEMBER_REPAIR,
                TOOL_CONTAINER_OUTLINE,
                TOOL_CONTAINER_COPY,
                TOOL_RECORD_FORK,
                TOOL_PROTOCOL_RUN_CREATE,
                TOOL_PROTOCOL_RUN_ADVANCE,
                TOOL_PROTOCOL_RUN_GET,
                TOOL_PROTOCOL_RUN_LIST,
                TOOL_PROTOCOL_RUN_COMPLETE,
                TOOL_PROTOCOL_RUN_ABANDON,
                TOOL_PACKAGE_DEPENDENCY_LIST,
                TOOL_PACKAGE_DEPENDENCY_SET,
                TOOL_PACKAGE_DEPENDENCY_REMOVE,
                TOOL_PACKAGE_UPGRADE,
                TOOL_NEIGHBOURS,
                TOOL_SIMILAR,
                TOOL_ATTACHMENT_ADD,
                TOOL_ATTACHMENT_LINK,
            ]
        );
        for tool in &tools {
            assert!(tool["description"].is_string());
            assert!(
                !tool["inputSchema"].as_object().unwrap().is_empty(),
                "tool {} has an empty input schema",
                tool["name"]
            );
        }
    }

    #[test]
    fn arrangement_tool_descriptions_state_outline_semantics() {
        let tools = list_tools()["tools"].as_array().unwrap().clone();
        let description = |name: &str| -> String {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .and_then(|tool| tool["description"].as_str())
                .expect("arrangement tool must advertise a description")
                .to_string()
        };
        // Order and depth are layout, never a semantic claim (RFC-043 [R5], [R13]).
        let add = description(TOOL_CONTAINER_MEMBER_ADD);
        assert!(add.contains("layout only"));
        assert!(add.contains("precedes relation when order is a semantic claim"));
        assert!(add.contains("appends at depth 0"));
        let remove = description(TOOL_CONTAINER_MEMBER_REMOVE);
        assert!(remove.contains("promoted"));
        assert!(remove.contains("arrangement-pointer"));
        assert!(
            description(TOOL_CONTAINER_MEMBER_MOVE).contains("whole run")
                || description(TOOL_CONTAINER_MEMBER_MOVE).contains("with its descendants")
        );
        assert!(description(TOOL_CONTAINER_MEMBER_REPAIR).contains("Idempotent"));
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
            meta: Some(serde_json::json!({"derivedFrom": "src-id"})),
        };
        assert_eq!(upd.instance_id, "iid");
        let ui: UpdateRecordInput = upd.into();
        assert_eq!(ui.field_values.get("title"), Some(&serde_json::json!("v1")));
        let meta = &ui.field_meta.as_ref().unwrap()["title"];
        assert_eq!(meta.source.as_deref(), Some("src"));
        assert_eq!(meta.edited_at.as_deref(), Some("t"));
        assert_eq!(ui.tags, Some(vec!["tag1".to_string()]));
        assert_eq!(ui.type_version, Some(2));
        assert_eq!(
            ui.extra.get("meta"),
            Some(&serde_json::json!({"derivedFrom": "src-id"}))
        );

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
            relation_type: Some("supersedes".into()),
            field_values: [("f3".to_string(), serde_json::json!("v3"))]
                .into_iter()
                .collect(),
            lifecycle_state: Some("draft".into()),
            type_version: Some(5),
            meta: Some(serde_json::json!({"derivedFrom": "pid"})),
        };
        assert_eq!(succ.predecessor_id, "pid");
        let si: CreateRecordSuccessorInput = succ.into();
        assert_eq!(si.relation_type.as_deref(), Some("supersedes"));
        assert_eq!(si.field_values.get("f3"), Some(&serde_json::json!("v3")));
        assert_eq!(si.lifecycle_state.as_deref(), Some("draft"));
        assert_eq!(si.type_version, Some(5));
        assert_eq!(
            si.extra.get("meta"),
            Some(&serde_json::json!({"derivedFrom": "pid"}))
        );

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

    /// Drift guard for the protocol run tools (#977): populate EVERY field of
    /// each shadow input and assert the conversion carries all of them into
    /// the service type.
    #[test]
    fn tool_input_conversion_protocol_run_exercises_every_field() {
        // ProtocolRunCreateToolInput → CreateRunInput
        let create = ProtocolRunCreateToolInput {
            protocol_id: "proto-1".into(),
            protocol_version: 3,
            container_id: "cont-1".into(),
            target_record_id: Some("rec-1".into()),
            initial_stage_id: Some("stage-a".into()),
        };
        let ci: CreateRunInput = create.into();
        assert_eq!(ci.protocol_id, "proto-1");
        assert_eq!(ci.protocol_version, 3);
        assert_eq!(ci.container_id, "cont-1");
        assert_eq!(ci.target_record_id.as_deref(), Some("rec-1"));
        assert_eq!(ci.initial_stage_id.as_deref(), Some("stage-a"));

        // ProtocolRunAdvanceToolInput → AdvanceStageInput
        let advance = ProtocolRunAdvanceToolInput {
            run_id: "run-1".into(),
            stage_id: "stage-b".into(),
            complete_current: true,
        };
        let ai: AdvanceStageInput = advance.into();
        assert_eq!(ai.run_id, "run-1");
        assert_eq!(ai.stage_id, "stage-b");
        assert!(ai.complete_current);

        // ProtocolRunIdToolInput (no conversion — field passed directly)
        let id = ProtocolRunIdToolInput {
            run_id: "run-2".into(),
        };
        assert_eq!(id.run_id, "run-2");

        // ProtocolRunListToolInput → RunListFilter
        let list = ProtocolRunListToolInput {
            protocol_id: Some("proto-2".into()),
            container_id: Some("cont-2".into()),
            status: Some("Active".into()),
        };
        let rlf: RunListFilter = list.into();
        assert_eq!(rlf.protocol_id.as_deref(), Some("proto-2"));
        assert_eq!(rlf.container_id.as_deref(), Some("cont-2"));
        assert_eq!(rlf.status.as_deref(), Some("Active"));
    }
}
