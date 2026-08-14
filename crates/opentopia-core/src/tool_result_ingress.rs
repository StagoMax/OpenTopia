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
const STRUCTURED_CONTENT_MAX_BYTES: usize = 12_000;
const PROVIDER_METADATA_MAX_BYTES: usize = 4_000;
const PROVIDER_RESULT_MAX_NON_MEDIA_BYTES: usize = 36_000;

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
    let (mut candidate, mut strategy) = match tool_name {
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

    let oversized_structured_content = result.content.iter().any(|part| {
        matches!(
            part,
            ModelContentPart::Json { value }
                if serde_json::to_vec(value)
                    .map(|encoded| encoded.len() > STRUCTURED_CONTENT_MAX_BYTES)
                    .unwrap_or(true)
        )
    });
    if candidate == raw_output && oversized_structured_content {
        candidate = compact_head_tail(&raw_output, GENERIC_SUCCESS_OUTPUT_MAX_BYTES);
        strategy = "bounded_structured_content";
    }

    if candidate == raw_output && !oversized_structured_content {
        insert_envelope_metadata(
            &mut result.metadata,
            strategy,
            "inline",
            raw_output.len(),
            raw_output.len(),
            None,
            &raw_output,
        );
        debug_assert!(
            provider_visible_tool_result_bytes(&result) <= PROVIDER_RESULT_MAX_NON_MEDIA_BYTES
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
    bound_structured_content(
        tool_name,
        &mut result.content,
        STRUCTURED_CONTENT_MAX_BYTES,
        artifact_id,
    );
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
    debug_assert!(
        provider_visible_tool_result_bytes(&result) <= PROVIDER_RESULT_MAX_NON_MEDIA_BYTES
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
    if serde_json::to_vec(&metadata)
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
    result.output.len()
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

fn bound_structured_content(
    tool_name: &str,
    content: &mut [ModelContentPart],
    max_bytes: usize,
    artifact_id: Uuid,
) {
    for part in content {
        let ModelContentPart::Json { value } = part else {
            continue;
        };
        let exceeds_limit = serde_json::to_vec(value)
            .map(|encoded| encoded.len() > max_bytes)
            .unwrap_or(true);
        if !exceeds_limit {
            continue;
        }
        *value = if tool_name == "spreadsheet" {
            compact_spreadsheet_json(value, max_bytes, artifact_id)
        } else {
            compact_generic_json(value, artifact_id)
        };
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
    {
        let Some(compacted_result) = compacted.get_mut("result").and_then(Value::as_object_mut)
        else {
            return compact_generic_json(value, artifact_id);
        };
        compacted_result.insert("rows".to_string(), Value::Array(Vec::new()));
        compacted_result.insert("totalRowCount".to_string(), json!(rows.len()));
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
        compacted_result.insert("returnedRowCount".to_string(), json!(returned));
        compacted_result.insert("hasMore".to_string(), json!(returned < rows.len()));
        if returned < rows.len() {
            compacted_result.insert("nextRow".to_string(), json!(start_row + returned as u64));
            compacted_result.insert(
                "continuation".to_string(),
                json!("Call spreadsheet read_rows/read_range for the remaining rows, or read_artifact for the full serialized result."),
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
        "success",
        "isError",
        "errorCode",
        "error",
        "changedPath",
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
    fn oversized_spreadsheet_json_is_artifact_backed_and_page_shaped() {
        let store = SqliteSessionStore::open(":memory:").expect("open store");
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
            "type": "range_read",
            "result": {
                "path": "orders.xlsx",
                "sheet": "Orders",
                "range": {
                    "start": { "row": 10, "column": 0 },
                    "end": { "row": 509, "column": 1 }
                },
                "rows": rows
            }
        });
        let output = serde_json::to_string_pretty(&value).expect("serialize spreadsheet result");
        let result = ToolResult {
            call_id: Uuid::new_v4(),
            output: output.clone(),
            content: vec![ModelContentPart::json(value)],
            metadata: json!({ "toolName": "spreadsheet", "success": true }),
        };

        let normalized =
            normalize_tool_result_at_ingress("spreadsheet", result, Some(&store), Some(thread.id));
        let projected = provider_tool_result_content(&normalized);
        let ModelContentPart::Json { value } = &projected[0] else {
            panic!("spreadsheet projection must stay structured");
        };
        assert_eq!(value.pointer("/result/totalRowCount"), Some(&json!(500)));
        assert_eq!(value.pointer("/result/hasMore"), Some(&json!(true)));
        assert!(value
            .pointer("/result/nextRow")
            .and_then(Value::as_u64)
            .is_some_and(|row| row > 10));
        assert!(serde_json::to_vec(value).unwrap().len() <= STRUCTURED_CONTENT_MAX_BYTES);
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
