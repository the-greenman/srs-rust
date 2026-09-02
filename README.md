# srs-rust

Rust reference implementation of the **Semantic Record System (SRS)** — the runtime side of the [SemanticOps ecosystem](#the-ecosystem). It provides the canonical Rust types, repository loading/validation services, embedded schema validation, the machine-facing `srs` CLI, and the WASM bindings that browser clients consume.

The sibling [`srs`](../srs) repo is the spec / source-of-truth side (RFCs, JSON schemas, and the live SRS repository used as test fixtures). This repo *follows* the spec; it does not define it.

## The ecosystem

Expected local layout (each is an independent git repo under a shared parent):

```text
semanticops/
├── srs           spec text, RFCs, schema source, canonical SRS data (srs/srs)
├── srs-rust      this repo — Rust engine, srs CLI, WASM bindings
├── srs-vscode    VS Code extension (thin client over the srs CLI)
└── srs-web       governance web editor (thin client over the WASM bindings)
```

**Architecture is library-first** (ADR-001): every capability is built once as a typed service in `srs-repository`, then exposed through *both* the CLI and the WASM bindings as thin adapters. Clients add presentation only — never semantics. See [`docs/architecture/capability-layering.md`](docs/architecture/capability-layering.md) before implementing any new capability.

## Workspace layout (7 crates)

| Crate | Responsibility | Constraints |
|---|---|---|
| `srs-core` | Canonical SRS types, serde shapes, in-memory validation | No file I/O, no async, no `schemars` |
| `srs-schema` | Embedded JSON schemas (compile-time), mirror of `../srs/docs/schema/2.0/` | Read-only mirror |
| `srs-repository` | Repository loading/writing, package resolution, ~47 service modules — **all business logic** | Depends on `srs-core` |
| `srs-cli` | The `srs` binary — clap arg parsing + JSON envelope output | One service call per handler, no business logic |
| `srs-bindings` | JSON-first `wasm-bindgen` surface over the same repository services | No logic duplicated from the CLI |
| `srs-gov` | The `srs-gov` binary — governance-flow CLI + ratatui TUI composing `srs` verbs | Exploratory client |
| `srs-projection` | Placeholder for SQL/search/graph projections | No work until a consumer exists |

Roughly **82K LOC** of Rust, with `srs-repository` (~52K) the dominant crate. Additional binaries under `crates/srs-cli/src/bin/`: `generate-schemas` and `generate-governance-seed`.

## The `srs` CLI

Every command returns a JSON envelope — `{ "ok": true, "command": "...", "payload": { ... } }` — with diagnostics reported in a top-level `diagnostics[]` array. **Exit code `0` means the command ran, not that the data is valid** — check `payload.diagnostics` separately.

Command groups:

```
note  repo  migrate  tag  relation-type  field  type  record  relation
extension  protocol  blueprint  container  render  package  theme  view
composition  vocabulary  lifecycle  term  tree  find
```

Most groups are CRUD. Notable non-CRUD surface: `repo validate|map|diff|copy|extensions`, `container resolve-view` (root + ordered members + Composition column spec), `render composition`, `lifecycle` transitions, `tree`, and `find` (the discovery contract).

Global flags (accepted by all commands):

```
--repo <PATH>        explicit repository root (auto-detected from cwd if omitted)
--container <ID>     scope list/create/delete to a container's membership
--format json|yaml|text   output format (JSON is the stable contract)
--pretty             pretty-print output
```

See the live help for the authoritative list:

```bash
cargo run --bin srs -- --help
cargo run --bin srs -- <group> --help
```

## WASM bindings

`crates/srs-bindings` is a **real** `wasm-bindgen` surface (cdylib), not a placeholder — it wraps ~20 repository services (records, relations, containers, blueprints, discovery/find, render, navigation, lifecycle, migrate-identity, …) and is covered by integration tests under `crates/srs-bindings/tests/`. Every load path (`load` for `.srsj`, `load_archive` for `.srs`, `load_tree` for an exploded file tree) yields the same in-memory tree session — a `FileStore` over `MemVfs` (ADR-038) — and `export_tree` returns the tree with untouched files byte-identical, which is what makes clean git diffs possible for browser clients. It is built for `wasm32-unknown-unknown` in CI and published as `srs-bindings-web.tar.gz` on every merge to `master` (`release.yml`), which `srs-web` fetches at build time.

Build it locally against a `srs-web` checkout:

```bash
wasm-pack build crates/srs-bindings --target web --out-dir ../srs-web/src/lib/srs_bindings
```

## Install / run

```bash
cargo install --path crates/srs-cli     # install the `srs` binary
cargo run --bin srs -- --help           # or run without installing
cargo run --bin srs-gov -- --help       # governance-flow TUI/CLI
```

## Development

```bash
cargo build                                          # build all crates
cargo test                                           # run all tests (~1,400 tests)
cargo test -p srs-core                               # one crate
cargo clippy --all-targets --all-features -- -D warnings
cargo run --bin srs -- --repo ../srs/srs repo validate   # validate the live spec repo (0 errors)
cargo run --bin generate-schemas                     # regenerate payload JSON Schema golden files
```

After changing any struct in `crates/srs-cli/src/payload.rs`, run `generate-schemas` and commit the updated files under `crates/srs-cli/schemas/payload/` — the `payload_contracts` golden test and the pre-commit hook enforce this (ADR-011).

**CI** (`.github/workflows/ci.yml`, on `master`) runs four jobs: Test (checks out sibling `srs` as fixtures), Lint (clippy `-D warnings` + `fmt --check`), WASM Build, and Schema Drift.

## Schema sync

`crates/srs-schema/schemas/2.0/` is a **read-only mirror** of `../srs/docs/schema/2.0/`. Never edit it directly.

```bash
scripts/sync-schemas-from-spec.sh        # copy *.json + regenerate SHA256SUMS
scripts/check-schema-drift.sh ../srs     # verify (also the CI `schema-drift` job)
```

`srs-vscode/schemas/2.0/` is a second mirror with the same constraint — sync both when spec schemas change. See `CLAUDE.md` for the multi-repo merge order.

## Capability status (summary)

The full, current implementation/conformance status lives in [`docs/roadmap/extension-implementation.md`](docs/roadmap/extension-implementation.md). In brief:

- **Implemented** — notes, tags, records, relations, relation types (RFC-005 mandatory resolution), containers (+ `resolve-view`, `--container` scoping), fields (read + create), types (read-only via CLI), extensions, protocols, packages, blueprints, Views L1 & L2 / document views, repeatable fields, field groups, themes (`ext:themes-l1`), type inheritance, lifecycle transitions.
- **Not yet implemented** — `ext:addressability`, `ext:federation`, and the `table` value type.

## Documentation

- [`ARCHITECTURE.md`](ARCHITECTURE.md) — crate boundaries and design overview.
- [`docs/architecture/capability-layering.md`](docs/architecture/capability-layering.md) — where functionality belongs (required reading before adding a capability).
- [`docs/adr/`](docs/adr/) — 27 architecture decision records.
- [`docs/project-management.md`](docs/project-management.md) — the canonical issue/priority process (Project #5).
- [`plans/`](plans/) — active implementation and phase plans.
- [`CLAUDE.md`](CLAUDE.md) — contributor rules (crate authority, handler pattern, payload contract).
