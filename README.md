# srs-rust

The reference engine for SRS: one core behind a CLI, WebAssembly bindings and an MCP server.

SRS (pronounced "source") is an open standard for portable semantic documents that people and AI can both understand and use. [semanticops.com](https://semanticops.com) explains it (agents start at [`/llms.txt`](https://semanticops.com/llms.txt)); [srs.semanticops.com](https://srs.semanticops.com) hosts the specification and schemas.

This repository is a library-first reference implementation. The core (`srs-core`) holds types and validation, and one repository service (`srs-repository`) sits over it. That service is exposed as a CLI with a stable JSON contract, as WASM bindings and as an MCP server. It *follows* the specification in [`srs`](https://github.com/the-greenman/srs) (RFCs, JSON schemas, and the live SRS repository used as test fixtures); it does not define it.

## The SemanticOps projects

| Project | Kind | In one line |
|---|---|---|
| [srs](https://github.com/the-greenman/srs) | Open standard | The specification, authored as its own data. |
| [srs-rust](https://github.com/the-greenman/srs-rust) (this repo) | Reference engine | One core behind a CLI, WebAssembly bindings and an MCP server. |
| [srs-web](https://github.com/the-greenman/srs-web) | Browser editor | Edit SRS repositories entirely client-side, on storage you own. |
| [srs-vscode](https://github.com/the-greenman/srs-vscode) | VS Code extension | Repositories in your workspace, with views for navigating them. |

muDemocracy is the first consumer of SRS, covering decision practice. See [semanticops.com/projects](https://semanticops.com/projects/) for each project in context.

**Architecture is library-first** (ADR-001): every capability is built once as a typed service in `srs-repository`, then exposed through the CLI, the WASM bindings and the MCP server as thin adapters. Clients add presentation only, never semantics. See [`docs/architecture/capability-layering.md`](docs/architecture/capability-layering.md) before implementing any new capability.

## Workspace layout

| Crate | Responsibility | Constraints |
|---|---|---|
| `srs-core` | Canonical SRS types, serde shapes, in-memory validation | No file I/O, no async, no `schemars` |
| `srs-schema` | Embedded JSON schemas (compile-time), mirror of [`srs/docs/schema/2.0/`](https://github.com/the-greenman/srs/tree/master/docs/schema/2.0) | Read-only mirror |
| `srs-repository` | Repository loading/writing, package resolution, the service modules: **all business logic** | Depends on `srs-core` |
| `srs-cli` | The `srs` binary: clap arg parsing + JSON envelope output | One service call per handler, no business logic |
| `srs-bindings` | JSON-first `wasm-bindgen` surface over the same repository services | No logic duplicated from the CLI |
| `srs-mcp-core` | Transport-agnostic MCP application core: tool catalogue, resources, prompts, JSON-RPC dispatch | No `rmcp`, Tokio or file paths |
| `srs-mcp` | The native stdio MCP adapter, shipped inside the binary as `srs mcp serve` | Sole owner of `rmcp`/`tokio`; no tool semantics |
| `srs-gov` | The `srs-gov` binary: governance-flow CLI + ratatui TUI composing `srs` verbs | Exploratory client |
| `srs-projection` | Placeholder for further projections | No work until a consumer exists |

Additional binaries under `crates/srs-cli/src/bin/`: `generate-schemas` and `generate-governance-seed`.

## The `srs` CLI

Every command returns a JSON envelope, `{ "ok": true, "command": "...", "payload": { ... } }`, with diagnostics reported in a top-level `diagnostics[]` array. **Exit code `0` means the command ran, not that the data is valid**: check `payload.diagnostics` separately.

Command groups:

```
note  schema  repo  migrate  tag  relation-type  field  type  record  relation
protocol  blueprint  container  render  package  theme  view  composition
vocabulary  lifecycle  term  mcp  tree  find  registry  context  attachment
archive  slice
```

Most groups are CRUD. Notable non-CRUD surface: `repo validate|map|diff|copy|extensions|doctor`, `container resolve-view` (root, ordered members and the Composition column spec), `render composition`, `record transition` (lifecycle state changes), `tree`, `find` (the discovery contract), `context` (field and record context for agents), `archive pack|unpack` (the `.srs` working format), `slice export` (one container as a standalone `.srs`) and `mcp serve`.

Global flags (accepted by all commands):

```
--repo <PATH>        explicit repository root (auto-detected from cwd if omitted)
--container <ID>     scope list/create/delete to a container's membership
--format json|yaml|text   output format (JSON is the stable contract)
--store <BACKEND>    storage backend override (inferred from the repository location by default)
--actor <JSON>       session actor for what this invocation creates (also SRS_ACTOR)
--pretty             pretty-print output
```

See the live help for the authoritative list:

```bash
cargo run --bin srs -- --help
cargo run --bin srs -- <group> --help
```

## MCP server

`srs mcp serve` runs a stdio [Model Context Protocol](https://modelcontextprotocol.io) server over one repository, so any MCP client (Claude Code, Cursor, Copilot, Goose and others) can read it and write to it with the same validation as the CLI. It exposes resources (map, navigation, records, containers, rendered Compositions, type schemas, relation types) and validated write tools (`record_create`, `record_update`, `record_transition`, `relation_create`, `note_create`, `container_member_add`, protocol run tools, and more), plus `find` and `repo_validate`. A rejected write returns the service diagnostics and writes nothing.

```json
{ "mcpServers": { "my-srs-repo": { "command": "srs", "args": ["mcp", "serve", "--repo", "/absolute/path/to/repo"] } } }
```

The tool and resource catalogue, and its limits, are in [`crates/srs-mcp/README.md`](crates/srs-mcp/README.md); the design is [ADR-037](docs/adr/037-mcp-adapter-surface.md). The tool and resource semantics live once in `srs-mcp-core`, which the WASM bindings reuse for a browser-side MCP session.

## WASM bindings

`crates/srs-bindings` is a **real** `wasm-bindgen` surface (cdylib), not a placeholder. It wraps the repository services (records, relations, containers, blueprints, discovery/find, render, navigation, lifecycle, migrate-identity, `.srspkg` package export/install (ADR-050), RFC-026 container slice export `export_slice` (ADR-051), and an MCP session over an open store) and is covered by integration tests under `crates/srs-bindings/tests/`. Every load path (`load` for `.srsj`, `load_archive` for `.srs`, `load_tree` for an exploded file tree) yields the same in-memory tree session, a `FileStore` over `MemVfs` (ADR-038), and `export_tree` returns the tree with untouched files byte-identical, which is what makes clean git diffs possible for browser clients. It is built for `wasm32-unknown-unknown` in CI and published as `srs-bindings-web.tar.gz` on every merge to `master` (`release.yml`), which [`srs-web`](https://github.com/the-greenman/srs-web) fetches at build time.

Build it locally against a `srs-web` checkout:

```bash
wasm-pack build crates/srs-bindings --target web --out-dir ../srs-web/src/lib/srs_bindings
```

## Install / run

Prebuilt `srs` binaries are attached to each [release](https://github.com/the-greenman/srs-rust/releases) (`srs-x86_64-unknown-linux-gnu.tar.gz`). To build from source:

```bash
cargo install --path crates/srs-cli     # install the `srs` binary
cargo run --bin srs -- --help           # or run without installing
cargo run --bin srs-gov -- --help       # governance-flow TUI/CLI
```

## Development

```bash
cargo build                                          # build all crates
cargo test                                           # run all tests
cargo test -p srs-core                               # one crate
cargo clippy --all-targets --all-features -- -D warnings
cargo run --bin srs -- --repo path/to/srs/srs repo validate   # validate the spec repo (a checkout of srs)
cargo run --bin generate-schemas                     # regenerate payload JSON Schema golden files
```

After changing any struct in `crates/srs-cli/src/payload.rs`, run `generate-schemas` and commit the updated files under `crates/srs-cli/schemas/payload/`: the `payload_contracts` golden test and the pre-commit hook enforce this (ADR-011).

**CI** (`.github/workflows/ci.yml`, on `master`) runs four jobs: Test (checks out [`srs`](https://github.com/the-greenman/srs) as fixtures), Lint (clippy `-D warnings` + `fmt --check`), WASM Build, and Schema Drift.

## Schema sync

`crates/srs-schema/schemas/2.0/` is a **read-only mirror** of [`srs/docs/schema/2.0/`](https://github.com/the-greenman/srs/tree/master/docs/schema/2.0). Never edit it directly.

```bash
scripts/sync-schemas-from-spec.sh        # copy *.json from the srs release asset + regenerate SHA256SUMS
scripts/check-schema-drift.sh path/to/srs   # verify (also the CI `schema-drift` job)
```

[`srs-vscode`](https://github.com/the-greenman/srs-vscode) keeps a second mirror with the same constraint: sync both when spec schemas change. See `CLAUDE.md` for the merge order.

## Capability status (summary)

The implementation is checked against the spec by `srs repo extensions conformance`, and the CLI help is the live surface. In brief, the engine implements:

- **Content and structure:** notes, tags, records (generic, typed by Type, with lifecycle transitions and successors), relations and relation types (RFC-005 mandatory resolution), containers (`resolve-view`, `--container` scoping, copy, slice export), fields and types (create, update, delete, JSON Schema projection), and type inheritance.
- **Definitions and packaging:** packages (create, import, install, upgrade, export as `.srspkg`, dependencies), blueprints, protocols (definitions and runs), vocabularies, themes (`ext:themes-l1`), and Views (`ext:views-l1`, including the `table` composite renderer) and Compositions (`ext:views-l2`), rendered with `render composition` and declared as Presentations with `repo presentation`.
- **Operation:** discovery (`find`), context queries (`context`), validation, migrations, `.srs` archives and `.srsj` carriers, and the MCP server.

Partial or behind the spec: `ext:addressability` (context queries exist, the full model does not) and `ext:import-tracking`. The status table in [`docs/roadmap/extension-implementation.md`](docs/roadmap/extension-implementation.md) is a dated snapshot that lags the code, so trust the CLI help and `repo extensions conformance` over it.

## Documentation

- [`ARCHITECTURE.md`](ARCHITECTURE.md): crate boundaries and design overview.
- [`docs/architecture/capability-layering.md`](docs/architecture/capability-layering.md): where functionality belongs (required reading before adding a capability).
- [`docs/adr/`](docs/adr/): the architecture decision records.
- [`docs/project-management.md`](docs/project-management.md): the canonical issue/priority process (Project #5).
- [`plans/`](plans/): active implementation and phase plans.
- [`CLAUDE.md`](CLAUDE.md): contributor rules (crate authority, handler pattern, payload contract).

## Licence

The SRS Rust reference implementation and WASM bindings are released under your choice of [MIT](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE).

Contributions to this repository are made under the terms of the [Developer Certificate of Origin](CONTRIBUTING.md#developer-certificate-of-origin). By submitting a pull request, you certify that you have the right to submit that work under the Apache License 2.0 and/or MIT by signing off your commits with `git commit -s`.
