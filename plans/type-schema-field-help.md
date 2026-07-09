# Plan: Type-Schema Field Help (description + instructions)

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.

## Summary

The `type schema` projection (`field_to_property` in `crates/srs-repository/src/type_schema_service.rs`)
only uses a field's `description` as a `title` fallback and never emits `instructions` at all —
`instructions` isn't even a typed field on core `Field` today, it only rides the `extra` flatten map.
This blocks the srs-web record editor from showing field help text. This plan models `instructions`
as a typed `Option<String>` on `Field` and projects both `description` and `instructions` into the
type-schema property object under dedicated `x-srs-*` vendor keys, so neither collides with the
already-occupied `title`/`description` JSON Schema keywords (see ADR-023).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Core Model Worker | Claude (this session) |
| Repository Service Worker | Claude (this session) |
| Verification | Claude (this session) |

See [agents.md](agents.md) for role definitions. This is a single-crate-pair, small-sized change; no
new agent role is required.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-023](../docs/adr/023-type-schema-field-help-keys.md) | `x-srs-description` / `x-srs-instructions` vendor keys carry field help text in the type-schema projection | proposed |
| [ADR-014](../docs/adr/014-composite-schema-property-naming.md) | Convention consistency (not governance): dedicated `x-srs-*` keys name a datum's semantic role rather than overloading a standard JSON Schema keyword | accepted (informs this plan's naming; ADR-014 itself governs only `blueprint_schema_service` property-key naming — none of `type_schema_service`'s existing vendor keys were ever governed by an ADR, so ADR-023 is the first ADR for *this* projection's vendor-key convention) |

ADR-023 already exists as a draft in this worktree from prior work on this issue; this plan carries
it forward with status `proposed` (per the standard ADR lifecycle — it flips to `accepted` in Stage
7.5 once the change ships) and finalizes its wording.

**Design decision note:** the only long-term-consequence choice here — the vendor-key names
`x-srs-description` / `x-srs-instructions` — directly follows the existing ADR-014 convention
(dedicated `x-srs-*` key per semantic role, already used for `x-srs-ai-guidance`, `x-srs-order`,
`x-srs-widget`, etc. in this same function). It does not introduce a new naming philosophy, so this
plan does not pause for a fresh human design decision; it cites precedent instead.

---

## Contracts

### CLI output contract (ADR-011)

No new or changed CLI commands. `TypeSchemaPayload` (and `field-get`/`type-get`) wrap the projected
schema as an opaque `serde_json::Value`, so adding keys inside that value produces **no** payload
struct change and **no** golden diff. Verify with `cargo run --bin generate-schemas` (expect clean
git diff under `schemas/payload/`) and `cargo test --test payload_contracts`.

### Entity schema sync (check-schema-sync.sh)

No changes to `srs/docs/schema/2.0/`. `instructions` is already declared on the spec's `field.json`
(confirmed at `srs/docs/schema/2.0/field.json:29`) — this plan only catches the Rust model up to the
spec, so `bash scripts/check-schema-sync.sh` is unaffected.

---

## Scope

- Add `pub instructions: Option<String>` to `Field` in `crates/srs-core/src/types/field.rs`, with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- Update every `Field { .. }` struct literal in the workspace to set `instructions: None` (or a real
  value where a test specifically exercises it) — **except** the two production package-loading
  paths below, which must thread the real value through.
- **Thread `instructions` through both real storage adapters' `FieldJson → Field` mapping** —
  `crates/srs-repository/src/json_store.rs` (`JsonStore`/`.srsj`) and `crates/srs-repository/src/store.rs`
  (`FileStore`). Both have a `FieldJson` deserialization struct that currently discards `instructions`
  into an unread `_extra` flatten map; without this, real repo loads would silently drop authored
  `instructions` even though the in-memory model and tests would look correct (`MemoryStore`-backed
  tests never touch `FieldJson`). This was caught in plan review (architecture reviewer, blocking) —
  see Phase 1b below.
- In `field_to_property` (`crates/srs-repository/src/type_schema_service.rs`):
  - Insert `x-srs-description` when `field.description` is non-empty.
  - Insert `x-srs-instructions` when `field.instructions` is `Some` and non-empty.
- No separate change needed for the RFC-007 field-group sub-field builder
  (`field_group_to_property`) — it already calls `field_to_property` per sub-field, so both new keys
  flow through automatically.
- Add a `type_schema_service` unit test asserting both keys appear for a field carrying description +
  instructions, and that `x-srs-instructions` is absent when `instructions` is `None`.

**Out of scope:**

- Any change to `srs/docs/schema/2.0/field.json` (already declares `instructions`).
- Any WASM binding change (the existing `type_schema` binding carries the projection automatically).
- srs-web consumption of the new keys (tracked as a companion issue in `the-greenman/srs-web`,
  referenced in the parent issue).
- Emitting `instructions` anywhere outside the type-schema projection (e.g. `field get` CLI output
  already returns the full `Field` via its existing serde derive — the new field appears there for
  free once modeled on `Field`, no extra work needed, but no other projection is touched).

---

## Phases

### Phase 1: Model `instructions` on core `Field`

**Goal:** `Field` carries a typed `instructions: Option<String>`; the whole workspace compiles with
every existing struct literal updated.

**Agent:** Core Model Worker

#### Tasks

- [x] In `crates/srs-core/src/types/field.rs`, add:
  ```rust
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub instructions: Option<String>,
  ```
  immediately after `pub description: String,` on the `Field` struct.
- [x] Update the two `Field { .. }` literals inside `crates/srs-core/src/types/field.rs` itself
  (lines ~46 and ~114) to add `instructions: None`.
- [x] Update every other `Field { .. }` struct literal in the workspace to add `instructions: None`:
  - `crates/srs-repository/src/blueprint_schema_service.rs` (`fn field`, ~line 294)
  - `crates/srs-repository/src/blueprint_brief_service.rs` (`fn make_field`, ~line 481; and the
    `FIELDS.iter().map(|(id, name, vt)| Field { .. })` closure, ~line 805)
  - `crates/srs-repository/src/discovery_service.rs` (`fn field`, ~line 226)
  - `crates/srs-repository/src/text_projection.rs` (`fn field`, ~line 187)
  - `crates/srs-repository/src/diff.rs` (`fn make_field`, ~line 490)
  - `crates/srs-repository/src/container_view_service.rs` (`fn field`, ~line 381)
  - Any other `Field { .. }` literal `cargo build` flags as missing the field (this list is from a
    workspace grep; treat it as a checklist, not necessarily exhaustive — let the compiler catch the
    rest).

#### Acceptance Criteria

- [x] `cargo build` succeeds workspace-wide with zero "missing field `instructions`" errors.
- [x] `Field`'s serde round-trip still omits `instructions` from JSON when `None` (existing
  `field.rs` serialization tests continue to pass unmodified).

#### Testing

```bash
cargo test -p srs-core
cargo build --workspace
```

Specific tests to write or verify:

- Existing `srs-core::types::field` tests — must pass unmodified (no new test needed here; this
  phase is purely additive/mechanical).

#### Milestone gate

1. Verify acceptance criteria above.
2. Confirm `cargo build --workspace` and `cargo test -p srs-core` pass.
3. ```bash
   cargo test -p srs-core
   cargo clippy -p srs-core -- -D warnings
   ```
4. Mark checkboxes `[x]`, commit: `feat(core): model instructions on Field (#415)`.

---

### Phase 1b: Thread `instructions` through the real storage adapters

**Goal:** `FileStore` and `JsonStore` package loads actually populate `Field.instructions` from disk
instead of silently dropping it — the gap an in-memory-only Phase 1 would otherwise leave.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `crates/srs-repository/src/json_store.rs`, add `instructions: Option<String>,` to the
  `FieldJson` struct (~line 79, alongside `description: Option<String>,`; no `#[serde(default)]`
  needed since the field is already `Option`). In the `Field { .. }` construction that consumes it
  (~line 362-375), add `instructions: fj.instructions,`.
- [ ] In `crates/srs-repository/src/store.rs`, make the identical change to its own `FieldJson`
  struct (~line 425) and `Field { .. }` construction (~line 568-580).
- [ ] Add a cross-store roundtrip test (per `CLAUDE.md`'s Storage Boundary Rules: "New service
  features need at least one cross-store roundtrip test") that writes a field with a non-empty
  `instructions` value, loads it back through both `FileStore` and `JsonStore`, and asserts
  `field.instructions` matches. Place it in `crates/srs-repository/src/json_store.rs`'s or
  `store.rs`'s existing test module (wherever existing package round-trip tests for `Field` already
  live — follow that file's convention).

#### Acceptance Criteria

- [ ] A `Field` with `instructions` set in its source JSON survives a `FileStore` load with
  `instructions` populated (not dropped into `extra`).
- [ ] The same holds for a `JsonStore` (`.srsj`) load.
- [ ] `MemoryStore` was already correct (it holds `Field` directly) — no change needed there.
- [ ] The *write* path (`save_field`/`update_field_file` in both stores) already serializes the whole
  `Field` struct directly via `serde_json::to_value`, not through `FieldJson` — writes already
  round-trip `instructions` with zero code change. Only the read path had the gap.

#### Testing

```bash
cargo test -p srs-repository json_store
cargo test -p srs-repository store
```

Specific tests to write or verify:

- A new cross-store roundtrip test asserting `instructions` survives `FileStore` and `JsonStore`
  loads (see Tasks above) — this is the test that would have caught the original gap.

#### Milestone gate

1. Verify acceptance criteria above.
2. Confirm the new roundtrip test exists and passes for both adapters.
3. ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark checkboxes `[x]`, commit: `fix(repository): thread instructions through FileStore/JsonStore field loading (#415)`.

---

### Phase 2: Project `description` + `instructions` into type-schema help keys

**Goal:** `field_to_property` emits `x-srs-description` and `x-srs-instructions`; a dedicated test
proves it; the whole workspace is green.

**Agent:** Repository Service Worker

#### Tasks

- [ ] In `field_to_property` (`crates/srs-repository/src/type_schema_service.rs`), after the existing
  `title` block, insert:
  ```rust
  if !field.description.is_empty() {
      prop.insert("x-srs-description".into(), json!(field.description));
  }
  if let Some(instructions) = &field.instructions {
      if !instructions.is_empty() {
          prop.insert("x-srs-instructions".into(), json!(instructions));
      }
  }
  ```
  Place it before the existing `x-srs-order` / `x-srs-field-id` inserts, or after — order in the
  `Map` doesn't affect JSON semantics; keep it adjacent to the `title` block since both derive from
  `description`.
- [ ] Add a unit test in `crates/srs-repository/src/type_schema_service.rs`'s existing test module
  (alongside the other `field_to_property` / `type_schema` tests) that:
  - builds a `Field` with a non-empty `description` and `instructions: Some("...".to_string())`,
  - asserts the resulting property object has `x-srs-description` equal to the field's description
    and `x-srs-instructions` equal to the field's instructions.
- [ ] Add a second assertion (same test or a sibling) that a `Field` with `instructions: None` does
  **not** carry an `x-srs-instructions` key.

#### Acceptance Criteria

- [ ] `srs` type-schema output for a field with both `description` and `instructions` carries both
  `x-srs-description` and `x-srs-instructions`.
- [ ] A field with empty `description`, or `instructions` that is `None` **or** `Some("")`, omits the
  corresponding key (no empty strings emitted) — the `!instructions.is_empty()` guard in the code
  above covers both the `None` and `Some("")` cases identically since it's checked after unwrapping.
- [ ] Field-group sub-fields (RFC-007) also carry the new keys, verified by the fact that
  `field_group_to_property` delegates to `field_to_property` unchanged — no explicit new test
  required there, but confirm no existing field-group test asserts a fixed property-key set that
  the new keys would silently break (`cargo test -p srs-repository field_group` must stay green).

#### Testing

```bash
cargo test -p srs-repository type_schema_service
cargo test -p srs-repository field_group
```

Specific tests to write or verify:

- `type_schema_service::tests::field_to_property_emits_description_and_instructions_keys` (new) —
  proves both keys appear with correct values.
- `type_schema_service::tests::field_to_property_omits_absent_instructions` (new, or folded into the
  same test as a second assertion) — proves no key is emitted when the source is empty/`None`.

#### Milestone gate

1. Verify acceptance criteria above.
2. Confirm the two named tests exist and pass.
3. ```bash
   cargo test -p srs-repository
   cargo clippy -p srs-repository -- -D warnings
   ```
4. Mark checkboxes `[x]`, commit: `feat(schema): project field description + instructions as x-srs-* help keys (#415)`.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] CLI output format unchanged (integration tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed, but run it to confirm)
- [ ] `cargo run --bin generate-schemas` produces no diff under `schemas/payload/`
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed, but run to confirm)
- [ ] `srs` type-schema output for a real repo/type with at least one field carrying both
  `description` and `instructions` carries `x-srs-description` and `x-srs-instructions` accordingly.
  No `com.mudemocracy.governance` fixture exists in this worktree — Stage 7.6 dogfooding will use a
  freshly created dogfood repo (`srs repo create` + `srs field create` with `instructions` set, or
  the `srs/srs` spec repo if any of its fields already declare `instructions`) rather than searching
  for a nonexistent fixture.

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers return changed file paths and a short behaviour summary when done.
- Lead Integrator owns final API naming and dependency boundaries.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass,
  update the plan checkboxes, then commit. Do not proceed to the next phase without completing the
  milestone gate.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- The workspace grep for `Field { .. }` literals in Phase 1 is a checklist, not a guaranteed
  exhaustive list; `cargo build --workspace` is the actual completeness check.
- No `Default` impl exists for `Field`, so every struct literal must be touched explicitly; this is
  intentional (matches existing style) and not something this plan changes.
- `field_group_to_property`'s delegation to `field_to_property` means no separate implementation
  work is needed for RFC-007 groups — only verification that no test breaks.
- `FieldJson`'s duplication between `json_store.rs` and `store.rs` (identified in plan review) is
  left as-is structurally — this plan keeps both copies in sync manually (Phase 1b) rather than
  extracting a shared mapper, which is tracked as a separate follow-up (`srs-rust#450`, parented
  under `muDemocracy.org#105`) to avoid scope creep on this small change.
