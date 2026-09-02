//! Shared operation contracts for the progressive spreadsheet protocol.
//!
//! The legacy spreadsheet tool remains the implementation source of truth.
//! This module projects each legacy action schema into the argument object the
//! model is allowed to supply after the protocol has bound `action` and the
//! primary resource. Both `spreadsheet_describe` and `spreadsheet_execute` use
//! the same projection so the advertised and enforced contracts cannot drift.

use super::{SpreadsheetTool, Tool};
use anyhow::Context;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum SpreadsheetProtocolOperation {
    ListSheets,
    ReadRange,
    ReadRanges,
    ReadRows,
    ReadColumns,
    Find,
    FilterRows,
    Validate,
    FillTemplate,
    TransferRows,
    ExportDelimited,
    Write,
    WriteRows,
    WriteColumns,
    CopyRows,
    CopyColumns,
    Batch,
}

impl SpreadsheetProtocolOperation {
    pub(super) const ALL: [Self; 17] = [
        Self::ListSheets,
        Self::ReadRange,
        Self::ReadRanges,
        Self::ReadRows,
        Self::ReadColumns,
        Self::Find,
        Self::FilterRows,
        Self::Validate,
        Self::FillTemplate,
        Self::TransferRows,
        Self::ExportDelimited,
        Self::Write,
        Self::WriteRows,
        Self::WriteColumns,
        Self::CopyRows,
        Self::CopyColumns,
        Self::Batch,
    ];

    pub(super) fn legacy_action(self) -> &'static str {
        match self {
            Self::ListSheets => "list_sheets",
            Self::ReadRange => "read_range",
            Self::ReadRanges => "read_ranges",
            Self::ReadRows => "read_rows",
            Self::ReadColumns => "read_columns",
            Self::Find => "find",
            Self::FilterRows => "filter_rows",
            Self::Validate => "validate",
            Self::FillTemplate => "fill_template",
            Self::TransferRows => "transfer_rows",
            Self::ExportDelimited => "export_delimited",
            Self::Write => "write",
            Self::WriteRows => "write_rows",
            Self::WriteColumns => "write_columns",
            Self::CopyRows => "copy_rows",
            Self::CopyColumns => "copy_columns",
            Self::Batch => "batch",
        }
    }

    pub(super) fn is_mutation(self) -> bool {
        !matches!(
            self,
            Self::ListSheets
                | Self::ReadRange
                | Self::ReadRanges
                | Self::ReadRows
                | Self::ReadColumns
                | Self::Find
                | Self::FilterRows
                | Self::Validate
        )
    }

    pub(super) fn primary_binding(self) -> &'static str {
        match self {
            Self::ListSheets
            | Self::ReadRange
            | Self::ReadRanges
            | Self::ReadRows
            | Self::ReadColumns
            | Self::Find
            | Self::FilterRows
            | Self::Validate
            | Self::ExportDelimited => "path",
            Self::FillTemplate => "templatePath",
            Self::TransferRows => "sourcePath",
            Self::Write | Self::WriteRows | Self::WriteColumns | Self::Batch => "sourcePath",
            Self::CopyRows | Self::CopyColumns => "from",
        }
    }
}

/// One protocol-safe projection of a legacy spreadsheet action schema.
pub(super) struct SpreadsheetOperationContract {
    arguments_schema: Value,
}

impl SpreadsheetOperationContract {
    pub(super) fn new(operation: SpreadsheetProtocolOperation) -> anyhow::Result<Self> {
        let mut excluded = vec!["action", operation.primary_binding()];
        if !operation.is_mutation() {
            // Observation variants in the legacy union support either source.
            // The protocol resource owns that choice, so neither alternative
            // may remain model-supplied in the projected argument contract.
            excluded.extend(["path", "attachmentId"]);
        }
        excluded.sort_unstable();
        excluded.dedup();

        let arguments_schema = project_legacy_action_schema(operation, &excluded)?;
        Ok(Self { arguments_schema })
    }

    pub(super) fn arguments_schema(&self) -> &Value {
        &self.arguments_schema
    }

    pub(super) fn validate_arguments(&self, arguments: &Value) -> anyhow::Result<()> {
        if let Some(error) = crate::provider::tool_input_schema_error(
            &self.arguments_schema,
            arguments,
            "spreadsheet_execute.arguments",
        ) {
            anyhow::bail!(error);
        }
        Ok(())
    }
}

fn project_legacy_action_schema(
    operation: SpreadsheetProtocolOperation,
    excluded: &[&str],
) -> anyhow::Result<Value> {
    static LEGACY_SCHEMA: OnceLock<Value> = OnceLock::new();
    let schema = LEGACY_SCHEMA.get_or_init(|| SpreadsheetTool.schema());
    let mut branch = schema["oneOf"]
        .as_array()
        .and_then(|branches| {
            branches.iter().find(|branch| {
                branch["properties"]["action"]["enum"]
                    .as_array()
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(operation.legacy_action()))
                    })
            })
        })
        .cloned()
        .with_context(|| {
            format!(
                "spreadsheet protocol operation `{}` has no legacy action schema",
                operation.legacy_action()
            )
        })?;

    let object = branch
        .as_object_mut()
        .context("spreadsheet legacy action schema must be an object")?;
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .context("spreadsheet legacy action schema must define properties")?;
    for key in excluded {
        properties.remove(*key);
    }
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|value| value.as_str().is_none_or(|key| !excluded.contains(&key)));
    }
    Ok(branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn projection_removes_every_host_bound_argument() {
        for operation in SpreadsheetProtocolOperation::ALL {
            let contract =
                SpreadsheetOperationContract::new(operation).expect("operation contract");
            let properties = contract.arguments_schema()["properties"]
                .as_object()
                .expect("projected properties");
            assert!(
                !properties.contains_key("action"),
                "{} exposes action",
                operation.legacy_action()
            );
            assert!(
                !properties.contains_key(operation.primary_binding()),
                "{} exposes its primary binding",
                operation.legacy_action()
            );
            let required = contract.arguments_schema()["required"]
                .as_array()
                .expect("projected required list");
            assert!(!required.iter().any(|value| value == "action"));
            assert!(!required
                .iter()
                .any(|value| value == operation.primary_binding()));
            if !operation.is_mutation() {
                assert!(!properties.contains_key("attachmentId"));
                assert!(!required.iter().any(|value| value == "attachmentId"));
            }
        }

        let contract =
            SpreadsheetOperationContract::new(SpreadsheetProtocolOperation::FillTemplate)
                .expect("fill-template contract");
        let properties = contract.arguments_schema()["properties"]
            .as_object()
            .expect("projected properties");
        assert!(properties.contains_key("dataPath"));
        assert!(properties.contains_key("outputPath"));
    }

    #[test]
    fn projection_is_the_execute_argument_validator() {
        let contract = SpreadsheetOperationContract::new(SpreadsheetProtocolOperation::ReadRange)
            .expect("read contract");
        let valid = json!({
            "sheet": "Summary",
            "range": {
                "start": { "row": 0, "column": 0 },
                "end": { "row": 1, "column": 1 }
            }
        });
        assert!(contract.validate_arguments(&valid).is_ok());

        for forbidden in ["action", "path", "attachmentId"] {
            let mut invalid = valid.clone();
            invalid[forbidden] = json!("read_range");
            let error = contract
                .validate_arguments(&invalid)
                .expect_err("host-bound argument must be rejected");
            assert!(error.to_string().contains(forbidden));
        }
    }
}
