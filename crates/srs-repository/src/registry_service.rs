use crate::error::RepositoryError;
use serde::Deserialize;
use srs_core::extensions::registry::{
    self, EntryOutcome, LoadedRegistry, PackageEntry, RegistryEntry, RegistryRefusal,
    REGISTRY_REVISION,
};
use std::path::PathBuf;

/// Discovery criteria for `registry list` (RFC-049: lookup by display fields is
/// discovery, never resolution). `publisher` and `tags` are package fields, so a
/// set criterion excludes entries of every other kind.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistryListFilter {
    pub publisher: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn refused(r: RegistryRefusal, path: Option<&std::path::Path>) -> RepositoryError {
    match r {
        RegistryRefusal::Malformed(message) => RepositoryError::RegistryMalformed {
            path: path.map(|p| p.to_path_buf()),
            message,
        },
        RegistryRefusal::RevisionUnsupported { found } => {
            RepositoryError::RegistryRevisionUnsupported {
                found,
                supported: REGISTRY_REVISION,
            }
        }
    }
}

/// Parse and load a registry document (RFC-049 [R9], [R15]): the whole document is
/// refused only when malformed or stamped too new; bad entries fail on their own.
pub fn parse_registry_json(json: &str) -> Result<LoadedRegistry, RepositoryError> {
    load_json(json, None)
}

fn load_json(
    json: &str,
    path: Option<&std::path::Path>,
) -> Result<LoadedRegistry, RepositoryError> {
    let doc = serde_json::from_str(json)
        .map_err(|e| refused(RegistryRefusal::Malformed(e.to_string()), path))?;
    registry::load(doc).map_err(|r| refused(r, path))
}

fn read_registry_file(path: &std::path::Path) -> Result<LoadedRegistry, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RepositoryError::NotFound {
                path: path.to_path_buf(),
            }
        } else {
            RepositoryError::RegistryIo {
                path: path.to_path_buf(),
                message: e.to_string(),
            }
        }
    })?;
    load_json(&content, Some(path))
}

/// Keep the entries that match every supplied criterion (absent = wildcard).
pub fn filter_registry_entries(
    entries: Vec<RegistryEntry>,
    filter: &RegistryListFilter,
) -> Vec<RegistryEntry> {
    let unfiltered = filter.publisher.is_none() && filter.tags.is_empty();
    entries
        .into_iter()
        .filter(|entry| match entry {
            RegistryEntry::Package(p) => {
                filter
                    .publisher
                    .as_ref()
                    .is_none_or(|pb| &p.publisher == pb)
                    && filter
                        .tags
                        .iter()
                        .all(|t| p.tags.as_ref().is_some_and(|ts| ts.contains(t)))
            }
            _ => unfiltered,
        })
        .collect()
}

#[derive(Debug)]
pub struct ListRegistryInput {
    pub path: PathBuf,
    pub filter: RegistryListFilter,
}

#[derive(Debug)]
pub struct ListRegistryResult {
    pub registry_revision: u64,
    pub registry_id: Option<String>,
    pub registry_name: Option<String>,
    pub catalog_version: Option<String>,
    pub updated_at: Option<String>,
    pub homepage: Option<String>,
    pub entries: Vec<RegistryEntry>,
    pub outcomes: Vec<EntryOutcome>,
    pub total_count: usize,
    pub filtered_count: usize,
}

/// Load a registry from a file path, apply an optional filter, and return the
/// catalog fields alongside the matching entries, the per-entry load outcomes
/// ([R15]) and counts (`totalCount` counts the entries that loaded).
pub fn list_registry(input: ListRegistryInput) -> Result<ListRegistryResult, RepositoryError> {
    let loaded = read_registry_file(&input.path)?;
    let total_count = loaded.entries.len();
    let doc = loaded.registry;
    let entries = filter_registry_entries(loaded.entries, &input.filter);
    Ok(ListRegistryResult {
        registry_revision: doc.registry_revision.unwrap_or(REGISTRY_REVISION),
        registry_id: doc.registry_id,
        registry_name: doc.registry_name,
        catalog_version: doc.catalog_version,
        updated_at: doc.updated_at,
        homepage: doc.homepage,
        filtered_count: entries.len(),
        entries,
        outcomes: loaded.outcomes,
        total_count,
    })
}

#[derive(Debug)]
pub struct GetRegistryEntryInput {
    pub path: PathBuf,
    pub package_name: String,
}

#[derive(Debug)]
pub struct GetRegistryEntryResult {
    pub registry_id: Option<String>,
    pub entries: Vec<PackageEntry>,
}

/// Discovery by `packageName` (RFC-049 [R3]: a display field never resolves):
/// every package entry with that name, one per version. Returns
/// `RegistryEntryNotFound` when none matches.
pub fn get_registry_entry(
    input: GetRegistryEntryInput,
) -> Result<GetRegistryEntryResult, RepositoryError> {
    let loaded = read_registry_file(&input.path)?;
    let entries: Vec<PackageEntry> = loaded
        .entries
        .into_iter()
        .filter_map(|e| match e {
            RegistryEntry::Package(p) if p.package_name == input.package_name => Some(p),
            _ => None,
        })
        .collect();
    if entries.is_empty() {
        return Err(RepositoryError::RegistryEntryNotFound {
            package_name: input.package_name,
        });
    }
    Ok(GetRegistryEntryResult {
        registry_id: loaded.registry.registry_id,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::io::Write;

    fn package(name: &str, version: &str, publisher: &str, tags: &[&str]) -> Value {
        json!({
            "kind": "package", "locator": {"mode": "remote", "url": "https://example.com/p.srspkg"},
            "packageId": format!("00000000-0000-4000-a000-{:012}", name.bytes().map(u64::from).sum::<u64>()),
            "packageName": name, "packageVersion": version, "publisher": publisher,
            "publishedAt": "2026-01-01T00:00:00Z", "fieldCount": 5, "typeCount": 2,
            "tags": tags
        })
    }

    fn repository() -> Value {
        json!({"kind": "repository", "id": "00000009-0000-4000-a000-000000000009",
               "locator": {"mode": "local", "path": "../research"}, "readOnly": true})
    }

    fn write_registry_file(entries: Vec<Value>) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let doc = json!({"registryRevision": 1, "registryId": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                         "registryName": "Test Registry", "entries": entries});
        write!(f, "{doc}").unwrap();
        f
    }

    fn list(entries: Vec<Value>, filter: RegistryListFilter) -> ListRegistryResult {
        let f = write_registry_file(entries);
        list_registry(ListRegistryInput {
            path: f.path().to_path_buf(),
            filter,
        })
        .unwrap()
    }

    fn names(r: &ListRegistryResult) -> Vec<&str> {
        r.entries
            .iter()
            .map(|e| match e {
                RegistryEntry::Package(p) => p.package_name.as_str(),
                RegistryEntry::Repository(_) => "repository",
            })
            .collect()
    }

    #[test]
    fn list_registry_no_filter_returns_every_kind() {
        let r = list(
            vec![package("com.a.pkg", "1.0.0", "a.com", &[]), repository()],
            RegistryListFilter::default(),
        );
        assert_eq!((r.total_count, r.filtered_count), (2, 2));
        assert_eq!(names(&r), vec!["com.a.pkg", "repository"]);
        assert_eq!(
            r.registry_id.as_deref(),
            Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
        );
        assert_eq!(r.registry_revision, 1);
    }

    #[test]
    fn list_registry_filters_match_package_fields_only() {
        let entries = vec![
            package("com.a.pkg", "1.0.0", "a.com", &["official", "governance"]),
            package("com.b.pkg", "1.0.0", "a.com", &["official"]),
            package("com.c.pkg", "1.0.0", "c.com", &["official", "governance"]),
            repository(),
        ];
        let by_publisher = list(
            entries.clone(),
            RegistryListFilter {
                publisher: Some("a.com".into()),
                tags: vec![],
            },
        );
        assert_eq!(names(&by_publisher), vec!["com.a.pkg", "com.b.pkg"]);
        let by_both = list(
            entries,
            RegistryListFilter {
                publisher: Some("a.com".into()),
                tags: vec!["official".into(), "governance".into()],
            },
        );
        assert_eq!(names(&by_both), vec!["com.a.pkg"]);
        assert_eq!(by_both.total_count, 4);
    }

    #[test]
    fn list_registry_reports_entry_outcomes_without_failing() {
        let mut bad = repository();
        bad["readOnly"] = json!("yes");
        let r = list(
            vec![bad, package("com.a.pkg", "1.0.0", "a.com", &[])],
            RegistryListFilter::default(),
        );
        assert_eq!(names(&r), vec!["com.a.pkg"]);
        assert_eq!(r.outcomes[0].code, "entry-malformed");
    }

    #[test]
    fn parse_registry_json_refusals_carry_codes() {
        let malformed = parse_registry_json("not json").unwrap_err();
        assert_eq!(malformed.code(), "registry-malformed");
        let too_new = parse_registry_json(r#"{"registryRevision": 9, "entries": []}"#).unwrap_err();
        assert_eq!(too_new.code(), "registry-revision-unsupported");
    }

    #[test]
    fn malformed_file_reports_its_path() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "[]").unwrap();
        let err = list_registry(ListRegistryInput {
            path: f.path().to_path_buf(),
            filter: RegistryListFilter::default(),
        })
        .unwrap_err();
        assert_eq!(err.report().details.unwrap()["path"], json!(f.path()));
    }

    #[test]
    fn read_registry_file_missing_returns_error() {
        let result = list_registry(ListRegistryInput {
            path: PathBuf::from("/nonexistent/registry.json"),
            filter: RegistryListFilter::default(),
        });
        assert!(matches!(result, Err(RepositoryError::NotFound { .. })));
    }

    #[test]
    fn get_registry_entry_is_discovery_returning_every_version() {
        let f = write_registry_file(vec![
            package("com.a.pkg", "1.0.0", "a.com", &[]),
            package("com.b.pkg", "1.0.0", "b.com", &[]),
            package("com.a.pkg", "2.0.0", "a.com", &[]),
        ]);
        let result = get_registry_entry(GetRegistryEntryInput {
            path: f.path().to_path_buf(),
            package_name: "com.a.pkg".into(),
        })
        .unwrap();
        let versions: Vec<_> = result
            .entries
            .iter()
            .map(|p| p.package_version.as_str())
            .collect();
        assert_eq!(versions, vec!["1.0.0", "2.0.0"]);
        assert_eq!(
            result.registry_id.as_deref(),
            Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
        );
    }

    #[test]
    fn get_registry_entry_not_found() {
        let f = write_registry_file(vec![package("com.a.pkg", "1.0.0", "a.com", &[])]);
        let err = get_registry_entry(GetRegistryEntryInput {
            path: f.path().to_path_buf(),
            package_name: "com.z.pkg".into(),
        })
        .unwrap_err();
        assert_eq!(
            err,
            RepositoryError::RegistryEntryNotFound {
                package_name: "com.z.pkg".into()
            }
        );
    }
}
