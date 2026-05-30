use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryMetadata {
    pub repository_id: String,
    pub namespace: String,
    pub srs_version: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStatus {
    pub exists: bool,
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

    store.initialize_repository(input)
}

pub fn get_repository_status(
    store: &dyn RepositoryStore,
) -> Result<RepositoryStatus, RepositoryError> {
    Ok(RepositoryStatus {
        exists: store.repository_exists()?,
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
}
