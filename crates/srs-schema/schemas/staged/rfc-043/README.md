# Staged RFC-043 schemas (dataModelRevision 8)

NOT a mirror. These five files are verbatim copies of `srs` `docs/schema/staged/rfc-043/`
(taken from `origin/master`, srs PR #853) for the schemas whose *shape* changes at
`dataModelRevision` 8: `container`, `composition`, `document-view-output`, `manifest`,
`srsj-envelope`. `srs-schema` compiles these in place of the `schemas/2.0/` copies of the
same names.

Why: the revision-bump choreography releases srs-rust (this support) before the `srs`
corpus-migration PR promotes the staged schemas into `docs/schema/2.0/` (srs#852). The
`schemas/2.0/` mirror therefore cannot carry them yet, and `check-schema-drift.sh` would
(rightly) fail if it did.

Retire this directory when srs#852 lands: run `scripts/sync-schemas-from-spec.sh`, delete
`schemas/staged/`, and point `include_schema!` in `src/lib.rs` back at `schemas/2.0/`.
