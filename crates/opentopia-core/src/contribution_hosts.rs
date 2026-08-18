use crate::capabilities::{ContributionKind, PluginContribution};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_APP_VIEW_MESSAGE_BYTES: usize = 256 * 1024;
pub const MEDIA_HANDLER_INVOCATION_API_VERSION: &str = "opentopia.mediaHandler.v1";
pub const MEDIA_HANDLER_RESULT_API_VERSION: &str = "opentopia.mediaHandlerResult.v1";
pub const MAX_MEDIA_HANDLER_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_MEDIA_HANDLER_OPTIONS_BYTES: usize = 64 * 1024;
pub const MAX_MEDIA_HANDLER_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MediaHandlerRuntime {
    McpV1 { server: String, tool: String },
    Builtin { adapter: String },
}

impl MediaHandlerRuntime {
    pub fn parse(value: &str) -> Result<Self, ContributionHostError> {
        let value = value.trim();
        if let Some(reference) = value.strip_prefix("mcp.v1:") {
            let (server, tool) = reference.split_once('/').ok_or_else(|| {
                ContributionHostError::InvalidRuntime(
                    "MCP handler runtime must use mcp.v1:<server>/<tool>".to_string(),
                )
            })?;
            ensure_runtime_segment("server", server)?;
            ensure_runtime_segment("tool", tool)?;
            return Ok(Self::McpV1 {
                server: server.to_string(),
                tool: tool.to_string(),
            });
        }
        if let Some(adapter) = value.strip_prefix("builtin:") {
            ensure_runtime_segment("builtin adapter", adapter)?;
            return Ok(Self::Builtin {
                adapter: adapter.to_string(),
            });
        }
        if value.starts_with("sidecar:") || value.starts_with("sidecar.v1:") {
            return Err(ContributionHostError::UnsupportedSidecarRuntime(
                value.to_string(),
            ));
        }
        Err(ContributionHostError::InvalidRuntime(format!(
            "unsupported handler runtime `{value}`; expected mcp.v1:<server>/<tool> or a host-owned builtin:<adapter>"
        )))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaHandlerOperation {
    Preview,
    LoadContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaHandlerSourceV1 {
    pub path: String,
    pub content_type: String,
    pub bytes: usize,
    pub content_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaHandlerInvocationV1 {
    pub api_version: String,
    pub operation: MediaHandlerOperation,
    pub contribution_id: String,
    pub source: MediaHandlerSourceV1,
    #[serde(default)]
    pub options: Value,
}

impl MediaHandlerInvocationV1 {
    pub fn new(
        operation: MediaHandlerOperation,
        contribution_id: impl Into<String>,
        relative_path: impl Into<String>,
        content_type: impl Into<String>,
        content: &[u8],
        options: Value,
    ) -> Result<Self, ContributionHostError> {
        if content.len() > MAX_MEDIA_HANDLER_INPUT_BYTES {
            return Err(ContributionHostError::HandlerInputTooLarge {
                actual: content.len(),
                maximum: MAX_MEDIA_HANDLER_INPUT_BYTES,
            });
        }
        if !options.is_object() {
            return Err(ContributionHostError::InvalidMessage(
                "media handler options must be a JSON object".to_string(),
            ));
        }
        let options_bytes = serde_json::to_vec(&options)
            .map_err(|error| ContributionHostError::InvalidMessage(error.to_string()))?
            .len();
        if options_bytes > MAX_MEDIA_HANDLER_OPTIONS_BYTES {
            return Err(ContributionHostError::HandlerOptionsTooLarge {
                actual: options_bytes,
                maximum: MAX_MEDIA_HANDLER_OPTIONS_BYTES,
            });
        }
        let path = relative_path.into();
        ensure_safe_relative_path(&path).map_err(ContributionHostError::InvalidMessage)?;
        let content_type = content_type.into();
        if content_type.trim().is_empty() || content_type.len() > 255 {
            return Err(ContributionHostError::InvalidMessage(
                "handler contentType must contain between 1 and 255 bytes".to_string(),
            ));
        }
        Ok(Self {
            api_version: MEDIA_HANDLER_INVOCATION_API_VERSION.to_string(),
            operation,
            contribution_id: contribution_id.into(),
            source: MediaHandlerSourceV1 {
                path,
                content_type,
                bytes: content.len(),
                content_base64: BASE64_STANDARD.encode(content),
            },
            options,
        })
    }

    pub fn into_mcp_arguments(self) -> Value {
        serde_json::json!({ "request": self })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaHandlerResultEnvelopeV1 {
    pub api_version: String,
    pub kind: MediaHandlerOperation,
    pub payload: Value,
}

impl MediaHandlerResultEnvelopeV1 {
    pub fn from_structured_content(
        value: Value,
        expected_kind: MediaHandlerOperation,
    ) -> Result<Self, ContributionHostError> {
        let result: Self = serde_json::from_value(value).map_err(|error| {
            ContributionHostError::InvalidHandlerOutput(format!(
                "structuredContent must be a media handler result envelope: {error}"
            ))
        })?;
        if result.api_version != MEDIA_HANDLER_RESULT_API_VERSION {
            return Err(ContributionHostError::InvalidHandlerOutput(format!(
                "unsupported result apiVersion `{}`",
                result.api_version
            )));
        }
        if result.kind != expected_kind {
            return Err(ContributionHostError::InvalidHandlerOutput(format!(
                "result kind {:?} does not match invocation kind {:?}",
                result.kind, expected_kind
            )));
        }
        let bytes = serde_json::to_vec(&result)
            .map_err(|error| ContributionHostError::InvalidHandlerOutput(error.to_string()))?
            .len();
        if bytes > MAX_MEDIA_HANDLER_OUTPUT_BYTES {
            return Err(ContributionHostError::HandlerOutputTooLarge {
                actual: bytes,
                maximum: MAX_MEDIA_HANDLER_OUTPUT_BYTES,
            });
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaHandlerDescriptor {
    pub contribution_id: String,
    pub plugin_id: String,
    pub local_id: String,
    pub kind: ContributionKind,
    pub extensions: Vec<String>,
    pub media_types: Vec<String>,
    pub priority: i32,
    pub runtime: String,
}

impl MediaHandlerDescriptor {
    pub fn from_contribution(
        contribution: &PluginContribution,
    ) -> Result<Self, ContributionHostError> {
        if !matches!(
            contribution.kind,
            ContributionKind::Previewer | ContributionKind::ContextLoader
        ) {
            return Err(ContributionHostError::WrongContributionKind {
                contribution_id: contribution.id.clone(),
                expected: "previewer or context_loader",
            });
        }
        let declaration = contribution.declaration.as_object().ok_or_else(|| {
            ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message: "handler declaration must be an object".to_string(),
            }
        })?;
        let extensions = normalized_string_array(declaration.get("extensions"), true)?;
        let media_types = normalized_string_array(
            declaration
                .get("mediaTypes")
                .or_else(|| declaration.get("media_types")),
            false,
        )?;
        if extensions.is_empty() && media_types.is_empty() {
            return Err(ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message: "handler must declare extensions or mediaTypes".to_string(),
            });
        }
        let runtime = declaration
            .get("runtime")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message: "handler runtime is required".to_string(),
            })?
            .to_string();
        let priority = declaration
            .get("priority")
            .and_then(Value::as_i64)
            .unwrap_or_default();
        let priority =
            i32::try_from(priority).map_err(|_| ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message: "handler priority is outside the i32 range".to_string(),
            })?;
        Ok(Self {
            contribution_id: contribution.id.clone(),
            plugin_id: contribution.plugin_id.clone(),
            local_id: contribution.local_id.clone(),
            kind: contribution.kind,
            extensions,
            media_types,
            priority,
            runtime,
        })
    }

    fn match_rank(&self, path: Option<&Path>, content_type: Option<&str>) -> Option<(u8, i32)> {
        let media_type = content_type
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        if media_type.as_ref().is_some_and(|media_type| {
            self.media_types
                .iter()
                .any(|candidate| candidate == media_type)
        }) {
            return Some((2, self.priority));
        }
        let extension = path
            .and_then(Path::extension)
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase);
        extension
            .filter(|extension| self.extensions.iter().any(|item| item == extension))
            .map(|_| (1, self.priority))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MediaHandlerSelection {
    None,
    Selected { handler: MediaHandlerDescriptor },
    Conflict { contribution_ids: Vec<String> },
}

#[derive(Debug, Clone, Default)]
pub struct ContributionHandlerRegistry {
    previewers: BTreeMap<String, MediaHandlerDescriptor>,
    context_loaders: BTreeMap<String, MediaHandlerDescriptor>,
}

impl ContributionHandlerRegistry {
    pub fn register(
        &mut self,
        contribution: &PluginContribution,
    ) -> Result<(), ContributionHostError> {
        let handler = MediaHandlerDescriptor::from_contribution(contribution)?;
        let target = match handler.kind {
            ContributionKind::Previewer => &mut self.previewers,
            ContributionKind::ContextLoader => &mut self.context_loaders,
            _ => unreachable!("handler kind checked above"),
        };
        if target
            .insert(handler.contribution_id.clone(), handler.clone())
            .is_some()
        {
            return Err(ContributionHostError::DuplicateContribution(
                handler.contribution_id,
            ));
        }
        Ok(())
    }

    pub fn previewers(&self) -> Vec<MediaHandlerDescriptor> {
        self.previewers.values().cloned().collect()
    }

    pub fn context_loaders(&self) -> Vec<MediaHandlerDescriptor> {
        self.context_loaders.values().cloned().collect()
    }

    pub fn select_previewer(
        &self,
        path: Option<&Path>,
        content_type: Option<&str>,
    ) -> MediaHandlerSelection {
        select_handler(self.previewers.values(), path, content_type)
    }

    pub fn select_context_loader(
        &self,
        path: Option<&Path>,
        content_type: Option<&str>,
    ) -> MediaHandlerSelection {
        select_handler(self.context_loaders.values(), path, content_type)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppViewSandbox {
    pub node_integration: bool,
    pub allow_popups: bool,
    pub allow_top_navigation: bool,
    pub allowed_host_apis: Vec<String>,
}

impl Default for AppViewSandbox {
    fn default() -> Self {
        Self {
            node_integration: false,
            allow_popups: false,
            allow_top_navigation: false,
            allowed_host_apis: vec!["appView.postMessage.v1".to_string()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppViewDescriptor {
    pub contribution_id: String,
    pub plugin_id: String,
    pub local_id: String,
    pub title: String,
    pub entry: String,
    pub allowed_channels: Vec<String>,
    pub sandbox: AppViewSandbox,
}

impl AppViewDescriptor {
    pub fn from_contribution(
        contribution: &PluginContribution,
    ) -> Result<Self, ContributionHostError> {
        if contribution.kind != ContributionKind::App {
            return Err(ContributionHostError::WrongContributionKind {
                contribution_id: contribution.id.clone(),
                expected: "app",
            });
        }
        let declaration = contribution.declaration.as_object().ok_or_else(|| {
            ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message: "app declaration must be an object".to_string(),
            }
        })?;
        let entry = declaration
            .get("entry")
            .or_else(|| declaration.get("path"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message: "app entry is required".to_string(),
            })?;
        ensure_safe_relative_path(entry).map_err(|message| {
            ContributionHostError::InvalidDeclaration {
                contribution_id: contribution.id.clone(),
                message,
            }
        })?;
        let title = declaration
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&contribution.local_id)
            .to_string();
        let allowed_channels = normalized_string_array(
            declaration
                .get("allowedChannels")
                .or_else(|| declaration.get("allowed_channels")),
            false,
        )?;
        Ok(Self {
            contribution_id: contribution.id.clone(),
            plugin_id: contribution.plugin_id.clone(),
            local_id: contribution.local_id.clone(),
            title,
            entry: entry.to_string(),
            allowed_channels,
            sandbox: AppViewSandbox::default(),
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppViewSessionStatus {
    Ready,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppViewSession {
    pub session_id: Uuid,
    pub thread_id: Uuid,
    pub descriptor: AppViewDescriptor,
    pub status: AppViewSessionStatus,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppViewMessage {
    pub session_id: Uuid,
    pub channel: String,
    pub payload: Value,
    pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct AppViewHost {
    apps: BTreeMap<String, AppViewDescriptor>,
    sessions: BTreeMap<Uuid, AppViewSession>,
}

impl AppViewHost {
    pub fn register(
        &mut self,
        contribution: &PluginContribution,
    ) -> Result<(), ContributionHostError> {
        let app = AppViewDescriptor::from_contribution(contribution)?;
        if self
            .apps
            .insert(app.contribution_id.clone(), app.clone())
            .is_some()
        {
            return Err(ContributionHostError::DuplicateContribution(
                app.contribution_id,
            ));
        }
        Ok(())
    }

    pub fn apps(&self) -> Vec<AppViewDescriptor> {
        self.apps.values().cloned().collect()
    }

    pub fn session(&self, session_id: Uuid) -> Option<AppViewSession> {
        self.sessions.get(&session_id).cloned()
    }

    pub fn start_contribution(
        &mut self,
        thread_id: Uuid,
        contribution: &PluginContribution,
    ) -> Result<AppViewSession, ContributionHostError> {
        let app = AppViewDescriptor::from_contribution(contribution)?;
        let contribution_id = app.contribution_id.clone();
        self.apps.insert(contribution_id.clone(), app);
        self.start(thread_id, &contribution_id)
    }

    pub fn start(
        &mut self,
        thread_id: Uuid,
        contribution_id: &str,
    ) -> Result<AppViewSession, ContributionHostError> {
        let descriptor = self.apps.get(contribution_id).cloned().ok_or_else(|| {
            ContributionHostError::UnknownContribution(contribution_id.to_string())
        })?;
        let session = AppViewSession {
            session_id: Uuid::new_v4(),
            thread_id,
            descriptor,
            status: AppViewSessionStatus::Ready,
            started_at: Utc::now(),
            stopped_at: None,
        };
        self.sessions.insert(session.session_id, session.clone());
        Ok(session)
    }

    pub fn post_message(
        &self,
        session_id: Uuid,
        channel: &str,
        payload: Value,
    ) -> Result<AppViewMessage, ContributionHostError> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or(ContributionHostError::UnknownSession(session_id))?;
        if session.status != AppViewSessionStatus::Ready {
            return Err(ContributionHostError::StoppedSession(session_id));
        }
        if !session
            .descriptor
            .allowed_channels
            .iter()
            .any(|allowed| allowed == channel)
        {
            return Err(ContributionHostError::ChannelNotAllowed {
                session_id,
                channel: channel.to_string(),
            });
        }
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| ContributionHostError::InvalidMessage(error.to_string()))?
            .len();
        if bytes > MAX_APP_VIEW_MESSAGE_BYTES {
            return Err(ContributionHostError::MessageTooLarge {
                actual: bytes,
                maximum: MAX_APP_VIEW_MESSAGE_BYTES,
            });
        }
        Ok(AppViewMessage {
            session_id,
            channel: channel.to_string(),
            payload,
            sent_at: Utc::now(),
        })
    }

    pub fn stop(&mut self, session_id: Uuid) -> Result<AppViewSession, ContributionHostError> {
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or(ContributionHostError::UnknownSession(session_id))?;
        session.status = AppViewSessionStatus::Stopped;
        session.stopped_at.get_or_insert_with(Utc::now);
        Ok(session.clone())
    }
}

#[derive(Debug, Error)]
pub enum ContributionHostError {
    #[error("contribution is already registered: {0}")]
    DuplicateContribution(String),
    #[error("unknown contribution: {0}")]
    UnknownContribution(String),
    #[error("contribution {contribution_id} is not an {expected} contribution")]
    WrongContributionKind {
        contribution_id: String,
        expected: &'static str,
    },
    #[error("invalid contribution {contribution_id}: {message}")]
    InvalidDeclaration {
        contribution_id: String,
        message: String,
    },
    #[error("unknown app view session: {0}")]
    UnknownSession(Uuid),
    #[error("app view session is stopped: {0}")]
    StoppedSession(Uuid),
    #[error("channel `{channel}` is not allowed for app view session {session_id}")]
    ChannelNotAllowed { session_id: Uuid, channel: String },
    #[error("app view message is {actual} bytes; maximum is {maximum}")]
    MessageTooLarge { actual: usize, maximum: usize },
    #[error("invalid app view message: {0}")]
    InvalidMessage(String),
    #[error("invalid media handler runtime: {0}")]
    InvalidRuntime(String),
    #[error("sidecar media handler runtime is not supported: {0}")]
    UnsupportedSidecarRuntime(String),
    #[error("media handler input is {actual} bytes; maximum is {maximum}")]
    HandlerInputTooLarge { actual: usize, maximum: usize },
    #[error("media handler options are {actual} bytes; maximum is {maximum}")]
    HandlerOptionsTooLarge { actual: usize, maximum: usize },
    #[error("media handler output is {actual} bytes; maximum is {maximum}")]
    HandlerOutputTooLarge { actual: usize, maximum: usize },
    #[error("invalid media handler output: {0}")]
    InvalidHandlerOutput(String),
}

fn ensure_runtime_segment(label: &str, value: &str) -> Result<(), ContributionHostError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(ContributionHostError::InvalidRuntime(format!(
            "{label} must be 1-128 ASCII letters, digits, dots, underscores, or hyphens"
        )));
    }
    Ok(())
}

fn select_handler<'a>(
    handlers: impl Iterator<Item = &'a MediaHandlerDescriptor>,
    path: Option<&Path>,
    content_type: Option<&str>,
) -> MediaHandlerSelection {
    let mut matches = handlers
        .filter_map(|handler| {
            handler
                .match_rank(path, content_type)
                .map(|rank| (rank, handler))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.contribution_id.cmp(&right.1.contribution_id))
    });
    let Some((best_rank, best)) = matches.first() else {
        return MediaHandlerSelection::None;
    };
    let conflicts = matches
        .iter()
        .take_while(|(rank, _)| rank == best_rank)
        .map(|(_, handler)| handler.contribution_id.clone())
        .collect::<Vec<_>>();
    if conflicts.len() > 1 {
        MediaHandlerSelection::Conflict {
            contribution_ids: conflicts,
        }
    } else {
        MediaHandlerSelection::Selected {
            handler: (*best).clone(),
        }
    }
}

fn normalized_string_array(
    value: Option<&Value>,
    trim_dot: bool,
) -> Result<Vec<String>, ContributionHostError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        ContributionHostError::InvalidMessage("expected a string array".to_string())
    })?;
    let mut normalized = BTreeSet::new();
    for value in values {
        let value = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ContributionHostError::InvalidMessage(
                    "handler arrays may contain only non-empty strings".to_string(),
                )
            })?;
        let value = if trim_dot {
            value.trim_start_matches('.')
        } else {
            value
        };
        normalized.insert(value.to_ascii_lowercase());
    }
    Ok(normalized.into_iter().collect())
}

fn ensure_safe_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("app entry must stay inside the plugin package".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{ContributionOrigin, PluginPermission};
    use serde_json::json;

    fn contribution(kind: ContributionKind, id: &str, declaration: Value) -> PluginContribution {
        PluginContribution {
            id: format!("plugin/{id}"),
            plugin_id: "plugin".to_string(),
            local_id: id.to_string(),
            kind,
            origin: ContributionOrigin::OpenTopia,
            api_version: "1".to_string(),
            required_host_capabilities: Vec::new(),
            permissions: Vec::<PluginPermission>::new(),
            configuration_schema: None,
            declaration,
        }
    }

    #[test]
    fn media_handlers_prefer_media_type_then_priority_and_report_ties() {
        let mut registry = ContributionHandlerRegistry::default();
        registry
            .register(&contribution(
                ContributionKind::Previewer,
                "extension",
                json!({"extensions": [".xlsx"], "runtime": "mcp.v1:office/preview", "priority": 50}),
            ))
            .unwrap();
        registry
            .register(&contribution(
                ContributionKind::Previewer,
                "media",
                json!({"mediaTypes": ["application/x-sheet"], "runtime": "mcp.v1:office/preview"}),
            ))
            .unwrap();
        let selected = registry.select_previewer(
            Some(Path::new("report.xlsx")),
            Some("application/x-sheet; charset=binary"),
        );
        assert!(matches!(
            selected,
            MediaHandlerSelection::Selected { handler } if handler.local_id == "media"
        ));

        registry
            .register(&contribution(
                ContributionKind::Previewer,
                "media-two",
                json!({"mediaTypes": ["application/x-sheet"], "runtime": "mcp.v1:office/preview"}),
            ))
            .unwrap();
        assert!(matches!(
            registry.select_previewer(None, Some("application/x-sheet")),
            MediaHandlerSelection::Conflict { contribution_ids } if contribution_ids.len() == 2
        ));
    }

    #[test]
    fn media_handler_runtime_is_versioned_and_rejects_sidecars() {
        assert_eq!(
            MediaHandlerRuntime::parse("mcp.v1:documents/render").unwrap(),
            MediaHandlerRuntime::McpV1 {
                server: "documents".to_string(),
                tool: "render".to_string(),
            }
        );
        assert!(matches!(
            MediaHandlerRuntime::parse("mcp:render"),
            Err(ContributionHostError::InvalidRuntime(_))
        ));
        assert!(matches!(
            MediaHandlerRuntime::parse("sidecar.v1:node ./render.js"),
            Err(ContributionHostError::UnsupportedSidecarRuntime(_))
        ));
    }

    #[test]
    fn media_handler_invocation_is_host_fed_and_bounded() {
        let invocation = MediaHandlerInvocationV1::new(
            MediaHandlerOperation::LoadContext,
            "plugin/text",
            "docs/readme.md",
            "text/markdown",
            b"hello",
            json!({"maxItems": 2}),
        )
        .unwrap();
        assert_eq!(invocation.source.content_base64, "aGVsbG8=");
        assert!(!serde_json::to_string(&invocation).unwrap().contains("J:\\"));
        assert!(matches!(
            MediaHandlerInvocationV1::new(
                MediaHandlerOperation::Preview,
                "plugin/preview",
                "../secret.txt",
                "text/plain",
                b"secret",
                json!({}),
            ),
            Err(ContributionHostError::InvalidMessage(_))
        ));
        assert!(MediaHandlerInvocationV1::new(
            MediaHandlerOperation::Preview,
            "plugin/preview",
            "safe.txt",
            "text/plain",
            b"text",
            Value::Null,
        )
        .is_err());
    }

    #[test]
    fn media_handler_result_requires_a_bounded_versioned_envelope() {
        let result = MediaHandlerResultEnvelopeV1::from_structured_content(
            json!({
                "apiVersion": MEDIA_HANDLER_RESULT_API_VERSION,
                "kind": "preview",
                "payload": {"type": "text", "text": "ready"}
            }),
            MediaHandlerOperation::Preview,
        )
        .unwrap();
        assert_eq!(result.payload["text"], "ready");
        assert!(MediaHandlerResultEnvelopeV1::from_structured_content(
            json!({
                "apiVersion": MEDIA_HANDLER_RESULT_API_VERSION,
                "kind": "load_context",
                "payload": null
            }),
            MediaHandlerOperation::Preview,
        )
        .is_err());
    }

    #[test]
    fn app_views_are_package_relative_sandboxed_and_channel_bounded() {
        let app = contribution(
            ContributionKind::App,
            "dashboard",
            json!({
                "entry": "apps/dashboard/index.html",
                "title": "Dashboard",
                "allowedChannels": ["refresh"]
            }),
        );
        let mut host = AppViewHost::default();
        host.register(&app).unwrap();
        let session = host.start(Uuid::new_v4(), "plugin/dashboard").unwrap();
        assert!(!session.descriptor.sandbox.node_integration);
        assert!(host
            .post_message(session.session_id, "refresh", json!({"page": 1}))
            .is_ok());
        assert!(matches!(
            host.post_message(session.session_id, "shell", Value::Null),
            Err(ContributionHostError::ChannelNotAllowed { .. })
        ));
        host.stop(session.session_id).unwrap();
        assert!(matches!(
            host.post_message(session.session_id, "refresh", Value::Null),
            Err(ContributionHostError::StoppedSession(_))
        ));
    }

    #[test]
    fn app_views_reject_package_escape() {
        let app = contribution(
            ContributionKind::App,
            "escape",
            json!({"entry": "../outside.html"}),
        );
        assert!(matches!(
            AppViewDescriptor::from_contribution(&app),
            Err(ContributionHostError::InvalidDeclaration { .. })
        ));
    }

    #[test]
    fn app_views_default_to_no_message_channels() {
        let app = contribution(
            ContributionKind::App,
            "silent",
            json!({"entry": "apps/silent.html"}),
        );
        let mut host = AppViewHost::default();
        let session = host.start_contribution(Uuid::new_v4(), &app).unwrap();
        assert!(matches!(
            host.post_message(session.session_id, "refresh", Value::Null),
            Err(ContributionHostError::ChannelNotAllowed { .. })
        ));
    }
}
