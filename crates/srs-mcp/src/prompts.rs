//! MCP prompts surface — one prompt per package blueprint (ADR-037 follow-up).
//!
//! Each handler is a thin wrapper: typed input → one `srs-repository` service
//! call + pure formatting → MCP result type (ADR-010, ADR-037). `render_brief_markdown`
//! is a pure formatting function (no I/O, no store access), not a service call.
//!
//! Prompt `name` = blueprint UUID so that `get_prompt` can pass it directly to
//! `blueprint_brief` as the `blueprint_id`, maintaining one service call per
//! handler (ADR-010). Human-readable `{namespace}/{name} v{version}` appears
//! in the `description` field that MCP clients show in pickers.

use rmcp::model::{GetPromptResult, JsonObject, ListPromptsResult, Prompt, PromptMessage, Role};
use rmcp::ErrorData as McpError;
use srs_repository::blueprint_brief_service::{
    blueprint_brief, render_brief_markdown, BlueprintBriefInput,
};
use srs_repository::blueprint_service::list_blueprints_summary;
use srs_repository::error::RepositoryError;

use crate::server::SrsMcpServer;

fn service_err(e: RepositoryError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn prompt_description(namespace: &str, name: &str, version: u32, description: &str) -> String {
    format!("{namespace}/{name} v{version}: {description}")
}

pub(crate) fn list_prompts(server: &SrsMcpServer) -> Result<ListPromptsResult, McpError> {
    let store = server.open_store();
    let result = list_blueprints_summary(&store).map_err(service_err)?;
    // Non-fatal diagnostics (missing blueprint files, duplicate IDs) are
    // intentionally not surfaced here — list_prompts has no warnings channel.
    let prompts = result
        .summaries
        .into_iter()
        .map(|s| {
            Prompt::new(
                s.id,
                Some(prompt_description(&s.namespace, &s.name, s.version, &s.description)),
                None,
            )
        })
        .collect();
    Ok(ListPromptsResult::with_all_items(prompts))
}

pub(crate) fn get_prompt(
    server: &SrsMcpServer,
    name: &str,
    arguments: Option<&JsonObject>,
) -> Result<GetPromptResult, McpError> {
    if arguments.map(|a| !a.is_empty()).unwrap_or(false) {
        return Err(McpError::invalid_params(
            format!("prompt '{name}' takes no arguments"),
            None,
        ));
    }
    let store = server.open_store();
    let result =
        blueprint_brief(&store, BlueprintBriefInput { blueprint_id: name.to_string() }).map_err(
            |e| match e {
                RepositoryError::BlueprintNotFound { .. } => {
                    McpError::invalid_params(format!("prompt not found: {name}"), None)
                }
                other => service_err(other),
            },
        )?;
    let rendered = render_brief_markdown(&result);
    // Role::User: blueprint briefs are guidance the agent consumes as
    // user-context — the briefing comes from the requester's side, not as
    // pre-authored assistant output.
    Ok(GetPromptResult::new(vec![PromptMessage::new_text(
        Role::User,
        rendered,
    )]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ErrorCode;
    use srs_core::types::blueprint::{Blueprint, TypeRef};
    use srs_repository::blueprint_service::create_blueprint;
    use srs_repository::repository_lifecycle::{
        InitializeRepositoryInput, PrimaryPackageMetadata, RepositoryMetadata, create_repository,
    };
    use srs_repository::store::FileStore;
    use tempfile::TempDir;

    fn make_test_server() -> (TempDir, SrsMcpServer) {
        let dir = tempfile::tempdir().unwrap();
        let store = FileStore::new(dir.path());
        let input = InitializeRepositoryInput {
            repository: RepositoryMetadata {
                repository_id: "test-repo-id".to_string(),
                namespace: "test.ns".to_string(),
                srs_version: "2.0".to_string(),
                title: Some("Test Repo".to_string()),
                description: None,
            },
            primary_package: PrimaryPackageMetadata {
                id: "test-pkg".to_string(),
                namespace: "test.ns".to_string(),
                name: "test".to_string(),
                version: "1.0.0".to_string(),
            },
        };
        create_repository(&store, &input).unwrap();
        let server = SrsMcpServer::new(dir.path().to_path_buf()).unwrap();
        (dir, server)
    }

    fn make_blueprint(name: &str, namespace: &str) -> Blueprint {
        Blueprint {
            id: String::new(),
            namespace: namespace.to_string(),
            name: name.to_string(),
            version: 1,
            description: format!("{name} description"),
            root_types: vec![TypeRef {
                type_id: "placeholder-type-id".to_string(),
                type_version: None,
            }],
            structure: vec![],
            required_types: vec![],
            ai_guidance: None,
            tags: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            lineage: None,
            provenance: None,
        }
    }

    #[test]
    fn list_prompts_returns_one_per_blueprint() {
        let (_dir, server) = make_test_server();
        let r1 =
            create_blueprint(&server.open_store(), make_blueprint("alpha", "test.ns"), None)
                .unwrap();
        let r2 =
            create_blueprint(&server.open_store(), make_blueprint("beta", "test.ns"), None)
                .unwrap();

        let result = list_prompts(&server).unwrap();
        assert_eq!(result.prompts.len(), 2);

        let names: Vec<&str> = result.prompts.iter().map(|p| p.name.as_str()).collect();
        assert!(
            names.contains(&r1.blueprint.id.as_str()),
            "prompt list should contain alpha's id"
        );
        assert!(
            names.contains(&r2.blueprint.id.as_str()),
            "prompt list should contain beta's id"
        );

        let alpha = result
            .prompts
            .iter()
            .find(|p| p.name == r1.blueprint.id)
            .unwrap();
        let desc = alpha.description.as_deref().unwrap_or_default();
        assert!(
            desc.contains("test.ns/alpha"),
            "description should contain namespace/name: {desc}"
        );
    }

    #[test]
    fn get_prompt_returns_rendered_markdown() {
        let (_dir, server) = make_test_server();
        let created =
            create_blueprint(&server.open_store(), make_blueprint("my-bp", "test.ns"), None)
                .unwrap();
        let bp_id = created.blueprint.id.clone();

        let result = get_prompt(&server, &bp_id, None).unwrap();
        assert_eq!(result.messages.len(), 1);

        let text = match &result.messages[0].content {
            rmcp::model::ContentBlock::Text(t) => t.text.clone(),
            other => panic!("expected Text content, got {other:?}"),
        };
        assert!(
            text.contains("my-bp"),
            "rendered markdown should contain blueprint name: {text}"
        );
    }

    #[test]
    fn get_prompt_unknown_name_returns_invalid_params() {
        let (_dir, server) = make_test_server();
        let err = get_prompt(&server, "no-such-id", None).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::INVALID_PARAMS,
            "expected invalid_params, got: {err:?}"
        );
        assert!(
            err.message.contains("prompt not found"),
            "expected 'prompt not found' in message: {}",
            err.message
        );
    }

    #[test]
    fn get_prompt_rejects_unexpected_arguments() {
        let (_dir, server) = make_test_server();
        let mut args = serde_json::Map::new();
        args.insert(
            "foo".to_string(),
            serde_json::Value::String("bar".to_string()),
        );
        let err = get_prompt(&server, "any-id", Some(&args)).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::INVALID_PARAMS,
            "expected invalid_params for unexpected arguments: {err:?}"
        );
    }
}
