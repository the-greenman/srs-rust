use crate::types::blueprint::Blueprint;
use crate::types::blueprint::BlueprintValidationResult;
use std::collections::HashSet;

pub fn validate_blueprint(blueprint: &Blueprint) -> BlueprintValidationResult {
    let mut errors: Vec<String> = vec![];

    if blueprint.version < 1 {
        errors.push("Blueprint version must be >= 1".to_string());
    }

    if blueprint.id.trim().is_empty() {
        errors.push("Blueprint id must not be empty".to_string());
    }

    if blueprint.namespace.trim().is_empty() {
        errors.push("Blueprint namespace must not be empty".to_string());
    }

    if blueprint.name.trim().is_empty() {
        errors.push("Blueprint name must not be empty".to_string());
    }

    if blueprint.root_types.is_empty() {
        errors.push("Blueprint root_types must not be empty".to_string());
    }

    // Validate TypeRef.type_version: Some(0) is not allowed.
    for tr in blueprint
        .root_types
        .iter()
        .chain(
            blueprint
                .structure
                .iter()
                .flat_map(|rs| [&rs.source_type, &rs.target_type]),
        )
        .chain(blueprint.required_types.iter())
    {
        if tr.type_version == Some(0) {
            errors.push(format!(
                "TypeRef '{}' has type_version 0; version must be >= 1 when specified",
                tr.type_id
            ));
        }
    }

    // Build the full type universe: root_types ∪ structure source/target types.
    let mut universe: HashSet<&str> = HashSet::new();
    for tr in &blueprint.root_types {
        universe.insert(tr.type_id.as_str());
    }
    for rs in &blueprint.structure {
        universe.insert(rs.source_type.type_id.as_str());
        universe.insert(rs.target_type.type_id.as_str());
    }

    // Every required_type must appear in the universe.
    for rt in &blueprint.required_types {
        if !universe.contains(rt.type_id.as_str()) {
            errors.push(format!(
                "required_type '{}' does not appear in root_types or structure source/target types",
                rt.type_id
            ));
        }
    }

    if errors.is_empty() {
        BlueprintValidationResult::ok()
    } else {
        BlueprintValidationResult::with_errors(errors)
    }
}
