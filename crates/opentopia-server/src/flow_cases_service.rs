use super::{ApiError, AppState};
use crate::flows_api::{ensure_flow_thread, flow_error, flow_runtime_context};
use crate::workflow_delivery::{deliver_run_output, reconcile_interrupted_deliveries};
use chrono::Utc;
use opentopia_core::{
    content_fingerprint, spawn_flow_run, ActiveFlowV1, FlowCaseStatusV1, FlowCaseV1,
    FlowRunStatusV1, FlowRunV1, FlowStatusV1, SessionStore, WorkflowDeliveryStatusV1,
    WorkflowIngressPolicyV1,
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
pub(crate) struct FlowCaseResult {
    pub case: FlowCaseV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<FlowRunV1>,
    pub reused: bool,
}

pub(crate) async fn accept_flow_case(
    state: &AppState,
    flow: &ActiveFlowV1,
    idempotency_key: String,
    input: Value,
) -> Result<FlowCaseResult, ApiError> {
    if flow.status != FlowStatusV1::Active {
        return Err(ApiError::conflict("Flow is paused"));
    }
    ensure_flow_thread(state, flow.thread_id)?;
    if let Some(existing) = state
        .store
        .get_flow_case(&flow.flow_id, &idempotency_key)
        .map_err(flow_error)?
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
        return resume_or_reuse_case(state, flow, existing).await;
    }

    let case = FlowCaseV1::accepted(flow, idempotency_key.clone(), &input)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Err(error) = state.store.insert_flow_case(&case) {
        if let Some(existing) = state
            .store
            .get_flow_case(&flow.flow_id, &idempotency_key)
            .map_err(flow_error)?
        {
            if existing.input_hash != case.input_hash {
                return Err(ApiError::conflict(
                    "idempotencyKey was already used with different input",
                ));
            }
            return resume_or_reuse_case(state, flow, existing).await;
        }
        return Err(flow_error(error));
    }

    if case.flow_revision.ingress_policy == WorkflowIngressPolicyV1::RequireReview {
        return Ok(FlowCaseResult {
            case,
            run: None,
            reused: false,
        });
    }

    start_accepted_case(state, flow, case, false).await
}

pub(crate) async fn start_pending_flow_case(
    state: &AppState,
    case_id: Uuid,
) -> Result<FlowCaseResult, ApiError> {
    let case = state
        .store
        .get_flow_case_by_id(case_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow case not found"))?;
    if case.status != FlowCaseStatusV1::Accepted || case.flow_run_id.is_some() {
        return Err(ApiError::conflict("Flow case is no longer pending"));
    }
    let flow = state
        .store
        .get_active_flow(&case.flow_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow not found"))?;
    if flow.status != FlowStatusV1::Active {
        return Err(ApiError::conflict(
            "Flow is paused; the pending case cannot start",
        ));
    }
    start_accepted_case(state, &flow, case, false).await
}

pub(crate) fn supersede_pending_flow_case(
    state: &AppState,
    case_id: Uuid,
    replacement_case_id: Option<Uuid>,
    note: String,
) -> Result<FlowCaseV1, ApiError> {
    let mut case = state
        .store
        .get_flow_case_by_id(case_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow case not found"))?;
    if let Some(replacement_id) = replacement_case_id {
        let replacement = state
            .store
            .get_flow_case_by_id(replacement_id)
            .map_err(flow_error)?
            .ok_or_else(|| ApiError::not_found("Replacement Flow case not found"))?;
        if replacement.status != FlowCaseStatusV1::Accepted || replacement.flow_run_id.is_some() {
            return Err(ApiError::conflict(
                "Replacement Flow case must still be pending",
            ));
        }
    }
    case.supersede(replacement_case_id, note)
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    state.store.update_flow_case(&case).map_err(flow_error)
}

async fn resume_or_reuse_case(
    state: &AppState,
    flow: &ActiveFlowV1,
    case: FlowCaseV1,
) -> Result<FlowCaseResult, ApiError> {
    if case.status == FlowCaseStatusV1::Accepted
        && case.flow_revision.ingress_policy == WorkflowIngressPolicyV1::Immediate
    {
        return start_accepted_case(state, flow, case, true).await;
    }
    reuse_case_result(state, case).await
}

async fn reuse_case_result(state: &AppState, case: FlowCaseV1) -> Result<FlowCaseResult, ApiError> {
    let run = match case.flow_run_id {
        Some(run_id) => Some(
            state
                .store
                .get_flow_run(run_id)
                .map_err(flow_error)?
                .ok_or_else(|| ApiError::not_found("Flow run not found"))?,
        ),
        None => None,
    };
    Ok(FlowCaseResult {
        case,
        run,
        reused: true,
    })
}

async fn start_accepted_case(
    state: &AppState,
    flow: &ActiveFlowV1,
    mut case: FlowCaseV1,
    reused: bool,
) -> Result<FlowCaseResult, ApiError> {
    if case.status != FlowCaseStatusV1::Accepted {
        return reuse_case_result(state, case).await;
    }
    let thread = ensure_flow_thread(state, flow.thread_id)?;
    let trigger = case.flow_revision.trigger.clone();
    let mut run = match FlowRunV1::new_from_revision(
        flow.thread_id,
        &case.flow_revision,
        case.input.clone(),
        &trigger,
    ) {
        Ok(run) => run,
        Err(error) => {
            case.status = FlowCaseStatusV1::Failed;
            case.error = Some(error.to_string());
            case.updated_at = Utc::now();
            let _ = state.store.update_flow_case(&case);
            return Err(ApiError::conflict(error.to_string()));
        }
    };
    run.id = Uuid::new_v5(&case.id, b"flow-run");
    let context = match flow_runtime_context(state, &thread, &run).await {
        Ok(context) => context,
        Err(error) => {
            case.status = FlowCaseStatusV1::Failed;
            case.error = Some(error.message.clone());
            case.updated_at = Utc::now();
            let _ = state.store.update_flow_case(&case);
            return Err(error);
        }
    };
    let (run, should_spawn) = match state.store.insert_flow_run(&run) {
        Ok(run) => (run, true),
        Err(error) => match state.store.get_flow_run(run.id).map_err(flow_error)? {
            Some(existing) => (existing, false),
            None => {
                case.status = FlowCaseStatusV1::Failed;
                case.error = Some(error.to_string());
                case.updated_at = Utc::now();
                let _ = state.store.update_flow_case(&case);
                return Err(flow_error(error));
            }
        },
    };
    case.flow_run_id = Some(run.id);
    case.status = FlowCaseStatusV1::Started;
    case.updated_at = Utc::now();
    case = state.store.update_flow_case(&case).map_err(flow_error)?;

    if should_spawn {
        spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    }
    Ok(FlowCaseResult {
        case,
        run: Some(run),
        reused,
    })
}

pub(crate) fn start_flow_worker(state: AppState) {
    tokio::spawn(async move {
        let interval_ms = std::env::var("OPENTOPIA_FLOW_WORKER_INTERVAL_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(5_000)
            .max(250);
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(error) = process_schedule_tick(&state).await {
                error!(?error, "Flow schedule tick failed");
            }
            if let Err(error) = process_delivery_tick(&state).await {
                error!(?error, "Flow delivery tick failed");
            }
        }
    });
}

async fn process_schedule_tick(state: &AppState) -> anyhow::Result<()> {
    let flows = state.store.list_active_flows(Some(FlowStatusV1::Active))?;
    let now = Utc::now();
    for mut flow in flows {
        let expected_revision = flow.revision;
        let Some(idempotency_key) = flow
            .active_revision
            .trigger
            .schedule_due_key_and_advance(now)
        else {
            continue;
        };
        flow.touch();
        if state
            .store
            .update_active_flow(&flow, expected_revision)
            .is_err()
        {
            continue;
        }
        let input = json!({
            "trigger": "schedule",
            "scheduledFor": idempotency_key.trim_start_matches("schedule:"),
        });
        if let Err(error) = accept_flow_case(state, &flow, idempotency_key, input).await {
            warn!(flow_id = %flow.flow_id, ?error, "scheduled Flow case failed");
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
        if run.flow_revision_id.is_none() {
            continue;
        }
        if let Err(error) = deliver_run_output(state, &run, false).await {
            warn!(run_id = %run.id, ?error, "Flow output delivery reconciliation failed");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlowFailureCluster {
    pub key: String,
    pub count: u32,
    pub sample: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlowEvaluationSummary {
    pub flow_revision_id: Uuid,
    pub total_runs: u32,
    pub run_status_counts: BTreeMap<String, u32>,
    pub evaluation_count: u32,
    pub pass_rate: Option<f64>,
    pub average_score: Option<f64>,
    pub delivery_status_counts: BTreeMap<String, u32>,
    pub failure_clusters: Vec<FlowFailureCluster>,
}

pub(crate) fn evaluation_summary(
    state: &AppState,
    flow_revision_id: Uuid,
) -> Result<FlowEvaluationSummary, ApiError> {
    let runs = state
        .store
        .list_all_flow_runs(None, 500)
        .map_err(flow_error)?
        .into_iter()
        .filter(|run| run.flow_revision_id == Some(flow_revision_id))
        .collect::<Vec<_>>();
    let evaluations = state
        .store
        .list_workflow_evaluations(Some(flow_revision_id))
        .map_err(flow_error)?;
    let receipts = state
        .store
        .list_workflow_delivery_receipts(Some(flow_revision_id), None)
        .map_err(flow_error)?;

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
    let passed = evaluations.iter().filter(|item| item.passed).count();
    let score_total = evaluations.iter().map(|item| item.score).sum::<f64>();
    let mut failure_clusters = clusters
        .into_iter()
        .map(|(key, (count, sample))| FlowFailureCluster { key, count, sample })
        .collect::<Vec<_>>();
    failure_clusters.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    failure_clusters.truncate(20);

    Ok(FlowEvaluationSummary {
        flow_revision_id,
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
    prefix
        .chars()
        .map(|character| {
            if character.is_ascii_digit() {
                '#'
            } else {
                character
            }
        })
        .collect::<String>()
        .to_lowercase()
        .chars()
        .take(120)
        .collect()
}
