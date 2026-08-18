use chrono::{DateTime, Utc};
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
            resolution: None,
            created_at: now,
            updated_at: now,
            resolved_at: None,
        }
    }

    pub fn resolve(
        &mut self,
        action: HumanTaskActionV1,
        note: Option<&str>,
        resolved_by: impl Into<String>,
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
        });
        self.resolved_at = Some(now);
        self.updated_at = now;
        self.revision = self.revision.saturating_add(1);
        Ok(())
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
