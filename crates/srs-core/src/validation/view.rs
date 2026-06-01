use crate::error::CoreError;
use crate::types::view::{DocumentView, View};
use std::collections::HashSet;

pub fn validate_view(view: &View) -> Result<(), CoreError> {
    if view.field_views.is_empty() {
        return Err(CoreError::EmptyViewFieldViews);
    }

    let mut seen_field_ids = HashSet::new();
    for field_view in &view.field_views {
        if !seen_field_ids.insert(&field_view.field_id) {
            return Err(CoreError::DuplicateFieldViewId {
                field_id: field_view.field_id.clone(),
            });
        }
    }

    if let Some(tags) = &view.tags {
        for tag in tags {
            if tag.is_empty() {
                return Err(CoreError::EmptyTag);
            }
        }
    }

    Ok(())
}

pub fn validate_document_view(dv: &DocumentView) -> Result<(), CoreError> {
    if dv.sections.is_empty() {
        return Err(CoreError::EmptyDocumentViewSections);
    }

    let mut seen_section_ids = HashSet::new();
    for section in &dv.sections {
        if !seen_section_ids.insert(&section.section_id) {
            return Err(CoreError::DuplicateDocumentSectionId {
                section_id: section.section_id.clone(),
            });
        }
    }

    if let Some(variants) = &dv.theme_variants {
        let mut seen_variant_names = HashSet::new();
        for variant in variants {
            if !seen_variant_names.insert(&variant.name) {
                return Err(CoreError::DuplicateThemeVariantName {
                    name: variant.name.clone(),
                });
            }
        }
    }

    if let Some(tags) = &dv.tags {
        for tag in tags {
            if tag.is_empty() {
                return Err(CoreError::EmptyTag);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::view::{
        DocumentSection, DocumentView, FieldView, SectionSource, ThemeMode, ThemeReference,
        ThemeVariant, View,
    };
    use std::collections::HashMap;

    fn minimal_view() -> View {
        View {
            id: "view-1".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "test-view".to_string(),
            version: 1,
            description: "desc".to_string(),
            field_views: vec![FieldView {
                field_id: "f1".to_string(),
                order: 0,
                required: None,
                visible: None,
                display_label: None,
            }],
            compatible_types: None,
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn minimal_document_view() -> DocumentView {
        DocumentView {
            id: "dv-1".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "test-doc-view".to_string(),
            version: 1,
            description: "desc".to_string(),
            container_type: None,
            sections: vec![DocumentSection {
                section_id: "s1".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::FixedInstances {
                    instance_ids: vec!["a".to_string()],
                },
                render_view_id: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
            }],
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn validate_empty_sections_fails() {
        let mut dv = minimal_document_view();
        dv.sections = vec![];
        assert_eq!(
            validate_document_view(&dv),
            Err(CoreError::EmptyDocumentViewSections)
        );
    }

    #[test]
    fn validate_duplicate_section_id_fails() {
        let mut dv = minimal_document_view();
        dv.sections.push(DocumentSection {
            section_id: "s1".to_string(),
            title: None,
            description: None,
            order: 1,
            source: SectionSource::FixedInstances {
                instance_ids: vec!["b".to_string()],
            },
            render_view_id: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
        });

        assert_eq!(
            validate_document_view(&dv),
            Err(CoreError::DuplicateDocumentSectionId {
                section_id: "s1".to_string()
            })
        );
    }

    #[test]
    fn validate_empty_field_views_fails() {
        let mut view = minimal_view();
        view.field_views = vec![];
        assert_eq!(validate_view(&view), Err(CoreError::EmptyViewFieldViews));
    }

    #[test]
    fn validate_duplicate_theme_variant_name_fails() {
        let mut dv = minimal_document_view();
        let v = ThemeVariant {
            name: "print".to_string(),
            description: None,
            theme_ref: ThemeReference {
                mode: ThemeMode::Local,
                path: Some("./theme".to_string()),
                url: None,
                theme_id: None,
            },
        };
        dv.theme_variants = Some(vec![v.clone(), v]);

        assert_eq!(
            validate_document_view(&dv),
            Err(CoreError::DuplicateThemeVariantName {
                name: "print".to_string()
            })
        );
    }

    #[test]
    fn validate_unique_theme_variant_names_passes() {
        let mut dv = minimal_document_view();
        dv.theme_variants = Some(vec![
            ThemeVariant {
                name: "print".to_string(),
                description: None,
                theme_ref: ThemeReference {
                    mode: ThemeMode::Local,
                    path: Some("./theme-print".to_string()),
                    url: None,
                    theme_id: None,
                },
            },
            ThemeVariant {
                name: "dark".to_string(),
                description: None,
                theme_ref: ThemeReference {
                    mode: ThemeMode::Remote,
                    path: None,
                    url: Some("https://example.com/theme-dark".to_string()),
                    theme_id: None,
                },
            },
        ]);
        assert!(validate_document_view(&dv).is_ok());
    }
}
