use super::*;
use schemars::{schema::RootSchema, schema_for, JsonSchema};
use serde_json::Value;
use std::fs;

const HTTP_SCHEMA_FILE: &str = "desktop-http-v1.schema.json";
const HTTP_FIXTURE_FILE: &str = "desktop-http-v1.fixture.json";

#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
struct DesktopHttpResponsesV1 {
    health: HealthResponse,
    get_settings: AppSettings,
    update_settings: AppSettings,
    get_windows_sandbox_setup: WindowsSandboxSetupStatus,
    setup_windows_sandbox: WindowsSandboxSetupStatus,
    remove_windows_sandbox: WindowsSandboxSetupStatus,
    list_library_providers: Vec<library_api::LibraryProviderDescriptor>,
    get_library_provider_status: library_api::LibraryProviderStatus,
    list_library_sources: library_api::LibrarySourcePageView,
    search_library: library_api::LibrarySearchResponseView,
    ingest_sag_text: library_api::LibraryIngestionResponseView,
    upload_library_source: library_api::LibraryIngestionResponseView,
    list_skills: Vec<SkillDescriptor>,
    list_plugins: Vec<PluginView>,
    install_plugin: PluginView,
    uninstall_plugin: DeleteResponse,
    set_thread_plugin: PluginView,
    list_provider_drivers: Vec<ProviderDriverDescriptor>,
    get_provider_health: Vec<ProviderHealth>,
    test_provider_connection: ProviderHealthCheck,
    get_codex_account: CodexAccountStatus,
    start_codex_login: CodexLoginStart,
    cancel_codex_login: DeleteResponse,
    logout_codex_account: DeleteResponse,
    sync_provider_models: ProviderModelSyncResult,
    get_plugin_detail: plugins_api::PluginDetailResponse,
    set_plugin_activation: plugins_api::PluginActivationResponse,
    get_plugin_settings: plugins_api::PluginSettingsResponse,
    update_plugin_settings: plugins_api::PluginSettingsResponse,
    get_plugin_permissions: plugins_api::PluginPermissionsResponse,
    set_plugin_permission: opentopia_core::PluginPermissionGrantRecord,
    get_plugin_contributions: Vec<opentopia_core::PluginContributionRecord>,
    get_plugin_health: Vec<opentopia_core::PluginRuntimeHealthRecord>,
    get_thread_capabilities: plugins_api::ThreadCapabilitiesResponse,
    list_agent_templates: Vec<agent_templates_api::AgentTemplateVersionView>,
    create_agent_template_version: agent_templates_api::AgentTemplateVersionView,
    publish_agent_template_version: agent_templates_api::AgentTemplateVersionView,
    delete_agent_template_version: DeleteResponse,
    archive_agent_template: DeleteResponse,
    create_agent_instance: agent_templates_api::CreateAgentInstanceResponse,
    list_thread_agent_instances: Vec<AgentInstanceV1>,
    get_bound_thread_agent_instance: Option<AgentInstanceV1>,
    bind_thread_agent_instance: AgentInstanceV1,
    update_agent_instance: AgentInstanceV1,
    search_flows: Vec<opentopia_core::FlowDefinitionV1>,
    list_flow_drafts: Vec<flows_api::FlowDraftView>,
    get_thread_flow_draft: Option<flows_api::FlowDraftView>,
    create_flow_draft: flows_api::FlowDraftView,
    update_flow_draft: flows_api::FlowDraftView,
    validate_flow_draft: flows_api::FlowDraftView,
    simulate_flow_draft: opentopia_core::FlowTrialV1,
    publish_flow_draft: opentopia_core::FlowDefinitionV1,
    list_flow_runs: Vec<opentopia_core::FlowRunV1>,
    get_flow_run: opentopia_core::FlowRunV1,
    start_flow_run: opentopia_core::FlowRunV1,
    pause_flow_run: opentopia_core::FlowRunV1,
    resume_flow_run: opentopia_core::FlowRunV1,
    cancel_flow_run: opentopia_core::FlowRunV1,
    list_human_tasks: Vec<opentopia_core::HumanTaskV1>,
    resolve_human_task: human_tasks_api::ResolveHumanTaskResponse,
    get_contribution_hosts: contributions_api::ContributionHostSnapshot,
    select_preview_handler: opentopia_core::MediaHandlerSelection,
    select_context_loader: opentopia_core::MediaHandlerSelection,
    invoke_media_handler: contributions_api::MediaHandlerInvocationResponse,
    start_plugin_app_session: contributions_api::AppViewSessionResponse,
    post_plugin_app_message: opentopia_core::AppViewMessage,
    stop_plugin_app_session: opentopia_core::AppViewSession,
    execute_local_git: opentopia_core::LocalGitV1Response,
    get_scm_remote_connector: scm_api::ScmRemoteConnectorResponse,
    set_scm_remote_connector: scm_api::ScmRemoteConnectorResponse,
    set_thread_model: opentopia_core::Thread,
    list_projects: Vec<opentopia_core::Project>,
    create_project: opentopia_core::Project,
    update_project: opentopia_core::Project,
    delete_project: DeleteResponse,
    list_threads: Vec<opentopia_core::Thread>,
    create_thread: opentopia_core::Thread,
    generate_thread_title: GenerateThreadTitleResponse,
    update_thread: opentopia_core::Thread,
    delete_thread: DeleteResponse,
    list_messages: Vec<opentopia_core::Message>,
    send_message: opentopia_core::Message,
    get_goal: Option<opentopia_core::GoalSnapshot>,
    update_goal: opentopia_core::GoalSnapshot,
    resume_external_action: ExternalActionResumeResponse,
    run_browser_command: opentopia_core::BrowserOutput,
    get_browser_runtime: BrowserRuntimeStatus,
    bind_browser_runtime: BrowserRuntimeStatus,
    list_computer_windows: Vec<opentopia_core::WindowTarget>,
    observe_computer_window: opentopia_core::ComputerObservation,
    close_computer_session: DeleteResponse,
    get_turn_status: Option<opentopia_core::TurnRecord>,
    list_agents: Vec<AgentListItem>,
    interrupt_agent: DeleteResponse,
    cancel_turn: TurnCancelResult,
    list_events: Vec<opentopia_core::AgentEvent>,
    list_conversation_events: Vec<opentopia_core::AgentEvent>,
    start_terminal_command: terminal_api::TerminalStartResponse,
    cancel_terminal_command: terminal_api::TerminalCancelResponse,
    list_terminal_history: Vec<opentopia_core::TerminalEvent>,
    get_terminal_session: Option<terminal_api::TerminalSessionResponse>,
    ensure_terminal_session: terminal_api::TerminalSessionResponse,
    write_terminal_session: terminal_api::TerminalSessionResponse,
    resize_terminal_session: terminal_api::TerminalSessionResponse,
    close_terminal_session: terminal_api::TerminalSessionResponse,
    decide_approval: ApprovalDecisionResponse,
    list_pending_approvals: Vec<opentopia_core::Approval>,
    list_pending_user_input: Vec<opentopia_core::UserInputRecord>,
    respond_to_user_input: UserInputResponseAccepted,
    list_workspace_tree: opentopia_core::WorkspaceTree,
    read_workspace_file: opentopia_core::WorkspaceFilePreview,
    get_workspace_diff: opentopia_core::WorkspaceDiff,
    get_turn_changes: opentopia_core::TurnChangeSet,
    get_turn_file_diff_preview: TurnFileDiffPreview,
    preview_turn_undo: TurnUndoPreview,
    undo_turn_changes: TurnUndoResult,
    run_git_workflow: GitWorkflowResponse,
    revert_workspace_file: WorkspaceDiffActionResponse,
    apply_workspace_diff_hunk: WorkspaceDiffActionResponse,
    get_sandbox: opentopia_core::SandboxDescriptor,
    get_context_status: ContextStatusResponse,
    compact_context: opentopia_core::ContextSummary,
    list_artifacts: Vec<opentopia_core::ArtifactMetadata>,
    get_artifact: opentopia_core::Artifact,
    resolve_preview: opentopia_core::PreviewDescriptor,
    get_resource_metadata: opentopia_core::PreviewDescriptor,
    write_resource_content: opentopia_core::PreviewDescriptor,
    get_spreadsheet_preview: opentopia_core::PreviewWorkbook,
    get_spreadsheet_preview_range: opentopia_core::PreviewRange,
    close_preview: ResourceReleaseResponse,
    list_mcp_servers: Vec<McpServerView>,
    list_mcp_tools: Vec<opentopia_core::McpToolDescriptor>,
    create_mcp_server: McpServerView,
    update_mcp_server: McpServerView,
    delete_mcp_server: DeleteResponse,
    list_thread_mcp_servers: Vec<ThreadMcpServerView>,
    set_thread_mcp_server: opentopia_core::ThreadMcpServer,
    call_mcp_tool: opentopia_core::McpCallResult,
    restart_mcp_server: opentopia_core::McpServerStatus,
}

pub(super) fn generate(output_dir: &FsPath, check: bool) -> anyhow::Result<()> {
    if !check {
        fs::create_dir_all(output_dir)?;
    }
    let encoded = encode_schema(&schema_for!(DesktopHttpResponsesV1))?;
    write_or_check(output_dir.join(HTTP_SCHEMA_FILE), &encoded, check)?;
    let mut fixture = serde_json::to_string_pretty(&health_fixture())?;
    fixture.push('\n');
    write_or_check(output_dir.join(HTTP_FIXTURE_FILE), &fixture, check)?;
    Ok(())
}

fn write_or_check(path: PathBuf, content: &str, check: bool) -> anyhow::Result<()> {
    if check {
        if fs::read_to_string(&path).ok().as_deref() != Some(content) {
            anyhow::bail!(
                "generated Desktop HTTP contract is stale: {}",
                path.display()
            );
        }
    } else {
        fs::write(path, content)?;
    }
    Ok(())
}

fn health_fixture() -> HealthResponse {
    HealthResponse {
        ok: true,
        service: "opentopia-server",
        api_version: 1,
        shell_runtime: opentopia_core::ShellRuntimeStatus {
            runtime: opentopia_core::ShellRuntime {
                program: PathBuf::from("pwsh"),
                dialect: opentopia_core::ShellDialect::PowerShell7,
                version: Some("7.4.0".to_string()),
                source: opentopia_core::ShellRuntimeSource::Path,
            },
            managed_version: "7.4.0",
            managed_status: opentopia_core::ManagedPowerShellStatus::NotRequired,
            managed_error: None,
        },
    }
}

fn encode_schema(schema: &RootSchema) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(schema)?;
    value
        .as_object_mut()
        .expect("a root JSON Schema is always an object")
        .insert("additionalProperties".to_string(), Value::Bool(false));
    normalize_schema(&mut value);
    let mut encoded = serde_json::to_string_pretty(&value)?;
    encoded.push('\n');
    Ok(encoded)
}

fn normalize_schema(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                normalize_schema(item);
            }
        }
        Value::Object(object) => {
            if object.get("format").and_then(Value::as_str) == Some("uuid") {
                object.remove("default");
            }
            for child in object.values_mut() {
                normalize_schema(child);
            }
            object.sort_keys();
        }
        _ => {}
    }
}
