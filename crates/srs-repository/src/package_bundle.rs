//! # `.srspkg` Package Bundle codec (ADR-050; srs-rust#632, #690, #663)
//!
//! One reader ([`read_package_bundle`], bytes in, store-free) and one writer
//! ([`export_package_bundle`], deterministic bytes out). Both install entry
//! points (directory, bundle) end in the one
//! [`crate::package_install_service::install_package_bundle`] core.

use std::path::Path;

use serde_json::Value;
use srs_schema::{SchemaRegistry, PACKAGE_BUNDLE_SCHEMA_ID};

use crate::error::RepositoryError;
use crate::field_type_migration_service::{CURRENT_DATA_MODEL_REVISION, DATA_MODEL_REVISION_KEY};
use crate::package_install_service::{
    definition_name, validate_source_definition, PackageSourceBundle, PackageSourceDefinition,
    INSTALL_ORDER,
};
use crate::package_service::definition_rel_path;
use crate::store::definition_kind_key;

/// The reader's result: the bundle plus non-fatal pre-load transformer notes
/// (RFC-043 `migration-memberorder-dropped`, ...).
#[derive(Debug, Clone)]
pub struct ReadPackageBundle {
    pub bundle: PackageSourceBundle,
    pub notes: Vec<String>,
}

fn refuse(code: &'static str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::InvalidPackageBundle {
        code,
        message: message.into(),
    }
}

/// Appended to a content-shape failure on a bundle older than current (PD1d).
fn revision_hint(rev: u64) -> String {
    if rev < CURRENT_DATA_MODEL_REVISION {
        format!("; bundle declares dataModelRevision {rev}; re-export it with a current srs")
    } else {
        String::new()
    }
}

/// Read a `.srspkg` from its bytes: parse, refuse a `readme` (srs-rust#1164),
/// refuse a newer revision, run the RFC-043 pre-load transformer (always;
/// idempotent), validate against `package-bundle.json`, then validate every
/// definition with the loader's own strictness.
pub fn read_package_bundle(bytes: &[u8]) -> Result<ReadPackageBundle, RepositoryError> {
    let mut v: Value = serde_json::from_slice(bytes)
        .map_err(|e| refuse("bundle-not-json", format!("not a JSON document: {e}")))?;
    if !v.is_object() {
        return Err(refuse(
            "bundle-not-json",
            "a package bundle is a JSON object",
        ));
    }
    if v.get("readme").is_some() {
        return Err(refuse(
            "bundle-readme-unsupported",
            "this srs cannot carry a bundle readme yet (RFC-045, srs-rust#1164); \
             remove `readme` and retry",
        ));
    }
    let rev = v
        .get(DATA_MODEL_REVISION_KEY)
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if rev > CURRENT_DATA_MODEL_REVISION {
        return Err(refuse(
            "bundle-revision-too-new",
            format!(
                "bundle declares dataModelRevision {rev}; this srs supports up to \
                 {CURRENT_DATA_MODEL_REVISION}; upgrade srs"
            ),
        ));
    }
    let notes =
        crate::rfc043_container_entries_migration_service::migrate_package_bundle_value(&mut v)?
            .diagnostics;
    SchemaRegistry::global()
        .validate_by_id(PACKAGE_BUNDLE_SCHEMA_ID, &v)
        .map_err(|e| {
            refuse(
                "bundle-schema-invalid",
                format!("{e}{}", revision_hint(rev)),
            )
        })?;

    let mut definitions = Vec::new();
    for kind in INSTALL_ORDER {
        let key = definition_kind_key(kind);
        let Some(items) = v.get(key).and_then(Value::as_array) else {
            continue;
        };
        for (index, item) in items.iter().enumerate() {
            let at = format!("<bundle>/{key}/{index}");
            let invalid = |msg: String| {
                refuse(
                    "bundle-definition-invalid",
                    format!("{at}: {msg}{}", revision_hint(rev)),
                )
            };
            validate_source_definition(kind, Path::new(&at), item)
                .map_err(|e| invalid(e.to_string()))?;
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| invalid("missing `id`".to_string()))?;
            // A blank name would slugify to "" and collide; refuse it.
            let name = definition_name(kind, item)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| invalid("missing name".to_string()))?;
            definitions.push(PackageSourceDefinition {
                kind,
                rel_path: definition_rel_path(kind, &name, id),
                value: item.clone(),
            });
        }
    }

    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    let package_dependencies = v["packageDependencies"]
        .as_array()
        .filter(|a| !a.is_empty())
        .cloned();
    Ok(ReadPackageBundle {
        bundle: PackageSourceBundle {
            id: s("packageId"),
            namespace: s("packageNamespace"),
            name: s("packageName"),
            version: s("packageVersion"),
            package_dependencies,
            definitions,
        },
        notes,
    })
}
