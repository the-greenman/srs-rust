use crate::types::protocol::{
    Protocol, ProtocolDiagnostic, ProtocolDiagnosticSeverity, ProtocolStage,
    ProtocolValidationResult,
};
use std::collections::{HashMap, HashSet};

/// Validate a protocol definition
/// Checks:
/// - No self-dependency in stages
/// - No cycles in dependsOn
/// - Order is consistent with dependsOn partial order
/// - All dependsOn stageIds exist
pub fn validate_protocol(protocol: &Protocol) -> ProtocolValidationResult {
    let mut diagnostics = vec![];

    // Build stage lookup
    let stage_map: HashMap<String, &ProtocolStage> = protocol
        .protocol_stages
        .iter()
        .map(|s| (s.stage_id.clone(), s))
        .collect();

    // Check all dependsOn references exist
    for stage in &protocol.protocol_stages {
        for dep_id in &stage.depends_on {
            if !stage_map.contains_key(dep_id) {
                diagnostics.push(ProtocolDiagnostic {
                    message: format!(
                        "Stage '{}' depends on non-existent stage '{}'",
                        stage.stage_id, dep_id
                    ),
                    severity: ProtocolDiagnosticSeverity::Error,
                });
            }
        }
    }

    // Check self-dependency
    for stage in &protocol.protocol_stages {
        if stage.depends_on.contains(&stage.stage_id) {
            diagnostics.push(ProtocolDiagnostic {
                message: format!("Stage '{}' has self-dependency", stage.stage_id),
                severity: ProtocolDiagnosticSeverity::Error,
            });
        }
    }

    // Check for cycles using DFS
    if let Some(cycle) = detect_cycle(&protocol.protocol_stages) {
        diagnostics.push(ProtocolDiagnostic {
            message: format!("Dependency cycle detected: {}", cycle.join(" -> ")),
            severity: ProtocolDiagnosticSeverity::Error,
        });
    }

    // Check order consistency with dependsOn partial order
    // If A depends on B, then order(A) should be > order(B)
    for stage in &protocol.protocol_stages {
        for dep_id in &stage.depends_on {
            if let Some(dep_stage) = stage_map.get(dep_id) {
                if stage.order <= dep_stage.order {
                    diagnostics.push(ProtocolDiagnostic {
                        message: format!(
                            "Stage '{}' has order {} but depends on '{}' with order {} - order must be greater than dependencies",
                            stage.stage_id, stage.order, dep_id, dep_stage.order
                        ),
                        severity: ProtocolDiagnosticSeverity::Error,
                    });
                }
            }
        }
    }

    if diagnostics.is_empty() {
        ProtocolValidationResult::ok()
    } else {
        ProtocolValidationResult {
            valid: false,
            diagnostics,
        }
    }
}

/// Detect cycles in stage dependencies using DFS
fn detect_cycle(stages: &[ProtocolStage]) -> Option<Vec<String>> {
    let stage_map: HashMap<String, &ProtocolStage> =
        stages.iter().map(|s| (s.stage_id.clone(), s)).collect();

    let mut visited: HashSet<String> = HashSet::new();
    let mut rec_stack: HashSet<String> = HashSet::new();
    let mut path: Vec<String> = vec![];

    for stage in stages {
        if !visited.contains(&stage.stage_id) {
            if let Some(cycle) = dfs_detect_cycle(
                &stage.stage_id,
                &stage_map,
                &mut visited,
                &mut rec_stack,
                &mut path,
            ) {
                return Some(cycle);
            }
        }
    }

    None
}

fn dfs_detect_cycle(
    node: &str,
    stage_map: &HashMap<String, &ProtocolStage>,
    visited: &mut HashSet<String>,
    rec_stack: &mut HashSet<String>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    visited.insert(node.to_string());
    rec_stack.insert(node.to_string());
    path.push(node.to_string());

    if let Some(stage) = stage_map.get(node) {
        for dep_id in &stage.depends_on {
            if !visited.contains(dep_id) {
                if let Some(cycle) = dfs_detect_cycle(dep_id, stage_map, visited, rec_stack, path) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(dep_id) {
                // Found cycle - extract cycle from path
                let cycle_start = path.iter().position(|p| p == dep_id).unwrap();
                let mut cycle = path[cycle_start..].to_vec();
                cycle.push(dep_id.clone());
                return Some(cycle);
            }
        }
    }

    path.pop();
    rec_stack.remove(node);
    None
}

/// Validate a single protocol stage
pub fn validate_protocol_stage(stage: &ProtocolStage) -> Vec<String> {
    let mut errors = vec![];

    if stage.stage_id.trim().is_empty() {
        errors.push("Stage ID cannot be empty".to_string());
    }

    if stage.name.trim().is_empty() {
        errors.push(format!("Stage '{}' name cannot be empty", stage.stage_id));
    }

    if stage.order < 0 {
        errors.push(format!(
            "Stage '{}' order must be non-negative",
            stage.stage_id
        ));
    }

    errors
}
