//! Derived catalog cache (ADR-056, srs-rust#1198).
//!
//! The per-file instance classification (read, parse, schema-validate) is a pure
//! function of a file's bytes. This module remembers its successful results in
//! `.srs/cache/catalog.json` so a later CLI invocation re-reads only files that
//! changed. The cache is derived and adapter-private: it never changes a result,
//! and a missing, corrupt or stale one falls back to the full build.

use crate::catalog::CatalogKind;
use crate::vfs::FileStamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(crate) const CACHE_FILE: &str = "catalog.json";

/// Bump when the on-disk shape or the meaning of an entry changes.
const CACHE_FORMAT: u32 = 1;

/// An entry whose mtime is within this window of the cache write may have been
/// modified again without changing size or mtime (coarse timestamp granularity,
/// clock skew), so it is confirmed by content hash. The approach git uses for
/// "racily clean" index entries.
const RACY_WINDOW_NS: u128 = 2_000_000_000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    size: u64,
    mtime_ns: u128,
    /// SHA-256 of the file text the classification was computed from.
    hash: String,
    kind: String,
    tier: u8,
    id: String,
}

impl Entry {
    pub(crate) fn new(stamp: FileStamp, text: &str, kind: CatalogKind, tier: u8, id: &str) -> Self {
        Entry {
            size: stamp.size,
            mtime_ns: stamp.mtime_ns,
            hash: hash_text(text),
            kind: kind.as_str().to_string(),
            tier,
            id: id.to_string(),
        }
    }

    pub(crate) fn matches_stamp(&self, stamp: FileStamp) -> bool {
        self.size == stamp.size && self.mtime_ns == stamp.mtime_ns
    }

    pub(crate) fn matches_text(&self, text: &str) -> bool {
        self.hash == hash_text(text)
    }

    /// The cached classification, or `None` for a kind this build no longer
    /// admits (treated as a miss).
    pub(crate) fn classification(&self) -> Option<(CatalogKind, u8, String)> {
        let kind = match self.kind.as_str() {
            "note" => CatalogKind::Note,
            "record" => CatalogKind::Record,
            _ => return None,
        };
        Some((kind, self.tier, self.id.clone()))
    }
}

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct CacheFile {
    format: u32,
    fingerprint: String,
    written_ns: u128,
    entries: BTreeMap<String, Entry>,
}

/// A loaded, validated cache ready for lookups.
pub(crate) struct Loaded {
    written_ns: u128,
    pub(crate) entries: BTreeMap<String, Entry>,
}

impl Loaded {
    /// Trust the entry's stamp alone only when its mtime is clearly older than
    /// the cache write; otherwise the content hash must confirm it.
    pub(crate) fn needs_hash_check(&self, stamp: FileStamp) -> bool {
        stamp.mtime_ns.saturating_add(RACY_WINDOW_NS) >= self.written_ns
    }
}

fn hash_text(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

/// Identity of everything that can change a classification other than the file:
/// the cache format, this build, and the embedded schema set.
fn fingerprint() -> String {
    let mut h = Sha256::new();
    h.update(CACHE_FORMAT.to_le_bytes());
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    // `srs --version` cannot tell builds apart, so identify the running binary.
    if let Some(meta) = std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
    {
        h.update(meta.len().to_le_bytes());
        if let Some(t) = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        {
            h.update(t.as_nanos().to_le_bytes());
        }
    }
    for id in srs_schema::ALL_SCHEMA_IDS {
        h.update(id.as_bytes());
        if let Some(src) = srs_schema::schema_source(id) {
            h.update(Sha256::digest(src.as_bytes()));
        }
    }
    hex::encode(h.finalize())
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Parse and validate cache bytes; any mismatch is a miss (`None`).
pub(crate) fn load(bytes: &[u8]) -> Option<Loaded> {
    let file: CacheFile = serde_json::from_slice(bytes).ok()?;
    if file.format != CACHE_FORMAT || file.fingerprint != fingerprint() {
        return None;
    }
    Some(Loaded {
        written_ns: file.written_ns,
        entries: file.entries,
    })
}

pub(crate) fn serialize(entries: BTreeMap<String, Entry>) -> Option<Vec<u8>> {
    serde_json::to_vec(&CacheFile {
        format: CACHE_FORMAT,
        fingerprint: fingerprint(),
        written_ns: now_ns(),
        entries,
    })
    .ok()
}
