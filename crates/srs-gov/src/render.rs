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
pub fn container_list(rows: &[ContainerRow]) {
    header("Governance   —   top-level containers");
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
            &r.container_id[..8]
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
/// `schema_props` comes from `payload.schema.properties` (type schema).
/// `field_values` comes from `payload.record.fieldValues`.
pub fn record_detail(record_id: &str, schema_props: &Value, field_values: &[Value]) {
    // Build fieldId → value map
    let mut fv_map: std::collections::HashMap<&str, &Value> = std::collections::HashMap::new();
    for fv in field_values {
        if let (Some(fid), Some(val)) = (fv["fieldId"].as_str(), fv.get("value")) {
            fv_map.insert(fid, val);
        }
    }

    // schema_props is the full schema object; properties and required are sub-keys
    let schema_required = Value::Array(vec![]);
    let schema_required_arr = schema_props.get("required").unwrap_or(&schema_required);
    let props = match schema_props.get("properties").and_then(|p| p.as_object()) {
        Some(m) => m,
        None => return,
    };
    let required_set: std::collections::HashSet<&str> = schema_required_arr
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut fields: Vec<(i64, &str, &str, bool)> = props
        .iter()
        .filter_map(|(name, prop)| {
            let order = prop
                .get("x-srs-order")
                .and_then(|v| v.as_i64())
                .unwrap_or(99);
            let label = prop.get("title").and_then(|v| v.as_str()).unwrap_or(name);
            let fid = prop.get("x-srs-field-id").and_then(|v| v.as_str())?;
            Some((order, label, fid, required_set.contains(name.as_str())))
        })
        .collect();
    fields.sort_by_key(|f| f.0);

    header(&format!("Record  {}", &record_id[..8]));
    println!();
    for (_, label, fid, req) in &fields {
        if let Some(val) = fv_map.get(fid) {
            let marker = if *req { "*" } else { " " };
            let owned;
            let text = if let Some(s) = val.as_str() {
                s
            } else {
                owned = val.to_string();
                &owned
            };
            // Wrap long values
            if text.len() > 72 {
                println!("  {marker} {label}:");
                for line in textwrap(text, 70) {
                    println!("      {line}");
                }
            } else {
                println!("  {marker} {:<26} {text}", format!("{label}:"));
            }
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
    println!("  Containers scaffolded:");
    println!(
        "    §  Articles      — charter article ({})",
        if has_purpose {
            "your purpose"
        } else {
            "placeholder"
        }
    );
    println!("    ⊕  Decision Log  — empty, ready for decisions");
    println!("    §  Roles         — empty, ready for role definitions");
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
        let end = (start + width).min(s.len());
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
