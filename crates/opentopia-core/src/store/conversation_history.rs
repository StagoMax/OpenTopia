use super::{
    map_event, map_message, map_turn, AgentEvent, Message, SqliteSessionStore, TurnRecord,
};
use chrono::{DateTime, Utc};
use rusqlite::params;
use uuid::Uuid;

const MAX_CONVERSATION_PAGE_SIZE: usize = 1_000;

impl SqliteSessionStore {
    /// Reads one stable, renderable conversation-message page without making
    /// callers deserialize the immutable prefix of a long conversation on
    /// every visit. Tool messages are represented by the event timeline and
    /// must not crowd user/assistant messages out of the visible page.
    pub fn list_conversation_message_page(
        &self,
        thread_id: Uuid,
        after: Option<(DateTime<Utc>, Uuid)>,
        before: Option<(DateTime<Utc>, Uuid)>,
        limit: usize,
    ) -> anyhow::Result<Vec<Message>> {
        let limit = bounded_limit(limit);
        let conn = self.read_connection();

        if let Some((created_at, id)) = after {
            let mut statement = conn.prepare(
                r#"
                SELECT id, thread_id, role, parts_json, created_at
                FROM messages
                WHERE thread_id = ?1
                  AND role IN ('user', 'assistant')
                  AND (created_at > ?2 OR (created_at = ?2 AND id > ?3))
                ORDER BY created_at ASC, id ASC
                LIMIT ?4
                "#,
            )?;
            let rows = statement.query_map(
                params![
                    thread_id.to_string(),
                    created_at.to_rfc3339(),
                    id.to_string(),
                    limit,
                ],
                map_message,
            )?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
        }

        if let Some((created_at, id)) = before {
            let mut statement = conn.prepare(
                r#"
                SELECT id, thread_id, role, parts_json, created_at
                FROM (
                    SELECT id, thread_id, role, parts_json, created_at
                    FROM messages
                    WHERE thread_id = ?1
                      AND role IN ('user', 'assistant')
                      AND (created_at < ?2 OR (created_at = ?2 AND id < ?3))
                    ORDER BY created_at DESC, id DESC
                    LIMIT ?4
                )
                ORDER BY created_at ASC, id ASC
                "#,
            )?;
            let rows = statement.query_map(
                params![
                    thread_id.to_string(),
                    created_at.to_rfc3339(),
                    id.to_string(),
                    limit,
                ],
                map_message,
            )?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
        }

        let mut statement = conn.prepare(
            r#"
            SELECT id, thread_id, role, parts_json, created_at
            FROM (
                SELECT id, thread_id, role, parts_json, created_at
                FROM messages
                WHERE thread_id = ?1
                  AND role IN ('user', 'assistant')
                ORDER BY created_at DESC, id DESC
                LIMIT ?2
            )
            ORDER BY created_at ASC, id ASC
            "#,
        )?;
        let rows = statement.query_map(params![thread_id.to_string(), limit], map_message)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Pages the compact conversation projection. `after_seq` is used for
    /// forward catch-up; `before_seq` is used only when older messages are
    /// explicitly requested.
    pub fn list_conversation_event_page(
        &self,
        thread_id: Uuid,
        after_seq: Option<i64>,
        before_seq: Option<i64>,
        limit: usize,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let limit = bounded_limit(limit);
        let conn = self.read_connection();

        if let Some(after_seq) = after_seq {
            let mut statement = conn.prepare(
                r#"
                SELECT id, thread_id, turn_id, seq, payload_json, created_at
                FROM conversation_events
                WHERE thread_id = ?1 AND seq > ?2
                ORDER BY seq ASC
                LIMIT ?3
                "#,
            )?;
            let rows =
                statement.query_map(params![thread_id.to_string(), after_seq, limit], map_event)?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
        }

        if let Some(before_seq) = before_seq {
            let mut statement = conn.prepare(
                r#"
                SELECT id, thread_id, turn_id, seq, payload_json, created_at
                FROM (
                    SELECT id, thread_id, turn_id, seq, payload_json, created_at
                    FROM conversation_events
                    WHERE thread_id = ?1 AND seq < ?2
                    ORDER BY seq DESC
                    LIMIT ?3
                )
                ORDER BY seq ASC
                "#,
            )?;
            let rows = statement
                .query_map(params![thread_id.to_string(), before_seq, limit], map_event)?;
            return rows.collect::<Result<Vec<_>, _>>().map_err(Into::into);
        }

        let mut statement = conn.prepare(
            r#"
            SELECT id, thread_id, turn_id, seq, payload_json, created_at
            FROM (
                SELECT id, thread_id, turn_id, seq, payload_json, created_at
                FROM conversation_events
                WHERE thread_id = ?1
                ORDER BY seq DESC
                LIMIT ?2
            )
            ORDER BY seq ASC
            "#,
        )?;
        let rows = statement.query_map(params![thread_id.to_string(), limit], map_event)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// One process-wide snapshot replaces the renderer's previous N+1 status
    /// reconciliation. Terminal turns are intentionally excluded: absence
    /// from this snapshot means an old live indicator can be cleared.
    pub fn list_live_turn_statuses(&self) -> anyhow::Result<Vec<TurnRecord>> {
        let conn = self.read_connection();
        let mut statement = conn.prepare(
            r#"
            SELECT turn_id, invocation_id, thread_id, user_message_id, status,
                   started_at, updated_at, completed_at, error
            FROM turns
            WHERE status IN (
                'running', 'cancelling', 'waiting_approval',
                'waiting_user_input', 'waiting_user_action'
            )
            ORDER BY started_at ASC, rowid ASC
            "#,
        )?;
        let rows = statement.query_map([], map_turn)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn bounded_limit(limit: usize) -> i64 {
    limit.clamp(1, MAX_CONVERSATION_PAGE_SIZE) as i64
}
