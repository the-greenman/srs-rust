use crate::analysis::build_repo_map;
use crate::error::RepositoryError;
use crate::package_service::list_types;
use crate::repository_navigation_service::repository_navigation_with_depth;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTypeEntry {
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub field_count: usize,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSectionEntry {
    pub instance_id: String,
    pub label: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentIndex {
    pub repository_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub total_instances: usize,
    /// Tier-2 Records only (Tier 1 / TypedRecord was retired — srs#448/rfc-decision-53635966, srs-rust#888).
    pub records: usize,
    pub notes: usize,
    pub types: Vec<AgentTypeEntry>,
    pub sections: Vec<AgentSectionEntry>,
    /// Suggested starting points from manifest.aiGuidance.suggestedEntryPoints —
    /// file paths (e.g. "records/notes/foundation.json") recommended as entry points.
    pub entry_points: Vec<String>,
}

/// Build a typed agent-index summary of a repository by composing existing services.
/// The rendering to a human/agent-readable format (markdown) is left to the CLI layer.
pub fn build_agent_index(store: &dyn RepositoryStore) -> Result<AgentIndex, RepositoryError> {
    let repo_map = build_repo_map(store)?;
    let type_list = list_types(store)?;
    // Only the top-level section labels are read below; the part-of tree under
    // them is never consulted, so do not build it (srs-rust#1113 makes full
    // depth combinatorial on a corpus with overlapping container membership).
    let navigation = repository_navigation_with_depth(store, Some(0))?;

    let types = type_list
        .into_iter()
        .map(|t| AgentTypeEntry {
            namespace: t.namespace,
            name: t.name,
            version: t.version,
            field_count: t.field_count,
            description: t.description,
        })
        .collect();

    let sections = navigation
        .sections
        .into_iter()
        .map(|s| AgentSectionEntry {
            instance_id: s.instance_id,
            label: s.display_label,
            type_name: s.type_name,
        })
        .collect();

    Ok(AgentIndex {
        repository_id: repo_map.repository.repository_id,
        title: repo_map.repository.title,
        description: repo_map.repository.description,
        total_instances: repo_map.counts.total_instances,
        records: repo_map.counts.records,
        notes: repo_map.counts.notes,
        types,
        sections,
        entry_points: repo_map.entry_points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use crate::store::FileStore;
    use crate::vfs::{DirCheck, MemVfs, Vfs, VfsEntry};
    use srs_core::types::record::{FieldValues, Record};
    use srs_core::types::relation::Relation;
    use std::collections::BTreeMap;

    #[test]
    fn test_build_agent_index_empty_repo() {
        // MemoryStore::default() is an empty store. Core package types are always
        // embedded, so types will be non-empty; instance counts will be zero.
        let store = MemoryStore::default();
        let result = build_agent_index(&store);
        assert!(
            result.is_ok(),
            "build_agent_index on empty repo should not error"
        );
        let idx = result.unwrap();
        assert_eq!(idx.total_instances, 0);
        assert_eq!(idx.records, 0);
        assert_eq!(idx.notes, 0);
        assert!(idx.sections.is_empty());
        assert!(idx.entry_points.is_empty());
    }

    #[test]
    fn test_build_agent_index_cross_store_roundtrip() {
        // Verifies the service works correctly against a FileStore (not just MemoryStore).

        let srsj = r#"{"srsj":"2","manifest":{"dataModelRevision":2,"repositoryId":"agent-index-test","namespace":"com.example.test","srsVersion":"2.0-draft","title":"Agent Index Test"},"data":{"package/package.json":{"$schema":"https://srs.semanticops.com/schema/2.0/package-manifest.json","id":"test-pkg","namespace":"com.example.test","name":"primary","version":"1.0.0","title":"Primary","description":"","status":"active","createdAt":"2026-01-01T00:00:00Z","fields":[],"types":[],"relationTypes":[],"views":[],"compositions":[]}}}"#;

        let store = crate::srsj::open_srsj(srsj).unwrap();
        let result = build_agent_index(&store);
        assert!(
            result.is_ok(),
            "build_agent_index on FileStore-backed repo should not error"
        );
        let idx = result.unwrap();
        assert_eq!(idx.repository_id.as_deref(), Some("agent-index-test"));
        assert_eq!(idx.title.as_deref(), Some("Agent Index Test"));
        assert_eq!(idx.total_instances, 0);
    }

    // --- depth-0 boundary (srs-rust#1114) ---

    /// Counts reads of a file under `records/` — i.e. an actual instance
    /// parse — while delegating everything else to an inner `MemVfs`.
    /// Mirrors `store.rs`'s `CountingVfs` (srs-rust#1108/#1113), scoped
    /// locally per this crate's convention of each test module owning its
    /// own fixtures.
    #[derive(Debug)]
    struct CountingVfs {
        inner: MemVfs,
        record_file_reads: std::cell::Cell<usize>,
    }

    impl CountingVfs {
        fn new(inner: MemVfs) -> Self {
            Self {
                inner,
                record_file_reads: std::cell::Cell::new(0),
            }
        }
    }

    impl Vfs for CountingVfs {
        fn read_to_string(&self, rel: &str) -> Result<String, RepositoryError> {
            if rel.starts_with("records/") {
                self.record_file_reads.set(self.record_file_reads.get() + 1);
            }
            self.inner.read_to_string(rel)
        }
        fn read_bytes(&self, rel: &str) -> Result<Vec<u8>, RepositoryError> {
            self.inner.read_bytes(rel)
        }
        fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), RepositoryError> {
            self.inner.write(rel, bytes)
        }
        fn remove(&self, rel: &str) -> Result<(), RepositoryError> {
            self.inner.remove(rel)
        }
        fn exists(&self, rel: &str) -> bool {
            self.inner.exists(rel)
        }
        fn is_dir(&self, rel: &str) -> bool {
            self.inner.is_dir(rel)
        }
        fn is_file(&self, rel: &str) -> bool {
            self.inner.is_file(rel)
        }
        fn byte_len(&self, rel: &str) -> Result<u64, RepositoryError> {
            self.inner.byte_len(rel)
        }
        fn list_dir(&self, rel: &str) -> Result<Vec<VfsEntry>, RepositoryError> {
            self.inner.list_dir(rel)
        }
        fn list_recursive(&self, rel: &str) -> Vec<String> {
            self.inner.list_recursive(rel)
        }
        fn create_dir_all(&self, rel: &str) -> Result<(), RepositoryError> {
            self.inner.create_dir_all(rel)
        }
        fn check_dir_within_root(&self, rel: &str) -> Result<DirCheck, RepositoryError> {
            self.inner.check_dir_within_root(rel)
        }
        fn as_mem_snapshot(&self) -> Option<BTreeMap<String, Vec<u8>>> {
            self.inner.as_mem_snapshot()
        }
    }

    fn minimal_record(id: &str) -> Record {
        Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "type-xyz-0001".to_string(),
            type_version: 1,
            type_namespace: "com.example".to_string(),
            type_name: "Section".to_string(),
            field_values: FieldValues::new(),
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: BTreeMap::new(),
        }
    }

    fn contains_relation(id: &str, from: &str, to: &str) -> Relation {
        Relation {
            relation_id: id.to_string(),
            relation_type: "contains".to_string(),
            source_instance_id: from.to_string(),
            target_instance_id: to.to_string(),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            notes: None,
            source_refs: None,
            meta: None,
        }
    }

    /// Builds a small repo (one root section, one `contains` child below it)
    /// and returns its final on-disk snapshot, ready to be reopened by a
    /// fresh, independently-counted `FileStore`.
    fn depth_boundary_fixture_snapshot() -> BTreeMap<String, Vec<u8>> {
        let root_id = "00000000-0000-4000-8000-000000000001";
        let child_id = "00000000-0000-4000-8000-000000000002";

        // No `container` yet: `catalog_save_instance` looks up the checked
        // catalog before every write, and a root container referencing an
        // instance that doesn't exist yet fails [R13] dangling-reference —
        // the container is added below, once its root instance exists.
        let manifest = serde_json::json!({
            "dataModelRevision": 2,
            "srsVersion": "2.0-draft",
            "repositoryId": "agent-index-depth-test",
            "namespace": "com.example.test",
        });
        let package = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
            "id": "test-pkg",
            "namespace": "com.example.test",
            "name": "primary",
            "version": "1.0.0",
            "title": "Primary",
            "description": "",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [],
            "types": [],
            "views": [],
            "compositions": [],
        });
        let mut files = BTreeMap::new();
        files.insert(
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        );
        files.insert(
            "package/package.json".to_string(),
            serde_json::to_vec_pretty(&package).unwrap(),
        );

        let store =
            FileStore::from_vfs(std::rc::Rc::new(MemVfs::from_map(files)) as std::rc::Rc<dyn Vfs>);

        store.save_record(&minimal_record(root_id)).unwrap();
        store.save_record(&minimal_record(child_id)).unwrap();
        store
            .save_relation(&contains_relation(
                "00000000-0000-4000-8000-000000000010",
                root_id,
                child_id,
            ))
            .unwrap();

        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c0".to_string(),
            title: "Root".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            root_instance_ids: Some(vec![root_id.to_string()]),
            member_instance_ids: None,
            child_container_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: BTreeMap::new(),
        });
        store.save_manifest(&manifest).unwrap();

        store.vfs().as_mem_snapshot().unwrap()
    }

    /// Reopens `snapshot` behind a fresh, independently-counted `FileStore`
    /// and returns how many `records/` files `f` reads while using it.
    fn count_record_reads(
        snapshot: &BTreeMap<String, Vec<u8>>,
        f: impl FnOnce(&FileStore),
    ) -> usize {
        let counting = std::rc::Rc::new(CountingVfs::new(MemVfs::from_map(snapshot.clone())));
        let store = FileStore::from_vfs(counting.clone() as std::rc::Rc<dyn Vfs>);
        counting.record_file_reads.set(0);
        f(&store);
        counting.record_file_reads.get()
    }

    /// `build_agent_index` builds navigation at depth 0 (PR #1114, commit
    /// 0f47feb): it must never trigger `tree_service::build_tree`'s
    /// `build_node_headers`, which does one full `get_instance_by_id` read
    /// per catalog instance regardless of reachability (srs-rust#1113) —
    /// exactly the whole-corpus pass depth-0 exists to skip.
    ///
    /// Both stores below load the *same* on-disk snapshot (one root section,
    /// one `contains` child), so any read-count difference comes only from
    /// how each call navigates it — not from what's on disk. Full-depth
    /// navigation (`repository_navigation`) must read strictly more than
    /// `build_agent_index`, proving the fixture actually exercises the
    /// tree-descent path this test guards against. If a future change widens
    /// `build_agent_index`'s navigation call past depth 0, its read count
    /// rises to match full depth and this test fails.
    #[test]
    fn build_agent_index_reads_fewer_records_than_full_depth_navigation() {
        let snapshot = depth_boundary_fixture_snapshot();

        let agent_index_reads = count_record_reads(&snapshot, |store| {
            let idx = build_agent_index(store).unwrap();
            assert_eq!(idx.sections.len(), 1);
            assert_eq!(
                idx.sections[0].instance_id,
                "00000000-0000-4000-8000-000000000001"
            );
        });
        let full_depth_reads = count_record_reads(&snapshot, |store| {
            let nav = crate::repository_navigation_service::repository_navigation(store).unwrap();
            assert_eq!(nav.sections.len(), 1);
            assert_eq!(
                nav.sections[0].children.len(),
                1,
                "control: full depth must reach the child"
            );
        });

        assert!(
            agent_index_reads < full_depth_reads,
            "build_agent_index (depth 0) read {agent_index_reads} records/ files, \
             full-depth navigation read {full_depth_reads}; depth-0 must read fewer, \
             or build_agent_index has started descending the part-of tree again"
        );
    }
}
