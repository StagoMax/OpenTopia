use crate::capabilities::{ContributionKind, PluginContribution};
use crate::local_git::{LocalGitRemote, NormalizedGitRemoteUrl, LOCAL_GIT_V1_API_VERSION};
use crate::store::SqliteSessionStore;
use anyhow::bail;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const SCM_CONNECTOR_HOST_API_VERSION: &str = "scmConnectorHost.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmConnectorDescriptor {
    pub plugin_id: String,
    pub connector_id: String,
    pub display_name: String,
    #[serde(default)]
    pub capabilities: BTreeSet<ScmConnectorCapability>,
    #[serde(default)]
    pub remote_matchers: Vec<ScmRemoteUrlMatcher>,
}

impl ScmConnectorDescriptor {
    pub fn from_contribution(contribution: &PluginContribution) -> anyhow::Result<Self> {
        if contribution.kind != ContributionKind::ScmConnector {
            bail!("contribution {} is not an scm_connector", contribution.id);
        }
        let mut declaration = contribution
            .declaration
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("SCM connector declaration must be an object"))?;
        declaration.insert(
            "pluginId".to_string(),
            Value::String(contribution.plugin_id.clone()),
        );
        declaration.insert(
            "connectorId".to_string(),
            Value::String(contribution.local_id.clone()),
        );
        if !declaration.contains_key("displayName") {
            declaration.insert(
                "displayName".to_string(),
                Value::String(contribution.local_id.clone()),
            );
        }
        serde_json::from_value(Value::Object(declaration))
            .map_err(|error| anyhow::anyhow!("invalid SCM connector {}: {error}", contribution.id))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ScmConnectorCapability {
    ChangeRequests,
    Issues,
    Automation,
    Reviews,
    Releases,
    RepositoryIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmRemoteUrlMatcher {
    pub matcher_id: String,
    #[serde(default)]
    pub schemes: BTreeSet<String>,
    pub host: ScmHostMatcher,
    pub path: ScmPathMatcher,
}

impl ScmRemoteUrlMatcher {
    pub fn match_specificity(
        &self,
        remote: &NormalizedGitRemoteUrl,
    ) -> Option<ScmMatcherSpecificity> {
        let scheme_specificity = if self.schemes.is_empty() {
            0
        } else if remote.scheme.as_ref().is_some_and(|scheme| {
            self.schemes
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(scheme))
        }) {
            1
        } else {
            return None;
        };

        let host = remote.host.as_deref()?;
        let host_specificity = self.host.match_specificity(host)?;
        let path_specificity = self.path.match_specificity(&remote.repository_path)?;
        Some(ScmMatcherSpecificity {
            host: host_specificity,
            path: path_specificity,
            scheme: scheme_specificity,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ScmHostMatcher {
    Exact(String),
    Suffix(String),
    Any,
}

impl ScmHostMatcher {
    fn match_specificity(&self, host: &str) -> Option<u8> {
        match self {
            Self::Exact(expected) if expected.eq_ignore_ascii_case(host) => Some(2),
            Self::Suffix(suffix) if host_matches_suffix(host, suffix) => Some(1),
            Self::Any => Some(0),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ScmPathMatcher {
    Exact(String),
    Prefix(String),
    Any,
}

impl ScmPathMatcher {
    fn match_specificity(&self, path: &str) -> Option<u8> {
        let path = canonical_match_path(path);
        match self {
            Self::Exact(expected) if canonical_match_path(expected) == path => Some(2),
            Self::Prefix(prefix) if path_matches_prefix(&path, &canonical_match_path(prefix)) => {
                Some(1)
            }
            Self::Any => Some(0),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct ScmMatcherSpecificity {
    pub host: u8,
    pub path: u8,
    pub scheme: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmRemoteBinding {
    pub workspace_key: String,
    pub remote_name: String,
    pub connector_plugin_id: String,
    pub connector_id: String,
    pub account_binding_id: Option<String>,
}

pub(crate) fn migrate_scm_remote_bindings(conn: &mut Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS scm_remote_bindings (
            workspace_key TEXT NOT NULL,
            remote_name TEXT NOT NULL,
            connector_plugin_id TEXT NOT NULL,
            connector_id TEXT NOT NULL,
            account_binding_id TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (workspace_key, remote_name)
        );
        CREATE INDEX IF NOT EXISTS idx_scm_remote_bindings_connector
            ON scm_remote_bindings(connector_plugin_id, connector_id);
        "#,
    )?;
    Ok(())
}

impl SqliteSessionStore {
    pub fn put_scm_remote_binding(
        &self,
        binding: &ScmRemoteBinding,
    ) -> anyhow::Result<ScmRemoteBinding> {
        validate_binding(binding)?;
        self.with_connection(|conn| {
            conn.execute(
                r#"
                INSERT INTO scm_remote_bindings (
                    workspace_key, remote_name, connector_plugin_id, connector_id,
                    account_binding_id, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(workspace_key, remote_name) DO UPDATE SET
                    connector_plugin_id = excluded.connector_plugin_id,
                    connector_id = excluded.connector_id,
                    account_binding_id = excluded.account_binding_id,
                    updated_at = excluded.updated_at
                "#,
                params![
                    binding.workspace_key,
                    binding.remote_name,
                    binding.connector_plugin_id,
                    binding.connector_id,
                    binding.account_binding_id,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )?;
            Ok(binding.clone())
        })
    }

    pub fn get_scm_remote_binding(
        &self,
        workspace_key: &str,
        remote_name: &str,
    ) -> anyhow::Result<Option<ScmRemoteBinding>> {
        self.with_connection(|conn| {
            conn.query_row(
                r#"
                SELECT connector_plugin_id, connector_id, account_binding_id
                FROM scm_remote_bindings
                WHERE workspace_key = ?1 AND remote_name = ?2
                "#,
                params![workspace_key, remote_name],
                |row| {
                    Ok(ScmRemoteBinding {
                        workspace_key: workspace_key.to_string(),
                        remote_name: remote_name.to_string(),
                        connector_plugin_id: row.get(0)?,
                        connector_id: row.get(1)?,
                        account_binding_id: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
        })
    }

    pub fn delete_scm_remote_binding(
        &self,
        workspace_key: &str,
        remote_name: &str,
    ) -> anyhow::Result<bool> {
        self.with_connection(|conn| {
            Ok(conn.execute(
                "DELETE FROM scm_remote_bindings WHERE workspace_key = ?1 AND remote_name = ?2",
                params![workspace_key, remote_name],
            )? > 0)
        })
    }

    pub fn list_scm_remote_bindings(
        &self,
        workspace_key: &str,
    ) -> anyhow::Result<Vec<ScmRemoteBinding>> {
        self.with_connection(|conn| {
            let mut statement = conn.prepare(
                r#"
                SELECT remote_name, connector_plugin_id, connector_id, account_binding_id
                FROM scm_remote_bindings
                WHERE workspace_key = ?1
                ORDER BY remote_name
                "#,
            )?;
            let rows = statement.query_map(params![workspace_key], |row| {
                Ok(ScmRemoteBinding {
                    workspace_key: workspace_key.to_string(),
                    remote_name: row.get(0)?,
                    connector_plugin_id: row.get(1)?,
                    connector_id: row.get(2)?,
                    account_binding_id: row.get(3)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }
}

fn validate_binding(binding: &ScmRemoteBinding) -> anyhow::Result<()> {
    for (name, value) in [
        ("workspace_key", binding.workspace_key.as_str()),
        ("remote_name", binding.remote_name.as_str()),
        ("connector_plugin_id", binding.connector_plugin_id.as_str()),
        ("connector_id", binding.connector_id.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("SCM binding {name} cannot be empty");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmConnectorCandidate {
    pub plugin_id: String,
    pub connector_id: String,
    pub matcher_id: String,
    pub specificity: ScmMatcherSpecificity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScmSelectionSource {
    BestMatch,
    RemoteBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScmConnectorSelection {
    Unmatched,
    Selected {
        candidate: ScmConnectorCandidate,
        source: ScmSelectionSource,
        account_binding_id: Option<String>,
    },
    Conflict {
        candidates: Vec<ScmConnectorCandidate>,
        binding_issue: Option<ScmBindingIssue>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScmBindingIssue {
    WrongWorkspaceOrRemote,
    ConnectorUnavailable,
    ConnectorNotBestMatch,
}

pub fn select_scm_connector(
    workspace_key: &str,
    remote: &LocalGitRemote,
    connectors: &[ScmConnectorDescriptor],
    binding: Option<&ScmRemoteBinding>,
) -> ScmConnectorSelection {
    let mut by_connector = BTreeMap::<(String, String), ScmConnectorCandidate>::new();
    for connector in connectors {
        for matcher in &connector.remote_matchers {
            for url in remote.fetch_urls.iter().chain(&remote.push_urls) {
                let Some(specificity) = matcher.match_specificity(url) else {
                    continue;
                };
                let candidate = ScmConnectorCandidate {
                    plugin_id: connector.plugin_id.clone(),
                    connector_id: connector.connector_id.clone(),
                    matcher_id: matcher.matcher_id.clone(),
                    specificity,
                };
                let entry = by_connector
                    .entry((connector.plugin_id.clone(), connector.connector_id.clone()))
                    .or_insert_with(|| candidate.clone());
                if candidate.specificity > entry.specificity {
                    *entry = candidate;
                }
            }
        }
    }

    let Some(best_specificity) = by_connector
        .values()
        .map(|candidate| candidate.specificity)
        .max()
    else {
        return ScmConnectorSelection::Unmatched;
    };
    let mut candidates = by_connector
        .into_values()
        .filter(|candidate| candidate.specificity == best_specificity)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        (&left.plugin_id, &left.connector_id).cmp(&(&right.plugin_id, &right.connector_id))
    });

    if candidates.len() == 1 {
        let candidate = candidates.remove(0);
        let account_binding_id = binding
            .filter(|binding| binding_matches_remote(binding, workspace_key, remote))
            .filter(|binding| binding_matches_candidate(binding, &candidate))
            .and_then(|binding| binding.account_binding_id.clone());
        return ScmConnectorSelection::Selected {
            candidate,
            source: ScmSelectionSource::BestMatch,
            account_binding_id,
        };
    }

    if let Some(binding) = binding {
        if !binding_matches_remote(binding, workspace_key, remote) {
            return ScmConnectorSelection::Conflict {
                candidates,
                binding_issue: Some(ScmBindingIssue::WrongWorkspaceOrRemote),
            };
        }
        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| binding_matches_candidate(binding, candidate))
            .cloned()
        {
            return ScmConnectorSelection::Selected {
                candidate,
                source: ScmSelectionSource::RemoteBinding,
                account_binding_id: binding.account_binding_id.clone(),
            };
        }
        let issue = if connectors.iter().any(|connector| {
            connector.plugin_id == binding.connector_plugin_id
                && connector.connector_id == binding.connector_id
        }) {
            ScmBindingIssue::ConnectorNotBestMatch
        } else {
            ScmBindingIssue::ConnectorUnavailable
        };
        return ScmConnectorSelection::Conflict {
            candidates,
            binding_issue: Some(issue),
        };
    }

    ScmConnectorSelection::Conflict {
        candidates,
        binding_issue: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmConnectorHostHandles {
    pub api_version: String,
    pub local_git_read: LocalGitV1ReadHandle,
    pub local_git_mutation: Option<LocalGitV1MutationHandle>,
}

impl ScmConnectorHostHandles {
    pub fn read_only(repository: PathBuf) -> Self {
        Self {
            api_version: SCM_CONNECTOR_HOST_API_VERSION.to_string(),
            local_git_read: LocalGitV1ReadHandle::new(repository),
            local_git_mutation: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitV1ReadHandle {
    pub api_version: String,
    pub repository: PathBuf,
}

impl LocalGitV1ReadHandle {
    pub fn new(repository: PathBuf) -> Self {
        Self {
            api_version: LOCAL_GIT_V1_API_VERSION.to_string(),
            repository,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalGitV1MutationHandle {
    pub api_version: String,
    pub repository: PathBuf,
    pub grant_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmConnectorHostContext {
    pub workspace_key: String,
    pub remote: LocalGitRemote,
    pub binding: Option<ScmRemoteBinding>,
    pub handles: ScmConnectorHostHandles,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStageStatus {
    NotAttempted,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStage<T> {
    pub status: WorkflowStageStatus,
    pub receipt: Option<T>,
    pub error: Option<ScmWorkflowError>,
    pub event_id: Option<String>,
    pub approval_id: Option<String>,
}

impl<T> WorkflowStage<T> {
    pub fn not_attempted() -> Self {
        Self {
            status: WorkflowStageStatus::NotAttempted,
            receipt: None,
            error: None,
            event_id: None,
            approval_id: None,
        }
    }

    pub fn succeeded(receipt: T) -> Self {
        Self {
            status: WorkflowStageStatus::Succeeded,
            receipt: Some(receipt),
            error: None,
            event_id: None,
            approval_id: None,
        }
    }

    pub fn failed(error: ScmWorkflowError) -> Self {
        Self {
            status: WorkflowStageStatus::Failed,
            receipt: None,
            error: Some(error),
            event_id: None,
            approval_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmWorkflowError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalCommitReceipt {
    pub commit_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalPushReceipt {
    pub remote_name: String,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScmChangeRequestReceipt {
    pub connector_id: String,
    pub remote_id: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitPushChangeRequestResult {
    pub commit: WorkflowStage<LocalCommitReceipt>,
    pub push: WorkflowStage<LocalPushReceipt>,
    pub change_request: WorkflowStage<ScmChangeRequestReceipt>,
}

impl CommitPushChangeRequestResult {
    pub fn outcome(&self) -> CommitPushChangeRequestOutcome {
        if self.commit.status != WorkflowStageStatus::Succeeded {
            CommitPushChangeRequestOutcome::FailedBeforeCommit
        } else if self.push.status != WorkflowStageStatus::Succeeded {
            CommitPushChangeRequestOutcome::CommitCreated
        } else if self.change_request.status != WorkflowStageStatus::Succeeded {
            CommitPushChangeRequestOutcome::BranchPushed
        } else {
            CommitPushChangeRequestOutcome::Complete
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommitPushChangeRequestOutcome {
    FailedBeforeCommit,
    CommitCreated,
    BranchPushed,
    Complete,
}

fn binding_matches_remote(
    binding: &ScmRemoteBinding,
    workspace_key: &str,
    remote: &LocalGitRemote,
) -> bool {
    binding.workspace_key == workspace_key && binding.remote_name == remote.name
}

fn binding_matches_candidate(
    binding: &ScmRemoteBinding,
    candidate: &ScmConnectorCandidate,
) -> bool {
    binding.connector_plugin_id == candidate.plugin_id
        && binding.connector_id == candidate.connector_id
}

fn host_matches_suffix(host: &str, suffix: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let suffix = suffix
        .trim()
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .to_ascii_lowercase();
    host == suffix || host.ends_with(&format!(".{suffix}"))
}

fn canonical_match_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let path = path.trim_matches('/');
    path.strip_suffix(".git").unwrap_or(path).to_string()
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|remaining| remaining.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{ContributionOrigin, PluginPermission};
    use crate::local_git::normalize_git_remote_url;
    use serde_json::json;

    #[test]
    fn connector_descriptor_projects_plugin_owned_manifest_data() {
        let contribution = PluginContribution {
            id: "github/github".to_string(),
            plugin_id: "github".to_string(),
            local_id: "github".to_string(),
            kind: ContributionKind::ScmConnector,
            origin: ContributionOrigin::OpenTopia,
            api_version: "1".to_string(),
            required_host_capabilities: Vec::new(),
            permissions: Vec::<PluginPermission>::new(),
            configuration_schema: None,
            declaration: json!({
                "displayName": "GitHub",
                "capabilities": ["change_requests"],
                "remoteMatchers": [{
                    "matcherId": "github.com",
                    "schemes": ["https", "ssh"],
                    "host": {"type": "exact", "value": "github.com"},
                    "path": {"type": "any"}
                }]
            }),
        };
        let descriptor = ScmConnectorDescriptor::from_contribution(&contribution).unwrap();
        assert_eq!(descriptor.plugin_id, "github");
        assert_eq!(descriptor.connector_id, "github");
        assert_eq!(descriptor.display_name, "GitHub");
    }

    #[test]
    fn remote_bindings_persist_by_workspace_and_remote() {
        let store = SqliteSessionStore::open(":memory:").unwrap();
        let binding = ScmRemoteBinding {
            workspace_key: "c:/work/demo".to_string(),
            remote_name: "origin".to_string(),
            connector_plugin_id: "github".to_string(),
            connector_id: "github".to_string(),
            account_binding_id: Some("account-1".to_string()),
        };
        store.put_scm_remote_binding(&binding).unwrap();
        assert_eq!(
            store
                .get_scm_remote_binding("c:/work/demo", "origin")
                .unwrap(),
            Some(binding.clone())
        );
        assert_eq!(
            store.list_scm_remote_bindings("c:/work/demo").unwrap(),
            vec![binding]
        );
        assert!(store
            .delete_scm_remote_binding("c:/work/demo", "origin")
            .unwrap());
    }

    fn connector(
        connector_id: &str,
        host: ScmHostMatcher,
        path: ScmPathMatcher,
    ) -> ScmConnectorDescriptor {
        ScmConnectorDescriptor {
            plugin_id: format!("plugin.{connector_id}"),
            connector_id: connector_id.to_string(),
            display_name: connector_id.to_string(),
            capabilities: BTreeSet::from([ScmConnectorCapability::ChangeRequests]),
            remote_matchers: vec![ScmRemoteUrlMatcher {
                matcher_id: "primary".to_string(),
                schemes: BTreeSet::new(),
                host,
                path,
            }],
        }
    }

    fn remote(name: &str, url: &str) -> LocalGitRemote {
        LocalGitRemote {
            name: name.to_string(),
            fetch_urls: vec![normalize_git_remote_url(url)],
            push_urls: Vec::new(),
        }
    }

    #[test]
    fn exact_host_and_path_beat_wildcard_matchers() {
        let remote = remote("origin", "https://code.example.test/acme/project.git");
        let connectors = vec![
            connector("generic", ScmHostMatcher::Any, ScmPathMatcher::Any),
            connector(
                "exact",
                ScmHostMatcher::Exact("code.example.test".to_string()),
                ScmPathMatcher::Exact("acme/project".to_string()),
            ),
        ];

        let ScmConnectorSelection::Selected { candidate, .. } =
            select_scm_connector("workspace", &remote, &connectors, None)
        else {
            panic!("expected a selected connector");
        };
        assert_eq!(candidate.connector_id, "exact");
    }

    #[test]
    fn per_remote_binding_resolves_equal_best_matches() {
        let remote = remote("origin", "ssh://git@code.example.test/acme/project.git");
        let connectors = vec![
            connector(
                "first",
                ScmHostMatcher::Exact("code.example.test".to_string()),
                ScmPathMatcher::Any,
            ),
            connector(
                "second",
                ScmHostMatcher::Exact("code.example.test".to_string()),
                ScmPathMatcher::Any,
            ),
        ];
        let binding = ScmRemoteBinding {
            workspace_key: "workspace".to_string(),
            remote_name: "origin".to_string(),
            connector_plugin_id: "plugin.second".to_string(),
            connector_id: "second".to_string(),
            account_binding_id: Some("account-2".to_string()),
        };

        assert!(matches!(
            select_scm_connector("workspace", &remote, &connectors, Some(&binding)),
            ScmConnectorSelection::Selected {
                source: ScmSelectionSource::RemoteBinding,
                account_binding_id: Some(account),
                candidate: ScmConnectorCandidate { connector_id, .. },
            } if connector_id == "second" && account == "account-2"
        ));
    }

    #[test]
    fn equal_matches_without_binding_are_an_explicit_conflict() {
        let remote = remote("origin", "https://code.example.test/acme/project");
        let connectors = vec![
            connector("a", ScmHostMatcher::Any, ScmPathMatcher::Any),
            connector("b", ScmHostMatcher::Any, ScmPathMatcher::Any),
        ];

        assert!(matches!(
            select_scm_connector("workspace", &remote, &connectors, None),
            ScmConnectorSelection::Conflict {
                candidates,
                binding_issue: None,
            } if candidates.len() == 2
        ));
    }

    #[test]
    fn bindings_are_scoped_by_workspace_and_remote() {
        let remote = remote("mirror", "https://code.example.test/acme/project");
        let connectors = vec![
            connector("a", ScmHostMatcher::Any, ScmPathMatcher::Any),
            connector("b", ScmHostMatcher::Any, ScmPathMatcher::Any),
        ];
        let binding = ScmRemoteBinding {
            workspace_key: "other".to_string(),
            remote_name: "origin".to_string(),
            connector_plugin_id: "plugin.b".to_string(),
            connector_id: "b".to_string(),
            account_binding_id: None,
        };

        assert!(matches!(
            select_scm_connector("workspace", &remote, &connectors, Some(&binding)),
            ScmConnectorSelection::Conflict {
                binding_issue: Some(ScmBindingIssue::WrongWorkspaceOrRemote),
                ..
            }
        ));
    }

    #[test]
    fn local_git_remains_available_when_no_connector_matches() {
        let remote = remote("origin", "file:///C:/work/repo.git");
        let connectors = vec![connector(
            "hosted",
            ScmHostMatcher::Exact("code.example.test".to_string()),
            ScmPathMatcher::Any,
        )];

        assert_eq!(
            select_scm_connector("workspace", &remote, &connectors, None),
            ScmConnectorSelection::Unmatched
        );
    }

    #[test]
    fn composition_reports_push_success_when_remote_creation_fails() {
        let result = CommitPushChangeRequestResult {
            commit: WorkflowStage::succeeded(LocalCommitReceipt {
                commit_id: Some("abc123".to_string()),
            }),
            push: WorkflowStage::succeeded(LocalPushReceipt {
                remote_name: "origin".to_string(),
                branch: "feature".to_string(),
            }),
            change_request: WorkflowStage::failed(ScmWorkflowError {
                code: "remote_unavailable".to_string(),
                message: "remote API unavailable".to_string(),
                retryable: true,
            }),
        };

        assert_eq!(
            result.outcome(),
            CommitPushChangeRequestOutcome::BranchPushed
        );
        let json = serde_json::to_value(result).unwrap();
        assert_eq!(json["push"]["status"], "succeeded");
        assert_eq!(json["changeRequest"]["status"], "failed");
    }

    #[test]
    fn connector_handles_are_read_only_without_a_mutation_grant() {
        let handles = ScmConnectorHostHandles::read_only(PathBuf::from("C:/work/repo"));

        assert_eq!(handles.api_version, SCM_CONNECTOR_HOST_API_VERSION);
        assert_eq!(handles.local_git_read.api_version, LOCAL_GIT_V1_API_VERSION);
        assert!(handles.local_git_mutation.is_none());
    }
}
