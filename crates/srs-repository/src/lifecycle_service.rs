use crate::error::RepositoryError;
use crate::package_types::DefinitionKind;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::lifecycle::Lifecycle;

pub fn list_lifecycles(store: &dyn RepositoryStore) -> Result<Vec<Lifecycle>, RepositoryError> {
    match store.load_package() {
        Ok(package) => Ok(package.lifecycles),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(vec![]),
        Err(e) => Err(e),
    }
}

pub fn get_lifecycle_by_id(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<Option<Lifecycle>, RepositoryError> {
    match store.load_package() {
        Ok(package) => Ok(package.lifecycles.into_iter().find(|lc| lc.id == id)),
        Err(RepositoryError::Io { .. }) | Err(RepositoryError::PackageLoad { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Result of `create_lifecycle`.
pub struct CreateLifecycleResult {
    pub lifecycle: Lifecycle,
}

/// Create a new Lifecycle in the primary package.
///
/// Writes `package/lifecycles/{slug}-{id}.json` and adds the path to
/// `package/package.json` → `lifecycles[]`. If `lifecycle.id` is empty,
/// a new UUID is generated. If `lifecycle.created_at` is empty, the
/// current timestamp is used.
pub fn create_lifecycle(
    store: &dyn RepositoryStore,
    mut lifecycle: Lifecycle,
) -> Result<CreateLifecycleResult, RepositoryError> {
    store.load_package_boundary(&None)?;

    if lifecycle.id.trim().is_empty() {
        lifecycle.id = new_instance_id();
    }
    if lifecycle.created_at.trim().is_empty() {
        lifecycle.created_at = chrono::Utc::now().to_rfc3339();
    }

    let slug = lifecycle
        .name
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-");
    let rel_filename = format!("lifecycles/{}-{}.json", slug, &lifecycle.id[..8]);
    let full_path = format!("package/{rel_filename}");

    store.ensure_lifecycles_dir("package/lifecycles")?;
    store.save_lifecycle(&full_path, &lifecycle)?;
    store.add_definition_to_boundary(&None, DefinitionKind::Lifecycle, &rel_filename)?;

    Ok(CreateLifecycleResult { lifecycle })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use srs_core::types::lifecycle::{LifecycleState, LifecycleTransition};

    fn make_lifecycle() -> Lifecycle {
        Lifecycle {
            id: "lc-test-id".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "test-lifecycle".to_string(),
            states: vec![
                LifecycleState {
                    id: Some("s1".to_string()),
                    version: None,
                    namespace: None,
                    key: "draft".to_string(),
                    label: None,
                    description: None,
                    aliases: None,
                    is_initial: Some(true),
                    is_final: None,
                    status: None,
                    properties: None,
                },
                LifecycleState {
                    id: Some("s2".to_string()),
                    version: None,
                    namespace: None,
                    key: "active".to_string(),
                    label: None,
                    description: None,
                    aliases: None,
                    is_initial: None,
                    is_final: Some(true),
                    status: None,
                    properties: None,
                },
            ],
            transitions: vec![LifecycleTransition {
                id: Some("t1".to_string()),
                name: "publish".to_string(),
                from: "draft".to_string(),
                to: "active".to_string(),
                description: None,
                properties: None,
            }],
            initial_state: "draft".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn list_lifecycles_empty_when_no_package() {
        let store = MemoryStore::default();
        let result = list_lifecycles(&store).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn get_lifecycle_by_id_returns_none_when_missing() {
        let store = MemoryStore::default();
        let result = get_lifecycle_by_id(&store, "00000000-0000-0000-0000-000000000000").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn create_lifecycle_assigns_id_and_is_retrievable() {
        let store = MemoryStore::default();
        let lc = make_lifecycle();
        let result = create_lifecycle(&store, lc).unwrap();
        assert_eq!(result.lifecycle.name, "test-lifecycle");
        assert!(!result.lifecycle.id.is_empty());

        let lifecycles = list_lifecycles(&store).unwrap();
        assert_eq!(lifecycles.len(), 1);
        assert_eq!(lifecycles[0].name, "test-lifecycle");

        let found = get_lifecycle_by_id(&store, &result.lifecycle.id).unwrap();
        assert!(found.is_some());
    }

    #[test]
    fn lifecycle_roundtrips_json() {
        let lc = make_lifecycle();
        let json = serde_json::to_string(&lc).unwrap();
        let parsed: Lifecycle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.states.len(), 2);
        assert_eq!(parsed.states[0].key, "draft");
        assert_eq!(parsed.transitions[0].name, "publish");
        assert_eq!(parsed.initial_state, "draft");
    }
}
