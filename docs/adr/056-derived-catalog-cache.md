# ADR-056: A derived catalog cache on `DiskVfs`

- **Status:** accepted
- **Date:** 2026-10-30
- **Supersedes:** —
- **Superseded by:** —

## Context

Every CLI invocation builds the catalog cold: it re-reads, re-parses and re-validates every instance
file. Agents drive the CLI one process per call, so a session of N commands pays N full builds even
when nothing changed (~0.5 s per call on a x10 muSrs corpus; srs-rust#1198, problem SP-78). The
within-process memo (srs-rust#1108) and the parallel prepass (srs-rust#1197) do not help across
processes.

The settled choices (owner direction, 2026-10-30, on srs-rust#1198) are recorded here as the decision.
They follow from the issue's invariant, *the cache must never change a result*, and the layer rules.
Implementation-only and derived, so no spec door applies (Door 1).

## Decision

1. **Location.** `.srs/cache/catalog.json` on `DiskVfs` only, with an automatically written
   `.srs/cache/.gitignore` (`*`). `MemVfs` and WASM have no stable file identity: they never read or
   create a cache. The cache is written only inside an initialised repository (`.srs/` exists), by
   write-then-rename, and a write failure is swallowed.
2. **Compatibility with RFC-038 and ADR-041 G2.** The cache is derived, adapter-private and never
   authoritative. Membership stays tree-authoritative: the tree walk is always performed, and
   `.srs/` is a skipped segment of classification, so cache files are never candidates. A missing,
   corrupt or stale cache always falls back to the full build.
3. **What is cached.** The per-file instance classification (kind, tier, instance id), which is a
   pure function of the file's bytes and the embedded schemas. Only successful classifications are
   cached; malformed or rejected files are re-evaluated on every build, so every diagnostic is
   produced by the same code as a cold build. The set-level checks ([R24], [R16] token, duplicates,
   reference resolution) always re-run over the full result.
4. **Key.** Relative path, size and mtime. When an entry's mtime is within two seconds of the cache
   write time (the "racy" window git uses, covering coarse timestamp granularity and clock skew),
   the stamp is not trusted and the SHA-256 of the file text must match. Every entry stores that hash.
5. **Invalidation.** Wholesale, on any change to the cache format version, the running binary
   (package version plus the executable's size and mtime, since `srs --version` cannot tell builds
   apart) or the embedded schema set (a digest of every bundled schema, equivalent to the mirror's
   `SHA256SUMS`).
6. **Seam.** `Vfs` gains default methods `stamp`, `cache_read` and `cache_write`;
   `RepositoryStore` gains `candidate_stamp`, `catalog_cache_read` and `catalog_cache_write`. The
   defaults mean no cache, so every other store and test double is unchanged. The logic lives in
   `srs-repository` (`catalog_cache`), not in the CLI.

## Consequences

**Positive:** a no-change invocation costs a tree walk plus a stat per candidate, instead of a read,
parse and schema validation per candidate. Cold, warm and edited builds are proven equal by test.

**Negative / trade-offs:** an edit that keeps size and mtime identical and is older than the racy
window is not detected (the same limitation as git's index); an operator can always delete
`.srs/cache/`. Relations, containers and definitions are not yet cached; they are few next to
instances. A first build after any binary change is cold.

**Neutral:** no `dataModelRevision` change, no payload change, no spec change.

## Implementation charter (ADR-048)

- [x] **Spec-first** — non-normative derived artefact; RFC-038 tree authority untouched (Door 1).
- [x] **Layer test** — `srs-repository` service and `Vfs` seam; CLI unchanged.
- [x] **One way per goal** — one classification function (`classify_source`) serves cold and cached builds.
- [x] **Parity and mirror obligations** — none: no schema, payload or pin change.
- [x] **Decision mode** — complicated.
