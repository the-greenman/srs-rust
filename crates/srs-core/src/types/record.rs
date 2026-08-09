use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// RFC-039 revision-2 value carrier: a JSON object keyed by `Field.name`
/// verbatim ([R2b]), whose values follow the recursive Change-B rule. Key
/// order is data — [R18] requires serialisation in `FieldAssignment.order` —
/// so the map is insertion-ordered (`serde_json::Map` under `preserve_order`,
/// ADR-043) at every boundary, including `to_value` funnels.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct FieldValues(pub serde_json::Map<String, serde_json::Value>);

/// [R9] at every entry point: an array `fieldValues` is a revision ≤ 1
/// document and is rejected with a diagnostic naming the expected revision —
/// on `Record` loads and on every input struct embedding `FieldValues`
/// (CLI stdin, MCP tools, bindings) alike. Never coerced.
impl<'de> Deserialize<'de> for FieldValues {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::Object(map) => Ok(FieldValues(map)),
            serde_json::Value::Array(_) => Err(serde::de::Error::custom(
                "fieldValues is an array — this is a dataModelRevision <= 1 document; \
                 expected dataModelRevision 2 (object keyed by Field.name, RFC-039 [R9]). \
                 Migrate the repository with `srs repo apply-migration --id rfc039-carrier`",
            )),
            other => Err(serde::de::Error::custom(format!(
                "fieldValues must be an object keyed by Field.name (RFC-039 [R1]), got {}",
                json_type_name(&other)
            ))),
        }
    }
}

impl FieldValues {
    #[must_use]
    pub fn new() -> Self {
        Self(serde_json::Map::new())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&serde_json::Value> {
        self.0.get(name)
    }

    pub fn insert(&mut self, name: impl Into<String>, value: serde_json::Value) {
        self.0.insert(name.into(), value);
    }

    #[must_use]
    pub fn contains_key(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &serde_json::Value)> {
        self.0.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }
}

/// Per-field provenance ([R6], RFC-039 Change C): metadata about the
/// assertion, keyed identically to the sibling `fieldValues` map. One object
/// per field — never per list item, never inside a composite interior.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub instance_id: String,
    pub type_id: String,
    pub type_version: u32,
    pub type_namespace: String,
    pub type_name: String,
    /// Name-keyed recursive value carrier (RFC-039 [R1]–[R3]). An array here
    /// is a revision ≤ 1 document, rejected at deserialization by
    /// `FieldValues`' own [R9] impl.
    pub field_values: FieldValues,
    /// Per-field provenance, keys ⊆ `field_values` keys ([R6]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_meta: Option<IndexMap<String, FieldMeta>>,
    /// ext:lifecycle — current lifecycle state. Must name a state in the associated
    /// Type's lifecycle.states[] when the Type declares a lifecycle (Invariant 6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// Topic membership labels. When a Vocabulary is declared, each value MUST resolve to a
    /// Term key or alias (tier-graduated: Records enforce resolution; Notes do not).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Envelope extras (`$schema`, `meta`, `sourceRefs`, …). `BTreeMap` for
    /// deterministic serialisation (ADR-043 canonical-types discipline).
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

pub(crate) fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

impl Record {
    /// Value at a `Field.name` key ([R2b]: verbatim, no transform).
    #[must_use]
    pub fn value(&self, name: &str) -> Option<&serde_json::Value> {
        self.field_values.get(name)
    }

    /// String value at a `Field.name` key.
    #[must_use]
    pub fn value_str(&self, name: &str) -> Option<&str> {
        self.value(name).and_then(|v| v.as_str())
    }

    /// Type-mediated `fieldId` recovery ([R19] rationale): the instance stores
    /// no ids — a key's `fieldId` comes from the Type's effective field set.
    #[must_use]
    pub fn field_id_for<'a>(
        &self,
        name: &str,
        effective_fields: &'a [crate::validation::value_shape::EffectiveField],
    ) -> Option<&'a str> {
        effective_fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.field_id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_record() -> Record {
        Record {
            instance_id: "00000000-0000-4000-8000-000000000001".to_string(),
            type_id: "00000000-0000-4000-8000-000000000002".to_string(),
            type_version: 1,
            type_namespace: "test.ns".to_string(),
            type_name: "test-type".to_string(),
            field_values: FieldValues::new(),
            field_meta: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn record_roundtrips_json() {
        let mut fv = FieldValues::new();
        fv.insert("title", json!("value1"));
        fv.insert("count", json!(42));
        let record = Record {
            field_values: fv,
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            ..minimal_record()
        };

        let json_str = serde_json::to_string(&record).unwrap();
        let parsed: Record = serde_json::from_str(&json_str).unwrap();

        assert_eq!(parsed.instance_id, record.instance_id);
        assert_eq!(parsed.type_namespace, "test.ns");
        assert_eq!(parsed.type_name, "test-type");
        assert_eq!(parsed.field_values.len(), 2);
        assert_eq!(parsed.value_str("title"), Some("value1"));
    }

    #[test]
    fn record_extra_fields_survive_roundtrip() {
        let json_str = r#"{
            "instanceId": "00000000-0000-4000-8000-000000000001",
            "typeId": "00000000-0000-4000-8000-000000000002",
            "typeVersion": 1,
            "typeNamespace": "test.ns",
            "typeName": "test-type",
            "fieldValues": {},
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json"
        }"#;

        let record: Record = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            record.extra.get("$schema"),
            Some(&json!("https://srs.semanticops.com/schema/2.0/record.json"))
        );

        let serialized = serde_json::to_string(&record).unwrap();
        assert!(serialized.contains("$schema"));
    }

    #[test]
    fn rev1_array_field_values_rejected_with_r9_diagnostic() {
        let json_str = r#"{
            "instanceId": "00000000-0000-4000-8000-000000000001",
            "typeId": "00000000-0000-4000-8000-000000000002",
            "typeVersion": 1,
            "typeNamespace": "test.ns",
            "typeName": "test-type",
            "fieldValues": [{"fieldId": "00000000-0000-4000-8000-000000000009", "value": "x"}]
        }"#;
        let err = serde_json::from_str::<Record>(json_str).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("dataModelRevision"),
            "diagnostic names the expected revision: {msg}"
        );
        assert!(msg.contains("[R9]"), "diagnostic cites the rule: {msg}");
    }

    #[test]
    fn scalar_field_values_rejected() {
        let json_str = r#"{
            "instanceId": "00000000-0000-4000-8000-000000000001",
            "typeId": "00000000-0000-4000-8000-000000000002",
            "typeVersion": 1,
            "typeNamespace": "test.ns",
            "typeName": "test-type",
            "fieldValues": "nope"
        }"#;
        assert!(serde_json::from_str::<Record>(json_str).is_err());
    }

    #[test]
    fn field_values_order_survives_to_value_roundtrip() {
        // ADR-043 / ADR-017 amendment: insertion order must survive the
        // serde_json::Value funnel (preserve_order) and direct serialisation.
        let mut fv = FieldValues::new();
        fv.insert("zeta", json!("z"));
        fv.insert("alpha", json!("a"));
        fv.insert("midway", json!("m"));
        let record = Record {
            field_values: fv,
            ..minimal_record()
        };

        let value = serde_json::to_value(&record).unwrap();
        let keys: Vec<&String> = value["fieldValues"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["zeta", "alpha", "midway"]);

        let direct = serde_json::to_string(&record).unwrap();
        let z = direct.find("zeta").unwrap();
        let a = direct.find("alpha").unwrap();
        let m = direct.find("midway").unwrap();
        assert!(
            z < a && a < m,
            "direct serialisation preserves insertion order"
        );
    }

    #[test]
    fn nested_composite_value_roundtrips() {
        // An inline-composite value is itself a fieldValues-shaped object
        // (RFC-039 Change B) — carried recursively with no wrapper construct.
        let mut fv = FieldValues::new();
        fv.insert(
            "rows",
            json!([
                {"cells": ["a", "b"]},
                {"cells": ["c", "d"]}
            ]),
        );
        let record = Record {
            field_values: fv,
            ..minimal_record()
        };
        let value = serde_json::to_value(&record).unwrap();
        let parsed: Record = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.value("rows").unwrap()[0]["cells"], json!(["a", "b"]));
    }

    #[test]
    fn field_meta_roundtrips_and_absent_omits_key() {
        let mut meta = IndexMap::new();
        meta.insert(
            "title".to_string(),
            FieldMeta {
                source: Some("human".to_string()),
                ..Default::default()
            },
        );
        let mut fv = FieldValues::new();
        fv.insert("title", json!("t"));
        let record = Record {
            field_values: fv,
            field_meta: Some(meta),
            ..minimal_record()
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["fieldMeta"]["title"]["source"], json!("human"));

        let bare = minimal_record();
        let bare_value = serde_json::to_value(&bare).unwrap();
        assert!(bare_value.get("fieldMeta").is_none());
    }

    #[test]
    fn lifecycle_state_roundtrips_and_not_in_extra() {
        let record = Record {
            lifecycle_state: Some("active".to_string()),
            ..minimal_record()
        };
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value["lifecycleState"], json!("active"));

        let parsed: Record = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.lifecycle_state.as_deref(), Some("active"));
        assert!(!parsed.extra.contains_key("lifecycleState"));
    }

    #[test]
    fn lifecycle_state_absent_omits_key() {
        let record = minimal_record();
        let value = serde_json::to_value(&record).unwrap();
        assert!(value.get("lifecycleState").is_none());
    }

    #[test]
    fn minimal_record_passes_schema_contract() {
        let reg = srs_schema::SchemaRegistry::global();
        let record = minimal_record();
        let mut value = serde_json::to_value(&record).unwrap();
        value["$schema"] = json!("https://srs.semanticops.com/schema/2.0/record.json");
        reg.validate_by_id(srs_schema::RECORD_SCHEMA_ID, &value)
            .expect("minimal Record must pass record.json schema");
    }
}
