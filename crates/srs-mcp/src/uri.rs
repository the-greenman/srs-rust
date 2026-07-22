//! The `srs://` resource URI scheme — implementation tooling, not spec (ADR-037).
//!
//! `srs://<repositoryId>/map`
//! `srs://<repositoryId>/navigation`
//! `srs://<repositoryId>/record/<instanceId>`
//! `srs://<repositoryId>/container/<containerId>`
//! `srs://<repositoryId>/view/<documentViewId>`
//!
//! Every component is an existing SRS identifier; the scheme adds no semantics.

use std::fmt;

const SCHEME: &str = "srs://";

/// A parsed `srs://` resource address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SrsUri {
    Map,
    Navigation,
    Record(String),
    Container(String),
    View(String),
}

/// Failure to parse an `srs://` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriError(pub String);

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid srs:// uri: {}", self.0)
    }
}

impl std::error::Error for UriError {}

/// Parse a URI against the serving repository's identity. A URI naming a
/// different repository is rejected — this server exposes exactly one repo.
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
            other => Err(UriError(format!("unknown resource kind '{other}'"))),
        },
        Some((kind, id)) if !id.is_empty() && !id.contains('/') => match kind {
            "record" => Ok(SrsUri::Record(id.to_string())),
            "container" => Ok(SrsUri::Container(id.to_string())),
            "view" => Ok(SrsUri::View(id.to_string())),
            other => Err(UriError(format!("unknown resource kind '{other}'"))),
        },
        Some(_) => Err(UriError(format!("malformed resource path in '{uri}'"))),
    }
}

/// Format an address back to its URI form.
pub fn format(kind: &SrsUri, repository_id: &str) -> String {
    match kind {
        SrsUri::Map => format!("{SCHEME}{repository_id}/map"),
        SrsUri::Navigation => format!("{SCHEME}{repository_id}/navigation"),
        SrsUri::Record(id) => format!("{SCHEME}{repository_id}/record/{id}"),
        SrsUri::Container(id) => format!("{SCHEME}{repository_id}/container/{id}"),
        SrsUri::View(id) => format!("{SCHEME}{repository_id}/view/{id}"),
    }
}

/// RFC 6570 template for record resources.
pub fn record_template(repository_id: &str) -> String {
    format!("{SCHEME}{repository_id}/record/{{instanceId}}")
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
            SrsUri::View("ghi".into()),
        ];
        for kind in kinds {
            let uri = format(&kind, REPO);
            assert_eq!(parse(&uri, REPO), Ok(kind.clone()), "roundtrip for {uri}");
        }
    }

    #[test]
    fn rejects_wrong_scheme_repo_and_shape() {
        assert!(parse("file:///x", REPO).is_err());
        assert!(parse("srs://other-repo/map", REPO).is_err());
        assert!(parse(&format!("srs://{REPO}/unknown"), REPO).is_err());
        assert!(parse(&format!("srs://{REPO}/record/"), REPO).is_err());
        assert!(parse(&format!("srs://{REPO}/record/a/b"), REPO).is_err());
        assert!(parse(&format!("srs://{REPO}"), REPO).is_err());
    }
}
