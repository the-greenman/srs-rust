use crate::types::field::{Field, ValueType};

#[derive(Debug, Clone, PartialEq)]
pub struct FieldDiagnostic {
    pub code: FieldDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldDiagnosticCode {
    /// V3: select/multiselect field must declare exactly one of selectOptions or vocabularyRef
    V3BothBindings,
    V3NoBinding,
}

/// V3: Validate select/multiselect field binding exclusivity.
///
/// A select or multiselect field must declare exactly one of:
/// - `allowedValues` (inline anonymous vocabulary) OR
/// - `vocabularyRef` (reference to a named Vocabulary)
///
/// Both present or neither present is an error.
pub fn validate_field_v3(field: &Field) -> Vec<FieldDiagnostic> {
    let mut diags = Vec::new();

    if !matches!(field.value_type, ValueType::Select | ValueType::Multiselect) {
        return diags;
    }

    let has_options = field.allowed_values.is_some();
    let has_vocab_ref = field.vocabulary_ref.is_some();

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
                "select/multiselect field '{}' must declare either allowedValues or vocabularyRef",
                field.name
            ),
        }),
        _ => {}
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_field(value_type: ValueType, allowed: bool, vocab_ref: bool) -> Field {
        Field {
            id: "f-1".to_string(),
            namespace: "com.test".to_string(),
            name: "test-field".to_string(),
            version: 1,
            description: "test".to_string(),
            ai_guidance: serde_json::Value::Null,
            value_type,
            allowed_values: if allowed {
                Some(vec!["a".to_string()])
            } else {
                None
            },
            vocabulary_ref: if vocab_ref {
                Some("vocab-id@1".to_string())
            } else {
                None
            },
            default_value: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: Default::default(),
        }
    }

    #[test]
    fn select_field_with_allowed_values_passes() {
        let f = make_field(ValueType::Select, true, false);
        assert!(validate_field_v3(&f).is_empty());
    }

    #[test]
    fn select_field_with_vocab_ref_passes() {
        let f = make_field(ValueType::Select, false, true);
        assert!(validate_field_v3(&f).is_empty());
    }

    #[test]
    fn select_field_both_bindings_is_error() {
        let f = make_field(ValueType::Select, true, true);
        let diags = validate_field_v3(&f);
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::V3BothBindings));
    }

    #[test]
    fn select_field_no_binding_is_error() {
        let f = make_field(ValueType::Select, false, false);
        let diags = validate_field_v3(&f);
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::V3NoBinding));
    }

    #[test]
    fn multiselect_field_no_binding_is_error() {
        let f = make_field(ValueType::Multiselect, false, false);
        let diags = validate_field_v3(&f);
        assert!(diags
            .iter()
            .any(|d| d.code == FieldDiagnosticCode::V3NoBinding));
    }

    #[test]
    fn string_field_without_binding_passes() {
        let f = make_field(ValueType::String, false, false);
        assert!(validate_field_v3(&f).is_empty());
    }
}
