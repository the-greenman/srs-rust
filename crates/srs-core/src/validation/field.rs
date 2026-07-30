use crate::types::field::{Field, ValueDomain};

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDiagnostic {
    pub code: FieldDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldDiagnosticCode {
    /// V3: a closed-domain field must declare exactly one of allowedValues or vocabularyRef
    V3BothBindings,
    V3NoBinding,
    /// RFC-032 conformance rules R1–R10 over the field's `fieldType`.
    FieldTypeNonConformant,
}

/// V3 + RFC-032 conformance: validate a Field's `fieldType`.
///
/// V3 originally said "a select/multiselect field must declare exactly one of
/// `allowedValues` or `vocabularyRef`". RFC-032 restates it as R3 over the
/// decomposed model — a `valueDomain: closed` field must draw from exactly one
/// source set — and the closed domain is now the successor of both
/// `select` and `multiselect`. The V3 codes are kept so existing consumers keep
/// resolving the same two conditions; every other RFC-032 rule surfaces under
/// [`FieldDiagnosticCode::FieldTypeNonConformant`].
pub fn validate_field_v3(field: &Field) -> Vec<FieldDiagnostic> {
    let mut diags = Vec::new();
    let ft = &field.field_type;

    if ft.effective_value_domain() == ValueDomain::Closed {
        let has_options = ft.allowed_values.as_ref().is_some_and(|v| !v.is_empty());
        let has_vocab_ref = ft.vocabulary_ref.as_ref().is_some_and(|s| !s.is_empty());
        match (has_options, has_vocab_ref) {
            (true, true) => diags.push(FieldDiagnostic {
                code: FieldDiagnosticCode::V3BothBindings,
                message: format!(
                    "field '{}' has both allowedValues and vocabularyRef — declare exactly one",
                    field.name
                ),
            }),
            (false, false) => diags.push(FieldDiagnostic {
                code: FieldDiagnosticCode::V3NoBinding,
                message: format!(
                    "closed-domain field '{}' must declare either allowedValues or vocabularyRef",
                    field.name
                ),
            }),
            _ => {}
        }
    }

    // The remaining RFC-032 rules (R2/R4/R5/R6/R9/R10). R3's closed-domain
    // source-set clause is already reported above under its V3 code, so it is
    // filtered out here rather than reported twice.
    for violation in ft.validate() {
        if violation.rule == "R3" && violation.message.contains("exactly one of") {
            continue;
        }
        diags.push(FieldDiagnostic {
            code: FieldDiagnosticCode::FieldTypeNonConformant,
            message: format!("field '{}': {violation}", field.name),
        });
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::field::{Datatype, FieldType};

    fn make_field(field_type: FieldType) -> Field {
        Field::new("f-1", "com.test", "test_field", field_type)
    }

    fn closed(allowed: bool, vocab_ref: bool, list: bool) -> Field {
        let mut ft = FieldType::string();
        ft.value_domain = Some(ValueDomain::Closed);
        if allowed {
            ft.allowed_values = Some(vec!["a".to_string()]);
        }
        if vocab_ref {
            ft.vocabulary_ref = Some("vocab-id@1".to_string());
        }
        if list {
            ft = ft.into_list();
        }
        make_field(ft)
    }

    #[test]
    fn closed_field_with_allowed_values_passes() {
        assert!(validate_field_v3(&closed(true, false, false)).is_empty());
    }

    #[test]
    fn closed_field_with_vocab_ref_passes() {
        assert!(validate_field_v3(&closed(false, true, false)).is_empty());
    }

    #[test]
    fn closed_field_both_bindings_is_error() {
        let diags = validate_field_v3(&closed(true, true, false));
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::V3BothBindings));
    }

    #[test]
    fn closed_field_no_binding_is_error() {
        let diags = validate_field_v3(&closed(false, false, false));
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::V3NoBinding));
    }

    #[test]
    fn closed_list_field_no_binding_is_error() {
        // The successor of the old `multiselect` case.
        let diags = validate_field_v3(&closed(false, false, true));
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::V3NoBinding));
    }

    #[test]
    fn open_string_field_without_binding_passes() {
        assert!(validate_field_v3(&make_field(FieldType::string())).is_empty());
    }

    #[test]
    fn a_no_binding_closed_field_reports_exactly_one_diagnostic() {
        // R3's source-set clause and V3NoBinding are the same defect — it must
        // not be reported twice under two codes.
        let diags = validate_field_v3(&closed(false, false, false));
        assert_eq!(diags.len(), 1, "{diags:?}");
    }

    #[test]
    fn other_rfc032_violations_surface_as_non_conformant() {
        // A `ref` with no rangeType (R2) has nothing to do with V3, but must
        // still be reported rather than silently accepted.
        let diags = validate_field_v3(&make_field(FieldType::new(Datatype::Ref)));
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::FieldTypeNonConformant
                && d.message.contains("[R2]")));
    }
}
