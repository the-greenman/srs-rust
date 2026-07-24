# Plan: Architecture-hygiene fixes from weekly review routine

> Issue: [#744](https://github.com/the-greenman/srs-rust/issues/744)

## Summary

A weekly automated architecture-review routine audited this week's commits against
`docs/architecture/capability-layering.md` and the ADRs it depends on. It surfaced five small,
independent hygiene issues: two DRY/false-positive bugs in the container-embed fallback path
(#723 cluster), one nominal-vs-structural inconsistency in RFC-017 validation, and two minor
duplication/error-swallowing issues in `srs-gov`. All five are implemented directly (each is a
mechanical, low-risk fix with no design decision beyond what's documented below) and verified
green before this plan was written.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | session lead |
| Repository Worker | session lead (all phases — already implemented) |
| Verification | Verification Agent (`agents.md#verification-agent`) |

## Architecture Decisions

No new ADRs. This plan applies existing decisions:
- ADR-010 (service boundary contract) — fixes 1/2 route validation/rendering through the existing
  `container_service::get_container` service helper instead of ad hoc store calls in callers.
- Capability-layering "structural not nominal" rule — fix 3 replaces a raw string comparison with
  the typed `SourceType`/`SourceRole` enums already used elsewhere for the same semantic check.

No new architectural constraints are established and no prior decision is changed.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed CLI commands or payload shapes. No `generate-schemas` run needed.

### Entity schema sync

No entity schema changes.

---

## Scope

- `crates/srs-repository/src/validation.rs` — document-view container-reference check uses
  `container_service::get_container` (embed-fallback aware) instead of raw `store.load_container`.
- `crates/srs-repository/src/validation.rs` — RFC-017 R2/R12 sourceRef check compares typed
  `SourceType`/`SourceRole` instead of raw string literals.
- `crates/srs-repository/src/render_service.rs` — `resolve_container_title`'s embed-fallback
  reimplementation replaced with a call to `container_service::get_container`.
- `crates/srs-gov/src/main.rs` — `cmd_relate` instance-ID-prefix resolution deduplicated into a
  `resolve_full_instance_id` helper.
- `crates/srs-gov/src/srs.rs` — `run_srs_impl` surfaces stderr + exit code when JSON parsing fails
  after a non-zero exit, instead of only reporting "not valid JSON".

**Out of scope** (each filed as a separate follow-up issue by the review routine, already parented):
- srs-rust#740 (srs-gov `cmd_create` placeholder-string leak — needs a design decision).
- srs-rust#741 (missing `--json` test coverage for the 5 new srs-gov editing verbs).
- srs-rust#742 (`delete_container` embed-only-root asymmetry — needs a product decision).
- srs-rust#743 (missing test coverage for `archive.rs::missing_tree_definitions`).

## Phases

### Phase 1: Container-embed fallback consistency

**Goal:** every container lookup path (validation diagnostics, render-title resolution) is
embed-aware via the single `container_service::get_container` helper — no ad hoc reimplementation.

**Agent:** Repository Worker

#### Tasks

- [x] `validation.rs`: dangling document-view container-reference check calls
  `container_service::get_container` instead of `store.load_container`.
- [x] `render_service.rs`: `resolve_container_title`'s raw-load + manual-embed-check fallback
  block replaced with one `container_service::get_container` call.

#### Acceptance Criteria

- [x] A document view referencing an embed-only RFC-013 root no longer produces a false-positive
  "dangling reference" diagnostic.
- [x] `resolve_container_title` has no inline reimplementation of embed-fallback logic.
- [x] A regression test exercises the actual bug scenario (added per Stage-3 architecture review
  finding): `validate_document_view_embed_only_root_container_ref_is_not_dangling` in
  `validation.rs`. Verified to fail against the pre-fix code (reproduces the exact false-positive
  diagnostic) and pass against the fix.

#### Testing

```bash
cargo test -p srs-repository --lib
```

#### Milestone gate

`cargo test -p srs-repository --lib` — 1246 passed, 0 failed. `cargo clippy --workspace -- -D
warnings` clean.

---

### Phase 2: RFC-017 structural comparison

**Goal:** the RFC-017 'attaches' sourceRef check uses the same typed vocabulary as the rest of the
codebase.

**Agent:** Repository Worker

#### Tasks

- [x] `validation.rs`: deserialize `sourceType`/`sourceRole` into `SourceType`/`SourceRole` and
  compare typed variants instead of raw JSON strings.

#### Acceptance Criteria

- [x] Behavior unchanged for well-formed sourceRefs (kebab-case serde matches the prior string
  literals exactly); malformed values are skipped the same as before (deserialization failure →
  `None` → `continue`, matching the old `as_str()` miss path).

#### Testing

```bash
cargo test -p srs-repository --lib validation
```

#### Milestone gate

Passed — see Phase 1 gate (same test run covers both files).

---

### Phase 3: srs-gov hygiene

**Goal:** remove the `cmd_relate` ID-resolution duplication and stop swallowing `srs` subprocess
stderr on invalid-JSON-after-nonzero-exit.

**Agent:** Repository Worker

#### Tasks

- [x] `crates/srs-gov/src/main.rs`: extract `resolve_full_instance_id(id, repo)`, use it for both
  source and target in `cmd_relate`.
- [x] `crates/srs-gov/src/srs.rs`: `run_srs_impl` includes stderr + exit code in the error when
  JSON parsing fails after a non-zero exit; successful-exit parse-failure message unchanged.

#### Acceptance Criteria

- [x] `cmd_relate` behavior unchanged (same error messages, same happy path).
- [x] A non-zero `srs` exit with invalid JSON stdout now surfaces stderr in the error.

#### Testing

```bash
cargo test -p srs-gov
```

#### Milestone gate

`cargo test -p srs-gov` — 54 passed (25 unit + 29 `flow.rs` integration), 0 failed.

---

## Final Acceptance

- [x] `cargo test` (workspace) passes
- [x] `cargo clippy --workspace -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (112/112 — unaffected, no payload changes)
- [x] No entity schema changes
- [x] No new ADRs required
