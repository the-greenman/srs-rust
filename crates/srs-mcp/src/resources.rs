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
use srs_repository::analysis::build_repo_map;
use srs_repository::container_service::{list_containers, ContainerListFilter};
use srs_repository::container_view_service::{resolve_container_view, ResolveContainerViewInput};
use srs_repository::error::RepositoryError;
use srs_repository::package_service::{list_types_filtered, TypeListFilter};
use srs_repository::record_store::get_record_by_id;
use srs_repository::render_service::{render_document_view, RenderDocumentViewOptions};
use srs_repository::repository_navigation_service::repository_navigation;
use srs_repository::type_schema_service::{type_schema, TypeSchemaInput};
use srs_repository::view_service::{list_document_views_summary, DocumentViewListFilter};

use crate::server::SrsMcpServer;
use crate::uri::{self, SrsUri};

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

pub(crate) fn list_resources(server: &SrsMcpServer) -> Result<ListResourcesResult, McpError> {
    let store = server.open_store();
    let repo_id = server.repository_id();

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
    ];

    for c in list_containers(&store, &ContainerListFilter::default()).map_err(service_err)? {
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

    for v in list_document_views_summary(&store, &DocumentViewListFilter::default())
        .map_err(service_err)?
    {
        resources.push(
            Resource::new(
                uri::format(&SrsUri::View(v.id.clone()), repo_id),
                // DocumentView has no title field — the namespace-qualified name
                // is the identity (plan review AR-5).
                format!("{}/{}", v.namespace, v.name),
            )
            .with_description(v.description)
            .with_mime_type(MIME_MARKDOWN),
        );
    }

    for t in list_types_filtered(&store, TypeListFilter::default()).map_err(service_err)? {
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

pub(crate) fn list_resource_templates(server: &SrsMcpServer) -> ListResourceTemplatesResult {
    let template = ResourceTemplate::new(uri::record_template(server.repository_id()), "record")
        .with_title("Record by instance id")
        .with_description(
            "Read a single record (any tier) as typed JSON by its instanceId. \
             Discover instanceIds via the find tool or container resources.",
        )
        .with_mime_type(MIME_JSON);
    let type_tmpl = ResourceTemplate::new(uri::type_template(server.repository_id()), "type")
        .with_title("Type authoring schema by type id")
        .with_description(
            "Authoring schema for a type: fieldIds, required flags, and aiGuidance \
             — read before record_create on an unfamiliar type.",
        )
        .with_mime_type(MIME_JSON);
    ListResourceTemplatesResult::with_all_items(vec![template, type_tmpl])
}

pub(crate) fn read_resource(
    server: &SrsMcpServer,
    raw_uri: &str,
) -> Result<ReadResourceResult, McpError> {
    let parsed = uri::parse(raw_uri, server.repository_id())
        .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
    let store = server.open_store();

    let contents = match parsed {
        SrsUri::Map => {
            let map = build_repo_map(&store).map_err(service_err)?;
            json_text(&map, raw_uri)?
        }
        SrsUri::Navigation => {
            let nav = repository_navigation(&store).map_err(service_err)?;
            json_text(&nav, raw_uri)?
        }
        SrsUri::Record(id) => match get_record_by_id(&store, &id).map_err(service_err)? {
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
                &store,
                ResolveContainerViewInput {
                    container_id: id,
                    view_id: None,
                },
            )
            .map_err(service_err)?;
            json_text(&view, raw_uri)?
        }
        SrsUri::View(id) => {
            let result = render_document_view(RenderDocumentViewOptions {
                store: &store,
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
                &store,
                TypeSchemaInput {
                    type_id: id,
                    type_version: None,
                },
            )
            .map_err(service_err)?;
            json_text(&result, raw_uri)?
        }
    };

    Ok(ReadResourceResult::new(vec![contents]))
}
