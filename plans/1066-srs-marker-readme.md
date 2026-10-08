# Plan: default `.srs/README.md` orientation file (#1066)

**Spec gate:** no spec change — RFC-038 [R5] / Invariant 45 already make the marker's contents implementation-private; which file satisfies the convention is tooling.

## Scope
- New `crates/srs-repository/src/marker_readme.md` (small, non-authoritative, points at `srs.semanticops.com`, names MCP/CLI without a tool roster, warns against hand-editing) exposed as `vfs::SRS_MARKER_README` / `SRS_MARKER_README_PATH`.
- Replace the `.srs/.gitkeep` emitters: `FileStore::initialize_repository` (`repo create`), `tree_session::export_tree`, `archive::with_marker`, `slice_service`.
- `.srsj` projection omits the README only when byte-identical to the default; custom content travels.

## Out of scope
- `gallery-project-v2` and `srs/.srs` README content (other repos; follow-up).
- Reading old `.gitkeep` trees: unchanged (still just a file under `.srs/`).

## Decisions
- ADR-038/050 untouched; non-normative text, so no new ADR. Decision mode: clear. Door: non-normative tooling (owner-merge retained per issue).

## Tests
- `export_tree_synthesizes_marker`, archive/srsj parity test, `repo create` writes README.
