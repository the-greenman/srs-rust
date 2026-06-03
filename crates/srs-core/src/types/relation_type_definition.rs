use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Defines a named relation type within a package's relation type vocabulary.
///
/// `RelationTypeDefinition` is a core SRS type that gives semantic meaning and
/// validation rules to a class of relations. Definitions are loaded from package
/// `relationTypes[]` entries and resolved into the effective installed set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationTypeDefinition {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Stable UUID identity for this definition.
    #[serde(default)]
    pub id: String,
    /// Monotonically increasing version. Starts at 1.
    pub version: u32,
    /// Canonical bare string (e.g. `precedes`) or namespaced `namespace/name` form.
    /// Serialized as `relationType`; also accepts `key` for RFC-006 forward compat.
    #[serde(rename = "relationType", alias = "key")]
    pub key: String,
    /// Package namespace this definition belongs to.
    pub namespace: String,
    /// Short human-readable label.
    pub label: String,
    /// Full semantic description.
    pub description: String,
    /// Structural category.
    pub category: RelationTypeCategory,
    /// ISO 8601 timestamp when this definition was created.
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inverse_type: Option<String>,
    /// When true, a relation from an instance to itself is invalid (E3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub irreflexive: Option<bool>,
    /// Allowed `semanticObjectType` values for the source instance (E4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_source_types: Option<Vec<String>>,
    /// Allowed `semanticObjectType` values for the target instance (E4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_target_types: Option<Vec<String>>,
    /// When true, source and target must share the same `semanticObjectType` (E4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_same_semantic_object_type: Option<bool>,
    /// Lifecycle status. Absent means active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RelationTypeStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Arbitrary metadata per substrate Change H policy. Unknown top-level fields are rejected; use this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, serde_json::Value>>,
}

/// Structural category of a relation type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationTypeCategory {
    #[serde(rename = "composition")]
    Composition,
    #[serde(rename = "refinement")]
    Refinement,
    #[serde(rename = "dependency")]
    Dependency,
    #[serde(rename = "sequence")]
    Sequence,
    #[serde(rename = "derivation")]
    Derivation,
    #[serde(rename = "evidence")]
    Evidence,
    #[serde(rename = "governance")]
    Governance,
    #[serde(rename = "association")]
    Association,
    #[serde(rename = "lifecycle")]
    Lifecycle,
    #[serde(rename = "provenance")]
    Provenance,
    #[serde(rename = "other")]
    Other,
}

/// Lifecycle status of a relation type definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelationTypeStatus {
    #[serde(rename = "active")]
    Active,
    /// Resolves but new writes are rejected.
    #[serde(rename = "deprecated")]
    Deprecated,
    /// Resolves for reads only.
    #[serde(rename = "tombstone")]
    Tombstone,
    /// Does not resolve.
    #[serde(rename = "retired")]
    Retired,
}

impl RelationTypeDefinition {
    /// Returns true if this definition is effectively active (resolves for reads and writes).
    /// Returns true if this definition resolves for new relation writes.
    pub fn accepts_new_relations(&self) -> bool {
        matches!(self.status, None | Some(RelationTypeStatus::Active))
    }

    /// Returns true if this definition resolves for historical reads.
    pub fn resolves_for_reads(&self) -> bool {
        !matches!(self.status, Some(RelationTypeStatus::Retired))
    }

    // Keep backwards-compat aliases used in existing tests
    pub fn is_active(&self) -> bool {
        self.accepts_new_relations()
    }

    pub fn resolves(&self) -> bool {
        self.resolves_for_reads()
    }

    pub fn accepts_writes(&self) -> bool {
        self.accepts_new_relations()
    }

    /// Returns true if `irreflexive` is set to true.
    pub fn is_irreflexive(&self) -> bool {
        self.irreflexive.unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn canonical_precedes() -> RelationTypeDefinition {
        RelationTypeDefinition {
            schema: None,
            id: "f7a8b9c0-d1e2-4f3a-8b4c-5d6e7f8a9b0c".to_string(),
            version: 1,
            key: "precedes".to_string(),
            namespace: "com.semanticops.srs".to_string(),
            label: "Precedes".to_string(),
            description: "Source comes before target in a sequence.".to_string(),
            category: RelationTypeCategory::Sequence,
            created_at: "2026-05-29T00:00:00Z".to_string(),
            canonical_direction: Some("source comes before target".to_string()),
            inverse_type: Some("follows".to_string()),
            irreflexive: Some(true),
            allowed_source_types: None,
            allowed_target_types: None,
            require_same_semantic_object_type: None,
            status: None,
            updated_at: None,
            properties: None,
        }
    }

    #[test]
    fn roundtrips_json() {
        let rtd = canonical_precedes();
        let json_str = serde_json::to_string(&rtd).unwrap();
        let parsed: RelationTypeDefinition = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed.key, "precedes");
        assert_eq!(parsed.category, RelationTypeCategory::Sequence);
        assert_eq!(parsed.irreflexive, Some(true));
        assert_eq!(parsed.status, None);
    }

    #[test]
    fn minimal_fields_serialize_without_optionals() {
        let rtd = canonical_precedes();
        let json_str = serde_json::to_string(&rtd).unwrap();
        assert!(!json_str.contains("allowedSourceTypes"));
        assert!(!json_str.contains("status"));
        assert!(!json_str.contains("updatedAt"));
    }

    #[test]
    fn is_active_without_status() {
        let rtd = canonical_precedes();
        assert!(rtd.is_active());
        assert!(rtd.resolves());
        assert!(rtd.accepts_writes());
    }

    #[test]
    fn deprecated_resolves_but_no_writes() {
        let rtd = RelationTypeDefinition {
            status: Some(RelationTypeStatus::Deprecated),
            ..canonical_precedes()
        };
        assert!(!rtd.is_active());
        assert!(rtd.resolves());
        assert!(!rtd.accepts_writes());
    }

    #[test]
    fn retired_does_not_resolve() {
        let rtd = RelationTypeDefinition {
            status: Some(RelationTypeStatus::Retired),
            ..canonical_precedes()
        };
        assert!(!rtd.resolves());
        assert!(!rtd.accepts_writes());
    }

    #[test]
    fn tombstone_resolves_reads_only() {
        let rtd = RelationTypeDefinition {
            status: Some(RelationTypeStatus::Tombstone),
            ..canonical_precedes()
        };
        assert!(!rtd.is_active());
        assert!(rtd.resolves());
        assert!(!rtd.accepts_writes());
    }

    #[test]
    fn is_irreflexive_true_when_set() {
        let rtd = canonical_precedes();
        assert!(rtd.is_irreflexive());
    }

    #[test]
    fn is_irreflexive_false_when_absent() {
        let rtd = RelationTypeDefinition {
            irreflexive: None,
            ..canonical_precedes()
        };
        assert!(!rtd.is_irreflexive());
    }

    #[test]
    fn deserializes_from_canonical_json() {
        let json_str = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/relation-type.json",
            "id": "f7a8b9c0-d1e2-4f3a-8b4c-5d6e7f8a9b0c",
            "version": 1,
            "relationType": "precedes",
            "namespace": "com.semanticops.srs",
            "label": "Precedes",
            "description": "Source comes before target.",
            "category": "sequence",
            "irreflexive": true,
            "createdAt": "2026-05-29T00:00:00Z"
        }"#;
        let rtd: RelationTypeDefinition = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            rtd.schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/relation-type.json")
        );
        assert_eq!(rtd.key, "precedes");
        assert_eq!(rtd.category, RelationTypeCategory::Sequence);
        assert!(rtd.is_irreflexive());
        assert!(rtd.is_active());
    }

    #[test]
    fn deserializes_deprecated_namespaced_type() {
        let json_str = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/relation-type.json",
            "id": "a1000001-0000-4000-b000-000000000001",
            "version": 1,
            "relationType": "com.semanticops.spec/rfc-change-sequence",
            "namespace": "com.semanticops.spec",
            "label": "RFC change sequence",
            "description": "Orders rfc-change records.",
            "category": "sequence",
            "status": "deprecated",
            "createdAt": "2026-05-29T00:00:00Z"
        }"#;
        let rtd: RelationTypeDefinition = serde_json::from_str(json_str).unwrap();
        assert_eq!(
            rtd.schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/relation-type.json")
        );
        assert_eq!(rtd.status, Some(RelationTypeStatus::Deprecated));
        assert!(!rtd.accepts_writes());
        assert!(rtd.resolves());
    }
}
