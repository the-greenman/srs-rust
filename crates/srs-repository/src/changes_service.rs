//! Changes since a baseline (srs-rust#1522): what is unsaved, and what changed since the
//! last save or since an actor last looked.
//!
//! Held in memory by the open store, shared by every clone of it (a browser's repository
//! handle and its MCP sessions), like [`crate::attention_service`]. Never written to disk and
//! never part of a corpus, so it needs no spec change (the srs-rust#1202 tripwire); a cursor
//! that outlives the session would (srs#915). There is no CLI surface: a one-shot process has
//! no session to hold a baseline in.
//!
//! The `saved` baseline is set by the host after it persists the document ([`mark_saved`]).
//! Each actor's cursor is keyed by the store's session actor, never a request field, so an
//! actor can only move its own cursor.

use crate::actor_service::validated_session_actor;
use crate::diff::{diff_snapshots, DiffSummary, ManifestDiff};
use crate::error::RepositoryError;
use crate::repository_portability::{export_repository_snapshot, RepositorySnapshot};
use crate::store::{ChangeKind, RepositoryStore};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// The live repository as it was at some point, and the write epoch it had then.
// ponytail: a full snapshot per baseline; derive from the write recorder if srs-rust#1193
// measures it as a cost on large repositories.
#[derive(Debug, Clone)]
struct Baseline {
    snapshot: Rc<RepositorySnapshot>,
    epoch: u64,
}

/// The host's save point: the baseline and the time the host gave for it.
#[derive(Debug, Clone)]
struct SavePoint {
    baseline: Baseline,
    saved_at: Option<String>,
}

/// The shared in-memory board. Cloning shares the same baselines.
#[derive(Debug, Clone, Default)]
pub struct BaselineBoard {
    saved: Rc<RefCell<Option<SavePoint>>>,
    cursors: Rc<RefCell<BTreeMap<String, Baseline>>>,
}

/// Which baseline a [`changes`] read compares the live repository with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum ChangesSince {
    /// The host's last save (or the document as opened, if the host marked that).
    Saved,
    /// The calling actor's own last `changes` read with `since: cursor`.
    Cursor,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceChange {
    pub instance_id: String,
    pub change: ChangeKind,
    pub tier: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationChange {
    pub relation_id: String,
    pub change: ChangeKind,
    pub relation_type: String,
    pub source_instance_id: String,
    pub target_instance_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerChange {
    pub container_id: String,
    pub change: ChangeKind,
    pub title: String,
}

/// What [`changes`] returns. Lists are sorted by id and carry no full values: read an entity
/// to see it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesResult {
    pub since: ChangesSince,
    /// Whether a save point was recorded. `false` means none was: a host writing straight to
    /// disk never records one, because every write is saved as it is made.
    pub tracked: bool,
    /// Whether anything was written since the save point (`null` when not tracked). The same
    /// signal as an editor's dirty flag, so it follows the write epoch, not the lists: a write
    /// later undone by hand, or an attachment (not listed), still counts.
    pub unsaved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
    pub write_epoch: u64,
    /// `since: cursor` only: this read started the caller's cursor, so nothing is listed yet.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cursor_started: bool,
    pub summary: DiffSummary,
    pub manifest: ManifestDiff,
    pub instances: Vec<InstanceChange>,
    pub relations: Vec<RelationChange>,
    /// Container level: a container whose entries or fields changed is `updated`.
    pub containers: Vec<ContainerChange>,
}

fn board(store: &dyn RepositoryStore) -> Result<BaselineBoard, RepositoryError> {
    store
        .baseline_board()
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: "this store holds no session baselines".to_string(),
        })
}

fn capture(store: &dyn RepositoryStore) -> Result<Baseline, RepositoryError> {
    Ok(Baseline {
        snapshot: Rc::new(export_repository_snapshot(store)?),
        epoch: store.write_epoch(),
    })
}

/// Record the repository as it is now as the save point. The host calls this after a
/// successful save or export, and when it opens a document.
pub fn mark_saved(
    store: &dyn RepositoryStore,
    saved_at: Option<String>,
) -> Result<(), RepositoryError> {
    let board = board(store)?;
    let baseline = capture(store)?;
    *board.saved.borrow_mut() = Some(SavePoint { baseline, saved_at });
    Ok(())
}

/// What changed since the save point or the session actor's cursor. A cursor read starts
/// the cursor on first use and moves it to now on every read.
pub fn changes(
    store: &dyn RepositoryStore,
    since: ChangesSince,
) -> Result<ChangesResult, RepositoryError> {
    let board = board(store)?;
    let write_epoch = store.write_epoch();
    let saved = board.saved.borrow().clone();
    let (tracked, unsaved, saved_at) = match &saved {
        Some(p) => (
            true,
            Some(p.baseline.epoch != write_epoch),
            p.saved_at.clone(),
        ),
        None => (false, None, None),
    };
    let live = capture(store)?;
    let (base, cursor_started) = match since {
        ChangesSince::Saved => match saved {
            Some(p) => (p.baseline, false),
            // Nothing to compare with: report the status and no changes.
            None => (live.clone(), false),
        },
        ChangesSince::Cursor => {
            let actor =
                validated_session_actor(store)?.ok_or_else(|| RepositoryError::InvalidInput {
                    message:
                        "a cursor needs a session actor; the host sets one before the session reads"
                            .to_string(),
                })?;
            let previous = board.cursors.borrow_mut().insert(actor.id, live.clone());
            match previous {
                Some(b) => (b, false),
                None => (live.clone(), true),
            }
        }
    };
    let diff = diff_snapshots(&base.snapshot, &live.snapshot);
    let (instances, relations, containers) = compact(&diff);
    Ok(ChangesResult {
        since,
        tracked,
        unsaved,
        saved_at,
        write_epoch,
        cursor_started,
        summary: diff.summary,
        manifest: diff.manifest,
        instances,
        relations,
        containers,
    })
}

/// The diff's entities as id-sorted lists without their values.
fn compact(
    diff: &crate::diff::RepoDiff,
) -> (
    Vec<InstanceChange>,
    Vec<RelationChange>,
    Vec<ContainerChange>,
) {
    let instance = |id: &str, tier: u8, change: ChangeKind, v: &serde_json::Value| {
        let title = v
            .get("title")
            .or_else(|| v.pointer("/fieldValues/title"))
            .and_then(|t| t.as_str())
            .map(str::to_string);
        let type_name = match (v.get("typeNamespace"), v.get("typeName")) {
            (Some(ns), Some(n)) => Some(format!(
                "{}/{}",
                ns.as_str().unwrap_or(""),
                n.as_str().unwrap_or("")
            )),
            _ => None,
        };
        InstanceChange {
            instance_id: id.to_string(),
            change,
            tier,
            title,
            type_name,
            created_by: v.get("createdBy").cloned(),
        }
    };
    let mut instances: Vec<_> = diff
        .instances
        .added
        .iter()
        .map(|i| instance(&i.instance_id, i.tier, ChangeKind::Created, &i.value))
        .chain(
            diff.instances
                .modified
                .iter()
                .map(|i| instance(&i.instance_id, i.tier, ChangeKind::Updated, &i.to_value)),
        )
        .chain(
            diff.instances
                .removed
                .iter()
                .map(|i| instance(&i.instance_id, i.tier, ChangeKind::Deleted, &i.value)),
        )
        .collect();
    instances.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));

    let relation = |id: &str, change: ChangeKind, v: &serde_json::Value| {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        RelationChange {
            relation_id: id.to_string(),
            change,
            relation_type: s("relationType"),
            source_instance_id: s("sourceInstanceId"),
            target_instance_id: s("targetInstanceId"),
        }
    };
    let mut relations: Vec<_> = diff
        .relations
        .added
        .iter()
        .map(|r| relation(&r.relation_id, ChangeKind::Created, &r.value))
        .chain(
            diff.relations
                .modified
                .iter()
                .map(|r| relation(&r.relation_id, ChangeKind::Updated, &r.to_value)),
        )
        .chain(
            diff.relations
                .removed
                .iter()
                .map(|r| relation(&r.relation_id, ChangeKind::Deleted, &r.value)),
        )
        .collect();
    relations.sort_by(|a, b| a.relation_id.cmp(&b.relation_id));

    let container = |change: ChangeKind| {
        move |c: &crate::diff::ContainerChanged| ContainerChange {
            container_id: c.container_id.clone(),
            change,
            title: c.title.clone(),
        }
    };
    let mut containers: Vec<_> = diff
        .containers
        .added
        .iter()
        .map(container(ChangeKind::Created))
        .chain(
            diff.containers
                .modified
                .iter()
                .map(container(ChangeKind::Updated)),
        )
        .chain(
            diff.containers
                .removed
                .iter()
                .map(container(ChangeKind::Deleted)),
        )
        .collect();
    containers.sort_by(|a, b| a.container_id.cmp(&b.container_id));
    (instances, relations, containers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_lifecycle::{create_blank_repository, CreateBlankRepositoryInput};
    use crate::services::create_note;
    use crate::store::FileStore;
    use serde_json::json;
    use srs_core::types::note::{Note, NoteSection};

    fn repo() -> FileStore {
        let s = crate::tree_session::new_tree_session();
        create_blank_repository(
            &s,
            CreateBlankRepositoryInput {
                namespace: Some("com.example.changes".into()),
                ..Default::default()
            },
        )
        .unwrap();
        s
    }

    fn note(store: &dyn RepositoryStore, title: &str) -> String {
        let n = Note {
            created_by: None,
            instance_id: String::new(),
            title: Some(title.to_string()),
            tags: None,
            sections: vec![NoteSection {
                name: "body".into(),
                label: None,
                content: "text".into(),
                content_hint: None,
                tags: None,
            }],
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        };
        create_note(store, n).unwrap().note.instance_id
    }

    #[test]
    fn untracked_until_the_host_marks_a_save_point() {
        let s = repo();
        note(&s, "First");
        let r = changes(&s, ChangesSince::Saved).unwrap();
        assert!(!r.tracked);
        assert_eq!(r.unsaved, None);
        assert!(r.instances.is_empty());
    }

    #[test]
    fn a_write_after_the_save_point_is_unsaved_and_listed_until_saved_again() {
        let s = repo();
        mark_saved(&s, Some("2026-10-29T12:00:00Z".into())).unwrap();
        assert_eq!(
            changes(&s, ChangesSince::Saved).unwrap().unsaved,
            Some(false)
        );

        let agent = s.clone();
        agent.set_session_actor(Some(json!({"kind":"ai","id":"agent:a"})));
        let id = note(&agent, "Added by an agent");

        let r = changes(&s, ChangesSince::Saved).unwrap();
        assert_eq!(r.unsaved, Some(true));
        assert_eq!(r.saved_at.as_deref(), Some("2026-10-29T12:00:00Z"));
        assert_eq!(r.instances.len(), 1);
        let i = &r.instances[0];
        assert_eq!(i.instance_id, id);
        assert_eq!(i.change, ChangeKind::Created);
        assert_eq!(i.title.as_deref(), Some("Added by an agent"));
        assert_eq!(i.created_by.as_ref().unwrap()["id"], "agent:a");
        assert_eq!(r.summary.instances_added, 1);

        mark_saved(&s, None).unwrap();
        let r = changes(&s, ChangesSince::Saved).unwrap();
        assert_eq!(r.unsaved, Some(false));
        assert!(r.instances.is_empty() && r.relations.is_empty() && r.containers.is_empty());
    }

    #[test]
    fn each_actors_cursor_is_its_own() {
        let s = repo();
        let a = s.clone();
        a.set_session_actor(Some(json!({"kind":"ai","id":"agent:a"})));
        let b = s.clone();
        b.set_session_actor(Some(json!({"kind":"ai","id":"agent:b"})));

        assert!(changes(&a, ChangesSince::Cursor).unwrap().cursor_started);
        assert!(changes(&b, ChangesSince::Cursor).unwrap().cursor_started);
        note(&a, "One");

        // A reads first; that must not hide the change from B.
        assert_eq!(
            changes(&a, ChangesSince::Cursor).unwrap().instances.len(),
            1
        );
        assert!(changes(&a, ChangesSince::Cursor)
            .unwrap()
            .instances
            .is_empty());
        let rb = changes(&b, ChangesSince::Cursor).unwrap();
        assert!(!rb.cursor_started);
        assert_eq!(rb.instances.len(), 1);
    }

    #[test]
    fn a_cursor_needs_a_session_actor() {
        let s = repo();
        assert!(changes(&s, ChangesSince::Cursor).is_err());
    }

    #[test]
    fn baselines_are_not_writes() {
        let s = repo();
        let e = s.write_epoch();
        mark_saved(&s, None).unwrap();
        changes(&s, ChangesSince::Saved).unwrap();
        assert_eq!(s.write_epoch(), e);
    }
}
