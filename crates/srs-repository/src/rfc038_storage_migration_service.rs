//! RFC-038 storage migration (`rfc038-storage`) — the placement transform.
//!
//! Moves a repository from the pre-cutover storage layout to the
//! tree-authoritative one. Three mechanical steps, no semantics:
//!
//! 1. **Explode relation collections** — every `{ "relations": [...] }` file
//!    becomes one standalone object per relation at `relations/<relationId>.json`
//!    (Change E, [R11]). The id set is unchanged and every id unique, or the run
//!    aborts.
//! 2. **Strip the retired manifest properties** ([`RETIRED_PROPERTIES`], Change K)
//!    from the manifest — the exploded `manifest.json`, and, on the `.srsj`
//!    surface, the envelope manifest and any `data["manifest.json"]` shadow.
//! 3. **Bump the `.srsj` envelope** from generation 1 to `srsj: "2"` ([R20]).
//!
//! Instances are left exactly where they are: placement is not identity, and
//! moving them would be a second, unrelated migration ([R6]).
//!
//! **No revision bump.** RFC-038 forbids a data-model revision 3, so
//! `migration_needed` is *structural*: it asks whether a retired key or a
//! relations collection is actually present, never what `dataModelRevision`
//! says. srs#242 Phase B stamped revision 2 while the index was still in place,
//! so a revision-keyed discriminator would report the whole corpus already
//! migrated. This transform is the one component permitted to read that
//! stamped-but-unstripped state, and the one permitted to read a generation-1
//! `.srsj` document ([R21]'s migrator exemption).
//!
//! **Two surfaces, one transform.** [`migrate_storage`] is the registered
//! store-level pass and does all of the work. [`migrate_srsj`] exists only
//! because [`crate::srsj::open_srsj`] refuses `srsj: "1"` ([R20]), so a
//! generation-1 document cannot be opened as a store at all: it normalises the
//! raw envelope until the codec will accept it, then hands off to
//! [`migrate_storage`]. Nothing about the migration itself lives there.
//!
//! **The manifest strip is blocked on a schema change (srs-rust#828).** The
//! published `manifest.json` schema still lists `instanceIndex` in `required`,
//! so a manifest this transform strips is one `repo validate` rejects, and
//! `Manifest` cannot stop serialising an empty index without breaking every
//! repository including a freshly created one. The transform is correct — the
//! schema is what has not caught up. Until it does, the registry's apply route
//! refuses (see `migration_registry_service`) and the only callers are Rust:
//! the Phase-6 fixture migration, which lands with the schema change and the
//! enforcement flip together.
//!
//! **Rollback is `git revert`.** No store implements batch rollback
//! (srs-rust#813), so an in-place run that fails partway leaves the tree as it
//! was at the failure. The runbook is a clean git tree before the run; RFC-038
//! names git as the recovery mechanism. [`migrate_storage`] therefore refuses
//! to run without an explicit [`StorageMigrationOptions::allow_non_atomic`],
//! so no caller rewrites a tree in place by accident.

use crate::error::RepositoryError;
use crate::manifest::rfc038::RETIRED_PROPERTIES;
use crate::store::RepositoryStore;
use serde::Serialize;
use serde_json::Value;
use srs_core::types::relation::{Relation, RelationsCollection};
use std::collections::{BTreeMap, BTreeSet};

const MANIFEST_PATH: &str = "manifest.json";

/// The only `.srsj` generation this transform reads in addition to the current
/// one ([R21] migrator exemption).
const SRSJ_LEGACY_VERSION: &str = "1";

#[derive(Debug, Clone, Copy, Default)]
pub struct StorageMigrationOptions {
    /// Proceed even though the store cannot roll a failed run back
    /// (srs-rust#813). The caller is asserting that the repository is under
    /// version control with a clean tree, so `git revert` is available.
    pub allow_non_atomic: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageMigrationResult {
    /// Relations written as standalone objects.
    pub relations_exploded: usize,
    /// Collection files consumed and removed.
    pub collections_removed: Vec<String>,
    /// Retired properties actually removed, as `"<locator>:<property>"`.
    pub manifest_properties_stripped: Vec<String>,
    /// `.srsj` surface only: the envelope was generation 1.
    pub srsj_version_bumped: bool,
    /// `.srsj` surface only: a `data["manifest.json"]` shadow was reconciled away.
    pub shadow_manifest_removed: bool,
}

/// Abort with a diagnostic naming the surface and the reason — never skip,
/// never coerce, never partially migrate.
fn abort(locator: &str, reason: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::InvalidSnapshotData {
        message: format!("rfc038-storage migration aborted at {locator}: {reason}"),
    }
}

// ---------------------------------------------------------------------------
// Discriminator
// ---------------------------------------------------------------------------

/// The retired properties actually present in a raw manifest value, in
/// [`RETIRED_PROPERTIES`] order.
///
/// Raw, not typed: `"instanceIndex": []` is a retired key that is still there,
/// and a typed round-trip through `Manifest` cannot see the difference between
/// an empty index and an absent one.
fn retired_properties_in(manifest: &Value) -> Vec<&'static str> {
    RETIRED_PROPERTIES
        .iter()
        .copied()
        .filter(|prop| manifest.get(prop).is_some())
        .collect()
}

/// Remove every retired property, returning the ones that were there.
fn strip_retired_properties(manifest: &mut Value) -> Vec<&'static str> {
    let present = retired_properties_in(manifest);
    if let Some(obj) = manifest.as_object_mut() {
        for prop in &present {
            obj.remove(*prop);
        }
    }
    present
}

/// Is the migration needed? **Structural**, never revision-keyed: a retired
/// manifest property or a relations collection is present.
///
/// A store that is not a file tree (`MemoryStore`) has no `manifest.json` and
/// no relations files to place.
///
/// A probe, so it never fails on data it merely cannot read: an unreadable
/// file under `relations/` is the catalog's to diagnose
/// (`SRS038-R9-CANDIDATE-MALFORMED`, surfaced by `repo validate`), and
/// propagating it from here would take `repo migrations` down for every
/// migration. [`migrate_storage`] is where it aborts — and an unreadable file
/// counts as "needed" precisely so a client is told there is something to run,
/// rather than reporting a repository with a corrupt relations file as already
/// migrated.
///
/// `relations_candidates` runs unconditionally before the boolean combination
/// below, so `||` costs nothing here — the scan already happened either way —
/// and every disjunct has direct test coverage of its own.
pub fn migration_needed(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    if !store.is_file_tree_store() {
        return Ok(false);
    }
    let manifest = load_raw_manifest(store)?;
    let retired = !retired_properties_in(&manifest).is_empty();
    let candidates = relations_candidates(store, &manifest)?;
    Ok(retired || !candidates.collections.is_empty() || !candidates.unreadable.is_empty())
}

fn load_raw_manifest(store: &dyn RepositoryStore) -> Result<Value, RepositoryError> {
    store.load_instance_json(MANIFEST_PATH)
}

// ---------------------------------------------------------------------------
// Store surface
// ---------------------------------------------------------------------------

/// Run the transform in place. Idempotent: a migrated repository is a no-op.
pub fn migrate_storage(
    store: &dyn RepositoryStore,
    options: &StorageMigrationOptions,
) -> Result<StorageMigrationResult, RepositoryError> {
    // Checked first: this is the more fundamental precondition, and it has
    // nothing to do with rollback. A caller on a non-file-tree store who hits
    // the atomicity message instead would be told to commit a clean git tree
    // and pass `allow_non_atomic: true` — advice that cannot fix the actual
    // problem, since the very next check would still fail.
    if !store.is_file_tree_store() {
        return Err(RepositoryError::InvalidInput {
            message: "rfc038-storage is a file-placement transform and applies only to a \
                      file-tree store (a directory repository or a `.srsj` tree session)"
                .to_string(),
        });
    }
    if !options.allow_non_atomic {
        // ponytail: an unconditional refusal today because no store implements
        // batch rollback. When srs-rust#813 lands real staging, this becomes a
        // store capability probe and the guard starts discriminating.
        return Err(RepositoryError::InvalidInput {
            message:
                "rfc038-storage rewrites files in place and no store can roll a failed run back \
                 (srs-rust#813). Commit a clean git tree, then pass \
                 `StorageMigrationOptions { allow_non_atomic: true }`; `git revert` is the rollback."
                    .to_string(),
        });
    }

    store.begin_batch();
    match run(store) {
        Ok(result) => {
            store.commit_batch()?;
            Ok(result)
        }
        Err(e) => {
            store.abort_batch();
            Err(e)
        }
    }
}

fn run(store: &dyn RepositoryStore) -> Result<StorageMigrationResult, RepositoryError> {
    let mut result = StorageMigrationResult::default();
    let mut manifest = load_raw_manifest(store)?;

    // Read the declared relationsPath before stripping it.
    let candidates = relations_candidates(store, &manifest)?;
    // Abort, never skip: the probe tolerates a file it cannot read, but the
    // transform must not migrate around one. Skipping it would strip
    // `relationsPath`, orphan the file, and then report the repository
    // migrated.
    if let Some(path) = candidates.unreadable.first() {
        return Err(abort(
            path,
            "file under relations/ cannot be read as JSON, so it can be neither exploded \
             nor ruled out as a collection; repair it (see `repo validate`) and re-run",
        ));
    }
    // The manifest's own pointer to relations data is about to be stripped. If
    // it names a file that exists but is not a collection, stripping would
    // orphan that file and report success — silent loss of whatever it holds.
    if let Some(path) = &candidates.declared_not_a_collection {
        return Err(abort(
            path,
            "manifest.relationsPath names a file that is not a relations collection; \
             stripping the pointer would orphan it. Move its content into relations/ \
             (or remove the declaration) and re-run",
        ));
    }
    explode_relations(store, &candidates.collections, &mut result)?;

    let stripped = strip_retired_properties(&mut manifest);
    if !stripped.is_empty() {
        store.save_instance_json(MANIFEST_PATH, &manifest)?;
        result
            .manifest_properties_stripped
            .extend(stripped.iter().map(|p| format!("{MANIFEST_PATH}:{p}")));
    }
    Ok(result)
}

/// Every relations-collection file, discovered by **presence** rather than by
/// the manifest's word for it (srs-rust#809: a transform that enumerates from
/// one declared path misses every root the declaration does not mention).
///
/// A collection is any JSON file under `relations/` — or at a declared
/// `relationsPath` outside it — carrying a top-level `relations` array. A
/// standalone relation object has no such key, so the two forms cannot be
/// confused and a half-migrated repository is enumerated correctly.
///
/// A file that cannot be read is reported separately rather than folded into
/// either answer. The probe ([`migration_needed`]) tolerates it — diagnosing it
/// is the catalog's job, and failing here would take `repo migrations` down for
/// every migration. The transform ([`migrate_storage`]) aborts on it, because a
/// file it cannot read is one it cannot rule out as a collection.
fn relations_candidates(
    store: &dyn RepositoryStore,
    manifest: &Value,
) -> Result<RelationsCandidates, RepositoryError> {
    let mut candidates: BTreeSet<String> = store
        .list_files_recursive("relations")
        .into_iter()
        .filter(|p| p.ends_with(".json"))
        .collect();
    let declared =
        crate::catalog::declared_location(manifest.get("relationsPath").and_then(|v| v.as_str()));
    if let Some(declared) = &declared {
        candidates.insert(declared.clone());
    }

    let mut found = RelationsCandidates::default();
    for path in candidates {
        match store.load_instance_json(&path) {
            Ok(value) => {
                // Key presence, not shape: `store::list_relations` skips a
                // file on the `relations` key alone, so a stricter test here
                // would leave a `relations` value that is not an array
                // invisible to both — unreadable, and reported migrated. A
                // non-array is caught by the typed parse in `explode_relations`.
                if value.get("relations").is_some() {
                    found.collections.push(path);
                } else if declared.as_deref() == Some(path.as_str()) {
                    found.declared_not_a_collection = Some(path);
                }
                // Otherwise: an ordinary standalone relation object under
                // `relations/`, already in its final placement.
            }
            Err(e) if e.is_not_found() => {}
            // Any other failure — bad JSON, non-UTF-8 bytes, a directory where
            // a file was declared — is a file this transform cannot read.
            // Enumerating error kinds here is how the non-UTF-8 case escaped as
            // a raw IO error instead of the designed abort.
            Err(_) => found.unreadable.push(path),
        }
    }
    Ok(found)
}

#[derive(Default)]
struct RelationsCandidates {
    collections: Vec<String>,
    unreadable: Vec<String>,
    /// `manifest.relationsPath` resolves to a readable file that is not a
    /// collection — so stripping the pointer would orphan it.
    declared_not_a_collection: Option<String>,
}

fn explode_relations(
    store: &dyn RepositoryStore,
    collections: &[String],
    result: &mut StorageMigrationResult,
) -> Result<(), RepositoryError> {
    if collections.is_empty() {
        return Ok(());
    }

    // A collection sitting where a relation object belongs would be deleted
    // along with an object written over it, and would have made every
    // `load_standalone` probe fail with a misleading `$schema` complaint.
    // Refuse up front, before anything is written or removed — there is no
    // rollback to undo either with (srs-rust#813).
    for path in collections {
        if let Some(stem) = path
            .strip_prefix("relations/")
            .and_then(|p| p.strip_suffix(".json"))
        {
            if crate::store::require_canonical_relation_id(stem).is_ok() {
                return Err(abort(
                    path,
                    "a relations collection occupies the locator of the relation object \
                     with that id; relations/ holds one object per relation ([R11])",
                ));
            }
        }
    }

    // The pre-migration id set: standalone objects already present plus every
    // id in every collection. `list_relations` skips collection-shaped files,
    // so the two halves are disjoint by construction.
    let mut expected: BTreeSet<String> = standalone_ids(store)?;

    // ── Pass 1: read and validate everything, write nothing ──
    //
    // Every rejection this transform can make is decided here. `abort_batch` is
    // a no-op on every store (srs-rust#813), so an abort *between* writes would
    // leave the collection in place alongside a half-exploded set of objects —
    // a state worse than the one we started from: `relation_service` then sees
    // each exploded relation twice and fails every read with
    // `DuplicateRelationId`, while a re-run aborts on the same entry without
    // ever clearing it. Refuse up front or not at all.
    let mut to_write: BTreeMap<String, Relation> = BTreeMap::new();
    for path in collections {
        let value = store.load_instance_json(path)?;
        let collection: RelationsCollection = serde_json::from_value(value)
            .map_err(|e| abort(path, format!("relations collection fails to parse: {e}")))?;
        for relation in collection.relations {
            let id = relation.relation_id.clone();
            // `save_relation` would reject this too, but only once writing has
            // started — the id is a filename component, so an uncanonical one
            // is a path-traversal write primitive.
            crate::store::require_canonical_relation_id(&id)
                .map_err(|e| abort(path, format!("relation id '{id}': {e}")))?;
            // Checked before the on-disk lookup, so a second entry for an id
            // already seen is diagnosed as the duplicate it is rather than as a
            // collision with our own output.
            if let Some(previous) = to_write.get(&id) {
                if previous != &relation {
                    return Err(abort(
                        path,
                        format!("duplicate relationId '{id}' with different content"),
                    ));
                }
                continue;
            }
            // A relation already standing alone with the same id must agree,
            // or one of the two would be lost without anyone noticing. If it
            // agrees, it is already exactly where it needs to be — already
            // counted in `expected` via the pre-loop standalone scan — so
            // there is nothing to write and nothing new to explode.
            if let Some(existing) = load_standalone(store, &id)? {
                if existing != relation {
                    return Err(abort(
                        path,
                        format!(
                            "relation '{id}' exists at relations/{id}.json with different content"
                        ),
                    ));
                }
                continue;
            }
            expected.insert(id.clone());
            to_write.insert(id, relation);
        }
    }

    // ── Pass 2: write, verify, then remove the collections ──
    for relation in to_write.values() {
        // The one write mechanism for a relation: canonical-id validation,
        // pinned `$schema`, flat `relations/<id>.json` ([R11]).
        store.save_relation(relation)?;
        result.relations_exploded += 1;
    }

    // Read back before the collections go: a mismatch here is a bug rather than
    // bad data, and dropping the collections on top of it would turn it into
    // loss. `list_relations` ignores collection-shaped files, so the answer is
    // the same either side of the delete.
    let after: BTreeSet<String> = standalone_ids(store)?;
    if after != expected {
        return Err(abort(
            "relations/",
            format!(
                "relation id set changed: {} expected, {} written",
                expected.len(),
                after.len()
            ),
        ));
    }

    for path in collections {
        store.delete_relations_json(path)?;
        result.collections_removed.push(path.clone());
    }
    Ok(())
}

/// The ids of the standalone relation objects already in `relations/`.
///
/// `list_relations` fails on any `.json` file there that is neither
/// collection-shaped nor a valid `$schema`-pinned relation object — a stray
/// `README.json` is enough. Left raw, that surfaced as "relation object is
/// missing the required $schema", the same misleading diagnostic the
/// locator-collision guard exists to avoid.
fn standalone_ids(store: &dyn RepositoryStore) -> Result<BTreeSet<String>, RepositoryError> {
    match store.list_relations() {
        Ok(relations) => Ok(relations.into_iter().map(|r| r.relation_id).collect()),
        Err(e) => Err(abort(
            "relations/",
            format!(
                "cannot enumerate the standalone relation objects: {e}. Every .json file \
                 under relations/ must be a relation object or a collection; move anything \
                 else out (see `repo validate`) and re-run"
            ),
        )),
    }
}

fn load_standalone(
    store: &dyn RepositoryStore,
    relation_id: &str,
) -> Result<Option<Relation>, RepositoryError> {
    match store.load_relation(relation_id) {
        Ok(relation) => Ok(Some(relation)),
        Err(RepositoryError::RelationNotFound { .. }) => Ok(None),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// `.srsj` surface
// ---------------------------------------------------------------------------

/// Migrate a `.srsj` document, returning the rewritten document.
///
/// A pure function from document to document, so there is nothing to roll back
/// and no atomicity guard: the caller's file is untouched until it writes the
/// result. All the actual work is [`migrate_storage`]'s — this normalises the
/// envelope until [`crate::srsj::open_srsj`] will accept it (reconcile a
/// `data["manifest.json"]` shadow, set `srsj` to the current generation), then
/// hands the tree session over.
///
/// Reading a generation-1 envelope here is [R21]'s migrator exemption, taken
/// explicitly in the one component that holds it rather than by weakening the
/// codec's [R20] refusal.
pub fn migrate_srsj(content: &str) -> Result<(String, StorageMigrationResult), RepositoryError> {
    let mut envelope: Value = serde_json::from_str(content)
        .map_err(|e| abort("<srsj-input>", format!("invalid .srsj document: {e}")))?;

    let version = envelope
        .get("srsj")
        .and_then(|v| v.as_str())
        .ok_or_else(|| abort("<srsj-input>", "document declares no `srsj` version"))?
        .to_string();
    if version != SRSJ_LEGACY_VERSION && version != crate::srsj::SRSJ_VERSION {
        return Err(abort(
            "<srsj-input>",
            format!(
                "unsupported srsj version '{version}' — this transform converts '{}' and \
                 re-emits '{}' ([R20]/[R21])",
                SRSJ_LEGACY_VERSION,
                crate::srsj::SRSJ_VERSION
            ),
        ));
    }

    // Bumping the envelope on a pre-RFC-039 document produces a generation-2
    // carrier still at data-model revision <= 1 — which the [R21] gate rejects
    // at the Phase-6 flip, leaving nothing able to open it and `rfc039-carrier`
    // (the fix) permanently out of reach. Storage generation and data-model
    // revision advance in that order, so the data model has to be there first.
    let revision = envelope
        .get("manifest")
        .and_then(|m| m.get("dataModelRevision"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if revision < crate::rfc039_carrier_migration_service::CARRIER_REVISION {
        return Err(abort(
            "<srsj-input>",
            format!(
                "document is at dataModelRevision {revision}; bumping it to srsj '{}' would \
                 produce a carrier the [R21] generation gate rejects, with no way left to \
                 migrate the data model. Bring it to revision {} first — open it with a \
                 pre-cutover binary and run `field-type` then `rfc039-carrier` — and re-run",
                crate::srsj::SRSJ_VERSION,
                crate::rfc039_carrier_migration_service::CARRIER_REVISION,
            ),
        ));
    }

    let shadow_removed = reconcile_shadow_manifest(&mut envelope)?;
    envelope["srsj"] = Value::String(crate::srsj::SRSJ_VERSION.to_string());

    let store = crate::srsj::open_srsj(&envelope.to_string())?;
    let mut result = migrate_storage(
        &store,
        &StorageMigrationOptions {
            allow_non_atomic: true,
        },
    )?;
    result.srsj_version_bumped = version == SRSJ_LEGACY_VERSION;
    result.shadow_manifest_removed = shadow_removed;
    Ok((crate::srsj::to_srsj_string(&store)?, result))
}

/// Resolve a `data["manifest.json"]` shadow, which the codec refuses outright
/// ([R19]) and which therefore makes a document unopenable until a migration
/// removes it.
///
/// The envelope manifest is the only manifest, so the shadow goes — but only
/// once it is known to say nothing the envelope does not. The comparison is
/// **verbatim**: an earlier version normalised both sides by stripping the
/// retired properties first, which quietly discarded a shadow whose only
/// disagreement was a different `relationsPath` — and with it the collection
/// that path named. A retired property is not noise just because this
/// transform is about to remove it; `relationsPath` points at data. Any
/// disagreement is an abort — dropping a differing shadow is silent loss, and
/// this is a pathological state a human should look at.
fn reconcile_shadow_manifest(envelope: &mut Value) -> Result<bool, RepositoryError> {
    let Some(data) = envelope.get_mut("data").and_then(|v| v.as_object_mut()) else {
        return Ok(false);
    };
    // `./manifest.json` is the same key by another spelling — the codec
    // normalises before it refuses, so this must too.
    let shadow_keys: Vec<String> = data
        .keys()
        .filter(|k| crate::vfs::ensure_contained(k).ok().as_deref() == Some(MANIFEST_PATH))
        .cloned()
        .collect();
    if shadow_keys.is_empty() {
        return Ok(false);
    }

    let envelope_manifest = envelope
        .get("manifest")
        .cloned()
        .ok_or_else(|| abort("<srsj-input>", "document declares no `manifest`"))?;

    let data = envelope
        .get_mut("data")
        .and_then(|v| v.as_object_mut())
        .expect("data object was present a moment ago");
    for key in &shadow_keys {
        let shadow = data
            .remove(key)
            .expect("key came from this map's own key list");
        if shadow != envelope_manifest {
            return Err(abort(
                &format!("data[\"{key}\"]"),
                "shadow manifest disagrees with the envelope manifest; the envelope manifest \
                 is the only manifest ([R19]) and dropping a differing shadow would lose data",
            ));
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srsj::open_srsj;
    use serde_json::json;

    const REL_A: &str = "11111111-1111-4111-8111-111111111111";
    const REL_B: &str = "22222222-2222-4222-8222-222222222222";
    const INST_A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const INST_B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn relation(id: &str) -> Value {
        json!({
            "relationId": id,
            "relationType": "precedes",
            "sourceInstanceId": INST_A,
            "targetInstanceId": INST_B,
            "createdAt": "2026-01-01T00:00:00Z",
        })
    }

    fn note(id: &str, title: &str) -> Value {
        json!({
            "instanceId": id,
            "title": title,
            "sections": [{ "name": "body", "content": "content" }],
        })
    }

    /// A generation-1 document in the pre-cutover layout: stamped
    /// `dataModelRevision: 2` (srs#242 Phase B) *and* still carrying the index,
    /// plus a relations collection.
    fn legacy_srsj() -> Value {
        json!({
            "srsj": "1",
            "manifest": {
                "srsVersion": "2.0-draft",
                "repositoryId": "00000000-0000-4000-8000-00000000aaaa",
                "namespace": "com.example.test",
                "dataModelRevision": 2,
                "instanceIndex": [
                    { "instanceId": INST_A, "tier": 0, "path": "records/notes/a.json" },
                    { "instanceId": INST_B, "tier": 0, "path": "records/notes/b.json" },
                ],
                "relationsPath": "relations/relations.json",
            },
            "data": {
                "package/package.json": {
                    "$schema": srs_schema::PACKAGE_MANIFEST_SCHEMA_ID,
                    "id": "00000000-0000-4000-8000-00000000bbbb",
                    "namespace": "com.example.test",
                    "name": "test-package",
                    "version": "1.0.0",
                    "title": "Test Package",
                    "description": "Fixture package for the rfc038-storage transform.",
                    "status": "draft",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "fields": [],
                    "types": [],
                },
                "records/notes/a.json": note(INST_A, "A"),
                "records/notes/b.json": note(INST_B, "B"),
                "relations/relations.json": {
                    "relations": [relation(REL_A), relation(REL_B)],
                },
            },
        })
    }

    /// The same repository already in the final layout, as a store.
    fn migrated_store(doc: &Value) -> crate::store::FileStore {
        let (out, _) = migrate_srsj(&doc.to_string()).expect("migration succeeds");
        open_srsj(&out).expect("migrated document opens")
    }

    fn ids(catalog: &crate::catalog::RepositoryCatalog) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (set, entries) in [
            ("instance", &catalog.instances),
            ("relation", &catalog.relations),
            ("container", &catalog.containers),
            ("source-document", &catalog.source_documents),
            ("definition", &catalog.definitions),
            ("extension", &catalog.extensions),
        ] {
            out.extend(
                entries
                    .iter()
                    .map(|e| (set.to_string(), format!("{}:{}", e.kind.as_str(), e.id))),
            );
        }
        out.sort();
        out
    }

    // --- Discriminator -----------------------------------------------------

    #[test]
    fn migration_needed_is_structural_not_revision_keyed() {
        // Stamped `dataModelRevision: 2` with the index still present — the
        // state srs#242 Phase B left the corpus in. A revision-keyed
        // discriminator would call this already migrated.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let store = open_srsj(&doc.to_string()).unwrap();
        assert_eq!(
            store.load_instance_json("manifest.json").unwrap()["dataModelRevision"],
            2
        );
        assert!(migration_needed(&store).unwrap());
    }

    #[test]
    fn migration_needed_is_false_once_migrated() {
        let store = migrated_store(&legacy_srsj());
        assert!(!migration_needed(&store).unwrap());
    }

    #[test]
    fn an_empty_index_still_counts_as_a_retired_key() {
        // `"instanceIndex": []` is the key still being there. A typed
        // round-trip cannot tell it from an absent one.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        doc["manifest"]["instanceIndex"] = json!([]);
        doc["manifest"]
            .as_object_mut()
            .unwrap()
            .remove("relationsPath");
        doc["data"]
            .as_object_mut()
            .unwrap()
            .remove("relations/relations.json");
        let store = open_srsj(&doc.to_string()).unwrap();
        assert!(migration_needed(&store).unwrap());
    }

    #[test]
    fn every_retired_property_is_stripped() {
        // Driven by the same list the [R2] deny is, so the two cannot drift.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        for prop in RETIRED_PROPERTIES {
            doc["manifest"][*prop] = json!([]);
        }
        let store = migrated_store(&doc);
        let manifest = store.load_instance_json("manifest.json").unwrap();
        for prop in RETIRED_PROPERTIES {
            assert!(
                manifest.get(*prop).is_none(),
                "'{prop}' survived the migration"
            );
        }
    }

    #[test]
    fn migration_needed_is_false_for_a_store_that_is_not_a_file_tree() {
        let store = crate::store::memory::MemoryStore::default();
        assert!(!migration_needed(&store).unwrap());
    }

    // --- The transform -----------------------------------------------------

    #[test]
    fn a_collection_explodes_into_one_object_per_relation() {
        let (out, result) = migrate_srsj(&legacy_srsj().to_string()).unwrap();
        assert_eq!(result.relations_exploded, 2);
        assert_eq!(result.collections_removed, vec!["relations/relations.json"]);
        assert!(result.srsj_version_bumped);

        let doc: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(doc["srsj"], "2");
        assert!(doc["data"].get("relations/relations.json").is_none());
        for id in [REL_A, REL_B] {
            let object = &doc["data"][format!("relations/{id}.json")];
            assert_eq!(object["relationId"], id);
            assert_eq!(
                object["$schema"],
                crate::store::RELATION_OBJECT_SCHEMA_URL,
                "the standalone object form is const-pinned (Change E)"
            );
        }
    }

    #[test]
    fn the_catalog_is_identical_to_the_pre_migration_baseline() {
        // The identity baseline: every id in every set, before and after. The
        // transform moves files; it must not touch what the repository *is*.
        let doc = legacy_srsj();
        let mut pre = doc.clone();
        pre["srsj"] = json!("2");
        // Reading the pre-migration state is the [R21] migrator exemption —
        // taken explicitly, as the transform itself does.
        let before = open_srsj(&pre.to_string())
            .unwrap()
            .with_rfc038_exemption()
            .catalog()
            .unwrap();
        let after = migrated_store(&doc).catalog().unwrap();

        assert_eq!(ids(&before), ids(&after));
        assert_eq!(before.validity_token(), after.validity_token());
    }

    #[test]
    fn untouched_objects_keep_their_content() {
        let doc = legacy_srsj();
        let after: Value =
            serde_json::from_str(&migrate_srsj(&doc.to_string()).unwrap().0).unwrap();
        for path in [
            "records/notes/a.json",
            "records/notes/b.json",
            "package/package.json",
        ] {
            // Object equality is key-order-independent, which is what lets the
            // codec canonicalise without this test caring.
            assert_eq!(
                after["data"][path], doc["data"][path],
                "'{path}' must ride through untouched"
            );
        }
    }

    #[test]
    fn migration_is_idempotent() {
        let (once, first) = migrate_srsj(&legacy_srsj().to_string()).unwrap();
        let (twice, second) = migrate_srsj(&once).unwrap();
        assert_eq!(once, twice, "a migrated document is a fixed point");
        assert_eq!(first.relations_exploded, 2);
        assert_eq!(second, StorageMigrationResult::default());
    }

    #[test]
    fn a_repository_with_no_collection_only_strips_the_manifest() {
        let mut doc = legacy_srsj();
        doc["data"]
            .as_object_mut()
            .unwrap()
            .remove("relations/relations.json");
        let (_, result) = migrate_srsj(&doc.to_string()).unwrap();
        assert_eq!(result.relations_exploded, 0);
        assert!(result.collections_removed.is_empty());
        assert!(result
            .manifest_properties_stripped
            .contains(&"manifest.json:instanceIndex".to_string()));
    }

    #[test]
    fn a_collection_outside_relations_is_found_through_the_declared_path() {
        // srs-rust#809: enumerate by presence, but do not miss a location only
        // the manifest names.
        let mut doc = legacy_srsj();
        let collection = doc["data"]["relations/relations.json"].clone();
        doc["data"]
            .as_object_mut()
            .unwrap()
            .remove("relations/relations.json");
        doc["data"]["legacy/edges.json"] = collection;
        doc["manifest"]["relationsPath"] = json!("legacy/edges.json");

        let (out, result) = migrate_srsj(&doc.to_string()).unwrap();
        assert_eq!(result.relations_exploded, 2);
        assert_eq!(result.collections_removed, vec!["legacy/edges.json"]);
        let after: Value = serde_json::from_str(&out).unwrap();
        assert!(after["data"].get("legacy/edges.json").is_none());
        assert!(after["data"]
            .get(format!("relations/{REL_A}.json"))
            .is_some());
    }

    #[test]
    fn a_relation_already_standing_alone_is_not_duplicated() {
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let mut standalone = relation(REL_A);
        standalone["$schema"] = json!(crate::store::RELATION_OBJECT_SCHEMA_URL);
        doc["data"][format!("relations/{REL_A}.json")] = standalone;

        let store = migrated_store(&doc);
        let relations = store.list_relations().unwrap();
        assert_eq!(relations.len(), 2, "REL_A must not be counted twice");
    }

    #[test]
    fn a_conflicting_standalone_relation_aborts() {
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let mut standalone = relation(REL_A);
        standalone["relationType"] = json!("refines");
        standalone["$schema"] = json!(crate::store::RELATION_OBJECT_SCHEMA_URL);
        doc["data"][format!("relations/{REL_A}.json")] = standalone;

        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(err.to_string().contains("different content"), "got: {err}");
    }

    #[test]
    fn a_duplicate_id_with_different_content_aborts() {
        let mut doc = legacy_srsj();
        let mut conflicting = relation(REL_A);
        conflicting["relationType"] = json!("refines");
        doc["data"]["relations/relations.json"]["relations"] =
            json!([relation(REL_A), conflicting]);
        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("duplicate relationId"),
            "got: {err}"
        );
    }

    #[test]
    fn a_collection_at_a_relation_object_locator_aborts_before_anything_is_written() {
        // `relations/<uuid>.json` is where the object with that id belongs, so
        // a collection there would be deleted along with the object written
        // over it. Refused up front, with a message that names the real
        // problem instead of a `$schema` complaint from a probe.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let collection = doc["data"]["relations/relations.json"].clone();
        doc["data"]
            .as_object_mut()
            .unwrap()
            .remove("relations/relations.json");
        doc["data"][format!("relations/{REL_A}.json")] = collection;
        doc["manifest"]["relationsPath"] = json!(format!("relations/{REL_A}.json"));

        let store = open_srsj(&doc.to_string()).unwrap();
        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("occupies the locator"),
            "got: {err}"
        );
        assert!(
            store
                .load_instance_json(&format!("relations/{REL_A}.json"))
                .unwrap()
                .get("relations")
                .is_some(),
            "the collection must still be there, untouched"
        );
    }

    /// A store with an unparseable file under `relations/` and **no** retired
    /// manifest property, so the collection scan is genuinely reached.
    fn store_with_an_unparseable_relations_file() -> crate::store::FileStore {
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        for prop in RETIRED_PROPERTIES {
            doc["manifest"].as_object_mut().unwrap().remove(*prop);
        }
        doc["data"]
            .as_object_mut()
            .unwrap()
            .remove("relations/relations.json");
        doc["data"]["relations/broken.json"] = json!("{ not json");
        open_srsj(&doc.to_string()).unwrap()
    }

    #[test]
    fn an_unparseable_relations_file_does_not_break_the_status_probe() {
        // `repo migrations` asks every migration for a status. One unparseable
        // file must not take the whole command down — the catalog is what
        // diagnoses it, and `repo validate` is where it surfaces.
        let store = store_with_an_unparseable_relations_file();
        // Not just Ok — TRUE. The fixture has no retired properties and no
        // readable collection, so the unreadable file is the only signal; if
        // it did not count, a repository with a corrupt relations file would
        // report itself migrated while the transform refuses to run on it.
        assert!(
            migration_needed(&store).expect("the probe must tolerate a file it cannot read"),
            "an unreadable relations file must read as 'needed', not 'migrated'"
        );
        assert!(
            store.catalog().is_err(),
            "and the catalog still reports it as a fatal diagnostic"
        );
    }

    #[test]
    fn an_unparseable_relations_file_aborts_the_transform() {
        // The probe tolerates it; the transform must not migrate around it.
        // Skipping would strip `relationsPath`, orphan the file, and then
        // report the repository migrated — abort, never skip.
        let store = store_with_an_unparseable_relations_file();
        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("relations/broken.json"),
            "got: {err}"
        );
        assert!(
            store.load_text_file("relations/broken.json").is_ok(),
            "and nothing was touched"
        );
    }

    #[test]
    fn a_rejected_entry_leaves_no_relation_objects_behind() {
        // Validation happens before any write. An abort between writes would
        // leave the collection alongside a half-exploded set of objects, and
        // `relation_service` would then see each of those twice and fail every
        // read with DuplicateRelationId — worse than not having run at all,
        // and with no rollback (srs-rust#813) to undo it.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let mut bad = relation(REL_B);
        bad["relationId"] = json!("not-a-uuid");
        // REL_A is valid and sorts first; the rejection comes after it.
        doc["data"]["relations/relations.json"]["relations"] = json!([relation(REL_A), bad]);
        let store = open_srsj(&doc.to_string()).unwrap();

        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("not-a-uuid"), "got: {err}");
        assert!(
            store
                .load_instance_json(&format!("relations/{REL_A}.json"))
                .is_err(),
            "the valid entry ahead of the rejection must not have been written"
        );
        assert!(
            store.load_instance_json("relations/relations.json").is_ok(),
            "and the collection must still be there"
        );
    }

    #[test]
    fn a_relations_file_whose_relations_value_is_not_an_array_is_not_invisible() {
        // `store::list_relations` skips a file on the `relations` key alone. A
        // stricter test here would make a non-array value invisible to both —
        // an unreachable relations file in a repository reporting itself
        // migrated.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        doc["data"]["relations/relations.json"]["relations"] = json!({ "not": "an array" });
        let store = open_srsj(&doc.to_string()).unwrap();

        assert!(migration_needed(&store).unwrap());
        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("fails to parse"), "got: {err}");
    }

    #[test]
    fn a_relations_file_that_is_not_utf8_is_handled_like_any_other_unreadable_one() {
        // Classifying only JSON *parse* failures as unreadable let a non-UTF-8
        // file escape as a raw IO error: `migration_needed` returned Err, which
        // at the Phase-6 flip would take `repo migrations` down for every
        // migration — the exact failure the probe exists to prevent.
        let dir = tempfile::TempDir::new().unwrap();
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let session = open_srsj(&doc.to_string()).unwrap();
        for (path, bytes) in crate::tree_session::export_tree(&session).unwrap() {
            let target = dir.path().join(&path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }
        std::fs::write(dir.path().join("relations/legacy.json"), [0xff, 0xfe, 0x00]).unwrap();

        let store = crate::store::FileStore::new(dir.path());
        assert!(
            migration_needed(&store).is_ok(),
            "the probe must not fail on a file it cannot read"
        );
        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("relations/legacy.json"),
            "got: {err}"
        );
        assert!(err.to_string().contains("cannot be read"), "got: {err}");
    }

    #[test]
    fn a_declared_relations_path_that_is_not_a_collection_aborts() {
        // The pointer is about to be stripped; if it names a file that is not a
        // collection, stripping orphans whatever that file holds.
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        doc["data"]
            .as_object_mut()
            .unwrap()
            .remove("relations/relations.json");
        let mut orphan = relation(REL_A);
        orphan["$schema"] = json!(crate::store::RELATION_OBJECT_SCHEMA_URL);
        doc["data"]["legacy/edges.json"] = orphan;
        doc["manifest"]["relationsPath"] = json!("legacy/edges.json");
        let store = open_srsj(&doc.to_string()).unwrap();

        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("would orphan it"), "got: {err}");
        assert!(store.load_instance_json("legacy/edges.json").is_ok());
    }

    #[test]
    fn a_stray_file_under_relations_aborts_with_a_diagnostic_that_names_it() {
        // `list_relations` fails on any .json there that is neither a
        // collection nor a valid relation object. Left raw, that surfaced as a
        // bare "missing the required $schema".
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        doc["data"]["relations/README.json"] = json!({ "note": "not a relation" });
        let store = open_srsj(&doc.to_string()).unwrap();

        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("relations/"), "got: {err}");
        assert!(err.to_string().contains("move anything"), "got: {err}");
    }

    #[test]
    fn a_pre_rfc039_document_is_not_bumped_into_an_unopenable_state() {
        // Generation 2 at data-model revision 1 is rejected by the [R21] gate
        // at the Phase-6 flip, and `rfc039-carrier` — the fix — could no longer
        // open it. The data model goes first.
        let mut doc = legacy_srsj();
        doc["manifest"]["dataModelRevision"] = json!(1);
        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("dataModelRevision 1"),
            "got: {err}"
        );
        assert!(err.to_string().contains("rfc039-carrier"), "got: {err}");
    }

    #[test]
    fn a_malformed_collection_aborts_rather_than_being_skipped() {
        let mut doc = legacy_srsj();
        doc["data"]["relations/relations.json"]["relations"] = json!([{ "nope": true }]);
        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(err.to_string().contains("fails to parse"), "got: {err}");
    }

    #[test]
    fn an_aborted_run_never_reports_success_or_looks_migrated() {
        // The `.srsj` surface is atomic by construction — a failed run yields
        // no document, so the caller's file never sees a partial state.
        let mut doc = legacy_srsj();
        doc["data"]["relations/relations.json"]["relations"] =
            json!([relation(REL_A), { "nope": true }]);
        assert!(migrate_srsj(&doc.to_string()).is_err());

        // The store surface cannot roll back (srs-rust#813). What it must
        // never do is remove the collection or leave the repository looking
        // migrated: a failed run stays diagnosable and re-runnable, and `git
        // revert` restores the tree.
        doc["srsj"] = json!("2");
        let store = open_srsj(&doc.to_string()).unwrap();
        assert!(migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .is_err());
        assert!(
            store.load_instance_json("relations/relations.json").is_ok(),
            "a failed run must not remove the collection"
        );
        assert!(
            migration_needed(&store).unwrap(),
            "a failed run must still report the repository as needing migration"
        );
    }

    // --- `.srsj` envelope --------------------------------------------------

    #[test]
    fn a_generation_1_document_is_readable_only_through_the_transform() {
        let doc = legacy_srsj().to_string();
        assert!(
            open_srsj(&doc).is_err(),
            "the codec must keep refusing srsj '1' ([R20])"
        );
        assert!(migrate_srsj(&doc).is_ok(), "the transform is the exemption");
    }

    #[test]
    fn an_unrecognised_generation_is_refused() {
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("3");
        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("unsupported srsj version '3'"),
            "got: {err}"
        );
    }

    #[test]
    fn an_agreeing_shadow_manifest_is_reconciled_away() {
        // A `data["manifest.json"]` shadow makes a document unopenable ([R19]);
        // the transform is what removes it.
        let mut doc = legacy_srsj();
        doc["data"]["manifest.json"] = doc["manifest"].clone();

        let (out, result) = migrate_srsj(&doc.to_string()).unwrap();
        assert!(result.shadow_manifest_removed);
        let after: Value = serde_json::from_str(&out).unwrap();
        assert!(after["data"].get("manifest.json").is_none());
        assert_eq!(
            after["manifest"]["repositoryId"],
            doc["manifest"]["repositoryId"]
        );
        assert!(after["manifest"].get("instanceIndex").is_none());
    }

    #[test]
    fn a_shadow_manifest_differing_only_in_relations_path_aborts() {
        // The comparison used to strip the retired properties from both sides
        // first, so a shadow that disagreed *only* about `relationsPath` looked
        // identical — and was dropped along with the collection it named. A
        // retired property is not noise: `relationsPath` points at data.
        let mut doc = legacy_srsj();
        let mut shadow = doc["manifest"].clone();
        shadow["relationsPath"] = json!("legacy/edges.json");
        doc["data"]["manifest.json"] = shadow;
        doc["data"]["legacy/edges.json"] = json!({ "relations": [relation(REL_A)] });

        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("shadow manifest disagrees"),
            "got: {err}"
        );
    }

    #[test]
    fn a_disagreeing_shadow_manifest_aborts() {
        let mut doc = legacy_srsj();
        let mut shadow = doc["manifest"].clone();
        shadow["repositoryId"] = json!("00000000-0000-4000-8000-00000000ffff");
        doc["data"]["manifest.json"] = shadow;

        let err = migrate_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("shadow manifest disagrees"),
            "got: {err}"
        );
    }

    #[test]
    fn a_dot_slash_spelling_of_the_shadow_is_the_same_shadow() {
        let mut doc = legacy_srsj();
        doc["data"]["./manifest.json"] = doc["manifest"].clone();
        let (out, result) = migrate_srsj(&doc.to_string()).unwrap();
        assert!(result.shadow_manifest_removed);
        assert!(serde_json::from_str::<Value>(&out).unwrap()["data"]
            .get("manifest.json")
            .is_none());
    }

    // --- RFC-038 acceptance test 6 ----------------------------------------

    #[test]
    fn a_sidecar_whose_content_file_is_absent_survives_as_a_source_document() {
        // [R15]: the sidecar is the identity and an absent content file is a
        // valid tombstone. The transform must carry it through untouched.
        const DOC_ID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let mut doc = legacy_srsj();
        doc["data"]["source-documents/gone.md.meta.json"] = json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/source-document-meta.json",
            "documentId": DOC_ID,
            "contentPath": "source-documents/gone.md",
            "contentType": "text/markdown",
            "createdAt": "2026-01-01T00:00:00Z",
        });

        let store = migrated_store(&doc);
        let catalog = store.catalog().unwrap();
        assert_eq!(
            catalog
                .source_documents
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            vec![DOC_ID],
            "the tombstone must survive as a valid source document"
        );
        assert!(
            store.load_text_file("source-documents/gone.md").is_err(),
            "and its content file must still be absent"
        );
    }

    // --- Guards ------------------------------------------------------------

    #[test]
    fn the_store_surface_refuses_without_the_explicit_opt_in() {
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let store = open_srsj(&doc.to_string()).unwrap();
        let err = migrate_storage(&store, &StorageMigrationOptions::default()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("srs-rust#813"), "got: {message}");
        assert!(message.contains("git revert"), "got: {message}");
        // And nothing was written.
        assert!(store.load_instance_json("relations/relations.json").is_ok());
    }

    #[test]
    fn the_store_surface_refuses_a_store_that_is_not_a_file_tree() {
        let store = crate::store::memory::MemoryStore::default();
        let err = migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("file-tree store"), "got: {err}");
    }

    #[test]
    fn a_directory_repository_migrates_the_same_way_as_a_document() {
        // The store surface is the transform; `.srsj` is only a carrier. A
        // disk repository must reach the identical end state.
        let dir = tempfile::TempDir::new().unwrap();
        let mut doc = legacy_srsj();
        doc["srsj"] = json!("2");
        let session = open_srsj(&doc.to_string()).unwrap();
        for (path, bytes) in crate::tree_session::export_tree(&session).unwrap() {
            let target = dir.path().join(&path);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, bytes).unwrap();
        }

        let store = crate::store::FileStore::new(dir.path());
        migrate_storage(
            &store,
            &StorageMigrationOptions {
                allow_non_atomic: true,
            },
        )
        .unwrap();

        assert!(!dir.path().join("relations/relations.json").exists());
        assert!(dir.path().join(format!("relations/{REL_A}.json")).exists());
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("manifest.json")).unwrap())
                .unwrap();
        assert!(manifest.get("instanceIndex").is_none());
        assert!(manifest.get("relationsPath").is_none());

        // Same catalog identity as the document surface.
        assert_eq!(
            ids(&store.catalog().unwrap()),
            ids(&migrated_store(&legacy_srsj()).catalog().unwrap())
        );
    }
}
