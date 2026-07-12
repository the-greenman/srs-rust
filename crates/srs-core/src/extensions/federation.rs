use serde::{Deserialize, Serialize};

/// Top-level federation events file (`ext:federation`).
///
/// Stored separately from the instance index so structural provenance is
/// auditable without polluting `manifest.json`. Shape mirrors
/// `federation-events.json` schema exactly.
///
/// No `deny_unknown_fields` — forward-compat per ADR-028.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationEventsFile {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub repository_id: String,
    pub events: Vec<FederationEvent>,
}

/// A single federation operation (merge, split, or import).
///
/// No `deny_unknown_fields` — forward-compat per ADR-028.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FederationEvent {
    pub event_id: String,
    pub event: FederationEventKind,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performed_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_repository_id: Option<String>,
    pub affected_instance_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<FederationStrategy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FederationEventKind {
    Merge,
    Split,
    Import,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FederationStrategy {
    PreserveIds,
    NewIdsWithLineage,
}

/// Top-level repository registry file (`ext:federation`).
///
/// Lists SRS document repositories known to this system or team. Shape mirrors
/// `federation-registry.json` schema exactly.
///
/// No `deny_unknown_fields` — forward-compat per ADR-028.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRegistry {
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub registry_id: String,
    pub title: String,
    pub updated_at: String,
    pub entries: Vec<RepositoryRegistryEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_registries: Option<Vec<String>>,
}

/// One known repository in a `RepositoryRegistry`.
///
/// No `deny_unknown_fields` — forward-compat per ADR-028.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryRegistryEntry {
    pub repository_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FederationEventKind ──────────────────────────────────────────────────

    #[test]
    fn federation_event_kind_roundtrips_json() {
        let cases = [
            (FederationEventKind::Merge, "\"merge\""),
            (FederationEventKind::Split, "\"split\""),
            (FederationEventKind::Import, "\"import\""),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "serialization mismatch for {:?}", variant);
            let parsed: FederationEventKind = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    // ── FederationStrategy ───────────────────────────────────────────────────

    #[test]
    fn federation_strategy_roundtrips_json() {
        let cases = [
            (FederationStrategy::PreserveIds, "\"preserve-ids\""),
            (
                FederationStrategy::NewIdsWithLineage,
                "\"new-ids-with-lineage\"",
            ),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "serialization mismatch for {:?}", variant);
            let parsed: FederationStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    // ── FederationEvent ──────────────────────────────────────────────────────

    fn minimal_event() -> FederationEvent {
        FederationEvent {
            event_id: "e0000001-0000-4000-a000-000000000001".to_string(),
            event: FederationEventKind::Merge,
            at: "2026-07-12T10:00:00Z".to_string(),
            performed_by: None,
            source_repository_id: Some(
                "r0000001-0000-4000-a000-000000000001".to_string(),
            ),
            target_repository_id: None,
            affected_instance_ids: vec!["i0000001-0000-4000-a000-000000000001".to_string()],
            strategy: None,
            note: None,
        }
    }

    #[test]
    fn federation_event_minimal_roundtrips_json() {
        let event = FederationEvent {
            event_id: "e0000001-0000-4000-a000-000000000001".to_string(),
            event: FederationEventKind::Import,
            at: "2026-07-12T10:00:00Z".to_string(),
            performed_by: None,
            source_repository_id: None,
            target_repository_id: None,
            affected_instance_ids: vec!["i0000001-0000-4000-a000-000000000001".to_string()],
            strategy: None,
            note: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: FederationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
        assert_eq!(parsed.event, FederationEventKind::Import);
        assert_eq!(parsed.affected_instance_ids.len(), 1);
    }

    #[test]
    fn federation_event_full_roundtrips_json() {
        let event = FederationEvent {
            event_id: "e0000002-0000-4000-a000-000000000002".to_string(),
            event: FederationEventKind::Split,
            at: "2026-07-12T10:00:00Z".to_string(),
            performed_by: Some("alice".to_string()),
            source_repository_id: None,
            target_repository_id: Some("r0000002-0000-4000-a000-000000000002".to_string()),
            affected_instance_ids: vec![
                "i0000001-0000-4000-a000-000000000001".to_string(),
                "i0000002-0000-4000-a000-000000000002".to_string(),
            ],
            strategy: Some(FederationStrategy::PreserveIds),
            note: Some("Splitting off project archive".to_string()),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: FederationEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
        assert_eq!(parsed.performed_by, Some("alice".to_string()));
        assert_eq!(parsed.strategy, Some(FederationStrategy::PreserveIds));
        assert_eq!(parsed.affected_instance_ids.len(), 2);
    }

    #[test]
    fn federation_event_omits_optional_fields() {
        let event = FederationEvent {
            event_id: "e0000001-0000-4000-a000-000000000001".to_string(),
            event: FederationEventKind::Merge,
            at: "2026-07-12T10:00:00Z".to_string(),
            performed_by: None,
            source_repository_id: None,
            target_repository_id: None,
            affected_instance_ids: vec!["i0000001-0000-4000-a000-000000000001".to_string()],
            strategy: None,
            note: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("performedBy"));
        assert!(!json.contains("sourceRepositoryId"));
        assert!(!json.contains("targetRepositoryId"));
        assert!(!json.contains("strategy"));
        assert!(!json.contains("note"));
    }

    #[test]
    fn federation_event_tolerates_unknown_field() {
        let json = r#"{
            "eventId": "e0000001-0000-4000-a000-000000000001",
            "event": "merge",
            "at": "2026-07-12T10:00:00Z",
            "affectedInstanceIds": ["i0000001-0000-4000-a000-000000000001"],
            "unknownField": "should be ignored"
        }"#;
        let result: Result<FederationEvent, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "unknown fields must be tolerated for forward compat");
    }

    // ── FederationEventsFile ─────────────────────────────────────────────────

    #[test]
    fn federation_events_file_roundtrips_json() {
        let file = FederationEventsFile {
            schema: Some(
                "https://srs.semanticops.com/schema/2.0/federation-events.json".to_string(),
            ),
            repository_id: "r0000001-0000-4000-a000-000000000001".to_string(),
            events: vec![minimal_event()],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: FederationEventsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.repository_id, file.repository_id);
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].event, FederationEventKind::Merge);
    }

    #[test]
    fn federation_events_file_tolerates_unknown_field() {
        let json = r#"{
            "repositoryId": "r0000001-0000-4000-a000-000000000001",
            "events": [],
            "unexpectedTopLevelField": "should be ignored"
        }"#;
        let result: Result<FederationEventsFile, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "unknown fields must be tolerated for forward compat");
    }

    // ── RepositoryRegistryEntry ──────────────────────────────────────────────

    fn minimal_entry() -> RepositoryRegistryEntry {
        RepositoryRegistryEntry {
            repository_id: "r0000001-0000-4000-a000-000000000001".to_string(),
            title: "Main Repo".to_string(),
            location: None,
            last_seen: None,
            tags: None,
        }
    }

    #[test]
    fn registry_entry_minimal_roundtrips_json() {
        let entry = minimal_entry();
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: RepositoryRegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.repository_id, "r0000001-0000-4000-a000-000000000001");
    }

    #[test]
    fn registry_entry_full_roundtrips_json() {
        let entry = RepositoryRegistryEntry {
            repository_id: "r0000002-0000-4000-a000-000000000002".to_string(),
            title: "Archive Repo".to_string(),
            location: Some("/repos/archive".to_string()),
            last_seen: Some("2026-07-10T09:00:00Z".to_string()),
            tags: Some(vec!["archive".to_string(), "read-only".to_string()]),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: RepositoryRegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, entry);
        assert_eq!(parsed.location, Some("/repos/archive".to_string()));
        assert_eq!(
            parsed.tags,
            Some(vec!["archive".to_string(), "read-only".to_string()])
        );
    }

    #[test]
    fn registry_entry_omits_optional_fields() {
        let entry = minimal_entry();
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("location"));
        assert!(!json.contains("lastSeen"));
        assert!(!json.contains("tags"));
        let parsed: RepositoryRegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.location, None);
        assert_eq!(parsed.tags, None);
    }

    #[test]
    fn registry_entry_tolerates_unknown_field() {
        let json = r#"{
            "repositoryId": "r0000001-0000-4000-a000-000000000001",
            "title": "Test",
            "unknownField": "should be ignored"
        }"#;
        let result: Result<RepositoryRegistryEntry, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "unknown fields must be tolerated for forward compat");
    }

    // ── RepositoryRegistry ───────────────────────────────────────────────────

    #[test]
    fn repository_registry_roundtrips_json() {
        let registry = RepositoryRegistry {
            schema: Some(
                "https://srs.semanticops.com/schema/2.0/federation-registry.json".to_string(),
            ),
            registry_id: "reg0001-0000-4000-a000-000000000001".to_string(),
            title: "Team Registry".to_string(),
            updated_at: "2026-07-12T00:00:00Z".to_string(),
            entries: vec![minimal_entry()],
            child_registries: Some(vec!["/registries/team-b.json".to_string()]),
        };
        let json = serde_json::to_string(&registry).unwrap();
        let parsed: RepositoryRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.registry_id, registry.registry_id);
        assert_eq!(parsed.title, "Team Registry");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(
            parsed.child_registries,
            Some(vec!["/registries/team-b.json".to_string()])
        );
    }

    #[test]
    fn repository_registry_omits_optional_fields() {
        let registry = RepositoryRegistry {
            schema: None,
            registry_id: "reg0001-0000-4000-a000-000000000001".to_string(),
            title: "Minimal Registry".to_string(),
            updated_at: "2026-07-12T00:00:00Z".to_string(),
            entries: vec![],
            child_registries: None,
        };
        let json = serde_json::to_string(&registry).unwrap();
        assert!(!json.contains("$schema"));
        assert!(!json.contains("childRegistries"));
        let parsed: RepositoryRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.schema, None);
        assert_eq!(parsed.child_registries, None);
    }

    #[test]
    fn repository_registry_tolerates_unknown_field() {
        let json = r#"{
            "registryId": "reg0001-0000-4000-a000-000000000001",
            "title": "Test",
            "updatedAt": "2026-07-12T00:00:00Z",
            "entries": [],
            "unknownTopLevelField": "should be ignored"
        }"#;
        let result: Result<RepositoryRegistry, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "unknown fields must be tolerated for forward compat");
    }
}
