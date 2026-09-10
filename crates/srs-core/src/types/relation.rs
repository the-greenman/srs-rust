use serde::{Deserialize, Serialize};

pub use super::source_reference::{SourceReference, SourceType};

/// A flat relation record as stored in `relations-collection.json`.
///
/// Shape matches the `relations-collection.json` schema exactly.
/// `additionalProperties: false` is enforced by `deny_unknown_fields`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Relation {
    #[serde(default)]
    pub relation_id: String,
    pub relation_type: String,
    pub source_instance_id: String,
    pub target_instance_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    // SourceReference omits deny_unknown_fields intentionally (forward-compat);
    // Relation's own deny_unknown_fields does not propagate into nested items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<Vec<SourceReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// The top-level relations collection file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationsCollection {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub relations: Vec<Relation>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_roundtrips_json() {
        let r = Relation {
            relation_id: "d0000001-0000-4000-a000-000000000001".to_string(),
            relation_type: "precedes".to_string(),
            source_instance_id: "aaaa0001-0000-4000-a000-000000000001".to_string(),
            target_instance_id: "aaaa0002-0000-4000-a000-000000000002".to_string(),
            created_at: Some("2026-05-29T00:00:00Z".to_string()),
            notes: None,
            source_refs: None,
            meta: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: Relation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn relation_unknown_field_fails_deserialization() {
        let json = r#"{
            "relationId": "d0000001-0000-4000-a000-000000000001",
            "relationType": "precedes",
            "sourceInstanceId": "aaaa0001-0000-4000-a000-000000000001",
            "targetInstanceId": "aaaa0002-0000-4000-a000-000000000002",
            "createdAt": "2026-05-29T00:00:00Z",
            "unknownField": "bad"
        }"#;
        let result: Result<Relation, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown field should fail");
    }

    #[test]
    fn relations_collection_parses_array() {
        let json = r#"{
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": [
                {
                    "relationId": "d0000001-0000-4000-a000-000000000001",
                    "relationType": "precedes",
                    "sourceInstanceId": "aaaa0001-0000-4000-a000-000000000001",
                    "targetInstanceId": "aaaa0002-0000-4000-a000-000000000002"
                }
            ]
        }"#;
        let coll: RelationsCollection = serde_json::from_str(json).unwrap();
        assert_eq!(coll.relations.len(), 1);
        assert_eq!(coll.relations[0].relation_type, "precedes");
    }

    #[test]
    fn relation_with_optional_schema_fields_parses() {
        let json = r#"{
            "relationId": "d0000001-0000-4000-a000-000000000001",
            "relationType": "precedes",
            "sourceInstanceId": "aaaa0001-0000-4000-a000-000000000001",
            "targetInstanceId": "aaaa0002-0000-4000-a000-000000000002",
            "sourceRefs": [{
                "sourceType": "repository-document",
                "sourceId": "doc-1"
            }],
            "meta": {"k":"v"}
        }"#;
        let relation: Relation = serde_json::from_str(json).unwrap();
        assert!(relation.source_refs.is_some());
    }

    /// Regression test for srs-rust#1022: `assertedBy`, `confidence`,
    /// `status`, `createdBy`, `validFrom`, `validUntil`, `sourceRepositoryId`,
    /// `targetRepositoryId` were removed from the canonical schema by srs#441
    /// but survived on the Rust struct — so `srs relation create` accepted
    /// them at parse time and only failed later, at the repository's schema
    /// gate, with a confusing "Additional properties are not allowed" error.
    /// Now the authoring surface itself rejects them, matching what it can
    /// actually persist.
    #[test]
    fn relation_rejects_fields_removed_by_srs_441() {
        for field in [
            r#""assertedBy": "human""#,
            r#""confidence": 0.8"#,
            r#""status": "active""#,
            r#""createdBy": "someone""#,
            r#""validFrom": "2026-01-01T00:00:00Z""#,
            r#""validUntil": "2026-12-31T00:00:00Z""#,
            r#""sourceRepositoryId": "repo-a""#,
            r#""targetRepositoryId": "repo-b""#,
        ] {
            let json = format!(
                r#"{{
                    "relationId": "d0000001-0000-4000-a000-000000000001",
                    "relationType": "precedes",
                    "sourceInstanceId": "aaaa0001-0000-4000-a000-000000000001",
                    "targetInstanceId": "aaaa0002-0000-4000-a000-000000000002",
                    {field}
                }}"#
            );
            let result: Result<Relation, _> = serde_json::from_str(&json);
            assert!(
                result.is_err(),
                "Relation must reject {field} — it was removed from the canonical schema by srs#441"
            );
        }
    }

    /// Struct/schema property-parity guard (srs-rust#777 pattern). Exhaustive
    /// destructure: the compiler forces this test to be touched the moment
    /// `Relation` gains or loses a field, so the property lists below cannot
    /// silently drift the way the removed fields above did.
    ///
    /// Compared against `RELATIONS_COLLECTION_SCHEMA_ID`'s embedded `Relation`
    /// def — the schema `schema_validate_relation` (srs-repository) actually
    /// enforces at create time today, per its own code comment. The standalone
    /// `relation.json` mirror (`RELATION_SCHEMA_ID`) additionally requires
    /// `$schema`, which `Relation` has no field for yet — that gap is tracked
    /// separately as srs-rust#1021 and is out of scope here.
    #[test]
    fn relation_struct_matches_relations_collection_relation_def_property_set() {
        let sample = Relation {
            relation_id: "d0000001-0000-4000-a000-000000000001".to_string(),
            relation_type: "precedes".to_string(),
            source_instance_id: "aaaa0001-0000-4000-a000-000000000001".to_string(),
            target_instance_id: "aaaa0002-0000-4000-a000-000000000002".to_string(),
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
        };
        let Relation {
            relation_id: _,
            relation_type: _,
            source_instance_id: _,
            target_instance_id: _,
            created_at: _,
            notes: _,
            source_refs: _,
            meta: _,
        } = sample;

        srs_schema::conformance::assert_property_parity(
            srs_schema::RELATIONS_COLLECTION_SCHEMA_ID,
            Some("Relation"),
            &[
                "relationId",
                "relationType",
                "sourceInstanceId",
                "targetInstanceId",
            ],
            &["createdAt", "notes", "sourceRefs", "meta"],
        )
        .unwrap_or_else(|report| panic!("Relation vs relations-collection.json Relation def: {report}"));
    }
}
