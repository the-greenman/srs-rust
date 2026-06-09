use crate::types::lifecycle::{Lifecycle, LifecycleState, LifecycleTransition};
use crate::types::term::VocabularyEntryStatus;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleDiagnostic {
    pub severity: LifecycleDiagnosticSeverity,
    pub code: LifecycleDiagnosticCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleDiagnosticSeverity {
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleDiagnosticCode {
    /// V9a: zero or multiple isInitial states
    V9NoInitialState,
    V9MultipleInitialStates,
    /// V9b: initial state is deprecated/tombstone/retired (must be active)
    V9InitialStateNotActive,
    /// V9c: transition from/to references unknown state key
    V9UnknownTransitionState,
    /// V9d: isFinal state appears as transition from
    V9FinalStateHasOutgoingTransition,
    /// V9e: duplicate transition ids
    V9DuplicateTransitionId,
    /// V5: duplicate state ids
    V5DuplicateStateId,
    /// Extension version mismatch
    ExtensionVersionMissing,
}

/// V9: Validate a Lifecycle for integrity invariants.
pub fn validate_lifecycle(lc: &Lifecycle) -> Vec<LifecycleDiagnostic> {
    let mut diags = Vec::new();

    // Extension field consistency
    match (
        lc.extends_lifecycle_id.as_ref(),
        lc.extends_lifecycle_version.as_ref(),
    ) {
        (Some(_), None) | (None, Some(_)) => {
            diags.push(LifecycleDiagnostic {
                severity: LifecycleDiagnosticSeverity::Error,
                code: LifecycleDiagnosticCode::ExtensionVersionMissing,
                message: "extendsLifecycleId and extendsLifecycleVersion must both be present"
                    .to_string(),
            });
        }
        _ => {}
    }

    // Effective states (non-retired)
    let effective_states: Vec<&LifecycleState> = lc
        .states
        .iter()
        .filter(|s| !matches!(s.status.as_ref(), Some(VocabularyEntryStatus::Retired)))
        .collect();

    // V9a: exactly one isInitial state
    let initial_count = effective_states
        .iter()
        .filter(|s| s.is_initial == Some(true))
        .count();
    match initial_count {
        0 => diags.push(LifecycleDiagnostic {
            severity: LifecycleDiagnosticSeverity::Error,
            code: LifecycleDiagnosticCode::V9NoInitialState,
            message: format!("lifecycle '{}' has no initial state", lc.name),
        }),
        1 => {}
        _ => diags.push(LifecycleDiagnostic {
            severity: LifecycleDiagnosticSeverity::Error,
            code: LifecycleDiagnosticCode::V9MultipleInitialStates,
            message: format!(
                "lifecycle '{}' has {} initial states, must have exactly 1",
                lc.name, initial_count
            ),
        }),
    }

    // V9b: initial state must be effectively active
    if let Some(initial) = effective_states.iter().find(|s| s.is_initial == Some(true)) {
        let is_active = matches!(
            initial.status.as_ref(),
            None | Some(VocabularyEntryStatus::Active)
        );
        if !is_active {
            diags.push(LifecycleDiagnostic {
                severity: LifecycleDiagnosticSeverity::Error,
                code: LifecycleDiagnosticCode::V9InitialStateNotActive,
                message: format!(
                    "lifecycle '{}' initial state '{}' must be active",
                    lc.name, initial.key
                ),
            });
        }
    }

    // Build effective state key set
    let state_keys: HashSet<&str> = effective_states.iter().map(|s| s.key.as_str()).collect();

    // V5: duplicate state ids
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for state in &effective_states {
        if let Some(ref id) = state.id {
            if !seen_ids.insert(id.as_str()) {
                diags.push(LifecycleDiagnostic {
                    severity: LifecycleDiagnosticSeverity::Error,
                    code: LifecycleDiagnosticCode::V5DuplicateStateId,
                    message: format!("lifecycle '{}' has duplicate state id '{}'", lc.name, id),
                });
            }
        }
    }

    // Build final state key set
    let final_keys: HashSet<&str> = effective_states
        .iter()
        .filter(|s| s.is_final == Some(true))
        .map(|s| s.key.as_str())
        .collect();

    // V9e: transition id uniqueness
    let mut seen_transition_ids: HashSet<&str> = HashSet::new();

    for t in &lc.transitions {
        // V9c: from/to must reference valid state keys
        if !state_keys.contains(t.from.as_str()) {
            diags.push(LifecycleDiagnostic {
                severity: LifecycleDiagnosticSeverity::Error,
                code: LifecycleDiagnosticCode::V9UnknownTransitionState,
                message: format!(
                    "lifecycle '{}' transition '{}' references unknown from-state '{}'",
                    lc.name, t.name, t.from
                ),
            });
        }
        if !state_keys.contains(t.to.as_str()) {
            diags.push(LifecycleDiagnostic {
                severity: LifecycleDiagnosticSeverity::Error,
                code: LifecycleDiagnosticCode::V9UnknownTransitionState,
                message: format!(
                    "lifecycle '{}' transition '{}' references unknown to-state '{}'",
                    lc.name, t.name, t.to
                ),
            });
        }

        // V9d: isFinal state must not be a from-state
        if final_keys.contains(t.from.as_str()) {
            diags.push(LifecycleDiagnostic {
                severity: LifecycleDiagnosticSeverity::Error,
                code: LifecycleDiagnosticCode::V9FinalStateHasOutgoingTransition,
                message: format!(
                    "lifecycle '{}' final state '{}' must not have outgoing transitions",
                    lc.name, t.from
                ),
            });
        }

        // V9e: transition id uniqueness
        if let Some(ref id) = t.id {
            if !seen_transition_ids.insert(id.as_str()) {
                diags.push(LifecycleDiagnostic {
                    severity: LifecycleDiagnosticSeverity::Error,
                    code: LifecycleDiagnosticCode::V9DuplicateTransitionId,
                    message: format!(
                        "lifecycle '{}' has duplicate transition id '{}'",
                        lc.name, id
                    ),
                });
            }
        }
    }

    diags
}

/// Validate a TypeLifecycle (inline lifecycle on a RecordType).
/// Uses the same V9 invariants as a standalone Lifecycle.
pub fn validate_type_lifecycle_v9(
    states: &[LifecycleState],
    transitions: &[LifecycleTransition],
    lifecycle_name: &str,
) -> Vec<LifecycleDiagnostic> {
    // Build a temporary Lifecycle to reuse validate_lifecycle logic
    let lc = Lifecycle {
        id: String::new(),
        version: 1,
        namespace: String::new(),
        name: lifecycle_name.to_string(),
        states: states.to_vec(),
        transitions: transitions.to_vec(),
        initial_state: String::new(),
        extends_lifecycle_id: None,
        extends_lifecycle_version: None,
        description: None,
        created_at: String::new(),
        extra: std::collections::HashMap::new(),
    };
    validate_lifecycle(&lc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::lifecycle::{Lifecycle, LifecycleState, LifecycleTransition};
    use crate::types::term::VocabularyEntryStatus;

    fn draft_state(initial: bool) -> LifecycleState {
        LifecycleState {
            id: Some("s-draft".to_string()),
            version: None,
            namespace: None,
            key: "draft".to_string(),
            label: None,
            description: None,
            aliases: None,
            is_initial: if initial { Some(true) } else { None },
            is_final: None,
            status: None,
            properties: None,
        }
    }

    fn active_state(is_final: bool) -> LifecycleState {
        LifecycleState {
            id: Some("s-active".to_string()),
            version: None,
            namespace: None,
            key: "active".to_string(),
            label: None,
            description: None,
            aliases: None,
            is_initial: None,
            is_final: if is_final { Some(true) } else { None },
            status: None,
            properties: None,
        }
    }

    fn promote_transition() -> LifecycleTransition {
        LifecycleTransition {
            id: Some("t-promote".to_string()),
            name: "promote".to_string(),
            from: "draft".to_string(),
            to: "active".to_string(),
            description: None,
            properties: None,
        }
    }

    fn make_lc(states: Vec<LifecycleState>, transitions: Vec<LifecycleTransition>) -> Lifecycle {
        Lifecycle {
            id: "lc-1".to_string(),
            version: 1,
            namespace: "com.test".to_string(),
            name: "test-lifecycle".to_string(),
            states,
            transitions,
            initial_state: "draft".to_string(),
            extends_lifecycle_id: None,
            extends_lifecycle_version: None,
            description: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn valid_lifecycle_no_errors() {
        let lc = make_lc(
            vec![draft_state(true), active_state(true)],
            vec![promote_transition()],
        );
        assert!(validate_lifecycle(&lc).is_empty());
    }

    #[test]
    fn lifecycle_zero_initial_is_error() {
        let lc = make_lc(vec![draft_state(false), active_state(false)], vec![]);
        let diags = validate_lifecycle(&lc);
        assert!(diags
            .iter()
            .any(|d| d.code == LifecycleDiagnosticCode::V9NoInitialState));
    }

    #[test]
    fn lifecycle_two_initial_is_error() {
        let mut s2 = active_state(false);
        s2.is_initial = Some(true);
        let lc = make_lc(vec![draft_state(true), s2], vec![]);
        let diags = validate_lifecycle(&lc);
        assert!(diags
            .iter()
            .any(|d| d.code == LifecycleDiagnosticCode::V9MultipleInitialStates));
    }

    #[test]
    fn lifecycle_deprecated_initial_is_error() {
        let mut s = draft_state(true);
        s.status = Some(VocabularyEntryStatus::Deprecated);
        let lc = make_lc(vec![s, active_state(false)], vec![]);
        let diags = validate_lifecycle(&lc);
        assert!(diags
            .iter()
            .any(|d| d.code == LifecycleDiagnosticCode::V9InitialStateNotActive));
    }

    #[test]
    fn lifecycle_final_state_as_source_is_error() {
        // active is final, but transition goes from active to draft
        let bad_transition = LifecycleTransition {
            id: Some("t-bad".to_string()),
            name: "demote".to_string(),
            from: "active".to_string(),
            to: "draft".to_string(),
            description: None,
            properties: None,
        };
        let lc = make_lc(
            vec![draft_state(true), active_state(true)],
            vec![bad_transition],
        );
        let diags = validate_lifecycle(&lc);
        assert!(diags
            .iter()
            .any(|d| d.code == LifecycleDiagnosticCode::V9FinalStateHasOutgoingTransition));
    }

    #[test]
    fn lifecycle_unknown_transition_state_is_error() {
        let bad_t = LifecycleTransition {
            id: None,
            name: "bad".to_string(),
            from: "draft".to_string(),
            to: "nonexistent".to_string(),
            description: None,
            properties: None,
        };
        let lc = make_lc(vec![draft_state(true), active_state(false)], vec![bad_t]);
        let diags = validate_lifecycle(&lc);
        assert!(diags
            .iter()
            .any(|d| d.code == LifecycleDiagnosticCode::V9UnknownTransitionState));
    }

    #[test]
    fn lifecycle_duplicate_transition_id_is_error() {
        let t1 = promote_transition();
        let mut t2 = promote_transition();
        t2.to = "draft".to_string();
        // t2's from is "draft" and to is "draft" — same id as t1
        let lc = make_lc(vec![draft_state(true), active_state(false)], vec![t1, t2]);
        let diags = validate_lifecycle(&lc);
        assert!(diags
            .iter()
            .any(|d| d.code == LifecycleDiagnosticCode::V9DuplicateTransitionId));
    }
}
