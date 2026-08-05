use crate::execution::{
    ExecutionEnvironment, FileDeleteRequest, FileReadRequest, FileWriteRequest,
};
use anyhow::Context;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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
        let mut applied = Vec::<&PreparedFileMutation>::new();

        for mutation in &self.mutations {
            let current = read_optional(environment, &mutation.path).await?;
            if current != mutation.original {
                let error = anyhow::anyhow!(
                    "file changed after patch validation: {}",
                    mutation.path.display()
                );
                return rollback_after_error(environment, &applied, error).await;
            }

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

        Ok(FileMutationBatchResult {
            changed_paths: self
                .mutations
                .iter()
                .map(|mutation| mutation.path.clone())
                .collect(),
        })
    }
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
}
