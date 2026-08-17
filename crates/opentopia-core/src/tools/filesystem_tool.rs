use super::{
    normalize_workspace_path, tool_resource_key, ToolExecutionPolicy, ToolInvocationContext,
    ToolSideEffect, TypedTool,
};
use crate::execution::FileReadRequest;
use crate::execution_authorization::ToolExecutionIntent;
use crate::file_mutation::{
    normalize_text_encoding, read_optional, FileMutationBatch, PreparedFileMutation,
};
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
const DEFAULT_READ_WINDOW_CHARS: usize = 12_000;
const MAX_READ_WINDOW_CHARS: usize = 256_000;
const DEFAULT_LIST_ENTRIES: usize = 200;
const MAX_LIST_ENTRIES: usize = 1_000;
const DEFAULT_FIND_DEPTH: usize = 16;
const MAX_FIND_DEPTH: usize = 64;
const MAX_FIND_QUERY_CHARS: usize = 256;
const MAX_FIND_VISITED_ENTRIES: usize = 100_000;
const MAX_WRITE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(super) enum FilesystemFindKind {
    #[default]
    Any,
    File,
    Directory,
}

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
        #[schemars(rename = "expectedHash")]
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
    Find {
        /// Directory to search recursively. Defaults to the workspace root.
        #[serde(default)]
        path: Option<String>,
        /// Literal substring matched against each entry's file name.
        #[schemars(rename = "nameContains", length(min = 1, max = 256))]
        name_contains: String,
        /// Match file names case-sensitively. Defaults to false.
        #[serde(default)]
        #[schemars(rename = "caseSensitive")]
        case_sensitive: bool,
        /// Restrict matches to files or directories. Defaults to any entry kind.
        #[serde(default)]
        kind: FilesystemFindKind,
        /// Maximum directory depth below path. One searches direct children only.
        #[serde(default)]
        #[schemars(rename = "maxDepth", range(min = 1, max = 64))]
        max_depth: Option<usize>,
        /// Maximum matches to return. Defaults to 200 and is capped at 1000.
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
            Self::List { path, .. } | Self::Find { path, .. } => {
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
            Self::Read { .. } | Self::List { .. } | Self::Find { .. } | Self::Stat { .. } => {
                Vec::new()
            }
        }
    }

    fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::Read { .. } | Self::List { .. } | Self::Find { .. } | Self::Stat { .. }
        )
    }

    fn resource_keys(&self) -> Vec<String> {
        match self {
            Self::Read { path, .. }
            | Self::Write { path, .. }
            | Self::Stat { path }
            | Self::Delete { path } => vec![tool_resource_key("file", path)],
            Self::List { path, .. } | Self::Find { path, .. } => {
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
        "Perform structured filesystem operations under the active workspace policy. Supports bounded UTF-8 read, optimistic full-file write, direct-child list, bounded recursive find by literal file-name substring, stat, and transactional file copy/move/delete. Prefer apply_patch for ordinary source edits and shell with rg for content search, tests, generators, directory operations, or bulk transformations. Absolute paths are accepted only when the behavior permission gateway authorizes their exact scope."
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
            FilesystemInput::Find {
                path,
                name_contains,
                case_sensitive,
                kind,
                max_depth,
                limit,
            } => {
                find_entries(
                    call_id,
                    path.as_deref().unwrap_or("."),
                    &name_contains,
                    case_sensitive,
                    kind,
                    max_depth,
                    limit,
                    &ctx,
                )
                .await
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
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(&contents);
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
    let visible_path = model_visible_path(ctx, &read.path);
    Ok(ToolResult {
        call_id,
        output,
        content: Vec::new(),
        metadata: json!({
            "toolName": "filesystem",
            "operation": "read",
            "path": visible_path,
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
    let path = normalized_path(ctx, raw_path)?;
    let original = read_optional(ctx.environment.as_ref(), &path).await?;
    verify_expected_hash(&path, original.as_deref(), expected_hash)?;
    let contents = normalize_text_encoding(&path, content.into_bytes());
    anyhow::ensure!(
        contents.len() <= MAX_WRITE_BYTES,
        "filesystem write is {} bytes; limit is {MAX_WRITE_BYTES} bytes",
        contents.len()
    );
    let hash = content_fingerprint(&contents);
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
        path.clone(),
        original,
        contents.clone(),
    )])?;
    ctx.commit_file_mutations(&batch).await?;
    let visible_path = model_visible_path(ctx, &path);
    Ok(ToolResult::text(
        call_id,
        format!("Wrote {} bytes to {visible_path}", contents.len()),
        json!({
            "toolName": "filesystem",
            "operation": "write",
            "changedPath": visible_path,
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
    let value = json!({ "entries": entries, "truncated": truncated });
    Ok(ToolResult::text(
        call_id,
        serde_json::to_string_pretty(&value)?,
        json!({
            "toolName": "filesystem",
            "operation": "list",
            "path": model_visible_path(ctx, &path),
            "count": entries.len(),
            "truncated": truncated,
            "success": true
        }),
    ))
}

async fn find_entries(
    call_id: Uuid,
    raw_path: &str,
    raw_name_contains: &str,
    case_sensitive: bool,
    kind: FilesystemFindKind,
    max_depth: Option<usize>,
    limit: Option<usize>,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let name_contains = raw_name_contains.trim();
    anyhow::ensure!(
        !name_contains.is_empty(),
        "filesystem find requires a non-empty nameContains value"
    );
    anyhow::ensure!(
        name_contains.chars().count() <= MAX_FIND_QUERY_CHARS,
        "filesystem find nameContains exceeds {MAX_FIND_QUERY_CHARS} characters"
    );
    let max_depth = max_depth.unwrap_or(DEFAULT_FIND_DEPTH);
    anyhow::ensure!(
        (1..=MAX_FIND_DEPTH).contains(&max_depth),
        "filesystem find maxDepth must be between 1 and {MAX_FIND_DEPTH}"
    );
    let limit = limit
        .unwrap_or(DEFAULT_LIST_ENTRIES)
        .clamp(1, MAX_LIST_ENTRIES);

    let logical = normalized_path(ctx, raw_path)?;
    let root = ctx.environment.resolve_read_path(&logical)?;
    anyhow::ensure!(
        tokio::fs::metadata(&root).await?.is_dir(),
        "filesystem find path is not a directory: {}",
        root.display()
    );

    let match_query = if case_sensitive {
        name_contains.to_string()
    } else {
        name_contains.to_lowercase()
    };
    let mut pending = vec![(root.clone(), 0usize)];
    let mut matches = Vec::new();
    let mut visited_entries = 0usize;
    let mut skipped_directories = 0usize;
    let mut truncation_reason = None;

    'search: while let Some((directory_path, directory_depth)) = pending.pop() {
        let mut directory = match tokio::fs::read_dir(&directory_path).await {
            Ok(directory) => directory,
            Err(error) if directory_depth > 0 => {
                let _ = error;
                skipped_directories += 1;
                continue;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to search {}", directory_path.display()));
            }
        };
        let mut children = Vec::new();
        let remaining_visit_budget = MAX_FIND_VISITED_ENTRIES - visited_entries;
        let mut directory_truncated = false;
        while let Some(entry) = directory.next_entry().await? {
            if children.len() == remaining_visit_budget {
                truncation_reason = Some("visit_limit");
                directory_truncated = true;
                break;
            }
            children.push(entry);
        }
        children.sort_by_key(|entry| entry.file_name().to_string_lossy().into_owned());

        let entry_depth = directory_depth + 1;
        let mut descendant_directories = Vec::new();
        for entry in children {
            visited_entries += 1;

            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let candidate = if case_sensitive {
                name.clone()
            } else {
                name.to_lowercase()
            };
            if candidate.contains(&match_query) && kind.matches(&file_type) {
                if matches.len() == limit {
                    truncation_reason = Some("match_limit");
                    break 'search;
                }
                let metadata = if file_type.is_file() {
                    entry.metadata().await.ok()
                } else {
                    None
                };
                matches.push(FilesystemFindEntry {
                    path: model_visible_path(ctx, &entry_path),
                    kind: filesystem_entry_kind(&file_type),
                    bytes: metadata.map(|item| item.len()),
                });
            }
            if file_type.is_dir() && entry_depth < max_depth {
                descendant_directories.push(entry_path);
            }
        }
        descendant_directories.reverse();
        pending.extend(
            descendant_directories
                .into_iter()
                .map(|path| (path, entry_depth)),
        );
        if directory_truncated {
            break 'search;
        }
    }

    let count = matches.len();
    let root_display = model_visible_path(ctx, &root);
    let truncated = truncation_reason.is_some();
    let value = json!({
        "entries": matches,
        "visitedEntries": visited_entries,
        "truncated": truncated,
        "truncationReason": truncation_reason,
        "skippedDirectories": skipped_directories
    });
    Ok(ToolResult::text(
        call_id,
        serde_json::to_string_pretty(&value)?,
        json!({
            "toolName": "filesystem",
            "operation": "find",
            "path": root_display,
            "count": count,
            "visitedEntries": visited_entries,
            "truncated": truncated,
            "truncationReason": truncation_reason,
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
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
        destination.clone(),
        destination_original,
        source_bytes.clone(),
    )])?;
    ctx.commit_file_mutations(&batch).await?;
    Ok(file_transfer_result(
        call_id,
        "copy",
        &source,
        &destination,
        source_bytes.len(),
        &ctx,
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
    let batch = FileMutationBatch::new(vec![
        PreparedFileMutation::write(
            destination.clone(),
            destination_original,
            source_bytes.clone(),
        ),
        PreparedFileMutation::delete(source.clone(), source_bytes.clone()),
    ])?;
    ctx.commit_file_mutations(&batch).await?;
    Ok(file_transfer_result(
        call_id,
        "move",
        &source,
        &destination,
        source_bytes.len(),
        &ctx,
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
    let batch = FileMutationBatch::new(vec![PreparedFileMutation::delete(path.clone(), original)])?;
    ctx.commit_file_mutations(&batch).await?;
    let visible_path = model_visible_path(ctx, &path);
    Ok(ToolResult::text(
        call_id,
        format!("Deleted {visible_path}"),
        json!({
            "toolName": "filesystem",
            "operation": "delete",
            "changedPath": visible_path,
            "success": true
        }),
    ))
}

fn normalized_path(ctx: &ToolInvocationContext, raw_path: &str) -> anyhow::Result<PathBuf> {
    let raw_path = raw_path.trim();
    anyhow::ensure!(!raw_path.is_empty(), "filesystem operation requires a path");
    normalize_workspace_path(&ctx.workspace_root, raw_path)
}

impl FilesystemFindKind {
    fn matches(self, file_type: &std::fs::FileType) -> bool {
        match self {
            Self::Any => true,
            Self::File => file_type.is_file(),
            Self::Directory => file_type.is_dir(),
        }
    }
}

fn filesystem_entry_kind(file_type: &std::fs::FileType) -> &'static str {
    if file_type.is_dir() {
        "directory"
    } else if file_type.is_file() {
        "file"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

fn model_visible_path(ctx: &ToolInvocationContext, path: &Path) -> String {
    let relative = path
        .strip_prefix(&ctx.workspace_root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let workspace_root = ctx.workspace_root.canonicalize().ok()?;
            let canonical_path = path.canonicalize().ok()?;
            canonical_path
                .strip_prefix(workspace_root)
                .ok()
                .map(Path::to_path_buf)
        });
    match relative {
        Some(path) if path.as_os_str().is_empty() => ".".to_string(),
        Some(path) => path.to_string_lossy().replace('\\', "/"),
        None => path.to_string_lossy().replace('\\', "/"),
    }
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
        anyhow::ensure!(
            original.is_none(),
            "filesystem write requires expectedHash when replacing existing file {}; reread it and retry",
            path.display()
        );
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
    ctx: &ToolInvocationContext,
) -> ToolResult {
    let source = model_visible_path(ctx, source);
    let destination = model_visible_path(ctx, destination);
    ToolResult::text(
        call_id,
        format!(
            "{} {} to {}",
            if operation == "copy" {
                "Copied"
            } else {
                "Moved"
            },
            source,
            destination
        ),
        json!({
            "toolName": "filesystem",
            "operation": operation,
            "source": source,
            "destination": destination,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilesystemFindEntry {
    path: String,
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
        assert_eq!(written.metadata["changedPath"], "notes/a.txt");

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
        assert_eq!(copied.metadata["source"], "notes/a.txt");
        assert_eq!(copied.metadata["destination"], "notes/b.txt");

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
        assert_eq!(read.metadata["path"], "notes/c.txt");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn powershell_write_adds_bom_without_exposing_it_as_text() {
        let (root, context) = fixture();
        let script = "Write-Output \"中文（全角括号）\"";
        let written = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "write",
                        "path": "scripts/unicode.ps1",
                        "content": script,
                        "expectedHash": "missing"
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();

        let bytes = fs::read(root.join("scripts/unicode.ps1")).unwrap();
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        assert_eq!(written.metadata["contentHash"], content_fingerprint(&bytes));

        let read = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({ "operation": "read", "path": "scripts/unicode.ps1" }),
                ),
                context,
            )
            .await
            .unwrap();
        assert_eq!(read.output, script);
        assert_eq!(read.metadata["totalChars"], script.chars().count());
        assert_eq!(read.metadata["contentHash"], content_fingerprint(&bytes));
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn default_read_window_is_token_bounded_and_pageable() {
        let (root, context) = fixture();
        fs::write(
            root.join("long.txt"),
            "x".repeat(DEFAULT_READ_WINDOW_CHARS + 10),
        )
        .unwrap();
        let read = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({ "operation": "read", "path": "long.txt" }),
                ),
                context,
            )
            .await
            .unwrap();

        assert_eq!(read.output.chars().count(), DEFAULT_READ_WINDOW_CHARS);
        assert_eq!(read.metadata["nextOffset"], DEFAULT_READ_WINDOW_CHARS);
        assert_eq!(read.metadata["path"], "long.txt");
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

    #[tokio::test]
    async fn find_matches_file_name_substrings_recursively_with_bounds() {
        let (root, context) = fixture();
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("src/alpha.rs"), "alpha").unwrap();
        fs::write(root.join("src/nested/ALPHA.md"), "alpha").unwrap();
        fs::write(root.join("src/nested/beta.txt"), "beta").unwrap();

        let found = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "find",
                        "path": "src",
                        "nameContains": "alpha",
                        "caseSensitive": false,
                        "kind": "file",
                        "maxDepth": 3,
                        "limit": 10
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&found.output).unwrap();
        let paths = value["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["src/alpha.rs", "src/nested/ALPHA.md"]);
        assert_eq!(value["truncated"], false);

        let shallow = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "find",
                        "path": "src",
                        "nameContains": "alpha",
                        "kind": "file",
                        "maxDepth": 1,
                        "limit": 10
                    }),
                ),
                context.clone(),
            )
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&shallow.output).unwrap();
        assert_eq!(value["entries"].as_array().unwrap().len(), 1);

        let limited = FilesystemTool
            .execute(
                ToolCall::new(
                    "filesystem",
                    json!({
                        "operation": "find",
                        "path": "src",
                        "nameContains": "a",
                        "kind": "file",
                        "maxDepth": 3,
                        "limit": 1
                    }),
                ),
                context,
            )
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&limited.output).unwrap();
        assert_eq!(value["entries"].as_array().unwrap().len(), 1);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["truncationReason"], "match_limit");
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

    #[test]
    fn provider_schema_and_serde_accept_the_same_camel_case_write_input() {
        let input = json!({
            "operation": "write",
            "path": "value.txt",
            "content": "replacement",
            "expectedHash": "missing"
        });
        assert_eq!(Tool::input_error(&FilesystemTool, &input), None);
        assert!(serde_json::from_value::<FilesystemInput>(input).is_ok());
    }

    #[test]
    fn provider_schema_and_serde_accept_the_same_camel_case_find_input() {
        let input = json!({
            "operation": "find",
            "path": ".",
            "nameContains": "policy",
            "caseSensitive": false,
            "kind": "file",
            "maxDepth": 8,
            "limit": 200
        });
        assert_eq!(Tool::input_error(&FilesystemTool, &input), None);
        assert!(serde_json::from_value::<FilesystemInput>(input).is_ok());
    }
}
