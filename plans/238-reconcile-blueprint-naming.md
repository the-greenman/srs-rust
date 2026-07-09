# Plan: Reconcile Blueprint naming (core definition, not ext:blueprint)

## Summary

Blueprint is a core package definition — parallel to Protocol per ADR-016 — yet `README.md` and a CLI help comment still label it `ext:blueprint`. This mislabelling (Drift D3 from `docs/roadmap/extension-implementation.md`) misleads users into treating Blueprint as an extension. This plan removes the incorrect `ext:` prefix from `README.md` (two occurrences) and the `srs-cli` command comment.

No new logic or payload structs are required — only a `///` clap doc comment in `srs-cli` and two README lines are corrected. Schema mirror files (`crates/srs-schema/schemas/2.0/`) are not in scope: they are canonical spec mirrors that must not be edited directly; the companion spec change in `the-greenman/srs` covers those.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude Code (this session) |
| CLI Worker | Claude Code (this session) |
| Verification | Verification Agent |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions — this plan implements the naming intent of ADR-016 (protocols/blueprints are package definitions, not `ext:` constructs).

| ADR | Decision | Status |
|---|---|---|
| [ADR-009](../docs/adr/009-package-boundary-model.md) | Blueprints are package definitions — the upstream decision ADR-016 builds on | accepted |
| [ADR-016](../docs/adr/016-protocols-are-package-definitions.md) | Blueprints, like protocols, are core package definitions and must not carry an `ext:` label | accepted |

---

## Contracts

### CLI output contract (ADR-011)

No CLI command output shapes change. The comment being fixed is Rust doc/clap help text only.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are modified by this plan. Schema mirror files are out of scope.

---

## Scope

- `README.md` line 51: change `Blueprints (\`ext:blueprint\`)` → `Blueprints (\`blueprint\`)`
- `README.md` line 78: remove `(ext:blueprint)` from the CLI surface entry for `blueprint`
- `crates/srs-cli/src/commands/mod.rs` line 281: remove `(ext:blueprint)` from the `/// Blueprint definition commands` comment

**Out of scope:**
- Schema mirror files (`crates/srs-schema/schemas/2.0/`) — mirrors of canonical spec; companion spec RFC handles these
- Fixture files in `tests/fixtures/spec-repo/` — vendored spec data
- The roadmap file `docs/roadmap/extension-implementation.md` — its D3 entry describes the drift and is accurate as written

---

## Phases

### Phase 1: Fix README and CLI comment

**Goal:** All `ext:blueprint` mislabelling in docs/comments is removed.

**Agent:** CLI Worker

#### Tasks

- [x] Edit `README.md` line 51: `Blueprints (\`ext:blueprint\`)` → `Blueprints (\`blueprint\`)`
- [x] Edit `README.md` line 78: `\`blueprint\` — CRUD, validate, structure (\`ext:blueprint\`)` → `\`blueprint\` — CRUD, validate, structure`
- [x] Edit `crates/srs-cli/src/commands/mod.rs` line 281: `/// Blueprint definition commands (ext:blueprint)` → `/// Blueprint definition commands`

#### Acceptance Criteria

- [x] `grep -n "ext:blueprint" README.md` returns no matches (zero hits in README)
- [x] `grep -n "ext:blueprint" crates/srs-cli/src/commands/mod.rs` returns no matches
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test` passes (2 srs-gov parallel-test-race failures are pre-existing; all pass single-threaded)

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
```

No new tests required — this is a doc/comment change with no runtime behaviour change.

#### Milestone gate

1. Verify all acceptance criteria above are met.
2. Run:
```bash
cargo test
cargo clippy -- -D warnings
```
3. Update plan checkboxes `[x]`.
4. Commit.

---

## Final Acceptance

- [x] `cargo test` passes with no failures (srs-gov parallel-race is pre-existing; single-threaded: all 17 pass)
- [x] `cargo clippy -- -D warnings` passes
- [x] `cargo test --test payload_contracts` passes (payload structs not changed)
- [x] `bash scripts/check-schema-sync.sh` exits 0 (schema files not changed)
- [x] `grep -rn "ext:blueprint" README.md crates/srs-cli/src/commands/mod.rs` returns no matches

## Coordination Rules

- Agents keep to their write scopes unless Lead Integrator explicitly expands them.
- Workers return changed file paths and a short behaviour summary when done.

## Assumptions

- The companion `ext:blueprint` rename in the spec repo (`the-greenman/srs`) and schema mirrors is tracked separately as issue #231's cross-repo work.
- `crates/srs-schema/schemas/2.0/` and `tests/fixtures/spec-repo/` are not in scope; they will drift-correct once the spec RFC lands.
