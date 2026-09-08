//! Schema ↔ Rust struct property-parity checks (srs-rust#777, "the third seam").
//!
//! Three model-definition seams exist in this ecosystem. Seam 1 (spec prose ↔
//! JSON Schema, within `srs`) is closed by RFC-031's
//! `check-idl-schema-conformance.mjs`. Seam 2 (schema *file* mirror fidelity,
//! `docs/schema/2.0/` vs this crate's copy) is closed by
//! `scripts/check-schema-drift.sh`. Neither checks whether `srs-core`'s Rust
//! structs and their serde shapes actually implement what the mirrored schema
//! declares — that gap is what let srs-rust#769 (`Field::id` defaulting to
//! `""` instead of being required; several properties left untyped) go
//! unnoticed. This module closes seam 3.
//!
//! ## Design decision: compiler-enforced property lists, not reflection
//!
//! `srs-core` forbids `schemars` (CLAUDE.md: "No `schemars`" is a hard
//! constraint), so a derive-macro-based structural comparison is off the
//! table. Full `syn`-based source reflection was also considered and
//! rejected: it would need a new build-time dependency and a parser for
//! serde's rename/skip/default attributes, essentially re-implementing serde
//! itself, for a benefit only test coverage (below) doesn't already give
//! more cheaply.
//!
//! Instead, each entity's test lists its schema-facing property names once,
//! split into `required`/`optional`, and — critically — pairs that list with
//! an **exhaustive destructure** of the struct (`let Field { a, b, .. } =
//! ...` with no `..`). Rust's own exhaustiveness check then fails to compile
//! the moment a field is added to or removed from the struct without the
//! test being updated, which is what makes this a structural check rather
//! than a hand-maintained list that silently drifts — the exact failure mode
//! RFC-031's own `MAPPING` table hit before those rows retired in favour of
//! generated artifacts (this seam has no such generator to fall back on: the
//! Rust structs are hand-authored, so this is a genuine, permanent gap to
//! guard, not a comparison of two projections of the same source).
//!
//! [`assert_property_parity`] takes that struct-derived list and diffs it
//! against the mirrored schema's own `required`/`properties` at the given
//! JSON Pointer, catching both directions of drift: a schema property with
//! no struct counterpart, a struct property the schema doesn't declare, and
//! a property whose required-ness disagrees between the two.

use crate::schema_source;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;

/// The result of comparing a struct's declared property names against a
/// mirrored JSON Schema's `properties`/`required` at some location.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PropertyParityReport {
    /// Location compared: `"<root>"` or a `$defs` key.
    pub location: String,
    /// Declared by the schema, but not accounted for by the struct's
    /// required+optional lists (a struct property is missing entirely).
    pub missing_from_struct: Vec<String>,
    /// Accounted for by the struct's lists, but not a schema property at all
    /// (the struct claims something the schema doesn't declare).
    pub missing_from_schema: Vec<String>,
    /// Present on both sides, but required on one and optional on the other.
    pub required_mismatches: Vec<String>,
}

impl PropertyParityReport {
    fn is_clean(&self) -> bool {
        self.missing_from_struct.is_empty()
            && self.missing_from_schema.is_empty()
            && self.required_mismatches.is_empty()
    }
}

impl fmt::Display for PropertyParityReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "property parity mismatch at {}", self.location)?;
        if !self.missing_from_struct.is_empty() {
            write!(
                f,
                "; schema declares {:?} but the struct's required/optional lists omit them",
                self.missing_from_struct
            )?;
        }
        if !self.missing_from_schema.is_empty() {
            write!(
                f,
                "; struct lists {:?} but the schema does not declare them",
                self.missing_from_schema
            )?;
        }
        if !self.required_mismatches.is_empty() {
            write!(
                f,
                "; required-ness disagrees for {:?}",
                self.required_mismatches
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for PropertyParityReport {}

/// Resolve the `{"properties": ..., "required": [...]}` object at `location`
/// within a parsed schema document: `None` for the schema root, or
/// `Some(key)` for `$defs.<key>`.
fn resolve_object<'a>(schema: &'a Value, location: Option<&str>) -> &'a Value {
    match location {
        None => schema,
        Some(key) => schema
            .get("$defs")
            .and_then(|defs| defs.get(key))
            .unwrap_or_else(|| panic!("no $defs entry named {key:?} in schema")),
    }
}

/// Compare a struct's schema-facing property names against the mirrored
/// schema's declared `properties`/`required` at `location` (`None` for the
/// schema root, `Some("$defs key")` for a nested definition).
///
/// `required_in_struct` and `optional_in_struct` together must name every
/// property the struct round-trips through serde, using the schema's
/// (camelCase, or `$schema`) property names — see the module docs for why
/// pairing this call with an exhaustive struct destructure is what keeps the
/// two lists honest.
pub fn assert_property_parity(
    schema_id: &str,
    location: Option<&str>,
    required_in_struct: &[&str],
    optional_in_struct: &[&str],
) -> Result<(), PropertyParityReport> {
    let src = schema_source(schema_id)
        .unwrap_or_else(|| panic!("srs-schema: unknown schema id {schema_id:?}"));
    let schema: Value = serde_json::from_str(src)
        .unwrap_or_else(|e| panic!("srs-schema: failed to parse {schema_id}: {e}"));
    let target = resolve_object(&schema, location);

    let schema_props: BTreeSet<String> = target
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();
    let schema_required: BTreeSet<String> = target
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let struct_required: BTreeSet<String> =
        required_in_struct.iter().map(|s| s.to_string()).collect();
    let struct_optional: BTreeSet<String> =
        optional_in_struct.iter().map(|s| s.to_string()).collect();
    let struct_all: BTreeSet<String> = struct_required.union(&struct_optional).cloned().collect();

    assert!(
        struct_required.is_disjoint(&struct_optional),
        "a property cannot be listed as both required and optional: {:?}",
        struct_required
            .intersection(&struct_optional)
            .collect::<Vec<_>>()
    );

    let report = PropertyParityReport {
        location: location.unwrap_or("<root>").to_string(),
        missing_from_struct: schema_props.difference(&struct_all).cloned().collect(),
        missing_from_schema: struct_all.difference(&schema_props).cloned().collect(),
        required_mismatches: schema_required
            .symmetric_difference(&struct_required)
            .filter(|p| schema_props.contains(*p) || struct_all.contains(*p))
            .cloned()
            .collect(),
    };

    if report.is_clean() {
        Ok(())
    } else {
        Err(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_schema_matches_its_own_declared_shape() {
        assert_property_parity(
            crate::FIELD_SCHEMA_ID,
            None,
            &[
                "id",
                "namespace",
                "name",
                "version",
                "description",
                "aiGuidance",
                "fieldType",
                "createdAt",
            ],
            &[
                "$schema",
                "instructions",
                "editorHint",
                "tags",
                "lineage",
                "provenance",
            ],
        )
        .unwrap();
    }

    #[test]
    fn missing_required_property_is_reported() {
        let report = assert_property_parity(
            crate::FIELD_SCHEMA_ID,
            None,
            &["namespace", "name", "version", "description", "createdAt"],
            &[
                "$schema",
                "id",
                "aiGuidance",
                "fieldType",
                "instructions",
                "editorHint",
                "tags",
                "lineage",
                "provenance",
            ],
        )
        .unwrap_err();
        assert!(
            report.required_mismatches.contains(&"id".to_string())
                || report
                    .required_mismatches
                    .contains(&"aiGuidance".to_string())
        );
    }

    #[test]
    fn unknown_struct_property_is_reported() {
        let report = assert_property_parity(
            crate::FIELD_SCHEMA_ID,
            None,
            &[
                "id",
                "namespace",
                "name",
                "version",
                "description",
                "aiGuidance",
                "fieldType",
                "createdAt",
            ],
            &[
                "$schema",
                "instructions",
                "editorHint",
                "tags",
                "lineage",
                "provenance",
                "notARealProperty",
            ],
        )
        .unwrap_err();
        assert_eq!(
            report.missing_from_schema,
            vec!["notARealProperty".to_string()]
        );
    }

    #[test]
    fn schema_property_absent_from_struct_lists_is_reported() {
        let report = assert_property_parity(
            crate::FIELD_SCHEMA_ID,
            None,
            &[
                "id",
                "namespace",
                "name",
                "version",
                "description",
                "aiGuidance",
                "createdAt",
                // "fieldType" deliberately dropped, simulating a struct that
                // never gained the property a schema change added.
            ],
            &[
                "$schema",
                "instructions",
                "editorHint",
                "tags",
                "lineage",
                "provenance",
            ],
        )
        .unwrap_err();
        assert_eq!(report.missing_from_struct, vec!["fieldType".to_string()]);
    }

    /// srs-rust#769: before the fix, `Field::id` had `#[serde(default)]` and
    /// defaulted to `""` instead of being a mandatory, non-defaulted
    /// property — i.e. the struct treated a schema-required property as
    /// effectively optional. This regression test proves this checker would
    /// have caught that shape: declaring `id` in `optional_in_struct`
    /// instead of `required_in_struct` (mirroring the pre-fix struct) must
    /// be flagged against `field.json`, which requires it.
    #[test]
    fn would_have_caught_srs_rust_769_id_treated_as_optional() {
        let report = assert_property_parity(
            crate::FIELD_SCHEMA_ID,
            None,
            &[
                "namespace",
                "name",
                "version",
                "description",
                "aiGuidance",
                "fieldType",
                "createdAt",
            ],
            &[
                "$schema",
                "id", // wrongly declared optional, mirroring the pre-#769 struct
                "instructions",
                "editorHint",
                "tags",
                "lineage",
                "provenance",
            ],
        )
        .unwrap_err();
        assert!(
            report.required_mismatches.contains(&"id".to_string()),
            "expected `id` to be flagged as a required-ness mismatch, got: {report}"
        );
    }

    #[test]
    fn nested_def_location_is_checked() {
        assert_property_parity(
            crate::FIELD_SCHEMA_ID,
            Some("com.semanticops.srs__ai-guidance__v1"),
            &["purpose"],
            &["extraction", "negativeGuidance", "examples"],
        )
        .unwrap();
    }

    #[test]
    fn unknown_def_location_panics() {
        let result = std::panic::catch_unwind(|| {
            let _ =
                assert_property_parity(crate::FIELD_SCHEMA_ID, Some("does-not-exist"), &[], &[]);
        });
        assert!(result.is_err());
    }
}
