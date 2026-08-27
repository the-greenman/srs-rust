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
//! has no `manifest.json` key — the envelope's `manifest` is the sole manifest
//! — and no `.srs` marker directory, which is synthesised on open. Files kept
//! *under* `.srs/` (agent profiles) are ordinary content and ride through.
//!
//! [R20]: only `srsj: "2"` is read. An unrecognised version is refused, never
//! coerced — there is no old-format reader here and no shadow migration.

use crate::error::RepositoryError;
use crate::store::{FileStore, RepositoryStore};
use crate::tree_session::{export_tree, new_tree_session, open_tree};
use crate::vfs::SRS_MARKER_DIR;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
/// a file at that path. A `data` key of `manifest.json` is a second authority
/// for something the envelope already owns and is refused rather than silently
/// dropped ([R19]).
pub fn tree_from_srsj(content: &str) -> Result<BTreeMap<String, Vec<u8>>, RepositoryError> {
    let envelope: SrsjEnvelope = serde_json::from_str(content)
        .map_err(|source| invalid(format!("invalid .srsj document: {source}")))?;
    if envelope.srsj != SRSJ_VERSION {
        return Err(invalid(format!(
            "unsupported srsj version '{}' — this build reads srsj '{}' only \
             (RFC-038 [R20]). Convert a pre-cutover document with the \
             `rfc038-storage` transform (`migrate_srsj`) before opening it.",
            envelope.srsj, SRSJ_VERSION
        )));
    }

    let mut tree = BTreeMap::new();
    for (key, value) in envelope.data {
        // Normalize first: `./manifest.json` is the same key by another
        // spelling and must hit the same refusal.
        let key = crate::vfs::ensure_contained(&key)?;
        if key == MANIFEST_KEY {
            return Err(invalid(
                "`.srsj` data carries a `manifest.json` key shadowing the envelope manifest \
                 — the envelope manifest is the only manifest (RFC-038 [R19])",
            ));
        }
        if let Some(previous) = tree.insert(key.clone(), value_to_bytes(value)) {
            if previous != tree[&key] {
                return Err(invalid(format!(
                    "two `.srsj` data keys resolve to '{key}' with different content"
                )));
            }
        }
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
/// Structured JSON at a `.json` path is carried as a JSON value; everything
/// else is carried as raw text. Nothing under `sourceDocumentsPath` is ever
/// carried structurally — those are opaque payloads, which [R9] requires to be
/// preserved unmodified, and canonicalising one would change its bytes and its
/// checksum.
///
/// A non-UTF-8 payload cannot ride in a JSON-only document at all, so the
/// projection **fails** naming it rather than dropping it silently: `.srs` is
/// the carrier that transports binary content.
///
/// `.srs/` content (agent profiles, and anything else the implementation keeps
/// there) rides along like any other file. The one exception is the synthetic
/// `.gitkeep` placeholder: it exists so a git host can see an otherwise empty
/// marker directory, and `open_tree`/`export_tree` regenerate it — carrying it
/// would make the document's shape depend on whether it had been through a
/// session.
fn srsj_from_tree(tree: &BTreeMap<String, Vec<u8>>) -> Result<String, RepositoryError> {
    let manifest_bytes = tree
        .get(MANIFEST_KEY)
        .ok_or_else(|| invalid("cannot project a repository with no manifest.json as `.srsj`"))?;
    let manifest: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|source| invalid(format!("invalid manifest.json: {source}")))?;

    // The same rule the catalog and the archive use, so all three agree about
    // which payloads are opaque.
    let opaque_prefix = crate::catalog::declared_location(
        manifest.get("sourceDocumentsPath").and_then(|v| v.as_str()),
    )
    .unwrap_or_else(|| "source-documents".to_string());
    let opaque_prefix = format!("{opaque_prefix}/");

    let gitkeep = format!("{SRS_MARKER_DIR}/.gitkeep");
    let mut data = BTreeMap::new();
    for (path, bytes) in tree {
        if path == MANIFEST_KEY || path == &gitkeep {
            continue;
        }
        // Same containment rule as decode: never emit a document this codec
        // would then refuse to reopen.
        let path = crate::vfs::ensure_contained(path)?;
        let text = std::str::from_utf8(bytes).map_err(|_| {
            invalid(format!(
                "'{path}' is not UTF-8 and cannot travel in a `.srsj` document — \
                 use the `.srs` archive format, which carries binary content"
            ))
        })?;
        // Structure is the point only for a `.json` file that is repository
        // content — never for an opaque payload under `sourceDocumentsPath`.
        let structural = path.ends_with(".json") && !path.starts_with(&opaque_prefix);
        data.insert(path, canonicalize(text_to_value(text, structural), false));
    }

    let envelope = SrsjEnvelope {
        srsj: SRSJ_VERSION.to_string(),
        manifest: canonicalize(manifest, false),
        data,
    };
    serde_json::to_string_pretty(&envelope)
        .map_err(|source| invalid(format!("cannot serialise .srsj document: {source}")))
}

/// The inverse of [`bytes_to_value`], under one rule: **a string value is the
/// file's raw text; anything else is the file's JSON content.** Structured
/// payloads come back canonicalised rather than byte-identical — that is
/// ADR-043's canonicalize-on-write, and it is why `.srsj` is a projection of a
/// tree rather than a copy of one.
fn value_to_bytes(value: serde_json::Value) -> Vec<u8> {
    match value {
        serde_json::Value::String(s) => s.into_bytes(),
        other => serde_json::to_vec_pretty(&other)
            .expect("serialising an owned serde_json::Value cannot fail"),
    }
}

/// Carry a payload structurally only where structure is the point: an object
/// or array at a `.json` path that is not an opaque source-document payload.
///
/// Every condition is load-bearing. Content alone would canonicalise a Markdown
/// file that happens to contain JSON; the path alone would turn a scalar JSON
/// document (`"hello"`, `42`) or an unparseable `.json` file into a bare string
/// and lose its own syntax; and neither would spare a JSON *attachment*, whose
/// bytes and checksum [R9] requires to survive untouched. Everything else is
/// carried as raw text and comes back byte-identical.
fn text_to_value(text: &str, structural: bool) -> serde_json::Value {
    if structural {
        if let Ok(v @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) =
            serde_json::from_str::<serde_json::Value>(text)
        {
            return v;
        }
    }
    serde_json::Value::String(text.to_string())
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

/// The directory a `.srsj` document lives in — what a session reports as its
/// repository root, since the document itself is the repository.
fn session_root(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
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
        Self::open_inner(path.into(), false)
    }

    /// Open with the RFC-038 [R21] migrator exemption — the migration tooling
    /// surface only (see [`FileStore::with_rfc038_exemption`]); every other
    /// caller uses [`SrsjSession::open`].
    pub fn open_for_migration(path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        Self::open_inner(path.into(), true)
    }

    fn open_inner(path: PathBuf, rfc038_exempt: bool) -> Result<Self, RepositoryError> {
        let raw = std::fs::read_to_string(&path).map_err(|source| RepositoryError::Io {
            path: path.clone(),
            source,
        })?;
        let mut store = open_srsj(&raw)?.rooted_at(session_root(&path));
        if rfc038_exempt {
            store = store.with_rfc038_exemption();
        }
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
        let store = new_tree_session().rooted_at(session_root(&path));
        Ok(Self {
            path,
            store,
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
        // The document *is* the repository, so a truncating write that is
        // interrupted (Ctrl-C, ENOSPC) destroys it. Write beside it and rename,
        // which is atomic on the same filesystem.
        let staged = self.path.with_extension("srsj.tmp");
        let io = |source| RepositoryError::Io {
            path: self.path.clone(),
            source,
        };
        std::fs::write(&staged, text).map_err(io)?;
        if let Err(source) = std::fs::rename(&staged, &self.path) {
            let _ = std::fs::remove_file(&staged);
            return Err(io(source));
        }
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
    fn marker_directory_content_rides_through_but_the_placeholder_does_not() {
        // `.srs/profiles/*.json` is real repository content (agent profiles);
        // `.srs/.gitkeep` is only a regenerated git-visibility placeholder.
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"][".srs/profiles/foundation.json"] = serde_json::json!({ "name": "foundation" });
        let store = open_srsj(&doc.to_string()).unwrap();
        assert!(store
            .load_text_file(".srs/profiles/foundation.json")
            .is_ok());

        let out: serde_json::Value =
            serde_json::from_str(&to_srsj_string(&store).unwrap()).unwrap();
        assert_eq!(
            out["data"][".srs/profiles/foundation.json"]["name"],
            "foundation"
        );
        assert!(out["data"].get(".srs/.gitkeep").is_none());
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
    fn non_utf8_payloads_fail_the_projection_rather_than_vanishing() {
        // A JSON-only document cannot carry binary content. Dropping it
        // silently is how `srs attachment add --repo repo.srsj` came to report
        // success for content that was never persisted.
        let mut tree = tree_from_srsj(&minimal_srsj("2")).unwrap();
        tree.insert(
            "source-documents/photo.png".to_string(),
            vec![0x89, 0x50, 0x4e, 0x47, 0xff, 0xfe],
        );
        let err = srsj_from_tree(&tree).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("source-documents/photo.png"), "{message}");
        assert!(message.contains("`.srs` archive format"), "{message}");
    }

    #[test]
    fn an_opaque_json_payload_is_preserved_unmodified() {
        // [R9]: source-document payloads are opaque. Canonicalising a JSON
        // attachment would change its bytes and its checksum.
        let raw = "{\"z\":1,\"a\":2}";
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"]["source-documents/payload.json"] = serde_json::json!(raw);
        let store = open_srsj(&doc.to_string()).unwrap();
        assert_eq!(
            store
                .load_text_file("source-documents/payload.json")
                .unwrap(),
            raw
        );
        let out: serde_json::Value =
            serde_json::from_str(&to_srsj_string(&store).unwrap()).unwrap();
        assert_eq!(out["data"]["source-documents/payload.json"], raw);
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

    /// ADR-043 canonicalize-on-write: every object's keys sort, except
    /// `fieldValues`/`fieldMeta` subtrees whose key order is data ([R18]).
    #[test]
    fn canonicalize_sorts_all_but_carrier_subtrees() {
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"]["records/r1.json"] = serde_json::json!({
            "typeVersion": 1,
            "typeNamespace": "com.test",
            "typeName": "t",
            "typeId": "t1",
            "instanceId": "r1",
            // Deliberately non-alphabetical: order is data ([R18]) at every
            // depth, including composite interiors.
            "fieldValues": {
                "zeta": "z",
                "alpha": "a",
                "rows": [{"zz_cell": "1", "aa_cell": "2"}]
            },
            "fieldMeta": {"zeta": {"source": "human"}, "alpha": {"source": "ai"}}
        });
        let store = open_srsj(&doc.to_string()).unwrap();

        let out: serde_json::Value =
            serde_json::from_str(&to_srsj_string(&store).unwrap()).unwrap();
        let record = &out["data"]["records/r1.json"];

        let record_keys: Vec<&String> = record.as_object().unwrap().keys().collect();
        let mut sorted = record_keys.clone();
        sorted.sort();
        assert_eq!(record_keys, sorted, "record envelope keys must sort");

        let fv_keys: Vec<&String> = record["fieldValues"].as_object().unwrap().keys().collect();
        assert_eq!(
            fv_keys,
            ["zeta", "alpha", "rows"].iter().collect::<Vec<_>>()
        );
        let row_keys: Vec<&String> = record["fieldValues"]["rows"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(row_keys, ["zz_cell", "aa_cell"].iter().collect::<Vec<_>>());
        let meta_keys: Vec<&String> = record["fieldMeta"].as_object().unwrap().keys().collect();
        assert_eq!(meta_keys, ["zeta", "alpha"].iter().collect::<Vec<_>>());
    }

    /// Definition `extra` properties (`$schema`, `aiGuidance` on a Type) must
    /// reach the loaded package through the codec, not be dropped by it (#684).
    #[test]
    fn definition_extras_survive_the_codec() {
        let mut doc: serde_json::Value = serde_json::from_str(&minimal_srsj("2")).unwrap();
        doc["data"]["package/package.json"]["types"] = serde_json::json!(["types/thing.json"]);
        doc["data"]["package/types/thing.json"] = serde_json::json!({
            "$schema": "https://srs.semanticops.com/schema/2.0/type.json",
            "id": "00000000-0000-4000-8000-00000000cccc",
            "namespace": "com.example.test",
            "name": "thing",
            "version": 1,
            "aiGuidance": "guidance survives",
            "fields": []
        });
        let store = open_srsj(&doc.to_string()).unwrap();
        let package = store.load_package().expect("load_package");
        let record_type = package
            .record_types
            .iter()
            .find(|t| t.name == "thing")
            .expect("type loaded");
        assert_eq!(
            record_type.ai_guidance.as_ref().and_then(|v| v.as_str()),
            Some("guidance survives")
        );
        assert_eq!(
            record_type.schema.as_deref(),
            Some("https://srs.semanticops.com/schema/2.0/type.json")
        );
    }

    #[test]
    fn open_reports_a_missing_or_malformed_document() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("absent.srsj");
        assert!(matches!(
            SrsjSession::open(&missing),
            Err(RepositoryError::Io { .. })
        ));

        let malformed = dir.path().join("bad.srsj");
        std::fs::write(&malformed, "{ not json").unwrap();
        match SrsjSession::open(&malformed) {
            Err(err) => assert!(
                err.to_string().contains("invalid .srsj document"),
                "got: {err}"
            ),
            Ok(_) => panic!("a malformed document must not open"),
        }
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
    fn a_session_reports_its_document_directory_as_the_repository_root() {
        // Not the `<memory>` sentinel: `repository_root()` reaches CLI payloads
        // and error messages, and `repo create --store json` must name a place
        // a human can find.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("repo.srsj");
        let session = SrsjSession::create(&path).unwrap();
        assert_eq!(session.store().repository_root(), dir.path());
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
