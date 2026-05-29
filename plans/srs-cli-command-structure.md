# Plan: SRS CLI Command Structure

## Summary

Define and implement a stable, entity-first `srs` CLI surface for essential CRUD across core SRS entities, package definitions, extension definition records, generic Tier 2 records, and protocol definitions. JSON remains the default output for the first implementation, while a global output option is introduced so structured human-readable output can be added without changing command semantics.

## Agent Assignments

| Role | Agent |
|---|---|
| Lead Integrator | — |
| Repository Service Worker | — |
| CLI Worker | — |
| Core Model Worker | — |
| Verification | — |

See [agents.md](agents.md) for role definitions.

## Architecture Decisions

| ADR | Decision | Status |
|---|---|---|
| [ADR-001](../docs/adr/001-library-first-architecture.md) | Library crates own reusable behavior; CLI is a thin process interface | accepted |
| [ADR-002](../docs/adr/002-tier2-generic-record-operations.md) | Tier 2 Record operations are generic, not type-specific | accepted |
| [ADR-003](../docs/adr/003-tagdefinition-is-core.md) | TagDefinition is a native core SRS type | accepted |
| [ADR-005](../docs/adr/005-extension-definitions-are-tier2-records.md) | Extension definitions are generic Tier 2 Records; no native Rust struct needed | accepted |
| [ADR-006](../docs/adr/006-protocol-definitions-are-tier2-records.md) | Protocol definitions are generic Tier 2 Records with typed validation structs | accepted |

This plan applies accepted architecture decisions to the public command surface.

---

## Scope

- Add global CLI options: `--repo <path>`, `--format json|text`, and `--pretty`.
- Preserve JSON envelope compatibility: `{ ok, command, version, payload }` and `{ ok:false, command, version, diagnostics }`.
- Keep JSON as the default output format.
- Add an output DTO/rendering seam in `srs-cli` so JSON and future text output consume the same structured values.
- Expose entity-first commands for `repo`, `note`, `tag`, `field`, `type`, `record`, `extension`, and `protocol`.
- Implement essential CRUD where the repository service layer can support it safely.
- Treat protocol commands as definition/package commands only; protocol runs and sessions are deferred.

**Out of scope:**

- Human-readable text rendering beyond a planned `--format text` surface. Returning a structured "not implemented" diagnostic is acceptable until a formatter is built.
- Protocol execution commands such as `protocol run start`, `protocol stage complete`, or `protocol advance`.
- Reworking SRS schemas or changing the persisted JSON shapes except where CRUD support requires manifest/package index updates.
- Adding type-specific library behavior for Tier 2 records beyond explicitly native core entities such as `TagDefinition`.

---

## Command Contract

Global invocation:

```bash
srs --repo <path> --format json|text --pretty <command> ...
```

Defaults and compatibility:

- `--repo` defaults to repository root detection from the current working directory.
- `--format` defaults to `json`.
- `--pretty` pretty-prints JSON only; it has no effect on text output.
- Existing per-command `--json` flags remain deprecated compatibility no-ops during the transition.
- Unsupported `--format text` commands must fail through the standard JSON error envelope while JSON remains the active default, unless a text renderer exists for that command.

Core repository commands:

```bash
srs repo map
srs repo validate
srs repo extensions list
srs repo extensions enable <extension-id>
srs repo extensions disable <extension-id>
```

> **Naming note:** These commands manage `declaredExtensions` in `manifest.json`. The subgroup is `extensions` (not `conformance`) to match the field name in the manifest data model. `srs extension list/get/create` (below) manages extension *definition records* — a distinct concept. `repo extensions` manages which extension IDs are declared active in this repository.

Core instance commands:

```bash
srs note list [--tag <tag>]
srs note get <id>
srs note create
srs note update <id>
srs note delete <id>
srs note tag add <id> <tag>
srs note tag remove <id> <tag>
srs note audit-tags
srs note foundations
```

> **Breaking change note:** The existing `srs note tag <id> <tag>` positional form is replaced by `srs note tag add <id> <tag>`. This is a breaking change for any caller using the old form. Migration: callers must add the `add` subcommand. The old form must not be silently accepted — it will parse as an unknown subcommand.

```bash
srs tag list [--role <role>]
srs tag get <id>
srs tag create
srs tag update <id>
srs tag delete <id>
```

Package definition commands:

```bash
srs field list [--namespace <ns>]
srs field get <id>
srs field create
srs field update <id>
srs field delete <id>

srs type list [--namespace <ns>]
srs type get <id>
srs type create
srs type update <id>
srs type delete <id>
```

Generic Tier 2 record commands:

```bash
srs record list --type <namespace>/<name> [--version <n>]
srs record get <id>
srs record create --type <namespace>/<name> [--dir <relative-dir>]
srs record update <id>
srs record delete <id>
```

Extension definition commands:

```bash
srs extension list
srs extension get <id-or-extension-id>
srs extension create
srs extension update <id>
srs extension delete <id>
```

> **Prerequisite:** Extension definition commands require a typed `Extension` struct in `srs-core`. These commands are blocked until that type exists. Do not implement the `extension` command group without it.

Protocol definition commands:

```bash
srs protocol list [--namespace <ns>] [--tag <tag>]
srs protocol get <id>
srs protocol stages <id>
srs protocol validate <id>
srs protocol export <id>
srs protocol import
```

> **Prerequisite:** Protocol commands require an ADR deciding the canonical storage location for protocol definitions. Phase 4 is blocked until that ADR is accepted.

Relation commands:

```bash
srs relation create
srs relation list [--source <id>] [--target <id>] [--type <relation-type>]
srs relation get <id>
srs relation delete <id>
```

> **Note:** Relation commands are in scope for this plan but were missing from the original draft. They read/write `relations/relations.json` and the existing relation loading infrastructure in `srs-repository`. `relation create` reads JSON from stdin.

Input conventions:

- `create`, `update`, `import`, and record writes read JSON from stdin.
- `record create` accepts a `fieldValues` array from stdin matching the canonical Record shape: `{ "fieldValues": [{ "fieldId": "<uuid>", "value": <json> }, ...] }`. The service resolves the target Type from `--type <namespace>/<name>` plus optional `--version` and validates field IDs and value types before writing.
- `record update` accepts the same `fieldValues` shape as `record create`; the service merges provided values onto the existing record and revalidates.
- `note update` accepts a full Note JSON object from stdin (same shape as `note get` output). The service replaces the stored note and updates the manifest title if changed.
- Delete commands remove the stored JSON file and update the manifest or package index in one service operation. Successful delete returns the deleted entity's `instanceId` and `path` in the payload for audit purposes.

---

## Phases

### Phase 1: Global CLI Shape and Output DTOs

**Goal:** The CLI has stable global options and output rendering infrastructure without changing existing command behavior.

**Agent:** CLI Worker

#### Tasks

- [ ] Move repository path resolution to a global `Cli` option while preserving command compatibility during the transition.
- [ ] Add `OutputFormat` enum with `json` and `text` values.
- [ ] Add `--pretty` for pretty JSON rendering.
- [ ] Replace direct string output construction in command handlers with shared output DTOs that can render JSON now and text later.
- [ ] Keep the existing JSON envelope keys and command names stable for current commands.
- [ ] Keep existing per-command `--json` flags as deprecated no-ops where they already exist.

#### Acceptance Criteria

- [ ] Existing commands still return parseable JSON envelopes by default.
- [ ] `srs --repo <path> repo map` works from outside the repository.
- [ ] `srs --format json repo map` matches default behavior.
- [ ] `srs --pretty repo map` returns pretty-printed JSON.
- [ ] `--format text` has a consistent planned behavior and does not panic.

#### Testing

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

Specific tests:

- `global_repo_option_resolves_repo` — verifies global `--repo`.
- `format_json_is_default` — verifies default output is JSON.
- `pretty_outputs_multiline_json` — verifies pretty rendering.
- `format_text_returns_planned_diagnostic_until_renderer_exists` — verifies safe text-mode behavior.

---

### Phase 2: Repository Services for Missing CRUD

**Goal:** Reusable repository services exist for all CRUD operations needed by the CLI.

**Agent:** Repository Service Worker

#### Tasks

- [ ] Add note update/delete services, including validation, file writes/removal, and manifest updates.
- [ ] Add tag update/delete services for native `TagDefinition`.
- [ ] Add `remove_note_tag` service alongside existing `add_note_tag`.
- [ ] Add package definition services for fields and types: list, get, create, update, delete.
- [ ] Add generic record update/delete services alongside existing list/get/create.
- [ ] Add manifest declared-extension services: list active extension IDs, add extension ID, remove extension ID.
- [ ] Add relation CRUD services: create, list (with source/target/type filters), get by ID, delete.
- [ ] Return structured result enums for not found, conflict, validation failure, and successful mutation.
- [ ] Extension definition services and protocol definition services are **deferred** — extension requires a typed model in `srs-core`; protocol requires an ADR.

#### Acceptance Criteria

- [ ] CLI handlers do not perform business logic or direct repository writes.
- [ ] CRUD services update manifest/package indexes atomically enough for the existing file-backed repository model.
- [ ] Delete operations remove the index entry and the target file and return the deleted entity ID.
- [ ] Type/field changes validate against `srs-core` models and schemas where available.
- [ ] Record operations resolve Types through package data and do not hardcode type UUIDs in CLI code.

#### Testing

```bash
cargo test -p srs-repository
cargo clippy -p srs-repository -- -D warnings
```

Specific tests:

- `note_update_rewrites_file_and_manifest_title`
- `note_delete_removes_file_and_manifest_entry`
- `note_tag_remove_updates_note`
- `tag_update_rewrites_definition`
- `tag_delete_removes_definition`
- `field_create_update_delete_updates_package_manifest`
- `type_create_update_delete_updates_package_manifest`
- `record_update_validates_against_type`
- `record_delete_removes_file_and_manifest_entry`
- `declared_extensions_enable_disable_updates_manifest`
- `relation_create_appends_to_relations_file`
- `relation_delete_removes_from_relations_file`

---

### Phase 3: Entity-First CLI Commands

**Goal:** The public CLI exposes the full command contract (excluding extension and protocol groups, which are blocked) and delegates behavior to repository services.

**Agent:** CLI Worker

#### Tasks

- [ ] Add nested `repo extensions` commands (list/enable/disable).
- [ ] Extend `note` with update/delete and promote `note tag` to a nested subgroup with `add` and `remove`.
- [ ] Extend `tag` with update/delete.
- [ ] Add `field` command group.
- [ ] Add `type` command group.
- [ ] Add `record` command group.
- [ ] Add `relation` command group.
- [ ] Standardize command names in output envelopes, for example `record list`, `repo extensions enable`.
- [ ] Standardize stdin parse errors as envelope diagnostics.
- [ ] Extension and protocol command groups are **deferred** to Phase 4 and blocked on their prerequisites.

#### Breaking change

`srs note tag <id> <tag>` (positional, current form) becomes `srs note tag add <id> <tag>`. Update all integration tests that use the old form before shipping Phase 3. Document the change in the phase commit message.

#### Acceptance Criteria

- [ ] Every non-blocked command in the command contract parses and returns a standard JSON envelope.
- [ ] Not-found conditions return `ok:false` with diagnostics and a nonzero process exit.
- [ ] Validation failures return `ok:false` with diagnostics and a nonzero process exit.
- [ ] Successful writes return the written entity plus the relative path or affected index metadata when useful.
- [ ] Existing command invocations used by current integration tests keep working (except `note tag` old form, which is explicitly replaced).

#### Testing

```bash
cargo test -p srs-cli
cargo clippy -p srs-cli -- -D warnings
```

Specific tests:

- Add parser/integration coverage for every non-blocked command in the contract.
- Add stdin failure tests for invalid JSON on each write command family.
- Add compatibility tests for existing `note`, `repo`, `migrate`, and `tag` commands.
- Add a test verifying `srs note tag <id> <tag>` (old form) fails with a parse error, not silent misbehavior.

---

### Phase 4: Extension and Protocol Definition Support

**Status:** open

**Goal:** Extension and protocol commands are implemented following the same pattern as Phase 3 commands. Per ADR-005, extensions use generic record services against the existing `meta.extension` type. Per ADR-006, protocols use generic record services for storage plus typed validation structs in `srs-core`.

**Agent:** Core Model Worker (protocol validation structs + spec package type) + Repository Service Worker (services) + CLI Worker (commands)

**Prerequisite gate (spec package work, before Rust implementation):**
- Create `srs/srs/package/types/protocol.json` defining `com.semanticops.srs/protocol@1`
- Create field definitions in `srs/srs/package/fields/` for: `protocol-id`, `protocol-namespace`, `protocol-name`, `protocol-version`, `protocol-description`, `protocol-target-type`, `protocol-stages`, `protocol-tags`, `protocol-created-at`
- Update `srs/srs/package/package.json` with new type and field paths

#### Tasks

- [ ] *(Spec)* Create `com.semanticops.srs/protocol@1` type and fields in `srs/srs/package/`
- [ ] *(Core)* Add `Protocol` and `ProtocolStage` validation-only structs to `srs-core/src/types/` (not storage structs — used only by the validation service)
- [ ] *(Core)* Add `validate_protocol` to `srs-core/src/validation/` covering: no self-dependency, no cycles in `dependsOn`, `order` consistent with `dependsOn` partial order, all `dependsOn` stageIds exist
- [ ] *(Repository)* Add `validate_protocol_definition(repo_root, id)` service — loads record via generic `get_record_by_id`, deserializes `stages` fieldValue into `Vec<ProtocolStage>`, runs `validate_protocol`, returns diagnostics
- [ ] *(Repository)* Add `list_protocol_stages(repo_root, id)` service — returns ordered stage summaries from the `stages` fieldValue
- [ ] *(Repository)* Extension services use existing `list_records_by_type` / `get_record_by_id` / `create_record` with `meta.extension` type resolved by name — no new service functions needed
- [ ] *(CLI)* Add `extension` command group: list, get, create, update, delete
- [ ] *(CLI)* Add `protocol` command group: list, get, stages, validate, export, import

#### Acceptance Criteria

- [ ] `extension list/get/create/update/delete` work against a repo with extension definition records.
- [ ] `protocol list` can enumerate definitions from a repo/package that declares protocols.
- [ ] `protocol get <id>` returns the full definition.
- [ ] `protocol stages <id>` returns ordered stage summaries with dependency metadata.
- [ ] `protocol validate <id>` reports protocol invariant violations without mutating files.
- [ ] `protocol export <id>` emits a portable JSON definition.
- [ ] `protocol import` validates stdin and writes only definition data, not run state.

#### Testing

```bash
cargo test -p srs-core
cargo test -p srs-repository
cargo test -p srs-cli
```

Specific tests:

- `extension_create_update_delete_updates_package`
- `protocol_validate_rejects_missing_depends_on_stage`
- `protocol_validate_rejects_self_dependency`
- `protocol_validate_rejects_order_before_dependency`
- `protocol_import_roundtrips_exported_definition`
- `protocol_commands_do_not_create_run_state`

---

### Phase 5: Verification and Compatibility Pass

**Goal:** The completed CLI surface is coherent, documented by tests, and does not violate crate boundaries.

**Agent:** Verification Agent

#### Tasks

- [ ] Run the full workspace test suite.
- [ ] Run clippy with warnings denied.
- [ ] Audit CLI handlers for duplicated business logic or direct filesystem writes.
- [ ] Audit JSON envelope compatibility for existing commands.
- [ ] Confirm protocol run/session commands are absent.
- [ ] Confirm `--format text` behavior is consistent and ready for future renderer work.
- [ ] Confirm `srs note tag <id> <tag>` old form is no longer accepted.

#### Acceptance Criteria

- [ ] All tests pass.
- [ ] Clippy passes with warnings denied.
- [ ] Existing JSON command outputs remain compatible except for intentionally added fields.
- [ ] All new public commands have integration coverage.
- [ ] No command implements semantic policy in `srs-cli`.

#### Testing

```bash
cargo test
cargo clippy -- -D warnings
```

---

## Final Acceptance

All of the following must be true before this plan is closed:

- [ ] `cargo test` passes.
- [ ] `cargo clippy -- -D warnings` passes.
- [ ] Every non-blocked command listed in the command contract parses successfully.
- [ ] Existing CLI JSON envelope shape remains stable.
- [ ] Global `--repo`, `--format`, and `--pretty` are implemented.
- [ ] Per-command `--json` compatibility flags do not break existing callers.
- [ ] CRUD services live in `srs-repository` or `srs-core`, not in CLI handlers.
- [ ] `srs note tag add/remove` replaces the old positional `srs note tag` form.
- [ ] `srs repo extensions` is used for declared-extension management; `srs extension` manages definition records.
- [ ] `record create` and `record update` stdin shape is `{ "fieldValues": [...] }`.
- [ ] `note update` stdin shape is a full Note JSON object.
- [ ] Delete responses include the deleted entity ID and path.
- [ ] Protocol support is limited to definitions; no run/session state is introduced.
- [ ] Extension and protocol commands are either fully implemented or explicitly deferred with their prerequisites documented.

## Coordination Rules

- Agents keep to their write scopes unless the Lead Integrator explicitly expands them.
- Agents must not revert edits made by others.
- Repository Service Worker implements reusable service APIs before CLI Worker wires command handlers.
- CLI Worker may add command parser stubs before services exist only if they return structured "not implemented" diagnostics and are replaced before final acceptance.
- Core Model Worker owns any new extension/protocol structs and validation that do not require filesystem access.
- Verification Agent runs after each major phase and before final sign-off.

## Assumptions

- JSON remains the default output until structured human-readable rendering is implemented.
- Entity-first commands are the public CLI; generic `record` commands remain available for package-defined Tier 2 records.
- `TagDefinition` remains native core behavior, not generic Tier 2 behavior.
- Extension definition records are ordinary package-defined records, but require a typed model in `srs-core` before services can be built.
- Protocol definitions belong to package/distribution data. Protocol execution is a later design. Storage location requires an ADR.
- The initial implementation may preserve existing `migrate packet --foundation` unchanged unless a later plan replaces migration handoff commands.
- `repo extensions` (managing `declaredExtensions` in the manifest) and `extension` (managing extension definition records) are distinct command groups with distinct semantics. They must not be merged.
