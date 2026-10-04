//! RFC-044 package requirements — the one core owner of:
//!
//! - the **installed set** of a repository (Change D items 1 to 5, [R4]);
//! - the optional **consumer check** (`ext:repository`, [R9]/[R10]) producing
//!   `package-dependency-unsatisfied` (warning) and
//!   `package-dependency-label-mismatch` (info), for the repository and for a
//!   bundle before install;
//! - the `DependencyRef` **shape check** ([R1]/[R2]) over every package
//!   manifest the catalog found — non-fatal by [R9]/[R11];
//! - the **write path** for `packageDependencies` (srs-rust#1168): entries are
//!   keyed by `packageId`, labels come from the resolved installed package and
//!   are never guessed, and other entries (legacy ones included) are preserved
//!   verbatim.
//!
//! The pure rules (version rule, shape, Change D steps 1 to 4) live in
//! `srs_core::validation::package_dependency`. Nothing here ever fails a load.

use crate::error::RepositoryError;
use crate::package_types::PackageSelector;
use crate::store::RepositoryStore;
use crate::validation::DiagnosticSeverity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srs_core::types::package_dependency::{DependencyRef, SemVer};
use srs_core::validation::package_dependency::{
    check_entry, shape_violations, InstalledPackage, ShapeViolation, UnsatisfiedReason,
};

pub const CODE_UNSATISFIED: &str = "package-dependency-unsatisfied";
pub const CODE_LABEL_MISMATCH: &str = "package-dependency-label-mismatch";
/// A `DependencyRef` that breaks [R1]/[R2] (the schema form of the rule,
/// checked in Rust so it is non-fatal under either schema mirror).
pub const CODE_INVALID: &str = "package-dependency-invalid";
/// RFC-029 Change A: the core base package's namespace label.
pub const CORE_PACKAGE_NAMESPACE: &str = "com.semanticops.core";

/// One member of the installed set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMember {
    #[serde(flatten)]
    pub package: InstalledPackage,
    /// The `PackageRef.path` the member resolved from; `None` for the core
    /// package and for a member known only from `PackageRef` hints.
    pub selector: Option<String>,
    /// The member's own `packageDependencies` (lenient read).
    #[serde(skip)]
    pub dependencies: Vec<DependencyRef>,
}

/// One check outcome or shape problem, reported as a validation diagnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDependencyFinding {
    pub code: &'static str,
    pub severity: DiagnosticSeverity,
    /// The requiring package manifest (or `bundle`).
    pub path: String,
    /// The requiring package's own `id`.
    pub requiring_package_id: String,
    pub entry: DependencyRef,
    pub reason: Option<UnsatisfiedReason>,
    pub candidate_versions: Vec<Option<String>>,
    pub mismatched_labels: Vec<String>,
    pub message: String,
}

/// RFC-044 [R9]/[R11]: a package manifest value with `packageDependencies`
/// taken out, for the package-manifest schema validations (the catalog's
/// fatal [R4] anchor check and `repo validate`'s report). That one property is
/// shape-checked by [`shape_diagnostics`] instead, non-fatally and under the
/// RFC's rule whichever schema mirror is embedded; every other manifest
/// property keeps its schema check.
pub fn without_package_dependencies(manifest: &Value) -> std::borrow::Cow<'_, Value> {
    match manifest.as_object() {
        Some(obj) if obj.contains_key("packageDependencies") => {
            let mut v = manifest.clone();
            if let Some(o) = v.as_object_mut() {
                o.remove("packageDependencies");
            }
            std::borrow::Cow::Owned(v)
        }
        _ => std::borrow::Cow::Borrowed(manifest),
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty())
}

/// RFC-044 Change D items 1 to 5.
pub fn installed_set(store: &dyn RepositoryStore) -> Result<Vec<InstalledMember>, RepositoryError> {
    let manifest = store.load_manifest()?;
    // 1. `packageRefs` (when present) wins over the singular `packageRef`.
    let refs: Vec<Value> = match manifest.extra.get("packageRefs") {
        Some(v) if !v.is_null() => v.as_array().cloned().unwrap_or_default(),
        _ => manifest
            .extra
            .get("packageRef")
            .filter(|v| v.is_object())
            .cloned()
            .into_iter()
            .collect(),
    };
    let mut members = Vec::new();
    // 1b (srs-rust#1223). The primary package is always installed, exactly
    // as `FileStore::list_package_boundaries` (store.rs) always lists it
    // before the `packageRefs` sub-packages. Declared refs below are added
    // on top, deduped by resolved package id (so `packageRef {mode: local,
    // path: "package"}` does not count twice). A non-default primary path
    // depends on srs-rust#1207.
    // Note: the loader reads only the plural `packageRefs`; the singular
    // `packageRef` fallback in item 1 is this function's own (pre-dates
    // #1223) and is not loader behaviour.
    if let Ok(primary) = store.load_package_boundary(&None) {
        if !primary.id.is_empty() {
            members.push(InstalledMember {
                package: InstalledPackage {
                    id: primary.id,
                    namespace: primary.namespace,
                    name: primary.name,
                    version: Some(primary.version).filter(|v| !v.is_empty()),
                },
                selector: None,
                dependencies: primary
                    .package_dependencies
                    .unwrap_or_default()
                    .iter()
                    .map(DependencyRef::from_value_lenient)
                    .collect(),
            });
        }
    }
    for r in &refs {
        let path = str_of(r, "path");
        // A local ref resolves to its manifest; the manifest wins over the
        // ref's hints (item 2). A remote ref would resolve through a registry
        // this store has none of, so it takes the hint fallback below.
        let resolved = match (str_of(r, "mode"), path) {
            (Some("local"), Some(p)) => store
                .load_package_boundary(&Some(p.to_string()))
                .ok()
                .filter(|b| !b.id.is_empty()),
            _ => None,
        };
        if let Some(b) = resolved {
            // Only the primary dedupes: same-id/different-version refs are distinct members.
            if members
                .first()
                .is_some_and(|m| m.selector.is_none() && m.package.id == b.id)
            {
                continue;
            }
            members.push(InstalledMember {
                package: InstalledPackage {
                    id: b.id,
                    namespace: b.namespace,
                    name: b.name,
                    version: Some(b.version).filter(|v| !v.is_empty()),
                },
                selector: path.map(str::to_string),
                dependencies: b
                    .package_dependencies
                    .unwrap_or_default()
                    .iter()
                    .map(DependencyRef::from_value_lenient)
                    .collect(),
            });
        } else if let Some(id) = str_of(r, "packageId") {
            // Unresolvable: identified by the hint, labels unknown.
            members.push(InstalledMember {
                package: InstalledPackage {
                    id: id.to_string(),
                    namespace: String::new(),
                    name: String::new(),
                    version: str_of(r, "packageVersion").map(str::to_string),
                },
                selector: None,
                dependencies: Vec::new(),
            });
        }
    }
    // 4. The core base package is logically present (RFC-029 Change A), at
    // the version this build implements. 5. `upstreamPackage` contributes
    // nothing.
    let core = crate::core_package::core_package();
    members.push(InstalledMember {
        package: InstalledPackage {
            id: core.package_id.clone(),
            namespace: CORE_PACKAGE_NAMESPACE.to_string(),
            name: core.package_name.clone(),
            version: Some(core.package_version.clone()),
        },
        selector: None,
        dependencies: Vec::new(),
    });
    Ok(members)
}

fn fmt_entry(d: &DependencyRef) -> String {
    format!(
        "packageId {} ({}/{}) version {}",
        d.package_id.as_deref().unwrap_or("<absent>"),
        d.namespace,
        d.name,
        d.version
    )
}

fn manifest_path(selector: &str) -> String {
    if selector.is_empty() {
        "package.json".to_string()
    } else {
        format!("{}/package.json", selector.trim_end_matches('/'))
    }
}

fn check_requirements(
    requiring_id: &str,
    path: &str,
    entries: &[DependencyRef],
    members: &[InstalledPackage],
) -> Vec<PackageDependencyFinding> {
    let mut out = Vec::new();
    for d in entries {
        let o = check_entry(requiring_id, d, members);
        let finding = |code, severity, message| PackageDependencyFinding {
            code,
            severity,
            path: path.to_string(),
            requiring_package_id: requiring_id.to_string(),
            entry: d.clone(),
            reason: o.reason,
            candidate_versions: o.candidate_versions.clone(),
            mismatched_labels: o.mismatched_labels.clone(),
            message,
        };
        if let Some(reason) = o.reason {
            let versions: Vec<&str> = o
                .candidate_versions
                .iter()
                .map(|v| v.as_deref().unwrap_or("unknown"))
                .collect();
            out.push(finding(
                CODE_UNSATISFIED,
                DiagnosticSeverity::Warning,
                format!(
                    "{CODE_UNSATISFIED} (reason: {}): package {requiring_id} requires {}; installed candidate version(s): [{}]",
                    reason.as_str(),
                    fmt_entry(d),
                    versions.join(", ")
                ),
            ));
        }
        if !o.mismatched_labels.is_empty() {
            out.push(finding(
                CODE_LABEL_MISMATCH,
                DiagnosticSeverity::Info,
                format!(
                    "{CODE_LABEL_MISMATCH}: package {requiring_id} requires {}; the installed package carries label(s) {} (satisfaction is decided by packageId alone; the entry's labels are stale)",
                    fmt_entry(d),
                    o.mismatched_labels.join(", ")
                ),
            ));
        }
    }
    out
}

fn packages(members: &[InstalledMember]) -> Vec<InstalledPackage> {
    members.iter().map(|m| m.package.clone()).collect()
}

/// The repository-wide check ([R9]): every entry of every member's
/// `packageDependencies`, against the installed set.
pub fn check_repository(
    store: &dyn RepositoryStore,
) -> Result<Vec<PackageDependencyFinding>, RepositoryError> {
    let members = installed_set(store)?;
    let set = packages(&members);
    Ok(members
        .iter()
        .filter_map(|m| Some((m, m.selector.as_deref()?)))
        .flat_map(|(m, sel)| {
            check_requirements(&m.package.id, &manifest_path(sel), &m.dependencies, &set)
        })
        .collect())
}

/// The package-level requirement list a bundle declares
/// (`package-bundle.json` `packageId` + `packageDependencies`). A client
/// checking a requirement list that is not a package's (srs-web's
/// `EditorDefinition` requires, srs-web#340) omits `packageId`: an empty id
/// is never a candidate, so no entry is a self-requirement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRequirements {
    #[serde(default)]
    pub package_id: String,
    /// Read leniently: a malformed entry is reported, never refused ([R9]).
    #[serde(default, deserialize_with = "crate::package::lenient_dependency_refs")]
    pub package_dependencies: Vec<DependencyRef>,
}

/// The pre-install check (Change D, bundle paragraph): the bundle's own
/// entries against the target repository's installed set, each with its
/// outcome. Not recursive. Exposed as `srs package dependency check` (stdin),
/// the WASM `check_package_requirements` binding.
pub fn check_bundle(
    store: &dyn RepositoryStore,
    bundle: &BundleRequirements,
) -> Result<PackageDependenciesResult, RepositoryError> {
    let set = packages(&installed_set(store)?);
    Ok(PackageDependenciesResult {
        selector: None,
        package_id: bundle.package_id.clone(),
        action: None,
        dependencies: statuses(&bundle.package_id, &bundle.package_dependencies, &set),
    })
}

/// [R1]/[R2] over the `packageDependencies` of every package manifest at
/// `package_roots` (the catalog's list). Never fatal ([R9]).
///
/// Severity: an [R1]/[R2] violation is an error ([R11]), except a missing
/// `packageId`, which is a **warning in this release**. Reason: the release
/// corpus gate fails on any error, and every tracked corpus still carries
/// legacy entries that cannot be repaired until a binary that tolerates
/// `packageId` is published — reporting them as errors now would re-create
/// the RFC-043 publish deadlock (srs-rust#1145). The legacy entry is also
/// reported as `package-dependency-unsatisfied` (`no-package-id`). Raise it to
/// error with the package-manifest schema mirror sync, once the corpora are
/// repaired (RFC-044 landing order).
pub fn shape_diagnostics(
    store: &dyn RepositoryStore,
    package_roots: &[String],
) -> Vec<PackageDependencyFinding> {
    let mut out = Vec::new();
    for root in package_roots {
        let path = manifest_path(root);
        let Ok(v) = store.load_instance_json(&path) else {
            continue;
        };
        let requiring_id = str_of(&v, "id").unwrap_or_default().to_string();
        let finding = |severity, entry: DependencyRef, message: String| PackageDependencyFinding {
            code: CODE_INVALID,
            severity,
            path: path.clone(),
            requiring_package_id: requiring_id.clone(),
            entry,
            reason: None,
            candidate_versions: Vec::new(),
            mismatched_labels: Vec::new(),
            message: format!("{CODE_INVALID}: {message}"),
        };
        match v.get("packageDependencies") {
            None => {}
            Some(Value::Array(entries)) => {
                for (i, e) in entries.iter().enumerate() {
                    let d = DependencyRef::from_value_lenient(e);
                    for sv in shape_violations(e) {
                        out.push(match sv {
                            ShapeViolation::MissingPackageId => finding(
                                DiagnosticSeverity::Warning,
                                d.clone(),
                                format!(
                                    "packageDependencies[{i}] ({}/{}@{}) has no packageId (RFC-044 [R1]/[R11]); repair it with `srs package dependency add --package-id <uuid> --version <semver>`",
                                    d.namespace, d.name, d.version
                                ),
                            ),
                            ShapeViolation::Invalid(m) => finding(
                                DiagnosticSeverity::Error,
                                d.clone(),
                                format!("packageDependencies[{i}]: {m} (RFC-044 [R1]/[R2])"),
                            ),
                        });
                    }
                }
            }
            Some(_) => out.push(finding(
                DiagnosticSeverity::Error,
                DependencyRef::default(),
                "packageDependencies is not an array".to_string(),
            )),
        }
    }
    out
}

// ── Write path (srs-rust#1168) ───────────────────────────────────────────────

/// Input for `srs package dependency add`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddPackageDependencyInput {
    /// The requiring package boundary (`None` = primary package).
    pub selector: PackageSelector,
    pub package_id: String,
    pub version: String,
    /// Replace the legacy entry (no `packageId`) whose `namespace`/`name`
    /// equal the resolved package's labels exactly. The caller supplies the
    /// id and opts in; the labels only locate which legacy entry the caller
    /// means. Without it, such an entry is never rewritten ([R11]: no writer
    /// supplies a missing `packageId` by matching labels) and the add is
    /// refused so the requirement is not declared twice.
    #[serde(default)]
    pub repair_legacy: bool,
}

/// Input for `srs package dependency remove`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovePackageDependencyInput {
    pub selector: PackageSelector,
    pub package_id: String,
}

/// What a write did to the entry list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DependencyWriteAction {
    /// A new entry was appended.
    Added,
    /// The entry with this `packageId` was replaced.
    Updated,
    /// On the caller's explicit `repair_legacy`: the legacy entry (no
    /// `packageId`) whose labels equal the resolved package's labels was
    /// replaced by the caller-supplied `packageId`.
    Repaired,
    /// Every entry with this `packageId` was removed.
    Removed,
}

/// One entry with its check outcome.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDependencyStatus {
    #[serde(flatten)]
    pub entry: DependencyRef,
    pub satisfied: bool,
    pub reason: Option<UnsatisfiedReason>,
    pub candidate_versions: Vec<Option<String>>,
    pub mismatched_labels: Vec<String>,
}

/// Result of list/add/remove: the boundary's entries after the operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageDependenciesResult {
    pub selector: PackageSelector,
    /// The requiring package's own `id`.
    pub package_id: String,
    pub action: Option<DependencyWriteAction>,
    pub dependencies: Vec<PackageDependencyStatus>,
}

fn lenient(raw: &[Value]) -> Vec<DependencyRef> {
    raw.iter().map(DependencyRef::from_value_lenient).collect()
}

fn statuses(
    requiring_id: &str,
    entries: &[DependencyRef],
    members: &[InstalledPackage],
) -> Vec<PackageDependencyStatus> {
    entries
        .iter()
        .map(|entry| {
            let o = check_entry(requiring_id, entry, members);
            PackageDependencyStatus {
                entry: entry.clone(),
                satisfied: o.reason.is_none(),
                reason: o.reason,
                candidate_versions: o.candidate_versions,
                mismatched_labels: o.mismatched_labels,
            }
        })
        .collect()
}

/// List one boundary's `packageDependencies` with each entry's outcome.
pub fn list_package_dependencies(
    store: &dyn RepositoryStore,
    selector: PackageSelector,
) -> Result<PackageDependenciesResult, RepositoryError> {
    let b = store.load_package_boundary(&selector)?;
    let set = packages(&installed_set(store)?);
    Ok(PackageDependenciesResult {
        dependencies: statuses(
            &b.id,
            &lenient(&b.package_dependencies.unwrap_or_default()),
            &set,
        ),
        selector,
        package_id: b.id,
        action: None,
    })
}

fn invalid(message: String) -> RepositoryError {
    RepositoryError::InvalidInput { message }
}

/// Add (or replace) the requirement on `package_id`. Labels are filled from
/// the installed package with that id; an id that resolves to no installed
/// package is refused ([R11]: never guessed). A legacy entry is rewritten
/// only on the caller's explicit `repair_legacy` (see that field).
pub fn add_package_dependency(
    store: &dyn RepositoryStore,
    input: AddPackageDependencyInput,
) -> Result<PackageDependenciesResult, RepositoryError> {
    let pid = input.package_id.trim().to_ascii_lowercase();
    if pid.len() != 36 || uuid::Uuid::parse_str(&pid).is_err() {
        return Err(invalid(format!(
            "packageId '{}' is not a UUID",
            input.package_id
        )));
    }
    if SemVer::parse(&input.version).is_none() {
        return Err(invalid(format!(
            "version '{}' is not a SemVer 2.0.0 version (RFC-044 [R2])",
            input.version
        )));
    }
    let mut b = store.load_package_boundary(&input.selector)?;
    if pid == b.id {
        return Err(invalid(format!(
            "package {pid} cannot require itself (RFC-044 Change D, self-requirement)"
        )));
    }
    let set = packages(&installed_set(store)?);
    let target = set
        .iter()
        .find(|m| m.id == pid && !(m.namespace.is_empty() && m.name.is_empty()))
        .ok_or_else(|| {
            invalid(format!(
                "packageId {pid} does not resolve to an installed package, so its labels cannot be filled (RFC-044 [R11]: never guessed); install or register the package first"
            ))
        })?;
    let new_entry = serde_json::json!({
        "packageId": pid,
        "namespace": target.namespace,
        "name": target.name,
        "version": input.version,
    });
    let mut raw = b.package_dependencies.take().unwrap_or_default();
    let views: Vec<DependencyRef> = raw.iter().map(DependencyRef::from_value_lenient).collect();
    let action = if let Some(i) = views
        .iter()
        .position(|d| d.package_id.as_deref() == Some(pid.as_str()))
    {
        raw[i] = new_entry;
        DependencyWriteAction::Updated
    } else {
        let legacy = views.iter().position(|d| {
            d.package_id.is_none() && d.namespace == target.namespace && d.name == target.name
        });
        match (legacy, input.repair_legacy) {
            (Some(i), true) => {
                raw[i] = new_entry;
                DependencyWriteAction::Repaired
            }
            (None, true) => {
                return Err(invalid(format!(
                    "repair requested, but package {} has no legacy packageDependencies entry (no packageId) labelled {}/{}",
                    b.id, target.namespace, target.name
                )))
            }
            (Some(_), false) => {
                return Err(invalid(format!(
                    "package {} has a legacy packageDependencies entry (no packageId) labelled {}/{}; pass repair_legacy (CLI --repair-legacy) to replace it with packageId {pid}, or remove it by hand (RFC-044 [R11]: a missing packageId is never supplied by matching labels)",
                    b.id, target.namespace, target.name
                )))
            }
            (None, false) => {
                raw.push(new_entry);
                DependencyWriteAction::Added
            }
        }
    };
    b.package_dependencies = Some(raw);
    store.save_package_boundary_metadata(&b)?;
    Ok(PackageDependenciesResult {
        dependencies: statuses(
            &b.id,
            &lenient(b.package_dependencies.as_deref().unwrap_or(&[])),
            &set,
        ),
        selector: input.selector,
        package_id: b.id,
        action: Some(action),
    })
}

/// Remove every entry requiring `package_id`. Refused when there is none.
pub fn remove_package_dependency(
    store: &dyn RepositoryStore,
    input: RemovePackageDependencyInput,
) -> Result<PackageDependenciesResult, RepositoryError> {
    let pid = input.package_id.trim().to_ascii_lowercase();
    let mut b = store.load_package_boundary(&input.selector)?;
    let mut raw = b.package_dependencies.take().unwrap_or_default();
    let before = raw.len();
    raw.retain(|e| {
        DependencyRef::from_value_lenient(e).package_id.as_deref() != Some(pid.as_str())
    });
    if raw.len() == before {
        return Err(invalid(format!(
            "package {} has no packageDependencies entry with packageId {pid}",
            b.id
        )));
    }
    b.package_dependencies = Some(raw);
    store.save_package_boundary_metadata(&b)?;
    let set = packages(&installed_set(store)?);
    Ok(PackageDependenciesResult {
        dependencies: statuses(
            &b.id,
            &lenient(b.package_dependencies.as_deref().unwrap_or(&[])),
            &set,
        ),
        selector: input.selector,
        package_id: b.id,
        action: Some(DependencyWriteAction::Removed),
    })
}
