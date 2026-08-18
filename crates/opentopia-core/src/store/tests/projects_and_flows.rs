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
fn thread_experience_modes_round_trip_and_filter_on_the_server() {
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

    let flow_thread = store
        .create_thread_in_project_with_mode(
            Some("Flow design".to_string()),
            project.id,
            ExperienceMode::Flow,
        )
        .expect("create flow thread");
    let loaded = store
        .get_thread(flow_thread.id)
        .expect("load flow thread")
        .expect("flow thread exists");
    assert_eq!(loaded.experience_mode, ExperienceMode::Flow);
    let flow_threads = store
        .list_threads_for_mode(false, ExperienceMode::Flow)
        .expect("list flow threads");
    assert_eq!(flow_threads.len(), 1);
    assert_eq!(flow_threads[0].id, flow_thread.id);
    assert!(store
        .list_threads_for_mode(false, ExperienceMode::Code)
        .expect("list code threads")
        .iter()
        .all(|thread| thread.experience_mode == ExperienceMode::Code));
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
        adapter_identity: "openai_responses".to_string(),
        response_id: "resp_123".to_string(),
        compatibility_hash: "compatible".to_string(),
        response_items: vec![serde_json::json!({ "type": "compaction", "id": "cmp_123" })],
        state_kind: ProviderContextStateKind::Hybrid,
        compaction_item_count: 1,
        checkpoint_id: Some(Uuid::new_v4()),
        updated_at: Utc::now(),
    };

    store
        .save_provider_conversation_state(&state)
        .expect("save state");
    assert_eq!(
        store
            .get_provider_conversation_state(thread.id, "/root")
            .expect("read state without consuming it"),
        Some(state.clone())
    );
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

#[test]
fn flow_drafts_require_current_validation_and_trial_before_immutable_publish() {
    use crate::enterprise::{AgentRiskClassV1, CapabilityProjection};
    use crate::flow::{
        simulate_flow, validate_flow_spec, FlowBudgetV1, FlowSourceV1, FlowSpecV1,
        GraphDefinitionV1, GraphNodeKindV1, GraphNodeV1,
    };
    use std::collections::BTreeSet;

    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread_with_mode(
            Some("flow design".to_string()),
            PathBuf::from("."),
            ExperienceMode::Flow,
        )
        .expect("create Flow thread");
    let capabilities = CapabilityProjection::deny_all();
    let spec = FlowSpecV1 {
        flow_id: "evidence-return".to_string(),
        name: "Evidence return".to_string(),
        description: "Return a verified result".to_string(),
        owner: "operations".to_string(),
        categories: BTreeSet::new(),
        source: FlowSourceV1::NaturalLanguage {
            description: "return the final result".to_string(),
        },
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        graph: GraphDefinitionV1 {
            schema_version: 1,
            entry_node_id: "output".to_string(),
            nodes: vec![GraphNodeV1 {
                id: "output".to_string(),
                label: "Return result".to_string(),
                kind: GraphNodeKindV1::Output,
                config: serde_json::json!({}),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
            }],
            edges: Vec::new(),
        },
        requested_capabilities: capabilities.clone(),
        budget: FlowBudgetV1::default(),
        risk_class: AgentRiskClassV1::Low,
        pending_decisions: Vec::new(),
    };
    let mut draft = FlowDraftV1::new(thread.id, spec, &capabilities);
    store.create_flow_draft(&draft).expect("persist draft");
    assert!(matches!(
        store
            .publish_flow_draft(draft.id, "reviewer")
            .unwrap_err()
            .downcast_ref::<FlowStoreError>(),
        Some(FlowStoreError::ValidationRequired)
    ));

    let report = validate_flow_spec(&draft.spec, &capabilities);
    assert!(report.valid);
    draft.last_validation = Some(report);
    draft.status = FlowDraftStatusV1::ReadyToPublish;
    draft.updated_at = Utc::now();
    store
        .update_flow_draft(&draft, draft.revision)
        .expect("persist validation");
    let trial = simulate_flow(&draft, serde_json::json!({}), &capabilities);
    store.insert_flow_trial(&trial).expect("persist trial");

    let definition = store
        .publish_flow_draft(draft.id, "reviewer")
        .expect("publish Flow");
    assert_eq!(definition.flow_id, "evidence-return");
    assert_eq!(definition.version, 1);
    assert_eq!(
        store
            .get_flow_definition("evidence-return", None)
            .expect("load definition")
            .expect("definition")
            .content_hash,
        definition.content_hash
    );
}

#[test]
fn goal_snapshot_and_lifecycle_are_projected_from_one_work_form() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/work-form-goal"))
        .expect("create thread");
    let created = store
        .create_goal(thread.id, "Ship the goal".into(), None)
        .expect("create goal");
    let scope = WorkScope::Goal(created.goal.id);
    let form = store
        .get_work_form_for_scope(scope)
        .expect("read form")
        .expect("goal form");
    assert_eq!(form.id, scope.form_id());
    assert!(form.items.is_empty());

    let mut form = WorkForm::new(
        thread.id,
        scope,
        created.work_form.objective.clone(),
        vec![crate::work_form::WorkItem {
            id: "implement".into(),
            title: "Implement".into(),
            status: WorkItemStatus::Pending,
            completion_disposition: crate::completion_runtime::CompletionDisposition::Blocking,
            note: None,
            depends_on: Vec::new(),
            acceptance: vec!["works".into()],
            evidence_refs: Vec::new(),
        }],
    );
    form.revision = 1;
    form.change_reason = Some("commit work".into());
    store.upsert_work_form(&form).expect("persist WorkForm");
    let snapshot = store
        .get_goal(created.goal.id)
        .expect("load goal")
        .expect("goal");
    assert_eq!(snapshot.work_form.items.len(), 1);
    assert_eq!(snapshot.work_form.items[0].id, "implement");
    assert_eq!(snapshot.work_form.revision, 1);

    let edited = store
        .update_goal_definition(
            thread.id,
            created.goal.id,
            Some("Ship the revised goal".into()),
            Some(vec!["Preserve compatibility".into()]),
            Some(vec!["All blocking work is verified".into()]),
        )
        .expect("edit goal definition")
        .expect("goal");
    assert_eq!(edited.work_form.objective, "Ship the revised goal");
    assert_eq!(edited.goal.objective, edited.work_form.objective);
    assert_eq!(edited.work_form.constraints.len(), 1);
    assert_eq!(edited.work_form.acceptance.len(), 1);

    let paused = store
        .update_goal_status(thread.id, created.goal.id, GoalStatus::Paused)
        .expect("pause goal")
        .expect("goal");
    assert_eq!(paused.status(), GoalStatus::Paused);
    assert_eq!(
        store
            .get_work_form_for_scope(scope)
            .unwrap()
            .unwrap()
            .status,
        WorkFormStatus::Paused
    );
    let resumed = store
        .update_goal_status(thread.id, created.goal.id, GoalStatus::Active)
        .expect("resume goal")
        .expect("goal");
    assert_eq!(resumed.status(), GoalStatus::Active);
}

#[test]
fn default_turn_forms_and_goal_forms_share_schema_without_sharing_state() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/work-form-turns"))
        .expect("create thread");
    let first_scope = WorkScope::Turn(Uuid::new_v4());
    let second_scope = WorkScope::Turn(Uuid::new_v4());
    for scope in [first_scope, second_scope] {
        store
            .upsert_work_form(&WorkForm::new(
                thread.id,
                scope,
                "same-model-namespace",
                Vec::new(),
            ))
            .expect("persist turn form");
    }
    assert_ne!(first_scope.form_id(), second_scope.form_id());
    assert!(store
        .get_work_form_for_scope(first_scope)
        .unwrap()
        .is_some());
    assert!(store
        .get_work_form_for_scope(second_scope)
        .unwrap()
        .is_some());
}
