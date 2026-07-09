use crate::container_service;
use crate::core_purpose;
use crate::error::RepositoryError;
use crate::index::InstanceIndexEntry;
use crate::loader;
use crate::paths;
use crate::record_store::write_new_record;
use crate::store::RepositoryStore;
use crate::writer;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateIdentityResult {
    pub old_identity_id: String,
    pub old_identity_tier: u8,
    pub new_identity_id: String,
    pub statement: String,
    pub title: Option<String>,
}

fn extract_identity_text(
    store: &dyn RepositoryStore,
    entry: &InstanceIndexEntry,
) -> Result<(String, Option<String>), RepositoryError> {
    match entry.tier() {
        0 => {
            let note = loader::load_note(store, entry.path())?;
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
                "identity instance '{}' is a Tier-2 record that is not a \
                 com.semanticops.core/purpose record; manual migration is required \
                 or the identity must first be changed to a Tier-0 note",
                entry.instance_id()
            ),
        }),
        t => Err(RepositoryError::InvalidInput {
            message: format!(
                "unsupported identity tier {t}: only Tier-0 and Tier-2 identities can be migrated"
            ),
        }),
    }
}

pub fn migrate_identity(
    store: &dyn RepositoryStore,
) -> Result<MigrateIdentityResult, RepositoryError> {
    let mut manifest = store.load_manifest()?;

    let mc = manifest
        .container
        .as_ref()
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: "manifest.container is not set".to_string(),
        })?
        .clone();

    let old_id = mc
        .identity_instance_id
        .clone()
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: "manifest.container.identityInstanceId is not set".to_string(),
        })?;

    let root_container_id = mc.container_id.clone();

    let entry = manifest
        .instance_index
        .iter()
        .find(|e| e.instance_id() == old_id)
        .cloned()
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: format!("identity instance '{old_id}' not found in instanceIndex"),
        })?;

    if entry.tier() == 2 {
        let raw = store.load_instance_json(entry.path())?;
        let ns_ok =
            raw.get("typeNamespace").and_then(|v| v.as_str()) == Some(core_purpose::PURPOSE_TYPE_NAMESPACE);
        let name_ok = raw.get("typeName").and_then(|v| v.as_str()) == Some(core_purpose::PURPOSE_TYPE_NAME);
        if ns_ok && name_ok {
            return Err(RepositoryError::InvalidInput {
                message: "already a com.semanticops.core/purpose record; no migration needed"
                    .to_string(),
            });
        }
    }

    let old_tier = entry.tier();
    let (statement, title) = extract_identity_text(store, &entry)?;

    let new_id = writer::new_instance_id();
    let now = chrono::Utc::now().to_rfc3339();

    let record = core_purpose::build_purpose_record(&new_id, &statement, title.as_deref(), &now);

    let relative_path = format!(
        "{}/purpose-{}.json",
        paths::DEFAULT_RECORD_DIR,
        &new_id[..8]
    );

    // Update manifest in memory: push index entry and repoint identity pointer.
    manifest.instance_index.push(InstanceIndexEntry {
        instance_id: new_id.clone(),
        tier: 2,
        path: relative_path.clone(),
        title: None,
        tags: None,
    });
    if let Some(ref mut mc) = manifest.container {
        mc.identity_instance_id = Some(new_id.clone());
    }

    // ADR-021 batch: record file + manifest + container membership atomically.
    // Also remove the old identity from container members: it was there only to satisfy
    // RFC-013 I-81 (identity must be a member). After migration the new record takes
    // that slot; leaving the old note would trigger RFC-013 I-82 (non-identity member
    // not rooting a section container).
    store.begin_batch();
    let batch_result = (|| -> Result<(), RepositoryError> {
        write_new_record(store, &record, paths::DEFAULT_RECORD_DIR)?;
        writer::write_manifest(store, &manifest)?;
        container_service::add_container_member(store, &root_container_id, &new_id)?;
        container_service::remove_container_member(store, &root_container_id, &old_id)?;
        Ok(())
    })();
    match batch_result {
        Ok(()) => {
            if let Err(e) = store.commit_batch() {
                store.abort_batch();
                return Err(e);
            }
        }
        Err(e) => {
            store.abort_batch();
            return Err(e);
        }
    }

    Ok(MigrateIdentityResult {
        old_identity_id: old_id,
        old_identity_tier: old_tier,
        new_identity_id: new_id,
        statement,
        title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service::{create_container, get_container};
    use crate::core_purpose;
    use crate::repository_portability::copy_repository;
    use crate::store::memory::MemoryStore;
    use crate::writer::{upsert_index_entry, write_manifest, write_note};
    use srs_core::types::container::Container;
    use srs_core::types::note::{Note, NoteSection};
    use std::collections::HashMap;

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
            extra: HashMap::new(),
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

        create_container(&store, bare_container(container_id)).unwrap();

        let note = make_note(note_id, note_title, sections);
        let note_path = "records/notes/identity.json";
        write_note(&store, &note, note_path).unwrap();

        let mut manifest = store.load_manifest().unwrap();
        let mut root = bare_container(container_id);
        root.identity_instance_id = Some(note_id.to_string());
        manifest.container = Some(root);
        upsert_index_entry(&mut manifest, &note, note_path);
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
            paths::DEFAULT_RECORD_DIR,
            &result.new_identity_id[..8]
        );
        let raw = store.load_instance_json(&expected_path).unwrap();
        assert_eq!(
            raw["typeNamespace"].as_str(),
            Some(core_purpose::PURPOSE_TYPE_NAMESPACE)
        );
        assert_eq!(raw["typeName"].as_str(), Some(core_purpose::PURPOSE_TYPE_NAME));
        assert_eq!(
            raw["instanceId"].as_str(),
            Some(result.new_identity_id.as_str())
        );
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
    fn migrate_index_entry_has_tier_2() {
        let (store, _) = make_store_with_identity(
            "11111111-1111-4111-8111-111111111116",
            Some("Repo"),
            one_section("Content."),
        );
        let result = migrate_identity(&store).unwrap();
        let manifest = store.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == result.new_identity_id)
            .expect("new_identity_id must be in instanceIndex");
        assert_eq!(entry.tier(), 2);
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
        create_container(&store, bare_container(container_id)).unwrap();

        let instance_id = "22222222-2222-4222-8222-222222222222";
        let record_json = serde_json::json!({
            "instanceId": instance_id,
            "typeNamespace": "com.example",
            "typeName": "some-other-type",
            "typeId": "99999999-9999-4999-a999-999999999999",
            "typeVersion": 1,
            "fieldValues": []
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
    fn migrate_errors_if_no_identity_pointer() {
        let store = MemoryStore::default();
        let container_id = "550e8400-e29b-41d4-a716-446655440000";
        create_container(&store, bare_container(container_id)).unwrap();
        let mut manifest = store.load_manifest().unwrap();
        manifest.container = Some(bare_container(container_id)); // identity_instance_id: None
        write_manifest(&store, &manifest).unwrap();

        let err = migrate_identity(&store).unwrap_err();
        assert!(
            matches!(&err, RepositoryError::InvalidInput { message } if message.contains("identityInstanceId")),
            "expected identityInstanceId error, got: {err:?}"
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

        // Verify the purpose record is present in the target's instance index.
        let manifest = target.load_manifest().unwrap();
        let entry = manifest
            .instance_index
            .iter()
            .find(|e| e.instance_id() == result.new_identity_id)
            .expect("purpose record must be in instanceIndex after roundtrip");
        assert_eq!(entry.tier(), 2);

        // Verify the container in the target still has the purpose record as a member.
        let container = get_container(&target, &container_id).unwrap();
        let members = container.member_instance_ids.unwrap_or_default();
        assert!(
            members.contains(&result.new_identity_id),
            "purpose record must remain in container members after roundtrip"
        );
    }
}
