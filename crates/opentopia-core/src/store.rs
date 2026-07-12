use crate::mcp::{McpServerConfig, ThreadMcpServer};
use crate::model::{
    AgentEvent, AgentEventPayload, Approval, ApprovalStatus, Artifact, ArtifactMetadata,
    ArtifactStorage, ArtifactStorageMetadata, Message, MessagePart, MessageRole,
    TerminalCommandHistory, TerminalCommandStatus, Thread, ToolResult,
};
use crate::settings::AppSettings;
use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub trait SessionStore: Send + Sync + std::fmt::Debug {
    fn create_thread(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Thread>;
    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>>;
    fn list_threads(&self) -> anyhow::Result<Vec<Thread>>;
    fn append_message(&self, message: Message) -> anyhow::Result<Message>;
    fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>>;
    fn append_event(&self, event: AgentEvent) -> anyhow::Result<AgentEvent>;
    fn list_events(
        &self,
        thread_id: Uuid,
        after_seq: Option<i64>,
    ) -> anyhow::Result<Vec<AgentEvent>>;
    fn insert_terminal_history(
        &self,
        history: TerminalCommandHistory,
    ) -> anyhow::Result<TerminalCommandHistory>;
    fn list_terminal_history(
        &self,
        thread_id: Uuid,
        after_seq: Option<u64>,
    ) -> anyhow::Result<Vec<TerminalCommandHistory>>;
    fn latest_terminal_history_seq(&self, thread_id: Uuid) -> anyhow::Result<u64>;
    fn insert_artifact(&self, artifact: Artifact) -> anyhow::Result<Artifact>;
    fn list_artifacts(&self, thread_id: Uuid) -> anyhow::Result<Vec<ArtifactMetadata>>;
    fn get_artifact(&self, thread_id: Uuid, artifact_id: Uuid) -> anyhow::Result<Option<Artifact>>;
    fn insert_approval(&self, approval: Approval) -> anyhow::Result<Approval>;
    fn get_approval(&self, approval_id: Uuid) -> anyhow::Result<Option<Approval>>;
    fn list_approvals(
        &self,
        thread_id: Uuid,
        status: Option<ApprovalStatus>,
    ) -> anyhow::Result<Vec<Approval>>;
    fn update_approval_status(
        &self,
        approval_id: Uuid,
        status: ApprovalStatus,
    ) -> anyhow::Result<Option<Approval>>;
    fn put_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
        continuation: Value,
    ) -> anyhow::Result<()>;
    fn get_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>>;
    fn delete_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<()>;

    fn insert_large_tool_output_artifact(
        &self,
        thread_id: Uuid,
        result: &ToolResult,
        threshold_bytes: usize,
    ) -> anyhow::Result<Option<Artifact>> {
        let output_bytes = result.output.len();
        if output_bytes <= threshold_bytes {
            return Ok(None);
        }
        let artifact = Artifact::inline(
            thread_id,
            "tool_output",
            "text/plain; charset=utf-8",
            result.output.clone(),
            serde_json::json!({
                "source": "tool_result",
                "callId": result.call_id,
                "outputBytes": output_bytes,
                "thresholdBytes": threshold_bytes,
                "toolResultMetadata": result.metadata.clone(),
            }),
        );
        self.insert_artifact(artifact).map(Some)
    }

    fn get_context_budget(&self, thread_id: Uuid) -> anyhow::Result<ContextBudget> {
        let messages = self.list_messages(thread_id)?;
        let message_count = messages.len();
        let total_tokens = std::env::var("OPENTOPIA_CONTEXT_WINDOW_TOKENS")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value: &usize| *value >= 4_096)
            .unwrap_or(128_000);
        let mut used_tokens: usize = 0;
        for msg in &messages {
            let text_len: usize = msg
                .parts
                .iter()
                .map(|part| match part {
                    MessagePart::Text { text } => text.len(),
                    MessagePart::ToolResult { result } => result.output.len(),
                    MessagePart::ToolCall { .. } => 100,
                    _ => 50,
                })
                .sum();
            let tokens = (text_len + 3) / 4 + 50;
            used_tokens += tokens;
        }
        let estimated_usage = if total_tokens > 0 {
            (used_tokens * 100) / total_tokens
        } else {
            0
        };
        Ok(ContextBudget {
            total_tokens,
            used_tokens,
            message_count,
            estimated_usage,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudget {
    pub total_tokens: usize,
    pub used_tokens: usize,
    pub message_count: usize,
    pub estimated_usage: usize,
}

#[derive(Debug)]
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

            CREATE TABLE IF NOT EXISTS terminal_history (
                command_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                seq_start INTEGER NOT NULL,
                seq_end INTEGER NOT NULL,
                command TEXT NOT NULL,
                cwd TEXT,
                stdout TEXT NOT NULL,
                stderr TEXT NOT NULL,
                exit_code INTEGER,
                status TEXT NOT NULL,
                message TEXT,
                started_at TEXT NOT NULL,
                completed_at TEXT NOT NULL,
                CHECK(status IN ('finished', 'failed', 'cancelled', 'timed_out', 'error')),
                CHECK(seq_end >= seq_start),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                content_type TEXT NOT NULL,
                storage_kind TEXT NOT NULL,
                path TEXT,
                inline_content TEXT,
                bytes INTEGER NOT NULL,
                metadata_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                CHECK(storage_kind IN ('inline', 'path')),
                CHECK(
                    (storage_kind = 'inline' AND inline_content IS NOT NULL AND path IS NULL) OR
                    (storage_kind = 'path' AND path IS NOT NULL AND inline_content IS NULL)
                ),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS approvals (
                approval_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                action TEXT NOT NULL,
                reason TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                decided_at TEXT,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS approval_continuations (
                approval_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                continuation_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(approval_id) REFERENCES approvals(approval_id) ON DELETE CASCADE,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS app_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                settings_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS mcp_servers (
                server_id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                command TEXT NOT NULL,
                args_json TEXT NOT NULL,
                cwd TEXT,
                env_keys_json TEXT NOT NULL,
                timeout_ms INTEGER NOT NULL,
                enabled INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS thread_mcp_servers (
                thread_id TEXT NOT NULL,
                server_id TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(thread_id, server_id),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
                FOREIGN KEY(server_id) REFERENCES mcp_servers(server_id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_thread_created
                ON messages(thread_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_events_thread_seq
                ON events(thread_id, seq);

            CREATE INDEX IF NOT EXISTS idx_terminal_history_thread_seq
                ON terminal_history(thread_id, seq_start, seq_end);

            CREATE INDEX IF NOT EXISTS idx_terminal_history_thread_completed
                ON terminal_history(thread_id, completed_at);

            CREATE INDEX IF NOT EXISTS idx_artifacts_thread_created
                ON artifacts(thread_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_artifacts_thread_kind_created
                ON artifacts(thread_id, kind, created_at);

            CREATE INDEX IF NOT EXISTS idx_approval_continuations_thread
                ON approval_continuations(thread_id);

            CREATE INDEX IF NOT EXISTS idx_approvals_thread_status_created
                ON approvals(thread_id, status, created_at);

            CREATE INDEX IF NOT EXISTS idx_thread_mcp_servers_thread
                ON thread_mcp_servers(thread_id, updated_at);
            "#,
        )?;
        Ok(())
    }

    pub fn load_settings(
        &self,
        default_permission_mode: crate::policy::PermissionMode,
    ) -> anyhow::Result<AppSettings> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let settings_json: Option<String> = conn
            .query_row(
                "SELECT settings_json FROM app_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match settings_json {
            Some(settings_json) => {
                let mut settings: AppSettings = serde_json::from_str(&settings_json)?;
                if settings.providers.is_empty() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&settings_json) {
                        if let Some(provider) = value.get("provider") {
                            if let Ok(p) = serde_json::from_value(provider.clone()) {
                                settings.providers = vec![p];
                            }
                        }
                    }
                    if settings.active_provider_id.is_empty() {
                        settings.active_provider_id = settings
                            .providers
                            .first()
                            .map(|p| p.id.clone())
                            .unwrap_or_default();
                    }
                }
                settings.touch();
                Ok(settings)
            }
            None => Ok(AppSettings::from_env(default_permission_mode)),
        }
    }

    pub fn save_settings(&self, mut settings: AppSettings) -> anyhow::Result<AppSettings> {
        settings.touch();
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO app_settings (id, settings_json, updated_at)
            VALUES (1, ?1, ?2)
            ON CONFLICT(id) DO UPDATE SET
                settings_json = excluded.settings_json,
                updated_at = excluded.updated_at
            "#,
            params![
                serde_json::to_string(&settings)?,
                settings.updated_at.to_rfc3339()
            ],
        )?;
        Ok(settings)
    }

    pub fn list_mcp_servers(&self) -> anyhow::Result<Vec<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT server_id, name, command, args_json, cwd, env_keys_json,
                   timeout_ms, enabled, created_at, updated_at
            FROM mcp_servers
            ORDER BY name ASC
            "#,
        )?;
        let rows = stmt.query_map([], map_mcp_server)?;
        collect_rows(rows)
    }

    pub fn get_mcp_server(&self, server_id: Uuid) -> anyhow::Result<Option<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT server_id, name, command, args_json, cwd, env_keys_json,
                   timeout_ms, enabled, created_at, updated_at
            FROM mcp_servers
            WHERE server_id = ?1
            "#,
            params![server_id.to_string()],
            map_mcp_server,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn insert_mcp_server(&self, config: McpServerConfig) -> anyhow::Result<McpServerConfig> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO mcp_servers (
                server_id, name, command, args_json, cwd, env_keys_json,
                timeout_ms, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                config.server_id.to_string(),
                &config.name,
                &config.command,
                serde_json::to_string(&config.args)?,
                config.cwd.as_ref().map(|path| path.display().to_string()),
                serde_json::to_string(&config.env_keys)?,
                config.timeout_ms as i64,
                config.enabled as i64,
                config.created_at.to_rfc3339(),
                config.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(config)
    }

    pub fn update_mcp_server(
        &self,
        config: McpServerConfig,
    ) -> anyhow::Result<Option<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let updated = conn.execute(
            r#"
            UPDATE mcp_servers
            SET name = ?1,
                command = ?2,
                args_json = ?3,
                cwd = ?4,
                env_keys_json = ?5,
                timeout_ms = ?6,
                enabled = ?7,
                updated_at = ?8
            WHERE server_id = ?9
            "#,
            params![
                &config.name,
                &config.command,
                serde_json::to_string(&config.args)?,
                config.cwd.as_ref().map(|path| path.display().to_string()),
                serde_json::to_string(&config.env_keys)?,
                config.timeout_ms as i64,
                config.enabled as i64,
                config.updated_at.to_rfc3339(),
                config.server_id.to_string(),
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        Ok(Some(config))
    }

    pub fn delete_mcp_server(&self, server_id: Uuid) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let deleted = conn.execute(
            "DELETE FROM mcp_servers WHERE server_id = ?1",
            params![server_id.to_string()],
        )?;
        Ok(deleted > 0)
    }

    pub fn list_thread_mcp_servers(&self, thread_id: Uuid) -> anyhow::Result<Vec<ThreadMcpServer>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT thread_id, server_id, enabled, updated_at
            FROM thread_mcp_servers
            WHERE thread_id = ?1
            ORDER BY updated_at DESC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_thread_mcp_server)?;
        collect_rows(rows)
    }

    pub fn set_thread_mcp_server(
        &self,
        thread_id: Uuid,
        server_id: Uuid,
        enabled: bool,
    ) -> anyhow::Result<ThreadMcpServer> {
        let binding = ThreadMcpServer {
            thread_id,
            server_id,
            enabled,
            updated_at: Utc::now(),
        };
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO thread_mcp_servers (thread_id, server_id, enabled, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(thread_id, server_id) DO UPDATE SET
                enabled = excluded.enabled,
                updated_at = excluded.updated_at
            "#,
            params![
                binding.thread_id.to_string(),
                binding.server_id.to_string(),
                binding.enabled as i64,
                binding.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(binding)
    }
}

impl SessionStore for SqliteSessionStore {
    fn create_thread(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Thread> {
        let thread = Thread::new(
            title.unwrap_or_else(|| "Untitled thread".to_string()),
            workspace_root,
        );
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

    fn list_events(
        &self,
        thread_id: Uuid,
        after_seq: Option<i64>,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, turn_id, seq, payload_json, created_at
            FROM events
            WHERE thread_id = ?1 AND seq > ?2
            ORDER BY seq ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string(), after_seq.unwrap_or(0)],
            map_event,
        )?;
        collect_rows(rows)
    }

    fn insert_terminal_history(
        &self,
        history: TerminalCommandHistory,
    ) -> anyhow::Result<TerminalCommandHistory> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let seq_start = i64::try_from(history.seq_start)
            .context("terminal seq_start exceeds sqlite INTEGER")?;
        let seq_end =
            i64::try_from(history.seq_end).context("terminal seq_end exceeds sqlite INTEGER")?;
        conn.execute(
            r#"
            INSERT INTO terminal_history (
                command_id, thread_id, seq_start, seq_end, command, cwd, stdout,
                stderr, exit_code, status, message, started_at, completed_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(command_id) DO UPDATE SET
                seq_start = excluded.seq_start,
                seq_end = excluded.seq_end,
                command = excluded.command,
                cwd = excluded.cwd,
                stdout = excluded.stdout,
                stderr = excluded.stderr,
                exit_code = excluded.exit_code,
                status = excluded.status,
                message = excluded.message,
                started_at = excluded.started_at,
                completed_at = excluded.completed_at
            "#,
            params![
                history.command_id.to_string(),
                history.thread_id.to_string(),
                seq_start,
                seq_end,
                &history.command,
                history.cwd.as_ref().map(|path| path.display().to_string()),
                &history.stdout,
                &history.stderr,
                history.exit_code,
                history.status.as_str(),
                &history.message,
                history.started_at.to_rfc3339(),
                history.completed_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, history.thread_id)?;
        Ok(history)
    }

    fn list_terminal_history(
        &self,
        thread_id: Uuid,
        after_seq: Option<u64>,
    ) -> anyhow::Result<Vec<TerminalCommandHistory>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let after_seq = i64::try_from(after_seq.unwrap_or(0))
            .context("terminal after_seq exceeds sqlite INTEGER")?;
        let mut stmt = conn.prepare(
            r#"
            SELECT command_id, thread_id, seq_start, seq_end, command, cwd, stdout,
                   stderr, exit_code, status, message, started_at, completed_at
            FROM terminal_history
            WHERE thread_id = ?1 AND seq_end > ?2
            ORDER BY seq_start ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string(), after_seq],
            map_terminal_history,
        )?;
        collect_rows(rows)
    }

    fn latest_terminal_history_seq(&self, thread_id: Uuid) -> anyhow::Result<u64> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq_end), 0) FROM terminal_history WHERE thread_id = ?1",
            params![thread_id.to_string()],
            |row| row.get(0),
        )?;
        parse_u64(seq, 0).map_err(Into::into)
    }

    fn insert_artifact(&self, artifact: Artifact) -> anyhow::Result<Artifact> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let (storage_kind, path, inline_content) = match &artifact.storage {
            ArtifactStorage::Inline { content } => ("inline", None, Some(content.as_str())),
            ArtifactStorage::Path { path } => ("path", Some(path.display().to_string()), None),
        };
        let metadata_json = serde_json::to_string(&artifact.metadata)?;
        let bytes =
            i64::try_from(artifact.bytes).context("artifact bytes exceed sqlite INTEGER")?;
        conn.execute(
            r#"
            INSERT INTO artifacts (
                id, thread_id, kind, content_type, storage_kind, path, inline_content,
                bytes, metadata_json, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                artifact.id.to_string(),
                artifact.thread_id.to_string(),
                &artifact.kind,
                &artifact.content_type,
                storage_kind,
                path,
                inline_content,
                bytes,
                metadata_json,
                artifact.created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, artifact.thread_id)?;
        Ok(artifact)
    }

    fn list_artifacts(&self, thread_id: Uuid) -> anyhow::Result<Vec<ArtifactMetadata>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, kind, content_type, storage_kind, path,
                   bytes, metadata_json, created_at
            FROM artifacts
            WHERE thread_id = ?1
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_artifact_metadata)?;
        collect_rows(rows)
    }

    fn get_artifact(&self, thread_id: Uuid, artifact_id: Uuid) -> anyhow::Result<Option<Artifact>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let artifact = conn
            .query_row(
                r#"
                SELECT id, thread_id, kind, content_type, storage_kind, path, inline_content,
                       bytes, metadata_json, created_at
                FROM artifacts
                WHERE thread_id = ?1 AND id = ?2
                "#,
                params![thread_id.to_string(), artifact_id.to_string()],
                map_artifact,
            )
            .optional()?;
        Ok(artifact)
    }

    fn insert_approval(&self, approval: Approval) -> anyhow::Result<Approval> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO approvals (
                approval_id, thread_id, action, reason, status, created_at, decided_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                approval.approval_id.to_string(),
                approval.thread_id.to_string(),
                &approval.action,
                &approval.reason,
                approval.status.as_str(),
                approval.created_at.to_rfc3339(),
                approval.decided_at.as_ref().map(DateTime::to_rfc3339),
            ],
        )?;
        touch_thread(&conn, approval.thread_id)?;
        Ok(approval)
    }

    fn get_approval(&self, approval_id: Uuid) -> anyhow::Result<Option<Approval>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let approval = conn
            .query_row(
                r#"
                SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
                FROM approvals
                WHERE approval_id = ?1
                "#,
                params![approval_id.to_string()],
                map_approval,
            )
            .optional()?;
        Ok(approval)
    }

    fn list_approvals(
        &self,
        thread_id: Uuid,
        status: Option<ApprovalStatus>,
    ) -> anyhow::Result<Vec<Approval>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        if let Some(status) = status {
            let mut stmt = conn.prepare(
                r#"
                SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
                FROM approvals
                WHERE thread_id = ?1 AND status = ?2
                ORDER BY created_at ASC
                "#,
            )?;
            let rows = stmt.query_map(
                params![thread_id.to_string(), status.as_str()],
                map_approval,
            )?;
            collect_rows(rows)
        } else {
            let mut stmt = conn.prepare(
                r#"
                SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
                FROM approvals
                WHERE thread_id = ?1
                ORDER BY created_at ASC
                "#,
            )?;
            let rows = stmt.query_map(params![thread_id.to_string()], map_approval)?;
            collect_rows(rows)
        }
    }

    fn update_approval_status(
        &self,
        approval_id: Uuid,
        status: ApprovalStatus,
    ) -> anyhow::Result<Option<Approval>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let decided_at = match status {
            ApprovalStatus::Pending => None,
            ApprovalStatus::Approved | ApprovalStatus::Denied => Some(Utc::now()),
        };
        let updated = conn.execute(
            r#"
            UPDATE approvals
            SET status = ?1, decided_at = ?2
            WHERE approval_id = ?3 AND status = ?4
            "#,
            params![
                status.as_str(),
                decided_at.as_ref().map(DateTime::to_rfc3339),
                approval_id.to_string(),
                ApprovalStatus::Pending.as_str(),
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        let approval = conn.query_row(
            r#"
            SELECT approval_id, thread_id, action, reason, status, created_at, decided_at
            FROM approvals
            WHERE approval_id = ?1
            "#,
            params![approval_id.to_string()],
            map_approval,
        )?;
        touch_thread(&conn, approval.thread_id)?;
        Ok(Some(approval))
    }

    fn put_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
        continuation: Value,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO approval_continuations
                (approval_id, thread_id, continuation_json, created_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(approval_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                continuation_json = excluded.continuation_json,
                created_at = excluded.created_at
            "#,
            params![
                approval_id.to_string(),
                thread_id.to_string(),
                serde_json::to_string(&continuation)?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn get_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let continuation = conn
            .query_row(
                r#"
                SELECT continuation_json
                FROM approval_continuations
                WHERE approval_id = ?1 AND thread_id = ?2
                "#,
                params![approval_id.to_string(), thread_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        continuation
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    fn delete_approval_continuation(
        &self,
        approval_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            "DELETE FROM approval_continuations WHERE approval_id = ?1 AND thread_id = ?2",
            params![approval_id.to_string(), thread_id.to_string()],
        )?;
        Ok(())
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
        turn_id: turn_id.map(|value| parse_uuid(value, 2)).transpose()?,
        seq: row.get(3)?,
        payload,
        created_at: parse_datetime(row.get(5)?, 5)?,
    })
}

fn map_terminal_history(row: &rusqlite::Row<'_>) -> rusqlite::Result<TerminalCommandHistory> {
    let cwd: Option<String> = row.get(5)?;
    let status_value: String = row.get(9)?;
    let status = TerminalCommandStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            9,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(TerminalCommandHistory {
        command_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        seq_start: parse_u64(row.get(2)?, 2)?,
        seq_end: parse_u64(row.get(3)?, 3)?,
        command: row.get(4)?,
        cwd: cwd.map(PathBuf::from),
        stdout: row.get(6)?,
        stderr: row.get(7)?,
        exit_code: row.get(8)?,
        status,
        message: row.get(10)?,
        started_at: parse_datetime(row.get(11)?, 11)?,
        completed_at: parse_datetime(row.get(12)?, 12)?,
    })
}

fn map_artifact_metadata(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactMetadata> {
    let storage_kind: String = row.get(4)?;
    let path: Option<String> = row.get(5)?;
    let metadata_json: String = row.get(7)?;
    Ok(ArtifactMetadata {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        kind: row.get(2)?,
        content_type: row.get(3)?,
        storage: parse_artifact_storage_metadata(&storage_kind, path, 4)?,
        bytes: parse_u64(row.get(6)?, 6)?,
        metadata: serde_json::from_str(&metadata_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(err))
        })?,
        created_at: parse_datetime(row.get(8)?, 8)?,
    })
}

fn map_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    let storage_kind: String = row.get(4)?;
    let path: Option<String> = row.get(5)?;
    let inline_content: Option<String> = row.get(6)?;
    let metadata_json: String = row.get(8)?;
    Ok(Artifact {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        kind: row.get(2)?,
        content_type: row.get(3)?,
        storage: parse_artifact_storage(&storage_kind, path, inline_content, 4)?,
        bytes: parse_u64(row.get(7)?, 7)?,
        metadata: serde_json::from_str(&metadata_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(err))
        })?,
        created_at: parse_datetime(row.get(9)?, 9)?,
    })
}

fn map_approval(row: &rusqlite::Row<'_>) -> rusqlite::Result<Approval> {
    let status_value: String = row.get(4)?;
    let status = ApprovalStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let decided_at: Option<String> = row.get(6)?;
    Ok(Approval {
        approval_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        action: row.get(2)?,
        reason: row.get(3)?,
        status,
        created_at: parse_datetime(row.get(5)?, 5)?,
        decided_at: decided_at
            .map(|value| parse_datetime(value, 6))
            .transpose()?,
    })
}

fn map_mcp_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<McpServerConfig> {
    let args_json: String = row.get(3)?;
    let env_keys_json: String = row.get(5)?;
    let cwd: Option<String> = row.get(4)?;
    Ok(McpServerConfig {
        server_id: parse_uuid(row.get(0)?, 0)?,
        name: row.get(1)?,
        command: row.get(2)?,
        args: serde_json::from_str(&args_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(err))
        })?,
        cwd: cwd.map(PathBuf::from),
        env_keys: serde_json::from_str(&env_keys_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(err))
        })?,
        timeout_ms: row.get::<_, i64>(6)? as u64,
        enabled: row.get::<_, i64>(7)? != 0,
        created_at: parse_datetime(row.get(8)?, 8)?,
        updated_at: parse_datetime(row.get(9)?, 9)?,
    })
}

fn map_thread_mcp_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadMcpServer> {
    Ok(ThreadMcpServer {
        thread_id: parse_uuid(row.get(0)?, 0)?,
        server_id: parse_uuid(row.get(1)?, 1)?,
        enabled: row.get::<_, i64>(2)? != 0,
        updated_at: parse_datetime(row.get(3)?, 3)?,
    })
}

fn parse_uuid(value: String, column: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

fn parse_artifact_storage_metadata(
    storage_kind: &str,
    path: Option<String>,
    column: usize,
) -> rusqlite::Result<ArtifactStorageMetadata> {
    match storage_kind {
        "inline" => Ok(ArtifactStorageMetadata::Inline),
        "path" => path
            .map(|path| ArtifactStorageMetadata::Path {
                path: PathBuf::from(path),
            })
            .ok_or_else(|| invalid_column(column, "artifact path storage missing path")),
        other => Err(invalid_column(
            column,
            format!("unknown artifact storage kind: {other}"),
        )),
    }
}

fn parse_artifact_storage(
    storage_kind: &str,
    path: Option<String>,
    inline_content: Option<String>,
    column: usize,
) -> rusqlite::Result<ArtifactStorage> {
    match storage_kind {
        "inline" => inline_content
            .map(|content| ArtifactStorage::Inline { content })
            .ok_or_else(|| invalid_column(column, "inline artifact missing content")),
        "path" => path
            .map(|path| ArtifactStorage::Path {
                path: PathBuf::from(path),
            })
            .ok_or_else(|| invalid_column(column, "path artifact missing path")),
        other => Err(invalid_column(
            column,
            format!("unknown artifact storage kind: {other}"),
        )),
    }
}

fn parse_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Integer, Box::new(err))
    })
}

fn parse_datetime(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

fn invalid_column(column: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.into(),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_store_persists_terminal_history() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(Some("terminal".to_string()), PathBuf::from("."))
            .expect("create thread");
        let now = Utc::now();
        let history = TerminalCommandHistory {
            command_id: Uuid::new_v4(),
            thread_id: thread.id,
            seq_start: 10,
            seq_end: 13,
            command: "echo hello".to_string(),
            cwd: Some(PathBuf::from("J:\\Project\\OpenTopia")),
            stdout: "hello\n".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            status: TerminalCommandStatus::Finished,
            message: None,
            started_at: now,
            completed_at: now,
        };

        store
            .insert_terminal_history(history.clone())
            .expect("insert terminal history");

        let rows = store
            .list_terminal_history(thread.id, None)
            .expect("list terminal history");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].command_id, history.command_id);
        assert_eq!(rows[0].stdout, "hello\n");
        assert_eq!(rows[0].status, TerminalCommandStatus::Finished);
        assert_eq!(
            store
                .latest_terminal_history_seq(thread.id)
                .expect("latest seq"),
            13
        );

        let rows = store
            .list_terminal_history(thread.id, Some(12))
            .expect("list terminal history after seq");
        assert_eq!(rows.len(), 1);
        let rows = store
            .list_terminal_history(thread.id, Some(13))
            .expect("list terminal history after final seq");
        assert!(rows.is_empty());
    }
}
