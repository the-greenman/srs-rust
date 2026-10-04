//! RFC-044 (srs-rust#1087, #1168): `packageDependencies` never fails a load
//! ([R9]/[R11]), the installed set (Change D items 1 to 5), every
//! `package-dependency-unsatisfied` reason, label mismatch, the bundle check,
//! and the write path.

use serde_json::{json, Value};
use srs_repository::catalog;
use srs_repository::field_type_migration_service::CURRENT_DATA_MODEL_REVISION;
use srs_repository::package_dependency_service::{
    add_package_dependency, check_bundle, check_repository, installed_set,
    list_package_dependencies, remove_package_dependency, AddPackageDependencyInput,
    BundleRequirements, DependencyWriteAction, PackageDependencyFinding,
    RemovePackageDependencyInput, CODE_INVALID, CODE_LABEL_MISMATCH, CODE_UNSATISFIED,
};
use srs_repository::srsj::open_srsj;
use srs_repository::store::{FileStore, RepositoryStore};
use srs_repository::validation::{validate_repository, DiagnosticSeverity};

const G: &str = "1cd9622e-3d05-4214-a683-4cb81d0c44d9"; // governance
const A: &str = "126eeba4-3124-4ee6-a9e5-5a943cb87cb5"; // argument
const P: &str = "e0000003-0000-4000-a000-000000000003"; // prerelease-only package
const U: &str = "e0000004-0000-4000-a000-000000000004"; // unknown version
const CORE: &str = "3a000001-0000-4000-a000-000000000001";
const UPSTREAM: &str = "e0000005-0000-4000-a000-000000000005";
const SINGULAR: &str = "e0000006-0000-4000-a000-000000000006";
const NOWHERE: &str = "e0000007-0000-4000-a000-000000000007";

fn pkg(id: &str, ns: &str, name: &str, version: &str, deps: Option<Value>) -> Value {
    let mut v = json!({
        "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
        "id": id, "namespace": ns, "name": name, "version": version,
        "title": name, "description": "", "status": "active",
        "createdAt": "2026-01-01T00:00:00Z", "fields": [], "types": [],
    });
    if let Some(d) = deps {
        v["packageDependencies"] = d;
    }
    v
}

fn dep(pid: &str, ns: &str, name: &str, version: &str) -> Value {
    json!({"packageId": pid, "namespace": ns, "name": name, "version": version})
}

/// governance 1.0.0 + 1.2.1 (same id), argument 1.3.0 requiring `arg_deps`,
/// a pre-release-only package, an unknown-version package. `packageRef`
/// (singular) and `upstreamPackage` are present and must contribute nothing.
fn repo(arg_deps: Value) -> FileStore {
    open_srsj(&repo_srsj(arg_deps).to_string()).unwrap()
}

fn repo_srsj(arg_deps: Value) -> Value {
    json!({
        "srsj": "2",
        "manifest": {
            "$schema": srs_schema::MANIFEST_SCHEMA_ID,
            "srsVersion": "2.0-draft",
            "dataModelRevision": CURRENT_DATA_MODEL_REVISION,
            "repositoryId": "00000000-0000-4000-8000-00000000dddd",
            "namespace": "com.example.deps",
            "title": "Package dependency fixture",
            "createdAt": "2026-01-01T00:00:00Z",
            "container": {
                "containerId": "00000000-0000-4000-8000-00000000eeee",
                "title": "Package dependency fixture",
            },
            "packageRef": { "mode": "local", "path": "package/singular" },
            "upstreamPackage": {
                "packageId": UPSTREAM, "namespace": "com.example", "name": "up",
                "version": "1.0.0", "installedAt": "2026-01-01T00:00:00Z",
            },
            "packageRefs": [
                { "mode": "local", "path": "package/gov-1" },
                { "mode": "local", "path": "package/gov-2" },
                { "mode": "local", "path": "package/argument" },
                { "mode": "local", "path": "package/pre" },
                { "mode": "local", "path": "package/unknown" },
            ],
        },
        "data": {
            "package/package.json": pkg("00000000-0000-4000-8000-00000000a0a0", "com.example.deps", "primary", "1.0.0", None),
            "package/singular/package.json": pkg(SINGULAR, "com.example", "singular", "1.0.0", None),
            "package/gov-1/package.json": pkg(G, "com.mudemocracy.governance", "governance", "1.0.0", None),
            "package/gov-2/package.json": pkg(G, "com.mudemocracy.governance", "governance", "1.2.1", None),
            "package/argument/package.json": pkg(A, "com.mudemocracy.argument", "argument", "1.3.0", Some(arg_deps)),
            "package/pre/package.json": pkg(P, "com.example", "pre", "1.3.0-rc.1", None),
            "package/unknown/package.json": pkg(U, "com.example", "unknown", "latest", None),
        },
    })
}

fn unsatisfied_reasons(findings: &[PackageDependencyFinding]) -> Vec<&'static str> {
    findings
        .iter()
        .filter(|f| f.code == CODE_UNSATISFIED)
        .map(|f| f.reason.unwrap().as_str())
        .collect()
}

#[test]
fn installed_set_follows_change_d() {
    let store = repo(json!([]));
    let ids: Vec<(String, Option<String>)> = installed_set(&store)
        .unwrap()
        .into_iter()
        .map(|m| (m.package.id, m.package.version))
        .collect();
    // packageRefs wins over packageRef; upstreamPackage contributes nothing;
    // both governance versions are members; the core package is a member.
    assert!(!ids.iter().any(|(id, _)| id == SINGULAR || id == UPSTREAM));
    assert_eq!(ids.iter().filter(|(id, _)| id == G).count(), 2);
    assert!(ids.iter().any(|(id, v)| id == CORE && v.is_some()));
    assert_eq!(ids.len(), 7); // 5 refs + primary + core
}

/// RFC-044 / srs-rust#1223: a repo whose manifest declares neither
/// `packageRefs` nor `packageRef` (the common case — every srs-web e2e
/// fixture and the bundled governance seed lack it) must still have its
/// primary `package/` resolve as installed, the same way `list_packages`
/// already treats it.
fn repo_without_ref() -> FileStore {
    repo_with_refs(json!({}))
}

/// Same fixture with `extra` merged into the manifest (`packageRefs` etc.);
/// carries a sub-package at `extensions/x` for refs to point at.
fn repo_with_refs(extra: Value) -> FileStore {
    let mut manifest = json!({
        "$schema": srs_schema::MANIFEST_SCHEMA_ID,
        "srsVersion": "2.0-draft",
        "dataModelRevision": CURRENT_DATA_MODEL_REVISION,
        "repositoryId": "00000000-0000-4000-8000-00000000dcdc",
        "namespace": "com.example.noref",
        "title": "No packageRef fixture",
        "createdAt": "2026-01-01T00:00:00Z",
        "container": {
            "containerId": "00000000-0000-4000-8000-00000000dcde",
            "title": "No packageRef fixture",
        },
    });
    manifest
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    open_srsj(
        &json!({
            "srsj": "2",
            "manifest": manifest,
            "data": {
                "package/package.json": pkg("00000000-0000-4000-8000-00000000a0a0", "com.example.deps", "primary", "1.0.0", None),
                "extensions/x/package.json": pkg(SINGULAR, "com.example", "ext-x", "1.0.0", None),
            },
        })
        .to_string(),
    )
    .unwrap()
}

fn count_id(store: &FileStore, id: &str) -> usize {
    installed_set(store)
        .unwrap()
        .iter()
        .filter(|m| m.package.id == id)
        .count()
}

const PRIMARY: &str = "00000000-0000-4000-8000-00000000a0a0";

/// Review of #1224: the primary is installed whatever the ref shape, and a
/// ref to `package` itself does not count it twice.
#[test]
fn installed_set_always_has_primary_once() {
    let sub = |r: Value| repo_with_refs(r);
    let s = sub(json!({"packageRefs": [{"mode": "local", "path": "extensions/x"}]}));
    assert_eq!(count_id(&s, PRIMARY), 1, "sub-only packageRefs");
    assert_eq!(count_id(&s, SINGULAR), 1);
    assert_eq!(
        count_id(&sub(json!({"packageRefs": []})), PRIMARY),
        1,
        "empty packageRefs"
    );
    let d = sub(json!({"packageRef": {"mode": "local", "path": "package"}}));
    assert_eq!(count_id(&d, PRIMARY), 1, "packageRef -> package");
}

#[test]
fn installed_set_includes_primary_when_no_ref_declared() {
    let store = repo_without_ref();
    let members = installed_set(&store).unwrap();
    let primary = members
        .iter()
        .find(|m| m.package.id == "00000000-0000-4000-8000-00000000a0a0")
        .expect("the primary package must be installed even with no packageRef declared (srs-rust#1223)");
    assert_eq!(primary.package.namespace, "com.example.deps");
    assert_eq!(primary.package.name, "primary");
    assert_eq!(primary.package.version, Some("1.0.0".to_string()));
    assert_eq!(primary.selector, None);
    // primary + the implicit core package only.
    assert_eq!(members.len(), 2);
}

/// srs-rust#1225: `installed_set` already treats a singular-only `packageRef`
/// as a one-element ref list (`installed_set_always_has_primary_once`), but
/// the actual loaders (`list_package_boundaries`, `load_package`) read only
/// the plural `packageRefs` — so a sub-package declared solely via the
/// singular form counted as "installed" for dependency checks while its own
/// metadata and field/type definitions were silently never loaded into the
/// catalog.
#[test]
fn singular_package_ref_is_loaded_not_just_counted_installed() {
    let store = repo_with_refs(json!({
        "packageRef": {"mode": "local", "path": "extensions/x"},
    }));

    // installed_set already saw it before the fix (that was never the bug).
    assert_eq!(count_id(&store, SINGULAR), 1);

    // The loader must see it too: list_package_boundaries...
    let boundaries = store.list_package_boundaries().unwrap();
    assert!(
        boundaries.iter().any(|b| b.id == SINGULAR),
        "a sub-package declared only via singular packageRef must be a \
         listed boundary: {boundaries:?}"
    );
}

/// Same fixture shape, proving the sub-package's own definition (not just its
/// package.json metadata) was merged by `load_package`.
#[test]
fn singular_package_ref_definitions_are_merged_into_catalog() {
    let extra = json!({
        "packageRef": {"mode": "local", "path": "extensions/x"},
    });
    let mut manifest = json!({
        "$schema": srs_schema::MANIFEST_SCHEMA_ID,
        "srsVersion": "2.0-draft",
        "dataModelRevision": CURRENT_DATA_MODEL_REVISION,
        "repositoryId": "00000000-0000-4000-8000-00000000dcdc",
        "namespace": "com.example.noref",
        "title": "No packageRef fixture",
        "createdAt": "2026-01-01T00:00:00Z",
        "container": {
            "containerId": "00000000-0000-4000-8000-00000000dcde",
            "title": "No packageRef fixture",
        },
    });
    manifest
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let field_id = "f0000000-0000-4000-a000-000000000001";
    let store = open_srsj(
        &json!({
            "srsj": "2",
            "manifest": manifest,
            "data": {
                "package/package.json": pkg("00000000-0000-4000-8000-00000000a0a0", "com.example.deps", "primary", "1.0.0", None),
                "extensions/x/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": SINGULAR, "namespace": "com.example", "name": "ext-x", "version": "1.0.0",
                    "title": "ext-x", "description": "", "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": ["fields/singular.json"],
                    "types": [],
                },
                "extensions/x/fields/singular.json": {
                    "$schema": srs_schema::FIELD_SCHEMA_ID,
                    "id": field_id,
                    "namespace": "com.example",
                    "name": "singular_field",
                    "version": 1,
                    "description": "d",
                    "aiGuidance": {"purpose": "p"},
                    "fieldType": {"datatype": "string"},
                    "createdAt": "2026-01-01T00:00:00Z",
                },
            },
        })
        .to_string(),
    )
    .unwrap();

    let package = store.load_package().expect("load_package must succeed");
    assert!(
        package.fields.iter().any(|f| f.id == field_id),
        "the singular packageRef's field must be merged into the catalog by load_package: {:?}",
        package.fields.iter().map(|f| &f.id).collect::<Vec<_>>()
    );
}

/// Item 3 of #1223's "Wanted": the WASM-bound `check_package_requirements`
/// (here, its underlying service `check_bundle`) must satisfy a requirement
/// on a repo's own primary package even when the manifest carries no
/// `packageRef`.
#[test]
fn check_bundle_satisfies_requirement_on_primary_when_no_ref_declared() {
    let store = repo_without_ref();
    let bundle: BundleRequirements = serde_json::from_value(json!({
        "packageId": "",
        "packageDependencies": [dep(
            "00000000-0000-4000-8000-00000000a0a0",
            "com.example.deps",
            "primary",
            "1.0.0",
        )],
    }))
    .unwrap();
    let result = check_bundle(&store, &bundle).unwrap();
    assert!(
        result.dependencies[0].satisfied,
        "a requirement on the repo's own primary package must be satisfied even without a \
         declared packageRef (srs-rust#1223): {:?}",
        result.dependencies[0]
    );
}

#[test]
fn every_reason_code() {
    let store = repo(json!([
        {"namespace": "com.mudemocracy.governance", "name": "governance", "version": "1.0.0"},
        dep(A, "com.mudemocracy.argument", "argument", "1.0.0"),
        dep(NOWHERE, "com.example", "nowhere", "1.0.0"),
        dep(U, "com.example", "unknown", "1.0.0"),
        dep(G, "com.mudemocracy.governance", "governance", "2.0.0"),
        dep(P, "com.example", "pre", "1.2.0"),
        dep(G, "com.mudemocracy.governance", "governance", "1.3.0"),
        // multi-version: 1.2.1 satisfies 1.1.0
        dep(G, "com.mudemocracy.governance", "governance", "1.1.0"),
        // upstreamPackage and the singular packageRef are not installed
        dep(UPSTREAM, "com.example", "up", "1.0.0"),
        dep(SINGULAR, "com.example", "singular", "1.0.0"),
    ]));
    let findings = check_repository(&store).unwrap();
    assert_eq!(
        unsatisfied_reasons(&findings),
        [
            "no-package-id",
            "self-requirement",
            "missing",
            "version-unknown",
            "incompatible",
            "prerelease-excluded",
            "version-too-low",
            "missing",
            "missing",
        ]
    );
    let incompatible = findings
        .iter()
        .find(|f| f.reason.map(|r| r.as_str()) == Some("incompatible"))
        .unwrap();
    assert_eq!(incompatible.severity, DiagnosticSeverity::Warning);
    assert_eq!(incompatible.requiring_package_id, A);
    assert_eq!(incompatible.path, "package/argument/package.json");
    assert_eq!(
        incompatible.candidate_versions,
        [Some("1.0.0".to_string()), Some("1.2.1".to_string())]
    );
    assert!(incompatible.message.contains("reason: incompatible"));
    assert!(incompatible.message.contains(G));
}

#[test]
fn core_package_is_a_member() {
    let store = repo(json!([
        dep(CORE, "com.semanticops.core", "core", "1.0.0"),
        dep(CORE, "com.semanticops.core", "core", "2.0.0"),
    ]));
    assert_eq!(
        unsatisfied_reasons(&check_repository(&store).unwrap()),
        ["incompatible"]
    );
}

#[test]
fn label_mismatch_is_info_and_never_decides() {
    let store = repo(json!([dep(G, "com.old.name", "governance", "1.0.0")]));
    let findings = check_repository(&store).unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, CODE_LABEL_MISMATCH);
    assert_eq!(findings[0].severity, DiagnosticSeverity::Info);
    assert_eq!(
        findings[0].mismatched_labels,
        ["com.mudemocracy.governance/governance"]
    );
    // Info counts as neither an error nor a warning.
    let report = validate_repository(&store).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Info
                && d.message.contains(CODE_LABEL_MISMATCH))
    );
}

fn package_dependency_diags(store: &dyn RepositoryStore) -> Vec<(DiagnosticSeverity, String)> {
    validate_repository(store)
        .unwrap()
        .diagnostics
        .into_iter()
        .filter(|d| d.message.starts_with("package-dependency"))
        .map(|d| (d.severity, d.message))
        .collect()
}

#[test]
fn legacy_entry_loads_with_diagnostics() {
    let store = repo(json!([
        {"namespace": "com.mudemocracy.governance", "name": "governance", "version": "1.0.0"}
    ]));
    // [R9]: never a load failure.
    catalog::build_checked(&store).expect("legacy entry must not fail the load");
    store
        .load_package()
        .expect("legacy entry must not fail the package load");
    let diags = package_dependency_diags(&store);
    assert_eq!(diags.len(), 2, "{diags:?}");
    assert!(diags.iter().all(|(s, _)| *s == DiagnosticSeverity::Warning));
    assert!(diags[0].1.starts_with(CODE_INVALID));
    assert!(diags[1].1.contains("reason: no-package-id"));
}

#[test]
fn repaired_entry_loads_clean() {
    let store = repo(json!([dep(
        G,
        "com.mudemocracy.governance",
        "governance",
        "1.0.0"
    )]));
    catalog::build_checked(&store).expect("RFC-044 entry must load under the embedded schema");
    assert!(package_dependency_diags(&store).is_empty());
}

#[test]
fn malformed_entries_are_errors_not_load_failures() {
    let store = repo(json!([
        {"packageId": "not-a-uuid", "namespace": 1, "version": 3, "extra": true},
        "a string",
    ]));
    catalog::build_checked(&store).expect("malformed entries must not fail the load");
    store
        .load_package()
        .expect("malformed entries must not fail the package load");
    let diags = package_dependency_diags(&store);
    let errors: Vec<_> = diags
        .iter()
        .filter(|(s, m)| *s == DiagnosticSeverity::Error && m.starts_with(CODE_INVALID))
        .collect();
    // extra, packageId, namespace, name, version, + the non-object entry.
    assert_eq!(errors.len(), 6, "{diags:?}");

    let not_array = repo(json!({"oops": true}));
    catalog::build_checked(&not_array).expect("non-array must not fail the load");
    assert!(package_dependency_diags(&not_array)
        .iter()
        .any(|(s, m)| *s == DiagnosticSeverity::Error && m.contains("not an array")));
}

#[test]
fn bundle_check_before_install() {
    let store = repo(json!([]));
    let check = |bundle: Value| -> Vec<Option<&'static str>> {
        let b: BundleRequirements = serde_json::from_value(bundle).unwrap();
        check_bundle(&store, &b)
            .unwrap()
            .dependencies
            .iter()
            .map(|d| d.reason.map(|r| r.as_str()))
            .collect()
    };
    assert!(check(json!({"packageId": G, "packageDependencies": []})).is_empty());
    // Rule 2: the bundle's own id is already installed (another version).
    assert_eq!(
        check(json!({"packageId": G, "packageDependencies": [dep(G, "x", "y", "1.0.0")]})),
        [Some("self-requirement")]
    );
    assert_eq!(
        check(json!({"packageId": NOWHERE, "packageDependencies": [
            dep(A, "com.mudemocracy.argument", "argument", "1.0.0"),
            dep(G, "com.mudemocracy.governance", "governance", "1.5.0"),
        ]})),
        [None, Some("version-too-low")]
    );
    // A requirement list that is not a package's (no packageId), with a
    // malformed and a legacy entry: reported, never refused ([R9]).
    assert_eq!(
        check(json!({"packageDependencies": [
            dep(G, "com.mudemocracy.governance", "governance", "1.0.0"),
            {"namespace": "a", "name": "b", "version": "1.0.0"},
            {"packageId": G, "version": 3},
        ]})),
        [None, Some("no-package-id"), Some("incompatible")]
    );
}

/// RFC-044's non-fatal carve-out is exactly one property: any other
/// package-manifest schema violation still fails the load.
#[test]
fn only_package_dependencies_leaves_the_fatal_check() {
    let mut src = repo_srsj(json!([{"bogus": 1}]));
    catalog::build_checked(&open_srsj(&src.to_string()).unwrap())
        .expect("a malformed packageDependencies must not fail the load");
    src["data"]["package/argument/package.json"]["unknownProperty"] = json!(true);
    assert!(
        catalog::build_checked(&open_srsj(&src.to_string()).unwrap()).is_err(),
        "an unknown manifest property must stay fatal"
    );
    src["data"]["package/argument/package.json"]
        .as_object_mut()
        .unwrap()
        .remove("unknownProperty");
    src["data"]["package/argument/package.json"]["fields"] = json!("not-an-array");
    assert!(
        catalog::build_checked(&open_srsj(&src.to_string()).unwrap()).is_err(),
        "a malformed definition array must stay fatal"
    );
}

/// Installing a package keeps its requirement list verbatim, so the check
/// sees an installed package's requirements.
#[test]
fn install_carries_package_dependencies() {
    use srs_repository::package_install_service::{
        install_package_bundle, InstallBundleOptions, PackageSourceBundle,
    };
    let store = repo(json!([]));
    let deps = vec![
        dep(G, "com.mudemocracy.governance", "governance", "9.0.0"),
        json!({"namespace": "legacy", "name": "kept", "version": "1.0.0"}),
    ];
    install_package_bundle(
        &store,
        &PackageSourceBundle {
            id: NOWHERE.to_string(),
            namespace: "com.example".to_string(),
            name: "installed".to_string(),
            version: "1.0.0".to_string(),
            package_dependencies: Some(deps.clone()),
            definitions: vec![],
        },
        InstallBundleOptions::default(),
    )
    .unwrap();
    assert_eq!(
        raw_deps(&store, "packages/installed/package.json"),
        Value::Array(deps)
    );
    let reasons = unsatisfied_reasons(&check_repository(&store).unwrap());
    assert!(reasons.contains(&"incompatible"), "{reasons:?}");
    assert!(reasons.contains(&"no-package-id"), "{reasons:?}");
}

fn raw_deps(store: &FileStore, path: &str) -> Value {
    store.load_instance_json(path).unwrap()["packageDependencies"].clone()
}

#[test]
fn write_path_repairs_updates_adds_and_removes() {
    let legacy_other = json!({"namespace": "com.other", "name": "other", "version": "1.0.0"});
    let store = repo(json!([
        {"namespace": "com.mudemocracy.governance", "name": "governance", "version": "1.0.0"},
        legacy_other.clone(),
    ]));
    let sel = Some("package/argument".to_string());
    let add_with = |pid: &str, version: &str, repair_legacy: bool| {
        add_package_dependency(
            &store,
            AddPackageDependencyInput {
                selector: sel.clone(),
                package_id: pid.to_string(),
                version: version.to_string(),
                repair_legacy,
            },
        )
    };
    let add = |pid: &str, version: &str| add_with(pid, version, false);

    // [R11]: without the explicit opt-in, a label-matching legacy entry is
    // never rewritten — the add is refused and the file is untouched.
    let before = raw_deps(&store, "package/argument/package.json");
    assert!(add(G, "1.0.0")
        .unwrap_err()
        .to_string()
        .contains("--repair-legacy"));
    assert_eq!(raw_deps(&store, "package/argument/package.json"), before);
    // Opting in with no label-matching legacy entry is refused too.
    assert!(add_with(CORE, "1.0.0", true)
        .unwrap_err()
        .to_string()
        .contains("no legacy"));

    // Repair: the caller supplies the id and opts in; labels come from the
    // installed package and only locate the legacy entry.
    let r = add_with(G, "1.0.0", true).unwrap();
    assert_eq!(r.action, Some(DependencyWriteAction::Repaired));
    assert_eq!(r.package_id, A);
    let raw = raw_deps(&store, "package/argument/package.json");
    assert_eq!(
        raw[0],
        dep(G, "com.mudemocracy.governance", "governance", "1.0.0")
    );
    // [R11]: the other legacy entry is preserved unchanged.
    assert_eq!(raw[1], legacy_other);

    // Update by packageId.
    assert_eq!(
        add(G, "1.2.0").unwrap().action,
        Some(DependencyWriteAction::Updated)
    );
    // Add a new one (the core package resolves too).
    let r = add(CORE, "1.0.0").unwrap();
    assert_eq!(r.action, Some(DependencyWriteAction::Added));
    assert!(r
        .dependencies
        .iter()
        .filter(|d| d.entry.package_id.is_some())
        .all(|d| d.satisfied));

    // Refusals: never guess labels; bad version; bad id; self.
    assert!(add(NOWHERE, "1.0.0")
        .unwrap_err()
        .to_string()
        .contains("never guessed"));
    assert!(add(G, "1.0").is_err());
    assert!(add("not-a-uuid", "1.0.0").is_err());
    assert!(add(A, "1.0.0").unwrap_err().to_string().contains("itself"));

    // Remove by packageId; removing an absent id is refused.
    let r = remove_package_dependency(
        &store,
        RemovePackageDependencyInput {
            selector: sel.clone(),
            package_id: CORE.to_string(),
        },
    )
    .unwrap();
    assert_eq!(r.action, Some(DependencyWriteAction::Removed));
    assert_eq!(r.dependencies.len(), 2);
    assert!(remove_package_dependency(
        &store,
        RemovePackageDependencyInput {
            selector: sel.clone(),
            package_id: CORE.to_string(),
        },
    )
    .is_err());

    let listed = list_package_dependencies(&store, sel).unwrap();
    assert_eq!(listed.action, None);
    assert_eq!(listed.dependencies[0].reason, None);
    assert_eq!(
        listed.dependencies[1].reason.map(|r| r.as_str()),
        Some("no-package-id")
    );
}

#[test]
fn metadata_update_preserves_dependencies() {
    let store = repo(json!([{"namespace": "a", "name": "b", "version": "1.0.0"}]));
    let before = raw_deps(&store, "package/argument/package.json");
    srs_repository::package_service::update_package_metadata(
        &store,
        Some("package/argument".to_string()),
        srs_repository::package_service::UpdatePackageMetadataInput {
            version: Some("1.4.0".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(raw_deps(&store, "package/argument/package.json"), before);
}
