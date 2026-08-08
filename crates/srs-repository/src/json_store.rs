use crate::error::RepositoryError;
use crate::field_json::FieldJson;
use crate::index::{InstanceIndexEntry, InstanceQuery, InstanceRef};
use crate::manifest::Manifest;
use crate::package::Package;
use crate::package_types::PackageBoundary;
use crate::repository_lifecycle::{
    default_repository_container, CreateRepositoryResult, InitializeRepositoryInput,
};
use crate::store::{
    instance_filename, note_from_value, note_to_value, record_to_value, RecordTier, RepositoryStore,
};
use chrono::Utc;
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
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(serde::Serialize, serde::Deserialize)]
struct JsonStoreFile {
    srsj: String,
    manifest: serde_json::Value,
    // BTreeMap (not HashMap) so the `.srsj` envelope serialises entries in
    // deterministic, sorted key order — minimal-diff, idempotent writes (ADR-017).
    data: BTreeMap<String, serde_json::Value>,
}

struct JsonStoreState {
    initialized: bool,
    manifest: Manifest,
    // BTreeMap for deterministic `.srsj` serialisation — see JsonStoreFile.data (ADR-017).
    data: BTreeMap<String, serde_json::Value>,
    // In-memory binary file storage for archive-loaded repositories (ADR-031 amendment).
    // Not serialised to `.srsj` — binary content is excluded from the JSON-only format per RFC-017.
    binary_files: HashMap<String, Vec<u8>>,
    // When true, flush() is suppressed until commit_batch() is called (ADR-021).
    batching: bool,
}

pub struct JsonStore {
    file_path: PathBuf,
    state: RefCell<JsonStoreState>,
}

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
    vocabularies: Vec<String>,
    #[serde(default)]
    lifecycles: Vec<String>,
}

impl JsonStore {
    pub fn create(file_path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let file_path = file_path.into();
        if file_path.exists() {
            return Err(RepositoryError::RepositoryAlreadyExists {
                path: file_path.clone(),
            });
        }
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
            root: file_path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_path_buf(),
        };
        let store = Self {
            file_path: file_path.clone(),
            state: RefCell::new(JsonStoreState {
                initialized: false,
                manifest,
                data: BTreeMap::new(),
                binary_files: HashMap::new(),
                batching: false,
            }),
        };
        store.flush()?;
        Ok(store)
    }

    /// Load a repository from a `.srsj` JSON string without touching the filesystem.
    ///
    /// `manifest.root` is set to `"."` — acceptable for read-only use because the `.srsj`
    /// format embeds all package definitions inline and requires no external path resolution.
    pub fn from_srsj(content: &str) -> Result<Self, RepositoryError> {
        let mem_path = PathBuf::from("<memory>");
        let envelope: JsonStoreFile =
            serde_json::from_str(content).map_err(|source| RepositoryError::Serialize {
                path: mem_path.clone(),
                source,
            })?;
        if envelope.srsj != "1" {
            return Err(RepositoryError::InvalidSnapshotData {
                message: format!("unsupported srsj version '{}'", envelope.srsj),
            });
        }
        let mut raw_manifest = envelope.manifest;
        crate::manifest::migrate_upstream_package(&mut raw_manifest);
        let mut manifest: Manifest = serde_json::from_value(raw_manifest).map_err(|source| {
            RepositoryError::ManifestParse {
                path: mem_path.clone(),
                source,
            }
        })?;
        manifest.root = PathBuf::from(".");
        // --- Open-time migrations ---
        // Invariants that every migration block here must observe:
        //   1. Must not call flush() — from_srsj is pure / WASM-safe (no filesystem writes).
        //   2. Malformed or unresolvable entries are silently discarded, not a load error.
        //   3. Each migration is idempotent: running from_srsj on already-migrated input
        //      produces the same result.
        //   4. No diagnostic output is possible from a constructor; document skip reasons
        //      with inline comments only.
        //
        // Reconcile index entries: if an entry is missing tags but the bundled record
        // file has tags, fill them in. `list_records_filtered` trusts the index for
        // tag queries without loading the file, so a stale or absent `tags` field
        // silently drops records from tag discovery (#406).
        for entry in &mut manifest.instance_index {
            if entry.tags.is_none() {
                if let Some(record_json) = envelope.data.get(entry.path()) {
                    if let Some(tags) = Self::extract_tags_from_record_json(record_json) {
                        entry.tags = Some(tags);
                    }
                }
            }
        }
        // Open-time migration: if manifest.container_index is None (old JsonStore-native .srsj
        // repos written before #466, which stored containerIndex only in data["manifest.json"]),
        // populate the typed index from that shadow so all subsequent saves/deletes operate
        // on the canonical index rather than leaving the shadow stale (#466).
        if manifest.container_index.is_none() {
            if let Some(shadow_manifest) = envelope.data.get("manifest.json") {
                if let Some(arr) = shadow_manifest
                    .get("containerIndex")
                    .and_then(|v| v.as_array())
                {
                    let migrated: Vec<ContainerIndexEntry> = arr
                        .iter()
                        .filter_map(|e| {
                            let mut entry: ContainerIndexEntry =
                                serde_json::from_value(e.clone()).ok()?;
                            if entry.path.is_none() {
                                // Pre-#466 shadow entries carry only {containerId, title}.
                                // Derive the path from the data map using the pre-#466
                                // JsonStore convention; skip entries with no matching key
                                // rather than importing a pathless entry that would fail
                                // schema validation ([/containerIndex/N] "path" is required).
                                let derived = Self::container_data_key(&entry.container_id);
                                if envelope.data.contains_key(&derived) {
                                    entry.path = Some(derived);
                                } else {
                                    return None;
                                }
                            }
                            Some(entry)
                        })
                        .collect();
                    if !migrated.is_empty() {
                        manifest.container_index = Some(migrated);
                    }
                }
            }
        }
        Ok(Self {
            file_path: mem_path,
            state: RefCell::new(JsonStoreState {
                initialized: true,
                manifest,
                data: envelope.data,
                binary_files: HashMap::new(),
                batching: false,
            }),
        })
    }

    /// Create an uninitialized in-memory store.
    ///
    /// The sentinel path `<memory>` suppresses all `flush()` calls. `initialized: false`
    /// satisfies `ensure_target_empty` in `import_repository_snapshot`, making this suitable
    /// as the target for `archive_unpack` and similar import operations.
    pub(crate) fn new_in_memory() -> Self {
        Self {
            file_path: PathBuf::from("<memory>"),
            state: RefCell::new(JsonStoreState {
                initialized: false,
                manifest: Manifest {
                    root: PathBuf::from("."),
                    ..Manifest::default()
                },
                data: BTreeMap::new(),
                binary_files: HashMap::new(),
                batching: false,
            }),
        }
    }

    /// Load a repository from a `.srs` binary archive (ZIP bytes) into an in-memory store.
    ///
    /// Analogous to [`Self::from_srsj`] for the binary archive format. The backing store uses
    /// `<memory>` as its sentinel path so no filesystem writes occur.
    pub fn from_archive(bytes: &[u8]) -> Result<Self, RepositoryError> {
        let cursor = std::io::Cursor::new(bytes);
        let store = Self::new_in_memory();
        crate::archive::archive_unpack(cursor, &store)?;
        Ok(store)
    }

    /// Returns the canonical data-map key for a container stored in JsonStore-native format.
    /// Used in save_container, load_container (fallback), delete_container, and the open-time
    /// migration that derives paths for pre-#466 shadow containerIndex entries (#490).
    fn container_data_key(container_id: &str) -> String {
        format!("containers/{container_id}.json")
    }

    /// Resolve an instance's data-map key (path) from the manifest index (ADR-042).
    fn json_instance_path(&self, instance_id: &str) -> Result<String, RepositoryError> {
        self.state
            .borrow()
            .manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id == instance_id)
            .map(|e| e.path.clone())
            .ok_or_else(|| RepositoryError::InstanceNotFound {
                id: instance_id.to_string(),
            })
    }

    /// The two-branch instance save shared by `save_record`/`save_note` (mirrors
    /// `save_container`): existing id ⇒ overwrite at the existing path (path + tier
    /// preserved, denormalized `title`/`tags` refreshed); new id ⇒ derive a filename
    /// and write data before index (ADR-007). `flush()` honours the batch seam (ADR-021).
    #[allow(clippy::too_many_arguments)]
    fn json_save_instance(
        &self,
        instance_id: &str,
        value: serde_json::Value,
        tier_dir: &str,
        slug_source: &str,
        new_tier: u8,
        title: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<(), RepositoryError> {
        {
            let mut state = self.state.borrow_mut();
            let existing = state
                .manifest
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
            state.data.insert(path.clone(), value);
            let entry = InstanceIndexEntry {
                instance_id: instance_id.to_string(),
                tier,
                path,
                title: title.map(serde_json::Value::String),
                tags,
            };
            if let Some(pos) = state
                .manifest
                .instance_index
                .iter()
                .position(|e| e.instance_id == instance_id)
            {
                state.manifest.instance_index[pos] = entry;
            } else {
                state.manifest.instance_index.push(entry);
            }
        }
        self.flush()
    }

    /// Extract the `tags` array from a record JSON value.
    /// Returns `None` if the field is absent, null, or not a valid `Vec<String>`.
    fn extract_tags_from_record_json(record_json: &serde_json::Value) -> Option<Vec<String>> {
        record_json
            .get("tags")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn open(file_path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let file_path = file_path.into();
        let raw = std::fs::read_to_string(&file_path).map_err(|source| RepositoryError::Io {
            path: file_path.clone(),
            source,
        })?;
        let mut store = Self::from_srsj(&raw).map_err(|e| match e {
            RepositoryError::Serialize { source, .. } => RepositoryError::Serialize {
                path: file_path.clone(),
                source,
            },
            RepositoryError::ManifestParse { source, .. } => RepositoryError::ManifestParse {
                path: file_path.clone(),
                source,
            },
            other => other,
        })?;
        store.file_path = file_path;
        Ok(store)
    }

    /// Returns the repository's current state as a `.srsj` JSON string.
    /// Pure: no filesystem access. Safe to call from WASM.
    pub fn to_srsj_string(&self) -> Result<String, RepositoryError> {
        let state = self.state.borrow();
        let manifest =
            serde_json::to_value(&state.manifest).map_err(|source| RepositoryError::Serialize {
                path: self.file_path.clone(),
                source,
            })?;
        let envelope = JsonStoreFile {
            srsj: "1".to_string(),
            manifest,
            data: state.data.clone(),
        };
        serde_json::to_string_pretty(&envelope).map_err(|source| RepositoryError::Serialize {
            path: self.file_path.clone(),
            source,
        })
    }

    fn flush(&self) -> Result<(), RepositoryError> {
        // Batch mode: defer disk writes until commit_batch() is called (ADR-021).
        if self.state.borrow().batching {
            return Ok(());
        }
        // In-memory stores (loaded from a string via `from_srsj`) use the sentinel
        // path "<memory>" and must not attempt file I/O. This is the normal
        // operating mode for the WASM browser binding.
        if self.file_path == std::path::Path::new("<memory>") {
            return Ok(());
        }
        let json = self.to_srsj_string()?;
        std::fs::write(&self.file_path, json).map_err(|source| RepositoryError::Io {
            path: self.file_path.clone(),
            source,
        })
    }

    fn not_found(path: &str) -> RepositoryError {
        RepositoryError::Io {
            path: PathBuf::from(path),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "not found in JsonStore"),
        }
    }

    fn data_get(&self, path: &str) -> Result<serde_json::Value, RepositoryError> {
        self.state
            .borrow()
            .data
            .get(path)
            .cloned()
            .ok_or_else(|| Self::not_found(path))
    }

    #[allow(clippy::type_complexity)]
    fn load_package_from_prefix(
        &self,
        package_prefix: &str,
        rt_by_type: &mut HashMap<String, (RelationTypeDefinition, PathBuf)>,
    ) -> Result<
        (
            PackageMetadata,
            Vec<Field>,
            Vec<RecordType>,
            Vec<View>,
            Vec<DocumentView>,
            Vec<Theme>,
            Vec<crate::package::LoadedProtocol>,
            Vec<Vocabulary>,
            Vec<Lifecycle>,
            Vec<crate::package::LoadedBlueprint>,
        ),
        RepositoryError,
    > {
        let package_json_path = format!("{package_prefix}/package.json");
        let package_json = self.data_get(&package_json_path)?;
        let metadata: PackageMetadata = serde_json::from_value(package_json).map_err(|source| {
            RepositoryError::PackageLoad {
                path: PathBuf::from(&package_json_path),
                source,
            }
        })?;

        let mut fields = Vec::new();
        for rel_path in &metadata.fields {
            let full = format!("{package_prefix}/{rel_path}");
            let fj: FieldJson =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            fields.push(fj.into_field(&PathBuf::from(&full))?);
        }

        let mut record_types = Vec::new();
        for rel_path in &metadata.types {
            let full = format!("{package_prefix}/{rel_path}");
            let tj: crate::type_json::TypeJson = serde_json::from_value(self.data_get(&full)?)
                .map_err(|source| RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                })?;
            record_types.push(tj.into_record_type());
        }

        for rel_path in &metadata.relation_types {
            let full = format!("{package_prefix}/{rel_path}");
            let def: RelationTypeDefinition = serde_json::from_value(self.data_get(&full)?)
                .map_err(|source| RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                })?;
            validate_relation_type_definition(&def).map_err(|source| {
                RepositoryError::RelationTypeDefinitionValidation {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            if let Some((existing, existing_path)) = rt_by_type.get(&def.key) {
                if existing != &def {
                    return Err(RepositoryError::RelationTypeDefinitionConflict {
                        relation_type: def.key.clone(),
                        path_a: existing_path.clone(),
                        path_b: PathBuf::from(full),
                    });
                }
            } else {
                rt_by_type.insert(def.key.clone(), (def, PathBuf::from(full)));
            }
        }

        let mut views = Vec::new();
        for rel_path in &metadata.views {
            let full = format!("{package_prefix}/{rel_path}");
            let view: View = serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                RepositoryError::ViewLoad {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            validate_view(&view).map_err(|source| RepositoryError::ViewValidation {
                path: PathBuf::from(&full),
                source,
            })?;
            views.push(view);
        }

        let mut document_views = Vec::new();
        for rel_path in &metadata.document_views {
            let full = format!("{package_prefix}/{rel_path}");
            let view: DocumentView =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::DocumentViewLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            validate_document_view(&view).map_err(|source| {
                RepositoryError::DocumentViewValidation {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            document_views.push(view);
        }

        let mut themes = Vec::new();
        for rel_path in &metadata.themes {
            let full = format!("{package_prefix}/{rel_path}");
            let theme: Theme = serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                }
            })?;
            validate_theme(&theme).map_err(|source| RepositoryError::ThemeValidation {
                path: PathBuf::from(&full),
                source,
            })?;
            themes.push(theme);
        }

        let mut vocabularies = Vec::new();
        for rel_path in &metadata.vocabularies {
            let full = format!("{package_prefix}/{rel_path}");
            let vocab: Vocabulary =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            vocabularies.push(vocab);
        }

        let mut protocols = Vec::new();
        for rel_path in &metadata.protocols {
            let full = format!("{package_prefix}/{rel_path}");
            let raw: serde_json::Value = self.data_get(&full)?;
            let protocol: srs_core::types::protocol::Protocol = serde_json::from_value(raw.clone())
                .map_err(|source| RepositoryError::PackageLoad {
                    path: PathBuf::from(&full),
                    source,
                })?;
            protocols.push(crate::package::LoadedProtocol {
                protocol,
                raw,
                source_package: None,
            });
        }

        let mut lifecycles = Vec::new();
        for rel_path in &metadata.lifecycles {
            let full = format!("{package_prefix}/{rel_path}");
            let lc: Lifecycle =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            lifecycles.push(lc);
        }

        let mut blueprints: Vec<crate::package::LoadedBlueprint> = Vec::new();
        for rel_path in &metadata.blueprints {
            let full = format!("{package_prefix}/{rel_path}");
            let blueprint: srs_core::types::blueprint::Blueprint =
                serde_json::from_value(self.data_get(&full)?).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: PathBuf::from(&full),
                        source,
                    }
                })?;
            blueprints.push(crate::package::LoadedBlueprint {
                blueprint,
                source_package: None,
            });
        }

        Ok((
            metadata,
            fields,
            record_types,
            views,
            document_views,
            themes,
            protocols,
            vocabularies,
            lifecycles,
            blueprints,
        ))
    }
}

impl RepositoryStore for JsonStore {
    fn repository_root(&self) -> PathBuf {
        match self.file_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    fn repository_exists(&self) -> Result<bool, RepositoryError> {
        Ok(self.state.borrow().initialized)
    }

    fn initialize_repository(
        &self,
        input: &InitializeRepositoryInput,
    ) -> Result<CreateRepositoryResult, RepositoryError> {
        if self.repository_exists()? {
            return Err(RepositoryError::RepositoryAlreadyExists {
                path: self.file_path.clone(),
            });
        }
        // `extra` is a HashMap (insertion order non-deterministic), but that is safe for
        // both `.srsj` determinism and archive pack determinism: `to_srsj_string` serialises
        // the manifest via `serde_json::to_value`, which normalises the flattened keys into
        // sorted order through serde_json's BTreeMap-backed Map; `load_text_file("manifest.json")`
        // applies the same `to_value` step before `to_string_pretty` (ADR-017, ADR-033).
        // Only the top-level `data` map is serialised directly, which is why it (and not this)
        // had to become a BTreeMap (ADR-017).
        let title = input
            .repository
            .title
            .as_deref()
            .unwrap_or_default()
            .to_string();
        let created_at = Utc::now().to_rfc3339();
        let mut extra = std::collections::BTreeMap::new();
        extra.insert(
            "$schema".to_string(),
            serde_json::Value::String(srs_schema::MANIFEST_SCHEMA_ID.to_string()),
        );
        extra.insert(
            "srsVersion".to_string(),
            serde_json::Value::String(input.repository.srs_version.clone()),
        );
        extra.insert(
            "repositoryId".to_string(),
            serde_json::Value::String(input.repository.repository_id.clone()),
        );
        extra.insert(
            "namespace".to_string(),
            serde_json::Value::String(input.repository.namespace.clone()),
        );
        extra.insert(
            "title".to_string(),
            serde_json::Value::String(title.clone()),
        );
        if let Some(desc) = &input.repository.description {
            extra.insert(
                "description".to_string(),
                serde_json::Value::String(desc.clone()),
            );
        }
        extra.insert(
            "createdAt".to_string(),
            serde_json::Value::String(created_at),
        );
        let container = default_repository_container(&input.repository.repository_id, &title);
        let mut state = self.state.borrow_mut();
        state.manifest = Manifest {
            instance_index: vec![],
            container: Some(container),
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra,
            source_documents_path: None,
            source_document_index: None,
            root: self.repository_root(),
        };
        state.data.insert(
            "package/package.json".to_string(),
            serde_json::json!({
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
                "documentViews": []
            }),
        );
        state.initialized = true;
        drop(state);
        self.flush()?;
        Ok(CreateRepositoryResult {
            repo_root: self.repository_root(),
            repository_id: input.repository.repository_id.clone(),
            package_id: input.primary_package.id.clone(),
            identity_instance_id: None,
        })
    }

    fn load_manifest(&self) -> Result<Manifest, RepositoryError> {
        let mut manifest = self.state.borrow().manifest.clone();
        manifest.root = self.repository_root();
        Ok(manifest)
    }

    fn save_manifest(&self, manifest: &Manifest) -> Result<(), RepositoryError> {
        self.state.borrow_mut().manifest = manifest.clone();
        self.flush()
    }

    fn begin_batch(&self) {
        self.state.borrow_mut().batching = true;
    }

    fn commit_batch(&self) -> Result<(), crate::error::RepositoryError> {
        self.state.borrow_mut().batching = false;
        self.flush()
    }

    fn abort_batch(&self) {
        self.state.borrow_mut().batching = false;
        // Restore in-memory state from disk so subsequent writes on this
        // instance don't flush partial import data (ADR-021). For the WASM
        // path (file_path == "<memory>"), there is no disk to reload from;
        // callers must not reuse a memory-backed store after abort_batch.
        //
        // Silent-failure contract: if the disk read or deserialisation fails
        // (e.g. the file was deleted between begin_batch and abort_batch), the
        // in-memory state is NOT restored and still holds partial import data.
        // `batching` is already cleared, so a subsequent flush() call on this
        // instance would write that partial state to disk. Callers MUST treat
        // an abort as terminal: propagate the import error and drop the store.
        if self.file_path != std::path::Path::new("<memory>") {
            if let Ok(raw) = std::fs::read_to_string(&self.file_path) {
                if let Ok(mut envelope) = serde_json::from_str::<JsonStoreFile>(&raw) {
                    crate::manifest::migrate_upstream_package(&mut envelope.manifest);
                    if let Ok(mut manifest) =
                        serde_json::from_value::<crate::manifest::Manifest>(envelope.manifest)
                    {
                        manifest.root = self.repository_root();
                        let mut state = self.state.borrow_mut();
                        state.manifest = manifest;
                        state.data = envelope.data;
                        // `initialized` has no on-disk representation; `true` matches
                        // the JsonStore::open() convention for an existing, readable file.
                        state.initialized = true;
                    }
                }
            }
        }
    }

    fn load_package(&self) -> Result<Package, RepositoryError> {
        let manifest = self.load_manifest()?;
        let mut rt_by_type: HashMap<String, (RelationTypeDefinition, PathBuf)> = HashMap::new();
        let (
            root_meta,
            mut fields,
            mut record_types,
            mut views,
            mut document_views,
            mut themes,
            mut protocols,
            mut vocabularies,
            mut lifecycles,
            mut blueprints,
        ) = self.load_package_from_prefix("package", &mut rt_by_type)?;

        if let Some(pkg_refs) = manifest.extra.get("packageRefs").and_then(|v| v.as_array()) {
            let mut field_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut type_sources: HashMap<(String, u32), PathBuf> = HashMap::new();
            let mut view_sources: HashMap<String, PathBuf> = HashMap::new();
            let mut doc_view_sources: HashMap<String, PathBuf> = HashMap::new();
            for f in &fields {
                field_sources.insert(f.id.clone(), PathBuf::from("package"));
            }
            for rt in &record_types {
                type_sources.insert((rt.id.clone(), rt.version), PathBuf::from("package"));
            }
            for v in &views {
                view_sources.insert(v.id.clone(), PathBuf::from("package"));
            }
            for dv in &document_views {
                doc_view_sources.insert(dv.id.clone(), PathBuf::from("package"));
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
                let (
                    ..,
                    sub_fields,
                    sub_types,
                    sub_views,
                    sub_doc_views,
                    sub_themes,
                    sub_protocols,
                    sub_vocabs,
                    sub_lcs,
                    sub_blueprints,
                ) = self.load_package_from_prefix(rel_path, &mut rt_by_type)?;

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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        field_sources.insert(field.id.clone(), PathBuf::from(rel_path));
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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        type_sources.insert(key, PathBuf::from(rel_path));
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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        view_sources.insert(view.id.clone(), PathBuf::from(rel_path));
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
                                second_path: PathBuf::from(rel_path),
                            });
                        }
                    } else {
                        doc_view_sources.insert(dv.id.clone(), PathBuf::from(rel_path));
                        document_views.push(dv);
                    }
                }
                // Themes: first definition of each id wins (primary package takes precedence
                // over sub-packages). Silent skip matches the bundled-theme lookup model
                // where themes are identified by stable UUID — duplicate IDs in different
                // packages indicate a packaging error, not a semantic override.
                for theme in sub_themes {
                    if !themes.iter().any(|t| t.id == theme.id) {
                        themes.push(theme);
                    }
                }
                for mut lp in sub_protocols {
                    if !protocols
                        .iter()
                        .any(|p| p.protocol.protocol_id == lp.protocol.protocol_id)
                    {
                        lp.source_package = Some(rel_path.to_string());
                        protocols.push(lp);
                    }
                }
                for vocab in sub_vocabs {
                    if !vocabularies.iter().any(|v| v.id == vocab.id) {
                        vocabularies.push(vocab);
                    }
                }
                for lc in sub_lcs {
                    if !lifecycles.iter().any(|l| l.id == lc.id) {
                        lifecycles.push(lc);
                    }
                }
                for mut lb in sub_blueprints {
                    if !blueprints.iter().any(|b| b.blueprint.id == lb.blueprint.id) {
                        lb.source_package = Some(rel_path.to_string());
                        blueprints.push(lb);
                    }
                }
            }
        }

        // Sort by (key, id) so this Vec is deterministic regardless of HashMap
        // iteration order — keeps regenerated package indexes stable across runs.
        let mut relation_type_definitions: Vec<RelationTypeDefinition> =
            rt_by_type.into_values().map(|(def, _)| def).collect();
        relation_type_definitions.sort_by(|a, b| a.key.cmp(&b.key).then(a.id.cmp(&b.id)));

        crate::core_package::merge_core_into_package(
            &mut fields,
            &mut record_types,
            &mut relation_type_definitions,
        )?;

        Ok(Package {
            id: root_meta.id,
            namespace: root_meta.namespace,
            name: root_meta.name,
            version: root_meta.version,
            fields,
            record_types,
            relation_type_definitions,
            views,
            document_views,
            themes,
            blueprints,
            protocols,
            root: self.repository_root(),
            dependency_refs: vec![],
            vocabularies,
            lifecycles,
        })
    }

    fn load_package_json(&self) -> Result<serde_json::Value, RepositoryError> {
        self.data_get("package/package.json")
    }

    fn save_package_json(&self, value: &serde_json::Value) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert("package/package.json".to_string(), value.clone());
        self.flush()
    }

    fn save_field(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(field).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_field_file(&self, relative_path: &str, field: &Field) -> Result<(), RepositoryError> {
        self.save_field(relative_path, field)
    }

    fn delete_field_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_fields_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_type(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(record_type).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_type_file(
        &self,
        relative_path: &str,
        record_type: &RecordType,
    ) -> Result<(), RepositoryError> {
        self.save_type(relative_path, record_type)
    }

    fn delete_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_relation_type_definition(
        &self,
        relative_path: &str,
        relation_type: &RelationTypeDefinition,
    ) -> Result<(), RepositoryError> {
        let v =
            serde_json::to_value(relation_type).map_err(|source| RepositoryError::Serialize {
                path: PathBuf::from(relative_path),
                source,
            })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn delete_relation_type_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_relation_types_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_view(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(view).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_view_file(&self, relative_path: &str, view: &View) -> Result<(), RepositoryError> {
        self.save_view(relative_path, view)
    }

    fn delete_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_document_view(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(view).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_document_view_file(
        &self,
        relative_path: &str,
        view: &DocumentView,
    ) -> Result<(), RepositoryError> {
        self.save_document_view(relative_path, view)
    }

    fn delete_document_view_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_document_views_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_theme(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(theme).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_theme_file(
        &self,
        relative_path: &str,
        theme: &srs_core::types::theme::Theme,
    ) -> Result<(), RepositoryError> {
        if !self.state.borrow().data.contains_key(relative_path) {
            return Err(RepositoryError::NotFound {
                path: PathBuf::from(relative_path),
            });
        }
        self.save_theme(relative_path, theme)
    }

    fn delete_theme_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_themes_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_blueprint(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(blueprint).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn update_blueprint_file(
        &self,
        relative_path: &str,
        blueprint: &srs_core::types::blueprint::Blueprint,
    ) -> Result<(), RepositoryError> {
        self.save_blueprint(relative_path, blueprint)
    }

    fn delete_blueprint_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_blueprints_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_vocabulary(
        &self,
        relative_path: &str,
        vocabulary: &srs_core::types::vocabulary::Vocabulary,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(vocabulary).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn ensure_vocabularies_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn save_lifecycle(
        &self,
        relative_path: &str,
        lifecycle: &srs_core::types::lifecycle::Lifecycle,
    ) -> Result<(), RepositoryError> {
        let v = serde_json::to_value(lifecycle).map_err(|source| RepositoryError::Serialize {
            path: PathBuf::from(relative_path),
            source,
        })?;
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), v);
        self.flush()
    }

    fn ensure_lifecycles_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn load_instance_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.data_get(relative_path)
    }

    fn save_instance_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), value.clone());
        self.flush()
    }

    fn delete_instance_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    fn ensure_instance_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn list_instance_files(&self, relative_dir: &str) -> Result<Vec<String>, RepositoryError> {
        let prefix = if relative_dir.ends_with('/') {
            relative_dir.to_string()
        } else {
            format!("{relative_dir}/")
        };
        let out = self
            .state
            .borrow()
            .data
            .keys()
            .filter(|k| {
                k.starts_with(&prefix) && k.ends_with(".json") && !k[prefix.len()..].contains('/')
            })
            .cloned()
            .collect();
        Ok(out)
    }

    fn record_tier_dir(&self, tier: RecordTier) -> &'static str {
        match tier {
            RecordTier::Note => "records/notes",
            RecordTier::Tier1 => "records/tier-1",
            RecordTier::Tier2 => "records/tier-2",
            RecordTier::Extension => "package/records",
        }
    }

    // --- Instances (logical-id + typed; ADR-042) ---

    fn save_record(&self, record: &Record) -> Result<(), RepositoryError> {
        let val = record_to_value(record)?;
        self.json_save_instance(
            &record.instance_id,
            val,
            self.record_tier_dir(RecordTier::Tier2),
            &record.type_name,
            2,
            None,
            record.tags.clone(),
        )
    }

    fn save_note(&self, note: &Note) -> Result<(), RepositoryError> {
        let val = note_to_value(note)?;
        self.json_save_instance(
            &note.instance_id,
            val,
            self.record_tier_dir(RecordTier::Note),
            note.title.as_deref().unwrap_or(""),
            0,
            note.title.clone(),
            note.tags.clone(),
        )
    }

    fn load_record_by_id(&self, instance_id: &str) -> Result<Record, RepositoryError> {
        let path = self.json_instance_path(instance_id)?;
        let val = self.data_get(&path)?;
        serde_json::from_value(val).map_err(|source| RepositoryError::RecordLoad {
            path: std::path::PathBuf::from(&path),
            source,
        })
    }

    fn load_note_by_id(&self, instance_id: &str) -> Result<Note, RepositoryError> {
        let path = self.json_instance_path(instance_id)?;
        let val = self.data_get(&path)?;
        // Parity with loader::load_note: parse (NoteLoad) + validate_note (NoteValidation).
        note_from_value(val, &path)
    }

    fn delete_instance(&self, instance_id: &str) -> Result<(), RepositoryError> {
        let path = self.json_instance_path(instance_id)?;
        {
            let mut state = self.state.borrow_mut();
            // ADR-007: remove the index entry before the data.
            state
                .manifest
                .instance_index
                .retain(|e| e.instance_id != instance_id);
            state.data.remove(&path);
        }
        self.flush()
    }

    fn find_instance(&self, instance_id: &str) -> Result<Option<InstanceRef>, RepositoryError> {
        Ok(self
            .state
            .borrow()
            .manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id == instance_id)
            .map(InstanceRef::from_index_entry))
    }

    fn list_instances(&self, query: &InstanceQuery) -> Result<Vec<InstanceRef>, RepositoryError> {
        Ok(self
            .state
            .borrow()
            .manifest
            .instance_index
            .iter()
            .filter(|e| query.matches(e))
            .map(InstanceRef::from_index_entry)
            .collect())
    }

    fn load_relations_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.data_get(relative_path)
    }

    fn save_relations_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), value.clone());
        self.flush()
    }

    fn ensure_relations_dir(&self, _relative_dir: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn load_container(
        &self,
        container_id: &str,
    ) -> Result<srs_core::types::container::Container, RepositoryError> {
        // Resolve via manifest.containerIndex.path first (covers .srsj packed from FileStore,
        // where containers live at slug-id8 paths rather than uuid-keyed paths).
        let indexed_path = self
            .state
            .borrow()
            .manifest
            .container_index
            .as_deref()
            .and_then(|entries| entries.iter().find(|e| e.container_id == container_id))
            .and_then(|e| e.path.clone());

        let (key, val) = if let Some(path) = indexed_path {
            match self.data_get(&path) {
                Ok(v) => (path, v),
                Err(_) => {
                    let fallback = Self::container_data_key(container_id);
                    let v = self.data_get(&fallback).map_err(|_| {
                        RepositoryError::ContainerNotFound {
                            container_id: container_id.to_string(),
                        }
                    })?;
                    (fallback, v)
                }
            }
        } else {
            let fallback = Self::container_data_key(container_id);
            let v = self
                .data_get(&fallback)
                .map_err(|_| RepositoryError::ContainerNotFound {
                    container_id: container_id.to_string(),
                })?;
            (fallback, v)
        };

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
        let key = Self::container_data_key(id);
        let val = serde_json::to_value(container).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from(&key),
            source,
        })?;
        {
            let mut state = self.state.borrow_mut();
            state.data.insert(key.clone(), val);
            // Update canonical manifest.container_index so load_container can resolve via path.
            let mut entries = state.manifest.container_index.take().unwrap_or_default();
            // Preserve any vendor-extended fields from the prior index entry so that
            // round-tripping a .srsj with extra ContainerIndexEntry fields doesn't silently drop them.
            let prior_extra = entries
                .iter()
                .find(|e| e.container_id == *id)
                .map(|e| e.extra.clone())
                .unwrap_or_default();
            entries.retain(|e| e.container_id != *id);
            entries.push(ContainerIndexEntry {
                container_id: id.clone(),
                title: Some(container.title.clone()),
                path: Some(key.clone()),
                container_type: container.container_type.clone(),
                tags: container.tags.clone(),
                extra: prior_extra,
            });
            state.manifest.container_index = Some(entries);
        }
        self.flush()
    }

    fn delete_container(&self, container_id: &str) -> Result<(), RepositoryError> {
        // Resolve the actual data key via index (covers slug-named containers from FileStore).
        let indexed_path = self
            .state
            .borrow()
            .manifest
            .container_index
            .as_deref()
            .and_then(|entries| entries.iter().find(|e| e.container_id == container_id))
            .and_then(|e| e.path.clone());
        let key = indexed_path.unwrap_or_else(|| Self::container_data_key(container_id));

        // Check existence before modifying state — return NotFound cleanly.
        if !self.state.borrow().data.contains_key(&key) {
            return Err(RepositoryError::ContainerNotFound {
                container_id: container_id.to_string(),
            });
        }

        // ADR-007: remove from index FIRST (delete ordering). An interrupted delete leaves an
        // orphaned data entry rather than a dangling index entry.
        // Keep Some([]) rather than collapsing to None — an explicitly-empty canonical index
        // signals "we own this index" and prevents list_container_summaries from falling through
        // to a stale shadow data["manifest.json"]["containerIndex"] entry.
        {
            let mut state = self.state.borrow_mut();
            let mut entries = state.manifest.container_index.take().unwrap_or_default();
            entries.retain(|e| e.container_id != container_id);
            state.manifest.container_index = Some(entries);
        }
        self.state.borrow_mut().data.remove(&key);
        self.flush()
    }

    fn list_container_summaries(&self) -> Result<Vec<(String, String)>, RepositoryError> {
        // Prefer manifest.container_index (canonical). It is populated for FileStore-packed .srsj
        // repos (where no data["manifest.json"] shadow exists) and for JsonStore-native repos after
        // any save_container call on this version. Fall through only for legacy repos that predate
        // the canonical index path.
        {
            let state = self.state.borrow();
            if let Some(entries) = state.manifest.container_index.as_deref() {
                return Ok(entries
                    .iter()
                    .map(|e| (e.container_id.clone(), e.title.clone().unwrap_or_default()))
                    .collect());
            }
        }
        // Fall back to the shadow data["manifest.json"] index (JsonStore-native repos written
        // before the canonical manifest index was populated by save_container).
        let manifest_val = self
            .data_get("manifest.json")
            .unwrap_or_else(|_| serde_json::json!({}));
        let shadow_entries: Vec<serde_json::Value> = manifest_val["containerIndex"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(shadow_entries
            .into_iter()
            .filter_map(|e| {
                let id = e["containerId"].as_str()?.to_string();
                let title = e["title"].as_str().unwrap_or("").to_string();
                Some((id, title))
            })
            .collect())
    }

    #[allow(deprecated)]
    fn load_container_json(
        &self,
        relative_path: &str,
    ) -> Result<serde_json::Value, RepositoryError> {
        self.data_get(relative_path)
    }

    #[allow(deprecated)]
    fn save_container_json(
        &self,
        relative_path: &str,
        value: &serde_json::Value,
    ) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .data
            .insert(relative_path.to_string(), value.clone());
        self.flush()
    }

    #[allow(deprecated)]
    fn delete_container_file(&self, relative_path: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.remove(relative_path);
        self.flush()
    }

    #[allow(deprecated)]
    fn ensure_containers_dir(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn list_files_recursive(&self, relative_dir: &str) -> Vec<String> {
        let state = self.state.borrow();
        // data and binary_files are disjoint per ADR-031; chain() is dedup-free.
        if relative_dir.is_empty() {
            return state
                .data
                .keys()
                .chain(state.binary_files.keys())
                .cloned()
                .collect();
        }
        let prefix = format!("{}/", relative_dir.trim_end_matches('/'));
        state
            .data
            .keys()
            .chain(state.binary_files.keys())
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect()
    }

    fn load_text_file(&self, relative_path: &str) -> Result<String, RepositoryError> {
        if relative_path == "manifest.json" {
            let manifest = self.load_manifest()?;
            // Route through to_value so the flattened `extra` HashMap keys are
            // normalised into BTreeMap order before serialization (ADR-017, ADR-033).
            let value =
                serde_json::to_value(&manifest).map_err(|source| RepositoryError::Serialize {
                    path: PathBuf::from(relative_path),
                    source,
                })?;
            return serde_json::to_string_pretty(&value).map_err(|source| {
                RepositoryError::Serialize {
                    path: PathBuf::from(relative_path),
                    source,
                }
            });
        }

        let value = self
            .state
            .borrow()
            .data
            .get(relative_path)
            .cloned()
            .ok_or_else(|| Self::not_found(relative_path))?;

        // Values written via `save_text_file` are stored as JSON strings;
        // values that arrived via the `.srsj` bundle are stored as their parsed
        // JSON types (objects, arrays). Both must be readable as text.
        match value {
            serde_json::Value::String(s) => Ok(s),
            other => serde_json::to_string(&other).map_err(|source| RepositoryError::Serialize {
                path: PathBuf::from(relative_path),
                source,
            }),
        }
    }

    fn save_text_file(&self, relative_path: &str, content: &str) -> Result<(), RepositoryError> {
        self.state.borrow_mut().data.insert(
            relative_path.to_string(),
            serde_json::Value::String(content.to_string()),
        );
        self.flush()
    }

    /// Load a binary file from the in-memory binary-file map (ADR-031 amendment).
    ///
    /// Returns the bytes when present, or a not-found error for absent paths.
    /// Binary content is populated by `archive_unpack` (via `save_binary_file`) when the
    /// repository is loaded from a `.srs` archive. Repositories loaded from `.srsj` strings
    /// never contain binary content — callers should treat a not-found result as tombstone state.
    fn load_binary_file(&self, relative_path: &str) -> Result<Vec<u8>, RepositoryError> {
        self.state
            .borrow()
            .binary_files
            .get(relative_path)
            .cloned()
            .ok_or_else(|| Self::not_found(relative_path))
    }

    /// Store a binary file in the in-memory binary-file map (ADR-031 amendment).
    ///
    /// The bytes are held in `JsonStoreState::binary_files` and are NOT serialised by
    /// `to_srsj_string()` — `.srsj` output remains binary-free per RFC-017.
    fn save_binary_file(&self, relative_path: &str, content: &[u8]) -> Result<(), RepositoryError> {
        self.state
            .borrow_mut()
            .binary_files
            .insert(relative_path.to_string(), content.to_vec());
        Ok(())
    }

    fn validate_package_ref_path(&self, _relative_path: &str) -> Result<(), RepositoryError> {
        Ok(())
    }

    // --- Package boundaries ---

    fn list_package_boundaries(
        &self,
    ) -> Result<Vec<crate::package_types::PackageBoundary>, RepositoryError> {
        let mut result = Vec::new();

        // Primary
        let primary_json = self.data_get("package/package.json")?;
        result.push(PackageBoundary::from_pkg_json(&primary_json, None));

        // Sub-packages from manifest
        let state = self.state.borrow();
        if let Some(refs) = state
            .manifest
            .extra
            .get("packageRefs")
            .and_then(|v| v.as_array())
        {
            for pkg_ref in refs {
                if pkg_ref.get("mode").and_then(|m| m.as_str()) != Some("local") {
                    continue;
                }
                if let Some(path) = pkg_ref.get("path").and_then(|p| p.as_str()) {
                    let key = format!("{path}/package.json");
                    if let Some(pkg_json) = state.data.get(&key).cloned() {
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
        selector: &crate::package_types::PackageSelector,
    ) -> Result<crate::package_types::PackageBoundary, RepositoryError> {
        let key = match selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let pkg_json = self
            .data_get(&key)
            .map_err(|_| RepositoryError::PackageNotFound {
                selector: selector.clone(),
            })?;
        Ok(PackageBoundary::from_pkg_json(&pkg_json, selector.clone()))
    }

    fn save_package_boundary_metadata(
        &self,
        boundary: &crate::package_types::PackageBoundary,
    ) -> Result<(), RepositoryError> {
        let key = match &boundary.selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let mut pkg_json = self.data_get(&key).unwrap_or_else(|_| {
            serde_json::json!({
                "fields": [],
                "types": [],
                "relationTypes": [],
                "views": [],
                "documentViews": [],
                "themes": []
            })
        });
        if let Some(obj) = pkg_json.as_object_mut() {
            obj.insert("id".to_string(), serde_json::json!(boundary.id));
            obj.insert(
                "namespace".to_string(),
                serde_json::json!(boundary.namespace),
            );
            obj.insert("name".to_string(), serde_json::json!(boundary.name));
            obj.insert("version".to_string(), serde_json::json!(boundary.version));
        }
        self.state.borrow_mut().data.insert(key, pkg_json);
        self.flush()
    }

    fn register_package_boundary(
        &self,
        selector: &crate::package_types::PackageSelector,
    ) -> Result<(), RepositoryError> {
        let path = match selector {
            None => return Ok(()),
            Some(p) => p.clone(),
        };
        let mut state = self.state.borrow_mut();
        let mut refs: Vec<serde_json::Value> = state
            .manifest
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
            state
                .manifest
                .extra
                .insert("packageRefs".to_string(), serde_json::Value::Array(refs));
        }
        drop(state);
        self.flush()
    }

    fn add_definition_to_boundary(
        &self,
        selector: &crate::package_types::PackageSelector,
        kind: crate::package_types::DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let key = match selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let mut pkg_json = self.data_get(&key)?;
        let array_key = crate::store::definition_kind_key(kind);
        // Auto-initialize missing array keys so older package.json files remain compatible.
        if pkg_json[array_key].is_null() {
            pkg_json[array_key] = serde_json::json!([]);
        }
        let arr =
            pkg_json[array_key]
                .as_array_mut()
                .ok_or_else(|| RepositoryError::PackageLoad {
                    path: PathBuf::from(&key),
                    source: serde_json::Error::custom(format!("{array_key} is not an array")),
                })?;
        if !arr.iter().any(|e| e.as_str() == Some(path)) {
            arr.push(serde_json::json!(path));
        }
        self.state.borrow_mut().data.insert(key, pkg_json);
        self.flush()
    }

    fn remove_definition_from_boundary(
        &self,
        selector: &crate::package_types::PackageSelector,
        kind: crate::package_types::DefinitionKind,
        path: &str,
    ) -> Result<(), RepositoryError> {
        let key = match selector {
            None => "package/package.json".to_string(),
            Some(p) => format!("{p}/package.json"),
        };
        let mut pkg_json = self.data_get(&key)?;
        let array_key = crate::store::definition_kind_key(kind);
        if let Some(arr) = pkg_json[array_key].as_array_mut() {
            arr.retain(|e| e.as_str() != Some(path));
        }
        self.state.borrow_mut().data.insert(key, pkg_json);
        self.flush()
    }

    fn resolve_definition_owner(
        &self,
        id: &str,
        kind: crate::package_types::DefinitionKind,
    ) -> Result<crate::package_types::PackageSelector, RepositoryError> {
        let array_key = crate::store::definition_kind_key(kind);
        let boundaries = self.list_package_boundaries()?;
        for boundary in &boundaries {
            let prefix = match &boundary.selector {
                None => "package".to_string(),
                Some(p) => p.clone(),
            };
            let pkg_key = format!("{prefix}/package.json");
            if let Ok(pkg_json) = self.data_get(&pkg_key) {
                if let Some(paths) = pkg_json[array_key].as_array() {
                    for entry in paths {
                        if let Some(rel) = entry.as_str() {
                            let data_key = format!("{prefix}/{rel}");
                            if let Ok(val) = self.data_get(&data_key) {
                                if val["id"].as_str() == Some(id) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_lifecycle::{create_repository, get_repository_status};
    use crate::repository_portability::{copy_repository, export_repository_snapshot};
    use crate::store::memory::MemoryStore;
    use crate::store::FileStore;
    use tempfile::TempDir;

    fn init_input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "json-repo".to_string(),
                namespace: "com.semanticops.json".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg-json".to_string(),
                namespace: "com.semanticops.json".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    use crate::repository_lifecycle::{PrimaryPackageMetadata, RepositoryMetadata};

    #[test]
    fn json_store_field_instructions_roundtrip() {
        use crate::package_service::create_field;
        use srs_core::types::field::{AiGuidance, Field, FieldType};

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let field = Field {
            schema: None,
            id: "00000000-0000-0000-0000-aabbccddee03".to_string(),
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
        drop(store);

        // Reopen from disk to prove the value survives an on-disk .srsj roundtrip,
        // not just an in-process cache.
        let reopened = JsonStore::open(&path).unwrap();
        let package = reopened.load_package().unwrap();
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
    fn json_store_create_then_open_roundtrips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        assert!(!store.repository_exists().unwrap());
        create_repository(&store, &init_input()).unwrap();
        drop(store);
        let reopened = JsonStore::open(&path).unwrap();
        assert!(reopened.repository_exists().unwrap());
        let manifest = reopened.load_manifest().unwrap();
        assert_eq!(
            manifest.extra.get("namespace").and_then(|v| v.as_str()),
            Some("com.semanticops.json")
        );
    }

    #[test]
    fn json_store_create_rejects_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        std::fs::write(&path, "{}").unwrap();
        let result = JsonStore::create(&path);
        assert!(matches!(
            result,
            Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn json_store_open_rejects_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("missing.srsj");
        let result = JsonStore::open(&path);
        assert!(matches!(result, Err(RepositoryError::Io { .. })));
    }

    #[test]
    fn json_store_open_rejects_malformed_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("bad.srsj");
        std::fs::write(&path, "{not-json").unwrap();
        let result = JsonStore::open(&path);
        assert!(matches!(result, Err(RepositoryError::Serialize { .. })));
    }

    #[test]
    fn json_store_initialize_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let second = create_repository(&store, &init_input());
        assert!(matches!(
            second,
            Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn json_store_flush_on_save_instance() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let val = serde_json::json!({"instanceId":"a","sections":[{"name":"b","content":"c"}]});
        store
            .save_instance_json("records/notes/a.json", &val)
            .unwrap();
        drop(store);
        let reopened = JsonStore::open(&path).unwrap();
        assert_eq!(
            reopened.load_instance_json("records/notes/a.json").unwrap(),
            val
        );
    }

    #[test]
    fn json_store_flush_on_delete() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let val = serde_json::json!({"k":"v"});
        store
            .save_instance_json("records/notes/a.json", &val)
            .unwrap();
        store.delete_instance_file("records/notes/a.json").unwrap();
        drop(store);
        let reopened = JsonStore::open(&path).unwrap();
        assert!(matches!(
            reopened.load_instance_json("records/notes/a.json"),
            Err(RepositoryError::Io { .. })
        ));
    }

    #[test]
    fn json_store_manifest_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("x".to_string(), serde_json::Value::String("y".to_string()));
        store.save_manifest(&manifest).unwrap();
        assert_eq!(
            store
                .load_manifest()
                .unwrap()
                .extra
                .get("x")
                .and_then(|v| v.as_str()),
            Some("y")
        );
    }

    #[test]
    fn json_store_package_json_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let package = serde_json::json!({"id":"p","namespace":"n","name":"x","version":"1","fields":[],"types":[],"relationTypes":[],"views":[],"documentViews":[]});
        store.save_package_json(&package).unwrap();
        assert_eq!(store.load_package_json().unwrap(), package);
    }

    #[test]
    fn json_store_list_instance_files_direct_children_only() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let v = serde_json::json!({"a":1});
        store
            .save_instance_json("records/notes/one.json", &v)
            .unwrap();
        store
            .save_instance_json("records/notes/deep/two.json", &v)
            .unwrap();
        let files = store.list_instance_files("records/notes").unwrap();
        assert!(files.contains(&"records/notes/one.json".to_string()));
        assert!(!files.contains(&"records/notes/deep/two.json".to_string()));
    }

    #[test]
    fn json_store_list_files_recursive_returns_all_depths() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        let v = serde_json::json!({"a":1});
        store.save_instance_json("records/a.json", &v).unwrap();
        store.save_instance_json("records/b/c.json", &v).unwrap();
        let all = store.list_files_recursive("records");
        assert!(all.contains(&"records/a.json".to_string()));
        assert!(all.contains(&"records/b/c.json".to_string()));
    }

    #[test]
    fn json_store_load_text_file_returns_string_value() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        store
            .save_instance_json(
                "docs/readme.txt",
                &serde_json::Value::String("hello".to_string()),
            )
            .unwrap();
        assert_eq!(store.load_text_file("docs/readme.txt").unwrap(), "hello");
    }

    #[test]
    fn json_store_load_text_file_returns_manifest_json() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let manifest_text = store.load_text_file("manifest.json").unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
        assert_eq!(manifest["repositoryId"], "json-repo");
    }

    #[test]
    fn json_store_copy_from_memory_store() {
        let source = MemoryStore::uninitialized();
        create_repository(&source, &init_input()).unwrap();
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let target = JsonStore::create(&path).unwrap();
        copy_repository(&source, &target).unwrap();
        let reopened = JsonStore::open(&path).unwrap();
        let snap = export_repository_snapshot(&reopened).unwrap();
        assert_eq!(snap.repository.repository_id, "json-repo");
    }

    #[test]
    fn json_store_copy_to_file_store() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let source = JsonStore::create(&path).unwrap();
        create_repository(&source, &init_input()).unwrap();
        source
            .save_instance_json(
                "records/notes/a.json",
                &serde_json::json!({"instanceId":"a","sections":[{"name":"b","content":"c"}]}),
            )
            .unwrap();

        let out = TempDir::new().unwrap();
        let target = FileStore::new(out.path());
        copy_repository(&source, &target).unwrap();
        assert!(out.path().join("manifest.json").is_file());
        assert!(out.path().join("package/package.json").is_file());
    }

    #[test]
    fn json_store_import_rejects_non_empty_target() {
        let source = MemoryStore::uninitialized();
        create_repository(&source, &init_input()).unwrap();

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let target = JsonStore::create(&path).unwrap();
        create_repository(&target, &init_input()).unwrap();
        let result = copy_repository(&source, &target);
        assert!(matches!(
            result,
            Err(RepositoryError::RepositoryNotEmpty { .. })
                | Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn json_store_repository_status_transitions() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        assert!(!get_repository_status(&store).unwrap().exists);
        create_repository(&store, &init_input()).unwrap();
        assert!(get_repository_status(&store).unwrap().exists);
    }

    // --- Package boundary tests for JsonStore ---

    #[test]
    fn json_store_package_boundaries_roundtrip() {
        use crate::package_service::{create_package, list_packages, CreatePackageInput};
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        // Primary boundary should be present after repo creation.
        let boundaries = store.list_package_boundaries().unwrap();
        assert_eq!(boundaries.len(), 1, "primary boundary should exist");
        assert!(boundaries[0].selector.is_none());

        // Create a sub-package and verify it appears.
        create_package(
            &store,
            CreatePackageInput {
                id: "json-sub-001".to_string(),
                namespace: "com.json".to_string(),
                name: "sub".to_string(),
                version: "1.0.0".to_string(),
                boundary_path: Some("pkg/sub".to_string()),
            },
        )
        .unwrap();

        let packages = list_packages(&store).unwrap();
        assert_eq!(packages.len(), 2);
        assert!(packages.iter().any(|p| p.id == "json-sub-001"));
    }

    #[test]
    fn json_store_add_remove_definition_boundary() {
        use crate::package_types::DefinitionKind;
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/foo.json")
            .unwrap();
        let b = store.load_package_boundary(&None).unwrap();
        assert!(b.field_paths.contains(&"fields/foo.json".to_string()));

        store
            .remove_definition_from_boundary(&None, DefinitionKind::Field, "fields/foo.json")
            .unwrap();
        let b2 = store.load_package_boundary(&None).unwrap();
        assert!(!b2.field_paths.contains(&"fields/foo.json".to_string()));
    }

    #[test]
    fn json_store_save_boundary_metadata_preserves_paths() {
        use crate::package_service::{update_package_metadata, UpdatePackageMetadataInput};
        use crate::package_types::DefinitionKind;
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .add_definition_to_boundary(&None, DefinitionKind::Field, "fields/keep.json")
            .unwrap();

        update_package_metadata(
            &store,
            None,
            UpdatePackageMetadataInput {
                name: Some("renamed".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let b = store.load_package_boundary(&None).unwrap();
        assert_eq!(b.name, "renamed");
        assert!(
            b.field_paths.contains(&"fields/keep.json".to_string()),
            "field_paths must survive save_package_boundary_metadata"
        );
    }

    #[test]
    fn json_store_resolve_definition_owner_returns_definition_not_found() {
        use crate::error::RepositoryError;
        use crate::package_types::DefinitionKind;
        use crate::store::RepositoryStore;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let err = store
            .resolve_definition_owner("nonexistent-id", DefinitionKind::Field)
            .unwrap_err();
        assert!(
            matches!(err, RepositoryError::DefinitionNotFound { .. }),
            "should return DefinitionNotFound, got: {err:?}"
        );
    }

    // --- Instance store tests for JsonStore (ADR-042) ---

    fn json_min_record(id: &str, type_name: &str, tags: Option<Vec<String>>) -> Record {
        Record {
            instance_id: id.to_string(),
            type_id: "type-j-0001".to_string(),
            type_version: 1,
            type_namespace: "com.example".to_string(),
            type_name: type_name.to_string(),
            field_values: vec![],
            group_values: None,
            lifecycle_state: None,
            tags,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn json_store_instance_operations_are_keyed_by_id() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .save_record(&json_min_record(
                "j-rec-0001",
                "Decision",
                Some(vec!["k".to_string()]),
            ))
            .unwrap();

        let loaded = store.load_record_by_id("j-rec-0001").unwrap();
        assert_eq!(loaded.instance_id, "j-rec-0001");
        assert_eq!(loaded.type_name, "Decision");

        let found = store.find_instance("j-rec-0001").unwrap().unwrap();
        assert_eq!(found.tier, 2);
        assert_eq!(
            store
                .list_instances(&InstanceQuery::default())
                .unwrap()
                .len(),
            1
        );

        store.delete_instance("j-rec-0001").unwrap();
        assert!(store.find_instance("j-rec-0001").unwrap().is_none());
        assert!(matches!(
            store.delete_instance("j-rec-0001"),
            Err(RepositoryError::InstanceNotFound { .. })
        ));
    }

    #[test]
    fn json_store_instance_persists_across_reopen() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        {
            let store = JsonStore::create(&path).unwrap();
            create_repository(&store, &init_input()).unwrap();
            store
                .save_record(&json_min_record("j-rec-0002", "Persisted", None))
                .unwrap();
        }
        let reopened = JsonStore::open(&path).unwrap();
        let loaded = reopened.load_record_by_id("j-rec-0002").unwrap();
        assert_eq!(loaded.type_name, "Persisted");
    }

    // --- Container store tests for JsonStore ---

    #[test]
    fn json_store_container_operations_are_keyed_by_id() {
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let container = Container {
            container_id: "json-c-001".to_string(),
            title: "My Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            tags: None,
            root_instance_ids: None,
            member_instance_ids: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        store.save_container(&container).unwrap();

        let loaded = store.load_container("json-c-001").unwrap();
        assert_eq!(loaded.container_id, "json-c-001");
        assert_eq!(loaded.title, "My Container");

        let summaries = store.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "json-c-001"));
    }

    #[test]
    fn json_store_container_persists_across_reopen() {
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        {
            let store = JsonStore::create(&path).unwrap();
            create_repository(&store, &init_input()).unwrap();
            store
                .save_container(&Container {
                    container_id: "persist-c".to_string(),
                    title: "Persisted".to_string(),
                    namespace: None,
                    name: None,
                    description: None,
                    container_type: None,
                    identity_instance_id: None,
                    tags: None,
                    root_instance_ids: None,
                    member_instance_ids: None,
                    created_at: None,
                    updated_at: None,
                    meta: None,
                    extra: std::collections::BTreeMap::new(),
                })
                .unwrap();
        }
        let reopened = JsonStore::open(&path).unwrap();
        let loaded = reopened.load_container("persist-c").unwrap();
        assert_eq!(loaded.title, "Persisted");
        let summaries = reopened.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "persist-c"));
    }

    #[test]
    fn json_store_delete_container_removes_entry() {
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .save_container(&Container {
                container_id: "delete-me".to_string(),
                title: "Delete Me".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                tags: None,
                root_instance_ids: None,
                member_instance_ids: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            })
            .unwrap();
        store.delete_container("delete-me").unwrap();

        let err = store.load_container("delete-me").unwrap_err();
        assert!(matches!(err, RepositoryError::ContainerNotFound { .. }));

        let summaries = store.list_container_summaries().unwrap();
        assert!(!summaries.iter().any(|(id, _)| id == "delete-me"));
    }

    #[test]
    fn file_store_container_adapter_preserves_existing_layout() {
        use crate::repository_lifecycle::create_repository;
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let store = FileStore::new(tmp.path());
        create_repository(&store, &init_input()).unwrap();

        let container = Container {
            container_id: "fs-c-001".to_string(),
            title: "File Store Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            tags: None,
            root_instance_ids: None,
            member_instance_ids: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        store.save_container(&container).unwrap();

        // File must exist under containers/ directory
        assert!(
            tmp.path().join("containers").is_dir(),
            "containers/ directory should exist"
        );
        let json_files: Vec<_> = std::fs::read_dir(tmp.path().join("containers"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
            .collect();
        assert_eq!(json_files.len(), 1, "one container file should exist");

        // Load it back
        let loaded = store.load_container("fs-c-001").unwrap();
        assert_eq!(loaded.title, "File Store Container");

        let summaries = store.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "fs-c-001"));
    }

    // Regression test for: JsonStore container lookup ignoring containerIndex path (#466).
    // When a .srsj is packed from FileStore, containers live at slug-id8 paths and the
    // manifest's containerIndex has a path field pointing there. load_container must resolve
    // via that path rather than always constructing containers/{uuid}.json.
    #[test]
    fn json_store_container_slug_path_resolution() {
        use crate::store::RepositoryStore;

        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "slug-test-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.slug",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"},
                "containerIndex": [
                    {
                        "containerId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                        "title": "Root",
                        "path": "containers/root-aaaabbbb.json"
                    }
                ]
            },
            "data": {
                "containers/root-aaaabbbb.json": {
                    "containerId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                    "title": "Root"
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).unwrap();

        // load_container must resolve via the indexed slug path, not containers/{uuid}.json
        let loaded = store
            .load_container("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
            .expect("load_container must succeed for slug-named container data key");
        assert_eq!(loaded.container_id, "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert_eq!(loaded.title, "Root");

        // list_container_summaries must also find the container via manifest.containerIndex
        let summaries = store.list_container_summaries().unwrap();
        assert!(
            summaries
                .iter()
                .any(|(id, _)| id == "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"),
            "list_container_summaries must include slug-keyed container"
        );
    }

    // Regression guard: JsonStore-native repos (UUID-keyed containers, no manifest.containerIndex)
    // must continue to work after the containerIndex-path fix (#466).
    #[test]
    fn json_store_container_uuid_path_still_works_after_index_fix() {
        use crate::repository_lifecycle::create_repository;
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .save_container(&Container {
                container_id: "uuid-guard-001".to_string(),
                title: "UUID Guard".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                tags: None,
                root_instance_ids: None,
                member_instance_ids: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            })
            .unwrap();

        let loaded = store.load_container("uuid-guard-001").unwrap();
        assert_eq!(loaded.title, "UUID Guard");
        let summaries = store.list_container_summaries().unwrap();
        assert!(summaries.iter().any(|(id, _)| id == "uuid-guard-001"));
    }

    // Regression test for save_container populating state.manifest.container_index (#466).
    // After save_container, load_manifest().container_index must carry the path so a reloaded
    // store can find the container via the canonical index.
    #[test]
    fn json_store_save_container_writes_manifest_index() {
        use crate::repository_lifecycle::create_repository;
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .save_container(&Container {
                container_id: "manifest-idx-c".to_string(),
                title: "Index Test".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                tags: None,
                root_instance_ids: None,
                member_instance_ids: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            })
            .unwrap();

        let manifest = store.load_manifest().unwrap();
        let index = manifest
            .container_index
            .as_deref()
            .expect("manifest.container_index must be Some after save_container");
        let entry = index
            .iter()
            .find(|e| e.container_id == "manifest-idx-c")
            .expect("index must contain the saved container");
        assert_eq!(
            entry.path.as_deref(),
            Some("containers/manifest-idx-c.json"),
            "index entry must carry the data key path"
        );
    }

    // Regression test for delete_container cleaning up state.manifest.container_index (#466).
    #[test]
    fn json_store_save_delete_manifest_index_consistency() {
        use crate::repository_lifecycle::create_repository;
        use crate::store::RepositoryStore;
        use srs_core::types::container::Container;

        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        store
            .save_container(&Container {
                container_id: "del-idx-c".to_string(),
                title: "Delete Index Test".to_string(),
                namespace: None,
                name: None,
                description: None,
                container_type: None,
                identity_instance_id: None,
                tags: None,
                root_instance_ids: None,
                member_instance_ids: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: std::collections::BTreeMap::new(),
            })
            .unwrap();
        store.delete_container("del-idx-c").unwrap();

        let manifest = store.load_manifest().unwrap();
        let has_entry = manifest
            .container_index
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|e| e.container_id == "del-idx-c");
        assert!(
            !has_entry,
            "manifest.container_index must not contain the deleted container"
        );
    }

    // Regression test for legacy-repo delete regression (arch reviewer finding 1, #466).
    // Old .srsj repos (pre-#466) stored containerIndex only in data["manifest.json"], not in
    // the top-level manifest. After delete_container, the shadow must not cause the deleted
    // container to reappear in list_container_summaries on the next load.
    #[test]
    fn json_store_legacy_shadow_delete_does_not_resurrect_container() {
        use crate::store::RepositoryStore;

        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "legacy-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.legacy",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
                // No top-level containerIndex — old format
            },
            "data": {
                "containers/aaaabbbb-cccc-dddd-eeee-ffffffffffff.json": {
                    "containerId": "aaaabbbb-cccc-dddd-eeee-ffffffffffff",
                    "title": "Legacy Container"
                },
                "manifest.json": {
                    "containerIndex": [
                        {
                            "containerId": "aaaabbbb-cccc-dddd-eeee-ffffffffffff",
                            "title": "Legacy Container"
                        }
                    ]
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).unwrap();

        // Before delete: open-time migration must make the container visible.
        let before = store.list_container_summaries().unwrap();
        assert!(
            before
                .iter()
                .any(|(id, _)| id == "aaaabbbb-cccc-dddd-eeee-ffffffffffff"),
            "legacy container must be visible before delete"
        );

        // Delete it.
        store
            .delete_container("aaaabbbb-cccc-dddd-eeee-ffffffffffff")
            .unwrap();

        // After delete: must not reappear.
        let after = store.list_container_summaries().unwrap();
        assert!(
            !after
                .iter()
                .any(|(id, _)| id == "aaaabbbb-cccc-dddd-eeee-ffffffffffff"),
            "deleted container must not reappear in list after delete"
        );
    }

    // Regression test for #490: pathless shadow containerIndex entries must get a derived
    // path during open-time migration, not be imported with path: None (which fails schema
    // validation on the next load).
    #[test]
    fn from_srsj_shadow_migration_derives_path_for_pathless_entry() {
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "test-repo-490a",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.490a",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
                // No top-level containerIndex — pre-#466 format
            },
            "data": {
                "containers/11111111-2222-3333-4444-555555555555.json": {
                    "containerId": "11111111-2222-3333-4444-555555555555",
                    "title": "Old Container"
                },
                "manifest.json": {
                    "containerIndex": [
                        {
                            "containerId": "11111111-2222-3333-4444-555555555555",
                            "title": "Old Container"
                            // No "path" field — pre-#466 shadow format
                        }
                    ]
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).unwrap();
        let manifest = store.load_manifest().unwrap();
        let index = manifest
            .container_index
            .as_deref()
            .expect("container_index must be populated after migration");
        let entry = index
            .iter()
            .find(|e| e.container_id == "11111111-2222-3333-4444-555555555555")
            .expect("migrated entry must be present in container_index");
        assert_eq!(
            entry.path,
            Some("containers/11111111-2222-3333-4444-555555555555.json".to_string()),
            "migration must derive path from data key using pre-#466 convention"
        );
    }

    // Regression test for #490: pathless shadow entries with no matching data key must be
    // silently skipped rather than imported with path: None.
    #[test]
    fn from_srsj_shadow_migration_skips_entry_with_no_matching_data_key() {
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "test-repo-490b",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.490b",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
                // No top-level containerIndex — pre-#466 format
            },
            "data": {
                "manifest.json": {
                    "containerIndex": [
                        {
                            "containerId": "aaaaaaaa-bbbb-cccc-dddd-000000000000",
                            "title": "Ghost Container"
                            // No "path" field AND no matching data key
                        }
                    ]
                }
                // No "containers/aaaaaaaa-bbbb-cccc-dddd-000000000000.json" key
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).unwrap();
        let manifest = store.load_manifest().unwrap();
        let index = manifest.container_index.as_deref().unwrap_or(&[]);
        assert!(
            !index
                .iter()
                .any(|e| e.container_id == "aaaaaaaa-bbbb-cccc-dddd-000000000000"),
            "pathless entry with no matching data key must be skipped, not imported"
        );
    }

    #[test]
    fn from_str_roundtrip() {
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "mem-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [
                    {"instanceId": "inst-001", "tier": 0, "path": "records/a.json"}
                ],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "records/a.json": {"instanceId": "inst-001", "sections": []}
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).unwrap();
        let manifest = store.load_manifest().unwrap();
        assert_eq!(manifest.instance_index.len(), 1);
        assert_eq!(manifest.instance_index[0].instance_id(), "inst-001");
    }

    #[test]
    fn from_str_bad_version() {
        let srsj = serde_json::json!({
            "srsj": "2",
            "manifest": {},
            "data": {}
        })
        .to_string();

        match JsonStore::from_srsj(&srsj) {
            Err(RepositoryError::InvalidSnapshotData { .. }) => {}
            Err(e) => panic!("expected InvalidSnapshotData, got {:?}", e),
            Ok(_) => panic!("expected error but got Ok"),
        }
    }

    #[test]
    fn open_delegates_to_from_str() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();
        drop(store);

        let content = std::fs::read_to_string(&path).unwrap();
        let via_open = JsonStore::open(&path).unwrap();
        let via_from_str = JsonStore::from_srsj(&content).unwrap();

        let manifest_open = via_open.load_manifest().unwrap();
        let manifest_str = via_from_str.load_manifest().unwrap();
        assert_eq!(
            manifest_open.instance_index.len(),
            manifest_str.instance_index.len()
        );
    }

    #[test]
    fn to_srsj_string_returns_valid_srsj_envelope() {
        // Build a minimal valid .srsj in-memory and round-trip through to_srsj_string.
        let srsj_content = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "mem-repo-b2",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.b2",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-b2",
                    "namespace": "com.test.b2",
                    "name": "primary",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj_content).unwrap();

        // to_srsj_string must succeed and produce valid JSON with srsj == "1".
        let result = store.to_srsj_string();
        assert!(result.is_ok(), "to_srsj_string returned Err: {:?}", result);

        let serialized = result.unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&serialized).expect("to_srsj_string output must be valid JSON");
        assert_eq!(
            parsed["srsj"].as_str(),
            Some("1"),
            "srsj key must equal \"1\""
        );
    }

    #[test]
    fn json_store_srsj_write_is_deterministic_and_idempotent() {
        // (1) Source `.srsj` with `data` keys in DELIBERATELY non-sorted order. A raw
        // string literal (not `serde_json::json!`, which would pre-sort via its
        // BTreeMap-backed Map) is required so the disorder actually reaches `from_srsj`.
        // After the BTreeMap change, `to_srsj_string` must emit them in sorted order,
        // identically every time, and idempotently across a read→write round-trip
        // (ADR-017, issue #171).
        let srsj_content = r#"{
            "srsj": "1",
            "manifest": {
                "repositoryId": "det-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.det",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "records/zebra.json": {"instanceId": "z"},
                "records/alpha.json": {"instanceId": "a"},
                "package/package.json": {
                    "id": "pkg-det", "namespace": "com.test.det", "name": "primary",
                    "version": "1.0.0", "fields": [], "types": [], "relationTypes": [],
                    "views": [], "documentViews": []
                },
                "records/mike.json": {"instanceId": "m"}
            }
        }"#;

        let store = JsonStore::from_srsj(srsj_content).unwrap();

        // (2) Two writes of the same store are byte-identical.
        let s1 = store.to_srsj_string().unwrap();
        let s2 = store.to_srsj_string().unwrap();
        assert_eq!(
            s1, s2,
            "two writes of the same store must be byte-identical"
        );

        // (3) write(read(x)) == write(read(write(read(x)))) — idempotent across round-trip.
        let reloaded = JsonStore::from_srsj(&s1).unwrap();
        assert_eq!(
            reloaded.to_srsj_string().unwrap(),
            s1,
            "re-serialising a reloaded store must reproduce the same bytes"
        );

        // (4) Top-level `data` keys are emitted in sorted order — non-vacuous because the
        // source literal above lists them as zebra, alpha, package, mike.
        let parsed: serde_json::Value = serde_json::from_str(&s1).unwrap();
        let keys: Vec<String> = parsed["data"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "data keys must be serialised in sorted order");
        // Guard against a future reader "tidying" the source into sorted order.
        assert_eq!(
            keys,
            vec![
                "package/package.json",
                "records/alpha.json",
                "records/mike.json",
                "records/zebra.json"
            ],
            "expected the four keys in sorted order"
        );
    }

    #[test]
    fn copy_file_to_json_preserves_vocabularies_and_lifecycles() {
        use crate::repository_lifecycle::create_repository;
        use crate::repository_portability::copy_repository;

        let src_tmp = TempDir::new().unwrap();
        let src_store = FileStore::new(src_tmp.path());
        create_repository(&src_store, &init_input()).unwrap();

        // Write a vocabulary and a lifecycle directly as JSON files into the source
        // file-store, then register them in package.json.
        let vocab_json = serde_json::json!({
            "id": "voc-test-01",
            "version": 1,
            "namespace": "com.semanticops.json",
            "name": "test-vocab",
            "mode": "open",
            "terms": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::create_dir_all(src_tmp.path().join("package/vocabularies")).unwrap();
        std::fs::write(
            src_tmp
                .path()
                .join("package/vocabularies/test-vocab-voc-test-0.json"),
            serde_json::to_string_pretty(&vocab_json).unwrap(),
        )
        .unwrap();

        let lc_json = serde_json::json!({
            "id": "lc-test-01",
            "version": 1,
            "namespace": "com.semanticops.json",
            "name": "test-lifecycle",
            "states": [
                {"id": "s1", "key": "draft", "isInitial": true},
                {"id": "s2", "key": "active", "isFinal": true}
            ],
            "transitions": [{"name": "publish", "from": "draft", "to": "active"}],
            "initialState": "draft",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        std::fs::create_dir_all(src_tmp.path().join("package/lifecycles")).unwrap();
        std::fs::write(
            src_tmp
                .path()
                .join("package/lifecycles/test-lifecycle-lc-test-0.json"),
            serde_json::to_string_pretty(&lc_json).unwrap(),
        )
        .unwrap();

        // Register both in package.json
        let pkg_path = src_tmp.path().join("package/package.json");
        let mut pkg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&pkg_path).unwrap()).unwrap();
        pkg["vocabularies"] = serde_json::json!(["vocabularies/test-vocab-voc-test-0.json"]);
        pkg["lifecycles"] = serde_json::json!(["lifecycles/test-lifecycle-lc-test-0.json"]);
        std::fs::write(&pkg_path, serde_json::to_string_pretty(&pkg).unwrap()).unwrap();

        // Copy file→json
        let dst_tmp = TempDir::new().unwrap();
        let dst_path = dst_tmp.path().join("copy.srsj");
        let dst_store = JsonStore::create(&dst_path).unwrap();
        copy_repository(&src_store, &dst_store).unwrap();
        drop(dst_store);

        // Reopen the .srsj and verify vocabularies and lifecycles survive the round-trip
        let reopened = JsonStore::open(&dst_path).unwrap();
        let pkg = reopened.load_package().unwrap();
        assert_eq!(
            pkg.vocabularies.len(),
            1,
            "expected 1 vocabulary in srsj, got {}",
            pkg.vocabularies.len()
        );
        assert_eq!(pkg.vocabularies[0].name, "test-vocab");
        assert_eq!(
            pkg.lifecycles.len(),
            1,
            "expected 1 lifecycle in srsj, got {}",
            pkg.lifecycles.len()
        );
        assert_eq!(pkg.lifecycles[0].name, "test-lifecycle");
    }

    // --- Batch write mode tests (ADR-021) ---

    #[test]
    fn json_store_batch_mode_suppresses_intermediate_flushes() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let val = serde_json::json!({"instanceId":"a","sections":[{"name":"b","content":"c"}]});
        store.begin_batch();
        store
            .save_instance_json("records/notes/a.json", &val)
            .unwrap();

        // File on disk must NOT yet contain the record while batch mode is active.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("records/notes/a.json"),
            "file was written to disk during batch mode before commit_batch()"
        );

        // After commit_batch the file must contain the record.
        store.commit_batch().unwrap();
        let reopened = JsonStore::open(&path).unwrap();
        assert_eq!(
            reopened.load_instance_json("records/notes/a.json").unwrap(),
            val
        );
    }

    #[test]
    fn json_store_abort_batch_leaves_file_unchanged() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let baseline = std::fs::read_to_string(&path).unwrap();

        store.begin_batch();
        let val = serde_json::json!({"instanceId":"b","sections":[{"name":"x","content":"y"}]});
        store
            .save_instance_json("records/notes/b.json", &val)
            .unwrap();
        store.abort_batch();

        // On-disk file must match the pre-import baseline.
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after, baseline,
            "abort_batch must not write partial data to disk"
        );

        // In-memory state must also be restored (record absent from same instance).
        assert!(
            matches!(
                store.load_instance_json("records/notes/b.json"),
                Err(RepositoryError::Io { .. })
            ),
            "abort_batch must restore in-memory state from disk"
        );
    }

    #[test]
    fn json_store_commit_batch_writes_all_accumulated_data() {
        let tmp = TempDir::new().unwrap();
        let srsj_path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&srsj_path).unwrap();
        create_repository(&store, &init_input()).unwrap();

        let records = [
            (
                "records/notes/r1.json",
                serde_json::json!({"instanceId":"r1"}),
            ),
            (
                "records/notes/r2.json",
                serde_json::json!({"instanceId":"r2"}),
            ),
            (
                "records/notes/r3.json",
                serde_json::json!({"instanceId":"r3"}),
            ),
        ];

        store.begin_batch();
        for (record_key, val) in &records {
            store.save_instance_json(record_key, val).unwrap();
        }
        store.commit_batch().unwrap();

        let reopened = JsonStore::open(&srsj_path).unwrap();
        for (record_key, val) in &records {
            assert_eq!(
                reopened.load_instance_json(record_key).unwrap(),
                *val,
                "record {record_key} not found after commit_batch"
            );
        }
    }

    #[test]
    fn json_store_load_package_includes_blueprints() {
        // Regression test for #368: JsonStore::load_package() was hardcoding
        // blueprints: vec![] even when the .srsj package contained blueprint entries.
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/governance-seed.srsj");
        let store = JsonStore::open(&fixture).expect("governance-seed.srsj must exist");
        let package = store.load_package().expect("load_package must succeed");
        assert!(
            !package.blueprints.is_empty(),
            "governance-seed.srsj contains at least one blueprint; load_package must not drop it"
        );
    }

    #[test]
    fn from_srsj_does_not_overwrite_existing_index_tags() {
        // If the index entry already has tags set, from_srsj must not overwrite them
        // even if the bundled record file has different tags.
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "instanceIndex": [
                    {
                        "instanceId": "00000000-0000-4000-8000-000000000001",
                        "tier": 2,
                        "path": "records/tier-2/test-record.json",
                        "tags": ["existing"]
                    }
                ]
            },
            "data": {
                "records/tier-2/test-record.json": {
                    "instanceId": "00000000-0000-4000-8000-000000000001",
                    "typeId": "00000000-0000-4000-8000-000000000002",
                    "typeVersion": 1,
                    "typeNamespace": "com.example.test",
                    "typeName": "test-type",
                    "fieldValues": [],
                    "tags": ["different"]
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).expect("must parse");
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.instance_index[0].tags,
            Some(vec!["existing".to_string()]),
            "from_srsj must not overwrite an index entry that already has tags"
        );
    }

    #[test]
    fn loaded_blueprint_source_package_json_store() {
        // Verify that JsonStore::load_package sets source_package correctly for
        // blueprints merged from a sub-package registered via packageRefs.
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "bp-prov-json-test",
                "srsVersion": "2.0-draft",
                "namespace": "com.test",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"},
                "packageRefs": [{"mode": "local", "path": "extensions/subpkg"}]
            },
            "data": {
                "package/package.json": {
                    "id": "primary-pkg",
                    "namespace": "com.test",
                    "name": "primary",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                    "views": [],
                    "documentViews": [],
                    "blueprints": ["blueprints/root-bp.json"]
                },
                "package/blueprints/root-bp.json": {
                    "id": "root-bp-001",
                    "namespace": "com.test",
                    "name": "Root Blueprint",
                    "version": 1,
                    "description": "Root package blueprint",
                    "rootTypes": [],
                    "createdAt": "2026-01-01T00:00:00Z"
                },
                "extensions/subpkg/package.json": {
                    "id": "sub-pkg-001",
                    "namespace": "com.test.ext",
                    "name": "subpkg",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                    "views": [],
                    "documentViews": [],
                    "blueprints": ["blueprints/sub-bp.json"]
                },
                "extensions/subpkg/blueprints/sub-bp.json": {
                    "id": "sub-bp-002",
                    "namespace": "com.test.ext",
                    "name": "Sub Blueprint",
                    "version": 1,
                    "description": "Sub-package blueprint",
                    "rootTypes": [],
                    "createdAt": "2026-01-01T00:00:00Z"
                }
            }
        })
        .to_string();

        let store = JsonStore::from_srsj(&srsj).expect("from_srsj must succeed");
        let package = store.load_package().expect("load_package must succeed");
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
    fn json_store_record_tier_dir_matches_canonical_values() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("repo.srsj");
        let store = JsonStore::create(&path).unwrap();
        assert_eq!(store.record_tier_dir(RecordTier::Note), "records/notes");
        assert_eq!(store.record_tier_dir(RecordTier::Tier1), "records/tier-1");
        assert_eq!(store.record_tier_dir(RecordTier::Tier2), "records/tier-2");
        assert_eq!(
            store.record_tier_dir(RecordTier::Extension),
            "package/records"
        );
    }

    // ── JsonStore binary-file storage (ADR-031 amendment) ───────────────────────

    fn binary_test_srsj() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "bin-test-repo",
                "srsVersion": "2.0-draft",
                "namespace": "com.test.bin",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"}
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-bin",
                    "namespace": "com.test.bin",
                    "name": "primary",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                    "relationTypes": [],
                    "views": [],
                    "documentViews": []
                }
            }
        })
        .to_string()
    }

    fn init_memory_store_for_archive() -> MemoryStore {
        let store = MemoryStore::uninitialized();
        store
            .initialize_repository(&InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "arc-test-repo".to_string(),
                    namespace: "com.example.arctest".to_string(),
                    srs_version: "2.0-draft".to_string(),
                    title: None,
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "arc-pkg".to_string(),
                    namespace: "com.example.arctest".to_string(),
                    name: "primary".to_string(),
                    version: "1.0.0".to_string(),
                },
            })
            .expect("initialize_repository failed");
        store
    }

    #[test]
    fn json_store_binary_file_save_and_load() {
        let store = JsonStore::from_srsj(&binary_test_srsj()).expect("load store");
        let bytes = b"\x00\x01\x02\xffpdf-content";
        store
            .save_binary_file("source-documents/doc.pdf", bytes)
            .expect("save_binary_file must succeed");
        let loaded = store
            .load_binary_file("source-documents/doc.pdf")
            .expect("load_binary_file must return saved bytes");
        assert_eq!(loaded, bytes);
    }

    #[test]
    fn json_store_binary_file_load_absent() {
        let store = JsonStore::from_srsj(&binary_test_srsj()).expect("load store");
        let err = store
            .load_binary_file("source-documents/absent.pdf")
            .expect_err("absent path must return an error");
        assert!(
            err.is_not_found(),
            "absent binary file must be a not-found error, got: {err:?}"
        );
    }

    #[test]
    fn json_store_srsj_excludes_binary() {
        let store = JsonStore::from_srsj(&binary_test_srsj()).expect("load store");
        store
            .save_binary_file("source-documents/secret.pdf", b"binary bytes")
            .expect("save must succeed");
        let srsj = store.to_srsj_string().expect("to_srsj_string must succeed");
        assert!(
            !srsj.contains("source-documents/secret.pdf"),
            "binary file path must not appear in .srsj output"
        );
        assert!(
            !srsj.contains("binary bytes"),
            "binary content must not appear in .srsj output"
        );
    }

    #[test]
    fn json_store_from_archive_binary_available() {
        use crate::archive::archive_pack;
        use srs_core::types::source_document::SourceDocumentIndexEntry;
        use std::io::Cursor;

        // Build a MemoryStore with a binary attachment.
        let source = init_memory_store_for_archive();
        const BYTES: &[u8] = b"\xde\xad\xbe\xef archive binary";
        source
            .save_binary_file("source-documents/my-doc.pdf", BYTES)
            .expect("save binary to source");
        let mut manifest = source.load_manifest().expect("load manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![SourceDocumentIndexEntry {
            document_id: "doc-0001".to_string(),
            sidecar_path: "my-doc.meta.json".to_string(),
            content_path: "my-doc.pdf".to_string(),
            title: None,
            sidecar_checksum: None,
            content_checksum: None,
        }]);
        source.save_manifest(&manifest).expect("save manifest");
        source
            .save_text_file(
                "source-documents/my-doc.meta.json",
                r#"{"documentId":"doc-0001","contentPath":"my-doc.pdf"}"#,
            )
            .expect("save sidecar");

        // Pack to archive bytes.
        let mut buf = Vec::new();
        archive_pack(&source, Cursor::new(&mut buf)).expect("pack archive");

        // Load archive into JsonStore and verify binary is accessible.
        let json_store = JsonStore::from_archive(&buf).expect("from_archive");
        let loaded = json_store
            .load_binary_file("source-documents/my-doc.pdf")
            .expect("binary must be available after from_archive");
        assert_eq!(loaded, BYTES);
    }

    #[test]
    fn json_store_from_archive_export_archive_roundtrip() {
        use crate::archive::{archive_pack, archive_to_vec};
        use srs_core::types::source_document::SourceDocumentIndexEntry;
        use std::io::Cursor;

        // Build source MemoryStore with binary content.
        let source = init_memory_store_for_archive();
        const BYTES: &[u8] = b"\xca\xfe\xba\xbe roundtrip content";
        source
            .save_binary_file("source-documents/rt.pdf", BYTES)
            .expect("save binary");
        let mut manifest = source.load_manifest().expect("load manifest");
        manifest.source_documents_path = Some("source-documents".to_string());
        manifest.source_document_index = Some(vec![SourceDocumentIndexEntry {
            document_id: "doc-rt".to_string(),
            sidecar_path: "rt.meta.json".to_string(),
            content_path: "rt.pdf".to_string(),
            title: None,
            sidecar_checksum: None,
            content_checksum: None,
        }]);
        source.save_manifest(&manifest).expect("save manifest");
        source
            .save_text_file(
                "source-documents/rt.meta.json",
                r#"{"documentId":"doc-rt","contentPath":"rt.pdf"}"#,
            )
            .expect("save sidecar");

        let mut first_buf = Vec::new();
        archive_pack(&source, Cursor::new(&mut first_buf)).expect("first pack");

        // Load into JsonStore, re-export, and verify bytes survive the second pack.
        let json_store = JsonStore::from_archive(&first_buf).expect("from_archive");
        let second_buf = archive_to_vec(&json_store).expect("re-export archive");
        let json_store2 = JsonStore::from_archive(&second_buf).expect("from second archive");
        let loaded = json_store2
            .load_binary_file("source-documents/rt.pdf")
            .expect("binary must survive re-export roundtrip");
        assert_eq!(loaded, BYTES);
    }

    #[test]
    fn json_store_type_extras_survive_load() {
        // Regression guard for the shared TypeJson fix (#684): $schema and
        // aiGuidance on a type definition must reach RecordType.extra when the
        // package loads from a .srsj envelope (previously dropped).
        let srsj = serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "extras-repo",
                "namespace": "com.test.extras",
                "srsVersion": "2.0-draft",
                "instanceIndex": []
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-extras",
                    "namespace": "com.test.extras",
                    "name": "extras",
                    "version": "1.0.0",
                    "fields": [],
                    "types": ["types/thing.json"]
                },
                "package/types/thing.json": {
                    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                    "id": "type-extras-1",
                    "namespace": "com.test.extras",
                    "name": "thing",
                    "version": 1,
                    "aiGuidance": "guidance survives",
                    "fields": []
                }
            }
        })
        .to_string();
        let store = JsonStore::from_srsj(&srsj).expect("from_srsj");
        let package = store.load_package().expect("load_package");
        let rt = package
            .record_types
            .iter()
            .find(|t| t.id == "type-extras-1")
            .expect("type loaded");
        assert_eq!(
            rt.extra.get("aiGuidance").and_then(|v| v.as_str()),
            Some("guidance survives")
        );
        assert_eq!(
            rt.extra.get("$schema").and_then(|v| v.as_str()),
            Some("https://srs.semanticops.com/schema/2.0/type.json")
        );
    }
}
