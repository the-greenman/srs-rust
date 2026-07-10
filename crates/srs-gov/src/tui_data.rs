use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::governance::by_root_type;
use crate::srs::run_srs;
use crate::tui_state::{AppState, ColumnItem, DetailRow, RecordItem, SectionItem};

pub fn load_app_state(repo: &str) -> Result<AppState> {
    let navigation = run_srs(&["repo", "navigation"], repo, false, false)
        .context("load repository navigation")?;

    let sections = sections_from_navigation(&navigation);
    let repo_title = navigation["navigation"]["identity"]["displayLabel"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Governance")
        .to_string();

    let mut state = AppState::new(repo_title, sections);
    refresh_records(repo, &mut state)?;
    Ok(state)
}

pub fn refresh_records(repo: &str, state: &mut AppState) -> Result<()> {
    let view = match state.selected_section() {
        Some(section) => load_section_view(
            repo,
            section,
            &state.search_query,
            state.show_all,
            state.newest_first,
        )?,
        None => SectionViewData::default(),
    };
    let count = view.records.len();
    state.set_view_context(view.document_view_id, view.columns, view.diagnostics);
    state.set_records(view.records);
    state.status = format!("{count} records");
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct SectionViewData {
    document_view_id: Option<String>,
    columns: Vec<ColumnItem>,
    diagnostics: Vec<String>,
    records: Vec<RecordItem>,
}

fn sections_from_navigation(payload: &Value) -> Vec<SectionItem> {
    let sections = payload["navigation"]["sections"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    sections
        .iter()
        .filter_map(|section| {
            let type_ns = section["typeNamespace"].as_str().unwrap_or("");
            let type_name = section["typeName"].as_str().unwrap_or("");
            let def = by_root_type(type_ns, type_name)?;
            let container_id = section["sectionContainerId"].as_str().map(String::from);
            // Use the canonical label from the type registry rather than the navigation response's displayLabel.
            Some(SectionItem {
                key: def.key.to_string(),
                label: def.label.to_string(),
                container_id,
            })
        })
        .collect()
}

fn load_section_view(
    repo: &str,
    section: &SectionItem,
    search_query: &str,
    show_all: bool,
    newest_first: bool,
) -> Result<SectionViewData> {
    let Some(container_id) = section.container_id.as_deref() else {
        return Ok(SectionViewData::default());
    };

    let payload = run_srs(
        &["container", "resolve-view", container_id],
        repo,
        false,
        false,
    )?;
    let view = &payload["containerView"];
    let root_id = view["root"]["instanceId"].as_str().unwrap_or("");
    let document_view_id = view["documentViewId"].as_str().map(String::from);
    let columns = column_items(view);
    let diagnostics = view["diagnostics"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let excludes = if show_all {
        Vec::new()
    } else {
        view["excludeLifecycleStates"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let search = (!search_query.is_empty()).then_some(search_query);
    let exclude_refs: Vec<&str> = excludes.iter().map(String::as_str).collect();
    let allowed =
        crate::find_query::resolve_hit_set(repo, container_id, &exclude_refs, search, &[])?;
    let mut schemas = HashMap::new();

    let mut records: Vec<RecordItem> = view["members"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|member| member["instanceId"].as_str() != Some(root_id))
        .filter(|member| match (&allowed, member["instanceId"].as_str()) {
            (Some(ids), Some(id)) => ids.contains(id),
            (Some(_), None) => false,
            (None, _) => true,
        })
        .map(|member| record_item(repo, member, &mut schemas))
        .collect::<Result<Vec<_>>>()?;

    records.sort_by(|left, right| {
        let ordering = left.created_at.cmp(&right.created_at);
        if newest_first {
            ordering.reverse()
        } else {
            ordering
        }
    });

    Ok(SectionViewData {
        document_view_id,
        columns,
        diagnostics,
        records,
    })
}

fn column_items(view: &Value) -> Vec<ColumnItem> {
    view["columns"]
        .as_array()
        .map(|columns| {
            columns
                .iter()
                .map(|column| ColumnItem {
                    field_id: column["fieldId"].as_str().unwrap_or("").to_string(),
                    field_name: column["fieldName"].as_str().unwrap_or("").to_string(),
                    display_label: column["displayLabel"].as_str().unwrap_or("").to_string(),
                    order: column["order"].as_i64().unwrap_or(99),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn record_item(
    repo: &str,
    member: &Value,
    schemas: &mut HashMap<(String, u64), Value>,
) -> Result<RecordItem> {
    let record = &member["record"];
    let type_id = record["typeId"].as_str().unwrap_or("").to_string();
    let type_version = record["typeVersion"].as_u64().unwrap_or(1);
    let schema = load_type_schema(repo, &type_id, type_version, schemas)?;
    Ok(RecordItem {
        instance_id: member["instanceId"].as_str().unwrap_or("").to_string(),
        label: member["displayLabel"]
            .as_str()
            .unwrap_or("(untitled)")
            .to_string(),
        lifecycle_state: record["lifecycleState"].as_str().map(String::from),
        tags: record["tags"]
            .as_array()
            .map(|tags| {
                tags.iter()
                    .filter_map(|tag| tag.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        created_at: record["createdAt"].as_str().map(String::from),
        type_id,
        type_version,
        detail_rows: detail_rows(
            &schema,
            record["fieldValues"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        ),
        record: record.clone(),
    })
}

fn load_type_schema(
    repo: &str,
    type_id: &str,
    type_version: u64,
    schemas: &mut HashMap<(String, u64), Value>,
) -> Result<Value> {
    let key = (type_id.to_string(), type_version);
    if let Some(schema) = schemas.get(&key) {
        return Ok(schema.clone());
    }

    let version = type_version.to_string();
    let payload = run_srs(
        &["type", "schema", type_id, "--type-version", &version],
        repo,
        false,
        false,
    )?;
    let schema = payload["schema"].clone();
    schemas.insert(key, schema.clone());
    Ok(schema)
}

/// Shape a type schema + a record's field values into ordered, labeled display rows.
///
/// `schema` is the full type schema (`payload.schema`); `field_values` is the record's
/// `fieldValues` array. Shared by the TUI detail pane and `render::record_detail` (CLI text
/// output) so the field ordering/labeling logic exists in exactly one place.
pub(crate) fn detail_rows(schema: &Value, field_values: &[Value]) -> Vec<DetailRow> {
    let values_by_field_id: HashMap<&str, &Value> = field_values
        .iter()
        .filter_map(|field_value| {
            Some((field_value["fieldId"].as_str()?, field_value.get("value")?))
        })
        .collect();
    let required_names: HashSet<&str> = schema["required"]
        .as_array()
        .map(|values| values.iter().filter_map(|value| value.as_str()).collect())
        .unwrap_or_default();
    let Some(properties) = schema["properties"].as_object() else {
        return Vec::new();
    };

    let mut rows: Vec<DetailRow> = properties
        .iter()
        .filter_map(|(name, property)| {
            let field_id = property["x-srs-field-id"].as_str()?;
            let value = values_by_field_id
                .get(field_id)
                .map(|value| display_value(value));
            Some(DetailRow {
                label: property["title"].as_str().unwrap_or(name).to_string(),
                value,
                required: required_names.contains(name.as_str()),
                order: property["x-srs-order"].as_i64().unwrap_or(99),
            })
        })
        .collect();
    rows.sort_by_key(|row| row.order);
    rows
}

fn display_value(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_from_navigation_maps_type_chain_to_governance_sections() {
        let payload = serde_json::json!({
            "navigation": {
                "identity": { "displayLabel": "Example" },
                "sections": [
                    {
                        "typeNamespace": "governance",
                        "typeName": "decision_log",
                        "sectionContainerId": "c-1"
                    },
                    {
                        "typeNamespace": "unknown",
                        "typeName": "something_else",
                        "sectionContainerId": "c-2"
                    }
                ]
            }
        });

        let sections = sections_from_navigation(&payload);

        // Only governance-typed sections are included; unknown types are filtered out.
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].key, "decision_log");
        assert_eq!(sections[0].container_id.as_deref(), Some("c-1"));
    }

    #[test]
    fn sections_from_navigation_no_container_id_produces_none() {
        let payload = serde_json::json!({
            "navigation": {
                "identity": { "displayLabel": "Example" },
                "sections": [
                    {
                        "typeNamespace": "governance",
                        "typeName": "decision_log"
                        // sectionContainerId absent — section not yet provisioned
                    }
                ]
            }
        });

        let sections = sections_from_navigation(&payload);

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].key, "decision_log");
        assert_eq!(sections[0].container_id, None);
    }

    #[test]
    fn sections_from_navigation_empty_sections_returns_empty() {
        let payload = serde_json::json!({
            "navigation": {
                "identity": { "displayLabel": "Example" },
                "sections": []
            }
        });

        let sections = sections_from_navigation(&payload);

        assert!(sections.is_empty());
    }

    #[test]
    fn record_item_reads_presentation_fields_without_type_specific_rules() {
        let member = serde_json::json!({
            "instanceId": "r-1",
            "displayLabel": "Adopt policy",
            "record": {
                "typeId": "type-decision",
                "typeVersion": 1,
                "lifecycleState": "ratified",
                "tags": ["tooling"],
                "createdAt": "2026-01-02T00:00:00Z",
                "fieldValues": [
                    { "fieldId": "title-field", "value": "Adopt policy" }
                ]
            }
        });

        let schema = serde_json::json!({
            "properties": {
                "title": {
                    "title": "Title",
                    "x-srs-field-id": "title-field",
                    "x-srs-order": 1
                }
            }
        });
        let mut schemas = HashMap::from([(("type-decision".to_string(), 1), schema)]);
        let item = record_item(".", &member, &mut schemas).expect("record item");

        assert_eq!(item.label, "Adopt policy");
        assert_eq!(item.lifecycle_state.as_deref(), Some("ratified"));
        assert_eq!(item.tags, vec!["tooling"]);
    }

    #[test]
    fn detail_rows_order_and_match_values_by_field_id() {
        let schema = serde_json::json!({
            "required": ["statement"],
            "properties": {
                "title": {
                    "title": "Title",
                    "x-srs-field-id": "field-title",
                    "x-srs-order": 2
                },
                "statement": {
                    "title": "Decision Statement",
                    "x-srs-field-id": "field-statement",
                    "x-srs-order": 1
                },
                "missing": {
                    "title": "Missing",
                    "x-srs-field-id": "field-missing",
                    "x-srs-order": 3
                }
            }
        });
        let field_values = vec![
            serde_json::json!({ "fieldId": "field-title", "value": "Adopt policy" }),
            serde_json::json!({ "fieldId": "field-statement", "value": "Use schema detail" }),
        ];

        let rows = detail_rows(&schema, &field_values);

        assert_eq!(rows[0].label, "Decision Statement");
        assert_eq!(rows[0].value.as_deref(), Some("Use schema detail"));
        assert!(rows[0].required);
        assert_eq!(rows[1].label, "Title");
        assert_eq!(rows[1].value.as_deref(), Some("Adopt policy"));
        assert_eq!(rows[2].label, "Missing");
        assert_eq!(rows[2].value, None);
    }
}
