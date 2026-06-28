//! Deterministic text projection — the searchable-content primitive of the
//! `ext:discovery` contract (RFC-012, `docs/schema/2.0/discovery.json`).
//!
//! [`project_text`] turns a [`Record`] into an ordered stream of [`TextSegment`]s.
//! A field's [`ValueType`](srs_core::types::field::ValueType) decides whether its
//! value is searchable. Normalization (NFC + Unicode simple lowercasing) is applied
//! **at match time** via [`normalize`], not at construction — segment `text` holds
//! the raw stored value so the stream is reproducible by any implementation.

use crate::error::RepositoryError;
use crate::package_service;
use crate::record_label;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::types::record::{FieldValue, Record};
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

/// Sentinel `fieldId`/`fieldName` for the display-label segment.
pub const LABEL_SENTINEL: &str = "label";
/// Sentinel `fieldId`/`fieldName` for tag segments.
pub const TAG_SENTINEL: &str = "tag";

/// One searchable unit of a record's text projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSegment {
    /// Field UUID, or a sentinel ([`LABEL_SENTINEL`] / [`TAG_SENTINEL`]).
    pub field_id: String,
    /// Field name (snake_case), or a sentinel.
    pub field_name: String,
    /// Raw stored text. Normalization is applied at match time, not here.
    pub text: String,
}

/// Field text metadata derived from the repository package, built once per batch
/// with [`build_field_text_index`] and reused across every record. Both maps are
/// prebuilt so projecting a record allocates nothing here.
pub struct FieldTextIndex {
    /// `field_id → field_name`, also the map [`record_label::record_display_label`] expects.
    names: HashMap<String, String>,
    /// Field ids whose `ValueType` is searchable.
    searchable: HashSet<String>,
}

impl FieldTextIndex {
    /// Borrow the prebuilt `field_id → field_name` map (no per-call allocation).
    pub(crate) fn names(&self) -> &HashMap<String, String> {
        &self.names
    }

    fn name_of(&self, field_id: &str) -> Option<&str> {
        self.names.get(field_id).map(String::as_str)
    }

    fn is_searchable(&self, field_id: &str) -> bool {
        self.searchable.contains(field_id)
    }
}

/// The searchable `ValueType`s per RFC-012 (compared against the lowercase
/// serialized form produced by `FieldSummary.value_type`).
fn is_searchable(value_type: &str) -> bool {
    matches!(
        value_type,
        "string" | "text" | "url" | "select" | "multiselect"
    )
}

/// Build the field text index from the repository package.
pub fn build_field_text_index(
    store: &dyn RepositoryStore,
) -> Result<FieldTextIndex, RepositoryError> {
    let fields = package_service::list_fields(store)?;
    let mut names = HashMap::new();
    let mut searchable = HashSet::new();
    for f in fields {
        if is_searchable(&f.value_type) {
            searchable.insert(f.id.clone());
        }
        names.insert(f.id, f.name);
    }
    Ok(FieldTextIndex { names, searchable })
}

/// Apply RFC-012 normalization: NFC then Unicode simple lowercasing. Used at match
/// time on both the segment text and the query needle.
pub fn normalize(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// Project a record into its ordered, deterministic text-segment stream.
///
/// Order: top-level `field_values` (record order, incl. repeated `entries`) →
/// `group_values` → display label → tags.
pub fn project_text(record: &Record, index: &FieldTextIndex) -> Vec<TextSegment> {
    let mut segments = Vec::new();

    for fv in &record.field_values {
        push_field_value(&mut segments, index, fv);
    }

    if let Some(groups) = &record.group_values {
        for group in groups {
            for entry in &group.entries {
                for fv in &entry.field_values {
                    push_field_value(&mut segments, index, fv);
                }
            }
        }
    }

    let label = record_label::record_display_label(record, index.names());
    if !label.is_empty() {
        segments.push(TextSegment {
            field_id: LABEL_SENTINEL.to_string(),
            field_name: LABEL_SENTINEL.to_string(),
            text: label,
        });
    }

    if let Some(tags) = &record.tags {
        for tag in tags {
            segments.push(TextSegment {
                field_id: TAG_SENTINEL.to_string(),
                field_name: TAG_SENTINEL.to_string(),
                text: tag.clone(),
            });
        }
    }

    segments
}

fn push_field_value(segments: &mut Vec<TextSegment>, index: &FieldTextIndex, fv: &FieldValue) {
    if !index.is_searchable(&fv.field_id) {
        return;
    }
    let Some(field_name) = index.name_of(&fv.field_id) else {
        return;
    };
    let mut push = |text: String| {
        segments.push(TextSegment {
            field_id: fv.field_id.clone(),
            field_name: field_name.to_string(),
            text,
        });
    };
    for text in value_strings(&fv.value) {
        push(text);
    }
    if let Some(entries) = &fv.entries {
        for entry in entries {
            for text in value_strings(&entry.value) {
                push(text);
            }
        }
    }
}

/// Extract searchable strings from a stored value: a string scalar, or each string
/// element of an array (Multiselect). Non-string JSON yields nothing.
fn value_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::field::{Field, ValueType};
    use srs_core::types::record::{FieldGroupEntry, FieldGroupValue, FieldValueEntry};
    use std::collections::HashMap;

    const TITLE: &str = "00000000-0000-4000-8000-00000000f001";
    const BODY: &str = "00000000-0000-4000-8000-00000000f002";
    const COUNT: &str = "00000000-0000-4000-8000-00000000f003";
    const TAGS_FIELD: &str = "00000000-0000-4000-8000-00000000f004";

    fn field(id: &str, name: &str, vt: ValueType) -> Field {
        Field {
            id: id.to_string(),
            namespace: "example".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            ai_guidance: serde_json::json!({}),
            value_type: vt,
            allowed_values: None,
            vocabulary_ref: None,
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    fn index() -> FieldTextIndex {
        let entries = [
            (TITLE, "title", true),
            (BODY, "body", true),
            (COUNT, "count", false),
            (TAGS_FIELD, "labels", true),
        ];
        let names = entries
            .iter()
            .map(|(id, name, _)| (id.to_string(), name.to_string()))
            .collect();
        let searchable = entries
            .iter()
            .filter(|(_, _, searchable)| *searchable)
            .map(|(id, _, _)| id.to_string())
            .collect();
        FieldTextIndex { names, searchable }
    }

    fn fv(field_id: &str, value: serde_json::Value) -> FieldValue {
        FieldValue {
            field_id: field_id.to_string(),
            value,
            entries: None,
            source: None,
            edited_at: None,
        }
    }

    fn record(field_values: Vec<FieldValue>) -> Record {
        Record {
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "example".to_string(),
            type_name: "entry".to_string(),
            field_values,
            group_values: None,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: HashMap::new(),
        }
    }

    fn texts(segments: &[TextSegment]) -> Vec<&str> {
        segments.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn projects_searchable_value_types_and_skips_non_searchable() {
        let rec = record(vec![
            fv(TITLE, serde_json::json!("Adopt consent")),
            fv(BODY, serde_json::json!("Use consent for changes")),
            fv(COUNT, serde_json::json!(42)),
        ]);
        let segments = project_text(&rec, &index());
        // Number field excluded; title + body present; label appended (= title).
        assert_eq!(
            texts(&segments),
            vec!["Adopt consent", "Use consent for changes", "Adopt consent"]
        );
        assert_eq!(segments.last().unwrap().field_id, LABEL_SENTINEL);
    }

    #[test]
    fn includes_repeated_entries_and_multiselect_arrays() {
        let mut multi = fv(TAGS_FIELD, serde_json::json!(["alpha", "beta"]));
        multi.entries = Some(vec![FieldValueEntry {
            value: serde_json::json!("gamma"),
            source: None,
            edited_at: None,
        }]);
        let rec = record(vec![multi]);
        let segments = project_text(&rec, &index());
        assert!(texts(&segments).contains(&"alpha"));
        assert!(texts(&segments).contains(&"beta"));
        assert!(texts(&segments).contains(&"gamma"));
    }

    #[test]
    fn includes_group_values() {
        let mut rec = record(vec![fv(TITLE, serde_json::json!("Root"))]);
        rec.group_values = Some(vec![FieldGroupValue {
            group_id: "g1".to_string(),
            entries: vec![FieldGroupEntry {
                field_values: vec![fv(BODY, serde_json::json!("nested body"))],
                entry_id: None,
            }],
        }]);
        let segments = project_text(&rec, &index());
        assert!(texts(&segments).contains(&"nested body"));
    }

    #[test]
    fn appends_label_and_tag_segments() {
        let mut rec = record(vec![fv(TITLE, serde_json::json!("Heading"))]);
        rec.tags = Some(vec!["policy".to_string(), "ops".to_string()]);
        let segments = project_text(&rec, &index());
        let tag_segs: Vec<&str> = segments
            .iter()
            .filter(|s| s.field_id == TAG_SENTINEL)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(tag_segs, vec!["policy", "ops"]);
    }

    #[test]
    fn normalize_is_nfc_and_lowercase() {
        // U+00C9 (É precomposed) and decomposed E + U+0301 normalize equal.
        assert_eq!(normalize("\u{00C9}cole"), normalize("E\u{0301}cole"));
        assert_eq!(normalize("MixedCase"), "mixedcase");
    }

    #[test]
    fn deterministic_segment_order() {
        let rec = record(vec![
            fv(BODY, serde_json::json!("b")),
            fv(TITLE, serde_json::json!("t")),
        ]);
        let a = project_text(&rec, &index());
        let b = project_text(&rec, &index());
        assert_eq!(a, b);
        // body precedes title (record order), then label.
        assert_eq!(a[0].field_name, "body");
        assert_eq!(a[1].field_name, "title");
    }

    #[test]
    fn value_type_serializes_lowercase_matches_searchable_set() {
        // Guards the string contract between FieldSummary.value_type and is_searchable.
        for (vt, searchable) in [
            (ValueType::String, true),
            (ValueType::Text, true),
            (ValueType::Url, true),
            (ValueType::Select, true),
            (ValueType::Multiselect, true),
            (ValueType::Number, false),
            (ValueType::Boolean, false),
            (ValueType::Date, false),
        ] {
            let s = format!("{:?}", field("x", "x", vt).value_type).to_lowercase();
            assert_eq!(is_searchable(&s), searchable, "value_type {s}");
        }
    }
}
