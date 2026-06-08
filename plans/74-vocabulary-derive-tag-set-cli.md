# Plan: Expose `derive-tag-set` as a user-facing CLI command

> Tracks srs-rust#74. Deferred from RFC-006 Task 3 (#73).

## Summary

The `derive_tag_set` service function — which lists every in-use tag key in a repository and classifies it against an open vocabulary (will-be-invalid / read-only-after-close / used-and-active) — already exists in `crates/srs-repository/src/vocabulary_service.rs` as the V10 promotion pre-flight helper. It has no user-facing surface: an author cannot inspect the live usage state of an open vocabulary *before* attempting to promote it. This plan adds `srs vocabulary derive-tag-set <vocabulary-id>`, a read-only command that runs the existing service and returns the classified tag set. All business logic exists; this is CLI wiring plus a payload contract.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude (this session) |
| CLI Worker | claude (this session) |
| Verification | Verification Agent (Stage 7) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | All logic stays in the existing `vocabulary_service::derive_tag_set`; the handler does arg-parse → one service call → `output::serialize`. | accepted |
| [ADR-011](../docs/adr/011-cli-output-contract.md) | New command output is a named payload struct + committed golden schema. | accepted |
| [ADR-012](../docs/adr/012-vocabulary-substrate.md) | Vocabulary substrate (governs the `derive_tag_set` semantics being exposed). | accepted |

No new ADRs — this plan implements ADR-010/011 using infrastructure that already exists.

**Payload shape decision (recorded, not open):** the payload is `VocabularyDeriveTagSetPayload { vocabulary: Vocabulary, entries: Vec<TagSetEntry> }`, as prescribed by issue #74. The full `Vocabulary` is included (mirroring `PromoteVocabularyPayload`) so the caller sees the vocabulary name/mode alongside the classified keys. `TagSetEntry`/`TagSetEntryClassification` are reused from `vocabulary_service` unchanged.

---

## Contracts

### CLI output contract (ADR-011)

**New command added** → add `VocabularyDeriveTagSetPayload` to `crates/srs-cli/src/payload.rs`, wire the handler with `output::serialize()`, register it in `crates/srs-cli/src/bin/generate-schemas.rs`, run `cargo run --bin generate-schemas`, commit the new `crates/srs-cli/schemas/payload/vocabulary-derive-tag-set.json`.

`TagSetEntry` is a `srs-repository` type; embed it in the payload via `#[schemars(with = "...")]` consistent with how `Vocabulary` is embedded elsewhere (`#[schemars(with = "Vec<serde_json::Value>")]` for the entries vector).

Verification: `cargo test --test payload_contracts` passes.

### Entity schema sync (check-schema-sync.sh)

**No** — no JSON Schema files under `srs/docs/schema/2.0/` are touched. No action required.

---

## Scope

- Add `Derive { id: String }` variant to `VocabularyCommand` in `crates/srs-cli/src/commands/mod.rs` (clap subcommand `derive-tag-set`).
- Add `VocabularyDeriveTagSetPayload { vocabulary, entries }` to `crates/srs-cli/src/payload.rs`.
- Add `cmd_vocabulary_derive_tag_set` handler in `crates/srs-cli/src/commands/vocabulary.rs`: fetch the vocabulary (for the payload), call `vocabulary_service::derive_tag_set`, serialize.
- Register golden schema in `generate-schemas.rs`; commit `vocabulary-derive-tag-set.json`.
- Integration tests in `crates/srs-cli/tests/integration_tests.rs`.
- Doc update: `srs/srs-usage.md` vocabulary command section (Stage 7.5, committed in the `srs` repo).

**Out of scope:**

- Any change to the `derive_tag_set` service logic or its classification rules.
- A `--format text` human rendering of the tag set (JSON only, consistent with the other vocabulary commands).
- Exposing `derive-tag-set` in the WASM bindings (separate follow-up if a consumer appears).

---

## Phases

### Phase 1: CLI command, handler, payload, golden schema

**Goal:** `srs vocabulary derive-tag-set <id>` returns an `ok` envelope with the classified tag set; golden schema committed and contract tests green.

**Agent:** CLI Worker

#### Tasks

- [x] Add `Derive { id: String }` variant to `VocabularyCommand` (clap renders it as `derive-tag-set`; doc comment: "Inspect the in-use tag set for an open vocabulary (V10 pre-flight, read-only)").
- [x] Add `VocabularyDeriveTagSetPayload` to `payload.rs` with `vocabulary: Vocabulary` (`#[schemars(with = "serde_json::Value")]`) and `entries: Vec<TagSetEntry>` (`#[schemars(with = "Vec<serde_json::Value>")]`). Import `TagSetEntry` from `srs_repository::vocabulary_service`.
- [x] Add `cmd_vocabulary_derive_tag_set(ctx, id)` to `commands/vocabulary.rs`: in one `with_store` closure, fetch the vocabulary via `get_vocabulary_by_id` (NotFound → `vocabulary get`-style not-found is unnecessary; return the service NotFound error envelope) and call `derive_tag_set`; serialize to `"vocabulary derive-tag-set"`.
- [x] Wire the new variant in `dispatch`.
- [x] Register `write_schema!("vocabulary-derive-tag-set", VocabularyDeriveTagSetPayload)` in `generate-schemas.rs`; run the generator; commit the golden file.

#### Acceptance Criteria

- [x] `srs vocabulary derive-tag-set <id>` on a vocabulary returns `ok:true`, `command == "vocabulary derive-tag-set"`, `payload.entries` sorted by key with correct `classification` values.
- [x] An unknown vocabulary id returns an error envelope (not a panic).
- [x] `payload.vocabulary` carries the resolved vocabulary (name, mode).
- [x] Golden `vocabulary-derive-tag-set.json` committed; `payload_contracts` passes.

#### Testing

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
cargo test --test payload_contracts
```

Specific tests to write:

- `vocabulary_derive_tag_set_classifies_in_use_keys` — create open vocab with one active term, tag two notes (one key with an active term → `used-and-active`, one key with no term → `will-be-invalid`); assert classifications and usage counts.
- `vocabulary_derive_tag_set_unknown_id_returns_error` — unknown id → `ok:false` envelope, no panic.

#### Milestone gate

1. Verify acceptance criteria.
2. Confirm both tests exist and pass.
3. `cargo test -p srs-cli && cargo clippy -p srs-cli -- -D warnings`.
4. Mark checkboxes `[x]`.
5. Commit referencing `(#74)`.

---

## Final Acceptance

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] CLI integration tests pass.
- [ ] `cargo test --test payload_contracts` passes.
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed — should be a no-op pass).
- [ ] `srs vocabulary derive-tag-set <id>` works end-to-end against a real repo.
- [ ] `srs/srs-usage.md` documents the new command (Stage 7.5).

## Coordination Rules

- Single agent (this session) — no cross-agent write coordination needed.
- No changes to `srs-repository` service logic.

## Assumptions

- `vocabulary_service::derive_tag_set`, `DeriveTagSetInput`, `DeriveTagSetResult`, `TagSetEntry`, and `TagSetEntryClassification` are stable and correct (validated as the V10 promote pre-flight). This plan only surfaces them.
- `get_vocabulary_by_id` is the right way to resolve the vocabulary for the payload (same call the service uses internally).
