# Staged RFC-052 schemas (dataModelRevision 10)

NOT a mirror. These three files are verbatim copies of `srs` `docs/schema/staged/rfc-052/`
(taken from `origin/master` at `3a19d50`, srs PR #942) for the schemas RFC-052 changes:
`manifest` (`rootContainerId` replaces the inline `container`), `srsj-envelope` (its embedded
manifest) and `container` (descriptions only). `srs-schema` compiles these in place of the
`schemas/2.0/` copies of the same names.

Why: the revision-bump choreography releases srs-rust (this support, srs-rust#1417) before the
`srs` corpus-migration PR promotes the staged schemas into `docs/schema/2.0/` (srs#948). The
`schemas/2.0/` mirror therefore cannot carry them yet, and `check-schema-drift.sh` would
(rightly) fail if it did. Same arrangement as `staged/rfc-043/` (srs-rust#1144).

Retire this directory when srs#948 lands: run `scripts/sync-schemas-from-spec.sh`, delete
`schemas/staged/`, and point `include_schema!` in `src/lib.rs` back at `schemas/2.0/`.
