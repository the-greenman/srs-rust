# ADR-021: Opt-in batch write mode for RepositoryStore (deferred flush)

- **Status:** proposed
- **Date:** 2026-07-04
- **Supersedes:** —
- **Superseded by:** —

## Context

`JsonStore` writes the entire `.srsj` file to disk on every `save_*` call via `flush()`. During
`import_repository_snapshot` this means the file is rewritten once per record inserted. If the
import fails mid-stream, the `.srsj` file on disk reflects a logically inconsistent intermediate
state: partial record data is present but `manifest.instanceIndex` is empty (because
`save_manifest` was never called).

`FileStore` is unaffected: it writes individual files and relies on ADR-007's file-before-index
ordering for consistency. `MemoryStore` is always in-memory and consistent.

Two alternative fixes were considered:

1. **Make `import_repository_snapshot` aware of concrete store types** (e.g. downcast to
   `JsonStore` and call internal methods). Rejected: violates the storage-agnostic service
   contract established by ADR-008 and introduces tight coupling between the service layer and
   a storage adapter.

2. **Accumulate to `MemoryStore` first, then copy to target.** Rejected: doubles the copy work
   and obscures the intent; the existing service contract already separates export from import.

3. **Opt-in batch methods on `RepositoryStore` trait with default no-op impls.** Accepted: keeps
   the service layer storage-agnostic, allows `JsonStore` to opt in to deferred flushing without
   any other store changing behaviour, and is directly callable from `import_repository_snapshot`.

## Decision

`RepositoryStore` gains three methods with **default no-op implementations**:

```rust
fn begin_batch(&self) {}
fn commit_batch(&self) -> Result<(), RepositoryError> { Ok(()) }
fn abort_batch(&self) {}
```

`JsonStore` overrides them using an internal `batching: bool` flag on `JsonStoreState`:
- `begin_batch`: sets `batching = true`.
- `flush()`: returns `Ok(())` immediately when `batching` is `true` (state updates still happen
  in-memory as normal).
- `commit_batch`: sets `batching = false`, then calls `flush()` to write the accumulated state to
  disk in a single write.
- `abort_batch`: sets `batching = false`, then attempts to restore the in-memory `JsonStoreState`
  from the on-disk `.srsj` file (reads the file, deserialises `JsonStoreFile`, reconstructs
  `manifest` and `data`). If restoration succeeds, the in-memory state matches the on-disk file
  (which was not modified during the batch). **Silent-failure contract**: if the file read or
  deserialisation fails (e.g. the file was deleted between `begin_batch` and `abort_batch`), the
  in-memory state is left holding partial import data. Since `batching` is already cleared, a
  subsequent `flush()` call on the same instance would write that partial state to disk. Callers
  must treat an abort as terminal: propagate the import error and drop the store instance. For the
  WASM `<memory>` path, no restoration is possible; callers must not reuse a memory-backed store
  after `abort_batch`.

`import_repository_snapshot` (in `srs-repository`) calls `begin_batch()` before the import loop
and either `commit_batch()` on success or `abort_batch()` on error.

The WASM path (`from_srsj`/`to_srsj_string`) is unaffected: the `<memory>` sentinel path in
`flush()` already causes an early return before any I/O; `batching` is initialised to `false` and
the WASM usage pattern does not call `import_repository_snapshot`.

## Consequences

**Positive:**
- `import_repository_snapshot` into a `JsonStore` is now atomic at the application level: a
  mid-import failure leaves the `.srsj` file unchanged.
- No change to `srs-cli`, `srs-bindings`, `srs-core`, or any schema/payload contract.
- `FileStore` and `MemoryStore` are unaffected (default no-ops).
- The WASM path is unaffected (existing `<memory>` guard handles it).

**Negative / trade-offs:**
- Adding three methods to `RepositoryStore` is a public API surface that future store
  implementations must be aware of (though their default no-op impls mean no action is required
  unless they want batch behaviour).
- `commit_batch()` uses a single `std::fs::write` call (not write-then-rename). This is
  sufficient for application-level atomicity (partial import vs. consistent file) but does not
  protect against OS-level partial writes (power loss mid-write). Write-then-rename can be added
  in a future enhancement if needed.

**Neutral:**
- `batching` is an internal `JsonStoreState` field; it does not appear in the `.srsj` serialised
  format and has no effect on `to_srsj_string`.
- `MemoryStore::begin_batch`, `commit_batch`, `abort_batch` use the trait defaults (no-ops);
  `MemoryStore` is already always consistent.
