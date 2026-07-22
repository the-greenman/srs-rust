# ADR-040: `materialize_tree` preserves real paths; canonical filenames disambiguate id-prefix collisions

- **Status:** proposed
- **Date:** 2026-07-22
- **Supersedes:** —
- **Superseded by:** —
- **Amends:** ADR-038 (§4 `materialize_tree`); refines the slug-`id8` filename convention
  introduced in #140 (and consumed by the #392 `repo-upgrade` path normalizer) under ADR-008
  snapshot import.

## Context

ADR-038 made the in-memory VFS tree the primary operational model and states its goal plainly:
"the operational tree keeps real paths", and it names "the snapshot pipeline
(`export_repository_snapshot` → `import_repository_snapshot`) re-canonicalizes instance and
definition paths … so re-serialization rewrites untouched files" as the problem to eliminate.

ADR-038 §4 nonetheless defined `materialize_tree` (the bridge that turns a `.srsj` codec load into
the operational tree) as a "snapshot round-trip". That round-trip re-derives every instance path
through `canonical_instance_path`, which names a file `{tier-dir}/{slug}-{id[..8]}.json` — only the
**first 8 hex characters** of the instance UUID. Two consequences bit in production (#696):

1. **Collision crash.** Repositories that use deterministic instance UUIDs sharing a prefix (e.g.
   `…5801` and `…5802`) collapse onto one canonical path
   (`records/tier-2/decision-00000000.json`). `import_repository_snapshot` — reached by the browser
   `.srsj` load, `srs repo copy`, and `export_srsj` — rejected the whole repository with a
   "canonical path collision" error, even though the source paths (full-UUID) were unique.
2. **Migration masking.** Because `materialize_tree` canonicalized on load, a `.srsj` opened in the
   browser arrived already normalized, so the `repo-upgrade` ("Normalise instance file paths")
   migration could no longer be detected as `needed` — defeating the read-side migration ramp
   ADR-038 explicitly preserved for opening old-form repositories in order to update them.

Both are the same defect class: re-canonicalization on load, plus a canonical form that is not
collision-safe for deterministic UUIDs (which the data model explicitly permits — UUID5 /
deterministic ids).

## Decision

1. **`materialize_tree` reproduces the source's real file tree faithfully.** It enumerates the
   source with the same authoritative faithful store→tree walk that `archive_pack` uses
   (`archive::tree_entries`, ADR-039) and hands the result to `open_tree`. Instance files keep
   their real, source-declared paths; no re-canonicalization happens at load time. This makes
   §4 of ADR-038 consistent with ADR-038's own stated intent.

2. **Canonical instance filenames are collision-safe with a full-UUID fallback.** The default
   filename stays the short `{slug}-{id[..8]}.json` form. When — and only when — that short form
   is shared by two or more instances within one repository, all instances in that bucket use the
   full-UUID form `{slug}-{id}.json` (globally unique because instance UUIDs are unique). The
   decision is a function of the whole instance set, so it is independent of iteration order. It is
   applied at both snapshot-consuming loop sites: `import_repository_snapshot` and
   `collect_planned_renames` (the `repo-upgrade` planner). A remaining post-disambiguation
   collision can only mean a genuine duplicate instance id and stays a hard error.

## Consequences

**Positive:**

- `.srsj` repositories with deterministic/adjacent UUIDs load, copy, export, and upgrade without
  crashing.
- The `repo-upgrade` migration is detectable again after a `.srsj` load — the read-side migration
  ramp works as ADR-038 intended.
- One authoritative faithful store→tree enumeration (`tree_entries`) now serves both archive pack
  and tree materialization (DRY).

**Negative / trade-offs:**

- Instances whose short filename collides get longer, full-UUID filenames. This is rare (only
  deterministic-prefix repositories) and is the only collision-free deterministic option that keeps
  short names for everyone else.
- `materialize_tree` from a `.srsj` codec emits instance JSON in `tree_entries`' compact form
  (matching `archive_pack`); pretty-printing is deferred as a separate cosmetic concern.

**Neutral:**

- `copy_repository` still canonicalizes to the target convention (now collision-safe); a
  path-*preserving* copy mode, if ever wanted, is a separate future decision.
- No CLI payload, service signature, or entity-schema change; no spec RFC.
