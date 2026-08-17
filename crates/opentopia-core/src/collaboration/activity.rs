use super::{AgentThreadId, AgentTurnId, AgentTurnStatus, CollaborationSessionId};
use crate::model::{AgentEvent, AgentEventPayload, ModelContentPart, ToolResult};
use crate::provider::redact_model_observation;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

const MAX_REASONING_TAIL_CHARS: usize = 16_000;
const MAX_TOOL_RESULT_CHARS: usize = 32_000;
const MAX_ACTIVITY_EVENTS: usize = 64;
const TOOL_INPUT_PREVIEW_CHARS: usize = 2_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityQuery {
    pub after_cursor: Option<i64>,
    pub reasoning_tail_chars: usize,
    pub tool_result_chars: usize,
    pub event_limit: usize,
}

impl Default for ActivityQuery {
    fn default() -> Self {
        Self {
            after_cursor: None,
            reasoning_tail_chars: 2_000,
            tool_result_chars: 4_000,
            event_limit: 12,
        }
    }
}

impl ActivityQuery {
    fn bounded(&self) -> Self {
        Self {
            after_cursor: self.after_cursor,
            reasoning_tail_chars: self.reasoning_tail_chars.clamp(1, MAX_REASONING_TAIL_CHARS),
            tool_result_chars: self.tool_result_chars.clamp(1, MAX_TOOL_RESULT_CHARS),
            event_limit: self.event_limit.clamp(1, MAX_ACTIVITY_EVENTS),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivityWindow {
    pub agent_thread_id: AgentThreadId,
    pub agent_turn_id: AgentTurnId,
    pub turn_status: AgentTurnStatus,
    pub model_round: Option<usize>,
    pub cursor: i64,
    pub reasoning_tail: Option<String>,
    pub recent_events: Vec<ActivityEvent>,
    pub recent_tool_results: Vec<ToolResultProjection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub seq: i64,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<ActivityEventDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEventDetails {
    ModelRound {
        round: usize,
    },
    ToolCallStarted {
        invocation_id: Uuid,
        tool_name: String,
        input_preview: Value,
    },
    ToolCallFinished {
        invocation_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    Waiting {
        reason: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultKind {
    Text,
    Json,
    Resource,
    Binary,
    Mixed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultProjection {
    pub invocation_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub kind: ToolResultKind,
    pub preview: Value,
    pub truncated: bool,
    pub result_ref: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AgentActivityReader;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentActivitySourceError {
    #[error("agent activity source is unavailable: {0}")]
    Unavailable(String),
}

/// Durable event projection plus a wake-up mechanism.
///
/// Correctness comes from the event cursor read, not from the notifier. A
/// notifier may coalesce or lose wake-ups as long as implementations re-check
/// durable events before and after waiting.
#[async_trait]
pub trait AgentActivitySource: Send + Sync {
    async fn read_activity(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        turn_status: AgentTurnStatus,
        query: ActivityQuery,
    ) -> Result<AgentActivityWindow, AgentActivitySourceError>;

    /// Returns the AgentThread whose durable event or terminal cursor changed.
    /// `None` means the bounded wait timed out without a relevant change.
    async fn wait_for_change(
        &self,
        session_id: CollaborationSessionId,
        target: Option<AgentThreadId>,
        after_cursor: Option<i64>,
        timeout: Duration,
    ) -> Result<Option<AgentThreadId>, AgentActivitySourceError>;
}

impl AgentActivityReader {
    pub fn read(
        &self,
        agent_thread_id: AgentThreadId,
        agent_turn_id: AgentTurnId,
        turn_status: AgentTurnStatus,
        events: &[AgentEvent],
        query: &ActivityQuery,
    ) -> AgentActivityWindow {
        let query = query.bounded();
        let mut scoped = events
            .iter()
            .filter(|event| {
                event.thread_id == agent_thread_id.as_uuid()
                    && event.turn_id == Some(agent_turn_id.as_uuid())
            })
            .collect::<Vec<_>>();
        scoped.sort_by_key(|event| event.seq);

        let cursor = scoped.last().map_or(0, |event| event.seq);
        let (model_round, round_boundary) = current_model_round(&scoped);
        let reasoning_start = query
            .after_cursor
            .unwrap_or(i64::MIN)
            .max(round_boundary.unwrap_or(i64::MIN));
        let reasoning = scoped
            .iter()
            .filter(|event| event.seq > reasoning_start)
            .filter_map(|event| match &event.payload {
                AgentEventPayload::ReasoningDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        let reasoning_tail =
            (!reasoning.is_empty()).then(|| tail_chars(&reasoning, query.reasoning_tail_chars));

        let visible = scoped
            .iter()
            .copied()
            .filter(|event| query.after_cursor.is_none_or(|cursor| event.seq > cursor))
            .collect::<Vec<_>>();
        let tool_names = tool_names_by_invocation(&scoped);
        let mut recent_events = visible
            .iter()
            .filter_map(|event| project_event(event, &tool_names))
            .collect::<Vec<_>>();
        if recent_events.len() > query.event_limit {
            recent_events.drain(0..recent_events.len() - query.event_limit);
        }

        let mut recent_tool_results = visible
            .iter()
            .filter_map(|event| match &event.payload {
                AgentEventPayload::ToolCallFinished { result } => Some(project_tool_result(
                    result,
                    tool_names.get(&result.call_id).cloned(),
                    query.tool_result_chars,
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        if recent_tool_results.len() > query.event_limit {
            recent_tool_results.drain(0..recent_tool_results.len() - query.event_limit);
        }

        AgentActivityWindow {
            agent_thread_id,
            agent_turn_id,
            turn_status,
            model_round,
            cursor,
            reasoning_tail,
            recent_events,
            recent_tool_results,
        }
    }
}

fn current_model_round(events: &[&AgentEvent]) -> (Option<usize>, Option<i64>) {
    events.iter().fold((None, None), |current, event| {
        let round = match &event.payload {
            AgentEventPayload::ModelContextBuilt { round, .. }
            | AgentEventPayload::ModelRequest { round, .. }
            | AgentEventPayload::ProviderRequestSent { round, .. }
            | AgentEventPayload::ProviderRequestRetried { round, .. }
            | AgentEventPayload::ProviderResponseReceived { round, .. } => Some(*round),
            _ => None,
        };
        round.map_or(current, |round| (Some(round), Some(event.seq)))
    })
}

fn tool_names_by_invocation(events: &[&AgentEvent]) -> HashMap<Uuid, String> {
    events
        .iter()
        .filter_map(|event| match &event.payload {
            AgentEventPayload::ToolCallStarted { call } => Some((call.id, call.name.clone())),
            _ => None,
        })
        .collect()
}

fn project_event(event: &AgentEvent, tool_names: &HashMap<Uuid, String>) -> Option<ActivityEvent> {
    let details = match &event.payload {
        AgentEventPayload::ModelContextBuilt { round, .. }
        | AgentEventPayload::ModelRequest { round, .. } => {
            Some(ActivityEventDetails::ModelRound { round: *round })
        }
        AgentEventPayload::ToolCallStarted { call } => {
            Some(ActivityEventDetails::ToolCallStarted {
                invocation_id: call.id,
                tool_name: call.name.clone(),
                input_preview: bounded_value(
                    redact_model_observation(&call.input),
                    TOOL_INPUT_PREVIEW_CHARS,
                )
                .0,
            })
        }
        AgentEventPayload::ToolCallFinished { result } => {
            Some(ActivityEventDetails::ToolCallFinished {
                invocation_id: result.call_id,
                tool_name: tool_names.get(&result.call_id).cloned(),
            })
        }
        AgentEventPayload::TurnSuspended { reason, .. } => Some(ActivityEventDetails::Waiting {
            reason: tail_chars(reason, 1_000),
        }),
        AgentEventPayload::TurnAwaitingInput { request_id } => {
            Some(ActivityEventDetails::Waiting {
                reason: format!("waiting for structured input {request_id}"),
            })
        }
        AgentEventPayload::Error { message } => Some(ActivityEventDetails::Error {
            message: tail_chars(message, 2_000),
        }),
        AgentEventPayload::TurnStarted { .. }
        | AgentEventPayload::TurnFinished { .. }
        | AgentEventPayload::TurnCancelled { .. } => None,
        _ => return None,
    };
    Some(ActivityEvent {
        seq: event.seq,
        kind: event.kind().to_string(),
        created_at: event.created_at,
        details,
    })
}

fn project_tool_result(
    result: &ToolResult,
    tool_name: Option<String>,
    max_chars: usize,
) -> ToolResultProjection {
    let (kind, value) = result_value(result);
    let (preview, truncated) = bounded_value(redact_model_observation(&value), max_chars);
    ToolResultProjection {
        invocation_id: result.call_id,
        tool_name,
        kind,
        preview,
        truncated,
        result_ref: format!("tool-result:{}", result.call_id),
    }
}

fn result_value(result: &ToolResult) -> (ToolResultKind, Value) {
    if !result.output.is_empty() {
        return (ToolResultKind::Text, Value::String(result.output.clone()));
    }
    if result.content.len() == 1 {
        return match &result.content[0] {
            ModelContentPart::Text { text } => (ToolResultKind::Text, Value::String(text.clone())),
            ModelContentPart::Json { value } => (ToolResultKind::Json, value.clone()),
            ModelContentPart::Resource {
                uri,
                content_type,
                name,
            } => (
                ToolResultKind::Resource,
                json!({ "uri": uri, "contentType": content_type, "name": name }),
            ),
            ModelContentPart::Image {
                content_type, data, ..
            } => (
                ToolResultKind::Binary,
                json!({ "contentType": content_type, "bytes": data.len() }),
            ),
        };
    }
    let contains_binary = result
        .content
        .iter()
        .any(|part| matches!(part, ModelContentPart::Image { .. }));
    let summary = result
        .content
        .iter()
        .map(|part| match part {
            ModelContentPart::Text { text } => json!({ "type": "text", "text": text }),
            ModelContentPart::Json { value } => json!({ "type": "json", "value": value }),
            ModelContentPart::Resource {
                uri,
                content_type,
                name,
            } => json!({
                "type": "resource",
                "uri": uri,
                "contentType": content_type,
                "name": name,
            }),
            ModelContentPart::Image {
                content_type, data, ..
            } => json!({
                "type": "image",
                "contentType": content_type,
                "bytes": data.len(),
            }),
        })
        .collect::<Vec<_>>();
    (
        if contains_binary {
            ToolResultKind::Binary
        } else {
            ToolResultKind::Mixed
        },
        Value::Array(summary),
    )
}

fn bounded_value(value: Value, max_chars: usize) -> (Value, bool) {
    let rendered = match &value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(&value).unwrap_or_default(),
    };
    if rendered.chars().count() <= max_chars {
        return (value, false);
    }
    (
        Value::String(format!(
            "[truncated; showing last {max_chars} characters]\n{}",
            tail_chars(&rendered, max_chars)
        )),
        true,
    )
}

fn tail_chars(value: &str, limit: usize) -> String {
    let count = value.chars().count();
    if count <= limit {
        return value.to_string();
    }
    value.chars().skip(count - limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AgentEvent, AgentEventPayload, ToolCall, ToolResult};
    use serde_json::json;

    fn event(
        thread_id: AgentThreadId,
        turn_id: AgentTurnId,
        seq: i64,
        payload: AgentEventPayload,
    ) -> AgentEvent {
        AgentEvent::new(thread_id.as_uuid(), Some(turn_id.as_uuid()), seq, payload)
    }

    #[test]
    fn activity_window_joins_lifecycle_events_with_actual_tool_result() {
        let thread_id = AgentThreadId::new();
        let turn_id = AgentTurnId::new();
        let call = ToolCall::new("cargo_test", json!({ "package": "opentopia-core" }));
        let events = vec![
            event(
                thread_id,
                turn_id,
                1,
                AgentEventPayload::ModelRequest {
                    request_id: Uuid::new_v4(),
                    round: 3,
                    request: Value::Null,
                },
            ),
            event(
                thread_id,
                turn_id,
                2,
                AgentEventPayload::ReasoningDelta {
                    text: "准备运行测试。".to_string(),
                },
            ),
            event(
                thread_id,
                turn_id,
                3,
                AgentEventPayload::ToolCallStarted { call: call.clone() },
            ),
            event(
                thread_id,
                turn_id,
                4,
                AgentEventPayload::ToolCallFinished {
                    result: ToolResult::text(call.id, "18 tests passed", json!({})),
                },
            ),
        ];

        let window = AgentActivityReader.read(
            thread_id,
            turn_id,
            AgentTurnStatus::Running,
            &events,
            &ActivityQuery::default(),
        );

        assert_eq!(window.model_round, Some(3));
        assert_eq!(window.reasoning_tail.as_deref(), Some("准备运行测试。"));
        assert_eq!(window.cursor, 4);
        assert_eq!(window.recent_tool_results.len(), 1);
        assert_eq!(
            window.recent_tool_results[0].preview,
            Value::String("18 tests passed".to_string())
        );
        assert_eq!(
            window.recent_tool_results[0].tool_name.as_deref(),
            Some("cargo_test")
        );
    }

    #[test]
    fn in_flight_tool_has_lifecycle_event_but_no_result() {
        let thread_id = AgentThreadId::new();
        let turn_id = AgentTurnId::new();
        let call = ToolCall::new("shell", json!({ "cmd": "cargo test" }));
        let events = vec![event(
            thread_id,
            turn_id,
            1,
            AgentEventPayload::ToolCallStarted { call },
        )];

        let window = AgentActivityReader.read(
            thread_id,
            turn_id,
            AgentTurnStatus::Running,
            &events,
            &ActivityQuery::default(),
        );
        assert_eq!(window.recent_events.len(), 1);
        assert!(window.recent_tool_results.is_empty());
    }

    #[test]
    fn result_projection_is_bounded_without_changing_canonical_result() {
        let thread_id = AgentThreadId::new();
        let turn_id = AgentTurnId::new();
        let call = ToolCall::new("shell", json!({}));
        let full_output = "a".repeat(200);
        let events = vec![event(
            thread_id,
            turn_id,
            1,
            AgentEventPayload::ToolCallFinished {
                result: ToolResult::text(call.id, full_output.clone(), json!({})),
            },
        )];
        let window = AgentActivityReader.read(
            thread_id,
            turn_id,
            AgentTurnStatus::Running,
            &events,
            &ActivityQuery {
                tool_result_chars: 20,
                ..ActivityQuery::default()
            },
        );

        assert!(window.recent_tool_results[0].truncated);
        assert_eq!(full_output.len(), 200);
    }
}
