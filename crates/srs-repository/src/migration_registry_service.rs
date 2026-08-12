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
    // Last by rule, not by accident: every migration above writes the manifest,
    // and until srs-rust#828 lands each of those writes restores the
    // `instanceIndex` this one strips.
    MigrationDefinition {
        id: "rfc038-storage",
        title: "Adopt RFC-038 tree-authoritative storage",
        description: "Explodes every relations collection into one standalone object per \
                       relation at relations/<relationId>.json, and strips the retired \
                       manifest properties (instanceIndex, containerIndex, \
                       sourceDocumentIndex, relationsChecksums, relationsPath). A `.srsj` \
                       document is additionally bumped to srsj '2'. Instances are left \
                       where they are. Structural, not revision-keyed — RFC-038 forbids a \
                       data-model revision 3, so this migration stamps nothing. Idempotent; \
                       aborts rather than skips. NOT YET APPLICABLE from any client: the \
                       published manifest.json schema still requires instanceIndex, so a \
                       stripped manifest is one `repo validate` rejects, and there is no \
                       rollback (srs-rust#813). Unblocked by srs-rust#828.",
        status_fn: |store| {
            if !store.is_file_tree_store() {
                return Ok(MigrationStatus::NotApplicable);
            }
            if crate::rfc038_storage_migration_service::migration_needed(store)? {
                Ok(MigrationStatus::Needed)
            } else {
                Ok(MigrationStatus::AlreadyApplied)
            }
        },
        // Reports its status — that is the signal the entry exists to give —
        // but refuses to apply, because applying it today leaves a repository
        // the shipped schema rejects and nothing can roll that back. One line
        // flips when srs-rust#828 lands with the #783 Phase-6 flip.
        apply_fn: |_store| {
            Err(RepositoryError::InvalidInput {
                message: "rfc038-storage cannot be applied yet: the published manifest.json \
                          schema still lists `instanceIndex` in `required`, so stripping it \
                          leaves a repository `repo validate` rejects, and no store can roll \
                          an in-place run back (srs-rust#813). Unblocked by srs-rust#828, \
                          which lands with the RFC-038 enforcement flip."
                    .to_string(),
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
        // Embed-only root container ([R1]): a physical containers/*.json file with the
        // same id as manifest.container would be a fatal SRS038-R12-DUPLICATE-ID.
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(bare_container(container_id));
        write_manifest(&store, &manifest).unwrap();
        store
    }

    #[test]
    fn list_migrations_returns_every_entry_for_store_with_no_identity_note() {
        let store = make_store_with_container_no_identity();
        let migrations = list_migrations(&store).unwrap();
        assert_eq!(migrations.len(), 5);
        assert_eq!(migrations[0].id, "field-type");
        assert_eq!(migrations[1].id, "rfc039-carrier");
        assert_eq!(migrations[2].id, "migrate-identity");
        assert_eq!(migrations[3].id, "repo-upgrade");
        assert_eq!(migrations[4].id, "rfc038-storage");
        // Unstamped manifest → field-type Needed
        assert_eq!(migrations[0].status, MigrationStatus::Needed);
        // Revision < 2 → rfc039-carrier Needed
        assert_eq!(migrations[1].status, MigrationStatus::Needed);
        // Container exists but identity_instance_id is None → migrate-identity Needed
        assert_eq!(migrations[2].status, MigrationStatus::Needed);
        // Zero instances → all paths canonical → AlreadyApplied
        assert_eq!(migrations[3].status, MigrationStatus::AlreadyApplied);
        // MemoryStore is not a file tree — there is no storage layout to place.
        assert_eq!(migrations[4].status, MigrationStatus::NotApplicable);
    }

    fn indexed_srsj_store() -> crate::store::FileStore {
        crate::srsj::open_srsj(
            &serde_json::json!({
                "srsj": "2",
                "manifest": {
                    "srsVersion": "2.0-draft",
                    "repositoryId": "00000000-0000-4000-8000-00000000aaaa",
                    "instanceIndex": [],
                    "packageRef": { "mode": "local", "path": "package" },
                },
                // A real package: the other registered migrations load it, and
                // a fixture without one fails before it reaches an assertion.
                "data": {
                    "package/package.json": {
                        "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                        "id": "00000000-0000-4000-8000-00000000bbbb",
                        "namespace": "com.semanticops.test",
                        "name": "primary",
                        "version": "1.0.0",
                        "title": "Test Package",
                        "description": "Registry fixture package.",
                        "status": "draft",
                        "createdAt": "2026-01-01T00:00:00Z",
                        "fields": [],
                        "types": [],
                    },
                },
            })
            .to_string(),
        )
        .unwrap()
    }

    fn status_of(store: &dyn RepositoryStore, id: &str) -> MigrationStatus {
        list_migrations(store)
            .unwrap()
            .into_iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("{id} must be registered"))
            .status
    }

    /// `rfc038-storage` reports its status — that is what registering it is
    /// for — but must refuse to apply while srs-rust#828 is open, because a
    /// stripped manifest is one the shipped schema rejects and nothing can roll
    /// that back.
    #[test]
    fn rfc038_storage_reports_its_status_and_refuses_to_apply() {
        let store = indexed_srsj_store();
        assert_eq!(status_of(&store, "rfc038-storage"), MigrationStatus::Needed);

        let err = apply_migration(&store, "rfc038-storage").unwrap_err();
        assert!(err.to_string().contains("srs-rust#828"), "got: {err}");
        assert!(
            store.load_instance_json("manifest.json").unwrap()["instanceIndex"].is_array(),
            "a refused apply must change nothing"
        );
    }

    /// It is the last entry, so a client applying every `Needed` migration in
    /// order runs it after — never before — the migrations that write the
    /// manifest and would restore the index it strips.
    #[test]
    fn rfc038_storage_is_the_last_registered_migration() {
        let store = indexed_srsj_store();
        let migrations = list_migrations(&store).unwrap();
        assert_eq!(migrations.last().unwrap().id, "rfc038-storage");
    }

    /// The stripped `instanceIndex` must stay stripped — a later
    /// `save_manifest` from any other migration or service must not write the
    /// retired key straight back and undo this one.
    ///
    /// The tripwire for srs-rust#828: un-ignoring this test (and the
    /// `apply_fn`) is the completion signal for the manifest-schema change
    /// that makes `instanceIndex` optional. It drives the transform directly
    /// because the registry route is gated on the same issue.
    #[test]
    #[ignore = "srs-rust#828: `Manifest.instance_index` cannot be skip_serializing_if until the published manifest.json schema drops `instanceIndex` from `required` — omitting it today fails validation for every repository, `repo create` included. Closed by the schema change plus the #783 Phase-6 field removal."]
    fn a_later_migration_does_not_restore_the_stripped_index() {
        let store = indexed_srsj_store();
        crate::rfc038_storage_migration_service::migrate_storage(
            &store,
            &crate::rfc038_storage_migration_service::StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap();
        apply_migration(&store, "field-type").expect("field-type must succeed");

        let manifest = store.load_instance_json("manifest.json").unwrap();
        assert!(
            manifest.get("instanceIndex").is_none(),
            "instanceIndex was written back by a later manifest save: {manifest}"
        );
        assert_eq!(
            status_of(&store, "rfc038-storage"),
            MigrationStatus::AlreadyApplied
        );
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
        // RFC-038: discoverability comes from the catalog, not manifest.instance_index.
        let cat = target.catalog().unwrap();
        let purpose_entries: Vec<_> = cat.instances.iter().filter(|e| e.tier == Some(2)).collect();
        assert_eq!(
            purpose_entries.len(),
            1,
            "exactly one Tier-2 purpose record must survive roundtrip"
        );
        let raw = target
            .load_instance_json(purpose_entries[0].locator.as_deref().unwrap())
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
