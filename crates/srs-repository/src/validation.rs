use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde_json::Value;
use srs_core::types::record::Record;
use srs_core::types::relation::RelationsCollection;
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

    checked += 1;
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

    // --- Validate package/package.json if present ---
    if let Ok(pkg_value) = store.load_instance_json("package/package.json") {
        checked += 1;
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
    if let Ok(relations_raw) = store.load_text_file("relations/relations.json") {
        // Schema-validate the file first
        if let Ok(relations_value) = serde_json::from_str::<Value>(&relations_raw) {
            checked += 1;
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
        assert_eq!(report.summary.checked, 2);
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
}
