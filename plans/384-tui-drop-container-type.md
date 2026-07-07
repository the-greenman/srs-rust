# Plan: srs-gov TUI: migrate section resolution off deprecated containerType

## Summary

`tui_data.rs` contains a fallback path (`sections_from_container_list`) that calls `container list` and matches containers via the RFC-009-deprecated `containerType` string hint + `governance::match_container`. The primary path (`sections_from_navigation`) already loads from `srs repo navigation`, but it reads `section["containerId"]` — a field that does not exist in the `NavigationNode` payload (the correct field is `sectionContainerId`). Because `containerId` is always absent, every `SectionItem` is built with `container_id: None`, making the TUI unable to load section views. The fallback then fires and uses the deprecated path. The fix corrects the field name, switches key resolution to `by_root_type` (matching the same UUID chain that `cmd_top`/`resolve_container_id` use in `main.rs`), and removes the now-dead `sections_from_container_list` path along with the `ContainerTypeDef::container_type` field and `match_container` function retained only for it.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude Code (this session) |
| srs-gov Worker | Claude Code (this session) |
| Verification | Claude Code (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements the RFC-009 section resolution contract already used by `cmd_top` and `resolve_container_id` in `main.rs`. The TUI path is the last caller of `containerType` matching; removing it completes Gate B of epic #262.

| ADR | Decision | Status |
|---|---|---|
| ADR-001 | `srs-gov` is a leaf client that shells out to `srs`; all navigation resolution uses the standard `srs repo navigation` payload | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command outputs change. `srs-gov` is a standalone binary with its own text rendering; it does not use `srs-cli`'s payload contract. No changes to `payload.rs` or golden schemas.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No schema sync needed.

---

## Scope

- Fix `sections_from_navigation` in `tui_data.rs` to read `section["sectionContainerId"]` (correct field) instead of `section["containerId"]` (absent field).
- Switch key resolution in `sections_from_navigation` to use `by_root_type(typeNamespace, typeName)` instead of `governance_key_for_label(displayLabel)`, matching the same approach used by `cmd_top`.
- Delete `sections_from_container_list` from `tui_data.rs` (now dead code).
- Remove the `match_container` import from `tui_data.rs`.
- Remove `ContainerTypeDef::container_type` field and `governance::match_container` function from `governance.rs`, along with their tests.
- Update the module-level doc comment in `governance.rs` to reflect that the `containerType` compat note is no longer needed.

**Out of scope:**

- Any other TUI rendering changes or bug fixes.
- The `GOVERNANCE_CONTAINERS` registry itself (only the deprecated fields/functions are removed).
- Updating `srs-web` or other consumers.
- Addressing why navigation might return empty sections on pre-RFC-013 repos (a diagnostic is already emitted by the service for that case).

---

## Phases

### Phase 1: Fix `sections_from_navigation` and delete the container-list fallback

**Goal:** `tui_data.rs` resolves sections entirely from the `srs repo navigation` payload using `sectionContainerId` and `by_root_type`; `sections_from_container_list` is deleted; `match_container` is no longer imported.

**Agent:** srs-gov Worker

#### Tasks

- [ ] In `tui_data.rs:5`, remove `match_container` from the import: change `use crate::governance::{match_container, GOVERNANCE_CONTAINERS};` to `use crate::governance::{by_root_type, GOVERNANCE_CONTAINERS};`.
- [ ] Rewrite `sections_from_navigation` (lines 59–76) to:
  1. Iterate `payload["navigation"]["sections"]` as before.
  2. For each section, read `type_ns = section["typeNamespace"].as_str().unwrap_or("")` and `type_name = section["typeName"].as_str().unwrap_or("")`.
  3. Call `by_root_type(type_ns, type_name)` — if `None`, skip the section (unknown type, not a governed container).
  4. Read `container_id = section["sectionContainerId"].as_str().map(String::from)`.
  5. Build `SectionItem { key: def.key.to_string(), label: def.label.to_string(), container_id }`.
- [ ] Delete the `sections_from_container_list` function (lines 78–101) in its entirety.
- [ ] In `load_app_state` (lines 9–31), remove the fallback block `if sections.is_empty() { let fallback = sections_from_container_list(repo)?; ... }`.
- [ ] Remove the `run_srs` import from the top of `tui_data.rs` only if it becomes unused after the deletion (it is still used in `load_section_view` etc. — leave it).
- [ ] Update the existing test `sections_from_navigation_maps_labels_to_governance_keys` to pass `typeNamespace`/`typeName`/`sectionContainerId` fields instead of `displayLabel`/`containerId`, confirming the new logic.

#### Acceptance Criteria

- [ ] `sections_from_navigation` reads `sectionContainerId` and uses `by_root_type` for key/label lookup.
- [ ] `sections_from_container_list` no longer exists in `tui_data.rs`.
- [ ] `match_container` is not imported in `tui_data.rs`.
- [ ] Existing test `sections_from_navigation_maps_labels_to_governance_keys` passes (updated to new payload shape).
- [ ] No reference to `containerType` remains in `tui_data.rs`.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `sections_from_navigation_maps_labels_to_governance_keys` — updated to pass `typeNamespace`/`typeName`/`sectionContainerId` and confirm `key` and `container_id` resolve correctly via `by_root_type`.

#### Milestone gate

1. Verify all acceptance criteria above.
2. Confirm the test exists and passes.
3. Run:
```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```
4. Mark task checkboxes `[x]` and commit.

---

### Phase 2: Remove `ContainerTypeDef::container_type` and `match_container` from `governance.rs`

**Goal:** `governance.rs` contains no `containerType` field and no `match_container` function; the module doc comment reflects the completed migration.

**Agent:** srs-gov Worker

#### Tasks

- [ ] In `governance.rs`, remove the `container_type: &'static str` field from `ContainerTypeDef` (line 25).
- [ ] Remove the `container_type: "decision_log"` entry from the `GOVERNANCE_CONTAINERS` static (line 42).
- [ ] Delete the `match_container` function (lines 67–86).
- [ ] Delete the `decision_log_container_matches_by_type` test in `governance.rs` tests (lines 106–117) — it tests `match_container` which is gone.
- [ ] Update the module-level doc comment: remove the paragraph that says "`container_type` field is retained only for the TUI path … it will be removed when the TUI migrates (epic #262)."

#### Acceptance Criteria

- [ ] `ContainerTypeDef` has no `container_type` field.
- [ ] `match_container` does not exist in `governance.rs`.
- [ ] No test references `match_container`.
- [ ] Module doc comment no longer mentions the TUI compat caveat.
- [ ] `GOVERNANCE_CONTAINERS` compiles without the removed field.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `by_root_type_finds_decision_log` — unchanged, still passes.
- `by_root_type_returns_none_for_unknown` — unchanged, still passes.
- `decision_log_container_matches_by_type` — deleted (tests removed function).

#### Milestone gate

1. Verify all acceptance criteria.
2. Run:
```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```
3. Mark checkboxes `[x]` and commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test -p srs-gov` passes
- [ ] No reference to `containerType`, `match_container`, or `container_type` field remains in `crates/srs-gov/`
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] TUI `--smoke` test passes: `cargo run -p srs-gov --bin srs-gov -- --repo <path> tui --smoke`

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.
- **At the end of each phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- `srs repo navigation` always returns `sectionContainerId` for sections that have a rooted governance container. Sections without `sectionContainerId` (e.g. records whose container hasn't been set) will be present in the list but unusable — `load_section_view` already handles `container_id: None` by returning an empty `SectionViewData`.
- Pre-RFC-013 repos (no `manifest.container`) already produce an empty navigation section list with a diagnostic; that case is handled by `load_app_state` showing an empty section list (no change from current behaviour once the fallback is removed).
