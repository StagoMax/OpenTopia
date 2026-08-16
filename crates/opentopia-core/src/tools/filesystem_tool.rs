use super::{
    normalize_workspace_path, tool_resource_key, ToolExecutionPolicy, ToolInvocationContext,
    ToolSideEffect, TypedTool,
};
use crate::execution::FileReadRequest;
use crate::execution_authorization::ToolExecutionIntent;
use crate::file_mutation::{read_optional, FileMutationBatch, PreparedFileMutation};
use crate::model::ToolResult;
use crate::model_context::content_fingerprint;
use crate::policy::PolicyDecision;
use anyhow::Context;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use uuid::Uuid;

const MAX_FILESYSTEM_READ_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_READ_WINDOW_CHARS: usize = 64_000;
const MAX_READ_WINDOW_CHARS: usize = 256_000;
const DEFAULT_LIST_ENTRIES: usize = 200;
const MAX_LIST_ENTRIES: usize = 1_000;
const MAX_WRITE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum FilesystemInput {
    Read {
        /// File path relative to the workspace, or an authorized absolute path.
        path: String,
        /// Zero-based character offset. Omit for the first window.
        #[serde(default)]
        offset: Option<u64>,
        /// Characters to return, capped at 256000.
        #[serde(default)]
        #[schemars(range(min = 1, max = 256000))]
        limit: Option<u64>,
    },
    Write {
        /// File path relative to the workspace, or an authorized absolute path.
        path: String,
        /// Complete UTF-8 file contents.
        content: String,
        /// Optional optimistic-concurrency token returned by read/stat. Use
        /// `missing` to require that the target does not already exist.
        #[serde(default)]
        expected_hash: Option<String>,
    },
    List {
        /// Directory path. Defaults to the workspace root.
        #[serde(default)]
        path: Option<String>,
        /// Maximum number of direct children to return.
        #[serde(default)]
        #[schemars(range(min = 1, max = 1000))]
        limit: Option<usize>,
    },
    Stat {
        /// File or directory path.
        path: String,
    },
    Copy {
        /// Existing source file.
        source: String,
        /// Destination file. Parent directories are created automatically.
        destination: String,
        /// Permit replacing an existing destination.
        #[serde(default)]
        overwrite: bool,
    },
    Move {
        /// Existing source file.
        source: String,
        /// Destination file. Parent directories are created automatically.
        destination: String,
        /// Permit replacing an existing destination.
        #[serde(default)]
        overwrite: bool,
    },
    Delete {
        /// Existing file to delete. Directory deletion is intentionally left
        /// to an explicitly reviewed shell command.
        path: String,
    },
}

impl FilesystemInput {
    fn read_paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Read { path, .. } | Self::Stat { path } => vec![PathBuf::from(path)],
            Self::List { path, .. } => {
                vec![PathBuf::from(path.as_deref().unwrap_or("."))]
            }
            Self::Copy { source, .. } | Self::Move { source, .. } => {
                vec![PathBuf::from(source)]
            }
            Self::Write { .. } | Self::Delete { .. } => Vec::new(),
        }
    }

    fn write_paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Write { path, .. } | Self::Delete { path } => vec![PathBuf::from(path)],
            Self::Copy { destination, .. } => vec![PathBuf::from(destination)],
            Self::Move {
                source,
                destination,
                ..
            } => vec![PathBuf::from(source), PathBuf::from(destination)],
            Self::Read { .. } | Self::List { .. } | Self::Stat { .. } => Vec::new(),
        }
    }

    fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::Read { .. } | Self::List { .. } | Self::Stat { .. }
        )
    }

    fn resource_keys(&self) -> Vec<String> {
        match self {
            Self::Read { path, .. }
            | Self::Write { path, .. }
            | Self::Stat { path }
            | Self::Delete { path } => vec![tool_resource_key("file", path)],
            Self::List { path, .. } => {
                vec![tool_resource_key("tree", path.as_deref().unwrap_or("."))]
            }
            Self::Copy {
                source,
                destination,
                ..
            }
            | Self::Move {
                source,
                destination,
                ..
            } => vec![
                tool_resource_key("file", source),
                tool_resource_key("file", destination),
            ],
        }
    }
}

pub struct FilesystemTool;

#[async_trait]
impl TypedTool for FilesystemTool {
    type Input = FilesystemInput;

    fn name(&self) -> &str {
        "filesystem"
    }

    fn description(&self) -> &str {
        "Perform structured filesystem operations under the active workspace policy. Supports bounded UTF-8 read, optimistic full-file write, direct-child list, stat, and transactional file copy/move/delete. Prefer apply_patch for ordinary source edits and shell for search, tests, generators, directory operations, or bulk transformations. Absolute paths are accepted only when the behavior permission gateway authorizes their exact scope."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        if input.is_read_only() {
            return ToolExecutionPolicy::read_only(input.resource_keys());
        }
        ToolExecutionPolicy {
            read_only: false,
            idempotent: matches!(
                input,
                FilesystemInput::Write { .. } | FilesystemInput::Copy { .. }
            ),
            parallel_safe: true,
            side_effect: ToolSideEffect::WorkspaceWrite,
            resource_keys: input.resource_keys(),
        }
    }

    fn execution_intent(&self, input: &Self::Input, _workspace_root: &Path) -> ToolExecutionIntent {
        let reads = input.read_paths();
        let writes = input.write_paths();
        if writes.is_empty() {
            ToolExecutionIntent::observation(reads)
        } else {
            ToolExecutionIntent::workspace_mutation(writes).with_read_paths(reads)
        }
    }

    fn authorization_preflight(
        &self,
        input: &Self::Input,
        _ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        matches!(input, FilesystemInput::Delete { .. }).then(|| PolicyDecision::Ask {
            reason: "Deleting a file through filesystem requires approval.".to_string(),
        })
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        match input {
            FilesystemInput::Read {
                path,
                offset,
                limit,
            } => read_file(call_id, &path, offset, limit, &ctx).await,
            FilesystemInput::Write {
                path,
                content,
                expected_hash,
            } => write_file(call_id, &path, content, expected_hash.as_deref(), &ctx).await,
            FilesystemInput::List { path, limit } => {
                list_directory(call_id, path.as_deref().unwrap_or("."), limit, &ctx).await
            }
            FilesystemInput::Stat { path } => stat_path(call_id, &path, &ctx).await,
            FilesystemInput::Copy {
                source,
                destination,
                overwrite,
            } => copy_file(call_id, &source, &destination, overwrite, &ctx).await,
            FilesystemInput::Move {
                source,
                destination,
                overwrite,
            } => move_file(call_id, &source, &destination, overwrite, &ctx).await,
            FilesystemInput::Delete { path } => delete_file(call_id, &path, &ctx).await,
        }
    }
}

async fn read_file(
    call_id: Uuid,
    raw_path: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let path = normalized_path(ctx, raw_path)?;
    let read = ctx
        .environment
        .read_file(FileReadRequest::new(&path).with_max_bytes(MAX_FILESYSTEM_READ_BYTES))
        .await?;
    let hash = content_fingerprint(&read.bytes);
    let contents = String::from_utf8(read.bytes).with_context(|| {
        format!(
            "filesystem read supports UTF-8 text only: {}",
            read.path.display()
        )
    })?;
    let total_chars = contents.chars().count();
    let offset = offset
        .map(usize::try_from)
        .transpose()
        .context("filesystem read offset is too large")?
        .unwrap_or(0);
    anyhow::ensure!(
        offset <= total_chars,
        "filesystem read offset {offset} exceeds total characters {total_chars}"
    );
    let limit = limit
        .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
        .unwrap_or(DEFAULT_READ_WINDOW_CHARS)
        .clamp(1, MAX_READ_WINDOW_CHARS);
    let output = contents
        .chars()
        .skip(offset)
        .take(limit)
        .collect::<String>();
    let next_offset = offset
        .saturating_add(output.chars().count())
        .lt(&total_chars)
        .then_some(offset.saturating_add(output.chars().count()));
    Ok(ToolResult {
        call_id,
        output,
        content: Vec::new(),
        metadata: json!({
            "toolName": "filesystem",
            "operation": "read",
            "path": read.path.display().to_string(),
            "contentHash": hash,
            "offset": offset,
            "nextOffset": next_offset,
            "totalChars": total_chars,
            "success": true
        }),
    })
}

async fn write_file(
    call_id: Uuid,
    raw_path: &str,
    content: String,
    expected_hash: Option<&str>,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    anyhow::ensure!(
        content.len() <= MAX_WRITE_BYTES,
        "filesystem write is {} bytes; limit is {MAX_WRITE_BYTES} bytes",
        content.len()
    );
    let path = normalized_path(ctx, raw_path)?;
    let original = read_optional(ctx.environment.as_ref(), &path).await?;
    verify_expected_hash(&path, original.as_deref(), expected_hash)?;
    let contents = content.into_bytes();
    let hash = content_fingerprint(&contents);
    FileMutationBatch::new(vec![PreparedFileMutation::write(
        path.clone(),
        original,
        contents.clone(),
    )])?
    .commit(ctx.environment.as_ref())
    .await?;
    Ok(ToolResult::text(
        call_id,
        format!("Wrote {} bytes to {}", contents.len(), path.display()),
        json!({
            "toolName": "filesystem",
            "operation": "write",
            "changedPath": path.display().to_string(),
            "bytes": contents.len(),
            "contentHash": hash,
            "success": true
        }),
    ))
}

async fn list_directory(
    call_id: Uuid,
    raw_path: &str,
    limit: Option<usize>,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let logical = normalized_path(ctx, raw_path)?;
    let path = ctx.environment.resolve_read_path(&logical)?;
    let limit = limit
        .unwrap_or(DEFAULT_LIST_ENTRIES)
        .clamp(1, MAX_LIST_ENTRIES);
    let mut directory = tokio::fs::read_dir(&path)
        .await
        .with_context(|| format!("failed to list {}", path.display()))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    while let Some(entry) = directory.next_entry().await? {
        if entries.len() == limit {
            truncated = true;
            break;
        }
        let file_type = entry.file_type().await?;
        let metadata = entry.metadata().await.ok();
        entries.push(FilesystemListEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind: if file_type.is_dir() {
                "directory"
            } else if file_type.is_file() {
                "file"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "other"
            },
            bytes: metadata
                .filter(|item| item.is_file())
                .map(|item| item.len()),
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    let value = json!({
        "path": path.display().to_string(),
        "entries": entries,
        "truncated": truncated
    });
    Ok(ToolResult::text(
        call_id,
        serde_json::to_string_pretty(&value)?,
        json!({
            "toolName": "filesystem",
            "operation": "list",
            "path": path.display().to_string(),
            "count": entries.len(),
            "truncated": truncated,
            "success": true
        }),
    ))
}

async fn stat_path(
    call_id: Uuid,
    raw_path: &str,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let logical = normalized_path(ctx, raw_path)?;
    let path = ctx.environment.resolve_read_path(&logical)?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis());
    let content_hash = if metadata.is_file() && metadata.len() <= MAX_FILESYSTEM_READ_BYTES {
        let read = ctx
            .environment
            .read_file(FileReadRequest::new(&path).with_max_bytes(MAX_FILESYSTEM_READ_BYTES))
            .await?;
        Some(content_fingerprint(&read.bytes))
    } else {
        None
    };
    let value = json!({
        "path": path.display().to_string(),
        "kind": if metadata.is_file() { "file" } else if metadata.is_dir() { "directory" } else { "other" },
        "bytes": metadata.len(),
        "readonly": metadata.permissions().readonly(),
        "modifiedUnixMs": modified_ms,
        "contentHash": content_hash
    });
    Ok(ToolResult::text(
        call_id,
        serde_json::to_string_pretty(&value)?,
        json!({
            "toolName": "filesystem",
            "operation": "stat",
            "success": true,
            "result": value
        }),
    ))
}

async fn copy_file(
    call_id: Uuid,
    raw_source: &str,
    raw_destination: &str,
    overwrite: bool,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let source = normalized_path(ctx, raw_source)?;
    let destination = normalized_path(ctx, raw_destination)?;
    let source_bytes = ctx
        .environment
        .read_file(FileReadRequest::new(&source))
        .await?
        .bytes;
    let destination_original = read_optional(ctx.environment.as_ref(), &destination).await?;
    anyhow::ensure!(
        overwrite || destination_original.is_none(),
        "filesystem copy destination already exists: {}",
        destination.display()
    );
    FileMutationBatch::new(vec![PreparedFileMutation::write(
        destination.clone(),
        destination_original,
        source_bytes.clone(),
    )])?
    .commit(ctx.environment.as_ref())
    .await?;
    Ok(file_transfer_result(
        call_id,
        "copy",
        &source,
        &destination,
        source_bytes.len(),
    ))
}

async fn move_file(
    call_id: Uuid,
    raw_source: &str,
    raw_destination: &str,
    overwrite: bool,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let source = normalized_path(ctx, raw_source)?;
    let destination = normalized_path(ctx, raw_destination)?;
    anyhow::ensure!(
        source != destination,
        "filesystem move source and destination are identical"
    );
    let source_bytes = ctx
        .environment
        .read_file(FileReadRequest::new(&source))
        .await?
        .bytes;
    let destination_original = read_optional(ctx.environment.as_ref(), &destination).await?;
    anyhow::ensure!(
        overwrite || destination_original.is_none(),
        "filesystem move destination already exists: {}",
        destination.display()
    );
    FileMutationBatch::new(vec![
        PreparedFileMutation::write(
            destination.clone(),
            destination_original,
            source_bytes.clone(),
        ),
        PreparedFileMutation::delete(source.clone(), source_bytes.clone()),
    ])?
    .commit(ctx.environment.as_ref())
    .await?;
    Ok(file_transfer_result(
        call_id,
        "move",
        &source,
        &destination,
        source_bytes.len(),
    ))
}

async fn delete_file(
    call_id: Uuid,
    raw_path: &str,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let path = normalized_path(ctx, raw_path)?;
    let original = ctx
        .environment
        .read_file(FileReadRequest::new(&path))
        .await?
        .bytes;
    FileMutationBatch::new(vec![PreparedFileMutation::delete(path.clone(), original)])?
        .commit(ctx.environment.as_ref())
        .await?;
    Ok(ToolResult::text(
        call_id,
        format!("Deleted {}", path.display()),
        json!({
            "toolName": "filesystem",
            "operation": "delete",
            "changedPath": path.display().to_string(),
            "success": true
        }),
    ))
}

fn normalized_path(ctx: &ToolInvocationContext, raw_path: &str) -> anyhow::Result<PathBuf> {
    let raw_path = raw_path.trim();
    anyhow::ensure!(!raw_path.is_empty(), "filesystem operation requires a path");
    normalize_workspace_path(&ctx.workspace_root, raw_path)
}

fn verify_expected_hash(
    path: &Path,
    original: Option<&[u8]>,
    expected_hash: Option<&str>,
) -> anyhow::Result<()> {
    let Some(expected_hash) = expected_hash
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let actual = original
        .map(content_fingerprint)
        .unwrap_or_else(|| "missing".to_string());
    anyhow::ensure!(
        actual.eq_ignore_ascii_case(expected_hash),
        "filesystem write precondition failed for {}: expected hash {}, actual {}",
        path.display(),
        expected_hash,
        actual
    );
    Ok(())
}

fn file_transfer_result(
    call_id: Uuid,
    operation: &str,
    source: &Path,
    destination: &Path,
    bytes: usize,
) -> ToolResult {
    ToolResult::text(
        call_id,
        format!(
            "{} {} to {}",
            if operation == "copy" {
                "Copied"
            } else {
                "Moved"
            },
            source.display(),
            destination.display()
        ),
        json!({
            "toolName": "filesystem",
            "operation": operation,
            "source": source.display().to_string(),
            "destination": destination.display().to_string(),
            "bytes": bytes,
            "success": true
        }),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilesystemListEntry {
    name: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolCall;
    use crate::policy::{BasicPolicyEngine, PermissionMode};
    use crate::tools::Tool;
    use serde_json::json;
    use std::fs;
    use std::sync::Arc;

    fn fixture() -> (PathBuf, ToolInvocationContext) {
        let root = std::env::temp_dir().join(format!("opentopia-filesystem-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let policy = Arc::new(BasicPolicyEngine::new(
            root.clone(),
            PermissionMode::Approve,
        ));
        let context = ToolInvocationContext::local(root.clone(), policy);
        (root, context)
    }

    #[tokio::test]
    async fn read_write_copy_move_round_trip() {
        let (root, context) = fixture();
        let written = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "write",
                        "path": "notes/a.txt",
                        "content": "alpha",
                        "expectedHash": "missing"
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            written.metadata["contentHash"],
            content_fingerprint(b"alpha")
        );

        let copied = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "copy",
                        "source": "notes/a.txt",
                        "destination": "notes/b.txt"
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        assert_eq!(copied.metadata["operation"], "copy");

        FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "move",
                        "source": "notes/b.txt",
                        "destination": "notes/c.txt"
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        assert!(!root.join("notes/b.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("notes/c.txt")).unwrap(),
            "alpha"
        );

        let read = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({ "operation": "read", "path": "notes/c.txt" }),
                ),
                context,
            )
            .await
            .unwrap();
        assert_eq!(read.output, "alpha");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn stale_hash_rejects_overwrite() {
        let (root, context) = fixture();
        fs::write(root.join("value.txt"), "current").unwrap();
        let error = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "write",
                        "path": "value.txt",
                        "content": "replacement",
                        "expectedHash": "stale"
                    }),
                ),
                context,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("precondition failed"));
        assert_eq!(
            fs::read_to_string(root.join("value.txt")).unwrap(),
            "current"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delete_is_always_an_approval_boundary() {
        let (root, context) = fixture();
        let call = ToolCall::new(
            "filesystem",
            json!({ "operation": "delete", "path": "value.txt" }),
        );
        assert!(matches!(
            Tool::authorization_preflight(&FilesystemTool, &call, &context),
            Some(PolicyDecision::Ask { .. })
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
