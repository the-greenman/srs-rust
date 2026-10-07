//! How much of each instance a read inlines (srs-rust#1285, #1286). One vocabulary for every
//! read that returns other instances: the neighbours of `context record` and the hits of
//! `find`/`similar`. `full` is the default and the complete shape; `card` and `label` exist so
//! an agent can afford to look before it reads.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcp-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Projection {
    /// Everything: a context neighbour as the whole Record/Note with edge labels and
    /// provenance; a find hit with its typeId, containerIds and matchedFields.
    #[default]
    Full,
    /// Enough to decide what to read next: id, uri, label, type and lifecycle state, plus one
    /// line of text (a context neighbour's summary field, a find hit's snippet and score).
    Card,
    /// As `card`, without the line of text: identity and label only.
    Label,
}

impl std::str::FromStr for Projection {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "full" => Ok(Projection::Full),
            "card" => Ok(Projection::Card),
            "label" => Ok(Projection::Label),
            other => Err(format!(
                "invalid projection '{other}' (expected full|card|label)"
            )),
        }
    }
}
