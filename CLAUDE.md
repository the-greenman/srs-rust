# CLAUDE.md — srs-rust

Rust implementation of the SRS system: `srs-core`, `srs-repository`, `srs-cli`, `srs-bindings`, `srs-projection`.

The top-level `semanticops/CLAUDE.md` contains the full SRS data model, CLI reference, and agentic usage rules. Read that first. This file adds rules specific to working inside the Rust codebase.

**Before implementing any new capability** (anything that queries, filters, traverses, validates, or projects), read **`docs/architecture/capability-layering.md`** — the default path for where functionality belongs. The short version: build it once as a `srs-repository` service returning a typed struct, expose it through both the CLI payload and a WASM binding, and keep clients free of semantics. Building semantics in a leaf client (as a prior srs-web search filter did) is the mistake that guide exists to prevent.

## Commands

Run from `srs-rust/`:

```bash
cargo build
cargo test
cargo test -p srs-core
cargo test test_name
cargo clippy -- -D warnings
cargo run --bin srs -- <args>
cargo run --bin generate-schemas          # regenerate payload JSON Schema golden files after changing payload.rs
```

## Crate Authority — what lives where

| Crate | Owns | Hard constraints |
|---|---|---|
| `srs-core` | Canonical Rust types, serde shapes, in-memory validation | No file I/O. No async. No `schemars`. |
| `srs-repository` | Repository loading, writing, package resolution, service functions, archive pack/unpack | Depends on `srs-core`. All business logic lives here, not in the CLI. |
| `srs-cli` | Arg parsing, stdin handling, JSON envelope output | One service call per handler. No business logic. No direct filesystem access in handlers. |
| `srs-bindings` | JSON-first binding surface over repository services | Calls the same services as the CLI. No duplicated logic. |
| `srs-projection` | Rendering and export projections | Placeholder — no work until consumers exist. |

When in doubt about where logic belongs: if it would also be needed by an HTTP API or Python binding, it belongs in `srs-repository`, not `srs-cli`.

## CLI Handler Pattern (ADR-010, ADR-011)

A CLI handler must contain exactly: arg parsing, one `serde_json::from_reader` or flag-to-struct mapping, one service call, `output::ok/err`. Nothing else.

```rust
fn cmd_note_create(ctx: CliContext) -> Result<OutputDTO> {
    let input: CreateNoteInput = serde_json::from_reader(io::stdin())?;
    let result = with_store(&ctx, |store| Ok(note_service::create(store, input)?))?;
    Ok(output::ok("note create", result))
}
```

If a handler exceeds ~15 lines, the excess is almost certainly business logic that belongs in `srs-repository`.

## Payload Contract (ADR-011)

Every CLI command output is a named struct in `crates/srs-cli/src/payload.rs`. No `json!({...})` literals in handlers.

After changing any struct in `payload.rs`:

```bash
cargo run --bin generate-schemas
# commit the updated files in crates/srs-cli/schemas/payload/
```

The pre-commit hook and `cargo test --test payload_contracts` enforce this. A failing golden-file test means the schema files are out of sync with the structs.

## Service Function Contract (ADR-010)

Service functions in `srs-repository` must use:
- **Input:** typed struct (e.g. `CreateNoteInput`) — no `serde_json::Value` parameters
- **Validation:** all validation in the service, not in the CLI handler
- **Output:** typed result struct — no `json!()` construction in the service
- **Filtering:** list functions take a filter struct, not multiple overloaded functions

## Storage Boundary Rules

- `FileStore` owns all file paths. Path strings (`records/`, `.srs/`, `manifest.json`) must not appear in service logic.
- `MemoryStore` is the canonical test double — tests that only work against `FileStore` are testing the adapter, not the service.
- New service features need at least one cross-store roundtrip test (e.g. memory → json → file).
- Do not introduce `async` traits until there is a concrete async consumer.

## Tags in This Codebase

Tags are weak discovery labels. They are not semantic claims, not formal ontology, not hidden command policy. If a command needs a tag set, it belongs in a named profile or explicit input, not hardcoded in command code.

## Working with the Spec Repo

`srs/` is an external SRS repository consumed by the Rust workspace as test data — it is not internal source. Do not embed spec content directly in Rust source or tests. Use fixture copies or vendor the spec repo.

```bash
srs repo validate --repo ../srs/srs        # should be 0 errors
cargo test --test payload_contracts        # golden schema tests
```

## Schema Sync

`crates/srs-schema/schemas/2.0/` is a **mirror** of `srs/docs/schema/2.0/` — never edit schema files there directly. The canonical source is always the `srs/` spec repo. `srs-vscode/schemas/2.0/` is a second mirror with the same constraint.

When schemas change in `srs/docs/schema/2.0/` (e.g. after a spec RFC merges into `srs`):

```bash
# From srs-rust/
scripts/sync-schemas-from-spec.sh          # copies *.json + regenerates SHA256SUMS
# Then sync srs-vscode:
../srs-vscode/scripts/sync-schemas-from-spec.sh
```

Verify everything is in sync before committing:
```bash
bash scripts/check-schema-sync.sh          # checks srs-rust and srs-vscode in one pass
```

**Never regenerate SHA256SUMS manually.** The sync script uses `sha256sum *.json | sort` (sorted by hash digest). The drift check validates with the exact same command — using `sort -k2` or any other variant will cause "SHA256SUMS mismatch" in CI. Always go through `sync-schemas-from-spec.sh`.

Commit in each repo separately:
```bash
# srs-rust
git add crates/srs-schema/schemas/2.0/
git commit -m "chore(schema): sync schemas from spec (RFC-NNN)"

# srs-vscode
cd ../srs-vscode
git add schemas/2.0/
git commit -m "chore(schema): sync schemas from spec (RFC-NNN)"
```

**Multi-repo merge order:** the srs-rust and srs-vscode schema mirror PRs must be merged **before** the corresponding `srs` spec PR — the release-drift CI in `srs` checks that the artifact copies are up to date at HEAD. Open both mirror PRs at the same time as the spec PR.

CI enforces correctness via the `schema-drift` job (`scripts/check-schema-drift.sh ../srs`). If it fails locally, running `sync-schemas-from-spec.sh` will fix it.

## Pre-commit Hook

The hook runs `cargo test --test payload_contracts`. If it fails, regenerate schemas with `cargo run --bin generate-schemas` and stage the updated files before committing.

## Project & priority management

Issues across the ecosystem are tracked on **Project #5 "SRS"** and prioritised **top-down from
user stories**. The authoritative process — the priority model (story MoSCoW → derived
`priority: Pn`), sub-issue linkage, the Status/iteration conventions, and the `gh-project` tool —
is in **[docs/project-management.md](docs/project-management.md)**.

Quick rules:
- **Never hand-set an implementation issue's priority.** It is derived from the user stories it
  serves (as native GitHub sub-issues) by `gh-project rollup`. Humans set **MoSCoW** on stories.
  Engineering work with no story inherits its **epic's Priority one tier down** (epic fallback).
- **File issues linked.** When you file an implementation issue, immediately parent it under the
  story or engineering epic it serves: `gh-project link <parent-repo>#<n> <child-repo>#<n>` (plain
  REST — works in proxy-bound routines too). An issue under no story and no epic is orphaned and
  gets no priority.
- **Bugs** are fixed ASAP — they floor at `priority: P1` even without a story.
- **Unlinked non-bug** work is flagged ("could get lost"), never dropped — link it to a story.
- **Epics are releases.** An `epic` (in muDemocracy.org) *is* a release: a human sets its **Release
  identity + Priority**; every descendant **inherits Release** via `gh-project release-sync` (never
  hand-set on a child). Every story should sit under an epic — `coverage` flags `orphan_stories_no_epic`.
- The tool **self-discovers** the board IDs — never hardcode project/field IDs in a prompt.
- **Explainable estimates:** `gh-project summary` shows all priority estimates with the six
  calculation stages; `gh-project explain <repo> <#>` walks one issue through them.
- Skills: `/triage`, `/stories`, `/roadmap`. Tool source: `scripts/gh-project.mjs`, released as a
  GitHub asset (`gh release download --repo the-greenman/srs-rust --pattern gh-project.mjs`).

## Branch & PR hygiene

Every branch must trace to a GitHub issue, and every PR must link its issue. This is how the ecosystem avoids the recurring failure mode where an issue is marked closed but its fix survives only on an unmerged, abandoned branch.

- **Naming** — human-created branches use `type/<issue#>-slug` (e.g. `feat/242-cross-field-rules`, `docs/432-migrate-identity`). Cloud-agent branches (`claude/<name>-<hash>`) are exempt from the scheme but their PR **must** carry `Closes #N`.
- **Linking** — every PR body includes `Closes #N` (or `Refs #N` if it should not auto-close). No PR without an issue reference. See `.github/pull_request_template.md`.
- **Merged branches auto-delete** — the repo has `deleteBranchOnMerge` enabled; a branch is removed automatically once its PR merges. Don't recreate deleted merged branches.
- **Abandoning work** — if a PR is closed **without merging** and the work is still wanted, reopen/flag the linked issue with a pointer to the branch **before** walking away. Otherwise the issue looks done while the fix lives only on a dead branch.
- **Automated safety net** — the weekly **SRS Branch Auditor** cloud routine reports merged-but-undeleted branches and reopens any issue whose fix survives only on an unmerged branch.
