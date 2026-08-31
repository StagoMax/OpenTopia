use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// A spreadsheet input is independent of the operation's output destination.
/// Workspace files can be addressed directly; user attachments are immutable
/// inputs addressed by attachment ID. Whether a separate destination may be
/// written is decided by the execution policy and sandbox, not by this type.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum OfficeResourceRef {
    WorkspaceFile {
        path: String,
    },
    Attachment {
        // Keep the generated JSON Schema and serde's enum-field contract
        // identical. `rename_all_fields` is not interpreted consistently by
        // every schema consumer, which previously advertised `attachment_id`
        // while runtime deserialization required `attachmentId`.
        #[serde(rename = "attachmentId", alias = "attachment_id")]
        #[schemars(rename = "attachmentId")]
        attachment_id: Uuid,
    },
}

impl OfficeResourceRef {
    pub(super) fn read_binding_key(&self) -> &'static str {
        match self {
            Self::WorkspaceFile { .. } => "path",
            Self::Attachment { .. } => "attachmentId",
        }
    }

    pub(super) fn offline_path(&self) -> anyhow::Result<&str> {
        match self {
            Self::WorkspaceFile { path } if !path.trim().is_empty() => Ok(path),
            Self::WorkspaceFile { .. } => anyhow::bail!("workspaceFile.path must not be empty"),
            Self::Attachment { .. } => anyhow::bail!(
                "this operation currently requires a workspaceFile; attachments are read-only spreadsheet sources"
            ),
        }
    }

    pub(super) fn read_binding(&self) -> anyhow::Result<(&'static str, Value)> {
        match self {
            Self::WorkspaceFile { path } if !path.trim().is_empty() => {
                Ok((self.read_binding_key(), Value::String(path.clone())))
            }
            Self::WorkspaceFile { .. } => anyhow::bail!("workspaceFile.path must not be empty"),
            Self::Attachment { attachment_id } => Ok((
                self.read_binding_key(),
                Value::String(attachment_id.to_string()),
            )),
        }
    }

    pub(super) fn resource_key(&self) -> String {
        match self {
            Self::WorkspaceFile { path } => format!("file:{path}"),
            Self::Attachment { attachment_id } => format!("attachment:{attachment_id}"),
        }
    }

    pub(super) fn descriptor(&self) -> Value {
        match self {
            Self::WorkspaceFile { path } => json!({
                "kind": "workspaceFile",
                "backend": "offlineFile",
                "path": path
            }),
            Self::Attachment { attachment_id } => json!({
                "kind": "attachment",
                "backend": "offlineAttachment",
                "attachmentId": attachment_id
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_contract_distinguishes_paths_from_immutable_attachment_inputs() {
        let workspace_file: OfficeResourceRef = serde_json::from_value(json!({
            "kind": "workspaceFile",
            "path": "reports/book.xlsx"
        }))
        .expect("workspace file resource");
        assert_eq!(workspace_file.resource_key(), "file:reports/book.xlsx");

        let attachment_id = Uuid::new_v4();
        let attachment: OfficeResourceRef = serde_json::from_value(json!({
            "kind": "attachment",
            "attachmentId": attachment_id
        }))
        .expect("attachment resource");
        assert_eq!(
            attachment.resource_key(),
            format!("attachment:{attachment_id}")
        );
        assert_eq!(attachment.read_binding().unwrap().0, "attachmentId");
        assert!(attachment.offline_path().is_err());

        let legacy_attachment: OfficeResourceRef = serde_json::from_value(json!({
            "kind": "attachment",
            "attachment_id": attachment_id
        }))
        .expect("legacy snake-case attachment resource remains compatible");
        assert_eq!(legacy_attachment.resource_key(), attachment.resource_key());

        let schema = serde_json::to_value(schemars::schema_for!(OfficeResourceRef)).unwrap();
        let schema_text = schema.to_string();
        assert!(schema_text.contains("attachmentId"));
        assert!(!schema_text.contains("attachment_id"));
    }

    #[test]
    fn unimplemented_live_session_is_not_part_of_the_resource_contract() {
        assert!(serde_json::from_value::<OfficeResourceRef>(json!({
            "kind": "liveSession",
            "sessionId": "session-1",
            "documentId": "workbook-1"
        }))
        .is_err());
    }
}
