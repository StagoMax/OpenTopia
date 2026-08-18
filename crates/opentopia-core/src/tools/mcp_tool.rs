use super::{
    enforce_policy_decision, Tool, ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect,
};
use crate::mcp::{McpCallResult, McpToolDescriptor};
use crate::mcp_host::McpExtensionHost;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::{PolicyDecision, ToolPermissionDescriptor};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct McpToolWrapper {
    host: McpExtensionHost,
    descriptor: McpToolDescriptor,
}

impl McpToolWrapper {
    pub fn new(host: McpExtensionHost, descriptor: McpToolDescriptor) -> Self {
        Self { host, descriptor }
    }

    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }

    fn annotation(&self, key: &str) -> bool {
        self.descriptor
            .annotations
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    fn has_permission_label(&self, candidates: &[&str]) -> bool {
        self.descriptor.permission_labels.iter().any(|label| {
            candidates
                .iter()
                .any(|candidate| label.eq_ignore_ascii_case(candidate))
        })
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.descriptor.public_name
    }

    fn description(&self) -> &str {
        self.descriptor.description.as_deref().unwrap_or_default()
    }

    fn schema(&self) -> Value {
        self.descriptor.input_schema.clone()
    }

    fn execution_policy(&self, _call: &ToolCall) -> ToolExecutionPolicy {
        let declares_read_only = self.annotation("readOnlyHint")
            || self.has_permission_label(&["read", "readonly", "read_only"]);
        let declares_write = self.has_permission_label(&["write", "modify", "mutation"]);
        let declares_destructive = self.annotation("destructiveHint")
            || self.has_permission_label(&["destructive", "delete", "dangerous"]);
        let read_only = declares_read_only && !declares_write && !declares_destructive;

        ToolExecutionPolicy {
            read_only,
            idempotent: read_only || self.annotation("idempotentHint"),
            // MCP uses request ids and the host keeps independent pending responses, so calls do
            // not need to be serialized merely because they share a transport or server.
            parallel_safe: true,
            side_effect: if read_only {
                ToolSideEffect::None
            } else {
                ToolSideEffect::External
            },
            // Read-only calls carry no exclusive resource claim. Mutating/unknown calls from the
            // same server stay ordered because MCP annotations do not identify the external
            // resource they affect; calls to different servers may still run concurrently.
            resource_keys: if read_only {
                Vec::new()
            } else {
                vec![format!("mcp:server:{}", self.descriptor.server_id)]
            },
        }
    }

    fn authorization_preflight(
        &self,
        _call: &ToolCall,
        ctx: &ToolInvocationContext,
    ) -> Option<PolicyDecision> {
        let permission = ToolPermissionDescriptor::from(&self.descriptor);
        Some(ctx.policy.inspect_mcp_tool_call(&permission))
    }

    async fn execute(
        &self,
        call: ToolCall,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult> {
        let permission = ToolPermissionDescriptor::from(&self.descriptor);
        enforce_policy_decision(
            ctx.policy.inspect_mcp_tool_call(&permission),
            ctx.approval_granted,
        )?;

        let result: McpCallResult = self
            .host
            .call_tool(&self.descriptor.public_name, call.input)
            .await?;
        let content = mcp_content_parts(&result.content, result.structured_content.as_ref());

        Ok(ToolResult {
            call_id: call.id,
            output: result.output,
            content,
            metadata: json!({
                "isError": result.is_error,
                "publicName": result.public_name,
                "toolName": result.tool_name,
                "serverId": result.server_id,
                "raw": result.raw,
            }),
        })
    }
}

pub(super) fn mcp_content_parts(
    content: &[Value],
    structured_content: Option<&Value>,
) -> Vec<ModelContentPart> {
    let mut parts = Vec::new();
    for item in content {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(ModelContentPart::text(text));
                } else {
                    parts.push(ModelContentPart::json(item.clone()));
                }
            }
            Some("image") => {
                let content_type = item
                    .get("mimeType")
                    .or_else(|| item.get("mime_type"))
                    .and_then(Value::as_str);
                let data = item.get("data").and_then(Value::as_str);
                match (content_type, data.and_then(decode_mcp_base64)) {
                    (Some(content_type), Some(data)) => {
                        parts.push(ModelContentPart::image(content_type, data));
                    }
                    _ => parts.push(ModelContentPart::json(item.clone())),
                }
            }
            Some("resource") => {
                let resource = item.get("resource").unwrap_or(item);
                let uri = resource.get("uri").and_then(Value::as_str);
                if let Some(uri) = uri {
                    parts.push(ModelContentPart::resource(
                        uri,
                        resource
                            .get("mimeType")
                            .or_else(|| resource.get("mime_type"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        resource
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ));
                    if let Some(text) = resource.get("text").and_then(Value::as_str) {
                        parts.push(ModelContentPart::text(text));
                    }
                } else {
                    parts.push(ModelContentPart::json(item.clone()));
                }
            }
            _ => parts.push(ModelContentPart::json(item.clone())),
        }
    }
    if let Some(value) = structured_content {
        parts.push(ModelContentPart::json(value.clone()));
    }
    parts
}

pub(super) fn decode_mcp_base64(value: &str) -> Option<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = value
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks_exact(4) {
        let first = sextet(chunk[0])?;
        let second = sextet(chunk[1])?;
        let third = if chunk[2] == b'=' {
            None
        } else {
            Some(sextet(chunk[2])?)
        };
        let fourth = if chunk[3] == b'=' {
            None
        } else {
            Some(sextet(chunk[3])?)
        };
        if third.is_none() && fourth.is_some() {
            return None;
        }
        decoded.push(first << 2 | second >> 4);
        if let Some(third) = third {
            decoded.push((second & 0b0000_1111) << 4 | third >> 2);
            if let Some(fourth) = fourth {
                decoded.push((third & 0b0000_0011) << 6 | fourth);
            }
        }
    }
    Some(decoded)
}
