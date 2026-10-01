//! RFC-043 container outline arrangement: the pure list algebra over
//! `Container.memberInstanceIds` entries (Change A, Change C, Change D).
//!
//! Everything here is I/O-free. The services (`container_service`) load, call these,
//! and persist; the renderer and navigation read effective depths from here, so there
//! is one implementation of the promoting removal ([R7]) and one of the validity
//! conditions ([R2]).

use crate::types::container::ContainerEntry;
use std::collections::HashSet;

pub const CODE_DEPTH: &str = "arrangement-depth";
pub const CODE_DUPLICATE: &str = "arrangement-duplicate";
pub const CODE_UNRESOLVED: &str = "arrangement-unresolved";
pub const CODE_IDENTITY: &str = "arrangement-identity";
pub const CODE_POINTER: &str = "arrangement-pointer";

/// One [R2]/[R7] finding — `code` is one of the `CODE_*` constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub code: &'static str,
    pub instance_id: String,
    pub message: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} ({})", self.code, self.message, self.instance_id)
    }
}

fn v(code: &'static str, id: &str, message: impl Into<String>) -> Violation {
    Violation {
        code,
        instance_id: id.to_string(),
        message: message.into(),
    }
}

/// [R2] structural conditions that need no catalog: first entry depth 0, depth rises by
/// at most one, ids exactly once, and (root container) identity at depth 0 with no
/// descendants. One violation per offending entry.
pub fn check_entries(entries: &[ContainerEntry], root_identity: Option<&str>) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut prev: Option<u32> = None;
    for e in entries {
        let d = e.depth();
        match prev {
            None if d != 0 => out.push(v(
                CODE_DEPTH,
                &e.instance_id,
                format!("the first entry must have depth 0 (found {d})"),
            )),
            Some(p) if d > p + 1 => out.push(v(
                CODE_DEPTH,
                &e.instance_id,
                format!("depth {d} exceeds the previous entry's depth {p} plus one"),
            )),
            _ => {}
        }
        prev = Some(d);
        if !seen.insert(e.instance_id.as_str()) {
            out.push(v(
                CODE_DUPLICATE,
                &e.instance_id,
                "instanceId appears more than once",
            ));
        }
    }
    if let Some(identity) = root_identity {
        if let Some(i) = entries.iter().position(|e| e.instance_id == identity) {
            if entries[i].depth() != 0 {
                out.push(v(
                    CODE_IDENTITY,
                    identity,
                    "the identity entry of the root container must be at depth 0",
                ));
            }
            if entries
                .get(i + 1)
                .is_some_and(|n| n.depth() > entries[i].depth())
            {
                out.push(v(
                    CODE_IDENTITY,
                    identity,
                    "the identity entry of the root container must have no descendants",
                ));
            }
        }
    }
    out
}

/// Length of the run at `idx`: the entry and the contiguous deeper entries after it.
pub fn run_len(entries: &[ContainerEntry], idx: usize) -> usize {
    let d = entries[idx].depth();
    1 + entries[idx + 1..]
        .iter()
        .take_while(|e| e.depth() > d)
        .count()
}

fn with_depth(mut e: ContainerEntry, d: u32) -> ContainerEntry {
    e.depth = (d > 0).then_some(d);
    e
}

/// The promoting removal ([R7]): delete the entry, reduce every entry of its run by one.
/// Returns the new list and the ids that were promoted. `None` when `id` is not an entry.
pub fn remove_promoting(
    entries: &[ContainerEntry],
    id: &str,
) -> Option<(Vec<ContainerEntry>, Vec<String>)> {
    let i = entries.iter().position(|e| e.instance_id == id)?;
    let run = run_len(entries, i);
    let mut out = entries[..i].to_vec();
    let mut promoted = Vec::new();
    for e in &entries[i + 1..i + run] {
        promoted.push(e.instance_id.clone());
        out.push(with_depth(e.clone(), e.depth() - 1));
    }
    out.extend_from_slice(&entries[i + run..]);
    Some((out, promoted))
}

/// Remove every entry whose id fails `keep`, each by the promoting removal. Used by
/// repair, slices and `typeFilter` (render-time virtual removal). Returns the new list
/// and the ids that were dropped.
pub fn retain_promoting(
    entries: &[ContainerEntry],
    keep: impl Fn(&str) -> bool,
) -> (Vec<ContainerEntry>, Vec<String>) {
    let mut cur = entries.to_vec();
    let mut dropped = Vec::new();
    for e in entries {
        if !keep(&e.instance_id) {
            if let Some((next, _)) = remove_promoting(&cur, &e.instance_id) {
                cur = next;
                dropped.push(e.instance_id.clone());
            }
        }
    }
    (cur, dropped)
}

fn validated(
    out: Vec<ContainerEntry>,
    identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    match check_entries(&out, identity).into_iter().next() {
        Some(violation) => Err(violation),
        None => Ok(out),
    }
}

/// **insert** (Change D): place `entry` at `position` (clamped to the end). No other
/// depth changes; rejected when the result violates [R2].
pub fn insert(
    entries: &[ContainerEntry],
    entry: ContainerEntry,
    position: Option<usize>,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let mut out = entries.to_vec();
    let at = position.unwrap_or(out.len()).min(out.len());
    out.insert(at, entry);
    validated(out, root_identity)
}

/// **move** (Change D): move the run of `id` to `position` (against the list without the
/// run, clamped) so that `id` takes `depth` (default: its current depth); the run's
/// depths shift together. Rejected when the result violates [R2].
pub fn move_run(
    entries: &[ContainerEntry],
    id: &str,
    position: Option<usize>,
    depth: Option<u32>,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let i = entries
        .iter()
        .position(|e| e.instance_id == id)
        .ok_or_else(|| v(CODE_UNRESOLVED, id, "not an entry of this container"))?;
    let run = run_len(entries, i);
    let base = entries[i].depth();
    let target = depth.unwrap_or(base);
    let mut out = entries[..i].to_vec();
    out.extend_from_slice(&entries[i + run..]);
    let at = position.unwrap_or(i).min(out.len());
    let moved: Vec<ContainerEntry> = entries[i..i + run]
        .iter()
        .map(|e| with_depth(e.clone(), e.depth() - base + target))
        .collect();
    out.splice(at..at, moved);
    validated(out, root_identity)
}

/// **set depth** (Change D): shift the run of `id` so the entry takes `depth`.
pub fn set_depth(
    entries: &[ContainerEntry],
    id: &str,
    depth: u32,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let i = entries
        .iter()
        .position(|e| e.instance_id == id)
        .ok_or_else(|| v(CODE_UNRESOLVED, id, "not an entry of this container"))?;
    move_run(entries, id, Some(i), Some(depth), root_identity)
}

/// Change C step 3: reverse every sibling list at every level (children stay under their
/// parent) and flatten in pre-order. Effective depths do not change.
pub fn reverse_siblings(entries: &[ContainerEntry]) -> Vec<ContainerEntry> {
    fn rec(entries: &[ContainerEntry], out: &mut Vec<ContainerEntry>) {
        // Split into sibling runs at the shallowest depth present.
        let Some(base) = entries.first().map(|e| e.depth()) else {
            return;
        };
        let mut runs: Vec<&[ContainerEntry]> = Vec::new();
        let mut start = 0;
        for i in 1..=entries.len() {
            if i == entries.len() || entries[i].depth() <= base {
                runs.push(&entries[start..i]);
                start = i;
            }
        }
        for run in runs.into_iter().rev() {
            out.push(run[0].clone());
            rec(&run[1..], out);
        }
    }
    let mut out = Vec::with_capacity(entries.len());
    rec(entries, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(spec: &[(&str, u32)]) -> Vec<ContainerEntry> {
        spec.iter()
            .map(|(i, d)| ContainerEntry::at(*i, *d))
            .collect()
    }
    fn shape(e: &[ContainerEntry]) -> Vec<(String, u32)> {
        e.iter()
            .map(|e| (e.instance_id.clone(), e.depth()))
            .collect()
    }
    fn s(spec: &[(&str, u32)]) -> Vec<(String, u32)> {
        spec.iter().map(|(i, d)| (i.to_string(), *d)).collect()
    }

    const PQRST: &[(&str, u32)] = &[("P", 0), ("Q", 1), ("R", 2), ("S", 1), ("T", 0)];

    #[test]
    fn r2_first_entry_and_rise() {
        assert!(check_entries(&outline(PQRST), None).is_empty());
        assert_eq!(
            check_entries(&outline(&[("a", 1)]), None)[0].code,
            CODE_DEPTH
        );
        assert_eq!(
            check_entries(&outline(&[("a", 0), ("b", 2)]), None)[0].code,
            CODE_DEPTH
        );
        assert_eq!(
            check_entries(&outline(&[("a", 0), ("a", 0)]), None)[0].code,
            CODE_DUPLICATE
        );
        assert_eq!(
            check_entries(&outline(&[("i", 0), ("b", 1)]), Some("i"))[0].code,
            CODE_IDENTITY
        );
    }

    #[test]
    fn r7_remove_promotes_run() {
        let (out, promoted) = remove_promoting(&outline(PQRST), "Q").unwrap();
        assert_eq!(shape(&out), s(&[("P", 0), ("R", 1), ("S", 1), ("T", 0)]));
        assert_eq!(promoted, vec!["R"]);
        // removing the first entry promotes its children to the removed depth
        let (out, _) = remove_promoting(&outline(PQRST), "P").unwrap();
        assert_eq!(shape(&out), s(&[("Q", 0), ("R", 1), ("S", 0), ("T", 0)]));
    }

    #[test]
    fn typefilter_exclusion_matches_the_worked_example() {
        let (out, _) = retain_promoting(&outline(PQRST), |id| id != "Q");
        assert_eq!(shape(&out), s(&[("P", 0), ("R", 1), ("S", 1), ("T", 0)]));
    }

    #[test]
    fn move_carries_the_run_and_set_depth_validates() {
        let out = move_run(&outline(PQRST), "Q", Some(3), Some(0), None).unwrap();
        assert_eq!(
            shape(&out),
            s(&[("P", 0), ("S", 1), ("T", 0), ("Q", 0), ("R", 1)])
        );
        // outdent a depth-2 entry to 0 in front of a depth-2 entry is rejected
        let e = outline(&[("a", 0), ("b", 1), ("c", 2), ("d", 2)]);
        assert_eq!(set_depth(&e, "c", 0, None).unwrap_err().code, CODE_DEPTH);
        // outdent adopts following deeper entries
        let e = outline(&[("a", 0), ("b", 1), ("c", 1), ("d", 2)]);
        let out = set_depth(&e, "c", 0, None).unwrap();
        assert_eq!(shape(&out), s(&[("a", 0), ("b", 1), ("c", 0), ("d", 1)]));
    }

    #[test]
    fn insert_adopts_and_is_rejected_on_jump() {
        let e = outline(&[("a", 0), ("b", 1)]);
        let out = insert(&e, ContainerEntry::new("n"), Some(1), None).unwrap();
        assert_eq!(shape(&out), s(&[("a", 0), ("n", 0), ("b", 1)]));
        assert!(insert(&e, ContainerEntry::at("n", 3), None, None).is_err());
    }

    #[test]
    fn desc_reverses_siblings_at_every_level() {
        let out = reverse_siblings(&outline(PQRST));
        assert_eq!(
            shape(&out),
            s(&[("T", 0), ("P", 0), ("S", 1), ("Q", 1), ("R", 2)])
        );
    }
}
