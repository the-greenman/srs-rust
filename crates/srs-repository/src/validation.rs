use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde_json::Value;
use srs_core::types::record::Record;
use srs_core::types::relation::RelationsCollection;
use srs_core::validation::lifecycle::{validate_lifecycle, LifecycleDiagnosticSeverity};
use srs_core::validation::record::validate_record;
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
    // TODO(phase-3): manifest.json schema requires formatVersion/srsVersion/conformance/container
    // which do not yet exist in live manifests. Re-enable once the manifest format is migrated.
    let _ = &manifest_value;

    // --- Load manifest for instanceIndex ---
    let manifest = store.load_manifest()?;

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
                package_for_tier2 = Some(store.load_package().ok());
            }
            match package_for_tier2.as_ref().and_then(|p| p.as_ref()) {
                Some(package) => match serde_json::from_value::<Record>(value.clone()) {
                    Ok(record) => {
                        if let Some(record_type) =
                            package.resolve_type(&record.type_id, record.type_version)
                        {
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
                            if let Some(rt) =
                                package.resolve_type(&record.type_id, record.type_version)
                            {
                                let lc_states: Option<
                                    Vec<&srs_core::types::lifecycle::LifecycleState>,
                                > = if let Some(ref_id) = &rt.lifecycle_ref {
                                    // If lifecycle_ref doesn't resolve, skip V8 — V7 will report it
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

    // --- RFC-006 vocabulary invariants V2, V5, V7, V9 ---
    // Use the package already loaded for tier-2 validation if available; otherwise try a fresh
    // load so that vocabulary/lifecycle invariants fire even in note-only repositories.
    if let Some(Some(ref pkg)) = package_for_tier2 {
        validate_vocabulary_invariants(pkg, &mut diagnostics);
    } else if package_for_tier2.is_none() {
        // Only fresh-load when no tier-2 records were processed (note-only repo).
        // When package_for_tier2 is Some(None), the load already failed; don't retry.
        if let Ok(pkg) = store.load_package() {
            validate_vocabulary_invariants(&pkg, &mut diagnostics);
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

        // Build semanticObjectType map: parse each indexed instance file for the field
        let mut instance_semantic_types: HashMap<String, String> = HashMap::new();
        for entry in &manifest.instance_index {
            if let Ok(val) = store.load_instance_json(entry.path()) {
                if let Some(sot) = val.get("semanticObjectType").and_then(|v| v.as_str()) {
                    instance_semantic_types
                        .insert(entry.instance_id().to_string(), sot.to_string());
                }
            }
        }

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

    // V7: every type.lifecycleRef must resolve to an installed Lifecycle UUID
    for rt in &pkg.record_types {
        if let Some(ref_id) = &rt.lifecycle_ref {
            if !pkg.lifecycles.iter().any(|lc| &lc.id == ref_id) {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    relative_path: "package/package.json".to_string(),
                    schema_id: None,
                    message: format!(
                        "V7: type '{}' lifecycleRef '{}' does not resolve to an installed Lifecycle",
                        rt.name, ref_id
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
            "formatVersion": "1.0",
            "srsVersion": "2.0",
            "conformance": "SRS 2.0 Core ext:repository",
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

    // --- V7: type lifecycleRef UUID resolution ---

    #[test]
    fn vocabulary_v7_missing_lifecycle_ref_produces_error() {
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
        let v7_error = report
            .diagnostics
            .iter()
            .find(|d| d.severity == DiagnosticSeverity::Error && d.message.contains("V7"));
        assert!(
            v7_error.is_some(),
            "expected V7 error for unresolved lifecycleRef, got: {:?}",
            report.diagnostics
        );
    }

    #[test]
    fn vocabulary_v7_resolved_lifecycle_ref_no_error() {
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
            "expected no V7 errors for resolved lifecycleRef, got: {:?}",
            v7_errors
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

    // --- V7: dangling lifecycleRef produces a clear diagnostic (#136) ---

    #[test]
    fn dangling_lifecycle_ref_produces_clear_v7_diagnostic() {
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
        let v7 = report.diagnostics.iter().find(|d| {
            d.severity == DiagnosticSeverity::Error
                && d.message.contains("V7")
                && d.message.contains(missing_lc_id)
        });
        assert!(
            v7.is_some(),
            "expected V7 diagnostic naming the dangling UUID, got: {:?}",
            report.diagnostics
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
            crate::record_store::create_record(&store, type_id, 1, vec![], None, None, "records")
                .unwrap();
        // Container rooted in that record, but with a stale containerType hint.
        let container = srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c1".to_string(),
            title: "Guide container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: Some("not-guide".to_string()),
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
        let record = crate::record_store::create_record(
            &file_store,
            type_id,
            1,
            vec![],
            None,
            None,
            "records",
        )
        .unwrap();
        let container = srs_core::types::container::Container {
            container_id: "00000000-0000-4000-8000-0000000000c4".to_string(),
            title: "Guide container".to_string(),
            namespace: None,
            name: None,
            description: None,
            container_type: Some("not-guide".to_string()),
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
}
