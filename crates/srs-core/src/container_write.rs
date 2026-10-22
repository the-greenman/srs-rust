//! RFC-051 [R8]/[R9]/[R11]: the exact entries a member operation writes. Pure and
//! I/O-free. An RFC-043 Change D operation runs on the decoded outline unchanged
//! (`arrangement`); this module turns its result into the stored entries to write and
//! remove, never the head. Keys come only from [`key_between`].

use crate::arrangement::{self, Violation};
use crate::arrangement_key::{key_between, CODE_KEY};
use crate::container_decode::{decode, StoredEntry, CODE_CYCLE, CODE_ORPHAN};
use crate::types::container::ContainerEntry;
use std::collections::{HashMap, HashSet};

/// What one operation writes: `write` in outline order, `remove` in stored order.
/// Applying both to the old entries decodes to `outline`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteSet {
    pub outline: Vec<ContainerEntry>,
    pub write: Vec<StoredEntry>,
    pub remove: Vec<String>,
}

/// [R8]: run `op` (an `arrangement` operation) on the decoded outline and compute its
/// write set. Detached entries are first re-attached as [`repair`] keys them ([R11]),
/// then the operation is keyed against that state ([R9]). `moved` is the entry a move
/// operation moves (`None` for insert and removals); it is written only when the
/// operation changes the outline. Rejections are the operation's own.
pub fn write_set(
    stored: &[StoredEntry],
    identity: Option<&str>,
    moved: Option<&str>,
    op: impl FnOnce(&[ContainerEntry]) -> Result<Vec<ContainerEntry>, Violation>,
) -> Result<WriteSet, Violation> {
    let decoded = decode(stored, identity);
    let (attached, mut written) = rekey(stored, &detached(&decoded), &decoded.entries, None)?;
    let outline = op(&decoded.entries)?;
    let moved = moved.filter(|_| outline != decoded.entries);
    let (next, w) = rekey(&attached, &HashSet::new(), &outline, moved)?;
    written.extend(w);
    Ok(finish(stored, next, &written, outline))
}

/// [R11] step 1: remove the entries `keep` rejects (RFC-043 [R20], promoting their runs)
/// and re-attach every orphan and loop break at the end of the top level, in decoding
/// order. It never removes a detached entry that `keep` accepts.
pub fn repair(
    stored: &[StoredEntry],
    identity: Option<&str>,
    keep: impl Fn(&str) -> bool,
) -> Result<WriteSet, Violation> {
    let decoded = decode(stored, identity);
    let outline = arrangement::retain_promoting(&decoded.entries, keep).0;
    let (next, written) = rekey(stored, &detached(&decoded), &outline, None)?;
    Ok(finish(stored, next, &written, outline))
}

fn detached(decoded: &crate::container_decode::Decoded) -> HashSet<String> {
    decoded
        .findings
        .iter()
        .filter(|f| f.code == CODE_ORPHAN || f.code == CODE_CYCLE)
        .map(|f| f.instance_id.clone())
        .collect()
}

fn finish(
    stored: &[StoredEntry],
    next: Vec<StoredEntry>,
    written: &HashSet<String>,
    outline: Vec<ContainerEntry>,
) -> WriteSet {
    let present: HashSet<&str> = next.iter().map(|e| e.instance_id.as_str()).collect();
    let remove = stored
        .iter()
        .filter(|e| !present.contains(e.instance_id.as_str()))
        .map(|e| e.instance_id.clone())
        .collect();
    let write = next
        .into_iter()
        .filter(|e| written.contains(&e.instance_id))
        .collect();
    WriteSet {
        outline,
        write,
        remove,
    }
}

/// The stored state for `target` (in outline order) and the ids written to reach it from
/// `stored`. Written: new entries, `force`d ones, `moved`, entries whose `parent` changes,
/// and the tied siblings [R9] adds. Every other entry keeps its `parent` and `key`.
fn rekey(
    stored: &[StoredEntry],
    force: &HashSet<String>,
    target: &[ContainerEntry],
    moved: Option<&str>,
) -> Result<(Vec<StoredEntry>, HashSet<String>), Violation> {
    let old: HashMap<&str, &StoredEntry> =
        stored.iter().map(|e| (e.instance_id.as_str(), e)).collect();
    let mut next: Vec<StoredEntry> = arrangement::outline(target)
        .into_iter()
        .map(|o| StoredEntry {
            key: old
                .get(o.instance_id.as_str())
                .map_or_else(String::new, |e| e.key.clone()),
            instance_id: o.instance_id,
            parent: o.parent_instance_id,
        })
        .collect();
    let mut written: Vec<bool> = next
        .iter()
        .map(|e| {
            let id = e.instance_id.as_str();
            old.get(id).is_none_or(|o| o.parent != e.parent)
                || force.contains(id)
                || moved == Some(id)
        })
        .collect();

    // Sibling lists in outline order.
    let mut groups: HashMap<Option<&str>, Vec<usize>> = HashMap::new();
    for (i, e) in next.iter().enumerate() {
        groups.entry(e.parent.as_deref()).or_default().push(i);
    }
    let mut keys: Vec<Option<String>> = vec![None; next.len()];
    for sibs in groups.values() {
        key_group(&next, sibs, &mut written, &mut keys)?;
    }
    for (e, k) in next.iter_mut().zip(keys) {
        if let Some(k) = k {
            e.key = k;
        }
    }
    let ids = next
        .iter()
        .zip(&written)
        .filter(|(_, &w)| w)
        .map(|(e, _)| e.instance_id.clone())
        .collect();
    Ok((next, ids))
}

/// [R9] over one sibling list: each run of written siblings is keyed in turn between its
/// preceding sibling and the next sibling that keeps its key. Where both neighbours share
/// a key, the run takes that key when it sorts there by `instanceId`; otherwise every
/// tied sibling after the place joins the run.
fn key_group(
    next: &[StoredEntry],
    sibs: &[usize],
    written: &mut [bool],
    keys: &mut [Option<String>],
) -> Result<(), Violation> {
    let fail = |i: usize, e: crate::arrangement_key::KeyError| Violation {
        code: CODE_KEY,
        instance_id: next[i].instance_id.clone(),
        message: e.0,
    };
    let mut s = 0;
    while s < sibs.len() {
        if !written[sibs[s]] {
            s += 1;
            continue;
        }
        let left = s.checked_sub(1).map(|l| sibs[l]);
        let mut end = s;
        while end < sibs.len() && written[sibs[end]] {
            end += 1;
        }
        let lk = left.map(|l| next[l].key.as_str());
        if let (Some(l), Some(&r)) = (left, sibs.get(end)) {
            let tied = next[l].key.as_str();
            if tied == next[r].key {
                let run = std::iter::once(l)
                    .chain(sibs[s..end].iter().copied())
                    .chain([r]);
                let ids: Vec<&str> = run.map(|i| next[i].instance_id.as_str()).collect();
                if ids.windows(2).all(|w| w[0] < w[1]) {
                    for &i in &sibs[s..end] {
                        keys[i] = Some(tied.to_string());
                    }
                    s = end;
                    continue;
                }
                while end < sibs.len() && (written[sibs[end]] || next[sibs[end]].key == tied) {
                    written[sibs[end]] = true;
                    end += 1;
                }
            }
        }
        let rk = sibs.get(end).map(|&r| next[r].key.as_str());
        let mut prev = lk.map(str::to_string);
        for &i in &sibs[s..end] {
            let k = key_between(prev.as_deref(), rk).map_err(|e| fail(i, e))?;
            keys[i] = Some(k.clone());
            prev = Some(k);
        }
        s = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrangement::{Placement, RelativeMove, Shift};

    fn e(id: &str, parent: Option<&str>, key: &str) -> StoredEntry {
        StoredEntry {
            instance_id: id.into(),
            parent: parent.map(Into::into),
            key: key.into(),
        }
    }

    /// The old entries with the write set applied.
    fn apply(stored: &[StoredEntry], ws: &WriteSet) -> Vec<StoredEntry> {
        let mut out: Vec<StoredEntry> = stored
            .iter()
            .filter(|s| !ws.remove.contains(&s.instance_id))
            .filter(|s| ws.write.iter().all(|w| w.instance_id != s.instance_id))
            .cloned()
            .collect();
        out.extend(ws.write.iter().cloned());
        out
    }

    /// decode(old + write set) is the operation's outline, with nothing detached.
    fn check(stored: &[StoredEntry], ws: &WriteSet) {
        let d = decode(&apply(stored, ws), None);
        assert_eq!(d.entries, ws.outline, "write set {ws:?}");
        assert!(d.findings.is_empty(), "{:?}", d.findings);
        for w in &ws.write {
            crate::arrangement_key::validate_key(&w.key).unwrap();
        }
    }

    fn run(
        stored: &[StoredEntry],
        moved: Option<&str>,
        op: impl FnOnce(&[ContainerEntry]) -> Result<Vec<ContainerEntry>, Violation>,
    ) -> WriteSet {
        let ws = write_set(stored, None, moved, op).unwrap();
        check(stored, &ws);
        ws
    }

    fn w(ws: &WriteSet) -> Vec<(String, Option<String>, String)> {
        ws.write
            .iter()
            .map(|e| (e.instance_id.clone(), e.parent.clone(), e.key.clone()))
            .collect()
    }
    fn x(spec: &[(&str, Option<&str>, &str)]) -> Vec<(String, Option<String>, String)> {
        spec.iter()
            .map(|(i, p, k)| (i.to_string(), p.map(Into::into), k.to_string()))
            .collect()
    }

    /// P Q(1) R(2) S(1) T, keyed as successive appends per sibling list.
    fn pqrst() -> Vec<StoredEntry> {
        vec![
            e("P", None, "a0"),
            e("Q", Some("P"), "a0"),
            e("R", Some("Q"), "a0"),
            e("S", Some("P"), "a1"),
            e("T", None, "a1"),
        ]
    }

    #[test]
    fn append_writes_one_entry_with_the_next_append_key() {
        let mut stored = Vec::new();
        for (id, key) in [("a", "a0"), ("b", "a1"), ("c", "a2")] {
            let ws = run(&stored, None, |o| {
                arrangement::insert(o, ContainerEntry::new(id), None, None)
            });
            assert_eq!(w(&ws), x(&[(id, None, key)]));
            assert!(ws.remove.is_empty());
            stored = apply(&stored, &ws);
        }
    }

    #[test]
    fn insert_writes_the_entry_and_the_entries_it_adopts() {
        let s = pqrst();
        let ws = run(&s, None, |o| {
            arrangement::insert(o, ContainerEntry::at("n", 1), Some(4), None)
        });
        assert_eq!(w(&ws), x(&[("n", Some("P"), "a2")]));
        // n at depth 0 after P adopts Q and S.
        let ws = run(&s, None, |o| {
            arrangement::insert(o, ContainerEntry::new("n"), Some(1), None)
        });
        assert_eq!(
            w(&ws),
            x(&[
                ("n", None, "a0V"),
                ("Q", Some("n"), "a0"),
                ("S", Some("n"), "a1")
            ])
        );
    }

    #[test]
    fn remove_writes_the_promoted_children_and_never_the_rest() {
        let s = pqrst();
        let ws = run(&s, None, |o| {
            Ok(arrangement::remove_promoting(o, "Q").unwrap().0)
        });
        assert_eq!(w(&ws), x(&[("R", Some("P"), "a0")]));
        assert_eq!(ws.remove, vec!["Q"]);
        let ws = run(&s, None, |o| {
            Ok(arrangement::remove_promoting(o, "T").unwrap().0)
        });
        assert!(ws.write.is_empty());
        assert_eq!(ws.remove, vec!["T"]);
        let ws = run(&s, None, |o| {
            Ok(arrangement::retain_promoting(o, |id| id != "P" && id != "R").0)
        });
        assert_eq!(w(&ws), x(&[("Q", None, "a0"), ("S", None, "a0V")]));
        assert_eq!(ws.remove, vec!["P", "R"]);
    }

    #[test]
    fn moves_write_the_moved_entry_and_adopted_entries_only() {
        let s = pqrst();
        let rm =
            |m: RelativeMove, id: &'static str| w(&run(&s, Some(id), |o| m.apply(o, id, None)));
        assert_eq!(
            rm(RelativeMove::Shift(Shift::Down), "P"),
            x(&[("P", None, "a2")])
        );
        assert_eq!(
            rm(RelativeMove::Shift(Shift::Indent), "T"),
            x(&[("T", Some("P"), "a2")])
        );
        // outdent Q adopts its following sibling S.
        assert_eq!(
            rm(RelativeMove::Shift(Shift::Outdent), "Q"),
            x(&[("Q", None, "a0V"), ("S", Some("Q"), "a1")])
        );
        assert_eq!(
            rm(
                RelativeMove::Place {
                    target: "Q".into(),
                    placement: Placement::Before
                },
                "T"
            ),
            x(&[("T", Some("P"), "Zz")])
        );
        // A no-op writes nothing.
        assert!(rm(RelativeMove::Shift(Shift::Up), "P").is_empty());
        assert!(rm(RelativeMove::Shift(Shift::Indent), "Q").is_empty());
        let ws = run(&s, Some("Q"), |o| {
            arrangement::move_run(o, "Q", Some(3), Some(0), None)
        });
        assert_eq!(w(&ws), x(&[("Q", None, "a2")]));
        let ws = run(&s, Some("S"), |o| arrangement::set_depth(o, "S", 2, None));
        assert_eq!(w(&ws), x(&[("S", Some("Q"), "a1")]));
    }

    /// Concurrent appends left x and y both at `a1`.
    fn tied() -> Vec<StoredEntry> {
        vec![e("p", None, "a0"), e("x", None, "a1"), e("y", None, "a1")]
    }

    #[test]
    fn a_written_entry_takes_the_tied_key_when_its_id_sorts_there() {
        let ws = run(&tied(), None, |o| {
            arrangement::insert(o, ContainerEntry::new("xn"), Some(2), None)
        });
        assert_eq!(w(&ws), x(&[("xn", None, "a1")]));
    }

    #[test]
    fn otherwise_every_tied_sibling_after_the_place_is_rekeyed() {
        let ws = run(&tied(), None, |o| {
            arrangement::insert(o, ContainerEntry::new("n"), Some(2), None)
        });
        assert_eq!(w(&ws), x(&[("n", None, "a2"), ("y", None, "a3")]));
        // A longer tie run, then a sibling that keeps its key.
        let s = vec![
            e("p", None, "a0"),
            e("x", None, "a1"),
            e("y", None, "a1"),
            e("z", None, "a1"),
            e("q", None, "a5"),
        ];
        let ws = run(&s, None, |o| {
            arrangement::insert(o, ContainerEntry::new("n"), Some(2), None)
        });
        assert_eq!(
            w(&ws),
            x(&[("n", None, "a2"), ("y", None, "a3"), ("z", None, "a4")])
        );
        // Moving the first of the tie run to the end needs no tie handling.
        let ws = run(&s, Some("x"), |o| {
            arrangement::move_run(o, "x", Some(4), None, None)
        });
        assert_eq!(w(&ws), x(&[("x", None, "a6")]));
        // Before the tie run: the place is not among equal keys.
        let ws = run(&s, None, |o| {
            arrangement::insert(o, ContainerEntry::new("n"), Some(1), None)
        });
        assert_eq!(w(&ws), x(&[("n", None, "a0V")]));
    }

    /// p attached; o orphaned; b→d→c→b a loop (break b) with t hanging off c.
    fn detached_state() -> Vec<StoredEntry> {
        vec![
            e("p", None, "a0"),
            e("o", Some("gone"), "a0"),
            e("b", Some("d"), "a0"),
            e("c", Some("b"), "a0"),
            e("d", Some("c"), "a0"),
            e("t", Some("c"), "a1"),
        ]
    }

    #[test]
    fn detached_entries_are_reattached_before_the_operation() {
        let s = detached_state();
        let ws = run(&s, None, |o| {
            arrangement::insert(o, ContainerEntry::new("n"), None, None)
        });
        assert_eq!(
            w(&ws),
            x(&[("b", None, "a1"), ("o", None, "a2"), ("n", None, "a3")])
        );
        // An insert between the attached top level and the re-attached entries keys
        // against the re-attached state ([R8] "first").
        let ws = run(&s, None, |o| {
            arrangement::insert(o, ContainerEntry::new("n"), Some(1), None)
        });
        assert_eq!(
            w(&ws),
            x(&[("n", None, "a0V"), ("b", None, "a1"), ("o", None, "a2")])
        );
        // Removing a detached entry still re-attaches the others.
        let ws = run(&s, None, |o| {
            Ok(arrangement::remove_promoting(o, "o").unwrap().0)
        });
        assert_eq!(w(&ws), x(&[("b", None, "a1")]));
        assert_eq!(ws.remove, vec!["o"]);
        // A no-op still writes the re-attachment.
        let ws = run(&s, Some("p"), |o| arrangement::step(o, "p", false, None));
        assert_eq!(w(&ws), x(&[("b", None, "a1"), ("o", None, "a2")]));
    }

    #[test]
    fn repair_reattaches_at_the_end_of_the_top_level_and_removes_unresolved() {
        let s = detached_state();
        let ws = repair(&s, None, |_| true).unwrap();
        check(&s, &ws);
        assert_eq!(w(&ws), x(&[("b", None, "a1"), ("o", None, "a2")]));
        assert!(ws.remove.is_empty());
        // Removing an unresolved detached entry first: the next one takes its key.
        let ws = repair(&s, None, |id| id != "b").unwrap();
        check(&s, &ws);
        assert_eq!(w(&ws), x(&[("c", None, "a1"), ("o", None, "a2")]));
        assert_eq!(ws.remove, vec!["b"]);
        let clean = pqrst();
        let ws = repair(&clean, None, |_| true).unwrap();
        assert!(ws.write.is_empty() && ws.remove.is_empty());
    }

    #[test]
    fn rejections_are_the_operations_own() {
        let err = write_set(&pqrst(), None, Some("Q"), |o| {
            arrangement::place(o, "Q", "R", Placement::Into, None)
        })
        .unwrap_err();
        assert_eq!(err.code, arrangement::CODE_TARGET);
    }

    #[test]
    fn identity_entry_is_passed_through() {
        let s = vec![e("i", None, "a0"), e("a", None, "a1")];
        let err = write_set(&s, Some("i"), Some("a"), |o| {
            arrangement::indent(o, "a", Some("i"))
        })
        .unwrap_err();
        assert_eq!(err.code, arrangement::CODE_IDENTITY);
    }

    type Op = dyn Fn(&[ContainerEntry]) -> Result<Vec<ContainerEntry>, Violation>;

    /// Every operation on every base: decode(apply) equals the outline, the write set is
    /// deterministic, and every written entry is new, the moved entry, re-attached, tied
    /// (only on a base with ties), or has a new parent.
    #[test]
    fn every_operation_round_trips_with_a_minimal_write_set() {
        let bases = [pqrst(), tied(), detached_state()];
        let mut count = 0;
        for s in &bases {
            let outline = decode(s, None).entries;
            let ids: Vec<String> = outline.iter().map(|e| e.instance_id.clone()).collect();
            let has_ties = s.iter().any(|a| {
                s.iter().any(|b| {
                    a.instance_id != b.instance_id && a.parent == b.parent && a.key == b.key
                })
            });
            let mut ops: Vec<(Option<String>, Box<Op>)> = Vec::new();
            for pos in 0..=ids.len() {
                for depth in 0..4 {
                    ops.push((
                        None,
                        Box::new(move |o| {
                            arrangement::insert(
                                o,
                                ContainerEntry::at("new", depth),
                                Some(pos),
                                None,
                            )
                        }),
                    ));
                }
            }
            for id in &ids {
                let i = id.clone();
                ops.push((
                    None,
                    Box::new(move |o| Ok(arrangement::remove_promoting(o, &i).unwrap().0)),
                ));
                for pos in 0..ids.len() {
                    for depth in 0..4 {
                        let i = id.clone();
                        ops.push((
                            Some(id.clone()),
                            Box::new(move |o| {
                                arrangement::move_run(o, &i, Some(pos), Some(depth), None)
                            }),
                        ));
                    }
                }
                for shift in [Shift::Indent, Shift::Outdent, Shift::Up, Shift::Down] {
                    let i = id.clone();
                    ops.push((
                        Some(id.clone()),
                        Box::new(move |o| RelativeMove::Shift(shift).apply(o, &i, None)),
                    ));
                }
                for t in &ids {
                    for placement in [Placement::Before, Placement::After, Placement::Into] {
                        let (i, t) = (id.clone(), t.clone());
                        ops.push((
                            Some(id.clone()),
                            Box::new(move |o| arrangement::place(o, &i, &t, placement, None)),
                        ));
                    }
                }
            }
            let reattached: HashSet<String> = detached(&decode(s, None));
            for (moved, op) in &ops {
                let Ok(ws) = write_set(s, None, moved.as_deref(), op) else {
                    continue;
                };
                count += 1;
                check(s, &ws);
                assert_eq!(write_set(s, None, moved.as_deref(), op).unwrap(), ws);
                for e in &ws.write {
                    let old = s.iter().find(|o| o.instance_id == e.instance_id);
                    let justified = old.is_none_or(|o| o.parent != e.parent)
                        || moved.as_deref() == Some(e.instance_id.as_str())
                        || reattached.contains(&e.instance_id)
                        || has_ties;
                    assert!(justified, "unjustified write {e:?} in {ws:?}");
                }
            }
        }
        assert!(count > 300, "only {count} operations ran");
    }
}
