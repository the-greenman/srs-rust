//! The shared structured diagnostic for the non-validation payload surfaces
//! (srs-rust#1491, ADR-057).
//!
//! `repo validate` has `ValidationDiagnostic` (ADR-055). The render, find,
//! container view, blueprint, navigation and type_schema results return their
//! diagnostics as a text list. Each now also returns `diagnosticsStructured`
//! beside it: the same findings as [`Diagnostic`] entries. The text list is
//! unchanged, code prefix included (its retirement is srs-rust#1496).

use crate::validation::DiagnosticSeverity;

/// One finding: `{severity, code, message, instanceId?}`.
///
/// `message` is the text-list entry verbatim (prefix included) so the two
/// lists agree entry for entry. `code` keeps today's spelling (renames are
/// srs-rust#1492); a site that names no code gets one derived from its rule.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

/// Collects the text list and the structured list together, so a site cannot
/// add to one and forget the other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    text: Vec<String>,
    structured: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, severity: DiagnosticSeverity, code: &str, message: String) {
        self.push_for(severity, code, message, None);
    }

    pub fn push_for(
        &mut self,
        severity: DiagnosticSeverity,
        code: &str,
        message: String,
        instance_id: Option<String>,
    ) {
        self.structured.push(Diagnostic {
            severity,
            code: code.to_string(),
            message: message.clone(),
            instance_id,
        });
        self.text.push(message);
    }

    pub fn warning(&mut self, code: &str, message: String) {
        self.push(DiagnosticSeverity::Warning, code, message);
    }

    pub fn error(&mut self, code: &str, message: String) {
        self.push(DiagnosticSeverity::Error, code, message);
    }

    /// Append another collection (nested service results).
    pub fn extend(&mut self, other: Diagnostics) {
        self.text.extend(other.text);
        self.structured.extend(other.structured);
    }

    /// The text list so far (what `diagnostics` will carry).
    pub fn texts(&self) -> &[String] {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn into_parts(self) -> (Vec<String>, Vec<Diagnostic>) {
        (self.text, self.structured)
    }
}

impl Diagnostics {
    /// Add a finding whose rule code is the one its message already leads with
    /// (the render projection's convention): `[R23] ...` gives `R23`,
    /// `[section:s1] ...` gives `section`, `ext:type-inheritance: ...` gives
    /// `ext:type-inheritance`. A message that leads with no tag gets the
    /// mechanically derived code `render` (srs-rust#1264 ruling, item 2:
    /// "a site that names no code gets a code derived mechanically"). A
    /// message carrying ` error:` after its tag is an error; everything else
    /// is a warning. The text list receives the message unchanged.
    pub fn push_derived(&mut self, message: impl Into<String>) {
        let message = message.into();
        let (code, severity) = derive_code(&message);
        self.push(severity, &code, message);
    }
}

fn derive_code(message: &str) -> (String, DiagnosticSeverity) {
    let severity = if message.contains("] error:") {
        DiagnosticSeverity::Error
    } else {
        DiagnosticSeverity::Warning
    };
    if let Some(rest) = message.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let tag = &rest[..end];
            let code = tag.split(':').next().unwrap_or(tag);
            if !code.is_empty() {
                return (code.to_string(), severity);
            }
        }
    }
    if message.starts_with("ext:type-inheritance:") {
        return ("ext:type-inheritance".to_string(), severity);
    }
    ("render".to_string(), severity)
}

/// Test guard (srs-rust#1491): the structured list is the text list, entry for
/// entry, each entry with a non-empty code. Used by every surface's test.
#[cfg(test)]
pub(crate) fn assert_parallel(text: &[String], structured: &[Diagnostic]) {
    assert_eq!(text.len(), structured.len(), "{text:?} vs {structured:?}");
    for (t, d) in text.iter().zip(structured) {
        assert_eq!(t, &d.message);
        assert!(!d.code.is_empty(), "entry without a code: {d:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_codes_follow_the_message_prefix() {
        let mut d = Diagnostics::new();
        d.push_derived("[R23] computed heading level 7 exceeds 6; clamped to 6");
        d.push_derived("[section:s1] error: needs a container");
        d.push_derived("ext:type-inheritance: boom");
        d.push_derived("field id x resolves to no Field definition");
        let (text, structured) = d.into_parts();
        assert_eq!(text.len(), 4);
        let codes: Vec<&str> = structured.iter().map(|d| d.code.as_str()).collect();
        assert_eq!(codes, ["R23", "section", "ext:type-inheritance", "render"]);
        assert_eq!(structured[1].severity, DiagnosticSeverity::Error);
        assert_eq!(structured[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(structured[0].message, text[0]);
    }

    #[test]
    fn structured_entry_serializes_camel_case_without_empty_instance_id() {
        let mut d = Diagnostics::new();
        d.warning("x", "m".into());
        d.push_for(DiagnosticSeverity::Error, "y", "n".into(), Some("i".into()));
        let (_, s) = d.into_parts();
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::json!([
                {"severity": "warning", "code": "x", "message": "m"},
                {"severity": "error", "code": "y", "message": "n", "instanceId": "i"}
            ])
        );
    }
}
