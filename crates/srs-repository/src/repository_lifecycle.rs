use crate::core_purpose;
use crate::error::RepositoryError;
use crate::paths::DEFAULT_RECORD_DIR;
use crate::record_store::{upsert_record_index_entry, write_new_record};
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use srs_core::types::container::Container;
use std::collections::HashMap;
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
    pub identity_instance_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatus {
    pub exists: bool,
}

// Purpose-type constants are in crate::core_purpose (shared with migrate_identity_service).

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

fn scaffold_purpose_record(
    store: &dyn RepositoryStore,
    repository_id: &str,
    container_title: &str,
    record_title: Option<&str>,
    description: Option<&str>,
) -> Result<String, RepositoryError> {
    let instance_id = new_instance_id();
    let now = Utc::now().to_rfc3339();
    let record = core_purpose::build_purpose_record(
        &instance_id,
        description.unwrap_or(""),
        record_title,
        &now,
    );

    let relative_path = write_new_record(store, &record, DEFAULT_RECORD_DIR)?;

    let mut manifest = store.load_manifest()?;
    upsert_record_index_entry(&mut manifest, &record, &relative_path);

    let container = manifest.container.get_or_insert_with(|| Container {
        container_id: repository_id.to_string(),
        title: container_title.to_string(),
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
        extra: HashMap::new(),
    });
    container.identity_instance_id = Some(instance_id.clone());
    container
        .member_instance_ids
        .get_or_insert_with(Vec::new)
        .push(instance_id.clone());

    store.save_manifest(&manifest)?;

    Ok(instance_id)
}

pub fn create_repository_with_intent(
    store: &dyn RepositoryStore,
    input: &InitializeRepositoryInput,
) -> Result<CreateRepositoryResult, RepositoryError> {
    let mut result = create_repository(store, input)?;

    // Effective title matches the normalization applied in create_repository.
    let effective_title = input
        .repository
        .title
        .as_deref()
        .unwrap_or(input.repository.namespace.as_str());
    let identity_instance_id = scaffold_purpose_record(
        store,
        &result.repository_id,
        effective_title,
        input.repository.title.as_deref(),
        input.repository.description.as_deref(),
    )?;
    result.identity_instance_id = Some(identity_instance_id);

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
    use crate::core_purpose;
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
        assert!(package.fields.iter().any(|f| f.namespace == "com.semanticops.core"));
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

    /// RFC-014 format: `upstreamPackage` at top level (not under `meta`).
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
    }

    // ── create_repository_with_intent / scaffold_purpose_record tests ────────

    #[test]
    fn create_repository_with_intent_returns_identity_instance_id() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        assert!(
            result.identity_instance_id.is_some(),
            "identity_instance_id must always be set"
        );
    }

    #[test]
    fn create_repository_with_intent_creates_purpose_record_in_index() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let in_index = manifest
            .instance_index
            .iter()
            .any(|e| e.instance_id() == id);
        assert!(in_index, "purpose record must appear in instance_index");
    }

    #[test]
    fn create_repository_with_intent_sets_container_identity_instance_id() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let container_id = manifest
            .container
            .as_ref()
            .and_then(|c| c.identity_instance_id.as_deref());
        assert_eq!(
            container_id,
            Some(id.as_str()),
            "container.identityInstanceId must match the purpose record"
        );
    }

    #[test]
    fn create_repository_with_intent_adds_to_member_instance_ids() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let members = manifest
            .container
            .as_ref()
            .and_then(|c| c.member_instance_ids.as_ref())
            .expect("member_instance_ids must be set");
        assert!(
            members.contains(&id),
            "purpose record must be in member_instance_ids"
        );
    }

    #[test]
    fn create_repository_with_intent_record_has_correct_type() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .unwrap();
        let record: srs_core::types::record::Record =
            serde_json::from_value(store.load_instance_json(entry.path()).unwrap()).unwrap();

        assert_eq!(record.type_id, core_purpose::purpose_type_id());
        assert_eq!(record.type_namespace, core_purpose::PURPOSE_TYPE_NAMESPACE);
        assert_eq!(record.type_name, core_purpose::PURPOSE_TYPE_NAME);
    }

    #[test]
    fn create_repository_with_intent_record_has_statement_field() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .unwrap();
        let record: srs_core::types::record::Record =
            serde_json::from_value(store.load_instance_json(entry.path()).unwrap()).unwrap();

        let has_statement = record
            .field_values
            .iter()
            .any(|fv| fv.field_id == core_purpose::statement_field_id());
        assert!(has_statement, "purpose record must have statement field");
    }

    #[test]
    fn create_repository_with_intent_with_title_has_title_field() {
        let store = MemoryStore::uninitialized();
        let mut titled = input();
        titled.repository.title = Some("My Project".to_string());
        let result = create_repository_with_intent(&store, &titled).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .unwrap();
        let record: srs_core::types::record::Record =
            serde_json::from_value(store.load_instance_json(entry.path()).unwrap()).unwrap();

        let title_fv = record
            .field_values
            .iter()
            .find(|fv| fv.field_id == core_purpose::title_field_id())
            .expect("title field must be present when title given");
        assert_eq!(title_fv.value.as_str(), Some("My Project"));
    }

    #[test]
    fn create_repository_with_intent_without_title_no_title_field() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .unwrap();
        let record: srs_core::types::record::Record =
            serde_json::from_value(store.load_instance_json(entry.path()).unwrap()).unwrap();

        let has_title = record
            .field_values
            .iter()
            .any(|fv| fv.field_id == core_purpose::title_field_id());
        assert!(!has_title, "title field must be absent when no title given");
    }

    #[test]
    fn create_repository_with_intent_record_path_uses_type_name_and_id8() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .unwrap();
        let path = entry.path();
        assert!(
            path.contains("purpose-"),
            "record path must use type_name-id8 convention, got: {path}"
        );
        assert!(
            path.contains(&id[..8]),
            "record path must include first 8 chars of instance_id, got: {path}"
        );
    }

    #[test]
    fn create_repository_with_intent_container_title_uses_namespace_fallback() {
        // When no title is provided, the container title should be the namespace,
        // not an empty string — consistent with what FileStore computes.
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        assert!(result.identity_instance_id.is_some());

        let manifest = store.load_manifest().unwrap();
        let container = manifest.container.as_ref().expect("container must be set");
        assert_eq!(
            container.title, "com.semanticops.test",
            "container title must fall back to namespace when no title given"
        );
    }

    #[test]
    fn create_repository_with_intent_record_has_created_at() {
        let store = MemoryStore::uninitialized();
        let result = create_repository_with_intent(&store, &input()).unwrap();
        let id = result.identity_instance_id.unwrap();

        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .unwrap();
        let record: srs_core::types::record::Record =
            serde_json::from_value(store.load_instance_json(entry.path()).unwrap()).unwrap();

        assert!(
            record.created_at.is_some(),
            "purpose record must have created_at timestamp"
        );
        assert!(
            record.updated_at.is_some(),
            "purpose record must have updated_at timestamp"
        );
    }

    #[test]
    fn create_repository_with_intent_roundtrips_via_file_store() {
        let tmp = TempDir::new().unwrap();
        let store = FileStore::new(tmp.path());

        let mut titled = input();
        titled.repository.title = Some("FileStore Project".to_string());
        titled.repository.description = Some("Testing roundtrip.".to_string());

        let result = create_repository_with_intent(&store, &titled).unwrap();
        let id = result.identity_instance_id.unwrap();

        // Reload from disk to verify persistence
        let store2 = FileStore::new(tmp.path());
        let manifest = store2.load_manifest().unwrap();

        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == id)
            .expect("purpose record must be in instance_index after file roundtrip");

        let record: srs_core::types::record::Record =
            serde_json::from_value(store2.load_instance_json(entry.path()).unwrap()).unwrap();
        assert_eq!(record.instance_id, id);
        assert_eq!(record.type_id, core_purpose::purpose_type_id());

        let container_id = manifest
            .container
            .as_ref()
            .and_then(|c| c.identity_instance_id.as_deref());
        assert_eq!(
            container_id,
            Some(id.as_str()),
            "container.identityInstanceId must survive file roundtrip"
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
    }
}
