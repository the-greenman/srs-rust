//! RFC-026 container slice export (ADR-051).
//!
//! A slice is one container exported as a standalone, valid `.srs`: the
//! container-membership closure (Change C, as amended by RFC-038 [R25] and
//! RFC-043) over the tree-authoritative catalog, cut edges recorded in
//! `slice.externalRelationRefs` (Change D), and the manifest rewritten to make
//! the boundary container the repository root. It is a dedicated filter over
//! the faithful store→tree enumeration ([`crate::archive::tree_entries`]),
//! zipped by the same deterministic writer as `archive_pack` — never a mode of
//! `archive_pack` itself.
//!
//! Owner rulings (srs-rust#631, 2026-10-04; RFC-026 Revision 9 pending):
//! - **D1 (P3):** every package the slice's records use is carried whole and
//!   unchanged, at its source path — never pruned or merged, so each carried
//!   package keeps a true identity. "Use" is followed at package granularity:
//!   the packages holding the included records' Types and the included
//!   relations' RelationTypes, closed over every PINNED/LINEAGE reference
//!   (`reference_sites`) and every RFC-044 `packageDependencies` entry of a
//!   carried package. The primary `package/` is always carried.
//! - **D2:** a boundary whose identity entry is not a depth-0 entry with no
//!   descendants is refused (`slice-root-identity-invalid`); the outline is
//!   never rewritten to make it fit.
//! - **D3:** a sub-container is carried only when it has at least one entry and
//!   every entry id is included (Change C step 6 without its vacuous case).
//! - Source documents referenced by included **relations** are carried too;
//!   `childContainerIds` are copied as-is; the manifest keeps every source
//!   property except the RFC-required rewrites (and the `packageRefs` of
//!   packages not carried, which would otherwise name absent paths).

use crate::archive::{carrying_file, pack_tree, tree_entries};
use crate::catalog::{self, INSTANCE_ROOT_NAMES, ROOT_CONTAINER_LOCATOR};
use crate::error::RepositoryError;
use crate::package_install_service::load_boundary_definitions;
use crate::package_types::DefinitionKind;
use crate::reference_sites::followed_references;
use crate::store::RepositoryStore;
use serde_json::{json, Value};
use srs_core::arrangement::{check_entries, retain_promoting, CODE_IDENTITY};
use srs_core::types::container::ContainerEntry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Refusal code for a boundary whose identity entry cannot be a root identity (D2).
pub const CODE_ROOT_IDENTITY_INVALID: &str = "slice-root-identity-invalid";

#[derive(Debug, Clone, Default)]
pub struct ExportSliceInput {
    pub container_id: String,
    /// RFC 3339; defaults to now. An override makes the output byte-deterministic.
    pub exported_at: Option<String>,
    /// The slice's own `repositoryId` (RFC-026 [R3]); defaults to a fresh UUID.
    pub repository_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceExportSummary {
    pub container_id: String,
    pub slice_repository_id: String,
    pub origin_repository_id: String,
    pub exported_at: String,
    pub instance_count: usize,
    pub relation_count: usize,
    /// Containers in the slice, the boundary (now the root) included.
    pub container_count: usize,
    pub source_document_count: usize,
    pub package_count: usize,
    pub external_relation_ref_count: usize,
}

#[derive(Debug, Clone)]
pub struct SliceExport {
    pub bytes: Vec<u8>,
    pub summary: SliceExportSummary,
}

fn refuse(code: &'static str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::SliceRefused {
        code,
        message: message.into(),
    }
}

fn parse(tree: &BTreeMap<String, Vec<u8>>, path: &str) -> Result<Value, RepositoryError> {
    let bytes = tree
        .get(path)
        .ok_or_else(|| RepositoryError::InvalidArchive {
            message: format!("catalog names '{path}' but the tree holds no such file"),
        })?;
    serde_json::from_slice(bytes).map_err(|e| RepositoryError::InvalidArchive {
        message: format!("invalid {path}: {e}"),
    })
}

/// A JSON file as written: pretty, newline-terminated.
fn pretty(v: &Value) -> Vec<u8> {
    (serde_json::to_string_pretty(v).expect("a Value serializes") + "\n").into_bytes()
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `repository-document` source ids named by a `sourceRefs` array (RFC-017).
fn document_refs(v: &Value) -> impl Iterator<Item = String> + '_ {
    v.get("sourceRefs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| str_of(r, "sourceType") == "repository-document")
        .map(|r| str_of(r, "sourceId").to_string())
}

/// Export one container as an RFC-026 slice archive (`.srs` bytes).
pub fn export_container_slice(
    store: &dyn RepositoryStore,
    input: ExportSliceInput,
) -> Result<SliceExport, RepositoryError> {
    let exported_at = match input.exported_at {
        Some(s) => {
            chrono::DateTime::parse_from_rfc3339(&s).map_err(|e| {
                refuse(
                    "slice-exported-at-invalid",
                    format!("exportedAt '{s}' is not RFC 3339: {e}"),
                )
            })?;
            s
        }
        None => chrono::Utc::now().to_rfc3339(),
    };
    let slice_repository_id = input
        .repository_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // ADR-045: an ordinary operation reads through the checked catalog.
    let cat = store.catalog()?;
    let tree = tree_entries(store)?;
    let mut manifest: Value = parse(&tree, "manifest.json")?;
    let origin_repository_id = str_of(&manifest, "repositoryId").to_string();
    if slice_repository_id == origin_repository_id {
        return Err(refuse(
            "slice-repository-id-reused",
            "a slice's repositoryId must differ from its origin's (RFC-026 [R3])",
        ));
    }

    // --- Containers: id → (path to write at, raw value). -----------------
    let mut containers: Vec<(String, String, Value)> = Vec::new();
    for e in &cat.containers {
        let Some(loc) = e.locator.as_deref() else {
            continue;
        };
        if loc == ROOT_CONTAINER_LOCATOR {
            let v = manifest.get("container").cloned().unwrap_or(Value::Null);
            containers.push((e.id.clone(), format!("containers/{}.json", e.id), v));
        } else {
            let path = carrying_file(store, loc)?;
            let v = parse(&tree, &path)?;
            containers.push((e.id.clone(), path, v));
        }
    }
    let entries_of = |v: &Value| -> Vec<ContainerEntry> {
        v.get("memberInstanceIds")
            .cloned()
            .and_then(|m| serde_json::from_value(m).ok())
            .unwrap_or_default()
    };
    let (_, _, boundary) = containers
        .iter()
        .find(|(id, _, _)| *id == input.container_id)
        .ok_or_else(|| RepositoryError::ContainerNotFound {
            container_id: input.container_id.clone(),
        })?;
    let mut boundary = boundary.clone();

    // --- Instances: the boundary's entry ids that name an instance (step 2;
    // at revision >= 8 the fixpoint adds nothing). RFC-043 [R18]/[R7]: an
    // entry naming no instance is dropped by the promoting removal; the checked
    // catalog already refuses a dangling entry, so today this keeps every one.
    let instances: HashMap<&str, &str> = cat
        .instances
        .iter()
        .filter_map(|e| Some((e.id.as_str(), e.locator.as_deref()?)))
        .collect();
    let (retained, dropped, _) =
        retain_promoting(&entries_of(&boundary), |id| instances.contains_key(id));
    if !dropped.is_empty() {
        boundary["memberInstanceIds"] = serde_json::to_value(&retained).expect("entries serialize");
    }
    let identity = boundary
        .get("identityInstanceId")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(identity) = identity.as_deref() {
        let mut problems: Vec<String> = check_entries(&retained, Some(identity))
            .into_iter()
            .filter(|v| v.code == CODE_IDENTITY)
            .map(|v| v.message)
            .collect();
        if !retained.iter().any(|e| e.instance_id == identity) {
            problems.push("the identity record must be one of the container's entries".into());
        }
        if !problems.is_empty() {
            return Err(refuse(
                CODE_ROOT_IDENTITY_INVALID,
                format!(
                    "container '{}' cannot be a slice root: identity entry '{identity}': {} \
                     (RFC-043; the outline is never rewritten to fit)",
                    input.container_id,
                    problems.join("; ")
                ),
            ));
        }
    }
    let included: BTreeSet<String> = retained.iter().map(|e| e.instance_id.clone()).collect();

    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut doc_ids: BTreeSet<String> = BTreeSet::new();
    let mut type_ids: BTreeSet<String> = BTreeSet::new();
    for id in &included {
        let path = carrying_file(store, instances[id.as_str()])?;
        let v = parse(&tree, &path)?;
        if let Some(t) = v.get("typeId").and_then(Value::as_str) {
            type_ids.insert(t.to_string());
        }
        doc_ids.extend(document_refs(&v));
        out.insert(path.clone(), tree[&path].clone());
    }

    // --- Relations (step 4, Change D). ----------------------------------
    let mut relation_types: BTreeSet<String> = BTreeSet::new();
    let mut external: Vec<(String, Value)> = Vec::new();
    let mut relation_count = 0;
    for e in &cat.relations {
        let Some(loc) = e.locator.as_deref() else {
            continue;
        };
        let path = carrying_file(store, loc)?;
        let v = parse(&tree, &path)?;
        let (s, t) = (
            str_of(&v, "sourceInstanceId"),
            str_of(&v, "targetInstanceId"),
        );
        match (included.contains(s), included.contains(t)) {
            (true, true) => {
                relation_types.insert(str_of(&v, "relationType").to_string());
                doc_ids.extend(document_refs(&v));
                out.insert(path.clone(), tree[&path].clone());
                relation_count += 1;
            }
            (false, false) => {}
            _ => external.push((
                e.id.clone(),
                json!({
                    "relationId": e.id,
                    "sourceInstanceId": s,
                    "targetInstanceId": t,
                    "relationType": str_of(&v, "relationType"),
                }),
            )),
        }
    }
    external.sort_by(|a, b| a.0.cmp(&b.0));

    // --- Sub-containers (step 6 + D3). The boundary itself goes inline. ---
    let mut container_count = 1;
    for (id, path, v) in &containers {
        if *id == input.container_id {
            continue;
        }
        let ids = entries_of(v);
        if ids.is_empty() || !ids.iter().all(|e| included.contains(&e.instance_id)) {
            continue;
        }
        let bytes = match tree.get(path) {
            Some(b) => b.clone(),
            // The source root container is inline in its manifest.
            None => pretty(v),
        };
        out.insert(path.clone(), bytes);
        container_count += 1;
    }

    // --- Source documents (step 5 + relations): sidecar, and content unless
    // tombstoned (absent). --------------------------------------------------
    let src_docs =
        catalog::declared_location(manifest.get("sourceDocumentsPath").and_then(Value::as_str))
            .unwrap_or_else(|| "source-documents".to_string());
    let mut source_document_count = 0;
    for e in cat
        .source_documents
        .iter()
        .filter(|e| doc_ids.contains(&e.id))
    {
        let Some(loc) = e.locator.as_deref() else {
            continue;
        };
        let sidecar = parse(&tree, loc)?;
        out.insert(loc.to_string(), tree[loc].clone());
        let content = format!("{src_docs}/{}", str_of(&sidecar, "contentPath"));
        if let Some(bytes) = tree.get(&content) {
            out.insert(content, bytes.clone());
        }
        source_document_count += 1;
    }

    // --- Packages (D1 = P3): whole boundaries, closed at package granularity.
    let kept_roots = used_package_roots(store, &type_ids, &relation_types)?;
    let mut all_roots: BTreeSet<String> = cat.package_roots.iter().cloned().collect();
    all_roots.extend(kept_roots.iter().cloned());
    for (path, bytes) in &tree {
        // The deepest package root containing the path owns it.
        let Some(root) = all_roots
            .iter()
            .filter(|r| !r.is_empty() && path.starts_with(&format!("{r}/")))
            .max_by_key(|r| r.len())
        else {
            continue;
        };
        let in_instance_root = INSTANCE_ROOT_NAMES
            .iter()
            .any(|n| path.starts_with(&format!("{root}/{n}/")));
        if kept_roots.contains(root) && !in_instance_root {
            out.insert(path.clone(), bytes.clone());
        }
    }

    // --- Manifest (Change A/B, C step 1). ---------------------------------
    manifest["repositoryId"] = json!(slice_repository_id);
    // The manifest embed is a bare Container (no `$schema`, which a container
    // file may carry).
    if let Some(o) = boundary.as_object_mut() {
        o.remove("$schema");
    }
    manifest["container"] = boundary;
    let exts = manifest
        .as_object_mut()
        .expect("manifest is an object")
        .entry("declaredExtensions")
        .or_insert_with(|| json!([]));
    if let Some(a) = exts.as_array_mut() {
        if !a.iter().any(|x| x == "ext:slices") {
            a.push(json!("ext:slices"));
        }
    }
    if let Some(refs) = manifest
        .get_mut("packageRefs")
        .and_then(Value::as_array_mut)
    {
        refs.retain(|r| {
            str_of(r, "mode") != "local"
                || crate::vfs::ensure_contained(str_of(r, "path"))
                    .is_ok_and(|p| kept_roots.contains(&p))
        });
    }
    let external_relation_ref_count = external.len();
    let mut slice = json!({
        "origin": { "repositoryId": origin_repository_id },
        "spec": { "type": "container", "id": input.container_id },
        "exportedAt": exported_at,
    });
    if !external.is_empty() {
        slice["externalRelationRefs"] =
            Value::Array(external.into_iter().map(|(_, v)| v).collect());
    }
    manifest["slice"] = slice;
    out.insert("manifest.json".to_string(), pretty(&manifest));
    out.insert(
        format!("{}/.gitkeep", crate::vfs::SRS_MARKER_DIR),
        Vec::new(),
    );

    let mut buf = std::io::Cursor::new(Vec::new());
    pack_tree(&out, &mut buf)?;
    Ok(SliceExport {
        bytes: buf.into_inner(),
        summary: SliceExportSummary {
            container_id: input.container_id,
            slice_repository_id,
            origin_repository_id,
            exported_at,
            instance_count: included.len(),
            relation_count,
            container_count,
            source_document_count,
            package_count: kept_roots.len(),
            external_relation_ref_count,
        },
    })
}

/// D1 (P3): the package roots the slice uses — the boundaries holding the
/// records' Types and the relations' RelationTypes, closed over every followed
/// definition reference and every `packageDependencies` entry of a carried
/// package. The primary `package/` is always carried. An unresolved reference
/// carries nothing: core definitions live in the binary, and anything else is
/// the validator's to report.
fn used_package_roots(
    store: &dyn RepositoryStore,
    type_ids: &BTreeSet<String>,
    relation_types: &BTreeSet<String>,
) -> Result<BTreeSet<String>, RepositoryError> {
    struct Boundary {
        root: String,
        package_id: String,
        dependencies: Vec<String>,
        definitions: Vec<(DefinitionKind, Value)>,
    }
    let mut boundaries = Vec::new();
    for b in store.list_package_boundaries()? {
        let root = b.selector.clone().unwrap_or_else(|| "package".to_string());
        let dependencies = store
            .load_instance_json(&format!("{root}/package.json"))
            .ok()
            .and_then(|v| v.get("packageDependencies").cloned())
            .and_then(|d| d.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|d| str_of(d, "packageId").to_string())
            .collect();
        let definitions = load_boundary_definitions(store, &b)?
            .into_iter()
            .map(|d| (d.kind, d.value))
            .collect();
        boundaries.push(Boundary {
            root,
            package_id: b.id,
            dependencies,
            definitions,
        });
    }
    let holds = |b: &Boundary, kind: Option<DefinitionKind>, id: &str| {
        b.definitions
            .iter()
            .any(|(k, v)| kind.is_none_or(|want| *k == want) && str_of(v, "id") == id)
    };

    let mut kept: BTreeSet<usize> = BTreeSet::new();
    let mut queue: Vec<usize> = Vec::new();
    let add = |i: usize, kept: &mut BTreeSet<usize>, queue: &mut Vec<usize>| {
        if kept.insert(i) {
            queue.push(i);
        }
    };
    for (i, b) in boundaries.iter().enumerate() {
        let seeded = b.root == "package"
            || type_ids
                .iter()
                .any(|t| holds(b, Some(DefinitionKind::Type), t))
            || b.definitions.iter().any(|(k, v)| {
                *k == DefinitionKind::RelationType && relation_types.contains(str_of(v, "key"))
            });
        if seeded {
            add(i, &mut kept, &mut queue);
        }
    }
    while let Some(i) = queue.pop() {
        let b = &boundaries[i];
        let mut targets: Vec<usize> = Vec::new();
        for (kind, v) in &b.definitions {
            for r in followed_references(*kind, v) {
                targets.extend(
                    (0..boundaries.len()).filter(|&j| holds(&boundaries[j], Some(r.target), &r.id)),
                );
            }
        }
        for dep in &b.dependencies {
            targets.extend((0..boundaries.len()).filter(|&j| boundaries[j].package_id == *dep));
        }
        for j in targets {
            add(j, &mut kept, &mut queue);
        }
    }
    Ok(kept
        .into_iter()
        .map(|i| boundaries[i].root.clone())
        .collect())
}
