use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision,
    looks_like_sandbox_denial, normalize_workspace_path, tool_resource_key, truncate, Tool,
    ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect, TypedTool,
};
use crate::execution::{ExecutionEnvironment, FileDeleteRequest, FileWriteRequest};
use crate::execution_authorization::ToolExecutionIntent;
use crate::file_mutation::{
    lock_mutation_paths, normalize_text_encoding, read_optional, FileMutationBatch,
    FileMutationTarget, PreparedFileMutation,
};
use crate::model::{ToolCall, ToolResult};
use crate::policy::{ApprovalRequired, PolicyDecision};
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

pub struct ApplyPatchTool;

/// Provider-native patch calls are normalized here instead of teaching the
/// workspace executor about any one transport. Their `diff` is commonly a bare
/// unified hunk (`@@ ...`) and therefore cannot be passed directly to `git apply`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativePatchOperation {
    CreateFile { path: String, diff: String },
    UpdateFile { path: String, diff: String },
    DeleteFile { path: String },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum ApplyPatchInput {
    Portable(PortablePatchInput),
    Structured(StructuredPatchInput),
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct PortablePatchInput {
    /// Portable unified diff patch.
    patch: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct StructuredPatchInput {
    /// Structured provider-native operation.
    operation: NativePatchOperation,
}

impl NativePatchOperation {
    pub fn path(&self) -> &str {
        match self {
            Self::CreateFile { path, .. }
            | Self::UpdateFile { path, .. }
            | Self::DeleteFile { path } => path,
        }
    }
}

#[async_trait]
impl TypedTool for ApplyPatchTool {
    type Input = ApplyPatchInput;

    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply file edits under the active workspace policy. Portable callers pass exactly one `patch` string using a `*** Begin Patch` ... `*** End Patch` envelope; update sections use `*** Update File: path` plus unified `@@` hunks. Native providers may instead pass one structured create_file/update_file/delete_file operation. Absolute paths outside the workspace require approval for that exact path. Structured SEARCH/REPLACE updates must use the exact `<<<<<<< SEARCH`, `=======`, and `>>>>>>> REPLACE` markers."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        let key = match input {
            ApplyPatchInput::Portable(_) => "workspace:*".to_string(),
            ApplyPatchInput::Structured(input) => tool_resource_key("file", input.operation.path()),
        };
        ToolExecutionPolicy {
            read_only: false,
            idempotent: false,
            parallel_safe: true,
            side_effect: ToolSideEffect::WorkspaceWrite,
            resource_keys: vec![key],
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        let paths = match input {
            ApplyPatchInput::Structured(input) => {
                vec![PathBuf::from(input.operation.path())]
            }
            ApplyPatchInput::Portable(input) => parse_apply_patch_envelope(&input.patch)
                .ok()
                .flatten()
                .map(|operations| {
                    operations
                        .into_iter()
                        .map(|operation| PathBuf::from(operation.path()))
                        .collect()
                })
                .unwrap_or_else(|| {
                    unified_diff_paths(&input.patch)
                        .into_iter()
                        .map(PathBuf::from)
                        .collect()
                }),
        };
        ToolExecutionIntent::workspace_mutation(paths)
    }

    fn authorization_preflight(
        &self,
        input: &Self::Input,
        _ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        let deletes_file = match input {
            ApplyPatchInput::Structured(input) => {
                matches!(&input.operation, NativePatchOperation::DeleteFile { .. })
            }
            ApplyPatchInput::Portable(input) => {
                parse_apply_patch_envelope(&input.patch)
                    .ok()
                    .flatten()
                    .is_some_and(|operations| {
                        operations.iter().any(|operation| {
                            matches!(operation, NativePatchOperation::DeleteFile { .. })
                        })
                    })
                    || unified_diff_deletes_file(&input.patch)
            }
        };
        deletes_file.then(|| PolicyDecision::Ask {
            reason: "Deleting a file through apply_patch requires approval.".to_string(),
        })
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        match input {
            ApplyPatchInput::Portable(input) => {
                execute_portable_patch(call_id, &input.patch, ctx).await
            }
            ApplyPatchInput::Structured(input) => {
                execute_native_patch_operation(call_id, input.operation, ctx).await
            }
        }
    }
}

impl_typed_tool!(ApplyPatchTool);

pub(super) async fn execute_portable_patch(
    call_id: Uuid,
    patch: &str,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    if let Some(operations) = parse_apply_patch_envelope(patch)? {
        let outcome = execute_native_patch_batch(operations, &ctx).await?;
        return Ok(ToolResult {
            call_id,
            output: outcome.outputs.join("\n"),
            content: Vec::new(),
            metadata: json!({
                "success": true,
                "changedPaths": outcome.changed_paths,
                "format": "apply_patch_envelope"
            }),
        });
    }

    if unified_diff_deletes_file(patch) {
        enforce_policy_decision(
            PolicyDecision::Ask {
                reason: "Deleting a file through apply_patch requires approval.".to_string(),
            },
            ctx.approval_granted,
        )?;
    }

    enforce_policy_decision(
        ctx.policy
            .inspect_command("git apply --whitespace=nowarn -"),
        ctx.approval_granted,
    )?;
    let mutation_scope = ctx.file_mutation_scope()?;

    let changed_paths = unified_diff_paths(patch);
    let mutation_paths = changed_paths
        .iter()
        .map(|path| normalize_workspace_path(&ctx.workspace_root, path))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let _locks = lock_mutation_paths(mutation_paths.clone()).await;
    let mut originals = Vec::with_capacity(mutation_paths.len());
    for path in &mutation_paths {
        originals.push(read_optional(ctx.environment.as_ref(), path).await?);
    }

    let result = ctx
        .environment
        .apply_patch(patch, ctx.execution_context(Duration::from_secs(30)))
        .await
        .map_err(|error| anyhow::anyhow!("git apply failed: {error:#}"))?;
    let output = result.exec;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.success && looks_like_sandbox_denial(&stderr) {
        return Err(ApprovalRequired::new(format!(
            "Patch was blocked by the sandbox: {}",
            truncate(&stderr, 2_000)
        ))
        .into());
    }
    if !output.success {
        anyhow::bail!(
            "git apply failed ({:?})\n{}",
            output.exit_code,
            truncate(&stderr, 12_000)
        );
    }

    let mut mutations = Vec::new();
    for (path, original) in mutation_paths.into_iter().zip(originals) {
        let current = read_optional(ctx.environment.as_ref(), &path).await?;
        if current == original {
            continue;
        }
        mutations.push(match current {
            Some(contents) => PreparedFileMutation {
                path,
                original,
                target: FileMutationTarget::Write(contents),
            },
            None => PreparedFileMutation {
                path,
                original,
                target: FileMutationTarget::Delete,
            },
        });
    }
    for index in 0..mutations.len() {
        let mutation = &mutations[index];
        let FileMutationTarget::Write(contents) = &mutation.target else {
            continue;
        };
        let path = mutation.path.clone();
        let normalized = normalize_text_encoding(&path, contents.clone());
        if normalized == *contents {
            continue;
        }
        if let Err(error) = ctx
            .environment
            .write_file(FileWriteRequest::new(&path, normalized.clone()))
            .await
        {
            rollback_external_mutations(ctx.environment.as_ref(), &mutations).await?;
            return Err(error.context(format!(
                "failed to normalize text encoding for {}",
                path.display()
            )));
        }
        mutations[index].target = FileMutationTarget::Write(normalized);
    }
    if let (Some(observer), Some(scope)) = (
        ctx.file_mutation_observer.as_deref(),
        mutation_scope.as_ref(),
    ) {
        if let Err(error) = observer.record_file_mutations(scope, &mutations).await {
            rollback_external_mutations(ctx.environment.as_ref(), &mutations).await?;
            return Err(error.context("failed to persist applied unified diff"));
        }
    }
    Ok(ToolResult {
        call_id,
        output: format!(
            "Patch applied.\n\n[stdout]\n{}\n\n[stderr]\n{}",
            truncate(&stdout, 8_000),
            truncate(&stderr, 8_000)
        ),
        content: Vec::new(),
        metadata: json!({
            "success": true,
            "bytes": result.bytes,
            "changedPaths": changed_paths,
            "sandbox": output.sandbox
        }),
    })
}

async fn rollback_external_mutations(
    environment: &dyn ExecutionEnvironment,
    mutations: &[PreparedFileMutation],
) -> anyhow::Result<()> {
    for mutation in mutations.iter().rev() {
        let current = read_optional(environment, &mutation.path).await?;
        let expected = match &mutation.target {
            FileMutationTarget::Write(contents) => Some(contents.as_slice()),
            FileMutationTarget::Delete => None,
        };
        anyhow::ensure!(
            current.as_deref() == expected,
            "cannot roll back unjournaled patch because {} changed again",
            mutation.path.display()
        );
        match &mutation.original {
            Some(contents) => {
                environment
                    .write_file(FileWriteRequest::new(&mutation.path, contents.clone()))
                    .await?;
            }
            None => {
                environment
                    .delete_file(FileDeleteRequest::new(&mutation.path))
                    .await?;
            }
        }
    }
    Ok(())
}

/// Execute one normalized native operation. This is public for transport
/// adapters that surface hosted apply-patch calls outside ordinary function
/// calling; portable callers continue to use [`ApplyPatchTool`].
pub async fn execute_native_patch_operation(
    call_id: Uuid,
    operation: NativePatchOperation,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let mut outcome = execute_native_patch_batch(vec![operation], &ctx).await?;
    let report = outcome
        .reports
        .pop()
        .context("native patch batch returned no operation report")?;
    let changed_path = outcome
        .changed_paths
        .pop()
        .context("native patch batch returned no changed path")?;
    Ok(ToolResult {
        call_id,
        output: outcome.outputs.pop().unwrap_or_default(),
        content: Vec::new(),
        metadata: json!({
            "success": true,
            "operation": report.operation,
            "changedPath": changed_path,
            "bytes": report.bytes
        }),
    })
}

#[derive(Debug)]
struct NativePatchReport {
    operation: &'static str,
    bytes: usize,
}

#[derive(Debug)]
struct NativePatchBatchOutcome {
    outputs: Vec<String>,
    reports: Vec<NativePatchReport>,
    changed_paths: Vec<String>,
}

async fn execute_native_patch_batch(
    operations: Vec<NativePatchOperation>,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<NativePatchBatchOutcome> {
    anyhow::ensure!(!operations.is_empty(), "native patch batch is empty");
    let mut mutations = Vec::with_capacity(operations.len());
    let mut outputs = Vec::with_capacity(operations.len());
    let mut reports = Vec::with_capacity(operations.len());

    // Complete parsing, path validation, authorization, and content generation
    // before the first filesystem mutation is attempted.
    for operation in operations {
        let logical_path = validate_patch_operation_path(operation.path())?;
        let target = normalize_workspace_path(&ctx.workspace_root, &logical_path)?;
        enforce_policy_decision(ctx.policy.inspect_write(&target), ctx.approval_granted)?;
        if matches!(&operation, NativePatchOperation::DeleteFile { .. }) {
            enforce_policy_decision(
                PolicyDecision::Ask {
                    reason: format!(
                        "Deleting workspace file {} requires approval.",
                        target.display()
                    ),
                },
                ctx.approval_granted,
            )?;
        }
        let original = read_optional(ctx.environment.as_ref(), &target).await?;

        match operation {
            NativePatchOperation::DeleteFile { .. } => {
                let original = original.with_context(|| {
                    format!("delete_file target does not exist: {logical_path}")
                })?;
                mutations.push(PreparedFileMutation::delete(&target, original));
                outputs.push(format!("Deleted {}", target.display()));
                reports.push(NativePatchReport {
                    operation: "delete_file",
                    bytes: 0,
                });
            }
            NativePatchOperation::CreateFile { diff, .. } => {
                anyhow::ensure!(
                    original.is_none(),
                    "create_file target already exists: {logical_path}"
                );
                let contents = normalize_text_encoding(
                    &target,
                    create_file_contents_from_diff(&diff)?.into_bytes(),
                );
                let bytes = contents.len();
                mutations.push(PreparedFileMutation::write(&target, None, contents));
                outputs.push(format!("Created {}", target.display()));
                reports.push(NativePatchReport {
                    operation: "create_file",
                    bytes,
                });
            }
            NativePatchOperation::UpdateFile { diff, .. } => {
                let original_bytes = original.with_context(|| {
                    format!("update_file target does not exist: {logical_path}")
                })?;
                let had_utf8_bom = original_bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
                let text_bytes = if had_utf8_bom {
                    &original_bytes[3..]
                } else {
                    &original_bytes
                };
                let original_text = String::from_utf8(text_bytes.to_vec()).with_context(|| {
                    format!("update_file target is not UTF-8 text: {logical_path}")
                })?;
                let updated = apply_text_patch(&original_text, &diff).with_context(|| {
                    format!("failed to apply update_file patch to {logical_path}")
                })?;
                anyhow::ensure!(
                    updated != original_text,
                    "update_file patch made no changes: {logical_path}"
                );
                let mut contents = updated.into_bytes();
                if had_utf8_bom {
                    let mut encoded = Vec::with_capacity(contents.len() + 3);
                    encoded.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                    encoded.append(&mut contents);
                    contents = encoded;
                }
                let contents = normalize_text_encoding(&target, contents);
                let bytes = contents.len();
                mutations.push(PreparedFileMutation::write(
                    &target,
                    Some(original_bytes),
                    contents,
                ));
                outputs.push(format!("Updated {}", target.display()));
                reports.push(NativePatchReport {
                    operation: "update_file",
                    bytes,
                });
            }
        }
    }

    let batch = FileMutationBatch::new(mutations)?;
    let committed = ctx.commit_file_mutations(&batch).await?;
    Ok(NativePatchBatchOutcome {
        outputs,
        reports,
        changed_paths: committed
            .changed_paths
            .into_iter()
            .map(|path| {
                path.strip_prefix(&ctx.workspace_root)
                    .unwrap_or(&path)
                    .display()
                    .to_string()
            })
            .collect(),
    })
}

fn unified_diff_deletes_file(patch: &str) -> bool {
    patch.lines().any(|line| {
        line.trim_end_matches('\r') == "+++ /dev/null"
            || line
                .trim_end_matches('\r')
                .starts_with("deleted file mode ")
    })
}

pub fn native_patch_operation_to_unified_diff(
    operation: &NativePatchOperation,
) -> anyhow::Result<String> {
    let path = validate_native_patch_path(operation.path())?;
    match operation {
        NativePatchOperation::DeleteFile { .. } => {
            anyhow::bail!("delete_file is executed directly and has no supplied diff")
        }
        NativePatchOperation::CreateFile { diff, .. } => {
            let hunks = normalize_native_create_hunks(diff)?;
            Ok(format!(
                "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n{hunks}"
            ))
        }
        NativePatchOperation::UpdateFile { diff, .. } => {
            let hunks = extract_native_hunks(diff)
                .context("update_file diff must contain at least one unified @@ hunk")?;
            Ok(format!(
                "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{hunks}"
            ))
        }
    }
}

fn parse_apply_patch_envelope(patch: &str) -> anyhow::Result<Option<Vec<NativePatchOperation>>> {
    let normalized = patch.replace("\r\n", "\n");
    let mut lines = normalized.lines().peekable();
    if lines.next().map(str::trim) != Some("*** Begin Patch") {
        return Ok(None);
    }
    let mut operations = Vec::new();
    while let Some(line) = lines.next() {
        if line.trim() == "*** End Patch" {
            anyhow::ensure!(!operations.is_empty(), "apply patch envelope is empty");
            return Ok(Some(operations));
        }
        let (kind, path) = if let Some(path) = line.strip_prefix("*** Update File: ") {
            ("update", path.trim())
        } else if let Some(path) = line.strip_prefix("*** Add File: ") {
            ("add", path.trim())
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ("delete", path.trim())
        } else if line.trim().is_empty() {
            continue;
        } else {
            anyhow::bail!("unsupported apply patch directive: {line}");
        };
        validate_patch_operation_path(path)?;
        if kind == "delete" {
            operations.push(NativePatchOperation::DeleteFile {
                path: path.to_string(),
            });
            continue;
        }
        let mut diff_lines = Vec::new();
        while let Some(next) = lines.peek() {
            if next.starts_with("*** ") {
                break;
            }
            diff_lines.push(lines.next().unwrap_or_default());
        }
        let mut diff = diff_lines.join("\n");
        if !diff.is_empty() {
            diff.push('\n');
        }
        operations.push(if kind == "add" {
            NativePatchOperation::CreateFile {
                path: path.to_string(),
                diff,
            }
        } else {
            NativePatchOperation::UpdateFile {
                path: path.to_string(),
                diff,
            }
        });
    }
    anyhow::bail!("apply patch envelope is missing *** End Patch")
}

fn unified_diff_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.replace("\r\n", "\n").lines() {
        let Some(raw) = line
            .strip_prefix("+++ ")
            .or_else(|| line.strip_prefix("--- "))
        else {
            continue;
        };
        let raw = raw.split('\t').next().unwrap_or(raw).trim();
        if raw == "/dev/null" {
            continue;
        }
        let path = raw
            .strip_prefix("a/")
            .or_else(|| raw.strip_prefix("b/"))
            .unwrap_or(raw);
        if !path.is_empty() && !paths.iter().any(|known| known == path) {
            paths.push(path.to_string());
        }
    }
    paths
}

fn create_file_contents_from_diff(diff: &str) -> anyhow::Result<String> {
    let normalized = diff.replace("\r\n", "\n");
    if let Some(hunks) = extract_native_hunks(&normalized) {
        let mut contents = Vec::new();
        for line in hunks.lines() {
            if line.starts_with("@@") || line == "\\ No newline at end of file" {
                continue;
            }
            if let Some(line) = line.strip_prefix('+') {
                contents.push(line);
            } else if line.starts_with('-') {
                anyhow::bail!("create_file diff cannot remove existing lines");
            } else if let Some(line) = line.strip_prefix(' ') {
                contents.push(line);
            }
        }
        return Ok(format!("{}\n", contents.join("\n")));
    }

    let mut contents = Vec::new();
    for line in normalized.lines() {
        let Some(line) = line.strip_prefix('+') else {
            anyhow::bail!("create_file content lines must start with '+'");
        };
        contents.push(line);
    }
    Ok(if contents.is_empty() {
        String::new()
    } else {
        format!("{}\n", contents.join("\n"))
    })
}

pub(super) fn apply_text_patch(original: &str, diff: &str) -> anyhow::Result<String> {
    let newline = if original.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let normalized = original.replace("\r\n", "\n");
    let diff = diff.replace("\r\n", "\n");
    let updated = if diff.contains("<<<<<<< SEARCH") {
        apply_search_replace_patch(&normalized, &diff)?
    } else {
        apply_unified_text_hunks(&normalized, &diff)?
    };
    Ok(if newline == "\r\n" {
        updated.replace('\n', "\r\n")
    } else {
        updated
    })
}

fn apply_search_replace_patch(original: &str, diff: &str) -> anyhow::Result<String> {
    const SEARCH: &str = "<<<<<<< SEARCH\n";
    const DIVIDER: &str = "=======\n";
    const REPLACE: &str = ">>>>>>> REPLACE";
    let mut remaining = diff;
    let mut updated = original.to_string();
    let mut replacements = 0usize;
    while let Some(start) = remaining.find(SEARCH) {
        remaining = &remaining[start + SEARCH.len()..];
        let divider = remaining
            .find(DIVIDER)
            .context("SEARCH/REPLACE patch is missing ======= divider")?;
        let search = &remaining[..divider];
        remaining = &remaining[divider + DIVIDER.len()..];
        let end = remaining
            .find(REPLACE)
            .context("SEARCH/REPLACE patch is missing >>>>>>> REPLACE marker")?;
        let replacement = &remaining[..end];
        let matches = updated
            .match_indices(search)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !matches.is_empty(),
            "SEARCH block was not found in target file"
        );
        anyhow::ensure!(
            matches.len() == 1,
            "SEARCH block matched more than once in target file"
        );
        updated.replace_range(matches[0]..matches[0] + search.len(), replacement);
        replacements += 1;
        remaining = &remaining[end + REPLACE.len()..];
    }
    anyhow::ensure!(
        replacements > 0,
        "SEARCH/REPLACE patch did not contain a replacement"
    );
    Ok(updated)
}

#[derive(Debug)]
struct TextPatchHunk {
    old_start: Option<usize>,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

fn apply_unified_text_hunks(original: &str, diff: &str) -> anyhow::Result<String> {
    let hunks = extract_native_hunks(diff)
        .context("update_file diff must contain a unified @@ hunk or SEARCH/REPLACE block")?;
    let mut parsed = Vec::<TextPatchHunk>::new();
    for line in hunks.lines() {
        if line.starts_with("@@") {
            let old_start = line
                .split_whitespace()
                .find(|part| part.starts_with('-'))
                .and_then(|part| part.trim_start_matches('-').split(',').next())
                .and_then(|value| value.parse::<usize>().ok());
            parsed.push(TextPatchHunk {
                old_start,
                old_lines: Vec::new(),
                new_lines: Vec::new(),
            });
            continue;
        }
        if line == "\\ No newline at end of file" {
            continue;
        }
        let hunk = parsed
            .last_mut()
            .context("patch content appeared before @@ hunk")?;
        if let Some(value) = line.strip_prefix(' ') {
            hunk.old_lines.push(value.to_string());
            hunk.new_lines.push(value.to_string());
        } else if let Some(value) = line.strip_prefix('-') {
            hunk.old_lines.push(value.to_string());
        } else if let Some(value) = line.strip_prefix('+') {
            hunk.new_lines.push(value.to_string());
        } else {
            anyhow::bail!("invalid unified patch line: {line}");
        }
    }

    let mut updated = original.to_string();
    for hunk in parsed {
        let old = hunk.old_lines.join("\n");
        let new = hunk.new_lines.join("\n");
        if old.is_empty() {
            let line = hunk.old_start.unwrap_or(1).saturating_sub(1);
            let offset = line_offset(&updated, line);
            updated.insert_str(offset, &new);
            continue;
        }
        let expected = line_offset(&updated, hunk.old_start.unwrap_or(1).saturating_sub(1));
        let mut candidates = updated
            .match_indices(&old)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            let old_with_newline = format!("{old}\n");
            let new_with_newline = format!("{new}\n");
            candidates = updated
                .match_indices(&old_with_newline)
                .map(|(index, _)| index)
                .collect();
            if !candidates.is_empty() {
                let index = *candidates
                    .iter()
                    .min_by_key(|index| index.abs_diff(expected))
                    .unwrap_or(&candidates[0]);
                updated.replace_range(index..index + old_with_newline.len(), &new_with_newline);
                continue;
            }
        }
        anyhow::ensure!(
            !candidates.is_empty(),
            "unified patch context was not found in target file"
        );
        let index = *candidates
            .iter()
            .min_by_key(|index| index.abs_diff(expected))
            .unwrap_or(&candidates[0]);
        updated.replace_range(index..index + old.len(), &new);
    }
    Ok(updated)
}

fn line_offset(text: &str, line_index: usize) -> usize {
    if line_index == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line_index.saturating_sub(1))
        .map_or(text.len(), |(index, _)| index + 1)
}

fn validate_native_patch_path(path: &str) -> anyhow::Result<String> {
    let path = validate_patch_operation_path(path)?;
    let candidate = Path::new(&path);
    if candidate.is_absolute() {
        anyhow::bail!("unified diff conversion requires a workspace-relative path: {path}")
    }
    Ok(path.replace('\\', "/"))
}

fn validate_patch_operation_path(path: &str) -> anyhow::Result<String> {
    let path = path.trim();
    if path.is_empty() || path.chars().any(|ch| matches!(ch, '\r' | '\n' | '\0')) {
        anyhow::bail!("patch path must be a non-empty single line")
    }
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        anyhow::bail!("patch path cannot contain '..': {path}")
    }
    Ok(path.to_string())
}

fn extract_native_hunks(diff: &str) -> Option<String> {
    let normalized = diff.replace("\r\n", "\n");
    let start = normalized
        .split_inclusive('\n')
        .scan(0usize, |offset, line| {
            let current = *offset;
            *offset += line.len();
            Some((current, line))
        })
        .find_map(|(offset, line)| line.starts_with("@@").then_some(offset))?;
    let mut hunks = normalized[start..].to_string();
    if !hunks.ends_with('\n') {
        hunks.push('\n');
    }
    Some(hunks)
}

fn normalize_native_create_hunks(diff: &str) -> anyhow::Result<String> {
    if let Some(hunks) = extract_native_hunks(diff) {
        return Ok(hunks);
    }
    let normalized = diff.replace("\r\n", "\n");
    let had_final_newline = normalized.ends_with('\n');
    let body = normalized.strip_suffix('\n').unwrap_or(&normalized);
    if body.is_empty() {
        return Ok(String::new());
    }
    let lines = body.lines().collect::<Vec<_>>();
    let mut hunks = format!("@@ -0,0 +1,{} @@\n", lines.len());
    for line in lines {
        if line.starts_with('+') {
            hunks.push_str(line);
        } else {
            hunks.push('+');
            hunks.push_str(line);
        }
        hunks.push('\n');
    }
    if !had_final_newline {
        hunks.push_str("\\ No newline at end of file\n");
    }
    Ok(hunks)
}
