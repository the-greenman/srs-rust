use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::governance::{match_container, GOVERNANCE_CONTAINERS};
use crate::srs::run_srs;
use crate::tui_state::{AppState, ColumnItem, DetailRow, RecordItem, SectionItem};

pub fn load_app_state(repo: &str) -> Result<AppState> {
    let navigation = run_srs(&["repo", "navigation"], repo, false, false)
        .context("load repository navigation")?;

    let mut sections = sections_from_navigation(&navigation);
    let mut repo_title = navigation["navigation"]["identity"]["displayLabel"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("Governance")
        .to_string();

    if sections.is_empty() {
        let fallback = sections_from_container_list(repo)?;
        if !fallback.is_empty() {
            sections = fallback;
            repo_title = "Governance".to_string();
        }
    }

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
        .map(|section| {
            let label = section["displayLabel"].as_str().unwrap_or("Untitled");
            SectionItem {
                key: governance_key_for_label(label),
                label: label.to_string(),
                container_id: section["containerId"].as_str().map(String::from),
            }
        })
        .collect()
}

fn sections_from_container_list(repo: &str) -> Result<Vec<SectionItem>> {
    let payload = run_srs(&["container", "list"], repo, false, false)?;
    let containers = payload["containers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut used_keys = HashSet::new();
    let mut sections = Vec::new();

    for container in containers {
        let title = container["title"].as_str().unwrap_or("");
        let container_type = container["containerType"].as_str();
        let container_id = container["containerId"].as_str().map(String::from);
        if let Some(def) = match_container(container_type, title, &mut used_keys) {
            sections.push(SectionItem {
                key: def.key.to_string(),
                label: def.label.to_string(),
                container_id,
            });
        }
    }

    Ok(sections)
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
    let allowed = allowed_hits(repo, container_id, search_query, &excludes)?;
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

fn allowed_hits(
    repo: &str,
    container_id: &str,
    search_query: &str,
    excludes: &[String],
) -> Result<Option<HashSet<String>>> {
    let need_find = !search_query.is_empty() || !excludes.is_empty();
    if !need_find {
        return Ok(None);
    }

    let mut args = vec![
        "--container".to_string(),
        container_id.to_string(),
        "find".to_string(),
    ];
    for exclude in excludes {
        args.push("--exclude-lifecycle-state".to_string());
        args.push(exclude.clone());
    }
    if !search_query.is_empty() {
        args.push("--text".to_string());
        args.push(search_query.to_string());
    }

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let payload = run_srs(&arg_refs, repo, false, false)?;
    let hits = payload["result"]["hits"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|hit| hit["instanceId"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(hits))
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
        detail_rows: detail_rows(&schema, record),
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

pub(crate) fn detail_rows(schema: &Value, record: &Value) -> Vec<DetailRow> {
    let values_by_field_id: HashMap<&str, &Value> = record["fieldValues"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|field_value| {
                    Some((field_value["fieldId"].as_str()?, field_value.get("value")?))
                })
                .collect()
        })
        .unwrap_or_default();
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

fn governance_key_for_label(label: &str) -> String {
    GOVERNANCE_CONTAINERS
        .iter()
        .find(|def| label.eq_ignore_ascii_case(def.label))
        .map(|def| def.key.to_string())
        .unwrap_or_else(|| {
            label
                .to_ascii_lowercase()
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>()
                .trim_matches('_')
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_from_navigation_maps_labels_to_governance_keys() {
        let payload = serde_json::json!({
            "navigation": {
                "identity": { "displayLabel": "Example" },
                "sections": [
                    { "displayLabel": "Decision Log", "containerId": "c-1" },
                    { "displayLabel": "Roles", "containerId": "c-2" }
                ]
            }
        });

        let sections = sections_from_navigation(&payload);

        assert_eq!(sections[0].key, "decision_log");
        assert_eq!(sections[0].container_id.as_deref(), Some("c-1"));
        assert_eq!(sections[1].key, "roles");
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
        let record = serde_json::json!({
            "fieldValues": [
                { "fieldId": "field-title", "value": "Adopt policy" },
                { "fieldId": "field-statement", "value": "Use schema detail" }
            ]
        });

        let rows = detail_rows(&schema, &record);

        assert_eq!(rows[0].label, "Decision Statement");
        assert_eq!(rows[0].value.as_deref(), Some("Use schema detail"));
        assert!(rows[0].required);
        assert_eq!(rows[1].label, "Title");
        assert_eq!(rows[1].value.as_deref(), Some("Adopt policy"));
        assert_eq!(rows[2].label, "Missing");
        assert_eq!(rows[2].value, None);
    }
}
