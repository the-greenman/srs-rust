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
//! - **Containers (RFC-034 [R9], Invariant I-151; ruling of 2026-10-04):** the
//!   slice carries the boundary and exactly its declared `childContainerIds`
//!   descendants, transitively, and includes every entry id of each. Membership
//!   is declared, never derived: an undeclared container is never carried, even
//!   when all its entries are included, and a declared child with no entries is
//!   still carried. A missing child or a cycle refuses the export ([R7]). This
//!   replaces RFC-026 Change C steps 2 and 6 (the subset test), so D3 is moot.
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
/// Refusal code for a `childContainerIds` entry naming no container (RFC-034 [R7]).
pub const CODE_CHILD_CONTAINER_MISSING: &str = "slice-child-container-missing";
/// Refusal code for a `childContainerIds` cycle, self-reference included (RFC-034 [R7]).
pub const CODE_CHILD_CONTAINER_CYCLE: &str = "slice-child-container-cycle";

/// Refusal code for two carried packages holding different definitions under one `id` and `version`.
pub const CODE_DEFINITION_IDENTITY_CONFLICT: &str = "slice-definition-identity-conflict";
/// Refusal code for a local package path resolving outside the repository root (RFC-017).
pub const CODE_PACKAGE_OUTSIDE_REPOSITORY: &str = "slice-package-outside-repository";

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

/// RFC-034 [R9]: `root` and every container reachable from it through declared
/// `childContainerIds` edges. [R7]: a missing target or a cycle leaves the
/// closure undefined, so it refuses rather than returning a partial set.
fn declared_closure(
    by_id: &HashMap<&str, &Value>,
    root: &str,
) -> Result<BTreeSet<String>, RepositoryError> {
    fn visit(
        by_id: &HashMap<&str, &Value>,
        id: &str,
        path: &mut Vec<String>,
        seen: &mut BTreeSet<String>,
    ) -> Result<(), RepositoryError> {
        if path.iter().any(|p| p == id) {
            return Err(refuse(
                CODE_CHILD_CONTAINER_CYCLE,
                format!(
                    "childContainerIds cycle: {} -> {id} (RFC-034 [R7])",
                    path.join(" -> ")
                ),
            ));
        }
        if !seen.insert(id.to_string()) {
            return Ok(());
        }
        path.push(id.to_string());
        let children = by_id[id]
            .get("childContainerIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str);
        for child in children {
            if !by_id.contains_key(child) {
                return Err(refuse(
                    CODE_CHILD_CONTAINER_MISSING,
                    format!(
                        "container '{id}' names child container '{child}', which does not \
                         exist (RFC-034 [R7]; repair the edge first)"
                    ),
                ));
            }
            visit(by_id, child, path, seen)?;
        }
        path.pop();
        Ok(())
    }
    let mut seen = BTreeSet::new();
    visit(by_id, root, &mut Vec::new(), &mut seen)?;
    Ok(seen)
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

    // --- Containers carried: the boundary and its declared descendants
    // (RFC-034 [R9], I-151). ---------------------------------------------
    let by_id: HashMap<&str, &Value> = containers
        .iter()
        .map(|(id, _, v)| (id.as_str(), v))
        .collect();
    let closure = declared_closure(&by_id, &input.container_id)?;

    // --- Instances: effective(boundary) = the entry ids of every container in
    // the closure that name an instance (RFC-034 [R9]). RFC-043 [R18]/[R7]: an
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
    let mut included: BTreeSet<String> = retained.iter().map(|e| e.instance_id.clone()).collect();
    for id in closure.iter().filter(|id| **id != input.container_id) {
        included.extend(
            entries_of(by_id[id.as_str()])
                .into_iter()
                .map(|e| e.instance_id)
                .filter(|i| instances.contains_key(i.as_str())),
        );
    }

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

    // --- Declared descendants, written whole (RFC-034 [R9]). The boundary
    // itself goes inline. -------------------------------------------------
    for (id, path, v) in &containers {
        if *id == input.container_id || !closure.contains(id) {
            continue;
        }
        let bytes = match tree.get(path) {
            Some(b) => b.clone(),
            // The source root container is inline in its manifest.
            None => pretty(v),
        };
        out.insert(path.clone(), bytes);
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
    let refs = match manifest.get("packageRefs").and_then(Value::as_array) {
        Some(a) => a.clone(),
        None => manifest.get("packageRef").cloned().into_iter().collect(),
    };
    for r in refs {
        let path = str_of(&r, "path");
        if str_of(&r, "mode") == "local" && crate::vfs::ensure_contained(path).is_err() {
            return Err(refuse(
                CODE_PACKAGE_OUTSIDE_REPOSITORY,
                format!("local package '{path}' resolves outside the repository root"),
            ));
        }
    }
    let kept_roots = used_package_roots(store, &type_ids, &relation_types)?;
    // ponytail: a package rooted at the repository root would make every file
    // "package content"; refused until a corpus needs it.
    if kept_roots.iter().any(|r| r.is_empty() || r == ".") {
        return Err(refuse(
            "slice-root-level-package-unsupported",
            "a package rooted at the repository root cannot be carried by a slice yet",
        ));
    }
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
        crate::vfs::SRS_MARKER_README_PATH.to_string(),
        crate::vfs::SRS_MARKER_README.as_bytes().to_vec(),
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
            container_count: closure.len(),
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
    let all = store.list_package_boundaries()?;
    for b in all.iter() {
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
        let definitions = load_boundary_definitions(store, b)?
            .into_iter()
            .map(|d| (d.kind, d.value))
            .collect();
        boundaries.push(Boundary {
            root,
            package_id: b.id.clone(),
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
    let carried: Vec<_> = kept.iter().map(|&i| all[i].clone()).collect();
    if let Some((id, version, at)) =
        crate::package_bundle::first_identity_conflict(store, &carried)?
    {
        return Err(refuse(
            CODE_DEFINITION_IDENTITY_CONFLICT,
            format!("id {id} version {version} has two different definitions ({at})"),
        ));
    }
    Ok(kept
        .into_iter()
        .map(|i| boundaries[i].root.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checked catalog already fatals on duplicate `id@version` in one tree, so an export cannot reach this through `export_container_slice`; the
    /// export's own conflict check is exercised directly on two package roots.
    #[test]
    fn differing_definitions_under_one_identity_refuse() {
        let field = |desc: &str| {
            serde_json::to_vec(&json!({"id": "dddddddd-0000-4000-8000-000000000001",
                "namespace": "n", "name": "dup", "version": 1, "description": desc,
                "aiGuidance": {"purpose": "x"}, "fieldType": {"datatype": "string"},
                "createdAt": "2026-07-22T00:00:00Z"}))
            .unwrap()
        };
        let pkg = |id: &str, deps: Value| {
            serde_json::to_vec(
                &json!({"id": id, "namespace": "n", "name": "p", "title": "p",
                "description": "", "status": "active", "createdAt": "2026-01-01T00:00:00Z",
                "version": "1.0.0", "fields": if id.starts_with("aaaaaaaa-0000-4000-8000-0000000000a0") { json!([]) } else { json!(["fields/dup.json"]) }, "types": [],
                "relationTypes": [], "packageDependencies": deps}),
            )
            .unwrap()
        };
        let dep =
            |id: &str| json!({"packageId": id, "namespace": "n", "name": "p", "version": "1.0.0"});
        let mut t = BTreeMap::new();
        t.insert(
            "manifest.json".to_string(),
            serde_json::to_vec(&json!({"srsVersion": "2.0-draft", "repositoryId": "11111111-1111-4111-8111-111111111111",
                "namespace": "n", "dataModelRevision": 2, "packageRefs": [{"mode": "local", "path": "a"}, {"mode": "local", "path": "b"}]}))
            .unwrap(),
        );
        t.insert(
            "package/package.json".into(),
            pkg(
                "aaaaaaaa-0000-4000-8000-0000000000a0",
                json!([
                    dep("aaaaaaaa-0000-4000-8000-0000000000a1"),
                    dep("aaaaaaaa-0000-4000-8000-0000000000a2")
                ]),
            ),
        );
        t.insert(
            "a/package.json".into(),
            pkg("aaaaaaaa-0000-4000-8000-0000000000a1", json!([])),
        );
        t.insert("a/fields/dup.json".into(), field("a"));
        t.insert(
            "b/package.json".into(),
            pkg("aaaaaaaa-0000-4000-8000-0000000000a2", json!([])),
        );
        t.insert("b/fields/dup.json".into(), field("b"));
        let store = crate::tree_session::open_tree(t).unwrap();
        match used_package_roots(&store, &BTreeSet::new(), &BTreeSet::new()) {
            Err(RepositoryError::SliceRefused { code, .. }) => {
                assert_eq!(code, CODE_DEFINITION_IDENTITY_CONFLICT)
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
}
