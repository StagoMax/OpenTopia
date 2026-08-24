fn temporary_db_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "opentopia-{label}-{}-{}.db",
        std::process::id(),
        Uuid::new_v4()
    ))
}

/// Tests that simulate a pre-v28 database start from the current canonical
/// schema. Reverse the v28 journal widening as part of that fixture downgrade;
/// otherwise the test would advertise an old user_version with a new schema.
fn restore_pre_v28_effect_journal(conn: &Connection) {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = OFF;
        ALTER TABLE effect_journal RENAME TO effect_journal_v28_source;
        CREATE TABLE effect_journal (
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
        INSERT INTO effect_journal
        SELECT * FROM effect_journal_v28_source;
        DROP TABLE effect_journal_v28_source;
        CREATE INDEX idx_effect_journal_turn
            ON effect_journal(turn_id, created_at);
        CREATE INDEX idx_effect_journal_recovery
            ON effect_journal(status, updated_at);
        PRAGMA foreign_keys = ON;
        "#,
    )
    .expect("restore the pre-v28 effect journal constraint");
}

fn remove_post_legacy_agent_runtime_tables(conn: &Connection) {
    restore_pre_v28_effect_journal(conn);
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS workflow_evaluations;
        DROP TABLE IF EXISTS workflow_delivery_receipts;
        DROP TABLE IF EXISTS workflow_trigger_invocations;
        DROP TABLE IF EXISTS workflow_releases;
        DROP TABLE IF EXISTS workflow_deployments;
        DROP TABLE IF EXISTS connection_capability_revisions;
        DROP TABLE IF EXISTS connections;
        DROP TABLE IF EXISTS integration_definitions;
        DROP TABLE IF EXISTS human_tasks;
        DROP TABLE IF EXISTS agent_activity_state;
        DROP TABLE IF EXISTS agent_turn_checkpoints;
        DROP TABLE IF EXISTS agent_provider_states;
        DROP TABLE IF EXISTS agent_events;
        "#,
    )
    .expect("remove post-legacy agent runtime tables");
}

/// Tests that advertise a v25 schema must restore the Flow-only HumanTask
/// constraint introduced by that version. Dropping only the v29 automation
/// tables leaves a newer polymorphic table behind and invalidates the fixture's
/// canonical schema fingerprint.
fn restore_v25_human_tasks(conn: &Connection) {
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = OFF;
        DROP INDEX idx_human_tasks_active_source_boundary;
        DROP INDEX idx_human_tasks_status_updated;
        DROP INDEX idx_human_tasks_thread_status_updated;
        DROP INDEX idx_human_tasks_flow_run_status;
        ALTER TABLE human_tasks RENAME TO human_tasks_v29_source;
        CREATE TABLE human_tasks (
            id TEXT PRIMARY KEY,
            revision INTEGER NOT NULL CHECK(revision > 0),
            thread_id TEXT NOT NULL,
            source_kind TEXT NOT NULL CHECK(source_kind IN ('flow_run')),
            source_id TEXT NOT NULL,
            source_node_run_id TEXT,
            task_type TEXT NOT NULL CHECK(task_type IN (
                'approval',
                'input_request',
                'output_review',
                'recovery',
                'reconnect',
                'data_correction',
                'manual'
            )),
            status TEXT NOT NULL CHECK(status IN ('pending', 'completed', 'cancelled')),
            document_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            resolved_at TEXT,
            FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
            FOREIGN KEY(source_id) REFERENCES flow_runs(id) ON DELETE CASCADE
        );
        INSERT INTO human_tasks (
            id, revision, thread_id, source_kind, source_id, source_node_run_id,
            task_type, status, document_json, created_at, updated_at, resolved_at
        )
        SELECT
            id, revision, thread_id, source_kind, source_id, source_node_run_id,
            task_type, status, document_json, created_at, updated_at, resolved_at
        FROM human_tasks_v29_source
        WHERE source_kind = 'flow_run';
        DROP TABLE human_tasks_v29_source;
        CREATE UNIQUE INDEX idx_human_tasks_active_source_boundary
            ON human_tasks(source_kind, source_id, source_node_run_id, task_type)
            WHERE status = 'pending';
        CREATE INDEX idx_human_tasks_status_updated
            ON human_tasks(status, updated_at DESC);
        CREATE INDEX idx_human_tasks_thread_status_updated
            ON human_tasks(thread_id, status, updated_at DESC);
        CREATE INDEX idx_human_tasks_flow_run_status
            ON human_tasks(source_id, status, updated_at DESC);
        PRAGMA foreign_keys = ON;
        "#,
    )
    .expect("restore the v25 HumanTask constraint");
}

#[test]
fn legacy_parallel_tool_defaults_migrate_once_and_preserve_later_opt_out() {
    let store = SqliteSessionStore::open(":memory:").expect("open settings store");
    let mut legacy =
        serde_json::to_value(AppSettings::from_env(crate::policy::PermissionMode::Auto)).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("parallelToolCallsMigrated");
    legacy["providers"][0]["parallelToolCalls"] = serde_json::json!(false);
    {
        let conn = store.conn.lock().expect("lock settings store");
        conn.execute(
            "INSERT INTO app_settings (id, settings_json, updated_at) VALUES (1, ?1, ?2)",
            params![legacy.to_string(), Utc::now().to_rfc3339()],
        )
        .unwrap();
    }

    let mut migrated = store
        .load_settings(crate::policy::PermissionMode::Auto)
        .unwrap();
    assert!(migrated.parallel_tool_calls_migrated);
    assert!(migrated.providers[0].parallel_tool_calls);

    migrated.providers[0].parallel_tool_calls = false;
    store.save_settings(migrated).unwrap();
    let reloaded = store
        .load_settings(crate::policy::PermissionMode::Auto)
        .unwrap();
    assert!(reloaded.parallel_tool_calls_migrated);
    assert!(!reloaded.providers[0].parallel_tool_calls);
}

fn enterprise_template_spec(tools: &[&str]) -> AgentTemplateSpecV1 {
    AgentTemplateSpecV1 {
        description: "Stored enterprise Agent".to_string(),
        instructions: "Perform the assigned case without expanding scope.".to_string(),
        capabilities: CapabilityProjection::only_tools(tools.iter().copied()),
        resource_grants: Vec::new(),
        model_policy: AgentModelPolicyV1::only([AgentModelBindingV1 {
            provider_id: "provider".to_string(),
            model_id: "model".to_string(),
        }]),
        state_schema: serde_json::json!({
            "type": "object",
            "required": ["caseId"],
            "properties": { "caseId": { "type": "string" } },
            "additionalProperties": false
        }),
        output_schema: serde_json::json!({"type": "object"}),
        allow_all_delegates: false,
        delegate_template_ids: BTreeSet::new(),
        budget: AgentBudgetV1::default(),
        risk_class: AgentRiskClassV1::Medium,
        connection_bindings: Vec::new(),
        knowledge_binding: None,
    }
}

#[test]
fn agent_template_versions_publish_and_instances_remain_isolated() {
    let store = SqliteSessionStore::open(":memory:").unwrap();
    let thread = store
        .create_thread_with_mode(
            Some("Flow design".to_string()),
            std::env::current_dir().unwrap(),
            ExperienceMode::Flow,
        )
        .unwrap();
    let draft = store
        .create_agent_template_version(
            "case-worker".to_string(),
            "Case worker".to_string(),
            "operations".to_string(),
            enterprise_template_spec(&["read_file"]),
        )
        .unwrap();
    let (published, diff) = store
        .publish_agent_template_version("case-worker", draft.version, "operations", true)
        .unwrap();
    assert!(diff.widens_capabilities);

    let second = store
        .create_agent_template_version(
            "case-worker".to_string(),
            "Case worker".to_string(),
            "operations".to_string(),
            enterprise_template_spec(&["read_file", "search"]),
        )
        .unwrap();
    let error = store
        .publish_agent_template_version("case-worker", second.version, "operations", false)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<AgentTemplateError>(),
        Some(&AgentTemplateError::CapabilityExpansionApprovalRequired)
    );
    let third = store
        .create_agent_template_version(
            "case-worker".to_string(),
            "Case worker".to_string(),
            "operations".to_string(),
            enterprise_template_spec(&["read_file", "search"]),
        )
        .unwrap();
    store
        .publish_agent_template_version("case-worker", third.version, "operations", true)
        .unwrap();
    let stale = store
        .publish_agent_template_version("case-worker", second.version, "operations", true)
        .unwrap_err();
    assert_eq!(
        stale.downcast_ref::<AgentTemplateStoreError>(),
        Some(&AgentTemplateStoreError::StaleVersion)
    );

    let profile = ExperienceSurfaceProfile::for_mode(ExperienceMode::Flow);
    let first_instance = AgentInstanceV1::instantiate(
        &published,
        thread.id,
        ExperienceMode::Flow,
        &profile.capabilities,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({"caseId": "one"}),
    )
    .unwrap();
    let second_instance = AgentInstanceV1::instantiate(
        &published,
        thread.id,
        ExperienceMode::Flow,
        &profile.capabilities,
        None,
        None,
        None,
        None,
        None,
        serde_json::json!({"caseId": "two"}),
    )
    .unwrap();
    store.insert_agent_instance(&first_instance).unwrap();
    store.insert_agent_instance(&second_instance).unwrap();
    store
        .bind_thread_agent_instance(thread.id, second_instance.id)
        .unwrap();
    assert_eq!(
        store
            .get_bound_thread_agent_instance(thread.id)
            .unwrap()
            .unwrap()
            .id,
        second_instance.id
    );
    assert_eq!(
        store.list_thread_agent_instances(thread.id).unwrap().len(),
        2
    );
    assert_eq!(
        store
            .list_agent_instances(None, Some(AgentInstanceStatusV1::Active), 50)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .list_agent_instances(Some("case-worker"), None, 1)
            .unwrap()
            .len(),
        1
    );

    let updated = store
        .update_agent_instance_state(
            first_instance.id,
            1,
            serde_json::json!({"caseId": "one-updated"}),
        )
        .unwrap();
    assert_eq!(updated.state_revision, 2);
    let conflict = store
        .update_agent_instance_state(first_instance.id, 1, serde_json::json!({"caseId": "stale"}))
        .unwrap_err();
    assert_eq!(
        conflict.downcast_ref::<AgentTemplateStoreError>(),
        Some(&AgentTemplateStoreError::StateRevisionConflict(2))
    );
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

fn mcp_server_fixture(store: &SqliteSessionStore, name: &str) -> McpServerConfig {
    store
        .insert_mcp_server(McpServerConfig {
            server_id: Uuid::new_v4(),
            name: name.to_string(),
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            cwd: None,
            env_keys: vec![],
            timeout_ms: 5_000,
            enabled: true,
            plugin_id: None,
            plugin_server_name: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .expect("insert mcp server")
}

fn tool_fixture(server_id: Uuid, tool_name: &str) -> McpToolDescriptor {
    McpToolDescriptor {
        public_name: format!("files__{tool_name}"),
        server_id,
        tool_name: tool_name.to_string(),
        description: Some(format!("{tool_name} description")),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        }),
        annotations: serde_json::json!({ "readOnlyHint": true }),
        meta: serde_json::json!({
            "com.opentopia/capabilities": ["fixture.capability/v1"]
        }),
        permission_labels: vec!["read".to_string()],
    }
}

#[test]
fn thread_plugin_activation_defaults_to_absent_and_round_trips() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(Some("bundled plugins".to_string()), std::env::temp_dir())
        .expect("create thread");

    assert!(store
        .list_thread_plugin_activations(thread.id)
        .expect("list default activations")
        .is_empty());

    store
        .set_thread_plugin_activation(thread.id, "browser-automation", false)
        .expect("disable bundled plugin");
    assert_eq!(
        store
            .list_thread_plugin_activations(thread.id)
            .expect("list disabled activation")
            .get("browser-automation"),
        Some(&false)
    );

    store
        .set_thread_plugin_activation(thread.id, "browser-automation", true)
        .expect("enable bundled plugin");
    assert_eq!(
        store
            .list_thread_plugin_activations(thread.id)
            .expect("list enabled activation")
            .get("browser-automation"),
        Some(&true)
    );
}

#[test]
fn mcp_tool_catalog_round_trips_through_sqlite() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let server = mcp_server_fixture(&store, "files");
    let tools = vec![
        tool_fixture(server.server_id, "read"),
        tool_fixture(server.server_id, "write"),
    ];

    store
        .replace_mcp_server_tools(server.server_id, &tools)
        .expect("persist tools");

    let restored = store
        .list_mcp_server_tools(server.server_id)
        .expect("load tools");
    assert_eq!(restored.len(), 2);
    let read = restored
        .iter()
        .find(|tool| tool.tool_name == "read")
        .expect("read tool persisted");
    assert_eq!(read.public_name, "files__read");
    assert_eq!(read.server_id, server.server_id);
    assert_eq!(read.description.as_deref(), Some("read description"));
    assert_eq!(read.permission_labels, vec!["read".to_string()]);
    assert_eq!(read.annotations["readOnlyHint"], serde_json::json!(true));
    assert_eq!(
        read.meta["com.opentopia/capabilities"][0],
        "fixture.capability/v1"
    );
    assert_eq!(read.input_schema["properties"]["path"]["type"], "string");
}

#[test]
fn migration_adds_mcp_tool_meta_to_existing_catalogs() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    {
        let conn = store.conn.lock().expect("lock store");
        remove_post_legacy_agent_runtime_tables(&conn);
        conn.execute_batch(
            r#"
            DROP TABLE mcp_server_tools;
            DROP TABLE agent_mailbox_messages;
            DROP TABLE agent_ledger_items;
            DROP TABLE agent_turns;
            DROP TABLE agent_threads;
            DROP TABLE agent_runtime_snapshots;
            DROP TABLE agent_sessions;
            DROP TABLE schema_migrations;
            CREATE TABLE mcp_server_tools (
                server_id TEXT NOT NULL,
                public_name TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                description TEXT,
                input_schema_json TEXT NOT NULL,
                annotations_json TEXT NOT NULL,
                permission_labels_json TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(server_id, public_name),
                FOREIGN KEY(server_id) REFERENCES mcp_servers(server_id) ON DELETE CASCADE
            );
            PRAGMA user_version = 8;
            "#,
        )
        .expect("restore legacy MCP tool table");
    }

    store.migrate().expect("migrate legacy MCP tool table");
    let conn = store.conn.lock().expect("lock migrated store");
    assert!(
        table_has_column(&conn, "mcp_server_tools", "meta_json").expect("inspect migrated table")
    );
}

#[test]
fn mcp_tool_catalog_replaces_rather_than_merges() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let server = mcp_server_fixture(&store, "files");
    store
        .replace_mcp_server_tools(
            server.server_id,
            &[
                tool_fixture(server.server_id, "read"),
                tool_fixture(server.server_id, "write"),
            ],
        )
        .expect("persist first catalog");

    // The server stopped advertising `write`; the stale row must not survive.
    store
        .replace_mcp_server_tools(server.server_id, &[tool_fixture(server.server_id, "read")])
        .expect("persist second catalog");

    let restored = store
        .list_mcp_server_tools(server.server_id)
        .expect("load tools");
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].tool_name, "read");
}

#[test]
fn mcp_tool_catalog_is_scoped_per_server_and_cascades_on_delete() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let files = mcp_server_fixture(&store, "files");
    let search = mcp_server_fixture(&store, "search");
    store
        .replace_mcp_server_tools(files.server_id, &[tool_fixture(files.server_id, "read")])
        .expect("persist files catalog");
    store
        .replace_mcp_server_tools(search.server_id, &[tool_fixture(search.server_id, "query")])
        .expect("persist search catalog");

    assert_eq!(
        store.list_all_mcp_server_tools().expect("load all").len(),
        2
    );

    assert!(store.delete_mcp_server(files.server_id).expect("delete"));

    let remaining = store.list_all_mcp_server_tools().expect("load all");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].server_id, search.server_id);
    assert!(store
        .list_mcp_server_tools(files.server_id)
        .expect("load deleted server tools")
        .is_empty());
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
        skipped: false,
        cancelled: false,
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
fn turn_checkpoint_keeps_control_state_separate_from_deduplicated_blobs() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/checkpoint"))
        .expect("create thread");
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
        .expect("insert turn");
    let blob = serde_json::json!([{"role": "user", "content": "stable ledger"}]);
    let first_ref = store
        .put_turn_checkpoint_blob("conversation", blob.clone())
        .expect("persist blob");
    let second_ref = store
        .put_turn_checkpoint_blob("conversation", blob.clone())
        .expect("deduplicate blob");
    assert_eq!(first_ref, second_ref);
    let other_kind_ref = store
        .put_turn_checkpoint_blob("tool_catalog", blob.clone())
        .expect("domain-separate blob kind");
    assert_ne!(first_ref, other_kind_ref);
    assert_eq!(
        store
            .get_turn_checkpoint_blob(&first_ref)
            .expect("load blob"),
        Some(blob)
    );

    let checkpoint = serde_json::json!({
        "turnId": turn.turn_id,
        "phase": "external_action",
        "conversationRef": first_ref,
    });
    store
        .put_turn_checkpoint(
            turn.turn_id,
            thread.id,
            "external_action",
            checkpoint.clone(),
        )
        .expect("persist checkpoint");
    assert_eq!(
        store
            .get_turn_checkpoint(turn.turn_id, thread.id)
            .expect("load checkpoint"),
        Some(("external_action".to_string(), checkpoint))
    );
    assert!(store
        .delete_turn_checkpoint(turn.turn_id, thread.id)
        .expect("delete checkpoint"));
    assert!(store
        .get_turn_checkpoint(turn.turn_id, thread.id)
        .expect("checkpoint deleted")
        .is_none());
}

#[test]
fn structured_input_and_external_action_resume_the_same_turn() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/resume-boundaries"))
        .expect("create thread");
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
        .expect("insert turn");
    store
        .update_turn_status(turn.turn_id, TurnStatus::WaitingUserInput, None)
        .expect("wait for input");
    let resumed = store
        .resume_turn_invocation(turn.turn_id)
        .expect("resume input")
        .expect("resumable turn");
    assert_eq!(resumed.turn_id, turn.turn_id);
    assert_eq!(resumed.invocation_id, 2);
    store
        .update_turn_status(turn.turn_id, TurnStatus::WaitingUserAction, None)
        .expect("wait for action");
    let resumed = store
        .resume_turn_invocation(turn.turn_id)
        .expect("resume action")
        .expect("resumable turn");
    assert_eq!(resumed.turn_id, turn.turn_id);
    assert_eq!(resumed.invocation_id, 3);
}

#[test]
fn migration_repairs_v18_turn_constraints_without_losing_turns() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/v18-turn-migration"))
        .expect("create thread");
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
        .expect("insert turn");
    {
        let conn = store.conn.lock().expect("lock store");
        remove_post_legacy_agent_runtime_tables(&conn);
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = OFF;
            CREATE TABLE turns_legacy_v18 (
                turn_id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                user_message_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                error TEXT,
                invocation_id INTEGER NOT NULL DEFAULT 1,
                CHECK(status IN (
                    'running', 'waiting_approval', 'cancelling', 'succeeded',
                    'failed', 'cancelled', 'interrupted'
                )),
                FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );
            INSERT INTO turns_legacy_v18 (
                turn_id, thread_id, user_message_id, status, started_at,
                updated_at, completed_at, error, invocation_id
            )
            SELECT turn_id, thread_id, user_message_id, status, started_at,
                   updated_at, completed_at, error, invocation_id
            FROM turns;
            DROP TABLE turns;
            ALTER TABLE turns_legacy_v18 RENAME TO turns;
            CREATE INDEX idx_turns_thread_started
                ON turns(thread_id, started_at DESC);
            CREATE UNIQUE INDEX idx_turns_thread_active
                ON turns(thread_id)
                WHERE status IN ('running', 'cancelling');
            DROP TABLE agent_mailbox_messages;
            DROP TABLE agent_ledger_items;
            DROP TABLE agent_turns;
            DROP TABLE agent_threads;
            DROP TABLE agent_runtime_snapshots;
            DROP TABLE agent_sessions;
            DROP TABLE schema_migrations;
            PRAGMA user_version = 18;
            PRAGMA foreign_keys = ON;
            "#,
        )
        .expect("restore v18 ledger with legacy turns constraint");
        assert!(
            !turns_table_supports_waiting_boundaries(&conn).expect("inspect legacy turns table")
        );
    }

    store.migrate().expect("reconcile v18 turns schema");

    let restored = store
        .get_turn(turn.turn_id)
        .expect("read preserved turn")
        .expect("turn remains after migration");
    assert_eq!(restored.user_message_id, turn.user_message_id);
    store
        .update_turn_status(turn.turn_id, TurnStatus::WaitingUserInput, None)
        .expect("new waiting status is accepted");
    let conn = store.conn.lock().expect("lock reconciled store");
    let schema_version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read schema version");
    assert_eq!(schema_version, CURRENT_DATABASE_SCHEMA_VERSION);
    assert!(turns_table_supports_waiting_boundaries(&conn).expect("inspect reconciled turns table"));
}
