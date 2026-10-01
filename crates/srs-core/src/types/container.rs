use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    #[serde(default)]
    pub container_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_instance_id: Option<String>,
    /// RFC-009 (amended by srs#446). The instance id of the record whose Type is this
    /// Container's typing anchor — what RFC-009 `rootTypeRefs` matching (I-63) and RFC-010's
    /// (Draft) three-way-merge conflict detection resolve against. Declared, never positional
    /// (`rfc-decision-cce3c00e`, cell Containment: "declaration over location"). When present,
    /// MUST equal a member id in `memberInstanceIds` (I-145, RFC-043 [R3]). The
    /// `rootInstanceIds[0]` fallback is withdrawn (RFC-043 [R4]). RFC-010 merge semantics (once an
    /// engine exists): merges as a declared scalar, the same as `identityInstanceId` — a
    /// `container-root` conflict on divergence, never resolved by precedence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anchor_instance_id: Option<String>,
    /// RFC-043 [R1]: the ordered outline of this container's members. Flat ids are
    /// `direct(C)` ([`Container::member_ids`]); array order and `depth` are layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_instance_ids: Option<Vec<ContainerEntry>>,
    /// RFC-034 Change B. Container ids of directly nested child scopes — the one
    /// place a Container references another Container. Effective membership
    /// (`effective(C)`) is the recursive, deduplicated closure over this edge;
    /// every entry MUST resolve to an existing, distinct Container and the graph
    /// MUST be acyclic (RFC-034 [R7]). Holds containerIds, not instance ids.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_container_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// RFC-043 [R1]: one entry of `Container.memberInstanceIds` — a member id and an optional
/// nesting `depth` (absent means 0). No other property is allowed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContainerEntry {
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
}

impl ContainerEntry {
    pub fn new(instance_id: impl Into<String>) -> Self {
        Self {
            instance_id: instance_id.into(),
            depth: None,
        }
    }

    pub fn at(instance_id: impl Into<String>, depth: u32) -> Self {
        Self {
            instance_id: instance_id.into(),
            // Writers SHOULD omit 0 ([R1]).
            depth: (depth > 0).then_some(depth),
        }
    }

    pub fn depth(&self) -> u32 {
        self.depth.unwrap_or(0)
    }
}

/// Flat entries (depth 0) from bare ids — the common construction in callers and tests.
pub fn entries<I, S>(ids: I) -> Vec<ContainerEntry>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    ids.into_iter().map(ContainerEntry::new).collect()
}

impl Container {
    /// `direct(C)` read flat (RFC-043 [R3]): the entry ids in array order, depth ignored.
    pub fn member_ids(&self) -> Vec<String> {
        self.member_instance_ids
            .iter()
            .flatten()
            .map(|e| e.instance_id.clone())
            .collect()
    }

    pub fn has_member(&self, instance_id: &str) -> bool {
        self.member_instance_ids
            .iter()
            .flatten()
            .any(|e| e.instance_id == instance_id)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerIndexEntry {
    // Intentionally not #[serde(default)]: a missing containerId is a malformed index entry
    // and should fail deserialization, unlike Container.container_id which defaults to "".
    pub container_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_roundtrips_all_fields() {
        let mut extra = BTreeMap::new();
        extra.insert("xCustom".to_string(), serde_json::json!("value"));
        let container = Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Sprint 1".to_string(),
            namespace: Some("team".to_string()),
            name: Some("sprint-1".to_string()),
            description: Some("desc".to_string()),
            container_type: Some("project".to_string()),
            identity_instance_id: Some("aaaaaaaa-0000-4000-8000-aaaaaaaaaaaa".to_string()),
            anchor_instance_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            member_instance_ids: Some(vec![ContainerEntry::new(
                "22222222-2222-4222-8222-222222222222",
            )]),
            child_container_ids: Some(vec!["33333333-3333-4333-8333-333333333333".to_string()]),
            tags: Some(vec!["alpha".to_string()]),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: Some("2026-01-02T00:00:00Z".to_string()),
            meta: Some(serde_json::json!({"k":"v"})),
            extra,
        };

        let json = serde_json::to_string(&container).unwrap();
        let parsed: Container = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, container);
    }

    #[test]
    fn container_minimal_roundtrips() {
        let container = Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Minimal".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            member_instance_ids: None,
            child_container_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: BTreeMap::new(),
        };

        let value = serde_json::to_value(&container).unwrap();
        assert!(value.get("namespace").is_none());
        assert!(value.get("memberInstanceIds").is_none());
        assert!(value.get("childContainerIds").is_none());
        assert!(value.get("anchorInstanceId").is_none());

        let parsed: Container = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, container);
    }

    #[test]
    fn container_extra_fields_survive() {
        let value = serde_json::json!({
            "containerId": "550e8400-e29b-41d4-a716-446655440000",
            "title": "Extra",
            "xOne": 1,
            "xTwo": {"a": true}
        });

        let parsed: Container = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.extra.get("xOne"), Some(&serde_json::json!(1)));
        assert_eq!(
            parsed.extra.get("xTwo"),
            Some(&serde_json::json!({"a": true}))
        );
    }

    #[test]
    fn container_missing_container_id_defaults_to_empty() {
        let value = serde_json::json!({
            "title": "No ID Provided"
        });
        let parsed: Container = serde_json::from_value(value).unwrap();
        assert_eq!(parsed.container_id, "");
        assert_eq!(parsed.title, "No ID Provided");
    }

    #[test]
    fn container_index_entry_roundtrips() {
        // Full entry — all fields survive round-trip
        let mut extra = BTreeMap::new();
        extra.insert("xFoo".to_string(), serde_json::json!(42));
        let entry = ContainerIndexEntry {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: Some("My Container".to_string()),
            path: Some("containers/my-container.json".to_string()),
            container_type: Some("section".to_string()),
            tags: Some(vec!["tag1".to_string()]),
            extra,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ContainerIndexEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);

        // Minimal entry — only containerId required
        let minimal = ContainerIndexEntry {
            container_id: "11111111-1111-4111-8111-111111111111".to_string(),
            title: None,
            path: None,
            container_type: None,
            tags: None,
            extra: BTreeMap::new(),
        };
        let value = serde_json::to_value(&minimal).unwrap();
        assert!(value.get("title").is_none());
        assert!(value.get("path").is_none());
        let parsed: ContainerIndexEntry = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, minimal);
    }

    #[test]
    fn minimal_container_passes_schema_contract() {
        let container = Container {
            container_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            title: "Sprint 1".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            anchor_instance_id: None,
            member_instance_ids: None,
            child_container_ids: None,
            tags: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
            meta: None,
            extra: BTreeMap::new(),
        };
        let mut value = serde_json::to_value(&container).unwrap();
        value["$schema"] =
            serde_json::json!("https://srs.semanticops.com/schema/2.0/container.json");
        srs_schema::SchemaRegistry::global()
            .validate_by_id(srs_schema::CONTAINER_SCHEMA_ID, &value)
            .expect("minimal Container must pass container.json schema");
    }
}
