use crate::model::{Artifact, ModelContentPart, ToolResult};
use crate::model_context::content_fingerprint;
use crate::store::SessionStore;
use serde_json::{json, Value};
use uuid::Uuid;

const ENVELOPE_KEY: &str = "toolResultEnvelope";
const ENVELOPE_SCHEMA_VERSION: u64 = 1;
const SEARCH_MODEL_OUTPUT_MAX_BYTES: usize = 12_000;
const SHELL_SUCCESS_OUTPUT_MAX_BYTES: usize = 8_000;
const FAILURE_OUTPUT_MAX_BYTES: usize = 12_000;
const GENERIC_SUCCESS_OUTPUT_MAX_BYTES: usize = 12_000;
const GENERIC_SUCCESS_NORMALIZE_AFTER_BYTES: usize = 16_000;

/// Builds the immutable, provider-visible form of a tool result exactly once,
/// before the first model round can observe it. If normalization would discard
/// text, the original output is persisted as an artifact first. When artifact
/// storage is unavailable, the lossless result is retained instead.
pub(crate) fn normalize_tool_result_at_ingress(
    tool_name: &str,
    mut result: ToolResult,
    store: Option<&dyn SessionStore>,
    thread_id: Option<Uuid>,
) -> ToolResult {
    if result.metadata.get(ENVELOPE_KEY).is_some() {
        return result;
    }

    let raw_output = result.output.clone();
    let is_error = tool_result_is_error(&result);
    let (candidate, strategy) = match tool_name {
        "shell" => compact_shell_output(&raw_output, &result.metadata, is_error),
        "search" => (
            compact_search_output(&raw_output, SEARCH_MODEL_OUTPUT_MAX_BYTES),
            "search_matches",
        ),
        _ if is_error => (
            compact_failure_output(&raw_output, FAILURE_OUTPUT_MAX_BYTES),
            "failure_diagnostics",
        ),
        _ if raw_output.len() > GENERIC_SUCCESS_NORMALIZE_AFTER_BYTES => (
            compact_head_tail(&raw_output, GENERIC_SUCCESS_OUTPUT_MAX_BYTES),
            "bounded_head_tail",
        ),
        _ => (raw_output.clone(), "passthrough"),
    };

    if candidate == raw_output {
        insert_envelope_metadata(
            &mut result.metadata,
            strategy,
            "inline",
            raw_output.len(),
            raw_output.len(),
            None,
            &raw_output,
        );
        return result;
    }

    let existing_artifact_id = artifact_id_from_metadata(&result.metadata);
    let reused_artifact = existing_artifact_id.is_some();
    let artifact_id = existing_artifact_id.or_else(|| {
        let (Some(store), Some(thread_id)) = (store, thread_id) else {
            return None;
        };
        let artifact = Artifact::inline(
            thread_id,
            "tool_output",
            "text/plain; charset=utf-8",
            raw_output.clone(),
            json!({
                "source": "tool_result_ingress",
                "callId": result.call_id,
                "toolName": tool_name,
                "outputBytes": raw_output.len(),
                "outputFingerprint": content_fingerprint(raw_output.as_bytes()),
                "toolResultMetadata": result.metadata.clone(),
            }),
        );
        store
            .insert_artifact(artifact)
            .ok()
            .map(|artifact| artifact.id)
    });

    let Some(artifact_id) = artifact_id else {
        // Saving tokens must never destroy the only copy of a tool result.
        insert_envelope_metadata(
            &mut result.metadata,
            strategy,
            "inline_artifact_unavailable",
            raw_output.len(),
            raw_output.len(),
            None,
            &raw_output,
        );
        return result;
    };

    let normalized_output = format!(
        "{}\n\n[Full output artifact: {artifact_id}]",
        candidate.trim_end()
    );
    replace_matching_text_content(&mut result.content, &raw_output, &normalized_output);
    result.output = normalized_output;
    if !reused_artifact {
        insert_artifact_metadata(&mut result.metadata, artifact_id, raw_output.len());
    }
    let output_bytes = result.output.len();
    insert_envelope_metadata(
        &mut result.metadata,
        strategy,
        "artifact_backed",
        raw_output.len(),
        output_bytes,
        Some(artifact_id),
        &raw_output,
    );
    result
}

/// Provider adapters serialize `output`, `content`, and `metadata` together.
/// Avoid sending an identical legacy text part a second time.
pub(crate) fn provider_tool_result_content(result: &ToolResult) -> Vec<ModelContentPart> {
    let artifact_backed = result
        .metadata
        .pointer(&format!("/{ENVELOPE_KEY}/storage"))
        .and_then(Value::as_str)
        == Some("artifact_backed");
    result
        .content
        .iter()
        .filter(|part| {
            !matches!(
                part,
                ModelContentPart::Text { text }
                    if text.is_empty() || artifact_backed || result.output.contains(text)
            )
        })
        .cloned()
        .collect()
}

/// Keeps event metadata useful for the desktop while removing fields that are
/// already represented in the provider-visible output. The returned value is
/// created once with the ProviderToolResult and then replayed byte-for-byte.
pub(crate) fn provider_tool_result_metadata(tool_name: &str, metadata: &Value) -> Value {
    let mut metadata = metadata.clone();
    let Some(object) = metadata.as_object_mut() else {
        return metadata;
    };
    match tool_name {
        "shell" => {
            object.remove("stdout");
            object.remove("stderr");
        }
        "search" => {
            object.remove("locations");
        }
        _ => {}
    }
    if object.get("toolSource").and_then(Value::as_str) == Some("mcp")
        || (object.contains_key("serverId") && object.contains_key("publicName"))
    {
        object.remove("raw");
    }
    bound_metadata_error_strings(object);
    metadata
}

fn compact_shell_output(raw: &str, metadata: &Value, is_error: bool) -> (String, &'static str) {
    let Some((command, stdout, stderr)) = parse_shell_envelope(raw) else {
        return if is_error {
            (
                compact_failure_output(raw, FAILURE_OUTPUT_MAX_BYTES),
                "shell_failure_diagnostics",
            )
        } else if raw.len() > SHELL_SUCCESS_OUTPUT_MAX_BYTES {
            (
                compact_head_tail(raw, SHELL_SUCCESS_OUTPUT_MAX_BYTES),
                "shell_success_head_tail",
            )
        } else {
            (raw.to_string(), "passthrough")
        };
    };

    let duration = metadata.get("durationMs").and_then(Value::as_u64);
    let exit_code = metadata.get("exitCode").and_then(Value::as_i64);
    let mut header = format!("$ {command}");
    if let Some(exit_code) = exit_code {
        header.push_str(&format!("\n[exit code: {exit_code}"));
        if let Some(duration) = duration {
            header.push_str(&format!(", duration: {duration} ms"));
        }
        header.push(']');
    } else if let Some(duration) = duration {
        header.push_str(&format!("\n[duration: {duration} ms]"));
    }

    if is_error {
        if raw.len() <= FAILURE_OUTPUT_MAX_BYTES {
            return (raw.to_string(), "passthrough");
        }
        let mut sections = vec![header];
        let stderr_excerpt = diagnostic_excerpt(stderr, FAILURE_OUTPUT_MAX_BYTES * 2 / 3);
        if !stderr_excerpt.trim().is_empty() {
            sections.push(format!("[stderr diagnostics]\n{stderr_excerpt}"));
        }
        let stdout_excerpt = diagnostic_excerpt(stdout, FAILURE_OUTPUT_MAX_BYTES / 3);
        if !stdout_excerpt.trim().is_empty() {
            sections.push(format!("[stdout diagnostics]\n{stdout_excerpt}"));
        }
        return (
            compact_head_tail(&sections.join("\n\n"), FAILURE_OUTPUT_MAX_BYTES),
            "shell_failure_diagnostics",
        );
    }

    if raw.len() <= SHELL_SUCCESS_OUTPUT_MAX_BYTES {
        return (raw.to_string(), "passthrough");
    }
    let body_budget = SHELL_SUCCESS_OUTPUT_MAX_BYTES.saturating_sub(header.len() + 32);
    let mut sections = vec![header];
    if !stdout.trim().is_empty() {
        sections.push(format!(
            "[stdout]\n{}",
            compact_head_tail(stdout, body_budget * 3 / 4)
        ));
    }
    if !stderr.trim().is_empty() {
        sections.push(format!(
            "[stderr]\n{}",
            compact_head_tail(stderr, body_budget / 4)
        ));
    }
    (
        compact_head_tail(&sections.join("\n\n"), SHELL_SUCCESS_OUTPUT_MAX_BYTES),
        "shell_success_head_tail",
    )
}

fn parse_shell_envelope(raw: &str) -> Option<(&str, &str, &str)> {
    const STDOUT_MARKER: &str = "\n\n[stdout]\n";
    const STDERR_MARKER: &str = "\n\n[stderr]\n";
    let stdout_at = raw.find(STDOUT_MARKER)?;
    let header = &raw[..stdout_at];
    let command = header.strip_prefix("$ ")?;
    let body = &raw[stdout_at + STDOUT_MARKER.len()..];
    let stderr_at = body.rfind(STDERR_MARKER)?;
    Some((
        command,
        &body[..stderr_at],
        &body[stderr_at + STDERR_MARKER.len()..],
    ))
}

fn compact_search_output(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    let units = if raw.contains("\n--\n") {
        raw.split("\n--\n").collect::<Vec<_>>()
    } else {
        raw.lines().collect::<Vec<_>>()
    };
    let separator = if raw.contains("\n--\n") {
        "\n--\n"
    } else {
        "\n"
    };
    let mut kept = Vec::new();
    let reserve = 96usize;
    let budget = max_bytes.saturating_sub(reserve);
    let mut bytes = 0usize;
    for unit in &units {
        let next = unit.len() + usize::from(!kept.is_empty()) * separator.len();
        if bytes + next > budget {
            if kept.is_empty() {
                kept.push(utf8_prefix(unit, budget).to_string());
            }
            break;
        }
        kept.push((*unit).to_string());
        bytes += next;
    }
    let omitted = units.len().saturating_sub(kept.len());
    let mut output = kept.join(separator);
    output.push_str(&format!(
        "\n\n[{omitted} additional search match block(s) omitted from model context]"
    ));
    output
}

fn compact_failure_output(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    diagnostic_excerpt(raw, max_bytes)
}

fn diagnostic_excerpt(raw: &str, max_bytes: usize) -> String {
    let lines = raw.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return String::new();
    }
    let mut selected = vec![false; lines.len()];
    for (index, line) in lines.iter().enumerate() {
        if is_diagnostic_line(line) {
            let start = index.saturating_sub(1);
            let end = (index + 2).min(lines.len() - 1);
            selected[start..=end].fill(true);
        }
    }
    // Build/test tools commonly put their authoritative summary at the end.
    let tail_start = lines.len().saturating_sub(24);
    selected[tail_start..].fill(true);

    let mut rendered = Vec::new();
    let mut previous = None;
    for (index, line) in lines.iter().enumerate() {
        if !selected[index] {
            continue;
        }
        if previous.is_some_and(|previous| index > previous + 1) {
            rendered.push("[... unrelated output omitted ...]".to_string());
        }
        rendered.push((*line).to_string());
        previous = Some(index);
    }
    compact_head_tail(&rendered.join("\n"), max_bytes)
}

fn is_diagnostic_line(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.contains("0 errors")
        || normalized.contains("0 failed")
        || normalized.contains("errors: 0")
        || normalized.contains("failed: 0")
    {
        return false;
    }
    [
        "error",
        "fatal",
        "failed",
        "failure",
        "exception",
        "traceback",
        "panic",
        "not found",
        "no such file",
        "cannot ",
        "could not",
        "denied",
        "invalid",
        "timed out",
        "timeout",
        "undefined reference",
        "categoryinfo",
        "fullyqualifiederrorid",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn compact_head_tail(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    let marker_reserve = 80usize.min(max_bytes);
    let content_budget = max_bytes.saturating_sub(marker_reserve);
    let head_budget = content_budget * 2 / 3;
    let tail_budget = content_budget.saturating_sub(head_budget);
    let head = utf8_prefix(raw, head_budget);
    let tail = utf8_suffix(raw, tail_budget);
    let omitted = raw.len().saturating_sub(head.len() + tail.len());
    format!("{head}\n\n[{omitted} bytes omitted]\n\n{tail}")
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

fn replace_matching_text_content(content: &mut [ModelContentPart], raw: &str, normalized: &str) {
    for part in content {
        if let ModelContentPart::Text { text } = part {
            if text == raw {
                *text = normalized.to_string();
            }
        }
    }
}

fn artifact_id_from_metadata(metadata: &Value) -> Option<Uuid> {
    metadata
        .get("artifactId")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
        .or_else(|| {
            metadata
                .pointer("/artifact/id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
        })
}

fn insert_artifact_metadata(metadata: &mut Value, artifact_id: Uuid, raw_bytes: usize) {
    ensure_object(metadata);
    let Some(object) = metadata.as_object_mut() else {
        return;
    };
    object.insert("artifactId".to_string(), json!(artifact_id));
    object.insert("artifactKind".to_string(), json!("tool_output"));
    object.insert(
        "artifact".to_string(),
        json!({
            "id": artifact_id,
            "kind": "tool_output",
            "contentType": "text/plain; charset=utf-8",
            "bytes": raw_bytes,
        }),
    );
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

fn tool_result_is_error(result: &ToolResult) -> bool {
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
}

fn bound_metadata_error_strings(object: &mut serde_json::Map<String, Value>) {
    if let Some(Value::String(error)) = object.get_mut("error") {
        *error = compact_failure_output(error, 2_000);
    }
    if let Some(record) = object.get_mut("errorRecord").and_then(Value::as_object_mut) {
        if let Some(Value::String(message)) = record.get_mut("message") {
            *message = compact_failure_output(message, 2_000);
        }
        if let Some(causes) = record.get_mut("causes").and_then(Value::as_array_mut) {
            causes.truncate(8);
            for cause in causes {
                if let Value::String(cause) = cause {
                    *cause = compact_failure_output(cause, 1_000);
                }
            }
        }
    }
    if let Some(chain) = object.get_mut("errorChain").and_then(Value::as_array_mut) {
        chain.truncate(8);
        for cause in chain {
            if let Value::String(cause) = cause {
                *cause = compact_failure_output(cause, 1_000);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArtifactStorage;
    use crate::store::SqliteSessionStore;

    #[test]
    fn failure_compaction_finds_diagnostic_in_the_middle_and_keeps_tail() {
        let raw = format!(
            "{}\nerror[E0425]: cannot find value `target` in this scope\n  --> src/lib.rs:42:9\n{}\nfinal test summary: FAILED",
            "ordinary build chatter\n".repeat(900),
            "more unrelated chatter\n".repeat(900),
        );
        let compacted = compact_failure_output(&raw, FAILURE_OUTPUT_MAX_BYTES);

        assert!(compacted.contains("error[E0425]"));
        assert!(compacted.contains("src/lib.rs:42:9"));
        assert!(compacted.contains("final test summary: FAILED"));
        assert!(compacted.len() <= FAILURE_OUTPUT_MAX_BYTES);
    }

    #[test]
    fn normalized_result_is_artifact_backed_and_idempotent() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
        let thread = store
            .create_thread(Some("tool ingress".to_string()), std::env::temp_dir())
            .expect("create thread");
        let raw = format!(
            "$ cargo test\n\n[stdout]\n{}done\n\n[stderr]\n",
            "passing test output\n".repeat(1_000)
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

        let normalized =
            normalize_tool_result_at_ingress("shell", result, Some(&store), Some(thread.id));
        assert!(normalized.output.len() < raw.len());
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
            normalize_tool_result_at_ingress("shell", normalized, Some(&store), Some(thread.id));
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
    fn search_compaction_keeps_complete_early_match_blocks() {
        let block = |index: usize| {
            format!(
                "src/file_{index}.rs:{index}:1\n> {index} | matching source {}",
                "x".repeat(300)
            )
        };
        let raw = (1..=100).map(block).collect::<Vec<_>>().join("\n--\n");
        let compacted = compact_search_output(&raw, SEARCH_MODEL_OUTPUT_MAX_BYTES);

        assert!(compacted.contains("src/file_1.rs:1:1"));
        assert!(compacted.contains("additional search match block(s) omitted"));
        assert!(compacted.len() <= SEARCH_MODEL_OUTPUT_MAX_BYTES);
    }
}
