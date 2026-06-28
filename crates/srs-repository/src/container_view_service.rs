//! Structured container-view projection for editor member lists (issue #254).
//!
//! `resolve_container_view` composes a Container, an optional DocumentView, and the
//! referenced View into a single read-only result the editor can render as an
//! interactive, selectable list: the container's root record, the ordered member
//! records (full [`Record`] + core-resolved display label + tier), and the
//! column/field spec resolved from the DocumentView.
//!
//! This is a Layer-1 typed projection — all semantics live here so the CLI, the WASM
//! binding, and any future consumer get the same answer (see
//! `docs/architecture/capability-layering.md`). Clients add presentation only.

use crate::container_service;
use crate::error::RepositoryError;
use crate::record_label;
use crate::record_store;
use crate::store::RepositoryStore;
use crate::view_service::{self, GetDocumentViewResult, GetViewResult};
use serde::{Deserialize, Serialize};
use srs_core::types::record::Record;
use srs_core::types::view::{DocumentSection, DocumentView, SectionSource};
use std::collections::HashMap;

/// Input to [`resolve_container_view`]. Constructed from CLI args / binding params;
/// never crosses a serde boundary, so it derives only `Debug, Clone`.
#[derive(Debug, Clone)]
pub struct ResolveContainerViewInput {
    pub container_id: String,
    /// Optional DocumentView UUID override. When `None`, the DocumentView is matched
    /// from the container's root type binding (`document_views_for_container`).
    pub view_id: Option<String>,
}

/// One column in the member list, resolved from a `FieldView`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnSpec {
    pub field_id: String,
    /// Field `name` from the package (falls back to `field_id` if unresolved).
    pub field_name: String,
    /// `FieldView.display_label` when set, else `field_name`.
    pub display_label: String,
    /// `i32` to match `FieldView.order`.
    pub order: i32,
    pub required: bool,
}

/// A resolved Tier-2 member (or root) of the container.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMember {
    pub instance_id: String,
    /// Always `2` — only Tier-2 Records are projected; non-Tier-2 members are skipped.
    pub tier: u8,
    /// Core-resolved label via `record_display_label`.
    pub display_label: String,
    pub record: Record,
}

/// The structured container view: root + ordered members + column spec.
///
/// `members` is the full roots-first deduped membership; when present, `root` is the
/// container's first root and also appears as the first entry of `members`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerView {
    pub container_id: String,
    /// UUID of the resolved DocumentView, or `None` when none resolves (columns empty).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_view_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<ResolvedMember>,
    pub members: Vec<ResolvedMember>,
    pub columns: Vec<ColumnSpec>,
    /// Authored default-hidden lifecycle states for this container's list, read from the
    /// **same** governing `DocumentSection` that drives `columns` (ADR-018 precedence). Empty
    /// unless that section is a `SectionSource::TypeQuery` declaring `excludeLifecycleStates`.
    /// Clients forward these to `find` (`--exclude-lifecycle-state`) for the default-hidden
    /// list, dropping them for a "show all" toggle — see ADR-020. Clients MUST NOT re-derive
    /// them from the DocumentView source.
    pub exclude_lifecycle_states: Vec<String>,
    /// Non-fatal notes (skipped non-Tier-2 members, unresolved view/field references).
    pub diagnostics: Vec<String>,
}

/// Resolve a container into root + ordered members + DocumentView-driven column spec.
///
/// Column source follows the precedence in
/// [ADR-018](../../docs/adr/018-container-view-column-source-precedence.md), via
/// [`select_governing_section`]: the section whose `source` targets this container
/// (`ContainerSubset { container_id }` or `TypeQuery { container_ids }`) and has a
/// `render_view_id` wins; otherwise the first section by `order` with a `render_view_id`;
/// otherwise the column spec is empty. The same governing section also supplies the authored
/// `exclude_lifecycle_states` ([ADR-020](../../docs/adr/020-resolve-view-authored-list-defaults.md)).
pub fn resolve_container_view(
    store: &dyn RepositoryStore,
    input: ResolveContainerViewInput,
) -> Result<ContainerView, RepositoryError> {
    let container_id = input.container_id.clone();
    let mut diagnostics: Vec<String> = Vec::new();

    // Validate the container exists and read its root binding directly. DocumentView
    // matching and member ordering below go through `document_views_for_container` and
    // `list_container_members`, which each re-load the container — an acceptable cost on
    // this Layer-1 read path in exchange for reusing the tested membership/matching logic
    // rather than duplicating it here.
    let container = container_service::get_container(store, &container_id)?;

    // Build instance_id -> tier lookup once, from the manifest index.
    let manifest = store.load_manifest()?;
    let tier_by_id: HashMap<String, u8> = manifest
        .instance_index
        .iter()
        .map(|e| (e.instance_id().to_string(), e.tier()))
        .collect();

    // Build the field_id -> field_name index once.
    let field_name_index = record_label::build_field_name_index(store)?;

    // Resolve the DocumentView.
    let document_view: Option<DocumentView> = match &input.view_id {
        Some(id) => match view_service::get_document_view_by_id(store, id)? {
            GetDocumentViewResult::Found(dv) => Some(*dv),
            GetDocumentViewResult::NotFound => {
                diagnostics.push(format!(
                    "resolve-container-view: documentView {id} not found"
                ));
                None
            }
        },
        // Reuse the tested matcher rather than re-deriving the root type binding.
        None => view_service::document_views_for_container(store, &container_id)?
            .into_iter()
            .next(),
    };
    let document_view_id = document_view.as_ref().map(|dv| dv.id.clone());

    // Resolve columns from the chosen DocumentView.
    let columns = match &document_view {
        Some(dv) => resolve_columns(
            store,
            dv,
            &container_id,
            &field_name_index,
            &mut diagnostics,
        )?,
        None => Vec::new(),
    };

    // Authored default-hidden lifecycle states from the same governing section (ADR-020).
    let exclude_lifecycle_states = document_view
        .as_ref()
        .and_then(|dv| select_governing_section(dv, &container_id))
        .map(section_exclude_lifecycle_states)
        .unwrap_or_default();

    // Resolve the root (first root_instance_id, if any).
    let root = match container
        .root_instance_ids
        .as_ref()
        .and_then(|ids| ids.first())
    {
        Some(root_id) => resolve_member(
            store,
            root_id,
            &tier_by_id,
            &field_name_index,
            "root instance",
            &mut diagnostics,
        )?,
        None => None,
    };

    // Resolve ordered members (roots-first, deduped).
    let member_ids = container_service::list_container_members(store, &container_id)?;
    let mut members = Vec::new();
    for id in &member_ids {
        if let Some(m) = resolve_member(
            store,
            id,
            &tier_by_id,
            &field_name_index,
            "instance",
            &mut diagnostics,
        )? {
            members.push(m);
        }
    }

    Ok(ContainerView {
        container_id,
        document_view_id,
        root,
        members,
        columns,
        exclude_lifecycle_states,
        diagnostics,
    })
}

/// Load one instance as a Tier-2 [`ResolvedMember`]; non-Tier-2 or unresolved
/// instances yield `None` plus a diagnostic (mirrors `tree_service`).
fn resolve_member(
    store: &dyn RepositoryStore,
    id: &str,
    tier_by_id: &HashMap<String, u8>,
    field_name_index: &HashMap<String, String>,
    kind: &str,
    diagnostics: &mut Vec<String>,
) -> Result<Option<ResolvedMember>, RepositoryError> {
    match tier_by_id.get(id) {
        Some(2) => match record_store::get_record_by_id(store, id)? {
            Some(record) => {
                let display_label = record_label::record_display_label(&record, field_name_index);
                Ok(Some(ResolvedMember {
                    instance_id: id.to_string(),
                    tier: 2,
                    display_label,
                    record,
                }))
            }
            None => {
                diagnostics.push(format!(
                    "resolve-container-view: {kind} {id} does not resolve"
                ));
                Ok(None)
            }
        },
        Some(_) => {
            diagnostics.push(format!(
                "resolve-container-view: {kind} {id} not a Tier 2 record — skipped"
            ));
            Ok(None)
        }
        None => {
            diagnostics.push(format!(
                "resolve-container-view: {kind} {id} not in manifest index — skipped"
            ));
            Ok(None)
        }
    }
}

/// Resolve the column spec from a DocumentView, per the ADR-018 precedence.
fn resolve_columns(
    store: &dyn RepositoryStore,
    dv: &DocumentView,
    container_id: &str,
    field_name_index: &HashMap<String, String>,
    diagnostics: &mut Vec<String>,
) -> Result<Vec<ColumnSpec>, RepositoryError> {
    let view_id = match select_column_view_id(dv, container_id) {
        Some(id) => id,
        None => return Ok(Vec::new()),
    };
    let view = match view_service::get_view_by_id(store, &view_id)? {
        GetViewResult::Found(v) => *v,
        GetViewResult::NotFound => {
            diagnostics.push(format!(
                "resolve-container-view: view {view_id} referenced by documentView {} not found",
                dv.id
            ));
            return Ok(Vec::new());
        }
    };

    let mut field_views: Vec<_> = view
        .field_views
        .iter()
        .filter(|fv| fv.visible != Some(false))
        .collect();
    field_views.sort_by_key(|fv| fv.order);

    let mut columns = Vec::new();
    for fv in field_views {
        let field_name = match field_name_index.get(&fv.field_id) {
            Some(n) => n.clone(),
            None => {
                diagnostics.push(format!(
                    "resolve-container-view: field {} not in package index",
                    fv.field_id
                ));
                fv.field_id.clone()
            }
        };
        let display_label = fv
            .display_label
            .clone()
            .unwrap_or_else(|| field_name.clone());
        columns.push(ColumnSpec {
            field_id: fv.field_id.clone(),
            field_name,
            display_label,
            order: fv.order,
            required: fv.required.unwrap_or(false),
        });
    }
    Ok(columns)
}

/// Pick the View UUID that drives the columns (ADR-018 precedence).
/// True when `source` explicitly targets `container_id` — either a `ContainerSubset` of this
/// container or a `TypeQuery` whose `container_ids` includes it. The canonical decision-log
/// section is now a `TypeQuery`, so the ADR-018 "targets this container" test (step 1) covers
/// both source shapes.
fn source_targets_container(source: &SectionSource, container_id: &str) -> bool {
    match source {
        SectionSource::ContainerSubset {
            container_id: cid, ..
        } => cid == container_id,
        SectionSource::TypeQuery {
            container_ids: Some(ids),
            ..
        } => ids.iter().any(|c| c == container_id),
        _ => false,
    }
}

/// Select the single `DocumentSection` that governs this container's list, per ADR-018
/// precedence: (1) a section that targets this container (any source shape) and has a
/// `render_view_id`; (2) otherwise the first section by `order` with a `render_view_id`;
/// (3) otherwise `None`. Both the column View (`render_view_id`) and the authored
/// `excludeLifecycleStates` (ADR-020) derive from this one selection. Tie-break: if both a
/// `ContainerSubset` and a `TypeQuery` target the container, the lower-`order` one wins (the
/// sort below is stable; sections are visited in `order` ascending).
fn select_governing_section<'a>(
    dv: &'a DocumentView,
    container_id: &str,
) -> Option<&'a DocumentSection> {
    let mut sections: Vec<&DocumentSection> = dv.sections.iter().collect();
    sections.sort_by_key(|s| s.order);

    // 1. Section explicitly targeting this container, with a render_view_id.
    if let Some(s) = sections
        .iter()
        .find(|s| s.render_view_id.is_some() && source_targets_container(&s.source, container_id))
    {
        return Some(s);
    }
    // 2. First section by order with a render_view_id.
    sections.into_iter().find(|s| s.render_view_id.is_some())
}

/// Authored default-hidden lifecycle states declared on the governing section's source
/// (ADR-020). Empty for any non-`TypeQuery` source or an absent list.
fn section_exclude_lifecycle_states(section: &DocumentSection) -> Vec<String> {
    match &section.source {
        SectionSource::TypeQuery {
            exclude_lifecycle_states: Some(states),
            ..
        } => states.clone(),
        _ => Vec::new(),
    }
}

fn select_column_view_id(dv: &DocumentView, container_id: &str) -> Option<String> {
    select_governing_section(dv, container_id).and_then(|s| s.render_view_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service;
    use crate::index::InstanceIndexEntry;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use srs_core::types::container::Container;
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record::{FieldValue, Record};
    use srs_core::types::view::{DocumentSection, DocumentView, FieldView, SectionSource, View};
    use std::path::PathBuf;

    const TYPE_ID: &str = "00000000-0000-4000-8000-00000000aaaa";
    const VIEW_ID: &str = "view-decision-1";
    const DV_ID: &str = "dv-decision-1";
    const ALT_DV_ID: &str = "dv-alt-1";
    const CONTAINER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn field(id: &str, name: &str) -> Field {
        Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            ai_guidance: serde_json::Value::Null,
            value_type: ValueType::String,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn field_view(
        field_id: &str,
        order: i32,
        visible: Option<bool>,
        label: Option<&str>,
    ) -> FieldView {
        FieldView {
            field_id: field_id.to_string(),
            order,
            required: None,
            visible,
            display_label: label.map(|s| s.to_string()),
        }
    }

    fn view_with_fields(field_views: Vec<FieldView>) -> View {
        View {
            id: VIEW_ID.to_string(),
            namespace: "com.test".to_string(),
            name: "decision-view".to_string(),
            version: 1,
            description: "decision view".to_string(),
            field_views,
            compatible_types: None,
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn section(
        section_id: &str,
        order: i32,
        source: SectionSource,
        render_view_id: Option<&str>,
    ) -> DocumentSection {
        DocumentSection {
            section_id: section_id.to_string(),
            title: None,
            description: None,
            order,
            source,
            render_view_id: render_view_id.map(|s| s.to_string()),
            type_dispatch: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
        }
    }

    fn document_view(id: &str, sections: Vec<DocumentSection>) -> DocumentView {
        DocumentView {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: format!("dv-{id}"),
            version: 1,
            description: "test dv".to_string(),
            container_type: None,
            root_type_refs: Some(vec![srs_core::types::view::ExactTypeRef {
                type_id: TYPE_ID.to_string(),
                type_version: 1,
            }]),
            sections,
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn record(instance_id: &str, title_field_id: &str, title: &str) -> Record {
        Record {
            instance_id: instance_id.to_string(),
            type_id: TYPE_ID.to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "decision".to_string(),
            field_values: vec![FieldValue {
                field_id: title_field_id.to_string(),
                value: serde_json::json!(title),
                entries: None,
                source: None,
                edited_at: None,
            }],
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    /// Build a store with the given fields, views, document views, and instances
    /// (id, tier, json). Instances are placed at `records/<id>.json`.
    fn build_store(
        fields: Vec<Field>,
        views: Vec<View>,
        document_views: Vec<DocumentView>,
        instances: Vec<(&str, u8, serde_json::Value)>,
    ) -> MemoryStore {
        let manifest = Manifest {
            instance_index: instances
                .iter()
                .map(|(id, tier, _)| InstanceIndexEntry {
                    instance_id: id.to_string(),
                    tier: *tier,
                    path: format!("records/{id}.json"),
                    title: None,
                    tags: None,
                })
                .collect(),
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types: vec![],
            relation_type_definitions: vec![],
            views,
            document_views,
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let mut store = MemoryStore::new(manifest, package);
        for (id, _, json) in instances {
            store = store.with_data(&format!("records/{id}.json"), json);
        }
        store
    }

    fn make_container(roots: Vec<&str>, members: Vec<&str>) -> Container {
        Container {
            container_id: CONTAINER_ID.to_string(),
            title: "Decisions".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            root_instance_ids: if roots.is_empty() {
                None
            } else {
                Some(roots.into_iter().map(|s| s.to_string()).collect())
            },
            member_instance_ids: if members.is_empty() {
                None
            } else {
                Some(members.into_iter().map(|s| s.to_string()).collect())
            },
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        }
    }

    /// Standard fixture: two fields (title, status), a view exposing both (status hidden
    /// in some tests), a document view targeting the container, and two Tier-2 records.
    fn standard_store(field_views: Vec<FieldView>, sections: Vec<DocumentSection>) -> MemoryStore {
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(field_views);
        let dv = document_view(DV_ID, sections);
        let root = record("root-1", "f-title", "Root Decision");
        let member = record("mem-1", "f-title", "Member Decision");
        build_store(
            fields,
            vec![view],
            vec![dv],
            vec![
                ("root-1", 2, serde_json::to_value(&root).unwrap()),
                ("mem-1", 2, serde_json::to_value(&member).unwrap()),
            ],
        )
    }

    fn input(view_id: Option<&str>) -> ResolveContainerViewInput {
        ResolveContainerViewInput {
            container_id: CONTAINER_ID.to_string(),
            view_id: view_id.map(|s| s.to_string()),
        }
    }

    #[test]
    fn resolve_container_view_returns_columns_from_matching_section() {
        let fvs = vec![
            field_view("f-status", 1, None, Some("Status")),
            field_view("f-title", 0, None, None),
            field_view("f-hidden", 2, Some(false), None),
        ];
        let sections = vec![section(
            "s1",
            0,
            SectionSource::ContainerSubset {
                container_id: CONTAINER_ID.to_string(),
                container_type: None,
                type_filter: None,
            },
            Some(VIEW_ID),
        )];
        let store = standard_store(fvs, sections);
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["mem-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        // visible:false excluded, ordered by `order` ascending.
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].field_id, "f-title");
        assert_eq!(result.columns[0].field_name, "title");
        // display_label falls back to field name when no override.
        assert_eq!(result.columns[0].display_label, "title");
        assert_eq!(result.columns[1].field_id, "f-status");
        // display_label override applied.
        assert_eq!(result.columns[1].display_label, "Status");
        assert_eq!(result.document_view_id.as_deref(), Some(DV_ID));
    }

    #[test]
    fn resolve_container_view_falls_back_to_first_section_with_view() {
        // No ContainerSubset matching this container; first section (by order) with a
        // render_view_id should drive columns.
        let fvs = vec![field_view("f-title", 0, None, None)];
        let sections = vec![
            section(
                "s-late",
                5,
                SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                Some(VIEW_ID),
            ),
            section(
                "s-early-noview",
                0,
                SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                None,
            ),
        ];
        let store = standard_store(fvs, sections);
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["mem-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0].field_id, "f-title");
    }

    #[test]
    fn resolve_container_view_view_id_override() {
        // Two document views; the override selects the alternate, whose section has no
        // render_view_id, so columns are empty but document_view_id is the override.
        let fields = vec![field("f-title", "title")];
        let view = view_with_fields(vec![field_view("f-title", 0, None, None)]);
        let primary = document_view(
            DV_ID,
            vec![section(
                "s1",
                0,
                SectionSource::ContainerSubset {
                    container_id: CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter: None,
                },
                Some(VIEW_ID),
            )],
        );
        let alt = document_view(
            ALT_DV_ID,
            vec![section(
                "s1",
                0,
                SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                None,
            )],
        );
        let root = record("root-1", "f-title", "Root");
        let store = build_store(
            fields,
            vec![view],
            vec![primary, alt],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(Some(ALT_DV_ID))).unwrap();
        assert_eq!(result.document_view_id.as_deref(), Some(ALT_DV_ID));
        assert!(result.columns.is_empty());
    }

    #[test]
    fn resolve_container_view_unknown_view_id_empty_columns_with_diagnostic() {
        let store = standard_store(
            vec![field_view("f-title", 0, None, None)],
            vec![section(
                "s1",
                0,
                SectionSource::ContainerSubset {
                    container_id: CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter: None,
                },
                Some(VIEW_ID),
            )],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(Some("no-such-dv"))).unwrap();
        assert!(result.document_view_id.is_none());
        assert!(result.columns.is_empty());
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.contains("documentView no-such-dv not found")));
        // Members/root still returned.
        assert!(result.root.is_some());
    }

    #[test]
    fn resolve_container_view_no_document_view_returns_members_only() {
        // Build a store with NO document views; columns empty, members present.
        let fields = vec![field("f-title", "title")];
        let view = view_with_fields(vec![field_view("f-title", 0, None, None)]);
        let root = record("root-1", "f-title", "Root");
        let member = record("mem-1", "f-title", "Member");
        let store = build_store(
            fields,
            vec![view],
            vec![],
            vec![
                ("root-1", 2, serde_json::to_value(&root).unwrap()),
                ("mem-1", 2, serde_json::to_value(&member).unwrap()),
            ],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["mem-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        assert!(result.document_view_id.is_none());
        assert!(result.columns.is_empty());
        assert_eq!(result.members.len(), 2);
        assert!(result.root.is_some());
    }

    #[test]
    fn resolve_container_view_skips_non_tier2_member_with_diagnostic() {
        let fields = vec![field("f-title", "title")];
        let view = view_with_fields(vec![field_view("f-title", 0, None, None)]);
        let dv = document_view(
            DV_ID,
            vec![section(
                "s1",
                0,
                SectionSource::ContainerSubset {
                    container_id: CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter: None,
                },
                Some(VIEW_ID),
            )],
        );
        let root = record("root-1", "f-title", "Root");
        // A Tier-0 note instance (not a Record). It must be SKIPPED, not loaded.
        let note_json = serde_json::json!({ "instanceId": "note-1", "tier": 0, "sections": [] });
        let store = build_store(
            fields,
            vec![view],
            vec![dv],
            vec![
                ("root-1", 2, serde_json::to_value(&root).unwrap()),
                ("note-1", 0, note_json),
            ],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["note-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        // Only the Tier-2 root is a member; the note is skipped.
        assert_eq!(result.members.len(), 1);
        assert_eq!(result.members[0].instance_id, "root-1");
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.contains("note-1 not a Tier 2 record")));
    }

    #[test]
    fn resolve_container_view_root_and_member_labels() {
        let store = standard_store(
            vec![field_view("f-title", 0, None, None)],
            vec![section(
                "s1",
                0,
                SectionSource::ContainerSubset {
                    container_id: CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter: None,
                },
                Some(VIEW_ID),
            )],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["mem-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        assert_eq!(result.root.as_ref().unwrap().display_label, "Root Decision");
        // members are roots-first: root-1 then mem-1.
        assert_eq!(result.members[0].display_label, "Root Decision");
        assert_eq!(result.members[1].display_label, "Member Decision");
        assert_eq!(result.members[0].tier, 2);
    }

    #[test]
    fn resolve_container_view_container_not_found_errors() {
        let store = standard_store(
            vec![field_view("f-title", 0, None, None)],
            vec![section(
                "s1",
                0,
                SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                Some(VIEW_ID),
            )],
        );
        // No container created.
        let err = resolve_container_view(&store, input(None)).unwrap_err();
        assert!(
            matches!(err, RepositoryError::ContainerNotFound { .. }),
            "expected ContainerNotFound, got {err:?}"
        );
    }

    #[test]
    fn resolve_container_view_roundtrip_stores() {
        // Cross-store roundtrip (memory -> file) per CLAUDE.md storage rules.
        // The snapshot importer requires identifiers >= 8 chars, so this test uses
        // its own snapshot-compliant fixture rather than the short-id `standard_store`.
        const F_TITLE: &str = "field-title-0001";
        const F_STATUS: &str = "field-status-0001";
        const VIEW: &str = "view-decision-0001";
        const DV: &str = "dv-decision-0001";
        const ROOT: &str = "record-root-0001";
        const MEM: &str = "record-member-0001";

        let fields = vec![field(F_TITLE, "title"), field(F_STATUS, "status")];
        let view = View {
            id: VIEW.to_string(),
            ..view_with_fields(vec![
                field_view(F_TITLE, 0, None, None),
                field_view(F_STATUS, 1, None, Some("Status")),
            ])
        };
        let dv = DocumentView {
            id: DV.to_string(),
            sections: vec![section(
                "section-0001",
                0,
                SectionSource::ContainerSubset {
                    container_id: CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter: None,
                },
                Some(VIEW),
            )],
            ..document_view(DV, vec![])
        };
        let root = record(ROOT, F_TITLE, "Root Decision");
        let member = record(MEM, F_TITLE, "Member Decision");
        let store = build_store(
            fields,
            vec![view],
            vec![dv],
            vec![
                (ROOT, 2, serde_json::to_value(&root).unwrap()),
                (MEM, 2, serde_json::to_value(&member).unwrap()),
            ],
        );
        container_service::create_container(&store, make_container(vec![ROOT], vec![MEM])).unwrap();

        let from_memory = resolve_container_view(&store, input(None)).unwrap();

        // Copy the whole repository memory -> file (FileStore) and re-run the service.
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = resolve_container_view(&file_store, input(None)).unwrap();

        assert_eq!(from_memory.columns.len(), 2, "fixture sanity: two columns");
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "ContainerView must be identical across stores (memory -> file)"
        );
    }

    /// A `type-query` source targeting this container (the canonical decision-log shape) is
    /// recognised by `select_governing_section`, drives columns, and surfaces its authored
    /// `excludeLifecycleStates` on the payload (ADR-020).
    fn type_query_source(exclude: Option<Vec<&str>>) -> SectionSource {
        SectionSource::TypeQuery {
            semantic_object_type: "com.test/decision".to_string(),
            lifecycle_state: None,
            container_ids: Some(vec![CONTAINER_ID.to_string()]),
            lifecycle_states: None,
            exclude_lifecycle_states: exclude
                .map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            container_scope: None,
        }
    }

    #[test]
    fn resolve_view_surfaces_type_query_exclude_lifecycle_states() {
        let fvs = vec![field_view("f-title", 0, None, None)];
        let sections = vec![section(
            "s1",
            0,
            type_query_source(Some(vec!["superseded", "closed"])),
            Some(VIEW_ID),
        )];
        let store = standard_store(fvs, sections);
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["mem-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        assert_eq!(
            result.exclude_lifecycle_states,
            vec!["superseded".to_string(), "closed".to_string()]
        );
        // Columns still resolve from the same governing (type-query) section.
        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0].field_id, "f-title");
        assert_eq!(result.document_view_id.as_deref(), Some(DV_ID));
    }

    #[test]
    fn resolve_view_exclude_lifecycle_states_empty_for_container_subset() {
        let fvs = vec![field_view("f-title", 0, None, None)];
        let sections = vec![section(
            "s1",
            0,
            SectionSource::ContainerSubset {
                container_id: CONTAINER_ID.to_string(),
                container_type: None,
                type_filter: None,
            },
            Some(VIEW_ID),
        )];
        let store = standard_store(fvs, sections);
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["mem-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        assert!(result.exclude_lifecycle_states.is_empty());
    }

    #[test]
    fn resolve_view_columns_unchanged_after_exclude_states_addition() {
        // The governing-section refactor must not change column selection: a type-query
        // section and a container-subset section over the same View resolve identical columns
        // and document_view_id (only exclude_lifecycle_states differs).
        let fvs = || {
            vec![
                field_view("f-title", 0, None, None),
                field_view("f-status", 1, None, Some("Status")),
            ]
        };
        let cs_store = standard_store(
            fvs(),
            vec![section(
                "s1",
                0,
                SectionSource::ContainerSubset {
                    container_id: CONTAINER_ID.to_string(),
                    container_type: None,
                    type_filter: None,
                },
                Some(VIEW_ID),
            )],
        );
        container_service::create_container(
            &cs_store,
            make_container(vec!["root-1"], vec!["mem-1"]),
        )
        .unwrap();
        let tq_store = standard_store(
            fvs(),
            vec![section(
                "s1",
                0,
                type_query_source(Some(vec!["closed"])),
                Some(VIEW_ID),
            )],
        );
        container_service::create_container(
            &tq_store,
            make_container(vec!["root-1"], vec!["mem-1"]),
        )
        .unwrap();

        let cs = resolve_container_view(&cs_store, input(None)).unwrap();
        let tq = resolve_container_view(&tq_store, input(None)).unwrap();

        assert_eq!(
            serde_json::to_value(&cs.columns).unwrap(),
            serde_json::to_value(&tq.columns).unwrap(),
            "column resolution must be source-shape-agnostic"
        );
        assert_eq!(cs.document_view_id, tq.document_view_id);
        assert!(cs.exclude_lifecycle_states.is_empty());
        assert_eq!(tq.exclude_lifecycle_states, vec!["closed".to_string()]);
    }

    #[test]
    fn resolve_view_roundtrip_type_query_exclude_states() {
        // Cross-store roundtrip (memory -> file) over the path that actually populates
        // exclude_lifecycle_states (a type-query governing section). Snapshot importer needs
        // ids >= 8 chars, so this uses its own long-id fixture.
        const F_TITLE: &str = "field-title-0001";
        const VIEW: &str = "view-decision-0001";
        const DV: &str = "dv-decision-0001";
        const ROOT: &str = "record-root-0001";
        const MEM: &str = "record-member-0001";

        let fields = vec![field(F_TITLE, "title")];
        let view = View {
            id: VIEW.to_string(),
            ..view_with_fields(vec![field_view(F_TITLE, 0, None, None)])
        };
        let dv = DocumentView {
            id: DV.to_string(),
            sections: vec![section(
                "section-0001",
                0,
                type_query_source(Some(vec!["superseded", "closed"])),
                Some(VIEW),
            )],
            ..document_view(DV, vec![])
        };
        let root = record(ROOT, F_TITLE, "Root Decision");
        let member = record(MEM, F_TITLE, "Member Decision");
        let store = build_store(
            fields,
            vec![view],
            vec![dv],
            vec![
                (ROOT, 2, serde_json::to_value(&root).unwrap()),
                (MEM, 2, serde_json::to_value(&member).unwrap()),
            ],
        );
        container_service::create_container(&store, make_container(vec![ROOT], vec![MEM])).unwrap();

        let from_memory = resolve_container_view(&store, input(None)).unwrap();
        assert_eq!(
            from_memory.exclude_lifecycle_states,
            vec!["superseded".to_string(), "closed".to_string()]
        );

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = resolve_container_view(&file_store, input(None)).unwrap();

        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "ContainerView (incl. exclude_lifecycle_states) must survive memory -> file"
        );
    }
}
