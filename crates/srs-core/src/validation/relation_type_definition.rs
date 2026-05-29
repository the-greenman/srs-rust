use crate::error::CoreError;
use crate::types::relation_type_definition::RelationTypeDefinition;

/// Validates a `RelationTypeDefinition`.
///
/// Checks:
/// - `id` is non-empty
/// - `relation_type` is non-empty
/// - `namespace` is non-empty
/// - `label` is non-empty
/// - `version` is at least 1
/// - `created_at` is non-empty
/// - Namespaced `relationType` values have the form `namespace/name` where both parts are non-empty
/// - `allowed_source_types` entries are non-empty (when present)
/// - `allowed_target_types` entries are non-empty (when present)
pub fn validate_relation_type_definition(rtd: &RelationTypeDefinition) -> Result<(), CoreError> {
    if rtd.id.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "id".to_string(),
        });
    }

    if rtd.relation_type.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "relationType".to_string(),
        });
    }

    if rtd.namespace.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "namespace".to_string(),
        });
    }

    if rtd.label.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "label".to_string(),
        });
    }

    if rtd.version < 1 {
        return Err(CoreError::MissingRequiredField {
            field_id: "version".to_string(),
        });
    }

    if rtd.created_at.is_empty() {
        return Err(CoreError::MissingRequiredField {
            field_id: "createdAt".to_string(),
        });
    }

    // If relationType contains '/', validate namespace/name form
    if rtd.relation_type.contains('/') {
        let parts: Vec<&str> = rtd.relation_type.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(CoreError::InvalidRelationType {
                relation_type: rtd.relation_type.clone(),
            });
        }
    }

    // Validate allowedSourceTypes entries are non-empty
    if let Some(src_types) = &rtd.allowed_source_types {
        for t in src_types {
            if t.is_empty() {
                return Err(CoreError::InvalidRelationType {
                    relation_type: rtd.relation_type.clone(),
                });
            }
        }
    }

    // Validate allowedTargetTypes entries are non-empty
    if let Some(tgt_types) = &rtd.allowed_target_types {
        for t in tgt_types {
            if t.is_empty() {
                return Err(CoreError::InvalidRelationType {
                    relation_type: rtd.relation_type.clone(),
                });
            }
        }
    }

    Ok(())
}

/// Returns true if a relation with this type can be written (E1 write gate).
pub fn accepts_writes(rtd: &RelationTypeDefinition) -> bool {
    rtd.accepts_writes()
}

/// Returns true if a relation with this type resolves for reads (E1 read gate).
pub fn resolves(rtd: &RelationTypeDefinition) -> bool {
    rtd.resolves()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::relation_type_definition::{RelationTypeCategory, RelationTypeStatus};

    fn make_rtd(relation_type: &str) -> RelationTypeDefinition {
        RelationTypeDefinition {
            schema: None,
            id: "f7a8b9c0-d1e2-4f3a-8b4c-5d6e7f8a9b0c".to_string(),
            version: 1,
            relation_type: relation_type.to_string(),
            namespace: "com.semanticops.srs".to_string(),
            label: "Test".to_string(),
            description: "Test relation type.".to_string(),
            category: RelationTypeCategory::Sequence,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            canonical_direction: None,
            inverse_type: None,
            irreflexive: None,
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: None,
            updated_at: None,
        }
    }

    #[test]
    fn valid_canonical_passes() {
        assert!(validate_relation_type_definition(&make_rtd("precedes")).is_ok());
    }

    #[test]
    fn valid_namespaced_passes() {
        assert!(validate_relation_type_definition(&make_rtd(
            "com.semanticops.spec/rfc-change-sequence"
        ))
        .is_ok());
    }

    #[test]
    fn empty_relation_type_fails() {
        let result = validate_relation_type_definition(&make_rtd(""));
        assert!(matches!(
            result.unwrap_err(),
            CoreError::MissingRequiredField { field_id } if field_id == "relationType"
        ));
    }

    #[test]
    fn empty_id_fails() {
        let rtd = RelationTypeDefinition {
            id: "".to_string(),
            ..make_rtd("precedes")
        };
        let result = validate_relation_type_definition(&rtd);
        assert!(matches!(
            result.unwrap_err(),
            CoreError::MissingRequiredField { field_id } if field_id == "id"
        ));
    }

    #[test]
    fn empty_label_fails() {
        let rtd = RelationTypeDefinition {
            label: "".to_string(),
            ..make_rtd("precedes")
        };
        let result = validate_relation_type_definition(&rtd);
        assert!(matches!(
            result.unwrap_err(),
            CoreError::MissingRequiredField { field_id } if field_id == "label"
        ));
    }

    #[test]
    fn namespaced_with_empty_name_part_fails() {
        let result = validate_relation_type_definition(&make_rtd("com.example/"));
        assert!(matches!(
            result.unwrap_err(),
            CoreError::InvalidRelationType { .. }
        ));
    }

    #[test]
    fn namespaced_with_empty_namespace_part_fails() {
        let result = validate_relation_type_definition(&make_rtd("/my-type"));
        assert!(matches!(
            result.unwrap_err(),
            CoreError::InvalidRelationType { .. }
        ));
    }

    #[test]
    fn namespaced_with_multiple_slashes_fails() {
        let result = validate_relation_type_definition(&make_rtd("bad/format/extra"));
        assert!(matches!(
            result.unwrap_err(),
            CoreError::InvalidRelationType { .. }
        ));
    }

    #[test]
    fn empty_allowed_source_type_fails() {
        let rtd = RelationTypeDefinition {
            allowed_source_types: Some(vec!["section".to_string(), "".to_string()]),
            ..make_rtd("precedes")
        };
        let result = validate_relation_type_definition(&rtd);
        assert!(matches!(
            result.unwrap_err(),
            CoreError::InvalidRelationType { .. }
        ));
    }

    #[test]
    fn valid_allowed_types_passes() {
        let rtd = RelationTypeDefinition {
            allowed_source_types: Some(vec!["section".to_string()]),
            allowed_target_types: Some(vec!["section".to_string(), "subsection".to_string()]),
            ..make_rtd("precedes")
        };
        assert!(validate_relation_type_definition(&rtd).is_ok());
    }

    #[test]
    fn accepts_writes_active() {
        let rtd = make_rtd("precedes");
        assert!(accepts_writes(&rtd));
    }

    #[test]
    fn accepts_writes_deprecated_false() {
        let rtd = RelationTypeDefinition {
            status: Some(RelationTypeStatus::Deprecated),
            ..make_rtd("precedes")
        };
        assert!(!accepts_writes(&rtd));
    }

    #[test]
    fn resolves_retired_false() {
        let rtd = RelationTypeDefinition {
            status: Some(RelationTypeStatus::Retired),
            ..make_rtd("precedes")
        };
        assert!(!resolves(&rtd));
    }
}
