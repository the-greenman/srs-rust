//! Shared normalization for raw JSON create inputs (issue #511).
//!
//! Every definition/instance create command that reads a JSON body on stdin
//! applies the same conservative normalization before typed deserialization:
//! server-stampable boilerplate (timestamps, empty-defaultable descriptions)
//! is defaulted when absent, while any value the caller *does* provide is
//! honoured untouched. Typed deserialization goes through
//! [`from_value_with_path`] so failures carry the JSON path into the input
//! (e.g. `sections[0]: missing field \`name\``) instead of a bare serde
//! line/column that is useless on single-line stdin JSON.

use crate::error::RepositoryError;
use serde::de::DeserializeOwned;

/// Default a missing/null string field to the current RFC 3339 timestamp.
///
/// Explicit caller-provided values win — only `undefined`/`null` are stamped.
pub fn default_created_at(raw: &mut serde_json::Value, key: &str) {
    if raw.get(key).is_none_or(serde_json::Value::is_null) {
        raw[key] = serde_json::json!(chrono::Utc::now().to_rfc3339());
    }
}

/// Default a missing/null field to the empty string.
///
/// Used for schema-required but semantically optional boilerplate such as
/// `description`. Explicit caller-provided values win.
pub fn default_empty_string(raw: &mut serde_json::Value, key: &str) {
    if raw.get(key).is_none_or(serde_json::Value::is_null) {
        raw[key] = serde_json::json!("");
    }
}

/// Deserialize a raw JSON value into `T`, attaching the JSON path to errors.
///
/// `what` names the expected shape in the message (e.g. `"Composition"`).
pub fn from_value_with_path<T: DeserializeOwned>(
    raw: serde_json::Value,
    what: &str,
) -> Result<T, RepositoryError> {
    serde_path_to_error::deserialize(raw).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.into_inner();
        // "." is the document root; "?" means the path is unknown. Neither
        // adds signal to the message.
        RepositoryError::InvalidInput {
            message: if path == "." || path == "?" {
                format!("invalid {what} JSON: {inner}")
            } else {
                format!("invalid {what} JSON at {path}: {inner}")
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_created_at_stamps_missing_and_null_only() {
        let mut raw = serde_json::json!({});
        default_created_at(&mut raw, "createdAt");
        assert!(raw["createdAt"].is_string());

        let mut raw = serde_json::json!({ "createdAt": null });
        default_created_at(&mut raw, "createdAt");
        assert!(raw["createdAt"].is_string());

        let mut raw = serde_json::json!({ "createdAt": "2020-01-01T00:00:00Z" });
        default_created_at(&mut raw, "createdAt");
        assert_eq!(raw["createdAt"], "2020-01-01T00:00:00Z");
    }

    #[test]
    fn default_empty_string_honours_explicit_values() {
        let mut raw = serde_json::json!({ "description": "keep me" });
        default_empty_string(&mut raw, "description");
        assert_eq!(raw["description"], "keep me");

        let mut raw = serde_json::json!({});
        default_empty_string(&mut raw, "description");
        assert_eq!(raw["description"], "");
    }

    #[test]
    fn from_value_with_path_reports_json_path() {
        #[derive(Debug, serde::Deserialize)]
        #[allow(dead_code)]
        struct Inner {
            name: String,
        }
        #[derive(Debug, serde::Deserialize)]
        #[allow(dead_code)]
        struct Outer {
            sections: Vec<Inner>,
        }

        let raw = serde_json::json!({ "sections": [ { "heading": "x" } ] });
        let err = from_value_with_path::<Outer>(raw, "note").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("sections[0]"), "message was: {msg}");
        assert!(msg.contains("missing field"), "message was: {msg}");
    }
}

/// Regression tests for issue #511: every definition create that reads a raw
/// JSON body applies the same normalization — server-stampable boilerplate
/// (`createdAt`, empty-defaultable `description`) is defaulted when absent,
/// explicit caller values always win, and typed parse errors carry the JSON
/// path into the input.
#[cfg(test)]
mod create_normalization_tests {
    use crate::store::memory::MemoryStore;
    use serde_json::json;

    fn minimal_composition() -> serde_json::Value {
        json!({
            "namespace": "com.test",
            "name": "test-doc-view",
            "version": 1,
            "sections": [
                {
                    "sectionId": "s1",
                    "order": 0,
                    "source": { "type": "fixed-instances", "instanceIds": [] }
                }
            ]
        })
    }

    #[test]
    fn composition_create_defaults_created_at_and_description() {
        let store = MemoryStore::default();
        let result =
            crate::view_service::create_composition_normalized(&store, minimal_composition(), None)
                .expect("create without createdAt should succeed");
        assert!(!result.composition.created_at.is_empty());
        assert_eq!(result.composition.description, "");
        assert!(!result.composition.id.is_empty());
    }

    #[test]
    fn composition_create_honours_explicit_created_at() {
        let store = MemoryStore::default();
        let mut raw = minimal_composition();
        raw["createdAt"] = json!("2020-05-05T00:00:00Z");
        raw["description"] = json!("explicit");
        let result = crate::view_service::create_composition_normalized(&store, raw, None).unwrap();
        assert_eq!(result.composition.created_at, "2020-05-05T00:00:00Z");
        assert_eq!(result.composition.description, "explicit");
    }

    #[test]
    fn composition_create_parse_error_names_json_path() {
        let store = MemoryStore::default();
        let mut raw = minimal_composition();
        // Wrong section shape: missing `order`.
        raw["sections"] = json!([{ "sectionId": "s1" }]);
        let err = match crate::view_service::create_composition_normalized(&store, raw, None) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("create with malformed section should fail"),
        };
        assert!(err.contains("sections[0]"), "error was: {err}");
        assert!(err.contains("missing field"), "error was: {err}");
    }

    #[test]
    fn view_create_defaults_created_at_and_description() {
        let store = MemoryStore::default();
        let raw = json!({
            "namespace": "com.test",
            "name": "test-view",
            "version": 1,
            "fieldViews": [ { "fieldId": "f1", "order": 0 } ]
        });
        let result = crate::view_service::create_view_normalized(&store, raw, None)
            .expect("view create without createdAt should succeed");
        assert!(!result.view.created_at.is_empty());
        assert_eq!(result.view.description, "");
    }

    #[test]
    fn theme_create_defaults_created_at_and_description() {
        let store = MemoryStore::default();
        let raw = json!({
            "namespace": "com.test",
            "name": "test-theme",
            "version": 1,
            "targets": ["markdown"]
        });
        let result = crate::theme_service::create_theme_normalized(&store, raw, None)
            .expect("theme create without createdAt should succeed");
        assert!(!result.theme.created_at.is_empty());
        assert_eq!(result.theme.description, "");
    }

    #[test]
    fn blueprint_create_defaults_created_at_and_description() {
        let store = MemoryStore::default();
        let raw = json!({
            "namespace": "com.test",
            "name": "test-blueprint",
            "version": 1,
            "rootTypes": [ { "typeId": "11111111-1111-4111-8111-111111111111" } ]
        });
        let result = crate::blueprint_service::create_blueprint_normalized(&store, raw, None)
            .expect("blueprint create without createdAt should succeed");
        assert!(!result.blueprint.created_at.is_empty());
        assert_eq!(result.blueprint.description, "");
    }

    #[test]
    fn type_create_defaults_created_at_and_description() {
        let store = MemoryStore::default();
        let raw = json!({
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "fields": []
        });
        let result = crate::package_service::create_type_normalized(&store, raw, None)
            .expect("type create without createdAt should succeed");
        assert!(!result.record_type.created_at.is_empty());
        assert_eq!(result.record_type.description, "");
        assert!(!result.record_type.id.is_empty());
    }

    #[test]
    fn relation_type_create_defaults_created_at_and_description() {
        let store = MemoryStore::default();
        let raw = json!({
            "version": 1,
            "key": "test-relates",
            "namespace": "com.test",
            "label": "Test Relates",
            "category": "association"
        });
        let result = crate::package_service::create_relation_type_normalized(&store, raw, None)
            .expect("relation-type create without createdAt should succeed");
        assert!(!result.relation_type_definition.created_at.is_empty());
        assert_eq!(result.relation_type_definition.description, "");
    }

    #[test]
    fn lifecycle_create_defaults_created_at_and_id() {
        let store = MemoryStore::default();
        let raw = json!({
            "version": 1,
            "namespace": "com.test",
            "name": "test-lifecycle",
            "states": [ { "key": "draft", "isInitial": true } ],
            "transitions": [],
            "initialState": "draft"
        });
        let result = crate::lifecycle_service::create_lifecycle_normalized(&store, raw, None)
            .expect("lifecycle create without createdAt/id should succeed");
        assert!(!result.lifecycle.created_at.is_empty());
        assert!(!result.lifecycle.id.is_empty());
    }

    #[test]
    fn vocabulary_create_defaults_created_at() {
        let store = MemoryStore::default();
        let raw = json!({
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "open",
            "terms": []
        });
        let result = crate::vocabulary_service::create_vocabulary_normalized(&store, raw)
            .expect("vocabulary create without createdAt should succeed");
        assert!(!result.vocabulary.created_at.is_empty());
    }

    #[test]
    fn protocol_create_defaults_created_at() {
        let store = MemoryStore::default();
        let raw = json!({
            "protocolId": "22222222-2222-4222-8222-222222222222",
            "protocolNamespace": "com.test",
            "protocolName": "test-protocol",
            "protocolVersion": 1,
            "protocolTargetType": "com.test/thing",
            "protocolStages": []
        });
        let result = crate::protocol_service::create_protocol(&store, raw, None)
            .expect("protocol create without protocolCreatedAt should succeed");
        let created = result.protocol["protocolCreatedAt"]
            .as_str()
            .expect("protocolCreatedAt should be stamped on the stored value");
        assert!(!created.is_empty());
    }

    #[test]
    fn protocol_create_honours_explicit_created_at() {
        let store = MemoryStore::default();
        let raw = json!({
            "protocolId": "33333333-3333-4333-8333-333333333333",
            "protocolNamespace": "com.test",
            "protocolName": "test-protocol-2",
            "protocolVersion": 1,
            "protocolTargetType": "com.test/thing",
            "protocolStages": [],
            "protocolCreatedAt": "2021-01-01T00:00:00Z"
        });
        let result = crate::protocol_service::create_protocol(&store, raw, None).unwrap();
        assert_eq!(
            result.protocol["protocolCreatedAt"].as_str(),
            Some("2021-01-01T00:00:00Z")
        );
    }

    #[test]
    fn field_create_normalized_still_defaults_mechanical_boilerplate() {
        let store = MemoryStore::default();
        let raw = json!({
            "namespace": "com.test",
            "name": "test_field",
            "version": 1,
            "aiGuidance": {"purpose": "captures a test value"},
            "valueType": "string"
        });
        let result = crate::package_service::create_field_normalized(&store, raw, None)
            .expect("field create without id/description/createdAt should succeed");
        assert!(!result.field.created_at.is_empty());
        assert!(!result.field.id.is_empty());
        // A pre-RFC-032 payload is upgraded, not rejected.
        assert_eq!(
            result.field.field_type,
            srs_core::types::field::FieldType::string()
        );
    }

    #[test]
    fn field_create_normalized_refuses_to_manufacture_ai_guidance() {
        // srs-rust#768: absent guidance must surface as an actionable error,
        // never as an injected `purpose: ""` that satisfies the schema while
        // carrying no information.
        let store = MemoryStore::default();
        let raw = json!({
            "namespace": "com.test",
            "name": "unguided_field",
            "version": 1,
            "valueType": "string"
        });
        let err = crate::package_service::create_field_normalized(&store, raw, None)
            .expect_err("a Field with no aiGuidance.purpose must be rejected");
        assert!(
            format!("{err}").contains("aiGuidance.purpose"),
            "error must name the missing property: {err}"
        );
    }
}
