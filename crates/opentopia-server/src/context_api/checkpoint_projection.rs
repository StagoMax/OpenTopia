//! Deterministic projection of durable agent events into checkpoint fields.
//!
//! This module deliberately does not interpret free-form model text. It owns
//! only facts already represented by typed local events; semantic synthesis is
//! retained separately as an opaque string.

use super::{truncate_chars, ContextCheckpointDraft};
use opentopia_core::{
    content_fingerprint, AgentEvent, AgentEventPayload, ContextCheckpoint,
    ContextCheckpointArtifact, ContextCheckpointCommand, ContextCheckpointFact,
    ContextCheckpointFile, ContextCheckpointStep, ContextFactStatus, Message, MessagePart,
    MessageRole, ToolCall, ToolResult, TurnFileChangeKind, WorkForm, WorkItemStatus,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use uuid::Uuid;

struct PendingCommand {
    command: String,
    tool_name: String,
    started_seq: i64,
}

pub(crate) fn project_checkpoint_draft(
    messages: &[Message],
    events: &[AgentEvent],
    semantic_summary: &str,
    previous: Option<&ContextCheckpoint>,
) -> ContextCheckpointDraft {
    let latest_form = latest_work_form(events);
    let goal = latest_form
        .map(|(_, form)| form.objective.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| first_semantic_line(semantic_summary))
        .or_else(|| latest_user_text(messages))
        .or_else(|| previous.map(|checkpoint| checkpoint.goal.clone()))
        .unwrap_or_else(|| "Continue the current task from the durable event log.".to_string());

    let mut draft = ContextCheckpointDraft {
        goal,
        ..ContextCheckpointDraft::default()
    };
    if let Some((seq, form)) = latest_form {
        project_work_form(&mut draft, seq, form);
    }
    draft.commands_and_validation = project_commands(events);
    draft.workspace_state.files_changed = project_files(events);
    draft.artifacts = project_artifacts(messages);
    draft
}

pub(crate) fn local_projection_retention_percentages(
    previous: Option<&ContextCheckpoint>,
    current: &ContextCheckpoint,
) -> (usize, usize) {
    let Some(previous) = previous else {
        return (100, 100);
    };
    let previous_facts = local_projection_keys(previous, false);
    let current_facts = local_projection_keys(current, false);
    let previous_constraints = local_projection_keys(previous, true);
    let current_constraints = local_projection_keys(current, true);
    (
        retained_percent(&previous_facts, &current_facts),
        retained_percent(&previous_constraints, &current_constraints),
    )
}

fn local_projection_keys(
    checkpoint: &ContextCheckpoint,
    active_constraints_only: bool,
) -> HashSet<String> {
    let constraints = checkpoint
        .user_constraints
        .iter()
        .filter(|fact| fact.id.starts_with("work-form-constraint-"))
        .filter(|fact| !active_constraints_only || fact.status == ContextFactStatus::Active)
        .map(|fact| format!("constraint:{}", fact.id));
    if active_constraints_only {
        return constraints.collect();
    }

    let mut keys = constraints.collect::<HashSet<_>>();
    keys.extend(
        checkpoint
            .workspace_state
            .files_changed
            .iter()
            .map(|file| format!("file:{}", file.path.to_string_lossy())),
    );
    keys.extend(
        checkpoint
            .commands_and_validation
            .iter()
            .map(|command| format!("command:{}", command.command.trim())),
    );
    keys.extend(checkpoint.artifacts.iter().filter_map(|artifact| {
        matches!(
            artifact.kind.as_str(),
            "file_reference" | "context_source" | "image_reference" | "image"
        )
        .then(|| {
            artifact
                .id
                .map(|id| format!("artifact:id:{id}"))
                .or_else(|| {
                    artifact
                        .path
                        .as_ref()
                        .map(|path| format!("artifact:path:{}", path.to_string_lossy()))
                })
                .unwrap_or_else(|| format!("artifact:{}", artifact.summary))
        })
    }));
    keys
}

fn retained_percent(previous: &HashSet<String>, current: &HashSet<String>) -> usize {
    if previous.is_empty() {
        return 100;
    }
    previous.intersection(current).count().saturating_mul(100) / previous.len()
}

fn latest_work_form(events: &[AgentEvent]) -> Option<(i64, &WorkForm)> {
    events.iter().rev().find_map(|event| match &event.payload {
        AgentEventPayload::WorkFormUpdated { form } => Some((event.seq, form)),
        AgentEventPayload::GoalUpdated { snapshot } => Some((event.seq, &snapshot.work_form)),
        _ => None,
    })
}

fn project_work_form(draft: &mut ContextCheckpointDraft, seq: i64, form: &WorkForm) {
    draft.user_constraints = form
        .constraints
        .iter()
        .filter_map(|constraint| {
            let text = constraint.trim();
            (!text.is_empty()).then(|| ContextCheckpointFact {
                id: format!(
                    "work-form-constraint-{}",
                    content_fingerprint(text.as_bytes())
                ),
                text: text.to_string(),
                status: ContextFactStatus::Active,
                source_seqs: vec![seq],
                confidence: Some(100),
            })
        })
        .collect();
    draft.next_steps = form
        .items
        .iter()
        .map(|item| ContextCheckpointStep {
            id: item.id.clone(),
            text: item.title.clone(),
            status: work_item_status(item.status).to_string(),
            source_seqs: vec![seq],
        })
        .collect();
}

fn work_item_status(status: WorkItemStatus) -> &'static str {
    match status {
        WorkItemStatus::Pending => "pending",
        WorkItemStatus::InProgress => "in_progress",
        WorkItemStatus::Completed => "completed",
        WorkItemStatus::Deferred => "deferred",
        WorkItemStatus::Blocked => "blocked",
        WorkItemStatus::Cancelled => "cancelled",
    }
}

fn project_commands(events: &[AgentEvent]) -> Vec<ContextCheckpointCommand> {
    let mut pending = HashMap::<Uuid, PendingCommand>::new();
    let mut commands = BTreeMap::<String, ContextCheckpointCommand>::new();
    for event in events {
        match &event.payload {
            AgentEventPayload::ToolCallStarted { call } => {
                if let Some(command) = command_text(call) {
                    pending.insert(
                        call.id,
                        PendingCommand {
                            command,
                            tool_name: call.name.clone(),
                            started_seq: event.seq,
                        },
                    );
                }
            }
            AgentEventPayload::ToolCallFinished { result } => {
                let Some(started) = pending.remove(&result.call_id) else {
                    continue;
                };
                let (outcome, summary) = command_outcome(&started.tool_name, result);
                commands.insert(
                    started.command.clone(),
                    ContextCheckpointCommand {
                        command: started.command,
                        outcome,
                        summary,
                        source_seqs: vec![started.started_seq, event.seq],
                    },
                );
            }
            _ => {}
        }
    }
    commands.into_values().collect()
}

fn command_text(call: &ToolCall) -> Option<String> {
    call.input
        .get("cmd")
        .or_else(|| call.input.get("command"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn command_outcome(tool_name: &str, result: &ToolResult) -> (String, String) {
    let exit_code = result
        .metadata
        .get("exitCode")
        .or_else(|| result.metadata.get("exit_code"))
        .and_then(serde_json::Value::as_i64);
    let succeeded = result
        .metadata
        .get("success")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            result
                .metadata
                .get("isError")
                .and_then(serde_json::Value::as_bool)
                .map(|is_error| !is_error)
        })
        .unwrap_or_else(|| exit_code.is_none_or(|code| code == 0));
    let outcome = if succeeded { "passed" } else { "failed" }.to_string();
    let summary = match exit_code {
        Some(code) => format!("{tool_name} finished with exit code {code}."),
        None if succeeded => format!("{tool_name} reported success."),
        None => format!("{tool_name} reported failure."),
    };
    (outcome, summary)
}

fn project_files(events: &[AgentEvent]) -> Vec<ContextCheckpointFile> {
    let mut files = BTreeMap::<PathBuf, ContextCheckpointFile>::new();
    for event in events {
        match &event.payload {
            AgentEventPayload::FileChanged { path, summary } => {
                files.insert(
                    path.clone(),
                    ContextCheckpointFile {
                        path: path.clone(),
                        status: "changed".to_string(),
                        summary: if summary.trim().is_empty() {
                            "Recorded by the local file-change event.".to_string()
                        } else {
                            truncate_chars(summary.trim(), 4_000)
                        },
                        source_seqs: vec![event.seq],
                    },
                );
            }
            AgentEventPayload::TurnChangesRecorded { change_set } => {
                for change in &change_set.files {
                    let Some(path) = change.display_path() else {
                        continue;
                    };
                    let status = file_change_status(change.kind);
                    let mut summary = format!("Recorded by the local turn change set as {status}");
                    if let (Some(additions), Some(deletions)) = (change.additions, change.deletions)
                    {
                        summary.push_str(&format!(" (+{additions}/-{deletions})"));
                    }
                    summary.push('.');
                    files.insert(
                        path.clone(),
                        ContextCheckpointFile {
                            path: path.clone(),
                            status: status.to_string(),
                            summary,
                            source_seqs: vec![event.seq],
                        },
                    );
                }
            }
            _ => {}
        }
    }
    files.into_values().collect()
}

fn project_artifacts(messages: &[Message]) -> Vec<ContextCheckpointArtifact> {
    let mut artifacts = BTreeMap::<String, ContextCheckpointArtifact>::new();
    for part in messages.iter().flat_map(|message| &message.parts) {
        let artifact = match part {
            MessagePart::FileRef { path } => Some(ContextCheckpointArtifact {
                id: None,
                path: Some(path.clone()),
                kind: "file_reference".to_string(),
                summary: "Referenced by the durable conversation.".to_string(),
                source_seqs: Vec::new(),
            }),
            MessagePart::SourceRef { source, .. } => Some(ContextCheckpointArtifact {
                id: Some(source.id),
                path: Some(source.path.clone()),
                kind: "context_source".to_string(),
                summary: format!("{} ({} bytes)", source.name, source.bytes),
                source_seqs: Vec::new(),
            }),
            MessagePart::ImageRef { image_id } => Some(ContextCheckpointArtifact {
                id: Some(*image_id),
                path: None,
                kind: "image_reference".to_string(),
                summary: "Referenced by the durable conversation.".to_string(),
                source_seqs: Vec::new(),
            }),
            MessagePart::Image {
                id: Some(id), name, ..
            } => Some(ContextCheckpointArtifact {
                id: Some(*id),
                path: None,
                kind: "image".to_string(),
                summary: name
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("Image attached to the durable conversation.")
                    .to_string(),
                source_seqs: Vec::new(),
            }),
            _ => None,
        };
        let Some(artifact) = artifact else {
            continue;
        };
        let key = artifact
            .id
            .map(|id| format!("id:{id}"))
            .or_else(|| {
                artifact
                    .path
                    .as_ref()
                    .map(|path| format!("path:{}", path.to_string_lossy()))
            })
            .unwrap_or_else(|| format!("{}:{}", artifact.kind, artifact.summary));
        artifacts.insert(key, artifact);
    }
    artifacts.into_values().collect()
}

fn file_change_status(kind: TurnFileChangeKind) -> &'static str {
    match kind {
        TurnFileChangeKind::Added => "added",
        TurnFileChangeKind::Modified => "modified",
        TurnFileChangeKind::Deleted => "deleted",
        TurnFileChangeKind::Renamed => "renamed",
    }
}

fn latest_user_text(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter(|message| message.role == MessageRole::User)
        .flat_map(|message| message.parts.iter().rev())
        .find_map(|part| match part {
            MessagePart::Text { text } | MessagePart::ProposedPlan { text } => {
                let text = text.trim();
                (!text.is_empty()).then(|| truncate_chars(text, 4_000))
            }
            _ => None,
        })
}

fn first_semantic_line(summary: &str) -> Option<String> {
    summary
        .lines()
        .map(str::trim)
        .map(|line| line.trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .map(|line| truncate_chars(line, 4_000))
}

#[cfg(test)]
mod tests {
    use super::{local_projection_retention_percentages, project_checkpoint_draft};
    use chrono::Utc;
    use opentopia_core::{
        AgentEvent, AgentEventPayload, CompletionDisposition, ContextCheckpoint,
        ContextCheckpointCommand, ContextCheckpointCoverage, ContextCheckpointFact,
        ContextFactStatus, Message, MessageRole, ToolCall, ToolResult, WorkForm, WorkItem,
        WorkItemStatus, WorkScope,
    };
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn projects_commands_and_work_form_without_parsing_model_text() {
        let thread_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let call = ToolCall::new("exec_command", json!({ "cmd": "cargo test" }));
        let mut form = WorkForm::new(
            thread_id,
            WorkScope::Turn(turn_id),
            "Implement local context projection",
            vec![WorkItem {
                id: "verify".to_string(),
                title: "Run targeted tests".to_string(),
                status: WorkItemStatus::InProgress,
                completion_disposition: CompletionDisposition::Blocking,
                depends_on: Vec::new(),
                note: None,
                acceptance: Vec::new(),
                evidence_refs: Vec::new(),
            }],
        );
        form.constraints = vec!["Do not duplicate the event log".to_string()];
        form.updated_at = Utc::now();
        let events = vec![
            AgentEvent::new(
                thread_id,
                Some(turn_id),
                10,
                AgentEventPayload::WorkFormUpdated { form },
            ),
            AgentEvent::new(
                thread_id,
                Some(turn_id),
                11,
                AgentEventPayload::ToolCallStarted { call: call.clone() },
            ),
            AgentEvent::new(
                thread_id,
                Some(turn_id),
                12,
                AgentEventPayload::ToolCallFinished {
                    result: ToolResult::text(
                        call.id,
                        "lots of raw output that should not be copied",
                        json!({ "success": true, "exitCode": 0 }),
                    ),
                },
            ),
        ];

        let draft = project_checkpoint_draft(
            &[Message::text(thread_id, MessageRole::User, "fallback goal")],
            &events,
            "A free-form summary with ``` and { arbitrary punctuation.",
            None,
        );

        assert_eq!(draft.goal, "Implement local context projection");
        assert_eq!(draft.user_constraints.len(), 1);
        assert_eq!(draft.next_steps[0].status, "in_progress");
        assert_eq!(draft.commands_and_validation[0].command, "cargo test");
        assert_eq!(draft.commands_and_validation[0].outcome, "passed");
        assert!(!draft.commands_and_validation[0]
            .summary
            .contains("raw output"));
    }

    #[test]
    fn rebuilds_the_projection_and_keeps_the_latest_repeated_command_outcome() {
        let thread_id = Uuid::new_v4();
        let old_call = ToolCall::new("exec_command", json!({ "cmd": "cargo test" }));
        let new_call = ToolCall::new("exec_command", json!({ "cmd": "cargo test" }));
        let events = vec![
            AgentEvent::new(
                thread_id,
                None,
                1,
                AgentEventPayload::ToolCallStarted {
                    call: old_call.clone(),
                },
            ),
            AgentEvent::new(
                thread_id,
                None,
                2,
                AgentEventPayload::ToolCallFinished {
                    result: ToolResult::text(old_call.id, "ok", json!({ "success": true })),
                },
            ),
            AgentEvent::new(
                thread_id,
                None,
                3,
                AgentEventPayload::ToolCallStarted {
                    call: new_call.clone(),
                },
            ),
            AgentEvent::new(
                thread_id,
                None,
                4,
                AgentEventPayload::ToolCallFinished {
                    result: ToolResult::text(new_call.id, "failed", json!({ "success": false })),
                },
            ),
        ];

        let draft = project_checkpoint_draft(&[], &events, "Current goal", None);

        assert_eq!(draft.commands_and_validation.len(), 1);
        assert_eq!(draft.commands_and_validation[0].command, "cargo test");
        assert_eq!(draft.commands_and_validation[0].outcome, "failed");
    }

    #[test]
    fn retention_metrics_ignore_legacy_model_semantics() {
        let thread_id = Uuid::new_v4();
        let mut previous = ContextCheckpoint::manual(
            thread_id,
            ContextCheckpointCoverage::default(),
            "old summary",
        );
        previous.decisions.push(ContextCheckpointFact {
            id: "model-decision".to_string(),
            text: "A semantic fact previously authored by the model".to_string(),
            status: ContextFactStatus::Active,
            source_seqs: vec![1],
            confidence: Some(80),
        });
        previous
            .commands_and_validation
            .push(ContextCheckpointCommand {
                command: "cargo test".to_string(),
                outcome: "passed".to_string(),
                summary: "exec_command reported success.".to_string(),
                source_seqs: vec![2],
            });
        let mut current = ContextCheckpoint::manual(
            thread_id,
            ContextCheckpointCoverage::default(),
            "new summary",
        );
        current.commands_and_validation = previous.commands_and_validation.clone();

        assert_eq!(
            local_projection_retention_percentages(Some(&previous), &current),
            (100, 100)
        );
        current.commands_and_validation.clear();
        assert_eq!(
            local_projection_retention_percentages(Some(&previous), &current),
            (0, 100)
        );
    }
}
