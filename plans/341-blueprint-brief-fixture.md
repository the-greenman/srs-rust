# Plan: dogfood fixture for blueprint brief contributesTo.typeId negative case

## Summary

The S7 dogfooding scenario ("Verify a document type is correctly composed — Blueprint schema + brief") exercises `blueprint brief` against `../../muDemocracy.org/muSrs` — an external repo unavailable in CI and fresh checkouts. The negative case for a protocol stage carrying a ghost `contributesTo.typeId` (added by #206) can only be confirmed manually in a full governance checkout. This plan adds a self-contained fixture repo under `crates/srs-cli/tests/fixtures/blueprint-brief/` with a blueprint and a protocol whose third stage has an unresolvable `typeId`, and rewrites S7's negative case in `docs/dogfooding.md` to drive the CLI against that fixture with concrete runnable commands.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | Claude Code (this session) |
| CLI Worker | Claude Code (this session) |
| Verification | Claude Code (this session) |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

No new architectural decisions. This plan adds static fixture data and updates documentation only. No ADR governs test fixture structure; the existing pattern (`crates/srs-cli/tests/fixtures/`) is followed directly.

---

## Contracts

### CLI output contract (ADR-011)

No CLI command outputs change. No handler, service, or payload struct is modified. No schema regeneration needed.

### Entity schema sync (check-schema-sync.sh)

No JSON Schema files under `srs/docs/schema/2.0/` are changed. No schema sync needed.

---

## Scope

- Add fixture repo at `crates/srs-cli/tests/fixtures/blueprint-brief/` with:
  - `manifest.json`
  - `.srs/.gitkeep`
  - `relations/relations.json` (empty)
  - `package/package.json`
  - `package/fields/title.json` (field id `00000000-0000-4000-8000-000000004601`)
  - `package/types/document.json` (type id `00000000-0000-4000-8000-000000004611`)
  - `package/blueprints/extraction.json` (blueprint id `00000000-0000-4000-8000-000000004621`)
  - `package/protocols/extraction.json` (three stages: clean / valid-typeId / ghost-typeId)
- Update `docs/dogfooding.md` S7 negative case to reference the fixture with runnable CLI commands.

**Out of scope:**

- Changing any CLI handler, service function, or payload struct.
- Updating the S7 happy-path steps (they still reference `muDemocracy.org/muSrs` for real-world coverage).
- Adding integration test code (`.rs` files) — this is a dogfooding-level exercise, not a unit test.
- Any other fixture beyond what is needed to exercise the typeId negative case.

---

## Phases

### Phase 1: Create the fixture and update the dogfooding guide

**Goal:** `srs repo validate --repo crates/srs-cli/tests/fixtures/blueprint-brief` returns `ok: true, errors: 0`; `srs blueprint brief 00000000-0000-4000-8000-000000004621 --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty` returns `ok: true` with exactly one diagnostic containing `"contributesTo type 00000000-0000-0000-0000-000000000000 not found in package"` and with the ghost-typeId stage still present in `payload.protocol.stages`; the S7 negative case in `docs/dogfooding.md` shows these as runnable commands.

**Agent:** CLI Worker

#### UUID legend

All deterministic UUIDs follow the fixture convention (high bytes encode the fixture index):

| Object | UUID / id |
|---|---|
| Repository | `00000000-0000-4000-8000-000000004600` |
| Field: title | `00000000-0000-4000-8000-000000004601` |
| Type: document | `00000000-0000-4000-8000-000000004611` |
| Blueprint | `00000000-0000-4000-8000-000000004621` |
| Ghost typeId (invalid) | `00000000-0000-0000-0000-000000000000` |

The protocol has no fixed UUID — `protocolId` is a short string `"brief-fixture-proto-1"`.

#### Tasks

- [ ] Create directory `crates/srs-cli/tests/fixtures/blueprint-brief/` with subdirectories:
  `.srs/`, `relations/`, `package/fields/`, `package/types/`, `package/blueprints/`, `package/protocols/`

- [ ] Write `manifest.json`:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/manifest.json",
    "srsVersion": "2.0",
    "repositoryId": "00000000-0000-4000-8000-000000004600",
    "namespace": "fixture.brief",
    "title": "Blueprint Brief Fixture",
    "container": {
      "containerId": "00000000-0000-4000-8000-000000004600",
      "title": "Blueprint Brief Fixture"
    },
    "containerIndex": [],
    "instanceIndex": [],
    "relationsPath": "relations/relations.json",
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```

- [ ] Write `.srs/.gitkeep` (empty file).

- [ ] Write `relations/relations.json`:
  ```json
  {"relations": []}
  ```

- [ ] Write `package/fields/title.json`:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/field.json",
    "id": "00000000-0000-4000-8000-000000004601",
    "namespace": "fixture.brief",
    "name": "title",
    "version": 1,
    "description": "Record title",
    "aiGuidance": {"purpose": "capture record title"},
    "valueType": "string",
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```

- [ ] Write `package/types/document.json`:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
    "id": "00000000-0000-4000-8000-000000004611",
    "namespace": "fixture.brief",
    "name": "document",
    "version": 1,
    "description": "A minimal document type for blueprint brief fixture",
    "fields": [
      {
        "fieldId": "00000000-0000-4000-8000-000000004601",
        "order": 0,
        "required": true,
        "repeatable": false
      }
    ],
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```

- [ ] Write `package/blueprints/extraction.json`:
  ```json
  {
    "id": "00000000-0000-4000-8000-000000004621",
    "namespace": "fixture.brief",
    "name": "extraction-blueprint",
    "version": 1,
    "description": "Blueprint for blueprint-brief fixture: one root type, targeted by the extraction protocol.",
    "rootTypes": [
      {
        "typeId": "00000000-0000-4000-8000-000000004611",
        "typeVersion": 1
      }
    ],
    "structure": [],
    "requiredTypes": [],
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```

- [ ] Write `package/protocols/extraction.json` with three stages:
  - Stage 1 `"gather"`: clean, `contributesTo: [{"fieldId": "..4601"}]` — no typeId.
  - Stage 2 `"extract"`: valid typeId, `contributesTo: [{"fieldId": "..4601", "typeId": "00000000-0000-4000-8000-000000004611"}]` — matches the document type's `id`.
  - Stage 3 `"classify"`: ghost typeId, `contributesTo: [{"fieldId": "..4601", "typeId": "00000000-0000-0000-0000-000000000000"}]` — does not match any package type.

  ```json
  {
    "protocolId": "brief-fixture-proto-1",
    "protocolNamespace": "fixture.brief",
    "protocolName": "extraction",
    "protocolVersion": 1,
    "protocolDescription": "Fixture protocol for testing contributesTo.typeId resolution in blueprint brief.",
    "protocolTargetType": "00000000-0000-4000-8000-000000004611",
    "protocolCreatedAt": "2026-01-01T00:00:00Z",
    "protocolStages": [
      {
        "stageId": "s1",
        "name": "Gather",
        "order": 1,
        "dependsOn": [],
        "question": "What is the subject?",
        "completionCriteria": "Subject identified.",
        "contributesTo": [
          {"fieldId": "00000000-0000-4000-8000-000000004601"}
        ],
        "aiGuidance": "Look for the main topic."
      },
      {
        "stageId": "s2",
        "name": "Extract",
        "order": 2,
        "dependsOn": ["s1"],
        "question": "What fields can be extracted?",
        "completionCriteria": "Fields extracted.",
        "contributesTo": [
          {
            "fieldId": "00000000-0000-4000-8000-000000004601",
            "typeId": "00000000-0000-4000-8000-000000004611"
          }
        ],
        "aiGuidance": "Extract field values."
      },
      {
        "stageId": "s3",
        "name": "Classify",
        "order": 3,
        "dependsOn": ["s2"],
        "question": "How should this be classified?",
        "completionCriteria": "Classification recorded.",
        "contributesTo": [
          {
            "fieldId": "00000000-0000-4000-8000-000000004601",
            "typeId": "00000000-0000-0000-0000-000000000000"
          }
        ],
        "aiGuidance": "Assign a category."
      }
    ]
  }
  ```

- [ ] Write `package/package.json`:
  ```json
  {
    "$schema": "https://srs.semanticops.com/schema/2.0/package-manifest.json",
    "id": "fixture.brief.pkg",
    "namespace": "fixture.brief",
    "name": "blueprint-brief-fixture",
    "title": "Blueprint Brief Fixture",
    "description": "Fixture for dogfooding blueprint brief contributesTo.typeId negative case.",
    "status": "active",
    "version": "1.0.0",
    "fields": ["fields/title.json"],
    "types": ["types/document.json"],
    "blueprints": ["blueprints/extraction.json"],
    "protocols": ["protocols/extraction.json"],
    "relationTypes": [],
    "views": [],
    "documentViews": [],
    "createdAt": "2026-01-01T00:00:00Z"
  }
  ```

- [ ] Validate the fixture:
  ```bash
  cargo run --bin srs -- repo validate --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty
  ```
  Must return `ok: true`, `summary.errors: 0`.

- [ ] Run `srs blueprint brief` against the fixture and confirm the output:
  ```bash
  cargo run --bin srs -- blueprint brief 00000000-0000-4000-8000-000000004621 \
    --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty
  ```
  Must return:
  - `ok: true`
  - `payload.diagnostics` contains exactly one entry: `"contributesTo type 00000000-0000-0000-0000-000000000000 not found in package"`
  - `payload.protocol.stages` has 3 entries (stages s1, s2, s3 all present — including the ghost-typeId stage)
  - `payload.protocol.stages[2].name` is `"Classify"` (the ghost-typeId stage is still present and named correctly)

- [ ] Update `docs/dogfooding.md` S7 negative case: replace the narrative description of the `contributesTo.typeId` case (currently lines 222-223) with concrete CLI commands pointing to the new fixture. The update must:
  - Retain the introductory narrative sentence.
  - Add the runnable `srs blueprint brief` command for the ghost-typeId stage.
  - Show the expected diagnostic message.
  - Confirm the ghost-typeId stage is still present in `payload.protocol.stages`.
  - Also update the S7 "Done when" criterion for the typeId case (line 225) to say the fixture command is the verification path.

- [ ] Update the fixture table in `docs/dogfooding.md` (lines ~30-40 of the Fixtures section) to add an entry for `blueprint-brief/`.

#### Acceptance Criteria

- [ ] `srs repo validate --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty` → `ok: true`, `summary.errors: 0`.
- [ ] `srs blueprint brief 00000000-0000-4000-8000-000000004621 --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty` → `ok: true`, exactly one diagnostic `"contributesTo type 00000000-0000-0000-0000-000000000000 not found in package"`, and all three stages present in `payload.protocol.stages`.
- [ ] Stage s2 produces no diagnostic (valid typeId `..4611` resolves to the `document` type).
- [ ] `docs/dogfooding.md` S7 negative case references the fixture with a runnable command and expected output.
- [ ] Fixture table in `docs/dogfooding.md` includes an entry for `blueprint-brief/`.
- [ ] `cargo test` passes with no failures.
- [ ] `cargo clippy -- -D warnings` passes.

#### Testing

```bash
cargo run --bin srs -- repo validate --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty
cargo run --bin srs -- blueprint brief 00000000-0000-4000-8000-000000004621 \
  --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty
cargo test
cargo clippy -- -D warnings
```

No new Rust test functions are added — this is fixture data + dogfooding guide only.

#### Milestone gate

1. Both CLI commands above return the expected output (validate: 0 errors; brief: ok:true + 1 diagnostic for ghost typeId + 3 stages).
2. `cargo test` and `cargo clippy -- -D warnings` pass.
3. Mark task checkboxes `[x]`, commit.

---

## Final Acceptance

- [ ] `cargo test` passes with no failures
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `srs repo validate --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty` → `ok: true`, `errors: 0`
- [ ] `srs blueprint brief 00000000-0000-4000-8000-000000004621 --repo crates/srs-cli/tests/fixtures/blueprint-brief --pretty` → `ok: true`, 1 diagnostic for ghost typeId, 3 stages present
- [ ] `docs/dogfooding.md` S7 negative case has runnable commands for both the fieldId and typeId cases
- [ ] `cargo test --test payload_contracts` passes (no payload structs changed)
- [ ] `bash scripts/check-schema-sync.sh` exits 0 (no entity schemas changed)

## Coordination Rules

- At the end of the phase: verify all acceptance criteria, confirm planned tests pass, update the plan checkboxes, then commit.

## Assumptions

- The type file's `"id"` field (`00000000-0000-4000-8000-000000004611`) is what `get_type_by_id_latest` matches against. The `typeId` key in the JSON (if present separately) goes into `extra` and is not used by the lookup. Setting `id == typeId` is deliberate to keep the fixture self-consistent.
- A valid `contributesTo.typeId` (stage s2) produces no diagnostic; only the ghost typeId (stage s3) triggers the `"contributesTo type X not found in package"` diagnostic.
- The fixture has no records (`instanceIndex: []`) — this is sufficient because `blueprint brief` reads only package definitions.
- The `protocolTargetType` in the protocol file (`..4611`) must equal the blueprint's `rootTypes[0].typeId` (`..4611`) for `find_protocol_by_target_type` to link the protocol to the blueprint.
