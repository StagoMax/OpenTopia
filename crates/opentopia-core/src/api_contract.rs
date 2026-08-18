use crate::collaboration::AgentThreadId;
use crate::AgentEvent;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DESKTOP_STREAM_API_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopStreamKind {
    AgentEvent,
    AgentActivity,
    TerminalEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStreamEnvelope<T> {
    #[schemars(range(min = 1, max = 1))]
    pub api_version: u16,
    pub kind: DesktopStreamKind,
    pub seq: i64,
    pub data: T,
}

impl<T> DesktopStreamEnvelope<T> {
    pub fn new(kind: DesktopStreamKind, seq: i64, data: T) -> Self {
        Self {
            api_version: DESKTOP_STREAM_API_VERSION,
            kind,
            seq,
            data,
        }
    }
}

pub type AgentEventEnvelopeV1 = DesktopStreamEnvelope<AgentEvent>;
pub type AgentActivityEnvelopeV1 = DesktopStreamEnvelope<AgentActivityNotification>;
pub type TerminalEventEnvelopeV1 = DesktopStreamEnvelope<TerminalEvent>;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivityNotification {
    pub seq: i64,
    pub agent_thread_id: AgentThreadId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEventKind {
    Started,
    Stdout,
    Stderr,
    Finished,
    Cancelled,
    Error,
}

impl TerminalEventKind {
    pub fn sse_event_name(self) -> &'static str {
        match self {
            Self::Started => "terminal_started",
            Self::Stdout => "terminal_stdout",
            Self::Stderr => "terminal_stderr",
            Self::Finished => "terminal_finished",
            Self::Cancelled => "terminal_cancelled",
            Self::Error => "terminal_error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TerminalEvent {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub command_id: Uuid,
    pub seq: u64,
    pub created_at: DateTime<Utc>,
    #[serde(rename = "type")]
    pub kind: TerminalEventKind,
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub data: Option<String>,
    pub exit_code: Option<i32>,
    pub success: Option<bool>,
    pub message: Option<String>,
}
