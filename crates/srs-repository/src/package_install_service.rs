//! # Package Install Service (#506)
//!
//! One-shot install of an external package directory (e.g. the canonical
//! `com.mudemocracy.governance` package) into an *existing* repository.
//!
//! Unlike `package_service::import_package_local` — which only registers a
//! boundary whose files already live inside the repository — `install_package`
//! copies every definition from an external source directory into a sub-package
//! boundary (default `packages/<name>`), registers the boundary (RFC-014), and
//! reports what was installed, skipped, or in conflict.
//!
//! ## Layering
//!
//! Reading the external source directory is **input acquisition**, not target-repo
//! storage: it happens in [`load_package_source_dir`] with direct `std::fs` reads,
//! exactly parallel to the repo-loading machinery in `store.rs`
//! (`load_package_from_dir`). All *target-repository* I/O goes through the
//! [`RepositoryStore`] trait, so [`install_package_bundle`] works against
//! `FileStore`, `JsonStore`, and `MemoryStore` alike.
//!
//! ## Conflict semantics
//!
//! - **Skip** identical-UUID definitions already present anywhere in the repo
//!   (embedded core, primary package, or any sub-package boundary). Counted as
//!   `skipped_identical`. This makes re-running the install idempotent.
//! - **Flag** same-key/different-UUID collisions (e.g. the source ships a
//!   `precedes` relation type but the target already defines `precedes` under a
//!   different UUID). These are *not* installed — they are listed in
//!   `conflicts` and the install still succeeds, unless `strict` is set, in
//!   which case the install fails before writing anything.
//!
//! "Same key" follows how each definition kind's identity works in this codebase:
//! `namespace/name@version` for fields, types, views, document views, themes,
//! blueprints, lifecycles, and vocabularies; the `key` string for relation types
//! (the loader hard-errors on same-key/different-content relation types, so
//! duplicating those would break `load_package`); and
//! `protocolNamespace/protocolName@protocolVersion` for protocols.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::de::Error as SerdeDeError;
use serde::{Deserialize, Serialize};
use srs_core::extensions::import_tracking::{
    ConflictState, DefinitionType, ImportMode, ImportRecord, ImportSummary,
};

use crate::error::RepositoryError;
use crate::package_service::{create_package, CreatePackageInput};
use crate::package_types::{validate_package_selector, DefinitionKind, PackageSelector};
use crate::store::{definition_kind_key, RepositoryStore};

/// Definition kinds handled by install, in install order (dependencies first).
const INSTALL_ORDER: [DefinitionKind; 10] = [
    DefinitionKind::Field,
    DefinitionKind::Type,
    DefinitionKind::RelationType,
    DefinitionKind::Lifecycle,
    DefinitionKind::Vocabulary,
    DefinitionKind::View,
    DefinitionKind::DocumentView,
    DefinitionKind::Theme,
    DefinitionKind::Blueprint,
    DefinitionKind::Protocol,
];

/// Human-readable singular label for a definition kind (used in reports).
fn kind_label(kind: DefinitionKind) -> &'static str {
    match kind {
        DefinitionKind::Field => "field",
        DefinitionKind::Type => "type",
        DefinitionKind::View => "view",
        DefinitionKind::DocumentView => "documentView",
        DefinitionKind::RelationType => "relationType",
        DefinitionKind::Blueprint => "blueprint",
        DefinitionKind::Protocol => "protocol",
        DefinitionKind::Vocabulary => "vocabulary",
        DefinitionKind::Lifecycle => "lifecycle",
        DefinitionKind::Theme => "theme",
    }
}

// ---------------------------------------------------------------------------
// Source bundle
// ---------------------------------------------------------------------------

/// One definition read from an external package source.
#[derive(Debug, Clone)]
pub struct PackageSourceDefinition {
    pub kind: DefinitionKind,
    /// Path relative to the package directory, exactly as listed in the source
    /// `package.json` (e.g. `"fields/title-d7e82557.json"`). Preserved verbatim
    /// in the target boundary.
    pub rel_path: String,
    /// Raw definition JSON, copied verbatim into the target repository.
    pub value: serde_json::Value,
}

/// An external package loaded into memory: metadata plus all definitions.
#[derive(Debug, Clone)]
pub struct PackageSourceBundle {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub definitions: Vec<PackageSourceDefinition>,
}

/// Read an external package directory (containing `package.json` plus the
/// definition subdirectories it indexes) into a [`PackageSourceBundle`].
///
/// This is the input-acquisition layer: it reads the *source* from the local
/// filesystem, mirroring `load_package_from_dir` in `store.rs`. It never touches
/// the target repository.
///
/// Every definition is validated with the same strictness the repository loader
/// applies (typed parse + core validation for views/document-views/themes/
/// relation types), so a successful install can never leave the target repo in a
/// state where `load_package()` fails.
pub fn load_package_source_dir(source_dir: &Path) -> Result<PackageSourceBundle, RepositoryError> {
    let pkg_json_path = source_dir.join("package.json");
    if !pkg_json_path.is_file() {
        return Err(RepositoryError::PackageRefMissing {
            path: source_dir.display().to_string(),
        });
    }
    let pkg_json = read_json_file(&pkg_json_path)?;

    let meta_str = |key: &str| -> Result<String, RepositoryError> {
        pkg_json[key]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "source package.json at {} is missing required '{key}'",
                    pkg_json_path.display()
                ),
            })
    };

    let mut bundle = PackageSourceBundle {
        id: meta_str("id")?,
        namespace: meta_str("namespace")?,
        name: meta_str("name")?,
        version: meta_str("version")?,
        definitions: Vec::new(),
    };

    for kind in INSTALL_ORDER {
        let Some(entries) = pkg_json[definition_kind_key(kind)].as_array() else {
            continue;
        };
        for entry in entries {
            let Some(rel_path) = entry.as_str() else {
                continue;
            };
            validate_source_rel_path(rel_path)?;
            let full = source_dir.join(rel_path);
            let value = read_json_file(&full)?;
            validate_source_definition(kind, &full, &value)?;
            if definition_id(kind, &value).is_none() {
                return Err(RepositoryError::InvalidRepositoryInitialization {
                    message: format!(
                        "source {} at {} is missing its identity field",
                        kind_label(kind),
                        full.display()
                    ),
                });
            }
            bundle.definitions.push(PackageSourceDefinition {
                kind,
                rel_path: rel_path.to_string(),
                value,
            });
        }
    }

    Ok(bundle)
}

fn read_json_file(path: &Path) -> Result<serde_json::Value, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|e| RepositoryError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    serde_json::from_str(&content).map_err(|e| RepositoryError::Serialize {
        path: path.to_path_buf(),
        source: e,
    })
}

/// A source-relative definition path must stay inside the source directory.
fn validate_source_rel_path(rel_path: &str) -> Result<(), RepositoryError> {
    if rel_path.trim().is_empty()
        || rel_path.starts_with('/')
        || rel_path.split('/').any(|c| c == "..")
    {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!("source package.json lists an invalid definition path '{rel_path}'"),
        });
    }
    Ok(())
}

/// Validate a source definition with the same strictness `load_package()` applies.
fn validate_source_definition(
    kind: DefinitionKind,
    path: &Path,
    value: &serde_json::Value,
) -> Result<(), RepositoryError> {
    let parse_err = |msg: String| RepositoryError::PackageLoad {
        path: path.to_path_buf(),
        source: serde_json::Error::custom(msg),
    };
    let require_str = |key: &str| -> Result<(), RepositoryError> {
        if value[key].as_str().is_none_or(|s| s.trim().is_empty()) {
            return Err(parse_err(format!("missing required string '{key}'")));
        }
        Ok(())
    };
    match kind {
        // Fields and types mirror the loader's lenient FieldJson/TypeJson shapes:
        // check the load-bearing keys instead of a strict typed parse.
        DefinitionKind::Field => {
            require_str("id")?;
            require_str("namespace")?;
            require_str("name")?;
            // RFC-032/RFC-039: a Field carries an inline `fieldType`; the
            // load-bearing key is its `datatype`.
            let datatype = value["fieldType"]["datatype"].as_str().unwrap_or("");
            const ALLOWED: [&str; 9] = [
                "string",
                "number",
                "integer",
                "boolean",
                "date",
                "date-time",
                "ref",
                "map",
                "dependent",
            ];
            if !ALLOWED.contains(&datatype) {
                return Err(RepositoryError::InvalidValueType {
                    path: path.to_path_buf(),
                    value_type: datatype.to_string(),
                });
            }
        }
        DefinitionKind::Type => {
            require_str("id")?;
            require_str("namespace")?;
            require_str("name")?;
            if !value["fields"].is_array() {
                return Err(parse_err("missing required array 'fields'".to_string()));
            }
        }
        DefinitionKind::RelationType => {
            let def: srs_core::types::relation_type_definition::RelationTypeDefinition =
                serde_json::from_value(value.clone()).map_err(|source| {
                    RepositoryError::PackageLoad {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
            srs_core::validation::relation_type_definition::validate_relation_type_definition(&def)
                .map_err(|source| RepositoryError::RelationTypeDefinitionValidation {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        DefinitionKind::View => {
            let view: srs_core::types::view::View =
                serde_json::from_value(value.clone()).map_err(|source| {
                    RepositoryError::ViewLoad {
                        path: path.to_path_buf(),
                        source,
                    }
                })?;
            srs_core::validation::view::validate_view(&view).map_err(|source| {
                RepositoryError::ViewValidation {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
        }
        DefinitionKind::DocumentView => {
            let dv: srs_core::types::view::DocumentView = serde_json::from_value(value.clone())
                .map_err(|source| RepositoryError::DocumentViewLoad {
                    path: path.to_path_buf(),
                    source,
                })?;
            srs_core::validation::view::validate_document_view(&dv).map_err(|source| {
                RepositoryError::DocumentViewValidation {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
        }
        DefinitionKind::Theme => {
            let theme: srs_core::types::theme::Theme = serde_json::from_value(value.clone())
                .map_err(|source| RepositoryError::ThemeLoad {
                    path: path.to_path_buf(),
                    source,
                })?;
            srs_core::validation::theme::validate_theme(&theme).map_err(|source| {
                RepositoryError::ThemeValidation {
                    path: path.to_path_buf(),
                    source,
                }
            })?;
        }
        DefinitionKind::Blueprint => {
            serde_json::from_value::<srs_core::types::blueprint::Blueprint>(value.clone())
                .map_err(|source| RepositoryError::PackageLoad {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        DefinitionKind::Protocol => {
            serde_json::from_value::<srs_core::types::protocol::Protocol>(value.clone()).map_err(
                |source| RepositoryError::PackageLoad {
                    path: path.to_path_buf(),
                    source,
                },
            )?;
        }
        DefinitionKind::Vocabulary => {
            serde_json::from_value::<srs_core::types::vocabulary::Vocabulary>(value.clone())
                .map_err(|source| RepositoryError::PackageLoad {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        DefinitionKind::Lifecycle => {
            serde_json::from_value::<srs_core::types::lifecycle::Lifecycle>(value.clone())
                .map_err(|source| RepositoryError::PackageLoad {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Definition identity
// ---------------------------------------------------------------------------

/// Stable UUID identity of a definition (`protocolId` for protocols, `id` otherwise).
fn definition_id(kind: DefinitionKind, value: &serde_json::Value) -> Option<String> {
    let key = match kind {
        DefinitionKind::Protocol => "protocolId",
        _ => "id",
    };
    value[key]
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

/// Logical identity key of a definition, per how each kind's identity works here.
fn definition_key(kind: DefinitionKind, value: &serde_json::Value) -> Option<String> {
    match kind {
        DefinitionKind::RelationType => value["key"]
            .as_str()
            .or_else(|| value["relationType"].as_str())
            .map(str::to_string),
        DefinitionKind::Protocol => {
            let ns = value["protocolNamespace"].as_str()?;
            let name = value["protocolName"].as_str()?;
            Some(format!("{ns}/{name}@{}", value["protocolVersion"]))
        }
        _ => {
            let ns = value["namespace"].as_str()?;
            let name = value["name"].as_str()?;
            Some(format!("{ns}/{name}@{}", value["version"]))
        }
    }
}

/// Map a DefinitionKind to its DefinitionType (returns None for unmappable kinds).
fn to_definition_type(kind: DefinitionKind) -> Option<DefinitionType> {
    match kind {
        DefinitionKind::Field => Some(DefinitionType::Field),
        DefinitionKind::Type => Some(DefinitionType::Type),
        DefinitionKind::RelationType => Some(DefinitionType::RelationType),
        DefinitionKind::View => Some(DefinitionType::View),
        DefinitionKind::Blueprint => Some(DefinitionType::Blueprint),
        DefinitionKind::Protocol => Some(DefinitionType::Protocol),
        DefinitionKind::DocumentView
        | DefinitionKind::Lifecycle
        | DefinitionKind::Vocabulary
        | DefinitionKind::Theme => None,
    }
}

/// Extract the namespace string from a definition JSON.
fn definition_namespace(kind: DefinitionKind, value: &serde_json::Value) -> Option<String> {
    match kind {
        DefinitionKind::Protocol => value["protocolNamespace"].as_str().map(str::to_string),
        _ => value["namespace"].as_str().map(str::to_string),
    }
}

/// Extract the logical name from a definition JSON.
fn definition_name(kind: DefinitionKind, value: &serde_json::Value) -> Option<String> {
    match kind {
        DefinitionKind::Protocol => value["protocolName"].as_str().map(str::to_string),
        DefinitionKind::RelationType => value["key"].as_str().map(str::to_string),
        _ => value["name"].as_str().map(str::to_string),
    }
}

/// Extract the version as u32 from a definition JSON (defaults to 1 if absent).
fn definition_version(kind: DefinitionKind, value: &serde_json::Value) -> u32 {
    let v = match kind {
        DefinitionKind::Protocol => &value["protocolVersion"],
        _ => &value["version"],
    };
    v.as_u64().unwrap_or(1) as u32
}

/// Index of definitions already present in the target repository.
#[derive(Default)]
struct ExistingIndex {
    /// (kind label, uuid)
    ids: HashSet<(&'static str, String)>,
    /// (kind label, logical key) → existing uuid
    keys: HashMap<(&'static str, String), String>,
}

impl ExistingIndex {
    fn insert(&mut self, kind: DefinitionKind, id: String, key: Option<String>) {
        let label = kind_label(kind);
        if let Some(k) = key {
            self.keys.entry((label, k)).or_insert_with(|| id.clone());
        }
        self.ids.insert((label, id));
    }
}

/// Collect every definition already present in the repository: the embedded
/// `com.semanticops.core` package plus every package boundary (primary and
/// sub-packages), walked through the store so it works on all adapters.
fn collect_existing(store: &dyn RepositoryStore) -> Result<ExistingIndex, RepositoryError> {
    let mut idx = ExistingIndex::default();

    // Embedded core (RFC-018): merged into every load_package(), so identical-UUID
    // copies shipped by a source package must be skipped, not duplicated.
    let core = crate::core_package::core_package();
    for f in &core.fields {
        idx.insert(
            DefinitionKind::Field,
            f.id.clone(),
            Some(format!("{}/{}@{}", f.namespace, f.name, f.version)),
        );
    }
    for t in &core.record_types {
        idx.insert(
            DefinitionKind::Type,
            t.id.clone(),
            Some(format!("{}/{}@{}", t.namespace, t.name, t.version)),
        );
    }

    for boundary in store.list_package_boundaries()? {
        let prefix = boundary.selector.as_deref().unwrap_or("package");
        let Ok(pkg_json) = store.load_instance_json(&format!("{prefix}/package.json")) else {
            continue;
        };
        for kind in INSTALL_ORDER {
            let Some(entries) = pkg_json[definition_kind_key(kind)].as_array() else {
                continue;
            };
            for entry in entries {
                let Some(rel) = entry.as_str() else { continue };
                let Ok(value) = store.load_instance_json(&format!("{prefix}/{rel}")) else {
                    continue;
                };
                if let Some(id) = definition_id(kind, &value) {
                    idx.insert(kind, id, definition_key(kind, &value));
                }
            }
        }
    }

    Ok(idx)
}

// ---------------------------------------------------------------------------
// Install input / result types
// ---------------------------------------------------------------------------

/// Input for [`install_package`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPackageInput {
    /// Filesystem path of the source package directory (contains `package.json`).
    pub source_dir: String,
    /// Target boundary path relative to the repo root. Defaults to
    /// `packages/<source package name>`.
    #[serde(default)]
    pub boundary_path: Option<String>,
    /// When true, fail (before writing anything) if any same-key/different-UUID
    /// conflict is found instead of skipping the conflicting definitions.
    #[serde(default)]
    pub strict: bool,
}

/// Options for [`install_package_bundle`] (the source-agnostic install core).
#[derive(Debug, Clone, Default)]
pub struct InstallBundleOptions {
    /// See [`InstallPackageInput::boundary_path`].
    pub boundary_path: Option<String>,
    /// See [`InstallPackageInput::strict`].
    pub strict: bool,
}

/// A same-key/different-UUID collision between the source and the target repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallConflictDetail {
    /// Definition kind label (`field`, `type`, `relationType`, ...).
    pub kind: String,
    /// The colliding logical key (e.g. `precedes` or `governance/title@1`).
    pub key: String,
    /// UUID the source ships for this key (not installed).
    pub source_id: String,
    /// UUID the target repository already defines for this key.
    pub existing_id: String,
}

/// Per-kind install counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallKindCount {
    pub kind: String,
    pub installed: usize,
    pub skipped_identical: usize,
    pub conflicts: usize,
}

/// Result of [`install_package`] / [`install_package_bundle`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallPackageResult {
    /// Boundary the package was installed into (or found already registered at).
    pub boundary_path: String,
    /// Upstream package identity (provenance echo).
    pub package_id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    /// When the boundary's provenance stamp says the package was installed.
    /// Preserved across re-runs (idempotent installs keep the original stamp).
    pub installed_at: String,
    /// Total definitions written into the boundary by this run.
    pub installed: usize,
    /// Total identical-UUID definitions skipped because they already exist.
    pub skipped_identical: usize,
    /// Same-key/different-UUID collisions (skipped, not silently duplicated).
    pub conflicts: Vec<InstallConflictDetail>,
    /// Per-kind breakdown, in install order, for kinds present in the source.
    pub kinds: Vec<InstallKindCount>,
}

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

/// Install an external package directory into the repository (one service call
/// per CLI handler, ADR-010). Loads the source from disk, then delegates to
/// [`install_package_bundle`].
pub fn install_package(
    store: &dyn RepositoryStore,
    input: InstallPackageInput,
) -> Result<InstallPackageResult, RepositoryError> {
    let source_dir = input.source_dir.trim();
    if source_dir.is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "source_dir must not be empty".to_string(),
        });
    }
    let bundle = load_package_source_dir(&PathBuf::from(source_dir))?;
    install_package_bundle(
        store,
        &bundle,
        InstallBundleOptions {
            boundary_path: input.boundary_path,
            strict: input.strict,
        },
    )
}

/// Install an in-memory [`PackageSourceBundle`] into the repository.
///
/// Source-agnostic core of the install: all target I/O goes through the store,
/// so this runs against `FileStore`, `JsonStore`, and `MemoryStore`.
///
/// Explicit IDs from the source are honoured for everything installed, which is
/// what makes re-running the install skip everything (idempotence).
pub fn install_package_bundle(
    store: &dyn RepositoryStore,
    bundle: &PackageSourceBundle,
    options: InstallBundleOptions,
) -> Result<InstallPackageResult, RepositoryError> {
    for (label, value) in [
        ("id", &bundle.id),
        ("namespace", &bundle.namespace),
        ("name", &bundle.name),
        ("version", &bundle.version),
    ] {
        if value.trim().is_empty() {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: format!("source package {label} must not be empty"),
            });
        }
    }

    let requested_path = options
        .boundary_path
        .clone()
        .unwrap_or_else(|| format!("packages/{}", bundle.name));
    validate_package_selector(&Some(requested_path.clone()))?;
    if requested_path == "package" {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "cannot install into the primary package boundary 'package'".to_string(),
        });
    }

    // ── Phase 1: analyse (no writes) ─────────────────────────────────────────
    let mut existing = collect_existing(store)?;

    enum Decision {
        Install,
        SkipIdentical,
        Conflict,
    }

    let mut decisions: Vec<Decision> = Vec::with_capacity(bundle.definitions.len());
    let mut conflicts: Vec<InstallConflictDetail> = Vec::new();
    // Per-kind counters keyed by label, filled in INSTALL_ORDER below.
    let mut counts: HashMap<&'static str, InstallKindCount> = HashMap::new();

    for def in &bundle.definitions {
        let label = kind_label(def.kind);
        let id = definition_id(def.kind, &def.value).ok_or_else(|| {
            RepositoryError::InvalidRepositoryInitialization {
                message: format!(
                    "source {label} '{}' is missing its identity field",
                    def.rel_path
                ),
            }
        })?;
        let key = definition_key(def.kind, &def.value);

        let entry = counts.entry(label).or_insert_with(|| InstallKindCount {
            kind: label.to_string(),
            installed: 0,
            skipped_identical: 0,
            conflicts: 0,
        });

        if existing.ids.contains(&(label, id.clone())) {
            entry.skipped_identical += 1;
            decisions.push(Decision::SkipIdentical);
            continue;
        }
        if let Some(existing_id) = key
            .as_ref()
            .and_then(|k| existing.keys.get(&(label, k.clone())))
            .cloned()
        {
            entry.conflicts += 1;
            conflicts.push(InstallConflictDetail {
                kind: label.to_string(),
                key: key.clone().unwrap_or_default(),
                source_id: id.clone(),
                existing_id,
            });
            decisions.push(Decision::Conflict);
            continue;
        }

        entry.installed += 1;
        decisions.push(Decision::Install);
        // Track within-run so later source entries can't duplicate earlier ones.
        existing.insert(def.kind, id, key);
    }

    if options.strict && !conflicts.is_empty() {
        let keys = conflicts
            .iter()
            .map(|c| format!("{} '{}'", c.kind, c.key))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(RepositoryError::PackageInstallConflicts {
            count: conflicts.len(),
            keys,
        });
    }

    // ── Phase 2: boundary create-or-reuse ────────────────────────────────────
    let boundaries = store.list_package_boundaries()?;
    let boundary_path = match boundaries.iter().find(|b| b.id == bundle.id) {
        Some(existing_boundary) => match &existing_boundary.selector {
            Some(path) => path.clone(),
            None => {
                return Err(RepositoryError::InvalidRepositoryInitialization {
                    message: format!(
                        "source package id '{}' is the repository's primary package — nothing to install",
                        bundle.id
                    ),
                });
            }
        },
        None => {
            create_package(
                store,
                CreatePackageInput {
                    id: bundle.id.clone(),
                    namespace: bundle.namespace.clone(),
                    name: bundle.name.clone(),
                    version: bundle.version.clone(),
                    boundary_path: Some(requested_path.clone()),
                },
            )?;
            requested_path
        }
    };
    let selector: PackageSelector = Some(boundary_path.clone());

    // ── Phase 3: write installs ──────────────────────────────────────────────
    let mut installed_total = 0usize;
    let mut skipped_total = 0usize;
    for (def, decision) in bundle.definitions.iter().zip(&decisions) {
        match decision {
            Decision::Install => {
                if let Some((dir, _)) = def.rel_path.rsplit_once('/') {
                    store.ensure_instance_dir(&format!("{boundary_path}/{dir}"))?;
                }
                store
                    .save_instance_json(&format!("{boundary_path}/{}", def.rel_path), &def.value)?;
                store.add_definition_to_boundary(&selector, def.kind, &def.rel_path)?;
                installed_total += 1;
            }
            Decision::SkipIdentical => skipped_total += 1,
            Decision::Conflict => {}
        }
    }

    // ── Phase 4: provenance stamp ────────────────────────────────────────────
    // Record the upstream package identity + installedAt on the boundary's
    // package.json (`upstreamPackage`, matching the manifest-level convention
    // stamped by init_new_repository for seeded governance repos). Minimal
    // provenance per #245/#246 / srs#107 — not the full ImportRecord machinery.
    let pkg_json_key = format!("{boundary_path}/package.json");
    let mut boundary_pkg_json = store.load_instance_json(&pkg_json_key)?;
    let installed_at = boundary_pkg_json["upstreamPackage"]["installedAt"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    boundary_pkg_json["upstreamPackage"] = serde_json::json!({
        "packageId": bundle.id,
        "namespace": bundle.namespace,
        "name": bundle.name,
        "version": bundle.version,
        "installedAt": installed_at,
    });
    store.save_instance_json(&pkg_json_key, &boundary_pkg_json)?;

    // ── Phase 5: import records + reference copies ───────────────────────────
    // Only written when something was actually installed (re-runs preserve the
    // existing ImportSummary because every definition is skipped-identical).
    // Per ADR-030: import-record writes are best-effort; a failure here does not
    // affect the definitions already committed in Phases 1–4.
    if installed_total > 0 {
        let _ = (|| -> Result<(), RepositoryError> {
            let import_prefix = format!("{boundary_path}/.srs-import");
            store.ensure_instance_dir(&import_prefix)?;
            store.ensure_instance_dir(&format!("{import_prefix}/refs"))?;

            let mut summary = ImportSummary {
                generated_at: installed_at.clone(),
                fields: Vec::new(),
                types: Vec::new(),
                views: Vec::new(),
                blueprints: Vec::new(),
                protocols: Vec::new(),
                relation_types: Vec::new(),
                skipped_definitions: Vec::new(),
            };

            for (def, decision) in bundle.definitions.iter().zip(&decisions) {
                if !matches!(decision, Decision::Install) {
                    continue;
                }
                let Some(def_type) = to_definition_type(def.kind) else {
                    summary.skipped_definitions.push(def.rel_path.clone());
                    continue;
                };
                let Some(id) = definition_id(def.kind, &def.value) else {
                    continue;
                };

                // Write reference copy alongside the installed definition.
                if let Some((dir, _)) = def.rel_path.rsplit_once('/') {
                    store.ensure_instance_dir(&format!("{import_prefix}/refs/{dir}"))?;
                }
                store.save_instance_json(
                    &format!("{import_prefix}/refs/{}", def.rel_path),
                    &def.value,
                )?;

                let namespace = definition_namespace(def.kind, &def.value).unwrap_or_default();
                let name = definition_name(def.kind, &def.value).unwrap_or_default();
                let version = definition_version(def.kind, &def.value);

                let record = ImportRecord {
                    definition_id: id,
                    definition_type: def_type.clone(),
                    namespace,
                    name,
                    version,
                    mode: ImportMode::UpstreamTracked,
                    imported_at: installed_at.clone(),
                    source_package_id: bundle.id.clone(),
                    source_package_name: bundle.namespace.clone(),
                    source_package_version: bundle.version.clone(),
                    latest_known_upstream_version: None,
                    update_available: None,
                    update_checked_at: None,
                    conflict_state: Some(ConflictState::Clean),
                    conflict_detected_at: None,
                    local_version: None,
                    local_edited_at: None,
                };

                match def_type {
                    DefinitionType::Field => summary.fields.push(record),
                    DefinitionType::Type => summary.types.push(record),
                    DefinitionType::View => summary.views.push(record),
                    DefinitionType::Blueprint => summary.blueprints.push(record),
                    DefinitionType::Protocol => summary.protocols.push(record),
                    DefinitionType::RelationType => summary.relation_types.push(record),
                }
            }

            let summary_path = format!("{import_prefix}/import-records.json");
            let summary_value =
                serde_json::to_value(&summary).map_err(|e| RepositoryError::Serialize {
                    path: PathBuf::from(&summary_path),
                    source: e,
                })?;
            store.save_instance_json(&summary_path, &summary_value)?;
            Ok(())
        })();
    }

    let kinds = INSTALL_ORDER
        .iter()
        .filter_map(|kind| counts.remove(kind_label(*kind)))
        .collect();

    Ok(InstallPackageResult {
        boundary_path,
        package_id: bundle.id.clone(),
        namespace: bundle.namespace.clone(),
        name: bundle.name.clone(),
        version: bundle.version.clone(),
        installed_at,
        installed: installed_total,
        skipped_identical: skipped_total,
        conflicts,
        kinds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;

    fn field_json(id: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "namespace": "com.ext.pkg",
            "name": name,
            "version": 1,
            "valueType": "string",
            "description": "A bundled field.",
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn relation_type_json(id: &str, key: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "version": 1,
            "key": key,
            "namespace": "com.ext.pkg",
            "label": "Precedes",
            "description": "Source comes before target.",
            "category": "sequence",
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn bundle() -> PackageSourceBundle {
        PackageSourceBundle {
            id: "ext-pkg-0001".to_string(),
            namespace: "com.ext.pkg".to_string(),
            name: "ext".to_string(),
            version: "1.0.0".to_string(),
            definitions: vec![
                PackageSourceDefinition {
                    kind: DefinitionKind::Field,
                    rel_path: "fields/alpha.json".to_string(),
                    value: field_json("00000000-0000-4000-8000-0000000000a1", "alpha"),
                },
                PackageSourceDefinition {
                    kind: DefinitionKind::Field,
                    rel_path: "fields/beta.json".to_string(),
                    value: field_json("00000000-0000-4000-8000-0000000000b1", "beta"),
                },
                PackageSourceDefinition {
                    kind: DefinitionKind::RelationType,
                    rel_path: "relation-types/precedes.json".to_string(),
                    value: relation_type_json("00000000-0000-4000-8000-0000000000c1", "precedes"),
                },
            ],
        }
    }

    #[test]
    fn memory_install_uses_default_boundary_and_counts() {
        let store = MemoryStore::default();
        let result =
            install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();
        assert_eq!(result.boundary_path, "packages/ext");
        assert_eq!(result.installed, 3);
        assert_eq!(result.skipped_identical, 0);
        assert!(result.conflicts.is_empty());

        // Boundary registered with the source package identity.
        let boundary = crate::store::RepositoryStore::load_package_boundary(
            &store,
            &Some("packages/ext".to_string()),
        )
        .unwrap();
        assert_eq!(boundary.id, "ext-pkg-0001");
        assert_eq!(boundary.field_paths.len(), 2);

        // Provenance stamp on the boundary package.json.
        let pkg_json =
            crate::store::RepositoryStore::load_instance_json(&store, "packages/ext/package.json")
                .unwrap();
        assert_eq!(
            pkg_json["upstreamPackage"]["packageId"].as_str(),
            Some("ext-pkg-0001")
        );
        assert!(pkg_json["upstreamPackage"]["installedAt"]
            .as_str()
            .is_some());
    }

    #[test]
    fn memory_install_writes_import_summary_and_reference_copies() {
        let store = MemoryStore::default();
        let result =
            install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();

        // ImportSummary written at the canonical path.
        let summary_json = crate::store::RepositoryStore::load_instance_json(
            &store,
            "packages/ext/.srs-import/import-records.json",
        )
        .expect("import-records.json must exist after install");

        assert_eq!(
            summary_json["generatedAt"].as_str(),
            Some(result.installed_at.as_str())
        );
        assert_eq!(summary_json["fields"].as_array().unwrap().len(), 2);
        assert_eq!(summary_json["types"].as_array().unwrap().len(), 0);
        assert_eq!(summary_json["relationTypes"].as_array().unwrap().len(), 1);

        let field0 = &summary_json["fields"][0];
        assert_eq!(field0["sourcePackageId"].as_str(), Some("ext-pkg-0001"));
        assert_eq!(field0["sourcePackageName"].as_str(), Some("com.ext.pkg"));
        assert_eq!(field0["mode"].as_str(), Some("upstream-tracked"));
        assert_eq!(field0["definitionType"].as_str(), Some("field"));
        assert_eq!(field0["conflictState"].as_str(), Some("clean"));

        // Reference copies present alongside the installed files.
        let ref_alpha = crate::store::RepositoryStore::load_instance_json(
            &store,
            "packages/ext/.srs-import/refs/fields/alpha.json",
        )
        .expect("reference copy for alpha must exist");
        assert_eq!(
            ref_alpha["id"].as_str(),
            Some("00000000-0000-4000-8000-0000000000a1")
        );
    }

    #[test]
    fn memory_rerun_preserves_existing_import_summary() {
        let store = MemoryStore::default();
        let first =
            install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();
        // Second run: everything skipped-identical → Phase 5 is skipped (installed_total == 0).
        install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();

        // The summary from the first run is still there and unchanged.
        let summary_json = crate::store::RepositoryStore::load_instance_json(
            &store,
            "packages/ext/.srs-import/import-records.json",
        )
        .expect("import-records.json must survive re-run");
        assert_eq!(
            summary_json["generatedAt"].as_str(),
            Some(first.installed_at.as_str())
        );
        assert_eq!(summary_json["fields"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn memory_rerun_is_idempotent() {
        let store = MemoryStore::default();
        let first =
            install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();
        let second =
            install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();
        assert_eq!(second.installed, 0);
        assert_eq!(second.skipped_identical, 3);
        assert!(second.conflicts.is_empty());
        assert_eq!(second.installed_at, first.installed_at);
    }

    #[test]
    fn memory_flags_same_key_different_uuid_conflict() {
        let store = MemoryStore::default();
        let existing: srs_core::types::relation_type_definition::RelationTypeDefinition =
            serde_json::from_value(relation_type_json(
                "00000000-9999-4000-8000-000000000099",
                "precedes",
            ))
            .unwrap();
        crate::package_service::create_relation_type(&store, existing, None).unwrap();

        let result =
            install_package_bundle(&store, &bundle(), InstallBundleOptions::default()).unwrap();
        assert_eq!(result.installed, 2);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].kind, "relationType");
        assert_eq!(result.conflicts[0].key, "precedes");
        assert_eq!(
            result.conflicts[0].existing_id,
            "00000000-9999-4000-8000-000000000099"
        );
    }

    #[test]
    fn memory_strict_fails_on_conflict() {
        let store = MemoryStore::default();
        let existing: srs_core::types::relation_type_definition::RelationTypeDefinition =
            serde_json::from_value(relation_type_json(
                "00000000-9999-4000-8000-000000000099",
                "precedes",
            ))
            .unwrap();
        crate::package_service::create_relation_type(&store, existing, None).unwrap();

        let err = install_package_bundle(
            &store,
            &bundle(),
            InstallBundleOptions {
                boundary_path: None,
                strict: true,
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::PackageInstallConflicts { count: 1, .. }
        ));
        // No boundary written in strict failure.
        assert!(crate::store::RepositoryStore::load_package_boundary(
            &store,
            &Some("packages/ext".to_string())
        )
        .is_err());
    }

    #[test]
    fn memory_skips_identical_uuid_from_embedded_core() {
        // Ship a copy of an embedded-core field: same UUID → skipped, not duplicated.
        let core_field = &crate::core_package::core_package().fields[0];
        let mut b = bundle();
        b.definitions.push(PackageSourceDefinition {
            kind: DefinitionKind::Field,
            rel_path: "fields/core-copy.json".to_string(),
            value: serde_json::json!({
                "id": core_field.id,
                "namespace": core_field.namespace,
                "name": core_field.name,
                "version": core_field.version,
                "valueType": "string",
                "description": "embedded core copy",
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        });

        let store = MemoryStore::default();
        let result = install_package_bundle(&store, &b, InstallBundleOptions::default()).unwrap();
        assert_eq!(result.installed, 3);
        assert_eq!(result.skipped_identical, 1);
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn memory_rejects_empty_metadata() {
        let store = MemoryStore::default();
        let mut b = bundle();
        b.name = "  ".to_string();
        let err = install_package_bundle(&store, &b, InstallBundleOptions::default()).unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::InvalidRepositoryInitialization { .. }
        ));
    }
}
