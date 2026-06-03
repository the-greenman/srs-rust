use crate::types::record_type::RecordType;

#[derive(Debug, Clone, PartialEq)]
pub struct RecordTypeDiagnostic {
    pub code: RecordTypeDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecordTypeDiagnosticCode {
    /// V7: Type declares both lifecycle and lifecycleRef — mutually exclusive
    V7BothLifecycleAndRef,
}

/// V7: Validate lifecycle/lifecycleRef mutual exclusivity on a RecordType.
pub fn validate_record_type_v7(rt: &RecordType) -> Vec<RecordTypeDiagnostic> {
    let mut diags = Vec::new();

    if rt.lifecycle.is_some() && rt.lifecycle_ref.is_some() {
        diags.push(RecordTypeDiagnostic {
            code: RecordTypeDiagnosticCode::V7BothLifecycleAndRef,
            message: format!(
                "type '{}' declares both lifecycle and lifecycleRef — declare exactly one",
                rt.name
            ),
        });
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::record_type::{RecordType, TypeLifecycle};
    use std::collections::HashMap;

    fn make_rt(lifecycle: bool, lifecycle_ref: bool) -> RecordType {
        RecordType {
            id: "rt-1".to_string(),
            namespace: "com.test".to_string(),
            name: "test-type".to_string(),
            version: 1,
            description: "test".to_string(),
            fields: vec![],
            field_groups: None,
            extends_type_id: None,
            extends_type_version: None,
            field_order: None,
            field_assignment_overrides: None,
            lifecycle: if lifecycle {
                Some(TypeLifecycle {
                    states: vec![],
                    transitions: vec![],
                    initial_state: "draft".to_string(),
                })
            } else {
                None
            },
            lifecycle_ref: if lifecycle_ref {
                Some("lc-ref-id".to_string())
            } else {
                None
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: HashMap::new(),
        }
    }

    #[test]
    fn record_type_both_lifecycle_and_ref_is_error() {
        let rt = make_rt(true, true);
        let diags = validate_record_type_v7(&rt);
        assert!(diags
            .iter()
            .any(|d| d.code == RecordTypeDiagnosticCode::V7BothLifecycleAndRef));
    }

    #[test]
    fn record_type_only_inline_passes() {
        let rt = make_rt(true, false);
        assert!(validate_record_type_v7(&rt).is_empty());
    }

    #[test]
    fn record_type_only_ref_passes() {
        let rt = make_rt(false, true);
        assert!(validate_record_type_v7(&rt).is_empty());
    }

    #[test]
    fn record_type_neither_passes() {
        let rt = make_rt(false, false);
        assert!(validate_record_type_v7(&rt).is_empty());
    }
}
