# srs-rust

Rust workspace for the Semantic Record System CLI, repository services, core types, and embedded schema artifacts.

This repo is the implementation side of SRS. The sibling [`srs`](../srs) repo holds the spec content, schema source files, and the live SRS definition repository used by tests and local development.

## Workspace Layout

- `crates/srs-core` — core Rust types and validation logic
- `crates/srs-schema` — embedded JSON schemas
- `crates/srs-repository` — repository loading, writing, validation, and services
- `crates/srs-cli` — the `srs` command-line binary
- `crates/srs-projection` — projection support
- `crates/srs-bindings` — bindings/support crate
- `scripts/` — schema sync and drift-check helpers
- `plans/` — implementation plans and design notes

## Requirements

- Rust toolchain with `cargo`
- Node.js for spec-side validation scripts
- A local checkout of the sibling spec repo at `../srs`

Expected local layout:

```text
semanticops/
├── srs
└── srs-rust
```

## Install The CLI

From the repo root:

```bash
cd /home/greenman/dev/semanticops/srs-rust
cargo install --path crates/srs-cli
```

This installs the `srs` binary into Cargo's bin directory, typically `~/.cargo/bin`.

For local development without installing:

```bash
cargo run -p srs -- --help
```

## Current CLI Surface

The current CLI is JSON-first and aimed at machine-facing workflows.

Available command groups:

```bash
srs note list [--repo <path>] [--tag <tag>]
srs note get <id> [--repo <path>]
srs note create [--repo <path>]
srs note tag <id> <tag> [--repo <path>]
srs note audit-tags [--repo <path>]
srs note foundations [--repo <path>]

srs tag list [--repo <path>] [--role <role>]
srs tag get <id> [--repo <path>]
srs tag create [--repo <path>]

srs repo map [--repo <path>]
srs repo validate [--repo <path>]

srs migrate packet [--repo <path>] [--foundation]

srs relation-type list [--repo <path>] [--status active|deprecated|tombstone|retired]
srs relation-type get <id> [--repo <path>]
```

If `--repo` is omitted, the CLI tries to detect the repository root from the current working directory.

## Common Commands

Validate the live SRS definition repository:

```bash
./target/debug/srs repo validate --repo ../srs/srs
```

List installed relation type definitions from the live SRS package:

```bash
./target/debug/srs relation-type list --repo ../srs/srs
```

Run the CLI package tests:

```bash
cargo test -p srs
```

## Development Workflow

Run the main Rust test suites:

```bash
cargo test -p srs-core
cargo test -p srs-repository
cargo test -p srs
```

Run formatting and linting:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
```

Validate the spec-side package and records:

```bash
cd ../srs
node scripts/validate-all.mjs
```

## Schema Sync

The source-of-truth schema files live in the sibling `srs` repo under `docs/schema/2.0/`.

To sync them into the embedded Rust schema crate:

```bash
cd /home/greenman/dev/semanticops/srs-rust
scripts/sync-schemas-from-spec.sh
scripts/check-schema-drift.sh
```

## Notes

- This repo currently ships a JSON-first CLI. Human-readable text formatting is planned separately.
- The CLI plan in [`plans/srs-cli-command-structure.md`](plans/srs-cli-command-structure.md) is broader than the currently implemented surface.
- Relation type validation from RFC-005 is now implemented in the Rust stack and exercised against the live SRS repo.
