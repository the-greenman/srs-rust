//! The transport-independent `srs://` resource URI contract.

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
    Tree,
    TreeFrom(String),
    AgentIndex,
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
    match path.split_once('/') {
        None => match path {
            "map" => Ok(SrsUri::Map),
            "navigation" => Ok(SrsUri::Navigation),
            "protocol" => Ok(SrsUri::ProtocolList),
            "tree" => Ok(SrsUri::Tree),
            "agent-index" => Ok(SrsUri::AgentIndex),
            other => Err(UriError(format!("unknown resource kind '{other}'"))),
        },
        Some((kind, id)) if !id.is_empty() && !id.contains('/') => match kind {
            "record" => Ok(SrsUri::Record(id.to_string())),
            "container" => Ok(SrsUri::Container(id.to_string())),
            "composition" => Ok(SrsUri::Composition(id.to_string())),
            "type" => Ok(SrsUri::Type(id.to_string())),
            "protocol" => Ok(SrsUri::Protocol(id.to_string())),
            "tree" => Ok(SrsUri::TreeFrom(id.to_string())),
            other => Err(UriError(format!("unknown resource kind '{other}'"))),
        },
        Some(_) => Err(UriError(format!("malformed resource path in '{uri}'"))),
    }
}

pub fn format(kind: &SrsUri, repository_id: &str) -> String {
    match kind {
        SrsUri::Map => format!("{SCHEME}{repository_id}/map"),
        SrsUri::Navigation => format!("{SCHEME}{repository_id}/navigation"),
        SrsUri::Record(id) => format!("{SCHEME}{repository_id}/record/{id}"),
        SrsUri::Container(id) => format!("{SCHEME}{repository_id}/container/{id}"),
        SrsUri::Composition(id) => format!("{SCHEME}{repository_id}/composition/{id}"),
        SrsUri::Type(id) => format!("{SCHEME}{repository_id}/type/{id}"),
        SrsUri::ProtocolList => format!("{SCHEME}{repository_id}/protocol"),
        SrsUri::Protocol(id) => format!("{SCHEME}{repository_id}/protocol/{id}"),
        SrsUri::Tree => format!("{SCHEME}{repository_id}/tree"),
        SrsUri::TreeFrom(id) => format!("{SCHEME}{repository_id}/tree/{id}"),
        SrsUri::AgentIndex => format!("{SCHEME}{repository_id}/agent-index"),
    }
}

pub fn record_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/record/{{instanceId}}")
}

pub fn tree_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/tree/{{instanceId}}")
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
            SrsUri::Tree,
            SrsUri::TreeFrom("mno".into()),
            SrsUri::AgentIndex,
        ];
        for kind in kinds {
            let uri = format(&kind, REPO);
            assert_eq!(parse(&uri, REPO), Ok(kind.clone()), "roundtrip for {uri}");
        }
    }
}
