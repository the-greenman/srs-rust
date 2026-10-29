//! Session AttentionState (srs-rust#1344): where each actor is looking right now.
//!
//! Held in memory by the open store, shared by every clone of it (a browser's repository
//! handle and its MCP sessions), keyed by the RFC-046 actor `id`. Never written to disk,
//! never stamped on a record, relation or note, and never part of a corpus; persistence is
//! a spec question owned by srs#915. There is no CLI surface: a one-shot process has no
//! session to hold attention in.
//!
//! The actor is always the store's session actor ([`crate::actor_service`]), never a request
//! field, so an actor can only move its own focus.

use crate::actor_service::validated_session_actor;
use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use serde::Serialize;
use srs_core::types::actor::Actor;
use srs_core::types::address::AttentionState;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

/// One actor's current focus. `seq` rises on every change anywhere on the board, so a
/// poller can tell "moved" from "same as last read" without a clock.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorAttention {
    pub actor: Actor,
    pub attention: AttentionState,
    pub seq: u64,
}

/// The shared in-memory board. Cloning shares the same entries.
#[derive(Debug, Clone, Default)]
pub struct AttentionBoard {
    entries: Rc<RefCell<BTreeMap<String, ActorAttention>>>,
    seq: Rc<Cell<u64>>,
}

impl AttentionBoard {
    fn next_seq(&self) -> u64 {
        let next = self.seq.get() + 1;
        self.seq.set(next);
        next
    }
}

/// Everyone's current attention, ordered by actor id.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionList {
    pub attention: Vec<ActorAttention>,
}

fn board(store: &dyn RepositoryStore) -> Result<AttentionBoard, RepositoryError> {
    store
        .attention_board()
        .ok_or_else(|| RepositoryError::InvalidInput {
            message: "this store holds no session attention".to_string(),
        })
}

fn session_actor(store: &dyn RepositoryStore) -> Result<Actor, RepositoryError> {
    validated_session_actor(store)?.ok_or_else(|| RepositoryError::InvalidInput {
        message: "attention needs a session actor; the host sets one before the session writes"
            .to_string(),
    })
}

/// Set the session actor's attention, replacing its previous one.
pub fn set_attention(
    store: &dyn RepositoryStore,
    attention: AttentionState,
) -> Result<ActorAttention, RepositoryError> {
    if attention.container_id.is_empty() {
        return Err(RepositoryError::InvalidInput {
            message: "attention containerId must be non-empty".to_string(),
        });
    }
    let actor = session_actor(store)?;
    let board = board(store)?;
    let entry = ActorAttention {
        actor,
        attention,
        seq: board.next_seq(),
    };
    board
        .entries
        .borrow_mut()
        .insert(entry.actor.id.clone(), entry.clone());
    Ok(entry)
}

/// Clear the session actor's attention. Returns whether it had one.
pub fn clear_attention(store: &dyn RepositoryStore) -> Result<bool, RepositoryError> {
    let actor = session_actor(store)?;
    let board = board(store)?;
    let removed = board.entries.borrow_mut().remove(&actor.id).is_some();
    if removed {
        board.next_seq();
    }
    Ok(removed)
}

/// The current attention of every actor. Empty when nobody has set one.
pub fn list_attention(store: &dyn RepositoryStore) -> Result<AttentionList, RepositoryError> {
    let board = board(store)?;
    let attention = board.entries.borrow().values().cloned().collect();
    Ok(AttentionList { attention })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::FileStore;
    use crate::vfs::MemVfs;
    use serde_json::json;
    use std::rc::Rc;

    fn store() -> FileStore {
        FileStore::from_vfs(Rc::new(MemVfs::new()))
    }

    fn focus(container: &str, record: Option<&str>) -> AttentionState {
        AttentionState {
            container_id: container.to_string(),
            record_id: record.map(str::to_string),
            field_id: None,
            protocol_run_id: None,
            stage_id: None,
        }
    }

    #[test]
    fn a_clone_reads_what_another_handle_set() {
        let writer = store();
        let reader = writer.clone();
        writer.set_session_actor(Some(json!({"kind":"human","id":"u1"})));
        reader.set_session_actor(Some(json!({"kind":"ai","id":"a1"})));
        set_attention(&writer, focus("c1", Some("r1"))).unwrap();
        set_attention(&reader, focus("c1", Some("r2"))).unwrap();
        let all = list_attention(&reader).unwrap().attention;
        let ids: Vec<_> = all.iter().map(|a| a.actor.id.as_str()).collect();
        assert_eq!(ids, ["a1", "u1"]);
        assert_eq!(all[1].attention.record_id.as_deref(), Some("r1"));
    }

    #[test]
    fn setting_again_replaces_and_bumps_seq() {
        let s = store();
        s.set_session_actor(Some(json!({"kind":"human","id":"u1"})));
        let first = set_attention(&s, focus("c1", Some("r1"))).unwrap();
        let second = set_attention(&s, focus("c1", Some("r2"))).unwrap();
        assert!(second.seq > first.seq);
        let all = list_attention(&s).unwrap().attention;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].attention.record_id.as_deref(), Some("r2"));
    }

    #[test]
    fn clear_removes_only_the_session_actors_entry() {
        let s = store();
        s.set_session_actor(Some(json!({"kind":"human","id":"u1"})));
        set_attention(&s, focus("c1", None)).unwrap();
        let other = s.clone();
        other.set_session_actor(Some(json!({"kind":"ai","id":"a1"})));
        set_attention(&other, focus("c1", None)).unwrap();
        assert!(clear_attention(&s).unwrap());
        assert!(!clear_attention(&s).unwrap());
        let all = list_attention(&s).unwrap().attention;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].actor.id, "a1");
    }

    #[test]
    fn needs_a_valid_session_actor_and_a_container() {
        let s = store();
        assert!(set_attention(&s, focus("c1", None)).is_err());
        s.set_session_actor(Some(json!({"kind":"ai","id":""})));
        assert!(set_attention(&s, focus("c1", None)).is_err());
        s.set_session_actor(Some(json!({"kind":"ai","id":"a1"})));
        assert!(set_attention(&s, focus("", None)).is_err());
        assert!(list_attention(&s).unwrap().attention.is_empty());
    }

    #[test]
    fn attention_is_not_a_write() {
        let s = store();
        s.set_session_actor(Some(json!({"kind":"ai","id":"a1"})));
        let epoch = s.write_epoch();
        set_attention(&s, focus("c1", None)).unwrap();
        assert_eq!(s.write_epoch(), epoch);
    }
}
