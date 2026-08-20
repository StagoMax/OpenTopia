use super::{
    enforce_policy_decision, Tool, ToolExecutionPolicy, ToolInvocationContext, ToolSideEffect,
};
use crate::mcp::{McpCallResult, McpToolDescriptor};
use crate::mcp_host::McpExtensionHost;
use crate::model::{ModelContentPart, ToolCall, ToolResult};
use crate::policy::{PolicyDecision, ToolPermissionDescriptor};
use crate::{
    mcp_operation_fingerprint, ConnectionOperationInvocationGate, ConnectionOperationRuntimeRoute,
    ExecutionConnectionOperationV1,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

enum McpInvocationRoute {
    LegacyPublicName,
    GrantedConnection(ConnectionOperationRuntimeRoute),
}

pub struct McpToolWrapper {
    host: McpExtensionHost,
    descriptor: McpToolDescriptor,
    route: McpInvocationRoute,
}

impl McpToolWrapper {
    pub fn new(host: McpExtensionHost, descriptor: McpToolDescriptor) -> Self {
        Self {
            host,
            descriptor,
            route: McpInvocationRoute::LegacyPublicName,
        }
    }

    /// Builds the model-visible wrapper for one immutable Connection grant.
    ///
    /// The descriptor is validated before its legacy public name is replaced
    /// with the connection-scoped model alias. Provider routing always remains
    /// fixed to `(server_id, provider_tool_name)` from the frozen operation.
    pub fn new_granted(
        host: McpExtensionHost,
        operation: ExecutionConnectionOperationV1,
        mut descriptor: McpToolDescriptor,
        gate: Arc<dyn ConnectionOperationInvocationGate>,
    ) -> anyhow::Result<Self> {
        if descriptor.server_id != operation.mcp_server_id
            || descriptor.tool_name != operation.provider_tool_name
        {
            anyhow::bail!(
                "live MCP descriptor does not match frozen Connection route {}:{}",
                operation.mcp_server_id,
                operation.provider_tool_name
            );
        }
        if mcp_operation_fingerprint(&descriptor) != operation.pinned_operation_fingerprint {
            anyhow::bail!(
                "MCP operation {} changed after its Connection grant was reviewed",
                operation.operation_id
            );
        }
        descriptor.public_name = operation.model_tool_name.clone();
        Ok(Self {
            host,
            descriptor,
            route: McpInvocationRoute::GrantedConnection(ConnectionOperationRuntimeRoute::new(
                operation, gate,
            )),
        })
    }

    pub fn descriptor(&self) -> &McpToolDescriptor {
        &self.descriptor
    }

    pub(crate) fn granted_route(&self) -> Option<ConnectionOperationRuntimeRoute> {
        match &self.route {
            McpInvocationRoute::GrantedConnection(route) => Some(route.clone()),
            McpInvocationRoute::LegacyPublicName => None,
        }
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
        enforce_policy_decision(ctx.policy.inspect_mcp_tool_call(&permission), &ctx)?;

        let result: McpCallResult = match &self.route {
            McpInvocationRoute::LegacyPublicName => {
                self.host
                    .call_tool(&self.descriptor.public_name, call.input)
                    .await?
            }
            McpInvocationRoute::GrantedConnection(route) => {
                // User approval can replay a policy `Ask`, but it never
                // supersedes live Connection state or the frozen route.
                route.authorize().await?;
                let operation = route.operation();
                self.host
                    .call_server_tool(
                        operation.mcp_server_id,
                        &operation.provider_tool_name,
                        &operation.pinned_operation_fingerprint,
                        call.input,
                    )
                    .await?
            }
        };
        let content = mcp_content_parts(&result.content, result.structured_content.as_ref());

        let mut metadata = json!({
            "isError": result.is_error,
            "publicName": result.public_name,
            "toolName": result.tool_name,
            "serverId": result.server_id,
            "raw": result.raw,
        });
        if let McpInvocationRoute::GrantedConnection(route) = &self.route {
            let operation = route.operation();
            metadata["connectionId"] = json!(operation.connection_id);
            metadata["operationId"] = json!(operation.operation_id);
            metadata["modelToolName"] = json!(operation.model_tool_name);
        }

        Ok(ToolResult {
            call_id: call.id,
            output: result.output,
            content,
            metadata,
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

#[cfg(test)]
mod granted_route_tests {
    use super::*;
    use crate::policy::BasicPolicyEngine;
    use crate::tools::ToolInvocationContext;
    use crate::PermissionMode;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    struct DenyingGate {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ConnectionOperationInvocationGate for DenyingGate {
        async fn authorize(
            &self,
            _operation: &ExecutionConnectionOperationV1,
        ) -> anyhow::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("connection disabled")
        }
    }

    #[tokio::test]
    async fn user_approval_never_bypasses_the_live_connection_gate() {
        let server_id = Uuid::new_v4();
        let connection_id = Uuid::new_v4();
        let descriptor = McpToolDescriptor {
            public_name: "crm__delete_customer".to_string(),
            server_id,
            tool_name: "delete_customer".to_string(),
            description: Some("Delete a customer".to_string()),
            input_schema: json!({ "type": "object" }),
            annotations: json!({ "destructiveHint": true }),
            meta: json!({}),
            permission_labels: vec!["destructive".to_string()],
        };
        let operation = ExecutionConnectionOperationV1 {
            connection_id,
            capability_revision: 7,
            operation_id: "connection:fixture:delete_customer".to_string(),
            mcp_server_id: server_id,
            provider_tool_name: "delete_customer".to_string(),
            model_tool_name: "mcp_delete_customer_fixture".to_string(),
            pinned_operation_fingerprint: mcp_operation_fingerprint(&descriptor),
        };
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let wrapper = McpToolWrapper::new_granted(
            McpExtensionHost::new(),
            operation,
            descriptor,
            Arc::new(DenyingGate {
                calls: gate_calls.clone(),
            }),
        )
        .expect("unchanged descriptor should register");
        let workspace = std::env::current_dir().expect("current directory");
        let policy = Arc::new(BasicPolicyEngine::new(
            workspace.clone(),
            PermissionMode::Auto,
        ));
        let mut context = ToolInvocationContext::local(workspace, policy);
        context.approval_granted = true;

        let err = wrapper
            .execute(
                ToolCall::new(wrapper.name(), json!({ "id": "customer-1" })),
                context,
            )
            .await
            .expect_err("live gate denial must win after an approved policy replay");
        assert!(err.to_string().contains("connection disabled"));
        assert_eq!(gate_calls.load(Ordering::SeqCst), 1);
    }
}
