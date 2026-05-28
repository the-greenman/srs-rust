use serde_json::json;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// All output: { ok, command, version, diagnostics, ...payload }
pub fn ok(command: &str, payload: serde_json::Value) -> String {
    json!({
        "ok": true,
        "command": command,
        "version": VERSION,
        "payload": payload
    })
    .to_string()
}

pub fn err(command: &str, diagnostics: Vec<String>) -> String {
    json!({
        "ok": false,
        "command": command,
        "version": VERSION,
        "diagnostics": diagnostics
    })
    .to_string()
}
