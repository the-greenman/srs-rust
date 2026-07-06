use crate::container_service::{add_container_member, add_root, create_container};
use crate::error::RepositoryError;
use crate::manifest_service::{set_manifest_root_container, SetManifestRootContainerInput};
use crate::record_store::{create_record_in_context, CreateRecordInput};
use crate::repository_lifecycle::{init_new_repository, InitNewRepositoryInput};
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::types::container::Container;
use srs_core::types::record::FieldValue;
use std::collections::HashMap;

/// Input for `scaffold_governance_repo`.
///
/// Caller supplies the human-facing title and optional purpose text. A pre-seeded
/// store (loaded from `governance-seed.srsj` and identity-stamped) is required —
/// this function only writes records and containers; it does not create the package.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldGovernanceRepoInput {
    pub title: String,
    pub purpose: Option<String>,
}

/// Result of `scaffold_governance_repo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldGovernanceRepoResult {
    pub identity_record_id: String,
    pub decision_log_container_id: String,
    pub decision_log_root_id: String,
    pub root_container_id: String,
}

/// Combined input for `create_governance_repository`.
///
/// Stamps manifest identity and scaffolds all governance records in a single call,
/// so CLI handlers and WASM bindings need exactly one service call.
///
/// `namespace`: when `None` (or omitted in JSON), derived as `"com.example.<slug>"`
/// where `<slug>` is the title lowercased with spaces replaced by hyphens.
///
/// `repository_id`: when `None`, a UUID v4 is minted inside the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGovernanceRepositoryInput {
    #[serde(default)]
    pub namespace: Option<String>,
    pub title: String,
    pub purpose: Option<String>,
    pub repository_id: Option<String>,
}

/// Result of `create_governance_repository`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGovernanceRepositoryResult {
    pub repository_id: String,
    pub identity_record_id: String,
    pub decision_log_container_id: String,
    pub decision_log_root_id: String,
    pub root_container_id: String,
}

/// Derive a default namespace from a repository title.
///
/// Produces `"com.example.<slug>"` where `<slug>` is the title lowercased,
/// stripped of non-alphanumeric-non-space characters, with spaces replaced
/// by hyphens (e.g. `"My Org"` → `"com.example.my-org"`).
///
/// `"com.example."` is an intentional placeholder prefix. Callers that require
/// a different organisational prefix should supply an explicit `namespace` instead
/// of relying on the derived default.
fn derive_namespace_from_title(title: &str) -> String {
    let slug = title
        .to_lowercase()
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("com.example.{slug}")
}

/// Scaffold governance records into an already-stamped seed store.
///
/// Writes three records and two containers:
/// - `governance/article` identity record (title + purpose)
/// - `governance/decision_log` container + root record
/// - untyped root container linking identity and decision-log root
///
/// The store's `manifest.container` navigation pointer is set to the root container
/// via `set_manifest_root_container`.
pub fn scaffold_governance_repo(
    store: &dyn RepositoryStore,
    input: ScaffoldGovernanceRepoInput,
) -> Result<ScaffoldGovernanceRepoResult, RepositoryError> {
    if input.title.trim().is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "title must not be empty".to_string(),
        });
    }

    let purpose_text = input
        .purpose
        .as_deref()
        .unwrap_or("Add your group's purpose statement here.");

    let package = store.load_package()?;
    let title_field_id = package
        .find_field_by_name("title")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "title field not found in package".to_string(),
        })?
        .id
        .clone();
    let article_text_field_id = package
        .find_field_by_name("article_text")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "article_text field not found in package".to_string(),
        })?
        .id
        .clone();

    // 1. Identity record: governance/article carrying title + purpose.
    let identity = create_record_in_context(
        store,
        "governance/article",
        None,
        CreateRecordInput {
            field_values: vec![
                FieldValue {
                    field_id: title_field_id.clone(),
                    value: serde_json::json!(input.title),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
                FieldValue {
                    field_id: article_text_field_id,
                    value: serde_json::json!(purpose_text),
                    entries: None,
                    source: None,
                    edited_at: None,
                },
            ],
            group_values: None,
            tags: None,
        },
        None,
        None,
    )?;
    let identity_id = identity.record.instance_id.clone();

    // 2. Decision Log container + root record.
    let dl_title = format!("{} Decision Log", input.title);
    let dl_container = create_container(
        store,
        Container {
            container_id: String::new(),
            title: dl_title.clone(),
            container_type: Some("decision_log".to_string()),
            namespace: None,
            name: None,
            description: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        },
    )?;
    let dl_container_id = dl_container.container_id.clone();

    let dl_root = create_record_in_context(
        store,
        "governance/decision_log",
        None,
        CreateRecordInput {
            field_values: vec![FieldValue {
                field_id: title_field_id.clone(),
                value: serde_json::json!(dl_title),
                entries: None,
                source: None,
                edited_at: None,
            }],
            group_values: None,
            tags: None,
        },
        None,
        None,
    )?;
    let dl_root_id = dl_root.record.instance_id.clone();
    add_root(store, &dl_container_id, &dl_root_id)?;

    // 3. Root container: untyped structural anchor.
    //    Members: identity + dl root (navigation reads memberInstanceIds for sections).
    //    Root: identity (the structural root of the document).
    let root_container = create_container(
        store,
        Container {
            container_id: String::new(),
            title: input.title.clone(),
            container_type: None,
            namespace: None,
            name: None,
            description: None,
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: HashMap::new(),
        },
    )?;
    let root_container_id = root_container.container_id.clone();

    add_container_member(store, &root_container_id, &identity_id)?;
    add_container_member(store, &root_container_id, &dl_root_id)?;
    add_root(store, &root_container_id, &identity_id)?;

    set_manifest_root_container(
        store,
        SetManifestRootContainerInput {
            container_id: root_container_id.clone(),
            identity_instance_id: identity_id.clone(),
        },
    )?;

    Ok(ScaffoldGovernanceRepoResult {
        identity_record_id: identity_id,
        decision_log_container_id: dl_container_id,
        decision_log_root_id: dl_root_id,
        root_container_id,
    })
}

/// Stamp manifest identity and scaffold all governance records in a single call.
///
/// The store must already contain a seeded `.srsj` bundle (loaded via
/// `JsonStore::from_srsj` after RFC-014 migration). This function:
/// 1. Calls `init_new_repository` to stamp `repositoryId`, `namespace`, `title`,
///    and `upstreamPackage.installedAt` into the manifest.
/// 2. Calls `scaffold_governance_repo` to create records + containers.
///
/// This is the one service call made by CLI handlers and WASM bindings for
/// governance repository creation (ADR-010: one service call per handler).
pub fn create_governance_repository(
    store: &dyn RepositoryStore,
    input: CreateGovernanceRepositoryInput,
) -> Result<CreateGovernanceRepositoryResult, RepositoryError> {
    // Fast-path guards: fail before namespace derivation. init_new_repository
    // also validates these, but catching them here avoids computing a derived
    // namespace from a blank title only to reject it a call later.
    if let Some(ref ns) = input.namespace {
        if ns.trim().is_empty() {
            return Err(RepositoryError::InvalidRepositoryInitialization {
                message: "namespace must not be empty".to_string(),
            });
        }
    }
    if input.title.trim().is_empty() {
        return Err(RepositoryError::InvalidRepositoryInitialization {
            message: "title must not be empty".to_string(),
        });
    }

    let namespace = input
        .namespace
        .unwrap_or_else(|| derive_namespace_from_title(&input.title));

    let init_result = init_new_repository(
        store,
        InitNewRepositoryInput {
            repository_id: input.repository_id,
            namespace,
            title: input.title.clone(),
            description: None,
        },
    )?;

    let scaffold = scaffold_governance_repo(
        store,
        ScaffoldGovernanceRepoInput {
            title: input.title,
            purpose: input.purpose,
        },
    )?;

    Ok(CreateGovernanceRepositoryResult {
        repository_id: init_result.repository_id,
        identity_record_id: scaffold.identity_record_id,
        decision_log_container_id: scaffold.decision_log_container_id,
        decision_log_root_id: scaffold.decision_log_root_id,
        root_container_id: scaffold.root_container_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_store::JsonStore;

    // Note: these tests use JsonStore exclusively rather than MemoryStore.
    // `scaffold_governance_repo` and `create_governance_repository` require a pre-seeded
    // store that already contains a governance package (record types, fields, views).
    // MemoryStore starts empty and cannot be pre-seeded without re-implementing the seed
    // loading logic — the canonical way to create a seeded store is JsonStore::from_srsj.
    // The JsonStore roundtrip test (below) demonstrates store-agnosticism at the service
    // boundary: changes made by the service survive serialisation and re-parse.

    fn load_seed_store() -> JsonStore {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/governance-seed.srsj"
        ))
        .expect("governance-seed.srsj must be present in crates/srs-repository/tests/fixtures/");
        let migrated =
            crate::srsj_migration_service::migrate_rfc014(&raw).expect("RFC-014 migration");
        JsonStore::from_srsj(&migrated).expect("seed parses as JsonStore")
    }

    #[test]
    fn scaffold_creates_required_records_and_containers() {
        let store = load_seed_store();
        let result = scaffold_governance_repo(
            &store,
            ScaffoldGovernanceRepoInput {
                title: "Test Org".to_string(),
                purpose: Some("To test things.".to_string()),
            },
        )
        .expect("scaffold succeeds");

        assert!(!result.identity_record_id.is_empty());
        assert!(!result.decision_log_container_id.is_empty());
        assert!(!result.decision_log_root_id.is_empty());
        assert!(!result.root_container_id.is_empty());

        // All IDs are distinct
        let ids = [
            &result.identity_record_id,
            &result.decision_log_container_id,
            &result.decision_log_root_id,
            &result.root_container_id,
        ];
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 4, "all result IDs must be distinct");
    }

    #[test]
    fn scaffold_uses_default_purpose_when_none_provided() {
        let store = load_seed_store();
        let result = scaffold_governance_repo(
            &store,
            ScaffoldGovernanceRepoInput {
                title: "No Purpose Org".to_string(),
                purpose: None,
            },
        )
        .expect("scaffold with None purpose succeeds");
        assert!(!result.identity_record_id.is_empty());
    }

    #[test]
    fn scaffold_rejects_empty_title() {
        let store = load_seed_store();
        let err = scaffold_governance_repo(
            &store,
            ScaffoldGovernanceRepoInput {
                title: "  ".to_string(),
                purpose: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, RepositoryError::InvalidRepositoryInitialization { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn create_governance_repository_stamps_manifest_and_scaffolds() {
        let store = load_seed_store();
        let result = create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("com.example.test".to_string()),
                title: "Example Gov".to_string(),
                purpose: Some("A test governance repo.".to_string()),
                repository_id: Some("test-repo-id-1234".to_string()),
            },
        )
        .expect("create_governance_repository succeeds");

        assert_eq!(result.repository_id, "test-repo-id-1234");
        assert!(!result.identity_record_id.is_empty());
        assert!(!result.root_container_id.is_empty());

        // Manifest must carry the stamped identity
        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.extra.get("repositoryId").and_then(|v| v.as_str()),
            Some("test-repo-id-1234")
        );
        assert_eq!(
            manifest.extra.get("namespace").and_then(|v| v.as_str()),
            Some("com.example.test")
        );
        assert_eq!(
            manifest.extra.get("title").and_then(|v| v.as_str()),
            Some("Example Gov")
        );
        // installedAt must be set on the RFC-014 top-level upstreamPackage
        let installed_at = manifest
            .extra
            .get("upstreamPackage")
            .and_then(|v| v.get("installedAt"))
            .and_then(|v| v.as_str());
        assert!(
            installed_at.is_some(),
            "upstreamPackage.installedAt must be stamped"
        );
    }

    #[test]
    fn create_governance_repository_mints_uuid_when_no_id_given() {
        let store = load_seed_store();
        let result = create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("com.example.mint".to_string()),
                title: "Minted ID Org".to_string(),
                purpose: None,
                repository_id: None,
            },
        )
        .expect("mints uuid");
        assert!(
            result.repository_id.len() >= 32,
            "minted id looks like a UUID: {}",
            result.repository_id
        );
    }

    #[test]
    fn create_governance_repository_rejects_empty_title() {
        let store = load_seed_store();
        let err = create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("com.example.empty".to_string()),
                title: "  ".to_string(),
                purpose: None,
                repository_id: None,
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::InvalidRepositoryInitialization { .. }
        ));
    }

    #[test]
    fn create_governance_repository_rejects_empty_namespace() {
        let store = load_seed_store();
        let err = create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("  ".to_string()),
                title: "Some Org".to_string(),
                purpose: None,
                repository_id: None,
            },
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RepositoryError::InvalidRepositoryInitialization { .. }
        ));
    }

    #[test]
    fn create_governance_repository_derives_namespace_from_title() {
        let store = load_seed_store();
        create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: None,
                title: "Test Org".to_string(),
                purpose: None,
                repository_id: Some("derived-ns-test".to_string()),
            },
        )
        .expect("create with None namespace succeeds");

        let manifest = store.load_manifest().unwrap();
        assert_eq!(
            manifest.extra.get("namespace").and_then(|v| v.as_str()),
            Some("com.example.test-org"),
            "namespace must be derived as com.example.<slug> when not provided"
        );
    }

    #[test]
    fn derive_namespace_roundtrip_survives_srsj_serialisation() {
        let store = load_seed_store();
        create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: None,
                title: "Roundtrip Derived".to_string(),
                purpose: None,
                repository_id: Some("derived-roundtrip-id".to_string()),
            },
        )
        .expect("create with None namespace succeeds");

        let srsj = store.to_srsj_string().expect("to_srsj_string");
        let store2 = JsonStore::from_srsj(&srsj).expect("re-parse");
        let manifest = store2.load_manifest().unwrap();
        assert_eq!(
            manifest.extra.get("namespace").and_then(|v| v.as_str()),
            Some("com.example.roundtrip-derived"),
            "derived namespace must survive to_srsj_string → from_srsj roundtrip"
        );
    }

    #[test]
    fn derive_namespace_strips_special_characters() {
        assert_eq!(
            derive_namespace_from_title("O'Reilly Org"),
            "com.example.oreilly-org"
        );
        assert_eq!(
            derive_namespace_from_title("Acme & Co."),
            "com.example.acme-co"
        );
    }

    #[test]
    fn json_store_roundtrip() {
        let store = load_seed_store();
        create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("com.example.roundtrip".to_string()),
                title: "Roundtrip Org".to_string(),
                purpose: Some("Testing roundtrip.".to_string()),
                repository_id: Some("roundtrip-id".to_string()),
            },
        )
        .expect("create succeeds");

        // Serialise → re-parse → check all stamped fields survive
        let srsj = store.to_srsj_string().expect("to_srsj_string");
        let store2 = JsonStore::from_srsj(&srsj).expect("re-parse");
        let manifest = store2.load_manifest().unwrap();
        assert_eq!(
            manifest.extra.get("repositoryId").and_then(|v| v.as_str()),
            Some("roundtrip-id")
        );
        assert_eq!(
            manifest.extra.get("namespace").and_then(|v| v.as_str()),
            Some("com.example.roundtrip")
        );
        assert_eq!(
            manifest.extra.get("title").and_then(|v| v.as_str()),
            Some("Roundtrip Org")
        );
    }
}
