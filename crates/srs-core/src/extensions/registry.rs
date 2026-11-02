//! `ext:registry` (RFC-049): one registry document whose entries are kinded —
//! `package` (the catalog entry this extension always had) and `repository`.
//! The document keeps every entry and field it does not understand ([R9]), so
//! entries are held raw and typed only when loaded.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The registry shape this implementation reads and writes ([R9]).
pub const REGISTRY_REVISION: u64 = 1;

/// A LOCATOR reference (`rfc-decision-c8704763`). `bundled` is not valid in a
/// registry, which carries no artifacts ([R3]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Locator {
    Local { path: String },
    Remote { url: String },
}

/// A `package` entry ([R14]): today's catalog entry with `downloadUrl` replaced
/// by `locator`. Identity is `packageId` + `packageVersion`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageEntry {
    pub locator: Locator,
    pub package_id: String,
    pub package_name: String,
    pub package_version: String,
    pub publisher: String,
    pub published_at: String,
    pub field_count: u32,
    pub type_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation_type_count: Option<u32>,
    /// SHA-256 hex digest for integrity verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// A `repository` entry ([R14]): identity is `id`, the repository's
/// `manifest.repositoryId`. `readOnly` absent means `false` ([R16]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryEntry {
    pub locator: Locator,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

// A handful of entries per registry, held briefly: the size gap costs nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RegistryEntry {
    Package(PackageEntry),
    Repository(RepositoryEntry),
}

/// The registry document. Catalog fields are optional and resolve nothing;
/// `entries` and unknown top-level properties are kept verbatim ([R9]). A catalog
/// field of the wrong type reads as absent and is kept, unchanged, in `extra`.
///
/// ponytail: numbers are held as f64/i64/u64 (no `arbitrary_precision`), so a number
/// in an unknown field or entry beyond f64 precision changes on rewrite; enable
/// serde_json's `arbitrary_precision` if a registry ever needs exact large numbers.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry_name: Option<String>,
    /// Registry's own version (semver).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub entries: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// A per-entry load outcome ([R15]); `index` is the entry's position in `entries`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryOutcome {
    pub index: usize,
    pub code: &'static str,
    pub message: String,
}

/// Why a whole registry document is refused, loading nothing ([R15]).
#[derive(Debug, Clone, PartialEq)]
pub enum RegistryRefusal {
    /// `registry-malformed`: not a JSON object with an `entries` array.
    Malformed(String),
    /// `registry-revision-unsupported`: stamped above the supported revision, or
    /// with a stamp that is not a non-negative integer.
    RevisionUnsupported { found: String },
}

/// A loaded registry: the document (brought to [`REGISTRY_REVISION`]), its usable
/// entries in document order, and the outcome of every entry that is not usable.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LoadedRegistry {
    pub registry: Registry,
    pub entries: Vec<RegistryEntry>,
    pub outcomes: Vec<EntryOutcome>,
}

/// Identity fields per supported kind, in [R4] order; `true` marks a UUID,
/// compared without regard to case.
fn identity_fields(kind: &str) -> Option<&'static [(&'static str, bool)]> {
    match kind {
        "package" => Some(&[("packageId", true), ("packageVersion", false)]),
        "repository" => Some(&[("id", true)]),
        _ => None,
    }
}

/// The 0 → 1 registry-form transformer ([R9]): an entry without `kind` is a
/// pre-RFC package entry, and its `downloadUrl` becomes a remote `locator`.
fn migrate_0_to_1(entries: &mut [Value]) {
    for entry in entries.iter_mut().filter_map(Value::as_object_mut) {
        if entry.contains_key("kind") {
            continue;
        }
        entry.insert("kind".into(), Value::from("package"));
        if let Some(url) = entry.remove("downloadUrl") {
            let mut locator = Map::new();
            locator.insert("mode".into(), Value::from("remote"));
            locator.insert("url".into(), url);
            entry.insert("locator".into(), Value::Object(locator));
        }
    }
}

/// A UUID in the 36-character hyphenated text form (any case).
fn is_uuid_text(s: &str) -> bool {
    s.len() == 36 && uuid::Uuid::parse_str(s).is_ok()
}

/// Catalog fields typed as strings on [`Registry`].
const CATALOG_FIELDS: [&str; 6] = [
    "schemaVersion",
    "registryId",
    "registryName",
    "catalogVersion",
    "updatedAt",
    "homepage",
];

fn type_entry(raw: &Value) -> Result<RegistryEntry, (&'static str, String)> {
    let kind = raw.get("kind").and_then(Value::as_str);
    match kind {
        None => Err(("entry-malformed", "entry has no string `kind`".into())),
        Some(k) if identity_fields(k).is_none() => {
            Err(("kind-unsupported", format!("kind '{k}' is not supported")))
        }
        Some(_) => {
            let entry: RegistryEntry = serde_json::from_value(raw.clone())
                .map_err(|e| ("entry-malformed", e.to_string()))?;
            let (id, locator) = match &entry {
                RegistryEntry::Package(p) => (&p.package_id, &p.locator),
                RegistryEntry::Repository(r) => (&r.id, &r.locator),
            };
            if !is_uuid_text(id) {
                return Err(("entry-malformed", format!("identity '{id}' is not a UUID")));
            }
            if locator_empty(locator) {
                return Err(("entry-malformed", "empty locator".into()));
            }
            Ok(entry)
        }
    }
}

fn locator_empty(l: &Locator) -> bool {
    match l {
        Locator::Local { path } => path.is_empty(),
        Locator::Remote { url } => url.is_empty(),
    }
}

/// The identity key an entry carries, if it is of a supported kind and carries
/// every identity field — even when otherwise malformed ([R4]).
fn identity_key(raw: &Value) -> Option<(String, Vec<String>)> {
    let kind = raw.get("kind")?.as_str()?;
    let key = identity_fields(kind)?
        .iter()
        .map(|(f, uuid)| {
            let v = raw.get(*f)?.as_str()?;
            Some(if *uuid {
                v.to_ascii_lowercase()
            } else {
                v.to_string()
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((kind.to_string(), key))
}

/// Load a registry document ([R9], [R15]): refuse the whole document only when it
/// is not an object with `entries` or is stamped too new; migrate an unstamped
/// (revision 0) document; then type each entry, failing only the entries involved.
pub fn load(doc: Value) -> Result<LoadedRegistry, RegistryRefusal> {
    let Value::Object(mut doc) = doc else {
        return Err(RegistryRefusal::Malformed("not a JSON object".into()));
    };
    if !doc.get("entries").is_some_and(Value::is_array) {
        return Err(RegistryRefusal::Malformed("no `entries` array".into()));
    }
    let revision = match doc.remove("registryRevision") {
        None => 0,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| RegistryRefusal::RevisionUnsupported {
                found: v.to_string(),
            })?,
    };
    // A wrongly typed catalog field never refuses the document ([R15]): it is set
    // aside, reads as absent, and goes back into `extra` to survive a rewrite.
    let mut mistyped = Map::new();
    for f in CATALOG_FIELDS {
        if doc.get(f).is_some_and(|v| !v.is_string()) {
            mistyped.extend(doc.remove_entry(f));
        }
    }
    let mut registry: Registry = serde_json::from_value(Value::Object(doc))
        .map_err(|e| RegistryRefusal::Malformed(e.to_string()))?;
    registry.extra.extend(mistyped);
    match revision {
        0 => migrate_0_to_1(&mut registry.entries),
        REGISTRY_REVISION => {}
        found => {
            return Err(RegistryRefusal::RevisionUnsupported {
                found: found.to_string(),
            })
        }
    }
    registry.registry_revision = Some(REGISTRY_REVISION);

    let mut seen: std::collections::HashMap<(String, Vec<String>), usize> = Default::default();
    for key in registry.entries.iter().filter_map(identity_key) {
        *seen.entry(key).or_default() += 1;
    }
    let (mut entries, mut outcomes) = (vec![], vec![]);
    for (index, raw) in registry.entries.iter().enumerate() {
        if identity_key(raw).is_some_and(|k| seen[&k] > 1) {
            outcomes.push(EntryOutcome {
                index,
                code: "duplicate-identity",
                message: "another entry of this kind has the same identity".into(),
            });
            continue;
        }
        match type_entry(raw) {
            Ok(entry) => entries.push(entry),
            Err((code, message)) => outcomes.push(EntryOutcome {
                index,
                code,
                message,
            }),
        }
    }
    Ok(LoadedRegistry {
        registry,
        entries,
        outcomes,
    })
}

/// Recursively sort object members by name (Unicode code point, RFC-003 [C5]).
fn sort_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut members: Vec<_> = map.into_iter().map(|(k, v)| (k, sort_keys(v))).collect();
            members.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(members.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

impl Registry {
    /// The registry's one held-and-travelling form ([R9]): sorted members at every
    /// depth, stamped `registryRevision`, `entries` ordered by kind, then the kind's
    /// identity fields (absent first), then each entry's own serialization.
    pub fn to_canonical_json(&self) -> String {
        let mut doc = self.clone();
        doc.registry_revision = Some(REGISTRY_REVISION);
        let mut entries: Vec<(String, Vec<Option<String>>, String, Value)> = doc
            .entries
            .drain(..)
            .map(|e| {
                let e = sort_keys(e);
                let kind = e.get("kind").and_then(Value::as_str).unwrap_or_default();
                let ident = identity_fields(kind)
                    .unwrap_or_default()
                    .iter()
                    .map(|(f, _)| e.get(*f).and_then(Value::as_str).map(str::to_string))
                    .collect();
                (kind.to_string(), ident, e.to_string(), e)
            })
            .collect();
        entries.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
        doc.entries = entries.into_iter().map(|e| e.3).collect();
        let value = serde_json::to_value(&doc).expect("a registry always serializes");
        sort_keys(value).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PKG: &str = "00000001-0000-4000-a000-000000000001";
    const REPO: &str = "00000002-0000-4000-a000-000000000002";

    fn package(id: &str, version: &str) -> Value {
        json!({
            "kind": "package", "locator": {"mode": "remote", "url": "https://example.com/p.srspkg"},
            "packageId": id, "packageName": "com.example.governance", "packageVersion": version,
            "publisher": "example.com", "publishedAt": "2026-01-01T00:00:00Z",
            "fieldCount": 10, "typeCount": 3
        })
    }

    fn repository(id: &str) -> Value {
        json!({"kind": "repository", "locator": {"mode": "local", "path": "../research"}, "id": id})
    }

    fn codes(l: &LoadedRegistry) -> Vec<(usize, &str)> {
        l.outcomes.iter().map(|o| (o.index, o.code)).collect()
    }

    #[test]
    fn r9_pre_rfc_catalog_is_migrated_on_read() {
        let doc = json!({
            "schemaVersion": "1.0", "registryId": PKG, "registryName": "Example",
            "catalogVersion": "1.0.0", "updatedAt": "2026-07-01T00:00:00Z",
            "entries": [{
                "packageId": PKG, "packageName": "com.example.governance", "packageVersion": "1.0.0",
                "publisher": "example.com", "publishedAt": "2026-01-01T00:00:00Z",
                "fieldCount": 5, "typeCount": 2, "downloadUrl": "https://example.com/g.srspkg"
            }, {
                "packageId": REPO, "packageName": "no.locator", "packageVersion": "1.0.0",
                "publisher": "example.com", "publishedAt": "2026-01-01T00:00:00Z",
                "fieldCount": 0, "typeCount": 0
            }]
        });
        let l = load(doc).unwrap();
        assert_eq!(l.registry.registry_revision, Some(1));
        let RegistryEntry::Package(p) = &l.entries[0] else {
            panic!("expected package")
        };
        assert_eq!(
            p.locator,
            Locator::Remote {
                url: "https://example.com/g.srspkg".into()
            }
        );
        assert!(l.registry.entries[0].get("downloadUrl").is_none());
        assert_eq!(codes(&l), vec![(1, "entry-malformed")]);
    }

    #[test]
    fn r15_document_refusals() {
        assert!(matches!(
            load(json!([])),
            Err(RegistryRefusal::Malformed(_))
        ));
        assert!(matches!(
            load(json!({"entries": {}})),
            Err(RegistryRefusal::Malformed(_))
        ));
        assert_eq!(
            load(json!({"registryRevision": 2, "entries": []})),
            Err(RegistryRefusal::RevisionUnsupported { found: "2".into() })
        );
        for bad in [json!(-1), json!(1.5), json!("1")] {
            assert!(matches!(
                load(json!({"registryRevision": bad, "entries": []})),
                Err(RegistryRefusal::RevisionUnsupported { .. })
            ));
        }
    }

    #[test]
    fn r4_duplicates_fail_both_and_ignore_uuid_case() {
        let mut malformed = repository(&REPO.to_uppercase());
        malformed["readOnly"] = json!("yes");
        let l = load(json!({"registryRevision": 1, "entries": [
            repository(REPO), malformed, package(PKG, "1.0.0"), package(PKG, "1.1.0")
        ]}))
        .unwrap();
        assert_eq!(
            codes(&l),
            vec![(0, "duplicate-identity"), (1, "duplicate-identity")]
        );
        assert_eq!(l.entries.len(), 2);
    }

    #[test]
    fn r3_r14_r15_malformed_entries_fail_alone() {
        let mut bundled = repository(REPO);
        bundled["locator"] = json!({"mode": "bundled", "id": "x"});
        let mut not_bool = repository("00000003-0000-4000-a000-000000000003");
        not_bool["readOnly"] = json!(1);
        let mut no_publisher = package(PKG, "1.0.0");
        no_publisher.as_object_mut().unwrap().remove("publisher");
        let l = load(json!({"registryRevision": 1, "entries": [
            bundled, not_bool, no_publisher, {"locator": {"mode": "local", "path": "x"}}, 7,
            repository("00000004-0000-4000-a000-000000000004")
        ]}))
        .unwrap();
        assert_eq!(
            codes(&l),
            vec![
                (0, "entry-malformed"),
                (1, "entry-malformed"),
                (2, "entry-malformed"),
                (3, "entry-malformed"),
                (4, "entry-malformed")
            ]
        );
        assert_eq!(l.entries.len(), 1);
    }

    #[test]
    fn r14_read_only_and_unknown_fields() {
        let mut r = repository(REPO);
        r["readOnly"] = json!(true);
        r["futureField"] = json!(1);
        let l = load(json!({"registryRevision": 1, "entries": [r]})).unwrap();
        let RegistryEntry::Repository(e) = &l.entries[0] else {
            panic!("expected repository")
        };
        assert!(e.read_only);
        assert!(l.outcomes.is_empty());
    }

    #[test]
    fn r9_unknown_kinds_and_fields_survive_a_byte_identical_rewrite() {
        let doc = json!({"registryRevision": 1, "zeta": {"b": 1, "a": 2}, "entries": [
            {"kind": "zzz-future", "locator": {"mode": "remote", "url": "u"}, "x": 1},
            repository(REPO), package(PKG, "1.0.0"),
            {"kind": "repository", "locator": {"mode": "local", "path": "p"}}
        ]});
        let l = load(doc).unwrap();
        assert_eq!(
            codes(&l),
            vec![(0, "kind-unsupported"), (3, "entry-malformed")]
        );
        let once = l.registry.to_canonical_json();
        let twice = load(serde_json::from_str(&once).unwrap())
            .unwrap()
            .registry
            .to_canonical_json();
        assert_eq!(once, twice);
        let v: Value = serde_json::from_str(&once).unwrap();
        let kinds: Vec<_> = v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["kind"].clone())
            .collect();
        // by kind; within `repository`, the entry missing `id` sorts first (absent first)
        assert_eq!(
            kinds,
            vec![
                json!("package"),
                json!("repository"),
                json!("repository"),
                json!("zzz-future")
            ]
        );
        assert!(v["entries"][1].get("id").is_none());
        assert!(
            once.starts_with(r#"{"entries":"#),
            "members sorted at the top level"
        );
        assert!(once.contains(r#""zeta":{"a":2,"b":1}"#));
    }

    #[test]
    fn r15_mistyped_catalog_fields_read_as_absent_and_survive() {
        let l = load(
            json!({"schemaVersion": 1, "homepage": {}, "registryName": "R",
                            "entries": []}),
        )
        .unwrap();
        assert_eq!(l.registry.schema_version, None);
        assert_eq!(l.registry.registry_name.as_deref(), Some("R"));
        let out = l.registry.to_canonical_json();
        assert!(out.contains(r#""homepage":{}"#) && out.contains(r#""schemaVersion":1"#));
    }

    #[test]
    fn r14_identities_must_be_uuids() {
        let mut urn = repository(REPO);
        urn["id"] = json!(format!("urn:uuid:{REPO}"));
        let l = load(json!({"registryRevision": 1, "entries": [
            repository("not-a-uuid"), urn, package("x", "1.0.0"),
            repository(&REPO.to_uppercase())
        ]}))
        .unwrap();
        assert_eq!(
            codes(&l),
            vec![
                (0, "entry-malformed"),
                (1, "entry-malformed"),
                (2, "entry-malformed")
            ]
        );
        assert_eq!(l.entries.len(), 1);
    }

    #[test]
    fn r9_member_and_entry_order_do_not_change_the_bytes() {
        let a = json!({"registryRevision": 1, "registryName": "R", "entries": [
            repository(REPO), package(PKG, "1.0.0")
        ]});
        let mut pkg = Map::new();
        for (k, v) in package(PKG, "1.0.0").as_object().unwrap().iter().rev() {
            pkg.insert(k.clone(), v.clone());
        }
        let b = json!({"entries": [Value::Object(pkg), repository(REPO)],
                       "registryName": "R", "registryRevision": 1});
        let bytes = |d| load(d).unwrap().registry.to_canonical_json();
        assert_eq!(bytes(a), bytes(b));
    }

    #[test]
    fn r9_explicit_revision_zero_migrates() {
        let l = load(json!({"registryRevision": 0, "entries": [{
            "packageId": PKG, "packageName": "p", "packageVersion": "1.0.0",
            "publisher": "e", "publishedAt": "2026-01-01T00:00:00Z",
            "fieldCount": 0, "typeCount": 0, "downloadUrl": "https://e/p.srspkg"
        }]}))
        .unwrap();
        assert!(l.outcomes.is_empty());
        assert_eq!(l.registry.registry_revision, Some(1));
    }

    #[test]
    fn ponytail_large_numbers_in_unknown_fields_lose_precision() {
        // Pins the documented limit (no `arbitrary_precision`): 2^64 + 1 is read as f64.
        let l = load(json!({"entries": [],
            "future": serde_json::from_str::<Value>("18446744073709551617").unwrap()}))
        .unwrap();
        assert!(!l
            .registry
            .to_canonical_json()
            .contains("18446744073709551617"));
    }
}
