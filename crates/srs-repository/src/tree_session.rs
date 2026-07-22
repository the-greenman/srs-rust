//! Tree sessions — open, export, and materialize in-memory file trees
//! (ADR-038).
//!
//! A tree session is a [`FileStore`] over a [`MemVfs`]: the exact store the
//! CLI runs on disk, serving an in-memory path→bytes map. Untouched entries
//! are never rewritten, so `open_tree` → `export_tree` round-trips
//! byte-identically — the clean-git-diff guarantee Epic 10 (muDemocracy.org#101)
//! is built on. Unknown files (README, CI config) ride through unchanged.

use crate::error::RepositoryError;
use crate::repository_portability::{
    export_repository_snapshot_with_options, import_repository_snapshot, ExportSnapshotOptions,
};
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
pub fn open_tree(files: BTreeMap<String, Vec<u8>>) -> Result<FileStore, RepositoryError> {
    if !files.contains_key("manifest.json") {
        return Err(RepositoryError::ManifestMissing {
            path: PathBuf::from("manifest.json"),
        });
    }
    let vfs = Rc::new(MemVfs::from_map(files));
    vfs.create_dir_all(SRS_MARKER_DIR)?;
    Ok(FileStore::from_vfs(vfs))
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
/// The bridge from codec-loaded sources (a `JsonStore` `.srsj` session, a
/// legacy snapshot archive) into the operational tree model: snapshot export
/// (with content blobs, ADR-031) → snapshot import into a MemVfs-backed
/// `FileStore` (canonical paths, ADR-008 contract).
pub fn materialize_tree(source: &dyn RepositoryStore) -> Result<FileStore, RepositoryError> {
    let snapshot = export_repository_snapshot_with_options(
        source,
        ExportSnapshotOptions {
            include_content_blobs: true,
        },
    )?;
    let store = FileStore::from_vfs(Rc::new(MemVfs::new()));
    import_repository_snapshot(&store, &snapshot)?;
    Ok(store)
}
