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

/// Order records by their `precedes` relations — Rule [N+12]'s topological sort.
///
/// `precedes` is a DAG, not a linked list: a node may have several successors.
/// RFC-013 step 4 requires a fork (or a cycle) to still yield **one
/// deterministic order plus a diagnostic** — a fork is not invalidity, and
/// resolving it by whichever edge the relations file happens to list last is
/// exactly the order-dependence RFC-038 [R14] forbids.
///
/// The traversal is Kahn's algorithm, so a node is never emitted before a
/// predecessor (which a chain-following walk gets wrong on a join). Among the
/// nodes that are ready, the canonical RFC-013 tiebreak decides — `createdAt`
/// ascending, then `instanceId` ascending, a total order, so the output is
/// byte-identical however the relations arrive (#532). Successors freed by the
/// node just emitted are preferred over the rest of the ready set, which keeps
/// a chain contiguous instead of interleaving unrelated records into the middle
/// of it. Records left over after the traversal are in a cycle; they are
/// appended in the same tiebreak order rather than dropped.
///
/// Extracted from `render_service` — shared by render and tree services.
pub(crate) fn sort_by_precedes_chain<T: PrecedesSortable>(
    records: Vec<T>,
    relations: &[Relation],
) -> Vec<T> {
    sort_by_precedes_chain_diagnosed(records, relations).0
}

/// [`sort_by_precedes_chain`] plus the RFC-013 step 4 diagnostics naming each
/// forking and each cyclic node. Callers with a diagnostics channel (repository
/// navigation) use this one; the rest take the order alone.
pub(crate) fn sort_by_precedes_chain_diagnosed<T: PrecedesSortable>(
    records: Vec<T>,
    relations: &[Relation],
) -> (Vec<T>, Vec<String>) {
    if records.len() <= 1 {
        return (records, Vec::new());
    }

    let id_set: HashSet<&str> = records.iter().map(|r| r.precedes_instance_id()).collect();
    let record_map: HashMap<&str, &T> = records
        .iter()
        .map(|r| (r.precedes_instance_id(), r))
        .collect();

    let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = id_set.iter().map(|id| (*id, 0)).collect();
    let mut seen_edges: HashSet<(&str, &str)> = HashSet::new();

    for rel in relations {
        if rel.relation_type != "precedes" {
            continue;
        }
        let src = rel.source_instance_id.as_str();
        let tgt = rel.target_instance_id.as_str();
        if !id_set.contains(src) || !id_set.contains(tgt) {
            continue;
        }
        // A duplicate edge is one claim asserted twice, not two constraints —
        // counting it twice would leave the target permanently unready.
        if !seen_edges.insert((src, tgt)) {
            continue;
        }
        successors.entry(src).or_default().push(tgt);
        *in_degree.entry(tgt).or_insert(0) += 1;
    }

    // The canonical RFC-013 tiebreak, over ids resolved through `record_map`.
    let tiebreak = |a: &str, b: &str| {
        let key = |id: &str| record_map.get(id).and_then(|r| r.precedes_created_at());
        key(a)
            .unwrap_or("")
            .cmp(key(b).unwrap_or(""))
            .then_with(|| a.cmp(b))
    };

    let mut diagnostics = Vec::new();
    let mut forks: Vec<(&str, usize)> = successors
        .iter()
        .filter(|(_, tgts)| tgts.len() > 1)
        .map(|(src, tgts)| (*src, tgts.len()))
        .collect();
    forks.sort_by(|a, b| tiebreak(a.0, b.0));
    for (id, count) in forks {
        diagnostics.push(format!(
            "`precedes` fork at {id}: {count} successors. Ordering resolved by the \
             (createdAt, instanceId) tiebreak; the order is deterministic but the \
             document intent is ambiguous (RFC-013 step 4)."
        ));
    }

    // Ready set seeded from the caller-supplied record order, never HashMap
    // iteration (which is randomized per process — the #532 nondeterminism).
    let mut ready: Vec<&str> = records
        .iter()
        .map(|r| r.precedes_instance_id())
        .filter(|id| in_degree.get(id) == Some(&0))
        .collect();
    let mut preferred: Vec<&str> = Vec::new();

    let mut result: Vec<T> = Vec::with_capacity(records.len());
    let mut emitted: HashSet<&str> = HashSet::new();

    loop {
        let pick = {
            let pool = if preferred.is_empty() {
                &ready
            } else {
                &preferred
            };
            match pool.iter().min_by(|a, b| tiebreak(a, b)) {
                Some(id) => *id,
                None => break,
            }
        };
        ready.retain(|id| *id != pick);
        preferred.retain(|id| *id != pick);
        emitted.insert(pick);
        if let Some(&record) = record_map.get(pick) {
            result.push(record.clone());
        }

        let mut freed = Vec::new();
        for tgt in successors.get(pick).into_iter().flatten() {
            let degree = in_degree.entry(tgt).or_insert(0);
            *degree = degree.saturating_sub(1);
            if *degree == 0 {
                ready.push(tgt);
                freed.push(*tgt);
            }
        }
        preferred = freed;
    }

    // Whatever Kahn could not reach is inside a `precedes` cycle. RFC-013 step 4
    // again: a deterministic order and a diagnostic, not a dropped record.
    let mut remaining: Vec<&T> = records
        .iter()
        .filter(|r| !emitted.contains(r.precedes_instance_id()))
        .collect();
    if !remaining.is_empty() {
        let mut ids: Vec<&str> = remaining.iter().map(|r| r.precedes_instance_id()).collect();
        ids.sort_by(|a, b| tiebreak(a, b));
        diagnostics.push(format!(
            "`precedes` cycle among {}. Ordering falls back to the (createdAt, \
             instanceId) tiebreak (RFC-013 step 4).",
            ids.join(", ")
        ));
    }
    remaining.sort_by(|a, b| tiebreak(a.precedes_instance_id(), b.precedes_instance_id()));
    result.extend(remaining.into_iter().cloned());

    (result, diagnostics)
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
    use srs_core::types::record::{FieldValues, Record};
    use srs_core::types::relation::Relation;

    fn make_record(id: &str, created_at: &str) -> Record {
        Record {
            field_meta: None,
            instance_id: id.to_string(),
            type_id: "t-test".to_string(),
            type_version: 1,
            type_namespace: "com.test".to_string(),
            type_name: "test".to_string(),
            field_values: FieldValues::new(),
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

    // ---- srs-rust#863: `precedes` is a DAG, not a linked list ----

    /// The B11 defect: with two outgoing `precedes` edges the old map kept only
    /// whichever the relations file listed last, so the order flipped with the
    /// file. Both branches must be ordered, identically, from any rotation.
    #[test]
    fn sort_by_precedes_chain_fork_is_relation_order_independent() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            make_record("root", ts),
            make_record("b", ts),
            make_record("a", ts),
        ];
        let base = vec![make_precedes("root", "b"), make_precedes("root", "a")];
        let expected = vec!["root", "a", "b"];
        for rotation in 0..base.len() {
            let mut relations = base.clone();
            relations.rotate_left(rotation);
            let sorted = sort_by_precedes_chain(records.clone(), &relations);
            let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
            assert_eq!(ids, expected, "rotation {rotation} must not change order");
        }
    }

    /// RFC-013 step 4: a fork is not invalidity — it gets an order *and* a
    /// diagnostic that names the forking node.
    #[test]
    fn sort_by_precedes_chain_fork_emits_diagnostic_naming_the_node() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            make_record("root", ts),
            make_record("a", ts),
            make_record("b", ts),
        ];
        let relations = vec![make_precedes("root", "a"), make_precedes("root", "b")];
        let (sorted, diagnostics) = sort_by_precedes_chain_diagnosed(records, &relations);
        assert_eq!(sorted.len(), 3);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("root"), "{}", diagnostics[0]);
        assert!(diagnostics[0].contains("fork"), "{}", diagnostics[0]);
    }

    /// A cycle terminates with a deterministic order and its own diagnostic.
    #[test]
    fn sort_by_precedes_chain_cycle_emits_diagnostic_and_keeps_records() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![make_record("b", ts), make_record("a", ts)];
        let relations = vec![make_precedes("a", "b"), make_precedes("b", "a")];
        let (sorted, diagnostics) = sort_by_precedes_chain_diagnosed(records, &relations);
        let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("cycle"), "{}", diagnostics[0]);
        assert!(diagnostics[0].contains("a"), "{}", diagnostics[0]);
    }

    /// A clean chain is not a fork: no diagnostic, and the chain stays
    /// contiguous even when an unchained record's timestamp falls inside it.
    #[test]
    fn sort_by_precedes_chain_keeps_chain_contiguous_without_diagnostics() {
        let records = vec![
            make_record("x", "2026-01-01T00:00:00Z"),
            make_record("y", "2026-05-01T00:00:00Z"),
            make_record("z", "2026-03-01T00:00:00Z"),
        ];
        let relations = vec![make_precedes("x", "y")];
        let (sorted, diagnostics) = sort_by_precedes_chain_diagnosed(records, &relations);
        let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["x", "y", "z"]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// A join (diamond) must never emit a node before one of its predecessors —
    /// the failure mode a chain-following walk with a visited set has.
    #[test]
    fn sort_by_precedes_chain_join_emits_after_all_predecessors() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            make_record("a", ts),
            make_record("b", ts),
            make_record("c", ts),
            make_record("d", ts),
        ];
        let relations = vec![
            make_precedes("a", "b"),
            make_precedes("a", "c"),
            make_precedes("b", "d"),
            make_precedes("c", "d"),
        ];
        let sorted = sort_by_precedes_chain(records, &relations);
        let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c", "d"]);
    }

    /// The same `precedes` claim written twice is one constraint, not two — a
    /// double count would leave the target permanently unready.
    #[test]
    fn sort_by_precedes_chain_tolerates_duplicate_edges() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![make_record("b", ts), make_record("a", ts)];
        let mut dup = make_precedes("a", "b");
        dup.relation_id = "rel-duplicate".to_string();
        let relations = vec![make_precedes("a", "b"), dup];
        let (sorted, diagnostics) = sort_by_precedes_chain_diagnosed(records, &relations);
        let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
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
