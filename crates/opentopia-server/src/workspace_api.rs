use super::{
    current_settings, ensure_thread, publish_payload, truncate_with_flag, ApiError, AppState,
};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use opentopia_core::{
    AgentEventPayload, CapabilityProjection, ChangedFile, ExecutionAuthority, ModelContentPart,
    PermissionMode, SandboxMode, Tool, ToolCall, ToolResult, ToolStateStore, WorkspaceDiff,
    WorkspaceDiffHunk, WorkspaceDiffScope, WorkspaceEntry, WorkspaceEntryKind,
    WorkspaceFilePreview, WorkspaceSearchTool, WorkspaceTree, GIT_NONINTERACTIVE_ENVIRONMENT,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use uuid::Uuid;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/threads/:thread_id/workspace/tree",
            get(list_workspace_tree),
        )
        .route(
            "/api/threads/:thread_id/workspace/file",
            get(read_workspace_file),
        )
        .route(
            "/api/threads/:thread_id/workspace/search",
            post(search_workspace),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff",
            get(get_workspace_diff),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff/revert",
            post(revert_workspace_file),
        )
        .route(
            "/api/threads/:thread_id/workspace/diff/hunk",
            post(apply_workspace_diff_hunk),
        )
}

async fn list_workspace_tree(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceTree>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let path = resolve_workspace_path(&root, query.path.as_deref())?;
    let entries = list_workspace_entries(&root, &path)?;
    Ok(Json(WorkspaceTree {
        root,
        path: relative_workspace_path(&thread.workspace_root, &path),
        entries,
    }))
}

async fn read_workspace_file(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Query(query): Query<WorkspacePathQuery>,
) -> Result<Json<WorkspaceFilePreview>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let path = resolve_workspace_path(&root, query.path.as_deref())?;
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| ApiError::not_found(format!("file not found: {}", path.display())))?;
    if !metadata.is_file() {
        return Err(ApiError::bad_request(format!(
            "path is not a file: {}",
            path.display()
        )));
    }

    let bytes = tokio::fs::read(&path).await?;
    let content = String::from_utf8_lossy(&bytes);
    let (content, truncated) = truncate_with_flag(&content, 64_000);
    Ok(Json(WorkspaceFilePreview {
        path: relative_workspace_path(&root, &path),
        content,
        bytes: bytes.len(),
        truncated,
        readonly: true,
    }))
}

/// A deterministic, read-only workspace-search entry point for the UI and
/// integration tests. It deliberately does not go through `send_message`, so
/// the result cannot be affected by provider availability or model behaviour.
async fn search_workspace(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<WorkspaceSearchRequest>,
) -> Result<Json<ToolResult>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let settings = current_settings(&state);
    let sandbox_config = settings
        .sandbox
        .to_local_sandbox_config()
        .with_sandbox_mode(SandboxMode::ReadOnly);
    let mut capabilities = CapabilityProjection::deny_all();
    capabilities.tools.insert("workspace_search".to_string());
    capabilities
        .workspace_roots
        .insert(thread.workspace_root.clone());
    let authority = ExecutionAuthority::new(
        thread.workspace_root.clone(),
        PermissionMode::ReadOnly,
        sandbox_config,
        capabilities,
    )?;
    let call = ToolCall::new(
        "workspace_search",
        json!({
            "query": request.query,
            "path": request.path,
            "fixedStrings": request.fixed_strings,
            "wordMatch": request.word_match,
            "maxResults": request.max_results,
        }),
    );
    publish_payload(
        &state,
        thread_id,
        None,
        AgentEventPayload::ToolCallStarted { call: call.clone() },
    );

    let mut context = authority.local_tool_context();
    context.state = Some(ToolStateStore::new(state.store.clone()));
    context.thread_id = Some(thread_id);
    let result = WorkspaceSearchTool.execute(call.clone(), context).await;
    match result {
        Ok(mut result) => {
            if let Some(metadata) = result.metadata.as_object_mut() {
                metadata.insert("toolName".to_string(), json!("workspace_search"));
                metadata.insert("success".to_string(), json!(true));
            }
            publish_payload(
                &state,
                thread_id,
                None,
                AgentEventPayload::ToolCallFinished {
                    result: result.clone(),
                },
            );
            Ok(Json(result))
        }
        Err(error) => {
            let message = error.to_string();
            let result = ToolResult {
                call_id: call.id,
                output: message.clone(),
                content: vec![ModelContentPart::text(message.clone())],
                metadata: json!({
                    "toolName": "workspace_search",
                    "success": false,
                    "error": message,
                }),
            };
            publish_payload(
                &state,
                thread_id,
                None,
                AgentEventPayload::ToolCallFinished { result },
            );
            Err(ApiError::bad_request(message))
        }
    }
}

async fn get_workspace_diff(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
) -> Result<Json<WorkspaceDiff>, ApiError> {
    let thread = ensure_thread(&state, thread_id)?;
    let diff = get_workspace_diff_inner(&thread.workspace_root).await?;
    Ok(Json(diff))
}

async fn revert_workspace_file(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<WorkspaceDiffRevertRequest>,
) -> Result<Json<WorkspaceDiffActionResponse>, ApiError> {
    if !request.confirm {
        return Err(ApiError::bad_request(
            "confirm must be true to revert a workspace file",
        ));
    }
    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let relative_path = validate_relative_git_path(&request.path)?;

    let status_output = run_git(
        &root,
        ["status", "--porcelain=v1", "--", relative_path.as_str()],
    )
    .await?;
    if status_output.trim().is_empty() {
        return Err(ApiError::bad_request(format!(
            "no working-tree change found for {}",
            relative_path
        )));
    }
    let status_files = parse_git_status(&status_output);
    let changed_file = status_files
        .iter()
        .find(|file| normalized_path_string(&file.path) == relative_path)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "no working-tree change found for {}",
                relative_path
            ))
        })?;
    if changed_file.is_untracked {
        return Err(ApiError::bad_request(
            "untracked files are not reverted by this safe action",
        ));
    }
    if changed_file.is_renamed {
        return Err(ApiError::bad_request(
            "renamed paths must be reverted manually for now",
        ));
    }
    if !changed_file.staged_status.is_empty() {
        return Err(ApiError::bad_request(
            "files with staged changes must be handled manually before worktree restore",
        ));
    }
    if !matches!(
        changed_file.unstaged_status.as_str(),
        "modified" | "deleted"
    ) {
        return Err(ApiError::bad_request(
            "only unstaged modified or deleted tracked files can be restored",
        ));
    }

    run_git(
        &root,
        ["ls-files", "--error-unmatch", "--", relative_path.as_str()],
    )
    .await?;
    run_git(
        &root,
        [
            "restore",
            "--source=HEAD",
            "--worktree",
            "--",
            relative_path.as_str(),
        ],
    )
    .await?;
    let diff = get_workspace_diff_inner(&root).await?;
    Ok(Json(WorkspaceDiffActionResponse {
        path: PathBuf::from(relative_path),
        diff,
    }))
}

async fn apply_workspace_diff_hunk(
    State(state): State<AppState>,
    Path(thread_id): Path<Uuid>,
    Json(request): Json<WorkspaceDiffHunkActionRequest>,
) -> Result<Json<WorkspaceDiffActionResponse>, ApiError> {
    if !request.confirm {
        return Err(ApiError::bad_request(
            "confirm must be true to change a workspace diff hunk",
        ));
    }
    if request.patch.len() > 100_000 {
        return Err(ApiError::bad_request("hunk patch is too large"));
    }

    let thread = ensure_thread(&state, thread_id)?;
    let root = canonical_workspace_root(&thread.workspace_root);
    let relative_path = validate_relative_git_path(&request.path)?;
    let current_diff = get_workspace_diff_inner(&root).await?;
    let current_hunk = current_diff.hunks.iter().find(|hunk| {
        normalized_path_string(&hunk.path) == relative_path
            && hunk.scope == request.scope
            && hunk.patch == request.patch
    });
    if current_hunk.is_none() {
        return Err(ApiError::conflict(
            "the selected hunk no longer matches the current workspace diff; refresh and retry",
        ));
    }

    let args: &[&str] = match (request.scope, request.action) {
        (WorkspaceDiffScope::Unstaged, WorkspaceDiffHunkAction::Stage) => &["apply", "--cached"],
        (WorkspaceDiffScope::Staged, WorkspaceDiffHunkAction::Unstage) => {
            &["apply", "--cached", "--reverse"]
        }
        (WorkspaceDiffScope::Unstaged, WorkspaceDiffHunkAction::Discard) => &["apply", "--reverse"],
        _ => {
            return Err(ApiError::bad_request(
                "invalid action for the selected diff scope",
            ))
        }
    };
    let mut check_args = args.to_vec();
    check_args.push("--check");
    run_git_with_input(&root, &check_args, &request.patch).await?;
    run_git_with_input(&root, args, &request.patch).await?;

    let diff = get_workspace_diff_inner(&root).await?;
    Ok(Json(WorkspaceDiffActionResponse {
        path: PathBuf::from(relative_path),
        diff,
    }))
}

pub(super) async fn get_workspace_diff_inner(
    workspace_root: &FsPath,
) -> anyhow::Result<WorkspaceDiff> {
    let (branch_output, remote_output, status_output, staged_output, unstaged_output) = tokio::join!(
        run_git(workspace_root, ["symbolic-ref", "--short", "HEAD"]),
        run_git(workspace_root, ["remote", "get-url", "origin"]),
        run_git(workspace_root, ["status", "--porcelain=v1"]),
        run_git(workspace_root, ["diff", "--cached", "--"]),
        run_git(workspace_root, ["diff", "--"]),
    );
    let branch = branch_output
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let remote_url = remote_output
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let status_output = status_output.unwrap_or_else(|_| String::new());
    let staged_output = staged_output.unwrap_or_else(|_| String::new());
    let unstaged_output = unstaged_output.unwrap_or_else(|_| String::new());
    let files = parse_git_status(&status_output);
    let (staged_diff, staged_truncated) = truncate_with_flag(&staged_output, 80_000);
    let (unstaged_diff, unstaged_truncated) = truncate_with_flag(&unstaged_output, 80_000);
    let mut hunks = parse_workspace_diff_hunks(&staged_diff, WorkspaceDiffScope::Staged);
    hunks.extend(parse_workspace_diff_hunks(
        &unstaged_diff,
        WorkspaceDiffScope::Unstaged,
    ));
    let diff = combine_workspace_diffs(&staged_diff, &unstaged_diff);
    Ok(WorkspaceDiff {
        command: "git diff --cached -- && git diff --".to_string(),
        branch,
        remote_url,
        files,
        diff,
        staged_diff,
        unstaged_diff,
        hunks,
        truncated: staged_truncated || unstaged_truncated,
        staged_truncated,
        unstaged_truncated,
    })
}

pub(super) fn canonical_workspace_root(workspace_root: &FsPath) -> PathBuf {
    workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf())
}

fn resolve_workspace_path(root: &FsPath, requested: Option<&str>) -> Result<PathBuf, ApiError> {
    let requested = requested.unwrap_or(".").trim();
    let requested = if requested.is_empty() { "." } else { requested };
    let raw = PathBuf::from(requested);
    if raw
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ApiError::bad_request("workspace path cannot contain .."));
    }
    let candidate = if raw.is_absolute() {
        raw
    } else {
        root.join(raw)
    };
    let resolved = candidate.canonicalize().map_err(|_| {
        ApiError::not_found(format!("workspace path not found: {}", candidate.display()))
    })?;
    if !resolved.starts_with(root) {
        return Err(ApiError::bad_request(format!(
            "path is outside workspace: {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn list_workspace_entries(root: &FsPath, path: &FsPath) -> Result<Vec<WorkspaceEntry>, ApiError> {
    let metadata = std::fs::metadata(path)
        .map_err(|_| ApiError::not_found(format!("path not found: {}", path.display())))?;
    if !metadata.is_dir() {
        return Err(ApiError::bad_request(format!(
            "path is not a directory: {}",
            path.display()
        )));
    }

    let mut entries = std::fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            let entry_path = entry.path();
            let metadata = entry.metadata()?;
            let file_type = entry.file_type()?;
            let kind = if file_type.is_symlink() {
                WorkspaceEntryKind::Symlink
            } else if metadata.is_dir() {
                WorkspaceEntryKind::Directory
            } else if metadata.is_file() {
                WorkspaceEntryKind::File
            } else {
                WorkspaceEntryKind::Other
            };
            Ok(WorkspaceEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                path: relative_workspace_path(root, &entry_path),
                kind,
                size: metadata.is_file().then_some(metadata.len()),
                modified_at: metadata.modified().ok().map(DateTime::<Utc>::from),
            })
        })
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    entries.sort_by(|left, right| {
        let left_dir = left.kind == WorkspaceEntryKind::Directory;
        let right_dir = right.kind == WorkspaceEntryKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

fn relative_workspace_path(root: &FsPath, path: &FsPath) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

fn validate_relative_git_path(path: &str) -> Result<String, ApiError> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        return Err(ApiError::bad_request("path cannot be empty"));
    }
    if normalized.contains(" -> ") {
        return Err(ApiError::bad_request(
            "renamed paths must be reverted manually for now",
        ));
    }
    let path_buf = PathBuf::from(&normalized);
    if path_buf.is_absolute()
        || path_buf.components().any(|component| {
            matches!(
                component,
                std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
                    | std::path::Component::ParentDir
            )
        })
    {
        return Err(ApiError::bad_request(
            "path must be a relative workspace path without ..",
        ));
    }
    Ok(normalized)
}

pub(super) async fn run_git<const N: usize>(
    workspace_root: &FsPath,
    args: [&str; N],
) -> Result<String, ApiError> {
    let output = timeout(
        Duration::from_secs(20),
        Command::new("git")
            .envs(GIT_NONINTERACTIVE_ENVIRONMENT)
            .args(args)
            .current_dir(workspace_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| ApiError::bad_request("git command timed out"))??;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn run_git_with_input(
    workspace_root: &FsPath,
    args: &[&str],
    input: &str,
) -> Result<String, ApiError> {
    let mut child = Command::new("git")
        .envs(GIT_NONINTERACTIVE_ENVIRONMENT)
        .args(args)
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await?;
    }
    let output = timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .map_err(|_| ApiError::bad_request("git command timed out"))??;
    if !output.status.success() {
        return Err(ApiError::bad_request(format!(
            "git command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_git_status(output: &str) -> Vec<ChangedFile> {
    output
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let status_code = &line[..2];
            let mut path = line[3..].trim();
            if path.is_empty() {
                return None;
            }
            let mut original_path = None;
            let is_renamed = status_code.contains('R') || status_code.contains('C');
            if is_renamed {
                if let Some((original, renamed)) = path.split_once(" -> ") {
                    original_path = Some(PathBuf::from(original));
                    path = renamed;
                }
            }
            let is_untracked = status_code == "??";
            let staged_status = if is_untracked {
                String::new()
            } else {
                git_status_name(status_code.chars().next().unwrap_or(' '))
            };
            let unstaged_status = if is_untracked {
                "untracked".to_string()
            } else {
                git_status_name(status_code.chars().nth(1).unwrap_or(' '))
            };
            let status = if is_untracked {
                "??".to_string()
            } else {
                status_code.trim().to_string()
            };
            Some(ChangedFile {
                path: PathBuf::from(path),
                status,
                staged_status,
                unstaged_status,
                original_path,
                is_untracked,
                is_renamed,
            })
        })
        .collect()
}

fn git_status_name(status: char) -> String {
    match status {
        'M' => "modified",
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'U' => "unmerged",
        '?' => "untracked",
        '!' => "ignored",
        _ => "",
    }
    .to_string()
}

fn combine_workspace_diffs(staged_diff: &str, unstaged_diff: &str) -> String {
    match (
        staged_diff.trim().is_empty(),
        unstaged_diff.trim().is_empty(),
    ) {
        (true, true) => String::new(),
        (false, true) => staged_diff.to_string(),
        (true, false) => unstaged_diff.to_string(),
        (false, false) => format!(
            "# staged: git diff --cached --\n{}\n\n# unstaged: git diff --\n{}",
            staged_diff.trim_end(),
            unstaged_diff.trim_start()
        ),
    }
}

fn parse_workspace_diff_hunks(diff: &str, scope: WorkspaceDiffScope) -> Vec<WorkspaceDiffHunk> {
    let mut hunks = Vec::new();
    let mut current_path: Option<PathBuf> = None;
    let mut current_hunk: Option<WorkspaceDiffHunk> = None;
    let mut current_file_header = Vec::new();

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            push_diff_hunk(&mut hunks, &mut current_hunk);
            current_path = parse_diff_git_path(line);
            current_file_header.clear();
            current_file_header.push(line.to_string());
            continue;
        }

        if let Some(path) = line.strip_prefix("--- ") {
            if current_path.is_none() {
                current_path = parse_diff_marker_path(path);
            }
            current_file_header.push(line.to_string());
            continue;
        }

        if let Some(path) = line.strip_prefix("+++ ") {
            if let Some(parsed_path) = parse_diff_marker_path(path) {
                current_path = Some(parsed_path);
            }
            current_file_header.push(line.to_string());
            continue;
        }

        if line.starts_with("@@ ") {
            push_diff_hunk(&mut hunks, &mut current_hunk);
            if let Some(path) = current_path.clone() {
                let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(line);
                current_hunk = Some(WorkspaceDiffHunk {
                    path,
                    scope,
                    header: line.to_string(),
                    lines: Vec::new(),
                    raw: line.to_string(),
                    patch: format!("{}\n{}\n", current_file_header.join("\n"), line),
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                });
            }
            continue;
        }

        if let Some(hunk) = &mut current_hunk {
            hunk.lines.push(line.to_string());
            hunk.raw.push('\n');
            hunk.raw.push_str(line);
            hunk.patch.push_str(line);
            hunk.patch.push('\n');
        } else if !current_file_header.is_empty() {
            current_file_header.push(line.to_string());
        }
    }

    push_diff_hunk(&mut hunks, &mut current_hunk);
    hunks
}

fn push_diff_hunk(
    hunks: &mut Vec<WorkspaceDiffHunk>,
    current_hunk: &mut Option<WorkspaceDiffHunk>,
) {
    if let Some(hunk) = current_hunk.take() {
        hunks.push(hunk);
    }
}

fn parse_hunk_header(header: &str) -> (Option<u32>, Option<u32>, Option<u32>, Option<u32>) {
    let Some(range_end) = header[3..].find("@@").map(|index| index + 3) else {
        return (None, None, None, None);
    };
    let mut ranges = header[3..range_end].split_whitespace();
    let (old_start, old_lines) = ranges
        .next()
        .and_then(|range| parse_hunk_range(range, '-'))
        .unwrap_or((None, None));
    let (new_start, new_lines) = ranges
        .next()
        .and_then(|range| parse_hunk_range(range, '+'))
        .unwrap_or((None, None));
    (old_start, old_lines, new_start, new_lines)
}

fn parse_hunk_range(range: &str, prefix: char) -> Option<(Option<u32>, Option<u32>)> {
    let range = range.strip_prefix(prefix)?;
    let (start, lines) = range
        .split_once(',')
        .map(|(start, lines)| (start, lines))
        .unwrap_or((range, "1"));
    Some((start.parse().ok(), lines.parse().ok()))
}

fn parse_diff_git_path(line: &str) -> Option<PathBuf> {
    line.rsplit_once(" b/")
        .map(|(_, path)| PathBuf::from(unquote_git_path(path.trim())))
}

fn parse_diff_marker_path(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path == "/dev/null" {
        return None;
    }
    path.strip_prefix("b/")
        .or_else(|| path.strip_prefix("a/"))
        .map(|path| PathBuf::from(unquote_git_path(path.trim())))
}

fn unquote_git_path(path: &str) -> String {
    path.trim_matches('"').replace("\\\"", "\"")
}

fn normalized_path_string(path: &FsPath) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Deserialize)]
struct WorkspacePathQuery {
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceSearchRequest {
    query: String,
    path: Option<String>,
    #[serde(default)]
    fixed_strings: bool,
    #[serde(default)]
    word_match: bool,
    max_results: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiffRevertRequest {
    path: String,
    #[serde(default)]
    confirm: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceDiffHunkAction {
    Stage,
    Unstage,
    Discard,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDiffHunkActionRequest {
    path: String,
    scope: WorkspaceDiffScope,
    patch: String,
    action: WorkspaceDiffHunkAction,
    #[serde(default)]
    confirm: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct WorkspaceDiffActionResponse {
    path: PathBuf,
    diff: WorkspaceDiff,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn relative_git_paths_reject_traversal_and_renames() {
        assert_eq!(
            validate_relative_git_path(r"src\lib.rs").expect("valid relative path"),
            "src/lib.rs"
        );
        for invalid in ["", "../secret.txt", "/absolute.txt", "old.txt -> new.txt"] {
            assert!(validate_relative_git_path(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn workspace_paths_reject_parent_components_before_io() {
        let error = resolve_workspace_path(FsPath::new("."), Some("../outside"))
            .expect_err("parent traversal must be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("cannot contain .."));
    }

    #[test]
    fn git_status_preserves_staged_unstaged_and_untracked_state() {
        let files = parse_git_status(" M src/lib.rs\nM  Cargo.toml\n?? notes.txt\n");

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].unstaged_status, "modified");
        assert!(files[0].staged_status.is_empty());
        assert_eq!(files[1].staged_status, "modified");
        assert!(files[1].unstaged_status.is_empty());
        assert!(files[2].is_untracked);
    }

    #[test]
    fn diff_hunks_keep_file_headers_scope_and_ranges() {
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,2 +2,3 @@\n-old\n+new\n";
        let hunks = parse_workspace_diff_hunks(diff, WorkspaceDiffScope::Unstaged);

        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].path, PathBuf::from("src/lib.rs"));
        assert_eq!(hunks[0].scope, WorkspaceDiffScope::Unstaged);
        assert_eq!(hunks[0].old_start, Some(2));
        assert_eq!(hunks[0].old_lines, Some(2));
        assert_eq!(hunks[0].new_start, Some(2));
        assert_eq!(hunks[0].new_lines, Some(3));
        assert!(hunks[0].patch.starts_with("diff --git"));
    }

    #[test]
    fn workspace_search_request_keeps_camel_case_contract() {
        let request: WorkspaceSearchRequest = serde_json::from_value(serde_json::json!({
            "query": "needle",
            "fixedStrings": true,
            "wordMatch": true,
            "maxResults": 25
        }))
        .expect("deserialize workspace search request");

        assert!(request.fixed_strings);
        assert!(request.word_match);
        assert_eq!(request.max_results, Some(25));
    }
}
