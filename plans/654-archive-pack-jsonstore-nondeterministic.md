# Plan: Fix non-deterministic archive pack from JsonStore source

## Summary

`srs archive pack` produces byte-different `.srs` archives on repeated runs when the source
is a `.srsj` (JsonStore). The differing ZIP entry is `manifest.json` only — the keys in the
`extra: HashMap<String, Value>` field of `Manifest` are emitted in HashMap-random order by
`JsonStore::load_text_file("manifest.json")`, which serializes the typed `Manifest` struct
directly via `serde_json::to_string_pretty`. This violates ADR-033 / ADR-036 determinism
guarantees. The `FileStore` path is already correct (it writes manifest.json via
`serde_json::to_value`, which normalises all keys through serde_json's BTreeMap-backed Map).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Repository Worker | Claude (this session) |
| Verification | Claude (this session) |

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-017](../docs/adr/017-deterministic-srsj-serialization.md) | BTreeMap / `to_value` for deterministic serialization | accepted |
| [ADR-033](../docs/adr/033-srs-archive-format.md) | Archive determinism requirement | accepted |
| [ADR-036](../docs/adr/036-srs-is-default-working-format.md) | `.srs` byte-stability guarantee | accepted |

No new ADRs required — the fix applies the existing ADR-017 pattern (route through
`serde_json::to_value` before serializing a map with a flattened `extra` HashMap) to
a code path that was missed.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. `manifest.json` content in the ZIP is semantically
unchanged (key order is not part of the JSON contract). No payload structs changed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are changed.

---

## Scope

- Fix `JsonStore::load_text_file("manifest.json")` to route through `serde_json::to_value`
  before `to_string_pretty`, matching the existing `FileStore::save_manifest` write path.
- Extend `test_archive_determinism` (currently MemoryStore only) to also cover a JsonStore
  source — self-consistency and cross-store byte-identity.

**Out of scope:**
- Changing `Manifest.extra` from `HashMap` to `BTreeMap` (deferred; that's a wider refactor
  touching many files; the `to_value` fix is sufficient and consistent with ADR-017).
- Changes to FileStore, MemoryStore, or any other store type.

---

## Phases

### Phase 1: Fix JsonStore manifest serialization

**Goal:** `JsonStore::load_manifest_raw_text` produces deterministic, sorted-key JSON.

**Agent:** Repository Worker

#### Tasks

- [x] In `crates/srs-repository/src/json_store.rs`, change the `"manifest.json"` branch of
  `load_text_file` to serialize via `serde_json::to_value(&manifest)` first, then
  `serde_json::to_string_pretty(&value)` — identical pattern to `FileStore::save_manifest`.

#### Acceptance Criteria

- [x] Two consecutive `archive_pack` calls on a JsonStore produce byte-identical output.
- [x] `archive_pack` on a JsonStore produces the same `manifest.json` bytes as packing from
  a FileStore containing identical repository state.

#### Testing

Specific tests to write or verify:

- `test_archive_determinism_from_jsonstore` in `crates/srs-repository/src/archive.rs` — packs the
  same JsonStore twice and asserts byte-identical output.
- `test_archive_manifest_bytes_identical_filestore_vs_jsonstore` — initializes both FileStore and
  JsonStore with identical data, packs each, extracts `manifest.json` from both ZIPs, asserts
  byte-identical manifest content.

```bash
cargo test -p srs-repository test_archive_determinism
cargo test -p srs-repository test_archive_determinism_from_jsonstore
cargo test -p srs-repository test_archive_manifest_bytes_identical_filestore_vs_jsonstore
```

#### Milestone gate

1. All three tests pass.
2. `cargo clippy -p srs-repository -- -D warnings` clean.
3. `cargo test -p srs-repository` passes in full.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] `test_archive_determinism_from_jsonstore` exists and passes
- [ ] `test_archive_manifest_bytes_identical_filestore_vs_jsonstore` exists and passes

## Coordination Rules

- Lead Integrator owns all writes in this plan.
- Scope is confined to `crates/srs-repository/src/json_store.rs` (fix) and the archive tests.

## Assumptions

- serde_json's `preserve_order` feature remains disabled (ADR-017 invariant).
- `serde_json::to_value` on a `Manifest` with `#[serde(flatten)] extra: HashMap<...>` produces a
  BTreeMap-backed Object with sorted keys — same guarantee relied on by `FileStore::save_manifest`.
