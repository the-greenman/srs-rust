//! RFC-044 package requirement identity: `DependencyRef` and the SemVer 2.0.0
//! version value the version rule ([R3]) compares.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// One entry of a package manifest's `packageDependencies` (RFC-044 Change A).
///
/// `package_id` is the key of the requirement; `namespace` and `name` are
/// display labels and never decide satisfaction ([R1], [R10]). Deserialization
/// is deliberately tolerant: a legacy entry without `packageId` (or with a
/// missing label) still reads, because a `DependencyRef` that fails the schema
/// must never fail a repository load ([R9], [R11]). Shape problems are reported
/// by `validation::package_dependency::shape_violations`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
}

impl DependencyRef {
    /// Read an entry from raw JSON without ever failing: non-string or absent
    /// properties read as absent/empty. The writer keeps the raw value; this
    /// view is only for checking ([R11]: a legacy entry is preserved unchanged).
    pub fn from_value_lenient(v: &serde_json::Value) -> Self {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        DependencyRef {
            package_id: s("packageId").filter(|p| !p.is_empty()),
            namespace: s("namespace").unwrap_or_default(),
            name: s("name").unwrap_or_default(),
            version: s("version").unwrap_or_default(),
        }
    }
}

/// A parsed SemVer 2.0.0 version. Build metadata is validated and dropped: it
/// never takes part in comparison (RFC-044 Change B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    /// Pre-release identifiers; empty for a release.
    pub pre: Vec<String>,
}

fn is_numeric_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_ident_chars(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn parse_core_part(s: &str) -> Option<u64> {
    if !is_numeric_id(s) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

impl SemVer {
    /// Parse with exactly the acceptance of the official SemVer 2.0.0 regex
    /// RFC-044 Change A quotes (anchored): accepts `1.0.0-rc.1+b5`, rejects
    /// `1.0`, `01.0.0`, `v1.0.0` and `1.0.0-01`.
    pub fn parse(s: &str) -> Option<SemVer> {
        let (rest, build) = match s.split_once('+') {
            Some((r, b)) => (r, Some(b)),
            None => (s, None),
        };
        if let Some(b) = build {
            if !b.split('.').all(is_ident_chars) {
                return None;
            }
        }
        let (core, pre) = match rest.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (rest, None),
        };
        let mut parts = core.split('.');
        let major = parse_core_part(parts.next()?)?;
        let minor = parse_core_part(parts.next()?)?;
        let patch = parse_core_part(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        let pre: Vec<String> = match pre {
            None => Vec::new(),
            Some(p) => {
                let ids: Vec<&str> = p.split('.').collect();
                for id in &ids {
                    if !is_ident_chars(id)
                        || (is_numeric_id(id) && id.len() > 1 && id.starts_with('0'))
                    {
                        return None;
                    }
                }
                ids.into_iter().map(str::to_string).collect()
            }
        };
        Some(SemVer {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// SemVer 2.0.0 section 11 precedence (build metadata already dropped).
    pub fn precedence(&self, other: &SemVer) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let ord = match (is_numeric_id(a), is_numeric_id(b)) {
                            // No leading zeros, so longer means larger.
                            (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
                            (true, false) => Ordering::Less,
                            (false, true) => Ordering::Greater,
                            (false, false) => a.cmp(b),
                        };
                        if ord != Ordering::Equal {
                            return ord;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            })
    }

    pub(crate) fn release(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}
