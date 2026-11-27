//! Small serde helpers shared by input contracts.

use serde::{Deserialize, Deserializer};

/// Deserializes a present field (including JSON `null`) as `Some(_)`, so the caller can tell
/// "key omitted" (`None`, via `#[serde(default)]`) apart from "key sent as `null`" (`Some(None)`).
pub fn deserialize_some<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}
