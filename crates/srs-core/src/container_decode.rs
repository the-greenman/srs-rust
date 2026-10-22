//! RFC-051 [R5]/[R6]: decode a container's stored entries (`parent` pointer + fractional
//! `key`) into the logical RFC-043 outline. Pure and I/O-free; keys are only compared
//! byte-wise here, never generated.

use crate::arrangement::{Violation, CODE_IDENTITY};
use crate::types::container::ContainerEntry;
use std::collections::HashMap;

pub const CODE_ORPHAN: &str = "arrangement-orphan";
pub const CODE_CYCLE: &str = "arrangement-cycle";

/// One stored entry (`entries/<instanceId>.json`), less its `containerId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEntry {
    pub instance_id: String,
    pub parent: Option<String>,
    pub key: String,
}

/// The decoded outline and its findings. `arrangement-orphan` and `arrangement-cycle` are
/// warnings ([R6]); `arrangement-identity` is RFC-043 [R2]'s standing root-identity error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub entries: Vec<ContainerEntry>,
    pub findings: Vec<Violation>,
}

fn finding(code: &'static str, id: &str, message: String) -> Violation {
    Violation {
        code,
        instance_id: id.to_string(),
        message,
    }
}

/// [R5]: orphans and loop breaks (lowest `instanceId` on the loop) become parentless and
/// follow the attached top level; siblings sort by `key`, then `instanceId`, byte-wise;
/// the walk is depth-first pre-order. `identity` is the root container's identity entry,
/// which must have no `parent` and be no entry's `parent` ([R6]). Instance ids are
/// unique by construction (one file per entry).
pub fn decode(stored: &[StoredEntry], identity: Option<&str>) -> Decoded {
    let index: HashMap<&str, usize> = stored
        .iter()
        .enumerate()
        .map(|(i, e)| (e.instance_id.as_str(), i))
        .collect();
    let mut findings = Vec::new();

    if let Some(id) = identity {
        if index.get(id).is_some_and(|&i| stored[i].parent.is_some()) {
            findings.push(finding(
                CODE_IDENTITY,
                id,
                "the identity entry of the root container must have no parent".into(),
            ));
        }
        if stored.iter().any(|e| e.parent.as_deref() == Some(id)) {
            findings.push(finding(
                CODE_IDENTITY,
                id,
                "the identity entry of the root container must be no entry's parent".into(),
            ));
        }
    }

    // Effective parent; `detached[i]` marks orphans and breaks.
    let mut parent: Vec<Option<usize>> = vec![None; stored.len()];
    let mut detached = vec![false; stored.len()];
    for (i, e) in stored.iter().enumerate() {
        if let Some(p) = &e.parent {
            match index.get(p.as_str()) {
                Some(&pi) => parent[i] = Some(pi),
                None => {
                    detached[i] = true;
                    findings.push(finding(
                        CODE_ORPHAN,
                        &e.instance_id,
                        format!("parent {p} names no entry of the container"),
                    ));
                }
            }
        }
    }

    // Loops: follow each unvisited chain; a chain that meets its own walk closes a loop.
    // 0 = unvisited, 1 = on the current walk, 2 = done.
    let mut state = vec![0u8; stored.len()];
    for start in 0..stored.len() {
        let mut walk = Vec::new();
        let mut cur = Some(start);
        while let Some(i) = cur {
            match state[i] {
                0 => {
                    state[i] = 1;
                    walk.push(i);
                    cur = parent[i];
                }
                1 => {
                    let at = walk.iter().position(|&w| w == i).unwrap();
                    let brk = *walk[at..]
                        .iter()
                        .min_by(|&&a, &&b| stored[a].instance_id.cmp(&stored[b].instance_id))
                        .unwrap();
                    parent[brk] = None;
                    detached[brk] = true;
                    findings.push(finding(
                        CODE_CYCLE,
                        &stored[brk].instance_id,
                        format!(
                            "parent pointers form a loop of {} entries; this entry, the lowest instanceId on it, is placed at the top level",
                            walk.len() - at
                        ),
                    ));
                    break;
                }
                _ => break,
            }
        }
        for w in walk {
            state[w] = 2;
        }
    }

    let order = |v: &mut Vec<usize>| {
        v.sort_by(|&a, &b| {
            (stored[a].key.as_bytes(), stored[a].instance_id.as_bytes())
                .cmp(&(stored[b].key.as_bytes(), stored[b].instance_id.as_bytes()))
        })
    };
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); stored.len()];
    let (mut top, mut tail) = (Vec::new(), Vec::new());
    for i in 0..stored.len() {
        match parent[i] {
            Some(p) => children[p].push(i),
            None if detached[i] => tail.push(i),
            None => top.push(i),
        }
    }
    children.iter_mut().for_each(order);
    order(&mut top);
    order(&mut tail);
    top.extend(tail);

    let mut entries = Vec::with_capacity(stored.len());
    let mut stack: Vec<(usize, u32)> = top.into_iter().rev().map(|i| (i, 0)).collect();
    while let Some((i, d)) = stack.pop() {
        entries.push(ContainerEntry::at(stored[i].instance_id.clone(), d));
        stack.extend(children[i].iter().rev().map(|&c| (c, d + 1)));
    }
    Decoded { entries, findings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrangement::{self, Placement};

    fn e(id: &str, parent: Option<&str>, key: &str) -> StoredEntry {
        StoredEntry {
            instance_id: id.into(),
            parent: parent.map(Into::into),
            key: key.into(),
        }
    }
    fn shape(d: &Decoded) -> Vec<(String, u32)> {
        d.entries
            .iter()
            .map(|e| (e.instance_id.clone(), e.depth()))
            .collect()
    }
    fn s(spec: &[(&str, u32)]) -> Vec<(String, u32)> {
        spec.iter().map(|(i, d)| (i.to_string(), *d)).collect()
    }
    fn codes(d: &Decoded) -> Vec<(&'static str, String)> {
        d.findings
            .iter()
            .map(|f| (f.code, f.instance_id.clone()))
            .collect()
    }

    #[test]
    fn orphan_and_its_descendants_follow_the_attached_top_level() {
        let d = decode(
            &[
                e("o", Some("gone"), "a0"),
                e("c", Some("o"), "a0"),
                e("t", None, "a5"),
                e("u", Some("t"), "a0"),
            ],
            None,
        );
        assert_eq!(shape(&d), s(&[("t", 0), ("u", 1), ("o", 0), ("c", 1)]));
        assert_eq!(codes(&d), vec![(CODE_ORPHAN, "o".into())]);
    }

    #[test]
    fn dangling_pointer_is_an_orphan_and_not_an_error() {
        let d = decode(&[e("x", Some("missing"), "a0")], None);
        assert_eq!(shape(&d), s(&[("x", 0)]));
        assert_eq!(codes(&d), vec![(CODE_ORPHAN, "x".into())]);
    }

    #[test]
    fn loop_with_a_tail_breaks_at_the_lowest_id() {
        // b -> c -> d -> b is the loop; t hangs off c. Break is b.
        let d = decode(
            &[
                e("r", None, "a0"),
                e("b", Some("d"), "a0"),
                e("c", Some("b"), "a0"),
                e("d", Some("c"), "a0"),
                e("t", Some("c"), "a1"),
            ],
            None,
        );
        assert_eq!(
            shape(&d),
            s(&[("r", 0), ("b", 0), ("c", 1), ("d", 2), ("t", 2)])
        );
        assert_eq!(codes(&d), vec![(CODE_CYCLE, "b".into())]);
    }

    #[test]
    fn loop_with_no_attached_entry_reattaches_only_the_break() {
        let d = decode(&[e("y", Some("x"), "a0"), e("x", Some("y"), "a0")], None);
        assert_eq!(shape(&d), s(&[("x", 0), ("y", 1)]));
        assert_eq!(codes(&d), vec![(CODE_CYCLE, "x".into())]);
        let d = decode(&[e("self", Some("self"), "a0")], None);
        assert_eq!(shape(&d), s(&[("self", 0)]));
        assert_eq!(codes(&d), vec![(CODE_CYCLE, "self".into())]);
    }

    #[test]
    fn detached_entries_sort_among_themselves_by_key() {
        let d = decode(
            &[
                e("o", Some("gone"), "a2"),
                e("x", Some("x"), "a1"),
                e("p", None, "a9"),
            ],
            None,
        );
        assert_eq!(shape(&d), s(&[("p", 0), ("x", 0), ("o", 0)]));
    }

    #[test]
    fn equal_keys_order_by_instance_id_bytewise() {
        let d = decode(
            &[
                e("b", None, "a0"),
                e("B", None, "a0"),
                e("a", None, "a0"),
                e("z", None, "Zz"),
            ],
            None,
        );
        assert_eq!(shape(&d), s(&[("z", 0), ("B", 0), ("a", 0), ("b", 0)]));
        assert!(d.findings.is_empty());
    }

    #[test]
    fn identity_must_have_no_parent_and_no_children() {
        let ok = decode(&[e("i", None, "a0"), e("s", None, "a1")], Some("i"));
        assert!(ok.findings.is_empty());
        let d = decode(&[e("s", None, "a0"), e("i", Some("s"), "a0")], Some("i"));
        assert_eq!(codes(&d), vec![(CODE_IDENTITY, "i".into())]);
        let d = decode(&[e("i", None, "a0"), e("c", Some("i"), "a0")], Some("i"));
        assert_eq!(codes(&d), vec![(CODE_IDENTITY, "i".into())]);
    }

    /// Store an outline as parent pointers (via `arrangement::outline`) with keys in list
    /// order, decode it, and get the same outline back.
    fn round_trip(entries: &[ContainerEntry]) {
        let stored: Vec<StoredEntry> = arrangement::outline(entries)
            .into_iter()
            .enumerate()
            .map(|(i, o)| StoredEntry {
                instance_id: o.instance_id,
                parent: o.parent_instance_id,
                key: format!("a{i:04}"),
            })
            .collect();
        let d = decode(&stored, None);
        assert_eq!(d.entries, entries);
        assert!(d.findings.is_empty());
    }

    #[test]
    fn round_trip_through_parents_and_keys() {
        let base: Vec<ContainerEntry> = [("P", 0), ("Q", 1), ("R", 2), ("S", 1), ("T", 0)]
            .iter()
            .map(|(i, d)| ContainerEntry::at(*i, *d))
            .collect();
        round_trip(&base);
        round_trip(&arrangement::reverse_siblings(&base));
        round_trip(&arrangement::move_run(&base, "Q", Some(3), Some(0), None).unwrap());
        round_trip(&arrangement::place(&base, "T", "Q", Placement::Before, None).unwrap());
        round_trip(&arrangement::remove_promoting(&base, "Q").unwrap().0);
        round_trip(&arrangement::set_depth(&base, "S", 2, None).unwrap());
        round_trip(&[]);
    }
}
