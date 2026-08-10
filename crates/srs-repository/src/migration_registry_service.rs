use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MigrationStatus {
    Needed,
    AlreadyApplied,
    NotApplicable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    pub status: MigrationStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationApplyResult {
    pub id: String,
    pub payload: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Projection struct for repo-upgrade apply_fn output (ADR-010: no json!())
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpgradePathsMigrationPayload {
    renames: Vec<crate::repository_portability::InstancePathRename>,
    total_instances: usize,
    already_canonical_count: usize,
}

// ---------------------------------------------------------------------------
// Registry definition (private)
// ---------------------------------------------------------------------------

struct MigrationDefinition {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    status_fn: fn(&dyn RepositoryStore) -> Result<MigrationStatus, RepositoryError>,
    apply_fn: fn(&dyn RepositoryStore) -> Result<serde_json::Value, RepositoryError>,
}

static MIGRATIONS: &[MigrationDefinition] = &[
    MigrationDefinition {
        id: "field-type",
        title: "Adopt the RFC-032 fieldType model",
        description: "Rewrites every Field definition from the pre-RFC-032 `valueType` \
                       (+ contentFormat/allowedValues/vocabularyRef/validationRules) to the \
                       decomposed `fieldType`, and stamps `dataModelRevision: 1` on the \
                       manifest. This is RFC-033's migration #1 (revision 0 → 1). \
                       Idempotent — safe to run multiple times.",
        status_fn: |store| {
            if crate::field_type_migration_service::migration_needed(store)? {
                Ok(MigrationStatus::Needed)
            } else {
                Ok(MigrationStatus::AlreadyApplied)
            }
        },
        apply_fn: |store| {
            let result = crate::field_type_migration_service::migrate_field_types(store)?;
            serde_json::to_value(&result).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: format!("failed to serialize field-type migration result: {e}"),
            })
        },
    },
    MigrationDefinition {
        id: "rfc039-carrier",
        title: "Adopt the RFC-039 name-keyed fieldValues carrier",
        description: "Rewrites every Tier-2 fieldValues array into the name-keyed \
                       object carrier, converts FieldGroups into inline-composite \
                       Fields over minted range Types (Change E.2), strips the \
                       deprecated FieldAssignment repeatable/minItems/maxItems trio, \
                       replaces Tier-1 valueType with an inline fieldType, and stamps \
                       dataModelRevision: 2. This is data-model migration #2 \
                       (revision 1 -> 2). Aborts rather than skips; an abort rolls \
                       the store back (ADR-021).",
        status_fn: |store| {
            if crate::rfc039_carrier_migration_service::migration_needed(store)? {
                Ok(MigrationStatus::Needed)
            } else {
                Ok(MigrationStatus::AlreadyApplied)
            }
        },
        apply_fn: |store| {
            let result = crate::rfc039_carrier_migration_service::migrate_carrier(store)?;
            serde_json::to_value(&result).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: format!("failed to serialize carrier migration result: {e}"),
            })
        },
    },
    MigrationDefinition {
        id: "migrate-identity",
        title: "Graduate identity to purpose record",
        description: "Converts a Tier-0 note identity (or a container with no identity \
                       pointer) to a typed com.semanticops.core/purpose Tier-2 Record \
                       and repoints manifest.container.identityInstanceId. Satisfies RFC-018.",
        status_fn: |store| {
            use crate::migrate_identity_service::IdentityMigrationStatus;
            match crate::migrate_identity_service::migration_status(store)? {
                IdentityMigrationStatus::Needed => Ok(MigrationStatus::Needed),
                IdentityMigrationStatus::AlreadyApplied => Ok(MigrationStatus::AlreadyApplied),
                IdentityMigrationStatus::NotApplicable => Ok(MigrationStatus::NotApplicable),
            }
        },
        apply_fn: |store| {
            let result = crate::migrate_identity_service::migrate_identity(store)?;
            serde_json::to_value(&result).map_err(|e| RepositoryError::InvalidSnapshotData {
                message: format!("failed to serialize migrate-identity result: {e}"),
            })
        },
    },
    MigrationDefinition {
        id: "repo-upgrade",
        title: "Normalise instance file paths",
        description: "Renames instance files to the canonical slug-id8 convention \
                       (e.g. title-a1b2c3d4.json). Idempotent — safe to run multiple times.",
        status_fn: |store| {
            let needed = crate::repository_portability::check_path_upgrade_needed(store)?;
            if needed {
                Ok(MigrationStatus::Needed)
            } else {
                Ok(MigrationStatus::AlreadyApplied)
            }
        },
        apply_fn: |store| {
            let result = crate::upgrade_repository_paths(store)?;
            serde_json::to_value(UpgradePathsMigrationPayload {
                already_canonical_count: result.already_canonical_count,
                total_instances: result.total_instances,
                renames: result.renames,
            })
            .map_err(|e| RepositoryError::InvalidSnapshotData {
                message: format!("failed to serialize repo-upgrade result: {e}"),
            })
        },
    },
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn list_migrations(
    store: &dyn RepositoryStore,
) -> Result<Vec<MigrationSummary>, RepositoryError> {
    MIGRATIONS
        .iter()
        .map(|m| {
            let status = (m.status_fn)(store)?;
            Ok(MigrationSummary {
                id: m.id.to_string(),
                title: m.title.to_string(),
                description: m.description.to_string(),
                status,
            })
        })
        .collect()
}

pub fn apply_migration(
    store: &dyn RepositoryStore,
    id: &str,
) -> Result<MigrationApplyResult, RepositoryError> {
    let m =
        MIGRATIONS
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| RepositoryError::InvalidInput {
                message: format!("unknown migration id: '{id}'"),
            })?;
    let payload = (m.apply_fn)(store)?;
    Ok(MigrationApplyResult {
        id: id.to_string(),
        payload,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service::create_container;
    use crate::repository_portability::copy_repository;
    use crate::store::memory::MemoryStore;
    use crate::writer::{write_manifest, write_note};
    use srs_core::types::container::Container;
    use srs_core::types::note::{Note, NoteSection};

    fn bare_container(container_id: &str) -> Container {
        Container {
            container_id: container_id.to_string(),
            title: "Test Repo".to_string(),
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
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn make_store_with_identity(
        note_id: &str,
        note_title: Option<&str>,
        sections: Vec<NoteSection>,
    ) -> (MemoryStore, String) {
        let store = MemoryStore::default();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";

        create_container(&store, bare_container(container_id)).unwrap();

        let note = Note {
            instance_id: note_id.to_string(),
            title: note_title.map(|t| t.to_string()),
            sections,
            tags: None,
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };
        let note_path = "records/notes/identity.json";
        write_note(&store, &note, note_path).unwrap();

        let mut manifest = store.load_manifest().unwrap();
        let mut root = bare_container(container_id);
        root.identity_instance_id = Some(note_id.to_string());
        manifest.container = Some(root);
        write_manifest(&store, &manifest).unwrap();

        (store, container_id.to_string())
    }

    fn one_section(content: &str) -> Vec<NoteSection> {
        vec![NoteSection {
            name: "body".to_string(),
            label: None,
            content: content.to_string(),
            content_hint: None,
            tags: None,
        }]
    }

    /// Store with a root container but no identity note; zero instances means repo-upgrade
    /// has nothing to rename (target state achieved = AlreadyApplied).
    fn make_store_with_container_no_identity() -> MemoryStore {
        let store = MemoryStore::default();
        let container_id = "770e8400-e29b-41d4-a716-446655440000";
        create_container(&store, bare_container(container_id)).unwrap();
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(bare_container(container_id));
        write_manifest(&store, &manifest).unwrap();
        store
    }

    #[test]
    fn list_migrations_returns_every_entry_for_store_with_no_identity_note() {
        let store = make_store_with_container_no_identity();
        let migrations = list_migrations(&store).unwrap();
        assert_eq!(migrations.len(), 4);
        assert_eq!(migrations[0].id, "field-type");
        assert_eq!(migrations[1].id, "rfc039-carrier");
        assert_eq!(migrations[2].id, "migrate-identity");
        assert_eq!(migrations[3].id, "repo-upgrade");
        // Unstamped manifest → field-type Needed
        assert_eq!(migrations[0].status, MigrationStatus::Needed);
        // Revision < 2 → rfc039-carrier Needed
        assert_eq!(migrations[1].status, MigrationStatus::Needed);
        // Container exists but identity_instance_id is None → migrate-identity Needed
        assert_eq!(migrations[2].status, MigrationStatus::Needed);
        // Zero instances → all paths canonical → AlreadyApplied
        assert_eq!(migrations[3].status, MigrationStatus::AlreadyApplied);
    }

    #[test]
    fn field_type_migration_stamps_the_manifest_and_is_idempotent() {
        use crate::field_type_migration_service::{data_model_revision, FIELD_TYPE_REVISION};
        let store = make_store_with_container_no_identity();
        assert_eq!(data_model_revision(&store).unwrap(), 0);

        // field-type is migration #1: it stamps ITS revision (1), not the
        // build's current revision (2 — that is rfc039-carrier's stamp).
        apply_migration(&store, "field-type").expect("first apply must succeed");
        assert_eq!(data_model_revision(&store).unwrap(), FIELD_TYPE_REVISION);
        assert_eq!(
            list_migrations(&store).unwrap()[0].status,
            MigrationStatus::AlreadyApplied
        );

        // Re-running changes nothing.
        apply_migration(&store, "field-type").expect("second apply must succeed");
        assert_eq!(data_model_revision(&store).unwrap(), FIELD_TYPE_REVISION);
    }

    #[test]
    fn list_migrations_migrate_identity_already_applied() {
        use crate::repository_lifecycle::{
            create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
            RepositoryMetadata,
        };
        let store = MemoryStore::uninitialized();
        let input = InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "repo-1".to_string(),
                namespace: "com.semanticops.test".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("My Repo".to_string()),
                description: Some("I build SRS.".to_string()),
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg-1".to_string(),
                namespace: "com.semanticops.test".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        };
        create_repository_with_intent(&store, &input).unwrap();
        let migrations = list_migrations(&store).unwrap();
        let identity_migration = migrations
            .iter()
            .find(|m| m.id == "migrate-identity")
            .unwrap();
        assert_eq!(identity_migration.status, MigrationStatus::AlreadyApplied);
    }

    #[test]
    fn apply_migration_migrate_identity_succeeds() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111111",
            Some("My Repo"),
            one_section("I build SRS."),
        );
        let result = apply_migration(&store, "migrate-identity").unwrap();
        assert_eq!(result.id, "migrate-identity");
        assert!(result.payload.is_object(), "payload must be a JSON object");
        assert!(
            result.payload.get("newIdentityId").is_some(),
            "payload must contain newIdentityId"
        );
    }

    #[test]
    fn apply_migration_unknown_id_returns_error() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111112",
            Some("Repo"),
            one_section("Content."),
        );
        let err = apply_migration(&store, "no-such-migration").unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("no-such-migration")),
            "expected InvalidInput with unknown id, got: {err:?}"
        );
    }

    #[test]
    fn apply_migration_repo_upgrade_succeeds() {
        // One instance at a non-canonical path so upgrade has work to do.
        let store = MemoryStore::default();
        let container_id = "880e8400-e29b-41d4-a716-446655440000";
        create_container(&store, bare_container(container_id)).unwrap();

        let instance_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let note = Note {
            instance_id: instance_id.to_string(),
            title: Some("My Note".to_string()),
            sections: one_section("content"),
            tags: None,
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };
        // Write to a non-canonical path
        let old_path = "records/notes/old-name.json";
        write_note(&store, &note, old_path).unwrap();

        let result = apply_migration(&store, "repo-upgrade").unwrap();
        assert_eq!(result.id, "repo-upgrade");
        let renames = result.payload["renames"].as_array().unwrap();
        assert_eq!(renames.len(), 1, "expected 1 rename");
    }

    #[test]
    fn list_migrations_no_container_identity_not_applicable() {
        // MemoryStore::default() starts with no manifest.container
        let store = MemoryStore::default();
        let migrations = list_migrations(&store).unwrap();
        let identity_migration = migrations
            .iter()
            .find(|m| m.id == "migrate-identity")
            .unwrap();
        assert_eq!(identity_migration.status, MigrationStatus::NotApplicable);
    }

    #[test]
    fn cross_store_roundtrip_apply_migrate_identity() {
        let (source, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111113",
            Some("Repo"),
            one_section("Content."),
        );
        apply_migration(&source, "migrate-identity").unwrap();

        let target = MemoryStore::uninitialized();
        copy_repository(&source, &target).unwrap();

        // copy_repository transfers instances, containers, AND manifest.container
        // (via snapshot.root_container). After migration the source's manifest.container
        // identityInstanceId points at the new purpose record; that pointer must survive
        // the roundtrip so migration_status returns AlreadyApplied on the target.
        let manifest = target.load_manifest().unwrap();
        let purpose_entries: Vec<_> = manifest
            .instance_index
            .iter()
            .filter(|e| e.tier == 2)
            .collect();
        assert_eq!(
            purpose_entries.len(),
            1,
            "exactly one Tier-2 purpose record must survive roundtrip"
        );
        let raw = target
            .load_instance_json(purpose_entries[0].path())
            .unwrap();
        assert_eq!(
            raw["typeNamespace"].as_str(),
            Some(crate::core_purpose::PURPOSE_TYPE_NAMESPACE),
        );

        // manifest.container was transferred by copy_repository — no patching needed.
        let migrations = list_migrations(&target).unwrap();
        let id_migration = migrations
            .iter()
            .find(|m| m.id == "migrate-identity")
            .unwrap();
        assert_eq!(id_migration.status, MigrationStatus::AlreadyApplied);
    }

    #[test]
    fn apply_migrate_identity_then_list_shows_already_applied() {
        // After applying migrate-identity, list_migrations must report AlreadyApplied
        // without any store manipulation — the registry status_fn must detect the outcome.
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111114",
            Some("Repo"),
            one_section("Content."),
        );

        let before = list_migrations(&store).unwrap();
        let id_before = before.iter().find(|m| m.id == "migrate-identity").unwrap();
        assert_eq!(
            id_before.status,
            MigrationStatus::Needed,
            "must be Needed before apply"
        );

        apply_migration(&store, "migrate-identity").unwrap();

        let after = list_migrations(&store).unwrap();
        let id_after = after.iter().find(|m| m.id == "migrate-identity").unwrap();
        assert_eq!(
            id_after.status,
            MigrationStatus::AlreadyApplied,
            "must be AlreadyApplied after apply"
        );
    }
}
