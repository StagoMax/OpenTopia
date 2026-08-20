//! Deterministic state-channel writes for Flow supersteps.
//!
//! Source graphs declare writes in `node.config.stateWrites`. The runtime
//! applies every write only when the whole superstep commits, in stable node
//! order, so parallel completion order never changes observable state.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const MAX_WORKFLOW_STATE_WRITES_PER_NODE: usize = 32;
pub const MAX_WORKFLOW_STATE_CHANNEL_LENGTH: usize = 128;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStateReducerV1 {
    Replace,
    Append,
    MergeObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowStateWriteV1 {
    pub channel: String,
    pub reducer: WorkflowStateReducerV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_path: Option<String>,
}

pub fn parse_state_writes(config: &Value) -> Result<Vec<WorkflowStateWriteV1>, String> {
    let Some(raw) = config.get("stateWrites") else {
        return Ok(Vec::new());
    };
    let writes = raw
        .as_array()
        .ok_or_else(|| "config.stateWrites must be an array".to_string())?;
    if writes.len() > MAX_WORKFLOW_STATE_WRITES_PER_NODE {
        return Err(format!(
            "config.stateWrites cannot contain more than {MAX_WORKFLOW_STATE_WRITES_PER_NODE} entries"
        ));
    }
    let mut parsed = Vec::with_capacity(writes.len());
    for raw_write in writes {
        let write: WorkflowStateWriteV1 = serde_json::from_value(raw_write.clone())
            .map_err(|error| format!("invalid state write: {error}"))?;
        validate_channel_name(&write.channel)?;
        if write
            .value_path
            .as_deref()
            .is_some_and(|path| path.len() > 256)
        {
            return Err("state write valuePath cannot exceed 256 characters".to_string());
        }
        parsed.push(write);
    }
    Ok(parsed)
}

pub fn validate_graph_state_writes<'a>(
    nodes: impl IntoIterator<Item = (&'a str, &'a Value)>,
) -> Vec<(String, String)> {
    let mut issues = Vec::new();
    let mut reducers = BTreeMap::<String, WorkflowStateReducerV1>::new();
    let mut replace_writers = BTreeMap::<String, String>::new();
    for (node_id, config) in nodes {
        let writes = match parse_state_writes(config) {
            Ok(writes) => writes,
            Err(message) => {
                issues.push((node_id.to_string(), message));
                continue;
            }
        };
        for write in writes {
            if let Some(existing) = reducers.insert(write.channel.clone(), write.reducer) {
                if existing != write.reducer {
                    issues.push((
                        node_id.to_string(),
                        format!(
                            "state channel '{}' uses conflicting reducers",
                            write.channel
                        ),
                    ));
                }
            }
            if write.reducer == WorkflowStateReducerV1::Replace {
                if let Some(existing) =
                    replace_writers.insert(write.channel.clone(), node_id.to_string())
                {
                    if existing != node_id {
                        issues.push((
                            node_id.to_string(),
                            format!(
                                "replace state channel '{}' must have exactly one writer (already written by {existing})",
                                write.channel
                            ),
                        ));
                    }
                }
            }
        }
    }
    issues
}

pub fn apply_state_writes(
    state: &mut BTreeMap<String, Value>,
    writes: &[WorkflowStateWriteV1],
    output: &Value,
) -> Result<(), String> {
    for write in writes {
        let value = match write.value_path.as_deref() {
            Some(path) => value_at_path(output, path).cloned().ok_or_else(|| {
                format!(
                    "state write for '{}' could not resolve valuePath '{path}'",
                    write.channel
                )
            })?,
            None => output.clone(),
        };
        match write.reducer {
            WorkflowStateReducerV1::Replace => {
                state.insert(write.channel.clone(), value);
            }
            WorkflowStateReducerV1::Append => {
                let target = state
                    .entry(write.channel.clone())
                    .or_insert_with(|| Value::Array(Vec::new()));
                let target = target
                    .as_array_mut()
                    .ok_or_else(|| format!("state channel '{}' is not an array", write.channel))?;
                match value {
                    Value::Array(values) => target.extend(values),
                    value => target.push(value),
                }
            }
            WorkflowStateReducerV1::MergeObject => {
                let target = state
                    .entry(write.channel.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                let target = target
                    .as_object_mut()
                    .ok_or_else(|| format!("state channel '{}' is not an object", write.channel))?;
                let value = value.as_object().ok_or_else(|| {
                    format!(
                        "merge_object state write for '{}' requires an object",
                        write.channel
                    )
                })?;
                for (key, value) in value {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
    }
    Ok(())
}

fn validate_channel_name(channel: &str) -> Result<(), String> {
    if channel.is_empty()
        || channel.len() > MAX_WORKFLOW_STATE_CHANNEL_LENGTH
        || !channel.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        return Err(format!(
            "state channel '{channel}' must use 1-{MAX_WORKFLOW_STATE_CHANNEL_LENGTH} letters, digits, dots, underscores, or hyphens"
        ));
    }
    Ok(())
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.trim().trim_start_matches("$.");
    if path.is_empty() || path == "$" {
        return Some(value);
    }
    path.split('.')
        .try_fold(value, |current, segment| current.as_object()?.get(segment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reducers_are_deterministic_and_typed() {
        let mut state = BTreeMap::new();
        apply_state_writes(
            &mut state,
            &[
                WorkflowStateWriteV1 {
                    channel: "items".to_string(),
                    reducer: WorkflowStateReducerV1::Append,
                    value_path: Some("payload.items".to_string()),
                },
                WorkflowStateWriteV1 {
                    channel: "summary".to_string(),
                    reducer: WorkflowStateReducerV1::MergeObject,
                    value_path: Some("payload.summary".to_string()),
                },
            ],
            &json!({"payload":{"items":[1,2],"summary":{"count":2}}}),
        )
        .unwrap();
        assert_eq!(state["items"], json!([1, 2]));
        assert_eq!(state["summary"], json!({"count": 2}));
    }

    #[test]
    fn conflicting_channel_contracts_fail_validation() {
        let left = json!({"stateWrites":[{"channel":"shared","reducer":"replace"}]});
        let right = json!({"stateWrites":[{"channel":"shared","reducer":"append"}]});
        let issues = validate_graph_state_writes([("left", &left), ("right", &right)]);
        assert!(issues
            .iter()
            .any(|(_, message)| message.contains("conflicting reducers")));
    }
}
