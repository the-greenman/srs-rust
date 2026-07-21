# ADR-036: `.srs` (SRSzip) is the default working/interchange format

- **Status:** accepted
- **Date:** 2026-07-21
- **Supersedes:** —
- **Superseded by:** —

## Context

SRS repositories exist in three serialized forms today:

| Format | Extension | Description |
|---|---|---|
| File-tree | (directory) | Native on-disk layout. Authoritative. Not a single file. |
| SRS JSON bundle | `.srsj` | Lightweight single-file projection. Good for snapshots, not for transfer. |
| SRSzip | `.srs` | Deterministic ZIP archive (ADR-033). Full-fidelity, portable, byte-stable. |

The `.srsj` format was the de-facto "portable" format before SRSzip existed, used by `srs repo
copy --to foo.srsj` and in WASM/web contexts. Its limitations:
- It is a projection (derived, not canonical) — round-trips may not preserve all manifest fields.
- It is JSON, not binary — awkward for source documents or large blobs.
- Key ordering is deterministic only when `preserve_order` is disabled (ADR-017), which is already
  enforced, but it is an accidental property of the configuration, not a designed guarantee.

The `.srs` format (ADR-033) was designed to address all of the above. It is a true copy (not a
projection), supports binary content, and guarantees byte-for-byte determinism by construction.

A clear recommendation is needed so:
1. Documentation, CLI UX, and tooling agree on which format to use by default.
2. New features (backup, import, bundle creation) choose `.srs` without revisiting the decision.
3. `.srsj` is still useful as a lightweight snapshot format but is positioned as secondary.

## Decision

**`.srs` (SRSzip) is the default working and interchange format for SRS repositories.**

Concretely:
- `srs archive pack` / `srs archive unpack` are the primary commands for packing/unpacking a repository.
- Documentation and help text should recommend `.srs` for transfer, backup, and round-trip use cases.
- New integrations that need a single-file repository representation should use `.srs` unless they
  have a specific reason to prefer the human-readable JSON form.

**`.srsj` is a legacy lightweight format, not deprecated but secondary.** It remains supported and
useful for:
- Human inspection and quick editing (it is plain JSON).
- Contexts where a WASM binding needs an in-memory snapshot and binary size is a concern.
- Existing tooling that already targets `.srsj`.

No automatic migration is required. `.srsj` support is not removed. The distinction is one of
recommendation, not enforcement.

## Consequences

**Positive:**
- Clear guidance for contributors: default to `.srs`; use `.srsj` only when its specific
  properties (human-readable JSON, in-memory snapshot) are needed.
- New tooling (CI backup scripts, import pipelines) can use `.srs` with a canonical CLI surface
  (`srs archive pack/unpack`) instead of ad-hoc approaches.
- The determinism guarantee of `.srs` (ADR-033) makes content-addressed storage and diffing
  straightforward.

**Negative / trade-offs:**
- `.srsj` loses its status as the recommended portable format. Teams that have built workflows
  around `.srsj` will eventually need to migrate.
- Two single-file formats add cognitive overhead. The distinction (`.srs` = canonical; `.srsj` =
  lightweight snapshot) must be documented clearly.

**Neutral:**
- `.srsj` is not deprecated and no removal timeline is set. It will continue to be supported
  indefinitely unless a future ADR supersedes this one.
- The file-tree (directory) format remains the native on-disk format and is not affected by this
  decision.
