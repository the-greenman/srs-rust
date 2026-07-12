use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub package_id: String,
    pub package_name: String,
    pub package_version: String,
    pub publisher: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub published_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub field_count: u32,
    pub type_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation_type_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    /// SHA-256 hex digest for integrity verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub schema_version: String,
    pub registry_id: String,
    pub registry_name: String,
    /// Registry's own version (semver).
    pub catalog_version: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub entries: Vec<RegistryEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_entry() -> RegistryEntry {
        RegistryEntry {
            package_id: "00000001-0000-4000-a000-000000000001".to_string(),
            package_name: "com.example.governance".to_string(),
            package_version: "1.0.0".to_string(),
            publisher: "example.com".to_string(),
            description: None,
            published_at: "2026-01-01T00:00:00Z".to_string(),
            homepage: None,
            tags: None,
            field_count: 10,
            type_count: 3,
            view_count: None,
            schema_count: None,
            protocol_count: None,
            relation_type_count: None,
            download_url: None,
            checksum: None,
        }
    }

    #[test]
    fn registry_entry_roundtrips_json() {
        let entry = RegistryEntry {
            package_id: "00000001-0000-4000-a000-000000000001".to_string(),
            package_name: "com.example.governance".to_string(),
            package_version: "1.2.3".to_string(),
            publisher: "example.com".to_string(),
            description: Some("A governance package".to_string()),
            published_at: "2026-06-01T12:00:00Z".to_string(),
            homepage: Some("https://example.com/governance".to_string()),
            tags: Some(vec!["governance".to_string(), "official".to_string()]),
            field_count: 15,
            type_count: 5,
            view_count: Some(2),
            schema_count: Some(1),
            protocol_count: Some(0),
            relation_type_count: Some(3),
            download_url: Some("https://example.com/governance-1.2.3.srsj".to_string()),
            checksum: Some(
                "abc123def456abc123def456abc123def456abc123def456abc123def456abc12345".to_string(),
            ),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: RegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.package_name, "com.example.governance");
        assert_eq!(parsed.package_version, "1.2.3");
        assert_eq!(parsed.description, Some("A governance package".to_string()));
        assert_eq!(parsed.field_count, 15);
        assert_eq!(parsed.view_count, Some(2));
        assert_eq!(parsed.tags, Some(vec!["governance".to_string(), "official".to_string()]));
        assert_eq!(parsed.checksum, entry.checksum);
    }

    #[test]
    fn registry_entry_omits_optional_fields() {
        let entry = minimal_entry();
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("description"));
        assert!(!json.contains("homepage"));
        assert!(!json.contains("tags"));
        assert!(!json.contains("viewCount"));
        assert!(!json.contains("schemaCount"));
        assert!(!json.contains("protocolCount"));
        assert!(!json.contains("relationTypeCount"));
        assert!(!json.contains("downloadUrl"));
        assert!(!json.contains("checksum"));
        let parsed: RegistryEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.package_name, "com.example.governance");
        assert_eq!(parsed.view_count, None);
    }

    #[test]
    fn registry_roundtrips_json() {
        let registry = Registry {
            schema_version: "1.0".to_string(),
            registry_id: "00000002-0000-4000-a000-000000000002".to_string(),
            registry_name: "Example Registry".to_string(),
            catalog_version: "2.1.0".to_string(),
            updated_at: "2026-07-01T00:00:00Z".to_string(),
            homepage: Some("https://registry.example.com".to_string()),
            entries: vec![minimal_entry(), minimal_entry()],
        };
        let json = serde_json::to_string(&registry).unwrap();
        let parsed: Registry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.registry_name, "Example Registry");
        assert_eq!(parsed.catalog_version, "2.1.0");
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].package_name, "com.example.governance");
        assert_eq!(parsed.homepage, Some("https://registry.example.com".to_string()));
    }

    #[test]
    fn registry_tolerates_unknown_fields() {
        let json = r#"{
            "schemaVersion": "1.0",
            "registryId": "00000002-0000-4000-a000-000000000002",
            "registryName": "Example Registry",
            "catalogVersion": "1.0.0",
            "updatedAt": "2026-07-01T00:00:00Z",
            "entries": [],
            "futureField": "should be ignored"
        }"#;
        let parsed: Registry = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.registry_name, "Example Registry");
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn registry_entry_tolerates_unknown_fields() {
        let json = r#"{
            "packageId": "00000001-0000-4000-a000-000000000001",
            "packageName": "com.example.governance",
            "packageVersion": "1.0.0",
            "publisher": "example.com",
            "publishedAt": "2026-01-01T00:00:00Z",
            "fieldCount": 5,
            "typeCount": 2,
            "futureEntryField": "should be ignored"
        }"#;
        let parsed: RegistryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.package_name, "com.example.governance");
        assert_eq!(parsed.field_count, 5);
    }
}
