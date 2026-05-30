# ADR-008: Storage-Agnostic Repository Lifecycle and Full-Repository Portability

- **Status:** accepted
- **Date:** 2026-05-30
- **Supersedes:** —
- **Superseded by:** —

## Context

SRS service logic previously assumed file-backed repository setup for creation and portability workflows. That coupling blocks alternate storage adapters (for example, SQL) and forces lifecycle behavior into CLI/filesystem code. We need a contract where repository creation and full-repository copy remain stable service operations while allowing adapters to own backend representation details.

Two related capabilities are required:

1. Create a new repository through `RepositoryStore`, without service-level filesystem writes.
2. Copy a complete logical repository between store implementations (MemoryStore -> FileStore today, SQL tomorrow) without path translation hacks.

The project also needs a clear sequencing rule: repository lifecycle/storage foundations must land before package-management and container-boundary storage refactors that depend on shared lifecycle types and snapshot shape.

## Decision

- Repository creation is an adapter-owned lifecycle operation exposed through `RepositoryStore` (`repository_exists`, `initialize_repository`), called by repository services.
- Service and CLI layers must not directly create repository files/directories for lifecycle workflows.
- Full-repository portability uses a logical, path-free `RepositorySnapshot` DTO and service operations (`export_repository_snapshot`, `import_repository_snapshot`, `copy_repository`).
- FileStore preserves the existing filesystem layout, but that layout is an adapter detail, not part of service contracts.
- Import into a non-empty target is rejected by typed error unless an explicit future replace mode is introduced.
- Ordering rule: this lifecycle/portability foundation precedes storage-agnostic package-management and container-boundary plan work.

## Consequences

**Positive:**
- Repository lifecycle APIs are backend-neutral and reusable across CLI, bindings, and future adapters.
- MemoryStore, FileStore, and future SQL adapters can represent storage differently while preserving one service contract.
- `srs repo create` and `srs repo copy` become thin orchestration over repository services.

**Negative / trade-offs:**
- Snapshot import currently materializes canonical file-backed paths in FileStore, which is acceptable for parity but leaves room for richer path policy later.
- Additional adapter methods are required to persist complete package/relation data without leaking path traversal logic into services.

**Neutral:**
- This ADR does not introduce SQL storage itself.
- This ADR does not introduce async traits.
- Partial/sliced import-export (RFC-003 scope) remains separate from full-repository portability.
