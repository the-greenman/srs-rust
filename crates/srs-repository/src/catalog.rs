//! `RepositoryCatalog` — the RFC-038 enumeration contract (srs-rust#783, Phase 1).
//!
//! One materialised snapshot of the six authoritative sets ([R1], Change L),
//! produced by classifying every candidate file under the reserved repository
//! locations ([R3]–[R10]) and carrying diagnostics with the result ([R23],
//! [R24]). Membership comes from the tree, never from `manifest.instanceIndex`
//! — the manifest is read only for configuration (`sourceDocumentsPath`,
//! extension paths, `declaredExtensions`) and for the inline root container,
//! for which it *is* authoritative ([R1]).
//!
//! Phase-1 transitional notes (removed at the Phase-6 enforcement flip):
//! - `relations/relations.json` / `relations-collection.json` collection files
//!   are still enumerated (the vendored fixtures are rev-2-with-collection;
//!   the [R11] collection deny activates at the flip).
//! - The [R2] retired-manifest-property deny and the [R21] generation gate are
//!   Phase 3/6 concerns and are not implemented here.
//! - [R7]'s `allOf` domain-composition branch (a domain schema composing a
//!   core entity schema via `allOf`) is not yet resolved: an unknown declared
//!   `$schema` URL is a `SCHEMA-UNRESOLVABLE` error today. No domain-composed
//!   schema exists in any fixture; resolution is owed to Phase 3 alongside
//!   the first service consumption of the catalog.
//! - The legacy `source-document.json` sidecar `$schema` is accepted as a
//!   transitional shim for a filed corpus defect (the-greenman/srs#369).

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::validation::DiagnosticSeverity;
use serde_json::Value;
use sha2::{Digest, Sha256};
use srs_core::types::relation::{Relation, RelationsCollection};
use srs_schema::SchemaRegistry;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Kinds and entries
// ---------------------------------------------------------------------------

/// The closed kind list of RFC-038 Change L.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogKind {
    Note,
    Record,
    Relation,
    Container,
    SourceDocument,
    Field,
    Type,
    View,
    Composition,
    Theme,
    RelationType,
    Vocabulary,
    Lifecycle,
    Blueprint,
    Protocol,
    Changelog,
}

impl CatalogKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CatalogKind::Note => "note",
            CatalogKind::Record => "record",
            CatalogKind::Relation => "relation",
            CatalogKind::Container => "container",
            CatalogKind::SourceDocument => "source-document",
            CatalogKind::Field => "field",
            CatalogKind::Type => "type",
            CatalogKind::View => "view",
            CatalogKind::Composition => "composition",
            CatalogKind::Theme => "theme",
            CatalogKind::RelationType => "relation-type",
            CatalogKind::Vocabulary => "vocabulary",
            CatalogKind::Lifecycle => "lifecycle",
            CatalogKind::Blueprint => "blueprint",
            CatalogKind::Protocol => "protocol",
            CatalogKind::Changelog => "changelog",
        }
    }
}

/// One enumerated object: `{id, kind, tier?, locator?}` ([R23]).
///
/// `tier` is present only for instances (0 Note, 2 Record — Tier 1/TypedRecord
/// was retired, srs#448/rfc-decision-53635966, srs-rust#888). `locator` is adapter-private,
/// for diagnostics and portable-tree projection only — it is never semantic
/// identity and is excluded from the validity token.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    pub id: String,
    pub kind: CatalogKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
}

/// A diagnostic with a stable identifier and declared severity ([R24]).
///
/// `locators` names every involved locator — a duplicate-id diagnostic names
/// all conflicting files, not just the second found ([R12]).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDiagnostic {
    pub code: &'static str,
    pub severity: DiagnosticSeverity,
    pub locators: Vec<String>,
    pub message: String,
}

/// Stable diagnostic identifiers ([R24]).
pub mod codes {
    /// `manifest.json` missing or unparseable ([R2]: it remains required).
    pub const MANIFEST_INVALID: &str = "SRS038-MANIFEST-INVALID";
    /// A `package.json` that is SRS-shaped but fails `package-manifest.json` ([R4]).
    pub const PACKAGE_MANIFEST_INVALID: &str = "SRS038-R4-PACKAGE-MANIFEST-INVALID";
    /// A `package.json` that is not valid JSON, so SRS-ness cannot be determined.
    /// Warning: an npm manifest with a syntax error must not fail the load.
    pub const PACKAGE_JSON_UNPARSEABLE: &str = "SRS038-R4-PACKAGE-JSON-UNPARSEABLE";
    /// Declared `$schema` does not resolve to a known entity schema ([R7]).
    pub const SCHEMA_UNRESOLVABLE: &str = "SRS038-R7-SCHEMA-UNRESOLVABLE";
    /// Declared `$schema` resolves to an entity not admissible at this location ([R7]).
    pub const SCHEMA_INADMISSIBLE: &str = "SRS038-R7-SCHEMA-INADMISSIBLE";
    /// Object fails validation against its declared `$schema` ([R7]).
    pub const SCHEMA_VALIDATION: &str = "SRS038-R7-SCHEMA-VALIDATION";
    /// No `$schema`, and the object validates as none of the admissible entities ([R8]).
    pub const SHAPE_NO_MATCH: &str = "SRS038-R8-SHAPE-NO-MATCH";
    /// No `$schema`, and the object validates as more than one admissible entity ([R8]).
    pub const SHAPE_AMBIGUOUS: &str = "SRS038-R8-SHAPE-AMBIGUOUS";
    /// The pairwise-discriminator standing check over the instance schemas failed ([R8]).
    pub const SHAPE_DISCRIMINATOR_BROKEN: &str = "SRS038-R8-DISCRIMINATOR-BROKEN";
    /// Malformed JSON in a candidate file under a reserved location ([R9]).
    pub const CANDIDATE_MALFORMED: &str = "SRS038-R9-CANDIDATE-MALFORMED";
    /// A non-candidate file under a reserved location's closed candidate policy ([R9]).
    pub const CANDIDATE_UNRECOGNISED: &str = "SRS038-R9-CANDIDATE-UNRECOGNISED";
    /// Relation filename disagrees with the in-file `relationId` ([R11]).
    pub const RELATION_FILENAME_MISMATCH: &str = "SRS038-R11-FILENAME-MISMATCH";
    /// A file nested in a subdirectory of `relations/`, which must be flat ([R11]).
    pub const RELATIONS_NOT_FLAT: &str = "SRS038-R11-NOT-FLAT";

    /// [R11]: at `dataModelRevision >= 2` a live repository must not contain a
    /// relations-collection file.
    pub const RELATIONS_COLLECTION_RETIRED: &str = "SRS038-R11-COLLECTION-RETIRED";
    /// Two objects in the same authoritative set declaring the same logical id ([R12]).
    pub const DUPLICATE_ID: &str = "SRS038-R12-DUPLICATE-ID";
    /// A reference resolving to nothing in the set it targets ([R13]).
    pub const DANGLING_REFERENCE: &str = "SRS038-R13-DANGLING-REFERENCE";
    /// A source-document sidecar with no parseable `documentId` ([R15]).
    pub const SIDECAR_NO_DOCUMENT_ID: &str = "SRS038-R15-SIDECAR-NO-DOCUMENT-ID";
    /// A path declared in a package manifest's definition arrays that resolves to no file.
    pub const DEFINITION_PATH_MISSING: &str = "SRS038-DEFINITION-PATH-MISSING";
    /// An extension aggregate whose `{kind, id}` identity cannot be projected (Change L).
    pub const EXTENSION_IDENTITY: &str = "SRS038-EXT-IDENTITY";
}

// ---------------------------------------------------------------------------
// RepositoryCatalog
// ---------------------------------------------------------------------------

/// Locator of the inline root container ([R1]: the manifest is authoritative
/// for the repository's root container — it is never a file under `containers/`).
pub const ROOT_CONTAINER_LOCATOR: &str = "manifest.json#/container";

/// One materialised snapshot of the six authoritative sets, with diagnostics.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCatalog {
    pub instances: Vec<CatalogEntry>,
    pub relations: Vec<CatalogEntry>,
    pub containers: Vec<CatalogEntry>,
    pub source_documents: Vec<CatalogEntry>,
    pub definitions: Vec<CatalogEntry>,
    pub extensions: Vec<CatalogEntry>,
    pub diagnostics: Vec<CatalogDiagnostic>,
    /// Every location discovered to be a local SRS package root, whether or
    /// not a `PackageRef` names it — the empty string is the repository root.
    ///
    /// Not a seventh authoritative set. Classification anchors on the subset
    /// whose manifest *validates* ([R3]/[R5]); this list also carries the
    /// near-misses [R4] diagnoses, because snapshot production needs to know
    /// a package is *there* ([R17]) even when it cannot be read as one —
    /// packing only the valid subset would drop a whole sub-package silently.
    /// A plain npm `package.json` is in neither: it is not an SRS package.
    pub package_roots: Vec<String>,
}

impl RepositoryCatalog {
    /// True when any diagnostic is an `error` — fatal to the load under [R24].
    pub fn has_fatal(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
    }

    /// [R16] validity token: a content digest over the enumerated id set
    /// (set → kind → id, locators excluded — path-free identity).
    pub fn validity_token(&self) -> String {
        let mut hasher = Sha256::new();
        for (set, entries) in [
            ("instances", &self.instances),
            ("relations", &self.relations),
            ("containers", &self.containers),
            ("source-documents", &self.source_documents),
            ("definitions", &self.definitions),
            ("extensions", &self.extensions),
        ] {
            for e in entries {
                hasher.update(set.as_bytes());
                hasher.update([0]);
                hasher.update(e.kind.as_str().as_bytes());
                hasher.update([0]);
                hasher.update(e.id.as_bytes());
                hasher.update([0]);
            }
        }
        hasher
            .finalize()
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                use std::fmt::Write;
                let _ = write!(s, "{b:02x}");
                s
            })
    }
}

/// Build the catalog and apply [R24] fatality: an `error` diagnostic under a
/// reserved location fails the load as a whole — no partial catalog is
/// reported as complete. The full diagnostic list travels in the error.
pub fn build_checked(store: &dyn RepositoryStore) -> Result<RepositoryCatalog, RepositoryError> {
    let catalog = build(store)?;
    if catalog.has_fatal() {
        let fatal = catalog
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count();
        let first = catalog
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error)
            .map(|d| format!("{}: {}", d.code, d.message))
            .unwrap_or_default();
        return Err(RepositoryError::CatalogLoad {
            fatal,
            first,
            diagnostics: catalog.diagnostics,
        });
    }
    Ok(catalog)
}

// ---------------------------------------------------------------------------
// The walker
// ---------------------------------------------------------------------------

/// Directory segments never entered by discovery ([R3], [R5]).
const SKIPPED_SEGMENTS: &[&str] = &[".git", "node_modules", ".srs"];

/// Directory segments that belong to another tool, never to the repository.
///
/// The discovery list minus `.srs`: snapshot production *does* carry the marker
/// directory ([R17]) even though classification never enters it.
pub(crate) fn is_foreign_tooling(path: &str) -> bool {
    path.split('/')
        .any(|segment| SKIPPED_SEGMENTS.contains(&segment) && segment != crate::vfs::SRS_MARKER_DIR)
}

/// Resolve a manifest-declared location to a repo-relative subpath.
///
/// The single rule both classification and snapshot production use, so they
/// cannot disagree about where a declared location is. Surrounding slashes are
/// tolerated (`/source-documents` is the same directory); anything that does
/// not resolve to a real subpath — absent, empty, `.`, or an escape — yields
/// `None`, because treating it as the repository root would make every walk
/// over it a blind directory sweep.
pub(crate) fn declared_location(value: Option<&str>) -> Option<String> {
    crate::vfs::normalize_relative(value?.trim_matches('/')).filter(|p| !p.is_empty())
}

/// The reserved instance root directory names ([R3]).
///
/// `typed-records` stays reserved even though Tier 1 (TypedRecord) is retired
/// (srs#448/rfc-decision-53635966, srs-rust#888): dropping it here would make
/// leftover Tier-1 content under a top-level `typed-records/` directory
/// invisible to classification instead of loudly [R8] `SHAPE_NO_MATCH`-fatal.
/// The clean cut is "no longer admissible," never "silently unenumerated."
pub(crate) const INSTANCE_ROOT_NAMES: &[&str] = &["records", "notes", "typed-records"];

/// The recognised instance-sidecar suffix list ([R9]) is now empty: the sole
/// entry, `.revisions.json`, was retired with the mechanism it backed
/// (rfc-decision-2a1e1590, srs-rust#866) — a `.revisions.json` file under a
/// reserved instance root is no longer conferred any recognition by filename;
/// it falls through to ordinary instance-candidate classification like any
/// other object under [R8]/[R9], where its shape does not match and it errors.
///
/// TRANSITIONAL shim masking a filed corpus defect (the-greenman/srs#369):
/// every live source-document sidecar declares this `$schema` URL, but no
/// `source-document.json` schema exists anywhere — the canonical sidecar
/// schema is `source-document-meta.json`. Without the shim, [R7]'s
/// unresolvable-`$schema` rule would fatally reject every first-party
/// repository. Remove when the corpus repairs its sidecar declarations.
const LEGACY_SOURCE_DOCUMENT_SCHEMA_ID: &str =
    "https://srs.semanticops.com/schema/2.0/source-document.json";

/// Definition arrays of `package-manifest.json`, the entity kind each declares,
/// and the schema it validates against. Every declared kind has a schema as of
/// the #297 cutover — no definition kind is left unchecked.
const DEFINITION_ARRAYS: &[(&str, CatalogKind, &str)] = &[
    ("fields", CatalogKind::Field, srs_schema::FIELD_SCHEMA_ID),
    ("types", CatalogKind::Type, srs_schema::TYPE_SCHEMA_ID),
    ("views", CatalogKind::View, srs_schema::VIEW_SCHEMA_ID),
    (
        "compositions",
        CatalogKind::Composition,
        srs_schema::COMPOSITION_SCHEMA_ID,
    ),
    ("themes", CatalogKind::Theme, srs_schema::THEME_SCHEMA_ID),
    (
        "relationTypes",
        CatalogKind::RelationType,
        srs_schema::RELATION_TYPE_SCHEMA_ID,
    ),
    (
        "vocabularies",
        CatalogKind::Vocabulary,
        srs_schema::VOCABULARY_SCHEMA_ID,
    ),
    (
        "lifecycles",
        CatalogKind::Lifecycle,
        srs_schema::LIFECYCLE_SCHEMA_ID,
    ),
    (
        "blueprints",
        CatalogKind::Blueprint,
        srs_schema::BLUEPRINT_SCHEMA_ID,
    ),
    (
        "protocols",
        CatalogKind::Protocol,
        srs_schema::PROTOCOL_SCHEMA_ID,
    ),
];

struct Builder<'a> {
    store: &'a dyn RepositoryStore,
    entries: Sets,
    diagnostics: Vec<CatalogDiagnostic>,
    /// (referring locator, fieldIds) collected from Type definitions, for [R13].
    type_field_refs: Vec<(String, Vec<String>)>,
    /// (relation id, source, target, locator) for [R13].
    relation_endpoints: Vec<(String, String, String, String)>,
    /// (container locator, referenced instance ids with property name) for [R13].
    container_refs: Vec<(String, &'static str, Vec<String>)>,
    /// (id, version, locator) for definition duplicate detection: versioned
    /// lineages legally share a UUID across versions, so the version joins
    /// the [R12] identity key for definitions.
    definition_dup_keys: Vec<(String, Option<u64>, String)>,
}

#[derive(Default)]
struct Sets {
    instances: Vec<CatalogEntry>,
    relations: Vec<CatalogEntry>,
    containers: Vec<CatalogEntry>,
    source_documents: Vec<CatalogEntry>,
    definitions: Vec<CatalogEntry>,
    extensions: Vec<CatalogEntry>,
}

/// Build the catalog with all diagnostics carried in the result (no [R24]
/// fatality applied — `validate`-style consumers need the complete picture).
/// `Err` is reserved for infrastructure failures (I/O other than not-found).
pub fn build(store: &dyn RepositoryStore) -> Result<RepositoryCatalog, RepositoryError> {
    let mut b = Builder {
        store,
        entries: Sets::default(),
        diagnostics: Vec::new(),
        type_field_refs: Vec::new(),
        relation_endpoints: Vec::new(),
        container_refs: Vec::new(),
        definition_dup_keys: Vec::new(),
    };

    // [R8] standing check: the instance-schema discriminators must hold before
    // shape classification is trusted.
    if let Some(err) = instance_discriminator_error() {
        b.error(
            codes::SHAPE_DISCRIMINATOR_BROKEN,
            vec!["schema:instance-candidate-set".to_string()],
            err.clone(),
        );
    }

    // The manifest remains required ([R2]); it is read for configuration only.
    let manifest = match store.load_manifest() {
        Ok(m) => m,
        Err(e) => {
            b.error(
                codes::MANIFEST_INVALID,
                vec!["manifest.json".to_string()],
                format!("manifest.json missing or unparseable: {e}"),
            );
            return Ok(b.finish(Vec::new()));
        }
    };
    let manifest_value =
        serde_json::to_value(&manifest).map_err(|source| RepositoryError::Serialize {
            path: std::path::PathBuf::from("manifest.json"),
            source,
        })?;

    let sd_path = declared_location(manifest.source_documents_path.as_deref())
        .unwrap_or_else(|| "source-documents".to_string());

    let declared_extensions: BTreeSet<String> = manifest_value
        .get("declaredExtensions")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let changelog_path = manifest_value
        .get("changelogPath")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let repository_id = manifest_value
        .get("repositoryId")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    // Enumerate the tree once, deterministically ([R14]: never expose
    // filesystem iteration order).
    let mut files: Vec<String> = store
        .list_files_recursive("")
        .into_iter()
        .filter(|p| !p.split('/').any(|seg| SKIPPED_SEGMENTS.contains(&seg)))
        .collect();
    files.sort();
    let file_set: BTreeSet<String> = files.iter().cloned().collect();

    // Extension-set locations are handled out of the main dispatch.
    let extension_paths: BTreeSet<String> =
        [changelog_path.clone()].into_iter().flatten().collect();

    // --- Anchor discovery (Changes B/D; [R3]/[R4]) ---

    let root_instance_roots: BTreeSet<String> = INSTANCE_ROOT_NAMES
        .iter()
        .filter(|d| dir_exists(&file_set, d))
        .map(|d| d.to_string())
        .collect();

    // Candidate package.json paths, shallow-first so a parent package root is
    // known before a nested candidate is considered.
    let mut pkg_candidates: Vec<&String> = files
        .iter()
        .filter(|p| {
            (p.as_str() == "package.json" || p.ends_with("/package.json"))
                && !under_any(p, &root_instance_roots)
                && !under(p, &sd_path)
                && !under(p, "relations")
                && !under(p, "containers")
        })
        .collect();
    pkg_candidates.sort_by_key(|p| (p.split('/').count(), p.as_str().to_string()));

    let mut package_roots: Vec<String> = Vec::new();
    let mut instance_roots: BTreeSet<String> = root_instance_roots.clone();
    // Declared definition path → (kind, schema id, owning package root).
    let mut declared_defs: BTreeMap<String, (CatalogKind, &'static str)> = BTreeMap::new();

    for pkg_path in pkg_candidates {
        // A package root nested inside a reserved instance root must not
        // anchor further roots ([R3]) — its package.json falls under the
        // instance-root candidate policy instead.
        if under_any(pkg_path, &instance_roots) {
            continue;
        }
        let root = pkg_path
            .strip_suffix("package.json")
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string();
        let value = match store.load_instance_json(pkg_path) {
            Ok(v) => v,
            Err(e) if e.is_not_found() => continue,
            Err(_) => {
                // Unparseable: SRS-ness cannot be determined, and an npm
                // manifest with a syntax error must not fail the load.
                b.warn(
                    codes::PACKAGE_JSON_UNPARSEABLE,
                    vec![pkg_path.clone()],
                    "package.json is not valid JSON; cannot determine whether it is an SRS package manifest".to_string(),
                );
                continue;
            }
        };
        let declares_srs_schema = value.get("$schema").and_then(|v| v.as_str())
            == Some(srs_schema::PACKAGE_MANIFEST_SCHEMA_ID);
        let srs_shaped = value.get("namespace").is_some()
            && (value.get("fields").is_some() || value.get("types").is_some());
        match SchemaRegistry::global()
            .validate_by_id(srs_schema::PACKAGE_MANIFEST_SCHEMA_ID, &value)
        {
            Ok(()) => {
                // Conforming SRS package manifest: a presence-keyed anchor.
                for name in INSTANCE_ROOT_NAMES {
                    let candidate = join(&root, name);
                    if dir_exists(&file_set, &candidate) {
                        instance_roots.insert(candidate);
                    }
                }
                for (array_key, kind, schema) in DEFINITION_ARRAYS {
                    if let Some(paths) = value.get(*array_key).and_then(|v| v.as_array()) {
                        for p in paths.iter().filter_map(|v| v.as_str()) {
                            declared_defs.insert(join(&root, p), (*kind, *schema));
                        }
                    }
                }
                package_roots.push(root);
            }
            Err(e) => {
                if declares_srs_schema || srs_shaped {
                    // [R4]: a near-miss package manifest is diagnosed, not
                    // silently skipped — it does not anchor classification, but
                    // it is still a package root that exists, and a snapshot
                    // must carry it ([R17]).
                    b.error(
                        codes::PACKAGE_MANIFEST_INVALID,
                        vec![pkg_path.clone()],
                        format!("package.json fails package-manifest.json: {e}"),
                    );
                    package_roots.push(root);
                }
                // else: an npm/application package.json — not an anchor, not
                // an error ([R4]).
            }
        }
    }

    // --- Classification dispatch (Changes C/D; [R5] most-specific wins) ---

    for path in &files {
        if path == "manifest.json" || extension_paths.contains(path) {
            continue;
        }
        if under(path, &sd_path) {
            b.classify_source_document_candidate(path, &file_set);
        } else if under_any(path, &instance_roots) {
            if path.ends_with(".json") {
                // No recognised sidecar suffix remains (see the
                // `SIDECAR_SUFFIX` doc comment above) — every `.json` file
                // under a reserved instance root, `.revisions.json` included,
                // is an ordinary instance candidate.
                b.classify_instance_candidate(path);
            } else {
                b.error(
                    codes::CANDIDATE_UNRECOGNISED,
                    vec![path.clone()],
                    "file under a reserved instance root is not a JSON candidate".to_string(),
                );
            }
        } else if under(path, "relations") {
            b.classify_relations_file(path);
        } else if under(path, "containers") {
            b.classify_container_candidate(path);
        } else if let Some((kind, schema)) = declared_defs.get(path) {
            b.classify_definition_candidate(path, *kind, schema);
        }
        // Everything else — package-root files not declared as definitions,
        // and files outside every reserved location — is application content:
        // not discovered, not validated, not modified ([R10]).
    }

    // Declared definition paths that resolve to no file.
    for (path, (kind, _)) in &declared_defs {
        if !file_set.contains(path) {
            b.error(
                codes::DEFINITION_PATH_MISSING,
                vec![path.clone()],
                format!(
                    "path declared in a package manifest's '{}' array resolves to no file",
                    kind.as_str()
                ),
            );
        }
    }

    // --- The inline root container (Change A: manifest is authoritative) ---

    if let Some(container) = &manifest.container {
        let locator = ROOT_CONTAINER_LOCATOR.to_string();
        if let Some(ids) = &container.root_instance_ids {
            b.container_refs
                .push((locator.clone(), "rootInstanceIds", ids.clone()));
        }
        if let Some(ids) = &container.member_instance_ids {
            b.container_refs
                .push((locator.clone(), "memberInstanceIds", ids.clone()));
        }
        b.entries.containers.push(CatalogEntry {
            id: container.container_id.clone(),
            kind: CatalogKind::Container,
            tier: None,
            locator: Some(locator),
        });
    }

    // --- Extension set (Change L; [R5] sixth location class) ---

    if declared_extensions.contains("ext:changelog") {
        if let Some(path) = &changelog_path {
            b.extension_entry(
                path,
                CatalogKind::Changelog,
                repository_id.as_deref(),
                "manifest.repositoryId",
            );
        }
    }
    // --- Set-level checks ([R12], [R13]) and ordering ([R14]) ---

    b.detect_duplicates();
    b.resolve_references();
    Ok(b.finish(package_roots))
}

impl Builder<'_> {
    fn error(&mut self, code: &'static str, locators: Vec<String>, message: String) {
        self.diagnostics.push(CatalogDiagnostic {
            code,
            severity: DiagnosticSeverity::Error,
            locators,
            message,
        });
    }

    fn warn(&mut self, code: &'static str, locators: Vec<String>, message: String) {
        self.diagnostics.push(CatalogDiagnostic {
            code,
            severity: DiagnosticSeverity::Warning,
            locators,
            message,
        });
    }

    /// Read a candidate JSON file; `None` emits the [R9] malformed diagnostic.
    fn read_candidate(&mut self, path: &str) -> Option<Value> {
        match self.store.load_instance_json(path) {
            Ok(v) => Some(v),
            Err(e) => {
                if e.is_not_found() {
                    // Enumerated then vanished — treat as absent.
                    return None;
                }
                self.error(
                    codes::CANDIDATE_MALFORMED,
                    vec![path.to_string()],
                    format!("malformed candidate under a reserved location: {e}"),
                );
                None
            }
        }
    }

    /// Classify one candidate under a reserved instance root (Changes B/C;
    /// [R6]/[R7]/[R8]). Returns true when an instance entry was produced.
    fn classify_instance_candidate(&mut self, path: &str) -> bool {
        let Some(value) = self.read_candidate(path) else {
            return false;
        };
        let admissible = [
            (srs_schema::NOTE_SCHEMA_ID, CatalogKind::Note, 0u8),
            (srs_schema::RECORD_SCHEMA_ID, CatalogKind::Record, 2),
        ];
        let registry = SchemaRegistry::global();
        let classified = if let Some(declared) = value.get("$schema").and_then(|v| v.as_str()) {
            // [R7]: declared, then validated. Never reclassified by shape.
            match admissible.iter().find(|(id, _, _)| *id == declared) {
                Some((schema_id, kind, tier)) => match registry.validate_by_id(schema_id, &value) {
                    Ok(()) => Some((*kind, *tier)),
                    Err(e) => {
                        self.error(
                            codes::SCHEMA_VALIDATION,
                            vec![path.to_string()],
                            format!("object fails its declared schema {declared}: {e}"),
                        );
                        None
                    }
                },
                None => {
                    if registry.schema_ids().contains(&declared) {
                        self.error(
                            codes::SCHEMA_INADMISSIBLE,
                            vec![path.to_string()],
                            format!(
                                "declared schema {declared} is not admissible under a reserved instance root"
                            ),
                        );
                    } else {
                        // [R7]'s allOf branch is not yet implemented: a
                        // domain schema composing a core entity via `allOf`
                        // would land here and error, not classify. No such
                        // schema exists in any fixture; resolution is owed
                        // to Phase 3 (first service consumption of the
                        // catalog). See the module-header transitional notes.
                        self.error(
                            codes::SCHEMA_UNRESOLVABLE,
                            vec![path.to_string()],
                            format!("declared schema {declared} does not resolve to a known entity schema"),
                        );
                    }
                    None
                }
            }
        } else {
            // [R8]: shape classification over the closed candidate set. The
            // discriminators are content-shape-based (`sections` / `fields` /
            // `typeId`+`fieldValues`), enforced by the schemas themselves.
            let matches: Vec<&(&str, CatalogKind, u8)> = admissible
                .iter()
                .filter(|(id, _, _)| registry.validate_by_id(id, &value).is_ok())
                .collect();
            match matches.as_slice() {
                [(_, kind, tier)] => Some((*kind, *tier)),
                [] => {
                    self.error(
                        codes::SHAPE_NO_MATCH,
                        vec![path.to_string()],
                        "object declares no $schema and validates as none of note.json, record.json \
                         (Tier 1 / TypedRecord is retired — srs#448, rfc-decision-53635966, \
                         srs-rust#888 — and no longer an admissible shape)".to_string(),
                    );
                    None
                }
                _ => {
                    let names: Vec<&str> = matches.iter().map(|(id, _, _)| *id).collect();
                    self.error(
                        codes::SHAPE_AMBIGUOUS,
                        vec![path.to_string()],
                        format!(
                            "object declares no $schema and validates as more than one candidate: {}",
                            names.join(", ")
                        ),
                    );
                    None
                }
            }
        };
        let Some((kind, tier)) = classified else {
            return false;
        };
        // Schema validation guarantees instanceId on success.
        let id = value
            .get("instanceId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        self.entries.instances.push(CatalogEntry {
            id,
            kind,
            tier: Some(tier),
            locator: Some(path.to_string()),
        });
        true
    }

    /// Classify a file under `relations/` (Change E; [R11]).
    fn classify_relations_file(&mut self, path: &str) {
        let rel = path.strip_prefix("relations/").unwrap_or(path);
        if rel.contains('/') {
            self.error(
                codes::RELATIONS_NOT_FLAT,
                vec![path.to_string()],
                "relations/ must be flat — no subfolders ([R11])".to_string(),
            );
            return;
        }
        if !path.ends_with(".json") {
            self.error(
                codes::CANDIDATE_UNRECOGNISED,
                vec![path.to_string()],
                "file under relations/ is not a JSON candidate".to_string(),
            );
            return;
        }
        let Some(value) = self.read_candidate(path) else {
            return;
        };
        if rel == "relations.json" || rel == "relations-collection.json" {
            // [R11], active since the Phase-6 flip: a live repository at
            // generation 2 must not contain a relations-collection file.
            // The one reader still entitled to enumerate one is the [R21]
            // exempt migration surface — it reads the pre-migration state by
            // definition (e.g. an upgrade preview before rfc038-storage runs).
            if self.store.rfc038_exempt() {
                match serde_json::from_value::<RelationsCollection>(value) {
                    Ok(collection) => {
                        for relation in collection.relations {
                            self.push_relation(relation, path, true);
                        }
                    }
                    Err(e) => self.error(
                        codes::CANDIDATE_MALFORMED,
                        vec![path.to_string()],
                        format!("relations collection fails to parse: {e}"),
                    ),
                }
                return;
            }
            self.error(
                codes::RELATIONS_COLLECTION_RETIRED,
                vec![path.to_string()],
                "relations-collection files are retired at dataModelRevision >= 2 ([R11]) \
                 — run the rfc038-storage migration"
                    .to_string(),
            );
            return;
        }
        // Standalone relation object ([R11]): one relation per file, filename
        // derivable from the in-file relationId, which is authoritative. `$schema`
        // is a declared, const-pinned property of the wire shape (Change E) but not
        // a field of the `Relation` struct itself (`deny_unknown_fields`) — strip it
        // before deserializing, mirroring `store::relation_object_from_value`. Every
        // relation `save_relation` writes carries this property, so failing to strip
        // it here would make every production-written relation uncatalogable.
        let mut value = value;
        if let Some(obj) = value.as_object_mut() {
            obj.remove("$schema");
        }
        match serde_json::from_value::<Relation>(value) {
            Ok(relation) => {
                let stem = rel.strip_suffix(".json").unwrap_or(rel);
                if relation.relation_id != stem {
                    self.error(
                        codes::RELATION_FILENAME_MISMATCH,
                        vec![path.to_string()],
                        format!(
                            "filename stem '{stem}' disagrees with in-file relationId '{}' — the in-file id is authoritative",
                            relation.relation_id
                        ),
                    );
                }
                self.push_relation(relation, path, false);
            }
            Err(e) => self.error(
                codes::CANDIDATE_MALFORMED,
                vec![path.to_string()],
                format!("object under relations/ is not a valid Relation: {e}"),
            ),
        }
    }

    fn push_relation(&mut self, relation: Relation, path: &str, from_collection: bool) {
        if relation.relation_id.is_empty() {
            self.error(
                codes::CANDIDATE_MALFORMED,
                vec![path.to_string()],
                "relation with no relationId".to_string(),
            );
            return;
        }
        let locator = if from_collection {
            format!("{path}#{}", relation.relation_id)
        } else {
            path.to_string()
        };
        self.relation_endpoints.push((
            relation.relation_id.clone(),
            relation.source_instance_id.clone(),
            relation.target_instance_id.clone(),
            locator.clone(),
        ));
        self.entries.relations.push(CatalogEntry {
            id: relation.relation_id,
            kind: CatalogKind::Relation,
            tier: None,
            locator: Some(locator),
        });
    }

    /// Classify a candidate under `containers/` ([R7]/[R8]; singleton
    /// admissible set `container.json`).
    fn classify_container_candidate(&mut self, path: &str) {
        if !path.ends_with(".json") {
            self.error(
                codes::CANDIDATE_UNRECOGNISED,
                vec![path.to_string()],
                "file under containers/ is not a JSON candidate".to_string(),
            );
            return;
        }
        let Some(value) = self.read_candidate(path) else {
            return;
        };
        let registry = SchemaRegistry::global();
        if let Some(declared) = value.get("$schema").and_then(|v| v.as_str()) {
            if declared != srs_schema::CONTAINER_SCHEMA_ID {
                let code = if registry.schema_ids().contains(&declared) {
                    codes::SCHEMA_INADMISSIBLE
                } else {
                    codes::SCHEMA_UNRESOLVABLE
                };
                self.error(
                    code,
                    vec![path.to_string()],
                    format!("declared schema {declared} is not admissible under containers/"),
                );
                return;
            }
        }
        if let Err(e) = registry.validate_by_id(srs_schema::CONTAINER_SCHEMA_ID, &value) {
            let (code, msg) = if value.get("$schema").is_some() {
                (
                    codes::SCHEMA_VALIDATION,
                    format!("object fails container.json: {e}"),
                )
            } else {
                (
                    codes::SHAPE_NO_MATCH,
                    format!(
                        "object declares no $schema and does not validate as container.json: {e}"
                    ),
                )
            };
            self.error(code, vec![path.to_string()], msg);
            return;
        }
        let id = value
            .get("containerId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        for prop in ["rootInstanceIds", "memberInstanceIds"] {
            if let Some(ids) = value.get(prop).and_then(|v| v.as_array()) {
                let ids: Vec<String> = ids
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect();
                let prop_static: &'static str = if prop == "rootInstanceIds" {
                    "rootInstanceIds"
                } else {
                    "memberInstanceIds"
                };
                self.container_refs
                    .push((path.to_string(), prop_static, ids));
            }
        }
        self.entries.containers.push(CatalogEntry {
            id,
            kind: CatalogKind::Container,
            tier: None,
            locator: Some(path.to_string()),
        });
    }

    /// Classify a candidate under `sourceDocumentsPath` (Change G; [R9]/[R15]).
    fn classify_source_document_candidate(&mut self, path: &str, _file_set: &BTreeSet<String>) {
        if !path.ends_with(".meta.json") {
            // Opaque source payload: never parsed, never classified, preserved
            // unmodified ([R9]). An absent content file for a sidecar is a
            // valid tombstone, so no cross-check runs here ([R15]).
            return;
        }
        let Some(value) = self.read_candidate(path) else {
            return;
        };
        // [R7] over the sidecar entity. The legacy `source-document.json` id
        // is accepted as a transitional shim for a filed corpus defect
        // (srs#369) — see LEGACY_SOURCE_DOCUMENT_SCHEMA_ID.
        if let Some(declared) = value.get("$schema").and_then(|v| v.as_str()) {
            if declared != srs_schema::SOURCE_DOCUMENT_META_SCHEMA_ID
                && declared != LEGACY_SOURCE_DOCUMENT_SCHEMA_ID
            {
                let code = if SchemaRegistry::global().schema_ids().contains(&declared) {
                    codes::SCHEMA_INADMISSIBLE
                } else {
                    codes::SCHEMA_UNRESOLVABLE
                };
                self.error(
                    code,
                    vec![path.to_string()],
                    format!(
                        "declared schema {declared} does not resolve to the source-document sidecar entity"
                    ),
                );
                return;
            }
        }
        // [R15]: the sidecar is the identity; documentId must be parseable.
        // Checked before schema validation so the specific [R15] diagnostic
        // is not masked by the schema's own `required` clause.
        let document_id = match value.get("documentId").and_then(|v| v.as_str()) {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                self.error(
                    codes::SIDECAR_NO_DOCUMENT_ID,
                    vec![path.to_string()],
                    "source-document sidecar has no parseable documentId ([R15])".to_string(),
                );
                return;
            }
        };
        // Validate the body against the sidecar schema regardless of which
        // alias was declared (the legacy id names the same entity).
        let mut body = value.clone();
        if let Some(obj) = body.as_object_mut() {
            obj.remove("$schema");
        }
        if let Err(e) = SchemaRegistry::global()
            .validate_by_id(srs_schema::SOURCE_DOCUMENT_META_SCHEMA_ID, &body)
        {
            self.error(
                codes::SCHEMA_VALIDATION,
                vec![path.to_string()],
                format!("sidecar fails source-document-meta.json: {e}"),
            );
            return;
        }
        // A sidecar whose content file is absent remains a valid source
        // document — the tombstone case ([R15]); no cross-check runs.
        self.entries.source_documents.push(CatalogEntry {
            id: document_id,
            kind: CatalogKind::SourceDocument,
            tier: None,
            locator: Some(path.to_string()),
        });
    }

    /// Classify one declared definition candidate ([R7]/[R8]; the admissible
    /// set at a declared definition path is a singleton, so there is nothing
    /// to discriminate against).
    fn classify_definition_candidate(
        &mut self,
        path: &str,
        kind: CatalogKind,
        schema_id: &'static str,
    ) {
        let Some(value) = self.read_candidate(path) else {
            return;
        };
        let registry = SchemaRegistry::global();
        if let Some(declared) = value.get("$schema").and_then(|v| v.as_str()) {
            if declared != schema_id {
                let code = if registry.schema_ids().contains(&declared) {
                    codes::SCHEMA_INADMISSIBLE
                } else {
                    codes::SCHEMA_UNRESOLVABLE
                };
                self.error(
                    code,
                    vec![path.to_string()],
                    format!(
                        "declared schema {declared} does not match the declared definition kind '{}' ({schema_id})",
                        kind.as_str()
                    ),
                );
                return;
            }
            if let Err(e) = registry.validate_by_id(schema_id, &value) {
                self.error(
                    codes::SCHEMA_VALIDATION,
                    vec![path.to_string()],
                    format!("object fails its declared schema {schema_id}: {e}"),
                );
                return;
            }
        } else if let Err(e) = registry.validate_by_id(schema_id, &value) {
            self.error(
                codes::SHAPE_NO_MATCH,
                vec![path.to_string()],
                format!("object declares no $schema and does not validate as {schema_id}: {e}"),
            );
            return;
        }
        let id_prop = if kind == CatalogKind::Protocol {
            "protocolId"
        } else {
            "id"
        };
        let Some(id) = value.get(id_prop).and_then(|v| v.as_str()) else {
            self.error(
                codes::CANDIDATE_MALFORMED,
                vec![path.to_string()],
                format!("definition object has no '{id_prop}' identifier"),
            );
            return;
        };
        self.definition_dup_keys.push((
            id.to_string(),
            value.get("version").and_then(|v| v.as_u64()),
            path.to_string(),
        ));
        if kind == CatalogKind::Type {
            let field_ids: Vec<String> = value
                .get("fields")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|fa| fa.get("fieldId").and_then(|v| v.as_str()))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            self.type_field_refs.push((path.to_string(), field_ids));
        }
        self.entries.definitions.push(CatalogEntry {
            id: id.to_string(),
            kind,
            tier: None,
            locator: Some(path.to_string()),
        });
    }

    /// Add an extension-set entry with `{kind, id}` identity (Change L).
    /// A declared path with no file present enumerates nothing.
    fn extension_entry(
        &mut self,
        path: &str,
        kind: CatalogKind,
        id: Option<&str>,
        id_source: &str,
    ) {
        // The location is reserved only when the file exists; a declared but
        // absent aggregate is simply empty.
        let exists = match self.store.load_instance_json(path) {
            Ok(_) => true,
            Err(e) if e.is_not_found() => false,
            Err(_) => {
                self.error(
                    codes::CANDIDATE_MALFORMED,
                    vec![path.to_string()],
                    format!("extension aggregate '{}' is not valid JSON", kind.as_str()),
                );
                false
            }
        };
        if !exists {
            return;
        }
        match id {
            Some(id) if !id.is_empty() => self.entries.extensions.push(CatalogEntry {
                id: id.to_string(),
                kind,
                tier: None,
                locator: Some(path.to_string()),
            }),
            _ => self.error(
                codes::EXTENSION_IDENTITY,
                vec![path.to_string()],
                format!(
                    "extension aggregate '{}' has no identity projection ({id_source} is absent)",
                    kind.as_str()
                ),
            ),
        }
    }

    /// [R12]: duplicate logical ids within a set are errors naming every
    /// conflicting locator; never resolved by precedence or enumeration order.
    ///
    /// Uniqueness is global within each core set; definitions key by
    /// `(id, version)` because a versioned lineage legally shares its UUID
    /// across version files; the extension set keys by `{kind, id}`.
    fn detect_duplicates(&mut self) {
        let mut dups: Vec<(String, Vec<String>)> = Vec::new();
        for (set_name, entries) in [
            ("instance", &self.entries.instances),
            ("relation", &self.entries.relations),
            ("container", &self.entries.containers),
            ("source-document", &self.entries.source_documents),
        ] {
            let mut by_id: BTreeMap<&str, Vec<String>> = BTreeMap::new();
            for e in entries {
                by_id
                    .entry(e.id.as_str())
                    .or_default()
                    .push(e.locator.clone().unwrap_or_default());
            }
            for (id, locators) in by_id {
                if locators.len() > 1 {
                    dups.push((
                        format!(
                            "duplicate {set_name} identifier '{id}' declared by {} objects",
                            locators.len()
                        ),
                        locators,
                    ));
                }
            }
        }
        let mut by_def_key: BTreeMap<(String, Option<u64>), Vec<String>> = BTreeMap::new();
        for (id, version, locator) in &self.definition_dup_keys {
            by_def_key
                .entry((id.clone(), *version))
                .or_default()
                .push(locator.clone());
        }
        for ((id, version), locators) in by_def_key {
            if locators.len() > 1 {
                let v = version.map_or(String::new(), |v| format!("@{v}"));
                dups.push((
                    format!(
                        "duplicate definition identifier '{id}{v}' declared by {} objects",
                        locators.len()
                    ),
                    locators,
                ));
            }
        }
        let mut by_ext_key: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
        for e in &self.entries.extensions {
            by_ext_key
                .entry((e.kind.as_str(), e.id.as_str()))
                .or_default()
                .push(e.locator.clone().unwrap_or_default());
        }
        for ((kind, id), locators) in by_ext_key {
            if locators.len() > 1 {
                dups.push((
                    format!(
                        "duplicate extension identity {{{kind}, {id}}} declared by {} objects",
                        locators.len()
                    ),
                    locators,
                ));
            }
        }
        for (message, mut locators) in dups {
            locators.sort();
            self.error(codes::DUPLICATE_ID, locators, message);
        }
    }

    /// [R13]: references resolve against the set they target.
    fn resolve_references(&mut self) {
        let instance_ids: BTreeSet<&str> = self
            .entries
            .instances
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        let field_ids: BTreeSet<&str> = self
            .entries
            .definitions
            .iter()
            .filter(|e| e.kind == CatalogKind::Field)
            .map(|e| e.id.as_str())
            .collect();

        let mut errors: Vec<(Vec<String>, String)> = Vec::new();
        for (relation_id, source, target, locator) in &self.relation_endpoints {
            for (prop, id) in [("sourceInstanceId", source), ("targetInstanceId", target)] {
                if !instance_ids.contains(id.as_str()) {
                    errors.push((
                        vec![locator.clone()],
                        format!(
                            "relation '{relation_id}' {prop} '{id}' resolves to nothing in the instance set"
                        ),
                    ));
                }
            }
        }
        for (locator, prop, ids) in &self.container_refs {
            for id in ids {
                if !instance_ids.contains(id.as_str()) {
                    errors.push((
                        vec![locator.clone()],
                        format!("container {prop} '{id}' resolves to nothing in the instance set"),
                    ));
                }
            }
        }
        for (locator, field_refs) in &self.type_field_refs {
            for field_id in field_refs {
                if !field_ids.contains(field_id.as_str()) {
                    errors.push((
                        vec![locator.clone()],
                        format!(
                            "FieldAssignment.fieldId '{field_id}' resolves to nothing in the definition set"
                        ),
                    ));
                }
            }
        }
        for (locators, message) in errors {
            self.error(codes::DANGLING_REFERENCE, locators, message);
        }
    }

    fn finish(mut self, mut package_roots: Vec<String>) -> RepositoryCatalog {
        // [R14]: deterministic total order by logical identifier, byte-wise
        // over the canonical lowercase hyphenated UUID form — ids are
        // lowercased before comparison so a mixed-case id cannot perturb the
        // order; locator as a stability tiebreak only.
        for set in [
            &mut self.entries.instances,
            &mut self.entries.relations,
            &mut self.entries.containers,
            &mut self.entries.source_documents,
            &mut self.entries.definitions,
        ] {
            set.sort_by(|a, b| {
                a.id.to_ascii_lowercase()
                    .cmp(&b.id.to_ascii_lowercase())
                    .then_with(|| a.locator.cmp(&b.locator))
            });
        }
        // The extension set orders first by kind byte-wise, then id ([R14]).
        self.entries.extensions.sort_by(|a, b| {
            a.kind
                .as_str()
                .cmp(b.kind.as_str())
                .then_with(|| a.id.to_ascii_lowercase().cmp(&b.id.to_ascii_lowercase()))
        });
        self.diagnostics.sort_by(|a, b| {
            a.locators
                .cmp(&b.locators)
                .then_with(|| a.code.cmp(b.code))
                .then_with(|| a.message.cmp(&b.message))
        });
        RepositoryCatalog {
            instances: self.entries.instances,
            relations: self.entries.relations,
            containers: self.entries.containers,
            source_documents: self.entries.source_documents,
            definitions: self.entries.definitions,
            extensions: self.entries.extensions,
            diagnostics: self.diagnostics,
            package_roots: {
                package_roots.sort();
                package_roots.dedup();
                package_roots
            },
        }
    }
}

// ---------------------------------------------------------------------------
// [R8] standing check
// ---------------------------------------------------------------------------

/// The pairwise-discriminator property of the instance candidate set, checked
/// against the mirrored schemas rather than assumed ([R8], Change C): every
/// schema is `additionalProperties: false` and has at least one `required`
/// property that is not a declared property of any other schema in the set.
/// Returns an error description when the property no longer holds.
pub fn instance_discriminator_error() -> Option<&'static String> {
    static CHECK: OnceLock<Option<String>> = OnceLock::new();
    CHECK
        .get_or_init(|| {
            let ids = [srs_schema::NOTE_SCHEMA_ID, srs_schema::RECORD_SCHEMA_ID];
            let mut parsed: Vec<(&str, Value)> = Vec::new();
            for id in ids {
                let Some(src) = srs_schema::schema_source(id) else {
                    return Some(format!("schema source for {id} unavailable"));
                };
                match serde_json::from_str::<Value>(src) {
                    Ok(v) => parsed.push((id, v)),
                    Err(e) => return Some(format!("schema {id} unparseable: {e}")),
                }
            }
            for (id, schema) in &parsed {
                if schema.get("additionalProperties") != Some(&Value::Bool(false)) {
                    return Some(format!("{id} is not additionalProperties: false"));
                }
                let required: BTreeSet<&str> = schema
                    .get("required")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                let others: BTreeSet<&str> = parsed
                    .iter()
                    .filter(|(other_id, _)| other_id != id)
                    .flat_map(|(_, s)| {
                        s.get("properties")
                            .and_then(|v| v.as_object())
                            .map(|o| o.keys().map(String::as_str).collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .collect();
                if required.difference(&others).next().is_none() {
                    return Some(format!(
                        "{id} has no required property outside the other candidates' declared properties"
                    ));
                }
            }
            None
        })
        .as_ref()
}

// ---------------------------------------------------------------------------
// Path helpers (repo-relative, forward-slash — the Vfs convention)
// ---------------------------------------------------------------------------

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Segment-wise prefix test ([R19]'s anti-glob discipline): `path` is under
/// directory `dir` iff `dir` is a whole-segment prefix.
fn under(path: &str, dir: &str) -> bool {
    !dir.is_empty()
        && path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn under_any(path: &str, dirs: &BTreeSet<String>) -> bool {
    dirs.iter().any(|d| under(path, d))
}

fn dir_exists(file_set: &BTreeSet<String>, dir: &str) -> bool {
    let prefix = format!("{dir}/");
    file_set
        .range(prefix.clone()..)
        .next()
        .is_some_and(|p| p.starts_with(&prefix))
}
