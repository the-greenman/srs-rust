use serde::{Deserialize, Serialize};

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
    pub asserted_by: Option<AssertedBy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<RelationStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<Vec<SourceReference>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_repository_id: Option<String>,
}

/// The top-level relations collection file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationsCollection {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssertedBy {
    Human,
    Ai,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelationStatus {
    Proposed,
    Active,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceReference {
    pub source_type: SourceType,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_standard: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_type: Option<SourceRelationType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceType {
    TranscriptChunk,
    TranscriptSegment,
    ExternalDocument,
    RepositoryDocument,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceRelationType {
    Evidence,
    DerivedFrom,
    QuotedFrom,
    InspiredBy,
    SupersedesContext,
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
            asserted_by: None,
            confidence: None,
            created_at: Some("2026-05-29T00:00:00Z".to_string()),
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
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
            "assertedBy": "human",
            "confidence": 0.8,
            "status": "active",
            "sourceRefs": [{
                "sourceType": "repository-document",
                "sourceId": "doc-1"
            }],
            "meta": {"k":"v"}
        }"#;
        let relation: Relation = serde_json::from_str(json).unwrap();
        assert_eq!(relation.asserted_by, Some(AssertedBy::Human));
        assert_eq!(relation.status, Some(RelationStatus::Active));
        assert!(relation.source_refs.is_some());
    }
}
