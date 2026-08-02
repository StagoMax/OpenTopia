use crate::execution::{ExecutionContext, ExecutionEnvironment};
use crate::git_workflow::{
    execute_git_workflow, parse_ahead_behind, parse_branch_list, parse_current_branch,
    parse_remote_list, CommitRequest, CompareRequest, CreateBranchRequest, CreateWorktreeRequest,
    FetchRequest, GitBranchInfo, GitPathsRequest, GitStatusRequest, GitWorkflowAction,
    GitWorkflowActionKind, GitWorkflowError, GitWorkflowParseError, GitWorkflowRequest,
    GitWorkflowResult, ListBranchesRequest, PullRequest, PushRequest, RemoveWorktreeRequest,
    SwitchBranchRequest,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use thiserror::Error;

pub const LOCAL_GIT_V1_API_VERSION: &str = "localGit.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitV1Request {
    pub repository: PathBuf,
    pub operation: LocalGitV1Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "request", rename_all = "snake_case")]
pub enum LocalGitV1Operation {
    Status(GitStatusRequest),
    Branches(ListBranchesRequest),
    Remotes,
    Stage(GitPathsRequest),
    Unstage(GitPathsRequest),
    Discard(LocalGitDiscardRequest),
    CreateBranch(CreateBranchRequest),
    SwitchBranch(SwitchBranchRequest),
    Commit(CommitRequest),
    Push(PushRequest),
    Fetch(FetchRequest),
    Pull(PullRequest),
    Compare(CompareRequest),
    CreateWorktree(CreateWorktreeRequest),
    ListWorktrees,
    RemoveWorktree(LocalGitRemoveWorktreeRequest),
}

impl LocalGitV1Operation {
    pub fn kind(&self) -> GitWorkflowActionKind {
        match self {
            Self::Status(_) => GitWorkflowActionKind::Status,
            Self::Branches(_) => GitWorkflowActionKind::ListBranches,
            Self::Remotes => GitWorkflowActionKind::ListRemotes,
            Self::Stage(_) => GitWorkflowActionKind::Stage,
            Self::Unstage(_) => GitWorkflowActionKind::Unstage,
            Self::Discard(_) => GitWorkflowActionKind::Discard,
            Self::CreateBranch(_) => GitWorkflowActionKind::CreateBranch,
            Self::SwitchBranch(_) => GitWorkflowActionKind::SwitchBranch,
            Self::Commit(_) => GitWorkflowActionKind::Commit,
            Self::Push(_) => GitWorkflowActionKind::Push,
            Self::Fetch(_) => GitWorkflowActionKind::Fetch,
            Self::Pull(_) => GitWorkflowActionKind::Pull,
            Self::Compare(_) => GitWorkflowActionKind::Compare,
            Self::CreateWorktree(_) => GitWorkflowActionKind::CreateWorktree,
            Self::ListWorktrees => GitWorkflowActionKind::ListWorktrees,
            Self::RemoveWorktree(_) => GitWorkflowActionKind::RemoveWorktree,
        }
    }

    pub fn is_mutation(&self) -> bool {
        self.kind().is_mutation()
    }

    /// Stable, argument-free label used by the host Policy engine. User-provided
    /// refs, paths and commit messages are deliberately excluded so this value
    /// can never be mistaken for a command to execute or cause policy injection.
    pub fn policy_command(&self) -> &'static str {
        match self {
            Self::Status(_) => "git status",
            Self::Branches(_) => "git for-each-ref",
            Self::Remotes => "git remote --verbose",
            Self::Stage(_) => "git add",
            Self::Unstage(_) => "git restore --staged",
            Self::Discard(_) => "git restore --source=HEAD --worktree",
            Self::CreateBranch(_) => "git branch",
            Self::SwitchBranch(_) => "git switch",
            Self::Commit(_) => "git commit",
            Self::Push(_) => "git push",
            Self::Fetch(_) => "git fetch --prune",
            Self::Pull(_) => "git pull --ff-only",
            Self::Compare(_) => "git diff --no-ext-diff --no-color",
            Self::CreateWorktree(_) => "git worktree add",
            Self::ListWorktrees => "git worktree list",
            Self::RemoveWorktree(_) => "git worktree remove",
        }
    }

    fn to_workflow_action(&self) -> Result<GitWorkflowAction, LocalGitV1Error> {
        Ok(match self {
            Self::Status(request) => GitWorkflowAction::Status(request.clone()),
            Self::Branches(request) => GitWorkflowAction::ListBranches(request.clone()),
            Self::Remotes => GitWorkflowAction::ListRemotes,
            Self::Stage(request) => GitWorkflowAction::Stage(request.clone()),
            Self::Unstage(request) => GitWorkflowAction::Unstage(request.clone()),
            Self::Discard(request) => {
                request.require_confirmation(GitWorkflowActionKind::Discard)?;
                GitWorkflowAction::Discard(GitPathsRequest {
                    paths: request.paths.clone(),
                })
            }
            Self::CreateBranch(request) => GitWorkflowAction::CreateBranch(request.clone()),
            Self::SwitchBranch(request) => GitWorkflowAction::SwitchBranch(request.clone()),
            Self::Commit(request) => GitWorkflowAction::Commit(request.clone()),
            Self::Push(request) => GitWorkflowAction::Push(request.clone()),
            Self::Fetch(request) => GitWorkflowAction::Fetch(request.clone()),
            Self::Pull(request) => GitWorkflowAction::Pull(request.clone()),
            Self::Compare(request) => GitWorkflowAction::Compare(request.clone()),
            Self::CreateWorktree(request) => GitWorkflowAction::CreateWorktree(request.clone()),
            Self::ListWorktrees => GitWorkflowAction::ListWorktrees,
            Self::RemoveWorktree(request) => {
                request.require_confirmation(GitWorkflowActionKind::RemoveWorktree)?;
                GitWorkflowAction::RemoveWorktree(RemoveWorktreeRequest {
                    path: request.path.clone(),
                    // A forced removal can delete changes in a dirty worktree. localGit.v1
                    // deliberately exposes only Git's safe, non-forced removal behavior.
                    force: false,
                })
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalGitDiscardRequest {
    pub paths: Vec<PathBuf>,
    pub confirm: bool,
}

impl LocalGitDiscardRequest {
    fn require_confirmation(
        &self,
        operation: GitWorkflowActionKind,
    ) -> Result<(), LocalGitV1Error> {
        if !self.confirm {
            return Err(LocalGitV1Error::ConfirmationRequired { operation });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalGitRemoveWorktreeRequest {
    pub path: PathBuf,
    pub confirm: bool,
}

impl LocalGitRemoveWorktreeRequest {
    fn require_confirmation(
        &self,
        operation: GitWorkflowActionKind,
    ) -> Result<(), LocalGitV1Error> {
        if !self.confirm {
            return Err(LocalGitV1Error::ConfirmationRequired { operation });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitV1Response {
    pub api_version: String,
    pub operation: GitWorkflowActionKind,
    pub command: LocalGitCommandSummary,
    pub output: LocalGitV1Output,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitCommandSummary {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub truncated: bool,
    pub stderr: Vec<u8>,
}

impl LocalGitCommandSummary {
    fn from_result(result: &GitWorkflowResult) -> Self {
        Self {
            exit_code: result.exit_code,
            success: result.success,
            truncated: result.truncated,
            stderr: result.stderr.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum LocalGitV1Output {
    Status(LocalGitStatus),
    Branches(Vec<GitBranchInfo>),
    Remotes(Vec<LocalGitRemote>),
    Worktrees(Vec<LocalGitWorktree>),
    Compare(Vec<u8>),
    Mutation(Vec<u8>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitStatus {
    pub branch: Option<String>,
    pub ahead_behind: Option<crate::git_workflow::AheadBehind>,
    pub porcelain_v2: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitRemote {
    pub name: String,
    pub fetch_urls: Vec<NormalizedGitRemoteUrl>,
    pub push_urls: Vec<NormalizedGitRemoteUrl>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitWorktree {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub bare: bool,
    pub locked: bool,
    pub lock_reason: Option<String>,
    pub prunable: bool,
    pub prunable_reason: Option<String>,
}

impl LocalGitRemote {
    pub fn preferred_url(&self) -> Option<&NormalizedGitRemoteUrl> {
        self.fetch_urls.first().or_else(|| self.push_urls.first())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedGitRemoteUrl {
    pub normalized: String,
    pub scheme: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub repository_path: String,
}

pub fn normalize_git_remote_url(raw: &str) -> NormalizedGitRemoteUrl {
    let raw = raw.trim();
    if is_windows_drive_path(raw) {
        let repository_path = normalize_repository_path(raw);
        return NormalizedGitRemoteUrl {
            normalized: repository_path.clone(),
            scheme: None,
            host: None,
            port: None,
            repository_path,
        };
    }
    if let Some(remote) = normalize_scp_remote(raw) {
        return remote;
    }

    if let Ok(mut url) = reqwest::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);

        let scheme = url.scheme().to_ascii_lowercase();
        let host = url.host_str().map(|value| value.to_ascii_lowercase());
        let port = url.port();
        let repository_path = normalize_repository_path(url.path());
        let normalized = normalized_remote_string(
            Some(scheme.as_str()),
            host.as_deref(),
            port,
            &repository_path,
        );
        return NormalizedGitRemoteUrl {
            normalized,
            scheme: Some(scheme),
            host,
            port,
            repository_path,
        };
    }

    let repository_path = normalize_repository_path(raw);
    NormalizedGitRemoteUrl {
        normalized: repository_path.clone(),
        scheme: None,
        host: None,
        port: None,
        repository_path,
    }
}

pub struct LocalGitV1Service<'a> {
    environment: &'a dyn ExecutionEnvironment,
}

impl<'a> LocalGitV1Service<'a> {
    pub fn new(environment: &'a dyn ExecutionEnvironment) -> Self {
        Self { environment }
    }

    pub async fn execute(
        &self,
        request: &LocalGitV1Request,
        context: ExecutionContext,
    ) -> Result<LocalGitV1Response, LocalGitV1Error> {
        let operation = request.operation.kind();
        let workflow = GitWorkflowRequest {
            repository: request.repository.clone(),
            action: request.operation.to_workflow_action()?,
        };
        let result = execute_git_workflow(self.environment, &workflow, context).await?;
        let command = LocalGitCommandSummary::from_result(&result);
        let output = parse_output(&request.operation, &result)?;

        Ok(LocalGitV1Response {
            api_version: LOCAL_GIT_V1_API_VERSION.to_string(),
            operation,
            command,
            output,
        })
    }
}

#[derive(Debug, Error)]
pub enum LocalGitV1Error {
    #[error(transparent)]
    Workflow(#[from] GitWorkflowError),
    #[error(transparent)]
    Parse(#[from] GitWorkflowParseError),
    #[error("git {operation:?} output was not valid UTF-8")]
    NonUtf8Output { operation: GitWorkflowActionKind },
    #[error("localGit.v1 {operation:?} requires explicit confirmation")]
    ConfirmationRequired { operation: GitWorkflowActionKind },
    #[error("git worktree record {record} is invalid: {reason}")]
    InvalidWorktreeOutput { record: usize, reason: &'static str },
}

fn parse_output(
    operation: &LocalGitV1Operation,
    result: &GitWorkflowResult,
) -> Result<LocalGitV1Output, LocalGitV1Error> {
    match operation {
        LocalGitV1Operation::Status(_) => {
            let output = utf8_stdout(result)?;
            Ok(LocalGitV1Output::Status(LocalGitStatus {
                branch: parse_current_branch(output),
                ahead_behind: parse_ahead_behind(output),
                porcelain_v2: output.to_string(),
            }))
        }
        LocalGitV1Operation::Branches(_) => Ok(LocalGitV1Output::Branches(parse_branch_list(
            utf8_stdout(result)?,
        )?)),
        LocalGitV1Operation::Remotes => {
            let remotes = parse_remote_list(utf8_stdout(result)?)?
                .into_iter()
                .map(|remote| LocalGitRemote {
                    name: remote.name,
                    fetch_urls: normalize_remote_urls(remote.fetch_urls),
                    push_urls: normalize_remote_urls(remote.push_urls),
                })
                .collect();
            Ok(LocalGitV1Output::Remotes(remotes))
        }
        LocalGitV1Operation::ListWorktrees => Ok(LocalGitV1Output::Worktrees(parse_worktree_list(
            utf8_stdout(result)?,
        )?)),
        LocalGitV1Operation::Compare(_) => Ok(LocalGitV1Output::Compare(result.stdout.clone())),
        _ => Ok(LocalGitV1Output::Mutation(result.stdout.clone())),
    }
}

fn parse_worktree_list(output: &str) -> Result<Vec<LocalGitWorktree>, LocalGitV1Error> {
    let mut worktrees = Vec::new();
    for (index, record) in output
        .split("\0\0")
        .filter(|record| !record.is_empty())
        .enumerate()
    {
        let record_number = index + 1;
        let mut fields = record.split('\0').filter(|field| !field.is_empty());
        let path = fields
            .next()
            .and_then(|field| field.strip_prefix("worktree "))
            .filter(|path| !path.is_empty())
            .ok_or(LocalGitV1Error::InvalidWorktreeOutput {
                record: record_number,
                reason: "missing worktree path",
            })?;
        let mut worktree = LocalGitWorktree {
            path: PathBuf::from(path),
            head: None,
            branch: None,
            detached: false,
            bare: false,
            locked: false,
            lock_reason: None,
            prunable: false,
            prunable_reason: None,
        };

        for field in fields {
            if let Some(head) = field.strip_prefix("HEAD ") {
                worktree.head = non_empty_string(head);
            } else if let Some(branch) = field.strip_prefix("branch ") {
                worktree.branch = non_empty_string(branch);
            } else if field == "detached" {
                worktree.detached = true;
            } else if field == "bare" {
                worktree.bare = true;
            } else if field == "locked" {
                worktree.locked = true;
            } else if let Some(reason) = field.strip_prefix("locked ") {
                worktree.locked = true;
                worktree.lock_reason = non_empty_string(reason);
            } else if field == "prunable" {
                worktree.prunable = true;
            } else if let Some(reason) = field.strip_prefix("prunable ") {
                worktree.prunable = true;
                worktree.prunable_reason = non_empty_string(reason);
            }
        }
        worktrees.push(worktree);
    }
    Ok(worktrees)
}

fn non_empty_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

fn utf8_stdout(result: &GitWorkflowResult) -> Result<&str, LocalGitV1Error> {
    std::str::from_utf8(&result.stdout).map_err(|_| LocalGitV1Error::NonUtf8Output {
        operation: result.action,
    })
}

fn normalize_remote_urls(urls: Vec<String>) -> Vec<NormalizedGitRemoteUrl> {
    urls.into_iter()
        .map(|url| normalize_git_remote_url(&url))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn normalize_scp_remote(raw: &str) -> Option<NormalizedGitRemoteUrl> {
    if raw.contains("://") {
        return None;
    }
    let (authority, path) = raw.split_once(':')?;
    if authority.is_empty()
        || path.is_empty()
        || authority.contains(['/', '\\'])
        || (authority.len() == 1 && (path.starts_with('/') || path.starts_with('\\')))
    {
        return None;
    }

    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority)
        .trim()
        .to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let repository_path = normalize_repository_path(path);
    Some(NormalizedGitRemoteUrl {
        normalized: normalized_remote_string(Some("ssh"), Some(&host), None, &repository_path),
        scheme: Some("ssh".to_string()),
        host: Some(host),
        port: None,
        repository_path,
    })
}

fn is_windows_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn normalize_repository_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let path = path.trim_matches('/');
    path.strip_suffix(".git").unwrap_or(path).to_string()
}

fn normalized_remote_string(
    scheme: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    repository_path: &str,
) -> String {
    match (scheme, host) {
        (Some(scheme), Some(host)) => {
            let port = port.map(|value| format!(":{value}")).unwrap_or_default();
            format!("{scheme}://{host}{port}/{repository_path}")
        }
        (Some("file"), None) => format!("file:///{repository_path}"),
        _ => repository_path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_https_and_removes_embedded_credentials() {
        let remote = normalize_git_remote_url(
            "https://token:secret@Git.Example.test/Org/Repo.git?access_token=hidden#fragment",
        );

        assert_eq!(remote.normalized, "https://git.example.test/Org/Repo");
        assert_eq!(remote.host.as_deref(), Some("git.example.test"));
        assert_eq!(remote.repository_path, "Org/Repo");
        assert!(!remote.normalized.contains("secret"));
        assert!(!remote.normalized.contains("hidden"));
    }

    #[test]
    fn normalizes_scp_style_ssh_remote_without_userinfo() {
        let remote = normalize_git_remote_url("git@Git.Example.test:Org/Repo.git");

        assert_eq!(remote.normalized, "ssh://git.example.test/Org/Repo");
        assert_eq!(remote.scheme.as_deref(), Some("ssh"));
        assert_eq!(remote.host.as_deref(), Some("git.example.test"));
    }

    #[test]
    fn leaves_local_paths_provider_neutral() {
        let remote = normalize_git_remote_url(r"C:\work\repo.git");

        assert_eq!(remote.host, None);
        assert_eq!(remote.repository_path, "C:/work/repo");
    }

    #[test]
    fn operation_mutation_classification_delegates_to_git_workflow() {
        assert!(!LocalGitV1Operation::Remotes.is_mutation());
        assert!(!LocalGitV1Operation::ListWorktrees.is_mutation());
        assert!(LocalGitV1Operation::Fetch(FetchRequest::default()).is_mutation());
        assert!(LocalGitV1Operation::Commit(CommitRequest {
            message: "message".to_string(),
            all_tracked: false,
        })
        .is_mutation());
    }

    #[test]
    fn policy_command_labels_exclude_user_input() {
        let push = LocalGitV1Operation::Push(PushRequest {
            remote: "origin".to_string(),
            branch: "topic/git reset --hard".to_string(),
            set_upstream: false,
        });
        let commit = LocalGitV1Operation::Commit(CommitRequest {
            message: "mention git reset --hard without executing it".to_string(),
            all_tracked: false,
        });

        assert_eq!(push.policy_command(), "git push");
        assert_eq!(commit.policy_command(), "git commit");
    }

    #[test]
    fn remote_output_is_structured_and_deduplicated() {
        let operation = LocalGitV1Operation::Remotes;
        let result = GitWorkflowResult {
            action: GitWorkflowActionKind::ListRemotes,
            stdout: concat!(
                "origin\thttps://token@git.example.test/org/repo.git (fetch)\n",
                "origin\thttps://token@git.example.test/org/repo.git (fetch)\n",
                "origin\tgit@git.example.test:org/repo.git (push)\n",
            )
            .as_bytes()
            .to_vec(),
            stderr: Vec::new(),
            exit_code: Some(0),
            success: true,
            truncated: false,
        };

        let LocalGitV1Output::Remotes(remotes) = parse_output(&operation, &result).unwrap() else {
            panic!("expected remote output");
        };
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].fetch_urls.len(), 1);
        assert_eq!(
            remotes[0].fetch_urls[0].normalized,
            "https://git.example.test/org/repo"
        );
    }

    #[test]
    fn expanded_operations_delegate_to_validated_workflow_actions() {
        let stage = LocalGitV1Operation::Stage(GitPathsRequest {
            paths: vec![PathBuf::from("src/lib.rs")],
        })
        .to_workflow_action()
        .unwrap();
        assert!(matches!(stage, GitWorkflowAction::Stage(_)));

        let fetch = LocalGitV1Operation::Fetch(FetchRequest {
            remote: Some("origin".to_string()),
        })
        .to_workflow_action()
        .unwrap();
        assert!(matches!(fetch, GitWorkflowAction::Fetch(_)));

        let pull = LocalGitV1Operation::Pull(PullRequest {
            remote: Some("origin".to_string()),
            branch: Some("main".to_string()),
        })
        .to_workflow_action()
        .unwrap();
        assert!(matches!(pull, GitWorkflowAction::Pull(_)));
        assert!(matches!(
            LocalGitV1Operation::ListWorktrees
                .to_workflow_action()
                .unwrap(),
            GitWorkflowAction::ListWorktrees
        ));
    }

    #[test]
    fn destructive_local_git_operations_require_confirmation() {
        let discard = LocalGitV1Operation::Discard(LocalGitDiscardRequest {
            paths: vec![PathBuf::from("src/lib.rs")],
            confirm: false,
        });
        assert!(matches!(
            discard.to_workflow_action(),
            Err(LocalGitV1Error::ConfirmationRequired {
                operation: GitWorkflowActionKind::Discard
            })
        ));

        let remove = LocalGitV1Operation::RemoveWorktree(LocalGitRemoveWorktreeRequest {
            path: PathBuf::from("../feature-tree"),
            confirm: false,
        });
        assert!(matches!(
            remove.to_workflow_action(),
            Err(LocalGitV1Error::ConfirmationRequired {
                operation: GitWorkflowActionKind::RemoveWorktree
            })
        ));
    }

    #[test]
    fn local_git_worktree_removal_never_forces_dirty_worktree_deletion() {
        let remove = LocalGitV1Operation::RemoveWorktree(LocalGitRemoveWorktreeRequest {
            path: PathBuf::from("../feature-tree"),
            confirm: true,
        })
        .to_workflow_action()
        .unwrap();

        let GitWorkflowAction::RemoveWorktree(remove) = remove else {
            panic!("expected worktree removal");
        };
        assert!(!remove.force);

        let serialized = serde_json::json!({
            "type": "remove_worktree",
            "request": {
                "path": "../feature-tree",
                "confirm": true,
                "force": true
            }
        });
        assert!(serde_json::from_value::<LocalGitV1Operation>(serialized).is_err());
    }

    #[test]
    fn worktree_output_is_structured_from_nul_terminated_porcelain() {
        let operation = LocalGitV1Operation::ListWorktrees;
        let result = GitWorkflowResult {
            action: GitWorkflowActionKind::ListWorktrees,
            stdout: concat!(
                "worktree C:/repo\0",
                "HEAD 0123456789abcdef\0",
                "branch refs/heads/main\0\0",
                "worktree C:/trees/review\0",
                "HEAD fedcba9876543210\0",
                "detached\0",
                "locked active review\0",
                "prunable gitdir file points to non-existent location\0\0",
            )
            .as_bytes()
            .to_vec(),
            stderr: Vec::new(),
            exit_code: Some(0),
            success: true,
            truncated: false,
        };

        let LocalGitV1Output::Worktrees(worktrees) = parse_output(&operation, &result).unwrap()
        else {
            panic!("expected structured worktree output");
        };
        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].path, PathBuf::from("C:/repo"));
        assert_eq!(worktrees[0].branch.as_deref(), Some("refs/heads/main"));
        assert!(worktrees[1].detached);
        assert!(worktrees[1].locked);
        assert_eq!(worktrees[1].lock_reason.as_deref(), Some("active review"));
        assert!(worktrees[1].prunable);
    }

    #[test]
    fn malformed_worktree_output_is_rejected() {
        let error = parse_worktree_list("HEAD abc123\0\0").unwrap_err();
        assert!(matches!(
            error,
            LocalGitV1Error::InvalidWorktreeOutput {
                record: 1,
                reason: "missing worktree path"
            }
        ));
    }
}
