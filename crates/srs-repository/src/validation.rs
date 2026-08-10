use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde_json::Value;
use srs_core::types::blueprint::{Blueprint, BlueprintDiagnosticSeverity};
use srs_core::types::lifecycle::RelationDirection;
use srs_core::types::protocol::{Protocol, ProtocolDiagnosticSeverity};
use srs_core::types::record::Record;
use srs_core::types::relation::RelationsCollection;
use srs_core::types::source_document_meta::SourceDocumentMeta;
use srs_core::types::source_reference::{SourceRole, SourceType};
use srs_core::validation::blueprint::validate_blueprint;
use srs_core::validation::lifecycle::{
    validate_lifecycle, validate_type_lifecycle_v9, LifecycleDiagnosticSeverity,
};
use srs_core::validation::protocol::validate_protocol;
use srs_core::validation::record::validate_record;
use srs_core::validation::record_type::{cross_field_type_map, validate_cross_field_rules};
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use srs_schema::{SchemaRegistry, NOTE_SCHEMA_ID, RECORD_SCHEMA_ID};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    /// Relative path within the repository that this diagnostic applies to.
    /// Serialized as "path" for JSON backward compatibility.
    #[serde(rename = "path")]
    pub relative_path: String,
    pub schema_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationSummary {
    pub checked: usize,
    pub errors: usize,
    pub warnings: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryValidationReport {
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub summary: ValidationSummary,
}

impl RepositoryValidationReport {
    pub fn is_ok(&self) -> bool {
        self.summary.errors == 0
    }
}

/// Shared namespace for the com.semanticops.spec type family and its fields.
/// Used for both type-dispatch and field-namespace lookups in the invariant uniqueness check.
const SPEC_NAMESPACE: &str = "com.semanticops.spec";
const SPEC_INVARIANT_TYPE_NAME: &str = "invariant";
const SPEC_INVARIANT_NUMBER_FIELD_NAME: &str = "invariant_number";

/// RFC-017 Change B/E: optional com.semanticops.base package with attachment_policy.
/// Field lookup is runtime-resolved via package.find_field to avoid UUID hardcoding.
const BASE_NAMESPACE: &str = "com.semanticops.base";
const BASE_REPO_SETTINGS_TYPE_NAME: &str = "repo_settings";
const BASE_ALLOWED_MIME_TYPES_FIELD: &str = "allowed_mime_types";
const BASE_MAX_PER_FILE_BYTES_FIELD: &str = "max_per_file_bytes";
const BASE_MAX_DOC_BYTES_FIELD: &str = "max_doc_bytes";
const BASE_MAX_TOTAL_BYTES_FIELD: &str = "max_total_bytes";

/// Validate an entire repository via the storage trait.
///
/// I/O errors and malformed JSON are returned as `Err(RepositoryError)`.
/// Schema violations are returned as diagnostics inside the report.
/// RFC-033 [R6] — compare the repository's `dataModelRevision` stamp against the
/// generation this build writes, and say something actionable either way.
fn data_model_revision_diagnostics(manifest_value: &Value) -> Vec<ValidationDiagnostic> {
    use crate::field_type_migration_service::{
        CURRENT_DATA_MODEL_REVISION, DATA_MODEL_REVISION_KEY,
    };
    let declared = manifest_value
        .get(DATA_MODEL_REVISION_KEY)
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if declared == CURRENT_DATA_MODEL_REVISION {
        return Vec::new();
    }
    let (severity, message) = if declared < CURRENT_DATA_MODEL_REVISION {
        (
            DiagnosticSeverity::Warning,
            format!(
                "this repository is at data-model revision {declared}; this build writes \
                 revision {CURRENT_DATA_MODEL_REVISION}. Definitions are being read through the \
                 compatibility path — run `srs repo apply-migration --id field-type` to persist them \
                 in the current model."
            ),
        )
    } else {
        (
            DiagnosticSeverity::Error,
            format!(
                "this repository is at data-model revision {declared}, which is newer than this \
                 build supports (revision {CURRENT_DATA_MODEL_REVISION}). Upgrade the `srs` \
                 binary — reading it with this build may silently drop newer definition content."
            ),
        )
    };
    vec![ValidationDiagnostic {
        severity,
        relative_path: "manifest.json".to_string(),
        schema_id: None,
        message,
    }]
}

pub fn validate_repository(
    store: &dyn RepositoryStore,
) -> Result<RepositoryValidationReport, RepositoryError> {
    let reg = SchemaRegistry::global();
    let mut diagnostics: Vec<ValidationDiagnostic> = Vec::new();
    let mut checked = 0usize;
    let mut package_for_tier2: Option<Option<crate::package::Package>> = None;
    // RFC-022: relations loaded lazily for the at-rest requiresRelation check.
    // Outer None = not loaded yet; inner None = load failed (check is skipped —
    // a corrupt relations file is reported by relation validation, not here).
    let mut relations_for_rfc022: Option<Option<Vec<crate::relation_service::RelationSummary>>> =
        None;
    // Tracks invariant numbers for the com.semanticops.spec/invariant uniqueness check.
    // Key: invariant-number string (e.g. "I-80"); value: list of (path, instance_id).
    let mut invariant_number_occurrences: HashMap<String, Vec<(String, String)>> = HashMap::new();
    // RFC-017 Change E: collects (path, record) tuples for post-loop attachment_policy check.
    let mut policy_records: Vec<(String, Record)> = Vec::new();
    // RFC-039 [R14]: reference-mode values to verify once every instance's type
    // is known. (path, record_id, key, target, expected_type, expected_version)
    let mut reference_values: Vec<(String, String, String, String, String, u32)> = Vec::new();
    // instance_id -> (type_id, type_version) for every Tier-2 record seen.
    let mut instance_types: HashMap<String, (String, u32)> = HashMap::new();

    // --- Validate root manifest.json ---
    let manifest_raw = store.load_text_file("manifest.json").map_err(|e| match e {
        RepositoryError::Io { path, source } => RepositoryError::Io { path, source },
        RepositoryError::NotFound { path } => RepositoryError::ManifestMissing { path },
        other => other,
    })?;
    let manifest_value: Value =
        serde_json::from_str(&manifest_raw).map_err(|e| RepositoryError::ManifestParse {
            path: std::path::PathBuf::from("manifest.json"),
            source: e,
        })?;

    // manifest.json is validated but not counted in `checked` — `checked` tracks only
    // instanceIndex entries so that summary.checked agrees with repo map's total_instances.
    if let Some(report) = validate_value_against_schema(
        &manifest_value,
        "manifest.json",
        srs_schema::MANIFEST_SCHEMA_ID,
        reg,
    ) {
        diagnostics.extend(report);
    }

    // RFC-033 [R6] / #265 — the data-model generation gate. A repository at a
    // *lower* revision than this build still loads (definitions are upgraded in
    // memory), so this is a warning naming the migration, not an error. A
    // repository at a *higher* revision was written by a newer build and is the
    // case where "failed to load" would otherwise be the only signal — RFC-033's
    // stated motivation for the stamp.
    diagnostics.extend(data_model_revision_diagnostics(&manifest_value));

    // RFC-039 [R15]: a revision >= 2 manifest must not declare the retired
    // extensions — a declaration implies constructs [R7] rejects.
    {
        let declared_revision = manifest_value
            .get(crate::field_type_migration_service::DATA_MODEL_REVISION_KEY)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let declared_exts: Vec<String> = manifest_value
            .get("declaredExtensions")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        for err in srs_core::validation::revision_guard::check_declared_extensions(
            &declared_exts,
            declared_revision,
        ) {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: "manifest.json".to_string(),
                schema_id: None,
                message: err.to_string(),
            });
        }
    }

    // --- Load manifest + one catalog snapshot for the whole validation run ---
    let manifest = store.load_manifest()?;
    // RFC-038 [R1]/[R24]: `repo validate` is the one caller that must report every
    // diagnostic rather than failing fatally, so this uses `catalog::build` (not
    // `build_checked`) and folds the catalog's own diagnostics into the report.
    let cat = crate::catalog::build(store)?;
    for d in &cat.diagnostics {
        diagnostics.push(ValidationDiagnostic {
            severity: d.severity,
            relative_path: d.locators.join(", "),
            schema_id: None,
            message: format!("{}: {}", d.code, d.message),
        });
    }

    // --- RFC-013 root container invariants (I-79, I-80, I-81, I-82) ---
    // When manifest.container is absent the schema validator above already fires a
    // "missing required field" error; I-79 below is the invariant-level companion.
    match manifest.container.as_ref() {
        None => {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: "manifest.json".to_string(),
                schema_id: None,
                message:
                    "RFC-013 I-79: manifest.container is absent; every SRS repository must declare a root container"
                        .to_string(),
            });
        }
        Some(root) => {
            // Same resolution as the navigation service: prefer the materialised
            // container, fall back to the manifest.container embed (embed-only roots
            // are valid — the embed is the canonical repository-identity source).
            let full_container_opt: Option<srs_core::types::container::Container> =
                match crate::container_service::resolve_root_container(store, &manifest) {
                    Ok(c) => c,
                    Err(e) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: "manifest.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "root container '{}' could not be loaded: {}",
                                root.container_id, e
                            ),
                        });
                        None
                    }
                };

            if let Some(ref full_container) = full_container_opt {
                // Structural checks (UUID format, title non-empty)
                if let Err(e) = srs_core::validation::container::validate_container(full_container)
                {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "manifest.json".to_string(),
                        schema_id: None,
                        message: format!("root container '{}': {}", root.container_id, e),
                    });
                }

                // Self-membership integrity (containerId must not be its own member/root)
                if full_container
                    .member_instance_ids
                    .as_ref()
                    .is_some_and(|ids| ids.iter().any(|id| id == &full_container.container_id))
                {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "manifest.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "root container '{}': containerId must not appear in memberInstanceIds",
                            root.container_id
                        ),
                    });
                }
                if full_container
                    .root_instance_ids
                    .as_ref()
                    .is_some_and(|ids| ids.iter().any(|id| id == &full_container.container_id))
                {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "manifest.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "root container '{}': containerId must not appear in rootInstanceIds",
                            root.container_id
                        ),
                    });
                }

                // I-80: memberInstanceIds and rootInstanceIds must all be in the instance set.
                // Uses the catalog snapshot already built above (RFC-038: no instanceIndex).
                let known_ids: HashSet<&str> =
                    cat.instances.iter().map(|e| e.id.as_str()).collect();
                if let Some(ref ids) = full_container.member_instance_ids {
                    for id in ids {
                        if !known_ids.contains(id.as_str()) {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Error,
                                relative_path: "manifest.json".to_string(),
                                schema_id: None,
                                message: format!(
                                    "RFC-013 I-80: memberInstanceId '{}' not found in the instance set",
                                    id
                                ),
                            });
                        }
                    }
                }
                if let Some(ref ids) = full_container.root_instance_ids {
                    for id in ids {
                        if !known_ids.contains(id.as_str()) {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Error,
                                relative_path: "manifest.json".to_string(),
                                schema_id: None,
                                message: format!(
                                    "RFC-013 I-80: rootInstanceId '{}' not found in the instance set",
                                    id
                                ),
                            });
                        }
                    }
                }

                // I-81: identityInstanceId must be in rootInstanceIds or memberInstanceIds
                if let Some(ref identity_id) = root.identity_instance_id {
                    let in_roots = full_container
                        .root_instance_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(identity_id));
                    let in_members = full_container
                        .member_instance_ids
                        .as_ref()
                        .is_some_and(|ids| ids.contains(identity_id));
                    if !in_roots && !in_members {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: "manifest.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "RFC-013 I-81: identityInstanceId '{}' is not in rootInstanceIds or memberInstanceIds of the root container",
                                identity_id
                            ),
                        });
                    }
                }

                // I-82: every non-identity member should root a container (warning; suppressed
                // when the catalog's container set has no file-backed containers).
                // RFC-013 I-80/R2 as amended by RFC-038 [R25]: membership resolves against the
                // catalog's container set, not `manifest.containerIndex` (retired, Change K).
                {
                    let file_backed_container_ids: Vec<&str> = cat
                        .containers
                        .iter()
                        .filter(|e| {
                            e.locator.as_deref() != Some(crate::catalog::ROOT_CONTAINER_LOCATOR)
                        })
                        .map(|e| e.id.as_str())
                        .collect();
                    if !file_backed_container_ids.is_empty() {
                        // BTreeSet for deterministic iteration order (ADR-017).
                        let union_members: BTreeSet<&str> = full_container
                            .member_instance_ids
                            .as_deref()
                            .unwrap_or(&[])
                            .iter()
                            .map(String::as_str)
                            .chain(
                                full_container
                                    .root_instance_ids
                                    .as_deref()
                                    .unwrap_or(&[])
                                    .iter()
                                    .map(String::as_str),
                            )
                            .collect();
                        if !union_members.is_empty() {
                            let mut section_container_roots: HashSet<String> = HashSet::new();
                            for container_id in file_backed_container_ids {
                                if let Ok(c) = store.load_container(container_id) {
                                    if let Some(ref roots) = c.root_instance_ids {
                                        section_container_roots.extend(roots.iter().cloned());
                                    }
                                }
                            }
                            let identity_id = root.identity_instance_id.as_deref().unwrap_or("");
                            for member_id in &union_members {
                                if *member_id == identity_id {
                                    continue;
                                }
                                if !section_container_roots.contains(*member_id) {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Warning,
                                        relative_path: "manifest.json".to_string(),
                                        schema_id: None,
                                        message: format!(
                                            "RFC-013 I-82: root container member '{}' is not the root of any container in the container set",
                                            member_id
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // RFC-018 I-81 extension: identityInstanceId MUST resolve to a Tier-2
            // com.semanticops.core/purpose Record.
            // - Tier-0 Note: Warning (transitional grace while migration tooling is absent)
            // - Tier-2 wrong-type: Warning (migration-period grace; migration tooling tracks #426)
            // - Other tiers: Warning (unexpected, should not occur in valid SRS repos)
            // Runs independently of full_container availability — only needs the catalog.
            if let Some(ref identity_id) = root.identity_instance_id {
                if let Some(cat_entry) = cat.instances.iter().find(|e| e.id == *identity_id) {
                    if cat_entry.tier == Some(0) {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: "manifest.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "RFC-018 I-81: identityInstanceId '{}' resolves to a Tier-0 Note; \
                                 must be migrated to a com.semanticops.core/purpose Record",
                                identity_id
                            ),
                        });
                    } else if cat_entry.tier == Some(2) {
                        match store
                            .load_instance_json(cat_entry.locator.as_deref().unwrap_or_default())
                        {
                            Ok(val) => {
                                let type_ns = val
                                    .get("typeNamespace")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let type_name =
                                    val.get("typeName").and_then(|v| v.as_str()).unwrap_or("");
                                if !(type_ns == "com.semanticops.core" && type_name == "purpose") {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Warning,
                                        relative_path: "manifest.json".to_string(),
                                        schema_id: None,
                                        message: format!(
                                            "RFC-018 I-81: identityInstanceId '{}' resolves to \
                                             type '{}/{}' but must be com.semanticops.core/purpose",
                                            identity_id, type_ns, type_name
                                        ),
                                    });
                                }
                            }
                            Err(e) => {
                                diagnostics.push(ValidationDiagnostic {
                                    severity: DiagnosticSeverity::Warning,
                                    relative_path: "manifest.json".to_string(),
                                    schema_id: None,
                                    message: format!(
                                        "RFC-018 I-81: could not load identity instance '{}' \
                                         to verify type: {}",
                                        identity_id, e
                                    ),
                                });
                            }
                        }
                    } else {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: "manifest.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "RFC-018 I-81: identityInstanceId '{}' resolves to an \
                                 unexpected tier {}; must be a Tier-2 com.semanticops.core/purpose Record",
                                identity_id,
                                cat_entry.tier.unwrap_or(2)
                            ),
                        });
                    }
                }
                // not found in the instance set: the membership check above already emits an Error
            }
        }
    }

    // --- RFC-017 [R2]/[R12] as amended by RFC-038 [R25]: pre-compute attaches
    // doc-id → content-present map from the catalog's source-document set
    // (sidecar discovery), not `manifest.sourceDocumentIndex` (retired, Change K).
    // Built once before the instance loop; used per-instance to check sourceRefs.
    let src_docs_base_for_attaches = manifest
        .source_documents_path
        .as_deref()
        .unwrap_or("source-documents");
    let attaches_doc_id_map: HashMap<String, bool> = cat
        .source_documents
        .iter()
        .filter_map(|entry| {
            let locator = entry.locator.as_deref()?;
            let sidecar = store.load_instance_json(locator).ok()?;
            let content_path = sidecar.get("contentPath").and_then(|v| v.as_str())?;
            let content_repo_rel = format!("{src_docs_base_for_attaches}/{content_path}");
            let content_present = !matches!(
                store.file_byte_len(&content_repo_rel),
                Err(ref e) if e.is_not_found()
            );
            Some((entry.id.clone(), content_present))
        })
        .collect();

    // --- Validate each instance in the catalog's instance set ---
    for entry in &cat.instances {
        let rel_path = entry.locator.clone().unwrap_or_default();
        let tier = entry.tier.unwrap_or(2);

        let value = match store.load_instance_json(&rel_path) {
            Ok(v) => v,
            Err(e) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: rel_path,
                    schema_id: None,
                    message: format!("I/O error: {e}"),
                });
                continue;
            }
        };

        checked += 1;

        // Determine expected schema from tier
        let tier_schema_id = tier_to_schema_id(tier);

        // Check declared $schema vs tier
        let declared = value.get("$schema").and_then(|v| v.as_str());
        if let (Some(tier_id), Some(decl)) = (tier_schema_id, declared) {
            if tier_id != decl {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: rel_path.clone(),
                    schema_id: Some(decl.to_string()),
                    message: format!(
                        "manifest tier {} expects schema {tier_id} but file declares {decl}",
                        tier
                    ),
                });
            }
        }

        // Validate against declared schema if known, else fall back to tier schema
        let schema_id_to_validate = declared
            .filter(|id| srs_schema::ALL_SCHEMA_IDS.contains(id))
            .or(tier_schema_id);

        if let Some(schema_id) = schema_id_to_validate {
            if let Err(e) = reg.validate_by_id(schema_id, &value) {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: rel_path.clone(),
                    schema_id: Some(schema_id.to_string()),
                    message: e.to_string(),
                });
            }
        } else {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Warning,
                relative_path: rel_path.clone(),
                schema_id: None,
                message: "no known $schema declared and tier has no default schema".to_string(),
            });
        }

        if tier == 2 {
            // RFC-039 [R7]: groupValues in a revision >= 2 instance is a removed
            // construct — named diagnostic from the raw document (the typed
            // loader's flatten would silently absorb it).
            {
                let declared_revision = manifest_value
                    .get(crate::field_type_migration_service::DATA_MODEL_REVISION_KEY)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                for err in srs_core::validation::revision_guard::check_record_document(
                    &value,
                    declared_revision,
                    &rel_path,
                ) {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: rel_path.clone(),
                        schema_id: None,
                        message: err.to_string(),
                    });
                }
            }
            if package_for_tier2.is_none() {
                let pkg = store.load_package().ok();
                package_for_tier2 = Some(pkg);
            }
            match package_for_tier2.as_ref().and_then(|p| p.as_ref()) {
                Some(package) => match serde_json::from_value::<Record>(value.clone()) {
                    Ok(record) => {
                        let rt_opt = package.resolve_type(&record.type_id, record.type_version);

                        if let Some(record_type) = rt_opt {
                            match package.resolved_effective_fields(record_type) {
                                Ok(effective_fields) => {
                                    if let Err(err) = validate_record(
                                        &record,
                                        record_type,
                                        &effective_fields,
                                        package,
                                    ) {
                                        diagnostics.push(ValidationDiagnostic {
                                            severity: DiagnosticSeverity::Error,
                                            relative_path: rel_path.clone(),
                                            schema_id: None,
                                            message: err.to_string(),
                                        });
                                    }
                                }
                                Err(err) => {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Error,
                                        relative_path: rel_path.clone(),
                                        schema_id: None,
                                        message: format!("type inheritance error: {err}"),
                                    });
                                }
                            }
                        }

                        // Tier-graduated tag resolution enforcement (C4):
                        // Only runs when at least one Vocabulary is declared in the package.
                        // Notes (tier 0) are exempt — only tier-2 Records enforce this.
                        if !package.vocabularies.is_empty() {
                            if let Some(tags) = &record.tags {
                                let any_open = package.vocabularies.iter().any(|v| {
                                    matches!(
                                        v.mode,
                                        srs_core::types::vocabulary::VocabularyMode::Open
                                    )
                                });
                                for tag in tags {
                                    let resolved = package
                                        .vocabularies
                                        .iter()
                                        .any(|v| v.resolve_term_by_key(tag).is_some());
                                    if !resolved {
                                        let severity = if any_open {
                                            DiagnosticSeverity::Warning
                                        } else {
                                            DiagnosticSeverity::Error
                                        };
                                        diagnostics.push(ValidationDiagnostic {
                                            severity,
                                            relative_path: rel_path.clone(),
                                            schema_id: None,
                                            message: format!(
                                                "tag '{}' on record '{}' does not resolve to any Term key or alias in the declared vocabularies",
                                                tag, record.instance_id
                                            ),
                                        });
                                    }
                                }
                            }
                        }

                        // V8: validate record's lifecycleState against its type's lifecycle
                        if let Some(state_value) = &record.lifecycle_state {
                            if let Some(rt) = rt_opt {
                                let lc_states: Option<
                                    Vec<&srs_core::types::lifecycle::LifecycleState>,
                                > = if let Some(ref_id) = &rt.lifecycle_ref {
                                    // If lifecycle_ref doesn't resolve, skip V8 — V8 will report it
                                    package
                                        .resolve_lifecycle(ref_id)
                                        .map(|lc| lc.states.iter().collect())
                                } else {
                                    rt.lifecycle
                                        .as_ref()
                                        .map(|inline_lc| inline_lc.states.iter().collect())
                                };
                                if let Some(states) = lc_states {
                                    let valid = states.iter().any(|s| {
                                        s.key == *state_value && !s.effective_status().is_retired()
                                    });
                                    if !valid {
                                        diagnostics.push(ValidationDiagnostic {
                                            severity: DiagnosticSeverity::Error,
                                            relative_path: rel_path.clone(),
                                            schema_id: None,
                                            message: format!(
                                                "V8: record '{}' lifecycleState '{}' is not a valid state key in the resolved lifecycle",
                                                record.instance_id, state_value
                                            ),
                                        });
                                    }

                                    // RFC-022 R1/R10: a record at rest in a requiresRelation
                                    // state with no satisfying relation is a warning.
                                    if let Some(req) = states
                                        .iter()
                                        .find(|s| s.key == *state_value)
                                        .and_then(|s| s.requires_relation.as_ref())
                                    {
                                        if relations_for_rfc022.is_none() {
                                            relations_for_rfc022 = Some(
                                                crate::relation_service::list_relations(
                                                    store,
                                                    Default::default(),
                                                )
                                                .ok(),
                                            );
                                        }
                                        if let Some(Some(rels)) = &relations_for_rfc022 {
                                            let declared = req.relation_type.types();
                                            let direction = req.effective_direction();
                                            let satisfied = rels.iter().any(|r| {
                                                let anchored = match direction {
                                                    RelationDirection::Incoming => {
                                                        r.target_id == record.instance_id
                                                    }
                                                    RelationDirection::Outgoing => {
                                                        r.source_id == record.instance_id
                                                    }
                                                };
                                                anchored
                                                    && declared
                                                        .iter()
                                                        .any(|t| r.relation_type == *t)
                                            });
                                            if !satisfied {
                                                diagnostics.push(ValidationDiagnostic {
                                                    severity: DiagnosticSeverity::Warning,
                                                    relative_path: rel_path.clone(),
                                                    schema_id: None,
                                                    message: format!(
                                                        "LIFECYCLE_RELATION_UNSATISFIED: record '{}' is in state '{}' which requires a '{}' relation of type {:?} and none satisfies it (RFC-022 R1)",
                                                        record.instance_id, state_value, direction, declared
                                                    ),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // ext:cross-field-validation — evaluate CrossFieldRule[] if present
                        if let Some(rt) = rt_opt {
                            if let Some(rules) = &rt.validation_rules {
                                if !rules.is_empty() {
                                    // Built per Type, not cached per store: the
                                    // `FieldAssignment.repeatable` half of
                                    // `effective-single` is per-assignment, so
                                    // the same Field can differ between Types.
                                    let ftype_map = cross_field_type_map(&package.fields, rt);
                                    let cfr_errors =
                                        validate_cross_field_rules(&record, rules, &ftype_map);
                                    for err in cfr_errors {
                                        diagnostics.push(ValidationDiagnostic {
                                            severity: DiagnosticSeverity::Error,
                                            relative_path: rel_path.clone(),
                                            schema_id: None,
                                            message: err.to_string(),
                                        });
                                    }
                                }
                            }
                        }

                        // Collect invariant numbers for the post-loop uniqueness check.
                        // Naming-based dispatch on type_namespace/type_name is intentional:
                        // the SRS data model has no structural uniqueness marker on fields,
                        // so this check must key off the type identity. The field ID is
                        // resolved at runtime via find_field to avoid UUID drift.
                        // Values are coerced to strings because spec repos store invariant
                        // numbers as JSON numbers (e.g. 1, 2) while new records use
                        // strings ("I-1"); both representations are compared after coercion.
                        if record.type_namespace == SPEC_NAMESPACE
                            && record.type_name == SPEC_INVARIANT_TYPE_NAME
                            && package
                                .find_field(SPEC_NAMESPACE, SPEC_INVARIANT_NUMBER_FIELD_NAME)
                                .is_some()
                        {
                            {
                                if let Some(value) = record.value(SPEC_INVARIANT_NUMBER_FIELD_NAME)
                                {
                                    let num_str = match value {
                                        serde_json::Value::String(s) => Some(s.clone()),
                                        serde_json::Value::Number(n) => Some(n.to_string()),
                                        _ => None,
                                    };
                                    if let Some(key) = num_str {
                                        invariant_number_occurrences
                                            .entry(key)
                                            .or_default()
                                            .push((rel_path.clone(), record.instance_id.clone()));
                                    }
                                }
                            }
                        }

                        // RFC-017 Change E: collect repo_settings records for the post-loop
                        // RFC-017 Change B uniqueness check and attachment_policy size/MIME diagnostics.
                        if record.type_namespace == BASE_NAMESPACE
                            && record.type_name == BASE_REPO_SETTINGS_TYPE_NAME
                        {
                            policy_records.push((rel_path.clone(), record.clone()));
                        }

                        // RFC-039 [R14]: collect reference-mode values for the
                        // post-loop integrity check (targets may appear later
                        // in the index).
                        instance_types.insert(
                            record.instance_id.clone(),
                            (record.type_id.clone(), record.type_version),
                        );
                        if let Some(rt) = rt_opt {
                            if let Ok(effective) = package.resolved_effective_fields(rt) {
                                for ef in &effective {
                                    let Some(ft) = &ef.field_type else { continue };
                                    if ft.datatype != srs_core::types::field_type::Datatype::Ref
                                        || ft.effective_mode()
                                            != srs_core::types::field_type::RefMode::Reference
                                    {
                                        continue;
                                    }
                                    let Some(range) = &ft.range_type else {
                                        continue;
                                    };
                                    let Some(value) = record.value(&ef.name) else {
                                        continue;
                                    };
                                    let targets: Vec<&str> = match value {
                                        Value::String(s) => vec![s.as_str()],
                                        Value::Array(items) => {
                                            items.iter().filter_map(|v| v.as_str()).collect()
                                        }
                                        _ => Vec::new(),
                                    };
                                    for target in targets {
                                        reference_values.push((
                                            rel_path.clone(),
                                            record.instance_id.clone(),
                                            ef.name.clone(),
                                            target.to_string(),
                                            range.type_id.clone(),
                                            range.type_version,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    Err(err) => diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: rel_path.clone(),
                        schema_id: None,
                        message: format!(
                            "failed to parse tier-2 record for semantic validation: {err}"
                        ),
                    }),
                },
                None => diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: rel_path.clone(),
                    schema_id: None,
                    message: "failed to load package for tier-2 semantic validation".to_string(),
                }),
            }
        }

        // --- RFC-017 R2/R12: validate 'attaches' sourceRefs against sourceDocumentIndex ---
        if let Some(refs_array) = value.get("sourceRefs").and_then(|v| v.as_array()) {
            for ref_val in refs_array {
                let source_type = ref_val
                    .get("sourceType")
                    .and_then(|v| serde_json::from_value::<SourceType>(v.clone()).ok());
                let source_role = ref_val
                    .get("sourceRole")
                    .and_then(|v| serde_json::from_value::<SourceRole>(v.clone()).ok());
                if source_type != Some(SourceType::RepositoryDocument)
                    || source_role != Some(SourceRole::Attaches)
                {
                    continue;
                }
                let source_id = match ref_val.get("sourceId").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => continue,
                };
                match attaches_doc_id_map.get(source_id) {
                    None => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: rel_path.clone(),
                            schema_id: None,
                            message: format!(
                                "RFC-017 R2: 'attaches' sourceRef sourceId '{}' does not \
                                 resolve to any documentId in sourceDocumentIndex",
                                source_id
                            ),
                        });
                    }
                    Some(false) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: rel_path.clone(),
                            schema_id: None,
                            message: format!(
                                "RFC-017 R12: 'attaches' sourceRef sourceId '{}' content \
                                 unavailable (tombstone)",
                                source_id
                            ),
                        });
                    }
                    Some(true) => {} // resolved and content present — no diagnostic
                }
            }
        }
    }

    // --- com.semanticops.spec/invariant number uniqueness ---
    // Emit an Error for every record that participates in a duplicate invariant number.
    // The same I-NN number on two or more records is a data-integrity error: projection
    // scripts and downstream validators cannot unambiguously resolve a number to one record.
    // RFC-039 [R14]: every reference-mode value must resolve to an indexed
    // instance of the declared rangeType@typeVersion.
    for (rel_path, record_id, key, target, expected_type, expected_version) in &reference_values {
        match instance_types.get(target) {
            None => diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: rel_path.clone(),
                schema_id: None,
                message: srs_core::error::CoreError::DanglingReference {
                    key: format!("{record_id}.{key}"),
                    target: target.clone(),
                }
                .to_string(),
            }),
            Some((tid, tver)) if tid != expected_type || tver != expected_version => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: rel_path.clone(),
                    schema_id: None,
                    message: srs_core::error::CoreError::ReferenceTypeMismatch {
                        key: format!("{record_id}.{key}"),
                        target: target.clone(),
                        expected_type: expected_type.clone(),
                        expected_version: *expected_version,
                    }
                    .to_string(),
                });
            }
            Some(_) => {}
        }
    }

    let mut invariant_dup_keys: Vec<&String> = invariant_number_occurrences
        .keys()
        .filter(|k| invariant_number_occurrences[*k].len() > 1)
        .collect();
    invariant_dup_keys.sort(); // deterministic diagnostic order
    for inv_num in invariant_dup_keys {
        let occurrences = &invariant_number_occurrences[inv_num];
        let count = occurrences.len();
        for (path, _id) in occurrences {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: path.clone(),
                schema_id: None,
                message: format!(
                    "com.semanticops.spec/invariant: duplicate invariant number '{}' — \
                     {count} records share this number",
                    inv_num
                ),
            });
        }
    }

    // --- RFC-017 I-107: attachment_policy size and MIME-type diagnostics ---
    // All attachment_policy warning conditions are governed by a single spec invariant (I-107).
    // The multiple-records guard below cites RFC-017 Change B (the base-package prose rule).
    if policy_records.len() > 1 {
        // RFC-017 Change B: at most one repo_settings record. Treat policy as absent and emit
        // an Error for each copy so the author knows exactly which paths to remove.
        let count = policy_records.len();
        for (path, _record) in &policy_records {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: path.clone(),
                schema_id: None,
                message: format!(
                    "RFC-017 Change B: at most one attachment_policy (repo_settings) record \
                     may exist per repository (found {count}); policy treated as absent — \
                     remove all but one"
                ),
            });
        }
    } else if let Some((policy_path, policy_record)) = policy_records.first() {
        // Single policy record — check source documents against its limits.
        // Gracefully skip the entire check if the package could not be loaded.
        if let Some(Some(pkg)) = package_for_tier2.as_ref() {
            // Extract optional u64 limit from a named policy field.
            let get_u64 = |field_name: &str| -> Option<u64> {
                pkg.find_field(BASE_NAMESPACE, field_name)?;
                policy_record.value(field_name)?.as_u64()
            };

            let max_per_file_bytes = get_u64(BASE_MAX_PER_FILE_BYTES_FIELD);
            let max_doc_bytes = get_u64(BASE_MAX_DOC_BYTES_FIELD);
            let max_total_bytes = get_u64(BASE_MAX_TOTAL_BYTES_FIELD);

            // Extract allowed_mime_types — accepts Array, JSON-array string, or single string.
            let allowed_mime_types: Option<Vec<String>> = 'mime: {
                if pkg
                    .find_field(BASE_NAMESPACE, BASE_ALLOWED_MIME_TYPES_FIELD)
                    .is_none()
                {
                    break 'mime None;
                }
                let value = match policy_record.value(BASE_ALLOWED_MIME_TYPES_FIELD) {
                    Some(v) => v,
                    None => break 'mime None,
                };
                match value {
                    Value::Array(arr) => Some(
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect(),
                    ),
                    Value::String(s) => {
                        let trimmed = s.trim_start();
                        if trimmed.starts_with('[') {
                            match serde_json::from_str::<Vec<String>>(trimmed) {
                                Ok(mimes) => Some(mimes),
                                Err(_) => {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Warning,
                                        relative_path: policy_path.clone(),
                                        schema_id: None,
                                        message: "attachment_policy: allowed_mime_types could not be parsed; MIME-type check skipped".to_string(),
                                    });
                                    break 'mime None;
                                }
                            }
                        } else {
                            Some(vec![s.clone()])
                        }
                    }
                    _ => None,
                }
            };

            let src_docs_base = manifest
                .source_documents_path
                .as_deref()
                .unwrap_or("source-documents");

            // RFC-038 [R25]: source documents resolve via the catalog's
            // source-document set (sidecar discovery), not
            // `manifest.sourceDocumentIndex` (retired, Change K).
            let src_doc_metas: Vec<SourceDocumentMeta> = cat
                .source_documents
                .iter()
                .filter_map(|entry| {
                    let locator = entry.locator.as_deref()?;
                    let sidecar = store.load_instance_json(locator).ok()?;
                    serde_json::from_value::<SourceDocumentMeta>(sidecar).ok()
                })
                .collect();

            // Build MIME map: content_path (relative to src_docs_base) → content_type.
            let mut mime_map: HashMap<String, String> = HashMap::new();
            for meta in &src_doc_metas {
                mime_map.insert(meta.content_path.clone(), meta.content_type.clone());
            }

            let mut total_bytes: u64 = 0;

            for entry in &src_doc_metas {
                let content_repo_rel = format!("{}/{}", src_docs_base, entry.content_path);

                let size = match store.file_byte_len(&content_repo_rel) {
                    Ok(n) => n,
                    // ADR-031 tombstone: content file absent is normal; skip silently.
                    Err(ref e) if e.is_not_found() => continue,
                    Err(_) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: content_repo_rel.clone(),
                            schema_id: None,
                            message: format!(
                                "attachment_policy: could not stat '{}'; size check skipped",
                                entry.content_path
                            ),
                        });
                        continue;
                    }
                };

                // I-107: max_per_file_bytes
                if let Some(limit) = max_per_file_bytes {
                    if size > limit {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: content_repo_rel.clone(),
                            schema_id: None,
                            message: format!(
                                "RFC-017 I-107: '{}' is {size} bytes, \
                                 exceeding max_per_file_bytes limit of {limit} bytes",
                                entry.content_path
                            ),
                        });
                    }
                }

                // I-107: max_doc_bytes (per-document limit; independent of max_per_file_bytes)
                if let Some(limit) = max_doc_bytes {
                    if size > limit {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: content_repo_rel.clone(),
                            schema_id: None,
                            message: format!(
                                "RFC-017 I-107: '{}' is {size} bytes, \
                                 exceeding max_doc_bytes limit of {limit} bytes",
                                entry.content_path
                            ),
                        });
                    }
                }

                total_bytes = total_bytes.saturating_add(size);

                // I-107: allowed_mime_types (exact case-sensitive match)
                if let Some(ref allowed) = allowed_mime_types {
                    if let Some(actual_mime) = mime_map.get(&entry.content_path) {
                        if !allowed.contains(actual_mime) {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Warning,
                                relative_path: content_repo_rel.clone(),
                                schema_id: None,
                                message: format!(
                                    "RFC-017 I-107: '{}' has MIME type '{}' which is not in \
                                     allowed_mime_types {:?}",
                                    entry.content_path, actual_mime, allowed
                                ),
                            });
                        }
                    }
                }
            }

            // I-107: max_total_bytes (aggregate)
            if let Some(limit) = max_total_bytes {
                if total_bytes > limit {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Warning,
                        relative_path: src_docs_base.to_string(),
                        schema_id: None,
                        message: format!(
                            "RFC-017 I-107: aggregate source-document bytes ({total_bytes}) \
                             exceed max_total_bytes limit of {limit} bytes"
                        ),
                    });
                }
            }
        }
        // If package_for_tier2 is Some(None) the package load already failed and an error
        // was emitted during the main loop; skip the policy check silently.
    }

    // --- Inv 43: warn about cross-package base type references ---
    if let Some(Some(pkg)) = &package_for_tier2 {
        for rt in pkg.record_types() {
            if let Some(base_id) = &rt.extends_type_id {
                let base_version = rt.extends_type_version.unwrap_or(1);
                if pkg.resolve_type(base_id, base_version).is_none() {
                    // The base type is not local. Check whether the specializing type's
                    // namespace (a proxy for its package) is covered by any dependency_refs entry.
                    // Cross-package base type resolution is V2 work (RFC-003); for now we warn
                    // only when no dependencyRefs entry matches the specializing type's namespace,
                    // which indicates the package has not declared its external dependency at all.
                    let covered_by_dep = pkg.dependency_refs.iter().any(|dep| {
                        dep.namespace == rt.namespace
                            || pkg
                                .record_types()
                                .iter()
                                .any(|t| &t.id == base_id && dep.namespace == t.namespace)
                    });
                    if !covered_by_dep {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Warning,
                            relative_path: "package/package.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "ext:type-inheritance (Inv 43): type '{}' extends base type '{}@{}' which is not in this package; add a dependencyRefs entry for the external package",
                                rt.id, base_id, base_version
                            ),
                        });
                    }
                }
            }
        }
    }

    // --- RFC-006 vocabulary invariants V2, V5, V7, V9; RFC-020 Rule [N+33] ---
    // Use the package already loaded for tier-2 validation if available; otherwise try a fresh
    // load so that these invariants fire even in note-only / type-only repositories. Rule
    // [N+33] runs independent of whether any Tier-2 Record of the type exists — a pure
    // Type-level invariant, matching Inv 43's shape above (not Inv 41's lazily record-triggered
    // shape) — so it shares this same resolution branch rather than the narrower Inv-43 guard.
    if let Some(Some(ref pkg)) = package_for_tier2 {
        validate_vocabulary_invariants(pkg, &mut diagnostics);
        validate_identity_field_invariants(pkg, &mut diagnostics);
        validate_field_type_conformance(pkg, &mut diagnostics);
        validate_title_field_id_eligibility(store, pkg, &mut diagnostics);
        validate_cross_field_rule_configuration(pkg, &mut diagnostics);
    } else if package_for_tier2.is_none() {
        // Only fresh-load when no tier-2 records were processed (note-only repo).
        // When package_for_tier2 is Some(None), the load already failed; don't retry.
        if let Ok(pkg) = store.load_package() {
            validate_vocabulary_invariants(&pkg, &mut diagnostics);
            validate_identity_field_invariants(&pkg, &mut diagnostics);
            validate_field_type_conformance(&pkg, &mut diagnostics);
            validate_title_field_id_eligibility(store, &pkg, &mut diagnostics);
            validate_cross_field_rule_configuration(&pkg, &mut diagnostics);
        }
    }

    // --- Validate package/package.json if present ---
    // package.json is infrastructure, not an instance — not counted in `checked`.
    if let Ok(pkg_value) = store.load_instance_json("package/package.json") {
        if let Some(report) = validate_value_against_schema(
            &pkg_value,
            "package/package.json",
            srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
            reg,
        ) {
            diagnostics.extend(report);
        }

        // RFC-039 [R7]: at dataModelRevision >= 2 a Type definition carrying a
        // removed construct (fieldGroups; the assignment trio) is rejected with
        // a named diagnostic — the typed loader's serde would silently absorb
        // the stray keys, so the check runs on the raw documents here.
        let declared_revision = manifest_value
            .get(crate::field_type_migration_service::DATA_MODEL_REVISION_KEY)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if let Some(type_paths) = pkg_value.get("types").and_then(|v| v.as_array()) {
            for rel in type_paths.iter().filter_map(|v| v.as_str()) {
                let path = format!("package/{rel}");
                if let Ok(type_doc) = store.load_instance_json(&path) {
                    for err in srs_core::validation::revision_guard::check_type_document(
                        &type_doc,
                        declared_revision,
                        &path,
                    ) {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: path.clone(),
                            schema_id: None,
                            message: err.to_string(),
                        });
                    }
                }
            }
        }
    }

    // --- Validate the authoritative relations file against E1-E4 ---
    // The relations file is infrastructure, not an instance — not counted in `checked`.
    // Resolve it through the same candidate order the relation service writes through
    // (manifest relationsPath → relations-collection.json → relations.json), read via
    // load_relations_json so at-rest validation covers whichever file is authoritative
    // across every store — including the JsonStore behind the WASM/srs-web path (#548).
    let relations_source = match crate::relation_service::resolve_relations_source(store) {
        Ok(source) => source,
        Err(err) => {
            // Present-but-unreadable/malformed relations file: surface as a diagnostic
            // rather than aborting the whole validation run. resolve_relations_source
            // re-attaches the relative candidate path to the Serialize error so the
            // diagnostic points at the actual file.
            let relative_path = match &err {
                RepositoryError::Serialize { path, .. } => path.display().to_string(),
                _ => "relations".to_string(),
            };
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path,
                schema_id: None,
                message: format!("failed to read relations file: {err}"),
            });
            None
        }
    };
    // Collection relation ids + path, captured for the [R12] duplicate check
    // against standalone relation objects below.
    let mut collection_relation_ids: HashSet<String> = HashSet::new();
    let mut collection_relations_path: Option<String> = None;
    if let Some((relations_path, relations_value)) = relations_source {
        // Schema-validate the (already-parsed) file first
        if let Some(schema_diags) = validate_value_against_schema(
            &relations_value,
            &relations_path,
            srs_schema::RELATIONS_COLLECTION_SCHEMA_ID,
            reg,
        ) {
            diagnostics.extend(schema_diags);
        }

        let pkg = match store.load_package() {
            Ok(pkg) => pkg,
            Err(err) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!("failed to load package for relation validation: {err}"),
                });
                let errors = diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Error)
                    .count();
                let warnings = diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Warning)
                    .count();
                return Ok(RepositoryValidationReport {
                    diagnostics,
                    summary: ValidationSummary {
                        checked,
                        errors,
                        warnings,
                    },
                });
            }
        };

        // Build known instance IDs + the semanticObjectType map from the catalog
        // snapshot already built above, via the shared helper so `repo validate`
        // and `create_relation` enforce E1/E4 over identical inputs (#556).
        let (known_instance_ids, instance_semantic_types) =
            crate::writer::known_instances_and_semantic_types(store, &cat);

        let coll: RelationsCollection = match serde_json::from_value(relations_value) {
            Ok(c) => c,
            Err(e) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: relations_path.clone(),
                    schema_id: None,
                    message: format!("malformed relations collection: {e}"),
                });
                let errors = diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Error)
                    .count();
                let warnings = diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Warning)
                    .count();
                return Ok(RepositoryValidationReport {
                    diagnostics,
                    summary: ValidationSummary {
                        checked,
                        errors,
                        warnings,
                    },
                });
            }
        };

        let ctx = RelationValidationContext {
            definitions: &pkg.relation_type_definitions,
            known_instance_ids: &known_instance_ids,
            instance_semantic_types: &instance_semantic_types,
        };
        for relation in &coll.relations {
            if let Err(errs) = validate_relation(relation, &ctx, false) {
                for e in errs {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: relations_path.clone(),
                        schema_id: None,
                        message: e.message,
                    });
                }
            }
        }
        collection_relation_ids = coll
            .relations
            .iter()
            .map(|r| r.relation_id.clone())
            .collect();
        collection_relations_path = Some(relations_path);
    }

    // --- Standalone relation objects (relations/<relationId>.json — RFC-038 Change E) ---
    // Transitional dual read: validated alongside the collection file until the
    // RFC-038 Phase-6 flip retires the collection form.
    match store.list_relations() {
        Ok(standalone) if !standalone.is_empty() => {
            match store.load_package() {
                Ok(pkg) => {
                    let (known_instance_ids, instance_semantic_types) =
                        crate::writer::known_instances_and_semantic_types(store, &cat);
                    let ctx = RelationValidationContext {
                        definitions: &pkg.relation_type_definitions,
                        known_instance_ids: &known_instance_ids,
                        instance_semantic_types: &instance_semantic_types,
                    };
                    for relation in &standalone {
                        let rel_path = format!("relations/{}.json", relation.relation_id);
                        if let Err(errs) = validate_relation(relation, &ctx, false) {
                            for e in errs {
                                diagnostics.push(ValidationDiagnostic {
                                    severity: DiagnosticSeverity::Error,
                                    relative_path: rel_path.clone(),
                                    schema_id: None,
                                    message: e.message,
                                });
                            }
                        }
                        // [R12]: the same relationId in the collection and as a
                        // standalone object — name both locators.
                        if collection_relation_ids.contains(&relation.relation_id) {
                            diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: rel_path.clone(),
                            schema_id: None,
                            message: format!(
                                "duplicate relationId '{}': found at {} and in {} (RFC-038 [R12])",
                                relation.relation_id,
                                rel_path,
                                collection_relations_path.as_deref().unwrap_or("the relations collection"),
                            ),
                        });
                        }
                    }
                }
                Err(err) => {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "package/package.json".to_string(),
                        schema_id: None,
                        message: format!("failed to load package for relation validation: {err}"),
                    });
                }
            }
        }
        Ok(_) => {}
        Err(err) => {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: "relations".to_string(),
                schema_id: None,
                message: format!("failed to read standalone relation objects: {err}"),
            });
        }
    }

    // --- RFC-009 root-type anchor diagnostics (I-63, I-64) ---
    // Both are advisory (Warning): neither invalidates the repository. See RFC-009.
    if let Ok(pkg) = store.load_package() {
        // I-63: each DocumentView.rootTypeRefs entry MUST resolve to a Type in the package.
        // An unresolved entry is reported and "will not be used for Container matching".
        // Read views from the already-loaded `pkg` (avoids a second package load).
        {
            for dv in &pkg.document_views {
                if let Some(refs) = &dv.root_type_refs {
                    for r in refs {
                        if pkg.resolve_type(&r.type_id, r.type_version).is_none() {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Warning,
                                relative_path: "package/package.json".to_string(),
                                schema_id: None,
                                message: format!(
                                    "RFC-009 I-63: documentView '{}' rootTypeRefs entry '{}@{}' does not resolve to a Type in the package; it will not be used for Container matching",
                                    dv.id, r.type_id, r.type_version
                                ),
                            });
                        }
                    }
                }
            }
        }

        // Dangling document-view container references (#509): a section whose source
        // names a containerId that does not resolve renders as empty at render time.
        // Advisory (Warning) — the repository stays valid, but the broken reference
        // should be visible before render time.
        {
            let mut checked_ids: HashMap<String, bool> = HashMap::new();
            let mut resolves = |id: &str| -> bool {
                if let Some(known) = checked_ids.get(id) {
                    return *known;
                }
                let ok = crate::container_service::get_container(store, id).is_ok();
                checked_ids.insert(id.to_string(), ok);
                ok
            };
            for dv in &pkg.document_views {
                for section in &dv.sections {
                    let referenced: Vec<&str> = match &section.source {
                        srs_core::types::view::SectionSource::ContainerSubset {
                            container_id,
                            ..
                        } => vec![container_id.as_str()],
                        srs_core::types::view::SectionSource::TypeQuery {
                            container_ids, ..
                        } => container_ids
                            .as_deref()
                            .unwrap_or(&[])
                            .iter()
                            .map(|s| s.as_str())
                            .collect(),
                        _ => Vec::new(),
                    };
                    for cid in referenced {
                        if !resolves(cid) {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Warning,
                                relative_path: "package/package.json".to_string(),
                                schema_id: None,
                                message: format!(
                                    "documentView '{}' section '{}' references containerId '{}' which does not resolve to a Container in this repository; the section will render as empty",
                                    dv.id, section.section_id, cid
                                ),
                            });
                        }
                    }
                }
            }
        }

        // I-027-2a: relationsPresentation.include entries must not duplicate relationType values
        // and must each resolve to a non-retired RTD. Both conditions are advisory (Warning).
        {
            use srs_core::types::relation_type_definition::RelationTypeStatus;
            for dv in &pkg.document_views {
                for section in &dv.sections {
                    if let Some(rp) = &section.relations_presentation {
                        let mut seen_types: HashSet<&str> = HashSet::new();
                        for entry in &rp.include {
                            if !seen_types.insert(entry.relation_type.as_str()) {
                                diagnostics.push(ValidationDiagnostic {
                                    severity: DiagnosticSeverity::Warning,
                                    relative_path: "package/package.json".to_string(),
                                    schema_id: None,
                                    message: format!(
                                        "I-027-2a: documentView '{}' section '{}' relationsPresentation.include has duplicate relationType '{}'; the duplicate entry will be skipped at render time",
                                        dv.id, section.section_id, entry.relation_type
                                    ),
                                });
                            }
                            match pkg.resolve_relation_type(&entry.relation_type) {
                                None => {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Warning,
                                        relative_path: "package/package.json".to_string(),
                                        schema_id: None,
                                        message: format!(
                                            "I-027-2a: documentView '{}' section '{}' relationsPresentation.include entry '{}' does not resolve to a relation type in the package; the entry will be skipped at render time",
                                            dv.id, section.section_id, entry.relation_type
                                        ),
                                    });
                                }
                                Some(rtd) if rtd.status == Some(RelationTypeStatus::Retired) => {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Warning,
                                        relative_path: "package/package.json".to_string(),
                                        schema_id: None,
                                        message: format!(
                                            "I-027-2a: documentView '{}' section '{}' relationsPresentation.include entry '{}' resolves to a retired relation type; the entry will be skipped at render time",
                                            dv.id, section.section_id, entry.relation_type
                                        ),
                                    });
                                }
                                Some(_) => {}
                            }
                        }
                    }
                }
            }
        }

        // I-64: when a Container has rootInstanceIds and a containerType, containerType SHOULD
        // equal the resolved root Type's bare `name`. A mismatch is a stale hint, not an error.
        // Edge cases (unloadable root Record, unresolved Type) skip the check — never error here.
        let id_to_path: HashMap<String, String> = cat
            .instances
            .iter()
            .filter_map(|e| Some((e.id.clone(), e.locator.clone()?)))
            .collect();
        if let Ok(container_summaries) = store.list_container_summaries() {
            for (container_id, _title) in container_summaries {
                let container = match store.load_container(&container_id) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let (Some(ctype), Some(roots)) =
                    (&container.container_type, &container.root_instance_ids)
                else {
                    continue;
                };
                let Some(first_root) = roots.first() else {
                    continue;
                };
                let Some(path) = id_to_path.get(first_root) else {
                    continue;
                };
                let Ok(val) = store.load_instance_json(path) else {
                    continue;
                };
                let (Some(type_id), Some(type_version)) = (
                    val.get("typeId").and_then(|v| v.as_str()),
                    val.get("typeVersion").and_then(|v| v.as_u64()),
                ) else {
                    continue;
                };
                let Some(rt) = pkg.resolve_type(type_id, type_version as u32) else {
                    continue;
                };
                if ctype != &rt.name {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Warning,
                        relative_path: format!("container {container_id}"),
                        schema_id: None,
                        message: format!(
                            "RFC-009 I-64: container '{}' containerType '{}' does not equal the resolved root Type's name '{}'; the hint is stale (the container remains valid)",
                            container_id, ctype, rt.name
                        ),
                    });
                }
            }
        }
    }

    // --- Validate blueprint and protocol definitions ---
    // Silent-skip for repos without a package (mirrors the `if let Ok(pkg)` guard pattern).
    if let Ok(boundaries) = store.list_package_boundaries() {
        for boundary in &boundaries {
            let prefix = boundary.selector.as_deref().unwrap_or("package");

            // Blueprint: JSON Schema validation + semantic validation
            for bp_path in &boundary.blueprint_paths {
                let full_path = format!("{prefix}/{bp_path}");
                let bp_value = match store.load_instance_json(&full_path) {
                    Ok(v) => v,
                    Err(e) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: full_path.clone(),
                            schema_id: None,
                            message: format!("failed to load blueprint definition: {e}"),
                        });
                        continue;
                    }
                };
                if let Some(schema_diags) = validate_value_against_schema(
                    &bp_value,
                    &full_path,
                    srs_schema::BLUEPRINT_SCHEMA_ID,
                    reg,
                ) {
                    diagnostics.extend(schema_diags);
                }
                match serde_json::from_value::<Blueprint>(bp_value) {
                    Ok(bp) => {
                        for diag in validate_blueprint(&bp).diagnostics {
                            let severity = match diag.severity {
                                BlueprintDiagnosticSeverity::Error => DiagnosticSeverity::Error,
                                BlueprintDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
                            };
                            diagnostics.push(ValidationDiagnostic {
                                severity,
                                relative_path: full_path.clone(),
                                schema_id: None,
                                message: diag.message,
                            });
                        }
                    }
                    Err(e) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: full_path.clone(),
                            schema_id: None,
                            message: format!("failed to parse blueprint definition: {e}"),
                        });
                    }
                }
            }

            // Protocol: semantic validation only
            for proto_path in &boundary.protocol_paths {
                let full_path = format!("{prefix}/{proto_path}");
                let proto_value = match store.load_instance_json(&full_path) {
                    Ok(v) => v,
                    Err(e) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: full_path.clone(),
                            schema_id: None,
                            message: format!("failed to load protocol definition: {e}"),
                        });
                        continue;
                    }
                };
                match serde_json::from_value::<Protocol>(proto_value) {
                    Ok(proto) => {
                        for diag in validate_protocol(&proto).diagnostics {
                            let severity = match diag.severity {
                                ProtocolDiagnosticSeverity::Error => DiagnosticSeverity::Error,
                                ProtocolDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
                            };
                            diagnostics.push(ValidationDiagnostic {
                                severity,
                                relative_path: full_path.clone(),
                                schema_id: None,
                                message: diag.message,
                            });
                        }
                    }
                    Err(e) => {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: full_path.clone(),
                            schema_id: None,
                            message: format!("failed to parse protocol definition: {e}"),
                        });
                    }
                }
            }
        }
    }

    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Warning)
        .count();

    Ok(RepositoryValidationReport {
        diagnostics,
        summary: ValidationSummary {
            checked,
            errors,
            warnings,
        },
    })
}

/// RFC-020 Rule [N+33] — every Type's effective `identityFieldId` (own or inherited) must
/// reference a `fieldId` present in that Type's effective field set. Accumulates diagnostics
/// (does not fail fast): a resolution error on one Type (e.g. an unrelated inheritance cycle)
/// is reported for that Type only and does not prevent other Types from being checked.
fn validate_identity_field_invariants(
    pkg: &crate::package::Package,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    for rt in pkg.record_types() {
        match pkg.effective_identity_field_id(rt) {
            Ok(Some(field_id)) => match pkg.effective_fields(rt) {
                Ok(fields) => {
                    if !fields.iter().any(|fa| fa.field_id == field_id) {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: "package/package.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "RFC-020 (Rule [N+33]): type '{}/{}@{}' identityFieldId '{}' is not in the effective field set",
                                rt.namespace, rt.name, rt.version, field_id
                            ),
                        });
                    }
                }
                Err(e) => {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "package/package.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "RFC-020 (Rule [N+33]): type '{}/{}@{}' effective field set could not be resolved to validate identityFieldId: {}",
                            rt.namespace, rt.name, rt.version, e
                        ),
                    });
                }
            },
            Ok(None) => {}
            Err(e) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "RFC-020 (Rule [N+33]): type '{}/{}@{}' identityFieldId could not be resolved: {}",
                        rt.namespace, rt.name, rt.version, e
                    ),
                });
            }
        }
    }
}

/// RFC-032 conformance rules R2–R10 over every Field in the package.
///
/// These are the semantic checks JSON Schema cannot express portably (the
/// frozen seed only approximates a few of them with `allOf`/`if`/`then`), so
/// without this pass they were declared and never run: `validate_field_v3` had
/// no production caller at all, and a package could carry an unresolvable `ref`
/// range or a datatype-inappropriate constraint and validate clean.
///
/// **Warning, not error — deliberately, for this release.** Turning a check on
/// for the first time finds pre-existing defects, not new ones: the spec repo's
/// own package has a `protocol-tags` Field that was a pre-RFC-032 `multiselect`
/// with neither `allowedValues` nor `vocabularyRef`, which RFC-032's migration
/// faithfully carries forward as a closed domain with no source set. Making
/// that a hard error would fail `repo validate` across the ecosystem for a
/// defect that predates this change. Reporting it makes the gap visible, which
/// is the point; promoting these to errors belongs with the data cleanup, as a
/// separate, deliberate step.
fn validate_field_type_conformance(
    pkg: &crate::package::Package,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    use srs_core::validation::field::validate_field_v3;
    for field in &pkg.fields {
        for d in validate_field_v3(field) {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Warning,
                relative_path: "package/package.json".to_string(),
                schema_id: None,
                message: format!("RFC-032 conformance: {}", d.message),
            });
        }
    }
}

/// I-92/94/95/96 (ext:cross-field-validation): "MUST be reported as a Type-level
/// validation error". `validate_cross_field_rules` (used at record write/validate time,
/// `record_store.rs`/this file's Tier-2 record loop) only ever runs against an actual
/// Record, so a Type carrying a misconfigured rule and zero Records was never flagged —
/// narrower than the invariants' text. This closes that gap at the Type level, independent
/// of record count. `Error`, matching the invariants' "MUST".
fn validate_cross_field_rule_configuration(
    pkg: &crate::package::Package,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    use srs_core::validation::record_type::validate_cross_field_rules_for_type;
    for rt in pkg.record_types() {
        for err in validate_cross_field_rules_for_type(rt, &pkg.fields) {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: "package/package.json".to_string(),
                schema_id: None,
                message: format!(
                    "ext:cross-field-validation (I-92/94/95/96): type '{}/{}@{}' {}",
                    rt.namespace, rt.name, rt.version, err
                ),
            });
        }
    }
}

/// `[N+1]` / ext:views-l2 — the "diagnose" half of the two-plane disposition the
/// owner settled for an ineligible `titleFieldId` (srs PR #341, 2026-08-02): the
/// render plane omits the heading (see [`crate::render_service::resolve_heading_field_id`]);
/// this is the validation plane, surfacing the misconfiguration rather than letting
/// it disappear silently into a missing heading.
///
/// `TypeQuery.semanticObjectType` and `ContainerSubset.typeFilter` declare their
/// candidate Types statically. `FixedInstances` and `RelationQuery` declare none —
/// their members are only known by resolving actual instances, exactly as the
/// render plane's `resolve_section_instances` does — so this reads those instances
/// (and, for `RelationQuery`, the relations file) via `store` to recover the same
/// per-record actual Type the render plane would use for `rt` in
/// `resolve_heading_field_id`. When an instance can't be resolved (I/O error, dangling
/// id, a Tier-0/1 instance with no Type), it contributes no candidate — referential
/// integrity is a separate diagnostic's job, and a Tier-0/1 instance renders with no
/// Type either, so there is nothing here to warn about for that instance.
fn validate_title_field_id_eligibility(
    store: &dyn RepositoryStore,
    pkg: &crate::package::Package,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    use srs_core::types::view::SectionSource;

    // Resolves the actual Type of a live instance, mirroring how the render plane
    // derives `rt` for `resolve_heading_field_id` (`record.type_id`/`type_version`
    // looked up in `pkg`). A plain fn, not a closure, so its return lifetime ties
    // to `pkg` rather than to whatever transient `&str` is passed in.
    fn resolve_instance_type<'p>(
        store: &dyn RepositoryStore,
        pkg: &'p crate::package::Package,
        id: &str,
    ) -> Option<&'p srs_core::types::record_type::RecordType> {
        match crate::record_store::get_instance_by_id(store, id)
            .ok()
            .flatten()?
        {
            crate::record_store::LoadedInstance::Record(record) => {
                pkg.resolve_type(&record.type_id, record.type_version)
            }
            crate::record_store::LoadedInstance::Note(_) => None,
        }
    }

    // Loaded at most once, and only when a RelationQuery section actually needs it.
    let mut relations: Option<Vec<srs_core::types::relation::Relation>> = None;

    for dv in &pkg.document_views {
        for section in &dv.sections {
            let Some(field_id) = &section.title_field_id else {
                continue;
            };
            // Unresolvable field ids are referential-integrity's job, not this
            // rule's — mirrors `title_field_id_is_eligible`'s own guard.
            if pkg.resolve_field(field_id).is_none() {
                continue;
            }

            let candidate_types: Vec<&srs_core::types::record_type::RecordType> =
                match &section.source {
                    SectionSource::TypeQuery {
                        semantic_object_type,
                        ..
                    } => match semantic_object_type.split_once('/') {
                        Some((namespace, name)) => pkg
                            .record_types()
                            .iter()
                            .filter(|t| t.namespace == namespace && t.name == name)
                            .collect(),
                        None => Vec::new(),
                    },
                    SectionSource::ContainerSubset {
                        type_filter: Some(keys),
                        ..
                    } => keys
                        .iter()
                        .filter_map(|key| key.split_once('/'))
                        .flat_map(|(namespace, name)| {
                            pkg.record_types()
                                .iter()
                                .filter(move |t| t.namespace == namespace && t.name == name)
                        })
                        .collect(),
                    SectionSource::FixedInstances { instance_ids } => instance_ids
                        .iter()
                        .filter_map(|id| resolve_instance_type(store, pkg, id))
                        .collect(),
                    SectionSource::RelationQuery {
                        from_instance_id,
                        relation_type,
                        direction,
                    } => {
                        let rels = relations.get_or_insert_with(|| {
                            crate::relation_service::load_relations(store).unwrap_or_default()
                        });
                        let dir = direction
                            .as_ref()
                            .unwrap_or(&srs_core::types::view::RelationDirection::Forward);
                        rels.iter()
                            .filter(|r| r.relation_type == *relation_type)
                            .filter_map(|r| match dir {
                                srs_core::types::view::RelationDirection::Forward
                                    if r.source_instance_id == *from_instance_id =>
                                {
                                    Some(r.target_instance_id.as_str())
                                }
                                srs_core::types::view::RelationDirection::Inverse
                                    if r.target_instance_id == *from_instance_id =>
                                {
                                    Some(r.source_instance_id.as_str())
                                }
                                _ => None,
                            })
                            .filter_map(|id| resolve_instance_type(store, pkg, id))
                            .collect()
                    }
                    _ => Vec::new(),
                };

            if candidate_types.is_empty() {
                if !crate::render_service::title_field_id_is_eligible(field_id, None, pkg) {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Warning,
                        relative_path: "package/package.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "RFC-032 Revision 7 ([N+1]): documentView '{}' section '{}' titleFieldId '{}' is not eligible (must be an effective-single, open-domain, prose-formatted string field); the heading will be omitted at render time",
                            dv.id, section.section_id, field_id
                        ),
                    });
                }
                continue;
            }

            for rt in candidate_types {
                if !crate::render_service::title_field_id_is_eligible(field_id, Some(rt), pkg) {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Warning,
                        relative_path: "package/package.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "RFC-032 Revision 7 ([N+1]): documentView '{}' section '{}' titleFieldId '{}' is not eligible for type '{}/{}@{}' (must be an effective-single, open-domain, prose-formatted string field); the heading will be omitted at render time",
                            dv.id, section.section_id, field_id, rt.namespace, rt.name, rt.version
                        ),
                    });
                }
            }
        }
    }
}

fn validate_vocabulary_invariants(
    pkg: &crate::package::Package,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    // V2: every field.vocabularyRef must resolve to an installed Vocabulary UUID
    for field in &pkg.fields {
        if let Some(ref_id) = &field.field_type.vocabulary_ref {
            if !pkg.vocabularies.iter().any(|v| &v.id == ref_id) {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "V2: field '{}' vocabularyRef '{}' does not resolve to an installed Vocabulary",
                        field.name, ref_id
                    ),
                });
            }
        }
    }

    // V5: key∪alias set must be disjoint within each vocabulary (non-retired terms only)
    for vocab in &pkg.vocabularies {
        let mut seen: HashSet<&str> = HashSet::new();
        for term in vocab.effective_terms() {
            if !seen.insert(term.key.as_str()) {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "V5: vocabulary '{}' has duplicate key '{}'",
                        vocab.name, term.key
                    ),
                });
            }
            if let Some(aliases) = &term.aliases {
                for alias in aliases {
                    if !seen.insert(alias.as_str()) {
                        diagnostics.push(ValidationDiagnostic {
                            severity: DiagnosticSeverity::Error,
                            relative_path: "package/package.json".to_string(),
                            schema_id: None,
                            message: format!(
                                "V5: vocabulary '{}' has duplicate key '{}'",
                                vocab.name, alias
                            ),
                        });
                    }
                }
            }
        }
    }

    // RFC-039 [R4]: within a Type's effective field set every referenced
    // Field.name must be distinct — name-keying makes a duplicate an instance
    // ambiguity, so it is rejected at definition time, not instance time.
    for rt in &pkg.record_types {
        if let Ok(effective) = pkg.resolved_effective_fields(rt) {
            let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for ef in &effective {
                if ef.name.is_empty() {
                    continue; // dangling fieldId — reported separately
                }
                if !seen.insert(ef.name.as_str()) {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        relative_path: "package/package.json".to_string(),
                        schema_id: None,
                        message: format!(
                            "type '{}/{}@{}': {}",
                            rt.namespace,
                            rt.name,
                            rt.version,
                            srs_core::error::CoreError::DuplicateEffectiveFieldName {
                                name: ef.name.clone(),
                            }
                        ),
                    });
                }
            }
        }
    }

    // V7: mutual exclusion (lifecycle and lifecycleRef both set)
    // V8: every type.lifecycleRef must resolve to an installed Lifecycle UUID
    // V9: structural integrity for inline TypeLifecycle
    for rt in &pkg.record_types {
        // V7: mutual exclusion
        if rt.lifecycle.is_some() && rt.lifecycle_ref.is_some() {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Error,
                relative_path: "package/package.json".to_string(),
                schema_id: None,
                message: format!(
                    "V7: type '{}' declares both 'lifecycle' and 'lifecycleRef'; exactly one is allowed",
                    rt.name
                ),
            });
            // Skip V8 and V9 for this type — V7 already fired
            continue;
        }

        // V8: lifecycleRef must resolve
        if let Some(ref_id) = &rt.lifecycle_ref {
            if !pkg.lifecycles.iter().any(|lc| &lc.id == ref_id) {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "V8: type '{}' lifecycleRef '{}' does not resolve to an installed Lifecycle",
                        rt.name, ref_id
                    ),
                });
            }
        }

        // V9: structural checks on inline TypeLifecycle
        if let Some(inline_lc) = &rt.lifecycle {
            for diag in
                validate_type_lifecycle_v9(&inline_lc.states, &inline_lc.transitions, &rt.name)
            {
                let severity = match diag.severity {
                    LifecycleDiagnosticSeverity::Error => DiagnosticSeverity::Error,
                };
                diagnostics.push(ValidationDiagnostic {
                    severity,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: diag.message,
                });
            }

            // V9: initialState field must match the isInitial state's key
            let initial_states: Vec<_> = inline_lc
                .states
                .iter()
                .filter(|s| s.is_initial == Some(true))
                .collect();
            if initial_states.len() == 1 && initial_states[0].key != inline_lc.initial_state {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "V9: inline lifecycle on type '{}' initialState '{}' does not match isInitial state key '{}'",
                        rt.name, inline_lc.initial_state, initial_states[0].key
                    ),
                });
            }
        }
    }

    // V5/V9: full lifecycle invariant validation for every standalone Lifecycle
    for lc in &pkg.lifecycles {
        for diag in validate_lifecycle(lc) {
            let severity = match diag.severity {
                LifecycleDiagnosticSeverity::Error => DiagnosticSeverity::Error,
            };
            diagnostics.push(ValidationDiagnostic {
                severity,
                relative_path: "package/package.json".to_string(),
                schema_id: None,
                message: diag.message,
            });
        }

        // V9: initialState field must match the key of the isInitial state
        let initial_states: Vec<&srs_core::types::lifecycle::LifecycleState> = lc
            .states
            .iter()
            .filter(|s| s.is_initial == Some(true))
            .collect();
        if initial_states.len() == 1 {
            let initial_key = &initial_states[0].key;
            if initial_key != &lc.initial_state {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "V9: lifecycle '{}' initialState '{}' does not match isInitial state key '{}'",
                        lc.name, lc.initial_state, initial_key
                    ),
                });
            }
        }
    }
}

fn tier_to_schema_id(tier: u8) -> Option<&'static str> {
    match tier {
        0 => Some(NOTE_SCHEMA_ID),
        2 => Some(RECORD_SCHEMA_ID),
        _ => None,
    }
}

fn validate_value_against_schema(
    value: &Value,
    rel_path: &str,
    schema_id: &'static str,
    reg: &SchemaRegistry,
) -> Option<Vec<ValidationDiagnostic>> {
    let mut diags = Vec::new();
    if let Err(e) = reg.validate_by_id(schema_id, value) {
        let message = e.to_string();
        if schema_id == srs_schema::PACKAGE_MANIFEST_SCHEMA_ID
            && rel_path == "package/package.json"
            && message.contains("Additional properties are not allowed")
            && message.contains("documentViews")
        {
            diags.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Warning,
                relative_path: rel_path.to_string(),
                schema_id: Some(schema_id.to_string()),
                message: "package manifest uses forward-compatible field 'documentViews' not yet present in embedded schema".to_string(),
            });
            return Some(diags);
        }
        diags.push(ValidationDiagnostic {
            severity: DiagnosticSeverity::Error,
            relative_path: rel_path.to_string(),
            schema_id: Some(schema_id.to_string()),
            message,
        });
    }
    Some(diags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn srs_spec_repo() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("SRS_SPEC_REPO") {
            return std::path::PathBuf::from(p);
        }
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let vendored = manifest.join("../../tests/fixtures/spec-repo");
        if let Ok(c) = vendored.canonicalize() {
            if c.join(".srs").exists() {
                return c;
            }
        }
        let mut dir = manifest.to_path_buf();
        loop {
            let candidate = dir.join("../srs/srs");
            if let Ok(c) = candidate.canonicalize() {
                if c.join(".srs").exists() {
                    return c;
                }
            }
            match dir.parent() {
                Some(p) if p != dir => dir = p.to_path_buf(),
                _ => break,
            }
        }
        manifest.join("../../../srs/srs")
    }

    fn write_json(dir: &Path, rel: &str, value: &Value) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn minimal_manifest(instance_index: serde_json::Value) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Test Repo",
            "container": {
                "containerId": "00000000-0000-4000-8000-000000000099",
                "title": "Test Repo"
            },
            "instanceIndex": instance_index,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn valid_note(instance_id: &str) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
            "instanceId": instance_id,
            "sections": [{"name": "body", "content": "hello"}]
        })
    }

    #[test]
    fn valid_repo_reports_no_errors() {
        let temp = TempDir::new().unwrap();
        let note_id = "00000000-0000-4000-8000-000000000001";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": note_id,
                "tier": 0,
                "path": "records/notes/note.json"
            }])),
        );
        write_json(temp.path(), "records/notes/note.json", &valid_note(note_id));

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(report.is_ok(), "diagnostics: {:?}", report.diagnostics);
        // checked counts only instanceIndex entries, not infrastructure files (manifest, package, relations)
        assert_eq!(report.summary.checked, 1);
    }

    #[test]
    fn invalid_note_produces_error_diagnostic() {
        let temp = TempDir::new().unwrap();
        let note_id = "00000000-0000-4000-8000-000000000001";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": note_id,
                "tier": 0,
                "path": "records/notes/note.json"
            }])),
        );
        // Missing required "sections" field
        write_json(
            temp.path(),
            "records/notes/note.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
                "instanceId": note_id
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(!report.is_ok());
        assert!(report.summary.errors >= 1);
        let msgs: Vec<_> = report.diagnostics.iter().map(|d| &d.message).collect();
        assert!(
            msgs.iter().any(|m| m.contains("sections")),
            "expected sections error, got: {msgs:?}"
        );
    }

    #[test]
    fn tier_schema_mismatch_produces_error_diagnostic() {
        let temp = TempDir::new().unwrap();
        let note_id = "00000000-0000-4000-8000-000000000001";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": note_id,
                "tier": 0,
                "path": "records/notes/note.json"
            }])),
        );
        // Tier 0 but declares record.json schema — mismatch
        write_json(
            temp.path(),
            "records/notes/note.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": note_id,
                "sections": []
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(!report.is_ok());
        // RFC-038 [R7]: a declared $schema is validated, never reclassified by shape —
        // this now surfaces as the catalog's own schema-validation diagnostic (folded
        // into the report) rather than validate_repository's separate tier-vs-schema
        // check, which never sees a catalog-rejected file.
        let mismatch = report.diagnostics.iter().any(|d| {
            d.message.contains("SRS038-R7-SCHEMA-VALIDATION") && d.message.contains("record.json")
        });
        assert!(
            mismatch,
            "expected a declared-schema validation diagnostic, got: {:?}",
            report.diagnostics
        );
    }

    fn minimal_package_json(type_path: Option<&str>, vocab_path: Option<&str>) -> Value {
        let types = if let Some(p) = type_path {
            json!([p])
        } else {
            json!([])
        };
        let vocabs = if let Some(p) = vocab_path {
            json!([p])
        } else {
            json!([])
        };
        json!({
            "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
            "title": "test-package",
            "description": "",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [],
            "types": types,
            "views": [],
            "vocabularies": vocabs
        })
    }

    fn minimal_type_json(type_id: &str) -> Value {
        json!({
            "$schema": srs_schema::TYPE_SCHEMA_ID,
            "id": type_id,
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "Test type",
            "fields": [],
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn minimal_record_json(record_id: &str, type_id: &str, tags: Option<Vec<&str>>) -> Value {
        let tag_value = tags.map(|t| json!(t)).unwrap_or(json!(null));
        let mut obj = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": record_id,
            "typeId": type_id,
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "test-type",
            "fieldValues": {},
            "createdAt": "2026-01-01T00:00:00Z"
        });
        if !tag_value.is_null() {
            obj["tags"] = tag_value;
        }
        obj
    }

    fn minimal_package_json_full(
        field_paths: &[&str],
        type_paths: &[&str],
        vocab_paths: &[&str],
        lifecycle_paths: &[&str],
    ) -> Value {
        json!({
            "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
            "title": "test-package",
            "description": "",
            "status": "active",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": field_paths,
            "types": type_paths,
            "views": [],
            "vocabularies": vocab_paths,
            "lifecycles": lifecycle_paths
        })
    }

    fn minimal_field_json_with_vocab_ref(
        field_id: &str,
        field_name: &str,
        vocab_ref: Option<&str>,
    ) -> Value {
        let mut obj = json!({
            "$schema": srs_schema::FIELD_SCHEMA_ID,
            "id": field_id,
            "namespace": "com.test",
            "name": field_name,
            "version": 1,
            "description": "Test field",
            "aiGuidance": {"purpose": "test field"},
            "fieldType": {"datatype": "string"},
            "createdAt": "2026-01-01T00:00:00Z"
        });
        if let Some(vr) = vocab_ref {
            // A vocabularyRef only makes sense on a closed domain (RFC-032 R3).
            obj["fieldType"] = json!({
                "datatype": "string",
                "valueDomain": "closed",
                "vocabularyRef": vr
            });
        }
        obj
    }

    fn minimal_type_json_with_identity_field_id(type_id: &str, identity_field_id: &str) -> Value {
        json!({
            "id": type_id,
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "Test type",
            "fields": [
                {
                    "fieldId": "00000000-0000-4000-8000-0000000000f1",
                    "order": 0,
                    "required": true
                }
            ],
            "identityFieldId": identity_field_id,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn minimal_type_json_with_lifecycle_ref(type_id: &str, lifecycle_ref: &str) -> Value {
        json!({
            "id": type_id,
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "Test type",
            "fields": [],
            "lifecycleRef": lifecycle_ref,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn minimal_type_json_with_inline_lifecycle(type_id: &str, lifecycle: Value) -> Value {
        json!({
            "id": type_id,
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "Test type",
            "fields": [],
            "lifecycle": lifecycle,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn minimal_lifecycle_json(lc_id: &str, initial_state: &str, states: Value) -> Value {
        json!({
            "id": lc_id,
            "version": 1,
            "namespace": "com.test",
            "name": "test-lifecycle",
            "states": states,
            "transitions": [],
            "initialState": initial_state,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn minimal_record_with_lifecycle_state(
        record_id: &str,
        type_id: &str,
        lifecycle_state: &str,
    ) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": record_id,
            "typeId": type_id,
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "test-type",
            "fieldValues": {},
            "lifecycleState": lifecycle_state,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    // Sets up a minimal package-only repo (no instances) with the given package.json content.
    // Used to test vocabulary invariants (V2/V5/V7/V9) without needing tier-2 records.
    fn setup_package_only_repo(temp: &TempDir, package_json: &Value) {
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(temp.path(), "package/package.json", package_json);
    }

    fn minimal_vocab_json(vocab_id: &str, mode: &str, terms: Vec<(&str, &str)>) -> Value {
        let term_array: Vec<Value> = terms
            .iter()
            .map(|(term_id, key)| {
                json!({
                    "id": term_id,
                    "version": 1,
                    "namespace": "com.test",
                    "key": key
                })
            })
            .collect();
        json!({
            "id": vocab_id,
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": mode,
            "terms": term_array,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn setup_repo_with_tagged_record(
        temp: &TempDir,
        vocab_mode: &str,
        tag_on_record: &str,
        term_key: &str,
    ) {
        let record_id = "00000000-0000-4000-8000-000000000002";
        let type_id = "00000000-0000-4000-8000-000000000003";
        let vocab_id = "00000000-0000-4000-8000-000000000004";
        let term_id = "00000000-0000-4000-8000-000000000005";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/my-record.json",
                "tags": [tag_on_record]
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(
                Some("types/test-type.json"),
                Some("vocabularies/test-vocab.json"),
            ),
        );
        write_json(
            temp.path(),
            "package/vocabularies/test-vocab.json",
            &minimal_vocab_json(vocab_id, vocab_mode, vec![(term_id, term_key)]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json(type_id),
        );
        write_json(
            temp.path(),
            "records/my-record.json",
            &minimal_record_json(record_id, type_id, Some(vec![tag_on_record])),
        );
    }

    #[test]
    fn no_vocab_declared_skips_tag_enforcement() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000000002";
        let type_id = "00000000-0000-4000-8000-000000000003";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/my-record.json",
                "tags": ["any:free-string"]
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        // Package with no vocabularies
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(Some("types/test-type.json"), None),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json(type_id),
        );
        write_json(
            temp.path(),
            "records/my-record.json",
            &minimal_record_json(record_id, type_id, Some(vec!["any:free-string"])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        // No tag enforcement without a declared vocabulary — must not produce a tag diagnostic
        let tag_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("does not resolve"))
            .collect();
        assert!(
            tag_diags.is_empty(),
            "expected no tag diagnostics without vocab, got: {:?}",
            tag_diags
        );
    }

    #[test]
    fn closed_vocab_unresolved_tag_produces_error() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_tagged_record(&temp, "closed", "unknown:tag", "construct:field");

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let tag_error = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("does not resolve")
        });
        assert!(
            tag_error.is_some(),
            "expected Error for unresolved tag in closed vocab, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn open_vocab_unresolved_tag_produces_warning() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_tagged_record(&temp, "open", "unknown:tag", "construct:field");

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let tag_warning = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Warning && d.message.contains("does not resolve")
        });
        assert!(
            tag_warning.is_some(),
            "expected Warning for unresolved tag in open vocab, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn resolved_tag_produces_no_diagnostic() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_tagged_record(&temp, "closed", "construct:field", "construct:field");

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let tag_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("does not resolve"))
            .collect();
        assert!(
            tag_diags.is_empty(),
            "expected no tag diagnostics for resolved tag, got: {:?}",
            tag_diags
        );
    }

    /// Regression test for issue #33: `repo map` and `repo validate` must agree on
    /// the instance count.  `validate`'s `summary.checked` must equal `map`'s
    /// `counts.total_instances` — both reflecting only `instanceIndex` entries, not
    /// infrastructure files (manifest.json, package/package.json, relations.json).
    #[test]
    fn map_and_validate_agree_on_instance_count() {
        use crate::analysis::build_repo_map;

        let temp = TempDir::new().unwrap();
        let note_id_1 = "00000000-0000-4000-8000-000000000011";
        let note_id_2 = "00000000-0000-4000-8000-000000000012";
        let note_id_3 = "00000000-0000-4000-8000-000000000013";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": note_id_1, "tier": 0, "path": "records/note1.json"},
                {"instanceId": note_id_2, "tier": 0, "path": "records/note2.json"},
                {"instanceId": note_id_3, "tier": 0, "path": "records/note3.json"}
            ])),
        );
        write_json(temp.path(), "records/note1.json", &valid_note(note_id_1));
        write_json(temp.path(), "records/note2.json", &valid_note(note_id_2));
        write_json(temp.path(), "records/note3.json", &valid_note(note_id_3));

        let store = crate::store::FileStore::new(temp.path());

        let validate_report = validate_repository(&store).unwrap();
        let repo_map = build_repo_map(&store).unwrap();

        assert_eq!(
            validate_report.summary.checked, repo_map.counts.total_instances,
            "repo validate checked ({}) != repo map total_instances ({})",
            validate_report.summary.checked, repo_map.counts.total_instances
        );
        // Sanity: both should equal 3 (the number of instanceIndex entries)
        assert_eq!(validate_report.summary.checked, 3);
    }

    // --- V2: field vocabularyRef UUID resolution ---

    #[test]
    fn vocabulary_v2_missing_vocabulary_ref_produces_error() {
        let temp = TempDir::new().unwrap();
        let field_id = "00000000-0000-4000-8000-000000000020";
        let nonexistent_vocab_id = "ffffffff-0000-4000-8000-000000000099";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&["fields/test-field.json"], &[], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/fields/test-field.json",
            &minimal_field_json_with_vocab_ref(field_id, "my-field", Some(nonexistent_vocab_id)),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v2_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V2"));
        assert!(
            v2_error.is_some(),
            "expected V2 error for unresolved vocabularyRef, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v2_resolved_vocabulary_ref_no_error() {
        let temp = TempDir::new().unwrap();
        let field_id = "00000000-0000-4000-8000-000000000020";
        let vocab_id = "00000000-0000-4000-8000-000000000030";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(
                &["fields/test-field.json"],
                &[],
                &["vocabularies/test-vocab.json"],
                &[],
            ),
        );
        write_json(
            temp.path(),
            "package/fields/test-field.json",
            &minimal_field_json_with_vocab_ref(field_id, "my-field", Some(vocab_id)),
        );
        write_json(
            temp.path(),
            "package/vocabularies/test-vocab.json",
            &minimal_vocab_json(vocab_id, "closed", vec![]),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v2_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V2"))
            .collect();
        assert!(
            v2_errors.is_empty(),
            "expected no V2 errors for resolved vocabularyRef, got: {:?}",
            v2_errors
        );
    }

    // --- V5: key∪alias uniqueness within vocabulary ---

    #[test]
    fn vocabulary_v5_duplicate_key_produces_error() {
        let temp = TempDir::new().unwrap();
        let vocab_id = "00000000-0000-4000-8000-000000000030";
        let term1_id = "00000000-0000-4000-8000-000000000031";
        let term2_id = "00000000-0000-4000-8000-000000000032";

        let vocab = json!({
            "id": vocab_id,
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "closed",
            "terms": [
                {"id": term1_id, "version": 1, "namespace": "com.test", "key": "duplicate"},
                {"id": term2_id, "version": 1, "namespace": "com.test", "key": "duplicate"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &["vocabularies/test-vocab.json"], &[]),
        );
        write_json(temp.path(), "package/vocabularies/test-vocab.json", &vocab);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v5_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V5"));
        assert!(
            v5_error.is_some(),
            "expected V5 error for duplicate key, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v5_duplicate_alias_produces_error() {
        let temp = TempDir::new().unwrap();
        let vocab_id = "00000000-0000-4000-8000-000000000030";
        let term1_id = "00000000-0000-4000-8000-000000000031";
        let term2_id = "00000000-0000-4000-8000-000000000032";

        // term2's alias "foo" duplicates term1's key "foo"
        let vocab = json!({
            "id": vocab_id,
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "closed",
            "terms": [
                {"id": term1_id, "version": 1, "namespace": "com.test", "key": "foo"},
                {"id": term2_id, "version": 1, "namespace": "com.test", "key": "bar", "aliases": ["foo"]}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &["vocabularies/test-vocab.json"], &[]),
        );
        write_json(temp.path(), "package/vocabularies/test-vocab.json", &vocab);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v5_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V5"));
        assert!(
            v5_error.is_some(),
            "expected V5 error for alias duplicating a key, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v5_duplicate_alias_alias_produces_error() {
        let temp = TempDir::new().unwrap();
        let vocab_id = "00000000-0000-4000-8000-000000000030";
        let term1_id = "00000000-0000-4000-8000-000000000031";
        let term2_id = "00000000-0000-4000-8000-000000000032";

        // both terms have alias "shared" — alias-alias collision
        let vocab = json!({
            "id": vocab_id,
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "open",
            "terms": [
                {"id": term1_id, "version": 1, "namespace": "com.test", "key": "a", "aliases": ["shared"]},
                {"id": term2_id, "version": 1, "namespace": "com.test", "key": "b", "aliases": ["shared"]}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &["vocabularies/test-vocab.json"], &[]),
        );
        write_json(temp.path(), "package/vocabularies/test-vocab.json", &vocab);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v5_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V5"));
        assert!(
            v5_error.is_some(),
            "expected V5 error for alias-alias collision, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v5_retired_term_excluded_from_uniqueness() {
        let temp = TempDir::new().unwrap();
        let vocab_id = "00000000-0000-4000-8000-000000000030";
        let term1_id = "00000000-0000-4000-8000-000000000031";
        let term2_id = "00000000-0000-4000-8000-000000000032";

        // retired term with same key as active — must not be a V5 conflict
        let vocab = json!({
            "id": vocab_id,
            "version": 1,
            "namespace": "com.test",
            "name": "test-vocab",
            "mode": "closed",
            "terms": [
                {"id": term1_id, "version": 1, "namespace": "com.test", "key": "foo"},
                {"id": term2_id, "version": 1, "namespace": "com.test", "key": "foo", "status": "retired"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &["vocabularies/test-vocab.json"], &[]),
        );
        write_json(temp.path(), "package/vocabularies/test-vocab.json", &vocab);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v5_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V5"))
            .collect();
        assert!(
            v5_errors.is_empty(),
            "expected no V5 errors when retired term shares key with active, got: {:?}",
            v5_errors
        );
    }

    // --- V8: type lifecycleRef UUID resolution ---

    #[test]
    fn vocabulary_v8_missing_lifecycle_ref_produces_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let nonexistent_lc_id = "ffffffff-0000-4000-8000-000000000099";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_lifecycle_ref(type_id, nonexistent_lc_id),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v8_error = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("V8")
                && d.message.contains("lifecycleRef")
        });
        assert!(
            v8_error.is_some(),
            "expected V8 lifecycleRef error for unresolved lifecycleRef, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v8_resolved_lifecycle_ref_no_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(
                &[],
                &["types/test-type.json"],
                &[],
                &["lifecycles/test-lc.json"],
            ),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_lifecycle_ref(type_id, lc_id),
        );
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(lc_id, "draft", json!([{"key": "draft", "isInitial": true}])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v8_ref_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V8") && d.message.contains("lifecycleRef"))
            .collect();
        assert!(
            v8_ref_errors.is_empty(),
            "expected no V8 lifecycleRef errors for resolved lifecycleRef, got: {:?}",
            v8_ref_errors
        );
    }

    // --- RFC-020 Rule [N+33]: identityFieldId effective-field-set validation ---

    #[test]
    fn identity_field_id_dangling_reference_produces_diagnostic() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let dangling_field_id = "ffffffff-0000-4000-8000-000000000099";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_identity_field_id(type_id, dangling_field_id),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let n33_error = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("N+33")
                && d.message.contains("identityFieldId")
        });
        assert!(
            n33_error.is_some(),
            "expected Rule [N+33] error for dangling identityFieldId, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn identity_field_id_valid_reference_no_diagnostic() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let valid_field_id = "00000000-0000-4000-8000-0000000000f1";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_identity_field_id(type_id, valid_field_id),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let n33_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("N+33"))
            .collect();
        assert!(
            n33_errors.is_empty(),
            "expected no Rule [N+33] errors for a valid identityFieldId, got: {:?}",
            n33_errors
        );
    }

    #[test]
    fn identity_field_id_resolution_error_on_one_type_does_not_block_others() {
        let temp = TempDir::new().unwrap();
        let cyclic_type_id = "00000000-0000-4000-8000-000000000041";
        let other_type_id = "00000000-0000-4000-8000-000000000042";
        let dangling_field_id = "ffffffff-0000-4000-8000-000000000099";

        // A type that extends itself — effective_identity_field_id will hit
        // TypeInheritanceCycle when resolving this type's ancestor chain.
        let cyclic_type = json!({
            "id": cyclic_type_id,
            "namespace": "com.test",
            "name": "cyclic-type",
            "version": 1,
            "description": "Self-extending type",
            "fields": [],
            "extendsTypeId": cyclic_type_id,
            "extendsTypeVersion": 1,
            "createdAt": "2026-01-01T00:00:00Z"
        });

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(
                &[],
                &["types/cyclic-type.json", "types/other-type.json"],
                &[],
                &[],
            ),
        );
        write_json(temp.path(), "package/types/cyclic-type.json", &cyclic_type);
        // The other, unrelated type has its own (independent) dangling identityFieldId. If the
        // cyclic type's Err aborted the whole validation loop instead of being accumulated, this
        // type's diagnostic would never be produced.
        write_json(
            temp.path(),
            "package/types/other-type.json",
            &minimal_type_json_with_identity_field_id(other_type_id, dangling_field_id),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();

        let n33_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("N+33"))
            .collect();
        assert!(
            n33_errors.iter().any(|d| d.message.contains(cyclic_type_id)),
            "expected a Rule [N+33] diagnostic reporting the cyclic type's resolution error, got: {:?}",
            report.diagnostics
        );
        // The "other" type's diagnostic uses namespace/name@version, not the raw UUID, so check
        // its distinguishing content (the dangling field id) instead of `other_type_id` directly.
        assert!(
            n33_errors.iter().any(|d| d.message.contains(dangling_field_id)
                && d.message.contains("effective field set")),
            "the cyclic type's error must not suppress diagnostics collection for the unrelated type; diagnostics: {:?}",
            report.diagnostics
        );
    }

    // --- V9: lifecycle initialState invariants ---

    #[test]
    fn lifecycle_v9_zero_initial_states_produces_error() {
        let temp = TempDir::new().unwrap();
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &[], &["lifecycles/test-lc.json"]),
        );
        // No isInitial:true on any state
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(lc_id, "draft", json!([{"key": "draft"}, {"key": "active"}])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("no initial state")
        });
        assert!(
            err.is_some(),
            "expected error for zero isInitial states, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn lifecycle_v9_multiple_initial_states_produces_error() {
        let temp = TempDir::new().unwrap();
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &[], &["lifecycles/test-lc.json"]),
        );
        // Two states with isInitial:true
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(
                lc_id,
                "draft",
                json!([
                    {"key": "draft", "isInitial": true},
                    {"key": "active", "isInitial": true}
                ]),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("initial states")
        });
        assert!(
            err.is_some(),
            "expected error for multiple isInitial states, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn lifecycle_v9_single_initial_state_no_error() {
        let temp = TempDir::new().unwrap();
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &[], &["lifecycles/test-lc.json"]),
        );
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(
                lc_id,
                "draft",
                json!([{"key": "draft", "isInitial": true}, {"key": "active"}]),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let lc_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("initial"))
            .collect();
        assert!(
            lc_errors.is_empty(),
            "expected no lifecycle errors for valid lifecycle, got: {:?}",
            lc_errors
        );
    }

    #[test]
    fn lifecycle_v9_initial_state_key_mismatch_produces_error() {
        let temp = TempDir::new().unwrap();
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &[], &["lifecycles/test-lc.json"]),
        );
        // isInitial state key is "draft" but initialState points to "other"
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(lc_id, "other", json!([{"key": "draft", "isInitial": true}])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v9_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V9"));
        assert!(
            v9_error.is_some(),
            "expected V9 error for initialState/isInitial key mismatch, got: {:?}",
            report.diagnostics
        );
    }

    // --- V9c: standalone lifecycle transition references undefined state (#135) ---

    #[test]
    fn lifecycle_standalone_transition_to_undefined_state_produces_error() {
        let temp = TempDir::new().unwrap();
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &[], &[], &["lifecycles/test-lc.json"]),
        );
        // Transition references "ghost" which is not in states[]
        let mut lc_json = minimal_lifecycle_json(
            lc_id,
            "draft",
            json!([{"key": "draft", "isInitial": true}, {"key": "active"}]),
        );
        lc_json["transitions"] = json!([{"name": "promote", "from": "draft", "to": "ghost"}]);
        write_json(temp.path(), "package/lifecycles/test-lc.json", &lc_json);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("ghost"));
        assert!(
            err.is_some(),
            "expected error for transition to undefined state 'ghost', got: {:?}",
            report.diagnostics
        );
    }

    // --- V8: dangling lifecycleRef produces a clear diagnostic (#136) ---

    #[test]
    fn dangling_lifecycle_ref_produces_clear_v8_diagnostic() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let missing_lc_id = "ffffffff-0000-4000-8000-000000000099";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_lifecycle_ref(type_id, missing_lc_id),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v8 = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("V8")
                && d.message.contains("lifecycleRef")
                && d.message.contains(missing_lc_id)
        });
        assert!(
            v8.is_some(),
            "expected V8 lifecycleRef diagnostic naming the dangling UUID, got: {:?}",
            report.diagnostics
        );
    }

    // --- V7: mutual exclusion of lifecycle and lifecycleRef ---

    fn minimal_type_json_with_both_lifecycle_fields(type_id: &str, lifecycle_ref: &str) -> Value {
        json!({
            "id": type_id,
            "namespace": "com.test",
            "name": "test-type",
            "version": 1,
            "description": "Test type",
            "fields": [],
            "lifecycle": {"states": [{"key": "draft", "isInitial": true}], "transitions": [], "initialState": "draft"},
            "lifecycleRef": lifecycle_ref,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    #[test]
    fn vocabulary_v7_both_lifecycle_and_ref_produces_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(
                &[],
                &["types/test-type.json"],
                &[],
                &["lifecycles/test-lc.json"],
            ),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_both_lifecycle_fields(type_id, lc_id),
        );
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(lc_id, "draft", json!([{"key": "draft", "isInitial": true}])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v7_error = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("V7")
                && d.message.contains("both")
        });
        assert!(
            v7_error.is_some(),
            "expected V7 mutual-exclusion error, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v7_only_lifecycle_ref_no_v7_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(
                &[],
                &["types/test-type.json"],
                &[],
                &["lifecycles/test-lc.json"],
            ),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_lifecycle_ref(type_id, lc_id),
        );
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(lc_id, "draft", json!([{"key": "draft", "isInitial": true}])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v7_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V7"))
            .collect();
        assert!(
            v7_errors.is_empty(),
            "expected no V7 errors when only lifecycleRef is set, got: {:?}",
            v7_errors
        );
    }

    #[test]
    fn vocabulary_v7_only_inline_lifecycle_no_v7_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(
                type_id,
                json!({"states": [{"key": "draft", "isInitial": true}], "transitions": [], "initialState": "draft"}),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v7_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V7"))
            .collect();
        assert!(
            v7_errors.is_empty(),
            "expected no V7 errors when only inline lifecycle is set, got: {:?}",
            v7_errors
        );
    }

    #[test]
    fn vocabulary_v7_both_set_no_v9_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";
        let lc_id = "00000000-0000-4000-8000-000000000050";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(
                &[],
                &["types/test-type.json"],
                &[],
                &["lifecycles/test-lc.json"],
            ),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_both_lifecycle_fields(type_id, lc_id),
        );
        write_json(
            temp.path(),
            "package/lifecycles/test-lc.json",
            &minimal_lifecycle_json(lc_id, "draft", json!([{"key": "draft", "isInitial": true}])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v9_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V9"))
            .collect();
        assert!(
            v9_errors.is_empty(),
            "expected no V9 errors when V7 already fired (both set), got: {:?}",
            v9_errors
        );
        let v7_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V7"))
            .collect();
        assert_eq!(
            v7_errors.len(),
            1,
            "expected exactly one V7 error, got: {:?}",
            v7_errors
        );
    }

    #[test]
    fn vocabulary_v7_both_lifecycle_and_ref_produces_error_memory_store() {
        // Cross-store variant: same semantic as vocabulary_v7_both_lifecycle_and_ref_produces_error
        // but uses MemoryStore::with_type() to confirm the check runs against the in-memory package.
        // The manifest.json text is added to the data map so validate_repository's load_text_file
        // call succeeds; the typed manifest in self.manifest remains the empty-container default
        // (fires I-79, which is fine — we only assert that V7 is present).
        let type_id = "00000000-0000-4000-8000-000000000040";
        let lc_id = "00000000-0000-4000-8000-000000000050";

        let type_json = minimal_type_json_with_both_lifecycle_fields(type_id, lc_id);
        let record_type: srs_core::types::record_type::RecordType =
            serde_json::from_value(type_json).unwrap();
        let manifest_str = serde_json::to_string(&minimal_manifest(json!([]))).unwrap();
        let store = MemoryStore::with_type(record_type)
            .with_data("manifest.json", serde_json::Value::String(manifest_str));

        let report = validate_repository(&store).unwrap();
        let v7_error = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("V7")
                && d.message.contains("both")
        });
        assert!(
            v7_error.is_some(),
            "expected V7 mutual-exclusion error (MemoryStore), got: {:?}",
            report.diagnostics
        );
    }

    // --- V9: inline TypeLifecycle structural integrity ---

    #[test]
    fn vocabulary_v9_inline_no_initial_state_produces_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(
                type_id,
                json!({"states": [{"key": "draft"}, {"key": "active"}], "transitions": [], "initialState": "draft"}),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("no initial state")
        });
        assert!(
            err.is_some(),
            "expected V9 error for inline lifecycle with no isInitial state, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v9_inline_multiple_initial_states_produces_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(
                type_id,
                json!({"states": [{"key": "draft", "isInitial": true}, {"key": "active", "isInitial": true}], "transitions": [], "initialState": "draft"}),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("initial state")
        });
        assert!(
            err.is_some(),
            "expected V9 error for inline lifecycle with multiple isInitial states, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v9_inline_unknown_transition_state_produces_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(
                type_id,
                json!({
                    "states": [{"key": "draft", "isInitial": true}, {"key": "active"}],
                    "transitions": [{"name": "promote", "from": "draft", "to": "ghost"}],
                    "initialState": "draft"
                }),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("ghost"));
        assert!(
            err.is_some(),
            "expected V9 error for transition to undefined state 'ghost', got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v9_inline_initial_state_mismatch_produces_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(
                type_id,
                // initialState says "active" but the isInitial state is "draft"
                json!({
                    "states": [{"key": "draft", "isInitial": true}, {"key": "active"}],
                    "transitions": [],
                    "initialState": "active"
                }),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("initialState")
                && d.message.contains("isInitial")
        });
        assert!(
            err.is_some(),
            "expected V9 initialState/isInitial mismatch error, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v9_inline_valid_no_error() {
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000040";

        setup_package_only_repo(
            &temp,
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(
                type_id,
                json!({
                    "states": [{"key": "draft", "isInitial": true}, {"key": "active"}],
                    "transitions": [{"name": "promote", "from": "draft", "to": "active"}],
                    "initialState": "draft"
                }),
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let lifecycle_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && (d.message.contains("V7") || d.message.contains("V9"))
            })
            .collect();
        assert!(
            lifecycle_errors.is_empty(),
            "expected no lifecycle errors for valid inline lifecycle, got: {:?}",
            lifecycle_errors
        );
    }

    // --- V8: record lifecycleState key validation ---

    fn setup_repo_with_inline_lifecycle_record(
        temp: &TempDir,
        lifecycle_state: &str,
        lifecycle_json: Value,
    ) {
        let record_id = "00000000-0000-4000-8000-000000000060";
        let type_id = "00000000-0000-4000-8000-000000000061";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/my-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json_with_inline_lifecycle(type_id, lifecycle_json),
        );
        write_json(
            temp.path(),
            "records/my-record.json",
            &minimal_record_with_lifecycle_state(record_id, type_id, lifecycle_state),
        );
    }

    #[test]
    fn record_v8_invalid_lifecycle_state_produces_error() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_inline_lifecycle_record(
            &temp,
            "nonexistent",
            json!({"states": [{"key": "draft", "isInitial": true}], "transitions": [], "initialState": "draft"}),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v8_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V8"));
        assert!(
            v8_error.is_some(),
            "expected V8 error for invalid lifecycleState key, got: {:?}",
            report.diagnostics
        );
    }

    // --- RFC-022: at-rest requiresRelation check ---

    fn rfc022_lifecycle_json() -> Value {
        json!({
            "states": [
                {"key": "draft", "isInitial": true},
                {"key": "superseded", "isFinal": true,
                 "requiresRelation": {"relationType": "supersedes"}}
            ],
            "transitions": [{"name": "supersede", "from": "draft", "to": "superseded"}],
            "initialState": "draft"
        })
    }

    #[test]
    fn rfc022_relational_state_without_relation_produces_warning() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_inline_lifecycle_record(&temp, "superseded", rfc022_lifecycle_json());

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let warn = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Warning
                && d.message.contains("LIFECYCLE_RELATION_UNSATISFIED")
        });
        assert!(
            warn.is_some(),
            "expected RFC-022 warning for orphan relational state, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc022_relational_state_with_satisfying_relation_no_warning() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_inline_lifecycle_record(&temp, "superseded", rfc022_lifecycle_json());
        // Incoming supersedes edge: successor → this record.
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                "relations": [{
                    "relationId": "00000000-0000-4000-8000-0000000000aa",
                    "relationType": "supersedes",
                    "sourceInstanceId": "00000000-0000-4000-8000-000000000099",
                    "targetInstanceId": "00000000-0000-4000-8000-000000000060"
                }]
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let warn = report
            .diagnostics
            .iter()
            .find(|d| d.message.contains("LIFECYCLE_RELATION_UNSATISFIED"));
        assert!(
            warn.is_none(),
            "expected no RFC-022 warning when the obligation is satisfied, got: {:?}",
            warn
        );
    }

    #[test]
    fn record_v8_valid_lifecycle_state_no_error() {
        let temp = TempDir::new().unwrap();
        setup_repo_with_inline_lifecycle_record(
            &temp,
            "draft",
            json!({"states": [{"key": "draft", "isInitial": true}, {"key": "active"}], "transitions": [], "initialState": "draft"}),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v8_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V8"))
            .collect();
        assert!(
            v8_errors.is_empty(),
            "expected no V8 errors for valid lifecycleState, got: {:?}",
            v8_errors
        );
    }

    #[test]
    fn record_v8_no_lifecycle_skips_check() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000000060";
        let type_id = "00000000-0000-4000-8000-000000000061";

        // Type has no lifecycle at all — V8 should not fire even with a lifecycleState on the record
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/my-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json_full(&[], &["types/test-type.json"], &[], &[]),
        );
        // Use plain minimal_type_json — no lifecycle
        write_json(
            temp.path(),
            "package/types/test-type.json",
            &minimal_type_json(type_id),
        );
        write_json(
            temp.path(),
            "records/my-record.json",
            &minimal_record_with_lifecycle_state(record_id, type_id, "active"),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let v8_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("V8"))
            .collect();
        assert!(
            v8_errors.is_empty(),
            "expected no V8 errors when type has no lifecycle, got: {:?}",
            v8_errors
        );
    }

    #[test]
    fn live_srs_repo_validates_cleanly() {
        let repo_root = srs_spec_repo();
        if !repo_root.join("manifest.json").exists() {
            println!("Skipping: live repo not found");
            return;
        }
        let store = crate::store::FileStore::new(&repo_root);
        let report = validate_repository(&store).unwrap();
        if !report.is_ok() {
            for d in &report.diagnostics {
                if d.severity == DiagnosticSeverity::Error {
                    println!("ERROR [{}]: {}", d.relative_path, d.message);
                }
            }
        }
        assert!(
            report.is_ok(),
            "live srs repo has {} schema errors",
            report.summary.errors
        );
    }

    // ── RFC-009 root-type anchor diagnostics (I-63, I-64) ────────────────────

    #[test]
    fn validate_flags_unresolved_root_type_ref() {
        // I-63: a DocumentView rootTypeRefs entry that does not resolve to a package
        // Type produces a Warning; the repository stays valid (is_ok).
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d1",
                "namespace": "com.test",
                "name": "dv",
                "version": 1,
                "description": "test doc view",
                "rootTypeRefs": [{
                    "typeId": "00000000-0000-4000-8000-0000000dead0",
                    "typeVersion": 1
                }],
                "sections": [{
                    "sectionId": "s1",
                    "order": 0,
                    "source": {"type": "fixed-instances", "instanceIds": []}
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "I-63 is advisory; repo must stay ok: {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("I-63") && d.severity == DiagnosticSeverity::Warning),
            "expected an I-63 warning, got {:?}",
            report.diagnostics
        );
    }

    // ── `[N+1]` / ext:views-l2 titleFieldId eligibility diagnostic ───────────

    #[test]
    fn validate_flags_ineligible_title_field_id() {
        // Owner decision (srs PR #341, 2026-08-02): an authored-but-ineligible
        // titleFieldId gets a validation diagnostic (this test), on top of the
        // render-time heading omission covered separately in render_service.rs.
        // CC-33: no first-party repo has one of these, so this must be constructed.
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/f1.json"],
                "types": ["types/t1.json"],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f1",
                "namespace": "com.test",
                "name": "closed-field",
                "version": 1,
                // `valueDomain: closed` fails `[N+1]`'s allow-list — the predicate
                // requires absent/open.
                "fieldType": {"datatype": "string", "valueDomain": "closed"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/types/t1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000e1",
                "namespace": "com.test",
                "name": "test-type",
                "version": 1,
                "description": "Test type",
                "fields": [{
                    "fieldId": "00000000-0000-4000-8000-0000000000f1",
                    "order": 0,
                    "required": false
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d2",
                "namespace": "com.test",
                "name": "dv",
                "version": 1,
                "description": "test doc view",
                "sections": [{
                    "sectionId": "s1",
                    "order": 0,
                    "source": {
                        "type": "type-query",
                        "semanticObjectType": "com.test/test-type"
                    },
                    "titleFieldId": "00000000-0000-4000-8000-0000000000f1"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "[N+1] is advisory; repo must stay ok: {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("[N+1]")
                    && d.message.contains("test-type")
                    && d.severity == DiagnosticSeverity::Warning),
            "expected an [N+1] warning naming the resolved type, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_silent_for_eligible_title_field_id() {
        // Same shape as `validate_flags_ineligible_title_field_id` but the
        // titleFieldId is an ordinary open-domain string field — must not fire.
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/f1.json"],
                "types": ["types/t1.json"],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f1",
                "namespace": "com.test",
                "name": "open-field",
                "version": 1,
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/types/t1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000e1",
                "namespace": "com.test",
                "name": "test-type",
                "version": 1,
                "description": "Test type",
                "fields": [{
                    "fieldId": "00000000-0000-4000-8000-0000000000f1",
                    "order": 0,
                    "required": false
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d2",
                "namespace": "com.test",
                "name": "dv",
                "version": 1,
                "description": "test doc view",
                "sections": [{
                    "sectionId": "s1",
                    "order": 0,
                    "source": {
                        "type": "type-query",
                        "semanticObjectType": "com.test/test-type"
                    },
                    "titleFieldId": "00000000-0000-4000-8000-0000000000f1"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("[N+1]")),
            "an eligible titleFieldId must not produce an [N+1] diagnostic, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_flags_ineligible_title_field_id_for_fixed_instances() {
        // srs-rust#795: FixedInstances declares no static candidate Type, so
        // eligibility must be checked against each candidate instance's *actual*
        // Type. Post-#242 (Change-I condition 4) cardinality is the sole
        // mechanism: the field is ineligible via `cardinality: "list"` — the
        // fixture still discriminates the resolve-the-instance's-Type branch.
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000000501";
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([rfc013_instance_entry(record_id)])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/f1.json"],
                "types": ["types/t1.json"],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f1",
                "namespace": "com.test",
                "name": "repeatable_title_field",
                "version": 1,
                // Open string, no format — but list cardinality makes it
                // [N+1]-ineligible (cardinality-only since the #242 cutover).
                "fieldType": {"datatype": "string", "cardinality": "list"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/types/t1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000e1",
                "namespace": "com.test",
                "name": "test-type",
                "version": 1,
                "description": "Test type",
                "fields": [{
                    "fieldId": "00000000-0000-4000-8000-0000000000f1",
                    "order": 0,
                    "required": false
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d2",
                "namespace": "com.test",
                "name": "dv",
                "version": 1,
                "description": "test doc view",
                "sections": [{
                    "sectionId": "s1",
                    "order": 0,
                    "source": {
                        "type": "fixed-instances",
                        "instanceIds": [record_id]
                    },
                    "titleFieldId": "00000000-0000-4000-8000-0000000000f1"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            &format!("records/{record_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": record_id,
                "typeId": "00000000-0000-4000-8000-0000000000e1",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "test-type",
                "fieldValues": {
                    "repeatable_title_field": ["Repeatable Title Value"]
                }
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("[N+1]")
                    && d.message.contains("test-type")
                    && d.severity == DiagnosticSeverity::Warning),
            "expected an [N+1] warning for a FixedInstances section whose only \
             candidate instance resolves to a repeatable-only-ineligible \
             titleFieldId, got {:?}",
            report.diagnostics
        );

        // The render plane must agree: the heading is omitted, not substituted.
        // Read it structurally off the JSON projection's `record_heading` (as
        // `identity_field_id_fallback_record_heading_json` does) rather than
        // string-matching the markdown, since the field's value legitimately still
        // renders as an ordinary body row either way (omit-not-substitute, srs
        // PR #341) — only heading *promotion* is what must not happen.
        let result = crate::render_service::render_document_view(
            crate::render_service::RenderDocumentViewOptions {
                store: &store,
                view_id: "00000000-0000-4000-8000-0000000000d2",
                format: Some("json"),
                theme_variant: None,
                container_id: None,
                instance_id_filter: None,
            },
        )
        .expect("render should succeed");
        let projection = result
            .projection
            .expect("json format should produce a projection");
        assert_eq!(
            projection.sections[0].records[0].record_heading, None,
            "the render plane must omit the heading for the same list-cardinality \
             ineligible titleFieldId the validation plane now warns about"
        );
    }

    #[test]
    fn validate_flags_ineligible_title_field_id_for_relation_query() {
        // srs-rust#795: same coverage gap as the FixedInstances fixture above, for
        // RelationQuery — its candidate instances are only known by resolving the
        // relations graph, which the validator did not do before this fix.
        let temp = TempDir::new().unwrap();
        let from_id = "00000000-0000-4000-8000-000000000601";
        let target_id = "00000000-0000-4000-8000-000000000602";
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                rfc013_instance_entry(from_id),
                rfc013_instance_entry(target_id)
            ])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/f1.json"],
                "types": ["types/t1.json"],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f1",
                "namespace": "com.test",
                "name": "repeatable_title_field",
                "version": 1,
                "fieldType": {"datatype": "string", "cardinality": "list"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/types/t1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000e1",
                "namespace": "com.test",
                "name": "test-type",
                "version": 1,
                "description": "Test type",
                "fields": [{
                    "fieldId": "00000000-0000-4000-8000-0000000000f1",
                    "order": 0,
                    "required": false
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d2",
                "namespace": "com.test",
                "name": "dv",
                "version": 1,
                "description": "test doc view",
                "sections": [{
                    "sectionId": "s1",
                    "order": 0,
                    "source": {
                        "type": "relation-query",
                        "fromInstanceId": from_id,
                        "relationType": "depends-on",
                        "direction": "forward"
                    },
                    "titleFieldId": "00000000-0000-4000-8000-0000000000f1"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            &format!("records/{from_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": from_id,
                "typeId": "00000000-0000-4000-8000-0000000000e1",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "test-type",
                "fieldValues": {}
            }),
        );
        write_json(
            temp.path(),
            &format!("records/{target_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": target_id,
                "typeId": "00000000-0000-4000-8000-0000000000e1",
                "typeVersion": 1,
                "typeNamespace": "com.test",
                "typeName": "test-type",
                "fieldValues": {
                    "repeatable_title_field": ["Related Title Value"]
                }
            }),
        );
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
                "relations": [{
                    "relationId": "00000000-0000-4000-8000-000000000699",
                    "sourceInstanceId": from_id,
                    "targetInstanceId": target_id,
                    "relationType": "depends-on",
                    "createdAt": "2026-01-01T00:00:00Z"
                }]
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("[N+1]")
                    && d.message.contains("test-type")
                    && d.severity == DiagnosticSeverity::Warning),
            "expected an [N+1] warning for a RelationQuery section whose resolved \
             target instance has a list-cardinality-ineligible titleFieldId, got {:?}",
            report.diagnostics
        );

        let result = crate::render_service::render_document_view(
            crate::render_service::RenderDocumentViewOptions {
                store: &store,
                view_id: "00000000-0000-4000-8000-0000000000d2",
                format: Some("json"),
                theme_variant: None,
                container_id: None,
                instance_id_filter: None,
            },
        )
        .expect("render should succeed");
        let projection = result
            .projection
            .expect("json format should produce a projection");
        assert_eq!(
            projection.sections[0].records[0].record_heading, None,
            "the render plane must omit the heading for the same list-cardinality \
             ineligible titleFieldId the validation plane now warns about"
        );
    }

    // ── I-92/94/95/96 cross-field-validation, Type-level enforcement ─────────

    #[test]
    fn validate_flags_misconfigured_cross_field_rule_with_zero_records() {
        // Owner-confirmed ordinary implementation work (issue #790 dispatch): I-94's
        // predicate-field eligibility is a Type-level invariant, and per-record
        // enforcement alone never sees a Type with zero Records. No record is
        // written anywhere in this fixture — the repository has none at all.
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/f1.json", "fields/f2.json"],
                "types": ["types/t1.json"],
                "views": [],
                "documentViews": []
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f1",
                "namespace": "com.test",
                "name": "repeat-field",
                "version": 1,
                "fieldType": {"datatype": "string", "cardinality": "list"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f2.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f2",
                "namespace": "com.test",
                "name": "target-field",
                "version": 1,
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/types/t1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000e1",
                "namespace": "com.test",
                "name": "test-type",
                "version": 1,
                "description": "Test type",
                "fields": [
                    {"fieldId": "00000000-0000-4000-8000-0000000000f1", "order": 0, "required": false},
                    {"fieldId": "00000000-0000-4000-8000-0000000000f2", "order": 1, "required": false}
                ],
                // I-94: a list-cardinality predicate field is not
                // effective-single (cardinality-only since the #242 cutover) —
                // ineligible as a conditional-required predicate.
                "validationRules": [{
                    "type": "conditional-required",
                    "predicateFieldId": "00000000-0000-4000-8000-0000000000f1",
                    "predicateValue": "yes",
                    "targetFieldId": "00000000-0000-4000-8000-0000000000f2"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            !report.is_ok(),
            "I-94 misconfiguration is a MUST — expected the repository to be invalid, got {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("I-92/94/95/96")
                    && d.message.contains("test-type")
                    && d.severity == DiagnosticSeverity::Error),
            "expected an I-92/94/95/96 error naming the Type, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_silent_for_well_configured_cross_field_rule_with_zero_records() {
        // Same shape, eligible predicate field — the Type-level pass must not
        // fire on a well-formed rule just because it owns zero Records.
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/f1.json", "fields/f2.json"],
                "types": ["types/t1.json"],
                "views": [],
                "documentViews": []
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f1",
                "namespace": "com.test",
                "name": "predicate-field",
                "version": 1,
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/fields/f2.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000f2",
                "namespace": "com.test",
                "name": "target-field",
                "version": 1,
                "fieldType": {"datatype": "string"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "package/types/t1.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000e1",
                "namespace": "com.test",
                "name": "test-type",
                "version": 1,
                "description": "Test type",
                "fields": [
                    {"fieldId": "00000000-0000-4000-8000-0000000000f1", "order": 0, "required": false},
                    {"fieldId": "00000000-0000-4000-8000-0000000000f2", "order": 1, "required": false}
                ],
                "validationRules": [{
                    "type": "conditional-required",
                    "predicateFieldId": "00000000-0000-4000-8000-0000000000f1",
                    "predicateValue": "yes",
                    "targetFieldId": "00000000-0000-4000-8000-0000000000f2"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "a well-configured rule must not be flagged merely for owning zero records, got {:?}",
            report.diagnostics
        );
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("I-92/94/95/96")),
            "expected no I-92/94/95/96 diagnostic, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_flags_dangling_document_view_container_ref() {
        // #509: a document-view section whose containerId does not resolve to a
        // Container produces a Warning; the repository stays valid (is_ok).
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d2",
                "namespace": "com.test",
                "name": "dv-dangling",
                "version": 1,
                "description": "doc view with a dangling container reference",
                "sections": [{
                    "sectionId": "broken",
                    "order": 0,
                    "source": {
                        "type": "container-subset",
                        "containerId": "00000000-0000-4000-8000-0000000dead0"
                    },
                    "emptyBehavior": "hide"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "dangling container ref is advisory; repo must stay ok: {:?}",
            report.diagnostics
        );
        assert!(
            report.diagnostics.iter().any(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("00000000-0000-4000-8000-0000000000d2")
                    && d.message.contains("broken")
                    && d.message.contains("00000000-0000-4000-8000-0000000dead0")
                    && d.message.contains("does not resolve to a Container")
            }),
            "expected a dangling document-view container warning, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_document_view_embed_only_root_container_ref_is_not_dangling() {
        // #744: a document-view section referencing the RFC-013 embed-only root
        // container (present only in manifest.container, no containerIndex entry)
        // must resolve via the same embed-fallback every other container operation
        // uses — it must NOT be reported as a dangling container reference.
        let temp = TempDir::new().unwrap();
        // minimal_manifest() embeds root container "...099" with no containerIndex
        // entry and no container file — this IS the embed-only case.
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000011",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "views": [],
                "documentViews": ["document-views/dv.json"]
            }),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &json!({
                "id": "00000000-0000-4000-8000-0000000000d3",
                "namespace": "com.test",
                "name": "dv-embed-root",
                "version": 1,
                "description": "doc view referencing the embed-only root container",
                "sections": [{
                    "sectionId": "root-section",
                    "order": 0,
                    "source": {
                        "type": "container-subset",
                        "containerId": "00000000-0000-4000-8000-000000000099"
                    },
                    "emptyBehavior": "hide"
                }],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "embed-only root container ref must validate clean: {:?}",
            report.diagnostics
        );
        assert!(
            !report.diagnostics.iter().any(|d| {
                d.message.contains("00000000-0000-4000-8000-000000000099")
                    && d.message.contains("does not resolve to a Container")
            }),
            "embed-only root container must not be reported as dangling: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_flags_stale_container_type_hint() {
        // I-64: containerType that does not equal the resolved root Type's bare name
        // produces a Warning; the container (and repo) remain valid.
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000abc";
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": ["types/guide.json"],
                "views": [],
                "documentViews": []
            }),
        );
        write_json(
            temp.path(),
            "package/types/guide.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "guide",
                "version": 1,
                "description": "guide type",
                "fields": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        // Create a tier-2 record of the guide type via the service (keeps index valid).
        let record = crate::record_store::create_record(
            &store,
            type_id,
            1,
            srs_core::types::record::FieldValues::new(),
            None,
            None,
        )
        .unwrap();
        // Container rooted in that record, but with a stale containerType hint.
        let container = srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c1".to_string(),
            title: "Guide container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: Some("not-guide".to_string()),
            identity_instance_id: None,
            root_instance_ids: Some(vec![record.instance_id.clone()]),
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        crate::container_service::create_container(&store, container).unwrap();

        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "I-64 mismatch is a warning; repo must stay ok: {:?}",
            report.diagnostics
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("I-64") && d.severity == DiagnosticSeverity::Warning),
            "expected an I-64 warning, got {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_skips_container_type_without_roots() {
        // A Container carrying containerType but no rootInstanceIds must not trigger I-64.
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "views": [],
                "documentViews": []
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let container = srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c2".to_string(),
            title: "Unrooted container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: Some("guide".to_string()),
            identity_instance_id: None,
            root_instance_ids: None,
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        crate::container_service::create_container(&store, container).unwrap();

        let report = validate_repository(&store).unwrap();
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("I-64")),
            "I-64 must not fire for a container without rootInstanceIds: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_skips_i64_when_root_record_unresolved() {
        // I-64 must skip (not error) when the first rootInstanceId cannot be loaded.
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": [],
                "views": [],
                "documentViews": []
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let container = srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c3".to_string(),
            title: "Dangling-root container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: Some("guide".to_string()),
            identity_instance_id: None,
            // Root id that is not present in the manifest index.
            root_instance_ids: Some(vec!["99999999-9999-4999-8999-999999999999".to_string()]),
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        crate::container_service::create_container(&store, container).unwrap();

        let report = validate_repository(&store).unwrap();
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.message.contains("I-64")),
            "I-64 must skip when the root Record is unresolvable: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_root_type_diagnostics_consistent_across_stores() {
        // Cross-store roundtrip: the same fixture must produce the same I-64 diagnostic
        // from FileStore and from a JsonStore reconstructed via snapshot import.
        let temp = TempDir::new().unwrap();
        let type_id = "00000000-0000-4000-8000-000000000abc";
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "title": "Test Package",
                "description": "test package",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": [],
                "types": ["types/guide.json"],
                "views": [],
                "documentViews": []
            }),
        );
        write_json(
            temp.path(),
            "package/types/guide.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "guide",
                "version": 1,
                "description": "guide type",
                "fields": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let file_store = crate::store::FileStore::new(temp.path());
        let record = crate::record_store::create_record(
            &file_store,
            type_id,
            1,
            srs_core::types::record::FieldValues::new(),
            None,
            None,
        )
        .unwrap();
        let container = srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c4".to_string(),
            title: "Guide container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: Some("not-guide".to_string()),
            identity_instance_id: None,
            root_instance_ids: Some(vec![record.instance_id.clone()]),
            member_instance_ids: None,
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        crate::container_service::create_container(&file_store, container).unwrap();

        // Reconstruct the same repository in a JsonStore via snapshot import.
        let snapshot =
            crate::repository_portability::export_repository_snapshot(&file_store).unwrap();
        let tmp2 = TempDir::new().unwrap();
        let json_store =
            crate::json_store::JsonStore::create(tmp2.path().join("repo.srsj")).unwrap();
        crate::repository_portability::import_repository_snapshot(&json_store, &snapshot).unwrap();

        let count_i64 = |r: &RepositoryValidationReport| {
            r.diagnostics
                .iter()
                .filter(|d| d.message.contains("I-64"))
                .count()
        };
        let file_report = validate_repository(&file_store).unwrap();
        let json_report = validate_repository(&json_store).unwrap();
        assert_eq!(
            count_i64(&file_report),
            1,
            "FileStore: {:?}",
            file_report.diagnostics
        );
        assert_eq!(
            count_i64(&json_report),
            count_i64(&file_report),
            "I-64 diagnostics must be store-agnostic (json: {:?})",
            json_report.diagnostics
        );
    }

    fn manifest_store(manifest_json: serde_json::Value) -> MemoryStore {
        let raw = serde_json::to_string(&manifest_json).unwrap();
        let store = MemoryStore::empty().with_data("manifest.json", serde_json::Value::String(raw));
        if let Ok(typed) =
            serde_json::from_value::<crate::manifest::Manifest>(manifest_json.clone())
        {
            let mut m = store.load_manifest().unwrap();
            m.container = typed.container;
            m.container_index = typed.container_index;
            store.save_manifest(&m).unwrap();
        }
        store
    }

    #[test]
    fn test_validate_manifest_missing_title() {
        let mut manifest = minimal_manifest(json!([]));
        manifest.as_object_mut().unwrap().remove("title");
        let store = manifest_store(manifest);
        let report = validate_repository(&store).unwrap();
        let manifest_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.relative_path == "manifest.json" && d.severity == DiagnosticSeverity::Error
            })
            .collect();
        assert!(
            !manifest_errors.is_empty(),
            "expected ERROR diagnostic for manifest.json (missing title), got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn test_validate_manifest_extra_property() {
        let mut manifest = minimal_manifest(json!([]));
        manifest
            .as_object_mut()
            .unwrap()
            .insert("name".to_string(), json!("should-not-be-here"));
        let store = manifest_store(manifest);
        let report = validate_repository(&store).unwrap();
        let manifest_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.relative_path == "manifest.json" && d.severity == DiagnosticSeverity::Error
            })
            .collect();
        assert!(
            !manifest_errors.is_empty(),
            "expected ERROR diagnostic for manifest.json (undeclared property), got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn unstamped_repository_warns_and_names_the_migration() {
        // RFC-033 [R6]: a revision-0 repository still loads (definitions are
        // upgraded in memory), so this is a warning — but it must say which
        // command clears it, or the stamp is just noise.
        let mut manifest = minimal_manifest(json!([]));
        manifest
            .as_object_mut()
            .unwrap()
            .remove("dataModelRevision");
        let store = manifest_store(manifest);
        let report = validate_repository(&store).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.message.contains("data-model revision"))
            .expect("an unstamped repository must be reported");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
        // The exact invocation, not an approximation of it: an actionable
        // diagnostic that names a command the CLI cannot parse is worse than
        // no diagnostic, because the reader trusts it.
        assert!(
            d.message
                .contains("srs repo apply-migration --id field-type"),
            "the diagnostic must name the runnable command: {}",
            d.message
        );
    }

    #[test]
    fn repository_from_a_newer_build_is_an_error_not_a_mystery() {
        // The case RFC-033 introduced the stamp for: without it, a
        // newer-than-supported repository surfaces as an unexplained load
        // failure instead of "upgrade your binary".
        let mut manifest = minimal_manifest(json!([]));
        manifest
            .as_object_mut()
            .unwrap()
            .insert("dataModelRevision".to_string(), json!(99));
        let store = manifest_store(manifest);
        let report = validate_repository(&store).unwrap();
        let d = report
            .diagnostics
            .iter()
            .find(|d| d.message.contains("data-model revision"))
            .expect("a newer-revision repository must be reported");
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert!(
            d.message.contains("Upgrade the `srs` binary"),
            "{}",
            d.message
        );
    }

    #[test]
    fn test_validate_manifest_valid() {
        let store = manifest_store(minimal_manifest(json!([])));
        let report = validate_repository(&store).unwrap();
        let manifest_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.relative_path == "manifest.json")
            .collect();
        assert!(
            manifest_diags.is_empty(),
            "expected zero manifest.json diagnostics for valid manifest, got: {:?}",
            manifest_diags
        );
    }

    // ---- RFC-013 root container invariant tests ----

    fn rfc013_container(
        id: &str,
        members: &[&str],
        roots: &[&str],
    ) -> srs_core::types::container::Container {
        srs_core::types::container::Container {
            container_id: id.to_string(),
            title: "Root Container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: None,
            member_instance_ids: if members.is_empty() {
                None
            } else {
                Some(members.iter().map(|s| s.to_string()).collect())
            },
            root_instance_ids: if roots.is_empty() {
                None
            } else {
                Some(roots.iter().map(|s| s.to_string()).collect())
            },
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn rfc013_instance_entry(id: &str) -> serde_json::Value {
        json!({"instanceId": id, "tier": 2, "path": format!("records/{id}.json")})
    }

    #[test]
    fn rfc013_i79_missing_container_errors() {
        let mut manifest = minimal_manifest(json!([]));
        manifest.as_object_mut().unwrap().remove("container");
        let store = manifest_store(manifest);
        let report = validate_repository(&store).unwrap();
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("I-79"))
            .collect();
        assert!(
            !errors.is_empty(),
            "expected I-79 error when manifest.container absent, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i80_member_not_in_instance_index_errors() {
        let temp = TempDir::new().unwrap();
        let root_id = "00000000-0000-4000-8000-000000000100";
        let member_id = "00000000-0000-4000-8000-000000000101";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container —
        // a separate `containers/root.json` with the same id would be a duplicate
        // ([R12]). memberInstanceIds is set directly on the embed.
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Test I-80",
            "container": {"containerId": root_id, "title": "Root", "memberInstanceIds": [member_id]},
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        // RFC-038 [R25]: the catalog's own [R13] dangling-reference check now catches
        // this before validate_repository's separate I-80 pass ever runs (resolving
        // the root container fails fatally first) — either diagnostic proves the point.
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && (d.message.contains("I-80") || d.message.contains("SRS038-R13"))
            })
            .collect();
        assert!(
            !errors.is_empty(),
            "expected I-80 error when memberInstanceId not in instanceIndex, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i80_root_not_in_instance_index_errors() {
        let temp = TempDir::new().unwrap();
        let root_id = "00000000-0000-4000-8000-000000000200";
        let root_member_id = "00000000-0000-4000-8000-000000000201";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container.
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Test I-80 root",
            "container": {"containerId": root_id, "title": "Root", "rootInstanceIds": [root_member_id]},
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        // RFC-038 [R25]: see the memberInstanceId case above for why either
        // diagnostic proves the point.
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && (d.message.contains("I-80") || d.message.contains("SRS038-R13"))
            })
            .collect();
        assert!(
            !errors.is_empty(),
            "expected I-80 error when rootInstanceId not in instanceIndex, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i81_identity_not_in_root_or_members_errors() {
        let temp = TempDir::new().unwrap();
        let root_id = "00000000-0000-4000-8000-000000000300";
        let identity_id = "00000000-0000-4000-8000-000000000301";
        let member_id = "00000000-0000-4000-8000-000000000302";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container.
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Test I-81 fail",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [member_id],
                "rootInstanceIds": [member_id]
            },
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            &format!("records/{member_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": member_id,
                "typeId": "t1",
                "typeVersion": 1,
                "typeNamespace": "ns",
                "typeName": "Section",
                "fieldValues": {}
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("I-81"))
            .collect();
        assert!(
            !errors.is_empty(),
            "expected I-81 error when identity not in root or members, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i81_identity_in_root_instance_ids_ok() {
        let temp = TempDir::new().unwrap();
        let root_id = "00000000-0000-4000-8000-000000000400";
        let identity_id = "00000000-0000-4000-8000-000000000401";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container.
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Test I-81 ok via root",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id],
                "rootInstanceIds": [identity_id]
            },
            "instanceIndex": [rfc013_instance_entry(identity_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            &format!("records/{identity_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": identity_id,
                "typeId": "t1",
                "typeVersion": 1,
                "typeNamespace": "ns",
                "typeName": "Identity",
                "fieldValues": {}
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let i81_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-013 I-81")
            })
            .collect();
        assert!(
            i81_errors.is_empty(),
            "expected no RFC-013 I-81 errors when identity in rootInstanceIds, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i81_identity_in_member_instance_ids_ok() {
        let temp = TempDir::new().unwrap();
        let root_id = "00000000-0000-4000-8000-000000000500";
        let identity_id = "00000000-0000-4000-8000-000000000501";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container.
        // Identity only in memberInstanceIds, not rootInstanceIds.
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Test I-81 ok via member",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id]
            },
            "instanceIndex": [rfc013_instance_entry(identity_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            &format!("records/{identity_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": identity_id,
                "typeId": "t1",
                "typeVersion": 1,
                "typeNamespace": "ns",
                "typeName": "Identity",
                "fieldValues": {}
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let i81_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-013 I-81")
            })
            .collect();
        assert!(
            i81_errors.is_empty(),
            "expected no RFC-013 I-81 errors when identity in memberInstanceIds, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i81_identity_not_in_root_or_members_memory_store() {
        // Same semantic as rfc013_i81_identity_not_in_root_or_members_errors but uses
        // MemoryStore via manifest_store + with_data (no on-disk files required).
        let root_id = "00000000-0000-4000-8000-000000000305";
        let identity_id = "00000000-0000-4000-8000-000000000306";
        let member_id = "00000000-0000-4000-8000-000000000307";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container.
        // Container has member_id in memberInstanceIds but NOT identity_id.
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "I-81 MemoryStore test",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [member_id],
                "rootInstanceIds": [member_id]
            },
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let store = manifest_store(manifest_val);

        let report = validate_repository(&store).unwrap();
        // RFC-038 [R25]: either the I-81 label or the catalog's own [R13]
        // dangling-reference diagnostic proves the same violation.
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && (d.message.contains("I-81") || d.message.contains("SRS038-R13"))
            })
            .collect();
        assert!(
            !errors.is_empty(),
            "expected I-81 error when identity not in root or members (MemoryStore), got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i82_suppressed_when_container_index_absent() {
        // containerIndex absent (or empty) → I-82 warnings suppressed even when a member
        // has no corresponding section container.
        let root_id = "00000000-0000-4000-8000-000000000600";
        let member_id = "00000000-0000-4000-8000-000000000601";
        let root_container = rfc013_container(root_id, &[member_id], &[member_id]);

        // manifest_store sets up both the text "manifest.json" and the typed manifest.
        // Passing empty containerIndex ([]) triggers the suppression path (ci.is_empty()).
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "I-82 suppressed test",
            "container": {"containerId": root_id, "title": "Root"},
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let store = manifest_store(manifest_val).with_data(
            &format!("containers/{root_id}.json"),
            serde_json::to_value(root_container).unwrap(),
        );

        let report = validate_repository(&store).unwrap();
        let i82_warnings: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-82"))
            .collect();
        assert!(
            i82_warnings.is_empty(),
            "expected no I-82 warnings when containerIndex absent, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_i82_member_not_rooting_section_container_warns() {
        // containerIndex present and non-empty, but member doesn't root any section container → I-82 warning.
        let root_id = "00000000-0000-4000-8000-000000000700";
        let member_id = "00000000-0000-4000-8000-000000000701";
        let section_container_id = "00000000-0000-4000-8000-000000000702";
        let other_id = "00000000-0000-4000-8000-000000000703";

        // Section container roots other_id, not member_id
        let section_container = rfc013_container(section_container_id, &[other_id], &[other_id]);

        // RFC-038 [R1]: the manifest embed is authoritative for the root container.
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "I-82 warn test",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "memberInstanceIds": [member_id],
                "rootInstanceIds": [member_id]
            },
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        // Use manifest_store + with_data to insert the section container file and the
        // real (RFC-038 [R1]/[R13]: catalog-discovered) instance files it and the root
        // container reference, without polluting the typed manifest.
        let store = manifest_store(manifest_val)
            .with_data(
                &format!("containers/{section_container_id}.json"),
                serde_json::to_value(section_container).unwrap(),
            )
            .with_data(
                &format!("records/{member_id}.json"),
                json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": member_id,
                    "typeId": "t1", "typeVersion": 1,
                    "typeNamespace": "ns", "typeName": "Section",
                    "fieldValues": {}
                }),
            )
            .with_data(
                &format!("records/{other_id}.json"),
                json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": other_id,
                    "typeId": "t1", "typeVersion": 1,
                    "typeNamespace": "ns", "typeName": "Section",
                    "fieldValues": {}
                }),
            );

        let report = validate_repository(&store).unwrap();
        let i82_warnings: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-82"))
            .collect();
        assert!(
            !i82_warnings.is_empty(),
            "expected I-82 warning when member doesn't root any section container, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn i82_fires_for_root_instance_ids_member() {
        // RFC-013 I-80/R2: membership = memberInstanceIds ∪ rootInstanceIds.
        // A section declared via rootInstanceIds only (not in memberInstanceIds) must also
        // trigger I-82 when it does not root any section container.
        let root_id = "00000000-0000-4000-8000-000000000a00";
        let identity_id = "00000000-0000-4000-8000-000000000a01";
        let section_id = "00000000-0000-4000-8000-000000000a02";
        let section_container_id = "00000000-0000-4000-8000-000000000a03";
        let other_id = "00000000-0000-4000-8000-000000000a04";

        // Section container roots other_id, not section_id
        let section_container = rfc013_container(section_container_id, &[other_id], &[other_id]);

        // RFC-038 [R1]: the manifest embed is authoritative for the root container —
        // section_id in rootInstanceIds only; memberInstanceIds has only identity.
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "I-82 rootInstanceIds test",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id],
                "rootInstanceIds": [section_id]
            },
            "instanceIndex": [
                rfc013_instance_entry(identity_id),
                rfc013_instance_entry(section_id),
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let record_json = |id: &str| {
            json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": id, "typeId": "t1", "typeVersion": 1,
                "typeNamespace": "ns", "typeName": "Section", "fieldValues": {}
            })
        };
        let store = manifest_store(manifest_val)
            .with_data(
                &format!("containers/{section_container_id}.json"),
                serde_json::to_value(section_container).unwrap(),
            )
            .with_data(
                &format!("records/{identity_id}.json"),
                record_json(identity_id),
            )
            .with_data(
                &format!("records/{section_id}.json"),
                record_json(section_id),
            )
            .with_data(&format!("records/{other_id}.json"), record_json(other_id));

        let report = validate_repository(&store).unwrap();
        let i82_warnings: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-82"))
            .collect();
        assert!(
            !i82_warnings.is_empty(),
            "expected I-82 warning for section declared via rootInstanceIds only, got: {:?}",
            report.diagnostics
        );
        assert!(
            i82_warnings.iter().any(|d| d.message.contains(section_id)),
            "I-82 warning must reference the section_id, got: {:?}",
            i82_warnings
        );
    }

    #[test]
    fn i82_fires_for_both_arrays_no_duplicates() {
        // When the same section ID appears in both rootInstanceIds and memberInstanceIds of the
        // root container, exactly one I-82 warning must be emitted (not two).
        let root_id = "00000000-0000-4000-8000-000000000b00";
        let identity_id = "00000000-0000-4000-8000-000000000b01";
        let section_id = "00000000-0000-4000-8000-000000000b02";
        let section_container_id = "00000000-0000-4000-8000-000000000b03";
        let other_id = "00000000-0000-4000-8000-000000000b04";

        let section_container = rfc013_container(section_container_id, &[other_id], &[other_id]);

        // RFC-038 [R1]: the manifest embed is authoritative for the root container —
        // section_id appears in BOTH memberInstanceIds and rootInstanceIds.
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "I-82 dedup test",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id, section_id],
                "rootInstanceIds": [section_id]
            },
            "instanceIndex": [
                rfc013_instance_entry(identity_id),
                rfc013_instance_entry(section_id),
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let record_json = |id: &str| {
            json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": id, "typeId": "t1", "typeVersion": 1,
                "typeNamespace": "ns", "typeName": "Section", "fieldValues": {}
            })
        };
        let store = manifest_store(manifest_val)
            .with_data(
                &format!("containers/{section_container_id}.json"),
                serde_json::to_value(section_container).unwrap(),
            )
            .with_data(
                &format!("records/{identity_id}.json"),
                record_json(identity_id),
            )
            .with_data(
                &format!("records/{section_id}.json"),
                record_json(section_id),
            )
            .with_data(&format!("records/{other_id}.json"), record_json(other_id));

        let report = validate_repository(&store).unwrap();
        let i82_for_section: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-82") && d.message.contains(section_id))
            .collect();
        assert_eq!(
            i82_for_section.len(),
            1,
            "expected exactly one I-82 for section_id (not two), got: {:?}",
            i82_for_section
        );
    }

    #[test]
    fn i82_no_warning_for_root_instance_ids_member_that_roots_a_section_container() {
        // Happy-path: a member declared via rootInstanceIds only that IS the root of a section
        // container in containerIndex must NOT trigger I-82.  This guards against a regression
        // where union_members grows to cover rootInstanceIds but section_container_roots is still
        // only populated from memberInstanceIds.
        let root_id = "00000000-0000-4000-8000-000000000c00";
        let identity_id = "00000000-0000-4000-8000-000000000c01";
        let section_id = "00000000-0000-4000-8000-000000000c02";
        let section_container_id = "00000000-0000-4000-8000-000000000c03";

        // section_id is declared via rootInstanceIds only (not memberInstanceIds).
        let root_container = srs_core::types::container::Container {
            container_id: root_id.to_string(),
            title: "Root".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: None,
            identity_instance_id: Some(identity_id.to_string()),
            member_instance_ids: Some(vec![identity_id.to_string()]),
            root_instance_ids: Some(vec![section_id.to_string()]),
            tags: None,
            created_at: None,
            updated_at: None,
            meta: None,
            extra: std::collections::BTreeMap::new(),
        };
        // Section container that roots section_id.
        let section_container =
            rfc013_container(section_container_id, &[section_id], &[section_id]);

        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "I-82 happy-path test",
            "container": {"containerId": root_id, "title": "Root", "identityInstanceId": identity_id},
            "containerIndex": [
                {"containerId": section_container_id, "title": "Section"}
            ],
            "instanceIndex": [
                rfc013_instance_entry(identity_id),
                rfc013_instance_entry(section_id),
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let store = manifest_store(manifest_val)
            .with_data(
                &format!("containers/{root_id}.json"),
                serde_json::to_value(root_container).unwrap(),
            )
            .with_data(
                &format!("containers/{section_container_id}.json"),
                serde_json::to_value(section_container).unwrap(),
            );

        let report = validate_repository(&store).unwrap();
        let i82_for_section: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-82") && d.message.contains(section_id))
            .collect();
        assert!(
            i82_for_section.is_empty(),
            "expected no I-82 for section_id that roots a section container, got: {:?}",
            i82_for_section
        );
    }

    #[test]
    fn rfc013_embed_only_root_container_broken_embed_trips_i81() {
        // manifest.container present but no container file → the embed itself is the
        // root container (canonical repository-identity source) and IS validated:
        // an identity that is not a member trips I-81 instead of being silently skipped.
        let root_id = "00000000-0000-4000-8000-000000000800";
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Embed Only",
            "container": {"containerId": root_id, "title": "Root", "identityInstanceId": "99999999-9999-4999-8999-999999999999"},
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &manifest);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let i81_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-013 I-81")
            })
            .collect();
        assert!(
            !i81_errors.is_empty(),
            "expected I-81 error for embed-only root whose identity is not a member, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_embed_only_root_container_canonical_embed_validates_clean() {
        // Embed-only root in the canonical shape written by `repo set-root-container`
        // ({containerId, identityInstanceId, memberInstanceIds: [identity], title}) —
        // no container file — must produce no RFC-013 errors and no RFC-018 warnings.
        let root_id = "00000000-0000-4000-8000-000000000800";
        let identity_id = "00000000-0000-4000-8000-000000000801";
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Embed Only",
            "container": {
                "containerId": root_id,
                "title": "Embed Only",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id]
            },
            "instanceIndex": [rfc013_instance_entry(identity_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            &format!("records/{identity_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": identity_id,
                "typeId": "t-purpose",
                "typeVersion": 1,
                "typeNamespace": "com.semanticops.core",
                "typeName": "purpose",
                "fieldValues": {}
            }),
        );
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let rfc_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.message.contains("I-79")
                    || d.message.contains("I-80")
                    || d.message.contains("I-81")
                    || d.message.contains("I-82")
            })
            .collect();
        assert!(
            rfc_diags.is_empty(),
            "expected canonical embed-only root to validate clean, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn rfc013_all_invariants_satisfied_no_diagnostics() {
        let temp = TempDir::new().unwrap();
        let root_id = "00000000-0000-4000-8000-000000000900";
        let identity_id = "00000000-0000-4000-8000-000000000901";
        let section_id = "00000000-0000-4000-8000-000000000902";
        let section_container_id = "00000000-0000-4000-8000-000000000903";

        // RFC-038 [R1]: the manifest embed is authoritative for the root container —
        // identity in rootInstanceIds + memberInstanceIds, section_id also member.
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Full Valid RFC-013 Repo",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id, section_id],
                "rootInstanceIds": [identity_id]
            },
            "instanceIndex": [
                rfc013_instance_entry(identity_id),
                rfc013_instance_entry(section_id)
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);

        // Section container: section_id is root
        let section_container = json!({
            "containerId": section_container_id,
            "title": "Section",
            "memberInstanceIds": [section_id],
            "rootInstanceIds": [section_id]
        });
        write_json(temp.path(), "containers/section.json", &section_container);

        // Write instance records: identity uses com.semanticops.core/purpose so the
        // repo also satisfies RFC-018 I-81. Section uses an arbitrary type.
        write_json(
            temp.path(),
            &format!("records/{identity_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": identity_id,
                "typeId": "t-purpose",
                "typeVersion": 1,
                "typeNamespace": "com.semanticops.core",
                "typeName": "purpose",
                "fieldValues": {}
            }),
        );
        write_json(
            temp.path(),
            &format!("records/{section_id}.json"),
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                "instanceId": section_id,
                "typeId": "t1",
                "typeVersion": 1,
                "typeNamespace": "ns",
                "typeName": "Entity",
                "fieldValues": {}
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let rfc013_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.message.contains("RFC-013 I-79")
                    || d.message.contains("RFC-013 I-80")
                    || d.message.contains("RFC-013 I-81")
                    || d.message.contains("RFC-013 I-82")
            })
            .collect();
        assert!(
            rfc013_diags.is_empty(),
            "expected no RFC-013 diagnostics when all invariants satisfied, got: {:?}",
            rfc013_diags
        );
    }

    #[test]
    fn rfc013_cross_store_file_and_json_agree() {
        // FileStore and JsonStore (from_srsj) must produce the same RFC-013 diagnostic count.
        // Member not in instanceIndex → I-80 error on both stores.
        //
        // RFC-038 [R1]: the manifest embed is authoritative for the root container — no
        // separate `containers/{root_id}.json` file (that would be an [R12] duplicate).
        let root_id = "00000000-0000-4000-8000-000000000a00";
        let member_id = "00000000-0000-4000-8000-000000000a01";
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": root_id,
            "title": "Cross-Store I-80",
            "container": {"containerId": root_id, "title": "Root", "memberInstanceIds": [member_id]},
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });

        // FileStore: write files to disk
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &manifest_val);
        let file_store = crate::store::FileStore::new(temp.path());

        // JsonStore via from_srsj: snapshot doesn't preserve manifest.container so we use from_srsj.
        let srsj = json!({
            "srsj": "1",
            "manifest": manifest_val,
            "data": {}
        });
        let json_store = crate::json_store::JsonStore::from_srsj(&srsj.to_string()).unwrap();

        let file_report = validate_repository(&file_store).unwrap();
        let json_report = validate_repository(&json_store).unwrap();

        // RFC-038 [R25]: either the I-80 label or the catalog's own [R13]
        // dangling-reference diagnostic proves the same violation.
        let is_i80_like = |d: &&ValidationDiagnostic| {
            d.message.contains("I-80") || d.message.contains("SRS038-R13")
        };
        let file_i80: Vec<_> = file_report.diagnostics.iter().filter(is_i80_like).collect();
        let json_i80: Vec<_> = json_report.diagnostics.iter().filter(is_i80_like).collect();

        assert!(
            !file_i80.is_empty(),
            "FileStore: expected I-80 diagnostic, got: {:?}",
            file_report.diagnostics
        );
        assert_eq!(
            file_i80.len(),
            json_i80.len(),
            "FileStore and JsonStore must produce same I-80 count (file: {:?}, json: {:?})",
            file_report.diagnostics,
            json_report.diagnostics
        );
    }

    // ------------------------------------------------------------------ //
    // Blueprint + Protocol definition validation
    // ------------------------------------------------------------------ //

    fn minimal_blueprint_json(id: &str, valid: bool) -> Value {
        if valid {
            json!({
                "id": id,
                "namespace": "com.test",
                "name": "test-blueprint",
                "version": 1,
                "description": "A test blueprint",
                "rootTypes": [{"typeId": "00000000-0000-4000-8000-000000000031", "typeVersion": 1}],
                "createdAt": "2026-01-01T00:00:00Z"
            })
        } else {
            // Empty rootTypes — fails semantic validation (root_types must not be empty)
            json!({
                "id": id,
                "namespace": "com.test",
                "name": "test-blueprint",
                "version": 1,
                "description": "A test blueprint",
                "rootTypes": [],
                "createdAt": "2026-01-01T00:00:00Z"
            })
        }
    }

    fn minimal_protocol_json(id: &str, with_cycle: bool) -> Value {
        if with_cycle {
            json!({
                "protocolId": id,
                "protocolNamespace": "com.test",
                "protocolName": "test-protocol",
                "protocolVersion": 1,
                "protocolTargetType": "00000000-0000-4000-8000-000000000040",
                "protocolCreatedAt": "2026-01-01T00:00:00Z",
                "protocolStages": [
                    {
                        "stageId": "stage-a",
                        "name": "Stage A",
                        "order": 1,
                        "dependsOn": ["stage-b"]
                    },
                    {
                        "stageId": "stage-b",
                        "name": "Stage B",
                        "order": 2,
                        "dependsOn": ["stage-a"]
                    }
                ]
            })
        } else {
            json!({
                "protocolId": id,
                "protocolNamespace": "com.test",
                "protocolName": "test-protocol",
                "protocolVersion": 1,
                "protocolTargetType": "00000000-0000-4000-8000-000000000040",
                "protocolCreatedAt": "2026-01-01T00:00:00Z",
                "protocolStages": [
                    {
                        "stageId": "stage-a",
                        "name": "Stage A",
                        "order": 1,
                        "dependsOn": []
                    }
                ]
            })
        }
    }

    fn setup_repo_with_blueprint(temp: &TempDir, blueprint_path: &str, blueprint_json: &Value) {
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "blueprints": [blueprint_path],
                "protocols": []
            }),
        );
        write_json(
            temp.path(),
            &format!("package/{blueprint_path}"),
            blueprint_json,
        );
    }

    fn setup_repo_with_protocol(temp: &TempDir, protocol_path: &str, protocol_json: &Value) {
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &json!({
                "id": "00000000-0000-4000-8000-000000000010",
                "namespace": "com.test",
                "name": "test-package",
                "version": "1.0.0",
                "fields": [],
                "types": [],
                "blueprints": [],
                "protocols": [protocol_path]
            }),
        );
        write_json(
            temp.path(),
            &format!("package/{protocol_path}"),
            protocol_json,
        );
    }

    #[test]
    fn test_validate_blueprint_valid_passes() {
        let temp = TempDir::new().unwrap();
        let bp_json = minimal_blueprint_json("00000000-0000-4000-8000-000000000030", true);
        setup_repo_with_blueprint(&temp, "blueprints/test-bp.json", &bp_json);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let bp_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.relative_path.contains("blueprints"))
            .collect();
        assert!(
            bp_diags.is_empty(),
            "expected no blueprint diagnostics for valid blueprint, got: {bp_diags:?}"
        );
    }

    #[test]
    fn test_validate_blueprint_semantic_empty_root_types_reports_diagnostic() {
        let temp = TempDir::new().unwrap();
        let bp_json = minimal_blueprint_json("00000000-0000-4000-8000-000000000030", false);
        setup_repo_with_blueprint(&temp, "blueprints/test-bp.json", &bp_json);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let bp_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.relative_path.contains("blueprints") && d.severity == DiagnosticSeverity::Error
            })
            .collect();
        assert!(
            !bp_errors.is_empty(),
            "expected at least one ERROR for blueprint with empty rootTypes, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn test_validate_blueprint_semantic_error_reports_diagnostic() {
        let temp = TempDir::new().unwrap();
        // typeVersion: 0 is a semantic error (must be >= 1)
        let bp_json = json!({
            "id": "00000000-0000-4000-8000-000000000030",
            "namespace": "com.test",
            "name": "test-blueprint",
            "version": 1,
            "description": "A test blueprint",
            "rootTypes": [{"typeId": "00000000-0000-4000-8000-000000000031", "typeVersion": 0}],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        setup_repo_with_blueprint(&temp, "blueprints/test-bp.json", &bp_json);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let bp_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.relative_path.contains("blueprints") && d.severity == DiagnosticSeverity::Error
            })
            .collect();
        assert!(
            !bp_errors.is_empty(),
            "expected ERROR for blueprint with typeVersion=0, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn test_validate_protocol_valid_passes() {
        let temp = TempDir::new().unwrap();
        let proto_json = minimal_protocol_json("proto-valid-id", false);
        setup_repo_with_protocol(&temp, "protocols/test-proto.json", &proto_json);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let proto_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.relative_path.contains("protocols"))
            .collect();
        assert!(
            proto_diags.is_empty(),
            "expected no protocol diagnostics for valid protocol, got: {proto_diags:?}"
        );
    }

    #[test]
    fn test_validate_protocol_cycle_reports_diagnostic() {
        let temp = TempDir::new().unwrap();
        let proto_json = minimal_protocol_json("proto-cycle-id", true);
        setup_repo_with_protocol(&temp, "protocols/test-proto.json", &proto_json);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let proto_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.relative_path.contains("protocols") && d.severity == DiagnosticSeverity::Error
            })
            .collect();
        assert!(
            !proto_errors.is_empty(),
            "expected ERROR for cyclic protocol, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn test_validate_blueprint_valid_memory() {
        // MemoryStore has blueprint_paths: vec![] by default.
        // validate_repository must produce zero blueprint diagnostics on a store with no blueprints.
        let store = manifest_store(minimal_manifest(json!([])));
        let report = validate_repository(&store).unwrap();
        let bp_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.relative_path.contains("blueprint"))
            .collect();
        assert!(
            bp_diags.is_empty(),
            "expected no blueprint diagnostics on MemoryStore with no blueprints, got: {bp_diags:?}"
        );
    }

    #[test]
    fn test_validate_blueprint_memory_with_data() {
        use crate::package_types::DefinitionKind;
        // Cross-store: validate blueprint on MemoryStore with an actual blueprint in the data map.
        // manifest_store seeds the raw manifest.json text required by validate_repository.
        let store = manifest_store(minimal_manifest(json!([])));
        let bp_json = minimal_blueprint_json("00000000-0000-4000-8000-000000000060", true);
        store
            .save_instance_json("package/blueprints/test-bp.json", &bp_json)
            .unwrap();
        store
            .add_definition_to_boundary(&None, DefinitionKind::Blueprint, "blueprints/test-bp.json")
            .unwrap();
        let report = validate_repository(&store).unwrap();
        let bp_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.relative_path.contains("blueprints"))
            .collect();
        assert!(
            bp_diags.is_empty(),
            "expected no blueprint diagnostics on MemoryStore with valid blueprint, got: {bp_diags:?}"
        );
    }

    #[test]
    fn test_validate_blueprint_json_schema_applied_to_extra_property() {
        use crate::package_types::DefinitionKind;
        // JSON Schema (additionalProperties: false) must fire for a blueprint with an unknown field.
        let store = manifest_store(minimal_manifest(json!([])));
        let mut bp_json = minimal_blueprint_json("00000000-0000-4000-8000-000000000061", true);
        bp_json["unknownField"] = json!("bad");
        store
            .save_instance_json("package/blueprints/extra-field-bp.json", &bp_json)
            .unwrap();
        store
            .add_definition_to_boundary(
                &None,
                DefinitionKind::Blueprint,
                "blueprints/extra-field-bp.json",
            )
            .unwrap();
        let report = validate_repository(&store).unwrap();
        let schema_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.schema_id == Some(srs_schema::BLUEPRINT_SCHEMA_ID.to_string()))
            .collect();
        assert!(
            !schema_diags.is_empty(),
            "expected a JSON Schema diagnostic with blueprint schema_id, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn test_validate_blueprint_json_schema_applied_to_extra_property_file_store() {
        // Cross-store roundtrip: JSON Schema error must fire through the FileStore adapter.
        let temp = TempDir::new().unwrap();
        let mut bp_json = minimal_blueprint_json("00000000-0000-4000-8000-000000000062", true);
        bp_json["unknownField"] = json!("bad");
        setup_repo_with_blueprint(&temp, "blueprints/extra-field-bp.json", &bp_json);
        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let schema_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.schema_id == Some(srs_schema::BLUEPRINT_SCHEMA_ID.to_string()))
            .collect();
        assert!(
            !schema_diags.is_empty(),
            "expected a JSON Schema diagnostic with blueprint schema_id via FileStore, got: {:?}",
            report.diagnostics
        );
    }

    // ── RFC-018 I-81 extension: identity type checks ────────────────────────

    fn manifest_with_identity(identity_id: &str, tier: u8, path: &str) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Test Repo",
            "container": {
                "containerId": "00000000-0000-4000-8000-000000000098",
                "title": "Root",
                "identityInstanceId": identity_id,
                "memberInstanceIds": [identity_id]
            },
            "instanceIndex": [{
                "instanceId": identity_id,
                "tier": tier,
                "path": path
            }],
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    #[test]
    fn identity_tier0_memory_store_emits_rfc018_warning() {
        // Cross-store: MemoryStore must produce the same RFC-018 Warning as FileStore
        // for a Tier-0 Note identity. The Tier-0 path only reads the index tier,
        // so no data write is needed — the index entry in the manifest is enough.
        // Note: manifest_store only copies container/container_index from the JSON,
        // not instance_index, so we push the entry manually after store creation.
        let identity_id = "00000000-0000-4000-8000-000000000011";
        let store = manifest_store(manifest_with_identity(
            identity_id,
            0,
            "records/notes/id.json",
        ));
        let mut m = store.load_manifest().unwrap();
        m.instance_index.push(crate::index::InstanceIndexEntry {
            instance_id: identity_id.to_string(),
            tier: 0,
            path: "records/notes/id.json".to_string(),
            title: None,
            tags: None,
        });
        store.save_manifest(&m).unwrap();
        // Provide the actual note data so the instance-index validation loop doesn't
        // emit an I/O error for the missing file.
        store
            .save_instance_json("records/notes/id.json", &valid_note(identity_id))
            .unwrap();

        let report = validate_repository(&store).unwrap();

        let rfc018_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-018 I-81"))
            .collect();
        assert_eq!(
            rfc018_diags.len(),
            1,
            "MemoryStore: expected exactly one RFC-018 I-81 diagnostic, got: {:?}",
            rfc018_diags
        );
        assert_eq!(
            rfc018_diags[0].severity,
            DiagnosticSeverity::Warning,
            "MemoryStore: expected Warning for Tier-0 note identity"
        );
        assert!(
            rfc018_diags[0].message.contains("Tier-0 Note"),
            "MemoryStore: expected 'Tier-0 Note' in message, got: {}",
            rfc018_diags[0].message
        );
        assert!(
            report.is_ok(),
            "MemoryStore: un-migrated repo must remain is_ok(): {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn identity_tier0_note_emits_rfc018_warning() {
        let temp = TempDir::new().unwrap();
        let identity_id = "00000000-0000-4000-8000-000000000001";
        write_json(
            temp.path(),
            "manifest.json",
            &manifest_with_identity(identity_id, 0, "records/notes/identity.json"),
        );
        write_json(
            temp.path(),
            "records/notes/identity.json",
            &valid_note(identity_id),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();

        let rfc018_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-018 I-81"))
            .collect();
        assert_eq!(
            rfc018_diags.len(),
            1,
            "expected exactly one RFC-018 I-81 diagnostic, got: {:?}",
            rfc018_diags
        );
        assert_eq!(
            rfc018_diags[0].severity,
            DiagnosticSeverity::Warning,
            "expected Warning severity for Tier-0 note identity"
        );
        assert!(
            rfc018_diags[0].message.contains("Tier-0 Note"),
            "expected 'Tier-0 Note' in message, got: {}",
            rfc018_diags[0].message
        );
        assert!(
            report.is_ok(),
            "un-migrated repo must remain is_ok() (warnings do not fail): {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn identity_tier2_wrong_type_emits_rfc018_warning() {
        let temp = TempDir::new().unwrap();
        let identity_id = "00000000-0000-4000-8000-000000000002";
        let type_id = "00000000-0000-4000-8000-000000000003";
        write_json(
            temp.path(),
            "manifest.json",
            &manifest_with_identity(identity_id, 2, "records/identity.json"),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(Some("types/guide.json"), None),
        );
        write_json(
            temp.path(),
            "package/types/guide.json",
            &minimal_type_json(type_id),
        );
        // A Tier-2 record with typeNamespace "com.test" / typeName "test-type"
        write_json(
            temp.path(),
            "records/identity.json",
            &minimal_record_json(identity_id, type_id, None),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();

        let rfc018_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-018 I-81"))
            .collect();
        assert_eq!(
            rfc018_diags.len(),
            1,
            "expected exactly one RFC-018 I-81 diagnostic for wrong-type Tier-2 identity, got: {:?}",
            report.diagnostics
        );
        assert_eq!(
            rfc018_diags[0].severity,
            DiagnosticSeverity::Warning,
            "expected Warning severity for wrong-type Tier-2 identity (migration-period grace)"
        );
        assert!(
            rfc018_diags[0].message.contains("com.test/test-type"),
            "expected actual type in message, got: {}",
            rfc018_diags[0].message
        );
    }

    #[test]
    fn identity_tier2_purpose_type_no_rfc018_diagnostic() {
        let temp = TempDir::new().unwrap();
        let identity_id = "00000000-0000-4000-8000-000000000004";
        let type_id = "00000000-0000-4000-8000-000000000005";
        write_json(
            temp.path(),
            "manifest.json",
            &manifest_with_identity(identity_id, 2, "records/identity.json"),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(Some("types/purpose.json"), None),
        );
        write_json(
            temp.path(),
            "package/types/purpose.json",
            &json!({
                "id": type_id,
                "namespace": "com.semanticops.core",
                "name": "purpose",
                "version": 1,
                "description": "Repository identity record",
                "fields": [],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        // A Tier-2 record with typeNamespace "com.semanticops.core" / typeName "purpose"
        let purpose_record = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": identity_id,
            "typeId": type_id,
            "typeVersion": 1,
            "typeNamespace": "com.semanticops.core",
            "typeName": "purpose",
            "fieldValues": {},
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "records/identity.json", &purpose_record);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();

        let rfc018_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-018 I-81"))
            .collect();
        assert!(
            rfc018_diags.is_empty(),
            "expected no RFC-018 I-81 diagnostics for correct purpose-type identity, got: {:?}",
            rfc018_diags
        );
    }

    // ── ext:cross-field-validation integration tests ─────────────────────────

    fn cfr_pkg_json(field_paths: &[&str], type_path: &str) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
            "id": "00000000-0000-4000-8000-000000009000",
            "namespace": "com.test",
            "name": "cfr-package",
            "title": "CFR Test Package",
            "description": "integration test package",
            "status": "active",
            "version": "1.0.0",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": field_paths,
            "types": [type_path],
            "views": [],
            "documentViews": []
        })
    }

    fn cfr_field_json(id: &str, name: &str, value_type: &str) -> Value {
        let field_type = match value_type {
            "text" => json!({"datatype": "string", "format": "plain"}),
            "date" => json!({"datatype": "date"}),
            other => json!({"datatype": other}),
        };
        json!({
            "id": id,
            "namespace": "com.test",
            "name": name,
            "version": 1,
            "description": format!("{name} field"),
            "aiGuidance": {},
            "fieldType": field_type,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    /// `field_values` pairs are (`Field.name`, value) — the RFC-039 carrier
    /// keys by name.
    fn cfr_record_json(
        record_id: &str,
        type_id: &str,
        type_name: &str,
        field_values: &[(&str, Value)],
    ) -> Value {
        let mut fvs = serde_json::Map::new();
        for (name, v) in field_values {
            fvs.insert((*name).to_string(), v.clone());
        }
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": record_id,
            "typeId": type_id,
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": type_name,
            "fieldValues": fvs,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    #[test]
    fn cfr_conditional_required_violation_produces_error() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000009001";
        let type_id = "00000000-0000-4000-8000-000000009002";
        let pred_field_id = "00000000-0000-4000-8000-000000009010";
        let target_field_id = "00000000-0000-4000-8000-000000009011";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/cfr-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &cfr_pkg_json(
                &["fields/pred.json", "fields/target.json"],
                "types/cfr-type.json",
            ),
        );
        write_json(
            temp.path(),
            "package/fields/pred.json",
            &cfr_field_json(pred_field_id, "status", "text"),
        );
        write_json(
            temp.path(),
            "package/fields/target.json",
            &cfr_field_json(target_field_id, "review-comment", "text"),
        );
        write_json(
            temp.path(),
            "package/types/cfr-type.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "cfr-type",
                "version": 1,
                "description": "CFR test type",
                "fields": [
                    {"fieldId": pred_field_id, "order": 1, "required": false},
                    {"fieldId": target_field_id, "order": 2, "required": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z",
                "validationRules": [{
                    "type": "conditional-required",
                    "predicateFieldId": pred_field_id,
                    "predicateValue": "approved",
                    "targetFieldId": target_field_id
                }]
            }),
        );
        // Record has predicate set to "approved" but target field absent → violation
        write_json(
            temp.path(),
            "records/cfr-record.json",
            &cfr_record_json(
                record_id,
                type_id,
                "cfr-type",
                &[("status", json!("approved"))],
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let cfr_err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("conditional-required")
        });
        assert!(
            cfr_err.is_some(),
            "expected conditional-required error, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn cfr_conditional_required_satisfied_no_error() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000009021";
        let type_id = "00000000-0000-4000-8000-000000009022";
        let pred_field_id = "00000000-0000-4000-8000-000000009030";
        let target_field_id = "00000000-0000-4000-8000-000000009031";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/cfr-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &cfr_pkg_json(
                &["fields/pred.json", "fields/target.json"],
                "types/cfr-type.json",
            ),
        );
        write_json(
            temp.path(),
            "package/fields/pred.json",
            &cfr_field_json(pred_field_id, "status", "text"),
        );
        write_json(
            temp.path(),
            "package/fields/target.json",
            &cfr_field_json(target_field_id, "review-comment", "text"),
        );
        write_json(
            temp.path(),
            "package/types/cfr-type.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "cfr-type",
                "version": 1,
                "description": "CFR test type",
                "fields": [
                    {"fieldId": pred_field_id, "order": 1, "required": false},
                    {"fieldId": target_field_id, "order": 2, "required": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z",
                "validationRules": [{
                    "type": "conditional-required",
                    "predicateFieldId": pred_field_id,
                    "predicateValue": "approved",
                    "targetFieldId": target_field_id
                }]
            }),
        );
        // Record has predicate = "approved" AND target present → no violation
        write_json(
            temp.path(),
            "records/cfr-record.json",
            &cfr_record_json(
                record_id,
                type_id,
                "cfr-type",
                &[
                    ("status", json!("approved")),
                    ("review-comment", json!("LGTM")),
                ],
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let cfr_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("conditional-required"))
            .collect();
        assert!(
            cfr_errs.is_empty(),
            "expected no conditional-required errors when target is present, got: {:?}",
            cfr_errs
        );
    }

    #[test]
    fn cfr_field_ordering_violation_produces_error() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000009041";
        let type_id = "00000000-0000-4000-8000-000000009042";
        let start_field_id = "00000000-0000-4000-8000-000000009050";
        let end_field_id = "00000000-0000-4000-8000-000000009051";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/cfr-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &cfr_pkg_json(
                &["fields/start.json", "fields/end.json"],
                "types/cfr-type.json",
            ),
        );
        write_json(
            temp.path(),
            "package/fields/start.json",
            &cfr_field_json(start_field_id, "start-date", "date"),
        );
        write_json(
            temp.path(),
            "package/fields/end.json",
            &cfr_field_json(end_field_id, "end-date", "date"),
        );
        // end-date must-follow start-date: end > start
        write_json(
            temp.path(),
            "package/types/cfr-type.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "cfr-type",
                "version": 1,
                "description": "CFR ordering type",
                "fields": [
                    {"fieldId": start_field_id, "order": 1, "required": false},
                    {"fieldId": end_field_id, "order": 2, "required": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z",
                "validationRules": [{
                    "type": "field-ordering",
                    "targetFieldId": end_field_id,
                    "effect": "must-follow",
                    "predicateFieldId": start_field_id
                }]
            }),
        );
        // end-date = "2026-01-01" < start-date = "2026-06-01" → violation
        write_json(
            temp.path(),
            "records/cfr-record.json",
            &cfr_record_json(
                record_id,
                type_id,
                "cfr-type",
                &[
                    ("start-date", json!("2026-06-01")),
                    ("end-date", json!("2026-01-01")),
                ],
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let cfr_err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("field-ordering")
        });
        assert!(
            cfr_err.is_some(),
            "expected field-ordering error when end < start, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn cfr_field_ordering_satisfied_no_error() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000009061";
        let type_id = "00000000-0000-4000-8000-000000009062";
        let start_field_id = "00000000-0000-4000-8000-000000009070";
        let end_field_id = "00000000-0000-4000-8000-000000009071";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/cfr-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &cfr_pkg_json(
                &["fields/start.json", "fields/end.json"],
                "types/cfr-type.json",
            ),
        );
        write_json(
            temp.path(),
            "package/fields/start.json",
            &cfr_field_json(start_field_id, "start-date", "date"),
        );
        write_json(
            temp.path(),
            "package/fields/end.json",
            &cfr_field_json(end_field_id, "end-date", "date"),
        );
        write_json(
            temp.path(),
            "package/types/cfr-type.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "cfr-type",
                "version": 1,
                "description": "CFR ordering type",
                "fields": [
                    {"fieldId": start_field_id, "order": 1, "required": false},
                    {"fieldId": end_field_id, "order": 2, "required": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z",
                "validationRules": [{
                    "type": "field-ordering",
                    "targetFieldId": end_field_id,
                    "effect": "must-follow",
                    "predicateFieldId": start_field_id
                }]
            }),
        );
        // end-date = "2026-12-01" > start-date = "2026-01-01" → valid
        write_json(
            temp.path(),
            "records/cfr-record.json",
            &cfr_record_json(
                record_id,
                type_id,
                "cfr-type",
                &[
                    ("start-date", json!("2026-01-01")),
                    ("end-date", json!("2026-12-01")),
                ],
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let cfr_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("field-ordering"))
            .collect();
        assert!(
            cfr_errs.is_empty(),
            "expected no field-ordering errors for valid date range, got: {:?}",
            cfr_errs
        );
    }

    #[test]
    fn cfr_mutual_exclusion_violation_produces_error() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000009081";
        let type_id = "00000000-0000-4000-8000-000000009082";
        let field_a_id = "00000000-0000-4000-8000-000000009090";
        let field_b_id = "00000000-0000-4000-8000-000000009091";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/cfr-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &cfr_pkg_json(
                &["fields/tag-a.json", "fields/tag-b.json"],
                "types/cfr-type.json",
            ),
        );
        write_json(
            temp.path(),
            "package/fields/tag-a.json",
            &cfr_field_json(field_a_id, "tag-a", "text"),
        );
        write_json(
            temp.path(),
            "package/fields/tag-b.json",
            &cfr_field_json(field_b_id, "tag-b", "text"),
        );
        write_json(
            temp.path(),
            "package/types/cfr-type.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "cfr-type",
                "version": 1,
                "description": "CFR mutex type",
                "fields": [
                    {"fieldId": field_a_id, "order": 1, "required": false},
                    {"fieldId": field_b_id, "order": 2, "required": false}
                ],
                "createdAt": "2026-01-01T00:00:00Z",
                "validationRules": [{
                    "type": "mutual-exclusion",
                    "fieldIds": [field_a_id, field_b_id]
                }]
            }),
        );
        // Both fields set → mutual exclusion violation
        write_json(
            temp.path(),
            "records/cfr-record.json",
            &cfr_record_json(
                record_id,
                type_id,
                "cfr-type",
                &[("tag-a", json!("value-a")), ("tag-b", json!("value-b"))],
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let cfr_err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("mutual-exclusion")
        });
        assert!(
            cfr_err.is_some(),
            "expected mutual-exclusion error when both fields set, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn cfr_no_validation_rules_no_cfr_errors() {
        let temp = TempDir::new().unwrap();
        let record_id = "00000000-0000-4000-8000-000000009101";
        let type_id = "00000000-0000-4000-8000-000000009102";
        let field_id = "00000000-0000-4000-8000-000000009110";

        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([{
                "instanceId": record_id,
                "tier": 2,
                "path": "records/cfr-record.json"
            }])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &cfr_pkg_json(&["fields/status.json"], "types/cfr-type.json"),
        );
        write_json(
            temp.path(),
            "package/fields/status.json",
            &cfr_field_json(field_id, "status", "text"),
        );
        // Type has no validationRules key
        write_json(
            temp.path(),
            "package/types/cfr-type.json",
            &json!({
                "id": type_id,
                "namespace": "com.test",
                "name": "cfr-type",
                "version": 1,
                "description": "Type without validation rules",
                "fields": [{"fieldId": field_id, "order": 1, "required": false}],
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
        write_json(
            temp.path(),
            "records/cfr-record.json",
            &cfr_record_json(
                record_id,
                type_id,
                "cfr-type",
                &[("status", json!("active"))],
            ),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let cfr_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.message.contains("conditional-required")
                    || d.message.contains("field-ordering")
                    || d.message.contains("mutual-exclusion")
            })
            .collect();
        assert!(
            cfr_errs.is_empty(),
            "expected no CFR errors for type without validationRules, got: {:?}",
            cfr_errs
        );
    }

    #[test]
    fn cfr_mutual_exclusion_violation_memory_store() {
        // Cross-store variant: MemoryStore exercises the same CFR path as FileStore.
        // mutual-exclusion is the simplest rule (no field-type lookup required).
        use crate::manifest::Manifest;
        use crate::package::Package;

        let record_id = "00000000-0000-4000-8000-000000009200";
        let type_id = "00000000-0000-4000-8000-000000009201";
        let field_a = "00000000-0000-4000-8000-000000009210";
        let field_b = "00000000-0000-4000-8000-000000009211";

        let record_type: srs_core::types::record_type::RecordType = serde_json::from_value(json!({
            "id": type_id,
            "namespace": "com.test",
            "name": "me-type",
            "version": 1,
            "description": "Mutual exclusion MemoryStore test type",
            "fields": [
                {"fieldId": field_a, "order": 1, "required": false},
                {"fieldId": field_b, "order": 2, "required": false}
            ],
            "validationRules": [{
                "type": "mutual-exclusion",
                "fieldIds": [field_a, field_b]
            }],
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        // Field definitions are needed under RFC-039: the CFR map bridges the
        // rule's fieldIds to the record's name-keyed carrier via Field.name.
        let cfr_field = |id: &str, name: &str| -> srs_core::types::field::Field {
            serde_json::from_value(cfr_field_json(id, name, "text")).unwrap()
        };

        let record_json = cfr_record_json(
            record_id,
            type_id,
            "me-type",
            &[
                ("field-a", json!("val-a")),
                ("field-b", json!("val-b")), // both set → mutual-exclusion violation
            ],
        );

        let manifest_json = minimal_manifest(json!([{
            "instanceId": record_id,
            "tier": 2,
            "path": "records/cfr-me-record.json"
        }]));
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        let package = Package {
            id: "00000000-0000-4000-8000-000000009000".to_string(),
            namespace: "com.test".to_string(),
            name: "cfr-package".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![cfr_field(field_a, "field-a"), cfr_field(field_b, "field-b")],
            record_types: vec![record_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package)
            .with_data("records/cfr-me-record.json", record_json)
            .with_data("manifest.json", serde_json::Value::String(manifest_str));

        let report = validate_repository(&store).unwrap();
        let cfr_err = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error && d.message.contains("mutual-exclusion")
        });
        assert!(
            cfr_err.is_some(),
            "expected mutual-exclusion CFR error via MemoryStore, got: {:?}",
            report.diagnostics
        );
    }

    // --- #548: validate reads the authoritative relations file (not just relations.json) ---

    /// Minimal repo with two indexed notes and a loadable package. Each test writes the
    /// relations file at whichever path it is exercising. Returns the two note UUIDs.
    fn setup_repo_for_relation_validation(temp: &TempDir) -> (String, String) {
        let a = "00000000-0000-4000-8000-00000000000a".to_string();
        let b = "00000000-0000-4000-8000-00000000000b".to_string();
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": a, "tier": 0, "path": "records/notes/a.json"},
                {"instanceId": b, "tier": 0, "path": "records/notes/b.json"},
            ])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(None, None),
        );
        write_json(temp.path(), "records/notes/a.json", &valid_note(&a));
        write_json(temp.path(), "records/notes/b.json", &valid_note(&b));
        (a, b)
    }

    fn bad_type_relation(rel_id: &str, src: &str, tgt: &str) -> Value {
        json!({
            "relationId": rel_id,
            "relationType": "totally-bogus-type",
            "sourceInstanceId": src,
            "targetInstanceId": tgt,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn relations_collection(relations: Vec<Value>) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/relations-collection.json",
            "relations": relations
        })
    }

    #[test]
    fn validate_reads_relations_collection_json_for_e1() {
        // #548: relations live at relations-collection.json (the default write path), not
        // relations.json. Before the fix, validate read only relations.json and reported
        // zero relation diagnostics — a bogus relation type slipped through at rest.
        let temp = TempDir::new().unwrap();
        let (a, b) = setup_repo_for_relation_validation(&temp);
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &relations_collection(vec![bad_type_relation(
                "00000000-0000-4000-8000-000000000101",
                &a,
                &b,
            )]),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let e1 = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("E1"));
        assert!(
            e1.is_some(),
            "expected an E1 diagnostic for a bogus relation type in relations-collection.json, got: {:?}",
            report.diagnostics
        );
        assert_eq!(
            e1.unwrap().relative_path,
            "relations/relations-collection.json",
            "diagnostic should point at the authoritative relations file"
        );
    }

    #[test]
    fn validate_reads_relations_collection_json_for_e2() {
        // #548: a dangling endpoint in relations-collection.json must be caught (E2).
        let temp = TempDir::new().unwrap();
        let (a, _b) = setup_repo_for_relation_validation(&temp);
        let ghost = "00000000-0000-4000-8000-0000000000ff";
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &relations_collection(vec![bad_type_relation(
                "00000000-0000-4000-8000-000000000102",
                &a,
                ghost,
            )]),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.diagnostics.iter().any(|d| d.message.contains("E2")),
            "expected an E2 dangling-endpoint diagnostic, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_covers_standalone_relation_objects() {
        // RFC-038 Change E dual read: a standalone relation object with a dangling
        // endpoint must be caught (E2) exactly as a collection entry is, with the
        // diagnostic attributed to its own file.
        let temp = TempDir::new().unwrap();
        let (a, _b) = setup_repo_for_relation_validation(&temp);
        let ghost = "00000000-0000-4000-8000-0000000000ff";
        let rel_id = "00000000-0000-4000-8000-000000000102";
        let mut rel = bad_type_relation(rel_id, &a, ghost);
        rel.as_object_mut().unwrap().insert(
            "$schema".to_string(),
            json!(crate::store::RELATION_OBJECT_SCHEMA_URL),
        );
        write_json(temp.path(), &format!("relations/{rel_id}.json"), &rel);

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.diagnostics.iter().any(|d| d.message.contains("E2")
                && d.relative_path == format!("relations/{rel_id}.json")),
            "expected an E2 diagnostic attributed to the standalone object, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_no_dangling_endpoint_after_cascade_delete() {
        // RFC-038 acceptance test 13: an instance delete cascades its incident
        // Relations ([R22] scoped cascade), so validation afterwards reports no
        // E2 dangling-endpoint diagnostic. Incident edges cover both storage
        // forms (collection entry + standalone object).
        let temp = TempDir::new().unwrap();
        let (a, b) = setup_repo_for_relation_validation(&temp);
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &relations_collection(vec![json!({
                "relationId": "00000000-0000-4000-8000-000000000201",
                "relationType": "contains",
                "sourceInstanceId": a,
                "targetInstanceId": b,
                "createdAt": "2026-01-01T00:00:00Z"
            })]),
        );
        let standalone_id = "00000000-0000-4000-8000-000000000202";
        write_json(
            temp.path(),
            &format!("relations/{standalone_id}.json"),
            &json!({
                "$schema": crate::store::RELATION_OBJECT_SCHEMA_URL,
                "relationId": standalone_id,
                "relationType": "contains",
                "sourceInstanceId": b,
                "targetInstanceId": a,
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );

        let store = crate::store::FileStore::new(temp.path());
        crate::services::delete_note(&store, &b).unwrap();

        let report = validate_repository(&store).unwrap();
        assert!(
            !report.diagnostics.iter().any(|d| d.message.contains("E2")),
            "cascade delete must leave no dangling-endpoint diagnostic, got: {:?}",
            report.diagnostics
        );
        assert!(
            crate::relation_service::load_relations(&store)
                .unwrap()
                .is_empty(),
            "both incident relations must be removed by the cascade"
        );
    }

    #[test]
    fn validate_reports_duplicate_relation_id_across_forms() {
        // RFC-038 [R12]: the same relationId as a collection entry AND a standalone
        // object is an error naming both locators.
        let temp = TempDir::new().unwrap();
        let (a, b) = setup_repo_for_relation_validation(&temp);
        let rel_id = "00000000-0000-4000-8000-000000000102";
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &relations_collection(vec![json!({
                "relationId": rel_id,
                "relationType": "contains",
                "sourceInstanceId": a,
                "targetInstanceId": b,
                "createdAt": "2026-01-01T00:00:00Z"
            })]),
        );
        let mut standalone = json!({
            "relationId": rel_id,
            "relationType": "contains",
            "sourceInstanceId": a,
            "targetInstanceId": b,
            "createdAt": "2026-01-01T00:00:00Z"
        });
        standalone.as_object_mut().unwrap().insert(
            "$schema".to_string(),
            json!(crate::store::RELATION_OBJECT_SCHEMA_URL),
        );
        write_json(
            temp.path(),
            &format!("relations/{rel_id}.json"),
            &standalone,
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let dup = report
            .diagnostics
            .iter()
            .find(|d| d.message.contains("duplicate relationId"))
            .unwrap_or_else(|| {
                panic!(
                    "expected a duplicate-relationId diagnostic, got: {:?}",
                    report.diagnostics
                )
            });
        assert!(dup.message.contains(&format!("relations/{rel_id}.json")));
        assert!(dup.message.contains("relations/relations-collection.json"));
    }

    #[test]
    fn validate_honours_manifest_relations_path() {
        // #548: a custom manifest relationsPath must be the file validate reads, and the
        // diagnostic must be attributed to it.
        let temp = TempDir::new().unwrap();
        let a = "00000000-0000-4000-8000-00000000000a";
        let b = "00000000-0000-4000-8000-00000000000b";
        let mut manifest = minimal_manifest(json!([
            {"instanceId": a, "tier": 0, "path": "records/notes/a.json"},
            {"instanceId": b, "tier": 0, "path": "records/notes/b.json"},
        ]));
        manifest
            .as_object_mut()
            .unwrap()
            .insert("relationsPath".to_string(), json!("relations/custom.json"));
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(None, None),
        );
        write_json(temp.path(), "records/notes/a.json", &valid_note(a));
        write_json(temp.path(), "records/notes/b.json", &valid_note(b));
        write_json(
            temp.path(),
            "relations/custom.json",
            &relations_collection(vec![bad_type_relation(
                "00000000-0000-4000-8000-000000000103",
                a,
                b,
            )]),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let diag = report
            .diagnostics
            .iter()
            .find(|d| d.message.contains("E1") && d.relative_path == "relations/custom.json");
        assert!(
            diag.is_some(),
            "expected an E1 diagnostic attributed to relations/custom.json, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_still_reads_legacy_relations_json() {
        // Back-compat: repos whose relations live only at the legacy relations/relations.json
        // path are still validated exactly as before.
        let temp = TempDir::new().unwrap();
        let (a, b) = setup_repo_for_relation_validation(&temp);
        write_json(
            temp.path(),
            "relations/relations.json",
            &relations_collection(vec![bad_type_relation(
                "00000000-0000-4000-8000-000000000104",
                &a,
                &b,
            )]),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let e1 = report.diagnostics.iter().find(|d| d.message.contains("E1"));
        assert!(
            e1.is_some(),
            "expected E1 from legacy relations/relations.json, got: {:?}",
            report.diagnostics
        );
        assert_eq!(e1.unwrap().relative_path, "relations/relations.json");
    }

    #[test]
    fn validate_finds_relations_in_jsonstore_cross_store() {
        // #548 regression for the WASM/srs-web path: relations are written as a JSON object
        // (save_relations_json), so validate must resolve them in a JsonStore too — not just
        // FileStore. Before the fix, resolve_relations_source used load_text_file, which
        // returns nothing for an object-backed store, so JsonStore validate silently reported
        // zero relation diagnostics even though the relation was readable via the API.
        let temp = TempDir::new().unwrap();
        // Distinct id-prefixes: snapshot import derives a canonical filename from the id's
        // first 8 hex chars when a note has no title, so same-prefix ids would collide.
        let a = "aaaaaaaa-0000-4000-8000-000000000001";
        let b = "bbbbbbbb-0000-4000-8000-000000000001";
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": a, "tier": 0, "path": "records/notes/a.json"},
                {"instanceId": b, "tier": 0, "path": "records/notes/b.json"},
            ])),
        );
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(
            temp.path(),
            "package/package.json",
            &minimal_package_json(None, None),
        );
        write_json(temp.path(), "records/notes/a.json", &valid_note(a));
        write_json(temp.path(), "records/notes/b.json", &valid_note(b));
        write_json(
            temp.path(),
            "relations/relations-collection.json",
            &relations_collection(vec![bad_type_relation(
                "00000000-0000-4000-8000-000000000105",
                a,
                b,
            )]),
        );

        let file_store = crate::store::FileStore::new(temp.path());
        // Reconstruct the same repository in a JsonStore via snapshot import (the .srsj store).
        let snapshot =
            crate::repository_portability::export_repository_snapshot(&file_store).unwrap();
        let tmp2 = TempDir::new().unwrap();
        let json_store =
            crate::json_store::JsonStore::create(tmp2.path().join("repo.srsj")).unwrap();
        crate::repository_portability::import_repository_snapshot(&json_store, &snapshot).unwrap();

        let has_e1 =
            |r: &RepositoryValidationReport| r.diagnostics.iter().any(|d| d.message.contains("E1"));
        let file_report = validate_repository(&file_store).unwrap();
        let json_report = validate_repository(&json_store).unwrap();
        assert!(
            has_e1(&file_report),
            "FileStore should flag the bogus relation type (E1): {:?}",
            file_report.diagnostics
        );
        assert!(
            has_e1(&json_report),
            "JsonStore (WASM/srs-web path) must also flag the bogus relation type (E1) — \
             cross-store regression guard for #548: {:?}",
            json_report.diagnostics
        );
    }

    #[test]
    fn validate_reports_malformed_relations_file_as_diagnostic() {
        // A corrupt relations file must produce a diagnostic attributed to that file,
        // not abort the whole validation run.
        let temp = TempDir::new().unwrap();
        setup_repo_for_relation_validation(&temp);
        std::fs::create_dir_all(temp.path().join("relations")).unwrap();
        std::fs::write(
            temp.path().join("relations/relations-collection.json"),
            "{ this is not valid json",
        )
        .unwrap();

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let diag = report
            .diagnostics
            .iter()
            .find(|d| d.message.contains("failed to read relations file"));
        assert!(
            diag.is_some(),
            "expected a diagnostic for the malformed relations file, got: {:?}",
            report.diagnostics
        );
        assert_eq!(
            diag.unwrap().relative_path,
            "relations/relations-collection.json",
            "malformed-file diagnostic should be attributed to the relative candidate path"
        );
    }

    // ── spec/invariant number uniqueness tests ───────────────────────────────

    /// Arbitrary field ID used in the spec/invariant test package.
    /// The validator resolves the real ID at runtime via `find_field`; tests use
    /// this to keep test data self-consistent without depending on the live spec repo.
    const TEST_INV_NUM_FIELD_ID: &str = "ff000020-0000-4000-a000-000000000020";

    fn write_spec_invariant_pkg(dir: &Path) {
        write_json(dir, "package/.srs", &json!({}));
        write_json(
            dir,
            "package/package.json",
            &json!({
                "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
                "id": "00000000-0000-4000-8000-000000001000",
                "namespace": "com.semanticops.spec",
                "name": "spec",
                "title": "Spec Test Package",
                "description": "test",
                "status": "active",
                "version": "1.0.0",
                "createdAt": "2026-01-01T00:00:00Z",
                "fields": ["fields/invariant-number.json"],
                "types": [],
                "views": [],
                "documentViews": []
            }),
        );
        write_json(
            dir,
            "package/fields/invariant-number.json",
            &json!({
                "id": TEST_INV_NUM_FIELD_ID,
                "namespace": "com.semanticops.spec",
                "name": "invariant_number",
                "version": 1,
                "description": "invariant number",
                "aiGuidance": {},
                "fieldType": {"datatype": "string", "format": "plain"},
                "createdAt": "2026-01-01T00:00:00Z"
            }),
        );
    }

    fn spec_invariant_record_json(instance_id: &str, inv_num: &str) -> Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": instance_id,
            "typeId": "2a000006-0000-4000-a000-000000000006",
            "typeVersion": 1,
            "typeNamespace": "com.semanticops.spec",
            "typeName": "invariant",
            "fieldValues": {"invariant_number": inv_num},
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    #[test]
    fn validate_invariant_number_uniqueness_duplicate_emits_error() {
        let temp = TempDir::new().unwrap();
        let id_a = "00000000-0000-4000-8000-0000000a0001";
        let id_b = "00000000-0000-4000-8000-0000000a0002";

        write_spec_invariant_pkg(temp.path());
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": id_a, "tier": 2, "path": "records/inv-a.json"},
                {"instanceId": id_b, "tier": 2, "path": "records/inv-b.json"}
            ])),
        );
        write_json(
            temp.path(),
            "records/inv-a.json",
            &spec_invariant_record_json(id_a, "I-01"),
        );
        write_json(
            temp.path(),
            "records/inv-b.json",
            &spec_invariant_record_json(id_b, "I-01"), // duplicate
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let dup_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && d.message.contains("duplicate invariant number")
            })
            .collect();
        assert_eq!(
            dup_errs.len(),
            2,
            "expected 2 duplicate-invariant-number errors (one per record), got: {:?}",
            report.diagnostics
        );
        assert!(
            dup_errs.iter().all(|d| d.message.contains("I-01")),
            "all errors should name 'I-01', got: {:?}",
            dup_errs
        );
    }

    #[test]
    fn validate_invariant_number_uniqueness_distinct_numbers_pass() {
        let temp = TempDir::new().unwrap();
        let id_a = "00000000-0000-4000-8000-0000000b0001";
        let id_b = "00000000-0000-4000-8000-0000000b0002";

        write_spec_invariant_pkg(temp.path());
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": id_a, "tier": 2, "path": "records/inv-a.json"},
                {"instanceId": id_b, "tier": 2, "path": "records/inv-b.json"}
            ])),
        );
        write_json(
            temp.path(),
            "records/inv-a.json",
            &spec_invariant_record_json(id_a, "I-01"),
        );
        write_json(
            temp.path(),
            "records/inv-b.json",
            &spec_invariant_record_json(id_b, "I-02"), // distinct
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let dup_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("duplicate invariant number"))
            .collect();
        assert!(
            dup_errs.is_empty(),
            "expected no duplicate-invariant-number diagnostics for distinct numbers, got: {:?}",
            dup_errs
        );
    }

    #[test]
    fn validate_invariant_number_uniqueness_non_spec_type_no_false_positive() {
        // A non-spec type with a field that happens to have value "I-01" on two records
        // must NOT trigger the uniqueness check.
        let temp = TempDir::new().unwrap();
        let id_a = "00000000-0000-4000-8000-0000000c0001";
        let id_b = "00000000-0000-4000-8000-0000000c0002";

        write_spec_invariant_pkg(temp.path()); // package still needed for tier-2 processing
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": id_a, "tier": 2, "path": "records/rec-a.json"},
                {"instanceId": id_b, "tier": 2, "path": "records/rec-b.json"}
            ])),
        );
        // Records of a different type — same field value, different namespace/name
        for (path, id) in [("records/rec-a.json", id_a), ("records/rec-b.json", id_b)] {
            write_json(
                temp.path(),
                path,
                &json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": id,
                    "typeId": "aa000001-0000-4000-a000-000000000001",
                    "typeVersion": 1,
                    "typeNamespace": "com.example",
                    "typeName": "not-invariant",
                    "fieldValues": {"invariant_number": "I-01"},
                    "createdAt": "2026-01-01T00:00:00Z"
                }),
            );
        }

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let dup_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("duplicate invariant number"))
            .collect();
        assert!(
            dup_errs.is_empty(),
            "expected no false-positive duplicate-invariant-number diagnostics for non-spec type, got: {:?}",
            dup_errs
        );
    }

    #[test]
    fn validate_invariant_number_uniqueness_three_duplicates_emits_all() {
        // Three records sharing the same number → all three get an error.
        let temp = TempDir::new().unwrap();
        let id_a = "00000000-0000-4000-8000-0000000d0001";
        let id_b = "00000000-0000-4000-8000-0000000d0002";
        let id_c = "00000000-0000-4000-8000-0000000d0003";

        write_spec_invariant_pkg(temp.path());
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": id_a, "tier": 2, "path": "records/inv-a.json"},
                {"instanceId": id_b, "tier": 2, "path": "records/inv-b.json"},
                {"instanceId": id_c, "tier": 2, "path": "records/inv-c.json"}
            ])),
        );
        for (path, id) in [
            ("records/inv-a.json", id_a),
            ("records/inv-b.json", id_b),
            ("records/inv-c.json", id_c),
        ] {
            write_json(
                temp.path(),
                path,
                &spec_invariant_record_json(id, "I-99"), // all three share I-99
            );
        }

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let dup_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && d.message.contains("duplicate invariant number")
            })
            .collect();
        assert_eq!(
            dup_errs.len(),
            3,
            "expected 3 duplicate-invariant-number errors (one per record), got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_invariant_number_uniqueness_duplicate_via_memory_store() {
        // Cross-store variant: MemoryStore exercises the same uniqueness-check path.
        use crate::manifest::Manifest;

        let id_a = "00000000-0000-4000-8000-0000000e0001";
        let id_b = "00000000-0000-4000-8000-0000000e0002";

        let inv_field: srs_core::types::field::Field = serde_json::from_value(json!({
            "id": TEST_INV_NUM_FIELD_ID,
            "namespace": "com.semanticops.spec",
            "name": "invariant_number",
            "version": 1,
            "description": "invariant number",
            "aiGuidance": {},
            "fieldType": {"datatype": "string", "format": "plain"},
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();

        let manifest_json = minimal_manifest(json!([
            {"instanceId": id_a, "tier": 2, "path": "records/inv-a.json"},
            {"instanceId": id_b, "tier": 2, "path": "records/inv-b.json"}
        ]));
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        let store = MemoryStore::with_field(inv_field)
            .with_data(
                "records/inv-a.json",
                spec_invariant_record_json(id_a, "I-07"),
            )
            .with_data(
                "records/inv-b.json",
                spec_invariant_record_json(id_b, "I-07"), // duplicate
            )
            .with_data("manifest.json", serde_json::Value::String(manifest_str));
        store.save_manifest(&manifest).unwrap();

        let report = validate_repository(&store).unwrap();
        let dup_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && d.message.contains("duplicate invariant number")
            })
            .collect();
        assert_eq!(
            dup_errs.len(),
            2,
            "expected 2 duplicate-invariant-number errors via MemoryStore, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn validate_invariant_number_uniqueness_integer_values_detected() {
        // The real spec repo stores invariant numbers as JSON integers (1, 2, 3) not strings.
        // This test confirms the Number variant is coerced and compared correctly.
        let temp = TempDir::new().unwrap();
        let id_a = "00000000-0000-4000-8000-0000000f0001";
        let id_b = "00000000-0000-4000-8000-0000000f0002";

        write_spec_invariant_pkg(temp.path());
        write_json(
            temp.path(),
            "manifest.json",
            &minimal_manifest(json!([
                {"instanceId": id_a, "tier": 2, "path": "records/inv-a.json"},
                {"instanceId": id_b, "tier": 2, "path": "records/inv-b.json"}
            ])),
        );
        // Use integer JSON values (as the real spec repo does)
        for (path, id) in [("records/inv-a.json", id_a), ("records/inv-b.json", id_b)] {
            write_json(
                temp.path(),
                path,
                &json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": id,
                    "typeId": "2a000006-0000-4000-a000-000000000006",
                    "typeVersion": 1,
                    "typeNamespace": "com.semanticops.spec",
                    "typeName": "invariant",
                    "fieldValues": {"invariant_number": 42},
                    "createdAt": "2026-01-01T00:00:00Z"
                }),
            );
        }

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let dup_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Error
                    && d.message.contains("duplicate invariant number")
            })
            .collect();
        assert_eq!(
            dup_errs.len(),
            2,
            "expected 2 duplicate-invariant-number errors for integer-valued duplicates, got: {:?}",
            report.diagnostics
        );
        assert!(
            dup_errs.iter().all(|d| d.message.contains("42")),
            "all errors should name '42', got: {:?}",
            dup_errs
        );
    }

    // ── RFC-017 attachment_policy size/MIME-type diagnostic tests ─────────────

    const POLICY_FIELD_ALLOWED_MIME: &str = "bb000001-0000-4000-b000-000000000001";
    const POLICY_FIELD_MAX_PER_FILE: &str = "bb000002-0000-4000-b000-000000000002";
    const POLICY_FIELD_MAX_DOC: &str = "bb000003-0000-4000-b000-000000000003";
    const POLICY_FIELD_MAX_TOTAL: &str = "bb000004-0000-4000-b000-000000000004";
    const POLICY_TYPE_ID: &str = "bb000010-0000-4000-b000-000000000010";
    const POLICY_RECORD_ID: &str = "bb000020-0000-4000-b000-000000000020";

    struct PolicyLimits {
        max_per_file_bytes: Option<u64>,
        max_doc_bytes: Option<u64>,
        max_total_bytes: Option<u64>,
        allowed_mime_types: Option<Vec<String>>,
    }

    impl PolicyLimits {
        fn empty() -> Self {
            PolicyLimits {
                max_per_file_bytes: None,
                max_doc_bytes: None,
                max_total_bytes: None,
                allowed_mime_types: None,
            }
        }
    }

    /// Build a MemoryStore pre-populated with:
    /// - a manifest with one tier-2 entry for the policy record and entries for each
    ///   content file in `sourceDocumentIndex`
    /// - a minimal `com.semanticops.base` package with the 4 limit fields and repo_settings type
    /// - a tier-2 policy record at `records/policy.json` encoding the limits
    /// - binary content files and sidecar .meta.json files under `source-documents/`
    fn build_policy_store(
        limits: &PolicyLimits,
        content_files: &[(&str, &[u8], &str)], // (rel_path_under_src_docs, bytes, mime_type)
    ) -> MemoryStore {
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::Field;
        use srs_core::types::record_type::RecordType;
        use srs_core::types::source_document::SourceDocumentIndexEntry;
        use std::path::PathBuf;

        let allowed_mime_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_ALLOWED_MIME,
            "namespace": "com.semanticops.base",
            "name": "allowed_mime_types",
            "version": 1, "description": "allowed MIME types",
            "aiGuidance": {}, "fieldType": {"datatype": "string", "format": "plain", "cardinality": "list"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_per_file_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_PER_FILE,
            "namespace": "com.semanticops.base",
            "name": "max_per_file_bytes",
            "version": 1, "description": "max per-file bytes",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_doc_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_DOC,
            "namespace": "com.semanticops.base",
            "name": "max_doc_bytes",
            "version": 1, "description": "max doc bytes",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_total_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_TOTAL,
            "namespace": "com.semanticops.base",
            "name": "max_total_bytes",
            "version": 1, "description": "max total bytes",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let repo_settings_type: RecordType = serde_json::from_value(json!({
            "id": POLICY_TYPE_ID,
            "namespace": "com.semanticops.base",
            "name": "repo_settings",
            "version": 1,
            "description": "repository attachment policy settings",
            "fields": [
                {"fieldId": POLICY_FIELD_ALLOWED_MIME, "order": 1, "required": false},
                {"fieldId": POLICY_FIELD_MAX_PER_FILE, "order": 2, "required": false},
                {"fieldId": POLICY_FIELD_MAX_DOC, "order": 3, "required": false},
                {"fieldId": POLICY_FIELD_MAX_TOTAL, "order": 4, "required": false}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();

        let package = Package {
            id: "bb000000-0000-4000-b000-000000000000".to_string(),
            namespace: "com.semanticops.base".to_string(),
            name: "base".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                allowed_mime_field,
                max_per_file_field,
                max_doc_field,
                max_total_field,
            ],
            record_types: vec![repo_settings_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        // Build source_document_index
        let src_doc_index: Vec<SourceDocumentIndexEntry> = content_files
            .iter()
            .enumerate()
            .map(|(i, (rel_path, _, _))| SourceDocumentIndexEntry {
                document_id: format!("cc{:06}-0000-4000-b000-000000000001", i + 1),
                sidecar_path: format!("{}.meta.json", rel_path),
                content_path: rel_path.to_string(),
                title: None,
                sidecar_checksum: None,
                content_checksum: None,
            })
            .collect();

        // Build the manifest JSON for both schema validation and typed access
        let src_doc_index_json: serde_json::Value = serde_json::to_value(&src_doc_index).unwrap();
        let manifest_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Policy Test Repo",
            "container": {
                "containerId": "00000000-0000-4000-8000-000000000099",
                "title": "Policy Test Repo"
            },
            "instanceIndex": [
                {"instanceId": POLICY_RECORD_ID, "tier": 2, "path": "records/policy.json"}
            ],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": src_doc_index_json,
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        // Build field values for the policy record
        let mut field_values = serde_json::Map::new();
        if let Some(v) = limits.max_per_file_bytes {
            field_values.insert("max_per_file_bytes".to_string(), json!(v));
        }
        if let Some(v) = limits.max_doc_bytes {
            field_values.insert("max_doc_bytes".to_string(), json!(v));
        }
        if let Some(v) = limits.max_total_bytes {
            field_values.insert("max_total_bytes".to_string(), json!(v));
        }
        if let Some(ref mimes) = limits.allowed_mime_types {
            field_values.insert("allowed_mime_types".to_string(), json!(mimes));
        }

        let policy_record_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": POLICY_RECORD_ID,
            "typeId": POLICY_TYPE_ID,
            "typeVersion": 1,
            "typeNamespace": "com.semanticops.base",
            "typeName": "repo_settings",
            "fieldValues": field_values,
            "createdAt": "2026-01-01T00:00:00Z"
        });

        let store = MemoryStore::new(manifest, package)
            .with_data("manifest.json", serde_json::Value::String(manifest_str))
            .with_data("records/policy.json", policy_record_json);

        // Binary content files and text sidecar files
        for (i, (rel_path, bytes, mime_type)) in content_files.iter().enumerate() {
            let content_full = format!("source-documents/{}", rel_path);
            let sidecar_full = format!("source-documents/{}.meta.json", rel_path);
            let doc_id = format!("cc{:06}-0000-4000-b000-000000000001", i + 1);
            store.save_binary_file(&content_full, bytes).unwrap();
            let sidecar = json!({
                "documentId": doc_id,
                "contentPath": rel_path,
                "contentType": mime_type,
                "createdAt": "2026-01-01T00:00:00Z"
            });
            store
                .save_text_file(&sidecar_full, &serde_json::to_string(&sidecar).unwrap())
                .unwrap();
        }

        store
    }

    #[test]
    fn policy_absent_no_warnings() {
        // No repo_settings record → zero attachment_policy warnings regardless of files.
        let store = MemoryStore::empty().with_data(
            "manifest.json",
            serde_json::Value::String(serde_json::to_string(&minimal_manifest(json!([]))).unwrap()),
        );
        let report = validate_repository(&store).unwrap();
        let policy_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.message.contains("attachment_policy")
                    || d.message.contains("I-107")
                    || d.message.contains("Change B")
            })
            .collect();
        assert!(
            policy_diags.is_empty(),
            "no policy record → no attachment_policy diagnostics, got: {:?}",
            policy_diags
        );
    }

    #[test]
    fn policy_max_per_file_bytes_warn() {
        // File 200 bytes, max_per_file_bytes limit 100 → one Warning mentioning max_per_file_bytes.
        let store = build_policy_store(
            &PolicyLimits {
                max_per_file_bytes: Some(100),
                ..PolicyLimits::empty()
            },
            &[("report.pdf", &[0u8; 200], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        let warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("max_per_file_bytes")
            })
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "expected 1 max_per_file_bytes warning, got: {:?}",
            report.diagnostics
        );
        assert!(
            warns[0].message.contains("200"),
            "warning should mention the actual byte size, got: {}",
            warns[0].message
        );
        assert!(
            warns[0].message.contains("100"),
            "warning should mention the limit, got: {}",
            warns[0].message
        );
    }

    #[test]
    fn policy_max_doc_bytes_warn() {
        // File 200 bytes, max_doc_bytes limit 100 → one Warning mentioning max_doc_bytes.
        let store = build_policy_store(
            &PolicyLimits {
                max_doc_bytes: Some(100),
                ..PolicyLimits::empty()
            },
            &[("report.pdf", &[0u8; 200], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        let warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning && d.message.contains("max_doc_bytes")
            })
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "expected 1 max_doc_bytes warning, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn policy_max_total_bytes_warn() {
        // Two 60-byte files, max_total_bytes 100 → one aggregate Warning.
        let store = build_policy_store(
            &PolicyLimits {
                max_total_bytes: Some(100),
                ..PolicyLimits::empty()
            },
            &[
                ("a.pdf", &[0u8; 60], "application/pdf"),
                ("b.pdf", &[0u8; 60], "application/pdf"),
            ],
        );
        let report = validate_repository(&store).unwrap();
        let warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning && d.message.contains("max_total_bytes")
            })
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "expected 1 max_total_bytes aggregate warning, got: {:?}",
            report.diagnostics
        );
        assert!(
            warns[0].message.contains("120"),
            "warning should mention the actual total (120), got: {}",
            warns[0].message
        );
    }

    #[test]
    fn policy_mime_type_mismatch_warn() {
        // allowed_mime_types: ["text/plain"], file is "application/pdf" → one Warning.
        let store = build_policy_store(
            &PolicyLimits {
                allowed_mime_types: Some(vec!["text/plain".to_string()]),
                ..PolicyLimits::empty()
            },
            &[("doc.pdf", &[0u8; 10], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        let warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning && d.message.contains("application/pdf")
            })
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "expected 1 MIME-type mismatch warning, got: {:?}",
            report.diagnostics
        );
        assert!(
            warns[0].message.contains("I-107"),
            "warning should reference I-107, got: {}",
            warns[0].message
        );
    }

    #[test]
    fn policy_mime_type_match_no_warn() {
        // File MIME type IS in allowed_mime_types → zero MIME warnings.
        let store = build_policy_store(
            &PolicyLimits {
                allowed_mime_types: Some(vec!["application/pdf".to_string()]),
                ..PolicyLimits::empty()
            },
            &[("doc.pdf", &[0u8; 10], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        let mime_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning && d.message.contains("I-107"))
            .collect();
        assert!(
            mime_warns.is_empty(),
            "matching MIME type should produce no I-107 warnings, got: {:?}",
            mime_warns
        );
    }

    #[test]
    fn policy_multiple_records_error() {
        // Two repo_settings records → two Errors citing RFC-017 Change B, zero size warnings.
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::Field;
        use srs_core::types::record_type::RecordType;
        use std::path::PathBuf;

        let second_id = "bb000021-0000-4000-b000-000000000020";

        let allowed_mime_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_ALLOWED_MIME, "namespace": "com.semanticops.base",
            "name": "allowed_mime_types", "version": 1, "description": "x",
            "aiGuidance": {}, "fieldType": {"datatype": "string", "format": "plain"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let repo_settings_type: RecordType = serde_json::from_value(json!({
            "id": POLICY_TYPE_ID, "namespace": "com.semanticops.base",
            "name": "repo_settings", "version": 1, "description": "x",
            "fields": [], "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let package = Package {
            id: "bb000000-0000-4000-b000-000000000000".to_string(),
            namespace: "com.semanticops.base".to_string(),
            name: "base".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![allowed_mime_field],
            record_types: vec![repo_settings_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        let manifest_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Multi-policy Repo",
            "container": {
                "containerId": "00000000-0000-4000-8000-000000000099",
                "title": "Multi-policy Repo"
            },
            "instanceIndex": [
                {"instanceId": POLICY_RECORD_ID, "tier": 2, "path": "records/policy1.json"},
                {"instanceId": second_id, "tier": 2, "path": "records/policy2.json"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        let policy_record = |id: &str, path: &str| {
            (
                path.to_string(),
                json!({
                    "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
                    "instanceId": id,
                    "typeId": POLICY_TYPE_ID,
                    "typeVersion": 1,
                    "typeNamespace": "com.semanticops.base",
                    "typeName": "repo_settings",
                    "fieldValues": {},
                    "createdAt": "2026-01-01T00:00:00Z"
                }),
            )
        };
        let (p1_path, p1) = policy_record(POLICY_RECORD_ID, "records/policy1.json");
        let (p2_path, p2) = policy_record(second_id, "records/policy2.json");

        let store = MemoryStore::new(manifest, package)
            .with_data("manifest.json", serde_json::Value::String(manifest_str))
            .with_data(&p1_path, p1)
            .with_data(&p2_path, p2);

        let report = validate_repository(&store).unwrap();
        let change_b_errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("Change B"))
            .collect();
        assert_eq!(
            change_b_errors.len(),
            2,
            "expected 2 RFC-017 Change B errors (one per duplicate policy record), got: {:?}",
            report.diagnostics
        );

        let size_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning && d.message.contains("I-107"))
            .collect();
        assert!(
            size_warns.is_empty(),
            "multiple policy records → no size/MIME warnings, got: {:?}",
            size_warns
        );
    }

    #[test]
    fn policy_fields_not_in_package_no_panic() {
        // Policy record present but package has no com.semanticops.base fields.
        // Expect: no panic, no size/MIME warnings (all limits absent → no checks).
        let store = build_policy_store(
            &PolicyLimits::empty(),
            &[("doc.pdf", &[0u8; 50], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        // With no limits set in the policy record, no size/MIME warnings are emitted.
        let policy_limit_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning && d.message.contains("I-107"))
            .collect();
        assert!(
            policy_limit_warns.is_empty(),
            "no limits set → no size/MIME warnings, got: {:?}",
            policy_limit_warns
        );
    }

    #[test]
    fn policy_warnings_dont_block_ok() {
        // Limits exceeded → report.is_ok() still true, errors == 0.
        let store = build_policy_store(
            &PolicyLimits {
                max_per_file_bytes: Some(1),
                max_doc_bytes: Some(1),
                max_total_bytes: Some(1),
                allowed_mime_types: Some(vec!["text/plain".to_string()]),
            },
            &[("report.pdf", &[0u8; 100], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "attachment_policy warnings must not affect is_ok(); report: {:?}",
            report.diagnostics
        );
        assert_eq!(
            report.summary.errors, 0,
            "attachment_policy warnings must not increment errors; report: {:?}",
            report.diagnostics
        );
        assert!(
            report.summary.warnings > 0,
            "should have at least some warnings when limits are exceeded, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn policy_tombstone_content_file_absent_skipped_silently() {
        // ADR-031: a source document registered in sourceDocumentIndex whose content file is
        // absent (tombstone state) must be silently skipped — no I-107 warnings.
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::Field;
        use srs_core::types::record_type::RecordType;
        use srs_core::types::source_document::SourceDocumentIndexEntry;
        use std::path::PathBuf;

        let allowed_mime_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_ALLOWED_MIME, "namespace": "com.semanticops.base",
            "name": "allowed_mime_types", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "string", "format": "plain"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_per_file_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_PER_FILE, "namespace": "com.semanticops.base",
            "name": "max_per_file_bytes", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_doc_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_DOC, "namespace": "com.semanticops.base",
            "name": "max_doc_bytes", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_total_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_TOTAL, "namespace": "com.semanticops.base",
            "name": "max_total_bytes", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let repo_settings_type: RecordType = serde_json::from_value(json!({
            "id": POLICY_TYPE_ID, "namespace": "com.semanticops.base",
            "name": "repo_settings", "version": 1, "description": "",
            "fields": [
                {"fieldId": POLICY_FIELD_ALLOWED_MIME, "order": 1, "required": false},
                {"fieldId": POLICY_FIELD_MAX_PER_FILE, "order": 2, "required": false},
                {"fieldId": POLICY_FIELD_MAX_DOC, "order": 3, "required": false},
                {"fieldId": POLICY_FIELD_MAX_TOTAL, "order": 4, "required": false}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let package = Package {
            id: "bb000000-0000-4000-b000-000000000000".to_string(),
            namespace: "com.semanticops.base".to_string(),
            name: "base".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                allowed_mime_field,
                max_per_file_field,
                max_doc_field,
                max_total_field,
            ],
            record_types: vec![repo_settings_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        let tombstone_entry = SourceDocumentIndexEntry {
            document_id: "cc000001-0000-4000-b000-000000000001".to_string(),
            sidecar_path: "ghost.pdf.meta.json".to_string(),
            content_path: "ghost.pdf".to_string(),
            title: None,
            sidecar_checksum: None,
            content_checksum: None,
        };
        let src_doc_index_json = serde_json::to_value(vec![&tombstone_entry]).unwrap();
        let manifest_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Tombstone Test",
            "container": {"containerId": "00000000-0000-4000-8000-000000000099", "title": "Tombstone Test"},
            "instanceIndex": [{"instanceId": POLICY_RECORD_ID, "tier": 2, "path": "records/policy.json"}],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": src_doc_index_json,
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        // Policy record with a tight per-file limit — would warn if the file existed.
        let policy_record_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": POLICY_RECORD_ID,
            "typeId": POLICY_TYPE_ID, "typeVersion": 1,
            "typeNamespace": "com.semanticops.base", "typeName": "repo_settings",
            "fieldValues": {"max_per_file_bytes": 1},
            "createdAt": "2026-01-01T00:00:00Z"
        });

        let store = MemoryStore::new(manifest, package)
            .with_data("manifest.json", serde_json::Value::String(manifest_str))
            .with_data("records/policy.json", policy_record_json);
        // Intentionally NOT saving source-documents/ghost.pdf — tombstone state.

        let report = validate_repository(&store).unwrap();
        let size_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-107"))
            .collect();
        assert!(
            size_warns.is_empty(),
            "tombstone (content file absent) must be skipped silently; got: {:?}",
            size_warns
        );
    }

    #[test]
    fn policy_mime_string_value_single_type() {
        // allowed_mime_types stored as a bare string (e.g. "text/plain") — a single-MIME
        // shorthand the String variant of the parser handles.
        // File is application/pdf → mismatch → I-107 warning.
        use crate::manifest::Manifest;
        use crate::package::Package;
        use srs_core::types::field::Field;
        use srs_core::types::record_type::RecordType;
        use srs_core::types::source_document::SourceDocumentIndexEntry;
        use std::path::PathBuf;

        let allowed_mime_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_ALLOWED_MIME, "namespace": "com.semanticops.base",
            "name": "allowed_mime_types", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "string", "format": "plain"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_per_file_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_PER_FILE, "namespace": "com.semanticops.base",
            "name": "max_per_file_bytes", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_doc_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_DOC, "namespace": "com.semanticops.base",
            "name": "max_doc_bytes", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let max_total_field: Field = serde_json::from_value(json!({
            "id": POLICY_FIELD_MAX_TOTAL, "namespace": "com.semanticops.base",
            "name": "max_total_bytes", "version": 1, "description": "",
            "aiGuidance": {}, "fieldType": {"datatype": "number"}, "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let repo_settings_type: RecordType = serde_json::from_value(json!({
            "id": POLICY_TYPE_ID, "namespace": "com.semanticops.base",
            "name": "repo_settings", "version": 1, "description": "",
            "fields": [
                {"fieldId": POLICY_FIELD_ALLOWED_MIME, "order": 1, "required": false},
                {"fieldId": POLICY_FIELD_MAX_PER_FILE, "order": 2, "required": false},
                {"fieldId": POLICY_FIELD_MAX_DOC, "order": 3, "required": false},
                {"fieldId": POLICY_FIELD_MAX_TOTAL, "order": 4, "required": false}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        let package = Package {
            id: "bb000000-0000-4000-b000-000000000000".to_string(),
            namespace: "com.semanticops.base".to_string(),
            name: "base".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![
                allowed_mime_field,
                max_per_file_field,
                max_doc_field,
                max_total_field,
            ],
            record_types: vec![repo_settings_type],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };

        let doc_entry = SourceDocumentIndexEntry {
            document_id: "cc000001-0000-4000-b000-000000000001".to_string(),
            sidecar_path: "report.pdf.meta.json".to_string(),
            content_path: "report.pdf".to_string(),
            title: None,
            sidecar_checksum: None,
            content_checksum: None,
        };
        let src_doc_index_json = serde_json::to_value(vec![&doc_entry]).unwrap();
        let manifest_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "String MIME Test",
            "container": {"containerId": "00000000-0000-4000-8000-000000000099", "title": "String MIME Test"},
            "instanceIndex": [{"instanceId": POLICY_RECORD_ID, "tier": 2, "path": "records/policy.json"}],
            "sourceDocumentsPath": "source-documents",
            "sourceDocumentIndex": src_doc_index_json,
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        // allowed_mime_types as a bare string "text/plain" (not an array).
        let policy_record_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": POLICY_RECORD_ID,
            "typeId": POLICY_TYPE_ID, "typeVersion": 1,
            "typeNamespace": "com.semanticops.base", "typeName": "repo_settings",
            "fieldValues": {"allowed_mime_types": "text/plain"},
            "createdAt": "2026-01-01T00:00:00Z"
        });

        let store = MemoryStore::new(manifest, package)
            .with_data("manifest.json", serde_json::Value::String(manifest_str))
            .with_data("records/policy.json", policy_record_json);
        store
            .save_binary_file("source-documents/report.pdf", &[0u8; 10])
            .unwrap();
        let sidecar = json!({"documentId": "cc000001-0000-4000-b000-000000000001",
            "contentPath": "report.pdf", "contentType": "application/pdf"});
        store
            .save_text_file(
                "source-documents/report.pdf.meta.json",
                &serde_json::to_string(&sidecar).unwrap(),
            )
            .unwrap();

        let report = validate_repository(&store).unwrap();
        let i107_mime_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("I-107")
                    && d.message.contains("application/pdf")
            })
            .collect();
        assert_eq!(
            i107_mime_warns.len(), 1,
            "bare-string allowed_mime_types 'text/plain' should trigger I-107 for application/pdf; got: {:?}",
            i107_mime_warns
        );
    }

    #[test]
    fn policy_both_per_file_limits_fire_independently() {
        // max_per_file_bytes and max_doc_bytes are independent limits, both citing I-107.
        // A file exceeding both must produce two separate warnings.
        let store = build_policy_store(
            &PolicyLimits {
                max_per_file_bytes: Some(50),
                max_doc_bytes: Some(75),
                ..PolicyLimits::empty()
            },
            &[("big.pdf", &[0u8; 200], "application/pdf")],
        );
        let report = validate_repository(&store).unwrap();
        let per_file_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("I-107")
                    && d.message.contains("max_per_file_bytes")
            })
            .collect();
        let doc_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("I-107")
                    && d.message.contains("max_doc_bytes")
            })
            .collect();
        assert_eq!(
            per_file_warns.len(),
            1,
            "expected I-107 (max_per_file_bytes) warning; got: {:?}",
            per_file_warns
        );
        assert_eq!(
            doc_warns.len(),
            1,
            "expected I-107 (max_doc_bytes) warning; got: {:?}",
            doc_warns
        );
    }

    // I-027-2a helpers
    fn rtd_json(key: &str, retired: bool) -> Value {
        let mut v = json!({
            "id": "00000000-0000-4000-8000-0000000000a0",
            "version": 1,
            "key": key,
            "namespace": "com.test",
            "label": "Test Relation",
            "description": "A test relation type",
            "category": "lifecycle",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        if retired {
            v["status"] = json!("retired");
        }
        v
    }

    fn rp_dv_json(include_entries: serde_json::Value) -> Value {
        json!({
            "id": "00000000-0000-4000-8000-0000000000d1",
            "namespace": "com.test",
            "name": "dv",
            "version": 1,
            "description": "test doc view",
            "sections": [{
                "sectionId": "s1",
                "order": 0,
                "source": {"type": "fixed-instances", "instanceIds": []},
                "relationsPresentation": { "include": include_entries }
            }],
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn rp_package_json(with_rtd: bool) -> Value {
        let relation_types: Value = if with_rtd {
            json!(["relation-types/rtd.json"])
        } else {
            json!([])
        };
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "com.test",
            "name": "test-package",
            "title": "Test Package",
            "description": "test",
            "status": "active",
            "version": "1.0.0",
            "createdAt": "2026-01-01T00:00:00Z",
            "fields": [],
            "types": [],
            "views": [],
            "relationTypes": relation_types,
            "documentViews": ["document-views/dv.json"]
        })
    }

    #[test]
    fn validation_relations_presentation_duplicate_entry_warns() {
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(temp.path(), "package/package.json", &rp_package_json(true));
        write_json(
            temp.path(),
            "package/relation-types/rtd.json",
            &rtd_json("precedes", false),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &rp_dv_json(json!([
                {"relationType": "precedes"},
                {"relationType": "precedes"}
            ])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "I-027-2a duplicate is advisory; repo must stay ok: {:?}",
            report.diagnostics
        );
        let warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("I-027-2a")
                    && d.message.contains("duplicate")
            })
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "expected exactly 1 I-027-2a duplicate warning; got: {:?}",
            warns
        );
    }

    #[test]
    fn validation_relations_presentation_nonresolving_entry_warns() {
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(temp.path(), "package/package.json", &rp_package_json(false));
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &rp_dv_json(json!([
                {"relationType": "nonexistent-type"}
            ])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        assert!(
            report.is_ok(),
            "I-027-2a non-resolving is advisory; repo must stay ok: {:?}",
            report.diagnostics
        );
        let warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning
                    && d.message.contains("I-027-2a")
                    && d.message.contains("does not resolve")
            })
            .collect();
        assert_eq!(
            warns.len(),
            1,
            "expected exactly 1 I-027-2a non-resolving warning; got: {:?}",
            warns
        );
    }

    #[test]
    fn validation_relations_presentation_valid_no_extra_warnings() {
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &minimal_manifest(json!([])));
        write_json(temp.path(), "package/.srs", &json!({}));
        write_json(temp.path(), "package/package.json", &rp_package_json(true));
        write_json(
            temp.path(),
            "package/relation-types/rtd.json",
            &rtd_json("precedes", false),
        );
        write_json(
            temp.path(),
            "package/document-views/dv.json",
            &rp_dv_json(json!([
                {"relationType": "precedes"}
            ])),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let warns_027: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-027-2a"))
            .collect();
        assert!(
            warns_027.is_empty(),
            "valid relationsPresentation should produce no I-027-2a warnings; got: {:?}",
            warns_027
        );
    }

    // ── RFC-017 R2/R12 attaches source-ref validation tests ──────────────────

    const ATTACHES_NOTE_ID: &str = "dd000001-0000-4000-d000-000000000001";
    const ATTACHES_DOC_ID: &str = "ee000001-0000-4000-e000-000000000001";
    const ATTACHES_CONTENT_PATH: &str = "my-doc.pdf";

    /// Minimal manifest JSON with an optional sourceDocumentIndex entry and one note.
    fn attaches_manifest_json(with_index_entry: bool) -> serde_json::Value {
        let mut m = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Attaches Test Repo",
            "container": {
                "containerId": "00000000-0000-4000-8000-000000000099",
                "title": "Attaches Test Repo"
            },
            "instanceIndex": [
                {"instanceId": ATTACHES_NOTE_ID, "tier": 0, "path": "records/note.json"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        if with_index_entry {
            m["sourceDocumentsPath"] = json!("source-documents");
            m["sourceDocumentIndex"] = json!([{
                "documentId": ATTACHES_DOC_ID,
                "sidecarPath": format!("{}.meta.json", ATTACHES_CONTENT_PATH),
                "contentPath": ATTACHES_CONTENT_PATH
            }]);
        }
        m
    }

    /// Build a note JSON with the given sourceRefs array.
    fn note_with_source_refs(refs: serde_json::Value) -> serde_json::Value {
        json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/note.json",
            "instanceId": ATTACHES_NOTE_ID,
            "sections": [{"name": "body", "content": "test"}],
            "sourceRefs": refs
        })
    }

    /// Build a MemoryStore with the given manifest JSON and note JSON, optionally
    /// saving a binary content file at `source-documents/<ATTACHES_CONTENT_PATH>`.
    fn attaches_memory_store(
        manifest_json: serde_json::Value,
        note_json: serde_json::Value,
        content_present: bool,
    ) -> MemoryStore {
        use crate::manifest::Manifest;
        use crate::package::Package;
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package)
            .with_data("manifest.json", serde_json::Value::String(manifest_str))
            .with_data("records/note.json", note_json);
        // RFC-038 [R25]: source documents resolve via sidecar discovery — the
        // legacy `sourceDocumentIndex` in `manifest_json` above is inert (kept
        // only so these fixtures still exercise the [R2] retired-property field
        // round-trip); write the real sidecar these tests actually need found.
        if let Some(entries) = manifest_json_source_doc_entries(&store) {
            for entry in entries {
                let sidecar_path = format!(
                    "source-documents/{}",
                    entry["sidecarPath"].as_str().unwrap()
                );
                let sidecar = json!({
                    "documentId": entry["documentId"],
                    "contentPath": entry["contentPath"],
                    "contentType": "application/pdf",
                    "createdAt": "2026-01-01T00:00:00Z"
                });
                store.save_instance_json(&sidecar_path, &sidecar).unwrap();
            }
        }
        if content_present {
            store
                .save_binary_file(
                    &format!("source-documents/{}", ATTACHES_CONTENT_PATH),
                    b"content",
                )
                .unwrap();
        }
        store
    }

    /// Read back `sourceDocumentIndex` entries from the manifest JSON already
    /// stored on `store`, if any — used only to seed real sidecar fixtures above.
    fn manifest_json_source_doc_entries(store: &MemoryStore) -> Option<Vec<serde_json::Value>> {
        let manifest_str = store.load_text_file("manifest.json").ok()?;
        let manifest_json: serde_json::Value = serde_json::from_str(&manifest_str).ok()?;
        manifest_json
            .get("sourceDocumentIndex")
            .and_then(|v| v.as_array())
            .map(|a| a.to_vec())
            .filter(|v| !v.is_empty())
    }

    #[test]
    fn test_attaches_r2_unresolved_source_id() {
        // Index present with one known entry, but sourceRef points to a DIFFERENT id → R2 Error.
        // This tests the "index populated but missing this particular sourceId" branch.
        let manifest = attaches_manifest_json(true); // index contains ATTACHES_DOC_ID
        let note = note_with_source_refs(json!([{
            "sourceType": "repository-document",
            "sourceRole": "attaches",
            "sourceId": "other-doc-id"  // not in the index
        }]));
        let store = attaches_memory_store(manifest, note, false);
        let report = validate_repository(&store).unwrap();
        let r2_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-017 R2"))
            .collect();
        assert_eq!(
            r2_errs.len(),
            1,
            "expected 1 R2 Error for sourceId not in populated index, got: {:?}",
            r2_errs
        );
        assert!(
            r2_errs[0].message.contains("other-doc-id"),
            "message should name the unresolved id, got: {}",
            r2_errs[0].message
        );
    }

    #[test]
    fn test_attaches_r2_empty_index() {
        // sourceDocumentIndex is present but empty → R2 Error for attaches ref
        let mut manifest = attaches_manifest_json(false);
        manifest["sourceDocumentsPath"] = json!("source-documents");
        manifest["sourceDocumentIndex"] = json!([]);
        let note = note_with_source_refs(json!([{
            "sourceType": "repository-document",
            "sourceRole": "attaches",
            "sourceId": ATTACHES_DOC_ID
        }]));
        let store = attaches_memory_store(manifest, note, false);
        let report = validate_repository(&store).unwrap();
        let r2_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-017 R2"))
            .collect();
        assert_eq!(
            r2_errs.len(),
            1,
            "expected 1 R2 Error for empty index, got: {:?}",
            r2_errs
        );
    }

    #[test]
    fn test_attaches_r12_tombstone_warning() {
        // documentId is in the index but content file is absent (tombstone) → R12 Warning
        let manifest = attaches_manifest_json(true);
        let note = note_with_source_refs(json!([{
            "sourceType": "repository-document",
            "sourceRole": "attaches",
            "sourceId": ATTACHES_DOC_ID
        }]));
        // content_present = false → tombstone
        let store = attaches_memory_store(manifest, note, false);
        let report = validate_repository(&store).unwrap();
        let r12_warns: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| {
                d.severity == DiagnosticSeverity::Warning && d.message.contains("RFC-017 R12")
            })
            .collect();
        assert_eq!(
            r12_warns.len(),
            1,
            "expected 1 R12 tombstone Warning, got: {:?}",
            r12_warns
        );
        assert!(
            r12_warns[0].message.contains(ATTACHES_DOC_ID),
            "message should name the tombstone docId, got: {}",
            r12_warns[0].message
        );
        // Must not be an Error
        let r2_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-017 R2"))
            .collect();
        assert!(
            r2_errs.is_empty(),
            "tombstone must not produce R2 Error, got: {:?}",
            r2_errs
        );
    }

    #[test]
    fn test_attaches_happy_path_no_diagnostic() {
        // documentId is in the index and content file is present → no R2/R12 diagnostic
        let manifest = attaches_manifest_json(true);
        let note = note_with_source_refs(json!([{
            "sourceType": "repository-document",
            "sourceRole": "attaches",
            "sourceId": ATTACHES_DOC_ID
        }]));
        // content_present = true
        let store = attaches_memory_store(manifest, note, true);
        let report = validate_repository(&store).unwrap();
        let r2_r12: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-017 R2") || d.message.contains("RFC-017 R12"))
            .collect();
        assert!(
            r2_r12.is_empty(),
            "resolved + present doc must not produce R2/R12 diagnostics, got: {:?}",
            r2_r12
        );
    }

    #[test]
    fn test_attaches_non_attaches_role_skipped() {
        // sourceRole != "attaches" → no R2/R12 check even if sourceId is unresolved
        let manifest = attaches_manifest_json(false);
        let note = note_with_source_refs(json!([{
            "sourceType": "repository-document",
            "sourceRole": "cites",
            "sourceId": "completely-unknown-id"
        }]));
        let store = attaches_memory_store(manifest, note, false);
        let report = validate_repository(&store).unwrap();
        let r2_r12: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-017 R2") || d.message.contains("RFC-017 R12"))
            .collect();
        assert!(
            r2_r12.is_empty(),
            "non-attaches role must not trigger R2/R12, got: {:?}",
            r2_r12
        );
    }

    #[test]
    fn test_attaches_no_source_document_index() {
        // Manifest has no sourceDocumentIndex at all → attaches ref is unresolved → R2 Error
        let manifest = attaches_manifest_json(false); // no sourceDocumentIndex
        let note = note_with_source_refs(json!([{
            "sourceType": "repository-document",
            "sourceRole": "attaches",
            "sourceId": ATTACHES_DOC_ID
        }]));
        let store = attaches_memory_store(manifest, note, false);
        let report = validate_repository(&store).unwrap();
        let r2_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-017 R2"))
            .collect();
        assert_eq!(
            r2_errs.len(),
            1,
            "no index → attaches ref unresolved → R2 Error; got: {:?}",
            r2_errs
        );
    }

    #[test]
    fn test_record_attaches_r2_via_extra() {
        // Tier-2 Record with sourceRefs in extra["sourceRefs"] → R2 Error via raw JSON path (ADR-034)
        let manifest_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "dataModelRevision": 2,
            "repositoryId": "00000000-0000-4000-8000-000000000099",
            "title": "Record Attaches Test",
            "container": {
                "containerId": "00000000-0000-4000-8000-000000000099",
                "title": "Record Attaches Test"
            },
            "instanceIndex": [
                {"instanceId": ATTACHES_NOTE_ID, "tier": 2, "path": "records/rec.json"}
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let record_json = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/record.json",
            "instanceId": ATTACHES_NOTE_ID,
            "typeId": "ff000001-0000-4000-f000-000000000001",
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "test_type",
            "fieldValues": {},
            "createdAt": "2026-01-01T00:00:00Z",
            "sourceRefs": [{
                "sourceType": "repository-document",
                "sourceRole": "attaches",
                "sourceId": "unresolved-record-doc-id"
            }]
        });
        use crate::manifest::Manifest;
        use crate::package::Package;
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();
        let package = Package {
            id: "test-pkg".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            document_views: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            dependency_refs: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        };
        let store = MemoryStore::new(manifest, package)
            .with_data("manifest.json", serde_json::Value::String(manifest_str))
            .with_data("records/rec.json", record_json);
        let report = validate_repository(&store).unwrap();
        let r2_errs: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("RFC-017 R2"))
            .collect();
        assert_eq!(
            r2_errs.len(),
            1,
            "Record.extra sourceRefs must produce R2 Error; got: {:?}",
            r2_errs
        );
        assert!(
            r2_errs[0].message.contains("unresolved-record-doc-id"),
            "message should name the unresolved id, got: {}",
            r2_errs[0].message
        );
    }

    #[test]
    fn test_attaches_non_repository_doc_source_type_skipped() {
        // sourceType != "repository-document" with sourceRole="attaches" → no R2/R12 check
        let manifest = attaches_manifest_json(false);
        let note = note_with_source_refs(json!([{
            "sourceType": "external-url",
            "sourceRole": "attaches",
            "sourceId": "completely-unknown-id"
        }]));
        let store = attaches_memory_store(manifest, note, false);
        let report = validate_repository(&store).unwrap();
        let r2_r12: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("RFC-017 R2") || d.message.contains("RFC-017 R12"))
            .collect();
        assert!(
            r2_r12.is_empty(),
            "non-repository-document sourceType must not trigger R2/R12, got: {:?}",
            r2_r12
        );
    }
}
