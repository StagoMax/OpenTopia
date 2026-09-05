use anyhow::Context;
use async_trait::async_trait;
use lopdf::{content, dictionary, Document, Object, Stream};
use opentopia_core::{
    tool_result_is_error, AgentCore, AgentEventPayload, AgentResumeSignal, AgentRunConfig,
    AgentRunIdentity, AgentTurnDriver, AgentTurnInput, AgentTurnOutcome, Artifact,
    BackgroundProcessRegistry, CapabilityProjection, CollaborationMode, ContextSourceKind,
    ContextSourceRef, ExecutionAuthority, LocalSandboxConfig, Message, MessagePart, MessageRole,
    ModelContentPart, ModelFinishReason, ModelProvider, ModelRequest, ModelResponse,
    PermissionMode, ProviderHealthCheck, ProviderToolCall, SessionStore, SqliteSessionStore, Tool,
    ToolCall, ToolExposurePolicy, ToolInvocationContext, ToolRegistry, ToolResult, UserInputAnswer,
    UserInputResponse,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

const EXPECTED_TOOLS: [&str; 28] = [
    "apply_patch",
    "background_output",
    "create_skill",
    "word_document",
    "filesystem",
    "list_skills",
    "pdf",
    "read_artifact",
    "read_attachment",
    "read_skill",
    "request_user_input",
    "shell",
    "spreadsheet_inspect",
    "spreadsheet_read_ranges",
    "spreadsheet_find",
    "spreadsheet_filter_rows",
    "spreadsheet_validate",
    "spreadsheet_write_range",
    "spreadsheet_copy_ranges",
    "spreadsheet_copy_rows",
    "spreadsheet_fill_ranges",
    "spreadsheet_convert_ranges",
    "spreadsheet_export_delimited",
    "spreadsheet_copy_sheet",
    "spreadsheet_delete_rows",
    "spreadsheet_delete_sheet",
    "update_plan",
    "view_attachment",
];

struct SmokeProvider {
    stage: Mutex<usize>,
    workspace: PathBuf,
    artifact_id: Uuid,
    text_attachment_id: Uuid,
    image_attachment_id: Uuid,
}

struct DeferredSmokeTool;

#[async_trait]
impl Tool for DeferredSmokeTool {
    fn name(&self) -> &str {
        "mcp_smoke_echo"
    }

    fn description(&self) -> &str {
        "Deferred smoke capability that returns the supplied marker."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "marker": { "type": "string" }
            },
            "required": ["marker"],
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        call: ToolCall,
        _ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let marker = call
            .input
            .get("marker")
            .and_then(Value::as_str)
            .context("mcp_smoke_echo marker is required")?;
        Ok(ToolResult::text(
            call.id,
            marker,
            json!({ "success": true }),
        ))
    }
}

#[derive(Default)]
struct DeferredSearchProvider {
    stage: Mutex<usize>,
}

impl SmokeProvider {
    fn next_stage(&self) -> usize {
        let mut stage = self.stage.lock().unwrap_or_else(|error| error.into_inner());
        let current = *stage;
        *stage += 1;
        current
    }

    fn one_call(stage: usize, name: &str, arguments: Value) -> ModelResponse {
        Self::calls(stage, vec![(name, arguments)])
    }

    fn calls(stage: usize, calls: Vec<(&str, Value)>) -> ModelResponse {
        ModelResponse {
            text: String::new(),
            tool_calls: calls
                .into_iter()
                .enumerate()
                .map(|(index, (name, arguments))| ProviderToolCall {
                    id: format!("smoke-{stage}-{index}-{name}"),
                    name: name.to_string(),
                    arguments,
                })
                .collect(),
            usage: None,
            response_id: None,
            provider_items: Vec::new(),
            finish_reason: ModelFinishReason::ToolCalls,
        }
    }

    fn background_job_id(request: &ModelRequest) -> anyhow::Result<String> {
        request
            .input
            .tool_results
            .iter()
            .rev()
            .find(|result| {
                result.name == "shell"
                    && result.metadata.get("background").and_then(Value::as_bool) == Some(true)
            })
            .and_then(|result| result.metadata.get("jobId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .context("background shell result did not expose jobId")
    }

    fn created_skill_id(&self) -> anyhow::Result<String> {
        let path = self
            .workspace
            .join(".agents/skills/topia-smoke/SKILL.md")
            .canonicalize()
            .context("created workspace Skill was not found")?;
        Ok(format!(
            "workspace:{}",
            path.to_string_lossy().replace('\\', "/")
        ))
    }
}

#[async_trait]
impl ModelProvider for SmokeProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        let response = match self.next_stage() {
            0 => Self::one_call(
                0,
                "filesystem",
                json!({
                    "operation": "write",
                    "path": "fs-smoke.txt",
                    "content": "TOPIA_FILESYSTEM_OK\n",
                    "expected_hash": "missing"
                }),
            ),
            1 => Self::calls(
                1,
                vec![
                    (
                        "filesystem",
                        json!({ "operation": "read", "path": "fs-smoke.txt" }),
                    ),
                    (
                        "filesystem",
                        json!({ "operation": "list", "path": ".", "limit": 50 }),
                    ),
                    (
                        "filesystem",
                        json!({
                            "operation": "find",
                            "path": ".",
                            "name_contains": "fs-smoke",
                            "case_sensitive": false,
                            "kind": "file",
                            "max_depth": 2,
                            "limit": 10
                        }),
                    ),
                ],
            ),
            2 => Self::one_call(
                2,
                "apply_patch",
                json!({
                    "operation": {
                        "type": "create_file",
                        "path": "patch-smoke.txt",
                        "diff": "+TOPIA_APPLY_PATCH_OK"
                    }
                }),
            ),
            3 => Self::one_call(
                3,
                "shell",
                json!({
                    "command": if cfg!(windows) {
                        "Write-Output 'TOPIA_SHELL_OK'"
                    } else {
                        "printf 'TOPIA_SHELL_OK\\n'"
                    }
                }),
            ),
            4 => Self::one_call(
                4,
                "shell",
                json!({
                    "command": if cfg!(windows) {
                        "Start-Sleep -Milliseconds 250; Write-Output 'TOPIA_BACKGROUND_OK'"
                    } else {
                        "sleep 0.25; printf 'TOPIA_BACKGROUND_OK\\n'"
                    },
                    "background": true,
                    "timeout_seconds": 10
                }),
            ),
            5 => Self::one_call(
                5,
                "background_output",
                json!({
                    "action": "read",
                    "job_id": Self::background_job_id(&request)?,
                    "timeout_ms": 5000
                }),
            ),
            6 => Self::one_call(
                6,
                "create_skill",
                json!({
                    "name": "topia-smoke",
                    "description": "Verify the real OpenTopia Skill tool path during smoke testing.",
                    "instructions": "Report the marker TOPIA_SKILL_OK.",
                    "scope": "workspace"
                }),
            ),
            7 => Self::one_call(7, "list_skills", json!({})),
            8 => Self::one_call(8, "read_skill", json!({ "id": self.created_skill_id()? })),
            9 => Self::one_call(
                9,
                "update_plan",
                json!({
                    "explanation": "Publish the initial smoke-test checklist.",
                    "plan": [{
                        "id": "exercise-tools",
                        "step": "Exercise the default tool surface",
                        "status": "in_progress",
                        "acceptance": ["All tool calls finish without a structured error."]
                    }]
                }),
            ),
            10 => Self::one_call(
                10,
                "update_plan",
                json!({
                    "explanation": "Record smoke-test evidence.",
                    "plan": [{
                        "id": "exercise-tools",
                        "step": "Exercise the default tool surface",
                        "status": "completed",
                        "acceptance": ["All tool calls finish without a structured error."],
                        "evidence_refs": ["smoke:default-tool-surface"]
                    }]
                }),
            ),
            11 => Self::one_call(
                11,
                "spreadsheet_write_range",
                json!({
                    "path": "smoke.xlsx",
                    "sheet": "Smoke",
                    "start": "A1",
                    "rows": [[{ "type": "string", "value": "TOPIA_SPREADSHEET_OK" }]]
                }),
            ),
            12 => Self::one_call(12, "spreadsheet_inspect", json!({ "path": "smoke.xlsx" })),
            13 => Self::one_call(
                13,
                "spreadsheet_read_ranges",
                json!({
                    "reads": [{
                        "path": "smoke.xlsx",
                        "sheet": "Smoke",
                        "range": "A1"
                    }]
                }),
            ),
            14 => Self::one_call(
                14,
                "word_document",
                json!({ "action": "inspect", "path": "sample.docx" }),
            ),
            15 => Self::one_call(
                15,
                "pdf",
                json!({ "action": "inspect", "path": "sample.pdf" }),
            ),
            16 => Self::one_call(
                16,
                "read_artifact",
                json!({ "artifact_id": self.artifact_id }),
            ),
            17 => Self::one_call(
                17,
                "read_attachment",
                json!({ "attachment_id": self.text_attachment_id }),
            ),
            18 => Self::one_call(
                18,
                "view_attachment",
                json!({
                    "attachment_id": self.image_attachment_id,
                    "focus": "Verify the smoke image is available."
                }),
            ),
            19 => Self::one_call(
                19,
                "request_user_input",
                json!({
                    "questions": [{
                        "id": "finish_smoke",
                        "header": "Smoke test",
                        "question": "Finish the default tool-surface smoke test?",
                        "options": [
                            {
                                "id": "finish",
                                "label": "Finish",
                                "description": "Resume the same Topia turn and complete the test.",
                                "recommended": true
                            },
                            {
                                "id": "stop",
                                "label": "Stop",
                                "description": "Leave the turn waiting for a different decision."
                            }
                        ],
                        "allow_custom": false
                    }]
                }),
            ),
            20 => ModelResponse::text("TOPIA_DEFAULT_TOOL_SURFACE_OK"),
            stage => anyhow::bail!("unexpected smoke-provider stage {stage}"),
        };
        Ok(response)
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

#[async_trait]
impl ModelProvider for DeferredSearchProvider {
    async fn complete(&self, request: ModelRequest) -> anyhow::Result<ModelResponse> {
        let mut stage = self.stage.lock().unwrap_or_else(|error| error.into_inner());
        let response = match *stage {
            0 => {
                anyhow::ensure!(request
                    .tool_candidates
                    .iter()
                    .any(|tool| tool.name == "tool_search"));
                anyhow::ensure!(!request
                    .tool_candidates
                    .iter()
                    .any(|tool| tool.name == "mcp_smoke_echo"));
                SmokeProvider::one_call(
                    100,
                    "tool_search",
                    json!({ "query": "deferred smoke capability" }),
                )
            }
            1 => {
                anyhow::ensure!(request
                    .input
                    .tool_results
                    .iter()
                    .any(|result| result.name == "tool_search" && !result.is_error));
                anyhow::ensure!(request
                    .tool_candidates
                    .iter()
                    .any(|tool| tool.name == "mcp_smoke_echo"));
                SmokeProvider::one_call(
                    101,
                    "mcp_smoke_echo",
                    json!({ "marker": "TOPIA_TOOL_SEARCH_AND_CALL_OK" }),
                )
            }
            2 => ModelResponse::text("TOPIA_DEFERRED_TOOL_OK"),
            other => anyhow::bail!("unexpected deferred-search stage {other}"),
        };
        *stage += 1;
        Ok(response)
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let workspace =
        std::env::temp_dir().join(format!("opentopia-tool-surface-smoke-{}", Uuid::new_v4()));
    fs::create_dir(&workspace).context("create isolated smoke workspace")?;
    let outcome = async {
        run_smoke(&workspace).await?;
        run_tool_search_smoke(&workspace).await
    }
    .await;
    match outcome {
        Ok(()) => {
            fs::remove_dir_all(&workspace).with_context(|| {
                format!("remove isolated smoke workspace {}", workspace.display())
            })?;
            Ok(())
        }
        Err(error) => {
            eprintln!(
                "Smoke workspace preserved for inspection: {}",
                workspace.display()
            );
            Err(error)
        }
    }
}

async fn run_tool_search_smoke(workspace: &Path) -> anyhow::Result<()> {
    let mut registry = ToolRegistry::with_core_tools();
    registry.insert_mcp("mcp_smoke_echo".to_string(), Arc::new(DeferredSmokeTool));
    let mut agent = AgentCore::new(Arc::new(DeferredSearchProvider::default()), registry);
    agent.set_tool_exposure_policy(ToolExposurePolicy::Progressive);
    let authority = ExecutionAuthority::new(
        workspace.to_path_buf(),
        PermissionMode::FullAccess,
        LocalSandboxConfig::danger_full_access(),
        CapabilityProjection::unrestricted(),
    )?;
    let agent = agent
        .begin_run(AgentRunConfig::using_current_provider(
            authority,
            AgentRunIdentity::root(Uuid::new_v4(), 1),
        ))?
        .finalize()?;

    let turn = agent.prepare_turn(
        AgentTurnInput {
            thread_id: Uuid::new_v4(),
            user_message_id: Uuid::new_v4(),
            workspace_root: workspace.to_path_buf(),
            content: "Discover and invoke the deferred smoke capability.".to_string(),
            user_content: Vec::new(),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: None,
            store: None,
            cancellation: None,
        },
        None,
    )?;
    let result = AgentTurnDriver::run_turn(&agent, turn, None).await?;
    anyhow::ensure!(matches!(result.outcome, AgentTurnOutcome::Completed));
    let names = result
        .events
        .iter()
        .filter_map(|event| match event {
            AgentEventPayload::ToolCallStarted { call } => Some(call.name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    anyhow::ensure!(names == ["tool_search", "mcp_smoke_echo"]);
    anyhow::ensure!(result.events.iter().any(|event| {
        matches!(
            event,
            AgentEventPayload::ToolCallFinished { result }
                if result.output.contains("TOPIA_TOOL_SEARCH_AND_CALL_OK")
        )
    }));
    println!("PASS tool_search -> deferred tool_call dispatch");
    Ok(())
}

async fn run_smoke(workspace: &Path) -> anyhow::Result<()> {
    fs::write(workspace.join("sample.pdf"), sample_pdf("TOPIA_PDF_OK"))?;
    fs::write(workspace.join("sample.docx"), sample_docx())?;
    fs::write(
        workspace.join("attachment.txt"),
        "TOPIA_READ_ATTACHMENT_OK\n",
    )?;

    let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::open(":memory:")?);
    let thread = store.create_thread(
        Some("Default tool-surface smoke".to_string()),
        workspace.to_path_buf(),
    )?;
    let artifact = store.insert_artifact(Artifact::inline(
        thread.id,
        "smoke_text",
        "text/plain",
        "TOPIA_READ_ARTIFACT_OK",
        json!({ "smoke": true }),
    ))?;

    let text_attachment_id = Uuid::new_v4();
    let image_attachment_id = Uuid::new_v4();
    let mut attachment_message = Message::text(
        thread.id,
        MessageRole::User,
        "Use these fixtures for the smoke test.",
    );
    attachment_message.parts.push(MessagePart::SourceRef {
        source: ContextSourceRef {
            id: text_attachment_id,
            path: workspace.join("attachment.txt"),
            name: "attachment.txt".to_string(),
            kind: ContextSourceKind::Text,
            content_type: "text/plain; charset=utf-8".to_string(),
            bytes: fs::metadata(workspace.join("attachment.txt"))?.len(),
            truncated: false,
        },
        inline: Some(false),
    });
    attachment_message.parts.push(MessagePart::Image {
        id: Some(image_attachment_id),
        content_type: "image/png".to_string(),
        data: sample_png()?,
        name: Some("smoke.png".to_string()),
    });
    store.append_message(attachment_message)?;

    let provider = Arc::new(SmokeProvider {
        stage: Mutex::new(0),
        workspace: workspace.to_path_buf(),
        artifact_id: artifact.id,
        text_attachment_id,
        image_attachment_id,
    });
    let registry = ToolRegistry::with_builtins();
    for removed_tool in [
        "list_files",
        "read_file",
        "read_files",
        "write_file",
        "search",
        "git_diff",
    ] {
        anyhow::ensure!(
            registry.get(removed_tool).is_none(),
            "legacy tool {removed_tool} is still registered"
        );
    }
    let mut agent = AgentCore::new(provider, registry);
    agent.set_background_processes(BackgroundProcessRegistry::default());

    let default_catalog = agent
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    let expected_default_catalog = EXPECTED_TOOLS
        .iter()
        .filter(|name| **name != "request_user_input")
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        default_catalog == expected_default_catalog,
        "default Code tool surface changed; expected {expected_default_catalog:?}, found {default_catalog:?}"
    );

    let authority = ExecutionAuthority::new(
        workspace.to_path_buf(),
        PermissionMode::FullAccess,
        LocalSandboxConfig::danger_full_access(),
        CapabilityProjection::unrestricted(),
    )?;
    let draft = agent.begin_run(
        AgentRunConfig::using_current_provider(
            authority,
            AgentRunIdentity::root(Uuid::new_v4(), 1),
        )
        .with_collaboration_mode(CollaborationMode::Plan, None),
    )?;
    let plan_catalog = draft
        .provider_tool_catalog()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<BTreeSet<_>>();
    let expected_catalog = EXPECTED_TOOLS
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        plan_catalog == expected_catalog,
        "Plan-mode Code tool surface changed; expected {expected_catalog:?}, found {plan_catalog:?}"
    );
    let agent = draft.finalize()?;

    let user_message_id = Uuid::new_v4();
    let turn = agent.prepare_turn(
        AgentTurnInput {
            thread_id: thread.id,
            user_message_id,
            workspace_root: workspace.to_path_buf(),
            content: "Run the complete default tool-surface smoke scenario.".to_string(),
            user_content: Vec::new(),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            context_budget: None,
            provider_cursor: None,
            store: Some(store.clone()),
            cancellation: None,
        },
        None,
    )?;
    let initial = AgentTurnDriver::run_turn(&agent, turn, None).await?;
    let (request, continuation) = match initial.outcome {
        AgentTurnOutcome::AwaitingInput {
            request,
            continuation,
        } => (request, continuation),
        other => anyhow::bail!("request_user_input did not pause the Topia turn: {other:?}"),
    };
    anyhow::ensure!(request.questions[0].id == "finish_smoke");

    let resumed = AgentTurnDriver::resume_turn(
        &agent,
        continuation,
        AgentResumeSignal::UserInput {
            request_id: request.request_id,
            response: UserInputResponse {
                answers: vec![UserInputAnswer {
                    question_id: "finish_smoke".to_string(),
                    option_id: Some("finish".to_string()),
                    custom_text: None,
                }],
                skipped: false,
                cancelled: false,
            },
        },
        Some(store.clone()),
        None,
        None,
    )
    .await?;
    anyhow::ensure!(matches!(resumed.outcome, AgentTurnOutcome::Completed));

    let events = initial
        .events
        .iter()
        .chain(resumed.events.iter())
        .collect::<Vec<_>>();
    verify_events(&events)?;
    anyhow::ensure!(fs::read_to_string(workspace.join("fs-smoke.txt"))? == "TOPIA_FILESYSTEM_OK\n");
    anyhow::ensure!(
        fs::read_to_string(workspace.join("patch-smoke.txt"))? == "TOPIA_APPLY_PATCH_OK\n"
    );
    anyhow::ensure!(workspace.join("smoke.xlsx").is_file());
    anyhow::ensure!(workspace
        .join(".agents/skills/topia-smoke/SKILL.md")
        .is_file());

    println!(
        "OpenTopia tool-surface smoke passed (17 default Code tools + Plan-only request_user_input)."
    );
    for name in EXPECTED_TOOLS {
        println!("PASS {name}");
    }
    Ok(())
}

fn verify_events(events: &[&AgentEventPayload]) -> anyhow::Result<()> {
    let mut call_names = BTreeMap::new();
    let mut started = BTreeSet::new();
    let mut failures = Vec::new();
    let mut saw_shell_marker = false;
    let mut saw_artifact_marker = false;
    let mut saw_attachment_marker = false;
    let mut saw_image = false;
    let mut saw_find_match = false;
    let mut work_form_updates = 0usize;

    for event in events {
        match event {
            AgentEventPayload::ToolCallStarted { call } => {
                started.insert(call.name.clone());
                call_names.insert(call.id, call.name.clone());
            }
            AgentEventPayload::ToolCallFinished { result } => {
                let name = call_names
                    .get(&result.call_id)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                if tool_result_is_error(result) {
                    failures.push(format!("{name}: {}", result.output));
                }
                if name == "shell" && result.output.contains("TOPIA_SHELL_OK") {
                    saw_shell_marker = true;
                }
                if name == "read_artifact" && result.output.contains("TOPIA_READ_ARTIFACT_OK") {
                    saw_artifact_marker = true;
                }
                if name == "read_attachment" && result.output.contains("TOPIA_READ_ATTACHMENT_OK") {
                    saw_attachment_marker = true;
                }
                if name == "view_attachment"
                    && result
                        .content
                        .iter()
                        .any(|part| matches!(part, ModelContentPart::Image { .. }))
                {
                    saw_image = true;
                }
                if name == "filesystem"
                    && result.metadata.get("operation").and_then(Value::as_str) == Some("find")
                    && result.output.contains("fs-smoke.txt")
                {
                    saw_find_match = true;
                }
            }
            AgentEventPayload::WorkFormUpdated { .. } => work_form_updates += 1,
            _ => {}
        }
    }

    let expected = EXPECTED_TOOLS
        .iter()
        .map(|name| (*name).to_string())
        .collect::<BTreeSet<_>>();
    anyhow::ensure!(
        started == expected,
        "not every default tool started: {started:?}"
    );
    anyhow::ensure!(
        failures.is_empty(),
        "tool failures: {}",
        failures.join(" | ")
    );
    anyhow::ensure!(saw_shell_marker, "shell output marker was not observed");
    anyhow::ensure!(
        saw_artifact_marker,
        "artifact output marker was not observed"
    );
    anyhow::ensure!(
        saw_attachment_marker,
        "attachment output marker was not observed"
    );
    anyhow::ensure!(saw_image, "view_attachment returned no typed image");
    anyhow::ensure!(
        saw_find_match,
        "filesystem find did not return its real smoke fixture"
    );
    anyhow::ensure!(
        work_form_updates >= 2,
        "plan tools did not update the WorkForm twice"
    );
    Ok(())
}

fn sample_pdf(text: &str) -> Vec<u8> {
    let mut document = Document::with_version("1.7");
    let pages_id = document.new_object_id();
    let page_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let page_content = content::Content {
        operations: vec![
            content::Operation::new("BT", vec![]),
            content::Operation::new("Tf", vec!["F1".into(), 12.into()]),
            content::Operation::new("Td", vec![48.into(), 760.into()]),
            content::Operation::new("Tj", vec![Object::string_literal(text)]),
            content::Operation::new("ET", vec![]),
        ],
    };
    let content_id = document.add_object(Stream::new(
        dictionary! {},
        page_content.encode().expect("encode smoke PDF"),
    ));
    document.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("save smoke PDF");
    bytes
}

fn sample_docx() -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(cursor);
    let options = SimpleFileOptions::default();
    let files = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>TOPIA_DOCUMENT_OK</w:t></w:r></w:p></w:body></w:document>"#,
        ),
    ];
    for (name, contents) in files {
        writer
            .start_file(name, options)
            .expect("start smoke DOCX part");
        writer
            .write_all(contents.as_bytes())
            .expect("write smoke DOCX part");
    }
    writer.finish().expect("finish smoke DOCX").into_inner()
}

fn sample_png() -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&[0x22, 0x88, 0xee, 0xff])?;
    }
    Ok(bytes)
}
