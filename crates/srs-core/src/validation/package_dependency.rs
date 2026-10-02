//! RFC-044 package requirement rules as pure functions: the version rule
//! ([R3], Change B), the `DependencyRef` shape ([R1]/[R2]) and the per-entry
//! consumer check (Change D steps 1 to 4, [R10]). File I/O and the installed
//! set live in `srs-repository::package_dependency_service`.

use crate::types::package_dependency::{DependencyRef, SemVer};
use serde::Serialize;
use std::cmp::Ordering;

/// The first Change B clause an installed version fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionClause {
    /// Clause 1: outside the requirement's compatibility band.
    Incompatible,
    /// Clause 2: a pre-release of a different release.
    PrereleaseExcluded,
    /// Clause 3: lower SemVer precedence than the requirement.
    VersionTooLow,
}

/// Clause 1: is `i` inside `r`'s compatibility band?
pub fn in_band(i: &SemVer, r: &SemVer) -> bool {
    if r.major >= 1 {
        i.major == r.major
    } else if r.minor >= 1 {
        i.major == 0 && i.minor == r.minor
    } else {
        i.major == 0 && i.minor == 0 && i.patch == r.patch
    }
}

/// [R3]: does installed `i` satisfy requirement `r`? `Err` names the first
/// clause it fails, in Change B's evaluation order.
pub fn satisfies(i: &SemVer, r: &SemVer) -> Result<(), VersionClause> {
    if !in_band(i, r) {
        return Err(VersionClause::Incompatible);
    }
    if !i.pre.is_empty() && i.release() != r.release() {
        return Err(VersionClause::PrereleaseExcluded);
    }
    if i.precedence(r) == Ordering::Less {
        return Err(VersionClause::VersionTooLow);
    }
    Ok(())
}

/// String form of [`satisfies`]; an unparseable side never satisfies.
pub fn version_satisfies(installed: &str, required: &str) -> bool {
    match (SemVer::parse(installed), SemVer::parse(required)) {
        (Some(i), Some(r)) => satisfies(&i, &r).is_ok(),
        _ => false,
    }
}

/// One way a raw `DependencyRef` value breaks [R1]/[R2].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeViolation {
    /// [R1]/[R11]: a legacy entry without `packageId`.
    MissingPackageId,
    /// Any other [R1]/[R2] violation, described.
    Invalid(String),
}

const DEPENDENCY_REF_KEYS: &[&str] = &["packageId", "namespace", "name", "version"];

/// Check one raw `packageDependencies` entry against [R1] and [R2].
pub fn shape_violations(v: &serde_json::Value) -> Vec<ShapeViolation> {
    let Some(obj) = v.as_object() else {
        return vec![ShapeViolation::Invalid(
            "entry is not an object".to_string(),
        )];
    };
    let mut out = Vec::new();
    for k in obj.keys() {
        if !DEPENDENCY_REF_KEYS.contains(&k.as_str()) {
            out.push(ShapeViolation::Invalid(format!(
                "unexpected property '{k}' (a DependencyRef has only packageId, namespace, name, version)"
            )));
        }
    }
    match obj.get("packageId") {
        None => out.push(ShapeViolation::MissingPackageId),
        Some(p) => {
            let ok = p
                .as_str()
                .is_some_and(|s| s.len() == 36 && uuid::Uuid::parse_str(s).is_ok());
            if !ok {
                out.push(ShapeViolation::Invalid(format!(
                    "packageId {p} is not a UUID"
                )));
            }
        }
    }
    for k in ["namespace", "name"] {
        if !obj.get(k).is_some_and(|x| x.is_string()) {
            out.push(ShapeViolation::Invalid(format!(
                "'{k}' is required and must be a string"
            )));
        }
    }
    match obj.get("version").and_then(|x| x.as_str()) {
        None => out.push(ShapeViolation::Invalid(
            "'version' is required and must be a string".to_string(),
        )),
        Some(s) if SemVer::parse(s).is_none() => out.push(ShapeViolation::Invalid(format!(
            "version '{s}' is not a SemVer 2.0.0 version"
        ))),
        Some(_) => {}
    }
    out
}

/// The closed reason list of `package-dependency-unsatisfied` (Change D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnsatisfiedReason {
    NoPackageId,
    SelfRequirement,
    Missing,
    VersionUnknown,
    Incompatible,
    PrereleaseExcluded,
    VersionTooLow,
}

impl UnsatisfiedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoPackageId => "no-package-id",
            Self::SelfRequirement => "self-requirement",
            Self::Missing => "missing",
            Self::VersionUnknown => "version-unknown",
            Self::Incompatible => "incompatible",
            Self::PrereleaseExcluded => "prerelease-excluded",
            Self::VersionTooLow => "version-too-low",
        }
    }
}

impl From<VersionClause> for UnsatisfiedReason {
    fn from(c: VersionClause) -> Self {
        match c {
            VersionClause::Incompatible => Self::Incompatible,
            VersionClause::PrereleaseExcluded => Self::PrereleaseExcluded,
            VersionClause::VersionTooLow => Self::VersionTooLow,
        }
    }
}

/// One member of an installed set (Change D items 1 to 5), as the check sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackage {
    pub id: String,
    pub namespace: String,
    pub name: String,
    /// `None` = unknown version.
    pub version: Option<String>,
}

/// The outcome of checking one entry.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryOutcome {
    /// `None` = satisfied.
    pub reason: Option<UnsatisfiedReason>,
    /// Installed version of each candidate (members with the entry's
    /// `packageId`), in installed-set order; `None` = unknown.
    pub candidate_versions: Vec<Option<String>>,
    /// `namespace/name` of every candidate whose labels differ from the
    /// entry's, deduplicated ([R10] `package-dependency-label-mismatch`).
    pub mismatched_labels: Vec<String>,
}

/// Change D steps 1 to 4, in order, for one entry `d` of the package whose
/// own id is `requiring_id`, against the installed set `members`.
pub fn check_entry(
    requiring_id: &str,
    d: &DependencyRef,
    members: &[InstalledPackage],
) -> EntryOutcome {
    let unsat = |reason| EntryOutcome {
        reason: Some(reason),
        ..Default::default()
    };
    // 1. Labels are never used to find a match.
    let Some(pid) = d.package_id.as_deref() else {
        return unsat(UnsatisfiedReason::NoPackageId);
    };
    // 2.
    if pid == requiring_id {
        return unsat(UnsatisfiedReason::SelfRequirement);
    }
    // 3.
    let candidates: Vec<&InstalledPackage> = members.iter().filter(|m| m.id == pid).collect();
    if candidates.is_empty() {
        return unsat(UnsatisfiedReason::Missing);
    }
    let mut mismatched_labels: Vec<String> = Vec::new();
    for c in &candidates {
        // A member known only from `PackageRef` hints has no labels to compare.
        let labelled = !(c.namespace.is_empty() && c.name.is_empty());
        if labelled && (c.namespace != d.namespace || c.name != d.name) {
            let label = format!("{}/{}", c.namespace, c.name);
            if !mismatched_labels.contains(&label) {
                mismatched_labels.push(label);
            }
        }
    }
    let mut out = EntryOutcome {
        reason: None,
        candidate_versions: candidates.iter().map(|c| c.version.clone()).collect(),
        mismatched_labels,
    };
    // 4.
    let known: Vec<SemVer> = candidates
        .iter()
        .filter_map(|c| c.version.as_deref().and_then(SemVer::parse))
        .collect();
    if known.is_empty() {
        out.reason = Some(UnsatisfiedReason::VersionUnknown);
        return out;
    }
    let Some(r) = SemVer::parse(&d.version) else {
        // [R2] violation (reported by shape_violations): no candidate can be
        // inside the band of a version that has none.
        out.reason = Some(UnsatisfiedReason::Incompatible);
        return out;
    };
    if known.iter().any(|i| satisfies(i, &r).is_ok()) {
        return out;
    }
    let chosen = known
        .iter()
        .filter(|i| in_band(i, &r))
        .max_by(|a, b| a.precedence(b))
        .or_else(|| known.iter().max_by(|a, b| a.precedence(b)))
        .expect("known is non-empty");
    out.reason = satisfies(chosen, &r).err().map(UnsatisfiedReason::from);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SemVer {
        SemVer::parse(s).unwrap_or_else(|| panic!("{s} should parse"))
    }
    fn sat(i: &str, r: &str) -> Result<(), VersionClause> {
        satisfies(&v(i), &v(r))
    }
    use VersionClause::*;

    #[test]
    fn semver_pattern_fixtures() {
        for ok in [
            "1.2.0",
            "0.1.0",
            "1.0.0-rc.1",
            "1.0.0-rc.1+b5",
            "1.0.0-alpha-1",
            "0.0.0",
            "1.0.0+001",
        ] {
            assert!(SemVer::parse(ok).is_some(), "{ok}");
        }
        for bad in [
            "1.0",
            "01.0.0",
            "v1.0.0",
            "1.0.0-01",
            "1.0.0-",
            "1.0.0+",
            "1.0.0-a..b",
            "1.0.0.0",
            "",
            "1.0.0+b!",
        ] {
            assert!(SemVer::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn precedence_follows_semver_section_11() {
        let order = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "2.0.0",
            "2.1.0",
            "2.1.1",
        ];
        for w in order.windows(2) {
            assert_eq!(
                v(w[0]).precedence(&v(w[1])),
                Ordering::Less,
                "{} < {}",
                w[0],
                w[1]
            );
        }
        assert_eq!(v("1.0.0+a").precedence(&v("1.0.0+b")), Ordering::Equal);
    }

    // RFC-044 Change B worked cases, verbatim.
    #[test]
    fn worked_cases_r_1_2_0() {
        assert_eq!(sat("1.2.0", "1.2.0"), Ok(()));
        assert_eq!(sat("1.3.5", "1.2.0"), Ok(()));
        assert_eq!(sat("1.2.0+b7", "1.2.0"), Ok(()));
        assert_eq!(sat("1.1.9", "1.2.0"), Err(VersionTooLow));
        assert_eq!(sat("2.0.0", "1.2.0"), Err(Incompatible));
        assert_eq!(sat("1.3.0-rc.1", "1.2.0"), Err(PrereleaseExcluded));
        assert_eq!(sat("1.2.0-rc.1", "1.2.0"), Err(VersionTooLow));
    }

    #[test]
    fn worked_cases_prerelease_requirement() {
        assert_eq!(sat("1.2.0-rc.1", "1.2.0-rc.1"), Ok(()));
        assert_eq!(sat("1.2.0-rc.2", "1.2.0-rc.1"), Ok(()));
        assert_eq!(sat("1.2.0", "1.2.0-rc.1"), Ok(()));
        assert_eq!(sat("1.2.1-rc.1", "1.2.0-rc.1"), Err(PrereleaseExcluded));
    }

    #[test]
    fn worked_cases_zero_minor_band() {
        assert_eq!(sat("0.1.0", "0.1.0"), Ok(()));
        assert_eq!(sat("0.1.7", "0.1.0"), Ok(()));
        assert_eq!(sat("0.2.0", "0.1.0"), Err(Incompatible));
        assert_eq!(sat("1.0.0", "0.1.0"), Err(Incompatible));
    }

    #[test]
    fn worked_cases_zero_zero_patch_band() {
        assert_eq!(sat("0.0.3", "0.0.3"), Ok(()));
        assert_eq!(sat("0.0.3+b1", "0.0.3"), Ok(()));
        assert_eq!(sat("0.0.4", "0.0.3"), Err(Incompatible));
        assert_eq!(sat("0.0.3-rc.1", "0.0.3"), Err(VersionTooLow));
        assert_eq!(sat("0.0.3-rc.1", "0.0.3-rc.1"), Ok(()));
        assert_eq!(sat("0.0.3-rc.2", "0.0.3-rc.1"), Ok(()));
        assert_eq!(sat("0.0.3", "0.0.3-rc.1"), Ok(()));
        assert_eq!(sat("0.0.0", "0.0.0"), Ok(()));
        assert_eq!(sat("0.0.1", "0.0.0"), Err(Incompatible));
    }

    #[test]
    fn version_satisfies_rejects_unparseable() {
        assert!(version_satisfies("3.0.0", "3.0.0"));
        assert!(!version_satisfies("3.0", "3.0.0"));
        assert!(!version_satisfies("3.0.0", "v3"));
    }

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn shape_rules() {
        let good = json(
            r#"{"packageId":"4a000001-0000-4000-a000-000000000001","namespace":"a","name":"b","version":"3.0.0"}"#,
        );
        assert!(shape_violations(&good).is_empty());
        let legacy = json(r#"{"namespace":"a","name":"b","version":"1.0.0"}"#);
        assert_eq!(
            shape_violations(&legacy),
            vec![ShapeViolation::MissingPackageId]
        );
        let bad = json(r#"{"packageId":"nope","name":3,"version":"1.0","extra":true}"#);
        let got = shape_violations(&bad);
        assert_eq!(got.len(), 5, "{got:?}"); // extra, packageId, namespace, name, version
        assert!(!got.contains(&ShapeViolation::MissingPackageId));
        assert_eq!(shape_violations(&json("3")).len(), 1);
    }

    const G: &str = "1cd9622e-3d05-4214-a683-4cb81d0c44d9";
    const A: &str = "126eeba4-3124-4ee6-a9e5-5a943cb87cb5";

    fn member(id: &str, ns: &str, name: &str, ver: Option<&str>) -> InstalledPackage {
        InstalledPackage {
            id: id.into(),
            namespace: ns.into(),
            name: name.into(),
            version: ver.map(str::to_string),
        }
    }
    fn gov(ver: &str) -> InstalledPackage {
        member(G, "com.mudemocracy.governance", "governance", Some(ver))
    }
    fn dep(pid: Option<&str>, ver: &str) -> DependencyRef {
        DependencyRef {
            package_id: pid.map(str::to_string),
            namespace: "com.mudemocracy.governance".into(),
            name: "governance".into(),
            version: ver.into(),
        }
    }
    fn reason(members: &[InstalledPackage], d: &DependencyRef) -> Option<UnsatisfiedReason> {
        check_entry(A, d, members).reason
    }
    use UnsatisfiedReason as U;

    #[test]
    fn check_steps_one_to_three() {
        let set = [gov("1.0.0")];
        assert_eq!(reason(&set, &dep(None, "1.0.0")), Some(U::NoPackageId));
        assert_eq!(
            reason(&set, &dep(Some(A), "1.0.0")),
            Some(U::SelfRequirement)
        );
        assert_eq!(reason(&[], &dep(Some(G), "1.0.0")), Some(U::Missing));
        // Labels never match: same labels, different id → missing.
        let impostor = [member(
            "00000000-0000-4000-a000-000000000000",
            "com.mudemocracy.governance",
            "governance",
            Some("1.0.0"),
        )];
        assert_eq!(reason(&impostor, &dep(Some(G), "1.0.0")), Some(U::Missing));
        assert_eq!(reason(&set, &dep(Some(G), "1.0.0")), None);
    }

    #[test]
    fn self_requirement_even_when_installed() {
        // Rule 2 fires whichever version of the requiring package is installed.
        let set = [gov("1.0.0"), gov("2.0.0")];
        assert_eq!(
            check_entry(G, &dep(Some(G), "1.0.0"), &set).reason,
            Some(U::SelfRequirement)
        );
    }

    #[test]
    fn version_unknown_when_every_candidate_is_unknown() {
        let set = [
            member(G, "com.mudemocracy.governance", "governance", None),
            member(
                G,
                "com.mudemocracy.governance",
                "governance",
                Some("not-semver"),
            ),
        ];
        let out = check_entry(A, &dep(Some(G), "1.0.0"), &set);
        assert_eq!(out.reason, Some(U::VersionUnknown));
        assert_eq!(
            out.candidate_versions,
            vec![None, Some("not-semver".to_string())]
        );
        // An unknown candidate is never chosen while a known one exists.
        let set = [
            member(G, "com.mudemocracy.governance", "governance", None),
            gov("1.0.0"),
        ];
        assert_eq!(reason(&set, &dep(Some(G), "1.1.0")), Some(U::VersionTooLow));
    }

    // RFC-044 Change D worked example (two installed versions), verbatim.
    #[test]
    fn worked_example_multi_version() {
        let set = [
            gov("1.0.0"),
            gov("1.2.1"),
            member(A, "com.mudemocracy.argument", "argument", Some("1.3.0")),
        ];
        assert_eq!(reason(&set, &dep(Some(G), "1.1.0")), None);
        assert_eq!(reason(&set, &dep(Some(G), "2.0.0")), Some(U::Incompatible));
        assert_eq!(reason(&set, &dep(Some(G), "1.3.0")), Some(U::VersionTooLow));
        let set = [gov("1.0.0"), gov("2.0.0")];
        assert_eq!(reason(&set, &dep(Some(G), "1.5.0")), Some(U::VersionTooLow));
    }

    #[test]
    fn prerelease_excluded_reason() {
        assert_eq!(
            reason(&[gov("1.3.0-rc.1")], &dep(Some(G), "1.2.0")),
            Some(U::PrereleaseExcluded)
        );
    }

    #[test]
    fn label_mismatch_reported_but_never_decides() {
        let renamed = [member(
            G,
            "com.mudemocracy.gov2",
            "governance",
            Some("1.0.0"),
        )];
        let out = check_entry(A, &dep(Some(G), "1.0.0"), &renamed);
        assert_eq!(out.reason, None);
        assert_eq!(
            out.mismatched_labels,
            vec!["com.mudemocracy.gov2/governance".to_string()]
        );
        assert!(check_entry(A, &dep(Some(G), "1.0.0"), &[gov("1.0.0")])
            .mismatched_labels
            .is_empty());
    }

    #[test]
    fn reason_names_are_the_closed_list() {
        let all = [
            U::NoPackageId,
            U::SelfRequirement,
            U::Missing,
            U::VersionUnknown,
            U::Incompatible,
            U::PrereleaseExcluded,
            U::VersionTooLow,
        ];
        let names: Vec<String> = all
            .iter()
            .map(|r| {
                serde_json::to_value(r)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            [
                "no-package-id",
                "self-requirement",
                "missing",
                "version-unknown",
                "incompatible",
                "prerelease-excluded",
                "version-too-low"
            ]
        );
        for (r, n) in all.iter().zip(&names) {
            assert_eq!(r.as_str(), n);
        }
    }

    #[test]
    fn legacy_entry_deserializes() {
        let d: DependencyRef =
            serde_json::from_str(r#"{"namespace":"a","name":"b","version":"1.0.0"}"#).unwrap();
        assert_eq!(d.package_id, None);
        let lenient =
            DependencyRef::from_value_lenient(&json(r#"{"packageId":3,"version":["x"]}"#));
        assert_eq!(lenient, DependencyRef::default());
    }
}
