use crate::execution::{
    ExecutionEnvironment, FileDeleteRequest, FileReadRequest, FileWriteRequest,
};
use anyhow::Context;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

type MutationLockRegistry = StdMutex<HashMap<String, Weak<AsyncMutex<()>>>>;

static MUTATION_LOCKS: OnceLock<MutationLockRegistry> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMutationScope {
    pub thread_id: Uuid,
    pub turn_id: Uuid,
    pub agent_path: String,
    pub workspace_root: PathBuf,
}

/// Receives only mutations that have successfully reached the filesystem.
/// Implementations persist the exact before/after versions while the writer
/// still owns every affected file lock.
#[async_trait]
pub trait FileMutationObserver: Send + Sync {
    async fn record_file_mutations(
        &self,
        scope: &FileMutationScope,
        mutations: &[PreparedFileMutation],
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileMutationTarget {
    Write(Vec<u8>),
    Delete,
}

/// One fully validated mutation. `original` is both the optimistic concurrency
/// precondition and the rollback value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFileMutation {
    pub path: PathBuf,
    pub original: Option<Vec<u8>>,
    pub target: FileMutationTarget,
}

impl PreparedFileMutation {
    pub fn write(
        path: impl Into<PathBuf>,
        original: Option<Vec<u8>>,
        contents: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            path: path.into(),
            original,
            target: FileMutationTarget::Write(contents.into()),
        }
    }

    pub fn delete(path: impl Into<PathBuf>, original: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            original: Some(original),
            target: FileMutationTarget::Delete,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileMutationBatch {
    mutations: Vec<PreparedFileMutation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMutationBatchResult {
    pub changed_paths: Vec<PathBuf>,
}

impl FileMutationBatch {
    pub fn new(mutations: Vec<PreparedFileMutation>) -> anyhow::Result<Self> {
        anyhow::ensure!(!mutations.is_empty(), "file mutation batch is empty");
        let mut paths = HashSet::new();
        for mutation in &mutations {
            let key = mutation_path_key(&mutation.path);
            anyhow::ensure!(
                paths.insert(key),
                "file mutation batch contains duplicate path: {}",
                mutation.path.display()
            );
        }
        Ok(Self { mutations })
    }

    pub fn mutations(&self) -> &[PreparedFileMutation] {
        &self.mutations
    }

    /// Commit all prepared mutations. Every target is re-read before mutation
    /// to reject stale patches. If any write/delete fails, completed mutations
    /// are restored in reverse order before the error is returned.
    pub async fn commit(
        &self,
        environment: &dyn ExecutionEnvironment,
    ) -> anyhow::Result<FileMutationBatchResult> {
        self.commit_observed(environment, None, None).await
    }

    /// Commit an already prepared batch under deterministic, process-wide
    /// per-file locks. Validation happens again after all locks are held, so a
    /// second OpenTopia conversation cannot slip a write between validation and
    /// persistence. Locks cover only the filesystem commit and journal append,
    /// never model reading or reasoning time.
    pub async fn commit_observed(
        &self,
        environment: &dyn ExecutionEnvironment,
        observer: Option<&dyn FileMutationObserver>,
        scope: Option<&FileMutationScope>,
    ) -> anyhow::Result<FileMutationBatchResult> {
        anyhow::ensure!(
            observer.is_some() == scope.is_some(),
            "file mutation observer and scope must be provided together"
        );
        let _locks =
            lock_mutation_paths(self.mutations.iter().map(|mutation| mutation.path.clone())).await;

        // Validate the complete batch before its first write. The previous
        // sequential validation could discover a stale later path only after an
        // earlier path had already been changed and then require rollback.
        for mutation in &self.mutations {
            let current = read_optional(environment, &mutation.path).await?;
            if current != mutation.original {
                anyhow::bail!(
                    "file changed after patch validation: {}; reread the latest file and retry",
                    mutation.path.display()
                );
            }
        }

        let mut applied = Vec::<&PreparedFileMutation>::new();

        for mutation in &self.mutations {
            let result = match &mutation.target {
                FileMutationTarget::Write(contents) => environment
                    .write_file(FileWriteRequest::new(&mutation.path, contents.clone()))
                    .await
                    .map(|_| ()),
                FileMutationTarget::Delete => environment
                    .delete_file(FileDeleteRequest::new(&mutation.path))
                    .await
                    .map(|_| ()),
            };

            if let Err(error) = result {
                return rollback_after_error(
                    environment,
                    &applied,
                    error.context(format!(
                        "failed to commit file mutation {}",
                        mutation.path.display()
                    )),
                )
                .await;
            }
            applied.push(mutation);
        }

        match (observer, scope) {
            (Some(observer), Some(scope)) => {
                if let Err(error) = observer.record_file_mutations(scope, &self.mutations).await {
                    return rollback_after_error(
                        environment,
                        &applied,
                        error.context("failed to persist the turn file-mutation journal"),
                    )
                    .await;
                }
            }
            (None, None) => {}
            _ => unreachable!("observer/scope pairing was validated before writing"),
        }

        Ok(FileMutationBatchResult {
            changed_paths: self
                .mutations
                .iter()
                .map(|mutation| mutation.path.clone())
                .collect(),
        })
    }
}

/// Acquire all requested locks in a stable order. Batches that touch the same
/// paths in opposite orders therefore cannot deadlock.
pub async fn lock_mutation_paths(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<OwnedMutexGuard<()>> {
    let mut keyed = paths
        .into_iter()
        .map(|path| (mutation_path_key(&path), path))
        .collect::<Vec<_>>();
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    keyed.dedup_by(|left, right| left.0 == right.0);

    let locks = {
        let registry = MUTATION_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
        let mut registry = registry
            .lock()
            .expect("file mutation lock registry poisoned");
        keyed
            .into_iter()
            .map(|(key, _path)| {
                if let Some(lock) = registry.get(&key).and_then(Weak::upgrade) {
                    lock
                } else {
                    let lock = Arc::new(AsyncMutex::new(()));
                    registry.insert(key, Arc::downgrade(&lock));
                    lock
                }
            })
            .collect::<Vec<_>>()
    };

    let mut guards = Vec::with_capacity(locks.len());
    for lock in locks {
        guards.push(lock.lock_owned().await);
    }
    guards
}

async fn rollback_after_error(
    environment: &dyn ExecutionEnvironment,
    applied: &[&PreparedFileMutation],
    original_error: anyhow::Error,
) -> anyhow::Result<FileMutationBatchResult> {
    let mut rollback_errors = Vec::new();
    for mutation in applied.iter().rev() {
        let expected_current = match &mutation.target {
            FileMutationTarget::Write(contents) => Some(contents.as_slice()),
            FileMutationTarget::Delete => None,
        };
        match read_optional(environment, &mutation.path).await {
            Ok(current) if current.as_deref() != expected_current => {
                rollback_errors.push(format!(
                    "rollback conflict at {}: file changed after batch write",
                    mutation.path.display()
                ));
                continue;
            }
            Err(error) => {
                rollback_errors.push(format!(
                    "failed to inspect {} during rollback: {error:#}",
                    mutation.path.display()
                ));
                continue;
            }
            Ok(_) => {}
        }

        let result = match &mutation.original {
            Some(contents) => environment
                .write_file(FileWriteRequest::new(&mutation.path, contents.clone()))
                .await
                .map(|_| ()),
            None => environment
                .delete_file(FileDeleteRequest::new(&mutation.path))
                .await
                .map(|_| ()),
        };
        if let Err(error) = result {
            rollback_errors.push(format!(
                "failed to restore {}: {error:#}",
                mutation.path.display()
            ));
        }
    }

    if rollback_errors.is_empty() {
        Err(original_error.context("file mutation batch was rolled back"))
    } else {
        Err(original_error.context(format!(
            "file mutation batch failed and rollback was incomplete: {}",
            rollback_errors.join("; ")
        )))
    }
}

pub async fn read_optional(
    environment: &dyn ExecutionEnvironment,
    path: &Path,
) -> anyhow::Result<Option<Vec<u8>>> {
    match environment.read_file(FileReadRequest::new(path)).await {
        Ok(read) => Ok(Some(read.bytes)),
        Err(error) if is_not_found_error(&error) => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read mutation target {}", path.display()))
        }
    }
}

fn is_not_found_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == ErrorKind::NotFound)
    })
}

fn mutation_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::LocalExecutionEnvironment;
    use crate::sandbox::LocalSandboxConfig;
    use std::fs;
    use uuid::Uuid;

    #[tokio::test]
    async fn failed_commit_restores_previously_written_files() {
        let root = std::env::temp_dir().join(format!("opentopia-file-batch-{}", Uuid::new_v4()));
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join("first.txt"), b"old-first").unwrap();
        fs::write(root.join(".git/config"), b"old-config").unwrap();
        let environment =
            LocalExecutionEnvironment::with_sandbox_config(&root, LocalSandboxConfig::enforce());
        let batch = FileMutationBatch::new(vec![
            PreparedFileMutation::write(
                root.join("first.txt"),
                Some(b"old-first".to_vec()),
                b"new-first".to_vec(),
            ),
            PreparedFileMutation::write(
                root.join(".git/config"),
                Some(b"old-config".to_vec()),
                b"new-config".to_vec(),
            ),
        ])
        .unwrap();

        let error = batch.commit(&environment).await.unwrap_err();
        assert!(error.to_string().contains("rolled back"));
        assert_eq!(fs::read(root.join("first.txt")).unwrap(), b"old-first");
        assert_eq!(fs::read(root.join(".git/config")).unwrap(), b"old-config");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn concurrent_stale_writers_cannot_both_commit() {
        let root = std::env::temp_dir().join(format!("opentopia-file-race-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("shared.txt");
        fs::write(&path, b"base").unwrap();
        let environment = Arc::new(LocalExecutionEnvironment::with_sandbox_config(
            &root,
            LocalSandboxConfig::enforce(),
        ));
        let first = FileMutationBatch::new(vec![PreparedFileMutation::write(
            &path,
            Some(b"base".to_vec()),
            b"first".to_vec(),
        )])
        .unwrap();
        let second = FileMutationBatch::new(vec![PreparedFileMutation::write(
            &path,
            Some(b"base".to_vec()),
            b"second".to_vec(),
        )])
        .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(3));

        let first_task = {
            let environment = Arc::clone(&environment);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                first.commit(environment.as_ref()).await
            })
        };
        let second_task = {
            let environment = Arc::clone(&environment);
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                second.commit(environment.as_ref()).await
            })
        };
        barrier.wait().await;
        let first_result = first_task.await.unwrap();
        let second_result = second_task.await.unwrap();

        assert_ne!(first_result.is_ok(), second_result.is_ok());
        let error = first_result.err().or_else(|| second_result.err()).unwrap();
        assert!(error.to_string().contains("changed after patch validation"));
        let final_contents = fs::read(&path).unwrap();
        assert!(final_contents == b"first" || final_contents == b"second");
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn invalid_observer_pairing_is_rejected_before_writing() {
        let root = std::env::temp_dir().join(format!("opentopia-file-scope-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("shared.txt");
        fs::write(&path, b"base").unwrap();
        let environment =
            LocalExecutionEnvironment::with_sandbox_config(&root, LocalSandboxConfig::enforce());
        let batch = FileMutationBatch::new(vec![PreparedFileMutation::write(
            &path,
            Some(b"base".to_vec()),
            b"changed".to_vec(),
        )])
        .unwrap();
        let scope = FileMutationScope {
            thread_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            agent_path: "/root".to_string(),
            workspace_root: root.clone(),
        };

        let error = batch
            .commit_observed(&environment, None, Some(&scope))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("must be provided together"));
        assert_eq!(fs::read(&path).unwrap(), b"base");
        fs::remove_dir_all(root).unwrap();
    }
}
