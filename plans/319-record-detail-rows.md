# Plan: Factor resolve-hit-set into find_query (srs-rust#319)

## Summary

Issue #319 identified two duplications in `srs-gov`:
1. `render::record_detail` and `tui_data::detail_rows` — **already resolved**: `render.rs` (commit d016508) now delegates to `tui_data::detail_rows`, which is the single canonical implementation.
2. The members∩hits composition logic (`build_find_args` → `run_srs` → `parse_hit_ids` → `Option<HashSet<String>>`) is written identically in `tui_data::allowed_hits` and inline in `main.rs::cmd_list`. This plan resolves item 2 only.

The fix: add `find_query::resolve_hit_set` — a `pub(crate)` function that wraps the build-run-parse sequence — and call it from both sites. `tui_data::allowed_hits` is deleted; `cmd_list`'s inline equivalent is replaced. No cross-crate changes, no new CLI commands, no new payload structs.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | claude |
| CLI/Gov Worker | claude |
| Verification | claude |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new ADRs are required. This plan is a within-crate dedup. The existing architectural rule that governs it: `build_find_args` and `parse_hit_ids` already live in `find_query.rs` specifically because that module is shared by both call sites (its own docstring says so). Adding `resolve_hit_set` completes that design.

If the stretch goal (promote `detail_rows` to `srs-repository`) is pursued in a future plan, ADR-010 would govern it: typed/structured projections belong in the service layer.

| ADR | Decision | Status |
|---|---|---|
| [ADR-010](../docs/adr/010-service-boundary-contract.md) | `resolve_hit_set` stays within `srs-gov` (binary crate, not a shared service); the `detail_rows` promotion to `srs-repository` is deferred pending design decision | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No new commands, no changed payload structs. No action required; golden schemas stay as-is.

### Entity schema sync (check-schema-sync.sh)

No schema files changed. No action required.

---

## Scope

- Add `pub(crate) fn resolve_hit_set(repo, container_id, excludes: &[&str], search: Option<&str>, tags: &[String]) -> Result<Option<HashSet<String>>>` to `crates/srs-gov/src/find_query.rs`.
- Delete `fn allowed_hits` from `crates/srs-gov/src/tui_data.rs`.
- Replace the inline members∩hits logic in `main.rs::cmd_list` (lines ~267–303) with a call to `find_query::resolve_hit_set`.
- Replace the call to `allowed_hits` in `tui_data::load_section_view` with a call to `find_query::resolve_hit_set`.
- All `srs-gov` tests pass; CLI `list` and TUI section-loading behaviour identical to before.

**Out of scope:**

- Promoting `detail_rows` / `DetailRow` to a `srs-repository` service (stretch goal — separate design decision, see below).
- Any cross-crate changes (`srs-repository`, `srs-bindings`, `srs-cli`).
- New WASM bindings.
- Changes to any CLI payload contract.

---

## Phases

### Phase 1: Add `resolve_hit_set` to `find_query.rs`

**Goal:** `find_query.rs` exports a single function that owns the full build-run-parse sequence, so callers only need to supply their filters.

**Agent:** CLI/Gov Worker

#### Tasks

- [x] In `crates/srs-gov/src/find_query.rs`:
  - Add imports at top of file:
    ```rust
    use anyhow::Result;
    use std::collections::HashSet;
    ```
  - Add this function after `parse_hit_ids`:
    ```rust
    /// Run a scoped `srs find` query and return the matching instance IDs as a set,
    /// or `None` when no filter is active (caller should show all members verbatim).
    pub(crate) fn resolve_hit_set(
        repo: &str,
        container_id: &str,
        excludes: &[&str],
        search: Option<&str>,
        tags: &[String],
    ) -> Result<Option<HashSet<String>>> {
        let need_find = !excludes.is_empty() || search.is_some() || !tags.is_empty();
        if !need_find {
            return Ok(None);
        }
        let args = build_find_args(container_id, excludes, search, tags);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let payload = crate::srs::run_srs(&arg_refs, repo, false, false)?;
        Ok(Some(parse_hit_ids(&payload).into_iter().collect()))
    }
    ```

#### Acceptance Criteria

- [x] `find_query::resolve_hit_set` compiles and is accessible as `pub(crate)`.
- [x] Existing tests in `find_query.rs` still pass.
- [x] A test `resolve_hit_set_returns_none_when_no_filter_active` added that asserts `resolve_hit_set(".", "c-1", &[], None, &[])` returns `Ok(None)` without invoking `srs` (pure logic path).

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `resolve_hit_set_returns_none_when_no_filter_active` — proves the early-exit path works without spawning a process.

#### Milestone gate

1. Check all acceptance criteria above.
2. Run:
   ```bash
   cargo test -p srs-gov
   cargo clippy -p srs-gov -- -D warnings
   ```
3. Mark checkboxes `[x]`, commit:
   ```bash
   git add crates/srs-gov/src/find_query.rs
   git commit -m "refactor(srs-gov): add find_query::resolve_hit_set (#319)"
   ```

---

### Phase 2: Replace both call sites

**Goal:** `allowed_hits` in `tui_data.rs` is deleted; `cmd_list` no longer has inline find-set logic; both call `find_query::resolve_hit_set`.

**Agent:** CLI/Gov Worker

#### Tasks

- [ ] In `crates/srs-gov/src/tui_data.rs`:
  - Delete the entire `fn allowed_hits(...)` function (lines 168–190).
  - Replace the call site in `load_section_view` at line 117:
    ```rust
    // Before:
    let allowed = allowed_hits(repo, container_id, search_query, &excludes)?;

    // After:
    let search = (!search_query.is_empty()).then_some(search_query);
    let exclude_refs: Vec<&str> = excludes.iter().map(String::as_str).collect();
    let allowed = crate::find_query::resolve_hit_set(repo, container_id, &exclude_refs, search, &[])?;
    ```
  - Remove unused `HashSet` import if it was only used by `allowed_hits`.

- [ ] In `crates/srs-gov/src/main.rs`, replace the inline find-set logic in `cmd_list` (approximately lines 267–303):
  ```rust
  // Remove: need_find, find_args, and the `if need_find { ... } else { None }` block.
  // Replace with:
  let allowed = find_query::resolve_hit_set(repo, &container_id, &effective_excludes, search, tags)?;
  ```
  - Remove any now-unused `run_srs` import at the top of the inline block (but keep the top-level `use srs::run_srs` if used elsewhere in the file — it is used in `cmd_top`, `cmd_get`, etc., so it stays).

#### Acceptance Criteria

- [ ] `fn allowed_hits` no longer exists anywhere in `tui_data.rs`.
- [ ] The inline `need_find`/`find_args`/`if need_find` block no longer exists in `cmd_list`.
- [ ] Both call sites compile and call `find_query::resolve_hit_set`.
- [ ] `cargo test -p srs-gov` passes.
- [ ] `cargo clippy -p srs-gov -- -D warnings` passes (no dead-code warnings from removed `HashSet` import).

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `record_item_reads_presentation_fields_without_type_specific_rules` (existing) — proves TUI data loading still works.
- `detail_rows_order_and_match_values_by_field_id` (existing) — proves row shaping is unaffected.
- `build_find_args_*` and `parse_hit_ids_*` (existing in `find_query.rs`) — regression guard.

#### Milestone gate

1. Check all acceptance criteria above.
2. Run:
   ```bash
   cargo test -p srs-gov
   cargo clippy -p srs-gov -- -D warnings
   ```
3. Mark checkboxes `[x]`, commit:
   ```bash
   git add crates/srs-gov/src/tui_data.rs crates/srs-gov/src/main.rs
   git commit -m "refactor(srs-gov): replace allowed_hits + cmd_list inline with resolve_hit_set (#319)"
   ```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `fn allowed_hits` is deleted; no inline find-set logic remains in `cmd_list`
- [ ] `find_query::resolve_hit_set` is the single implementation of the build-run-parse sequence
- [ ] CLI `list` output format is unchanged (existing tests pass)
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)

## Coordination Rules

- All changes are within `crates/srs-gov/`. No other crate is touched.
- Do not change the public behaviour of `build_find_args` or `parse_hit_ids`.
- The TUI section-loading and CLI `list` filtering must remain behaviour-identical to the pre-refactor state.

## Assumptions

- The `srs-gov` binary crate does not constrain I/O to any module — mixing I/O and arg-building in `find_query.rs` is acceptable (no purity concern in a binary crate).
- The stretch goal (promoting `detail_rows` to `srs-repository`) is deferred — a separate design decision must be made before a follow-up plan can be written.
