use crate::context_sources::{ContextSourceKind, LoadedContextSource};
use crate::guardian::{GuardianReviewStatus, GuardianRiskLevel, GuardianUserAuthorization};
use crate::model_context::{ModelContextItem, ThreadContextSnapshot, TurnContextSnapshot};
use crate::skills::LoadedSkill;
use crate::subagents::SubagentRun;
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
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceMode {
    Work,
    #[default]
    Code,
}

impl ExperienceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Code => "code",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "work" => Ok(Self::Work),
            "code" => Ok(Self::Code),
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
/// Text remains the compatibility path for existing providers and tools, while
/// the other variants retain information that would otherwise be flattened into
/// a prompt string. `Image` stores the original bytes so provider adapters can
/// choose their native multimodal representation at the last possible point.
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Deferred,
    Blocked,
    Cancelled,
}

impl TaskPlanStepStatus {
    pub fn marker(self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::InProgress => "[>]",
            Self::Completed => "[x]",
            Self::Deferred => "[-]",
            Self::Blocked => "[!]",
            Self::Cancelled => "[/]",
        }
    }

    pub fn is_actionable(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }

    pub fn is_resolved(self) -> bool {
        !self.is_actionable()
    }

    pub fn requires_status_reason(self) -> bool {
        matches!(self, Self::Deferred | Self::Blocked | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskPlanStep {
    #[serde(default)]
    pub id: String,
    #[serde(alias = "step")]
    pub title: String,
    pub status: TaskPlanStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskPlan {
    #[serde(default)]
    pub plan_revision: u64,
    #[serde(default)]
    pub goal_id: String,
    #[serde(
        default,
        alias = "explanation",
        skip_serializing_if = "Option::is_none"
    )]
    pub change_reason: Option<String>,
    pub steps: Vec<TaskPlanStep>,
}

impl TaskPlan {
    pub fn normalize_legacy(mut self) -> Self {
        if self.goal_id.trim().is_empty() {
            self.goal_id = "legacy-plan".to_string();
        }
        for index in 0..self.steps.len() {
            if !self.steps[index].id.trim().is_empty() {
                continue;
            }
            let mut suffix = index + 1;
            loop {
                let candidate = format!("legacy-step-{suffix}");
                if !self.steps.iter().any(|step| step.id == candidate) {
                    self.steps[index].id = candidate;
                    break;
                }
                suffix += 1;
            }
        }
        self
    }

    pub fn is_active(&self) -> bool {
        self.steps.iter().any(|step| {
            !matches!(
                step.status,
                TaskPlanStepStatus::Completed | TaskPlanStepStatus::Cancelled
            )
        })
    }

    pub fn has_actionable_steps(&self) -> bool {
        self.steps.iter().any(|step| step.status.is_actionable())
    }

    pub fn next_runnable_step(&self) -> Option<&TaskPlanStep> {
        self.steps
            .iter()
            .find(|step| step.status == TaskPlanStepStatus::InProgress)
            .or_else(|| {
                self.steps.iter().find(|step| {
                    step.status == TaskPlanStepStatus::Pending
                        && step.dependencies.iter().all(|dependency| {
                            self.steps.iter().any(|candidate| {
                                candidate.id == *dependency
                                    && candidate.status == TaskPlanStepStatus::Completed
                            })
                        })
                })
            })
    }

    pub fn render_for_model(&self) -> String {
        let plan = self.clone().normalize_legacy();
        let mut lines = vec![format!(
            "Goal: {} (plan revision {})",
            plan.goal_id, plan.plan_revision
        )];
        if let Some(change_reason) = plan.change_reason.as_deref() {
            lines.push(format!("Last change: {change_reason}"));
        }
        for step in &plan.steps {
            lines.push(format!(
                "{} {}: {}",
                step.status.marker(),
                step.id,
                step.title
            ));
            if let Some(reason) = step.status_reason.as_deref() {
                lines.push(format!("  Status reason: {reason}"));
            }
            if !step.dependencies.is_empty() {
                lines.push(format!("  Depends on: {}", step.dependencies.join(", ")));
            }
            for criterion in &step.acceptance_criteria {
                lines.push(format!("  Acceptance: {criterion}"));
            }
            for evidence in &step.evidence {
                lines.push(format!("  Evidence: {evidence}"));
            }
        }
        lines.join("\n")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Draft,
    Ready,
    Active,
    Paused,
    Completed,
    Blocked,
    Cancelled,
    Failed,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "draft" => Ok(Self::Draft),
            "ready" => Ok(Self::Ready),
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "blocked" => Ok(Self::Blocked),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown goal status: {other}"),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalTaskStatus {
    Pending,
    Running,
    Succeeded,
    Deferred,
    Blocked,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalAttemptStatus {
    Running,
    Succeeded,
    Failed,
    Interrupted,
}

impl GoalAttemptStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            other => anyhow::bail!("unknown goal attempt status: {other}"),
        }
    }
}

impl GoalTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Deferred => "deferred",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "deferred" => Ok(Self::Deferred),
            "blocked" => Ok(Self::Blocked),
            "cancelled" => Ok(Self::Cancelled),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown goal task status: {other}"),
        }
    }

    pub fn is_resolved(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

impl From<TaskPlanStepStatus> for GoalTaskStatus {
    fn from(value: TaskPlanStepStatus) -> Self {
        match value {
            TaskPlanStepStatus::Pending => Self::Pending,
            TaskPlanStepStatus::InProgress => Self::Running,
            TaskPlanStepStatus::Completed => Self::Succeeded,
            TaskPlanStepStatus::Deferred => Self::Deferred,
            TaskPlanStepStatus::Blocked => Self::Blocked,
            TaskPlanStepStatus::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalRecord {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub objective: String,
    pub status: GoalStatus,
    pub plan_revision: u64,
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
    pub version: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl GoalRecord {
    pub fn new(
        thread_id: Uuid,
        objective: impl Into<String>,
        status: GoalStatus,
        token_budget: Option<u64>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            thread_id,
            objective: objective.into(),
            status,
            plan_revision: 0,
            token_budget,
            tokens_used: 0,
            time_used_seconds: 0,
            version: 1,
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalTask {
    pub goal_id: Uuid,
    pub step_id: String,
    pub ordinal: usize,
    pub title: String,
    pub status: GoalTaskStatus,
    pub status_reason: Option<String>,
    pub dependencies: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub evidence: Vec<String>,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalTaskAttempt {
    pub id: Uuid,
    pub goal_id: Uuid,
    pub step_id: String,
    pub turn_id: Uuid,
    pub attempt_no: u32,
    pub status: GoalAttemptStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub evidence: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoalSnapshot {
    pub goal: GoalRecord,
    pub tasks: Vec<GoalTask>,
    pub attempts: Vec<GoalTaskAttempt>,
}

impl GoalSnapshot {
    pub fn completed_tasks(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.status == GoalTaskStatus::Succeeded)
            .count()
    }

    pub fn render_for_model(&self) -> String {
        let mut lines = vec![
            format!("Goal id: {}", self.goal.id),
            format!("Objective: {}", self.goal.objective),
            format!("Status: {}", self.goal.status.as_str()),
            format!("Plan revision: {}", self.goal.plan_revision),
        ];
        for task in &self.tasks {
            lines.push(format!(
                "- {} [{}]: {}",
                task.step_id,
                task.status.as_str(),
                task.title
            ));
        }
        lines.join("\n")
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
        token_estimate: usize,
        items: Vec<ModelContextItem>,
    },
    ModelRequest {
        #[serde(default = "Uuid::new_v4")]
        request_id: Uuid,
        round: usize,
        request: Value,
    },
    ProviderRequestSent {
        request_id: Uuid,
        round: usize,
        attempt: usize,
        adapter: String,
        method: String,
        endpoint: String,
        body: Value,
    },
    ProviderRequestRetried {
        request_id: Uuid,
        round: usize,
        attempt: usize,
        reason: String,
        body: Value,
    },
    ProviderResponseReceived {
        request_id: Uuid,
        round: usize,
        attempt: usize,
        status: Option<u16>,
        response_id: Option<String>,
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
    PlanUpdated {
        plan: TaskPlan,
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
    },
    AutoReviewInterruptionWarning {
        message: String,
    },
    ContextCompacted {
        summary: ContextSummary,
    },
    ContextWarning {
        stage: String,
        message: String,
    },
    TokenUsage {
        input_tokens: usize,
        output_tokens: usize,
        total_tokens: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cached_input_tokens: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cache_write_tokens: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_tokens: Option<usize>,
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
            Self::PlanUpdated { .. } => "plan_updated",
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
            Self::ContextWarning { .. } => "context_warning",
            Self::TokenUsage { .. } => "token_usage",
            Self::SubagentUpdated { .. } => "subagent_updated",
            Self::TurnFinished { .. } => "turn_finished",
            Self::TurnSuspended { .. } => "turn_suspended",
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
    fn legacy_task_plans_gain_stable_fields_when_restored() {
        let legacy: TaskPlan = serde_json::from_value(json!({
            "explanation": "Continue the existing work",
            "steps": [
                { "step": "Inspect the repository", "status": "in_progress" },
                { "step": "Implement the fix", "status": "pending" }
            ]
        }))
        .unwrap();

        let restored = legacy.normalize_legacy();
        assert_eq!(restored.plan_revision, 0);
        assert_eq!(restored.goal_id, "legacy-plan");
        assert_eq!(
            restored.change_reason.as_deref(),
            Some("Continue the existing work")
        );
        assert_eq!(restored.steps[0].id, "legacy-step-1");
        assert_eq!(restored.steps[0].title, "Inspect the repository");
        assert!(restored.steps[0].dependencies.is_empty());
        assert!(restored
            .render_for_model()
            .contains("[>] legacy-step-1: Inspect the repository"));
    }

    #[test]
    fn task_plan_selects_the_next_dependency_ready_step() {
        let plan: TaskPlan = serde_json::from_value(json!({
            "planRevision": 4,
            "goalId": "ordered-work",
            "steps": [
                {
                    "id": "inspect",
                    "title": "Inspect",
                    "status": "completed",
                    "evidence": ["Inspection finished"]
                },
                {
                    "id": "implement",
                    "title": "Implement",
                    "status": "pending",
                    "dependencies": ["inspect"]
                },
                {
                    "id": "verify",
                    "title": "Verify",
                    "status": "pending",
                    "dependencies": ["implement"]
                }
            ]
        }))
        .unwrap();

        assert!(plan.is_active());
        assert_eq!(
            plan.next_runnable_step().map(|step| step.id.as_str()),
            Some("implement")
        );

        let mut deferred_plan = plan.clone();
        deferred_plan.steps[1].status = TaskPlanStepStatus::Deferred;
        deferred_plan.steps[1].status_reason = Some("Continue next session".to_string());
        deferred_plan.steps[2].status = TaskPlanStepStatus::Cancelled;
        deferred_plan.steps[2].status_reason = Some("No longer required".to_string());
        assert!(deferred_plan.is_active());
        assert!(!deferred_plan.has_actionable_steps());
        assert!(deferred_plan.next_runnable_step().is_none());
    }
}
