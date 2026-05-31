use crate::container_service;
use crate::error::RepositoryError;
use crate::loader::load_note;
use crate::manifest::Manifest;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const AI_HANDOFF_GUIDANCE: &str = "This packet is deterministic repository data for external AI-assisted migration. The SRS CLI and library do not infer, extract, or decide semantic migrations. An external AI may propose candidate higher-tier records from this packet, but humans must review, revise, accept, and commit meaning.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisProfile {
    pub profile_id: String,
    pub description: Option<String>,
    pub include_tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMap {
    pub repository: RepositorySummary,
    pub counts: CountsSummary,
    pub schemas: SchemaSummary,
    pub source_documents: SourceDocumentsSummary,
    pub relations_summary: RelationsSummary,
    pub containers_summary: ContainersSummary,
    pub ai_guidance: Option<Value>,
    pub entry_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainersSummary {
    pub count: usize,
    pub types: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySummary {
    pub repository_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub conformance: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountsSummary {
    pub total_instances: usize,
    pub by_tier: BTreeMap<String, usize>,
    pub notes: usize,
    pub typed_records: usize,
    pub records: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSummary {
    pub schema_dir: String,
    pub schema_paths: Vec<String>,
    pub package_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentsSummary {
    pub source_documents_path: Option<String>,
    pub has_source_document_index: bool,
    pub source_document_index_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationsSummary {
    pub relations_path: Option<String>,
    pub exists: bool,
    pub relation_count: usize,
    pub relation_types: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagAudit {
    pub total_notes: usize,
    pub tag_counts: Vec<TagCount>,
    pub note_level_usage: Vec<TagCount>,
    pub section_level_usage: Vec<TagCount>,
    pub singleton_tags: Vec<String>,
    pub likely_singular_plural_duplicates: Vec<TagDuplicate>,
    pub broad_high_frequency_tags: Vec<TagCount>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub tag: String,
    pub count: usize,
    /// Relative paths within the repository of files where this tag appears.
    #[serde(rename = "sourcePaths")]
    pub source_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDuplicate {
    pub singular: String,
    pub plural: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationNoteSet {
    pub signal_tags: Vec<String>,
    pub notes: Vec<FoundationNoteSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundationNoteSummary {
    pub instance_id: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub matched_tags: Vec<String>,
    pub section_names: Vec<String>,
    pub source_ref_count: usize,
    pub source_refs: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPacket {
    pub profile: String,
    pub repository: RepositorySummary,
    pub schemas: SchemaSummary,
    pub counts: CountsSummary,
    pub tag_audit: TagAudit,
    pub foundation_notes: FoundationNoteSet,
    pub source_reference_summary: SourceReferenceSummary,
    pub relations_summary: RelationsSummary,
    pub manifest_entries: Vec<ManifestEntrySummary>,
    pub ai_handoff_guidance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReferenceSummary {
    pub notes_with_source_refs: usize,
    pub total_source_refs: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntrySummary {
    pub instance_id: String,
    pub tier: u8,
    /// Relative path within the repository. Intentionally included for AI migration tooling.
    #[serde(rename = "relativePath")]
    pub relative_path: String,
    pub title: Option<String>,
}

pub fn build_repo_map(store: &dyn RepositoryStore) -> Result<RepoMap, RepositoryError> {
    let manifest = store.load_manifest()?;
    build_repo_map_from_manifest(store, &manifest)
}

fn build_repo_map_from_manifest(
    store: &dyn RepositoryStore,
    manifest: &Manifest,
) -> Result<RepoMap, RepositoryError> {
    let counts = summarize_counts(manifest);
    let relations_summary = summarize_relations(store, manifest)?;
    let schemas = summarize_schemas(store);
    let source_documents = summarize_source_documents(manifest);
    let containers_summary = summarize_containers(store)?;
    let ai_guidance = manifest.extra.get("aiGuidance").cloned();
    let entry_points = ai_guidance
        .as_ref()
        .and_then(|guidance| guidance.get("suggestedEntryPoints"))
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();

    Ok(RepoMap {
        repository: summarize_repository(manifest),
        counts,
        schemas,
        source_documents,
        relations_summary,
        containers_summary,
        ai_guidance,
        entry_points,
    })
}

pub fn audit_note_tags(store: &dyn RepositoryStore) -> Result<TagAudit, RepositoryError> {
    let manifest = store.load_manifest()?;
    audit_note_tags_from_manifest(store, &manifest)
}

/// Audit note tag usage scoped to notes sharing tags with the given note ID.
///
/// Loads the target note to collect its full tag set (note-level + section-level),
/// then restricts the audit to notes that share at least one of those tags
/// (matched via the manifest index). The target note itself is always included.
pub fn audit_note_tags_for_note(
    store: &dyn RepositoryStore,
    note_id: &str,
) -> Result<TagAudit, RepositoryError> {
    let manifest = store.load_manifest()?;

    let target_entry = manifest
        .instance_index
        .iter()
        .find(|e| e.is_note() && e.instance_id() == note_id)
        .ok_or_else(|| crate::error::RepositoryError::NoteNotFound {
            path: std::path::PathBuf::from("records/notes"),
            id: note_id.to_string(),
        })?;

    let target_note = load_note(store, target_entry.path())?;
    let mut scope_tags: BTreeSet<String> = target_note.tags.iter().flatten().cloned().collect();
    for section in &target_note.sections {
        for tag in section.tags.iter().flatten() {
            scope_tags.insert(tag.clone());
        }
    }

    let mut scoped_manifest = manifest.clone();
    scoped_manifest.instance_index.retain(|entry| {
        if !entry.is_note() {
            return false;
        }
        if entry.instance_id() == note_id {
            return true;
        }
        entry.tags.iter().flatten().any(|t| scope_tags.contains(t))
    });

    audit_note_tags_from_manifest(store, &scoped_manifest)
}

fn audit_note_tags_from_manifest(
    store: &dyn RepositoryStore,
    manifest: &Manifest,
) -> Result<TagAudit, RepositoryError> {
    let mut note_level: BTreeMap<String, TagAccumulator> = BTreeMap::new();
    let mut section_level: BTreeMap<String, TagAccumulator> = BTreeMap::new();
    let mut total_notes = 0;

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }
        let Ok(note) = load_note(store, entry.path()) else {
            continue;
        };
        total_notes += 1;
        let path = entry.path().to_string();

        for tag in note.tags.unwrap_or_default() {
            note_level.entry(tag).or_default().add(path.clone());
        }

        for section in note.sections {
            for tag in section.tags.unwrap_or_default() {
                section_level.entry(tag).or_default().add(path.clone());
            }
        }
    }

    let mut combined: BTreeMap<String, TagAccumulator> = BTreeMap::new();
    for (tag, acc) in note_level.iter().chain(section_level.iter()) {
        let entry = combined.entry(tag.clone()).or_default();
        entry.count += acc.count;
        entry.files.extend(acc.files.iter().cloned());
    }

    let tag_counts = to_tag_counts(combined);
    let singleton_tags = tag_counts
        .iter()
        .filter(|tag| tag.count == 1)
        .map(|tag| tag.tag.clone())
        .collect();
    let likely_singular_plural_duplicates = find_singular_plural_duplicates(&tag_counts);
    let broad_high_frequency_tags = tag_counts
        .iter()
        .filter(|tag| tag.count >= 5)
        .cloned()
        .collect();

    Ok(TagAudit {
        total_notes,
        tag_counts,
        note_level_usage: to_tag_counts(note_level),
        section_level_usage: to_tag_counts(section_level),
        singleton_tags,
        likely_singular_plural_duplicates,
        broad_high_frequency_tags,
    })
}

pub fn collect_foundation_notes(
    store: &dyn RepositoryStore,
    signal_tags: &[String],
) -> Result<FoundationNoteSet, RepositoryError> {
    let manifest = store.load_manifest()?;
    collect_foundation_notes_from_manifest(store, &manifest, signal_tags)
}

fn collect_foundation_notes_from_manifest(
    store: &dyn RepositoryStore,
    manifest: &Manifest,
    signal_tags: &[String],
) -> Result<FoundationNoteSet, RepositoryError> {
    let signal_tags: BTreeSet<String> = signal_tags.iter().cloned().collect();
    let mut notes = Vec::new();

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }
        let Ok(note) = load_note(store, entry.path()) else {
            continue;
        };

        let mut all_tags: BTreeSet<String> =
            note.tags.clone().unwrap_or_default().into_iter().collect();
        for section in &note.sections {
            for tag in section.tags.clone().unwrap_or_default() {
                all_tags.insert(tag);
            }
        }

        let matched_tags: Vec<String> = all_tags.intersection(&signal_tags).cloned().collect();
        if matched_tags.is_empty() {
            continue;
        }

        let source_refs = note
            .source_refs
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|source_ref| serde_json::to_value(source_ref).ok())
            .collect::<Vec<_>>();

        notes.push(FoundationNoteSummary {
            instance_id: note.instance_id,
            title: note.title,
            tags: note.tags.unwrap_or_default(),
            matched_tags,
            section_names: note
                .sections
                .into_iter()
                .map(|section| section.name)
                .collect(),
            source_ref_count: source_refs.len(),
            source_refs,
        });
    }

    notes.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));

    Ok(FoundationNoteSet {
        signal_tags: signal_tags.into_iter().collect(),
        notes,
    })
}

pub fn build_migration_packet(
    store: &dyn RepositoryStore,
    profile: &str,
    foundation_signal_tags: &[String],
) -> Result<MigrationPacket, RepositoryError> {
    let manifest = store.load_manifest()?;
    let repo_map = build_repo_map_from_manifest(store, &manifest)?;
    let tag_audit = audit_note_tags_from_manifest(store, &manifest)?;
    let foundation_notes =
        collect_foundation_notes_from_manifest(store, &manifest, foundation_signal_tags)?;
    let manifest_entries = manifest
        .instance_index
        .iter()
        .map(|entry| ManifestEntrySummary {
            instance_id: entry.instance_id().to_string(),
            tier: entry.tier(),
            relative_path: entry.path().to_string(),
            title: entry.title(),
        })
        .collect();
    let source_reference_summary = SourceReferenceSummary {
        notes_with_source_refs: foundation_notes
            .notes
            .iter()
            .filter(|note| note.source_ref_count > 0)
            .count(),
        total_source_refs: foundation_notes
            .notes
            .iter()
            .map(|note| note.source_ref_count)
            .sum(),
    };

    Ok(MigrationPacket {
        profile: profile.to_string(),
        repository: repo_map.repository,
        schemas: repo_map.schemas,
        counts: repo_map.counts,
        tag_audit,
        foundation_notes,
        source_reference_summary,
        relations_summary: repo_map.relations_summary,
        manifest_entries,
        ai_handoff_guidance: AI_HANDOFF_GUIDANCE.to_string(),
    })
}

pub fn load_analysis_profile(
    store: &dyn RepositoryStore,
    profile_id: &str,
) -> Result<AnalysisProfile, RepositoryError> {
    let relative_path = format!(".srs/profiles/{profile_id}.json");
    let content = store.load_text_file(&relative_path)?;
    let profile: AnalysisProfile =
        serde_json::from_str(&content).map_err(|source| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from(&relative_path),
            source,
        })?;
    Ok(profile)
}

#[derive(Debug, Clone, Default)]
struct TagAccumulator {
    count: usize,
    files: BTreeSet<String>,
}

impl TagAccumulator {
    fn add(&mut self, file: String) {
        self.count += 1;
        self.files.insert(file);
    }
}

fn summarize_repository(manifest: &Manifest) -> RepositorySummary {
    RepositorySummary {
        repository_id: string_extra(manifest, "repositoryId"),
        title: string_extra(manifest, "title"),
        description: string_extra(manifest, "description"),
        conformance: string_extra(manifest, "conformance"),
    }
}

fn summarize_counts(manifest: &Manifest) -> CountsSummary {
    let mut by_tier: BTreeMap<String, usize> = BTreeMap::new();

    for entry in &manifest.instance_index {
        *by_tier.entry(entry.tier().to_string()).or_default() += 1;
    }

    CountsSummary {
        total_instances: manifest.instance_index.len(),
        notes: *by_tier.get("0").unwrap_or(&0),
        typed_records: *by_tier.get("1").unwrap_or(&0),
        records: *by_tier.get("2").unwrap_or(&0),
        by_tier,
    }
}

fn summarize_schemas(store: &dyn RepositoryStore) -> SchemaSummary {
    let mut schema_paths = store.list_files_recursive("schemas");
    schema_paths.retain(|p| p.ends_with(".json"));
    schema_paths.sort();

    // Check if package exists by trying to list package/package.json
    let package_path = if store
        .list_files_recursive("package")
        .iter()
        .any(|p| p == "package/package.json")
    {
        Some("package/package.json".to_string())
    } else if !store.list_files_recursive("package").is_empty() {
        Some("package".to_string())
    } else {
        None
    };

    SchemaSummary {
        schema_dir: "schemas".to_string(),
        schema_paths,
        package_path,
    }
}

fn summarize_source_documents(manifest: &Manifest) -> SourceDocumentsSummary {
    let source_document_index_count = manifest
        .extra
        .get("sourceDocumentIndex")
        .and_then(|value| value.as_array())
        .map_or(0, Vec::len);

    SourceDocumentsSummary {
        source_documents_path: string_extra(manifest, "sourceDocumentsPath"),
        has_source_document_index: source_document_index_count > 0,
        source_document_index_count,
    }
}

fn summarize_relations(
    store: &dyn RepositoryStore,
    manifest: &Manifest,
) -> Result<RelationsSummary, RepositoryError> {
    let relations_path = string_extra(manifest, "relationsPath")
        .or_else(|| Some("relations/relations.json".to_string()));
    let Some(relative_path) = relations_path.clone() else {
        return Ok(RelationsSummary {
            relations_path,
            exists: false,
            relation_count: 0,
            relation_types: BTreeMap::new(),
        });
    };

    match store.load_relations_json(&relative_path) {
        Ok(value) => {
            let relations = value
                .get("relations")
                .and_then(|relations| relations.as_array())
                .cloned()
                .unwrap_or_default();
            let mut relation_types = BTreeMap::new();
            for relation in &relations {
                let relation_type = relation
                    .get("relationType")
                    .or_else(|| relation.get("type"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                *relation_types.entry(relation_type.to_string()).or_default() += 1;
            }
            Ok(RelationsSummary {
                relations_path,
                exists: true,
                relation_count: relations.len(),
                relation_types,
            })
        }
        Err(RepositoryError::Io { .. } | RepositoryError::NotFound { .. }) => {
            Ok(RelationsSummary {
                relations_path,
                exists: false,
                relation_count: 0,
                relation_types: BTreeMap::new(),
            })
        }
        Err(e) => Err(e),
    }
}

fn to_tag_counts(map: BTreeMap<String, TagAccumulator>) -> Vec<TagCount> {
    let mut counts: Vec<_> = map
        .into_iter()
        .map(|(tag, acc)| TagCount {
            tag,
            count: acc.count,
            source_paths: acc.files.into_iter().collect(),
        })
        .collect();
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.tag.cmp(&b.tag)));
    counts
}

fn find_singular_plural_duplicates(tag_counts: &[TagCount]) -> Vec<TagDuplicate> {
    let tags: BTreeSet<_> = tag_counts.iter().map(|count| count.tag.as_str()).collect();
    let mut duplicates = Vec::new();

    for tag in &tags {
        if let Some(singular) = tag.strip_suffix('s') {
            if !singular.is_empty() && tags.contains(singular) {
                duplicates.push(TagDuplicate {
                    singular: singular.to_string(),
                    plural: (*tag).to_string(),
                });
            }
        }
    }

    duplicates
}

fn summarize_containers(store: &dyn RepositoryStore) -> Result<ContainersSummary, RepositoryError> {
    let summaries = container_service::list_containers(store, None, None, None)?;
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    for summary in &summaries {
        if let Some(ct) = &summary.container_type {
            *types.entry(ct.clone()).or_default() += 1;
        }
    }
    Ok(ContainersSummary {
        count: summaries.len(),
        types,
    })
}

fn string_extra(manifest: &Manifest, key: &str) -> Option<String> {
    manifest
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;

    fn fixture_store() -> MemoryStore {
        let store = MemoryStore::default();

        // Save manifest
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "repositoryId".to_string(),
            json!("4172fada-bc38-5479-ac18-4be3194a68ca"),
        );
        manifest
            .extra
            .insert("title".to_string(), json!("Fixture Repo"));
        manifest.extra.insert(
            "relationsPath".to_string(),
            json!("relations/relations.json"),
        );
        manifest
            .extra
            .insert("sourceDocumentsPath".to_string(), json!("source-documents"));
        manifest.extra.insert(
            "aiGuidance".to_string(),
            json!({"suggestedEntryPoints": ["records/notes/foundation.json"]}),
        );
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: "11111111-1111-4111-8111-111111111111".to_string(),
                tier: 0,
                path: "records/notes/foundation.json".to_string(),
                title: Some(json!("Foundation")),
                tags: None,
            });
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: "22222222-2222-4222-8222-222222222222".to_string(),
                tier: 0,
                path: "records/notes/problem.json".to_string(),
                title: Some(json!("Problem")),
                tags: None,
            });
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: "33333333-3333-4333-8333-333333333333".to_string(),
                tier: 2,
                path: "records/example.json".to_string(),
                title: Some(json!("Example")),
                tags: None,
            });
        store.save_manifest(&manifest).unwrap();

        // Save notes
        store
            .save_instance_json(
                "records/notes/foundation.json",
                &json!({
                    "instanceId": "11111111-1111-4111-8111-111111111111",
                    "title": "Foundation",
                    "tags": ["meaning-first", "projection"],
                    "sections": [
                        {"name": "purpose", "content": "x", "tags": ["purpose", "projections"]},
                        {"name": "domain", "content": "x", "tags": ["domain"]}
                    ],
                    "sourceRefs": [
                        {"sourceType": "external-document", "sourceId": "source-1"}
                    ]
                }),
            )
            .unwrap();
        store
            .save_instance_json(
                "records/notes/problem.json",
                &json!({
                    "instanceId": "22222222-2222-4222-8222-222222222222",
                    "title": "Problem",
                    "tags": ["problems"],
                    "sections": [
                        {"name": "purpose", "content": "x", "tags": ["purpose"]}
                    ]
                }),
            )
            .unwrap();

        // Save relations
        store
            .save_relations_json(
                "relations/relations.json",
                &json!({
                    "relations": [
                        {"type": "derived-from"},
                        {"relationType": "contains"}
                    ]
                }),
            )
            .unwrap();

        // Save a schema file (as text via load_text_file key)
        store
            .save_instance_json("schemas/note.json", &json!({"title": "note"}))
            .unwrap();

        store
    }

    #[test]
    fn repo_map_summarizes_manifest_and_relations() {
        let store = fixture_store();
        let map = build_repo_map(&store).unwrap();

        assert_eq!(map.repository.title.as_deref(), Some("Fixture Repo"));
        assert_eq!(map.counts.notes, 2);
        assert_eq!(map.counts.records, 1);
        assert_eq!(map.relations_summary.relation_count, 2);
        assert_eq!(map.entry_points, vec!["records/notes/foundation.json"]);
        assert_eq!(map.containers_summary.count, 0);
    }

    #[test]
    fn repo_map_containers_summary_counts_and_types() {
        use srs_core::types::container::Container;
        use std::collections::HashMap;

        fn make_container(id: &str, title: &str, container_type: &str) -> Container {
            Container {
                container_id: id.to_string(),
                title: title.to_string(),
                container_type: Some(container_type.to_string()),
                namespace: None,
                name: None,
                description: None,
                root_instance_ids: None,
                member_instance_ids: None,
                tags: None,
                created_at: None,
                updated_at: None,
                meta: None,
                extra: HashMap::new(),
            }
        }

        let store = fixture_store();
        store
            .save_container(&make_container(
                "aaaaaaaa-aaaa-4aaa-aaaa-aaaaaaaaaaaa",
                "Alpha",
                "feature-set",
            ))
            .unwrap();
        store
            .save_container(&make_container(
                "bbbbbbbb-bbbb-4bbb-bbbb-bbbbbbbbbbbb",
                "Beta",
                "feature-set",
            ))
            .unwrap();
        store
            .save_container(&make_container(
                "cccccccc-cccc-4ccc-cccc-cccccccccccc",
                "Gamma",
                "release",
            ))
            .unwrap();

        let map = build_repo_map(&store).unwrap();
        assert_eq!(map.containers_summary.count, 3);
        assert_eq!(map.containers_summary.types["feature-set"], 2);
        assert_eq!(map.containers_summary.types["release"], 1);
    }

    #[test]
    fn tag_audit_counts_levels_and_duplicates() {
        let store = fixture_store();
        let audit = audit_note_tags(&store).unwrap();

        assert_eq!(audit.total_notes, 2);
        assert!(audit
            .tag_counts
            .iter()
            .any(|count| count.tag == "purpose" && count.count == 2));
        assert!(audit
            .likely_singular_plural_duplicates
            .iter()
            .any(|dup| dup.singular == "projection" && dup.plural == "projections"));
        assert!(audit.singleton_tags.contains(&"domain".to_string()));
    }

    #[test]
    fn foundation_selection_uses_only_signal_tags() {
        let store = fixture_store();
        let signal_tags = vec!["meaning-first".to_string(), "problems".to_string()];
        let foundations = collect_foundation_notes(&store, &signal_tags).unwrap();

        assert_eq!(foundations.notes.len(), 2);
        assert!(foundations.notes.iter().any(|note| {
            note.instance_id == "11111111-1111-4111-8111-111111111111"
                && note.matched_tags.contains(&"meaning-first".to_string())
        }));
    }

    #[test]
    fn migration_packet_is_deterministic_handoff_data() {
        let store = fixture_store();
        let signal_tags = vec!["meaning-first".to_string(), "problems".to_string()];
        let packet = build_migration_packet(&store, "foundation", &signal_tags).unwrap();

        assert_eq!(packet.profile, "foundation");
        assert_eq!(packet.foundation_notes.notes.len(), 2);
        assert!(packet.ai_handoff_guidance.contains("external AI"));
        assert_eq!(packet.source_reference_summary.total_source_refs, 1);
    }

    #[test]
    fn audit_for_note_unknown_id_returns_error() {
        let store = fixture_store();
        let result = audit_note_tags_for_note(&store, "nonexistent-id");
        assert!(result.is_err());
    }

    #[test]
    fn audit_for_note_scopes_to_shared_tags() {
        // fixture_store has foundation (tags: meaning-first, projection) and
        // problem (tags: problems). They share no tags, so scoping to foundation
        // should include only foundation itself.
        let store = fixture_store();
        let audit =
            audit_note_tags_for_note(&store, "11111111-1111-4111-8111-111111111111").unwrap();
        assert_eq!(audit.total_notes, 1);
        // Should contain foundation's tags (note-level + section-level)
        assert!(audit.tag_counts.iter().any(|t| t.tag == "meaning-first"));
    }

    #[test]
    fn audit_for_note_includes_notes_sharing_a_tag() {
        // Add a third note that shares a tag with foundation
        let store = fixture_store();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .instance_index
            .push(crate::index::InstanceIndexEntry {
                instance_id: "44444444-4444-4444-8444-444444444444".to_string(),
                tier: 0,
                path: "records/notes/sibling.json".to_string(),
                title: Some(json!("Sibling")),
                tags: Some(vec!["meaning-first".to_string()]),
            });
        store.save_manifest(&manifest).unwrap();
        store
            .save_instance_json(
                "records/notes/sibling.json",
                &json!({
                    "instanceId": "44444444-4444-4444-8444-444444444444",
                    "title": "Sibling",
                    "tags": ["meaning-first"],
                    "sections": []
                }),
            )
            .unwrap();

        let audit =
            audit_note_tags_for_note(&store, "11111111-1111-4111-8111-111111111111").unwrap();
        // foundation + sibling both have meaning-first → 2 notes in scope
        assert_eq!(audit.total_notes, 2);
        let meaning_count = audit
            .tag_counts
            .iter()
            .find(|t| t.tag == "meaning-first")
            .map(|t| t.count)
            .unwrap_or(0);
        assert_eq!(meaning_count, 2);
    }

    #[test]
    fn audit_for_note_excludes_unrelated_notes() {
        // problem note has tags: ["problems"] — no overlap with foundation
        // Scoping to foundation should not include problem
        let store = fixture_store();
        let audit =
            audit_note_tags_for_note(&store, "11111111-1111-4111-8111-111111111111").unwrap();
        // problem note should be excluded
        assert!(!audit.tag_counts.iter().any(|t| t.tag == "problems"));
    }
}
