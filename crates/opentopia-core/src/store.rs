use crate::model::{AgentEvent, AgentEventPayload, Message, MessagePart, MessageRole, Thread};
use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub trait SessionStore: Send + Sync {
    fn create_thread(&self, title: Option<String>, workspace_root: PathBuf) -> anyhow::Result<Thread>;
    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>>;
    fn list_threads(&self) -> anyhow::Result<Vec<Thread>>;
    fn append_message(&self, message: Message) -> anyhow::Result<Message>;
    fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>>;
    fn append_event(&self, event: AgentEvent) -> anyhow::Result<AgentEvent>;
    fn list_events(&self, thread_id: Uuid, after_seq: Option<i64>) -> anyhow::Result<Vec<AgentEvent>>;
}

pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
}

impl SqliteSessionStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open sqlite db {}", path.display()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                workspace_root TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                role TEXT NOT NULL,
                parts_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                turn_id TEXT,
                seq INTEGER NOT NULL,
                kind TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(thread_id, seq),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_thread_created
                ON messages(thread_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_events_thread_seq
                ON events(thread_id, seq);
            "#,
        )?;
        Ok(())
    }
}

impl SessionStore for SqliteSessionStore {
    fn create_thread(&self, title: Option<String>, workspace_root: PathBuf) -> anyhow::Result<Thread> {
        let thread = Thread::new(title.unwrap_or_else(|| "Untitled thread".to_string()), workspace_root);
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO threads (id, title, workspace_root, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                thread.id.to_string(),
                &thread.title,
                thread.workspace_root.display().to_string(),
                thread.created_at.to_rfc3339(),
                thread.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(thread)
    }

    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let thread = conn
            .query_row(
                r#"
                SELECT id, title, workspace_root, created_at, updated_at
                FROM threads
                WHERE id = ?1
                "#,
                params![id.to_string()],
                map_thread,
            )
            .optional()?;
        Ok(thread)
    }

    fn list_threads(&self) -> anyhow::Result<Vec<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, title, workspace_root, created_at, updated_at
            FROM threads
            ORDER BY updated_at DESC
            "#,
        )?;
        let rows = stmt.query_map([], map_thread)?;
        collect_rows(rows)
    }

    fn append_message(&self, message: Message) -> anyhow::Result<Message> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let parts_json = serde_json::to_string(&message.parts)?;
        conn.execute(
            r#"
            INSERT INTO messages (id, thread_id, role, parts_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                message.id.to_string(),
                message.thread_id.to_string(),
                message.role.as_str(),
                parts_json,
                message.created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, message.thread_id)?;
        Ok(message)
    }

    fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, role, parts_json, created_at
            FROM messages
            WHERE thread_id = ?1
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_message)?;
        collect_rows(rows)
    }

    fn append_event(&self, mut event: AgentEvent) -> anyhow::Result<AgentEvent> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE thread_id = ?1",
            params![event.thread_id.to_string()],
            |row| row.get(0),
        )?;
        event.seq = seq;
        let payload_json = serde_json::to_string(&event.payload)?;
        conn.execute(
            r#"
            INSERT INTO events (id, thread_id, turn_id, seq, kind, payload_json, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                event.id.to_string(),
                event.thread_id.to_string(),
                event.turn_id.as_ref().map(|id| id.to_string()),
                event.seq,
                event.kind(),
                payload_json,
                event.created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, event.thread_id)?;
        Ok(event)
    }

    fn list_events(&self, thread_id: Uuid, after_seq: Option<i64>) -> anyhow::Result<Vec<AgentEvent>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, turn_id, seq, payload_json, created_at
            FROM events
            WHERE thread_id = ?1 AND seq > ?2
            ORDER BY seq ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string(), after_seq.unwrap_or(0)], map_event)?;
        collect_rows(rows)
    }
}

fn touch_thread(conn: &Connection, thread_id: Uuid) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE threads SET updated_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), thread_id.to_string()],
    )?;
    Ok(())
}

fn collect_rows<T, F>(rows: rusqlite::MappedRows<'_, F>) -> anyhow::Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut output = Vec::new();
    for row in rows {
        output.push(row?);
    }
    Ok(output)
}

fn map_thread(row: &rusqlite::Row<'_>) -> rusqlite::Result<Thread> {
    Ok(Thread {
        id: parse_uuid(row.get(0)?, 0)?,
        title: row.get(1)?,
        workspace_root: PathBuf::from(row.get::<_, String>(2)?),
        created_at: parse_datetime(row.get(3)?, 3)?,
        updated_at: parse_datetime(row.get(4)?, 4)?,
    })
}

fn map_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
    let parts_json: String = row.get(3)?;
    let parts: Vec<MessagePart> = serde_json::from_str(&parts_json)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err)))?;
    let role_value: String = row.get(2)?;
    let role = MessageRole::from_str(&role_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(Message {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        role,
        parts,
        created_at: parse_datetime(row.get(4)?, 4)?,
    })
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentEvent> {
    let turn_id: Option<String> = row.get(2)?;
    let payload_json: String = row.get(4)?;
    let payload: AgentEventPayload = serde_json::from_str(&payload_json)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(err)))?;
    Ok(AgentEvent {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        turn_id: turn_id
            .map(|value| parse_uuid(value, 2))
            .transpose()?,
        seq: row.get(3)?,
        payload,
        created_at: parse_datetime(row.get(5)?, 5)?,
    })
}

fn parse_uuid(value: String, column: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

fn parse_datetime(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}
