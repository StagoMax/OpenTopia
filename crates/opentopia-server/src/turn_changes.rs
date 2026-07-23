use anyhow::Context;
use chrono::Utc;
use opentopia_core::{
    normalize_workspace_key, SessionStore, SqliteSessionStore, TurnChangeSet, TurnChangeSetStatus,
    TurnFileChange, TurnFileChangeKind,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::process::Output;
use std::sync::{Arc, Mutex, Weak};
use tokio::process::Command;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

const MAX_MERGE_FILE_BYTES: usize = 16 * 1024 * 1024;
const TURN_FILE_DIFF_PAGE_BYTES: usize = 96 * 1024;

#[derive(Clone)]
pub struct TurnChangeManager {
    store: Arc<SqliteSessionStore>,
    workspace_locks: Arc<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnUndoConflictKind {
    Unavailable,
    AlreadyReverted,
    WorkspaceChanged,
    MergeConflict,
    BinaryChanged,
    PathConflict,
    UnsupportedFileType,
    TooLarge,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnUndoConflict {
    pub path: Option<PathBuf>,
    pub kind: TurnUndoConflictKind,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnUndoPreview {
    pub turn_id: Uuid,
    pub can_undo: bool,
    pub files_to_change: usize,
    pub additions: u64,
    pub deletions: u64,
    pub conflicts: Vec<TurnUndoConflict>,
    pub change_set: TurnChangeSet,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnUndoResult {
    pub applied: bool,
    pub files_changed: usize,
    pub preview: TurnUndoPreview,
    pub change_set: TurnChangeSet,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFileDiffPreview {
    pub turn_id: Uuid,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub binary: bool,
    pub diff: String,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub total_bytes: usize,
}

#[derive(Debug, Clone)]
struct RepoContext {
    workspace_root: PathBuf,
    repo_root: PathBuf,
    workspace_prefix: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeEntry {
    mode: String,
    oid: String,
}

#[derive(Debug)]
enum UndoAction {
    Write {
        path: PathBuf,
        contents: Vec<u8>,
        mode: String,
    },
    Delete {
        path: PathBuf,
    },
}

#[derive(Debug)]
struct UndoPlan {
    preview: TurnUndoPreview,
    actions: Vec<UndoAction>,
    observed: BTreeMap<String, Option<TreeEntry>>,
    repo: RepoContext,
}

#[derive(Debug)]
enum BackupState {
    Missing,
    File {
        contents: Vec<u8>,
        permissions: std::fs::Permissions,
    },
}

#[derive(Debug)]
struct FileBackup {
    path: PathBuf,
    state: BackupState,
}

impl TurnChangeManager {
    pub fn new(store: Arc<SqliteSessionStore>) -> Self {
        Self {
            store,
            workspace_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn lock_workspace(&self, workspace_root: &Path) -> OwnedMutexGuard<()> {
        let key = normalize_workspace_key(workspace_root);
        let lock = {
            let mut locks = self
                .workspace_locks
                .lock()
                .expect("workspace change lock registry poisoned");
            if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    pub async fn begin_capture(
        &self,
        turn_id: Uuid,
        thread_id: Uuid,
        workspace_root: &Path,
    ) -> anyhow::Result<TurnChangeSet> {
        let mut change_set =
            TurnChangeSet::capturing(turn_id, thread_id, canonical_or_original(workspace_root));
        let capture = async {
            let repo = discover_repo(workspace_root).await?;
            let reference = turn_snapshot_ref(turn_id, "before");
            let tree = capture_tree(&repo, Some(&reference)).await?;
            anyhow::Ok((repo, tree))
        }
        .await;

        match capture {
            Ok((repo, tree)) => {
                change_set.repo_root = Some(repo.repo_root);
                change_set.workspace_prefix = Some(repo.workspace_prefix);
                change_set.before_tree = Some(tree);
            }
            Err(error) => {
                change_set.status = TurnChangeSetStatus::Failed;
                change_set.error = Some(error.to_string());
                change_set.finalized_at = Some(Utc::now());
            }
        }
        self.store.upsert_turn_change_set(&change_set)?;
        Ok(change_set)
    }

    pub async fn finalize_capture(&self, turn_id: Uuid) -> anyhow::Result<TurnChangeSet> {
        let mut change_set = self
            .store
            .get_turn_change_set(turn_id)?
            .context("turn change set was not started")?;
        if change_set.status != TurnChangeSetStatus::Capturing {
            return Ok(change_set);
        }
        let result = async {
            let repo = repo_from_change_set(&change_set)?;
            let before_tree = change_set
                .before_tree
                .as_deref()
                .context("before tree is unavailable")?;
            let reference = turn_snapshot_ref(turn_id, "after");
            let after_tree = capture_tree(&repo, Some(&reference)).await?;
            let files = diff_trees(&repo, before_tree, &after_tree).await?;
            anyhow::Ok((after_tree, files))
        }
        .await;

        change_set.finalized_at = Some(Utc::now());
        match result {
            Ok((after_tree, files)) => {
                change_set.after_tree = Some(after_tree);
                change_set.additions = files.iter().filter_map(|file| file.additions).sum();
                change_set.deletions = files.iter().filter_map(|file| file.deletions).sum();
                change_set.status = if files.is_empty() {
                    TurnChangeSetStatus::Empty
                } else {
                    TurnChangeSetStatus::Ready
                };
                change_set.files = files;
                change_set.error = None;
            }
            Err(error) => {
                change_set.status = TurnChangeSetStatus::Failed;
                change_set.error = Some(error.to_string());
            }
        }
        self.store.upsert_turn_change_set(&change_set)?;
        Ok(change_set)
    }

    pub async fn preview_undo(&self, change_set: TurnChangeSet) -> anyhow::Result<TurnUndoPreview> {
        let _guard = self.lock_workspace(&change_set.workspace_root).await;
        Ok(self.build_undo_plan(change_set).await?.preview)
    }

    pub async fn preview_file_diff(
        &self,
        change_set: &TurnChangeSet,
        requested_path: &Path,
        requested_offset: usize,
    ) -> anyhow::Result<TurnFileDiffPreview> {
        let repo = repo_from_change_set(change_set)?;
        validate_workspace_relative_path(requested_path)?;
        let requested = git_path(requested_path);
        let change = change_set
            .files
            .iter()
            .find(|change| {
                change
                    .old_path
                    .iter()
                    .chain(change.new_path.iter())
                    .any(|path| git_path(path) == requested)
            })
            .with_context(|| {
                format!(
                    "file is not part of this turn change set: {}",
                    requested_path.display()
                )
            })?;
        let path = change
            .display_path()
            .cloned()
            .context("turn file change has no path")?;

        if change.binary {
            return Ok(TurnFileDiffPreview {
                turn_id: change_set.turn_id,
                path,
                old_path: change.old_path.clone(),
                new_path: change.new_path.clone(),
                binary: true,
                diff: String::new(),
                offset: 0,
                next_offset: None,
                total_bytes: 0,
            });
        }

        let before_tree = change_set
            .before_tree
            .as_deref()
            .context("before-turn tree is unavailable")?;
        let after_tree = change_set
            .after_tree
            .as_deref()
            .context("after-turn tree is unavailable")?;
        let mut args = vec![
            "--literal-pathspecs".to_string(),
            "diff".to_string(),
            "--no-ext-diff".to_string(),
            "--no-color".to_string(),
            "--find-renames".to_string(),
            "--unified=3".to_string(),
            before_tree.to_string(),
            after_tree.to_string(),
            "--".to_string(),
        ];
        if let Some(old_path) = &change.old_path {
            validate_workspace_relative_path(old_path)?;
            args.push(repo_path(&repo, old_path));
        }
        if let Some(new_path) = change
            .new_path
            .as_ref()
            .filter(|path| Some(*path) != change.old_path.as_ref())
        {
            validate_workspace_relative_path(new_path)?;
            args.push(repo_path(&repo, new_path));
        }
        let output = git_output_strings(&repo.repo_root, &args, None).await?;
        ensure_git_success(&output, "git diff for turn file preview")?;
        let diff = String::from_utf8_lossy(&output.stdout);
        let total_bytes = diff.len();
        let mut offset = requested_offset.min(total_bytes);
        while offset > 0 && !diff.is_char_boundary(offset) {
            offset -= 1;
        }
        let mut end = offset
            .saturating_add(TURN_FILE_DIFF_PAGE_BYTES)
            .min(total_bytes);
        while end > offset && !diff.is_char_boundary(end) {
            end -= 1;
        }
        let next_offset = (end < total_bytes).then_some(end);

        Ok(TurnFileDiffPreview {
            turn_id: change_set.turn_id,
            path,
            old_path: change.old_path.clone(),
            new_path: change.new_path.clone(),
            binary: false,
            diff: diff[offset..end].to_string(),
            offset,
            next_offset,
            total_bytes,
        })
    }

    pub async fn undo(&self, change_set: TurnChangeSet) -> anyhow::Result<TurnUndoResult> {
        let _guard = self.lock_workspace(&change_set.workspace_root).await;
        let plan = self.build_undo_plan(change_set).await?;
        if !plan.preview.can_undo {
            return Ok(TurnUndoResult {
                applied: false,
                files_changed: 0,
                change_set: plan.preview.change_set.clone(),
                preview: plan.preview,
            });
        }

        if let Some(conflict) = verify_observed_entries(&plan).await? {
            let mut preview = plan.preview;
            preview.can_undo = false;
            preview.conflicts.push(conflict);
            return Ok(TurnUndoResult {
                applied: false,
                files_changed: 0,
                change_set: preview.change_set.clone(),
                preview,
            });
        }

        apply_actions(&plan.repo.workspace_root, &plan.actions).await?;
        let reverted = self
            .store
            .mark_turn_change_set_reverted(plan.preview.turn_id, Utc::now())?
            .context("turn change set disappeared after undo")?;
        Ok(TurnUndoResult {
            applied: true,
            files_changed: plan.actions.len(),
            preview: plan.preview,
            change_set: reverted,
        })
    }

    async fn build_undo_plan(&self, change_set: TurnChangeSet) -> anyhow::Result<UndoPlan> {
        let mut conflicts = Vec::new();
        if change_set.reverted_at.is_some() {
            conflicts.push(TurnUndoConflict {
                path: None,
                kind: TurnUndoConflictKind::AlreadyReverted,
                reason: "this turn has already been undone".to_string(),
            });
        } else if change_set.status != TurnChangeSetStatus::Ready || change_set.files.is_empty() {
            conflicts.push(TurnUndoConflict {
                path: None,
                kind: TurnUndoConflictKind::Unavailable,
                reason: change_set
                    .error
                    .clone()
                    .unwrap_or_else(|| "this turn has no undoable file changes".to_string()),
            });
        }

        let repo = repo_from_change_set(&change_set)?;
        let current_tree = capture_tree(&repo, None).await?;
        let mut actions = Vec::new();
        let mut observed = BTreeMap::new();

        if conflicts.is_empty() {
            for file in &change_set.files {
                plan_file_undo(
                    &repo,
                    &current_tree,
                    file,
                    &mut actions,
                    &mut observed,
                    &mut conflicts,
                )
                .await?;
            }
        }

        let preview = TurnUndoPreview {
            turn_id: change_set.turn_id,
            can_undo: conflicts.is_empty(),
            files_to_change: actions.len(),
            additions: change_set.additions,
            deletions: change_set.deletions,
            conflicts,
            change_set,
        };
        Ok(UndoPlan {
            preview,
            actions,
            observed,
            repo,
        })
    }
}

async fn plan_file_undo(
    repo: &RepoContext,
    current_tree: &str,
    change: &TurnFileChange,
    actions: &mut Vec<UndoAction>,
    observed: &mut BTreeMap<String, Option<TreeEntry>>,
    conflicts: &mut Vec<TurnUndoConflict>,
) -> anyhow::Result<()> {
    let old_repo_path = change.old_path.as_ref().map(|path| repo_path(repo, path));
    let new_repo_path = change.new_path.as_ref().map(|path| repo_path(repo, path));
    let old_current = match old_repo_path.as_deref() {
        Some(path) => Some((path, tree_entry(&repo.repo_root, current_tree, path).await?)),
        None => None,
    };
    let new_current = match new_repo_path.as_deref() {
        Some(path) if Some(path) != old_repo_path.as_deref() => {
            Some((path, tree_entry(&repo.repo_root, current_tree, path).await?))
        }
        Some(path) => old_current.as_ref().map(|(_, entry)| (path, entry.clone())),
        None => None,
    };
    if let Some((path, entry)) = &old_current {
        observed.insert((*path).to_string(), entry.clone());
    }
    if let Some((path, entry)) = &new_current {
        observed.insert((*path).to_string(), entry.clone());
    }

    match change.kind {
        TurnFileChangeKind::Added => {
            let path = change.new_path.as_ref().context("added file has no path")?;
            let current = new_current.as_ref().and_then(|(_, entry)| entry.as_ref());
            let after = expected_entry(change.after_oid.as_deref(), change.after_mode.as_deref())?;
            if current == Some(&after) && is_regular_mode(&after.mode) {
                actions.push(UndoAction::Delete { path: path.clone() });
            } else {
                conflicts.push(file_conflict(
                    path,
                    if current.is_some() && !is_regular_mode(&after.mode) {
                        TurnUndoConflictKind::UnsupportedFileType
                    } else {
                        TurnUndoConflictKind::WorkspaceChanged
                    },
                    "the file created by this turn was changed or replaced later",
                ));
            }
        }
        TurnFileChangeKind::Deleted => {
            let path = change
                .old_path
                .as_ref()
                .context("deleted file has no path")?;
            let current = old_current.as_ref().and_then(|(_, entry)| entry.as_ref());
            let before =
                expected_entry(change.before_oid.as_deref(), change.before_mode.as_deref())?;
            if !is_regular_mode(&before.mode) {
                conflicts.push(file_conflict(
                    path,
                    TurnUndoConflictKind::UnsupportedFileType,
                    "restoring this file type is not supported",
                ));
            } else if current.is_none() {
                let repo_path = old_repo_path
                    .as_deref()
                    .context("deleted repo path missing")?;
                let contents = read_blob(&repo.repo_root, repo_path, &before.oid).await?;
                actions.push(UndoAction::Write {
                    path: path.clone(),
                    contents,
                    mode: before.mode,
                });
            } else if current == Some(&before) {
                // The file was already restored outside OpenTopia.
            } else {
                conflicts.push(file_conflict(
                    path,
                    TurnUndoConflictKind::PathConflict,
                    "the deleted path is occupied by a different file",
                ));
            }
        }
        TurnFileChangeKind::Modified => {
            let path = change
                .new_path
                .as_ref()
                .or(change.old_path.as_ref())
                .context("modified file has no path")?;
            let repo_path = new_repo_path
                .as_deref()
                .or(old_repo_path.as_deref())
                .context("modified repo path missing")?;
            let current = new_current
                .as_ref()
                .or(old_current.as_ref())
                .and_then(|(_, entry)| entry.as_ref());
            plan_modified_file(
                repo, repo_path, path, path, current, change, actions, conflicts,
            )
            .await?;
        }
        TurnFileChangeKind::Renamed => {
            let old_path = change.old_path.as_ref().context("rename has no old path")?;
            let new_path = change.new_path.as_ref().context("rename has no new path")?;
            let before =
                expected_entry(change.before_oid.as_deref(), change.before_mode.as_deref())?;
            let old_entry = old_current.as_ref().and_then(|(_, entry)| entry.as_ref());
            let new_entry = new_current.as_ref().and_then(|(_, entry)| entry.as_ref());
            if old_entry == Some(&before) && new_entry.is_none() {
                return Ok(());
            }
            if old_entry.is_some() {
                conflicts.push(file_conflict(
                    old_path,
                    TurnUndoConflictKind::PathConflict,
                    "the original rename path is occupied",
                ));
                return Ok(());
            }
            let new_repo_path = new_repo_path
                .as_deref()
                .context("rename target path missing")?;
            let action_count = actions.len();
            plan_modified_file(
                repo,
                new_repo_path,
                new_path,
                old_path,
                new_entry,
                change,
                actions,
                conflicts,
            )
            .await?;
            if actions.len() > action_count {
                actions.push(UndoAction::Delete {
                    path: new_path.clone(),
                });
            }
        }
    }
    Ok(())
}

async fn plan_modified_file(
    repo: &RepoContext,
    current_repo_path: &str,
    current_workspace_path: &Path,
    output_path: &Path,
    current: Option<&TreeEntry>,
    change: &TurnFileChange,
    actions: &mut Vec<UndoAction>,
    conflicts: &mut Vec<TurnUndoConflict>,
) -> anyhow::Result<()> {
    let before = expected_entry(change.before_oid.as_deref(), change.before_mode.as_deref())?;
    let after = expected_entry(change.after_oid.as_deref(), change.after_mode.as_deref())?;
    let Some(current) = current else {
        conflicts.push(file_conflict(
            output_path,
            TurnUndoConflictKind::WorkspaceChanged,
            "the file no longer exists",
        ));
        return Ok(());
    };
    if !is_regular_mode(&before.mode)
        || !is_regular_mode(&after.mode)
        || !is_regular_mode(&current.mode)
    {
        conflicts.push(file_conflict(
            output_path,
            TurnUndoConflictKind::UnsupportedFileType,
            "three-way undo only supports regular files",
        ));
        return Ok(());
    }

    let target_mode = if current.mode == after.mode {
        before.mode.clone()
    } else if before.mode == after.mode {
        current.mode.clone()
    } else {
        conflicts.push(file_conflict(
            output_path,
            TurnUndoConflictKind::WorkspaceChanged,
            "both this turn and a later edit changed the file mode",
        ));
        return Ok(());
    };

    let old_repo_path = change
        .old_path
        .as_ref()
        .map(|path| repo_path(repo, path))
        .unwrap_or_else(|| current_repo_path.to_string());
    let new_repo_path = change
        .new_path
        .as_ref()
        .map(|path| repo_path(repo, path))
        .unwrap_or_else(|| current_repo_path.to_string());
    let before_contents = read_blob(&repo.repo_root, &old_repo_path, &before.oid).await?;

    let contents = if current.oid == after.oid {
        before_contents
    } else {
        if change.binary {
            conflicts.push(file_conflict(
                output_path,
                TurnUndoConflictKind::BinaryChanged,
                "a binary file changed after this turn and cannot be merged",
            ));
            return Ok(());
        }
        let current_repo_path = repo_path(repo, current_workspace_path);
        let current_contents = read_blob(&repo.repo_root, &current_repo_path, &current.oid).await?;
        let after_contents = read_blob(&repo.repo_root, &new_repo_path, &after.oid).await?;
        if [
            current_contents.len(),
            after_contents.len(),
            before_contents.len(),
        ]
        .into_iter()
        .any(|size| size > MAX_MERGE_FILE_BYTES)
        {
            conflicts.push(file_conflict(
                output_path,
                TurnUndoConflictKind::TooLarge,
                "the file is too large for a safe three-way merge",
            ));
            return Ok(());
        }
        if [&current_contents, &after_contents, &before_contents]
            .into_iter()
            .any(|contents| contents.contains(&0))
        {
            conflicts.push(file_conflict(
                output_path,
                TurnUndoConflictKind::BinaryChanged,
                "the file contains binary data and changed after this turn",
            ));
            return Ok(());
        }
        match merge_contents(&current_contents, &after_contents, &before_contents).await? {
            Ok(merged) => merged,
            Err(()) => {
                conflicts.push(file_conflict(
                    output_path,
                    TurnUndoConflictKind::MergeConflict,
                    "later edits overlap the lines changed by this turn",
                ));
                return Ok(());
            }
        }
    };

    actions.push(UndoAction::Write {
        path: output_path.to_path_buf(),
        contents,
        mode: target_mode,
    });
    Ok(())
}

async fn discover_repo(workspace_root: &Path) -> anyhow::Result<RepoContext> {
    let workspace_root = tokio::fs::canonicalize(workspace_root)
        .await
        .with_context(|| format!("workspace does not exist: {}", workspace_root.display()))?;
    let output = git_output(&workspace_root, &["rev-parse", "--show-toplevel"], None).await?;
    ensure_git_success(&output, "git rev-parse --show-toplevel")?;
    let repo_root = PathBuf::from(String::from_utf8(output.stdout)?.trim());
    let repo_root = tokio::fs::canonicalize(&repo_root).await?;
    let prefix = workspace_root.strip_prefix(&repo_root).with_context(|| {
        format!(
            "workspace {} is outside repository {}",
            workspace_root.display(),
            repo_root.display()
        )
    })?;
    let workspace_prefix = if prefix.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        prefix.to_path_buf()
    };
    Ok(RepoContext {
        workspace_root,
        repo_root,
        workspace_prefix,
    })
}

fn repo_from_change_set(change_set: &TurnChangeSet) -> anyhow::Result<RepoContext> {
    Ok(RepoContext {
        workspace_root: change_set.workspace_root.clone(),
        repo_root: change_set
            .repo_root
            .clone()
            .context("Git repository is unavailable for this turn")?,
        workspace_prefix: change_set
            .workspace_prefix
            .clone()
            .unwrap_or_else(|| PathBuf::from(".")),
    })
}

async fn capture_tree(repo: &RepoContext, reference: Option<&str>) -> anyhow::Result<String> {
    let temp_index = std::env::temp_dir().join(format!("opentopia-index-{}", Uuid::new_v4()));
    let verify = git_output(
        &repo.repo_root,
        &["rev-parse", "--verify", "HEAD"],
        Some(&temp_index),
    )
    .await?;
    let read_args = if verify.status.success() {
        vec!["read-tree".to_string(), "HEAD".to_string()]
    } else {
        vec!["read-tree".to_string(), "--empty".to_string()]
    };

    let result = async {
        run_git_strings(&repo.repo_root, &read_args, Some(&temp_index)).await?;
        run_git_strings(
            &repo.repo_root,
            &[
                "add".to_string(),
                "-A".to_string(),
                "--".to_string(),
                git_path(&repo.workspace_prefix),
            ],
            Some(&temp_index),
        )
        .await?;
        let output = git_output(&repo.repo_root, &["write-tree"], Some(&temp_index)).await?;
        ensure_git_success(&output, "git write-tree")?;
        let tree = String::from_utf8(output.stdout)?.trim().to_string();
        if let Some(reference) = reference {
            run_git_strings(
                &repo.repo_root,
                &[
                    "update-ref".to_string(),
                    reference.to_string(),
                    tree.clone(),
                ],
                None,
            )
            .await?;
        }
        anyhow::Ok(tree)
    }
    .await;

    let _ = tokio::fs::remove_file(&temp_index).await;
    let _ = tokio::fs::remove_file(temp_index.with_extension("lock")).await;
    result
}

async fn diff_trees(
    repo: &RepoContext,
    before_tree: &str,
    after_tree: &str,
) -> anyhow::Result<Vec<TurnFileChange>> {
    let args = vec![
        "diff".to_string(),
        "--name-status".to_string(),
        "-z".to_string(),
        "--find-renames".to_string(),
        before_tree.to_string(),
        after_tree.to_string(),
        "--".to_string(),
        git_path(&repo.workspace_prefix),
    ];
    let output = git_output_strings(&repo.repo_root, &args, None).await?;
    ensure_git_success(&output, "git diff --name-status")?;
    let fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8(field.to_vec()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut changes = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = fields[index].as_str();
        index += 1;
        let (kind, old_repo_path, new_repo_path) = if status.starts_with('R') {
            let old = fields
                .get(index)
                .context("rename source is missing")?
                .clone();
            let new = fields
                .get(index + 1)
                .context("rename destination is missing")?
                .clone();
            index += 2;
            (TurnFileChangeKind::Renamed, Some(old), Some(new))
        } else {
            let path = fields
                .get(index)
                .context("changed path is missing")?
                .clone();
            index += 1;
            match status.chars().next() {
                Some('A') => (TurnFileChangeKind::Added, None, Some(path)),
                Some('D') => (TurnFileChangeKind::Deleted, Some(path), None),
                Some('M' | 'T') => (TurnFileChangeKind::Modified, Some(path.clone()), Some(path)),
                other => anyhow::bail!("unsupported Git diff status: {other:?}"),
            }
        };
        let before = match old_repo_path.as_deref() {
            Some(path) => tree_entry(&repo.repo_root, before_tree, path).await?,
            None => None,
        };
        let after = match new_repo_path.as_deref() {
            Some(path) => tree_entry(&repo.repo_root, after_tree, path).await?,
            None => None,
        };
        let (additions, deletions, binary) = file_stats(
            &repo.repo_root,
            before_tree,
            after_tree,
            old_repo_path.as_deref(),
            new_repo_path.as_deref(),
        )
        .await?;
        changes.push(TurnFileChange {
            kind,
            old_path: old_repo_path
                .as_deref()
                .map(|path| workspace_relative_path(repo, path))
                .transpose()?,
            new_path: new_repo_path
                .as_deref()
                .map(|path| workspace_relative_path(repo, path))
                .transpose()?,
            before_oid: before.as_ref().map(|entry| entry.oid.clone()),
            after_oid: after.as_ref().map(|entry| entry.oid.clone()),
            before_mode: before.as_ref().map(|entry| entry.mode.clone()),
            after_mode: after.as_ref().map(|entry| entry.mode.clone()),
            additions,
            deletions,
            binary,
        });
    }
    Ok(changes)
}

async fn file_stats(
    repo_root: &Path,
    before_tree: &str,
    after_tree: &str,
    old_path: Option<&str>,
    new_path: Option<&str>,
) -> anyhow::Result<(Option<u64>, Option<u64>, bool)> {
    let mut args = vec![
        "diff".to_string(),
        "--numstat".to_string(),
        "--find-renames".to_string(),
        before_tree.to_string(),
        after_tree.to_string(),
        "--".to_string(),
    ];
    if let Some(path) = old_path {
        args.push(path.to_string());
    }
    if let Some(path) = new_path.filter(|path| Some(*path) != old_path) {
        args.push(path.to_string());
    }
    let output = git_output_strings(repo_root, &args, None).await?;
    ensure_git_success(&output, "git diff --numstat")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut additions = 0u64;
    let mut deletions = 0u64;
    let mut binary = false;
    let mut found = false;
    for line in text.lines() {
        let mut fields = line.splitn(3, '\t');
        let added = fields.next().unwrap_or_default();
        let deleted = fields.next().unwrap_or_default();
        if added == "-" || deleted == "-" {
            binary = true;
            found = true;
        } else if let (Ok(added), Ok(deleted)) = (added.parse::<u64>(), deleted.parse::<u64>()) {
            additions = additions.saturating_add(added);
            deletions = deletions.saturating_add(deleted);
            found = true;
        }
    }
    Ok(if binary {
        (None, None, true)
    } else if found {
        (Some(additions), Some(deletions), false)
    } else {
        (Some(0), Some(0), false)
    })
}

async fn tree_entry(repo_root: &Path, tree: &str, path: &str) -> anyhow::Result<Option<TreeEntry>> {
    let output = git_output_strings(
        repo_root,
        &[
            "ls-tree".to_string(),
            "-z".to_string(),
            tree.to_string(),
            "--".to_string(),
            path.to_string(),
        ],
        None,
    )
    .await?;
    ensure_git_success(&output, "git ls-tree")?;
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let header = output
        .stdout
        .split(|byte| *byte == b'\t')
        .next()
        .context("invalid git ls-tree output")?;
    let header = String::from_utf8(header.to_vec())?;
    let mut fields = header.split_ascii_whitespace();
    let mode = fields.next().context("tree mode missing")?.to_string();
    let _kind = fields.next().context("tree object type missing")?;
    let oid = fields.next().context("tree object ID missing")?.to_string();
    Ok(Some(TreeEntry { mode, oid }))
}

async fn read_blob(repo_root: &Path, path: &str, oid: &str) -> anyhow::Result<Vec<u8>> {
    let filtered = git_output_strings(
        repo_root,
        &[
            "cat-file".to_string(),
            "--filters".to_string(),
            format!("--path={path}"),
            oid.to_string(),
        ],
        None,
    )
    .await?;
    if filtered.status.success() {
        return Ok(filtered.stdout);
    }
    let raw = git_output_strings(
        repo_root,
        &["cat-file".to_string(), "blob".to_string(), oid.to_string()],
        None,
    )
    .await?;
    ensure_git_success(&raw, "git cat-file blob")?;
    Ok(raw.stdout)
}

async fn merge_contents(
    current: &[u8],
    after: &[u8],
    before: &[u8],
) -> anyhow::Result<Result<Vec<u8>, ()>> {
    let root = std::env::temp_dir().join(format!("opentopia-merge-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&root).await?;
    let current_path = root.join("current");
    let after_path = root.join("after");
    let before_path = root.join("before");
    tokio::fs::write(&current_path, current).await?;
    tokio::fs::write(&after_path, after).await?;
    tokio::fs::write(&before_path, before).await?;
    let output = Command::new("git")
        .arg("merge-file")
        .arg("-p")
        .arg("--diff3")
        .arg("-L")
        .arg("current workspace")
        .arg("-L")
        .arg("turn result")
        .arg("-L")
        .arg("before turn")
        .arg(&current_path)
        .arg(&after_path)
        .arg(&before_path)
        .kill_on_drop(true)
        .output()
        .await?;
    let _ = tokio::fs::remove_dir_all(&root).await;
    match output.status.code() {
        Some(0) => Ok(Ok(output.stdout)),
        Some(1) => Ok(Err(())),
        _ => anyhow::bail!(
            "git merge-file failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

async fn verify_observed_entries(plan: &UndoPlan) -> anyhow::Result<Option<TurnUndoConflict>> {
    let tree = capture_tree(&plan.repo, None).await?;
    for (path, expected) in &plan.observed {
        let actual = tree_entry(&plan.repo.repo_root, &tree, path).await?;
        if &actual != expected {
            return Ok(Some(TurnUndoConflict {
                path: workspace_relative_path(&plan.repo, path).ok(),
                kind: TurnUndoConflictKind::WorkspaceChanged,
                reason: "the workspace changed while the undo was being prepared; retry"
                    .to_string(),
            }));
        }
    }
    Ok(None)
}

async fn apply_actions(workspace_root: &Path, actions: &[UndoAction]) -> anyhow::Result<()> {
    let mut backups = Vec::new();
    let mut paths = BTreeMap::<PathBuf, ()>::new();
    for action in actions {
        let relative = match action {
            UndoAction::Write { path, .. } | UndoAction::Delete { path } => path,
        };
        paths.insert(relative.clone(), ());
    }
    for relative in paths.keys() {
        let path = safe_workspace_path(workspace_root, relative)?;
        let state = match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.file_type().is_file() => BackupState::File {
                contents: tokio::fs::read(&path).await?,
                permissions: metadata.permissions(),
            },
            Ok(_) => anyhow::bail!("undo target is not a regular file: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BackupState::Missing,
            Err(error) => return Err(error.into()),
        };
        backups.push(FileBackup { path, state });
    }

    let result = async {
        for action in actions {
            match action {
                UndoAction::Write {
                    path,
                    contents,
                    mode,
                } => {
                    let path = safe_workspace_path(workspace_root, path)?;
                    write_file_atomic(&path, contents).await?;
                    apply_git_mode(&path, mode).await?;
                }
                UndoAction::Delete { path } => {
                    let path = safe_workspace_path(workspace_root, path)?;
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        anyhow::Ok(())
    }
    .await;

    if let Err(error) = result {
        for backup in backups.into_iter().rev() {
            match backup.state {
                BackupState::Missing => {
                    let _ = tokio::fs::remove_file(&backup.path).await;
                }
                BackupState::File {
                    contents,
                    permissions,
                } => {
                    let _ = write_file_atomic(&backup.path, &contents).await;
                    let _ = tokio::fs::set_permissions(&backup.path, permissions).await;
                }
            }
        }
        return Err(error);
    }
    Ok(())
}

async fn write_file_atomic(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temp = path.with_file_name(format!(".{file_name}.opentopia-{}.tmp", Uuid::new_v4()));
    tokio::fs::write(&temp, contents).await?;
    if tokio::fs::symlink_metadata(path).await.is_ok() {
        tokio::fs::remove_file(path).await?;
    }
    if let Err(error) = tokio::fs::rename(&temp, path).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error.into());
    }
    Ok(())
}

#[cfg(unix)]
async fn apply_git_mode(path: &Path, mode: &str) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = if mode == "100755" { 0o755 } else { 0o644 };
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(permissions)).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn apply_git_mode(_path: &Path, _mode: &str) -> anyhow::Result<()> {
    Ok(())
}

fn safe_workspace_path(workspace_root: &Path, relative: &Path) -> anyhow::Result<PathBuf> {
    if validate_workspace_relative_path(relative).is_err() {
        anyhow::bail!(
            "invalid workspace-relative undo path: {}",
            relative.display()
        );
    }
    let mut current = workspace_root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        let Component::Normal(component) = component else {
            unreachable!()
        };
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("undo path traverses a symbolic link: {}", current.display())
            }
            Ok(metadata) if !metadata.is_dir() => {
                anyhow::bail!("undo path parent is not a directory: {}", current.display())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(workspace_root.join(relative))
}

fn validate_workspace_relative_path(relative: &Path) -> anyhow::Result<()> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        anyhow::bail!("invalid workspace-relative path: {}", relative.display());
    }
    Ok(())
}

async fn run_git_strings(
    repo_root: &Path,
    args: &[String],
    index: Option<&Path>,
) -> anyhow::Result<Output> {
    let output = git_output_strings(repo_root, args, index).await?;
    ensure_git_success(&output, &format!("git {}", args.join(" ")))?;
    Ok(output)
}

async fn git_output(
    repo_root: &Path,
    args: &[&str],
    index: Option<&Path>,
) -> anyhow::Result<Output> {
    let args = args
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    git_output_strings(repo_root, &args, index).await
}

async fn git_output_strings(
    repo_root: &Path,
    args: &[String],
    index: Option<&Path>,
) -> anyhow::Result<Output> {
    let mut command = Command::new("git");
    command.current_dir(repo_root).args(args).kill_on_drop(true);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command.output().await.map_err(Into::into)
}

fn ensure_git_success(output: &Output, action: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "{action} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn expected_entry(oid: Option<&str>, mode: Option<&str>) -> anyhow::Result<TreeEntry> {
    Ok(TreeEntry {
        oid: oid.context("snapshot blob is missing")?.to_string(),
        mode: mode.context("snapshot mode is missing")?.to_string(),
    })
}

fn is_regular_mode(mode: &str) -> bool {
    mode == "100644" || mode == "100755"
}

fn repo_path(repo: &RepoContext, workspace_relative: &Path) -> String {
    let prefix = git_path(&repo.workspace_prefix);
    let relative = git_path(workspace_relative);
    if prefix == "." || prefix.is_empty() {
        relative
    } else {
        format!("{prefix}/{relative}")
    }
}

fn workspace_relative_path(repo: &RepoContext, repo_path: &str) -> anyhow::Result<PathBuf> {
    let prefix = git_path(&repo.workspace_prefix);
    let relative = if prefix == "." || prefix.is_empty() {
        repo_path
    } else {
        repo_path
            .strip_prefix(&format!("{prefix}/"))
            .with_context(|| format!("Git path is outside workspace: {repo_path}"))?
    };
    let path = PathBuf::from(relative);
    safe_workspace_path(&repo.workspace_root, &path)?;
    Ok(path)
}

fn git_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn turn_snapshot_ref(turn_id: Uuid, phase: &str) -> String {
    format!("refs/opentopia/turns/{turn_id}/{phase}")
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn file_conflict(
    path: &Path,
    kind: TurnUndoConflictKind,
    reason: impl Into<String>,
) -> TurnUndoConflict {
    TurnUndoConflict {
        path: Some(path.to_path_buf()),
        kind,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentopia_core::{Message, MessageRole, TurnRecord, TurnStatus};
    use std::fs;
    use std::process::Command as StdCommand;

    struct TestRepo {
        root: PathBuf,
    }

    impl TestRepo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("opentopia-turn-undo-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q"]);
            Self { root }
        }

        fn write(&self, path: &str, contents: &str) {
            let path = self.root.join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }

        fn read(&self, path: &str) -> String {
            fs::read_to_string(self.root.join(path)).unwrap()
        }

        fn commit_all(&self) {
            git(&self.root, &["add", "-A"]);
            git(
                &self.root,
                &[
                    "-c",
                    "user.name=OpenTopia Test",
                    "-c",
                    "user.email=test@opentopia.local",
                    "commit",
                    "-q",
                    "-m",
                    "baseline",
                ],
            );
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn git(root: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn manager(repo: &TestRepo) -> (TurnChangeManager, Arc<SqliteSessionStore>, Uuid) {
        let store = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
        let thread = store.create_thread(None, repo.root.clone()).unwrap();
        (TurnChangeManager::new(store.clone()), store, thread.id)
    }

    fn insert_turn(store: &SqliteSessionStore, thread_id: Uuid) -> Uuid {
        let message = store
            .append_message(Message::text(thread_id, MessageRole::User, "change it"))
            .unwrap();
        store
            .insert_turn(TurnRecord::running(thread_id, message.id))
            .unwrap()
            .turn_id
    }

    #[tokio::test]
    async fn file_diff_preview_pages_large_historical_diff() {
        let repo = TestRepo::new();
        repo.write("sample.txt", "before\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);
        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        let after = format!(
            "start\n{}\nend\n",
            "x".repeat(TURN_FILE_DIFF_PAGE_BYTES + 16_000)
        );
        repo.write("sample.txt", &after);
        let change_set = manager.finalize_capture(turn_id).await.unwrap();

        let mut offset = 0;
        let mut combined = String::new();
        let mut page_count = 0;
        loop {
            let page = manager
                .preview_file_diff(&change_set, Path::new("sample.txt"), offset)
                .await
                .unwrap();
            assert_eq!(page.offset, combined.len());
            assert!(!page.binary);
            combined.push_str(&page.diff);
            page_count += 1;
            match page.next_offset {
                Some(next_offset) => offset = next_offset,
                None => {
                    assert_eq!(page.total_bytes, combined.len());
                    break;
                }
            }
            assert!(page_count < 10, "preview pagination did not terminate");
        }

        assert!(page_count > 1);
        assert!(combined.contains("-before"));
        assert!(combined.contains("+start"));
        assert!(combined.contains("+end"));
        assert!(manager
            .preview_file_diff(&change_set, Path::new("not-changed.txt"), 0)
            .await
            .is_err());
        assert!(manager
            .preview_file_diff(&change_set, Path::new("../outside.txt"), 0)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn undo_historical_turn_preserves_later_non_overlapping_edit() {
        let repo = TestRepo::new();
        repo.write("sample.txt", "one\ntwo\nthree\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);

        let first = insert_turn(&store, thread_id);
        manager
            .begin_capture(first, thread_id, &repo.root)
            .await
            .unwrap();
        repo.write("sample.txt", "ONE\ntwo\nthree\n");
        manager.finalize_capture(first).await.unwrap();
        store
            .update_turn_status(first, TurnStatus::Succeeded, None)
            .unwrap();

        let second = insert_turn(&store, thread_id);
        manager
            .begin_capture(second, thread_id, &repo.root)
            .await
            .unwrap();
        repo.write("sample.txt", "ONE\ntwo\nTHREE\n");
        manager.finalize_capture(second).await.unwrap();

        let first_changes = store.get_turn_change_set(first).unwrap().unwrap();
        let result = manager.undo(first_changes).await.unwrap();
        assert!(result.applied, "conflicts: {:?}", result.preview.conflicts);
        assert_eq!(
            repo.read("sample.txt").replace("\r\n", "\n"),
            "one\ntwo\nTHREE\n"
        );
    }

    #[tokio::test]
    async fn overlapping_later_edit_reports_conflict_without_writing() {
        let repo = TestRepo::new();
        repo.write("sample.txt", "one\ntwo\nthree\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);
        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        repo.write("sample.txt", "one\nTWO\nthree\n");
        manager.finalize_capture(turn_id).await.unwrap();
        repo.write("sample.txt", "one\nTWO LATER\nthree\n");

        let changes = store.get_turn_change_set(turn_id).unwrap().unwrap();
        let preview = manager.preview_undo(changes).await.unwrap();
        assert!(!preview.can_undo);
        assert_eq!(
            preview.conflicts[0].kind,
            TurnUndoConflictKind::MergeConflict
        );
        assert_eq!(repo.read("sample.txt"), "one\nTWO LATER\nthree\n");
    }

    #[tokio::test]
    async fn undo_restores_dirty_workspace_baseline_instead_of_head() {
        let repo = TestRepo::new();
        repo.write("sample.txt", "committed\n");
        repo.commit_all();
        repo.write("sample.txt", "user work in progress\n");
        repo.write("draft.txt", "untracked user draft\n");
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        repo.write("sample.txt", "agent result\n");
        repo.write("draft.txt", "agent changed draft\n");
        manager.finalize_capture(turn_id).await.unwrap();

        let changes = store.get_turn_change_set(turn_id).unwrap().unwrap();
        let result = manager.undo(changes).await.unwrap();
        assert!(result.applied, "conflicts: {:?}", result.preview.conflicts);
        assert_eq!(
            repo.read("sample.txt").replace("\r\n", "\n"),
            "user work in progress\n"
        );
        assert_eq!(
            repo.read("draft.txt").replace("\r\n", "\n"),
            "untracked user draft\n"
        );
    }

    #[tokio::test]
    async fn undo_reverses_added_deleted_and_renamed_files_together() {
        let repo = TestRepo::new();
        repo.write("deleted.txt", "restore me\n");
        repo.write("old-name.txt", "renamed contents\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        fs::remove_file(repo.root.join("deleted.txt")).unwrap();
        fs::rename(
            repo.root.join("old-name.txt"),
            repo.root.join("new-name.txt"),
        )
        .unwrap();
        repo.write("added.txt", "remove me\n");
        let change_set = manager.finalize_capture(turn_id).await.unwrap();

        assert_eq!(change_set.status, TurnChangeSetStatus::Ready);
        assert_eq!(change_set.files.len(), 3);
        assert!(change_set
            .files
            .iter()
            .any(|change| change.kind == TurnFileChangeKind::Added));
        assert!(change_set
            .files
            .iter()
            .any(|change| change.kind == TurnFileChangeKind::Deleted));
        assert!(change_set
            .files
            .iter()
            .any(|change| change.kind == TurnFileChangeKind::Renamed));

        let result = manager.undo(change_set).await.unwrap();
        assert!(result.applied, "conflicts: {:?}", result.preview.conflicts);
        assert!(!repo.root.join("added.txt").exists());
        assert!(!repo.root.join("new-name.txt").exists());
        assert_eq!(
            repo.read("deleted.txt").replace("\r\n", "\n"),
            "restore me\n"
        );
        assert_eq!(
            repo.read("old-name.txt").replace("\r\n", "\n"),
            "renamed contents\n"
        );
    }
}
