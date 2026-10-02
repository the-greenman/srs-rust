//! RFC-046 actor provenance — the ONE place the session actor is resolved and the
//! creation/update diagnostics are decided. Creation services call
//! [`creation_actor`] and stamp the result; update services call
//! [`reconcile_update_extra`] / [`check_update_actor`]. Adapters (CLI, MCP, WASM)
//! only hand the actor to the store ([`RepositoryStore::session_actor`]).

use crate::error::RepositoryError;
use crate::field_type_migration_service::{data_model_revision, RFC046_ACTOR_PROVENANCE_REVISION};
use crate::store::RepositoryStore;
use srs_core::types::actor::Actor;
use std::collections::BTreeMap;

/// The JSON key an envelope-extras bag must never carry on create ([R4]).
pub const CREATED_BY_KEY: &str = "createdBy";

fn refuse(code: &'static str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::ActorProvenance {
        code,
        message: message.into(),
    }
}

/// The session actor, validated ([R1]) but with no corpus-revision check — for
/// operations that create the corpus itself (`repo create`), which is born at the
/// current revision. `None` = unattributed; an invalid actor is `actor-invalid` ([R12]).
pub fn validated_session_actor(
    store: &dyn RepositoryStore,
) -> Result<Option<Actor>, RepositoryError> {
    match store.session_actor() {
        None => Ok(None),
        Some(raw) => Actor::from_value(&raw).map(Some).map_err(|e| {
            refuse(
                "actor-invalid",
                format!("session actor is not a valid Actor ([R1]): {e}"),
            )
        }),
    }
}

/// Resolve the actor a creating operation stamps (`None` = unattributed).
///
/// `supplied` is whether the *request* carried a `createdBy`. Precedence ([R12]):
/// `actor-invalid`, then `actor-supplied`, then `revision-too-old`. Nothing has
/// been written when this errors, so call it before the first write.
pub fn creation_actor(
    store: &dyn RepositoryStore,
    supplied: bool,
) -> Result<Option<Actor>, RepositoryError> {
    let actor = validated_session_actor(store)?;
    if supplied {
        return Err(refuse(
            "actor-supplied",
            "a creation request may not carry createdBy; the implementation stamps it \
             from the session actor ([R4])",
        ));
    }
    if actor.is_some() {
        let revision = data_model_revision(store)?;
        if revision < RFC046_ACTOR_PROVENANCE_REVISION {
            return Err(refuse(
                "revision-too-old",
                format!(
                    "the session has an actor but the corpus is at dataModelRevision {revision} \
                     (< {RFC046_ACTOR_PROVENANCE_REVISION}); run `srs repo apply-migration --id \
                     rfc046-actor-provenance` first ([R11])"
                ),
            ));
        }
    }
    Ok(actor)
}

fn same(request: &serde_json::Value, stored: &Option<Actor>) -> bool {
    match stored {
        None => request.is_null(),
        Some(a) => serde_json::to_value(a).is_ok_and(|v| &v == request),
    }
}

fn changed() -> RepositoryError {
    refuse(
        "actor-changed",
        "an update may carry createdBy only when identical to the stored value ([R5])",
    )
}

/// Update path for an open envelope-extras bag: a `createdBy` key identical to the
/// stored value is dropped (the typed `created_by` already carries it); anything
/// else is `actor-changed`.
pub fn reconcile_update_extra(
    extra: &mut BTreeMap<String, serde_json::Value>,
    stored: &Option<Actor>,
) -> Result<(), RepositoryError> {
    match extra.remove(CREATED_BY_KEY) {
        Some(v) if !same(&v, stored) => Err(changed()),
        _ => Ok(()),
    }
}

/// Update path for a typed whole-object input: absent means "preserve" (returned
/// as the stored value), identical is allowed, anything else is `actor-changed`.
pub fn check_update_actor(
    requested: &Option<Actor>,
    stored: &Option<Actor>,
) -> Result<Option<Actor>, RepositoryError> {
    match requested {
        None => Ok(stored.clone()),
        Some(_) if requested == stored => Ok(stored.clone()),
        Some(_) => Err(changed()),
    }
}
