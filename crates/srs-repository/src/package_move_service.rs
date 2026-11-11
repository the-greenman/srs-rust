//! Move a definition between package boundaries of one repository (srs-rust#1639).
//!
//! A definition keeps its id, version, namespace and content: only the file's location and the
//! two boundaries' `package.json` lists change. Records and relations resolve definitions by id
//! (and relation types by key), so nothing that uses it is touched. Every version of the id moves
//! together, because a definition's lineage lives in one boundary (the cross-boundary duplicate
//! guard matches by id across versions, `refuse_cross_boundary_duplicate_id`).
//!
//! Refusals (each a [`RepositoryError::PackageMoveRefused`] code, ADR-053):
//! - `move-same-boundary`: the definition is already in the target;
//! - `move-split-lineage`: the id is already in more than one boundary (repair that first);
//! - `move-definition-imported`: an installed boundary's `.srs-import` records track it, so it
//!   is refined at its source package, not in the consumer;
//! - `move-target-installed`: the target is an installed package boundary.
//!
//! An unknown id is `definition-not-found`; an unregistered target is `package-not-found`.

use crate::error::RepositoryError;
use crate::package_service::new_definition_rel_path;
use crate::package_types::{DefinitionKind, PackageSelector};
use crate::store::{definition_kind_key, RepositoryStore};
use serde::{Deserialize, Serialize};

const KINDS: [DefinitionKind; 10] = [
    DefinitionKind::Field,
    DefinitionKind::Type,
    DefinitionKind::View,
    DefinitionKind::Composition,
    DefinitionKind::RelationType,
    DefinitionKind::Blueprint,
    DefinitionKind::Protocol,
    DefinitionKind::Vocabulary,
    DefinitionKind::Lifecycle,
    DefinitionKind::Theme,
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveDefinitionInput {
    /// The definition's id; every version of it moves.
    pub id: String,
    /// The target boundary: `None` for the primary package, `Some(path)` for a sub-package.
    pub to: PackageSelector,
    /// Report what would move and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovedDefinition {
    /// The `package.json` list it sits in (`fields`, `relationTypes`, ...).
    pub kind: String,
    pub id: String,
    pub version: u64,
    /// The source boundary path (`package` for the primary).
    pub from: String,
    /// The target boundary path.
    pub to: String,
    /// The file's path relative to the boundary, the same in both unless the target already
    /// holds another definition under that name.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveDefinitionOutput {
    pub moved: Vec<MovedDefinition>,
    /// Definition files left in the source boundary that mention the moved id. A bundled export
    /// of the source still carries the moved definition through its references; this is
    /// information, not a refusal.
    pub referenced_by: Vec<String>,
    pub dry_run: bool,
}

fn refused(code: &'static str, message: String) -> RepositoryError {
    RepositoryError::PackageMoveRefused { code, message }
}

fn prefix(selector: &PackageSelector) -> String {
    selector.clone().unwrap_or_else(|| "package".to_string())
}

fn kind_name(kind: DefinitionKind) -> &'static str {
    definition_kind_key(kind)
}

/// The definition ids an installed boundary's `.srs-import/import-records.json` tracks, or
/// `None` when the boundary is not installed.
fn imported_ids<S: RepositoryStore + ?Sized>(
    store: &S,
    boundary_prefix: &str,
) -> Option<Vec<String>> {
    let summary = store
        .load_instance_json(&format!(
            "{boundary_prefix}/.srs-import/import-records.json"
        ))
        .ok()?;
    let ids = summary
        .as_object()
        .into_iter()
        .flat_map(|o| o.values())
        .filter_map(|v| v.as_array())
        .flatten()
        .filter_map(|r| r["definitionId"].as_str().map(str::to_string))
        .collect();
    Some(ids)
}

/// Move every version of a definition to another package boundary, keeping its id.
pub fn move_definition<S: RepositoryStore + ?Sized>(
    store: &S,
    input: &MoveDefinitionInput,
) -> Result<MoveDefinitionOutput, RepositoryError> {
    let boundaries = store.list_package_boundaries()?;
    if !boundaries.iter().any(|b| b.selector == input.to) {
        return Err(RepositoryError::PackageNotFound {
            selector: input.to.clone(),
        });
    }
    let to_prefix = prefix(&input.to);

    // Every (boundary, kind, rel path, value) whose definition carries the id, and every other
    // definition file, for `referenced_by`.
    struct Found {
        selector: PackageSelector,
        kind: DefinitionKind,
        rel: String,
        value: serde_json::Value,
    }
    let mut found: Vec<Found> = Vec::new();
    let mut others: Vec<(PackageSelector, String, serde_json::Value)> = Vec::new();
    for b in &boundaries {
        let p = prefix(&b.selector);
        let Ok(pkg) = store.load_instance_json(&format!("{p}/package.json")) else {
            continue;
        };
        for kind in KINDS {
            for rel in pkg[definition_kind_key(kind)]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str())
            {
                let Ok(value) = store.load_instance_json(&format!("{p}/{rel}")) else {
                    continue;
                };
                if value["id"].as_str() == Some(input.id.as_str()) {
                    found.push(Found {
                        selector: b.selector.clone(),
                        kind,
                        rel: rel.to_string(),
                        value,
                    });
                } else {
                    others.push((b.selector.clone(), format!("{p}/{rel}"), value));
                }
            }
        }
    }

    let Some(first) = found.first() else {
        return Err(RepositoryError::DefinitionNotFound {
            id: input.id.clone(),
        });
    };
    let from = first.selector.clone();
    if found.iter().any(|f| f.selector != from) {
        return Err(refused(
            "move-split-lineage",
            format!(
                "definition {} is in more than one package boundary; repair the duplicate first",
                input.id
            ),
        ));
    }
    let from_prefix = prefix(&from);
    if from == input.to {
        return Err(refused(
            "move-same-boundary",
            format!("definition {} is already in {to_prefix}", input.id),
        ));
    }
    if imported_ids(store, &from_prefix).is_some_and(|ids| ids.contains(&input.id)) {
        return Err(refused(
            "move-definition-imported",
            format!(
                "definition {} was installed into {from_prefix} from a package; refine it in its source package",
                input.id
            ),
        ));
    }
    if imported_ids(store, &to_prefix).is_some() {
        return Err(refused(
            "move-target-installed",
            format!("{to_prefix} is an installed package boundary; move definitions into the package it was installed from"),
        ));
    }

    let mut moved = Vec::new();
    for f in &found {
        let version = f.value["version"].as_u64().unwrap_or(1);
        // Keep the file name unless the target already holds another definition under it.
        let held_by_other = store
            .load_instance_json(&format!("{to_prefix}/{}", f.rel))
            .ok()
            .is_some_and(|v| v["id"].as_str() != Some(input.id.as_str()));
        let path = if held_by_other {
            let slug = f.value["name"]
                .as_str()
                .or_else(|| f.value["key"].as_str())
                .unwrap_or(&input.id);
            new_definition_rel_path(store, &to_prefix, f.kind, slug, &input.id, version)
        } else {
            f.rel.clone()
        };
        moved.push(MovedDefinition {
            kind: kind_name(f.kind).to_string(),
            id: input.id.clone(),
            version,
            from: from_prefix.clone(),
            to: to_prefix.clone(),
            path,
        });
    }
    // ponytail: a text scan of the source boundary's definition files for the id; a typed walk
    // of PINNED/LINEAGE references (package_bundle's closure) if this ever reports false hits.
    let mut referenced_by: Vec<String> = others
        .iter()
        .filter(|(sel, _, v)| *sel == from && v.to_string().contains(input.id.as_str()))
        .map(|(_, path, _)| path.clone())
        .collect();
    referenced_by.sort();

    if !input.dry_run {
        // Write the target first, then the lists, then delete the source files: a failure
        // partway leaves the definition findable in at least one boundary.
        for (f, m) in found.iter().zip(&moved) {
            let target = format!("{to_prefix}/{}", m.path);
            // a new boundary may not have this kind's directory yet
            if let Some((dir, _)) = target.rsplit_once('/') {
                store.ensure_instance_dir(dir)?;
            }
            store.save_instance_json(&target, &f.value)?;
            store.add_definition_to_boundary(&input.to, f.kind, &m.path)?;
        }
        for f in &found {
            store.remove_definition_from_boundary(&from, f.kind, &f.rel)?;
            store.delete_instance_file(&format!("{from_prefix}/{}", f.rel))?;
        }
    }

    Ok(MoveDefinitionOutput {
        moved,
        referenced_by,
        dry_run: input.dry_run,
    })
}
