use super::{AgentThreadId, AgentTurnId, CollaborationSessionId};
use crate::{AgentResumeSignal, UserInputResponse};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Commands at the whole-Agent-Run boundary.
///
/// Implementations may apply process-wide concurrency limits and queueing, but
/// must hand an admitted run to the ordinary AgentTurnDriver. They must not
/// implement model rounds, tool scheduling, or continuation state themselves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentRunCommand {
    Start {
        session_id: CollaborationSessionId,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
    },
    Resume {
        session_id: CollaborationSessionId,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        invocation_id: u64,
        signal: AgentRunResumeSignal,
    },
    Cancel {
        session_id: CollaborationSessionId,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentRunResumeSignal {
    Approval {
        approval_id: Option<uuid::Uuid>,
        approved: bool,
    },
    UserInput {
        request_id: uuid::Uuid,
        response: UserInputResponse,
    },
    ExternalAction {
        observation: String,
    },
}

impl From<AgentRunResumeSignal> for AgentResumeSignal {
    fn from(value: AgentRunResumeSignal) -> Self {
        match value {
            AgentRunResumeSignal::Approval {
                approval_id,
                approved,
            } => Self::Approval {
                approval_id,
                approved,
            },
            AgentRunResumeSignal::UserInput {
                request_id,
                response,
            } => Self::UserInput {
                request_id,
                response,
            },
            AgentRunResumeSignal::ExternalAction { observation } => {
                Self::ExternalAction { observation }
            }
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentRunSchedulerError {
    #[error("agent run queue is unavailable: {0}")]
    Unavailable(String),
    #[error("agent run command was rejected: {0}")]
    Rejected(String),
}

#[async_trait]
pub trait AgentRunScheduler: Send + Sync {
    async fn submit(&self, command: AgentRunCommand) -> Result<(), AgentRunSchedulerError>;
}
