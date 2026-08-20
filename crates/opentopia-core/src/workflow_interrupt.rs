//! Durable human-interrupt protocol for resumable Workflow Agent nodes.
//!
//! The Workflow Runtime owns checkpoint identity while Agent Core owns the
//! continuation payload. This module is the narrow, serializable boundary
//! between them: a continuation is hashed before persistence, and every human
//! action becomes an idempotent resume command against one interrupt revision.

use crate::agent::AgentContinuation;
use crate::agent_runtime::AgentResumeSignal;
use crate::model::UserInputResponse;
use crate::model_context::content_fingerprint;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const WORKFLOW_INTERRUPT_SCHEMA_VERSION_V1: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentContinuationEnvelopeV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub content_hash: String,
    pub payload: Value,
}

impl AgentContinuationEnvelopeV1 {
    pub fn encode(continuation: &AgentContinuation) -> anyhow::Result<Self> {
        let payload = serde_json::to_value(continuation)?;
        let content_hash = continuation_payload_hash(&payload)?;
        Ok(Self {
            schema_version: WORKFLOW_INTERRUPT_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            content_hash,
            payload,
        })
    }

    pub fn decode(&self) -> anyhow::Result<AgentContinuation> {
        anyhow::ensure!(
            self.schema_version == WORKFLOW_INTERRUPT_SCHEMA_VERSION_V1,
            "unsupported Agent continuation envelope schema version {}",
            self.schema_version
        );
        anyhow::ensure!(
            continuation_payload_hash(&self.payload)? == self.content_hash,
            "Agent continuation payload hash mismatch"
        );
        serde_json::from_value(self.payload.clone()).map_err(Into::into)
    }
}

fn continuation_payload_hash(payload: &Value) -> anyhow::Result<String> {
    Ok(content_fingerprint(&serde_json::to_vec(payload)?))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowInterruptKindV1 {
    Approval,
    InputRequest,
    ExternalAction,
    EffectReconciliation,
    ResumeRetry,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowNodeInterruptV1 {
    pub id: Uuid,
    pub revision: u32,
    pub kind: WorkflowInterruptKindV1,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub payload: Value,
    pub continuation: AgentContinuationEnvelopeV1,
    pub tool_calls: u32,
    #[serde(default)]
    pub transcript: Vec<crate::flow_runtime::FlowTranscriptEntryV1>,
    pub created_at: DateTime<Utc>,
}

impl FlowNodeInterruptV1 {
    pub fn new(
        kind: WorkflowInterruptKindV1,
        title: impl Into<String>,
        description: impl Into<String>,
        payload: Value,
        continuation: &AgentContinuation,
        tool_calls: u32,
        transcript: Vec<crate::flow_runtime::FlowTranscriptEntryV1>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            id: Uuid::new_v4(),
            revision: 1,
            kind,
            title: title.into(),
            description: description.into(),
            payload,
            continuation: AgentContinuationEnvelopeV1::encode(continuation)?,
            tool_calls,
            transcript,
            created_at: Utc::now(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInterruptRequestV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub revision: u32,
    pub checkpoint_id: Uuid,
    pub superstep: u32,
    pub node_id: String,
    pub node_run_id: Uuid,
    pub kind: WorkflowInterruptKindV1,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub payload: Value,
    pub continuation: AgentContinuationEnvelopeV1,
    pub tool_calls: u32,
    #[serde(default)]
    pub transcript: Vec<crate::flow_runtime::FlowTranscriptEntryV1>,
    pub created_at: DateTime<Utc>,
}

impl WorkflowInterruptRequestV1 {
    pub fn at_checkpoint(
        checkpoint_id: Uuid,
        superstep: u32,
        node_id: impl Into<String>,
        node_run_id: Uuid,
        interrupt: FlowNodeInterruptV1,
    ) -> Self {
        Self {
            schema_version: WORKFLOW_INTERRUPT_SCHEMA_VERSION_V1,
            id: interrupt.id,
            revision: interrupt.revision,
            checkpoint_id,
            superstep,
            node_id: node_id.into(),
            node_run_id,
            kind: interrupt.kind,
            title: interrupt.title,
            description: interrupt.description,
            payload: interrupt.payload,
            continuation: interrupt.continuation,
            tool_calls: interrupt.tool_calls,
            transcript: interrupt.transcript,
            created_at: interrupt.created_at,
        }
    }

    pub fn resume_retry(
        previous: &WorkflowInterruptRequestV1,
        command: &FlowResumeCommandV1,
        error: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: WORKFLOW_INTERRUPT_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            revision: 1,
            checkpoint_id: previous.checkpoint_id,
            superstep: previous.superstep,
            node_id: previous.node_id.clone(),
            node_run_id: previous.node_run_id,
            kind: WorkflowInterruptKindV1::ResumeRetry,
            title: "恢复中断，需要人工重试".to_string(),
            description:
                "Agent continuation 在恢复期间遇到可中断错误。确认后将再次恢复同一 continuation，不会重新执行节点。"
                    .to_string(),
            payload: serde_json::json!({
                "error": error.into(),
                "previousResumeCommandId": command.id,
                "resumeSignal": command.signal,
            }),
            continuation: previous.continuation.clone(),
            tool_calls: previous.tool_calls,
            transcript: previous.transcript.clone(),
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FlowResumeSignalV1 {
    Approval {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_id: Option<Uuid>,
        approved: bool,
    },
    UserInput {
        request_id: Uuid,
        response: UserInputResponse,
    },
    ExternalAction {
        observation: String,
    },
}

impl FlowResumeSignalV1 {
    pub fn into_agent_signal(self) -> AgentResumeSignal {
        match self {
            Self::Approval {
                approval_id,
                approved,
            } => AgentResumeSignal::Approval {
                approval_id,
                approved,
            },
            Self::UserInput {
                request_id,
                response,
            } => AgentResumeSignal::UserInput {
                request_id,
                response,
            },
            Self::ExternalAction { observation } => {
                AgentResumeSignal::ExternalAction { observation }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowResumeCommandV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub idempotency_key: String,
    pub interrupt_id: Uuid,
    pub expected_interrupt_revision: u32,
    pub signal: FlowResumeSignalV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub issued_by: String,
    pub issued_at: DateTime<Utc>,
}

impl FlowResumeCommandV1 {
    pub fn new(
        task_id: Uuid,
        idempotency_key: impl Into<String>,
        interrupt: &WorkflowInterruptRequestV1,
        signal: FlowResumeSignalV1,
        note: Option<&str>,
        issued_by: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let idempotency_key = idempotency_key.into();
        anyhow::ensure!(
            !idempotency_key.trim().is_empty(),
            "resume idempotency key cannot be empty"
        );
        Ok(Self {
            schema_version: WORKFLOW_INTERRUPT_SCHEMA_VERSION_V1,
            id: Uuid::new_v5(&task_id, idempotency_key.as_bytes()),
            idempotency_key,
            interrupt_id: interrupt.id,
            expected_interrupt_revision: interrupt.revision,
            signal,
            note: note
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            issued_by: issued_by.into(),
            issued_at: Utc::now(),
        })
    }

    pub fn validates(&self, interrupt: &WorkflowInterruptRequestV1) -> bool {
        self.interrupt_id == interrupt.id && self.expected_interrupt_revision == interrupt.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentContinuationState;
    use crate::model::CollaborationMode;
    use crate::policy::PermissionMode;
    use serde_json::json;

    fn continuation() -> AgentContinuation {
        AgentContinuation {
            thread_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            invocation_id: 1,
            user_message_id: Uuid::new_v4(),
            workspace_root: std::env::temp_dir(),
            context_summary: None,
            conversation: Vec::new(),
            permission_mode: PermissionMode::FullAccess,
            execution_authority: None,
            context_budget: None,
            rollout_budget: None,
            model_context: Default::default(),
            collaboration_mode: CollaborationMode::Default,
            goal: None,
            state: AgentContinuationState::Provider {
                model_user_message: "continue".to_string(),
                model_user_content: Vec::new(),
                tool_candidates: Vec::new(),
                provider_tool_calls: Vec::new(),
                provider_tool_results: Vec::new(),
                pending_tool_calls: Vec::new(),
                compacted_tool_history: String::new(),
                provider_response_items: Vec::new(),
                model_rounds: 1,
                rollout_reviews: 0,
                runtime_state: Default::default(),
                branch_developer_instructions: None,
                provider_compatibility_hash: String::new(),
            },
        }
    }

    #[test]
    fn continuation_envelope_detects_mutation() {
        let mut envelope = AgentContinuationEnvelopeV1::encode(&continuation()).unwrap();
        assert_eq!(envelope.decode().unwrap().invocation_id, 1);
        envelope.payload["invocationId"] = json!(2);
        assert!(envelope.decode().is_err());
    }

    #[test]
    fn resume_command_id_is_stable_for_one_human_action() {
        let node = FlowNodeInterruptV1::new(
            WorkflowInterruptKindV1::Approval,
            "Approve",
            "Review",
            json!({}),
            &continuation(),
            0,
            Vec::new(),
        )
        .unwrap();
        let interrupt = WorkflowInterruptRequestV1::at_checkpoint(
            Uuid::new_v4(),
            1,
            "agent",
            Uuid::new_v4(),
            node,
        );
        let task_id = Uuid::new_v4();
        let first = FlowResumeCommandV1::new(
            task_id,
            "same-command",
            &interrupt,
            FlowResumeSignalV1::Approval {
                approval_id: None,
                approved: true,
            },
            None,
            "operator",
        )
        .unwrap();
        let second = FlowResumeCommandV1::new(
            task_id,
            "same-command",
            &interrupt,
            FlowResumeSignalV1::Approval {
                approval_id: None,
                approved: true,
            },
            None,
            "operator",
        )
        .unwrap();
        assert_eq!(first.id, second.id);
        assert!(first.validates(&interrupt));
    }
}
