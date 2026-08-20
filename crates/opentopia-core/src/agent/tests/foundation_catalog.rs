#[test]
fn rebinding_a_clone_does_not_mutate_the_template_guardian() {
    let template = AgentCore::default();
    let original_guardian = template.guardian.clone();
    let configured = template
        .clone()
        .with_guardian_provider(Arc::new(MockProvider));

    assert!(Arc::ptr_eq(&template.guardian, &original_guardian));
    assert!(!Arc::ptr_eq(&template.guardian, &configured.guardian));
}

struct CatalogTestTool {
    name: String,
    description: String,
}

struct JournalTestTool {
    executions: Arc<AtomicUsize>,
    requires_approval: bool,
}

struct JournalChainedFailureTool;

struct ParallelObservationTestTool {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

struct ParallelProcessTestTool {
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

#[test]
fn turn_events_coalesce_adjacent_stream_fragments_before_persistence() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut events = TurnEvents::new(Some(sender));
    for _ in 0..1_000 {
        events.push(AgentEventPayload::ReasoningDelta {
            text: "片段".to_string(),
        });
    }

    let events = events.into_vec();
    let reasoning = events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ReasoningDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(reasoning, "片段".repeat(1_000));
    assert!(events.len() < 10, "stream fragments should be coalesced");

    let published = std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(published.len(), events.len());
}

#[test]
fn turn_events_flush_stream_fragments_before_semantic_events() {
    let mut events = TurnEvents::new(None);
    events.push(AgentEventPayload::ReasoningDelta {
        text: "reasoning".to_string(),
    });
    events.push(AgentEventPayload::ModelDelta {
        text: "answer".to_string(),
    });
    events.push(AgentEventPayload::ContextWarning {
        stage: "test".to_string(),
        message: "boundary".to_string(),
    });

    let events = events.into_vec();
    assert!(matches!(
        &events[0],
        AgentEventPayload::ReasoningDelta { text } if text == "reasoning"
    ));
    assert!(matches!(
        &events[1],
        AgentEventPayload::ModelDelta { text } if text == "answer"
    ));
    assert!(matches!(
        &events[2],
        AgentEventPayload::ContextWarning { message, .. } if message == "boundary"
    ));
}

#[test]
fn execution_failures_preserve_preflight_and_started_semantics() {
    let prepare_error = anyhow::Error::new(ExecutionFailure::without_os_error(
        ExecutionStage::PrepareSandbox,
        "dedicated-user backend is not configured",
    ));
    let mut prepare_metadata = json!({});
    insert_classified_anyhow_error_record(&mut prepare_metadata, &prepare_error);
    assert_eq!(
        prepare_metadata["errorRecord"]["code"],
        "sandbox_preparation_failed"
    );
    assert_eq!(prepare_metadata["errorRecord"]["phase"], "preflight");
    assert_eq!(prepare_metadata["errorRecord"]["executed"], false);
    assert_eq!(prepare_metadata["errorRecord"]["retryable"], true);
    assert_eq!(prepare_metadata["executionStage"], "prepare_sandbox");

    let wait_error = anyhow::Error::new(ExecutionFailure::without_os_error(
        ExecutionStage::Wait,
        "process wait failed",
    ));
    let mut wait_metadata = json!({});
    insert_classified_anyhow_error_record(&mut wait_metadata, &wait_error);
    assert_eq!(wait_metadata["errorRecord"]["code"], "process_wait_failed");
    assert_eq!(wait_metadata["errorRecord"]["executed"], true);
    assert_eq!(wait_metadata["errorRecord"]["retryable"], false);
}

#[test]
fn cache_lineage_ignores_turn_context_and_tool_catalog_but_cursor_compatibility_does_not() {
    let workspace = test_workspace("cache-lineage");
    let mut context =
        default_agent_model_context(&workspace, &LocalSandboxConfig::danger_full_access());
    context.prompt_cache_key = Some("custom-routing-namespace".to_string());
    let tools = vec![ProviderToolCandidate::direct(
        "filesystem",
        "Perform structured filesystem operations",
        json!({ "type": "object" }),
    )];
    let baseline = prompt_cache_lineage_key(&context, None, &tools);
    let baseline_compatibility = provider_compatibility_hash(&context, None, &tools, None);
    let mut data_wrapped_header = context.clone();
    data_wrapped_header
        .items
        .iter_mut()
        .find(|item| item.source == "opentopia:workspace_scope")
        .expect("workspace scope")
        .authority = ContextAuthority::Data;
    assert_ne!(
        baseline,
        prompt_cache_lineage_key(&data_wrapped_header, None, &tools)
    );
    assert_eq!(
        baseline,
        prompt_cache_lineage_key(
            &context,
            Some("Active task plan:\n[>] changing plan state"),
            &tools,
        )
    );
    assert_ne!(
        baseline,
        prompt_cache_lineage_key(
            &context,
            Some("Compacted prior history\n\nActive task plan:\n[>] current"),
            &tools,
        )
    );

    context.items.push(ModelContextItem::text(
        ContextItemKind::WorldState,
        ContextRole::Developer,
        "opentopia:world_state",
        "changing date and git status",
        ContextCacheScope::Turn,
        ContextSensitivity::Workspace,
    ));
    assert_eq!(baseline, prompt_cache_lineage_key(&context, None, &tools));
    assert_eq!(
        baseline_compatibility,
        provider_compatibility_hash(&context, None, &tools, None)
    );

    context.items.push(ModelContextItem::text(
        ContextItemKind::DeveloperInstructions,
        ContextRole::Developer,
        "opentopia:execution_lineage",
        "branch policy",
        ContextCacheScope::Thread,
        ContextSensitivity::Workspace,
    ));
    assert_ne!(baseline, prompt_cache_lineage_key(&context, None, &tools));
    assert_ne!(
        baseline_compatibility,
        provider_compatibility_hash(&context, None, &tools, None)
    );
    assert_eq!(
        prompt_cache_lineage_key(&context, None, &tools),
        prompt_cache_lineage_key(
            &context,
            None,
            &[ProviderToolCandidate::direct(
                "apply_patch",
                "Apply a workspace patch",
                json!({ "type": "object" }),
            )],
        )
    );
    assert_ne!(
        provider_compatibility_hash(&context, None, &tools, None),
        provider_compatibility_hash(
            &context,
            None,
            &[ProviderToolCandidate::direct(
                "apply_patch",
                "Apply a workspace patch",
                json!({ "type": "object" }),
            )],
            None,
        )
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn model_request_rejects_invalid_context_classification() {
    let context = CompiledModelContext {
        items: vec![ModelContextItem::text(
            ContextItemKind::BaseInstructions,
            ContextRole::System,
            "opentopia:base",
            "Base policy",
            ContextCacheScope::Stable,
            ContextSensitivity::Public,
        )
        .with_semantics(ContextAuthority::Data, ContextLifecycle::Build)],
        prompt_cache_key: None,
    };

    let error = build_model_request(
        &context,
        None,
        Vec::new(),
        "Question".to_string(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        None,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("base_instructions items require system authority"));
}

#[test]
fn later_round_token_estimates_use_observed_provider_calibration() {
    let mut events = TurnEvents::new(None);
    let mut breakdown = crate::model_context::TokenEstimateBreakdown::default();
    breakdown.current_user = 100;
    breakdown.recalculate_total();
    events.push(AgentEventPayload::TokenUsage {
        request_id: Some(Uuid::new_v4()),
        round: Some(1),
        purpose: ModelCallPurpose::AgentRound,
        input_tokens: 120,
        output_tokens: 10,
        total_tokens: 130,
        cached_input_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
        local_input_estimate: Some(100),
        input_breakdown: Some(breakdown),
    });

    assert_eq!(calibrated_input_estimate(&events, 50), 60);
}

#[async_trait]
impl Tool for JournalTestTool {
    fn name(&self) -> &str {
        "journal_test"
    }

    fn description(&self) -> &str {
        "Exercise durable tool-call journaling in tests."
    }

    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn execution_policy(&self, _call: &ToolCall) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: false,
            side_effect: ToolSideEffect::External,
            resource_keys: vec!["journal-test".to_string()],
        }
    }

    async fn execute(
        &self,
        call: ToolCall,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        if self.requires_approval && !ctx.approval_granted {
            return Err(ApprovalRequired::new("approve journal test").into());
        }
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(ToolResult::text(
            call.id,
            "executed",
            json!({ "success": true }),
        ))
    }
}

#[async_trait]
impl Tool for JournalChainedFailureTool {
    fn name(&self) -> &str {
        "journal_chained_failure"
    }

    fn description(&self) -> &str {
        "Return a chained read-only execution error for journal tests."
    }

    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn execution_policy(&self, _call: &ToolCall) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec!["git:index-and-worktree".to_string()])
    }

    async fn execute(
        &self,
        _call: ToolCall,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        Err(anyhow::anyhow!("sandbox process creation was denied")
            .context("git diff execution failed"))
    }
}

#[async_trait]
impl Tool for ParallelObservationTestTool {
    fn name(&self) -> &str {
        "parallel_observation_test"
    }

    fn description(&self) -> &str {
        "Test-only bounded read-only observation."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "resource": { "type": "string" } },
            "required": ["resource"],
            "additionalProperties": false
        })
    }

    fn execution_policy(&self, call: &ToolCall) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![format!(
            "test:{}",
            call.input
                .get("resource")
                .and_then(Value::as_str)
                .unwrap_or("*")
        )])
    }

    fn authorization_preflight(
        &self,
        _call: &ToolCall,
        _ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        Some(PolicyDecision::Allow)
    }

    async fn execute(
        &self,
        call: ToolCall,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(ToolResult::text(
            call.id,
            "observed",
            json!({ "success": true }),
        ))
    }
}

#[async_trait]
impl Tool for ParallelProcessTestTool {
    fn name(&self) -> &str {
        "parallel_process_test"
    }

    fn description(&self) -> &str {
        "Test-only parallel process with a declared logical resource."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "resource": { "type": "string" } },
            "required": ["resource"],
            "additionalProperties": false
        })
    }

    fn execution_policy(&self, call: &ToolCall) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: true,
            side_effect: ToolSideEffect::Process,
            resource_keys: vec![format!(
                "test-process:{}",
                call.input
                    .get("resource")
                    .and_then(Value::as_str)
                    .unwrap_or("*")
            )],
        }
    }

    fn authorization_preflight(
        &self,
        _call: &ToolCall,
        _ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        Some(PolicyDecision::Allow)
    }

    async fn execute(
        &self,
        call: ToolCall,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(ToolResult::text(
            call.id,
            "processed",
            json!({ "success": true }),
        ))
    }
}

fn journal_test_context(
    store: Arc<dyn SessionStore>,
    thread_id: Uuid,
    workspace: PathBuf,
    approval_granted: bool,
) -> ToolInvocationContext {
    let policy = Arc::new(BasicPolicyEngine::new(
        workspace.clone(),
        PermissionMode::FullAccess,
    ));
    let mut ctx = ToolInvocationContext::local(workspace, policy);
    ctx.state = Some(ToolStateStore::new(store));
    ctx.thread_id = Some(thread_id);
    ctx.approval_granted = approval_granted;
    ctx
}

#[async_trait]
impl Tool for CatalogTestTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn execute(
        &self,
        call: ToolCall,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::text(call.id, "ok", json!({ "success": true })))
    }
}

#[test]
fn progressive_tool_search_reveals_only_matching_deferred_schemas() {
    let mut registry = ToolRegistry::with_builtins();
    registry.insert_mcp(
        "mcp_issue_lookup".to_string(),
        Arc::new(CatalogTestTool {
            name: "mcp_issue_lookup".to_string(),
            description: "Look up issue tracker records".to_string(),
        }),
    );
    registry.insert_mcp(
        "mcp_invoice_send".to_string(),
        Arc::new(CatalogTestTool {
            name: "mcp_invoice_send".to_string(),
            description: "Send a customer invoice".to_string(),
        }),
    );
    let mut agent = AgentCore::new(Arc::new(MockProvider), registry);
    agent.set_tool_exposure_policy(ToolExposurePolicy::Progressive);
    let mut exposed = agent.provider_tool_catalog();
    assert!(exposed.iter().any(|tool| tool.name == TOOL_SEARCH_NAME));
    assert!(!exposed.iter().any(|tool| tool.name == "mcp_issue_lookup"));
    assert!(!exposed.iter().any(|tool| tool.name == "mcp_invoice_send"));

    let mut events = TurnEvents::new(None);
    let result = agent
        .execute_tool_search_call(
            &ProviderToolCall {
                id: "search-tools".to_string(),
                name: TOOL_SEARCH_NAME.to_string(),
                arguments: json!({ "query": "issue tracker" }),
            },
            &mut events,
        )
        .expect("search deferred tools");
    assert!(agent.reveal_tools_from_search_result(&result, &mut exposed));
    assert!(exposed.iter().any(|tool| tool.name == "mcp_issue_lookup"));
    assert!(!exposed.iter().any(|tool| tool.name == "mcp_invoice_send"));
}

#[test]
fn automatic_tool_disclosure_keeps_small_local_catalogs_eager() {
    let mut registry = ToolRegistry::with_builtins();
    registry.insert_mcp(
        "mcp_issue_lookup".to_string(),
        Arc::new(CatalogTestTool {
            name: "mcp_issue_lookup".to_string(),
            description: "Look up issue tracker records".to_string(),
        }),
    );
    let mut agent = AgentCore::new(Arc::new(MockProvider), registry);
    agent.disable_all_bundled_plugins();

    let catalog = agent.provider_tool_catalog();
    assert!(catalog
        .iter()
        .any(|candidate| candidate.name == "mcp_issue_lookup"));
    assert!(!catalog
        .iter()
        .any(|candidate| candidate.name == TOOL_SEARCH_NAME));
}

#[test]
fn default_office_schemas_are_eager_without_attachment_hints() {
    let mut agent = AgentCore::default();

    let baseline = agent.provider_tool_catalog();
    for tool in [
        "document",
        "pdf",
        "spreadsheet_inspect",
        "spreadsheet_describe",
        "spreadsheet_execute",
    ] {
        assert!(baseline.iter().any(|candidate| candidate.name == tool));
    }
    assert!(!baseline
        .iter()
        .any(|candidate| candidate.name == "spreadsheet"));
    assert!(!baseline
        .iter()
        .any(|candidate| candidate.name == TOOL_SEARCH_NAME));

    agent.set_attachment_preloaded_tools(["pdf", "spreadsheet_execute"]);
    let projected = agent.provider_tool_catalog();
    assert!(projected.iter().any(|candidate| candidate.name == "pdf"));
    assert!(projected
        .iter()
        .any(|candidate| candidate.name == "spreadsheet_execute"));
    assert!(projected
        .iter()
        .any(|candidate| candidate.name == "document"));
    assert!(!projected
        .iter()
        .any(|candidate| candidate.name == TOOL_SEARCH_NAME));
    assert_eq!(baseline, projected);
}

#[test]
fn eager_disclosure_cannot_expose_internal_legacy_spreadsheet_executor() {
    let mut agent = AgentCore::default();
    agent.set_tool_exposure_policy(ToolExposurePolicy::Eager);

    let catalog = agent.provider_tool_catalog();
    assert!(!catalog
        .iter()
        .any(|candidate| candidate.name == "spreadsheet"));
    for tool in [
        "spreadsheet_inspect",
        "spreadsheet_describe",
        "spreadsheet_execute",
    ] {
        assert!(catalog.iter().any(|candidate| candidate.name == tool));
    }
}

#[test]
fn attachment_projection_cannot_enable_a_disabled_bundled_plugin() {
    let mut agent = AgentCore::default();
    agent.set_bundled_plugin_activations(&HashMap::from([
        ("pdf".to_string(), false),
        ("spreadsheet".to_string(), true),
    ]));
    agent.set_attachment_preloaded_tools(["pdf"]);

    assert!(!agent
        .provider_tool_catalog()
        .iter()
        .any(|candidate| candidate.name == "pdf"));
    assert!(!agent.tool_is_allowed("pdf"));
}

#[test]
fn native_deferred_loading_keeps_default_office_tools_direct() {
    let mut agent = AgentCore::default();
    agent.provider_tool_protocol = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        deferred_tool_loading: ProviderFeatureSupport::Supported,
        namespace_tools: ProviderFeatureSupport::Supported,
        hosted_tool_search: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };

    let pdf = agent
        .provider_tool_catalog()
        .into_iter()
        .find(|candidate| candidate.name == "pdf")
        .expect("eligible PDF tool descriptor");
    assert_eq!(pdf.disclosure, ProviderToolDisclosure::Direct);
}

#[test]
fn automatic_tool_disclosure_defers_large_local_catalogs() {
    let mut registry = ToolRegistry::with_builtins();
    for index in 0..AUTOMATIC_TOOL_DISCLOSURE_COUNT_THRESHOLD {
        let name = format!("mcp_catalog_tool_{index}");
        registry.insert_mcp(
            name.clone(),
            Arc::new(CatalogTestTool {
                name,
                description: format!("Inspect external catalog record {index}"),
            }),
        );
    }
    let agent = AgentCore::new(Arc::new(MockProvider), registry);

    let catalog = agent.provider_tool_catalog();
    assert!(catalog
        .iter()
        .any(|candidate| candidate.name == TOOL_SEARCH_NAME));
    assert!(!catalog
        .iter()
        .any(|candidate| candidate.name == "mcp_catalog_tool_0"));
}

#[test]
fn release_gate_native_tool_search_keeps_office_direct_and_defers_external_namespace() {
    let mut registry = ToolRegistry::with_builtins();
    registry.insert_mcp(
        "github__search_issues".to_string(),
        Arc::new(CatalogTestTool {
            name: "github__search_issues".to_string(),
            description: "Search GitHub issues".to_string(),
        }),
    );
    let mut agent = AgentCore::new(Arc::new(MockProvider), registry);
    agent.provider_tool_protocol = ProviderToolProtocolCapabilities {
        function_tools: ProviderFeatureSupport::Supported,
        deferred_tool_loading: ProviderFeatureSupport::Supported,
        namespace_tools: ProviderFeatureSupport::Supported,
        hosted_tool_search: ProviderFeatureSupport::Supported,
        ..ProviderToolProtocolCapabilities::default()
    };

    let catalog = agent.provider_tool_catalog();
    let filesystem = catalog
        .iter()
        .find(|candidate| candidate.name == "filesystem")
        .expect("common tool");
    assert_eq!(filesystem.disclosure, ProviderToolDisclosure::Direct);
    let github = catalog
        .iter()
        .find(|candidate| candidate.name == "github__search_issues")
        .expect("external tool descriptor");
    assert_eq!(github.disclosure, ProviderToolDisclosure::DeferredNamespace);
    assert_eq!(github.namespace.as_ref().unwrap().name, "github");
    for office in [
        "document",
        "pdf",
        "spreadsheet_inspect",
        "spreadsheet_describe",
        "spreadsheet_execute",
    ] {
        let candidate = catalog
            .iter()
            .find(|candidate| candidate.name == office)
            .expect("default Office tool");
        assert_eq!(candidate.disclosure, ProviderToolDisclosure::Direct);
    }
    assert!(!catalog
        .iter()
        .any(|candidate| candidate.name == TOOL_SEARCH_NAME));
}

#[test]
fn release_gate_mode_bundles_project_flow_plan_task_and_goal_tools() {
    let names = |agent: &AgentCore| {
        agent
            .provider_tool_catalog()
            .into_iter()
            .map(|candidate| candidate.name)
            .collect::<HashSet<_>>()
    };

    let mut code = AgentCore::default();
    code.apply_experience_mode(ExperienceMode::Code);
    let code_names = names(&code);
    assert!(!code_names.iter().any(|name| name.starts_with("flow_")));
    assert!(!code_names.contains("request_user_input"));
    assert!(code_names.contains("set_plan"));
    assert!(code_names.contains("update_plan"));
    assert!(!code_names.contains("complete_task"));

    let mut child = AgentCore::default();
    child.set_agent_context(Uuid::new_v4(), 1);
    let child_names = names(&child);
    assert!(!child_names.contains("set_plan"));
    assert!(!child_names.contains("update_plan"));
    assert!(!child_names.contains("complete_task"));

    let mut work = AgentCore::default();
    work.apply_experience_mode(ExperienceMode::Work);
    assert_eq!(code_names, names(&work));

    let mut flow = AgentCore::default();
    flow.apply_experience_mode(ExperienceMode::Flow);
    assert!(names(&flow).contains("flow_run"));

    let thread_id = Uuid::new_v4();
    let goal = GoalRecord::new(thread_id, "Execute a durable goal", None);
    let mut goal_agent = AgentCore::default();
    goal_agent
        .apply_collaboration_mode(CollaborationMode::Goal, Some(goal))
        .expect("Goal mode");
    let goal_names = names(&goal_agent);
    assert!(goal_names.contains("set_plan"));
    assert!(goal_names.contains("update_plan"));
    assert!(!goal_names.contains("complete_task"));
    assert!(!goal_names.contains("request_user_input"));

    let mut plan_agent = AgentCore::default();
    plan_agent
        .apply_collaboration_mode(CollaborationMode::Plan, None)
        .expect("Plan mode");
    assert!(names(&plan_agent).contains("request_user_input"));
}

#[test]
fn attachment_capability_backend_is_hidden_behind_view_attachment() {
    let public_name = "opaque_server__run";
    let mut registry = ToolRegistry::with_builtins();
    registry.insert_mcp(
        public_name.to_string(),
        Arc::new(CatalogTestTool {
            name: public_name.to_string(),
            description: "Process a supplied asset".to_string(),
        }),
    );
    let mut agent = AgentCore::new(Arc::new(MockProvider), registry);
    agent.tool_host.active_mcp_tools = vec![McpToolDescriptor {
        public_name: public_name.to_string(),
        server_id: Uuid::new_v4(),
        tool_name: "run".to_string(),
        description: Some("Process a supplied asset".to_string()),
        input_schema: json!({
            "type": "object",
            "properties": {
                "image": { "type": "object" },
                "focus": { "type": "string" }
            }
        }),
        annotations: json!({ "readOnlyHint": true }),
        meta: json!({
            "com.opentopia/capabilities": ["media.image.inspect/v1"]
        }),
        permission_labels: vec!["read".to_string()],
    }];

    let exposed = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<HashSet<_>>();
    assert!(exposed.contains("view_attachment"));
    assert!(!exposed.contains(public_name));
    assert!(agent
        .search_deferred_tools("asset image", 10)
        .iter()
        .all(|tool| tool.name != public_name));
}
