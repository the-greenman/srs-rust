//! The transport-independent `srs://` resource URI contract.

use srs_repository::resource_uri;
use std::fmt;

const SCHEME: &str = "srs://";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SrsUri {
    Map,
    Navigation,
    Record(String),
    Container(String),
    Composition(String),
    Type(String),
    ProtocolList,
    Protocol(String),
    /// `tree[?maxDepth=&relationType=&typeFilter=]` (#1229).
    Tree(TreeQuery),
    TreeFrom(String, TreeQuery),
    AgentIndex,
    /// `relation-types` (#1251): every installed RelationTypeDefinition.
    RelationTypes,
    /// `context/{instanceId}` or `context/{containerId}/{instanceId}` (#1134).
    Context {
        container_id: Option<String>,
        instance_id: String,
        /// `?excludeRelationCategories=composition,sequence` (#1188); wire spellings, parsed by the adapter.
        exclude_relation_categories: Vec<String>,
    },
}

/// The `build_tree` controls a tree URI may carry (#1229); `None` = the service default.
/// Values are taken verbatim (no percent-decoding): a `namespace/name` type filter and a
/// relation key contain no reserved characters. `relationType` is not checked against the
/// installed relation types, matching `build_tree`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeQuery {
    pub max_depth: Option<u32>,
    pub relation_type: Option<String>,
    pub type_filter: Option<String>,
}

fn parse_tree_query(query: &str, uri: &str) -> Result<TreeQuery, UriError> {
    let mut q = TreeQuery::default();
    let dup = |key: &str| UriError(format!("duplicate query key '{key}' in '{uri}'"));
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        match pair.split_once('=') {
            Some(("maxDepth", v)) => {
                let depth = v.parse().map_err(|_| {
                    UriError(format!(
                        "maxDepth must be a non-negative integer in '{uri}'"
                    ))
                })?;
                q.max_depth
                    .replace(depth)
                    .map_or(Ok(()), |_| Err(dup("maxDepth")))?;
            }
            Some(("relationType", v)) if !v.is_empty() => q
                .relation_type
                .replace(v.to_string())
                .map_or(Ok(()), |_| Err(dup("relationType")))?,
            Some(("typeFilter", v)) if !v.is_empty() => q
                .type_filter
                .replace(v.to_string())
                .map_or(Ok(()), |_| Err(dup("typeFilter")))?,
            _ => return Err(UriError(format!("unsupported query in '{uri}'"))),
        }
    }
    Ok(q)
}

fn tree_query_string(q: &TreeQuery) -> String {
    let mut parts = vec![];
    if let Some(d) = q.max_depth {
        parts.push(format!("maxDepth={d}"));
    }
    if let Some(r) = &q.relation_type {
        parts.push(format!("relationType={r}"));
    }
    if let Some(t) = &q.type_filter {
        parts.push(format!("typeFilter={t}"));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriError(pub String);

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid srs:// uri: {}", self.0)
    }
}

impl std::error::Error for UriError {}

pub fn parse(uri: &str, repository_id: &str) -> Result<SrsUri, UriError> {
    let rest = uri
        .strip_prefix(SCHEME)
        .ok_or_else(|| UriError(format!("expected scheme {SCHEME}, got '{uri}'")))?;
    let (repo, path) = rest
        .split_once('/')
        .ok_or_else(|| UriError(format!("missing path after repository id in '{uri}'")))?;
    if repo != repository_id {
        return Err(UriError(format!(
            "uri names repository '{repo}' but this server serves '{repository_id}'"
        )));
    }
    if let Some(ids) = path.strip_prefix("context/") {
        let (ids, query) = ids.split_once('?').unwrap_or((ids, ""));
        let mut exclude_relation_categories = vec![];
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            match pair.split_once('=') {
                Some(("excludeRelationCategories", v)) => exclude_relation_categories
                    .extend(v.split(',').filter(|c| !c.is_empty()).map(str::to_string)),
                _ => return Err(UriError(format!("unsupported query in '{uri}'"))),
            }
        }
        return match ids.split('/').collect::<Vec<_>>().as_slice() {
            [iid] if !iid.is_empty() => Ok(SrsUri::Context {
                container_id: None,
                instance_id: iid.to_string(),
                exclude_relation_categories,
            }),
            [cid, iid] if !cid.is_empty() && !iid.is_empty() => Ok(SrsUri::Context {
                container_id: Some(cid.to_string()),
                instance_id: iid.to_string(),
                exclude_relation_categories,
            }),
            _ => Err(UriError(format!("malformed resource path in '{uri}'"))),
        };
    }
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let is_tree = path == "tree" || path.starts_with("tree/");
    if !query.is_empty() && !is_tree {
        return Err(UriError(format!("unsupported query in '{uri}'")));
    }
    match path.split_once('/') {
        None => match path {
            "map" => Ok(SrsUri::Map),
            "navigation" => Ok(SrsUri::Navigation),
            "protocol" => Ok(SrsUri::ProtocolList),
            "tree" => Ok(SrsUri::Tree(parse_tree_query(query, uri)?)),
            "agent-index" => Ok(SrsUri::AgentIndex),
            "relation-types" => Ok(SrsUri::RelationTypes),
            other => Err(UriError(format!("unknown resource kind '{other}'"))),
        },
        Some((kind, id)) if !id.is_empty() && !id.contains('/') => match kind {
            "record" => Ok(SrsUri::Record(id.to_string())),
            "container" => Ok(SrsUri::Container(id.to_string())),
            "composition" => Ok(SrsUri::Composition(id.to_string())),
            "type" => Ok(SrsUri::Type(id.to_string())),
            "protocol" => Ok(SrsUri::Protocol(id.to_string())),
            "tree" => Ok(SrsUri::TreeFrom(
                id.to_string(),
                parse_tree_query(query, uri)?,
            )),
            other => Err(UriError(format!("unknown resource kind '{other}'"))),
        },
        Some(_) => Err(UriError(format!("malformed resource path in '{uri}'"))),
    }
}

pub fn format(kind: &SrsUri, repository_id: &str) -> String {
    match kind {
        SrsUri::Map => format!("{SCHEME}{repository_id}/map"),
        SrsUri::Navigation => format!("{SCHEME}{repository_id}/navigation"),
        SrsUri::Record(id) => resource_uri::record_uri(repository_id, id),
        SrsUri::Container(id) => resource_uri::container_uri(repository_id, id),
        SrsUri::Composition(id) => format!("{SCHEME}{repository_id}/composition/{id}"),
        SrsUri::Type(id) => resource_uri::type_uri(repository_id, id),
        SrsUri::ProtocolList => format!("{SCHEME}{repository_id}/protocol"),
        SrsUri::Protocol(id) => format!("{SCHEME}{repository_id}/protocol/{id}"),
        SrsUri::Tree(q) => format!("{SCHEME}{repository_id}/tree{}", tree_query_string(q)),
        SrsUri::TreeFrom(id, q) => {
            format!("{SCHEME}{repository_id}/tree/{id}{}", tree_query_string(q))
        }
        SrsUri::AgentIndex => format!("{SCHEME}{repository_id}/agent-index"),
        SrsUri::RelationTypes => resource_uri::relation_types_uri(repository_id),
        SrsUri::Context {
            container_id,
            instance_id,
            exclude_relation_categories,
        } => {
            let ids = match container_id {
                None => instance_id.clone(),
                Some(cid) => format!("{cid}/{instance_id}"),
            };
            let query = if exclude_relation_categories.is_empty() {
                String::new()
            } else {
                format!(
                    "?excludeRelationCategories={}",
                    exclude_relation_categories.join(",")
                )
            };
            format!("{SCHEME}{repository_id}/context/{ids}{query}")
        }
    }
}

pub fn record_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/record/{{instanceId}}")
}

pub fn tree_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/tree/{{instanceId}}{{?maxDepth,relationType,typeFilter}}")
}

pub fn context_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/context/{{containerId}}/{{instanceId}}{{?excludeRelationCategories}}")
}

pub fn type_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/type/{{typeId}}")
}

pub fn protocol_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/protocol/{{protocolId}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "11111111-2222-3333-4444-555555555555";

    #[test]
    fn context_uri_rejects_malformed() {
        for bad in ["context/", "context/a/", "context//b", "context/a/b/c"] {
            assert!(
                parse(&format!("srs://{REPO}/{bad}"), REPO).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn context_uri_query_parses_and_rejects_unknown_keys() {
        let u = format!("srs://{REPO}/context/c/i?excludeRelationCategories=composition,sequence");
        assert_eq!(
            parse(&u, REPO),
            Ok(SrsUri::Context {
                container_id: Some("c".into()),
                instance_id: "i".into(),
                exclude_relation_categories: vec!["composition".into(), "sequence".into()],
            })
        );
        assert!(parse(&format!("srs://{REPO}/context/i?bogus=1"), REPO).is_err());
        assert!(parse(
            &format!("srs://{REPO}/context/i?excludeRelationCategories=sequence&foo=1"),
            REPO
        )
        .is_err());
    }

    #[test]
    fn tree_uri_query_rejects_bad_depth_duplicates_unknown_keys_and_other_kinds() {
        for bad in [
            "tree?maxDepth=x",
            "tree?maxDepth=-1",
            "tree?maxDepth=1&maxDepth=2",
            "tree/abc?relationType=a&relationType=b",
            "tree?bogus=1",
            "tree?relationType=",
            "record/abc?maxDepth=1",
            "map?maxDepth=1",
        ] {
            assert!(
                parse(&format!("srs://{REPO}/{bad}"), REPO).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            parse(
                &format!("srs://{REPO}/tree/abc?typeFilter=ns/name&maxDepth=0"),
                REPO
            ),
            Ok(SrsUri::TreeFrom(
                "abc".into(),
                TreeQuery {
                    max_depth: Some(0),
                    relation_type: None,
                    type_filter: Some("ns/name".into()),
                }
            ))
        );
    }

    #[test]
    fn uri_roundtrip_all_kinds() {
        let kinds = [
            SrsUri::Map,
            SrsUri::Navigation,
            SrsUri::Record("abc".into()),
            SrsUri::Container("def".into()),
            SrsUri::Composition("ghi".into()),
            SrsUri::Type("jkl".into()),
            SrsUri::ProtocolList,
            SrsUri::Protocol("mno".into()),
            SrsUri::Tree(TreeQuery::default()),
            SrsUri::TreeFrom("mno".into(), TreeQuery::default()),
            SrsUri::Tree(TreeQuery {
                max_depth: Some(0),
                relation_type: Some("depends-on".into()),
                type_filter: Some("com.x/section".into()),
            }),
            SrsUri::TreeFrom(
                "mno".into(),
                TreeQuery {
                    max_depth: Some(2),
                    ..TreeQuery::default()
                },
            ),
            SrsUri::AgentIndex,
            SrsUri::Context {
                container_id: None,
                instance_id: "i".into(),
                exclude_relation_categories: vec![],
            },
            SrsUri::Context {
                container_id: Some("c".into()),
                instance_id: "i".into(),
                exclude_relation_categories: vec![],
            },
            SrsUri::Context {
                container_id: None,
                instance_id: "i".into(),
                exclude_relation_categories: vec!["composition".into(), "sequence".into()],
            },
        ];
        for kind in kinds {
            let uri = format(&kind, REPO);
            assert_eq!(parse(&uri, REPO), Ok(kind.clone()), "roundtrip for {uri}");
        }
    }
}
