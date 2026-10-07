use serde::{Deserialize, Serialize};

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::{
    container_service, package_service, protocol_run_service, record_store, relation_service,
};
use relation_service::ListRelationsFilter;
use srs_core::arrangement::OutlineEntry;
use srs_core::types::relation_type_definition::RelationTypeCategory;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldContextQuery {
    pub record_id: String,
    pub field_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordContextQuery {
    pub record_id: String,
    /// When set, the record must be a member of this container and the result carries its
    /// arrangement `entry` and `subtree` (#1134).
    #[serde(default)]
    pub container_id: Option<String>,
    /// Drop edges whose relation type's `RelationTypeDefinition.category` is listed (#1188),
    /// e.g. `composition` + `sequence` removes contains/precedes without naming them. Empty =
    /// no filtering (default). Edges whose type has no installed definition are kept.
    #[serde(default)]
    pub exclude_relation_categories: Vec<RelationTypeCategory>,
    /// How much of each neighbour to inline (#1285). Default `full`.
    #[serde(default)]
    pub projection: ContextProjection,
}

/// How much of each relation neighbour [`get_record_context`] inlines (#1285). The context
/// record itself is always returned in full; only the instances at the other end shrink.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextProjection {
    /// Every neighbour as the whole Record/Note, plus each edge's endpoint labels and provenance.
    #[default]
    Full,
    /// Each neighbour as a [`NeighbourSummary`] card: id, uri, label, type, lifecycle state and
    /// one summary field. Edges drop their endpoint labels and provenance (the card carries the
    /// label; `full` keeps the provenance). Enough to decide which neighbour to read next.
    Card,
    /// As `card`, without the summary: identity and label only.
    Label,
}

impl std::str::FromStr for ContextProjection {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "full" => Ok(ContextProjection::Full),
            "card" => Ok(ContextProjection::Card),
            "label" => Ok(ContextProjection::Label),
            other => Err(format!(
                "invalid projection '{other}' (expected full|card|label)"
            )),
        }
    }
}

/// Characters of the summary field kept on a `card` neighbour.
pub const CARD_SUMMARY_CHARS: usize = 160;

/// Which end of the edge the context record sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EdgeDirection {
    /// The context record is the relation's source.
    Out,
    /// The context record is the relation's target.
    In,
}

impl EdgeDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeDirection::Out => "out",
            EdgeDirection::In => "in",
        }
    }
}

impl std::str::FromStr for EdgeDirection {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "out" => Ok(EdgeDirection::Out),
            "in" => Ok(EdgeDirection::In),
            other => Err(format!("invalid direction '{other}' (expected out|in)")),
        }
    }
}

/// A relation neighbour, loaded by tier (`LoadedInstance` is not `Serialize`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContextInstance {
    Record(srs_core::types::record::Record),
    Note(srs_core::types::note::Note),
    /// A `card`/`label` projection of a Record or Note (#1285).
    Card(NeighbourSummary),
}

/// One edge touching the context record: the relation, which way it points, and the
/// instance at the other end inline (`None` when it does not resolve).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRelation {
    pub direction: EdgeDirection,
    #[serde(flatten)]
    pub relation: crate::relation_service::RelationSummary,
    pub neighbour: Option<ContextInstance>,
    /// The relation's own provenance (#1246): when it was asserted and by whom, so an
    /// agent can find the attachments it made. Absent on legacy/unstamped relations.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_by: Option<srs_core::types::actor::Actor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldContextResult {
    pub record_id: String,
    pub field_id: String,
    pub field_name: Option<String>,
    pub field_namespace: Option<String>,
    /// None when field not in package, or when field.ai_guidance.purpose is empty
    pub ai_guidance: Option<serde_json::Value>,
    pub current_value: Option<serde_json::Value>,
    /// Always empty; placeholder for tagged-chunk storage (#582)
    pub tagged_chunks: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordContextResult {
    pub record_id: String,
    /// type_id/type_name/type_namespace are String (not Option) — always present on a
    /// found Tier-2 Record
    pub type_id: String,
    pub type_name: String,
    pub type_namespace: String,
    pub display_label: String,
    pub field_values: srs_core::types::record::FieldValues,
    /// Every relation touching the record, both directions, sorted by relationType, then
    /// neighbour `createdAt`, then relationId (a comment thread reads chronologically).
    pub relations: Vec<ContextRelation>,
    /// Set with `RecordContextQuery::container_id`: this record's arrangement entry.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub container_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub entry: Option<OutlineEntry>,
    /// Descendants of `entry` in that container (outline entries only).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub subtree: Option<Vec<OutlineEntry>>,
    /// Always empty; placeholder for tagged-chunk storage (#582)
    pub tagged_chunks: Vec<serde_json::Value>,
    /// Protocol runs targeting this record
    pub protocol_run_history: Vec<serde_json::Value>,
}

/// Assemble field context: current value and aiGuidance from package.
pub fn get_field_context(
    store: &dyn RepositoryStore,
    query: FieldContextQuery,
) -> Result<FieldContextResult, RepositoryError> {
    let record = record_store::get_record_by_id(store, &query.record_id)?.ok_or_else(|| {
        RepositoryError::NotFound {
            path: std::path::PathBuf::from(&query.record_id),
        }
    })?;

    // Query addresses the field by id; the RFC-039 carrier keys by name —
    // recover the name through the package (Type-mediated resolution).
    let field_name = store
        .load_package()?
        .resolve_field(&query.field_id)
        .map(|f| f.name.clone());
    let current_value = field_name
        .as_deref()
        .and_then(|name| record.value(name))
        .cloned()
        .filter(|v| !v.is_null());

    let (field_name, field_namespace, ai_guidance) =
        match package_service::get_field_by_id(store, &query.field_id)? {
            package_service::GetFieldResult::Found(field) => {
                let guidance = field
                    .ai_guidance
                    .as_ref()
                    .filter(|g| !g.purpose.is_empty())
                    .and_then(|g| serde_json::to_value(g).ok());
                (
                    Some(field.name.clone()),
                    Some(field.namespace.clone()),
                    guidance,
                )
            }
            package_service::GetFieldResult::NotFound => (None, None, None),
        };

    Ok(FieldContextResult {
        record_id: query.record_id,
        field_id: query.field_id,
        field_name,
        field_namespace,
        ai_guidance,
        current_value,
        tagged_chunks: vec![],
    })
}

/// Assemble record context: all field values, every relation touching the record (both
/// directions, neighbour inline) and, given a container, the record's arrangement subtree.
pub fn get_record_context(
    store: &dyn RepositoryStore,
    query: RecordContextQuery,
) -> Result<RecordContextResult, RepositoryError> {
    let summary =
        record_store::get_record_summary_by_id(store, &query.record_id)?.ok_or_else(|| {
            RepositoryError::NotFound {
                path: std::path::PathBuf::from(&query.record_id),
            }
        })?;

    let compact = query.projection != ContextProjection::Full;
    // Only load the package when filtering or projecting: the default path stays as it was.
    // Category filtering needs it; a compact projection only improves with it (labels and
    // summaries), so — like `list_neighbours` — it tolerates a package that will not load.
    let package = if !query.exclude_relation_categories.is_empty() {
        Some(store.load_package()?)
    } else if compact {
        store.load_package().ok()
    } else {
        None
    };
    let card_ctx = if compact {
        let manifest = store.load_manifest()?;
        Some(CardContext {
            package: package.as_ref(),
            labels: package
                .as_ref()
                .map(crate::record_label::build_label_indexes_from_package),
            repo_id: crate::resource_uri::repository_id(&manifest)
                .unwrap_or_default()
                .to_string(),
            with_summary: query.projection == ContextProjection::Card,
        })
    } else {
        None
    };
    // ponytail: two full relation scans and a neighbour load per edge; add a per-id cache /
    // single pass if hub records measure slow. A self-relation appears once as out, once as in.
    let mut relations = Vec::new();
    for (direction, filter) in [
        (
            EdgeDirection::Out,
            ListRelationsFilter {
                source: Some(query.record_id.clone()),
                ..Default::default()
            },
        ),
        (
            EdgeDirection::In,
            ListRelationsFilter {
                target: Some(query.record_id.clone()),
                ..Default::default()
            },
        ),
    ] {
        for relation in relation_service::list_relations(store, filter)? {
            if let Some(def) = package
                .as_ref()
                .and_then(|p| p.resolve_relation_type(&relation.relation_type))
            {
                if query.exclude_relation_categories.contains(&def.category) {
                    continue;
                }
            }
            let other = match direction {
                EdgeDirection::Out => &relation.target_id,
                EdgeDirection::In => &relation.source_id,
            };
            let neighbour = record_store::get_instance_by_id(store, other)?;
            let created = neighbour
                .as_ref()
                .and_then(|n| n.created_at())
                .map(str::to_string);
            let neighbour = neighbour.map(|n| match (&card_ctx, n) {
                (Some(c), n) => ContextInstance::Card(c.card(other, &n)),
                (None, record_store::LoadedInstance::Record(r)) => ContextInstance::Record(r),
                (None, record_store::LoadedInstance::Note(n)) => ContextInstance::Note(n),
            });
            let mut relation = relation;
            let own = if compact {
                relation.source_label = None;
                relation.target_label = None;
                None
            } else {
                store.load_relation(&relation.relation_id).ok()
            };
            relations.push((
                created,
                ContextRelation {
                    direction,
                    created_at: own.as_ref().and_then(|r| r.created_at.clone()),
                    created_by: own.and_then(|r| r.created_by),
                    relation,
                    neighbour,
                },
            ));
        }
    }
    relations.sort_by(|(ca, a), (cb, b)| {
        // createdAt: None sorts last (Option's own order puts it first).
        let key = |c: &Option<String>| (c.is_none(), c.clone());
        (&a.relation.relation_type, key(ca), &a.relation.relation_id).cmp(&(
            &b.relation.relation_type,
            key(cb),
            &b.relation.relation_id,
        ))
    });
    let relations = relations.into_iter().map(|(_, r)| r).collect();

    let (entry, subtree) = match query.container_id.as_deref() {
        None => (None, None),
        Some(cid) => {
            let outline = container_service::get_outline(store, cid)?;
            let i = outline
                .entries
                .iter()
                .position(|e| e.instance_id == query.record_id)
                .ok_or_else(|| RepositoryError::InvalidInput {
                    message: format!("{} is not a member of container {cid}", query.record_id),
                })?;
            let entry = outline.entries[i].clone();
            let subtree = outline.entries[i + 1..entry.run_end].to_vec();
            (Some(entry), Some(subtree))
        }
    };

    let protocol_run_history = protocol_run_service::list_runs_for_record(store, &query.record_id)
        .unwrap_or_else(|_| vec![])
        .into_iter()
        .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
        .collect();

    Ok(RecordContextResult {
        record_id: query.record_id,
        type_id: summary.record.type_id.clone(),
        type_name: summary.record.type_name.clone(),
        type_namespace: summary.record.type_namespace.clone(),
        display_label: summary.display_label.clone(),
        field_values: summary.record.field_values.clone(),
        relations,
        container_id: query.container_id,
        entry,
        subtree,
        tagged_chunks: vec![],
        protocol_run_history,
    })
}

/// [`get_record_context`] rendered as compact markdown for an agent (#1285): the record's
/// fields as the renderer's baseline rows ([`crate::render_service`], so values, labels and
/// order match every other markdown the engine emits), then one line per edge grouped by
/// relation type and direction, each carrying the neighbour's label, type, lifecycle state
/// and instance id, with the `card` summary beneath. `query.projection` is forced to
/// `card` unless it is `label` (no summaries); a `full` read has no compact form.
pub fn render_record_context_markdown(
    store: &dyn RepositoryStore,
    mut query: RecordContextQuery,
) -> Result<String, RepositoryError> {
    if query.projection == ContextProjection::Full {
        query.projection = ContextProjection::Card;
    }
    let ctx = get_record_context(store, query)?;
    let record = record_store::get_record_by_id(store, &ctx.record_id)?.ok_or_else(|| {
        RepositoryError::NotFound {
            path: std::path::PathBuf::from(&ctx.record_id),
        }
    })?;
    let package = store.load_package().ok();
    // Render diagnostics have no channel in a markdown read; the rows still render.
    let mut diagnostics = Vec::new();
    let rows = crate::render_service::render_record_rows(
        package.as_ref(),
        &record,
        "markdown",
        Some(ctx.display_label.trim()),
        &mut diagnostics,
    );
    Ok(context_markdown(&ctx, &rows))
}

fn context_markdown(ctx: &RecordContextResult, field_rows: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# {}", ctx.display_label);
    let _ = writeln!(
        out,
        "`{}` · {}/{}\n",
        ctx.record_id, ctx.type_namespace, ctx.type_name
    );
    out.push_str(field_rows);
    if let (Some(cid), Some(subtree)) = (&ctx.container_id, &ctx.subtree) {
        let _ = writeln!(
            out,
            "In container `{cid}`; {} descendant(s){}\n",
            subtree.len(),
            subtree
                .iter()
                .map(|e| format!("\n- `{}`", e.instance_id))
                .collect::<String>()
        );
    }
    // One heading per (relation type, direction): the service orders by type then
    // neighbour createdAt, which interleaves in- and out-edges of one type; a stable sort
    // groups them and keeps that order within each group.
    let mut edges: Vec<&ContextRelation> = ctx.relations.iter().collect();
    edges.sort_by_key(|r| {
        (
            r.relation.relation_type.as_str(),
            r.direction == EdgeDirection::In,
        )
    });
    if !edges.is_empty() {
        let _ = writeln!(out, "## Relations ({})", edges.len());
    }
    let mut group: Option<(&str, EdgeDirection)> = None;
    for rel in edges {
        let key = (rel.relation.relation_type.as_str(), rel.direction);
        if group != Some(key) {
            let arrow = match rel.direction {
                EdgeDirection::Out => "→",
                EdgeDirection::In => "←",
            };
            let _ = writeln!(out, "\n### {arrow} {}", rel.relation.relation_type);
            group = Some(key);
        }
        let other = match rel.direction {
            EdgeDirection::Out => &rel.relation.target_id,
            EdgeDirection::In => &rel.relation.source_id,
        };
        match &rel.neighbour {
            Some(ContextInstance::Card(c)) => {
                let kind = c.type_name.as_deref().unwrap_or("note");
                let state = c
                    .lifecycle_state
                    .as_deref()
                    .map(|s| format!(" · {s}"))
                    .unwrap_or_default();
                let label = c.label.as_deref().unwrap_or(other);
                let _ = writeln!(out, "- {label} ({kind}{state}) `{other}`");
                if let Some(summary) = &c.summary {
                    let _ = writeln!(out, "  {}", summary.replace('\n', " "));
                }
            }
            // `full` neighbours never reach here (the projection is forced); a dangling
            // edge has no neighbour.
            _ => {
                let _ = writeln!(out, "- (unresolved) `{other}`");
            }
        }
    }
    if !ctx.protocol_run_history.is_empty() {
        let _ = writeln!(
            out,
            "\n{} protocol run(s) target this record.",
            ctx.protocol_run_history.len()
        );
    }
    out
}

/// What a `card`/`label` projection needs, loaded once per [`get_record_context`] call.
/// `package`/`labels` are `None` when the package does not load: cards then carry
/// identity and type only, as `list_neighbours` does.
struct CardContext<'a> {
    package: Option<&'a crate::package::Package>,
    labels: Option<LabelIndexes>,
    repo_id: String,
    with_summary: bool,
}

type LabelIndexes = (
    crate::record_label::FieldNameIndex,
    crate::record_label::IdentityFieldIndex,
);

impl CardContext<'_> {
    fn card(&self, id: &str, instance: &record_store::LoadedInstance) -> NeighbourSummary {
        let mut card = neighbour_summary(id, Some(instance), self.labels.as_ref(), &self.repo_id);
        match instance {
            record_store::LoadedInstance::Record(rec) => {
                card.lifecycle_state = rec.lifecycle_state.clone();
                if self.with_summary {
                    card.summary = self.package.and_then(|p| {
                        record_summary(rec, p, card.label.as_deref().unwrap_or_default())
                    });
                }
            }
            record_store::LoadedInstance::Note(n) if self.with_summary => {
                card.summary = n
                    .sections
                    .iter()
                    .map(|s| s.content.as_str())
                    .find(|c| !c.trim().is_empty())
                    .map(truncate_summary);
            }
            record_store::LoadedInstance::Note(_) => {}
        }
        card
    }
}

/// The one mapping from a neighbour instance to its [`NeighbourSummary`], shared by
/// `list_neighbours` and the context `card`/`label` projection: label via
/// `record_display_label` (Tier 2, when the label indexes loaded) or the note title,
/// type for records, and the `srs://` uri. `lifecycle_state`/`summary` are left `None`.
fn neighbour_summary(
    id: &str,
    instance: Option<&record_store::LoadedInstance>,
    labels: Option<&LabelIndexes>,
    repo_id: &str,
) -> NeighbourSummary {
    let (label, type_namespace, type_name) = match instance {
        Some(record_store::LoadedInstance::Record(rec)) => (
            labels.map(|(fni, ifi)| crate::record_label::record_display_label(rec, ifi, fni)),
            Some(rec.type_namespace.clone()),
            Some(rec.type_name.clone()),
        ),
        Some(record_store::LoadedInstance::Note(n)) => (
            Some(n.title.clone().unwrap_or_else(|| n.instance_id.clone())),
            None,
            None,
        ),
        None => (None, None, None),
    };
    NeighbourSummary {
        uri: crate::resource_uri::record_uri(repo_id, id),
        instance_id: id.to_string(),
        label,
        type_namespace,
        type_name,
        lifecycle_state: None,
        summary: None,
    }
}

/// The first non-empty string field, in the Type's effective field order (inheritance,
/// `fieldOrder` and overrides applied), that is not the label: the one line a card shows
/// beside the label. `None` when the Type does not resolve.
fn record_summary(
    rec: &srs_core::types::record::Record,
    package: &crate::package::Package,
    label: &str,
) -> Option<String> {
    let rt = package.resolve_type(&rec.type_id, rec.type_version)?;
    let fields = package.effective_fields(rt).ok()?;
    fields.iter().find_map(|fa| {
        let name = &package.resolve_field(&fa.field_id)?.name;
        let text = rec.value(name)?.as_str()?.trim();
        (!text.is_empty() && text != label).then(|| truncate_summary(text))
    })
}

/// At most [`CARD_SUMMARY_CHARS`] chars, cut on a char boundary and marked with `…`.
fn truncate_summary(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= CARD_SUMMARY_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(CARD_SUMMARY_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// Which edges of `instance_id` to list (srs-rust#1229). `direction: None` = both.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighboursQuery {
    pub instance_id: String,
    #[serde(default)]
    pub relation_type: Option<String>,
    #[serde(default)]
    pub direction: Option<EdgeDirection>,
}

/// Result shaping for [`list_neighbours`], mirroring `discovery_service::FindPage`:
/// `limit: None` means every edge; any default cap is the adapter's choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NeighboursPage {
    pub limit: Option<usize>,
    pub offset: usize,
}

/// The instance at the other end of an edge: identity and a display label only, never the
/// record. `label`/`type*` are omitted when the neighbour does not resolve (or fails to load)
/// and `type*` for a Tier-0 note.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighbourSummary {
    pub instance_id: String,
    /// `srs://<repo>/record/<id>`, the same URI a `find` hit carries (#1227).
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// Set by a context `card`/`label` projection (#1285); never by `list_neighbours`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lifecycle_state: Option<String>,
    /// A context `card` projection's one-line summary (#1285): the first non-label string
    /// field, at most [`CARD_SUMMARY_CHARS`] chars. Never set by `list_neighbours`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighbourEdge {
    pub direction: EdgeDirection,
    pub relation_id: String,
    pub relation_type: String,
    pub neighbour: NeighbourSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeighboursResult {
    pub instance_id: String,
    /// Every matching edge, before paging.
    pub total: usize,
    pub neighbours: Vec<NeighbourEdge>,
}

/// Bounded read of an instance's relation neighbours. Edges are filtered, sorted by
/// `(relationType, createdAt none-last, relationId)` and paged first; only the returned page
/// loads its neighbours (unlike [`get_record_context`], which inlines all of them).
pub fn list_neighbours(
    store: &dyn RepositoryStore,
    query: NeighboursQuery,
    page: NeighboursPage,
) -> Result<NeighboursResult, RepositoryError> {
    let id = &query.instance_id;
    if record_store::get_instance_by_id(store, id)?.is_none() {
        return Err(RepositoryError::NotFound {
            path: std::path::PathBuf::from(id),
        });
    }
    let wants = |d| query.direction.is_none_or(|q| q == d);
    let mut edges = Vec::new();
    for r in relation_service::load_relations(store)? {
        if query
            .relation_type
            .as_ref()
            .is_some_and(|t| *t != r.relation_type)
        {
            continue;
        }
        // A self-relation is both an out and an in edge, as in `get_record_context`.
        if r.source_instance_id == *id && wants(EdgeDirection::Out) {
            edges.push((EdgeDirection::Out, r.target_instance_id.clone(), r.clone()));
        }
        if r.target_instance_id == *id && wants(EdgeDirection::In) {
            edges.push((EdgeDirection::In, r.source_instance_id.clone(), r));
        }
    }
    // createdAt: None sorts last (Option's own order puts it first).
    edges.sort_by(|(_, _, a), (_, _, b)| {
        (
            &a.relation_type,
            a.created_at.is_none(),
            &a.created_at,
            &a.relation_id,
        )
            .cmp(&(
                &b.relation_type,
                b.created_at.is_none(),
                &b.created_at,
                &b.relation_id,
            ))
    });
    let total = edges.len();
    // Label indexes only matter for Tier-2 neighbours; tolerate a package that will not load.
    let indexes = crate::record_label::build_label_indexes(store).ok();
    let manifest = store.load_manifest()?;
    let repo_id = crate::resource_uri::repository_id(&manifest).unwrap_or_default();
    let neighbours = edges
        .into_iter()
        .skip(page.offset)
        .take(page.limit.unwrap_or(usize::MAX))
        .map(|(direction, other, r)| {
            let instance = record_store::get_instance_by_id(store, &other)
                .ok()
                .flatten();
            NeighbourEdge {
                direction,
                relation_id: r.relation_id,
                relation_type: r.relation_type,
                neighbour: neighbour_summary(&other, instance.as_ref(), indexes.as_ref(), repo_id),
            }
        })
        .collect();
    Ok(NeighboursResult {
        instance_id: query.instance_id,
        total,
        neighbours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::record_store;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use srs_core::types::field::{AiGuidance, Field, FieldType};
    use srs_core::types::record::FieldValues;
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use std::path::PathBuf;

    fn make_store() -> MemoryStore {
        make_store_with(vec![])
    }

    fn make_store_with(
        relation_type_definitions: Vec<
            srs_core::types::relation_type_definition::RelationTypeDefinition,
        >,
    ) -> MemoryStore {
        let name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            // Intentionally empty (not the crate-wide "Test guidance" default) — this
            // MemoryStore-typed-Package fixture never round-trips through JSON/catalog
            // validation, and `field_context_ai_guidance_null` specifically exercises
            // the empty-guidance → `ai_guidance: None` behavior.
            ai_guidance: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        // Optional second text field: what a `card` projection shows as the summary (#1285).
        let summary_field = Field {
            id: "field-summary-001".to_string(),
            name: "test-summary".to_string(),
            description: "Summary field".to_string(),
            ..name_field.clone()
        };
        let test_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type".to_string(),
            fields: vec![
                FieldAssignment {
                    field_id: "field-name-001".to_string(),
                    order: 0,
                    required: true,
                    display_label: Some("Name".to_string()),
                    description: None,
                },
                FieldAssignment {
                    field_id: "field-summary-001".to_string(),
                    order: 1,
                    required: false,
                    display_label: Some("Summary".to_string()),
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
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field, summary_field],
            record_types: vec![test_type],
            relation_type_definitions,
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
        MemoryStore::new(manifest, package)
    }

    fn make_field_values(name: &str, value: serde_json::Value) -> FieldValues {
        let mut fv = FieldValues::new();
        fv.insert(name, value);
        fv
    }

    #[test]
    fn field_context_current_value() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Alice"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id.clone(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert_eq!(result.record_id, rec.instance_id);
        assert_eq!(result.field_id, "field-name-001");
        assert_eq!(result.current_value, Some(json!("Alice")));
    }

    #[test]
    fn field_context_ai_guidance_from_package() {
        // Build a store where field-name-001 has non-null ai_guidance
        let mut name_field = Field {
            schema: None,
            id: "field-name-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-name".to_string(),
            version: 1,
            field_type: FieldType::string(),
            description: "Name field".to_string(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        // Optional second text field: what a `card` projection shows as the summary (#1285).
        let summary_field = Field {
            id: "field-summary-001".to_string(),
            name: "test-summary".to_string(),
            description: "Summary field".to_string(),
            ..name_field.clone()
        };
        let test_type = RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: "type-test-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "Test type".to_string(),
            fields: vec![FieldAssignment {
                field_id: "field-name-001".to_string(),
                order: 0,
                required: true,
                display_label: None,
                description: None,
            }],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        };
        name_field.ai_guidance = Some(AiGuidance {
            purpose: "Write the full legal name".to_string(),
            ..Default::default()
        });
        let manifest = Manifest {
            container: None,
            upstream_package: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-package-001".to_string(),
            namespace: "com.test".to_string(),
            name: "test-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![name_field, summary_field],
            record_types: vec![test_type],
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

        let fv = make_field_values("test-name", json!("Charlie"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id,
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            result.ai_guidance,
            Some(json!({"purpose": "Write the full legal name"}))
        );
        assert_eq!(result.field_name, Some("test-name".to_string()));
        assert_eq!(result.field_namespace, Some("com.test".to_string()));
    }

    #[test]
    fn field_context_ai_guidance_null() {
        // make_store() has ai_guidance: null on field-name-001
        let store = make_store();
        let fv = make_field_values("test-name", json!("Dana"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_field_context(
            &store,
            FieldContextQuery {
                record_id: rec.instance_id,
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap();

        assert!(result.ai_guidance.is_none());
    }

    #[test]
    fn field_context_not_found() {
        let store = make_store();
        let err = get_field_context(
            &store,
            FieldContextQuery {
                record_id: "nonexistent-record-id".to_string(),
                field_id: "field-name-001".to_string(),
            },
        )
        .unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }

    #[test]
    fn record_context_field_values() {
        let store = make_store();
        let fv = make_field_values("test-name", json!("Eve"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: rec.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        )
        .unwrap();

        assert_eq!(result.record_id, rec.instance_id);
        assert_eq!(result.type_id, "type-test-001");
        assert_eq!(result.type_name, "test-type");
        assert_eq!(result.type_namespace, "com.test");
        assert_eq!(result.field_values.len(), 1);
        assert_eq!(result.field_values.get("test-name"), Some(&json!("Eve")));
        assert!(result.tagged_chunks.is_empty());
        assert!(result.protocol_run_history.is_empty());
    }

    #[test]
    fn record_context_relations() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };
        let store = make_store();
        let depends_on_def = RelationTypeDefinition {
            schema: None,
            id: "rtd-depends-on".to_string(),
            version: 1,
            key: "depends-on".to_string(),
            namespace: "com.test".to_string(),
            label: "depends-on".to_string(),
            description: "Dependency relation".to_string(),
            category: RelationTypeCategory::Dependency,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };
        let defs = vec![depends_on_def];
        let fv1 = make_field_values("test-name", json!("Source"));
        let fv2 = make_field_values("test-name", json!("Target"));
        let src = record_store::create_record(&store, "type-test-001", 1, fv1, None, None).unwrap();
        let tgt = record_store::create_record(&store, "type-test-001", 1, fv2, None, None).unwrap();
        let unrelated = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("Unrelated")),
            None,
            None,
        )
        .unwrap();

        create_relation(
            &store,
            Relation {
                created_by: None,
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: src.instance_id.clone(),
                target_instance_id: tgt.instance_id.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            },
            &defs,
        )
        .unwrap();
        // Create a relation FROM unrelated to something — must not appear in src's context
        create_relation(
            &store,
            Relation {
                created_by: None,
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: unrelated.instance_id.clone(),
                target_instance_id: src.instance_id.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            },
            &defs,
        )
        .unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: src.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        )
        .unwrap();

        // Both directions are returned; the unrelated record's edge TO src is the inbound one.
        assert_eq!(result.relations.len(), 2);
        let out = result
            .relations
            .iter()
            .find(|r| r.direction == EdgeDirection::Out)
            .unwrap();
        assert_eq!(out.relation.source_id, src.instance_id);
        assert_eq!(out.relation.target_id, tgt.instance_id);
        assert!(
            matches!(&out.neighbour, Some(ContextInstance::Record(r)) if r.instance_id == tgt.instance_id)
        );
        let inn = result
            .relations
            .iter()
            .find(|r| r.direction == EdgeDirection::In)
            .unwrap();
        assert_eq!(inn.relation.source_id, unrelated.instance_id);
        assert!(
            matches!(&inn.neighbour, Some(ContextInstance::Record(r)) if r.instance_id == unrelated.instance_id)
        );
        assert!(result.entry.is_none() && result.subtree.is_none());
    }

    fn depends_on_defs() -> [srs_core::types::relation_type_definition::RelationTypeDefinition; 1] {
        [
            srs_core::types::relation_type_definition::RelationTypeDefinition {
                schema: None,
                id: "rtd-depends-on".to_string(),
                version: 1,
                key: "depends-on".to_string(),
                namespace: "com.test".to_string(),
                label: "depends-on".to_string(),
                description: "Dependency relation".to_string(),
                category: RelationTypeCategory::Dependency,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                canonical_direction: None,
                inverse_type: None,
                irreflexive: None,
                require_same_type: None,
                status: None,
                updated_at: None,
                meta: None,
            },
        ]
    }

    #[test]
    fn record_context_card_and_label_projections_shrink_neighbours_only() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        let store = make_store();
        let defs = depends_on_defs();
        let mut fv = make_field_values("test-name", json!("Hub"));
        fv.insert("test-summary".to_string(), json!("hub summary"));
        let hub = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        // The summary is the first string field (Type order) that is not the label.
        let long = "x".repeat(CARD_SUMMARY_CHARS + 50);
        let mut fv = make_field_values("test-name", json!(long));
        fv.insert("test-summary".to_string(), json!("second field"));
        let spoke =
            record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let bare = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("Bare")),
            None,
            None,
        )
        .unwrap();
        for target in [&spoke.instance_id, &bare.instance_id] {
            create_relation(
                &store,
                Relation {
                    created_by: None,
                    relation_id: String::new(),
                    relation_type: "depends-on".to_string(),
                    source_instance_id: hub.instance_id.clone(),
                    target_instance_id: target.clone(),
                    created_at: None,
                    notes: None,
                    source_refs: None,
                    meta: None,
                },
                &defs,
            )
            .unwrap();
        }
        let ctx = |projection| {
            get_record_context(
                &store,
                RecordContextQuery {
                    record_id: hub.instance_id.clone(),
                    container_id: None,
                    exclude_relation_categories: vec![],
                    projection,
                },
            )
            .unwrap()
        };
        let card_of = |r: &RecordContextResult, id: &str| -> NeighbourSummary {
            let rel = r
                .relations
                .iter()
                .find(|e| e.relation.target_id == id)
                .expect("edge");
            assert!(rel.relation.source_label.is_none() && rel.relation.target_label.is_none());
            match rel.neighbour.clone().expect("neighbour") {
                ContextInstance::Card(c) => c,
                other => panic!("expected a card, got {other:?}"),
            }
        };

        // full: unchanged — whole records inline, endpoint labels present.
        let full = ctx(ContextProjection::Full);
        assert!(full
            .relations
            .iter()
            .all(|e| matches!(e.neighbour, Some(ContextInstance::Record(_)))
                && e.relation.target_label.is_some()));

        // card: label, type and a truncated summary; the subject itself stays whole.
        let card = ctx(ContextProjection::Card);
        assert_eq!(card.field_values, full.field_values);
        // The card's label is the same display label `full` reports for that endpoint.
        let full_label = |id: &str| {
            full.relations
                .iter()
                .find(|e| e.relation.target_id == id)
                .and_then(|e| e.relation.target_label.clone())
        };
        let c = card_of(&card, &spoke.instance_id);
        assert_eq!(c.label, full_label(&spoke.instance_id));
        assert_eq!(c.type_name.as_deref(), Some("test-type"));
        assert!(c.uri.ends_with(&spoke.instance_id));
        let summary = c.summary.expect("summary");
        assert_eq!(summary.chars().count(), CARD_SUMMARY_CHARS + 1);
        assert!(summary.ends_with('…'));
        assert_eq!(
            card_of(&card, &bare.instance_id).summary.as_deref(),
            Some("Bare")
        );

        // label: identity only.
        let label = ctx(ContextProjection::Label);
        let l = card_of(&label, &spoke.instance_id);
        assert_eq!(l.label, full_label(&spoke.instance_id));
        assert!(l.summary.is_none());

        // Smaller on the wire, same edges.
        let size = |r: &RecordContextResult| serde_json::to_string(r).unwrap().len();
        assert_eq!(card.relations.len(), full.relations.len());
        assert!(size(&label) < size(&card) && size(&card) < size(&full));
    }

    #[test]
    fn record_context_markdown_lists_each_edge_with_label_type_and_id() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        let store = make_store();
        let hub = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("Hub")),
            None,
            None,
        )
        .unwrap();
        let mut fv = make_field_values("test-name", json!("Spoke"));
        fv.insert("test-summary".to_string(), json!("line one\nline two"));
        let spoke =
            record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        create_relation(
            &store,
            Relation {
                created_by: None,
                relation_id: String::new(),
                relation_type: "depends-on".to_string(),
                source_instance_id: spoke.instance_id.clone(),
                target_instance_id: hub.instance_id.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            },
            &depends_on_defs(),
        )
        .unwrap();
        let q = |projection| RecordContextQuery {
            record_id: hub.instance_id.clone(),
            container_id: None,
            exclude_relation_categories: vec![],
            projection,
        };
        // `full` renders as `card`.
        let md = render_record_context_markdown(&store, q(ContextProjection::Full)).unwrap();
        assert_eq!(
            md,
            render_record_context_markdown(&store, q(ContextProjection::Card)).unwrap()
        );
        assert!(md.contains(&format!("`{}`", hub.instance_id)));
        assert!(md.contains("## Relations (1)"));
        assert!(md.contains("### ← depends-on"));
        assert!(md.contains(&format!("(test-type) `{}`", spoke.instance_id)));
        // The summary is one line under its edge.
        assert!(md.contains("  Spoke"));
        let label = render_record_context_markdown(&store, q(ContextProjection::Label)).unwrap();
        assert!(label.contains(&format!("`{}`", spoke.instance_id)));
        assert!(!label.contains("  Spoke"));
    }

    #[test]
    fn record_context_markdown_groups_interleaved_edges_and_uses_renderer_rows() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        let store = make_store();
        let defs = depends_on_defs();
        let mut fv = make_field_values("test-name", json!("Hub"));
        // A block-opening value: the renderer never glues it to the label ([FR-037-3]).
        fv.insert("test-summary".to_string(), json!("- alpha\n- beta"));
        let hub = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();
        let mk = |name: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(name)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let (b, c, d) = (mk("B"), mk("C"), mk("D"));
        // Created out, in, out: the service orders by neighbour createdAt, interleaving them.
        for (src, tgt) in [
            (&hub.instance_id, &b),
            (&c, &hub.instance_id),
            (&hub.instance_id, &d),
        ] {
            create_relation(
                &store,
                Relation {
                    created_by: None,
                    relation_id: String::new(),
                    relation_type: "depends-on".to_string(),
                    source_instance_id: src.clone(),
                    target_instance_id: tgt.clone(),
                    created_at: None,
                    notes: None,
                    source_refs: None,
                    meta: None,
                },
                &defs,
            )
            .unwrap();
        }
        let md = render_record_context_markdown(
            &store,
            RecordContextQuery {
                record_id: hub.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
                projection: ContextProjection::Card,
            },
        )
        .unwrap();
        assert_eq!(md.matches("### → depends-on").count(), 1, "{md}");
        assert_eq!(md.matches("### ← depends-on").count(), 1, "{md}");
        let out_heading = md.find("### → depends-on").unwrap();
        let in_heading = md.find("### ← depends-on").unwrap();
        let pos = |id: &str| md.find(&format!("`{id}`")).unwrap();
        assert!(out_heading < pos(&b) && pos(&b) < in_heading && pos(&d) < in_heading);
        assert!(pos(&c) > in_heading);
        // Field rows come from the renderer: the display label, and a block-opening value
        // on its own lines rather than glued after the colon.
        assert!(md.contains("**Summary**:"), "{md}");
        assert!(md.contains("- alpha") && md.contains("- beta"), "{md}");
        assert!(!md.contains("**Summary**: - alpha"), "{md}");
    }

    #[test]
    fn context_projection_parses_wire_spellings() {
        assert_eq!("card".parse(), Ok(ContextProjection::Card));
        assert_eq!("label".parse(), Ok(ContextProjection::Label));
        assert_eq!("full".parse(), Ok(ContextProjection::Full));
        assert!("compact".parse::<ContextProjection>().is_err());
        let q: RecordContextQuery =
            serde_json::from_str(r#"{"recordId":"r","projection":"card"}"#).unwrap();
        assert_eq!(q.projection, ContextProjection::Card);
        let d: RecordContextQuery = serde_json::from_str(r#"{"recordId":"r"}"#).unwrap();
        assert_eq!(d.projection, ContextProjection::Full);
    }

    #[test]
    fn record_context_excludes_relation_categories() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        use srs_core::types::relation_type_definition::{
            RelationTypeCategory, RelationTypeDefinition,
        };
        let def = |key: &str, category| RelationTypeDefinition {
            schema: None,
            id: format!("rtd-{key}"),
            version: 1,
            key: key.to_string(),
            namespace: "com.test".to_string(),
            label: key.to_string(),
            description: key.to_string(),
            category,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
        };
        let defs = vec![
            def("a-seq", RelationTypeCategory::Sequence),
            def("b-assoc", RelationTypeCategory::Association),
        ];
        let store = make_store_with(defs.clone());
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let (a, b, c) = (mk("A"), mk("B"), mk("C"));
        for (ty, tgt) in [("a-seq", &b), ("b-assoc", &c)] {
            create_relation(
                &store,
                Relation {
                    created_by: None,
                    relation_id: String::new(),
                    relation_type: ty.to_string(),
                    source_instance_id: a.clone(),
                    target_instance_id: tgt.clone(),
                    created_at: None,
                    notes: None,
                    source_refs: None,
                    meta: None,
                },
                &defs,
            )
            .unwrap();
        }
        let ctx = |ex: Vec<RelationTypeCategory>| {
            get_record_context(
                &store,
                RecordContextQuery {
                    record_id: a.clone(),
                    container_id: None,
                    exclude_relation_categories: ex,
                    projection: Default::default(),
                },
            )
            .unwrap()
            .relations
        };
        assert_eq!(ctx(vec![]).len(), 2);
        let kept = ctx(vec![RelationTypeCategory::Sequence]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].relation.relation_type, "b-assoc");
    }

    #[test]
    fn record_context_note_neighbour_dangling_and_order() {
        use crate::relation_service::create_relation;
        use srs_core::types::relation::Relation;
        let store = make_store();
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
        };
        let defs = vec![
            srs_core::types::relation_type_definition::RelationTypeDefinition {
                schema: None,
                id: "rtd-depends-on".to_string(),
                version: 1,
                key: "depends-on".to_string(),
                namespace: "com.test".to_string(),
                label: "depends-on".to_string(),
                description: "d".to_string(),
                category:
                    srs_core::types::relation_type_definition::RelationTypeCategory::Dependency,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                canonical_direction: None,
                inverse_type: None,
                irreflexive: None,
                require_same_type: None,
                status: None,
                updated_at: None,
                meta: None,
            },
        ];
        let para = mk("para");
        let rel = |src: &str, tgt: &str| Relation {
            created_by: None,
            relation_id: String::new(),
            relation_type: "depends-on".to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        };
        // Two inbound edges created second-then-first: the thread must come back by neighbour
        // createdAt (creation order), not by call order or relationId.
        let first = mk("first");
        let second = mk("second");
        for n in [&second, &first] {
            create_relation(&store, rel(&n.instance_id, &para.instance_id), &defs).unwrap();
        }
        let r = get_record_context(
            &store,
            RecordContextQuery {
                record_id: para.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        )
        .unwrap();
        let ids: Vec<_> = r
            .relations
            .iter()
            .map(|e| e.relation.source_id.clone())
            .collect();
        assert_eq!(ids.len(), 2);
        let created = |id: &str| {
            r.relations
                .iter()
                .find(|e| e.relation.source_id == id)
                .and_then(|e| match &e.neighbour {
                    Some(ContextInstance::Record(rec)) => rec.created_at.clone(),
                    _ => None,
                })
        };
        assert!(created(&ids[0]) <= created(&ids[1]));
        assert!(r.relations.iter().all(|e| e.direction == EdgeDirection::In));
    }

    #[test]
    fn record_context_subtree_slice() {
        use srs_core::types::container::{Container, ContainerEntry};
        let store = make_store();
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let (a, b, c, d) = (mk("a"), mk("b"), mk("c"), mk("d"));
        let entry = |id: &String, depth| ContainerEntry {
            instance_id: id.clone(),
            depth,
        };
        // a(0) > b(1) > c(2); d(0)
        let container = Container {
            container_id: String::new(),
            title: "doc".into(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            member_instance_ids: Some(vec![
                entry(&a, None),
                entry(&b, Some(1)),
                entry(&c, Some(2)),
                entry(&d, None),
            ]),
            child_container_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: Default::default(),
        };
        let cid = container_service::create_container(&store, container)
            .unwrap()
            .container_id;
        let ctx = |id: &String| {
            get_record_context(
                &store,
                RecordContextQuery {
                    record_id: id.clone(),
                    container_id: Some(cid.clone()),
                    exclude_relation_categories: vec![],
                    projection: Default::default(),
                },
            )
            .unwrap()
        };
        let ra = ctx(&a);
        let ids = |r: &RecordContextResult| -> Vec<String> {
            r.subtree
                .as_ref()
                .unwrap()
                .iter()
                .map(|e| e.instance_id.clone())
                .collect()
        };
        assert_eq!(ids(&ra), vec![b.clone(), c.clone()]);
        assert_eq!(ra.entry.as_ref().unwrap().instance_id, a);
        assert_eq!(ids(&ctx(&b)), vec![c.clone()]);
        assert!(ids(&ctx(&c)).is_empty());
        assert!(ids(&ctx(&d)).is_empty());
        // A real container that does not contain the record: InvalidInput, not not-found.
        let outsider = mk("outsider");
        let err = get_record_context(
            &store,
            RecordContextQuery {
                record_id: outsider,
                container_id: Some(cid.clone()),
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidInput { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn record_context_non_member_container_errors() {
        let store = make_store();
        let rec = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("x")),
            None,
            None,
        )
        .unwrap();
        let err = get_record_context(
            &store,
            RecordContextQuery {
                record_id: rec.instance_id,
                container_id: Some("no-such-container".into()),
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn record_context_includes_run_history() {
        use crate::protocol_run_service::{create_run, CreateRunInput};
        let store = make_store();

        let fv = make_field_values("test-name", json!("run-history-test"));
        let rec = record_store::create_record(&store, "type-test-001", 1, fv, None, None).unwrap();

        // Create a run targeting this record.
        create_run(
            &store,
            CreateRunInput {
                protocol_id: "proto-ctx".to_string(),
                protocol_version: 1,
                container_id: "c-ctx-run".to_string(),
                target_record_id: Some(rec.instance_id.clone()),
                initial_stage_id: None,
            },
        )
        .unwrap();

        let result = get_record_context(
            &store,
            RecordContextQuery {
                record_id: rec.instance_id.clone(),
                container_id: None,
                exclude_relation_categories: vec![],
                projection: Default::default(),
            },
        )
        .unwrap();

        assert_eq!(result.protocol_run_history.len(), 1);
        let entry = &result.protocol_run_history[0];
        assert_eq!(entry["protocolId"], "proto-ctx");
        assert_eq!(entry["status"], "Active");
    }
    /// A hub with `n_in` inbound `depends-on` edges and one outbound `refines`.
    fn neighbours_fixture(n_in: usize) -> (crate::store::memory::MemoryStore, String, String) {
        use srs_core::types::relation::Relation;
        let store = make_store();
        let mk = |n: &str| {
            record_store::create_record(
                &store,
                "type-test-001",
                1,
                make_field_values("test-name", json!(n)),
                None,
                None,
            )
            .unwrap()
            .instance_id
        };
        let hub = mk("hub");
        let out_target = mk("target");
        let rel = |t: &str, s: &str, d: &str| Relation {
            created_by: None,
            relation_id: String::new(),
            relation_type: t.to_string(),
            source_instance_id: s.to_string(),
            target_instance_id: d.to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        };
        let mut all = vec![rel("refines", &hub, &out_target)];
        for i in 0..n_in {
            all.push(rel("depends-on", &mk(&format!("n{i}")), &hub));
        }
        for r in all {
            // No relation-type definitions needed: write the standalone object directly.
            let mut r = r;
            r.relation_id = uuid::Uuid::new_v4().to_string();
            store.save_relation(&r).unwrap();
        }
        (store, hub, out_target)
    }

    fn nq(id: &str) -> NeighboursQuery {
        NeighboursQuery {
            instance_id: id.to_string(),
            relation_type: None,
            direction: None,
        }
    }

    #[test]
    fn neighbours_pages_with_total() {
        let (store, hub, _) = neighbours_fixture(5);
        let all = list_neighbours(&store, nq(&hub), NeighboursPage::default()).unwrap();
        assert_eq!((all.total, all.neighbours.len()), (6, 6));
        let page = list_neighbours(
            &store,
            nq(&hub),
            NeighboursPage {
                limit: Some(3),
                offset: 2,
            },
        )
        .unwrap();
        assert_eq!((page.total, page.neighbours.len()), (6, 3));
        let ids = |r: &NeighboursResult| {
            r.neighbours
                .iter()
                .map(|e| e.relation_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(&page),
            ids(&all)[2..5].to_vec(),
            "page is a slice of the sorted whole"
        );
        let past = list_neighbours(
            &store,
            nq(&hub),
            NeighboursPage {
                limit: Some(3),
                offset: 99,
            },
        )
        .unwrap();
        assert_eq!((past.total, past.neighbours.len()), (6, 0));
    }

    #[test]
    fn neighbours_filters_direction_and_type() {
        let (store, hub, target) = neighbours_fixture(2);
        let out = list_neighbours(
            &store,
            NeighboursQuery {
                direction: Some(EdgeDirection::Out),
                ..nq(&hub)
            },
            NeighboursPage::default(),
        )
        .unwrap();
        assert_eq!(out.total, 1);
        assert_eq!(out.neighbours[0].neighbour.instance_id, target);
        assert!(out.neighbours[0]
            .neighbour
            .uri
            .ends_with(&format!("/record/{target}")));
        assert!(out.neighbours[0].neighbour.label.is_some());
        assert_eq!(
            out.neighbours[0].neighbour.type_name.as_deref(),
            Some("test-type")
        );
        let deps = list_neighbours(
            &store,
            NeighboursQuery {
                relation_type: Some("depends-on".into()),
                direction: Some(EdgeDirection::In),
                ..nq(&hub)
            },
            NeighboursPage::default(),
        )
        .unwrap();
        assert_eq!(deps.total, 2);
        assert!(deps
            .neighbours
            .iter()
            .all(|e| e.direction == EdgeDirection::In));
    }

    #[test]
    fn neighbours_missing_subject_not_found() {
        let store = make_store();
        let err = list_neighbours(&store, nq("nope"), NeighboursPage::default()).unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }
    #[test]
    fn neighbours_order_is_type_then_created_at_none_last_then_id() {
        use srs_core::types::relation::Relation;
        let (store, hub, _) = neighbours_fixture(0);
        let other = record_store::create_record(
            &store,
            "type-test-001",
            1,
            make_field_values("test-name", json!("o")),
            None,
            None,
        )
        .unwrap()
        .instance_id;
        let rel = |id: &str, t: &str, at: Option<&str>| Relation {
            created_by: None,
            relation_id: id.to_string(),
            relation_type: t.to_string(),
            source_instance_id: other.clone(),
            target_instance_id: hub.clone(),
            created_at: at.map(str::to_string),
            notes: None,
            source_refs: None,
            meta: None,
        };
        for r in [
            rel("00000000-0000-4000-8000-000000000003", "depends-on", None),
            rel(
                "00000000-0000-4000-8000-000000000002",
                "depends-on",
                Some("2026-02-01T00:00:00Z"),
            ),
            rel(
                "00000000-0000-4000-8000-000000000001",
                "depends-on",
                Some("2026-01-01T00:00:00Z"),
            ),
        ] {
            store.save_relation(&r).unwrap();
        }
        let ids: Vec<_> = list_neighbours(&store, nq(&hub), NeighboursPage::default())
            .unwrap()
            .neighbours
            .into_iter()
            .map(|e| (e.relation_type, e.relation_id))
            .collect();
        let pos = |id: &str| ids.iter().position(|(_, r)| r == id).unwrap();
        assert!(
            pos("00000000-0000-4000-8000-000000000001")
                < pos("00000000-0000-4000-8000-000000000002")
                && pos("00000000-0000-4000-8000-000000000002")
                    < pos("00000000-0000-4000-8000-000000000003"),
            "{ids:?}"
        );
        assert_eq!(
            ids[0].0, "depends-on",
            "depends-on sorts before refines: {ids:?}"
        );
    }

    #[test]
    fn neighbours_note_subject_and_note_neighbour() {
        use srs_core::types::relation::Relation;
        let (store, hub, _) = neighbours_fixture(0);
        let note = crate::services::create_note(
            &store,
            serde_json::from_value(json!({ "title": "A note", "sections": [] })).unwrap(),
        )
        .unwrap()
        .note
        .instance_id;
        store
            .save_relation(&Relation {
                created_by: None,
                relation_id: "00000000-0000-4000-8000-0000000000aa".into(),
                relation_type: "depends-on".into(),
                source_instance_id: note.clone(),
                target_instance_id: hub.clone(),
                created_at: None,
                notes: None,
                source_refs: None,
                meta: None,
            })
            .unwrap();
        let from_note = list_neighbours(&store, nq(&note), NeighboursPage::default()).unwrap();
        assert_eq!(from_note.total, 1);
        assert_eq!(from_note.neighbours[0].neighbour.instance_id, hub);
        let from_hub = list_neighbours(&store, nq(&hub), NeighboursPage::default()).unwrap();
        let n = from_hub
            .neighbours
            .iter()
            .find(|e| e.neighbour.instance_id == note)
            .unwrap();
        assert_eq!(n.neighbour.label.as_deref(), Some("A note"));
        assert!(n.neighbour.type_name.is_none());
    }
}
