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
/// with [`build_field_text_index`] and reused across every record. All maps are
/// prebuilt so projecting a record allocates nothing here.
pub struct FieldTextIndex {
    /// `field_id → field_name`, also the map [`record_label::record_display_label`] expects.
    names: HashMap<String, String>,
    /// Field ids whose `fieldType` projects searchable text.
    searchable: HashSet<String>,
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

    fn name_of(&self, field_id: &str) -> Option<&str> {
        self.names.get(field_id).map(String::as_str)
    }

    fn is_searchable(&self, field_id: &str) -> bool {
        self.searchable.contains(field_id)
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
    let mut searchable = HashSet::new();
    for f in &package.fields {
        // I-120 / RFC-012 `[R8]`: `datatype == string` **and** an allow-listed
        // `format`. Not datatype alone — RFC-032 Revision 7 excludes the
        // string-datatyped `uuid` and `email` formats, so a field can be
        // `datatype: string` and still contribute no `TextSegment`s. See
        // `FieldType::is_text_searchable` for why the datatype-only reading is
        // sound over the legacy eight and unsound over the model as a whole.
        if f.field_type.is_text_searchable() {
            searchable.insert(f.id.clone());
        }
        names.insert(f.id.clone(), f.name.clone());
    }
    let identity_field_ids = record_label::identity_field_index_from_package(&package);
    Ok(FieldTextIndex {
        names,
        searchable,
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
    use srs_core::types::field::{AiGuidance, Field, FieldType, LegacyValueType, StringFormat};
    use srs_core::types::record::{FieldGroupEntry, FieldGroupValue, FieldValueEntry};
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
            ai_guidance: AiGuidance::default(),
            field_type: vt.clone(),
            default_value: None,
            editor_hint: None,
            tags: None,
            lineage: None,
            provenance: None,
            deprecated_at: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn index() -> FieldTextIndex {
        // Built from real Fields so searchability is *derived* from `fieldType`
        // exactly as `build_field_text_index` derives it, rather than asserted
        // by a hand-maintained boolean that could drift from the projection.
        let fields = [
            field(TITLE, "title", FieldType::string()),
            field(BODY, "body", FieldType::text()),
            field(COUNT, "count", FieldType::number()),
            field(TAGS_FIELD, "labels", FieldType::multiselect(["a", "b"])),
        ];
        let names = fields
            .iter()
            .map(|f| (f.id.clone(), f.name.clone()))
            .collect();
        let searchable = fields
            .iter()
            .filter(|f| f.field_type.is_text_searchable())
            .map(|f| f.id.clone())
            .collect();
        FieldTextIndex {
            names,
            searchable,
            identity_field_ids: HashMap::new(),
        }
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
        let index = FieldTextIndex {
            names: fields
                .iter()
                .map(|f| (f.id.clone(), f.name.clone()))
                .collect(),
            searchable: fields
                .iter()
                .filter(|f| f.field_type.is_text_searchable())
                .map(|f| f.id.clone())
                .collect(),
            identity_field_ids: HashMap::new(),
        };

        let rec = record(vec![
            fv(TITLE, serde_json::json!("Adopt consent")),
            fv(
                KEY,
                serde_json::json!("00000000-0000-4000-8000-000000000001"),
            ),
            fv(CONTACT, serde_json::json!("someone@example.test")),
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
}
