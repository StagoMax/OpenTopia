#[test]
fn schema_manifest_compares_indexes_by_column_identity_not_legacy_ordinal() {
    let canonical = Connection::open_in_memory().expect("open canonical schema");
    canonical
        .execute_batch(
            r#"
            CREATE TABLE sample (a TEXT PRIMARY KEY, b TEXT, c TEXT);
            CREATE INDEX idx_sample_a_c ON sample(a, c DESC);
            "#,
        )
        .expect("create canonical schema");
    let reordered = Connection::open_in_memory().expect("open reordered schema");
    reordered
        .execute_batch(
            r#"
            CREATE TABLE sample (c TEXT, b TEXT, a TEXT PRIMARY KEY);
            CREATE INDEX idx_sample_a_c ON sample(a, c DESC);
            "#,
        )
        .expect("create reordered schema");

    assert_eq!(
        store_migrations::inspect_schema(&canonical).expect("inspect canonical schema"),
        store_migrations::inspect_schema(&reordered).expect("inspect reordered schema")
    );
}

#[test]
fn migration_reconciles_mislabeled_v19_goals_and_preserves_retired_data() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/v19-goal-migration"))
        .expect("create thread");
    let created = store
        .create_goal(thread.id, "Preserve the current goal".into(), Some(4096))
        .expect("create goal");
    {
        let conn = store.conn.lock().expect("lock store");
        remove_post_legacy_agent_runtime_tables(&conn);
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = OFF;
            CREATE TABLE goals_legacy_v19 (
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
            INSERT INTO goals_legacy_v19 (
                id, thread_id, objective, status, plan_revision, token_budget,
                tokens_used, time_used_seconds, version, created_at, updated_at,
                completed_at
            )
            SELECT id, thread_id, objective, 'active', 3, token_budget,
                   tokens_used, time_used_seconds, version, created_at, updated_at,
                   NULL
            FROM goals;
            DROP TABLE goals;
            ALTER TABLE goals_legacy_v19 RENAME TO goals;
            CREATE INDEX idx_goals_thread_updated
                ON goals(thread_id, updated_at DESC);
            CREATE TABLE harness_learning_cases (
                run_id TEXT PRIMARY KEY,
                legacy_payload TEXT NOT NULL
            );
            INSERT INTO harness_learning_cases (run_id, legacy_payload)
            VALUES ('legacy-run', 'preserve-me');
            DROP TABLE agent_mailbox_messages;
            DROP TABLE agent_ledger_items;
            DROP TABLE agent_turns;
            DROP TABLE agent_threads;
            DROP TABLE agent_runtime_snapshots;
            DROP TABLE agent_sessions;
            DROP TABLE schema_migrations;
            PRAGMA user_version = 19;
            PRAGMA foreign_keys = ON;
            "#,
        )
        .expect("restore mislabeled v19 schema");
    }

    store.migrate().expect("reconcile mislabeled v19 schema");

    let restored = store
        .get_thread_goal(thread.id)
        .expect("read restored goal")
        .expect("goal remains after migration");
    assert_eq!(restored.goal.id, created.goal.id);
    assert_eq!(restored.goal.objective, "Preserve the current goal");
    assert_eq!(restored.goal.token_budget, Some(4096));
    let conn = store.conn.lock().expect("lock migrated store");
    assert!(!table_has_column(&conn, "goals", "status").expect("inspect goal status"));
    assert!(!table_has_column(&conn, "goals", "plan_revision").expect("inspect goal plan revision"));
    assert!(!table_has_column(&conn, "goals", "completed_at").expect("inspect goal completion"));
    let legacy_payload: String = conn
        .query_row(
            "SELECT legacy_payload FROM harness_learning_cases WHERE run_id = 'legacy-run'",
            [],
            |row| row.get(0),
        )
        .expect("read preserved legacy evaluation");
    assert_eq!(legacy_payload, "preserve-me");
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read migrated schema version");
    assert_eq!(schema_version, CURRENT_DATABASE_SCHEMA_VERSION);
    drop(conn);

    store.migrate().expect("reopen managed migrated schema");
}

#[test]
fn opening_a_future_database_version_fails_before_mutating_it() {
    let path = temporary_db_path("future-schema");
    {
        let conn = Connection::open(&path).expect("create future database");
        conn.pragma_update(None, "user_version", CURRENT_DATABASE_SCHEMA_VERSION + 1)
            .expect("mark future schema");
    }

    let error = SqliteSessionStore::open(&path)
        .expect_err("an older binary must not rewrite a newer database");
    assert!(error.to_string().contains("newer than supported version"));

    let conn = Connection::open(&path).expect("reopen untouched future database");
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read untouched schema version");
    assert_eq!(schema_version, CURRENT_DATABASE_SCHEMA_VERSION + 1);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn migration_ledger_records_verified_baseline_and_current_version() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let conn = store.conn.lock().expect("lock store");
    let mut stmt = conn
        .prepare(
            r#"
            SELECT version, name, checksum, schema_fingerprint, app_build
            FROM schema_migrations
            ORDER BY version
            "#,
        )
        .expect("prepare migration ledger query");
    let records = collect_rows(
        stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .expect("query migration ledger"),
    )
    .expect("read migration ledger");

    assert_eq!(records.len(), 10);
    assert_eq!(records[0].0, LEGACY_DATABASE_SCHEMA_VERSION);
    assert_eq!(records[0].1, "legacy_baseline_v19");
    assert_eq!(records[1].0, 20);
    assert_eq!(records[1].1, "verified_migration_ledger");
    assert_eq!(records[2].0, 21);
    assert_eq!(records[2].1, "agent_collaboration_domain");
    assert_eq!(records[3].0, 22);
    assert_eq!(records[3].1, "agent_runtime_cutover");
    assert_eq!(records[4].0, 23);
    assert_eq!(records[4].1, "agent_turn_checkpoints");
    assert_eq!(records[5].0, 24);
    assert_eq!(records[5].1, "agent_activity_projection");
    assert_eq!(records[6].0, 25);
    assert_eq!(records[6].1, "flow_human_tasks");
    assert_eq!(records[7].0, 26);
    assert_eq!(records[7].1, "connections_control_plane");
    assert_eq!(records[8].0, 27);
    assert_eq!(records[8].1, "workflow_deployments");
    assert_eq!(records[9].0, CURRENT_DATABASE_SCHEMA_VERSION);
    assert_eq!(records[9].1, "workflow_activity_receipts");
    assert!(records.iter().all(|record| record.2.starts_with("sha256:")));
    assert!(records.iter().all(|record| !record.3.is_empty()));
    assert!(records
        .iter()
        .all(|record| record.4 == env!("CARGO_PKG_VERSION")));
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read user version mirror");
    assert_eq!(user_version, CURRENT_DATABASE_SCHEMA_VERSION);
}

#[test]
fn migration_upgrades_pre_checkpoint_v22_database() {
    let path = temporary_db_path("migration-v22-checkpoints");
    {
        let store = SqliteSessionStore::open(&path).expect("create current database");
        let conn = store.conn.lock().expect("lock current database");
        restore_pre_v28_effect_journal(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE workflow_deployments;
            DROP TABLE connection_capability_revisions;
            DROP TABLE connections;
            DROP TABLE integration_definitions;
            DROP TABLE human_tasks;
            DROP TABLE agent_activity_state;
            DROP TABLE agent_turn_checkpoints;
            DROP INDEX idx_agent_events_activity_visible;
            DROP INDEX idx_agent_events_reasoning_tail;
            DROP INDEX idx_agent_events_tool_results;
            DROP INDEX idx_agent_events_model_round;
            DELETE FROM schema_migrations WHERE version = 28;
            DELETE FROM schema_migrations WHERE version = 27;
            DELETE FROM schema_migrations WHERE version = 26;
            DELETE FROM schema_migrations WHERE version = 25;
            DELETE FROM schema_migrations WHERE version = 24;
            DELETE FROM schema_migrations WHERE version = 23;
            PRAGMA user_version = 22;
            "#,
        )
        .expect("restore the pre-checkpoint v22 schema");
    }

    let migrated = SqliteSessionStore::open(&path).expect("upgrade v22 database");
    let conn = migrated.conn.lock().expect("lock migrated database");
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read migrated schema version");
    assert_eq!(schema_version, CURRENT_DATABASE_SCHEMA_VERSION);
    let checkpoint_table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'agent_turn_checkpoints')",
            [],
            |row| row.get(0),
        )
        .expect("inspect checkpoint table");
    assert!(checkpoint_table_exists);
    drop(conn);
    drop(migrated);
    let _ = std::fs::remove_file(path);
}

#[test]
fn v26_migrates_legacy_mcp_server_into_definition_and_connection() {
    use crate::connection::{ConnectionAuthVerificationV1, ConnectionRuntimeBindingV1};

    let path = temporary_db_path("migration-v26-legacy-mcp");
    let server_id = {
        let store = SqliteSessionStore::open(&path).expect("create current database");
        let mut server = McpServerConfig::new("Legacy CRM".to_string(), "crm-mcp".to_string());
        server.env_keys = vec!["CRM_TOKEN".to_string()];
        let server = store
            .insert_mcp_server(server)
            .expect("insert legacy MCP server");
        let conn = store.conn.lock().expect("lock current database");
        restore_pre_v28_effect_journal(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE workflow_deployments;
            DROP TABLE connection_capability_revisions;
            DROP TABLE connections;
            DROP TABLE integration_definitions;
            DELETE FROM schema_migrations WHERE version = 28;
            DELETE FROM schema_migrations WHERE version = 27;
            DELETE FROM schema_migrations WHERE version = 26;
            PRAGMA user_version = 25;
            "#,
        )
        .expect("restore v25 schema with legacy MCP data");
        server.server_id
    };

    let migrated = SqliteSessionStore::open(&path).expect("apply v26 migration");
    let definition = migrated
        .get_integration_definition(server_id)
        .expect("load migrated definition")
        .expect("definition exists");
    let connection = migrated
        .get_connection(server_id)
        .expect("load migrated connection")
        .expect("connection exists");

    assert_eq!(definition.id, server_id);
    assert_eq!(connection.id, server_id);
    assert_eq!(connection.integration_definition_id, definition.id);
    assert_eq!(
        connection.runtime_binding,
        ConnectionRuntimeBindingV1::McpServer { server_id }
    );
    assert_eq!(
        connection.auth_context.verification,
        ConnectionAuthVerificationV1::LegacyUnverified
    );
    assert_eq!(connection.active_capability_revision, None);
    assert!(
        migrated
            .list_connection_capability_revisions(connection.id)
            .expect("list migrated capability revisions")
            .is_empty(),
        "legacy mcp_server_tools are deliberately not promoted until an explicit tools/list refresh"
    );

    drop(migrated);
    let _ = std::fs::remove_file(path);
}

#[test]
fn migration_ledger_is_idempotent_across_reopen() {
    let path = temporary_db_path("migration-idempotency");
    let before = {
        let store = SqliteSessionStore::open(&path).expect("create managed database");
        let conn = store.conn.lock().expect("lock store");
        let mut stmt = conn
            .prepare(
                "SELECT version, checksum, schema_fingerprint, applied_at FROM schema_migrations ORDER BY version",
            )
            .expect("prepare ledger snapshot");
        collect_rows(
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .expect("query first ledger snapshot"),
        )
        .expect("read first ledger snapshot")
    };

    let after = {
        let store = SqliteSessionStore::open(&path).expect("reopen managed database");
        let conn = store.conn.lock().expect("lock reopened store");
        let mut stmt = conn
            .prepare(
                "SELECT version, checksum, schema_fingerprint, applied_at FROM schema_migrations ORDER BY version",
            )
            .expect("prepare reopened ledger snapshot");
        collect_rows(
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .expect("query reopened ledger snapshot"),
        )
        .expect("read reopened ledger snapshot")
    };

    assert_eq!(before, after);
    let _ = std::fs::remove_file(path);
}

#[test]
fn migration_checksum_mismatch_fails_before_mutating_database() {
    let path = temporary_db_path("migration-checksum");
    {
        let store = SqliteSessionStore::open(&path).expect("create managed database");
        drop(store);
    }
    {
        let conn = Connection::open(&path).expect("open database for corruption fixture");
        conn.execute(
            "UPDATE schema_migrations SET checksum = 'tampered' WHERE version = ?1",
            params![CURRENT_DATABASE_SCHEMA_VERSION],
        )
        .expect("tamper migration checksum");
    }

    let error = SqliteSessionStore::open(&path)
        .expect_err("checksum mismatch must fail before startup mutation");
    assert!(error.to_string().contains("migration checksum mismatch"));

    let conn = Connection::open(&path).expect("reopen rejected database");
    let checksum: String = conn
        .query_row(
            "SELECT checksum FROM schema_migrations WHERE version = ?1",
            params![CURRENT_DATABASE_SCHEMA_VERSION],
            |row| row.get(0),
        )
        .expect("read untouched checksum");
    assert_eq!(checksum, "tampered");
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn managed_schema_drift_fails_closed_without_silent_repair() {
    let path = temporary_db_path("schema-drift");
    {
        let store = SqliteSessionStore::open(&path).expect("create managed database");
        drop(store);
    }
    {
        let conn = Connection::open(&path).expect("open database for drift fixture");
        conn.execute_batch("DROP INDEX idx_messages_thread_created;")
            .expect("remove managed index");
    }

    let error = SqliteSessionStore::open(&path)
        .expect_err("managed schema drift must not be silently repaired");
    assert!(error.to_string().contains("database schema drift detected"));

    let conn = Connection::open(&path).expect("reopen rejected database");
    let index_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'index' AND name = 'idx_messages_thread_created')",
            [],
            |row| row.get(0),
        )
        .expect("inspect rejected schema");
    assert!(
        !index_exists,
        "startup must not repair drift before failing"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn unknown_legacy_schema_is_not_baselined() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    {
        let conn = store.conn.lock().expect("lock store");
        remove_post_legacy_agent_runtime_tables(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE agent_mailbox_messages;
            DROP TABLE agent_ledger_items;
            DROP TABLE agent_turns;
            DROP TABLE agent_threads;
            DROP TABLE agent_runtime_snapshots;
            DROP TABLE agent_sessions;
            DROP TABLE schema_migrations;
            CREATE TABLE unexpected_legacy_table (id TEXT PRIMARY KEY);
            PRAGMA user_version = 19;
            "#,
        )
        .expect("create unknown legacy profile");
    }

    let error = store
        .migrate()
        .expect_err("unknown legacy shape must not become a trusted baseline");
    assert!(error.to_string().contains("canonical manifest"));
    let conn = store.conn.lock().expect("lock rejected legacy store");
    assert!(!store_migrations::has_migration_ledger(&conn).expect("inspect migration ledger"));
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read rejected legacy version");
    assert_eq!(user_version, LEGACY_DATABASE_SCHEMA_VERSION);
}
