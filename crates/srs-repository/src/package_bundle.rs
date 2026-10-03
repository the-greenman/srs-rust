//! # `.srspkg` Package Bundle codec (ADR-050; srs-rust#632, #690, #663)
//!
//! One reader ([`read_package_bundle`], bytes in, store-free) and one writer
//! ([`export_package_bundle`], deterministic bytes out). Both install entry
//! points (directory, bundle) end in the one
//! [`crate::package_install_service::install_package_bundle`] core.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use srs_schema::{SchemaRegistry, PACKAGE_BUNDLE_SCHEMA_ID};

use crate::error::RepositoryError;
use crate::field_type_migration_service::{CURRENT_DATA_MODEL_REVISION, DATA_MODEL_REVISION_KEY};
use crate::package_install_service::{
    definition_name, kind_label, load_boundary_definitions, validate_source_definition,
    PackageSourceBundle, PackageSourceDefinition, INSTALL_ORDER,
};
use crate::package_service::definition_rel_path;
use crate::package_types::DefinitionKind;
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

/// Read a `.srspkg` from its bytes: parse, refuse a `readme` (srs-rust#1164),
/// refuse a newer revision, run the RFC-043 pre-load transformer (always;
/// idempotent), validate against `package-bundle.json`, then validate every
/// definition with the loader's own strictness.
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
    let notes =
        crate::rfc043_container_entries_migration_service::migrate_package_bundle_value(&mut v)
            .map_err(|e| refuse("bundle-migration-refused", e.to_string()))?
            .diagnostics;
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
                rel_path: definition_rel_path(kind, &name, id),
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

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

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
    pub data_model_revision: u64,
    pub published_at: String,
    /// `sha256:<64 lowercase hex>` over `text`'s UTF-8 bytes.
    pub sha256: String,
    pub byte_length: usize,
    pub definition_count: usize,
    /// Ids inlined from other boundaries by the closure (sorted).
    pub inlined: Vec<String>,
    /// Per-kind counts, install order, kinds with count > 0 only.
    pub kinds: Vec<PackageExportKindCount>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageExportKindCount {
    pub kind: String,
    pub count: usize,
}

fn string_leaves(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| string_leaves(x, out)),
        Value::Object(o) => o.values().for_each(|x| string_leaves(x, out)),
        _ => {}
    }
}

/// Export a whole package boundary as a deterministic `mode: "bundled"`
/// `.srspkg` (ADR-050): closure over the repository's other boundaries (core
/// omitted, OD5), definitions sorted by id, keys canonicalized, validated
/// against `package-bundle.json` before it is returned.
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
    store
        .load_instance_json(&format!("{prefix}/package.json"))
        .map_err(|e| {
            refuse(
                "bundle-boundary-unreadable",
                format!("package boundary '{prefix}': {prefix}/package.json cannot be loaded: {e}"),
            )
        })?;

    // Every other boundary's definitions, by id (first boundary wins).
    let mut index: HashMap<String, (DefinitionKind, Value)> = HashMap::new();
    for b in boundaries.iter().filter(|b| b.selector != target.selector) {
        for d in load_boundary_definitions(store, b)? {
            if let Some(id) = d.value["id"].as_str() {
                index.entry(id.to_string()).or_insert((d.kind, d.value));
            }
        }
    }
    let mut selected: Vec<(DefinitionKind, Value)> = load_boundary_definitions(store, target)?
        .into_iter()
        .map(|d| (d.kind, d.value))
        .collect();
    let mut seen: HashSet<String> = selected
        .iter()
        .filter_map(|(_, v)| v["id"].as_str().map(str::to_string))
        .collect();
    let mut inlined = BTreeSet::new();
    // ponytail: id-scan closure; replace with a typed reference walker if a non-reference string ever carries a definition UUID.
    let mut next = 0;
    while next < selected.len() {
        let mut leaves = Vec::new();
        string_leaves(&selected[next].1, &mut leaves);
        next += 1;
        for leaf in leaves {
            if seen.contains(&leaf) {
                continue;
            }
            if let Some((kind, value)) = index.get(&leaf) {
                seen.insert(leaf.clone());
                selected.push((*kind, value.clone()));
                inlined.insert(leaf);
            }
        }
    }

    let manifest = store.load_manifest()?;
    let mut bundle = serde_json::Map::new();
    let mut put = |k: &str, v: Value| {
        bundle.insert(k.to_string(), v);
    };
    put("$schema", Value::from(PACKAGE_BUNDLE_SCHEMA_ID));
    put(
        "schemaVersion",
        manifest
            .extra
            .get("srsVersion")
            .cloned()
            .unwrap_or_else(|| Value::from("2.0")),
    );
    put("packageId", Value::from(target.id.clone()));
    put("packageNamespace", Value::from(target.namespace.clone()));
    put("packageName", Value::from(target.name.clone()));
    put("packageVersion", Value::from(target.version.clone()));
    put(
        DATA_MODEL_REVISION_KEY,
        Value::from(CURRENT_DATA_MODEL_REVISION),
    );
    put("publishedAt", Value::from(published_at.clone()));
    if let Some(p) = input.publisher {
        put("publisher", Value::from(p));
    }
    put("mode", Value::from("bundled"));
    let mut kinds = Vec::new();
    for kind in INSTALL_ORDER {
        let mut items: Vec<Value> = selected
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, v)| v.clone())
            .collect();
        items.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
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
    put("dependencyRefs", Value::Array(vec![]));
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
    let summary = PackageExportSummary {
        package_id: target.id.clone(),
        package_namespace: target.namespace.clone(),
        package_name: target.name.clone(),
        package_version: target.version.clone(),
        data_model_revision: CURRENT_DATA_MODEL_REVISION,
        published_at,
        sha256: crate::attachment_service::sha256_hex(text.as_bytes()),
        byte_length: text.len(),
        definition_count: selected.len(),
        inlined: inlined.into_iter().collect(),
        kinds,
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
        store
            .initialize_repository(&InitializeRepositoryInput {
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
            })
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
    fn export_definitions_sorted_by_id_and_empty_kinds_omitted() {
        let (_t, store) = fresh();
        sub_package(&store, "packages/s", "5e000000-0000-4000-8000-000000000001");
        put(
            &store,
            Some("packages/s"),
            DefinitionKind::Field,
            field("ffffffff-0000-4000-8000-000000000002", "zeta"),
        );
        put(
            &store,
            Some("packages/s"),
            DefinitionKind::Field,
            field("11111111-0000-4000-8000-000000000001", "alpha"),
        );
        let b = parsed(&export(&store, Some("packages/s")));
        let ids: Vec<&str> = b["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec![
                "11111111-0000-4000-8000-000000000001",
                "ffffffff-0000-4000-8000-000000000002"
            ]
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
        assert_eq!(parsed(&e)["fields"][0]["id"], fid);
    }

    /// Two non-target boundaries define the same id: the first listed (primary) wins.
    #[test]
    fn export_closure_first_boundary_wins_on_duplicate_id() {
        let (_t, store) = fresh();
        let fid = "f1e1d000-0000-4000-8000-0000000000dd";
        let mut primary = field(fid, "dup");
        primary["description"] = json!("from primary");
        put(&store, None, DefinitionKind::Field, primary);
        sub_package(&store, "packages/b", "5e000000-0000-4000-8000-000000000006");
        let mut other = field(fid, "dup");
        other["description"] = json!("from b");
        put(&store, Some("packages/b"), DefinitionKind::Field, other);
        sub_package(&store, "packages/t", "5e000000-0000-4000-8000-000000000007");
        put(
            &store,
            Some("packages/t"),
            DefinitionKind::Type,
            ty("7e000000-0000-4000-8000-000000000003", fid),
        );
        let b = parsed(&export(&store, Some("packages/t")));
        assert_eq!(b["fields"].as_array().unwrap().len(), 1);
        assert_eq!(b["fields"][0]["description"], "from primary");
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
    fn export_omits_core_package_definitions() {
        let (_t, store) = fresh();
        let core_field = crate::core_package::core_package().fields[0].id.clone();
        sub_package(&store, "packages/c", "5e000000-0000-4000-8000-000000000003");
        // A view, not a type: the [R13] catalog resolves a Type's FieldAssignment only
        // against the repository's own definition files, never the embedded core
        // (pre-existing; see the plan's deviations), while views resolve against the
        // merged package (ADR-025), which is exactly what OD5 relies on.
        put(
            &store,
            Some("packages/c"),
            DefinitionKind::View,
            json!({"$schema": "https://srs.semanticops.com/schema/2.0/view.json",
                "id": "7e000000-0000-4000-8000-000000000002", "namespace": "com.test.export",
                "name": "v", "version": 1, "description": "d", "createdAt": "2026-01-01T00:00:00Z",
                "fieldViews": [{"fieldId": core_field, "order": 0, "required": false, "visible": true}]}),
        );
        let e = export(&store, Some("packages/c"));
        assert!(e.summary.inlined.is_empty());
        assert_eq!(parsed(&e)["fields"], json!([]));

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
    fn export_stamps_current_data_model_revision() {
        let (_t, store) = with_fixture();
        let e = export(&store, Some(FIXTURE));
        assert_eq!(parsed(&e)["dataModelRevision"], CURRENT_DATA_MODEL_REVISION);
        assert_eq!(e.summary.data_model_revision, CURRENT_DATA_MODEL_REVISION);
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
}
