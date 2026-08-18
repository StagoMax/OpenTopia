use anyhow::Context;
use opentopia_core::{
    TurnChangeSet, TurnFileChange, TurnFileChangeKind, GIT_NONINTERACTIVE_ENVIRONMENT,
};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::process::Output;
use tokio::process::Command;
use uuid::Uuid;

const GIT_FAILURE_DETAIL_CHARS: usize = 600;

#[derive(Debug, Clone)]
pub(super) struct RepoContext {
    pub(super) workspace_root: PathBuf,
    pub(super) repo_root: PathBuf,
    pub(super) workspace_prefix: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeEntry {
    pub(super) mode: String,
    pub(super) oid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IgnoredPathMode {
    Skip,
    Include,
}

pub(super) async fn discover_repo(workspace_root: &Path) -> anyhow::Result<RepoContext> {
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

pub(super) fn repo_from_change_set(change_set: &TurnChangeSet) -> anyhow::Result<RepoContext> {
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

pub(super) fn normalize_recorded_change_path(
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

pub(super) fn normalized_changed_paths(
    change_set: &TurnChangeSet,
    reported: &[PathBuf],
) -> Vec<PathBuf> {
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

pub(super) fn change_set_workspace_paths(change_set: &TurnChangeSet) -> Vec<PathBuf> {
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

pub(super) fn change_set_mutation_paths(change_set: &TurnChangeSet) -> Vec<PathBuf> {
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

pub(super) fn change_is_external(change: &TurnFileChange) -> bool {
    change
        .old_path
        .iter()
        .chain(change.new_path.iter())
        .any(|path| path.is_absolute())
}

pub(super) fn same_change_path(left: &Path, right: &Path) -> bool {
    comparable_path_text(&normalized_path_text(left))
        == comparable_path_text(&normalized_path_text(right))
}

pub(super) fn normalized_path_text(path: &Path) -> String {
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

pub(super) async fn capture_tree_for_paths(
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

pub(super) async fn git_path_is_ignored(repo_root: &Path, path: &str) -> anyhow::Result<bool> {
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

pub(super) async fn diff_trees(
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

pub(super) async fn file_stats(
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

pub(super) async fn tree_entry(
    repo_root: &Path,
    tree: &str,
    path: &str,
) -> anyhow::Result<Option<TreeEntry>> {
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

pub(super) async fn tree_entries(
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

pub(super) async fn read_blob(repo_root: &Path, path: &str, oid: &str) -> anyhow::Result<Vec<u8>> {
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

pub(super) fn safe_workspace_path(
    workspace_root: &Path,
    relative: &Path,
) -> anyhow::Result<PathBuf> {
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

pub(super) fn validate_workspace_relative_path(relative: &Path) -> anyhow::Result<()> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        anyhow::bail!("invalid workspace-relative path: {}", relative.display());
    }
    Ok(())
}

pub(super) fn validate_external_absolute_path(path: &Path) -> anyhow::Result<()> {
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

pub(super) async fn run_git_strings(
    repo_root: &Path,
    args: &[String],
    index: Option<&Path>,
) -> anyhow::Result<Output> {
    let output = git_output_strings(repo_root, args, index).await?;
    ensure_git_success(&output, &format!("git {}", args.join(" ")))?;
    Ok(output)
}

pub(super) async fn git_output(
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

pub(super) async fn git_output_strings(
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

pub(super) fn ensure_git_success(output: &Output, action: &str) -> anyhow::Result<()> {
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

pub(super) fn repo_path(repo: &RepoContext, workspace_relative: &Path) -> String {
    let prefix = git_path(&repo.workspace_prefix);
    let relative = git_path(workspace_relative);
    if prefix == "." || prefix.is_empty() {
        relative
    } else {
        format!("{prefix}/{relative}")
    }
}

pub(super) fn workspace_relative_path(
    repo: &RepoContext,
    repo_path: &str,
) -> anyhow::Result<PathBuf> {
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

pub(super) fn turn_snapshot_ref(turn_id: Uuid, phase: &str) -> String {
    format!("refs/opentopia/turns/{turn_id}/{phase}")
}

pub(super) fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{git_failure_detail, GIT_FAILURE_DETAIL_CHARS};

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
}
