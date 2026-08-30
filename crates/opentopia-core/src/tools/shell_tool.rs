use super::{
    await_cancellable, decode_typed_tool_input, derived_tool_schema, enforce_policy_decision,
    enforce_read_policy, looks_like_sandbox_denial, normalize_workspace_path, tool_resource_key,
    truncate, Tool, ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect, TypedTool,
    MAX_WAIT_TIMEOUT_MS,
};
use crate::background::{BackgroundScope, BackgroundSessionSpawnRequest, BackgroundSpawnRequest};
use crate::execution::{shell_command_compatibility_error, ExecRequest, ShellDialect};
use crate::execution_authorization::{
    ApprovalEscalation, FilesystemAccess, NetworkAccess, ProcessLifetime, ToolExecutionIntent,
};
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::{ApprovalRequired, PolicyDecision};
use crate::shell_analysis::{analyze_shell_command, ShellCapability, ShellCommandAnalysis};
use crate::tool_error::insert_preserved_tool_error_record;
use crate::tool_output_truncation::{truncate_tool_result_at_source, ToolOutputSourceKind};
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub struct ShellTool;

/// Display copies of the streams kept in result metadata. They are smaller than
/// the model-facing envelope on purpose: the timeline only needs enough to show
/// the call, and the untruncated text stays in the output (or its artifact).
const SHELL_DISPLAY_STDOUT_LIMIT: usize = 16_000;
const SHELL_DISPLAY_STDERR_LIMIT: usize = 8_000;

/// A foreground command blocks the model for its whole runtime, so its ceiling stays
/// modest; anything longer belongs in the background, where waiting costs nothing.
pub(super) const MAX_FOREGROUND_TIMEOUT_SECONDS: u64 = 1_800;
pub(super) const MAX_BACKGROUND_TIMEOUT_SECONDS: u64 = 21_600;
pub(super) const DEFAULT_BACKGROUND_TIMEOUT_SECONDS: u64 = 3_600;
/// Keep ordinary commands feeling synchronous, then yield the model instead of
/// letting one slow process hold an entire parallel tool batch hostage.
pub(super) const DEFAULT_FOREGROUND_YIELD_MILLISECONDS: u64 = 30_000;
pub(super) const MAX_FOREGROUND_YIELD_MILLISECONDS: u64 = 120_000;

pub(super) fn effective_foreground_yield_milliseconds(
    requested: Option<u64>,
    minimum: Duration,
) -> u64 {
    let minimum = u64::try_from(minimum.as_millis())
        .unwrap_or(MAX_FOREGROUND_YIELD_MILLISECONDS)
        .clamp(1, MAX_FOREGROUND_YIELD_MILLISECONDS);
    requested
        .unwrap_or(DEFAULT_FOREGROUND_YIELD_MILLISECONDS)
        .clamp(minimum, MAX_FOREGROUND_YIELD_MILLISECONDS)
}

pub(super) fn background_scope(ctx: &ToolInvocationContext) -> anyhow::Result<BackgroundScope> {
    Ok(BackgroundScope {
        thread_id: ctx
            .thread_id
            .context("background commands need an owning thread")?,
        agent_path: ctx.agent_path.clone(),
    })
}

pub struct BackgroundOutputTool;

#[derive(Debug, Clone, Copy)]
pub(super) enum BackgroundOutputActionInput {
    Read,
    List,
    Write,
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "action",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum BackgroundOutputInput {
    #[schemars(rename_all = "camelCase")]
    Read {
        job_id: String,
        /// Maximum time to wait. Defaults to one hour; zero is an immediate snapshot.
        #[serde(default)]
        #[schemars(range(min = 0, max = 3600000))]
        timeout_ms: Option<u64>,
    },
    #[schemars(rename_all = "camelCase")]
    List {},
    #[schemars(rename_all = "camelCase")]
    Write {
        job_id: String,
        data: String,
        #[serde(default)]
        append_newline: bool,
    },
    #[schemars(rename_all = "camelCase")]
    Stop { job_id: String },
}

impl BackgroundOutputInput {
    fn action(&self) -> BackgroundOutputActionInput {
        match self {
            Self::Read { .. } => BackgroundOutputActionInput::Read,
            Self::List {} => BackgroundOutputActionInput::List,
            Self::Write { .. } => BackgroundOutputActionInput::Write,
            Self::Stop { .. } => BackgroundOutputActionInput::Stop,
        }
    }

    fn job_id(&self) -> Option<&str> {
        match self {
            Self::Read { job_id, .. } | Self::Write { job_id, .. } | Self::Stop { job_id } => {
                Some(job_id)
            }
            Self::List {} => None,
        }
    }
}

struct BackgroundOutputExecutionInput {
    action: BackgroundOutputActionInput,
    job_id: Option<String>,
    data: Option<String>,
    append_newline: bool,
    timeout_ms: Option<u64>,
}

impl From<BackgroundOutputInput> for BackgroundOutputExecutionInput {
    fn from(input: BackgroundOutputInput) -> Self {
        match input {
            BackgroundOutputInput::Read { job_id, timeout_ms } => Self {
                action: BackgroundOutputActionInput::Read,
                job_id: Some(job_id),
                data: None,
                append_newline: false,
                timeout_ms,
            },
            BackgroundOutputInput::List {} => Self {
                action: BackgroundOutputActionInput::List,
                job_id: None,
                data: None,
                append_newline: false,
                timeout_ms: None,
            },
            BackgroundOutputInput::Write {
                job_id,
                data,
                append_newline,
            } => Self {
                action: BackgroundOutputActionInput::Write,
                job_id: Some(job_id),
                data: Some(data),
                append_newline,
                timeout_ms: None,
            },
            BackgroundOutputInput::Stop { job_id } => Self {
                action: BackgroundOutputActionInput::Stop,
                job_id: Some(job_id),
                data: None,
                append_newline: false,
                timeout_ms: None,
            },
        }
    }
}

#[async_trait]
impl TypedTool for BackgroundOutputTool {
    type Input = BackgroundOutputInput;

    fn name(&self) -> &str {
        "background_output"
    }

    fn description(&self) -> &str {
        "Control background jobs and persistent stdio sessions you started: list them, read, write input, or stop one. Ordinary command completions are delivered automatically, so do not call read immediately after shell or browser merely to collect a result. Read is reserved for work that is blocked on a still-running job, or for interactive sessions where it also returns on new output. It is a cancellable wait and defaults to one hour; set timeoutMs to 0 only when an immediate snapshot is genuinely needed."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let key = input
            .job_id()
            .map(|job_id| tool_resource_key("session", job_id))
            .unwrap_or_else(|| "*".to_string());
        match input.action() {
            BackgroundOutputActionInput::List => {
                ToolExecutionPolicy::read_only(vec!["sessions:self".to_string()])
            }
            BackgroundOutputActionInput::Read => ToolExecutionPolicy {
                read_only: false,
                idempotent: false,
                parallel_safe: true,
                side_effect: ToolSideEffect::SessionMutation,
                resource_keys: vec![key],
            },
            BackgroundOutputActionInput::Write | BackgroundOutputActionInput::Stop => {
                ToolExecutionPolicy {
                    read_only: false,
                    idempotent: false,
                    parallel_safe: true,
                    side_effect: ToolSideEffect::SessionMutation,
                    resource_keys: vec![key],
                }
            }
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let input = BackgroundOutputExecutionInput::from(input);
        let registry = ctx
            .background
            .as_ref()
            .context("background commands are unavailable in this runtime")?;
        let scope = background_scope(&ctx)?;
        let job_id = input
            .job_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .context("jobId must be a UUID")?;

        let (value, metadata) = match input.action {
            BackgroundOutputActionInput::List => {
                let jobs = registry.list(&scope);
                let running = jobs.iter().filter(|job| !job.status.is_terminal()).count();
                (
                    json!({ "jobs": jobs, "running": running }),
                    json!({ "jobCount": jobs.len(), "running": running, "success": true }),
                )
            }
            BackgroundOutputActionInput::Stop => {
                let job_id = job_id.context("background_output stop requires jobId")?;
                registry.stop(&scope, job_id)?;
                (
                    json!({
                        "jobId": job_id,
                        "stopped": true,
                        "note": "The command was signalled to stop. Its final status arrives with the next update."
                    }),
                    json!({ "jobId": job_id, "success": true }),
                )
            }
            BackgroundOutputActionInput::Write => {
                let job_id = job_id.context("background_output write requires jobId")?;
                let mut data = input
                    .data
                    .context("background_output write requires data")?
                    .to_string();
                if input.append_newline {
                    data.push('\n');
                }
                registry
                    .write_stdin(&scope, job_id, data.as_bytes())
                    .await?;
                (
                    json!({ "jobId": job_id, "bytesWritten": data.len(), "written": true }),
                    json!({ "jobId": job_id, "bytesWritten": data.len(), "success": true }),
                )
            }
            BackgroundOutputActionInput::Read => {
                let job_id = job_id.context("background_output read requires jobId")?;
                let timeout_ms = input
                    .timeout_ms
                    .unwrap_or(MAX_WAIT_TIMEOUT_MS)
                    .min(MAX_WAIT_TIMEOUT_MS);
                let chunk = if timeout_ms == 0 {
                    registry.read_output(&scope, job_id)?
                } else {
                    match await_cancellable(
                        ctx.cancel.as_ref(),
                        registry.wait_for_readable_output(
                            &scope,
                            job_id,
                            Duration::from_millis(timeout_ms),
                        ),
                    )
                    .await??
                    {
                        Some(chunk) => chunk,
                        None => registry.read_output(&scope, job_id)?,
                    }
                };
                let metadata = json!({
                    "jobId": job_id,
                    "status": chunk.job.status.as_str(),
                    "terminal": chunk.job.status.is_terminal(),
                    "exitCode": chunk.job.exit_code,
                    "waited": timeout_ms > 0,
                    "timeoutMs": timeout_ms,
                    "success": true
                });
                (serde_json::to_value(&chunk)?, metadata)
            }
        };

        Ok(ToolResult {
            call_id,
            output: serde_json::to_string_pretty(&value)?,
            content: vec![ModelContentPart::json(value)],
            metadata: {
                let mut metadata = metadata;
                if let Some(object) = metadata.as_object_mut() {
                    object.insert("toolName".to_string(), json!("background_output"));
                }
                metadata
            },
        })
    }
}

impl_typed_tool!(BackgroundOutputTool);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ShellInput {
    /// Command interpreted by the platform shell.
    command: String,
    /// Existing workspace-relative directory. Defaults to the workspace root.
    #[serde(default)]
    workdir: Option<String>,
    /// Timeout in seconds.
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Run detached and return a job id immediately. Use only for work genuinely
    /// expected to exceed the ordinary foreground window, not for quick searches,
    /// reads, or inspections.
    #[serde(default)]
    background: bool,
    /// Optionally extend how long an ordinary command stays in the foreground.
    /// The runtime always enforces its own 30-second minimum.
    #[serde(default)]
    #[schemars(range(min = 30000, max = 120000))]
    yield_time_ms: Option<u64>,
    /// Keep stdin open as a persistent stdio session.
    #[serde(default)]
    interactive: bool,
}

#[async_trait]
impl TypedTool for ShellTool {
    type Input = ShellInput;

    fn name(&self) -> &str {
        "shell"
    }

    fn description(&self) -> &str {
        if cfg!(windows) {
            "Run a command with the configured Windows PowerShell runtime (PowerShell 7 preferred, Windows PowerShell 5.1 fallback) in a workspace directory with timeout and output caps. The runtime prompt and result metadata identify the active dialect. Multiple shell calls from one model response may start concurrently, so emit dependent or overlapping writes in separate rounds. Ordinary commands remain in the foreground for at least 30 seconds; yieldTimeMs may only extend that window. Use background only for genuinely long work, not quick inspection. Commands that outlast the foreground window continue in the background and report completion automatically; use interactive for a persistent stdio session through background_output."
        } else {
            "Run a POSIX `sh` command in a workspace directory with timeout and output caps; do not use PowerShell cmdlets or `$env:` syntax. Multiple shell calls from one model response may start concurrently, so emit dependent or overlapping writes in separate rounds. Ordinary commands remain in the foreground for at least 30 seconds; yieldTimeMs may only extend that window. Use background only for genuinely long work, not quick inspection. Commands that outlast the foreground window continue in the background and report completion automatically; use interactive for a persistent stdio session through background_output."
        }
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let analysis = analyze_shell_command(&input.command);
        if !input.background && !input.interactive && analysis.is_strictly_read_only() {
            return ToolExecutionPolicy::read_only(Vec::new());
        }
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: true,
            side_effect: ToolSideEffect::Process,
            // Shell calls are intentionally not serialized by guessed resource
            // conflicts. A model-issued tool batch has no intra-batch result
            // dependency; command failures remain structured observations for
            // the next model round to inspect and repair.
            resource_keys: Vec::new(),
        }
    }

    fn authorization_preflight(
        &self,
        input: &Self::Input,
        ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        if analyze_shell_command(&input.command).is_unreviewable_destructive_action() {
            // Let execution return the structured UnreviewableAction result so
            // the model can concretize the target without creating a useless
            // approval request for an action that cannot be authorized safely.
            return Some(PolicyDecision::Allow);
        }
        Some(ctx.policy.inspect_command(&input.command))
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        // A nominally foreground call may yield into the shared background
        // registry, so its authority must describe the process's real lifetime.
        shell_execution_intent(&analyze_shell_command(&input.command))
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let command = input.command.trim();
        anyhow::ensure!(!command.is_empty(), "shell requires a command");
        if let Some(error) = shell_command_compatibility_error(command) {
            return Ok(shell_compatibility_error_result(call_id, error));
        }
        let analysis = analyze_shell_command(command);
        if analysis.is_unreviewable_destructive_action() {
            return Ok(unreviewable_shell_action_result(call_id, command));
        }
        enforce_policy_decision(ctx.policy.inspect_command(command), &ctx)?;

        let interactive = input.interactive;
        let background = interactive || input.background;
        let requested_workdir = input.workdir.as_deref().unwrap_or(".");
        let logical_workdir = normalize_workspace_path(&ctx.workspace_root, requested_workdir)?;
        enforce_read_policy(&ctx, &logical_workdir)?;
        let workdir = ctx.environment.resolve_read_path(&logical_workdir)?;
        if !workdir.is_dir() {
            anyhow::bail!("shell workdir is not a directory: {}", workdir.display());
        }
        let can_auto_yield = !background && ctx.background.is_some() && ctx.thread_id.is_some();
        let long_lived = background || can_auto_yield;
        let timeout_seconds = input
            .timeout_seconds
            .unwrap_or(if long_lived {
                DEFAULT_BACKGROUND_TIMEOUT_SECONDS
            } else {
                30
            })
            .min(if long_lived {
                MAX_BACKGROUND_TIMEOUT_SECONDS
            } else {
                MAX_FOREGROUND_TIMEOUT_SECONDS
            });

        if interactive {
            let registry = ctx
                .background
                .as_ref()
                .context("interactive commands are unavailable in this runtime")?;
            let job = registry
                .spawn_session(
                    ctx.environment.clone(),
                    BackgroundSessionSpawnRequest {
                        scope: background_scope(&ctx)?,
                        command: command.to_string(),
                        request: model_shell_request(command, true).cwd(&workdir),
                        context: ctx.execution_context(Duration::from_secs(timeout_seconds)),
                    },
                )
                .await?;
            let value = json!({
                "jobId": job.job_id,
                "status": job.status.as_str(),
                "command": job.command,
                "workdir": workdir.display().to_string(),
                "interactive": true,
                "transport": "stdio",
                "startedAt": job.started_at,
                "note": "The persistent stdio session is running. Use background_output write/read/stop with this job id."
            });
            return Ok(ToolResult {
                call_id,
                output: serde_json::to_string_pretty(&value)?,
                content: vec![ModelContentPart::json(value)],
                metadata: json!({
                    "toolName": "shell",
                    "background": true,
                    "interactive": true,
                    "transport": "stdio",
                    "shellDialect": ShellDialect::current().id(),
                    "jobId": job.job_id,
                    "workdir": workdir.display().to_string(),
                    "success": true
                }),
            });
        }

        if background || can_auto_yield {
            let registry = ctx
                .background
                .as_ref()
                .context("background commands are unavailable in this runtime")?;
            let scope = background_scope(&ctx)?;
            let started_at = Instant::now();
            let job = registry.spawn(
                ctx.environment.clone(),
                BackgroundSpawnRequest {
                    scope: scope.clone(),
                    command: command.to_string(),
                    request: model_shell_request(command, false).cwd(&workdir),
                    context: ctx.execution_context(Duration::from_secs(timeout_seconds)),
                },
            )?;
            if background {
                return shell_background_result(call_id, &job, &workdir, false, None);
            }

            let yield_time_ms = effective_foreground_yield_milliseconds(
                input.yield_time_ms,
                ctx.minimum_foreground_yield,
            );
            if let Some(chunk) = registry
                .wait_for_output(&scope, job.job_id, Duration::from_millis(yield_time_ms))
                .await?
            {
                if chunk.job.status == crate::background::BackgroundJobStatus::Cancelled {
                    anyhow::bail!(
                        "{}",
                        chunk
                            .job
                            .error
                            .as_deref()
                            .unwrap_or("shell execution cancelled")
                    );
                }
                let stderr = if chunk.stderr.trim().is_empty() {
                    chunk.job.error.clone().unwrap_or_default()
                } else {
                    chunk.stderr
                };
                if let Some(reason) = chunk.job.approval_required {
                    return Err(ApprovalRequired::new(reason).into());
                }
                if !chunk.job.success && looks_like_sandbox_denial(&stderr) {
                    return Err(ApprovalRequired::new(format!(
                        "Command was blocked by the sandbox: {}",
                        truncate(&stderr, 2_000)
                    ))
                    .into());
                }
                let mut result = shell_completed_result(
                    call_id,
                    command,
                    &workdir,
                    started_at.elapsed().as_millis() as u64,
                    chunk.stdout,
                    stderr,
                    chunk.job.exit_code,
                    chunk.job.success,
                    chunk.job.truncated,
                    chunk.job.sandbox,
                )?;
                if let Some(error_record) = chunk.job.error_record.as_ref() {
                    insert_preserved_tool_error_record(&mut result.metadata, error_record);
                }
                return Ok(truncate_tool_result_at_source(
                    "shell",
                    result,
                    ToolOutputSourceKind::Shell,
                    ctx.state.as_ref(),
                    ctx.thread_id,
                ));
            }

            return shell_background_result(call_id, &job, &workdir, true, Some(yield_time_ms));
        }

        let started_at = Instant::now();
        let output = ctx
            .environment
            .exec(
                model_shell_request(command, false).cwd(&workdir),
                ctx.execution_context(Duration::from_secs(timeout_seconds)),
            )
            .await?;
        let duration_ms = started_at.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.success && looks_like_sandbox_denial(&stderr) {
            return Err(ApprovalRequired::new(format!(
                "Command was blocked by the sandbox: {}",
                truncate(&stderr, 2_000)
            ))
            .into());
        }
        let result = shell_completed_result(
            call_id,
            command,
            &workdir,
            duration_ms,
            stdout,
            stderr,
            output.exit_code,
            output.success,
            output.truncated,
            output.sandbox,
        )?;
        Ok(truncate_tool_result_at_source(
            "shell",
            result,
            ToolOutputSourceKind::Shell,
            ctx.state.as_ref(),
            ctx.thread_id,
        ))
    }
}

fn shell_background_result(
    call_id: Uuid,
    job: &crate::background::BackgroundJobSnapshot,
    workdir: &Path,
    auto_detached: bool,
    yield_time_ms: Option<u64>,
) -> anyhow::Result<ToolResult> {
    let note = if auto_detached {
        "The command exceeded the foreground wait and is still running. Carry on with independent work; completion is delivered automatically. Do not immediately call background_output merely to collect it; use that tool only to stop it, interact with it, or when progress is blocked on this still-running job and no independent work remains."
    } else {
        "The command is running detached. Carry on with independent work; completion is delivered automatically. Do not immediately call background_output merely to collect it; use that tool only to stop it, interact with it, or when progress is blocked on this still-running job and no independent work remains."
    };
    let value = json!({
        "jobId": job.job_id,
        "status": job.status.as_str(),
        "command": job.command,
        "workdir": workdir.display().to_string(),
        "startedAt": job.started_at,
        "autoDetached": auto_detached,
        "yieldTimeMs": yield_time_ms,
        "note": note
    });
    Ok(ToolResult {
        call_id,
        output: serde_json::to_string_pretty(&value)?,
        content: vec![ModelContentPart::json(value)],
        metadata: json!({
            "toolName": "shell",
            "background": true,
            "autoDetached": auto_detached,
            "yieldTimeMs": yield_time_ms,
            "shellDialect": ShellDialect::current().id(),
            "jobId": job.job_id,
            "workdir": workdir.display().to_string(),
            "success": true
        }),
    })
}

#[allow(clippy::too_many_arguments)]
fn shell_completed_result(
    call_id: Uuid,
    command: &str,
    workdir: &Path,
    duration_ms: u64,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    success: bool,
    truncated: bool,
    sandbox: Option<crate::execution::ExecutionSandboxMetadata>,
) -> anyhow::Result<ToolResult> {
    let full_combined = format!(
        "$ {}\n\n[stdout]\n{}\n\n[stderr]\n{}",
        command, stdout, stderr
    );
    // The shell output adapter applies the producer budget immediately after
    // this lossless envelope is built. The UI uses the smaller stream previews
    // below and therefore does not need the artifact in its timeline.
    let result = ToolResult {
        call_id,
        output: full_combined,
        content: Vec::new(),
        metadata: json!({
            "command": command,
            "shellDialect": ShellDialect::current().id(),
            "workdir": workdir.display().to_string(),
            "exitCode": exit_code,
            "success": success,
            "truncated": truncated,
            "durationMs": duration_ms,
            "stdout": truncate(&stdout, SHELL_DISPLAY_STDOUT_LIMIT),
            "stderr": truncate(&stderr, SHELL_DISPLAY_STDERR_LIMIT),
            "sandbox": sandbox
        }),
    };

    Ok(result)
}

impl_typed_tool!(ShellTool);

pub(super) fn shell_execution_intent(analysis: &ShellCommandAnalysis) -> ToolExecutionIntent {
    let reads_files = analysis.capabilities.iter().any(|capability| {
        matches!(
            capability,
            ShellCapability::ReadFiles | ShellCapability::GitRead
        )
    });
    let writes_files = analysis.capabilities.iter().any(|capability| {
        matches!(
            capability,
            ShellCapability::WorkspaceWrite
                | ShellCapability::DeleteFiles
                | ShellCapability::GitMutation
        )
    });
    let needs_network = analysis.capabilities.contains(&ShellCapability::Network);
    let network_is_unknown = analysis.capabilities.iter().any(|capability| {
        matches!(
            capability,
            ShellCapability::DynamicExecution | ShellCapability::Unknown
        )
    });
    let command_scoped = analysis.capabilities.iter().any(|capability| {
        matches!(
            capability,
            ShellCapability::DynamicExecution
                | ShellCapability::Unknown
                | ShellCapability::GitMutation
        )
    });
    let concrete_paths = analysis
        .concrete_targets
        .iter()
        .filter(|target| shell_target_is_path(target))
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    let mut intent = if writes_files {
        ToolExecutionIntent::workspace_mutation(concrete_paths.clone())
    } else if reads_files || analysis.is_strictly_read_only() {
        ToolExecutionIntent::observation(concrete_paths.clone())
    } else {
        ToolExecutionIntent::session_process(ProcessLifetime::Background)
    };
    intent.process_lifetime = ProcessLifetime::Background;
    intent.network = if needs_network {
        NetworkAccess::Required
    } else if network_is_unknown {
        // A script or arbitrary executable may open sockets internally even when
        // its command line contains no URL. Absence of a recognized network verb
        // is not proof that dynamic code is offline, so honor the session policy.
        NetworkAccess::InheritSession
    } else {
        NetworkAccess::PreferDeny
    };
    intent.filesystem = if writes_files {
        FilesystemAccess::WriteWorkspace
    } else if reads_files || analysis.is_strictly_read_only() {
        FilesystemAccess::ReadWorkspace
    } else {
        FilesystemAccess::InheritSession
    };
    intent.approval_escalation = if command_scoped {
        ApprovalEscalation::CommandScopedHostAccess
    } else if concrete_paths.is_empty() {
        ApprovalEscalation::None
    } else {
        ApprovalEscalation::ExactPaths
    };
    if reads_files && !writes_files {
        intent.requested_read_paths = concrete_paths;
    }
    intent
}

fn shell_target_is_path(target: &str) -> bool {
    let target = target.trim();
    !target.is_empty()
        && !target.contains("://")
        && !matches!(
            target,
            "workspace:command-scope"
                | "repository:current-workdir"
                | "repository:index-and-worktree"
        )
}

fn model_shell_request(command: &str, interactive: bool) -> ExecRequest {
    let request = ExecRequest::shell(command);
    if interactive {
        request
    } else {
        request.envs([
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GCM_INTERACTIVE", "Never"),
            ("GIT_PAGER", "cat"),
            ("GH_PAGER", "cat"),
            ("PAGER", "cat"),
        ])
    }
}

fn shell_compatibility_error_result(
    call_id: Uuid,
    error: crate::execution::ShellCompatibilityError,
) -> ToolResult {
    let dialect = ShellDialect::current().id();
    ToolResult {
        call_id,
        output: error.message.clone(),
        content: vec![ModelContentPart::text(error.message.clone())],
        metadata: json!({
            "toolName": "shell",
            "shellDialect": dialect,
            "success": false,
            "error": error.message,
            "errorRecord": {
                "recorded": true,
                "code": error.code,
                "phase": "validation",
                "executed": false,
                "retryable": true,
                "message": error.message,
            }
        }),
    }
}

fn unreviewable_shell_action_result(call_id: Uuid, command: &str) -> ToolResult {
    let message = format!(
        "UnreviewableAction: destructive shell command contains an unresolved variable, wildcard, command substitution, or no concrete target. Resolve the target and submit a new tool call. Command: {command}"
    );
    ToolResult {
        call_id,
        output: message.clone(),
        content: vec![ModelContentPart::text(message.clone())],
        metadata: json!({
            "toolName": "shell",
            "shellDialect": ShellDialect::current().id(),
            "success": false,
            "reviewability": "unreviewable_action",
            "error": message,
            "errorRecord": {
                "recorded": true,
                "code": "unreviewable_action",
                "phase": "validation",
                "executed": false,
                "retryable": true,
                "message": message,
            }
        }),
    }
}
