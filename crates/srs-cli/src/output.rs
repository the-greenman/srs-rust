use crate::commands::OutputFormat;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Structured output DTO that can be rendered to different formats
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputDTO {
    pub ok: bool,
    pub command: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<String>>,
}

impl OutputDTO {
    /// Create a successful output DTO
    pub fn ok(command: &str, payload: serde_json::Value) -> Self {
        Self {
            ok: true,
            command: command.to_string(),
            version: VERSION.to_string(),
            payload: Some(payload),
            diagnostics: None,
        }
    }

    /// Create an error output DTO
    pub fn err(command: &str, diagnostics: Vec<String>) -> Self {
        Self {
            ok: false,
            command: command.to_string(),
            version: VERSION.to_string(),
            payload: None,
            diagnostics: Some(diagnostics),
        }
    }

    /// Render the output to a string in the specified format
    pub fn render(self, format: OutputFormat, pretty: bool) -> String {
        match format {
            OutputFormat::Json => self.render_json(pretty),
            OutputFormat::Yaml => self.render_yaml(),
            OutputFormat::Text => self.render_text(),
        }
    }

    /// Render as JSON
    fn render_json(&self, pretty: bool) -> String {
        let value = serde_json::to_value(self).unwrap_or(json!(null));
        if pretty {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
        } else {
            value.to_string()
        }
    }

    /// Render as YAML
    fn render_yaml(&self) -> String {
        let value = serde_json::to_value(self).unwrap_or(serde_json::json!(null));
        serde_yaml::to_string(&value).unwrap_or_else(|e| format!("yaml serialization error: {}", e))
    }

    /// Render as text (planned - currently returns diagnostic)
    fn render_text(&self) -> String {
        // Text format is planned but not yet implemented
        // Return a diagnostic message in the error envelope format
        let diag = if self.ok {
            format!(
                "Text format is planned but not yet implemented for command: {}\nPayload: {}",
                self.command,
                self.payload
                    .as_ref()
                    .map(|p| p.to_string())
                    .unwrap_or_default()
            )
        } else {
            format!(
                "Text format is planned but not yet implemented for command: {}\nErrors: {}",
                self.command,
                self.diagnostics
                    .as_ref()
                    .map(|d| d.join(", "))
                    .unwrap_or_default()
            )
        };
        diag
    }
}

/// Serialize a typed payload struct and return a compact JSON ok response.
/// This is the preferred function for command handlers; use `ok` only for
/// cases where a `serde_json::Value` is already constructed.
pub fn serialize<T: serde::Serialize>(command: &str, payload: T) -> anyhow::Result<String> {
    let value = serde_json::to_value(payload)
        .map_err(|e| anyhow::anyhow!("Failed to serialize payload for '{}': {}", command, e))?;
    Ok(ok(command, value))
}

/// Legacy convenience function for JSON ok output (always compact)
pub fn ok(command: &str, payload: serde_json::Value) -> String {
    OutputDTO::ok(command, payload).render(OutputFormat::Json, false)
}

/// Legacy convenience function for JSON error output (always compact)
pub fn err(command: &str, diagnostics: Vec<String>) -> String {
    OutputDTO::err(command, diagnostics).render(OutputFormat::Json, false)
}
