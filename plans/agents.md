# Agent Operational Brief — srs-rust

This document is a direct operational brief for Claude agents working on the srs-rust project.
Read it as instructions addressed to you. The role you are assigned is stated in the plan's
Agent Assignments table.

Each role defines your write scope, decision rules, escalation triggers, and GitHub issue
responsibilities.

---

## Lead Integrator

**You own:** architecture decisions, work sequencing, final integration, public API naming,
crate-boundary enforcement, and PR readiness.

**Write scope:** workspace `Cargo.toml`, cross-crate wiring, final cleanup across any crate.

**Decision rules:**

- If two workers propose conflicting type names or function signatures, you decide. Post the
  decision as an issue comment so all agents see it.
- If a worker's output violates the crate-boundary model (e.g. `srs-core` gaining file I/O,
  `srs-cli` gaining business logic), reject it and reassign with corrected scope.
- If a new architectural constraint emerges during implementation, create an ADR in
  `docs/adr/` before proceeding.
- If the plan's scope needs to expand, update the GitHub issue body before delegating the
  additional work.

**Escalation:** You are the final escalation point. Decide or defer explicitly — document
deferrals as issue comments.

**GitHub issue responsibilities:**

- Open the governing issue before assigning workers.
- At each milestone gate, verify worker checkboxes are updated.
- Post a comment when the plan moves to a new phase.
- Close the issue when Final Acceptance passes and the PR is merged.

---

## Repository Service Worker

**You own:** service logic and repository operations in `srs-repository`.

**Write scope:** `crates/srs-repository/**`

**Decision rules:**

- If a service operation requires multiple functions, consolidate into one function with a
  filter or options struct. Do not expose multiple overloads for the same logical operation.
- If validation logic would be needed by any consumer beyond the CLI, it belongs here — not
  in the CLI handler.
- If a multi-step operation must be atomic from the caller's perspective, implement it as a
  single service function.
- Service functions must use typed input structs and typed output structs (ADR-010). No
  `serde_json::Value` parameters on public service functions.
- `MemoryStore` is the canonical test double. Write tests against `MemoryStore`, not
  `FileStore`. Tests that only pass against `FileStore` are testing the adapter.
- If you need to touch `srs-core` types, check with the Lead Integrator first — your write
  scope is `srs-repository` only.

**Escalation triggers — post an issue comment and wait before continuing if:**

- The correct location for logic is ambiguous between `srs-core` and `srs-repository`.
- A service change requires modifying the `RepositoryStore` trait in a way that breaks both
  `MemoryStore` and `FileStore` simultaneously.
- You discover business logic in `srs-cli` handlers that overlaps with your work.

**Conflict rule:** If you find edits in `crates/srs-repository/**` that you did not make,
do not revert them. Post an issue comment describing what you found and wait for the Lead
Integrator.

**GitHub issue responsibilities:** At each milestone gate:

```bash
gh issue comment <number> --repo the-greenman/srs-rust \
  --body "Repository Service Worker: Phase N complete. Changed: [...]. Behaviour: [...]."
```

Then update the phase checkboxes in the issue body.

---

## CLI Worker

**You own:** command handlers in `srs-cli` — argument parsing, stdin handling, JSON output.

**Write scope:** `crates/srs-cli/**`

**Decision rules:**

- A handler must contain exactly: arg parsing, one service call, `output::ok/err`. Nothing
  else. If a handler exceeds ~15 lines, the excess is business logic — move it to
  `srs-repository`.
- Every new command output shape requires a named struct in `payload.rs` (ADR-011). No
  `json!({...})` literals in handlers.
- After changing any struct in `payload.rs`, run `cargo run --bin generate-schemas` and
  commit the updated files in `crates/srs-cli/schemas/payload/` on the feature branch.
- If you are unsure of the right field names for a new payload struct, check existing
  `payload.rs` structs and the service output types before inventing names.

**Escalation triggers — post an issue comment and wait before continuing if:**

- The service function you need does not exist and you would have to implement business logic
  in the handler as a workaround.
- A required payload struct shape conflicts with an existing shape in `payload.rs`.
- `cargo test --test payload_contracts` fails and regenerating schemas does not fix it.

**Conflict rule:** If you find edits in `crates/srs-cli/**` that you did not make, do not
revert. Surface to Lead Integrator via issue comment.

**GitHub issue responsibilities:** At each milestone gate, post a comment listing changed
handlers and whether `generate-schemas` was run. Update issue checkboxes.

---

## Core Model Worker

**You own:** in-memory SRS types and validation in `srs-core`.

**Write scope:** `crates/srs-core/**`

**Decision rules:**

- `srs-core` must remain I/O-free and async-free. If you reach for `std::fs` or `tokio`,
  you are in the wrong crate.
- `srs-core` must not gain a `schemars` dependency. Payload mirror structs that need
  `JsonSchema` derivation live in `payload.rs` in `srs-cli`.
- Serde field names must align with the existing JSON schemas in `srs/docs/schema/2.0/`.
  Check the schema before adding a new field.
- Validation that depends only on in-memory data belongs here. Validation that requires a
  store lookup belongs in `srs-repository`.
- If adding a type that will be embedded in a CLI payload, coordinate with the CLI Worker:
  they may need a mirror struct with `#[schemars(with = "serde_json::Value")]`.

**Escalation triggers — post an issue comment and wait before continuing if:**

- A type change requires modifying serde names referenced by existing JSON schema files
  (breaking change to the on-disk format).
- You need to add a dependency that would transitively introduce I/O or async.
- A type you are adding overlaps with a type already in `srs-repository`.

**Conflict rule:** If you find edits in `crates/srs-core/**` that you did not make, do not
revert. Surface to Lead Integrator via issue comment.

**GitHub issue responsibilities:** At each milestone gate, post a comment listing new types
added and any serde name decisions made. Update issue checkboxes.

---

## Bindings Worker

**You own:** the JSON-first binding surface over library services in `srs-bindings`.

**Write scope:** `crates/srs-bindings/**`

**Decision rules:**

- Bindings accept repo paths and return JSON strings or JSON-compatible data. No opaque Rust
  types in the public surface.
- Call the same service functions that `srs-cli` calls. Do not implement parallel logic. If
  a service function does not exist, coordinate with the Repository Service Worker.
- Smoke tests must assert parseability via `serde_json::from_str`, not just that the call
  did not panic.
- If a service output type is not `Serialize`, coordinate to make it so — do not add
  serialization logic in bindings.

**Escalation triggers — post an issue comment and wait before continuing if:**

- The service you need does not exist or has the wrong signature.
- You would need to duplicate CLI-side logic to make a binding work.

**Conflict rule:** If you find edits in `crates/srs-bindings/**` that you did not make, do
not revert. Surface to Lead Integrator via issue comment.

**GitHub issue responsibilities:** At each milestone gate, post a comment listing new binding
functions and confirming smoke tests pass. Update issue checkboxes.

---

## Verification Agent

**You own:** test runs, architecture audits, and duplication checks.

**Write scope:** none by default. You may patch tests only if the Lead Integrator explicitly
authorizes it in the plan or via an issue comment.

**At each phase gate, run and report:**

```bash
cargo test
cargo clippy -- -D warnings
cargo test --test payload_contracts
bash scripts/check-schema-sync.sh
```

**Boundary audit:**

```bash
grep -r 'std::fs' crates/srs-core/           # must return empty
grep -rn 'json!' crates/srs-cli/src/commands/ # must return empty
```

Review any handler exceeding ~15 lines for embedded business logic. Check for duplicated
logic between `srs-cli` handlers and `srs-repository` services.

**Post a structured report as an issue comment:**

```bash
gh issue comment <number> --repo the-greenman/srs-rust --body "$(cat <<'EOF'
Verification Agent report — Phase N

Test run: PASS / FAIL (N failures)
Clippy: PASS / FAIL
payload_contracts: PASS / FAIL
check-schema-sync: PASS / FAIL

Boundary audit:
- srs-core I/O-free: YES / NO (details)
- No json! in handlers: YES / NO (details)
- No business logic in handlers: YES / NO (details)

Duplication check: CLEAN / ISSUES FOUND (details)

Recommendation: PROCEED / BLOCK (reason)
EOF
)"
```

**Escalation:** If you find a boundary violation or duplication issue, post a BLOCK report
and wait for Lead Integrator direction. Do not attempt to fix violations yourself unless
explicitly authorized. A BLOCK recommendation without Lead Integrator acknowledgment is a
hard stop.
