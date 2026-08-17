use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use opentopia_core::{
    lock_mutation_paths, normalize_workspace_key, FileMutationObserver, FileMutationScope,
    FileMutationTarget, PreparedFileMutation, SessionStore, SqliteSessionStore, TurnChangeSet,
    TurnChangeSetStatus, TurnFileChange, TurnFileChangeKind, GIT_NONINTERACTIVE_ENVIRONMENT,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{
    Mutex as AsyncMutex, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock as AsyncRwLock,
};
use tracing::info;
use uuid::Uuid;

const MAX_MERGE_FILE_BYTES: usize = 16 * 1024 * 1024;
const TURN_FILE_DIFF_PAGE_BYTES: usize = 96 * 1024;
const GIT_FAILURE_DETAIL_CHARS: usize = 600;

#[derive(Clone)]
pub struct TurnChangeManager {
    store: Arc<SqliteSessionStore>,
    workspace_locks: Arc<Mutex<HashMap<String, Weak<AsyncRwLock<()>>>>>,
    journal_lock: Arc<AsyncMutex<()>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IgnoredPathMode {
    Skip,
    Include,
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
    external_observed: BTreeMap<PathBuf, Option<String>>,
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
            journal_lock: Arc::new(AsyncMutex::new(())),
        }
    }

    fn workspace_lock(&self, workspace_root: &Path) -> Arc<AsyncRwLock<()>> {
        let key = normalize_workspace_key(workspace_root);
        let mut locks = self
            .workspace_locks
            .lock()
            .expect("workspace change lock registry poisoned");
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(AsyncRwLock::new(()));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        }
    }

    /// Keeps destructive workspace operations out while allowing independent
    /// threads in the same workspace to execute concurrently.
    pub async fn lock_workspace_shared(&self, workspace_root: &Path) -> OwnedRwLockReadGuard<()> {
        self.workspace_lock(workspace_root).read_owned().await
    }

    /// Exclusively coordinates undo operations with all active turns in a
    /// workspace so an undo never races a model-owned file mutation.
    pub async fn lock_workspace(&self, workspace_root: &Path) -> OwnedRwLockWriteGuard<()> {
        self.workspace_lock(workspace_root).write_owned().await
    }

    pub async fn begin_capture(
        &self,
        turn_id: Uuid,
        thread_id: Uuid,
        workspace_root: &Path,
    ) -> anyhow::Result<TurnChangeSet> {
        let started = Instant::now();
        let mut change_set =
            TurnChangeSet::capturing(turn_id, thread_id, canonical_or_original(workspace_root));
        let capture = async {
            let repo = discover_repo(workspace_root).await?;
            anyhow::Ok(repo)
        }
        .await;

        match capture {
            Ok(repo) => {
                change_set.repo_root = Some(repo.repo_root);
                change_set.workspace_prefix = Some(repo.workspace_prefix);
            }
            Err(error) => {
                change_set.status = TurnChangeSetStatus::Failed;
                change_set.error = Some(error.to_string());
                change_set.finalized_at = Some(Utc::now());
            }
        }
        self.store.upsert_turn_change_set(&change_set)?;
        info!(
            %turn_id,
            elapsed_ms = elapsed_millis(started),
            status = ?change_set.status,
            "turn change journal started"
        );
        Ok(change_set)
    }

    #[cfg(test)]
    pub async fn finalize_capture(&self, turn_id: Uuid) -> anyhow::Result<TurnChangeSet> {
        self.finalize_capture_scoped(turn_id, None).await
    }

    /// Finalize a capture using only file paths attributed to workspace-write
    /// tools in this turn. An empty slice is a confident "no turn-owned writes"
    /// result; it must not absorb unrelated edits that happened elsewhere in
    /// the workspace while the turn ran.
    pub async fn finalize_capture_for_paths(
        &self,
        turn_id: Uuid,
        changed_paths: &[PathBuf],
    ) -> anyhow::Result<TurnChangeSet> {
        self.finalize_capture_scoped(turn_id, Some(changed_paths))
            .await
    }

    async fn finalize_capture_scoped(
        &self,
        turn_id: Uuid,
        changed_paths: Option<&[PathBuf]>,
    ) -> anyhow::Result<TurnChangeSet> {
        let started = Instant::now();
        let mut change_set = self
            .store
            .get_turn_change_set(turn_id)?
            .context("turn change set was not started")?;
        if change_set.status != TurnChangeSetStatus::Capturing {
            return Ok(change_set);
        }
        if change_set.files.is_empty() && changed_paths.is_some_and(|paths| paths.is_empty()) {
            change_set.status = TurnChangeSetStatus::Empty;
            change_set.files.clear();
            change_set.additions = 0;
            change_set.deletions = 0;
            change_set.error = None;
            change_set.finalized_at = Some(Utc::now());
            self.store.upsert_turn_change_set(&change_set)?;
            info!(
                %turn_id,
                elapsed_ms = elapsed_millis(started),
                "turn change capture finalized without reported writes"
            );
            return Ok(change_set);
        }
        if change_set.files.is_empty() && changed_paths.is_some_and(|paths| !paths.is_empty()) {
            let repo = repo_from_change_set(&change_set)?;
            let reported = normalized_changed_paths(
                &change_set,
                changed_paths.expect("checked non-empty changed paths"),
            );
            let mut has_unjournaled_change = false;
            for path in reported {
                if path.is_absolute()
                    || !git_path_is_ignored(&repo.repo_root, &repo_path(&repo, &path)).await?
                {
                    has_unjournaled_change = true;
                    break;
                }
            }
            if !has_unjournaled_change {
                change_set.status = TurnChangeSetStatus::Empty;
                change_set.error = None;
                change_set.finalized_at = Some(Utc::now());
                self.store.upsert_turn_change_set(&change_set)?;
                return Ok(change_set);
            }
            change_set.status = TurnChangeSetStatus::Failed;
            change_set.error = Some(
                "workspace-write tools reported changed paths, but no exact file mutations were journaled; shell or external writes are not safely undoable"
                    .to_string(),
            );
            change_set.finalized_at = Some(Utc::now());
            self.store.upsert_turn_change_set(&change_set)?;
            return Ok(change_set);
        }
        let result = async {
            let repo = repo_from_change_set(&change_set)?;
            let journaled_external = change_set
                .files
                .iter()
                .filter(|change| change_is_external(change))
                .cloned()
                .collect::<Vec<_>>();
            let journaled_workspace = change_set
                .files
                .iter()
                .filter(|change| !change_is_external(change))
                .cloned()
                .collect::<Vec<_>>();
            let before_reference = turn_snapshot_ref(turn_id, "before");
            let after_reference = turn_snapshot_ref(turn_id, "after");
            let before_tree = capture_journal_tree(
                &repo,
                &journaled_workspace,
                JournalTreeSide::Before,
                Some(&before_reference),
            )
            .await?;
            let after_tree = capture_journal_tree(
                &repo,
                &journaled_workspace,
                JournalTreeSide::After,
                Some(&after_reference),
            )
            .await?;
            protect_external_blobs(
                &repo.repo_root,
                &journaled_external,
                JournalTreeSide::Before,
                &turn_snapshot_ref(turn_id, "external-before"),
            )
            .await?;
            protect_external_blobs(
                &repo.repo_root,
                &journaled_external,
                JournalTreeSide::After,
                &turn_snapshot_ref(turn_id, "external-after"),
            )
            .await?;
            // The write-boundary journal is authoritative. Tool-result path
            // metadata is only a compatibility signal for unjournaled writers
            // and must not discard an exact mutation record.
            let mut files = diff_trees(&repo, &before_tree, &after_tree).await?;
            for mut change in journaled_external {
                let (additions, deletions, binary) =
                    external_file_stats(&repo.repo_root, &change).await?;
                change.additions = additions;
                change.deletions = deletions;
                change.binary = binary;
                files.push(change);
            }
            files.sort_by(|left, right| {
                left.display_path()
                    .map(|path| normalized_path_text(path))
                    .cmp(&right.display_path().map(|path| normalized_path_text(path)))
            });
            anyhow::Ok((before_tree, after_tree, files))
        }
        .await;

        change_set.finalized_at = Some(Utc::now());
        match result {
            Ok((before_tree, after_tree, files)) => {
                change_set.before_tree = Some(before_tree);
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
        info!(
            %turn_id,
            elapsed_ms = elapsed_millis(started),
            reported_paths = changed_paths.map_or(0, <[PathBuf]>::len),
            captured_files = change_set.files.len(),
            status = ?change_set.status,
            "turn change capture finalized"
        );
        Ok(change_set)
    }

    pub async fn preview_undo(&self, change_set: TurnChangeSet) -> anyhow::Result<TurnUndoPreview> {
        let started = Instant::now();
        let turn_id = change_set.turn_id;
        // Preview is read-only and undo rebuilds the plan under the exclusive
        // lock, so active turns do not need to block this dialog from opening.
        let _guard = self.lock_workspace_shared(&change_set.workspace_root).await;
        let preview = self.build_undo_plan(change_set).await?.preview;
        info!(
            %turn_id,
            elapsed_ms = elapsed_millis(started),
            files = preview.change_set.files.len(),
            conflicts = preview.conflicts.len(),
            "turn undo preview built"
        );
        Ok(preview)
    }

    pub async fn preview_file_diff(
        &self,
        change_set: &TurnChangeSet,
        requested_path: &Path,
        requested_offset: usize,
    ) -> anyhow::Result<TurnFileDiffPreview> {
        let repo = repo_from_change_set(change_set)?;
        validate_recorded_change_path(requested_path)?;
        let change = change_set
            .files
            .iter()
            .find(|change| {
                change
                    .old_path
                    .iter()
                    .chain(change.new_path.iter())
                    .any(|path| same_change_path(path, requested_path))
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

        if change_is_external(change) {
            let diff = external_blob_diff(&repo.repo_root, change, &path).await?;
            return Ok(paginate_file_diff(
                change_set.turn_id,
                change,
                path,
                diff,
                requested_offset,
            ));
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
        Ok(paginate_file_diff(
            change_set.turn_id,
            change,
            path,
            String::from_utf8_lossy(&output.stdout).into_owned(),
            requested_offset,
        ))
    }

    pub async fn undo(&self, change_set: TurnChangeSet) -> anyhow::Result<TurnUndoResult> {
        let started = Instant::now();
        let turn_id = change_set.turn_id;
        let _guard = self.lock_workspace(&change_set.workspace_root).await;
        let _path_guards = lock_mutation_paths(change_set_mutation_paths(&change_set)).await;
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
        info!(
            %turn_id,
            elapsed_ms = elapsed_millis(started),
            files = plan.actions.len(),
            "turn changes undone"
        );
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
        let mut actions = Vec::new();
        let mut observed = BTreeMap::new();
        let mut external_observed = BTreeMap::new();

        if conflicts.is_empty() {
            let after_tree = change_set
                .after_tree
                .as_deref()
                .context("after-turn tree is unavailable")?;
            let workspace_paths = change_set_workspace_paths(&change_set);
            let current_tree = capture_tree_for_paths(
                &repo,
                after_tree,
                &workspace_paths,
                IgnoredPathMode::Include,
                None,
            )
            .await?;
            let repo_paths = workspace_paths
                .iter()
                .map(|path| repo_path(&repo, path))
                .collect::<Vec<_>>();
            let current_entries = tree_entries(&repo.repo_root, &current_tree, &repo_paths).await?;
            for file in &change_set.files {
                if change_is_external(file) {
                    plan_external_file_undo(
                        &repo,
                        file,
                        &mut actions,
                        &mut external_observed,
                        &mut conflicts,
                    )
                    .await?;
                } else {
                    plan_file_undo(
                        &repo,
                        &current_entries,
                        file,
                        &mut actions,
                        &mut observed,
                        &mut conflicts,
                    )
                    .await?;
                }
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
            external_observed,
            repo,
        })
    }
}

#[async_trait]
impl FileMutationObserver for TurnChangeManager {
    async fn record_file_mutations(
        &self,
        scope: &FileMutationScope,
        mutations: &[PreparedFileMutation],
    ) -> anyhow::Result<()> {
        if mutations.is_empty() {
            return Ok(());
        }
        // Different files may be written concurrently, but their per-Turn net
        // journal is one record. Serialize only this small merge/persist step.
        let _journal_guard = self.journal_lock.lock().await;
        let mut change_set = self
            .store
            .get_turn_change_set(scope.turn_id)?
            .context("turn change set was not started before a file mutation")?;
        anyhow::ensure!(
            change_set.thread_id == scope.thread_id,
            "file mutation thread does not match the active turn"
        );
        anyhow::ensure!(
            normalize_workspace_key(&change_set.workspace_root)
                == normalize_workspace_key(&scope.workspace_root),
            "file mutation workspace does not match the active turn"
        );
        anyhow::ensure!(
            change_set.status == TurnChangeSetStatus::Capturing,
            "turn file mutation arrived after capture finalized"
        );
        let repo = repo_from_change_set(&change_set)?;

        for mutation in mutations {
            let recorded_path = normalize_recorded_change_path(&change_set, &mutation.path)?;
            let external = recorded_path.is_absolute();
            let before_contents = mutation.original.as_deref();
            let after_contents = match &mutation.target {
                FileMutationTarget::Write(contents) => Some(contents.as_slice()),
                FileMutationTarget::Delete => None,
            };
            let existing_index = change_set.files.iter().position(|change| {
                change
                    .old_path
                    .iter()
                    .chain(change.new_path.iter())
                    .any(|path| same_change_path(path, &recorded_path))
            });
            let existing = existing_index.map(|index| change_set.files[index].clone());
            if !external
                && existing.is_none()
                && git_path_is_ignored(&repo.repo_root, &repo_path(&repo, &recorded_path)).await?
            {
                continue;
            }

            let before_oid = match existing.as_ref() {
                Some(change) => change.before_oid.clone(),
                None => match before_contents {
                    Some(contents) => Some(write_git_blob(&repo.repo_root, contents).await?),
                    None => None,
                },
            };
            let before_mode = existing
                .as_ref()
                .and_then(|change| change.before_mode.clone())
                .or_else(|| before_oid.as_ref().map(|_| "100644".to_string()));
            let after_oid = match after_contents {
                Some(contents) => Some(write_git_blob(&repo.repo_root, contents).await?),
                None => None,
            };
            let after_mode = after_oid.as_ref().map(|_| "100644".to_string());

            if before_oid == after_oid && before_mode == after_mode {
                if let Some(index) = existing_index {
                    change_set.files.remove(index);
                }
                continue;
            }

            let kind = match (before_oid.is_some(), after_oid.is_some()) {
                (false, true) => TurnFileChangeKind::Added,
                (true, false) => TurnFileChangeKind::Deleted,
                (true, true) => TurnFileChangeKind::Modified,
                (false, false) => continue,
            };
            let binary = existing.as_ref().is_some_and(|change| change.binary)
                || before_contents
                    .into_iter()
                    .chain(after_contents)
                    .any(|contents| contents.contains(&0));
            let change = TurnFileChange {
                kind,
                old_path: before_oid.as_ref().map(|_| recorded_path.clone()),
                new_path: after_oid.as_ref().map(|_| recorded_path.clone()),
                before_oid,
                after_oid,
                before_mode,
                after_mode,
                additions: None,
                deletions: None,
                binary,
            };
            match existing_index {
                Some(index) => change_set.files[index] = change,
                None => change_set.files.push(change),
            }
        }

        change_set.files.sort_by(|left, right| {
            left.display_path()
                .map(|path| normalized_path_text(path))
                .cmp(&right.display_path().map(|path| normalized_path_text(path)))
        });
        self.store.upsert_turn_change_set(&change_set)?;
        Ok(())
    }
}

fn paginate_file_diff(
    turn_id: Uuid,
    change: &TurnFileChange,
    path: PathBuf,
    diff: String,
    requested_offset: usize,
) -> TurnFileDiffPreview {
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
    TurnFileDiffPreview {
        turn_id,
        path,
        old_path: change.old_path.clone(),
        new_path: change.new_path.clone(),
        binary: false,
        diff: diff[offset..end].to_string(),
        offset,
        next_offset,
        total_bytes,
    }
}

#[derive(Debug, Clone, Copy)]
enum JournalTreeSide {
    Before,
    After,
}

async fn capture_journal_tree(
    repo: &RepoContext,
    changes: &[TurnFileChange],
    side: JournalTreeSide,
    reference: Option<&str>,
) -> anyhow::Result<String> {
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
        let mut entries = BTreeMap::<String, Option<TreeEntry>>::new();
        for change in changes {
            let (keep_path, keep_oid, keep_mode, remove_path) = match side {
                JournalTreeSide::Before => (
                    change.old_path.as_ref(),
                    change.before_oid.as_ref(),
                    change.before_mode.as_ref(),
                    change
                        .new_path
                        .as_ref()
                        .filter(|path| Some(*path) != change.old_path.as_ref()),
                ),
                JournalTreeSide::After => (
                    change.new_path.as_ref(),
                    change.after_oid.as_ref(),
                    change.after_mode.as_ref(),
                    change
                        .old_path
                        .as_ref()
                        .filter(|path| Some(*path) != change.new_path.as_ref()),
                ),
            };
            if let Some(path) = remove_path {
                entries.insert(repo_path(repo, path), None);
            }
            if let Some(path) = keep_path {
                let entry = match (keep_oid, keep_mode) {
                    (Some(oid), Some(mode)) => Some(TreeEntry {
                        mode: mode.clone(),
                        oid: oid.clone(),
                    }),
                    (None, None) => None,
                    _ => anyhow::bail!(
                        "journal entry has incomplete object metadata: {}",
                        path.display()
                    ),
                };
                entries.insert(repo_path(repo, path), entry);
            }
        }

        for (path, entry) in entries {
            match entry {
                Some(entry) => {
                    run_git_strings(
                        &repo.repo_root,
                        &[
                            "update-index".to_string(),
                            "--add".to_string(),
                            "--cacheinfo".to_string(),
                            format!("{},{},{}", entry.mode, entry.oid, path),
                        ],
                        Some(&temp_index),
                    )
                    .await?;
                }
                None => {
                    run_git_strings(
                        &repo.repo_root,
                        &[
                            "update-index".to_string(),
                            "--force-remove".to_string(),
                            "--".to_string(),
                            path,
                        ],
                        Some(&temp_index),
                    )
                    .await?;
                }
            }
        }

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

async fn protect_external_blobs(
    repo_root: &Path,
    changes: &[TurnFileChange],
    side: JournalTreeSide,
    reference: &str,
) -> anyhow::Result<()> {
    let mut oids = BTreeMap::new();
    for oid in changes.iter().filter_map(|change| match side {
        JournalTreeSide::Before => change.before_oid.as_deref(),
        JournalTreeSide::After => change.after_oid.as_deref(),
    }) {
        oids.entry(oid.to_string()).or_insert(());
    }
    if oids.is_empty() {
        return Ok(());
    }

    let temp_index = std::env::temp_dir().join(format!("opentopia-index-{}", Uuid::new_v4()));
    let result = async {
        run_git_strings(
            repo_root,
            &["read-tree".to_string(), "--empty".to_string()],
            Some(&temp_index),
        )
        .await?;
        for (index, oid) in oids.keys().enumerate() {
            run_git_strings(
                repo_root,
                &[
                    "update-index".to_string(),
                    "--add".to_string(),
                    "--cacheinfo".to_string(),
                    format!("100644,{oid},blob/{index:08}"),
                ],
                Some(&temp_index),
            )
            .await?;
        }
        let output = git_output(repo_root, &["write-tree"], Some(&temp_index)).await?;
        ensure_git_success(&output, "git write-tree for external turn blobs")?;
        let tree = String::from_utf8(output.stdout)?.trim().to_string();
        run_git_strings(
            repo_root,
            &["update-ref".to_string(), reference.to_string(), tree],
            None,
        )
        .await?;
        anyhow::Ok(())
    }
    .await;
    let _ = tokio::fs::remove_file(&temp_index).await;
    let _ = tokio::fs::remove_file(temp_index.with_extension("lock")).await;
    result
}

async fn write_git_blob(repo_root: &Path, contents: &[u8]) -> anyhow::Result<String> {
    let mut command = Command::new("git");
    command
        .current_dir(repo_root)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.envs(GIT_NONINTERACTIVE_ENVIRONMENT.iter().copied());
    let mut child = command.spawn().context("failed to start git hash-object")?;
    let mut stdin = child
        .stdin
        .take()
        .context("git hash-object stdin unavailable")?;
    stdin.write_all(contents).await?;
    drop(stdin);
    let output = child.wait_with_output().await?;
    ensure_git_success(&output, "git hash-object")?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn external_file_stats(
    repo_root: &Path,
    change: &TurnFileChange,
) -> anyhow::Result<(Option<u64>, Option<u64>, bool)> {
    let before = read_optional_raw_blob(repo_root, change.before_oid.as_deref()).await?;
    let after = read_optional_raw_blob(repo_root, change.after_oid.as_deref()).await?;
    if [&before, &after]
        .into_iter()
        .any(|contents| contents.contains(&0))
    {
        return Ok((None, None, true));
    }
    let before_oid = blob_oid_or_empty(repo_root, change.before_oid.as_deref()).await?;
    let after_oid = blob_oid_or_empty(repo_root, change.after_oid.as_deref()).await?;
    file_stats(repo_root, &before_oid, &after_oid, None, None).await
}

async fn external_blob_diff(
    repo_root: &Path,
    change: &TurnFileChange,
    path: &Path,
) -> anyhow::Result<String> {
    let before_oid = blob_oid_or_empty(repo_root, change.before_oid.as_deref()).await?;
    let after_oid = blob_oid_or_empty(repo_root, change.after_oid.as_deref()).await?;
    let output = git_output_strings(
        repo_root,
        &[
            "diff".to_string(),
            "--no-ext-diff".to_string(),
            "--no-color".to_string(),
            "--unified=3".to_string(),
            before_oid.clone(),
            after_oid.clone(),
        ],
        None,
    )
    .await?;
    ensure_git_success(&output, "git diff for external turn file preview")?;
    let label = normalized_path_text(path);
    Ok(String::from_utf8_lossy(&output.stdout)
        .replace(&format!("a/{before_oid}"), &format!("a/{label}"))
        .replace(&format!("b/{after_oid}"), &format!("b/{label}")))
}

async fn blob_oid_or_empty(repo_root: &Path, oid: Option<&str>) -> anyhow::Result<String> {
    match oid {
        Some(oid) => Ok(oid.to_string()),
        None => write_git_blob(repo_root, &[]).await,
    }
}

async fn read_optional_raw_blob(repo_root: &Path, oid: Option<&str>) -> anyhow::Result<Vec<u8>> {
    match oid {
        Some(oid) => read_raw_blob(repo_root, oid).await,
        None => Ok(Vec::new()),
    }
}

async fn read_raw_blob(repo_root: &Path, oid: &str) -> anyhow::Result<Vec<u8>> {
    let output = git_output_strings(
        repo_root,
        &["cat-file".to_string(), "blob".to_string(), oid.to_string()],
        None,
    )
    .await?;
    ensure_git_success(&output, "git cat-file external blob")?;
    Ok(output.stdout)
}

async fn plan_external_file_undo(
    repo: &RepoContext,
    change: &TurnFileChange,
    actions: &mut Vec<UndoAction>,
    observed: &mut BTreeMap<PathBuf, Option<String>>,
    conflicts: &mut Vec<TurnUndoConflict>,
) -> anyhow::Result<()> {
    let path = change
        .display_path()
        .context("external file change has no path")?;
    validate_external_absolute_path(path)?;
    let current = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.file_type().is_file() => Some(tokio::fs::read(path).await?),
        Ok(_) => {
            conflicts.push(file_conflict(
                path,
                TurnUndoConflictKind::UnsupportedFileType,
                "the external path is no longer a regular file",
            ));
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let current_oid = match current.as_deref() {
        Some(contents) => Some(hash_git_blob(&repo.repo_root, contents).await?),
        None => None,
    };
    observed.insert(path.clone(), current_oid.clone());

    match change.kind {
        TurnFileChangeKind::Added => {
            let after = change
                .after_oid
                .as_deref()
                .context("external added file snapshot is missing")?;
            if current_oid.as_deref() == Some(after) {
                actions.push(UndoAction::Delete { path: path.clone() });
            } else {
                conflicts.push(file_conflict(
                    path,
                    TurnUndoConflictKind::WorkspaceChanged,
                    "the external file created by this turn was changed or replaced later",
                ));
            }
        }
        TurnFileChangeKind::Deleted => {
            let before = change
                .before_oid
                .as_deref()
                .context("external deleted file snapshot is missing")?;
            if current.is_none() {
                actions.push(UndoAction::Write {
                    path: path.clone(),
                    contents: read_raw_blob(&repo.repo_root, before).await?,
                    mode: change
                        .before_mode
                        .clone()
                        .unwrap_or_else(|| "100644".to_string()),
                });
            } else if current_oid.as_deref() != Some(before) {
                conflicts.push(file_conflict(
                    path,
                    TurnUndoConflictKind::PathConflict,
                    "the deleted external path is occupied by a different file",
                ));
            }
        }
        TurnFileChangeKind::Modified => {
            let before_oid = change
                .before_oid
                .as_deref()
                .context("external before snapshot is missing")?;
            let after_oid = change
                .after_oid
                .as_deref()
                .context("external after snapshot is missing")?;
            let Some(current_contents) = current else {
                conflicts.push(file_conflict(
                    path,
                    TurnUndoConflictKind::WorkspaceChanged,
                    "the external file no longer exists",
                ));
                return Ok(());
            };
            let before_contents = read_raw_blob(&repo.repo_root, before_oid).await?;
            let contents = if current_oid.as_deref() == Some(after_oid) {
                before_contents
            } else {
                if change.binary || current_contents.contains(&0) {
                    conflicts.push(file_conflict(
                        path,
                        TurnUndoConflictKind::BinaryChanged,
                        "a binary external file changed after this turn and cannot be merged",
                    ));
                    return Ok(());
                }
                let after_contents = read_raw_blob(&repo.repo_root, after_oid).await?;
                if [
                    current_contents.len(),
                    after_contents.len(),
                    before_contents.len(),
                ]
                .into_iter()
                .any(|size| size > MAX_MERGE_FILE_BYTES)
                {
                    conflicts.push(file_conflict(
                        path,
                        TurnUndoConflictKind::TooLarge,
                        "the external file is too large for a safe three-way merge",
                    ));
                    return Ok(());
                }
                match merge_contents(&current_contents, &after_contents, &before_contents).await? {
                    Ok(merged) => merged,
                    Err(()) => {
                        conflicts.push(file_conflict(
                            path,
                            TurnUndoConflictKind::MergeConflict,
                            "later edits overlap the external lines changed by this turn",
                        ));
                        return Ok(());
                    }
                }
            };
            actions.push(UndoAction::Write {
                path: path.clone(),
                contents,
                mode: change
                    .before_mode
                    .clone()
                    .unwrap_or_else(|| "100644".to_string()),
            });
        }
        TurnFileChangeKind::Renamed => {
            conflicts.push(file_conflict(
                path,
                TurnUndoConflictKind::UnsupportedFileType,
                "external rename records are not supported",
            ));
        }
    }
    Ok(())
}

async fn hash_git_blob(repo_root: &Path, contents: &[u8]) -> anyhow::Result<String> {
    let mut command = Command::new("git");
    command
        .current_dir(repo_root)
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(GIT_NONINTERACTIVE_ENVIRONMENT.iter().copied());
    let mut child = command.spawn().context("failed to start git hash-object")?;
    child
        .stdin
        .take()
        .context("git hash-object stdin unavailable")?
        .write_all(contents)
        .await?;
    let output = child.wait_with_output().await?;
    ensure_git_success(&output, "git hash-object")?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn plan_file_undo(
    repo: &RepoContext,
    current_entries: &BTreeMap<String, TreeEntry>,
    change: &TurnFileChange,
    actions: &mut Vec<UndoAction>,
    observed: &mut BTreeMap<String, Option<TreeEntry>>,
    conflicts: &mut Vec<TurnUndoConflict>,
) -> anyhow::Result<()> {
    let old_repo_path = change.old_path.as_ref().map(|path| repo_path(repo, path));
    let new_repo_path = change.new_path.as_ref().map(|path| repo_path(repo, path));
    let old_current = match old_repo_path.as_deref() {
        Some(path) => Some((path, current_entries.get(path).cloned())),
        None => None,
    };
    let new_current = match new_repo_path.as_deref() {
        Some(path) if Some(path) != old_repo_path.as_deref() => {
            Some((path, current_entries.get(path).cloned()))
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

fn normalize_reported_change_path(change_set: &TurnChangeSet, reported: &Path) -> Option<PathBuf> {
    if reported.as_os_str().is_empty() {
        return None;
    }
    if reported.is_relative() {
        return Some(reported.to_path_buf());
    }

    let reported = normalized_path_text(reported);
    let workspace = normalized_path_text(&change_set.workspace_root);
    let reported_cmp = comparable_path_text(&reported);
    let workspace_cmp = comparable_path_text(&workspace);
    if reported_cmp == workspace_cmp {
        return Some(PathBuf::from("."));
    }
    let prefix = format!("{workspace_cmp}/");
    reported_cmp
        .strip_prefix(&prefix)
        .map(|relative| PathBuf::from(&reported[reported.len() - relative.len()..]))
}

fn normalize_recorded_change_path(
    change_set: &TurnChangeSet,
    reported: &Path,
) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !reported.as_os_str().is_empty(),
        "file mutation path is empty"
    );
    if let Some(relative) = normalize_reported_change_path(change_set, reported) {
        validate_workspace_relative_path(&relative)?;
        return Ok(relative);
    }
    validate_external_absolute_path(reported)?;
    Ok(reported.to_path_buf())
}

fn normalized_changed_paths(change_set: &TurnChangeSet, reported: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = BTreeMap::new();
    for path in reported {
        let Ok(path) = normalize_recorded_change_path(change_set, path) else {
            continue;
        };
        let key = comparable_path_text(&normalized_path_text(&path));
        paths.entry(key).or_insert(path);
    }
    paths.into_values().collect()
}

fn change_set_workspace_paths(change_set: &TurnChangeSet) -> Vec<PathBuf> {
    let mut paths = BTreeMap::new();
    for path in change_set
        .files
        .iter()
        .flat_map(|file| file.old_path.iter().chain(file.new_path.iter()))
    {
        if path.is_absolute() {
            continue;
        }
        let key = comparable_path_text(&normalized_path_text(path));
        paths.entry(key).or_insert_with(|| path.clone());
    }
    paths.into_values().collect()
}

fn change_set_mutation_paths(change_set: &TurnChangeSet) -> Vec<PathBuf> {
    let mut paths = BTreeMap::new();
    for path in change_set
        .files
        .iter()
        .flat_map(|file| file.old_path.iter().chain(file.new_path.iter()))
    {
        let absolute = if path.is_absolute() {
            path.clone()
        } else {
            change_set.workspace_root.join(path)
        };
        let key = comparable_path_text(&normalized_path_text(&absolute));
        paths.entry(key).or_insert(absolute);
    }
    paths.into_values().collect()
}

fn change_is_external(change: &TurnFileChange) -> bool {
    change
        .old_path
        .iter()
        .chain(change.new_path.iter())
        .any(|path| path.is_absolute())
}

fn same_change_path(left: &Path, right: &Path) -> bool {
    comparable_path_text(&normalized_path_text(left))
        == comparable_path_text(&normalized_path_text(right))
}

fn normalized_path_text(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_string()
}

fn comparable_path_text(path: &str) -> String {
    if cfg!(windows) {
        path.to_ascii_lowercase()
    } else {
        path.to_string()
    }
}

async fn capture_tree_for_paths(
    repo: &RepoContext,
    base_tree: &str,
    workspace_paths: &[PathBuf],
    ignored_path_mode: IgnoredPathMode,
    reference: Option<&str>,
) -> anyhow::Result<String> {
    let temp_index = std::env::temp_dir().join(format!("opentopia-index-{}", Uuid::new_v4()));
    let result = async {
        run_git_strings(
            &repo.repo_root,
            &["read-tree".to_string(), base_tree.to_string()],
            Some(&temp_index),
        )
        .await?;

        let candidates = workspace_paths
            .iter()
            .map(|path| {
                validate_workspace_relative_path(path)?;
                anyhow::Ok((path, repo_path(repo, path)))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let candidate_repo_paths = candidates
            .iter()
            .map(|(_, path)| path.clone())
            .collect::<Vec<_>>();
        let base_entries = tree_entries(&repo.repo_root, base_tree, &candidate_repo_paths).await?;
        let mut repo_paths = Vec::new();
        for (workspace_path, repo_path) in candidates {
            let exists =
                match tokio::fs::symlink_metadata(repo.workspace_root.join(workspace_path)).await {
                    Ok(_) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => return Err(error.into()),
                };
            let existed_in_base = base_entries.contains_key(&repo_path);
            if exists
                && !existed_in_base
                && ignored_path_mode == IgnoredPathMode::Skip
                && git_path_is_ignored(&repo.repo_root, &repo_path).await?
            {
                continue;
            }
            if exists || existed_in_base {
                repo_paths.push(repo_path);
            }
        }
        for paths in repo_paths.chunks(64) {
            let mut args = vec![
                "--literal-pathspecs".to_string(),
                "add".to_string(),
                "-A".to_string(),
            ];
            if ignored_path_mode == IgnoredPathMode::Include {
                args.push("--force".to_string());
            }
            args.push("--".to_string());
            args.extend(paths.iter().cloned());
            run_git_strings(&repo.repo_root, &args, Some(&temp_index)).await?;
        }

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

async fn git_path_is_ignored(repo_root: &Path, path: &str) -> anyhow::Result<bool> {
    let args = vec![
        "check-ignore".to_string(),
        "--quiet".to_string(),
        "--".to_string(),
        path.to_string(),
    ];
    let output = git_output_strings(repo_root, &args, None).await?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            ensure_git_success(&output, "git check-ignore")?;
            unreachable!("a successful git check-ignore exits with status zero")
        }
    }
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

async fn tree_entries(
    repo_root: &Path,
    tree: &str,
    paths: &[String],
) -> anyhow::Result<BTreeMap<String, TreeEntry>> {
    let mut entries = BTreeMap::new();
    for paths in paths.chunks(64) {
        let mut args = vec![
            "--literal-pathspecs".to_string(),
            "ls-tree".to_string(),
            "-z".to_string(),
            tree.to_string(),
            "--".to_string(),
        ];
        args.extend(paths.iter().cloned());
        let output = git_output_strings(repo_root, &args, None).await?;
        ensure_git_success(&output, "git ls-tree")?;
        for record in output.stdout.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let tab = record
                .iter()
                .position(|byte| *byte == b'\t')
                .context("invalid git ls-tree output")?;
            let header = String::from_utf8(record[..tab].to_vec())?;
            let path = String::from_utf8(record[tab + 1..].to_vec())?;
            let mut fields = header.split_ascii_whitespace();
            let mode = fields.next().context("tree mode missing")?.to_string();
            let _kind = fields.next().context("tree object type missing")?;
            let oid = fields.next().context("tree object ID missing")?.to_string();
            entries.insert(path, TreeEntry { mode, oid });
        }
    }
    Ok(entries)
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
        .envs(GIT_NONINTERACTIVE_ENVIRONMENT)
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
    let after_tree = plan
        .preview
        .change_set
        .after_tree
        .as_deref()
        .context("after-turn tree is unavailable")?;
    let workspace_paths = change_set_workspace_paths(&plan.preview.change_set);
    let tree = capture_tree_for_paths(
        &plan.repo,
        after_tree,
        &workspace_paths,
        IgnoredPathMode::Include,
        None,
    )
    .await?;
    let repo_paths = plan.observed.keys().cloned().collect::<Vec<_>>();
    let entries = tree_entries(&plan.repo.repo_root, &tree, &repo_paths).await?;
    for (path, expected) in &plan.observed {
        let actual = entries.get(path);
        if actual != expected.as_ref() {
            return Ok(Some(TurnUndoConflict {
                path: workspace_relative_path(&plan.repo, path).ok(),
                kind: TurnUndoConflictKind::WorkspaceChanged,
                reason: "the workspace changed while the undo was being prepared; retry"
                    .to_string(),
            }));
        }
    }
    for (path, expected) in &plan.external_observed {
        let actual = match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                Some(hash_git_blob(&plan.repo.repo_root, &tokio::fs::read(path).await?).await?)
            }
            Ok(_) => {
                return Ok(Some(file_conflict(
                    path,
                    TurnUndoConflictKind::UnsupportedFileType,
                    "the external path changed type while undo was being prepared",
                )))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if &actual != expected {
            return Ok(Some(file_conflict(
                path,
                TurnUndoConflictKind::WorkspaceChanged,
                "the external file changed while undo was being prepared; retry",
            )));
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
    for recorded in paths.keys() {
        let path = safe_recorded_path(workspace_root, recorded)?;
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
                    let path = safe_recorded_path(workspace_root, path)?;
                    write_file_atomic(&path, contents).await?;
                    apply_git_mode(&path, mode).await?;
                }
                UndoAction::Delete { path } => {
                    let path = safe_recorded_path(workspace_root, path)?;
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

fn safe_recorded_path(workspace_root: &Path, recorded: &Path) -> anyhow::Result<PathBuf> {
    if recorded.is_relative() {
        return safe_workspace_path(workspace_root, recorded);
    }
    validate_external_absolute_path(recorded)?;
    let components = recorded.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component.as_os_str());
        if !matches!(component, Component::Normal(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!(
                    "external undo path traverses a symbolic link: {}",
                    current.display()
                )
            }
            Ok(metadata) if !metadata.is_dir() => {
                anyhow::bail!(
                    "external undo path parent is not a directory: {}",
                    current.display()
                )
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(recorded.to_path_buf())
}

fn validate_recorded_change_path(path: &Path) -> anyhow::Result<()> {
    if path.is_absolute() {
        validate_external_absolute_path(path)
    } else {
        validate_workspace_relative_path(path)
    }
}

fn validate_external_absolute_path(path: &Path) -> anyhow::Result<()> {
    let components = path.components().collect::<Vec<_>>();
    if !path.is_absolute()
        || components.is_empty()
        || !matches!(components.last(), Some(Component::Normal(_)))
        || components
            .iter()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        anyhow::bail!("invalid external absolute file path: {}", path.display());
    }
    Ok(())
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
    command
        .envs(GIT_NONINTERACTIVE_ENVIRONMENT)
        .current_dir(repo_root)
        .args(args)
        .kill_on_drop(true);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command.output().await.map_err(Into::into)
}

fn ensure_git_success(output: &Output, action: &str) -> anyhow::Result<()> {
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!("{action} failed: {}", git_failure_detail(&output.stderr))
}

fn git_failure_detail(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let lines = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let decisive = lines
        .iter()
        .copied()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            line.starts_with("error:") || line.starts_with("fatal:")
        })
        .collect::<Vec<_>>();
    let selected = if decisive.is_empty() {
        lines.iter().rev().take(4).copied().collect::<Vec<_>>()
    } else {
        decisive.into_iter().rev().take(4).collect::<Vec<_>>()
    };
    let detail = selected.into_iter().rev().collect::<Vec<_>>().join(" ");
    let detail = if detail.is_empty() {
        "unknown Git error".to_string()
    } else {
        detail
    };
    let mut chars = detail.chars();
    let truncated = chars
        .by_ref()
        .take(GIT_FAILURE_DETAIL_CHARS)
        .collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
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

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
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

    async fn journal_mutations(
        manager: &TurnChangeManager,
        repo: &TestRepo,
        thread_id: Uuid,
        turn_id: Uuid,
        mutations: Vec<PreparedFileMutation>,
    ) {
        manager
            .record_file_mutations(
                &FileMutationScope {
                    thread_id,
                    turn_id,
                    agent_path: "/root".to_string(),
                    workspace_root: repo.root.clone(),
                },
                &mutations,
            )
            .await
            .unwrap();
    }

    async fn journal_write(
        manager: &TurnChangeManager,
        repo: &TestRepo,
        thread_id: Uuid,
        turn_id: Uuid,
        path: &str,
        contents: &str,
    ) {
        let target = repo.root.join(path);
        let original = fs::read(&target).ok();
        repo.write(path, contents);
        journal_mutations(
            manager,
            repo,
            thread_id,
            turn_id,
            vec![PreparedFileMutation::write(
                target,
                original,
                contents.as_bytes().to_vec(),
            )],
        )
        .await;
    }

    async fn journal_delete(
        manager: &TurnChangeManager,
        repo: &TestRepo,
        thread_id: Uuid,
        turn_id: Uuid,
        path: &str,
    ) {
        let target = repo.root.join(path);
        let original = fs::read(&target).unwrap();
        fs::remove_file(&target).unwrap();
        journal_mutations(
            manager,
            repo,
            thread_id,
            turn_id,
            vec![PreparedFileMutation::delete(target, original)],
        )
        .await;
    }

    #[test]
    fn git_failure_detail_prefers_actionable_errors_over_noisy_warnings() {
        let stderr = format!(
            "{}error: open(\"app\"): Function not implemented\nerror: unable to index file 'app'\nfatal: adding files failed\n",
            "warning: LF will be replaced by CRLF\n".repeat(200)
        );
        let detail = git_failure_detail(stderr.as_bytes());
        assert!(detail.contains("unable to index file 'app'"));
        assert!(detail.contains("fatal: adding files failed"));
        assert!(!detail.contains("LF will be replaced"));
        assert!(detail.chars().count() <= GIT_FAILURE_DETAIL_CHARS + 1);
    }

    #[tokio::test]
    async fn active_turns_share_a_workspace_while_undo_remains_exclusive() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
        let manager = TurnChangeManager::new(store);
        let workspace = PathBuf::from("C:/workspace/shared-project");

        let first_turn = manager.lock_workspace_shared(&workspace).await;
        let second_turn = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            manager.lock_workspace_shared(&workspace),
        )
        .await
        .expect("a second thread in the same workspace must not be serialized");

        let mut undo = Box::pin(manager.lock_workspace(&workspace));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut undo)
                .await
                .is_err(),
            "undo must wait until every active workspace turn finishes"
        );

        drop(first_turn);
        drop(second_turn);
        tokio::time::timeout(std::time::Duration::from_secs(1), undo)
            .await
            .expect("undo should proceed after active turns release the workspace");
    }

    #[tokio::test]
    async fn undo_preview_does_not_wait_for_an_active_turn_in_the_same_workspace() {
        let repo = TestRepo::new();
        repo.write("sample.txt", "before\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);
        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        journal_write(&manager, &repo, thread_id, turn_id, "sample.txt", "after\n").await;
        let change_set = manager.finalize_capture(turn_id).await.unwrap();

        let _active_turn = manager.lock_workspace_shared(&repo.root).await;
        let preview = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            manager.preview_undo(change_set),
        )
        .await
        .expect("a read-only undo preview should not wait for active turns")
        .unwrap();
        assert!(preview.can_undo, "conflicts: {:?}", preview.conflicts);
    }

    #[tokio::test]
    async fn scoped_finalize_ignores_unreported_concurrent_workspace_edits() {
        let repo = TestRepo::new();
        repo.write("owned.txt", "before\n");
        repo.write("unrelated.txt", "before\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        journal_write(&manager, &repo, thread_id, turn_id, "owned.txt", "after\n").await;
        repo.write("unrelated.txt", "changed elsewhere\n");

        let changes = manager
            .finalize_capture_for_paths(turn_id, &[PathBuf::from("owned.txt")])
            .await
            .unwrap();
        assert_eq!(changes.status, TurnChangeSetStatus::Ready);
        assert_eq!(changes.files.len(), 1);
        assert_eq!(
            changes.files[0].new_path.as_deref(),
            Some(Path::new("owned.txt"))
        );
        let repo_context = repo_from_change_set(&changes).unwrap();
        let before_tree = changes.before_tree.as_deref().unwrap();
        let after_tree = changes.after_tree.as_deref().unwrap();
        assert_eq!(
            tree_entry(&repo_context.repo_root, before_tree, "unrelated.txt")
                .await
                .unwrap(),
            tree_entry(&repo_context.repo_root, after_tree, "unrelated.txt")
                .await
                .unwrap(),
            "the scoped after snapshot must not rescan unrelated workspace edits"
        );
    }

    #[tokio::test]
    async fn scoped_finalize_is_empty_without_successful_reported_writes() {
        let repo = TestRepo::new();
        repo.write("unrelated.txt", "before\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        repo.write("unrelated.txt", "changed elsewhere\n");

        let changes = manager
            .finalize_capture_for_paths(turn_id, &[])
            .await
            .unwrap();
        assert_eq!(changes.status, TurnChangeSetStatus::Empty);
        assert!(changes.files.is_empty());
        assert_eq!(changes.before_tree, changes.after_tree);
    }

    #[tokio::test]
    async fn scoped_finalize_skips_a_reported_ignored_file() {
        let repo = TestRepo::new();
        repo.write(".gitignore", "ignored.txt\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        journal_write(
            &manager,
            &repo,
            thread_id,
            turn_id,
            "ignored.txt",
            "generated output\n",
        )
        .await;

        let changes = manager
            .finalize_capture_for_paths(turn_id, &[PathBuf::from("ignored.txt")])
            .await
            .unwrap();
        assert_eq!(
            changes.status,
            TurnChangeSetStatus::Empty,
            "capture error: {:?}",
            changes.error
        );
        assert!(changes.files.is_empty());
        assert_eq!(changes.before_tree, changes.after_tree);
        assert_eq!(repo.read("ignored.txt"), "generated output\n");
    }

    #[tokio::test]
    async fn undo_preview_detects_an_ignored_file_occupying_a_deleted_path() {
        let repo = TestRepo::new();
        repo.write("sample.txt", "tracked before\n");
        repo.commit_all();
        repo.write(".gitignore", "sample.txt\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        journal_delete(&manager, &repo, thread_id, turn_id, "sample.txt").await;
        let change_set = manager.finalize_capture(turn_id).await.unwrap();
        repo.write("sample.txt", "later ignored contents\n");

        let preview = manager.preview_undo(change_set).await.unwrap();
        assert!(!preview.can_undo);
        assert_eq!(preview.conflicts.len(), 1);
        assert_eq!(
            preview.conflicts[0].kind,
            TurnUndoConflictKind::PathConflict
        );
        assert_eq!(repo.read("sample.txt"), "later ignored contents\n");
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
        journal_write(&manager, &repo, thread_id, turn_id, "sample.txt", &after).await;
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
        journal_write(
            &manager,
            &repo,
            thread_id,
            first,
            "sample.txt",
            "ONE\ntwo\nthree\n",
        )
        .await;
        manager.finalize_capture(first).await.unwrap();
        store
            .update_turn_status(first, TurnStatus::Succeeded, None)
            .unwrap();

        let second = insert_turn(&store, thread_id);
        manager
            .begin_capture(second, thread_id, &repo.root)
            .await
            .unwrap();
        journal_write(
            &manager,
            &repo,
            thread_id,
            second,
            "sample.txt",
            "ONE\ntwo\nTHREE\n",
        )
        .await;
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
        journal_write(
            &manager,
            &repo,
            thread_id,
            turn_id,
            "sample.txt",
            "one\nTWO\nthree\n",
        )
        .await;
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
        journal_write(
            &manager,
            &repo,
            thread_id,
            turn_id,
            "sample.txt",
            "agent result\n",
        )
        .await;
        journal_write(
            &manager,
            &repo,
            thread_id,
            turn_id,
            "draft.txt",
            "agent changed draft\n",
        )
        .await;
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
        journal_delete(&manager, &repo, thread_id, turn_id, "deleted.txt").await;
        let old_path = repo.root.join("old-name.txt");
        let new_path = repo.root.join("new-name.txt");
        let renamed_contents = fs::read(&old_path).unwrap();
        fs::rename(&old_path, &new_path).unwrap();
        journal_mutations(
            &manager,
            &repo,
            thread_id,
            turn_id,
            vec![
                PreparedFileMutation::delete(&old_path, renamed_contents.clone()),
                PreparedFileMutation::write(&new_path, None, renamed_contents),
            ],
        )
        .await;
        journal_write(
            &manager,
            &repo,
            thread_id,
            turn_id,
            "added.txt",
            "remove me\n",
        )
        .await;
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

    #[tokio::test]
    async fn external_absolute_file_is_journaled_previewed_and_undone() {
        let repo = TestRepo::new();
        repo.write("tracked.txt", "baseline\n");
        repo.commit_all();
        let (manager, store, thread_id) = manager(&repo);
        let external_root =
            std::env::temp_dir().join(format!("opentopia-external-turn-{}", Uuid::new_v4()));
        fs::create_dir_all(&external_root).unwrap();
        let external = external_root.join("settings.txt");
        fs::write(&external, "before\n").unwrap();
        let turn_id = insert_turn(&store, thread_id);

        manager
            .begin_capture(turn_id, thread_id, &repo.root)
            .await
            .unwrap();
        fs::write(&external, "after\n").unwrap();
        journal_mutations(
            &manager,
            &repo,
            thread_id,
            turn_id,
            vec![PreparedFileMutation::write(
                external.clone(),
                Some(b"before\n".to_vec()),
                b"after\n".to_vec(),
            )],
        )
        .await;
        let change_set = manager
            .finalize_capture_for_paths(turn_id, std::slice::from_ref(&external))
            .await
            .unwrap();

        assert_eq!(change_set.status, TurnChangeSetStatus::Ready);
        assert_eq!(change_set.files.len(), 1);
        assert_eq!(
            change_set.files[0].new_path.as_deref(),
            Some(external.as_path())
        );
        assert_eq!(change_set.files[0].additions, Some(1));
        assert_eq!(change_set.files[0].deletions, Some(1));
        let protected_ref = turn_snapshot_ref(turn_id, "external-after");
        let protected = StdCommand::new("git")
            .current_dir(&repo.root)
            .args(["show-ref", "--verify", "--quiet", &protected_ref])
            .status()
            .unwrap();
        assert!(protected.success(), "external blob snapshot ref is missing");
        let preview = manager
            .preview_file_diff(&change_set, &external, 0)
            .await
            .unwrap();
        assert!(preview.diff.contains("-before"));
        assert!(preview.diff.contains("+after"));

        let result = manager.undo(change_set).await.unwrap();
        assert!(result.applied, "conflicts: {:?}", result.preview.conflicts);
        assert_eq!(fs::read_to_string(&external).unwrap(), "before\n");
        fs::remove_dir_all(external_root).unwrap();
    }

    #[tokio::test]
    async fn external_file_undo_across_threads_preserves_later_non_overlapping_turn() {
        let repo = TestRepo::new();
        repo.write("tracked.txt", "baseline\n");
        repo.commit_all();
        let store = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
        let first_thread = store.create_thread(None, repo.root.clone()).unwrap();
        let second_thread = store.create_thread(None, repo.root.clone()).unwrap();
        let manager = TurnChangeManager::new(store.clone());
        let external_root =
            std::env::temp_dir().join(format!("opentopia-external-merge-{}", Uuid::new_v4()));
        fs::create_dir_all(&external_root).unwrap();
        let external = external_root.join("shared.txt");
        fs::write(&external, "one\ntwo\nthree\n").unwrap();

        let first_turn = insert_turn(&store, first_thread.id);
        manager
            .begin_capture(first_turn, first_thread.id, &repo.root)
            .await
            .unwrap();
        fs::write(&external, "ONE\ntwo\nthree\n").unwrap();
        journal_mutations(
            &manager,
            &repo,
            first_thread.id,
            first_turn,
            vec![PreparedFileMutation::write(
                external.clone(),
                Some(b"one\ntwo\nthree\n".to_vec()),
                b"ONE\ntwo\nthree\n".to_vec(),
            )],
        )
        .await;
        manager.finalize_capture(first_turn).await.unwrap();
        store
            .update_turn_status(first_turn, TurnStatus::Succeeded, None)
            .unwrap();

        let second_turn = insert_turn(&store, second_thread.id);
        manager
            .begin_capture(second_turn, second_thread.id, &repo.root)
            .await
            .unwrap();
        fs::write(&external, "ONE\ntwo\nTHREE\n").unwrap();
        journal_mutations(
            &manager,
            &repo,
            second_thread.id,
            second_turn,
            vec![PreparedFileMutation::write(
                external.clone(),
                Some(b"ONE\ntwo\nthree\n".to_vec()),
                b"ONE\ntwo\nTHREE\n".to_vec(),
            )],
        )
        .await;
        manager.finalize_capture(second_turn).await.unwrap();

        let first_changes = store.get_turn_change_set(first_turn).unwrap().unwrap();
        let result = manager.undo(first_changes).await.unwrap();
        assert!(result.applied, "conflicts: {:?}", result.preview.conflicts);
        assert_eq!(
            fs::read_to_string(&external).unwrap().replace("\r\n", "\n"),
            "one\ntwo\nTHREE\n"
        );
        fs::remove_dir_all(external_root).unwrap();
    }
}
