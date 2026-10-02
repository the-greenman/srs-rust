//! Session write guard (srs-rust#1165): application policy for agent sessions.
//!
//! Protected containers and records are read-only to the session, except that
//! `fillOnlyFields` may be set while unset. Membership is resolved at write time
//! through `container_service::list_members`, never snapshotted.

use crate::tools::{
    self, ContainerIdToolInput, ContainerMemberAddToolInput, ContainerMemberMoveToolInput,
    ContainerMemberToolInput, NoteCreateToolInput, NoteGraduateToolInput, RecordCreateToolInput,
    RecordSuccessorToolInput, RecordTransitionToolInput, RecordUpdateToolInput,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{Map, Value};
use srs_repository::{container_service, record_store, store::RepositoryStore};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteGuard {
    /// Guarded containers: the container itself and every member at write time.
    #[serde(default)]
    pub container_ids: Vec<String>,
    /// Individually guarded records.
    #[serde(default)]
    pub instance_ids: Vec<String>,
    /// `Field.name`s writable on guarded records only while unset.
    #[serde(default)]
    pub fill_only_fields: Vec<String>,
}

/// Unset = absent, null or "".
fn current(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null() && v.as_str() != Some(""))
}

fn parse<T: DeserializeOwned>(args: &Map<String, Value>) -> Option<T> {
    // A malformed call is left for the tool's own parse to report.
    serde_json::from_value(Value::Object(args.clone())).ok()
}

impl WriteGuard {
    fn deny(&self, what: &str) -> String {
        format!("Rejected by the session write guard: {what}")
    }

    fn container_guarded(&self, id: &str) -> bool {
        self.container_ids.iter().any(|c| c == id)
    }

    fn record_guarded(&self, store: &dyn RepositoryStore, id: &str) -> bool {
        self.instance_ids.iter().any(|i| i == id)
            || self.container_ids.iter().any(|c| {
                container_service::list_members(store, c)
                    .is_ok_and(|members| members.iter().any(|m| m == id))
            })
    }

    /// `Err(message)` when the session's write guard forbids this tool call.
    pub fn check(
        &self,
        store: &dyn RepositoryStore,
        name: &str,
        args: Option<&Map<String, Value>>,
    ) -> Result<(), String> {
        let Some(args) = args else { return Ok(()) };
        let locked = |id: &str, tool: &str| {
            Err(self.deny(&format!(
                "record '{id}' is protected; {tool} is not allowed on it"
            )))
        };
        let container_locked = |id: &str, tool: &str| {
            Err(self.deny(&format!(
                "container '{id}' is protected; {tool} is not allowed on it"
            )))
        };
        match name {
            tools::TOOL_RECORD_UPDATE => {
                let Some(input) = parse::<RecordUpdateToolInput>(args) else {
                    return Ok(());
                };
                if !self.record_guarded(store, &input.instance_id) {
                    return Ok(());
                }
                self.check_update(store, &input)
            }
            tools::TOOL_RECORD_TRANSITION => match parse::<RecordTransitionToolInput>(args) {
                Some(i) if self.record_guarded(store, &i.instance_id) => {
                    locked(&i.instance_id, name)
                }
                _ => Ok(()),
            },
            tools::TOOL_RECORD_SUCCESSOR => match parse::<RecordSuccessorToolInput>(args) {
                Some(i) if self.record_guarded(store, &i.predecessor_id) => {
                    locked(&i.predecessor_id, name)
                }
                _ => Ok(()),
            },
            tools::TOOL_CONTAINER_MEMBER_ADD => match parse::<ContainerMemberAddToolInput>(args) {
                Some(i) if self.container_guarded(&i.container_id) => {
                    container_locked(&i.container_id, name)
                }
                _ => Ok(()),
            },
            tools::TOOL_CONTAINER_MEMBER_REMOVE => match parse::<ContainerMemberToolInput>(args) {
                Some(i) if self.container_guarded(&i.container_id) => {
                    container_locked(&i.container_id, name)
                }
                _ => Ok(()),
            },
            tools::TOOL_CONTAINER_MEMBER_MOVE => {
                match parse::<ContainerMemberMoveToolInput>(args) {
                    Some(i) if self.container_guarded(&i.container_id) => {
                        container_locked(&i.container_id, name)
                    }
                    _ => Ok(()),
                }
            }
            tools::TOOL_CONTAINER_MEMBER_REPAIR => match parse::<ContainerIdToolInput>(args) {
                Some(i) if self.container_guarded(&i.container_id) => {
                    container_locked(&i.container_id, name)
                }
                _ => Ok(()),
            },
            // Creating an instance is allowed, but filing it into a guarded
            // container is a membership write.
            tools::TOOL_RECORD_CREATE => self.check_new_in(
                parse::<RecordCreateToolInput>(args).and_then(|i| i.container_id),
                name,
            ),
            tools::TOOL_NOTE_CREATE => self.check_new_in(
                parse::<NoteCreateToolInput>(args).and_then(|i| i.container_id),
                name,
            ),
            tools::TOOL_NOTE_GRADUATE => self.check_new_in(
                parse::<NoteGraduateToolInput>(args).and_then(|i| i.container_id),
                name,
            ),
            _ => Ok(()),
        }
    }

    fn check_new_in(&self, container_id: Option<String>, tool: &str) -> Result<(), String> {
        match container_id {
            Some(c) if self.container_guarded(&c) => Err(self.deny(&format!(
                "container '{c}' is protected; {tool} may not add a new instance to it"
            ))),
            _ => Ok(()),
        }
    }

    /// `record_update` replaces the whole `fieldValues`, so compare against the
    /// stored record: only a fill-only field that is currently unset may differ.
    fn check_update(
        &self,
        store: &dyn RepositoryStore,
        input: &RecordUpdateToolInput,
    ) -> Result<(), String> {
        let id = &input.instance_id;
        if input.tags.is_some() || input.type_version.is_some() || input.meta.is_some() {
            return Err(self.deny(&format!(
                "record '{id}' is protected; tags, typeVersion and meta cannot be changed"
            )));
        }
        let Ok(Some(record)) = record_store::get_record_by_id(store, id) else {
            return Ok(()); // the service reports a missing / unloadable record
        };
        let new = &input.field_values;
        let old = &record.field_values.0;
        let changed = new
            .keys()
            .chain(old.keys())
            .filter(|k| current(new.get(*k)) != current(old.get(*k)));
        for key in changed {
            if !self.fill_only_fields.contains(key) {
                return Err(self.deny(&format!(
                    "record '{id}' is protected; field '{key}' cannot be changed"
                )));
            }
            if current(old.get(key)).is_some() {
                return Err(self.deny(&format!(
                    "record '{id}' is protected; fill-only field '{key}' is already set"
                )));
            }
        }
        if let Some(meta) = &input.field_meta {
            if let Some(key) = meta.keys().find(|k| !self.fill_only_fields.contains(*k)) {
                return Err(self.deny(&format!(
                    "record '{id}' is protected; fieldMeta for '{key}' cannot be changed"
                )));
            }
        }
        Ok(())
    }
}
