use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::{self, Write as _};
use thiserror::Error;
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            Serialize,
            Deserialize,
            JsonSchema,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.as_uuid()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

uuid_id!(CollaborationSessionId);
uuid_id!(AgentThreadId);
uuid_id!(AgentTurnId);
uuid_id!(RuntimeSnapshotId);
uuid_id!(AgentMailboxMessageId);

pub const ROOT_AGENT_PATH: &str = "/root";
pub const MAX_AGENT_TASK_NAME_CHARS: usize = 64;

#[derive(
    Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(transparent)]
pub struct AgentPath(String);

impl AgentPath {
    pub fn root() -> Self {
        Self(ROOT_AGENT_PATH.to_string())
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, CollaborationDomainError> {
        let value = value.as_ref();
        if value == ROOT_AGENT_PATH {
            return Ok(Self::root());
        }
        let Some(suffix) = value.strip_prefix("/root/") else {
            return Err(CollaborationDomainError::InvalidAgentPath(
                value.to_string(),
            ));
        };
        if suffix.is_empty() || suffix.split('/').any(|segment| !valid_task_name(segment)) {
            return Err(CollaborationDomainError::InvalidAgentPath(
                value.to_string(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn child(&self, task_name: &str) -> Result<Self, CollaborationDomainError> {
        validate_task_name(task_name)?;
        Self::parse(format!("{}/{task_name}", self.0))
    }

    pub fn parent(&self) -> Option<Self> {
        if self.0 == ROOT_AGENT_PATH {
            return None;
        }
        self.0
            .rsplit_once('/')
            .map(|(parent, _)| Self(parent.to_string()))
    }

    pub fn depth(&self) -> u16 {
        self.0
            .split('/')
            .filter(|segment| !segment.is_empty())
            .count() as u16
            - 1
    }

    pub fn is_descendant_of(&self, ancestor: &Self) -> bool {
        self != ancestor
            && self
                .0
                .strip_prefix(ancestor.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AsRef<str> for AgentPath {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

pub(crate) fn validate_task_name(value: &str) -> Result<(), CollaborationDomainError> {
    if valid_task_name(value) {
        Ok(())
    } else {
        Err(CollaborationDomainError::InvalidTaskName(value.to_string()))
    }
}

fn valid_task_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_AGENT_TASK_NAME_CHARS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationSessionPolicy {
    pub max_agents: usize,
    pub max_active_runs: usize,
    pub max_depth: u16,
}

impl Default for CollaborationSessionPolicy {
    fn default() -> Self {
        Self {
            max_agents: 16,
            max_active_runs: 6,
            max_depth: 1,
        }
    }
}

impl CollaborationSessionPolicy {
    pub fn validate(&self) -> Result<(), CollaborationDomainError> {
        if self.max_agents == 0 {
            return Err(CollaborationDomainError::InvalidSessionPolicy(
                "max_agents must be greater than zero".to_string(),
            ));
        }
        if self.max_active_runs == 0 {
            return Err(CollaborationDomainError::InvalidSessionPolicy(
                "max_active_runs must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSpawnPolicy {
    pub allow_child_spawns: bool,
    pub max_depth: u16,
    pub max_direct_children: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshotSeed {
    pub id: RuntimeSnapshotId,
    pub parent_snapshot_id: Option<RuntimeSnapshotId>,
    pub content_hash: String,
    pub snapshot: Value,
}

impl RuntimeSnapshotSeed {
    pub fn new(parent_snapshot_id: Option<RuntimeSnapshotId>, snapshot: Value) -> Self {
        let content_hash = snapshot_content_hash(&snapshot);
        Self {
            id: RuntimeSnapshotId::new(),
            parent_snapshot_id,
            content_hash,
            snapshot,
        }
    }

    pub fn validate(&self) -> Result<(), CollaborationDomainError> {
        let actual = snapshot_content_hash(&self.snapshot);
        if actual != self.content_hash {
            return Err(CollaborationDomainError::InvalidRuntimeSnapshot(
                "content hash does not match snapshot payload".to_string(),
            ));
        }
        super::RuntimeSnapshotV1::decode(&self.snapshot)?;
        Ok(())
    }
}

fn snapshot_content_hash(snapshot: &Value) -> String {
    let encoded = serde_json::to_vec(snapshot).expect("serde_json::Value always serializes");
    let digest = Sha256::digest(encoded);
    let mut content_hash = String::with_capacity("sha256:".len() + digest.len() * 2);
    content_hash.push_str("sha256:");
    for byte in digest {
        write!(&mut content_hash, "{byte:02x}").expect("writing to a String cannot fail");
    }
    content_hash
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeSnapshotRecord {
    pub id: RuntimeSnapshotId,
    pub session_id: CollaborationSessionId,
    pub parent_snapshot_id: Option<RuntimeSnapshotId>,
    pub content_hash: String,
    pub snapshot: Value,
    pub created_at: DateTime<Utc>,
}

impl AgentRuntimeSnapshotRecord {
    pub fn decode(&self) -> Result<super::RuntimeSnapshotV1, CollaborationDomainError> {
        super::RuntimeSnapshotV1::decode(&self.snapshot)
    }
}

impl AgentSpawnPolicy {
    pub fn disabled(max_depth: u16) -> Self {
        Self {
            allow_child_spawns: false,
            max_depth,
            max_direct_children: 0,
        }
    }

    pub fn allows_children(max_depth: u16, max_direct_children: usize) -> Self {
        Self {
            allow_child_spawns: true,
            max_depth,
            max_direct_children,
        }
    }

    pub fn is_attenuation_of(&self, parent: &Self) -> bool {
        (!self.allow_child_spawns || parent.allow_child_spawns)
            && self.max_depth <= parent.max_depth
            && self.max_direct_children <= parent.max_direct_children
    }
}

impl Default for AgentSpawnPolicy {
    fn default() -> Self {
        Self::disabled(1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationSessionRecord {
    pub id: CollaborationSessionId,
    pub user_task_id: Uuid,
    pub policy: CollaborationSessionPolicy,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentThreadRecord {
    pub id: AgentThreadId,
    pub session_id: CollaborationSessionId,
    pub parent_agent_thread_id: Option<AgentThreadId>,
    pub path: AgentPath,
    pub task_name: String,
    pub agent_type: String,
    pub runtime_snapshot_id: RuntimeSnapshotId,
    pub spawn_policy: AgentSpawnPolicy,
    pub created_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
}

impl AgentThreadRecord {
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnStatus {
    Queued,
    Running,
    WaitingApproval,
    WaitingInput,
    WaitingAction,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl AgentTurnStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::WaitingInput => "waiting_input",
            Self::WaitingAction => "waiting_action",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CollaborationDomainError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "waiting_approval" => Ok(Self::WaitingApproval),
            "waiting_input" => Ok(Self::WaitingInput),
            "waiting_action" => Ok(Self::WaitingAction),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(CollaborationDomainError::Persistence(format!(
                "unknown agent turn status `{other}`"
            ))),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }

    pub fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::WaitingApproval | Self::WaitingInput | Self::WaitingAction
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        use AgentTurnStatus::*;
        matches!(
            (self, next),
            (Queued, Running | Cancelled | Interrupted)
                | (Running, WaitingApproval | WaitingInput | WaitingAction)
                | (Running, Completed | Failed | Cancelled | Interrupted)
                | (
                    WaitingApproval | WaitingInput | WaitingAction,
                    Running | Cancelled | Interrupted
                )
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnRecord {
    pub id: AgentTurnId,
    pub session_id: CollaborationSessionId,
    pub agent_thread_id: AgentThreadId,
    pub requested_by_agent_thread_id: Option<AgentThreadId>,
    pub requested_by_turn_id: Option<AgentTurnId>,
    pub sequence: u64,
    pub task_message: String,
    pub status: AgentTurnStatus,
    pub invocation_id: u64,
    pub outcome_ref: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl AgentTurnRecord {
    pub fn transition(
        &mut self,
        next: AgentTurnStatus,
        now: DateTime<Utc>,
    ) -> Result<(), CollaborationDomainError> {
        if self.status == next {
            return Ok(());
        }
        if !self.status.can_transition_to(next) {
            return Err(CollaborationDomainError::InvalidTurnTransition {
                from: self.status,
                to: next,
            });
        }
        if self.status == AgentTurnStatus::Queued && next == AgentTurnStatus::Running {
            self.started_at.get_or_insert(now);
        }
        if next.is_terminal() {
            self.completed_at = Some(now);
        }
        self.status = next;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentAvailability {
    Idle,
    Queued,
    Running,
    NeedsAttention,
    Archived,
}

impl AgentAvailability {
    pub fn derive(thread: &AgentThreadRecord, latest_turn: Option<&AgentTurnRecord>) -> Self {
        if thread.is_archived() {
            return Self::Archived;
        }
        match latest_turn.map(|turn| turn.status) {
            None => Self::Idle,
            Some(AgentTurnStatus::Queued) => Self::Queued,
            Some(AgentTurnStatus::Running) => Self::Running,
            Some(status) if status.needs_attention() => Self::NeedsAttention,
            Some(_) => Self::Idle,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CollaborationDomainError {
    #[error("invalid agent task name `{0}`; use lowercase letters, digits, and underscores")]
    InvalidTaskName(String),
    #[error("invalid canonical agent path `{0}`")]
    InvalidAgentPath(String),
    #[error("invalid collaboration session policy: {0}")]
    InvalidSessionPolicy(String),
    #[error("agent type cannot be empty")]
    EmptyAgentType,
    #[error("agent task message cannot be empty")]
    EmptyTaskMessage,
    #[error("invalid runtime snapshot: {0}")]
    InvalidRuntimeSnapshot(String),
    #[error("collaboration session not found: {0}")]
    SessionNotFound(CollaborationSessionId),
    #[error("agent thread not found: {0}")]
    AgentThreadNotFound(AgentThreadId),
    #[error("agent turn not found: {0}")]
    AgentTurnNotFound(AgentTurnId),
    #[error("agent path already exists in the session: {0}")]
    DuplicateAgentPath(AgentPath),
    #[error("agent thread is archived: {0}")]
    AgentThreadArchived(AgentThreadId),
    #[error("agent thread already has a non-terminal turn: {0}")]
    AgentTurnAlreadyActive(AgentThreadId),
    #[error("agent spawn is disabled for {0}")]
    SpawnDisabled(AgentPath),
    #[error("agent depth {actual} exceeds maximum {maximum}")]
    MaximumDepth { actual: u16, maximum: u16 },
    #[error("direct child limit reached for {path} (maximum {maximum})")]
    MaximumDirectChildren { path: AgentPath, maximum: usize },
    #[error("session agent limit reached (maximum {maximum})")]
    MaximumAgents { maximum: usize },
    #[error("child spawn policy would expand the parent capability")]
    SpawnPolicyEscalation,
    #[error("requesting turn {turn_id} does not belong to agent {agent_thread_id}")]
    RequestingTurnOwnership {
        turn_id: AgentTurnId,
        agent_thread_id: AgentThreadId,
    },
    #[error("agent {caller} cannot manage lifecycle of {target}")]
    LifecyclePermissionDenied {
        caller: AgentPath,
        target: AgentPath,
    },
    #[error("invalid agent turn transition from {from:?} to {to:?}")]
    InvalidTurnTransition {
        from: AgentTurnStatus,
        to: AgentTurnStatus,
    },
    #[error("collaboration registry lock is poisoned")]
    RegistryPoisoned,
    #[error("collaboration persistence failed: {0}")]
    Persistence(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_paths_support_recursive_children() {
        let root = AgentPath::root();
        let child = root.child("research").unwrap();
        let grandchild = child.child("reviewer_2").unwrap();

        assert_eq!(root.depth(), 0);
        assert_eq!(child.depth(), 1);
        assert_eq!(grandchild.depth(), 2);
        assert_eq!(grandchild.parent(), Some(child.clone()));
        assert!(grandchild.is_descendant_of(&root));
        assert!(!root.is_descendant_of(&child));
    }

    #[test]
    fn invalid_paths_and_task_names_are_rejected() {
        for invalid in ["Research", "two-agents", "", "agent/path"] {
            assert!(AgentPath::root().child(invalid).is_err(), "{invalid}");
        }
        assert!(AgentPath::parse("/other/research").is_err());
        assert!(AgentPath::parse("/root/research/").is_err());
    }

    #[test]
    fn terminal_turn_cannot_be_reopened() {
        assert!(AgentTurnStatus::Running.can_transition_to(AgentTurnStatus::Completed));
        assert!(!AgentTurnStatus::Completed.can_transition_to(AgentTurnStatus::Running));
        assert!(AgentTurnStatus::WaitingInput.can_transition_to(AgentTurnStatus::Running));
    }

    #[test]
    fn child_spawn_policy_must_only_narrow_capability() {
        let parent = AgentSpawnPolicy::allows_children(3, 4);
        assert!(AgentSpawnPolicy::allows_children(2, 2).is_attenuation_of(&parent));
        assert!(AgentSpawnPolicy::disabled(3).is_attenuation_of(&parent));
        assert!(!AgentSpawnPolicy::allows_children(4, 2).is_attenuation_of(&parent));
        assert!(!AgentSpawnPolicy::allows_children(2, 5).is_attenuation_of(&parent));
    }
}
