use crate::execution::{ExecutionFailure, ExecutionStage};
use crate::model::ToolResult;
use crate::tool_result_ingress::tool_result_is_error;
use serde_json::{json, Value};

pub(crate) fn insert_tool_error_record(
    metadata: &mut Value,
    code: &str,
    phase: &str,
    executed: bool,
    retryable: bool,
    message: &str,
) {
    if !metadata.is_object() {
        *metadata = json!({});
    }
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert("success".to_string(), json!(false));
    object
        .entry("error".to_string())
        .or_insert_with(|| json!(message));
    object.insert(
        "errorRecord".to_string(),
        json!({
            "recorded": true,
            "code": code,
            "phase": phase,
            "executed": executed,
            "retryable": retryable,
            "message": message,
        }),
    );
}

pub(crate) fn insert_anyhow_error_record(
    metadata: &mut Value,
    code: &str,
    phase: &str,
    executed: bool,
    retryable: bool,
    error: &anyhow::Error,
) {
    let message = format!("{error:#}");
    insert_tool_error_record(metadata, code, phase, executed, retryable, &message);
    let chain = error.chain().map(ToString::to_string).collect::<Vec<_>>();
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert("errorChain".to_string(), json!(&chain));
    if let Some(record) = object.get_mut("errorRecord").and_then(Value::as_object_mut) {
        record.insert("causes".to_string(), json!(chain));
    }
}

pub(crate) fn insert_classified_anyhow_error_record(metadata: &mut Value, error: &anyhow::Error) {
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
    insert_anyhow_error_record(metadata, code, phase, executed, retryable, error);
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    let os_error = execution_failure
        .and_then(|failure| failure.os_error)
        .or_else(|| io_error.and_then(std::io::Error::raw_os_error));
    if let Some(failure) = execution_failure {
        object.insert("executionStage".to_string(), json!(failure.stage));
    }
    if let Some(os_error) = os_error {
        object.insert("osError".to_string(), json!(os_error));
    }
    if let Some(record) = object.get_mut("errorRecord").and_then(Value::as_object_mut) {
        if let Some(failure) = execution_failure {
            record.insert("executionStage".to_string(), json!(failure.stage));
        }
        if let Some(os_error) = os_error {
            record.insert("osError".to_string(), json!(os_error));
        }
    }
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
    } else {
        ("tool_execution_failed", "execution", true, false)
    };
    let message = result
        .metadata
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or(&result.output)
        .to_string();
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
    use super::insert_classified_anyhow_error_record;
    use serde_json::json;

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
}
