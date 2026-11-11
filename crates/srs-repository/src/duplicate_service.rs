//! # Record duplicate service (srs-rust#1505)
//!
//! Duplicate one record as an independent successor: a new record of the same type with
//! deep-copied field values and envelope extras, linked `derived-from` its source, with only
//! the relations the caller selects recreated on the copy. Unlike a fork
//! ([`crate::fork_service::fork_subtree`]) the source keeps its place everywhere: the copy is
//! never swapped in for it, and an optional target container only gains the copy as a member.
//!
//! One core, shared by the CLI, WASM and MCP surfaces (capability layering). The copy itself is
//! [`crate::fork_service::fork_records`], so actor stamping (RFC-046), relation validation and
//! the initial lifecycle state are the fork core's. Everything created is removed again on
//! failure (best effort, ADR-024).

use crate::container_service;
use crate::error::RepositoryError;
use crate::fork_service::{fork_records, CarryRelations, FORK_RELATION_TYPE};
use crate::record_store::attempt_rollback_delete;
use crate::relation_service;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::types::relation::Relation;
use std::collections::BTreeSet;

/// Input of [`duplicate_record`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
pub struct DuplicateRecordInput {
    /// Relations of the source to recreate on the copy, by relation id. Each must touch the
    /// source. `derived-from` is never carried.
    #[serde(default)]
    pub relation_ids: Vec<String>,
    /// Also recreate the source's relations by direction: `outgoing` (source is the source end)
    /// or `all` (source is either end). `none` (default) selects nothing by direction.
    #[serde(default)]
    pub carry_relations: CarryRelations,
    /// Limit the direction selection to these relation types; empty means every type.
    /// Relations named in `relationIds` are carried whatever their type.
    #[serde(default)]
    pub relation_types: Vec<String>,
    /// Add the copy to this container (appended to its outline). The source is untouched.
    #[serde(default)]
    pub target_container: Option<String>,
}

/// Result of [`duplicate_record`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateRecordResult {
    pub original_id: String,
    pub duplicate_id: String,
    /// The `derived-from` relation (copy -> source).
    pub relation: Relation,
    /// The relations recreated on the copy; omitted when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried_relations: Vec<Relation>,
    /// The container the copy was added to, when one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
}

/// Duplicate the record `source_id` (see the module docs).
pub fn duplicate_record(
    store: &dyn RepositoryStore,
    source_id: &str,
    input: &DuplicateRecordInput,
) -> Result<DuplicateRecordResult, RepositoryError> {
    if let Some(cid) = input.target_container.as_deref() {
        if container_service::is_root_container(store, cid) {
            return Err(RepositoryError::ContainerIsRepositoryRoot {
                container_id: cid.to_string(),
            });
        }
        // Fail before creating anything when the container does not exist.
        container_service::get_container(store, cid)?;
    }
    let all = relation_service::load_relations(store)?;
    // Resolve the selection up front so a bad relation id creates nothing.
    let mut selected: Vec<Relation> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for rid in &input.relation_ids {
        let r = all.iter().find(|r| &r.relation_id == rid).ok_or_else(|| {
            RepositoryError::InvalidInput {
                message: format!("relation {rid} does not exist"),
            }
        })?;
        if r.source_instance_id != source_id && r.target_instance_id != source_id {
            return Err(RepositoryError::InvalidInput {
                message: format!("relation {rid} does not touch {source_id}"),
            });
        }
        if r.relation_type != FORK_RELATION_TYPE && seen.insert(r.relation_id.clone()) {
            selected.push(r.clone());
        }
    }
    if input.carry_relations != CarryRelations::None {
        for r in &all {
            let touches = r.source_instance_id == source_id
                || (input.carry_relations == CarryRelations::All
                    && r.target_instance_id == source_id);
            let typed =
                input.relation_types.is_empty() || input.relation_types.contains(&r.relation_type);
            if touches
                && typed
                && r.relation_type != FORK_RELATION_TYPE
                && seen.insert(r.relation_id.clone())
            {
                selected.push(r.clone());
            }
        }
    }

    let (pairs, relations) = fork_records(store, &[source_id.to_string()])?;
    let dup = pairs[0].fork_id.clone();
    let mut carried: Vec<Relation> = Vec::new();
    let mut added_to: Option<String> = None;
    let done = (|| -> Result<(), RepositoryError> {
        let mut existing: BTreeSet<(String, String, String)> = all
            .iter()
            .map(|r| {
                (
                    r.relation_type.clone(),
                    r.source_instance_id.clone(),
                    r.target_instance_id.clone(),
                )
            })
            .collect();
        for r in &selected {
            let swap = |id: &str| {
                if id == source_id {
                    dup.clone()
                } else {
                    id.to_string()
                }
            };
            let mut c = r.clone();
            c.relation_id = String::new();
            c.created_at = None;
            c.created_by = None;
            c.source_instance_id = swap(&r.source_instance_id);
            c.target_instance_id = swap(&r.target_instance_id);
            let key = (
                c.relation_type.clone(),
                c.source_instance_id.clone(),
                c.target_instance_id.clone(),
            );
            if !existing.insert(key) {
                continue;
            }
            carried.push(relation_service::create_relation_auto(store, c)?.relation);
        }
        if let Some(cid) = input.target_container.as_deref() {
            container_service::add_member(store, cid, &dup, None, None)?;
            added_to = Some(cid.to_string());
        }
        Ok(())
    })();
    if let Err(e) = done {
        for r in &carried {
            let _ = relation_service::delete_relation(store, &r.relation_id);
        }
        attempt_rollback_delete(store, &dup);
        return Err(e);
    }
    Ok(DuplicateRecordResult {
        original_id: source_id.to_string(),
        duplicate_id: dup,
        relation: relations
            .into_iter()
            .next()
            .expect("one fork, one relation"),
        carried_relations: carried,
        container_id: added_to,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_store::{
        self,
        tests::{create_lc_record, make_store_with_lifecycle},
    };
    use crate::relation_service::{list_relations, ListRelationsFilter};

    fn rec(store: &crate::store::memory::MemoryStore) -> String {
        create_lc_record(store).instance_id
    }

    fn edges(store: &crate::store::memory::MemoryStore, ty: &str) -> Vec<(String, String)> {
        list_relations(store, ListRelationsFilter::default())
            .unwrap()
            .into_iter()
            .filter(|r| r.relation_type == ty)
            .map(|r| (r.source_id, r.target_id))
            .collect()
    }

    #[test]
    fn duplicate_without_relations_is_derived_from_and_independent() {
        let store = make_store_with_lifecycle();
        let a = rec(&store);
        let r = duplicate_record(&store, &a, &DuplicateRecordInput::default()).unwrap();
        assert_eq!(r.original_id, a);
        assert_ne!(r.duplicate_id, a);
        assert_eq!(
            edges(&store, FORK_RELATION_TYPE),
            vec![(r.duplicate_id.clone(), a.clone())]
        );
        assert!(r.carried_relations.is_empty() && r.container_id.is_none());
        let src = record_store::get_record_by_id(&store, &a).unwrap().unwrap();
        let dup = record_store::get_record_by_id(&store, &r.duplicate_id)
            .unwrap()
            .unwrap();
        assert_eq!(src.field_values, dup.field_values);
        assert_ne!(src.instance_id, dup.instance_id);
        // independent: updating the copy leaves the source's values alone
        let mut changed = dup.clone();
        changed
            .field_values
            .insert("changed", serde_json::json!(true));
        store.save_record(&changed).unwrap();
        let src2 = record_store::get_record_by_id(&store, &a).unwrap().unwrap();
        assert_eq!(src.field_values, src2.field_values);
    }

    #[test]
    fn duplicate_carries_only_selected_relations() {
        let store = make_store_with_lifecycle();
        let (a, b, c) = (rec(&store), rec(&store), rec(&store));
        let mk = |s: &str, t: &str| {
            let rel: Relation = serde_json::from_value(serde_json::json!({
                "relationType": "depends-on", "sourceInstanceId": s, "targetInstanceId": t
            }))
            .unwrap();
            relation_service::create_relation_auto(&store, rel)
                .unwrap()
                .relation
        };
        let out = mk(&a, &b);
        let _inc = mk(&c, &a);
        // by id
        let r = duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                relation_ids: vec![out.relation_id.clone()],
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.carried_relations.len(), 1);
        assert_eq!(r.carried_relations[0].source_instance_id, r.duplicate_id);
        assert_eq!(r.carried_relations[0].target_instance_id, b);
        // by direction: outgoing leaves the incoming edge, all takes it too
        let o = duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                carry_relations: CarryRelations::Outgoing,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(o.carried_relations.len(), 1);
        let all = duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                carry_relations: CarryRelations::All,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.carried_relations.len(), 2);
        // type filter that matches nothing carries nothing
        let none = duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                carry_relations: CarryRelations::All,
                relation_types: vec!["refines".into()],
                ..Default::default()
            },
        )
        .unwrap();
        assert!(none.carried_relations.is_empty());
        // the source's own relations are untouched
        assert!(edges(&store, "depends-on").contains(&(a.clone(), b.clone())));
        assert!(edges(&store, "depends-on").contains(&(c.clone(), a.clone())));
    }

    #[test]
    fn duplicate_into_container_appends_and_leaves_the_source_member() {
        let store = make_store_with_lifecycle();
        let a = rec(&store);
        let cid = "c1c1c1c1-0000-4000-8000-000000000001";
        let mut c: srs_core::types::container::Container =
            serde_json::from_value(serde_json::json!({"containerId": cid, "title": "Doc"}))
                .unwrap();
        c.member_instance_ids = Some(vec![srs_core::types::container::ContainerEntry::at(&a, 0)]);
        store.save_container(&c).unwrap();
        let r = duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                target_container: Some(cid.into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(r.container_id.as_deref(), Some(cid));
        let members = container_service::list_members(&store, cid).unwrap();
        assert_eq!(members, vec![a, r.duplicate_id]);
    }

    #[test]
    fn bad_input_creates_nothing() {
        let store = make_store_with_lifecycle();
        let (a, b, c) = (rec(&store), rec(&store), rec(&store));
        let before = record_store::list_all_records(&store).unwrap().len();
        let rel: Relation = serde_json::from_value(serde_json::json!({
            "relationType": "depends-on", "sourceInstanceId": b, "targetInstanceId": c
        }))
        .unwrap();
        let unrelated = relation_service::create_relation_auto(&store, rel)
            .unwrap()
            .relation;
        // a relation that does not touch the source is refused
        assert!(duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                relation_ids: vec![unrelated.relation_id],
                ..Default::default()
            }
        )
        .is_err());
        assert!(duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                relation_ids: vec!["nope".into()],
                ..Default::default()
            }
        )
        .is_err());
        assert!(duplicate_record(
            &store,
            &a,
            &DuplicateRecordInput {
                target_container: Some("missing".into()),
                ..Default::default()
            }
        )
        .is_err());
        assert!(
            duplicate_record(&store, "not-a-record", &DuplicateRecordInput::default()).is_err()
        );
        assert_eq!(
            record_store::list_all_records(&store).unwrap().len(),
            before
        );
    }
}
