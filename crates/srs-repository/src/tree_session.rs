//! Tree sessions — open, export, and materialize in-memory file trees
//! (ADR-038).
//!
//! A tree session is a [`FileStore`] over a [`MemVfs`]: the exact store the
//! CLI runs on disk, serving an in-memory path→bytes map. Untouched entries
//! are never rewritten, so `open_tree` → `export_tree` round-trips
//! byte-identically — the clean-git-diff guarantee Epic 10 (muDemocracy.org#101)
//! is built on. Unknown files (README, CI config) ride through unchanged.

use crate::error::RepositoryError;
use crate::store::{FileStore, RepositoryStore};
use crate::vfs::{MemVfs, Vfs, SRS_MARKER_DIR};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

/// Open an exploded repository from an in-memory file tree.
///
/// `files` maps repo-relative forward-slash paths to raw bytes (e.g. every
/// blob of a git tree). Errors when `manifest.json` is absent — the map is
/// not an SRS repository root. The `.srs/` marker directory is synthesized
/// when missing: git cannot track empty directories, so fetched trees
/// usually lack it.
///
/// This is the ingestion boundary for every untrusted tree — a `.srsj`
/// document, a `.srs` archive, a fetched git tree — so every key is checked to
/// resolve inside the repository root before it can reach a store that may
/// later materialise onto a real filesystem.
pub fn open_tree(files: BTreeMap<String, Vec<u8>>) -> Result<FileStore, RepositoryError> {
    if !files.contains_key("manifest.json") {
        return Err(RepositoryError::ManifestMissing {
            path: PathBuf::from("manifest.json"),
        });
    }
    for path in files.keys() {
        crate::vfs::ensure_contained(path)?;
    }
    let vfs = Rc::new(MemVfs::from_map(files));
    vfs.create_dir_all(SRS_MARKER_DIR)?;
    Ok(FileStore::from_vfs(vfs))
}

/// A fresh, empty tree session.
///
/// The starting point for a repository that does not exist yet — `repo create`
/// on a `.srsj` target, a snapshot import target, a test double. Unlike
/// [`open_tree`] it requires no `manifest.json`, because the caller is about to
/// write one.
pub fn new_tree_session() -> FileStore {
    FileStore::from_vfs(Rc::new(MemVfs::new()))
}

/// Export a tree session as a path→bytes map — the raw `MemVfs` snapshot.
///
/// Emits `.srs/.gitkeep` (0-byte) when no file under `.srs/` exists, so the
/// exported tree stays clone-detectable on git hosts. Errors for disk-backed
/// stores: those export via the filesystem, not this API.
pub fn export_tree(store: &FileStore) -> Result<BTreeMap<String, Vec<u8>>, RepositoryError> {
    let mut map =
        store
            .vfs()
            .as_mem_snapshot()
            .ok_or_else(|| RepositoryError::InvalidSnapshotData {
                message: "export_tree requires a memory-backed tree session \
                      (disk repositories already are their own file tree)"
                    .to_string(),
            })?;
    let marker_prefix = format!("{SRS_MARKER_DIR}/");
    if !map.keys().any(|k| k.starts_with(&marker_prefix)) {
        map.insert(format!("{SRS_MARKER_DIR}/.gitkeep"), Vec::new());
    }
    Ok(map)
}

/// Materialize any repository into a fresh in-memory tree session.
///
/// The bridge from codec-loaded sources (a `FileStore` `.srsj` session, a legacy snapshot
/// archive) into the operational tree model. It reproduces the source's **real** file tree
/// faithfully — the same authoritative enumeration `archive_pack` uses
/// (`archive::tree_entries`) — then opens it as a MemVfs-backed `FileStore`.
///
/// Crucially it does **not** re-canonicalize instance paths (the old snapshot round-trip did).
/// Re-canonicalization collapsed deterministic-UUID siblings that share an 8-hex-char id prefix
/// onto one path — crashing valid repositories on load — and pre-normalized paths so the
/// `repo-upgrade` migration could no longer detect a repository as needing normalization
/// (srs-rust#696). Keeping real paths honors ADR-038's stated goal ("the operational tree keeps
/// real paths") and the ADR-039 byte-faithful guarantee (ADR-040).
pub fn materialize_tree(source: &dyn RepositoryStore) -> Result<FileStore, RepositoryError> {
    open_tree(crate::archive::tree_entries(source)?)
}
