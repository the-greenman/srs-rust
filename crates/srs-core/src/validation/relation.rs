use crate::types::relation::Relation;
use crate::types::relation_type_definition::{RelationTypeDefinition, RelationTypeStatus};
use std::collections::{HashMap, HashSet};

/// Context required to validate a relation against the effective installed definitions.
pub struct RelationValidationContext<'a> {
    /// All installed relation type definitions in the effective package set.
    pub definitions: &'a [RelationTypeDefinition],
    /// All known instance IDs in the repository. Used for E2 endpoint checks.
    pub known_instance_ids: &'a HashSet<String>,
    /// Maps instanceId → semanticObjectType for instances that carry the field.
    /// If an instance is absent from this map, E4 type-constraint checks are skipped for it.
    pub instance_semantic_types: &'a HashMap<String, String>,
}

/// A single relation validation error.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationValidationError {
    pub relation_id: String,
    pub code: RelationValidationCode,
    pub message: String,
}

/// The error code classifying which invariant was violated.
#[derive(Debug, Clone, PartialEq)]
pub enum RelationValidationCode {
    /// E1: relation type could not be resolved.
    E1UnknownRelationType,
    /// E1: relation type is retired — does not resolve even for reads.
    E1RetiredRelationType,
    /// E1: relation type is deprecated/tombstone and a write was attempted.
    E1WriteRejected,
    /// E1: two definitions share the same relationType but differ in id/version/content.
    E1Conflict,
    /// E2: source or target instance ID is not known in the repository.
    E2UnknownEndpoint,
    /// E3: irreflexive constraint — source and target are the same instance.
    E3Irreflexive,
    /// E4: source or target semanticObjectType violates allowedSourceTypes/allowedTargetTypes.
    E4TypeConstraint,
}

/// Validate a single relation against the context.
///
/// Returns `Ok(())` if all checks pass, or `Err(Vec<RelationValidationError>)` listing
/// every violated invariant.
pub fn validate_relation(
    relation: &Relation,
    ctx: &RelationValidationContext,
    is_write: bool,
) -> Result<(), Vec<RelationValidationError>> {
    let mut errors: Vec<RelationValidationError> = Vec::new();

    // E1 — resolve relationType
    let definition = resolve_definition(relation, ctx, is_write, &mut errors);

    // E2 — endpoints must exist in known_instance_ids
    if !ctx
        .known_instance_ids
        .contains(&relation.source_instance_id)
    {
        errors.push(RelationValidationError {
            relation_id: relation.relation_id.clone(),
            code: RelationValidationCode::E2UnknownEndpoint,
            message: format!(
                "E2: source instance '{}' not found in repository",
                relation.source_instance_id
            ),
        });
    }
    if !ctx
        .known_instance_ids
        .contains(&relation.target_instance_id)
    {
        errors.push(RelationValidationError {
            relation_id: relation.relation_id.clone(),
            code: RelationValidationCode::E2UnknownEndpoint,
            message: format!(
                "E2: target instance '{}' not found in repository",
                relation.target_instance_id
            ),
        });
    }

    // E3 + E4 only apply when we have a resolved definition
    if let Some(def) = definition {
        // E3 — irreflexive
        if def.is_irreflexive() && relation.source_instance_id == relation.target_instance_id {
            errors.push(RelationValidationError {
                relation_id: relation.relation_id.clone(),
                code: RelationValidationCode::E3Irreflexive,
                message: format!(
                    "E3: relation type '{}' is irreflexive but source == target ('{}') ",
                    relation.relation_type, relation.source_instance_id
                ),
            });
        }

        // E4 — semantic object type constraints
        validate_e4(relation, def, ctx, &mut errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Resolves the relationType against the definition list, emitting E1 errors as needed.
/// Returns the resolved definition if it was found and usable.
fn resolve_definition<'a>(
    relation: &Relation,
    ctx: &RelationValidationContext<'a>,
    is_write: bool,
    errors: &mut Vec<RelationValidationError>,
) -> Option<&'a RelationTypeDefinition> {
    let matching: Vec<&RelationTypeDefinition> = ctx
        .definitions
        .iter()
        .filter(|d| d.relation_type == relation.relation_type)
        .collect();

    match matching.len() {
        0 => {
            errors.push(RelationValidationError {
                relation_id: relation.relation_id.clone(),
                code: RelationValidationCode::E1UnknownRelationType,
                message: format!(
                    "E1: relation type '{}' is not installed in the package",
                    relation.relation_type
                ),
            });
            None
        }
        1 => {
            let def = matching[0];
            // Retired never resolves
            if matches!(def.status, Some(RelationTypeStatus::Retired)) {
                errors.push(RelationValidationError {
                    relation_id: relation.relation_id.clone(),
                    code: RelationValidationCode::E1RetiredRelationType,
                    message: format!(
                        "E1: relation type '{}' is retired and does not resolve",
                        relation.relation_type
                    ),
                });
                return None;
            }
            // Deprecated/tombstone reject writes
            if is_write {
                let rejects_write = matches!(
                    def.status,
                    Some(RelationTypeStatus::Deprecated) | Some(RelationTypeStatus::Tombstone)
                );
                if rejects_write {
                    let status_str = match &def.status {
                        Some(RelationTypeStatus::Deprecated) => "deprecated",
                        Some(RelationTypeStatus::Tombstone) => "tombstone",
                        _ => "unknown",
                    };
                    errors.push(RelationValidationError {
                        relation_id: relation.relation_id.clone(),
                        code: RelationValidationCode::E1WriteRejected,
                        message: format!(
                            "E1: relation type '{}' is {} — new writes are rejected",
                            relation.relation_type, status_str
                        ),
                    });
                    return None;
                }
            }
            Some(def)
        }
        _ => {
            // Multiple definitions with same relationType — check if they are identical (coalesce)
            // or truly conflicting.
            let first = matching[0];
            let all_identical = matching.iter().all(|d| {
                d.id == first.id && d.version == first.version && d.namespace == first.namespace
            });
            if all_identical {
                // Coalesce: treat as a single canonical definition
                Some(first)
            } else {
                errors.push(RelationValidationError {
                    relation_id: relation.relation_id.clone(),
                    code: RelationValidationCode::E1Conflict,
                    message: format!(
                        "E1: relation type '{}' has conflicting definitions (different id/version/content)",
                        relation.relation_type
                    ),
                });
                None
            }
        }
    }
}

fn validate_e4(
    relation: &Relation,
    def: &RelationTypeDefinition,
    ctx: &RelationValidationContext,
    errors: &mut Vec<RelationValidationError>,
) {
    // allowedSourceTypes
    if let Some(allowed) = &def.allowed_source_types {
        if let Some(src_type) = ctx
            .instance_semantic_types
            .get(&relation.source_instance_id)
        {
            if !allowed.contains(src_type) {
                errors.push(RelationValidationError {
                    relation_id: relation.relation_id.clone(),
                    code: RelationValidationCode::E4TypeConstraint,
                    message: format!(
                        "E4: source instance '{}' has semanticObjectType '{}' which is not in allowedSourceTypes {:?} for relation type '{}'",
                        relation.source_instance_id, src_type, allowed, relation.relation_type
                    ),
                });
            }
        }
    }

    // allowedTargetTypes
    if let Some(allowed) = &def.allowed_target_types {
        if let Some(tgt_type) = ctx
            .instance_semantic_types
            .get(&relation.target_instance_id)
        {
            if !allowed.contains(tgt_type) {
                errors.push(RelationValidationError {
                    relation_id: relation.relation_id.clone(),
                    code: RelationValidationCode::E4TypeConstraint,
                    message: format!(
                        "E4: target instance '{}' has semanticObjectType '{}' which is not in allowedTargetTypes {:?} for relation type '{}'",
                        relation.target_instance_id, tgt_type, allowed, relation.relation_type
                    ),
                });
            }
        }
    }

    // requireSameSemanticObjectType
    if def.require_same_semantic_object_type.unwrap_or(false) {
        if let (Some(src_type), Some(tgt_type)) = (
            ctx.instance_semantic_types
                .get(&relation.source_instance_id),
            ctx.instance_semantic_types
                .get(&relation.target_instance_id),
        ) {
            if src_type != tgt_type {
                errors.push(RelationValidationError {
                    relation_id: relation.relation_id.clone(),
                    code: RelationValidationCode::E4TypeConstraint,
                    message: format!(
                        "E4: relation type '{}' requires same semanticObjectType but source is '{}' and target is '{}'",
                        relation.relation_type, src_type, tgt_type
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::relation_type_definition::RelationTypeCategory;

    fn make_def(relation_type: &str) -> RelationTypeDefinition {
        RelationTypeDefinition {
            schema: None,
            id: "f7a8b9c0-d1e2-4f3a-8b4c-5d6e7f8a9b0c".to_string(),
            version: 1,
            relation_type: relation_type.to_string(),
            namespace: "com.semanticops.srs".to_string(),
            label: "Test".to_string(),
            description: "desc".to_string(),
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

    fn make_relation(rel_type: &str, src: &str, tgt: &str) -> Relation {
        Relation {
            relation_id: "r0000001-0000-4000-a000-000000000001".to_string(),
            relation_type: rel_type.to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            asserted_by: None,
            confidence: None,
            created_at: Some("2026-05-29T00:00:00Z".to_string()),
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        }
    }

    fn ctx_with_defs<'a>(
        defs: &'a [RelationTypeDefinition],
        ids: &'a HashSet<String>,
        types: &'a HashMap<String, String>,
    ) -> RelationValidationContext<'a> {
        RelationValidationContext {
            definitions: defs,
            known_instance_ids: ids,
            instance_semantic_types: types,
        }
    }

    #[test]
    fn e1_missing_definition_is_error() {
        let defs = vec![];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("unknown-type", "src", "tgt");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E1UnknownRelationType));
    }

    #[test]
    fn e1_retired_definition_is_error() {
        let defs = vec![RelationTypeDefinition {
            status: Some(RelationTypeStatus::Retired),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E1RetiredRelationType));
    }

    #[test]
    fn e1_deprecated_write_is_error() {
        let defs = vec![RelationTypeDefinition {
            status: Some(RelationTypeStatus::Deprecated),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, true).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E1WriteRejected));
    }

    #[test]
    fn e1_deprecated_read_is_ok() {
        let defs = vec![RelationTypeDefinition {
            status: Some(RelationTypeStatus::Deprecated),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        assert!(validate_relation(&r, &ctx, false).is_ok());
    }

    #[test]
    fn e1_tombstone_write_is_error() {
        let defs = vec![RelationTypeDefinition {
            status: Some(RelationTypeStatus::Tombstone),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, true).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E1WriteRejected));
    }

    #[test]
    fn e1_tombstone_read_is_ok() {
        let defs = vec![RelationTypeDefinition {
            status: Some(RelationTypeStatus::Tombstone),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        assert!(validate_relation(&r, &ctx, false).is_ok());
    }

    #[test]
    fn e1_conflict_same_relation_type_different_id() {
        let def_a = make_def("precedes");
        let def_b = RelationTypeDefinition {
            id: "aaaaaaaa-0000-4000-a000-000000000000".to_string(),
            ..make_def("precedes")
        };
        let defs = vec![def_a, def_b];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E1Conflict));
    }

    #[test]
    fn e1_coalesce_identical_definitions() {
        let def = make_def("precedes");
        let defs = vec![def.clone(), def];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        assert!(validate_relation(&r, &ctx, false).is_ok());
    }

    #[test]
    fn e2_unknown_source_endpoint_is_error() {
        let defs = vec![make_def("precedes")];
        let ids: HashSet<String> = ["tgt".to_string()].into(); // src missing
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E2UnknownEndpoint));
    }

    #[test]
    fn e3_irreflexive_self_relation_is_error() {
        let defs = vec![RelationTypeDefinition {
            irreflexive: Some(true),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["self-id".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "self-id", "self-id");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E3Irreflexive));
    }

    #[test]
    fn e3_irreflexive_false_self_relation_is_ok() {
        let defs = vec![RelationTypeDefinition {
            irreflexive: Some(false),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["self-id".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "self-id", "self-id");
        assert!(validate_relation(&r, &ctx, false).is_ok());
    }

    #[test]
    fn e4_allowed_source_type_rejected() {
        let defs = vec![RelationTypeDefinition {
            allowed_source_types: Some(vec!["section".to_string()]),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types: HashMap<String, String> = [("src".to_string(), "note".to_string())].into();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E4TypeConstraint));
    }

    #[test]
    fn e4_require_same_type_mismatch() {
        let defs = vec![RelationTypeDefinition {
            require_same_semantic_object_type: Some(true),
            ..make_def("precedes")
        }];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types: HashMap<String, String> = [
            ("src".to_string(), "section".to_string()),
            ("tgt".to_string(), "note".to_string()),
        ]
        .into();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        let err = validate_relation(&r, &ctx, false).unwrap_err();
        assert!(err
            .iter()
            .any(|e| e.code == RelationValidationCode::E4TypeConstraint));
    }

    #[test]
    fn valid_relation_passes_all_checks() {
        let defs = vec![make_def("precedes")];
        let ids: HashSet<String> = ["src".to_string(), "tgt".to_string()].into();
        let types = HashMap::new();
        let ctx = ctx_with_defs(&defs, &ids, &types);
        let r = make_relation("precedes", "src", "tgt");
        assert!(validate_relation(&r, &ctx, false).is_ok());
    }
}
