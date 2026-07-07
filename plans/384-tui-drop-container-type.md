# Plan: srs-gov TUI: migrate section resolution off deprecated containerType

## Summary

`tui_data.rs` contains a fallback path (`sections_from_container_list`) that calls `container list` and matches containers via the RFC-009-deprecated `containerType` string hint + `governance::match_container`. The primary path (`sections_from_navigation`) already loads from `srs repo navigation`, but it reads `section["containerId"]` — a field that does not exist in the `NavigationNode` payload (the correct field is `sectionContainerId`). Because `containerId` is always absent, every `SectionItem` is built with `container_id: None`, making the TUI unable to load section views. The fallback then fires and uses the deprecated path. The fix corrects the field name, switches key resolution to `by_root_type` (matching the same UUID chain that `cmd_top`/`resolve_container_id` use in `main.rs`), and removes all deprecated code in one pass: `sections_from_container_list`, `governance_key_for_label`, `ContainerTypeDef::container_type`, and `governance::match_container`.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude Code (this session) |
| srs-gov Worker | Claude Code (this session) |
| Verification | Claude Code (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. No ADR governs `srs-gov`'s established shell-out design (it is documented in CLAUDE.md). This plan removes the last caller of the RFC-009-deprecated `containerType` path, completing Gate B of epic #262.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command outputs change. `srs-gov` is a standalone binary with its own text rendering; it does not use `srs-cli`'s payload contract. No changes to `payload.rs` or golden schemas.

### Entity schema sync (check-schema-sync.sh)

No changes to JSON Schema files under `srs/docs/schema/2.0/`. No schema sync needed.

---

## Scope

- Fix `sections_from_navigation` in `tui_data.rs`: use `filter_map` over `payload["navigation"]["sections"]`, resolving each entry with `by_root_type(typeNamespace, typeName)` and reading `sectionContainerId` for the container ID.
- Delete `governance_key_for_label` from `tui_data.rs` (only caller was `sections_from_navigation`).
- Delete `sections_from_container_list` from `tui_data.rs` and its call-site in `load_app_state`.
- Change the import in `tui_data.rs` to `use crate::governance::by_root_type;` (remove `match_container` and `GOVERNANCE_CONTAINERS`).
- Remove `ContainerTypeDef::container_type` field and `governance::match_container` function from `governance.rs`, along with the `decision_log_container_matches_by_type` test.
- Update the module-level doc comment in `governance.rs` to reflect the completed migration.

**Out of scope:**

- Any other TUI rendering changes or bug fixes.
- The `GOVERNANCE_CONTAINERS` registry itself (only deprecated fields/functions are removed).
- Updating `srs-web` or other consumers.
- Addressing pre-RFC-013 repos (no `manifest.container`) — already handled by the navigation service returning empty sections + diagnostic.

---

## Phases

### Phase 1: Remove all deprecated containerType code in one pass

**Goal:** `srs-gov` compiles, tests pass, and `cargo clippy -- -D warnings` is clean with no reference to `containerType`, `match_container`, `container_type` field, or `governance_key_for_label` anywhere in the crate.

Both `tui_data.rs` and `governance.rs` changes are done in a single phase to avoid intermediate dead-code lint failures between commits.

**Agent:** srs-gov Worker

#### Tasks

- [ ] **`tui_data.rs` — fix import (line 5):** Change `use crate::governance::{match_container, GOVERNANCE_CONTAINERS};` to `use crate::governance::by_root_type;`.

- [ ] **`tui_data.rs` — rewrite `sections_from_navigation` (lines 59–76):** Replace the function body with a `filter_map` over the sections array:
  ```rust
  fn sections_from_navigation(payload: &Value) -> Vec<SectionItem> {
      let sections = payload["navigation"]["sections"]
          .as_array()
          .cloned()
          .unwrap_or_default();

      sections
          .iter()
          .filter_map(|section| {
              let type_ns = section["typeNamespace"].as_str().unwrap_or("");
              let type_name = section["typeName"].as_str().unwrap_or("");
              let def = by_root_type(type_ns, type_name)?;
              let container_id = section["sectionContainerId"].as_str().map(String::from);
              Some(SectionItem {
                  key: def.key.to_string(),
                  label: def.label.to_string(),
                  container_id,
              })
          })
          .collect()
  }
  ```

- [ ] **`tui_data.rs` — delete `governance_key_for_label` (lines 329–343):** Remove the private function entirely (its only caller was `sections_from_navigation`).

- [ ] **`tui_data.rs` — delete `sections_from_container_list` (lines 78–101):** Remove the function. In `load_app_state` (lines 9–31), remove the fallback block:
  ```rust
  if sections.is_empty() {
      let fallback = sections_from_container_list(repo)?;
      if !fallback.is_empty() {
          sections = fallback;
          repo_title = "Governance".to_string();
      }
  }
  ```
  Note: `HashSet` is still used in `allowed_hits` and `detail_rows` — retain `use std::collections::HashSet;`.

- [ ] **`tui_data.rs` — update test `sections_from_navigation_maps_labels_to_governance_keys`:** Replace the fixture and assertions with:
  ```rust
  #[test]
  fn sections_from_navigation_maps_labels_to_governance_keys() {
      let payload = serde_json::json!({
          "navigation": {
              "identity": { "displayLabel": "Example" },
              "sections": [
                  {
                      "typeNamespace": "governance",
                      "typeName": "decision_log",
                      "sectionContainerId": "c-1"
                  },
                  {
                      "typeNamespace": "unknown",
                      "typeName": "something_else",
                      "sectionContainerId": "c-2"
                  }
              ]
          }
      });

      let sections = sections_from_navigation(&payload);

      // Only governance-typed sections are included; unknown types are filtered out.
      assert_eq!(sections.len(), 1);
      assert_eq!(sections[0].key, "decision_log");
      assert_eq!(sections[0].container_id.as_deref(), Some("c-1"));
  }
  ```

- [ ] **`governance.rs` — remove `container_type` field (line 25):** Delete `pub container_type: &'static str,` from `ContainerTypeDef`.

- [ ] **`governance.rs` — remove `container_type` entry from static (line 42):** Delete `container_type: "decision_log",` from the `GOVERNANCE_CONTAINERS` initializer.

- [ ] **`governance.rs` — delete `match_container` function (lines 67–86):** Remove the entire function.

- [ ] **`governance.rs` — delete `decision_log_container_matches_by_type` test (lines 106–117):** Remove the test (it tests the now-deleted function).

- [ ] **`governance.rs` — update module doc comment:** Remove the paragraph noting "`container_type` field is retained only for the TUI path … it will be removed when the TUI migrates (epic #262)." Replace with a sentence noting that the RFC-009 migration to `typeNamespace`/`typeName` is now complete.

#### Acceptance Criteria

- [ ] `sections_from_navigation` uses `filter_map` + `by_root_type` + reads `sectionContainerId`.
- [ ] `sections_from_container_list` does not exist in `tui_data.rs`.
- [ ] `governance_key_for_label` does not exist in `tui_data.rs`.
- [ ] Import in `tui_data.rs` is `use crate::governance::by_root_type;` only.
- [ ] `ContainerTypeDef` has no `container_type` field.
- [ ] `match_container` does not exist in `governance.rs`.
- [ ] `decision_log_container_matches_by_type` test does not exist.
- [ ] Test `sections_from_navigation_maps_labels_to_governance_keys` asserts `sections.len() == 1`, `key == "decision_log"`, `container_id == Some("c-1")`, and the unknown-type section is absent.
- [ ] No reference to `containerType`, `match_container`, `container_type` field, or `governance_key_for_label` in `crates/srs-gov/`.
- [ ] `cargo test -p srs-gov` passes.
- [ ] `cargo clippy -p srs-gov -- -D warnings` is clean.

#### Testing

```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```

Specific tests:
- `sections_from_navigation_maps_labels_to_governance_keys` — verifies `filter_map` + `by_root_type` + `sectionContainerId` + unknown-type filtering.
- `by_root_type_finds_decision_log` — unchanged, still passes.
- `by_root_type_returns_none_for_unknown` — unchanged, still passes.
- `record_item_reads_presentation_fields_without_type_specific_rules` — unchanged, still passes.
- `detail_rows_order_and_match_values_by_field_id` — unchanged, still passes.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo test -p srs-gov
cargo clippy -p srs-gov -- -D warnings
```
3. Mark task checkboxes `[x]` and commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test -p srs-gov` passes
- [ ] No reference to `containerType`, `match_container`, `container_type` field, or `governance_key_for_label` in `crates/srs-gov/`
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)
- [ ] TUI smoke test passes: `cargo run -p srs-gov --bin srs-gov -- --repo <governance-repo-path> tui --smoke`

## Coordination Rules

- **At the end of the phase:** verify all acceptance criteria, confirm planned tests exist and pass, update the plan checkboxes, then commit.

## Assumptions

- `srs repo navigation` sections with no matching `by_root_type` entry are silently skipped — correct behaviour (unknown/future governance types should not crash the TUI).
- `sectionContainerId` may be `None` for a section record with no container yet; `load_section_view` already handles `container_id: None` by returning empty `SectionViewData`.
- Pre-RFC-013 repos (no `manifest.container`) produce an empty navigation section list with a diagnostic; removing the `container list` fallback does not regress these repos — the TUI shows an empty section list either way.
