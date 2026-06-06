//! # Tag Service
//!
//! Public API for tag definition operations. This module is the sole entry point for
//! all tag logic. CLI handlers and future API handlers must call these
//! functions; they must not call internal helpers directly.
//!
//! ## Service boundary contract (ADR-010)
//!
//! - Every public function takes a typed input struct and returns a typed result struct.
//! - All validation, container orchestration, and multi-step operations happen here.
//! - Functions marked `pub(crate)` are internal helpers; do not promote them to `pub`.
//!
//! ## Handler pattern
//!
//! ```rust,ignore
//! // CLI or API handler — this is the entire function body
//! let input: CreateTagInput = serde_json::from_reader(io::stdin())?;
//! let result = tag_service::create_tag_definition(store, input)?;
//! output::ok("tag create", result)
//! ```

use crate::container_service;
use crate::error::RepositoryError;
use crate::loader::load_tag_definition;
use crate::store::RepositoryStore;
use crate::vocabulary_service;
use crate::writer::{new_instance_id, upsert_tag_definition_index_entry, write_manifest};
use srs_core::types::tag_definition::TagDefinition;
use srs_core::types::term::Term;
use srs_core::validation::tag_definition::validate_tag_definition;

/// Summary for list operations
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDefinitionSummary {
    pub instance_id: String,
    pub tag_key: String,
    pub label: Option<String>,
    pub roles: Option<Vec<String>>,
    pub status: Option<String>,
}

/// Result type for get_tag_definition_by_id
pub enum GetTagDefinitionResult {
    Found(Box<TagDefinition>),
    NotFound,
}

/// Result type for create_tag_definition
pub struct CreateTagDefinitionResult {
    pub tag_definition: TagDefinition,
}

/// Result type for update_tag_definition
pub struct UpdateTagDefinitionResult {
    pub tag_definition: TagDefinition,
}

/// Result type for delete_tag_definition
#[derive(Debug)]
pub struct DeleteTagDefinitionResult {
    pub instance_id: String,
}

/// A single hit returned by `query_by_tag`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagQueryHit {
    pub instance_id: String,
    pub tier: u8,
    pub path: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
}

/// Result of `query_by_tag`.
pub struct TagQueryResult {
    pub key: String,
    pub hits: Vec<TagQueryHit>,
}

/// Filter for `audit_tags`.
pub struct AuditTagsFilter {
    /// Facet prefixes that every tier-2 Record must have at least one tag for.
    /// E.g. `["construct", "layer"]` requires at least one tag beginning with `"construct:"`.
    pub required_facets: Vec<String>,
}

/// A single advisory finding from `audit_tags`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFinding {
    pub instance_id: String,
    pub path: String,
    pub title: Option<String>,
    pub kind: AuditFindingKind,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditFindingKind {
    /// No tag found for a required facet prefix.
    MissingFacet { facet: String },
    /// Tag present but no vocab is declared, so resolution cannot be verified.
    /// (Informational — not an error; included when the repo carries no vocabulary.)
    NoVocabDeclared,
}

/// Result of `audit_tags`.
pub struct AuditTagsResult {
    /// All audit findings (advisory only — never causes a validation failure).
    pub findings: Vec<AuditFinding>,
    /// Number of tier-2 Records examined.
    pub records_checked: usize,
}

/// Convert a tag key to a filesystem-friendly slug.
fn slugify_tag_key(tag_key: &str) -> String {
    tag_key
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != ' ', "")
        .replace(' ', "-")
}

/// List all TagDefinitions in the repository.
#[allow(deprecated)]
pub fn list_tag_definitions(
    store: &dyn RepositoryStore,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mut summaries = Vec::new();

    for entry in &manifest.instance_index {
        if !entry.is_tag_definition() {
            continue;
        }

        match load_tag_definition(store, &entry.path) {
            Ok(td) => {
                summaries.push(TagDefinitionSummary {
                    instance_id: td.instance_id,
                    tag_key: td.tag_key,
                    label: td.label,
                    roles: td.roles,
                    status: td.status,
                });
            }
            Err(_) => continue,
        }
    }

    Ok(summaries)
}

/// List TagDefinitions filtered by role.
pub fn list_tag_definitions_by_role(
    store: &dyn RepositoryStore,
    role: &str,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError> {
    let all = list_tag_definitions(store)?;
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|td| {
            td.roles
                .as_ref()
                .map(|roles| roles.iter().any(|r| r == role))
                .unwrap_or(false)
        })
        .collect();
    Ok(filtered)
}

/// Get a TagDefinition by its instance ID.
#[allow(deprecated)]
pub fn get_tag_definition_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<GetTagDefinitionResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == id && e.is_tag_definition())
        .cloned();

    match entry {
        Some(entry) => {
            let td = load_tag_definition(store, &entry.path)?;
            Ok(GetTagDefinitionResult::Found(Box::new(td)))
        }
        None => Ok(GetTagDefinitionResult::NotFound),
    }
}

/// Get all foundation signal tags.
pub fn get_foundation_signal_tags(
    store: &dyn RepositoryStore,
) -> Result<Vec<String>, RepositoryError> {
    let foundation_defs = list_tag_definitions_by_role(store, "foundation")?;
    let tag_keys: Vec<String> = foundation_defs.into_iter().map(|td| td.tag_key).collect();
    Ok(tag_keys)
}

/// List all Terms from vocabularies in the package (RFC-006).
pub fn list_terms(store: &dyn RepositoryStore) -> Result<Vec<Term>, RepositoryError> {
    vocabulary_service::list_terms(store)
}

/// Find a Term by id across all vocabularies in the package (RFC-006).
pub fn get_term_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Term>, RepositoryError> {
    vocabulary_service::get_term_by_id(store, id)
}

/// Create a new TagDefinition.
#[deprecated(
    note = "Tag terms are now package definitions (RFC-006). Manage terms via the package vocabulary instead."
)]
pub fn create_tag_definition(
    store: &dyn RepositoryStore,
    mut tag_definition: TagDefinition,
) -> Result<CreateTagDefinitionResult, RepositoryError> {
    validate_tag_definition(&tag_definition).map_err(|e| {
        RepositoryError::TagDefinitionValidation {
            path: std::path::PathBuf::from("records/tag-definitions"),
            source: e,
        }
    })?;

    if tag_definition.instance_id.is_empty() {
        tag_definition.instance_id = new_instance_id();
    }

    store.ensure_instance_dir("records/tag-definitions")?;

    let slug = slugify_tag_key(&tag_definition.tag_key);
    let filename = format!("{}-{}.json", slug, &tag_definition.instance_id[..8]);
    let relative_path = format!("records/tag-definitions/{}", filename);

    let value = serde_json::to_value(&tag_definition).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(&relative_path),
        source: e,
    })?;
    store.save_instance_json(&relative_path, &value)?;

    let mut manifest = store.load_manifest()?;
    upsert_tag_definition_index_entry(&mut manifest, &tag_definition, &relative_path);
    write_manifest(store, &manifest)?;

    Ok(CreateTagDefinitionResult { tag_definition })
}

/// Update an existing TagDefinition.
#[deprecated(
    note = "Tag terms are now package definitions (RFC-006). Manage terms via the package vocabulary instead."
)]
#[allow(deprecated)]
pub fn update_tag_definition(
    store: &dyn RepositoryStore,
    tag_definition: TagDefinition,
) -> Result<UpdateTagDefinitionResult, RepositoryError> {
    validate_tag_definition(&tag_definition).map_err(|e| {
        RepositoryError::TagDefinitionValidation {
            path: std::path::PathBuf::from("records/tag-definitions"),
            source: e,
        }
    })?;

    let mut manifest = store.load_manifest()?;

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == tag_definition.instance_id && e.is_tag_definition())
        .cloned()
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records/tag-definitions"),
        })?;

    let value = serde_json::to_value(&tag_definition).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(entry.path()),
        source: e,
    })?;
    store.save_instance_json(entry.path(), &value)?;

    upsert_tag_definition_index_entry(&mut manifest, &tag_definition, entry.path());
    write_manifest(store, &manifest)?;

    Ok(UpdateTagDefinitionResult { tag_definition })
}

/// Filter options for listing tag definitions
#[derive(Debug, Clone, Default)]
pub struct TagListFilter {
    /// If Some, only return tag definitions that are members of this container.
    pub container_id: Option<String>,
}

/// List tag definitions with optional container filter.
pub fn list_tag_definitions_filtered(
    store: &dyn RepositoryStore,
    filter: TagListFilter,
) -> Result<Vec<TagDefinitionSummary>, RepositoryError> {
    let member_ids: Option<std::collections::HashSet<String>> =
        if let Some(ref cid) = filter.container_id {
            let members = container_service::list_members(store, cid)?;
            Some(members.into_iter().collect())
        } else {
            None
        };

    let all = list_tag_definitions(store)?;

    let filtered = all
        .into_iter()
        .filter(|td| {
            if let Some(ref member_set) = member_ids {
                member_set.contains(&td.instance_id)
            } else {
                true
            }
        })
        .collect();

    Ok(filtered)
}

/// Create a tag definition and optionally add it to a container atomically.
#[deprecated(
    note = "Tag terms are now package definitions (RFC-006). Manage terms via the package vocabulary instead."
)]
#[allow(deprecated)]
pub fn create_tag_definition_in_context(
    store: &dyn RepositoryStore,
    tag: TagDefinition,
    container_id: Option<String>,
) -> Result<CreateTagDefinitionResult, RepositoryError> {
    if let Some(ref cid) = container_id {
        container_service::get_container(store, cid)?;
    }

    let result = create_tag_definition(store, tag)?;

    if let Some(ref cid) = container_id {
        container_service::add_member(store, cid, &result.tag_definition.instance_id)?;
    }

    Ok(result)
}

/// Delete a tag definition with optional container-scoped membership check.
#[deprecated(
    note = "Tag terms are now package definitions (RFC-006). Manage terms via the package vocabulary instead."
)]
#[allow(deprecated)]
pub fn delete_tag_definition_in_context(
    store: &dyn RepositoryStore,
    id: String,
    container_id: Option<String>,
) -> Result<DeleteTagDefinitionResult, RepositoryError> {
    if let Some(ref cid) = container_id {
        if !container_service::is_member(store, cid, &id)? {
            return Err(RepositoryError::NotFound {
                path: std::path::PathBuf::from(format!(
                    "Instance '{}' is not a member of container '{}'",
                    id, cid
                )),
            });
        }
        container_service::remove_member(store, cid, &id)?;
    }

    delete_tag_definition(store, &id)
}

/// Update a tag definition after validating that the ID in the body matches
/// the provided command ID.
#[deprecated(
    note = "Tag terms are now package definitions (RFC-006). Manage terms via the package vocabulary instead."
)]
#[allow(deprecated)]
pub fn update_tag_definition_validated(
    store: &dyn RepositoryStore,
    id: &str,
    tag: TagDefinition,
) -> Result<UpdateTagDefinitionResult, RepositoryError> {
    if tag.instance_id != id {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: format!(
                "Tag definition ID in body ({}) does not match path ID ({})",
                tag.instance_id, id
            ),
        });
    }
    update_tag_definition(store, tag)
}

/// Returns instance IDs of any manifest entries whose `tags` list contains `tag_key`.
fn find_instances_using_tag(
    store: &dyn RepositoryStore,
    tag_key: &str,
) -> Result<Vec<String>, RepositoryError> {
    let manifest = store.load_manifest()?;
    let refs: Vec<String> = manifest
        .instance_index
        .iter()
        .filter(|e| {
            e.tags
                .as_ref()
                .map(|tags| tags.iter().any(|t| t == tag_key))
                .unwrap_or(false)
        })
        .map(|e| e.instance_id().to_string())
        .collect();
    Ok(refs)
}

/// Delete a TagDefinition by ID.
/// Returns `CannotDeleteInUse` if any instance's manifest entry references this tag key.
#[deprecated(
    note = "Tag terms are now package definitions (RFC-006). Manage terms via the package vocabulary instead."
)]
#[allow(deprecated)]
pub fn delete_tag_definition(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<DeleteTagDefinitionResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let entry_index = manifest
        .instance_index
        .iter()
        .position(|e| e.instance_id() == id && e.is_tag_definition())
        .ok_or_else(|| RepositoryError::NotFound {
            path: std::path::PathBuf::from("records/tag-definitions"),
        })?;

    let path = manifest.instance_index[entry_index].path().to_string();

    // Resolve the tag key before deletion so we can check usage
    let tag_key = load_tag_definition(store, &path)
        .map(|td| td.tag_key)
        .unwrap_or_default();

    if !tag_key.is_empty() {
        let refs = find_instances_using_tag(store, &tag_key)?;
        if !refs.is_empty() {
            return Err(RepositoryError::CannotDeleteInUse {
                entity_type: "tag".to_string(),
                id: id.to_string(),
                used_by: refs,
            });
        }
    }

    let _ = store.delete_instance_file(&path); // best-effort; ignore if file missing

    manifest.instance_index.remove(entry_index);
    write_manifest(store, &manifest)?;

    Ok(DeleteTagDefinitionResult {
        instance_id: id.to_string(),
    })
}

/// Cross-tier tag query — returns all instances (Notes, TypedRecords, Records) whose
/// manifest index entry carries `tag_key`.  Reads the manifest only; no per-record file loads.
pub fn query_by_tag(
    store: &dyn RepositoryStore,
    tag_key: &str,
    container_id: Option<&str>,
) -> Result<TagQueryResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let filtered_ids: Option<std::collections::HashSet<String>> = if let Some(cid) = container_id {
        let members = crate::container_service::list_container_members(store, cid)?;
        Some(members.into_iter().collect())
    } else {
        None
    };

    let hits: Vec<TagQueryHit> = manifest
        .instance_index
        .iter()
        .filter(|e| {
            e.tags
                .as_ref()
                .map(|tags| tags.iter().any(|t| t == tag_key))
                .unwrap_or(false)
        })
        .filter(|e| {
            filtered_ids
                .as_ref()
                .map(|ids| ids.contains(e.instance_id()))
                .unwrap_or(true)
        })
        .map(|e| TagQueryHit {
            instance_id: e.instance_id().to_string(),
            tier: e.tier(),
            path: e.path().to_string(),
            title: e.title(),
            tags: e.tags.clone().unwrap_or_default(),
        })
        .collect();

    Ok(TagQueryResult {
        key: tag_key.to_string(),
        hits,
    })
}

/// Advisory tag audit — checks tier-2 Records for missing required facets.
/// Never causes validation to fail; findings are informational only.
/// Reads the manifest only; no per-record file loads.
pub fn audit_tags(
    store: &dyn RepositoryStore,
    filter: AuditTagsFilter,
) -> Result<AuditTagsResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let records: Vec<_> = manifest
        .instance_index
        .iter()
        .filter(|e| e.tier() == 2)
        .collect();

    let records_checked = records.len();
    let mut findings: Vec<AuditFinding> = Vec::new();

    for entry in records {
        let tags: Vec<&str> = entry
            .tags
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str())
            .collect();

        for facet in &filter.required_facets {
            let prefix = format!("{}:", facet);
            let has_facet = tags.iter().any(|t| t.starts_with(&prefix));
            if !has_facet {
                findings.push(AuditFinding {
                    instance_id: entry.instance_id().to_string(),
                    path: entry.path().to_string(),
                    title: entry.title(),
                    kind: AuditFindingKind::MissingFacet {
                        facet: facet.clone(),
                    },
                });
            }
        }
    }

    Ok(AuditTagsResult {
        findings,
        records_checked,
    })
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use std::collections::HashMap;

    fn make_store() -> MemoryStore {
        MemoryStore::default()
    }

    fn create_test_td(tag_key: &str) -> TagDefinition {
        TagDefinition {
            instance_id: String::new(),
            tag_key: tag_key.to_string(),
            label: Some(format!("{} Label", tag_key)),
            description: Some(format!("Description for {}", tag_key)),
            roles: Some(vec!["foundation".to_string()]),
            aliases: None,
            status: Some("active".to_string()),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn list_tag_definitions_empty_repo() {
        let store = make_store();
        let result = list_tag_definitions(&store).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn create_tag_definition_writes_and_updates_manifest() {
        let store = make_store();
        let td = create_test_td("test-tag");
        let result = create_tag_definition(&store, td).unwrap();

        assert!(!result.tag_definition.instance_id.is_empty());

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == result.tag_definition.instance_id);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().tier(), 3);
    }

    #[test]
    fn get_tag_definition_by_id_finds_created() {
        let store = make_store();
        let td = create_test_td("test-tag");
        let created = create_tag_definition(&store, td).unwrap();

        let result = get_tag_definition_by_id(&store, &created.tag_definition.instance_id).unwrap();
        match result {
            GetTagDefinitionResult::Found(td) => assert_eq!(td.tag_key, "test-tag"),
            GetTagDefinitionResult::NotFound => panic!("Should have found the tag definition"),
        }
    }

    #[test]
    fn get_tag_definition_by_id_not_found() {
        let store = make_store();
        let result =
            get_tag_definition_by_id(&store, "00000000-0000-0000-0000-000000000000").unwrap();
        match result {
            GetTagDefinitionResult::Found(_) => panic!("Should not have found anything"),
            GetTagDefinitionResult::NotFound => (),
        }
    }

    #[test]
    fn list_tag_definitions_by_role_filters_correctly() {
        let store = make_store();

        let mut foundation_td = create_test_td("foundation-tag");
        foundation_td.roles = Some(vec!["foundation".to_string()]);
        create_tag_definition(&store, foundation_td).unwrap();

        let mut nav_td = create_test_td("nav-tag");
        nav_td.roles = Some(vec!["navigation".to_string()]);
        create_tag_definition(&store, nav_td).unwrap();

        let foundation_results = list_tag_definitions_by_role(&store, "foundation").unwrap();
        assert_eq!(foundation_results.len(), 1);
        assert_eq!(foundation_results[0].tag_key, "foundation-tag");

        let nav_results = list_tag_definitions_by_role(&store, "navigation").unwrap();
        assert_eq!(nav_results.len(), 1);
        assert_eq!(nav_results[0].tag_key, "nav-tag");
    }

    #[test]
    fn get_foundation_signal_tags_returns_tag_keys() {
        let store = make_store();
        let mut td = create_test_td("purpose");
        td.roles = Some(vec!["foundation".to_string()]);
        create_tag_definition(&store, td).unwrap();

        let signal_tags = get_foundation_signal_tags(&store).unwrap();
        assert_eq!(signal_tags, vec!["purpose"]);
    }

    #[test]
    fn get_foundation_signal_tags_empty_when_none_defined() {
        let store = make_store();
        let signal_tags = get_foundation_signal_tags(&store).unwrap();
        assert!(signal_tags.is_empty());
    }

    #[test]
    fn slugify_tag_key_works() {
        assert_eq!(slugify_tag_key("Foundation"), "foundation");
        assert_eq!(slugify_tag_key("My Tag"), "my-tag");
        assert_eq!(slugify_tag_key("Complex!!!Tag"), "complextag");
    }

    #[test]
    fn tag_update_rewrites_definition() {
        let store = make_store();
        let td = create_test_td("test-tag");
        let created = create_tag_definition(&store, td).unwrap();
        let instance_id = created.tag_definition.instance_id.clone();

        let mut updated = created.tag_definition;
        updated.label = Some("Updated Label".to_string());

        let result = update_tag_definition(&store, updated).unwrap();
        assert_eq!(
            result.tag_definition.label,
            Some("Updated Label".to_string())
        );

        let fetched = get_tag_definition_by_id(&store, &instance_id).unwrap();
        match fetched {
            GetTagDefinitionResult::Found(td) => {
                assert_eq!(td.label, Some("Updated Label".to_string()));
            }
            _ => panic!("Should find updated tag"),
        }
    }

    #[test]
    fn tag_delete_blocked_when_note_uses_slug() {
        use crate::services::create_note;
        use srs_core::types::note::{Note, NoteSection};

        let store = make_store();
        let td = create_test_td("my-tag");
        let created_td = create_tag_definition(&store, td).unwrap();

        // Create a note that carries the tag slug in its top-level tags array
        let note = Note {
            instance_id: String::new(),
            title: Some("Tagged Note".to_string()),
            tags: Some(vec!["my-tag".to_string()]),
            sections: vec![NoteSection {
                name: "body".to_string(),
                content: "content".to_string(),
                label: None,
                content_hint: None,
                tags: None,
            }],
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            graduated_at: None,
            meta: None,
            source_refs: None,
            updated_at: None,
        };
        create_note(&store, note).unwrap();

        let result = delete_tag_definition(&store, &created_td.tag_definition.instance_id);
        match result {
            Err(RepositoryError::CannotDeleteInUse {
                entity_type,
                id,
                used_by,
            }) => {
                assert_eq!(entity_type, "tag");
                assert_eq!(id, created_td.tag_definition.instance_id);
                assert!(!used_by.is_empty());
            }
            other => panic!("expected CannotDeleteInUse, got {:?}", other),
        }
    }

    #[test]
    fn tag_delete_succeeds_when_no_usage() {
        let store = make_store();
        let td = create_test_td("unused-tag");
        let created = create_tag_definition(&store, td).unwrap();

        delete_tag_definition(&store, &created.tag_definition.instance_id).unwrap();
    }

    #[test]
    fn tag_delete_removes_definition() {
        let store = make_store();
        let td = create_test_td("deletable-tag");
        let created = create_tag_definition(&store, td).unwrap();
        let instance_id = created.tag_definition.instance_id.clone();

        let result = delete_tag_definition(&store, &instance_id).unwrap();
        assert_eq!(result.instance_id, instance_id);

        let manifest = store.load_manifest().unwrap();
        assert!(manifest.instance_index.is_empty());

        let fetched = get_tag_definition_by_id(&store, &instance_id).unwrap();
        match fetched {
            GetTagDefinitionResult::NotFound => {}
            _ => panic!("Should not find deleted tag"),
        }
    }
}
