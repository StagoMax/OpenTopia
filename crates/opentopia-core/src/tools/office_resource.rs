use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// A spreadsheet target is independent of its offline storage binding.
/// Workspace files are mutable under workspace policy; user attachments are
/// opaque, immutable sources addressed by attachment ID.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum OfficeResourceRef {
    WorkspaceFile { path: String },
    Attachment { attachment_id: Uuid },
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
                "path": path,
                "writeSupported": true
            }),
            Self::Attachment { attachment_id } => json!({
                "kind": "attachment",
                "backend": "offlineAttachment",
                "attachmentId": attachment_id,
                "writeSupported": false
            }),
        }
    }

    pub(super) fn supports_mutation(&self) -> bool {
        matches!(self, Self::WorkspaceFile { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_contract_distinguishes_mutable_files_from_immutable_attachments() {
        let workspace_file: OfficeResourceRef = serde_json::from_value(json!({
            "kind": "workspaceFile",
            "path": "reports/book.xlsx"
        }))
        .expect("workspace file resource");
        assert!(workspace_file.supports_mutation());
        assert_eq!(workspace_file.resource_key(), "file:reports/book.xlsx");

        let attachment_id = Uuid::new_v4();
        let attachment: OfficeResourceRef = serde_json::from_value(json!({
            "kind": "attachment",
            "attachmentId": attachment_id
        }))
        .expect("attachment resource");
        assert!(!attachment.supports_mutation());
        assert_eq!(
            attachment.resource_key(),
            format!("attachment:{attachment_id}")
        );
        assert_eq!(attachment.read_binding().unwrap().0, "attachmentId");
        assert!(attachment.offline_path().is_err());
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
