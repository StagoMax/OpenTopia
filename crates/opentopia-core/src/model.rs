use crate::context_sources::{ContextSourceKind, LoadedContextSource};
use crate::guardian::{
    GuardianDecisionSource, GuardianReviewFailureKind, GuardianReviewStatus, GuardianRiskLevel,
    GuardianUserAuthorization,
};
use crate::model_context::{
    ModelContextItem, ThreadContextSnapshot, TokenEstimateBreakdown, TurnContextSnapshot,
};
use crate::provider::ModelUsage;
use crate::settings::ProviderAdapterKind;
use crate::skills::LoadedSkill;
use crate::subagents::SubagentRun;
use crate::work_form::{WorkForm, WorkFormStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub workspace_root: Option<PathBuf>,
    pub pinned: bool,
    pub sort_order: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Project {
    pub fn new(name: impl Into<String>, workspace_root: Option<PathBuf>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            workspace_root,
            pinned: false,
            sort_order: 0,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: Uuid,
    pub title: String,
    pub workspace_root: PathBuf,
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub experience_mode: ExperienceMode,
    /// Model chosen for this conversation. Pinned at creation so a catalog
    /// refresh never swaps the model mid-thread; `None` means "use the active
    /// connection's default", which keeps pre-existing threads working.
    #[serde(default)]
    pub model_selection: Option<ThreadModelSelection>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A concrete model to run a thread with. The connection supplies the endpoint
/// and credentials; this only narrows which model and how hard it thinks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadModelSelection {
    pub connection_id: String,
    pub model_id: String,
    #[serde(default)]
    pub adapter: Option<ProviderAdapterKind>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceMode {
    Work,
    #[default]
    Code,
    Flow,
}

impl ExperienceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Code => "code",
            Self::Flow => "flow",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "work" => Ok(Self::Work),
            "code" => Ok(Self::Code),
            "flow" => Ok(Self::Flow),
            other => anyhow::bail!("unknown experience mode: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationMode {
    #[default]
    Default,
    Plan,
    Goal,
}

impl CollaborationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Plan => "plan",
            Self::Goal => "goal",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "default" => Ok(Self::Default),
            "plan" => Ok(Self::Plan),
            "goal" => Ok(Self::Goal),
            other => anyhow::bail!("unknown collaboration mode: {other}"),
        }
    }
}

impl Thread {
    pub fn new(title: impl Into<String>, workspace_root: PathBuf) -> Self {
        Self::new_with_mode(title, workspace_root, ExperienceMode::Code)
    }

    pub fn new_with_mode(
        title: impl Into<String>,
        workspace_root: PathBuf,
        experience_mode: ExperienceMode,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            workspace_root,
            project_id: None,
            experience_mode,
            model_selection: None,
            archived_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn new_in_project(
        title: impl Into<String>,
        workspace_root: PathBuf,
        project_id: Uuid,
    ) -> Self {
        Self::new_in_project_with_mode(title, workspace_root, project_id, ExperienceMode::Code)
    }

    pub fn new_in_project_with_mode(
        title: impl Into<String>,
        workspace_root: PathBuf,
        project_id: Uuid,
        experience_mode: ExperienceMode,
    ) -> Self {
        let mut thread = Self::new_with_mode(title, workspace_root, experience_mode);
        thread.project_id = Some(project_id);
        thread
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
    Text {
        text: String,
    },
    Image {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<Uuid>,
        #[serde(rename = "contentType", alias = "content_type")]
        content_type: String,
        data: Vec<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    ImageRef {
        image_id: Uuid,
    },
    ToolCall {
        call: ToolCall,
    },
    ToolResult {
        result: ToolResult,
    },
    FileRef {
        path: PathBuf,
    },
    SourceRef {
        source: ContextSourceRef,
    },
    SkillRef {
        skill: SkillRef,
    },
    TurnContext {
        collaboration_mode: CollaborationMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        goal_id: Option<Uuid>,
        /// Optional per-Turn retrieval backend selected by the conversation UI.
        /// This is execution metadata only; model history projection omits the
        /// TurnContext part itself.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        library_provider: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSourceRef {
    pub id: Uuid,
    pub path: PathBuf,
    pub name: String,
    pub kind: ContextSourceKind,
    pub content_type: String,
    pub bytes: u64,
    pub truncated: bool,
}

impl From<&LoadedContextSource> for ContextSourceRef {
    fn from(source: &LoadedContextSource) -> Self {
        Self {
            id: Uuid::new_v4(),
            path: source.path.clone(),
            name: source.name.clone(),
            kind: source.kind,
            content_type: source.content_type.clone(),
            bytes: source.bytes,
            truncated: source.truncated,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub truncated: bool,
}

impl From<&LoadedSkill> for SkillRef {
    fn from(skill: &LoadedSkill) -> Self {
        Self {
            id: skill.descriptor.id.clone(),
            name: skill.descriptor.name.clone(),
            description: skill.descriptor.description.clone(),
            path: skill.descriptor.path.clone(),
            truncated: skill.truncated,
        }
    }
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

/// A typed unit of model input or tool output.
///
/// Text is the portable baseline for providers and tools, while the other
/// variants retain information that would otherwise be flattened into a prompt
/// string. `Image` stores the original bytes so provider adapters can choose
/// their native multimodal representation at the last possible point.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModelContentPart {
    Text {
        text: String,
    },
    Json {
        value: Value,
    },
    Image {
        content_type: String,
        data: Vec<u8>,
    },
    Resource {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl ModelContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn json(value: Value) -> Self {
        Self::Json { value }
    }

    pub fn image(content_type: impl Into<String>, data: Vec<u8>) -> Self {
        Self::Image {
            content_type: content_type.into(),
            data,
        }
    }

    pub fn resource(
        uri: impl Into<String>,
        content_type: Option<String>,
        name: Option<String>,
    ) -> Self {
        Self::Resource {
            uri: uri.into(),
            content_type,
            name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    pub call_id: Uuid,
    /// Legacy text output. New tools should populate `content`; consumers can
    /// use `content_or_legacy_text` while callers migrate.
    pub output: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ModelContentPart>,
    /// Tool-specific metadata is also the forward-compatible place for context
    /// and artifact hints, such as truncated/originalBytes/maxResults.
    pub metadata: Value,
}

impl ToolResult {
    pub fn text(call_id: Uuid, output: impl Into<String>, metadata: Value) -> Self {
        let output = output.into();
        Self {
            call_id,
            content: vec![ModelContentPart::text(output.clone())],
            output,
            metadata,
        }
    }

    /// Returns typed content for both new and persisted legacy results.
    pub fn content_or_legacy_text(&self) -> Vec<ModelContentPart> {
        if self.content.is_empty() {
            vec![ModelContentPart::text(self.output.clone())]
        } else {
            self.content.clone()
        }
    }
}

pub type GoalStatus = WorkFormStatus;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalRecord {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub objective: String,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
    pub version: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl GoalRecord {
    pub fn new(thread_id: Uuid, objective: impl Into<String>, token_budget: Option<u64>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            thread_id,
            objective: objective.into(),
            token_budget,
            tokens_used: 0,
            time_used_seconds: 0,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalSnapshot {
    pub goal: GoalRecord,
    pub work_form: WorkForm,
}

impl GoalSnapshot {
    pub fn status(&self) -> GoalStatus {
        self.work_form.status
    }

    pub fn completed_tasks(&self) -> usize {
        self.work_form.completed_items()
    }

    pub fn render_for_model(&self) -> String {
        format!(
            "Goal id: {}\n{}",
            self.goal.id,
            self.work_form.render_for_model()
        )
    }
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

pub const CONTEXT_CHECKPOINT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextCheckpointMode {
    #[default]
    LegacyText,
    Manual,
    StructuredLocal,
    NativeProvider,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextFactStatus {
    #[default]
    Active,
    Resolved,
    Superseded,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointCoverage {
    pub through_seq: i64,
    pub through_message_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointFact {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub status: ContextFactStatus,
    #[serde(default)]
    pub source_seqs: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointFile {
    pub path: PathBuf,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub source_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointWorkspace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_status: Option<String>,
    #[serde(default)]
    pub files_changed: Vec<ContextCheckpointFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointCommand {
    pub command: String,
    pub outcome: String,
    pub summary: String,
    #[serde(default)]
    pub source_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointStep {
    pub id: String,
    pub text: String,
    pub status: String,
    #[serde(default)]
    pub source_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointInteraction {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub source_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpointArtifact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub source_seqs: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCheckpoint {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub schema_version: u32,
    pub mode: ContextCheckpointMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_checkpoint_id: Option<Uuid>,
    pub coverage: ContextCheckpointCoverage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_compatibility_hash: Option<String>,
    pub goal: String,
    #[serde(default)]
    pub user_constraints: Vec<ContextCheckpointFact>,
    #[serde(default)]
    pub decisions: Vec<ContextCheckpointFact>,
    #[serde(default)]
    pub workspace_state: ContextCheckpointWorkspace,
    #[serde(default)]
    pub commands_and_validation: Vec<ContextCheckpointCommand>,
    #[serde(default)]
    pub open_issues: Vec<ContextCheckpointFact>,
    #[serde(default)]
    pub next_steps: Vec<ContextCheckpointStep>,
    #[serde(default)]
    pub pending_interactions: Vec<ContextCheckpointInteraction>,
    #[serde(default)]
    pub artifacts: Vec<ContextCheckpointArtifact>,
    pub created_at: DateTime<Utc>,
}

impl ContextCheckpoint {
    pub fn manual(
        thread_id: Uuid,
        coverage: ContextCheckpointCoverage,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            thread_id,
            schema_version: CONTEXT_CHECKPOINT_SCHEMA_VERSION,
            mode: ContextCheckpointMode::Manual,
            previous_checkpoint_id: None,
            coverage,
            provider_compatibility_hash: None,
            goal: summary.into(),
            user_constraints: Vec::new(),
            decisions: Vec::new(),
            workspace_state: ContextCheckpointWorkspace::default(),
            commands_and_validation: Vec::new(),
            open_issues: Vec::new(),
            next_steps: Vec::new(),
            pending_interactions: Vec::new(),
            artifacts: Vec::new(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextProjection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_mode: Option<String>,
    pub checkpoint_tokens: usize,
    pub covered_through_seq: i64,
    pub covered_message_count: usize,
    pub unsummarized_message_count: usize,
    pub unsummarized_event_count: usize,
    pub recent_tail_tokens: usize,
    pub native_compaction_supported: bool,
    pub provider_state_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state_kind: Option<String>,
    pub provider_item_count: usize,
    pub native_compaction_item_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompactionMetrics {
    pub source: String,
    pub input_tokens: usize,
    pub checkpoint_tokens: usize,
    pub token_reduction_percent: usize,
    pub latency_ms: u64,
    pub fact_retention_percent: usize,
    pub active_constraint_retention_percent: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextCompactionDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<Uuid>,
    pub mode: ContextCheckpointMode,
    pub coverage: ContextCheckpointCoverage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_state_checkpoint_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<ContextCompactionMetrics>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<ContextCheckpoint>,
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
            checkpoint: None,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputOption {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<UserInputOption>,
    #[serde(default = "default_true")]
    pub allow_custom: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputRequest {
    pub request_id: Uuid,
    pub questions: Vec<UserInputQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputAnswer {
    pub question_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputResponse {
    pub answers: Vec<UserInputAnswer>,
    /// Skip the optional decision and let the same Turn continue with a
    /// reasonable assumption.
    #[serde(default)]
    pub skipped: bool,
    /// Dismiss the decision boundary and end the waiting Turn without another
    /// model invocation. A later user message starts a new Turn normally.
    #[serde(default)]
    pub cancelled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserInputStatus {
    Pending,
    Answered,
}

impl UserInputStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Answered => "answered",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "answered" => Ok(Self::Answered),
            other => anyhow::bail!("unknown user input status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserInputRecord {
    pub thread_id: Uuid,
    pub request: UserInputRequest,
    pub status: UserInputStatus,
    pub response: Option<UserInputResponse>,
    pub created_at: DateTime<Utc>,
    pub answered_at: Option<DateTime<Utc>>,
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
pub enum TurnStatus {
    Running,
    WaitingApproval,
    WaitingUserInput,
    WaitingUserAction,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl TurnStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting_approval",
            Self::WaitingUserInput => "waiting_user_input",
            Self::WaitingUserAction => "waiting_user_action",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "waiting_approval" => Ok(Self::WaitingApproval),
            "waiting_user_input" => Ok(Self::WaitingUserInput),
            "waiting_user_action" => Ok(Self::WaitingUserAction),
            "cancelling" => Ok(Self::Cancelling),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "interrupted" => Ok(Self::Interrupted),
            other => anyhow::bail!("unknown turn status: {other}"),
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnRecord {
    pub turn_id: Uuid,
    /// Monotonic execution attempt inside this logical Turn. Interactive
    /// resumes increment this value without changing `turn_id`.
    #[serde(default = "default_turn_invocation_id")]
    pub invocation_id: u64,
    pub thread_id: Uuid,
    pub user_message_id: Uuid,
    pub status: TurnStatus,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl TurnRecord {
    pub fn running(thread_id: Uuid, user_message_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            turn_id: Uuid::new_v4(),
            invocation_id: 1,
            thread_id,
            user_message_id,
            status: TurnStatus::Running,
            started_at: now,
            updated_at: now,
            completed_at: None,
            error: None,
        }
    }
}

fn default_turn_invocation_id() -> u64 {
    1
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnChangeSetStatus {
    Capturing,
    Ready,
    Empty,
    Failed,
}

impl TurnChangeSetStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Capturing => "capturing",
            Self::Ready => "ready",
            Self::Empty => "empty",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "capturing" => Ok(Self::Capturing),
            "ready" => Ok(Self::Ready),
            "empty" => Ok(Self::Empty),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown turn change-set status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnFileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnFileChange {
    pub kind: TurnFileChangeKind,
    pub old_path: Option<PathBuf>,
    pub new_path: Option<PathBuf>,
    pub before_oid: Option<String>,
    pub after_oid: Option<String>,
    pub before_mode: Option<String>,
    pub after_mode: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub binary: bool,
}

impl TurnFileChange {
    pub fn display_path(&self) -> Option<&PathBuf> {
        self.new_path.as_ref().or(self.old_path.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnChangeSet {
    pub turn_id: Uuid,
    pub thread_id: Uuid,
    pub workspace_root: PathBuf,
    pub repo_root: Option<PathBuf>,
    pub workspace_prefix: Option<PathBuf>,
    pub before_tree: Option<String>,
    pub after_tree: Option<String>,
    pub status: TurnChangeSetStatus,
    pub files: Vec<TurnFileChange>,
    pub additions: u64,
    pub deletions: u64,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
    pub reverted_at: Option<DateTime<Utc>>,
}

impl TurnChangeSet {
    pub fn capturing(turn_id: Uuid, thread_id: Uuid, workspace_root: PathBuf) -> Self {
        Self {
            turn_id,
            thread_id,
            workspace_root,
            repo_root: None,
            workspace_prefix: None,
            before_tree: None,
            after_tree: None,
            status: TurnChangeSetStatus::Capturing,
            files: Vec::new(),
            additions: 0,
            deletions: 0,
            error: None,
            created_at: Utc::now(),
            finalized_at: None,
            reverted_at: None,
        }
    }

    pub fn is_undoable(&self) -> bool {
        self.status == TurnChangeSetStatus::Ready
            && !self.files.is_empty()
            && self.reverted_at.is_none()
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
    ThreadContextSnapshot {
        snapshot: ThreadContextSnapshot,
    },
    TurnContextSnapshot {
        snapshot: TurnContextSnapshot,
    },
    TurnStarted {
        user_message_id: Uuid,
    },
    ModelContextBuilt {
        #[serde(default = "Uuid::new_v4")]
        request_id: Uuid,
        round: usize,
        context_hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stable_prefix_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dynamic_tail_hash: Option<String>,
        token_estimate: usize,
        #[serde(default)]
        purpose: ModelCallPurpose,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_breakdown: Option<TokenEstimateBreakdown>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<ModelContextItem>,
    },
    ModelRequest {
        #[serde(default = "Uuid::new_v4")]
        request_id: Uuid,
        round: usize,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        request: Value,
    },
    ProviderRequestSent {
        request_id: Uuid,
        round: usize,
        attempt: usize,
        adapter: String,
        method: String,
        endpoint: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        body: Value,
    },
    ProviderRequestRetried {
        request_id: Uuid,
        round: usize,
        attempt: usize,
        #[serde(default)]
        retry_kind: ProviderRetryKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_index: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_limit: Option<usize>,
        reason: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        body: Value,
    },
    ProviderResponseReceived {
        request_id: Uuid,
        round: usize,
        attempt: usize,
        status: Option<u16>,
        response_id: Option<String>,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        body: Value,
    },
    ModelDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallStarted {
        call: ToolCall,
    },
    ToolCallFinished {
        result: ToolResult,
    },
    WorkFormUpdated {
        form: WorkForm,
    },
    GoalUpdated {
        snapshot: GoalSnapshot,
    },
    UserInputRequested {
        request: UserInputRequest,
    },
    AssistantMessage {
        message: Message,
    },
    FileChanged {
        path: PathBuf,
        summary: String,
    },
    TurnChangesRecorded {
        change_set: TurnChangeSet,
    },
    TurnUndoCompleted {
        target_turn_id: Uuid,
        files_changed: usize,
    },
    ApprovalRequested {
        approval_id: Uuid,
        reason: String,
        action: String,
    },
    AutomaticApprovalReviewStarted {
        review_id: Uuid,
        target_item_id: String,
        action: Value,
    },
    AutomaticApprovalReviewCompleted {
        review_id: Uuid,
        target_item_id: String,
        status: GuardianReviewStatus,
        risk_level: Option<GuardianRiskLevel>,
        user_authorization: Option<GuardianUserAuthorization>,
        rationale: String,
        action: Value,
        #[serde(default)]
        usage: ModelUsage,
        #[serde(default)]
        attempts: usize,
        #[serde(default)]
        tool_rounds: usize,
        #[serde(default)]
        decision_source: GuardianDecisionSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure_kind: Option<GuardianReviewFailureKind>,
    },
    AutoReviewInterruptionWarning {
        message: String,
    },
    ContextCompacted {
        summary: ContextSummary,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<ContextCompactionDetails>,
    },
    ContextProjectionBuilt {
        projection: ContextProjection,
    },
    ProviderContextStateUpdated {
        provider_id: String,
        model: String,
        state_kind: String,
        response_item_count: usize,
        compaction_item_count: usize,
    },
    ProviderContextStateInvalidated {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        reason: String,
    },
    ContextWarning {
        stage: String,
        message: String,
    },
    TokenUsage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<Uuid>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        round: Option<usize>,
        #[serde(default)]
        purpose: ModelCallPurpose,
        input_tokens: usize,
        output_tokens: usize,
        total_tokens: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_input_tokens: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_write_tokens: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_input_estimate: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_breakdown: Option<TokenEstimateBreakdown>,
    },
    SubagentUpdated {
        run: SubagentRun,
    },
    TurnFinished {
        summary: String,
    },
    TurnSuspended {
        approval_id: Uuid,
        reason: String,
    },
    BrowserHandoffRequired {
        action: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
    BrowserHandoffCompleted {
        prior_turn_id: Uuid,
    },
    TurnAwaitingInput {
        request_id: Uuid,
    },
    TurnCancelled {
        reason: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCallPurpose {
    #[default]
    AgentRound,
    ContextCompaction,
    GuardianReview,
    TitleGeneration,
    Other,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRetryKind {
    #[default]
    Network,
    #[serde(alias = "compatibility")]
    StateRecovery,
}

impl AgentEventPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ThreadContextSnapshot { .. } => "thread_context_snapshot",
            Self::TurnContextSnapshot { .. } => "turn_context_snapshot",
            Self::TurnStarted { .. } => "turn_started",
            Self::ModelContextBuilt { .. } => "model_context_built",
            Self::ModelRequest { .. } => "model_request",
            Self::ProviderRequestSent { .. } => "provider_request_sent",
            Self::ProviderRequestRetried { .. } => "provider_request_retried",
            Self::ProviderResponseReceived { .. } => "provider_response_received",
            Self::ModelDelta { .. } => "model_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallFinished { .. } => "tool_call_finished",
            Self::WorkFormUpdated { .. } => "work_form_updated",
            Self::GoalUpdated { .. } => "goal_updated",
            Self::UserInputRequested { .. } => "user_input_requested",
            Self::AssistantMessage { .. } => "assistant_message",
            Self::FileChanged { .. } => "file_changed",
            Self::TurnChangesRecorded { .. } => "turn_changes_recorded",
            Self::TurnUndoCompleted { .. } => "turn_undo_completed",
            Self::ApprovalRequested { .. } => "approval_requested",
            Self::AutomaticApprovalReviewStarted { .. } => "automatic_approval_review_started",
            Self::AutomaticApprovalReviewCompleted { .. } => "automatic_approval_review_completed",
            Self::AutoReviewInterruptionWarning { .. } => "auto_review_interruption_warning",
            Self::ContextCompacted { .. } => "context_compacted",
            Self::ContextProjectionBuilt { .. } => "context_projection_built",
            Self::ProviderContextStateUpdated { .. } => "provider_context_state_updated",
            Self::ProviderContextStateInvalidated { .. } => "provider_context_state_invalidated",
            Self::ContextWarning { .. } => "context_warning",
            Self::TokenUsage { .. } => "token_usage",
            Self::SubagentUpdated { .. } => "subagent_updated",
            Self::TurnFinished { .. } => "turn_finished",
            Self::TurnSuspended { .. } => "turn_suspended",
            Self::BrowserHandoffRequired { .. } => "browser_handoff_required",
            Self::BrowserHandoffCompleted { .. } => "browser_handoff_completed",
            Self::TurnAwaitingInput { .. } => "turn_awaiting_input",
            Self::TurnCancelled { .. } => "turn_cancelled",
            Self::Error { .. } => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_collaboration_mode_round_trips_without_becoming_default() {
        assert_eq!(
            CollaborationMode::from_str("plan").expect("parse plan mode"),
            CollaborationMode::Plan
        );
        assert_eq!(
            serde_json::to_value(CollaborationMode::Plan).expect("serialize plan mode"),
            json!("plan")
        );
    }

    #[test]
    fn legacy_context_compacted_events_deserialize_without_details() {
        let thread_id = Uuid::new_v4();
        let summary = ContextSummary::new(thread_id, 4, 2, "legacy summary");
        let payload = json!({
            "type": "context_compacted",
            "summary": summary,
        });

        let parsed: AgentEventPayload = serde_json::from_value(payload).expect("deserialize");
        assert!(matches!(
            parsed,
            AgentEventPayload::ContextCompacted { details: None, .. }
        ));
    }

    #[test]
    fn legacy_automatic_review_events_deserialize_with_new_defaults() {
        let current = AgentEventPayload::AutomaticApprovalReviewCompleted {
            review_id: Uuid::new_v4(),
            target_item_id: "call-1".to_string(),
            status: GuardianReviewStatus::DeniedByPolicy,
            risk_level: Some(GuardianRiskLevel::High),
            user_authorization: Some(GuardianUserAuthorization::Unknown),
            rationale: "legacy denial".to_string(),
            action: json!({ "type": "command" }),
            usage: ModelUsage::default(),
            attempts: 0,
            tool_rounds: 0,
            decision_source: GuardianDecisionSource::Guardian,
            failure_kind: None,
        };
        let mut payload = serde_json::to_value(current).expect("serialize");
        let object = payload.as_object_mut().expect("event object");
        object.insert("status".to_string(), json!("denied"));
        object.remove("usage");
        object.remove("attempts");
        object.remove("tool_rounds");
        object.remove("decision_source");
        object.remove("failure_kind");

        let parsed: AgentEventPayload = serde_json::from_value(payload).expect("deserialize");
        assert!(matches!(
            parsed,
            AgentEventPayload::AutomaticApprovalReviewCompleted {
                status: GuardianReviewStatus::DeniedByPolicy,
                attempts: 0,
                tool_rounds: 0,
                decision_source: GuardianDecisionSource::Guardian,
                failure_kind: None,
                ..
            }
        ));
    }

    #[test]
    fn legacy_tool_output_remains_typed_text_content() {
        let result = ToolResult {
            call_id: Uuid::nil(),
            output: "legacy output".to_string(),
            content: Vec::new(),
            metadata: json!({}),
        };

        assert_eq!(
            result.content_or_legacy_text(),
            vec![ModelContentPart::text("legacy output")]
        );
    }

    #[test]
    fn typed_content_round_trips_through_json() {
        let content = vec![
            ModelContentPart::image("image/png", vec![1, 2, 3]),
            ModelContentPart::resource(
                "file:///workspace/spec.pdf",
                Some("application/pdf".to_string()),
                Some("spec.pdf".to_string()),
            ),
            ModelContentPart::json(json!({ "rows": 4 })),
        ];
        let serialized = serde_json::to_string(&content).unwrap();
        let restored: Vec<ModelContentPart> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored, content);
    }

    #[test]
    fn reasoning_delta_uses_the_public_snake_case_event_contract() {
        let payload = AgentEventPayload::ReasoningDelta {
            text: "检查项目结构".to_string(),
        };

        assert_eq!(
            serde_json::to_value(payload).unwrap(),
            json!({
                "type": "reasoning_delta",
                "text": "检查项目结构"
            })
        );
    }

    #[test]
    fn model_request_uses_the_public_snapshot_event_contract() {
        let payload = AgentEventPayload::ModelRequest {
            request_id: Uuid::nil(),
            round: 2,
            request: json!({
                "systemPrompt": "system",
                "userMessage": "current"
            }),
        };

        assert_eq!(
            serde_json::to_value(payload).unwrap(),
            json!({
                "type": "model_request",
                "request_id": Uuid::nil(),
                "round": 2,
                "request": {
                    "systemPrompt": "system",
                    "userMessage": "current"
                }
            })
        );
    }

    #[test]
    fn context_summary_checkpoint_is_backward_compatible() {
        let thread_id = Uuid::new_v4();
        let legacy = ContextSummary::new(thread_id, 7, 3, "legacy summary");
        let legacy_value = serde_json::to_value(&legacy).expect("serialize legacy summary");
        assert!(legacy_value.get("checkpoint").is_none());
        let restored: ContextSummary =
            serde_json::from_value(legacy_value).expect("restore legacy summary");
        assert!(restored.checkpoint.is_none());

        let mut structured = ContextSummary::new(thread_id, 9, 4, "structured summary");
        structured.checkpoint = Some(ContextCheckpoint::manual(
            thread_id,
            ContextCheckpointCoverage {
                through_seq: 9,
                through_message_count: 4,
            },
            "finish the implementation",
        ));
        let value = serde_json::to_value(&structured).expect("serialize checkpoint");
        let restored: ContextSummary = serde_json::from_value(value).expect("restore checkpoint");
        assert_eq!(
            restored
                .checkpoint
                .expect("checkpoint")
                .coverage
                .through_message_count,
            4
        );
    }
}
