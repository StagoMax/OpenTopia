#[tokio::test]
async fn durable_tool_effect_replays_a_succeeded_provider_call() {
    let workspace = test_workspace("journal-replay");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
    let thread = store.create_thread(None, workspace.clone()).unwrap();
    let user_message_id = Uuid::new_v4();
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, user_message_id))
        .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert(
        "journal_test".to_string(),
        Arc::new(JournalTestTool {
            executions: Arc::clone(&executions),
            requires_approval: false,
        }),
    );
    let agent = AgentCore::new(Arc::new(MockProvider), registry);
    let provider_call = ProviderToolCall {
        id: "stable-provider-call".to_string(),
        name: "journal_test".to_string(),
        arguments: json!({}),
    };

    let first = agent
        .execute_provider_tool_call(
            &provider_call,
            user_message_id,
            journal_test_context(Arc::clone(&store), thread.id, workspace.clone(), false),
            &mut TurnEvents::new(None),
        )
        .await
        .unwrap();
    let replay = agent
        .execute_provider_tool_call(
            &provider_call,
            user_message_id,
            journal_test_context(Arc::clone(&store), thread.id, workspace.clone(), false),
            &mut TurnEvents::new(None),
        )
        .await
        .unwrap();

    assert_eq!(first.output, "executed");
    assert_eq!(replay.output, "executed");
    assert_eq!(replay.metadata["effectJournalReplay"], true);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let effects = store.list_turn_effects(turn.turn_id).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].status, EffectStatus::Succeeded);
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn failed_provider_tool_result_preserves_error_chain_and_fails_effect() {
    let workspace = test_workspace("journal-error-result");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
    let thread = store.create_thread(None, workspace.clone()).unwrap();
    let user_message_id = Uuid::new_v4();
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, user_message_id))
        .unwrap();
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert(
        "journal_chained_failure".to_string(),
        Arc::new(JournalChainedFailureTool),
    );
    let agent = AgentCore::new(Arc::new(MockProvider), registry);

    let result = agent
        .execute_provider_tool_call(
            &ProviderToolCall {
                id: "chained-failure-call".to_string(),
                name: "journal_chained_failure".to_string(),
                arguments: json!({}),
            },
            user_message_id,
            journal_test_context(Arc::clone(&store), thread.id, workspace.clone(), false),
            &mut TurnEvents::new(None),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output.contains("git diff execution failed"));
    assert!(result
        .output
        .contains("sandbox process creation was denied"));
    assert_eq!(
        result.metadata["errorChain"],
        json!([
            "git diff execution failed",
            "sandbox process creation was denied"
        ])
    );
    let effects = store.list_turn_effects(turn.turn_id).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].status, EffectStatus::Failed);
    assert!(effects[0].result.is_some());
    assert!(effects[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("sandbox process creation was denied")));
    let _ = fs::remove_dir_all(workspace);
}

#[cfg(windows)]
#[tokio::test]
async fn shell_dialect_preflight_is_a_failed_unexecuted_effect() {
    if ShellDialect::current() != ShellDialect::WindowsPowerShell51 {
        return;
    }
    let workspace = test_workspace("journal-shell-dialect");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
    let thread = store.create_thread(None, workspace.clone()).unwrap();
    let user_message_id = Uuid::new_v4();
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, user_message_id))
        .unwrap();
    let agent = AgentCore::new(Arc::new(MockProvider), ToolRegistry::with_core_tools());

    let result = agent
        .execute_provider_tool_call(
            &ProviderToolCall {
                id: "shell-dialect-call".to_string(),
                name: "shell".to_string(),
                arguments: json!({ "command": "git status && git log -1" }),
            },
            user_message_id,
            journal_test_context(Arc::clone(&store), thread.id, workspace.clone(), false),
            &mut TurnEvents::new(None),
        )
        .await
        .unwrap();

    assert!(result.is_error);
    assert_eq!(
        result.metadata["errorRecord"]["code"],
        "shell_dialect_mismatch"
    );
    assert_eq!(result.metadata["errorRecord"]["executed"], false);
    let effects = store.list_turn_effects(turn.turn_id).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].status, EffectStatus::Failed);
    assert!(effects[0]
        .error
        .as_deref()
        .is_some_and(|error| error.contains("Windows PowerShell 5.1")));
    let _ = fs::remove_dir_all(workspace);
}

#[tokio::test]
async fn approved_retry_restarts_the_same_failed_effect_record() {
    let workspace = test_workspace("journal-approved-retry");
    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
    let thread = store.create_thread(None, workspace.clone()).unwrap();
    let user_message_id = Uuid::new_v4();
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, user_message_id))
        .unwrap();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert(
        "journal_test".to_string(),
        Arc::new(JournalTestTool {
            executions: Arc::clone(&executions),
            requires_approval: true,
        }),
    );
    let agent = AgentCore::new(Arc::new(MockProvider), registry);
    let provider_call = ProviderToolCall {
        id: "approval-provider-call".to_string(),
        name: "journal_test".to_string(),
        arguments: json!({}),
    };

    let denied = agent
        .execute_provider_tool_call(
            &provider_call,
            user_message_id,
            journal_test_context(Arc::clone(&store), thread.id, workspace.clone(), false),
            &mut TurnEvents::new(None),
        )
        .await
        .unwrap_err();
    assert!(approval_required(&denied).is_some());
    assert_eq!(
        store.list_turn_effects(turn.turn_id).unwrap()[0].status,
        EffectStatus::Failed
    );

    let approved = agent
        .execute_provider_tool_call(
            &provider_call,
            user_message_id,
            journal_test_context(Arc::clone(&store), thread.id, workspace.clone(), true),
            &mut TurnEvents::new(None),
        )
        .await
        .unwrap();
    assert_eq!(approved.output, "executed");
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let effects = store.list_turn_effects(turn.turn_id).unwrap();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].status, EffectStatus::Succeeded);
    assert_eq!(effects[0].attempt, 2);
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn registry_contains_only_canonical_file_tools() {
    let agent = AgentCore::default();
    let tools = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();

    assert!(tools.contains("apply_patch"));
    assert!(tools.contains("filesystem"));
    for removed in [
        "list_files",
        "read_file",
        "read_files",
        "write_file",
        "search",
        "git_diff",
    ] {
        assert!(!tools.contains(removed));
        assert!(agent.tool_host.catalog.get(removed).is_none());
    }
}

#[test]
fn flow_profile_exposes_work_code_and_orchestration_tools_to_the_provider() {
    let mut agent = AgentCore::default();
    agent.apply_experience_mode(ExperienceMode::Flow);
    agent.restrict_capabilities(
        &crate::enterprise::ExperienceSurfaceProfile::for_mode(crate::model::ExperienceMode::Flow)
            .capabilities,
    );
    let tools = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();

    for expected in [
        "read_attachment",
        "view_attachment",
        "apply_patch",
        "shell",
        "flow_run",
    ] {
        assert!(tools.contains(expected), "missing Flow tool: {expected}");
    }
    assert!(!tools.contains(TOOL_SEARCH_NAME));
    assert!(tools.contains("document_open"));
    assert!(tools.contains("document_get_operation_schemas"));
    assert!(!tools.contains("document_execute"));

    let before_preload_hint = agent.provider_tool_catalog();
    agent.set_attachment_preloaded_tools(["document_execute"]);
    assert_eq!(agent.provider_tool_catalog(), before_preload_hint);
}

#[test]
fn thread_activation_filters_bundled_tools_from_catalog_and_execution_guard() {
    let mut agent = AgentCore::default();
    agent.set_tool_exposure_policy(ToolExposurePolicy::Eager);
    agent.set_bundled_plugin_activations(&HashMap::from([
        ("browser-automation".to_string(), true),
        ("computer-use".to_string(), false),
        ("spreadsheet".to_string(), false),
    ]));
    assert!(agent
        .provider_tool_catalog()
        .iter()
        .any(|tool| tool.name == "browser"));

    agent.set_bundled_plugin_activations(&HashMap::from([
        ("browser-automation".to_string(), false),
        ("computer-use".to_string(), true),
    ]));

    let tools = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();
    assert!(!tools.contains("browser"));
    assert!(tools.contains("computer"));
    assert!(!tools.contains("document_execute"));
    assert!(!agent.tool_is_allowed("browser"));
    assert!(agent.tool_is_allowed("computer"));
    assert!(!agent.tool_is_allowed("document_execute"));

    let mut metadata = json!({});
    agent.insert_tool_source_metadata("computer", &mut metadata);
    assert_eq!(metadata["toolSource"], "bundled_plugin");
    assert_eq!(metadata["pluginName"], "computer-use");
}

#[test]
fn computer_tool_requires_vision() {
    let mut agent = AgentCore::default();
    agent.set_tool_exposure_policy(ToolExposurePolicy::Eager);
    agent.set_bundled_plugin_activations(&HashMap::from([("computer-use".to_string(), true)]));
    agent.set_computer_allowed_applications(["OpenTopia.exe", "chrome.exe"]);

    let tools = agent.provider_tool_catalog();
    assert!(tools.iter().any(|tool| tool.name == "computer"));

    agent.tool_host.model_supports_vision = false;
    assert!(!agent
        .provider_tool_catalog()
        .iter()
        .any(|tool| tool.name == "computer"));
}

#[test]
fn disabling_bundled_plugins_clears_computer_application_authority() {
    let mut agent = AgentCore::default();
    agent.set_computer_allowed_applications(["OpenTopia.exe"]);
    assert!(!agent.tool_host.computer_access_policy.is_empty());
    agent.disable_all_bundled_plugins();
    assert!(agent.tool_host.computer_access_policy.is_empty());
}

#[test]
fn default_mode_exposes_work_memory_without_plan_only_input() {
    let agent = AgentCore::default();
    let tools = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();

    assert!(tools.contains("filesystem"));
    assert!(!tools.contains("read_file"));
    assert!(!tools.contains("read_files"));
    assert!(!tools.contains("search"));
    assert!(!tools.contains("git_diff"));
    assert!(tools.contains("request_user_input"));
    assert!(tools.contains("update_plan"));
    assert!(!tools.contains("complete_task"));
    assert!(tools.contains("shell"));
    assert!(!tools.contains("write_file"));
    assert!(tools.contains("apply_patch"));
    assert!(tools.contains("create_skill"));
    assert!(!tools.contains("spawn_agent"));
}

/// Structured user input is a Plan-mode interaction boundary. Its schema stays
/// stable across root modes for prompt caching, while runtime capability and
/// execution checks reject it outside Plan. Children never see the schema.
#[test]
fn request_user_input_is_available_only_to_the_root_plan_agent() {
    let default_agent = AgentCore::default();
    assert_eq!(default_agent.collaboration_mode, CollaborationMode::Default);
    let default_instructions = default_agent
        .lineage_instructions()
        .expect("default mode instructions");
    for required_instruction in [
        "Collaboration Mode: Default",
        "A visible checklist tool is never, by itself, a reason to create a checklist",
        "Do not create a checklist for a simple or single-step request",
        "Create a checklist only when at least one of these conditions applies",
        "The user asks for a plan or TODOs",
    ] {
        assert!(
            default_instructions.contains(required_instruction),
            "missing Default-mode checklist policy: {required_instruction}"
        );
    }
    let default_tools = default_agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();
    assert!(default_tools.contains("request_user_input"));
    assert!(
        !default_agent
            .prompt_runtime_capabilities(RuntimeSurface::Desktop)
            .request_user_input_available
    );

    let mut plan_agent = AgentCore::default();
    plan_agent
        .apply_collaboration_mode(CollaborationMode::Plan, None)
        .expect("apply plan interaction profile");
    let plan_instructions = plan_agent.lineage_instructions().expect("plan instructions");
    assert!(plan_instructions.contains("If the user asks for execution"));
    assert!(plan_instructions.contains("must not edit or write files"));
    assert!(plan_instructions.contains("<proposed_plan>"));
    assert!(plan_agent
        .provider_tool_catalog()
        .iter()
        .any(|tool| tool.name == "request_user_input"));
    assert!(plan_agent
        .provider_tool_catalog()
        .iter()
        .any(|tool| tool.name == "update_plan"));
    assert!(
        plan_agent
            .prompt_runtime_capabilities(RuntimeSurface::Desktop)
            .request_user_input_available
    );

    let unavailable = compile_runtime_prompt_modules(
        &AgentRuntimeSettings::default(),
        default_agent.prompt_runtime_capabilities(RuntimeSurface::Desktop),
    );
    let unavailable_module = unavailable
        .iter()
        .find(|item| item.metadata["promptModuleId"] == "clarification_policy")
        .expect("clarification module");
    assert_eq!(unavailable_module.metadata["settingValue"], "unavailable");

    let available = compile_runtime_prompt_modules(
        &AgentRuntimeSettings::default(),
        plan_agent.prompt_runtime_capabilities(RuntimeSurface::Desktop),
    );
    let available_module = available
        .iter()
        .find(|item| item.metadata["promptModuleId"] == "clarification_policy")
        .expect("clarification module");
    assert_eq!(available_module.metadata["settingValue"], "available");
    assert!(available_module
        .text_content()
        .contains("request_user_input"));

    let thread_id = Uuid::new_v4();
    let goal = GoalRecord::new(thread_id, "Execute a durable goal", None);
    let mut goal_agent = AgentCore::default();
    goal_agent
        .apply_collaboration_mode(CollaborationMode::Goal, Some(goal))
        .expect("Goal mode");
    assert!(goal_agent
        .provider_tool_catalog()
        .iter()
        .any(|tool| tool.name == "request_user_input"));

    let mut child_plan_agent = AgentCore::default();
    child_plan_agent
        .apply_collaboration_mode(CollaborationMode::Plan, None)
        .expect("Plan mode");
    child_plan_agent.set_agent_context(Uuid::new_v4(), 1);
    assert!(!child_plan_agent
        .provider_tool_catalog()
        .iter()
        .any(|tool| tool.name == "request_user_input"));
}

#[test]
fn changing_collaboration_mode_replaces_stale_mode_instructions() {
    let mut agent = AgentCore::default();
    agent
        .apply_collaboration_mode(CollaborationMode::Plan, None)
        .expect("Plan mode");
    assert!(agent
        .lineage_instructions()
        .expect("plan instructions")
        .contains("must not edit or write files"));

    agent
        .apply_collaboration_mode(CollaborationMode::Default, None)
        .expect("Default mode");
    let instructions = agent.lineage_instructions().expect("default instructions");
    assert!(instructions.contains("Collaboration Mode: Default"));
    assert!(!instructions.contains("must not edit or write files"));
}

#[test]
fn tool_restrictions_can_only_narrow_the_provider_catalog() {
    let mut agent = AgentCore::default();
    assert!(agent
        .provider_tool_candidates()
        .iter()
        .any(|candidate| candidate.name == "filesystem"));

    agent.restrict_to_tools(["filesystem", "shell"]);
    let names = agent
        .provider_tool_candidates()
        .into_iter()
        .map(|candidate| candidate.name)
        .collect::<HashSet<_>>();
    assert_eq!(
        names,
        HashSet::from(["filesystem".to_string(), "shell".to_string()])
    );

    agent.restrict_to_tools(["shell"]);
    assert!(agent.tool_is_allowed("shell"));
    assert!(!agent.tool_is_allowed("filesystem"));
}

#[test]
fn execution_context_projection_filters_catalog_and_execution_guard() {
    let mut agent = AgentCore::default();
    agent.restrict_capabilities(&CapabilityProjection::only_tools(["filesystem", "shell"]));
    let names = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();
    assert_eq!(
        names,
        HashSet::from(["filesystem".to_string(), "shell".to_string()])
    );
    assert!(!agent.tool_is_allowed("apply_patch"));

    agent.restrict_capabilities(&CapabilityProjection::only_tools(["shell"]));
    assert!(!agent.tool_is_allowed("filesystem"));
    assert!(agent.tool_is_allowed("shell"));
}

use std::fs;
use std::sync::Mutex;

struct ScriptedProvider {
    requests: Mutex<Vec<ModelRequest>>,
    responses: Mutex<VecDeque<ModelResponse>>,
}

struct SteerAfterParseProvider {
    inbox: Arc<dyn TurnInbox>,
    turn_id: Uuid,
    requests: Mutex<Vec<ModelRequest>>,
    rounds: AtomicUsize,
}

impl SteerAfterParseProvider {
    fn new(inbox: Arc<dyn TurnInbox>, turn_id: Uuid) -> Self {
        Self {
            inbox,
            turn_id,
            requests: Mutex::new(Vec::new()),
            rounds: AtomicUsize::new(0),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl ScriptedProvider {
    fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into()),
        }
    }

    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

fn rollout_tool_response(round: usize) -> ModelResponse {
    ModelResponse {
        text: String::new(),
        tool_calls: vec![ProviderToolCall {
            id: format!("rollout-list-{round}"),
            name: "filesystem".to_string(),
            arguments: json!({ "operation": "list", "path": "." }),
        }],
        usage: None,
        response_id: None,
        provider_items: Vec::new(),
        finish_reason: ModelFinishReason::ToolCalls,
    }
}

#[async_trait::async_trait]
impl ModelProvider for ScriptedProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        self.requests.lock().expect("requests lock").push(request);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("no scripted response"))
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        Ok(ProviderHealthCheck {
            reachable: true,
            latency_ms: None,
            model_available: true,
            error: None,
            openai_compatibility: None,
        })
    }
}

#[async_trait::async_trait]
impl ModelProvider for SteerAfterParseProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        self.requests.lock().expect("requests lock").push(request);
        if self.rounds.fetch_add(1, Ordering::SeqCst) == 0 {
            self.inbox.push(
                self.turn_id,
                TurnInboxItem::Steer {
                    message_id: Uuid::new_v4(),
                    content: "Do not write the file; explain the safer path instead.".into(),
                },
            );
            return Ok(ModelResponse {
                text: String::new(),
                tool_calls: vec![ProviderToolCall {
                    id: "discarded-write".into(),
                    name: "filesystem".into(),
                    arguments: json!({
                        "operation": "write",
                        "path": "must-not-exist.txt",
                        "content": "stale"
                    }),
                }],
                usage: None,
                response_id: None,
                provider_items: Vec::new(),
                finish_reason: ModelFinishReason::ToolCalls,
            });
        }
        Ok(ModelResponse::text(
            "I incorporated the steering message without executing the stale write.",
        ))
    }

    async fn check_health(&self) -> anyhow::Result<ProviderHealthCheck> {
        Ok(ProviderHealthCheck {
            reachable: true,
            latency_ms: None,
            model_available: true,
            error: None,
            openai_compatibility: None,
        })
    }
}

#[test]
fn parallel_selection_supports_mutations_and_skips_resource_conflicts() {
    let workspace = test_workspace("parallel-batch-selection");
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert_mcp(
        "parallel_observation_test".to_string(),
        Arc::new(ParallelObservationTestTool {
            active: Arc::new(AtomicUsize::new(0)),
            max_active: Arc::new(AtomicUsize::new(0)),
        }),
    );
    let agent = AgentCore::new(Arc::new(MockProvider), registry);
    let read = |id: &str, path: &str| ProviderToolCall {
        id: id.to_string(),
        name: "filesystem".to_string(),
        arguments: json!({ "operation": "read", "path": path }),
    };

    assert_eq!(
        agent.parallel_tool_call_indices(
            &[read("a", "a.txt"), read("b", "b.txt")],
            &workspace,
            PermissionMode::Approve,
        ),
        vec![0, 1]
    );
    assert_eq!(
        agent.parallel_tool_call_indices(
            &[
                ProviderToolCall {
                    id: "mcp-a".to_string(),
                    name: "parallel_observation_test".to_string(),
                    arguments: json!({ "resource": "shared" }),
                },
                ProviderToolCall {
                    id: "mcp-b".to_string(),
                    name: "parallel_observation_test".to_string(),
                    arguments: json!({ "resource": "shared" }),
                },
            ],
            &workspace,
            PermissionMode::Approve,
        ),
        vec![0, 1]
    );
    assert_eq!(
        agent.parallel_tool_call_indices(
            &[
                read("a", "same.txt"),
                read("b", "same.txt"),
                read("c", "other.txt"),
            ],
            &workspace,
            PermissionMode::Approve,
        ),
        vec![0, 1, 2]
    );
    assert_eq!(
        agent.parallel_tool_call_indices(
            &[read("outside", "../outside.txt"), read("b", "b.txt")],
            &workspace,
            PermissionMode::Approve,
        ),
        vec![1]
    );
    assert_eq!(
        agent.parallel_tool_call_indices(
            &[
                ProviderToolCall {
                    id: "write-a".to_string(),
                    name: "filesystem".to_string(),
                    arguments: json!({
                        "operation": "write",
                        "path": "a.txt",
                        "content": "changed"
                    }),
                },
                ProviderToolCall {
                    id: "write-b".to_string(),
                    name: "filesystem".to_string(),
                    arguments: json!({
                        "operation": "write",
                        "path": "b.txt",
                        "content": "changed"
                    }),
                },
            ],
            &workspace,
            PermissionMode::FullAccess,
        ),
        vec![0, 1]
    );
    assert_eq!(
        agent.parallel_tool_call_indices(
            &[
                ProviderToolCall {
                    id: "write-a".to_string(),
                    name: "filesystem".to_string(),
                    arguments: json!({
                        "operation": "write",
                        "path": "same.txt",
                        "content": "a"
                    }),
                },
                ProviderToolCall {
                    id: "write-b".to_string(),
                    name: "filesystem".to_string(),
                    arguments: json!({
                        "operation": "write",
                        "path": "same.txt",
                        "content": "b"
                    }),
                },
            ],
            &workspace,
            PermissionMode::FullAccess,
        ),
        vec![0]
    );
    assert_eq!(
        agent.parallel_tool_call_indices(
            &[
                ProviderToolCall {
                    id: "shell-a".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "git status --short" }),
                },
                ProviderToolCall {
                    id: "shell-b".to_string(),
                    name: "shell".to_string(),
                    arguments: json!({ "command": "git log -1 --oneline" }),
                },
            ],
            &workspace,
            PermissionMode::Approve,
        ),
        vec![0, 1]
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn approved_path_lease_survives_turn_state_serialization_without_widening_scope() {
    let id = Uuid::new_v4();
    let workspace = test_workspace("turn-path-lease");
    let outside = std::env::temp_dir().join(format!("opentopia-turn-path-lease-{id}"));
    fs::create_dir_all(&outside).expect("create external lease fixture");
    let approved = outside.join("approved.txt");
    let sibling = outside.join("sibling.txt");
    fs::write(&approved, "approved").expect("create approved file");
    fs::write(&sibling, "sibling").expect("create sibling file");

    let agent = AgentCore::new(Arc::new(MockProvider), ToolRegistry::with_core_tools())
        .with_sandbox_config(LocalSandboxConfig::enforce());
    let call = ProviderToolCall {
        id: "approved-read".to_string(),
        name: "filesystem".to_string(),
        arguments: json!({
            "operation": "read",
            "path": approved.display().to_string()
        }),
    };
    let mut runtime_state = TurnRuntimeState::default();
    agent
        .grant_turn_path_leases(&mut runtime_state, std::slice::from_ref(&call), &workspace)
        .expect("grant exact turn path lease");

    let serialized = serde_json::to_value(&runtime_state).expect("serialize turn state");
    let restored: TurnRuntimeState =
        serde_json::from_value(serialized).expect("restore turn state");
    let sandbox = restored.sandbox_config_with_path_leases(&agent.tool_host.sandbox_config);
    let policy = BasicPolicyEngine::new_with_sandbox_config(
        workspace.clone(),
        PermissionMode::Auto,
        &sandbox,
    );
    assert!(matches!(
        policy.inspect_read(&approved),
        PolicyDecision::Allow
    ));
    assert!(matches!(
        policy.inspect_read(&sibling),
        PolicyDecision::Allow
    ));
    assert!(sandbox.is_within_approved_read_scope(&approved));
    assert!(!sandbox.is_within_approved_read_scope(&sibling));

    fs::remove_dir_all(workspace).expect("remove lease workspace");
    fs::remove_dir_all(outside).expect("remove external lease fixture");
}
