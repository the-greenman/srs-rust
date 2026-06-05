use crate::error::CoreError;
use crate::types::theme::Theme;
use std::collections::HashSet;

pub fn validate_theme(theme: &Theme) -> Result<(), CoreError> {
    if theme.targets.is_empty() {
        return Err(CoreError::ThemeTargetsEmpty);
    }

    if let Some(element_templates) = &theme.element_templates {
        if let Some(overrides) = &element_templates.section_wrapper_overrides {
            let mut seen = HashSet::new();
            for override_ in overrides {
                if !seen.insert(&override_.section_id) {
                    return Err(CoreError::DuplicateThemeSectionOverrideId {
                        section_id: override_.section_id.clone(),
                    });
                }
            }
        }

        if let Some(overrides) = &element_templates.record_wrapper_overrides {
            let mut seen = HashSet::new();
            for override_ in overrides {
                if !seen.insert(&override_.type_id) {
                    return Err(CoreError::DuplicateThemeRecordOverrideTypeId {
                        type_id: override_.type_id.clone(),
                    });
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::theme::{
        AssetDeclaration, AssetMode, AssetType, ElementTemplates, RecordWrapperOverride,
        SectionWrapperOverride, Theme,
    };
    use std::collections::HashMap;

    fn minimal_theme() -> Theme {
        Theme {
            id: "00000000-0000-4000-8000-000000000910".to_string(),
            namespace: "fixture.theme".to_string(),
            name: "test-theme".to_string(),
            version: 1,
            description: "desc".to_string(),
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
        }
    }

    #[test]
    fn validate_theme_empty_targets_fails() {
        let mut theme = minimal_theme();
        theme.targets = vec![];
        assert_eq!(validate_theme(&theme), Err(CoreError::ThemeTargetsEmpty));
    }

    #[test]
    fn validate_theme_single_target_passes() {
        assert!(validate_theme(&minimal_theme()).is_ok());
    }

    #[test]
    fn validate_theme_duplicate_section_override_id_fails() {
        let mut theme = minimal_theme();
        theme.element_templates = Some(ElementTemplates {
            document_wrapper: None,
            section_wrapper: None,
            section_wrapper_overrides: Some(vec![
                SectionWrapperOverride {
                    section_id: "s1".to_string(),
                    template: "a".to_string(),
                },
                SectionWrapperOverride {
                    section_id: "s1".to_string(),
                    template: "b".to_string(),
                },
            ]),
            record_wrapper: None,
            record_wrapper_overrides: None,
            field_row: None,
            group_field_templates: None,
        });

        assert_eq!(
            validate_theme(&theme),
            Err(CoreError::DuplicateThemeSectionOverrideId {
                section_id: "s1".to_string()
            })
        );
    }

    #[test]
    fn validate_theme_unique_section_override_ids_passes() {
        let mut theme = minimal_theme();
        theme.element_templates = Some(ElementTemplates {
            document_wrapper: None,
            section_wrapper: None,
            section_wrapper_overrides: Some(vec![
                SectionWrapperOverride {
                    section_id: "s1".to_string(),
                    template: "a".to_string(),
                },
                SectionWrapperOverride {
                    section_id: "s2".to_string(),
                    template: "b".to_string(),
                },
            ]),
            record_wrapper: None,
            record_wrapper_overrides: None,
            field_row: None,
            group_field_templates: None,
        });

        assert!(validate_theme(&theme).is_ok());
    }

    #[test]
    fn validate_theme_duplicate_record_override_type_id_fails() {
        let mut theme = minimal_theme();
        theme.element_templates = Some(ElementTemplates {
            document_wrapper: None,
            section_wrapper: None,
            section_wrapper_overrides: None,
            record_wrapper: None,
            record_wrapper_overrides: Some(vec![
                RecordWrapperOverride {
                    type_id: "00000000-0000-4000-8000-000000000911".to_string(),
                    template: "a".to_string(),
                },
                RecordWrapperOverride {
                    type_id: "00000000-0000-4000-8000-000000000911".to_string(),
                    template: "b".to_string(),
                },
            ]),
            field_row: None,
            group_field_templates: None,
        });

        assert_eq!(
            validate_theme(&theme),
            Err(CoreError::DuplicateThemeRecordOverrideTypeId {
                type_id: "00000000-0000-4000-8000-000000000911".to_string()
            })
        );
    }

    #[test]
    fn validate_theme_full_theme_with_assets_passes() {
        let mut theme = minimal_theme();
        theme.assets = Some(HashMap::from([(
            "logo".to_string(),
            AssetDeclaration {
                asset_type: AssetType::Image,
                mode: AssetMode::Inline,
                path: None,
                url: None,
                data: Some("Zm9v".to_string()),
                mime_type: Some("image/png".to_string()),
            },
        )]));

        assert!(validate_theme(&theme).is_ok());
    }
}
