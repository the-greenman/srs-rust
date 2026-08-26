//! Deterministic text projection — the searchable-content primitive of the
//! `ext:discovery` contract (RFC-012, `docs/schema/2.0/discovery.json`).
//!
//! [`project_text`] turns a [`Record`] into an ordered stream of [`TextSegment`]s.
//! A field's [`FieldType`](srs_core::types::field::FieldType) decides whether its
//! value is searchable. Normalization (NFC + Unicode simple lowercasing) is applied
//! **at match time** via [`normalize`], not at construction — segment `text` holds
//! the raw stored value so the stream is reproducible by any implementation.

use crate::error::RepositoryError;
use crate::record_label;
use crate::store::RepositoryStore;
use serde::{Deserialize, Serialize};
use srs_core::types::note::Note;
use srs_core::types::record::Record;
use std::collections::{HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

/// Sentinel `fieldId`/`fieldName` for the display-label segment.
pub const LABEL_SENTINEL: &str = "label";
/// Sentinel `fieldId`/`fieldName` for tag segments.
pub const TAG_SENTINEL: &str = "tag";
/// Sentinel `fieldId`/`fieldName` for a Tier-0 Note's title segment.
pub const NOTE_TITLE_SENTINEL: &str = "note-title";
/// Sentinel `fieldId` for a Tier-0 Note section segment; `fieldName` is the
/// section's `name`.
pub const NOTE_SECTION_SENTINEL: &str = "note-section";
/// Sentinel `fieldId`/`fieldName` for a Tier-1 TypedRecord's title segment.
pub const TYPED_RECORD_TITLE_SENTINEL: &str = "typed-record-title";
/// Sentinel `fieldId` for a Tier-1 TypedRecord field segment; `fieldName` is the
/// `TypedField.name`.
pub const TYPED_RECORD_FIELD_SENTINEL: &str = "typed-record-field";

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
/// with [`build_field_text_index`] and reused across every record. All maps are
/// prebuilt so projecting a record allocates nothing here.
pub struct FieldTextIndex {
    /// `field_id → field_name`, also the map [`record_label::record_display_label`] expects.
    names: HashMap<String, String>,
    /// `Field.name`s whose `fieldType` projects searchable text — the RFC-039
    /// carrier keys by name, so searchability is tested on the key.
    searchable_names: HashSet<String>,
    /// `field_name → field_id` for `TextSegment.field_id` (Type-mediated
    /// recovery; addressability keeps citing fields by id).
    ids_by_name: HashMap<String, String>,
    /// RFC-020 — `(type_id, type_version) → identityFieldId`, the other map
    /// [`record_label::record_display_label`] expects.
    identity_field_ids: HashMap<(String, u32), String>,
}

impl FieldTextIndex {
    /// Borrow the prebuilt `field_id → field_name` map (no per-call allocation).
    pub(crate) fn names(&self) -> &HashMap<String, String> {
        &self.names
    }

    /// Borrow the prebuilt `(type_id, type_version) → identityFieldId` map.
    pub(crate) fn identity_field_ids(&self) -> &HashMap<(String, u32), String> {
        &self.identity_field_ids
    }

    fn is_searchable_name(&self, name: &str) -> bool {
        self.searchable_names.contains(name)
    }
}

/// Build the field text index from the repository package.
///
/// Loads the package once and derives `names`, `searchable`, and `identity_field_ids`
/// from it directly, rather than calling `package_service::list_fields` (its own
/// `store.load_package()`) and `record_label::build_identity_field_index` (a second
/// `store.load_package()`) — `store.load_package()` has no caching and re-reads/re-parses
/// every package file on `FileStore`.
pub fn build_field_text_index(
    store: &dyn RepositoryStore,
) -> Result<FieldTextIndex, RepositoryError> {
    let package = store.load_package()?;
    let mut names = HashMap::new();
    let mut searchable_names = HashSet::new();
    let mut ids_by_name = HashMap::new();
    for f in &package.fields {
        // I-120 / RFC-012 `[R8]`: `datatype == string` **and** an allow-listed
        // `format`. Not datatype alone — RFC-032 Revision 7 excludes the
        // string-datatyped `uuid` and `email` formats, so a field can be
        // `datatype: string` and still contribute no `TextSegment`s. See
        // `FieldType::is_text_searchable` for why the datatype-only reading is
        // sound over the legacy eight and unsound over the model as a whole.
        // Composites (`ref` in both modes, `dependent`, `map`) are excluded
        // outright — no composite recursion is defined (RFC-032 Rev 7).
        if f.field_type.is_text_searchable() {
            searchable_names.insert(f.name.clone());
        }
        names.insert(f.id.clone(), f.name.clone());
        ids_by_name.insert(f.name.clone(), f.id.clone());
    }
    let identity_field_ids = record_label::identity_field_index_from_package(&package);
    Ok(FieldTextIndex {
        names,
        searchable_names,
        ids_by_name,
        identity_field_ids,
    })
}

/// Apply RFC-012 normalization: NFC then Unicode simple lowercasing. Used at match
/// time on both the segment text and the query needle.
pub fn normalize(s: &str) -> String {
    s.nfc().collect::<String>().to_lowercase()
}

/// Project a record into its ordered, deterministic text-segment stream.
///
/// Order: `fieldValues` keys in stored order — which [R18] fixes to
/// `FieldAssignment.order`, so the projection is reproducible from content
/// (this supersedes RFC-012's array-order signal; [R18] governs) → display
/// label → tags.
pub fn project_text(record: &Record, index: &FieldTextIndex) -> Vec<TextSegment> {
    let mut segments = Vec::new();

    for (name, value) in record.field_values.iter() {
        push_field_value(&mut segments, index, name, value);
    }

    let label =
        record_label::record_display_label(record, index.identity_field_ids(), index.names());
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

fn push_field_value(
    segments: &mut Vec<TextSegment>,
    index: &FieldTextIndex,
    name: &str,
    value: &serde_json::Value,
) {
    if !index.is_searchable_name(name) {
        return;
    }
    let field_id = index.ids_by_name.get(name).cloned().unwrap_or_default();
    for text in value_strings(value) {
        segments.push(TextSegment {
            field_id: field_id.clone(),
            field_name: name.to_string(),
            text,
        });
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

/// Project a Tier-0 Note into its ordered, deterministic text-segment stream
/// (RFC-012 Change B).
///
/// Order: title (if non-empty) → sections (array order, non-empty `content` only) →
/// tags.
pub fn project_note_text(note: &Note) -> Vec<TextSegment> {
    let mut segments = Vec::new();

    if let Some(title) = note.title.as_deref() {
        if !title.is_empty() {
            segments.push(TextSegment {
                field_id: NOTE_TITLE_SENTINEL.to_string(),
                field_name: NOTE_TITLE_SENTINEL.to_string(),
                text: title.to_string(),
            });
        }
    }

    for section in &note.sections {
        if !section.content.is_empty() {
            segments.push(TextSegment {
                field_id: NOTE_SECTION_SENTINEL.to_string(),
                field_name: section.name.clone(),
                text: section.content.clone(),
            });
        }
    }

    if let Some(tags) = &note.tags {
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

/// Project a Tier-1 TypedRecord into its ordered, deterministic text-segment
/// stream (RFC-012 Change B). Operates on the raw JSON value — no typed
/// `TypedRecord` struct exists in `srs-core` yet (see
/// `docs/schema/2.0/typed-record.json` for the storage shape).
///
/// Order: title (if non-empty) → `fields[]` (array order, searchable and
/// non-empty only) → tags. RFC-039 [R8]: a TypedField carries an inline
/// `fieldType`; it is searchable when `datatype == "string"` with an
/// allow-listed prose/uri `format` (mirroring I-120 at Tier 2). A TypedField
/// with no `fieldType` is a revision ≤ 1 document ([R9]) — the reader rejects
/// it before projection; here it defensively contributes nothing.
pub fn project_typed_record_text(value: &serde_json::Value) -> Vec<TextSegment> {
    let mut segments = Vec::new();

    if let Some(title) = value.get("title").and_then(|v| v.as_str()) {
        if !title.is_empty() {
            segments.push(TextSegment {
                field_id: TYPED_RECORD_TITLE_SENTINEL.to_string(),
                field_name: TYPED_RECORD_TITLE_SENTINEL.to_string(),
                text: title.to_string(),
            });
        }
    }

    if let Some(fields) = value.get("fields").and_then(|v| v.as_array()) {
        for field in fields {
            let Some(name) = field.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let field_value = field.get("value").unwrap_or(&serde_json::Value::Null);

            let searchable = field
                .get("fieldType")
                .map(|ft| {
                    ft.get("datatype").and_then(|d| d.as_str()) == Some("string")
                        && matches!(
                            ft.get("format").and_then(|f| f.as_str()),
                            None | Some("plain") | Some("markdown") | Some("uri")
                        )
                })
                .unwrap_or(false);
            if !searchable {
                continue;
            }

            for text in value_strings(field_value) {
                if !text.is_empty() {
                    segments.push(TextSegment {
                        field_id: TYPED_RECORD_FIELD_SENTINEL.to_string(),
                        field_name: name.to_string(),
                        text,
                    });
                }
            }
        }
    }

    if let Some(tags) = value.get("tags").and_then(|v| v.as_array()) {
        for tag in tags.iter().filter_map(|t| t.as_str()) {
            segments.push(TextSegment {
                field_id: TAG_SENTINEL.to_string(),
                field_name: TAG_SENTINEL.to_string(),
                text: tag.to_string(),
            });
        }
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use srs_core::types::field::{AiGuidance, Field, FieldType, LegacyValueType, StringFormat};
    use srs_core::types::record::FieldValues;
    use std::collections::HashMap;

    const TITLE: &str = "00000000-0000-4000-8000-00000000f001";
    const BODY: &str = "00000000-0000-4000-8000-00000000f002";
    const COUNT: &str = "00000000-0000-4000-8000-00000000f003";
    const TAGS_FIELD: &str = "00000000-0000-4000-8000-00000000f004";

    fn field(id: &str, name: &str, vt: FieldType) -> Field {
        Field {
            schema: None,
            id: id.to_string(),
            namespace: "example".to_string(),
            name: name.to_string(),
            version: 1,
            description: String::new(),
            instructions: None,
            ai_guidance: Some(AiGuidance {
                purpose: "Test guidance".to_string(),
                ..Default::default()
            }),
            field_type: vt.clone(),
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// Derive a FieldTextIndex from real Fields, exactly as
    /// `build_field_text_index` derives it, rather than asserting searchability
    /// by a hand-maintained boolean that could drift from the projection.
    fn index_from(fields: &[Field]) -> FieldTextIndex {
        FieldTextIndex {
            names: fields
                .iter()
                .map(|f| (f.id.clone(), f.name.clone()))
                .collect(),
            searchable_names: fields
                .iter()
                .filter(|f| f.field_type.is_text_searchable())
                .map(|f| f.name.clone())
                .collect(),
            ids_by_name: fields
                .iter()
                .map(|f| (f.name.clone(), f.id.clone()))
                .collect(),
            identity_field_ids: HashMap::new(),
        }
    }

    fn index() -> FieldTextIndex {
        index_from(&[
            field(TITLE, "title", FieldType::string()),
            field(BODY, "body", FieldType::text()),
            field(COUNT, "count", FieldType::number()),
            field(TAGS_FIELD, "labels", FieldType::multiselect(["a", "b"])),
        ])
    }

    fn record(pairs: Vec<(&str, serde_json::Value)>) -> Record {
        let mut field_values = FieldValues::new();
        for (name, value) in pairs {
            field_values.insert(name, value);
        }
        Record {
            field_meta: None,
            instance_id: "r1".to_string(),
            type_id: "t1".to_string(),
            type_version: 1,
            type_namespace: "example".to_string(),
            type_name: "entry".to_string(),
            field_values,
            lifecycle_state: None,
            tags: None,
            created_at: None,
            updated_at: None,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn texts(segments: &[TextSegment]) -> Vec<&str> {
        segments.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn projects_searchable_value_types_and_skips_non_searchable() {
        let rec = record(vec![
            ("title", serde_json::json!("Adopt consent")),
            ("body", serde_json::json!("Use consent for changes")),
            ("count", serde_json::json!(42)),
        ]);
        let segments = project_text(&rec, &index());
        // Number field excluded; title + body present; label appended (= title).
        assert_eq!(
            texts(&segments),
            vec!["Adopt consent", "Use consent for changes", "Adopt consent"]
        );
        assert_eq!(segments.last().unwrap().field_id, LABEL_SENTINEL);
    }

    /// RFC-039 Change D: a list-cardinality field's value is a plain array;
    /// every string item projects (the successor of the retired `entries`).
    #[test]
    fn includes_list_cardinality_array_items() {
        let rec = record(vec![(
            "labels",
            serde_json::json!(["alpha", "beta", "gamma"]),
        )]);
        let segments = project_text(&rec, &index());
        assert!(texts(&segments).contains(&"alpha"));
        assert!(texts(&segments).contains(&"beta"));
        assert!(texts(&segments).contains(&"gamma"));
    }

    /// RFC-032 Rev 7 / RFC-039: composite (`ref`) fields are excluded from the
    /// text projection outright — no composite recursion is defined. The
    /// interior strings of an inline-composite value contribute nothing.
    #[test]
    fn composite_field_contributes_no_segments() {
        const ROWS: &str = "00000000-0000-4000-8000-00000000f007";
        let index = index_from(&[
            field(TITLE, "title", FieldType::string()),
            field(
                ROWS,
                "rows",
                FieldType::inline_ref(srs_core::types::field_type::ExactTypeRef {
                    type_id: "00000000-0000-4000-8000-0000000000bb".to_string(),
                    type_version: 1,
                })
                .into_list(),
            ),
        ]);
        let rec = record(vec![
            ("title", serde_json::json!("Root")),
            ("rows", serde_json::json!([{"cells": ["nested body"]}])),
        ]);
        let segments = project_text(&rec, &index);
        assert!(texts(&segments).contains(&"Root"));
        assert!(
            !texts(&segments).contains(&"nested body"),
            "composite interiors must not project: {:?}",
            texts(&segments)
        );
    }

    #[test]
    fn appends_label_and_tag_segments() {
        let mut rec = record(vec![("title", serde_json::json!("Heading"))]);
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
            ("body", serde_json::json!("b")),
            ("title", serde_json::json!("t")),
        ]);
        let a = project_text(&rec, &index());
        let b = project_text(&rec, &index());
        assert_eq!(a, b);
        // body precedes title (record order), then label.
        assert_eq!(a[0].field_name, "body");
        assert_eq!(a[1].field_name, "title");
    }

    /// I-120 / RFC-012 `[R8]` at its real consumption site.
    ///
    /// `rfc012_searchable_set_survives_the_rfc032_decomposition` below pins the
    /// legacy-eight parity, and passes under both the datatype-only reading and
    /// the RFC-032 Revision 7 allow-list — so it cannot show the allow-list is
    /// actually wired into the index. This drives `project_text` with
    /// `format: uuid` and `format: email` fields, which no first-party Tier-2
    /// record uses (srs-rust#790, CC-33), and asserts they contribute no
    /// `TextSegment`s while a plain string beside them still does.
    #[test]
    fn i120_r8_uuid_and_email_formats_contribute_no_text_segments() {
        const KEY: &str = "00000000-0000-4000-8000-00000000f005";
        const CONTACT: &str = "00000000-0000-4000-8000-00000000f006";

        let fields = [
            field(TITLE, "title", FieldType::string()),
            field(
                KEY,
                "key",
                FieldType::string().with_format(StringFormat::Uuid),
            ),
            field(
                CONTACT,
                "contact",
                FieldType::string().with_format(StringFormat::Email),
            ),
        ];
        // Derived exactly as `build_field_text_index` derives it.
        let index = index_from(&fields);

        let rec = record(vec![
            ("title", serde_json::json!("Adopt consent")),
            (
                "key",
                serde_json::json!("00000000-0000-4000-8000-000000000001"),
            ),
            ("contact", serde_json::json!("someone@example.test")),
        ]);
        let segments = project_text(&rec, &index);
        let projected = texts(&segments);
        assert!(
            projected.contains(&"Adopt consent"),
            "a plain string field must still project, got: {projected:?}"
        );
        assert!(
            !projected
                .iter()
                .any(|t| t.contains("00000000-0000-4000-8000-000000000001")),
            "a format: uuid field must contribute no segment, got: {projected:?}"
        );
        assert!(
            !projected.iter().any(|t| t.contains("someone@example.test")),
            "a format: email field must contribute no segment, got: {projected:?}"
        );
    }

    #[test]
    fn rfc012_searchable_set_survives_the_rfc032_decomposition() {
        // Guards the contract that migrating a pre-RFC-032 package does not
        // silently change which fields are searchable.
        for (legacy, searchable) in [
            (LegacyValueType::String, true),
            (LegacyValueType::Text, true),
            (LegacyValueType::Url, true),
            (LegacyValueType::Select, true),
            (LegacyValueType::Multiselect, true),
            (LegacyValueType::Number, false),
            (LegacyValueType::Boolean, false),
            (LegacyValueType::Date, false),
        ] {
            let ft = FieldType::from_legacy(legacy, &Default::default());
            assert_eq!(ft.is_text_searchable(), searchable, "{legacy:?}");
        }
    }

    fn note(title: Option<&str>, sections: Vec<(&str, &str)>, tags: Option<Vec<&str>>) -> Note {
        use srs_core::types::note::NoteSection;
        Note {
            instance_id: "n1".to_string(),
            title: title.map(str::to_string),
            tags: tags.map(|ts| ts.into_iter().map(str::to_string).collect()),
            sections: sections
                .into_iter()
                .map(|(name, content)| NoteSection {
                    name: name.to_string(),
                    label: None,
                    content: content.to_string(),
                    content_hint: None,
                    tags: None,
                })
                .collect(),
            graduated_at: None,
            source_refs: None,
            created_at: None,
            updated_at: None,
            meta: None,
        }
    }

    #[test]
    fn note_projection_orders_title_sections_then_tags() {
        let n = note(
            Some("Meeting capture"),
            vec![
                ("background", "Full-text search requires a portable floor."),
                ("findings", ""), // empty content — must not project
            ],
            Some(vec!["meeting", "search"]),
        );
        let segments = project_note_text(&n);
        assert_eq!(
            texts(&segments),
            vec![
                "Meeting capture",
                "Full-text search requires a portable floor.",
                "meeting",
                "search",
            ]
        );
        assert_eq!(segments[0].field_id, NOTE_TITLE_SENTINEL);
        assert_eq!(segments[1].field_id, NOTE_SECTION_SENTINEL);
        assert_eq!(segments[1].field_name, "background");
        assert_eq!(segments[2].field_id, TAG_SENTINEL);
    }

    #[test]
    fn note_projection_skips_missing_title_and_empty_sections() {
        let n = note(None, vec![("body", "")], None);
        assert!(project_note_text(&n).is_empty());
    }

    #[test]
    fn typed_record_projection_orders_title_fields_then_tags() {
        // RFC-039 [R8]: every TypedField carries an inline fieldType.
        let value = serde_json::json!({
            "instanceId": "tr1",
            "title": "Discovery Feature Planning Meeting",
            "fields": [
                { "name": "agenda", "fieldType": {"datatype": "string"}, "value": "Discuss text projection algorithm" },
                { "name": "attendee_count", "fieldType": {"datatype": "number"}, "value": 4 },
                { "name": "labels", "fieldType": {"datatype": "string", "valueDomain": "closed", "allowedValues": ["alpha", "beta"], "cardinality": "list"}, "value": ["alpha", "beta"] },
                { "name": "plain_note", "fieldType": {"datatype": "string"}, "value": "no valueType but string value" }
            ],
            "tags": ["searchable"]
        });
        let segments = project_typed_record_text(&value);
        assert_eq!(
            texts(&segments),
            vec![
                "Discovery Feature Planning Meeting",
                "Discuss text projection algorithm",
                "alpha",
                "beta",
                "no valueType but string value",
                "searchable",
            ]
        );
        assert_eq!(segments[0].field_id, TYPED_RECORD_TITLE_SENTINEL);
        assert_eq!(segments[1].field_id, TYPED_RECORD_FIELD_SENTINEL);
        assert_eq!(segments[1].field_name, "agenda");
        assert_eq!(segments.last().unwrap().field_id, TAG_SENTINEL);
    }

    #[test]
    fn typed_record_projection_skips_number_boolean_date_and_empty_values() {
        let value = serde_json::json!({
            "instanceId": "tr2",
            "fields": [
                { "name": "score", "fieldType": {"datatype": "number"}, "value": 3 },
                { "name": "done", "fieldType": {"datatype": "boolean"}, "value": true },
                { "name": "due", "fieldType": {"datatype": "date"}, "value": "2026-01-01" },
                { "name": "empty", "fieldType": {"datatype": "string"}, "value": "" },
                { "name": "no_value", "fieldType": {"datatype": "string"} }
            ]
        });
        assert!(project_typed_record_text(&value).is_empty());
    }
}
