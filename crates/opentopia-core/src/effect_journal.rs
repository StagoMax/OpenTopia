use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Durable boundary around one model or tool side effect. The journal records
/// intent before execution and the observed outcome afterwards; it does not
/// claim exactly-once execution for external systems.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    ModelRequest,
    ToolCall,
    Approval,
    Finalization,
}

impl EffectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ModelRequest => "model_request",
            Self::ToolCall => "tool_call",
            Self::Approval => "approval",
            Self::Finalization => "finalization",
        }
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "model_request" => Ok(Self::ModelRequest),
            "tool_call" => Ok(Self::ToolCall),
            "approval" => Ok(Self::Approval),
            "finalization" => Ok(Self::Finalization),
            other => anyhow::bail!("unknown effect kind: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    Prepared,
    Running,
    Succeeded,
    Failed,
    Indeterminate,
}

impl EffectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Indeterminate => "indeterminate",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed)
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "indeterminate" => Ok(Self::Indeterminate),
            other => anyhow::bail!("unknown effect status: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectSideEffectClass {
    None,
    Workspace,
    External,
    Unknown,
}

impl EffectSideEffectClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Workspace => "workspace",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "workspace" => Ok(Self::Workspace),
            "external" => Ok(Self::External),
            "unknown" => Ok(Self::Unknown),
            other => anyhow::bail!("unknown effect side-effect class: {other}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectIntent {
    pub thread_id: Uuid,
    pub turn_id: Uuid,
    pub agent_path: String,
    /// Stable across retries of the same logical action.
    pub idempotency_key: String,
    pub kind: EffectKind,
    pub operation: String,
    /// Fingerprint of the exact model request or tool arguments.
    pub input_hash: String,
    #[serde(default)]
    pub input: Value,
    pub side_effect_class: EffectSideEffectClass,
    pub idempotent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectJournalRecord {
    pub effect_id: Uuid,
    pub thread_id: Uuid,
    pub turn_id: Uuid,
    pub agent_path: String,
    pub idempotency_key: String,
    pub kind: EffectKind,
    pub operation: String,
    pub input_hash: String,
    pub input: Value,
    pub result: Option<Value>,
    pub status: EffectStatus,
    pub side_effect_class: EffectSideEffectClass,
    pub idempotent: bool,
    pub attempt: u32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl EffectJournalRecord {
    pub fn requires_reconciliation(&self) -> bool {
        self.status == EffectStatus::Indeterminate
            && self.side_effect_class != EffectSideEffectClass::None
            && !self.idempotent
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EffectJournalError {
    #[error("effect idempotency key cannot be empty")]
    EmptyIdempotencyKey,
    #[error("effect operation cannot be empty")]
    EmptyOperation,
    #[error("effect input hash cannot be empty")]
    EmptyInputHash,
    #[error("idempotency key {key} was reused with a different operation or input")]
    IdempotencyConflict { key: String },
    #[error("effect {effect_id} cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        effect_id: Uuid,
        from: EffectStatus,
        to: EffectStatus,
    },
}

pub fn validate_effect_intent(intent: &EffectIntent) -> Result<(), EffectJournalError> {
    if intent.idempotency_key.trim().is_empty() {
        return Err(EffectJournalError::EmptyIdempotencyKey);
    }
    if intent.operation.trim().is_empty() {
        return Err(EffectJournalError::EmptyOperation);
    }
    if intent.input_hash.trim().is_empty() {
        return Err(EffectJournalError::EmptyInputHash);
    }
    Ok(())
}

pub fn valid_effect_transition(from: EffectStatus, to: EffectStatus) -> bool {
    matches!(
        (from, to),
        (EffectStatus::Prepared, EffectStatus::Running)
            | (EffectStatus::Prepared, EffectStatus::Failed)
            | (EffectStatus::Running, EffectStatus::Succeeded)
            | (EffectStatus::Running, EffectStatus::Failed)
            | (EffectStatus::Running, EffectStatus::Indeterminate)
            | (EffectStatus::Indeterminate, EffectStatus::Running)
            | (EffectStatus::Indeterminate, EffectStatus::Succeeded)
            | (EffectStatus::Indeterminate, EffectStatus::Failed)
            | (EffectStatus::Failed, EffectStatus::Running)
    ) || from == to
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_idempotent_external_effect_requires_reconciliation_after_uncertain_exit() {
        let now = Utc::now();
        let record = EffectJournalRecord {
            effect_id: Uuid::new_v4(),
            thread_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            agent_path: "/root".to_string(),
            idempotency_key: "turn/tool/call".to_string(),
            kind: EffectKind::ToolCall,
            operation: "send_message".to_string(),
            input_hash: "abc".to_string(),
            input: Value::Null,
            result: None,
            status: EffectStatus::Indeterminate,
            side_effect_class: EffectSideEffectClass::External,
            idempotent: false,
            attempt: 1,
            error: None,
            created_at: now,
            started_at: Some(now),
            completed_at: None,
            updated_at: now,
        };
        assert!(record.requires_reconciliation());
    }

    #[test]
    fn terminal_effects_cannot_restart_without_an_explicit_new_record() {
        assert!(!valid_effect_transition(
            EffectStatus::Succeeded,
            EffectStatus::Running
        ));
        assert!(valid_effect_transition(
            EffectStatus::Indeterminate,
            EffectStatus::Running
        ));
        assert!(valid_effect_transition(
            EffectStatus::Indeterminate,
            EffectStatus::Succeeded
        ));
    }
}
