use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde_json::Value;
use srs_core::types::blueprint::{Blueprint, BlueprintDiagnosticSeverity};
use srs_core::types::field::ValueType;
use srs_core::types::lifecycle::RelationDirection;
use srs_core::types::protocol::{Protocol, ProtocolDiagnosticSeverity};
use srs_core::types::record::Record;
use srs_core::types::relation::RelationsCollection;
use srs_core::validation::blueprint::validate_blueprint;
use srs_core::validation::lifecycle::{
    validate_lifecycle, validate_type_lifecycle_v9, LifecycleDiagnosticSeverity,
};
use srs_core::validation::protocol::validate_protocol;
use srs_core::validation::record::validate_record;
use srs_core::validation::record_type::validate_cross_field_rules;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use srs_schema::{SchemaRegistry, NOTE_SCHEMA_ID, RECORD_SCHEMA_ID};
use std::collections::{HashMap, HashSet};

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

/// Validate an entire repository via the storage trait.
///
/// I/O errors and malformed JSON are returned as `Err(RepositoryError)`.
/// Schema violations are returned as diagnostics inside the report.
pub fn validate_repository(
    store: &dyn RepositoryStore,
) -> Result<RepositoryValidationReport, RepositoryError> {
    let reg = SchemaRegistry::global();
    let mut diagnostics: Vec<ValidationDiagnostic> = Vec::new();
    let mut checked = 0usize;
    let mut package_for_tier2: Option<Option<crate::package::Package>> = None;
    let mut field_type_map: Option<HashMap<String, ValueType>> = None;
    // RFC-022: relations loaded lazily for the at-rest requiresRelation check.
    // Outer None = not loaded yet; inner None = load failed (check is skipped —
    // a corrupt relations file is reported by relation validation, not here).
    let mut relations_for_rfc022: Option<Option<Vec<crate::relation_service::RelationSummary>>> =
        None;

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

    // --- Load manifest for instanceIndex ---
    let manifest = store.load_manifest()?;

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

                // I-80: memberInstanceIds and rootInstanceIds must all be in instanceIndex.
                // Uses the manifest already loaded above — no second load_manifest().
                let known_ids: HashSet<&str> = manifest
                    .instance_index
                    .iter()
                    .map(|e| e.instance_id())
                    .collect();
                if let Some(ref ids) = full_container.member_instance_ids {
                    for id in ids {
                        if !known_ids.contains(id.as_str()) {
                            diagnostics.push(ValidationDiagnostic {
                                severity: DiagnosticSeverity::Error,
                                relative_path: "manifest.json".to_string(),
                                schema_id: None,
                                message: format!(
                                    "RFC-013 I-80: memberInstanceId '{}' not found in instanceIndex",
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
                                    "RFC-013 I-80: rootInstanceId '{}' not found in instanceIndex",
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
                // when containerIndex is absent or empty)
                if let Some(ref ci) = manifest.container_index {
                    if !ci.is_empty() {
                        if let Some(ref members) = full_container.member_instance_ids {
                            let mut section_container_roots: HashSet<String> = HashSet::new();
                            for entry in ci {
                                if let Ok(c) = store.load_container(&entry.container_id) {
                                    if let Some(ref roots) = c.root_instance_ids {
                                        section_container_roots.extend(roots.iter().cloned());
                                    }
                                }
                            }
                            let identity_id = root.identity_instance_id.as_deref().unwrap_or("");
                            for member_id in members {
                                if member_id.as_str() == identity_id {
                                    continue;
                                }
                                if !section_container_roots.contains(member_id.as_str()) {
                                    diagnostics.push(ValidationDiagnostic {
                                        severity: DiagnosticSeverity::Warning,
                                        relative_path: "manifest.json".to_string(),
                                        schema_id: None,
                                        message: format!(
                                            "RFC-013 I-82: root container member '{}' is not the root of any container in containerIndex",
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
            // Runs independently of full_container availability — only needs the index.
            if let Some(ref identity_id) = root.identity_instance_id {
                if let Some(idx_entry) = manifest
                    .instance_index
                    .iter()
                    .find(|e| e.instance_id() == identity_id.as_str())
                {
                    if idx_entry.tier() == 0 {
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
                    } else if idx_entry.tier() == 2 {
                        match store.load_instance_json(idx_entry.path()) {
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
                                idx_entry.tier()
                            ),
                        });
                    }
                }
                // not found in index: the membership check above already emits an Error
            }
        }
    }

    // --- Validate each instanceIndex entry ---
    for entry in &manifest.instance_index {
        let rel_path = entry.path().to_string();

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
        let tier_schema_id = tier_to_schema_id(entry.tier());

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
                        entry.tier()
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

        if entry.tier() == 2 {
            if package_for_tier2.is_none() {
                let pkg = store.load_package().ok();
                field_type_map = Some(match &pkg {
                    Some(p) => p
                        .fields
                        .iter()
                        .map(|f| (f.id.clone(), f.value_type))
                        .collect(),
                    None => HashMap::new(),
                });
                package_for_tier2 = Some(pkg);
            }
            match package_for_tier2.as_ref().and_then(|p| p.as_ref()) {
                Some(package) => match serde_json::from_value::<Record>(value.clone()) {
                    Ok(record) => {
                        let rt_opt = package.resolve_type(&record.type_id, record.type_version);

                        if let Some(record_type) = rt_opt {
                            match package.effective_fields(record_type) {
                                Ok(effective_fields) => {
                                    if let Err(err) =
                                        validate_record(&record, record_type, &effective_fields)
                                    {
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
                                    let ftype_map = field_type_map
                                        .as_ref()
                                        .expect("field_type_map is populated in the same tier-2 lazy-load block");
                                    let cfr_errors =
                                        validate_cross_field_rules(&record, rules, ftype_map);
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
    } else if package_for_tier2.is_none() {
        // Only fresh-load when no tier-2 records were processed (note-only repo).
        // When package_for_tier2 is Some(None), the load already failed; don't retry.
        if let Ok(pkg) = store.load_package() {
            validate_vocabulary_invariants(&pkg, &mut diagnostics);
            validate_identity_field_invariants(&pkg, &mut diagnostics);
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
    }

    // --- Validate relations/relations.json against E1-E4 ---
    // relations.json is infrastructure, not an instance — not counted in `checked`.
    if let Ok(relations_raw) = store.load_text_file("relations/relations.json") {
        // Schema-validate the file first
        if let Ok(relations_value) = serde_json::from_str::<Value>(&relations_raw) {
            if let Some(schema_diags) = validate_value_against_schema(
                &relations_value,
                "relations/relations.json",
                srs_schema::RELATIONS_COLLECTION_SCHEMA_ID,
                reg,
            ) {
                diagnostics.extend(schema_diags);
            }
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

        // Build known instance IDs from manifest index
        let known_instance_ids: HashSet<String> = manifest
            .instance_index
            .iter()
            .map(|e| e.instance_id().to_string())
            .collect();

        // Build the semanticObjectType map via the shared helper so `repo validate`
        // and `create_relation` enforce E4 over identical inputs (#556).
        let instance_semantic_types = crate::writer::build_instance_semantic_types(store, &manifest);

        let coll: RelationsCollection = match serde_json::from_str(&relations_raw) {
            Ok(c) => c,
            Err(e) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "relations/relations.json".to_string(),
                    schema_id: None,
                    message: format!("JSON parse error: {e}"),
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
                        relative_path: "relations/relations.json".to_string(),
                        schema_id: None,
                        message: e.message,
                    });
                }
            }
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
                let ok = store.load_container(id).is_ok();
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

        // I-64: when a Container has rootInstanceIds and a containerType, containerType SHOULD
        // equal the resolved root Type's bare `name`. A mismatch is a stale hint, not an error.
        // Edge cases (unloadable root Record, unresolved Type) skip the check — never error here.
        let id_to_path: HashMap<String, String> = manifest
            .instance_index
            .iter()
            .map(|e| (e.instance_id().to_string(), e.path().to_string()))
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

fn validate_vocabulary_invariants(
    pkg: &crate::package::Package,
    diagnostics: &mut Vec<ValidationDiagnostic>,
) {
    // V2: every field.vocabularyRef must resolve to an installed Vocabulary UUID
    for field in &pkg.fields {
        if let Some(ref_id) = &field.vocabulary_ref {
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
        let mismatch = report
            .diagnostics
            .iter()
            .any(|d| d.message.contains("tier") && d.message.contains("expects schema"));
        assert!(
            mismatch,
            "expected tier/schema mismatch diagnostic, got: {:?}",
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
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
            "fields": [],
            "types": types,
            "views": [],
            "vocabularies": vocabs
        })
    }

    fn minimal_type_json(type_id: &str) -> Value {
        json!({
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
            "fieldValues": [],
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
            "id": "00000000-0000-4000-8000-000000000010",
            "namespace": "com.test",
            "name": "test-package",
            "version": "1.0.0",
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
            "id": field_id,
            "namespace": "com.test",
            "name": field_name,
            "version": 1,
            "valueType": "string",
            "createdAt": "2026-01-01T00:00:00Z"
        });
        if let Some(vr) = vocab_ref {
            obj["vocabularyRef"] = json!(vr);
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
            "fieldValues": [],
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
        let record =
            crate::record_store::create_record(&store, type_id, 1, vec![], None, None).unwrap();
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
            extra: std::collections::HashMap::new(),
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
            extra: std::collections::HashMap::new(),
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
            extra: std::collections::HashMap::new(),
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
        let record =
            crate::record_store::create_record(&file_store, type_id, 1, vec![], None, None)
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
            extra: std::collections::HashMap::new(),
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
            extra: std::collections::HashMap::new(),
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

        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Test I-80",
            "container": {"containerId": root_id, "title": "Root"},
            "containerIndex": [{"containerId": root_id, "title": "Root", "path": "containers/root.json"}],
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            "containers/root.json",
            &serde_json::to_value(rfc013_container(root_id, &[member_id], &[])).unwrap(),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("I-80"))
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

        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Test I-80 root",
            "container": {"containerId": root_id, "title": "Root"},
            "containerIndex": [{"containerId": root_id, "title": "Root", "path": "containers/root.json"}],
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            "containers/root.json",
            &serde_json::to_value(rfc013_container(root_id, &[], &[root_member_id])).unwrap(),
        );

        let store = crate::store::FileStore::new(temp.path());
        let report = validate_repository(&store).unwrap();
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("I-80"))
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

        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Test I-81 fail",
            "container": {"containerId": root_id, "title": "Root", "identityInstanceId": identity_id},
            "containerIndex": [{"containerId": root_id, "title": "Root", "path": "containers/root.json"}],
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            "containers/root.json",
            &serde_json::to_value(rfc013_container(root_id, &[member_id], &[member_id])).unwrap(),
        );
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
                "fieldValues": []
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

        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Test I-81 ok via root",
            "container": {"containerId": root_id, "title": "Root", "identityInstanceId": identity_id},
            "containerIndex": [{"containerId": root_id, "title": "Root", "path": "containers/root.json"}],
            "instanceIndex": [rfc013_instance_entry(identity_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        write_json(
            temp.path(),
            "containers/root.json",
            &serde_json::to_value(rfc013_container(root_id, &[identity_id], &[identity_id]))
                .unwrap(),
        );
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
                "fieldValues": []
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

        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Test I-81 ok via member",
            "container": {"containerId": root_id, "title": "Root", "identityInstanceId": identity_id},
            "containerIndex": [{"containerId": root_id, "title": "Root", "path": "containers/root.json"}],
            "instanceIndex": [rfc013_instance_entry(identity_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);
        // Identity only in memberInstanceIds, not rootInstanceIds
        write_json(
            temp.path(),
            "containers/root.json",
            &serde_json::to_value(rfc013_container(root_id, &[identity_id], &[])).unwrap(),
        );
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
                "fieldValues": []
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

        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "I-81 MemoryStore test",
            "container": {"containerId": root_id, "title": "Root", "identityInstanceId": identity_id},
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        // Container has member_id in memberInstanceIds but NOT identity_id
        let container_val =
            serde_json::to_value(rfc013_container(root_id, &[member_id], &[member_id])).unwrap();
        let store = manifest_store(manifest_val)
            .with_data(&format!("containers/{root_id}.json"), container_val);

        let report = validate_repository(&store).unwrap();
        let errors: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("I-81"))
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

        let root_container = rfc013_container(root_id, &[member_id], &[member_id]);
        // Section container roots other_id, not member_id
        let section_container = rfc013_container(section_container_id, &[other_id], &[other_id]);

        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "I-82 warn test",
            "container": {"containerId": root_id, "title": "Root"},
            "containerIndex": [
                {"containerId": section_container_id, "title": "Section"}
            ],
            "instanceIndex": [rfc013_instance_entry(member_id)],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        // Use manifest_store + with_data to insert container files without polluting typed manifest
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
    fn rfc013_embed_only_root_container_broken_embed_trips_i81() {
        // manifest.container present but no container file → the embed itself is the
        // root container (canonical repository-identity source) and IS validated:
        // an identity that is not a member trips I-81 instead of being silently skipped.
        let root_id = "00000000-0000-4000-8000-000000000800";
        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
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
                "fieldValues": []
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

        let manifest = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Full Valid RFC-013 Repo",
            "container": {
                "containerId": root_id,
                "title": "Root",
                "identityInstanceId": identity_id
            },
            "containerIndex": [
                {"containerId": root_id, "title": "Root", "path": "containers/root.json"},
                {"containerId": section_container_id, "title": "Section", "path": "containers/section.json"}
            ],
            "instanceIndex": [
                rfc013_instance_entry(identity_id),
                rfc013_instance_entry(section_id)
            ],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        write_json(temp.path(), "manifest.json", &manifest);

        // Root container: identity in rootInstanceIds + memberInstanceIds, section_id also member
        let root_container = json!({
            "containerId": root_id,
            "title": "Root",
            "memberInstanceIds": [identity_id, section_id],
            "rootInstanceIds": [identity_id]
        });
        write_json(temp.path(), "containers/root.json", &root_container);

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
                "fieldValues": []
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
                "fieldValues": []
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
        // Key alignment: FileStore uses the containerIndex `path` field; JsonStore uses
        // `"containers/{id}.json"` as the data key. We use "containers/{root_id}.json" for both.
        let root_id = "00000000-0000-4000-8000-000000000a00";
        let member_id = "00000000-0000-4000-8000-000000000a01";
        let container_file = format!("containers/{root_id}.json");
        let manifest_val = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
            "srsVersion": "2.0",
            "repositoryId": root_id,
            "title": "Cross-Store I-80",
            "container": {"containerId": root_id, "title": "Root"},
            "containerIndex": [{"containerId": root_id, "title": "Root", "path": container_file}],
            "instanceIndex": [],
            "createdAt": "2026-01-01T00:00:00Z"
        });
        let container_val =
            serde_json::to_value(rfc013_container(root_id, &[member_id], &[])).unwrap();

        // FileStore: write files to disk
        let temp = TempDir::new().unwrap();
        write_json(temp.path(), "manifest.json", &manifest_val);
        write_json(
            temp.path(),
            &format!("containers/{root_id}.json"),
            &container_val,
        );
        let file_store = crate::store::FileStore::new(temp.path());

        // JsonStore via from_srsj: snapshot doesn't preserve manifest.container so we use from_srsj.
        // Data key "containers/{root_id}.json" matches what JsonStore::load_container looks for.
        let mut data = serde_json::Map::new();
        data.insert(format!("containers/{root_id}.json"), container_val.clone());
        let srsj = json!({
            "srsj": "1",
            "manifest": manifest_val,
            "data": data
        });
        let json_store = crate::json_store::JsonStore::from_srsj(&srsj.to_string()).unwrap();

        let file_report = validate_repository(&file_store).unwrap();
        let json_report = validate_repository(&json_store).unwrap();

        let file_i80: Vec<_> = file_report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-80"))
            .collect();
        let json_i80: Vec<_> = json_report
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("I-80"))
            .collect();

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
            "fieldValues": [],
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
        json!({
            "id": id,
            "namespace": "com.test",
            "name": name,
            "version": 1,
            "description": format!("{name} field"),
            "aiGuidance": {},
            "valueType": value_type,
            "createdAt": "2026-01-01T00:00:00Z"
        })
    }

    fn cfr_record_json(
        record_id: &str,
        type_id: &str,
        type_name: &str,
        field_values: &[(&str, Value)],
    ) -> Value {
        let fvs: Vec<Value> = field_values
            .iter()
            .map(|(fid, v)| json!({"fieldId": fid, "value": v}))
            .collect();
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
                &[(pred_field_id, json!("approved"))],
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
                    (pred_field_id, json!("approved")),
                    (target_field_id, json!("LGTM")),
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
                    (start_field_id, json!("2026-06-01")),
                    (end_field_id, json!("2026-01-01")),
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
                    (start_field_id, json!("2026-01-01")),
                    (end_field_id, json!("2026-12-01")),
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
                &[
                    (field_a_id, json!("value-a")),
                    (field_b_id, json!("value-b")),
                ],
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
                &[(field_id, json!("active"))],
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

        let record_json = cfr_record_json(
            record_id,
            type_id,
            "me-type",
            &[
                (field_a, json!("val-a")),
                (field_b, json!("val-b")), // both set → mutual-exclusion violation
            ],
        );

        let manifest_json = minimal_manifest(json!([{
            "instanceId": record_id,
            "tier": 2,
            "path": "records/cfr-me-record.json"
        }]));
        let manifest_str = serde_json::to_string(&manifest_json).unwrap();
        let manifest: Manifest = serde_json::from_value(manifest_json).unwrap();

        let store = MemoryStore::with_type(record_type)
            .with_data("records/cfr-me-record.json", record_json)
            .with_data("manifest.json", serde_json::Value::String(manifest_str));
        store.save_manifest(&manifest).unwrap();

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
}
