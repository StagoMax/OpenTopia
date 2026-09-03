use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// A document resource is only an address. Filesystem authority belongs to the
/// execution policy and sandbox, not to this schema, so both relative and
/// absolute paths are valid inputs when the active authority permits them.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(super) enum DocumentResourceRef {
    File {
        path: String,
    },
    Attachment {
        #[serde(rename = "attachmentId")]
        #[schemars(rename = "attachmentId")]
        attachment_id: Uuid,
    },
}

impl DocumentResourceRef {
    pub(super) fn file_path(&self) -> Option<&str> {
        match self {
            Self::File { path } if !path.trim().is_empty() => Some(path),
            Self::File { .. } | Self::Attachment { .. } => None,
        }
    }

    pub(super) fn read_binding_key(&self) -> &'static str {
        match self {
            Self::File { .. } => "path",
            Self::Attachment { .. } => "attachmentId",
        }
    }

    pub(super) fn read_binding(&self) -> anyhow::Result<(&'static str, Value)> {
        match self {
            Self::File { path } if !path.trim().is_empty() => {
                Ok((self.read_binding_key(), Value::String(path.clone())))
            }
            Self::File { .. } => anyhow::bail!("file.path must not be empty"),
            Self::Attachment { attachment_id } => Ok((
                self.read_binding_key(),
                Value::String(attachment_id.to_string()),
            )),
        }
    }

    pub(super) fn resource_key(&self) -> String {
        match self {
            Self::File { path } => format!("file:{path}"),
            Self::Attachment { attachment_id } => format!("attachment:{attachment_id}"),
        }
    }

    pub(super) fn descriptor(&self) -> Value {
        match self {
            Self::File { path } => json!({
                "kind": "file",
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
        let file: DocumentResourceRef = serde_json::from_value(json!({
            "kind": "file",
            "path": "reports/book.xlsx"
        }))
        .expect("file resource");
        assert_eq!(file.resource_key(), "file:reports/book.xlsx");

        let attachment_id = Uuid::new_v4();
        let attachment: DocumentResourceRef = serde_json::from_value(json!({
            "kind": "attachment",
            "attachmentId": attachment_id
        }))
        .expect("attachment resource");
        assert_eq!(
            attachment.resource_key(),
            format!("attachment:{attachment_id}")
        );
        assert_eq!(attachment.read_binding().unwrap().0, "attachmentId");
        assert!(attachment.file_path().is_none());

        assert!(serde_json::from_value::<DocumentResourceRef>(json!({
            "kind": "attachment",
            "attachment_id": attachment_id
        }))
        .is_err());

        let schema = serde_json::to_value(schemars::schema_for!(DocumentResourceRef)).unwrap();
        let schema_text = schema.to_string();
        assert!(schema_text.contains("\"file\""));
        assert!(!schema_text.contains("workspaceFile"));
        assert!(schema_text.contains("attachmentId"));
        assert!(!schema_text.contains("attachment_id"));
        assert!(serde_json::from_value::<DocumentResourceRef>(json!({
            "kind": "workspaceFile",
            "path": "reports/book.xlsx"
        }))
        .is_err());
    }

    #[test]
    fn unimplemented_live_session_is_not_part_of_the_resource_contract() {
        assert!(serde_json::from_value::<DocumentResourceRef>(json!({
            "kind": "liveSession",
            "sessionId": "session-1",
            "documentId": "workbook-1"
        }))
        .is_err());
    }
}
