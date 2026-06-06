//! `type schema` projection service.
//!
//! Resolves a Type plus its referenced Fields and emits a **draft-07 JSON Schema**
//! describing a single record's `fieldValues`, keyed by field `name`. This is a
//! pure projection over already-loaded `RecordType` + `Field` data — no new data
//! model and no write path. See issue #60 and `plans/type-schema-command.md`.
//!
//! Non-fatal projection problems (a dangling `fieldId`, a select/multiselect field
//! with no `allowedValues`) are collected into [`TypeSchemaResult::diagnostics`]
//! rather than aborting the projection. An unresolvable Type is a hard
//! [`RepositoryError`].

use crate::error::RepositoryError;
use crate::package_service::{get_field_by_id, get_type_by_id, get_type_by_id_latest};
use crate::package_service::{GetFieldResult, GetTypeResult};
use crate::store::RepositoryStore;
use serde_json::{json, Map, Value};
use srs_core::types::field::{Field, ValueType};
use srs_core::types::record_type::FieldAssignment;

/// Input contract for [`type_schema`].
#[derive(Debug, Clone)]
pub struct TypeSchemaInput {
    pub type_id: String,
    /// When `None`, the latest version of the Type is resolved.
    pub type_version: Option<u32>,
}

/// Output contract for [`type_schema`].
#[derive(Debug, Clone)]
pub struct TypeSchemaResult {
    /// The generated draft-07 JSON Schema object.
    pub schema: Value,
    /// Non-fatal problems encountered while projecting (dangling fields, missing
    /// `allowedValues`). Surfaced by the CLI in the envelope's top-level
    /// `diagnostics[]`.
    pub diagnostics: Vec<String>,
}

/// Project a Type + its Fields into a draft-07 JSON Schema for a record's `fieldValues`.
///
/// Returns `Err(RepositoryError::TypeNotFound)` when the Type cannot be resolved.
pub fn type_schema(
    store: &dyn RepositoryStore,
    input: TypeSchemaInput,
) -> Result<TypeSchemaResult, RepositoryError> {
    let record_type = match input.type_version {
        Some(version) => match get_type_by_id(store, &input.type_id, version)? {
            GetTypeResult::Found(rt) => rt,
            GetTypeResult::NotFound => {
                return Err(RepositoryError::TypeNotFound {
                    type_id: input.type_id,
                    version,
                })
            }
        },
        None => match get_type_by_id_latest(store, &input.type_id)? {
            GetTypeResult::Found(rt) => rt,
            GetTypeResult::NotFound => {
                return Err(RepositoryError::TypeNotFound {
                    type_id: input.type_id,
                    // 0 signals "any version" in the not-found message for the latest lookup.
                    version: 0,
                });
            }
        },
    };

    let mut diagnostics = Vec::new();
    let mut properties = Map::new();
    let mut required = Vec::new();

    // Project the Type's own field assignments, in declared order.
    let mut assignments: Vec<&FieldAssignment> = record_type.fields.iter().collect();
    assignments.sort_by_key(|fa| fa.order);

    for fa in assignments {
        let field = match get_field_by_id(store, &fa.field_id)? {
            GetFieldResult::Found(field) => *field,
            GetFieldResult::NotFound => {
                diagnostics.push(format!(
                    "field assignment references unknown fieldId '{}'; skipped",
                    fa.field_id
                ));
                continue;
            }
        };

        let property = field_to_property(&field, fa, &mut diagnostics);
        if fa.required {
            required.push(Value::String(field.name.clone()));
        }
        properties.insert(field.name.clone(), property);
    }

    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "properties": Value::Object(properties),
        "required": Value::Array(required),
        "additionalProperties": false
    });

    Ok(TypeSchemaResult {
        schema,
        diagnostics,
    })
}

/// Map a single resolved Field + its assignment to a draft-07 property schema.
fn field_to_property(
    field: &Field,
    assignment: &FieldAssignment,
    diagnostics: &mut Vec<String>,
) -> Value {
    let mut prop = Map::new();

    match field.value_type {
        ValueType::String => {
            prop.insert("type".into(), json!("string"));
        }
        ValueType::Text => {
            prop.insert("type".into(), json!("string"));
            prop.insert("x-srs-widget".into(), json!("textarea"));
        }
        ValueType::Number => {
            prop.insert("type".into(), json!("number"));
        }
        ValueType::Boolean => {
            prop.insert("type".into(), json!("boolean"));
        }
        ValueType::Date => {
            prop.insert("type".into(), json!("string"));
            prop.insert("format".into(), json!("date"));
        }
        ValueType::Url => {
            prop.insert("type".into(), json!("string"));
            prop.insert("format".into(), json!("uri"));
        }
        ValueType::Select => {
            insert_enum(&mut prop, field, diagnostics);
        }
        ValueType::Multiselect => {
            prop.insert("type".into(), json!("array"));
            let mut items = Map::new();
            insert_enum(&mut items, field, diagnostics);
            prop.insert("items".into(), Value::Object(items));
        }
    }

    // title: displayLabel wins, else the field's description.
    let title = assignment
        .display_label
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if field.description.is_empty() {
                None
            } else {
                Some(field.description.clone())
            }
        });
    if let Some(title) = title {
        prop.insert("title".into(), json!(title));
    }

    if let Some(default) = &field.default_value {
        prop.insert("default".into(), default.clone());
    }

    prop.insert("x-srs-order".into(), json!(assignment.order));
    prop.insert("x-srs-field-id".into(), json!(field.id));

    // aiGuidance: a string becomes `description`; a structured object is preserved
    // under a vendor key.
    match &field.ai_guidance {
        Value::String(s) if !s.is_empty() => {
            prop.insert("description".into(), json!(s));
        }
        Value::Null => {}
        other => {
            prop.insert("x-srs-ai-guidance".into(), other.clone());
        }
    }

    Value::Object(prop)
}

/// Insert an `enum` populated from the field's `allowedValues`. Emits a diagnostic
/// when no values are declared (the property is left without an `enum`).
fn insert_enum(target: &mut Map<String, Value>, field: &Field, diagnostics: &mut Vec<String>) {
    match &field.allowed_values {
        Some(values) if !values.is_empty() => {
            target.insert(
                "enum".into(),
                Value::Array(values.iter().map(|v| json!(v)).collect()),
            );
        }
        _ => {
            diagnostics.push(format!(
                "field '{}' ({:?}) has no allowedValues; enum omitted",
                field.name, field.value_type
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use crate::package::Package;
    use crate::store::memory::MemoryStore;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn field(id: &str, name: &str, value_type: ValueType) -> Field {
        Field {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: format!("{name} description"),
            ai_guidance: json!(null),
            value_type,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn assignment(field_id: &str, order: u32, required: bool) -> FieldAssignment {
        FieldAssignment {
            field_id: field_id.to_string(),
            order,
            required,
            display_label: None,
            repeatable: false,
            min_items: None,
            max_items: None,
        }
    }

    /// Build a MemoryStore seeded with the given fields and a single type.
    fn store_with(
        fields: Vec<Field>,
        record_type: srs_core::types::record_type::RecordType,
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
            record_types: vec![record_type],
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
        MemoryStore::new(manifest, package)
    }

    fn make_type(
        id: &str,
        assignments: Vec<FieldAssignment>,
    ) -> srs_core::types::record_type::RecordType {
        srs_core::types::record_type::RecordType {
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "A test type".to_string(),
            fields: assignments,
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

    const TID: &str = "00000000-0000-4000-8000-0000000000aa";

    fn fid(n: u8) -> String {
        format!("00000000-0000-4000-8000-0000000000{n:02x}")
    }

    #[test]
    fn type_schema_covers_all_value_types() {
        let types = [
            ValueType::String,
            ValueType::Text,
            ValueType::Number,
            ValueType::Boolean,
            ValueType::Date,
            ValueType::Url,
            ValueType::Select,
            ValueType::Multiselect,
        ];
        let mut fields = Vec::new();
        let mut assignments = Vec::new();
        for (i, vt) in types.iter().enumerate() {
            let id = fid(i as u8);
            let name = format!("f_{i}");
            let mut f = field(&id, &name, *vt);
            if matches!(vt, ValueType::Select | ValueType::Multiselect) {
                f.allowed_values = Some(vec!["a".into(), "b".into()]);
            }
            fields.push(f);
            assignments.push(assignment(&id, i as u32, false));
        }
        let store = store_with(fields, make_type(TID, assignments));

        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        let props = &result.schema["properties"];

        assert_eq!(props["f_0"]["type"], json!("string"));
        assert_eq!(props["f_1"]["type"], json!("string"));
        assert_eq!(props["f_1"]["x-srs-widget"], json!("textarea"));
        assert_eq!(props["f_2"]["type"], json!("number"));
        assert_eq!(props["f_3"]["type"], json!("boolean"));
        assert_eq!(props["f_4"]["format"], json!("date"));
        assert_eq!(props["f_5"]["format"], json!("uri"));
        assert_eq!(props["f_6"]["enum"], json!(["a", "b"]));
        assert_eq!(props["f_7"]["type"], json!("array"));
        assert_eq!(props["f_7"]["items"]["enum"], json!(["a", "b"]));
        assert_eq!(
            result.schema["$schema"],
            json!("http://json-schema.org/draft-07/schema#")
        );
        assert_eq!(result.schema["additionalProperties"], json!(false));
        assert!(
            result.diagnostics.is_empty(),
            "no diagnostics expected: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn type_schema_select_emits_enum() {
        let mut sel = field(&fid(1), "color", ValueType::Select);
        sel.allowed_values = Some(vec!["red".into(), "green".into()]);
        let mut multi = field(&fid(2), "tags", ValueType::Multiselect);
        multi.allowed_values = Some(vec!["x".into(), "y".into()]);
        let store = store_with(
            vec![sel, multi],
            make_type(
                TID,
                vec![assignment(&fid(1), 0, false), assignment(&fid(2), 1, false)],
            ),
        );
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        assert_eq!(
            result.schema["properties"]["color"]["enum"],
            json!(["red", "green"])
        );
        assert_eq!(
            result.schema["properties"]["tags"]["items"]["enum"],
            json!(["x", "y"])
        );
    }

    #[test]
    fn type_schema_required_array() {
        let store = store_with(
            vec![
                field(&fid(1), "a", ValueType::String),
                field(&fid(2), "b", ValueType::String),
            ],
            make_type(
                TID,
                vec![assignment(&fid(1), 0, true), assignment(&fid(2), 1, false)],
            ),
        );
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        assert_eq!(result.schema["required"], json!(["a"]));
    }

    #[test]
    fn type_schema_order_recoverable() {
        let store = store_with(
            vec![
                field(&fid(1), "a", ValueType::String),
                field(&fid(2), "b", ValueType::String),
            ],
            // Declared out of order; service sorts by `order`.
            make_type(
                TID,
                vec![assignment(&fid(2), 5, false), assignment(&fid(1), 2, false)],
            ),
        );
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        assert_eq!(result.schema["properties"]["a"]["x-srs-order"], json!(2));
        assert_eq!(result.schema["properties"]["b"]["x-srs-order"], json!(5));
    }

    #[test]
    fn type_schema_title_prefers_display_label() {
        // display_label set -> wins.
        let mut a = assignment(&fid(1), 0, false);
        a.display_label = Some("Custom Label".into());
        // display_label absent -> falls back to field.description.
        let b = assignment(&fid(2), 1, false);
        let store = store_with(
            vec![
                field(&fid(1), "a", ValueType::String),
                field(&fid(2), "b", ValueType::String),
            ],
            make_type(TID, vec![a, b]),
        );
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        assert_eq!(
            result.schema["properties"]["a"]["title"],
            json!("Custom Label")
        );
        assert_eq!(
            result.schema["properties"]["b"]["title"],
            json!("b description")
        );
    }

    #[test]
    fn type_schema_unknown_type_errors() {
        let store = store_with(vec![], make_type(TID, vec![]));
        // Unknown id.
        let err = type_schema(
            &store,
            TypeSchemaInput {
                type_id: "nope".to_string(),
                type_version: None,
            },
        );
        assert!(matches!(err, Err(RepositoryError::TypeNotFound { .. })));
        // Unknown version of an existing id.
        let err = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: Some(99),
            },
        );
        assert!(matches!(err, Err(RepositoryError::TypeNotFound { .. })));
    }

    #[test]
    fn type_schema_dangling_field_skipped() {
        let store = store_with(
            vec![field(&fid(1), "a", ValueType::String)],
            make_type(
                TID,
                vec![
                    assignment(&fid(1), 0, false),
                    assignment("missing-field-id", 1, false),
                ],
            ),
        );
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        // Resolvable field present; dangling one absent; result still Ok.
        assert!(result.schema["properties"].get("a").is_some());
        assert_eq!(result.schema["properties"].as_object().unwrap().len(), 1);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.contains("missing-field-id")),
            "expected a diagnostic naming the dangling field: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn type_schema_memory_roundtrip() {
        // Populate a store, project, and confirm the output serializes as JSON
        // (cross-store coverage per the storage-boundary rules).
        let mut f = field(&fid(1), "title", ValueType::String);
        f.default_value = Some(json!("untitled"));
        let store = store_with(vec![f], make_type(TID, vec![assignment(&fid(1), 0, true)]));
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        let serialized = serde_json::to_string(&result.schema).unwrap();
        let reparsed: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            reparsed["properties"]["title"]["default"],
            json!("untitled")
        );
        assert_eq!(
            reparsed["properties"]["title"]["x-srs-field-id"],
            json!(fid(1))
        );
    }
}
