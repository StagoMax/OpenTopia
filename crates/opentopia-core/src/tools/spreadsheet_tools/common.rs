use super::super::{
    spreadsheet_tool::execute_spreadsheet_backend, tool_resource_key, ToolExecutionPolicy,
    ToolInvocationContext, ToolSideEffect,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::model::ToolResult;
use crate::spreadsheet::{
    parse_a1_address, SpreadsheetFilterCondition, SpreadsheetFilterOperator, SpreadsheetFilterValue,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) struct SpreadsheetRowConditionInput {
    /// Worksheet column name, for example G.
    pub(super) column: String,
    pub(super) operator: SpreadsheetFilterOperator,
    #[serde(default)]
    pub(super) value: Option<SpreadsheetFilterValue>,
    #[serde(default)]
    pub(super) case_sensitive: bool,
}

pub(super) fn parse_column_name(value: &str) -> anyhow::Result<u32> {
    let value = value.trim().trim_start_matches('$');
    anyhow::ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphabetic()),
        "spreadsheet column must be an Excel column name such as A or BG"
    );
    Ok(parse_a1_address(&format!("{value}1"))?.column)
}

pub(super) fn parse_row_conditions(
    conditions: Vec<SpreadsheetRowConditionInput>,
) -> anyhow::Result<Vec<SpreadsheetFilterCondition>> {
    conditions
        .into_iter()
        .map(|condition| {
            Ok(SpreadsheetFilterCondition {
                column: parse_column_name(&condition.column)?,
                operator: condition.operator,
                value: condition.value,
                case_sensitive: condition.case_sensitive,
            })
        })
        .collect()
}

pub(super) async fn execute_action(
    tool_name: &str,
    call_id: Uuid,
    input: Value,
    ctx: ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let mut result = execute_spreadsheet_backend(call_id, input, ctx).await?;
    if let Some(metadata) = result.metadata.as_object_mut() {
        metadata.insert("toolName".to_string(), Value::String(tool_name.to_string()));
        metadata.remove("action");
    }
    Ok(result)
}

pub(super) fn read_policy(paths: impl IntoIterator<Item = String>) -> ToolExecutionPolicy {
    ToolExecutionPolicy::read_only(
        paths
            .into_iter()
            .map(|path| tool_resource_key("file", &path))
            .collect(),
    )
}

pub(super) fn mutation_policy(
    reads: impl IntoIterator<Item = String>,
    writes: impl IntoIterator<Item = String>,
) -> ToolExecutionPolicy {
    let mut resource_keys = reads
        .into_iter()
        .chain(writes)
        .map(|path| tool_resource_key("file", &path))
        .collect::<Vec<_>>();
    resource_keys.sort();
    resource_keys.dedup();
    ToolExecutionPolicy {
        read_only: false,
        idempotent: false,
        parallel_safe: true,
        side_effect: ToolSideEffect::WorkspaceWrite,
        resource_keys,
    }
}

pub(super) fn observation_intent(paths: impl IntoIterator<Item = String>) -> ToolExecutionIntent {
    ToolExecutionIntent::observation(paths.into_iter().map(PathBuf::from))
}

pub(super) fn mutation_intent(
    reads: impl IntoIterator<Item = String>,
    writes: impl IntoIterator<Item = String>,
) -> ToolExecutionIntent {
    ToolExecutionIntent::workspace_mutation(writes.into_iter().map(PathBuf::from))
        .with_read_paths(reads.into_iter().map(PathBuf::from))
}
