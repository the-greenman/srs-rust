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
use crate::package_service::GetTypeResult;
use crate::package_service::{get_type_by_id, get_type_by_id_latest};
use crate::store::RepositoryStore;
use serde_json::{json, Map, Value};
use srs_core::types::field::{Datatype, Field, RefMode, StringFormat};
use srs_core::types::record_type::FieldAssignment;

/// Input contract for [`type_schema`].
#[derive(Debug, Clone)]
pub struct TypeSchemaInput {
    pub type_id: String,
    /// When `None`, the latest version of the Type is resolved.
    pub type_version: Option<u32>,
}

/// Output contract for [`type_schema`].
#[derive(Debug, Clone, serde::Serialize)]
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

    let package = store.load_package()?;
    // Walk the inheritance chain to collect all effective field assignments
    // (own + inherited), sorted by order.
    let assignments = package.effective_fields(&record_type)?;

    let mut diagnostics = Vec::new();
    let mut properties = Map::new();
    let mut required = Vec::new();

    for (idx, fa) in assignments.iter().enumerate() {
        let field = match package.resolve_field(&fa.field_id) {
            Some(f) => f.clone(),
            None => {
                diagnostics.push(format!(
                    "field assignment references unknown fieldId '{}'; skipped",
                    fa.field_id
                ));
                continue;
            }
        };

        let mut visiting = vec![(record_type.id.clone(), record_type.version)];
        let mut property = field_to_property(&field, fa, &package, &mut visiting, &mut diagnostics);
        if let Some(obj) = property.as_object_mut() {
            obj.insert("x-srs-order".into(), json!(idx + 1));
        }
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
///
/// `visiting` carries the (typeId, version) chain of inline-composite ranges
/// currently being expanded, for cycle protection.
fn field_to_property(
    field: &Field,
    assignment: &FieldAssignment,
    package: &crate::package::Package,
    visiting: &mut Vec<(String, u32)>,
    diagnostics: &mut Vec<String>,
) -> Value {
    let mut prop = Map::new();

    // RFC-032: the shape comes from the `fieldType` facets. A list wraps the
    // single-value shape in an array, which is how the pre-RFC-032
    // `multiselect` case used to be special-cased.
    let mut value_shape = Map::new();
    insert_value_shape(&mut value_shape, field, package, visiting, diagnostics);
    if field.is_list() {
        prop.insert("type".into(), json!("array"));
        prop.insert("items".into(), Value::Object(value_shape));
        if let Some(min) = field.field_type.min_items {
            prop.insert("minItems".into(), json!(min));
        }
        if let Some(max) = field.field_type.max_items {
            prop.insert("maxItems".into(), json!(max));
        }
    } else {
        prop.extend(value_shape);
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

    // Field help text: `description` (short caption) and `instructions` (fuller
    // human guidance) each get a dedicated vendor key so neither collides with
    // `title` (label) or `description` (already occupied by string aiGuidance
    // below). See ADR-026.
    if !field.description.is_empty() {
        prop.insert("x-srs-description".into(), json!(field.description));
    }
    if let Some(instructions) = &field.instructions {
        if !instructions.is_empty() {
            prop.insert("x-srs-instructions".into(), json!(instructions));
        }
    }

    if let Some(default) = &field.default_value {
        prop.insert("default".into(), default.clone());
    }

    // RFC-039: `x-srs-field-id` is retired — instance keys and schema keys are
    // both `Field.name`, so there is no id-keyed instance left to bridge to.
    prop.insert("x-srs-order".into(), json!(assignment.order));

    // aiGuidance.purpose becomes `description`; any richer structured
    // guidance (extraction, negativeGuidance, examples) is preserved under a
    // vendor key, since it has no standard JSON Schema keyword to land on.
    if !field.ai_guidance.purpose.is_empty() {
        prop.insert("description".into(), json!(field.ai_guidance.purpose));
    }
    let has_structured_guidance = field.ai_guidance.extraction.is_some()
        || field.ai_guidance.negative_guidance.is_some()
        || field.ai_guidance.examples.is_some();
    if has_structured_guidance {
        if let Ok(guidance) = serde_json::to_value(&field.ai_guidance) {
            prop.insert("x-srs-ai-guidance".into(), guidance);
        }
    }

    Value::Object(prop)
}

/// Insert the single-value shape a field's `fieldType` projects to, ignoring
/// cardinality (the caller wraps a list in an array).
///
/// This is the **editor-facing** projection: draft-07 plus `x-srs-*` hints,
/// consumed by schema-driven form renderers. The standards-compliant
/// validation projection is `srs_projection::type_to_json_schema` — the two are
/// deliberately separate artifacts (srs-rust#770).
fn insert_value_shape(
    target: &mut Map<String, Value>,
    field: &Field,
    package: &crate::package::Package,
    visiting: &mut Vec<(String, u32)>,
    diagnostics: &mut Vec<String>,
) {
    let ft = &field.field_type;
    match ft.datatype {
        Datatype::String => {
            if ft.is_closed() {
                insert_enum(target, field, diagnostics);
                return;
            }
            target.insert("type".into(), json!("string"));
            match ft.format {
                // Prose formats keep the multi-line widget the pre-RFC-032
                // `text` valueType used to select.
                Some(StringFormat::Markdown) => {
                    target.insert("contentMediaType".into(), json!("text/markdown"));
                    target.insert("x-srs-widget".into(), json!("textarea"));
                }
                Some(StringFormat::Plain) => {
                    target.insert("x-srs-widget".into(), json!("textarea"));
                }
                Some(StringFormat::Uri) => {
                    target.insert("format".into(), json!("uri"));
                }
                Some(StringFormat::Uuid) => {
                    target.insert("format".into(), json!("uuid"));
                }
                Some(StringFormat::Email) => {
                    target.insert("format".into(), json!("email"));
                }
                None => {}
            }
            if let Some(c) = &ft.constraints {
                if let Some(min) = c.min_length {
                    target.insert("minLength".into(), json!(min));
                }
                if let Some(max) = c.max_length {
                    target.insert("maxLength".into(), json!(max));
                }
                if let Some(pattern) = &c.pattern {
                    target.insert("pattern".into(), json!(pattern));
                }
            }
        }
        Datatype::Number | Datatype::Integer => {
            target.insert(
                "type".into(),
                json!(if ft.datatype == Datatype::Integer {
                    "integer"
                } else {
                    "number"
                }),
            );
            if let Some(c) = &ft.constraints {
                if let Some(min) = &c.minimum {
                    target.insert("minimum".into(), json!(min));
                }
                if let Some(max) = &c.maximum {
                    target.insert("maximum".into(), json!(max));
                }
            }
        }
        Datatype::Boolean => {
            target.insert("type".into(), json!("boolean"));
        }
        Datatype::Date => {
            target.insert("type".into(), json!("string"));
            target.insert("format".into(), json!("date"));
        }
        Datatype::DateTime => {
            target.insert("type".into(), json!("string"));
            target.insert("format".into(), json!("date-time"));
        }
        Datatype::Ref => {
            match ft.effective_mode() {
                // A reference carries the target instance id.
                RefMode::Reference => {
                    target.insert("type".into(), json!("string"));
                    target.insert("format".into(), json!("uuid"));
                }
                // RFC-039 [R3]: an inline-composite value is a fieldValues map
                // for the rangeType — the editor projection expands the range
                // recursively so schema-driven editors can render the nested
                // form/grid the retired FieldGroup projection used to supply.
                RefMode::Inline => {
                    target.insert("type".into(), json!("object"));
                    if let Some(range) = &field.field_type.range_type {
                        let key = (range.type_id.clone(), range.type_version);
                        if visiting.contains(&key) {
                            diagnostics.push(format!(
                                "inline composite range cycle at {}@{}; emitted unexpanded object",
                                range.type_id, range.type_version
                            ));
                        } else if let Some(range_rt) =
                            package.resolve_type(&range.type_id, range.type_version)
                        {
                            visiting.push(key);
                            let range_rt = range_rt.clone();
                            let mut item_props = Map::new();
                            let mut item_required = Vec::new();
                            match package.effective_fields(&range_rt) {
                                Ok(assignments) => {
                                    for fa in &assignments {
                                        match package.resolve_field(&fa.field_id) {
                                            Some(f) => {
                                                let f = f.clone();
                                                let prop = field_to_property(
                                                    &f,
                                                    fa,
                                                    package,
                                                    visiting,
                                                    diagnostics,
                                                );
                                                if fa.required {
                                                    item_required
                                                        .push(Value::String(f.name.clone()));
                                                }
                                                item_props.insert(f.name.clone(), prop);
                                            }
                                            None => diagnostics.push(format!(
                                                "range type {}@{} references unknown fieldId '{}'; skipped",
                                                range.type_id, range.type_version, fa.field_id
                                            )),
                                        }
                                    }
                                    target.insert("properties".into(), Value::Object(item_props));
                                    target.insert("required".into(), Value::Array(item_required));
                                    target.insert("additionalProperties".into(), json!(false));
                                }
                                Err(e) => diagnostics.push(format!(
                                    "could not resolve effective fields of range {}@{}: {e}",
                                    range.type_id, range.type_version
                                )),
                            }
                            visiting.pop();
                        } else {
                            diagnostics.push(format!(
                                "inline composite rangeType {}@{} does not resolve; emitted unexpanded object",
                                range.type_id, range.type_version
                            ));
                        }
                    }
                }
            }
            if let Some(range) = &ft.range_type {
                target.insert("x-srs-range-type-id".into(), json!(range.type_id));
                target.insert("x-srs-range-type-version".into(), json!(range.type_version));
            }
        }
        Datatype::Map => {
            target.insert("type".into(), json!("object"));
        }
        Datatype::Dependent => {
            // Deliberately unconstrained: the value conforms to another field's
            // type, which JSON Schema cannot express here.
            target.insert("x-srs-depends-on".into(), json!(ft.depends_on));
        }
    }
}

/// Insert an `enum` populated from the field's closed-domain `allowedValues`.
/// Emits a diagnostic when no values are declared (the property is left without
/// an `enum`).
fn insert_enum(target: &mut Map<String, Value>, field: &Field, diagnostics: &mut Vec<String>) {
    // No `type` keyword: `enum` alone constrains the value, and this matches the
    // shape editors have consumed since before RFC-032.
    match field.allowed_values() {
        Some(values) if !values.is_empty() => {
            target.insert(
                "enum".into(),
                Value::Array(values.iter().map(|v| json!(v)).collect()),
            );
        }
        _ => {
            diagnostics.push(format!(
                "field '{}' (closed {}) has no allowedValues; enum omitted",
                field.name,
                field.datatype().as_str()
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
    use srs_core::types::field::{AiGuidance, FieldType};
    use std::path::PathBuf;

    fn field(id: &str, name: &str, field_type: FieldType) -> Field {
        Field {
            schema: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version: 1,
            description: format!("{name} description"),
            instructions: None,
            ai_guidance: AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            },
            field_type,
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn assignment(field_id: &str, order: u32, required: bool) -> FieldAssignment {
        FieldAssignment {
            field_id: field_id.to_string(),
            order,
            required,
            display_label: None,
        }
    }

    fn make_package(
        fields: Vec<Field>,
        record_types: Vec<srs_core::types::record_type::RecordType>,
    ) -> Package {
        Package {
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
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    /// Build a MemoryStore seeded with the given fields and a single type.
    fn store_with(
        fields: Vec<Field>,
        record_type: srs_core::types::record_type::RecordType,
    ) -> MemoryStore {
        store_with_types(fields, vec![record_type])
    }

    /// Build a MemoryStore seeded with the given fields and multiple types.
    fn store_with_types(
        fields: Vec<Field>,
        record_types: Vec<srs_core::types::record_type::RecordType>,
    ) -> MemoryStore {
        let manifest = Manifest {
            instance_index: vec![],
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: std::collections::BTreeMap::new(),
            source_documents_path: None,
            source_document_index: None,
            root: PathBuf::from("/memory"),
        };
        MemoryStore::new(manifest, make_package(fields, record_types))
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
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::BTreeMap::new(),
        }
    }

    const TID: &str = "00000000-0000-4000-8000-0000000000aa";

    fn fid(n: u8) -> String {
        format!("00000000-0000-4000-8000-0000000000{n:02x}")
    }

    #[test]
    fn type_schema_covers_every_datatype() {
        let types = [
            FieldType::string(),
            FieldType::text(),
            FieldType::markdown(),
            FieldType::number(),
            FieldType::integer(),
            FieldType::boolean(),
            FieldType::date(),
            FieldType::date_time(),
            FieldType::uri(),
            FieldType::select(["a", "b"]),
            FieldType::multiselect(["a", "b"]),
        ];
        let mut fields = Vec::new();
        let mut assignments = Vec::new();
        for (i, vt) in types.iter().enumerate() {
            let id = fid(i as u8);
            let name = format!("f_{i}");
            let f = field(&id, &name, vt.clone());
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

        // Order matches the `types` array above.
        assert_eq!(props["f_0"]["type"], json!("string"));
        // `string (plain)` and `string (markdown)` both keep the multi-line
        // widget the pre-RFC-032 `text` valueType selected.
        assert_eq!(props["f_1"]["type"], json!("string"));
        assert_eq!(props["f_1"]["x-srs-widget"], json!("textarea"));
        assert_eq!(props["f_2"]["contentMediaType"], json!("text/markdown"));
        assert_eq!(props["f_2"]["x-srs-widget"], json!("textarea"));
        assert_eq!(props["f_3"]["type"], json!("number"));
        assert_eq!(props["f_4"]["type"], json!("integer"));
        assert_eq!(props["f_5"]["type"], json!("boolean"));
        assert_eq!(props["f_6"]["format"], json!("date"));
        assert_eq!(props["f_7"]["format"], json!("date-time"));
        assert_eq!(props["f_8"]["format"], json!("uri"));
        assert_eq!(props["f_9"]["enum"], json!(["a", "b"]));
        assert_eq!(props["f_10"]["type"], json!("array"));
        assert_eq!(props["f_10"]["items"]["enum"], json!(["a", "b"]));
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
    fn field_to_property_emits_description_and_instructions_keys() {
        let mut f = field("f-help", "help_field", FieldType::string());
        f.description = "Short caption.".to_string();
        f.instructions = Some("Fuller how-to-complete guidance.".to_string());
        let a = assignment("f-help", 0, false);
        let pkg = make_package(vec![], vec![]);
        let mut visiting = Vec::new();
        let mut diagnostics = Vec::new();

        let prop = field_to_property(&f, &a, &pkg, &mut visiting, &mut diagnostics);

        assert_eq!(prop["x-srs-description"], json!("Short caption."));
        assert_eq!(
            prop["x-srs-instructions"],
            json!("Fuller how-to-complete guidance.")
        );
    }

    #[test]
    fn field_to_property_omits_absent_instructions() {
        let mut f = field("f-no-help", "no_help_field", FieldType::string());
        f.description = String::new();
        f.instructions = None;
        let a = assignment("f-no-help", 0, false);
        let pkg = make_package(vec![], vec![]);
        let mut visiting = Vec::new();
        let mut diagnostics = Vec::new();

        let prop = field_to_property(&f, &a, &pkg, &mut visiting, &mut diagnostics);

        assert!(prop.get("x-srs-description").is_none());
        assert!(prop.get("x-srs-instructions").is_none());

        // Some("") must also omit the key — empty strings carry no help text.
        f.instructions = Some(String::new());
        let prop = field_to_property(&f, &a, &pkg, &mut visiting, &mut diagnostics);
        assert!(prop.get("x-srs-instructions").is_none());
    }

    #[test]
    fn type_schema_select_emits_enum() {
        let sel = field(&fid(1), "color", FieldType::select(["red", "green"]));
        let multi = field(&fid(2), "tags", FieldType::multiselect(["x", "y"]));
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
                field(&fid(1), "a", FieldType::string()),
                field(&fid(2), "b", FieldType::string()),
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
                field(&fid(1), "a", FieldType::string()),
                field(&fid(2), "b", FieldType::string()),
            ],
            // Declared out of order; service sorts by `assignment.order` (2 < 5),
            // then emits 1-based positional x-srs-order so fieldOrder reordering
            // is reflected without collisions across inheritance levels.
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
        // a has assignment.order=2 (sorted first), so position 1; b has order=5, position 2.
        assert_eq!(result.schema["properties"]["a"]["x-srs-order"], json!(1));
        assert_eq!(result.schema["properties"]["b"]["x-srs-order"], json!(2));
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
                field(&fid(1), "a", FieldType::string()),
                field(&fid(2), "b", FieldType::string()),
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
            vec![field(&fid(1), "a", FieldType::string())],
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
        let mut f = field(&fid(1), "title", FieldType::string());
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
        // RFC-039: schema keys are Field.name; no id bridge key is emitted.
        assert!(reparsed["properties"]["title"]
            .get("x-srs-field-id")
            .is_none());
    }

    #[test]
    fn type_schema_includes_inherited_fields() {
        // A child type extends a parent type. The projected schema must include
        // both the parent's own field and the child's own field.
        const PARENT_TID: &str = "00000000-0000-4000-8000-000000000001";
        const CHILD_TID: &str = "00000000-0000-4000-8000-000000000002";

        let parent = make_type(PARENT_TID, vec![assignment(&fid(1), 0, true)]);
        // child declares its own field at order 1 and inherits parent's field at order 0.
        let mut child = make_type(CHILD_TID, vec![assignment(&fid(2), 1, false)]);
        child.extends_type_id = Some(PARENT_TID.to_string());
        child.extends_type_version = Some(1); // matches make_type's default version: 1

        // Both fields must be in the flat Package.fields list; resolve_field searches it.
        let store = store_with_types(
            vec![
                field(&fid(1), "parent_field", FieldType::string()),
                field(&fid(2), "child_field", FieldType::string()),
            ],
            vec![parent, child],
        );

        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: CHILD_TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();

        let props = result.schema["properties"].as_object().unwrap();
        assert!(
            props.contains_key("parent_field"),
            "inherited parent_field missing from schema: {:?}",
            props.keys().collect::<Vec<_>>()
        );
        assert!(
            props.contains_key("child_field"),
            "own child_field missing from schema: {:?}",
            props.keys().collect::<Vec<_>>()
        );
        assert_eq!(props.len(), 2, "expected exactly 2 properties");
        assert!(
            result.diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            result.diagnostics
        );
        // Parent field is required, child field is not.
        assert_eq!(result.schema["required"], json!(["parent_field"]));
    }

    /// RFC-039: an inline-composite list Field expands its range Type into a
    /// nested object schema — the successor of the retired FieldGroup
    /// projection.
    #[test]
    fn type_schema_expands_inline_composite_range() {
        use srs_core::types::field_type::ExactTypeRef;

        const RANGE_TID: &str = "00000000-0000-4000-8000-0000000000bb";

        let heading = field(&fid(0), "heading", FieldType::string());
        let columns = field(&fid(1), "columns", FieldType::text());
        let cells = field(&fid(2), "cells", FieldType::text());
        let mut rows = field(
            &fid(3),
            "rows",
            FieldType::inline_ref(ExactTypeRef {
                type_id: RANGE_TID.to_string(),
                type_version: 1,
            })
            .into_list(),
        );
        rows.description = String::new();

        let mut range_rt = make_type(
            RANGE_TID,
            vec![assignment(&fid(1), 0, false), assignment(&fid(2), 1, true)],
        );
        range_rt.name = "row".to_string();
        let rt = make_type(
            TID,
            vec![assignment(&fid(0), 0, false), assignment(&fid(3), 1, false)],
        );

        let store = store_with_types(vec![heading, columns, cells, rows], vec![rt, range_rt]);
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();

        let props = result.schema["properties"].as_object().unwrap();
        // Flat field still present alongside the composite.
        assert!(props.contains_key("heading"));

        let rows_prop = &props["rows"];
        assert_eq!(rows_prop["type"], "array", "list-cardinality → array");
        let items = &rows_prop["items"];
        assert_eq!(items["type"], "object");
        assert_eq!(items["additionalProperties"], json!(false));
        assert_eq!(items["x-srs-range-type-id"], json!(RANGE_TID));

        let item_props = items["properties"].as_object().unwrap();
        assert!(item_props.contains_key("columns"));
        assert!(item_props.contains_key("cells"));

        // Required sub-field surfaces in the item object's `required`.
        let item_required = items["required"].as_array().unwrap();
        assert!(item_required.iter().any(|v| v == "cells"));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // heading(order=0) gets position 1; rows(order=1) gets position 2.
        assert_eq!(rows_prop["x-srs-order"], json!(2));
    }

    /// [R2b]: the schema's property keys are `Field.name` verbatim — no case
    /// mapping, no slugging, no id substitution.
    #[test]
    fn domain_type_keys_are_field_name_verbatim() {
        let names = ["title", "decision_statement", "Mixed_Case-name", "città"];
        let mut fields = Vec::new();
        let mut assignments = Vec::new();
        for (i, name) in names.iter().enumerate() {
            fields.push(field(&fid(i as u8), name, FieldType::string()));
            assignments.push(assignment(&fid(i as u8), i as u32, false));
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

        let keys: Vec<&String> = result.schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(keys, names.iter().collect::<Vec<_>>());
    }

    /// RFC-039: `x-srs-field-id` is retired — no property anywhere in the
    /// projected schema (including expanded composite interiors) carries it.
    #[test]
    fn no_x_srs_field_id_emitted() {
        use srs_core::types::field_type::ExactTypeRef;

        const RANGE_TID: &str = "00000000-0000-4000-8000-0000000000bb";
        let inner = field(&fid(1), "cells", FieldType::text());
        let rows = field(
            &fid(2),
            "rows",
            FieldType::inline_ref(ExactTypeRef {
                type_id: RANGE_TID.to_string(),
                type_version: 1,
            })
            .into_list(),
        );
        let title = field(&fid(3), "title", FieldType::string());
        let range_rt = make_type(RANGE_TID, vec![assignment(&fid(1), 0, false)]);
        let rt = make_type(
            TID,
            vec![assignment(&fid(3), 0, true), assignment(&fid(2), 1, false)],
        );
        let store = store_with_types(vec![inner, rows, title], vec![rt, range_rt]);

        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();

        fn assert_no_field_id(value: &Value) {
            match value {
                Value::Object(map) => {
                    assert!(
                        !map.contains_key("x-srs-field-id"),
                        "x-srs-field-id must not be emitted anywhere: {map:?}"
                    );
                    map.values().for_each(assert_no_field_id);
                }
                Value::Array(items) => items.iter().for_each(assert_no_field_id),
                _ => {}
            }
        }
        assert_no_field_id(&result.schema);
    }

    /// The projected schema describes a record's `fieldValues` object only —
    /// no envelope keys (instanceId, typeId, fieldMeta, …) appear as
    /// properties, and `additionalProperties: false` seals the field set.
    #[test]
    fn projected_schema_covers_field_values_only_not_envelope() {
        let store = store_with(
            vec![
                field(&fid(1), "title", FieldType::string()),
                field(&fid(2), "body", FieldType::markdown()),
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

        let props = result.schema["properties"].as_object().unwrap();
        let keys: Vec<&String> = props.keys().collect();
        assert_eq!(keys, ["title", "body"].iter().collect::<Vec<_>>());
        for envelope_key in [
            "instanceId",
            "typeId",
            "typeVersion",
            "typeNamespace",
            "typeName",
            "fieldValues",
            "fieldMeta",
            "lifecycleState",
            "tags",
            "createdAt",
            "updatedAt",
        ] {
            assert!(
                !props.contains_key(envelope_key),
                "envelope key {envelope_key} must not appear in the fieldValues schema"
            );
        }
        assert_eq!(result.schema["additionalProperties"], json!(false));
    }

    #[test]
    fn type_schema_no_groups_retains_field_order() {
        // Regression guard: a type with three fields and no groups still gets
        // 1-based positional x-srs-order on each field.
        let store = store_with(
            vec![
                field(&fid(1), "a", FieldType::string()),
                field(&fid(2), "b", FieldType::string()),
                field(&fid(3), "c", FieldType::string()),
            ],
            make_type(
                TID,
                vec![
                    assignment(&fid(1), 0, false),
                    assignment(&fid(2), 1, false),
                    assignment(&fid(3), 2, false),
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

        let props = result.schema["properties"].as_object().unwrap();
        assert_eq!(props["a"]["x-srs-order"], json!(1));
        assert_eq!(props["b"]["x-srs-order"], json!(2));
        assert_eq!(props["c"]["x-srs-order"], json!(3));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    /// ADR-043 Decision 4 / RFC-039 "same rule read in two directions": the
    /// value grammar (`validate_value`/`validate_field_values_map`) and the
    /// emitted editor schema must agree on what they accept over scalars,
    /// lists, and a recursive composite — drift here means a second grammar.
    #[test]
    fn value_grammar_and_emitted_schema_agree() {
        use srs_core::types::field_type::ExactTypeRef;
        use srs_core::validation::value_shape::{validate_field_values_map, EffectiveField};

        const RANGE_TID: &str = "00000000-0000-4000-8000-0000000000bb";

        let title = field(&fid(0), "title", FieldType::string());
        let cells = field(&fid(2), "cells", FieldType::string().into_list());
        let rows = field(
            &fid(3),
            "rows",
            FieldType::inline_ref(ExactTypeRef {
                type_id: RANGE_TID.to_string(),
                type_version: 1,
            })
            .into_list(),
        );
        let mut range_rt = make_type(RANGE_TID, vec![assignment(&fid(2), 0, true)]);
        range_rt.name = "row".to_string();
        let rt = make_type(
            TID,
            vec![assignment(&fid(0), 0, true), assignment(&fid(3), 1, false)],
        );

        let store = store_with_types(vec![title, cells, rows], vec![rt, range_rt]);
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();
        let compiled = jsonschema::validator_for(&result.schema).unwrap();

        let package = store.load_package().unwrap();
        let rt = package.resolve_type(TID, 1).unwrap().clone();
        let effective: Vec<EffectiveField> = package.resolved_effective_fields(&rt).unwrap();

        let cases: Vec<(serde_json::Value, bool)> = vec![
            (json!({"title": "ok", "rows": [{"cells": ["a"]}]}), true),
            (json!({"title": "ok", "rows": []}), true),
            (json!({"title": 42, "rows": []}), false),
            (
                json!({"title": "ok", "rows": [{"cells": "not-a-list"}]}),
                false,
            ),
            (json!({"title": "ok", "rows": [{"mystery": ["a"]}]}), false),
            (json!({"title": "ok", "rows": {"cells": ["a"]}}), false),
        ];
        for (value_map, expect_valid) in cases {
            let schema_ok = compiled.is_valid(&value_map);
            let mut diags = Vec::new();
            validate_field_values_map(
                "",
                value_map.as_object().unwrap(),
                &effective,
                &package,
                &mut diags,
            );
            let grammar_ok = diags.is_empty();
            assert_eq!(schema_ok, expect_valid, "schema verdict for {value_map}");
            assert_eq!(
                grammar_ok, expect_valid,
                "grammar verdict for {value_map}: {diags:?}"
            );
        }
    }
}
