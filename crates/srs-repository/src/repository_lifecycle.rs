use crate::error::RepositoryError;
use crate::services::create_note;
use crate::store::RepositoryStore;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srs_core::types::container::Container;
use srs_core::types::note::{Note, NoteSection};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryMetadata {
    pub repository_id: String,
    pub namespace: String,
    pub srs_version: String,
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryPackageMetadata {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRepositoryInput {
    pub repository: RepositoryMetadata,
    pub primary_package: PrimaryPackageMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryResult {
    pub repo_root: PathBuf,
    pub repository_id: String,
    pub package_id: String,
    pub root_note_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatus {
    pub exists: bool,
}

/// Build the initial container for a newly created repository.
/// Business rule: containerId = repositoryId, title = the repository's effective title.
pub(crate) fn default_repository_container(container_id: &str, title: &str) -> Container {
    Container {
        container_id: container_id.to_string(),
        title: title.to_string(),
        namespace: None,
        name: None,
        description: None,
        container_type: None,
        identity_instance_id: None,
        root_instance_ids: None,
        member_instance_ids: None,
        tags: None,
        created_at: None,
        updated_at: None,
        meta: None,
        extra: std::collections::HashMap::new(),
    }
}

pub fn create_repository(
    store: &dyn RepositoryStore,
    input: &InitializeRepositoryInput,
) -> Result<CreateRepositoryResult, RepositoryError> {
    validate_initialize_input(input)?;

    if store.repository_exists()? {
        return Err(RepositoryError::RepositoryAlreadyExists {
            path: store.repository_root(),
        });
    }

    // Normalize title: if not provided, fall back to namespace (business rule, not adapter concern).
    let mut resolved = input.clone();
    if resolved.repository.title.is_none() {
        resolved.repository.title = Some(input.repository.namespace.clone());
    }
    store.initialize_repository(&resolved)
}

/// Create a repository and optionally create a root intent note when name or
/// description is provided. Both operations share the same store so they are
/// executed atomically from the store's perspective. The repository is written
/// first; if note creation fails the caller receives the error and the repo
/// directory will exist (no rollback), which is the intended behaviour — the
/// caller can retry the note creation separately.
pub fn create_repository_with_intent(
    store: &dyn RepositoryStore,
    input: &InitializeRepositoryInput,
) -> Result<CreateRepositoryResult, RepositoryError> {
    let mut result = create_repository(store, input)?;

    let title = input.repository.title.clone();
    let description = input.repository.description.clone();

    if title.is_some() || description.is_some() {
        let title = title.unwrap_or_else(|| "Repository Intent".to_string());
        let content = description.unwrap_or_default();
        let note = Note {
            instance_id: String::new(),
            title: Some(title),
            tags: Some(vec!["intent".to_string()]),
            sections: vec![NoteSection {
                name: "intent".to_string(),
                label: None,
                content,
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };
        let note_result = create_note(store, note)?;
        result.root_note_id = Some(note_result.note.instance_id);
    }

    Ok(result)
}

pub fn get_repository_status(
    store: &dyn RepositoryStore,
) -> Result<RepositoryStatus, RepositoryError> {
    Ok(RepositoryStatus {
        exists: store.repository_exists()?,
    })
}

/// Input for re-stamping a seed repository's identity.
///
/// # ID generation
/// `repository_id: None` triggers UUID v4 auto-mint inside the service, so
/// WASM and other non-CLI callers benefit without needing UUID generation logic
/// of their own. This intentionally differs from `create_repository`, which
/// requires the caller to pre-mint an ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitNewRepositoryInput {
    pub repository_id: Option<String>,
    pub namespace: String,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitNewRepositoryResult {
    pub repository_id: String,
    pub namespace: String,
    pub package_id: String,
    pub package_version: String,
}

/// Re-stamp a seed repository's identity and update `installedAt` on its upstream package.
///
/// The store must already contain a manifest with an `upstreamPackage` object at either
/// the RFC-014 top-level location or the pre-RFC-014 `meta.upstreamPackage` location.
/// The top-level location is tried first (RFC-014-migrated stores); `meta.upstreamPackage`
/// is the fallback for pre-RFC-014 seeds. All other `upstreamPackage` fields are preserved.
pub fn init_new_repository(
    store: &dyn RepositoryStore,
    input: InitNewRepositoryInput,
) -> Result<InitNewRepositoryResult, RepositoryError> {
    if input.namespace.trim().is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "namespace must not be empty".to_string(),
        });
    }
    if input.title.trim().is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "title must not be empty".to_string(),
        });
    }

    let repository_id = input
        .repository_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut manifest = store.load_manifest()?;

    manifest.extra.insert(
        "repositoryId".to_string(),
        Value::String(repository_id.clone()),
    );
    manifest.extra.insert(
        "namespace".to_string(),
        Value::String(input.namespace.clone()),
    );
    manifest
        .extra
        .insert("title".to_string(), Value::String(input.title));
    if let Some(d) = input.description {
        manifest
            .extra
            .insert("description".to_string(), Value::String(d));
    }

    // Stamp installedAt at whichever location carries upstreamPackage.
    // RFC-014-migrated stores (all governance seeds after migrate_rfc014) have it at top level;
    // pre-RFC-014 seeds have it under meta.upstreamPackage.
    if let Some(up) = manifest
        .extra
        .get_mut("upstreamPackage")
        .and_then(|v| v.as_object_mut())
    {
        up.insert(
            "installedAt".to_string(),
            Value::String(Utc::now().to_rfc3339()),
        );
    } else {
        let meta_val = manifest.extra.get_mut("meta").ok_or_else(|| {
            RepositoryError::InvalidRepositoryInitialization {
                message:
                    "upstreamPackage is absent — store must be a seed with upstream provenance"
                        .to_string(),
            }
        })?;
        let upstream = meta_val
            .get_mut("upstreamPackage")
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
                message: "meta.upstreamPackage is absent or not an object".to_string(),
            })?;
        upstream.insert(
            "installedAt".to_string(),
            Value::String(Utc::now().to_rfc3339()),
        );
    }

    store.save_manifest(&manifest)?;

    let pkg = store.load_package()?;
    Ok(InitNewRepositoryResult {
        repository_id,
        namespace: input.namespace,
        package_id: pkg.id,
        package_version: pkg.version,
    })
}

fn validate_initialize_input(input: &InitializeRepositoryInput) -> Result<(), RepositoryError> {
    let checks = [
        (
            "repository.repository_id",
            input.repository.repository_id.trim(),
        ),
        ("repository.namespace", input.repository.namespace.trim()),
        (
            "repository.srs_version",
            input.repository.srs_version.trim(),
        ),
        ("primary_package.id", input.primary_package.id.trim()),
        (
            "primary_package.namespace",
            input.primary_package.namespace.trim(),
        ),
        ("primary_package.name", input.primary_package.name.trim()),
        (
            "primary_package.version",
            input.primary_package.version.trim(),
        ),
    ];
    for (field, value) in checks {
        if value.is_empty() {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: format!("{field} must not be empty"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use crate::store::{FileStore, RepositoryStore};
    use tempfile::TempDir;

    fn input() -> InitializeRepositoryInput {
        InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "repo-1".to_string(),
                namespace: "com.semanticops.test".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: None,
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg-1".to_string(),
                namespace: "com.semanticops.test".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        }
    }

    #[test]
    fn create_repository_service_initializes_memory_store() {
        let store = MemoryStore::uninitialized();
        let result = create_repository(&store, &input()).unwrap();
        assert_eq!(result.repo_root, std::path::PathBuf::from("/memory"));

        let package = store.load_package().unwrap();
        assert_eq!(package.id, "pkg-1");
    }

    #[test]
    fn create_repository_service_initializes_filestore() {
        let tmp = TempDir::new().unwrap();
        let store = FileStore::new(tmp.path());

        create_repository(&store, &input()).unwrap();
        assert!(tmp.path().join(".srs").is_dir());
        assert!(tmp.path().join("manifest.json").is_file());
        assert!(tmp.path().join("package/package.json").is_file());

        let package = store.load_package().unwrap();
        assert_eq!(package.id, "pkg-1");
        assert!(package.fields.is_empty());
    }

    #[test]
    fn create_repository_service_rejects_duplicate() {
        let store = MemoryStore::uninitialized();
        create_repository(&store, &input()).unwrap();

        let second = create_repository(&store, &input());
        assert!(matches!(
            second,
            Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }

    #[test]
    fn create_repository_service_rejects_invalid_metadata() {
        let store = MemoryStore::uninitialized();
        let mut bad = input();
        bad.repository.namespace = " ".to_string();

        let result = create_repository(&store, &bad);
        assert!(matches!(
            result,
            Err(RepositoryError::InvalidRepositoryInitialization { .. })
        ));
    }

    // ── init_new_repository tests ─────────────────────────────────────────────

    /// Build an empty MemoryStore and stamp meta.upstreamPackage into its manifest.
    fn seed_memory_store() -> MemoryStore {
        let store = MemoryStore::empty();
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "repositoryId".to_string(),
            serde_json::json!("seed-repo-id"),
        );
        manifest.extra.insert(
            "namespace".to_string(),
            serde_json::json!("com.mudemocracy.governance"),
        );
        manifest
            .extra
            .insert("title".to_string(), serde_json::json!("Seed"));
        manifest.extra.insert(
            "meta".to_string(),
            serde_json::json!({
                "upstreamPackage": {
                    "packageId": "pkg-upstream-001",
                    "namespace": "com.mudemocracy.governance",
                    "name": "Governance",
                    "version": "1.0.0",
                    "installedAt": ""
                }
            }),
        );
        store.save_manifest(&manifest).unwrap();
        store
    }

    fn seed_srsj() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "seed-repo-id",
                "srsVersion": "2.0-draft",
                "namespace": "com.mudemocracy.governance",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"},
                "meta": {
                    "upstreamPackage": {
                        "packageId": "pkg-upstream-001",
                        "namespace": "com.mudemocracy.governance",
                        "name": "Governance",
                        "version": "1.0.0",
                        "installedAt": ""
                    }
                }
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-upstream-001",
                    "namespace": "com.mudemocracy.governance",
                    "name": "Governance",
                    "version": "1.0.0",
                    "fields": [], "types": [], "relationTypes": [],
                    "views": [], "documentViews": []
                }
            }
        })
        .to_string()
    }

    /// RFC-014 format: `upstreamPackage` at top level (not under `meta`), plus `contentHash`.
    fn seed_rfc014_srsj() -> String {
        serde_json::json!({
            "srsj": "1",
            "manifest": {
                "repositoryId": "seed-repo-id",
                "srsVersion": "2.0-draft",
                "namespace": "com.mudemocracy.governance",
                "instanceIndex": [],
                "packageRef": {"mode": "local", "path": "package"},
                "upstreamPackage": {
                    "packageId": "pkg-upstream-001",
                    "namespace": "com.mudemocracy.governance",
                    "name": "Governance",
                    "version": "1.0.0",
                    "contentHash": "sha256:abc123",
                    "installedAt": ""
                }
            },
            "data": {
                "package/package.json": {
                    "id": "pkg-upstream-001",
                    "namespace": "com.mudemocracy.governance",
                    "name": "Governance",
                    "version": "1.0.0",
                    "fields": [], "types": [], "relationTypes": [],
                    "views": [], "documentViews": []
                }
            }
        })
        .to_string()
    }

    #[test]
    fn init_new_repository_updates_identity_on_memory_store() {
        let store = seed_memory_store();
        let input = super::InitNewRepositoryInput {
            repository_id: None,
            namespace: "com.example.test".to_string(),
            title: "Test Repo".to_string(),
            description: Some("A test description".to_string()),
        };

        let result = super::init_new_repository(&store, input).unwrap();

        assert_ne!(result.repository_id, "seed-repo-id", "should have a new ID");
        assert_eq!(result.namespace, "com.example.test");

        let manifest = store.load_manifest().unwrap();
        let installed_at = manifest.extra["meta"]["upstreamPackage"]["installedAt"]
            .as_str()
            .unwrap();
        assert!(!installed_at.is_empty(), "installedAt should be set");
        assert!(installed_at.contains('T'), "installedAt should be ISO-8601");

        // Other upstreamPackage fields unchanged
        assert_eq!(
            manifest.extra["meta"]["upstreamPackage"]["packageId"]
                .as_str()
                .unwrap(),
            "pkg-upstream-001"
        );
        assert_eq!(
            manifest.extra["meta"]["upstreamPackage"]["namespace"]
                .as_str()
                .unwrap(),
            "com.mudemocracy.governance"
        );

        // Description persisted
        assert_eq!(
            manifest.extra["description"].as_str().unwrap(),
            "A test description"
        );
    }

    #[test]
    fn init_new_repository_roundtrips_via_json_store() {
        let store = crate::json_store::JsonStore::from_srsj(&seed_srsj()).unwrap();
        let input = super::InitNewRepositoryInput {
            repository_id: Some("new-fixed-uuid".to_string()),
            namespace: "com.example.roundtrip".to_string(),
            title: "Roundtrip Test".to_string(),
            description: None,
        };

        let result = super::init_new_repository(&store, input).unwrap();
        assert_eq!(result.repository_id, "new-fixed-uuid");
        assert_eq!(result.package_id, "pkg-upstream-001");
        assert_eq!(result.package_version, "1.0.0");

        let serialized = store.to_srsj_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            parsed["manifest"]["repositoryId"].as_str().unwrap(),
            "new-fixed-uuid"
        );
        assert_eq!(
            parsed["manifest"]["namespace"].as_str().unwrap(),
            "com.example.roundtrip"
        );
        assert_eq!(
            parsed["manifest"]["title"].as_str().unwrap(),
            "Roundtrip Test"
        );

        // Provenance preserved
        assert_eq!(
            parsed["manifest"]["meta"]["upstreamPackage"]["packageId"]
                .as_str()
                .unwrap(),
            "pkg-upstream-001"
        );
        let installed_at = parsed["manifest"]["meta"]["upstreamPackage"]["installedAt"]
            .as_str()
            .unwrap();
        assert!(
            !installed_at.is_empty(),
            "installedAt should be set after init"
        );
    }

    #[test]
    fn init_new_repository_rejects_missing_upstream_package() {
        // MemoryStore::empty() has an empty manifest — no meta.upstreamPackage
        let store = MemoryStore::empty();
        let input = super::InitNewRepositoryInput {
            repository_id: None,
            namespace: "com.example".to_string(),
            title: "Test".to_string(),
            description: None,
        };

        let err = super::init_new_repository(&store, input).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRepositoryInitialization { .. }),
            "expected InvalidRepositoryInitialization, got {:?}",
            err
        );
    }

    #[test]
    fn init_new_repository_rejects_empty_namespace() {
        let store = seed_memory_store();
        let input = super::InitNewRepositoryInput {
            repository_id: None,
            namespace: " ".to_string(),
            title: "Test".to_string(),
            description: None,
        };

        let err = super::init_new_repository(&store, input).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRepositoryInitialization { .. }),
            "expected InvalidRepositoryInitialization, got {:?}",
            err
        );
    }

    #[test]
    fn init_new_repository_rejects_empty_title() {
        let store = seed_memory_store();
        let input = super::InitNewRepositoryInput {
            repository_id: None,
            namespace: "com.example".to_string(),
            title: " ".to_string(),
            description: None,
        };

        let err = super::init_new_repository(&store, input).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRepositoryInitialization { .. }),
            "expected InvalidRepositoryInitialization, got {:?}",
            err
        );
    }

    #[test]
    fn init_new_repository_rejects_meta_without_upstream_package() {
        let store = MemoryStore::empty();
        let mut manifest = store.load_manifest().unwrap();
        manifest
            .extra
            .insert("meta".to_string(), serde_json::json!({}));
        store.save_manifest(&manifest).unwrap();

        let input = super::InitNewRepositoryInput {
            repository_id: None,
            namespace: "com.example".to_string(),
            title: "Test".to_string(),
            description: None,
        };

        let err = super::init_new_repository(&store, input).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRepositoryInitialization { .. }),
            "expected InvalidRepositoryInitialization, got {:?}",
            err
        );
    }

    #[test]
    fn init_new_repository_rejects_upstream_package_non_object() {
        let store = MemoryStore::empty();
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "meta".to_string(),
            serde_json::json!({"upstreamPackage": "not-an-object"}),
        );
        store.save_manifest(&manifest).unwrap();

        let input = super::InitNewRepositoryInput {
            repository_id: None,
            namespace: "com.example".to_string(),
            title: "Test".to_string(),
            description: None,
        };

        let err = super::init_new_repository(&store, input).unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRepositoryInitialization { .. }),
            "expected InvalidRepositoryInitialization, got {:?}",
            err
        );
    }

    #[test]
    fn init_new_repository_handles_rfc014_top_level_upstream_package() {
        // Post-RFC-014 store: upstreamPackage at top level, meta.upstreamPackage absent.
        let store = MemoryStore::empty();
        let mut manifest = store.load_manifest().unwrap();
        manifest.extra.insert(
            "repositoryId".to_string(),
            serde_json::json!("seed-repo-id"),
        );
        manifest.extra.insert(
            "namespace".to_string(),
            serde_json::json!("com.mudemocracy.governance"),
        );
        manifest
            .extra
            .insert("title".to_string(), serde_json::json!("Seed"));
        manifest.extra.insert(
            "upstreamPackage".to_string(),
            serde_json::json!({
                "packageId": "pkg-upstream-001",
                "namespace": "com.mudemocracy.governance",
                "name": "governance",
                "version": "1.0.0",
                "contentHash": "sha256:abc123",
                "installedAt": ""
            }),
        );
        store.save_manifest(&manifest).unwrap();

        let result = super::init_new_repository(
            &store,
            super::InitNewRepositoryInput {
                repository_id: None,
                namespace: "com.example.test".to_string(),
                title: "Test Repo".to_string(),
                description: None,
            },
        )
        .unwrap();

        assert_ne!(result.repository_id, "seed-repo-id", "should have a new ID");
        assert_eq!(result.namespace, "com.example.test");

        let manifest = store.load_manifest().unwrap();
        let installed_at = manifest.extra["upstreamPackage"]["installedAt"]
            .as_str()
            .unwrap();
        assert!(!installed_at.is_empty(), "installedAt should be set");
        assert!(installed_at.contains('T'), "installedAt should be ISO-8601");

        // Other upstreamPackage fields preserved
        assert_eq!(
            manifest.extra["upstreamPackage"]["packageId"]
                .as_str()
                .unwrap(),
            "pkg-upstream-001"
        );
        assert_eq!(
            manifest.extra["upstreamPackage"]["namespace"]
                .as_str()
                .unwrap(),
            "com.mudemocracy.governance"
        );
        assert_eq!(
            manifest.extra["upstreamPackage"]["contentHash"]
                .as_str()
                .unwrap(),
            "sha256:abc123"
        );
    }

    #[test]
    fn init_new_repository_rfc014_roundtrips_via_json_store() {
        let store = crate::json_store::JsonStore::from_srsj(&seed_rfc014_srsj()).unwrap();
        let input = super::InitNewRepositoryInput {
            repository_id: Some("new-fixed-uuid-rfc014".to_string()),
            namespace: "com.example.roundtrip-rfc014".to_string(),
            title: "RFC-014 Roundtrip Test".to_string(),
            description: None,
        };

        let result = super::init_new_repository(&store, input).unwrap();
        assert_eq!(result.repository_id, "new-fixed-uuid-rfc014");
        assert_eq!(result.package_id, "pkg-upstream-001");
        assert_eq!(result.package_version, "1.0.0");

        let serialized = store.to_srsj_string().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            parsed["manifest"]["repositoryId"].as_str().unwrap(),
            "new-fixed-uuid-rfc014"
        );
        assert_eq!(
            parsed["manifest"]["namespace"].as_str().unwrap(),
            "com.example.roundtrip-rfc014"
        );
        assert_eq!(
            parsed["manifest"]["title"].as_str().unwrap(),
            "RFC-014 Roundtrip Test"
        );

        // upstreamPackage at top level (RFC-014), not under meta
        assert!(
            parsed["manifest"].get("meta").is_none()
                || parsed["manifest"]["meta"].get("upstreamPackage").is_none(),
            "upstreamPackage should not be under meta for RFC-014 seeds"
        );
        let installed_at = parsed["manifest"]["upstreamPackage"]["installedAt"]
            .as_str()
            .unwrap();
        assert!(
            !installed_at.is_empty(),
            "installedAt should be set after init"
        );
        assert!(installed_at.contains('T'), "installedAt should be ISO-8601");

        // Other upstreamPackage fields preserved
        assert_eq!(
            parsed["manifest"]["upstreamPackage"]["packageId"]
                .as_str()
                .unwrap(),
            "pkg-upstream-001"
        );
        assert_eq!(
            parsed["manifest"]["upstreamPackage"]["contentHash"]
                .as_str()
                .unwrap(),
            "sha256:abc123"
        );
    }
}
