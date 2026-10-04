use crate::analysis::build_repo_map;
use crate::error::RepositoryError;
use crate::package_service::list_types;
use crate::repository_navigation_service::repository_navigation_with_depth;
use crate::resource_uri;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTypeEntry {
    /// Path to `srs://<repo>/type/{typeId}`.
    pub type_id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub field_count: usize,
    pub description: Option<String>,
}

/// A suggested entry point: the manifest's file path, resolved to the instance it holds
/// (and its `srs://` URI) when the path names one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEntryPoint {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
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
    /// Suggested starting points from manifest.aiGuidance.suggestedEntryPoints
    /// (file paths such as "records/notes/foundation.json"), resolved to URIs.
    pub entry_points: Vec<AgentEntryPoint>,
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
            type_id: t.id,
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

    let cat = store.catalog()?;
    let repo_id = repo_map
        .repository
        .repository_id
        .clone()
        .unwrap_or_default();
    let entry_points = repo_map
        .entry_points
        .into_iter()
        .map(|path| {
            let instance_id = cat
                .instances
                .iter()
                .find(|e| e.locator.as_deref() == Some(path.as_str()))
                .map(|e| e.id.clone());
            let uri = instance_id
                .as_deref()
                .map(|id| resource_uri::record_uri(&repo_id, id));
            AgentEntryPoint {
                path,
                instance_id,
                uri,
            }
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
        entry_points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;

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
}
