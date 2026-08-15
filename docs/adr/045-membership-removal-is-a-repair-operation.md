# ADR-045: Container membership removal is a repair operation

- **Status:** accepted
- **Date:** 2026-08-14
- **Accepted:** 2026-08-14, on the owner merge of PR #846 (srs-rust#841), which
  landed both this ADR and the `remove_member`/`remove_root` repair seam it
  describes.
- **Supersedes:** —
- **Superseded by:** —

## Context

RFC-038 [R13] makes a container `rootInstanceIds` / `memberInstanceIds` entry that resolves
to no instance a catalog **error**, and [R24] makes an error diagnostic fatal to the load as a
whole. Every ordinary command builds the catalog through `store.catalog()` →
`catalog::build_checked`, so a single dangling container reference does not merely make one
command fail — it makes *all* of them fail.

srs-rust#841 showed that this is reachable through the CLI. `container roots add` and
`container members add` accepted an instance id that resolved to nothing (an empty string, a
whitespace-only string, or a well-formed but non-existent UUID), reported `ok: true`, and
persisted it. From that moment the repository was unloadable, and — crucially — the
`container roots remove` that would have undone the write failed on exactly the same fatal
catalog build. There was no CLI path back out; the only recovery was hand-editing JSON, which
`srs-usage.md` explicitly forbids.

Guarding the write side (the id must resolve before it is persisted) closes the door going
forward, but it does nothing for a repository already in that state, nor for the remaining
ways a dangling reference can arrive — `container create` / `container update` take the
membership list wholesale, and an imported or hand-authored repository can carry one from
outside this implementation entirely.

There are already sanctioned [R24] exemptions. `repo validate` and
`container_service::validate_container_invariants` build through the unchecked
`catalog::build` because their entire purpose is to *report* an incoherent repository, which
they could not do if incoherence failed the call; `archive::pack` does the same so that
archiving stays a faithful copier — it must not refuse a repository `repo validate` would
merely report on. Repair is the missing half of the first of those workflows: `repo validate`
names the offending container and id, and something has to be able to act on the answer.

## Decision

**Container membership removal reads and writes through the unchecked catalog builder.**

`container_service::remove_member` and `remove_root` resolve and persist their container via
`RepositoryStore::catalog_unchecked` (`catalog::build`, all diagnostics carried in the result)
rather than `RepositoryStore::catalog` (`catalog::build_checked`, [R24] fatality applied).
Concretely:

- `RepositoryStore` gains `catalog_unchecked`, and the trait-default
  `load_container_unchecked` / `save_container_unchecked`, which differ from their checked
  twins *only* in which builder resolves the container's locator. Locator and path handling
  stay inside `store.rs`; no path string enters a service function (ADR-041 G1).
- `container_service` gains `load_container_for_repair` / `save_container_for_repair`,
  which use those typed store methods (ADR-041 G3/G4, ADR-042 — containers keep their typed
  logical-id currency; nothing reverts to path+`Value`). The repair loader reads the
  `manifest.container` embed **directly** rather than through `resolve_root_container`, which
  routes via the checked `store.load_container` and would re-raise the very error being
  repaired — the embed-only root is the common shape, and the one #841 actually reproduces on.

The exemption is granted to **removal only**. Removal can only ever reduce the set of
references a container makes, so it can never introduce a dangling reference; that is what
distinguishes it from every other mutation. Every membership *writer* keeps the checked path
and additionally requires its incoming ids to resolve — `add_member`/`add_root` (#841) and,
since srs-rust#845, `create_container`/`update_container` for their whole membership list.

One consequence of that completeness is worth stating: the repair seam is now the **only**
surface in the codebase that can express a container with a dangling reference. Tests whose
subject is an already-incoherent container therefore construct it through
`save_container_unchecked`, which is a legitimate use — it is the seam standing in for the
external damage the repair path exists to undo — and not a widening of the exemption.

## Consequences

**Positive:**
- A repository bricked by a dangling container reference is repairable through the CLI —
  `repo validate` (already exempt) names the offending container and id, and
  `container roots remove` / `container members remove` act on it. No hand-edited JSON.
- The exemption is narrow and reuses the shared bodies of the checked path, so the checked and
  unchecked variants cannot drift.
- Reviewing the exemption set means grepping for two things, not one: the `*_unchecked` store
  methods introduced here (used by the two `*_for_repair` service helpers and by
  `validate_container_invariants`), and any remaining direct `crate::catalog::build` call.
  `archive::pack` is the one such direct caller left, with its own documented rationale.
- Store-agnostic by construction: the unchecked builder is a free function over
  `&dyn RepositoryStore`, so disk and `.srsj`/tree sessions behave identically.

**Negative / trade-offs:**
- This is the first [R24] exemption granted to a **write**; the existing ones (`repo validate`,
  `validate_container_invariants`, `archive::pack`) all only read. Every exemption is a place
  where a caller sees a repository the rest of the system considers unloadable, and a writing
  one more so. The mitigation is scope: removal is the only mutation that qualifies, and the
  reasoning is written at both helpers.
- `remove_member` / `remove_root` no longer fail fast on an unrelated fatal diagnostic
  elsewhere in the repository. That is the point, but it does mean a caller can successfully
  remove a membership entry from a repository that is still broken for other reasons.

**Neutral:**
- Removal remains a no-op returning the unchanged list when the id is not present.
- The exemption changes no diagnostic severity: `SRS038-R13-DANGLING-REFERENCE` is still an
  error, and `repo validate` still reports it.

## Rejected alternatives

**Demote `SRS038-R13-DANGLING-REFERENCE` to a warning.** This would un-brick every command at
once, and is the obvious smaller change. Rejected on two grounds: [R13]/[R24] severity is a
spec disposition, not an implementation choice, so changing it needs an RFC in `srs`; and it
would make the incoherent state *permitted* rather than *repairable* — silently rendering and
exporting from a repository whose membership does not resolve is precisely what [R24] exists
to stop.

**Put the unchecked locator resolution in `container_service`.** Rejected: it would reintroduce
path strings and `load_instance_json` / `save_instance_json` into a service function for the one
entity family that has already graduated to typed logical-id persistence (ADR-041 G1/G3/G4,
ADR-042, which cites containers as the template). The unchecked seam belongs beside the checked
one in `store.rs`.

**Add a dedicated `repo repair` command.** Rejected: removal already *is* the repair operation
for this failure mode; a second command for the same goal is the parallel-mechanism drift the
codebase deliberately avoids.
