use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Defines a named relation type within a package's relation type vocabulary.
///
/// `RelationTypeDefinition` is a core SRS type that gives semantic meaning and
/// validation rules to a class of relations. Definitions are loaded from package
/// `relationTypes[]` entries and resolved into the effective installed set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationTypeDefinition {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Stable UUID identity for this definition.
    #[serde(default)]
    pub id: String,
    /// Monotonically increasing version. Starts at 1.
    pub version: u32,
    /// Canonical bare string (e.g. `precedes`) or namespaced `namespace/name` form.
    /// Serialized as `key` per RFC-006 VocabularyEntry substrate; also accepts `relationType` for backward compat.
    #[serde(rename = "key", alias = "relationType")]
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
    /// When true, source and target must resolve to the same bound Type (E4).
    /// srs-rust#910: the semanticObjectType collapse (srs#372/#383/#524,
    /// `rfc-decision-c8704763`) re-keys the retired `requireSameSemanticObjectType`
    /// onto the Type system itself. `allowedSourceTypes`/`allowedTargetTypes`
    /// retired with no successor — both were keyed on the same retired
    /// semanticObjectType string and, per #372, could never fire against a
    /// schema-conforming Record/Note (neither declares that property).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require_same_type: Option<bool>,
    /// Lifecycle status. Absent means active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RelationTypeStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Substrate escape-bag (rfc-decision-6fc7e142 / srs#433 / srs PR #510).
    /// Arbitrary metadata per substrate Change H policy. Unknown top-level
    /// fields are rejected; use this. Serializes as `meta`; still accepts
    /// the pre-rev-5 `properties` key on read (monotonic support, RFC-033) —
    /// the `substrate-properties-to-meta` migration (#5) persists the rename.
    #[serde(skip_serializing_if = "Option::is_none", alias = "properties")]
    pub meta: Option<BTreeMap<String, serde_json::Value>>,
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
            require_same_type: None,
            status: None,
            updated_at: None,
            meta: None,
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

    /// srs-rust#894 (srs#433, srs PR #510): a rev-4 repo still carrying the
    /// pre-rename `properties` key must load (monotonic support, RFC-033) —
    /// tolerated via serde alias, not rejected. Writes always use `meta`;
    /// the `substrate-properties-to-meta` migration (#5) persists the rename.
    #[test]
    fn tolerates_legacy_properties_key_on_read() {
        let json_str = r#"{
            "id": "f7a8b9c0-d1e2-4f3a-8b4c-5d6e7f8a9b0c",
            "version": 1,
            "relationType": "precedes",
            "namespace": "com.semanticops.srs",
            "label": "Precedes",
            "description": "Source comes before target.",
            "category": "sequence",
            "createdAt": "2026-05-29T00:00:00Z",
            "properties": {"color": "blue"}
        }"#;
        let rtd: RelationTypeDefinition =
            serde_json::from_str(json_str).expect("legacy `properties` key must tolerate");
        assert_eq!(
            rtd.meta.as_ref().and_then(|m| m.get("color")),
            Some(&serde_json::json!("blue"))
        );
        let serialized = serde_json::to_string(&rtd).unwrap();
        assert!(serialized.contains("\"meta\""), "writes must use `meta`");
        assert!(
            !serialized.contains("\"properties\""),
            "writes must not reintroduce the retired `properties` key"
        );
    }
}
