# Plan: Patch the memoized catalog on write (srs-rust#1196)

## Summary
`FileStore` write/remove dropped both memoized catalogs, so every write in a long-lived session (srs-web, `srs mcp serve`) cost a full re-walk, re-parse and re-validate. Keep the catalog as per-file contributions (`catalog::CatalogState`); a single-file write replaces one contribution and the next read re-runs only the set-level checks.

## Architecture Decisions
No new ADR: implements the memoization of #1108/#1162 within ADR-045's catalog seam. Spec change: none (catalog output is identical by construction and by test). Layer: `srs-repository` only; no CLI/payload change.

## Design
- `build()` = `CatalogState::build` + `assemble`: one classifier (`classify_path`), one merge, one set of [R12]/[R13]/[R14] checks, so a fresh build and a patched state share all code.
- `patch(path, present)` handles a write/removal under a reserved root. It declines (full rebuild) for `manifest.json`, any `package.json`, the extension aggregate, and the first file under a not-yet-existing instance root.
- `create_dir_all` no longer invalidates: the catalog enumerates files only.
- Clones: epoch is synced before every mutation so a stale state is never patched; other handles still drop on epoch change.

## Acceptance
- Property test `patched_catalog_equals_fresh_build_over_random_writes` (MemVfs, random create/update/delete incl. malformed, duplicate ids, fallback paths): patched catalog == fresh `build`/`build_checked`, diagnostics included.
- `cargo test --workspace` (fresh `SRS_SPEC_DIR`), `cargo clippy --workspace --all-targets -- -D warnings`.

## Out of scope
Benchmark (sibling sub-issue of #1193). `assemble` is still O(corpus) string clones (no I/O, parsing or validation).
