use crate::error::RepositoryError;
use crate::store::RepositoryStore;
use crate::writer::new_instance_id;
use srs_core::types::address::AttentionState;
use srs_core::types::protocol::{
    ProtocolRun, ProtocolRunStatus, ProtocolRunsCollection, StageState, StageStatus,
};
use std::path::PathBuf;

const RUNS_PATH: &str = "runs/runs.json";

// --- Input / result types ---

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunInput {
    pub protocol_id: String,
    pub protocol_version: i32,
    pub container_id: String,
    pub target_record_id: Option<String>,
    pub initial_stage_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvanceStageInput {
    pub run_id: String,
    pub stage_id: String,
    /// If true, mark the current Active stage as Completed before appending the new one.
    pub complete_current: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListFilter {
    pub protocol_id: Option<String>,
    pub container_id: Option<String>,
    /// "Active" | "Completed" | "Abandoned"
    pub status: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub run_id: String,
    pub protocol_id: String,
    pub container_id: String,
    pub status: String,
    pub current_stage_id: Option<String>,
    pub started_at: String,
}

#[derive(Debug)]
pub enum GetRunResult {
    Found(Box<ProtocolRun>),
    NotFound,
}

#[derive(Debug)]
pub struct RunResult {
    pub run: ProtocolRun,
}

// --- Private helpers ---

fn load_runs(store: &dyn RepositoryStore) -> Result<ProtocolRunsCollection, RepositoryError> {
    match store.load_instance_json(RUNS_PATH) {
        Ok(v) => Ok(serde_json::from_value(v).map_err(|e| RepositoryError::Serialize {
            path: PathBuf::from(RUNS_PATH),
            source: e,
        })?),
        Err(RepositoryError::NotFound { .. }) => Ok(ProtocolRunsCollection { runs: vec![] }),
        Err(RepositoryError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(ProtocolRunsCollection { runs: vec![] })
        }
        Err(e) => Err(e),
    }
}

fn save_runs(
    store: &dyn RepositoryStore,
    collection: &ProtocolRunsCollection,
) -> Result<(), RepositoryError> {
    let value = serde_json::to_value(collection).map_err(|e| RepositoryError::Serialize {
        path: PathBuf::from(RUNS_PATH),
        source: e,
    })?;
    store.save_instance_json(RUNS_PATH, &value)
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn run_not_found(run_id: &str) -> RepositoryError {
    RepositoryError::NotFound {
        path: PathBuf::from(format!("runs/{}", run_id)),
    }
}

fn status_to_string(status: &ProtocolRunStatus) -> String {
    match status {
        ProtocolRunStatus::Active => "Active".to_string(),
        ProtocolRunStatus::Completed => "Completed".to_string(),
        ProtocolRunStatus::Abandoned => "Abandoned".to_string(),
    }
}

fn run_to_summary(run: &ProtocolRun) -> RunSummary {
    RunSummary {
        run_id: run.run_id.clone(),
        protocol_id: run.protocol_id.clone(),
        container_id: run.container_id.clone(),
        status: status_to_string(&run.status),
        current_stage_id: run.attention_state.stage_id.clone(),
        started_at: run.started_at.clone(),
    }
}

// --- Public service functions ---

/// Create a new protocol run. Returns the created run.
pub fn create_run(
    store: &dyn RepositoryStore,
    input: CreateRunInput,
) -> Result<RunResult, RepositoryError> {
    let run_id = new_instance_id();
    let started_at = now_iso8601();

    let stage_states = if let Some(ref sid) = input.initial_stage_id {
        vec![StageState {
            stage_id: sid.clone(),
            status: StageStatus::Active,
            started_at: Some(started_at.clone()),
            completed_at: None,
        }]
    } else {
        vec![]
    };

    let run = ProtocolRun {
        run_id: run_id.clone(),
        protocol_id: input.protocol_id,
        protocol_version: input.protocol_version,
        container_id: input.container_id.clone(),
        target_record_id: input.target_record_id.clone(),
        status: ProtocolRunStatus::Active,
        attention_state: AttentionState {
            container_id: input.container_id,
            record_id: input.target_record_id,
            field_id: None,
            protocol_run_id: Some(run_id),
            stage_id: input.initial_stage_id,
        },
        stage_states,
        started_at,
        completed_at: None,
    };

    let mut collection = load_runs(store)?;
    collection.runs.push(run.clone());
    save_runs(store, &collection)?;
    Ok(RunResult { run })
}

/// Advance a run to a new stage. Updates `attention_state.stage_id`.
pub fn advance_stage(
    store: &dyn RepositoryStore,
    input: AdvanceStageInput,
) -> Result<RunResult, RepositoryError> {
    let mut collection = load_runs(store)?;
    let run = collection
        .runs
        .iter_mut()
        .find(|r| r.run_id == input.run_id)
        .ok_or_else(|| run_not_found(&input.run_id))?;

    if run.status != ProtocolRunStatus::Active {
        return Err(RepositoryError::RunInvalidState {
            run_id: input.run_id.clone(),
            message: format!("status is {:?}", run.status),
        });
    }

    let now = now_iso8601();

    if input.complete_current {
        for ss in run.stage_states.iter_mut() {
            if ss.status == StageStatus::Active {
                ss.status = StageStatus::Completed;
                ss.completed_at = Some(now.clone());
            }
        }
    }

    run.stage_states.push(StageState {
        stage_id: input.stage_id.clone(),
        status: StageStatus::Active,
        started_at: Some(now),
        completed_at: None,
    });

    run.attention_state.stage_id = Some(input.stage_id);

    let run = run.clone();
    save_runs(store, &collection)?;
    Ok(RunResult { run })
}

/// Get a single protocol run by its run_id.
pub fn get_run(
    store: &dyn RepositoryStore,
    run_id: &str,
) -> Result<GetRunResult, RepositoryError> {
    let collection = load_runs(store)?;
    Ok(collection
        .runs
        .into_iter()
        .find(|r| r.run_id == run_id)
        .map(|r| GetRunResult::Found(Box::new(r)))
        .unwrap_or(GetRunResult::NotFound))
}

/// List runs, optionally filtered by protocol_id, container_id, or status string.
pub fn list_runs(
    store: &dyn RepositoryStore,
    filter: RunListFilter,
) -> Result<Vec<RunSummary>, RepositoryError> {
    let collection = load_runs(store)?;
    let summaries = collection
        .runs
        .iter()
        .filter(|r| {
            if let Some(ref pid) = filter.protocol_id {
                if r.protocol_id != *pid {
                    return false;
                }
            }
            if let Some(ref cid) = filter.container_id {
                if r.container_id != *cid {
                    return false;
                }
            }
            if let Some(ref status) = filter.status {
                if status_to_string(&r.status) != *status {
                    return false;
                }
            }
            true
        })
        .map(run_to_summary)
        .collect();
    Ok(summaries)
}

/// Returns summaries of all runs whose target_record_id matches `record_id`.
pub fn list_runs_for_record(
    store: &dyn RepositoryStore,
    record_id: &str,
) -> Result<Vec<RunSummary>, RepositoryError> {
    let collection = load_runs(store)?;
    Ok(collection
        .runs
        .iter()
        .filter(|r| r.target_record_id.as_deref() == Some(record_id))
        .map(run_to_summary)
        .collect())
}

/// Mark an Active run as Completed.
pub fn complete_run(
    store: &dyn RepositoryStore,
    run_id: &str,
) -> Result<RunResult, RepositoryError> {
    let mut collection = load_runs(store)?;
    let run = collection
        .runs
        .iter_mut()
        .find(|r| r.run_id == run_id)
        .ok_or_else(|| run_not_found(run_id))?;
    if run.status != ProtocolRunStatus::Active {
        return Err(RepositoryError::RunInvalidState {
            run_id: run_id.to_string(),
            message: format!("status is {:?}", run.status),
        });
    }

    let now = now_iso8601();
    run.status = ProtocolRunStatus::Completed;
    run.completed_at = Some(now.clone());
    for ss in run.stage_states.iter_mut() {
        if ss.status == StageStatus::Active {
            ss.status = StageStatus::Completed;
            ss.completed_at = Some(now.clone());
        }
    }

    let run = run.clone();
    save_runs(store, &collection)?;
    Ok(RunResult { run })
}

/// Mark an Active run as Abandoned.
pub fn abandon_run(
    store: &dyn RepositoryStore,
    run_id: &str,
) -> Result<RunResult, RepositoryError> {
    let mut collection = load_runs(store)?;
    let run = collection
        .runs
        .iter_mut()
        .find(|r| r.run_id == run_id)
        .ok_or_else(|| run_not_found(run_id))?;
    if run.status != ProtocolRunStatus::Active {
        return Err(RepositoryError::RunInvalidState {
            run_id: run_id.to_string(),
            message: format!("status is {:?}", run.status),
        });
    }

    let now = now_iso8601();
    run.status = ProtocolRunStatus::Abandoned;
    run.completed_at = Some(now);

    let run = run.clone();
    save_runs(store, &collection)?;
    Ok(RunResult { run })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::memory::MemoryStore;

    fn make_create_input(
        protocol_id: &str,
        container_id: &str,
        stage_id: Option<&str>,
    ) -> CreateRunInput {
        CreateRunInput {
            protocol_id: protocol_id.to_string(),
            protocol_version: 1,
            container_id: container_id.to_string(),
            target_record_id: None,
            initial_stage_id: stage_id.map(|s| s.to_string()),
        }
    }

    #[test]
    fn create_run_basic() {
        let store = MemoryStore::empty();
        let result = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        assert!(!result.run.run_id.is_empty());
        assert_eq!(result.run.status, ProtocolRunStatus::Active);
        assert_eq!(result.run.stage_states.len(), 0);
    }

    #[test]
    fn create_run_with_initial_stage() {
        let store = MemoryStore::empty();
        let result =
            create_run(&store, make_create_input("p-1", "c-1", Some("stage-a"))).unwrap();
        assert_eq!(result.run.stage_states.len(), 1);
        assert_eq!(result.run.stage_states[0].stage_id, "stage-a");
        assert_eq!(result.run.stage_states[0].status, StageStatus::Active);
        assert_eq!(
            result.run.attention_state.stage_id.as_deref(),
            Some("stage-a")
        );
    }

    #[test]
    fn get_run_found() {
        let store = MemoryStore::empty();
        let created = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        let found = get_run(&store, &created.run.run_id).unwrap();
        match found {
            GetRunResult::Found(r) => assert_eq!(r.run_id, created.run.run_id),
            GetRunResult::NotFound => panic!("expected Found, got NotFound"),
        }
    }

    #[test]
    fn get_run_not_found() {
        let store = MemoryStore::empty();
        let result = get_run(&store, "nonexistent-uuid").unwrap();
        assert!(matches!(result, GetRunResult::NotFound));
    }

    #[test]
    fn list_runs_empty() {
        let store = MemoryStore::empty();
        let runs = list_runs(&store, RunListFilter::default()).unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn list_runs_filter_by_status() {
        let store = MemoryStore::empty();
        let r1 = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        complete_run(&store, &r1.run.run_id).unwrap();

        let active = list_runs(
            &store,
            RunListFilter {
                status: Some("Active".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, "Active");
    }

    #[test]
    fn advance_stage_updates_attention() {
        let store = MemoryStore::empty();
        let created =
            create_run(&store, make_create_input("p-1", "c-1", Some("stage-a"))).unwrap();
        let advanced = advance_stage(
            &store,
            AdvanceStageInput {
                run_id: created.run.run_id.clone(),
                stage_id: "stage-b".to_string(),
                complete_current: false,
            },
        )
        .unwrap();
        assert_eq!(
            advanced.run.attention_state.stage_id.as_deref(),
            Some("stage-b")
        );
        assert_eq!(advanced.run.stage_states.len(), 2);
    }

    #[test]
    fn advance_stage_complete_current() {
        let store = MemoryStore::empty();
        let created =
            create_run(&store, make_create_input("p-1", "c-1", Some("stage-a"))).unwrap();
        let advanced = advance_stage(
            &store,
            AdvanceStageInput {
                run_id: created.run.run_id.clone(),
                stage_id: "stage-b".to_string(),
                complete_current: true,
            },
        )
        .unwrap();
        let prev = advanced
            .run
            .stage_states
            .iter()
            .find(|s| s.stage_id == "stage-a")
            .unwrap();
        assert_eq!(prev.status, StageStatus::Completed);
        assert!(prev.completed_at.is_some());
    }

    #[test]
    fn complete_run_sets_status() {
        let store = MemoryStore::empty();
        let created = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        let completed = complete_run(&store, &created.run.run_id).unwrap();
        assert_eq!(completed.run.status, ProtocolRunStatus::Completed);
        assert!(completed.run.completed_at.is_some());
    }

    #[test]
    fn abandon_run_sets_status() {
        let store = MemoryStore::empty();
        let created = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        let abandoned = abandon_run(&store, &created.run.run_id).unwrap();
        assert_eq!(abandoned.run.status, ProtocolRunStatus::Abandoned);
        assert!(abandoned.run.completed_at.is_some());
    }

    #[test]
    fn complete_run_already_completed_returns_error() {
        let store = MemoryStore::empty();
        let created = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        complete_run(&store, &created.run.run_id).unwrap();
        let err = complete_run(&store, &created.run.run_id);
        assert!(err.is_err());
    }

    #[test]
    fn abandon_run_already_abandoned_returns_error() {
        let store = MemoryStore::empty();
        let created = create_run(&store, make_create_input("p-1", "c-1", None)).unwrap();
        abandon_run(&store, &created.run.run_id).unwrap();
        let err = abandon_run(&store, &created.run.run_id);
        assert!(err.is_err());
    }
}

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use crate::store::memory::MemoryStore;

    #[test]
    fn run_roundtrip_memory_to_json() {
        let store = MemoryStore::empty();
        let created = create_run(
            &store,
            CreateRunInput {
                protocol_id: "p-rt".to_string(),
                protocol_version: 1,
                container_id: "c-rt".to_string(),
                target_record_id: Some("rec-rt".to_string()),
                initial_stage_id: Some("stage-x".to_string()),
            },
        )
        .unwrap();
        let run_id = created.run.run_id.clone();

        advance_stage(
            &store,
            AdvanceStageInput {
                run_id: run_id.clone(),
                stage_id: "stage-y".to_string(),
                complete_current: true,
            },
        )
        .unwrap();

        // Extract the JSON from the store directly to verify the storage shape.
        let json_val = store.load_instance_json(RUNS_PATH).unwrap();
        let collection: ProtocolRunsCollection = serde_json::from_value(json_val).unwrap();
        let stored = collection.runs.iter().find(|r| r.run_id == run_id).unwrap();
        assert_eq!(stored.stage_states.len(), 2);
        assert_eq!(stored.attention_state.stage_id.as_deref(), Some("stage-y"));
        assert_eq!(
            stored.stage_states[0].status,
            StageStatus::Completed,
            "first stage must be marked completed"
        );
    }
}
