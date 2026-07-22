//! Filesystem seam for [`crate::store::FileStore`] (ADR-038).
//!
//! A [`Vfs`] speaks **repo-relative, forward-slash** paths. `DiskVfs` resolves
//! them against a root directory on the real filesystem (the CLI path);
//! `MemVfs` serves them from an in-memory `BTreeMap` (the WASM/tree-session
//! path). One `FileStore` implementation runs over either, which is what makes
//! the in-memory file tree the primary operational model: untouched entries in
//! a `MemVfs` are never rewritten, so a tree session round-trips byte-identically.
//!
//! Error contract: read/metadata failures wrap a `std::io::Error` whose
//! `ErrorKind::NotFound` is preserved, so `RepositoryError::is_not_found()`
//! (and the relations-filename fallback built on it) behaves identically for
//! both backends.

use crate::error::RepositoryError;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The repository marker directory. Single source for the literal — referenced
/// by `FileStore::repository_exists`/`initialize_repository` and `tree_session`.
pub(crate) const SRS_MARKER_DIR: &str = ".srs";

/// A direct child of a directory, as returned by [`Vfs::list_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VfsEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Result of [`Vfs::check_dir_within_root`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirCheck {
    /// Directory exists and resolves inside the root.
    Ok,
    /// Path does not resolve to an existing directory.
    Missing,
    /// Path resolves outside the repository root.
    OutsideRoot,
}

/// Minimal filesystem surface consumed by `FileStore`.
///
/// All paths are repo-relative with forward slashes. Implementations own path
/// resolution and must keep `ErrorKind::NotFound` observable through
/// `RepositoryError::is_not_found()`.
pub trait Vfs: std::fmt::Debug {
    fn read_to_string(&self, rel: &str) -> Result<String, RepositoryError>;
    fn read_bytes(&self, rel: &str) -> Result<Vec<u8>, RepositoryError>;
    /// Plain write — parent directories must already exist (callers that need
    /// them call [`Vfs::create_dir_all`] first, mirroring pre-Vfs `FileStore`).
    fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), RepositoryError>;
    /// Idempotent delete: removing a missing file is `Ok(())`.
    fn remove(&self, rel: &str) -> Result<(), RepositoryError>;
    fn exists(&self, rel: &str) -> bool;
    fn is_dir(&self, rel: &str) -> bool;
    fn is_file(&self, rel: &str) -> bool;
    fn byte_len(&self, rel: &str) -> Result<u64, RepositoryError>;
    /// Direct children of `rel`. A missing directory yields an empty list.
    fn list_dir(&self, rel: &str) -> Result<Vec<VfsEntry>, RepositoryError>;
    /// All file paths under `rel` (repo-relative, forward-slash), recursively.
    /// `""` lists the whole tree. A missing directory yields an empty list.
    fn list_recursive(&self, rel: &str) -> Vec<String>;
    fn create_dir_all(&self, rel: &str) -> Result<(), RepositoryError>;
    /// Escape check for directory references (sub-package roots): does `rel`
    /// name an existing directory that resolves inside the root?
    fn check_dir_within_root(&self, rel: &str) -> Result<DirCheck, RepositoryError>;
    /// `Some(map)` for memory-backed implementations: a snapshot of every file.
    /// The Mem/Disk discriminator used by `tree_session::export_tree` and the
    /// archive pack branch (ADR-038/038) — no `Any` downcasting.
    fn as_mem_snapshot(&self) -> Option<BTreeMap<String, Vec<u8>>>;
}

/// Join a repo-relative prefix and a child path with a forward slash.
pub(crate) fn vfs_join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

// ---------------------------------------------------------------------------
// DiskVfs
// ---------------------------------------------------------------------------

/// Real-filesystem backend: resolves repo-relative paths against `root`.
#[derive(Debug, Clone)]
pub struct DiskVfs {
    root: PathBuf,
}

impl DiskVfs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn abs(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn io_err(path: PathBuf, source: std::io::Error) -> RepositoryError {
        RepositoryError::Io { path, source }
    }
}

impl Vfs for DiskVfs {
    fn read_to_string(&self, rel: &str) -> Result<String, RepositoryError> {
        let path = self.abs(rel);
        std::fs::read_to_string(&path).map_err(|source| Self::io_err(path, source))
    }

    fn read_bytes(&self, rel: &str) -> Result<Vec<u8>, RepositoryError> {
        let path = self.abs(rel);
        std::fs::read(&path).map_err(|source| Self::io_err(path, source))
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), RepositoryError> {
        let path = self.abs(rel);
        std::fs::write(&path, bytes).map_err(|source| Self::io_err(path, source))
    }

    fn remove(&self, rel: &str) -> Result<(), RepositoryError> {
        let path = self.abs(rel);
        if path.exists() {
            std::fs::remove_file(&path).map_err(|source| Self::io_err(path, source))?;
        }
        Ok(())
    }

    fn exists(&self, rel: &str) -> bool {
        self.abs(rel).exists()
    }

    fn is_dir(&self, rel: &str) -> bool {
        self.abs(rel).is_dir()
    }

    fn is_file(&self, rel: &str) -> bool {
        self.abs(rel).is_file()
    }

    fn byte_len(&self, rel: &str) -> Result<u64, RepositoryError> {
        let path = self.abs(rel);
        std::fs::metadata(&path)
            .map(|m| m.len())
            .map_err(|source| Self::io_err(path, source))
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<VfsEntry>, RepositoryError> {
        let dir = self.abs(rel);
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut entries = Vec::new();
        let read = std::fs::read_dir(&dir).map_err(|source| Self::io_err(dir.clone(), source))?;
        for entry in read {
            let entry = entry.map_err(|source| Self::io_err(dir.clone(), source))?;
            entries.push(VfsEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: entry.path().is_dir(),
            });
        }
        Ok(entries)
    }

    fn list_recursive(&self, rel: &str) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, result: &mut Vec<String>) {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(root, &path, result);
                } else {
                    let relative = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    result.push(relative);
                }
            }
        }
        let mut result = Vec::new();
        walk(&self.root, &self.abs(rel), &mut result);
        result
    }

    fn create_dir_all(&self, rel: &str) -> Result<(), RepositoryError> {
        let path = self.abs(rel);
        std::fs::create_dir_all(&path).map_err(|source| Self::io_err(path, source))
    }

    fn check_dir_within_root(&self, rel: &str) -> Result<DirCheck, RepositoryError> {
        let root_canonical = self
            .root
            .canonicalize()
            .map_err(|source| Self::io_err(self.root.clone(), source))?;
        let candidate = self.abs(rel);
        let candidate_canonical = match candidate.canonicalize() {
            Ok(p) => p,
            Err(_) => return Ok(DirCheck::Missing),
        };
        if !candidate_canonical.starts_with(&root_canonical) {
            return Ok(DirCheck::OutsideRoot);
        }
        Ok(DirCheck::Ok)
    }

    fn as_mem_snapshot(&self) -> Option<BTreeMap<String, Vec<u8>>> {
        None
    }
}

// ---------------------------------------------------------------------------
// MemVfs
// ---------------------------------------------------------------------------

/// In-memory backend: a `BTreeMap` of file paths plus an explicit-directory
/// set (for directories that exist without containing files, e.g. `.srs/`).
/// `BTreeMap` keeps iteration deterministic (ADR-017 reasoning). Interior
/// mutability via `RefCell` follows the `JsonStore` precedent — single-threaded
/// use only (CLI and WASM are both single-threaded).
#[derive(Debug, Default)]
pub struct MemVfs {
    files: RefCell<BTreeMap<String, Vec<u8>>>,
    dirs: RefCell<BTreeSet<String>>,
}

impl MemVfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an existing path→bytes map (tree session open).
    pub fn from_map(files: BTreeMap<String, Vec<u8>>) -> Self {
        Self {
            files: RefCell::new(files),
            dirs: RefCell::new(BTreeSet::new()),
        }
    }

    fn not_found(rel: &str) -> RepositoryError {
        RepositoryError::Io {
            path: PathBuf::from(rel),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no such file in memory tree: {rel}"),
            ),
        }
    }

    /// Lexically normalize `rel`; `None` if it escapes the root via `..`.
    fn normalize(rel: &str) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        for seg in rel.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                s => parts.push(s),
            }
        }
        Some(parts.join("/"))
    }
}

impl Vfs for MemVfs {
    fn read_to_string(&self, rel: &str) -> Result<String, RepositoryError> {
        let bytes = self.read_bytes(rel)?;
        String::from_utf8(bytes).map_err(|e| RepositoryError::Io {
            path: PathBuf::from(rel),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        })
    }

    fn read_bytes(&self, rel: &str) -> Result<Vec<u8>, RepositoryError> {
        self.files
            .borrow()
            .get(rel)
            .cloned()
            .ok_or_else(|| Self::not_found(rel))
    }

    fn write(&self, rel: &str, bytes: &[u8]) -> Result<(), RepositoryError> {
        self.files
            .borrow_mut()
            .insert(rel.to_string(), bytes.to_vec());
        Ok(())
    }

    fn remove(&self, rel: &str) -> Result<(), RepositoryError> {
        self.files.borrow_mut().remove(rel);
        Ok(())
    }

    fn exists(&self, rel: &str) -> bool {
        self.is_file(rel) || self.is_dir(rel)
    }

    fn is_dir(&self, rel: &str) -> bool {
        if rel.is_empty() {
            return true;
        }
        if self.dirs.borrow().contains(rel) {
            return true;
        }
        let prefix = format!("{rel}/");
        self.files
            .borrow()
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(k, _)| k.starts_with(&prefix))
            || self
                .dirs
                .borrow()
                .range(prefix.clone()..)
                .next()
                .is_some_and(|k| k.starts_with(&prefix))
    }

    fn is_file(&self, rel: &str) -> bool {
        self.files.borrow().contains_key(rel)
    }

    fn byte_len(&self, rel: &str) -> Result<u64, RepositoryError> {
        self.files
            .borrow()
            .get(rel)
            .map(|b| b.len() as u64)
            .ok_or_else(|| Self::not_found(rel))
    }

    fn list_dir(&self, rel: &str) -> Result<Vec<VfsEntry>, RepositoryError> {
        let prefix = if rel.is_empty() {
            String::new()
        } else {
            format!("{rel}/")
        };
        let mut names: BTreeMap<String, bool> = BTreeMap::new();
        for key in self.files.borrow().keys() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                if rest.is_empty() {
                    continue;
                }
                match rest.split_once('/') {
                    Some((first, _)) => {
                        names.insert(first.to_string(), true);
                    }
                    None => {
                        names.insert(rest.to_string(), false);
                    }
                }
            }
        }
        for dir in self.dirs.borrow().iter() {
            if let Some(rest) = dir.strip_prefix(&prefix) {
                if rest.is_empty() {
                    continue;
                }
                let first = rest.split('/').next().unwrap_or(rest);
                names.insert(first.to_string(), true);
            }
        }
        Ok(names
            .into_iter()
            .map(|(name, is_dir)| VfsEntry { name, is_dir })
            .collect())
    }

    fn list_recursive(&self, rel: &str) -> Vec<String> {
        if rel.is_empty() {
            return self.files.borrow().keys().cloned().collect();
        }
        let prefix = format!("{rel}/");
        self.files
            .borrow()
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect()
    }

    fn create_dir_all(&self, rel: &str) -> Result<(), RepositoryError> {
        if rel.is_empty() {
            return Ok(());
        }
        let mut dirs = self.dirs.borrow_mut();
        let mut acc = String::new();
        for seg in rel.split('/').filter(|s| !s.is_empty()) {
            if acc.is_empty() {
                acc = seg.to_string();
            } else {
                acc = format!("{acc}/{seg}");
            }
            dirs.insert(acc.clone());
        }
        Ok(())
    }

    fn check_dir_within_root(&self, rel: &str) -> Result<DirCheck, RepositoryError> {
        match Self::normalize(rel) {
            None => Ok(DirCheck::OutsideRoot),
            Some(normalized) => {
                if self.is_dir(&normalized) {
                    Ok(DirCheck::Ok)
                } else {
                    Ok(DirCheck::Missing)
                }
            }
        }
    }

    fn as_mem_snapshot(&self) -> Option<BTreeMap<String, Vec<u8>>> {
        Some(self.files.borrow().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_vfs_roundtrip() {
        let vfs = MemVfs::new();
        assert!(!vfs.exists("a/b.json"));
        vfs.write("a/b.json", b"{}").unwrap();
        assert!(vfs.exists("a/b.json"));
        assert!(vfs.is_file("a/b.json"));
        assert!(vfs.is_dir("a"));
        assert_eq!(vfs.read_to_string("a/b.json").unwrap(), "{}");
        assert_eq!(vfs.byte_len("a/b.json").unwrap(), 2);
        vfs.remove("a/b.json").unwrap();
        assert!(!vfs.exists("a/b.json"));
        // Idempotent remove
        vfs.remove("a/b.json").unwrap();
    }

    #[test]
    fn mem_vfs_not_found_kind() {
        let vfs = MemVfs::new();
        let err = vfs.read_to_string("missing.json").unwrap_err();
        assert!(err.is_not_found(), "expected is_not_found(), got {err:?}");
        let err = vfs.read_bytes("missing.bin").unwrap_err();
        assert!(err.is_not_found());
        let err = vfs.byte_len("missing.bin").unwrap_err();
        assert!(err.is_not_found());
    }

    #[test]
    fn mem_vfs_list_dir_and_recursive() {
        let vfs = MemVfs::new();
        vfs.write("records/tier-2/a.json", b"1").unwrap();
        vfs.write("records/tier-2/b.json", b"2").unwrap();
        vfs.write("records/notes/n.json", b"3").unwrap();
        vfs.write("manifest.json", b"4").unwrap();
        vfs.create_dir_all(".srs").unwrap();

        let root = vfs.list_dir("").unwrap();
        let names: Vec<(String, bool)> = root.into_iter().map(|e| (e.name, e.is_dir)).collect();
        assert_eq!(
            names,
            vec![
                (".srs".to_string(), true),
                ("manifest.json".to_string(), false),
                ("records".to_string(), true),
            ]
        );

        let tier2 = vfs.list_dir("records/tier-2").unwrap();
        assert_eq!(tier2.len(), 2);
        assert!(tier2.iter().all(|e| !e.is_dir));

        let mut all = vfs.list_recursive("records");
        all.sort();
        assert_eq!(
            all,
            vec![
                "records/notes/n.json".to_string(),
                "records/tier-2/a.json".to_string(),
                "records/tier-2/b.json".to_string(),
            ]
        );
        assert_eq!(vfs.list_recursive("").len(), 4);
        assert!(vfs.list_recursive("missing").is_empty());
        assert!(vfs.list_dir("missing").unwrap().is_empty());
    }

    #[test]
    fn mem_vfs_escape_rejected() {
        let vfs = MemVfs::new();
        vfs.write("package/package.json", b"{}").unwrap();
        assert_eq!(
            vfs.check_dir_within_root("../outside").unwrap(),
            DirCheck::OutsideRoot
        );
        assert_eq!(
            vfs.check_dir_within_root("package/../../escape").unwrap(),
            DirCheck::OutsideRoot
        );
        assert_eq!(vfs.check_dir_within_root("package").unwrap(), DirCheck::Ok);
        assert_eq!(
            vfs.check_dir_within_root("nope").unwrap(),
            DirCheck::Missing
        );
    }

    #[test]
    fn disk_vfs_escape_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir_all(root.join("package")).unwrap();
        std::fs::create_dir_all(tmp.path().join("outside")).unwrap();
        let vfs = DiskVfs::new(&root);
        assert_eq!(
            vfs.check_dir_within_root("../outside").unwrap(),
            DirCheck::OutsideRoot
        );
        assert_eq!(vfs.check_dir_within_root("package").unwrap(), DirCheck::Ok);
        assert_eq!(
            vfs.check_dir_within_root("missing").unwrap(),
            DirCheck::Missing
        );
    }

    #[test]
    fn as_mem_snapshot_discriminates() {
        let mem = MemVfs::new();
        mem.write("manifest.json", b"{}").unwrap();
        let snap = mem.as_mem_snapshot().expect("MemVfs snapshots");
        assert_eq!(snap.len(), 1);

        let tmp = tempfile::tempdir().unwrap();
        let disk = DiskVfs::new(tmp.path());
        assert!(disk.as_mem_snapshot().is_none());
    }
}
