use crate::error::RepositoryError;
use crate::field_json::FieldJson;
use crate::index::{InstanceIndexEntry, InstanceQuery, InstanceRef};
use crate::manifest::Manifest;
use crate::package::Package;
use crate::package_types::{DefinitionKind, PackageBoundary, PackageSelector};
use crate::repository_lifecycle::{
    default_repository_container, CreateRepositoryResult, InitializeRepositoryInput,
};
use crate::vfs::{vfs_join, DirCheck, DiskVfs, Vfs, SRS_MARKER_DIR};
use serde::de::Error as SerdeDeError;
use srs_core::types::container::ContainerIndexEntry;
use srs_core::types::field::Field;
use srs_core::types::lifecycle::Lifecycle;
use srs_core::types::note::Note;
use srs_core::types::record::Record;
use srs_core::types::record_type::RecordType;
use srs_core::types::relation_type_definition::RelationTypeDefinition;
use srs_core::types::theme::Theme;
use srs_core::types::view::{DocumentView, View};
use srs_core::types::vocabulary::Vocabulary;
use srs_core::validation::relation_type_definition::validate_relation_type_definition;
use srs_core::validation::theme::validate_theme;
use srs_core::validation::view::{validate_document_view, validate_view};
use srs_schema::{NOTE_SCHEMA_ID, RECORD_SCHEMA_ID};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

// ---------------------------------------------------------------------------
// RecordTier — logical storage tier for instance records
// ---------------------------------------------------------------------------

/// Identifies the logical storage tier for instance records.
/// Adapters map this to backend-specific paths or keys via `record_tier_dir`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordTier {
    /// Notes (Tier 0): free-text sections — maps to `records/notes`
    Note,
    /// Typed records (Tier 1): named fields, no Type binding — maps to `records/tier-1`
    Tier1,
    /// Records (Tier 2): instantiated Type — maps to `records/tier-2`
    Tier2,
    /// Extension/package records — maps to `package/records`
    Extension,
}

impl RecordTier {
    fn dir(self) -> &'static str {
        match self {
            RecordTier::Note => "records/notes",
            RecordTier::Tier1 => "records/tier-1",
            RecordTier::Tier2 => "records/tier-2",
            RecordTier::Extension => "package/records",
        }
    }
}

// ---------------------------------------------------------------------------
// Standalone relation objects (RFC-038 Change E)
// ---------------------------------------------------------------------------

/// Pinned `$schema` for standalone relation objects at `relations/<relationId>.json`
/// (RFC-038 Change E). The mirror schema file (`relation.json`) lands with the
/// Phase-1 schema-mirror PR; until it exists in `srs-schema` the pin is enforced
/// in code (write always, verify on read) rather than via `SchemaRegistry`.
pub const RELATION_OBJECT_SCHEMA_URL: &str = "https://srs.semanticops.com/schema/2.0/relation.json";

/// Derived locator for a relation object: `relations/<relationId>.json`.
/// `relations/` is flat by rule (RFC-038 Change E) — no subfolders.
pub(crate) fn relation_object_path(relation_id: &str) -> String {
    format!("relations/{relation_id}.json")
}

/// Serialize a relation as a standalone object with the pinned `$schema` first.
fn relation_object_to_value(
    relation: &srs_core::types::relation::Relation,
    path: &str,
) -> Result<serde_json::Value, RepositoryError> {
    let value = serde_json::to_value(relation).map_err(|source| RepositoryError::Serialize {
        path: PathBuf::from(path),
        source,
    })?;
    let mut map = serde_json::Map::new();
    map.insert(
        "$schema".to_string(),
        serde_json::Value::String(RELATION_OBJECT_SCHEMA_URL.to_string()),
    );
    if let serde_json::Value::Object(obj) = value {
        map.extend(obj);
    }
    Ok(serde_json::Value::Object(map))
}

/// Parse a standalone relation object: `$schema` is required and const-pinned
/// (RFC-038 Change E); the remaining properties are exactly the Relation shape
/// (`deny_unknown_fields` enforces `additionalProperties: false`).
pub(crate) fn relation_object_from_value(
    mut value: serde_json::Value,
    path: &str,
) -> Result<srs_core::types::relation::Relation, RepositoryError> {
    match value.as_object_mut().and_then(|o| o.remove("$schema")) {
        Some(schema) if schema.as_str() == Some(RELATION_OBJECT_SCHEMA_URL) => {}
        Some(schema) => {
            return Err(RepositoryError::SchemaValidation {
                path: PathBuf::from(path),
                message: format!(
                    "relation object $schema must be '{RELATION_OBJECT_SCHEMA_URL}', found {schema}"
                ),
            });
        }
        None => {
            return Err(RepositoryError::SchemaValidation {
                path: PathBuf::from(path),
                message: format!(
                    "relation object is missing the required $schema ('{RELATION_OBJECT_SCHEMA_URL}')"
                ),
            });
        }
    }
    serde_json::from_value(value).map_err(|source| RepositoryError::RecordLoad {
        path: PathBuf::from(path),
        source,
    })
}

// ---------------------------------------------------------------------------
// RepositoryStore trait
// ---------------------------------------------------------------------------

/// Abstracts all I/O operations performed by service functions.
///
/// Service functions accept `&dyn RepositoryStore` so the storage backend
/// (filesystem, SQLite, in-memory) can be swapped without touching service logic.
/// All path arguments are *relative* to the repository root so that
/// implementations can resolve them however they choose.
pub trait RepositoryStore {
    // --- Repository lifecycle ---

    fn repository_root(&self) -> PathBuf;
    fn repository_exists(&self) -> Result<bool, RepositoryError>;
    fn initialize_repository(
        &self,
        input: &InitializeRepositoryInput,
    ) -> Result<CreateRepositoryResult, RepositoryError>;

    // --- Manifest ---

    fn load_manifest(&self) -> Result<Manifest, RepositoryError>;
    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError>;

    // --- Batch write mode ---
    //
    // Optional opt-in for stores that benefit from deferred flushing during
    // bulk operations (e.g. JsonStore during import_repository_snapshot).
    // Default implementations are no-ops so FileStore and MemoryStore require
    // no changes. See ADR-021.

    /// Signal that a bulk write operation is starting. Stores that support
    /// batch mode may defer disk writes until `commit_batch` is called.
    fn begin_batch(&self) {}

    /// Flush all deferred writes atomically. Called after a successful bulk
    /// operation. The default no-op is correct for stores that flush eagerly.
    fn commit_batch(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    /// Abandon deferred writes without flushing. Called when a bulk operation
    /// fails. The on-disk state reverts to what it was before `begin_batch`.
    fn abort_batch(&self) {}

    // --- Package (read) ---

    fn load_package(&self) -> Result<Package, RepositoryError>;

    // --- Package index (package.json raw) ---

    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError>;
    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError>;

    // --- Fields ---

    fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError>;
    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError>;
    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_fields_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Types ---

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError>;
    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError>;
    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Relation type definitions ---

    fn save_relation_type_definition(
        &self,
        relative_path: &str,
        relation_type: &RelationTypeDefinition,
    ) -> Result<(), RepositoryError>;
    fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_relation_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Views (L1) ---

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError>;
    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Document Views (L2) ---

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError>;
    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError>;
    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_document_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Themes ---

    fn save_theme(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError>;
    fn update_theme_file(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError>;
    fn delete_theme_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_themes_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Blueprints ---

    fn save_blueprint(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError>;
    fn update_blueprint_file(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError>;
    fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_blueprints_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Vocabularies ---

    fn save_vocabulary(
        &self,
        relative_path: &str,
        vocabulary: &Vocabulary,
    ) -> Result<(), RepositoryError>;
    fn ensure_vocabularies_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Lifecycles ---

    fn save_lifecycle(
        &self,
        relative_path: &str,
        lifecycle: &Lifecycle,
    ) -> Result<(), RepositoryError>;
    fn ensure_lifecycles_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;

    // --- Instances (Notes, TypedRecords, Records) ---

    fn load_instance_json(&self, relative_path: &str)
        -> Result<serde_json::Value, RepositoryError>;
    fn save_instance_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    fn ensure_instance_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
    /// Returns relative paths of all JSON files directly under `relative_dir`.
    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError>;

    /// Returns the relative directory for instance records of the given tier.
    ///
    /// Required — each adapter declares its own layout explicitly.
    fn record_tier_dir(&self, tier: RecordTier) -> &'static str;

    // --- Instances (logical-id + typed; ADR-042) ---
    //
    // The typed, logical-id-keyed instance surface (ADR-041 G3–G5). These
    // address instances by `instance_id`, not by path — service code no longer
    // walks `manifest.instance_index` by `InstanceIndexEntry.path`. The Value/path
    // methods above are retained transitionally as a generic JSON shim (see #726);
    // do not use them for instance persistence in new code.
    //
    // Tier is derived from the runtime type: `Note` → Tier 0, `Record` → Tier 2.

    /// Persist a `Record` (Tier 2) by its logical id. Mirrors `save_container`'s
    /// two branches: an existing id overwrites the entity **at its existing indexed
    /// path** (path + tier preserved, no rename) and refreshes the index entry's
    /// denormalized `tags`; a new id derives a collision-safe filename and writes
    /// the **entity before the index entry** (ADR-007).
    fn save_record(&self, record: &Record) -> Result<(), RepositoryError>;

    /// Persist a `Note` (Tier 0) by its logical id. Same two-branch shape as
    /// [`save_record`](Self::save_record); refreshes the index entry's `title`/`tags`.
    fn save_note(&self, note: &Note) -> Result<(), RepositoryError>;

    /// Load a `Record` by its logical id. Returns `InstanceNotFound` if no instance
    /// with that id is indexed, or `RecordLoad` if the stored bytes fail to parse.
    fn load_record_by_id(&self, instance_id: &str) -> Result<Record, RepositoryError>;

    /// Load a `Note` by its logical id. Returns `InstanceNotFound` if no instance
    /// with that id is indexed.
    fn load_note_by_id(&self, instance_id: &str) -> Result<Note, RepositoryError>;

    /// Delete an instance by its logical id, removing the **index entry before the
    /// entity file** (ADR-007 index-first on delete). Returns `InstanceNotFound`
    /// for an unknown id.
    ///
    /// Distinct from the transitional generic-shim `delete_instance_file(path)` above:
    /// this is keyed by logical id and maintains the index.
    fn delete_instance(&self, instance_id: &str) -> Result<(), RepositoryError>;

    /// Look up one instance's index-answerable summary by id, or `None` if absent.
    fn find_instance(&self, instance_id: &str) -> Result<Option<InstanceRef>, RepositoryError>;

    /// Enumerate instances matching `query`, answered from the index without
    /// loading entity bodies (G5). Order is not guaranteed.
    fn list_instances(&self, query: &InstanceQuery) -> Result<Vec<InstanceRef>, RepositoryError>;

    // --- Relations ---

    fn load_relations_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError>;
    fn save_relations_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    fn ensure_relations_dir(&self, relative_dir: &str) -> Result<(), RepositoryError>;
    /// Delete one file under `relations/` by relative path. Generic seam consumed
    /// by the typed default methods below; idempotent on a missing file.
    fn delete_relations_json(&self, relative_path: &str) -> Result<(), RepositoryError>;

    // --- Relations (logical-id + typed; RFC-038 Change E, ADR-042 template) ---
    //
    // One standalone object per relation at `relations/<relationId>.json`, with the
    // `$schema` const-pinned to [`RELATION_OBJECT_SCHEMA_URL`]. The filename is a
    // locator, not the identity: the in-file `relationId` is authoritative, and a
    // filename that disagrees with it is an error naming both ([R11]). Enumeration
    // is ascending byte-wise `relationId`; enumeration order carries no meaning —
    // `precedes` is the only ordering semantics.
    //
    // These are default methods over the generic relations-JSON seam so FileStore
    // (Disk and MemVfs), MemoryStore, and JsonStore behave identically. JsonStore's
    // real treatment (codec collapse) is RFC-038 Phase 4.
    // phase-3: route duplicate-id detection and enumeration via RepositoryCatalog
    // once the Phase-1 catalog seam lands.

    /// Persist one relation as a standalone object at `relations/<relationId>.json`.
    /// Writes only that file. Overwrites an existing object with the same id.
    fn save_relation(
        &self,
        relation: &srs_core::types::relation::Relation,
    ) -> Result<(), RepositoryError> {
        if relation.relation_id.trim().is_empty() {
            return Err(RepositoryError::RelationValidation {
                relation_id: String::new(),
                message: "relationId must be non-empty to persist a relation object".to_string(),
            });
        }
        let path = relation_object_path(&relation.relation_id);
        let value = relation_object_to_value(relation, &path)?;
        self.ensure_relations_dir("relations")?;
        self.save_relations_json(&path, &value)
    }

    /// Load one relation by its logical id from `relations/<relationId>.json`.
    /// Returns `RelationNotFound` if the object does not exist, and
    /// `RelationFilenameMismatch` if the in-file `relationId` disagrees ([R11]).
    fn load_relation(
        &self,
        relation_id: &str,
    ) -> Result<srs_core::types::relation::Relation, RepositoryError> {
        let path = relation_object_path(relation_id);
        let value = self.load_relations_json(&path).map_err(|e| {
            if e.is_not_found() {
                RepositoryError::RelationNotFound {
                    relation_id: relation_id.to_string(),
                }
            } else {
                e
            }
        })?;
        let relation = relation_object_from_value(value, &path)?;
        if relation.relation_id != relation_id {
            return Err(RepositoryError::RelationFilenameMismatch {
                path: PathBuf::from(path),
                file_relation_id: relation.relation_id,
            });
        }
        Ok(relation)
    }

    /// Delete one relation object by its logical id. Touches only its own file.
    /// Returns `RelationNotFound` if the object does not exist.
    fn delete_relation(&self, relation_id: &str) -> Result<(), RepositoryError> {
        // Existence check first so a missing relation is a typed error.
        self.load_relation(relation_id)?;
        self.delete_relations_json(&relation_object_path(relation_id))
    }

    /// Enumerate all standalone relation objects under `relations/`, ascending by
    /// `relationId` (byte-wise over the id string).
    ///
    /// Transitional (removed at the RFC-038 Phase-6 flip): collection-shaped files
    /// (a top-level `relations` array — `relations.json`, `relations-collection.json`,
    /// or a manifest-declared `relationsPath`) are skipped here; the service-level
    /// dual read in `relation_service` merges them in.
    fn list_relations(&self) -> Result<Vec<srs_core::types::relation::Relation>, RepositoryError> {
        let mut out = Vec::new();
        for path in self.list_files_recursive("relations") {
            if !path.ends_with(".json") {
                continue;
            }
            let value = self.load_relations_json(&path)?;
            if value.get("relations").is_some() {
                continue; // collection form — handled by the transitional dual read
            }
            let relation = relation_object_from_value(value, &path)?;
            let stem = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".json");
            if relation.relation_id != stem {
                return Err(RepositoryError::RelationFilenameMismatch {
                    path: PathBuf::from(&path),
                    file_relation_id: relation.relation_id,
                });
            }
            out.push(relation);
        }
        out.sort_by(|a, b| a.relation_id.cmp(&b.relation_id));
        Ok(out)
    }

    // --- Containers ---

    /// Load a container by its logical `container_id`.
    /// Returns `ContainerNotFound` if no container with that ID is registered.
    fn load_container(
        &self,
        container_id: &str,
    ) -> Result<srs_core::types::container::Container, RepositoryError>;

    /// Persist a container by its logical `container_id`.
    /// Creates it if it does not exist; overwrites if it does.
    /// Implementations must write entity data before updating any index (ADR-007).
    fn save_container(
        &self,
        container: &srs_core::types::container::Container,
    ) -> Result<(), RepositoryError>;

    /// Delete a container by its logical `container_id`.
    /// Returns `ContainerNotFound` if no container with that ID is registered.
    fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError>;

    /// List all containers as lightweight summaries `(container_id, title)`.
    /// Order is not guaranteed.
    fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError>;

    // --- Containers (transitional path-based methods — do not use in new service code) ---

    #[deprecated(note = "Use load_container instead")]
    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError>;
    #[deprecated(note = "Use save_container instead")]
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError>;
    #[deprecated(note = "Use delete_container instead")]
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError>;
    #[deprecated(note = "No-op in logical container model; remove call sites")]
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError>;

    // --- Package boundaries ---

    /// Return metadata for all package boundaries (primary + all sub-packages).
    fn list_package_boundaries(&self) -> Result<Vec<PackageBoundary>, RepositoryError>;

    /// Return metadata for one boundary. Returns `PackageNotFound` if missing.
    fn load_package_boundary(
        &self,
        selector: &PackageSelector,
    ) -> Result<PackageBoundary, RepositoryError>;

    /// Persist id/namespace/name/version for one boundary.
    /// Creates the boundary's `package.json` if it does not exist.
    fn save_package_boundary_metadata(
        &self,
        boundary: &PackageBoundary,
    ) -> Result<(), RepositoryError>;

    /// Register a boundary in the manifest's packageRefs (no-op for primary).
    /// No-op if already registered.
    fn register_package_boundary(&self, selector: &PackageSelector) -> Result<(), RepositoryError>;

    /// Add a definition path to a boundary's index (e.g. `"fields/foo.json"`).
    fn add_definition_to_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError>;

    /// Remove a definition path from a boundary's index.
    fn remove_definition_from_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError>;

    /// Find which boundary owns a field or type by ID.
    ///
    /// **Implementation note:** This is an O(n×m) linear scan in file-backed
    /// and in-memory stores (walks each boundary, loads each definition file
    /// and compares the `id` field). SQL adapters may maintain an index.
    fn resolve_definition_owner(
        &self,
        id: &str,
        kind: DefinitionKind,
    ) -> Result<PackageSelector, RepositoryError>;

    // --- Sub-package path validation ---

    /// List all files under `relative_dir` recursively, returning relative paths.
    /// Returns an empty Vec if the directory does not exist.
    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String>;

    /// Return true if any addressability revision sidecar (`.revisions.json`) exists.
    fn has_revision_sidecars(&self) -> bool {
        self.list_files_recursive("records")
            .iter()
            .any(|p| p.ends_with(".revisions.json"))
    }

    /// Read a text file at `relative_path` and return its contents.
    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError>;

    /// Write `content` to `relative_path`, creating parent directories as needed.
    fn save_text_file(&self, relative_path: &str, content: &str) -> Result<(), RepositoryError>;

    /// Read raw bytes from `relative_path`.
    fn load_binary_file(&self, relative_path: &str) -> Result<Vec<u8>, RepositoryError>;

    /// Return the byte length of the file at `relative_path` without necessarily
    /// reading the full content. The default reads the file; `FileStore` overrides
    /// with `std::fs::metadata` to avoid loading large binary content during validation.
    fn file_byte_len(&self, relative_path: &str) -> Result<u64, RepositoryError> {
        Ok(self.load_binary_file(relative_path)?.len() as u64)
    }

    /// Write raw bytes to `relative_path`, creating parent directories as needed.
    fn save_binary_file(&self, relative_path: &str, content: &[u8]) -> Result<(), RepositoryError>;

    /// A verbatim snapshot of the store's file tree, when the backend is an
    /// in-memory tree session (ADR-038/038). `None` for every other store.
    fn as_tree_snapshot(&self) -> Option<std::collections::BTreeMap<String, Vec<u8>>> {
        None
    }

    /// True when the store persists files verbatim as a tree (`FileStore` over
    /// any Vfs backend). The native archive unpack writes raw files into such
    /// stores without snapshot re-canonicalization (ADR-039).
    fn is_file_tree_store(&self) -> bool {
        false
    }

    /// Verify that `relative_path` (relative to repo root) points to a directory
    /// containing a `package.json`.
    ///
    /// Contract:
    ///   - `FileStore`: resolves against `repo_root`, checks the directory and
    ///     `package.json` exist, returns `PackageRefMissing` if not.
    ///   - `MemoryStore`: returns `Ok(())` unconditionally — path existence is
    ///     not meaningful in memory.
    fn validate_package_ref_path(&self, relative_path: &str) -> Result<(), RepositoryError>;

    fn load_manifest_raw_text(&self) -> Result<String, RepositoryError> {
        self.load_text_file("manifest.json")
    }

    fn load_primary_package_raw_text(&self) -> Result<String, RepositoryError> {
        self.load_text_file("package/package.json")
    }

    /// Returns `None` if no relations file exists.
    /// Tries `relations/relations-collection.json` first (canonical write path),
    /// then `relations/relations.json` (legacy alternate convention).
    fn load_relations_raw_text(&self) -> Result<Option<String>, RepositoryError> {
        match self.load_text_file("relations/relations-collection.json") {
            Ok(s) => return Ok(Some(s)),
            Err(e) if e.is_not_found() => {}
            Err(e) => return Err(e),
        }
        match self.load_text_file("relations/relations.json") {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    // --- Catalog (RFC-038 Change L / [R23]; srs-rust#783 Phase 1) ---
    //
    // The store enumeration seam: one materialised snapshot of the six
    // authoritative sets, path-free identity, diagnostics carried with the
    // result. [R24] applies: an `error` diagnostic under a reserved location
    // fails the load as a whole (`RepositoryError::CatalogLoad` carries the
    // complete diagnostic list). Object-safe and sync (ADR-041 G1/G7).
    //
    // Defaults return `CatalogUnsupported` so stores outside the contract
    // (JsonStore, pending its Phase-4 collapse into a codec) compile
    // unchanged; FileStore (over any Vfs) and MemoryStore override with the
    // one shared walker in `crate::catalog`.

    /// Enumerate the repository into a [`crate::catalog::RepositoryCatalog`].
    fn catalog(&self) -> Result<crate::catalog::RepositoryCatalog, RepositoryError> {
        Err(RepositoryError::CatalogUnsupported)
    }

    /// [R16] validity token: a content digest over the enumerated id set.
    /// Changes whenever the enumerable id set changes; a cached catalog is
    /// served only while its token matches the store's current one.
    fn catalog_validity_token(&self) -> Result<String, RepositoryError> {
        Err(RepositoryError::CatalogUnsupported)
    }
}

// ---------------------------------------------------------------------------
// FileStore — file-backed implementation
// ---------------------------------------------------------------------------

/// File-backed implementation of [`RepositoryStore`].
///
/// All I/O is funnelled through the [`Vfs`] seam (ADR-038): `DiskVfs` for the
/// CLI's on-disk repositories, `MemVfs` for in-memory tree sessions (WASM).
/// Service functions must not import `std::fs` directly.
#[derive(Debug, Clone)]
pub struct FileStore {
    /// Display-only root: feeds `repository_root()` and error paths. All I/O
    /// goes through `vfs`. MemVfs-backed stores use the `"<memory>"` sentinel
    /// (ADR-021 convention).
    repo_root: PathBuf,
    vfs: Rc<dyn Vfs>,
}

impl FileStore {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        let repo_root = repo_root.into();
        let vfs = Rc::new(DiskVfs::new(repo_root.clone()));
        Self { repo_root, vfs }
    }

    /// Construct over an arbitrary [`Vfs`] (e.g. `MemVfs` for tree sessions).
    /// `repository_root()` reports the `"<memory>"` sentinel.
    pub fn from_vfs(vfs: Rc<dyn Vfs>) -> Self {
        Self {
            repo_root: PathBuf::from("<memory>"),
            vfs,
        }
    }

    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    pub(crate) fn vfs(&self) -> &dyn Vfs {
        self.vfs.as_ref()
    }

    /// Display path for error reporting only — never used for I/O.
    fn abs(&self, relative_path: &str) -> PathBuf {
        self.repo_root.join(relative_path)
    }

    fn read_json(&self, rel: &str) -> Result<serde_json::Value, RepositoryError> {
        let content = self.vfs.read_to_string(rel)?;
        serde_json::from_str(&content).map_err(|e| RepositoryError::Serialize {
            path: self.abs(rel),
            source: e,
        })
    }

    fn write_json(&self, rel: &str, value: &serde_json::Value) -> Result<(), RepositoryError> {
        let json = serde_json::to_string_pretty(value).map_err(|e| RepositoryError::Serialize {
            path: self.abs(rel),
            source: e,
        })?;
        self.vfs.write(rel, json.as_bytes())
    }

    fn ensure_dir(&self, rel: &str) -> Result<(), RepositoryError> {
        self.vfs.create_dir_all(rel)
    }

    fn delete_file(&self, rel: &str) -> Result<(), RepositoryError> {
        self.vfs.remove(rel)
    }
}

// --- Package loading helpers (private to FileStore) ---

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageMetadata {
    id: String,
    namespace: String,
    name: String,
    version: String,
    #[serde(default)]
    fields: Vec<String>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    relation_types: Vec<String>,
    #[serde(default)]
    views: Vec<String>,
    #[serde(default)]
    document_views: Vec<String>,
    #[serde(default)]
    themes: Vec<String>,
    #[serde(default)]
    blueprints: Vec<String>,
    #[serde(default)]
    protocols: Vec<String>,
    #[serde(default)]
    dependency_refs: Vec<crate::package::DependencyRef>,
    #[serde(default)]
    vocabularies: Vec<String>,
    #[serde(default)]
    lifecycles: Vec<String>,
}

#[allow(clippy::type_complexity)]
fn load_package_from_dir(
    vfs: &dyn Vfs,
    prefix: &str,
    err_root: &std::path::Path,
    rt_by_type: &mut HashMap<String, (RelationTypeDefinition, PathBuf)>,
) -> Result<
    (
        Vec<Field>,
        Vec<RecordType>,
        Vec<View>,
        Vec<DocumentView>,
        Vec<Theme>,
        Vec<crate::package::LoadedBlueprint>,
        Vec<crate::package::LoadedProtocol>,
        Vec<Lifecycle>,
    ),
    RepositoryError,
> {
    let package_json_rel = vfs_join(prefix, "package.json");
    let package_content = vfs.read_to_string(&package_json_rel)?;
    let metadata: PackageMetadata =
        serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
            path: err_root.join(&package_json_rel),
            source: e,
        })?;

    let mut fields = Vec::new();
    for field_path in &metadata.fields {
        let rel = vfs_join(prefix, field_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let fj: FieldJson =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;
        fields.push(fj.into_field(&full_path)?);
    }

    let mut record_types = Vec::new();
    for type_path in &metadata.types {
        let rel = vfs_join(prefix, type_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let tj: crate::type_json::TypeJson =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;
        record_types.push(tj.into_record_type());
    }

    for rt_path in &metadata.relation_types {
        let rel = vfs_join(prefix, rt_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let def: RelationTypeDefinition =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source: e,
            })?;
        validate_relation_type_definition(&def).map_err(|source| {
            RepositoryError::RelationTypeDefinitionValidation {
                path: full_path.clone(),
                source,
            }
        })?;
        if let Some((existing, existing_path)) = rt_by_type.get(&def.key) {
            if existing != &def {
                return Err(RepositoryError::RelationTypeDefinitionConflict {
                    relation_type: def.key.clone(),
                    path_a: existing_path.clone(),
                    path_b: full_path,
                });
            }
        } else {
            rt_by_type.insert(def.key.clone(), (def, full_path));
        }
    }

    let mut views = Vec::new();
    for view_path in &metadata.views {
        let rel = vfs_join(prefix, view_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let view: View =
            serde_json::from_str(&content).map_err(|source| RepositoryError::ViewLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_view(&view).map_err(|source| RepositoryError::ViewValidation {
            path: full_path.clone(),
            source,
        })?;
        views.push(view);
    }

    let mut document_views = Vec::new();
    for dv_path in &metadata.document_views {
        let rel = vfs_join(prefix, dv_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let dv: DocumentView =
            serde_json::from_str(&content).map_err(|source| RepositoryError::DocumentViewLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_document_view(&dv).map_err(|source| RepositoryError::DocumentViewValidation {
            path: full_path.clone(),
            source,
        })?;
        document_views.push(dv);
    }

    let mut themes = Vec::new();
    for theme_path in &metadata.themes {
        let rel = vfs_join(prefix, theme_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let theme: Theme =
            serde_json::from_str(&content).map_err(|source| RepositoryError::ThemeLoad {
                path: full_path.clone(),
                source,
            })?;
        validate_theme(&theme).map_err(|source| RepositoryError::ThemeValidation {
            path: full_path.clone(),
            source,
        })?;
        themes.push(theme);
    }

    let mut blueprints: Vec<crate::package::LoadedBlueprint> = Vec::new();
    for blueprint_path in &metadata.blueprints {
        let rel = vfs_join(prefix, blueprint_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let blueprint: srs_core::types::blueprint::Blueprint = serde_json::from_str(&content)
            .map_err(|source| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source,
            })?;
        blueprints.push(crate::package::LoadedBlueprint {
            blueprint,
            source_package: None,
        });
    }

    let mut protocols: Vec<crate::package::LoadedProtocol> = Vec::new();
    for protocol_path in &metadata.protocols {
        let rel = vfs_join(prefix, protocol_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let raw: serde_json::Value =
            serde_json::from_str(&content).map_err(|source| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source,
            })?;
        let protocol: srs_core::types::protocol::Protocol = serde_json::from_value(raw.clone())
            .map_err(|source| RepositoryError::PackageLoad {
                path: full_path.clone(),
                source,
            })?;
        protocols.push(crate::package::LoadedProtocol {
            protocol,
            raw,
            source_package: None,
        });
    }

    let mut lifecycles: Vec<Lifecycle> = Vec::new();
    for lc_path in &metadata.lifecycles {
        let rel = vfs_join(prefix, lc_path);
        let full_path = err_root.join(&rel);
        let content = vfs.read_to_string(&rel)?;
        let lc: Lifecycle =
            serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                path: full_path,
                source: e,
            })?;
        lifecycles.push(lc);
    }

    Ok((
        fields,
        record_types,
        views,
        document_views,
        themes,
        blueprints,
        protocols,
        lifecycles,
    ))
}

impl RepositoryStore for FileStore {
    fn repository_root(&self) -> PathBuf {
        self.repo_root.clone()
    }

    fn repository_exists(&self) -> Result<bool, RepositoryError> {
        Ok(self.vfs.is_dir(SRS_MARKER_DIR)
            && self.vfs.is_file("manifest.json")
            && self.vfs.is_file("package/package.json"))
    }

    fn initialize_repository(
        &self,
        input: &InitializeRepositoryInput,
    ) -> Result<CreateRepositoryResult, RepositoryError> {
        if self.repository_exists()? {
            return Err(RepositoryError::RepositoryAlreadyExists {
                path: self.repo_root.clone(),
            });
        }

        self.ensure_dir(SRS_MARKER_DIR)?;
        self.ensure_dir("package")?;

        let title = input
            .repository
            .title
            .as_deref()
            .unwrap_or_default()
            .to_string();
        let created_at = chrono::Utc::now().to_rfc3339();
        let container_value = serde_json::to_value(default_repository_container(
            &input.repository.repository_id,
            &title,
        ))
        .unwrap_or_default();
        let mut manifest = serde_json::json!({
            "$schema": srs_schema::MANIFEST_SCHEMA_ID,
            "instanceIndex": [],
            "srsVersion": input.repository.srs_version,
            "repositoryId": input.repository.repository_id,
            "namespace": input.repository.namespace,
            "title": title,
            "container": container_value,
            "createdAt": created_at
        });
        if let Some(desc) = &input.repository.description {
            manifest["description"] = serde_json::Value::String(desc.clone());
        }
        self.write_json("manifest.json", &manifest)?;

        let package = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
            "id": input.primary_package.id,
            "namespace": input.primary_package.namespace,
            "name": input.primary_package.name,
            "version": input.primary_package.version,
            "title": input.primary_package.name,
            "description": "",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [],
            "types": [],
            "relationTypes": [],
            "views": [],
            "documentViews": [],
            "blueprints": []
        });
        self.write_json("package/package.json", &package)?;

        Ok(CreateRepositoryResult {
            repo_root: self.repo_root.clone(),
            repository_id: input.repository.repository_id.clone(),
            package_id: input.primary_package.id.clone(),
            identity_instance_id: None,
        })
    }

    // --- Manifest ---

    fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
        let manifest_path = self.abs("manifest.json");
        if !self.vfs.is_file("manifest.json") {
            return Err(RepositoryError::ManifestMissing {
                path: manifest_path,
            });
        }
        let content = self.vfs.read_to_string("manifest.json")?;
        let mut raw: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| RepositoryError::ManifestParse {
                path: manifest_path.clone(),
                source: e,
            })?;
        crate::manifest::migrate_upstream_package(&mut raw);
        let mut manifest: Manifest =
            serde_json::from_value(raw).map_err(|e| RepositoryError::ManifestParse {
                path: manifest_path.clone(),
                source: e,
            })?;
        manifest.root = self.repo_root.clone();
        Ok(manifest)
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(manifest).map_err(|e| RepositoryError::Serialize {
            path: self.abs("manifest.json"),
            source: e,
        })?;
        self.write_json("manifest.json", &value)
    }

    // --- Package ---

    fn load_package(&self) -> Result<Package, RepositoryError> {
        let package_dir = self.repo_root.join("package");

        let package_content = self.vfs.read_to_string("package/package.json")?;
        let metadata: PackageMetadata =
            serde_json::from_str(&package_content).map_err(|e| RepositoryError::PackageLoad {
                path: self.abs("package/package.json"),
                source: e,
            })?;

        let mut rt_by_type: HashMap<String, (RelationTypeDefinition, PathBuf)> = HashMap::new();
        let (
            mut fields,
            mut record_types,
            mut views,
            mut document_views,
            mut themes,
            mut blueprints,
            mut protocols,
            mut lifecycles,
        ) = load_package_from_dir(self.vfs(), "package", &self.repo_root, &mut rt_by_type)?;

        // Merge sub-packages from manifest packageRefs
        let manifest = self.load_manifest()?;
        if let Some(pkg_refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            let mut field_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut type_sources: HashMap<(String, u32), PathBuf> = HashMap::new();
            let mut view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut doc_view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut theme_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut blueprint_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut protocol_sources: HashMap<String, PathBuf> = HashMap::new();
            for f in &fields {
                field_sources.insert(f.id.clone(), package_dir.clone());
            }
            for rt in &record_types {
                type_sources.insert((rt.id.clone(), rt.version), package_dir.clone());
            }
            for v in &views {
                view_sources.insert(v.id.clone(), package_dir.clone());
            }
            for dv in &document_views {
                doc_view_sources.insert(dv.id.clone(), package_dir.clone());
            }
            for theme in &themes {
                theme_sources.insert(theme.id.clone(), package_dir.clone());
            }
            for lb in &blueprints {
                blueprint_sources.insert(lb.blueprint.id.clone(), package_dir.clone());
            }
            for lp in &protocols {
                protocol_sources.insert(lp.protocol.protocol_id.clone(), package_dir.clone());
            }

            for pkg_ref in pkg_refs {
                let mode = pkg_ref.get("mode").and_then(|m| m.as_str()).unwrap_or("");
                if mode != "local" {
                    continue;
                }
                let rel_path = match pkg_ref.get("path").and_then(|p| p.as_str()) {
                    Some(p) => p,
                    None => continue,
                };
                let sub_dir = self.repo_root.join(rel_path);
                if !self.vfs.is_file(&vfs_join(rel_path, "package.json")) {
                    return Err(RepositoryError::PackageRefMissing {
                        path: rel_path.to_string(),
                    });
                }
                let (
                    sub_fields,
                    sub_types,
                    sub_views,
                    sub_doc_views,
                    sub_themes,
                    sub_blueprints,
                    sub_protocols,
                    sub_lifecycles,
                ) = load_package_from_dir(self.vfs(), rel_path, &self.repo_root, &mut rt_by_type)?;

                for field in sub_fields {
                    if let Some(first_path) = field_sources.get(&field.id) {
                        let existing = fields.iter().find(|f| f.id == field.id).unwrap();
                        if existing.version != field.version
                            || existing.namespace != field.namespace
                            || existing.name != field.name
                        {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "field".to_string(),
                                id: field.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        field_sources.insert(field.id.clone(), sub_dir.clone());
                        fields.push(field);
                    }
                }
                for rt in sub_types {
                    let key = (rt.id.clone(), rt.version);
                    if let Some(first_path) = type_sources.get(&key) {
                        let existing = record_types
                            .iter()
                            .find(|r| r.id == rt.id && r.version == rt.version)
                            .unwrap();
                        if existing.namespace != rt.namespace || existing.name != rt.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "type".to_string(),
                                id: rt.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        type_sources.insert(key, sub_dir.clone());
                        record_types.push(rt);
                    }
                }
                for view in sub_views {
                    if let Some(first_path) = view_sources.get(&view.id) {
                        let existing = views.iter().find(|v| v.id == view.id).unwrap();
                        if existing.name != view.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "view".to_string(),
                                id: view.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        view_sources.insert(view.id.clone(), sub_dir.clone());
                        views.push(view);
                    }
                }
                for dv in sub_doc_views {
                    if let Some(first_path) = doc_view_sources.get(&dv.id) {
                        let existing = document_views.iter().find(|d| d.id == dv.id).unwrap();
                        if existing.name != dv.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "document-view".to_string(),
                                id: dv.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        doc_view_sources.insert(dv.id.clone(), sub_dir.clone());
                        document_views.push(dv);
                    }
                }
                for theme in sub_themes {
                    if let Some(first_path) = theme_sources.get(&theme.id) {
                        let existing = themes.iter().find(|t| t.id == theme.id).unwrap();
                        if existing.namespace != theme.namespace
                            || existing.name != theme.name
                            || existing.version != theme.version
                        {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "theme".to_string(),
                                id: theme.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        theme_sources.insert(theme.id.clone(), sub_dir.clone());
                        themes.push(theme);
                    }
                }
                for mut lb in sub_blueprints {
                    if let Some(first_path) = blueprint_sources.get(&lb.blueprint.id) {
                        let existing = blueprints
                            .iter()
                            .find(|b| b.blueprint.id == lb.blueprint.id)
                            .unwrap();
                        if existing.blueprint.name != lb.blueprint.name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "blueprint".to_string(),
                                id: lb.blueprint.id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        lb.source_package = Some(rel_path.to_string());
                        blueprint_sources.insert(lb.blueprint.id.clone(), sub_dir.clone());
                        blueprints.push(lb);
                    }
                }
                for mut lp in sub_protocols {
                    if let Some(first_path) = protocol_sources.get(&lp.protocol.protocol_id) {
                        let existing = protocols
                            .iter()
                            .find(|p| p.protocol.protocol_id == lp.protocol.protocol_id)
                            .unwrap();
                        if existing.protocol.protocol_name != lp.protocol.protocol_name {
                            return Err(RepositoryError::PackageRefConflict {
                                path: rel_path.to_string(),
                                kind: "protocol".to_string(),
                                id: lp.protocol.protocol_id.clone(),
                                first_path: first_path.clone(),
                                second_path: sub_dir.clone(),
                            });
                        }
                    } else {
                        lp.source_package = Some(rel_path.to_string());
                        protocol_sources.insert(lp.protocol.protocol_id.clone(), sub_dir.clone());
                        protocols.push(lp);
                    }
                }
                for lc in sub_lifecycles {
                    // First occurrence wins (same policy as other definition kinds).
                    if !lifecycles.iter().any(|existing| existing.id == lc.id) {
                        lifecycles.push(lc);
                    }
                }
            }
        }

        // `rt_by_type` is a HashMap, whose iteration order is randomized per
        // process. Sort by (key, id) so the resulting Vec — and anything derived
        // from it, e.g. the regenerated package.json `relationTypes` index in
        // `repo copy` — is deterministic across runs.
        let mut relation_type_definitions: Vec<RelationTypeDefinition> =
            rt_by_type.into_values().map(|(def, _)| def).collect();
        relation_type_definitions.sort_by(|a, b| a.key.cmp(&b.key).then(a.id.cmp(&b.id)));

        let mut vocabularies: Vec<Vocabulary> = Vec::new();
        for vocab_path in &metadata.vocabularies {
            let rel = vfs_join("package", vocab_path);
            let full_path = package_dir.join(vocab_path);
            let content = self.vfs.read_to_string(&rel)?;
            let vocab: Vocabulary =
                serde_json::from_str(&content).map_err(|e| RepositoryError::PackageLoad {
                    path: full_path,
                    source: e,
                })?;
            vocabularies.push(vocab);
        }

        crate::core_package::merge_core_into_package(
            &mut fields,
            &mut record_types,
            &mut relation_type_definitions,
        )?;

        Ok(Package {
            id: metadata.id,
            namespace: metadata.namespace,
            name: metadata.name,
            version: metadata.version,
            fields,
            record_types,
            relation_type_definitions,
            views,
            document_views,
            themes,
            blueprints,
            protocols,
            root: self.repo_root.clone(),
            dependency_refs: metadata.dependency_refs.clone(),
            vocabularies,
            lifecycles,
        })
    }

    // --- Package JSON ---

    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
        self.read_json("package/package.json")
    }

    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError> {
        self.write_json("package/package.json", value)
    }

    // --- Fields ---

    fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(field).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        self.save_field(relative_path, field)
    }

    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_fields_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    // --- Types ---

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(record_type).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        self.save_type(relative_path, record_type)
    }

    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    fn save_relation_type_definition(
        &self,
        relative_path: &str,
        relation_type: &RelationTypeDefinition,
    ) -> Result<(), RepositoryError> {
        let value =
            serde_json::to_value(relation_type).map_err(|e| RepositoryError::Serialize {
                path: self.abs(relative_path),
                source: e,
            })?;
        self.write_json(relative_path, &value)
    }

    fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_relation_types_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    // --- Views (L1) ---

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(view).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        self.save_view(relative_path, view)
    }

    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    // --- Document Views (L2) ---

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(view).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        self.save_document_view(relative_path, view)
    }

    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_document_views_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    // --- Themes ---

    fn save_theme(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(theme).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn update_theme_file(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError> {
        self.save_theme(relative_path, theme)
    }

    fn delete_theme_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_themes_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    // --- Blueprints ---

    fn save_blueprint(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(blueprint).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn update_blueprint_file(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        self.save_blueprint(relative_path, blueprint)
    }

    fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_blueprints_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    fn save_vocabulary(
        &self,
        relative_path: &str,
        vocabulary: &Vocabulary,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(vocabulary).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn ensure_vocabularies_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    fn save_lifecycle(
        &self,
        relative_path: &str,
        lifecycle: &Lifecycle,
    ) -> Result<(), RepositoryError> {
        let value = serde_json::to_value(lifecycle).map_err(|e| RepositoryError::Serialize {
            path: self.abs(relative_path),
            source: e,
        })?;
        self.write_json(relative_path, &value)
    }

    fn ensure_lifecycles_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    // --- Instances ---

    fn load_instance_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(relative_path)
    }

    fn save_instance_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(relative_path, value)
    }

    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    fn ensure_instance_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError> {
        let mut paths = Vec::new();
        for entry in self.vfs.list_dir(relative_dir)? {
            if !entry.is_dir && entry.name.ends_with(".json") {
                paths.push(vfs_join(relative_dir, &entry.name));
            }
        }
        Ok(paths)
    }

    fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
        tier.dir()
    }

    // --- Instances (logical-id + typed; ADR-042) ---

    fn save_record(&self, record: &Record) -> Result<(), RepositoryError> {
        let val = record_to_value(record)?;
        let tier_dir = self.record_tier_dir(RecordTier::Tier2);
        self.ensure_dir(tier_dir)?;
        file_store_save_instance(
            self,
            &record.instance_id,
            &val,
            tier_dir,
            &record.type_name,
            2,
            None,
            record.tags.clone(),
        )
    }

    fn save_note(&self, note: &Note) -> Result<(), RepositoryError> {
        let val = note_to_value(note)?;
        let tier_dir = self.record_tier_dir(RecordTier::Note);
        self.ensure_dir(tier_dir)?;
        let slug_source = note.title.as_deref().unwrap_or("");
        file_store_save_instance(
            self,
            &note.instance_id,
            &val,
            tier_dir,
            slug_source,
            0,
            note.title.clone(),
            note.tags.clone(),
        )
    }

    fn load_record_by_id(&self, instance_id: &str) -> Result<Record, RepositoryError> {
        let entry = file_store_find_instance_entry(self, instance_id)?.ok_or_else(|| {
            RepositoryError::InstanceNotFound {
                id: instance_id.to_string(),
            }
        })?;
        let val = self.read_json(&entry.path)?;
        serde_json::from_value(val).map_err(|source| RepositoryError::RecordLoad {
            path: self.abs(&entry.path),
            source,
        })
    }

    fn load_note_by_id(&self, instance_id: &str) -> Result<Note, RepositoryError> {
        let entry = file_store_find_instance_entry(self, instance_id)?.ok_or_else(|| {
            RepositoryError::InstanceNotFound {
                id: instance_id.to_string(),
            }
        })?;
        let val = self.read_json(&entry.path)?;
        // Parity with loader::load_note: parse (NoteLoad) + validate_note (NoteValidation).
        note_from_value(val, &entry.path)
    }

    fn delete_instance(&self, instance_id: &str) -> Result<(), RepositoryError> {
        let entry = file_store_find_instance_entry(self, instance_id)?.ok_or_else(|| {
            RepositoryError::InstanceNotFound {
                id: instance_id.to_string(),
            }
        })?;
        // ADR-007: remove the index entry before the file.
        file_store_remove_instance_index(self, instance_id)?;
        let _ = self.delete_file(&entry.path);
        Ok(())
    }

    fn find_instance(&self, instance_id: &str) -> Result<Option<InstanceRef>, RepositoryError> {
        Ok(file_store_find_instance_entry(self, instance_id)?
            .as_ref()
            .map(InstanceRef::from_index_entry))
    }

    fn list_instances(&self, query: &InstanceQuery) -> Result<Vec<InstanceRef>, RepositoryError> {
        let manifest = self.load_manifest()?;
        Ok(manifest
            .instance_index
            .iter()
            .filter(|e| query.matches(e))
            .map(InstanceRef::from_index_entry)
            .collect())
    }

    // --- Catalog (RFC-038; one walker over the Vfs seam: DiskVfs and MemVfs) ---

    fn catalog(&self) -> Result<crate::catalog::RepositoryCatalog, RepositoryError> {
        crate::catalog::build_checked(self)
    }

    fn catalog_validity_token(&self) -> Result<String, RepositoryError> {
        Ok(crate::catalog::build(self)?.validity_token())
    }

    // --- Relations ---

    fn load_relations_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(relative_path)
    }

    fn save_relations_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(relative_path, value)
    }

    fn ensure_relations_dir(&self, relative_dir: &str) -> Result<(), RepositoryError> {
        self.ensure_dir(relative_dir)
    }

    fn delete_relations_json(&self, relative_path: &str) -> Result<(), RepositoryError> {
        match self.delete_file(relative_path) {
            Err(e) if e.is_not_found() => Ok(()),
            other => other,
        }
    }

    // --- Containers ---

    fn load_container(
        &self,
        container_id: &str,
    ) -> Result<srs_core::types::container::Container, RepositoryError> {
        let path = file_store_find_container_path(self, container_id)?;
        let val = self.read_json(&path)?;
        serde_json::from_value(val).map_err(|source| RepositoryError::ManifestParse {
            path: self.abs(&path),
            source,
        })
    }

    fn save_container(
        &self,
        container: &srs_core::types::container::Container,
    ) -> Result<(), RepositoryError> {
        let id = &container.container_id;
        let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from("containers"),
            source,
        })?;
        self.ensure_dir("containers")?;
        match file_store_find_container_path(self, id) {
            Ok(path) => {
                // Existing container — overwrite file in place; index unchanged
                self.write_json(&path, &val)
            }
            Err(RepositoryError::ContainerNotFound { .. }) => {
                // New container — file-before-index (ADR-007: orphaned file is safe,
                // dangling index entry causes read errors on every subsequent load)
                let slug = container
                    .title
                    .to_lowercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect::<String>();
                let prefix = &id[..id.len().min(8)];
                let filename = format!("containers/{slug}-{prefix}.json");
                self.write_json(&filename, &val)?;
                file_store_upsert_container_index(self, id, &container.title, &filename)
            }
            Err(e) => Err(e),
        }
    }

    fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError> {
        let path = file_store_find_container_path(self, container_id)?;
        // Remove from index
        file_store_remove_container_index(self, container_id)?;
        // Delete file (ignore missing-file errors)
        let _ = self.delete_file(&path);
        Ok(())
    }

    fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
        let index = file_store_load_container_index(self)?;
        Ok(index
            .into_iter()
            .map(|(id, title, _path)| (id, title))
            .collect())
    }

    #[allow(deprecated)]
    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.read_json(relative_path)
    }

    #[allow(deprecated)]
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.write_json(relative_path, value)
    }

    #[allow(deprecated)]
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.delete_file(relative_path)
    }

    #[allow(deprecated)]
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
        self.ensure_dir("containers")
    }

    // --- Package boundaries ---

    fn list_package_boundaries(&self) -> Result<Vec<PackageBoundary>, RepositoryError> {
        let mut result = Vec::new();

        // Primary package
        let primary_json = self.read_json("package/package.json")?;
        result.push(PackageBoundary::from_pkg_json(&primary_json, None));

        // Sub-packages from manifest packageRefs
        let manifest = self.load_manifest()?;
        if let Some(refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            for pkg_ref in refs {
                if pkg_ref.get("mode").and_then(|m| m.as_str()) != Some("local") {
                    continue;
                }
                if let Some(path) = pkg_ref.get("path").and_then(|p| p.as_str()) {
                    let pkg_json_rel = vfs_join(path, "package.json");
                    if let Ok(pkg_json) = self.read_json(&pkg_json_rel) {
                        result.push(PackageBoundary::from_pkg_json(
                            &pkg_json,
                            Some(path.to_string()),
                        ));
                    }
                }
            }
        }
        Ok(result)
    }

    fn load_package_boundary(
        &self,
        selector: &PackageSelector,
    ) -> Result<PackageBoundary, RepositoryError> {
        let pkg_json_rel = match selector {
            None => "package/package.json".to_string(),
            Some(path) => vfs_join(path, "package.json"),
        };
        let pkg_json =
            self.read_json(&pkg_json_rel)
                .map_err(|_| RepositoryError::PackageNotFound {
                    selector: selector.clone(),
                })?;
        Ok(PackageBoundary::from_pkg_json(&pkg_json, selector.clone()))
    }

    fn save_package_boundary_metadata(
        &self,
        boundary: &PackageBoundary,
    ) -> Result<(), RepositoryError> {
        let pkg_json_rel = match &boundary.selector {
            None => "package/package.json".to_string(),
            Some(path) => vfs_join(path, "package.json"),
        };
        // Load existing or create a skeleton
        let mut pkg_json = if self.vfs.is_file(&pkg_json_rel) {
            self.read_json(&pkg_json_rel)?
        } else {
            serde_json::json!({
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": []
            })
        };
        if let Some(obj) = pkg_json.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!(boundary.id));
            obj.insert(
                "namespace".to_string(),
                serde_json::json!(boundary.namespace),
            );
            obj.insert("name".to_string(), serde_json::json!(boundary.name));
            obj.insert("version".to_string(), serde_json::json!(boundary.version));
        }
        self.write_json(&pkg_json_rel, &pkg_json)
    }

    fn register_package_boundary(&self, selector: &PackageSelector) -> Result<(), RepositoryError> {
        let path = match selector {
            None => return Ok(()), // primary — no-op
            Some(p) => p,
        };
        let mut manifest = self.load_manifest()?;
        let mut refs: Vec<serde_json::Value> = manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let already = refs
            .iter()
            .any(|r| r.get("path").and_then(|p| p.as_str()) == Some(path));
        if !already {
            refs.push(serde_json::json!({"mode": "local", "path": path}));
            refs.sort_by(|a, b| {
                a.get("path")
                    .and_then(|p| p.as_str())
                    .cmp(&b.get("path").and_then(|p| p.as_str()))
            });
            manifest
                .extra
                .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
            self.save_manifest(&manifest)?;
        }
        Ok(())
    }

    fn add_definition_to_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let pkg_json_rel = match selector {
            None => "package/package.json".to_string(),
            Some(p) => vfs_join(p, "package.json"),
        };
        let mut pkg_json = self.read_json(&pkg_json_rel)?;
        let key = definition_kind_key(kind);
        // Insert an empty array if the key is absent (e.g. pre-RFC-006 package.json files).
        if pkg_json[key].is_null() {
            pkg_json[key] = serde_json::json!([]);
        }
        let arr = pkg_json[key]
            .as_array_mut()
            .ok_or_else(|| RepositoryError::PackageLoad {
                path: self.abs(&pkg_json_rel),
                source: serde_json::Error::custom(format!("{key} is not an array")),
            })?;
        if !arr.iter().any(|e| e.as_str() == Some(path)) {
            arr.push(serde_json::json!(path));
        }
        self.write_json(&pkg_json_rel, &pkg_json)
    }

    fn remove_definition_from_boundary(
        &self,
        selector: &PackageSelector,
        kind: DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let pkg_json_rel = match selector {
            None => "package/package.json".to_string(),
            Some(p) => vfs_join(p, "package.json"),
        };
        let mut pkg_json = self.read_json(&pkg_json_rel)?;
        let key = definition_kind_key(kind);
        if let Some(arr) = pkg_json[key].as_array_mut() {
            arr.retain(|e| e.as_str() != Some(path));
        }
        self.write_json(&pkg_json_rel, &pkg_json)
    }

    fn resolve_definition_owner(
        &self,
        id: &str,
        kind: DefinitionKind,
    ) -> Result<PackageSelector, RepositoryError> {
        let boundaries = self.list_package_boundaries()?;
        let key = definition_kind_key(kind);
        for boundary in &boundaries {
            let boundary_prefix = match &boundary.selector {
                None => "package".to_string(),
                Some(p) => p.clone(),
            };
            let pkg_json_rel = vfs_join(&boundary_prefix, "package.json");
            if let Ok(pkg_json) = self.read_json(&pkg_json_rel) {
                if let Some(paths) = pkg_json[key].as_array() {
                    for entry in paths {
                        if let Some(rel) = entry.as_str() {
                            let full = vfs_join(&boundary_prefix, rel);
                            if let Ok(def_json) = self.read_json(&full) {
                                if def_json["id"].as_str() == Some(id) {
                                    return Ok(boundary.selector.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(RepositoryError::DefinitionNotFound { id: id.to_string() })
    }

    // --- Generic file access ---

    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String> {
        self.vfs.list_recursive(relative_dir)
    }

    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError> {
        self.vfs.read_to_string(relative_path)
    }

    fn save_text_file(&self, relative_path: &str, content: &str) -> Result<(), RepositoryError> {
        if let Some((parent, _)) = relative_path.rsplit_once('/') {
            self.vfs.create_dir_all(parent)?;
        }
        self.vfs.write(relative_path, content.as_bytes())
    }

    fn load_binary_file(&self, relative_path: &str) -> Result<Vec<u8>, RepositoryError> {
        self.vfs.read_bytes(relative_path)
    }

    fn file_byte_len(&self, relative_path: &str) -> Result<u64, RepositoryError> {
        self.vfs.byte_len(relative_path)
    }

    fn save_binary_file(&self, relative_path: &str, content: &[u8]) -> Result<(), RepositoryError> {
        if let Some((parent, _)) = relative_path.rsplit_once('/') {
            self.vfs.create_dir_all(parent)?;
        }
        self.vfs.write(relative_path, content)
    }

    // --- Sub-package path validation ---

    fn as_tree_snapshot(&self) -> Option<std::collections::BTreeMap<String, Vec<u8>>> {
        self.vfs.as_mem_snapshot()
    }

    fn is_file_tree_store(&self) -> bool {
        true
    }

    fn validate_package_ref_path(&self, relative_path: &str) -> Result<(), RepositoryError> {
        match self.vfs.check_dir_within_root(relative_path)? {
            DirCheck::Missing => Err(RepositoryError::PackageRefMissing {
                path: relative_path.to_string(),
            }),
            DirCheck::OutsideRoot => Err(RepositoryError::PackageRefOutsideRepo {
                path: relative_path.to_string(),
            }),
            DirCheck::Ok => {
                if !self.vfs.is_file(&vfs_join(relative_path, "package.json")) {
                    return Err(RepositoryError::PackageRefMissing {
                        path: relative_path.to_string(),
                    });
                }
                Ok(())
            }
        }
    }
}

/// Map a `DefinitionKind` to the JSON array key in `package.json`.
pub(crate) fn definition_kind_key(kind: DefinitionKind) -> &'static str {
    match kind {
        DefinitionKind::Field => "fields",
        DefinitionKind::Type => "types",
        DefinitionKind::View => "views",
        DefinitionKind::DocumentView => "documentViews",
        DefinitionKind::RelationType => "relationTypes",
        DefinitionKind::Blueprint => "blueprints",
        DefinitionKind::Protocol => "protocols",
        DefinitionKind::Vocabulary => "vocabularies",
        DefinitionKind::Lifecycle => "lifecycles",
        DefinitionKind::Theme => "themes",
    }
}

/// Load the container index as `(container_id, title, path)` triples from the manifest.
fn file_store_load_container_index(
    store: &FileStore,
) -> Result<Vec<(String, String, String)>, RepositoryError> {
    let manifest = store.load_manifest()?;
    Ok(manifest
        .container_index
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| {
            let path = e.path?;
            Some((e.container_id, e.title.unwrap_or_default(), path))
        })
        .collect())
}

/// Find the file path for a container by its `container_id`.
fn file_store_find_container_path(
    store: &FileStore,
    container_id: &str,
) -> Result<String, RepositoryError> {
    let index = file_store_load_container_index(store)?;
    index
        .into_iter()
        .find(|(id, _, _)| id == container_id)
        .map(|(_, _, path)| path)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: container_id.to_string(),
        })
}

/// Insert or update an entry in the manifest `containerIndex`.
fn file_store_upsert_container_index(
    store: &FileStore,
    container_id: &str,
    title: &str,
    path: &str,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    let mut entries = manifest.container_index.unwrap_or_default();
    entries.retain(|e| e.container_id != container_id);
    entries.push(ContainerIndexEntry {
        container_id: container_id.to_string(),
        title: Some(title.to_string()),
        path: Some(path.to_string()),
        container_type: None,
        tags: None,
        extra: std::collections::BTreeMap::new(),
    });
    manifest.container_index = Some(entries);
    store.save_manifest(&manifest)
}

/// Remove an entry from the manifest `containerIndex`.
fn file_store_remove_container_index(
    store: &FileStore,
    container_id: &str,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    let mut entries = manifest.container_index.unwrap_or_default();
    entries.retain(|e| e.container_id != container_id);
    manifest.container_index = if entries.is_empty() {
        None
    } else {
        Some(entries)
    };
    store.save_manifest(&manifest)
}

// ---------------------------------------------------------------------------
// Instance persistence helpers (ADR-042) — shared by the typed store methods.
// ---------------------------------------------------------------------------

/// Serialize a `Record` to its on-disk JSON value, injecting `$schema` (parity
/// with the transitional `write_record`).
pub(crate) fn record_to_value(record: &Record) -> Result<serde_json::Value, RepositoryError> {
    let mut value = serde_json::to_value(record).map_err(|source| RepositoryError::Serialize {
        path: PathBuf::from("records/tier-2"),
        source,
    })?;
    if let serde_json::Value::Object(ref mut obj) = value {
        obj.insert(
            "$schema".to_string(),
            serde_json::Value::String(RECORD_SCHEMA_ID.to_string()),
        );
    }
    Ok(value)
}

/// Serialize a `Note` to its on-disk JSON value, injecting `$schema` (parity
/// with the transitional `write_note`).
pub(crate) fn note_to_value(note: &Note) -> Result<serde_json::Value, RepositoryError> {
    let mut value = serde_json::to_value(note).map_err(|source| RepositoryError::Serialize {
        path: PathBuf::from("records/notes"),
        source,
    })?;
    if let serde_json::Value::Object(ref mut obj) = value {
        obj.insert(
            "$schema".to_string(),
            serde_json::Value::String(NOTE_SCHEMA_ID.to_string()),
        );
    }
    Ok(value)
}

/// Deserialize + validate a `Note` from its on-disk JSON value — the exact parity
/// counterpart of `loader::load_note` (parse errors → `NoteLoad`, `validate_note`
/// failures → `NoteValidation`), so `load_note_by_id` preserves the read-side
/// validation the path-based loader performed.
pub(crate) fn note_from_value(
    value: serde_json::Value,
    relative_path: &str,
) -> Result<Note, RepositoryError> {
    let note: Note = serde_json::from_value(value).map_err(|source| RepositoryError::NoteLoad {
        path: PathBuf::from(relative_path),
        source,
    })?;
    srs_core::validation::note::validate_note(&note).map_err(|source| {
        RepositoryError::NoteValidation {
            path: PathBuf::from(relative_path),
            source,
        }
    })?;
    Ok(note)
}

/// Collision-safe canonical filename for a new instance: `{tier_dir}/{slug}-{id8}.json`
/// (id-only when the slug is empty). Reuses the scheme from `create_record_at_dir`
/// (ADR-040 cited for reuse, not for its full-UUID disambiguation fallback).
pub(crate) fn instance_filename(tier_dir: &str, slug_source: &str, instance_id: &str) -> String {
    let slug = crate::writer::slugify_instance_name(slug_source);
    let id8 = &instance_id[..instance_id.len().min(8)];
    if slug.is_empty() {
        format!("{tier_dir}/{id8}.json")
    } else {
        format!("{tier_dir}/{slug}-{id8}.json")
    }
}

/// Find the manifest index entry for an instance by its logical id.
fn file_store_find_instance_entry(
    store: &FileStore,
    instance_id: &str,
) -> Result<Option<InstanceIndexEntry>, RepositoryError> {
    let manifest = store.load_manifest()?;
    Ok(manifest
        .instance_index
        .into_iter()
        .find(|e| e.instance_id == instance_id))
}

/// Insert or replace an instance's manifest index entry.
fn file_store_upsert_instance_index(
    store: &FileStore,
    entry: InstanceIndexEntry,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    if let Some(pos) = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id == entry.instance_id)
    {
        manifest.instance_index[pos] = entry;
    } else {
        manifest.instance_index.push(entry);
    }
    store.save_manifest(&manifest)
}

/// Remove an instance's manifest index entry.
fn file_store_remove_instance_index(
    store: &FileStore,
    instance_id: &str,
) -> Result<(), RepositoryError> {
    let mut manifest = store.load_manifest()?;
    manifest
        .instance_index
        .retain(|e| e.instance_id != instance_id);
    store.save_manifest(&manifest)
}

/// The two-branch save shared by `save_record`/`save_note` (mirrors `save_container`):
/// existing id ⇒ overwrite at the existing path (path + tier preserved, denormalized
/// `title`/`tags` refreshed); new id ⇒ derive a filename and write entity-before-index
/// (ADR-007). `title` is passed pre-normalized (records pass `None`).
#[allow(clippy::too_many_arguments)]
fn file_store_save_instance(
    store: &FileStore,
    instance_id: &str,
    value: &serde_json::Value,
    tier_dir: &str,
    slug_source: &str,
    new_tier: u8,
    title: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<(), RepositoryError> {
    let existing = file_store_find_instance_entry(store, instance_id)?;
    let (path, tier) = match &existing {
        Some(e) => (e.path.clone(), e.tier), // preserve path + tier (no rename)
        None => (
            instance_filename(tier_dir, slug_source, instance_id),
            new_tier,
        ),
    };
    // Entity before index (ADR-007) in both branches.
    store.write_json(&path, value)?;
    file_store_upsert_instance_index(
        store,
        InstanceIndexEntry {
            instance_id: instance_id.to_string(),
            tier,
            path,
            title: title.map(serde_json::Value::String),
            tags,
        },
    )
}

// ---------------------------------------------------------------------------
// MemoryStore — in-memory test implementation
// ---------------------------------------------------------------------------

pub mod memory {
    use super::*;
    use std::cell::RefCell;

    /// Fault-injection point for `MemoryStore`. When armed, the next call to the
    /// named operation returns an `Io` error; subsequent calls proceed normally.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailPoint {
        /// Fail the next `save_manifest` call.
        SaveManifest,
        /// Fail the next `delete_instance_file` call.
        DeleteInstanceFile,
        /// Fail the next container-index update in `save_container` (after data write).
        /// Simulates a crash between the file write and the index update for ADR-007 testing.
        SaveContainerIndex,
        /// Fail the next instance-index update in `save_record`/`save_note` (after data write).
        /// Simulates a crash between the file write and the index update for ADR-007 testing.
        SaveInstanceIndex,
        /// Fail the next `load_package` call.
        LoadPackage,
    }

    /// In-memory implementation of [`RepositoryStore`] for unit tests.
    ///
    /// No filesystem access. All `ensure_*` methods are no-ops.
    pub struct MemoryStore {
        manifest: RefCell<Manifest>,
        package: RefCell<Package>,
        data: RefCell<HashMap<String, serde_json::Value>>,
        /// Binary files keyed by relative path (parallel to `data` for text/JSON).
        binary_data: RefCell<HashMap<String, Vec<u8>>>,
        repository_initialized: RefCell<bool>,
        /// Package boundary metadata keyed by `PackageSelector`.
        /// Always pre-populated with the primary boundary (`None`).
        boundaries: RefCell<HashMap<Option<String>, crate::package_types::PackageBoundary>>,
        fail_at: RefCell<Option<FailPoint>>,
    }

    impl MemoryStore {
        pub fn new(manifest: Manifest, package: Package) -> Self {
            let pkg_json = Self::package_to_json(&package);
            let primary_boundary = crate::package_types::PackageBoundary {
                selector: None,
                id: package.id.clone(),
                namespace: package.namespace.clone(),
                name: package.name.clone(),
                version: package.version.clone(),
                field_paths: vec![],
                type_paths: vec![],
                blueprint_paths: vec![],
                protocol_paths: vec![],
                view_paths: vec![],
                relation_type_paths: vec![],
                lifecycle_paths: vec![],
                document_view_paths: vec![],
            };
            let mut boundaries = HashMap::new();
            boundaries.insert(None, primary_boundary);
            let store = Self {
                manifest: RefCell::new(manifest),
                package: RefCell::new(package),
                data: RefCell::new(HashMap::new()),
                binary_data: RefCell::new(HashMap::new()),
                repository_initialized: RefCell::new(true),
                boundaries: RefCell::new(boundaries),
                fail_at: RefCell::new(None),
            };
            store
                .data
                .borrow_mut()
                .insert("package/package.json".to_string(), pkg_json);
            store
        }

        /// Minimal empty store — empty manifest, empty package, minimal package.json.
        pub fn empty() -> Self {
            let manifest = Manifest {
                instance_index: vec![],
                container: None,
                container_index: None,
                federation_path: None,
                upstream_package: None,
                federation_events_path: None,
                extra: std::collections::BTreeMap::new(),
                source_documents_path: None,
                source_document_index: None,
                root: PathBuf::from("/memory"),
            };
            let package = Package {
                id: "test-pkg".to_string(),
                namespace: "com.test".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            };
            Self::new(manifest, package)
        }

        /// Build a store pre-populated with a single field.
        pub fn with_field(field: Field) -> Self {
            let store = Self::empty();
            let filename = format!(
                "fields/{}-{}.json",
                field.name.to_lowercase().replace(' ', "-"),
                &field.id[..8]
            );
            // Update the in-memory package
            store.package.borrow_mut().fields.push(field.clone());
            // Update package.json index (paths are package-relative, no "package/" prefix)
            store
                .data
                .borrow_mut()
                .get_mut("package/package.json")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .get_mut("fields")
                .unwrap()
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(filename.clone()));
            // Store the field data file at repo-root-relative key ("package/fields/...")
            let field_val = serde_json::to_value(&field).unwrap();
            store
                .data
                .borrow_mut()
                .insert(format!("package/{filename}"), field_val);
            // Update primary boundary field_paths for resolve_definition_owner
            store
                .boundaries
                .borrow_mut()
                .get_mut(&None)
                .unwrap()
                .field_paths
                .push(filename);
            store
        }

        /// Build a store pre-populated with a single type.
        pub fn with_type(record_type: RecordType) -> Self {
            let store = Self::empty();
            let filename = format!(
                "types/{}-{}.json",
                record_type.name.to_lowercase().replace(' ', "-"),
                &record_type.id[..8]
            );
            store
                .package
                .borrow_mut()
                .record_types
                .push(record_type.clone());
            store
                .data
                .borrow_mut()
                .get_mut("package/package.json")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .get_mut("types")
                .unwrap()
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!(filename.clone()));
            let type_val = serde_json::to_value(&record_type).unwrap();
            store
                .data
                .borrow_mut()
                .insert(format!("package/{filename}"), type_val);
            // Update primary boundary type_paths for resolve_definition_owner
            store
                .boundaries
                .borrow_mut()
                .get_mut(&None)
                .unwrap()
                .type_paths
                .push(filename);
            store
        }

        fn package_to_json(pkg: &Package) -> serde_json::Value {
            serde_json::json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": pkg.id,
                "namespace": pkg.namespace,
                "name": pkg.name,
                "version": pkg.version,
                "title": pkg.name,
                "description": "",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": [],
                "protocols": [],
                "vocabularies": [],
                "lifecycles": []
            })
        }

        /// Pre-populate with a JSON value at the given relative path.
        pub fn with_data(self, path: &str, value: serde_json::Value) -> Self {
            self.data.borrow_mut().insert(path.to_string(), value);
            self
        }

        /// Pre-populate the in-memory package with a protocol definition.
        ///
        /// `MemoryStore::load_package()` returns `self.package` directly, so only
        /// the typed package field needs updating — no `self.data` write required.
        pub fn with_protocol(self, protocol: crate::package::LoadedProtocol) -> Self {
            self.package.borrow_mut().protocols.push(protocol);
            self
        }

        /// Arm a fail point; the next call to the named operation will return an error.
        pub fn with_fail_at(self, point: FailPoint) -> Self {
            *self.fail_at.borrow_mut() = Some(point);
            self
        }

        pub fn arm_fail_at(&self, point: FailPoint) {
            *self.fail_at.borrow_mut() = Some(point);
        }

        /// Cancel a previously armed fail point without triggering it.
        /// Useful in multi-step tests where a fault is armed conditionally.
        pub fn disarm_fail_at(&self) {
            *self.fail_at.borrow_mut() = None;
        }

        pub fn uninitialized() -> Self {
            let manifest = Manifest {
                instance_index: vec![],
                container: None,
                container_index: None,
                federation_path: None,
                upstream_package: None,
                federation_events_path: None,
                extra: std::collections::BTreeMap::new(),
                source_documents_path: None,
                source_document_index: None,
                root: PathBuf::from("/memory"),
            };
            let package = Package {
                id: "".to_string(),
                namespace: "".to_string(),
                name: "".to_string(),
                version: "".to_string(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            };
            Self {
                manifest: RefCell::new(manifest),
                package: RefCell::new(package),
                data: RefCell::new(HashMap::new()),
                binary_data: RefCell::new(HashMap::new()),
                repository_initialized: RefCell::new(false),
                boundaries: RefCell::new(HashMap::new()),
                fail_at: RefCell::new(None),
            }
        }

        /// Return a clone of all stored data (for assertions).
        pub fn all_data(&self) -> HashMap<String, serde_json::Value> {
            self.data.borrow().clone()
        }

        /// Resolve an instance's data key (path) from the manifest index (ADR-042).
        fn mem_instance_path(&self, instance_id: &str) -> Result<String, RepositoryError> {
            self.manifest
                .borrow()
                .instance_index
                .iter()
                .find(|e| e.instance_id == instance_id)
                .map(|e| e.path.clone())
                .ok_or_else(|| RepositoryError::InstanceNotFound {
                    id: instance_id.to_string(),
                })
        }

        /// The two-branch instance save shared by `save_record`/`save_note`
        /// (mirrors `save_container`): existing id ⇒ overwrite at the existing path
        /// (path + tier preserved, denormalized `title`/`tags` refreshed); new id ⇒
        /// derive a filename and write data before index (ADR-007). Honours
        /// `FailPoint::SaveInstanceIndex` between the two writes.
        #[allow(clippy::too_many_arguments)]
        fn mem_save_instance(
            &self,
            instance_id: &str,
            value: &serde_json::Value,
            tier_dir: &str,
            slug_source: &str,
            new_tier: u8,
            title: Option<String>,
            tags: Option<Vec<String>>,
        ) -> Result<(), RepositoryError> {
            let existing = self
                .manifest
                .borrow()
                .instance_index
                .iter()
                .find(|e| e.instance_id == instance_id)
                .map(|e| (e.path.clone(), e.tier));
            let (path, tier) = match existing {
                Some((p, t)) => (p, t),
                None => (
                    instance_filename(tier_dir, slug_source, instance_id),
                    new_tier,
                ),
            };
            // Data before index (ADR-007).
            self.data.borrow_mut().insert(path.clone(), value.clone());
            if matches!(*self.fail_at.borrow(), Some(FailPoint::SaveInstanceIndex)) {
                *self.fail_at.borrow_mut() = None;
                return Err(RepositoryError::Io {
                    path: std::path::PathBuf::from("injected"),
                    source: std::io::Error::other("injected fault: save_instance_index"),
                });
            }
            let entry = InstanceIndexEntry {
                instance_id: instance_id.to_string(),
                tier,
                path,
                title: title.map(serde_json::Value::String),
                tags,
            };
            let mut manifest = self.manifest.borrow_mut();
            if let Some(pos) = manifest
                .instance_index
                .iter()
                .position(|e| e.instance_id == instance_id)
            {
                manifest.instance_index[pos] = entry;
            } else {
                manifest.instance_index.push(entry);
            }
            Ok(())
        }

        /// Sync the `data["<prefix>/package.json"]` JSON entry when a definition
        /// is added or removed, so `load_package_json()` stays consistent.
        fn memory_store_sync_pkg_json(
            &self,
            selector: &PackageSelector,
            kind: crate::package_types::DefinitionKind,
            path: &str,
            add: bool, // true = add, false = remove
        ) -> Result<(), RepositoryError> {
            use crate::store::definition_kind_key;
            let data_key = match selector {
                None => "package/package.json".to_string(),
                Some(p) => format!("{p}/package.json"),
            };
            let array_key = definition_kind_key(kind);
            let mut data = self.data.borrow_mut();
            if let Some(pkg_json) = data.get_mut(&data_key) {
                // Mirror FileStore::add_definition_to_boundary: create the array if absent so
                // definition kinds not pre-seeded in package_to_json (e.g. protocols) still sync.
                if add && pkg_json[array_key].is_null() {
                    pkg_json[array_key] = serde_json::json!([]);
                }
                if let Some(arr) = pkg_json[array_key].as_array_mut() {
                    if add {
                        if !arr.iter().any(|e| e.as_str() == Some(path)) {
                            arr.push(serde_json::json!(path));
                        }
                    } else {
                        arr.retain(|e| e.as_str() != Some(path));
                    }
                }
            }
            Ok(())
        }
    }

    impl Default for MemoryStore {
        fn default() -> Self {
            Self::empty()
        }
    }

    fn not_found(path: &str) -> RepositoryError {
        RepositoryError::Io {
            path: PathBuf::from(path),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found in MemoryStore"),
        }
    }

    impl RepositoryStore for MemoryStore {
        fn repository_root(&self) -> PathBuf {
            PathBuf::from("/memory")
        }

        fn repository_exists(&self) -> Result<bool, RepositoryError> {
            Ok(*self.repository_initialized.borrow())
        }

        fn initialize_repository(
            &self,
            input: &InitializeRepositoryInput,
        ) -> Result<CreateRepositoryResult, RepositoryError> {
            if *self.repository_initialized.borrow() {
                return Err(RepositoryError::RepositoryAlreadyExists {
                    path: PathBuf::from("/memory"),
                });
            }

            let mut manifest_extra = std::collections::BTreeMap::new();
            manifest_extra.insert(
                "srsVersion".to_string(),
                serde_json::Value::String(input.repository.srs_version.clone()),
            );
            manifest_extra.insert(
                "repositoryId".to_string(),
                serde_json::Value::String(input.repository.repository_id.clone()),
            );
            manifest_extra.insert(
                "namespace".to_string(),
                serde_json::Value::String(input.repository.namespace.clone()),
            );
            if let Some(title) = &input.repository.title {
                manifest_extra.insert(
                    "title".to_string(),
                    serde_json::Value::String(title.clone()),
                );
            }
            if let Some(desc) = &input.repository.description {
                manifest_extra.insert(
                    "description".to_string(),
                    serde_json::Value::String(desc.clone()),
                );
            }

            *self.manifest.borrow_mut() = Manifest {
                instance_index: vec![],
                container: None,
                container_index: None,
                federation_path: None,
                upstream_package: None,
                federation_events_path: None,
                extra: manifest_extra,
                source_documents_path: None,
                source_document_index: None,
                root: PathBuf::from("/memory"),
            };

            *self.package.borrow_mut() = Package {
                id: input.primary_package.id.clone(),
                namespace: input.primary_package.namespace.clone(),
                name: input.primary_package.name.clone(),
                version: input.primary_package.version.clone(),
                fields: vec![],
                record_types: vec![],
                relation_type_definitions: vec![],
                views: vec![],
                document_views: vec![],
                themes: vec![],
                blueprints: vec![],
                protocols: vec![],
                root: PathBuf::from("/memory"),
                dependency_refs: vec![],
                vocabularies: vec![],
                lifecycles: vec![],
            };

            let package_json = serde_json::json!({
                "id": input.primary_package.id,
                "namespace": input.primary_package.namespace,
                "name": input.primary_package.name,
                "version": input.primary_package.version,
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "blueprints": [],
                "protocols": []
            });
            self.data
                .borrow_mut()
                .insert("package/package.json".to_string(), package_json);
            *self.repository_initialized.borrow_mut() = true;

            Ok(CreateRepositoryResult {
                repo_root: PathBuf::from("/memory"),
                repository_id: input.repository.repository_id.clone(),
                package_id: input.primary_package.id.clone(),
                identity_instance_id: None,
            })
        }

        fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
            Ok(self.manifest.borrow().clone())
        }

        fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError> {
            let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::SaveManifest));
            if should_fail {
                *self.fail_at.borrow_mut() = None;
                return Err(RepositoryError::Io {
                    path: std::path::PathBuf::from("manifest.json"),
                    source: std::io::Error::other("injected fault: save_manifest"),
                });
            }
            *self.manifest.borrow_mut() = manifest.clone();
            Ok(())
        }

        fn load_package(&self) -> Result<Package, RepositoryError> {
            let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::LoadPackage));
            if should_fail {
                *self.fail_at.borrow_mut() = None;
                return Err(RepositoryError::Io {
                    path: std::path::PathBuf::from("package/package.json"),
                    source: std::io::Error::other("injected fault: load_package"),
                });
            }
            let mut pkg = self.package.borrow().clone();
            // Supplement the static package with any protocols written via the
            // write path (save_instance_json + add_definition_to_boundary). This
            // lets write-then-read tests (e.g. import_protocol followed by
            // find_protocol_by_target_type) work correctly in MemoryStore.
            let data = self.data.borrow();
            if let Some(pkg_json) = data.get("package/package.json") {
                if let Some(paths) = pkg_json["protocols"].as_array() {
                    for path_val in paths {
                        if let Some(rel) = path_val.as_str() {
                            let full = format!("package/{rel}");
                            if let Some(raw) = data.get(&full) {
                                let proto = serde_json::from_value::<
                                    srs_core::types::protocol::Protocol,
                                >(raw.clone())
                                .map_err(|source| RepositoryError::PackageLoad {
                                    path: std::path::PathBuf::from(&full),
                                    source,
                                })?;
                                if !pkg
                                    .protocols
                                    .iter()
                                    .any(|p| p.protocol.protocol_id == proto.protocol_id)
                                {
                                    pkg.protocols.push(crate::package::LoadedProtocol {
                                        protocol: proto,
                                        raw: raw.clone(),
                                        source_package: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            drop(data);
            crate::core_package::merge_core_into_package(
                &mut pkg.fields,
                &mut pkg.record_types,
                &mut pkg.relation_type_definitions,
            )?;
            Ok(pkg)
        }

        fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get("package/package.json")
                .cloned()
                .ok_or_else(|| not_found("package/package.json"))
        }

        fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert("package/package.json".to_string(), value.clone());
            Ok(())
        }

        fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(field).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_field_file(
            &self,
            relative_path: &str,
            field: &Field,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_field(relative_path, field)
        }

        fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_fields_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_type(
            &self,
            relative_path: &str,
            record_type: &RecordType,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(record_type).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_type_file(
            &self,
            relative_path: &str,
            record_type: &RecordType,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_type(relative_path, record_type)
        }

        fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_relation_type_definition(
            &self,
            relative_path: &str,
            relation_type: &RelationTypeDefinition,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(relation_type).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            // Keep self.package in sync so load_package() reflects writes.
            let mut pkg = self.package.borrow_mut();
            if let Some(existing) = pkg
                .relation_type_definitions
                .iter_mut()
                .find(|rt| rt.id == relation_type.id)
            {
                *existing = relation_type.clone();
            } else {
                pkg.relation_type_definitions.push(relation_type.clone());
            }
            Ok(())
        }

        fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            let key = format!("package/{relative_path}");
            self.data.borrow_mut().remove(&key);
            Ok(())
        }

        fn ensure_relation_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(view).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_view_file(
            &self,
            relative_path: &str,
            view: &View,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_view(relative_path, view)
        }

        fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_document_view(
            &self,
            relative_path: &str,
            view: &DocumentView,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(view).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_document_view_file(
            &self,
            relative_path: &str,
            view: &DocumentView,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_document_view(relative_path, view)
        }

        fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_document_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_theme(
            &self,
            relative_path: &str,
            theme: &srs_core::types::theme::Theme,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(theme).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_theme_file(
            &self,
            relative_path: &str,
            theme: &srs_core::types::theme::Theme,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_theme(relative_path, theme)
        }

        fn delete_theme_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_themes_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_blueprint(
            &self,
            relative_path: &str,
            blueprint: &srs_core::types::blueprint::Blueprint,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(blueprint).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            Ok(())
        }

        fn update_blueprint_file(
            &self,
            relative_path: &str,
            blueprint: &srs_core::types::blueprint::Blueprint,
        ) -> Result<(), RepositoryError> {
            if !self.data.borrow().contains_key(relative_path) {
                return Err(not_found(relative_path));
            }
            self.save_blueprint(relative_path, blueprint)
        }

        fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_blueprints_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_vocabulary(
            &self,
            relative_path: &str,
            vocabulary: &Vocabulary,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(vocabulary).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            // Keep self.package in sync so load_package() reflects writes.
            let mut pkg = self.package.borrow_mut();
            if let Some(existing) = pkg
                .vocabularies
                .iter_mut()
                .find(|vc| vc.id == vocabulary.id)
            {
                *existing = vocabulary.clone();
            } else {
                pkg.vocabularies.push(vocabulary.clone());
            }
            Ok(())
        }

        fn ensure_vocabularies_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn save_lifecycle(
            &self,
            relative_path: &str,
            lifecycle: &Lifecycle,
        ) -> Result<(), RepositoryError> {
            let v = serde_json::to_value(lifecycle).unwrap();
            self.data.borrow_mut().insert(relative_path.to_string(), v);
            // Keep self.package in sync so load_package() reflects writes.
            let mut pkg = self.package.borrow_mut();
            if let Some(existing) = pkg.lifecycles.iter_mut().find(|lc| lc.id == lifecycle.id) {
                *existing = lifecycle.clone();
            } else {
                pkg.lifecycles.push(lifecycle.clone());
            }
            Ok(())
        }

        fn ensure_lifecycles_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn load_instance_json(
            &self,
            relative_path: &str,
        ) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        fn save_instance_json(
            &self,
            relative_path: &str,
            value: &serde_json::Value,
        ) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert(relative_path.to_string(), value.clone());
            Ok(())
        }

        fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::DeleteInstanceFile));
            if should_fail {
                *self.fail_at.borrow_mut() = None;
                return Err(RepositoryError::Io {
                    path: std::path::PathBuf::from(relative_path),
                    source: std::io::Error::other("injected fault: delete_instance_file"),
                });
            }
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn ensure_instance_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError> {
            let prefix = if relative_dir.ends_with('/') {
                relative_dir.to_string()
            } else {
                format!("{}/", relative_dir)
            };
            // Direct children only: no additional '/' after the prefix (non-recursive).
            let paths = self
                .data
                .borrow()
                .keys()
                .filter(|k| {
                    k.starts_with(&prefix)
                        && k.ends_with(".json")
                        && !k[prefix.len()..].contains('/')
                })
                .cloned()
                .collect();
            Ok(paths)
        }

        fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
            tier.dir()
        }

        // --- Instances (logical-id + typed; ADR-042) ---

        fn save_record(&self, record: &Record) -> Result<(), RepositoryError> {
            let val = record_to_value(record)?;
            self.mem_save_instance(
                &record.instance_id,
                &val,
                self.record_tier_dir(RecordTier::Tier2),
                &record.type_name,
                2,
                None,
                record.tags.clone(),
            )
        }

        fn save_note(&self, note: &Note) -> Result<(), RepositoryError> {
            let val = note_to_value(note)?;
            self.mem_save_instance(
                &note.instance_id,
                &val,
                self.record_tier_dir(RecordTier::Note),
                note.title.as_deref().unwrap_or(""),
                0,
                note.title.clone(),
                note.tags.clone(),
            )
        }

        fn load_record_by_id(&self, instance_id: &str) -> Result<Record, RepositoryError> {
            let path = self.mem_instance_path(instance_id)?;
            let val = self.load_instance_json(&path)?;
            serde_json::from_value(val).map_err(|source| RepositoryError::RecordLoad {
                path: std::path::PathBuf::from(&path),
                source,
            })
        }

        fn load_note_by_id(&self, instance_id: &str) -> Result<Note, RepositoryError> {
            let path = self.mem_instance_path(instance_id)?;
            let val = self.load_instance_json(&path)?;
            // Parity with loader::load_note: parse (NoteLoad) + validate_note (NoteValidation).
            note_from_value(val, &path)
        }

        fn delete_instance(&self, instance_id: &str) -> Result<(), RepositoryError> {
            let path = self.mem_instance_path(instance_id)?;
            // ADR-007: remove the index entry before the data. Routed through
            // `save_manifest`/`delete_instance_file` (rather than mutating `manifest`/
            // `data` directly) so this honours `FailPoint::SaveManifest` and
            // `FailPoint::DeleteInstanceFile` the same way the legacy write path did.
            let mut manifest = self.manifest.borrow().clone();
            manifest
                .instance_index
                .retain(|e| e.instance_id != instance_id);
            self.save_manifest(&manifest)?;
            let _ = self.delete_instance_file(&path);
            Ok(())
        }

        fn find_instance(&self, instance_id: &str) -> Result<Option<InstanceRef>, RepositoryError> {
            Ok(self
                .manifest
                .borrow()
                .instance_index
                .iter()
                .find(|e| e.instance_id == instance_id)
                .map(InstanceRef::from_index_entry))
        }

        fn list_instances(
            &self,
            query: &InstanceQuery,
        ) -> Result<Vec<InstanceRef>, RepositoryError> {
            Ok(self
                .manifest
                .borrow()
                .instance_index
                .iter()
                .filter(|e| query.matches(e))
                .map(InstanceRef::from_index_entry)
                .collect())
        }

        // --- Catalog (RFC-038): the shared walker enumerates this store's
        // object maps through `list_files_recursive`/`load_instance_json` ---

        fn catalog(&self) -> Result<crate::catalog::RepositoryCatalog, RepositoryError> {
            crate::catalog::build_checked(self)
        }

        fn catalog_validity_token(&self) -> Result<String, RepositoryError> {
            Ok(crate::catalog::build(self)?.validity_token())
        }

        fn load_relations_json(
            &self,
            relative_path: &str,
        ) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        fn save_relations_json(
            &self,
            relative_path: &str,
            value: &serde_json::Value,
        ) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert(relative_path.to_string(), value.clone());
            Ok(())
        }

        fn ensure_relations_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn delete_relations_json(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        fn load_container(
            &self,
            container_id: &str,
        ) -> Result<srs_core::types::container::Container, RepositoryError> {
            let key = format!("containers/{container_id}.json");
            let val = self.data.borrow().get(&key).cloned().ok_or_else(|| {
                RepositoryError::ContainerNotFound {
                    container_id: container_id.to_string(),
                }
            })?;
            serde_json::from_value(val).map_err(|source| RepositoryError::ManifestParse {
                path: std::path::PathBuf::from(&key),
                source,
            })
        }

        fn save_container(
            &self,
            container: &srs_core::types::container::Container,
        ) -> Result<(), RepositoryError> {
            let id = &container.container_id;
            let key = format!("containers/{id}.json");
            let val =
                serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
                    path: std::path::PathBuf::from(&key),
                    source,
                })?;
            self.data.borrow_mut().insert(key, val);
            // Fault injection point: after data write, before index update.
            // Simulates a crash between the file write and the index update (ADR-007).
            let should_fail = matches!(*self.fail_at.borrow(), Some(FailPoint::SaveContainerIndex));
            if should_fail {
                *self.fail_at.borrow_mut() = None;
                return Err(RepositoryError::Io {
                    path: std::path::PathBuf::from("injected"),
                    source: std::io::Error::other("injected fault: save_container_index"),
                });
            }
            // Update summary index in manifest
            let mut manifest = self.manifest.borrow_mut();
            let mut entries = manifest.container_index.take().unwrap_or_default();
            entries.retain(|e| &e.container_id != id);
            entries.push(srs_core::types::container::ContainerIndexEntry {
                container_id: id.clone(),
                title: Some(container.title.clone()),
                path: None,
                container_type: None,
                tags: None,
                extra: std::collections::BTreeMap::new(),
            });
            manifest.container_index = Some(entries);
            Ok(())
        }

        fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError> {
            let key = format!("containers/{container_id}.json");
            if self.data.borrow_mut().remove(&key).is_none() {
                return Err(RepositoryError::ContainerNotFound {
                    container_id: container_id.to_string(),
                });
            }
            // Remove from manifest index
            let mut manifest = self.manifest.borrow_mut();
            let mut entries = manifest.container_index.take().unwrap_or_default();
            entries.retain(|e| e.container_id != container_id);
            manifest.container_index = if entries.is_empty() {
                None
            } else {
                Some(entries)
            };
            Ok(())
        }

        fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
            let manifest = self.manifest.borrow();
            Ok(manifest
                .container_index
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(|e| (e.container_id.clone(), e.title.clone().unwrap_or_default()))
                .collect())
        }

        #[allow(deprecated)]
        fn load_container_json(
            &self,
            relative_path: &str,
        ) -> Result<serde_json::Value, RepositoryError> {
            self.data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        #[allow(deprecated)]
        fn save_container_json(
            &self,
            relative_path: &str,
            value: &serde_json::Value,
        ) -> Result<(), RepositoryError> {
            self.data
                .borrow_mut()
                .insert(relative_path.to_string(), value.clone());
            Ok(())
        }

        #[allow(deprecated)]
        fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
            self.data.borrow_mut().remove(relative_path);
            Ok(())
        }

        #[allow(deprecated)]
        fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn list_files_recursive(&self, relative_dir: &str) -> Vec<String> {
            let data = self.data.borrow();
            let binary_data = self.binary_data.borrow();
            // data and binary_data are disjoint per ADR-031; chain() is dedup-free.
            if relative_dir.is_empty() {
                return data.keys().chain(binary_data.keys()).cloned().collect();
            }
            let prefix = format!("{}/", relative_dir.trim_end_matches('/'));
            data.keys()
                .chain(binary_data.keys())
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect()
        }

        fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError> {
            let value = self
                .data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))?;
            match value {
                serde_json::Value::String(s) => Ok(s),
                other => {
                    serde_json::to_string(&other).map_err(|source| RepositoryError::Serialize {
                        path: std::path::PathBuf::from(relative_path),
                        source,
                    })
                }
            }
        }

        fn save_text_file(
            &self,
            relative_path: &str,
            content: &str,
        ) -> Result<(), RepositoryError> {
            self.data.borrow_mut().insert(
                relative_path.to_string(),
                serde_json::Value::String(content.to_string()),
            );
            Ok(())
        }

        fn load_binary_file(&self, relative_path: &str) -> Result<Vec<u8>, RepositoryError> {
            self.binary_data
                .borrow()
                .get(relative_path)
                .cloned()
                .ok_or_else(|| not_found(relative_path))
        }

        fn save_binary_file(
            &self,
            relative_path: &str,
            content: &[u8],
        ) -> Result<(), RepositoryError> {
            self.binary_data
                .borrow_mut()
                .insert(relative_path.to_string(), content.to_vec());
            Ok(())
        }

        fn validate_package_ref_path(&self, _relative_path: &str) -> Result<(), RepositoryError> {
            Ok(())
        }

        fn load_manifest_raw_text(&self) -> Result<String, RepositoryError> {
            // Route through to_value so the flattened `extra` HashMap keys are
            // normalised into BTreeMap order before serialization (ADR-017, ADR-033).
            let value = serde_json::to_value(&*self.manifest.borrow()).map_err(|e| {
                RepositoryError::Serialize {
                    path: PathBuf::from("manifest.json"),
                    source: e,
                }
            })?;
            serde_json::to_string_pretty(&value).map_err(|e| RepositoryError::Serialize {
                path: PathBuf::from("manifest.json"),
                source: e,
            })
        }

        // --- Package boundaries ---

        fn list_package_boundaries(
            &self,
        ) -> Result<Vec<crate::package_types::PackageBoundary>, RepositoryError> {
            Ok(self.boundaries.borrow().values().cloned().collect())
        }

        fn load_package_boundary(
            &self,
            selector: &PackageSelector,
        ) -> Result<crate::package_types::PackageBoundary, RepositoryError> {
            self.boundaries
                .borrow()
                .get(selector)
                .cloned()
                .ok_or_else(|| RepositoryError::PackageNotFound {
                    selector: selector.clone(),
                })
        }

        fn save_package_boundary_metadata(
            &self,
            boundary: &crate::package_types::PackageBoundary,
        ) -> Result<(), RepositoryError> {
            let mut boundaries = self.boundaries.borrow_mut();
            let entry = boundaries
                .entry(boundary.selector.clone())
                .or_insert_with(|| boundary.clone());
            entry.id = boundary.id.clone();
            entry.namespace = boundary.namespace.clone();
            entry.name = boundary.name.clone();
            entry.version = boundary.version.clone();
            // field_paths and type_paths intentionally not updated — managed by
            // add_definition_to_boundary / remove_definition_from_boundary only.
            Ok(())
        }

        fn register_package_boundary(
            &self,
            selector: &PackageSelector,
        ) -> Result<(), RepositoryError> {
            let path = match selector {
                None => return Ok(()), // primary — no-op
                Some(p) => p.clone(),
            };
            let mut boundaries = self.boundaries.borrow_mut();
            boundaries.entry(selector.clone()).or_insert_with(|| {
                crate::package_types::PackageBoundary {
                    selector: Some(path.clone()),
                    id: String::new(),
                    namespace: String::new(),
                    name: String::new(),
                    version: String::new(),
                    field_paths: vec![],
                    type_paths: vec![],
                    blueprint_paths: vec![],
                    protocol_paths: vec![],
                    view_paths: vec![],
                    relation_type_paths: vec![],
                    lifecycle_paths: vec![],
                    document_view_paths: vec![],
                }
            });
            drop(boundaries);
            // Seed the sub-package's package.json in data so memory_store_sync_pkg_json
            // can update its arrays and find_view_path can read from it.
            let data_key = format!("{path}/package.json");
            self.data.borrow_mut().entry(data_key).or_insert_with(|| {
                serde_json::json!({
                    "id": "", "namespace": "", "name": "", "version": "",
                    "fields": [], "types": [], "relationTypes": [],
                    "views": [], "documentViews": [], "blueprints": []
                })
            });
            // Also update manifest packageRefs
            let mut manifest = self.manifest.borrow().clone();
            let mut refs: Vec<serde_json::Value> = manifest
                .extra
                .get("packageRefs")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let already = refs
                .iter()
                .any(|r| r.get("path").and_then(|p| p.as_str()) == Some(&path));
            if !already {
                refs.push(serde_json::json!({"mode": "local", "path": path}));
                manifest
                    .extra
                    .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
                *self.manifest.borrow_mut() = manifest;
            }
            Ok(())
        }

        fn add_definition_to_boundary(
            &self,
            selector: &PackageSelector,
            kind: crate::package_types::DefinitionKind,
            path: &str,
        ) -> Result<(), RepositoryError> {
            {
                let mut boundaries = self.boundaries.borrow_mut();
                let boundary = boundaries.get_mut(selector).ok_or_else(|| {
                    RepositoryError::PackageNotFound {
                        selector: selector.clone(),
                    }
                })?;
                match kind {
                    crate::package_types::DefinitionKind::Field
                        if !boundary.field_paths.iter().any(|p| p == path) =>
                    {
                        boundary.field_paths.push(path.to_string());
                    }
                    crate::package_types::DefinitionKind::Type
                        if !boundary.type_paths.iter().any(|p| p == path) =>
                    {
                        boundary.type_paths.push(path.to_string());
                    }
                    crate::package_types::DefinitionKind::Blueprint
                        if !boundary.blueprint_paths.iter().any(|p| p == path) =>
                    {
                        boundary.blueprint_paths.push(path.to_string());
                    }
                    crate::package_types::DefinitionKind::Protocol
                        if !boundary.protocol_paths.iter().any(|p| p == path) =>
                    {
                        boundary.protocol_paths.push(path.to_string());
                    }
                    _ => {} // View/DocumentView/RelationType/Vocabulary/Lifecycle — resolved via
                            // package.json in the data map (memory_store_sync_pkg_json)
                }
            }
            // Sync the data["<prefix>/package.json"] so load_package_json stays consistent
            self.memory_store_sync_pkg_json(selector, kind, path, true)
        }

        fn remove_definition_from_boundary(
            &self,
            selector: &PackageSelector,
            kind: crate::package_types::DefinitionKind,
            path: &str,
        ) -> Result<(), RepositoryError> {
            {
                let mut boundaries = self.boundaries.borrow_mut();
                if let Some(boundary) = boundaries.get_mut(selector) {
                    match kind {
                        crate::package_types::DefinitionKind::Field => {
                            boundary.field_paths.retain(|p| p != path);
                        }
                        crate::package_types::DefinitionKind::Type => {
                            boundary.type_paths.retain(|p| p != path);
                        }
                        crate::package_types::DefinitionKind::Blueprint => {
                            boundary.blueprint_paths.retain(|p| p != path);
                        }
                        crate::package_types::DefinitionKind::Protocol => {
                            boundary.protocol_paths.retain(|p| p != path);
                        }
                        _ => {}
                    }
                }
            }
            self.memory_store_sync_pkg_json(selector, kind, path, false)
        }

        fn resolve_definition_owner(
            &self,
            id: &str,
            kind: crate::package_types::DefinitionKind,
        ) -> Result<PackageSelector, RepositoryError> {
            use crate::store::definition_kind_key;
            let boundaries = self.boundaries.borrow();
            for (selector, boundary) in boundaries.iter() {
                let prefix = match selector {
                    None => "package".to_string(),
                    Some(p) => p.clone(),
                };
                // For Field/Type use the in-memory boundary paths (fast path).
                // For View/DocumentView/RelationType, read from the boundary's package.json in data.
                match kind {
                    crate::package_types::DefinitionKind::Field => {
                        for rel_path in &boundary.field_paths {
                            let data_key = format!("{prefix}/{rel_path}");
                            if let Some(val) = self.data.borrow().get(&data_key) {
                                if val["id"].as_str() == Some(id) {
                                    return Ok(selector.clone());
                                }
                            }
                        }
                    }
                    crate::package_types::DefinitionKind::Type => {
                        for rel_path in &boundary.type_paths {
                            let data_key = format!("{prefix}/{rel_path}");
                            if let Some(val) = self.data.borrow().get(&data_key) {
                                if val["id"].as_str() == Some(id) {
                                    return Ok(selector.clone());
                                }
                            }
                        }
                    }
                    _ => {
                        // For View, DocumentView, RelationType: scan the boundary's package.json
                        let pkg_key = format!("{prefix}/package.json");
                        let array_key = definition_kind_key(kind);
                        let data = self.data.borrow();
                        if let Some(pkg_json) = data.get(&pkg_key) {
                            if let Some(paths) = pkg_json[array_key].as_array() {
                                for entry in paths {
                                    if let Some(rel) = entry.as_str() {
                                        let def_key = format!("{prefix}/{rel}");
                                        if let Some(val) = data.get(&def_key) {
                                            if val["id"].as_str() == Some(id) {
                                                return Ok(selector.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(RepositoryError::DefinitionNotFound { id: id.to_string() })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::memory::MemoryStore;
    use super::*;
    use srs_core::types::record::FieldValues;
    use tempfile::TempDir;

    fn minimal_manifest(repo_root: &std::path::Path) -> Manifest {
        Manifest {
            instance_index: vec![],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: repo_root.to_path_buf(),
        }
    }

    fn empty_package(repo_root: &std::path::Path) -> Package {
        Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: repo_root.to_path_buf(),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn write_minimal_file_repo(temp: &TempDir) {
        let root = temp.path();
        std::fs::create_dir_all(root.join("package")).unwrap();

        let manifest = serde_json::json!({
            "instanceIndex": [],
            "srsVersion": "2.0-draft",
            "repositoryId": "test-repo-id",
            "namespace": "com.test"
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let package_json = serde_json::json!({
            "id": "test-pkg",
            "namespace": "com.test",
            "name": "test",
            "version": "1.0.0",
            "fields": [],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&package_json).unwrap(),
        )
        .unwrap();
    }

    // --- FileStore tests ---

    #[test]
    fn file_store_field_instructions_roundtrip() {
        use crate::package_service::create_field;
        use srs_core::types::field::{AiGuidance, Field, FieldType};

        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());

        let field = Field {
            schema: None,
            id: "00000000-0000-0000-0000-aabbccddee02".to_string(),
            namespace: "com.test".to_string(),
            name: "help-field".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "A help field".to_string(),
            instructions: Some("Fill this in carefully.".to_string()),
            ai_guidance: AiGuidance::default(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        create_field(&store, field.clone()).unwrap();

        // Reload via a fresh FileStore instance to prove the value survives an
        // on-disk roundtrip, not just an in-process cache.
        let reloaded = FileStore::new(temp.path());
        let package = reloaded.load_package().unwrap();
        let loaded_field = package
            .fields
            .iter()
            .find(|f| f.id == field.id)
            .expect("field must be present after reload");
        assert_eq!(
            loaded_field.instructions,
            Some("Fill this in carefully.".to_string())
        );
    }

    #[test]
    fn file_store_load_manifest_roundtrips() {
        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let manifest = store.load_manifest().unwrap();
        assert!(manifest.instance_index.is_empty());
        assert_eq!(manifest.root, temp.path());
    }

    #[test]
    fn file_store_manifest_container_and_container_index_roundtrip() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("package")).unwrap();

        let manifest_json = serde_json::json!({
            "instanceIndex": [],
            "container": {
                "containerId": "aaaaaaaa-0000-4000-8000-000000000001",
                "identityInstanceId": "bbbbbbbb-0000-4000-8000-000000000002"
            },
            "containerIndex": [
                {
                    "containerId": "aaaaaaaa-0000-4000-8000-000000000001",
                    "title": "Root",
                    "path": "containers/root.json"
                }
            ]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let manifest = store.load_manifest().unwrap();

        // Typed fields populated
        let container = manifest
            .container
            .as_ref()
            .expect("container should be Some");
        assert_eq!(
            container.container_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
        assert_eq!(
            container.identity_instance_id.as_deref(),
            Some("bbbbbbbb-0000-4000-8000-000000000002")
        );

        let index = manifest
            .container_index
            .as_ref()
            .expect("container_index should be Some");
        assert_eq!(index.len(), 1);
        assert_eq!(
            index[0].container_id,
            "aaaaaaaa-0000-4000-8000-000000000001"
        );
        assert_eq!(index[0].title.as_deref(), Some("Root"));
        assert_eq!(index[0].path.as_deref(), Some("containers/root.json"));

        // Not duplicated in extra
        assert!(!manifest.extra.contains_key("container"));
        assert!(!manifest.extra.contains_key("containerIndex"));

        // Write back and reload — verify round-trip
        store.save_manifest(&manifest).unwrap();
        let reloaded = store.load_manifest().unwrap();
        assert_eq!(reloaded.container, manifest.container);
        assert_eq!(reloaded.container_index, manifest.container_index);
    }

    #[test]
    fn file_store_load_package_returns_package() {
        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let package = store.load_package().unwrap();
        assert_eq!(package.namespace, "com.test");
        assert!(package
            .fields
            .iter()
            .any(|f| f.namespace == "com.semanticops.core"));
    }

    #[test]
    fn load_package_includes_core_types() {
        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let package = store.load_package().unwrap();
        assert!(
            package
                .resolve_type_by_name("com.semanticops.core", "purpose")
                .is_some(),
            "FileStore::load_package must include core purpose type"
        );
    }

    #[test]
    fn load_package_memory_store_includes_core_types() {
        let root = std::path::PathBuf::from("/fake");
        let store = memory::MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let package = store.load_package().unwrap();
        assert!(
            package
                .resolve_type_by_name("com.semanticops.core", "purpose")
                .is_some(),
            "MemoryStore::load_package must include core purpose type"
        );
    }

    #[test]
    fn load_package_repo_declaring_core_field_conflicts() {
        let temp = TempDir::new().unwrap();
        write_minimal_file_repo(&temp);

        // Write a field file that reuses the core statement field's ID
        let fields_dir = temp.path().join("package/fields");
        std::fs::create_dir_all(&fields_dir).unwrap();
        let conflict_field = serde_json::json!({
            "id": "3b000001-0000-4000-a000-000000000001",
            "namespace": "com.test",
            "name": "shadow-field",
            "version": 1,
            "valueType": "string",
            "description": "This conflicts with the core statement field.",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::write(
            fields_dir.join("shadow-field.json"),
            serde_json::to_string_pretty(&conflict_field).unwrap(),
        )
        .unwrap();

        // Update package.json to reference the conflicting field
        let package_json = serde_json::json!({
            "id": "test-pkg",
            "namespace": "com.test",
            "name": "test",
            "version": "1.0.0",
            "fields": ["fields/shadow-field.json"],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            temp.path().join("package/package.json"),
            serde_json::to_string_pretty(&package_json).unwrap(),
        )
        .unwrap();

        let store = FileStore::new(temp.path());
        let result = store.load_package();
        assert!(
            matches!(
                result,
                Err(RepositoryError::CorePackageConflict { ref kind, .. }) if kind == "field"
            ),
            "expected CorePackageConflict(field), got: {result:?}"
        );
    }

    // --- MemoryStore tests ---

    #[test]
    fn memory_store_load_manifest_returns_configured() {
        let root = std::path::PathBuf::from("/fake");
        let manifest = minimal_manifest(&root);
        let store = MemoryStore::new(manifest.clone(), empty_package(&root));
        let loaded = store.load_manifest().unwrap();
        assert_eq!(loaded.instance_index, manifest.instance_index);
    }

    #[test]
    fn memory_store_save_and_load_instance_json() {
        let root = std::path::PathBuf::from("/fake");
        let store = MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let value = serde_json::json!({ "instanceId": "abc-123", "title": "Test" });
        store
            .save_instance_json("notes/test-note.json", &value)
            .unwrap();
        let loaded = store.load_instance_json("notes/test-note.json").unwrap();
        assert_eq!(loaded["instanceId"], "abc-123");
    }

    #[test]
    fn memory_store_delete_instance_removes_key() {
        let root = std::path::PathBuf::from("/fake");
        let store = MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let value = serde_json::json!({ "instanceId": "to-delete" });
        store
            .save_instance_json("notes/to-delete.json", &value)
            .unwrap();
        store.delete_instance_file("notes/to-delete.json").unwrap();
        let result = store.load_instance_json("notes/to-delete.json");
        assert!(result.is_err());
    }

    #[test]
    fn memory_store_list_instance_files_direct_children_only() {
        let root = std::path::PathBuf::from("/fake");
        let store = MemoryStore::new(minimal_manifest(&root), empty_package(&root));
        let v = serde_json::json!({});
        store
            .save_instance_json("records/notes/a.json", &v)
            .unwrap();
        store
            .save_instance_json("records/notes/b.json", &v)
            .unwrap();
        // nested — must NOT appear when listing records/notes
        store
            .save_instance_json("records/notes/subdir/c.json", &v)
            .unwrap();
        // sibling directory — must NOT appear
        store
            .save_instance_json("records/other/d.json", &v)
            .unwrap();

        let mut files = store.list_instance_files("records/notes").unwrap();
        files.sort();
        assert_eq!(
            files,
            vec![
                "records/notes/a.json".to_string(),
                "records/notes/b.json".to_string(),
            ]
        );
    }

    // --- Container store tests ---

    fn minimal_container_for_store(id: &str, title: &str) -> srs_core::types::container::Container {
        srs_core::types::container::Container {
            container_id: id.to_string(),
            title: title.to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            tags: None,
            root_instance_ids: None,
            member_instance_ids: None,
            created_at: None,
            updated_at: None,
            meta: None,
            identity_instance_id: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn memory_store_container_operations_are_keyed_by_id() {
        let store = MemoryStore::default();
        let container = minimal_container_for_store("c-111", "Sprint 1");

        store.save_container(&container).unwrap();

        // Load back via logical ID
        let loaded = store.load_container("c-111").unwrap();
        assert_eq!(loaded.container_id, "c-111");
        assert_eq!(loaded.title, "Sprint 1");

        // load_instance_json at id-keyed path must succeed (proves storage is id-keyed)
        store
            .load_instance_json("containers/c-111.json")
            .expect("container should be stored at id-keyed path");
    }

    #[test]
    fn memory_store_container_summaries_reflects_saves() {
        let store = MemoryStore::default();
        for i in 1..=3u32 {
            store
                .save_container(&minimal_container_for_store(
                    &format!("cid-{i}"),
                    &format!("Container {i}"),
                ))
                .unwrap();
        }
        let summaries = store.list_container_summaries().unwrap();
        assert_eq!(summaries.len(), 3);
        assert!(summaries.iter().any(|(id, _)| id == "cid-1"));
        assert!(summaries.iter().any(|(id, _)| id == "cid-2"));
        assert!(summaries.iter().any(|(id, _)| id == "cid-3"));
    }

    #[test]
    fn memory_store_delete_container_removes_entry() {
        let store = MemoryStore::default();
        store
            .save_container(&minimal_container_for_store("del-me", "Delete Me"))
            .unwrap();

        store.delete_container("del-me").unwrap();

        let err = store.load_container("del-me").unwrap_err();
        assert!(
            matches!(err, RepositoryError::ContainerNotFound { .. }),
            "should get ContainerNotFound after delete"
        );
        let summaries = store.list_container_summaries().unwrap();
        assert!(
            !summaries.iter().any(|(id, _)| id == "del-me"),
            "summary index should not contain deleted container"
        );
    }

    #[test]
    fn memory_store_delete_container_missing_returns_not_found() {
        let store = MemoryStore::default();
        let err = store.delete_container("nonexistent").unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));
    }

    #[test]
    fn save_container_file_first_failed_index_leaves_orphaned_data_safe() {
        // ADR-007: with file-before-index ordering, a failed index update after a successful
        // data write must leave the data in the backing store (orphaned, invisible to readers) but
        // no dangling index entry. Proves the invariant: the index is always internally consistent.
        use memory::FailPoint;

        let store = MemoryStore::empty();
        let container = minimal_container_for_store("c-test-adr007", "ADR-007 Test");

        store.arm_fail_at(FailPoint::SaveContainerIndex);
        let result = store.save_container(&container);

        // The call must have failed (simulating a crash between file write and index update)
        assert!(
            matches!(result, Err(RepositoryError::Io { .. })),
            "save_container should return Io error when SaveContainerIndex fail point is armed"
        );

        // Data was written before the injected failure — orphaned entry present in backing store (safe)
        assert!(
            store
                .all_data()
                .contains_key("containers/c-test-adr007.json"),
            "container data should exist as an orphaned entry after failed index update"
        );

        // Index must NOT have an entry — no dangling index entry
        let summaries = store.list_container_summaries().unwrap();
        assert!(
            summaries.is_empty(),
            "container index must have no entry after failed index update (no dangling entry)"
        );
    }

    // --- Instance store tests (ADR-042) ---

    fn minimal_record_for_store(id: &str, type_name: &str, tags: Option<Vec<String>>) -> Record {
        Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "type-xyz-0001".to_string(),
            type_version: 1,
            type_namespace: "com.example".to_string(),
            type_name: type_name.to_string(),
            field_values: FieldValues::new(),
            lifecycle_state: None,
            tags,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn minimal_note_for_store(id: &str, title: &str, tags: Option<Vec<String>>) -> Note {
        Note {
            instance_id: id.to_string(),
            title: Some(title.to_string()),
            tags,
            sections: vec![],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    #[test]
    fn save_record_roundtrip_by_id_across_stores() {
        let store = MemoryStore::empty();
        let rec = minimal_record_for_store("rec-00000001-aaaa", "Decision", Some(vec!["u".into()]));
        store.save_record(&rec).unwrap();

        let loaded = store.load_record_by_id("rec-00000001-aaaa").unwrap();
        assert_eq!(loaded.instance_id, "rec-00000001-aaaa");
        assert_eq!(loaded.type_name, "Decision");

        // memory -> file
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = file_store.load_record_by_id("rec-00000001-aaaa").unwrap();
        assert_eq!(from_file.instance_id, "rec-00000001-aaaa");
        assert_eq!(from_file.type_name, "Decision");
    }

    #[test]
    fn save_note_roundtrip_by_id_across_stores() {
        let store = MemoryStore::empty();
        let note = minimal_note_for_store("note-00000002-bbbb", "My Note", None);
        store.save_note(&note).unwrap();

        let loaded = store.load_note_by_id("note-00000002-bbbb").unwrap();
        assert_eq!(loaded.instance_id, "note-00000002-bbbb");
        assert_eq!(loaded.title.as_deref(), Some("My Note"));

        // note is Tier 0
        let r = store.find_instance("note-00000002-bbbb").unwrap().unwrap();
        assert_eq!(r.tier, 0);

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = file_store.load_note_by_id("note-00000002-bbbb").unwrap();
        assert_eq!(from_file.title.as_deref(), Some("My Note"));
    }

    // --- Standalone relation objects (RFC-038 Change E) ---

    fn minimal_relation_for_store(id: &str) -> srs_core::types::relation::Relation {
        srs_core::types::relation::Relation {
            relation_id: id.to_string(),
            relation_type: "precedes".to_string(),
            source_instance_id: "aaaa0001-0000-4000-a000-000000000001".to_string(),
            target_instance_id: "aaaa0002-0000-4000-a000-000000000002".to_string(),
            asserted_by: None,
            confidence: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        }
    }

    #[test]
    fn save_relation_roundtrip_across_stores() {
        // memory
        let store = MemoryStore::empty();
        let rel = minimal_relation_for_store("d0000001-0000-4000-a000-000000000001");
        store.save_relation(&rel).unwrap();
        let loaded = store
            .load_relation("d0000001-0000-4000-a000-000000000001")
            .unwrap();
        assert_eq!(loaded, rel);

        // file
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = FileStore::new(temp.path());
        file_store.save_relation(&rel).unwrap();
        let from_file = file_store
            .load_relation("d0000001-0000-4000-a000-000000000001")
            .unwrap();
        assert_eq!(from_file, rel);
        // one object per file, at the derived locator, with the pinned $schema
        let raw = file_store
            .load_relations_json("relations/d0000001-0000-4000-a000-000000000001.json")
            .unwrap();
        assert_eq!(
            raw.get("$schema").and_then(|v| v.as_str()),
            Some(RELATION_OBJECT_SCHEMA_URL)
        );
        assert_eq!(
            store.list_relations().unwrap(),
            file_store.list_relations().unwrap()
        );
    }

    #[test]
    fn load_relation_filename_mismatch_is_error() {
        // [R11]: the in-file relationId is authoritative; a disagreeing filename is
        // an error naming both.
        let store = MemoryStore::empty();
        let rel = minimal_relation_for_store("d0000001-0000-4000-a000-00000000000b");
        let mut value = serde_json::to_value(&rel).unwrap();
        value.as_object_mut().unwrap().insert(
            "$schema".to_string(),
            serde_json::json!(RELATION_OBJECT_SCHEMA_URL),
        );
        store
            .save_relations_json(
                "relations/d0000001-0000-4000-a000-00000000000a.json",
                &value,
            )
            .unwrap();

        let err = store
            .load_relation("d0000001-0000-4000-a000-00000000000a")
            .unwrap_err();
        assert!(
            matches!(err, RepositoryError::RelationFilenameMismatch { .. }),
            "expected RelationFilenameMismatch, got {err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("d0000001-0000-4000-a000-00000000000a"));
        assert!(msg.contains("d0000001-0000-4000-a000-00000000000b"));

        let list_err = store.list_relations().unwrap_err();
        assert!(matches!(
            list_err,
            RepositoryError::RelationFilenameMismatch { .. }
        ));
    }

    #[test]
    fn load_relation_missing_schema_is_error() {
        let store = MemoryStore::empty();
        let rel = minimal_relation_for_store("d0000001-0000-4000-a000-00000000000c");
        let value = serde_json::to_value(&rel).unwrap();
        store
            .save_relations_json(
                "relations/d0000001-0000-4000-a000-00000000000c.json",
                &value,
            )
            .unwrap();
        let err = store
            .load_relation("d0000001-0000-4000-a000-00000000000c")
            .unwrap_err();
        assert!(
            matches!(err, RepositoryError::SchemaValidation { .. }),
            "expected SchemaValidation for missing $schema, got {err:?}"
        );
    }

    #[test]
    fn list_relations_skips_collection_files_and_sorts() {
        let store = MemoryStore::empty();
        // A collection file must not be treated as a relation object.
        store
            .save_relations_json(
                "relations/relations-collection.json",
                &serde_json::json!({ "relations": [] }),
            )
            .unwrap();
        let b = minimal_relation_for_store("d0000002-0000-4000-a000-000000000002");
        let a = minimal_relation_for_store("d0000001-0000-4000-a000-000000000001");
        store.save_relation(&b).unwrap();
        store.save_relation(&a).unwrap();

        let listed = store.list_relations().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|r| r.relation_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "d0000001-0000-4000-a000-000000000001",
                "d0000002-0000-4000-a000-000000000002"
            ],
            "ascending byte-wise relationId order"
        );
    }

    #[test]
    fn delete_relation_removes_only_its_file() {
        let store = MemoryStore::empty();
        let a = minimal_relation_for_store("d0000001-0000-4000-a000-000000000001");
        let b = minimal_relation_for_store("d0000002-0000-4000-a000-000000000002");
        store.save_relation(&a).unwrap();
        store.save_relation(&b).unwrap();

        store
            .delete_relation("d0000001-0000-4000-a000-000000000001")
            .unwrap();
        let listed = store.list_relations().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].relation_id,
            "d0000002-0000-4000-a000-000000000002"
        );

        let err = store
            .delete_relation("d0000001-0000-4000-a000-000000000001")
            .unwrap_err();
        assert!(matches!(err, RepositoryError::RelationNotFound { .. }));
    }

    #[test]
    fn save_record_existing_id_preserves_path() {
        // Existing-id save overwrites at the existing indexed path (no rename on slug change).
        let store = MemoryStore::empty();
        let rec = minimal_record_for_store("rec-00000003-cccc", "OldTypeName", None);
        store.save_record(&rec).unwrap();
        let path1 = store
            .load_manifest()
            .unwrap()
            .instance_index
            .iter()
            .find(|e| e.instance_id == "rec-00000003-cccc")
            .unwrap()
            .path
            .clone();

        // Type-version migration changes type_name (hence the slug) — path must NOT change.
        let mut rec2 = rec.clone();
        rec2.type_name = "BrandNewTypeName".to_string();
        rec2.tags = Some(vec!["added".to_string()]);
        store.save_record(&rec2).unwrap();

        let entry2 = store
            .load_manifest()
            .unwrap()
            .instance_index
            .into_iter()
            .find(|e| e.instance_id == "rec-00000003-cccc")
            .unwrap();
        assert_eq!(
            entry2.path, path1,
            "existing-id save must not rename the file"
        );
        // Denormalized index tags refreshed from the entity.
        assert_eq!(entry2.tags.as_deref(), Some(&["added".to_string()][..]));
    }

    #[test]
    fn save_record_file_first_failed_index_leaves_orphaned_data_safe() {
        // ADR-007: a failed index update after a successful data write leaves orphaned data,
        // no dangling index entry. Mirrors the container fault-injection test.
        use memory::FailPoint;
        let store = MemoryStore::empty();
        let rec = minimal_record_for_store("rec-00000004-dddd", "Thing", None);

        store.arm_fail_at(FailPoint::SaveInstanceIndex);
        let result = store.save_record(&rec);
        assert!(
            matches!(result, Err(RepositoryError::Io { .. })),
            "save_record should return Io error when SaveInstanceIndex fail point is armed"
        );
        // Data written before the injected failure — orphaned entry present.
        assert!(
            store
                .all_data()
                .keys()
                .any(|k| k.contains("rec-00000004-dddd") || k.starts_with("records/tier-2")),
            "record data should exist as an orphaned entry after failed index update"
        );
        // No dangling index entry.
        assert!(
            store.find_instance("rec-00000004-dddd").unwrap().is_none(),
            "instance index must have no entry after a failed index update"
        );
    }

    #[test]
    fn delete_instance_index_first_and_not_found() {
        let store = MemoryStore::empty();
        let rec = minimal_record_for_store("rec-00000005-eeee", "Thing", None);
        store.save_record(&rec).unwrap();
        assert!(store.find_instance("rec-00000005-eeee").unwrap().is_some());

        store.delete_instance("rec-00000005-eeee").unwrap();
        assert!(store.find_instance("rec-00000005-eeee").unwrap().is_none());
        assert!(
            store.load_record_by_id("rec-00000005-eeee").is_err(),
            "loading a deleted instance must error"
        );

        // Unknown id -> InstanceNotFound.
        assert!(matches!(
            store.delete_instance("does-not-exist"),
            Err(RepositoryError::InstanceNotFound { .. })
        ));
    }

    #[test]
    fn list_instances_filters_by_tier_and_tag_from_index() {
        let store = MemoryStore::empty();
        store
            .save_record(&minimal_record_for_store(
                "rec-00000006-ffff",
                "R",
                Some(vec!["alpha".to_string()]),
            ))
            .unwrap();
        store
            .save_note(&minimal_note_for_store(
                "note-00000007-0000",
                "N",
                Some(vec!["beta".to_string()]),
            ))
            .unwrap();

        let all = store.list_instances(&InstanceQuery::default()).unwrap();
        assert_eq!(all.len(), 2, "default query returns every instance");

        let tier2 = store
            .list_instances(&InstanceQuery {
                tier: Some(2),
                tag: None,
            })
            .unwrap();
        assert_eq!(tier2.len(), 1);
        assert_eq!(tier2[0].instance_id, "rec-00000006-ffff");

        let tagged = store
            .list_instances(&InstanceQuery {
                tier: None,
                tag: Some("beta".to_string()),
            })
            .unwrap();
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0].instance_id, "note-00000007-0000");
        assert_eq!(tagged[0].tier, 0);
    }

    #[test]
    fn load_note_by_id_validates_note_body() {
        // Read-side parity with loader::load_note (arch-review finding): an invalid
        // note body (duplicate section names) must surface NoteValidation on load,
        // not be silently accepted.
        let store = MemoryStore::empty();
        let mut note = minimal_note_for_store("note-00000009-2222", "Dup Sections", None);
        let section = srs_core::types::note::NoteSection {
            name: "s1".to_string(),
            label: None,
            content: "a".to_string(),
            content_hint: None,
            tags: None,
        };
        note.sections = vec![section.clone(), section];
        // Bypass write-side validation deliberately: write the invalid body via the
        // typed save (save_note does not validate; creation-time validation lives in
        // note_service::create).
        store.save_note(&note).unwrap();

        let err = store.load_note_by_id("note-00000009-2222").unwrap_err();
        assert!(
            matches!(err, RepositoryError::NoteValidation { .. }),
            "invalid note body must fail load with NoteValidation, got: {err:?}"
        );
    }

    #[test]
    fn find_instance_returns_tier_or_none() {
        let store = MemoryStore::empty();
        store
            .save_record(&minimal_record_for_store("rec-00000008-1111", "R", None))
            .unwrap();
        let found = store.find_instance("rec-00000008-1111").unwrap();
        assert_eq!(found.map(|r| r.tier), Some(2));
        assert!(store.find_instance("missing").unwrap().is_none());
    }

    // --- Package boundary tests ---

    #[test]
    fn memory_store_save_field_uses_package_prefix_key() {
        // This test is load-bearing: it proves MemoryStore stores fields at
        // "package/fields/..." rather than bare "fields/...", which is the key
        // invariant that makes resolve_definition_owner work correctly.
        use crate::package_service::create_field;
        use srs_core::types::field::{AiGuidance, Field, FieldType};

        let store = MemoryStore::default();
        let field = Field {
            schema: None,
            id: "00000000-0000-0000-0000-aabbccddee01".to_string(),
            namespace: "com.test".to_string(),
            name: "my-field".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: String::new(),
            instructions: None,
            ai_guidance: AiGuidance::default(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        create_field(&store, field).unwrap();

        let data = store.all_data();
        // Must have a key under "package/fields/..."
        let has_package_prefix = data
            .keys()
            .any(|k| k.starts_with("package/fields/") && k.contains("my-field"));
        assert!(
            has_package_prefix,
            "field should be stored at package/fields/... but keys were: {:?}",
            data.keys().collect::<Vec<_>>()
        );
        // Must NOT have a bare "fields/..." key
        let has_bare = data
            .keys()
            .any(|k| k.starts_with("fields/") && k.contains("my-field"));
        assert!(!has_bare, "field should not be stored at bare fields/...");
    }

    #[test]
    fn memory_store_list_package_boundaries_returns_primary() {
        let store = MemoryStore::default();
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 1);
        assert!(
            boundaries[0].selector.is_none(),
            "primary boundary has None selector"
        );
    }

    #[test]
    fn memory_store_register_sub_package_adds_to_boundaries() {
        let store = MemoryStore::default();
        store
            .register_package_boundary(&Some("pkg/ext".to_string()))
            .unwrap();
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 2);
        let has_ext = boundaries
            .iter()
            .any(|b| b.selector == Some("pkg/ext".to_string()));
        assert!(has_ext, "sub-package boundary should be registered");
    }

    #[test]
    fn memory_store_add_definition_to_boundary_updates_paths() {
        use crate::package_types::DefinitionKind;

        let store = MemoryStore::default();
        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/foo.json")
            .unwrap();
        let boundary = store.load_package_boundary(&None).unwrap();
        assert!(
            boundary
                .field_paths
                .contains(&"fields/foo.json".to_string()),
            "field path should appear in primary boundary field_paths"
        );
    }

    #[test]
    fn memory_store_resolve_definition_owner_primary() {
        use crate::package_types::DefinitionKind;
        use srs_core::types::field::{AiGuidance, Field, FieldType};

        let store = MemoryStore::default();
        let field_id = "00000000-0000-0000-0000-111111111111";
        let field = Field {
            schema: None,
            id: field_id.to_string(),
            namespace: "com.test".to_string(),
            name: "resolve-me".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: String::new(),
            instructions: None,
            ai_guidance: AiGuidance::default(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        // Store field data at the primary package key
        store
            .add_definition_to_boundary(
                &None,
                DefinitionKind::Field,
                "fields/resolve-me-11111111.json",
            )
            .unwrap();
        store
            .save_instance_json(
                "package/fields/resolve-me-11111111.json",
                &serde_json::to_value(&field).unwrap(),
            )
            .unwrap();

        let owner = store
            .resolve_definition_owner(field_id, DefinitionKind::Field)
            .unwrap();
        assert!(owner.is_none(), "primary boundary owner should be None");
    }

    #[test]
    fn memory_store_resolve_definition_owner_sub_package() {
        use crate::package_types::DefinitionKind;
        use srs_core::types::field::{AiGuidance, Field, FieldType};

        let store = MemoryStore::default();
        let selector = Some("pkg/ext".to_string());
        store.register_package_boundary(&selector).unwrap();

        let field_id = "00000000-0000-0000-0000-222222222222";
        let field = Field {
            schema: None,
            id: field_id.to_string(),
            namespace: "com.test".to_string(),
            name: "sub-field".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: String::new(),
            instructions: None,
            ai_guidance: AiGuidance::default(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store
            .add_definition_to_boundary(
                &selector,
                DefinitionKind::Field,
                "fields/sub-field-22222222.json",
            )
            .unwrap();
        store
            .save_instance_json(
                "pkg/ext/fields/sub-field-22222222.json",
                &serde_json::to_value(&field).unwrap(),
            )
            .unwrap();

        let owner = store
            .resolve_definition_owner(field_id, DefinitionKind::Field)
            .unwrap();
        assert_eq!(
            owner, selector,
            "sub-package owner should be Some(\"pkg/ext\")"
        );
    }

    #[test]
    fn file_store_package_boundary_maps_existing_layout() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Set up primary package
        std::fs::create_dir_all(root.join("package/fields")).unwrap();
        let manifest = serde_json::json!({
            "instanceIndex": [],
            "srsVersion": "2.0-draft",
            "repositoryId": "boundary-test",
            "namespace": "com.test",
            "packageRefs": [{"mode": "local", "path": "extensions/myext"}]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let primary_pkg = serde_json::json!({
            "id": "primary-pkg",
            "namespace": "com.test",
            "name": "primary",
            "version": "1.0.0",
            "fields": ["fields/field-aaa.json"],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&primary_pkg).unwrap(),
        )
        .unwrap();

        // Set up sub-package
        std::fs::create_dir_all(root.join("extensions/myext")).unwrap();
        let sub_pkg = serde_json::json!({
            "id": "ext-pkg",
            "namespace": "com.test.ext",
            "name": "myext",
            "version": "0.1.0",
            "fields": [],
            "types": [],
            "views": [],
            "documentViews": []
        });
        std::fs::write(
            root.join("extensions/myext/package.json"),
            serde_json::to_string_pretty(&sub_pkg).unwrap(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 2, "should have primary + 1 sub-package");

        let primary = boundaries.iter().find(|b| b.selector.is_none()).unwrap();
        assert_eq!(primary.id, "primary-pkg");
        assert_eq!(primary.field_paths, vec!["fields/field-aaa.json"]);

        let ext = boundaries
            .iter()
            .find(|b| b.selector == Some("extensions/myext".to_string()))
            .unwrap();
        assert_eq!(ext.id, "ext-pkg");
    }

    #[test]
    fn loaded_blueprint_source_package_filestore() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Primary package with one blueprint.
        std::fs::create_dir_all(root.join("package/blueprints")).unwrap();
        let manifest = serde_json::json!({
            "instanceIndex": [],
            "srsVersion": "2.0-draft",
            "repositoryId": "bp-prov-test",
            "namespace": "com.test",
            "packageRefs": [{"mode": "local", "path": "extensions/subpkg"}]
        });
        std::fs::write(
            root.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let primary_pkg = serde_json::json!({
            "id": "primary-pkg",
            "namespace": "com.test",
            "name": "primary",
            "version": "1.0.0",
            "fields": [],
            "types": [],
            "views": [],
            "documentViews": [],
            "blueprints": ["blueprints/root-bp.json"]
        });
        std::fs::write(
            root.join("package/package.json"),
            serde_json::to_string_pretty(&primary_pkg).unwrap(),
        )
        .unwrap();
        let root_bp = serde_json::json!({
            "id": "root-bp-001",
            "namespace": "com.test",
            "name": "Root Blueprint",
            "version": 1,
            "description": "Root package blueprint",
            "rootTypes": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::write(
            root.join("package/blueprints/root-bp.json"),
            serde_json::to_string_pretty(&root_bp).unwrap(),
        )
        .unwrap();

        // Sub-package with one blueprint.
        std::fs::create_dir_all(root.join("extensions/subpkg/blueprints")).unwrap();
        let sub_pkg = serde_json::json!({
            "id": "sub-pkg-001",
            "namespace": "com.test.ext",
            "name": "subpkg",
            "version": "1.0.0",
            "fields": [],
            "types": [],
            "views": [],
            "documentViews": [],
            "blueprints": ["blueprints/sub-bp.json"]
        });
        std::fs::write(
            root.join("extensions/subpkg/package.json"),
            serde_json::to_string_pretty(&sub_pkg).unwrap(),
        )
        .unwrap();
        let sub_bp = serde_json::json!({
            "id": "sub-bp-002",
            "namespace": "com.test.ext",
            "name": "Sub Blueprint",
            "version": 1,
            "description": "Sub-package blueprint",
            "rootTypes": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::write(
            root.join("extensions/subpkg/blueprints/sub-bp.json"),
            serde_json::to_string_pretty(&sub_bp).unwrap(),
        )
        .unwrap();

        let store = FileStore::new(root);
        let package = store.load_package().unwrap();
        assert_eq!(package.blueprints.len(), 2);

        let root_loaded = package
            .blueprints
            .iter()
            .find(|lb| lb.blueprint.id == "root-bp-001")
            .expect("root blueprint must be present");
        assert_eq!(
            root_loaded.source_package, None,
            "root package blueprint must have source_package = None"
        );

        let sub_loaded = package
            .blueprints
            .iter()
            .find(|lb| lb.blueprint.id == "sub-bp-002")
            .expect("sub-package blueprint must be present");
        assert_eq!(
            sub_loaded.source_package,
            Some("extensions/subpkg".to_string()),
            "sub-package blueprint must carry source_package = Some(rel_path)"
        );
    }

    #[test]
    fn record_tier_dir_values() {
        let store = MemoryStore::empty();
        assert_eq!(store.record_tier_dir(RecordTier::Note), "records/notes");
        assert_eq!(store.record_tier_dir(RecordTier::Tier1), "records/tier-1");
        assert_eq!(store.record_tier_dir(RecordTier::Tier2), "records/tier-2");
        assert_eq!(
            store.record_tier_dir(RecordTier::Extension),
            "package/records"
        );
    }

    // --- FailPoint smoke tests ---

    #[test]
    fn memory_store_save_manifest_fail_point_triggers_error() {
        use memory::FailPoint;
        let store = MemoryStore::empty();
        store.arm_fail_at(FailPoint::SaveManifest);
        let manifest = store.load_manifest().unwrap();
        let err = store.save_manifest(&manifest);
        assert!(
            matches!(err, Err(RepositoryError::Io { .. })),
            "armed SaveManifest should return Io error"
        );
        // One-shot: second call succeeds
        store
            .save_manifest(&manifest)
            .expect("fail point must be disarmed after first fire");
    }

    #[test]
    fn memory_store_delete_instance_file_fail_point_triggers_error() {
        use memory::FailPoint;
        let store = MemoryStore::empty();
        let path = "records/test.json";
        store
            .save_instance_json(path, &serde_json::json!({"x": 1}))
            .unwrap();
        store.arm_fail_at(FailPoint::DeleteInstanceFile);
        let err = store.delete_instance_file(path);
        assert!(
            matches!(err, Err(RepositoryError::Io { .. })),
            "armed DeleteInstanceFile should return Io error"
        );
        // One-shot: second call succeeds
        store
            .delete_instance_file(path)
            .expect("fail point must be disarmed after first fire");
    }

    #[test]
    fn memory_store_with_fail_at_builder_and_disarm() {
        use memory::FailPoint;
        // with_fail_at arms at construction time
        let store = MemoryStore::empty().with_fail_at(FailPoint::SaveManifest);
        let manifest = store.load_manifest().unwrap();
        let err = store.save_manifest(&manifest);
        assert!(
            matches!(err, Err(RepositoryError::Io { .. })),
            "with_fail_at should arm the point at construction"
        );
        // Re-arm then explicitly disarm — subsequent call must succeed
        store.arm_fail_at(FailPoint::SaveManifest);
        store.disarm_fail_at();
        store
            .save_manifest(&manifest)
            .expect("disarmed point must not fire");
    }

    #[test]
    fn memory_store_fail_point_does_not_cross_contaminate() {
        use memory::FailPoint;
        // Arming SaveManifest must not consume the point when delete_instance_file is called
        let store = MemoryStore::empty();
        let path = "records/cross.json";
        store
            .save_instance_json(path, &serde_json::json!({}))
            .unwrap();
        store.arm_fail_at(FailPoint::SaveManifest);
        // delete_instance_file does not match — point must survive
        store.delete_instance_file(path).unwrap();
        // save_manifest now fires the armed point
        let manifest = store.load_manifest().unwrap();
        let err = store.save_manifest(&manifest);
        assert!(
            matches!(err, Err(RepositoryError::Io { .. })),
            "SaveManifest point must still fire after a delete_instance_file call"
        );
    }

    #[test]
    fn binary_file_roundtrip_memory() {
        let store = MemoryStore::empty();
        let bytes = b"binary\x00\x01\x02\xffcontent";
        store
            .save_binary_file("source-documents/doc.pdf", bytes)
            .expect("save_binary_file must succeed on MemoryStore");
        let loaded = store
            .load_binary_file("source-documents/doc.pdf")
            .expect("load_binary_file must return bytes that were saved");
        assert_eq!(loaded, bytes);
    }

    #[test]
    fn binary_file_roundtrip_memory_not_found() {
        let store = MemoryStore::empty();
        let err = store
            .load_binary_file("source-documents/absent.pdf")
            .expect_err("load_binary_file must return an error for absent path");
        assert!(
            err.is_not_found(),
            "absent binary file must return a not-found error, got: {err:?}"
        );
    }

    #[test]
    fn binary_file_roundtrip_file() {
        let temp = tempfile::TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let bytes = b"binary\x00\x01\x02\xffcontent";
        store
            .save_binary_file("source-documents/doc.pdf", bytes)
            .expect("save_binary_file must succeed on FileStore");
        let loaded = store
            .load_binary_file("source-documents/doc.pdf")
            .expect("load_binary_file must return bytes that were saved");
        assert_eq!(loaded, bytes);
    }

    #[test]
    fn binary_file_roundtrip_file_not_found() {
        let temp = tempfile::TempDir::new().unwrap();
        write_minimal_file_repo(&temp);
        let store = FileStore::new(temp.path());
        let err = store
            .load_binary_file("source-documents/absent.pdf")
            .expect_err("load_binary_file must return an error for absent path");
        assert!(
            err.is_not_found(),
            "absent binary file must return a not-found error, got: {err:?}"
        );
    }
}
