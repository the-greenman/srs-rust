//! Session write guard (srs-rust#1165): application policy for agent sessions.
//!
//! Protected containers and records are read-only to the session, except that
//! `fillOnlyFields` may be set while unset. Membership is resolved at write time
//! through `container_service::list_members`, never snapshotted. Session-only:
//! the stdio `srs mcp serve` has no guard (deferred per the issue).

use crate::tools::{
    self, ContainerIdToolInput, ContainerMemberAddToolInput, ContainerMemberMoveToolInput,
    ContainerMemberToolInput, NoteCreateToolInput, NoteGraduateToolInput, RecordCreateToolInput,
    RecordSuccessorToolInput, RecordTransitionToolInput, RecordUpdateToolInput,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{Map, Value};
use srs_repository::container_service::{self, ContainerCreateInput};
use srs_repository::{record_store, store::RepositoryStore};

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

    /// Guarded containers plus their `childContainerIds` closure. A container
    /// that cannot be loaded fails closed: `Err` rejects the write.
    fn guarded_containers(&self, store: &dyn RepositoryStore) -> Result<Vec<String>, String> {
        let mut all = Vec::new();
        for c in &self.container_ids {
            let closure = container_service::container_closure(store, c)
                .map_err(|e| self.deny(&format!("cannot resolve guarded container '{c}': {e}")))?;
            all.extend(closure);
        }
        Ok(all)
    }

    fn container_guarded(&self, store: &dyn RepositoryStore, id: &str) -> Result<bool, String> {
        Ok(self.guarded_containers(store)?.iter().any(|c| c == id))
    }

    fn record_guarded(&self, store: &dyn RepositoryStore, id: &str) -> Result<bool, String> {
        if self.instance_ids.iter().any(|i| i == id) {
            return Ok(true);
        }
        for c in &self.container_ids {
            let members = container_service::list_members(store, c)
                .map_err(|e| self.deny(&format!("cannot resolve guarded container '{c}': {e}")))?;
            if members.iter().any(|m| m == id) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `Err(message)` when the session's write guard forbids this tool call.
    pub fn check(
        &self,
        store: &dyn RepositoryStore,
        name: &str,
        args: Option<&Map<String, Value>>,
    ) -> Result<(), String> {
        let Some(args) = args else { return Ok(()) };
        // Container structure writes and instance creation filed into a container.
        let container = match name {
            tools::TOOL_CONTAINER_MEMBER_ADD => {
                parse::<ContainerMemberAddToolInput>(args).map(|i| i.container_id)
            }
            tools::TOOL_CONTAINER_MEMBER_REMOVE => {
                parse::<ContainerMemberToolInput>(args).map(|i| i.container_id)
            }
            tools::TOOL_CONTAINER_MEMBER_MOVE => {
                parse::<ContainerMemberMoveToolInput>(args).map(|i| i.container_id)
            }
            tools::TOOL_RECORD_FORK => {
                parse::<tools::RecordForkToolInput>(args).map(|i| i.container_id)
            }
            tools::TOOL_CONTAINER_MEMBER_REPAIR => {
                parse::<ContainerIdToolInput>(args).map(|i| i.container_id)
            }
            tools::TOOL_RECORD_CREATE => {
                parse::<RecordCreateToolInput>(args).and_then(|i| i.container_id)
            }
            tools::TOOL_NOTE_CREATE => {
                parse::<NoteCreateToolInput>(args).and_then(|i| i.container_id)
            }
            tools::TOOL_NOTE_GRADUATE => {
                parse::<NoteGraduateToolInput>(args).and_then(|i| i.container_id)
            }
            _ => None,
        };
        if let Some(c) = container {
            if self.container_guarded(store, &c)? {
                return Err(self.deny(&format!(
                    "container '{c}' is protected; {name} is not allowed on it"
                )));
            }
            return Ok(());
        }
        // `container_create` over an existing id replaces that container (core
        // callers rely on create-as-upsert), so a guarded id is rejected here.
        if name == tools::TOOL_CONTAINER_CREATE {
            if let Some(c) = parse::<ContainerCreateInput>(args).and_then(|i| i.container_id) {
                if self.container_guarded(store, &c)? {
                    return Err(self.deny(&format!(
                        "container '{c}' is protected; {name} would overwrite it"
                    )));
                }
            }
            return Ok(());
        }
        // `container_copy` only reads its source; it is rejected only when the NEW id is guarded.
        if name == tools::TOOL_CONTAINER_COPY {
            if let Some(c) =
                parse::<tools::ContainerCopyToolInput>(args).and_then(|i| i.container_id)
            {
                if self.container_guarded(store, &c)? {
                    return Err(self.deny(&format!(
                        "container '{c}' is protected; {name} would overwrite it"
                    )));
                }
            }
            return Ok(());
        }
        let record = match name {
            tools::TOOL_RECORD_UPDATE => {
                parse::<RecordUpdateToolInput>(args).map(|i| i.instance_id)
            }
            tools::TOOL_RECORD_TRANSITION => {
                parse::<RecordTransitionToolInput>(args).map(|i| i.instance_id)
            }
            tools::TOOL_RECORD_SUCCESSOR => {
                parse::<RecordSuccessorToolInput>(args).map(|i| i.predecessor_id)
            }
            _ => None,
        };
        let Some(id) = record else { return Ok(()) };
        if !self.record_guarded(store, &id)? {
            return Ok(());
        }
        if name == tools::TOOL_RECORD_UPDATE {
            if let Some(input) = parse::<RecordUpdateToolInput>(args) {
                return self.check_update(store, input);
            }
        }
        Err(self.deny(&format!(
            "record '{id}' is protected; {name} is not allowed on it"
        )))
    }

    /// `record_update` replaces the whole `fieldValues` and `fieldMeta`, so
    /// compare against the stored record: only a fill-only field that is
    /// currently unset may differ.
    fn check_update(
        &self,
        store: &dyn RepositoryStore,
        input: RecordUpdateToolInput,
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
        // Provenance of every non-fill field must survive a whole-map replace.
        if let Some(meta) = tools::field_meta_map(input.field_meta) {
            let stored = serde_json::to_value(&record.field_meta).unwrap_or(Value::Null);
            let sent = serde_json::to_value(&meta).unwrap_or(Value::Null);
            let keep = |v: &Value| {
                let mut m = v.as_object().cloned().unwrap_or_default();
                m.retain(|k, _| !self.fill_only_fields.contains(k));
                m
            };
            if keep(&stored) != keep(&sent) {
                return Err(self.deny(&format!(
                    "record '{id}' is protected; fieldMeta of non-fill fields cannot be changed"
                )));
            }
        }
        Ok(())
    }
}
