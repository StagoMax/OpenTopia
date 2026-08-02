use super::{
    current_settings, ensure_thread, plugins_api, publish_payload, ApiError, AppState,
    GIT_OUTPUT_BYTES_LIMIT,
};
use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use opentopia_core::{
    normalize_workspace_key, select_scm_connector, AgentEventPayload, BasicPolicyEngine,
    ContributionKind, ExecutionContext, LocalExecutionEnvironment, LocalGitRemote,
    LocalGitV1Operation, LocalGitV1Output, LocalGitV1Request, LocalGitV1Response,
    LocalGitV1Service, PolicyDecision, PolicyEngine, ResourceLimit, ScmConnectorCandidate,
    ScmConnectorDescriptor, ScmConnectorSelection, ScmRemoteBinding, ToolCall, ToolResult,
    LOCAL_GIT_V1_API_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;
use uuid::Uuid;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/threads/:thread_id/local-git/v1",
            post(execute_local_git),
        )
        .route(
            "/api/threads/:thread_id/scm/remotes/:remote_name/connector",
            get(get_remote_connector).put(put_remote_connector),
        )
}

async fn execute_local_git(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(mut request): Json<LocalGitV1Request>,
) -> Result<Json<LocalGitV1Response>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    request.repository = resolve_repository(&thread.workspace_root, &request.repository)?;
    let call = ToolCall::new(
        "local_git.v1",
        json!({
            "apiVersion": LOCAL_GIT_V1_API_VERSION,
            "request": &request,
        }),
    );
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::ToolCallStarted { call: call.clone() },
    );
    let settings = current_settings(&state);
    let config = settings.sandbox.to_local_sandbox_config();
    let policy = BasicPolicyEngine::new_with_sandbox_config(
        thread.workspace_root.clone(),
        settings.permission_mode,
        &config,
    );
    if let Err(error) = enforce_local_git_policy(&policy, &request.repository, &request.operation) {
        publish_local_git_failure(&state, thread_id, &call, &request.operation, &error.message);
        return Err(error);
    }
    let environment =
        LocalExecutionEnvironment::with_sandbox_config(thread.workspace_root.clone(), config);
    let response = match LocalGitV1Service::new(&environment)
        .execute(&request, git_execution_context())
        .await
    {
        Ok(response) => response,
        Err(error) => {
            let message = error.to_string();
            publish_local_git_failure(&state, thread_id, &call, &request.operation, &message);
            return Err(ApiError::bad_request(message));
        }
    };
    publish_local_git_success(&state, thread_id, &call, &response);
    Ok(Json(response))
}

async fn get_remote_connector(
    State(state): State<AppState>,
    Path((thread_id, remote_name)): Path<(Uuid, String)>,
) -> Result<Json<ScmRemoteConnectorResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let remote = load_remote(&state, &thread, &remote_name).await?;
    let connectors = active_connectors(&state, &thread)?;
    let workspace_key = normalize_workspace_key(&thread.workspace_root);
    let binding = state
        .store
        .get_scm_remote_binding(&workspace_key, &remote_name)?;
    let selection = select_scm_connector(&workspace_key, &remote, &connectors, binding.as_ref());
    Ok(Json(ScmRemoteConnectorResponse {
        remote,
        connectors,
        binding,
        selection,
    }))
}

async fn put_remote_connector(
    State(state): State<AppState>,
    Path((thread_id, remote_name)): Path<(Uuid, String)>,
    Json(request): Json<PutRemoteConnectorRequest>,
) -> Result<Json<ScmRemoteConnectorResponse>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let remote = load_remote(&state, &thread, &remote_name).await?;
    let connectors = active_connectors(&state, &thread)?;
    let workspace_key = normalize_workspace_key(&thread.workspace_root);
    let binding = match (request.connector_plugin_id, request.connector_id) {
        (None, None) => {
            state
                .store
                .delete_scm_remote_binding(&workspace_key, &remote_name)?;
            None
        }
        (Some(plugin_id), Some(connector_id)) => {
            let candidate = best_candidates(&select_scm_connector(
                &workspace_key,
                &remote,
                &connectors,
                None,
            ))
            .into_iter()
            .find(|candidate| {
                candidate.plugin_id == plugin_id && candidate.connector_id == connector_id
            })
            .ok_or_else(|| {
                ApiError::bad_request(
                    "selected connector is not an active best match for this remote",
                )
            })?;
            let binding = ScmRemoteBinding {
                workspace_key: workspace_key.clone(),
                remote_name: remote_name.clone(),
                connector_plugin_id: candidate.plugin_id,
                connector_id: candidate.connector_id,
                account_binding_id: request.account_binding_id,
            };
            Some(state.store.put_scm_remote_binding(&binding)?)
        }
        _ => {
            return Err(ApiError::bad_request(
                "connectorPluginId and connectorId must be set or cleared together",
            ))
        }
    };
    let selection = select_scm_connector(&workspace_key, &remote, &connectors, binding.as_ref());
    Ok(Json(ScmRemoteConnectorResponse {
        remote,
        connectors,
        binding,
        selection,
    }))
}

async fn load_remote(
    state: &AppState,
    thread: &opentopia_core::Thread,
    remote_name: &str,
) -> Result<LocalGitRemote, ApiError> {
    let config = current_settings(state).sandbox.to_local_sandbox_config();
    let environment =
        LocalExecutionEnvironment::with_sandbox_config(thread.workspace_root.clone(), config);
    let response = LocalGitV1Service::new(&environment)
        .execute(
            &LocalGitV1Request {
                repository: thread.workspace_root.clone(),
                operation: LocalGitV1Operation::Remotes,
            },
            git_execution_context(),
        )
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let opentopia_core::LocalGitV1Output::Remotes(remotes) = response.output else {
        return Err(ApiError::internal(
            "localGit.v1 returned the wrong remote output",
        ));
    };
    remotes
        .into_iter()
        .find(|remote| remote.name == remote_name)
        .ok_or_else(|| ApiError::not_found(format!("Git remote not found: {remote_name}")))
}

fn active_connectors(
    state: &AppState,
    thread: &opentopia_core::Thread,
) -> Result<Vec<ScmConnectorDescriptor>, ApiError> {
    let mut connectors = Vec::new();
    for contribution in plugins_api::active_contributions_for_thread(&state.store, thread)
        .map_err(|error| ApiError::bad_request(error.to_string()))?
        .into_iter()
        .filter(|contribution| contribution.kind == ContributionKind::ScmConnector)
    {
        connectors.push(
            ScmConnectorDescriptor::from_contribution(&contribution)
                .map_err(|error| ApiError::bad_request(error.to_string()))?,
        );
    }
    connectors.sort_by(|left, right| {
        left.plugin_id
            .cmp(&right.plugin_id)
            .then_with(|| left.connector_id.cmp(&right.connector_id))
    });
    Ok(connectors)
}

fn best_candidates(selection: &ScmConnectorSelection) -> Vec<ScmConnectorCandidate> {
    match selection {
        ScmConnectorSelection::Selected { candidate, .. } => vec![candidate.clone()],
        ScmConnectorSelection::Conflict { candidates, .. } => candidates.clone(),
        ScmConnectorSelection::Unmatched => Vec::new(),
    }
}

fn resolve_repository(workspace_root: &FsPath, repository: &FsPath) -> Result<PathBuf, ApiError> {
    let root = workspace_root
        .canonicalize()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let candidate = if repository.as_os_str().is_empty() {
        root.clone()
    } else if repository.is_absolute() {
        repository.to_path_buf()
    } else {
        root.join(repository)
    };
    let repository = candidate
        .canonicalize()
        .map_err(|_| ApiError::bad_request("Git repository path was not found"))?;
    if !repository.starts_with(&root) {
        return Err(ApiError::bad_request(
            "Git repository must stay inside the thread workspace",
        ));
    }
    Ok(repository)
}

fn git_execution_context() -> ExecutionContext {
    ExecutionContext::with_timeout(Duration::from_secs(120)).with_resource_limits(ResourceLimit {
        max_output_bytes: Some(GIT_OUTPUT_BYTES_LIMIT),
        ..ResourceLimit::default()
    })
}

fn enforce_local_git_policy(
    policy: &dyn PolicyEngine,
    repository: &FsPath,
    operation: &LocalGitV1Operation,
) -> Result<(), ApiError> {
    let repository_decision = if operation.is_mutation() {
        policy.inspect_write(repository)
    } else {
        policy.inspect_read(repository)
    };
    enforce_policy_decision(repository_decision)?;
    if operation.is_mutation() {
        enforce_policy_decision(policy.inspect_command(operation.policy_command()))?;
    }

    let worktree_path = match operation {
        LocalGitV1Operation::CreateWorktree(request) => Some(request.path.as_path()),
        LocalGitV1Operation::RemoveWorktree(request) => Some(request.path.as_path()),
        _ => None,
    };
    if let Some(path) = worktree_path {
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repository.join(path)
        };
        enforce_policy_decision(policy.inspect_write(&target))?;
    }
    Ok(())
}

fn enforce_policy_decision(decision: PolicyDecision) -> Result<(), ApiError> {
    match decision {
        PolicyDecision::Allow => Ok(()),
        PolicyDecision::Deny { reason } => Err(ApiError::bad_request(reason)),
        PolicyDecision::Ask { reason } => Err(ApiError::bad_request(format!(
            "localGit.v1 requires approval before execution: {reason}"
        ))),
    }
}

fn publish_local_git_success(
    state: &AppState,
    thread_id: Uuid,
    call: &ToolCall,
    response: &LocalGitV1Response,
) {
    let output_type = match &response.output {
        LocalGitV1Output::Status(_) => "status",
        LocalGitV1Output::Branches(_) => "branches",
        LocalGitV1Output::Remotes(_) => "remotes",
        LocalGitV1Output::Worktrees(_) => "worktrees",
        LocalGitV1Output::Compare(_) => "compare",
        LocalGitV1Output::Mutation(_) => "mutation",
    };
    publish_payload(
        state,
        thread_id,
        None,
        AgentEventPayload::ToolCallFinished {
            result: ToolResult::text(
                call.id,
                format!("localGit.v1 {:?} completed", response.operation),
                json!({
                    "success": response.command.success,
                    "apiVersion": response.api_version,
                    "operation": response.operation,
                    "outputType": output_type,
                    "exitCode": response.command.exit_code,
                    "truncated": response.command.truncated,
                }),
            ),
        },
    );
}

fn publish_local_git_failure(
    state: &AppState,
    thread_id: Uuid,
    call: &ToolCall,
    operation: &LocalGitV1Operation,
    message: &str,
) {
    let message = message.chars().take(2048).collect::<String>();
    publish_payload(
        state,
        thread_id,
        None,
        AgentEventPayload::ToolCallFinished {
            result: ToolResult::text(
                call.id,
                message.clone(),
                json!({
                    "success": false,
                    "apiVersion": LOCAL_GIT_V1_API_VERSION,
                    "operation": operation.kind(),
                    "error": message,
                }),
            ),
        },
    );
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScmRemoteConnectorResponse {
    remote: LocalGitRemote,
    connectors: Vec<ScmConnectorDescriptor>,
    binding: Option<ScmRemoteBinding>,
    selection: ScmConnectorSelection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PutRemoteConnectorRequest {
    connector_plugin_id: Option<String>,
    connector_id: Option<String>,
    account_binding_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentopia_core::{GitPathsRequest, GitStatusRequest};

    struct DenyWritesPolicy;

    impl PolicyEngine for DenyWritesPolicy {
        fn inspect_read(&self, _path: &FsPath) -> PolicyDecision {
            PolicyDecision::Allow
        }

        fn inspect_write(&self, _path: &FsPath) -> PolicyDecision {
            PolicyDecision::Deny {
                reason: "writes disabled".to_string(),
            }
        }

        fn inspect_command(&self, _command: &str) -> PolicyDecision {
            PolicyDecision::Allow
        }
    }

    #[test]
    fn best_candidates_preserve_explicit_conflicts() {
        let candidates = vec![ScmConnectorCandidate {
            plugin_id: "github".to_string(),
            connector_id: "github".to_string(),
            matcher_id: "github.com".to_string(),
            specificity: opentopia_core::ScmMatcherSpecificity {
                host: 2,
                path: 0,
                scheme: 1,
            },
        }];
        assert_eq!(
            best_candidates(&ScmConnectorSelection::Conflict {
                candidates: candidates.clone(),
                binding_issue: None,
            }),
            candidates
        );
    }

    #[test]
    fn local_git_execution_context_keeps_timeout_and_output_limit() {
        let context = git_execution_context();

        assert_eq!(context.timeout, Duration::from_secs(120));
        assert_eq!(
            context.resource_limits.max_output_bytes,
            Some(GIT_OUTPUT_BYTES_LIMIT)
        );
    }

    #[test]
    fn local_git_mutations_cannot_bypass_write_policy() {
        let repository = FsPath::new("C:/workspace");
        let mutation = LocalGitV1Operation::Stage(GitPathsRequest {
            paths: vec![PathBuf::from("src/lib.rs")],
        });
        let read = LocalGitV1Operation::Status(GitStatusRequest::default());

        assert!(enforce_local_git_policy(&DenyWritesPolicy, repository, &mutation).is_err());
        assert!(enforce_local_git_policy(&DenyWritesPolicy, repository, &read).is_ok());
    }

    struct DenyPushPolicy;

    impl PolicyEngine for DenyPushPolicy {
        fn inspect_read(&self, _path: &FsPath) -> PolicyDecision {
            PolicyDecision::Allow
        }

        fn inspect_write(&self, _path: &FsPath) -> PolicyDecision {
            PolicyDecision::Allow
        }

        fn inspect_command(&self, command: &str) -> PolicyDecision {
            if command == "git push" {
                PolicyDecision::Deny {
                    reason: "push disabled".to_string(),
                }
            } else {
                PolicyDecision::Allow
            }
        }
    }

    #[test]
    fn local_git_mutations_cannot_bypass_command_policy() {
        let repository = FsPath::new("C:/workspace");
        let push = LocalGitV1Operation::Push(opentopia_core::PushRequest {
            remote: "origin".to_string(),
            branch: "main".to_string(),
            set_upstream: false,
        });

        assert!(enforce_local_git_policy(&DenyPushPolicy, repository, &push).is_err());
    }
}
