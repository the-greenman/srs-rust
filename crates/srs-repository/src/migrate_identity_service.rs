use crate::container_service;
use crate::core_purpose;
use crate::error::RepositoryError;
use crate::loader;
use crate::record_store::create_record;
use crate::store::RepositoryStore;
use crate::writer;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateIdentityResult {
    pub old_identity_id: Option<String>,
    pub old_identity_tier: Option<u8>,
    pub new_identity_id: String,
    pub statement: String,
    pub title: Option<String>,
}

fn extract_identity_text(
    store: &dyn RepositoryStore,
    instance_id: &str,
    tier: u8,
    path: &str,
) -> Result<(String, Option<String>), RepositoryError> {
    match tier {
        0 => {
            let note = loader::load_note(store, path)?;
            let joined = note
                .sections
                .iter()
                .map(|s| s.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let statement = joined.trim().to_string();
            let title = note.title.clone();
            let statement = if statement.is_empty() {
                title.clone().unwrap_or_default()
            } else {
                statement
            };
            if statement.is_empty() {
                return Err(RepositoryError::InvalidInput {
                    message: "identity note has no extractable statement".to_string(),
                });
            }
            Ok((statement, title))
        }
        2 => Err(RepositoryError::InvalidInput {
            message: format!(
                "identity instance '{instance_id}' is a Tier-2 record that is not a \
                 com.semanticops.core/purpose record; manual migration is required \
                 or the identity must first be changed to a Tier-0 note"
            ),
        }),
        t => Err(RepositoryError::InvalidInput {
            message: format!(
                "unsupported identity tier {t}: only Tier-0 and Tier-2 identities can be migrated"
            ),
        }),
    }
}

/// Local status enum for the identity migration — returned by [`migration_status`] so that
/// `migrate_identity_service` does not need to import from `migration_registry_service`.
/// The registry's `status_fn` maps from this type to the registry's own `MigrationStatus`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityMigrationStatus {
    Needed,
    AlreadyApplied,
    NotApplicable,
}

pub fn migration_status(
    store: &dyn RepositoryStore,
) -> Result<IdentityMigrationStatus, RepositoryError> {
    let manifest = store.load_manifest()?;
    let mc = match manifest.container.as_ref() {
        None => return Ok(IdentityMigrationStatus::NotApplicable),
        Some(c) => c,
    };

    match mc.identity_instance_id.as_deref() {
        None => Ok(IdentityMigrationStatus::Needed),
        Some(id) => {
            let cat = store.catalog()?;
            let entry = cat.instances.iter().find(|e| e.id == id);
            match entry {
                None => Ok(IdentityMigrationStatus::Needed),
                Some(e) if e.tier != Some(2) => Ok(IdentityMigrationStatus::Needed),
                Some(e) => {
                    let raw = store.load_instance_json(e.locator.as_deref().unwrap_or_default())?;
                    let ns_ok = raw.get("typeNamespace").and_then(|v| v.as_str())
                        == Some(core_purpose::PURPOSE_TYPE_NAMESPACE);
                    let name_ok = raw.get("typeName").and_then(|v| v.as_str())
                        == Some(core_purpose::PURPOSE_TYPE_NAME);
                    if ns_ok && name_ok {
                        Ok(IdentityMigrationStatus::AlreadyApplied)
                    } else {
                        Ok(IdentityMigrationStatus::NotApplicable)
                    }
                }
            }
        }
    }
}

pub fn migrate_identity(
    store: &dyn RepositoryStore,
) -> Result<MigrateIdentityResult, RepositoryError> {
    let manifest = store.load_manifest()?;

    let mc = manifest
        .container
        .as_ref()
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: "manifest.container is not set".to_string(),
        })?
        .clone();

    // None-branch: identity_instance_id is absent — derive purpose record from container metadata.
    if mc.identity_instance_id.is_none() {
        let statement = mc
            .description
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .unwrap_or(mc.title.as_str());
        if statement.is_empty() {
            return Err(RepositoryError::InvalidInput {
                message: "container has no title or description to derive a purpose statement"
                    .to_string(),
            });
        }
        let record_title = if mc.title.is_empty() {
            None
        } else {
            Some(mc.title.as_str())
        };

        let spec = core_purpose::purpose_record_spec(statement, record_title);

        store.begin_batch();
        let batch_result = (|| -> Result<String, RepositoryError> {
            // Route through create_record so CFR validation runs at write time (ADR-002, #481).
            // create_record writes the record file and updates the manifest index entry internally.
            let record = create_record(
                store,
                &spec.type_id,
                spec.type_version,
                spec.field_values,
                None,
                None,
            )?;
            let new_id = record.instance_id.clone();

            // Reload manifest to capture the index entry create_record just wrote, then add
            // container metadata and write a second time (two manifest writes per ADR-021 batch).
            let mut manifest = store.load_manifest()?;
            if let Some(ref mut container) = manifest.container {
                container.identity_instance_id = Some(new_id.clone());
                container
                    .member_instance_ids
                    .get_or_insert_with(Vec::new)
                    .push(new_id.clone());
            }
            writer::write_manifest(store, &manifest)?;
            // Load the real container file (which holds pre-existing section members),
            // patch in the new identity pointer and member, then save — mirrors the
            // non-None branch. Falls back to mc on ContainerNotFound. mc is the
            // pre-batch snapshot from line 121 (before any batch writes), so new_id
            // is NOT yet in mc.member_instance_ids; the push below adds it exactly
            // once. (The within-batch manifest.container already has new_id pushed at
            // lines 163-167 but is a distinct struct from the container file.)
            let mut persisted_container = match store.load_container(&mc.container_id) {
                Ok(c) => c,
                Err(RepositoryError::ContainerNotFound { .. }) => mc.clone(),
                Err(e) => return Err(e),
            };
            persisted_container.identity_instance_id = Some(new_id.clone());
            persisted_container
                .member_instance_ids
                .get_or_insert_with(Vec::new)
                .push(new_id.clone());
            store.save_container(&persisted_container)?;
            Ok(new_id)
        })();
        match batch_result {
            Ok(new_id) => {
                if let Err(e) = store.commit_batch() {
                    store.abort_batch();
                    return Err(e);
                }
                return Ok(MigrateIdentityResult {
                    old_identity_id: None,
                    old_identity_tier: None,
                    new_identity_id: new_id,
                    statement: statement.to_string(),
                    title: record_title.map(str::to_string),
                });
            }
            Err(e) => {
                store.abort_batch();
                return Err(e);
            }
        }
    }

    let old_id = mc.identity_instance_id.clone().unwrap();
    let root_container_id = mc.container_id.clone();

    let cat = store.catalog()?;
    let entry = cat
        .instances
        .iter()
        .find(|e| e.id == old_id)
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: format!("identity instance '{old_id}' not found in the instance set"),
        })?;
    let old_tier = entry.tier.unwrap_or(2);
    let entry_path = entry.locator.clone().unwrap_or_default();

    if old_tier == 2 {
        let raw = store.load_instance_json(&entry_path)?;
        let ns_ok = raw.get("typeNamespace").and_then(|v| v.as_str())
            == Some(core_purpose::PURPOSE_TYPE_NAMESPACE);
        let name_ok =
            raw.get("typeName").and_then(|v| v.as_str()) == Some(core_purpose::PURPOSE_TYPE_NAME);
        if ns_ok && name_ok {
            return Err(RepositoryError::InvalidInput {
                message: "already a com.semanticops.core/purpose record; no migration needed"
                    .to_string(),
            });
        }
    }

    let (statement, title) = extract_identity_text(store, &old_id, old_tier, &entry_path)?;

    let spec = core_purpose::purpose_record_spec(&statement, title.as_deref());

    // ADR-021 batch: record file + manifest + container membership atomically.
    // Also remove the old identity from container members: it was there only to satisfy
    // RFC-013 I-81 (identity must be a member). After migration the new record takes
    // that slot; leaving the old note would trigger RFC-013 I-82 (non-identity member
    // not rooting a section container).
    store.begin_batch();
    let batch_result = (|| -> Result<String, RepositoryError> {
        // Route through create_record so CFR validation runs at write time (ADR-002, #481).
        // create_record writes the record file and updates the manifest index entry internally.
        let record = create_record(
            store,
            &spec.type_id,
            spec.type_version,
            spec.field_values,
            None,
            None,
        )?;
        let new_id = record.instance_id.clone();

        // Reload manifest to capture the index entry create_record just wrote, then update
        // identity pointer and write a second time (two manifest writes per ADR-021 batch).
        let mut manifest = store.load_manifest()?;
        if let Some(ref mut mc) = manifest.container {
            mc.identity_instance_id = Some(new_id.clone());
        }
        writer::write_manifest(store, &manifest)?;
        container_service::add_container_member(store, &root_container_id, &new_id)?;
        container_service::remove_container_member(store, &root_container_id, &old_id)?;
        // Update the persisted Container record's identityInstanceId in lockstep with the
        // manifest embed. Without this the container file disagrees with manifest.container
        // (issue #462).
        //
        // IMPORTANT: Do NOT replace this with container_service::update_container.
        // For root containers, update_container calls begin_batch/commit_batch internally
        // (container_service.rs ~line 234). Nesting that inside this outer batch causes
        // FileStore to flush prematurely and disable batch protection for subsequent writes
        // — violating ADR-021 atomicity on the WASM/srsj path. MemoryStore tests would not
        // catch this regression.
        //
        // store.load_container has no embed fallback: it only resolves a *file-backed*
        // container (store.rs docs) and the catalog treats a physical containers/*.json
        // file sharing the root's id as a fatal SRS038-R12-DUPLICATE-ID ([R1]: the manifest
        // embed is authoritative for the root). A purely embed-backed root — the normal
        // shape once `store.save_container` has synced it into the manifest — has no such
        // file, so `store.load_container` would wrongly error ContainerNotFound. Mirror the
        // None-branch's fallback above via container_service::resolve_root_container.
        let manifest_for_root = store.load_manifest()?;
        let mut persisted_container =
            container_service::resolve_root_container(store, &manifest_for_root)?.ok_or_else(
                || RepositoryError::ContainerNotFound {
                    container_id: root_container_id.clone(),
                },
            )?;
        persisted_container.identity_instance_id = Some(new_id.clone());
        store.save_container(&persisted_container)?;
        Ok(new_id)
    })();
    match batch_result {
        Ok(new_id) => {
            if let Err(e) = store.commit_batch() {
                store.abort_batch();
                return Err(e);
            }
            Ok(MigrateIdentityResult {
                old_identity_id: Some(old_id),
                old_identity_tier: Some(old_tier),
                new_identity_id: new_id,
                statement,
                title,
            })
        }
        Err(e) => {
            store.abort_batch();
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service::get_container;
    use crate::core_purpose;
    use crate::index::InstanceIndexEntry;
    use crate::repository_portability::copy_repository;
    use crate::store::memory::MemoryStore;
    use crate::store::RecordTier;
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

    fn make_note(id: &str, title: Option<&str>, sections: Vec<NoteSection>) -> Note {
        Note {
            instance_id: id.to_string(),
            title: title.map(|t| t.to_string()),
            sections,
            tags: None,
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    /// Build a MemoryStore with a root container and a Tier-0 identity note.
    /// Returns (store, root_container_id).
    fn make_store_with_identity(
        note_id: &str,
        note_title: Option<&str>,
        sections: Vec<NoteSection>,
    ) -> (MemoryStore, String) {
        let store = MemoryStore::default();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";

        let note = make_note(note_id, note_title, sections);
        let note_path = "records/notes/identity.json";
        write_note(&store, &note, note_path).unwrap();

        // Embed-only root container ([R1]): manifest.container is the authoritative
        // declaration. A physical containers/*.json file with the same id would be a
        // fatal SRS038-R12-DUPLICATE-ID under the catalog's duplicate-id check.
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

    #[test]
    fn migrate_creates_purpose_record() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111111",
            Some("My Repo"),
            one_section("I build SRS."),
        );
        let result = migrate_identity(&store).unwrap();
        let expected_path = format!(
            "{}/purpose-{}.json",
            store.record_tier_dir(RecordTier::Tier2),
            &result.new_identity_id[..8]
        );
        let raw = store.load_instance_json(&expected_path).unwrap();
        assert_eq!(
            raw["typeNamespace"].as_str(),
            Some(core_purpose::PURPOSE_TYPE_NAMESPACE)
        );
        assert_eq!(
            raw["typeName"].as_str(),
            Some(core_purpose::PURPOSE_TYPE_NAME)
        );
        assert_eq!(
            raw["instanceId"].as_str(),
            Some(result.new_identity_id.as_str())
        );
        // Regression guard for #441: migrate_identity's own purpose-record builds must
        // use the same carrier keys as the repo-create scaffold path (core_purpose module).
        let keys = raw["fieldValues"]
            .as_object()
            .expect("fieldValues must be a name-keyed object (RFC-039)");
        assert!(
            keys.contains_key("statement"),
            "migrated purpose record must carry the core 'statement' key"
        );
        assert!(
            keys.contains_key("title"),
            "migrated purpose record must carry the core 'title' key"
        );
    }

    #[test]
    fn migrate_identity_recognizes_scaffolded_purpose_record() {
        use crate::repository_lifecycle::{
            create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
            RepositoryMetadata,
        };

        // MemoryStore::uninitialized() — not ::default() — because
        // create_repository_with_intent errors with RepositoryAlreadyExists on an
        // already-initialized store.
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

        let result = create_repository_with_intent(&store, &input).unwrap();
        let scaffolded_id = result.identity_instance_id.unwrap();

        // RFC-038: instance discoverability comes from the catalog (tree-derived), not
        // manifest.instance_index — create_record no longer writes the latter.
        let cat = store.catalog().unwrap();
        let entry = cat
            .instances
            .iter()
            .find(|e| e.id == scaffolded_id)
            .unwrap();
        let record: srs_core::types::record::Record = serde_json::from_value(
            store
                .load_instance_json(entry.locator.as_deref().unwrap())
                .unwrap(),
        )
        .unwrap();

        // Regression guard for #441: repo create's scaffold and repo migrate-identity
        // previously used divergent purpose-record builds. This ties the scaffold's
        // actual output to the same carrier keys migrate_identity reads.
        assert!(
            record.field_values.contains_key("statement"),
            "scaffolded record must carry the core 'statement' key"
        );
        assert!(
            record.field_values.contains_key("title"),
            "scaffolded record must carry the core 'title' key"
        );

        let err = migrate_identity(&store).unwrap_err();
        match err {
            RepositoryError::InvalidInput { message } => {
                assert!(
                    message.contains("no migration needed"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn migrate_sets_statement_from_note_sections() {
        let sections = vec![
            NoteSection {
                name: "s1".to_string(),
                label: None,
                content: "First.".to_string(),
                content_hint: None,
                tags: None,
            },
            NoteSection {
                name: "s2".to_string(),
                label: None,
                content: "Second.".to_string(),
                content_hint: None,
                tags: None,
            },
        ];
        let (store, _) =
            make_store_with_identity("11111111-1111-4111-8111-111111111112", None, sections);
        let result = migrate_identity(&store).unwrap();
        assert_eq!(result.statement, "First.\nSecond.");
    }

    #[test]
    fn migrate_uses_title_from_note() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111113",
            Some("The Repo Title"),
            one_section("Content."),
        );
        let result = migrate_identity(&store).unwrap();
        assert_eq!(result.title, Some("The Repo Title".to_string()));
    }

    #[test]
    fn migrate_updates_manifest_identity_pointer() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111114",
            Some("Repo"),
            one_section("Content."),
        );
        let result = migrate_identity(&store).unwrap();
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.container.unwrap().identity_instance_id,
            Some(result.new_identity_id)
        );
    }

    #[test]
    fn migrate_adds_new_and_removes_old_from_container_members() {
        let old_id = "11111111-1111-4111-8111-111111111115";
        let (store, container_id) =
            make_store_with_identity(old_id, Some("Repo"), one_section("Content."));
        let result = migrate_identity(&store).unwrap();
        let container = get_container(&store, &container_id).unwrap();
        let members = container.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&result.new_identity_id),
            "expected new_identity_id in members, got: {members:?}"
        );
        assert!(
            !members.contains(&old_id.to_string()),
            "expected old_identity_id removed from members, got: {members:?}"
        );
    }

    #[test]
    fn migrate_updates_persisted_container_identity_pointer() {
        let (store, container_id) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111118",
            Some("Repo"),
            one_section("Content."),
        );
        let result = migrate_identity(&store).unwrap();
        let container = get_container(&store, &container_id).unwrap();
        assert_eq!(
            container.identity_instance_id,
            Some(result.new_identity_id),
            "persisted Container record must have identityInstanceId updated after migration"
        );
    }

    #[test]
    fn migrate_index_entry_has_tier_2() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111116",
            Some("Repo"),
            one_section("Content."),
        );
        let result = migrate_identity(&store).unwrap();
        // RFC-038: discoverability comes from the catalog, not manifest.instance_index.
        let cat = store.catalog().unwrap();
        let entry = cat
            .instances
            .iter()
            .find(|e| e.id == result.new_identity_id)
            .expect("new_identity_id must be in the catalog's instance set");
        assert_eq!(entry.tier, Some(2));
    }

    #[test]
    fn migrate_errors_if_already_purpose() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111117",
            Some("Repo"),
            one_section("Content."),
        );
        migrate_identity(&store).unwrap();
        let err = migrate_identity(&store).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("already")),
            "expected already-migrated error, got: {err:?}"
        );
    }

    #[test]
    fn migrate_errors_if_tier2_non_purpose() {
        // Set up a Tier-2 record that is NOT a purpose record as the identity.
        let store = MemoryStore::default();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";

        let instance_id = "22222222-2222-4222-8222-222222222222";
        let record_json = serde_json::json!({
            "instanceId": instance_id,
            "typeNamespace": "com.example",
            "typeName": "some-other-type",
            "typeId": "99999999-9999-4999-a999-999999999999",
            "typeVersion": 1,
            "fieldValues": {}
        });
        store
            .save_instance_json(&format!("records/tier-2/{instance_id}.json"), &record_json)
            .unwrap();

        let mut manifest = store.load_manifest().unwrap();
        let mut root = bare_container(container_id);
        root.identity_instance_id = Some(instance_id.to_string());
        manifest.container = Some(root);
        manifest.instance_index.push(InstanceIndexEntry {
            instance_id: instance_id.to_string(),
            tier: 2,
            path: format!("records/tier-2/{instance_id}.json"),
            title: None,
            tags: None,
        });
        write_manifest(&store, &manifest).unwrap();

        let err = migrate_identity(&store).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("manual migration")),
            "expected non-purpose tier-2 error, got: {err:?}"
        );
    }

    #[test]
    fn cross_store_roundtrip() {
        let (source, container_id) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111119",
            Some("Repo"),
            one_section("Content."),
        );
        let result = migrate_identity(&source).unwrap();

        // copy_repository does not transfer manifest.container metadata; it does
        // transfer instances, the instance index, and container files (with members).
        let target = MemoryStore::uninitialized();
        copy_repository(&source, &target).unwrap();

        // Verify the purpose record is present in the target's catalog (RFC-038:
        // discoverability is tree-derived, not manifest.instance_index).
        let cat = target.catalog().unwrap();
        let entry = cat
            .instances
            .iter()
            .find(|e| e.id == result.new_identity_id)
            .expect("purpose record must be in the catalog's instance set after roundtrip");
        assert_eq!(entry.tier, Some(2));

        // Verify the container in the target still has the purpose record as a member,
        // and that identityInstanceId was carried across (regression for #462).
        let container = get_container(&target, &container_id).unwrap();
        let members = container.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&result.new_identity_id),
            "purpose record must remain in container members after roundtrip"
        );
        assert_eq!(
            container.identity_instance_id,
            Some(result.new_identity_id),
            "container identityInstanceId must be preserved after copy_repository roundtrip"
        );
    }

    /// Build a MemoryStore with a root container whose `identity_instance_id` is `None`.
    /// Returns (store, root_container_id).
    fn make_store_without_identity(
        title: &str,
        description: Option<&str>,
    ) -> (MemoryStore, String) {
        let store = MemoryStore::default();
        let container_id = "660e8400-e29b-41d4-a716-446655440001";
        let mut container = bare_container(container_id);
        container.title = title.to_string();
        container.description = description.map(|d| d.to_string());
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(container);
        write_manifest(&store, &manifest).unwrap();
        (store, container_id.to_string())
    }

    #[test]
    fn migrate_creates_purpose_from_container_title_and_description() {
        let (store, _) = make_store_without_identity("My Repo", Some("We build SRS."));
        let result = migrate_identity(&store).unwrap();
        assert_eq!(result.statement, "We build SRS.");
        assert_eq!(result.title, Some("My Repo".to_string()));
        assert!(result.old_identity_id.is_none());
        assert!(result.old_identity_tier.is_none());
        let expected_path = format!(
            "{}/purpose-{}.json",
            store.record_tier_dir(RecordTier::Tier2),
            &result.new_identity_id[..8]
        );
        assert!(
            store.load_instance_json(&expected_path).is_ok(),
            "purpose record must exist at {expected_path}"
        );
    }

    #[test]
    fn migrate_creates_purpose_from_container_title_only() {
        let (store, _) = make_store_without_identity("My Repo", None);
        let result = migrate_identity(&store).unwrap();
        assert_eq!(result.statement, "My Repo");
        assert_eq!(result.title, Some("My Repo".to_string()));
    }

    #[test]
    fn migrate_from_container_sets_identity_instance_id() {
        let (store, _) = make_store_without_identity("My Repo", Some("We build SRS."));
        let result = migrate_identity(&store).unwrap();
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.container.unwrap().identity_instance_id,
            Some(result.new_identity_id)
        );
    }

    #[test]
    fn migrate_from_container_adds_to_members() {
        let (store, container_id) = make_store_without_identity("My Repo", Some("We build SRS."));
        let result = migrate_identity(&store).unwrap();
        let container = get_container(&store, &container_id).unwrap();
        let members = container.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&result.new_identity_id),
            "new_identity_id must be in container members, got: {members:?}"
        );
    }

    #[test]
    fn migrate_creates_purpose_from_description_when_title_empty() {
        // Empty title + non-empty description: statement = description, no title field in record.
        let store = MemoryStore::default();
        let container_id = "660e8400-e29b-41d4-a716-446655440003";
        let mut container = bare_container(container_id);
        container.title = "".to_string();
        container.description = Some("We build SRS.".to_string());
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(container);
        write_manifest(&store, &manifest).unwrap();

        let result = migrate_identity(&store).unwrap();
        assert_eq!(result.statement, "We build SRS.");
        assert!(
            result.title.is_none(),
            "title must be None when container title is empty"
        );
    }

    #[test]
    fn migrate_from_container_errors_if_empty_title_and_no_description() {
        // Bypass create_container (which validates title is non-empty) to set up the error case.
        let store = MemoryStore::default();
        let container_id = "660e8400-e29b-41d4-a716-446655440002";
        let mut container = bare_container(container_id);
        container.title = "".to_string();
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(container);
        write_manifest(&store, &manifest).unwrap();

        let err = migrate_identity(&store).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("no title or description")),
            "expected no-title-or-description error, got: {err:?}"
        );
    }

    #[test]
    fn migrate_from_container_errors_on_second_run() {
        let (store, _) = make_store_without_identity("My Repo", Some("We build SRS."));
        migrate_identity(&store).unwrap();
        let err = migrate_identity(&store).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("already")),
            "expected already-migrated error on second run, got: {err:?}"
        );
    }

    #[test]
    fn cross_store_roundtrip_none_branch() {
        let (source, container_id) = make_store_without_identity("My Repo", Some("We build SRS."));
        let result = migrate_identity(&source).unwrap();

        let target = MemoryStore::uninitialized();
        copy_repository(&source, &target).unwrap();

        // RFC-038: discoverability comes from the catalog, not manifest.instance_index.
        let cat = target.catalog().unwrap();
        let entry = cat
            .instances
            .iter()
            .find(|e| e.id == result.new_identity_id)
            .expect("purpose record must be in the catalog's instance set after roundtrip");
        assert_eq!(entry.tier, Some(2));

        let container = get_container(&target, &container_id).unwrap();
        let members = container.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&result.new_identity_id),
            "purpose record must be in container members after roundtrip"
        );
    }

    /// Health-check for issue #518: migrate_identity on a FileStore repo created by
    /// create_repository_with_intent must report "already migrated" (purpose-record identity),
    /// not crash, panic, or produce an unexpected error variant.
    #[test]
    fn migrate_identity_on_file_store_repo_created_by_create_repository_with_intent() {
        use crate::repository_lifecycle::{
            create_repository_with_intent, InitializeRepositoryInput, PrimaryPackageMetadata,
            RepositoryMetadata,
        };
        use crate::store::FileStore;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let store = FileStore::new(tmp.path());
        let input = InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "a0000518-0000-4000-8000-000000000518".to_string(),
                namespace: "com.semanticops.test".to_string(),
                srs_version: "2.0-draft".to_string(),
                title: Some("FileStore Repo".to_string()),
                description: Some("Health check for #518.".to_string()),
            },
            primary_package: PrimaryPackageMetadata {
                id: "pkg-1".to_string(),
                namespace: "com.semanticops.test".to_string(),
                name: "primary".to_string(),
                version: "1.0.0".to_string(),
            },
        };

        create_repository_with_intent(&store, &input).unwrap();

        // Purpose-record identity → migrate_identity returns "no migration needed" early
        // without reaching load_container. The test confirms no crash or unexpected error.
        let err = migrate_identity(&store).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("no migration needed")),
            "expected already-migrated error on FileStore repo, got: {err:?}"
        );
    }

    /// Regression test for #607: None-branch migration must not erase pre-existing section
    /// members from the container file. Before the fix, `save_container(&manifest.container)`
    /// persisted the manifest embed (containing only the new identity member), overwriting the
    /// real container file and losing all section members.
    #[test]
    fn none_branch_migration_preserves_pre_existing_section_members() {
        let section_member_id = "aaaa0000-0000-4000-8000-000000000001";

        let store = MemoryStore::default();
        let container_id = "660e8400-e29b-41d4-a716-446655440099";
        let mut container = bare_container(container_id);
        container.title = "My Repo".to_string();
        container.description = Some("We build SRS.".to_string());
        // Pre-existing section member that must survive migration. Must be a real
        // catalog-valid instance — a member id with no backing file is a fatal
        // SRS038-R13-DANGLING-REFERENCE.
        container.member_instance_ids = Some(vec![section_member_id.to_string()]);
        let section_note = make_note(section_member_id, Some("Section"), one_section("Body."));
        write_note(&store, &section_note, "records/notes/section.json").unwrap();
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(container);
        write_manifest(&store, &manifest).unwrap();

        let result = migrate_identity(&store).unwrap();

        let persisted = get_container(&store, container_id).unwrap();
        let members = persisted.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&section_member_id.to_string()),
            "pre-existing section member must survive None-branch migration, got: {members:?}"
        );
        assert!(
            members.contains(&result.new_identity_id),
            "new identity must also be in container members, got: {members:?}"
        );
    }
}
