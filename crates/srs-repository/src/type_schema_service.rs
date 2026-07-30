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
use crate::package::{EffectiveFieldsAndGroups, OrderedGroup};
use crate::package_service::GetTypeResult;
use crate::package_service::{get_type_by_id, get_type_by_id_latest};
use crate::store::RepositoryStore;
use serde_json::{json, Map, Value};
use srs_core::types::field::{Datatype, Field, RefMode, StringFormat};
use srs_core::types::record_type::{FieldAssignment, FieldGroup};

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
    // (own + inherited), sorted by order. Also computes each group's 1-based
    // position in the merged field+group sequence so x-srs-order is consistent
    // across fields and groups in the same schema.
    let EffectiveFieldsAndGroups {
        fields: assignments,
        field_positions,
        groups: ordered_groups,
    } = package.effective_fields_and_groups(&record_type)?;

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

        let mut property = field_to_property(&field, fa, &mut diagnostics);
        // Use the merged position (1-based) as x-srs-order so that fields and groups
        // share a single consistent position namespace. When no groups are present,
        // field_positions[i] == i+1, giving the same behaviour as before.
        let merged_pos = field_positions.get(idx).copied().unwrap_or(idx + 1);
        if let Some(obj) = property.as_object_mut() {
            obj.insert("x-srs-order".into(), json!(merged_pos));
        }
        if fa.required {
            required.push(Value::String(field.name.clone()));
        }
        properties.insert(field.name.clone(), property);
    }

    // ext:field-groups (RFC-007) — emit each repeatable/composite group as an
    // array (or object) property so schema-driven editors can render it. The
    // group's `groupId` is the property key; sub-fields become the item schema.
    // merged_position is the 1-based position in the combined field+group sequence,
    // ensuring groups and fields share a single consistent x-srs-order namespace.
    for OrderedGroup {
        group,
        merged_position,
    } in &ordered_groups
    {
        let property = field_group_to_property(group, &package, *merged_position, &mut diagnostics);
        if group.required {
            required.push(Value::String(group.group_id.clone()));
        }
        properties.insert(group.group_id.clone(), property);
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

    // RFC-032: the shape comes from the `fieldType` facets. A list wraps the
    // single-value shape in an array, which is how the pre-RFC-032
    // `multiselect` case used to be special-cased.
    let mut value_shape = Map::new();
    insert_value_shape(&mut value_shape, field, diagnostics);
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

    prop.insert("x-srs-order".into(), json!(assignment.order));
    prop.insert("x-srs-field-id".into(), json!(field.id));

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

/// Map a field group (ext:field-groups, RFC-007) to a draft-07 property schema.
///
/// A repeatable group becomes an `array` of objects; a non-repeatable group an
/// `object`. The group's sub-fields become the item object's properties. The
/// `x-srs-group-id`, `x-srs-composite-renderer`, and `x-srs-repeatable` hints let
/// a schema-driven editor pick the right widget (e.g. a table grid).
///
/// `merged_position` is the 1-based position of this group in the combined
/// field+group sequence; it is written to `x-srs-order` so editors can correctly
/// interleave groups and fields.
fn field_group_to_property(
    group: &FieldGroup,
    package: &crate::package::Package,
    merged_position: usize,
    diagnostics: &mut Vec<String>,
) -> Value {
    let mut item_props = Map::new();
    let mut item_required = Vec::new();
    for fa in &group.fields {
        match package.resolve_field(&fa.field_id) {
            Some(field) => {
                let prop = field_to_property(&field.clone(), fa, diagnostics);
                if fa.required {
                    item_required.push(Value::String(field.name.clone()));
                }
                item_props.insert(field.name.clone(), prop);
            }
            None => diagnostics.push(format!(
                "field group '{}' references unknown fieldId '{}'; skipped",
                group.group_id, fa.field_id
            )),
        }
    }

    let item = json!({
        "type": "object",
        "properties": Value::Object(item_props),
        "required": Value::Array(item_required),
        "additionalProperties": false
    });

    let mut prop = Map::new();
    if group.repeatable {
        prop.insert("type".into(), json!("array"));
        prop.insert("items".into(), item);
        if let Some(min) = group.min_items {
            prop.insert("minItems".into(), json!(min));
        }
        if let Some(max) = group.max_items {
            prop.insert("maxItems".into(), json!(max));
        }
    } else {
        // Non-repeatable group: a single object with the same item shape.
        if let Value::Object(obj) = item {
            prop.extend(obj);
        }
    }

    if let Some(label) = group.label.clone().filter(|s| !s.is_empty()) {
        prop.insert("title".into(), json!(label));
    }
    if let Some(desc) = group.description.clone().filter(|s| !s.is_empty()) {
        prop.insert("description".into(), json!(desc));
    }
    prop.insert("x-srs-order".into(), json!(merged_position));
    prop.insert("x-srs-group-id".into(), json!(group.group_id));
    prop.insert("x-srs-repeatable".into(), json!(group.repeatable));
    if let Some(renderer) = &group.composite_renderer {
        prop.insert("x-srs-composite-renderer".into(), json!(renderer));
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
                if let Some(min) = c.minimum {
                    target.insert("minimum".into(), json!(min));
                }
                if let Some(max) = c.maximum {
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
                // An inline ref carries a nested object. The editor projection
                // does not expand the range here — that is the standards
                // projection's job; the range is named so a form renderer can
                // fetch it.
                RefMode::Inline => {
                    target.insert("type".into(), json!("object"));
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
    use std::collections::HashMap;
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
            ai_guidance: AiGuidance::default(),
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
            container: None,
            container_index: None,
            federation_path: None,
            upstream_package: None,
            federation_events_path: None,
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
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
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        MemoryStore::new(manifest, package)
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
            extra: HashMap::new(),
            source_documents_path: None,
            source_document_index: None,
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
            protocols: vec![],
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

            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
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
        let mut diagnostics = Vec::new();

        let prop = field_to_property(&f, &a, &mut diagnostics);

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
        let mut diagnostics = Vec::new();

        let prop = field_to_property(&f, &a, &mut diagnostics);

        assert!(prop.get("x-srs-description").is_none());
        assert!(prop.get("x-srs-instructions").is_none());

        // Some("") must also omit the key — empty strings carry no help text.
        f.instructions = Some(String::new());
        let prop = field_to_property(&f, &a, &mut diagnostics);
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
        assert_eq!(
            reparsed["properties"]["title"]["x-srs-field-id"],
            json!(fid(1))
        );
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

    #[test]
    fn type_schema_emits_field_groups_with_composite_renderer() {
        let heading = field(&fid(0), "heading", FieldType::string());
        let columns = field(&fid(1), "columns", FieldType::text());
        let rows = field(&fid(2), "rows", FieldType::text());

        let mut rt = make_type(TID, vec![assignment(&fid(0), 0, false)]);
        rt.field_groups = Some(vec![FieldGroup {
            group_id: "tables".to_string(),
            order: 1,
            fields: vec![assignment(&fid(1), 0, false), assignment(&fid(2), 1, true)],
            label: Some("Tables".to_string()),
            description: None,
            required: false,
            repeatable: true,
            min_items: None,
            max_items: None,
            composite_renderer: Some("table".to_string()),
        }]);

        let store = store_with(vec![heading, columns, rows], rt);
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();

        let props = result.schema["properties"].as_object().unwrap();
        // Flat field still present alongside the group.
        assert!(props.contains_key("heading"));

        let tables = &props["tables"];
        assert_eq!(tables["type"], "array", "repeatable group → array");
        assert_eq!(tables["x-srs-composite-renderer"], "table");
        assert_eq!(tables["x-srs-repeatable"], true);
        assert_eq!(tables["x-srs-group-id"], "tables");
        assert_eq!(tables["title"], "Tables");

        let item_props = tables["items"]["properties"].as_object().unwrap();
        assert!(item_props.contains_key("columns"));
        assert!(item_props.contains_key("rows"));

        // Required sub-field surfaces in the item object's `required`.
        let item_required = tables["items"]["required"].as_array().unwrap();
        assert!(item_required.iter().any(|v| v == "rows"));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        // heading(order=0) gets position 1; tables(order=1) gets position 2 in merged sort.
        assert_eq!(tables["x-srs-order"], json!(2));
    }

    #[test]
    fn type_schema_group_order_is_positional_not_raw() {
        // group(raw order=0) with fields at order=1 and order=2, no fieldOrder.
        // Merged sort: group(order=0) < field(order=1) → group gets position 1.
        // This is the bug from issue #148: previously the group would get x-srs-order=0
        // (raw group.order), colliding with field positions 1 and 2.
        let f1 = field(&fid(1), "alpha", FieldType::string());
        let f2 = field(&fid(2), "beta", FieldType::string());
        let mut rt = make_type(
            TID,
            vec![assignment(&fid(1), 1, false), assignment(&fid(2), 2, false)],
        );
        rt.field_groups = Some(vec![FieldGroup {
            group_id: "items".to_string(),
            order: 0,
            fields: vec![],
            label: None,
            description: None,
            required: false,
            repeatable: true,
            min_items: None,
            max_items: None,
            composite_renderer: None,
        }]);

        let store = store_with(vec![f1, f2], rt);
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();

        let props = result.schema["properties"].as_object().unwrap();
        // group at order=0 sorts before fields at order=1 and order=2 → position 1.
        assert_eq!(
            props["items"]["x-srs-order"],
            json!(1),
            "group at raw order=0 must get merged position 1, not raw 0"
        );
        // fields get positions 2 and 3.
        assert_eq!(props["alpha"]["x-srs-order"], json!(2));
        assert_eq!(props["beta"]["x-srs-order"], json!(3));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    }

    #[test]
    fn type_schema_field_order_interleaves_groups() {
        // fieldOrder: [field_a, group_id, field_b] → positions 1, 2, 3.
        let fa_field = field(&fid(1), "field_a", FieldType::string());
        let fb_field = field(&fid(2), "field_b", FieldType::string());
        let mut rt = make_type(
            TID,
            vec![assignment(&fid(1), 0, false), assignment(&fid(2), 1, false)],
        );
        rt.field_groups = Some(vec![FieldGroup {
            group_id: "grp".to_string(),
            order: 99,
            fields: vec![],
            label: None,
            description: None,
            required: false,
            repeatable: false,
            min_items: None,
            max_items: None,
            composite_renderer: None,
        }]);
        rt.field_order = Some(vec![fid(1), "grp".to_string(), fid(2)]);

        let store = store_with(vec![fa_field, fb_field], rt);
        let result = type_schema(
            &store,
            TypeSchemaInput {
                type_id: TID.to_string(),
                type_version: None,
            },
        )
        .unwrap();

        let props = result.schema["properties"].as_object().unwrap();
        assert_eq!(props["field_a"]["x-srs-order"], json!(1));
        assert_eq!(props["grp"]["x-srs-order"], json!(2));
        assert_eq!(props["field_b"]["x-srs-order"], json!(3));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
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
}
