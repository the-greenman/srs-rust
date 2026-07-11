//! Shared stdin JSON input handling for CLI handlers (issue #511).
//!
//! All create/update handlers that read a JSON body on stdin route through
//! these helpers so parse errors carry the JSON path into the input
//! (e.g. `sections[0]: missing field \`name\``) instead of only serde's
//! line/column, which is useless on single-line stdin JSON.

use anyhow::{anyhow, Result};
use serde::de::DeserializeOwned;
use std::io::Read;

/// Read all of stdin and deserialize it as `T` with JSON-path-aware errors.
///
/// `what` names the expected shape in the message (e.g. `"note"`).
pub fn from_stdin<T: DeserializeOwned>(what: &str) -> Result<T> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    from_str(what, &buf)
}

/// Read all of stdin as a raw JSON value (syntax check only).
///
/// Used by handlers that pass the raw value to a normalizing service, where
/// typed (path-aware) deserialization happens after normalization.
pub fn value_from_stdin(what: &str) -> Result<serde_json::Value> {
    from_stdin::<serde_json::Value>(what)
}

/// Deserialize a JSON string as `T`, attaching the JSON path to errors.
pub fn from_str<T: DeserializeOwned>(what: &str, raw: &str) -> Result<T> {
    let mut de = serde_json::Deserializer::from_str(raw);
    let value: T = serde_path_to_error::deserialize(&mut de).map_err(|e| {
        let path = e.path().to_string();
        let inner = e.into_inner();
        // "." is the document root; "?" means the path is unknown (e.g. a
        // syntax error before any structure was entered). Neither adds signal.
        if path == "." || path == "?" {
            anyhow!("Failed to parse {what} JSON: {inner}")
        } else {
            anyhow!("Failed to parse {what} JSON at {path}: {inner}")
        }
    })?;
    de.end()
        .map_err(|e| anyhow!("Failed to parse {what} JSON: trailing characters: {e}"))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Section {
        name: String,
        content: String,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct NoteShape {
        title: String,
        sections: Vec<Section>,
    }

    #[test]
    fn from_str_reports_json_path_for_nested_missing_field() {
        let raw = r#"{"title":"t","sections":[{"heading":"h","body":"b"}]}"#;
        let err = from_str::<NoteShape>("note", raw).unwrap_err().to_string();
        assert!(err.contains("sections[0]"), "error was: {err}");
        assert!(err.contains("missing field"), "error was: {err}");
    }

    #[test]
    fn from_str_reports_plain_error_at_top_level() {
        let err = from_str::<NoteShape>("note", "{").unwrap_err().to_string();
        assert!(
            err.starts_with("Failed to parse note JSON:"),
            "error was: {err}"
        );
    }

    #[test]
    fn from_str_rejects_trailing_garbage() {
        let raw = r#"{"title":"t","sections":[]} extra"#;
        let err = from_str::<NoteShape>("note", raw).unwrap_err().to_string();
        assert!(err.contains("trailing"), "error was: {err}");
    }
}
