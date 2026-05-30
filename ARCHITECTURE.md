# SRS Rust Architecture Rules

This file is written for humans and AI agents that need to understand the repo quickly.

## Authority Boundaries

- `srs-core` owns storage-independent SRS data structures, serialization shape, and validation.
- `srs-repository` owns repository loading, writing, indexing, deterministic analysis, and service functions.
- `srs-cli` owns argument parsing, repository path resolution, and JSON envelope printing only.
- `srs-bindings` must call library services. It must not duplicate CLI behavior or business logic.
- `srs-projection` owns rendering/export projection logic.

## Policy And Configuration

- Do not hardcode repository-specific semantic policy in the CLI.
- Do not use tags as formal ontology or hidden command policy.
- Named analysis behavior must come from repository data or explicit command input.
- The library may define generic config shapes and deterministic algorithms, but not project-specific tag sets.
- AI-facing guidance is data. Semantic decisions are not made by the CLI or library.

## Tags

- Tags are weak discovery labels.
- Tags are not semantic claims.
- Relations and typed records carry stable semantic meaning.
- If a command needs a tag set, the tag set belongs in a named profile or explicit input, not in command code.

## Determinism

- CLI and library analysis must be read-only unless the command is explicitly a write command.
- Analysis output must be stable JSON suitable for AI handoff.
- The CLI/library may assemble evidence, counts, paths, indexes, and profile matches.
- The CLI/library must not infer migrations, extract meaning, or decide semantic upgrades.

## DRY Rule

- Essential logic belongs in a library crate once.
- CLI commands should be thin wrappers over library services.
- Python bindings should expose the same library services as JSON-first calls.
- When logic is needed by both CLI and bindings, move it down before adding another caller.

## Storage Direction

- The current repository implementation is file-backed and synchronous.
- Do not introduce async traits until there is a concrete async consumer.
- Do not split a new file-adapter crate until a second storage adapter creates real pressure.
- Keep storage boundaries visible so a database-backed implementation can be introduced later.

## Repository Lifecycle And Portability

- Repository creation is a repository service request executed by `RepositoryStore` adapters.
- Service code must not create repository directories/files directly; adapters own backend details.
- Full repository copy is a logical operation using a backend-neutral `RepositorySnapshot`.
- CLI `repo copy` must call portability services, not perform filesystem copy operations.
- File paths (`.srs/`, `manifest.json`, `package/package.json`) are FileStore implementation details, not service contracts.
- Future SQL-backed adapters may use table storage while preserving the same service API.
- Ordering constraint: repository lifecycle/portability foundation lands before package-management and container-boundary storage plans.

## Package Boundaries

- A package is a logical definition boundary, not a filesystem directory shape.
- Services address packages through package IDs/namespaces, not filesystem paths.
- Raw `package.json` paths and package file index arrays are FileStore implementation details; they must not leak into service API signatures.
- `load_package()` returns the merged effective view across all declared package boundaries. Services that need the merged view call this.
- `create_package` registers a new package boundary in the manifest and creates the package skeleton. It must not accept a filesystem path as its primary identifier.
- Package create/update/delete operations must be callable through `&dyn RepositoryStore` without `std::fs`.
- A future SQL adapter must be able to implement package boundaries as table rows without changing service APIs.

## Store Matrix Testing

- Storage-boundary behavior must be validated against multiple concrete stores, not only one adapter.
- `srs-repository` tests for lifecycle and portability must include both `FileStore` and `JsonStore` paths.
- New repository service features should add at least one cross-store roundtrip test (for example: memory -> json -> file).
