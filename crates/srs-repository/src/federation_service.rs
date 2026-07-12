use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::extensions::federation::{
    FederationEvent, FederationEventKind, FederationEventsFile, RepositoryRegistry,
    RepositoryRegistryEntry,
};
use std::collections::HashSet;
use std::path::Path;

pub const DEFAULT_FEDERATION_REGISTRY_PATH: &str = "federation/registry.json";
pub const DEFAULT_FEDERATION_EVENTS_PATH: &str = "federation/events.json";

// ── Input/output types ────────────────────────────────────────────────────

#[derive(Debug)]
pub struct ResolveRepositoryInput {
    pub repository_id: String,
}

#[derive(Debug)]
pub struct ResolveRepositoryResult {
    pub found: bool,
    /// registry_id of the registry that contained the match (`found: true`),
    /// or registry_id of the root registry (`found: false`).
    pub registry_id: String,
    pub entry: Option<RepositoryRegistryEntry>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFederationEventsFilter {
    pub source_repository_id: Option<String>,
    pub target_repository_id: Option<String>,
    /// "merge" | "split" | "import"
    pub kind: Option<String>,
}

#[derive(Debug)]
pub struct ListFederationEventsInput {
    pub filter: ListFederationEventsFilter,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFederationEventsResult {
    pub repository_id: String,
    pub events: Vec<FederationEvent>,
    pub total_count: usize,
    pub filtered_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendFederationEventInput {
    pub repository_id: String,
    pub event: FederationEvent,
}

#[derive(Debug)]
pub struct AppendFederationEventResult {
    pub event_id: String,
    pub total_events: usize,
}

// ── Pure helpers ──────────────────────────────────────────────────────────

/// Parse a federation registry JSON string into a `RepositoryRegistry`.
/// Returns `RepositoryError::FederationRegistryParse` on deserialization failure.
pub fn parse_federation_registry_json(json: &str) -> Result<RepositoryRegistry, RepositoryError> {
    serde_json::from_str(json)
        .map_err(|source| RepositoryError::FederationRegistryParse { source })
}

/// Apply a `ListFederationEventsFilter` to a `FederationEventsFile`, retaining
/// only events that match all supplied criteria (absent field = wildcard).
pub fn filter_federation_events(
    mut file: FederationEventsFile,
    filter: &ListFederationEventsFilter,
) -> FederationEventsFile {
    file.events.retain(|e| {
        if let Some(src) = &filter.source_repository_id {
            if e.source_repository_id.as_ref() != Some(src) {
                return false;
            }
        }
        if let Some(tgt) = &filter.target_repository_id {
            if e.target_repository_id.as_ref() != Some(tgt) {
                return false;
            }
        }
        if let Some(kind_str) = &filter.kind {
            let event_kind_str = match &e.event {
                FederationEventKind::Merge => "merge",
                FederationEventKind::Split => "split",
                FederationEventKind::Import => "import",
            };
            if event_kind_str != kind_str {
                return false;
            }
        }
        true
    });
    file
}

// ── Private helpers ───────────────────────────────────────────────────────

fn load_registry_at(path: &Path) -> Result<RepositoryRegistry, RepositoryError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            RepositoryError::NotFound {
                path: path.to_path_buf(),
            }
        } else {
            RepositoryError::Io {
                path: path.to_path_buf(),
                source: e,
            }
        }
    })?;
    serde_json::from_str(&content).map_err(|source| RepositoryError::FederationRegistryLoad {
        path: path.to_path_buf(),
        source,
    })
}

/// DFS search through a registry and its children for a `repository_id`.
/// `seen` tracks visited `registry_id`s to detect cycles (Invariant 62).
/// Returns `(registry_id, entry)` if found, `None` if the subtree has no match.
fn dfs_search_registry(
    registry: RepositoryRegistry,
    registry_path: &Path,
    target_id: &str,
    seen: &mut HashSet<String>,
) -> Result<Option<(String, RepositoryRegistryEntry)>, RepositoryError> {
    if seen.contains(&registry.registry_id) {
        return Err(RepositoryError::FederationRegistryCycle {
            registry_id: registry.registry_id,
        });
    }
    seen.insert(registry.registry_id.clone());

    let this_registry_id = registry.registry_id;
    let child_registries = registry.child_registries;

    for entry in registry.entries {
        if entry.repository_id == target_id {
            return Ok(Some((this_registry_id, entry)));
        }
    }

    if let Some(children) = child_registries {
        let parent_dir = registry_path.parent().unwrap_or(registry_path);
        for child_location in children {
            let child_path = parent_dir.join(&child_location);
            let child_registry = load_registry_at(&child_path)?;
            if let Some(result) =
                dfs_search_registry(child_registry, &child_path, target_id, seen)?
            {
                return Ok(Some(result));
            }
        }
    }

    Ok(None)
}

// ── Service functions ─────────────────────────────────────────────────────

/// Resolve a `repository_id` via DFS through the local federation registry.
///
/// Returns `found: false` (not an error) when the ID is not in any registry —
/// cross-repo Relation references are preserved as citations, never rejected.
/// Returns `RepositoryError::NotFound` when the registry file itself is absent.
pub fn resolve_repository(
    store: &dyn RepositoryStore,
    input: ResolveRepositoryInput,
) -> Result<ResolveRepositoryResult, RepositoryError> {
    let manifest = store.load_manifest()?;
    let federation_path = manifest
        .extra
        .get("federationPath")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_FEDERATION_REGISTRY_PATH);
    let registry_path = store.repository_root().join(federation_path);

    let root_registry = load_registry_at(&registry_path)?;
    let root_registry_id = root_registry.registry_id.clone();

    let mut seen = HashSet::new();
    match dfs_search_registry(root_registry, &registry_path, &input.repository_id, &mut seen)? {
        Some((registry_id, entry)) => Ok(ResolveRepositoryResult {
            found: true,
            registry_id,
            entry: Some(entry),
        }),
        None => Ok(ResolveRepositoryResult {
            found: false,
            registry_id: root_registry_id,
            entry: None,
        }),
    }
}

/// List federation events from the repository's configured events file.
///
/// Returns an empty result (not an error) when the events file does not yet
/// exist — a repo that has `ext:federation` enabled but has recorded no events
/// is in a valid state.
pub fn list_federation_events(
    store: &dyn RepositoryStore,
    input: ListFederationEventsInput,
) -> Result<ListFederationEventsResult, RepositoryError> {
    let manifest = store.load_manifest()?;
    let events_path = manifest
        .extra
        .get("federationEventsPath")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_FEDERATION_EVENTS_PATH);
    let events_abs_path = store.repository_root().join(events_path);

    let repository_id = manifest
        .extra
        .get("repositoryId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let content = match std::fs::read_to_string(&events_abs_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ListFederationEventsResult {
                repository_id,
                events: vec![],
                total_count: 0,
                filtered_count: 0,
            });
        }
        Err(e) => {
            return Err(RepositoryError::Io {
                path: events_abs_path,
                source: e,
            });
        }
    };

    let events_file: FederationEventsFile = serde_json::from_str(&content).map_err(|source| {
        RepositoryError::FederationEventsLoad {
            path: events_abs_path.clone(),
            source,
        }
    })?;

    let total_count = events_file.events.len();
    let filtered = filter_federation_events(events_file, &input.filter);
    let filtered_count = filtered.events.len();

    Ok(ListFederationEventsResult {
        repository_id,
        events: filtered.events,
        total_count,
        filtered_count,
    })
}

/// Append a `FederationEvent` to the repository's events file, creating the
/// file (and its parent directory) if it does not yet exist.
pub fn append_federation_event(
    store: &dyn RepositoryStore,
    input: AppendFederationEventInput,
) -> Result<AppendFederationEventResult, RepositoryError> {
    let manifest = store.load_manifest()?;
    let events_path = manifest
        .extra
        .get("federationEventsPath")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_FEDERATION_EVENTS_PATH);
    let events_abs_path = store.repository_root().join(events_path);

    let event_id = input.event.event_id.clone();

    let mut events_file = match std::fs::read_to_string(&events_abs_path) {
        Ok(content) => {
            serde_json::from_str::<FederationEventsFile>(&content).map_err(|source| {
                RepositoryError::FederationEventsLoad {
                    path: events_abs_path.clone(),
                    source,
                }
            })?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FederationEventsFile {
            schema: None,
            repository_id: input.repository_id,
            events: vec![],
        },
        Err(e) => {
            return Err(RepositoryError::Io {
                path: events_abs_path,
                source: e,
            });
        }
    };

    events_file.events.push(input.event);
    let total_events = events_file.events.len();

    if let Some(parent) = events_abs_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| RepositoryError::FederationEventsWrite {
            path: events_abs_path.clone(),
            source,
        })?;
    }

    let json =
        serde_json::to_vec_pretty(&events_file).expect("FederationEventsFile is always serializable");
    std::fs::write(&events_abs_path, json).map_err(|source| RepositoryError::FederationEventsWrite {
        path: events_abs_path,
        source,
    })?;

    Ok(AppendFederationEventResult {
        event_id,
        total_events,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use crate::store::FileStore;
    use srs_core::extensions::federation::{
        FederationEventKind, FederationEventsFile, RepositoryRegistry, RepositoryRegistryEntry,
    };
    use std::io::Write;
    use tempfile::TempDir;

    // ── Test helpers ──────────────────────────────────────────────────────

    fn write_manifest(dir: &TempDir, extra: &serde_json::Value) {
        let mut manifest = serde_json::json!({"instanceIndex": []});
        if let (Some(obj), Some(ext)) = (manifest.as_object_mut(), extra.as_object()) {
            for (k, v) in ext {
                obj.insert(k.clone(), v.clone());
            }
        }
        std::fs::write(
            dir.path().join("manifest.json"),
            serde_json::to_string(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn minimal_entry(repository_id: &str, title: &str) -> RepositoryRegistryEntry {
        RepositoryRegistryEntry {
            repository_id: repository_id.to_string(),
            title: title.to_string(),
            location: None,
            last_seen: None,
            tags: None,
        }
    }

    fn make_registry(registry_id: &str, entries: Vec<RepositoryRegistryEntry>, children: Option<Vec<&str>>) -> RepositoryRegistry {
        RepositoryRegistry {
            schema: None,
            registry_id: registry_id.to_string(),
            title: "Test Registry".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            entries,
            child_registries: children.map(|c| c.into_iter().map(|s| s.to_string()).collect()),
        }
    }

    fn write_json<T: serde::Serialize>(path: &std::path::Path, value: &T) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        write!(f, "{}", serde_json::to_string(value).unwrap()).unwrap();
    }

    fn minimal_event(event_id: &str, kind: FederationEventKind) -> FederationEvent {
        FederationEvent {
            event_id: event_id.to_string(),
            event: kind,
            at: "2026-01-01T00:00:00Z".to_string(),
            performed_by: None,
            source_repository_id: None,
            target_repository_id: None,
            affected_instance_ids: vec!["i-001".to_string()],
            strategy: None,
            note: None,
        }
    }

    // ── resolve_repository tests ──────────────────────────────────────────

    #[test]
    fn resolve_repository_finds_entry_in_root() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        let registry = make_registry(
            "reg-root",
            vec![minimal_entry("repo-aaa", "Repo A")],
            None,
        );
        write_json(&tmp.path().join("federation/registry.json"), &registry);

        let store = FileStore::new(tmp.path());
        let result = resolve_repository(
            &store,
            ResolveRepositoryInput { repository_id: "repo-aaa".to_string() },
        )
        .unwrap();

        assert!(result.found);
        assert_eq!(result.registry_id, "reg-root");
        assert_eq!(result.entry.unwrap().repository_id, "repo-aaa");
    }

    #[test]
    fn resolve_repository_finds_entry_in_child() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));

        // Root registry has no matching entry, references a child
        let root = make_registry("reg-root", vec![minimal_entry("repo-root-only", "Root Only")], Some(vec!["child-registry.json"]));
        write_json(&tmp.path().join("federation/registry.json"), &root);

        // Child registry has the target entry
        let child = make_registry("reg-child", vec![minimal_entry("repo-child-only", "Child Only")], None);
        write_json(&tmp.path().join("federation/child-registry.json"), &child);

        let store = FileStore::new(tmp.path());
        let result = resolve_repository(
            &store,
            ResolveRepositoryInput { repository_id: "repo-child-only".to_string() },
        )
        .unwrap();

        assert!(result.found);
        assert_eq!(result.registry_id, "reg-child");
        assert_eq!(result.entry.unwrap().repository_id, "repo-child-only");
    }

    #[test]
    fn resolve_repository_returns_false_when_not_found() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        let registry = make_registry("reg-root", vec![minimal_entry("repo-aaa", "Repo A")], None);
        write_json(&tmp.path().join("federation/registry.json"), &registry);

        let store = FileStore::new(tmp.path());
        let result = resolve_repository(
            &store,
            ResolveRepositoryInput { repository_id: "repo-does-not-exist".to_string() },
        )
        .unwrap();

        assert!(!result.found);
        assert_eq!(result.registry_id, "reg-root");
        assert!(result.entry.is_none());
    }

    #[test]
    fn resolve_repository_detects_cycle() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));

        // Root references a child
        let root = make_registry("reg-root", vec![], Some(vec!["cycle-child.json"]));
        write_json(&tmp.path().join("federation/registry.json"), &root);

        // Child has same registry_id as root → cycle
        let cycle_child = make_registry("reg-root", vec![], None);
        write_json(&tmp.path().join("federation/cycle-child.json"), &cycle_child);

        let store = FileStore::new(tmp.path());
        let err = resolve_repository(
            &store,
            ResolveRepositoryInput { repository_id: "repo-x".to_string() },
        )
        .unwrap_err();

        assert!(
            matches!(err, RepositoryError::FederationRegistryCycle { ref registry_id } if registry_id == "reg-root"),
            "expected FederationRegistryCycle, got: {:?}",
            err
        );
    }

    #[test]
    fn resolve_repository_returns_not_found_when_registry_file_absent() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        // No federation/registry.json created

        let store = FileStore::new(tmp.path());
        let err = resolve_repository(
            &store,
            ResolveRepositoryInput { repository_id: "repo-x".to_string() },
        )
        .unwrap_err();

        assert!(
            matches!(err, RepositoryError::NotFound { .. }),
            "expected NotFound, got: {:?}",
            err
        );
    }

    // ── list_federation_events tests ──────────────────────────────────────

    fn write_events_file(tmp: &TempDir, events: Vec<FederationEvent>) {
        let file = FederationEventsFile {
            schema: None,
            repository_id: "repo-aaa".to_string(),
            events,
        };
        write_json(&tmp.path().join("federation/events.json"), &file);
    }

    #[test]
    fn list_events_missing_file_returns_empty_not_error() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        // No events file

        let store = FileStore::new(tmp.path());
        let result = list_federation_events(
            &store,
            ListFederationEventsInput { filter: ListFederationEventsFilter::default() },
        )
        .unwrap();

        assert_eq!(result.total_count, 0);
        assert_eq!(result.filtered_count, 0);
        assert!(result.events.is_empty());
    }

    #[test]
    fn list_events_no_filter_returns_all() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({"repositoryId": "repo-aaa"}));
        write_events_file(&tmp, vec![
            minimal_event("e-001", FederationEventKind::Merge),
            minimal_event("e-002", FederationEventKind::Split),
            minimal_event("e-003", FederationEventKind::Import),
        ]);

        let store = FileStore::new(tmp.path());
        let result = list_federation_events(
            &store,
            ListFederationEventsInput { filter: ListFederationEventsFilter::default() },
        )
        .unwrap();

        assert_eq!(result.repository_id, "repo-aaa");
        assert_eq!(result.total_count, 3);
        assert_eq!(result.filtered_count, 3);
        assert_eq!(result.events.len(), 3);
    }

    #[test]
    fn list_events_filter_by_kind() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        write_events_file(&tmp, vec![
            minimal_event("e-001", FederationEventKind::Merge),
            minimal_event("e-002", FederationEventKind::Split),
            minimal_event("e-003", FederationEventKind::Merge),
        ]);

        let store = FileStore::new(tmp.path());
        let result = list_federation_events(
            &store,
            ListFederationEventsInput {
                filter: ListFederationEventsFilter {
                    kind: Some("merge".to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap();

        assert_eq!(result.total_count, 3);
        assert_eq!(result.filtered_count, 2);
        assert!(result.events.iter().all(|e| matches!(e.event, FederationEventKind::Merge)));
    }

    #[test]
    fn list_events_filter_by_source_repository_id() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));

        let mut e1 = minimal_event("e-001", FederationEventKind::Import);
        e1.source_repository_id = Some("src-repo-1".to_string());
        let mut e2 = minimal_event("e-002", FederationEventKind::Import);
        e2.source_repository_id = Some("src-repo-2".to_string());
        let e3 = minimal_event("e-003", FederationEventKind::Import);

        write_events_file(&tmp, vec![e1, e2, e3]);

        let store = FileStore::new(tmp.path());
        let result = list_federation_events(
            &store,
            ListFederationEventsInput {
                filter: ListFederationEventsFilter {
                    source_repository_id: Some("src-repo-1".to_string()),
                    ..Default::default()
                },
            },
        )
        .unwrap();

        assert_eq!(result.total_count, 3);
        assert_eq!(result.filtered_count, 1);
        assert_eq!(result.events[0].event_id, "e-001");
    }

    // ── append_federation_event tests ─────────────────────────────────────

    #[test]
    fn append_event_creates_new_file() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        // No events file initially

        let store = FileStore::new(tmp.path());
        let event = minimal_event("e-001", FederationEventKind::Merge);
        let result = append_federation_event(
            &store,
            AppendFederationEventInput {
                repository_id: "repo-aaa".to_string(),
                event,
            },
        )
        .unwrap();

        assert_eq!(result.event_id, "e-001");
        assert_eq!(result.total_events, 1);

        // Verify file was created
        assert!(tmp.path().join("federation/events.json").exists());

        // Verify content
        let content = std::fs::read_to_string(tmp.path().join("federation/events.json")).unwrap();
        let file: FederationEventsFile = serde_json::from_str(&content).unwrap();
        assert_eq!(file.events.len(), 1);
        assert_eq!(file.events[0].event_id, "e-001");
        assert_eq!(file.repository_id, "repo-aaa");
    }

    #[test]
    fn append_event_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        write_manifest(&tmp, &serde_json::json!({}));
        write_events_file(&tmp, vec![minimal_event("e-001", FederationEventKind::Merge)]);

        let store = FileStore::new(tmp.path());
        let result = append_federation_event(
            &store,
            AppendFederationEventInput {
                repository_id: "repo-aaa".to_string(),
                event: minimal_event("e-002", FederationEventKind::Split),
            },
        )
        .unwrap();

        assert_eq!(result.event_id, "e-002");
        assert_eq!(result.total_events, 2);

        let content = std::fs::read_to_string(tmp.path().join("federation/events.json")).unwrap();
        let file: FederationEventsFile = serde_json::from_str(&content).unwrap();
        assert_eq!(file.events.len(), 2);
        assert_eq!(file.events[1].event_id, "e-002");
    }

    // ── pure helper tests ─────────────────────────────────────────────────

    #[test]
    fn filter_federation_events_no_filter_returns_all() {
        let file = FederationEventsFile {
            schema: None,
            repository_id: "repo-aaa".to_string(),
            events: vec![
                minimal_event("e-001", FederationEventKind::Merge),
                minimal_event("e-002", FederationEventKind::Split),
            ],
        };
        let result = filter_federation_events(file, &ListFederationEventsFilter::default());
        assert_eq!(result.events.len(), 2);
    }

    #[test]
    fn filter_federation_events_by_kind() {
        let file = FederationEventsFile {
            schema: None,
            repository_id: "repo-aaa".to_string(),
            events: vec![
                minimal_event("e-001", FederationEventKind::Merge),
                minimal_event("e-002", FederationEventKind::Import),
                minimal_event("e-003", FederationEventKind::Merge),
            ],
        };
        let result = filter_federation_events(
            file,
            &ListFederationEventsFilter {
                kind: Some("merge".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].event_id, "e-001");
        assert_eq!(result.events[1].event_id, "e-003");
    }

    #[test]
    fn filter_federation_events_by_source() {
        let mut e1 = minimal_event("e-001", FederationEventKind::Import);
        e1.source_repository_id = Some("src-A".to_string());
        let mut e2 = minimal_event("e-002", FederationEventKind::Import);
        e2.source_repository_id = Some("src-B".to_string());

        let file = FederationEventsFile {
            schema: None,
            repository_id: "repo-aaa".to_string(),
            events: vec![e1, e2],
        };
        let result = filter_federation_events(
            file,
            &ListFederationEventsFilter {
                source_repository_id: Some("src-A".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].event_id, "e-001");
    }

    #[test]
    fn parse_federation_registry_json_roundtrip() {
        let registry = make_registry("reg-001", vec![minimal_entry("repo-001", "Repo One")], None);
        let json = serde_json::to_string(&registry).unwrap();
        let parsed = parse_federation_registry_json(&json).unwrap();
        assert_eq!(parsed.registry_id, "reg-001");
        assert_eq!(parsed.entries[0].repository_id, "repo-001");
    }

    #[test]
    fn parse_federation_registry_json_invalid_returns_error() {
        let err = parse_federation_registry_json("not json").unwrap_err();
        assert!(
            matches!(err, RepositoryError::FederationRegistryParse { .. }),
            "expected FederationRegistryParse, got: {:?}",
            err
        );
    }

    // ── MemoryStore test ──────────────────────────────────────────────────

    #[test]
    fn memory_store_non_default_federation_path() {
        // Verifies that the service reads federationPath from manifest.extra,
        // not a hardcoded default. MemoryStore.repository_root() = "/memory",
        // which doesn't exist on disk, so the service will attempt to open
        // "/memory/custom/registry.json" and get NotFound — that's expected.
        let store = MemoryStore::default();
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "federationPath".to_string(),
            serde_json::Value::String("custom/registry.json".to_string()),
        );
        store.save_manifest(&manifest).unwrap();

        let err = resolve_repository(
            &store,
            ResolveRepositoryInput { repository_id: "any".to_string() },
        )
        .unwrap_err();

        // The path in the error must reference the CUSTOM path, not the default
        match err {
            RepositoryError::NotFound { path } => {
                assert!(
                    path.to_string_lossy().contains("custom/registry.json"),
                    "expected path to contain 'custom/registry.json', got: {:?}",
                    path
                );
            }
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    #[test]
    fn memory_store_list_events_missing_file_returns_empty() {
        // MemoryStore.repository_root() = "/memory" (non-existent).
        // list_federation_events returns empty when events file absent.
        let store = MemoryStore::default();
        let result = list_federation_events(
            &store,
            ListFederationEventsInput { filter: ListFederationEventsFilter::default() },
        )
        .unwrap();
        assert_eq!(result.total_count, 0);
        assert!(result.events.is_empty());
    }
}
