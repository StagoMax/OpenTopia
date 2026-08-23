use crate::enterprise_connection_grants::ExecutionConnectionOperationV1;
use crate::model_context::content_fingerprint;
use crate::workflow::{WorkflowDeploymentStatusV1, WorkflowDeploymentV1};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
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
    Webhook {
        trigger_id: Uuid,
        token_ref: String,
    },
    Schedule {
        trigger_id: Uuid,
        interval_seconds: u32,
        next_fire_at: DateTime<Utc>,
    },
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
    Webhook {
        endpoint: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential_ref: Option<String>,
    },
    ConnectionOperation {
        operation: ExecutionConnectionOperationV1,
    },
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
pub enum WorkflowReleaseStatusV1 {
    Active,
    Disabled,
}

impl WorkflowReleaseStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowReleaseV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub revision: u32,
    pub release_key: String,
    pub environment: String,
    pub thread_id: Uuid,
    pub status: WorkflowReleaseStatusV1,
    pub trigger: WorkflowTriggerSpecV1,
    #[serde(default)]
    pub ingress_policy: WorkflowIngressPolicyV1,
    pub primary_deployment_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canary_deployment_id: Option<Uuid>,
    pub canary_percent: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_primary_deployment_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
}

impl WorkflowReleaseV1 {
    pub fn new(
        release_key: impl Into<String>,
        environment: impl Into<String>,
        thread_id: Uuid,
        deployment: &WorkflowDeploymentV1,
        trigger: WorkflowTriggerSpecV1,
        created_by: impl Into<String>,
    ) -> anyhow::Result<Self> {
        Self::new_with_ingress_policy(
            release_key,
            environment,
            thread_id,
            deployment,
            trigger,
            WorkflowIngressPolicyV1::Immediate,
            created_by,
        )
    }

    pub fn new_with_ingress_policy(
        release_key: impl Into<String>,
        environment: impl Into<String>,
        thread_id: Uuid,
        deployment: &WorkflowDeploymentV1,
        trigger: WorkflowTriggerSpecV1,
        ingress_policy: WorkflowIngressPolicyV1,
        created_by: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let release_key = release_key.into().trim().to_string();
        let environment = environment.into().trim().to_string();
        let created_by = created_by.into().trim().to_string();
        validate_bounded_label(&release_key, "releaseKey")?;
        validate_bounded_label(&environment, "environment")?;
        validate_bounded_label(&created_by, "createdBy")?;
        trigger.validate()?;
        anyhow::ensure!(
            deployment.status == WorkflowDeploymentStatusV1::Active,
            "primary deployment must be active"
        );
        anyhow::ensure!(
            deployment.environment == environment,
            "release and deployment environments must match"
        );
        let now = Utc::now();
        Ok(Self {
            schema_version: WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1,
            id: Uuid::new_v4(),
            revision: 1,
            release_key,
            environment,
            thread_id,
            status: WorkflowReleaseStatusV1::Active,
            trigger,
            ingress_policy,
            primary_deployment_id: deployment.id,
            canary_deployment_id: None,
            canary_percent: 0,
            previous_primary_deployment_id: None,
            created_at: now,
            updated_at: now,
            created_by,
        })
    }

    pub fn select_deployment(&self, idempotency_key: &str) -> anyhow::Result<Uuid> {
        anyhow::ensure!(
            self.status == WorkflowReleaseStatusV1::Active,
            "Workflow release is disabled"
        );
        let Some(canary) = self.canary_deployment_id else {
            return Ok(self.primary_deployment_id);
        };
        let mut hasher = Sha256::new();
        hasher.update(self.id.as_bytes());
        hasher.update(idempotency_key.as_bytes());
        let digest = hasher.finalize();
        let bucket = u16::from_be_bytes([digest[0], digest[1]]) % 100;
        Ok(if bucket < u16::from(self.canary_percent) {
            canary
        } else {
            self.primary_deployment_id
        })
    }

    pub fn set_canary(
        &mut self,
        deployment: &WorkflowDeploymentV1,
        percent: u8,
    ) -> anyhow::Result<()> {
        anyhow::ensure!((1..=99).contains(&percent), "canaryPercent must be 1..99");
        anyhow::ensure!(
            deployment.status == WorkflowDeploymentStatusV1::Active
                && deployment.environment == self.environment,
            "canary deployment must be active in the release environment"
        );
        anyhow::ensure!(
            deployment.id != self.primary_deployment_id,
            "canary deployment must differ from the primary"
        );
        self.canary_deployment_id = Some(deployment.id);
        self.canary_percent = percent;
        self.touch();
        Ok(())
    }

    pub fn promote_canary(&mut self) -> anyhow::Result<()> {
        let canary = self
            .canary_deployment_id
            .take()
            .ok_or_else(|| anyhow::anyhow!("release has no canary to promote"))?;
        self.previous_primary_deployment_id = Some(self.primary_deployment_id);
        self.primary_deployment_id = canary;
        self.canary_percent = 0;
        self.touch();
        Ok(())
    }

    pub fn rollback(&mut self) -> anyhow::Result<()> {
        let previous = self
            .previous_primary_deployment_id
            .take()
            .ok_or_else(|| anyhow::anyhow!("release has no previous primary to restore"))?;
        let replaced = self.primary_deployment_id;
        self.primary_deployment_id = previous;
        self.previous_primary_deployment_id = Some(replaced);
        self.canary_deployment_id = None;
        self.canary_percent = 0;
        self.touch();
        Ok(())
    }

    pub fn disable(&mut self) {
        self.status = WorkflowReleaseStatusV1::Disabled;
        self.touch();
    }

    pub fn touch(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerInvocationStatusV1 {
    Accepted,
    Started,
    Failed,
}

impl WorkflowTriggerInvocationStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Started => "started",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTriggerInvocationV1 {
    pub schema_version: u16,
    pub id: Uuid,
    pub release_id: Uuid,
    pub trigger_id: Uuid,
    pub idempotency_key: String,
    pub deployment_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_run_id: Option<Uuid>,
    pub status: WorkflowTriggerInvocationStatusV1,
    pub input_hash: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkflowTriggerInvocationV1 {
    pub fn accepted(
        release: &WorkflowReleaseV1,
        idempotency_key: impl Into<String>,
        deployment_id: Uuid,
        input: &Value,
    ) -> anyhow::Result<Self> {
        let idempotency_key = idempotency_key.into().trim().to_string();
        validate_bounded_text(&idempotency_key, "idempotencyKey", 256)?;
        let trigger_id = release
            .trigger
            .trigger_id()
            .unwrap_or_else(|| Uuid::new_v5(&release.id, b"manual-trigger"));
        let now = Utc::now();
        Ok(Self {
            schema_version: WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1,
            id: Uuid::new_v5(&release.id, idempotency_key.as_bytes()),
            release_id: release.id,
            trigger_id,
            idempotency_key,
            deployment_id,
            flow_run_id: None,
            status: WorkflowTriggerInvocationStatusV1::Accepted,
            input_hash: content_fingerprint(&serde_json::to_vec(input)?),
            input: input.clone(),
            error: None,
            created_at: now,
            updated_at: now,
        })
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
    pub deployment_id: Uuid,
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
    pub fn pending(run_id: Uuid, deployment_id: Uuid, output_kind: &str) -> Self {
        let now = Utc::now();
        Self {
            schema_version: WORKFLOW_AUTOMATION_SCHEMA_VERSION_V1,
            id: Uuid::new_v5(&run_id, b"workflow-output-delivery"),
            revision: 1,
            run_id,
            deployment_id,
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
    pub deployment_id: Uuid,
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
        deployment_id: Uuid,
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
            deployment_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enterprise::CapabilityProjection;
    use crate::flow::{FlowBudgetV1, GraphDefinitionV1};
    use crate::workflow::CompiledWorkflowV1;
    use std::collections::BTreeMap;

    fn deployment(environment: &str) -> WorkflowDeploymentV1 {
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
        WorkflowDeploymentV1::new("Deployment", environment, compiled, "tester").unwrap()
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
        assert_eq!(release.primary_deployment_id, canary.id);
        release.rollback().unwrap();
        assert_eq!(release.primary_deployment_id, primary.id);
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
}
