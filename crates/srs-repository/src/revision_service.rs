//! # Revision Service
//!
//! Manages per-record revision sidecars for ext:addressability.
//!
//! ## Storage layout
//!
//! Each record's revisions live in a sidecar file at the same path as the
//! record file but with a `.revisions.json` suffix instead of `.json`.
//! Example: `records/abc123.json` → `records/abc123.revisions.json`.
//!
//! The sidecar is created on first append and deleted when the record is
//! deleted. Revision lists are append-only; there is no delete surface.
//!
//! ## Growth model
//!
//! The primary write path is lifecycle-triggered snapshots: one revision per
//! field per lifecycle transition. For a Type with N fields and T transitions
//! that is at most N×T revisions. Lists support `--limit` and `--offset` for
//! forward compatibility.
//!
//! ## Failure handling
//!
//! `append` is called after a transition commits. If `append` fails the
//! transition is already persisted; the caller receives a
//! `REVISION_APPEND_FAILED` diagnostic rather than an error. See
//! `transition_record_lifecycle` in `record_store`.

use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use srs_core::types::revision::{Revision, RevisionSidecar};

/// Derive the sidecar path from the record's instance file path.
///
/// `records/foo.json` → `records/foo.revisions.json`
pub(crate) fn sidecar_path_for(record_path: &str) -> String {
    if let Some(stem) = record_path.strip_suffix(".json") {
        format!("{}.revisions.json", stem)
    } else {
        format!("{}.revisions.json", record_path)
    }
}

/// Load the sidecar for `record_id` at `record_path`, or return an empty one.
fn load_or_empty(
    store: &dyn RepositoryStore,
    record_id: &str,
    sidecar_path: &str,
) -> RevisionSidecar {
    match store.load_instance_json(sidecar_path) {
        Ok(v) => serde_json::from_value(v).unwrap_or(RevisionSidecar {
            record_id: record_id.to_string(),
            revisions: vec![],
        }),
        Err(_) => RevisionSidecar {
            record_id: record_id.to_string(),
            revisions: vec![],
        },
    }
}

/// Append a single revision to the sidecar for the given record.
///
/// Creates the sidecar if it does not yet exist.
/// On success returns `Ok(())`. On I/O failure returns `Err(…)` — callers
/// that treat revision append as best-effort should convert this to a
/// diagnostic rather than propagating the error.
pub fn append(
    store: &dyn RepositoryStore,
    record_path: &str,
    revision: Revision,
) -> Result<(), RepositoryError> {
    let sidecar_path = sidecar_path_for(record_path);
    let mut sidecar = load_or_empty(store, &revision.record_id, &sidecar_path);
    sidecar.revisions.push(revision);
    let value = serde_json::to_value(&sidecar).map_err(|e| RepositoryError::Serialize {
        path: std::path::PathBuf::from(&sidecar_path),
        source: e,
    })?;
    store.save_instance_json(&sidecar_path, &value)
}

/// List revisions for a record, optionally filtered by `field_id`.
///
/// Returns revisions in append order (oldest first). Supports pagination
/// via `limit` and `offset` for forward compatibility with large sidecars.
pub fn list(
    store: &dyn RepositoryStore,
    record_path: &str,
    record_id: &str,
    field_id: Option<&str>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<Revision>, RepositoryError> {
    let sidecar_path = sidecar_path_for(record_path);
    let sidecar = load_or_empty(store, record_id, &sidecar_path);

    let filtered: Vec<Revision> = sidecar
        .revisions
        .into_iter()
        .filter(|r| field_id.map(|fid| r.field_id == fid).unwrap_or(true))
        .collect();

    let start = offset.unwrap_or(0).min(filtered.len());
    let slice = &filtered[start..];
    let result = match limit {
        Some(n) => slice[..n.min(slice.len())].to_vec(),
        None => slice.to_vec(),
    };
    Ok(result)
}

/// Get a single revision by its `revision_id`, searching the sidecar at `record_path`.
pub fn get(
    store: &dyn RepositoryStore,
    record_path: &str,
    record_id: &str,
    revision_id: &str,
) -> Result<Option<Revision>, RepositoryError> {
    let sidecar_path = sidecar_path_for(record_path);
    let sidecar = load_or_empty(store, record_id, &sidecar_path);
    Ok(sidecar
        .revisions
        .into_iter()
        .find(|r| r.revision_id == revision_id))
}

/// Delete the sidecar for a record (called when the record itself is deleted).
pub fn delete_sidecar(
    store: &dyn RepositoryStore,
    record_path: &str,
) -> Result<(), RepositoryError> {
    let sidecar_path = sidecar_path_for(record_path);
    // If the sidecar doesn't exist, that's fine — nothing to delete.
    match store.delete_instance_file(&sidecar_path) {
        Ok(()) => Ok(()),
        Err(RepositoryError::NotFound { .. }) => Ok(()),
        Err(RepositoryError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;
    use serde_json::json;
    use srs_core::types::revision::{RevisionAgent, RevisionProvenance};

    fn make_revision(
        id: &str,
        record_id: &str,
        field_id: &str,
        value: serde_json::Value,
    ) -> Revision {
        Revision {
            revision_id: id.to_string(),
            record_id: record_id.to_string(),
            field_id: field_id.to_string(),
            value,
            prior_revision_id: None,
            agent: RevisionAgent::Human,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn sidecar_path_derives_correctly() {
        assert_eq!(
            sidecar_path_for("records/abc.json"),
            "records/abc.revisions.json"
        );
        assert_eq!(
            sidecar_path_for("records/abc"),
            "records/abc.revisions.json"
        );
    }

    #[test]
    fn append_and_list_roundtrip() {
        let store = MemoryStore::empty();
        let rev = make_revision("rev-1", "rec-1", "field-1", json!("hello"));
        append(&store, "records/rec-1.json", rev).unwrap();

        let revs = list(&store, "records/rec-1.json", "rec-1", None, None, None).unwrap();
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].revision_id, "rev-1");
    }

    #[test]
    fn list_filters_by_field_id() {
        let store = MemoryStore::empty();
        append(
            &store,
            "records/rec-1.json",
            make_revision("rev-1", "rec-1", "field-a", json!("a")),
        )
        .unwrap();
        append(
            &store,
            "records/rec-1.json",
            make_revision("rev-2", "rec-1", "field-b", json!("b")),
        )
        .unwrap();

        let revs = list(
            &store,
            "records/rec-1.json",
            "rec-1",
            Some("field-a"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(revs.len(), 1);
        assert_eq!(revs[0].field_id, "field-a");
    }

    #[test]
    fn list_pagination() {
        let store = MemoryStore::empty();
        for i in 0..5 {
            append(
                &store,
                "records/rec-1.json",
                make_revision(&format!("rev-{i}"), "rec-1", "f1", json!(i)),
            )
            .unwrap();
        }

        let page = list(
            &store,
            "records/rec-1.json",
            "rec-1",
            None,
            Some(2),
            Some(1),
        )
        .unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].revision_id, "rev-1");
        assert_eq!(page[1].revision_id, "rev-2");
    }

    #[test]
    fn get_returns_correct_revision() {
        let store = MemoryStore::empty();
        append(
            &store,
            "records/rec-1.json",
            make_revision("rev-1", "rec-1", "f1", json!("v1")),
        )
        .unwrap();
        append(
            &store,
            "records/rec-1.json",
            make_revision("rev-2", "rec-1", "f1", json!("v2")),
        )
        .unwrap();

        let rev = get(&store, "records/rec-1.json", "rec-1", "rev-2").unwrap();
        assert_eq!(rev.unwrap().revision_id, "rev-2");

        let missing = get(&store, "records/rec-1.json", "rec-1", "rev-99").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn list_empty_when_no_sidecar() {
        let store = MemoryStore::empty();
        let revs = list(
            &store,
            "records/rec-none.json",
            "rec-none",
            None,
            None,
            None,
        )
        .unwrap();
        assert!(revs.is_empty());
    }

    #[test]
    fn lifecycle_provenance_roundtrips() {
        let store = MemoryStore::empty();
        let rev = Revision {
            revision_id: "rev-lc".to_string(),
            record_id: "rec-1".to_string(),
            field_id: "f1".to_string(),
            value: json!("new-val"),
            prior_revision_id: None,
            agent: RevisionAgent::Ai,
            provenance: Some(RevisionProvenance {
                lifecycle_transition: Some("active".to_string()),
                transitioned_at: Some("2026-06-01T12:00:00Z".to_string()),
                import_source: None,
            }),
            created_at: "2026-06-01T12:00:00Z".to_string(),
        };
        append(&store, "records/rec-1.json", rev).unwrap();

        let revs = list(&store, "records/rec-1.json", "rec-1", None, None, None).unwrap();
        let p = revs[0].provenance.as_ref().unwrap();
        assert_eq!(p.lifecycle_transition.as_deref(), Some("active"));
    }
}
