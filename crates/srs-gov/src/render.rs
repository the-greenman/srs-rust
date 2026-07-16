use serde_json::Value;

const RULE: &str = "────────────────────────────────────────────────────────────────";
const THIN: &str = "· · · · · · · · · · · · · · · · · · · · · · · · · · · · · · ·";

pub fn header(title: &str) {
    println!();
    println!("{RULE}");
    println!("  {title}");
    println!("{RULE}");
}

pub fn section(title: &str) {
    println!();
    println!("{THIN}");
    println!("  {title}");
    println!("{THIN}");
}

/// Print the governance container list (top-level `srs-gov` with no subcommand).
pub fn container_list(title: &str, rows: &[ContainerRow]) {
    header(title);
    println!();
    println!("  {:<18} {:<14} {:<5}  ID", "SECTION", "TYPE", "COUNT");
    println!("  {}", "─".repeat(70));
    for r in rows {
        println!(
            "  {} {:<16} {:<14} {:>5}  {}",
            r.icon,
            r.key,
            r.container_type,
            r.member_count,
            short_id(&r.container_id)
        );
    }
    println!();
    println!("  Run:  srs-gov <key> list");
}

pub struct ContainerRow {
    pub icon: &'static str,
    pub key: String,
    pub container_type: String,
    pub member_count: usize,
    pub container_id: String,
}

/// Print record fields in schema order using core-provided labels.
///
/// `schema_props` comes from `payload.schema` (full type schema, not just `.properties`).
/// `field_values` comes from `payload.record.fieldValues`. Row shaping (field ordering,
/// labeling, required-marking) is delegated to `tui_data::detail_rows` — the single
/// implementation shared with the TUI detail pane; this function owns only CLI text
/// formatting (skip-if-missing, wrap-if-long).
pub fn record_detail(record_id: &str, schema_props: &Value, field_values: &[Value]) {
    let rows = crate::tui_data::detail_rows(schema_props, field_values);

    header(&format!("Record  {}", short_id(record_id)));
    println!();
    for row in &rows {
        let Some(text) = row.value.as_deref() else {
            continue;
        };
        let marker = if row.required { "*" } else { " " };
        // Wrap long values
        if text.len() > 72 {
            println!("  {marker} {}:", row.label);
            for line in textwrap(text, 70) {
                println!("      {line}");
            }
        } else {
            println!("  {marker} {:<26} {text}", format!("{}:", row.label));
        }
    }
    println!();
}

pub fn repo_created(output: &str, title: &str, repository_id: &str, has_purpose: bool) {
    header(&format!("Created  {title}"));
    println!();
    println!("  File:          {output}");
    println!("  Repository ID: {repository_id}");
    println!("  Package:       com.mudemocracy.governance @1.0.0");
    println!();
    println!(
        "  Identity:      purpose record ({})",
        if has_purpose {
            "your purpose"
        } else {
            "placeholder"
        }
    );
    println!("  Root container: purpose record + Decision Log (RFC-013)");
    println!("  Containers scaffolded:");
    println!("    ⊕  Decision Log  — empty, ready for decisions");
    println!();
    println!("  Open in srs-web, or explore with:");
    println!("    srs-gov --repo {output}");
    println!("    srs repo validate --repo {output}");
    println!();
}

fn textwrap(s: &str, width: usize) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut end = (start + width).min(s.len());
        while end > start && !s.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = s[start..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| start + idx)
                .unwrap_or(s.len());
        }
        // try to break at a space
        let end = if end < s.len() {
            s[start..end]
                .rfind(' ')
                .map(|p| start + p + 1)
                .unwrap_or(end)
        } else {
            end
        };
        lines.push(s[start..end].trim_end());
        start = end;
    }
    lines
}

fn short_id(id: &str) -> &str {
    &id[..8.min(id.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textwrap_does_not_split_multibyte_boundaries() {
        let text = format!("{}—{}", "a".repeat(69), "decision");
        let lines = textwrap(&text, 70);

        assert!(!lines.is_empty());
        assert_eq!(lines.concat(), text);
    }

    #[test]
    fn record_detail_accepts_short_ids() {
        let schema = serde_json::json!({
            "required": ["title"],
            "properties": {
                "title": {
                    "title": "Title",
                    "x-srs-field-id": "field-title",
                    "x-srs-order": 1
                }
            }
        });
        let field_values = vec![serde_json::json!({
            "fieldId": "field-title",
            "value": "Short ID smoke"
        })];

        record_detail("abc", &schema, &field_values);
    }
}
