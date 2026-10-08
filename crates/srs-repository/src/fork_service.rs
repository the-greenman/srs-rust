//! # Fork service (srs-rust#1136)
//!
//! "Make local copy": clone records into NEW records linked `derived-from` their originals,
//! and swap the clones into ONE container in place. Container copy
//! ([`crate::container_service::copy_container`]) forks its anchor through the same core.
//!
//! One fork core ([`fork_records`]) = [`record_store::create_record_successor`] with
//! `derived-from` (actor stamping from the session, relation validation, same type and
//! fieldValues, initial lifecycle state). v1: Tier 2 records only; fieldValues only.

use crate::container_service;
use crate::error::RepositoryError;
use crate::record_store::{self, attempt_rollback_delete, CreateRecordSuccessorInput};
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::arrangement;
use srs_core::types::relation::Relation;
use std::collections::BTreeMap;

/// The relation type a fork asserts (fork -> original).
pub const FORK_RELATION_TYPE: &str = "derived-from";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkPair {
    pub original_id: String,
    pub fork_id: String,
    /// Ids of the relations carried onto this fork (srs-rust#1354); empty by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried_relation_ids: Vec<String>,
}

/// Which of the original's relations a fork re-creates (srs-rust#1354, ADR-054).
/// `derived-from` is never carried.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
pub enum CarryRelations {
    /// Today's behaviour: the fork is linked only `derived-from` its original.
    #[default]
    None,
    /// Relations where the original is the source.
    Outgoing,
    /// Relations where the original is the source or the target.
    All,
}

/// Options of [`fork_subtree`].
#[derive(Debug, Clone, Default)]
pub struct ForkOptions {
    /// Fork into this container; when the original is not a member it is appended to the
    /// container's outline first. Wins over the `container_id` argument.
    pub target_container: Option<String>,
    pub carry_relations: CarryRelations,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkResult {
    pub container_id: String,
    pub forks: Vec<ForkPair>,
    pub relations: Vec<Relation>,
    /// The relations carried onto the forks (srs-rust#1354); omitted when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried_relations: Vec<Relation>,
}

/// THE fork core. Forks `ids` in order; on any failure every fork already created is
/// removed again (best effort, ADR-024) and nothing is left behind.
pub fn fork_records(
    store: &dyn RepositoryStore,
    ids: &[String],
) -> Result<(Vec<ForkPair>, Vec<Relation>), RepositoryError> {
    let mut pairs: Vec<ForkPair> = Vec::new();
    let mut relations = Vec::new();
    for id in ids {
        let step = (|| {
            let original = record_store::get_record_by_id(store, id)?.ok_or_else(|| {
                RepositoryError::InvalidInput {
                    message: format!("{id} is not a Tier 2 record; only records can be forked"),
                }
            })?;
            record_store::create_record_successor(
                store,
                id,
                CreateRecordSuccessorInput {
                    relation_type: Some(FORK_RELATION_TYPE.to_string()),
                    field_values: original.field_values.clone(),
                    lifecycle_state: None,
                    type_version: None,
                    extra: BTreeMap::new(),
                },
            )
        })();
        match step {
            Ok(r) => {
                pairs.push(ForkPair {
                    original_id: id.clone(),
                    fork_id: r.record.instance_id,
                    carried_relation_ids: Vec::new(),
                });
                relations.push(r.relation);
            }
            Err(e) => {
                for p in &pairs {
                    attempt_rollback_delete(store, &p.fork_id);
                }
                return Err(e);
            }
        }
    }
    Ok((pairs, relations))
}

/// Re-create the relations of the originals on their forks (srs-rust#1354). A relation is
/// carried when its source is a forked original (or, for `All`, its target); an end that is
/// itself forked in this call is re-pointed to that fork, so a relation between two forked
/// records is created once, between both forks. `derived-from` is never carried and the
/// originals' relations are untouched. Created through the normal relation path, so type
/// validation and session-actor stamping apply. `created` collects what was made (rollback).
fn carry_relations(
    store: &dyn RepositoryStore,
    forks: &mut [ForkPair],
    mode: CarryRelations,
    created: &mut Vec<Relation>,
) -> Result<(), RepositoryError> {
    if mode == CarryRelations::None {
        return Ok(());
    }
    let map: BTreeMap<String, usize> = forks
        .iter()
        .enumerate()
        .map(|(i, p)| (p.original_id.clone(), i))
        .collect();
    let fork_ids: Vec<String> = forks.iter().map(|p| p.fork_id.clone()).collect();
    let sub = |id: &str| {
        map.get(id)
            .map_or_else(|| id.to_string(), |&i| fork_ids[i].clone())
    };
    let todo: Vec<Relation> = crate::relation_service::load_relations(store)?
        .into_iter()
        .filter(|r| {
            r.relation_type != FORK_RELATION_TYPE
                && (map.contains_key(&r.source_instance_id)
                    || (mode == CarryRelations::All && map.contains_key(&r.target_instance_id)))
        })
        .collect();
    for r in todo {
        // The fork the relation hangs off (source side first) lists it.
        let owner = map
            .get(&r.source_instance_id)
            .or_else(|| map.get(&r.target_instance_id))
            .copied();
        let mut c = r.clone();
        c.relation_id = String::new();
        c.created_at = None;
        c.created_by = None;
        c.source_instance_id = sub(&r.source_instance_id);
        c.target_instance_id = sub(&r.target_instance_id);
        let made = crate::relation_service::create_relation_auto(store, c)?.relation;
        if let Some(i) = owner {
            forks[i].carried_relation_ids.push(made.relation_id.clone());
        }
        created.push(made);
    }
    Ok(())
}

/// Fork the arrangement subtree rooted at `root_instance_id` and swap the forks into ONE
/// container in place (same order and depth). That container is `opts.target_container`
/// when given (the original is appended to its outline first if it is not yet a member),
/// else `container_id`. Other containers keep the originals. The anchor/identity entries
/// cannot be forked this way, and the repository root container is refused. With
/// `opts.carry_relations` the originals' relations are re-created on the forks
/// ([`carry_relations`]). Everything created is removed again on failure (best effort,
/// ADR-024). v1: a forked entry that anchors a child container leaves that container
/// pointing at the original. Forking only READS the originals, so the session write guard
/// checks the target container alone.
pub fn fork_subtree(
    store: &dyn RepositoryStore,
    container_id: Option<&str>,
    root_instance_id: &str,
    opts: &ForkOptions,
) -> Result<ForkResult, RepositoryError> {
    let container_id = opts
        .target_container
        .as_deref()
        .or(container_id)
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: "a container is required: the container the fork is swapped into".into(),
        })?;
    if container_service::is_root_container(store, container_id) {
        return Err(RepositoryError::ContainerIsRepositoryRoot {
            container_id: container_id.to_string(),
        });
    }
    let mut container = container_service::get_container(store, container_id)?;
    let mut added = false;
    if opts.target_container.is_some()
        && !container
            .member_instance_ids
            .iter()
            .flatten()
            .any(|e| e.instance_id == root_instance_id)
    {
        if record_store::get_record_by_id(store, root_instance_id)?.is_none() {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "{root_instance_id} is not a Tier 2 record; only records can be forked"
                ),
            });
        }
        container_service::add_member(store, container_id, root_instance_id, None, None)?;
        added = true;
        container = container_service::get_container(store, container_id)?;
    }
    let undo_add = |store: &dyn RepositoryStore| {
        if added {
            let _ = container_service::remove_member(store, container_id, root_instance_id);
        }
    };
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    let Some(idx) = entries
        .iter()
        .position(|e| e.instance_id == root_instance_id)
    else {
        return Err(RepositoryError::InvalidInput {
            message: format!("{root_instance_id} is not a member of container {container_id}"),
        });
    };
    let ids: Vec<String> = entries[idx..idx + arrangement::run_len(&entries, idx)]
        .iter()
        .map(|e| e.instance_id.clone())
        .collect();
    // Pre-validate the whole run before creating anything.
    for id in &ids {
        if container.anchor_instance_id.as_deref() == Some(id)
            || container.identity_instance_id.as_deref() == Some(id)
        {
            undo_add(store);
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "{id} is the anchor/identity of {container_id}; it cannot be forked in place"
                ),
            });
        }
    }
    let (mut forks, relations) = match fork_records(store, &ids) {
        Ok(v) => v,
        Err(e) => {
            undo_add(store);
            return Err(e);
        }
    };
    let mut carried = Vec::new();
    let swaps: Vec<(String, String)> = forks
        .iter()
        .map(|p| (p.original_id.clone(), p.fork_id.clone()))
        .collect();
    let done = carry_relations(store, &mut forks, opts.carry_relations, &mut carried)
        .and_then(|()| container_service::replace_members(store, container_id, &swaps).map(|_| ()));
    if let Err(e) = done {
        for r in &carried {
            let _ = crate::relation_service::delete_relation(store, &r.relation_id);
        }
        for p in &forks {
            attempt_rollback_delete(store, &p.fork_id);
        }
        undo_add(store);
        return Err(e);
    }
    Ok(ForkResult {
        container_id: container_id.to_string(),
        forks,
        relations,
        carried_relations: carried,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_service::{self, ContainerCopyInput};
    use crate::record_store::tests::{create_lc_record, make_store_with_lifecycle};
    use crate::relation_service::{list_relations, ListRelationsFilter};
    use crate::store::memory::MemoryStore;
    use srs_core::types::container::{Container, ContainerEntry};

    fn container(id: &str, anchor: Option<&str>, entries: &[(&str, u32)]) -> Container {
        let mut c: Container = serde_json::from_value(serde_json::json!({
            "containerId": id, "title": "Doc"
        }))
        .unwrap();
        c.anchor_instance_id = anchor.map(String::from);
        c.member_instance_ids = Some(
            entries
                .iter()
                .map(|(i, d)| ContainerEntry::at(*i, *d))
                .collect(),
        );
        c
    }

    fn rec(store: &MemoryStore) -> String {
        create_lc_record(store).instance_id
    }

    fn derived_edges(store: &MemoryStore) -> Vec<(String, String)> {
        list_relations(store, ListRelationsFilter::default())
            .unwrap()
            .into_iter()
            .filter(|r| r.relation_type == FORK_RELATION_TYPE)
            .map(|r| (r.source_id, r.target_id))
            .collect()
    }

    fn count(store: &MemoryStore) -> usize {
        record_store::list_all_records(store)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    #[test]
    fn fork_swaps_only_in_target_container_and_preserves_depth() {
        let store = make_store_with_lifecycle();
        let (a, b, c) = (rec(&store), rec(&store), rec(&store));
        let c1 = container(
            "c1c1c1c1-0000-4000-8000-000000000001",
            None,
            &[(&a, 0), (&b, 1), (&c, 0)],
        );
        let c2 = container(
            "c2c2c2c2-0000-4000-8000-000000000002",
            None,
            &[(&a, 0), (&b, 1)],
        );
        container_service::create_container(&store, c1.clone()).unwrap();
        container_service::create_container(&store, c2.clone()).unwrap();

        let r = fork_subtree(&store, Some(&c1.container_id), &a, &ForkOptions::default()).unwrap();
        assert_eq!(r.forks.len(), 2); // a + its child b
        let m1 = container_service::get_arrangement(&store, &c1.container_id).unwrap();
        assert_eq!(m1[0].instance_id, r.forks[0].fork_id);
        assert_eq!(m1[1].instance_id, r.forks[1].fork_id);
        assert_eq!(m1[1].depth(), 1);
        assert_eq!(m1[2].instance_id, c);
        // other container still holds the originals
        let m2 = container_service::get_arrangement(&store, &c2.container_id).unwrap();
        assert_eq!(
            (m2[0].instance_id.as_str(), m2[1].instance_id.as_str()),
            (a.as_str(), b.as_str())
        );
        let edges = derived_edges(&store);
        assert!(edges.contains(&(r.forks[0].fork_id.clone(), a.clone())));
        assert!(edges.contains(&(r.forks[1].fork_id.clone(), b.clone())));
        // fork carries the same field values
        let orig = record_store::get_record_by_id(&store, &a).unwrap().unwrap();
        let fork = record_store::get_record_by_id(&store, &r.forks[0].fork_id)
            .unwrap()
            .unwrap();
        assert_eq!(orig.field_values, fork.field_values);
    }

    #[test]
    fn fork_refuses_anchor_and_non_member_without_writing() {
        let store = make_store_with_lifecycle();
        let (a, b, x) = (rec(&store), rec(&store), rec(&store));
        let c1 = container(
            "c1c1c1c1-0000-4000-8000-000000000001",
            Some(&a),
            &[(&a, 0), (&b, 0)],
        );
        container_service::create_container(&store, c1.clone()).unwrap();
        let before = count(&store);
        assert!(fork_subtree(&store, Some(&c1.container_id), &a, &ForkOptions::default()).is_err());
        assert!(fork_subtree(&store, Some(&c1.container_id), &x, &ForkOptions::default()).is_err());
        assert_eq!(count(&store), before);
    }

    #[test]
    fn fork_records_rolls_back_on_failure() {
        let store = make_store_with_lifecycle();
        let a = rec(&store);
        let before = count(&store);
        let err = fork_records(&store, &[a, "dddddddd-dddd-4ddd-8ddd-dddddddddddd".into()]);
        assert!(err.is_err());
        assert_eq!(count(&store), before);
        assert!(derived_edges(&store).is_empty());
    }

    #[test]
    fn copy_shares_members_and_forks_only_the_anchor() {
        let store = make_store_with_lifecycle();
        let (t, p1, p2) = (rec(&store), rec(&store), rec(&store));
        let mut src = container(
            "c1c1c1c1-0000-4000-8000-000000000001",
            Some(&t),
            &[(&t, 0), (&p1, 0), (&p2, 1)],
        );
        src.identity_instance_id = Some(t.clone());
        container_service::create_container(&store, src.clone()).unwrap();
        let before = count(&store);

        let r = container_service::copy_container(
            &store,
            &src.container_id,
            ContainerCopyInput::default(),
        )
        .unwrap();
        assert_eq!(count(&store), before + 1); // only the anchor fork
        assert_eq!(r.forks.len(), 1);
        let f = &r.forks[0].fork_id;
        let n = &r.container;
        assert_ne!(n.container_id, src.container_id);
        assert_eq!(n.title, "Doc (copy)");
        assert_eq!(n.anchor_instance_id.as_deref(), Some(f.as_str()));
        assert_eq!(n.identity_instance_id.as_deref(), Some(f.as_str()));
        let ids: Vec<_> = n
            .member_instance_ids
            .as_ref()
            .unwrap()
            .iter()
            .map(|e| (e.instance_id.clone(), e.depth()))
            .collect();
        assert_eq!(ids, vec![(f.clone(), 0), (p1.clone(), 0), (p2.clone(), 1)]);
        // source untouched; both containers hold the shared paragraphs
        let s = container_service::get_container(&store, &src.container_id).unwrap();
        assert_eq!(s.anchor_instance_id.as_deref(), Some(t.as_str()));
        assert!(derived_edges(&store).contains(&(f.clone(), t.clone())));
    }

    #[test]
    fn copy_without_anchor_creates_no_record_and_refuses_existing_id() {
        let store = make_store_with_lifecycle();
        let a = rec(&store);
        let src = container("c1c1c1c1-0000-4000-8000-000000000001", None, &[(&a, 0)]);
        container_service::create_container(&store, src.clone()).unwrap();
        let before = count(&store);
        let r = container_service::copy_container(
            &store,
            &src.container_id,
            ContainerCopyInput {
                title: Some("X".into()),
                container_id: None,
            },
        )
        .unwrap();
        assert_eq!(count(&store), before);
        assert!(r.forks.is_empty());
        let dup = container_service::copy_container(
            &store,
            &src.container_id,
            ContainerCopyInput {
                title: None,
                container_id: Some(src.container_id.clone()),
            },
        );
        assert!(dup.is_err());
    }

    // ── srs-rust#1354: --into and carried relations ──

    fn rels(store: &MemoryStore) -> Vec<(String, String, String)> {
        let mut v: Vec<_> = crate::relation_service::load_relations(store)
            .unwrap()
            .into_iter()
            .map(|r| (r.relation_type, r.source_instance_id, r.target_instance_id))
            .collect();
        v.sort();
        v
    }

    fn link(store: &MemoryStore, ty: &str, s: &str, t: &str) {
        let r: Relation = serde_json::from_value(serde_json::json!({
            "relationType": ty, "sourceInstanceId": s, "targetInstanceId": t,
            "notes": "keep me"
        }))
        .unwrap();
        crate::relation_service::create_relation_auto(store, r).unwrap();
    }

    fn setup() -> (MemoryStore, String, String, String, String) {
        let store = make_store_with_lifecycle();
        let (a, x, y) = (rec(&store), rec(&store), rec(&store));
        let cid = "c1c1c1c1-0000-4000-8000-000000000001".to_string();
        container_service::create_container(&store, container(&cid, None, &[(&x, 0)])).unwrap();
        link(&store, "refines", &a, &x); // outgoing
        link(&store, "supersedes", &y, &a); // incoming
        (store, cid, a, x, y)
    }

    #[test]
    fn into_adds_non_member_then_forks_there() {
        let (store, cid, a, _x, _y) = setup();
        let opts = ForkOptions {
            target_container: Some(cid.clone()),
            ..Default::default()
        };
        // Without --into semantics a non-member is refused.
        assert!(fork_subtree(&store, Some(&cid), &a, &ForkOptions::default()).is_err());
        let r = fork_subtree(&store, None, &a, &opts).unwrap();
        let m = container_service::list_members(&store, &cid).unwrap();
        assert!(m.contains(&r.forks[0].fork_id) && !m.contains(&a));
        assert!(r.carried_relations.is_empty());
    }

    #[test]
    fn into_failure_leaves_container_unchanged() {
        let (store, cid, _a, _x, _y) = setup();
        let before = container_service::list_members(&store, &cid).unwrap();
        let opts = ForkOptions {
            target_container: Some(cid.clone()),
            ..Default::default()
        };
        assert!(fork_subtree(&store, None, "dddddddd-dddd-4ddd-8ddd-dddddddddddd", &opts).is_err());
        assert_eq!(
            container_service::list_members(&store, &cid).unwrap(),
            before
        );
    }

    #[test]
    fn carry_all_recreates_links_and_leaves_original_alone() {
        let (store, cid, a, x, y) = setup();
        let before = rels(&store);
        let r = fork_subtree(
            &store,
            None,
            &a,
            &ForkOptions {
                target_container: Some(cid),
                carry_relations: CarryRelations::All,
            },
        )
        .unwrap();
        let f = r.forks[0].fork_id.clone();
        let after = rels(&store);
        for e in &before {
            assert!(after.contains(e), "original relation lost: {e:?}");
        }
        assert!(after.contains(&("refines".into(), f.clone(), x)));
        assert!(after.contains(&("supersedes".into(), y, f.clone())));
        // derived-from is not duplicated: exactly the fork's own edge.
        let d: Vec<_> = after.iter().filter(|e| e.0 == FORK_RELATION_TYPE).collect();
        assert_eq!(d.len(), 1);
        assert_eq!(r.carried_relations.len(), 2);
        assert_eq!(r.forks[0].carried_relation_ids.len(), 2);
        assert!(r
            .carried_relations
            .iter()
            .all(|c| c.notes.as_deref() == Some("keep me")));
    }

    #[test]
    fn carry_outgoing_skips_incoming() {
        let (store, cid, a, x, _y) = setup();
        let r = fork_subtree(
            &store,
            Some(&cid),
            &a,
            &ForkOptions {
                target_container: Some(cid.clone()),
                carry_relations: CarryRelations::Outgoing,
            },
        )
        .unwrap();
        assert_eq!(r.carried_relations.len(), 1);
        assert_eq!(r.carried_relations[0].target_instance_id, x);
    }

    #[test]
    fn default_carries_nothing() {
        let (store, cid, a, _x, _y) = setup();
        let n = rels(&store).len();
        let r = fork_subtree(
            &store,
            None,
            &a,
            &ForkOptions {
                target_container: Some(cid),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rels(&store).len(), n + 1); // only derived-from
        assert!(r.forks[0].carried_relation_ids.is_empty());
    }

    #[test]
    fn relation_between_two_forked_records_is_repointed_to_both_forks() {
        let store = make_store_with_lifecycle();
        let (p, c) = (rec(&store), rec(&store));
        let cid = "c1c1c1c1-0000-4000-8000-000000000001".to_string();
        container_service::create_container(&store, container(&cid, None, &[(&p, 0), (&c, 1)]))
            .unwrap();
        link(&store, "refines", &p, &c);
        let r = fork_subtree(
            &store,
            Some(&cid),
            &p,
            &ForkOptions {
                carry_relations: CarryRelations::All,
                ..Default::default()
            },
        )
        .unwrap();
        let (fp, fc) = (r.forks[0].fork_id.clone(), r.forks[1].fork_id.clone());
        let after = rels(&store);
        assert!(after.contains(&("refines".into(), fp, fc)));
        assert_eq!(r.carried_relations.len(), 1);
    }
}
