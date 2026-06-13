# Plan: Add targets[] to theme list/get payload (#122)

## Summary

`srs theme list` and `srs theme get` return id, name, namespace, version, description but not `targets`. Callers cannot discover which render formats a theme supports without loading the full Theme via a separate get, or parsing the full Theme JSON. The fix is to add `targets: Vec<String>` to `ThemeSummary` (used by both list and get-summary paths) and regenerate the golden payload schemas.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| Verification | — |

## Architecture Decisions

No new architectural decisions — this plan extends an existing summary struct per the pattern already established for `source_package`.

---

## Contracts

### CLI output contract (ADR-011)

`ThemeListPayload` uses `Vec<ThemeSummary>` via `#[schemars(with = "Vec<serde_json::Value>")]`. Adding `targets` to `ThemeSummary` changes the serialised shape, so:
- Update `ThemeSummary` in `crates/srs-repository/src/theme_service.rs`
- Update the mapping in `list_themes_with_provenance` to populate `targets`
- Run `cargo run --bin generate-schemas`
- Commit updated `crates/srs-cli/schemas/payload/theme-list.json` and `theme-get.json`

`ThemePayload` (used by `theme get`) embeds the full `Theme` struct which already carries `targets` — no change needed there.

### Entity schema sync

No entity schema changes.

---

## Scope

- Add `targets: Vec<String>` to `ThemeSummary` struct in `crates/srs-repository/src/theme_service.rs`
- Populate `targets` in `list_themes_with_provenance` mapping
- Regenerate `schemas/payload/`

**Out of scope:** `theme create`/`update` input shapes; `ThemePayload` (already carries full Theme with targets).

---

## Phases

### Phase 1: Add targets to ThemeSummary

**Goal:** `srs theme list` and summary paths include `targets` in every ThemeSummary entry.

**Agent:** Repository Service Worker

#### Tasks

- [x] Add `pub targets: Vec<String>` to `ThemeSummary` in `theme_service.rs`
- [x] Populate `targets: t.targets.clone()` in the `list_themes_with_provenance` mapping
- [x] Run `cargo run --bin generate-schemas` and stage updated schema files
- [x] Confirm `cargo test --test payload_contracts` passes

#### Acceptance Criteria

- [x] `ThemeSummary` carries `targets` field
- [x] `srs theme list` output includes `targets` array for each theme
- [x] `cargo test --test payload_contracts` passes

#### Milestone gate

```bash
cargo test -p srs-repository
cargo test -p srs-cli
cargo test --test payload_contracts
cargo clippy -- -D warnings
```

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test --test payload_contracts` passes
- [ ] `srs theme list` output includes `targets` field in each theme entry

## Assumptions

- `targets` on Theme is always populated (validated non-empty in `validate_theme`).
