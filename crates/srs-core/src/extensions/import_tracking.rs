use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ImportMode {
    #[default]
    UpstreamTracked,
    LocalCopy,
    LocalFork,
}

impl fmt::Display for ImportMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UpstreamTracked => write!(f, "upstream-tracked"),
            Self::LocalCopy => write!(f, "local-copy"),
            Self::LocalFork => write!(f, "local-fork"),
        }
    }
}

impl TryFrom<&str> for ImportMode {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "upstream-tracked" => Ok(Self::UpstreamTracked),
            "local-copy" => Ok(Self::LocalCopy),
            "local-fork" => Ok(Self::LocalFork),
            other => Err(format!(
                "invalid import mode '{other}': must be upstream-tracked, local-copy, or local-fork"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DefinitionType {
    Field,
    Type,
    View,
    Blueprint,
    Protocol,
    RelationType,
}

impl fmt::Display for DefinitionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field => write!(f, "field"),
            Self::Type => write!(f, "type"),
            Self::View => write!(f, "view"),
            Self::Blueprint => write!(f, "blueprint"),
            Self::Protocol => write!(f, "protocol"),
            Self::RelationType => write!(f, "relation-type"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictState {
    Clean,
    LocalAhead,
    UpstreamAhead,
    Diverged,
}

impl fmt::Display for ConflictState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clean => write!(f, "clean"),
            Self::LocalAhead => write!(f, "local-ahead"),
            Self::UpstreamAhead => write!(f, "upstream-ahead"),
            Self::Diverged => write!(f, "diverged"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRecord {
    pub definition_id: String,
    pub definition_type: DefinitionType,
    pub namespace: String,
    pub name: String,
    pub version: u32,
    pub mode: ImportMode,
    pub imported_at: String,
    pub source_package_id: String,
    pub source_package_name: String,
    pub source_package_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_known_upstream_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_checked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_state: Option<ConflictState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_detected_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_edited_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub generated_at: String,
    pub fields: Vec<ImportRecord>,
    pub types: Vec<ImportRecord>,
    pub views: Vec<ImportRecord>,
    pub blueprints: Vec<ImportRecord>,
    pub protocols: Vec<ImportRecord>,
    pub relation_types: Vec<ImportRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_definitions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamPackage {
    pub package_id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub installed_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_import_record() -> ImportRecord {
        ImportRecord {
            definition_id: "00000001-0000-4000-a000-000000000001".to_string(),
            definition_type: DefinitionType::Field,
            namespace: "com.example.gov".to_string(),
            name: "decision_title".to_string(),
            version: 1,
            mode: ImportMode::UpstreamTracked,
            imported_at: "2026-07-01T00:00:00Z".to_string(),
            source_package_id: "00000002-0000-4000-a000-000000000002".to_string(),
            source_package_name: "com.mudemocracy.governance".to_string(),
            source_package_version: "1.0.0".to_string(),
            latest_known_upstream_version: None,
            update_available: None,
            update_checked_at: None,
            conflict_state: None,
            conflict_detected_at: None,
            local_version: None,
            local_edited_at: None,
        }
    }

    #[test]
    fn import_mode_display() {
        assert_eq!(ImportMode::UpstreamTracked.to_string(), "upstream-tracked");
        assert_eq!(ImportMode::LocalCopy.to_string(), "local-copy");
        assert_eq!(ImportMode::LocalFork.to_string(), "local-fork");
    }

    #[test]
    fn import_mode_try_from_str() {
        assert_eq!(
            ImportMode::try_from("upstream-tracked"),
            Ok(ImportMode::UpstreamTracked)
        );
        assert_eq!(
            ImportMode::try_from("local-copy"),
            Ok(ImportMode::LocalCopy)
        );
        assert_eq!(
            ImportMode::try_from("local-fork"),
            Ok(ImportMode::LocalFork)
        );
        assert!(ImportMode::try_from("invalid").is_err());
    }

    #[test]
    fn definition_type_display() {
        assert_eq!(DefinitionType::Field.to_string(), "field");
        assert_eq!(DefinitionType::Type.to_string(), "type");
        assert_eq!(DefinitionType::View.to_string(), "view");
        assert_eq!(DefinitionType::Blueprint.to_string(), "blueprint");
        assert_eq!(DefinitionType::Protocol.to_string(), "protocol");
        assert_eq!(DefinitionType::RelationType.to_string(), "relation-type");
    }

    #[test]
    fn conflict_state_display() {
        assert_eq!(ConflictState::Clean.to_string(), "clean");
        assert_eq!(ConflictState::LocalAhead.to_string(), "local-ahead");
        assert_eq!(ConflictState::UpstreamAhead.to_string(), "upstream-ahead");
        assert_eq!(ConflictState::Diverged.to_string(), "diverged");
    }

    #[test]
    fn import_mode_roundtrips_json() {
        let cases = [
            (ImportMode::UpstreamTracked, "\"upstream-tracked\""),
            (ImportMode::LocalCopy, "\"local-copy\""),
            (ImportMode::LocalFork, "\"local-fork\""),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "serialization mismatch for {:?}", variant);
            let parsed: ImportMode = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn definition_type_roundtrips_json() {
        let cases = [
            (DefinitionType::Field, "\"field\""),
            (DefinitionType::Type, "\"type\""),
            (DefinitionType::View, "\"view\""),
            (DefinitionType::Blueprint, "\"blueprint\""),
            (DefinitionType::Protocol, "\"protocol\""),
            (DefinitionType::RelationType, "\"relation-type\""),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "serialization mismatch for {:?}", variant);
            let parsed: DefinitionType = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn conflict_state_roundtrips_json() {
        let cases = [
            (ConflictState::Clean, "\"clean\""),
            (ConflictState::LocalAhead, "\"local-ahead\""),
            (ConflictState::UpstreamAhead, "\"upstream-ahead\""),
            (ConflictState::Diverged, "\"diverged\""),
        ];
        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, expected, "serialization mismatch for {:?}", variant);
            let parsed: ConflictState = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn import_record_roundtrips_full() {
        let record = ImportRecord {
            definition_id: "00000001-0000-4000-a000-000000000001".to_string(),
            definition_type: DefinitionType::Type,
            namespace: "com.example.gov".to_string(),
            name: "decision".to_string(),
            version: 2,
            mode: ImportMode::LocalFork,
            imported_at: "2026-06-01T00:00:00Z".to_string(),
            source_package_id: "00000002-0000-4000-a000-000000000002".to_string(),
            source_package_name: "com.mudemocracy.governance".to_string(),
            source_package_version: "1.0.0".to_string(),
            latest_known_upstream_version: Some(3),
            update_available: Some(true),
            update_checked_at: Some("2026-07-01T00:00:00Z".to_string()),
            conflict_state: Some(ConflictState::Diverged),
            conflict_detected_at: Some("2026-07-02T00:00:00Z".to_string()),
            local_version: Some(4),
            local_edited_at: Some("2026-07-03T00:00:00Z".to_string()),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: ImportRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.definition_id, record.definition_id);
        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.latest_known_upstream_version, Some(3));
        assert_eq!(parsed.update_available, Some(true));
        assert_eq!(parsed.conflict_state, Some(ConflictState::Diverged));
        assert_eq!(parsed.local_version, Some(4));
    }

    #[test]
    fn import_record_omits_optional_fields() {
        let record = minimal_import_record();
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("latestKnownUpstreamVersion"));
        assert!(!json.contains("updateAvailable"));
        assert!(!json.contains("updateCheckedAt"));
        assert!(!json.contains("conflictState"));
        assert!(!json.contains("conflictDetectedAt"));
        assert!(!json.contains("localVersion"));
        assert!(!json.contains("localEditedAt"));
        let parsed: ImportRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.definition_id, record.definition_id);
        assert_eq!(parsed.conflict_state, None);
    }

    #[test]
    fn import_record_tolerates_unknown_fields() {
        let json = r#"{
            "definitionId": "00000001-0000-4000-a000-000000000001",
            "definitionType": "field",
            "namespace": "com.example.gov",
            "name": "title",
            "version": 1,
            "mode": "upstream-tracked",
            "importedAt": "2026-07-01T00:00:00Z",
            "sourcePackageId": "00000002-0000-4000-a000-000000000002",
            "sourcePackageName": "com.example.gov",
            "sourcePackageVersion": "1.0.0",
            "unknownFutureField": "should be ignored"
        }"#;
        let parsed: ImportRecord = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "title");
        assert_eq!(parsed.version, 1);
    }

    #[test]
    fn import_summary_roundtrips_json() {
        let record = minimal_import_record();
        let summary = ImportSummary {
            generated_at: "2026-07-12T00:00:00Z".to_string(),
            fields: vec![record.clone()],
            types: vec![],
            views: vec![],
            blueprints: vec![],
            protocols: vec![],
            relation_types: vec![record],
            skipped_definitions: vec!["view/some-view.json".to_string()],
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: ImportSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.generated_at, "2026-07-12T00:00:00Z");
        assert_eq!(parsed.fields.len(), 1);
        assert_eq!(parsed.types.len(), 0);
        assert_eq!(parsed.relation_types.len(), 1);
        assert_eq!(parsed.skipped_definitions, vec!["view/some-view.json"]);
    }

    #[test]
    fn import_summary_skipped_definitions_omitted_when_empty() {
        let summary = ImportSummary {
            generated_at: "2026-07-12T00:00:00Z".to_string(),
            fields: vec![],
            types: vec![],
            views: vec![],
            blueprints: vec![],
            protocols: vec![],
            relation_types: vec![],
            skipped_definitions: vec![],
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("skippedDefinitions"));
    }

    #[test]
    fn import_summary_tolerates_unknown_fields() {
        let json = r#"{
            "generatedAt": "2026-07-12T00:00:00Z",
            "fields": [],
            "types": [],
            "views": [],
            "blueprints": [],
            "protocols": [],
            "relationTypes": [],
            "futureTopLevelField": "should be ignored"
        }"#;
        let parsed: ImportSummary = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.generated_at, "2026-07-12T00:00:00Z");
        assert!(parsed.fields.is_empty());
        assert!(parsed.skipped_definitions.is_empty());
    }

    #[test]
    fn upstream_package_roundtrips_json() {
        let pkg = UpstreamPackage {
            package_id: "1cd9622e-3d05-4214-a683-4cb81d0c44d9".to_string(),
            namespace: "com.mudemocracy.governance".to_string(),
            name: "governance".to_string(),
            version: "1.0.0".to_string(),
            installed_at: "2026-06-28T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&pkg).unwrap();
        let parsed: UpstreamPackage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.package_id, pkg.package_id);
        assert_eq!(parsed.namespace, "com.mudemocracy.governance");
        assert_eq!(parsed.version, "1.0.0");
        assert_eq!(parsed.installed_at, pkg.installed_at);
    }

    #[test]
    fn upstream_package_tolerates_unknown_fields() {
        let json = r#"{
            "packageId": "1cd9622e-3d05-4214-a683-4cb81d0c44d9",
            "namespace": "com.mudemocracy.governance",
            "name": "governance",
            "version": "1.0.0",
            "installedAt": "2026-06-28T12:00:00Z",
            "futureField": "should be ignored"
        }"#;
        let parsed: UpstreamPackage = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.name, "governance");
        assert_eq!(parsed.version, "1.0.0");
    }
}
