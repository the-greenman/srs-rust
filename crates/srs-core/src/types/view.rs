use serde::{Deserialize, Serialize};

/// RFC-036 Change A (ext:views-l1) — a view-owned composite rendering dispatch
/// record. Presentation only ([CR-036-20]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompositeRendererBinding {
    /// Renderer identifier. Bare lower-kebab names are SRS-reserved (`table`,
    /// and the sentinel `baseline` meaning explicitly no renderer); vendor
    /// identifiers use `{reverse-domain}/{name}` ([CR-036-1]).
    pub renderer: String,
    /// Explicit role → Field.id bindings overriding the by-name defaults of
    /// [CR-036-8]. Role names are renderer-defined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<std::collections::BTreeMap<String, String>>,
}

/// RFC-036 Change B (ext:views-l2) — a CompositeRendererBinding plus the
/// composite-range Field it binds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompositeRendererDirective {
    pub field_id: String,
    pub renderer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldView {
    /// Declared by `view.json` — carried so a load/write round trip keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor_hint_override: Option<serde_json::Value>,
    pub field_id: String,
    pub order: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
    /// RFC-036 — render this field's composite-range value through a named
    /// composite renderer. Highest-precedence declaration site ([CR-036-6]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_renderer: Option<CompositeRendererBinding>,
}

/// RFC-041 Change B — the closed, DERIVED vocabulary of top-level Record
/// properties a `RecordPropertyView` row may present. Regenerated (never
/// hand-edited) from `record.json` by `srs`'s
/// `scripts/gen-record-property-view-enum.mjs` — this Rust enum mirrors that
/// generated `view.json` enum, so it must be kept in step with it by hand
/// until the metamodel-generation epic (#272) subsumes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordProperty {
    LifecycleState,
    Tags,
    CreatedAt,
    UpdatedAt,
}

impl RecordProperty {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LifecycleState => "lifecycleState",
            Self::Tags => "tags",
            Self::CreatedAt => "createdAt",
            Self::UpdatedAt => "updatedAt",
        }
    }

    /// RFC-041 Change C — the per-property default row label, used when the
    /// row carries no `displayLabel` override. Fixed table inside one general
    /// mechanism, not per-instance special-casing ([R3]).
    pub const fn default_label(self) -> &'static str {
        match self {
            Self::LifecycleState => "Status",
            Self::Tags => "Tags",
            Self::CreatedAt => "Created",
            Self::UpdatedAt => "Updated",
        }
    }
}

impl std::fmt::Display for RecordProperty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// RFC-041 Change A — presentation of a top-level Record property, a sibling
/// row kind of [`FieldView`] in `View.fieldViews[]`. Deliberately carries no
/// `required`, `editorHintOverride`, or `compositeRenderer` — a record-level
/// property is never a Field a Type can require and has no composite-range
/// value to dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordPropertyView {
    pub property: RecordProperty,
    pub order: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
}

/// RFC-041 [R1] — a row in `View.fieldViews[]`. The two sibling kinds are
/// discriminated by which of two mutually exclusive required keys the JSON
/// object carries: `fieldId` (`FieldView`) XOR `property` (`RecordPropertyView`).
/// `#[serde(untagged)]` over two `deny_unknown_fields` variants enforces the
/// XOR structurally: a row naming both or neither key matches no variant and
/// is rejected at deserialize time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViewRow {
    Field(FieldView),
    RecordProperty(RecordPropertyView),
}

impl ViewRow {
    pub fn order(&self) -> i32 {
        match self {
            Self::Field(row) => row.order,
            Self::RecordProperty(row) => row.order,
        }
    }

    pub fn visible(&self) -> bool {
        match self {
            Self::Field(row) => row.visible.unwrap_or(true),
            Self::RecordProperty(row) => row.visible.unwrap_or(true),
        }
    }

    pub fn display_label(&self) -> Option<&str> {
        match self {
            Self::Field(row) => row.display_label.as_deref(),
            Self::RecordProperty(row) => row.display_label.as_deref(),
        }
    }

    pub fn as_field(&self) -> Option<&FieldView> {
        match self {
            Self::Field(row) => Some(row),
            Self::RecordProperty(_) => None,
        }
    }

    pub fn as_record_property(&self) -> Option<&RecordPropertyView> {
        match self {
            Self::RecordProperty(row) => Some(row),
            Self::Field(_) => None,
        }
    }
}

impl From<FieldView> for ViewRow {
    fn from(value: FieldView) -> Self {
        Self::Field(value)
    }
}

impl From<RecordPropertyView> for ViewRow {
    fn from(value: RecordPropertyView) -> Self {
        Self::RecordProperty(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub omit_empty_fields: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViewProtection {
    None,
    ReadOnly,
    FillIn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct View {
    /// The `$schema` pointer the file may carry — declared by the schema itself,
    /// preserved so a loaded-then-written definition keeps it.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub field_views: Vec<ViewRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<ViewProtection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_config: Option<ExportConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    /// Authoring guidance for the View (`view.json` `aiGuidance`) — carried,
    /// not interpreted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    /// RFC-014 provenance/lineage — declared by the schema; carried so a
    /// load/write round trip keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerScope {
    Explicit,
    Repository,
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SectionSource {
    #[serde(rename_all = "camelCase")]
    FixedInstances { instance_ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    TypeQuery {
        /// KEYED `namespace/name` (version-independent) resolved against the
        /// effective package set — the same convention as `ContainerSubset.type_filter`
        /// and `DocumentSection.type_dispatch`. srs-rust#910: renamed from the
        /// retired `semanticObjectType` (owner ruling on #383, srs#372/#481/#524,
        /// `rfc-decision-c8704763`) — the resolution behind this field was already
        /// real Type-keyed selection (`list_records_by_type(namespace, name)`),
        /// never the dead E4 `semanticObjectType` string; only the name was wrong.
        type_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle_state: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        container_ids: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle_states: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exclude_lifecycle_states: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        container_scope: Option<ContainerScope>,
    },
    #[serde(rename_all = "camelCase")]
    RelationQuery {
        from_instance_id: String,
        relation_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        direction: Option<RelationDirection>,
    },
    #[serde(rename_all = "camelCase")]
    ContainerSubset {
        container_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        container_type: Option<String>,
        /// RFC-008 (ext:views-l2). When present and non-empty, restricts the section to container
        /// members whose resolved type (namespace/name, version-independent) matches one of these
        /// keys. Ordering is computed over the full container then projected onto survivors.
        #[serde(skip_serializing_if = "Option::is_none")]
        type_filter: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationDirection {
    Forward,
    Inverse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SectionOrdering {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<SortDirection>,
    /// RFC-015 [N+29] — the view-owned explicit presentation sequence for a
    /// `container-subset` section: `instanceId`s in presentation order, with
    /// unlisted members appended in [N+12] order. Declared by
    /// `composition.json`, so a strict `SectionOrdering` has to model it or a
    /// schema-valid Composition would stop loading. Carried, not yet consumed
    /// — honouring it is srs-rust#567.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_order: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmptyBehavior {
    Hide,
    ShowPlaceholder,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresentationDirection {
    Forward,
    Inverse,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationPresentationEntry {
    pub relation_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directions: Option<PresentationDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inverse_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationsPresentation {
    pub include: Vec<RelationPresentationEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    // Reserved per RFC-027; ignored at render time.
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentSection {
    pub section_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub order: i32,
    pub source: SectionSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_view_id: Option<String>,
    /// RFC-008 (ext:views-l2). Map from resolved type key (namespace/name, version-independent)
    /// to the ext:views-l1 View UUID for rendering records of that type within this section.
    /// Consulted before renderViewId; unmatched types fall back to renderViewId then baseline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_dispatch: Option<std::collections::BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordering: Option<SectionOrdering>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_behavior: Option<EmptyBehavior>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relations_presentation: Option<RelationsPresentation>,
    /// RFC-036 — composite renderer dispatch for records rendered by this
    /// section; primary ext:views-l2 declaration site ([CR-036-6]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_renderers: Option<Vec<CompositeRendererDirective>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavigationLink {
    pub from_section_id: String,
    pub to_section_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bidirectional: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Local,
    Remote,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeReference {
    pub mode: ThemeMode,
    /// Relative path to the theme directory, as declared in the view document (mode: "local" only).
    /// This is a stored configuration value. srs-core never opens this path.
    /// Any code that resolves this to a real file must live in srs-repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeVariant {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub theme_ref: ThemeReference,
}

/// A version-exact reference to a Type, used in `Composition.root_type_refs` (RFC-009).
///
/// Distinct from the blueprint-level [`crate::types::blueprint::TypeRef`], where
/// `type_version` is optional. `ExactTypeRef` requires `type_version` because it is a
/// package-validation-time anchor (RFC-009 I-63): each entry must resolve to a specific
/// Type version in the package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExactTypeRef {
    pub type_id: String,
    pub type_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Composition {
    /// The `$schema` pointer the file may carry — declared by the schema itself,
    /// preserved so a loaded-then-written definition keeps it.
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    /// RFC-009: version-exact Type anchors. When present and non-empty, this Composition
    /// applies to Containers whose root Record resolves to one of these Types (OR semantics).
    /// Replaces `container_type` as the load-bearing join; `container_type` is a back-compat hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_type_refs: Option<Vec<ExactTypeRef>>,
    pub sections: Vec<DocumentSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub navigation_links: Option<Vec<NavigationLink>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_ref: Option<ThemeReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_variants: Option<Vec<ThemeVariant>>,
    /// RFC-036 — document-wide default composite renderer dispatch, applied to
    /// sections with no matching entry; lowest precedence ([CR-036-6]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_renderers: Option<Vec<CompositeRendererDirective>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    /// Authoring guidance for the Composition (`composition.json`
    /// `aiGuidance`) — carried, not interpreted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_guidance: Option<serde_json::Value>,
    /// RFC-014 provenance/lineage — declared by the schema; carried so a
    /// load/write round trip keeps it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lineage: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn view_row_round_trips_both_kinds() {
        let field_json = serde_json::json!({
            "fieldId": "00000000-0000-4000-8000-000000000001",
            "order": 0,
            "displayLabel": "Title"
        });
        let property_json = serde_json::json!({
            "property": "lifecycleState",
            "order": 1,
            "displayLabel": "Status",
            "visible": false
        });

        let field: ViewRow = serde_json::from_value(field_json.clone()).unwrap();
        let property: ViewRow = serde_json::from_value(property_json.clone()).unwrap();

        assert!(matches!(field, ViewRow::Field(_)));
        assert!(matches!(property, ViewRow::RecordProperty(_)));
        assert_eq!(serde_json::to_value(field).unwrap(), field_json);
        assert_eq!(serde_json::to_value(property).unwrap(), property_json);
    }

    /// RFC-041 [R1]: `fieldId` XOR `property`. Neither key, or both, is rejected.
    #[test]
    fn view_row_rejects_both_or_neither_discriminator() {
        let both = serde_json::json!({
            "fieldId": "00000000-0000-4000-8000-000000000001",
            "property": "tags",
            "order": 0
        });
        let neither = serde_json::json!({ "order": 0 });

        assert!(serde_json::from_value::<ViewRow>(both).is_err());
        assert!(serde_json::from_value::<ViewRow>(neither).is_err());
    }

    /// RFC-041 [R2]: the enum is closed — an unrecognized `property` is rejected.
    #[test]
    fn record_property_is_closed_and_uses_schema_names() {
        for (json_name, property, label) in [
            ("lifecycleState", RecordProperty::LifecycleState, "Status"),
            ("tags", RecordProperty::Tags, "Tags"),
            ("createdAt", RecordProperty::CreatedAt, "Created"),
            ("updatedAt", RecordProperty::UpdatedAt, "Updated"),
        ] {
            assert_eq!(
                serde_json::from_value::<RecordProperty>(serde_json::json!(json_name)).unwrap(),
                property
            );
            assert_eq!(property.as_str(), json_name);
            assert_eq!(property.default_label(), label);
        }
        assert!(serde_json::from_value::<RecordProperty>(serde_json::json!("sourceRefs")).is_err());
    }

    #[test]
    fn view_row_helpers_share_common_row_behavior() {
        let row = ViewRow::from(RecordPropertyView {
            property: RecordProperty::Tags,
            order: 3,
            display_label: Some("Topics".to_string()),
            visible: None,
        });

        assert_eq!(row.order(), 3);
        assert!(row.visible());
        assert_eq!(row.display_label(), Some("Topics"));
        assert!(row.as_field().is_none());
        assert_eq!(
            row.as_record_property().map(|view| view.property),
            Some(RecordProperty::Tags)
        );
    }

    #[test]
    fn composition_roundtrips_json() {
        let dv = Composition {
            schema: None,
            ai_guidance: None,
            lineage: None,
            provenance: None,
            updated_at: None,
            composite_renderers: None,
            id: "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "srs-spec-composition".to_string(),
            version: 1,
            description: "desc".to_string(),
            container_type: Some("spec".to_string()),
            root_type_refs: Some(vec![ExactTypeRef {
                type_id: "11111111-1111-4111-8111-111111111111".to_string(),
                type_version: 2,
            }]),
            sections: vec![DocumentSection {
                section_id: "spec-sections".to_string(),
                title: Some("Specification".to_string()),
                description: Some("full spec".to_string()),
                order: 0,
                source: SectionSource::TypeQuery {
                    type_key: "com.semanticops.srs/meta.section".to_string(),
                    lifecycle_state: Some("active".to_string()),
                    container_ids: Some(vec!["c1".to_string()]),
                    lifecycle_states: None,
                    exclude_lifecycle_states: None,
                    container_scope: None,
                },
                render_view_id: Some("view-1".to_string()),
                type_dispatch: None,
                title_field_id: Some("field-title".to_string()),
                ordering: Some(SectionOrdering {
                    member_order: None,
                    field_id: Some("field-order".to_string()),
                    direction: Some(SortDirection::Asc),
                }),
                required: Some(true),
                empty_behavior: Some(EmptyBehavior::Hide),
                relations_presentation: None,
                composite_renderers: None,
            }],
            navigation_links: Some(vec![NavigationLink {
                from_section_id: "a".to_string(),
                to_section_id: "b".to_string(),
                label: Some("next".to_string()),
                bidirectional: Some(false),
            }]),
            preamble: Some("{{heading-1-open}}{{container-title}}{{heading-1-close}}".to_string()),
            format: Some("markdown".to_string()),
            depth_offset: Some(1),
            theme_ref: Some(ThemeReference {
                mode: ThemeMode::Bundled,
                path: Some("themes/default".to_string()),
                url: None,
                theme_id: Some("default".to_string()),
            }),
            theme_variants: Some(vec![ThemeVariant {
                name: "print".to_string(),
                description: Some("printer-friendly".to_string()),
                theme_ref: ThemeReference {
                    mode: ThemeMode::Local,
                    path: Some("./themes/print".to_string()),
                    url: None,
                    theme_id: None,
                },
            }]),
            tags: Some(vec!["spec".to_string()]),
            created_at: "2026-05-29T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&dv).unwrap();
        assert!(
            json.contains("\"rootTypeRefs\""),
            "rootTypeRefs must serialize with camelCase key"
        );
        let parsed: Composition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, dv);
        assert_eq!(
            parsed.root_type_refs.as_ref().unwrap()[0].type_version,
            2,
            "ExactTypeRef.typeVersion must survive the roundtrip"
        );
    }

    #[test]
    fn section_source_type_query_deserialises() {
        let json = r#"{"type":"type-query","typeKey":"com.example/decision"}"#;
        let parsed: SectionSource = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            SectionSource::TypeQuery {
                type_key: "com.example/decision".to_string(),
                lifecycle_state: None,
                container_ids: None,
                lifecycle_states: None,
                exclude_lifecycle_states: None,
                container_scope: None,
            }
        );
    }

    #[test]
    fn section_source_type_query_new_fields_round_trip() {
        // All three new fields present — should round-trip correctly.
        let source = SectionSource::TypeQuery {
            type_key: "com.example/decision".to_string(),
            lifecycle_state: None,
            container_ids: None,
            lifecycle_states: Some(vec!["active".to_string(), "draft".to_string()]),
            exclude_lifecycle_states: Some(vec!["superseded".to_string()]),
            container_scope: Some(ContainerScope::Repository),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(
            json.contains("\"lifecycleStates\""),
            "lifecycleStates must serialise as camelCase: {json}"
        );
        assert!(
            json.contains("\"excludeLifecycleStates\""),
            "excludeLifecycleStates must serialise as camelCase: {json}"
        );
        assert!(
            json.contains("\"containerScope\""),
            "containerScope must serialise as camelCase: {json}"
        );
        assert!(
            json.contains("\"repository\""),
            "ContainerScope::Repository must serialise as 'repository': {json}"
        );
        let parsed: SectionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, source);
    }

    #[test]
    fn section_source_type_query_new_fields_absent_round_trip() {
        // When new fields are absent, they must not appear in serialised JSON and must deserialise to None.
        let source = SectionSource::TypeQuery {
            type_key: "com.example/decision".to_string(),
            lifecycle_state: None,
            container_ids: None,
            lifecycle_states: None,
            exclude_lifecycle_states: None,
            container_scope: None,
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(
            !json.contains("lifecycleStates"),
            "absent lifecycleStates must not appear in JSON: {json}"
        );
        assert!(
            !json.contains("excludeLifecycleStates"),
            "absent excludeLifecycleStates must not appear in JSON: {json}"
        );
        assert!(
            !json.contains("containerScope"),
            "absent containerScope must not appear in JSON: {json}"
        );
        let parsed: SectionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, source);
    }

    #[test]
    fn container_scope_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&ContainerScope::Explicit).unwrap(),
            "\"explicit\""
        );
        assert_eq!(
            serde_json::to_string(&ContainerScope::Repository).unwrap(),
            "\"repository\""
        );
        assert_eq!(
            serde_json::to_string(&ContainerScope::Subtree).unwrap(),
            "\"subtree\""
        );
    }

    #[test]
    fn section_source_fixed_instances_deserialises() {
        let json = r#"{"type":"fixed-instances","instanceIds":["a","b"]}"#;
        let parsed: SectionSource = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            SectionSource::FixedInstances {
                instance_ids: vec!["a".to_string(), "b".to_string()]
            }
        );
    }

    #[test]
    fn section_source_relation_query_defaults_forward() {
        let json = r#"{"type":"relation-query","fromInstanceId":"r1","relationType":"precedes"}"#;
        let parsed: SectionSource = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            SectionSource::RelationQuery {
                from_instance_id: "r1".to_string(),
                relation_type: "precedes".to_string(),
                direction: None
            }
        );
    }

    #[test]
    fn container_subset_type_filter_round_trips() {
        let source = SectionSource::ContainerSubset {
            container_id: "cid-1".to_string(),
            container_type: None,
            type_filter: Some(vec!["ns/name".to_string(), "ns/other".to_string()]),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(
            json.contains("\"typeFilter\""),
            "typeFilter must serialize as camelCase: {json}"
        );
        assert!(
            json.contains("\"ns/name\""),
            "typeFilter values must be preserved: {json}"
        );
        let parsed: SectionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, source);
    }

    #[test]
    fn container_subset_no_type_filter_omitted_from_json() {
        let source = SectionSource::ContainerSubset {
            container_id: "cid-1".to_string(),
            container_type: None,
            type_filter: None,
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(
            !json.contains("typeFilter"),
            "typeFilter: None must be omitted from JSON: {json}"
        );
    }

    #[test]
    fn document_section_relations_presentation_round_trips() {
        let section = DocumentSection {
            composite_renderers: None,
            section_id: "s1".to_string(),
            title: None,
            description: None,
            order: 0,
            source: SectionSource::FixedInstances {
                instance_ids: vec![],
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
            relations_presentation: Some(RelationsPresentation {
                include: vec![
                    RelationPresentationEntry {
                        relation_type: "supersedes".to_string(),
                        directions: Some(PresentationDirection::Forward),
                        forward_label: Some("Supersedes".to_string()),
                        inverse_label: None,
                    },
                    RelationPresentationEntry {
                        relation_type: "com.example/depends-on".to_string(),
                        directions: Some(PresentationDirection::Both),
                        forward_label: None,
                        inverse_label: Some("Required by".to_string()),
                    },
                ],
                label: None,
            }),
        };
        let json = serde_json::to_string(&section).unwrap();
        assert!(
            json.contains("\"relationsPresentation\""),
            "relationsPresentation must serialize as camelCase: {json}"
        );
        assert!(
            json.contains("\"relationType\""),
            "relationType must serialize as camelCase: {json}"
        );
        assert!(
            json.contains("\"forwardLabel\""),
            "forwardLabel must serialize as camelCase: {json}"
        );
        assert!(
            json.contains("\"inverseLabel\""),
            "inverseLabel must serialize as camelCase: {json}"
        );
        assert!(
            json.contains("\"both\""),
            "PresentationDirection::Both must serialize as \"both\": {json}"
        );
        let parsed: DocumentSection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, section);
    }

    #[test]
    fn document_section_relations_presentation_absent_omitted() {
        let section = DocumentSection {
            section_id: "s2".to_string(),
            title: None,
            description: None,
            order: 0,
            source: SectionSource::FixedInstances {
                instance_ids: vec![],
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
            relations_presentation: None,
            composite_renderers: None,
        };
        let json = serde_json::to_string(&section).unwrap();
        assert!(
            !json.contains("relationsPresentation"),
            "absent relationsPresentation must not appear in JSON: {json}"
        );
        let parsed: DocumentSection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.relations_presentation, None);
    }

    #[test]
    fn document_section_type_dispatch_round_trips() {
        let mut dispatch = BTreeMap::new();
        dispatch.insert("ns/name".to_string(), "view-uuid-1".to_string());
        dispatch.insert("ns/other".to_string(), "view-uuid-2".to_string());
        let section = DocumentSection {
            section_id: "s".to_string(),
            title: None,
            description: None,
            order: 0,
            source: SectionSource::FixedInstances {
                instance_ids: vec![],
            },
            render_view_id: None,
            type_dispatch: Some(dispatch.clone()),
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
            relations_presentation: None,
            composite_renderers: None,
        };
        let json = serde_json::to_string(&section).unwrap();
        assert!(
            json.contains("\"typeDispatch\""),
            "typeDispatch must serialize as camelCase: {json}"
        );
        let parsed: DocumentSection = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.type_dispatch, Some(dispatch));
    }
}
