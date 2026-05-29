# srs-rust

Rust workspace for the Semantic Record System implementation.

This repo is the runtime implementation side of SRS. The sibling [`srs`](../srs) repo is the spec/source-of-truth side (RFCs, schemas, and the live SRS repository used in tests).

## Repo Relationship

Expected local layout:

```text
semanticops/
├── srs
└── srs-rust
```

- `../srs`: spec text, RFCs, schema source, canonical SRS repository data (`srs/srs`)
- `./srs-rust`: Rust types, services, CLI, and embedded schema validation

## Workspace Layout

- `crates/srs-core` — core SRS types + validation
- `crates/srs-schema` — embedded JSON schemas
- `crates/srs-repository` — repository loading/writing/services/validation
- `crates/srs-cli` — `srs` CLI binary
- `crates/srs-projection` — projection/export crate (early stage)
- `crates/srs-bindings` — bindings support
- `plans/` — implementation plans and phase docs

## Spec To Implementation Map (Current)

As of 2026-05-29.

| Area | Spec Status | Rust Implementation Status |
|---|---|---|
| Notes (`note`) | Defined and stable | Implemented (CRUD, tagging, audits) |
| Tags (`tag`) | Defined and stable | Implemented (CRUD) |
| Records (`record`) | Defined and stable | Implemented (CRUD with type validation) |
| Relations (`relation`) | Defined and stable | Implemented (CRUD + validation paths) |
| Relation Types (`relation-type`) | RFC-005 aligned | Implemented (status lifecycle + resolver behavior) |
| Containers (`container`) | Defined + invariants | Implemented (CRUD, members, roots, invariant validation, `--container` scoping for list/create/delete on note/tag/record) |
| Fields (`field`) | Defined | Implemented (list, get, create — update/delete not exposed) |
| Types (`type`) | Defined | Read-only via CLI (list, get); authoring is via package files |
| Extensions (`extension`) | Defined | Implemented (CRUD) |
| Protocols (`protocol`) | Defined | Implemented (CRUD, validation, stages, import/export) |
| Package refs (`srs package`) | Defined | Implemented (list, enable, disable — with scope + existence validation) |
| Views L1 (`ext:views-l1`) | RFC-001 acceptance in progress | Implemented in model + render pipeline; no CLI CRUD (views are package-defined) |
| Views L2 / Document Views (`ext:views-l2`) | RFC-001 acceptance in progress | Implemented — `srs render document-view` works; section sourcing via TypeQuery/RelationQuery/FixedInstances/ContainerSubset; repeatable fields and field groups rendered |
| Repeatable field entries (`ext:repeatable-fields`) | In schema/spec | Implemented (typed model, validation constraints, rendering) |
| Field groups (`ext:field-groups`) | In schema/spec | Implemented (typed model, required/group-count validation, rendering) |
| Lifecycle state machine (`ext:lifecycle`) | In progress | Stub — `lifecycleState` field parsed but not enforced; no state transition logic |
| Type inheritance (`ext:type-inheritance`) | In planning | Not implemented — `fieldOrder` present in model but ignored at render time |
| Themes (`ext:themes-l1`) | RFC-002 in progress | Not implemented |
| Addressability (`ext:addressability`) | Declared | Not implemented |
| Recommended relations (`ext:recommended-relations`) | Declared | Not implemented |
| Federation (`ext:federation`) | Not declared | Not implemented |
| Subsection nesting in renders | In spec (via relations) | Not implemented — renders are one level deep (section → records); subsection-of relation traversal not implemented |
| Table value type | Mentioned in planning | Not implemented (not in `ValueType` enum or field schemas) |

## Current CLI Surface

Top-level command groups currently available:

- `note` — CRUD, tag management, audits
- `repo` — validate, map, extensions list/enable/disable
- `migrate` — packet
- `tag` — CRUD
- `relation-type` — list, get
- `field` — list, get, create
- `type` — list, get
- `record` — CRUD
- `relation` — CRUD
- `extension` — CRUD
- `protocol` — CRUD, validation, stages, import/export
- `container` — CRUD, members, roots, validate
- `package` — list, enable, disable
- `render` — `document-view` (render to stdout or `--output <file>`)

Global flags:

- `--repo <path>`: explicit repository root
- `--container <container-id>`: scope boundary for list/create/delete on note/tag/record
- `--format json|text`: JSON is fully supported; text is currently planned/diagnostic-only
- `--pretty`: pretty JSON output

Check current command help:

```bash
cargo run -p srs -- --help
```

## Install / Run

Install CLI:

```bash
cargo install --path crates/srs-cli
```

Run without install:

```bash
cargo run -p srs -- --help
```

## Development Workflow

Run full tests:

```bash
cargo test
```

Run lints:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Validate live SRS repo data:

```bash
cargo run -p srs -- --repo ../srs/srs repo validate
```

## Schema Sync

Source-of-truth schema files are in `../srs/docs/schema/2.0/`.

Sync into embedded Rust schema crate:

```bash
scripts/sync-schemas-from-spec.sh
scripts/check-schema-drift.sh
```

## Near-Term Roadmap

- Land RFC-001/RFC-002 record acceptance updates in `../srs/srs`
- Implement subsection nesting in document view renders (traverse subsection-of relations to produce multi-level hierarchy)
- Implement lifecycle state enforcement (`ext:lifecycle` state machine)
- Decide and implement table-like value modeling (if kept in spec scope)

## Notes

- Architecture boundaries are documented in [ARCHITECTURE.md](ARCHITECTURE.md).
- Active implementation planning lives in [plans/](plans/).
