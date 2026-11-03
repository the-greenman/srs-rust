//! Routing a request to one repository of a boundary (RFC-049 [R13], [R16], [R17]).
//!
//! Shared by every MCP adapter that serves more than one repository: which repository
//! a tool call names, whether it writes, the advertised `repositoryId` parameter, and
//! the `write-refused` code. The boundary itself is `srs_repository::boundary_service`.

use serde_json::{json, Map, Value};

use crate::tools::{self, ToolProfile};
use crate::McpApplicationError;

/// A tool call's routing id and its remaining arguments.
pub type Routed = (Option<String>, Option<Map<String, Value>>);

/// Whether `name` is a tool `profile` advertises (checked before routing, so an unknown
/// tool is reported as unknown, never as a boundary refusal).
pub fn known_tool(profile: ToolProfile, name: &str) -> bool {
    tools::list_tools_for(profile)["tools"]
        .as_array()
        .is_some_and(|ts| ts.iter().any(|t| t["name"] == name))
}

/// Whether the tool `name` writes: every tool outside the `read` profile.
pub fn is_write(name: &str) -> bool {
    !ToolProfile::Read.allows(name)
}

/// Split a tool call's routing argument from its own arguments: `repositoryId`, or for
/// `read` the repository segment of its `uri`. The tool never sees `repositoryId`. A
/// `repositoryId` that is present but not a string or null, or that names a different
/// repository than `read`'s uri, is invalid params.
pub fn take_repository_id(
    name: &str,
    mut arguments: Option<Map<String, Value>>,
) -> Result<Routed, McpApplicationError> {
    let explicit = match arguments.as_mut().and_then(|a| a.remove("repositoryId")) {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) => Some(id),
        Some(_) => {
            return Err(McpApplicationError::invalid_params(
                "repositoryId must be a string",
            ))
        }
    };
    let from_uri = (name == tools::TOOL_READ)
        .then(|| arguments.as_ref()?.get("uri")?.as_str())
        .flatten()
        .and_then(crate::uri::repository_segment)
        .map(ToString::to_string);
    match (explicit, from_uri) {
        (Some(a), Some(b)) if !a.eq_ignore_ascii_case(&b) => {
            Err(McpApplicationError::invalid_params(format!(
                "repositoryId '{a}' conflicts with the uri's repository '{b}'"
            )))
        }
        (a, b) => Ok((a.or(b), arguments)),
    }
}

/// Add the optional `repositoryId` parameter to every tool of a `tools/list` result.
pub fn advertise_repository_id(list: &mut Value) {
    for tool in list["tools"].as_array_mut().into_iter().flatten() {
        let schema = &mut tool["inputSchema"];
        if !schema["properties"].is_object() {
            schema["properties"] = json!({});
        }
        schema["properties"]["repositoryId"] = json!({
            "type": "string",
            "description": "The repository this call is for (default: the server's default repository)."
        });
    }
}

/// A write the target repository refuses on I/O (permissions, a lock) is reported as
/// `write-refused` ([R17]).
pub fn mark_write_refused(result: &mut Value) {
    if result["structuredContent"]["code"] == "io" {
        result["structuredContent"]["code"] = json!("write-refused");
    }
}
