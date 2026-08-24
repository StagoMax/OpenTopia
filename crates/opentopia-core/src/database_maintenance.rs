use crate::model::AgentEventPayload;
use anyhow::Context;
use rusqlite::backup::Backup;
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use serde_json::json;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const COMPACTED_STREAM_CHUNK_BYTES: usize = 8 * 1024;
const TRACE_EVENT_KINDS: &[&str] = &[
    "model_context_built",
    "model_request",
    "provider_request_sent",
    "provider_request_retried",
    "provider_response_headers_received",
    "provider_first_token_received",
    "provider_stream_progress",
    "provider_response_commit_started",
    "provider_response_received",
    "model_delta",
    "reasoning_delta",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCompactionReport {
    pub source: PathBuf,
    pub output: PathBuf,
    pub trace_archive: PathBuf,
    pub source_bytes: u64,
    pub output_bytes: u64,
    pub archived_event_rows: u64,
    pub compacted_event_rows: u64,
    pub compacted_agent_event_rows: u64,
    pub pruned_conversation_rows: u64,
}

/// Builds a verified compact database beside the source and archives every raw
/// trace row that is rewritten. The source is never modified or replaced.
pub fn compact_database_copy(
    source: impl AsRef<Path>,
    output: impl AsRef<Path>,
    trace_archive: impl AsRef<Path>,
) -> anyhow::Result<DatabaseCompactionReport> {
    let source = source.as_ref().canonicalize().with_context(|| {
        format!(
            "failed to resolve source database {}",
            source.as_ref().display()
        )
    })?;
    anyhow::ensure!(source.is_file(), "source database is not a file");
    let output = resolve_new_file_path(output.as_ref())?;
    let trace_archive = resolve_new_file_path(trace_archive.as_ref())?;
    anyhow::ensure!(source != output, "output database must differ from source");
    anyhow::ensure!(
        source != trace_archive && output != trace_archive,
        "source, output, and trace archive paths must be distinct"
    );

    let source_bytes = fs::metadata(&source)?.len();
    let result = compact_database_copy_inner(&source, &output, &trace_archive, source_bytes);
    if result.is_err() {
        let _ = fs::remove_file(&output);
        let _ = fs::remove_file(&trace_archive);
    }
    result
}

fn compact_database_copy_inner(
    source: &Path,
    output: &Path,
    trace_archive: &Path,
    source_bytes: u64,
) -> anyhow::Result<DatabaseCompactionReport> {
    let source_connection = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut output_connection = Connection::open(output)?;
    {
        let backup = Backup::new(&source_connection, &mut output_connection)?;
        backup.run_to_completion(512, Duration::from_millis(10), None)?;
    }
    drop(source_connection);

    output_connection.pragma_update(None, "foreign_keys", true)?;
    let archived_event_rows = archive_trace_events(&output_connection, trace_archive)?;
    strip_archived_diagnostic_payloads(&output_connection)?;
    let compacted_event_rows = compact_stream_events(&mut output_connection, "events")?;
    let compacted_agent_event_rows = compact_stream_events(&mut output_connection, "agent_events")?;
    let pruned_conversation_rows = prune_completed_conversation_streams(&output_connection)?;

    output_connection.execute_batch("PRAGMA journal_mode = DELETE; VACUUM;")?;
    crate::store_migrations::validate_database_integrity(&output_connection)?;
    output_connection.execute_batch("PRAGMA journal_mode = WAL;")?;
    drop(output_connection);

    Ok(DatabaseCompactionReport {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        trace_archive: trace_archive.to_path_buf(),
        source_bytes,
        output_bytes: fs::metadata(output)?.len(),
        archived_event_rows,
        compacted_event_rows,
        compacted_agent_event_rows,
        pruned_conversation_rows,
    })
}

fn resolve_new_file_path(path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!path.exists(), "refusing to overwrite {}", path.display());
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let parent = parent
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", parent.display()))?;
    let file_name = path
        .file_name()
        .context("output path must include a file name")?;
    Ok(parent.join(file_name))
}

fn archive_trace_events(connection: &Connection, destination: &Path) -> anyhow::Result<u64> {
    let file = File::create(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    let mut archive = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(1));
    archive.start_file("trace-events.jsonl", options)?;
    let mut archived = archive_event_table(connection, &mut archive, "events")?;
    archived += archive_event_table(connection, &mut archive, "agent_events")?;
    archive.finish()?;
    Ok(archived)
}

fn archive_event_table(
    connection: &Connection,
    destination: &mut impl Write,
    table: &str,
) -> anyhow::Result<u64> {
    let (kind_column, sequence_column, identity_columns) = match table {
        "events" => (
            "kind",
            "seq",
            "thread_id, turn_id, NULL AS session_id, NULL AS invocation_id",
        ),
        "agent_events" => (
            "event_kind",
            "event_seq",
            "agent_thread_id, agent_turn_id, session_id, invocation_id",
        ),
        _ => anyhow::bail!("unsupported trace table {table}"),
    };
    let placeholders = (1..=TRACE_EVENT_KINDS.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT id, {sequence_column}, {kind_column}, payload_json, created_at, {identity_columns} \
         FROM {table} WHERE {kind_column} IN ({placeholders}) ORDER BY {sequence_column}"
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(rusqlite::params_from_iter(TRACE_EVENT_KINDS.iter()))?;
    let mut count = 0_u64;
    while let Some(row) = rows.next()? {
        let payload_json: String = row.get(3)?;
        let record = json!({
            "table": table,
            "id": row.get::<_, String>(0)?,
            "sequence": row.get::<_, i64>(1)?,
            "kind": row.get::<_, String>(2)?,
            "payload": serde_json::from_str::<serde_json::Value>(&payload_json)
                .unwrap_or_else(|_| serde_json::Value::String(payload_json)),
            "createdAt": row.get::<_, String>(4)?,
            "threadId": row.get::<_, String>(5)?,
            "turnId": row.get::<_, Option<String>>(6)?,
            "sessionId": row.get::<_, Option<String>>(7)?,
            "invocationId": row.get::<_, Option<i64>>(8)?,
        });
        serde_json::to_writer(&mut *destination, &record)?;
        destination.write_all(b"\n")?;
        count += 1;
    }
    Ok(count)
}

fn strip_archived_diagnostic_payloads(connection: &Connection) -> anyhow::Result<()> {
    for table in ["events", "agent_events"] {
        let kind_column = if table == "events" {
            "kind"
        } else {
            "event_kind"
        };
        connection.execute_batch(&format!(
            r#"
            UPDATE {table}
            SET payload_json = CASE {kind_column}
                WHEN 'model_context_built' THEN json_remove(payload_json, '$.items')
                WHEN 'model_request' THEN json_remove(payload_json, '$.request')
                WHEN 'provider_request_sent' THEN json_remove(payload_json, '$.body')
                WHEN 'provider_request_retried' THEN json_remove(payload_json, '$.body')
                WHEN 'provider_response_received' THEN json_remove(payload_json, '$.body')
                ELSE payload_json
            END
            WHERE {kind_column} IN (
                'model_context_built', 'model_request', 'provider_request_sent',
                'provider_request_retried', 'provider_response_received'
            );
            "#
        ))?;
    }
    Ok(())
}

#[derive(Debug)]
struct StreamRow {
    id: String,
    seq: i64,
    kind: String,
    text: Option<String>,
}

#[derive(Debug)]
struct StreamMerge {
    keep_id: String,
    payload_json: String,
    delete_ids: Vec<String>,
}

fn stream_text(kind: &str, payload_json: &str) -> rusqlite::Result<Option<String>> {
    if !matches!(kind, "model_delta" | "reasoning_delta") {
        return Ok(None);
    }
    let payload: serde_json::Value = serde_json::from_str(payload_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    payload
        .get("text")
        .and_then(serde_json::Value::as_str)
        .map(|text| Some(text.to_string()))
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "stream event is missing text",
                )),
            )
        })
}

fn compact_stream_events(connection: &mut Connection, table: &str) -> anyhow::Result<u64> {
    let (partition_sql, sequence_column, kind_column) = match table {
        "events" => (
            "SELECT DISTINCT thread_id, turn_id FROM events WHERE kind IN ('model_delta', 'reasoning_delta')",
            "seq",
            "kind",
        ),
        "agent_events" => (
            "SELECT DISTINCT agent_thread_id, agent_turn_id FROM agent_events WHERE event_kind IN ('model_delta', 'reasoning_delta')",
            "event_seq",
            "event_kind",
        ),
        _ => anyhow::bail!("unsupported stream table {table}"),
    };
    let partitions = {
        let mut statement = connection.prepare(partition_sql)?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut removed = 0_u64;
    for (thread_id, turn_id) in partitions {
        let query = format!(
            "SELECT id, {sequence_column}, {kind_column}, payload_json FROM {table} \
             WHERE {} = ?1 AND {} IS ?2 ORDER BY {sequence_column}",
            if table == "events" {
                "thread_id"
            } else {
                "agent_thread_id"
            },
            if table == "events" {
                "turn_id"
            } else {
                "agent_turn_id"
            },
        );
        let rows = {
            let mut statement = connection.prepare(&query)?;
            let mapped = statement.query_map(params![thread_id, turn_id], |row| {
                let kind: String = row.get(2)?;
                let payload_json: String = row.get(3)?;
                let text = stream_text(&kind, &payload_json)?;
                Ok(StreamRow {
                    id: row.get(0)?,
                    seq: row.get(1)?,
                    kind,
                    text,
                })
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let merges = plan_stream_merges(rows)?;
        if merges.is_empty() {
            continue;
        }
        let transaction = connection.transaction()?;
        for merge in merges {
            transaction.execute(
                &format!("UPDATE {table} SET payload_json = ?2 WHERE id = ?1"),
                params![&merge.keep_id, &merge.payload_json],
            )?;
            if table == "events" {
                transaction.execute(
                    "UPDATE conversation_events SET payload_json = ?2 WHERE id = ?1",
                    params![&merge.keep_id, &merge.payload_json],
                )?;
            }
            for delete_id in merge.delete_ids {
                removed += transaction.execute(
                    &format!("DELETE FROM {table} WHERE id = ?1"),
                    params![delete_id],
                )? as u64;
            }
        }
        transaction.commit()?;
    }
    Ok(removed)
}

fn plan_stream_merges(rows: Vec<StreamRow>) -> anyhow::Result<Vec<StreamMerge>> {
    let mut merges = Vec::new();
    let mut pending: Option<(String, i64, String, String, Vec<String>)> = None;
    for row in rows {
        let Some(text) = row.text else {
            flush_stream_merge(&mut pending, &mut merges)?;
            continue;
        };
        let can_merge = pending
            .as_ref()
            .is_some_and(|(_, last_seq, kind, combined, _)| {
                *last_seq + 1 == row.seq
                    && *kind == row.kind
                    && combined.len() + text.len() <= COMPACTED_STREAM_CHUNK_BYTES
            });
        if can_merge {
            let (_, last_seq, _, combined, delete_ids) = pending.as_mut().unwrap();
            *last_seq = row.seq;
            combined.push_str(&text);
            delete_ids.push(row.id);
        } else {
            flush_stream_merge(&mut pending, &mut merges)?;
            pending = Some((row.id, row.seq, row.kind, text, Vec::new()));
        }
    }
    flush_stream_merge(&mut pending, &mut merges)?;
    Ok(merges)
}

fn flush_stream_merge(
    pending: &mut Option<(String, i64, String, String, Vec<String>)>,
    merges: &mut Vec<StreamMerge>,
) -> anyhow::Result<()> {
    let Some((keep_id, _, kind, text, delete_ids)) = pending.take() else {
        return Ok(());
    };
    if delete_ids.is_empty() {
        return Ok(());
    }
    let payload = match kind.as_str() {
        "model_delta" => AgentEventPayload::ModelDelta { text },
        "reasoning_delta" => AgentEventPayload::ReasoningDelta { text },
        _ => anyhow::bail!("unexpected stream event kind {kind}"),
    };
    merges.push(StreamMerge {
        keep_id,
        payload_json: serde_json::to_string(&payload)?,
        delete_ids,
    });
    Ok(())
}

fn prune_completed_conversation_streams(connection: &Connection) -> anyhow::Result<u64> {
    connection.execute_batch(
        r#"
        CREATE TEMP TABLE compact_completed_turns (
            thread_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            PRIMARY KEY(thread_id, turn_id)
        ) WITHOUT ROWID;
        INSERT INTO compact_completed_turns (thread_id, turn_id)
        SELECT DISTINCT thread_id, turn_id
        FROM events
        WHERE kind = 'assistant_message' AND turn_id IS NOT NULL;
        "#,
    )?;
    let removed = connection.execute(
        r#"
        DELETE FROM conversation_events
        WHERE id IN (
            SELECT stream.id
            FROM events AS stream
            INNER JOIN compact_completed_turns AS completed
                ON completed.thread_id = stream.thread_id
               AND completed.turn_id = stream.turn_id
            WHERE stream.kind = 'model_delta'
        )
        "#,
        [],
    )? as u64;
    connection.execute_batch("DROP TABLE compact_completed_turns;")?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::AgentEvent;
    use crate::store::{SessionStore, SqliteSessionStore};
    use uuid::Uuid;

    #[test]
    fn stream_merge_planning_preserves_boundaries_and_text() {
        let rows = vec![
            stream_row(1, "reasoning_delta", "one"),
            stream_row(2, "reasoning_delta", "two"),
            stream_row(4, "reasoning_delta", "gap"),
            stream_row(5, "model_delta", "answer"),
        ];
        let merges = plan_stream_merges(rows).expect("plan stream merges");
        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].delete_ids.len(), 1);
        let payload: AgentEventPayload =
            serde_json::from_str(&merges[0].payload_json).expect("parse merged payload");
        assert!(matches!(
            payload,
            AgentEventPayload::ReasoningDelta { text } if text == "onetwo"
        ));
    }

    #[test]
    fn legacy_non_stream_payloads_are_boundaries_without_deserialization() {
        assert_eq!(
            stream_text(
                "model_context_built",
                r#"{"type":"model_context_built","items":[{"missing":"directToolSchemas"}]}"#,
            )
            .expect("legacy semantic event is skipped"),
            None
        );
    }

    fn stream_row(seq: i64, kind: &str, text: &str) -> StreamRow {
        StreamRow {
            id: Uuid::new_v4().to_string(),
            seq,
            kind: kind.to_string(),
            text: Some(text.to_string()),
        }
    }

    #[test]
    fn compaction_creates_a_verified_copy_and_preserves_the_source() {
        let directory = std::env::temp_dir().join(format!("opentopia-compact-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create compaction fixture directory");
        let source = directory.join("source.db");
        let output = directory.join("compact.db");
        let archive = directory.join("traces.zip");
        let thread_id;
        {
            let store = SqliteSessionStore::open(&source).expect("create source database");
            let thread = store
                .create_thread(None, directory.clone())
                .expect("create source thread");
            thread_id = thread.id;
            let turn_id = Uuid::new_v4();
            store
                .append_events(vec![
                    AgentEvent::new(
                        thread.id,
                        Some(turn_id),
                        0,
                        AgentEventPayload::ReasoningDelta {
                            text: "one".to_string(),
                        },
                    ),
                    AgentEvent::new(
                        thread.id,
                        Some(turn_id),
                        0,
                        AgentEventPayload::ReasoningDelta {
                            text: "two".to_string(),
                        },
                    ),
                ])
                .expect("append source events");
        }
        let source_size = fs::metadata(&source).expect("source metadata").len();

        let report = compact_database_copy(&source, &output, &archive).expect("compact database");

        assert_eq!(
            fs::metadata(&source).expect("source metadata").len(),
            source_size
        );
        assert!(archive.is_file());
        assert_eq!(report.compacted_event_rows, 1);
        let compact = SqliteSessionStore::open(&output).expect("open compact database");
        let events = compact
            .list_events(thread_id, None)
            .expect("list compact events");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].payload,
            AgentEventPayload::ReasoningDelta { text } if text == "onetwo"
        ));
        drop(compact);
        let _ = fs::remove_dir_all(directory);
    }
}
