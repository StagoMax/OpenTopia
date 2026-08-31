use super::{
    current_settings, ensure_experience_mode_enabled, ensure_thread, load_bound_agent_context,
    provider_settings_for_thread, sync_thread_bundled_plugin_activations, ApiError, AppState,
};
use crate::connection_operation_runtime::{
    connection_authority_for_context, ConnectionOperationUnavailable,
};
use crate::flow_library_runtime::{
    register_workflow_library_tool, validate_workflow_library_provider,
    WorkflowLibraryProviderUpdate,
};
use crate::thread_runtime::sync_runtime_connection_tools;
use crate::workflow_compiler::compile_published_workflow;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use opentopia_core::{
    activation_root_node_ids, agent_model_context_with_runtime, default_graph_trigger,
    definition_from_draft, experience_mode_module, graph_ingress_policy_for_trigger,
    prepare_flow_resume, resolve_flow_approval, simulate_flow, spawn_flow_run, validate_flow_spec,
    ActiveFlowStoreError, ActiveFlowV1, AgentRunConfig, AgentRunIdentity, CapabilityProjection,
    ExecutionAuthority, ExperienceMode, ExperienceSurfaceProfile, FlowDraftStatusV1, FlowDraftV1,
    FlowRunStatusV1, FlowRunV1, FlowSourceV1, FlowSpecV1, FlowStatusV1, FlowStoreError,
    FlowTrialV1, HumanTaskActionV1, HumanTaskStoreError, RuntimeConnectionAuthorityV1,
    RuntimeSurface, SessionStore, ToolInvocationContext, ToolStateStore, TurnStatus,
    WorkflowAgentSpecV1, WorkflowCompileError, WorkflowLibraryProviderV1,
    WorkflowOutputReviewPolicyV1, WorkflowOutputSpecV1,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/flows", get(search_flows))
        .route("/api/flows/:flow_id", get(get_flow))
        .route(
            "/api/threads/:thread_id/flow-drafts",
            get(list_flow_drafts).post(create_flow_draft),
        )
        .route(
            "/api/threads/:thread_id/flow-draft",
            get(get_thread_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id",
            get(get_flow_draft).put(update_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id/validate",
            post(validate_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id/simulate",
            post(simulate_flow_draft),
        )
        .route(
            "/api/flow-drafts/:draft_id/test-run",
            post(start_flow_test_run),
        )
        .route(
            "/api/flow-drafts/:draft_id/activate",
            post(activate_flow_draft),
        )
        .route("/api/flows/:flow_id/pause", post(pause_flow))
        .route("/api/flows/:flow_id/resume", post(resume_flow))
        .route("/api/flows/:flow_id/copy", post(copy_flow))
        .route("/api/threads/:thread_id/flow-runs", get(list_flow_runs))
        .route("/api/flow-runs", get(list_all_flow_runs))
        .route("/api/flow-runs/:run_id", get(get_flow_run))
        .route("/api/flow-runs/:run_id/pause", post(pause_flow_run))
        .route("/api/flow-runs/:run_id/resume", post(resume_flow_run))
        .route("/api/flow-runs/:run_id/cancel", post(cancel_flow_run))
}

async fn search_flows(
    State(state): State<AppState>,
    Query(query): Query<SearchFlowsQuery>,
) -> Result<Json<Vec<ActiveFlowV1>>, ApiError> {
    ensure_enterprise(&state)?;
    let needle = query
        .query
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let flows = state
        .store
        .list_active_flows(query.status)
        .map_err(active_flow_error)?
        .into_iter()
        .filter(|flow| {
            needle.is_empty()
                || flow.flow_id.to_lowercase().contains(&needle)
                || flow.name.to_lowercase().contains(&needle)
                || flow.created_by.to_lowercase().contains(&needle)
        })
        .collect();
    Ok(Json(flows))
}

async fn get_flow(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
) -> Result<Json<ActiveFlowV1>, ApiError> {
    ensure_enterprise(&state)?;
    state
        .store
        .get_active_flow(&flow_id)
        .map_err(active_flow_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Flow not found"))
}

async fn list_flow_drafts(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<FlowDraftView>>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    let drafts = state
        .store
        .list_flow_drafts(Some(thread_id))
        .map_err(flow_error)?;
    let views = drafts
        .into_iter()
        .map(|draft| draft_view(&state, draft))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(views))
}

async fn get_thread_flow_draft(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Option<FlowDraftView>>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    let draft = state
        .store
        .get_thread_flow_draft(thread_id)
        .map_err(flow_error)?;
    Ok(Json(
        draft.map(|draft| draft_view(&state, draft)).transpose()?,
    ))
}

async fn create_flow_draft(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<CreateFlowDraftRequest>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    if let FlowSourceV1::RunTrace { run_id, .. } = &request.spec.source {
        let run = state
            .store
            .get_turn(*run_id)
            .map_err(flow_error)?
            .ok_or_else(|| ApiError::not_found("source Run/Trace not found"))?;
        if run.status != TurnStatus::Succeeded {
            return Err(ApiError::conflict(
                "only a successful Run/Trace can be converted into a FlowDraft",
            ));
        }
    }
    let capabilities = flow_capabilities(&state, thread_id)?;
    let draft = FlowDraftV1::new(thread_id, request.spec, &capabilities);
    let draft = state.store.create_flow_draft(&draft).map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn get_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_enterprise(&state)?;
    let draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn update_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<UpdateFlowDraftRequest>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_enterprise(&state)?;
    let mut draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    if draft.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Flow draft revision conflict; current revision is {}",
            draft.revision
        )));
    }
    let capabilities = flow_capabilities(&state, draft.thread_id)?;
    draft.replace_spec(request.spec, &capabilities);
    let draft = state
        .store
        .update_flow_draft(&draft, request.expected_revision)
        .map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn validate_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
) -> Result<Json<FlowDraftView>, ApiError> {
    ensure_enterprise(&state)?;
    let mut draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    let expected_revision = draft.revision;
    let report = validate_flow_spec(&draft.spec, &flow_capabilities(&state, draft.thread_id)?);
    draft.status = if report.valid {
        FlowDraftStatusV1::ReadyToPublish
    } else {
        FlowDraftStatusV1::Reviewing
    };
    draft.last_validation = Some(report);
    draft.updated_at = Utc::now();
    let draft = state
        .store
        .update_flow_draft(&draft, expected_revision)
        .map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn simulate_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<SimulateFlowDraftRequest>,
) -> Result<Json<FlowTrialV1>, ApiError> {
    ensure_enterprise(&state)?;
    let mut draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    let expected_revision = draft.revision;
    let trial = simulate_flow(
        &draft,
        request.input,
        &flow_capabilities(&state, draft.thread_id)?,
    );
    draft.last_validation = Some(trial.report.clone());
    draft.status = if trial.report.valid {
        FlowDraftStatusV1::ReadyToPublish
    } else {
        FlowDraftStatusV1::Reviewing
    };
    draft.updated_at = Utc::now();
    state
        .store
        .update_flow_draft(&draft, expected_revision)
        .map_err(flow_error)?;
    Ok(Json(
        state.store.insert_flow_trial(&trial).map_err(flow_error)?,
    ))
}

async fn start_flow_test_run(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<StartFlowTestRunRequest>,
) -> Result<Json<FlowRunV1>, ApiError> {
    ensure_enterprise(&state)?;
    let draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    let thread = ensure_flow_thread(&state, draft.thread_id)?;
    if !draft
        .last_validation
        .as_ref()
        .is_some_and(|report| report.valid)
    {
        return Err(ApiError::conflict(
            "Flow draft must pass validation before a Test Run",
        ));
    }
    if request.started_by.trim().is_empty() {
        return Err(ApiError::bad_request("startedBy is required"));
    }
    let candidate = definition_from_draft(&draft, draft.revision, request.started_by.trim());
    let compiled = compile_published_workflow(&state.store, &candidate)
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    validate_workflow_library_provider(&compiled, request.library_provider)?;
    let run = FlowRunV1::new_for_test_run(
        draft.thread_id,
        draft.id,
        draft.revision,
        compiled,
        request.library_provider,
        request.input,
    )
    .map_err(ApiError::from)?;
    let context = flow_runtime_context(&state, &thread, &run).await?;
    let run = state.store.insert_flow_run(&run).map_err(flow_error)?;
    spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    Ok(Json(run))
}

async fn activate_flow_draft(
    State(state): State<AppState>,
    Path(draft_id): Path<Uuid>,
    Json(request): Json<ActivateFlowDraftRequest>,
) -> Result<Json<ActiveFlowV1>, ApiError> {
    ensure_enterprise(&state)?;
    let draft = state
        .store
        .get_flow_draft(draft_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow draft not found"))?;
    ensure_flow_thread(&state, draft.thread_id)?;
    if request.activated_by.trim().is_empty() {
        return Err(ApiError::bad_request("activatedBy is required"));
    }
    // Reject a stale activation before publishing the immutable definition.
    // Publishing changes the draft permanently, so the mutable Flow CAS must
    // be validated first even though the final update remains CAS-protected.
    let existing = state
        .store
        .get_active_flow(&draft.spec.flow_id)
        .map_err(active_flow_error)?;
    if let Some(flow) = existing.as_ref() {
        if request.expected_flow_revision != Some(flow.revision) {
            return Err(ApiError::conflict(format!(
                "Flow revision conflict; current revision is {}",
                flow.revision
            )));
        }
    }
    let definition = state
        .store
        .publish_flow_draft(draft_id, request.activated_by.trim())
        .map_err(flow_error)?;
    let compiled = compile_published_workflow(&state.store, &definition)
        .map_err(|error| ApiError::conflict(error.to_string()))?;
    let trigger = default_graph_trigger(&compiled.graph);
    trigger
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let ingress_policy = graph_ingress_policy_for_trigger(&compiled.graph, &trigger);
    let output = request
        .output
        .or_else(|| {
            existing
                .as_ref()
                .map(|flow| flow.active_revision.output.clone())
        })
        .unwrap_or(WorkflowOutputSpecV1::Inbox);
    output
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let output_review_policy = request
        .output_review_policy
        .or_else(|| {
            existing
                .as_ref()
                .map(|flow| flow.active_revision.output_review_policy)
        })
        .unwrap_or(WorkflowOutputReviewPolicyV1::ExplicitNodesOnly);
    let library_provider = request.library_provider.resolve(
        existing
            .as_ref()
            .and_then(|flow| flow.active_revision.library_provider),
    );
    validate_workflow_library_provider(&compiled, library_provider)?;

    let flow = if let Some(mut flow) = existing {
        let expected = flow.revision;
        flow.apply_revision(
            definition.name,
            compiled,
            trigger,
            ingress_policy,
            output,
            output_review_policy,
            library_provider,
            request.activated_by,
        )
        .map_err(workflow_compile_error)?;
        state
            .store
            .update_active_flow(&flow, expected)
            .map_err(active_flow_error)?
    } else {
        let flow = ActiveFlowV1::new_with_runtime_options(
            definition.name,
            draft.thread_id,
            compiled,
            trigger,
            ingress_policy,
            output,
            output_review_policy,
            library_provider,
            request.activated_by,
        )
        .map_err(workflow_compile_error)?;
        state
            .store
            .insert_active_flow(&flow)
            .map_err(active_flow_error)?
    };
    Ok(Json(flow))
}

async fn pause_flow(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
    Json(request): Json<ChangeFlowStatusRequest>,
) -> Result<Json<ActiveFlowV1>, ApiError> {
    let mut flow = active_flow_for_request(&state, &flow_id)?;
    if flow.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Flow revision conflict; current revision is {}",
            flow.revision
        )));
    }
    if flow.status == FlowStatusV1::Paused {
        return Err(ApiError::conflict("Flow is already paused"));
    }
    flow.pause();
    Ok(Json(
        state
            .store
            .update_active_flow(&flow, request.expected_revision)
            .map_err(active_flow_error)?,
    ))
}

async fn resume_flow(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
    Json(request): Json<ChangeFlowStatusRequest>,
) -> Result<Json<ActiveFlowV1>, ApiError> {
    let mut flow = active_flow_for_request(&state, &flow_id)?;
    if flow.revision != request.expected_revision {
        return Err(ApiError::conflict(format!(
            "Flow revision conflict; current revision is {}",
            flow.revision
        )));
    }
    if flow.status == FlowStatusV1::Active {
        return Err(ApiError::conflict("Flow is already active"));
    }
    flow.resume();
    Ok(Json(
        state
            .store
            .update_active_flow(&flow, request.expected_revision)
            .map_err(active_flow_error)?,
    ))
}

async fn copy_flow(
    State(state): State<AppState>,
    Path(flow_id): Path<String>,
    Json(request): Json<CopyFlowRequest>,
) -> Result<Json<FlowDraftView>, ApiError> {
    let flow = active_flow_for_request(&state, &flow_id)?;
    let new_flow_id = request.flow_id.trim();
    let name = request.name.trim();
    if new_flow_id.is_empty() || name.is_empty() || request.owner.trim().is_empty() {
        return Err(ApiError::bad_request(
            "flowId, name, and owner are required",
        ));
    }
    if state
        .store
        .get_active_flow(new_flow_id)
        .map_err(active_flow_error)?
        .is_some()
    {
        return Err(ApiError::conflict("Flow already exists"));
    }
    let source = state
        .store
        .get_flow_definition(
            &flow.flow_id,
            Some(flow.active_revision.compiled_workflow.flow_version),
        )
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow definition not found"))?;
    let mut spec = source.to_spec();
    spec.flow_id = new_flow_id.to_string();
    spec.name = name.to_string();
    spec.owner = request.owner.trim().to_string();
    disable_automatic_ingress(&mut spec);
    let capabilities = flow_capabilities(&state, flow.thread_id)?;
    let draft = FlowDraftV1::new(flow.thread_id, spec, &capabilities);
    let draft = state.store.create_flow_draft(&draft).map_err(flow_error)?;
    Ok(Json(draft_view(&state, draft)?))
}

async fn list_flow_runs(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<Vec<FlowRunV1>>, ApiError> {
    ensure_flow_thread(&state, thread_id)?;
    Ok(Json(
        state.store.list_flow_runs(thread_id).map_err(flow_error)?,
    ))
}

async fn list_all_flow_runs(
    State(state): State<AppState>,
    Query(query): Query<FlowRunListQuery>,
) -> Result<Json<Vec<FlowRunV1>>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(
        state
            .store
            .list_all_flow_runs(query.status, query.limit.unwrap_or(200))
            .map_err(flow_error)?,
    ))
}

async fn get_flow_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<FlowRunV1>, ApiError> {
    ensure_enterprise(&state)?;
    Ok(Json(flow_run_for_request(&state, run_id)?))
}

async fn pause_flow_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<FlowRunV1>, ApiError> {
    let mut run = flow_run_for_request(&state, run_id)?;
    if !matches!(
        run.status,
        FlowRunStatusV1::Queued | FlowRunStatusV1::Running | FlowRunStatusV1::Resuming
    ) {
        return Err(ApiError::conflict(
            "only a queued or running Flow can be paused",
        ));
    }
    let expected = run.revision;
    run.status = FlowRunStatusV1::PauseRequested;
    run.touch();
    Ok(Json(
        state
            .store
            .update_flow_run(&run, expected)
            .map_err(flow_error)?,
    ))
}

async fn resume_flow_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
    Json(request): Json<ResumeFlowRunRequest>,
) -> Result<Json<FlowRunV1>, ApiError> {
    let mut run = flow_run_for_request(&state, run_id)?;
    let thread = ensure_flow_thread(&state, run.thread_id)?;
    let expected = run.revision;
    let mut human_task = state
        .store
        .get_pending_human_task_for_flow_run(run.id)
        .map_err(flow_error)?;
    let human_task_expected_revision = human_task.as_ref().map(|task| task.revision);
    let mut human_task_action = None;
    match run.status {
        FlowRunStatusV1::Paused => {
            if request.approved.is_some() {
                return Err(ApiError::bad_request(
                    "approved is only valid while a Flow is waiting for approval",
                ));
            }
            prepare_flow_resume(&mut run, request.retry_interrupted_node)
                .map_err(ApiError::from)?;
            run.status = FlowRunStatusV1::Running;
            run.error = None;
            run.touch();
            if human_task.is_some() {
                human_task_action = Some(HumanTaskActionV1::Retry);
            }
        }
        FlowRunStatusV1::WaitingApproval => {
            let approved = request
                .approved
                .ok_or_else(|| ApiError::bad_request("approved is required"))?;
            resolve_flow_approval(&mut run, approved, request.note.as_deref())
                .map_err(ApiError::from)?;
            human_task_action = Some(if approved {
                HumanTaskActionV1::Approve
            } else {
                HumanTaskActionV1::Reject
            });
        }
        _ => {
            return Err(ApiError::conflict(
                "Flow run is not paused or waiting for approval",
            ))
        }
    }
    // Prepare the resume runtime before committing the run/task transition so
    // a revoked Connection leaves both durable records pending and retryable.
    let context = if run.status.is_terminal() {
        None
    } else {
        Some(flow_runtime_context(&state, &thread, &run).await?)
    };
    let run = if let (Some(mut task), Some(action), Some(task_revision)) = (
        human_task.take(),
        human_task_action,
        human_task_expected_revision,
    ) {
        task.resolve(action, request.note.as_deref(), "local_operator")
            .map_err(ApiError::from)?;
        state
            .store
            .update_flow_run_and_human_task(&run, expected, &task, Some(task_revision))
            .map_err(flow_error)?
            .0
    } else {
        state
            .store
            .update_flow_run(&run, expected)
            .map_err(flow_error)?
    };
    if let Some(context) = context {
        spawn_flow_run(run.id, context).map_err(ApiError::from)?;
    }
    Ok(Json(run))
}

async fn cancel_flow_run(
    State(state): State<AppState>,
    Path(run_id): Path<Uuid>,
) -> Result<Json<FlowRunV1>, ApiError> {
    let mut run = flow_run_for_request(&state, run_id)?;
    if run.status.is_terminal() {
        return Err(ApiError::conflict("Flow run is already terminal"));
    }
    let expected = run.revision;
    if matches!(
        run.status,
        FlowRunStatusV1::Paused | FlowRunStatusV1::WaitingApproval | FlowRunStatusV1::WaitingHuman
    ) {
        run.status = FlowRunStatusV1::Cancelled;
        run.completed_at = Some(Utc::now());
    } else {
        run.status = FlowRunStatusV1::CancelRequested;
    }
    run.active_human_task_id = None;
    run.touch();
    let pending_task = state
        .store
        .get_pending_human_task_for_flow_run(run.id)
        .map_err(flow_error)?;
    let run = if let Some(mut task) = pending_task {
        let task_revision = task.revision;
        task.cancel(Some("Flow run cancelled by operator"), "local_operator")
            .map_err(ApiError::from)?;
        state
            .store
            .update_flow_run_and_human_task(&run, expected, &task, Some(task_revision))
            .map_err(flow_error)?
            .0
    } else {
        state
            .store
            .update_flow_run(&run, expected)
            .map_err(flow_error)?
    };
    Ok(Json(run))
}

fn flow_run_for_request(state: &AppState, run_id: Uuid) -> Result<FlowRunV1, ApiError> {
    let run = state
        .store
        .get_flow_run(run_id)
        .map_err(flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow run not found"))?;
    ensure_flow_thread(state, run.thread_id)?;
    Ok(run)
}

fn active_flow_for_request(state: &AppState, flow_id: &str) -> Result<ActiveFlowV1, ApiError> {
    ensure_enterprise(state)?;
    state
        .store
        .get_active_flow(flow_id)
        .map_err(active_flow_error)?
        .ok_or_else(|| ApiError::not_found("Flow not found"))
}

fn disable_automatic_ingress(spec: &mut FlowSpecV1) {
    let mut roots = activation_root_node_ids(&spec.graph);
    if roots.is_empty() {
        roots.insert(spec.graph.entry_node_id.clone());
    }
    for node in &mut spec.graph.nodes {
        if !roots.contains(&node.id) {
            continue;
        }
        if !node.config.is_object() {
            node.config = json!({});
        }
        node.config.as_object_mut().expect("object config").insert(
            "activation".to_string(),
            json!({
                "expression": {
                    "operator": "source",
                    "source": { "kind": "manual" }
                },
                "ingressPolicy": "require_review"
            }),
        );
    }
}

pub(crate) async fn flow_runtime_context(
    state: &AppState,
    thread: &opentopia_core::Thread,
    run: &FlowRunV1,
) -> Result<ToolInvocationContext, ApiError> {
    let parent_run_id = run.id;
    let capabilities = run.harness_capabilities();
    let connection_authority = run.harness_connection_authority();
    let workflow_agent_specs = run.workflow_agent_specs();
    let settings = current_settings(state);
    let selected_provider =
        provider_settings_for_thread(&settings, thread.model_selection.as_ref());
    validate_workflow_agent_model_policies(
        &workflow_agent_specs,
        &selected_provider.id,
        &selected_provider.model,
    )
    .map_err(ApiError::forbidden)?;
    let harness_capabilities = ExperienceSurfaceProfile::flow_harness_capabilities(
        thread.workspace_root.clone(),
        &capabilities,
    );
    let authority = ExecutionAuthority::new(
        thread.workspace_root.clone(),
        settings.permission_mode,
        settings.sandbox.to_local_sandbox_config(),
        harness_capabilities,
    )
    .map_err(ApiError::from)?;
    let config = AgentRunConfig::from_settings(
        &settings,
        thread.model_selection.as_ref(),
        authority.clone(),
        AgentRunIdentity::root(parent_run_id, 1),
    )
    .with_experience_mode(thread.experience_mode);
    let mut agent = state
        .agent
        .read()
        .expect("agent lock poisoned")
        .begin_run(config)
        .map_err(ApiError::from)?;
    agent.set_mcp_host(state.mcp_host.clone());
    if capabilities.allow_all_plugins || !capabilities.plugins.is_empty() {
        sync_thread_bundled_plugin_activations(&state.store, thread.id, &mut agent);
    } else {
        agent.disable_all_bundled_plugins();
    }
    sync_runtime_connection_tools(
        &state.store,
        &state.mcp_host,
        thread.id,
        &connection_authority.attenuate(&capabilities),
        &mut agent,
    )
    .await
    .map_err(flow_runtime_context_error)?;
    register_workflow_library_tool(
        &mut agent,
        state.library_providers.clone(),
        run,
        &workflow_agent_specs,
    );
    let agent = agent.finalize().map_err(ApiError::from)?;
    let sandbox = authority.sandbox_config().clone();
    let mut context = authority.local_tool_context();
    context.state = Some(ToolStateStore::new(state.store.clone()));
    context.thread_id = Some(thread.id);
    context.agent_turn_id = Some(parent_run_id);
    context.background = Some(state.background.clone());
    context.browser = Some(state.browser.clone());
    context.computer = Some(state.computer.clone());
    agent.project_external_tools_to_context(&mut context);
    let mut model_context = agent_model_context_with_runtime(
        &thread.workspace_root,
        &sandbox,
        &settings.agent_runtime,
        agent.prompt_runtime_capabilities(RuntimeSurface::Desktop),
    );
    model_context
        .items
        .push(experience_mode_module(ExperienceMode::Flow));
    context.fork_model_context = Some(model_context);
    context.flow_harness = Some(Arc::new(agent));
    Ok(context)
}

fn flow_runtime_context_error(error: anyhow::Error) -> ApiError {
    if error
        .downcast_ref::<ConnectionOperationUnavailable>()
        .is_some()
    {
        return ApiError::conflict(error.to_string());
    }
    ApiError::from(error)
}

fn validate_workflow_agent_model_policies(
    specs: &[WorkflowAgentSpecV1],
    provider_id: &str,
    model_id: &str,
) -> Result<(), String> {
    for spec in specs {
        if !spec.model_policy.allows(provider_id, model_id) {
            return Err(format!(
                "Workflow Agent node {} template {}@{} does not allow model {}:{}",
                spec.node_id, spec.template_id, spec.template_version, provider_id, model_id,
            ));
        }
    }
    Ok(())
}

fn draft_view(state: &AppState, draft: FlowDraftV1) -> Result<FlowDraftView, ApiError> {
    let trials = state.store.list_flow_trials(draft.id).map_err(flow_error)?;
    let test_runs = state
        .store
        .list_flow_runs(draft.thread_id)
        .map_err(flow_error)?
        .into_iter()
        .filter(|run| run.test_draft_id == Some(draft.id))
        .collect();
    Ok(FlowDraftView {
        draft,
        trials,
        test_runs,
    })
}

pub(crate) fn ensure_enterprise(state: &AppState) -> Result<(), ApiError> {
    ensure_experience_mode_enabled(&current_settings(state), ExperienceMode::Flow)
}

pub(crate) fn ensure_flow_thread(
    state: &AppState,
    thread_id: Uuid,
) -> Result<opentopia_core::Thread, ApiError> {
    ensure_enterprise(state)?;
    let thread = ensure_thread(state, thread_id)?;
    if thread.experience_mode != ExperienceMode::Flow {
        return Err(ApiError::not_found("Flow session not found"));
    }
    Ok(thread)
}

fn flow_capabilities(state: &AppState, thread_id: Uuid) -> Result<CapabilityProjection, ApiError> {
    let thread = ensure_flow_thread(state, thread_id)?;
    flow_execution_authority(state, &thread).map(|(capabilities, _)| capabilities)
}

fn flow_execution_authority(
    state: &AppState,
    thread: &opentopia_core::Thread,
) -> Result<(CapabilityProjection, RuntimeConnectionAuthorityV1), ApiError> {
    let (instance, template) = load_bound_agent_context(state, thread)?;
    let mut capabilities = instance
        .as_ref()
        .map(|instance| instance.execution_context.capabilities.clone())
        .unwrap_or_else(|| {
            ExperienceSurfaceProfile::flow_runtime_baseline(thread.workspace_root.clone())
        });
    if !capabilities.allow_all_tools {
        capabilities
            .tools
            .extend(ExperienceSurfaceProfile::flow_control_tools());
    }
    let connection_authority = connection_authority_for_context(
        ExperienceMode::Flow,
        instance.as_ref(),
        template.as_ref(),
        &capabilities,
    )
    .attenuate(&capabilities);
    Ok((capabilities, connection_authority))
}

pub(crate) fn flow_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<HumanTaskStoreError>() {
        return match error {
            HumanTaskStoreError::NotFound(_) => ApiError::not_found(error.to_string()),
            HumanTaskStoreError::RevisionConflict(_) => ApiError::conflict(error.to_string()),
        };
    }
    if let Some(error) = error.downcast_ref::<FlowStoreError>() {
        return match error {
            FlowStoreError::DraftNotFound(_) | FlowStoreError::RunNotFound(_) => {
                ApiError::not_found(error.to_string())
            }
            FlowStoreError::RevisionConflict(_)
            | FlowStoreError::RunRevisionConflict(_)
            | FlowStoreError::ValidationRequired
            | FlowStoreError::PassedTrialRequired
            | FlowStoreError::SuccessfulTestRunRequired
            | FlowStoreError::IndependentApproverRequired => ApiError::conflict(error.to_string()),
        };
    }
    ApiError::from(error)
}

fn workflow_compile_error(error: WorkflowCompileError) -> ApiError {
    ApiError::bad_request(error.to_string())
}

fn active_flow_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<ActiveFlowStoreError>() {
        return match error {
            ActiveFlowStoreError::NotFound(_) => ApiError::not_found(error.to_string()),
            ActiveFlowStoreError::RevisionConflict(_) => ApiError::conflict(error.to_string()),
        };
    }
    ApiError::from(error)
}

#[derive(Debug, Deserialize)]
struct SearchFlowsQuery {
    query: Option<String>,
    #[serde(default)]
    status: Option<FlowStatusV1>,
}

#[derive(Debug, Deserialize)]
struct FlowRunListQuery {
    status: Option<FlowRunStatusV1>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CreateFlowDraftRequest {
    spec: FlowSpecV1,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateFlowDraftRequest {
    expected_revision: u32,
    spec: FlowSpecV1,
}

#[derive(Debug, Deserialize)]
struct SimulateFlowDraftRequest {
    #[serde(default)]
    input: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartFlowTestRunRequest {
    #[serde(default)]
    input: Value,
    #[serde(default)]
    library_provider: Option<WorkflowLibraryProviderV1>,
    started_by: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivateFlowDraftRequest {
    activated_by: String,
    #[serde(default)]
    expected_flow_revision: Option<u32>,
    #[serde(default)]
    output: Option<WorkflowOutputSpecV1>,
    #[serde(default)]
    output_review_policy: Option<WorkflowOutputReviewPolicyV1>,
    #[serde(default)]
    library_provider: WorkflowLibraryProviderUpdate,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChangeFlowStatusRequest {
    expected_revision: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CopyFlowRequest {
    flow_id: String,
    name: String,
    owner: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResumeFlowRunRequest {
    #[serde(default)]
    approved: Option<bool>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    retry_interrupted_node: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct FlowDraftView {
    draft: FlowDraftV1,
    trials: Vec<FlowTrialV1>,
    test_runs: Vec<FlowRunV1>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn unavailable_connection_is_a_recoverable_flow_conflict() {
        let error = flow_runtime_context_error(
            ConnectionOperationUnavailable::new("Connection is disabled").into(),
        );

        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.message, "Connection is disabled");
    }

    #[test]
    fn unexpected_runtime_failure_remains_internal() {
        let error = flow_runtime_context_error(anyhow::anyhow!("unexpected runtime failure"));

        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
