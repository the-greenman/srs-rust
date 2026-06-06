use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetMode {
    Local,
    Remote,
    Inline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetType {
    Image,
    Font,
    Stylesheet,
    Data,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDeclaration {
    #[serde(rename = "type")]
    pub asset_type: AssetType,
    pub mode: AssetMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionWrapperOverride {
    pub section_id: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordWrapperOverride {
    pub type_id: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementTemplates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_wrapper: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_wrapper: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section_wrapper_overrides: Option<Vec<SectionWrapperOverride>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_wrapper: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_wrapper_overrides: Option<Vec<RecordWrapperOverride>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_row: Option<String>,
    /// Per-field-name templates for individual field rows in group entries. [T-Gx1]
    /// Key: Field.name. Value: template string with `{{field-value}}` and `{{field-label}}`.
    /// Applies only when the group has no known compositeRenderer (or falls back to baseline).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_field_row_templates: Option<HashMap<String, String>>,
    /// Per-renderer config, keyed by the same identifier space as FieldGroup.compositeRenderer.
    /// "table" renderer reads: tableClass, wrapperTemplate, captionTemplate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub composite_renderer_config: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    #[serde(default)]
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub description: String,
    pub targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assets: Option<HashMap<String, AssetDeclaration>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub css_class_fields: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_templates: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_templates: Option<ElementTemplates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stylesheet: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography: Option<serde_json::Value>,
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
    fn theme_roundtrips_minimal_json() {
        let theme = Theme {
            id: "00000000-0000-4000-8000-000000000901".to_string(),
            namespace: "fixture.theme".to_string(),
            name: "minimal".to_string(),
            version: 1,
            description: "Minimal theme".to_string(),
            targets: vec!["markdown".to_string()],
            assets: None,
            css_class_fields: None,
            page_templates: None,
            element_templates: None,
            stylesheet: None,
            typography: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        };

        let json = serde_json::to_string(&theme).expect("serialize");
        let roundtrip: Theme = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(theme, roundtrip);
    }

    #[test]
    fn theme_roundtrips_full_element_templates() {
        let mut extra = HashMap::new();
        extra.insert("xCustom".to_string(), serde_json::json!(true));

        let theme = Theme {
            id: "00000000-0000-4000-8000-000000000902".to_string(),
            namespace: "fixture.theme".to_string(),
            name: "full".to_string(),
            version: 1,
            description: "Full theme".to_string(),
            targets: vec!["markdown".to_string(), "text".to_string()],
            assets: Some(HashMap::from([(
                "logo".to_string(),
                AssetDeclaration {
                    asset_type: AssetType::Image,
                    mode: AssetMode::Inline,
                    path: None,
                    url: None,
                    data: Some("Zm9v".to_string()),
                    mime_type: Some("image/png".to_string()),
                },
            )])),
            css_class_fields: Some(vec!["00000000-0000-4000-8000-000000000903".to_string()]),
            page_templates: Some(serde_json::json!({"coverPage": "{{container-title}}"})),
            element_templates: Some(ElementTemplates {
                document_wrapper: Some("<main>{{content}}</main>".to_string()),
                section_wrapper: Some("<section>{{content}}</section>".to_string()),
                section_wrapper_overrides: Some(vec![SectionWrapperOverride {
                    section_id: "decision-log".to_string(),
                    template: "<section class=\"decision\">{{content}}</section>".to_string(),
                }]),
                record_wrapper: Some("<article>{{content}}</article>".to_string()),
                record_wrapper_overrides: Some(vec![RecordWrapperOverride {
                    type_id: "00000000-0000-4000-8000-000000000904".to_string(),
                    template: "<article class=\"special\">{{content}}</article>".to_string(),
                }]),
                field_row: Some("<p>{{field-label}}: {{field-value}}</p>".to_string()),
                group_field_row_templates: None,
                composite_renderer_config: None,
            }),
            stylesheet: Some(serde_json::json!({"mode": "inline", "content": "body{}"})),
            typography: Some(serde_json::json!({"baseFont": "Georgia"})),
            tags: Some(vec!["theme".to_string()]),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra,
        };

        let json = serde_json::to_string(&theme).expect("serialize");
        let roundtrip: Theme = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(theme, roundtrip);
    }

    #[test]
    fn theme_deserializes_unknown_top_level_fields_silently() {
        let json = r#"
        {
          "id": "00000000-0000-4000-8000-000000000905",
          "namespace": "fixture.theme",
          "name": "unknown-fields",
          "version": 1,
          "description": "Theme with extra fields",
          "targets": ["markdown"],
          "createdAt": "2026-01-01T00:00:00Z",
          "xCustom": "keep"
        }
        "#;

        let theme: Theme = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            theme.extra.get("xCustom").and_then(|v| v.as_str()),
            Some("keep")
        );
    }
}
