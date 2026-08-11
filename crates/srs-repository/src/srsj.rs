//! `.srsj` codec — the boundary between the JSON envelope and the operational
//! tree (RFC-038 [R17]/[R19]/[R20], ADR-038/ADR-040).
//!
//! There is exactly one `.srsj` mechanism: decode the envelope into a
//! path→bytes file tree, open that tree as a `MemVfs`-backed [`FileStore`],
//! operate on it with the same services the CLI runs on disk, and project it
//! back out. `.srsj` is a carrier, never session state — nothing here
//! implements `RepositoryStore`, and no service branches on it.
//!
//! Membership follows [R19]: `data` keys resolve segment-wise under reserved
//! instance roots once materialised, so the catalog walker that enumerates a
//! disk repository enumerates a `.srsj` session unchanged. A `.srsj` document
//! has neither a `manifest.json` key nor an `.srs` marker — the envelope's
//! `manifest` is the sole manifest, and the marker is synthesised on open and
//! stripped on export.
//!
//! [R20]: only `srsj: "2"` is read. An unrecognised version is refused, never
//! coerced — there is no old-format reader here and no shadow migration.

use crate::error::RepositoryError;
use crate::store::{FileStore, RepositoryStore};
use crate::tree_session::{export_tree, open_tree};
use crate::vfs::{MemVfs, SRS_MARKER_DIR};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The only `.srsj` envelope version this build reads or writes ([R20]).
pub const SRSJ_VERSION: &str = "2";

/// The manifest's path once materialised into the tree.
const MANIFEST_KEY: &str = "manifest.json";

#[derive(Serialize, Deserialize)]
struct SrsjEnvelope {
    srsj: String,
    manifest: serde_json::Value,
    // BTreeMap (not HashMap) so the envelope serialises in sorted key order —
    // minimal-diff, idempotent writes (ADR-017/ADR-043).
    #[serde(default)]
    data: BTreeMap<String, serde_json::Value>,
}

fn invalid(message: impl Into<String>) -> RepositoryError {
    RepositoryError::InvalidSnapshotData {
        message: message.into(),
    }
}

/// Decode a `.srsj` document into a repository file tree.
///
/// The envelope's `manifest` becomes `manifest.json`; every `data` key becomes
/// a file at that path. A `data` key of `manifest.json` or under the `.srs`
/// marker is a second authority for something the envelope already owns and is
/// refused rather than silently dropped ([R19]).
pub fn tree_from_srsj(content: &str) -> Result<BTreeMap<String, Vec<u8>>, RepositoryError> {
    let envelope: SrsjEnvelope = serde_json::from_str(content)
        .map_err(|source| invalid(format!("invalid .srsj document: {source}")))?;
    if envelope.srsj != SRSJ_VERSION {
        return Err(invalid(format!(
            "unsupported srsj version '{}' — this build reads srsj '{}' only \
             (RFC-038 [R20]); run the rfc038-storage migration",
            envelope.srsj, SRSJ_VERSION
        )));
    }

    let mut tree = BTreeMap::new();
    let marker_prefix = format!("{SRS_MARKER_DIR}/");
    for (key, value) in envelope.data {
        if key == MANIFEST_KEY {
            return Err(invalid(
                "`.srsj` data carries a `manifest.json` key shadowing the envelope manifest \
                 — the envelope manifest is the only manifest (RFC-038 [R19])",
            ));
        }
        if key == SRS_MARKER_DIR || key.starts_with(&marker_prefix) {
            return Err(invalid(format!(
                "`.srsj` data carries marker-directory key '{key}' — `.srs/` contents are \
                 implementation-private and are not carried in a `.srsj` document (RFC-038 [R19])"
            )));
        }
        tree.insert(key, value_to_bytes(value));
    }

    tree.insert(
        MANIFEST_KEY.to_string(),
        serde_json::to_vec_pretty(&canonicalize(envelope.manifest, false))
            .map_err(|source| invalid(format!("cannot serialise manifest: {source}")))?,
    );
    Ok(tree)
}

/// Open a `.srsj` document as an in-memory tree session — the identical
/// `FileStore` the CLI runs on disk (ADR-038).
pub fn open_srsj(content: &str) -> Result<FileStore, RepositoryError> {
    open_tree(tree_from_srsj(content)?)
}

/// Project any repository as a `.srsj` document.
///
/// Enumeration is the single authoritative store→tree walk ([R17]), so the
/// projection carries the source's real paths rather than re-canonicalising
/// them (ADR-040, srs-rust#696).
pub fn to_srsj_string(source: &dyn RepositoryStore) -> Result<String, RepositoryError> {
    srsj_from_tree(&crate::archive::tree_entries(source)?)
}

/// Encode a file tree as a `.srsj` document.
///
/// Payloads at `.json` paths are carried as JSON values; everything else UTF-8
/// is carried as a string. Non-UTF-8 payloads are attachment content, which
/// RFC-017 makes transient transport rather than repository content — they are
/// not carried by the JSON-only format.
fn srsj_from_tree(tree: &BTreeMap<String, Vec<u8>>) -> Result<String, RepositoryError> {
    let manifest_bytes = tree
        .get(MANIFEST_KEY)
        .ok_or_else(|| invalid("cannot project a repository with no manifest.json as `.srsj`"))?;
    let manifest: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|source| invalid(format!("invalid manifest.json: {source}")))?;

    let marker_prefix = format!("{SRS_MARKER_DIR}/");
    let mut data = BTreeMap::new();
    for (path, bytes) in tree {
        if path == MANIFEST_KEY || path == SRS_MARKER_DIR || path.starts_with(&marker_prefix) {
            continue;
        }
        if let Some(value) = bytes_to_value(path, bytes) {
            data.insert(path.clone(), canonicalize(value, false));
        }
    }

    let envelope = SrsjEnvelope {
        srsj: SRSJ_VERSION.to_string(),
        manifest: canonicalize(manifest, false),
        data,
    };
    serde_json::to_string_pretty(&envelope)
        .map_err(|source| invalid(format!("cannot serialise .srsj document: {source}")))
}

fn value_to_bytes(value: serde_json::Value) -> Vec<u8> {
    match value {
        // Text payloads (Markdown source documents, rendered exports) are
        // carried as JSON strings; their bytes are the string itself.
        serde_json::Value::String(s) => s.into_bytes(),
        other => serde_json::to_vec_pretty(&other)
            .expect("serialising an owned serde_json::Value cannot fail"),
    }
}

fn bytes_to_value(path: &str, bytes: &[u8]) -> Option<serde_json::Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    if path.ends_with(".json") {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            return Some(value);
        }
    }
    Some(serde_json::Value::String(text.to_string()))
}

/// Recursively sort object keys for deterministic `.srsj` output (ADR-043),
/// leaving `fieldValues`/`fieldMeta` subtrees in stored order — their key
/// order is data ([R18]). `in_carrier` marks descent below such a key.
fn canonicalize(value: serde_json::Value, in_carrier: bool) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            if in_carrier {
                // Order is data at every depth of a carrier subtree ([R18]);
                // recurse without re-sorting.
                serde_json::Value::Object(
                    map.into_iter()
                        .map(|(k, v)| (k, canonicalize(v, true)))
                        .collect(),
                )
            } else {
                let mut sorted: Vec<(String, serde_json::Value)> = map
                    .into_iter()
                    .map(|(k, v)| {
                        let carrier = k == "fieldValues" || k == "fieldMeta";
                        (k.clone(), canonicalize(v, carrier))
                    })
                    .collect();
                sorted.sort_by(|a, b| a.0.cmp(&b.0));
                serde_json::Value::Object(sorted.into_iter().collect())
            }
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(|v| canonicalize(v, in_carrier))
                .collect(),
        ),
        other => other,
    }
}

/// A file-backed `.srsj` working session: decode → operate on the tree →
/// project back to the file.
///
/// The CLI's `--repo <file>.srsj` path and `repo create --store json`. The
/// file is rewritten only when the tree actually changed, so a read-only or
/// dry-run command leaves it byte-identical.
pub struct SrsjSession {
    path: PathBuf,
    store: FileStore,
    baseline: BTreeMap<String, Vec<u8>>,
}

impl SrsjSession {
    /// Open an existing `.srsj` file as a tree session.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let path = path.into();
        let raw = std::fs::read_to_string(&path).map_err(|source| RepositoryError::Io {
            path: path.clone(),
            source,
        })?;
        let store = open_srsj(&raw)?;
        let baseline = export_tree(&store)?;
        Ok(Self {
            path,
            store,
            baseline,
        })
    }

    /// Start a session for a `.srsj` file that does not exist yet.
    ///
    /// The session opens empty — there is no manifest until the caller
    /// initialises the repository — and the file appears on the first
    /// [`SrsjSession::flush`].
    pub fn create(path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let path = path.into();
        if path.exists() {
            return Err(RepositoryError::RepositoryAlreadyExists { path });
        }
        Ok(Self {
            path,
            store: FileStore::from_vfs(Rc::new(MemVfs::new())),
            baseline: BTreeMap::new(),
        })
    }

    pub fn store(&self) -> &FileStore {
        &self.store
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write the session back to its file when — and only when — the tree
    /// changed.
    pub fn flush(&mut self) -> Result<(), RepositoryError> {
        let current = export_tree(&self.store)?;
        if current == self.baseline {
            return Ok(());
        }
        let text = srsj_from_tree(&current)?;
        std::fs::write(&self.path, text).map_err(|source| RepositoryError::Io {
            path: self.path.clone(),
            source,
        })?;
        self.baseline = current;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_srsj(version: &str) -> String {
        serde_json::json!({
            "srsj": version,
            "manifest": {
                "srsVersion": "2.0-draft",
                "repositoryId": "00000000-0000-4000-8000-00000000aaaa",
                "namespace": "com.example.test",
                "dataModelRevision": 2,
            },
            "data": {
                "package/package.json": {
                    "id": "00000000-0000-4000-8000-00000000bbbb",
                    "namespace": "com.example.test",
                    "name": "test-package",
                    "version": "1.0.0",
                    "fields": [],
                    "types": [],
                },
            },
        })
        .to_string()
    }

    #[test]
    fn r20_refuses_a_generation_1_document() {
        let err = tree_from_srsj(&minimal_srsj("1")).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("unsupported srsj version '1'"),
            "must name the refused version, got: {message}"
        );
        assert!(
            message.contains("[R20]"),
            "must cite the governing rule, got: {message}"
        );
    }

    #[test]
    fn r20_refuses_an_unrecognised_future_version() {
        let err = tree_from_srsj(&minimal_srsj("3")).unwrap_err();
        assert!(err.to_string().contains("unsupported srsj version '3'"));
    }

    #[test]
    fn decode_materialises_manifest_and_data_as_files() {
        let tree = tree_from_srsj(&minimal_srsj("2")).unwrap();
        assert!(tree.contains_key("manifest.json"));
        assert!(tree.contains_key("package/package.json"));
        let manifest: serde_json::Value =
            serde_json::from_slice(&tree["manifest.json"]).expect("manifest is JSON");
        assert_eq!(manifest["namespace"], "com.example.test");
    }

    #[test]
    fn r19_refuses_a_shadow_manifest_key() {
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"]["manifest.json"] = serde_json::json!({ "repositoryId": "other" });
        let err = tree_from_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("shadowing the envelope manifest"),
            "got: {err}"
        );
    }

    #[test]
    fn r19_refuses_marker_directory_keys() {
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"][".srs/.gitkeep"] = serde_json::json!("");
        let err = tree_from_srsj(&doc.to_string()).unwrap_err();
        assert!(
            err.to_string().contains("marker-directory key"),
            "got: {err}"
        );
    }

    #[test]
    fn envelope_round_trip_is_byte_stable() {
        let store = open_srsj(&minimal_srsj("2")).unwrap();
        let once = to_srsj_string(&store).unwrap();
        let twice = to_srsj_string(&open_srsj(&once).unwrap()).unwrap();
        assert_eq!(once, twice, "decode → encode must be idempotent");
        let doc: serde_json::Value = serde_json::from_str(&once).unwrap();
        assert_eq!(doc["srsj"], SRSJ_VERSION);
        assert!(
            doc["data"].get(".srs/.gitkeep").is_none(),
            "the marker is synthesised on open, never carried"
        );
        assert!(
            doc["data"].get("manifest.json").is_none(),
            "the manifest lives in the envelope, never in data"
        );
    }

    #[test]
    fn non_utf8_payloads_are_not_carried() {
        // RFC-017: attachment content is transient transport, not repository
        // content — the JSON-only carrier drops it rather than failing.
        let mut tree = tree_from_srsj(&minimal_srsj("2")).unwrap();
        tree.insert(
            "source-documents/photo.png".to_string(),
            vec![0x89, 0x50, 0x4e, 0x47, 0xff, 0xfe],
        );
        let text = srsj_from_tree(&tree).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(doc["data"].get("source-documents/photo.png").is_none());
    }

    #[test]
    fn text_payloads_survive_the_round_trip() {
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"]["source-documents/notes.md"] = serde_json::json!("# Heading\n\nbody\n");
        let store = open_srsj(&doc.to_string()).unwrap();
        assert_eq!(
            store.load_text_file("source-documents/notes.md").unwrap(),
            "# Heading\n\nbody\n"
        );
        let out: serde_json::Value =
            serde_json::from_str(&to_srsj_string(&store).unwrap()).unwrap();
        assert_eq!(
            out["data"]["source-documents/notes.md"],
            "# Heading\n\nbody\n"
        );
    }

    #[test]
    fn session_does_not_rewrite_an_untouched_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("repo.srsj");
        let canonical = to_srsj_string(&open_srsj(&minimal_srsj("2")).unwrap()).unwrap();
        std::fs::write(&path, &canonical).unwrap();
        let before = std::fs::metadata(&path).unwrap().len();

        let mut session = SrsjSession::open(&path).unwrap();
        let _ = session.store().load_manifest().unwrap();
        session.flush().unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), canonical);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
    }

    #[test]
    fn session_writes_back_a_changed_tree() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("repo.srsj");
        std::fs::write(
            &path,
            to_srsj_string(&open_srsj(&minimal_srsj("2")).unwrap()).unwrap(),
        )
        .unwrap();

        let mut session = SrsjSession::open(&path).unwrap();
        session
            .store()
            .save_text_file("spec/readme.md", "hello")
            .unwrap();
        session.flush().unwrap();

        let reopened = SrsjSession::open(&path).unwrap();
        assert_eq!(
            reopened.store().load_text_file("spec/readme.md").unwrap(),
            "hello"
        );
    }

    #[test]
    fn session_create_rejects_an_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("repo.srsj");
        std::fs::write(&path, "{}").unwrap();
        assert!(matches!(
            SrsjSession::create(&path),
            Err(RepositoryError::RepositoryAlreadyExists { .. })
        ));
    }
}
