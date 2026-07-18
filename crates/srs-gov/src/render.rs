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

pub fn attachment_added(content_path: &str, document_id: &str, base_dir: &str) {
    header("Attachment stored");
    println!();
    println!("  Path:        {base_dir}/{content_path}");
    println!("  Document ID: {document_id}");
    println!();
    println!("  Run: srs-gov attachment list  to see all attachments");
    println!();
}

pub fn attachment_list(base_dir: &str, entries: &[serde_json::Value]) {
    header(&format!("Attachments  —  {base_dir}/"));
    println!();
    if entries.is_empty() {
        println!("  (no attachments)");
        println!();
        return;
    }
    println!("  {:<50}  TITLE", "PATH");
    println!("  {}", "─".repeat(70));
    for e in entries {
        let path = e["path"].as_str().unwrap_or("");
        let title = e["title"].as_str().unwrap_or("—");
        println!("  {:<50}  {title}", path);
    }
    println!();
}

pub struct LinkedAttachment {
    pub document_id: String,
    pub title: Option<String>,
    pub content_path: Option<String>,
    /// Size in bytes, `None` when unavailable (JSON-store repos or file not found).
    pub size_bytes: Option<u64>,
}

pub fn linked_attachments(attachments: &[LinkedAttachment]) {
    if attachments.is_empty() {
        return;
    }
    section("Linked Attachments");
    println!("  {:<42}  {:<28}  SIZE", "PATH · DOCUMENT ID", "TITLE");
    println!("  {}", "─".repeat(78));
    for a in attachments {
        let path_str = a.content_path.as_deref().unwrap_or("(no path)");
        let path_col = format!("{} ({})", path_str, short_id(&a.document_id));
        let title_str = a.title.as_deref().unwrap_or("—");
        let size_str = a.size_bytes.map(fmt_size).unwrap_or_else(|| "—".into());
        println!("  {:<42}  {:<28}  {size_str}", path_col, title_str);
    }
    println!();
}

fn fmt_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{} MB", bytes / 1_048_576)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
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
    fn linked_attachments_empty_silent() {
        linked_attachments(&[]);
    }

    #[test]
    fn linked_attachments_renders_row() {
        linked_attachments(&[LinkedAttachment {
            document_id: "doc-abc12345-test".to_string(),
            title: Some("Q3 Report".to_string()),
            content_path: Some("phase-1/report.pdf".to_string()),
            size_bytes: Some(2048),
        }]);
    }

    #[test]
    fn fmt_size_thresholds() {
        assert_eq!(fmt_size(0), "0 B");
        assert_eq!(fmt_size(1023), "1023 B");
        assert_eq!(fmt_size(1024), "1 KB");
        assert_eq!(fmt_size(1_048_576), "1 MB");
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
