//! The one builder for `srs://<repositoryId>/…` resource URIs of instances, types and
//! containers. `srs-mcp-core::uri` (parse + every other kind) delegates here, so a hit,
//! a neighbour and an MCP resource all spell the same URI (srs-rust#1227).
//! Scope: instance-addressed kinds only; singleton kinds (map, tree, ...) stay in `srs-mcp-core::uri`.

use crate::manifest::Manifest;

const SCHEME: &str = "srs://";

pub fn record_uri(repository_id: &str, instance_id: &str) -> String {
    format!("{SCHEME}{repository_id}/record/{instance_id}")
}

pub fn type_uri(repository_id: &str, type_id: &str) -> String {
    format!("{SCHEME}{repository_id}/type/{type_id}")
}

pub fn container_uri(repository_id: &str, container_id: &str) -> String {
    format!("{SCHEME}{repository_id}/container/{container_id}")
}

pub fn relation_types_uri(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/relation-types")
}

/// The manifest's `repositoryId`, if it has one.
pub fn repository_id(manifest: &Manifest) -> Option<&str> {
    manifest.extra.get("repositoryId").and_then(|v| v.as_str())
}
