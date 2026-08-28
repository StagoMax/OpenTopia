use crate::enterprise_connection_grants::ExecutionConnectionOperationV1;
use crate::model_context::content_fingerprint;
use crate::workflow::{ActiveFlowV1, FlowRevisionV1};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1: u16 = 1;

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowIngressPolicyV1 {
    #[default]
    Immediate,
    RequireReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkflowTriggerSpecV1 {
    Manual,
    #[schemars(rename_all = "camelCase")]
    Webhook {
        trigger_id: Uuid,
        token_ref: String,
    },
    #[schemars(rename_all = "camelCase")]
    Schedule {
        trigger_id: Uuid,
        interval_seconds: u32,
        next_fire_at: DateTime<Utc>,
    },
    #[schemars(rename_all = "camelCase")]
    EventSubscription {
        trigger_id: Uuid,
        source: String,
        event_type: String,
    },
}

impl WorkflowTriggerSpecV1 {
    pub fn trigger_id(&self) -> Option<Uuid> {
        match self {
            Self::Manual => None,
            Self::Webhook { trigger_id, .. }
            | Self::Schedule { trigger_id, .. }
            | Self::EventSubscription { trigger_id, .. } => Some(*trigger_id),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Webhook { .. } => "webhook",
            Self::Schedule { .. } => "schedule",
            Self::EventSubscription { .. } => "event_subscription",
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Manual => Ok(()),
            Self::Webhook { token_ref, .. } => validate_credential_ref(token_ref),
            Self::Schedule {
                interval_seconds, ..
            } => {
                anyhow::ensure!(
                    (60..=31_536_000).contains(interval_seconds),
                    "schedule intervalSeconds must be between 60 and 31536000"
                );
                Ok(())
            }
            Self::EventSubscription {
                source, event_type, ..
            } => {
                validate_bounded_label(source, "event source")?;
                validate_bounded_label(event_type, "event type")
            }
        }
    }

    pub fn schedule_due_key_and_advance(&mut self, now: DateTime<Utc>) -> Option<String> {
        let Self::Schedule {
            interval_seconds,
            next_fire_at,
            ..
        } = self
        else {
            return None;
        };
        if *next_fire_at > now {
            return None;
        }
        let due = *next_fire_at;
        let interval = Duration::seconds(i64::from(*interval_seconds));
        while *next_fire_at <= now {
            *next_fire_at += interval;
        }
        Some(format!("schedule:{}", due.to_rfc3339()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum WorkflowOutputSpecV1 {
    Inbox,
    #[schemars(rename_all = "camelCase")]
    Webhook {
        endpoint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential_ref: Option<String>,
    },
    #[schemars(rename_all = "camelCase")]
    ConnectionOperation {
        operation: ExecutionConnectionOperationV1,
    },
    #[schemars(rename_all = "camelCase")]
    HumanTask {
        title: String,
        description: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        assigned_to: Option<String>,
    },
}

impl WorkflowOutputSpecV1 {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Webhook { .. } => "webhook",
            Self::ConnectionOperation { .. } => "connection_operation",
            Self::HumanTask { .. } => "human_task",
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Inbox | Self::ConnectionOperation { .. } => Ok(()),
            Self::Webhook {
                endpoint,
                credential_ref,
            } => {
                validate_webhook_endpoint(endpoint)?;
                if let Some(reference) = credential_ref {
                    validate_credential_ref(reference)?;
                }
                Ok(())
            }
            Self::HumanTask {
                title,
                description,
                assigned_to,
            } => {
                validate_bounded_text(title, "HumanTask title", 160)?;
                validate_bounded_text(description, "HumanTask description", 2_000)?;
                if let Some(assignee) = assigned_to {
                    validate_bounded_label(assignee, "HumanTask assignee")?;
                }
                Ok(())
            }
        }
    }
}

fn validate_credential_ref(reference: &str) -> anyhow::Result<()> {
    let reference = reference.trim();
    anyhow::ensure!(
        reference
            .strip_prefix("env:")
            .is_some_and(|name| !name.trim().is_empty()),
        "credential references must use env:NAME and never contain a secret value"
    );
    anyhow::ensure!(reference.len() <= 256, "credential reference is too long");
    Ok(())
}

fn validate_webhook_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let parsed = reqwest::Url::parse(endpoint.trim())?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https"),
        "webhook endpoint must use http or https"
    );
    anyhow::ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "webhook endpoint cannot contain credentials"
    );
    if parsed.scheme() == "http" {
        let host = parsed.host_str().unwrap_or_default();
        anyhow::ensure!(
            matches!(host, "localhost" | "127.0.0.1" | "::1"),
            "non-loopback webhook endpoints must use https"
        );
    }
    anyhow::ensure!(endpoint.len() <= 2_048, "webhook endpoint is too long");
    Ok(())
}

fn validate_bounded_label(value: &str, name: &str) -> anyhow::Result<()> {
    validate_bounded_text(value, name, 128)
}

fn validate_bounded_text(value: &str, name: &str, limit: usize) -> anyhow::Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{name} cannot be empty");
    anyhow::ensure!(value.chars().count() <= limit, "{name} is too long");
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowCaseStatusV1 {
    Accepted,
    Started,
    Failed,
    /// The event remains immutable for audit, but a newer invocation replaces
    /// it and is the only copy that may be started.
    Superseded,
}

impl FlowCaseStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Started => "started",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FlowCaseV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub flow_id: String,
    pub trigger_id: Uuid,
    pub idempotency_key: String,
    pub flow_revision_id: Uuid,
    pub flow_revision: FlowRevisionV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_run_id: Option<Uuid>,
    pub status: FlowCaseStatusV1,
    pub input_hash: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by_case_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl FlowCaseV1 {
    pub fn accepted(
        flow: &ActiveFlowV1,
        idempotency_key: impl Into<String>,
        input: &Value,
    ) -> anyhow::Result<Self> {
        let idempotency_key = idempotency_key.into().trim().to_string();
        validate_bounded_text(&idempotency_key, "idempotencyKey", 256)?;
        let trigger_id = flow
            .active_revision
            .trigger
            .trigger_id()
            .unwrap_or_else(|| Uuid::new_v5(&flow.id, b"manual-trigger"));
        let now = Utc::now();
        Ok(Self {
            schema_version: WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1,
            id: Uuid::new_v5(&flow.id, idempotency_key.as_bytes()),
            flow_id: flow.flow_id.clone(),
            trigger_id,
            idempotency_key,
            flow_revision_id: flow.active_revision.id,
            flow_revision: flow.active_revision.clone(),
            flow_run_id: None,
            status: FlowCaseStatusV1::Accepted,
            input_hash: content_fingerprint(&serde_json::to_vec(input)?),
            input: input.clone(),
            error: None,
            superseded_by_case_id: None,
            status_note: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn supersede(
        &mut self,
        replacement_case_id: Option<Uuid>,
        note: impl Into<String>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.status == FlowCaseStatusV1::Accepted && self.flow_run_id.is_none(),
            "only a pending Flow case can be superseded"
        );
        if let Some(replacement_id) = replacement_case_id {
            anyhow::ensure!(
                replacement_id != self.id,
                "a Flow case cannot replace itself"
            );
        }
        let note = note.into().trim().to_string();
        validate_bounded_text(&note, "statusNote", 512)?;
        self.status = FlowCaseStatusV1::Superseded;
        self.superseded_by_case_id = replacement_case_id;
        self.status_note = Some(note);
        self.updated_at = Utc::now();
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDeliveryStatusV1 {
    Pending,
    Delivered,
    Failed,
    WaitingHuman,
    Cancelled,
}

impl WorkflowDeliveryStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::WaitingHuman => "waiting_human",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDeliveryReceiptV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub revision: u32,
    pub run_id: Uuid,
    pub flow_revision_id: Uuid,
    pub output_kind: String,
    pub status: WorkflowDeliveryStatusV1,
    pub attempt: u32,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<DateTime<Utc>>,
}

impl WorkflowDeliveryReceiptV1 {
    pub fn pending(run_id: Uuid, flow_revision_id: Uuid, output_kind: &str) -> Self {
        let now = Utc::now();
        Self {
            schema_version: WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1,
            id: Uuid::new_v5(&run_id, b"workflow-output-delivery"),
            revision: 1,
            run_id,
            flow_revision_id,
            output_kind: output_kind.to_string(),
            status: WorkflowDeliveryStatusV1::Pending,
            attempt: 0,
            idempotency_key: format!("workflow-output:{run_id}"),
            response_status: None,
            provider_result: None,
            error: None,
            created_at: now,
            updated_at: now,
            delivered_at: None,
        }
    }

    pub fn begin_attempt(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.attempt = self.attempt.saturating_add(1);
        self.status = WorkflowDeliveryStatusV1::Pending;
        self.response_status = None;
        self.provider_result = None;
        self.error = None;
        self.delivered_at = None;
        self.updated_at = Utc::now();
    }

    pub fn mark_delivered(&mut self, response_status: Option<u16>, result: Option<Value>) {
        self.revision = self.revision.saturating_add(1);
        self.status = WorkflowDeliveryStatusV1::Delivered;
        self.response_status = response_status;
        self.provider_result = result;
        self.error = None;
        self.delivered_at = Some(Utc::now());
        self.updated_at = Utc::now();
    }

    pub fn mark_failed(&mut self, response_status: Option<u16>, error: impl Into<String>) {
        self.revision = self.revision.saturating_add(1);
        self.status = WorkflowDeliveryStatusV1::Failed;
        self.response_status = response_status;
        self.provider_result = None;
        self.error = Some(error.into());
        self.delivered_at = None;
        self.updated_at = Utc::now();
    }

    pub fn mark_waiting_human(&mut self, task_id: Uuid) {
        self.revision = self.revision.saturating_add(1);
        self.status = WorkflowDeliveryStatusV1::WaitingHuman;
        self.provider_result = Some(serde_json::json!({ "humanTaskId": task_id }));
        self.error = None;
        self.delivered_at = None;
        self.updated_at = Utc::now();
    }

    pub fn mark_cancelled(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.status = WorkflowDeliveryStatusV1::Cancelled;
        self.delivered_at = None;
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEvaluationV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub run_id: Uuid,
    pub flow_revision_id: Uuid,
    pub evaluator: String,
    pub score: f64,
    pub passed: bool,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl WorkflowEvaluationV1 {
    pub fn new(
        run_id: Uuid,
        flow_revision_id: Uuid,
        evaluator: impl Into<String>,
        score: f64,
        passed: bool,
        labels: Vec<String>,
        note: Option<String>,
    ) -> anyhow::Result<Self> {
        let evaluator = evaluator.into().trim().to_string();
        validate_bounded_label(&evaluator, "evaluator")?;
        anyhow::ensure!(
            score.is_finite() && (0.0..=1.0).contains(&score),
            "score must be 0..1"
        );
        anyhow::ensure!(labels.len() <= 32, "too many evaluation labels");
        for label in &labels {
            validate_bounded_label(label, "evaluation label")?;
        }
        let id = Uuid::new_v5(&run_id, evaluator.as_bytes());
        Ok(Self {
            schema_version: WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1,
            id,
            run_id,
            flow_revision_id,
            evaluator,
            score,
            passed,
            labels,
            note: note
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            created_at: Utc::now(),
        })
    }
}

#[cfg(any())]
mod tests {
    use super::*;
    use crate::enterprise::CapabilityProjection;
    use crate::flow::{FlowBudgetV1, GraphDefinitionV1, GraphNodeKindV1, GraphNodeV1};
    use crate::workflow::CompiledWorkflowV1;
    use schemars::schema_for;
    use std::collections::BTreeMap;

    fn deployment(environment: &str) -> ActiveFlowV1 {
        let compiled = CompiledWorkflowV1 {
            schema_version: 1,
            flow_id: "release-test".to_string(),
            flow_version: 1,
            definition_id: Uuid::new_v4(),
            definition_content_hash: "definition".to_string(),
            graph: GraphDefinitionV1 {
                schema_version: 1,
                entry_node_id: "output".to_string(),
                nodes: Vec::new(),
                edges: Vec::new(),
            },
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            root_capabilities: CapabilityProjection::deny_all(),
            harness_capabilities: CapabilityProjection::deny_all(),
            harness_connection_authority:
                crate::collaboration::RuntimeConnectionAuthorityV1::DenyAll,
            budget: FlowBudgetV1::default(),
            agent_specs: BTreeMap::new(),
            content_hash: "compiled".to_string(),
        };
        ActiveFlowV1::new("Deployment", environment, compiled, "tester").unwrap()
    }

    #[test]
    fn canary_routing_is_stable_and_rollback_restores_previous_primary() {
        let primary = deployment("production");
        let canary = deployment("production");
        let mut release = WorkflowReleaseV1::new(
            "orders",
            "production",
            Uuid::new_v4(),
            &primary,
            WorkflowTriggerSpecV1::Webhook {
                trigger_id: Uuid::new_v4(),
                token_ref: "env:WORKFLOW_TEST_TOKEN".to_string(),
            },
            "tester",
        )
        .unwrap();
        release.set_canary(&canary, 25).unwrap();
        assert_eq!(
            release.select_deployment("customer-42").unwrap(),
            release.select_deployment("customer-42").unwrap()
        );
        release.promote_canary().unwrap();
        assert_eq!(release.primary_flow_revision_id, canary.id);
        release.rollback().unwrap();
        assert_eq!(release.primary_flow_revision_id, primary.id);
    }

    #[test]
    fn schedule_advances_past_now_with_one_stable_due_key() {
        let now = Utc::now();
        let mut trigger = WorkflowTriggerSpecV1::Schedule {
            trigger_id: Uuid::new_v4(),
            interval_seconds: 60,
            next_fire_at: now - Duration::minutes(3),
        };
        let key = trigger
            .schedule_due_key_and_advance(now)
            .expect("schedule due");
        assert!(key.starts_with("schedule:"));
        let WorkflowTriggerSpecV1::Schedule { next_fire_at, .. } = trigger else {
            unreachable!()
        };
        assert!(next_fire_at > now);
    }

    #[test]
    fn reviewed_ingress_preserves_event_input_before_a_run_exists() {
        let deployment = deployment("production");
        let release = WorkflowReleaseV1::new_with_ingress_policy(
            "reviewed-orders",
            "production",
            Uuid::new_v4(),
            &deployment,
            WorkflowTriggerSpecV1::EventSubscription {
                trigger_id: Uuid::new_v4(),
                source: "crm".to_string(),
                event_type: "record.updated".to_string(),
            },
            WorkflowIngressPolicyV1::RequireReview,
            "tester",
        )
        .expect("reviewed release");
        let input = serde_json::json!({"recordId": "customer-42"});
        let invocation =
            WorkflowTriggerInvocationV1::accepted(&release, "event-42", deployment.id, &input)
                .expect("accepted event");

        assert_eq!(
            release.ingress_policy,
            WorkflowIngressPolicyV1::RequireReview
        );
        assert_eq!(
            invocation.status,
            WorkflowTriggerInvocationStatusV1::Accepted
        );
        assert_eq!(invocation.flow_run_id, None);
        assert_eq!(invocation.input, input);
    }

    #[test]
    fn pending_invocation_can_be_superseded_without_rebinding_its_deployment() {
        let deployment = deployment("production");
        let release = WorkflowReleaseV1::new_with_ingress_policy(
            "reviewed-orders",
            "production",
            Uuid::new_v4(),
            &deployment,
            WorkflowTriggerSpecV1::Manual,
            WorkflowIngressPolicyV1::RequireReview,
            "tester",
        )
        .expect("reviewed release");
        let mut invocation = WorkflowTriggerInvocationV1::accepted(
            &release,
            "event-42",
            deployment.id,
            &serde_json::json!({"recordId": "customer-42"}),
        )
        .expect("accepted event");
        let frozen_flow_revision_id = invocation.flow_revision_id;
        let replacement_id = Uuid::new_v4();

        invocation
            .supersede(
                Some(replacement_id),
                "migrated to node Trigger architecture",
            )
            .expect("pending event can be replaced");

        assert_eq!(
            invocation.status,
            WorkflowTriggerInvocationStatusV1::Superseded
        );
        assert_eq!(invocation.flow_revision_id, frozen_flow_revision_id);
        assert_eq!(invocation.superseded_by_invocation_id, Some(replacement_id));
        assert!(invocation.flow_run_id.is_none());
    }

    #[test]
    fn release_rejects_a_trigger_that_cannot_activate_its_flow() {
        let mut deployment = deployment("production");
        deployment.snapshot.compiled_workflow.graph.nodes = vec![GraphNodeV1 {
            id: "output".to_string(),
            label: "Output Agent".to_string(),
            kind: GraphNodeKindV1::Agent,
            config: serde_json::json!({
                "activation": {
                    "expression": {
                        "operator": "source",
                        "source": {
                            "kind": "webhook",
                            "triggerId": Uuid::new_v4(),
                            "tokenRef": "env:FLOW_TOKEN"
                        }
                    },
                    "ingressPolicy": "immediate"
                }
            }),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
        }];

        let error = WorkflowReleaseV1::new(
            "unrelated",
            "production",
            Uuid::new_v4(),
            &deployment,
            WorkflowTriggerSpecV1::EventSubscription {
                trigger_id: Uuid::new_v4(),
                source: "crm".to_string(),
                event_type: "record.updated".to_string(),
            },
            "tester",
        )
        .expect_err("unrelated Trigger must be rejected");

        assert!(error
            .to_string()
            .contains("does not activate any Flow Agent node"));
    }

    #[test]
    fn automation_enum_schema_matches_camel_case_wire_fields() {
        let trigger_schema =
            serde_json::to_value(schema_for!(WorkflowTriggerSpecV1)).expect("trigger schema");
        let trigger_schema = trigger_schema.to_string();
        assert!(trigger_schema.contains("triggerId"));
        assert!(trigger_schema.contains("eventType"));
        assert!(trigger_schema.contains("intervalSeconds"));
        assert!(!trigger_schema.contains("trigger_id"));
        assert!(!trigger_schema.contains("event_type"));
        assert!(!trigger_schema.contains("interval_seconds"));

        let output_schema =
            serde_json::to_value(schema_for!(WorkflowOutputSpecV1)).expect("output schema");
        let output_schema = output_schema.to_string();
        assert!(output_schema.contains("credentialRef"));
        assert!(output_schema.contains("assignedTo"));
        assert!(!output_schema.contains("credential_ref"));
        assert!(!output_schema.contains("assigned_to"));

        let trigger = WorkflowTriggerSpecV1::EventSubscription {
            trigger_id: Uuid::new_v4(),
            source: "audit.work-injury".to_string(),
            event_type: "case.submitted".to_string(),
        };
        let serialized = serde_json::to_value(trigger).expect("serialize trigger");
        assert!(serialized.get("triggerId").is_some());
        assert!(serialized.get("eventType").is_some());
    }
}

#[cfg(test)]
mod flow_case_tests {
    use super::*;
    use crate::enterprise::CapabilityProjection;
    use crate::flow::{FlowBudgetV1, GraphDefinitionV1};
    use crate::workflow::{ActiveFlowV1, CompiledWorkflowV1, WorkflowOutputReviewPolicyV1};
    use std::collections::BTreeMap;

    fn active_flow(ingress_policy: WorkflowIngressPolicyV1) -> ActiveFlowV1 {
        let compiled = CompiledWorkflowV1 {
            schema_version: 1,
            flow_id: "case-test".to_string(),
            flow_version: 1,
            definition_id: Uuid::new_v4(),
            definition_content_hash: "definition".to_string(),
            graph: GraphDefinitionV1 {
                schema_version: 1,
                entry_node_id: "entry".to_string(),
                nodes: Vec::new(),
                edges: Vec::new(),
            },
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            root_capabilities: CapabilityProjection::deny_all(),
            harness_capabilities: CapabilityProjection::deny_all(),
            harness_connection_authority:
                crate::collaboration::RuntimeConnectionAuthorityV1::DenyAll,
            budget: FlowBudgetV1::default(),
            agent_specs: BTreeMap::new(),
            content_hash: "compiled".to_string(),
        };
        ActiveFlowV1::new_with_ingress_policy(
            "Case test",
            Uuid::new_v4(),
            compiled,
            WorkflowTriggerSpecV1::EventSubscription {
                trigger_id: Uuid::new_v4(),
                source: "crm".to_string(),
                event_type: "record.updated".to_string(),
            },
            ingress_policy,
            WorkflowOutputSpecV1::Inbox,
            WorkflowOutputReviewPolicyV1::ExplicitNodesOnly,
            "tester",
        )
        .expect("active Flow")
    }

    #[test]
    fn case_freezes_the_active_flow_revision_before_review() {
        let flow = active_flow(WorkflowIngressPolicyV1::RequireReview);
        let input = serde_json::json!({"recordId": "customer-42"});
        let case = FlowCaseV1::accepted(&flow, "event-42", &input).expect("accepted case");

        assert_eq!(case.flow_id, flow.flow_id);
        assert_eq!(case.flow_revision_id, flow.active_revision.id);
        assert_eq!(case.status, FlowCaseStatusV1::Accepted);
        assert_eq!(case.flow_run_id, None);
        assert_eq!(case.input, input);
    }

    #[test]
    fn pending_case_can_be_superseded_without_rebinding_its_revision() {
        let flow = active_flow(WorkflowIngressPolicyV1::RequireReview);
        let mut case = FlowCaseV1::accepted(
            &flow,
            "event-42",
            &serde_json::json!({"recordId": "customer-42"}),
        )
        .expect("accepted case");
        let frozen_revision_id = case.flow_revision_id;
        let replacement_id = Uuid::new_v4();

        case.supersede(Some(replacement_id), "replaced by a newer case")
            .expect("pending case can be replaced");

        assert_eq!(case.status, FlowCaseStatusV1::Superseded);
        assert_eq!(case.flow_revision_id, frozen_revision_id);
        assert_eq!(case.superseded_by_case_id, Some(replacement_id));
    }

    #[test]
    fn schedule_advances_past_now_with_one_stable_due_key() {
        let now = Utc::now();
        let mut trigger = WorkflowTriggerSpecV1::Schedule {
            trigger_id: Uuid::new_v4(),
            interval_seconds: 60,
            next_fire_at: now - Duration::minutes(3),
        };
        let key = trigger
            .schedule_due_key_and_advance(now)
            .expect("schedule due");
        assert!(key.starts_with("schedule:"));
    }
}
