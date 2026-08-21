use super::{ApiError, AppState};
use crate::flows_api::{ensure_flow_thread, flow_error, flow_runtime_context};
use crate::workflow_delivery::{deliver_run_output, reconcile_interrupted_deliveries};
use chrono::Utc;
use opentopia_core::{
    content_fingerprint, spawn_flow_run, FlowRunStatusV1, FlowRunV1, SessionStore,
    WorkflowDeliveryStatusV1, WorkflowReleaseStatusV1, WorkflowReleaseV1,
    WorkflowTriggerInvocationStatusV1, WorkflowTriggerInvocationV1,
};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{error, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowInvocationResult {
    pub invocation: WorkflowTriggerInvocationV1,
    pub run: FlowRunV1,
    pub reused: bool,
}

pub(crate) async fn start_release_invocation(
    state: &AppState,
    release: &WorkflowReleaseV1,
    idempotency_key: String,
    input: Value,
) -> Result<WorkflowInvocationResult, ApiError> {
    if let Some(existing) = state
        .store
        .get_workflow_trigger_invocation(release.id, &idempotency_key)
        .map_err(workflow_automation_error)?
    {
        let input_hash = content_fingerprint(
            &serde_json::to_vec(&input)
                .map_err(|error| ApiError::bad_request(error.to_string()))?,
        );
        if existing.input_hash != input_hash {
            return Err(ApiError::conflict(
                "idempotencyKey was already used with different input",
            ));
        }
        let run_id = existing
            .flow_run_id
            .ok_or_else(|| ApiError::conflict("invocation has not produced a Flow run"))?;
        let run = state
            .store
            .get_flow_run(run_id)
            .map_err(flow_error)?
            .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
        return Ok(WorkflowInvocationResult {
            invocation: existing,
            run,
            reused: true,
        });
    }

    let deployment_id = release
        .select_deployment(&idempotency_key)
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    let deployment = state
        .store
        .get_workflow_deployment(deployment_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Workflow deployment not found"))?;
    if deployment.environment != release.environment {
        return Err(ApiError::conflict(
            "selected deployment is outside the release environment",
        ));
    }
    let thread = ensure_flow_thread(state, release.thread_id)?;
    let mut invocation = WorkflowTriggerInvocationV1::accepted(
        release,
        idempotency_key.clone(),
        deployment_id,
        &input,
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;

    if let Err(error) = state.store.insert_workflow_trigger_invocation(&invocation) {
        if let Some(existing) = state
            .store
            .get_workflow_trigger_invocation(release.id, &idempotency_key)
            .map_err(workflow_automation_error)?
        {
            if existing.input_hash != invocation.input_hash {
                return Err(ApiError::conflict(
                    "idempotencyKey was already used with different input",
                ));
            }
            let run_id = existing
                .flow_run_id
                .ok_or_else(|| ApiError::conflict("invocation is already being started"))?;
            let run = state
                .store
                .get_flow_run(run_id)
                .map_err(flow_error)?
                .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
            return Ok(WorkflowInvocationResult {
                invocation: existing,
                run,
                reused: true,
            });
        }
        return Err(workflow_automation_error(error));
    }

    let run = match FlowRunV1::new_from_deployment(release.thread_id, &deployment, input) {
        Ok(run) => run,
        Err(error) => {
            invocation.status = WorkflowTriggerInvocationStatusV1::Failed;
            invocation.error = Some(error.to_string());
            invocation.updated_at = Utc::now();
            let _ = state.store.update_workflow_trigger_invocation(&invocation);
            return Err(ApiError::conflict(error.to_string()));
        }
    };
    let run = match state.store.insert_flow_run(&run) {
        Ok(run) => run,
        Err(error) => {
            invocation.status = WorkflowTriggerInvocationStatusV1::Failed;
            invocation.error = Some(error.to_string());
            invocation.updated_at = Utc::now();
            let _ = state.store.update_workflow_trigger_invocation(&invocation);
            return Err(flow_error(error));
        }
    };
    invocation.flow_run_id = Some(run.id);
    invocation.status = WorkflowTriggerInvocationStatusV1::Started;
    invocation.updated_at = Utc::now();
    invocation = state
        .store
        .update_workflow_trigger_invocation(&invocation)
        .map_err(workflow_automation_error)?;

    let context = flow_runtime_context(
        state,
        &thread,
        run.id,
        run.harness_capabilities(),
        run.harness_connection_authority(),
        run.workflow_agent_specs(),
    )
    .await?;
    spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    Ok(WorkflowInvocationResult {
        invocation,
        run,
        reused: false,
    })
}

pub(crate) fn start_workflow_automation_worker(state: AppState) {
    tokio::spawn(async move {
        let interval_ms = std::env::var("OPENTOPIA_WORKFLOW_AUTOMATION_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(5_000)
            .max(250);
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = process_schedule_tick(&state).await {
                error!(?error, "workflow schedule tick failed");
            }
            if let Err(error) = process_delivery_tick(&state).await {
                error!(?error, "workflow delivery tick failed");
            }
        }
    });
}

async fn process_schedule_tick(state: &AppState) -> anyhow::Result<()> {
    let releases = state
        .store
        .list_workflow_releases(Some(WorkflowReleaseStatusV1::Active))?;
    let now = Utc::now();
    for mut release in releases {
        let expected_revision = release.revision;
        let Some(idempotency_key) = release.trigger.schedule_due_key_and_advance(now) else {
            continue;
        };
        release.touch();
        if state
            .store
            .update_workflow_release(&release, expected_revision)
            .is_err()
        {
            continue;
        }
        let input = json!({
            "trigger": "schedule",
            "scheduledFor": idempotency_key.trim_start_matches("schedule:"),
        });
        if let Err(error) = start_release_invocation(state, &release, idempotency_key, input).await
        {
            warn!(release_id = %release.id, ?error, "scheduled Workflow invocation failed");
        }
    }
    Ok(())
}

async fn process_delivery_tick(state: &AppState) -> anyhow::Result<()> {
    reconcile_interrupted_deliveries(state)?;
    for run in state
        .store
        .list_all_flow_runs(Some(FlowRunStatusV1::Succeeded), 500)?
    {
        if run.deployment_id.is_none() {
            continue;
        }
        if let Err(error) = deliver_run_output(state, &run, false).await {
            warn!(run_id = %run.id, ?error, "Workflow output delivery reconciliation failed");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowFailureCluster {
    pub key: String,
    pub count: u32,
    pub sample: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkflowEvaluationSummary {
    pub deployment_id: Uuid,
    pub total_runs: u32,
    pub run_status_counts: BTreeMap<String, u32>,
    pub evaluation_count: u32,
    pub pass_rate: Option<f64>,
    pub average_score: Option<f64>,
    pub delivery_status_counts: BTreeMap<String, u32>,
    pub failure_clusters: Vec<WorkflowFailureCluster>,
}

pub(crate) fn evaluation_summary(
    state: &AppState,
    deployment_id: Uuid,
) -> Result<WorkflowEvaluationSummary, ApiError> {
    let runs = state
        .store
        .list_all_flow_runs(None, 500)
        .map_err(workflow_automation_error)?
        .into_iter()
        .filter(|run| run.deployment_id == Some(deployment_id))
        .collect::<Vec<_>>();
    let evaluations = state
        .store
        .list_workflow_evaluations(Some(deployment_id))
        .map_err(workflow_automation_error)?;
    let receipts = state
        .store
        .list_workflow_delivery_receipts(Some(deployment_id), None)
        .map_err(workflow_automation_error)?;

    let mut run_status_counts = BTreeMap::new();
    let mut clusters = BTreeMap::<String, (u32, String)>::new();
    for run in &runs {
        *run_status_counts
            .entry(run.status.as_str().to_string())
            .or_default() += 1;
        if let Some(error) = &run.error {
            let key = normalize_failure(error);
            let entry = clusters.entry(key).or_insert((0, error.clone()));
            entry.0 += 1;
        }
    }
    let mut delivery_status_counts = BTreeMap::new();
    for receipt in &receipts {
        *delivery_status_counts
            .entry(receipt.status.as_str().to_string())
            .or_default() += 1;
        if receipt.status == WorkflowDeliveryStatusV1::Failed {
            let error = receipt.error.as_deref().unwrap_or("output delivery failed");
            let key = format!("delivery:{}", normalize_failure(error));
            let entry = clusters.entry(key).or_insert((0, error.to_string()));
            entry.0 += 1;
        }
    }
    let passed = evaluations
        .iter()
        .filter(|evaluation| evaluation.passed)
        .count();
    let score_total = evaluations
        .iter()
        .map(|evaluation| evaluation.score)
        .sum::<f64>();
    let mut failure_clusters = clusters
        .into_iter()
        .map(|(key, (count, sample))| WorkflowFailureCluster { key, count, sample })
        .collect::<Vec<_>>();
    failure_clusters.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    failure_clusters.truncate(20);

    Ok(WorkflowEvaluationSummary {
        deployment_id,
        total_runs: runs.len() as u32,
        run_status_counts,
        evaluation_count: evaluations.len() as u32,
        pass_rate: (!evaluations.is_empty()).then(|| passed as f64 / evaluations.len() as f64),
        average_score: (!evaluations.is_empty()).then(|| score_total / evaluations.len() as f64),
        delivery_status_counts,
        failure_clusters,
    })
}

fn normalize_failure(error: &str) -> String {
    let first_line = error.lines().next().unwrap_or("unknown failure");
    let prefix = first_line.split(':').next().unwrap_or(first_line);
    let normalized = prefix
        .chars()
        .map(|character| {
            if character.is_ascii_digit() {
                '#'
            } else {
                character
            }
        })
        .collect::<String>()
        .to_lowercase();
    normalized.chars().take(120).collect()
}

pub(crate) fn workflow_automation_error(error: anyhow::Error) -> ApiError {
    flow_error(error)
}
