//! RFC-046 actor provenance: the one `Actor` shape behind `createdBy` on
//! Record, Note and Relation. Testimony, never authority ([R8]).

use serde::{Deserialize, Serialize};

/// The two actor kinds ([R1]) — the actor values of `FieldMeta.source` ([R9]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    Human,
    Ai,
}

/// Who performed an act. Exactly `kind` + `id` (+ optional display `name`);
/// any other property, an unknown kind or an empty `id` fails to parse ([R1]).
/// Compare actors by `id` alone ([R7]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawActor")]
pub struct Actor {
    pub kind: ActorKind,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawActor {
    kind: ActorKind,
    id: String,
    #[serde(default)]
    name: Option<String>,
}

impl TryFrom<RawActor> for Actor {
    type Error = String;
    fn try_from(raw: RawActor) -> Result<Self, String> {
        if raw.id.is_empty() {
            return Err("actor id must be a non-empty string".to_string());
        }
        Ok(Actor {
            kind: raw.kind,
            id: raw.id,
            name: raw.name,
        })
    }
}

impl Actor {
    /// Parse and validate an actor from JSON ([R1]).
    pub fn from_value(value: &serde_json::Value) -> Result<Actor, String> {
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_actor_roundtrips() {
        let v = json!({"kind": "ai", "id": "agent-1", "name": "Scribe"});
        let a = Actor::from_value(&v).unwrap();
        assert_eq!(a.kind, ActorKind::Ai);
        assert_eq!(serde_json::to_value(&a).unwrap(), v);
        let bare = json!({"kind": "human", "id": "u1"});
        assert_eq!(
            serde_json::to_value(Actor::from_value(&bare).unwrap()).unwrap(),
            bare
        );
    }

    /// [R6]: `createdBy` survives a typed parse/serialize round trip on all three
    /// entities, and absence stays absent (no key is invented).
    #[test]
    fn created_by_round_trips_on_record_note_relation() {
        use crate::types::{note::Note, record::Record, relation::Relation};
        let actor = json!({"kind": "ai", "id": "agent-1", "name": "Scribe"});
        let record = json!({
            "instanceId": "r", "typeId": "t", "typeVersion": 1, "typeNamespace": "n",
            "typeName": "x", "fieldValues": {}, "createdBy": actor
        });
        let r: Record = serde_json::from_value(record.clone()).unwrap();
        assert!(r.created_by.is_some() && !r.extra.contains_key("createdBy"));
        assert_eq!(serde_json::to_value(&r).unwrap(), record);

        let note = json!({"instanceId": "n", "sections": [], "createdBy": actor});
        let n: Note = serde_json::from_value(note.clone()).unwrap();
        assert_eq!(serde_json::to_value(&n).unwrap(), note);

        let rel = json!({"relationId": "e", "relationType": "evidences",
            "sourceInstanceId": "a", "targetInstanceId": "b", "createdBy": actor});
        let e: Relation = serde_json::from_value(rel.clone()).unwrap();
        assert_eq!(serde_json::to_value(&e).unwrap(), rel);

        let bare: Relation = serde_json::from_value(json!({"relationId": "e",
            "relationType": "evidences", "sourceInstanceId": "a", "targetInstanceId": "b"}))
        .unwrap();
        assert!(serde_json::to_value(&bare)
            .unwrap()
            .get("createdBy")
            .is_none());
        // An invalid actor shape fails to parse on every entity ([R1]).
        assert!(serde_json::from_value::<Relation>(json!({"relationId": "e",
            "relationType": "evidences", "sourceInstanceId": "a", "targetInstanceId": "b",
            "createdBy": "someone"}))
        .is_err());
    }

    #[test]
    fn invalid_actors_are_rejected() {
        for bad in [
            json!({"kind": "ai", "id": ""}),
            json!({"kind": "bot", "id": "x"}),
            json!({"kind": "ai", "id": "x", "extra": 1}),
            json!({"kind": "ai"}),
            json!("someone"),
        ] {
            assert!(Actor::from_value(&bad).is_err(), "{bad} must be invalid");
        }
    }
}
