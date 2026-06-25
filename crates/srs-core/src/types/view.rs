use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldView {
    pub field_id: String,
    pub order: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preamble: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_order: Option<Vec<String>>,
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
#[serde(rename_all = "camelCase")]
pub struct View {
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub field_views: Vec<FieldView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compatible_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection: Option<ViewProtection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_config: Option<ExportConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SectionSource {
    #[serde(rename_all = "camelCase")]
    FixedInstances { instance_ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    TypeQuery {
        semantic_object_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle_state: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        container_ids: Option<Vec<String>>,
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
#[serde(rename_all = "camelCase")]
pub struct SectionOrdering {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<SortDirection>,
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
#[serde(rename_all = "camelCase")]
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
    pub type_dispatch: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_field_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordering: Option<SectionOrdering>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_behavior: Option<EmptyBehavior>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct ThemeVariant {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub theme_ref: ThemeReference,
}

/// A version-exact reference to a Type, used in `DocumentView.root_type_refs` (RFC-009).
///
/// Distinct from the blueprint-level [`crate::types::blueprint::TypeRef`], where
/// `type_version` is optional. `ExactTypeRef` requires `type_version` because it is a
/// package-validation-time anchor (RFC-009 I-63): each entry must resolve to a specific
/// Type version in the package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExactTypeRef {
    pub type_id: String,
    pub type_version: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    /// RFC-009: version-exact Type anchors. When present and non-empty, this DocumentView
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub created_at: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_view_roundtrips_json() {
        let mut extra = HashMap::new();
        extra.insert("xCustom".to_string(), serde_json::json!("keep"));
        let dv = DocumentView {
            id: "ec34f54b-8636-5c8b-af5b-c9eb3df24fe6".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "srs-spec-document-view".to_string(),
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
                    semantic_object_type: "com.semanticops.srs/meta.section".to_string(),
                    lifecycle_state: Some("active".to_string()),
                    container_ids: Some(vec!["c1".to_string()]),
                },
                render_view_id: Some("view-1".to_string()),
                type_dispatch: None,
                title_field_id: Some("field-title".to_string()),
                ordering: Some(SectionOrdering {
                    field_id: Some("field-order".to_string()),
                    direction: Some(SortDirection::Asc),
                }),
                required: Some(true),
                empty_behavior: Some(EmptyBehavior::Hide),
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
            extra,
        };

        let json = serde_json::to_string(&dv).unwrap();
        assert!(
            json.contains("\"rootTypeRefs\""),
            "rootTypeRefs must serialize with camelCase key"
        );
        let parsed: DocumentView = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, dv);
        assert_eq!(
            parsed.root_type_refs.as_ref().unwrap()[0].type_version,
            2,
            "ExactTypeRef.typeVersion must survive the roundtrip"
        );
    }

    #[test]
    fn section_source_type_query_deserialises() {
        let json = r#"{"type":"type-query","semanticObjectType":"com.example/decision"}"#;
        let parsed: SectionSource = serde_json::from_str(json).unwrap();
        assert_eq!(
            parsed,
            SectionSource::TypeQuery {
                semantic_object_type: "com.example/decision".to_string(),
                lifecycle_state: None,
                container_ids: None
            }
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
    fn document_section_type_dispatch_round_trips() {
        let mut dispatch = std::collections::HashMap::new();
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
