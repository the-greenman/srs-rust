use crate::container_service;
use crate::error::RepositoryError;
use crate::package::Package;
use crate::record_label;
use crate::record_store::{self, LoadedInstance};
use crate::relation_graph;
use crate::relation_service;
use crate::store::RepositoryStore;
use crate::tree_service;
use crate::writer::slugify_instance_name;
use srs_core::types::field::{EditorHint, Field};
use srs_core::types::field_type::{Datatype, StringFormat};
use srs_core::types::relation::Relation;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug)]
pub struct OkfExportInput {
    pub container_id: String,
}

/// A relation from this entry to another entry in the same bundle, already
/// resolved to that entry's exported path — the writer has no id→path index
/// of its own, so resolution happens here, once, at export time.
#[derive(Debug, Clone)]
pub struct OkfRelationLink {
    pub relation_type: String,
    pub target_path: String,
    pub target_display_label: String,
}

#[derive(Debug)]
pub struct OkfEntry {
    pub path: String,
    pub display_label: String,
    pub instance_id: String,
    pub type_label: String,
    pub field_pairs: Vec<(String, String)>,
    /// `(heading, content)` — text-formatted field values (RFC-032
    /// `datatype: string` with `format: markdown`, or a `textarea`/`rich-text`
    /// `editorHint`), rendered as body sections instead of frontmatter scalars
    /// (srs-rust#1106). Empty for a Tier-0 note (its content is `note_text`).
    pub body_sections: Vec<(String, String)>,
    pub note_text: Option<String>,
    /// Outgoing edges whose target is also in this bundle, sorted by
    /// (relation_type, target_path) for deterministic output.
    pub outgoing_relations: Vec<OkfRelationLink>,
}

#[derive(Debug)]
pub struct OkfBundle {
    pub container_title: String,
    pub entries: Vec<OkfEntry>,
    pub diagnostics: Vec<String>,
}

pub fn export_okf_bundle(
    store: &dyn RepositoryStore,
    input: OkfExportInput,
) -> Result<OkfBundle, RepositoryError> {
    let container = container_service::get_container(store, &input.container_id)?;
    let member_ids = container_service::list_container_members(store, &input.container_id)?;

    let package = store.load_package()?;
    let (fni, ifi) = record_label::build_label_indexes_from_package(&package);
    let all_relations = relation_service::load_relations(store)?;
    // RFC-034 [R1]: a container's own direct membership can duplicate what a
    // sibling's `contains` subtree already reaches (the same corpus shape
    // `tree_service::child_ids` merges as extra part-of children) — the
    // `visited` set below is what keeps that from being walked twice.
    let container_children = container_service::direct_children_by_root(store)?;

    let mut member_instances: Vec<LoadedInstance> = Vec::new();
    let mut diagnostics: Vec<String> = Vec::new();

    for id in &member_ids {
        match record_store::get_instance_by_id(store, id)? {
            Some(inst) => member_instances.push(inst),
            None => diagnostics.push(format!("instance not found: {id}")),
        }
    }

    // Direct members set the top-level order (unchanged from before); everything
    // each one transitively `contains` is then pulled in by descending the
    // tree rather than dropped, per srs-rust#1104.
    let sorted_members = relation_graph::sort_by_precedes_chain(member_instances, &all_relations);

    let mut entries = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut used_slots: HashSet<String> = HashSet::new();
    for member in &sorted_members {
        collect_entries(
            store,
            member.instance_id(),
            "",
            &all_relations,
            &container_children,
            &package,
            &fni,
            &ifi,
            &mut visited,
            &mut used_slots,
            &mut entries,
            &mut diagnostics,
        )?;
    }

    attach_relation_links(&mut entries, &all_relations, &mut diagnostics);

    Ok(OkfBundle {
        container_title: container.title,
        entries,
        diagnostics,
    })
}

/// Resolves every relation whose source is a bundle member to its target's
/// exported path, and attaches the result to the source entry. A target
/// outside the bundle cannot be linked — that would be a dangling markdown
/// link indistinguishable from a rename, exactly the failure mode OKF export
/// exists to avoid reproducing — so it is reported as a diagnostic instead.
fn attach_relation_links(
    entries: &mut [OkfEntry],
    all_relations: &[Relation],
    diagnostics: &mut Vec<String>,
) {
    let id_to_path: HashMap<&str, &str> = entries
        .iter()
        .map(|e| (e.instance_id.as_str(), e.path.as_str()))
        .collect();
    let id_to_label: HashMap<&str, &str> = entries
        .iter()
        .map(|e| (e.instance_id.as_str(), e.display_label.as_str()))
        .collect();
    let member_ids: HashSet<&str> = id_to_path.keys().copied().collect();

    let mut links_by_source: HashMap<&str, Vec<OkfRelationLink>> = HashMap::new();
    for rel in all_relations {
        let source = rel.source_instance_id.as_str();
        if !member_ids.contains(source) {
            continue;
        }
        let target = rel.target_instance_id.as_str();
        match (id_to_path.get(target), id_to_label.get(target)) {
            (Some(path), Some(label)) => {
                links_by_source
                    .entry(source)
                    .or_default()
                    .push(OkfRelationLink {
                        relation_type: rel.relation_type.clone(),
                        target_path: (*path).to_string(),
                        target_display_label: (*label).to_string(),
                    });
            }
            _ => diagnostics.push(format!(
                "relation {} ({}) from {source} points outside the bundle: {target}",
                rel.relation_id, rel.relation_type
            )),
        }
    }

    for entry in entries.iter_mut() {
        if let Some(mut links) = links_by_source.remove(entry.instance_id.as_str()) {
            links.sort_by(|a, b| {
                a.relation_type
                    .cmp(&b.relation_type)
                    .then_with(|| a.target_path.cmp(&b.target_path))
            });
            entry.outgoing_relations = links;
        }
    }
}

/// Groups an entry's outgoing links by relation type for frontmatter
/// rendering — one key per relation type, list of target paths.
pub fn group_relation_links_by_type(links: &[OkfRelationLink]) -> Vec<(String, Vec<String>)> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for link in links {
        map.entry(link.relation_type.clone())
            .or_default()
            .push(link.target_path.clone());
    }
    map.into_iter().collect()
}

/// Walks the `contains` tree rooted at `instance_id`, emitting one
/// [`OkfEntry`] per instance the *first* time it is reached. A node with
/// children becomes a directory (named after its own file, sans `.md`)
/// holding an `index.md` for its own content plus one file per child; a leaf
/// node is written directly into `dir_prefix`.
///
/// `visited` is shared across the whole export, not per top-level member: a
/// `contains` node can legally be reachable from more than one parent (a
/// declared container membership that overlaps a sibling's subtree — the
/// same corpus shape `render_service`'s container-subset fix addressed,
/// commit 3a20e59), and re-walking it per parent duplicates both the output
/// and the work, compounding with depth. First-reached wins; later paths to
/// an already-visited instance are skipped entirely, without recursing
/// further.
#[allow(clippy::too_many_arguments)]
fn collect_entries(
    store: &dyn RepositoryStore,
    instance_id: &str,
    dir_prefix: &str,
    relations: &[Relation],
    container_children: &HashMap<String, Vec<String>>,
    package: &Package,
    fni: &record_label::FieldNameIndex,
    ifi: &record_label::IdentityFieldIndex,
    visited: &mut HashSet<String>,
    used_slots: &mut HashSet<String>,
    entries: &mut Vec<OkfEntry>,
    diagnostics: &mut Vec<String>,
) -> Result<(), RepositoryError> {
    if !visited.insert(instance_id.to_string()) {
        return Ok(());
    }

    let instance = match record_store::get_instance_by_id(store, instance_id)? {
        Some(inst) => inst,
        None => {
            diagnostics.push(format!("instance not found: {instance_id}"));
            return Ok(());
        }
    };

    let child_ids = tree_service::child_ids(
        store,
        instance_id,
        "contains",
        relations,
        container_children,
    )?;

    let mut entry = okf_entry_from_instance(&instance, package, fni, ifi);
    let mut own_filename = entry.path.clone();

    // Slug + first-8-hex-chars can collide: this corpus has hand-authored
    // fixture/example instances that intentionally share an 8-char id
    // prefix (and sometimes a display label too) with their siblings. A
    // collision here would otherwise silently overwrite one sibling's file
    // with another's — fall back to the full instance id, which is unique
    // by construction, instead.
    if !used_slots.insert(join_path(dir_prefix, &own_filename)) {
        own_filename = format!("{instance_id}.md");
        used_slots.insert(join_path(dir_prefix, &own_filename));
    }

    if child_ids.is_empty() {
        entry.path = join_path(dir_prefix, &own_filename);
        entries.push(entry);
    } else {
        let dir_name = own_filename.strip_suffix(".md").unwrap_or(&own_filename);
        let node_dir = join_path(dir_prefix, dir_name);
        entry.path = join_path(&node_dir, "index.md");
        entries.push(entry);
        for child_id in &child_ids {
            collect_entries(
                store,
                child_id,
                &node_dir,
                relations,
                container_children,
                package,
                fni,
                ifi,
                visited,
                used_slots,
                entries,
                diagnostics,
            )?;
        }
    }

    Ok(())
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn okf_entry_from_instance(
    instance: &LoadedInstance,
    package: &Package,
    fni: &record_label::FieldNameIndex,
    ifi: &record_label::IdentityFieldIndex,
) -> OkfEntry {
    match instance {
        LoadedInstance::Record(r) => {
            let display_label = record_label::record_display_label(r, ifi, fni);
            let slug = slugify_instance_name(&display_label);
            let id8 = &r.instance_id[..r.instance_id.len().min(8)];
            let path = if slug.is_empty() {
                format!("{id8}.md")
            } else {
                format!("{slug}-{id8}.md")
            };
            let type_label = format!("{}/{}", r.type_namespace, r.type_name);
            // RFC-039: keys are Field.name already — no id→name index needed.
            let text_fields = text_field_display_labels(package, &r.type_id, r.type_version);
            let mut field_pairs = Vec::new();
            let mut body_sections = Vec::new();
            for (name, value) in r.field_values.iter() {
                match text_fields.get(name) {
                    Some(heading) => {
                        let content = value
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| value.to_string());
                        body_sections.push((heading.clone(), content));
                    }
                    None => field_pairs.push((name.clone(), value.to_string())),
                }
            }
            OkfEntry {
                path,
                display_label,
                instance_id: r.instance_id.clone(),
                type_label,
                field_pairs,
                body_sections,
                note_text: None,
                outgoing_relations: Vec::new(),
            }
        }
        LoadedInstance::Note(n) => {
            let display_label = n
                .title
                .as_deref()
                .unwrap_or_else(|| &n.instance_id[..n.instance_id.len().min(8)])
                .to_string();
            let slug = slugify_instance_name(&display_label);
            let id8 = &n.instance_id[..n.instance_id.len().min(8)];
            let path = if slug.is_empty() {
                format!("{id8}.md")
            } else {
                format!("{slug}-{id8}.md")
            };
            let note_text = if n.sections.is_empty() {
                None
            } else {
                Some(
                    n.sections
                        .iter()
                        .map(|s| s.content.as_str())
                        .collect::<Vec<_>>()
                        .join("\n\n"),
                )
            };
            OkfEntry {
                path,
                display_label,
                instance_id: n.instance_id.clone(),
                type_label: "note".to_string(),
                field_pairs: vec![],
                body_sections: vec![],
                note_text,
                outgoing_relations: Vec::new(),
            }
        }
    }
}

/// A field renders as a body section rather than a frontmatter scalar when it
/// is declared for long-form prose: a `textarea`/`rich-text` `editorHint`
/// (already presentation-only per RFC-032, RFC-015's "view-owned, not
/// type-owned" test applied to a Field-level hint), or `datatype: string`
/// with `format: markdown` for a Field that predates `editorHint` adoption.
fn is_text_field(field: &Field) -> bool {
    if matches!(
        field.editor_hint,
        Some(EditorHint::Textarea) | Some(EditorHint::RichText)
    ) {
        return true;
    }
    field.field_type.datatype == Datatype::String
        && field.field_type.format == Some(StringFormat::Markdown)
}

/// `field name → display heading` for every text-classified field effective
/// on `type_id`/`type_version` (RFC-020 `displayLabel`, falling back to the
/// Field's own name — the same fallback `FieldView.display_label` uses).
/// An unresolvable Type or a broken inheritance chain degrades to "no text
/// fields" rather than failing the export — consistent with
/// `identity_field_index_from_package`'s graceful-degradation contract.
fn text_field_display_labels(
    package: &Package,
    type_id: &str,
    type_version: u32,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let Some(record_type) = package.resolve_type(type_id, type_version) else {
        return result;
    };
    let Ok(effective_fields) = package.effective_fields(record_type) else {
        return result;
    };
    for fa in &effective_fields {
        let Some(field) = package.resolve_field(&fa.field_id) else {
            continue;
        };
        if is_text_field(field) {
            let heading = fa
                .display_label
                .clone()
                .unwrap_or_else(|| field.name.clone());
            result.insert(field.name.clone(), heading);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service::{add_member, create_container};
    use crate::store::memory::MemoryStore;
    use srs_core::types::container::Container;
    use srs_core::types::note::{Note, NoteSection};
    use srs_core::types::record::{FieldValues, Record};
    use srs_core::types::relation::Relation;

    fn make_store() -> MemoryStore {
        MemoryStore::default()
    }

    fn minimal_container(id: &str, title: &str) -> Container {
        Container {
            container_id: id.to_string(),
            title: title.to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            child_container_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn minimal_record(id: &str, created_at: Option<&str>) -> Record {
        Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "t-test-0001".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "item".to_string(),
            field_values: FieldValues::new(),
            lifecycle_state: None,
            tags: None,
            created_at: created_at.map(|s| s.to_string()),
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn minimal_note(id: &str, title: Option<&str>, sections: Vec<NoteSection>) -> Note {
        Note {
            instance_id: id.to_string(),
            title: title.map(|s| s.to_string()),
            tags: None,
            sections,
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    fn make_section(name: &str, content: &str) -> NoteSection {
        NoteSection {
            name: name.to_string(),
            label: None,
            content: content.to_string(),
            content_hint: None,
            tags: None,
        }
    }

    fn make_precedes_relation(id: &str, src: &str, tgt: &str) -> Relation {
        make_relation(id, "precedes", src, tgt)
    }

    fn make_relation(id: &str, relation_type: &str, src: &str, tgt: &str) -> Relation {
        Relation {
            relation_id: id.to_string(),
            relation_type: relation_type.to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        }
    }

    fn write_relations(store: &MemoryStore, relations: &[Relation]) {
        let rel_json = serde_json::json!({
            "relations": relations
                .iter()
                .map(|r| serde_json::json!({
                    "relationId": r.relation_id,
                    "relationType": r.relation_type,
                    "sourceInstanceId": r.source_instance_id,
                    "targetInstanceId": r.target_instance_id,
                }))
                .collect::<Vec<_>>()
        });
        crate::store::write_relations_standalone_for_test(store, &rel_json);
    }

    #[test]
    fn empty_container_returns_empty_bundle() {
        let store = make_store();
        let c = create_container(&store, minimal_container("", "Empty")).unwrap();
        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(bundle.container_title, "Empty");
        assert!(bundle.entries.is_empty());
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn record_member_produces_entry_with_correct_path_and_type_label() {
        let store = make_store();
        let r = minimal_record(
            "00000001-aabb-4ccd-8eef-000000000001",
            Some("2026-01-01T00:00:00Z"),
        );
        store.save_record(&r).unwrap();
        let c = create_container(&store, minimal_container("", "Sprint")).unwrap();
        add_member(&store, &c.container_id, &r.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.instance_id, r.instance_id);
        assert_eq!(entry.type_label, "com.test/item");
        assert!(entry.path.ends_with(".md"));
        assert!(bundle.diagnostics.is_empty());
    }

    #[test]
    fn note_member_produces_note_text_from_sections() {
        let store = make_store();
        let sections = vec![
            make_section("intro", "First paragraph"),
            make_section("body", "Second paragraph"),
        ];
        let n = minimal_note(
            "00000002-aabb-4ccd-8eef-000000000002",
            Some("My Note"),
            sections,
        );
        store.save_note(&n).unwrap();
        let c = create_container(&store, minimal_container("", "Docs")).unwrap();
        add_member(&store, &c.container_id, &n.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.display_label, "My Note");
        assert_eq!(entry.type_label, "note");
        let text = entry.note_text.as_deref().unwrap();
        assert!(text.contains("First paragraph"));
        assert!(text.contains("Second paragraph"));
    }

    // A dangling container-member reference is a fatal [R13] catalog diagnostic,
    // and okf export is an ordinary operation — it fails like every other
    // non-`repo validate` caller ([R24]). The pre-RFC-038 contract here emitted a
    // partial bundle with a soft diagnostic; exporting silently-incomplete
    // content from an incoherent repository is precisely what [R24] exists to
    // stop, and a second validate-style exemption would be a second way to do
    // the same thing. Owner ruling (2026-08-11): experimental surface, take the
    // simplest coherent path — no exemption.
    #[test]
    fn missing_instance_fails_the_export() {
        let store = make_store();
        // Every service writer now rejects an id that resolves to nothing —
        // `add_member` (srs-rust#841) and `create_container` (srs-rust#845) — so
        // the dangling state this test is about is planted through the ADR-045
        // repair seam, the one surface that can still express it.
        let mut c = minimal_container("550e8400-e29b-41d4-a716-446655440099", "Partial");
        c.member_instance_ids = Some(vec!["does-not-exist".to_string()]);
        store.save_container_unchecked(&c).unwrap();

        let err = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap_err();

        assert!(
            matches!(&err, RepositoryError::CatalogLoad { first, .. } if first.contains("does-not-exist")),
            "expected a fatal [R13] catalog diagnostic naming the dangling id, got: {err:?}"
        );
    }

    #[test]
    fn nonexistent_container_returns_error() {
        let store = make_store();
        let err = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: "no-such-container".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::ContainerNotFound { container_id } if container_id == "no-such-container"
        ));
    }

    #[test]
    fn note_with_no_title_falls_back_to_id_prefix() {
        let store = make_store();
        let note_id = "0bcdef01-1111-4111-8111-111111111111";
        let n = minimal_note(note_id, None, vec![make_section("main", "Some text")]);
        store.save_note(&n).unwrap();
        let c = create_container(&store, minimal_container("", "Fallback")).unwrap();
        add_member(&store, &c.container_id, note_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        // display_label falls back to first 8 chars of instance_id
        assert_eq!(entry.display_label, &note_id[..8]);
        assert!(entry.path.ends_with(".md"));
    }

    #[test]
    fn precedes_relation_orders_members() {
        let store = make_store();
        let r1 = minimal_record(
            "00000011-aaaa-4001-8000-000000000011",
            Some("2026-01-01T00:00:00Z"),
        );
        let r2 = minimal_record(
            "00000012-aaaa-4002-8000-000000000012",
            Some("2026-01-02T00:00:00Z"),
        );
        let r3 = minimal_record(
            "00000013-aaaa-4003-8000-000000000013",
            Some("2026-01-03T00:00:00Z"),
        );
        store.save_record(&r1).unwrap();
        store.save_record(&r2).unwrap();
        store.save_record(&r3).unwrap();

        let c = create_container(&store, minimal_container("", "Ordered")).unwrap();
        // Add in reverse order to verify sort overrides insertion order
        add_member(&store, &c.container_id, &r3.instance_id).unwrap();
        add_member(&store, &c.container_id, &r1.instance_id).unwrap();
        add_member(&store, &c.container_id, &r2.instance_id).unwrap();

        let relations = [
            make_precedes_relation(
                "eeeeeeee-0000-4000-8000-000000000001",
                &r1.instance_id,
                &r2.instance_id,
            ),
            make_precedes_relation(
                "eeeeeeee-0000-4000-8000-000000000002",
                &r2.instance_id,
                &r3.instance_id,
            ),
        ];
        let rel_json = serde_json::json!({
            "relations": relations
                .iter()
                .map(|r| serde_json::json!({
                    "relationId": r.relation_id,
                    "relationType": r.relation_type,
                    "sourceInstanceId": r.source_instance_id,
                    "targetInstanceId": r.target_instance_id,
                }))
                .collect::<Vec<_>>()
        });
        crate::store::write_relations_standalone_for_test(&store, &rel_json);

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 3);
        assert_eq!(bundle.entries[0].instance_id, r1.instance_id);
        assert_eq!(bundle.entries[1].instance_id, r2.instance_id);
        assert_eq!(bundle.entries[2].instance_id, r3.instance_id);
    }

    #[test]
    fn record_with_field_values_produces_field_pairs() {
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::{AiGuidance, Field, FieldType};
        use std::path::PathBuf;

        let field_id = "f-title-0001-0000-0000-0000000000001".to_string();
        let field = Field {
            schema: None,
            id: field_id.clone(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            field_type: FieldType::string(),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![field],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package);

        let mut r = minimal_record(
            "00000021-f1e1-4d00-8000-000000000021",
            Some("2026-01-01T00:00:00Z"),
        );
        r.field_values = {
            let mut fv = FieldValues::new();
            fv.insert("title", serde_json::json!("My Title"));
            fv
        };
        store.save_record(&r).unwrap();
        let c = create_container(&store, minimal_container("", "Fields")).unwrap();
        add_member(&store, &c.container_id, &r.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.field_pairs.len(), 1);
        assert_eq!(entry.field_pairs[0].0, "title");
        // serde_json stringifies strings with surrounding quotes (valid JSON/YAML scalar)
        assert_eq!(entry.field_pairs[0].1, "\"My Title\"");
    }

    // Regression for srs-rust#1106: a `text`-formatted field (RFC-032
    // `datatype: string` + `format: markdown`, `editorHint: textarea` — the
    // real `description` field's shape) must render as a `## <heading>` body
    // section, not a frontmatter scalar with embedded `\n` escapes. Requires a
    // resolvable RecordType so `text_field_display_labels` can classify the
    // record's fields; `title` (a bare `datatype: string`, no format/hint)
    // stays in frontmatter to prove the split, not a blanket body dump.
    #[test]
    fn record_with_text_field_renders_body_section_scalar_stays_in_frontmatter() {
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::{EditorHint, Field};
        use srs_core::types::field_type::{Datatype, FieldType, StringFormat};
        use srs_core::types::record_type::{FieldAssignment, RecordType};
        use std::path::PathBuf;

        let title_field_id = "f-title-0000-0000-0000-000000000001".to_string();
        let desc_field_id = "f-desc-0000-0000-0000-0000000000002".to_string();

        let title_field = Field {
            schema: None,
            id: title_field_id.clone(),
            namespace: "com.test".to_string(),
            name: "title".to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: None,
            field_type: FieldType::string(),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let mut desc_field_type = FieldType::new(Datatype::String);
        desc_field_type.format = Some(StringFormat::Markdown);
        let desc_field = Field {
            schema: None,
            id: desc_field_id.clone(),
            namespace: "com.test".to_string(),
            name: "description".to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: None,
            field_type: desc_field_type,
            editor_hint: Some(EditorHint::Textarea),
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let record_type = RecordType {
            schema: None,
            id: "t-test-0001".to_string(),
            namespace: "com.test".to_string(),
            name: "item".to_string(),
            version: 1,
            description: String::new(),
            fields: vec![
                FieldAssignment {
                    field_id: title_field_id,
                    order: 1,
                    required: true,
                    display_label: None,
                    description: None,
                },
                FieldAssignment {
                    field_id: desc_field_id,
                    order: 2,
                    required: false,
                    display_label: Some("Description".to_string()),
                    description: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            ai_guidance: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![title_field, desc_field],
            record_types: vec![record_type],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package);

        let mut r = minimal_record(
            "00000022-f1e1-4d00-8000-000000000022",
            Some("2026-01-01T00:00:00Z"),
        );
        let long_prose = "First paragraph.\n\nSecond paragraph with detail.";
        r.field_values = {
            let mut fv = FieldValues::new();
            fv.insert("title", serde_json::json!("My Title"));
            fv.insert("description", serde_json::json!(long_prose));
            fv
        };
        store.save_record(&r).unwrap();
        let c = create_container(&store, minimal_container("", "TextFields")).unwrap();
        add_member(&store, &c.container_id, &r.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];

        // Scalar field: still a frontmatter pair.
        assert_eq!(entry.field_pairs.len(), 1);
        assert_eq!(entry.field_pairs[0].0, "title");
        assert_eq!(entry.field_pairs[0].1, "\"My Title\"");

        // Text field: a body section, raw (unescaped, un-quoted) content —
        // not a JSON-quoted frontmatter scalar with embedded `\n` escapes.
        assert_eq!(entry.body_sections.len(), 1);
        assert_eq!(entry.body_sections[0].0, "Description");
        assert_eq!(entry.body_sections[0].1, long_prose);
    }

    // A record with only scalar fields must keep an empty body — no
    // regression from the srs-rust#1106 fix for the common case.
    #[test]
    fn record_with_only_scalar_fields_has_empty_body_sections() {
        let store = make_store();
        let r = minimal_record(
            "00000023-f1e1-4d00-8000-000000000023",
            Some("2026-01-01T00:00:00Z"),
        );
        store.save_record(&r).unwrap();
        let c = create_container(&store, minimal_container("", "NoText")).unwrap();
        add_member(&store, &c.container_id, &r.instance_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        assert!(bundle.entries[0].body_sections.is_empty());
    }

    #[test]
    fn mixed_record_and_note_precedes_ordering_respected() {
        let store = make_store();
        let note_id = "00000031-1234-4001-8000-000000000031";
        let rec_id = "00000032-1234-4002-8000-000000000032";
        let n = minimal_note(note_id, Some("First"), vec![make_section("s", "text")]);
        let r = minimal_record(rec_id, Some("2026-01-02T00:00:00Z"));
        store.save_note(&n).unwrap();
        store.save_record(&r).unwrap();

        let c = create_container(&store, minimal_container("", "Mixed")).unwrap();
        // Add record first, note second — precedes should reverse the order
        add_member(&store, &c.container_id, &r.instance_id).unwrap();
        add_member(&store, &c.container_id, &n.instance_id).unwrap();

        let rel_json = serde_json::json!({
            "relations": [{
                "relationId": "eeeeeeee-0000-4000-8000-000000000011",
                "relationType": "precedes",
                "sourceInstanceId": note_id,
                "targetInstanceId": rec_id,
            }]
        });
        crate::store::write_relations_standalone_for_test(&store, &rel_json);

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 2);
        assert_eq!(bundle.entries[0].instance_id, note_id);
        assert_eq!(bundle.entries[0].type_label, "note");
        assert_eq!(bundle.entries[1].instance_id, rec_id);
        assert_eq!(bundle.entries[1].type_label, "com.test/item");
    }

    #[test]
    fn display_label_that_slugifies_to_empty_uses_bare_id_path() {
        let store = make_store();
        let note_id = "0bcdef02-2222-4222-8222-222222222222";
        // Title consisting only of non-alphanumeric chars → slug is empty
        let n = minimal_note(note_id, Some("!!!"), vec![]);
        store.save_note(&n).unwrap();
        let c = create_container(&store, minimal_container("", "Symbols")).unwrap();
        add_member(&store, &c.container_id, note_id).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry = &bundle.entries[0];
        assert_eq!(entry.display_label, "!!!");
        // path should be just id8.md when slug is empty
        let id8 = &note_id[..8];
        assert_eq!(entry.path, format!("{id8}.md"));
    }

    // Regression for srs-rust#1104: the export must descend the `contains`
    // tree below each direct container member, not just export the flat
    // member list. A two-level chain (member -> child -> grandchild) proves
    // both levels of descent.
    #[test]
    fn contains_descent_reaches_grandchild_two_levels_deep() {
        let store = make_store();
        let root_id = "00000041-aaaa-4001-8000-000000000041";
        let child_id = "00000042-aaaa-4002-8000-000000000042";
        let grandchild_id = "00000043-aaaa-4003-8000-000000000043";
        store
            .save_record(&minimal_record(root_id, Some("2026-01-01T00:00:00Z")))
            .unwrap();
        store
            .save_record(&minimal_record(child_id, Some("2026-01-02T00:00:00Z")))
            .unwrap();
        store
            .save_record(&minimal_record(grandchild_id, Some("2026-01-03T00:00:00Z")))
            .unwrap();

        let c = create_container(&store, minimal_container("", "Tree")).unwrap();
        // Only the top-level root is a direct container member — child and
        // grandchild hang off it purely via `contains` relations, exactly
        // like the spec repo's section -> concept -> ... structure.
        add_member(&store, &c.container_id, root_id).unwrap();

        let rel_json = serde_json::json!({
            "relations": [
                {
                    "relationId": "eeeeeeee-1111-4000-8000-000000000001",
                    "relationType": "contains",
                    "sourceInstanceId": root_id,
                    "targetInstanceId": child_id,
                },
                {
                    "relationId": "eeeeeeee-1111-4000-8000-000000000002",
                    "relationType": "contains",
                    "sourceInstanceId": child_id,
                    "targetInstanceId": grandchild_id,
                },
            ]
        });
        crate::store::write_relations_standalone_for_test(&store, &rel_json);

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert!(bundle.diagnostics.is_empty(), "{:?}", bundle.diagnostics);
        assert_eq!(
            bundle.entries.len(),
            3,
            "expected one entry per instance reachable via contains, got {:?}",
            bundle
                .entries
                .iter()
                .map(|e| &e.instance_id)
                .collect::<Vec<_>>()
        );

        let by_id = |id: &str| {
            bundle
                .entries
                .iter()
                .find(|e| e.instance_id == id)
                .unwrap_or_else(|| panic!("missing entry for {id}"))
        };

        let root_entry = by_id(root_id);
        let child_entry = by_id(child_id);
        let grandchild_entry = by_id(grandchild_id);

        // Root and child both have children of their own, so each becomes a
        // directory (named after its own leaf filename) holding an index.md.
        let root_dir = root_entry.path.strip_suffix("/index.md").unwrap();
        assert!(
            child_entry.path.starts_with(&format!("{root_dir}/")),
            "child path {:?} should nest under root dir {:?}",
            child_entry.path,
            root_dir
        );
        let child_dir = child_entry.path.strip_suffix("/index.md").unwrap();
        assert!(
            grandchild_entry.path.starts_with(&format!("{child_dir}/")),
            "grandchild path {:?} should nest under child dir {:?}",
            grandchild_entry.path,
            child_dir
        );
        // The grandchild is a leaf: no index.md indirection.
        assert!(!grandchild_entry.path.ends_with("/index.md"));
    }

    // Regression for srs-rust#1104: the real spec corpus has `contains` nodes
    // reachable from more than one parent (an RFC-034 [R1] container's
    // declared membership overlapping a sibling's `contains` subtree — the
    // same shape as the historical `render_service` container-subset bug,
    // commit 3a20e59). Without a global visited set this both duplicates the
    // output and re-walks the shared subtree once per parent, compounding
    // with depth.
    #[test]
    fn multiparent_contains_child_is_emitted_exactly_once() {
        let store = make_store();
        let member_a = "00000051-aaaa-4001-8000-000000000051";
        let member_b = "00000052-aaaa-4002-8000-000000000052";
        let shared_child = "00000053-aaaa-4003-8000-000000000053";
        store
            .save_record(&minimal_record(member_a, Some("2026-01-01T00:00:00Z")))
            .unwrap();
        store
            .save_record(&minimal_record(member_b, Some("2026-01-02T00:00:00Z")))
            .unwrap();
        store
            .save_record(&minimal_record(shared_child, Some("2026-01-03T00:00:00Z")))
            .unwrap();

        let c = create_container(&store, minimal_container("", "Diamond")).unwrap();
        add_member(&store, &c.container_id, member_a).unwrap();
        add_member(&store, &c.container_id, member_b).unwrap();

        let rel_json = serde_json::json!({
            "relations": [
                {
                    "relationId": "eeeeeeee-2222-4000-8000-000000000001",
                    "relationType": "contains",
                    "sourceInstanceId": member_a,
                    "targetInstanceId": shared_child,
                },
                {
                    "relationId": "eeeeeeee-2222-4000-8000-000000000002",
                    "relationType": "contains",
                    "sourceInstanceId": member_b,
                    "targetInstanceId": shared_child,
                },
            ]
        });
        crate::store::write_relations_standalone_for_test(&store, &rel_json);

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        let occurrences = bundle
            .entries
            .iter()
            .filter(|e| e.instance_id == shared_child)
            .count();
        assert_eq!(
            occurrences,
            1,
            "shared child must be emitted exactly once, got {occurrences} occurrences in {:?}",
            bundle
                .entries
                .iter()
                .map(|e| (&e.instance_id, &e.path))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            bundle.entries.len(),
            3,
            "expected member_a + member_b + shared_child, no duplicates"
        );
    }

    // Regression for srs-rust#1104: real fixture/example instances in the
    // spec corpus can share an 8-hex-char id prefix (and even a display
    // label) with a sibling. Without a collision guard, the second entry's
    // file silently overwrites the first's on disk, which breaks the
    // "file count within 1 of reachable instances" contract.
    #[test]
    fn id8_collision_falls_back_to_full_instance_id() {
        let store = make_store();
        // Both records fall back to the same display label (their shared
        // type name, "item" — see `record_display_label`'s fallback ladder)
        // and share the same first 8 hex chars, so their default id8 paths
        // would otherwise collide on "item-aaaaaaaa.md".
        let first = "aaaaaaaa-1111-4001-8000-000000000001";
        let second = "aaaaaaaa-2222-4002-8000-000000000002";
        store
            .save_record(&minimal_record(first, Some("2026-01-01T00:00:00Z")))
            .unwrap();
        store
            .save_record(&minimal_record(second, Some("2026-01-02T00:00:00Z")))
            .unwrap();

        let c = create_container(&store, minimal_container("", "Collide")).unwrap();
        add_member(&store, &c.container_id, first).unwrap();
        add_member(&store, &c.container_id, second).unwrap();

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 2);
        let paths: std::collections::HashSet<&str> =
            bundle.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths.len(), 2, "both entries must land at distinct paths");
        // The earlier-sorted record keeps the short id8 path; the collision
        // forces the later one onto its full instance id.
        assert!(bundle.entries.iter().any(|e| e.path == "item-aaaaaaaa.md"));
        assert!(bundle
            .entries
            .iter()
            .any(|e| e.path == format!("{second}.md")));
    }

    #[test]
    fn refines_relation_resolves_to_target_path_within_bundle() {
        let store = make_store();
        let a_id = "0000a001-aaaa-4001-8000-0000000000a1";
        let b_id = "0000a002-aaaa-4002-8000-0000000000a2";
        let a = minimal_record(a_id, Some("2026-01-01T00:00:00Z"));
        let b = minimal_record(b_id, Some("2026-01-02T00:00:00Z"));
        store.save_record(&a).unwrap();
        store.save_record(&b).unwrap();

        let c = create_container(&store, minimal_container("", "Refines")).unwrap();
        add_member(&store, &c.container_id, a_id).unwrap();
        add_member(&store, &c.container_id, b_id).unwrap();

        write_relations(
            &store,
            &[make_relation(
                "eeeeeeee-1000-4000-8000-000000000001",
                "refines",
                a_id,
                b_id,
            )],
        );

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert!(bundle.diagnostics.is_empty());
        let entry_a = bundle
            .entries
            .iter()
            .find(|e| e.instance_id == a_id)
            .unwrap();
        let entry_b = bundle
            .entries
            .iter()
            .find(|e| e.instance_id == b_id)
            .unwrap();
        assert_eq!(entry_a.outgoing_relations.len(), 1);
        let link = &entry_a.outgoing_relations[0];
        assert_eq!(link.relation_type, "refines");
        assert_eq!(link.target_path, entry_b.path);
        assert_eq!(link.target_display_label, entry_b.display_label);
        // Target carries no outgoing edge of its own.
        assert!(entry_b.outgoing_relations.is_empty());
    }

    #[test]
    fn relation_target_outside_bundle_produces_diagnostic_not_a_dangling_link() {
        let store = make_store();
        let a_id = "0000b001-bbbb-4001-8000-0000000000b1";
        let outside_id = "0000b002-bbbb-4002-8000-0000000000b2";
        let a = minimal_record(a_id, Some("2026-01-01T00:00:00Z"));
        let outside = minimal_record(outside_id, Some("2026-01-02T00:00:00Z"));
        store.save_record(&a).unwrap();
        store.save_record(&outside).unwrap();

        // Only `a` is a member of the bundle's container — `outside` is not.
        let c = create_container(&store, minimal_container("", "Dangling")).unwrap();
        add_member(&store, &c.container_id, a_id).unwrap();

        write_relations(
            &store,
            &[make_relation(
                "eeeeeeee-2000-4000-8000-000000000002",
                "depends-on",
                a_id,
                outside_id,
            )],
        );

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        assert_eq!(bundle.entries.len(), 1);
        let entry_a = &bundle.entries[0];
        assert!(
            entry_a.outgoing_relations.is_empty(),
            "target outside the bundle must not produce a link"
        );
        assert_eq!(bundle.diagnostics.len(), 1);
        assert!(bundle.diagnostics[0].contains(outside_id));
        assert!(bundle.diagnostics[0].contains("depends-on"));
    }

    #[test]
    fn precedes_relation_is_represented_on_the_entry_not_only_in_index_order() {
        let store = make_store();
        let a_id = "0000c001-cccc-4001-8000-0000000000c1";
        let b_id = "0000c002-cccc-4002-8000-0000000000c2";
        let a = minimal_record(a_id, Some("2026-01-01T00:00:00Z"));
        let b = minimal_record(b_id, Some("2026-01-02T00:00:00Z"));
        store.save_record(&a).unwrap();
        store.save_record(&b).unwrap();

        let c = create_container(&store, minimal_container("", "Precedes")).unwrap();
        add_member(&store, &c.container_id, a_id).unwrap();
        add_member(&store, &c.container_id, b_id).unwrap();

        write_relations(
            &store,
            &[make_precedes_relation(
                "eeeeeeee-3000-4000-8000-000000000003",
                a_id,
                b_id,
            )],
        );

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: c.container_id.clone(),
            },
        )
        .unwrap();

        let entry_a = bundle
            .entries
            .iter()
            .find(|e| e.instance_id == a_id)
            .unwrap();
        assert_eq!(entry_a.outgoing_relations.len(), 1);
        assert_eq!(entry_a.outgoing_relations[0].relation_type, "precedes");
    }

    #[test]
    fn multiple_relation_types_group_by_type_and_sort_deterministically() {
        let store = make_store();
        let a_id = "0000d001-dddd-4001-8000-0000000000d1";
        let b_id = "0000d002-dddd-4002-8000-0000000000d2";
        let c_id = "0000d003-dddd-4003-8000-0000000000d3";
        let a = minimal_record(a_id, Some("2026-01-01T00:00:00Z"));
        let b = minimal_record(b_id, Some("2026-01-02T00:00:00Z"));
        let c_rec = minimal_record(c_id, Some("2026-01-03T00:00:00Z"));
        store.save_record(&a).unwrap();
        store.save_record(&b).unwrap();
        store.save_record(&c_rec).unwrap();

        let container = create_container(&store, minimal_container("", "Grouped")).unwrap();
        add_member(&store, &container.container_id, a_id).unwrap();
        add_member(&store, &container.container_id, b_id).unwrap();
        add_member(&store, &container.container_id, c_id).unwrap();

        write_relations(
            &store,
            &[
                make_relation(
                    "eeeeeeee-4000-4000-8000-000000000004",
                    "depends-on",
                    a_id,
                    c_id,
                ),
                make_relation(
                    "eeeeeeee-4000-4000-8000-000000000005",
                    "refines",
                    a_id,
                    b_id,
                ),
            ],
        );

        let bundle = export_okf_bundle(
            &store,
            OkfExportInput {
                container_id: container.container_id.clone(),
            },
        )
        .unwrap();

        let entry_a = bundle
            .entries
            .iter()
            .find(|e| e.instance_id == a_id)
            .unwrap();
        let grouped = group_relation_links_by_type(&entry_a.outgoing_relations);
        let types: Vec<&str> = grouped.iter().map(|(t, _)| t.as_str()).collect();
        // "depends-on" sorts before "refines" — deterministic, not insertion order.
        assert_eq!(types, vec!["depends-on", "refines"]);
    }
}
