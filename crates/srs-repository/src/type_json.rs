//! Shared JSON intermediate for Type definition files (mirror of
//! [`crate::field_json`]).
//!
//! `FileStore` parses type definition files through [`TypeJson`] and converts with
//! [`TypeJson::into_record_type`].
//!
//! A Type file is **definition-layer** data, so this reader is a trust boundary:
//! it rejects keys `type.json` does not declare (`deny_unknown_fields`, matching
//! that schema's `additionalProperties: false`) rather than absorbing them into
//! a catch-all — decision `rfc-decision-2e0cd70a`, srs-rust#863. Every key the
//! schema *does* declare is modelled here and carried through to
//! [`RecordType`], so `$schema`/`aiGuidance` still survive load → edit → save
//! (the archive/type fidelity bug fixed under #684) — now by name rather than
//! by bag.

use srs_core::types::field::{Lineage, Provenance};
use srs_core::types::record_type::{
    CrossFieldRule, FieldAssignment, FieldAssignmentOverride, RecordType, TypeLifecycle,
};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TypeJson {
    /// Declared by `type.json` itself — not an unknown property.
    #[serde(rename = "$schema", default)]
    schema: Option<String>,
    id: String,
    namespace: String,
    name: String,
    version: u32,
    description: Option<String>,
    fields: Vec<FieldAssignmentJson>,
    #[serde(default)]
    extends_type_id: Option<String>,
    #[serde(default)]
    extends_type_version: Option<u32>,
    #[serde(default)]
    field_order: Option<Vec<String>>,
    #[serde(default)]
    field_assignment_overrides: Option<Vec<FieldAssignmentOverrideJson>>,
    #[serde(default)]
    identity_field_id: Option<String>,
    #[serde(default)]
    lifecycle: Option<TypeLifecycle>,
    #[serde(default)]
    lifecycle_ref: Option<String>,
    #[serde(default)]
    validation_rules: Option<Vec<CrossFieldRule>>,
    // RFC-040 Change E (srs#477/#867): previously unmodelled here — silently
    // preserved raw in `extra` but never typed onto `RecordType`, exactly the
    // `FieldAssignment.repeatable`-class loss this loader must not repeat.
    #[serde(default)]
    lineage: Option<Lineage>,
    #[serde(default)]
    provenance: Option<Provenance>,
    created_at: Option<String>,
    #[serde(default)]
    ai_guidance: Option<serde_json::Value>,
    /// TRANSITIONAL (srs#372/#383/#422; collapse pending srs#272): still
    /// declared by `type.json`, still accepted here, but routed into
    /// `RecordType.extra` rather than a typed field in `into_record_type` —
    /// it was ruled a duplicate of the Type system, not real Type surface.
    /// Delete this field (and the `extra` insert below) when #383 executes
    /// the collapse.
    #[serde(default)]
    semantic_object_type: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// RFC-039 [R7] retired this construct. It is accepted here and dropped so
    /// that `repo validate` can still report it as a *named diagnostic* over
    /// the raw document; rejecting it at load would leave the repository
    /// unreadable by the very command meant to explain the problem.
    ///
    /// Dropped, not carried — a deliberate delta from the removed `extra` bag,
    /// which round-tripped it. A revision ≤ 1 Type file rewritten by this
    /// binary therefore loses `fieldGroups`, which is the intended outcome of
    /// the RFC-039 migration and is what `apply-migration --id rfc039-carrier`
    /// does on purpose. Every first-party corpus is already revision 2.
    #[serde(default, rename = "fieldGroups")]
    _retired_field_groups: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FieldAssignmentJson {
    field_id: String,
    order: u32,
    required: Option<bool>,
    display_label: Option<String>,
    // RFC-040 Change C (srs#477/#867): was missing here entirely — not even
    // caught by a flatten catch-all (`TypeJson::extra` only flattens the
    // top-level document, not each `fields[]` entry), so a `description` on a
    // FieldAssignment would be silently and irrecoverably dropped on load.
    #[serde(default)]
    description: Option<String>,
    /// RFC-039 [R7] retired the assignment trio — accepted and dropped on the
    /// same terms as `fieldGroups` above, so the diagnostic path survives.
    #[serde(default, rename = "repeatable")]
    _retired_repeatable: Option<serde_json::Value>,
    #[serde(default, rename = "minItems")]
    _retired_min_items: Option<serde_json::Value>,
    #[serde(default, rename = "maxItems")]
    _retired_max_items: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FieldAssignmentOverrideJson {
    field_id: String,
    display_label: Option<String>,
    display_hint: Option<String>,
    required: Option<bool>,
}

fn into_assignment(fa: FieldAssignmentJson) -> FieldAssignment {
    FieldAssignment {
        field_id: fa.field_id,
        order: fa.order,
        required: fa.required.unwrap_or(true),
        display_label: fa.display_label,
        description: fa.description,
    }
}

impl TypeJson {
    pub(crate) fn into_record_type(self) -> RecordType {
        let fields: Vec<FieldAssignment> = self.fields.into_iter().map(into_assignment).collect();
        let field_assignment_overrides = self.field_assignment_overrides.map(|overrides| {
            overrides
                .into_iter()
                .map(|o| FieldAssignmentOverride {
                    field_id: o.field_id,
                    display_label: o.display_label,
                    display_hint: o.display_hint,
                    required: o.required,
                })
                .collect()
        });
        // TRANSITIONAL (srs#372/#383/#422): semanticObjectType rides in
        // `extra` untyped, not as a named RecordType field — see the
        // field's own doc comment above and RecordType::extra's.
        let mut extra = std::collections::BTreeMap::new();
        if let Some(v) = self.semantic_object_type {
            extra.insert(
                "semanticObjectType".to_string(),
                serde_json::Value::String(v),
            );
        }
        RecordType {
            schema: self.schema,
            ai_guidance: self.ai_guidance,
            tags: self.tags,
            extra,
            id: self.id,
            namespace: self.namespace,
            name: self.name,
            version: self.version,
            description: self.description.unwrap_or_default(),
            fields,
            extends_type_id: self.extends_type_id,
            extends_type_version: self.extends_type_version,
            field_order: self.field_order,
            field_assignment_overrides,
            identity_field_id: self.identity_field_id,
            lifecycle: self.lifecycle,
            lifecycle_ref: self.lifecycle_ref,
            validation_rules: self.validation_rules,
            created_at: self.created_at.unwrap_or_default(),
            lineage: self.lineage,
            provenance: self.provenance,
        }
    }
}

/// Snapshot/portability compatibility reader (mirrors
/// [`crate::field_json::deserialize_fields_compat`]).
///
/// A `PackageBoundarySnapshot` carries Types as JSON, so it must read them
/// through the same definition reader the file loader uses — otherwise the two
/// paths disagree about what a Type file may contain, and a legacy archive
/// (revision ≤ 1, where RFC-039's retired constructs were still legal) stops
/// loading at exactly the moment `repo validate` needs to explain why.
pub(crate) fn deserialize_types_compat<'de, D>(deserializer: D) -> Result<Vec<RecordType>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let raw = Vec::<TypeJson>::deserialize(deserializer)?;
    Ok(raw.into_iter().map(TypeJson::into_record_type).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot/bundle path must read Types exactly as the file path does,
    /// including the retired-construct tolerance — otherwise a legacy archive
    /// stops importing.
    #[test]
    fn deserialize_types_compat_matches_the_file_reader() {
        #[derive(Debug, serde::Deserialize)]
        struct Snapshot {
            #[serde(deserialize_with = "super::deserialize_types_compat")]
            types: Vec<RecordType>,
        }
        let snap: Snapshot = serde_json::from_str(
            r#"{"types": [{
                "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                "id": "t1", "namespace": "com.test", "name": "thing",
                "version": 1, "fieldGroups": {"g": {}},
                "fields": [{"fieldId": "f1", "order": 1, "repeatable": true}]
            }]}"#,
        )
        .expect("a snapshot carrying retired constructs must import");
        assert_eq!(snap.types.len(), 1);
        assert_eq!(
            snap.types[0].schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/type.json")
        );

        // …and it is the same reader, so it is equally strict about keys the
        // schema never declared.
        let err = serde_json::from_str::<Snapshot>(
            r#"{"types": [{"id": "t1", "namespace": "com.test", "name": "thing",
                 "version": 1, "fields": [], "xUnknown": 1}]}"#,
        )
        .expect_err("unknown keys are rejected on this path too");
        assert!(err.to_string().contains("xUnknown"), "{err}");
    }

    /// rfc-decision-0225099b: `defaultValue` was ruled removed on both the
    /// Field and FieldAssignment sites — there is no carry mechanism for it
    /// left anywhere in the definition layer. A `fields[]` entry declaring it
    /// must be rejected outright (not silently accepted-and-dropped, the
    /// treatment retired constructs like `fieldGroups` still get).
    #[test]
    fn field_assignment_default_value_is_rejected_not_carried() {
        let err = serde_json::from_str::<TypeJson>(
            r#"{
                "id": "t1", "namespace": "com.test", "name": "thing",
                "version": 1,
                "fields": [{"fieldId": "f1", "order": 1, "defaultValue": "nope"}]
            }"#,
        )
        .expect_err("defaultValue must be rejected per ruling 0225099b, not carried");
        assert!(err.to_string().contains("defaultValue"), "{err}");
    }

    /// Owner ruling 2026-08-26 on #876: `semanticObjectType` was ruled a
    /// duplicate of the Type system (srs#372/#383/#422; collapse execution
    /// pending at srs#272), so unlike `$schema`/`aiGuidance`/`tags` it is
    /// deliberately NOT re-typed as a named `RecordType` field — re-typing it
    /// would re-entrench the construct being removed. It still round-trips
    /// byte-faithfully, just via `RecordType.extra` instead, until #383
    /// executes the collapse.
    #[test]
    fn semantic_object_type_round_trips_via_extra_not_a_typed_field() {
        let tj: TypeJson = serde_json::from_str(
            r#"{
                "id": "t1", "namespace": "com.test", "name": "thing",
                "version": 1, "fields": [],
                "semanticObjectType": "com.example/decision"
            }"#,
        )
        .unwrap();
        let rt = tj.into_record_type();

        assert_eq!(
            rt.extra.get("semanticObjectType"),
            Some(&serde_json::Value::String(
                "com.example/decision".to_string()
            )),
            "semanticObjectType must survive load, carried in extra"
        );

        let val = serde_json::to_value(&rt).unwrap();
        assert_eq!(
            val["semanticObjectType"], "com.example/decision",
            "must re-emit as a top-level key, not nested under an \"extra\" wrapper"
        );
    }

    /// The single-key transitional bag still enforces the definition layer's
    /// reject-unknown policy (srs-rust#863) for everything except the one
    /// legacy key it exists for.
    #[test]
    fn record_type_extra_rejects_anything_other_than_semantic_object_type() {
        use srs_core::types::record_type::RecordType;
        let err = serde_json::from_str::<RecordType>(
            r#"{
                "id": "t1", "namespace": "com.test", "name": "thing",
                "version": 1, "description": "a type", "fields": [],
                "createdAt": "2026-01-01T00:00:00Z",
                "somethingElse": true
            }"#,
        )
        .expect_err("only semanticObjectType may ride in RecordType.extra");
        assert!(err.to_string().contains("somethingElse"), "{err}");
    }

    /// srs-rust#863: a Type file is definition-layer data, so the loader
    /// rejects a key `type.json` does not declare — and names it.
    #[test]
    fn unknown_key_is_rejected_and_named() {
        let err = serde_json::from_str::<TypeJson>(
            r#"{
                "id": "t1", "namespace": "com.test", "name": "thing",
                "version": 1, "fields": [], "xUnknown": "nope"
            }"#,
        )
        .expect_err("definition layer must reject unknown keys");
        assert!(err.to_string().contains("xUnknown"), "{err}");
    }

    /// RFC-039 [R7]'s retired constructs stay *loadable* on purpose: `repo
    /// validate` reports them as named diagnostics over the raw document, and
    /// it cannot do that for a repository it can no longer read.
    #[test]
    fn retired_constructs_load_and_are_dropped() {
        let tj: TypeJson = serde_json::from_str(
            r#"{
                "id": "t1", "namespace": "com.test", "name": "thing",
                "version": 1, "fieldGroups": {"g": {}},
                "fields": [{"fieldId": "f1", "order": 1, "repeatable": true,
                            "minItems": 1, "maxItems": 3}]
            }"#,
        )
        .expect("retired constructs must not break the reader");
        let rt = tj.into_record_type();
        let val = serde_json::to_value(&rt).unwrap();
        assert!(val.get("fieldGroups").is_none());
        assert!(val["fields"][0].get("repeatable").is_none());
    }

    /// `$schema` and `aiGuidance` are `type.json` properties, so they are
    /// modelled fields that survive the load — not catch-all content.
    #[test]
    fn schema_declared_keys_survive_into_record_type() {
        let tj: TypeJson = serde_json::from_str(
            r#"{
                "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
                "id": "t1",
                "namespace": "com.test",
                "name": "thing",
                "version": 1,
                "aiGuidance": "guidance text",
                "fields": [{"fieldId": "f1", "order": 1}]
            }"#,
        )
        .unwrap();
        let rt = tj.into_record_type();
        assert_eq!(
            rt.schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/type.json")
        );
        assert_eq!(
            rt.ai_guidance.as_ref().and_then(|v| v.as_str()),
            Some("guidance text")
        );
        // Round-trip: serialization re-emits the preserved keys.
        let val = serde_json::to_value(&rt).unwrap();
        assert_eq!(val["aiGuidance"], "guidance text");
    }

    /// RFC-040 (srs#477/#867) regression: the metamodel v1.1.0 additions must
    /// round-trip through this loader like every other `FieldAssignment`/`Type`
    /// property, not repeat the historical `FieldAssignment.repeatable`-class
    /// loss where a nested property was silently dropped because it fell
    /// outside every flatten catch-all.
    #[test]
    fn v1_1_0_metamodel_additions_round_trip() {
        let tj: TypeJson = serde_json::from_str(
            r#"{
                "id": "t1",
                "namespace": "com.test",
                "name": "thing",
                "version": 1,
                "fields": [
                    {"fieldId": "f1", "order": 1, "description": "context for f1"}
                ],
                "lineage": {"sourceDefinitionId": "src-1", "sourceVersion": 1},
                "provenance": {"publisher": "com.example"}
            }"#,
        )
        .unwrap();
        let rt = tj.into_record_type();

        assert_eq!(
            rt.fields[0].description.as_deref(),
            Some("context for f1"),
            "FieldAssignment.description must survive load, not silently drop"
        );
        assert_eq!(
            rt.lineage
                .as_ref()
                .and_then(|l| l.source_definition_id.as_deref()),
            Some("src-1")
        );
        assert_eq!(
            rt.provenance.as_ref().and_then(|p| p.publisher.as_deref()),
            Some("com.example")
        );

        // And the typed properties re-emit on the wire as named keys, not a
        // flatten catch-all echo — RecordType carries no `extra` bag at all
        // (srs-rust#863: the definition layer rejects unknown keys instead).
        let val = serde_json::to_value(&rt).unwrap();
        assert_eq!(val["fields"][0]["description"], "context for f1");
        assert_eq!(val["lineage"]["sourceDefinitionId"], "src-1");
        assert_eq!(val["provenance"]["publisher"], "com.example");
    }
}
