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
| Fields/Types (`field`, `type`) | Defined | Implemented (definition management) |
| Extensions (`extension`) | Defined | Implemented command surface |
| Protocols (`protocol`) | Defined | Implemented command surface |
| Views L1/L2 (`ext:views-l1`, `ext:views-l2`) | RFC-001 in progress to acceptance in repo records | Not yet implemented in runtime package/model/render pipeline |
| Themes L1 (`ext:themes-l1`) | RFC-002 in progress to acceptance in repo records | Not implemented |
| Render command (`srs render ...`) | Planned | Not implemented |
| Repeatable field entries (`ext:repeatable-fields`) | In schema/spec | Implemented (typed model, validation constraints, rendering support) |
| Field groups (`ext:field-groups`) | In schema/spec | Implemented (typed model, required/group-count validation, rendering support) |
| Table value type | Mentioned in planning discussions | Not implemented (not in `ValueType` enum or field schemas) |

## Current CLI Surface

Top-level command groups currently available:

- `note`
- `repo`
- `migrate`
- `tag`
- `relation-type`
- `field`
- `type`
- `record`
- `relation`
- `extension`
- `protocol`
- `container`

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
- Implement L1/L2 view models + package loading
- Implement `render document-view`
- Decide and implement table-like value modeling (if kept in spec scope)

## Notes

- Architecture boundaries are documented in [ARCHITECTURE.md](ARCHITECTURE.md).
- Active implementation planning lives in [plans/](plans/).
