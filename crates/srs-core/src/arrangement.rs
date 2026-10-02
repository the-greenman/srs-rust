//! RFC-043 container outline arrangement: the pure list algebra over
//! `Container.memberInstanceIds` entries (Change A, Change C, Change D).
//!
//! Everything here is I/O-free. The services (`container_service`) load, call these,
//! and persist; the renderer and navigation read effective depths from here, so there
//! is one implementation of the promoting removal ([R7]) and one of the validity
//! conditions ([R2]).

use crate::types::container::ContainerEntry;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const CODE_DEPTH: &str = "arrangement-depth";
pub const CODE_DUPLICATE: &str = "arrangement-duplicate";
pub const CODE_UNRESOLVED: &str = "arrangement-unresolved";
pub const CODE_IDENTITY: &str = "arrangement-identity";
pub const CODE_POINTER: &str = "arrangement-pointer";
pub const CODE_TARGET: &str = "arrangement-target";

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
    let i = index_of(entries, id)?;
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
    let i = index_of(entries, id)?;
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

/// Where `place` puts the moved run relative to its target: a sibling just before or just
/// after the target's run, or the target's last child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    Before,
    After,
    Into,
}

/// A one-step relative edit of the entry's own run: indent / outdent by one level, or swap
/// with the previous (`Up`) / next (`Down`) sibling run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shift {
    Indent,
    Outdent,
    Up,
    Down,
}

/// Every relative operation as one value, so the service, CLI, MCP and WASM adapters share
/// one parse and one dispatch ([`RelativeMove::apply`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelativeMove {
    Place {
        target: String,
        placement: Placement,
    },
    Shift(Shift),
}

impl RelativeMove {
    /// Adapter-side parse of the optional `relativeTo` + `placement` pair and the optional
    /// `shift`. `Ok(None)` when neither was given; an error when they are mixed or partial.
    pub fn parse(
        relative_to: Option<&str>,
        placement: Option<&str>,
        shift: Option<&str>,
    ) -> Result<Option<Self>, String> {
        let quote = |s: &str| serde_json::Value::String(s.to_string());
        match (relative_to, placement, shift) {
            (None, None, None) => Ok(None),
            (Some(t), Some(p), None) => Ok(Some(Self::Place {
                target: t.to_string(),
                placement: serde_json::from_value(quote(p))
                    .map_err(|_| format!("placement must be before, after or into (got {p:?})"))?,
            })),
            (None, None, Some(s)) => Ok(Some(Self::Shift(
                serde_json::from_value(quote(s)).map_err(|_| {
                    format!("shift must be indent, outdent, up or down (got {s:?})")
                })?,
            ))),
            _ => Err("give either relativeTo with placement, or shift, and nothing else".into()),
        }
    }

    /// Resolve against `entries` through `move_run`, so there is one validity path.
    pub fn apply(
        &self,
        entries: &[ContainerEntry],
        id: &str,
        root_identity: Option<&str>,
    ) -> Result<Vec<ContainerEntry>, Violation> {
        match self {
            Self::Place { target, placement } => {
                place(entries, id, target, *placement, root_identity)
            }
            Self::Shift(Shift::Indent) => indent(entries, id, root_identity),
            Self::Shift(Shift::Outdent) => outdent(entries, id, root_identity),
            Self::Shift(Shift::Up) => step(entries, id, false, root_identity),
            Self::Shift(Shift::Down) => step(entries, id, true, root_identity),
        }
    }
}

fn index_of(entries: &[ContainerEntry], id: &str) -> Result<usize, Violation> {
    entries
        .iter()
        .position(|e| e.instance_id == id)
        .ok_or_else(|| v(CODE_UNRESOLVED, id, "not an entry of this container"))
}

/// **place**: move the run of `id` before / after `target`'s run (as its sibling, at the
/// target's depth) or into it (as its last child). `target` must lie outside the moved run.
pub fn place(
    entries: &[ContainerEntry],
    id: &str,
    target: &str,
    placement: Placement,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let i = index_of(entries, id)?;
    let t = index_of(entries, target)?;
    let run = run_len(entries, i);
    if (i..i + run).contains(&t) {
        return Err(v(
            CODE_TARGET,
            target,
            "cannot place an entry relative to itself or its own descendant",
        ));
    }
    // Position is measured against the list without the moved run (`move_run` contract).
    let ti = t - if t > i { run } else { 0 };
    let mut rest = entries[..i].to_vec();
    rest.extend_from_slice(&entries[i + run..]);
    let d = rest[ti].depth();
    let (at, depth) = match placement {
        Placement::Before => (ti, d),
        Placement::After => (ti + run_len(&rest, ti), d),
        Placement::Into => (ti + run_len(&rest, ti), d + 1),
    };
    move_run(entries, id, Some(at), Some(depth), root_identity)
}

/// **indent**: depth + 1, carrying the run, clamped by [R2] to the previous entry's depth +
/// 1. A no-op when the clamp leaves the depth unchanged (first entry, or already a child).
pub fn indent(
    entries: &[ContainerEntry],
    id: &str,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let i = index_of(entries, id)?;
    let max = i.checked_sub(1).map_or(0, |p| entries[p].depth() + 1);
    set_depth(
        entries,
        id,
        (entries[i].depth() + 1).min(max),
        root_identity,
    )
}

/// **outdent**: depth - 1 (a no-op at depth 0), carrying the run; it adopts following
/// shallower-run entries per Change D.
pub fn outdent(
    entries: &[ContainerEntry],
    id: &str,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let d = entries[index_of(entries, id)?].depth();
    set_depth(entries, id, d.saturating_sub(1), root_identity)
}

/// **step**: swap the run of `id` with its previous (`down == false`) or next sibling run.
/// A no-op when there is no such sibling (first / last child).
pub fn step(
    entries: &[ContainerEntry],
    id: &str,
    down: bool,
    root_identity: Option<&str>,
) -> Result<Vec<ContainerEntry>, Violation> {
    let i = index_of(entries, id)?;
    let d = entries[i].depth();
    let sibling = if down {
        let next = i + run_len(entries, i);
        entries.get(next).filter(|e| e.depth() == d)
    } else {
        entries[..i]
            .iter()
            .rev()
            .take_while(|e| e.depth() >= d)
            .find(|e| e.depth() == d)
    };
    let Some(sibling) = sibling else {
        return Ok(entries.to_vec());
    };
    let placement = if down {
        Placement::After
    } else {
        Placement::Before
    };
    place(entries, id, &sibling.instance_id, placement, root_identity)
}

/// One entry of the derived outline read: parent, depth and run, so clients never re-derive
/// them. `run_end` is the exclusive index of the run's end in the same list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineEntry {
    pub instance_id: String,
    pub depth: u32,
    pub parent_instance_id: Option<String>,
    pub has_children: bool,
    pub run_size: usize,
    pub run_end: usize,
}

/// The derived outline ([R5]): an entry's parent is the nearest preceding shallower entry.
pub fn outline(entries: &[ContainerEntry]) -> Vec<OutlineEntry> {
    let mut stack: Vec<usize> = Vec::new();
    (0..entries.len())
        .map(|i| {
            let d = entries[i].depth();
            while stack.last().is_some_and(|&p| entries[p].depth() >= d) {
                stack.pop();
            }
            let parent = stack.last().map(|&p| entries[p].instance_id.clone());
            stack.push(i);
            let run = run_len(entries, i);
            OutlineEntry {
                instance_id: entries[i].instance_id.clone(),
                depth: d,
                parent_instance_id: parent,
                has_children: run > 1,
                run_size: run,
                run_end: i + run,
            }
        })
        .collect()
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

    #[test]
    fn place_before_after_into_carry_runs() {
        let e = outline(PQRST); // P Q(1) R(2) S(1) T
        let p = |id, t, pl| shape(&place(&e, id, t, pl, None).unwrap());
        assert_eq!(
            p("T", "Q", Placement::Before),
            s(&[("P", 0), ("T", 1), ("Q", 1), ("R", 2), ("S", 1)])
        );
        assert_eq!(
            p("Q", "T", Placement::After),
            s(&[("P", 0), ("S", 1), ("T", 0), ("Q", 0), ("R", 1)])
        );
        assert_eq!(
            p("T", "P", Placement::Into),
            s(&[("P", 0), ("Q", 1), ("R", 2), ("S", 1), ("T", 1)])
        );
        assert_eq!(
            p("S", "R", Placement::Into),
            s(&[("P", 0), ("Q", 1), ("R", 2), ("S", 3), ("T", 0)])
        );
        // illegal: self and descendant
        for t in ["Q", "R"] {
            assert_eq!(
                place(&e, "Q", t, Placement::Into, None).unwrap_err().code,
                CODE_TARGET
            );
        }
        assert_eq!(
            place(&e, "Q", "nope", Placement::Before, None)
                .unwrap_err()
                .code,
            CODE_UNRESOLVED
        );
    }

    #[test]
    fn indent_outdent_clamp() {
        let e = outline(PQRST);
        let i = |id| shape(&indent(&e, id, None).unwrap());
        assert_eq!(i("P"), shape(&e)); // first entry: clamped no-op
        assert_eq!(i("Q"), shape(&e)); // already a child of P: clamped
        assert_eq!(
            i("T"),
            s(&[("P", 0), ("Q", 1), ("R", 2), ("S", 1), ("T", 1)])
        );
        assert_eq!(
            i("S"),
            s(&[("P", 0), ("Q", 1), ("R", 2), ("S", 2), ("T", 0)])
        );
        let o = |id| shape(&outdent(&e, id, None).unwrap());
        assert_eq!(o("P"), shape(&e));
        assert_eq!(
            o("Q"),
            s(&[("P", 0), ("Q", 0), ("R", 1), ("S", 1), ("T", 0)])
        );
        assert_eq!(
            o("R"),
            s(&[("P", 0), ("Q", 1), ("R", 1), ("S", 1), ("T", 0)])
        );
    }

    #[test]
    fn step_swaps_sibling_runs() {
        let e = outline(PQRST);
        let st = |id, down| shape(&step(&e, id, down, None).unwrap());
        assert_eq!(
            st("P", true),
            s(&[("T", 0), ("P", 0), ("Q", 1), ("R", 2), ("S", 1)])
        );
        assert_eq!(
            st("T", false),
            s(&[("T", 0), ("P", 0), ("Q", 1), ("R", 2), ("S", 1)])
        );
        assert_eq!(
            st("Q", true),
            s(&[("P", 0), ("S", 1), ("Q", 1), ("R", 2), ("T", 0)])
        );
        assert_eq!(st("P", false), shape(&e)); // first: no-op
        assert_eq!(st("T", true), shape(&e)); // last: no-op
        assert_eq!(st("R", true), shape(&e)); // only child: no sibling
    }

    #[test]
    fn identity_rule_is_r2_not_a_pin() {
        // RFC-043 [R2]/[R12]: the root identity entry stays at depth 0 with no descendants;
        // its position is free (navigation excludes it). Relative ops add no rule of their own.
        let e = outline(&[("I", 0), ("a", 0), ("b", 0)]);
        let id = Some("I");
        let sh = |r: Result<Vec<ContainerEntry>, Violation>| shape(&r.unwrap());
        assert_eq!(
            sh(step(&e, "I", true, id)),
            s(&[("a", 0), ("I", 0), ("b", 0)])
        );
        assert_eq!(
            sh(step(&e, "a", false, id)),
            s(&[("a", 0), ("I", 0), ("b", 0)])
        );
        // nothing may become the identity's child, and it may not be nested
        assert_eq!(
            place(&e, "a", "I", Placement::Into, id).unwrap_err().code,
            CODE_IDENTITY
        );
        assert_eq!(indent(&e, "a", id).unwrap_err().code, CODE_IDENTITY);
        assert_eq!(indent(&e, "I", id), Ok(e.clone())); // first entry: clamped no-op
        assert_eq!(
            place(&e, "I", "a", Placement::Into, id).unwrap_err().code,
            CODE_IDENTITY
        );
    }

    #[test]
    fn outline_derives_parents_and_runs() {
        let o = super::outline(&outline_entries());
        assert_eq!(o[0].run_end, 4);
        assert_eq!(o[1].parent_instance_id.as_deref(), Some("P"));
        assert_eq!(o[2].parent_instance_id.as_deref(), Some("Q"));
        assert_eq!(o[3].parent_instance_id.as_deref(), Some("P"));
        assert_eq!(o[4].parent_instance_id, None);
        assert!(o[1].has_children && !o[3].has_children);
        assert_eq!(o[1].run_size, 2);
    }
    fn outline_entries() -> Vec<ContainerEntry> {
        outline_spec(PQRST)
    }
    fn outline_spec(spec: &[(&str, u32)]) -> Vec<ContainerEntry> {
        spec.iter()
            .map(|(i, d)| ContainerEntry::at(*i, *d))
            .collect()
    }

    #[test]
    fn relative_move_parse() {
        assert_eq!(RelativeMove::parse(None, None, None), Ok(None));
        assert!(RelativeMove::parse(Some("a"), Some("into"), None)
            .unwrap()
            .is_some());
        assert!(RelativeMove::parse(None, None, Some("up"))
            .unwrap()
            .is_some());
        assert!(RelativeMove::parse(Some("a"), None, None).is_err());
        assert!(RelativeMove::parse(Some("a"), Some("sideways"), None).is_err());
        assert!(RelativeMove::parse(Some("a"), Some("into"), Some("up")).is_err());
    }
}
