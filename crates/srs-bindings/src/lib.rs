use serde::{Deserialize, Serialize};
use srs_core::arrangement::RelativeMove;
use srs_core::types::record::{FieldMeta, FieldValues};
use srs_repository::attachment_service::{
    self as attachment_service, AddAttachmentInput, GetAttachmentBytesInput,
    GetRecordAttachmentsInput, LinkAttachmentInput, ListAttachmentsFilter,
    ResolveCompositionAttachmentsInput,
};
use srs_repository::blueprint_schema_service::{self, BlueprintSchemaInput};
use srs_repository::blueprint_service;
use srs_repository::container_service::{self, ContainerListFilter};
use srs_repository::container_view_service::{self, ResolveContainerViewInput};
use srs_repository::context_query_service::{
    self, EdgeDirection, FieldContextQuery, NeighboursPage, NeighboursQuery, RecordContextQuery,
};
use srs_repository::discovery_service::{self, DiscoveryQuery, FindPage, MatchMode};
use srs_repository::doctor_service::{self, DoctorInput};
use srs_repository::governance_scaffold_service::{self, CreateGovernanceRepositoryInput};
use srs_repository::manifest_service;
use srs_repository::migrate_identity_service;
use srs_repository::migration_registry_service;
use srs_repository::package_bundle;
use srs_repository::package_dependency_service;
use srs_repository::package_install_service;
use srs_repository::package_service::{
    self, FieldListFilter, GetFieldResult, GetTypeResult, ListPackageImportsFilter,
    RelationTypeListFilter, TypeListFilter,
};
use srs_repository::projection::Projection;
use srs_repository::protocol_run_service::{
    self as run_service, AdvanceStageInput as RunAdvanceInput, CreateRunInput as RunCreateInput,
    GetRunResult, RunListFilter,
};
use srs_repository::protocol_service::{self, GetProtocolResult};
use srs_repository::record_store::{
    self, CreateRecordInput, RecordListFilter, TransitionLifecycleInput,
};
use srs_repository::registry_service::{
    filter_registry_entries, parse_registry_json, RegistryListFilter,
};
use srs_repository::relation_service::{
    self, InsertIntoPrecedesChainInput, ListRelationsFilter, MoveInPrecedesChainInput,
    OrderByPrecedesInput, RebuildPrecedesChainInput, RemoveFromPrecedesChainInput,
};
use srs_repository::render_service::{self, RenderCompositionOptions};
use srs_repository::repository_lifecycle::{self, InitNewRepositoryInput};
use srs_repository::repository_navigation_service;
use srs_repository::services::{
    self, graduate_note as graduate_note_service, GraduateNoteInput, ListNotesFilter,
};
use srs_repository::store::RepositoryStore as _;
use srs_repository::tag_service;
use srs_repository::type_schema_service::{self, TypeSchemaInput};
use srs_repository::validation;
use srs_repository::view_service::{self, CompositionListFilter, GetViewResult};
use srs_repository::FileStore;
use wasm_bindgen::prelude::*;

/// Serialise `value` to a JSON string via serde_json (which respects all serde attributes
/// including `rename_all` and `flatten`), then parse it as a JS value via the browser's
/// native JSON.parse. This is more reliable than serde_wasm_bindgen::to_value for structs
/// that use #[serde(flatten)] or complex serde transformations.
fn to_js<T: serde::Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let json = serde_json::to_string(value).map_err(|e| js_err(e.to_string()))?;
    js_sys::JSON::parse(&json).map_err(|e| js_err(format!("{e:?}")))
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// A trailing optional `projection` argument (srs-rust#1286): `full` when absent.
fn parse_projection(p: Option<String>) -> Result<Projection, JsValue> {
    p.map_or(Ok(Projection::default()), |p| p.parse().map_err(js_err))
}

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

/// The JS-facing shape of an RFC-035 projection: the structured schema, the
/// canonical bytes, and what could not be expressed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSchemaBindingResult {
    schema: serde_json::Value,
    canonical_json: String,
    inexpressible: Vec<String>,
}

#[wasm_bindgen]
pub struct SrsRepository {
    store: FileStore,
}

/// A browser-hosted MCP session over an already-open repository (srs-rust#1057).
///
/// Wraps the transport-agnostic `srs-mcp-core` dispatcher over a clone of the
/// repository's store. Clones share the in-memory VFS (and catalog-cache
/// validity), so MCP writes are visible to this repository handle's reads and
/// to `export_*`; nothing is persisted until the host explicitly saves/exports.
/// Transport (Streamable HTTP framing, auth, origin policy) is the host's job.
#[wasm_bindgen]
pub struct McpSession {
    dispatcher: srs_mcp_core::McpDispatcher<srs_mcp_core::SrsMcpApplication<FileStore>>,
    store: FileStore,
}

#[wasm_bindgen]
impl McpSession {
    /// Handle one JSON-RPC message (text). Returns the JSON-RPC response text,
    /// or `undefined` for a notification (host answers 202 / no body).
    pub fn handle(&mut self, message: &str) -> Option<String> {
        self.dispatcher.dispatch_str(message)
    }

    /// Write generation of the underlying repository (shared with the
    /// `SrsRepository` handle). Compare before/after `handle` to learn whether
    /// a request mutated it: reads and rejected writes leave it unchanged.
    /// Exposed as `f64` (exact below 2^53) because JS numbers are doubles.
    pub fn write_epoch(&self) -> f64 {
        self.store.write_epoch() as f64
    }

    /// What the last `handle` wrote, once (srs-rust#1202, ADR-049), as JSON:
    /// `{"tool":"record_update","changed":[{"target":"instance|relation|container","id":"..","kind":"created|updated|deleted"}]}`.
    /// `undefined` when the request wrote nothing (reads, guard rejections). Recorded by
    /// the store at its write seam; the MCP response is unchanged. A container member
    /// change is `updated`; the manifest's root container is not reported.
    pub fn take_write_summary(&mut self) -> Option<String> {
        let summary = self.dispatcher.application_mut().take_write_summary()?;
        serde_json::to_string(&summary).ok()
    }

    /// Install a write guard for this session, replacing any previous one
    /// (srs-rust#1165): `{"containerIds":[..],"instanceIds":[..],"fillOnlyFields":[..]}`,
    /// every key optional. Guarded records are read-only to MCP writes except
    /// `fillOnlyFields` while unset; rejections are tool errors. The UI's own
    /// writes through `SrsRepository` are unaffected.
    pub fn set_write_guard(&mut self, json: &str) -> Result<(), JsValue> {
        let guard: srs_mcp_core::guard::WriteGuard =
            serde_json::from_str(json).map_err(|e| js_err(e.to_string()))?;
        self.dispatcher
            .application_mut()
            .set_write_guard(Some(guard));
        Ok(())
    }

    /// Set the session actor (RFC-046) stamped as `createdBy` on everything this MCP
    /// session creates: `{"kind":"human|ai","id":"<non-empty>","name":"<optional>"}`.
    /// The host fixes `kind` and `id`; omit `name` and the session fills it from the MCP
    /// `initialize` `clientInfo.name` (control chars stripped, trimmed, capped at 120 chars; none if empty). That
    /// handle is client-supplied and display-only (RFC-046 `Actor.name` is a hint, never
    /// identity); a non-empty `name` the host sets wins (null/"" do not count), and the client can never change `kind`/`id`.
    /// Call before `initialize`; a later `set_actor` replaces the actor, handle included.
    /// Host-supplied only — never from tool arguments. The JSON is not validated here:
    /// an invalid actor refuses every creating tool call with `actor-invalid` (R12),
    /// and an actor on a corpus below dataModelRevision 9 with `revision-too-old` (R11).
    pub fn set_actor(&mut self, json: &str) -> Result<(), JsValue> {
        let actor: serde_json::Value =
            serde_json::from_str(json).map_err(|e| js_err(e.to_string()))?;
        self.dispatcher
            .application_mut()
            .set_session_actor(Some(actor));
        Ok(())
    }

    /// Clear the session actor: subsequent creations are unattributed.
    pub fn clear_actor(&mut self) {
        self.dispatcher.application_mut().set_session_actor(None);
    }

    /// Remove the session's write guard.
    pub fn clear_write_guard(&mut self) {
        self.dispatcher.application_mut().set_write_guard(None);
    }

    /// Whether the client has completed `initialize`.
    pub fn is_initialized(&self) -> bool {
        self.dispatcher.is_initialized()
    }
}

#[wasm_bindgen]
impl SrsRepository {
    /// Write generation of this repository: advances on every successful mutation (UI
    /// service calls and MCP writes alike), never on reads. The same store counter
    /// `McpSession::write_epoch` reads. `f64` because JS numbers are doubles (exact below 2^53).
    pub fn write_epoch(&self) -> f64 {
        self.store.write_epoch() as f64
    }

    /// Set the session actor (RFC-046) stamped as `createdBy` on everything created through
    /// this handle (UI writes), as `{"kind":"human|ai","id":"<non-empty>","name":"<optional>"}`.
    /// Independent of any MCP session's actor. Not validated here (see
    /// `McpSession::set_actor`).
    pub fn set_actor(&self, json: &str) -> Result<(), JsValue> {
        let actor: serde_json::Value =
            serde_json::from_str(json).map_err(|e| js_err(e.to_string()))?;
        self.store.set_session_actor(Some(actor));
        Ok(())
    }

    /// Clear this handle's session actor: subsequent creations are unattributed.
    pub fn clear_actor(&self) {
        self.store.set_session_actor(None);
    }

    /// Open an MCP session over this repository (resources, prompts, and the
    /// validated tool surface — the same application `srs mcp serve` runs).
    pub fn open_mcp_session(&self) -> Result<McpSession, JsValue> {
        let application = srs_mcp_core::SrsMcpApplication::open(self.store.clone())
            .map_err(|e| js_err(e.message))?;
        Ok(McpSession {
            store: self.store.clone(),
            dispatcher: srs_mcp_core::McpDispatcher::new(application),
        })
    }

    /// Load a repository from a `.srsj` JSON string.
    ///
    /// `.srsj` is a boundary codec (ADR-038, RFC-038 [R19]): the envelope is
    /// decoded into a file tree and opened as the same operational store the
    /// CLI runs on disk. Only `srsj: "2"` is accepted — an unrecognised
    /// version is refused rather than coerced ([R20]/[R21]).
    pub fn load(srsj: &str) -> Result<SrsRepository, JsValue> {
        let store = srs_repository::srsj::open_srsj(srsj).map_err(js_err)?;
        Ok(SrsRepository { store })
    }

    /// Load a repository for the migration tooling surface only.
    ///
    /// RFC-038 [R21]: a conforming reader rejects pre-generation-2 data; a
    /// migration tool operating under an explicit opt-in is the one exempt
    /// reader. This is that opt-in for the WASM client's repo-upgrade flow —
    /// it skips the [R2]/[R21] manifest checks so `available_migrations` and
    /// `apply_migration` can inspect and transform an old document. Every
    /// other caller uses [`SrsRepository::load`].
    pub fn load_for_migration(srsj: &str) -> Result<SrsRepository, JsValue> {
        let store = srs_repository::srsj::open_srsj(srsj)
            .map_err(js_err)?
            .with_rfc038_exemption();
        Ok(SrsRepository { store })
    }

    /// Load a repository from a `.srs` binary archive (ZIP bytes).
    ///
    /// Native tree archives (ADR-039) load layout-faithfully; legacy snapshot
    /// archives take the migration ramp and are re-saved in the new format on
    /// the next export.
    pub fn load_archive(bytes: &[u8]) -> Result<SrsRepository, JsValue> {
        let store = srs_repository::archive_to_tree(std::io::Cursor::new(bytes)).map_err(js_err)?;
        Ok(SrsRepository { store })
    }

    /// Create a blank repository in memory (no seed): the same core service as
    /// `srs repo create`. `input_json` is `CreateBlankRepositoryInput`:
    /// `{ "namespace"?: string, "title"?, "description"?, "repositoryId"?,
    /// "srsVersion"?, "packageId"?, "packageName"?, "packageVersion"?,
    /// "packageNamespace"? }` — defaults are the core's. `namespace` is
    /// optional: when omitted it is derived as `com.example.<slug>` from
    /// `title`; one of the two is required. Export with
    /// `export_srsj()` / `export_tree()` / `export_archive()`.
    pub fn create(input_json: &str) -> Result<SrsRepository, JsValue> {
        Ok(SrsRepository {
            store: create_blank_from_json(input_json).map_err(js_err)?,
        })
    }

    /// Load a repository from an exploded file tree (ADR-038).
    ///
    /// `files` is a JS object mapping repo-relative forward-slash paths to
    /// `Uint8Array` contents — e.g. every blob of a fetched git tree. Unknown
    /// files (README, CI config) ride along untouched and reappear verbatim in
    /// [`SrsRepository::export_tree`].
    pub fn load_tree(files: JsValue) -> Result<SrsRepository, JsValue> {
        let obj: js_sys::Object = files
            .dyn_into()
            .map_err(|_| js_err("load_tree expects an object of { path: Uint8Array }"))?;
        let mut map = std::collections::BTreeMap::new();
        for key in js_sys::Object::keys(&obj).iter() {
            let path = key
                .as_string()
                .ok_or_else(|| js_err("load_tree keys must be strings"))?;
            let value = js_sys::Reflect::get(&obj, &key).map_err(|_| js_err("bad tree entry"))?;
            let bytes: js_sys::Uint8Array = value
                .dyn_into()
                .map_err(|_| js_err(format!("load_tree entry '{path}' must be a Uint8Array")))?;
            map.insert(path, bytes.to_vec());
        }
        let store = srs_repository::open_tree(map).map_err(js_err)?;
        Ok(SrsRepository { store })
    }

    /// Validate the repository. Returns a `RepositoryValidationReport` as a JS value with two
    /// top-level keys:
    ///
    /// - `diagnostics`: array of `{ severity, path, schemaId, message }` objects; `severity` is
    ///   `"error"`, `"warning"` or `"info"`. Entries with `severity: "warning"` (and `"info"`,
    ///   counted in neither summary total) are non-blocking advisories; they do not affect `summary.errors`
    ///   and the repository still passes validation when they are present. RFC-017 I-107
    ///   attachment size-limit violations (emitted when a `com.semanticops.base/repo_settings`
    ///   record specifies `max_per_file_bytes`) are one example of a warning source.
    /// - `summary`: `{ checked, errors, warnings }`. `summary.warnings` counts warning-severity
    ///   diagnostics. A repository passes validation when `summary.errors === 0`, even if
    ///   `summary.warnings > 0`.
    ///
    /// Callers should filter `diagnostics` by `severity` to distinguish errors from warnings.
    pub fn validate(&self) -> Result<JsValue, JsValue> {
        let report = validation::validate_repository(&self.store).map_err(js_err)?;
        to_js(&report)
    }

    /// `repo doctor` (srs-rust#857): detect and, when `fix` is true, repair
    /// damage from raw file adds and manual edits (duplicate instance ids,
    /// dangling container/relation references, relation filename/id
    /// mismatches, retired manifest keys). Dry-run by default (`fix: false`)
    /// — this never runs implicitly on any other call.
    ///
    /// Returns a `DoctorReport` as a JS value: `fixApplied`, `findings`
    /// (each `{ class, locators, message, outcome, detail }`), `repaired`,
    /// `remaining`. `class` and `outcome` are kebab-case strings, e.g.
    /// `"duplicate-instance-id"` / `"repaired"`.
    pub fn doctor(&self, fix: bool) -> Result<JsValue, JsValue> {
        let report = doctor_service::doctor(&self.store, DoctorInput { fix }).map_err(js_err)?;
        to_js(&report)
    }

    /// Return a conformance report comparing the manifest's `declaredExtensions` against the
    /// implementation's supported set and detected content usage.
    /// Returns a `DeclaredExtensionsReport` as a JS value with four camelCase keys:
    /// `declared`, `supported`, `declaredButUnsupported`, `usedButUndeclared`.
    pub fn declared_extensions_conformance(&self) -> Result<JsValue, JsValue> {
        let report =
            manifest_service::declared_extensions_conformance(&self.store).map_err(js_err)?;
        to_js(&report)
    }

    /// List records. `filter_json` is a JSON string matching `RecordListFilter`
    /// (`{"typeNamespace":"...","typeName":"...","containerId":"..."}`); pass `"{}"` for all records.
    /// Returns a JS array of `RecordSummary` objects — each `{ instanceId, displayLabel, record }`,
    /// where `displayLabel` is the core-resolved label (same resolution `srs tree` uses) and
    /// `record` is the full `Record`. Clients render `displayLabel` directly and must not
    /// re-derive titles from `fieldValues`.
    pub fn list_records(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RecordListFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let summaries = record_store::list_record_summaries(&self.store, filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Run a discovery query against the repository.
    /// `query_json` is a JSON object matching `DiscoveryQuery` (camelCase fields;
    /// all optional — omit or pass `"{}"` for "return all").
    /// `limit` (default: all matches) and `offset` (default 0) page the hits after the
    /// deterministic sort; `total` is the full match count. `rank` (default false) orders
    /// content-match hits by BM25 relevance and fills `score`. `by_type_limit` caps
    /// `facets.byType` values (default 20; 0 = every type). `match_mode` is `"all"`
    /// (default: every content word must occur) or `"any"` (any significant word, as a whole
    /// token, for natural-language queries; always ranked) — srs-rust#1284. `facets` returns
    /// the facets (default: only when `limit` is 0); `projection` is `"full"` (default),
    /// `"card"` or `"label"` — srs-rust#1286. All are trailing and optional, so existing
    /// callers are unchanged.
    /// Returns a `DiscoveryResult` as a JS value.
    #[allow(clippy::too_many_arguments)]
    pub fn find(
        &self,
        query_json: &str,
        limit: Option<usize>,
        offset: Option<usize>,
        rank: Option<bool>,
        by_type_limit: Option<usize>,
        match_mode: Option<String>,
        facets: Option<bool>,
        projection: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let query: DiscoveryQuery =
            serde_json::from_str(query_json).map_err(|e| js_err(format!("invalid query: {e}")))?;
        let match_mode: MatchMode = match match_mode {
            None => MatchMode::default(),
            Some(m) => m.parse().map_err(js_err)?,
        };
        let page = FindPage {
            limit,
            offset: offset.unwrap_or(0),
            rank: rank.unwrap_or(false),
            match_mode,
            by_type_limit,
            facets,
            projection: parse_projection(projection)?,
        };
        let result = discovery_service::find(&self.store, query, page).map_err(js_err)?;
        to_js(&result)
    }

    /// "More like this": instances similar to `instance_id`, ranked by BM25 over its top-weighted
    /// terms, excluding itself. `query_json` carries the structured `DiscoveryQuery` filters (no
    /// `contentMatch`); `limit`/`offset` page the hits; `projection` is as in [`Self::find`].
    /// Returns a `DiscoveryResult` as a JS value.
    pub fn find_similar(
        &self,
        instance_id: &str,
        query_json: &str,
        limit: Option<usize>,
        offset: Option<usize>,
        projection: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let query: DiscoveryQuery =
            serde_json::from_str(query_json).map_err(|e| js_err(format!("invalid query: {e}")))?;
        let page = FindPage {
            limit,
            offset: offset.unwrap_or(0),
            rank: true,
            projection: parse_projection(projection)?,
            ..Default::default()
        };
        let result =
            discovery_service::similar(&self.store, instance_id, query, page).map_err(js_err)?;
        to_js(&result)
    }

    /// Get a single record by instance ID. Returns a `RecordSummary` (`{ instanceId, displayLabel, record }`)
    /// as a JS value, or `null` if not found.
    pub fn get_record(&self, id: &str) -> Result<JsValue, JsValue> {
        match record_store::get_record_summary_by_id(&self.store, id).map_err(js_err)? {
            Some(summary) => to_js(&summary),
            None => Ok(JsValue::NULL),
        }
    }

    /// List notes. Returns a `ListNotesResult` as a JS value.
    pub fn list_notes(&self) -> Result<JsValue, JsValue> {
        let result =
            services::list_notes(&self.store, ListNotesFilter::default()).map_err(js_err)?;
        to_js(&result)
    }

    /// Graduate a Note to a typed Record in one atomic step.
    ///
    /// `input_json` is a `CreateRecordInput` JSON object
    /// (`fieldValues` — an object keyed by `Field.name`, RFC-039 carrier —
    /// `fieldMeta?`, `tags?`). Returns `{ note, record }`; a
    /// `derived-from` Relation (record -> note) is asserted atomically as the
    /// sole graduation-provenance record — `note.graduatedAt` is never stamped.
    /// `container_id` is optional; when supplied, the new Record is added to
    /// that container atomically.
    #[wasm_bindgen]
    pub fn graduate_note(
        &self,
        note_id: &str,
        type_ref: &str,
        type_version: Option<u32>,
        container_id: Option<String>,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let record_input: CreateRecordInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = graduate_note_service(
            &self.store,
            GraduateNoteInput {
                note_id: note_id.to_string(),
                type_ref: type_ref.to_string(),
                type_version,
                record_input,
                container_id,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Serialise the current repository state to a `.srsj` JSON string.
    /// The browser caller can use this to offer a download of the edited repo.
    ///
    /// `.srsj` is JSON-only and cannot carry binary content: a repository
    /// holding attachment bytes (added here, or loaded from a `.srs` archive)
    /// is refused with a diagnostic naming the file. Offer
    /// [`SrsRepository::export_archive`] for those — `.srs` carries them.
    #[wasm_bindgen]
    pub fn export_srsj(&self) -> Result<String, JsValue> {
        srs_repository::srsj::to_srsj_string(&self.store).map_err(js_err)
    }

    /// Export the session as an exploded file tree (ADR-038): a JS object of
    /// `{ path: Uint8Array }`. Untouched files are byte-identical to what was
    /// loaded — the clean-git-diff guarantee.
    pub fn export_tree(&self) -> Result<JsValue, JsValue> {
        let map = srs_repository::export_tree(&self.store).map_err(js_err)?;
        let obj = js_sys::Object::new();
        for (path, bytes) in &map {
            js_sys::Reflect::set(
                &obj,
                &JsValue::from_str(path),
                &js_sys::Uint8Array::from(bytes.as_slice()).into(),
            )
            .map_err(|_| js_err("failed to build export_tree object"))?;
        }
        Ok(obj.into())
    }

    /// Export the current repository state as a `.srs` binary archive (ZIP bytes).
    pub fn export_archive(&self) -> Result<js_sys::Uint8Array, JsValue> {
        let bytes = srs_repository::archive_to_vec(&self.store).map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }

    /// Export one container as an RFC-026 slice archive (`.srs` ZIP bytes): the
    /// same `slice_service::export_container_slice` as `srs slice export`
    /// (ADR-051). Refusals surface as the service error, e.g.
    /// `slice-root-identity-invalid: ...`.
    pub fn export_slice(&self, container_id: &str) -> Result<js_sys::Uint8Array, JsValue> {
        let export = srs_repository::slice_service::export_container_slice(
            &self.store,
            srs_repository::slice_service::ExportSliceInput {
                container_id: container_id.to_string(),
                ..Default::default()
            },
        )
        .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(export.bytes.as_slice()))
    }

    /// Return the raw bytes of a source-document attachment by `documentId`.
    ///
    /// Repositories loaded via [`SrsRepository::load`] (from a `.srsj` string) never contain
    /// binary file content — [`SrsRepository::load_archive`] is the path that populates binary
    /// content. A `.srsj`-loaded repository will return an error for this method.
    ///
    /// Returns the attachment file bytes as a `Uint8Array`, or a JS error string when:
    /// - `documentId` is not in `manifest.sourceDocumentIndex` (not found in index)
    /// - binary content is absent (tombstone state — archive does not contain the file)
    pub fn get_attachment_bytes(&self, document_id: &str) -> Result<js_sys::Uint8Array, JsValue> {
        let result = attachment_service::get_attachment_bytes(
            &self.store,
            GetAttachmentBytesInput {
                document_id: document_id.to_string(),
            },
        )
        .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(result.bytes.as_slice()))
    }

    /// Create a record. `input_json` is a JSON object with fields:
    /// `fieldValues` (an object keyed by `Field.name`, RFC-039 carrier),
    /// `fieldMeta` (optional object keyed identically), and `tags` (optional
    /// array of strings).
    /// Returns the created `Record` as a JS value.
    pub fn create_record(
        &self,
        type_id: &str,
        type_version: u32,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: CreateRecordBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        srs_repository::actor_service::creation_actor(
            &self.store,
            input
                .extra
                .contains_key(srs_repository::actor_service::CREATED_BY_KEY),
        )
        .map_err(js_err)?;
        let record = record_store::create_record(
            &self.store,
            type_id,
            type_version,
            input.field_values,
            input.field_meta,
            input.tags,
        )
        .map_err(js_err)?;
        to_js(&record)
    }

    /// Create a Tier-2 record and add it to a container in one call.
    ///
    /// `container_id` is the UUID of the container to add the record to.
    /// `type_id` is the UUID of the type; `type_version` is the version number.
    /// `input_json` is a JSON object with `fieldValues` (required), `fieldMeta` (optional),
    /// and `tags` (optional) — the same shape as `create_record`.
    ///
    /// Returns the created `Record` as a JS value.
    /// Returns a JS error if the container does not exist, the type is not found,
    /// or field validation fails.
    pub fn create_record_in_container(
        &self,
        container_id: &str,
        type_id: &str,
        type_version: u32,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: CreateRecordBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = record_store::create_record_in_container(
            &self.store,
            record_store::CreateRecordInContainerInput {
                container_id: container_id.to_string(),
                type_id: type_id.to_string(),
                type_version,
                field_values: input.field_values,
                field_meta: input.field_meta,
                tags: input.tags,
                extra: input.extra,
            },
        )
        .map_err(js_err)?;
        to_js(&result.record)
    }

    /// Update a record. `input_json` is a JSON object with fields:
    /// `fieldValues` (an object keyed by `Field.name`, RFC-039 carrier),
    /// `fieldMeta` (optional), `tags` (optional),
    /// `typeVersion` (optional u32 — omit to keep the stored version).
    /// Returns the updated `Record` as a JS value.
    pub fn update_record(&self, instance_id: &str, input_json: &str) -> Result<JsValue, JsValue> {
        let input: record_store::UpdateRecordInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let record =
            record_store::update_record(&self.store, instance_id, input).map_err(js_err)?;
        to_js(&record)
    }

    /// Delete a record by instance ID. `cascade` mirrors the CLI's `--cascade`
    /// flag (srs-rust#1025): `false` (the default a caller should pass unless
    /// it has already decided) refuses a record that is the **target** of
    /// inbound relations — e.g. a successor's `derived-from` edge — rather
    /// than silently dropping them; `true` deletes them with the record.
    /// Returns `{ instanceId, cascadedRelations: RelationSummary[] }` — every
    /// relation (inbound or outbound) this delete actually removed.
    pub fn delete_record(&self, instance_id: &str, cascade: bool) -> Result<JsValue, JsValue> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct DeleteRecordOutput {
            instance_id: String,
            cascaded_relations: Vec<relation_service::RelationSummary>,
        }
        let result =
            record_store::delete_record(&self.store, instance_id, cascade).map_err(js_err)?;
        to_js(&DeleteRecordOutput {
            instance_id: result.instance_id,
            cascaded_relations: result.cascaded_relations,
        })
    }

    /// List relations. `filter_json` is a JSON object with optional camelCase fields:
    /// `{ "source": "uuid", "target": "uuid", "relationType": "...", "containerId": "uuid" }`
    /// Pass `"{}"` for all relations.
    /// Returns a JS array of `RelationSummary` objects.
    pub fn list_relations(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        struct FilterInput {
            source: Option<String>,
            target: Option<String>,
            relation_type: Option<String>,
            container_id: Option<String>,
        }
        let input: FilterInput = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = ListRelationsFilter {
            source: input.source,
            target: input.target,
            relation_type: input.relation_type,
            container_id: input.container_id,
        };
        let summaries = relation_service::list_relations(&self.store, filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Bounded read of an instance's relation neighbours (srs-rust#1229): each edge's type and
    /// direction plus the neighbour's id, label and type, never the record. `direction` is
    /// `"out"` | `"in"` (omit for both); `limit` (default: all) and `offset` (default 0) page
    /// the edges after the deterministic sort; `total` counts every match.
    /// Returns a `NeighboursResult` as a JS value.
    pub fn neighbours(
        &self,
        instance_id: &str,
        relation_type: Option<String>,
        direction: Option<String>,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let direction = direction
            .map(|d| d.parse::<EdgeDirection>())
            .transpose()
            .map_err(js_err)?;
        let result = context_query_service::list_neighbours(
            &self.store,
            NeighboursQuery {
                instance_id: instance_id.to_string(),
                relation_type,
                direction,
            },
            NeighboursPage {
                limit,
                offset: offset.unwrap_or(0),
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Create a relation. `input_json` is a JSON object whose fields match the `Relation` struct
    /// (camelCase: `relationType`, `sourceInstanceId`, `targetInstanceId`; `relationId` is
    /// auto-generated if absent or empty). Also accepts the canonical standalone `relation.json`
    /// carrier's `$schema` property, which is validated (if present) and then discarded
    /// (srs-rust#1021).
    /// Returns the created `Relation` as a JS value.
    pub fn create_relation(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let raw: serde_json::Value = serde_json::from_str(input_json)
            .map_err(|e| js_err(format!("invalid relation input: {e}")))?;
        if let Some(obj) = raw.as_object() {
            srs_repository::actor_service::reject_supplied_created_by(&self.store, obj)
                .map_err(js_err)?;
        }
        let relation = relation_service::parse_relation_input(raw).map_err(js_err)?;
        let result =
            relation_service::create_relation_auto(&self.store, relation).map_err(js_err)?;
        to_js(&result.relation)
    }

    /// Delete a relation by its `relation_id`. Returns `undefined` on success.
    pub fn delete_relation(&self, relation_id: &str) -> Result<(), JsValue> {
        relation_service::delete_relation(&self.store, relation_id).map_err(js_err)?;
        Ok(())
    }

    /// Order a set of instance IDs by following the `precedes` relation chain.
    ///
    /// `input_json` is `{ "instanceIds": ["uuid1", "uuid2", ...] }`.
    /// Returns `{ "orderedIds": [...] }` — same IDs, reordered by the `precedes`
    /// chain. Falls back to `created_at` ascending, then `instanceId` ascending,
    /// for records not connected by a `precedes` edge. Handles cycles.
    pub fn order_by_precedes(&self, input_json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            instance_ids: Vec<String>,
        }
        let parsed: Input =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = relation_service::order_by_precedes(
            &self.store,
            OrderByPrecedesInput {
                instance_ids: parsed.instance_ids,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Atomically rebuild a linear `precedes` chain.
    ///
    /// `input_json` is `{ "instanceIds": ["uuid1", ...], "clearIds": ["uuid1", ...] }`.
    /// All `precedes` edges where source OR target is in `clearIds` are deleted first;
    /// then `n-1` new `precedes` edges connect `instanceIds[0]→[1]→…→[n-1]`.
    ///
    /// Returns `{ "created": [<RelationSummary>, ...] }` as a JS value where each
    /// `RelationSummary` is `{ "relationId", "relationType", "sourceId", "targetId" }`.
    pub fn rebuild_precedes_chain(&self, input_json: &str) -> Result<JsValue, JsValue> {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Input {
            instance_ids: Vec<String>,
            clear_ids: Vec<String>,
        }
        let parsed: Input =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = relation_service::rebuild_precedes_chain(
            &self.store,
            RebuildPrecedesChainInput {
                instance_ids: parsed.instance_ids,
                clear_ids: parsed.clear_ids,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Insert one instance into a `precedes` chain right after `afterId` or right
    /// before `beforeId` (exactly one required) — chain-local: only edges incident
    /// to the anchor and the inserted instance are touched, so a disjoint sibling
    /// chain (srs-rust#1124) is untouched.
    ///
    /// `input_json` is `{ "instanceId": "uuid", "afterId": "uuid" }` or
    /// `{ "instanceId": "uuid", "beforeId": "uuid" }`.
    ///
    /// Returns `{ "created": [<RelationSummary>, ...], "removed": [<RelationSummary>, ...] }`.
    /// Errors if `instanceId` already has `precedes` edges.
    pub fn insert_into_precedes_chain(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: InsertIntoPrecedesChainInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            relation_service::insert_into_precedes_chain(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Remove one instance from its `precedes` chain, reconnecting its predecessor
    /// to its successor when both existed — chain-local, a disjoint sibling chain
    /// is untouched (srs-rust#1124).
    ///
    /// `input_json` is `{ "instanceId": "uuid" }`.
    /// Returns `{ "created": [<RelationSummary>, ...], "removed": [<RelationSummary>, ...] }`.
    pub fn remove_from_precedes_chain(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: RemoveFromPrecedesChainInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            relation_service::remove_from_precedes_chain(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Move one instance to a new position in a `precedes` chain in one batch:
    /// remove it from its current position (reconnecting prev→next), then insert
    /// it after `afterId` or before `beforeId` (exactly one required) —
    /// chain-local throughout (srs-rust#1124).
    ///
    /// `input_json` is `{ "instanceId": "uuid", "afterId": "uuid" }` or
    /// `{ "instanceId": "uuid", "beforeId": "uuid" }`.
    /// Returns `{ "created": [<RelationSummary>, ...], "removed": [<RelationSummary>, ...] }`.
    pub fn move_in_precedes_chain(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: MoveInPrecedesChainInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            relation_service::move_in_precedes_chain(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Transition a record's lifecycle state.
    /// `state` is the target state name (e.g. `"ratified"`).
    /// Returns `{ "record": <Record>, "warnings": ["LIFECYCLE_FINAL_STATE: ..."] }` as a JS value.
    /// `warnings` is empty for non-final transitions; contains a `LIFECYCLE_FINAL_STATE` entry
    /// when the target state has `isFinal: true`.
    /// Note: a bare flip into a state declaring `requiresRelation` (RFC-022) is rejected unless
    /// the obligation is already satisfied — use `transition_record` with a `fulfillment` input.
    pub fn set_lifecycle_state(&self, instance_id: &str, state: &str) -> Result<JsValue, JsValue> {
        let input = TransitionLifecycleInput {
            to: Some(state.to_string()),
            by_transition: None,
            fulfillment: None,
        };
        let result = record_store::transition_record_lifecycle(&self.store, instance_id, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Transition a record's lifecycle state with the full RFC-022 input surface.
    /// `input_json` matches the CLI `record transition` stdin contract:
    /// `{ "to"?: string, "byTransition"?: string, "fulfillment"?: {
    ///    "newRecord"?: { "fieldValues": {...}, "typeVersion"?: N },
    ///    "existingInstanceId"?: "<uuid>", "relationType"?: "supersedes" } }`.
    /// Returns `{ "record", "warnings", "successor"?, "relation"? }` as a JS value —
    /// `successor`/`relation` are present when the transition was fulfilled.
    pub fn transition_record(
        &self,
        instance_id: &str,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: TransitionLifecycleInput = serde_json::from_str(input_json)
            .map_err(|e| js_err(format!("invalid transition input: {e}")))?;
        let result = record_store::transition_record_lifecycle(&self.store, instance_id, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Query the allowed lifecycle transitions for a record (ext:lifecycle).
    /// Returns `{ "currentState": string, "transitions": [{ "name": string, "to": string,
    /// "toIsFinal": bool, "requiresRelation"?: { "relationType": string|string[],
    /// "direction"?: "incoming"|"outgoing" } }], "isImmutable": bool }` as a JS value.
    /// `requiresRelation` (RFC-022) is the target state's relation obligation — clients route
    /// successor-flow UX from it instead of string-matching state names.
    pub fn get_allowed_lifecycle_transitions(&self, instance_id: &str) -> Result<JsValue, JsValue> {
        let result = record_store::get_allowed_lifecycle_transitions(&self.store, instance_id)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Create a successor record that supersedes or refines an existing record.
    /// `predecessor_id` is the instance ID of the record being superseded/refined.
    /// `input_json` is a JSON object:
    ///   `{ "relationType"?: "supersedes"|"refines", "fieldValues": {...}, "lifecycleState"?: "...", "typeVersion"?: N }`.
    /// `relationType` may be omitted: the core derives it from the predecessor's lifecycle
    /// `requiresRelation` (RFC-022 R6) or errors with `SUCCESSOR_RELATION_TYPE_UNDETERMINED`.
    /// Returns `{ "record": <Record>, "relation": <Relation> }` as a JS value.
    /// The relation runs from the successor (source) to the predecessor (target).
    pub fn create_record_successor(
        &self,
        predecessor_id: &str,
        input_json: &str,
    ) -> Result<JsValue, JsValue> {
        let input: record_store::CreateRecordSuccessorInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = record_store::create_record_successor(&self.store, predecessor_id, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Project a blueprint into a nested draft-07 JSON Schema describing the whole
    /// multi-record document it declares. `blueprint_id` is the blueprint's UUID.
    /// Returns `{ "schema": <json-schema>, "diagnostics": [<string>, ...] }` as a JS value;
    /// non-fatal projection problems surface in `diagnostics`.
    pub fn blueprint_schema(&self, blueprint_id: &str) -> Result<JsValue, JsValue> {
        let result = blueprint_schema_service::blueprint_schema(
            &self.store,
            BlueprintSchemaInput {
                blueprint_id: blueprint_id.to_string(),
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Render a document view. `view_id` is the view's UUID; `format` is one of
    /// `"json"`, `"markdown"`, `"html"` or `"adoc"`;
    /// `container_id` optionally scopes TypeQuery sections to a container's membership;
    /// `instance_id_filter` optionally scopes ContainerSubset sections to a single record,
    /// producing a per-record export document.
    /// Returns `{ "rendered": <string>, "diagnostics": [...], "projection": <json|null> }`.
    /// When `format == "json"`, `projection` is a `CompositionProjection` object with shape:
    /// `{ $schema, compositionId, containerId: string|null, generatedAt, containerTitle,
    ///   preamble?, sections: [{ sectionId, title?, order, records: [{ instanceId, typeId,
    ///   typeVersion, typeNamespace, typeName, recordHeading?, preamble?, fields,
    ///   orderedFieldKeys, relations?, properties?, children?: [<nested record, same shape,
    ///   recursive>] }], sections?: [<nested ProjectedSection, same shape, recursive>] }] }`.
    /// `records[*].children` (srs-rust#1127) carries the record's `contains` children —
    /// same order and same condition (the enclosing section declares a `titleFieldId`) as
    /// the markdown/html/adoc renderer's nested-heading recursion — so a client can take
    /// document structure from the engine instead of re-deriving it from raw `contains`
    /// relations. Omitted when the section has no `titleFieldId`, or the record has no
    /// `contains` children.
    /// `sections[*].sections` (RFC-042 Revision 5 [R25]) carries nested sections produced by a
    /// `container-subset` source with `containerScope: "subtree"`; it is omitted when a section
    /// renders no nested section, and its records are never flattened into the parent's `records`.
    /// `exclude_instance_ids` (JS `string[]`, optional) drops those instances from container-subset
    /// sections; an arranged section removes each by the RFC-043 promoting removal. Additive: the
    /// single-record `instance_id_filter` is unchanged.
    /// `containerId` is always present in the JSON but may be `null` when the view is
    /// not scoped to a container.
    /// `records[*].relations` is present when the document view defines a `relationsPresentation`;
    /// each entry is `{ label: string, targets: [{ instanceId, displayLabel }] }`.
    /// `records[*].properties` is present when the document view defines a `RecordPropertyView`
    /// (RFC-041); each entry is `{ property, label, value }` — `value` is a string, or a string
    /// array when `property` is `tags`.
    /// `fields` carries a composite value recursively under its own key (RFC-039 [R11]) —
    /// there is no separate `fieldGroups` projection.
    pub fn render_composition(
        &self,
        view_id: &str,
        format: &str,
        container_id: Option<String>,
        instance_id_filter: Option<String>,
        exclude_instance_ids: Option<Vec<String>>,
    ) -> Result<JsValue, JsValue> {
        let exclude = exclude_instance_ids.unwrap_or_default();
        let result = render_service::render_composition(RenderCompositionOptions {
            store: &self.store,
            view_id,
            format: Some(format),
            theme_variant: None,
            container_id: container_id.as_deref(),
            instance_id_filter: instance_id_filter.as_deref(),
            exclude_instance_ids: &exclude,
        })
        .map_err(js_err)?;
        to_js(&result)
    }

    /// List composition (L2) summaries. `filter_json` is a JSON string matching
    /// `{ "namespace"?: string, "name"?: string, "containerType"?: string, "rootTypeId"?: string }`;
    /// pass `"{}"` for all document views. `name` filters to an exact view name. `rootTypeId` keeps only views whose
    /// `rootTypeRefs` include that Type UUID (RFC-009). Returns a JS array of objects
    /// `{ id, namespace, name, version, description, containerType?, rootTypeRefs?, sourcePackage? }`.
    pub fn list_compositions(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let parsed: CompositionListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = CompositionListFilter {
            namespace: parsed.namespace,
            name: parsed.name,
            container_type: parsed.container_type,
            root_type_id: parsed.root_type_id,
        };
        let summaries =
            view_service::list_compositions_summary(&self.store, &filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// List container summaries. `filter_json` is a JSON string matching
    /// `{ "containerType"?: string, "memberInstanceId"?: string, "anchorInstanceId"?: string }`
    /// (matches the container's `anchorInstanceId`, RFC-043 [R4]; `rootInstanceId` is accepted as an alias);
    /// pass `"{}"` for all containers. Returns a JS array of `ContainerSummary` objects.
    pub fn list_containers(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let parsed: ContainerListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let filter = ContainerListFilter {
            container_type: parsed.container_type,
            member_instance_id: parsed.member_instance_id,
            anchor_instance_id: parsed.anchor_instance_id,
        };
        let summaries = container_service::list_containers(&self.store, &filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Get a single container by ID, including its ordered `memberInstanceIds` entries.
    /// Returns the `Container` as a JS value.
    pub fn get_container(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let container =
            container_service::get_container(&self.store, container_id).map_err(js_err)?;
        to_js(&container)
    }

    /// Create a container (RFC-043 revision-8 shape; same service as `srs container create` and
    /// the MCP `container_create` tool). `input_json` is
    /// `{ "title": string, "containerId"?: uuid (minted if omitted), "description"?, "containerType"?,
    /// "anchorInstanceId"?, "identityInstanceId"?, "memberInstanceIds"?: [{instanceId, depth?}],
    /// "tags"? }`. Returns the created `Container`; a JS error carries the validation message.
    pub fn create_container(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let container = create_container_from_json(&self.store, input_json).map_err(js_err)?;
        to_js(&container)
    }

    /// Patch a container (srs-rust#1199; same service as `srs container update`). `patch_json` is
    /// the CLI's `ContainerPatch`: any of `{ title, namespace, name, description, containerType,
    /// tags, meta, identityInstanceId, anchorInstanceId, memberInstanceIds, childContainerIds }`;
    /// omitted keys are untouched, unknown keys are rejected. `identityInstanceId: null` clears it
    /// (srs-rust#1293) — the one key here that distinguishes omitted from explicit `null`.
    /// Returns `{ container, diagnostics }`.
    pub fn update_container(
        &self,
        container_id: &str,
        patch_json: &str,
    ) -> Result<JsValue, JsValue> {
        let result =
            update_container_from_json(&self.store, container_id, patch_json).map_err(js_err)?;
        to_js(&result)
    }

    /// Copy a container (srs-rust#1136; same service as `srs container copy` and the MCP
    /// `container_copy` tool): shares the member records, forks only the anchor.
    /// `input_json` is `{ "title"?: string, "containerId"?: uuid }` (may be empty/`{}`).
    /// Returns `{ container, forks: [{originalId, forkId}], relations }`.
    pub fn copy_container(&self, source_id: &str, input_json: &str) -> Result<JsValue, JsValue> {
        let result =
            copy_container_from_json(&self.store, source_id, input_json).map_err(js_err)?;
        to_js(&result)
    }

    /// Fork `instance_id` and its nested children inside `container_id` (srs-rust#1136; same
    /// service as `srs record fork` and the MCP `record_fork` tool). Returns
    /// `{ containerId, forks: [{originalId, forkId}], relations }`.
    pub fn fork_record(&self, container_id: &str, instance_id: &str) -> Result<JsValue, JsValue> {
        let result =
            srs_repository::fork_service::fork_subtree(&self.store, container_id, instance_id)
                .map_err(js_err)?;
        to_js(&result)
    }

    /// Add an instance to a container's ordered `memberInstanceIds` outline (RFC-043).
    /// `position` (0-based; omit to append) and `depth` (omit for 0). Idempotent when neither is
    /// given. Returns `{ members: [{instanceId, depth?}], promoted, removed }`.
    pub fn add_container_member(
        &self,
        container_id: &str,
        instance_id: &str,
        position: Option<u32>,
        depth: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let result = container_service::add_member(
            &self.store,
            container_id,
            instance_id,
            position.map(|p| p as usize),
            depth,
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Remove an instance from a container's outline, promoting its descendants. Same result
    /// shape as `add_container_member`.
    pub fn remove_container_member(
        &self,
        container_id: &str,
        instance_id: &str,
    ) -> Result<JsValue, JsValue> {
        let result = container_service::remove_member(&self.store, container_id, instance_id)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Move an entry's run to `position` and/or set its `depth` (RFC-043 move / set depth).
    pub fn move_container_member(
        &self,
        container_id: &str,
        instance_id: &str,
        position: Option<u32>,
        depth: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let result = container_service::move_member(
            &self.store,
            container_id,
            instance_id,
            position.map(|p| p as usize),
            depth,
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Relative move (issue #1156): give `relative_to` + `placement` (`"before"` | `"after"` |
    /// `"into"`) or `shift` (`"indent"` | `"outdent"` | `"up"` | `"down"`), not both. The core
    /// resolves it against the outline, so no client arithmetic is needed. Same result shape as
    /// `move_container_member`; a JS error carries a rejection (illegal target, identity pin, [R2]).
    pub fn move_container_member_relative(
        &self,
        container_id: &str,
        instance_id: &str,
        relative_to: Option<String>,
        placement: Option<String>,
        shift: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let mv = RelativeMove::parse(
            relative_to.as_deref(),
            placement.as_deref(),
            shift.as_deref(),
        )
        .map_err(|e| JsValue::from_str(&e))?
        .ok_or_else(|| JsValue::from_str("give relativeTo with placement, or shift"))?;
        let result =
            container_service::move_member_relative(&self.store, container_id, instance_id, &mv)
                .map_err(js_err)?;
        to_js(&result)
    }

    /// Add an instance and place it `before` / `after` / `into` `relative_to` in one write.
    pub fn add_container_member_relative(
        &self,
        container_id: &str,
        instance_id: &str,
        relative_to: &str,
        placement: &str,
    ) -> Result<JsValue, JsValue> {
        let Some(RelativeMove::Place { target, placement }) =
            RelativeMove::parse(Some(relative_to), Some(placement), None)
                .map_err(|e| JsValue::from_str(&e))?
        else {
            return Err(JsValue::from_str("relativeTo and placement are required"));
        };
        let result = container_service::add_member_relative(
            &self.store,
            container_id,
            instance_id,
            &target,
            placement,
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// The derived outline: `{ containerId, anchorInstanceId, identityInstanceId, entries, body }`
    /// where each entry is `{ instanceId, depth, parentInstanceId, hasChildren, runSize, runEnd }`
    /// and `body` excludes the anchor and identity entries (the renderer's rule).
    pub fn get_container_outline(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let outline = container_service::get_outline(&self.store, container_id).map_err(js_err)?;
        to_js(&outline)
    }

    /// Remove every entry that no longer resolves to an instance (RFC-043 repair).
    pub fn repair_container_members(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let result =
            container_service::repair_members(&self.store, container_id).map_err(js_err)?;
        to_js(&result)
    }

    /// The container's arrangement: its entries with `depth`, in order (RFC-043 [R15]).
    pub fn get_container_arrangement(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let entries =
            container_service::get_arrangement(&self.store, container_id).map_err(js_err)?;
        to_js(&entries)
    }

    /// List the containers an instance belongs to — every container whose `memberInstanceIds`
    /// includes `instance_id`. Returns a JS array of `ContainerSummary` objects (same shape as
    /// `list_containers`). Equivalent to `list_containers('{"memberInstanceId": instance_id}')`,
    /// exposed by name for the web client (issue #181).
    pub fn containers_for_instance(&self, instance_id: &str) -> Result<JsValue, JsValue> {
        let summaries =
            container_service::containers_for_instance(&self.store, instance_id).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Project a Type into a draft-07 JSON Schema describing a single record's `fieldValues`,
    /// keyed by field `name`. `type_id` is the Type's UUID; `type_version` selects a version —
    /// pass `undefined` (omit the argument) to resolve the latest version.
    /// Returns `{ "schema": <json-schema>, "diagnostics": [<string>, ...] }` as a JS value;
    /// non-fatal projection problems (a dangling `fieldId`, a select field with no
    /// `allowedValues`) surface in `diagnostics`. An unresolvable Type is an error.
    pub fn type_schema(
        &self,
        type_id: &str,
        type_version: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let result = type_schema_service::type_schema(
            &self.store,
            TypeSchemaInput {
                type_id: type_id.to_string(),
                type_version,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Project a Type into a **standard** JSON Schema 2020-12 definition schema
    /// (RFC-035). Distinct from `type_schema`, which returns the editor-facing
    /// draft-07 + `x-srs-*` projection: this one is what validates a Record.
    ///
    /// Returns `{ schema, canonicalJson, inexpressible }`. `schema` is the
    /// structured object (its keys arrive sorted, since JS objects are built
    /// from a `serde_json::Value`); `canonicalJson` is the exact byte sequence
    /// `projection-rules.md` pins, which is what byte-parity is defined against
    /// and what a client must write if it persists the artifact.
    /// `inexpressible` names every constraint JSON Schema could not carry —
    /// never silently dropped.
    pub fn type_json_schema(
        &self,
        type_id: &str,
        type_version: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let result = srs_projection::type_to_json_schema(
            &self.store,
            srs_projection::TypeToJsonSchemaInput {
                type_id: type_id.to_string(),
                type_version,
            },
        )
        .map_err(js_err)?;
        let canonical_json = srs_projection::json_schema::to_canonical_json(&result.schema)
            .map_err(|e| JsValue::from_str(&format!("failed to serialize canonical JSON: {e}")))?;
        to_js(&JsonSchemaBindingResult {
            schema: serde_json::to_value(&result.schema).unwrap_or(serde_json::Value::Null),
            canonical_json,
            inexpressible: result.inexpressible,
        })
    }

    /// Emit the RFC-035 generated-schema bundle envelope for the named
    /// meta-model entities (default: `field`, `type`), stamped with the
    /// repository's `dataModelRevision`. Same `{ ..., canonicalJson,
    /// inexpressible }` contract as `type_json_schema`.
    pub fn generate_schema_bundle(&self, entities: Vec<String>) -> Result<JsValue, JsValue> {
        let entities = if entities.is_empty() {
            vec!["field".to_string(), "type".to_string()]
        } else {
            entities
        };
        let result = srs_projection::schema_bundle(
            &self.store,
            srs_projection::SchemaBundleInput { entities },
        )
        .map_err(js_err)?;
        let canonical_json = srs_projection::json_schema::to_canonical_json(&result.bundle)
            .map_err(|e| JsValue::from_str(&format!("failed to serialize canonical JSON: {e}")))?;
        to_js(&JsonSchemaBindingResult {
            schema: serde_json::to_value(&result.bundle).unwrap_or(serde_json::Value::Null),
            canonical_json,
            inexpressible: result.inexpressible,
        })
    }

    /// List blueprint summaries across all package boundaries.
    /// Returns a JS value matching `BlueprintListResult`; WARN-level
    /// provenance issues (missing files, duplicate IDs) surface in `diagnostics`.
    pub fn list_blueprints(&self) -> Result<JsValue, JsValue> {
        let result = blueprint_service::list_blueprints_summary(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// List a blueprint's `structure` (RelationSpec list, with type names resolved),
    /// sorted deterministically by `(sourceTypeId, targetTypeId, relationType)`.
    /// `blueprint_id` is the blueprint's UUID. Same shape as the CLI's
    /// `blueprint structure` command payload's `relationSpecs`: a JS array of
    /// `{ relationType, sourceTypeId, sourceTypeName?, targetTypeId, targetTypeName?,
    /// cardinality?, required? }` objects.
    pub fn list_blueprint_structure(&self, blueprint_id: &str) -> Result<JsValue, JsValue> {
        let result = blueprint_service::list_blueprint_structure(&self.store, blueprint_id)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// List protocol summaries from the compiled package model.
    /// Returns a JS array of `{ protocolId, protocolNamespace, protocolName, protocolVersion,
    /// stageCount, sourcePackage? }` objects.
    pub fn list_protocols(&self) -> Result<JsValue, JsValue> {
        let summaries = protocol_service::list_protocols(&self.store).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Get a protocol's stored definition JSON by its `id`.
    /// Returns the full protocol definition as a JS value, or `null` if not found.
    pub fn get_protocol_by_id(&self, id: &str) -> Result<JsValue, JsValue> {
        match protocol_service::get_protocol_by_id(&self.store, id).map_err(js_err)? {
            GetProtocolResult::Found(val) => to_js(&val),
            GetProtocolResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// Find the first protocol whose `targetType` matches `target_type_id`.
    /// Returns `{ protocolId, protocolName, stages, diagnostics }` as a JS value,
    /// or `null` if no protocol targets that type.
    pub fn find_protocol_by_target_type(&self, target_type_id: &str) -> Result<JsValue, JsValue> {
        match protocol_service::find_protocol_by_target_type(&self.store, target_type_id)
            .map_err(js_err)?
        {
            Some(result) => to_js(&result),
            None => Ok(JsValue::NULL),
        }
    }

    // ── Definition browse (fields, types, L1 views, packages) ────────────────────

    /// List field definitions from the compiled package, optionally filtered by namespace or
    /// package boundary path. `filter_json` is `{}` or `{"namespace":"...","package":"..."}`.
    /// Returns a JS array of `FieldSummary` objects.
    pub fn list_fields(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: FieldListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let fields = package_service::list_fields_filtered(
            &self.store,
            FieldListFilter {
                namespace: filter.namespace,
                package: filter.package.map(Some),
            },
        )
        .map_err(js_err)?;
        to_js(&fields)
    }

    /// Get a field definition by its id. Returns the full `Field` object, or `null` if not found.
    pub fn get_field(&self, id: &str) -> Result<JsValue, JsValue> {
        match package_service::get_field_by_id(&self.store, id).map_err(js_err)? {
            GetFieldResult::Found(field) => to_js(&*field), // GetFieldResult wraps Box<Field>
            GetFieldResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List type definitions from the compiled package, optionally filtered by namespace or
    /// package boundary path. `filter_json` is `{}` or `{"namespace":"...","package":"..."}`.
    /// Returns a JS array of `TypeSummary` objects.
    pub fn list_types(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: TypeListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let types = package_service::list_types_filtered(
            &self.store,
            TypeListFilter {
                namespace: filter.namespace,
                package: filter.package.map(Some),
            },
        )
        .map_err(js_err)?;
        to_js(&types)
    }

    /// List relation type definitions from the compiled package.
    /// `filter_json` is `{}` or `{"status":"active"}` to filter by status.
    /// Returns a JS array of `RelationTypeDefinition` objects.
    pub fn list_relation_types(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RelationTypeListBindingFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let relation_types = package_service::list_relation_types_filtered(
            &self.store,
            RelationTypeListFilter {
                status: filter.status,
            },
        )
        .map_err(js_err)?;
        to_js(&relation_types)
    }

    /// Get a type definition by its id (latest version). Returns the full `RecordType` object,
    /// or `null` if not found.
    pub fn get_type(&self, id: &str) -> Result<JsValue, JsValue> {
        match package_service::get_type_by_id_latest(&self.store, id).map_err(js_err)? {
            GetTypeResult::Found(record_type) => to_js(&record_type),
            GetTypeResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List L1 view definitions from the compiled package. Returns a JS array of `ViewSummary`
    /// objects (`{id, namespace, name, version, description, compatibleTypes?, sourcePackage?}`).
    pub fn list_views(&self) -> Result<JsValue, JsValue> {
        let views = view_service::list_views_summary(&self.store).map_err(js_err)?;
        to_js(&views)
    }

    /// Get an L1 view definition by its id. Returns the full `View` object, or `null` if not found.
    pub fn get_view(&self, id: &str) -> Result<JsValue, JsValue> {
        match view_service::get_view_by_id(&self.store, id).map_err(js_err)? {
            GetViewResult::Found(view) => to_js(&*view),
            GetViewResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List all package boundaries (primary + sub-packages) from the repository manifest.
    /// Returns a JS array of objects with shape `{id, namespace, name, version,
    /// boundaryPath: string | null, fieldCount, typeCount}`.
    /// `boundaryPath` is `null` for the primary package and the boundary path string for sub-packages.
    pub fn list_packages(&self) -> Result<JsValue, JsValue> {
        let packages = package_service::list_packages(&self.store).map_err(js_err)?;
        to_js(&packages)
    }

    /// RFC-044 requirement check before install (same service as `srs package dependency
    /// check`): `input_json` is `{ "packageId"?: uuid, "packageDependencies": DependencyRef[] }`
    /// — a package bundle, or a requirement list that is not a package's (srs-web's
    /// `EditorDefinition` requires; omit `packageId`). Returns `{ selector: null, packageId,
    /// action: null, dependencies: [{packageId?, namespace, name, version, satisfied,
    /// reason?, candidateVersions, mismatchedLabels}] }`; `reason` is one of RFC-044's seven
    /// codes. Clients present the outcome; they never compare versions themselves.
    pub fn check_package_requirements(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: package_dependency_service::BundleRequirements =
            serde_json::from_str(input_json).map_err(js_err)?;
        let result =
            package_dependency_service::check_bundle(&self.store, &input).map_err(js_err)?;
        to_js(&result)
    }

    /// Export a package boundary as a deterministic .srspkg (same service as `srs package export`).
    /// input_json: {"selector"?: string|null, "publishedAt"?: string, "publisher"?: string}.
    /// Pass a fixed publishedAt for a reproducible sha256. Returns {text, summary}
    /// (PackageBundleExport; summary.sha256 is "sha256:<hex>"). Read-only: write_epoch does not move.
    pub fn export_package_bundle(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: package_bundle::ExportPackageInput =
            serde_json::from_str(input_json).map_err(js_err)?;
        let result = package_bundle::export_package_bundle(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Install a .srspkg (same service as `srs package install --bundle`). bundle_json is the file's
    /// text (verify its sha256 over the file bytes before calling); options_json:
    /// {"boundaryPath"?: string, "strict"?: bool} ("{}" for defaults). Returns InstallPackageResult
    /// (same fields as the CLI payload, incl. notes). Advances write_epoch; the session is dirty.
    /// Call check_package_requirements(bundle_json) first for RFC-044 requirement outcomes
    /// (BundleRequirements reads packageId/packageDependencies from the same text).
    pub fn install_package_bundle(
        &self,
        bundle_json: &str,
        options_json: &str,
    ) -> Result<JsValue, JsValue> {
        let options: package_install_service::InstallBundleOptions =
            serde_json::from_str(options_json).map_err(js_err)?;
        let result = package_install_service::install_package_bundle_bytes(
            &self.store,
            bundle_json.as_bytes(),
            options,
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Upgrade an installed package from a newer .srspkg (same service as `srs package upgrade`).
    /// bundle_json is the file's text; options_json: {"dryRun"?: bool, "boundaryPath"?: string}
    /// ("{}" for defaults). Returns UpgradePackageResult (same fields as the CLI payload:
    /// packageId, name, previousVersion, version, upgraded, dryRun, added, newVersions, updated,
    /// unchanged, conflicts, removedUpstream, dependencyWarnings, notes). A downgrade or a package
    /// that is not installed is an error. Advances write_epoch unless dryRun (a dry run writes
    /// nothing); the session is dirty after a real run.
    pub fn upgrade_package_bundle(
        &self,
        bundle_json: &str,
        options_json: &str,
    ) -> Result<JsValue, JsValue> {
        let options: package_install_service::UpgradeOptions =
            serde_json::from_str(options_json).map_err(js_err)?;
        let result = package_install_service::upgrade_package_bundle(
            &self.store,
            bundle_json.as_bytes(),
            options,
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Aggregate import records across all boundaries and run live divergence detection.
    /// Returns an `ImportSummary` object (`{generatedAt, fields, types, views, blueprints,
    /// protocols, relationTypes, skippedDefinitions?}`).
    /// Each record includes `conflictState` ("clean" | "local-ahead") for
    /// `upstream-tracked` definitions where a reference copy exists.
    pub fn list_package_imports_json(&self) -> Result<JsValue, JsValue> {
        let summary =
            package_service::list_package_imports(&self.store, ListPackageImportsFilter::default())
                .map_err(js_err)?;
        to_js(&summary)
    }

    /// List the document views (L2) bound to a container's root type. Resolves the container's
    /// first root instance's `typeId`/`typeVersion`, then returns every `Composition` whose
    /// `rootTypeRefs` includes that exact type binding (RFC-009). Returns an empty array — not an
    /// error — when the container has no root instance, the root carries no type binding (Tier 0/1),
    /// or no view matches. Returns a JS array of **full** `Composition` objects (including
    /// `sections`) — not the lighter summaries that `list_compositions` returns — because the
    /// caller needs the section definitions to render the view.
    pub fn compositions_for_container(&self, container_id: &str) -> Result<JsValue, JsValue> {
        let views =
            view_service::compositions_for_container(&self.store, container_id).map_err(js_err)?;
        to_js(&views)
    }

    /// Resolve a structured container view for an editor member list (issue #254, #256):
    /// the container root record, the ordered member records (Tier-0, Tier-1, or Tier-2;
    /// full `Record` present only for Tier-2), the Composition-driven column spec, and
    /// diagnostics. `view_id` optionally overrides the Composition; when omitted it is
    /// matched from the container's root type binding. Returns a `ContainerView` object.
    pub fn resolve_container_view(
        &self,
        container_id: &str,
        view_id: Option<String>,
    ) -> Result<JsValue, JsValue> {
        let result = container_view_service::resolve_container_view(
            &self.store,
            ResolveContainerViewInput {
                container_id: container_id.to_string(),
                view_id,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Return the repository's root identity and precedes-ordered section navigation.
    /// Equivalent to `srs repo navigation`.
    ///
    /// Returns `{ rootContainerId, identity, sections, diagnostics }`.
    /// - `identity`: `NavigationNode` for the root identity record (`instanceId`, `typeId`,
    ///   `typeVersion`, `typeNamespace`, `typeName`, `displayLabel`).
    /// - `sections`: precedes-ordered `NavigationNode` array; each carries an optional
    ///   `sectionContainerId` pointing to the section's own container.
    /// - `diagnostics`: non-empty when `manifest.container` is absent (pre-RFC-013 repo) or a
    ///   member id fails to resolve; in that case `rootContainerId` is `""`, `identity` has
    ///   all-empty-string/zero fields (it is a present object, not `null`), and `sections` is `[]`.
    pub fn repository_navigation(&self) -> Result<JsValue, JsValue> {
        let result =
            repository_navigation_service::repository_navigation(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// List all vocabulary Terms (RFC-006) defined in the package.
    /// Returns a JS array of `Term` objects — the same terms returned by `srs term list`.
    /// srs-web uses this to populate the tag picker / tag cloud.
    pub fn list_terms(&self) -> Result<JsValue, JsValue> {
        let terms = tag_service::list_terms(&self.store).map_err(js_err)?;
        to_js(&terms)
    }

    /// Scaffold a governance repository from a seeded `.srsj` store.
    ///
    /// `input_json` is a JSON string matching `CreateGovernanceRepositoryInput`
    /// (`{"title":"...","purpose":"...","repositoryId":"..."}`).
    /// `namespace` is optional — when omitted or `null`, the service derives
    /// `"com.example.<slug>"` from the title (e.g. `"My Org"` → `"com.example.my-org"`).
    /// Pass an explicit `namespace` (`{"namespace":"com.example.myorg","title":"..."}`)
    /// when a different organisational prefix is required.
    ///
    /// Stamps manifest identity (repositoryId, namespace, title) and creates the
    /// com.semanticops.core/purpose identity record (RFC-018 I-81), Decision Log container
    /// + root record, and root container — all in one call. After this returns, call
    /// `export_srsj()` to get the final bundle for download.
    pub fn scaffold_new_repository(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: CreateGovernanceRepositoryInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = governance_scaffold_service::create_governance_repository(&self.store, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Re-stamp a seed repository's identity. `input_json` is a JSON string matching
    /// `InitNewRepositoryInput` (`{"namespace":"...","title":"...",...}`).
    /// Updates `repositoryId`, `namespace`, `title`, `description`, and
    /// `installedAt` on the upstream package reference — at top-level
    /// `upstreamPackage` for RFC-014-migrated seeds, or `meta.upstreamPackage`
    /// for legacy seeds. Returns an
    /// `InitNewRepositoryResult` as a JS value.
    pub fn init_new_repository(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: InitNewRepositoryInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            repository_lifecycle::init_new_repository(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Migrate the repository's identity instance to a `com.semanticops.core/purpose` record.
    ///
    /// Converts a Tier-0 note identity (or a container with no identity pointer) to a typed
    /// purpose record. Returns a `MigrateIdentityResult` as a JS value. Errors if the identity
    /// is already a purpose record or the container has no title/description to derive a
    /// statement from. After this returns, call `export_srsj()` to get the updated bundle.
    pub fn migrate_identity(&self) -> Result<JsValue, JsValue> {
        let result = migrate_identity_service::migrate_identity(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// List all known migrations with their applicability status for this repository.
    ///
    /// Returns a JSON array of `{ id, title, description, status }` objects where
    /// `status` is a string: `needed`, `alreadyApplied` or `notApplicable`.
    pub fn available_migrations(&self) -> Result<JsValue, JsValue> {
        let result = migration_registry_service::list_migrations(&self.store).map_err(js_err)?;
        to_js(&result)
    }

    /// Apply a migration by ID and return its result payload.
    ///
    /// The result shape is `{ id: string, payload: object }` where `payload` is
    /// migration-specific. Returns an error if the ID is unknown.
    pub fn apply_migration(&self, id: &str) -> Result<JsValue, JsValue> {
        let result =
            migration_registry_service::apply_migration(&self.store, id).map_err(js_err)?;
        to_js(&result)
    }

    /// Return the value of a named field on a record, by its exact package-defined name
    /// (the `name` field in the field definition JSON, e.g. `"title"` or `"decision-summary"`).
    /// No case normalization is performed — the caller must pass the exact name.
    ///
    /// Returns the field value as a JS value, or `null` if the field is absent from
    /// the record, the field name is not part of the type schema, or the record is
    /// not found. Never errors on a missing/unknown field — callers treat `null` as
    /// a graceful no-op.
    pub fn get_field_value_by_name(
        &self,
        instance_id: &str,
        field_name: &str,
    ) -> Result<JsValue, JsValue> {
        let result = record_store::get_field_value_by_name(
            &self.store,
            record_store::GetFieldValueByNameInput {
                instance_id: instance_id.to_string(),
                field_name: field_name.to_string(),
            },
        )
        .map_err(js_err)?;
        match result.value {
            Some(v) => to_js(&v),
            None => Ok(JsValue::NULL),
        }
    }

    /// Assemble context for a single field: current value, aiGuidance.
    ///
    /// `input_json` is `{"recordId": "<id>", "fieldId": "<id>"}`.
    /// Returns a `FieldContextResult` with `recordId`, `fieldId`, `fieldName`,
    /// `fieldNamespace`, `aiGuidance`, `currentValue`, and `taggedChunks`.
    pub fn context_field(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: FieldContextQuery =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            context_query_service::get_field_context(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// Assemble context for a record: field values and every relation touching it (both
    /// directions, neighbour inline).
    ///
    /// `input_json` is `{"recordId": "<id>", "containerId"?: "<id>", "excludeRelationCategories"?: ["composition","sequence"], "projection"?: "full"|"card"|"label"}`
    /// (`projection`, #1285: `card`/`label` inline each neighbour as a compact card instead of the whole instance);
    /// the latter drops edges by `RelationTypeDefinition.category` (#1188); with `containerId` the
    /// result also carries `entry` and `subtree` (the record's arrangement there).
    /// Returns a `RecordContextResult` with `recordId`, `typeId`, `typeName`,
    /// `typeNamespace`, `displayLabel`, `fieldValues`, `relations`, optional
    /// `containerId`/`entry`/`subtree`, `taggedChunks`, and `protocolRunHistory`.
    pub fn context_record(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: RecordContextQuery =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            context_query_service::get_record_context(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// [`Self::context_record`] as compact markdown (#1285): same `input_json`; the
    /// projection is `card` unless `"label"` is given. Returns the markdown string.
    pub fn context_record_markdown(&self, input_json: &str) -> Result<String, JsValue> {
        let input: RecordContextQuery =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        context_query_service::render_record_context_markdown(&self.store, input).map_err(js_err)
    }

    // ── Protocol runs (ext:protocol execution) ────────────────────────────────

    /// Create a new protocol run.
    ///
    /// `input_json` is `{"protocolId","protocolVersion","containerId","targetRecordId?","initialStageId?"}`.
    /// Returns the created `ProtocolRun` as a JS value.
    pub fn protocol_run_create(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: RunCreateInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = run_service::create_run(&self.store, input).map_err(js_err)?;
        to_js(&result.run)
    }

    /// Advance a protocol run to a new stage.
    ///
    /// `input_json` is `{"runId","stageId","completeCurrent"}`.
    /// Returns the updated `ProtocolRun`.
    pub fn protocol_run_advance(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: RunAdvanceInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = run_service::advance_stage(&self.store, input).map_err(js_err)?;
        to_js(&result.run)
    }

    /// Get a protocol run by its `runId`. Returns the `ProtocolRun`, or `null` if not found.
    pub fn protocol_run_get(&self, run_id: &str) -> Result<JsValue, JsValue> {
        match run_service::get_run(&self.store, run_id).map_err(js_err)? {
            GetRunResult::Found(run) => to_js(&*run),
            GetRunResult::NotFound => Ok(JsValue::NULL),
        }
    }

    /// List protocol runs. `filter_json` is `{}` or `{"protocolId"?,"containerId"?,"status"?}`.
    /// Returns a JS array of `RunSummary` objects.
    pub fn protocol_run_list(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: RunListFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let summaries = run_service::list_runs(&self.store, filter).map_err(js_err)?;
        to_js(&summaries)
    }

    /// Mark a protocol run as Completed. Returns the updated `ProtocolRun`,
    /// or a JS error string if the run is not found.
    pub fn protocol_run_complete(&self, run_id: &str) -> Result<JsValue, JsValue> {
        let result = run_service::complete_run(&self.store, run_id).map_err(js_err)?;
        to_js(&result.run)
    }

    /// Mark a protocol run as Abandoned. Returns the updated `ProtocolRun`,
    /// or a JS error string if the run is not found.
    pub fn protocol_run_abandon(&self, run_id: &str) -> Result<JsValue, JsValue> {
        let result = run_service::abandon_run(&self.store, run_id).map_err(js_err)?;
        to_js(&result.run)
    }

    /// Resolve linked attachments for a list of record instance IDs.
    ///
    /// `input_json` is `{"instanceIds": ["<uuid>", ...]}` (from a rendered composition).
    /// Returns `{sourceDocumentsPath, records: [{instanceId, attachments: [{documentId,
    /// contentPath, sidecarPath, title?, contentChecksum?, sidecarChecksum?, sizeBytes?}]}]}`.
    pub fn resolve_composition_attachments(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: ResolveCompositionAttachmentsInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = attachment_service::resolve_composition_attachments(&self.store, input)
            .map_err(js_err)?;
        to_js(&result)
    }

    /// Retrieve attachments linked to a single record via its sourceRefs.
    ///
    /// `input_json` is `{"instanceId": "<uuid>"}`.
    /// Returns `null` when the record is not found, or
    /// `{instanceId, sourceDocumentsPath, attachments: [{documentId, contentPath?,
    /// sidecarPath?, title?, contentChecksum?, sidecarChecksum?, sizeBytes?}]}` as a JS value.
    pub fn get_record_attachments(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: GetRecordAttachmentsInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result =
            attachment_service::get_record_attachments(&self.store, input).map_err(js_err)?;
        to_js(&result)
    }

    /// List source-document attachments.
    ///
    /// `filter_json` is reserved for future filter fields; pass `"{}"` for all attachments.
    /// Returns `{"sourceDocumentsPath": "...", "entries": [{...}]}` as a JS value.
    pub fn list_attachments(&self, filter_json: &str) -> Result<JsValue, JsValue> {
        let filter: ListAttachmentsFilter = serde_json::from_str(filter_json)
            .map_err(|e| js_err(format!("invalid filter: {e}")))?;
        let result = attachment_service::list_attachments(&self.store, filter).map_err(js_err)?;
        to_js(&result)
    }

    /// Store a source-document attachment from raw bytes.
    ///
    /// `input_json` is `{"fileName":"...","subdir"?:"...","title"?:"...","contentType"?:"..."}`.
    /// `file_bytes` is the raw file content (a `Uint8Array` on the JS side).
    /// Returns `{"documentId","contentPath","sidecarPath","sourceDocumentsPath",
    ///           "contentChecksum","sidecarChecksum"}` as a JS value.
    pub fn add_attachment(&self, input_json: &str, file_bytes: &[u8]) -> Result<JsValue, JsValue> {
        let input: AddAttachmentBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = attachment_service::add_attachment(
            &self.store,
            AddAttachmentInput {
                file_name: input.file_name,
                content: file_bytes.to_vec(),
                subdir: input.subdir,
                title: input.title,
                content_type: input.content_type,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }

    /// Link an existing source document to a record instance.
    ///
    /// `input_json` is `{"instanceId":"<uuid>","documentId":"<uuid>"}`.
    /// Returns `{"instanceId","documentId","sourceRefsCount"}` as a JS value.
    pub fn link_attachment(&self, input_json: &str) -> Result<JsValue, JsValue> {
        let input: LinkAttachmentBindingInput =
            serde_json::from_str(input_json).map_err(|e| js_err(format!("invalid input: {e}")))?;
        let result = attachment_service::link_attachment(
            &self.store,
            LinkAttachmentInput {
                instance_id: input.instance_id,
                document_id: input.document_id,
            },
        )
        .map_err(js_err)?;
        to_js(&result)
    }
}

// ── Repo-independent free functions (ADR-013 addendum) ───────────────────────

/// Render markdown to safe HTML (srs-rust#1191): raw HTML is escaped and link/image
/// URLs are limited to http, https, mailto and relative, so the result is safe to
/// inject with `innerHTML`. Repo-independent free function; JS name `renderMarkdown`.
#[wasm_bindgen(js_name = renderMarkdown)]
pub fn render_markdown(md: &str) -> String {
    srs_core::markdown::render_markdown(md)
}

/// Parse a registry catalog JSON string into a `Registry` object.
///
/// `catalog_json` is the raw text of a `.json` registry catalog file
/// (the `ext:registry` schema). Returns the parsed `Registry` as a JS value,
/// or a JS error string on parse failure.
///
/// This is a repo-independent free function (ADR-013 addendum): it does not
/// require a loaded SRS repository and operates on a caller-supplied payload.
#[wasm_bindgen]
pub fn parse_registry(catalog_json: &str) -> Result<JsValue, JsValue> {
    let registry = parse_registry_json(catalog_json).map_err(js_err)?;
    to_js(&registry)
}

/// Parse a registry catalog JSON string and apply an optional filter.
///
/// `catalog_json` is the raw registry catalog text. `filter_json` is a JSON
/// object with optional `publisher` (string) and `tags` ([string]) keys; pass
/// `"{}"` to return all entries. An empty or absent `tags` array matches all
/// entries. Multiple tags are AND-conjoined — an entry must carry every listed
/// tag. (Note: the initial cut used a singular `"tag"` string key; that key is
/// now rejected — use `"tags": [...]` instead.)
///
/// Returns a `Registry` JS value whose `entries` array contains only the
/// matching entries (all entries if no filter criteria are set).
///
/// This is a repo-independent free function (ADR-013 addendum).
#[wasm_bindgen]
pub fn list_registry_entries(catalog_json: &str, filter_json: &str) -> Result<JsValue, JsValue> {
    let registry = parse_registry_json(catalog_json).map_err(js_err)?;
    let filter: RegistryListFilter =
        serde_json::from_str(filter_json).map_err(|e| js_err(format!("invalid filter: {e}")))?;
    let filtered = filter_registry_entries(registry, &filter);
    to_js(&filtered)
}

/// Input shape for `list_compositions` — parsed from caller-supplied JSON.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CompositionListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    container_type: Option<String>,
    #[serde(default)]
    root_type_id: Option<String>,
}

/// `create` core. Free function so native tests exercise it.
fn create_blank_from_json(input_json: &str) -> Result<srs_repository::FileStore, String> {
    let input: repository_lifecycle::CreateBlankRepositoryInput =
        serde_json::from_str(input_json).map_err(|e| format!("invalid input: {e}"))?;
    let store = srs_repository::new_tree_session();
    repository_lifecycle::create_blank_repository(&store, input).map_err(|e| e.to_string())?;
    Ok(store)
}

/// `create_container` core: parse the container JSON and call the one core service.
/// Free function so native tests exercise it (`to_js` panics off-wasm).
fn create_container_from_json(
    store: &srs_repository::FileStore,
    input_json: &str,
) -> Result<srs_core::types::container::Container, String> {
    let input: container_service::ContainerCreateInput =
        serde_json::from_str(input_json).map_err(|e| format!("invalid input: {e}"))?;
    container_service::create_container(store, input.into()).map_err(|e| e.to_string())
}

/// `update_container` core: parse the patch and call the one core service.
fn update_container_from_json(
    store: &srs_repository::FileStore,
    container_id: &str,
    patch_json: &str,
) -> Result<serde_json::Value, String> {
    let patch: container_service::ContainerPatch =
        serde_json::from_str(patch_json).map_err(|e| format!("invalid input: {e}"))?;
    let r = container_service::update_container(store, container_id, patch)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "container": r.container, "diagnostics": r.diagnostics }))
}

/// `copy_container` core: parse the optional input and call the one core service.
fn copy_container_from_json(
    store: &srs_repository::FileStore,
    source_id: &str,
    input_json: &str,
) -> Result<container_service::ContainerCopyResult, String> {
    let input: container_service::ContainerCopyInput = if input_json.trim().is_empty() {
        Default::default()
    } else {
        serde_json::from_str(input_json).map_err(|e| format!("invalid input: {e}"))?
    };
    container_service::copy_container(store, source_id, input).map_err(|e| e.to_string())
}

/// Input shape for `list_containers` — parsed from caller-supplied JSON.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ContainerListBindingFilter {
    #[serde(default)]
    container_type: Option<String>,
    #[serde(default)]
    member_instance_id: Option<String>,
    #[serde(default, alias = "rootInstanceId")]
    anchor_instance_id: Option<String>,
}

/// Input shape for `list_fields` — parsed from caller-supplied JSON.
/// `package: null` means no package filter (returns all); a specific sub-package path
/// narrows to that boundary. The primary-package-only filter (`Some(None)` in the service)
/// is not expressible through this binding — omit `package` to get all fields.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FieldListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

/// Input shape for `list_types` — parsed from caller-supplied JSON.
/// Same package-filter semantics as `FieldListBindingFilter`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct TypeListBindingFilter {
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    package: Option<String>,
}

/// Input shape for `list_relation_types` — parsed from caller-supplied JSON.
/// `status`: None returns all relation type definitions; Some(s) returns only those
/// whose serialized status string equals s (e.g. "active").
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RelationTypeListBindingFilter {
    #[serde(default)]
    status: Option<String>,
}

/// Input shape for `create_record` — parsed from caller-supplied JSON.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRecordBindingInput {
    field_values: FieldValues,
    #[serde(default)]
    field_meta: Option<indexmap::IndexMap<String, FieldMeta>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// Only so a request-supplied `createdBy` is caught (`actor-supplied`, RFC-046).
    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AddAttachmentBindingInput {
    file_name: String,
    #[serde(default)]
    subdir: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkAttachmentBindingInput {
    instance_id: String,
    document_id: String,
}

#[cfg(test)]
mod tests {
    use super::SrsRepository;
    use srs_repository::RepositoryStore;

    #[test]
    fn mcp_session_write_guard_rejects_then_clears() {
        let store = super::create_blank_from_json(
            r#"{"title":"T","description":"Why","namespace":"com.t.x"}"#,
        )
        .unwrap();
        let identity = store
            .load_manifest()
            .unwrap()
            .container
            .unwrap()
            .identity_instance_id
            .unwrap();
        let c = super::create_container_from_json(&store, r#"{"title":"C"}"#).unwrap();
        let repo = SrsRepository { store };
        let mut s = repo.open_mcp_session().unwrap();
        let init = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
        s.handle(init).unwrap();
        s.set_write_guard(&format!(r#"{{"containerIds":["{}"]}}"#, c.container_id))
            .unwrap();
        let call = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"container_member_add","arguments":{{"containerId":"{}","instanceId":"{identity}"}}}}}}"#,
            c.container_id
        );
        let e0 = s.write_epoch();
        let rejected = s.handle(&call).unwrap();
        assert!(rejected.contains("write guard") && rejected.contains("\"isError\":true"));
        assert_eq!(s.write_epoch(), e0, "rejection must not advance the epoch");
        s.clear_write_guard();
        assert!(s.handle(&call).unwrap().contains("\"isError\":false"));
        assert!(s.write_epoch() > e0);
    }

    /// RFC-046: the actor is host-set per handle — the MCP session's actor and the UI
    /// handle's actor are independent, and neither comes from tool arguments.
    #[test]
    fn session_actors_are_per_handle_and_stamp_createdby() {
        let store = super::create_blank_from_json(
            r#"{"title":"T","description":"Why","namespace":"com.t.x"}"#,
        )
        .unwrap();
        let repo = SrsRepository { store };
        let mut s = repo.open_mcp_session().unwrap();
        s.handle(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#)
            .unwrap();
        let note = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"note_create","arguments":{"title":"N","sections":[{"name":"b","content":"x"}]}}}"#;

        s.set_actor(r#"{"kind":"ai","id":"agent-1"}"#).unwrap();
        let stamped = s.handle(note).unwrap();
        assert!(
            stamped.contains(r#"\"createdBy\""#) && stamped.contains("agent-1"),
            "{stamped}"
        );

        // The UI handle has its own (here: no) actor — its writes are unattributed.
        let ui = srs_repository::services::create_note(
            &repo.store,
            serde_json::from_str(r#"{"title":"U","sections":[{"name":"b","content":"x"}]}"#)
                .unwrap(),
        )
        .unwrap();
        assert!(ui.note.created_by.is_none());
        repo.set_actor(r#"{"kind":"human","id":"user-7"}"#).unwrap();
        let ui = srs_repository::services::create_note(
            &repo.store,
            serde_json::from_str(r#"{"title":"U2","sections":[{"name":"b","content":"x"}]}"#)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ui.note.created_by.unwrap().id, "user-7");

        s.clear_actor();
        let anon = s.handle(note).unwrap();
        assert!(!anon.contains("createdBy"), "{anon}");
    }

    #[test]
    fn repository_write_epoch_advances_on_ui_writes_not_reads() {
        let store = super::create_blank_from_json(
            r#"{"title":"T","description":"Why","namespace":"com.t.x"}"#,
        )
        .unwrap();
        let identity = store
            .load_manifest()
            .unwrap()
            .container
            .unwrap()
            .identity_instance_id
            .unwrap();
        let repo = SrsRepository { store };
        let e0 = repo.write_epoch();

        let c = super::create_container_from_json(&repo.store, r#"{"title":"C"}"#).unwrap();
        let e1 = repo.write_epoch();
        assert!(e1 > e0, "container create must advance the epoch");

        srs_repository::container_service::get_container(&repo.store, &c.container_id).unwrap();
        srs_repository::container_service::list_containers(&repo.store, &Default::default())
            .unwrap();
        assert_eq!(repo.write_epoch(), e1, "reads must not advance it");

        let copy = super::copy_container_from_json(&repo.store, &c.container_id, "").unwrap();
        assert_eq!(copy.container.title, "C (copy)");
        assert!(
            repo.write_epoch() > e1,
            "container copy must advance the epoch"
        );
        assert!(
            super::copy_container_from_json(&repo.store, &c.container_id, r#"{"x":1}"#).is_err()
        );

        srs_repository::container_service::add_member(
            &repo.store,
            &c.container_id,
            &identity,
            None,
            None,
        )
        .unwrap();
        assert!(repo.write_epoch() > e1, "member add must advance the epoch");

        // One counter: an MCP session over this repository reads the same epoch.
        let session = repo.open_mcp_session().unwrap();
        assert_eq!(session.write_epoch(), repo.write_epoch());
        super::create_container_from_json(&repo.store, r#"{"title":"D"}"#).unwrap();
        assert_eq!(session.write_epoch(), repo.write_epoch());
    }

    #[test]
    fn update_container_patches_title_and_rejects_unknown_keys() {
        let store =
            super::create_blank_from_json(r#"{"title":"T","namespace":"com.t.x"}"#).unwrap();
        let c = super::create_container_from_json(&store, r#"{"title":"Old"}"#).unwrap();
        let r = super::update_container_from_json(&store, &c.container_id, r#"{"title":"New"}"#)
            .unwrap();
        assert_eq!(r["container"]["title"], "New");
        assert_eq!(r["diagnostics"], serde_json::json!([]));
        assert!(
            super::update_container_from_json(&store, &c.container_id, r#"{"bogus":1}"#).is_err()
        );
    }

    #[test]
    fn create_container_accepts_child_containers_and_meta_and_rejects_unknown_keys() {
        let store = super::create_blank_from_json(
            r#"{"title":"T","description":"Why","namespace":"com.t.x"}"#,
        )
        .unwrap();
        let child = super::create_container_from_json(&store, r#"{"title":"C"}"#).unwrap();
        let parent = super::create_container_from_json(
            &store,
            &serde_json::json!({
                "title": "P",
                "childContainerIds": [child.container_id],
                "meta": { "x": 1 }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(parent.child_container_ids, Some(vec![child.container_id]));
        assert_eq!(parent.meta, Some(serde_json::json!({ "x": 1 })));
        assert!(super::create_container_from_json(&store, r#"{"title":"x","bogus":1}"#).is_err());
    }
    #[test]
    fn create_blank_validates_and_round_trips() {
        let store = super::create_blank_from_json(
            r#"{"title":"T","description":"Why","namespace":"com.t.x"}"#,
        )
        .unwrap();
        let report = srs_repository::validation::validate_repository(&store).unwrap();
        assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
        let m = store.load_manifest().unwrap();
        assert_eq!(m.extra["dataModelRevision"], 9);
        let c = m.container.as_ref().unwrap();
        let id = c.identity_instance_id.clone().unwrap();
        let members = c.member_instance_ids.as_ref().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].instance_id, id);
        assert!(members[0].depth.unwrap_or(0) == 0);
        assert!(!c.extra.contains_key("rootInstanceIds"));

        // session round trip: export_srsj -> load
        let srsj = srs_repository::srsj::to_srsj_string(&store).unwrap();
        let again = srs_repository::srsj::open_srsj(&srsj).unwrap();
        let r2 = srs_repository::validation::validate_repository(&again).unwrap();
        assert_eq!(r2.summary.errors, 0);
        assert_eq!(
            again.load_manifest().unwrap().extra["repositoryId"],
            m.extra["repositoryId"]
        );
        assert!(
            super::create_blank_from_json("{}").is_err(),
            "namespace or title required"
        );
        let derived = super::create_blank_from_json(r#"{"title":"My Essays"}"#).unwrap();
        assert_eq!(
            derived.load_manifest().unwrap().extra["namespace"],
            "com.example.my-essays"
        );
        assert!(
            super::create_blank_from_json(r#"{"namespace":"a.b","purpose":"x"}"#).is_err(),
            "unknown keys refused"
        );
        assert!(
            super::create_blank_from_json(r#"{"namespace":"a.b","title":" "}"#).is_err(),
            "blank title refused"
        );
    }

    use super::create_container_from_json;
    use serde_json::json;
    use srs_repository::record_store::CreateRecordInput;
    use srs_repository::services::{graduate_note as graduate_note_service, GraduateNoteInput};

    /// Minimal `.srsj` with one note (tier-0) and one optional-field type `com.test/bind-type`.
    fn srsj_with_note_and_type() -> String {
        serde_json::json!({
            "srsj": "2",
            "manifest": {
                "repositoryId": "test-repo-bindings-graduate",
                "srsVersion": "2.0-draft",
                "dataModelRevision": 2,
                "namespace": "com.test",
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "pkg-bindings-001",
                    "title": "Test Package",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "namespace": "com.test",
                    "name": "bind-package",
                    "version": "1.0.0",
                    "fields": ["fields/body.json"],
                    "types": ["types/bind-type.json"],
                    "relationTypes": [],
                    "views": [],
                    "compositions": []
                },
                "package/fields/body.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                    "id": "field-bind-00001",
                    "namespace": "com.test",
                    "name": "body",
                    "version": 1,
                    "fieldType": {"datatype": "string"},
                    "description": "Body",
                    "aiGuidance": {"purpose": "Test guidance"},
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/bind-type.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": "type-bind-00001",
                    "namespace": "com.test",
                    "name": "bind-type",
                    "version": 1,
                    "description": "Binding test type",
                    "fields": [{
                        "fieldId": "field-bind-00001",
                        "order": 0,
                        "required": false
                    }],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "records/notes/binding-test-note.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
                    "instanceId": "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                    "sections": [{"name": "body", "content": "test content"}]
                }
            }
        })
        .to_string()
    }

    #[test]
    fn graduate_note_service_result_serialises() {
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let result = graduate_note_service(
            &store,
            GraduateNoteInput {
                note_id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string(),
                type_ref: "com.test/bind-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: Default::default(),
                    field_meta: None,
                    tags: None,
                    lifecycle_state: None,
                    extra: std::collections::BTreeMap::new(),
                },
                container_id: None,
            },
        )
        .expect("graduate_note should succeed");

        let json = serde_json::to_value(&result).expect("result must serialize");
        assert!(
            json["note"]["graduatedAt"].is_null(),
            "note.graduatedAt must NOT be stamped — the derived-from relation is the \
             sole graduation-provenance record (srs-rust#779)"
        );
        assert!(
            json["record"].is_object(),
            "record must be present as an object"
        );
        assert!(
            json["record"]["instanceId"].is_string(),
            "record.instanceId must be present"
        );

        let relations = srs_repository::relation_service::list_relations(
            &store,
            srs_repository::relation_service::ListRelationsFilter {
                source: Some(result.record.instance_id.clone()),
                target: Some("dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string()),
                relation_type: Some("derived-from".to_string()),
                container_id: None,
            },
        )
        .expect("list_relations should succeed");
        assert_eq!(
            relations.len(),
            1,
            "graduate_note must assert exactly one derived-from edge (record -> note)"
        );
    }

    /// Minimal `.srsj` with one type `com.test/container-item-type` (one required string field)
    /// and one container. Used by `create_record_in_container_result_serialises`.
    fn srsj_with_container_and_type() -> String {
        serde_json::json!({
            "srsj": "2",
            "manifest": {
                "repositoryId": "test-repo-bindings-container",
                "srsVersion": "2.0-draft",
                "dataModelRevision": 2,
                "namespace": "com.test",
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "pkg-container-001",
                    "title": "Test Package",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "namespace": "com.test",
                    "name": "container-package",
                    "version": "1.0.0",
                    "fields": ["fields/title.json"],
                    "types": ["types/container-item-type.json"],
                    "relationTypes": [],
                    "views": [],
                    "compositions": []
                },
                "package/fields/title.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                    "id": "field-title-00001",
                    "namespace": "com.test",
                    "name": "title",
                    "version": 1,
                    "fieldType": {"datatype": "string"},
                    "description": "Title",
                    "aiGuidance": {"purpose": "Test guidance"},
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/container-item-type.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": "type-container-001",
                    "namespace": "com.test",
                    "name": "container-item-type",
                    "version": 1,
                    "description": "Binding test type for container membership",
                    "fields": [{
                        "fieldId": "field-title-00001",
                        "order": 0,
                        "required": true
                    }],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "containers/cccccccc-cccc-4ccc-8ccc-cccccccccccc.json": {
                    "containerId": "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                    "title": "Test Container",
                    "memberInstanceIds": []
                }
            }
        })
        .to_string()
    }

    #[test]
    fn create_record_in_container_result_serialises() {
        use srs_repository::container_service;
        use srs_repository::record_store::{self, CreateRecordInContainerInput};

        let srsj = srsj_with_container_and_type();
        let store = srs_repository::srsj::open_srsj(&srsj).expect("load srsj");

        let result = record_store::create_record_in_container(
            &store,
            CreateRecordInContainerInput {
                container_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".to_string(),
                type_id: "type-container-001".to_string(),
                type_version: 1,
                field_values: srs_core::types::record::FieldValues(
                    [("title".to_string(), serde_json::json!("My Decision"))]
                        .into_iter()
                        .collect(),
                ),
                field_meta: None,
                tags: None,
                extra: Default::default(),
            },
        )
        .expect("create_record_in_container should succeed");

        let json = serde_json::to_value(&result.record).expect("record must serialize");
        let instance_id = json["instanceId"]
            .as_str()
            .expect("instanceId must be string");
        assert!(!instance_id.is_empty(), "instanceId must be non-empty");

        let container =
            container_service::get_container(&store, "cccccccc-cccc-4ccc-8ccc-cccccccccccc")
                .expect("container loaded");
        let members = container.member_ids();
        assert!(
            members.contains(&instance_id.to_string()),
            "instanceId must appear in container memberInstanceIds"
        );
    }

    #[test]
    fn blueprint_schema_result_serialises() {
        use srs_repository::blueprint_schema_service::BlueprintSchemaResult;
        let result = BlueprintSchemaResult {
            schema: serde_json::Value::Null,
            diagnostics: vec![],
        };
        let json = serde_json::to_value(&result).expect("BlueprintSchemaResult must serialize");
        assert!(json["schema"].is_null(), "schema field must be present");
        assert!(
            json["diagnostics"].is_array(),
            "diagnostics field must be present as array"
        );
    }

    #[test]
    fn get_record_summary_by_id_smoke() {
        use srs_repository::record_store::{get_record_summary_by_id, CreateRecordInput};
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        // Graduate the note to get a typed record we can look up
        let note_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let graduated = srs_repository::services::graduate_note(
            &store,
            srs_repository::services::GraduateNoteInput {
                note_id: note_id.to_string(),
                type_ref: "com.test/bind-type".to_string(),
                type_version: None,
                record_input: CreateRecordInput {
                    field_values: Default::default(),
                    field_meta: None,
                    tags: None,
                    lifecycle_state: None,
                    extra: std::collections::BTreeMap::new(),
                },
                container_id: None,
            },
        )
        .expect("graduate should succeed");
        let record_id = &graduated.record.instance_id;
        let summary = get_record_summary_by_id(&store, record_id)
            .expect("should not error")
            .expect("should find record");
        assert_eq!(summary.instance_id, *record_id);
        // No title/name/label fields → falls back to type_name
        assert!(
            !summary.display_label.is_empty(),
            "display_label must be non-empty"
        );
        assert_eq!(summary.record.instance_id, *record_id);
    }

    #[test]
    fn declared_extensions_conformance_report_serialises() {
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let report = srs_repository::manifest_service::declared_extensions_conformance(&store)
            .expect("conformance report should succeed");
        let json = serde_json::to_value(&report).expect("report must serialize");
        assert!(json["declared"].is_array(), "declared must be a JSON array");
        assert!(
            json["supported"].is_array(),
            "supported must be a JSON array"
        );
        assert!(
            json["declaredButUnsupported"].is_array(),
            "declaredButUnsupported must be a JSON array"
        );
        assert!(
            json["usedButUndeclared"].is_array(),
            "usedButUndeclared must be a JSON array"
        );
        // A minimal repo with no declaredExtensions has an empty declared list
        assert!(
            json["declared"]
                .as_array()
                .expect("declared must be an array")
                .is_empty(),
            "no extensions declared in a minimal srsj repo"
        );
        // The supported list must include the known extensions
        let supported: Vec<String> = json["supported"]
            .as_array()
            .expect("supported must be an array")
            .iter()
            .map(|v| {
                v.as_str()
                    .expect("supported entry must be a string")
                    .to_string()
            })
            .collect();
        assert!(
            supported.contains(&"ext:lifecycle".to_string()),
            "supported must include ext:lifecycle"
        );
    }

    // ── get_field_value_by_name service integration tests ────────────────────
    //
    // These tests call `record_store::get_field_value_by_name` directly because
    // `JsValue` is not available in a native target. Their purpose is to confirm
    // the service function is reachable from the srs-bindings crate imports.
    // The binding's `to_js`/`JsValue::NULL` branches are validated exclusively
    // by the `cargo build --target wasm32-unknown-unknown -p srs-bindings` gate.

    /// Minimal `.srsj` with one Tier-2 record whose `"title"` field is set to `"My Title"`.
    fn srsj_with_titled_record() -> String {
        serde_json::json!({
            "srsj": "2",
            "manifest": {
                "repositoryId": "test-repo-get-field-value",
                "srsVersion": "2.0-draft",
                "dataModelRevision": 2,
                "namespace": "com.test",
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "pkg-gfvbn-001",
                    "title": "Test Package",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "namespace": "com.test",
                    "name": "gfvbn-package",
                    "version": "1.0.0",
                    "fields": ["fields/title.json"],
                    "types": ["types/titled-type.json"],
                    "relationTypes": [],
                    "views": [],
                    "compositions": []
                },
                "package/fields/title.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                    "id": "field-gfvbn-title",
                    "namespace": "com.test",
                    "name": "title",
                    "version": 1,
                    "fieldType": {"datatype": "string"},
                    "description": "Title",
                    "aiGuidance": {"purpose": "Test guidance"},
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/titled-type.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": "type-gfvbn-001",
                    "namespace": "com.test",
                    "name": "titled-type",
                    "version": 1,
                    "description": "Type with a title field",
                    "fields": [{
                        "fieldId": "field-gfvbn-title",
                        "order": 0,
                        "required": true
                    }],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "records/titled-record.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
                    "typeId": "type-gfvbn-001",
                    "typeVersion": 1,
                    "typeNamespace": "com.test",
                    "typeName": "titled-type",
                    "fieldValues": {"title": "My Title"}
                }
            }
        })
        .to_string()
    }

    #[test]
    fn get_field_value_by_name_returns_value_for_known_field() {
        use srs_repository::record_store;
        let store = srs_repository::srsj::open_srsj(&srsj_with_titled_record()).expect("load srsj");

        let result = record_store::get_field_value_by_name(
            &store,
            record_store::GetFieldValueByNameInput {
                instance_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
                field_name: "title".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert_eq!(
            result.value,
            Some(serde_json::json!("My Title")),
            "value must match the stored field value"
        );
    }

    #[test]
    fn get_field_value_by_name_returns_none_for_unknown_field() {
        use srs_repository::record_store;
        let store = srs_repository::srsj::open_srsj(&srsj_with_titled_record()).expect("load srsj");

        let result = record_store::get_field_value_by_name(
            &store,
            record_store::GetFieldValueByNameInput {
                instance_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
                field_name: "nonexistent".to_string(),
            },
        )
        .expect("get_field_value_by_name should not error");

        assert!(
            result.value.is_none(),
            "unknown field name must return None, not an error"
        );
    }

    // ── Protocol run binding smoke tests ─────────────────────────────────────

    #[test]
    fn protocol_run_create_smoke() {
        use srs_repository::protocol_run_service::{create_run, CreateRunInput};
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let result = create_run(
            &store,
            CreateRunInput {
                protocol_id: "proto-bind-001".to_string(),
                protocol_version: 1,
                container_id: "c-bind-001".to_string(),
                target_record_id: None,
                initial_stage_id: Some("s1".to_string()),
            },
        )
        .expect("create_run should succeed");

        let json = serde_json::to_value(&result.run).expect("run must serialize");
        assert_eq!(json["protocolId"].as_str(), Some("proto-bind-001"));
        assert_eq!(json["status"].as_str(), Some("Active"));
        assert_eq!(json["attentionState"]["stageId"].as_str(), Some("s1"));
    }

    #[test]
    fn protocol_run_advance_smoke() {
        use srs_repository::protocol_run_service::{
            advance_stage, create_run, AdvanceStageInput, CreateRunInput,
        };
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let run = create_run(
            &store,
            CreateRunInput {
                protocol_id: "proto-bind-002".to_string(),
                protocol_version: 1,
                container_id: "c-bind-002".to_string(),
                target_record_id: None,
                initial_stage_id: Some("s1".to_string()),
            },
        )
        .expect("create_run");

        let result = advance_stage(
            &store,
            AdvanceStageInput {
                run_id: run.run.run_id.clone(),
                stage_id: "s2".to_string(),
                complete_current: true,
            },
        )
        .expect("advance_stage should succeed");

        let json = serde_json::to_value(&result.run).expect("run must serialize");
        assert_eq!(json["attentionState"]["stageId"].as_str(), Some("s2"));
    }

    #[test]
    fn protocol_run_get_not_found_returns_null() {
        use srs_repository::protocol_run_service::{get_run, GetRunResult};
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let result = get_run(&store, "no-such-run").expect("get_run should not error");
        assert!(matches!(result, GetRunResult::NotFound));
    }

    #[test]
    fn protocol_run_list_empty_smoke() {
        use srs_repository::protocol_run_service::{list_runs, RunListFilter};
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");
        let runs = list_runs(&store, RunListFilter::default()).expect("list_runs should succeed");
        assert!(runs.is_empty(), "fresh repo has no runs");
    }

    #[test]
    fn protocol_run_complete_and_abandon_smoke() {
        use srs_repository::protocol_run_service::{
            abandon_run, complete_run, create_run, CreateRunInput,
        };
        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");

        let r1 = create_run(
            &store,
            CreateRunInput {
                protocol_id: "p1".to_string(),
                protocol_version: 1,
                container_id: "c1".to_string(),
                target_record_id: None,
                initial_stage_id: None,
            },
        )
        .expect("create run 1");
        let r2 = create_run(
            &store,
            CreateRunInput {
                protocol_id: "p2".to_string(),
                protocol_version: 1,
                container_id: "c2".to_string(),
                target_record_id: None,
                initial_stage_id: None,
            },
        )
        .expect("create run 2");

        let completed = complete_run(&store, &r1.run.run_id).expect("complete_run");
        let json = serde_json::to_value(&completed.run).expect("serialize");
        assert_eq!(json["status"].as_str(), Some("Completed"));

        let abandoned = abandon_run(&store, &r2.run.run_id).expect("abandon_run");
        let json2 = serde_json::to_value(&abandoned.run).expect("serialize");
        assert_eq!(json2["status"].as_str(), Some("Abandoned"));
    }

    #[test]
    fn test_rebuild_precedes_chain_binding_smoke() {
        use srs_repository::relation_service::{rebuild_precedes_chain, RebuildPrecedesChainInput};

        let srsj = serde_json::json!({
            "srsj": "2",
            "manifest": { "dataModelRevision": 2 },
            "data": {
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "00000000-0000-0000-0000-000000000099",
                    "title": "Test Package",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "namespace": "com.test",
                    "name": "test-package",
                    "version": "1",
                    "fields": [],
                    "types": [],
                    "relationTypes": ["relation-types/precedes.json"]
                },
                "package/relation-types/precedes.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/relation-type.json",
                    "id": "00000000-0000-0000-0000-000000000001",
                    "version": 1,
                    "namespace": "com.semanticops.srs",
                    "key": "precedes",
                    "label": "Precedes",
                    "description": "Source precedes target",
                    "category": "association",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                // RFC-038: catalog-backed E2 endpoint resolution needs real
                // (shape-valid) instance bodies at the declared paths.
                "records/id-a.json": {"instanceId": "id-a", "sections": []},
                "records/id-b.json": {"instanceId": "id-b", "sections": []},
                "records/id-c.json": {"instanceId": "id-c", "sections": []}
            }
        })
        .to_string();

        let store = srs_repository::srsj::open_srsj(&srsj).expect("load srsj");
        let result = rebuild_precedes_chain(
            &store,
            RebuildPrecedesChainInput {
                instance_ids: vec!["id-a".into(), "id-b".into(), "id-c".into()],
                clear_ids: vec![],
            },
        )
        .expect("rebuild_precedes_chain should succeed");

        assert_eq!(result.created.len(), 2);
        assert_eq!(result.created[0].source_id, "id-a");
        assert_eq!(result.created[0].target_id, "id-b");
        assert_eq!(result.created[1].source_id, "id-b");
        assert_eq!(result.created[1].target_id, "id-c");
    }

    fn precedes_chain_splice_test_srsj() -> String {
        serde_json::json!({
            "srsj": "2",
            "manifest": { "dataModelRevision": 2 },
            "data": {
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "00000000-0000-0000-0000-000000000099",
                    "title": "Test Package",
                    "description": "",
                    "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "namespace": "com.test",
                    "name": "test-package",
                    "version": "1",
                    "fields": [],
                    "types": [],
                    "relationTypes": ["relation-types/precedes.json"]
                },
                "package/relation-types/precedes.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/relation-type.json",
                    "id": "00000000-0000-0000-0000-000000000001",
                    "version": 1,
                    "namespace": "com.semanticops.srs",
                    "key": "precedes",
                    "label": "Precedes",
                    "description": "Source precedes target",
                    "category": "association",
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "records/id-a.json": {"instanceId": "id-a", "sections": []},
                "records/id-b.json": {"instanceId": "id-b", "sections": []},
                "records/id-c.json": {"instanceId": "id-c", "sections": []},
                "relations/dddddddd-0000-4000-8000-0000000000ab.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/relation.json",
                    "relationId": "dddddddd-0000-4000-8000-0000000000ab",
                    "relationType": "precedes",
                    "sourceInstanceId": "id-a",
                    "targetInstanceId": "id-b",
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string()
    }

    #[test]
    fn test_insert_into_precedes_chain_binding_smoke() {
        use srs_repository::relation_service::{
            insert_into_precedes_chain, InsertIntoPrecedesChainInput,
        };
        let store =
            srs_repository::srsj::open_srsj(&precedes_chain_splice_test_srsj()).expect("load srsj");
        let result = insert_into_precedes_chain(
            &store,
            InsertIntoPrecedesChainInput {
                instance_id: "id-c".into(),
                after_id: Some("id-a".into()),
                before_id: None,
            },
        )
        .expect("insert_into_precedes_chain should succeed");
        assert_eq!(result.created.len(), 2);
        assert_eq!(result.removed.len(), 1);
    }

    #[test]
    fn test_remove_from_precedes_chain_binding_smoke() {
        use srs_repository::relation_service::{
            remove_from_precedes_chain, RemoveFromPrecedesChainInput,
        };
        let store =
            srs_repository::srsj::open_srsj(&precedes_chain_splice_test_srsj()).expect("load srsj");
        let result = remove_from_precedes_chain(
            &store,
            RemoveFromPrecedesChainInput {
                instance_id: "id-a".into(),
            },
        )
        .expect("remove_from_precedes_chain should succeed");
        assert_eq!(result.removed.len(), 1);
        assert!(result.created.is_empty(), "no predecessor to reconnect");
    }

    #[test]
    fn test_move_in_precedes_chain_binding_smoke() {
        use srs_repository::relation_service::{move_in_precedes_chain, MoveInPrecedesChainInput};
        let store =
            srs_repository::srsj::open_srsj(&precedes_chain_splice_test_srsj()).expect("load srsj");
        let result = move_in_precedes_chain(
            &store,
            MoveInPrecedesChainInput {
                instance_id: "id-a".into(),
                after_id: None,
                before_id: Some("id-c".into()),
            },
        )
        .expect("move_in_precedes_chain should succeed");
        assert!(!result.created.is_empty());
    }

    // Note: load_archive / export_archive route through js_sys::Uint8Array and JsValue, which
    // are not meaningful on a native target. The test below validates the service functions
    // (archive_to_vec + archive_to_tree) that back the bindings.
    // The wasm32 build gate confirms the binding wrapper layer compiles and links correctly.
    #[test]
    fn archive_service_roundtrip_smoke() {
        use srs_repository::services::{list_notes, ListNotesFilter};

        let store = srs_repository::srsj::open_srsj(&srsj_with_note_and_type()).expect("load srsj");

        let bytes = srs_repository::archive_to_vec(&store).expect("archive_to_vec");

        let reloaded = srs_repository::archive::archive_to_tree(std::io::Cursor::new(&bytes))
            .expect("from_archive");
        let result = list_notes(&reloaded, ListNotesFilter::default()).expect("list notes");
        assert!(
            !result.notes.is_empty(),
            "reloaded store should preserve the note"
        );
    }

    // ── RFC-017 I-107 size-warning surface test ───────────────────────────────
    //
    // Proves that validation::validate_repository emits Warning-severity diagnostics
    // for attachment-size violations and that they surface in the RepositoryValidationReport
    // returned by the validate() binding. Uses srs_repository::srsj::open_srsj() consistent with the
    // existing test style in this file; binary content is added via save_binary_file after
    // loading because .srsj is a JSON-only format. The wasm32 build gate confirms the
    // to_js(&report) call in validate() compiles and links correctly against this report shape.
    #[test]
    fn validate_size_warning_surfaces_through_report() {
        use srs_repository::store::RepositoryStore;
        use srs_repository::validation::{self, DiagnosticSeverity};

        const MAX_FILE: &str = "bb000002-0000-4000-b000-000000000002";
        const TYPE_ID: &str = "bb000010-0000-4000-b000-000000000010";
        const RECORD_ID: &str = "bb000020-0000-4000-b000-000000000020";
        const DOC_ID: &str = "cc000001-0000-4000-b000-000000000001";

        let srsj = serde_json::json!({
            "srsj": "2",
            "manifest": {
                "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
                "srsVersion": "2.0",
                "dataModelRevision": 2,
                "repositoryId": "00000000-0000-4000-8000-000000000099",
                "title": "Size Warning Test",
                "container": {
                    "containerId": "00000000-0000-4000-8000-000000000099",
                    "title": "Size Warning Test"
                },
                "sourceDocumentsPath": "source-documents",
                "packageRef": {"mode": "local", "path": "package"},
                "createdAt": "2026-01-01T00:00:00Z"
            },
            "data": {
                "package/package.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                    "id": "bb000000-0000-4000-b000-000000000000",
                    "namespace": "com.semanticops.base",
                    "name": "base",
                    "title": "Base Package",
                    "description": "Attachment policy fields and types for size-warning tests.",
                    "status": "active",
                    "version": "1.0.0",
                    "fields": ["fields/max_per_file_bytes.json"],
                    "types": ["types/repo_settings.json"],
                    "relationTypes": [],
                    "views": [],
                    "compositions": [],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/fields/max_per_file_bytes.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
                    "id": MAX_FILE,
                    "namespace": "com.semanticops.base",
                    "name": "max_per_file_bytes",
                    "version": 1,
                    "description": "max per-file bytes",
                    "aiGuidance": {"purpose": "Test guidance"},
                    "fieldType": {"datatype": "number"},
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "package/types/repo_settings.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": TYPE_ID,
                    "namespace": "com.semanticops.base",
                    "name": "repo_settings",
                    "version": 1,
                    "description": "attachment policy",
                    "fields": [{"fieldId": MAX_FILE, "order": 1, "required": false}],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "records/policy.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": RECORD_ID,
                    "typeId": TYPE_ID,
                    "typeVersion": 1,
                    "typeNamespace": "com.semanticops.base",
                    "typeName": "repo_settings",
                    "fieldValues": {"max_per_file_bytes": 50},
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string();

        let store = srs_repository::srsj::open_srsj(&srsj).expect("load srsj fixture");

        store
            .save_binary_file("source-documents/big-file.bin", &[0u8; 200])
            .expect("save binary content");
        store
            .save_text_file(
                "source-documents/big-file.bin.meta.json",
                &serde_json::to_string(&serde_json::json!({
                    "documentId": DOC_ID,
                    "contentPath": "big-file.bin",
                    "contentType": "application/octet-stream",
                    "createdAt": "2026-01-01T00:00:00Z"
                }))
                .unwrap(),
            )
            .expect("save sidecar");

        let report =
            validation::validate_repository(&store).expect("validate_repository should not error");

        assert_eq!(
            report.summary.errors, 0,
            "size-limit violations must not raise errors (non-blocking); diagnostics: {:?}",
            report.diagnostics
        );
        assert!(
            report.is_ok(),
            "is_ok() must return true when only warnings are present"
        );
        assert!(
            report.summary.warnings > 0,
            "expected at least one warning diagnostic, got: {:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| {
                d.severity == DiagnosticSeverity::Warning && d.message.contains("I-107")
            }),
            "expected a Warning-severity I-107 diagnostic, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn list_attachments_result_serialises() {
        use srs_repository::attachment_service::{AttachmentEntry, ListAttachmentsResult};
        let result = ListAttachmentsResult {
            source_documents_path: "source-documents".to_string(),
            entries: vec![
                AttachmentEntry {
                    path: "foo.pdf".to_string(),
                    document_id: Some("doc-001".to_string()),
                    title: Some("Foo".to_string()),
                    content_checksum: Some("sha256:abc".to_string()),
                    sidecar_checksum: None,
                    size_bytes: None,
                },
                AttachmentEntry {
                    path: "bar.pdf".to_string(),
                    document_id: None,
                    title: None,
                    content_checksum: None,
                    sidecar_checksum: None,
                    size_bytes: Some(99),
                },
            ],
        };
        let json = serde_json::to_value(&result).expect("ListAttachmentsResult must serialize");
        assert_eq!(
            json["sourceDocumentsPath"].as_str(),
            Some("source-documents")
        );
        assert!(json["entries"].is_array());
        assert_eq!(json["entries"][0]["documentId"].as_str(), Some("doc-001"));
        assert_eq!(
            json["entries"][0]["contentChecksum"].as_str(),
            Some("sha256:abc")
        );
        assert!(
            json["entries"][0].get("sidecarChecksum").is_none(),
            "skip_serializing_if absent"
        );
        assert!(
            json["entries"][0].get("sizeBytes").is_none(),
            "None size_bytes skipped in serialization"
        );
        assert_eq!(
            json["entries"][1]["sizeBytes"].as_u64(),
            Some(99),
            "Some size_bytes present in serialization"
        );
    }

    #[test]
    fn add_attachment_result_serialises() {
        use srs_repository::attachment_service::AddAttachmentResult;
        let result = AddAttachmentResult {
            document_id: "doc-002".to_string(),
            content_path: "brief.pdf".to_string(),
            sidecar_path: "brief.meta.json".to_string(),
            source_documents_path: "source-documents".to_string(),
            content_checksum: "sha256:aaa".to_string(),
            sidecar_checksum: "sha256:bbb".to_string(),
        };
        let json = serde_json::to_value(&result).expect("AddAttachmentResult must serialize");
        assert_eq!(json["documentId"].as_str(), Some("doc-002"));
        assert_eq!(json["contentPath"].as_str(), Some("brief.pdf"));
        assert_eq!(json["sidecarPath"].as_str(), Some("brief.meta.json"));
        assert_eq!(
            json["sourceDocumentsPath"].as_str(),
            Some("source-documents")
        );
        assert_eq!(json["contentChecksum"].as_str(), Some("sha256:aaa"));
        assert_eq!(json["sidecarChecksum"].as_str(), Some("sha256:bbb"));
    }

    #[test]
    fn link_attachment_result_serialises() {
        use srs_repository::attachment_service::LinkAttachmentResult;
        let result = LinkAttachmentResult {
            instance_id: "inst-001".to_string(),
            document_id: "doc-001".to_string(),
            source_refs_count: 3,
        };
        let json = serde_json::to_value(&result).expect("LinkAttachmentResult must serialize");
        assert_eq!(json["instanceId"].as_str(), Some("inst-001"));
        assert_eq!(json["documentId"].as_str(), Some("doc-001"));
        assert_eq!(json["sourceRefsCount"].as_u64(), Some(3));
    }

    /// Verify that `ProjectedRecord.relations` serialises with camelCase keys matching the
    /// TypeScript interface declared in `srs-web/src/lib/srs-client.ts` (#713).
    ///
    /// srs-rust#817: `relationType`/`direction` (RTD-27) and `typeVersion` now
    /// serialise too — the canonical `document-view-output.json` requires
    /// both, and the prior omission was the conformance bug this issue fixed.
    #[test]
    fn projected_record_with_relations_serialises() {
        use srs_repository::render_service::{
            ProjectedRecord, ProjectedRelationDirection, ProjectedRelationRow,
            ProjectedRelationTarget,
        };
        let target = ProjectedRelationTarget {
            instance_id: "target-001".to_string(),
            display_label: "Target Label".to_string(),
        };
        let row = ProjectedRelationRow {
            relation_type: "relates-to".to_string(),
            direction: ProjectedRelationDirection::Forward,
            label: "Related decisions".to_string(),
            targets: vec![target],
        };
        let record = ProjectedRecord {
            instance_id: "rec-001".to_string(),
            type_id: "type-001".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "decision".to_string(),
            record_heading: None,
            preamble: None,
            fields: serde_json::json!({}),
            ordered_field_keys: vec![],
            relations: Some(vec![row]),
            properties: None,
            children: vec![],
            depth: None,
        };
        let json = serde_json::to_value(&record).expect("ProjectedRecord must serialize");
        assert_eq!(json["typeVersion"].as_u64(), Some(1));
        let relations = json["relations"]
            .as_array()
            .expect("relations must be array");
        assert_eq!(relations.len(), 1, "one relation row");
        assert_eq!(
            json["relations"][0]["relationType"].as_str(),
            Some("relates-to")
        );
        assert_eq!(json["relations"][0]["direction"].as_str(), Some("forward"));
        assert_eq!(
            json["relations"][0]["label"].as_str(),
            Some("Related decisions")
        );
        let targets = json["relations"][0]["targets"]
            .as_array()
            .expect("targets must be array");
        assert_eq!(targets.len(), 1, "one target");
        assert_eq!(
            json["relations"][0]["targets"][0]["instanceId"].as_str(),
            Some("target-001")
        );
        assert_eq!(
            json["relations"][0]["targets"][0]["displayLabel"].as_str(),
            Some("Target Label")
        );
        // Absence: when relations is None the key must be absent (skip_serializing_if)
        let record_no_relations = ProjectedRecord {
            relations: None,
            ..record
        };
        let json2 =
            serde_json::to_value(&record_no_relations).expect("ProjectedRecord must serialize");
        assert!(
            json2.get("relations").is_none(),
            "relations key must be absent when None"
        );
    }

    /// RFC-039 [R11]: a composite value is carried recursively inside `fields`
    /// under its own key — the former `fieldGroups` projection is gone.
    #[test]
    fn projected_record_carries_composite_value_in_fields() {
        use srs_repository::render_service::ProjectedRecord;
        let record = ProjectedRecord {
            instance_id: "rec-002".to_string(),
            type_id: "type-001".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "decision".to_string(),
            record_heading: None,
            preamble: None,
            fields: serde_json::json!({"rows": [{"cells": ["a", "b"]}]}),
            ordered_field_keys: vec!["rows".to_string()],
            relations: None,
            properties: None,
            children: vec![],
            depth: None,
        };
        let json = serde_json::to_value(&record).expect("ProjectedRecord must serialize");
        assert_eq!(
            json["fields"]["rows"],
            serde_json::json!([{"cells": ["a", "b"]}])
        );
        assert!(
            json.get("fieldGroups").is_none(),
            "fieldGroups key no longer exists on ProjectedRecord"
        );
    }

    fn empty_store() -> srs_repository::FileStore {
        use srs_repository::repository_lifecycle::{
            create_repository, InitializeRepositoryInput, PrimaryPackageMetadata,
            RepositoryMetadata,
        };
        let store = srs_repository::tree_session::new_tree_session();
        create_repository(
            &store,
            &InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "parity".into(),
                    namespace: "com.example.parity".into(),
                    srs_version: "2.0".into(),
                    title: Some("Parity".into()),
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "pkg".into(),
                    namespace: "com.example.parity".into(),
                    name: "fixture".into(),
                    version: "1.0.0".into(),
                },
            },
        )
        .unwrap();
        store
    }

    /// WASM `create_container` and the MCP `container_create` tool take the same JSON and
    /// persist the same container (#1133 parity).
    #[test]
    fn create_container_wasm_and_mcp_agree() {
        let wasm_store = empty_store();
        let mcp_store = empty_store();
        let note = |store: &srs_repository::FileStore| {
            srs_repository::services::create_note(
                store,
                srs_core::types::note::Note {
                    created_by: None,
                    instance_id: String::new(),
                    title: Some("Intro".into()),
                    tags: None,
                    sections: vec![],
                    graduated_at: None,
                    source_refs: None,
                    created_at: None,
                    updated_at: None,
                    meta: None,
                },
            )
            .unwrap()
            .note
            .instance_id
        };
        let (a, b) = (note(&wasm_store), note(&mcp_store));
        let args = |id: &str| {
            json!({
                "containerId": "c0000000-0000-4000-8000-000000000001",
                "title": "Essay", "containerType": "essay",
                "anchorInstanceId": id, "identityInstanceId": id,
                "memberInstanceIds": [{"instanceId": id}]
            })
        };
        let wasm = create_container_from_json(&wasm_store, &args(&a).to_string()).unwrap();

        let app = srs_mcp_core::SrsMcpApplication::open(mcp_store.clone()).unwrap();
        let mut d = srs_mcp_core::McpDispatcher::new(app);
        d.dispatch_str(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
        d.dispatch_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        let call = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"container_create","arguments":args(&b)}});
        let resp: serde_json::Value =
            serde_json::from_str(&d.dispatch_str(&call.to_string()).unwrap()).unwrap();
        assert_eq!(resp["result"]["isError"], false, "{resp}");
        let mcp: srs_core::types::container::Container =
            serde_json::from_value(resp["result"]["structuredContent"].clone()).unwrap();

        let norm = |mut c: srs_core::types::container::Container, id: &str| {
            c.created_at = None;
            c.updated_at = None;
            serde_json::to_string(&c).unwrap().replace(id, "<id>")
        };
        assert_eq!(norm(wasm, &a), norm(mcp, &b));

        // An unresolvable member is rejected with a message, not persisted.
        let bad = json!({"title":"x","memberInstanceIds":[{"instanceId":"nope"}]}).to_string();
        assert!(create_container_from_json(&wasm_store, &bad).is_err());
    }
}
