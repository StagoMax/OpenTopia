use crate::model::{ModelContentPart, ToolResult};
use crate::model_context::content_fingerprint;
use crate::tool_output_truncation::{
    ensure_output_artifact, formatted_truncate_text, output_artifact_id_from_metadata,
    replace_matching_text_content, token_budget_bytes, truncate_middle_bytes,
    with_artifact_reference, HISTORY_TOOL_OUTPUT_MAX_TOKENS,
};
use crate::tool_state::ToolStateStore;
use serde_json::{json, Value};
use uuid::Uuid;

const ENVELOPE_KEY: &str = "toolResultEnvelope";
const ENVELOPE_SCHEMA_VERSION: u64 = 1;
const PROVIDER_METADATA_MAX_BYTES: usize = 4_000;
const PROVIDER_RESULT_MAX_NON_MEDIA_BYTES: usize =
    token_budget_bytes(HISTORY_TOOL_OUTPUT_MAX_TOKENS) + PROVIDER_METADATA_MAX_BYTES;

/// Applies the tool-agnostic history safety budget exactly once, before the
/// first model round can observe the result. Tool-specific producer adapters
/// normally keep their output below this 1.2x serialization budget already.
pub(crate) fn normalize_tool_result_at_ingress(
    tool_name: &str,
    mut result: ToolResult,
    store: Option<&ToolStateStore>,
    thread_id: Option<Uuid>,
) -> ToolResult {
    if result.metadata.get(ENVELOPE_KEY).is_some() {
        return result;
    }

    let raw_output = result.output.clone();
    let max_bytes = token_budget_bytes(HISTORY_TOOL_OUTPUT_MAX_TOKENS);
    let output_json = serde_json::from_str::<Value>(&raw_output).ok();
    let structured_bytes = result
        .content
        .iter()
        .filter_map(|part| match part {
            ModelContentPart::Json { value }
                if raw_output.len() <= max_bytes && output_json.as_ref() == Some(value) =>
            {
                None
            }
            ModelContentPart::Json { value } => {
                serde_json::to_vec(value).ok().map(|value| value.len())
            }
            _ => None,
        })
        .fold(0usize, usize::saturating_add);
    let structured_budget = structured_bytes.min(max_bytes / 2);
    let output_budget = max_bytes.saturating_sub(structured_budget);
    let output_tokens = output_budget
        .saturating_sub(512)
        .checked_div(4)
        .unwrap_or_default()
        .max(1);
    let candidate = formatted_truncate_text(&raw_output, output_tokens);
    // Interactive control payloads are protocol state rather than tool prose.
    // Their metadata must remain intact even when it exceeds the replay budget.
    let enforce_provider_budget = tool_name != "request_user_input";
    let oversized_content = has_oversized_non_media_content(&result.content, max_bytes)
        || (enforce_provider_budget
            && provider_visible_tool_result_bytes(&result) > PROVIDER_RESULT_MAX_NON_MEDIA_BYTES);
    let source_artifact_id = output_artifact_id_from_metadata(&result.metadata);

    if candidate == raw_output && !oversized_content {
        let storage = if source_artifact_id.is_some() {
            "artifact_backed"
        } else {
            "inline"
        };
        insert_envelope_metadata(
            &mut result.metadata,
            "history_safety_passthrough",
            storage,
            raw_output.len(),
            raw_output.len(),
            source_artifact_id,
            &raw_output,
        );
        debug_assert!(
            !enforce_provider_budget
                || provider_visible_tool_result_bytes(&result)
                    <= PROVIDER_RESULT_MAX_NON_MEDIA_BYTES
        );
        return result;
    }

    let Some(artifact_id) = ensure_output_artifact(
        tool_name,
        &mut result,
        &raw_output,
        store,
        thread_id,
        "tool_result_history_ingress",
    ) else {
        // Saving tokens must never destroy the only copy of a tool result.
        insert_envelope_metadata(
            &mut result.metadata,
            "history_safety_middle",
            "inline_artifact_unavailable",
            raw_output.len(),
            raw_output.len(),
            None,
            &raw_output,
        );
        return result;
    };

    let normalized_output = with_artifact_reference(&candidate, artifact_id);
    replace_matching_text_content(&mut result.content, &raw_output, &normalized_output);
    bound_text_content(&mut result.content, max_bytes);
    bound_structured_content(
        tool_name,
        &mut result.content,
        structured_budget,
        artifact_id,
    );
    result.output = normalized_output;
    let output_bytes = result.output.len();
    insert_envelope_metadata(
        &mut result.metadata,
        "history_safety_middle",
        "artifact_backed",
        raw_output.len(),
        output_bytes,
        Some(artifact_id),
        &raw_output,
    );
    debug_assert!(
        !enforce_provider_budget
            || provider_visible_tool_result_bytes(&result) <= PROVIDER_RESULT_MAX_NON_MEDIA_BYTES
    );
    result
}

/// Keep the stored/UI representation untouched, but minify JSON before it
/// reaches the model so pretty-printing whitespace is not paid on every replay.
pub(crate) fn provider_tool_result_output(result: &ToolResult) -> String {
    serde_json::from_str::<Value>(&result.output)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| result.output.clone())
}

/// Provider adapters serialize `output`, `content`, and `metadata` together.
/// Avoid sending typed copies that carry the same value as the legacy output.
pub(crate) fn provider_tool_result_content(result: &ToolResult) -> Vec<ModelContentPart> {
    let artifact_backed = result
        .metadata
        .pointer(&format!("/{ENVELOPE_KEY}/storage"))
        .and_then(Value::as_str)
        == Some("artifact_backed");
    let output_json = serde_json::from_str::<Value>(&result.output).ok();
    let tool_name = result.metadata.get("toolName").and_then(Value::as_str);
    result
        .content
        .iter()
        .filter(|part| match part {
            ModelContentPart::Text { text } => {
                !(text.is_empty() || artifact_backed || result.output.contains(text))
            }
            ModelContentPart::Json { value } => {
                output_json.as_ref() != Some(value) && tool_name != Some("update_plan")
            }
            ModelContentPart::Image { .. } | ModelContentPart::Resource { .. } => true,
        })
        .cloned()
        .collect()
}

/// Builds provider-facing metadata without mutating the stored/UI ToolResult.
/// The returned value is created once with the ProviderToolResult and then
/// replayed byte-for-byte.
pub(crate) fn provider_tool_result_metadata(tool_name: &str, metadata: &Value) -> Value {
    let mut metadata = metadata.clone();
    let Some(object) = metadata.as_object_mut() else {
        return metadata;
    };
    object.remove("toolName");
    object.remove("providerToolCallId");
    object.remove("success");
    match tool_name {
        "shell" => {
            object.remove("stdout");
            object.remove("stderr");
        }
        "search" => {
            object.remove("locations");
        }
        "filesystem" => compact_filesystem_metadata(object),
        "update_plan" => {
            object.remove("workForm");
            object.remove("formId");
            object.remove("completed");
            object.remove("resolved");
            object.remove("total");
            object.remove("revision");
            object.remove("status");
            object.remove("nextRunnableItem");
            object.remove("currentItemIndex");
        }
        "list_skills" | "list_agents" => {
            object.remove("count");
        }
        "word_document" | "pdf" => {
            object.remove("action");
        }
        name if name.starts_with("spreadsheet_") => {
            object.remove("action");
        }
        _ => {}
    }
    if object.get("toolSource").and_then(Value::as_str) == Some("mcp")
        || (object.contains_key("serverId") && object.contains_key("publicName"))
    {
        object.remove("raw");
    }
    bound_metadata_error_strings(object);
    if tool_name != "request_user_input"
        && serde_json::to_vec(&metadata)
            .map(|encoded| encoded.len() > PROVIDER_METADATA_MAX_BYTES)
            .unwrap_or(false)
    {
        metadata = compact_provider_metadata(&metadata);
    }
    metadata
}

/// Estimated non-media bytes that provider adapters will serialize for this result.
/// Image bytes are deliberately excluded because they have provider-specific token
/// accounting and are governed by the multimodal input policy instead.
pub(crate) fn provider_visible_tool_result_bytes(result: &ToolResult) -> usize {
    let content = provider_tool_result_content(result);
    provider_tool_result_output(result).len()
        + serde_json::to_vec(&provider_tool_result_metadata(
            result
                .metadata
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            &result.metadata,
        ))
        .map(|encoded| encoded.len())
        .unwrap_or_default()
        + content
            .iter()
            .map(|part| match part {
                ModelContentPart::Image { .. } => 0,
                _ => serde_json::to_vec(part)
                    .map(|encoded| encoded.len())
                    .unwrap_or_default(),
            })
            .sum::<usize>()
}

fn compact_filesystem_metadata(metadata: &mut serde_json::Map<String, Value>) {
    let operation = metadata
        .get("operation")
        .and_then(Value::as_str)
        .map(str::to_string);
    metadata.remove("operation");
    match operation.as_deref() {
        Some("list" | "find" | "stat") => {
            for key in [
                "path",
                "count",
                "visitedEntries",
                "truncated",
                "truncationReason",
                "result",
            ] {
                metadata.remove(key);
            }
        }
        Some("write" | "copy" | "move" | "delete") => {
            metadata.remove("changedPath");
            metadata.remove("source");
            metadata.remove("destination");
            metadata.remove("bytes");
        }
        Some("read") | None | Some(_) => {}
    }
}

fn has_oversized_non_media_content(content: &[ModelContentPart], max_bytes: usize) -> bool {
    let mut total = 0usize;
    for part in content {
        let bytes = match part {
            ModelContentPart::Text { text } => text.len(),
            ModelContentPart::Json { value } => serde_json::to_vec(value)
                .map(|encoded| encoded.len())
                .unwrap_or(usize::MAX),
            ModelContentPart::Image { .. } | ModelContentPart::Resource { .. } => 0,
        };
        total = total.saturating_add(bytes);
        if total > max_bytes {
            return true;
        }
    }
    false
}

fn bound_text_content(content: &mut Vec<ModelContentPart>, max_bytes: usize) {
    let mut remaining = max_bytes;
    let mut omitted = 0usize;
    let mut bounded = Vec::with_capacity(content.len());
    for part in std::mem::take(content) {
        match part {
            ModelContentPart::Text { .. } if remaining == 0 => omitted += 1,
            ModelContentPart::Text { text } => {
                if text.len() <= remaining {
                    remaining = remaining.saturating_sub(text.len());
                    bounded.push(ModelContentPart::text(text));
                } else {
                    let snippet = truncate_middle_bytes(&text, remaining);
                    if snippet.is_empty() {
                        omitted += 1;
                    } else {
                        bounded.push(ModelContentPart::text(snippet));
                    }
                    remaining = 0;
                }
            }
            media_or_json => bounded.push(media_or_json),
        }
    }
    if omitted > 0 {
        bounded.push(ModelContentPart::text(format!(
            "[omitted {omitted} text items ...]"
        )));
    }
    *content = bounded;
}

fn bound_structured_content(
    tool_name: &str,
    content: &mut [ModelContentPart],
    max_bytes: usize,
    artifact_id: Uuid,
) {
    let mut remaining = max_bytes;
    for part in content {
        let ModelContentPart::Json { value } = part else {
            continue;
        };
        let encoded_bytes = serde_json::to_vec(value)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        if encoded_bytes <= remaining {
            remaining = remaining.saturating_sub(encoded_bytes);
            continue;
        }

        let compacted = if remaining >= 1_000 && tool_name == "spreadsheet_filter_rows" {
            compact_spreadsheet_json(value, remaining, artifact_id)
        } else {
            compact_generic_json(value, artifact_id)
        };
        let compacted_bytes = serde_json::to_vec(&compacted)
            .map(|encoded| encoded.len())
            .unwrap_or(usize::MAX);
        if compacted_bytes <= remaining {
            *value = compacted;
            remaining = remaining.saturating_sub(compacted_bytes);
        } else {
            *value = Value::Null;
            remaining = remaining.saturating_sub(4);
        }
    }
}

fn compact_spreadsheet_json(value: &Value, max_bytes: usize, artifact_id: Uuid) -> Value {
    let Some(root) = value.as_object() else {
        return compact_generic_json(value, artifact_id);
    };
    let Some(result) = root.get("result").and_then(Value::as_object) else {
        return compact_generic_json(value, artifact_id);
    };
    let Some(rows) = result.get("rows").and_then(Value::as_array) else {
        let Some(ranges) = result.get("ranges").and_then(Value::as_array) else {
            return compact_generic_json(value, artifact_id);
        };
        let per_range_budget = max_bytes
            .saturating_sub(1_000)
            .checked_div(ranges.len().max(1))
            .unwrap_or(max_bytes)
            .max(1_000);
        let mut projected_ranges = ranges
            .iter()
            .map(|range| {
                compact_spreadsheet_json(
                    &json!({ "type": "range_read", "result": range }),
                    per_range_budget,
                    artifact_id,
                )
                .get("result")
                .cloned()
                .unwrap_or_else(|| compact_generic_json(range, artifact_id))
            })
            .collect::<Vec<_>>();
        let mut compacted = value.clone();
        loop {
            let Some(compacted_result) = compacted.get_mut("result").and_then(Value::as_object_mut)
            else {
                return compact_generic_json(value, artifact_id);
            };
            compacted_result.insert("ranges".to_string(), Value::Array(projected_ranges.clone()));
            compacted_result.insert("totalRangeCount".to_string(), json!(ranges.len()));
            compacted_result.insert("artifactId".to_string(), json!(artifact_id));
            compacted_result.insert(
                "hasMoreRanges".to_string(),
                json!(projected_ranges.len() < ranges.len()),
            );
            if serde_json::to_vec(&compacted)
                .map(|encoded| encoded.len() <= max_bytes)
                .unwrap_or(false)
                || projected_ranges.is_empty()
            {
                return compacted;
            }
            projected_ranges.pop();
        }
    };

    let mut compacted = value.clone();
    let total_row_count = result
        .get("matchedRowCount")
        .and_then(Value::as_u64)
        .unwrap_or(rows.len() as u64);
    let matched_row_indices = result.get("matchedRowIndices").and_then(Value::as_array);
    {
        let Some(compacted_result) = compacted.get_mut("result").and_then(Value::as_object_mut)
        else {
            return compact_generic_json(value, artifact_id);
        };
        compacted_result.insert("rows".to_string(), Value::Array(Vec::new()));
        if matched_row_indices.is_some() {
            compacted_result.insert("matchedRowIndices".to_string(), Value::Array(Vec::new()));
        }
        compacted_result.insert("totalRowCount".to_string(), json!(total_row_count));
        compacted_result.insert("artifactId".to_string(), json!(artifact_id));
    }

    let mut kept = Vec::new();
    for row in rows {
        kept.push(row.clone());
        if let Some(compacted_result) = compacted.get_mut("result").and_then(Value::as_object_mut) {
            compacted_result.insert("rows".to_string(), Value::Array(kept.clone()));
        }
        if serde_json::to_vec(&compacted)
            .map(|encoded| encoded.len() > max_bytes)
            .unwrap_or(true)
        {
            kept.pop();
            break;
        }
    }
    let start_row = result
        .get("range")
        .and_then(Value::as_object)
        .and_then(|range| range.get("start"))
        .and_then(Value::as_object)
        .and_then(|start| start.get("row"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    loop {
        let returned = kept.len();
        let Some(compacted_result) = compacted.get_mut("result").and_then(Value::as_object_mut)
        else {
            return compact_generic_json(value, artifact_id);
        };
        compacted_result.insert("rows".to_string(), Value::Array(kept.clone()));
        if let Some(indices) = matched_row_indices {
            compacted_result.insert(
                "matchedRowIndices".to_string(),
                Value::Array(indices.iter().take(returned).cloned().collect()),
            );
        }
        compacted_result.insert("returnedRowCount".to_string(), json!(returned));
        compacted_result.insert(
            "hasMore".to_string(),
            json!((returned as u64) < total_row_count),
        );
        if (returned as u64) < total_row_count {
            compacted_result.insert("nextRow".to_string(), json!(start_row + returned as u64));
            compacted_result.insert(
                "continuation".to_string(),
                json!(
                    "Use read_artifact to retrieve a bounded window of the full filtered result."
                ),
            );
        }
        if serde_json::to_vec(&compacted)
            .map(|encoded| encoded.len() <= max_bytes)
            .unwrap_or(false)
            || kept.is_empty()
        {
            break;
        }
        kept.pop();
    }
    compacted
}

fn compact_generic_json(value: &Value, artifact_id: Uuid) -> Value {
    let (kind, item_count, keys) = match value {
        Value::Array(items) => ("array", Some(items.len()), Vec::new()),
        Value::Object(object) => (
            "object",
            None,
            object.keys().take(64).cloned().collect::<Vec<_>>(),
        ),
        Value::String(_) => ("string", None, Vec::new()),
        _ => ("scalar", None, Vec::new()),
    };
    json!({
        "truncated": true,
        "kind": kind,
        "itemCount": item_count,
        "keys": keys,
        "artifactId": artifact_id,
        "continuation": "Use read_artifact to retrieve a bounded window of the full result."
    })
}

fn compact_provider_metadata(metadata: &Value) -> Value {
    const KEYS: &[&str] = &[
        "toolName",
        "action",
        "operation",
        "success",
        "isError",
        "errorCode",
        "error",
        "changedPath",
        "documentId",
        "documentType",
        "rowsWritten",
        "columnsWritten",
        "artifactId",
        "artifactKind",
        "artifact",
        ENVELOPE_KEY,
    ];
    let Some(object) = metadata.as_object() else {
        return json!({ "metadataTruncated": true });
    };
    let mut compacted = serde_json::Map::new();
    for key in KEYS {
        if let Some(value) = object.get(*key) {
            compacted.insert((*key).to_string(), value.clone());
        }
    }
    compacted.insert("metadataTruncated".to_string(), Value::Bool(true));
    Value::Object(compacted)
}

#[allow(clippy::too_many_arguments)]
fn insert_envelope_metadata(
    metadata: &mut Value,
    strategy: &str,
    storage: &str,
    raw_bytes: usize,
    output_bytes: usize,
    artifact_id: Option<Uuid>,
    raw_output: &str,
) {
    ensure_object(metadata);
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert(
        ENVELOPE_KEY.to_string(),
        json!({
            "schemaVersion": ENVELOPE_SCHEMA_VERSION,
            "stage": "pre_model_ingress",
            "immutable": true,
            "strategy": strategy,
            "storage": storage,
            "rawBytes": raw_bytes,
            "outputBytes": output_bytes,
            "rawFingerprint": content_fingerprint(raw_output.as_bytes()),
            "artifactId": artifact_id,
        }),
    );
}

fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = json!({});
    }
}

pub fn tool_result_is_error(result: &ToolResult) -> bool {
    result
        .metadata
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| !success)
        || result
            .metadata
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || result.metadata.get("toolError").is_some()
        || result.metadata.get("errorRecord").is_some()
        || result.metadata.get("error").is_some()
}

fn bound_metadata_error_strings(object: &mut serde_json::Map<String, Value>) {
    if let Some(Value::String(error)) = object.get_mut("error") {
        *error = truncate_middle_bytes(error, 2_000);
    }
    if let Some(record) = object.get_mut("errorRecord").and_then(Value::as_object_mut) {
        if let Some(Value::String(message)) = record.get_mut("message") {
            *message = truncate_middle_bytes(message, 2_000);
        }
        if let Some(causes) = record.get_mut("causes").and_then(Value::as_array_mut) {
            causes.truncate(8);
            for cause in causes {
                if let Value::String(cause) = cause {
                    *cause = truncate_middle_bytes(cause, 1_000);
                }
            }
        }
    }
    if let Some(chain) = object.get_mut("errorChain").and_then(Value::as_array_mut) {
        chain.truncate(8);
        for cause in chain {
            if let Value::String(cause) = cause {
                *cause = truncate_middle_bytes(cause, 1_000);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArtifactStorage;
    use crate::store::{SessionStore, SqliteSessionStore};
    use std::sync::Arc;

    #[test]
    fn source_truncation_is_artifact_backed_and_history_does_not_truncate_twice() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let state = ToolStateStore::new(store.clone());
        let thread = store
            .create_thread(Some("tool ingress".to_string()), std::env::temp_dir())
            .expect("create thread");
        let raw = format!(
            "$ cargo test\n\n[stdout]\n{}done\n\n[stderr]\n",
            "passing test output\n".repeat(3_000)
        );
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: raw.clone(),
            content: vec![ModelContentPart::text(raw.clone())],
            metadata: json!({
                "success": true,
                "exitCode": 0,
                "durationMs": 1200,
                "stdout": "display copy",
                "stderr": "",
            }),
        };

        let source_bounded = crate::tool_output_truncation::truncate_tool_result_at_source(
            "shell",
            result,
            crate::tool_output_truncation::ToolOutputSourceKind::Shell,
            Some(&state),
            Some(thread.id),
        );
        assert_eq!(
            source_bounded.metadata["toolOutputTruncation"]["stage"],
            "tool_output_source"
        );
        let source_output = source_bounded.output.clone();
        let normalized = normalize_tool_result_at_ingress(
            "shell",
            source_bounded,
            Some(&state),
            Some(thread.id),
        );
        assert!(normalized.output.len() < raw.len());
        assert_eq!(normalized.output, source_output);
        assert!(normalized.output.contains("Full output artifact"));
        assert_eq!(
            normalized.metadata[ENVELOPE_KEY]["stage"],
            "pre_model_ingress"
        );
        let artifact_id = Uuid::parse_str(
            normalized.metadata["artifactId"]
                .as_str()
                .expect("artifact id"),
        )
        .expect("valid artifact id");
        let artifact = store
            .get_artifact(thread.id, artifact_id)
            .expect("load artifact")
            .expect("artifact exists");
        assert!(matches!(
            artifact.storage,
            ArtifactStorage::Inline { ref content } if content == &raw
        ));

        let serialized = serde_json::to_value(&normalized).expect("serialize result");
        let replayed =
            normalize_tool_result_at_ingress("shell", normalized, Some(&state), Some(thread.id));
        assert_eq!(
            serde_json::to_value(replayed).expect("serialize replay"),
            serialized
        );
        assert_eq!(store.list_artifacts(thread.id).unwrap().len(), 1);
    }

    #[test]
    fn provider_projection_removes_duplicate_text_and_stream_copies() {
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: "ok".to_string(),
            content: vec![
                ModelContentPart::text("ok"),
                ModelContentPart::resource("artifact://1", None, None),
            ],
            metadata: json!({
                "success": true,
                "stdout": "ok",
                "stderr": "",
                "exitCode": 0,
            }),
        };

        let content = provider_tool_result_content(&result);
        assert_eq!(content.len(), 1);
        assert!(matches!(content[0], ModelContentPart::Resource { .. }));
        let metadata = provider_tool_result_metadata("shell", &result.metadata);
        assert!(metadata.get("stdout").is_none());
        assert!(metadata.get("stderr").is_none());
        assert_eq!(metadata["exitCode"], 0);
    }

    #[test]
    fn provider_projection_minifies_json_and_removes_duplicate_structured_copies() {
        let value = json!({
            "entries": [
                { "path": "src/lib.rs", "kind": "file" },
                { "path": "src/main.rs", "kind": "file" }
            ],
            "truncated": false
        });
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: serde_json::to_string_pretty(&value).unwrap(),
            content: vec![ModelContentPart::json(value.clone())],
            metadata: json!({
                "toolName": "filesystem",
                "operation": "list",
                "path": ".",
                "count": 2,
                "truncated": false,
                "success": true
            }),
        };

        assert_eq!(provider_tool_result_output(&result), value.to_string());
        assert!(provider_tool_result_content(&result).is_empty());
        assert_eq!(
            provider_tool_result_metadata("filesystem", &result.metadata),
            json!({})
        );
        assert!(provider_visible_tool_result_bytes(&result) < result.output.len());
    }

    #[test]
    fn work_form_projection_keeps_the_compact_render_only() {
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: "Objective: ship\nWork form revision: 2\nStatus: Active".to_string(),
            content: vec![ModelContentPart::json(json!({
                "objective": "ship",
                "revision": 2,
                "status": "active"
            }))],
            metadata: json!({
                "toolName": "update_plan",
                "workForm": { "objective": "ship", "revision": 2 },
                "formId": Uuid::new_v4(),
                "revision": 2,
                "status": "active",
                "success": true
            }),
        };

        assert!(provider_tool_result_content(&result).is_empty());
        assert_eq!(
            provider_tool_result_metadata("update_plan", &result.metadata),
            json!({})
        );
    }

    #[test]
    fn provider_projection_preserves_large_user_input_control_metadata() {
        let metadata = json!({
            "toolName": "request_user_input",
            "success": true,
            "userInputRequest": {
                "requestId": Uuid::new_v4(),
                "questions": [{
                    "id": "q1",
                    "header": "Choice",
                    "question": "Choose an approach",
                    "options": [{
                        "id": "o1",
                        "label": "First",
                        "description": "x".repeat(PROVIDER_METADATA_MAX_BYTES),
                        "recommended": true
                    }, {
                        "id": "o2",
                        "label": "Second",
                        "description": "y".repeat(PROVIDER_METADATA_MAX_BYTES),
                        "recommended": false
                    }],
                    "allowCustom": true
                }]
            }
        });

        let projected = provider_tool_result_metadata("request_user_input", &metadata);
        assert!(projected.get("userInputRequest").is_some());
        assert!(projected.get("metadataTruncated").is_none());
    }

    #[test]
    fn provider_projection_drops_artifact_backed_text_but_keeps_typed_media() {
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output:
                "bounded output\n\n[Full output artifact: 00000000-0000-0000-0000-000000000001]"
                    .to_string(),
            content: vec![
                ModelContentPart::text("very long original MCP text"),
                ModelContentPart::image("image/png", vec![1, 2, 3]),
            ],
            metadata: json!({
                "toolSource": "mcp",
                "raw": { "large": "duplicate" },
                ENVELOPE_KEY: {
                    "storage": "artifact_backed"
                }
            }),
        };

        let content = provider_tool_result_content(&result);
        assert_eq!(content.len(), 1);
        assert!(matches!(content[0], ModelContentPart::Image { .. }));
        let metadata = provider_tool_result_metadata("server__tool", &result.metadata);
        assert!(metadata.get("raw").is_none());
    }

    #[test]
    fn oversized_filtered_rows_are_artifact_backed_and_page_shaped() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let state = ToolStateStore::new(store.clone());
        let thread = store
            .create_thread(
                Some("spreadsheet ingress".to_string()),
                std::env::temp_dir(),
            )
            .expect("create thread");
        let rows = (0..500)
            .map(|row| {
                json!([
                    { "value": { "type": "string", "value": format!("row-{row}-{}", "x".repeat(80)) } },
                    { "value": { "type": "number", "value": row } }
                ])
            })
            .collect::<Vec<_>>();
        let value = json!({
            "type": "rows_filtered",
            "result": {
                "path": "orders.xlsx",
                "sheet": "Orders",
                "range": {
                    "start": { "row": 10, "column": 0 },
                    "end": { "row": 509, "column": 1 }
                },
                "matchedRowCount": 500,
                "matchedRowIndices": (10..510).collect::<Vec<_>>(),
                "rows": rows
            }
        });
        let output = serde_json::to_string_pretty(&value).expect("serialize spreadsheet result");
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: output.clone(),
            content: vec![ModelContentPart::json(value)],
            metadata: json!({
                "toolName": "spreadsheet_filter_rows",
                "operation": "filter_rows",
                "success": true
            }),
        };

        let normalized = normalize_tool_result_at_ingress(
            "spreadsheet_filter_rows",
            result,
            Some(&state),
            Some(thread.id),
        );
        let projected = provider_tool_result_content(&normalized);
        let ModelContentPart::Json { value } = &projected[0] else {
            panic!("spreadsheet projection must stay structured");
        };
        assert_eq!(value.pointer("/result/totalRowCount"), Some(&json!(500)));
        assert_eq!(value.pointer("/result/hasMore"), Some(&json!(true)));
        assert_eq!(
            value
                .pointer("/result/matchedRowIndices")
                .and_then(Value::as_array)
                .map(Vec::len),
            value
                .pointer("/result/rows")
                .and_then(Value::as_array)
                .map(Vec::len)
        );
        assert!(value
            .pointer("/result/nextRow")
            .and_then(Value::as_u64)
            .is_some_and(|row| row > 10));
        assert!(
            serde_json::to_vec(value).unwrap().len()
                <= token_budget_bytes(HISTORY_TOOL_OUTPUT_MAX_TOKENS)
        );
        assert!(
            provider_visible_tool_result_bytes(&normalized) <= PROVIDER_RESULT_MAX_NON_MEDIA_BYTES
        );
        let artifact_id = normalized.metadata["artifactId"]
            .as_str()
            .and_then(|value| Uuid::parse_str(value).ok())
            .expect("artifact id");
        let artifact = store
            .get_artifact(thread.id, artifact_id)
            .expect("load artifact")
            .expect("artifact exists");
        assert!(
            matches!(artifact.storage, ArtifactStorage::Inline { content } if content == output)
        );
    }
}
