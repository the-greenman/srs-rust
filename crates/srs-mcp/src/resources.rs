//! MCP resource handlers — the read surface.
//!
//! Each arm is exactly one `srs-repository` service call whose typed result is
//! serialized as-is (ADR-010/ADR-037). Rendering opinions stay out: JSON
//! resources carry the service struct verbatim; document views carry the
//! service-rendered markdown.

use rmcp::model::{
    ListResourceTemplatesResult, ListResourcesResult, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate,
};
use rmcp::ErrorData as McpError;
use serde::Serialize;
use srs_repository::agent_index_service::build_agent_index;
use srs_repository::analysis::build_repo_map;
use srs_repository::container_service::{list_containers, ContainerListFilter};
use srs_repository::container_view_service::{resolve_container_view, ResolveContainerViewInput};
use srs_repository::error::RepositoryError;
use srs_repository::package_service::{list_types_filtered, TypeListFilter};
use srs_repository::protocol_service::{
    get_protocol_by_id, list_protocol_stages, list_protocols, GetProtocolResult,
};
use srs_repository::record_store::get_record_by_id;
use srs_repository::render_service::{render_composition, RenderCompositionOptions};
use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::tree_service::{build_tree, TreeOptions};
use srs_repository::type_schema_service::{type_schema, TypeSchemaInput};
use srs_repository::view_service::{list_compositions_summary, CompositionListFilter};

use crate::uri::{self, SrsUri};
use srs_repository::store::RepositoryStore;

const MIME_JSON: &str = "application/json";
const MIME_MARKDOWN: &str = "text/markdown";

fn service_err(e: RepositoryError) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

fn json_text<T: Serialize>(value: &T, uri: &str) -> Result<ResourceContents, McpError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    Ok(ResourceContents::text(text, uri).with_mime_type(MIME_JSON))
}

pub(crate) fn list_resources(
    store: &dyn RepositoryStore,
    repo_id: &str,
) -> Result<ListResourcesResult, McpError> {
    let mut resources = vec![
        Resource::new(uri::format(&SrsUri::Map, repo_id), "map")
            .with_title("Repository map")
            .with_description(
                "Counts, package info, relation summary and description for this repository \
                 — read this first to orient.",
            )
            .with_mime_type(MIME_JSON),
        Resource::new(uri::format(&SrsUri::Navigation, repo_id), "navigation")
            .with_title("Repository navigation")
            .with_description(
                "The repository's identity record and ordered navigation sections \
                 (root container structure).",
            )
            .with_mime_type(MIME_JSON),
        Resource::new(uri::format(&SrsUri::Tree, repo_id), "tree")
            .with_title("Repository tree")
            .with_description(
                "Recursive `contains` tree from every auto-detected root (records not \
                 targeted by a contains edge), with depth and cycle pruning — the same \
                 result as `srs tree`. Subtrees: srs://<repositoryId>/tree/{instanceId}.",
            )
            .with_mime_type(MIME_JSON),
        Resource::new(uri::format(&SrsUri::AgentIndex, repo_id), "agent-index")
            .with_title("Agent index")
            .with_description(
                "AI orientation index: repository identity, counts, installed types, \
                 top-level sections and suggested entry points — same as `srs repo agent-index`.",
            )
            .with_mime_type(MIME_JSON),
    ];

    for c in list_containers(store, &ContainerListFilter::default()).map_err(service_err)? {
        resources.push(
            Resource::new(
                uri::format(&SrsUri::Container(c.container_id.clone()), repo_id),
                c.title.clone(),
            )
            .with_title(c.title)
            .with_description("Container: authored columns and ordered members (resolve-view).")
            .with_mime_type(MIME_JSON),
        );
    }

    for v in
        list_compositions_summary(store, &CompositionListFilter::default()).map_err(service_err)?
    {
        resources.push(
            Resource::new(
                uri::format(&SrsUri::Composition(v.id.clone()), repo_id),
                // Composition has no title field — the namespace-qualified name
                // is the identity (plan review AR-5).
                format!("{}/{}", v.namespace, v.name),
            )
            .with_description(v.description)
            .with_mime_type(MIME_MARKDOWN),
        );
    }

    resources.push(
        Resource::new(uri::format(&SrsUri::ProtocolList, repo_id), "protocol")
            .with_title("Installed protocols")
            .with_description(
                "Every installed Protocol definition: id, namespace/name@version, targetType, \
                 stageCount. Read one via srs://<repositoryId>/protocol/{protocolId}.",
            )
            .with_mime_type(MIME_JSON),
    );

    for p in list_protocols(store).map_err(service_err)? {
        resources.push(
            Resource::new(
                uri::format(&SrsUri::Protocol(p.protocol_id), repo_id),
                format!("{}/{}", p.protocol_namespace, p.protocol_name),
            )
            .with_description("Protocol definition with its stages in dependsOn order.")
            .with_mime_type(MIME_JSON),
        );
    }

    for t in list_types_filtered(store, TypeListFilter::default()).map_err(service_err)? {
        resources.push(
            Resource::new(
                uri::format(&SrsUri::Type(t.id.clone()), repo_id),
                format!("{}/{}", t.namespace, t.name),
            )
            .with_description(t.description.unwrap_or_else(|| {
                "Type schema: fieldAssignments + aiGuidance for authoring".to_string()
            }))
            .with_mime_type(MIME_JSON),
        );
    }

    Ok(ListResourcesResult::with_all_items(resources))
}

pub(crate) fn list_resource_templates(repo_id: &str) -> ListResourceTemplatesResult {
    let template = ResourceTemplate::new(uri::record_template(repo_id), "record")
        .with_title("Record by instance id")
        .with_description(
            "Read a single record (any tier) as typed JSON by its instanceId. \
             Discover instanceIds via the find tool or container resources.",
        )
        .with_mime_type(MIME_JSON);
    let type_tmpl = ResourceTemplate::new(uri::type_template(repo_id), "type")
        .with_title("Type authoring schema by type id")
        .with_description(
            "Authoring schema for a type: fieldIds, required flags, and aiGuidance \
             — read before record_create on an unfamiliar type.",
        )
        .with_mime_type(MIME_JSON);
    let protocol_tmpl = ResourceTemplate::new(uri::protocol_template(repo_id), "protocol")
        .with_title("Protocol definition by protocol id")
        .with_description(
            "A Protocol definition (same shape as `srs protocol get`) plus its stages \
                 sorted by order — the dependsOn walk an agent follows.",
        )
        .with_mime_type(MIME_JSON);
    let tree_tmpl = ResourceTemplate::new(uri::tree_template(repo_id), "tree")
        .with_title("Subtree by root instance id")
        .with_description(
            "Recursive `contains` tree rooted at one instance — descend from any \
             navigation section or container member by its instanceId.",
        )
        .with_mime_type(MIME_JSON);
    ListResourceTemplatesResult::with_all_items(vec![template, type_tmpl, protocol_tmpl, tree_tmpl])
}

pub(crate) fn read_resource(
    store: &dyn RepositoryStore,
    repository_id: &str,
    raw_uri: &str,
) -> Result<ReadResourceResult, McpError> {
    let parsed = uri::parse(raw_uri, repository_id)
        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

    let contents = match parsed {
        SrsUri::Map => {
            let map = build_repo_map(store).map_err(service_err)?;
            json_text(&map, raw_uri)?
        }
        SrsUri::Navigation => {
            let nav = repository_navigation(store).map_err(service_err)?;
            json_text(&nav, raw_uri)?
        }
        SrsUri::Tree => {
            let tree = build_tree(store, TreeOptions::default()).map_err(service_err)?;
            json_text(&tree, raw_uri)?
        }
        SrsUri::TreeFrom(id) => {
            let tree = build_tree(
                store,
                TreeOptions {
                    root_ids: Some(vec![id]),
                    ..TreeOptions::default()
                },
            )
            .map_err(service_err)?;
            json_text(&tree, raw_uri)?
        }
        SrsUri::AgentIndex => {
            let index = build_agent_index(store).map_err(service_err)?;
            json_text(&index, raw_uri)?
        }
        SrsUri::Record(id) => match get_record_by_id(store, &id).map_err(service_err)? {
            // `Ok(None)` is not a service error, so there is no service message
            // to reuse — the not-found text is adapter-authored (plan review AR-6).
            None => {
                return Err(McpError::resource_not_found(
                    format!("resource not found: {raw_uri}"),
                    None,
                ))
            }
            Some(record) => json_text(&record, raw_uri)?,
        },
        SrsUri::Container(id) => {
            let view = resolve_container_view(
                store,
                ResolveContainerViewInput {
                    container_id: id,
                    view_id: None,
                },
            )
            .map_err(service_err)?;
            json_text(&view, raw_uri)?
        }
        SrsUri::Composition(id) => {
            let result = render_composition(RenderCompositionOptions {
                store,
                view_id: &id,
                format: Some("markdown"),
                theme_variant: None,
                container_id: None,
                instance_id_filter: None,
            })
            .map_err(service_err)?;
            ResourceContents::text(result.rendered, raw_uri).with_mime_type(MIME_MARKDOWN)
        }
        // Same pattern as the Container/View arms: `type_schema` returns
        // Err(RepositoryError::TypeNotFound) for unknown ids — no Ok(None) branch.
        SrsUri::Type(id) => {
            let result = type_schema(
                store,
                TypeSchemaInput {
                    type_id: id,
                    type_version: None,
                },
            )
            .map_err(service_err)?;
            json_text(&result, raw_uri)?
        }
        SrsUri::ProtocolList => {
            let protocols = list_protocols(store).map_err(service_err)?;
            json_text(&serde_json::json!({ "protocols": protocols }), raw_uri)?
        }
        // Mirrors `srs protocol get` + `srs protocol stages` in one read: the
        // stored definition verbatim, plus the stages sorted by `order`.
        SrsUri::Protocol(id) => match get_protocol_by_id(store, &id).map_err(service_err)? {
            GetProtocolResult::NotFound => {
                return Err(McpError::resource_not_found(
                    format!("resource not found: {raw_uri}"),
                    None,
                ))
            }
            GetProtocolResult::Found(protocol) => {
                let stages = list_protocol_stages(store, &id).map_err(service_err)?;
                json_text(
                    &serde_json::json!({ "protocol": protocol, "stages": stages }),
                    raw_uri,
                )?
            }
        },
    };

    Ok(ReadResourceResult::new(vec![contents]))
}
