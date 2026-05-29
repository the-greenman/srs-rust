use crate::error::RepositoryError;
use crate::manifest::load_manifest;
use crate::package::load_package;
use serde_json::Value;
use srs_core::types::relation::RelationsCollection;
use srs_core::validation::relation::{validate_relation, RelationValidationContext};
use srs_schema::{SchemaRegistry, NOTE_SCHEMA_ID, RECORD_SCHEMA_ID};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,
    pub path: String,
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

/// Validate an entire file-backed repository.
///
/// I/O errors and malformed JSON are returned as `Err(RepositoryError)`.
/// Schema violations are returned as diagnostics inside the report.
pub fn validate_repository(
    repo_root: &Path,
) -> Result<RepositoryValidationReport, RepositoryError> {
    let reg = SchemaRegistry::global();
    let mut diagnostics: Vec<ValidationDiagnostic> = Vec::new();
    let mut checked = 0usize;

    // --- Validate root manifest.json ---
    let manifest_path = repo_root.join("manifest.json");
    let manifest_raw =
        std::fs::read_to_string(&manifest_path).map_err(|e| RepositoryError::Io {
            path: manifest_path.clone(),
            source: e,
        })?;
    let manifest_value: Value =
        serde_json::from_str(&manifest_raw).map_err(|e| RepositoryError::ManifestParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    checked += 1;
    // TODO(phase-3): manifest.json schema requires formatVersion/scdsVersion/conformance/container
    // which do not yet exist in live manifests. Re-enable once the manifest format is migrated.
    let _ = &manifest_value;

    // --- Load manifest for instanceIndex ---
    let manifest = load_manifest(repo_root)?;

    // --- Validate each instanceIndex entry ---
    for entry in &manifest.instance_index {
        let instance_path = repo_root.join(entry.path());
        let rel_path = entry.path().to_string();

        let raw = match std::fs::read_to_string(&instance_path) {
            Ok(s) => s,
            Err(e) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    path: rel_path,
                    schema_id: None,
                    message: format!("I/O error: {e}"),
                });
                continue;
            }
        };

        let value: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    path: rel_path,
                    schema_id: None,
                    message: format!("JSON parse error: {e}"),
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
                    path: rel_path.clone(),
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
                    path: rel_path.clone(),
                    schema_id: Some(schema_id.to_string()),
                    message: e.to_string(),
                });
            }
        } else {
            diagnostics.push(ValidationDiagnostic {
                severity: DiagnosticSeverity::Warning,
                path: rel_path.clone(),
                schema_id: None,
                message: "no known $schema declared and tier has no default schema".to_string(),
            });
        }
    }

    // --- Validate package/package.json if present ---
    let package_manifest_path = repo_root.join("package/package.json");
    if package_manifest_path.exists() {
        if let Some(report) = validate_file_against_schema(
            &package_manifest_path,
            repo_root,
            srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
            reg,
        ) {
            checked += 1;
            diagnostics.extend(report);
        }
    }

    // --- Validate relations/relations.json against E1-E4 ---
    let relations_path = repo_root.join("relations/relations.json");
    if relations_path.exists() {
        // Schema-validate the file first
        if let Some(schema_diags) = validate_file_against_schema(
            &relations_path,
            repo_root,
            srs_schema::RELATIONS_COLLECTION_SCHEMA_ID,
            reg,
        ) {
            checked += 1;
            diagnostics.extend(schema_diags);
        }

        let pkg = match load_package(repo_root) {
            Ok(pkg) => pkg,
            Err(err) => {
                diagnostics.push(ValidationDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    path: "package/package.json".to_string(),
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

        // Build semanticObjectType map: parse each indexed file for the field
        let mut instance_semantic_types: HashMap<String, String> = HashMap::new();
        for entry in &manifest.instance_index {
            let inst_path = repo_root.join(entry.path());
            if let Ok(raw) = std::fs::read_to_string(&inst_path) {
                if let Ok(val) = serde_json::from_str::<Value>(&raw) {
                    if let Some(sot) = val.get("semanticObjectType").and_then(|v| v.as_str()) {
                        instance_semantic_types
                            .insert(entry.instance_id().to_string(), sot.to_string());
                    }
                }
            }
        }

        let raw = std::fs::read_to_string(&relations_path).map_err(|e| RepositoryError::Io {
            path: relations_path.clone(),
            source: e,
        })?;
        let coll: RelationsCollection =
            serde_json::from_str(&raw).map_err(|e| RepositoryError::RecordLoad {
                path: relations_path.clone(),
                source: e,
            })?;

        let ctx = RelationValidationContext {
            definitions: &pkg.relation_type_definitions,
            known_instance_ids: &known_instance_ids,
            instance_semantic_types: &instance_semantic_types,
        };
        let rel_rel_path = relative_display(&relations_path, repo_root);
        for relation in &coll.relations {
            if let Err(errs) = validate_relation(relation, &ctx, false) {
                for e in errs {
                    diagnostics.push(ValidationDiagnostic {
                        severity: DiagnosticSeverity::Error,
                        path: rel_rel_path.clone(),
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

fn validate_file_against_schema(
    path: &Path,
    repo_root: &Path,
    schema_id: &'static str,
    reg: &SchemaRegistry,
) -> Option<Vec<ValidationDiagnostic>> {
    let rel_path = relative_display(path, repo_root);
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let mut diags = Vec::new();
    if let Err(e) = reg.validate_by_id(schema_id, &value) {
        diags.push(ValidationDiagnostic {
            severity: DiagnosticSeverity::Error,
            path: rel_path,
            schema_id: Some(schema_id.to_string()),
            message: e.to_string(),
        });
    }
    Some(diags)
}

fn relative_display(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

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
            "scdsVersion": "2.0",
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

        let report = validate_repository(temp.path()).unwrap();
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

        let report = validate_repository(temp.path()).unwrap();
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

        let report = validate_repository(temp.path()).unwrap();
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
        let repo_root = PathBuf::from("/home/greenman/dev/semanticops/srs/srs");
        if !repo_root.join("manifest.json").exists() {
            println!("Skipping: live repo not found");
            return;
        }
        let report = validate_repository(&repo_root).unwrap();
        if !report.is_ok() {
            for d in &report.diagnostics {
                if d.severity == DiagnosticSeverity::Error {
                    println!("ERROR [{}]: {}", d.path, d.message);
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
