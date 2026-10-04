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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkResult {
    pub container_id: String,
    pub forks: Vec<ForkPair>,
    pub relations: Vec<Relation>,
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

/// Fork the arrangement subtree rooted at `root_instance_id` of `container_id` and swap the
/// forks into THAT container only, in place (same order and depth). Other containers keep
/// the originals. The anchor/identity entries cannot be forked this way, and the repository
/// root container is refused. v1: a forked entry that anchors a child container leaves that
/// container pointing at the original. Forking only READS the originals (it creates new records),
/// so the session write guard checks the target container alone.
pub fn fork_subtree(
    store: &dyn RepositoryStore,
    container_id: &str,
    root_instance_id: &str,
) -> Result<ForkResult, RepositoryError> {
    if container_service::is_root_container(store, container_id) {
        return Err(RepositoryError::ContainerIsRepositoryRoot {
            container_id: container_id.to_string(),
        });
    }
    let container = container_service::get_container(store, container_id)?;
    let entries = container.member_instance_ids.clone().unwrap_or_default();
    let idx = entries
        .iter()
        .position(|e| e.instance_id == root_instance_id)
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: format!("{root_instance_id} is not a member of container {container_id}"),
        })?;
    let ids: Vec<String> = entries[idx..idx + arrangement::run_len(&entries, idx)]
        .iter()
        .map(|e| e.instance_id.clone())
        .collect();
    // Pre-validate the whole run before creating anything.
    for id in &ids {
        if container.anchor_instance_id.as_deref() == Some(id)
            || container.identity_instance_id.as_deref() == Some(id)
        {
            return Err(RepositoryError::InvalidInput {
                message: format!(
                    "{id} is the anchor/identity of {container_id}; it cannot be forked in place"
                ),
            });
        }
    }
    let (forks, relations) = fork_records(store, &ids)?;
    let swaps: Vec<(String, String)> = forks
        .iter()
        .map(|p| (p.original_id.clone(), p.fork_id.clone()))
        .collect();
    if let Err(e) = container_service::replace_members(store, container_id, &swaps) {
        for p in &forks {
            attempt_rollback_delete(store, &p.fork_id);
        }
        return Err(e);
    }
    Ok(ForkResult {
        container_id: container_id.to_string(),
        forks,
        relations,
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

        let r = fork_subtree(&store, &c1.container_id, &a).unwrap();
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
        assert!(fork_subtree(&store, &c1.container_id, &a).is_err());
        assert!(fork_subtree(&store, &c1.container_id, &x).is_err());
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
}
