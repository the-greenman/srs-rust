use crate::error::RepositoryError;
use crate::package::Package;
use crate::record_store::{get_instance_by_id, LoadedInstance};
use crate::store::RepositoryStore;
use srs_core::types::record::Record;
use srs_core::types::relation::Relation;
use srs_core::types::view::{SectionOrdering, SectionSource, SortDirection};
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
/// byte-identical however the relations arrive (#532).
///
/// Successors freed by the node just emitted are preferred over the rest of the
/// ready set. That is not arbitrary: [N+12] reads "order the member instances by
/// the `precedes` relation chain among them (topological sort); instances **not
/// connected by any `precedes` relation** are ordered by `createdAt` ascending
/// as a tiebreak". The tiebreak is for the *unconnected* members, not a global
/// interleaving key — so a chain stays contiguous and an unchained record takes
/// its place among the other unchained ones, rather than being spliced into the
/// middle of a chain because its timestamp happens to fall there. Several
/// topological orders satisfy [N+12]; this is the one that reads it that way,
/// and it is also the order the pre-Kahn implementation produced, so no existing
/// document reorders.
///
/// Records left over after the traversal are in a cycle; they are appended in
/// the same tiebreak order rather than dropped.
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

    // RFC-013 step 4 defines a fork as "a member with two successors **or two
    // predecessors**", so a pure join — `a precedes c`, `b precedes c`, nobody
    // with out-degree > 1 — is reportable too. Diagnosing only fan-out would
    // leave that shape silent.
    let mut diagnostics = Vec::new();
    let mut forks: Vec<(&str, usize, &str)> = successors
        .iter()
        .filter(|(_, tgts)| tgts.len() > 1)
        .map(|(src, tgts)| (*src, tgts.len(), "successors"))
        .chain(
            in_degree
                .iter()
                .filter(|(_, d)| **d > 1)
                .map(|(id, d)| (*id, *d, "predecessors")),
        )
        .collect();
    forks.sort_by(|a, b| tiebreak(a.0, b.0).then_with(|| a.2.cmp(b.2)));
    for (id, count, direction) in forks {
        diagnostics.push(format!(
            "`precedes` fork at {id}: {count} {direction}. Ordering resolved by the \
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
            "`precedes` cycle: {} could not be placed by topological order — a \
             cycle among them, or downstream of one. Ordering falls back to the \
             (createdAt, instanceId) tiebreak (RFC-013 step 4).",
            ids.join(", ")
        ));
    }
    remaining.sort_by(|a, b| tiebreak(a.precedes_instance_id(), b.precedes_instance_id()));
    result.extend(remaining.into_iter().cloned());

    (result, diagnostics)
}

/// Return child instances reached via `relation_type` edges from `source_id`,
/// ordered by precedes chain. Tier-aware: a target that resolves to a Tier-0
/// Note loads as `LoadedInstance::Note` rather than through the Tier-2-only
/// record loader, which hard-errors on one (`missing field typeId`) instead of
/// returning it — the same trap `tree_service`'s `child_ids` already fixed for
/// the tree walk (srs-rust#1070). Skips IDs that don't resolve to any instance.
pub(crate) fn children_by_relation_type(
    source_id: &str,
    relation_type: &str,
    all_relations: &[Relation],
    store: &dyn RepositoryStore,
) -> Result<Vec<LoadedInstance>, RepositoryError> {
    let target_ids: Vec<&str> = all_relations
        .iter()
        .filter(|r| r.relation_type == relation_type && r.source_instance_id == source_id)
        .map(|r| r.target_instance_id.as_str())
        .collect();

    let mut children = Vec::new();
    for id in target_ids {
        if let Some(instance) = get_instance_by_id(store, id)? {
            children.push(instance);
        }
    }

    Ok(sort_by_precedes_chain(children, all_relations))
}

/// Derive the RFC-008 `typeFilter` (when declared and non-empty) and the
/// `FixedInstances` flag from a section's `source`, for an
/// [`apply_section_ordering`] caller whose contract is "this section's
/// rendered subset" (render/project). A caller whose contract is "the full
/// membership, reordered" (the container-view editor projection) passes
/// `(None, false)` to `apply_section_ordering` directly instead of calling
/// this helper — see that function's `type_filter` doc.
pub(crate) fn section_ordering_inputs(source: &SectionSource) -> (Option<&[String]>, bool) {
    let type_filter = match source {
        SectionSource::ContainerSubset {
            type_filter: Some(f),
            ..
        } if !f.is_empty() => Some(f.as_slice()),
        _ => None,
    };
    let is_fixed_instances = matches!(source, SectionSource::FixedInstances { .. });
    (type_filter, is_fixed_instances)
}

/// Apply a `DocumentSection`'s full ordering ladder: RFC-015 [N+29]
/// `ordering.memberOrder`, else authored `ordering.fieldId`+`direction`, else
/// the [N+12] precedes/createdAt fallback — plus the RFC-008 `typeFilter`
/// projection ([N+18]-[N+22], and RFC-015 [N+30] when `memberOrder` is also
/// present).
///
/// `type_filter` is the RFC-008 `typeFilter` to project onto the result
/// (`None` to skip filtering entirely — a caller whose contract is "the full
/// membership, reordered" rather than "this section's rendered subset", e.g.
/// the container-view editor projection, passes `None`). `is_fixed_instances`
/// suppresses the [N+12] fallback: a `FixedInstances` section's declared
/// `instance_ids` order is the author's intent and must not be overridden
/// when no explicit `ordering` is present.
///
/// One shared implementation for every ordering consumer (render, container
/// view) — see `docs/architecture/capability-layering.md`: if two callers
/// could disagree about section order, the logic was in the wrong place.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_section_ordering(
    mut records: Vec<LoadedInstance>,
    ordering: Option<&SectionOrdering>,
    type_filter: Option<&[String]>,
    is_fixed_instances: bool,
    package: &Package,
    relations: &[Relation],
    section_id: &str,
    diagnostics: &mut Vec<String>,
) -> Vec<LoadedInstance> {
    if let Some(ordering) = ordering {
        if let Some(member_order) = &ordering.member_order {
            return apply_member_order(
                records,
                member_order,
                ordering.direction.clone(),
                type_filter,
                package,
                relations,
                section_id,
                diagnostics,
            );
        }
        if let Some(field_id) = &ordering.field_id {
            // `SectionOrdering.field_id` is a Field UUID; the RFC-039 carrier
            // keys values by `Field.name` — bridge via the package.
            let field_name = package
                .resolve_field(field_id)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| field_id.clone());
            records.sort_by(|a, b| {
                let av = a.get_field_value_str(&field_name).unwrap_or("");
                let bv = b.get_field_value_str(&field_name).unwrap_or("");
                av.cmp(bv)
            });
            if matches!(ordering.direction, Some(SortDirection::Desc)) {
                records.reverse();
            }
            apply_type_filter(&mut records, type_filter, package);
            return records;
        }
    }

    // No explicit ordering: [N+12] fallback, unless the section's declared
    // instance order (FixedInstances) must be preserved as authored.
    if !is_fixed_instances {
        records = sort_by_precedes_chain(records, relations);
    }
    apply_type_filter(&mut records, type_filter, package);
    records
}

/// RFC-008 `typeFilter`: restrict container-subset members to matching
/// types. Tier-0 notes have no type and never match an explicit `typeFilter`.
fn apply_type_filter(
    records: &mut Vec<LoadedInstance>,
    type_filter: Option<&[String]>,
    package: &Package,
) {
    let Some(filter) = type_filter else {
        return;
    };
    records.retain(|inst| {
        let Some(r) = inst.as_record() else {
            return false;
        };
        if let Some(rt) = package.resolve_type(&r.type_id, r.type_version) {
            let key = format!("{}/{}", rt.namespace, rt.name);
            filter.iter().any(|f| f == &key)
        } else {
            false
        }
    });
}

/// RFC-015 [N+29]/[N+30]: `memberOrder` applied over the (optionally
/// `typeFilter`-narrowed) member set.
///
/// Step (1)/(2): a listed id present in the filtered set is emitted in
/// declared order; a listed id excluded only by `typeFilter` is skipped
/// silently ([N+30] — no diagnostic); a listed id that is not a container
/// member at all is a departed entry and is diagnosed ([N+29] step 2), never
/// treated as a validation failure. Step (3): surviving members not named in
/// `memberOrder` are appended in [N+12] order, computed over the filtered
/// set. Step (4): `direction: desc` reverses the whole combined sequence.
#[allow(clippy::too_many_arguments)]
fn apply_member_order(
    records: Vec<LoadedInstance>,
    member_order: &[String],
    direction: Option<SortDirection>,
    type_filter: Option<&[String]>,
    package: &Package,
    relations: &[Relation],
    section_id: &str,
    diagnostics: &mut Vec<String>,
) -> Vec<LoadedInstance> {
    let all_ids: HashSet<String> = records
        .iter()
        .map(|r| r.instance_id().to_string())
        .collect();

    let mut filtered = records;
    apply_type_filter(&mut filtered, type_filter, package);

    let mut by_id: HashMap<String, LoadedInstance> = filtered
        .into_iter()
        .map(|r| (r.instance_id().to_string(), r))
        .collect();

    let mut result = Vec::with_capacity(member_order.len());
    for id in member_order {
        if let Some(inst) = by_id.remove(id) {
            result.push(inst);
        } else if !all_ids.contains(id) {
            diagnostics.push(format!(
                "[section:{section_id}] memberOrder entry {id} is not a current \
                 container member; skipped (RFC-015 [N+29])"
            ));
        }
        // else: present in the container but excluded by `typeFilter` — RFC-015
        // [N+30] silent skip, no diagnostic.
    }

    // Step (3): survivors not named in `memberOrder`, in [N+12] order over the
    // filtered set.
    let remaining: Vec<LoadedInstance> = by_id.into_values().collect();
    result.extend(sort_by_precedes_chain(remaining, relations));

    // Step (4).
    if matches!(direction, Some(SortDirection::Desc)) {
        result.reverse();
    }
    result
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
            created_at: None,
            notes: None,
            source_refs: None,
            meta: None,
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

    /// RFC-013 step 4 counts "two predecessors" as a fork too. A pure join —
    /// nobody with out-degree > 1 — must not pass silently.
    #[test]
    fn sort_by_precedes_chain_join_emits_diagnostic_naming_the_node() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            make_record("a", ts),
            make_record("b", ts),
            make_record("c", ts),
        ];
        let relations = vec![make_precedes("a", "c"), make_precedes("b", "c")];
        let (sorted, diagnostics) = sort_by_precedes_chain_diagnosed(records, &relations);
        let ids: Vec<&str> = sorted.iter().map(|r| r.instance_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("c"), "{}", diagnostics[0]);
        assert!(
            diagnostics[0].contains("predecessors"),
            "{}",
            diagnostics[0]
        );
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

    // ── RFC-015 [N+29]/[N+30]: apply_section_ordering / apply_member_order ────

    use srs_core::types::record_type::RecordType;

    fn minimal_package() -> Package {
        Package {
            id: "pkg-test".to_string(),
            namespace: "com.test".to_string(),
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            fields: vec![],
            record_types: vec![],
            relation_type_definitions: vec![],
            views: vec![],
            compositions: vec![],
            themes: vec![],
            blueprints: vec![],
            protocols: vec![],
            root: std::path::PathBuf::from("/memory"),
            package_dependencies: vec![],
            vocabularies: vec![],
            lifecycles: vec![],
        }
    }

    fn minimal_record_type(id: &str, namespace: &str, name: &str) -> RecordType {
        RecordType {
            schema: None,
            ai_guidance: None,
            tags: None,
            id: id.to_string(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            fields: vec![],
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            identity_field_id: None,
            lifecycle: None,
            lifecycle_ref: None,
            validation_rules: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    fn loaded(id: &str, created_at: &str) -> LoadedInstance {
        LoadedInstance::Record(make_record(id, created_at))
    }

    fn typed_loaded(id: &str, created_at: &str, type_id: &str) -> LoadedInstance {
        let mut r = make_record(id, created_at);
        r.type_id = type_id.to_string();
        LoadedInstance::Record(r)
    }

    #[test]
    fn apply_member_order_basic_sequence() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![loaded("a", ts), loaded("b", ts), loaded("c", ts)];
        let ordering = SectionOrdering {
            field_id: None,
            direction: None,
            member_order: Some(vec!["c".into(), "a".into(), "b".into()]),
        };
        let package = minimal_package();
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            Some(&ordering),
            None,
            false,
            &package,
            &[],
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["c", "a", "b"]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// RFC-015 [N+29] step (3): survivors not named in `memberOrder` are
    /// appended in [N+12] order, not container/list order.
    #[test]
    fn apply_member_order_appends_unlisted_in_precedes_order() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            loaded("a", ts),
            loaded("b", ts),
            loaded("c", ts),
            loaded("d", ts),
        ];
        let relations = vec![make_precedes("d", "b")];
        let ordering = SectionOrdering {
            field_id: None,
            direction: None,
            member_order: Some(vec!["c".into()]),
        };
        let package = minimal_package();
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            Some(&ordering),
            None,
            false,
            &package,
            &relations,
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        // c first (listed); then the [N+12] order of {a, b, d}: a and d are
        // both ready (createdAt tie -> instanceId), a < d, then d frees b.
        assert_eq!(ids, vec!["c", "a", "d", "b"]);
    }

    /// RFC-015 [N+29] step (2): a `memberOrder` entry naming an id that is no
    /// longer a container member is diagnosed, never a validation failure.
    #[test]
    fn apply_member_order_departed_entry_diagnosed_not_failed() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![loaded("a", ts), loaded("b", ts)];
        let ordering = SectionOrdering {
            field_id: None,
            direction: None,
            member_order: Some(vec!["ghost".into(), "a".into()]),
        };
        let package = minimal_package();
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            Some(&ordering),
            None,
            false,
            &package,
            &[],
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("ghost"), "{}", diagnostics[0]);
    }

    /// RFC-015 [N+29] step (4): `direction: desc` reverses the whole combined
    /// sequence (listed + appended tail), not just the listed prefix.
    #[test]
    fn apply_member_order_desc_reverses_combined_sequence() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![loaded("a", ts), loaded("b", ts), loaded("c", ts)];
        let ordering = SectionOrdering {
            field_id: None,
            direction: Some(SortDirection::Desc),
            member_order: Some(vec!["a".into(), "b".into()]),
        };
        let package = minimal_package();
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            Some(&ordering),
            None,
            false,
            &package,
            &[],
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["c", "b", "a"]);
    }

    /// RFC-015 [N+30]: a `memberOrder` entry naming a real container member
    /// that `typeFilter` excludes is skipped silently — no diagnostic, unlike
    /// a genuinely departed member.
    #[test]
    fn apply_member_order_with_type_filter_silently_skips_excluded_members() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            typed_loaded("a", ts, "t-keep"),
            typed_loaded("b", ts, "t-drop"),
            typed_loaded("c", ts, "t-keep"),
        ];
        let ordering = SectionOrdering {
            field_id: None,
            direction: None,
            member_order: Some(vec!["b".into(), "c".into(), "a".into()]),
        };
        let mut package = minimal_package();
        package.record_types = vec![
            minimal_record_type("t-keep", "com.test", "keep"),
            minimal_record_type("t-drop", "com.test", "drop"),
        ];
        let type_filter = vec!["com.test/keep".to_string()];
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            Some(&ordering),
            Some(&type_filter),
            false,
            &package,
            &[],
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["c", "a"]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn apply_section_ordering_no_ordering_falls_back_to_precedes_chain() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![loaded("z", ts), loaded("m", ts)];
        let relations = vec![make_precedes("m", "z")];
        let package = minimal_package();
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            None,
            None,
            false,
            &package,
            &relations,
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["m", "z"]);
    }

    /// A `FixedInstances` section's declared order is the author's intent and
    /// must survive even though the [N+12] fallback would reorder it.
    #[test]
    fn apply_section_ordering_fixed_instances_preserves_declared_order_absent_ordering() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![loaded("z", ts), loaded("m", ts)];
        let relations = vec![make_precedes("m", "z")];
        let package = minimal_package();
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            None,
            None,
            true,
            &package,
            &relations,
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["z", "m"]);
    }

    /// The container-view editor projection passes `type_filter: None` even
    /// when its governing section declares one — `ContainerView.members` is
    /// documented as the full membership, reordered, never narrowed.
    #[test]
    fn apply_section_ordering_none_type_filter_keeps_all_members() {
        let ts = "2026-01-01T00:00:00Z";
        let records = vec![
            typed_loaded("a", ts, "t-keep"),
            typed_loaded("b", ts, "t-drop"),
        ];
        let ordering = SectionOrdering {
            field_id: None,
            direction: None,
            member_order: Some(vec!["b".into(), "a".into()]),
        };
        let mut package = minimal_package();
        package.record_types = vec![minimal_record_type("t-keep", "com.test", "keep")];
        let mut diagnostics = Vec::new();
        let result = apply_section_ordering(
            records,
            Some(&ordering),
            None,
            false,
            &package,
            &[],
            "s1",
            &mut diagnostics,
        );
        let ids: Vec<&str> = result.iter().map(|r| r.instance_id()).collect();
        assert_eq!(ids, vec!["b", "a"]);
    }
}
