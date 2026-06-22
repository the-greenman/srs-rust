# Plan: Deterministic `.srsj` writes (JsonStore `data` ordering)

## Summary

`JsonStore` serialises its in-memory `data` map directly into the `.srsj` envelope, and that map is a `std::collections::HashMap` ([`crates/srs-repository/src/json_store.rs:27`](../crates/srs-repository/src/json_store.rs#L27) and [`:33`](../crates/srs-repository/src/json_store.rs#L33)). HashMap iteration order is non-deterministic, so every write re-emits the `data` entries in a different order even when content is unchanged. The result: a one-field edit rewrites the whole file (~1400+ line diff in a real repo), writes are not idempotent, and committed `.srsj` fixtures cannot be regenerated or reviewed with a minimal diff. `to_srsj_string` is shared by `srs-bindings::export_srsj`, so srs-web's in-app saves are equally non-deterministic. This plan makes `.srsj` serialisation deterministic and idempotent by switching `data` to a `BTreeMap`. Issue [#171](https://github.com/the-greenman/srs-rust/issues/171).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Lead Integrator |
| Repository Service Worker | Repository Service Worker |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-007](../docs/adr/007-file-index-io-ordering.md) | File-before-index write ordering keeps the index consistent. ADR-017 complements it: index consistency + byte-stable serialisation. | accepted |
| [ADR-015](../docs/adr/015-wasm-write-and-export.md) | `to_srsj_string` is the pure `.srsj` export primitive; `flush()`/`export_srsj` delegate to it. This plan strengthens it with a determinism guarantee. | proposed |
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | The `.srsj` envelope's `data` map is a `BTreeMap`, giving deterministic, minimal-diff, idempotent writes. | accepted |

**Why a new ADR:** Switching `data` from `HashMap` to `BTreeMap` establishes a constraint that a future contributor might plausibly revisit (e.g. reintroducing `HashMap` for lookup performance, or enabling serde_json `preserve_order`). ADR-017 records *why* deterministic ordering is a required property of the `.srsj` format so the choice is not silently reverted.

---

## Contracts

### CLI output contract (ADR-011)

**No new/changed commands.** This plan changes the on-disk byte ordering of `.srsj` files, not any CLI command payload struct. No `payload.rs` change, no schema regeneration. The JSON *content* of every command output is unchanged.

### Entity schema sync (check-schema-sync.sh)

**No.** This plan does not touch any JSON Schema under `srs/docs/schema/2.0/`. The `.srsj` envelope is an implementation serialisation format, not a spec-defined entity schema.

---

## Scope

- Change `JsonStoreFile.data` and `JsonStoreState.data` from `HashMap<String, serde_json::Value>` to `BTreeMap<String, serde_json::Value>` in [`crates/srs-repository/src/json_store.rs`](../crates/srs-repository/src/json_store.rs).
- Update the two `HashMap::new()` initialisers for `data` (in `create` and `initialize_repository`) to `BTreeMap::new()`.
- Add a regression test asserting `.srsj` write idempotence and deterministic key ordering (the round-trip byte-equality test the issue suggests).
- Draft ADR-017.

**Out of scope:**

- `Manifest.extra` (`HashMap`, flattened) and entity-level `extra` HashMaps (Field/Type/Container/…): already deterministic in the `.srsj` write path because they are serialised via `serde_json::to_value` before reaching `data`, which produces a `BTreeMap`-backed `serde_json::Value` (serde_json's `preserve_order` feature is **not** enabled in this workspace, confirmed in `Cargo.toml`). Sorted keys regardless of HashMap order. No change needed. This determinism depends on `preserve_order` staying disabled *and* on save paths continuing to use `to_value` (not hand-built `json!()` from HashMap sources) — ADR-017 records both as required rules.
- The `load_text_file("manifest.json")` display path (uses `to_string_pretty(&manifest)` directly) — its ordering only affects an in-memory display string, never a committed file. Not part of the `.srsj` write contract. Left as-is.
- Any change to `FileStore` (one-file-per-record; each file's key order is already governed by serde struct field order / BTreeMap-backed `Value`).

---

## Phases

### Phase 1: Deterministic `data` ordering

**Goal:** `.srsj` writes are byte-for-byte deterministic and idempotent; a no-op content write produces an identical file.

**Agent:** Repository Service Worker

**Call-site audit (done at plan time):** `data` is accessed only via `.get`, `.insert`, `.remove`,
`.contains_key`, `.keys`, and `.clone` (confirmed by `grep -oE '\.data\.[a-z_]+'` over the file — no
`entry()`, no capacity hints, no `HashMap`-specific API). All have identical signatures on `BTreeMap`, so
the type swap compiles with **zero** call-site changes. The only iteration-order-sensitive uses are
`.keys()` in `list_instance_files` (line ~1146) and `list_files_recursive` (line ~1304); after the change
these return keys in sorted order instead of arbitrary order — a benign, strictly-more-deterministic change
(no caller depends on discovery-iteration order; loaders iterate without order assumptions).

#### Tasks

- [x] Change `JsonStoreFile.data` (line ~27) to `BTreeMap<String, serde_json::Value>`.
- [x] Change `JsonStoreState.data` (line ~33) to `BTreeMap<String, serde_json::Value>`.
- [x] Add `use std::collections::BTreeMap;` and update **only the two `data` initialisers** — `JsonStore::create` (line ~204) and `initialize_repository` (the `state.data` path; `data: envelope.data` in `from_srsj` needs no change as it deserialises into the new type) — from `HashMap::new()` to `BTreeMap::new()`. **Do NOT touch** the `HashMap::new()` calls for `manifest.extra` (lines ~193, ~604) or any other `HashMap` (`rt_by_type`, `field_sources`, entity `extra`).
- [x] Draft `docs/adr/017-deterministic-srsj-serialization.md` (status `proposed`) — done as part of this plan; commit it in the milestone.
- [x] Add test `json_store_srsj_write_is_deterministic_and_idempotent` with these explicit assertions:
  1. Build a store via `from_srsj` whose `data` has ≥3 entries whose keys are NOT in sorted order in the source JSON (e.g. `"zebra.json"`, `"alpha.json"`, `"package/package.json"`).
  2. `let s1 = store.to_srsj_string().unwrap(); let s2 = store.to_srsj_string().unwrap(); assert_eq!(s1, s2);` — same store, two writes, byte-identical.
  3. `let reloaded = JsonStore::from_srsj(&s1).unwrap(); assert_eq!(reloaded.to_srsj_string().unwrap(), s1);` — write(read(x)) == write(read(write(read(x)))), proving idempotence across a round-trip.
  4. Parse `s1`, take `parsed["data"].as_object().unwrap().keys()`, collect to a `Vec`, clone it, sort the clone, and `assert_eq!` the two — proving top-level `data` keys are emitted in sorted order. (Holds because `serde_json::Map` is `BTreeMap`-backed *and* the source `data` is now a `BTreeMap`.)

#### Acceptance Criteria

- [x] `data` is a `BTreeMap` in both structs; crate compiles with no other call-site changes (BTreeMap supports `get`/`insert`/`remove`/`keys`/`contains_key`/`clone` already used).
- [x] The new determinism test passes.
- [x] No regression: all existing `json_store_*` and `from_str_*`/`to_srsj_*` tests pass.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests to write or verify:

- `json_store_srsj_write_is_deterministic_and_idempotent` (new) — proves writes are stable and idempotent and keys are sorted.
- Regression: run `cargo test -p srs-repository json_store` — the existing `json_store_*`/`from_str_*`/`to_srsj_*`/`open_delegates_to_from_str` tests (incl. `to_srsj_string_returns_valid_srsj_envelope` at line 2139, `from_str_roundtrip` at 2078, `open_delegates_to_from_str` at 2119 — all confirmed present) must pass unchanged.

#### Milestone gate

1. Verify acceptance criteria.
2. Confirm the named test exists and passes.
3. Run:

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

4. Mark checkboxes `[x]`.
5. Commit referencing `#171`.

---

## Final Acceptance

- [x] `cargo test` passes with no failures.
- [x] `cargo clippy -- -D warnings` passes.
- [x] CLI output format unchanged (integration tests pass).
- [x] `cargo test --test payload_contracts` passes (no payload structs changed — expected pass unchanged).
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed) — or N/A if not present.
- [ ] A no-op `srs type update` on a real `.srsj` produces a zero-line `diff` (verified in dogfooding).
- [x] ADR-017 drafted and accepted (ships in this change).

## Coordination Rules

- Single-phase, single-crate change — Repository Service Worker owns the edit; Verification Agent confirms no regression.
- Keep the change minimal: only `data` becomes `BTreeMap`; do not refactor unrelated `HashMap` usage.

## Assumptions

- serde_json `preserve_order` remains disabled (confirmed in `Cargo.toml`), so `serde_json::Value` objects serialise with sorted keys — the only non-determinism is the top-level `data` HashMap. If `preserve_order` were ever enabled, nested `Value` ordering would follow insertion order; that is out of scope here and would be a separate decision.
- No consumer depends on the current (non-deterministic) `data` ordering. None can, by definition.
