//! # `.srspkg` Package Bundle codec (ADR-050; srs-rust#632, #690, #663, #1212)
//!
//! One reader ([`read_package_bundle`], bytes in, store-free) and one writer
//! ([`export_package_bundle`], deterministic bytes out), conforming to RFC-003
//! Revision 10: the writer's closure follows the reference-site table
//! (`reference_sites`), the reader brings older bundles forward step by step ([C6]). Both install entry
//! points (directory, bundle) end in the one
//! [`crate::package_install_service::install_package_bundle`] core.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use srs_schema::{SchemaRegistry, PACKAGE_BUNDLE_SCHEMA_ID};

use crate::error::RepositoryError;
use crate::field_type_migration_service::{CURRENT_DATA_MODEL_REVISION, DATA_MODEL_REVISION_KEY};
use crate::migration_registry_service::{revision_step_from, BundleForm};
use crate::package_install_service::{
    definition_name, kind_label, load_boundary_definitions, validate_source_definition,
    PackageSourceBundle, PackageSourceDefinition, INSTALL_ORDER,
};
use crate::package_service::definition_rel_path;
use crate::package_types::{DefinitionKind, PackageBoundary};
use crate::reference_sites::followed_references;
use crate::store::{definition_kind_key, RepositoryStore};

/// The reader's result: the bundle plus non-fatal pre-load transformer notes
/// (RFC-043 `migration-memberorder-dropped`, ...).
#[derive(Debug, Clone)]
pub struct ReadPackageBundle {
    pub bundle: PackageSourceBundle,
    pub notes: Vec<String>,
}

fn refuse(code: &'static str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::InvalidPackageBundle {
        code,
        message: message.into(),
    }
}

/// Appended to a content-shape failure on a bundle older than current (PD1d).
fn revision_hint(rev: u64) -> String {
    if rev < CURRENT_DATA_MODEL_REVISION {
        format!("; bundle declares dataModelRevision {rev}; re-export it with a current srs")
    } else {
        String::new()
    }
}

/// Bring a bundle stamped `rev` forward to the current revision, one registry
/// step at a time (RFC-003 [C6]): `Restamp` steps set the stamp, `Transform`
/// steps rewrite the bundle; the stamp is written after every step. A step
/// with no bundle form refuses with `bundle-migration-step-missing`.
fn bring_forward(v: &mut Value, rev: u64) -> Result<Vec<String>, RepositoryError> {
    let mut notes = Vec::new();
    for r in rev..CURRENT_DATA_MODEL_REVISION {
        let step = revision_step_from(r);
        let Some((_, step)) = step.filter(|(_, s)| !matches!(s.bundle, BundleForm::Unspecified))
        else {
            let id = step.map_or("unregistered", |(id, _)| id);
            return Err(refuse(
                "bundle-migration-step-missing",
                format!(
                    "bundle declares dataModelRevision {rev}{}; data-model step \
                     {r} -> {} ({id}) has no bundle-form transformer; this srs reads bundles \
                     from revision {}; re-export it with a current srs",
                    if rev == 0 { " (absent = 0)" } else { "" },
                    r + 1,
                    reader_floor()
                ),
            ));
        };
        if let BundleForm::Transform(f) = step.bundle {
            notes.extend(f(v).map_err(|e| refuse("bundle-migration-refused", e.to_string()))?);
        }
        v[DATA_MODEL_REVISION_KEY] = Value::from(step.to);
    }
    Ok(notes)
}

/// Read a `.srspkg` from its bytes: parse, refuse a `readme` (srs-rust#1164),
/// refuse a newer revision, bring an older one forward through the registry's
/// bundle forms or refuse naming the missing step ([C6], [`bring_forward`]),
/// validate against `package-bundle.json`, then validate every definition with
/// the loader's own strictness.
pub fn read_package_bundle(bytes: &[u8]) -> Result<ReadPackageBundle, RepositoryError> {
    let mut v: Value = serde_json::from_slice(bytes)
        .map_err(|e| refuse("bundle-not-json", format!("not a JSON document: {e}")))?;
    if !v.is_object() {
        return Err(refuse(
            "bundle-not-json",
            "a package bundle is a JSON object",
        ));
    }
    if v.get("readme").is_some() {
        return Err(refuse(
            "bundle-readme-unsupported",
            "this srs cannot carry a bundle readme yet (RFC-045, srs-rust#1164); \
             remove `readme` and retry",
        ));
    }
    let rev = v
        .get(DATA_MODEL_REVISION_KEY)
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if rev > CURRENT_DATA_MODEL_REVISION {
        return Err(refuse(
            "bundle-revision-too-new",
            format!(
                "bundle declares dataModelRevision {rev}; this srs supports up to \
                 {CURRENT_DATA_MODEL_REVISION}; upgrade srs"
            ),
        ));
    }
    let notes = bring_forward(&mut v, rev)?;
    SchemaRegistry::global()
        .validate_by_id(PACKAGE_BUNDLE_SCHEMA_ID, &v)
        .map_err(|e| {
            refuse(
                "bundle-schema-invalid",
                format!("{e}{}", revision_hint(rev)),
            )
        })?;

    let mut definitions = Vec::new();
    for kind in INSTALL_ORDER {
        let key = definition_kind_key(kind);
        let Some(items) = v.get(key).and_then(Value::as_array) else {
            continue;
        };
        for (index, item) in items.iter().enumerate() {
            let at = format!("<bundle>/{key}/{index}");
            let invalid = |msg: String| {
                refuse(
                    "bundle-definition-invalid",
                    format!("{at}: {msg}{}", revision_hint(rev)),
                )
            };
            validate_source_definition(kind, Path::new(&at), item)
                .map_err(|e| invalid(e.to_string()))?;
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| invalid("missing `id`".to_string()))?;
            // A blank name would slugify to "" and collide; refuse it.
            let name = definition_name(kind, item)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| invalid("missing name".to_string()))?;
            definitions.push(PackageSourceDefinition {
                kind,
                rel_path: versioned_rel_path(definition_rel_path(kind, &name, id), item),
                value: item.clone(),
            });
        }
    }

    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let package_dependencies = v["packageDependencies"]
        .as_array()
        .filter(|a| !a.is_empty())
        .cloned();
    Ok(ReadPackageBundle {
        bundle: PackageSourceBundle {
            id: s("packageId"),
            namespace: s("packageNamespace"),
            name: s("packageName"),
            version: s("packageVersion"),
            package_dependencies,
            definitions,
        },
        notes,
    })
}

/// Each version of a definition keeps its own file (#1267): v1 keeps the
/// legacy path, later versions get a `-v{n}` suffix.
fn versioned_rel_path(path: String, item: &Value) -> String {
    match item.get("version").and_then(Value::as_u64) {
        Some(v) if v > 1 => version_suffixed(&path, v),
        _ => path,
    }
}

/// The one `-v{n}` path rule (reader, install and upgrade): `a/b.json` -> `a/b-v2.json`.
pub(crate) fn version_suffixed(path: &str, version: u64) -> String {
    match path.strip_suffix(".json") {
        Some(stem) => format!("{stem}-v{version}.json"),
        None => format!("{path}-v{version}"),
    }
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// A `.srspkg` `mode` (RFC-003): `bundled` carries the reference closure,
/// `standalone` carries the source package's own definitions only. Both list
/// every reached definition in `dependencyRefs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BundleMode {
    #[default]
    Bundled,
    Standalone,
}

impl BundleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            BundleMode::Bundled => "bundled",
            BundleMode::Standalone => "standalone",
        }
    }
}

impl std::str::FromStr for BundleMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "bundled" => Ok(BundleMode::Bundled),
            "standalone" => Ok(BundleMode::Standalone),
            other => Err(format!(
                "unknown bundle mode '{other}' (expected bundled or standalone)"
            )),
        }
    }
}

/// Input for [`export_package_bundle`] (also the WASM `input_json` contract).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPackageInput {
    /// Package boundary selector; `None` = the primary package.
    #[serde(default)]
    pub selector: Option<String>,
    /// RFC 3339; `None` = now (UTC). Part of the bytes: a reproducible sha256
    /// needs a fixed value.
    #[serde(default)]
    pub published_at: Option<String>,
    /// Optional `publisher`; omitted from the bundle when `None`.
    #[serde(default)]
    pub publisher: Option<String>,
    /// `bundled` (default) or `standalone` (RFC-003; owner ruling O4).
    #[serde(default)]
    pub mode: BundleMode,
    /// Optional `homepage` (RFC-003 property table: caller-supplied); omitted when `None`.
    #[serde(default)]
    pub homepage: Option<String>,
}

/// An exported `.srspkg`: the exact text plus its summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageBundleExport {
    /// The exact `.srspkg` text (pretty JSON + trailing newline); the sha256 is over these bytes.
    pub text: String,
    pub summary: PackageExportSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExportSummary {
    pub package_id: String,
    pub package_namespace: String,
    pub package_name: String,
    pub package_version: String,
    /// The repository's own `dataModelRevision` stamp, written into the bundle ([C6]).
    pub data_model_revision: u64,
    pub published_at: String,
    /// `sha256:<64 lowercase hex>` over `text`'s UTF-8 bytes.
    pub sha256: String,
    pub byte_length: usize,
    pub definition_count: usize,
    /// Ids carried from other package boundaries by the closure (sorted); core
    /// definitions are listed, never carried. Always empty in `standalone`.
    pub inlined: Vec<String>,
    /// Per-kind counts, install order, kinds with count > 0 only.
    pub kinds: Vec<PackageExportKindCount>,
    /// Entries in the bundle's `dependencyRefs`.
    pub dependency_ref_count: usize,
    pub mode: BundleMode,
    /// Non-fatal export notes; today only `bundle-below-reader-floor` (PD11).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExportKindCount {
    pub kind: String,
    pub count: usize,
}

/// `package-bundle.json` `Reference.definitionType` (kebab `relation-type`,
/// unlike `kind_label`'s `relationType`).
fn definition_type(kind: DefinitionKind) -> &'static str {
    match kind {
        DefinitionKind::RelationType => "relation-type",
        other => kind_label(other),
    }
}

/// One `(id, version)` of the effective package set (PD3, PD6).
struct Holding {
    kind: DefinitionKind,
    namespace: String,
    name: String,
    /// A boundary's copy; `None` when only the embedded core holds it.
    value: Option<Value>,
    /// Where the copy came from (`package`, a sub-package path, or `core`).
    from: String,
    /// `Some(other location)` when two holders differ (PD3).
    conflict: Option<String>,
    /// The embedded core holds this exact `(id, version)`: listed, never carried
    /// unless the source package lists it (`rfc-decision-a8dcbfe5`).
    core: bool,
}

type EffectiveIndex = BTreeMap<String, BTreeMap<u64, Holding>>;

/// First `(id, version, "a and b")` held differently by two of `boundaries` (or
/// by one and the embedded core), reusing the export's PD3 comparison.
pub(crate) fn first_identity_conflict(
    store: &dyn RepositoryStore,
    boundaries: &[PackageBoundary],
) -> Result<Option<(String, u64, String)>, RepositoryError> {
    Ok(effective_index(store, boundaries)?
        .into_iter()
        .find_map(|(id, vs)| {
            vs.into_iter()
                .find_map(|(v, h)| h.conflict.map(|at| (id.clone(), v, at)))
        }))
}

/// Every boundary's definitions plus the embedded core's Fields and Types
/// (PD6: core RelationTypes are KEYED, never reached, so not indexed).
fn effective_index(
    store: &dyn RepositoryStore,
    boundaries: &[PackageBoundary],
) -> Result<EffectiveIndex, RepositoryError> {
    let mut index = EffectiveIndex::new();
    for b in boundaries {
        let from = b.selector.as_deref().unwrap_or("package").to_string();
        for d in load_boundary_definitions(store, b)? {
            let Some(id) = d.value["id"].as_str().map(str::to_string) else {
                continue;
            };
            let version = d.value["version"].as_u64().unwrap_or(0);
            match index.entry(id).or_default().entry(version) {
                Entry::Vacant(e) => {
                    e.insert(Holding {
                        kind: d.kind,
                        namespace: d.value["namespace"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        name: definition_name(d.kind, &d.value).unwrap_or_default(),
                        value: Some(d.value),
                        from: from.clone(),
                        conflict: None,
                        core: false,
                    });
                }
                Entry::Occupied(mut e) => {
                    let h = e.get_mut();
                    let same = h.kind == d.kind
                        && h.value.clone().map(|v| crate::srsj::canonicalize(v, false))
                            == Some(crate::srsj::canonicalize(d.value, false));
                    if !same && h.conflict.is_none() {
                        h.conflict = Some(format!("{} and {from}", h.from));
                    }
                }
            }
        }
    }
    let core = crate::core_package::core_package();
    let core_items = core
        .fields
        .iter()
        .map(|f| {
            (
                DefinitionKind::Field,
                &f.id,
                f.version,
                &f.namespace,
                &f.name,
            )
        })
        .chain(core.record_types.iter().map(|t| {
            (
                DefinitionKind::Type,
                &t.id,
                t.version,
                &t.namespace,
                &t.name,
            )
        }));
    for (kind, id, version, namespace, name) in core_items {
        match index
            .entry(id.clone())
            .or_default()
            .entry(u64::from(version))
        {
            Entry::Vacant(e) => {
                e.insert(Holding {
                    kind,
                    namespace: namespace.clone(),
                    name: name.clone(),
                    value: None,
                    from: "core".to_string(),
                    conflict: None,
                    core: true,
                });
            }
            // ADR-025: same id and version with the same namespace and name is the
            // same core definition; anything else is an identity conflict.
            Entry::Occupied(mut e) => {
                let h = e.get_mut();
                h.core = true;
                if (h.kind != kind || &h.namespace != namespace || &h.name != name)
                    && h.conflict.is_none()
                {
                    h.conflict = Some(format!("{} and core", h.from));
                }
            }
        }
    }
    Ok(index)
}

/// The lowest `dataModelRevision` this srs reads a bundle from: walk down from
/// current while each step has a bundle form ([C6]).
fn reader_floor() -> u64 {
    let mut floor = CURRENT_DATA_MODEL_REVISION;
    while floor > 0
        && revision_step_from(floor - 1)
            .is_some_and(|(_, s)| !matches!(s.bundle, BundleForm::Unspecified))
    {
        floor -= 1;
    }
    floor
}

/// Export a whole package boundary as a deterministic `.srspkg` (ADR-050,
/// RFC-003 Revision 10): the source package's own definitions plus, in
/// `bundled` mode, every definition its PINNED and LINEAGE references reach
/// ([C1], `reference_sites`), except core definitions, which are listed in
/// `dependencyRefs` and never carried. An unresolved followed reference, an
/// identity conflict or a carried definition failing its schema refuses the
/// export and writes nothing. Keys canonicalized, definitions sorted by id then
/// version, validated against `package-bundle.json` before it is returned.
pub fn export_package_bundle(
    store: &dyn RepositoryStore,
    input: ExportPackageInput,
) -> Result<PackageBundleExport, RepositoryError> {
    let published_at = match input.published_at {
        Some(s) => {
            chrono::DateTime::parse_from_rfc3339(&s).map_err(|e| {
                refuse(
                    "bundle-published-at-invalid",
                    format!("publishedAt '{s}' is not RFC 3339: {e}"),
                )
            })?;
            s
        }
        None => chrono::Utc::now().to_rfc3339(),
    };
    let boundaries = store.list_package_boundaries()?;
    let target = boundaries
        .iter()
        .find(|b| b.selector == input.selector)
        .ok_or_else(|| RepositoryError::PackageNotFound {
            selector: input.selector.clone(),
        })?;

    // The target's index must load: `load_boundary_definitions` treats a missing
    // index as "contributes nothing", which here would be a silently empty bundle.
    let prefix = target.selector.as_deref().unwrap_or("package");
    let target_index = store
        .load_instance_json(&format!("{prefix}/package.json"))
        .map_err(|e| {
            refuse(
                "bundle-boundary-unreadable",
                format!("package boundary '{prefix}': {prefix}/package.json cannot be loaded: {e}"),
            )
        })?;

    let index = effective_index(store, &boundaries)?;
    let conflict = |id: &str, version: u64, h: &Holding| match &h.conflict {
        Some(at) => Err(refuse(
            "bundle-identity-conflict",
            format!(
                "{} {id} version {version} has two different definitions ({at}); \
                 an export cannot choose between them",
                kind_label(h.kind)
            ),
        )),
        None => Ok(()),
    };
    let validate = |kind: DefinitionKind, id: &str, v: &Value| {
        validate_source_definition(kind, Path::new(&format!("<bundle>/{id}")), v).map_err(|e| {
            refuse(
                "bundle-definition-invalid",
                format!("{} {id}: {e}", kind_label(kind)),
            )
        })
    };

    // Carried definitions keyed (id, version): iteration order is the bundle's.
    let mut carried: BTreeMap<(String, u64), (DefinitionKind, Value)> = BTreeMap::new();
    let mut queue: Vec<(DefinitionKind, Value)> = Vec::new();
    for d in load_boundary_definitions(store, target)? {
        let id = d.value["id"].as_str().unwrap_or_default().to_string();
        validate(d.kind, &id, &d.value)?;
        let version = d.value["version"].as_u64().unwrap_or(0);
        if let Some(h) = index.get(&id).and_then(|vs| vs.get(&version)) {
            conflict(&id, version, h)?;
        }
        carried.insert((id, version), (d.kind, d.value.clone()));
        queue.push((d.kind, d.value));
    }

    // dependencyRefs keyed (id, version, definitionType) -> (namespace, name).
    let mut refs: BTreeMap<(String, u64, &'static str), (String, String)> = BTreeMap::new();
    let mut inlined = BTreeSet::new();
    while let Some((kind, value)) = queue.pop() {
        let from_id = value["id"].as_str().unwrap_or_default().to_string();
        for r in followed_references(kind, &value) {
            let held: Vec<(u64, &Holding)> = index
                .get(&r.id)
                .into_iter()
                .flatten()
                .filter(|(v, h)| h.kind == r.target && r.version.is_none_or(|want| **v == want))
                .map(|(v, h)| (*v, h))
                .collect();
            if held.is_empty() {
                let version = r
                    .version
                    .map_or_else(|| "any version".to_string(), |v| format!("version {v}"));
                return Err(refuse(
                    "bundle-reference-unresolved",
                    format!(
                        "{} {from_id} {}: {} {} ({version}) is not in the repository's \
                         package set or the core package",
                        kind_label(kind),
                        r.path,
                        kind_label(r.target),
                        r.id
                    ),
                ));
            }
            for (version, h) in held {
                conflict(&r.id, version, h)?;
                refs.insert(
                    (r.id.clone(), version, definition_type(h.kind)),
                    (h.namespace.clone(), h.name.clone()),
                );
                let key = (r.id.clone(), version);
                if input.mode == BundleMode::Standalone || h.core || carried.contains_key(&key) {
                    continue;
                }
                let Some(v) = h.value.clone() else { continue };
                validate(h.kind, &r.id, &v)?;
                inlined.insert(r.id.clone());
                carried.insert(key, (h.kind, v.clone()));
                queue.push((h.kind, v));
            }
        }
    }

    let data_model_revision = crate::field_type_migration_service::data_model_revision(store)?;
    let mut bundle = serde_json::Map::new();
    let mut put = |k: &str, v: Value| {
        bundle.insert(k.to_string(), v);
    };
    put("$schema", Value::from(PACKAGE_BUNDLE_SCHEMA_ID));
    put("schemaVersion", Value::from("2.0"));
    put("packageId", Value::from(target.id.clone()));
    put("packageNamespace", Value::from(target.namespace.clone()));
    put("packageName", Value::from(target.name.clone()));
    put("packageVersion", Value::from(target.version.clone()));
    put(DATA_MODEL_REVISION_KEY, Value::from(data_model_revision));
    put("publishedAt", Value::from(published_at.clone()));
    if let Some(p) = input.publisher {
        put("publisher", Value::from(p));
    }
    if let Some(d) = target_index.get("description").filter(|d| d.is_string()) {
        put("description", d.clone());
    }
    if let Some(h) = input.homepage {
        put("homepage", Value::from(h));
    }
    put("mode", Value::from(input.mode.as_str()));
    let mut kinds = Vec::new();
    for kind in INSTALL_ORDER {
        let items: Vec<Value> = carried
            .values()
            .filter(|(k, _)| *k == kind)
            .map(|(_, v)| v.clone())
            .collect();
        if !items.is_empty() {
            kinds.push(PackageExportKindCount {
                kind: kind_label(kind).to_string(),
                count: items.len(),
            });
        }
        if !items.is_empty() || matches!(kind, DefinitionKind::Field | DefinitionKind::Type) {
            put(definition_kind_key(kind), Value::Array(items));
        }
    }
    let dependency_ref_count = refs.len();
    put(
        "dependencyRefs",
        Value::Array(
            refs.into_iter()
                .map(|((id, version, definition_type), (namespace, name))| {
                    serde_json::json!({"id": id, "namespace": namespace, "name": name,
                        "version": version, "definitionType": definition_type})
                })
                .collect(),
        ),
    );
    put(
        "packageDependencies",
        Value::Array(target.package_dependencies.clone().unwrap_or_default()),
    );

    let value = crate::srsj::canonicalize(Value::Object(bundle), false);
    SchemaRegistry::global()
        .validate_by_id(PACKAGE_BUNDLE_SCHEMA_ID, &value)
        .map_err(|e| {
            let msg = e.to_string();
            let hint = if msg.contains("[/packageDependencies") {
                "; repair with `srs package dependency add --repair-legacy`"
            } else {
                ""
            };
            refuse("bundle-schema-invalid", format!("{msg}{hint}"))
        })?;
    let mut text =
        serde_json::to_string_pretty(&value).map_err(|e| RepositoryError::Serialize {
            path: std::path::PathBuf::from("<bundle>"),
            source: e,
        })?;
    text.push('\n');
    let floor = reader_floor();
    let notes = if data_model_revision < floor {
        vec![format!(
            "bundle-below-reader-floor: repository dataModelRevision {data_model_revision}; \
             readers at revision {CURRENT_DATA_MODEL_REVISION} refuse bundles below {floor}; \
             migrate the repository first (srs repo apply-migration)"
        )]
    } else {
        Vec::new()
    };
    let summary = PackageExportSummary {
        package_id: target.id.clone(),
        package_namespace: target.namespace.clone(),
        package_name: target.name.clone(),
        package_version: target.version.clone(),
        data_model_revision,
        published_at,
        sha256: crate::attachment_service::sha256_hex(text.as_bytes()),
        byte_length: text.len(),
        definition_count: carried.len(),
        inlined: inlined.into_iter().collect(),
        kinds,
        dependency_ref_count,
        mode: input.mode,
        notes,
    };
    Ok(PackageBundleExport { text, summary })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_install_service::{install_package, InstallPackageInput};
    use crate::package_service::{create_package, CreatePackageInput};
    use crate::repository_lifecycle::{
        InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata,
    };
    use crate::store::FileStore;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    const AT: &str = "2026-10-03T00:00:00Z";
    const FIXTURE: &str = "packages/install-fixture";

    fn fresh() -> (TempDir, FileStore) {
        let temp = TempDir::new().unwrap();
        let store = FileStore::new(temp.path());
        crate::repository_lifecycle::create_repository(
            &store,
            &InitializeRepositoryInput {
                repository: RepositoryMetadata {
                    repository_id: "17575e57-0000-4000-8000-175753e57002".to_string(),
                    namespace: "com.test.export".to_string(),
                    srs_version: "2.0-draft".to_string(),
                    title: Some("Export Test".to_string()),
                    description: None,
                },
                primary_package: PrimaryPackageMetadata {
                    id: "export-test-primary".to_string(),
                    namespace: "com.test.export".to_string(),
                    name: "primary".to_string(),
                    version: "1.0.0".to_string(),
                },
            },
        )
        .unwrap();
        (temp, store)
    }

    fn with_fixture() -> (TempDir, FileStore) {
        let (t, store) = fresh();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/install-package");
        install_package(
            &store,
            InstallPackageInput {
                source_dir: dir.display().to_string(),
                boundary_path: None,
                strict: false,
            },
        )
        .unwrap();
        (t, store)
    }

    fn export(store: &FileStore, selector: Option<&str>) -> PackageBundleExport {
        export_package_bundle(
            store,
            ExportPackageInput {
                selector: selector.map(str::to_string),
                published_at: Some(AT.to_string()),
                publisher: None,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn parsed(e: &PackageBundleExport) -> Value {
        serde_json::from_str(&e.text).unwrap()
    }

    fn code(err: RepositoryError) -> (&'static str, String) {
        match err {
            RepositoryError::InvalidPackageBundle { code, message } => (code, message),
            other => panic!("expected InvalidPackageBundle, got {other:?}"),
        }
    }

    /// Write one definition into a boundary the way install does.
    fn put(store: &FileStore, selector: Option<&str>, kind: DefinitionKind, v: Value) -> String {
        let prefix = selector.unwrap_or("package");
        let name = definition_name(kind, &v).unwrap();
        let rel = definition_rel_path(kind, &name, v["id"].as_str().unwrap());
        store
            .ensure_instance_dir(&format!("{prefix}/{}", rel.rsplit_once('/').unwrap().0))
            .unwrap();
        store
            .save_instance_json(&format!("{prefix}/{rel}"), &v)
            .unwrap();
        store
            .add_definition_to_boundary(&selector.map(str::to_string), kind, &rel)
            .unwrap();
        rel
    }

    fn sub_package(store: &FileStore, path: &str, id: &str) {
        create_package(
            store,
            CreatePackageInput {
                id: id.to_string(),
                namespace: "com.test.export".to_string(),
                name: path.rsplit('/').next().unwrap().to_string(),
                version: "0.1.0".to_string(),
                boundary_path: Some(path.to_string()),
            },
        )
        .unwrap();
    }

    fn field(id: &str, name: &str) -> Value {
        json!({"id": id, "namespace": "com.test.export", "name": name, "version": 1,
            "description": "d", "fieldType": {"datatype": "string"},
            "aiGuidance": {"purpose": "p"}, "createdAt": "2026-01-01T00:00:00Z"})
    }

    fn ty(id: &str, field_id: &str) -> Value {
        json!({"id": id, "namespace": "com.test.export", "name": "t", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z",
            "fields": [{"fieldId": field_id, "order": 0, "required": false}]})
    }

    #[test]
    fn export_whole_package_preserves_package_identity() {
        let (_t, store) = with_fixture();
        let e = export(&store, Some(FIXTURE));
        let b = parsed(&e);
        assert_eq!(b["packageId"], "9a1b0c2d-1111-4aaa-8bbb-000000000001");
        assert_eq!(b["packageNamespace"], "com.example.install");
        assert_eq!(b["packageName"], "install-fixture");
        assert_eq!(b["packageVersion"], "1.0.0");
        assert_eq!(b["mode"], "bundled");
        assert_eq!(e.summary.package_id, "9a1b0c2d-1111-4aaa-8bbb-000000000001");
    }

    #[test]
    fn export_preserves_definition_identity_verbatim() {
        let (_t, store) = with_fixture();
        let b = parsed(&export(&store, Some(FIXTURE)));
        let boundary = store
            .load_package_boundary(&Some(FIXTURE.to_string()))
            .unwrap();
        let stored = load_boundary_definitions(&store, &boundary).unwrap();
        assert_eq!(stored.len(), 11);
        for d in stored {
            let items = b[definition_kind_key(d.kind)].as_array().unwrap();
            let out = items.iter().find(|i| i["id"] == d.value["id"]).unwrap();
            assert_eq!(out, &crate::srsj::canonicalize(d.value, false));
        }
    }

    #[test]
    fn export_is_byte_deterministic() {
        let (_t, store) = with_fixture();
        let a = export(&store, Some(FIXTURE));
        let b = export(&store, Some(FIXTURE));
        assert_eq!(a.text, b.text);
        assert_eq!(a.summary.sha256, b.summary.sha256);
    }

    #[test]
    fn export_sha256_is_prefixed_hex_of_text() {
        let (_t, store) = with_fixture();
        let e = export(&store, Some(FIXTURE));
        let want = format!("sha256:{}", hex::encode(Sha256::digest(e.text.as_bytes())));
        assert_eq!(e.summary.sha256, want);
        let hex_part = e.summary.sha256.strip_prefix("sha256:").unwrap();
        assert_eq!(hex_part.len(), 64);
        assert!(hex_part
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(e.summary.byte_length, e.text.len());
        assert!(e.text.ends_with("}\n"));
    }

    fn assert_sorted(v: &Value) {
        match v {
            Value::Object(o) => {
                let keys: Vec<&String> = o.keys().collect();
                let mut sorted = keys.clone();
                sorted.sort();
                assert_eq!(keys, sorted);
                o.values().for_each(assert_sorted);
            }
            Value::Array(a) => a.iter().for_each(assert_sorted),
            _ => {}
        }
    }

    #[test]
    fn export_keys_are_sorted() {
        let (_t, store) = with_fixture();
        assert_sorted(&parsed(&export(&store, Some(FIXTURE))));
    }

    #[test]
    fn export_inlines_cross_boundary_references() {
        let (_t, store) = fresh();
        let fid = "f1e1d000-0000-4000-8000-000000000001";
        put(&store, None, DefinitionKind::Field, field(fid, "shared"));
        sub_package(&store, "packages/b", "5e000000-0000-4000-8000-000000000002");
        put(
            &store,
            Some("packages/b"),
            DefinitionKind::Type,
            ty("7e000000-0000-4000-8000-000000000001", fid),
        );
        let e = export(&store, Some("packages/b"));
        assert_eq!(e.summary.inlined, vec![fid.to_string()]);
        assert_eq!(e.summary.definition_count, 2);
        let b = parsed(&e);
        assert_eq!(b["fields"][0]["id"], fid);
        assert_eq!(
            b["dependencyRefs"],
            json!([{"id": fid, "namespace": "com.test.export", "name": "shared",
                "version": 1, "definitionType": "field"}])
        );
        assert_eq!(e.summary.dependency_ref_count, 1);
    }

    /// A boundary the store lists but whose package.json cannot be loaded is refused,
    /// never exported as an empty bundle. (FileStore never lists such a boundary, so
    /// this drives MemoryStore, whose boundary map and file map are separate.)
    #[test]
    fn export_refuses_unreadable_target_index() {
        let store = crate::store::memory::MemoryStore::default();
        create_package(
            &store,
            CreatePackageInput {
                id: "5e000000-0000-4000-8000-000000000008".to_string(),
                namespace: "com.test.export".to_string(),
                name: "x".to_string(),
                version: "0.1.0".to_string(),
                boundary_path: Some("packages/x".to_string()),
            },
        )
        .unwrap();
        store
            .delete_instance_file("packages/x/package.json")
            .unwrap();
        assert!(store
            .load_package_boundary(&Some("packages/x".to_string()))
            .is_ok());
        let err = export_package_bundle(
            &store,
            ExportPackageInput {
                selector: Some("packages/x".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        let (c, msg) = code(err);
        assert_eq!(c, "bundle-boundary-unreadable");
        assert!(msg.contains("packages/x/package.json"), "{msg}");
    }

    #[test]
    fn export_primary_package_when_selector_absent() {
        let (_t, store) = with_fixture();
        let e = export(&store, None);
        assert_eq!(e.summary.package_id, "export-test-primary");
        assert_eq!(e.summary.definition_count, 0);
    }

    #[test]
    fn export_unknown_selector_is_package_not_found() {
        let (_t, store) = fresh();
        let err = export_package_bundle(
            &store,
            ExportPackageInput {
                selector: Some("packages/nope".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::PackageNotFound { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn export_rejects_invalid_published_at() {
        let (_t, store) = fresh();
        let err = export_package_bundle(
            &store,
            ExportPackageInput {
                published_at: Some("yesterday".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert_eq!(code(err).0, "bundle-published-at-invalid");
    }

    #[test]
    fn export_refuses_legacy_package_dependencies() {
        let (_t, store) = fresh();
        sub_package(&store, "packages/d", "5e000000-0000-4000-8000-000000000004");
        let sel = Some("packages/d".to_string());
        let mut boundary = store.load_package_boundary(&sel).unwrap();
        boundary.package_dependencies = Some(vec![
            json!({"namespace": "x", "name": "y", "version": "1.0.0"}),
        ]);
        store.save_package_boundary_metadata(&boundary).unwrap();
        let err = export_package_bundle(
            &store,
            ExportPackageInput {
                selector: sel,
                published_at: Some(AT.to_string()),
                publisher: None,
                ..Default::default()
            },
        )
        .unwrap_err();
        let (c, msg) = code(err);
        assert_eq!(c, "bundle-schema-invalid");
        assert!(msg.contains("--repair-legacy"), "{msg}");
    }

    #[test]
    fn export_output_validates_against_package_bundle_schema() {
        let (_t, store) = with_fixture();
        let e = export_package_bundle(
            &store,
            ExportPackageInput {
                selector: Some(FIXTURE.to_string()),
                published_at: None,
                publisher: Some("Example Org".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let b = parsed(&e);
        assert_eq!(b["publisher"], "Example Org");
        SchemaRegistry::global()
            .validate_by_id(PACKAGE_BUNDLE_SCHEMA_ID, &b)
            .unwrap();
    }

    #[test]
    fn export_missing_listed_definition_file_is_hard_error() {
        let (_t, store) = fresh();
        sub_package(&store, "packages/m", "5e000000-0000-4000-8000-000000000005");
        store
            .add_definition_to_boundary(
                &Some("packages/m".to_string()),
                DefinitionKind::Field,
                "fields/ghost-00000000.json",
            )
            .unwrap();
        let err = export_package_bundle(
            &store,
            ExportPackageInput {
                selector: Some("packages/m".to_string()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("packages/m/fields/ghost-00000000.json"),
            "{err}"
        );
    }

    #[test]
    fn export_input_json_maps_to_export_package_input() {
        let i: ExportPackageInput =
            serde_json::from_str(r#"{"selector":null,"publishedAt":"2026-10-03T00:00:00Z"}"#)
                .unwrap();
        assert_eq!(i.selector, None);
        assert_eq!(i.published_at.as_deref(), Some(AT));
        assert_eq!(i.publisher, None);
        let d: ExportPackageInput = serde_json::from_str("{}").unwrap();
        assert!(d.selector.is_none() && d.published_at.is_none());
    }

    // -----------------------------------------------------------------------
    // RFC-003 Revision 10 (srs-rust#1212)
    // -----------------------------------------------------------------------

    const T: &str = "packages/t";
    const B: &str = "packages/b";

    fn try_export(
        store: &FileStore,
        selector: &str,
        mode: BundleMode,
    ) -> Result<PackageBundleExport, RepositoryError> {
        export_package_bundle(
            store,
            ExportPackageInput {
                selector: Some(selector.to_string()),
                published_at: Some(AT.to_string()),
                mode,
                ..Default::default()
            },
        )
    }

    /// Write a definition at an explicit path (two versions of one id share a name).
    fn put_at(store: &FileStore, selector: &str, kind: DefinitionKind, rel: &str, v: Value) {
        store
            .ensure_instance_dir(&format!("{selector}/{}", rel.rsplit_once('/').unwrap().0))
            .unwrap();
        store
            .save_instance_json(&format!("{selector}/{rel}"), &v)
            .unwrap();
        store
            .add_definition_to_boundary(&Some(selector.to_string()), kind, rel)
            .unwrap();
    }

    /// Target `packages/t` plus a second boundary `packages/b`.
    fn two_boundaries() -> (TempDir, FileStore) {
        let (tmp, store) = fresh();
        sub_package(&store, T, "5e000000-0000-4000-8000-0000000000a1");
        sub_package(&store, B, "5e000000-0000-4000-8000-0000000000b1");
        (tmp, store)
    }

    fn field_v(id: &str, name: &str, version: u64) -> Value {
        let mut f = field(id, name);
        f["version"] = json!(version);
        f
    }

    fn view(id: &str, field_ids: &[&str]) -> Value {
        let fvs: Vec<Value> = field_ids
            .iter()
            .enumerate()
            .map(|(i, f)| json!({"fieldId": f, "order": i, "required": false, "visible": true}))
            .collect();
        json!({"$schema": "https://srs.semanticops.com/schema/2.0/view.json",
            "id": id, "namespace": "com.test.export", "name": "v", "version": 1,
            "description": "d", "createdAt": "2026-01-01T00:00:00Z", "fieldViews": fvs})
    }

    fn ids_of(b: &Value, key: &str) -> Vec<(String, u64)> {
        b[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|d| {
                        (
                            d["id"].as_str().unwrap().to_string(),
                            d["version"].as_u64().unwrap(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn dep_refs(b: &Value) -> Vec<(String, u64, String)> {
        b["dependencyRefs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["id"].as_str().unwrap().to_string(),
                    r["version"].as_u64().unwrap(),
                    r["definitionType"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    const FID: &str = "f1e1d000-0000-4000-8000-0000000000f1";
    const TID: &str = "7e000000-0000-4000-8000-0000000000e1";

    #[test]
    fn export_follows_pinned_reference_to_exact_version() {
        let (_t, store) = two_boundaries();
        let fid = "f1e1d000-0000-4000-8000-0000000000f2";
        put(&store, Some(B), DefinitionKind::Field, field(fid, "leaf"));
        let mut t1 = ty(TID, fid);
        t1["name"] = json!("range");
        let mut t2 = t1.clone();
        t2["version"] = json!(2);
        put_at(&store, B, DefinitionKind::Type, "types/range-v1.json", t1);
        put_at(&store, B, DefinitionKind::Type, "types/range-v2.json", t2);
        let mut f = field(FID, "composite");
        f["fieldType"] = json!({"datatype": "ref", "mode": "inline",
            "rangeType": {"typeId": TID, "typeVersion": 2}});
        put(&store, Some(T), DefinitionKind::Field, f);
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(ids_of(&b, "types"), vec![(TID.to_string(), 2)]);
        assert!(dep_refs(&b).contains(&(TID.to_string(), 2, "type".to_string())));
        assert!(!dep_refs(&b).contains(&(TID.to_string(), 1, "type".to_string())));
    }

    #[test]
    fn export_follows_lineage_reference_to_every_version() {
        let (_t, store) = two_boundaries();
        put_at(
            &store,
            B,
            DefinitionKind::Field,
            "fields/f-v1.json",
            field_v(FID, "f", 1),
        );
        put_at(
            &store,
            B,
            DefinitionKind::Field,
            "fields/f-v2.json",
            field_v(FID, "f", 2),
        );
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(
            ids_of(&b, "fields"),
            vec![(FID.to_string(), 1), (FID.to_string(), 2)]
        );
        let fields: Vec<_> = dep_refs(&b)
            .into_iter()
            .filter(|r| r.2 == "field")
            .collect();
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn export_does_not_follow_keyed_reference() {
        let (_t, store) = two_boundaries();
        put(
            &store,
            Some(B),
            DefinitionKind::RelationType,
            json!({"$schema": "https://srs.semanticops.com/schema/2.0/relation-type.json",
                "id": "9e000000-0000-4000-8000-0000000000c1", "namespace": "com.test.export",
                "key": "com.test.export/linked", "label": "Linked", "version": 1,
                "description": "d", "createdAt": "2026-01-01T00:00:00Z"}),
        );
        put(&store, Some(T), DefinitionKind::Field, field(FID, "f"));
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let pinned = json!({"typeId": TID, "typeVersion": 1});
        put(
            &store,
            Some(T),
            DefinitionKind::Blueprint,
            json!({"id": "b1000000-0000-4000-8000-000000000001", "namespace": "com.test.export",
                "name": "bp", "version": 1, "description": "d", "createdAt": "2026-01-01T00:00:00Z",
                "rootTypes": [pinned],
                "structure": [{"relationType": "com.test.export/linked",
                    "sourceType": pinned, "targetType": pinned}]}),
        );
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert!(b.get("relationTypes").is_none());
        assert!(dep_refs(&b).iter().all(|r| r.2 != "relation-type"));
    }

    #[test]
    fn export_does_not_follow_locator_reference() {
        let (_t, store) = two_boundaries();
        let theme = "7e3e0000-0000-4000-8000-000000000001";
        put(
            &store,
            Some(B),
            DefinitionKind::Theme,
            json!({"$schema": "https://srs.semanticops.com/schema/2.0/theme.json",
                "id": theme, "namespace": "com.test.export", "name": "plain", "version": 1,
                "description": "d", "createdAt": "2026-01-01T00:00:00Z", "targets": ["markdown"]}),
        );
        put(
            &store,
            Some(T),
            DefinitionKind::Composition,
            json!({"$schema": "https://srs.semanticops.com/schema/2.0/composition.json",
                "id": "c0000000-0000-4000-8000-000000000001", "namespace": "com.test.export",
                "name": "c", "version": 1, "description": "d", "createdAt": "2026-01-01T00:00:00Z",
                "themeRef": {"mode": "bundled", "themeId": theme}, "sections": [{"sectionId": "s", "title": "S", "order": 0, "emptyBehavior": "hide",
                    "source": {"type": "discovery-query",
                        "query": {"typeNamespace": "com.test.export", "typeName": "t"}}}],
                "exportConfig": {"format": "markdown"}}),
        );
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert!(b.get("themes").is_none());
        assert_eq!(dep_refs(&b), vec![]);
    }

    /// The id-scan false positive: provenance is not a reference.
    #[test]
    fn export_does_not_follow_lineage_provenance() {
        let (_t, store) = two_boundaries();
        let other = "f1e1d000-0000-4000-8000-0000000000f3";
        put(
            &store,
            Some(B),
            DefinitionKind::Field,
            field(other, "origin"),
        );
        let mut f = field(FID, "fork");
        f["lineage"] = json!({"sourceDefinitionId": other, "sourceVersion": 1});
        put(&store, Some(T), DefinitionKind::Field, f);
        let e = try_export(&store, T, BundleMode::Bundled).unwrap();
        assert_eq!(ids_of(&parsed(&e), "fields"), vec![(FID.to_string(), 1)]);
        assert!(e.summary.inlined.is_empty());
    }

    /// Target Type extends a base Type in `packages/b`, whose Field also lives
    /// there: Type -> Type -> Field (a View holds no Type site, F2).
    fn transitive_setup() -> (TempDir, FileStore) {
        let (tmp, store) = two_boundaries();
        let base = "7e000000-0000-4000-8000-0000000000e2";
        put(&store, Some(B), DefinitionKind::Field, field(FID, "deep"));
        let mut bt = ty(base, FID);
        bt["name"] = json!("base");
        put(&store, Some(B), DefinitionKind::Type, bt);
        let own_field = "f1e1d000-0000-4000-8000-0000000000f4";
        put(
            &store,
            Some(T),
            DefinitionKind::Field,
            field(own_field, "own"),
        );
        let mut t = ty(TID, own_field);
        t["extendsTypeId"] = json!(base);
        t["extendsTypeVersion"] = json!(1);
        put(&store, Some(T), DefinitionKind::Type, t);
        (tmp, store)
    }

    #[test]
    fn export_follows_references_transitively() {
        let (_t, store) = transitive_setup();
        let e = try_export(&store, T, BundleMode::Bundled).unwrap();
        let b = parsed(&e);
        assert!(ids_of(&b, "fields").contains(&(FID.to_string(), 1)));
        assert_eq!(ids_of(&b, "types").len(), 2);
        assert_eq!(
            e.summary.inlined,
            vec![
                "7e000000-0000-4000-8000-0000000000e2".to_string(),
                FID.to_string()
            ]
        );
        assert!(dep_refs(&b).contains(&(FID.to_string(), 1, "field".to_string())));
    }

    #[test]
    fn export_lists_own_referenced_definitions() {
        let (_t, store) = two_boundaries();
        put(&store, Some(T), DefinitionKind::Field, field(FID, "f"));
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(
            dep_refs(&b),
            vec![(FID.to_string(), 1, "field".to_string())]
        );
        assert_eq!(b["dependencyRefs"][0]["namespace"], "com.test.export");
        assert_eq!(b["dependencyRefs"][0]["name"], "f");
    }

    #[test]
    fn export_unreferenced_own_definition_is_carried_not_listed() {
        let (_t, store) = two_boundaries();
        put(&store, Some(T), DefinitionKind::Field, field(FID, "lonely"));
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(ids_of(&b, "fields"), vec![(FID.to_string(), 1)]);
        assert_eq!(b["dependencyRefs"], json!([]));
    }

    #[test]
    fn export_dependency_refs_sorted_by_id_version_definition_type() {
        let (_t, store) = two_boundaries();
        let a = "0a000000-0000-4000-8000-000000000001";
        let z = "fa000000-0000-4000-8000-000000000001";
        put(&store, Some(T), DefinitionKind::Field, field(z, "z"));
        put_at(
            &store,
            T,
            DefinitionKind::Field,
            "fields/a-v10.json",
            field_v(a, "a", 10),
        );
        put_at(
            &store,
            T,
            DefinitionKind::Field,
            "fields/a-v2.json",
            field_v(a, "a", 2),
        );
        let mut t = ty(TID, z);
        t["fields"] = json!([{"fieldId": z, "order": 0, "required": false},
            {"fieldId": a, "order": 1, "required": false}]);
        put(&store, Some(T), DefinitionKind::Type, t);
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(
            dep_refs(&b),
            vec![
                (a.to_string(), 2, "field".to_string()),
                (a.to_string(), 10, "field".to_string()),
                (z.to_string(), 1, "field".to_string())
            ]
        );
    }

    fn core_field() -> &'static srs_core::types::field::Field {
        &crate::core_package::core_package().fields[0]
    }

    /// A boundary copy of a core field: same id, version, namespace and name.
    fn core_copy(name: &str) -> Value {
        let c = core_field();
        let mut f = field_v(&c.id, name, u64::from(c.version));
        f["namespace"] = json!(c.namespace);
        f
    }

    #[test]
    fn export_lists_core_definitions_without_carrying_them() {
        let (_t, store) = fresh();
        let c = core_field();
        sub_package(&store, "packages/c", "5e000000-0000-4000-8000-000000000003");
        // A view, not a type: the [R13] catalog resolves a Type's FieldAssignment only
        // against the repository's own definition files, never the embedded core
        // (srs-rust#1208), while views resolve against the merged package (ADR-025).
        put(
            &store,
            Some("packages/c"),
            DefinitionKind::View,
            view("7e000000-0000-4000-8000-000000000002", &[&c.id]),
        );
        let e = export(&store, Some("packages/c"));
        assert!(e.summary.inlined.is_empty());
        let b = parsed(&e);
        assert_eq!(b["fields"], json!([]));
        assert_eq!(
            b["dependencyRefs"],
            json!([{"id": c.id, "namespace": c.namespace, "name": c.name,
                "version": c.version, "definitionType": "field"}])
        );

        let (_t2, dest) = fresh();
        let r = crate::package_install_service::install_package_bundle_bytes(
            &dest,
            e.text.as_bytes(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(r.installed, 1);
        let report = crate::validation::validate_repository(&dest).unwrap();
        assert_eq!(report.summary.errors, 0, "{:?}", report.diagnostics);
    }

    #[test]
    fn export_carries_a_core_definition_the_source_package_lists() {
        let (_t, store) = two_boundaries();
        let c = core_field();
        put(&store, Some(T), DefinitionKind::Field, core_copy(&c.name));
        put(
            &store,
            Some(T),
            DefinitionKind::View,
            view("7e000000-0000-4000-8000-0000000000e3", &[&c.id]),
        );
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(
            ids_of(&b, "fields"),
            vec![(c.id.clone(), u64::from(c.version))]
        );
        assert_eq!(
            dep_refs(&b),
            vec![(c.id.clone(), u64::from(c.version), "field".to_string())]
        );
    }

    /// The core exception is per `(id, version)`: a boundary's other version of a
    /// core id is an ordinary definition and is carried in bundled mode.
    #[test]
    fn export_carries_a_non_core_version_of_a_core_id() {
        let (_t, store) = two_boundaries();
        let c = core_field();
        let other = u64::from(c.version) + 1;
        let mut f = core_copy(&c.name);
        f["version"] = json!(other);
        put(&store, Some(B), DefinitionKind::Field, f);
        put(
            &store,
            Some(T),
            DefinitionKind::View,
            view("7e000000-0000-4000-8000-0000000000e7", &[&c.id]),
        );
        let e = try_export(&store, T, BundleMode::Bundled).unwrap();
        let b = parsed(&e);
        assert_eq!(ids_of(&b, "fields"), vec![(c.id.clone(), other)]);
        assert_eq!(e.summary.inlined, vec![c.id.clone()]);
        let refs = dep_refs(&b);
        assert!(refs.contains(&(c.id.clone(), other, "field".to_string())));
        assert!(refs.contains(&(c.id.clone(), u64::from(c.version), "field".to_string())));
    }

    #[test]
    fn export_boundary_copy_of_core_field_is_not_a_conflict() {
        let (_t, store) = two_boundaries();
        let c = core_field();
        put(&store, Some(B), DefinitionKind::Field, core_copy(&c.name));
        put(
            &store,
            Some(T),
            DefinitionKind::View,
            view("7e000000-0000-4000-8000-0000000000e4", &[&c.id]),
        );
        let e = try_export(&store, T, BundleMode::Bundled).unwrap();
        let b = parsed(&e);
        assert_eq!(b["fields"], json!([]));
        assert_eq!(dep_refs(&b).len(), 1);
        assert!(e.summary.inlined.is_empty());
    }

    #[test]
    fn export_core_id_with_different_name_is_identity_conflict() {
        let (_t, store) = two_boundaries();
        let c = core_field();
        put(
            &store,
            Some(B),
            DefinitionKind::Field,
            core_copy("not-the-core-name"),
        );
        put(
            &store,
            Some(T),
            DefinitionKind::View,
            view("7e000000-0000-4000-8000-0000000000e5", &[&c.id]),
        );
        let (code, msg) = code(try_export(&store, T, BundleMode::Bundled).unwrap_err());
        assert_eq!(code, "bundle-identity-conflict");
        assert!(msg.contains(&c.id) && msg.contains("core"), "{msg}");
    }

    #[test]
    fn export_refuses_unresolved_lineage_reference() {
        let (_t, store) = two_boundaries();
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let (code, msg) = code(try_export(&store, T, BundleMode::Bundled).unwrap_err());
        assert_eq!(code, "bundle-reference-unresolved");
        assert!(
            msg.contains(TID) && msg.contains("/fields/0/fieldId") && msg.contains(FID),
            "{msg}"
        );
    }

    #[test]
    fn export_refuses_unresolved_pinned_version() {
        let (_t, store) = two_boundaries();
        put(
            &store,
            Some(B),
            DefinitionKind::Field,
            field("f1e1d000-0000-4000-8000-0000000000f5", "leaf"),
        );
        put(
            &store,
            Some(B),
            DefinitionKind::Type,
            ty(TID, "f1e1d000-0000-4000-8000-0000000000f5"),
        );
        let mut f = field(FID, "composite");
        f["fieldType"] = json!({"datatype": "ref", "mode": "inline",
            "rangeType": {"typeId": TID, "typeVersion": 3}});
        put(&store, Some(T), DefinitionKind::Field, f);
        let (code, msg) = code(try_export(&store, T, BundleMode::Bundled).unwrap_err());
        assert_eq!(code, "bundle-reference-unresolved");
        assert!(msg.contains("version 3"), "{msg}");
    }

    #[test]
    fn export_refuses_identity_conflict() {
        let (_t, store) = fresh();
        let mut primary = field(FID, "dup");
        primary["description"] = json!("from primary");
        put(&store, None, DefinitionKind::Field, primary);
        sub_package(&store, B, "5e000000-0000-4000-8000-000000000006");
        let mut other = field(FID, "dup");
        other["description"] = json!("from b");
        put(&store, Some(B), DefinitionKind::Field, other);
        sub_package(&store, T, "5e000000-0000-4000-8000-000000000007");
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let (code, msg) = code(try_export(&store, T, BundleMode::Bundled).unwrap_err());
        assert_eq!(code, "bundle-identity-conflict");
        assert!(
            msg.contains(FID) && msg.contains("package and packages/b"),
            "{msg}"
        );
    }

    #[test]
    fn export_refuses_own_definition_conflicting_with_another_boundary() {
        let (_t, store) = two_boundaries();
        put(&store, Some(T), DefinitionKind::Field, field(FID, "mine"));
        let mut other = field(FID, "mine");
        other["description"] = json!("different");
        put(&store, Some(B), DefinitionKind::Field, other);
        let (code, _) = code(try_export(&store, T, BundleMode::Bundled).unwrap_err());
        assert_eq!(code, "bundle-identity-conflict");
    }

    #[test]
    fn export_identity_duplicate_with_equal_content_is_one_definition() {
        let (_t, store) = two_boundaries();
        put(&store, None, DefinitionKind::Field, field(FID, "same"));
        put(&store, Some(B), DefinitionKind::Field, field(FID, "same"));
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let b = parsed(&try_export(&store, T, BundleMode::Bundled).unwrap());
        assert_eq!(ids_of(&b, "fields"), vec![(FID.to_string(), 1)]);
    }

    #[test]
    fn export_refuses_carried_definition_failing_its_schema() {
        let (_t, store) = two_boundaries();
        let mut bad = field(FID, "bad");
        bad["bogus"] = json!(1);
        put(&store, Some(B), DefinitionKind::Field, bad);
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let (code, msg) = code(try_export(&store, T, BundleMode::Bundled).unwrap_err());
        assert_eq!(code, "bundle-definition-invalid");
        assert!(msg.contains("field") && msg.contains(FID), "{msg}");
    }

    #[test]
    fn export_standalone_carries_only_own_definitions() {
        let (_t, store) = transitive_setup();
        let e = try_export(&store, T, BundleMode::Standalone).unwrap();
        let b = parsed(&e);
        assert_eq!(b["mode"], "standalone");
        assert_eq!(e.summary.mode, BundleMode::Standalone);
        assert_eq!(ids_of(&b, "types"), vec![(TID.to_string(), 1)]);
        assert_eq!(
            ids_of(&b, "fields"),
            vec![("f1e1d000-0000-4000-8000-0000000000f4".to_string(), 1)]
        );
        assert!(e.summary.inlined.is_empty());
    }

    #[test]
    fn export_standalone_lists_every_reached_non_own_and_core_definition() {
        let (_t, store) = two_boundaries();
        let c = core_field();
        let base = "7e000000-0000-4000-8000-0000000000e2";
        // The non-own base Type's own Field reference is not followed (F4), so
        // its Field need not even exist.
        let mut bt = ty(base, "f1e1d000-0000-4000-8000-0000000000ff");
        bt["name"] = json!("base");
        put(&store, Some(B), DefinitionKind::Type, bt);
        put(&store, Some(B), DefinitionKind::Field, field(FID, "cross"));
        let mut t = ty(TID, FID);
        t["extendsTypeId"] = json!(base);
        t["extendsTypeVersion"] = json!(1);
        put(&store, Some(T), DefinitionKind::Type, t);
        put(
            &store,
            Some(T),
            DefinitionKind::View,
            view("7e000000-0000-4000-8000-0000000000e6", &[&c.id]),
        );
        let b = parsed(&try_export(&store, T, BundleMode::Standalone).unwrap());
        let refs = dep_refs(&b);
        assert!(refs.contains(&(FID.to_string(), 1, "field".to_string())));
        assert!(refs.contains(&(base.to_string(), 1, "type".to_string())));
        assert!(refs.contains(&(c.id.clone(), u64::from(c.version), "field".to_string())));
        assert_eq!(refs.len(), 3);
        assert_eq!(b["fields"], json!([]));
    }

    #[test]
    fn export_standalone_refuses_unresolved_reference() {
        let (_t, store) = two_boundaries();
        put(&store, Some(T), DefinitionKind::Type, ty(TID, FID));
        let (code, _) = code(try_export(&store, T, BundleMode::Standalone).unwrap_err());
        assert_eq!(code, "bundle-reference-unresolved");
    }

    #[test]
    fn export_standalone_is_byte_deterministic() {
        let (_t, store) = transitive_setup();
        let a = try_export(&store, T, BundleMode::Standalone).unwrap();
        let b = try_export(&store, T, BundleMode::Standalone).unwrap();
        assert_eq!(a.text, b.text);
    }

    #[test]
    fn export_mode_defaults_to_bundled() {
        let i: ExportPackageInput = serde_json::from_str("{}").unwrap();
        assert_eq!(i.mode, BundleMode::Bundled);
        let (_t, store) = with_fixture();
        assert_eq!(parsed(&export(&store, Some(FIXTURE)))["mode"], "bundled");
    }

    #[test]
    fn export_input_json_maps_mode() {
        let i: ExportPackageInput =
            serde_json::from_str(r#"{"mode":"standalone","homepage":"https://x"}"#).unwrap();
        assert_eq!(i.mode, BundleMode::Standalone);
        assert_eq!(i.homepage.as_deref(), Some("https://x"));
        assert!(serde_json::from_str::<ExportPackageInput>(r#"{"mode":"bogus"}"#).is_err());
        assert_eq!(
            "standalone".parse::<BundleMode>(),
            Ok(BundleMode::Standalone)
        );
        assert!("bogus".parse::<BundleMode>().is_err());
    }

    #[test]
    fn export_from_repository_below_reader_floor_writes_bundle_its_reader_refuses() {
        let (_t, store) = with_fixture();
        assert!(export(&store, Some(FIXTURE)).summary.notes.is_empty());
        crate::field_type_migration_service::stamp_data_model_revision(&store, 6).unwrap();
        let e = export(&store, Some(FIXTURE));
        assert_eq!(parsed(&e)["dataModelRevision"], 6);
        assert_eq!(e.summary.notes.len(), 1);
        assert!(
            e.summary.notes[0].starts_with("bundle-below-reader-floor"),
            "{:?}",
            e.summary.notes
        );
        let (code, _) = code(read_package_bundle(e.text.as_bytes()).unwrap_err());
        assert_eq!(code, "bundle-migration-step-missing");
    }

    #[test]
    fn read_bundle_restamp_writes_each_step_target() {
        // One step at a time: 7 -> 8 is the RFC-043 transform, 8 -> 9 the re-stamp.
        let mut v = json!({"dataModelRevision": 7, "compositions": []});
        assert!(bring_forward(&mut v, 7).unwrap().is_empty());
        assert_eq!(v["dataModelRevision"], CURRENT_DATA_MODEL_REVISION);
        let mut v = json!({"dataModelRevision": 8});
        bring_forward(&mut v, 8).unwrap();
        assert_eq!(v["dataModelRevision"], 9);
        // A refused step leaves the stamp at the last step reached.
        let mut v = json!({"dataModelRevision": 6});
        assert!(bring_forward(&mut v, 6).is_err());
        assert_eq!(v["dataModelRevision"], 6);
        // Stamp after the transform step alone is its target (8), not current.
        let mut v = json!({"dataModelRevision": 7});
        let (_, step) = revision_step_from(7).unwrap();
        let BundleForm::Transform(f) = step.bundle else {
            panic!("7 -> 8 is a transform")
        };
        f(&mut v).unwrap();
        assert_eq!(v["dataModelRevision"], 8);
    }

    #[test]
    fn export_writes_homepage_when_given() {
        let (_t, store) = with_fixture();
        let e = export_package_bundle(
            &store,
            ExportPackageInput {
                selector: Some(FIXTURE.to_string()),
                published_at: Some(AT.to_string()),
                homepage: Some("https://example.org/pkg".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(parsed(&e)["homepage"], "https://example.org/pkg");
        assert!(parsed(&export(&store, Some(FIXTURE)))
            .get("homepage")
            .is_none());
    }

    #[test]
    fn export_stamps_repository_data_model_revision() {
        let (_t, store) = with_fixture();
        crate::field_type_migration_service::stamp_data_model_revision(&store, 8).unwrap();
        let e = export(&store, Some(FIXTURE));
        assert_eq!(parsed(&e)["dataModelRevision"], 8);
        assert_eq!(e.summary.data_model_revision, 8);
        assert!(e.summary.notes.is_empty());
    }

    #[test]
    fn export_schema_version_is_constant_2_0() {
        let (_t, store) = with_fixture();
        assert_eq!(
            parsed(&export(&store, Some(FIXTURE)))["schemaVersion"],
            "2.0"
        );
    }

    #[test]
    fn export_definitions_sorted_by_id_then_version() {
        let (_t, store) = fresh();
        sub_package(&store, "packages/s", "5e000000-0000-4000-8000-000000000001");
        let z = "ffffffff-0000-4000-8000-000000000002";
        let a = "11111111-0000-4000-8000-000000000001";
        put(
            &store,
            Some("packages/s"),
            DefinitionKind::Field,
            field(z, "zeta"),
        );
        put_at(
            &store,
            "packages/s",
            DefinitionKind::Field,
            "fields/alpha-v10.json",
            field_v(a, "alpha", 10),
        );
        put_at(
            &store,
            "packages/s",
            DefinitionKind::Field,
            "fields/alpha-v2.json",
            field_v(a, "alpha", 2),
        );
        let b = parsed(&export(&store, Some("packages/s")));
        assert_eq!(
            ids_of(&b, "fields"),
            vec![(a.to_string(), 2), (a.to_string(), 10), (z.to_string(), 1)]
        );
        assert_eq!(b["types"], json!([]));
        for key in [
            "views",
            "compositions",
            "relationTypes",
            "blueprints",
            "protocols",
            "vocabularies",
            "lifecycles",
            "themes",
        ] {
            assert!(b.get(key).is_none(), "{key} must be omitted when empty");
        }
    }

    #[test]
    fn export_writes_manifest_description() {
        let (_t, store) = with_fixture();
        let path = format!("{FIXTURE}/package.json");
        let mut pkg = store.load_instance_json(&path).unwrap();
        pkg["description"] = json!("An install fixture.");
        store.save_instance_json(&path, &pkg).unwrap();
        assert_eq!(
            parsed(&export(&store, Some(FIXTURE)))["description"],
            "An install fixture."
        );
    }

    #[test]
    fn export_omits_description_when_manifest_has_none() {
        let (_t, store) = with_fixture();
        let path = format!("{FIXTURE}/package.json");
        let mut pkg = store.load_instance_json(&path).unwrap();
        pkg.as_object_mut().unwrap().remove("description");
        store.save_instance_json(&path, &pkg).unwrap();
        assert!(parsed(&export(&store, Some(FIXTURE)))
            .get("description")
            .is_none());
    }
}
