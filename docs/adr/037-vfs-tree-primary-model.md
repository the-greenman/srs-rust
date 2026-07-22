# ADR-037: Vfs seam — the in-memory file tree is the primary operational model

- **Status:** proposed
- **Date:** 2026-07-22
- **Supersedes:** ADR-013 (bundle-format rationale only — the "rejected alternative" of a
  `{path: content}` map); amends ADR-015 ("the WASM `SrsRepository` wraps a `JsonStore`")
- **Superseded by:** —

## Context

Epic 10 (muDemocracy.org#101) requires srs-web to open an exploded SRS repository from a git
host, edit it in the browser, and write all edits back as **one commit with clean per-file
diffs** — only genuinely-changed files may appear in the diff, and unknown files (README, CI
config) must ride through untouched.

The `JsonStore`-backed WASM session (ADR-013/ADR-015) cannot deliver this:

- `JsonStore` holds parsed `serde_json::Value`s; original raw bytes of loaded files are not
  preserved, so re-serialization rewrites untouched files.
- The snapshot pipeline (`export_repository_snapshot` → `import_repository_snapshot`)
  re-canonicalizes instance and definition paths and synthesizes `package/package.json`.
- `archive_pack` never emits per-definition files — a browser edit to a type or field would
  strand stale `package/fields|types/*.json` on the git host, corrupting the repo.

ADR-013 rejected "passing a raw multi-file structure (a map of `{path: content}`)" because it
"would require a new serialisation contract and a new store implementation". That trade-off has
inverted: the multi-file structure **is the product requirement**, and the store implementation
already exists — `FileStore` is the CLI's store and defines the on-disk semantics a git-hosted
exploded repo already matches. Owner decision (2026-07-22): the in-memory VFS tree is the
**primary operational model**, especially for web. Import/export formats (`.srsj`, `.srs`,
tree) are codecs at the boundary and say nothing about how data is managed in memory.

## Decision

1. **`Vfs` trait** (`crates/srs-repository/src/vfs.rs`): a minimal filesystem seam speaking
   repo-relative forward-slash paths (`read_bytes`, `read_to_string`, `write`, `remove`,
   `exists`, `is_dir`, `byte_len`, `list_dir`, `list_recursive`, `create_dir_all`,
   path-escape guard). `Vfs: std::fmt::Debug` (the store derives `Debug` over `Rc<dyn Vfs>`).
   `as_mem_snapshot(&self) -> Option<BTreeMap<String, Vec<u8>>>` is the Mem/Disk
   discriminator (Some only for `MemVfs`) used by `export_tree` and the archive pack branch —
   no `Any` downcasting. Errors preserve `std::io::ErrorKind::NotFound` so
   `RepositoryError::is_not_found()` semantics are unchanged. The `.srs` marker literal lives
   once as `SRS_MARKER_DIR`, shared by the store's `repository_exists` check and
   `tree_session`.
2. **`DiskVfs`** absorbs `FileStore`'s root-join and `canonicalize()`-based escape checking;
   **`MemVfs`** is `RefCell<BTreeMap<String, Vec<u8>>>` (+ explicit-dir set) with lexical
   escape checking. `BTreeMap` keeps iteration deterministic per ADR-017's reasoning.
3. **`FileStore { repo_root: PathBuf, vfs: Rc<dyn Vfs> }`** — one store implementation serves
   disk (CLI) and memory (WASM). `repo_root` is display-only (it feeds `repository_root()`;
   MemVfs-backed stores use the `"<memory>"` sentinel per ADR-021); all I/O goes through
   `vfs`. `FileStore::new(root)` keeps its signature; `FileStore::from_vfs` is the in-memory
   constructor. The refactor is behavior-neutral for disk.
4. **Tree sessions** (`tree_session.rs`): `open_tree(BTreeMap<String, Vec<u8>>) -> FileStore`
   (synthesizes the `.srs/` marker dir — git cannot track empty directories),
   `export_tree(&FileStore) -> BTreeMap<String, Vec<u8>>` (raw map dump; emits
   `.srs/.gitkeep` iff no `.srs/` file exists), and
   `materialize_tree(&dyn RepositoryStore) -> FileStore` (snapshot round-trip — the bridge
   from any codec-loaded source into the operational tree).
5. **All WASM load paths produce a MemVfs-backed `FileStore`**: `load(srsj)` goes
   `load_from_srsj` (codec + migrations) → `materialize_tree`; `load_archive` unzips to a
   tree; `load_tree` opens the tree directly. `SrsRepository` wraps `FileStore`, not
   `JsonStore`.
6. **`JsonStore` is demoted to a `.srsj` codec.** It remains the CLI's operational store for
   `--repo <file>.srsj` sessions until a filed follow-up migrates the CLI onto the tree
   model; in the bindings it appears only inside `load`/`export_srsj` as the format
   implementation.
7. Untouched files round-trip **byte-identically** by construction: the session store never
   rewrites a file it was not asked to write. This is the clean-diff guarantee, enforced by
   the `open_export_roundtrip_byte_identical` test.

## Consequences

**Positive:**
- CLI parity by construction — browser sessions run the exact store the CLI runs; one
  semantics everywhere (capability layering).
- Clean git diffs: only files a service actually wrote differ; unknown files ride through.
- Per-definition files are first-class in the browser (edits reach the host correctly).
- Attachments/binary source-documents work in tree- and archive-loaded sessions via
  `load_binary_file` (previously JsonStore's in-memory-only `binary_files`).
- The Vfs seam is reusable (future read-only stores, overlay stores, test fixtures).

**Negative / trade-offs:**
- `export_srsj` becomes a projection (snapshot → codec) and re-canonicalizes paths; `.srsj`
  output is no longer byte-continuous with prior sessions. Accepted: `.srsj` is interchange,
  not session state, and the owner has waived backwards compatibility.
- `Rc<dyn Vfs>` makes `FileStore` single-threaded-clone; acceptable — no threading exists in
  the workspace, and `JsonStore` already set the `RefCell` precedent.
- ~39 non-test fs call sites in `store.rs` are touched by the refactor (mechanical risk,
  mitigated by the behavior-neutral gate: full suite green before any functional change
  lands; `#[cfg(test)]` fixture builders stay on `std::fs` deliberately).

**Neutral:**
- `FileStore`/`DiskVfs` compile on wasm32 today (dead code unless referenced); MemVfs is the
  only Vfs constructed in bindings.
- The snapshot machinery (`export_repository_snapshot` / `import_repository_snapshot`)
  remains unchanged — it is the codec bridge and the RFC-014 portability engine.
