//! Structured container-view projection for editor member lists (issue #254, #256).
//!
//! `resolve_container_view` composes a Container, an optional DocumentView, and the
//! referenced View into a single read-only result the editor can render as an
//! interactive, selectable list: the container's root record, the ordered member
//! records (Tier-0, Tier-1, or Tier-2; full [`Record`] present only for Tier-2), and
//! the column/field spec resolved from the DocumentView.
//!
//! Tier-0 (Note) and Tier-1 (TypedRecord) members carry `record: None` and a
//! `display_label` sourced from the manifest index entry's `title`. Clients that drive
//! field-column cells check `record.is_some()`; cells for non-Tier-2 rows are empty.
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
    /// True when this column's `fieldId` is the effective `identityFieldId` shared by **all**
    /// Types in the DocumentView's `root_type_refs` — see ADR-023 (single-entry case) and
    /// ADR-027 (common-identity multi-entry extension). `false` whenever that resolution is
    /// absent, ambiguous, or any referenced Type disagrees. Never affects column order.
    pub is_identity_column: bool,
}

/// A resolved member (or root) of the container.
///
/// Tier-2 Records carry a full [`Record`] in `record`; Tier-0 (Note) and Tier-1
/// (TypedRecord) members carry `record: None` — their `display_label` is sourced from
/// the manifest index entry's `title`. Clients that render field-column cells check
/// `record.is_some()`; cells for non-Tier-2 rows are intentionally empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedMember {
    pub instance_id: String,
    /// Instance tier: 0 (Note), 1 (TypedRecord), or 2 (Record).
    pub tier: u8,
    /// Core-resolved label: `record_display_label` for Tier-2; manifest index `title`
    /// (falling back to `instance_id`) for Tier-0/1.
    pub display_label: String,
    /// `false` when this member's `lifecycleState` is in the container's authored
    /// `excludeLifecycleStates` list (ADR-020). `true` when the lifecycle state is absent
    /// or not in the exclusion list. Always `true` for Tier-0/1 (no lifecycle state).
    /// Clients use this to implement a "show all" toggle without re-querying.
    pub is_visible_by_default: bool,
    /// Present for Tier-2 Records; `None` for Tier-0/1. Serialized with
    /// `skip_serializing_if = "Option::is_none"` so the JSON field is absent (not `null`)
    /// for non-Tier-2 members.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record: Option<Record>,
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
    /// Non-fatal notes (skipped unknown-tier members, unresolved view/field references).
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

    // Build instance_id -> tier and instance_id -> display label lookups from one catalog
    // snapshot. The label index provides display labels for Tier-0/1 members (the entity
    // body's `title` field, catalog-derived — RFC-038 Change K retires the index column).
    // Tier-2 members use record_display_label instead; the label_by_id entry still exists
    // but is unused for them.
    let cat = store.catalog()?;
    let mut tier_by_id: HashMap<String, u8> = HashMap::new();
    let mut label_by_id: HashMap<String, String> = HashMap::new();
    for entry in &cat.instances {
        let id = entry.id.clone();
        let r = crate::store::catalog_instance_ref(store, entry)?;
        let label = r.title.unwrap_or_else(|| id.clone());
        tier_by_id.insert(id.clone(), entry.tier.unwrap_or(2));
        label_by_id.insert(id, label);
    }

    // Build the field_id -> field_name and (type_id, type_version) -> identityFieldId
    // indexes together, from a single Package load.
    let (field_name_index, identity_field_index) = record_label::build_label_indexes(store)?;

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
            &identity_field_index,
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
            &label_by_id,
            &identity_field_index,
            &field_name_index,
            &exclude_lifecycle_states,
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
            &label_by_id,
            &identity_field_index,
            &field_name_index,
            &exclude_lifecycle_states,
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

/// Load one instance as a [`ResolvedMember`].
///
/// - **Tier 2 (Record):** loads the full record, resolves `display_label` via
///   `record_display_label`, and sets `is_visible_by_default` from `lifecycleState`.
/// - **Tier 0 (Note) or Tier 1 (TypedRecord):** sets `record: None`, uses the manifest
///   index `title` (falling back to `instance_id`) as `display_label`, and sets
///   `is_visible_by_default: true` (no lifecycle state on these tiers).
/// - **Unknown tier:** emits a diagnostic and returns `None` (mirrors `tree_service`).
/// - **Not in manifest:** emits a diagnostic and returns `None`.
#[allow(clippy::too_many_arguments)]
fn resolve_member(
    store: &dyn RepositoryStore,
    id: &str,
    tier_by_id: &HashMap<String, u8>,
    label_by_id: &HashMap<String, String>,
    identity_field_index: &HashMap<(String, u32), String>,
    field_name_index: &HashMap<String, String>,
    exclude_lifecycle_states: &[String],
    kind: &str,
    diagnostics: &mut Vec<String>,
) -> Result<Option<ResolvedMember>, RepositoryError> {
    match tier_by_id.get(id) {
        Some(t @ 0) | Some(t @ 1) => {
            let display_label = label_by_id
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.to_string());
            Ok(Some(ResolvedMember {
                instance_id: id.to_string(),
                tier: *t,
                display_label,
                is_visible_by_default: true,
                record: None,
            }))
        }
        Some(2) => match record_store::get_record_by_id(store, id)? {
            Some(record) => {
                let display_label = record_label::record_display_label(
                    &record,
                    identity_field_index,
                    field_name_index,
                );
                let is_visible_by_default = record
                    .lifecycle_state
                    .as_deref()
                    .is_none_or(|s| !exclude_lifecycle_states.iter().any(|e| e == s));
                Ok(Some(ResolvedMember {
                    instance_id: id.to_string(),
                    tier: 2,
                    display_label,
                    is_visible_by_default,
                    record: Some(record),
                }))
            }
            None => {
                diagnostics.push(format!(
                    "resolve-container-view: {kind} {id} does not resolve"
                ));
                Ok(None)
            }
        },
        Some(t) => {
            diagnostics.push(format!(
                "resolve-container-view: {kind} {id} has unknown tier {t} — skipped"
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

/// Returns the common `identityFieldId` for a DocumentView's `root_type_refs` (ADR-027):
/// - `None` / empty: `None`
/// - 1 entry: same as ADR-023 — look up that Type in the index
/// - N > 1 entries: return the field only if ALL entries resolve to the *same* field ID;
///   `None` if any entry is absent from the index or disagrees.
fn common_identity_field<'a>(
    dv: &DocumentView,
    identity_field_index: &'a record_label::IdentityFieldIndex,
) -> Option<&'a String> {
    let refs = dv.root_type_refs.as_deref()?;
    if refs.is_empty() {
        return None;
    }
    let first = identity_field_index.get(&(refs[0].type_id.clone(), refs[0].type_version))?;
    let all_agree = refs[1..]
        .iter()
        .all(|r| identity_field_index.get(&(r.type_id.clone(), r.type_version)) == Some(first));
    if all_agree {
        Some(first)
    } else {
        None
    }
}

/// Resolve the column spec from a DocumentView, per the ADR-018 precedence.
///
/// `is_identity_column` (ADR-023, ADR-027, RFC-020) is derived via `common_identity_field` —
/// the same `(type_id, type_version) -> identityFieldId` index already built by
/// `resolve_container_view` for `record_display_label`. When all `root_type_refs` entries agree
/// on the same field ID (the single-entry case from ADR-023, or the common-identity multi-entry
/// case from ADR-027), that column is marked `true`; every other case — absent, empty, or any
/// disagreeing/absent Type — yields `is_identity_column: false`, which is a normal outcome, not
/// an error. This function never independently calls `Package::effective_identity_field_id`.
fn resolve_columns(
    store: &dyn RepositoryStore,
    dv: &DocumentView,
    container_id: &str,
    identity_field_index: &HashMap<(String, u32), String>,
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

    let identity_field_id = common_identity_field(dv, identity_field_index);

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
            is_identity_column: identity_field_id == Some(&fv.field_id),
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
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use srs_core::types::container::Container;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record::{FieldValues, Record};
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use srs_core::types::view::{
        DocumentSection, DocumentView, ExactTypeRef, FieldView, SectionSource, View,
    };
    use std::path::PathBuf;

    const TYPE_ID: &str = "00000000-0000-4000-8000-00000000aaaa";
    const TYPE_ID_2: &str = "00000000-0000-4000-8000-00000000bbbb";
    const VIEW_ID: &str = "view-decision-1";
    const DV_ID: &str = "dv-decision-1";
    const ALT_DV_ID: &str = "dv-alt-1";
    const CONTAINER_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn field(id: &str, name: &str) -> Field {
        Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            field_type: FieldType::string(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn field_view(
        field_id: &str,
        order: i32,
        visible: Option<bool>,
        label: Option<&str>,
    ) -> FieldView {
        FieldView {
            composite_renderer: None,
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
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn section(
        section_id: &str,
        order: i32,
        source: SectionSource,
        render_view_id: Option<&str>,
    ) -> DocumentSection {
        DocumentSection {
            composite_renderers: None,
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
            relations_presentation: None,
        }
    }

    fn document_view(id: &str, sections: Vec<DocumentSection>) -> DocumentView {
        DocumentView {
            composite_renderers: None,
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
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn record(instance_id: &str, title_field_name: &str, title: &str) -> Record {
        Record {
            field_meta: None,
            instance_id: instance_id.to_string(),
            type_id: TYPE_ID.to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "decision".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert(title_field_name, serde_json::json!(title));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
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
        build_store_with_types(fields, vec![], views, document_views, instances)
    }

    /// Like [`build_store`], but also installs the given `record_types` in the package —
    /// needed for `identityFieldId`/`isIdentityColumn` tests, which must resolve a Type by
    /// `(type_id, type_version)`.
    fn build_store_with_types(
        fields: Vec<Field>,
        record_types: Vec<RecordType>,
        views: Vec<View>,
        document_views: Vec<DocumentView>,
        instances: Vec<(&str, u8, serde_json::Value)>,
    ) -> MemoryStore {
        let manifest = Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types,
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
            identity_instance_id: None,
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
            extra: std::collections::BTreeMap::new(),
        }
    }

    /// Standard fixture: two fields (title, status), a view exposing both (status hidden
    /// in some tests), a document view targeting the container, and two Tier-2 records.
    fn standard_store(field_views: Vec<FieldView>, sections: Vec<DocumentSection>) -> MemoryStore {
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(field_views);
        let dv = document_view(DV_ID, sections);
        let root = record("root-1", "title", "Root Decision");
        let member = record("mem-1", "title", "Member Decision");
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
        let root = record("root-1", "title", "Root");
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
        let root = record("root-1", "title", "Root");
        let member = record("mem-1", "title", "Member");
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

    // resolve_container_view_unknown_tier_member_skipped retired by RFC-038
    // Phase 3 (srs-rust#783): its scenario — a stored member whose tier is
    // outside 0/1/2 — cannot exist under the catalog model. Tier comes from
    // body shape classification (note/typed-record/record), and an object
    // matching none of those shapes is a fatal SRS038-R8-SHAPE-NO-MATCH at
    // catalog build, so resolve_container_view can never see an
    // "unknown-tier" member to skip.

    /// Like [`build_store`], but each instance tuple includes an optional manifest index title.
    /// Use this when testing `display_label` for Tier-0/1 members where the title must be
    /// non-null; `build_store` always sets `title: None`.
    fn build_store_titled(
        fields: Vec<Field>,
        views: Vec<View>,
        document_views: Vec<DocumentView>,
        instances: Vec<(&str, u8, Option<&str>, serde_json::Value)>,
    ) -> MemoryStore {
        let manifest = Manifest {
            container: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
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
        for (id, _, _, json) in &instances {
            store = store.with_data(&format!("records/{id}.json"), json.clone());
        }
        store
    }

    #[test]
    fn resolve_container_view_includes_tier0_note_member() {
        // A Tier-0 member with a manifest index title must appear in members with
        // record: None and display_label sourced from the index title.
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
        let root = record("root-1", "title", "Root Decision");
        // RFC-038: shape classification is body-driven — a stray "tier" property
        // breaks note.json (additionalProperties: false), and the display title
        // must live in the body (the manifest index is no longer read).
        let note_json =
            serde_json::json!({ "instanceId": "note-1", "title": "My Note", "sections": [] });
        let store = build_store_titled(
            fields,
            vec![view],
            vec![dv],
            vec![
                ("root-1", 2, None, serde_json::to_value(&root).unwrap()),
                ("note-1", 0, Some("My Note"), note_json),
            ],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["note-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        assert_eq!(
            result.members.len(),
            2,
            "root + Tier-0 note must both appear"
        );
        let note = result
            .members
            .iter()
            .find(|m| m.instance_id == "note-1")
            .expect("Tier-0 note member must appear");
        assert_eq!(note.tier, 0);
        assert!(
            note.record.is_none(),
            "Tier-0 member must have record: None"
        );
        assert_eq!(note.display_label, "My Note");
        assert!(note.is_visible_by_default);
        assert!(result.diagnostics.is_empty(), "no diagnostics expected");
    }

    #[test]
    fn resolve_container_view_includes_tier1_typed_record_member() {
        // A Tier-1 member with a manifest index title must appear with record: None.
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
        let root = record("root-1", "title", "Root Decision");
        // RFC-038: Tier-1 shape is typed-record.json (requires `fields`); title
        // lives in the body; a stray "tier" property breaks shape-match.
        let typed_json = serde_json::json!({
            "instanceId": "typed-1",
            "title": "TypedRecord One",
            "fields": []
        });
        let store = build_store_titled(
            fields,
            vec![view],
            vec![dv],
            vec![
                ("root-1", 2, None, serde_json::to_value(&root).unwrap()),
                ("typed-1", 1, Some("TypedRecord One"), typed_json),
            ],
        );
        container_service::create_container(
            &store,
            make_container(vec!["root-1"], vec!["typed-1"]),
        )
        .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        let typed = result
            .members
            .iter()
            .find(|m| m.instance_id == "typed-1")
            .expect("Tier-1 member must appear");
        assert_eq!(typed.tier, 1);
        assert!(
            typed.record.is_none(),
            "Tier-1 member must have record: None"
        );
        assert_eq!(typed.display_label, "TypedRecord One");
        assert!(typed.is_visible_by_default);
        assert!(result.diagnostics.is_empty(), "no diagnostics expected");
    }

    #[test]
    fn resolve_container_view_tier01_label_falls_back_to_instance_id() {
        // When a Tier-0 member has no manifest index title, display_label falls back to
        // instance_id.
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
        let root = record("root-1", "title", "Root Decision");
        let note_json = serde_json::json!({ "instanceId": "note-1", "sections": [] });
        let store = build_store_titled(
            fields,
            vec![view],
            vec![dv],
            vec![
                ("root-1", 2, None, serde_json::to_value(&root).unwrap()),
                ("note-1", 0, None, note_json), // no title in body → fall back to instance_id
            ],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec!["note-1"]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        let note = result
            .members
            .iter()
            .find(|m| m.instance_id == "note-1")
            .expect("Tier-0 note member must appear");
        assert_eq!(note.tier, 0);
        assert_eq!(
            note.display_label, "note-1",
            "display_label falls back to instance_id"
        );
        assert!(note.record.is_none());
    }

    #[test]
    fn resolve_container_view_tier0_root_projects_correctly() {
        // A container whose root is a Tier-0 Note — root is resolved via the same
        // `resolve_member` path as members, so this guards against future special-casing.
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
        let note_json =
            serde_json::json!({ "instanceId": "note-root", "title": "Root Note", "sections": [] });
        let store = build_store_titled(
            fields,
            vec![view],
            vec![dv],
            vec![("note-root", 0, Some("Root Note"), note_json)],
        );
        container_service::create_container(&store, make_container(vec!["note-root"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();
        let root = result.root.as_ref().expect("root must be present");
        assert_eq!(root.tier, 0);
        assert!(root.record.is_none(), "Tier-0 root must have record: None");
        assert_eq!(root.display_label, "Root Note");
        assert!(root.is_visible_by_default);
        assert!(result.diagnostics.is_empty(), "no diagnostics expected");
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
        // Cross-store roundtrip (memory -> file) per CLAUDE.md storage rules, covering
        // a mixed-tier container: two Tier-2 records + one Tier-0 note member.
        // The snapshot importer requires identifiers >= 8 chars, so this test uses
        // its own snapshot-compliant fixture rather than the short-id `standard_store`.
        const F_TITLE: &str = "field-title-0001";
        const F_STATUS: &str = "field-status-0001";
        const VIEW: &str = "view-decision-0001";
        const DV: &str = "dv-decision-0001";
        const ROOT: &str = "record-root-0001";
        const MEM: &str = "record-member-0001";
        const NOTE: &str = "record-note-0001";

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
        let root = record(ROOT, "title", "Root Decision");
        let member = record(MEM, "title", "Member Decision");
        let note_json =
            serde_json::json!({ "instanceId": NOTE, "title": "Roundtrip Note", "sections": [] });
        let store = build_store_titled(
            fields,
            vec![view],
            vec![dv],
            vec![
                (ROOT, 2, None, serde_json::to_value(&root).unwrap()),
                (MEM, 2, None, serde_json::to_value(&member).unwrap()),
                (NOTE, 0, Some("Roundtrip Note"), note_json),
            ],
        );
        container_service::create_container(&store, make_container(vec![ROOT], vec![MEM, NOTE]))
            .unwrap();

        let from_memory = resolve_container_view(&store, input(None)).unwrap();

        // Verify the Tier-0 member is present with record: None.
        let note_mem = from_memory
            .members
            .iter()
            .find(|m| m.instance_id == NOTE)
            .expect("Tier-0 note must appear in memory result");
        assert_eq!(note_mem.tier, 0);
        assert!(
            note_mem.record.is_none(),
            "Tier-0 member must have record: None"
        );
        assert_eq!(note_mem.display_label, "Roundtrip Note");

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
        let root = record(ROOT, "title", "Root Decision");
        let member = record(MEM, "title", "Member Decision");
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

    #[test]
    fn resolve_view_is_visible_by_default_computed() {
        // Three members: no lifecycle state (→ true), "active" (→ true), "superseded" (→ false)
        // when the section declares excludeLifecycleStates: ["superseded", "abandoned"].
        const F_TITLE: &str = "f-title";

        let no_state = record("no-state", "title", "No State");

        let mut active = record("active-1", "title", "Active Record");
        active.lifecycle_state = Some("active".to_string());

        let mut superseded_rec = record("superseded1", "title", "Superseded Record");
        superseded_rec.lifecycle_state = Some("superseded".to_string());

        let dv = document_view(
            DV_ID,
            vec![section(
                "s1",
                0,
                type_query_source(Some(vec!["superseded", "abandoned"])),
                Some(VIEW_ID),
            )],
        );
        let store = build_store(
            vec![field(F_TITLE, "title")],
            vec![view_with_fields(vec![field_view(F_TITLE, 0, None, None)])],
            vec![dv],
            vec![
                ("no-state", 2, serde_json::to_value(&no_state).unwrap()),
                ("active-1", 2, serde_json::to_value(&active).unwrap()),
                (
                    "superseded1",
                    2,
                    serde_json::to_value(&superseded_rec).unwrap(),
                ),
            ],
        );
        container_service::create_container(
            &store,
            make_container(vec!["no-state"], vec!["active-1", "superseded1"]),
        )
        .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        // Root (no lifecycle state) is visible.
        assert!(
            result.root.as_ref().unwrap().is_visible_by_default,
            "None lifecycle state → visible by default"
        );
        // members order: roots-first (no-state), then active-1, superseded1.
        assert!(
            result.members[0].is_visible_by_default,
            "None lifecycle state → visible by default"
        );
        assert!(
            result.members[1].is_visible_by_default,
            "'active' not in exclusion list → visible by default"
        );
        assert!(
            !result.members[2].is_visible_by_default,
            "'superseded' in exclusion list → not visible by default"
        );
    }

    /// Cross-store roundtrip: `is_visible_by_default: false` must survive memory → file.
    #[test]
    fn resolve_view_roundtrip_is_visible_by_default_false() {
        const F_TITLE: &str = "field-title-0001";
        const VIEW: &str = "view-decision-0001";
        const DV: &str = "dv-decision-0001";
        const ROOT: &str = "record-root-0001";
        const SUPERSEDED: &str = "record-superseded-0001";

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
                type_query_source(Some(vec!["superseded", "abandoned"])),
                Some(VIEW),
            )],
            ..document_view(DV, vec![])
        };
        let root = record(ROOT, "title", "Root Decision");
        let mut superseded_rec = record(SUPERSEDED, "title", "Superseded Decision");
        superseded_rec.lifecycle_state = Some("superseded".to_string());

        let store = build_store(
            fields,
            vec![view],
            vec![dv],
            vec![
                (ROOT, 2, serde_json::to_value(&root).unwrap()),
                (
                    SUPERSEDED,
                    2,
                    serde_json::to_value(&superseded_rec).unwrap(),
                ),
            ],
        );
        container_service::create_container(&store, make_container(vec![ROOT], vec![SUPERSEDED]))
            .unwrap();

        let from_memory = resolve_container_view(&store, input(None)).unwrap();
        // Sanity: root is visible, superseded member is not.
        assert!(from_memory.root.as_ref().unwrap().is_visible_by_default);
        let sup_mem = from_memory
            .members
            .iter()
            .find(|m| m.instance_id == SUPERSEDED)
            .expect("superseded member present");
        assert!(
            !sup_mem.is_visible_by_default,
            "memory: 'superseded' → not visible by default"
        );

        // Roundtrip through FileStore.
        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = resolve_container_view(&file_store, input(None)).unwrap();

        let sup_file = from_file
            .members
            .iter()
            .find(|m| m.instance_id == SUPERSEDED)
            .expect("superseded member present in file result");
        assert!(
            !sup_file.is_visible_by_default,
            "file: 'superseded' → not visible by default"
        );

        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "is_visible_by_default must survive memory -> file roundtrip"
        );
    }

    // --- ADR-023 / RFC-020: ColumnSpec.is_identity_column ---

    fn record_type_with_identity(identity_field_id: Option<&str>) -> RecordType {
        RecordType {
            id: TYPE_ID.to_string(),
            namespace: "com.test".to_string(),
            name: "decision".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-title".to_string(),
                    order: 0,
                    required: true,
                    display_label: None,
                },
                FieldAssignment {
                    field_id: "f-status".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: identity_field_id.map(|s| s.to_string()),
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn record_type_with_identity_v2(identity_field_id: Option<&str>) -> RecordType {
        RecordType {
            id: TYPE_ID_2.to_string(),
            namespace: "com.test".to_string(),
            name: "decision-b".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "f-title".to_string(),
                    order: 0,
                    required: true,
                    display_label: None,
                },
                FieldAssignment {
                    field_id: "f-status".to_string(),
                    order: 1,
                    required: false,
                    display_label: None,
                },
            ],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: identity_field_id.map(|s| s.to_string()),
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    #[test]
    fn resolve_container_view_marks_identity_column_for_single_type_container() {
        let fvs = vec![
            field_view("f-title", 0, None, None),
            field_view("f-status", 1, None, None),
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
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(fvs);
        let dv = document_view(DV_ID, sections);
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![record_type_with_identity(Some("f-title"))],
            vec![view],
            vec![dv],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        let title_col = result
            .columns
            .iter()
            .find(|c| c.field_id == "f-title")
            .unwrap();
        let status_col = result
            .columns
            .iter()
            .find(|c| c.field_id == "f-status")
            .unwrap();
        assert!(
            title_col.is_identity_column,
            "f-title must be marked as the identity column"
        );
        assert!(
            !status_col.is_identity_column,
            "f-status must not be marked as the identity column"
        );
    }

    #[test]
    fn resolve_container_view_no_identity_field_id_all_columns_false() {
        let fvs = vec![
            field_view("f-title", 0, None, None),
            field_view("f-status", 1, None, None),
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
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(fvs);
        let dv = document_view(DV_ID, sections);
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![record_type_with_identity(None)],
            vec![view],
            vec![dv],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        assert!(
            result.columns.iter().all(|c| !c.is_identity_column),
            "no Type declares identityFieldId — every column must be false, got: {:?}",
            result.columns
        );
    }

    #[test]
    fn resolve_container_view_disagreeing_root_type_refs_all_columns_false() {
        // Two root_type_refs entries where the second (TYPE_ID_2) is absent from the identity
        // index (no RecordType installed for it) — types disagree/absent → all columns false.
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
        let fields = vec![field("f-title", "title")];
        let view = view_with_fields(fvs);
        let mut dv = document_view(DV_ID, sections);
        dv.root_type_refs = Some(vec![
            ExactTypeRef {
                type_id: TYPE_ID.to_string(),
                type_version: 1,
            },
            ExactTypeRef {
                type_id: TYPE_ID_2.to_string(),
                type_version: 1,
            },
        ]);
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![record_type_with_identity(Some("f-title"))],
            vec![view],
            vec![dv],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        assert!(
            result.columns.iter().all(|c| !c.is_identity_column),
            "disagreeing root_type_refs (one Type absent from index) must yield no identity signal, got: {:?}",
            result.columns
        );
    }

    #[test]
    fn resolve_container_view_view_id_override_still_resolves_identity_column() {
        // The explicit view_id branch skips document_views_for_container entirely
        // (container_view_service.rs ~126-135) — this test proves is_identity_column still
        // resolves correctly via the explicitly-referenced DocumentView's own root_type_refs.
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(vec![
            field_view("f-title", 0, None, None),
            field_view("f-status", 1, None, None),
        ]);
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
        // The alt DV also targets TYPE_ID via `document_view()`'s default root_type_refs, and
        // its own section carries a render_view_id so columns actually resolve.
        let alt = document_view(
            ALT_DV_ID,
            vec![section(
                "s1",
                0,
                SectionSource::FixedInstances {
                    instance_ids: vec![],
                },
                Some(VIEW_ID),
            )],
        );
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![record_type_with_identity(Some("f-title"))],
            vec![view],
            vec![primary, alt],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(Some(ALT_DV_ID))).unwrap();

        assert_eq!(result.document_view_id.as_deref(), Some(ALT_DV_ID));
        let title_col = result
            .columns
            .iter()
            .find(|c| c.field_id == "f-title")
            .unwrap();
        assert!(
            title_col.is_identity_column,
            "identity column must resolve via the explicitly-referenced DocumentView, got: {:?}",
            result.columns
        );
    }

    // --- ADR-027: common-identity extension for multi-entry root_type_refs ---

    fn multi_type_dv(sections: Vec<DocumentSection>) -> DocumentView {
        let mut dv = document_view(DV_ID, sections);
        dv.root_type_refs = Some(vec![
            ExactTypeRef {
                type_id: TYPE_ID.to_string(),
                type_version: 1,
            },
            ExactTypeRef {
                type_id: TYPE_ID_2.to_string(),
                type_version: 1,
            },
        ]);
        dv
    }

    fn two_col_sections() -> Vec<DocumentSection> {
        vec![section(
            "s1",
            0,
            SectionSource::ContainerSubset {
                container_id: CONTAINER_ID.to_string(),
                container_type: None,
                type_filter: None,
            },
            Some(VIEW_ID),
        )]
    }

    #[test]
    fn resolve_container_view_marks_identity_column_when_all_types_agree() {
        // ADR-027: two root_type_refs entries, both with identityFieldId "f-title" → common
        // identity → f-title column is true, f-status is false.
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(vec![
            field_view("f-title", 0, None, None),
            field_view("f-status", 1, None, None),
        ]);
        let dv = multi_type_dv(two_col_sections());
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![
                record_type_with_identity(Some("f-title")),
                record_type_with_identity_v2(Some("f-title")),
            ],
            vec![view],
            vec![dv],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        let title_col = result
            .columns
            .iter()
            .find(|c| c.field_id == "f-title")
            .unwrap();
        let status_col = result
            .columns
            .iter()
            .find(|c| c.field_id == "f-status")
            .unwrap();
        assert!(
            title_col.is_identity_column,
            "all types agree on f-title → must be marked identity column, got: {:?}",
            result.columns
        );
        assert!(
            !status_col.is_identity_column,
            "f-status must not be identity column, got: {:?}",
            result.columns
        );
    }

    #[test]
    fn resolve_container_view_no_signal_when_types_disagree_on_identity() {
        // ADR-027: two root_type_refs with different identityFieldIds → no common identity →
        // all columns false.
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(vec![
            field_view("f-title", 0, None, None),
            field_view("f-status", 1, None, None),
        ]);
        let dv = multi_type_dv(two_col_sections());
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![
                record_type_with_identity(Some("f-title")),
                record_type_with_identity_v2(Some("f-status")),
            ],
            vec![view],
            vec![dv],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        assert!(
            result.columns.iter().all(|c| !c.is_identity_column),
            "types disagree on identity field → every column must be false, got: {:?}",
            result.columns
        );
    }

    #[test]
    fn resolve_container_view_no_signal_when_one_type_has_no_identity() {
        // ADR-027: two root_type_refs where one has no identityFieldId → can't agree →
        // all columns false.
        let fields = vec![field("f-title", "title"), field("f-status", "status")];
        let view = view_with_fields(vec![
            field_view("f-title", 0, None, None),
            field_view("f-status", 1, None, None),
        ]);
        let dv = multi_type_dv(two_col_sections());
        let root = record("root-1", "title", "Root Decision");
        let store = build_store_with_types(
            fields,
            vec![
                record_type_with_identity(Some("f-title")),
                record_type_with_identity_v2(None),
            ],
            vec![view],
            vec![dv],
            vec![("root-1", 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec!["root-1"], vec![]))
            .unwrap();

        let result = resolve_container_view(&store, input(None)).unwrap();

        assert!(
            result.columns.iter().all(|c| !c.is_identity_column),
            "one type has no identityFieldId → every column must be false, got: {:?}",
            result.columns
        );
    }

    /// Cross-store roundtrip: common-identity `is_identity_column: true` must survive memory → file.
    #[test]
    fn resolve_view_roundtrip_marks_identity_column_when_all_types_agree() {
        const F_TITLE: &str = "field-title-0001";
        const F_STATUS: &str = "field-status-0001";
        const VIEW: &str = "view-decision-0001";
        const DV: &str = "dv-decision-0001";
        const TYPE_A: &str = "type-decision-aaa1";
        const TYPE_B: &str = "type-decision-bbb1";
        const ROOT: &str = "record-root-0001";

        let fields = vec![field(F_TITLE, "title"), field(F_STATUS, "status")];
        let view = View {
            id: VIEW.to_string(),
            ..view_with_fields(vec![
                field_view(F_TITLE, 0, None, None),
                field_view(F_STATUS, 1, None, None),
            ])
        };
        let dv = DocumentView {
            id: DV.to_string(),
            root_type_refs: Some(vec![
                ExactTypeRef {
                    type_id: TYPE_A.to_string(),
                    type_version: 1,
                },
                ExactTypeRef {
                    type_id: TYPE_B.to_string(),
                    type_version: 1,
                },
            ]),
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
        let make_rt = |id: &str| RecordType {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: id.to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![FieldAssignment {
                field_id: F_TITLE.to_string(),
                order: 0,
                required: true,
                display_label: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: Some(F_TITLE.to_string()),
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        };
        let root = Record {
            field_meta: None,
            instance_id: ROOT.to_string(),
            type_id: TYPE_A.to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "decision".to_string(),
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert("title", serde_json::json!("Root Decision"));
                fv
            },
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        };
        let store = build_store_with_types(
            fields,
            vec![make_rt(TYPE_A), make_rt(TYPE_B)],
            vec![view],
            vec![dv],
            vec![(ROOT, 2, serde_json::to_value(&root).unwrap())],
        );
        container_service::create_container(&store, make_container(vec![ROOT], vec![])).unwrap();

        let from_memory = resolve_container_view(&store, input(None)).unwrap();
        let title_col_mem = from_memory
            .columns
            .iter()
            .find(|c| c.field_id == F_TITLE)
            .unwrap();
        assert!(
            title_col_mem.is_identity_column,
            "memory: all types agree → f-title must be identity column"
        );

        let temp = tempfile::TempDir::new().unwrap();
        let file_store = crate::store::FileStore::new(temp.path());
        crate::repository_portability::copy_repository(&store, &file_store).unwrap();
        let from_file = resolve_container_view(&file_store, input(None)).unwrap();

        let title_col_file = from_file
            .columns
            .iter()
            .find(|c| c.field_id == F_TITLE)
            .unwrap();
        assert!(
            title_col_file.is_identity_column,
            "file: is_identity_column: true must survive memory → file roundtrip"
        );
        assert_eq!(
            serde_json::to_value(&from_memory).unwrap(),
            serde_json::to_value(&from_file).unwrap(),
            "full ContainerView must be identical across stores"
        );
    }
}
