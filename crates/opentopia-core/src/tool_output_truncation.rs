//! Shared model-facing output truncation primitives.
//!
//! Tool adapters apply the 10k-token producer budget when they materialize a
//! result. The tool-result ingress applies the 1.2x history safety budget. Both
//! layers deliberately use the same symmetric middle truncation so a result
//! that already fits after producer shaping is not truncated a second time.

use crate::model::{Artifact, ModelContentPart, ToolResult};
use crate::model_context::content_fingerprint;
use crate::tool_state::ToolStateStore;
use serde_json::{json, Value};
use uuid::Uuid;

const APPROX_BYTES_PER_TOKEN: usize = 4;
const SOURCE_TRUNCATION_KEY: &str = "toolOutputTruncation";
pub(crate) const DEFAULT_TOOL_OUTPUT_MAX_TOKENS: usize = 10_000;
pub(crate) const HISTORY_TOOL_OUTPUT_MAX_TOKENS: usize = DEFAULT_TOOL_OUTPUT_MAX_TOKENS * 12 / 10;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolOutputSourceKind {
    Shell,
    WorkspaceSearch,
    Mcp,
}

impl ToolOutputSourceKind {
    fn strategy(self) -> &'static str {
        match self {
            Self::Shell => "shell_formatted_middle",
            Self::WorkspaceSearch => "search_formatted_middle",
            Self::Mcp => "mcp_content_aware",
        }
    }
}

/// Applies the producer budget where a tool materializes its result. Full text
/// is saved before truncation, while typed media stays inline.
pub(crate) fn truncate_tool_result_at_source(
    tool_name: &str,
    mut result: ToolResult,
    kind: ToolOutputSourceKind,
    store: Option<&ToolStateStore>,
    thread_id: Option<Uuid>,
) -> ToolResult {
    if result.metadata.get(SOURCE_TRUNCATION_KEY).is_some() {
        return result;
    }

    let raw_output = result.output.clone();
    let max_bytes = token_budget_bytes(DEFAULT_TOOL_OUTPUT_MAX_TOKENS);
    let candidate = formatted_truncate_text(&raw_output, DEFAULT_TOOL_OUTPUT_MAX_TOKENS);
    let oversized_content = result.content.iter().any(|part| match part {
        ModelContentPart::Text { text } => text.len() > max_bytes,
        ModelContentPart::Json { value } => serde_json::to_vec(value)
            .map(|encoded| encoded.len() > max_bytes)
            .unwrap_or(true),
        ModelContentPart::Image { .. } | ModelContentPart::Resource { .. } => false,
    });
    if candidate == raw_output && !oversized_content {
        return result;
    }

    let Some(artifact_id) = ensure_output_artifact(
        tool_name,
        &mut result,
        &raw_output,
        store,
        thread_id,
        "tool_output_source",
    ) else {
        insert_source_truncation_metadata(
            &mut result.metadata,
            kind.strategy(),
            "inline_artifact_unavailable",
            raw_output.len(),
            raw_output.len(),
            None,
        );
        return result;
    };

    let normalized_output = with_artifact_reference(&candidate, artifact_id);
    replace_matching_text_content(&mut result.content, &raw_output, &normalized_output);
    bound_source_content(&mut result.content, max_bytes, artifact_id);
    result.output = normalized_output;
    insert_source_truncation_metadata(
        &mut result.metadata,
        kind.strategy(),
        "artifact_backed",
        raw_output.len(),
        result.output.len(),
        Some(artifact_id),
    );
    result
}

pub(crate) const fn token_budget_bytes(max_tokens: usize) -> usize {
    max_tokens.saturating_mul(APPROX_BYTES_PER_TOKEN)
}

pub(crate) fn approx_token_count(value: &str) -> usize {
    value.len().div_ceil(APPROX_BYTES_PER_TOKEN)
}

pub(crate) fn formatted_truncate_text(value: &str, max_tokens: usize) -> String {
    if value.len() <= token_budget_bytes(max_tokens) {
        return value.to_string();
    }

    let original_token_count = approx_token_count(value);
    let total_lines = value.lines().count();
    let truncated = truncate_middle_tokens(value, max_tokens);
    format!(
        "Warning: truncated output (original token count: {original_token_count})\n\
         Total output lines: {total_lines}\n\n{truncated}"
    )
}

pub(crate) fn truncate_middle_tokens(value: &str, max_tokens: usize) -> String {
    truncate_middle(value, token_budget_bytes(max_tokens), true)
}

pub(crate) fn truncate_middle_bytes(value: &str, max_bytes: usize) -> String {
    truncate_middle(value, max_bytes, false)
}

fn truncate_middle(value: &str, max_bytes: usize, report_tokens: bool) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    // Match Codex's primitive: the full policy budget is split 50/50 between
    // the prefix and suffix, and the human-readable marker is appended outside
    // that content budget.
    let (head_budget, tail_budget) = split_budget(max_bytes);
    let head = utf8_prefix(value, head_budget);
    let tail = utf8_suffix(value, tail_budget);
    let removed_chars = value[head.len()..value.len().saturating_sub(tail.len())]
        .chars()
        .count();
    let removed_count = if report_tokens {
        value
            .len()
            .saturating_sub(max_bytes)
            .div_ceil(APPROX_BYTES_PER_TOKEN)
    } else {
        removed_chars
    };
    let unit = if report_tokens { "tokens" } else { "chars" };
    format!("{head}…{removed_count} {unit} truncated…{tail}")
}

fn split_budget(budget: usize) -> (usize, usize) {
    let head = budget / 2;
    (head, budget.saturating_sub(head))
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn utf8_suffix(value: &str, max_bytes: usize) -> &str {
    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    &value[start..]
}

pub(crate) fn replace_matching_text_content(
    content: &mut [ModelContentPart],
    raw: &str,
    normalized: &str,
) {
    for part in content {
        if let ModelContentPart::Text { text } = part {
            if text == raw {
                *text = normalized.to_string();
            }
        }
    }
}

pub(crate) fn with_artifact_reference(candidate: &str, artifact_id: Uuid) -> String {
    let marker = format!("[Full output artifact: {artifact_id}]");
    if candidate.contains(&marker) {
        candidate.to_string()
    } else {
        format!("{}\n\n{marker}", candidate.trim_end())
    }
}

pub(crate) fn output_artifact_id_from_metadata(metadata: &Value) -> Option<Uuid> {
    metadata
        .pointer(&format!("/{SOURCE_TRUNCATION_KEY}/artifactId"))
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .or_else(|| {
            metadata
                .get("outputArtifactId")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
        })
        .or_else(|| {
            (metadata.get("artifactKind").and_then(Value::as_str) == Some("tool_output"))
                .then(|| metadata.get("artifactId").and_then(Value::as_str))
                .flatten()
                .and_then(|value| Uuid::parse_str(value).ok())
        })
}

pub(crate) fn ensure_output_artifact(
    tool_name: &str,
    result: &mut ToolResult,
    raw_output: &str,
    store: Option<&ToolStateStore>,
    thread_id: Option<Uuid>,
    source: &str,
) -> Option<Uuid> {
    if let Some(artifact_id) = output_artifact_id_from_metadata(&result.metadata) {
        return Some(artifact_id);
    }
    let (Some(store), Some(thread_id)) = (store, thread_id) else {
        return None;
    };
    let (content_type, artifact_content) = lossless_artifact_content(result, raw_output);
    let artifact = Artifact::inline(
        thread_id,
        "tool_output",
        content_type,
        artifact_content,
        json!({
            "source": source,
            "callId": result.call_id,
            "toolName": tool_name,
            "outputBytes": raw_output.len(),
            "outputFingerprint": content_fingerprint(raw_output.as_bytes()),
        }),
    );
    let artifact_id = store.insert_artifact(artifact).ok()?.id;
    insert_artifact_metadata(
        &mut result.metadata,
        artifact_id,
        raw_output.len(),
        content_type,
    );
    Some(artifact_id)
}

fn lossless_artifact_content(result: &ToolResult, raw_output: &str) -> (&'static str, String) {
    let output_json = serde_json::from_str::<Value>(raw_output).ok();
    let additional_content = result
        .content
        .iter()
        .filter(|part| match part {
            ModelContentPart::Text { text } => text != raw_output,
            ModelContentPart::Json { value } => output_json.as_ref() != Some(value),
            ModelContentPart::Resource { .. } => true,
            // Images remain typed and inline; copying their bytes into a text
            // artifact would create a second large payload without improving
            // model recoverability.
            ModelContentPart::Image { .. } => false,
        })
        .cloned()
        .collect::<Vec<_>>();
    if additional_content.is_empty() {
        return ("text/plain; charset=utf-8", raw_output.to_string());
    }

    let envelope = json!({
        "output": raw_output,
        "content": additional_content,
    });
    (
        "application/json",
        serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| raw_output.to_string()),
    )
}

fn insert_artifact_metadata(
    metadata: &mut Value,
    artifact_id: Uuid,
    raw_bytes: usize,
    content_type: &str,
) {
    ensure_object(metadata);
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert("outputArtifactId".to_string(), json!(artifact_id));
    if !object.contains_key("artifactId") {
        object.insert("artifactId".to_string(), json!(artifact_id));
        object.insert("artifactKind".to_string(), json!("tool_output"));
        object.insert(
            "artifact".to_string(),
            json!({
                "id": artifact_id,
                "kind": "tool_output",
                "contentType": content_type,
                "bytes": raw_bytes,
            }),
        );
    }
}

fn insert_source_truncation_metadata(
    metadata: &mut Value,
    strategy: &str,
    storage: &str,
    raw_bytes: usize,
    output_bytes: usize,
    artifact_id: Option<Uuid>,
) {
    ensure_object(metadata);
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert(
        SOURCE_TRUNCATION_KEY.to_string(),
        json!({
            "schemaVersion": 1,
            "stage": "tool_output_source",
            "strategy": strategy,
            "storage": storage,
            "maxOutputTokens": DEFAULT_TOOL_OUTPUT_MAX_TOKENS,
            "rawBytes": raw_bytes,
            "outputBytes": output_bytes,
            "artifactId": artifact_id,
        }),
    );
}

fn bound_source_content(content: &mut Vec<ModelContentPart>, max_bytes: usize, artifact_id: Uuid) {
    let mut remaining = max_bytes;
    let mut omitted_text = 0usize;
    let mut omitted_json = 0usize;
    let mut bounded = Vec::with_capacity(content.len());
    for part in std::mem::take(content) {
        match part {
            ModelContentPart::Text { .. } if remaining == 0 => omitted_text += 1,
            ModelContentPart::Text { text } => {
                if text.len() <= remaining {
                    remaining = remaining.saturating_sub(text.len());
                    bounded.push(ModelContentPart::text(text));
                } else {
                    let snippet = truncate_middle_bytes(&text, remaining);
                    if snippet.is_empty() {
                        omitted_text += 1;
                    } else {
                        bounded.push(ModelContentPart::text(snippet));
                    }
                    remaining = 0;
                }
            }
            ModelContentPart::Json { .. } if remaining == 0 => omitted_json += 1,
            ModelContentPart::Json { value } => {
                let encoded_bytes = serde_json::to_vec(&value)
                    .map(|encoded| encoded.len())
                    .unwrap_or(usize::MAX);
                if encoded_bytes <= remaining {
                    remaining = remaining.saturating_sub(encoded_bytes);
                    bounded.push(ModelContentPart::json(value));
                    continue;
                }
                let kind = match &value {
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                    _ => "scalar",
                };
                let summary = json!({
                    "truncated": true,
                    "kind": kind,
                    "artifactId": artifact_id,
                    "message": "Structured tool output exceeded the producer budget; read the full output artifact for the lossless result."
                });
                let summary_bytes = serde_json::to_vec(&summary)
                    .map(|encoded| encoded.len())
                    .unwrap_or(usize::MAX);
                if summary_bytes <= remaining {
                    remaining = remaining.saturating_sub(summary_bytes);
                    bounded.push(ModelContentPart::json(summary));
                } else {
                    omitted_json += 1;
                }
            }
            media @ (ModelContentPart::Image { .. } | ModelContentPart::Resource { .. }) => {
                bounded.push(media);
            }
        }
    }
    if omitted_text > 0 || omitted_json > 0 {
        bounded.push(ModelContentPart::text(format!(
            "[omitted {omitted_text} text items and {omitted_json} JSON items ...]"
        )));
    }
    *content = bounded;
}

fn ensure_object(value: &mut Value) {
    if !value.is_object() {
        *value = json!({});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArtifactStorage;
    use crate::store::{SessionStore, SqliteSessionStore};
    use std::sync::Arc;

    #[test]
    fn token_truncation_keeps_symmetric_head_and_tail() {
        let value = format!("{}{}", "a".repeat(120), "z".repeat(120));
        let truncated = truncate_middle_tokens(&value, 20);

        assert!(truncated.starts_with(&"a".repeat(40)));
        assert!(truncated.ends_with(&"z".repeat(40)));
        assert!(truncated.contains("…40 tokens truncated…"));
    }

    #[test]
    fn byte_truncation_preserves_utf8_boundaries() {
        let value = "前".repeat(200);
        let truncated = truncate_middle_bytes(&value, 128);

        assert!(truncated.len() < value.len());
        assert!(truncated.starts_with('前'));
        assert!(truncated.ends_with('前'));
        assert!(truncated.contains("chars truncated"));
    }

    #[test]
    fn formatted_output_reports_original_size_and_lines() {
        let value = "line\n".repeat(100);
        let truncated = formatted_truncate_text(&value, 20);

        assert!(truncated.starts_with("Warning: truncated output"));
        assert!(truncated.contains("Total output lines: 100"));
    }

    #[test]
    fn mcp_source_truncation_preserves_structured_data_in_artifact_and_keeps_image() {
        let store = Arc::new(SqliteSessionStore::open(":memory:").expect("open store"));
        let state = ToolStateStore::new(store.clone());
        let thread = store
            .create_thread(Some("mcp truncation".to_string()), std::env::temp_dir())
            .expect("create thread");
        let structured = json!({ "rows": ["x".repeat(50_000)] });
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: "short MCP summary".to_string(),
            content: vec![
                ModelContentPart::json(structured),
                ModelContentPart::image("image/png", vec![1, 2, 3]),
            ],
            metadata: json!({ "isError": false }),
        };

        let bounded = truncate_tool_result_at_source(
            "server__large_tool",
            result,
            ToolOutputSourceKind::Mcp,
            Some(&state),
            Some(thread.id),
        );

        assert!(bounded.output.contains("Full output artifact"));
        assert!(bounded
            .content
            .iter()
            .any(|part| matches!(part, ModelContentPart::Image { .. })));
        let ModelContentPart::Json { value } = &bounded.content[0] else {
            panic!("structured MCP result should remain a JSON summary");
        };
        assert_eq!(value["truncated"], true);

        let artifact_id =
            output_artifact_id_from_metadata(&bounded.metadata).expect("tool output artifact id");
        let artifact = store
            .get_artifact(thread.id, artifact_id)
            .expect("load artifact")
            .expect("artifact exists");
        assert_eq!(artifact.content_type, "application/json");
        assert!(matches!(
            artifact.storage,
            ArtifactStorage::Inline { ref content }
                if content.contains("\"rows\"") && content.contains(&"x".repeat(1_000))
        ));
    }
}
