use super::{
    decode_typed_tool_input, derived_tool_schema, enforce_policy_decision, mcp_content_parts,
    tool_resource_key, Tool, ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect, TypedTool,
};
use crate::context_sources::{
    load_context_sources, ContextSourceKind, ContextSourcePolicy, LoadedContextSource,
};
use crate::execution_authorization::ToolExecutionIntent;
use crate::mcp::McpToolDescriptor;
use crate::model::{MessagePart, ModelContentPart, ToolCall, ToolResult};
use crate::policy::{PolicyDecision, ToolPermissionDescriptor};
use crate::tool_state::ToolStateStore;
use anyhow::Context;
use async_trait::async_trait;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(super) const ATTACHMENT_RESULT_BOUNDARY: &str = "Attachment content:";
const ATTACHMENT_READ_WINDOW_CHARS: usize = 16_000;

#[derive(Debug, Clone)]
enum StoredAttachment {
    InlineImage {
        id: Uuid,
        content_type: String,
        data: Vec<u8>,
        name: String,
    },
    ContextSource {
        id: Uuid,
        path: PathBuf,
        kind: ContextSourceKind,
        content_type: String,
        name: String,
        bytes: u64,
    },
}

impl StoredAttachment {
    fn id(&self) -> Uuid {
        match self {
            Self::InlineImage { id, .. } | Self::ContextSource { id, .. } => *id,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::InlineImage { name, .. } | Self::ContextSource { name, .. } => name,
        }
    }

    fn content_type(&self) -> &str {
        match self {
            Self::InlineImage { content_type, .. } | Self::ContextSource { content_type, .. } => {
                content_type
            }
        }
    }

    fn bytes(&self) -> u64 {
        match self {
            Self::InlineImage { data, .. } => data.len() as u64,
            Self::ContextSource { bytes, .. } => *bytes,
        }
    }
}

fn attachment_context(ctx: &ToolInvocationContext) -> anyhow::Result<(ToolStateStore, Uuid)> {
    let store = ctx
        .state
        .clone()
        .context("attachment tools require a persistent session store")?;
    let thread_id = ctx
        .thread_id
        .context("attachment tools require a thread context")?;
    Ok((store, thread_id))
}

fn find_stored_attachment(
    ctx: &ToolInvocationContext,
    attachment_id: Uuid,
) -> anyhow::Result<StoredAttachment> {
    let (store, thread_id) = attachment_context(ctx)?;
    let messages = store.list_messages(thread_id)?;
    for message in messages.iter().rev() {
        for part in message.parts.iter().rev() {
            match part {
                MessagePart::Image {
                    id: Some(id),
                    content_type,
                    data,
                    name,
                } if *id == attachment_id => {
                    return Ok(StoredAttachment::InlineImage {
                        id: *id,
                        content_type: content_type.clone(),
                        data: data.clone(),
                        name: name.clone().unwrap_or_else(|| "image".to_string()),
                    });
                }
                MessagePart::SourceRef { source, .. } if source.id == attachment_id => {
                    return Ok(StoredAttachment::ContextSource {
                        id: source.id,
                        path: source.path.clone(),
                        kind: source.kind,
                        content_type: source.content_type.clone(),
                        name: source.name.clone(),
                        bytes: source.bytes,
                    });
                }
                _ => {}
            }
        }
    }
    anyhow::bail!("attachment {attachment_id} is not available in this thread")
}

/// Resolve the host-selected source behind an attachment ID without reading it.
///
/// The returned path is only a locator. Callers must pass it through their
/// normal execution-intent, policy, sandbox, and bounded-read path before
/// touching the file.
pub(super) fn stored_attachment_read_path(
    ctx: &ToolInvocationContext,
    attachment_id: Uuid,
) -> anyhow::Result<PathBuf> {
    match find_stored_attachment(ctx, attachment_id)? {
        StoredAttachment::ContextSource { path, .. } => {
            anyhow::ensure!(
                !path.as_os_str().is_empty(),
                "attachment {attachment_id} has no readable source path"
            );
            Ok(path)
        }
        StoredAttachment::InlineImage { .. } => {
            anyhow::bail!("attachment {attachment_id} is an inline image, not a file source")
        }
    }
}

async fn load_stored_context_source(
    attachment: &StoredAttachment,
) -> anyhow::Result<LoadedContextSource> {
    let StoredAttachment::ContextSource { path, .. } = attachment else {
        anyhow::bail!("attachment is not a context source")
    };
    let path = path.clone();
    tokio::task::spawn_blocking(move || {
        load_context_sources(&[path], &ContextSourcePolicy::default())
            .map_err(anyhow::Error::from)
            .and_then(|mut sources| {
                sources
                    .pop()
                    .context("attachment source disappeared while it was being read")
            })
    })
    .await
    .context("attachment read task failed")?
}

#[derive(Debug)]
pub(super) struct StoredAttachmentFile {
    id: Uuid,
    path: PathBuf,
    name: String,
    content_type: String,
    pub(super) data: Vec<u8>,
}

impl StoredAttachmentFile {
    pub(super) fn original_logical_path(&self, fallback_extension: &str) -> PathBuf {
        let name = PathBuf::from(&self.name);
        if name.extension().is_some() {
            return name;
        }
        if self.path.extension().is_some() {
            return self.path.clone();
        }
        PathBuf::from(format!("attachment-{}.{}", self.id, fallback_extension))
    }

    pub(super) fn logical_path(&self, expected_extension: &str) -> PathBuf {
        let name = PathBuf::from(&self.name);
        if name.extension().is_some_and(|extension| {
            extension
                .to_str()
                .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
        }) {
            return name;
        }
        if self.path.extension().is_some_and(|extension| {
            extension
                .to_str()
                .is_some_and(|extension| extension.eq_ignore_ascii_case(expected_extension))
        }) {
            return self.path.clone();
        }
        PathBuf::from(format!("attachment-{}.{}", self.id, expected_extension))
    }

    pub(super) fn metadata(&self) -> Value {
        json!({
            "provenance": "user_attachment",
            "attachmentId": self.id,
            "name": self.name,
            "contentType": self.content_type,
            "bytes": self.data.len()
        })
    }
}

pub(super) async fn read_stored_attachment_file(
    ctx: &ToolInvocationContext,
    attachment_id: Uuid,
    max_bytes: u64,
) -> anyhow::Result<StoredAttachmentFile> {
    let attachment = find_stored_attachment(ctx, attachment_id)?;
    let StoredAttachment::ContextSource {
        id,
        path,
        content_type,
        name,
        ..
    } = attachment
    else {
        anyhow::bail!("attachment {attachment_id} is an inline image, not an Office file")
    };
    tokio::task::spawn_blocking(move || {
        let source_metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect attachment {}", path.display()))?;
        anyhow::ensure!(
            source_metadata.file_type().is_file(),
            "attachment {} is not a regular file",
            path.display()
        );
        let resolved = path
            .canonicalize()
            .with_context(|| format!("attachment {} is no longer available", path.display()))?;
        let metadata = fs::symlink_metadata(&resolved)
            .with_context(|| format!("failed to inspect attachment {}", resolved.display()))?;
        anyhow::ensure!(
            metadata.file_type().is_file(),
            "attachment {} is not a regular file",
            resolved.display()
        );
        anyhow::ensure!(
            metadata.len() <= max_bytes,
            "attachment {} is {} bytes; limit is {} bytes",
            name,
            metadata.len(),
            max_bytes
        );
        let data = fs::read(&resolved)
            .with_context(|| format!("failed to read attachment {}", resolved.display()))?;
        Ok(StoredAttachmentFile {
            id,
            path: resolved,
            name,
            content_type,
            data,
        })
    })
    .await
    .context("attachment file read task failed")?
}

pub(super) fn insert_attachment_provenance(metadata: &mut Value, attachment: &Value) {
    let Some(target) = metadata.as_object_mut() else {
        return;
    };
    let Some(source) = attachment.as_object() else {
        return;
    };
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReadAttachmentInput {
    /// Opaque attachment ID shown in the user message's attachment manifest.
    pub(super) attachment_id: String,
    /// Character offset for text attachments. Defaults to 0.
    #[serde(default)]
    pub(super) offset: u64,
    /// Maximum characters to return, capped at 16000.
    #[serde(default)]
    #[schemars(range(min = 1, max = 16000))]
    pub(super) limit: Option<u64>,
}

pub struct ReadAttachmentTool;

#[async_trait]
impl TypedTool for ReadAttachmentTool {
    type Input = ReadAttachmentInput;

    fn name(&self) -> &str {
        "read_attachment"
    }

    fn description(&self) -> &str {
        "Read a user-attached text or document source by its opaque attachmentId. Use view_attachment for images."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy::read_only(vec![tool_resource_key("attachment", &input.attachment_id)])
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let attachment_id = Uuid::parse_str(input.attachment_id.trim())
            .context("attachmentId must be a UUID from the attachment manifest")?;
        let attachment = find_stored_attachment(&ctx, attachment_id)?;
        if matches!(attachment, StoredAttachment::InlineImage { .. }) {
            anyhow::bail!(
                "{} is an image; call view_attachment instead",
                attachment.name()
            );
        }
        if matches!(
            attachment,
            StoredAttachment::ContextSource {
                kind: ContextSourceKind::Image,
                ..
            }
        ) {
            anyhow::bail!(
                "{} is an image; call view_attachment instead",
                attachment.name()
            );
        }

        let source = load_stored_context_source(&attachment).await?;
        let offset = input.offset as usize;
        let limit = input
            .limit
            .map_or(ATTACHMENT_READ_WINDOW_CHARS, |value| value as usize)
            .clamp(1, ATTACHMENT_READ_WINDOW_CHARS);
        let mut content = vec![ModelContentPart::text(ATTACHMENT_RESULT_BOUNDARY)];
        let output = if let Some(text) = source.text {
            let total_chars = text.chars().count();
            let window = text.chars().skip(offset).take(limit).collect::<String>();
            let read_to = offset.saturating_add(window.chars().count());
            let next_offset = (read_to < total_chars).then_some(read_to);
            content.push(ModelContentPart::text(window.clone()));
            format!(
                "{ATTACHMENT_RESULT_BOUNDARY}\nAttachment {} ({}) characters {offset}-{} of {total_chars}.{}\n\n{window}",
                attachment.name(),
                attachment.id(),
                read_to.saturating_sub(1),
                next_offset
                    .map(|next| format!(" Call read_attachment again with offset {next} for the rest."))
                    .unwrap_or_default(),
            )
        } else {
            content.extend(source.content_or_legacy_text());
            format!(
                "{ATTACHMENT_RESULT_BOUNDARY}\nAttachment {} ({}, {}, {} bytes) is available as a typed resource in this tool result.",
                attachment.name(),
                attachment.id(),
                attachment.content_type(),
                attachment.bytes(),
            )
        };

        Ok(ToolResult {
            call_id,
            output,
            content,
            metadata: json!({
                "success": true,
                "provenance": "user_attachment",
                "attachmentId": attachment.id(),
                "name": attachment.name(),
                "contentType": attachment.content_type(),
                "bytes": attachment.bytes(),
                "offset": offset,
            }),
        })
    }
}

impl_typed_tool!(ReadAttachmentTool);

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ViewAttachmentInput {
    /// Opaque image attachment ID shown in the user message's attachment manifest.
    pub(super) attachment_id: String,
    /// Optional question or focus for a text-only external attachment inspector.
    #[serde(default)]
    pub(super) focus: Option<String>,
}

pub struct ViewAttachmentTool;

#[async_trait]
impl TypedTool for ViewAttachmentTool {
    type Input = ViewAttachmentInput;

    fn name(&self) -> &str {
        "view_attachment"
    }

    fn description(&self) -> &str {
        "View a user-attached image by its opaque attachmentId. The runtime delivers native image content to vision-capable models; for text-only models it may use an explicitly declared compatible MCP attachment inspector. Optionally provide focus to describe what should be inspected."
    }

    fn execution_policy(&self, input: &Self::Input) -> ToolExecutionPolicy {
        ToolExecutionPolicy {
            read_only: true,
            idempotent: true,
            parallel_safe: true,
            side_effect: ToolSideEffect::External,
            resource_keys: vec![tool_resource_key("attachment", &input.attachment_id)],
        }
    }

    async fn execute_typed(
        &self,
        call_id: Uuid,
        input: Self::Input,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let attachment_id = Uuid::parse_str(input.attachment_id.trim())
            .context("attachmentId must be a UUID from the attachment manifest")?;
        let attachment = find_stored_attachment(&ctx, attachment_id)?;
        let (content_type, data) = attachment_image_bytes(&attachment).await?;
        if !ctx.model_supports_vision {
            return inspect_attachment_through_mcp(
                call_id,
                &attachment,
                &content_type,
                &data,
                input.focus.as_deref(),
                &ctx,
            )
            .await;
        }
        let output = format!(
            "{ATTACHMENT_RESULT_BOUNDARY}\nImage attachment {} ({}, {}, {} bytes) follows as typed image data.",
            attachment.name(),
            attachment.id(),
            content_type,
            data.len(),
        );
        Ok(ToolResult {
            call_id,
            output: output.clone(),
            content: vec![
                ModelContentPart::text(output),
                ModelContentPart::image(content_type, data),
            ],
            metadata: json!({
                "success": true,
                "provenance": "user_attachment",
                "attachmentId": attachment.id(),
                "name": attachment.name(),
                "contentType": attachment.content_type(),
                "bytes": attachment.bytes(),
            }),
        })
    }
}

impl_typed_tool!(ViewAttachmentTool);

pub(super) const MCP_IMAGE_INSPECTION_CAPABILITY: &str = "media.image.inspect/v1";
const OPENTOPIA_MCP_CAPABILITIES_META_KEY: &str = "com.opentopia/capabilities";
const DEFAULT_ATTACHMENT_INSPECTION_FOCUS: &str =
    "Describe the image accurately and answer the user's request about it.";
const MAX_ATTACHMENT_INSPECTION_FOCUS_CHARS: usize = 4_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpImageInputEncoding {
    ObjectBase64,
    Base64,
    DataUrl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct McpImageInspectionBinding {
    pub(super) priority: i32,
    pub(super) image_pointer: String,
    pub(super) focus_pointer: Option<String>,
    pub(super) image_encoding: McpImageInputEncoding,
}

pub(crate) fn mcp_tool_declares_image_inspection(tool: &McpToolDescriptor) -> bool {
    match tool.meta.get(OPENTOPIA_MCP_CAPABILITIES_META_KEY) {
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| item.as_str() == Some(MCP_IMAGE_INSPECTION_CAPABILITY)),
        Some(Value::Object(items)) => items.contains_key(MCP_IMAGE_INSPECTION_CAPABILITY),
        _ => false,
    }
}

pub(super) fn parse_mcp_image_inspection_binding(
    tool: &McpToolDescriptor,
) -> anyhow::Result<Option<McpImageInspectionBinding>> {
    let Some(capabilities) = tool.meta.get(OPENTOPIA_MCP_CAPABILITIES_META_KEY) else {
        return Ok(None);
    };
    let declaration = match capabilities {
        Value::Array(items)
            if items
                .iter()
                .any(|item| item.as_str() == Some(MCP_IMAGE_INSPECTION_CAPABILITY)) =>
        {
            Value::Object(serde_json::Map::new())
        }
        Value::Object(items) => match items.get(MCP_IMAGE_INSPECTION_CAPABILITY) {
            Some(Value::Bool(true)) => Value::Object(serde_json::Map::new()),
            Some(value @ Value::Object(_)) => value.clone(),
            Some(_) => anyhow::bail!(
                "MCP tool `{}` declares `{MCP_IMAGE_INSPECTION_CAPABILITY}` with an invalid object",
                tool.public_name
            ),
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    let declaration = declaration
        .as_object()
        .expect("capability declaration normalized to an object");
    let priority = declaration
        .get("priority")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let priority = i32::try_from(priority).with_context(|| {
        format!(
            "MCP tool `{}` image-inspection priority is outside the i32 range",
            tool.public_name
        )
    })?;
    let input = declaration.get("input").and_then(Value::as_object);
    let (image_pointer, image_encoding) =
        parse_mcp_image_input_binding(tool, input.and_then(|input| input.get("image")))?;
    let focus_pointer = match input.and_then(|input| input.get("focus")) {
        Some(Value::Null) => None,
        Some(value) => Some(parse_binding_pointer(tool, "focus", value, "/focus")?),
        None => Some("/focus".to_string()),
    };
    validate_binding_root_property(tool, &image_pointer, "image")?;
    if let Some(pointer) = focus_pointer.as_deref() {
        validate_binding_root_property(tool, pointer, "focus")?;
    }
    Ok(Some(McpImageInspectionBinding {
        priority,
        image_pointer,
        focus_pointer,
        image_encoding,
    }))
}

fn parse_mcp_image_input_binding(
    tool: &McpToolDescriptor,
    value: Option<&Value>,
) -> anyhow::Result<(String, McpImageInputEncoding)> {
    let pointer = parse_binding_pointer(tool, "image", value.unwrap_or(&Value::Null), "/image")?;
    let encoding = value
        .and_then(Value::as_object)
        .and_then(|value| value.get("encoding"))
        .and_then(Value::as_str)
        .unwrap_or("object_base64");
    let encoding = match encoding {
        "object_base64" => McpImageInputEncoding::ObjectBase64,
        "base64" => McpImageInputEncoding::Base64,
        "data_url" => McpImageInputEncoding::DataUrl,
        other => anyhow::bail!(
            "MCP tool `{}` declares unsupported image encoding `{other}`",
            tool.public_name
        ),
    };
    Ok((pointer, encoding))
}

fn parse_binding_pointer(
    tool: &McpToolDescriptor,
    field: &str,
    value: &Value,
    default: &str,
) -> anyhow::Result<String> {
    let pointer = value
        .as_str()
        .or_else(|| {
            value
                .as_object()
                .and_then(|value| value.get("pointer"))
                .and_then(Value::as_str)
        })
        .unwrap_or(default)
        .trim();
    anyhow::ensure!(
        pointer.starts_with('/') && pointer.len() > 1,
        "MCP tool `{}` declares invalid {field} JSON pointer `{pointer}`",
        tool.public_name
    );
    anyhow::ensure!(
        !pointer.split('/').skip(1).any(str::is_empty),
        "MCP tool `{}` declares empty {field} JSON pointer segments",
        tool.public_name
    );
    Ok(pointer.to_string())
}

fn validate_binding_root_property(
    tool: &McpToolDescriptor,
    pointer: &str,
    field: &str,
) -> anyhow::Result<()> {
    let Some(properties) = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
    else {
        return Ok(());
    };
    let root = decode_json_pointer_segment(
        pointer
            .split('/')
            .nth(1)
            .expect("validated pointer has a root segment"),
    )?;
    anyhow::ensure!(
        properties.contains_key(&root)
            || tool
                .input_schema
                .get("additionalProperties")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        "MCP tool `{}` maps {field} to `{pointer}`, but `{root}` is absent from its input schema",
        tool.public_name
    );
    Ok(())
}

fn decode_json_pointer_segment(segment: &str) -> anyhow::Result<String> {
    let mut decoded = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            _ => anyhow::bail!("invalid JSON pointer escape in `{segment}`"),
        }
    }
    Ok(decoded)
}

fn set_object_json_pointer(target: &mut Value, pointer: &str, value: Value) -> anyhow::Result<()> {
    let segments = pointer
        .split('/')
        .skip(1)
        .map(decode_json_pointer_segment)
        .collect::<anyhow::Result<Vec<_>>>()?;
    anyhow::ensure!(
        !segments.is_empty(),
        "JSON pointer must address an object field"
    );
    let mut current = target;
    for segment in &segments[..segments.len() - 1] {
        let object = current
            .as_object_mut()
            .context("attachment capability JSON pointers may only traverse objects")?;
        current = object.entry(segment.clone()).or_insert_with(|| json!({}));
    }
    let object = current
        .as_object_mut()
        .context("attachment capability JSON pointer parent must be an object")?;
    object.insert(
        segments
            .last()
            .expect("non-empty JSON pointer segments")
            .clone(),
        value,
    );
    Ok(())
}

pub(super) fn select_mcp_image_inspector(
    tools: &[McpToolDescriptor],
) -> anyhow::Result<(McpToolDescriptor, McpImageInspectionBinding)> {
    let mut candidates = Vec::new();
    let mut invalid = Vec::new();
    for tool in tools {
        match parse_mcp_image_inspection_binding(tool) {
            Ok(Some(binding)) => candidates.push((tool.clone(), binding)),
            Ok(None) => {}
            Err(error) => invalid.push(error.to_string()),
        }
    }
    candidates.sort_by(|(left_tool, left), (right_tool, right)| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left_tool.public_name.cmp(&right_tool.public_name))
    });
    let Some((tool, binding)) = candidates.first().cloned() else {
        if invalid.is_empty() {
            anyhow::bail!(
                "the selected model does not support native image input and no enabled MCP tool explicitly declares `{MCP_IMAGE_INSPECTION_CAPABILITY}`"
            );
        }
        anyhow::bail!(
            "no valid `{MCP_IMAGE_INSPECTION_CAPABILITY}` MCP binding is available: {}",
            invalid.join("; ")
        );
    };
    if candidates
        .get(1)
        .is_some_and(|(_, candidate)| candidate.priority == binding.priority)
    {
        let conflicts = candidates
            .iter()
            .take_while(|(_, candidate)| candidate.priority == binding.priority)
            .map(|(candidate, _)| candidate.public_name.as_str())
            .collect::<Vec<_>>();
        anyhow::bail!(
            "multiple MCP image inspectors have priority {}: {}; configure distinct priorities",
            binding.priority,
            conflicts.join(", ")
        );
    }
    Ok((tool, binding))
}

pub(super) fn mcp_image_inspection_arguments(
    binding: &McpImageInspectionBinding,
    focus: &str,
    name: &str,
    content_type: &str,
    data: &[u8],
) -> anyhow::Result<Value> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    let image = match binding.image_encoding {
        McpImageInputEncoding::ObjectBase64 => json!({
            "data": encoded,
            "mimeType": content_type,
            "name": name,
        }),
        McpImageInputEncoding::Base64 => json!(encoded),
        McpImageInputEncoding::DataUrl => {
            json!(format!("data:{content_type};base64,{encoded}"))
        }
    };
    let mut arguments = json!({});
    set_object_json_pointer(&mut arguments, &binding.image_pointer, image)?;
    if let Some(pointer) = binding.focus_pointer.as_deref() {
        set_object_json_pointer(&mut arguments, pointer, json!(focus))?;
    }
    Ok(arguments)
}

async fn inspect_attachment_through_mcp(
    call_id: Uuid,
    attachment: &StoredAttachment,
    content_type: &str,
    data: &[u8],
    focus: Option<&str>,
    ctx: &ToolInvocationContext,
) -> anyhow::Result<ToolResult> {
    let focus = focus
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_ATTACHMENT_INSPECTION_FOCUS);
    anyhow::ensure!(
        focus.chars().count() <= MAX_ATTACHMENT_INSPECTION_FOCUS_CHARS,
        "view_attachment focus exceeds {MAX_ATTACHMENT_INSPECTION_FOCUS_CHARS} characters"
    );
    let (descriptor, binding) = select_mcp_image_inspector(&ctx.mcp_tools)?;
    let permission = ToolPermissionDescriptor::from(&descriptor);
    enforce_policy_decision(ctx.policy.inspect_mcp_tool_call(&permission), ctx)?;
    let host = ctx
        .mcp_host
        .clone()
        .context("the configured MCP attachment-inspection host is unavailable")?;
    let arguments =
        mcp_image_inspection_arguments(&binding, focus, attachment.name(), content_type, data)?;
    let result = if let Some(route) = ctx.connection_operations.get(&descriptor.public_name) {
        // Indirect consumers cross the same live boundary as a direct model
        // tool call. An approved view_attachment replay cannot bypass a
        // disabled/changed Connection operation.
        route.authorize().await?;
        let operation = route.operation();
        host.call_server_tool(
            operation.mcp_server_id,
            &operation.provider_tool_name,
            &operation.pinned_operation_fingerprint,
            arguments,
        )
        .await?
    } else if !ctx.connection_operations.is_empty() {
        anyhow::bail!("attachment inspector is outside the frozen Connection operation authority");
    } else {
        host.call_tool(&descriptor.public_name, arguments).await?
    };
    let mut content = vec![ModelContentPart::text(ATTACHMENT_RESULT_BOUNDARY)];
    if !result.output.trim().is_empty() {
        content.push(ModelContentPart::text(result.output.clone()));
    }
    for part in mcp_content_parts(&result.content, result.structured_content.as_ref()) {
        match part {
            ModelContentPart::Image { .. } => content.push(ModelContentPart::text(
                "The external attachment inspector returned image data that this text-only model cannot inspect.",
            )),
            other => content.push(other),
        }
    }
    let output = format!(
        "{ATTACHMENT_RESULT_BOUNDARY}\nImage inspection for {} ({}) via configured capability provider {}:\n{}",
        attachment.name(),
        attachment.id(),
        descriptor.public_name,
        result.output,
    );
    Ok(ToolResult {
        call_id,
        output,
        content,
        metadata: json!({
            "success": !result.is_error,
            "isError": result.is_error,
            "provenance": "user_attachment_mcp_inspection",
            "route": "mcp_capability",
            "capability": MCP_IMAGE_INSPECTION_CAPABILITY,
            "attachmentId": attachment.id(),
            "name": attachment.name(),
            "contentType": content_type,
            "bytes": data.len(),
            "providerTool": descriptor.public_name,
            "serverId": descriptor.server_id,
        }),
    })
}

async fn attachment_image_bytes(
    attachment: &StoredAttachment,
) -> anyhow::Result<(String, Vec<u8>)> {
    match attachment {
        StoredAttachment::InlineImage {
            content_type, data, ..
        } => Ok((content_type.clone(), data.clone())),
        StoredAttachment::ContextSource {
            kind: ContextSourceKind::Image,
            ..
        } => {
            let source = load_stored_context_source(attachment).await?;
            source
                .content
                .into_iter()
                .find_map(|part| match part {
                    ModelContentPart::Image { content_type, data } => Some((content_type, data)),
                    _ => None,
                })
                .context("image attachment loader returned no image data")
        }
        StoredAttachment::ContextSource { .. } => {
            anyhow::bail!("{} is not an image", attachment.name())
        }
    }
}

#[cfg(test)]
mod connection_route_tests {
    use super::*;
    use crate::policy::BasicPolicyEngine;
    use crate::{
        mcp_operation_fingerprint, ConnectionOperationInvocationGate,
        ConnectionOperationRuntimeRoute, ExecutionConnectionOperationV1, PermissionMode,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct DenyingGate(Arc<AtomicUsize>);

    #[async_trait]
    impl ConnectionOperationInvocationGate for DenyingGate {
        async fn authorize(
            &self,
            _operation: &ExecutionConnectionOperationV1,
        ) -> anyhow::Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("Connection revoked")
        }
    }

    #[tokio::test]
    async fn approved_attachment_inspection_still_requires_the_live_connection_gate() {
        let server_id = Uuid::new_v4();
        let connection_id = Uuid::new_v4();
        let model_tool_name = "mcp_image_inspector_fixture".to_string();
        let native_descriptor = McpToolDescriptor {
            public_name: "vision__inspect".to_string(),
            server_id,
            tool_name: "inspect".to_string(),
            description: Some("Inspect an image".to_string()),
            input_schema: json!({ "type": "object" }),
            annotations: json!({ "destructiveHint": true }),
            meta: json!({
                "com.opentopia/capabilities": {
                    "media.image.inspect/v1": {
                        "priority": 1,
                        "input": {
                            "image": { "pointer": "/image", "encoding": "base64" },
                            "focus": "/focus"
                        }
                    }
                }
            }),
            permission_labels: vec!["destructive".to_string()],
        };
        let operation = ExecutionConnectionOperationV1 {
            connection_id,
            capability_revision: 1,
            operation_id: format!("connection:{connection_id}:tool:inspect"),
            mcp_server_id: server_id,
            provider_tool_name: "inspect".to_string(),
            model_tool_name: model_tool_name.clone(),
            pinned_operation_fingerprint: mcp_operation_fingerprint(&native_descriptor),
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let mut structured_descriptor = native_descriptor;
        structured_descriptor.public_name = model_tool_name.clone();
        let workspace = std::env::current_dir().expect("current directory");
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace.clone(),
            PermissionMode::Auto,
        ));
        let mut context = ToolInvocationContext::local(workspace, policy);
        context.approval_granted = true;
        context.mcp_host = Some(crate::mcp_host::McpExtensionHost::new());
        context.mcp_tools = vec![structured_descriptor];
        context.connection_operations.insert(
            model_tool_name,
            ConnectionOperationRuntimeRoute::new(operation, Arc::new(DenyingGate(calls.clone()))),
        );
        let attachment = StoredAttachment::InlineImage {
            id: Uuid::new_v4(),
            content_type: "image/png".to_string(),
            data: vec![1, 2, 3],
            name: "fixture.png".to_string(),
        };

        let err = inspect_attachment_through_mcp(
            Uuid::new_v4(),
            &attachment,
            "image/png",
            &[1, 2, 3],
            None,
            &context,
        )
        .await
        .expect_err("approved replay must not bypass Connection revocation");
        assert!(err.to_string().contains("Connection revoked"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
