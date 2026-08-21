//! RFC-035 — the JSON Schema projection: meta-model `Type` records → a neutral
//! IR → JSON Schema 2020-12.
//!
//! This is the Rust half of RFC-035's contract. The normative source→schema
//! mapping is `docs/schema/2.0/projection-rules.md` in the spec repo, and the
//! reference implementation is `scripts/lib/schema-emitter.mjs` +
//! `scripts/lib/rfc-032-fieldtype.mjs::projectField`. A conforming emitter MUST
//! produce **byte-identical** output, so this module is held to the same
//! committed goldens (`tests/rfc-035/goldens/`) as the reference emitter — see
//! `tests/rfc_035_parity.rs`.
//!
//! **Byte-parity is a serialization constraint, not just a content one.** The
//! projection rules pin key order (top-level, `$defs` bag, and intra-fragment),
//! and this module predates the workspace enabling serde_json's
//! `preserve_order` (ADR-043, amending ADR-017/036/037 — determinism now comes
//! from canonical types + the .srsj canonicalize-on-write step). So the
//! schema is modelled as **typed structs whose field declaration order is the
//! emitted key order**, and ordered maps are `OrderedMap`, a `Vec`-backed map
//! that serializes in insertion order. Nothing here builds a
//! `serde_json::Value` object, which would silently re-sort.
//!
//! Two artifacts are deliberately *not* produced here:
//!
//! * `aiGuidance` — it has no JSON Schema equivalent and is the product
//!   differentiator; it belongs alongside the validation schema, not inside it.
//! * `editorHint` / `FieldAssignment.order` — presentation. Those live in the
//!   editor-facing projection (`srs_repository::type_schema_service`), which is
//!   a separate artifact by design (srs-rust#770).

use serde::{Serialize, Serializer};
use srs_core::types::field::{Datatype, Field, FieldType, RefMode, StringFormat, ValueDomain};
use srs_core::types::record_type::RecordType;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Ordered map — insertion order is the wire contract.
// ---------------------------------------------------------------------------

/// A JSON object whose keys serialize in **insertion** order.
///
/// `serde_json::Map` is `BTreeMap`-backed in this workspace (ADR-017 relies on
/// that), so it cannot carry the pinned orderings the projection rules require.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OrderedMap<T>(Vec<(String, T)>);

impl<T> OrderedMap<T> {
    pub fn new() -> Self {
        OrderedMap(Vec::new())
    }

    /// Insert or replace. JS object assignment (`obj[key] = v`) collapses a
    /// repeated key; appending here instead would emit a duplicate key and
    /// produce a JSON document no parser agrees on.
    pub fn insert(&mut self, key: impl Into<String>, value: T) {
        let key = key.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => self.0.push((key, value)),
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    fn fill(&mut self, index: usize, value: T) {
        self.0[index].1 = value;
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }
}

impl<T: Default> OrderedMap<T> {
    /// Reserve a slot so a later nested insertion lands *after* it — the
    /// pre-order-DFS discipline the `$defs` bag order requires. The placeholder
    /// is always overwritten by [`OrderedMap::fill`] before serialization.
    fn reserve(&mut self, key: impl Into<String>) -> usize {
        self.0.push((key.into(), T::default()));
        self.0.len() - 1
    }
}

impl<T: Serialize> Serialize for OrderedMap<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

// ---------------------------------------------------------------------------
// The emitted shapes. Field declaration order == emitted key order.
// ---------------------------------------------------------------------------

/// `additionalProperties` is either a boolean or a subschema (`map` ranges).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AdditionalProperties {
    Bool(bool),
    Schema(Box<SchemaNode>),
}

/// One property's projected subschema.
///
/// Every projection row in `projectField` emits a subset of these keys, and the
/// declaration order below is the union of those rows in their pinned order. No
/// single row uses conflicting keys, so one struct expresses them all without
/// reordering any row.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SchemaNode {
    #[serde(rename = "$ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<&'static str>,
    #[serde(rename = "contentMediaType", skip_serializing_if = "Option::is_none")]
    pub content_media_type: Option<&'static str>,
    #[serde(rename = "minLength", skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u32>,
    #[serde(rename = "maxLength", skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_js_number"
    )]
    pub minimum: Option<serde_json::Number>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_js_number"
    )]
    pub maximum: Option<serde_json::Number>,
    #[serde(rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<Vec<String>>,
    #[serde(rename = "x-srs-range-type", skip_serializing_if = "Option::is_none")]
    pub range_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<SchemaNode>>,
    #[serde(rename = "minItems", skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(rename = "maxItems", skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    #[serde(
        rename = "additionalProperties",
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_properties: Option<AdditionalProperties>,
    /// From `FieldAssignment.displayLabel` — appended last (projection rules,
    /// "Per-node key order").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A Type's object body — used both for a top-level entity's `properties`
/// block and for each inline range's `$def`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ObjectBody {
    #[serde(rename = "type")]
    pub ty: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(rename = "additionalProperties")]
    pub additional_properties: bool,
    pub properties: OrderedMap<SchemaNode>,
}

/// A complete JSON Schema 2020-12 definition schema for one Type.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntitySchema {
    #[serde(rename = "$schema")]
    pub schema: &'static str,
    #[serde(rename = "$id")]
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub ty: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(rename = "additionalProperties")]
    pub additional_properties: bool,
    pub properties: OrderedMap<SchemaNode>,
    #[serde(rename = "$defs", skip_serializing_if = "Option::is_none")]
    pub defs: Option<OrderedMap<ObjectBody>>,
}

/// The generated-schema bundle envelope (RFC-035 Change H). Distinct artifact
/// from RFC-033's `package-bundle.json` (the `.srsj` record bundle).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SchemaBundle {
    #[serde(rename = "dataModelRevision")]
    pub data_model_revision: u64,
    pub schemas: OrderedMap<EntitySchema>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// A Type named by an entity list or by a `ref` range is not in the package.
    UnknownType(String),
    /// A `ref` field's `rangeType.typeId` does not resolve in the package.
    UnresolvedRange { field: String, type_id: String },
    /// A Type's FieldAssignment names a `fieldId` the package does not define.
    UnknownField { type_name: String, field_id: String },
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::UnknownType(name) => {
                write!(f, "json-schema projection: unknown type '{name}'")
            }
            ProjectionError::UnresolvedRange { field, type_id } => write!(
                f,
                "json-schema projection: field '{field}' has an unresolved rangeType typeId '{type_id}'"
            ),
            ProjectionError::UnknownField {
                type_name,
                field_id,
            } => write!(
                f,
                "json-schema projection: type '{type_name}' references unknown fieldId '{field_id}'"
            ),
        }
    }
}

impl std::error::Error for ProjectionError {}

// ---------------------------------------------------------------------------
// Naming (RFC-035 Change D/E)
// ---------------------------------------------------------------------------

/// The committed override table: metamodel `Field.name` → JSON key, where the
/// mechanical projection differs from the intended wire key.
const NAME_OVERRIDES: &[(&str, &str)] = &[("assignment_default_value", "defaultValue")];

/// snake_case → lowerCamelCase, with the override table applied first.
/// Deterministic and injective over the in-scope metamodel field names.
pub fn json_key(field_name: &str) -> String {
    if let Some((_, override_key)) = NAME_OVERRIDES.iter().find(|(k, _)| *k == field_name) {
        return (*override_key).to_string();
    }
    let mut out = String::with_capacity(field_name.len());
    let mut upper_next = false;
    for c in field_name.chars() {
        if c == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// The `$defs` key for an inline range — an injective function of
/// `(namespace, name, version)`, spelled `<namespace>__<name>__v<version>`.
///
/// Emitter-owned (RFC-032 Change G / RFC-035 Change D): closure comparisons
/// resolve `$ref`s away rather than comparing this spelling.
pub fn range_def_key(namespace: &str, name: &str, version: u32) -> String {
    format!("{namespace}__{name}__v{version}")
}

/// The namespace whose `field`/`type` Types are the frozen meta-model entities.
const METAMODEL_NAMESPACE: &str = "com.semanticops.srs";

/// `$id` for a Type (RFC-035 Change C).
///
/// The two frozen meta-model entities keep their reserved data-model-line ids;
/// every other Type uses the RFC-004 generated-schema template. The reserved
/// ids are keyed on `(namespace, name)` rather than name alone, so a domain
/// Type that happens to be called `field` cannot claim `2.0/field.json`.
pub fn schema_id(namespace: &str, name: &str, version: u32) -> String {
    if namespace == METAMODEL_NAMESPACE && (name == "field" || name == "type") {
        return format!("https://srs.semanticops.com/schema/2.0/{name}.json");
    }
    format!("https://srs.semanticops.com/schema/domain/{namespace}/{name}/{version}.json")
}

// ---------------------------------------------------------------------------
// The projection
// ---------------------------------------------------------------------------

/// A resolved package view: the Types and Fields the projection walks.
pub struct ProjectionContext<'a> {
    types_by_id: HashMap<&'a str, &'a RecordType>,
    types_by_name: HashMap<&'a str, &'a RecordType>,
    fields_by_id: HashMap<&'a str, &'a Field>,
}

impl<'a> ProjectionContext<'a> {
    /// Index a package's Types and Fields for projection.
    ///
    /// `types_by_name` is a convenience for addressing entities by name (the
    /// bundle's `entities` list); it is **last-wins**, matching the reference
    /// emitter's plain assignment. Name is not a unique key, so every
    /// correctness-bearing lookup goes through `types_by_id` instead — which is
    /// how `ref` range resolution finds a range's body. (The reference emitter
    /// resolves that body by *name*, so in a package with two same-named Types
    /// in different namespaces it emits the wrong body; that is an upstream bug
    /// to fix in `schema-emitter.mjs`, not a behaviour to copy.)
    pub fn new(types: &'a [RecordType], fields: &'a [Field]) -> Self {
        let mut types_by_id = HashMap::new();
        let mut types_by_name = HashMap::new();
        for t in types {
            types_by_id.insert(t.id.as_str(), t);
            types_by_name.insert(t.name.as_str(), t);
        }
        let fields_by_id = fields.iter().map(|f| (f.id.as_str(), f)).collect();
        ProjectionContext {
            types_by_id,
            types_by_name,
            fields_by_id,
        }
    }

    pub fn type_by_name(&self, name: &str) -> Option<&'a RecordType> {
        self.types_by_name.get(name).copied()
    }

    fn require_type_by_name(&self, name: &str) -> Result<&'a RecordType, ProjectionError> {
        self.types_by_name
            .get(name)
            .copied()
            .ok_or_else(|| ProjectionError::UnknownType(name.to_string()))
    }
}

/// Project the Type named `type_name` into a complete JSON Schema 2020-12
/// definition schema. Prefer [`emit_entity_for`] when the caller has already
/// resolved the Type — a name does not identify one.
pub fn emit_entity(
    ctx: &ProjectionContext<'_>,
    type_name: &str,
) -> Result<EntitySchema, ProjectionError> {
    emit_entity_for(ctx, ctx.require_type_by_name(type_name)?)
}

/// Project an already-resolved Type.
pub fn emit_entity_for(
    ctx: &ProjectionContext<'_>,
    record_type: &RecordType,
) -> Result<EntitySchema, ProjectionError> {
    let mut defs: OrderedMap<ObjectBody> = OrderedMap::new();
    // Walking the entity fills `defs` in pre-order DFS by first reference.
    let body = emit_body(ctx, record_type, &mut defs)?;

    Ok(EntitySchema {
        schema: "https://json-schema.org/draft/2020-12/schema",
        id: schema_id(
            &record_type.namespace,
            &record_type.name,
            record_type.version,
        ),
        description: Some(record_type.description.clone()).filter(|d| !d.is_empty()),
        ty: "object",
        required: body.required,
        additional_properties: false,
        properties: body.properties,
        defs: if defs.is_empty() { None } else { Some(defs) },
    })
}

/// Emit a Type's object body — used for both entities and their value-object
/// `$defs`.
fn emit_body(
    ctx: &ProjectionContext<'_>,
    record_type: &RecordType,
    defs: &mut OrderedMap<ObjectBody>,
) -> Result<ObjectBody, ProjectionError> {
    let mut assignments: Vec<_> = record_type.fields.iter().collect();
    assignments.sort_by_key(|a| a.order);

    let mut properties = OrderedMap::new();
    let mut required = Vec::new();

    for assignment in assignments {
        let field = ctx.fields_by_id.get(assignment.field_id.as_str()).ok_or(
            ProjectionError::UnknownField {
                type_name: record_type.name.clone(),
                field_id: assignment.field_id.clone(),
            },
        )?;
        let key = json_key(&field.name);
        let mut node = render_node(ctx, field, defs)?;
        // FieldAssignment.displayLabel → title (presentation annotation).
        if let Some(label) = assignment.display_label.as_ref().filter(|l| !l.is_empty()) {
            node.title = Some(label.clone());
        }
        if assignment.required {
            required.push(key.clone());
        }
        properties.insert(key, node);
    }

    Ok(ObjectBody {
        ty: "object",
        required: if required.is_empty() {
            None
        } else {
            Some(required)
        },
        additional_properties: false,
        properties,
    })
}

/// Render one Field's `fieldType`. For an inline `ref`, the range's `$def` is
/// ensured first (pre-order DFS); a `reference` ref emits the id shape and
/// contributes no `$def`.
fn render_node(
    ctx: &ProjectionContext<'_>,
    field: &Field,
    defs: &mut OrderedMap<ObjectBody>,
) -> Result<SchemaNode, ProjectionError> {
    let ft = &field.field_type;
    if ft.datatype == Datatype::Ref {
        let range = ft
            .range_type
            .as_ref()
            .ok_or_else(|| ProjectionError::UnresolvedRange {
                field: field.name.clone(),
                type_id: "<absent>".to_string(),
            })?;
        let target = ctx.types_by_id.get(range.type_id.as_str()).copied().ok_or(
            ProjectionError::UnresolvedRange {
                field: field.name.clone(),
                type_id: range.type_id.clone(),
            },
        )?;
        let key = range_def_key(&target.namespace, &target.name, range.type_version);
        if ft.effective_mode() == RefMode::Inline {
            ensure_def(ctx, target, &key, defs)?;
        }
        return Ok(project_field(
            ft,
            Some(&RangeParts {
                namespace: &target.namespace,
                name: &target.name,
                version: range.type_version,
                def_key: &key,
            }),
        ));
    }
    Ok(project_field(ft, None))
}

/// Ensure an inline range's `$def` body exists, **reserving its slot before
/// recursing** so a parent `$def` always precedes its nested ones.
fn ensure_def(
    ctx: &ProjectionContext<'_>,
    target: &RecordType,
    key: &str,
    defs: &mut OrderedMap<ObjectBody>,
) -> Result<(), ProjectionError> {
    if defs.contains_key(key) {
        return Ok(());
    }
    let slot = defs.reserve(key);
    let body = emit_body(ctx, target, defs)?;
    defs.fill(slot, body);
    Ok(())
}

struct RangeParts<'a> {
    namespace: &'a str,
    name: &'a str,
    version: u32,
    def_key: &'a str,
}

/// RFC-032 Change G — a `fieldType` → a JSON Schema fragment.
///
/// The Rust twin of `scripts/lib/rfc-032-fieldtype.mjs::projectField`, row for
/// row. `range` is supplied by the caller once `rangeType.typeId` has been
/// resolved against the package.
fn project_field(ft: &FieldType, range: Option<&RangeParts<'_>>) -> SchemaNode {
    let core = match ft.datatype {
        Datatype::Ref => {
            let parts = range.expect("a ref node is only rendered with a resolved range");
            match ft.effective_mode() {
                RefMode::Reference => SchemaNode {
                    ty: Some("string"),
                    format: Some("uuid"),
                    range_type: Some(format!(
                        "{}/{}@{}",
                        parts.namespace, parts.name, parts.version
                    )),
                    ..SchemaNode::default()
                },
                RefMode::Inline => SchemaNode {
                    reference: Some(format!("#/$defs/{}", parts.def_key)),
                    ..SchemaNode::default()
                },
            }
        }
        Datatype::Map => SchemaNode {
            ty: Some("object"),
            additional_properties: Some(match ft.value_range {
                Some(srs_core::types::field::MapValueRange::Open) | None => {
                    AdditionalProperties::Bool(true)
                }
                Some(scalar) => AdditionalProperties::Schema(Box::new(project_scalar(
                    &FieldType::new(map_value_datatype(scalar)),
                ))),
            }),
            ..SchemaNode::default()
        },
        // Deliberately lossy: a broad permissible value. Conformance to the
        // depended-on field's type is a validation obligation, not something
        // JSON Schema can express here.
        Datatype::Dependent => SchemaNode::default(),
        _ => project_scalar(ft),
    };

    if ft.is_list() {
        return SchemaNode {
            ty: Some("array"),
            items: Some(Box::new(core)),
            min_items: ft.min_items,
            max_items: ft.max_items,
            ..SchemaNode::default()
        };
    }
    core
}

fn map_value_datatype(range: srs_core::types::field::MapValueRange) -> Datatype {
    use srs_core::types::field::MapValueRange as M;
    match range {
        M::String | M::Open => Datatype::String,
        M::Number => Datatype::Number,
        M::Integer => Datatype::Integer,
        M::Boolean => Datatype::Boolean,
        M::Date => Datatype::Date,
        M::DateTime => Datatype::DateTime,
    }
}

/// The portable scalar table + `format` + `constraints` + closed-domain `enum`.
fn project_scalar(ft: &FieldType) -> SchemaNode {
    let mut node = match ft.datatype {
        Datatype::String => SchemaNode {
            ty: Some("string"),
            ..SchemaNode::default()
        },
        Datatype::Number => SchemaNode {
            ty: Some("number"),
            ..SchemaNode::default()
        },
        Datatype::Integer => SchemaNode {
            ty: Some("integer"),
            ..SchemaNode::default()
        },
        Datatype::Boolean => SchemaNode {
            ty: Some("boolean"),
            ..SchemaNode::default()
        },
        Datatype::Date => SchemaNode {
            ty: Some("string"),
            format: Some("date"),
            ..SchemaNode::default()
        },
        Datatype::DateTime => SchemaNode {
            ty: Some("string"),
            format: Some("date-time"),
            ..SchemaNode::default()
        },
        // Composite datatypes never reach here — `project_field` dispatches
        // them before calling this.
        Datatype::Ref | Datatype::Dependent | Datatype::Map => SchemaNode::default(),
    };

    if ft.datatype == Datatype::String {
        match ft.format {
            Some(StringFormat::Markdown) => node.content_media_type = Some("text/markdown"),
            Some(StringFormat::Uri) => node.format = Some("uri"),
            Some(StringFormat::Uuid) => node.format = Some("uuid"),
            Some(StringFormat::Email) => node.format = Some("email"),
            // `plain` carries no JSON Schema keyword.
            Some(StringFormat::Plain) | None => {}
        }
    }

    if let Some(c) = &ft.constraints {
        node.min_length = c.min_length;
        node.max_length = c.max_length;
        node.pattern = c.pattern.clone();
        node.minimum = c.minimum.clone();
        node.maximum = c.maximum.clone();
    }

    // A configurable data range projects to a pure enum. This *replaces* the
    // node rather than adding to it — matching `projectField`, where the closed
    // branch reassigns `s` and so drops any constraints. A closed domain is
    // fully described by its term list.
    if ft.datatype == Datatype::String && ft.effective_value_domain() == ValueDomain::Closed {
        node = SchemaNode {
            ty: Some("string"),
            // `vocabularyRef` resolution to the Vocabulary's effective Term
            // keys is approximated: the v1 emitter handles inline
            // `allowedValues` only, and an unresolved reference projects to an
            // empty enum rather than silently dropping the constraint.
            enumeration: Some(ft.allowed_values.clone().unwrap_or_default()),
            ..SchemaNode::default()
        };
    }

    node
}

/// Emit a number the way `JSON.stringify` does.
///
/// JavaScript has one number type, so a whole-valued float prints without its
/// fractional part: `1.0` → `1`, `-0.0` → `0`. `serde_json::Number` keeps the
/// distinction, which is right for round-tripping a stored Field but wrong for
/// byte-parity with the reference emitter. Storage keeps the distinction;
/// **this projection** does not.
fn serialize_js_number<S: Serializer>(
    value: &Option<serde_json::Number>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        None => serializer.serialize_none(),
        Some(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.is_finite() && f.abs() < 1e21 => {
                serializer.serialize_i64(f as i64)
            }
            _ => n.serialize(serializer),
        },
    }
}

/// Serialize an emitted artifact exactly as the reference emitter does:
/// `JSON.stringify(obj, null, 2)` plus a trailing newline.
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(serde_json::to_string_pretty(value)? + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::field::{ExactTypeRef, FieldTypeConstraints};
    use srs_core::types::record_type::FieldAssignment;

    fn field(id: &str, name: &str, ft: FieldType) -> Field {
        Field {
            description: String::new(),
            ai_guidance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            ..Field::new(id, "com.test", name, ft)
        }
    }

    fn record_type(id: &str, name: &str, version: u32, fields: Vec<FieldAssignment>) -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            semantic_object_type: None,
            tags: None,
            id: id.to_string(),
            namespace: "com.test".to_string(),
            name: name.to_string(),
            version,
            description: format!("v{version} shape"),
            fields,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn assign(field_id: &str, order: u32) -> FieldAssignment {
        FieldAssignment {
            default_value: None,
            field_id: field_id.to_string(),
            order,
            required: false,
            display_label: None,
        }
    }

    #[test]
    fn two_fields_projecting_to_one_key_do_not_emit_a_duplicate() {
        // `assignment_default_value` maps to `defaultValue` through the override
        // table; `default_value` maps to it mechanically. JS object assignment
        // collapses the collision — appending would emit the key twice and
        // produce a document no two parsers agree on.
        let fields = vec![
            field("f-1", "assignment_default_value", FieldType::string()),
            field("f-2", "default_value", FieldType::number()),
        ];
        let types = vec![record_type(
            "t-1",
            "thing",
            1,
            vec![assign("f-1", 0), assign("f-2", 1)],
        )];
        let ctx = ProjectionContext::new(&types, &fields);
        let schema = emit_entity(&ctx, "thing").expect("projects");
        assert_eq!(schema.properties.len(), 1, "one key, not two");
        let json = to_canonical_json(&schema).unwrap();
        assert_eq!(
            json.matches("\"defaultValue\"").count(),
            1,
            "the key must appear once: {json}"
        );
        // Last write wins, as it does in JS.
        assert_eq!(
            schema.properties.iter().next().unwrap().1.ty,
            Some("number")
        );
    }

    #[test]
    fn whole_valued_floats_print_the_way_json_stringify_prints_them() {
        // JS has one number type, so `1.0` serializes as `1`. Byte-parity is
        // defined against that, and the metamodel carries no numeric
        // constraints, so no golden covers this.
        let ft = FieldType::number().with_constraints(FieldTypeConstraints {
            minimum: Some(serde_json::Number::from_f64(1.0).unwrap()),
            maximum: Some(serde_json::Number::from_f64(2.5).unwrap()),
            ..Default::default()
        });
        let node = project_field(&ft, None);
        let json = serde_json::to_string(&node).unwrap();
        assert_eq!(json, r#"{"type":"number","minimum":1,"maximum":2.5}"#);
    }

    #[test]
    fn a_ranges_body_is_resolved_by_id_not_by_name() {
        // Two Types share the name `thing` in different namespaces. Resolving
        // the range body by name would emit the wrong one.
        let mut other = record_type("t-other", "thing", 1, vec![]);
        other.namespace = "com.other".to_string();
        let target = record_type("t-target", "thing", 1, vec![assign("f-leaf", 0)]);
        let holder = record_type("t-holder", "holder", 1, vec![assign("f-ref", 0)]);
        let fields = vec![
            field("f-leaf", "leaf", FieldType::string()),
            field(
                "f-ref",
                "child",
                FieldType::inline_ref(ExactTypeRef {
                    type_id: "t-target".to_string(),
                    type_version: 1,
                }),
            ),
        ];
        let types = vec![other, target, holder];
        let ctx = ProjectionContext::new(&types, &fields);
        let schema = emit_entity(&ctx, "holder").expect("projects");
        let defs = schema.defs.expect("an inline ref contributes a $def");
        let (_, body) = defs.iter().next().unwrap();
        assert_eq!(
            body.properties.len(),
            1,
            "the $def body must come from `t-target`, not the same-named `com.other/thing`"
        );
    }
}
