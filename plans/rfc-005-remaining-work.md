# Plan: RFC-005 Remaining Work — Spec Record Amendments and VS Code Support

> **Usage note:** This plan file is for agent review and execution. Write it with that reader in mind.

## Context

RFC-005 (Core Relation Type Definitions) is mostly implemented. The Rust implementation (Phases 1–5 of the prior plan at `plans/completed/rfc-005-relation-type-definitions.md`) is complete:
- JSON schemas updated, synced, and drift-free
- `RelationTypeDefinition` Rust struct, validation (E1–E4), package loader, CLI commands all done
- 16 canonical definition files shipped; `srs/srs/` validates cleanly (`node scripts/validate-all.mjs` passes)
- `schema drift check` passes

**Three things remain:**

1. **Spec record amendments** — three SRS subsection records reference the old pre-RFC-005 semantics and must be updated per the RFC-005 §"Spec record amendments required" table
2. **VS Code extension** — `srs-vscode` already has a `relation-type` tree node but is missing relation-type-aware relation creation UI (type picker needs to call `relation-type list` and use installed definitions rather than a hardcoded list) and a `relation-type create/update/delete` command palette surface
3. **`node scripts/validate-all.mjs` CI gate** — listed as unchecked in the completed plan; confirm it now passes and close the checkbox

---

## Phase 1: Spec Record Amendments

**Write scope:** `srs/srs/records/subsections/`

Three records need body text (`fieldId: 1a000002-0000-4000-a000-000000000002`) updated. All other fields (instanceId, typeId, typeVersion, sourceRefs, createdAt) are unchanged.

### Record 1 — `07-11-ext-recommended-relations.json`

**instanceId:** `e327ca1f-4cc4-5322-b837-5a27906d5abf`

Replace the entire `value` of field `1a000002-...` with:

```
**Retired as of RFC-005.** The canonical SRS relation vocabulary (`contains`, `depends-on`, `supersedes`, `refines`, `derived-from`, `evidences`, `precedes`) is now provided as installed `RelationTypeDefinition` records in the `com.semanticops.srs` package. See §5 (Package).

Implementations that previously declared `ext:recommended-relations` may remove it. The canonical definitions are unconditionally available to any repository using the SRS package.

The statement that "`RelationTypeDefinition` is optional metadata" is superseded. As of RFC-005, every `Relation.relationType` string must resolve to an installed `RelationTypeDefinition` in the effective package set before a Relation is accepted. A missing or conflicting definition is a validation error. See §9-1 (Core conformance requirements).
```

### Record 2 — `09-1-core-conformance-requirements.json`

**instanceId:** `ab421ac0-561d-5ec0-87b0-09e8bc930a4d`

Replace the `value` of field `1a000002-...`. Keep all existing bullet points; append a new bullet:

```
A core-conformant implementation must:
- Accept and validate `Field`, `Type`, `Record` (Tier 2), `Relation`, and `Container` inputs against this specification
- Enforce Invariants 1–3, 7–9, 16–21, 28, 38
- Support the Foundation and Distribution groups in full
- Implement the namespace format and reference format correctly
- Not accept `relationType` strings that include `/` except in `namespace/name` format
- Resolve every `Relation.relationType` against an installed `RelationTypeDefinition` in the effective package set before accepting a Relation write. A missing or conflicting definition is a validation error.

Support for `Note` (Tier 0) and `Typed Record` (Tier 1) is optional at core conformance level.
```

### Record 3 — `09-2-extension-conformance-requirements.json`

**instanceId:** `95bef379-2358-5848-abb1-ff72cb07a25e`

Replace the `value` of field `1a000002-...`. Keep all existing text; append a sentence noting the retirement:

```
An implementation declaring a given extension must:
- Accept and validate all types defined by that extension
- Enforce all invariants assigned to that extension
- Respect the declared dependency chain (e.g., `ext:views-l2` requires `ext:views-l1` to also be declared)

`ext:recommended-relations` is retired as of RFC-005. It no longer owns any normative semantics. Implementations must not treat it as a capability gate — the canonical relation vocabulary is now mandatory core behaviour provided by the `com.semanticops.srs` package.
```

### Acceptance criteria — Phase 1

- `node scripts/validate-all.mjs` from `srs/` passes (it already does; confirm it still does after edits)
- All three records render correctly under `srs render document-view` (spot-check)
- Mark the two remaining unchecked boxes in `plans/completed/rfc-005-relation-type-definitions.md` Phase 1 and Phase 5 acceptance criteria (`node scripts/validate-all.mjs`)

---

## Phase 2: VS Code Extension — Relation Type Integration

**Write scope:** `srs-vscode/src/`

The VS Code extension has partial support: tree nodes for `relation-type list/get` exist and the type picker in `editCommands.ts` calls the CLI. The gaps are:

1. **Relation creation picker uses hardcoded list** — `editCommands.ts` likely has a static array or simple string input for `relationType`. It should call `srs relation-type list` and present installed definitions as quick-pick items (label + description from the definition).
2. **No `relation-type create/update/delete` command palette entries** — the CLI already has these commands (`srs relation-type create/update/delete`); they need VS Code command registrations and editor scaffolding.

### 2a — Dynamic relation type picker

**File:** `srs-vscode/src/commands/editCommands.ts` (and `srs-vscode/src/cli/types.ts` if needed)

Find the existing relation-creation flow. Where it currently collects `relationType` (likely a `vscode.window.showInputBox` or a static `showQuickPick`), replace it with a dynamic call:

```typescript
// Call CLI to get installed definitions
const listResult = await cliBridge.runCommand(['relation-type', 'list', '--repo', repoRoot]);
// Parse as RelationTypeListPayload
// Build QuickPickItem[] from definitions: label = def.label, description = def.relationType
// Filter out deprecated/retired if desired (show with warning label instead)
const picked = await vscode.window.showQuickPick(items, { title: 'Select relation type' });
```

Use `RelationTypeListPayload` (already defined in `src/cli/types.ts` at line 115).

### 2b — Command palette: `relation-type create/update/delete`

**Files:** `srs-vscode/src/commands/editCommands.ts`, `srs-vscode/package.json`

Register three new VS Code commands following the existing pattern for `srs.createRelation`:

| VS Code command ID | Label | CLI call |
|---|---|---|
| `srs.createRelationType` | SRS: Create Relation Type | `relation-type create` (stdin JSON scaffold) |
| `srs.updateRelationType` | SRS: Update Relation Type | `relation-type update <id>` |
| `srs.deleteRelationType` | SRS: Delete Relation Type | `relation-type delete <id>` |

For `create`: open a pre-filled JSON editor with the required fields (`id` auto-generated client side via `crypto.randomUUID()`, `version: 1`, `createdAt: new Date().toISOString()`) and pipe it to `srs relation-type create`. Match the existing `srs.createRelation` pattern in `editCommands.ts`.

For `update` and `delete`: prompt for the definition ID via a quick-pick from `relation-type list`, then call the appropriate CLI command.

Add the three commands to `package.json` under `contributes.commands`.

### Acceptance criteria — Phase 2

- Relation creation flow calls `srs relation-type list` and shows installed definitions in a quick-pick (not a static list or free-text box)
- `SRS: Create Relation Type` command opens a JSON editor scaffold and pipes to `srs relation-type create`
- `SRS: Update Relation Type` and `SRS: Delete Relation Type` commands work against a test repo
- No TypeScript compiler errors (`npm run compile` from `srs-vscode/`)
- Existing `srs.createRelation` flow is not broken

---

## Verification

```bash
# Phase 1
cd srs && node scripts/validate-all.mjs

# Phase 2
cd srs-vscode && npm run compile
# Manual smoke test: open srs/srs/ as repo in VS Code, run SRS: Create Relation, confirm type picker shows 16 definitions

# Cross-check prior plan closure
cd srs-rust && cargo test && cargo clippy -- -D warnings
scripts/check-schema-drift.sh
```

---

## Files to Touch

| File | Change |
|---|---|
| `srs/srs/records/subsections/07-11-ext-recommended-relations.json` | Replace body field value |
| `srs/srs/records/subsections/09-1-core-conformance-requirements.json` | Append mandatory lookup bullet |
| `srs/srs/records/subsections/09-2-extension-conformance-requirements.json` | Append ext:recommended-relations retirement notice |
| `srs-vscode/src/commands/editCommands.ts` | Replace static relationType input with dynamic picker; add create/update/delete commands |
| `srs-vscode/package.json` | Register `srs.createRelationType`, `srs.updateRelationType`, `srs.deleteRelationType` commands |
| `plans/completed/rfc-005-relation-type-definitions.md` | Check the two remaining `validate-all.mjs` boxes |
