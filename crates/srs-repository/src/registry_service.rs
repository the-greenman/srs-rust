use crate::error::RepositoryError;
use serde::Deserialize;
use srs_core::extensions::registry::{Registry, RegistryEntry};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryListFilter {
    pub publisher: Option<String>,
    pub tag: Option<String>,
}

fn read_registry_file(path: &std::path::Path) -> Result<Registry, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|_| RepositoryError::NotFound {
        path: path.to_path_buf(),
    })?;
    parse_registry_json(&content)
}

/// Parse a registry JSON string into a `Registry`.
/// Returns `RegistryParse` on deserialization failure.
pub fn parse_registry_json(json: &str) -> Result<Registry, RepositoryError> {
    serde_json::from_str(json).map_err(|source| RepositoryError::RegistryParse { source })
}

/// Apply a `RegistryListFilter` to a `Registry`, returning a new `Registry` with
/// only entries that match all supplied filter criteria (absent = wildcard).
pub fn filter_registry_entries(mut registry: Registry, filter: &RegistryListFilter) -> Registry {
    registry.entries.retain(|entry| {
        if let Some(pub_filter) = &filter.publisher {
            if &entry.publisher != pub_filter {
                return false;
            }
        }
        if let Some(tag_filter) = &filter.tag {
            match &entry.tags {
                Some(tags) => {
                    if !tags.contains(tag_filter) {
                        return false;
                    }
                }
                None => return false,
            }
        }
        true
    });
    registry
}

#[derive(Debug)]
pub struct ListRegistryInput {
    pub path: PathBuf,
    pub filter: RegistryListFilter,
}

#[derive(Debug)]
pub struct ListRegistryResult {
    pub registry_id: String,
    pub registry_name: String,
    pub catalog_version: String,
    pub updated_at: String,
    pub homepage: Option<String>,
    pub entries: Vec<RegistryEntry>,
    pub total_count: usize,
    pub filtered_count: usize,
}

/// Load a registry from a file path, apply an optional filter, and return
/// summary metadata alongside the matching entries and counts.
pub fn list_registry(input: ListRegistryInput) -> Result<ListRegistryResult, RepositoryError> {
    let registry = read_registry_file(&input.path).map_err(|e| match e {
        RepositoryError::NotFound { path } | RepositoryError::RegistryLoad { path, .. } => {
            RepositoryError::NotFound { path }
        }
        other => other,
    })?;
    let total_count = registry.entries.len();
    let registry_id = registry.registry_id.clone();
    let registry_name = registry.registry_name.clone();
    let catalog_version = registry.catalog_version.clone();
    let updated_at = registry.updated_at.clone();
    let homepage = registry.homepage.clone();
    let filtered = filter_registry_entries(registry, &input.filter);
    let filtered_count = filtered.entries.len();
    Ok(ListRegistryResult {
        registry_id,
        registry_name,
        catalog_version,
        updated_at,
        homepage,
        entries: filtered.entries,
        total_count,
        filtered_count,
    })
}

#[derive(Debug)]
pub struct GetRegistryEntryInput {
    pub path: PathBuf,
    pub package_name: String,
}

#[derive(Debug)]
pub struct GetRegistryEntryResult {
    pub registry_id: String,
    pub entry: RegistryEntry,
}

/// Load a registry from a file path and look up a single entry by `package_name`.
/// Returns `RegistryEntryNotFound` when no match exists.
pub fn get_registry_entry(
    input: GetRegistryEntryInput,
) -> Result<GetRegistryEntryResult, RepositoryError> {
    let registry = read_registry_file(&input.path)?;
    let registry_id = registry.registry_id.clone();
    registry
        .entries
        .into_iter()
        .find(|e| e.package_name == input.package_name)
        .map(|entry| GetRegistryEntryResult { registry_id, entry })
        .ok_or_else(|| RepositoryError::RegistryEntryNotFound {
            package_name: input.package_name,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::extensions::registry::{Registry, RegistryEntry};
    use std::io::Write;

    fn make_entry(package_name: &str, publisher: &str, tags: Option<Vec<&str>>) -> RegistryEntry {
        RegistryEntry {
            package_id: format!("00000000-0000-4000-a000-{:012}", package_name.len()),
            package_name: package_name.to_string(),
            package_version: "1.0.0".to_string(),
            publisher: publisher.to_string(),
            description: None,
            published_at: "2026-01-01T00:00:00Z".to_string(),
            homepage: None,
            tags: tags.map(|ts| ts.into_iter().map(|t| t.to_string()).collect()),
            field_count: 5,
            type_count: 2,
            view_count: None,
            schema_count: None,
            protocol_count: None,
            relation_type_count: None,
            download_url: None,
            checksum: None,
        }
    }

    fn make_registry(entries: Vec<RegistryEntry>) -> Registry {
        Registry {
            schema_version: "1.0".to_string(),
            registry_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            registry_name: "Test Registry".to_string(),
            catalog_version: "1.0.0".to_string(),
            updated_at: "2026-07-01T00:00:00Z".to_string(),
            homepage: None,
            entries,
        }
    }

    fn registry_to_json(registry: &Registry) -> String {
        serde_json::to_string(registry).unwrap()
    }

    fn write_registry_file(registry: &Registry) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        write!(f, "{}", registry_to_json(registry)).unwrap();
        f
    }

    #[test]
    fn list_registry_no_filter_returns_all_entries() {
        let entries = vec![
            make_entry("com.a.pkg", "a.com", None),
            make_entry("com.b.pkg", "b.com", None),
        ];
        let registry = make_registry(entries);
        let f = write_registry_file(&registry);
        let result = list_registry(ListRegistryInput {
            path: f.path().to_path_buf(),
            filter: RegistryListFilter::default(),
        })
        .unwrap();
        assert_eq!(result.total_count, 2);
        assert_eq!(result.filtered_count, 2);
        assert_eq!(result.entries.len(), 2);
        assert_eq!(result.registry_id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    }

    #[test]
    fn list_registry_filter_by_publisher() {
        let entries = vec![
            make_entry("com.a.pkg", "a.com", None),
            make_entry("com.b.pkg", "b.com", None),
            make_entry("com.c.pkg", "a.com", None),
        ];
        let registry = make_registry(entries);
        let f = write_registry_file(&registry);
        let result = list_registry(ListRegistryInput {
            path: f.path().to_path_buf(),
            filter: RegistryListFilter {
                publisher: Some("a.com".to_string()),
                tag: None,
            },
        })
        .unwrap();
        assert_eq!(result.total_count, 3);
        assert_eq!(result.filtered_count, 2);
        assert!(result
            .entries
            .iter()
            .all(|e| e.publisher == "a.com"));
    }

    #[test]
    fn list_registry_filter_by_tag() {
        let entries = vec![
            make_entry("com.a.pkg", "a.com", Some(vec!["governance"])),
            make_entry("com.b.pkg", "b.com", Some(vec!["risk"])),
            make_entry("com.c.pkg", "c.com", None),
        ];
        let registry = make_registry(entries);
        let f = write_registry_file(&registry);
        let result = list_registry(ListRegistryInput {
            path: f.path().to_path_buf(),
            filter: RegistryListFilter {
                publisher: None,
                tag: Some("governance".to_string()),
            },
        })
        .unwrap();
        assert_eq!(result.total_count, 3);
        assert_eq!(result.filtered_count, 1);
        assert_eq!(result.entries[0].package_name, "com.a.pkg");
    }

    #[test]
    fn list_registry_filter_both_publisher_and_tag() {
        let entries = vec![
            make_entry("com.a.pkg", "a.com", Some(vec!["governance"])),
            make_entry("com.b.pkg", "a.com", Some(vec!["risk"])),
            make_entry("com.c.pkg", "b.com", Some(vec!["governance"])),
        ];
        let registry = make_registry(entries);
        let f = write_registry_file(&registry);
        let result = list_registry(ListRegistryInput {
            path: f.path().to_path_buf(),
            filter: RegistryListFilter {
                publisher: Some("a.com".to_string()),
                tag: Some("governance".to_string()),
            },
        })
        .unwrap();
        assert_eq!(result.total_count, 3);
        assert_eq!(result.filtered_count, 1);
        assert_eq!(result.entries[0].package_name, "com.a.pkg");
    }

    #[test]
    fn parse_registry_json_roundtrip() {
        let registry = make_registry(vec![make_entry("com.x.pkg", "x.com", None)]);
        let json = registry_to_json(&registry);
        let parsed = parse_registry_json(&json).unwrap();
        assert_eq!(parsed.registry_id, registry.registry_id);
        assert_eq!(parsed.entries[0].package_name, "com.x.pkg");
    }

    #[test]
    fn read_registry_file_missing_returns_error() {
        let result = list_registry(ListRegistryInput {
            path: PathBuf::from("/nonexistent/path/registry.json"),
            filter: RegistryListFilter::default(),
        });
        assert!(result.is_err());
        matches!(result.unwrap_err(), RepositoryError::NotFound { .. });
    }

    #[test]
    fn get_registry_entry_found() {
        let entries = vec![
            make_entry("com.a.pkg", "a.com", None),
            make_entry("com.b.pkg", "b.com", None),
        ];
        let registry = make_registry(entries);
        let f = write_registry_file(&registry);
        let result = get_registry_entry(GetRegistryEntryInput {
            path: f.path().to_path_buf(),
            package_name: "com.a.pkg".to_string(),
        })
        .unwrap();
        assert_eq!(result.entry.package_name, "com.a.pkg");
        assert_eq!(result.registry_id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    }

    #[test]
    fn get_registry_entry_not_found() {
        let registry = make_registry(vec![make_entry("com.a.pkg", "a.com", None)]);
        let f = write_registry_file(&registry);
        let err = get_registry_entry(GetRegistryEntryInput {
            path: f.path().to_path_buf(),
            package_name: "com.missing.pkg".to_string(),
        })
        .unwrap_err();
        assert_eq!(
            err,
            RepositoryError::RegistryEntryNotFound {
                package_name: "com.missing.pkg".to_string()
            }
        );
    }
}
