use crate::McpToolDescriptor;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Produces the credential-free fingerprint reviewed by an Agent template.
/// Raw MCP `_meta` is intentionally excluded.
pub fn mcp_operation_fingerprint(descriptor: &McpToolDescriptor) -> String {
    connection_capability_operation_fingerprint(
        descriptor.server_id.to_string(),
        &descriptor.public_name,
        &descriptor.tool_name,
        descriptor.description.as_deref(),
        &descriptor.input_schema,
        &normalize_public_mcp_annotations(&descriptor.annotations),
        &normalize_permission_labels(&descriptor.permission_labels),
    )
}

pub(crate) fn connection_capability_operation_fingerprint(
    server_id: String,
    public_name: &str,
    tool_name: &str,
    description: Option<&str>,
    input_schema: &Value,
    annotations: &Value,
    permission_labels: &[String],
) -> String {
    let payload = canonicalize_connection_capability_json(&serde_json::json!({
        "serverId": server_id,
        "publicName": public_name,
        "toolName": tool_name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": annotations,
        "permissionLabels": permission_labels,
    }));
    let encoded = serde_json::to_vec(&payload).unwrap_or_default();
    let digest = Sha256::digest(encoded);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

/// Single normalization source for discovery, publication, and invocation.
pub(crate) fn normalize_public_mcp_annotations(value: &Value) -> Value {
    const STRING_KEYS: &[&str] = &["title"];
    const BOOL_KEYS: &[&str] = &[
        "readOnlyHint",
        "destructiveHint",
        "idempotentHint",
        "openWorldHint",
    ];
    let mut public = Map::new();
    for key in STRING_KEYS {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            public.insert((*key).to_string(), Value::String(text.to_string()));
        }
    }
    for key in BOOL_KEYS {
        if let Some(flag) = value.get(*key).and_then(Value::as_bool) {
            public.insert((*key).to_string(), Value::Bool(flag));
        }
    }
    Value::Object(public)
}

pub(crate) fn normalize_permission_labels(labels: &[String]) -> Vec<String> {
    let mut normalized = labels
        .iter()
        .map(|label| label.trim().to_ascii_lowercase())
        .filter(|label| !label.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

pub(crate) fn canonicalize_connection_capability_json(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(canonicalize_connection_capability_json)
                .collect(),
        ),
        Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut canonical = Map::new();
            for (key, value) in entries {
                canonical.insert(key.clone(), canonicalize_connection_capability_json(value));
            }
            Value::Object(canonical)
        }
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    fn descriptor(server_id: Uuid) -> McpToolDescriptor {
        McpToolDescriptor {
            public_name: "crm__customer_get".to_string(),
            server_id,
            tool_name: "customer_get".to_string(),
            description: Some("Read one customer".to_string()),
            input_schema: json!({"properties": {"id": {"type": "string"}}, "type": "object"}),
            annotations: json!({
                "readOnlyHint": true,
                "privateProviderHint": "excluded",
            }),
            meta: json!({"secret": "excluded"}),
            permission_labels: vec![" Read ".to_string(), "read".to_string()],
        }
    }

    #[test]
    fn provider_private_metadata_is_outside_the_fingerprint() {
        let server_id = Uuid::new_v4();
        let first = descriptor(server_id);
        let mut second = first.clone();
        second.meta = json!({"token": "changed"});
        second.annotations["privateProviderHint"] = json!("changed");
        assert_eq!(
            mcp_operation_fingerprint(&first),
            mcp_operation_fingerprint(&second)
        );
        second.input_schema["required"] = json!(["id"]);
        assert_ne!(
            mcp_operation_fingerprint(&first),
            mcp_operation_fingerprint(&second)
        );
    }
}
