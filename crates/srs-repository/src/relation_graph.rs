use crate::error::RepositoryError;
use crate::record_store::get_record_by_id;
use crate::store::RepositoryStore;
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use std::collections::{HashMap, HashSet};

/// Sort records by following the `precedes` relation chain among them.
///
/// Builds a linked-list ordering from `precedes` relations whose both endpoints
/// are in the candidate set. Records not connected by any precedes relation fall
/// back to `created_at` order. Handles cycles via a visited set.
///
/// Extracted from `render_service` — shared by render and tree services.
pub(crate) fn sort_by_precedes_chain(records: Vec<Record>, relations: &[Relation]) -> Vec<Record> {
    if records.len() <= 1 {
        return records;
    }

    let id_set: HashSet<&str> = records.iter().map(|r| r.instance_id.as_str()).collect();

    let mut next: HashMap<&str, &str> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = id_set.iter().map(|id| (*id, 0)).collect();

    for rel in relations {
        if rel.relation_type != "precedes" {
            continue;
        }
        let src = rel.source_instance_id.as_str();
        let tgt = rel.target_instance_id.as_str();
        if id_set.contains(src) && id_set.contains(tgt) {
            // NOTE: `next` is a 1:1 map — if a record has multiple outgoing `precedes`
            // edges the last one wins. The SRS spec defines precedes as a linked-list
            // chain (each node precedes exactly one successor), so fan-out is not a
            // valid configuration; this limitation matches the spec invariant.
            next.insert(src, tgt);
            *in_degree.entry(tgt).or_insert(0) += 1;
        }
    }

    let mut heads: Vec<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();
    heads.sort_by(|a, b| {
        let ta = records
            .iter()
            .find(|r| r.instance_id == *a)
            .and_then(|r| r.created_at.as_deref())
            .unwrap_or("");
        let tb = records
            .iter()
            .find(|r| r.instance_id == *b)
            .and_then(|r| r.created_at.as_deref())
            .unwrap_or("");
        ta.cmp(tb)
    });

    let record_map: HashMap<&str, &Record> = records
        .iter()
        .map(|r| (r.instance_id.as_str(), r))
        .collect();

    let mut result: Vec<Record> = Vec::with_capacity(records.len());
    let mut visited: HashSet<&str> = HashSet::new();

    for head in heads {
        let mut current = head;
        loop {
            if visited.contains(current) {
                break;
            }
            visited.insert(current);
            if let Some(&record) = record_map.get(current) {
                result.push(record.clone());
            }
            match next.get(current) {
                Some(&nxt) => current = nxt,
                None => break,
            }
        }
    }

    let mut remaining: Vec<&Record> = records
        .iter()
        .filter(|r| !visited.contains(r.instance_id.as_str()))
        .collect();
    remaining.sort_by(|a, b| {
        a.created_at
            .as_deref()
            .unwrap_or("")
            .cmp(b.created_at.as_deref().unwrap_or(""))
    });
    result.extend(remaining.into_iter().cloned());

    result
}

/// Return child records reached via `relation_type` edges from `source_id`,
/// ordered by precedes chain. Skips IDs that don't resolve to a Tier 2 record.
pub(crate) fn children_by_relation_type(
    source_id: &str,
    relation_type: &str,
    all_relations: &[Relation],
    store: &dyn RepositoryStore,
) -> Result<Vec<Record>, RepositoryError> {
    let target_ids: Vec<&str> = all_relations
        .iter()
        .filter(|r| r.relation_type == relation_type && r.source_instance_id == source_id)
        .map(|r| r.target_instance_id.as_str())
        .collect();

    let mut children = Vec::new();
    for id in target_ids {
        if let Some(record) = get_record_by_id(store, id)? {
            children.push(record);
        }
    }

    Ok(sort_by_precedes_chain(children, all_relations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::record::Record;
    use srs_core::types::relation::Relation;
    use std::collections::HashMap;

    fn make_record(id: &str, created_at: &str) -> Record {
        Record {
            instance_id: id.to_string(),
            type_id: "t-test".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "test".to_string(),
            field_values: vec![],
            group_values: None,
            lifecycle_state: None,
            created_at: Some(created_at.to_string()),
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    fn make_precedes(src: &str, tgt: &str) -> Relation {
        Relation {
            relation_id: format!("rel-{src}-precedes-{tgt}"),
            relation_type: "precedes".to_string(),
            source_instance_id: src.to_string(),
            target_instance_id: tgt.to_string(),
            asserted_by: None,
            confidence: None,
            created_at: None,
            created_by: None,
            status: None,
            valid_from: None,
            valid_until: None,
            notes: None,
            source_refs: None,
            meta: None,
            source_repository_id: None,
            target_repository_id: None,
        }
    }

    #[test]
    fn sort_by_precedes_chain_basic() {
        let a = make_record("a", "2026-01-01T00:00:00Z");
        let b = make_record("b", "2026-01-02T00:00:00Z");
        let c = make_record("c", "2026-01-03T00:00:00Z");
        let records = vec![c.clone(), a.clone(), b.clone()];
        let relations = vec![make_precedes("a", "b"), make_precedes("b", "c")];
        let sorted = sort_by_precedes_chain(records, &relations);
        assert_eq!(sorted[0].instance_id, "a");
        assert_eq!(sorted[1].instance_id, "b");
        assert_eq!(sorted[2].instance_id, "c");
    }

    #[test]
    fn sort_by_precedes_chain_cycle() {
        let a = make_record("a", "2026-01-01T00:00:00Z");
        let b = make_record("b", "2026-01-02T00:00:00Z");
        let records = vec![b.clone(), a.clone()];
        let relations = vec![make_precedes("a", "b"), make_precedes("b", "a")];
        let sorted = sort_by_precedes_chain(records, &relations);
        assert_eq!(sorted.len(), 2, "should not drop records on cycle");
    }

    #[test]
    fn sort_by_precedes_chain_no_relations_falls_back_to_created_at() {
        let later = make_record("b-later", "2026-06-01T10:00:00Z");
        let earlier = make_record("a-earlier", "2026-06-01T09:00:00Z");
        let authored = vec![later.clone(), earlier.clone()];
        let sorted = sort_by_precedes_chain(authored, &[]);
        assert_eq!(sorted[0].instance_id, "a-earlier");
        assert_eq!(sorted[1].instance_id, "b-later");
    }
}
