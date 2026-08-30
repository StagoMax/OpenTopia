use crate::model::ToolResult;
use serde_json::Value;
use uuid::Uuid;

pub const CONVERSATION_TOOL_DETAIL_METADATA_KEY: &str = "_opentopiaConversationDetail";
const CONVERSATION_TOOL_OUTPUT_PREVIEW_BYTES: usize = 2_048;
const CONVERSATION_TOOL_METADATA_BUDGET_BYTES: usize = 16_384;

impl ToolResult {
    /// Builds the lightweight result carried by the conversation event view.
    /// The canonical event remains untouched in `events`, and the event id in
    /// the reserved metadata entry lets presentation clients request it only
    /// when a user expands the tool card.
    pub fn conversation_summary(&self, event_id: Uuid) -> Self {
        if self
            .metadata
            .get(CONVERSATION_TOOL_DETAIL_METADATA_KEY)
            .is_some()
        {
            let mut projected = self.clone();
            projected.content.clear();
            return projected;
        }

        let original_output_bytes = self.output.len();
        let original_metadata_bytes = serde_json::to_vec(&self.metadata)
            .map(|value| value.len())
            .unwrap_or_default();
        let (output, output_truncated) =
            conversation_output_preview(&self.output, CONVERSATION_TOOL_OUTPUT_PREVIEW_BYTES);
        let mut metadata = conversation_metadata_summary(&self.metadata);
        let detail = serde_json::json!({
            "eventId": event_id,
            "outputTruncated": output_truncated,
            "originalOutputBytes": original_output_bytes,
            "originalMetadataBytes": original_metadata_bytes,
        });
        metadata
            .as_object_mut()
            .expect("conversation metadata summary is always an object")
            .insert(CONVERSATION_TOOL_DETAIL_METADATA_KEY.to_string(), detail);

        Self {
            call_id: self.call_id,
            output,
            content: Vec::new(),
            metadata,
        }
    }
}

fn conversation_output_preview(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}\n…", &value[..end]), true)
}

fn conversation_metadata_summary(metadata: &Value) -> Value {
    let Some(source) = metadata.as_object() else {
        return serde_json::json!({});
    };
    let mut compact = serde_json::Map::new();
    for (key, value) in source {
        if conversation_metadata_body_key(key) {
            continue;
        }
        compact.insert(key.clone(), compact_conversation_json(value, 0));
    }
    let compact = Value::Object(compact);
    if serde_json::to_vec(&compact)
        .map(|value| value.len() <= CONVERSATION_TOOL_METADATA_BUDGET_BYTES)
        .unwrap_or(false)
    {
        return compact;
    }

    const SUMMARY_KEYS: &[&str] = &[
        "action",
        "bytes",
        "changedPath",
        "command",
        "contentType",
        "count",
        "durationMs",
        "error",
        "errorChain",
        "errorRecord",
        "exitCode",
        "format",
        "maxResults",
        "name",
        "operation",
        "originalBytes",
        "path",
        "query",
        "repository",
        "returnedMatches",
        "sandbox",
        "status",
        "success",
        "toolError",
        "toolName",
        "truncated",
        "workdir",
    ];
    let mut fallback = serde_json::Map::new();
    for key in SUMMARY_KEYS {
        if let Some(value) = source.get(*key) {
            fallback.insert((*key).to_string(), compact_conversation_json(value, 0));
        }
    }
    Value::Object(fallback)
}

fn compact_conversation_json(value: &Value, depth: usize) -> Value {
    const MAX_STRING_BYTES: usize = 1_024;
    const MAX_ARRAY_ITEMS: usize = 16;
    const MAX_OBJECT_FIELDS: usize = 48;
    const MAX_DEPTH: usize = 4;

    if depth >= MAX_DEPTH {
        return match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
            Value::String(text) => {
                Value::String(conversation_output_preview(text, MAX_STRING_BYTES).0)
            }
            Value::Array(items) => serde_json::json!({ "itemCount": items.len() }),
            Value::Object(fields) => serde_json::json!({ "fieldCount": fields.len() }),
        };
    }
    match value {
        Value::String(text) => Value::String(conversation_output_preview(text, MAX_STRING_BYTES).0),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .take(MAX_ARRAY_ITEMS)
                .map(|item| compact_conversation_json(item, depth + 1))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| !conversation_metadata_body_key(key))
                .take(MAX_OBJECT_FIELDS)
                .map(|(key, value)| (key.clone(), compact_conversation_json(value, depth + 1)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn conversation_metadata_body_key(key: &str) -> bool {
    matches!(
        key,
        "base64"
            | "body"
            | "content"
            | "data"
            | "image"
            | "output"
            | "response"
            | "stderr"
            | "stdout"
            | "toolResultEnvelope"
            | "tool_result_envelope"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conversation_tool_summary_keeps_detail_reference_and_utf8_boundary() {
        let event_id = Uuid::new_v4();
        let result = ToolResult::text(
            Uuid::new_v4(),
            "工具输出".repeat(1_000),
            json!({
                "command": "cargo test",
                "exitCode": 0,
                "stdout": "large stream".repeat(2_000),
            }),
        );

        let summary = result.conversation_summary(event_id);
        assert!(summary.output.len() < result.output.len());
        assert!(summary.output.is_char_boundary(summary.output.len()));
        assert!(summary.content.is_empty());
        assert!(summary.metadata.get("stdout").is_none());
        assert_eq!(summary.metadata["command"], json!("cargo test"));
        assert_eq!(
            summary.metadata[CONVERSATION_TOOL_DETAIL_METADATA_KEY]["eventId"],
            json!(event_id)
        );
        assert_eq!(
            summary.metadata[CONVERSATION_TOOL_DETAIL_METADATA_KEY]["outputTruncated"],
            json!(true)
        );
    }

    #[test]
    fn conversation_tool_summary_is_idempotent() {
        let event_id = Uuid::new_v4();
        let result = ToolResult::text(Uuid::new_v4(), "result", json!({ "count": 1 }));
        let once = result.conversation_summary(event_id);
        let twice = once.conversation_summary(event_id);

        assert_eq!(
            serde_json::to_value(once).expect("serialize first summary"),
            serde_json::to_value(twice).expect("serialize second summary")
        );
    }
}
