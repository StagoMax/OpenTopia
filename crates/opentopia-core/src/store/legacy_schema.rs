use super::project_repository::{backfill_thread_projects, table_has_column};
use super::sqlite_runtime::recover_interrupted_runtime_state;
use super::SqliteSessionStore;
use crate::store_migrations::LEGACY_DATABASE_SCHEMA_VERSION;
use rusqlite::{Connection, OptionalExtension};

pub(super) fn turns_table_supports_waiting_boundaries(conn: &Connection) -> anyhow::Result<bool> {
    let sql = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'turns'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Ok(["waiting_user_input", "waiting_user_action"]
        .iter()
        .all(|status| sql.contains(status)))
}

fn rebuild_turns_with_waiting_boundaries(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let migration = conn.execute_batch(
        r#"
        BEGIN IMMEDIATE;
        DROP TABLE IF EXISTS turns_waiting_migration;
        CREATE TABLE turns_waiting_migration (
            turn_id TEXT PRIMARY KEY,
            invocation_id INTEGER NOT NULL DEFAULT 1,
            thread_id TEXT NOT NULL,
            user_message_id TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            error TEXT,
            CHECK(status IN (
                'running', 'waiting_approval', 'waiting_user_input',
                'waiting_user_action', 'cancelling', 'succeeded',
                'failed', 'cancelled', 'interrupted'
            )),
            FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
        );
        INSERT INTO turns_waiting_migration (
            turn_id, invocation_id, thread_id, user_message_id, status,
            started_at, updated_at, completed_at, error
        )
        SELECT turn_id, invocation_id, thread_id, user_message_id, status,
               started_at, updated_at, completed_at, error
        FROM turns;
        DROP TABLE turns;
        ALTER TABLE turns_waiting_migration RENAME TO turns;
        CREATE INDEX IF NOT EXISTS idx_turns_thread_started
            ON turns(thread_id, started_at DESC);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_turns_thread_active
            ON turns(thread_id)
            WHERE status IN ('running', 'cancelling');
        COMMIT;
        "#,
    );
    if migration.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    let foreign_keys = conn.execute_batch("PRAGMA foreign_keys = ON;");
    migration?;
    foreign_keys?;

    let foreign_key_error: Option<String> = conn
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()?;
    if let Some(table) = foreign_key_error {
        anyhow::bail!("turns waiting-boundary migration failed for table {table}");
    }
    Ok(())
}

fn goals_table_has_retired_columns(conn: &Connection) -> anyhow::Result<bool> {
    for column in ["status", "plan_revision", "completed_at"] {
        if table_has_column(conn, "goals", column)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn rebuild_goals_without_retired_columns(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
    let migration = conn.execute_batch(
        r#"
        BEGIN IMMEDIATE;
        DROP TABLE IF EXISTS goals_v15;
        CREATE TABLE goals_v15 (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL,
            objective TEXT NOT NULL,
            token_budget INTEGER,
            tokens_used INTEGER NOT NULL DEFAULT 0,
            time_used_seconds INTEGER NOT NULL DEFAULT 0,
            version INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
        );
        INSERT INTO goals_v15 (
            id, thread_id, objective, token_budget, tokens_used,
            time_used_seconds, version, created_at, updated_at
        )
        SELECT id, thread_id, objective, token_budget, tokens_used,
               time_used_seconds, version, created_at, updated_at
        FROM goals;
        DROP TABLE goals;
        ALTER TABLE goals_v15 RENAME TO goals;
        CREATE INDEX idx_goals_thread_updated
            ON goals(thread_id, updated_at DESC);
        COMMIT;
        "#,
    );
    if migration.is_err() {
        let _ = conn.execute_batch("ROLLBACK;");
    }
    let foreign_keys = conn.execute_batch("PRAGMA foreign_keys = ON;");
    migration?;
    foreign_keys?;

    let foreign_key_error: Option<String> = conn
        .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
        .optional()?;
    if let Some(table) = foreign_key_error {
        anyhow::bail!("retired goal schema reconciliation failed for table {table}");
    }
    Ok(())
}

impl SqliteSessionStore {
    pub(super) fn migrate_legacy_database(&self) -> anyhow::Result<()> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        anyhow::ensure!(
            schema_version <= LEGACY_DATABASE_SCHEMA_VERSION,
            "legacy database schema version {schema_version} is newer than supported legacy version {LEGACY_DATABASE_SCHEMA_VERSION}"
        );
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
                    CHECK(experience_mode IN ('work', 'code', 'flow')),
                model_selection TEXT,
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
                invocation_id INTEGER NOT NULL DEFAULT 1,
                thread_id TEXT NOT NULL,
                user_message_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                error TEXT,
                CHECK(status IN (
                    'running', 'waiting_approval', 'waiting_user_input',
                    'waiting_user_action', 'cancelling', 'succeeded', 'failed',
                    'cancelled', 'interrupted'
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

            CREATE TABLE IF NOT EXISTS work_forms (
                form_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                scope_kind TEXT NOT NULL CHECK(scope_kind IN ('turn', 'goal')),
                scope_id TEXT NOT NULL,
                form_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(scope_kind, scope_id),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS goals (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                objective TEXT NOT NULL,
                token_budget INTEGER,
                tokens_used INTEGER NOT NULL DEFAULT 0,
                time_used_seconds INTEGER NOT NULL DEFAULT 0,
                version INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
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

            CREATE TABLE IF NOT EXISTS conversation_events (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                turn_id TEXT,
                seq INTEGER NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(thread_id, seq),
                FOREIGN KEY(id) REFERENCES events(id) ON DELETE CASCADE,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS effect_journal (
                effect_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                agent_path TEXT NOT NULL,
                idempotency_key TEXT NOT NULL,
                kind TEXT NOT NULL,
                operation TEXT NOT NULL,
                input_hash TEXT NOT NULL,
                input_json TEXT NOT NULL,
                result_json TEXT,
                status TEXT NOT NULL,
                side_effect_class TEXT NOT NULL,
                idempotent INTEGER NOT NULL,
                attempt INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                updated_at TEXT NOT NULL,
                UNIQUE(thread_id, turn_id, agent_path, idempotency_key),
                CHECK(kind IN ('model_request', 'tool_call', 'approval', 'finalization')),
                CHECK(status IN ('prepared', 'running', 'succeeded', 'failed', 'indeterminate')),
                CHECK(side_effect_class IN ('none', 'workspace', 'external', 'unknown')),
                CHECK(idempotent IN (0, 1)),
                CHECK(attempt >= 0),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
                FOREIGN KEY(turn_id) REFERENCES turns(turn_id) ON DELETE CASCADE
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
                execution_contract_json TEXT NOT NULL DEFAULT '{}',
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
                adapter_identity TEXT NOT NULL DEFAULT '',
                response_id TEXT NOT NULL,
                compatibility_hash TEXT NOT NULL,
                response_items_json TEXT NOT NULL DEFAULT '[]',
                state_kind TEXT NOT NULL DEFAULT 'stored_response',
                compaction_item_count INTEGER NOT NULL DEFAULT 0,
                checkpoint_id TEXT,
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

            CREATE TABLE IF NOT EXISTS turn_checkpoints (
                turn_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                wait_kind TEXT NOT NULL CHECK(wait_kind IN (
                    'approval', 'user_input', 'external_action'
                )),
                checkpoint_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(turn_id) REFERENCES turns(turn_id) ON DELETE CASCADE,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS turn_checkpoint_blobs (
                content_hash TEXT PRIMARY KEY,
                kind TEXT NOT NULL CHECK(kind IN (
                    'conversation', 'model_context', 'tool_catalog'
                )),
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL
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

            CREATE TABLE IF NOT EXISTS thread_plugin_activations (
                thread_id TEXT NOT NULL,
                plugin_name TEXT NOT NULL,
                enabled INTEGER NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(thread_id, plugin_name),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS mcp_server_tools (
                server_id TEXT NOT NULL,
                public_name TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                description TEXT,
                input_schema_json TEXT NOT NULL,
                annotations_json TEXT NOT NULL,
                meta_json TEXT NOT NULL DEFAULT '{}',
                permission_labels_json TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(server_id, public_name),
                FOREIGN KEY(server_id) REFERENCES mcp_servers(server_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS agent_templates (
                template_id TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                archived_at TEXT
            );

            CREATE TABLE IF NOT EXISTS agent_template_versions (
                template_id TEXT NOT NULL,
                version INTEGER NOT NULL CHECK(version > 0),
                status TEXT NOT NULL CHECK(status IN ('draft', 'published')),
                content_hash TEXT NOT NULL,
                document_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                published_at TEXT,
                published_by TEXT,
                PRIMARY KEY(template_id, version),
                FOREIGN KEY(template_id) REFERENCES agent_templates(template_id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS agent_instances (
                instance_id TEXT PRIMARY KEY,
                template_id TEXT NOT NULL,
                template_version INTEGER NOT NULL,
                thread_id TEXT NOT NULL,
                parent_instance_id TEXT,
                status TEXT NOT NULL CHECK(status IN ('active', 'suspended', 'completed', 'revoked')),
                state_revision INTEGER NOT NULL CHECK(state_revision > 0),
                document_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(template_id, template_version)
                    REFERENCES agent_template_versions(template_id, version),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
                FOREIGN KEY(parent_instance_id) REFERENCES agent_instances(instance_id)
            );

            CREATE TABLE IF NOT EXISTS thread_agent_bindings (
                thread_id TEXT PRIMARY KEY,
                instance_id TEXT NOT NULL UNIQUE,
                updated_at TEXT NOT NULL,
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
                FOREIGN KEY(instance_id) REFERENCES agent_instances(instance_id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_thread_created
                ON messages(thread_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_effect_journal_turn
                ON effect_journal(turn_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_effect_journal_recovery
                ON effect_journal(status, updated_at);

            CREATE INDEX IF NOT EXISTS idx_events_thread_seq
                ON events(thread_id, seq);

            CREATE INDEX IF NOT EXISTS idx_conversation_events_thread_seq
                ON conversation_events(thread_id, seq);

            CREATE INDEX IF NOT EXISTS idx_turns_thread_started
                ON turns(thread_id, started_at DESC);

            CREATE UNIQUE INDEX IF NOT EXISTS idx_turns_thread_active
                ON turns(thread_id)
                WHERE status IN ('running', 'cancelling');

            CREATE INDEX IF NOT EXISTS idx_turn_queue_thread_created
                ON turn_queue(thread_id, queued_at, message_id);

            CREATE INDEX IF NOT EXISTS idx_goals_thread_updated
                ON goals(thread_id, updated_at DESC);

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

            CREATE INDEX IF NOT EXISTS idx_turn_checkpoints_thread
                ON turn_checkpoints(thread_id, updated_at DESC);

            CREATE INDEX IF NOT EXISTS idx_thread_mcp_servers_thread
                ON thread_mcp_servers(thread_id, updated_at);

            CREATE INDEX IF NOT EXISTS idx_thread_plugin_activations_thread
                ON thread_plugin_activations(thread_id, updated_at);

            CREATE INDEX IF NOT EXISTS idx_agent_template_versions_status
                ON agent_template_versions(template_id, status, version DESC);

            CREATE INDEX IF NOT EXISTS idx_agent_instances_thread_created
                ON agent_instances(thread_id, created_at DESC);

            CREATE INDEX IF NOT EXISTS idx_agent_instances_parent_created
                ON agent_instances(parent_instance_id, created_at);

            CREATE INDEX IF NOT EXISTS idx_projects_order
                ON projects(pinned DESC, sort_order ASC, created_at ASC);
            "#,
        )?;
        crate::plugin_control::migrate_plugin_control(&mut conn)?;
        crate::scm_connector::migrate_scm_remote_bindings(&mut conn)?;

        if !table_has_column(&conn, "threads", "project_id")? {
            conn.execute(
                "ALTER TABLE threads ADD COLUMN project_id TEXT REFERENCES projects(id) ON DELETE SET NULL",
                [],
            )?;
        }
        if !table_has_column(&conn, "turns", "invocation_id")? {
            conn.execute(
                "ALTER TABLE turns ADD COLUMN invocation_id INTEGER NOT NULL DEFAULT 1",
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
        if !table_has_column(&conn, "mcp_server_tools", "meta_json")? {
            conn.execute(
                "ALTER TABLE mcp_server_tools ADD COLUMN meta_json TEXT NOT NULL DEFAULT '{}'",
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
        if !table_has_column(&conn, "threads", "model_selection")? {
            conn.execute("ALTER TABLE threads ADD COLUMN model_selection TEXT", [])?;
        }
        for (column, definition) in [
            ("adapter_identity", "TEXT NOT NULL DEFAULT ''"),
            ("response_items_json", "TEXT NOT NULL DEFAULT '[]'"),
            ("state_kind", "TEXT NOT NULL DEFAULT 'stored_response'"),
            ("compaction_item_count", "INTEGER NOT NULL DEFAULT 0"),
            ("checkpoint_id", "TEXT"),
        ] {
            if !table_has_column(&conn, "provider_conversation_states", column)? {
                conn.execute(
                    &format!(
                        "ALTER TABLE provider_conversation_states ADD COLUMN {column} {definition}"
                    ),
                    [],
                )?;
            }
        }
        for (column, definition) in [
            ("agent_path", "TEXT NOT NULL DEFAULT ''"),
            ("parent_agent_path", "TEXT NOT NULL DEFAULT '/root'"),
            ("agent_type", "TEXT NOT NULL DEFAULT 'default'"),
            ("fork_turns", "TEXT NOT NULL DEFAULT 'all'"),
            ("last_task_message", "TEXT NOT NULL DEFAULT ''"),
            ("execution_contract_json", "TEXT NOT NULL DEFAULT '{}'"),
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
        if schema_version < 8 {
            conn.execute_batch("PRAGMA user_version = 8;")?;
        }
        if schema_version < 9 {
            conn.execute_batch(
                r#"
                INSERT OR REPLACE INTO conversation_events (
                    id, thread_id, turn_id, seq, payload_json, created_at
                )
                SELECT id, thread_id, turn_id, seq,
                    CASE kind
                    WHEN 'model_context_built'
                        THEN json_remove(payload_json, '$.items')
                    WHEN 'model_request'
                        THEN json_remove(payload_json, '$.request')
                    WHEN 'provider_request_sent'
                        THEN json_remove(payload_json, '$.body')
                    WHEN 'provider_request_retried'
                        THEN json_remove(payload_json, '$.body')
                    WHEN 'provider_response_received'
                        THEN json_remove(payload_json, '$.body')
                    WHEN 'tool_call_finished'
                        THEN json_remove(payload_json, '$.result.content')
                    ELSE payload_json
                    END,
                    created_at
                FROM events
                WHERE kind <> 'reasoning_delta';
                "#,
            )?;
            conn.execute_batch("PRAGMA user_version = 9;")?;
        }
        if schema_version < 10 {
            conn.execute_batch(
                r#"
                UPDATE conversation_events
                SET payload_json = json_remove(payload_json, '$.result.content')
                WHERE json_extract(payload_json, '$.type') = 'tool_call_finished';
                PRAGMA user_version = 10;
                "#,
            )?;
        }
        if schema_version < 11 {
            // SQLite cannot alter a CHECK constraint in place. Rebuild only
            // the threads table while foreign-key enforcement is paused, then
            // verify the complete database before accepting the migration.
            conn.execute_batch(
                r#"
                PRAGMA foreign_keys = OFF;
                CREATE TABLE threads_v11 (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL,
                    workspace_root TEXT NOT NULL,
                    project_id TEXT,
                    experience_mode TEXT NOT NULL DEFAULT 'code'
                        CHECK(experience_mode IN ('work', 'code', 'flow')),
                    model_selection TEXT,
                    archived_at TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE SET NULL
                );
                INSERT INTO threads_v11 (
                    id, title, workspace_root, project_id, experience_mode,
                    model_selection, archived_at, created_at, updated_at
                )
                SELECT id, title, workspace_root, project_id, experience_mode,
                       model_selection, archived_at, created_at, updated_at
                FROM threads;
                DROP TABLE threads;
                ALTER TABLE threads_v11 RENAME TO threads;
                CREATE INDEX IF NOT EXISTS idx_threads_project_updated
                    ON threads(project_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_threads_archived_updated
                    ON threads(archived_at, updated_at DESC);
                PRAGMA foreign_keys = ON;
                PRAGMA user_version = 11;
                "#,
            )?;
            let foreign_key_error: Option<String> = conn
                .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
                .optional()?;
            if let Some(table) = foreign_key_error {
                anyhow::bail!("schema v11 foreign-key check failed for table {table}");
            }
        }
        if schema_version < 12 {
            // Phase 1 enterprise control-plane tables are created by the
            // idempotent schema block above. Advancing the version here keeps
            // older databases on the same migration ledger.
            conn.execute_batch("PRAGMA user_version = 12;")?;
        }
        if schema_version < 13 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS flow_drafts (
                    id TEXT PRIMARY KEY,
                    thread_id TEXT NOT NULL,
                    flow_id TEXT NOT NULL,
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    status TEXT NOT NULL,
                    document_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
                );
                CREATE TABLE IF NOT EXISTS thread_flow_drafts (
                    thread_id TEXT PRIMARY KEY,
                    draft_id TEXT NOT NULL UNIQUE,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
                    FOREIGN KEY(draft_id) REFERENCES flow_drafts(id) ON DELETE CASCADE
                );
                CREATE TABLE IF NOT EXISTS flow_trials (
                    id TEXT PRIMARY KEY,
                    draft_id TEXT NOT NULL,
                    draft_revision INTEGER NOT NULL,
                    status TEXT NOT NULL CHECK(status IN ('passed', 'failed')),
                    document_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY(draft_id) REFERENCES flow_drafts(id) ON DELETE CASCADE
                );
                CREATE TABLE IF NOT EXISTS flow_definitions (
                    id TEXT PRIMARY KEY,
                    flow_id TEXT NOT NULL,
                    version INTEGER NOT NULL CHECK(version > 0),
                    name TEXT NOT NULL,
                    owner TEXT NOT NULL,
                    content_hash TEXT NOT NULL,
                    document_json TEXT NOT NULL,
                    published_at TEXT NOT NULL,
                    published_by TEXT NOT NULL,
                    UNIQUE(flow_id, version)
                );
                CREATE INDEX IF NOT EXISTS idx_flow_drafts_thread_updated
                    ON flow_drafts(thread_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_flow_drafts_flow_updated
                    ON flow_drafts(flow_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_flow_trials_draft_created
                    ON flow_trials(draft_id, created_at DESC);
                CREATE INDEX IF NOT EXISTS idx_flow_definitions_search
                    ON flow_definitions(flow_id, version DESC);
                PRAGMA user_version = 13;
                "#,
            )?;
        }
        if schema_version < 14 {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS flow_runs (
                    id TEXT PRIMARY KEY,
                    thread_id TEXT NOT NULL,
                    flow_id TEXT NOT NULL,
                    flow_version INTEGER NOT NULL CHECK(flow_version > 0),
                    revision INTEGER NOT NULL CHECK(revision > 0),
                    status TEXT NOT NULL CHECK(status IN (
                        'queued', 'running', 'pause_requested', 'paused',
                        'waiting_approval', 'succeeded', 'failed',
                        'cancel_requested', 'cancelled'
                    )),
                    document_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    completed_at TEXT,
                    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
                );
                CREATE INDEX IF NOT EXISTS idx_flow_runs_thread_updated
                    ON flow_runs(thread_id, updated_at DESC);
                CREATE INDEX IF NOT EXISTS idx_flow_runs_definition_updated
                    ON flow_runs(flow_id, flow_version, updated_at DESC);
                PRAGMA user_version = 14;
                "#,
            )?;
        }
        let goals_have_retired_columns = goals_table_has_retired_columns(&conn)?;
        if schema_version < 15 {
            conn.execute_batch(
                r#"
                DROP TABLE IF EXISTS goal_task_attempts;
                DROP TABLE IF EXISTS goal_tasks;
                DROP TABLE IF EXISTS goal_plan_revisions;
                "#,
            )?;
            if goals_have_retired_columns {
                rebuild_goals_without_retired_columns(&conn)?;
            }
            conn.pragma_update(None, "user_version", 15)?;
            let foreign_key_error: Option<String> = conn
                .query_row("PRAGMA foreign_key_check", [], |row| row.get(0))
                .optional()?;
            if let Some(table) = foreign_key_error {
                anyhow::bail!("schema v15 foreign-key check failed for table {table}");
            }
        } else if goals_have_retired_columns {
            // Several development builds wrote v19 to the version pragma while
            // retaining the pre-v15 goals table. Reconcile the actual invariant
            // before accepting the verified baseline; the retired companion
            // tables remain preserved as opaque historical data.
            rebuild_goals_without_retired_columns(&conn)?;
        }
        if schema_version < 16 {
            // SQLite cannot widen an existing CHECK constraint in place.
            if !turns_table_supports_waiting_boundaries(&conn)? {
                rebuild_turns_with_waiting_boundaries(&conn)?;
            }
            conn.pragma_update(None, "user_version", 16)?;
        }
        if schema_version < LEGACY_DATABASE_SCHEMA_VERSION
            || !turns_table_supports_waiting_boundaries(&conn)?
        {
            // Some development databases reached v17/v18 while retaining the
            // pre-waiting-boundary CHECK constraint. Reconcile the actual table
            // invariant instead of trusting the version ledger alone.
            if !turns_table_supports_waiting_boundaries(&conn)? {
                rebuild_turns_with_waiting_boundaries(&conn)?;
            }
            conn.pragma_update(None, "user_version", LEGACY_DATABASE_SCHEMA_VERSION)?;
        }
        recover_interrupted_runtime_state(&conn)?;
        Ok(())
    }

    pub(crate) fn with_connection<T>(
        &self,
        action: impl FnOnce(&mut Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        action(&mut conn)
    }
}
