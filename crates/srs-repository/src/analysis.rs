use crate::error::RepositoryError;
use crate::loader::load_note_relative;
use crate::manifest::{load_manifest, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const AI_HANDOFF_GUIDANCE: &str = "This packet is deterministic repository data for external AI-assisted migration. The SRS CLI and library do not infer, extract, or decide semantic migrations. An external AI may propose candidate higher-tier records from this packet, but humans must review, revise, accept, and commit meaning.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMap {
    pub repository: RepositorySummary,
    pub counts: CountsSummary,
    pub schemas: SchemaSummary,
    pub source_documents: SourceDocumentsSummary,
    pub relations_summary: RelationsSummary,
    pub ai_guidance: Option<Value>,
    pub entry_points: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositorySummary {
    pub repository_id: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub conformance: Option<String>,
    pub root: String,
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
    pub files: Vec<String>,
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
    pub path: String,
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
    pub path: String,
    pub title: Option<String>,
}

pub fn build_repo_map(repo_root: &Path) -> Result<RepoMap, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    build_repo_map_from_manifest(repo_root, &manifest)
}

fn build_repo_map_from_manifest(
    repo_root: &Path,
    manifest: &Manifest,
) -> Result<RepoMap, RepositoryError> {
    let counts = summarize_counts(manifest);
    let relations_summary = summarize_relations(repo_root, manifest)?;
    let schemas = summarize_schemas(repo_root)?;
    let source_documents = summarize_source_documents(manifest);
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
        repository: summarize_repository(repo_root, manifest),
        counts,
        schemas,
        source_documents,
        relations_summary,
        ai_guidance,
        entry_points,
    })
}

pub fn audit_note_tags(repo_root: &Path) -> Result<TagAudit, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    audit_note_tags_from_manifest(repo_root, &manifest)
}

fn audit_note_tags_from_manifest(
    repo_root: &Path,
    manifest: &Manifest,
) -> Result<TagAudit, RepositoryError> {
    let mut note_level: BTreeMap<String, TagAccumulator> = BTreeMap::new();
    let mut section_level: BTreeMap<String, TagAccumulator> = BTreeMap::new();
    let mut total_notes = 0;

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }
        let Ok(note) = load_note_relative(repo_root, entry.path()) else {
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
    repo_root: &Path,
    signal_tags: &[&str],
) -> Result<FoundationNoteSet, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    collect_foundation_notes_from_manifest(repo_root, &manifest, signal_tags)
}

fn collect_foundation_notes_from_manifest(
    repo_root: &Path,
    manifest: &Manifest,
    signal_tags: &[&str],
) -> Result<FoundationNoteSet, RepositoryError> {
    let signal_tags: BTreeSet<String> = signal_tags.iter().map(|tag| (*tag).to_string()).collect();
    let mut notes = Vec::new();

    for entry in &manifest.instance_index {
        if !entry.is_note() {
            continue;
        }
        let Ok(note) = load_note_relative(repo_root, entry.path()) else {
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
            path: entry.path().to_string(),
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

    notes.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(FoundationNoteSet {
        signal_tags: signal_tags.into_iter().collect(),
        notes,
    })
}

pub fn build_migration_packet(
    repo_root: &Path,
    profile: &str,
    foundation_signal_tags: &[&str],
) -> Result<MigrationPacket, RepositoryError> {
    let manifest = load_manifest(repo_root)?;
    let repo_map = build_repo_map_from_manifest(repo_root, &manifest)?;
    let tag_audit = audit_note_tags_from_manifest(repo_root, &manifest)?;
    let foundation_notes =
        collect_foundation_notes_from_manifest(repo_root, &manifest, foundation_signal_tags)?;
    let manifest_entries = manifest
        .instance_index
        .iter()
        .map(|entry| ManifestEntrySummary {
            instance_id: entry.instance_id().to_string(),
            tier: entry.tier(),
            path: entry.path().to_string(),
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

fn summarize_repository(repo_root: &Path, manifest: &Manifest) -> RepositorySummary {
    RepositorySummary {
        repository_id: string_extra(manifest, "repositoryId"),
        title: string_extra(manifest, "title"),
        description: string_extra(manifest, "description"),
        conformance: string_extra(manifest, "conformance"),
        root: repo_root.display().to_string(),
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

fn summarize_schemas(repo_root: &Path) -> Result<SchemaSummary, RepositoryError> {
    let schema_dir = repo_root.join("schemas");
    let mut schema_paths = Vec::new();
    if schema_dir.exists() {
        collect_json_paths(repo_root, &schema_dir, &mut schema_paths)?;
    }
    schema_paths.sort();

    let package_path = if repo_root.join("package/package.json").exists() {
        Some("package/package.json".to_string())
    } else if repo_root.join("package").exists() {
        Some("package".to_string())
    } else {
        None
    };

    Ok(SchemaSummary {
        schema_dir: "schemas".to_string(),
        schema_paths,
        package_path,
    })
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
    repo_root: &Path,
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

    let full_path = repo_root.join(&relative_path);
    if !full_path.exists() {
        return Ok(RelationsSummary {
            relations_path,
            exists: false,
            relation_count: 0,
            relation_types: BTreeMap::new(),
        });
    }

    let content = fs::read_to_string(&full_path).map_err(|source| RepositoryError::Io {
        path: full_path.clone(),
        source,
    })?;
    let value: Value =
        serde_json::from_str(&content).map_err(|source| RepositoryError::ManifestParse {
            path: full_path.clone(),
            source,
        })?;
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

fn collect_json_paths(
    repo_root: &Path,
    dir: &Path,
    paths: &mut Vec<String>,
) -> Result<(), RepositoryError> {
    let entries = fs::read_dir(dir).map_err(|source| RepositoryError::Io {
        path: dir.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| RepositoryError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_paths(repo_root, &path, paths)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            paths.push(relative_string(repo_root, &path));
        }
    }

    Ok(())
}

fn to_tag_counts(map: BTreeMap<String, TagAccumulator>) -> Vec<TagCount> {
    let mut counts: Vec<_> = map
        .into_iter()
        .map(|(tag, acc)| TagCount {
            tag,
            count: acc.count,
            files: acc.files.into_iter().collect(),
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

fn string_extra(manifest: &Manifest, key: &str) -> Option<String> {
    manifest
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn relative_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_json(path: PathBuf, value: Value) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    fn fixture_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".srs")).unwrap();
        write_json(
            temp.path().join("manifest.json"),
            serde_json::json!({
                "repositoryId": "4172fada-bc38-5479-ac18-4be3194a68ca",
                "title": "Fixture Repo",
                "relationsPath": "relations/relations.json",
                "sourceDocumentsPath": "source-documents",
                "aiGuidance": {
                    "suggestedEntryPoints": ["records/notes/foundation.json"]
                },
                "instanceIndex": [
                    {
                        "instanceId": "11111111-1111-4111-8111-111111111111",
                        "tier": 0,
                        "path": "records/notes/foundation.json",
                        "title": "Foundation"
                    },
                    {
                        "instanceId": "22222222-2222-4222-8222-222222222222",
                        "tier": 0,
                        "path": "records/notes/problem.json",
                        "title": "Problem"
                    },
                    {
                        "instanceId": "33333333-3333-4333-8333-333333333333",
                        "tier": 2,
                        "path": "records/example.json",
                        "title": "Example"
                    }
                ]
            }),
        );
        write_json(
            temp.path().join("records/notes/foundation.json"),
            serde_json::json!({
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
        );
        write_json(
            temp.path().join("records/notes/problem.json"),
            serde_json::json!({
                "instanceId": "22222222-2222-4222-8222-222222222222",
                "title": "Problem",
                "tags": ["problems"],
                "sections": [
                    {"name": "purpose", "content": "x", "tags": ["purpose"]}
                ]
            }),
        );
        write_json(
            temp.path().join("relations/relations.json"),
            serde_json::json!({
                "relations": [
                    {"type": "derived-from"},
                    {"relationType": "contains"}
                ]
            }),
        );
        write_json(
            temp.path().join("schemas/note.json"),
            serde_json::json!({"title": "note"}),
        );
        temp
    }

    #[test]
    fn repo_map_summarizes_manifest_and_relations() {
        let temp = fixture_repo();
        let map = build_repo_map(temp.path()).unwrap();

        assert_eq!(map.repository.title.as_deref(), Some("Fixture Repo"));
        assert_eq!(map.counts.notes, 2);
        assert_eq!(map.counts.records, 1);
        assert_eq!(map.relations_summary.relation_count, 2);
        assert_eq!(map.entry_points, vec!["records/notes/foundation.json"]);
        assert_eq!(map.schemas.schema_paths, vec!["schemas/note.json"]);
    }

    #[test]
    fn tag_audit_counts_levels_and_duplicates() {
        let temp = fixture_repo();
        let audit = audit_note_tags(temp.path()).unwrap();

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
        let temp = fixture_repo();
        let foundations =
            collect_foundation_notes(temp.path(), &["meaning-first", "problems"]).unwrap();

        assert_eq!(foundations.notes.len(), 2);
        assert!(foundations.notes.iter().any(|note| {
            note.path == "records/notes/foundation.json"
                && note.matched_tags.contains(&"meaning-first".to_string())
        }));
    }

    #[test]
    fn migration_packet_is_deterministic_handoff_data() {
        let temp = fixture_repo();
        let packet =
            build_migration_packet(temp.path(), "foundation", &["meaning-first", "problems"])
                .unwrap();

        assert_eq!(packet.profile, "foundation");
        assert_eq!(packet.foundation_notes.notes.len(), 2);
        assert!(packet.ai_handoff_guidance.contains("external AI"));
        assert_eq!(packet.source_reference_summary.total_source_refs, 1);
    }
}
