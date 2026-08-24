use crate::execution::{ExecutionFailure, ExecutionStage};
use crate::model::ToolResult;
use crate::tool_result_ingress::tool_result_is_error;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ToolErrorRecord {
    pub(crate) recorded: bool,
    pub(crate) code: String,
    pub(crate) phase: String,
    pub(crate) executed: bool,
    pub(crate) retryable: bool,
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) causes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) execution_stage: Option<ExecutionStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) os_error: Option<i32>,
}

impl ToolErrorRecord {
    fn new(code: &str, phase: &str, executed: bool, retryable: bool, message: &str) -> Self {
        Self {
            recorded: true,
            code: code.to_string(),
            phase: phase.to_string(),
            executed,
            retryable,
            message: message.to_string(),
            causes: Vec::new(),
            execution_stage: None,
            os_error: None,
        }
    }
}

pub(crate) fn insert_preserved_tool_error_record(metadata: &mut Value, record: &ToolErrorRecord) {
    if !metadata.is_object() {
        *metadata = json!({});
    }
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert("success".to_string(), json!(false));
    object
        .entry("error".to_string())
        .or_insert_with(|| json!(&record.message));
    object.insert(
        "errorRecord".to_string(),
        serde_json::to_value(record).expect("ToolErrorRecord must serialize"),
    );
    if !record.causes.is_empty() {
        object.insert("errorChain".to_string(), json!(&record.causes));
    }
    if let Some(stage) = record.execution_stage {
        object.insert("executionStage".to_string(), json!(stage));
    }
    if let Some(os_error) = record.os_error {
        object.insert("osError".to_string(), json!(os_error));
    }
}

pub(crate) fn insert_tool_error_record(
    metadata: &mut Value,
    code: &str,
    phase: &str,
    executed: bool,
    retryable: bool,
    message: &str,
) {
    let record = ToolErrorRecord::new(code, phase, executed, retryable, message);
    insert_preserved_tool_error_record(metadata, &record);
}

pub(crate) fn classify_anyhow_error(error: &anyhow::Error) -> ToolErrorRecord {
    let execution_failure = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<ExecutionFailure>());
    let io_error = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>());
    let transient_file_conflict = io_error
        .and_then(std::io::Error::raw_os_error)
        .is_some_and(|code| cfg!(windows) && matches!(code, 32 | 33 | 1224));
    let (code, phase, executed, retryable) = match execution_failure.map(|failure| failure.stage) {
        Some(ExecutionStage::ResolveRuntime) => {
            ("execution_runtime_unavailable", "preflight", false, true)
        }
        Some(ExecutionStage::ValidatePolicy) => (
            "execution_policy_unsatisfied",
            "authorization",
            false,
            false,
        ),
        Some(ExecutionStage::PrepareSandbox) => {
            ("sandbox_preparation_failed", "preflight", false, true)
        }
        Some(ExecutionStage::Spawn) => ("process_spawn_failed", "execution", false, true),
        Some(ExecutionStage::Wait) => ("process_wait_failed", "execution", true, false),
        Some(ExecutionStage::Terminate) => ("process_termination_failed", "execution", true, false),
        Some(ExecutionStage::CollectOutput) => {
            ("output_collection_failed", "execution", true, false)
        }
        None if transient_file_conflict => ("filesystem_temporarily_busy", "execution", true, true),
        None => ("tool_execution_failed", "execution", true, false),
    };
    let os_error = execution_failure
        .and_then(|failure| failure.os_error)
        .or_else(|| io_error.and_then(std::io::Error::raw_os_error));
    ToolErrorRecord {
        recorded: true,
        code: code.to_string(),
        phase: phase.to_string(),
        executed,
        retryable,
        message: format!("{error:#}"),
        causes: error.chain().map(ToString::to_string).collect(),
        execution_stage: execution_failure.map(|failure| failure.stage),
        os_error,
    }
}

pub(crate) fn insert_classified_anyhow_error_record(metadata: &mut Value, error: &anyhow::Error) {
    let record = classify_anyhow_error(error);
    insert_preserved_tool_error_record(metadata, &record);
}

pub(crate) fn ensure_tool_error_record(result: &mut ToolResult) {
    if !tool_result_is_error(result) || result.metadata.get("errorRecord").is_some() {
        return;
    }
    let (code, phase, executed, retryable) = if result
        .metadata
        .get("invalidToolArguments")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        ("invalid_tool_arguments", "validation", false, false)
    } else if result
        .metadata
        .get("reconciliationRequired")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        ("effect_reconciliation_required", "preflight", false, true)
    } else if result.metadata.get("flowToolCallBudget").is_some() {
        ("tool_budget_exhausted", "scheduling", false, false)
    } else if result
        .metadata
        .get("approvalRequired")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        ("approval_required", "authorization", false, true)
    } else if result
        .metadata
        .get("exitCode")
        .and_then(Value::as_i64)
        .is_some_and(|exit_code| exit_code != 0)
    {
        ("command_exit_nonzero", "command", true, false)
    } else {
        ("tool_execution_failed", "execution", true, false)
    };
    let message = if code == "command_exit_nonzero" {
        format!(
            "Command exited with code {}",
            result
                .metadata
                .get("exitCode")
                .and_then(Value::as_i64)
                .expect("classified command exit has an exit code")
        )
    } else {
        result
            .metadata
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or(&result.output)
            .to_string()
    };
    insert_tool_error_record(
        &mut result.metadata,
        code,
        phase,
        executed,
        retryable,
        &message,
    );
}

#[cfg(test)]
mod tests {
    use super::{ensure_tool_error_record, insert_classified_anyhow_error_record};
    use crate::model::ToolResult;
    use serde_json::json;
    use uuid::Uuid;

    #[cfg(windows)]
    #[test]
    fn windows_file_mapping_conflict_is_structured_and_retryable() {
        let error = anyhow::Error::from(std::io::Error::from_raw_os_error(1224));
        let mut metadata = json!({});

        insert_classified_anyhow_error_record(&mut metadata, &error);

        assert_eq!(
            metadata.pointer("/errorRecord/code"),
            Some(&json!("filesystem_temporarily_busy"))
        );
        assert_eq!(
            metadata.pointer("/errorRecord/retryable"),
            Some(&json!(true))
        );
        assert_eq!(metadata.get("osError"), Some(&json!(1224)));
    }

    #[test]
    fn nonzero_process_exit_is_a_command_outcome_not_a_tool_failure() {
        let mut result = ToolResult {
            call_id: Uuid::new_v4(),
            output: "validation reported errors".into(),
            content: Vec::new(),
            metadata: json!({
                "success": false,
                "exitCode": 1,
                "stderr": "validation reported errors"
            }),
        };

        ensure_tool_error_record(&mut result);

        assert_eq!(
            result.metadata.pointer("/errorRecord/code"),
            Some(&json!("command_exit_nonzero"))
        );
        assert_eq!(
            result.metadata.pointer("/errorRecord/phase"),
            Some(&json!("command"))
        );
        assert_eq!(
            result.metadata.pointer("/errorRecord/executed"),
            Some(&json!(true))
        );
    }
}
