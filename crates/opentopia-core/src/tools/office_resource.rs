use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

/// A document target is deliberately independent of the execution backend.
/// Offline files and attachments are usable now; `liveSession` is the stable
/// contract for a future Excel/Office add-in and is never silently treated as
/// a filesystem path.
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
        attachment_id: Uuid,
    },
    LiveSession {
        session_id: String,
        document_id: String,
    },
}

impl OfficeResourceRef {
    pub(super) fn offline_path(&self) -> anyhow::Result<&str> {
        match self {
            Self::WorkspaceFile { path } if !path.trim().is_empty() => Ok(path),
            Self::WorkspaceFile { .. } => anyhow::bail!("workspaceFile.path must not be empty"),
            Self::Attachment { .. } => anyhow::bail!(
                "this operation currently requires a workspaceFile; attachments are read-only spreadsheet sources"
            ),
            Self::LiveSession { .. } => anyhow::bail!(
                "liveSession resources require an Office live-session provider, which is not connected in this build"
            ),
        }
    }

    pub(super) fn read_binding(&self) -> anyhow::Result<(&'static str, Value)> {
        match self {
            Self::WorkspaceFile { path } if !path.trim().is_empty() => {
                Ok(("path", Value::String(path.clone())))
            }
            Self::WorkspaceFile { .. } => anyhow::bail!("workspaceFile.path must not be empty"),
            Self::Attachment { attachment_id } => {
                Ok(("attachmentId", Value::String(attachment_id.to_string())))
            }
            Self::LiveSession { .. } => anyhow::bail!(
                "liveSession resources require an Office live-session provider, which is not connected in this build"
            ),
        }
    }

    pub(super) fn resource_key(&self) -> String {
        match self {
            Self::WorkspaceFile { path } => format!("file:{path}"),
            Self::Attachment { attachment_id } => format!("attachment:{attachment_id}"),
            Self::LiveSession {
                session_id,
                document_id,
            } => format!("live-session:{session_id}:{document_id}"),
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
            Self::LiveSession {
                session_id,
                document_id,
            } => json!({
                "kind": "liveSession",
                "backend": "liveOfficeSession",
                "sessionId": session_id,
                "documentId": document_id,
                "available": false,
                "reason": "No Office live-session provider is connected."
            }),
        }
    }

    pub(super) fn ensure_available(&self) -> anyhow::Result<()> {
        if matches!(self, Self::LiveSession { .. }) {
            anyhow::bail!(
                "liveSession resources require an Office live-session provider, which is not connected in this build"
            );
        }
        Ok(())
    }
}
