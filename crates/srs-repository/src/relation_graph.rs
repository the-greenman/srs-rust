use crate::error::RepositoryError;
use crate::record_store::get_record_by_id;
use crate::store::RepositoryStore;
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use std::collections::{HashMap, HashSet};

/// Anything that can participate in a `precedes`-chain sort: it has an instance
/// ID and an optional creation timestamp for the fallback ordering.
pub(crate) trait PrecedesSortable: Clone {
    fn precedes_instance_id(&self) -> &str;
    fn precedes_created_at(&self) -> Option<&str>;
}

impl PrecedesSortable for Record {
    fn precedes_instance_id(&self) -> &str {
        &self.instance_id
    }
    fn precedes_created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
}

impl PrecedesSortable for crate::record_store::LoadedInstance {
    fn precedes_instance_id(&self) -> &str {
        self.instance_id()
    }
    fn precedes_created_at(&self) -> Option<&str> {
        self.created_at()
    }
}

/// Sort records by following the `precedes` relation chain among them.
///
/// Builds a linked-list ordering from `precedes` relations whose both endpoints
/// are in the candidate set. Records not connected by any precedes relation fall
/// back to the canonical tiebreak order: `created_at` ascending, then
/// `instance_id` ascending. The tiebreak is a total order, so output is
/// byte-identical across runs even when timestamps collide or are absent
/// (#532 — previously chain heads were emitted in `HashMap` iteration order).
/// Handles cycles via a visited set.
///
/// Extracted from `render_service` — shared by render and tree services.
pub(crate) fn sort_by_precedes_chain<T: PrecedesSortable>(
    records: Vec<T>,
    relations: &[Relation],
) -> Vec<T> {
    if records.len() <= 1 {
        return records;
    }

    let id_set: HashSet<&str> = records.iter().map(|r| r.precedes_instance_id()).collect();

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

    let record_map: HashMap<&str, &T> = records
        .iter()
        .map(|r| (r.precedes_instance_id(), r))
        .collect();

    // Collect chain heads from the caller-supplied record order (NOT HashMap
    // iteration, which is randomized per process), then sort by the canonical
    // (created_at, instance_id) tiebreak. Without the instance_id component,
    // heads with equal or missing timestamps would keep whatever transient
    // order they arrived in — the #532 nondeterminism.
    let mut heads: Vec<&str> = records
        .iter()
        .map(|r| r.precedes_instance_id())
        .filter(|id| in_degree.get(id) == Some(&0))
        .collect();
    heads.sort_by(|a, b| {
        let ta = record_map
            .get(a)
            .and_then(|r| r.precedes_created_at())
            .unwrap_or("");
        let tb = record_map
            .get(b)
            .and_then(|r| r.precedes_created_at())
            .unwrap_or("");
        ta.cmp(tb).then_with(|| a.cmp(b))
    });

    let mut result: Vec<T> = Vec::with_capacity(records.len());
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

    let mut remaining: Vec<&T> = records
        .iter()
        .filter(|r| !visited.contains(r.precedes_instance_id()))
        .collect();
    // Same canonical (created_at, instance_id) tiebreak as chain heads.
    remaining.sort_by(|a, b| {
        a.precedes_created_at()
            .unwrap_or("")
            .cmp(b.precedes_created_at().unwrap_or(""))
            .then_with(|| a.precedes_instance_id().cmp(b.precedes_instance_id()))
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
            tags: None,
            created_at: Some(created_at.to_string()),
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
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

    /// #532: equal `created_at` timestamps must break ties by `instance_id`
    /// ascending — a total order, so the result is identical however the
    /// candidates arrive.
    #[test]
    fn sort_by_precedes_chain_created_at_ties_break_by_instance_id() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            make_record("c", ts),
            make_record("a", ts),
            make_record("b", ts),
        ];
        let sorted = sort_by_precedes_chain(records, &[]);
        let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    /// #532: records with no `created_at` at all still get a deterministic
    /// order (instance_id ascending), including chain heads.
    #[test]
    fn sort_by_precedes_chain_missing_created_at_orders_by_instance_id() {
        let make_no_ts = |id: &str| {
            let mut r = make_record(id, "");
            r.created_at = None;
            r
        };
        // Two singleton heads plus one two-element chain (m -> z); every
        // permutation of the input must produce the same output.
        let base = vec![
            make_no_ts("z"),
            make_no_ts("m"),
            make_no_ts("b"),
            make_no_ts("a"),
        ];
        let relations = vec![make_precedes("m", "z")];
        let expected = vec!["a", "b", "m", "z"];
        for rotation in 0..base.len() {
            let mut input = base.clone();
            input.rotate_left(rotation);
            let sorted = sort_by_precedes_chain(input, &relations);
            let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
            assert_eq!(ids, expected, "rotation {rotation} must not change order");
        }
    }
}
