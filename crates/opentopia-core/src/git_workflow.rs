use crate::execution::{ExecRequest, ExecResult, ExecutionContext, ExecutionEnvironment};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_COMMIT_MESSAGE_BYTES: usize = 32 * 1024;
const MAX_REF_BYTES: usize = 1_024;
const MAX_REMOTE_BYTES: usize = 255;
const BRANCH_LIST_FORMAT: &str =
    "%(refname)%00%(refname:short)%00%(HEAD)%00%(upstream:short)%00%(symref)";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkflowRequest {
    pub repository: PathBuf,
    pub action: GitWorkflowAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "request", rename_all = "snake_case")]
pub enum GitWorkflowAction {
    Status(GitStatusRequest),
    ListBranches(ListBranchesRequest),
    CreateBranch(CreateBranchRequest),
    SwitchBranch(SwitchBranchRequest),
    Commit(CommitRequest),
    Push(PushRequest),
    Compare(CompareRequest),
    CreateWorktree(CreateWorktreeRequest),
}

impl GitWorkflowAction {
    pub fn kind(&self) -> GitWorkflowActionKind {
        match self {
            Self::Status(_) => GitWorkflowActionKind::Status,
            Self::ListBranches(_) => GitWorkflowActionKind::ListBranches,
            Self::CreateBranch(_) => GitWorkflowActionKind::CreateBranch,
            Self::SwitchBranch(_) => GitWorkflowActionKind::SwitchBranch,
            Self::Commit(_) => GitWorkflowActionKind::Commit,
            Self::Push(_) => GitWorkflowActionKind::Push,
            Self::Compare(_) => GitWorkflowActionKind::Compare,
            Self::CreateWorktree(_) => GitWorkflowActionKind::CreateWorktree,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitWorkflowActionKind {
    Status,
    ListBranches,
    CreateBranch,
    SwitchBranch,
    Commit,
    Push,
    Compare,
    CreateWorktree,
}

impl GitWorkflowActionKind {
    pub fn is_mutation(self) -> bool {
        matches!(
            self,
            Self::CreateBranch
                | Self::SwitchBranch
                | Self::Commit
                | Self::Push
                | Self::CreateWorktree
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusRequest {
    pub include_untracked: bool,
}

impl Default for GitStatusRequest {
    fn default() -> Self {
        Self {
            include_untracked: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListBranchesRequest {
    pub include_remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateBranchRequest {
    pub branch: String,
    pub start_point: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SwitchBranchRequest {
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitRequest {
    pub message: String,
    pub all_tracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub remote: String,
    pub branch: String,
    pub set_upstream: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompareMode {
    #[default]
    Direct,
    MergeBase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompareRequest {
    pub base: String,
    pub head: String,
    pub mode: CompareMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorktreeRequest {
    pub path: PathBuf,
    pub target: WorktreeTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorktreeTarget {
    ExistingBranch {
        branch: String,
    },
    NewBranch {
        branch: String,
        start_point: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitWorkflowResult {
    pub action: GitWorkflowActionKind,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub truncated: bool,
}

impl GitWorkflowResult {
    fn from_exec(action: GitWorkflowActionKind, result: ExecResult) -> Self {
        Self {
            action,
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
            success: result.success,
            truncated: result.truncated,
        }
    }

    pub fn stdout_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    pub fn stderr_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

#[derive(Debug, Error)]
pub enum GitWorkflowError {
    #[error("repository path is empty")]
    EmptyRepositoryPath,
    #[error("repository path contains a NUL or line break")]
    InvalidRepositoryPath,
    #[error("invalid branch {value:?}: {reason}")]
    InvalidBranch { value: String, reason: &'static str },
    #[error("invalid ref {value:?}: {reason}")]
    InvalidRef { value: String, reason: &'static str },
    #[error("invalid remote {value:?}: {reason}")]
    InvalidRemote { value: String, reason: &'static str },
    #[error("invalid commit message: {reason}")]
    InvalidCommitMessage { reason: &'static str },
    #[error("worktree path is empty")]
    EmptyWorktreePath,
    #[error("worktree path must be valid Unicode")]
    NonUnicodeWorktreePath,
    #[error("worktree path contains a NUL or line break")]
    InvalidWorktreePath,
    #[error("failed to execute git {action:?}: {source}")]
    Execution {
        action: GitWorkflowActionKind,
        #[source]
        source: anyhow::Error,
    },
    #[error("git mutation {action:?} failed")]
    MutationFailed {
        action: GitWorkflowActionKind,
        result: GitWorkflowResult,
    },
}

impl GitWorkflowError {
    pub fn failed_result(&self) -> Option<&GitWorkflowResult> {
        match self {
            Self::MutationFailed { result, .. } => Some(result),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitBranchInfo {
    pub full_ref: String,
    pub name: String,
    pub current: bool,
    pub remote: bool,
    pub upstream: Option<String>,
    pub symbolic_target: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AheadBehind {
    pub ahead: u64,
    pub behind: u64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum GitWorkflowParseError {
    #[error("branch list record {line} has {actual} fields; expected 5")]
    InvalidBranchRecord { line: usize, actual: usize },
    #[error("branch list record {line} has an empty ref or branch name")]
    EmptyBranchRecord { line: usize },
}

pub fn validate_branch(branch: &str) -> Result<(), GitWorkflowError> {
    validate_ref_token(branch).map_err(|reason| GitWorkflowError::InvalidBranch {
        value: branch.to_string(),
        reason,
    })?;

    if branch.starts_with("refs/") {
        return Err(GitWorkflowError::InvalidBranch {
            value: branch.to_string(),
            reason: "branch must use a short name, not a refs/ path",
        });
    }

    if is_reserved_branch_name(branch) {
        return Err(GitWorkflowError::InvalidBranch {
            value: branch.to_string(),
            reason: "name is reserved by Git",
        });
    }

    Ok(())
}

pub fn validate_ref(reference: &str) -> Result<(), GitWorkflowError> {
    validate_ref_token(reference).map_err(|reason| GitWorkflowError::InvalidRef {
        value: reference.to_string(),
        reason,
    })
}

pub fn validate_remote(remote: &str) -> Result<(), GitWorkflowError> {
    let error = |reason| GitWorkflowError::InvalidRemote {
        value: remote.to_string(),
        reason,
    };

    if remote.is_empty() {
        return Err(error("remote is empty"));
    }
    if remote.len() > MAX_REMOTE_BYTES {
        return Err(error("remote is too long"));
    }
    if remote.starts_with('-') {
        return Err(error("remote starts with '-'"));
    }
    if remote.contains(['\0', '\r', '\n']) {
        return Err(error("remote contains a NUL or line break"));
    }
    if remote == "." || remote == ".." {
        return Err(error("remote is not a named remote"));
    }
    if !remote
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(error(
            "remote contains characters outside the safe name set",
        ));
    }

    Ok(())
}

pub fn validate_commit_message(message: &str) -> Result<(), GitWorkflowError> {
    if message.as_bytes().contains(&0) {
        return Err(GitWorkflowError::InvalidCommitMessage {
            reason: "message contains a NUL",
        });
    }
    if message.trim().is_empty() {
        return Err(GitWorkflowError::InvalidCommitMessage {
            reason: "message is empty",
        });
    }
    if message.len() > MAX_COMMIT_MESSAGE_BYTES {
        return Err(GitWorkflowError::InvalidCommitMessage {
            reason: "message exceeds the byte limit",
        });
    }

    Ok(())
}

pub fn build_git_exec_request(
    request: &GitWorkflowRequest,
) -> Result<ExecRequest, GitWorkflowError> {
    validate_repository_path(&request.repository)?;

    let mut args = Vec::new();
    match &request.action {
        GitWorkflowAction::Status(status) => {
            args.extend(strings([
                "status",
                "--porcelain=v2",
                "--branch",
                "--ahead-behind",
            ]));
            args.push(if status.include_untracked {
                "--untracked-files=all".to_string()
            } else {
                "--untracked-files=no".to_string()
            });
        }
        GitWorkflowAction::ListBranches(list) => {
            args.extend(strings([
                "for-each-ref",
                &format!("--format={BRANCH_LIST_FORMAT}"),
                "refs/heads/",
            ]));
            if list.include_remote {
                args.push("refs/remotes/".to_string());
            }
        }
        GitWorkflowAction::CreateBranch(create) => {
            validate_branch(&create.branch)?;
            if let Some(start_point) = create.start_point.as_deref() {
                validate_ref(start_point)?;
            }
            args.extend(strings(["branch", "--", &create.branch]));
            if let Some(start_point) = &create.start_point {
                args.push(start_point.clone());
            }
        }
        GitWorkflowAction::SwitchBranch(switch) => {
            validate_branch(&switch.branch)?;
            args.extend(strings(["switch", "--", &switch.branch]));
        }
        GitWorkflowAction::Commit(commit) => {
            validate_commit_message(&commit.message)?;
            args.push("commit".to_string());
            if commit.all_tracked {
                args.push("--all".to_string());
            }
            args.extend(strings(["--message", &commit.message]));
        }
        GitWorkflowAction::Push(push) => {
            validate_remote(&push.remote)?;
            validate_branch(&push.branch)?;
            args.push("push".to_string());
            if push.set_upstream {
                args.push("--set-upstream".to_string());
            }
            args.extend(strings(["--", &push.remote, &push.branch]));
        }
        GitWorkflowAction::Compare(compare) => {
            validate_ref(&compare.base)?;
            validate_ref(&compare.head)?;
            args.extend(strings(["diff", "--no-ext-diff", "--no-color"]));
            if compare.mode == CompareMode::MergeBase {
                args.push("--merge-base".to_string());
            }
            args.extend(strings([&compare.base, &compare.head, "--"]));
        }
        GitWorkflowAction::CreateWorktree(worktree) => {
            let path = validate_worktree_path(&worktree.path)?;
            args.extend(strings(["worktree", "add"]));
            match &worktree.target {
                WorktreeTarget::ExistingBranch { branch } => {
                    validate_branch(branch)?;
                    args.extend(strings(["--", path, branch]));
                }
                WorktreeTarget::NewBranch {
                    branch,
                    start_point,
                } => {
                    validate_branch(branch)?;
                    if let Some(start_point) = start_point.as_deref() {
                        validate_ref(start_point)?;
                    }
                    args.extend(strings(["-b", branch, "--", path]));
                    if let Some(start_point) = start_point {
                        args.push(start_point.clone());
                    }
                }
            }
        }
    }

    Ok(ExecRequest::new("git")
        .args(args)
        .cwd(request.repository.clone()))
}

pub async fn execute_git_workflow(
    environment: &dyn ExecutionEnvironment,
    request: &GitWorkflowRequest,
    context: ExecutionContext,
) -> Result<GitWorkflowResult, GitWorkflowError> {
    let action = request.action.kind();
    let exec_request = build_git_exec_request(request)?;
    let exec_result = environment
        .exec(exec_request, context)
        .await
        .map_err(|source| GitWorkflowError::Execution { action, source })?;
    let result = GitWorkflowResult::from_exec(action, exec_result);

    if action.is_mutation() && !result.success {
        return Err(GitWorkflowError::MutationFailed { action, result });
    }

    Ok(result)
}

pub fn parse_current_branch(status_output: &str) -> Option<String> {
    status_output.lines().find_map(|line| {
        let branch = line.strip_prefix("# branch.head ")?.trim_end_matches('\r');
        if matches!(branch, "(detached)" | "(unknown)") || branch.is_empty() {
            None
        } else {
            Some(branch.to_string())
        }
    })
}

pub fn parse_branch_list(output: &str) -> Result<Vec<GitBranchInfo>, GitWorkflowParseError> {
    let mut branches = Vec::new();
    for (index, raw_line) in output.lines().enumerate() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }

        let fields = line.split('\0').collect::<Vec<_>>();
        if fields.len() != 5 {
            return Err(GitWorkflowParseError::InvalidBranchRecord {
                line: index + 1,
                actual: fields.len(),
            });
        }
        if fields[0].is_empty() || fields[1].is_empty() {
            return Err(GitWorkflowParseError::EmptyBranchRecord { line: index + 1 });
        }

        branches.push(GitBranchInfo {
            full_ref: fields[0].to_string(),
            name: fields[1].to_string(),
            current: fields[2] == "*",
            remote: fields[0].starts_with("refs/remotes/"),
            upstream: option_string(fields[3]),
            symbolic_target: option_string(fields[4]),
        });
    }

    Ok(branches)
}

pub fn parse_ahead_behind(status_output: &str) -> Option<AheadBehind> {
    status_output.lines().find_map(|line| {
        let values = line.strip_prefix("# branch.ab ")?;
        let mut fields = values.split_whitespace();
        let ahead = fields.next()?.strip_prefix('+')?.parse().ok()?;
        let behind = fields.next()?.strip_prefix('-')?.parse().ok()?;
        if fields.next().is_some() {
            return None;
        }
        Some(AheadBehind { ahead, behind })
    })
}

fn validate_repository_path(path: &Path) -> Result<(), GitWorkflowError> {
    if path.as_os_str().is_empty() {
        return Err(GitWorkflowError::EmptyRepositoryPath);
    }
    if path.to_string_lossy().contains(['\0', '\r', '\n']) {
        return Err(GitWorkflowError::InvalidRepositoryPath);
    }
    Ok(())
}

fn validate_worktree_path(path: &Path) -> Result<&str, GitWorkflowError> {
    if path.as_os_str().is_empty() {
        return Err(GitWorkflowError::EmptyWorktreePath);
    }
    let path = path
        .to_str()
        .ok_or(GitWorkflowError::NonUnicodeWorktreePath)?;
    if path.contains(['\0', '\r', '\n']) {
        return Err(GitWorkflowError::InvalidWorktreePath);
    }
    Ok(path)
}

fn validate_ref_token(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        return Err("name is empty");
    }
    if value.len() > MAX_REF_BYTES {
        return Err("name is too long");
    }
    if value.starts_with('-') {
        return Err("name starts with '-'");
    }
    if value == "@" {
        return Err("'@' is an implicit HEAD alias");
    }
    if value.contains(['\0', '\r', '\n']) {
        return Err("name contains a NUL or line break");
    }
    if value.starts_with('/') || value.ends_with('/') || value.contains("//") {
        return Err("name contains an empty path component");
    }
    if value.contains("..") {
        return Err("name contains a revision range or consecutive dots");
    }
    if value.contains("@{") {
        return Err("name contains reflog syntax");
    }
    if !value.chars().all(is_safe_ref_character) {
        return Err("name contains an unsafe or revision-control character");
    }

    for component in value.split('/') {
        if component.starts_with('.') || component.ends_with('.') {
            return Err("a path component starts or ends with '.'");
        }
        if component.to_ascii_lowercase().ends_with(".lock") {
            return Err("a path component ends with '.lock'");
        }
    }

    Ok(())
}

fn is_safe_ref_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
}

fn is_reserved_branch_name(branch: &str) -> bool {
    const RESERVED: &[&str] = &[
        "HEAD",
        "FETCH_HEAD",
        "ORIG_HEAD",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "BISECT_HEAD",
        "AUTO_MERGE",
    ];
    RESERVED
        .iter()
        .any(|reserved| branch.eq_ignore_ascii_case(reserved))
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_string).collect()
}

fn option_string(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{
        FileReadRequest, FileReadResult, FileWriteRequest, StdioSession, WriteResult,
    };
    use crate::sandbox::ExecutionEnvironmentKind;
    use async_trait::async_trait;
    use std::sync::Mutex;

    fn workflow(action: GitWorkflowAction) -> GitWorkflowRequest {
        GitWorkflowRequest {
            repository: PathBuf::from("C:/work/repository"),
            action,
        }
    }

    fn built(action: GitWorkflowAction) -> ExecRequest {
        build_git_exec_request(&workflow(action)).expect("request should be valid")
    }

    #[test]
    fn status_uses_porcelain_v2_and_ahead_behind() {
        let request = built(GitWorkflowAction::Status(GitStatusRequest::default()));

        assert_eq!(request.program, "git");
        assert_eq!(
            request.args,
            strings([
                "status",
                "--porcelain=v2",
                "--branch",
                "--ahead-behind",
                "--untracked-files=all",
            ])
        );
    }

    #[test]
    fn status_can_omit_untracked_files() {
        let request = built(GitWorkflowAction::Status(GitStatusRequest {
            include_untracked: false,
        }));

        assert_eq!(request.args.last().unwrap(), "--untracked-files=no");
    }

    #[test]
    fn list_branches_uses_machine_readable_fields() {
        let request = built(GitWorkflowAction::ListBranches(ListBranchesRequest {
            include_remote: true,
        }));

        assert_eq!(request.args[0], "for-each-ref");
        assert_eq!(request.args[1], format!("--format={BRANCH_LIST_FORMAT}"));
        assert_eq!(
            &request.args[2..],
            &strings(["refs/heads/", "refs/remotes/"])
        );
    }

    #[test]
    fn create_branch_keeps_start_point_in_a_separate_argument() {
        let request = built(GitWorkflowAction::CreateBranch(CreateBranchRequest {
            branch: "feature/safe".to_string(),
            start_point: Some("origin/main".to_string()),
        }));

        assert_eq!(
            request.args,
            strings(["branch", "--", "feature/safe", "origin/main"])
        );
    }

    #[test]
    fn switch_branch_uses_end_of_options_marker() {
        let request = built(GitWorkflowAction::SwitchBranch(SwitchBranchRequest {
            branch: "feature/safe".to_string(),
        }));

        assert_eq!(request.args, strings(["switch", "--", "feature/safe"]));
    }

    #[test]
    fn commit_message_is_one_argument_even_when_it_looks_like_shell_input() {
        let message = "finish feature; Remove-Item -Recurse .\n\"quoted\" && echo nope";
        let request = built(GitWorkflowAction::Commit(CommitRequest {
            message: message.to_string(),
            all_tracked: false,
        }));

        assert_eq!(request.program, "git");
        assert_eq!(request.args, vec!["commit", "--message", message]);
    }

    #[test]
    fn unicode_commit_message_is_preserved() {
        let message = "修复工作树切换，并保留 emoji ✓";
        let request = built(GitWorkflowAction::Commit(CommitRequest {
            message: message.to_string(),
            all_tracked: true,
        }));

        assert_eq!(request.args, vec!["commit", "--all", "--message", message]);
    }

    #[test]
    fn push_validates_names_and_separates_arguments() {
        let request = built(GitWorkflowAction::Push(PushRequest {
            remote: "origin".to_string(),
            branch: "feature/safe".to_string(),
            set_upstream: true,
        }));

        assert_eq!(
            request.args,
            strings(["push", "--set-upstream", "--", "origin", "feature/safe",])
        );
    }

    #[test]
    fn compare_never_builds_a_revision_expression() {
        let request = built(GitWorkflowAction::Compare(CompareRequest {
            base: "origin/main".to_string(),
            head: "feature/safe".to_string(),
            mode: CompareMode::MergeBase,
        }));

        assert_eq!(
            request.args,
            strings([
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--merge-base",
                "origin/main",
                "feature/safe",
                "--",
            ])
        );
        assert!(!request.args.iter().any(|arg| arg.contains("...")));
    }

    #[test]
    fn windows_worktree_path_stays_in_one_argument() {
        let path = PathBuf::from(r"C:\Users\A Person\trees\feature one");
        let request = built(GitWorkflowAction::CreateWorktree(CreateWorktreeRequest {
            path: path.clone(),
            target: WorktreeTarget::ExistingBranch {
                branch: "feature/windows".to_string(),
            },
        }));

        assert_eq!(request.args[2], "--");
        assert_eq!(request.args[3], path.to_str().unwrap());
        assert_eq!(request.args.len(), 5);
    }

    #[test]
    fn windows_repository_path_is_preserved_as_cwd() {
        let repository = PathBuf::from(r"C:\Users\A Person\source\repo");
        let request = GitWorkflowRequest {
            repository: repository.clone(),
            action: GitWorkflowAction::Status(GitStatusRequest::default()),
        };

        let command = build_git_exec_request(&request).unwrap();
        assert_eq!(command.cwd, Some(repository));
    }

    #[test]
    fn new_worktree_branch_and_start_point_are_separate_arguments() {
        let request = built(GitWorkflowAction::CreateWorktree(CreateWorktreeRequest {
            path: PathBuf::from("../trees/new-feature"),
            target: WorktreeTarget::NewBranch {
                branch: "feature/new".to_string(),
                start_point: Some("origin/main".to_string()),
            },
        }));

        assert_eq!(
            request.args,
            strings([
                "worktree",
                "add",
                "-b",
                "feature/new",
                "--",
                "../trees/new-feature",
                "origin/main",
            ])
        );
    }

    #[test]
    fn rejects_option_injection_for_branch_ref_and_remote() {
        assert!(matches!(
            validate_branch("--force"),
            Err(GitWorkflowError::InvalidBranch { .. })
        ));
        assert!(matches!(
            validate_ref("--output=/tmp/result"),
            Err(GitWorkflowError::InvalidRef { .. })
        ));
        assert!(matches!(
            validate_remote("--upload-pack=payload"),
            Err(GitWorkflowError::InvalidRemote { .. })
        ));
    }

    #[test]
    fn rejects_line_break_and_nul_in_names() {
        for branch in ["safe\n--force", "safe\rmain", "safe\0main"] {
            assert!(validate_branch(branch).is_err(), "accepted {branch:?}");
        }
        for reference in ["main\nHEAD", "main\rHEAD", "main\0HEAD"] {
            assert!(validate_ref(reference).is_err(), "accepted {reference:?}");
        }
        for remote in ["origin\n--force", "origin\rnext", "origin\0next"] {
            assert!(validate_remote(remote).is_err(), "accepted {remote:?}");
        }
    }

    #[test]
    fn rejects_revision_operators_and_dangerous_ref_components() {
        for reference in [
            "main..next",
            "main...next",
            "main@{1}",
            "main~2",
            "main^{}",
            "main:path",
            "main.lock",
            ".hidden/main",
            "main//next",
            "main/.",
        ] {
            assert!(validate_ref(reference).is_err(), "accepted {reference:?}");
        }
    }

    #[test]
    fn accepts_normal_branches_refs_and_remotes() {
        for branch in ["main", "feature/git-workflow", "修复/工作树"] {
            validate_branch(branch).unwrap();
        }
        for reference in ["HEAD", "origin/main", "refs/tags/v1.2.3", "a1b2c3d"] {
            validate_ref(reference).unwrap();
        }
        for remote in ["origin", "upstream-2", "company.mirror"] {
            validate_remote(remote).unwrap();
        }
    }

    #[test]
    fn rejects_reserved_or_full_branch_names() {
        for branch in ["HEAD", "fetch_head", "refs/heads/main"] {
            assert!(validate_branch(branch).is_err(), "accepted {branch:?}");
        }
    }

    #[test]
    fn rejects_unsafe_ascii_punctuation_in_names() {
        for branch in ["feature;payload", "feature$payload", "feature payload"] {
            assert!(validate_branch(branch).is_err(), "accepted {branch:?}");
        }
        for remote in ["https://example.test/repo", "origin/path", "origin name"] {
            assert!(validate_remote(remote).is_err(), "accepted {remote:?}");
        }
    }

    #[test]
    fn commit_message_rejects_empty_nul_and_oversized_values() {
        assert!(validate_commit_message(" \r\n\t").is_err());
        assert!(validate_commit_message("subject\0body").is_err());
        assert!(validate_commit_message(&"x".repeat(MAX_COMMIT_MESSAGE_BYTES + 1)).is_err());
        validate_commit_message(&"x".repeat(MAX_COMMIT_MESSAGE_BYTES)).unwrap();
    }

    #[test]
    fn worktree_path_with_line_break_is_rejected() {
        let request = workflow(GitWorkflowAction::CreateWorktree(CreateWorktreeRequest {
            path: PathBuf::from("../tree\n--force"),
            target: WorktreeTarget::ExistingBranch {
                branch: "main".to_string(),
            },
        }));

        assert!(matches!(
            build_git_exec_request(&request),
            Err(GitWorkflowError::InvalidWorktreePath)
        ));
    }

    #[test]
    fn parses_current_branch_from_porcelain_v2() {
        let output = "# branch.oid abc123\n# branch.head feature/safe\n1 .M N... file.rs\n";
        assert_eq!(
            parse_current_branch(output),
            Some("feature/safe".to_string())
        );
    }

    #[test]
    fn detached_head_has_no_current_branch() {
        let output = "# branch.oid abc123\n# branch.head (detached)\n";
        assert_eq!(parse_current_branch(output), None);
    }

    #[test]
    fn parses_branch_list_with_remote_and_symbolic_ref() {
        let output = concat!(
            "refs/heads/main\0main\0*\0origin/main\0\n",
            "refs/remotes/origin/HEAD\0origin/HEAD\0 \0\0refs/remotes/origin/main\n",
            "refs/remotes/origin/main\0origin/main\0 \0\0\n"
        );

        let branches = parse_branch_list(output).unwrap();
        assert_eq!(branches.len(), 3);
        assert!(branches[0].current);
        assert_eq!(branches[0].upstream.as_deref(), Some("origin/main"));
        assert!(branches[1].remote);
        assert_eq!(
            branches[1].symbolic_target.as_deref(),
            Some("refs/remotes/origin/main")
        );
        assert_eq!(branches[2].name, "origin/main");
    }

    #[test]
    fn malformed_branch_list_is_rejected() {
        let error = parse_branch_list("refs/heads/main\0main\0*\n").unwrap_err();
        assert_eq!(
            error,
            GitWorkflowParseError::InvalidBranchRecord { line: 1, actual: 3 }
        );
    }

    #[test]
    fn parses_ahead_and_behind_counts() {
        let output = concat!(
            "# branch.head main\n",
            "# branch.upstream origin/main\n",
            "# branch.ab +12 -3\n"
        );

        assert_eq!(
            parse_ahead_behind(output),
            Some(AheadBehind {
                ahead: 12,
                behind: 3,
            })
        );
    }

    #[test]
    fn missing_or_malformed_ahead_behind_is_none() {
        assert_eq!(parse_ahead_behind("# branch.head main\n"), None);
        assert_eq!(parse_ahead_behind("# branch.ab +many -2\n"), None);
        assert_eq!(parse_ahead_behind("# branch.ab +1 -2 extra\n"), None);
    }

    #[test]
    fn action_kind_marks_only_mutations() {
        assert!(GitWorkflowActionKind::CreateBranch.is_mutation());
        assert!(GitWorkflowActionKind::SwitchBranch.is_mutation());
        assert!(GitWorkflowActionKind::Commit.is_mutation());
        assert!(GitWorkflowActionKind::Push.is_mutation());
        assert!(GitWorkflowActionKind::CreateWorktree.is_mutation());
        assert!(!GitWorkflowActionKind::Status.is_mutation());
        assert!(!GitWorkflowActionKind::ListBranches.is_mutation());
        assert!(!GitWorkflowActionKind::Compare.is_mutation());
    }

    struct MockEnvironment {
        root: PathBuf,
        requests: Mutex<Vec<ExecRequest>>,
        result: ExecResult,
        execution_error: Option<String>,
    }

    impl MockEnvironment {
        fn with_result(result: ExecResult) -> Self {
            Self {
                root: PathBuf::from("C:/work"),
                requests: Mutex::new(Vec::new()),
                result,
                execution_error: None,
            }
        }
    }

    #[async_trait]
    impl ExecutionEnvironment for MockEnvironment {
        fn id(&self) -> &str {
            "git-workflow-test"
        }

        fn kind(&self) -> ExecutionEnvironmentKind {
            ExecutionEnvironmentKind::Local
        }

        fn workspace_root(&self) -> &Path {
            &self.root
        }

        async fn exec(
            &self,
            request: ExecRequest,
            _context: ExecutionContext,
        ) -> anyhow::Result<ExecResult> {
            self.requests.lock().unwrap().push(request);
            if let Some(message) = &self.execution_error {
                anyhow::bail!(message.clone());
            }
            Ok(self.result.clone())
        }

        async fn spawn_stdio(
            &self,
            _request: ExecRequest,
            _context: ExecutionContext,
        ) -> anyhow::Result<Box<dyn StdioSession>> {
            anyhow::bail!("not used by git workflow tests")
        }

        async fn read_file(&self, _request: FileReadRequest) -> anyhow::Result<FileReadResult> {
            anyhow::bail!("not used by git workflow tests")
        }

        async fn write_file(&self, _request: FileWriteRequest) -> anyhow::Result<WriteResult> {
            anyhow::bail!("not used by git workflow tests")
        }

        async fn cancel(&self, _request_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("not used by git workflow tests")
        }
    }

    fn exec_result(success: bool) -> ExecResult {
        ExecResult {
            stdout: b"stdout".to_vec(),
            stderr: b"stderr".to_vec(),
            exit_code: Some(if success { 0 } else { 1 }),
            success,
            truncated: false,
            sandbox: None,
        }
    }

    #[tokio::test]
    async fn execution_goes_through_environment_with_caller_context() {
        let environment = MockEnvironment::with_result(exec_result(true));
        let request = workflow(GitWorkflowAction::Status(GitStatusRequest::default()));

        let result = execute_git_workflow(
            &environment,
            &request,
            ExecutionContext::with_timeout(std::time::Duration::from_secs(7)),
        )
        .await
        .unwrap();

        assert!(result.success);
        assert_eq!(result.stdout, b"stdout");
        let requests = environment.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].program, "git");
        assert_eq!(requests[0].cwd, Some(request.repository));
    }

    #[tokio::test]
    async fn failed_mutation_is_a_structured_error() {
        let environment = MockEnvironment::with_result(exec_result(false));
        let request = workflow(GitWorkflowAction::SwitchBranch(SwitchBranchRequest {
            branch: "main".to_string(),
        }));

        let error = execute_git_workflow(&environment, &request, ExecutionContext::default())
            .await
            .unwrap_err();

        let result = error.failed_result().expect("mutation result is retained");
        assert_eq!(result.action, GitWorkflowActionKind::SwitchBranch);
        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.stdout, b"stdout");
        assert_eq!(result.stderr, b"stderr");
    }

    #[tokio::test]
    async fn failed_read_action_returns_unsuccessful_structured_result() {
        let environment = MockEnvironment::with_result(exec_result(false));
        let request = workflow(GitWorkflowAction::Compare(CompareRequest {
            base: "main".to_string(),
            head: "feature/safe".to_string(),
            mode: CompareMode::Direct,
        }));

        let result = execute_git_workflow(&environment, &request, ExecutionContext::default())
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.exit_code, Some(1));
        assert_eq!(result.action, GitWorkflowActionKind::Compare);
    }

    #[tokio::test]
    async fn environment_error_is_not_converted_to_command_success() {
        let mut environment = MockEnvironment::with_result(exec_result(true));
        environment.execution_error = Some("cancelled".to_string());
        let request = workflow(GitWorkflowAction::Status(GitStatusRequest::default()));

        let error = execute_git_workflow(&environment, &request, ExecutionContext::default())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            GitWorkflowError::Execution {
                action: GitWorkflowActionKind::Status,
                ..
            }
        ));
    }
}
