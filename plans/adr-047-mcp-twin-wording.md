# Plan: ADR-047 — correct the "WASM/MCP twins" overreach

> **Usage note:** The purpose of a plan file is to be reviewed and executed by agents. Write it with that reader in mind: unambiguous tasks, explicit file paths, named functions, checkable acceptance criteria. A plan that requires human interpretation at execution time is incomplete.
>
> Save this file to `plans/<slug>.md` before assigning agents. Agents receive the plan file as their primary brief.

## Summary

Weekly architecture review (2026-08-21) found that `docs/adr/047-repo-doctor-explicit-repair-surface.md`'s Decision section states `doctor_service::doctor` is reachable "only from the `repo doctor` CLI handler and its **WASM/MCP twins**". This is inaccurate: only a CLI handler (`crates/srs-cli/src/commands/repo.rs::cmd_repo_doctor`) and a WASM binding (`SrsRepository::doctor` in `crates/srs-bindings/src/lib.rs`) exist. No `srs-mcp` tool for doctor exists anywhere in the codebase (`crates/srs-mcp/src/tools.rs` defines only `TOOL_REPO_VALIDATE` for the `repo` namespace; confirmed by grep and by the exhaustive MCP tool list exercised in `crates/srs-mcp/tests/tools.rs`). This plan is a pure documentation correction — no code, schema, or behavior changes. Whether an MCP tool for doctor should be added is tracked separately as #861 (not implemented here).

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude (this session) |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan corrects a factual claim inside an existing ADR (ADR-047) to match the codebase. It does not change ADR-047's actual decision (never-on-load-path, reuse of `catalog_unchecked`, the repair inventory, adopt's stop condition) — only the sentence describing which surfaces currently expose the service.

| ADR | Decision | Status |
|---|---|---|
| [ADR-047](../docs/adr/047-repo-doctor-explicit-repair-surface.md) | Text-accuracy fix only — no change to the decision itself | proposed (unchanged) |

**Why an in-place edit, not an Amendment block:** this repo's convention (see ADR-015, 019, 025, 031, 035, 037, 038, 040, 043) is to append a dated `## Amendment (YYYY-MM-DD, #issue)` section to an already-`accepted` ADR rather than rewrite its original Decision prose. ADR-047 is still `Status: proposed` — it has not been frozen by acceptance — so an in-place correction of the inaccurate sentence is the right move here, not an Amendment. If ADR-047 is accepted before this plan lands, switch to an Amendment block instead.

---

## Contracts

### CLI output contract (ADR-011)

No new/changed commands, no payload changes. No action required.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema changes. No action required.

---

## Scope

- Edit `docs/adr/047-repo-doctor-explicit-repair-surface.md`'s Decision section (the "Never on any load path" paragraph) to name only the surfaces that actually exist today — the CLI handler and the WASM binding — and drop the false "MCP twins" claim, replacing it with an explicit note that MCP exposure is not yet implemented (tracked in #861).
- Grep the rest of the repo's docs (`*.md`) for the same "WASM/MCP twins" phrasing or any other doctor+MCP claim, in case it was copied elsewhere, and fix any hit.

**Out of scope:**

- Implementing an MCP tool for `repo doctor` — filed and tracked separately as #861, parented under #350.
- Any change to `doctor_service.rs`, the CLI handler, or the WASM binding — this plan touches documentation only.

---

## Phases

### Phase 1: Correct ADR-047

**Goal:** ADR-047's Decision section accurately describes the two surfaces (CLI, WASM) that expose `doctor_service::doctor` today, with no dangling references to a nonexistent MCP tool anywhere in the repo's docs.

**Agent:** Lead Integrator

#### Tasks

- [x] In `docs/adr/047-repo-doctor-explicit-repair-surface.md`, replace the sentence "`doctor_service::doctor` is reachable only from the `repo doctor` CLI handler and its WASM/MCP twins, all triggered by an explicit user request." with text naming the CLI handler and the WASM binding by their actual names/paths, and stating MCP exposure is not yet implemented (tracked in #861).
- [x] `rg -n "WASM/MCP|MCP twin" --glob '*.md' .` from the repo root and fix any other hit describing doctor's surfaces inaccurately.

#### Acceptance Criteria

- [x] ADR-047 no longer claims an MCP twin exists for `repo doctor`.
- [x] ADR-047 correctly names `cmd_repo_doctor` (CLI) and `SrsRepository::doctor` (WASM) as the two real entry points.
- [x] No other doc in the repo repeats the inaccurate claim.
- [x] `doctor_service::doctor`'s actual behavior (never-on-load-path, the repair inventory, the reuse-not-widen seam decision) is unchanged in the ADR text — only the surfaces sentence changes.

#### Testing

```bash
# Docs-only change; confirm nothing else regressed
cargo test -p srs-repository --test doctor_service
cargo clippy -- -D warnings
rg -n "WASM/MCP|MCP twin" --glob '*.md' .
```

Specific tests to write or verify: none — no code changed. The doctor_service integration test suite is re-run only as a sanity check that the worktree is otherwise clean.

#### Milestone gate

1. Verify both acceptance-criteria checkboxes above are met.
2. Confirm the `rg` sweep returns no remaining "WASM/MCP"/"MCP twin" hits anywhere under `*.md`.
3. Run:

```bash
cargo test -p srs-repository --test doctor_service
cargo clippy -- -D warnings
```

4. Update this plan file: mark the Phase 1 task and acceptance-criteria checkboxes `[x]`.
5. Commit: `git commit`.

Do not proceed to Stage 6 (sync + final acceptance) until this gate passes and the plan file is updated.

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [x] `cargo test` passes with no failures
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (no payload structs changed by this plan — confirmed no-op, 115 passed)
- [x] `bash scripts/check-schema-sync.sh` — no entity schemas changed by this plan, so N/A; the script itself errors in this worktree only because it expects a sibling `../srs` checkout that doesn't exist at the worktree's path (unrelated to this change — the checkout at `/home/user/srs-rust`'s sibling `/home/user/srs` is unaffected)
- [x] ADR-047 no longer contains the phrase "WASM/MCP twins" (or any other claim of an MCP tool for doctor)
- [x] `rg -n "WASM/MCP|MCP twin" --glob '*.md' .` from the repo root returns no hits (outside this plan file's own description of the fix)
- [x] ADR-047's Decision content (never-on-load-path, repair inventory, reuse-not-widen seam) is byte-for-byte unchanged apart from the one corrected sentence

## Coordination Rules

- Single-agent plan (Lead Integrator only) — no cross-agent write-scope conflicts to manage.
- **At the end of the one phase:** verify all acceptance criteria, confirm the `rg` sweep is clean, update the plan checkboxes, then commit. Do not open the PR without completing the milestone gate.
- Verification Agent runs in Stage 7 (code review loop) against the final diff before PR.

## Assumptions

- ADR-047 remains `Status: proposed` for the duration of this plan (see the in-place-edit-vs-Amendment note above). If it gets accepted mid-flight, the wording fix should be redone as an Amendment block instead of an in-place edit.
- No other repo (`srs`, `srs-web`, `srs-vscode`) repeats the "WASM/MCP twins" claim — this plan's grep sweep is scoped to `srs-rust` only, since ADR-047 and the doctor feature live entirely in this repo.
