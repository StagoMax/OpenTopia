use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: Uuid,
    pub title: String,
    pub workspace_root: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Thread {
    pub fn new(title: impl Into<String>, workspace_root: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            workspace_root,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "system" => Ok(Self::System),
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            "tool" => Ok(Self::Tool),
            other => anyhow::bail!("unknown message role: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub role: MessageRole,
    pub parts: Vec<MessagePart>,
    pub created_at: DateTime<Utc>,
}

impl Message {
    pub fn text(thread_id: Uuid, role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            role,
            parts: vec![MessagePart::Text { text: text.into() }],
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    Text { text: String },
    ToolCall { call: ToolCall },
    ToolResult { result: ToolResult },
    FileRef { path: PathBuf },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: Uuid,
    pub name: String,
    pub input: Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, input: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            input,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: Uuid,
    pub output: String,
    /// Tool-specific metadata is also the forward-compatible place for context
    /// and artifact hints, such as truncated/originalBytes/maxResults.
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub kind: String,
    pub content_type: String,
    pub storage: ArtifactStorage,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
    pub metadata: Value,
}

impl Artifact {
    pub fn inline(
        thread_id: Uuid,
        kind: impl Into<String>,
        content_type: impl Into<String>,
        content: impl Into<String>,
        metadata: Value,
    ) -> Self {
        let content = content.into();
        Self {
            id: Uuid::new_v4(),
            thread_id,
            kind: kind.into(),
            content_type: content_type.into(),
            bytes: content.len() as u64,
            storage: ArtifactStorage::Inline { content },
            created_at: Utc::now(),
            metadata,
        }
    }

    pub fn path(
        thread_id: Uuid,
        kind: impl Into<String>,
        content_type: impl Into<String>,
        path: PathBuf,
        bytes: u64,
        metadata: Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            kind: kind.into(),
            content_type: content_type.into(),
            storage: ArtifactStorage::Path { path },
            bytes,
            created_at: Utc::now(),
            metadata,
        }
    }

    pub fn metadata(&self) -> ArtifactMetadata {
        ArtifactMetadata {
            id: self.id,
            thread_id: self.thread_id,
            kind: self.kind.clone(),
            content_type: self.content_type.clone(),
            storage: self.storage.metadata(),
            bytes: self.bytes,
            created_at: self.created_at,
            metadata: self.metadata.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArtifactStorage {
    Inline { content: String },
    Path { path: PathBuf },
}

impl ArtifactStorage {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Inline { .. } => "inline",
            Self::Path { .. } => "path",
        }
    }

    pub fn metadata(&self) -> ArtifactStorageMetadata {
        match self {
            Self::Inline { .. } => ArtifactStorageMetadata::Inline,
            Self::Path { path } => ArtifactStorageMetadata::Path { path: path.clone() },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMetadata {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub kind: String,
    pub content_type: String,
    pub storage: ArtifactStorageMetadata,
    pub bytes: u64,
    pub created_at: DateTime<Utc>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArtifactStorageMetadata {
    Inline,
    Path { path: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSummary {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub covered_through_seq: i64,
    pub message_count: usize,
    pub summary: String,
    pub token_estimate: Option<usize>,
    pub created_at: DateTime<Utc>,
    pub metadata: Value,
}

impl ContextSummary {
    pub fn new(
        thread_id: Uuid,
        covered_through_seq: i64,
        message_count: usize,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            covered_through_seq,
            message_count,
            summary: summary.into(),
            token_estimate: None,
            created_at: Utc::now(),
            metadata: Value::Null,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
}

impl ApprovalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "denied" => Ok(Self::Denied),
            other => anyhow::bail!("unknown approval status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub approval_id: Uuid,
    pub thread_id: Uuid,
    pub action: String,
    pub reason: String,
    pub status: ApprovalStatus,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

impl Approval {
    pub fn pending(
        approval_id: Uuid,
        thread_id: Uuid,
        action: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            approval_id,
            thread_id,
            action: action.into(),
            reason: reason.into(),
            status: ApprovalStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCommandStatus {
    Finished,
    Failed,
    Cancelled,
    TimedOut,
    Error,
}

impl TerminalCommandStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Error => "error",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "finished" => Ok(Self::Finished),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "timed_out" => Ok(Self::TimedOut),
            "error" => Ok(Self::Error),
            other => anyhow::bail!("unknown terminal command status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCommandHistory {
    pub command_id: Uuid,
    pub thread_id: Uuid,
    pub seq_start: u64,
    pub seq_end: u64,
    pub command: String,
    pub cwd: Option<PathBuf>,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub status: TerminalCommandStatus,
    pub message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub turn_id: Option<Uuid>,
    pub seq: i64,
    pub created_at: DateTime<Utc>,
    pub payload: AgentEventPayload,
}

impl AgentEvent {
    pub fn new(
        thread_id: Uuid,
        turn_id: Option<Uuid>,
        seq: i64,
        payload: AgentEventPayload,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            turn_id,
            seq,
            created_at: Utc::now(),
            payload,
        }
    }

    pub fn kind(&self) -> &'static str {
        self.payload.kind()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEventPayload {
    TurnStarted {
        user_message_id: Uuid,
    },
    ModelDelta {
        text: String,
    },
    ToolCallStarted {
        call: ToolCall,
    },
    ToolCallFinished {
        result: ToolResult,
    },
    AssistantMessage {
        message: Message,
    },
    FileChanged {
        path: PathBuf,
        summary: String,
    },
    ApprovalRequested {
        approval_id: Uuid,
        reason: String,
        action: String,
    },
    ContextCompacted {
        summary: ContextSummary,
    },
    TokenUsage {
        input_tokens: usize,
        output_tokens: usize,
        total_tokens: usize,
    },
    TurnFinished {
        summary: String,
    },
    TurnSuspended {
        approval_id: Uuid,
        reason: String,
    },
    TurnCancelled {
        reason: String,
    },
    Error {
        message: String,
    },
}

impl AgentEventPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TurnStarted { .. } => "turn_started",
            Self::ModelDelta { .. } => "model_delta",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallFinished { .. } => "tool_call_finished",
            Self::AssistantMessage { .. } => "assistant_message",
            Self::FileChanged { .. } => "file_changed",
            Self::ApprovalRequested { .. } => "approval_requested",
            Self::ContextCompacted { .. } => "context_compacted",
            Self::TokenUsage { .. } => "token_usage",
            Self::TurnFinished { .. } => "turn_finished",
            Self::TurnSuspended { .. } => "turn_suspended",
            Self::TurnCancelled { .. } => "turn_cancelled",
            Self::Error { .. } => "error",
        }
    }
}
