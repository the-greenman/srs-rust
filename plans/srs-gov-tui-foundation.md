# TDD Agentic Plan: `srs-gov` Interactive Governance CLI

## Agentic Execution Model

Recommended model roles:
- Lead implementation agent: GPT-5 Codex. Owns repo inspection, test-first edits, implementation, verification, and section checkpoints.
- Architecture reviewer: GPT-5. Reviews service boundaries, capability-layering decisions, CLI contracts, and TUI architecture before larger green/refactor steps.
- Fast checker: GPT-5 mini or equivalent. Handles narrow review tasks such as grep audits, docs drift checks, test inventory, and acceptance checklist validation.

Execution pattern:
- Keep one lead agent in control of the worktree to avoid conflicting edits.
- Use reviewer agents for notes/checklists unless explicitly asking them to produce patches.
- For each section: Red -> Review -> Green -> Refactor -> Acceptance.
- Stop after each section with a concise status and acceptance checklist.
- Never delegate core Rust service or TUI implementation to a lightweight model.

Bug handling:
- Fix bugs immediately only when they block the active section or corrupt the contract being implemented.
- Log proper non-blocking bugs in the Bug Log with reproduction/context and keep the section scope intact.
- Do not bury unrelated fixes inside feature commits.

## Bug Log

- `srs-gov` top-level container counts were always zero because `ContainerSummary`
  does not include `memberInstanceIds`; fixed by reading full containers for counts.
- `srs-gov` `textwrap` sliced strings by byte offset and could panic on multibyte
  UTF-8; fixed with char-boundary-aware wrapping.
- `repository_navigation` included the identity record as a section when
  `identityInstanceId` was also a root-container member; fixed in Section 2 hardening.
- `srs-gov` had inconsistent document-container disambiguation between top-level
  display and `resolve_container_id`; fixed by making top-level matching title-aware.
- `srs-gov create` dry-run JSON was assembled by string interpolation and broke on
  quotes; fixed by constructing the heredoc body with `serde_json`.
- `srs-gov` rendering had unguarded 8-byte ID slices that could panic on short IDs;
  fixed with guarded short-id rendering.
- `repository_navigation` hand-rolled `precedes` ordering instead of reusing the
  canonical relation graph helper; fixed in Section 2 hardening.
- `repository_navigation` did per-section container scans; fixed by hoisting a single
  container scan into a root-to-container map. Record lookup batching remains a future
  store-level optimization if profiling calls for it.
- `repository_navigation` had a test-only field-id fallback around display labels;
  fixed by relying on `record_label::record_display_label` and updating fixtures to
  use package field metadata.

## 0. Bootstrap Worktree + Plan File

Goal: create a dedicated implementation worktree and write this plan into it before touching code.

Implementation steps:
- From `/home/greenman/dev/semanticops/srs-rust`, create:
  - worktree: `/home/greenman/dev/semanticops/.worktrees/srs-gov-tui-foundation`
  - branch: `feat/srs-gov-tui-foundation`
- Write the plan to:
  - `/home/greenman/dev/semanticops/.worktrees/srs-gov-tui-foundation/plans/srs-gov-tui-foundation.md`
- All future code, docs, and test edits happen only inside that worktree.

Acceptance criteria:
- `git worktree list` shows `srs-gov-tui-foundation`.
- `plans/srs-gov-tui-foundation.md` exists in that worktree.
- `git status --short` outside the new worktree is unchanged.

## 1. Characterize Existing `srs-gov`

TDD loop:
- Red: add/confirm tests capturing current `srs-gov` behavior for no-arg, `list`, `get`, `repo-create`, `--json`, and `--explain`.
- Green: make no product changes yet; tests should pass against existing behavior.
- Refactor: isolate test helpers only if needed.

Acceptance criteria:
- Existing `crates/srs-gov/tests/flow.rs` still passes.
- New tests document the current command contract.
- No TUI dependencies or feature code added yet.

## 2. Structural Navigation Contract

TDD loop:
- Red: add service/CLI tests for `repository_navigation` behavior expected by Gate B.
- Green: implement or consume `srs-repository` navigation service:
  - identity/root node
  - precedes-ordered section nodes
  - enough section data to resolve each section container
  - display labels resolved in Rust
- Refactor: keep CLI handlers thin and move semantics into `srs-repository`.

Acceptance criteria:
- `srs repo navigation --repo X --format json` returns deterministic identity + ordered sections.
- Memory/file-store coverage proves the service is store-agnostic.
- No navigation logic depends on `containerType` string matching.

Status:
- Implemented `srs-repository::repository_navigation_service` and `srs repo navigation`.
- Hardened missing `manifest.container` to return an empty navigation payload with a diagnostic for pre-RFC-013 repositories.
- Excluded `identityInstanceId` from navigable sections even when it is also a root-container member.
- Replaced local `precedes` sorting with the canonical relation graph ordering helper.
- Verified with `cargo test -p srs-repository`, `cargo test -p srs repo_navigation`, and `cargo test -p srs-gov`.

## 3. Decision-log list = authored view + discovery query

A decision-log list is an **authored SRS view** attached to the decision-log
container (the container *is* the decision log; the DocumentView/View describes
list shape, visible columns, default sort, default-hidden states, searchable
fields) composed with **generic SRS query rules** applied at runtime (lifecycle
include/exclude, tag filter, content search, sort). It is NOT a governance-specific
or bespoke Rust "list projection".

Superseded approach (removed): a prior pass added
`srs-repository::record_projection_service` with `RecordProjectionInput` /
`RecordProjection` / `ProjectedRecord`. Even after being made package-agnostic it
**bypassed the authored DocumentView/View model** and re-expressed visible fields,
hidden states, searchable fields, tag filters, and sort order in a separate input
struct — a shortcut around the view/query system, and a bespoke precursor of the
already-tracked discovery service (#216). It has been deleted (module + `lib.rs`
line removed). Its test scenarios are preserved as the **discovery acceptance
checklist** below.

Staged exactly per the layering recommendation: (a) consume `resolve-view`;
(b) fill the remaining gaps with **generic** discovery features; (c) only then
wire srs-gov.

### 3a. Gap analysis (consume `resolve-view` first)

| List semantic | Covered today by | Gap / action |
|---|---|---|
| Visible columns | `resolve-view` `columns` (L1 View `field_views`) | none — consume |
| Ordered members | `resolve-view` `members` (roots-first dedup) | none — consume |
| Hide `superseded`/`closed` by default | `exclude_lifecycle_states` on **TypeQuery** only; the `decision-log` view uses a **container-subset** source | author default exclusion in view metadata; discovery applies lifecycle filtering generically |
| Show-all toggle (runtime) | nothing (authored filters are static) | `discovery_service::find` runtime query overrides the authored default |
| Topic/tag filter | nothing | `DiscoveryQuery.tag` axis |
| Search over title + decision_statement | nothing | `project_text` + `DiscoveryQuery.text`/`fields`; searchable fields declared in view metadata |
| Newest/oldest sort | `SectionOrdering` field sort; `createdAt` is record metadata, no runtime toggle | default sort in view metadata; runtime sort via discovery/CLI |

Note: decision "status" is a **lifecycle state**, not a field
(`draft → proposed → ratified → closed|superseded`). `abandoned` is client-invented
and not in the package. So "hide by default" is lifecycle-state exclusion — generic.

### 3b. Generic feature work (the `ext:discovery` contract — epic #212)

Implement the spec'd Discovery Contract in dependency order; reuse, don't duplicate:
- **ADR-018** (#214): `docs/adr/018-discovery-service.md`, status `proposed` — service/struct/command surface + deferred `DiscoveryIndex` (no `async`).
- **`project_text` / `TextSegment`** (#215): `ValueType`-driven searchable split (`String|Text|Url|Select|Multiselect` searchable; `Number|Boolean|Date` not), tags + display label included, NFC+lowercase normalization, deterministic ordering. Reuse the `record_label::build_field_name_index` pattern for a `field_id → Field` map.
- **`discovery_service::find`** (#216): `find(store, DiscoveryQuery) -> DiscoveryResult` composing `record_store::list_records_filtered` (`RecordListFilter`) + `project_text` + case-insensitive substring match (recall floor, `score: None` at Layer 1). Typed in/out (ADR-010); `record_label` for hit labels.
- **`srs find` CLI** (#217): handler = parse → one `find` call → `output::ok`; `FindPayload` in `payload.rs`; regenerate golden schema.

### 3c. Authored list defaults in DocumentView/View metadata

Add additive, optional metadata so the governance `decision-log` view carries:
searchable fields (title + `decision_statement`), default-excluded lifecycle
states (`superseded`, `closed`), default sort (newest-first). Canonical schema in
`srs/docs/schema/2.0/`, mirrored to srs-rust + srs-vscode (mirror PRs merge before
the `srs` spec PR). Governance-specific *values* stay in package data;
`srs-repository` only reads them generically. Fold into RFC #213's scope.

TDD loop:
- Red: port the discovery acceptance checklist (below) as `discovery_service` tests, plus a cross-store roundtrip (memory → json → file) matching a non-title field the old web filter missed.
- Green: implement `project_text` then `find`; wire `srs find`.
- Refactor: keep governance specifics in the authored view + thin srs-gov adapter config, never in `srs-repository`.

Acceptance criteria:
- `find` output typed, serde-friendly, consumed identically by CLI/WASM later.
- Discovery acceptance checklist reproduces through `resolve-view` + `find`.
- Client code owns presentation only.

**Discovery acceptance checklist** (lifted from the removed projection tests; "hidden status" now maps to lifecycle-state exclusion):
- hide configured lifecycle states by default (`superseded`, `closed`)
- show-all toggle includes the hidden states
- tag filter narrows results; available tags are enumerable
- case-insensitive substring search over declared searchable fields (title + `decision_statement`), including a match on a non-title field
- newest/oldest sort by created metadata, stable tiebreak on instance id
- store-backed path filters by type namespace/name + tag and resolves field metadata

Status:
- Removed `srs-repository::record_projection_service` (module + `lib.rs` line).
- Landed the generic discovery foundation (epic #212):
  - ADR-019 `docs/adr/019-discovery-service.md` (018 was taken; #214's "018" is stale).
  - `text_projection::{project_text, TextSegment, build_field_text_index, normalize}`
    (#215) — `ValueType`-driven searchable split, entries + group_values, label/tag
    sentinels, NFC+lowercase at match time. 7 unit tests.
  - `discovery_service::find` (#216) — `DiscoveryQuery`/`DiscoveryResult`/`DiscoveryHit`
    per `discovery.json` (not the stale #216 draft struct). 8 tests incl. a
    memory → file cross-store roundtrip matching a non-title field.
  - `srs find` CLI (#217) — `FindPayload` + golden schema; verified end-to-end
    against `srs/srs` (content recall over non-title fields, structured filters,
    tier-0 deferral diagnostic).
  - Added workspace dep `unicode-normalization`.
- Verified: `cargo clippy -p srs-repository -p srs -- -D warnings` clean; full
  workspace `cargo test` green (~1098 tests); `payload_contracts` green.
- No `srs-web` files mutated; web migration tracked on epic #212 (#219).

Remaining (staged next increment, gated on cross-repo coordination):
- 3c authored-defaults metadata is an **additive schema change in the external
  `srs/` spec repo** (`docs/schema/2.0/`) under RFC #213, with the srs-rust +
  srs-vscode mirror PRs merging before the spec PR. Not a srs-rust-local edit.
- Section 4 wiring of `srs-gov list`/TUI to compose `resolve-view` + `find` depends
  on 3c for the authored default-hidden states / searchable fields, so it follows.

## 4. Scriptable CLI Gate B Update

TDD loop:
- Red: update `srs-gov` command tests for structural behavior.
- Green:
  - no-arg `srs-gov` renders root identity + ordered sections
  - `list <section>` composes structural navigation + `resolve-view` (columns + ordered members + authored defaults) with `find` (runtime text/tag/lifecycle/sort); interactive controls map straight onto `DiscoveryQuery` — search → `text`/`fields`, tag facet → `tag`, show-all → drop the authored lifecycle exclusion, sort toggle → sort param
  - `get <id>` remains human-readable
- Refactor: remove hardcoded governance container matching as a join mechanism; governance specifics (`decision_statement`, default-hidden states, searchable fields) come from the authored view + thin srs-gov adapter config, never `srs-repository`.

Acceptance criteria:
- `srs-gov` can explore the governance repo without `containerType`/title heuristics.
- `--json` and `--explain` remain useful and non-interactive.
- `rg "containerType"` confirms it is not driving governance navigation logic.
- A #220-style pass confirms no governance semantics leaked into `srs-repository`.

Status:
- Implemented Stage 4's authored-view composition path for `srs-gov list`:
  `container resolve-view` supplies columns, ordered members, and authored
  `excludeLifecycleStates`; `srs find` applies runtime lifecycle exclusion,
  `--search`, and repeated `--tag`; the displayed list is
  `resolve-view.members ∩ find.hits` in resolve-view order.
- Added `srs-gov list` runtime flags: `--all`, `--search <text>`, and repeatable
  `--tag <tag>`.
- Kept `--json` non-interactive by printing the raw `container resolve-view`
  envelope; `--explain` prints both composed commands when runtime filtering is
  active.
- Added self-contained `srs-gov` integration coverage that creates a temporary
  governance `.srsj`, adds draft/ratified/superseded/closed decisions, and proves:
  default hidden states, `--all`, non-title content search, and tag filtering.
- Documented the flow in `docs/dogfooding.md` S15 and ADR-020.
- Remaining follow-up: remove the older `containerType`/title container-resolution
  heuristic from `srs-gov` entirely by using structural navigation for `cmd_top`,
  `list`, `get`, and `create`. Stage 4 behavior is complete, but the final
  heuristic-removal acceptance item is carried forward as a focused cleanup before
  Stage 5.

## 5. TUI Foundation

Status: complete.

Implementation notes:
- Selected `ratatui` + `crossterm` for the interactive terminal foundation.
- Added a pure `AppState`/reducer layer, a first-frame renderer, data loading through generic `repo navigation`, `container resolve-view`, and `find`, plus `srs-gov tui --smoke`.
- The TUI is read-only in this foundation pass.
- Verified with `cargo test -p srs-gov`, `cargo build -p srs-gov`, and direct dogfood:
  - `SRS_BIN=target/debug/srs target/debug/srs-gov --repo /home/greenman/dev/semanticops/srs/docs/spec/examples/gallery-project-v2 tui --smoke`
  - `SRS_BIN=target/debug/srs target/debug/srs-gov --repo /tmp/srs-gov-tui-dogfood-20260628.srsj tui --smoke`

TDD loop:
- Red: add a non-interactive smoke test for TUI state initialization and first-frame render.
- Green:
  - add `ratatui` + `crossterm`
  - add `srs-gov tui`
  - build pure app state separate from terminal I/O
  - render first screen into a test backend
- Refactor: split modules into state, data loading, rendering, input handling.

Acceptance criteria:
- [x] `srs-gov tui --smoke --repo X` exits successfully.
- [x] Smoke test verifies a nonblank first frame.
- [x] Normal `srs-gov tui --repo X` enters terminal UI.
- [x] Terminal cleanup is safe on error path via a drop guard.

## 6. TUI Exploration Interactions

Status: complete.

Implementation notes:
- Added reducer/input coverage for section navigation, record navigation, record detail focus, back/escape behavior, search entry, sort toggle, show-all toggle, and quit.
- Runtime interaction reloads record lists when section/filter/sort state changes.
- Edit/export actions are intentionally absent from the read-only foundation rather than pretending to be active commands.

TDD loop:
- Red: add unit tests for input reducer/state transitions.
- Green implement:
  - section navigation
  - record list navigation
  - select/open record detail
  - back/escape behavior
  - search entry mode
  - sort toggle
  - show-all toggle
  - quit
- Refactor: keep keybindings declarative and easy to extend.

Acceptance criteria:
- [x] User can navigate sections and decisions by keyboard.
- [x] User can select a decision and view detail.
- [x] Search/filter/sort behavior uses the same generic `resolve-view` + `find` data path as the scriptable list.
- [x] Edit/export actions are not exposed in the read-only foundation.

## 7. Documentation + Handoff

Status: complete.

Implementation notes:
- Updated `docs/governance-flow.md` to describe the current read-only TUI foundation, the shallow detail pane, and the new keybindings.
- Kept the handoff doc aligned with the current command surface instead of implying full record-field rendering or write actions.

TDD/check loop:
- Red: docs test or grep check for outdated `srs-gov` examples if practical.
- Green:
  - update `docs/governance-flow.md`
  - document TUI keybindings
  - document read-only foundation scope
  - note future edit/export gates
- Refactor: keep docs short and command-focused.

Acceptance criteria:
- [x] Plan file remains in `plans/`.
- [x] Governance flow docs mention structural navigation and `srs-gov tui`.
- [x] Test suite passes for affected crates.
- [x] Final status includes commands run and any blocked network/dependency steps.

## Agent Rules

- Work only in `/home/greenman/dev/semanticops/.worktrees/srs-gov-tui-foundation`.
- Write failing tests before implementation for each section.
- Do not mutate `srs-web` unless a Rust contract requires a matching fixture or documentation update; if needed, use a separate web worktree.
- Keep service semantics in `srs-repository`; `srs-gov` and TUI code render and route only.
- Stop after each section with a concise status and acceptance checklist.
