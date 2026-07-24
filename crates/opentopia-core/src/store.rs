use crate::mcp::{McpServerConfig, ThreadMcpServer};
use crate::model::{
    AgentEvent, AgentEventPayload, Approval, ApprovalStatus, Artifact, ArtifactMetadata,
    ArtifactStorage, ArtifactStorageMetadata, ExperienceMode, GoalAttemptStatus, GoalRecord,
    GoalSnapshot, GoalStatus, GoalTask, GoalTaskAttempt, GoalTaskStatus, Message, MessagePart,
    MessageRole, Project, TaskPlan, TerminalCommandHistory, TerminalCommandStatus, Thread,
    ToolResult, TurnChangeSet, TurnChangeSetStatus, TurnRecord, TurnStatus, UserInputRecord,
    UserInputRequest, UserInputResponse, UserInputStatus,
};
use crate::provider::ModelConversationMessage;
use crate::settings::AppSettings;
use crate::subagents::{SubagentRun, SubagentRunStatus};
use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

pub trait SessionStore: Send + Sync + std::fmt::Debug {
    fn create_project(
        &self,
        name: String,
        workspace_root: Option<PathBuf>,
        pinned: bool,
        sort_order: i64,
    ) -> anyhow::Result<Project>;
    fn get_project(&self, id: Uuid) -> anyhow::Result<Option<Project>>;
    fn find_project_by_workspace(&self, workspace_root: &Path) -> anyhow::Result<Option<Project>>;
    fn find_or_create_project(
        &self,
        name: String,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Project>;
    fn list_projects(&self) -> anyhow::Result<Vec<Project>>;
    fn update_project(
        &self,
        id: Uuid,
        name: Option<String>,
        workspace_root: Option<Option<PathBuf>>,
        pinned: Option<bool>,
        sort_order: Option<i64>,
    ) -> anyhow::Result<Option<Project>>;
    fn delete_project(&self, id: Uuid) -> anyhow::Result<bool>;
    fn create_thread(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Thread>;
    fn create_thread_with_mode(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread>;
    fn create_thread_in_project(
        &self,
        title: Option<String>,
        project_id: Uuid,
    ) -> anyhow::Result<Thread>;
    fn create_thread_in_project_with_mode(
        &self,
        title: Option<String>,
        project_id: Uuid,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread>;
    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>>;
    fn list_threads(&self) -> anyhow::Result<Vec<Thread>>;
    fn list_threads_including_archived(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<Thread>>;
    fn update_thread(
        &self,
        id: Uuid,
        title: Option<String>,
        project_id: Option<Option<Uuid>>,
        archived: Option<bool>,
    ) -> anyhow::Result<Option<Thread>>;
    fn delete_thread(&self, id: Uuid) -> anyhow::Result<bool>;
    fn create_goal(
        &self,
        thread_id: Uuid,
        objective: String,
        status: GoalStatus,
        token_budget: Option<u64>,
    ) -> anyhow::Result<GoalSnapshot>;
    fn get_goal(&self, id: Uuid) -> anyhow::Result<Option<GoalSnapshot>>;
    fn get_thread_goal(&self, thread_id: Uuid) -> anyhow::Result<Option<GoalSnapshot>>;
    fn update_goal_status(
        &self,
        thread_id: Uuid,
        goal_id: Uuid,
        status: GoalStatus,
    ) -> anyhow::Result<Option<GoalSnapshot>>;
    fn apply_goal_plan(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        plan: &TaskPlan,
    ) -> anyhow::Result<GoalSnapshot>;
    fn add_goal_usage(
        &self,
        goal_id: Uuid,
        tokens: u64,
        elapsed_seconds: u64,
    ) -> anyhow::Result<Option<GoalSnapshot>>;
    fn append_message(&self, message: Message) -> anyhow::Result<Message>;
    fn list_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Message>>;
    fn enqueue_turn_message(&self, thread_id: Uuid, message_id: Uuid) -> anyhow::Result<()>;
    fn list_queued_turn_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Uuid>>;
    fn remove_queued_turn_message(&self, thread_id: Uuid, message_id: Uuid)
        -> anyhow::Result<bool>;
    fn insert_turn(&self, turn: TurnRecord) -> anyhow::Result<TurnRecord>;
    fn get_turn(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn get_active_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn get_latest_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>>;
    fn update_turn_status(
        &self,
        turn_id: Uuid,
        status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>>;
    fn interrupt_active_turns(&self) -> anyhow::Result<usize>;
    fn upsert_turn_change_set(&self, change_set: &TurnChangeSet) -> anyhow::Result<()>;
    fn get_turn_change_set(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnChangeSet>>;
    fn list_turn_change_sets(&self, thread_id: Uuid) -> anyhow::Result<Vec<TurnChangeSet>>;
    fn mark_turn_change_set_reverted(
        &self,
        turn_id: Uuid,
        reverted_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<TurnChangeSet>>;
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
    fn upsert_subagent_run(&self, run: &SubagentRun) -> anyhow::Result<()>;
    fn get_subagent_run(&self, run_id: Uuid) -> anyhow::Result<Option<SubagentRun>>;
    fn list_subagent_runs(&self, thread_id: Uuid) -> anyhow::Result<Vec<SubagentRun>>;
    fn list_all_subagent_runs(&self) -> anyhow::Result<Vec<SubagentRun>>;
    fn save_subagent_conversation(
        &self,
        run_id: Uuid,
        conversation: &[ModelConversationMessage],
    ) -> anyhow::Result<()>;
    fn load_subagent_conversation(
        &self,
        run_id: Uuid,
    ) -> anyhow::Result<Option<Vec<ModelConversationMessage>>>;
    fn save_provider_conversation_state(
        &self,
        state: &ProviderConversationState,
    ) -> anyhow::Result<()>;
    fn take_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<Option<ProviderConversationState>>;
    fn clear_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<bool>;
    fn fail_interrupted_subagent_runs(&self) -> anyhow::Result<usize>;
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
    fn put_user_input_request(
        &self,
        thread_id: Uuid,
        request: &UserInputRequest,
        continuation: Value,
    ) -> anyhow::Result<UserInputRecord>;
    fn get_user_input_request(&self, request_id: Uuid) -> anyhow::Result<Option<UserInputRecord>>;
    fn list_user_input_requests(
        &self,
        thread_id: Uuid,
        status: Option<UserInputStatus>,
    ) -> anyhow::Result<Vec<UserInputRecord>>;
    fn get_user_input_continuation(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>>;
    fn resolve_user_input_request(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
        response: &UserInputResponse,
    ) -> anyhow::Result<Option<UserInputRecord>>;

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
            let message_tokens: usize = msg
                .parts
                .iter()
                .map(|part| match part {
                    MessagePart::Text { text } => crate::model_context::estimate_tokens(text),
                    MessagePart::ToolResult { result } => {
                        crate::model_context::estimate_tokens(&result.output)
                    }
                    MessagePart::ToolCall { call } => {
                        crate::model_context::estimate_tokens(&call.name)
                            .saturating_add(crate::model_context::estimate_tokens(
                                &call.input.to_string(),
                            ))
                            .saturating_add(16)
                    }
                    _ => 16,
                })
                .sum();
            used_tokens = used_tokens.saturating_add(message_tokens.saturating_add(50));
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

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("project name cannot be empty")]
    EmptyProjectName,
    #[error("thread title cannot be empty")]
    EmptyThreadTitle,
    #[error("workspace root cannot be empty")]
    EmptyWorkspaceRoot,
    #[error("a project already exists for workspace: {0}")]
    DuplicateWorkspace(String),
    #[error("project not found: {0}")]
    ProjectNotFound(Uuid),
    #[error("project has no workspace root: {0}")]
    ProjectHasNoWorkspace(Uuid),
    #[error("project workspace root cannot be cleared while it owns threads: {0}")]
    ProjectWorkspaceInUse(Uuid),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextBudget {
    pub total_tokens: usize,
    pub used_tokens: usize,
    pub message_count: usize,
    pub estimated_usage: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConversationState {
    pub thread_id: Uuid,
    pub agent_path: String,
    pub provider_id: String,
    pub model: String,
    pub response_id: String,
    pub compatibility_hash: String,
    pub updated_at: DateTime<Utc>,
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
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                workspace_root TEXT,
                workspace_key TEXT UNIQUE,
                pinned INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                workspace_root TEXT NOT NULL,
                project_id TEXT,
                experience_mode TEXT NOT NULL DEFAULT 'code'
                    CHECK(experience_mode IN ('work', 'code')),
                archived_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                role TEXT NOT NULL,
                parts_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS turns (
                turn_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                user_message_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                error TEXT,
                CHECK(status IN (
                    'running', 'waiting_approval', 'cancelling', 'succeeded',
                    'failed', 'cancelled', 'interrupted'
                )),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS turn_queue (
                message_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                queued_at TEXT NOT NULL,
                FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                objective TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN (
                    'draft', 'ready', 'active', 'paused', 'completed',
                    'blocked', 'cancelled', 'failed'
                )),
                plan_revision INTEGER NOT NULL DEFAULT 0,
                token_budget INTEGER,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                time_used_seconds INTEGER NOT NULL DEFAULT 0,
                version INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS goal_plan_revisions (
                goal_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                plan_json TEXT NOT NULL,
                change_reason TEXT,
                created_at TEXT NOT NULL,
                PRIMARY KEY(goal_id, revision),
                FOREIGN KEY(goal_id) REFERENCES goals(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS goal_tasks (
                goal_id TEXT NOT NULL,
                step_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN (
                    'pending', 'running', 'succeeded', 'deferred',
                    'blocked', 'cancelled', 'failed'
                )),
                status_reason TEXT,
                dependencies_json TEXT NOT NULL,
                acceptance_criteria_json TEXT NOT NULL,
                evidence_json TEXT NOT NULL,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(goal_id, step_id),
                FOREIGN KEY(goal_id) REFERENCES goals(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS goal_task_attempts (
                id TEXT PRIMARY KEY,
                goal_id TEXT NOT NULL,
                step_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                attempt_no INTEGER NOT NULL,
                status TEXT NOT NULL CHECK(status IN (
                    'running', 'succeeded', 'failed', 'interrupted'
                )),
                started_at TEXT NOT NULL,
                finished_at TEXT,
                evidence_json TEXT NOT NULL,
                error TEXT,
                UNIQUE(goal_id, step_id, attempt_no),
                FOREIGN KEY(goal_id, step_id) REFERENCES goal_tasks(goal_id, step_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS turn_change_sets (
                turn_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                workspace_root TEXT NOT NULL,
                repo_root TEXT,
                workspace_prefix TEXT,
                before_tree TEXT,
                after_tree TEXT,
                status TEXT NOT NULL,
                files_json TEXT NOT NULL,
                additions INTEGER NOT NULL DEFAULT 0,
                deletions INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                created_at TEXT NOT NULL,
                finalized_at TEXT,
                reverted_at TEXT,
                CHECK(status IN ('capturing', 'ready', 'empty', 'failed')),
                FOREIGN KEY(turn_id) REFERENCES turns(turn_id) ON DELETE CASCADE,
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

            CREATE TABLE IF NOT EXISTS subagent_runs (
                id TEXT PRIMARY KEY,
                parent_thread_id TEXT NOT NULL,
                parent_turn_id TEXT NOT NULL,
                agent_path TEXT NOT NULL DEFAULT '',
                parent_agent_path TEXT NOT NULL DEFAULT '/root',
                name TEXT NOT NULL,
                agent_type TEXT NOT NULL DEFAULT 'default',
                input TEXT NOT NULL,
                fork_turns TEXT NOT NULL DEFAULT 'all',
                last_task_message TEXT NOT NULL DEFAULT '',
                depth INTEGER NOT NULL,
                status TEXT NOT NULL,
                result TEXT,
                error TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                CHECK(status IN ('queued', 'running', 'completed', 'failed', 'cancelled', 'timed_out')),
                FOREIGN KEY(parent_thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS subagent_conversations (
                run_id TEXT PRIMARY KEY,
                conversation_json TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(run_id) REFERENCES subagent_runs(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS provider_conversation_states (
                thread_id TEXT NOT NULL,
                agent_path TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                response_id TEXT NOT NULL,
                compatibility_hash TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(thread_id, agent_path),
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

            CREATE TABLE IF NOT EXISTS user_input_requests (
                request_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                request_json TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('pending', 'answered')),
                response_json TEXT,
                continuation_json TEXT,
                created_at TEXT NOT NULL,
                answered_at TEXT,
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
                plugin_id TEXT,
                plugin_server_name TEXT,
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

            CREATE INDEX IF NOT EXISTS idx_turns_thread_started
                ON turns(thread_id, started_at DESC);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_turns_thread_active
                ON turns(thread_id)
                WHERE status IN ('running', 'cancelling');

            CREATE INDEX IF NOT EXISTS idx_turn_queue_thread_created
                ON turn_queue(thread_id, queued_at, message_id);

            CREATE INDEX IF NOT EXISTS idx_goals_thread_updated
                ON goals(thread_id, updated_at DESC);

            CREATE INDEX IF NOT EXISTS idx_goal_tasks_goal_ordinal
                ON goal_tasks(goal_id, ordinal);

            CREATE INDEX IF NOT EXISTS idx_goal_attempts_goal_started
                ON goal_task_attempts(goal_id, started_at);

            CREATE INDEX IF NOT EXISTS idx_turn_change_sets_thread_created
                ON turn_change_sets(thread_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_terminal_history_thread_seq
                ON terminal_history(thread_id, seq_start, seq_end);

            CREATE INDEX IF NOT EXISTS idx_terminal_history_thread_completed
                ON terminal_history(thread_id, completed_at);

            CREATE INDEX IF NOT EXISTS idx_artifacts_thread_created
                ON artifacts(thread_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_artifacts_thread_kind_created
                ON artifacts(thread_id, kind, created_at);

            CREATE INDEX IF NOT EXISTS idx_subagent_runs_thread_created
                ON subagent_runs(parent_thread_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_approval_continuations_thread
                ON approval_continuations(thread_id);

            CREATE INDEX IF NOT EXISTS idx_approvals_thread_status_created
                ON approvals(thread_id, status, created_at);

            CREATE INDEX IF NOT EXISTS idx_user_input_thread_status_created
                ON user_input_requests(thread_id, status, created_at);

            CREATE INDEX IF NOT EXISTS idx_thread_mcp_servers_thread
                ON thread_mcp_servers(thread_id, updated_at);

            CREATE INDEX IF NOT EXISTS idx_projects_order
                ON projects(pinned DESC, sort_order ASC, created_at ASC);
            "#,
        )?;

        if !table_has_column(&conn, "threads", "project_id")? {
            conn.execute(
                "ALTER TABLE threads ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL",
                [],
            )?;
        }
        if !table_has_column(&conn, "mcp_servers", "plugin_id")? {
            conn.execute("ALTER TABLE mcp_servers ADD COLUMN plugin_id TEXT", [])?;
        }
        if !table_has_column(&conn, "mcp_servers", "plugin_server_name")? {
            conn.execute(
                "ALTER TABLE mcp_servers ADD COLUMN plugin_server_name TEXT",
                [],
            )?;
        }
        conn.execute(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_servers_plugin_origin
            ON mcp_servers(plugin_id, plugin_server_name)
            WHERE plugin_id IS NOT NULL AND plugin_server_name IS NOT NULL
            "#,
            [],
        )?;
        if !table_has_column(&conn, "threads", "archived_at")? {
            conn.execute("ALTER TABLE threads ADD COLUMN archived_at TEXT", [])?;
        }
        if !table_has_column(&conn, "threads", "experience_mode")? {
            conn.execute(
                "ALTER TABLE threads ADD COLUMN experience_mode TEXT NOT NULL DEFAULT 'code' CHECK(experience_mode IN ('work', 'code'))",
                [],
            )?;
        }
        for (column, definition) in [
            ("agent_path", "TEXT NOT NULL DEFAULT ''"),
            ("parent_agent_path", "TEXT NOT NULL DEFAULT '/root'"),
            ("agent_type", "TEXT NOT NULL DEFAULT 'default'"),
            ("fork_turns", "TEXT NOT NULL DEFAULT 'all'"),
            ("last_task_message", "TEXT NOT NULL DEFAULT ''"),
        ] {
            if !table_has_column(&conn, "subagent_runs", column)? {
                conn.execute(
                    &format!("ALTER TABLE subagent_runs ADD COLUMN {column} {definition}"),
                    [],
                )?;
            }
        }
        conn.execute(
            "UPDATE subagent_runs SET agent_path = '/root/' || id WHERE agent_path = ''",
            [],
        )?;
        conn.execute(
            "UPDATE subagent_runs SET last_task_message = input WHERE last_task_message = ''",
            [],
        )?;
        conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_threads_project_updated
                ON threads(project_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_threads_archived_updated
                ON threads(archived_at, updated_at DESC);
            "#,
        )?;
        if schema_version < 1 {
            backfill_thread_projects(&mut conn)?;
        }
        if schema_version < 7 {
            conn.execute_batch("PRAGMA user_version = 7;")?;
        }
        let recovered_at = Utc::now().to_rfc3339();
        conn.execute(
            r#"
            UPDATE goal_task_attempts
            SET status = 'interrupted', finished_at = ?1,
                error = COALESCE(error, 'server restarted before task attempt completed')
            WHERE status = 'running'
            "#,
            params![&recovered_at],
        )?;
        conn.execute(
            r#"
            UPDATE goal_tasks
            SET status = 'blocked', status_reason = 'server restarted during task execution',
                updated_at = ?1
            WHERE status = 'running'
            "#,
            params![&recovered_at],
        )?;
        conn.execute(
            r#"
            UPDATE goals
            SET status = 'blocked', updated_at = ?1, version = version + 1
            WHERE status = 'active'
              AND EXISTS (
                  SELECT 1 FROM goal_tasks
                  WHERE goal_tasks.goal_id = goals.id AND goal_tasks.status = 'blocked'
              )
            "#,
            params![recovered_at],
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
                   timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
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
                   timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
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
                timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
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
                &config.plugin_id,
                &config.plugin_server_name,
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
                plugin_id = ?8,
                plugin_server_name = ?9,
                updated_at = ?10
            WHERE server_id = ?11
            "#,
            params![
                &config.name,
                &config.command,
                serde_json::to_string(&config.args)?,
                config.cwd.as_ref().map(|path| path.display().to_string()),
                serde_json::to_string(&config.env_keys)?,
                config.timeout_ms as i64,
                config.enabled as i64,
                &config.plugin_id,
                &config.plugin_server_name,
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

    pub fn list_plugin_mcp_servers(&self, plugin_id: &str) -> anyhow::Result<Vec<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT server_id, name, command, args_json, cwd, env_keys_json,
                   timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
            FROM mcp_servers
            WHERE plugin_id = ?1
            ORDER BY name ASC
            "#,
        )?;
        let rows = stmt.query_map(params![plugin_id], map_mcp_server)?;
        collect_rows(rows)
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
    fn create_project(
        &self,
        name: String,
        workspace_root: Option<PathBuf>,
        pinned: bool,
        sort_order: i64,
    ) -> anyhow::Result<Project> {
        let name = validated_project_name(name)?;
        let (workspace_root_value, workspace_key) = project_workspace_values(&workspace_root)?;
        let mut project = Project::new(name, workspace_root);
        project.pinned = pinned;
        project.sort_order = sort_order;

        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        ensure_workspace_available(&conn, workspace_key.as_deref(), None)?;
        insert_project(
            &conn,
            &project,
            workspace_root_value.as_deref(),
            workspace_key.as_deref(),
        )?;
        Ok(project)
    }

    fn get_project(&self, id: Uuid) -> anyhow::Result<Option<Project>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_project(&conn, id)
    }

    fn find_project_by_workspace(&self, workspace_root: &Path) -> anyhow::Result<Option<Project>> {
        let key = validated_workspace_key(workspace_root)?;
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        query_project_by_workspace_key(&conn, &key)
    }

    fn find_or_create_project(
        &self,
        name: String,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Project> {
        let name = validated_project_name(name)?;
        let workspace_key = validated_workspace_key(&workspace_root)?;
        let workspace_root_value = workspace_root.to_string_lossy().into_owned();
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        if let Some(project) = query_project_by_workspace_key(&conn, &workspace_key)? {
            return Ok(project);
        }

        let sort_order = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects",
            [],
            |row| row.get(0),
        )?;
        let mut project = Project::new(name, Some(workspace_root));
        project.sort_order = sort_order;
        insert_project(
            &conn,
            &project,
            Some(&workspace_root_value),
            Some(&workspace_key),
        )?;
        Ok(project)
    }

    fn list_projects(&self) -> anyhow::Result<Vec<Project>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT id, name, workspace_root, pinned, sort_order, created_at, updated_at
            FROM projects
            ORDER BY pinned DESC, sort_order ASC, created_at ASC
            "#,
        )?;
        let rows = stmt.query_map([], map_project)?;
        collect_rows(rows)
    }

    fn update_project(
        &self,
        id: Uuid,
        name: Option<String>,
        workspace_root: Option<Option<PathBuf>>,
        pinned: Option<bool>,
        sort_order: Option<i64>,
    ) -> anyhow::Result<Option<Project>> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let Some(mut project) = query_project(&tx, id)? else {
            return Ok(None);
        };

        if let Some(name) = name {
            project.name = validated_project_name(name)?;
        }
        if let Some(workspace_root) = workspace_root {
            let (_, workspace_key) = project_workspace_values(&workspace_root)?;
            ensure_workspace_available(&tx, workspace_key.as_deref(), Some(id))?;
            if workspace_root.is_none() {
                let thread_count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM threads WHERE project_id = ?1",
                    params![id.to_string()],
                    |row| row.get(0),
                )?;
                if thread_count > 0 {
                    return Err(StoreError::ProjectWorkspaceInUse(id).into());
                }
            }
            project.workspace_root = workspace_root;
        }
        if let Some(pinned) = pinned {
            project.pinned = pinned;
        }
        if let Some(sort_order) = sort_order {
            project.sort_order = sort_order;
        }
        project.updated_at = Utc::now();
        let (workspace_root_value, workspace_key) =
            project_workspace_values(&project.workspace_root)?;
        tx.execute(
            r#"
            UPDATE projects
            SET name = ?1, workspace_root = ?2, workspace_key = ?3,
                pinned = ?4, sort_order = ?5, updated_at = ?6
            WHERE id = ?7
            "#,
            params![
                &project.name,
                workspace_root_value,
                workspace_key,
                project.pinned as i64,
                project.sort_order,
                project.updated_at.to_rfc3339(),
                id.to_string(),
            ],
        )?;
        if let Some(workspace_root) = project.workspace_root.as_ref() {
            tx.execute(
                r#"
                UPDATE threads
                SET workspace_root = ?1, updated_at = ?2
                WHERE project_id = ?3 AND workspace_root != ?1
                "#,
                params![
                    workspace_root.to_string_lossy(),
                    project.updated_at.to_rfc3339(),
                    id.to_string(),
                ],
            )?;
        }
        tx.commit()?;
        Ok(Some(project))
    }

    fn delete_project(&self, id: Uuid) -> anyhow::Result<bool> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        if query_project(&tx, id)?.is_none() {
            return Ok(false);
        }
        let archived_at = Utc::now().to_rfc3339();
        tx.execute(
            r#"
            UPDATE threads
            SET project_id = NULL,
                archived_at = COALESCE(archived_at, ?1),
                updated_at = ?1
            WHERE project_id = ?2
            "#,
            params![archived_at, id.to_string()],
        )?;
        let deleted = tx.execute(
            "DELETE FROM projects WHERE id = ?1",
            params![id.to_string()],
        )?;
        tx.commit()?;
        Ok(deleted > 0)
    }

    fn create_thread(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
    ) -> anyhow::Result<Thread> {
        self.create_thread_with_mode(title, workspace_root, ExperienceMode::Code)
    }

    fn create_thread_with_mode(
        &self,
        title: Option<String>,
        workspace_root: PathBuf,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread> {
        let thread = Thread::new_with_mode(
            title.unwrap_or_else(|| "Untitled thread".to_string()),
            workspace_root,
            experience_mode,
        );
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        insert_thread(&conn, &thread)?;
        Ok(thread)
    }

    fn create_thread_in_project(
        &self,
        title: Option<String>,
        project_id: Uuid,
    ) -> anyhow::Result<Thread> {
        self.create_thread_in_project_with_mode(title, project_id, ExperienceMode::Code)
    }

    fn create_thread_in_project_with_mode(
        &self,
        title: Option<String>,
        project_id: Uuid,
        experience_mode: ExperienceMode,
    ) -> anyhow::Result<Thread> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let project =
            query_project(&conn, project_id)?.ok_or(StoreError::ProjectNotFound(project_id))?;
        let workspace_root = project
            .workspace_root
            .ok_or(StoreError::ProjectHasNoWorkspace(project_id))?;
        let thread = Thread::new_in_project_with_mode(
            title.unwrap_or_else(|| "Untitled thread".to_string()),
            workspace_root,
            project_id,
            experience_mode,
        );
        insert_thread(&conn, &thread)?;
        Ok(thread)
    }

    fn get_thread(&self, id: Uuid) -> anyhow::Result<Option<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let thread = conn
            .query_row(
                r#"
                SELECT id, title, workspace_root, project_id, archived_at, experience_mode, created_at, updated_at
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
        self.list_threads_including_archived(false)
    }

    fn list_threads_including_archived(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let sql = if include_archived {
            r#"
            SELECT id, title, workspace_root, project_id, archived_at, experience_mode, created_at, updated_at
            FROM threads
            ORDER BY updated_at DESC
            "#
        } else {
            r#"
            SELECT id, title, workspace_root, project_id, archived_at, experience_mode, created_at, updated_at
            FROM threads
            WHERE archived_at IS NULL
            ORDER BY updated_at DESC
            "#
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], map_thread)?;
        collect_rows(rows)
    }

    fn update_thread(
        &self,
        id: Uuid,
        title: Option<String>,
        project_id: Option<Option<Uuid>>,
        archived: Option<bool>,
    ) -> anyhow::Result<Option<Thread>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let Some(mut thread) = query_thread(&conn, id)? else {
            return Ok(None);
        };
        if let Some(title) = title {
            let title = title.trim();
            if title.is_empty() {
                return Err(StoreError::EmptyThreadTitle.into());
            }
            thread.title = title.to_string();
        }
        if let Some(project_id) = project_id {
            match project_id {
                Some(project_id) => {
                    let project = query_project(&conn, project_id)?
                        .ok_or(StoreError::ProjectNotFound(project_id))?;
                    let workspace_root = project
                        .workspace_root
                        .ok_or(StoreError::ProjectHasNoWorkspace(project_id))?;
                    thread.project_id = Some(project_id);
                    thread.workspace_root = workspace_root;
                }
                None => thread.project_id = None,
            }
        }
        if let Some(archived) = archived {
            thread.archived_at = archived.then(Utc::now);
        }
        thread.updated_at = Utc::now();
        conn.execute(
            r#"
            UPDATE threads
            SET title = ?1, workspace_root = ?2, project_id = ?3,
                archived_at = ?4, updated_at = ?5
            WHERE id = ?6
            "#,
            params![
                &thread.title,
                thread.workspace_root.to_string_lossy(),
                thread.project_id.map(|value| value.to_string()),
                thread.archived_at.map(|value| value.to_rfc3339()),
                thread.updated_at.to_rfc3339(),
                id.to_string(),
            ],
        )?;
        Ok(Some(thread))
    }

    fn delete_thread(&self, id: Uuid) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let deleted = conn.execute("DELETE FROM threads WHERE id = ?1", params![id.to_string()])?;
        Ok(deleted > 0)
    }

    fn create_goal(
        &self,
        thread_id: Uuid,
        objective: String,
        status: GoalStatus,
        token_budget: Option<u64>,
    ) -> anyhow::Result<GoalSnapshot> {
        let objective = objective.trim().to_string();
        anyhow::ensure!(!objective.is_empty(), "goal objective cannot be empty");
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        anyhow::ensure!(
            query_thread(&conn, thread_id)?.is_some(),
            "thread not found: {thread_id}"
        );
        let goal = GoalRecord::new(thread_id, objective, status, token_budget);
        conn.execute(
            r#"
            INSERT INTO goals (
                id, thread_id, objective, status, plan_revision, token_budget,
                tokens_used, time_used_seconds, version, created_at, updated_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                goal.id.to_string(),
                goal.thread_id.to_string(),
                &goal.objective,
                goal.status.as_str(),
                goal.plan_revision as i64,
                goal.token_budget.map(|value| value as i64),
                goal.tokens_used as i64,
                goal.time_used_seconds as i64,
                goal.version as i64,
                goal.created_at.to_rfc3339(),
                goal.updated_at.to_rfc3339(),
                goal.completed_at.map(|value| value.to_rfc3339()),
            ],
        )?;
        load_goal_snapshot(&conn, goal.id)?.context("created goal disappeared")
    }

    fn get_goal(&self, id: Uuid) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        load_goal_snapshot(&conn, id)
    }

    fn get_thread_goal(&self, thread_id: Uuid) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let goal_id = conn
            .query_row(
                r#"
                SELECT id
                FROM goals
                WHERE thread_id = ?1
                ORDER BY
                    CASE WHEN status IN ('completed', 'cancelled', 'failed') THEN 1 ELSE 0 END,
                    updated_at DESC, rowid DESC
                LIMIT 1
                "#,
                params![thread_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|value| Uuid::parse_str(&value))
            .transpose()?;
        goal_id
            .map(|goal_id| load_goal_snapshot(&conn, goal_id))
            .transpose()
            .map(Option::flatten)
    }

    fn update_goal_status(
        &self,
        thread_id: Uuid,
        goal_id: Uuid,
        status: GoalStatus,
    ) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let current = match query_goal(&conn, goal_id)? {
            Some(goal) if goal.thread_id == thread_id => goal,
            Some(_) => anyhow::bail!("goal {goal_id} does not belong to thread {thread_id}"),
            None => return Ok(None),
        };
        anyhow::ensure!(
            valid_goal_transition(current.status, status),
            "invalid goal transition: {} -> {}",
            current.status.as_str(),
            status.as_str()
        );
        let now = Utc::now();
        let completed_at = status.is_terminal().then(|| now.to_rfc3339());
        conn.execute(
            r#"
            UPDATE goals
            SET status = ?3, updated_at = ?4, completed_at = ?5, version = version + 1
            WHERE id = ?1 AND thread_id = ?2
            "#,
            params![
                goal_id.to_string(),
                thread_id.to_string(),
                status.as_str(),
                now.to_rfc3339(),
                completed_at,
            ],
        )?;
        load_goal_snapshot(&conn, goal_id)
    }

    fn apply_goal_plan(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
        plan: &TaskPlan,
    ) -> anyhow::Result<GoalSnapshot> {
        let goal_id = Uuid::parse_str(plan.goal_id.trim())
            .with_context(|| format!("task plan has invalid goal id: {}", plan.goal_id))?;
        anyhow::ensure!(!plan.steps.is_empty(), "goal plan cannot be empty");
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let goal = query_goal(&tx, goal_id)?.context("goal does not exist")?;
        anyhow::ensure!(
            goal.thread_id == thread_id,
            "goal {goal_id} does not belong to thread {thread_id}"
        );
        anyhow::ensure!(
            plan.plan_revision >= goal.plan_revision,
            "stale goal plan revision {} (current {})",
            plan.plan_revision,
            goal.plan_revision
        );
        if plan.plan_revision == goal.plan_revision && goal.plan_revision > 0 {
            let snapshot = load_goal_snapshot(&tx, goal_id)?.context("goal disappeared")?;
            tx.commit()?;
            return Ok(snapshot);
        }

        let now = Utc::now();
        tx.execute(
            r#"
            INSERT INTO goal_plan_revisions (
                goal_id, revision, plan_json, change_reason, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![
                goal_id.to_string(),
                plan.plan_revision as i64,
                serde_json::to_string(plan)?,
                plan.change_reason.as_deref(),
                now.to_rfc3339(),
            ],
        )?;

        let existing = query_goal_task_states(&tx, goal_id)?;
        let incoming_ids = plan
            .steps
            .iter()
            .map(|step| step.id.clone())
            .collect::<std::collections::HashSet<_>>();

        for (ordinal, step) in plan.steps.iter().enumerate() {
            let new_status = GoalTaskStatus::from(step.status);
            let (old_status, old_attempt_count, max_attempts) = existing
                .get(&step.id)
                .copied()
                .map(|(status, attempts, max)| (Some(status), attempts, max))
                .unwrap_or((None, 0, 3));
            let starts_attempt = new_status == GoalTaskStatus::Running
                && old_status != Some(GoalTaskStatus::Running);
            let attempt_count = old_attempt_count + u32::from(starts_attempt);
            anyhow::ensure!(
                attempt_count <= max_attempts,
                "task {} exceeded its retry limit ({max_attempts})",
                step.id
            );

            tx.execute(
                r#"
                INSERT INTO goal_tasks (
                    goal_id, step_id, ordinal, title, status, status_reason,
                    dependencies_json, acceptance_criteria_json, evidence_json,
                    attempt_count, max_attempts, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT(goal_id, step_id) DO UPDATE SET
                    ordinal = excluded.ordinal,
                    title = excluded.title,
                    status = excluded.status,
                    status_reason = excluded.status_reason,
                    dependencies_json = excluded.dependencies_json,
                    acceptance_criteria_json = excluded.acceptance_criteria_json,
                    evidence_json = excluded.evidence_json,
                    attempt_count = excluded.attempt_count,
                    updated_at = excluded.updated_at
                "#,
                params![
                    goal_id.to_string(),
                    &step.id,
                    ordinal as i64,
                    &step.title,
                    new_status.as_str(),
                    step.status_reason.as_deref(),
                    serde_json::to_string(&step.dependencies)?,
                    serde_json::to_string(&step.acceptance_criteria)?,
                    serde_json::to_string(&step.evidence)?,
                    attempt_count as i64,
                    max_attempts as i64,
                    now.to_rfc3339(),
                ],
            )?;

            if starts_attempt {
                tx.execute(
                    r#"
                    INSERT INTO goal_task_attempts (
                        id, goal_id, step_id, turn_id, attempt_no, status,
                        started_at, finished_at, evidence_json, error
                    ) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6, NULL, '[]', NULL)
                    "#,
                    params![
                        Uuid::new_v4().to_string(),
                        goal_id.to_string(),
                        &step.id,
                        turn_id.to_string(),
                        attempt_count as i64,
                        now.to_rfc3339(),
                    ],
                )?;
            } else if old_status == Some(GoalTaskStatus::Running)
                && new_status != GoalTaskStatus::Running
            {
                let (attempt_status, error) = match new_status {
                    GoalTaskStatus::Succeeded => (GoalAttemptStatus::Succeeded, None),
                    GoalTaskStatus::Cancelled | GoalTaskStatus::Deferred => (
                        GoalAttemptStatus::Interrupted,
                        step.status_reason.as_deref(),
                    ),
                    _ => (GoalAttemptStatus::Failed, step.status_reason.as_deref()),
                };
                tx.execute(
                    r#"
                    UPDATE goal_task_attempts
                    SET status = ?4, finished_at = ?5, evidence_json = ?6, error = ?7
                    WHERE goal_id = ?1 AND step_id = ?2 AND attempt_no = ?3
                      AND status = 'running'
                    "#,
                    params![
                        goal_id.to_string(),
                        &step.id,
                        old_attempt_count as i64,
                        attempt_status.as_str(),
                        now.to_rfc3339(),
                        serde_json::to_string(&step.evidence)?,
                        error,
                    ],
                )?;
            }
        }

        for (step_id, (old_status, _, _)) in &existing {
            if incoming_ids.contains(step_id) {
                continue;
            }
            tx.execute(
                r#"
                UPDATE goal_tasks
                SET status = 'cancelled', status_reason = 'removed by plan revision', updated_at = ?3
                WHERE goal_id = ?1 AND step_id = ?2
                "#,
                params![goal_id.to_string(), step_id, now.to_rfc3339()],
            )?;
            if *old_status == GoalTaskStatus::Running {
                tx.execute(
                    r#"
                    UPDATE goal_task_attempts
                    SET status = 'interrupted', finished_at = ?3,
                        error = 'step removed by plan revision'
                    WHERE goal_id = ?1 AND step_id = ?2 AND status = 'running'
                    "#,
                    params![goal_id.to_string(), step_id, now.to_rfc3339()],
                )?;
            }
        }

        let projected_status = if goal.status == GoalStatus::Draft {
            GoalStatus::Ready
        } else if goal.status == GoalStatus::Active && !plan.is_active() {
            GoalStatus::Completed
        } else {
            goal.status
        };
        let completed_at = (projected_status == GoalStatus::Completed).then(|| now.to_rfc3339());
        tx.execute(
            r#"
            UPDATE goals
            SET status = ?2, plan_revision = ?3, updated_at = ?4,
                completed_at = ?5, version = version + 1
            WHERE id = ?1
            "#,
            params![
                goal_id.to_string(),
                projected_status.as_str(),
                plan.plan_revision as i64,
                now.to_rfc3339(),
                completed_at,
            ],
        )?;
        let snapshot = load_goal_snapshot(&tx, goal_id)?.context("goal disappeared")?;
        tx.commit()?;
        Ok(snapshot)
    }

    fn add_goal_usage(
        &self,
        goal_id: Uuid,
        tokens: u64,
        elapsed_seconds: u64,
    ) -> anyhow::Result<Option<GoalSnapshot>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE goals
            SET tokens_used = tokens_used + ?2,
                time_used_seconds = time_used_seconds + ?3,
                updated_at = ?4,
                version = version + 1
            WHERE id = ?1
            "#,
            params![
                goal_id.to_string(),
                i64::try_from(tokens).context("goal token usage exceeds SQLite range")?,
                i64::try_from(elapsed_seconds).context("goal time usage exceeds SQLite range")?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        load_goal_snapshot(&conn, goal_id)
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

    fn enqueue_turn_message(&self, thread_id: Uuid, message_id: Uuid) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO turn_queue (message_id, thread_id, queued_at)
            VALUES (?1, ?2, ?3)
            "#,
            params![
                message_id.to_string(),
                thread_id.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, thread_id)?;
        Ok(())
    }

    fn list_queued_turn_messages(&self, thread_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT message_id
            FROM turn_queue
            WHERE thread_id = ?1
            ORDER BY queued_at ASC, rowid ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], |row| {
            let raw: String = row.get(0)?;
            Uuid::parse_str(&raw).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })
        })?;
        collect_rows(rows)
    }

    fn remove_queued_turn_message(
        &self,
        thread_id: Uuid,
        message_id: Uuid,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let removed = conn.execute(
            "DELETE FROM turn_queue WHERE thread_id = ?1 AND message_id = ?2",
            params![thread_id.to_string(), message_id.to_string()],
        )?;
        Ok(removed > 0)
    }

    fn insert_turn(&self, turn: TurnRecord) -> anyhow::Result<TurnRecord> {
        anyhow::ensure!(
            turn.status == TurnStatus::Running,
            "new turns must start in running status"
        );
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO turns (
                turn_id, thread_id, user_message_id, status, started_at,
                updated_at, completed_at, error
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                turn.turn_id.to_string(),
                turn.thread_id.to_string(),
                turn.user_message_id.to_string(),
                turn.status.as_str(),
                turn.started_at.to_rfc3339(),
                turn.updated_at.to_rfc3339(),
                turn.completed_at.map(|value| value.to_rfc3339()),
                turn.error.as_deref(),
            ],
        )?;
        touch_thread(&conn, turn.thread_id)?;
        Ok(turn)
    }

    fn get_turn(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, user_message_id, status, started_at,
                   updated_at, completed_at, error
            FROM turns
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_active_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, user_message_id, status, started_at,
                   updated_at, completed_at, error
            FROM turns
            WHERE thread_id = ?1 AND status IN ('running', 'cancelling')
            ORDER BY started_at DESC, rowid DESC
            LIMIT 1
            "#,
            params![thread_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_latest_turn(&self, thread_id: Uuid) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, user_message_id, status, started_at,
                   updated_at, completed_at, error
            FROM turns
            WHERE thread_id = ?1
            ORDER BY started_at DESC, rowid DESC
            LIMIT 1
            "#,
            params![thread_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn update_turn_status(
        &self,
        turn_id: Uuid,
        status: TurnStatus,
        error: Option<String>,
    ) -> anyhow::Result<Option<TurnRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now();
        let completed_at = status.is_terminal().then(|| now.to_rfc3339());
        let changed = conn.execute(
            r#"
            UPDATE turns
            SET status = ?2, updated_at = ?3, completed_at = ?4, error = ?5
            WHERE turn_id = ?1
            "#,
            params![
                turn_id.to_string(),
                status.as_str(),
                now.to_rfc3339(),
                completed_at,
                error,
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, user_message_id, status, started_at,
                   updated_at, completed_at, error
            FROM turns
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn,
        )
        .optional()
        .map_err(Into::into)
    }

    fn interrupt_active_turns(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now().to_rfc3339();
        let changed = conn.execute(
            r#"
            UPDATE turns
            SET status = 'interrupted', updated_at = ?1, completed_at = ?1,
                error = 'server restarted before turn completed'
            WHERE status IN ('running', 'cancelling')
            "#,
            params![now],
        )?;
        Ok(changed)
    }

    fn upsert_turn_change_set(&self, change_set: &TurnChangeSet) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO turn_change_sets (
                turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                before_tree, after_tree, status, files_json, additions, deletions,
                error, created_at, finalized_at, reverted_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            ON CONFLICT(turn_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                workspace_root = excluded.workspace_root,
                repo_root = excluded.repo_root,
                workspace_prefix = excluded.workspace_prefix,
                before_tree = excluded.before_tree,
                after_tree = excluded.after_tree,
                status = excluded.status,
                files_json = excluded.files_json,
                additions = excluded.additions,
                deletions = excluded.deletions,
                error = excluded.error,
                finalized_at = excluded.finalized_at,
                reverted_at = excluded.reverted_at
            "#,
            params![
                change_set.turn_id.to_string(),
                change_set.thread_id.to_string(),
                change_set.workspace_root.to_string_lossy(),
                change_set
                    .repo_root
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                change_set
                    .workspace_prefix
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
                change_set.before_tree.as_deref(),
                change_set.after_tree.as_deref(),
                change_set.status.as_str(),
                serde_json::to_string(&change_set.files)?,
                i64::try_from(change_set.additions)
                    .context("turn additions exceed SQLite range")?,
                i64::try_from(change_set.deletions)
                    .context("turn deletions exceed SQLite range")?,
                change_set.error.as_deref(),
                change_set.created_at.to_rfc3339(),
                change_set.finalized_at.map(|value| value.to_rfc3339()),
                change_set.reverted_at.map(|value| value.to_rfc3339()),
            ],
        )?;
        Ok(())
    }

    fn get_turn_change_set(&self, turn_id: Uuid) -> anyhow::Result<Option<TurnChangeSet>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                   before_tree, after_tree, status, files_json, additions, deletions,
                   error, created_at, finalized_at, reverted_at
            FROM turn_change_sets
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn_change_set,
        )
        .optional()
        .map_err(Into::into)
    }

    fn list_turn_change_sets(&self, thread_id: Uuid) -> anyhow::Result<Vec<TurnChangeSet>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                   before_tree, after_tree, status, files_json, additions, deletions,
                   error, created_at, finalized_at, reverted_at
            FROM turn_change_sets
            WHERE thread_id = ?1
            ORDER BY created_at ASC, rowid ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_turn_change_set)?;
        collect_rows(rows)
    }

    fn mark_turn_change_set_reverted(
        &self,
        turn_id: Uuid,
        reverted_at: DateTime<Utc>,
    ) -> anyhow::Result<Option<TurnChangeSet>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            "UPDATE turn_change_sets SET reverted_at = ?2 WHERE turn_id = ?1",
            params![turn_id.to_string(), reverted_at.to_rfc3339()],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        conn.query_row(
            r#"
            SELECT turn_id, thread_id, workspace_root, repo_root, workspace_prefix,
                   before_tree, after_tree, status, files_json, additions, deletions,
                   error, created_at, finalized_at, reverted_at
            FROM turn_change_sets
            WHERE turn_id = ?1
            "#,
            params![turn_id.to_string()],
            map_turn_change_set,
        )
        .optional()
        .map_err(Into::into)
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

    fn upsert_subagent_run(&self, run: &SubagentRun) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO subagent_runs (
                id, parent_thread_id, parent_turn_id, agent_path, parent_agent_path,
                name, agent_type, input, fork_turns, last_task_message, depth, status,
                result, error, created_at, started_at, completed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            ON CONFLICT(id) DO UPDATE SET
                parent_turn_id = excluded.parent_turn_id,
                last_task_message = excluded.last_task_message,
                status = excluded.status,
                result = excluded.result,
                error = excluded.error,
                started_at = excluded.started_at,
                completed_at = excluded.completed_at
            "#,
            params![
                run.id.to_string(),
                run.parent_thread_id.to_string(),
                run.parent_turn_id.to_string(),
                &run.agent_path,
                &run.parent_agent_path,
                &run.name,
                &run.agent_type,
                &run.input,
                &run.fork_turns,
                &run.last_task_message,
                i64::from(run.depth),
                run.status.as_str(),
                run.result.as_deref(),
                run.error.as_deref(),
                run.created_at.to_rfc3339(),
                run.started_at.as_ref().map(DateTime::to_rfc3339),
                run.completed_at.as_ref().map(DateTime::to_rfc3339),
            ],
        )?;
        touch_thread(&conn, run.parent_thread_id)?;
        Ok(())
    }

    fn get_subagent_run(&self, run_id: Uuid) -> anyhow::Result<Option<SubagentRun>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        Ok(conn
            .query_row(
                r#"
                SELECT id, parent_thread_id, parent_turn_id, agent_path, parent_agent_path,
                       name, agent_type, input, fork_turns, last_task_message, depth, status,
                       result, error, created_at, started_at, completed_at
                FROM subagent_runs
                WHERE id = ?1
                "#,
                params![run_id.to_string()],
                map_subagent_run,
            )
            .optional()?)
    }

    fn list_subagent_runs(&self, thread_id: Uuid) -> anyhow::Result<Vec<SubagentRun>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut statement = conn.prepare(
            r#"
            SELECT id, parent_thread_id, parent_turn_id, agent_path, parent_agent_path,
                   name, agent_type, input, fork_turns, last_task_message, depth, status,
                   result, error, created_at, started_at, completed_at
            FROM subagent_runs
            WHERE parent_thread_id = ?1
            ORDER BY created_at DESC
            "#,
        )?;
        let rows = statement.query_map(params![thread_id.to_string()], map_subagent_run)?;
        collect_rows(rows)
    }

    fn list_all_subagent_runs(&self) -> anyhow::Result<Vec<SubagentRun>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut statement = conn.prepare(
            r#"
            SELECT id, parent_thread_id, parent_turn_id, agent_path, parent_agent_path,
                   name, agent_type, input, fork_turns, last_task_message, depth, status,
                   result, error, created_at, started_at, completed_at
            FROM subagent_runs
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = statement.query_map([], map_subagent_run)?;
        collect_rows(rows)
    }

    fn save_subagent_conversation(
        &self,
        run_id: Uuid,
        conversation: &[ModelConversationMessage],
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO subagent_conversations (run_id, conversation_json, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(run_id) DO UPDATE SET
                conversation_json = excluded.conversation_json,
                updated_at = excluded.updated_at
            "#,
            params![
                run_id.to_string(),
                serde_json::to_string(conversation)?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn load_subagent_conversation(
        &self,
        run_id: Uuid,
    ) -> anyhow::Result<Option<Vec<ModelConversationMessage>>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let value = conn
            .query_row(
                "SELECT conversation_json FROM subagent_conversations WHERE run_id = ?1",
                params![run_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    fn save_provider_conversation_state(
        &self,
        state: &ProviderConversationState,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO provider_conversation_states (
                thread_id, agent_path, provider_id, model, response_id,
                compatibility_hash, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(thread_id, agent_path) DO UPDATE SET
                provider_id = excluded.provider_id,
                model = excluded.model,
                response_id = excluded.response_id,
                compatibility_hash = excluded.compatibility_hash,
                updated_at = excluded.updated_at
            "#,
            params![
                state.thread_id.to_string(),
                &state.agent_path,
                &state.provider_id,
                &state.model,
                &state.response_id,
                &state.compatibility_hash,
                state.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn take_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<Option<ProviderConversationState>> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let transaction = conn.transaction()?;
        let state = transaction
            .query_row(
                r#"
                SELECT provider_id, model, response_id, compatibility_hash, updated_at
                FROM provider_conversation_states
                WHERE thread_id = ?1 AND agent_path = ?2
                "#,
                params![thread_id.to_string(), agent_path],
                |row| {
                    Ok(ProviderConversationState {
                        thread_id,
                        agent_path: agent_path.to_string(),
                        provider_id: row.get(0)?,
                        model: row.get(1)?,
                        response_id: row.get(2)?,
                        compatibility_hash: row.get(3)?,
                        updated_at: parse_datetime(row.get::<_, String>(4)?, 4)?,
                    })
                },
            )
            .optional()?;
        transaction.execute(
            "DELETE FROM provider_conversation_states WHERE thread_id = ?1 AND agent_path = ?2",
            params![thread_id.to_string(), agent_path],
        )?;
        transaction.commit()?;
        Ok(state)
    }

    fn clear_provider_conversation_state(
        &self,
        thread_id: Uuid,
        agent_path: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        Ok(conn.execute(
            "DELETE FROM provider_conversation_states WHERE thread_id = ?1 AND agent_path = ?2",
            params![thread_id.to_string(), agent_path],
        )? > 0)
    }

    fn fail_interrupted_subagent_runs(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let now = Utc::now().to_rfc3339();
        Ok(conn.execute(
            r#"
            UPDATE subagent_runs
            SET status = 'failed',
                error = 'OpenTopia restarted before this subagent completed.',
                completed_at = ?1
            WHERE status IN ('queued', 'running')
            "#,
            params![now],
        )?)
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

    fn put_user_input_request(
        &self,
        thread_id: Uuid,
        request: &UserInputRequest,
        continuation: Value,
    ) -> anyhow::Result<UserInputRecord> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let created_at = Utc::now();
        conn.execute(
            r#"
            INSERT INTO user_input_requests (
                request_id, thread_id, request_json, status, response_json,
                continuation_json, created_at, answered_at
            ) VALUES (?1, ?2, ?3, 'pending', NULL, ?4, ?5, NULL)
            ON CONFLICT(request_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                request_json = excluded.request_json,
                status = 'pending',
                response_json = NULL,
                continuation_json = excluded.continuation_json,
                created_at = excluded.created_at,
                answered_at = NULL
            "#,
            params![
                request.request_id.to_string(),
                thread_id.to_string(),
                serde_json::to_string(request)?,
                serde_json::to_string(&continuation)?,
                created_at.to_rfc3339(),
            ],
        )?;
        touch_thread(&conn, thread_id)?;
        Ok(UserInputRecord {
            thread_id,
            request: request.clone(),
            status: UserInputStatus::Pending,
            response: None,
            created_at,
            answered_at: None,
        })
    }

    fn get_user_input_request(&self, request_id: Uuid) -> anyhow::Result<Option<UserInputRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT request_id, thread_id, request_json, status, response_json,
                   created_at, answered_at
            FROM user_input_requests
            WHERE request_id = ?1
            "#,
            params![request_id.to_string()],
            map_user_input_record,
        )
        .optional()
        .map_err(Into::into)
    }

    fn list_user_input_requests(
        &self,
        thread_id: Uuid,
        status: Option<UserInputStatus>,
    ) -> anyhow::Result<Vec<UserInputRecord>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let select = r#"
            SELECT request_id, thread_id, request_json, status, response_json,
                   created_at, answered_at
            FROM user_input_requests
        "#;
        let records = if let Some(status) = status {
            let mut stmt = conn.prepare(&format!(
                "{select} WHERE thread_id = ?1 AND status = ?2 ORDER BY created_at ASC"
            ))?;
            let records = collect_rows(stmt.query_map(
                params![thread_id.to_string(), status.as_str()],
                map_user_input_record,
            )?)?;
            records
        } else {
            let mut stmt = conn.prepare(&format!(
                "{select} WHERE thread_id = ?1 ORDER BY created_at ASC"
            ))?;
            let records = collect_rows(
                stmt.query_map(params![thread_id.to_string()], map_user_input_record)?,
            )?;
            records
        };
        Ok(records)
    }

    fn get_user_input_continuation(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<Value>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let continuation = conn
            .query_row(
                r#"
                SELECT continuation_json
                FROM user_input_requests
                WHERE request_id = ?1 AND thread_id = ?2 AND status = 'pending'
                "#,
                params![request_id.to_string(), thread_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        continuation
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    fn resolve_user_input_request(
        &self,
        request_id: Uuid,
        thread_id: Uuid,
        response: &UserInputResponse,
    ) -> anyhow::Result<Option<UserInputRecord>> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let answered_at = Utc::now();
        let changed = tx.execute(
            r#"
            UPDATE user_input_requests
            SET status = 'answered', response_json = ?1, continuation_json = NULL,
                answered_at = ?2
            WHERE request_id = ?3 AND thread_id = ?4 AND status = 'pending'
            "#,
            params![
                serde_json::to_string(response)?,
                answered_at.to_rfc3339(),
                request_id.to_string(),
                thread_id.to_string(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        touch_thread(&tx, thread_id)?;
        tx.commit()?;
        drop(conn);
        self.get_user_input_request(request_id)
    }
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn backfill_thread_projects(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    let mut projects_by_key = HashMap::new();
    {
        let mut stmt =
            tx.prepare("SELECT id, workspace_key FROM projects WHERE workspace_key IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, key) = row?;
            projects_by_key.insert(key, id);
        }
    }

    let mut threads = Vec::new();
    {
        let mut stmt = tx.prepare(
            r#"
            SELECT id, workspace_root, created_at, updated_at
            FROM threads
            WHERE project_id IS NULL
            ORDER BY created_at ASC
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            threads.push(row?);
        }
    }

    for (thread_id, workspace_root, created_at, updated_at) in threads {
        let workspace_key = normalize_workspace_key(Path::new(&workspace_root));
        let project_id = if let Some(project_id) = projects_by_key.get(&workspace_key) {
            project_id.clone()
        } else {
            let project_id = Uuid::new_v4().to_string();
            tx.execute(
                r#"
                INSERT INTO projects (
                    id, name, workspace_root, workspace_key, pinned, sort_order,
                    created_at, updated_at
                )
                VALUES (
                    ?1, ?2, ?3, ?4, 0,
                    (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects),
                    ?5, ?6
                )
                "#,
                params![
                    &project_id,
                    project_name_from_workspace(&workspace_root),
                    &workspace_root,
                    &workspace_key,
                    &created_at,
                    &updated_at,
                ],
            )?;
            projects_by_key.insert(workspace_key, project_id.clone());
            project_id
        };
        tx.execute(
            "UPDATE threads SET project_id = ?1 WHERE id = ?2",
            params![project_id, thread_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn normalize_workspace_key(path: &Path) -> String {
    let original = path.to_string_lossy();
    let had_backslash = original.contains('\\');
    let mut value = original.trim().replace('\\', "/");
    let lowercase = value.to_ascii_lowercase();
    let mut is_windows = had_backslash;

    if lowercase.starts_with("//?/unc/") {
        value = format!("//{}", &value[8..]);
        is_windows = true;
    } else if lowercase.starts_with("//?/") {
        value = value[4..].to_string();
        is_windows = true;
    }

    let bytes = value.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let is_unc = value.starts_with("//");
    let is_absolute = !is_unc && value.starts_with('/');
    let drive_absolute = has_drive && value.as_bytes().get(2) == Some(&b'/');
    is_windows |= has_drive || is_unc;

    let minimum_depth = if is_unc {
        2
    } else if drive_absolute {
        1
    } else {
        0
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in value.split('/').filter(|segment| !segment.is_empty()) {
        match segment {
            "." => {}
            ".." if segments.len() > minimum_depth && segments.last() != Some(&"..") => {
                segments.pop();
            }
            ".." if !is_absolute && !drive_absolute && !is_unc => segments.push(segment),
            ".." => {}
            _ => segments.push(segment),
        }
    }

    let mut normalized = if is_unc {
        format!("//{}", segments.join("/"))
    } else if is_absolute {
        format!("/{}", segments.join("/"))
    } else {
        segments.join("/")
    };
    if drive_absolute && segments.len() == 1 {
        normalized.push('/');
    }
    if normalized.is_empty() && !original.trim().is_empty() {
        normalized.push('.');
    }
    if is_windows {
        normalized.make_ascii_lowercase();
    }
    normalized
}

fn project_name_from_workspace(workspace_root: &str) -> String {
    let normalized = workspace_root.trim().replace('\\', "/");
    normalized
        .trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .filter(|part| *part != ".")
        .unwrap_or("Workspace")
        .to_string()
}

fn validated_project_name(name: String) -> anyhow::Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::EmptyProjectName.into());
    }
    Ok(name.to_string())
}

fn validated_workspace_key(workspace_root: &Path) -> anyhow::Result<String> {
    let key = normalize_workspace_key(workspace_root);
    if key.is_empty() {
        return Err(StoreError::EmptyWorkspaceRoot.into());
    }
    Ok(key)
}

fn project_workspace_values(
    workspace_root: &Option<PathBuf>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    workspace_root
        .as_ref()
        .map(|path| {
            Ok((
                Some(path.to_string_lossy().into_owned()),
                Some(validated_workspace_key(path)?),
            ))
        })
        .unwrap_or(Ok((None, None)))
}

fn ensure_workspace_available(
    conn: &Connection,
    workspace_key: Option<&str>,
    exclude_project_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let Some(workspace_key) = workspace_key else {
        return Ok(());
    };
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM projects WHERE workspace_key = ?1",
            params![workspace_key],
            |row| row.get(0),
        )
        .optional()?;
    if existing.as_deref()
        != exclude_project_id
            .as_ref()
            .map(|id| id.to_string())
            .as_deref()
        && existing.is_some()
    {
        return Err(StoreError::DuplicateWorkspace(workspace_key.to_string()).into());
    }
    Ok(())
}

fn insert_project(
    conn: &Connection,
    project: &Project,
    workspace_root: Option<&str>,
    workspace_key: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO projects (
            id, name, workspace_root, workspace_key, pinned, sort_order,
            created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            project.id.to_string(),
            &project.name,
            workspace_root,
            workspace_key,
            project.pinned as i64,
            project.sort_order,
            project.created_at.to_rfc3339(),
            project.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn query_project(conn: &Connection, id: Uuid) -> anyhow::Result<Option<Project>> {
    conn.query_row(
        r#"
        SELECT id, name, workspace_root, pinned, sort_order, created_at, updated_at
        FROM projects
        WHERE id = ?1
        "#,
        params![id.to_string()],
        map_project,
    )
    .optional()
    .map_err(Into::into)
}

fn query_project_by_workspace_key(
    conn: &Connection,
    workspace_key: &str,
) -> anyhow::Result<Option<Project>> {
    conn.query_row(
        r#"
        SELECT id, name, workspace_root, pinned, sort_order, created_at, updated_at
        FROM projects
        WHERE workspace_key = ?1
        "#,
        params![workspace_key],
        map_project,
    )
    .optional()
    .map_err(Into::into)
}

fn insert_thread(conn: &Connection, thread: &Thread) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO threads (
            id, title, workspace_root, project_id, archived_at, experience_mode, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            thread.id.to_string(),
            &thread.title,
            thread.workspace_root.to_string_lossy(),
            thread.project_id.map(|id| id.to_string()),
            thread.archived_at.map(|value| value.to_rfc3339()),
            thread.experience_mode.as_str(),
            thread.created_at.to_rfc3339(),
            thread.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn query_thread(conn: &Connection, id: Uuid) -> anyhow::Result<Option<Thread>> {
    conn.query_row(
        r#"
        SELECT id, title, workspace_root, project_id, archived_at, experience_mode, created_at, updated_at
        FROM threads
        WHERE id = ?1
        "#,
        params![id.to_string()],
        map_thread,
    )
    .optional()
    .map_err(Into::into)
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
    let project_id: Option<String> = row.get(3)?;
    let archived_at: Option<String> = row.get(4)?;
    let experience_mode_value: String = row.get(5)?;
    let experience_mode = ExperienceMode::from_str(&experience_mode_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    Ok(Thread {
        id: parse_uuid(row.get(0)?, 0)?,
        title: row.get(1)?,
        workspace_root: PathBuf::from(row.get::<_, String>(2)?),
        project_id: project_id.map(|value| parse_uuid(value, 3)).transpose()?,
        experience_mode,
        archived_at: archived_at
            .map(|value| parse_datetime(value, 4))
            .transpose()?,
        created_at: parse_datetime(row.get(6)?, 6)?,
        updated_at: parse_datetime(row.get(7)?, 7)?,
    })
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: parse_uuid(row.get(0)?, 0)?,
        name: row.get(1)?,
        workspace_root: row.get::<_, Option<String>>(2)?.map(PathBuf::from),
        pinned: row.get::<_, i64>(3)? != 0,
        sort_order: row.get(4)?,
        created_at: parse_datetime(row.get(5)?, 5)?,
        updated_at: parse_datetime(row.get(6)?, 6)?,
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

fn map_turn(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRecord> {
    let status_value: String = row.get(3)?;
    let status = TurnStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let completed_at: Option<String> = row.get(6)?;
    Ok(TurnRecord {
        turn_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        user_message_id: parse_uuid(row.get(2)?, 2)?,
        status,
        started_at: parse_datetime(row.get(4)?, 4)?,
        updated_at: parse_datetime(row.get(5)?, 5)?,
        completed_at: completed_at
            .map(|value| parse_datetime(value, 6))
            .transpose()?,
        error: row.get(7)?,
    })
}

fn valid_goal_transition(from: GoalStatus, to: GoalStatus) -> bool {
    if from == to {
        return true;
    }
    match from {
        GoalStatus::Draft => matches!(
            to,
            GoalStatus::Ready | GoalStatus::Active | GoalStatus::Cancelled | GoalStatus::Failed
        ),
        GoalStatus::Ready => matches!(
            to,
            GoalStatus::Active | GoalStatus::Cancelled | GoalStatus::Failed
        ),
        GoalStatus::Active => matches!(
            to,
            GoalStatus::Paused
                | GoalStatus::Blocked
                | GoalStatus::Completed
                | GoalStatus::Cancelled
                | GoalStatus::Failed
        ),
        GoalStatus::Paused | GoalStatus::Blocked => {
            matches!(
                to,
                GoalStatus::Active | GoalStatus::Cancelled | GoalStatus::Failed
            )
        }
        GoalStatus::Completed | GoalStatus::Cancelled | GoalStatus::Failed => false,
    }
}

fn query_goal(conn: &Connection, id: Uuid) -> anyhow::Result<Option<GoalRecord>> {
    conn.query_row(
        r#"
        SELECT id, thread_id, objective, status, plan_revision, token_budget,
               tokens_used, time_used_seconds, version, created_at, updated_at, completed_at
        FROM goals
        WHERE id = ?1
        "#,
        params![id.to_string()],
        map_goal,
    )
    .optional()
    .map_err(Into::into)
}

fn load_goal_snapshot(conn: &Connection, id: Uuid) -> anyhow::Result<Option<GoalSnapshot>> {
    let Some(goal) = query_goal(conn, id)? else {
        return Ok(None);
    };
    let mut task_stmt = conn.prepare(
        r#"
        SELECT goal_id, step_id, ordinal, title, status, status_reason,
               dependencies_json, acceptance_criteria_json, evidence_json,
               attempt_count, max_attempts, updated_at
        FROM goal_tasks
        WHERE goal_id = ?1
        ORDER BY ordinal ASC, rowid ASC
        "#,
    )?;
    let tasks = collect_rows(task_stmt.query_map(params![id.to_string()], map_goal_task)?)?;
    let mut attempt_stmt = conn.prepare(
        r#"
        SELECT id, goal_id, step_id, turn_id, attempt_no, status,
               started_at, finished_at, evidence_json, error
        FROM goal_task_attempts
        WHERE goal_id = ?1
        ORDER BY started_at ASC, rowid ASC
        "#,
    )?;
    let attempts =
        collect_rows(attempt_stmt.query_map(params![id.to_string()], map_goal_task_attempt)?)?;
    Ok(Some(GoalSnapshot {
        goal,
        tasks,
        attempts,
    }))
}

fn query_goal_task_states(
    conn: &Connection,
    goal_id: Uuid,
) -> anyhow::Result<HashMap<String, (GoalTaskStatus, u32, u32)>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT step_id, status, attempt_count, max_attempts
        FROM goal_tasks
        WHERE goal_id = ?1
        "#,
    )?;
    let rows = stmt.query_map(params![goal_id.to_string()], |row| {
        let raw_status: String = row.get(1)?;
        let status = GoalTaskStatus::from_str(&raw_status)
            .map_err(|error| invalid_column(1, error.to_string()))?;
        let attempts = u32::try_from(row.get::<_, i64>(2)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(2, Type::Integer, Box::new(error))
        })?;
        let max_attempts = u32::try_from(row.get::<_, i64>(3)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(3, Type::Integer, Box::new(error))
        })?;
        Ok((row.get::<_, String>(0)?, (status, attempts, max_attempts)))
    })?;
    collect_rows(rows).map(|rows| rows.into_iter().collect())
}

fn map_goal(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalRecord> {
    let raw_status: String = row.get(3)?;
    let status =
        GoalStatus::from_str(&raw_status).map_err(|error| invalid_column(3, error.to_string()))?;
    let token_budget = row
        .get::<_, Option<i64>>(5)?
        .map(|value| u64::try_from(value))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Integer, Box::new(error))
        })?;
    let completed_at: Option<String> = row.get(11)?;
    Ok(GoalRecord {
        id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        objective: row.get(2)?,
        status,
        plan_revision: parse_u64(row.get(4)?, 4)?,
        token_budget,
        tokens_used: parse_u64(row.get(6)?, 6)?,
        time_used_seconds: parse_u64(row.get(7)?, 7)?,
        version: parse_u64(row.get(8)?, 8)?,
        created_at: parse_datetime(row.get(9)?, 9)?,
        updated_at: parse_datetime(row.get(10)?, 10)?,
        completed_at: completed_at
            .map(|value| parse_datetime(value, 11))
            .transpose()?,
    })
}

fn map_goal_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalTask> {
    let raw_status: String = row.get(4)?;
    let status = GoalTaskStatus::from_str(&raw_status)
        .map_err(|error| invalid_column(4, error.to_string()))?;
    let dependencies_json: String = row.get(6)?;
    let acceptance_json: String = row.get(7)?;
    let evidence_json: String = row.get(8)?;
    Ok(GoalTask {
        goal_id: parse_uuid(row.get(0)?, 0)?,
        step_id: row.get(1)?,
        ordinal: usize::try_from(row.get::<_, i64>(2)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(2, Type::Integer, Box::new(error))
        })?,
        title: row.get(3)?,
        status,
        status_reason: row.get(5)?,
        dependencies: parse_json_column(&dependencies_json, 6)?,
        acceptance_criteria: parse_json_column(&acceptance_json, 7)?,
        evidence: parse_json_column(&evidence_json, 8)?,
        attempt_count: u32::try_from(row.get::<_, i64>(9)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(9, Type::Integer, Box::new(error))
        })?,
        max_attempts: u32::try_from(row.get::<_, i64>(10)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(10, Type::Integer, Box::new(error))
        })?,
        updated_at: parse_datetime(row.get(11)?, 11)?,
    })
}

fn map_goal_task_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<GoalTaskAttempt> {
    let raw_status: String = row.get(5)?;
    let status = GoalAttemptStatus::from_str(&raw_status)
        .map_err(|error| invalid_column(5, error.to_string()))?;
    let finished_at: Option<String> = row.get(7)?;
    let evidence_json: String = row.get(8)?;
    Ok(GoalTaskAttempt {
        id: parse_uuid(row.get(0)?, 0)?,
        goal_id: parse_uuid(row.get(1)?, 1)?,
        step_id: row.get(2)?,
        turn_id: parse_uuid(row.get(3)?, 3)?,
        attempt_no: u32::try_from(row.get::<_, i64>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Integer, Box::new(error))
        })?,
        status,
        started_at: parse_datetime(row.get(6)?, 6)?,
        finished_at: finished_at
            .map(|value| parse_datetime(value, 7))
            .transpose()?,
        evidence: parse_json_column(&evidence_json, 8)?,
        error: row.get(9)?,
    })
}

fn parse_json_column<T: serde::de::DeserializeOwned>(
    value: &str,
    column: usize,
) -> rusqlite::Result<T> {
    serde_json::from_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

fn map_turn_change_set(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnChangeSet> {
    let status_value: String = row.get(7)?;
    let status = TurnChangeSetStatus::from_str(&status_value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            7,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;
    let files_json: String = row.get(8)?;
    let additions: i64 = row.get(9)?;
    let deletions: i64 = row.get(10)?;
    let finalized_at: Option<String> = row.get(13)?;
    let reverted_at: Option<String> = row.get(14)?;
    Ok(TurnChangeSet {
        turn_id: parse_uuid(row.get(0)?, 0)?,
        thread_id: parse_uuid(row.get(1)?, 1)?,
        workspace_root: PathBuf::from(row.get::<_, String>(2)?),
        repo_root: row.get::<_, Option<String>>(3)?.map(PathBuf::from),
        workspace_prefix: row.get::<_, Option<String>>(4)?.map(PathBuf::from),
        before_tree: row.get(5)?,
        after_tree: row.get(6)?,
        status,
        files: serde_json::from_str(&files_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(8, Type::Text, Box::new(err))
        })?,
        additions: u64::try_from(additions).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(9, Type::Integer, Box::new(err))
        })?,
        deletions: u64::try_from(deletions).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(10, Type::Integer, Box::new(err))
        })?,
        error: row.get(11)?,
        created_at: parse_datetime(row.get(12)?, 12)?,
        finalized_at: finalized_at
            .map(|value| parse_datetime(value, 13))
            .transpose()?,
        reverted_at: reverted_at
            .map(|value| parse_datetime(value, 14))
            .transpose()?,
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

fn map_subagent_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubagentRun> {
    let depth_value: i64 = row.get(10)?;
    let depth = u8::try_from(depth_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(10, Type::Integer, Box::new(error))
    })?;
    let status_value: String = row.get(11)?;
    let status = SubagentRunStatus::parse(&status_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            11,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })?;
    let started_at: Option<String> = row.get(15)?;
    let completed_at: Option<String> = row.get(16)?;
    Ok(SubagentRun {
        id: parse_uuid(row.get(0)?, 0)?,
        parent_thread_id: parse_uuid(row.get(1)?, 1)?,
        parent_turn_id: parse_uuid(row.get(2)?, 2)?,
        agent_path: row.get(3)?,
        parent_agent_path: row.get(4)?,
        name: row.get(5)?,
        agent_type: row.get(6)?,
        input: row.get(7)?,
        fork_turns: row.get(8)?,
        last_task_message: row.get(9)?,
        depth,
        status,
        result: row.get(12)?,
        error: row.get(13)?,
        created_at: parse_datetime(row.get(14)?, 14)?,
        started_at: started_at
            .map(|value| parse_datetime(value, 15))
            .transpose()?,
        completed_at: completed_at
            .map(|value| parse_datetime(value, 16))
            .transpose()?,
        initial_conversation: Vec::new(),
        initial_model_context: None,
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

fn map_user_input_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserInputRecord> {
    let request_id = parse_uuid(row.get(0)?, 0)?;
    let thread_id = parse_uuid(row.get(1)?, 1)?;
    let request_json: String = row.get(2)?;
    let mut request: UserInputRequest = serde_json::from_str(&request_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(2, Type::Text, Box::new(error))
    })?;
    request.request_id = request_id;
    let status_value: String = row.get(3)?;
    let status = UserInputStatus::from_str(&status_value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        )
    })?;
    let response_json: Option<String> = row.get(4)?;
    let response = response_json
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })
        })
        .transpose()?;
    let answered_at: Option<String> = row.get(6)?;
    Ok(UserInputRecord {
        thread_id,
        request,
        status,
        response,
        created_at: parse_datetime(row.get(5)?, 5)?,
        answered_at: answered_at
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
        plugin_id: row.get(8)?,
        plugin_server_name: row.get(9)?,
        created_at: parse_datetime(row.get(10)?, 10)?,
        updated_at: parse_datetime(row.get(11)?, 11)?,
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
    use crate::model::{TaskPlanStep, TaskPlanStepStatus, TurnFileChange, TurnFileChangeKind};

    fn temporary_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "opentopia-{label}-{}-{}.db",
            std::process::id(),
            Uuid::new_v4()
        ))
    }

    fn remove_sqlite_files(path: &Path) {
        let base = path.to_string_lossy();
        for candidate in [
            base.to_string(),
            format!("{base}-wal"),
            format!("{base}-shm"),
        ] {
            let _ = std::fs::remove_file(candidate);
        }
    }

    #[test]
    fn user_input_request_persists_continuation_and_answer() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/user-input"))
            .expect("create thread");
        let request = UserInputRequest {
            request_id: Uuid::new_v4(),
            questions: vec![crate::model::UserInputQuestion {
                id: "architecture".to_string(),
                header: "Architecture".to_string(),
                question: "Which architecture should be planned?".to_string(),
                options: vec![
                    crate::model::UserInputOption {
                        id: "modular".to_string(),
                        label: "Modular".to_string(),
                        description: "Keep explicit boundaries.".to_string(),
                        recommended: true,
                    },
                    crate::model::UserInputOption {
                        id: "minimal".to_string(),
                        label: "Minimal".to_string(),
                        description: "Keep the change compact.".to_string(),
                        recommended: false,
                    },
                ],
                allow_custom: true,
            }],
        };
        store
            .put_user_input_request(thread.id, &request, serde_json::json!({"resume": true}))
            .expect("persist request");

        let pending = store
            .list_user_input_requests(thread.id, Some(UserInputStatus::Pending))
            .expect("list pending");
        assert_eq!(pending.len(), 1);
        assert!(store
            .get_user_input_continuation(request.request_id, thread.id)
            .expect("load continuation")
            .is_some());

        let response = UserInputResponse {
            answers: vec![crate::model::UserInputAnswer {
                question_id: "architecture".to_string(),
                option_id: Some("modular".to_string()),
                custom_text: None,
            }],
        };
        let answered = store
            .resolve_user_input_request(request.request_id, thread.id, &response)
            .expect("resolve request")
            .expect("request exists");
        assert_eq!(answered.status, UserInputStatus::Answered);
        assert_eq!(answered.response, Some(response));
        assert!(store
            .list_user_input_requests(thread.id, Some(UserInputStatus::Pending))
            .expect("list pending after answer")
            .is_empty());
        assert!(store
            .get_user_input_continuation(request.request_id, thread.id)
            .expect("continuation cleared")
            .is_none());
    }

    #[test]
    fn sqlite_store_round_trips_reasoning_delta_events() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/reasoning-events"))
            .expect("create thread");
        let turn_id = Uuid::new_v4();
        let event = AgentEvent::new(
            thread.id,
            Some(turn_id),
            0,
            AgentEventPayload::ReasoningDelta {
                text: "正在核对依赖".to_string(),
            },
        );

        let stored = store.append_event(event).expect("append reasoning event");
        assert_eq!(stored.kind(), "reasoning_delta");

        let events = store.list_events(thread.id, None).expect("list events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].turn_id, Some(turn_id));
        match &events[0].payload {
            AgentEventPayload::ReasoningDelta { text } => {
                assert_eq!(text, "正在核对依赖");
            }
            payload => panic!("unexpected payload: {payload:?}"),
        }
    }

    #[test]
    fn workspace_keys_normalize_windows_drive_and_unc_paths() {
        let drive = normalize_workspace_key(Path::new(r"J:\Project\OpenTopia\"));
        assert_eq!(drive, "j:/project/opentopia");
        assert_eq!(
            drive,
            normalize_workspace_key(Path::new(r"\\?\j:\PROJECT\OpenTopia"))
        );
        assert_eq!(
            drive,
            normalize_workspace_key(Path::new("J:/Project/./Scratch/../OpenTopia/"))
        );

        let unc = normalize_workspace_key(Path::new(r"\\Server\Share\Repo\"));
        assert_eq!(unc, "//server/share/repo");
        assert_eq!(
            unc,
            normalize_workspace_key(Path::new(r"\\?\UNC\server\SHARE\repo"))
        );
        assert_ne!(
            normalize_workspace_key(Path::new("/srv/Repo")),
            normalize_workspace_key(Path::new("/srv/repo"))
        );
    }

    #[test]
    fn queued_turn_messages_are_persisted_and_removed() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/turn-queue"))
            .expect("create thread");
        let first = store
            .append_message(Message::text(thread.id, MessageRole::User, "first"))
            .expect("append first message");
        let second = store
            .append_message(Message::text(thread.id, MessageRole::User, "second"))
            .expect("append second message");

        store
            .enqueue_turn_message(thread.id, first.id)
            .expect("enqueue first message");
        store
            .enqueue_turn_message(thread.id, second.id)
            .expect("enqueue second message");

        assert_eq!(
            store
                .list_queued_turn_messages(thread.id)
                .expect("list queued messages"),
            vec![first.id, second.id]
        );
        assert!(store
            .remove_queued_turn_message(thread.id, first.id)
            .expect("remove first message"));
        assert_eq!(
            store
                .list_queued_turn_messages(thread.id)
                .expect("list remaining messages"),
            vec![second.id]
        );
    }

    #[test]
    fn goal_plan_projection_tracks_attempts_and_completion() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/goal-projection"))
            .expect("create thread");
        let created = store
            .create_goal(
                thread.id,
                "Ship durable goal execution".to_string(),
                GoalStatus::Draft,
                Some(50_000),
            )
            .expect("create goal");
        let goal_id = created.goal.id;
        let turn_id = Uuid::new_v4();
        let plan = |revision, status, evidence: Vec<&str>| TaskPlan {
            plan_revision: revision,
            goal_id: goal_id.to_string(),
            change_reason: Some(format!("revision {revision}")),
            steps: vec![TaskPlanStep {
                id: "implement".to_string(),
                title: "Implement and verify".to_string(),
                status,
                status_reason: None,
                dependencies: Vec::new(),
                acceptance_criteria: vec!["tests pass".to_string()],
                evidence: evidence.into_iter().map(str::to_string).collect(),
            }],
        };

        let ready = store
            .apply_goal_plan(
                thread.id,
                turn_id,
                &plan(1, TaskPlanStepStatus::Pending, vec![]),
            )
            .expect("project draft plan");
        assert_eq!(ready.goal.status, GoalStatus::Ready);
        assert_eq!(ready.tasks[0].status, GoalTaskStatus::Pending);

        store
            .update_goal_status(thread.id, goal_id, GoalStatus::Active)
            .expect("activate goal");
        let running = store
            .apply_goal_plan(
                thread.id,
                turn_id,
                &plan(2, TaskPlanStepStatus::InProgress, vec![]),
            )
            .expect("start task");
        assert_eq!(running.tasks[0].attempt_count, 1);
        assert_eq!(running.attempts.len(), 1);
        assert_eq!(running.attempts[0].status, GoalAttemptStatus::Running);

        let completed = store
            .apply_goal_plan(
                thread.id,
                turn_id,
                &plan(3, TaskPlanStepStatus::Completed, vec!["cargo test passed"]),
            )
            .expect("complete task");
        assert_eq!(completed.goal.status, GoalStatus::Completed);
        assert_eq!(completed.completed_tasks(), 1);
        assert_eq!(completed.attempts[0].status, GoalAttemptStatus::Succeeded);
        assert_eq!(completed.attempts[0].evidence, vec!["cargo test passed"]);
    }

    #[test]
    fn goal_recovery_interrupts_running_attempts_without_replaying_them() {
        let path = temporary_db_path("goal-recovery");
        let (thread_id, goal_id) = {
            let store = SqliteSessionStore::open(&path).expect("open goal store");
            let thread = store
                .create_thread(None, PathBuf::from("C:/workspace/goal-recovery"))
                .expect("create thread");
            let goal = store
                .create_goal(
                    thread.id,
                    "Recover safely".to_string(),
                    GoalStatus::Active,
                    None,
                )
                .expect("create goal");
            let plan = TaskPlan {
                plan_revision: 1,
                goal_id: goal.goal.id.to_string(),
                change_reason: Some("start".to_string()),
                steps: vec![TaskPlanStep {
                    id: "side-effect".to_string(),
                    title: "Perform side effect".to_string(),
                    status: TaskPlanStepStatus::InProgress,
                    status_reason: None,
                    dependencies: Vec::new(),
                    acceptance_criteria: vec!["effect observed".to_string()],
                    evidence: Vec::new(),
                }],
            };
            store
                .apply_goal_plan(thread.id, Uuid::new_v4(), &plan)
                .expect("start attempt");
            (thread.id, goal.goal.id)
        };

        let recovered = SqliteSessionStore::open(&path).expect("reopen goal store");
        let snapshot = recovered
            .get_goal(goal_id)
            .expect("load goal")
            .expect("goal exists");
        assert_eq!(snapshot.goal.thread_id, thread_id);
        assert_eq!(snapshot.goal.status, GoalStatus::Blocked);
        assert_eq!(snapshot.tasks[0].status, GoalTaskStatus::Blocked);
        assert_eq!(snapshot.attempts[0].status, GoalAttemptStatus::Interrupted);
        drop(recovered);
        remove_sqlite_files(&path);
    }

    #[test]
    fn context_budget_uses_unicode_aware_token_estimates() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/context-budget"))
            .expect("create thread");
        store
            .append_message(Message::text(
                thread.id,
                MessageRole::User,
                "\u{4f60}\u{597d}\u{4e16}\u{754c}",
            ))
            .expect("append non-ASCII message");

        let budget = store
            .get_context_budget(thread.id)
            .expect("calculate context budget");
        assert_eq!(budget.message_count, 1);
        assert_eq!(budget.used_tokens, 54);
    }

    #[test]
    fn turn_lifecycle_round_trips_and_returns_latest_record() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/turn-lifecycle"))
            .expect("create thread");
        let first = store
            .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
            .expect("insert running turn");

        assert_eq!(
            store
                .get_active_turn(thread.id)
                .expect("get active turn")
                .expect("active turn")
                .turn_id,
            first.turn_id
        );
        let waiting = store
            .update_turn_status(first.turn_id, TurnStatus::WaitingApproval, None)
            .expect("pause turn")
            .expect("updated turn");
        assert_eq!(waiting.status, TurnStatus::WaitingApproval);
        assert!(waiting.completed_at.is_none());
        assert!(store
            .get_active_turn(thread.id)
            .expect("get active turn")
            .is_none());

        let second = store
            .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
            .expect("insert resumed turn");
        let succeeded = store
            .update_turn_status(second.turn_id, TurnStatus::Succeeded, None)
            .expect("finish turn")
            .expect("updated turn");
        assert!(succeeded.completed_at.is_some());
        assert_eq!(
            store
                .get_latest_turn(thread.id)
                .expect("get latest turn")
                .expect("latest turn")
                .turn_id,
            second.turn_id
        );
    }

    #[test]
    fn turn_change_sets_round_trip_and_can_be_marked_reverted() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(None, PathBuf::from("C:/workspace/turn-changes"))
            .expect("create thread");
        let turn = store
            .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
            .expect("insert turn");
        let mut change_set =
            TurnChangeSet::capturing(turn.turn_id, thread.id, thread.workspace_root.clone());
        change_set.repo_root = Some(PathBuf::from("C:/workspace/turn-changes"));
        change_set.workspace_prefix = Some(PathBuf::from("."));
        change_set.before_tree = Some("before-tree".to_string());
        change_set.after_tree = Some("after-tree".to_string());
        change_set.status = TurnChangeSetStatus::Ready;
        change_set.files = vec![TurnFileChange {
            kind: TurnFileChangeKind::Modified,
            old_path: Some(PathBuf::from("src/main.rs")),
            new_path: Some(PathBuf::from("src/main.rs")),
            before_oid: Some("before-oid".to_string()),
            after_oid: Some("after-oid".to_string()),
            before_mode: Some("100644".to_string()),
            after_mode: Some("100644".to_string()),
            additions: Some(3),
            deletions: Some(1),
            binary: false,
        }];
        change_set.additions = 3;
        change_set.deletions = 1;
        change_set.finalized_at = Some(Utc::now());

        store
            .upsert_turn_change_set(&change_set)
            .expect("store turn changes");
        let loaded = store
            .get_turn_change_set(turn.turn_id)
            .expect("load turn changes")
            .expect("turn changes exist");
        assert_eq!(loaded.status, TurnChangeSetStatus::Ready);
        assert_eq!(loaded.files, change_set.files);
        assert_eq!(
            store
                .list_turn_change_sets(thread.id)
                .expect("list turn changes"),
            vec![loaded.clone()]
        );

        let reverted_at = Utc::now();
        let reverted = store
            .mark_turn_change_set_reverted(turn.turn_id, reverted_at)
            .expect("mark reverted")
            .expect("turn changes exist");
        assert_eq!(reverted.reverted_at, Some(reverted_at));
    }

    #[test]
    fn startup_recovery_interrupts_only_active_turns() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let first_thread = store
            .create_thread(None, PathBuf::from("C:/workspace/interrupted-running"))
            .expect("create first thread");
        let second_thread = store
            .create_thread(None, PathBuf::from("C:/workspace/interrupted-cancelling"))
            .expect("create second thread");
        let third_thread = store
            .create_thread(None, PathBuf::from("C:/workspace/waiting-approval"))
            .expect("create third thread");
        let running = store
            .insert_turn(TurnRecord::running(first_thread.id, Uuid::new_v4()))
            .expect("insert running turn");
        let cancelling = store
            .insert_turn(TurnRecord::running(second_thread.id, Uuid::new_v4()))
            .expect("insert cancelling turn");
        store
            .update_turn_status(cancelling.turn_id, TurnStatus::Cancelling, None)
            .expect("mark cancelling");
        let waiting = store
            .insert_turn(TurnRecord::running(third_thread.id, Uuid::new_v4()))
            .expect("insert waiting turn");
        store
            .update_turn_status(waiting.turn_id, TurnStatus::WaitingApproval, None)
            .expect("mark waiting");

        assert_eq!(store.interrupt_active_turns().expect("recover turns"), 2);
        for turn_id in [running.turn_id, cancelling.turn_id] {
            let recovered = store
                .get_turn(turn_id)
                .expect("get recovered turn")
                .expect("recovered turn");
            assert_eq!(recovered.status, TurnStatus::Interrupted);
            assert!(recovered.completed_at.is_some());
            assert!(recovered.error.is_some());
        }
        assert_eq!(
            store
                .get_turn(waiting.turn_id)
                .expect("get waiting turn")
                .expect("waiting turn")
                .status,
            TurnStatus::WaitingApproval
        );
    }

    #[test]
    fn project_crud_validates_names_and_duplicate_workspaces() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let empty_name = store
            .create_project("   ".to_string(), None, false, 0)
            .expect_err("empty project name should fail");
        assert!(matches!(
            empty_name.downcast_ref::<StoreError>(),
            Some(StoreError::EmptyProjectName)
        ));

        let project = store
            .create_project(
                " OpenTopia ".to_string(),
                Some(PathBuf::from(r"J:\Project\OpenTopia")),
                false,
                7,
            )
            .expect("create project");
        assert_eq!(project.name, "OpenTopia");
        let duplicate = store
            .create_project(
                "Duplicate".to_string(),
                Some(PathBuf::from(r"\\?\j:\project\opentopia\")),
                false,
                8,
            )
            .expect_err("equivalent workspace should fail");
        assert!(matches!(
            duplicate.downcast_ref::<StoreError>(),
            Some(StoreError::DuplicateWorkspace(_))
        ));
        let found = store
            .find_or_create_project(
                "Ignored duplicate name".to_string(),
                PathBuf::from(r"j:/PROJECT/OpenTopia/"),
            )
            .expect("find existing project");
        assert_eq!(found.id, project.id);

        let updated = store
            .update_project(
                project.id,
                Some("Renamed".to_string()),
                Some(None),
                Some(true),
                Some(1),
            )
            .expect("update project")
            .expect("project exists");
        assert_eq!(updated.name, "Renamed");
        assert!(updated.workspace_root.is_none());
        assert!(updated.pinned);
        assert_eq!(updated.sort_order, 1);
        assert_eq!(store.list_projects().expect("list projects").len(), 1);
        assert!(store
            .update_project(Uuid::new_v4(), None, None, None, None)
            .expect("update missing project")
            .is_none());
    }

    #[test]
    fn project_and_thread_json_use_camel_case_nullable_fields() {
        let project = Project::new("OpenTopia", None);
        let project_json = serde_json::to_value(&project).expect("serialize project");
        assert_eq!(project_json["workspaceRoot"], Value::Null);
        assert_eq!(project_json["sortOrder"], 0);
        assert!(project_json.get("createdAt").is_some());
        assert!(project_json.get("workspace_root").is_none());

        let thread =
            Thread::new_in_project("Thread", PathBuf::from(r"J:\Project\OpenTopia"), project.id);
        let thread_json = serde_json::to_value(&thread).expect("serialize thread");
        assert_eq!(thread_json["projectId"], project.id.to_string());
        assert_eq!(thread_json["experienceMode"], "code");
        assert_eq!(thread_json["archivedAt"], Value::Null);
        assert!(thread_json.get("project_id").is_none());
    }

    #[test]
    fn thread_experience_mode_defaults_to_code_and_round_trips_work() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let project = store
            .create_project(
                "OpenTopia".to_string(),
                Some(PathBuf::from(r"J:\Project\OpenTopia")),
                false,
                0,
            )
            .expect("create project");

        let code_thread = store
            .create_thread_in_project(Some("Code task".to_string()), project.id)
            .expect("create code thread");
        assert_eq!(code_thread.experience_mode, ExperienceMode::Code);

        let work_thread = store
            .create_thread_in_project_with_mode(
                Some("Work task".to_string()),
                project.id,
                ExperienceMode::Work,
            )
            .expect("create work thread");
        let loaded = store
            .get_thread(work_thread.id)
            .expect("load work thread")
            .expect("work thread exists");
        assert_eq!(loaded.experience_mode, ExperienceMode::Work);
    }

    #[test]
    fn project_thread_lifecycle_preserves_ownership_workspace_and_history() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let project = store
            .create_project(
                "OpenTopia".to_string(),
                Some(PathBuf::from(r"J:\Project\OpenTopia")),
                false,
                0,
            )
            .expect("create project");
        let thread = store
            .create_thread_in_project(Some("First".to_string()), project.id)
            .expect("create project thread");
        assert_eq!(thread.project_id, Some(project.id));
        assert_eq!(
            thread.workspace_root,
            PathBuf::from(r"J:\Project\OpenTopia")
        );

        store
            .append_message(Message::text(thread.id, MessageRole::User, "hello"))
            .expect("append message");

        let moved_workspace = PathBuf::from(r"J:\Project\OpenTopia-next");
        let updated_project = store
            .update_project(
                project.id,
                None,
                Some(Some(moved_workspace.clone())),
                None,
                None,
            )
            .expect("update project workspace")
            .expect("project exists");
        assert_eq!(
            updated_project.workspace_root,
            Some(moved_workspace.clone())
        );
        assert_eq!(
            store
                .get_thread(thread.id)
                .expect("read synchronized thread")
                .expect("thread exists")
                .workspace_root,
            moved_workspace
        );

        let clear_error = store
            .update_project(project.id, None, Some(None), None, None)
            .expect_err("owned threads require a project workspace");
        assert!(matches!(
            clear_error.downcast_ref::<StoreError>(),
            Some(StoreError::ProjectWorkspaceInUse(id)) if *id == project.id
        ));

        let archived = store
            .update_thread(thread.id, Some("Renamed".to_string()), None, Some(true))
            .expect("archive thread")
            .expect("thread exists");
        assert_eq!(archived.title, "Renamed");
        assert!(archived.archived_at.is_some());
        assert!(store
            .list_threads()
            .expect("list active threads")
            .is_empty());
        assert_eq!(
            store
                .list_threads_including_archived(true)
                .expect("list all threads")
                .len(),
            1
        );

        let restored = store
            .update_thread(thread.id, None, None, Some(false))
            .expect("restore thread")
            .expect("thread exists");
        assert!(restored.archived_at.is_none());
        assert_eq!(store.list_threads().expect("list active threads").len(), 1);

        assert!(store.delete_project(project.id).expect("delete project"));
        let detached = store
            .get_thread(thread.id)
            .expect("get detached thread")
            .expect("thread remains");
        assert!(detached.project_id.is_none());
        assert!(detached.archived_at.is_some());
        assert!(store
            .list_threads()
            .expect("list active threads")
            .is_empty());
        assert_eq!(
            store
                .list_messages(thread.id)
                .expect("messages remain")
                .len(),
            1
        );

        let replacement = store
            .create_project(
                "OpenTopia restored".to_string(),
                Some(PathBuf::from(r"J:\Project\OpenTopia-restored")),
                false,
                0,
            )
            .expect("create replacement project");
        let recovered = store
            .update_thread(thread.id, None, Some(Some(replacement.id)), Some(false))
            .expect("reassign and restore thread")
            .expect("thread exists");
        assert_eq!(recovered.project_id, Some(replacement.id));
        assert_eq!(
            recovered.workspace_root,
            PathBuf::from(r"J:\Project\OpenTopia-restored")
        );
        assert!(recovered.archived_at.is_none());

        assert!(store.delete_thread(thread.id).expect("delete thread"));
        assert!(store
            .get_thread(thread.id)
            .expect("get deleted thread")
            .is_none());
        assert!(store
            .list_messages(thread.id)
            .expect("messages cascade")
            .is_empty());
    }

    #[test]
    fn thread_reassignment_validates_target_project() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let source = store
            .create_project(
                "Source".to_string(),
                Some(PathBuf::from(r"J:\Project\Source")),
                false,
                0,
            )
            .expect("create source project");
        let target = store
            .create_project(
                "Target".to_string(),
                Some(PathBuf::from(r"J:\Project\Target")),
                false,
                1,
            )
            .expect("create target project");
        let empty_target = store
            .create_project("Empty".to_string(), None, false, 2)
            .expect("create workspace-less project");
        let thread = store
            .create_thread_in_project(Some("Move me".to_string()), source.id)
            .expect("create thread");

        let missing_id = Uuid::new_v4();
        let missing = store
            .update_thread(thread.id, None, Some(Some(missing_id)), None)
            .expect_err("missing project should fail");
        assert!(matches!(
            missing.downcast_ref::<StoreError>(),
            Some(StoreError::ProjectNotFound(id)) if *id == missing_id
        ));

        let no_workspace = store
            .update_thread(thread.id, None, Some(Some(empty_target.id)), None)
            .expect_err("workspace-less project should fail");
        assert!(matches!(
            no_workspace.downcast_ref::<StoreError>(),
            Some(StoreError::ProjectHasNoWorkspace(id)) if *id == empty_target.id
        ));

        let moved = store
            .update_thread(thread.id, None, Some(Some(target.id)), None)
            .expect("reassign thread")
            .expect("thread exists");
        assert_eq!(moved.project_id, Some(target.id));
        assert_eq!(moved.workspace_root, PathBuf::from(r"J:\Project\Target"));

        let detached = store
            .update_thread(thread.id, None, Some(None), None)
            .expect("detach thread")
            .expect("thread exists");
        assert!(detached.project_id.is_none());
        assert_eq!(detached.workspace_root, moved.workspace_root);
    }

    #[test]
    fn sqlite_store_persists_and_recovers_subagent_runs() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(Some("Parent".to_string()), PathBuf::from("."))
            .expect("create thread");
        let run = SubagentRun {
            id: Uuid::new_v4(),
            parent_thread_id: thread.id,
            parent_turn_id: Uuid::new_v4(),
            agent_path: "/root/reviewer".to_string(),
            parent_agent_path: "/root".to_string(),
            name: "reviewer".to_string(),
            agent_type: "default".to_string(),
            input: "review changes".to_string(),
            fork_turns: "all".to_string(),
            last_task_message: "review changes".to_string(),
            depth: 1,
            status: SubagentRunStatus::Running,
            result: None,
            error: None,
            created_at: Utc::now(),
            started_at: Some(Utc::now()),
            completed_at: None,
            initial_conversation: Vec::new(),
            initial_model_context: None,
        };
        store.upsert_subagent_run(&run).expect("persist run");
        let conversation = vec![ModelConversationMessage {
            role: crate::provider::ModelConversationRole::User,
            content: "continue".to_string(),
            content_parts: Vec::new(),
        }];
        store
            .save_subagent_conversation(run.id, &conversation)
            .expect("persist agent conversation");
        assert_eq!(
            store.load_subagent_conversation(run.id).unwrap().unwrap(),
            conversation
        );
        assert_eq!(
            store.get_subagent_run(run.id).unwrap().unwrap().status,
            SubagentRunStatus::Running
        );
        assert_eq!(store.list_subagent_runs(thread.id).unwrap().len(), 1);
        assert_eq!(store.fail_interrupted_subagent_runs().unwrap(), 1);
        let recovered = store.get_subagent_run(run.id).unwrap().unwrap();
        assert_eq!(recovered.status, SubagentRunStatus::Failed);
        assert!(recovered.error.unwrap().contains("restarted"));
        assert!(recovered.completed_at.is_some());
    }

    #[test]
    fn provider_conversation_state_is_consumed_before_a_turn() {
        let store = SqliteSessionStore::open(":memory:").expect("open memory store");
        let thread = store
            .create_thread(Some("Stateful".to_string()), PathBuf::from("."))
            .expect("create thread");
        let state = ProviderConversationState {
            thread_id: thread.id,
            agent_path: "/root".to_string(),
            provider_id: "openai".to_string(),
            model: "gpt-test".to_string(),
            response_id: "resp_123".to_string(),
            compatibility_hash: "compatible".to_string(),
            updated_at: Utc::now(),
        };

        store
            .save_provider_conversation_state(&state)
            .expect("save state");
        assert_eq!(
            store
                .take_provider_conversation_state(thread.id, "/root")
                .expect("take state"),
            Some(state)
        );
        assert!(store
            .take_provider_conversation_state(thread.id, "/root")
            .expect("state remains consumed")
            .is_none());
    }

    #[test]
    fn migration_deduplicates_legacy_thread_workspaces() {
        let path = temporary_db_path("project-migration");
        let now = Utc::now().to_rfc3339();
        {
            let conn = Connection::open(&path).expect("open legacy database");
            conn.execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    workspace_root TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                "#,
            )
            .expect("create legacy schema");
            for (title, workspace_root) in [
                ("drive-a", r"J:\Project\OpenTopia\"),
                ("drive-b", r"\\?\j:\PROJECT\OpenTopia"),
                ("unc-a", r"\\Server\Share\Repo\"),
                ("unc-b", r"\\?\UNC\server\SHARE\repo"),
            ] {
                conn.execute(
                    r#"
                    INSERT INTO threads (id, title, workspace_root, created_at, updated_at)
                    VALUES (?1, ?2, ?3, ?4, ?4)
                    "#,
                    params![Uuid::new_v4().to_string(), title, workspace_root, &now],
                )
                .expect("insert legacy thread");
            }
        }

        let detached_project_id;
        {
            let store = SqliteSessionStore::open(&path).expect("migrate database");
            let projects = store.list_projects().expect("list migrated projects");
            assert_eq!(projects.len(), 2);
            let threads = store
                .list_threads_including_archived(true)
                .expect("list migrated threads");
            assert_eq!(threads.len(), 4);
            assert!(threads
                .iter()
                .all(|thread| thread.experience_mode == ExperienceMode::Code));
            let mut project_counts = HashMap::new();
            for thread in threads {
                *project_counts
                    .entry(thread.project_id.expect("migrated project id"))
                    .or_insert(0) += 1;
            }
            assert_eq!(project_counts.len(), 2);
            assert!(project_counts.values().all(|count| *count == 2));

            detached_project_id = projects[0].id;
            assert!(store
                .delete_project(detached_project_id)
                .expect("delete migrated project"));
        }

        {
            let reopened = SqliteSessionStore::open(&path).expect("reopen migrated database");
            assert_eq!(reopened.list_projects().expect("list projects").len(), 1);
            assert_eq!(
                reopened
                    .list_threads_including_archived(true)
                    .expect("list threads")
                    .iter()
                    .filter(|thread| thread.project_id.is_none())
                    .count(),
                2
            );
            assert_eq!(
                reopened
                    .list_threads_including_archived(true)
                    .expect("list archived threads")
                    .iter()
                    .filter(|thread| thread.project_id.is_none() && thread.archived_at.is_some())
                    .count(),
                2
            );
            assert!(reopened
                .get_project(detached_project_id)
                .expect("get deleted project")
                .is_none());
        }
        remove_sqlite_files(&path);
    }

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
