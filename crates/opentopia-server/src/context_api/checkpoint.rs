//! Structured checkpoint parsing, validation, merging, and budget enforcement.

use crate::{estimate_tokens, latest_active_work_form_event, truncate_chars, ApiError};
use chrono::Utc;
use opentopia_core::{
    AgentEvent, AgentEventPayload, ContextCheckpoint, ContextCheckpointArtifact,
    ContextCheckpointCommand, ContextCheckpointCoverage, ContextCheckpointFact,
    ContextCheckpointInteraction, ContextCheckpointMode, ContextCheckpointStep,
    ContextCheckpointWorkspace, ContextFactStatus, CONTEXT_CHECKPOINT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

pub(crate) fn context_summary_system_prompt() -> &'static str {
    "You merge an AI coding-agent session into a durable structured checkpoint. Return only JSON matching the supplied schema. The server deterministically merges entries by stable id or natural key, so unchanged entries from the previous checkpoint may be omitted. Include every new or changed fact needed to update it. Preserve exact file paths, commands, errors, identifiers, active user constraints, unresolved risks, pending interactions, and artifact references. Source sequence numbers must refer only to supplied event seq values. Mark resolved or superseded facts explicitly instead of silently deleting them. Omit greetings, repetition, transient progress narration, large raw tool output, and secrets. Never claim unfinished work or failed validation is completed."
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextCheckpointDraft {
    goal: String,
    #[serde(default)]
    user_constraints: Vec<ContextCheckpointFact>,
    #[serde(default)]
    decisions: Vec<ContextCheckpointFact>,
    #[serde(default)]
    workspace_state: ContextCheckpointWorkspace,
    #[serde(default)]
    commands_and_validation: Vec<ContextCheckpointCommand>,
    #[serde(default)]
    open_issues: Vec<ContextCheckpointFact>,
    #[serde(default)]
    next_steps: Vec<ContextCheckpointStep>,
    #[serde(default)]
    pending_interactions: Vec<ContextCheckpointInteraction>,
    #[serde(default)]
    artifacts: Vec<ContextCheckpointArtifact>,
}

pub(crate) fn context_checkpoint_schema() -> Value {
    let source_seqs = json!({
        "type": "array",
        "items": { "type": "integer", "minimum": 1 },
        "maxItems": 32
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "goal", "userConstraints", "decisions", "workspaceState",
            "commandsAndValidation", "openIssues", "nextSteps",
            "pendingInteractions", "artifacts"
        ],
        "properties": {
            "goal": { "type": "string", "maxLength": 12000 },
            "userConstraints": { "type": "array", "maxItems": 96, "items": { "$ref": "#/$defs/fact" } },
            "decisions": { "type": "array", "maxItems": 96, "items": { "$ref": "#/$defs/fact" } },
            "workspaceState": { "$ref": "#/$defs/workspace" },
            "commandsAndValidation": { "type": "array", "maxItems": 96, "items": { "$ref": "#/$defs/command" } },
            "openIssues": { "type": "array", "maxItems": 96, "items": { "$ref": "#/$defs/fact" } },
            "nextSteps": { "type": "array", "maxItems": 64, "items": { "$ref": "#/$defs/step" } },
            "pendingInteractions": { "type": "array", "maxItems": 64, "items": { "$ref": "#/$defs/interaction" } },
            "artifacts": { "type": "array", "maxItems": 96, "items": { "$ref": "#/$defs/artifact" } }
        },
        "$defs": {
            "fact": {
                "type": "object",
                "additionalProperties": false,
                "required": ["id", "text", "status", "sourceSeqs", "confidence"],
                "properties": {
                    "id": { "type": "string", "maxLength": 160 },
                    "text": { "type": "string", "maxLength": 4000 },
                    "status": { "type": "string", "enum": ["active", "resolved", "superseded"] },
                    "sourceSeqs": source_seqs.clone(),
                    "confidence": { "type": ["integer", "null"], "minimum": 0, "maximum": 100 }
                }
            },
            "file": {
                "type": "object",
                "additionalProperties": false,
                "required": ["path", "status", "summary", "sourceSeqs"],
                "properties": {
                    "path": { "type": "string", "maxLength": 2000 },
                    "status": { "type": "string", "maxLength": 160 },
                    "summary": { "type": "string", "maxLength": 4000 },
                    "sourceSeqs": source_seqs.clone()
                }
            },
            "workspace": {
                "type": "object",
                "additionalProperties": false,
                "required": ["branch", "gitStatus", "filesChanged"],
                "properties": {
                    "branch": { "type": ["string", "null"], "maxLength": 500 },
                    "gitStatus": { "type": ["string", "null"], "maxLength": 4000 },
                    "filesChanged": { "type": "array", "maxItems": 160, "items": { "$ref": "#/$defs/file" } }
                }
            },
            "command": {
                "type": "object",
                "additionalProperties": false,
                "required": ["command", "outcome", "summary", "sourceSeqs"],
                "properties": {
                    "command": { "type": "string", "maxLength": 4000 },
                    "outcome": { "type": "string", "maxLength": 160 },
                    "summary": { "type": "string", "maxLength": 4000 },
                    "sourceSeqs": source_seqs.clone()
                }
            },
            "step": {
                "type": "object",
                "additionalProperties": false,
                "required": ["id", "text", "status", "sourceSeqs"],
                "properties": {
                    "id": { "type": "string", "maxLength": 160 },
                    "text": { "type": "string", "maxLength": 4000 },
                    "status": { "type": "string", "maxLength": 160 },
                    "sourceSeqs": source_seqs.clone()
                }
            },
            "interaction": {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind", "summary", "sourceSeqs"],
                "properties": {
                    "kind": { "type": "string", "maxLength": 160 },
                    "summary": { "type": "string", "maxLength": 4000 },
                    "sourceSeqs": source_seqs.clone()
                }
            },
            "artifact": {
                "type": "object",
                "additionalProperties": false,
                "required": ["id", "path", "kind", "summary", "sourceSeqs"],
                "properties": {
                    "id": { "type": ["string", "null"], "format": "uuid" },
                    "path": { "type": ["string", "null"], "maxLength": 2000 },
                    "kind": { "type": "string", "maxLength": 160 },
                    "summary": { "type": "string", "maxLength": 4000 },
                    "sourceSeqs": source_seqs
                }
            }
        }
    })
}

pub(crate) fn parse_checkpoint_response(text: &str) -> Result<Value, ApiError> {
    let mut candidate = text.trim();
    if candidate.starts_with("```") {
        candidate = candidate
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        candidate = candidate
            .strip_suffix("```")
            .map(str::trim)
            .unwrap_or(candidate);
    }
    serde_json::from_str(candidate)
        .map_err(|error| ApiError::bad_gateway(format!("checkpoint response is not JSON: {error}")))
}

pub(crate) fn sanitize_checkpoint_draft(
    draft: &mut ContextCheckpointDraft,
    covered_through_seq: i64,
) -> Result<(), ApiError> {
    draft.goal = truncate_chars(draft.goal.trim(), 12_000);
    if draft.goal.is_empty() {
        return Err(ApiError::bad_gateway("checkpoint goal cannot be empty"));
    }
    draft.user_constraints.truncate(96);
    draft.decisions.truncate(96);
    draft.commands_and_validation.truncate(96);
    draft.open_issues.truncate(96);
    draft.next_steps.truncate(64);
    draft.pending_interactions.truncate(64);
    draft.artifacts.truncate(96);
    draft.workspace_state.files_changed.truncate(160);

    for fact in draft
        .user_constraints
        .iter_mut()
        .chain(draft.decisions.iter_mut())
        .chain(draft.open_issues.iter_mut())
    {
        fact.id = truncate_chars(fact.id.trim(), 160);
        fact.text = truncate_chars(fact.text.trim(), 4_000);
        fact.confidence = fact.confidence.map(|value| value.min(100));
        sanitize_source_seqs(&mut fact.source_seqs, covered_through_seq);
        if fact.id.is_empty() || fact.text.is_empty() {
            return Err(ApiError::bad_gateway(
                "checkpoint facts require non-empty id and text",
            ));
        }
    }
    for file in &mut draft.workspace_state.files_changed {
        file.status = truncate_chars(file.status.trim(), 160);
        file.summary = truncate_chars(file.summary.trim(), 4_000);
        sanitize_source_seqs(&mut file.source_seqs, covered_through_seq);
        if file.path.as_os_str().is_empty() {
            return Err(ApiError::bad_gateway(
                "checkpoint file entries require a path",
            ));
        }
    }
    for command in &mut draft.commands_and_validation {
        command.command = truncate_chars(command.command.trim(), 4_000);
        command.outcome = truncate_chars(command.outcome.trim(), 160);
        command.summary = truncate_chars(command.summary.trim(), 4_000);
        sanitize_source_seqs(&mut command.source_seqs, covered_through_seq);
        if command.command.is_empty() {
            return Err(ApiError::bad_gateway(
                "checkpoint command entries require a command",
            ));
        }
    }
    for step in &mut draft.next_steps {
        step.id = truncate_chars(step.id.trim(), 160);
        step.text = truncate_chars(step.text.trim(), 4_000);
        step.status = truncate_chars(step.status.trim(), 160);
        sanitize_source_seqs(&mut step.source_seqs, covered_through_seq);
        if step.id.is_empty() || step.text.is_empty() {
            return Err(ApiError::bad_gateway(
                "checkpoint steps require non-empty id and text",
            ));
        }
    }
    for interaction in &mut draft.pending_interactions {
        interaction.kind = truncate_chars(interaction.kind.trim(), 160);
        interaction.summary = truncate_chars(interaction.summary.trim(), 4_000);
        sanitize_source_seqs(&mut interaction.source_seqs, covered_through_seq);
        if interaction.kind.is_empty() || interaction.summary.is_empty() {
            return Err(ApiError::bad_gateway(
                "checkpoint interactions require non-empty kind and summary",
            ));
        }
    }
    for artifact in &mut draft.artifacts {
        artifact.kind = truncate_chars(artifact.kind.trim(), 160);
        artifact.summary = truncate_chars(artifact.summary.trim(), 4_000);
        sanitize_source_seqs(&mut artifact.source_seqs, covered_through_seq);
        if artifact.kind.is_empty() || artifact.summary.is_empty() {
            return Err(ApiError::bad_gateway(
                "checkpoint artifacts require non-empty kind and summary",
            ));
        }
    }
    Ok(())
}

fn sanitize_source_seqs(source_seqs: &mut Vec<i64>, covered_through_seq: i64) {
    source_seqs.retain(|seq| *seq > 0 && *seq <= covered_through_seq);
    source_seqs.sort_unstable();
    source_seqs.dedup();
    source_seqs.truncate(32);
}

pub(crate) fn validate_checkpoint_draft(
    draft: &ContextCheckpointDraft,
    events: &[AgentEvent],
) -> Result<(), ApiError> {
    let mut command_by_call = HashMap::<Uuid, String>::new();
    let mut command_success = HashMap::<String, bool>::new();
    for event in events {
        match &event.payload {
            AgentEventPayload::ToolCallStarted { call } => {
                if let Some(command) = call
                    .input
                    .get("cmd")
                    .or_else(|| call.input.get("command"))
                    .and_then(Value::as_str)
                {
                    command_by_call.insert(call.id, command.trim().to_string());
                }
            }
            AgentEventPayload::ToolCallFinished { result } => {
                let Some(command) = command_by_call.get(&result.call_id) else {
                    continue;
                };
                let succeeded = result
                    .metadata
                    .get("success")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
                    && result
                        .metadata
                        .get("exitCode")
                        .and_then(Value::as_i64)
                        .is_none_or(|exit_code| exit_code == 0);
                command_success.insert(command.clone(), succeeded);
            }
            _ => {}
        }
    }
    for command in &draft.commands_and_validation {
        if checkpoint_status_is_resolved(&command.outcome)
            && command_success.get(command.command.trim()) == Some(&false)
        {
            return Err(ApiError::bad_gateway(format!(
                "checkpoint incorrectly marks failed command '{}' as successful",
                command.command
            )));
        }
    }
    let Some(active_form) = latest_active_work_form_event(events) else {
        return Ok(());
    };
    for step in &draft.next_steps {
        let Some(runtime_step) = active_form
            .items
            .iter()
            .find(|candidate| candidate.id == step.id)
        else {
            continue;
        };
        if runtime_step.status.is_actionable() && checkpoint_status_is_resolved(&step.status) {
            return Err(ApiError::bad_gateway(format!(
                "checkpoint incorrectly marks active plan step '{}' as resolved",
                step.id
            )));
        }
    }
    Ok(())
}

fn checkpoint_status_is_resolved(status: &str) -> bool {
    matches!(
        status.trim().to_ascii_lowercase().as_str(),
        "completed" | "complete" | "done" | "resolved" | "succeeded" | "passed"
    )
}

pub(crate) fn checkpoint_token_budget(context_window: usize) -> usize {
    (context_window / 10)
        .clamp(1_024, 16_384)
        .min((context_window / 4).max(1_024))
}

pub(crate) fn merge_context_checkpoint(
    previous: Option<&ContextCheckpoint>,
    draft: ContextCheckpointDraft,
    thread_id: Uuid,
    coverage: ContextCheckpointCoverage,
    provider_compatibility_hash: Option<String>,
) -> ContextCheckpoint {
    let previous_id = previous.map(|checkpoint| checkpoint.id);
    let mut checkpoint = previous.cloned().unwrap_or_else(|| ContextCheckpoint {
        id: Uuid::new_v4(),
        thread_id,
        schema_version: CONTEXT_CHECKPOINT_SCHEMA_VERSION,
        mode: ContextCheckpointMode::StructuredLocal,
        previous_checkpoint_id: None,
        coverage: ContextCheckpointCoverage::default(),
        provider_compatibility_hash: None,
        goal: String::new(),
        user_constraints: Vec::new(),
        decisions: Vec::new(),
        workspace_state: ContextCheckpointWorkspace::default(),
        commands_and_validation: Vec::new(),
        open_issues: Vec::new(),
        next_steps: Vec::new(),
        pending_interactions: Vec::new(),
        artifacts: Vec::new(),
        created_at: Utc::now(),
    });

    checkpoint.id = Uuid::new_v4();
    checkpoint.thread_id = thread_id;
    checkpoint.schema_version = CONTEXT_CHECKPOINT_SCHEMA_VERSION;
    checkpoint.mode = ContextCheckpointMode::StructuredLocal;
    checkpoint.previous_checkpoint_id = previous_id;
    checkpoint.coverage = coverage;
    checkpoint.provider_compatibility_hash = provider_compatibility_hash
        .or_else(|| previous.and_then(|checkpoint| checkpoint.provider_compatibility_hash.clone()));
    checkpoint.created_at = Utc::now();
    if !draft.goal.trim().is_empty() {
        checkpoint.goal = draft.goal;
    }

    checkpoint.user_constraints = merge_checkpoint_entries(
        checkpoint.user_constraints,
        draft.user_constraints,
        |fact| checkpoint_fact_key(fact),
    );
    checkpoint.decisions =
        merge_checkpoint_entries(checkpoint.decisions, draft.decisions, |fact| {
            checkpoint_fact_key(fact)
        });
    checkpoint.open_issues =
        merge_checkpoint_entries(checkpoint.open_issues, draft.open_issues, |fact| {
            checkpoint_fact_key(fact)
        });
    checkpoint.commands_and_validation = merge_checkpoint_entries(
        checkpoint.commands_and_validation,
        draft.commands_and_validation,
        |command| command.command.trim().to_owned(),
    );
    checkpoint.next_steps =
        merge_checkpoint_entries(checkpoint.next_steps, draft.next_steps, |step| {
            if step.id.trim().is_empty() {
                step.text.trim().to_owned()
            } else {
                step.id.trim().to_owned()
            }
        });
    checkpoint.pending_interactions = merge_checkpoint_entries(
        checkpoint.pending_interactions,
        draft.pending_interactions,
        |interaction| {
            format!(
                "{}\u{0}{}",
                interaction.kind.trim(),
                interaction.summary.trim()
            )
        },
    );
    checkpoint.artifacts =
        merge_checkpoint_entries(checkpoint.artifacts, draft.artifacts, |artifact| {
            artifact
                .id
                .map(|id| format!("id:{id}"))
                .or_else(|| {
                    artifact
                        .path
                        .as_ref()
                        .map(|path| format!("path:{}", path.to_string_lossy()))
                })
                .unwrap_or_else(|| {
                    format!("{}\u{0}{}", artifact.kind.trim(), artifact.summary.trim())
                })
        });
    checkpoint.workspace_state.branch = draft
        .workspace_state
        .branch
        .or(checkpoint.workspace_state.branch);
    checkpoint.workspace_state.git_status = draft
        .workspace_state
        .git_status
        .or(checkpoint.workspace_state.git_status);
    checkpoint.workspace_state.files_changed = merge_checkpoint_entries(
        checkpoint.workspace_state.files_changed,
        draft.workspace_state.files_changed,
        |file| file.path.to_string_lossy().into_owned(),
    );
    checkpoint
}

fn checkpoint_fact_key(fact: &ContextCheckpointFact) -> String {
    if fact.id.trim().is_empty() {
        fact.text.trim().to_owned()
    } else {
        fact.id.trim().to_owned()
    }
}

pub(crate) fn checkpoint_retention_percentages(
    previous: Option<&ContextCheckpoint>,
    current: &ContextCheckpoint,
) -> (usize, usize) {
    let Some(previous) = previous else {
        return (100, 100);
    };
    let previous_keys = checkpoint_retention_keys(previous, false);
    let current_keys = checkpoint_retention_keys(current, false);
    let previous_constraints = checkpoint_retention_keys(previous, true);
    let current_constraints = checkpoint_retention_keys(current, true);
    (
        retained_percent(&previous_keys, &current_keys),
        retained_percent(&previous_constraints, &current_constraints),
    )
}

fn checkpoint_retention_keys(
    checkpoint: &ContextCheckpoint,
    active_constraints_only: bool,
) -> HashSet<String> {
    if active_constraints_only {
        return checkpoint
            .user_constraints
            .iter()
            .filter(|fact| fact.status == ContextFactStatus::Active)
            .map(|fact| format!("constraint:{}", checkpoint_fact_key(fact)))
            .collect();
    }
    let mut keys = HashSet::new();
    for fact in &checkpoint.user_constraints {
        keys.insert(format!("constraint:{}", checkpoint_fact_key(fact)));
    }
    for fact in &checkpoint.decisions {
        keys.insert(format!("decision:{}", checkpoint_fact_key(fact)));
    }
    for fact in &checkpoint.open_issues {
        keys.insert(format!("issue:{}", checkpoint_fact_key(fact)));
    }
    for file in &checkpoint.workspace_state.files_changed {
        keys.insert(format!("file:{}", file.path.to_string_lossy()));
    }
    for command in &checkpoint.commands_and_validation {
        keys.insert(format!("command:{}", command.command.trim()));
    }
    for step in &checkpoint.next_steps {
        keys.insert(format!("step:{}", step.id.trim()));
    }
    for artifact in &checkpoint.artifacts {
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
        keys.insert(format!("artifact:{key}"));
    }
    keys
}

fn retained_percent(previous: &HashSet<String>, current: &HashSet<String>) -> usize {
    if previous.is_empty() {
        return 100;
    }
    previous.intersection(current).count().saturating_mul(100) / previous.len()
}

fn merge_checkpoint_entries<T, F>(previous: Vec<T>, current: Vec<T>, key: F) -> Vec<T>
where
    F: Fn(&T) -> String,
{
    let mut merged = previous;
    let mut indexes = merged
        .iter()
        .enumerate()
        .map(|(index, item)| (key(item), index))
        .collect::<BTreeMap<_, _>>();
    for item in current {
        let item_key = key(&item);
        if let Some(index) = indexes.get(&item_key).copied() {
            merged[index] = item;
        } else {
            indexes.insert(item_key, merged.len());
            merged.push(item);
        }
    }
    merged
}

pub(crate) fn trim_checkpoint_to_budget(checkpoint: &mut ContextCheckpoint, token_budget: usize) {
    if checkpoint_token_estimate(checkpoint) <= token_budget {
        return;
    }

    compact_checkpoint_text(checkpoint, 1_000, 4_000);
    while checkpoint_token_estimate(checkpoint) > token_budget
        && remove_lowest_priority_checkpoint_entry(checkpoint)
    {}
    if checkpoint_token_estimate(checkpoint) > token_budget {
        compact_checkpoint_text(checkpoint, 400, 2_000);
        while checkpoint_token_estimate(checkpoint) > token_budget
            && remove_lowest_priority_checkpoint_entry(checkpoint)
        {}
    }
}

pub(crate) fn checkpoint_token_estimate(checkpoint: &ContextCheckpoint) -> usize {
    serde_json::to_string(checkpoint)
        .map(|serialized| estimate_tokens(&serialized))
        .unwrap_or(usize::MAX)
}

fn compact_checkpoint_text(
    checkpoint: &mut ContextCheckpoint,
    item_char_limit: usize,
    goal_char_limit: usize,
) {
    checkpoint.goal = truncate_chars(&checkpoint.goal, goal_char_limit);
    checkpoint.workspace_state.git_status = checkpoint
        .workspace_state
        .git_status
        .as_deref()
        .map(|value| truncate_chars(value, item_char_limit));
    for fact in checkpoint
        .user_constraints
        .iter_mut()
        .chain(checkpoint.decisions.iter_mut())
        .chain(checkpoint.open_issues.iter_mut())
    {
        fact.text = truncate_chars(&fact.text, item_char_limit);
    }
    for file in &mut checkpoint.workspace_state.files_changed {
        file.summary = truncate_chars(&file.summary, item_char_limit);
    }
    for command in &mut checkpoint.commands_and_validation {
        command.command = truncate_chars(&command.command, item_char_limit);
        command.summary = truncate_chars(&command.summary, item_char_limit);
    }
    for step in &mut checkpoint.next_steps {
        step.text = truncate_chars(&step.text, item_char_limit);
    }
    for interaction in &mut checkpoint.pending_interactions {
        interaction.summary = truncate_chars(&interaction.summary, item_char_limit);
    }
    for artifact in &mut checkpoint.artifacts {
        artifact.summary = truncate_chars(&artifact.summary, item_char_limit);
    }
}

fn remove_lowest_priority_checkpoint_entry(checkpoint: &mut ContextCheckpoint) -> bool {
    if checkpoint.artifacts.pop().is_some() {
        return true;
    }
    if remove_inactive_fact(&mut checkpoint.open_issues)
        || remove_inactive_fact(&mut checkpoint.decisions)
    {
        return true;
    }
    if checkpoint.pending_interactions.pop().is_some() {
        return true;
    }
    false
}

fn remove_inactive_fact(facts: &mut Vec<ContextCheckpointFact>) -> bool {
    let Some(index) = facts
        .iter()
        .rposition(|fact| fact.status != ContextFactStatus::Active)
    else {
        return false;
    };
    facts.remove(index);
    true
}

pub(crate) fn render_context_checkpoint(checkpoint: &ContextCheckpoint) -> String {
    serde_json::to_string_pretty(checkpoint)
        .unwrap_or_else(|_| format!("{{\"goal\":{}}}", json!(checkpoint.goal)))
}

#[cfg(test)]
mod tests {
    use super::{
        checkpoint_retention_percentages, checkpoint_token_budget, checkpoint_token_estimate,
        merge_context_checkpoint, parse_checkpoint_response, sanitize_checkpoint_draft,
        trim_checkpoint_to_budget, validate_checkpoint_draft, ContextCheckpointDraft,
    };
    use opentopia_core::{
        AgentEvent, AgentEventPayload, ContextCheckpoint, ContextCheckpointArtifact,
        ContextCheckpointCommand, ContextCheckpointCoverage, ContextCheckpointFact,
        ContextFactStatus, ToolCall, ToolResult,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[test]
    fn checkpoint_response_is_parsed_sanitized_and_bounded() {
        let payload = json!({
            "goal": "  preserve the current implementation  ",
            "userConstraints": [{
                "id": "constraint-1",
                "text": "keep compatibility",
                "status": "active",
                "sourceSeqs": [4, 4, 999],
                "confidence": 200
            }],
            "decisions": [],
            "workspaceState": { "branch": null, "gitStatus": null, "filesChanged": [] },
            "commandsAndValidation": [],
            "openIssues": [],
            "nextSteps": [],
            "pendingInteractions": [],
            "artifacts": []
        });
        let fenced = format!("```json\n{}\n```", payload);
        let value = parse_checkpoint_response(&fenced).expect("parse fenced JSON");
        let mut draft: ContextCheckpointDraft =
            serde_json::from_value(value).expect("deserialize draft");
        sanitize_checkpoint_draft(&mut draft, 10).expect("sanitize draft");

        assert_eq!(draft.goal, "preserve the current implementation");
        assert_eq!(draft.user_constraints[0].source_seqs, vec![4]);
        assert_eq!(draft.user_constraints[0].confidence, Some(100));
        assert_eq!(checkpoint_token_budget(128_000), 12_800);
        assert_eq!(checkpoint_token_budget(4_096), 1_024);
    }

    #[test]
    fn checkpoint_cannot_relabel_a_known_failed_command_as_successful() {
        let thread_id = Uuid::new_v4();
        let call = ToolCall::new("exec_command", json!({ "cmd": "cargo test" }));
        let events = vec![
            AgentEvent::new(
                thread_id,
                None,
                1,
                AgentEventPayload::ToolCallStarted { call: call.clone() },
            ),
            AgentEvent::new(
                thread_id,
                None,
                2,
                AgentEventPayload::ToolCallFinished {
                    result: ToolResult::text(
                        call.id,
                        "tests failed",
                        json!({ "success": false, "exitCode": 1 }),
                    ),
                },
            ),
        ];
        let draft = ContextCheckpointDraft {
            goal: "fix the tests".to_string(),
            commands_and_validation: vec![ContextCheckpointCommand {
                command: "cargo test".to_string(),
                outcome: "passed".to_string(),
                summary: "all tests passed".to_string(),
                source_seqs: vec![2],
            }],
            ..ContextCheckpointDraft::default()
        };

        let error = validate_checkpoint_draft(&draft, &events)
            .expect_err("failed command must remain failed");
        assert!(error.message.contains("marks failed command"));
    }

    #[test]
    fn checkpoint_delta_merge_preserves_unmentioned_facts_and_updates_stable_keys() {
        let thread_id = Uuid::new_v4();
        let mut previous = ContextCheckpoint::manual(
            thread_id,
            ContextCheckpointCoverage {
                through_seq: 10,
                through_message_count: 3,
            },
            "implement compaction",
        );
        previous.user_constraints.push(ContextCheckpointFact {
            id: "constraint-language".to_string(),
            text: "keep the API backward compatible".to_string(),
            status: ContextFactStatus::Active,
            source_seqs: vec![2],
            confidence: Some(100),
        });
        previous.decisions.push(ContextCheckpointFact {
            id: "decision-format".to_string(),
            text: "use plain text".to_string(),
            status: ContextFactStatus::Active,
            source_seqs: vec![4],
            confidence: Some(80),
        });
        let previous_id = previous.id;
        let draft = ContextCheckpointDraft {
            goal: "implement compaction fully".to_string(),
            decisions: vec![ContextCheckpointFact {
                id: "decision-format".to_string(),
                text: "use structured JSON".to_string(),
                status: ContextFactStatus::Active,
                source_seqs: vec![12],
                confidence: Some(100),
            }],
            open_issues: vec![ContextCheckpointFact {
                id: "issue-eval".to_string(),
                text: "run the long-context fixture".to_string(),
                status: ContextFactStatus::Active,
                source_seqs: vec![13],
                confidence: Some(90),
            }],
            ..ContextCheckpointDraft::default()
        };

        let merged = merge_context_checkpoint(
            Some(&previous),
            draft,
            thread_id,
            ContextCheckpointCoverage {
                through_seq: 14,
                through_message_count: 6,
            },
            Some("hash-2".to_string()),
        );

        assert_eq!(merged.previous_checkpoint_id, Some(previous_id));
        assert_eq!(merged.user_constraints, previous.user_constraints);
        assert_eq!(merged.decisions.len(), 1);
        assert_eq!(merged.decisions[0].text, "use structured JSON");
        assert_eq!(merged.open_issues[0].id, "issue-eval");
        assert_eq!(merged.coverage.through_message_count, 6);
        assert_eq!(
            checkpoint_retention_percentages(Some(&previous), &merged),
            (100, 100)
        );
    }

    #[test]
    fn checkpoint_budget_trimming_never_silently_drops_critical_recovery_keys() {
        let thread_id = Uuid::new_v4();
        let mut checkpoint = ContextCheckpoint::manual(
            thread_id,
            ContextCheckpointCoverage::default(),
            "goal ".repeat(4_000),
        );
        checkpoint.user_constraints.push(ContextCheckpointFact {
            id: "constraint-keep".to_string(),
            text: "constraint ".repeat(1_000),
            status: ContextFactStatus::Active,
            source_seqs: vec![1],
            confidence: Some(100),
        });
        checkpoint
            .workspace_state
            .files_changed
            .push(opentopia_core::ContextCheckpointFile {
                path: PathBuf::from("src/critical.rs"),
                status: "modified".to_string(),
                summary: "file summary ".repeat(1_000),
                source_seqs: vec![2],
            });
        checkpoint
            .commands_and_validation
            .push(ContextCheckpointCommand {
                command: "cargo test --workspace".to_string(),
                outcome: "passed".to_string(),
                summary: "validation ".repeat(1_000),
                source_seqs: vec![3],
            });
        for index in 0..40 {
            checkpoint.artifacts.push(ContextCheckpointArtifact {
                id: None,
                path: Some(PathBuf::from(format!("tmp/artifact-{index}.log"))),
                kind: "log".to_string(),
                summary: "noise ".repeat(1_000),
                source_seqs: vec![4],
            });
        }

        trim_checkpoint_to_budget(&mut checkpoint, 4_096);

        assert_eq!(checkpoint.user_constraints[0].id, "constraint-keep");
        assert_eq!(
            checkpoint.workspace_state.files_changed[0].path,
            PathBuf::from("src/critical.rs")
        );
        assert_eq!(
            checkpoint.commands_and_validation[0].command,
            "cargo test --workspace"
        );
        assert!(checkpoint.artifacts.len() < 40);
        assert!(checkpoint_token_estimate(&checkpoint) <= 4_096);
    }
}
