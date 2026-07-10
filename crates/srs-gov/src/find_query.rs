use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;

/// Build the `srs --container <id> find ...` argument list shared by the CLI list
/// command and the TUI's section-view loader (members ∩ find-hits composition).
pub(crate) fn build_find_args(
    container_id: &str,
    excludes: &[&str],
    search: Option<&str>,
    tags: &[String],
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--container".into(),
        container_id.to_string(),
        "find".into(),
    ];
    for state in excludes {
        args.push("--exclude-lifecycle-state".into());
        args.push((*state).to_string());
    }
    if let Some(text) = search {
        args.push("--text".into());
        args.push(text.to_string());
    }
    for tag in tags {
        args.push("--tag".into());
        args.push(tag.clone());
    }
    args
}

/// Run a scoped `srs find` query and return the matching instance IDs as a set,
/// or `None` when no filter is active (caller should show all members verbatim).
pub(crate) fn resolve_hit_set(
    repo: &str,
    container_id: &str,
    excludes: &[&str],
    search: Option<&str>,
    tags: &[String],
) -> Result<Option<HashSet<String>>> {
    let need_find = !excludes.is_empty() || search.is_some() || !tags.is_empty();
    if !need_find {
        return Ok(None);
    }
    let args = build_find_args(container_id, excludes, search, tags);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let payload = crate::srs::run_srs(&arg_refs, repo, false, false)?;
    Ok(Some(parse_hit_ids(&payload).into_iter().collect()))
}

/// Extract the matched instance ids from a `srs find` result payload.
pub(crate) fn parse_hit_ids(payload: &Value) -> Vec<String> {
    payload["result"]["hits"]
        .as_array()
        .map(|hits| {
            hits.iter()
                .filter_map(|hit| hit["instanceId"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_find_args_orders_global_container_before_subcommand() {
        let args = build_find_args(
            "container-123",
            &["superseded", "closed"],
            Some("budget"),
            &["finance".to_string(), "q1".to_string()],
        );
        assert_eq!(
            args,
            vec![
                "--container",
                "container-123",
                "find",
                "--exclude-lifecycle-state",
                "superseded",
                "--exclude-lifecycle-state",
                "closed",
                "--text",
                "budget",
                "--tag",
                "finance",
                "--tag",
                "q1",
            ]
        );
    }

    #[test]
    fn build_find_args_minimal_is_just_scoped_find() {
        assert_eq!(
            build_find_args("c-1", &[], None, &[]),
            vec!["--container", "c-1", "find"]
        );
    }

    #[test]
    fn parse_hit_ids_extracts_instance_ids_in_order() {
        let payload = serde_json::json!({
            "result": {
                "hits": [
                    { "instanceId": "r-1" },
                    { "instanceId": "r-2" },
                ]
            }
        });
        assert_eq!(parse_hit_ids(&payload), vec!["r-1", "r-2"]);
    }

    #[test]
    fn parse_hit_ids_defaults_to_empty_when_absent() {
        assert_eq!(parse_hit_ids(&serde_json::json!({})), Vec::<String>::new());
    }

    #[test]
    fn resolve_hit_set_returns_none_when_no_filter_active() {
        // No excludes, no search, no tags → no `srs` process spawned; returns Ok(None).
        let result = resolve_hit_set(".", "c-1", &[], None, &[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }
}
