//! `ext:discovery` (RFC-012) — the canonical `DiscoveryQuery` shape.
//!
//! Lives in `srs-core` (not `srs-repository`) so it can be the **one** query
//! type consumed by both `srs-repository::discovery_service::find` (the
//! `find`/MCP/bindings entry point) and
//! [`crate::types::view::SectionSource::DiscoveryQuery`] (a Composition
//! section's selection predicate, srs#525 / srs-rust#924's "SectionSource no
//! longer re-implements discovery axes divergently" collapse). `srs-core` has
//! no file I/O and cannot depend on `srs-repository`, so the shared shape has
//! to live on this side of the boundary — `discovery_service` re-exports this
//! type rather than defining its own.

use serde::{Deserialize, Serialize};

/// A conjunction of structured predicates plus an optional content-match floor.
/// Mirrors `DiscoveryQuery` in `docs/schema/2.0/discovery.json`. Unspecified
/// predicates are wildcards.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_id: Option<String>,
    /// AND-conjunction: the instance's tags must contain ALL specified values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tag: Vec<String>,
    /// Exact match on `Record.lifecycleState` (case-sensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<String>,
    /// RFC-012 Rev 11 (srs#525) — inclusive multi-value lifecycle filter, OR
    /// semantics: matches an instance whose `lifecycleState` equals ANY listed
    /// value. Independent of the exact-match `lifecycle_state` predicate above
    /// — the schema documents them as not meant to be combined on one query,
    /// but this is a conjunction mechanism like every other predicate here, so
    /// both are applied (ANDed) when both happen to be set. Formerly
    /// `SectionSource.type-query`'s own `lifecycleStates` axis (RFC-011
    /// Change A); carried forward here by the SectionSource → DiscoveryQuery
    /// collapse (srs#525 / srs-rust#924) as the one query mechanism.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifecycle_states: Vec<String>,
    /// Exclude instances whose `lifecycleState` matches any listed value (RFC-011
    /// parity; applied after `lifecycle_state`/`lifecycle_states`). An empty list
    /// excludes nothing — the "show all" override for an authored default-hidden set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_lifecycle_states: Vec<String>,
    /// Instance tier filter (0=Note, 2=Record — Tier 1/TypedRecord is retired,
    /// srs#448/rfc-decision-53635966, srs-rust#888). Unspecified matches all
    /// tiers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    /// Free-text recall-floor predicate over the Text Projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_match: Option<String>,
}
