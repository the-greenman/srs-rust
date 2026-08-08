//! RFC-039 [R7]/[R15] — rejection of removed constructs at `dataModelRevision >= 2`.
//!
//! Definition files carry no document-local revision discriminator ([R9]'s
//! structural test covers instances only), so revision MUST be resolved from
//! the enclosing repository or package manifest **before** these checks run —
//! the caller (the repository loader) plumbs it in.

use crate::error::CoreError;

/// The two extensions [R15] retires. A revision ≥ 2 manifest declaring either
/// is an error, not ignored — a declaration implies constructs [R7] rejects.
const RETIRED_EXTENSIONS: [&str; 2] = ["ext:field-groups", "ext:repeatable-fields"];

/// [R15]: reject retired extension declarations at revision ≥ 2.
pub fn check_declared_extensions(declared: &[String], revision: u32) -> Vec<CoreError> {
    if revision < 2 {
        return Vec::new();
    }
    declared
        .iter()
        .filter(|e| RETIRED_EXTENSIONS.contains(&e.as_str()))
        .map(|e| CoreError::RetiredExtensionDeclared {
            extension: e.clone(),
        })
        .collect()
}

/// [R7]: reject removed constructs in a raw **Type definition** document at
/// revision ≥ 2 — `fieldGroups`, and the `repeatable`/`minItems`/`maxItems`
/// trio on any `fields[]` assignment. (`fieldType.minItems`/`maxItems` on a
/// Field definition are RFC-032 facets and remain legal — this check reads
/// assignment level only, which is why it operates on the raw JSON rather
/// than the typed struct, whose serde would silently drop the keys.)
pub fn check_type_document(
    doc: &serde_json::Value,
    revision: u32,
    location: &str,
) -> Vec<CoreError> {
    if revision < 2 {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    if doc.get("fieldGroups").is_some() {
        diagnostics.push(CoreError::RemovedConstruct {
            construct: "fieldGroups".to_string(),
            location: location.to_string(),
        });
    }
    if let Some(fields) = doc.get("fields").and_then(|f| f.as_array()) {
        for (i, assignment) in fields.iter().enumerate() {
            for key in ["repeatable", "minItems", "maxItems"] {
                if assignment.get(key).is_some() {
                    diagnostics.push(CoreError::RemovedConstruct {
                        construct: format!("FieldAssignment.{key}"),
                        location: format!("{location} fields[{i}]"),
                    });
                }
            }
        }
    }
    diagnostics
}

/// [R7]: reject removed constructs in a raw **Record instance** document at
/// revision ≥ 2 — `groupValues` (an array `fieldValues` is [R9]'s structural
/// test, reported as `UnsupportedGeneration` by the deserializer, not here).
pub fn check_record_document(
    doc: &serde_json::Value,
    revision: u32,
    location: &str,
) -> Vec<CoreError> {
    if revision < 2 {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    if doc.get("groupValues").is_some() {
        diagnostics.push(CoreError::RemovedConstruct {
            construct: "groupValues".to_string(),
            location: location.to_string(),
        });
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rev2_manifest_declaring_retired_extension_rejected() {
        let declared = vec!["ext:lifecycle".to_string(), "ext:field-groups".to_string()];
        let diags = check_declared_extensions(&declared, 2);
        assert!(
            matches!(&diags[..], [CoreError::RetiredExtensionDeclared { extension }]
                if extension == "ext:field-groups")
        );
        // Same declaration at revision ≤ 1 is legal.
        assert!(check_declared_extensions(&declared, 1).is_empty());
    }

    #[test]
    fn rev2_type_with_field_groups_or_trio_rejected() {
        let doc = json!({
            "fieldGroups": [],
            "fields": [
                {"fieldId": "f1", "order": 0, "required": true, "repeatable": true},
                {"fieldId": "f2", "order": 1, "required": false}
            ]
        });
        let diags = check_type_document(&doc, 2, "types/t.json");
        assert_eq!(diags.len(), 2, "{diags:?}");
        assert!(check_type_document(&doc, 1, "types/t.json").is_empty());
    }

    #[test]
    fn field_type_min_items_not_flagged() {
        // fieldType.minItems is an RFC-032 facet, not the assignment trio.
        let doc = json!({
            "fields": [{"fieldId": "f1", "order": 0, "required": true}],
            "fieldType": {"datatype": "string", "minItems": 1}
        });
        assert!(check_type_document(&doc, 2, "x").is_empty());
    }

    #[test]
    fn rev2_record_with_group_values_rejected() {
        let doc = json!({"groupValues": []});
        assert_eq!(check_record_document(&doc, 2, "records/r.json").len(), 1);
        assert!(check_record_document(&doc, 1, "records/r.json").is_empty());
    }
}
