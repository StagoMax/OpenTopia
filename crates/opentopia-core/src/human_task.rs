use crate::workflow_interrupt::{WorkflowInterruptKindV1, WorkflowInterruptRequestV1};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const HUMAN_TASK_SCHEMA_VERSION_V1: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanTaskTypeV1 {
    Approval,
    InputRequest,
    OutputReview,
    Recovery,
    Reconnect,
    DataCorrection,
    Reconciliation,
    Manual,
}

impl HumanTaskTypeV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::InputRequest => "input_request",
            Self::OutputReview => "output_review",
            Self::Recovery => "recovery",
            Self::Reconnect => "reconnect",
            Self::DataCorrection => "data_correction",
            // The v25 indexed compatibility column groups reconciliation under
            // recovery. The typed document preserves `reconciliation`.
            Self::Reconciliation => "recovery",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanTaskStatusV1 {
    Pending,
    Completed,
    Cancelled,
}

impl HumanTaskStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanTaskSourceKindV1 {
    FlowRun,
}

impl HumanTaskSourceKindV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FlowRun => "flow_run",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HumanTaskActionV1 {
    Approve,
    Reject,
    Retry,
    Resume,
    Submit,
    Reconnect,
    Acknowledge,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HumanTaskResolutionV1 {
    pub action: HumanTaskActionV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub resolved_by: String,
    pub resolved_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HumanTaskV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub revision: u32,
    pub thread_id: Uuid,
    pub source_kind: HumanTaskSourceKindV1,
    pub source_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_node_run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_node_id: Option<String>,
    pub task_type: HumanTaskTypeV1,
    pub status: HumanTaskStatusV1,
    pub title: String,
    pub description: String,
    pub allowed_actions: Vec<HumanTaskActionV1>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<HumanTaskResolutionV1>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

impl HumanTaskV1 {
    pub fn flow_approval(
        thread_id: Uuid,
        flow_run_id: Uuid,
        node_run_id: Uuid,
        node_id: impl Into<String>,
        node_label: impl AsRef<str>,
        payload: Value,
    ) -> Self {
        let node_id = node_id.into();
        let node_label = node_label.as_ref();
        let now = Utc::now();
        Self {
            schema_version: HUMAN_TASK_SCHEMA_VERSION_V1,
            id: stable_task_id(flow_run_id, "approval", &node_run_id.to_string()),
            revision: 1,
            thread_id,
            source_kind: HumanTaskSourceKindV1::FlowRun,
            source_id: flow_run_id,
            source_node_run_id: Some(node_run_id),
            source_node_id: Some(node_id),
            task_type: HumanTaskTypeV1::Approval,
            status: HumanTaskStatusV1::Pending,
            title: format!("审批：{node_label}"),
            description: "Flow 已在节点边界暂停，等待人工决定后再继续执行。".to_string(),
            allowed_actions: vec![HumanTaskActionV1::Approve, HumanTaskActionV1::Reject],
            payload,
            action_schema: None,
            assigned_to: None,
            claimed_by: None,
            claimed_at: None,
            due_at: Some(now + Duration::hours(24)),
            checkpoint_id: None,
            continuation_id: None,
            resolution: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
        }
    }

    pub fn flow_recovery(
        thread_id: Uuid,
        flow_run_id: Uuid,
        source_node_run_id: Option<Uuid>,
        source_node_id: Option<String>,
        boundary_key: impl AsRef<str>,
        description: impl Into<String>,
        payload: Value,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: HUMAN_TASK_SCHEMA_VERSION_V1,
            id: stable_task_id(flow_run_id, "recovery", boundary_key.as_ref()),
            revision: 1,
            thread_id,
            source_kind: HumanTaskSourceKindV1::FlowRun,
            source_id: flow_run_id,
            source_node_run_id,
            source_node_id,
            task_type: HumanTaskTypeV1::Recovery,
            status: HumanTaskStatusV1::Pending,
            title: "检查并恢复 Flow".to_string(),
            description: description.into(),
            allowed_actions: vec![HumanTaskActionV1::Retry, HumanTaskActionV1::Cancel],
            payload,
            action_schema: None,
            assigned_to: None,
            claimed_by: None,
            claimed_at: None,
            due_at: Some(now + Duration::hours(4)),
            checkpoint_id: None,
            continuation_id: None,
            resolution: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
        }
    }

    pub fn flow_output_review(
        thread_id: Uuid,
        flow_run_id: Uuid,
        source_node_run_id: Option<Uuid>,
        source_node_id: Option<String>,
        checkpoint_id: Uuid,
        payload: Value,
    ) -> Self {
        let now = Utc::now();
        Self {
            schema_version: HUMAN_TASK_SCHEMA_VERSION_V1,
            id: stable_task_id(flow_run_id, "output-review", &checkpoint_id.to_string()),
            revision: 1,
            thread_id,
            source_kind: HumanTaskSourceKindV1::FlowRun,
            source_id: flow_run_id,
            source_node_run_id,
            source_node_id,
            task_type: HumanTaskTypeV1::OutputReview,
            status: HumanTaskStatusV1::Pending,
            title: "审阅 Flow 输出".to_string(),
            description: "输出已经在一致 Checkpoint 提交。通过后 Run 才会标记为成功。".to_string(),
            allowed_actions: vec![HumanTaskActionV1::Approve, HumanTaskActionV1::Reject],
            payload,
            action_schema: Some(serde_json::json!({
                "type": "output_review",
                "required": ["decision"],
            })),
            assigned_to: None,
            claimed_by: None,
            claimed_at: None,
            due_at: Some(now + Duration::hours(24)),
            checkpoint_id: Some(checkpoint_id),
            continuation_id: None,
            resolution: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
        }
    }

    pub fn flow_interrupt(
        thread_id: Uuid,
        flow_run_id: Uuid,
        interrupt: &WorkflowInterruptRequestV1,
        payload: Value,
    ) -> Self {
        let now = Utc::now();
        let (task_type, allowed_actions, due_hours) = match interrupt.kind {
            WorkflowInterruptKindV1::Approval => (
                HumanTaskTypeV1::Approval,
                vec![HumanTaskActionV1::Approve, HumanTaskActionV1::Reject],
                24,
            ),
            WorkflowInterruptKindV1::InputRequest => (
                HumanTaskTypeV1::InputRequest,
                vec![HumanTaskActionV1::Submit, HumanTaskActionV1::Cancel],
                24,
            ),
            WorkflowInterruptKindV1::ExternalAction => (
                HumanTaskTypeV1::Reconnect,
                vec![HumanTaskActionV1::Resume, HumanTaskActionV1::Cancel],
                8,
            ),
            WorkflowInterruptKindV1::EffectReconciliation => (
                HumanTaskTypeV1::Reconciliation,
                vec![HumanTaskActionV1::Acknowledge, HumanTaskActionV1::Cancel],
                4,
            ),
            WorkflowInterruptKindV1::ResumeRetry => (
                HumanTaskTypeV1::Recovery,
                vec![HumanTaskActionV1::Retry, HumanTaskActionV1::Cancel],
                4,
            ),
        };
        Self {
            schema_version: HUMAN_TASK_SCHEMA_VERSION_V1,
            id: stable_task_id(flow_run_id, "interrupt", &interrupt.id.to_string()),
            revision: 1,
            thread_id,
            source_kind: HumanTaskSourceKindV1::FlowRun,
            source_id: flow_run_id,
            source_node_run_id: Some(interrupt.node_run_id),
            source_node_id: Some(interrupt.node_id.clone()),
            task_type,
            status: HumanTaskStatusV1::Pending,
            title: interrupt.title.clone(),
            description: interrupt.description.clone(),
            allowed_actions,
            payload,
            action_schema: Some(interrupt_action_schema(interrupt)),
            assigned_to: None,
            claimed_by: None,
            claimed_at: None,
            due_at: Some(now + Duration::hours(due_hours)),
            checkpoint_id: Some(interrupt.checkpoint_id),
            continuation_id: Some(interrupt.continuation.id),
            resolution: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
        }
    }

    pub fn claim(&mut self, actor: impl Into<String>) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.status == HumanTaskStatusV1::Pending,
            "Human task is no longer pending"
        );
        let actor = actor.into();
        anyhow::ensure!(!actor.trim().is_empty(), "claim actor cannot be empty");
        anyhow::ensure!(
            self.claimed_by
                .as_deref()
                .is_none_or(|claimed_by| claimed_by == actor),
            "Human task is already claimed by another operator"
        );
        if self.claimed_by.as_deref() == Some(actor.as_str()) {
            return Ok(());
        }
        let now = Utc::now();
        self.claimed_by = Some(actor);
        self.claimed_at = Some(now);
        self.updated_at = now;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn assign(&mut self, assignee: Option<&str>) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.status == HumanTaskStatusV1::Pending,
            "Human task is no longer pending"
        );
        self.assigned_to = assignee
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self.updated_at = Utc::now();
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn resolve(
        &mut self,
        action: HumanTaskActionV1,
        note: Option<&str>,
        resolved_by: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.resolve_with_command(action, note, resolved_by, None, None, None)
    }

    pub fn resolve_with_command(
        &mut self,
        action: HumanTaskActionV1,
        note: Option<&str>,
        resolved_by: impl Into<String>,
        command_id: Option<Uuid>,
        idempotency_key: Option<&str>,
        response: Option<Value>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.status == HumanTaskStatusV1::Pending,
            "Human task is no longer pending"
        );
        anyhow::ensure!(
            self.allowed_actions.contains(&action),
            "action is not allowed for this Human task"
        );
        let now = Utc::now();
        self.status = if action == HumanTaskActionV1::Cancel {
            HumanTaskStatusV1::Cancelled
        } else {
            HumanTaskStatusV1::Completed
        };
        self.resolution = Some(HumanTaskResolutionV1 {
            action,
            note: note
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            resolved_by: resolved_by.into(),
            resolved_at: now,
            command_id,
            idempotency_key: idempotency_key
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            response,
        });
        self.resolved_at = Some(now);
        self.updated_at = now;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn cancel(
        &mut self,
        note: Option<&str>,
        resolved_by: impl Into<String>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.status == HumanTaskStatusV1::Pending,
            "Human task is no longer pending"
        );
        let now = Utc::now();
        self.status = HumanTaskStatusV1::Cancelled;
        self.resolution = Some(HumanTaskResolutionV1 {
            action: HumanTaskActionV1::Cancel,
            note: note
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            resolved_by: resolved_by.into(),
            resolved_at: now,
            command_id: None,
            idempotency_key: None,
            response: None,
        });
        self.resolved_at = Some(now);
        self.updated_at = now;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

fn interrupt_action_schema(interrupt: &WorkflowInterruptRequestV1) -> Value {
    match interrupt.kind {
        WorkflowInterruptKindV1::Approval => serde_json::json!({
            "type": "approval",
            "required": ["decision"],
        }),
        WorkflowInterruptKindV1::InputRequest => serde_json::json!({
            "type": "user_input_response",
            "request": interrupt.payload.get("request").cloned().unwrap_or(Value::Null),
        }),
        WorkflowInterruptKindV1::ExternalAction => serde_json::json!({
            "type": "external_observation",
            "required": ["observation"],
        }),
        WorkflowInterruptKindV1::EffectReconciliation => serde_json::json!({
            "type": "effect_reconciliation",
            "required": ["observation"],
            "effectId": interrupt.payload.get("effectId").cloned().unwrap_or(Value::Null),
        }),
        WorkflowInterruptKindV1::ResumeRetry => serde_json::json!({
            "type": "resume_retry",
            "required": ["retry"],
        }),
    }
}

fn stable_task_id(flow_run_id: Uuid, kind: &str, boundary_key: &str) -> Uuid {
    Uuid::new_v5(
        &flow_run_id,
        format!("human-task:{kind}:{boundary_key}").as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flow_approval_ids_are_stable_for_the_same_wait_boundary() {
        let run_id = Uuid::new_v4();
        let node_run_id = Uuid::new_v4();
        let first = HumanTaskV1::flow_approval(
            Uuid::new_v4(),
            run_id,
            node_run_id,
            "approve",
            "Deploy",
            json!({}),
        );
        let second = HumanTaskV1::flow_approval(
            first.thread_id,
            run_id,
            node_run_id,
            "approve",
            "Deploy",
            json!({}),
        );
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn task_resolution_enforces_the_action_contract() {
        let mut task = HumanTaskV1::flow_approval(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            "approve",
            "Deploy",
            json!({}),
        );
        assert!(task
            .resolve(HumanTaskActionV1::Retry, None, "operator")
            .is_err());
        task.resolve(HumanTaskActionV1::Approve, Some(" reviewed "), "operator")
            .expect("resolve task");
        assert_eq!(task.status, HumanTaskStatusV1::Completed);
        assert_eq!(task.revision, 2);
        assert_eq!(
            task.resolution
                .as_ref()
                .and_then(|value| value.note.as_deref()),
            Some("reviewed")
        );
    }
}
