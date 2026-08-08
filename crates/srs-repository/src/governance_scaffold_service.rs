use crate::container_service::{add_container_member, add_root, create_container};
use crate::error::RepositoryError;
use crate::manifest_service::{set_manifest_root_container, SetManifestRootContainerInput};
use crate::record_store::{create_record_in_context, CreateRecordInput};
use crate::repository_lifecycle::{init_new_repository, InitNewRepositoryInput};
use crate::store::RepositoryStore;
use crate::view_service::{delete_document_view, list_document_views, update_document_view};
use serde::{Deserialize, Serialize};
use srs_core::types::container::Container;
use srs_core::types::record::FieldValues;
use srs_core::types::view::SectionSource;


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
    /// DocumentViews whose sections were rewritten to reference the containers this
    /// scaffold created (srs#163 — the package ships gallery container UUIDs).
    pub rebound_document_view_ids: Vec<String>,
    /// DocumentViews removed because none of their sections can bind to a
    /// scaffold-created container in the release-1 (decision-log-only) shape.
    pub removed_document_view_ids: Vec<String>,
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
    /// See [`ScaffoldGovernanceRepoResult::rebound_document_view_ids`].
    pub rebound_document_view_ids: Vec<String>,
    /// See [`ScaffoldGovernanceRepoResult::removed_document_view_ids`].
    pub removed_document_view_ids: Vec<String>,
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
/// - `com.semanticops.core/purpose` identity record (statement + title; RFC-018 I-81)
/// - `governance/decision_log` container + root record
/// - untyped root container linking identity and decision-log root
///
/// The store's `manifest.container` navigation pointer is set to the root container
/// via `set_manifest_root_container`.
///
/// Finally, the installed document views are re-bound to the containers created above
/// (srs#163): the canonical package ships gallery-example container UUIDs, which are
/// meaningless in a fresh install. See [`rebind_document_views_to_scaffold`].
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
    // RFC-018 I-81: the identity record must be com.semanticops.core/purpose.
    // The core types are available via the ADR-025 implicit-core merge.
    // RFC-039: the carrier keys by Field.name; the lookups below still assert
    // the fields exist in the package before we write records against them.
    package
        .find_field("com.semanticops.core", "statement")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "com.semanticops.core/statement field not found in package".to_string(),
        })?;
    package
        .find_field("com.semanticops.core", "title")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "com.semanticops.core/title field not found in package".to_string(),
        })?;
    package
        .find_field("governance", "title")
        .ok_or_else(|| RepositoryError::InvalidRepositoryInitialization {
            message: "governance/title field not found in package".to_string(),
        })?;

    // 1. Identity record: com.semanticops.core/purpose carrying statement + title (RFC-018 I-81).
    let identity = create_record_in_context(
        store,
        "com.semanticops.core/purpose",
        None,
        CreateRecordInput {
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert("statement", serde_json::json!(purpose_text));
                fv.insert("title", serde_json::json!(input.title));
                fv
            },
            field_meta: None,
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
            extra: std::collections::BTreeMap::new(),
        },
    )?;
    let dl_container_id = dl_container.container_id.clone();

    let dl_root = create_record_in_context(
        store,
        "governance/decision_log",
        None,
        CreateRecordInput {
            field_values: {
                let mut fv = FieldValues::new();
                fv.insert("title", serde_json::json!(dl_title));
                fv
            },
            field_meta: None,
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
            extra: std::collections::BTreeMap::new(),
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
            title: None,
        },
    )?;

    // 4. Re-bind the installed DocumentViews to the containers this scaffold created
    //    (srs#163): the canonical package ships document views whose sections reference
    //    the gallery example's container UUIDs, which do not exist in any fresh install.
    let (rebound_document_view_ids, removed_document_view_ids) =
        rebind_document_views_to_scaffold(store, &dl_container_id)?;

    Ok(ScaffoldGovernanceRepoResult {
        identity_record_id: identity_id,
        decision_log_container_id: dl_container_id,
        decision_log_root_id: dl_root_id,
        root_container_id,
        rebound_document_view_ids,
        removed_document_view_ids,
    })
}

/// The `namespace/name` key of the record type held by the Decision Log container.
/// A TypeQuery section over this type re-binds to the scaffold's decision-log container.
const DECISION_TYPE_KEY: &str = "governance/decision";

/// The stable `sectionId` the canonical governance package uses for decision-log
/// sections across all of its document views (`decision-log`, `decision-deliberation`,
/// `governance-document`).
const DECISIONS_SECTION_ID: &str = "decisions";

/// Does `id` resolve to a Container in this repository?
fn container_exists(store: &dyn RepositoryStore, id: &str) -> bool {
    store.load_container(id).is_ok()
}

/// Rewrite the installed document views so their sections reference the containers the
/// scaffold actually created, instead of the gallery-example container UUIDs shipped in
/// the canonical package (srs#163).
///
/// Matching is by role, not by UUID:
/// - a section is a *decision* section when its TypeQuery targets
///   [`DECISION_TYPE_KEY`] or its `sectionId` is [`DECISIONS_SECTION_ID`]; dangling
///   container references in such sections are re-bound to the freshly created
///   Decision Log container;
/// - sections whose dangling references have no scaffold-time counterpart (Articles /
///   Roles — those types are deliberately dormant in the release-1, decision-log-only
///   shape) are trimmed from the installed view, so a fresh repo validates with zero
///   dangling-reference warnings (#509's validate check) rather than hiding broken
///   sections behind `emptyBehavior`;
/// - a view left with no bindable sections at all (`articles-and-roles`) is removed
///   from the install. Package upgrade (muDemocracy.org#37) is the path that will
///   reintroduce Articles/Roles views once those containers exist.
///
/// Sections whose references all resolve are left untouched. Returns
/// `(rebound_view_ids, removed_view_ids)`.
fn rebind_document_views_to_scaffold(
    store: &dyn RepositoryStore,
    decision_log_container_id: &str,
) -> Result<(Vec<String>, Vec<String>), RepositoryError> {
    let mut rebound = Vec::new();
    let mut removed = Vec::new();

    for view in list_document_views(store)? {
        let mut changed = false;
        let mut kept_sections = Vec::with_capacity(view.sections.len());

        for mut section in view.sections.iter().cloned() {
            let is_decision_section = section.section_id == DECISIONS_SECTION_ID
                || matches!(
                    &section.source,
                    SectionSource::TypeQuery { semantic_object_type, .. }
                        if semantic_object_type == DECISION_TYPE_KEY
                );

            let keep = match &mut section.source {
                SectionSource::ContainerSubset { container_id, .. } => {
                    if container_exists(store, container_id) {
                        true
                    } else if is_decision_section {
                        *container_id = decision_log_container_id.to_string();
                        changed = true;
                        true
                    } else {
                        changed = true;
                        false
                    }
                }
                SectionSource::TypeQuery { container_ids, .. } => match container_ids {
                    Some(ids) if ids.iter().any(|id| !container_exists(store, id)) => {
                        let mut resolved: Vec<String> = ids
                            .iter()
                            .filter(|id| container_exists(store, id))
                            .cloned()
                            .collect();
                        changed = true;
                        if is_decision_section
                            && !resolved.iter().any(|id| id == decision_log_container_id)
                        {
                            resolved.push(decision_log_container_id.to_string());
                        }
                        if resolved.is_empty() {
                            false
                        } else {
                            *ids = resolved;
                            true
                        }
                    }
                    _ => true,
                },
                _ => true,
            };

            if keep {
                kept_sections.push(section);
            }
        }

        if !changed {
            continue;
        }
        if kept_sections.is_empty() {
            delete_document_view(store, &view.id)?;
            removed.push(view.id);
        } else {
            let mut updated = view.clone();
            updated.sections = kept_sections;
            update_document_view(store, &view.id, updated)?;
            rebound.push(view.id);
        }
    }

    Ok((rebound, removed))
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
        rebound_document_view_ids: scaffold.rebound_document_view_ids,
        removed_document_view_ids: scaffold.removed_document_view_ids,
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
    fn scaffold_rebinds_document_views_to_created_containers() {
        // srs#163: the canonical package ships document views referencing gallery
        // container UUIDs. After scaffold, every remaining container reference must
        // resolve, decision sections must point at the scaffold's decision-log
        // container, and views with no bindable section are removed.
        let store = load_seed_store();
        let result = scaffold_governance_repo(
            &store,
            ScaffoldGovernanceRepoInput {
                title: "Rebind Org".to_string(),
                purpose: None,
            },
        )
        .expect("scaffold succeeds");

        let views = crate::view_service::list_document_views(&store).unwrap();
        let dl = &result.decision_log_container_id;

        // articles-and-roles has no bindable section in the release-1 shape → removed.
        assert!(
            !views.iter().any(|v| v.name == "articles-and-roles"),
            "articles-and-roles view must be removed from a fresh install"
        );
        assert_eq!(
            result.removed_document_view_ids,
            vec!["78b11038-e5d8-4269-9982-fe5c459802b2".to_string()],
            "removed set is exactly the articles-and-roles view"
        );

        // decision-log (type-query): explicit containerIds re-bound to the created container.
        let decision_log = views
            .iter()
            .find(|v| v.name == "decision-log")
            .expect("decision-log view present");
        match &decision_log.sections[0].source {
            SectionSource::TypeQuery { container_ids, .. } => {
                assert_eq!(
                    container_ids.as_deref(),
                    Some(std::slice::from_ref(dl)),
                    "decision-log type-query must target the scaffold's decision-log container"
                );
            }
            other => panic!("decision-log section must stay a type-query, got {other:?}"),
        }

        // decision-deliberation (container-subset): re-bound to the created container.
        let deliberation = views
            .iter()
            .find(|v| v.name == "decision-deliberation")
            .expect("decision-deliberation view present");
        match &deliberation.sections[0].source {
            SectionSource::ContainerSubset { container_id, .. } => assert_eq!(container_id, dl),
            other => panic!("deliberation section must stay a container-subset, got {other:?}"),
        }

        // governance-document: articles + roles sections trimmed, decisions re-bound.
        let gov_doc = views
            .iter()
            .find(|v| v.name == "governance-document")
            .expect("governance-document view present");
        assert_eq!(
            gov_doc.sections.len(),
            1,
            "articles/roles sections must be trimmed in the release-1 shape"
        );
        assert_eq!(gov_doc.sections[0].section_id, "decisions");
        match &gov_doc.sections[0].source {
            SectionSource::ContainerSubset { container_id, .. } => assert_eq!(container_id, dl),
            other => {
                panic!("gov-doc decisions section must stay a container-subset, got {other:?}")
            }
        }

        // Every surviving container reference resolves — no dangling refs remain.
        for view in &views {
            for section in &view.sections {
                let refs: Vec<&str> = match &section.source {
                    SectionSource::ContainerSubset { container_id, .. } => {
                        vec![container_id.as_str()]
                    }
                    SectionSource::TypeQuery { container_ids, .. } => container_ids
                        .as_deref()
                        .unwrap_or(&[])
                        .iter()
                        .map(String::as_str)
                        .collect(),
                    _ => Vec::new(),
                };
                for id in refs {
                    assert!(
                        store.load_container(id).is_ok(),
                        "view '{}' section '{}' still references dangling container '{}'",
                        view.name,
                        section.section_id,
                        id
                    );
                }
            }
        }

        // The three surviving views are reported as rebound.
        let mut rebound = result.rebound_document_view_ids.clone();
        rebound.sort();
        let mut expected = vec![
            "5a3ce87e-8340-4d91-a140-ab56b57f704f".to_string(), // decision-deliberation
            "732a982b-3765-4f22-90e0-e456463bac54".to_string(), // governance-document
            "b5c8d124-2084-4a6b-a231-425e800e1e55".to_string(), // decision-log
        ];
        expected.sort();
        assert_eq!(rebound, expected);
    }

    #[test]
    fn create_governance_repository_validates_with_zero_i81_warnings() {
        // RFC-018 I-81: the identity record must be com.semanticops.core/purpose.
        // A freshly created governance repo must not emit any I-81 diagnostic.
        let store = load_seed_store();
        create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("com.example.i81-check".to_string()),
                title: "I-81 Check Org".to_string(),
                purpose: Some("Check I-81 compliance.".to_string()),
                repository_id: Some("i81-check-id".to_string()),
            },
        )
        .expect("create succeeds");

        let srsj = store.to_srsj_string().expect("to_srsj_string");
        let store2 = crate::json_store::JsonStore::from_srsj(&srsj).expect("re-parse");
        let report = crate::validation::validate_repository(&store2).expect("validate runs");
        let i81_warnings: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-018 I-81"))
            .collect();
        assert!(
            i81_warnings.is_empty(),
            "freshly created governance repo must have zero RFC-018 I-81 warnings: {i81_warnings:?}"
        );
    }

    #[test]
    fn scaffold_rebinding_survives_srsj_roundtrip_and_validates_clean() {
        // The rewritten views must survive serialisation, and a fresh scaffold must
        // produce zero dangling document-view container warnings (#509 validate check).
        let store = load_seed_store();
        create_governance_repository(
            &store,
            CreateGovernanceRepositoryInput {
                namespace: Some("com.example.rebind".to_string()),
                title: "Rebind Roundtrip".to_string(),
                purpose: None,
                repository_id: Some("rebind-roundtrip-id".to_string()),
            },
        )
        .expect("create succeeds");

        let srsj = store.to_srsj_string().expect("to_srsj_string");
        let store2 = JsonStore::from_srsj(&srsj).expect("re-parse");
        let report = crate::validation::validate_repository(&store2).expect("validate runs");
        let dangling: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("references containerId"))
            .collect();
        assert!(
            dangling.is_empty(),
            "fresh repo-create must not ship dangling document-view container refs: {dangling:?}"
        );
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
        // installedAt must be set on the typed upstream_package field
        let installed_at = manifest
            .upstream_package
            .as_ref()
            .map(|up| up.installed_at.as_str());
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
