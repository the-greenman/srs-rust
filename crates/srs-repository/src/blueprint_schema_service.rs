//! `blueprint schema` composition service.
//!
//! Resolves a Blueprint and emits a **nested draft-07 JSON Schema** describing the
//! entire multi-record document it declares. The schema contains:
//!
//! - `definitions`: one sub-schema per member Type, projected via
//!   [`type_schema_service::type_schema`] — no duplication of the field→widget
//!   mapping logic.
//! - `root`: a `$ref` (single root type) or `oneOf` (multiple root types) pointing
//!   into `definitions`.
//! - One child-array property per distinct `relationType` in `blueprint.structure`,
//!   keyed by the lowerCamelCase conversion of the relation type string (ADR-014).
//!
//! Non-fatal projection problems (unresolvable TypeRef, unparseable cardinality)
//! are collected into [`BlueprintSchemaResult::diagnostics`] as plain strings.
//! A hard error (blueprint not found) is returned as [`RepositoryError`].

use crate::blueprint_service::{get_blueprint_by_id, GetBlueprintResult};
use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::type_schema_service::{type_schema, TypeSchemaInput};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

/// Input contract for [`blueprint_schema`].
#[derive(Debug, Clone)]
pub struct BlueprintSchemaInput {
    pub blueprint_id: String,
}

/// Output contract for [`blueprint_schema`].
#[derive(Debug, Clone)]
pub struct BlueprintSchemaResult {
    /// The generated nested draft-07 JSON Schema object.
    pub schema: Value,
    /// Non-fatal projection problems (unresolvable types, unparseable cardinality,
    /// per-type field warnings). Surfaced by the CLI in the payload `diagnostics`
    /// field.
    pub diagnostics: Vec<String>,
}

/// Project a Blueprint into a nested draft-07 JSON Schema for a whole document.
///
/// Returns `Err(RepositoryError::BlueprintNotFound)` when the Blueprint cannot be
/// resolved. All other failures are non-fatal and reported in `result.diagnostics`.
pub fn blueprint_schema(
    store: &dyn RepositoryStore,
    input: BlueprintSchemaInput,
) -> Result<BlueprintSchemaResult, RepositoryError> {
    let blueprint_id = input.blueprint_id;

    let blueprint = match get_blueprint_by_id(store, &blueprint_id)? {
        GetBlueprintResult::Found(bp) => *bp,
        GetBlueprintResult::NotFound => {
            return Err(RepositoryError::BlueprintNotFound { blueprint_id })
        }
    };

    let mut diagnostics: Vec<String> = Vec::new();

    // Collect unique TypeRefs to project: root_types + structure[].target_type only.
    // source_type TypeRefs are deliberately excluded — source-only types produce
    // unreachable definitions entries with no $ref pointing to them.
    let mut type_ids_ordered: Vec<(String, Option<u32>)> = Vec::new();
    let mut seen_type_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut add_type_ref = |type_id: &str, type_version: Option<u32>| {
        if seen_type_ids.insert(type_id.to_string()) {
            type_ids_ordered.push((type_id.to_string(), type_version));
        }
    };

    for tr in &blueprint.root_types {
        add_type_ref(&tr.type_id, tr.type_version);
    }
    for spec in &blueprint.structure {
        add_type_ref(&spec.target_type.type_id, spec.target_type.type_version);
    }

    // Project each unique TypeRef into a definitions sub-schema.
    let mut definitions: BTreeMap<String, Value> = BTreeMap::new();
    for (type_id, type_version) in &type_ids_ordered {
        match type_schema(
            store,
            TypeSchemaInput {
                type_id: type_id.clone(),
                type_version: *type_version,
            },
        ) {
            Ok(result) => {
                for d in result.diagnostics {
                    diagnostics.push(format!("{type_id}: {d}"));
                }
                definitions.insert(type_id.clone(), result.schema);
            }
            Err(e) => {
                diagnostics.push(format!(
                    "type '{type_id}' could not be projected: {e}; omitted from definitions"
                ));
            }
        }
    }

    // Build the `root` property.
    let root_prop = match blueprint.root_types.as_slice() {
        [] => json!({ "description": "no root types declared" }),
        [single] => json!({ "$ref": format!("#/definitions/{}", single.type_id) }),
        multiple => {
            let refs: Vec<Value> = multiple
                .iter()
                .map(|tr| json!({ "$ref": format!("#/definitions/{}", tr.type_id) }))
                .collect();
            json!({ "oneOf": refs })
        }
    };

    // Group structure RelationSpecs by relationType → child-array properties.
    // Iterate in a deterministic order (sort by camelCase property key).
    let mut relation_groups: BTreeMap<String, Vec<&srs_core::types::blueprint::RelationSpec>> =
        BTreeMap::new();
    for spec in &blueprint.structure {
        let key = relation_type_to_property_key(&spec.relation_type);
        relation_groups.entry(key).or_default().push(spec);
    }

    let mut properties: Map<String, Value> = Map::new();
    properties.insert("root".into(), root_prop);

    let mut required: Vec<Value> = Vec::new();

    for (prop_key, specs) in &relation_groups {
        // Collect unique target type_ids for this relation group.
        let mut seen_targets: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut target_ids: Vec<String> = Vec::new();
        for spec in specs {
            if seen_targets.insert(spec.target_type.type_id.clone()) {
                target_ids.push(spec.target_type.type_id.clone());
            }
        }

        let items = match target_ids.as_slice() {
            [] => json!({}),
            [single] => json!({ "$ref": format!("#/definitions/{single}") }),
            multiple => {
                let refs: Vec<Value> = multiple
                    .iter()
                    .map(|id| json!({ "$ref": format!("#/definitions/{id}") }))
                    .collect();
                json!({ "oneOf": refs })
            }
        };

        // Parse cardinality from the first spec in the group.
        let raw_relation_type = specs[0].relation_type.clone();
        let cardinality_str = specs[0].cardinality.as_deref();
        let (min_items, max_items) =
            parse_cardinality(cardinality_str, &raw_relation_type, &mut diagnostics);

        let mut child_prop: Map<String, Value> = Map::new();
        child_prop.insert("type".into(), json!("array"));
        child_prop.insert("x-srs-ordered-by".into(), json!(raw_relation_type));
        child_prop.insert("items".into(), items);
        if let Some(min) = min_items {
            child_prop.insert("minItems".into(), json!(min));
        }
        if let Some(max) = max_items {
            child_prop.insert("maxItems".into(), json!(max));
        }

        properties.insert(prop_key.clone(), Value::Object(child_prop));

        // Add to required[] if any spec in this group has required: Some(true).
        if specs.iter().any(|s| s.required == Some(true)) {
            required.push(Value::String(prop_key.clone()));
        }
    }

    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": Value::Object(properties),
        "required": Value::Array(required),
        "definitions": definitions
    });

    Ok(BlueprintSchemaResult {
        schema,
        diagnostics,
    })
}

/// Convert a kebab-case or snake_case relation type string to lowerCamelCase.
///
/// Examples: `section-sequence` → `sectionSequence`, `depends-on` → `dependsOn`,
/// `precedes` → `precedes`, `contains` → `contains`.
pub(crate) fn relation_type_to_property_key(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut first = true;
    for segment in s.split(['-', '_']) {
        if segment.is_empty() {
            continue;
        }
        if first {
            result.push_str(&segment.to_lowercase());
            first = false;
        } else {
            let mut chars = segment.chars();
            if let Some(c) = chars.next() {
                result.extend(c.to_uppercase());
                result.push_str(&chars.as_str().to_lowercase());
            }
        }
    }
    result
}

/// Parse a cardinality string into (minItems, maxItems) optional values.
///
/// Formats supported:
/// - `"N..*"` → minItems N (omit when N=0)
/// - `"N..M"` → minItems N (omit when N=0), maxItems M
/// - `"N"` → minItems N, maxItems N (omit minItems when N=0)
/// - `None` or unparseable → both None; pushes diagnostic for unparseable strings
fn parse_cardinality(
    s: Option<&str>,
    relation_type: &str,
    diagnostics: &mut Vec<String>,
) -> (Option<u64>, Option<u64>) {
    let Some(s) = s else {
        return (None, None);
    };

    if let Some(dot_pos) = s.find("..") {
        let n_str = &s[..dot_pos];
        let m_str = &s[dot_pos + 2..];

        let Ok(n) = n_str.parse::<u64>() else {
            diagnostics.push(format!(
                "cardinality '{s}' on relation '{relation_type}' could not be parsed; minItems/maxItems omitted"
            ));
            return (None, None);
        };

        let min = if n == 0 { None } else { Some(n) };

        if m_str == "*" {
            return (min, None);
        }

        match m_str.parse::<u64>() {
            Ok(m) => (min, Some(m)),
            Err(_) => {
                diagnostics.push(format!(
                    "cardinality '{s}' on relation '{relation_type}' could not be parsed; minItems/maxItems omitted"
                ));
                (None, None)
            }
        }
    } else {
        match s.parse::<u64>() {
            Ok(n) => {
                let min = if n == 0 { None } else { Some(n) };
                (min, Some(n))
            }
            Err(_) => {
                diagnostics.push(format!(
                    "cardinality '{s}' on relation '{relation_type}' could not be parsed; minItems/maxItems omitted"
                ));
                (None, None)
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint_service::create_blueprint;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use srs_core::types::blueprint::{Blueprint, RelationSpec, TypeRef};
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record_type::{FieldAssignment, RecordType};
    use std::collections::HashMap;
    use std::path::PathBuf;

    // ── Helpers ──────────────────────────────────────────────────────────────

    fn field(id: &str, name: &str) -> Field {
        Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: format!("{name} description"),
            ai_guidance: json!(null),
            value_type: ValueType::String,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn assignment(field_id: &str, order: u32) -> FieldAssignment {
        FieldAssignment {
            field_id: field_id.to_string(),
            order,
            required: false,
            display_label: None,
            repeatable: false,
            min_items: None,
            max_items: None,
        }
    }

    fn record_type(id: &str, field_id: &str) -> RecordType {
        RecordType {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: format!("type-{id}"),
            version: 1,
            description: "test type".to_string(),
            fields: vec![assignment(field_id, 0)],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: None,
            lifecycle_ref: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn type_ref(type_id: &str) -> TypeRef {
        TypeRef {
            type_id: type_id.to_string(),
            type_version: None,
        }
    }

    fn minimal_blueprint_with_structure(
        root_type_ids: Vec<&str>,
        structure: Vec<RelationSpec>,
    ) -> Blueprint {
        Blueprint {
            id: String::new(),
            namespace: "test".to_string(),
            name: "test-blueprint".to_string(),
            version: 1,
            description: "test".to_string(),
            root_types: root_type_ids.into_iter().map(type_ref).collect(),
            structure,
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    fn relation_spec(
        relation_type: &str,
        source_id: &str,
        target_id: &str,
        cardinality: Option<&str>,
        required: Option<bool>,
    ) -> RelationSpec {
        RelationSpec {
            relation_type: relation_type.to_string(),
            source_type: type_ref(source_id),
            target_type: type_ref(target_id),
            cardinality: cardinality.map(|s| s.to_string()),
            required,
        }
    }

    /// Build a MemoryStore with given fields and types, with a blueprint package registered.
    fn store_with_types_and_blueprint(
        fields: Vec<Field>,
        record_types: Vec<RecordType>,
    ) -> MemoryStore {
        let manifest = Manifest {
            instance_index: vec![],
            extra: HashMap::new(),
            root: PathBuf::from("/memory"),
        };
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields,
            record_types,
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package);
        store.register_package_boundary(&None).unwrap();
        store
    }

    const FIELD_ID: &str = "00000000-0000-4000-8000-000000000001";
    const ROOT_ID: &str = "00000000-0000-4000-8000-000000000010";
    const SECTION_ID: &str = "00000000-0000-4000-8000-000000000011";
    const ATTACH_ID: &str = "00000000-0000-4000-8000-000000000012";

    // ── camelCase conversion ──────────────────────────────────────────────────

    #[test]
    fn blueprint_schema_relation_type_to_camelcase() {
        assert_eq!(
            relation_type_to_property_key("section-sequence"),
            "sectionSequence"
        );
        assert_eq!(relation_type_to_property_key("precedes"), "precedes");
        assert_eq!(relation_type_to_property_key("contains"), "contains");
        assert_eq!(relation_type_to_property_key("depends-on"), "dependsOn");
        assert_eq!(relation_type_to_property_key("refines"), "refines");
        assert_eq!(relation_type_to_property_key("derived-from"), "derivedFrom");
    }

    // ── Single root, two relation types ──────────────────────────────────────

    #[test]
    fn blueprint_schema_single_root_and_two_relation_types() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
                record_type(ATTACH_ID, FIELD_ID),
            ],
        );

        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![
                relation_spec(
                    "section-sequence",
                    ROOT_ID,
                    SECTION_ID,
                    Some("1..*"),
                    Some(true),
                ),
                relation_spec("contains", ROOT_ID, ATTACH_ID, None, None),
            ],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;

        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();
        assert!(
            result.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );

        let schema = &result.schema;
        // root is bare $ref
        assert_eq!(
            schema["properties"]["root"]["$ref"],
            json!(format!("#/definitions/{ROOT_ID}"))
        );
        // sectionSequence is an array
        assert_eq!(
            schema["properties"]["sectionSequence"]["type"],
            json!("array")
        );
        assert_eq!(
            schema["properties"]["sectionSequence"]["x-srs-ordered-by"],
            json!("section-sequence")
        );
        assert_eq!(
            schema["properties"]["sectionSequence"]["items"]["$ref"],
            json!(format!("#/definitions/{SECTION_ID}"))
        );
        assert_eq!(
            schema["properties"]["sectionSequence"]["minItems"],
            json!(1)
        );
        assert!(schema["properties"]["sectionSequence"]
            .get("maxItems")
            .is_none());
        // contains is an array with no cardinality constraints
        assert_eq!(schema["properties"]["contains"]["type"], json!("array"));
        assert!(schema["properties"]["contains"].get("minItems").is_none());
        // definitions has exactly three entries
        let defs = schema["definitions"].as_object().unwrap();
        assert_eq!(
            defs.len(),
            3,
            "expected 3 definitions, got: {:?}",
            defs.keys().collect::<Vec<_>>()
        );
        assert!(defs.contains_key(ROOT_ID));
        assert!(defs.contains_key(SECTION_ID));
        assert!(defs.contains_key(ATTACH_ID));
        // required includes sectionSequence (required: Some(true))
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("sectionSequence")));
        assert!(!schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("contains")));
    }

    // ── Multiple root types ───────────────────────────────────────────────────

    #[test]
    fn blueprint_schema_multiple_root_types() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
            ],
        );

        let bp = minimal_blueprint_with_structure(vec![ROOT_ID, SECTION_ID], vec![]);
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;

        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();
        let root = &result.schema["properties"]["root"];
        assert!(root.get("$ref").is_none(), "expected oneOf not $ref");
        let one_of = root["oneOf"].as_array().unwrap();
        assert_eq!(one_of.len(), 2);
        assert!(one_of
            .iter()
            .any(|v| v["$ref"] == json!(format!("#/definitions/{ROOT_ID}"))));
        assert!(one_of
            .iter()
            .any(|v| v["$ref"] == json!(format!("#/definitions/{SECTION_ID}"))));
    }

    // ── Cardinality parsing ───────────────────────────────────────────────────

    #[test]
    fn blueprint_schema_cardinality_min_max() {
        let mut diag = Vec::new();
        // "1..*" → minItems: 1, no maxItems
        let (min, max) = parse_cardinality(Some("1..*"), "test", &mut diag);
        assert_eq!(min, Some(1));
        assert_eq!(max, None);
        // "0..3" → no minItems (N=0 omitted), maxItems: 3
        let (min, max) = parse_cardinality(Some("0..3"), "test", &mut diag);
        assert_eq!(min, None);
        assert_eq!(max, Some(3));
        // "1" → minItems: 1, maxItems: 1
        let (min, max) = parse_cardinality(Some("1"), "test", &mut diag);
        assert_eq!(min, Some(1));
        assert_eq!(max, Some(1));
        // None → both None, no diagnostic
        let (min, max) = parse_cardinality(None, "test", &mut diag);
        assert_eq!(min, None);
        assert_eq!(max, None);
        assert!(diag.is_empty(), "unexpected diagnostics: {:?}", diag);
        // unparseable → both None + diagnostic
        let (min, max) = parse_cardinality(Some("bad"), "test", &mut diag);
        assert_eq!(min, None);
        assert_eq!(max, None);
        assert!(!diag.is_empty());
    }

    // ── Single vs multiple targets ────────────────────────────────────────────

    #[test]
    fn blueprint_schema_single_target_uses_ref_not_oneof() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
            ],
        );
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![relation_spec("contains", ROOT_ID, SECTION_ID, None, None)],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        let items = &result.schema["properties"]["contains"]["items"];
        assert!(
            items.get("$ref").is_some(),
            "expected bare $ref, got: {items}"
        );
        assert!(
            items.get("oneOf").is_none(),
            "expected no oneOf for single target"
        );
    }

    #[test]
    fn blueprint_schema_multiple_targets_uses_oneof() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
                record_type(ATTACH_ID, FIELD_ID),
            ],
        );
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![
                relation_spec("contains", ROOT_ID, SECTION_ID, None, None),
                relation_spec("contains", ROOT_ID, ATTACH_ID, None, None),
            ],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        let items = &result.schema["properties"]["contains"]["items"];
        assert!(
            items.get("oneOf").is_some(),
            "expected oneOf for multiple targets"
        );
        assert_eq!(items["oneOf"].as_array().unwrap().len(), 2);
    }

    // ── Unresolvable type → diagnostic ───────────────────────────────────────

    #[test]
    fn blueprint_schema_unresolvable_type_emits_diagnostic() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![record_type(ROOT_ID, FIELD_ID)],
        );
        // Target type SECTION_ID is NOT in the package — cannot be projected.
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![relation_spec("contains", ROOT_ID, SECTION_ID, None, None)],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        // SECTION_ID should be absent from definitions; ROOT_ID still present.
        let defs = result.schema["definitions"].as_object().unwrap();
        assert!(
            defs.contains_key(ROOT_ID),
            "root type should still be projected"
        );
        assert!(
            !defs.contains_key(SECTION_ID),
            "unresolvable type should be absent"
        );
        assert!(
            result.diagnostics.iter().any(|d| d.contains(SECTION_ID)),
            "expected diagnostic naming the unresolvable type: {:?}",
            result.diagnostics
        );
        // No [WARN] prefix
        for d in &result.diagnostics {
            assert!(
                !d.starts_with("[WARN]"),
                "diagnostic must not have [WARN] prefix: {d}"
            );
        }
    }

    // ── Unknown blueprint → Err ───────────────────────────────────────────────

    #[test]
    fn blueprint_schema_unknown_blueprint_errors() {
        let store = store_with_types_and_blueprint(vec![], vec![]);
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: "nonexistent".to_string(),
            },
        );
        assert!(
            matches!(result, Err(RepositoryError::BlueprintNotFound { .. })),
            "expected BlueprintNotFound, got: {:?}",
            result
        );
    }

    // ── Required propagates from RelationSpec ─────────────────────────────────

    #[test]
    fn blueprint_schema_required_propagates_from_relation_spec() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
            ],
        );
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![relation_spec(
                "section-sequence",
                ROOT_ID,
                SECTION_ID,
                None,
                Some(true),
            )],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        let required = result.schema["required"].as_array().unwrap();
        assert!(
            required.contains(&json!("sectionSequence")),
            "sectionSequence should be in required: {:?}",
            required
        );
    }

    // ── x-srs-ordered-by is raw relation type ─────────────────────────────────

    #[test]
    fn blueprint_schema_xsrs_ordered_by_is_raw_relation_type() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
            ],
        );
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![relation_spec(
                "section-sequence",
                ROOT_ID,
                SECTION_ID,
                None,
                None,
            )],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        assert_eq!(
            result.schema["properties"]["sectionSequence"]["x-srs-ordered-by"],
            json!("section-sequence"),
            "x-srs-ordered-by must be the raw relation type string, not camelCase"
        );
    }

    // ── Source-only types excluded from definitions ────────────────────────────

    #[test]
    fn blueprint_schema_source_only_types_not_in_definitions() {
        // SOURCE_ID is only used as source_type, not as a root_type or target_type.
        const SOURCE_ID: &str = "00000000-0000-4000-8000-0000000000ff";

        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SOURCE_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
            ],
        );
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![relation_spec("contains", SOURCE_ID, SECTION_ID, None, None)],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        let defs = result.schema["definitions"].as_object().unwrap();
        assert!(
            !defs.contains_key(SOURCE_ID),
            "source-only type must not appear in definitions: {:?}",
            defs.keys().collect::<Vec<_>>()
        );
        assert!(
            defs.contains_key(ROOT_ID),
            "root type should be in definitions"
        );
        assert!(
            defs.contains_key(SECTION_ID),
            "target type should be in definitions"
        );
    }

    // ── Memory roundtrip ──────────────────────────────────────────────────────

    #[test]
    fn blueprint_schema_memory_roundtrip() {
        let store = store_with_types_and_blueprint(
            vec![field(FIELD_ID, "title")],
            vec![
                record_type(ROOT_ID, FIELD_ID),
                record_type(SECTION_ID, FIELD_ID),
            ],
        );
        let bp = minimal_blueprint_with_structure(
            vec![ROOT_ID],
            vec![relation_spec(
                "section-sequence",
                ROOT_ID,
                SECTION_ID,
                Some("1..*"),
                None,
            )],
        );
        let bp = create_blueprint(&store, bp, None).unwrap().blueprint;
        let result = blueprint_schema(
            &store,
            BlueprintSchemaInput {
                blueprint_id: bp.id,
            },
        )
        .unwrap();

        let serialized = serde_json::to_string(&result.schema).unwrap();
        let reparsed: Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            reparsed["$schema"],
            json!("http://json-schema.org/draft-07/schema#")
        );
        assert_eq!(reparsed["type"], json!("object"));
        assert!(reparsed["definitions"]
            .as_object()
            .unwrap()
            .contains_key(ROOT_ID));
        assert!(reparsed["definitions"]
            .as_object()
            .unwrap()
            .contains_key(SECTION_ID));
        assert_eq!(
            reparsed["properties"]["sectionSequence"]["type"],
            json!("array")
        );
        assert_eq!(
            reparsed["properties"]["sectionSequence"]["minItems"],
            json!(1)
        );
    }
}
