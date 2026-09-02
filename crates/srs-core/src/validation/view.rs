use crate::error::CoreError;
use crate::types::view::{Composition, View, ViewRow};
use std::collections::HashSet;

pub fn validate_view(view: &View) -> Result<(), CoreError> {
    if view.field_views.is_empty() {
        return Err(CoreError::EmptyViewFieldViews);
    }

    let mut seen_field_ids = HashSet::new();
    for row in &view.field_views {
        if let ViewRow::Field(field_view) = row {
            if !seen_field_ids.insert(&field_view.field_id) {
                return Err(CoreError::DuplicateFieldViewId {
                    field_id: field_view.field_id.clone(),
                });
            }
        }
    }

    // RFC-041 [R7]: FieldView and RecordPropertyView rows share one ordering
    // axis; a duplicate `order` anywhere in the mixed list is a validation
    // error (mirrors `srs`'s `scripts/validate-package.mjs` package-validation
    // check — JSON Schema's `uniqueItems` cannot express uniqueness by a
    // single property across two differently-shaped object kinds).
    let mut seen_orders = HashSet::new();
    for row in &view.field_views {
        let order = row.order();
        if !seen_orders.insert(order) {
            return Err(CoreError::DuplicateViewRowOrder { order });
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

pub fn validate_composition(dv: &Composition) -> Result<(), CoreError> {
    if dv.sections.is_empty() {
        return Err(CoreError::EmptyCompositionSections);
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
        Composition, DocumentSection, FieldView, RecordProperty, RecordPropertyView, SectionSource,
        ThemeMode, ThemeReference, ThemeVariant, View,
    };

    fn minimal_view() -> View {
        View {
            schema: None,
            ai_guidance: None,
            lineage: None,
            provenance: None,
            updated_at: None,
            id: "view-1".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "test-view".to_string(),
            version: 1,
            description: "desc".to_string(),
            field_views: vec![FieldView {
                display_hint: None,
                editor_hint_override: None,
                field_id: "f1".to_string(),
                order: 0,
                required: None,
                visible: None,
                display_label: None,
                composite_renderer: None,
            }
            .into()],
            compatible_types: None,
            protection: None,
            export_config: None,
            tags: None,
            created_at: "2026-05-29T00:00:00Z".to_string(),
        }
    }

    fn minimal_composition() -> Composition {
        Composition {
            schema: None,
            ai_guidance: None,
            lineage: None,
            provenance: None,
            updated_at: None,
            composite_renderers: None,
            id: "dv-1".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            name: "test-doc-view".to_string(),
            version: 1,
            description: "desc".to_string(),
            container_type: None,
            root_type_refs: None,
            sections: vec![DocumentSection {
                composite_renderers: None,
                section_id: "s1".to_string(),
                title: None,
                description: None,
                order: 0,
                source: SectionSource::FixedInstances {
                    instance_ids: vec!["a".to_string()],
                },
                render_view_id: None,
                type_dispatch: None,
                title_field_id: None,
                ordering: None,
                required: None,
                empty_behavior: None,
                relations_presentation: None,
            }],
            navigation_links: None,
            preamble: None,
            format: None,
            depth_offset: None,
            theme_ref: None,
            theme_variants: None,
            tags: None,
            created_at: "2026-05-29T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn validate_empty_sections_fails() {
        let mut dv = minimal_composition();
        dv.sections = vec![];
        assert_eq!(
            validate_composition(&dv),
            Err(CoreError::EmptyCompositionSections)
        );
    }

    #[test]
    fn validate_duplicate_section_id_fails() {
        let mut dv = minimal_composition();
        dv.sections.push(DocumentSection {
            composite_renderers: None,
            section_id: "s1".to_string(),
            title: None,
            description: None,
            order: 1,
            source: SectionSource::FixedInstances {
                instance_ids: vec!["b".to_string()],
            },
            render_view_id: None,
            type_dispatch: None,
            title_field_id: None,
            ordering: None,
            required: None,
            empty_behavior: None,
            relations_presentation: None,
        });

        assert_eq!(
            validate_composition(&dv),
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
    fn validate_duplicate_field_view_id_still_fails_with_mixed_rows() {
        let mut view = minimal_view();
        view.field_views.insert(
            0,
            RecordPropertyView {
                property: RecordProperty::LifecycleState,
                order: 5,
                display_label: None,
                visible: None,
            }
            .into(),
        );
        let duplicate = view.field_views[1].as_field().unwrap().clone();
        view.field_views.push(
            FieldView {
                order: 6,
                ..duplicate
            }
            .into(),
        );

        assert_eq!(
            validate_view(&view),
            Err(CoreError::DuplicateFieldViewId {
                field_id: "f1".to_string()
            })
        );
    }

    /// RFC-041 [R7]: order must be unique across the whole mixed row list,
    /// not just within one row kind.
    #[test]
    fn validate_duplicate_row_order_fails_across_row_kinds() {
        let mut view = minimal_view();
        view.field_views.push(
            RecordPropertyView {
                property: RecordProperty::Tags,
                order: 0, // duplicates the FieldView row's order in minimal_view()
                display_label: None,
                visible: None,
            }
            .into(),
        );

        assert_eq!(
            validate_view(&view),
            Err(CoreError::DuplicateViewRowOrder { order: 0 })
        );
    }

    #[test]
    fn validate_distinct_mixed_rows_passes() {
        let mut view = minimal_view();
        view.field_views.push(
            RecordPropertyView {
                property: RecordProperty::Tags,
                order: 1,
                display_label: Some("Labels".to_string()),
                visible: Some(true),
            }
            .into(),
        );

        assert!(validate_view(&view).is_ok());
    }

    #[test]
    fn validate_duplicate_theme_variant_name_fails() {
        let mut dv = minimal_composition();
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
            validate_composition(&dv),
            Err(CoreError::DuplicateThemeVariantName {
                name: "print".to_string()
            })
        );
    }

    #[test]
    fn validate_unique_theme_variant_names_passes() {
        let mut dv = minimal_composition();
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
        assert!(validate_composition(&dv).is_ok());
    }
}
