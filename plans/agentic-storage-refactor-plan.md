# Agentic Implementation Plan: Storage-Agnostic SRS Rust Library

## Summary

Refactor SRS Rust so reusable SRS logic lives in library crates and the CLI is only an interface. The immediate priority is concrete and synchronous: move note operations out of `srs-cli` and into `srs-repository`, so the same logic can be called by CLI, Python bindings, and future applications without duplication.

The long-term direction is storage-agnostic SRS support for file-backed, database-backed, and embedded applications. The near-term implementation should avoid speculative complexity: no async traits yet, no new file-adapter crate yet, and no model-specific assumptions in the plan.

## Agent Roles

### Lead Integrator

- **Owns:** architecture decisions, sequencing, final integration, public API consistency, and review.
- **Write scope:** workspace manifests, cross-crate wiring, final cleanup.
- **Coordination notes:** merge worker outputs, resolve API disagreements, and enforce the crate-boundary model.

### Repository Service Worker

- **Owns:** moving note service logic and repository operations into `srs-repository`.
- **Write scope:** `crates/srs-repository/**`.
- **Deliverables:**
  - note list/get/create/tag services
  - library-owned slugging
  - manifest update helpers without repeated loads
  - tests proving CLI-independent behavior

### CLI Worker

- **Owns:** thinning CLI commands to call library services only.
- **Write scope:** `crates/srs-cli/**`.
- **Deliverables:**
  - command behavior preserved
  - JSON envelope compatibility
  - no duplicated business logic

### Core Model Worker

- **Owns:** in-memory SRS types and validation in `srs-core`.
- **Write scope:** `crates/srs-core/**`.
- **Deliverables:**
  - canonical Rust structs for remaining SRS data
  - serde-compatible JSON shapes
  - storage-independent validation

### Bindings Worker

- **Owns:** JSON-first Python binding surface over library services.
- **Write scope:** `crates/srs-bindings/**`.
- **Deliverables:**
  - JSON-first callable APIs
  - smoke tests for parseable outputs
  - no duplicated CLI logic

### Verification Agent

- **Owns:** test runs, architecture audits, and duplication checks.
- **Write scope:** none unless explicitly asked to patch tests.
- **Deliverables:**
  - command/test transcript summary
  - crate-boundary audit
  - duplicated-logic report

## Implementation Phases

### Phase 1: Move Note Services To The Library

Move essential note behavior out of `srs-cli` into `srs-repository`.

Services to add:

- `list_notes(repo_root, filter)`
- `get_note_by_id(repo_root, id)`
- `create_note(repo_root, note)`
- `add_note_tag(repo_root, id, tag)`
- `slugify_title(title)`

Requirements:

- Preserve existing CLI output behavior.
- Keep this phase synchronous.
- Keep file-backed support inside `srs-repository` for now.
- Avoid double-loading the manifest in tag/update flows.
- Move slugging out of CLI because it is essential library behavior.
- Return structured service results that CLI and bindings can serialize without reconstructing business logic.

Acceptance:

- Existing `srs note list/get/create/tag` behavior remains compatible.
- CLI command handlers only parse arguments/stdin, call services, and wrap output.
- Service tests cover list, get, create, tag, slugging, missing IDs, non-note IDs, and manifest updates.
- Duplicated note logic is removed from `srs-cli`.

### Phase 2: Add Remaining Core In-Memory Types

Add canonical Rust structs to `srs-core` for:

- fields
- types
- typed records
- records
- packages
- relations
- manifests
- source references

Requirements:

- Keep serde names aligned with existing JSON schemas.
- Preserve extension-tolerant fields where schemas allow implementation-local `meta` or loose package members.
- Keep validation that depends only on in-memory data in `srs-core`.
- Do not add filesystem dependencies to `srs-core`.

Acceptance:

- Core structs roundtrip representative schema-compatible JSON.
- Existing note serialization remains compatible.
- Tests cover field/type/record/package/relation examples.

### Phase 3: Introduce Synchronous Storage Boundaries In `srs-repository`

Define a synchronous repository boundary inside `srs-repository`, alongside the current file implementation.

The first boundary should support:

- manifest load/save
- note load/save
- generic instance JSON load/save
- relations load/save
- schema path discovery
- package JSON load
- source document discovery

Requirements:

- Use synchronous traits or interfaces first.
- Do not introduce async until there is a concrete async consumer.
- Do not create `srs-file-repository` yet.
- Keep the existing filesystem implementation as the first implementation of the boundary.
- Service functions should move toward depending on the boundary rather than directly on paths.

Acceptance:

- Services can run against an in-memory fake store in tests.
- File-backed behavior remains compatible.
- No async runtime, async-trait dependency, pinning, or async lifetime complexity is introduced.

### Phase 4: Defer File Adapter Extraction Until A Second Adapter Exists

Do not create a separate `srs-file-repository` crate in this pass.

Extraction criteria for later:

- a database-backed adapter is being implemented, or
- another non-file adapter needs to share the same service layer, or
- `srs-repository` becomes too large to maintain cleanly.

Until then:

- `srs-repository` owns storage-agnostic services and the file-backed implementation.
- File-specific logic should be isolated into modules, not spread through services.
- The eventual extraction path should remain obvious.

Acceptance:

- The current crate count does not grow prematurely.
- The boundary is clear enough that later extraction is mechanical.
- Database-backed implementations can be planned without forcing the file split now.

### Phase 5: CLI And Python Bindings Over Services

CLI:

- parse command args/stdin
- resolve repo path
- call `srs-repository` services
- print JSON envelopes

Python bindings:

- expose JSON-first functions over the same services
- accept repo paths for file-backed use
- return JSON strings or Python-native JSON-compatible data after parsing

Initial binding functions:

- `repo_map_json`
- `note_list_json`
- `note_get_json`
- `note_create_json`
- `note_tag_json`
- `note_audit_tags_json`
- `note_foundations_json`
- `migration_packet_json`

Acceptance:

- Python bindings do not duplicate logic from CLI.
- Binding smoke tests prove outputs are parseable JSON.
- CLI behavior remains compatible.

## Testing And Acceptance

Required test commands:

```bash
cargo test
node scripts/validate-all.mjs
```

Additional checks:

- Unit test note services directly in `srs-repository`.
- Unit test core model serialization against schema-compatible examples.
- Unit test the synchronous repository boundary with an in-memory fake store.
- Keep CLI integration tests for command compatibility.
- Add binding smoke tests for JSON parseability.
- Run architecture audit for duplicated note logic, storage-specific logic in the wrong layer, and policy embedded in storage-agnostic services.

Final acceptance criteria:

- CLI behavior remains compatible.
- Note logic is reusable without calling CLI code.
- Core data structures are usable in any application.
- File-backed repo remains first-class but is not the only architectural assumption.
- Python bindings call Rust services, not duplicated logic.
- Database-backed implementations can be added later by implementing the synchronous boundary or by introducing async when a concrete async consumer exists.
- SRS record validation still passes.

## Coordination Rules

- Agents are not alone in the codebase.
- Agents must keep to their write scopes unless the Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Workers should return changed file paths and a short behavior summary.
- The Lead Integrator owns final API naming and dependency boundaries.
- The Verification Agent should run after each major phase and before final handoff.

## Assumptions

- Synchronous APIs come first.
- Async is deferred until there is a concrete async consumer.
- Python bindings are JSON-first initially.
- Database adapter implementation is out of scope for this pass.
- A separate `srs-file-repository` crate is deferred until a second adapter justifies the split.
- The CLI may own workflow-facing profile policy, but reusable logic belongs in library crates.
