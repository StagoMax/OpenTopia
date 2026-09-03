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
            provider_attempt: None,
        },
    );

    let stored = store.append_event(event).expect("append reasoning event");
    assert_eq!(stored.kind(), "reasoning_delta");

    let events = store.list_events(thread.id, None).expect("list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].turn_id, Some(turn_id));
    match &events[0].payload {
        AgentEventPayload::ReasoningDelta { text, .. } => {
            assert_eq!(text, "正在核对依赖");
        }
        payload => panic!("unexpected payload: {payload:?}"),
    }
}

#[test]
fn sqlite_store_appends_event_batches_with_contiguous_sequences() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/event-batches"))
        .expect("create thread");
    let turn_id = Uuid::new_v4();
    let stored = store
        .append_events(vec![
            AgentEvent::new(
                thread.id,
                Some(turn_id),
                0,
                AgentEventPayload::ReasoningDelta {
                    text: "first".to_string(),
                    provider_attempt: None,
                },
            ),
            AgentEvent::new(
                thread.id,
                Some(turn_id),
                0,
                AgentEventPayload::ModelDelta {
                    text: "second".to_string(),
                    provider_attempt: None,
                },
            ),
        ])
        .expect("append event batch");

    assert_eq!(
        stored.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [1, 2]
    );
    let listed = store
        .list_events(thread.id, None)
        .expect("list event batch");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, stored[0].id);
    assert_eq!(listed[1].id, stored[1].id);
}

#[test]
fn sqlite_store_commits_messages_and_events_in_one_conversation_batch() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/conversation-batch"))
        .expect("create thread");
    let message = Message::text(thread.id, MessageRole::Tool, "tool output");
    let stored = store
        .append_conversation_batch(
            vec![message.clone()],
            vec![AgentEvent::new(
                thread.id,
                Some(Uuid::new_v4()),
                0,
                AgentEventPayload::ModelDelta {
                    text: "progress".to_string(),
                    provider_attempt: None,
                },
            )],
        )
        .expect("append conversation batch");

    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].seq, 1);
    let messages = store.list_messages(thread.id).expect("list messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, message.id);
    assert_eq!(
        store.list_events(thread.id, None).expect("list events")[0].id,
        stored[0].id
    );
}

#[test]
fn conversation_history_pages_keep_stable_message_and_event_order() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/history-pages"))
        .expect("create thread");
    let base = Utc::now() - chrono::Duration::minutes(10);
    let mut messages = Vec::new();
    for index in 0..5 {
        let mut message = Message::text(thread.id, MessageRole::User, index.to_string());
        message.created_at = base + chrono::Duration::seconds(index);
        messages.push(store.append_message(message).expect("append message"));
    }
    let events = store
        .append_events(
            (0..5)
                .map(|index| {
                    AgentEvent::new(
                        thread.id,
                        Some(Uuid::new_v4()),
                        0,
                        AgentEventPayload::ModelDelta {
                            text: index.to_string(),
                            provider_attempt: None,
                        },
                    )
                })
                .collect(),
        )
        .expect("append events");

    let recent_messages = store
        .list_conversation_message_page(thread.id, None, None, 2)
        .expect("list recent messages");
    assert_eq!(
        recent_messages.iter().map(|message| message.id).collect::<Vec<_>>(),
        messages[3..].iter().map(|message| message.id).collect::<Vec<_>>()
    );
    let older_messages = store
        .list_conversation_message_page(
            thread.id,
            None,
            Some((messages[3].created_at, messages[3].id)),
            2,
        )
        .expect("list older messages");
    assert_eq!(
        older_messages.iter().map(|message| message.id).collect::<Vec<_>>(),
        messages[1..3].iter().map(|message| message.id).collect::<Vec<_>>()
    );
    let message_delta = store
        .list_conversation_message_page(
            thread.id,
            Some((messages[1].created_at, messages[1].id)),
            None,
            2,
        )
        .expect("list message delta");
    assert_eq!(
        message_delta.iter().map(|message| message.id).collect::<Vec<_>>(),
        messages[2..4].iter().map(|message| message.id).collect::<Vec<_>>()
    );

    let recent_events = store
        .list_conversation_event_page(thread.id, None, None, 2)
        .expect("list recent events");
    assert_eq!(
        recent_events.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [events[3].seq, events[4].seq]
    );
    let older_events = store
        .list_conversation_event_page(thread.id, None, Some(events[3].seq), 2)
        .expect("list older events");
    assert_eq!(
        older_events.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [events[1].seq, events[2].seq]
    );
    let event_delta = store
        .list_conversation_event_page(thread.id, Some(events[1].seq), None, 2)
        .expect("list event delta");
    assert_eq!(
        event_delta.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [events[2].seq, events[3].seq]
    );

    for index in 0..3 {
        let mut tool_message =
            Message::text(thread.id, MessageRole::Tool, format!("tool-{index}"));
        tool_message.created_at = base + chrono::Duration::minutes(index + 1);
        store
            .append_message(tool_message)
            .expect("append tool message");
    }
    let recent_visible_messages = store
        .list_conversation_message_page(thread.id, None, None, 2)
        .expect("list recent visible messages");
    assert_eq!(
        recent_visible_messages
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        messages[3..]
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn live_turn_status_snapshot_excludes_terminal_turns() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let running_thread = store
        .create_thread(None, PathBuf::from("C:/workspace/live-running"))
        .expect("create running thread");
    let waiting_thread = store
        .create_thread(None, PathBuf::from("C:/workspace/live-waiting"))
        .expect("create waiting thread");
    let completed_thread = store
        .create_thread(None, PathBuf::from("C:/workspace/live-completed"))
        .expect("create completed thread");
    let running = store
        .insert_turn(TurnRecord::running(running_thread.id, Uuid::new_v4()))
        .expect("insert running turn");
    let waiting = store
        .insert_turn(TurnRecord::running(waiting_thread.id, Uuid::new_v4()))
        .expect("insert waiting turn");
    store
        .update_turn_status(waiting.turn_id, TurnStatus::WaitingUserInput, None)
        .expect("update waiting turn");
    let completed = store
        .insert_turn(TurnRecord::running(completed_thread.id, Uuid::new_v4()))
        .expect("insert completed turn");
    store
        .update_turn_status(completed.turn_id, TurnStatus::Succeeded, None)
        .expect("complete turn");

    let statuses = store
        .list_live_turn_statuses()
        .expect("list live turn statuses");
    let turn_ids = statuses
        .iter()
        .map(|turn| turn.turn_id)
        .collect::<HashSet<_>>();

    assert_eq!(turn_ids, HashSet::from([running.turn_id, waiting.turn_id]));
}

#[test]
fn completed_assistant_message_replaces_historical_stream_in_conversation_view() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/conversation-snapshot"))
        .expect("create thread");
    let turn_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let message = Message::text(thread.id, MessageRole::Assistant, "complete answer");
    store
        .append_events(vec![
            AgentEvent::new(
                thread.id,
                Some(turn_id),
                0,
                AgentEventPayload::ProviderFirstTokenReceived { request_id },
            ),
            AgentEvent::new(
                thread.id,
                Some(turn_id),
                0,
                AgentEventPayload::ModelDelta {
                    text: "partial ".to_string(),
                    provider_attempt: None,
                },
            ),
            AgentEvent::new(
                thread.id,
                Some(turn_id),
                0,
                AgentEventPayload::AssistantMessage { message },
            ),
        ])
        .expect("append completed stream");

    let diagnostic_events = store
        .list_events(thread.id, None)
        .expect("list full events");
    assert_eq!(diagnostic_events.len(), 3);
    let conversation_events = store
        .list_conversation_events(thread.id, None)
        .expect("list conversation projection");
    assert_eq!(conversation_events.len(), 2);
    assert!(matches!(
        conversation_events[0].payload,
        AgentEventPayload::ProviderFirstTokenReceived {
            request_id: projected_request_id
        } if projected_request_id == request_id
    ));
    assert!(matches!(
        conversation_events[1].payload,
        AgentEventPayload::AssistantMessage { .. }
    ));
}

#[test]
fn conversation_retry_event_preserves_reconnect_progress() {
    let request_id = Uuid::new_v4();
    let cache_trace = crate::model::ProviderCacheTrace {
        schema_version: 1,
        prefix_hash: "prefix-hash".to_string(),
        segments: vec![crate::model::ProviderCacheTraceSegment {
            kind: crate::model::ProviderCacheTraceSegmentKind::ToolResult,
            source: "messages[2]".to_string(),
            name: Some("filesystem".to_string()),
            content_hash: "content-hash".to_string(),
            token_estimate: 42,
        }],
        tool_catalog_hash: Some("tools-hash".to_string()),
        prompt_cache_key_hash: None,
        previous_response_id_present: false,
        configuration: Vec::new(),
    };
    let payload = AgentEventPayload::ProviderRequestRetried {
        request_id,
        round: 2,
        attempt: 4,
        retry_kind: crate::model::ProviderRetryKind::Network,
        retry_index: Some(3),
        retry_limit: Some(5),
        reason: "connection reset".to_string(),
        cache_trace: Some(cache_trace.clone()),
        body: serde_json::json!({"secret": "removed"}),
    };
    let full = serde_json::to_string(&payload).expect("serialize full payload");
    let compact = conversation_payload_json(Uuid::new_v4(), &payload, &full)
        .expect("project conversation payload")
        .expect("retry event remains visible");
    let projected: AgentEventPayload =
        serde_json::from_str(&compact).expect("deserialize projected retry payload");

    assert!(matches!(
        projected,
        AgentEventPayload::ProviderRequestRetried {
            request_id: projected_request_id,
            retry_kind: crate::model::ProviderRetryKind::Network,
            retry_index: Some(3),
            retry_limit: Some(5),
            cache_trace: Some(projected_cache_trace),
            body,
            ..
        } if projected_request_id == request_id
            && body.is_null()
            && projected_cache_trace == cache_trace
    ));
}

#[test]
fn conversation_event_view_removes_diagnostic_bodies_and_hidden_reasoning() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/conversation-events"))
        .expect("create thread");
    let turn_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();

    for payload in [
        AgentEventPayload::ModelRequest {
            request_id,
            round: 1,
            request: serde_json::json!({"prompt": "x".repeat(32_000)}),
        },
        AgentEventPayload::ProviderRequestSent {
            request_id,
            round: 1,
            attempt: 1,
            adapter: "test".to_string(),
            method: "POST".to_string(),
            endpoint: "http://localhost/model".to_string(),
            cache_trace: None,
            body: serde_json::json!({"input": "y".repeat(32_000)}),
            checkpoint: None,
        },
        AgentEventPayload::ReasoningDelta {
            text: "hidden historical reasoning".to_string(),
            provider_attempt: None,
        },
        AgentEventPayload::ToolCallFinished {
            result: ToolResult::text(
                Uuid::new_v4(),
                "tool output".repeat(1_000),
                serde_json::json!({}),
            ),
        },
        AgentEventPayload::TokenUsage {
            request_id: None,
            round: None,
            purpose: crate::model::ModelCallPurpose::AgentRound,
            input_tokens: 10,
            output_tokens: 5,
            total_tokens: 15,
            cached_input_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
            local_input_estimate: None,
            input_breakdown: None,
        },
        AgentEventPayload::TurnFinished {
            summary: "done".to_string(),
        },
    ] {
        store
            .append_event(AgentEvent::new(thread.id, Some(turn_id), 0, payload))
            .expect("append event");
    }

    let raw = store.list_events(thread.id, None).expect("list raw events");
    assert_eq!(raw.len(), 6);
    assert!(matches!(
        &raw[0].payload,
        AgentEventPayload::ModelRequest { request, .. }
            if request.get("prompt").is_some()
    ));

    let conversation = store
        .list_conversation_events(thread.id, None)
        .expect("list conversation events");
    assert_eq!(conversation.len(), 5);
    assert!(matches!(
        &conversation[0].payload,
        AgentEventPayload::ModelRequest { request, .. } if request.is_null()
    ));
    assert!(matches!(
        &conversation[1].payload,
        AgentEventPayload::ProviderRequestSent { body, .. } if body.is_null()
    ));
    assert!(matches!(
        &conversation[2].payload,
        AgentEventPayload::ToolCallFinished { result }
            if result.content.is_empty()
                && result.output.len() < 11_000
                && result.metadata[crate::CONVERSATION_TOOL_DETAIL_METADATA_KEY]["eventId"]
                    == serde_json::json!(raw[3].id)
    ));
    assert!(conversation
        .iter()
        .all(|event| !matches!(event.payload, AgentEventPayload::ReasoningDelta { .. })));

    let turn_tool_results = store
        .list_turn_tool_result_events(thread.id, turn_id)
        .expect("list one turn's tool results");
    assert_eq!(turn_tool_results.len(), 1);
    assert!(matches!(
        &turn_tool_results[0].payload,
        AgentEventPayload::ToolCallFinished { result }
            if result.content.is_empty() && result.output.len() < 11_000
    ));

    let full_tool_event = store
        .get_event(thread.id, raw[3].id)
        .expect("load canonical tool event")
        .expect("canonical tool event exists");
    assert!(matches!(
        full_tool_event.payload,
        AgentEventPayload::ToolCallFinished { result } if result.output.len() == 11_000
    ));
    assert!(store
        .get_event(Uuid::new_v4(), raw[3].id)
        .expect("scope canonical event lookup to its thread")
        .is_none());

    let context = store
        .list_context_events(thread.id)
        .expect("list context events");
    assert_eq!(context.len(), 1);
    assert!(matches!(
        context[0].payload,
        AgentEventPayload::TokenUsage { .. }
    ));
    assert_eq!(
        store
            .count_events_after(thread.id, 2)
            .expect("count later events"),
        4
    );
}

#[test]
fn migration_backfills_conversation_events_for_existing_databases() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/event-backfill"))
        .expect("create thread");
    store
        .append_event(AgentEvent::new(
            thread.id,
            None,
            0,
            AgentEventPayload::TurnFinished {
                summary: "existing event".to_string(),
            },
        ))
        .expect("append event");
    {
        let conn = store.conn.lock().expect("lock store");
        remove_post_legacy_agent_runtime_tables(&conn);
        conn.execute("DELETE FROM conversation_events", [])
            .expect("remove projected rows");
        conn.execute_batch(
            r#"
            DROP TABLE agent_mailbox_messages;
            DROP TABLE agent_ledger_items;
            DROP TABLE agent_turns;
            DROP TABLE agent_threads;
            DROP TABLE agent_runtime_snapshots;
            DROP TABLE agent_sessions;
            DROP TABLE schema_migrations;
            PRAGMA user_version = 8;
            "#,
        )
        .expect("restore previous schema version");
    }

    store.migrate().expect("rerun migration");

    let events = store
        .list_conversation_events(thread.id, None)
        .expect("list backfilled events");
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].payload,
        AgentEventPayload::TurnFinished { .. }
    ));
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
fn effect_journal_deduplicates_intent_and_recovers_running_effects() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/effect-journal"))
        .expect("create thread");
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
        .expect("insert turn");
    let intent = EffectIntent {
        thread_id: thread.id,
        turn_id: turn.turn_id,
        agent_path: "/root".to_string(),
        idempotency_key: format!("{}/tool/call-1", turn.turn_id),
        kind: EffectKind::ToolCall,
        operation: "send_message".to_string(),
        input_hash: "input-v1".to_string(),
        input: serde_json::json!({ "message": "hello" }),
        side_effect_class: EffectSideEffectClass::External,
        idempotent: false,
    };

    let prepared = store.prepare_effect(&intent).expect("prepare effect");
    let duplicate = store
        .prepare_effect(&intent)
        .expect("deduplicate prepared effect");
    assert_eq!(prepared.effect_id, duplicate.effect_id);
    let running = store
        .start_effect(prepared.effect_id)
        .expect("start effect");
    assert_eq!(running.status, EffectStatus::Running);
    assert_eq!(running.attempt, 1);

    assert_eq!(
        store
            .mark_running_effects_indeterminate()
            .expect("recover running effects"),
        1
    );
    let uncertain = store
        .get_effect(prepared.effect_id)
        .expect("load effect")
        .expect("effect exists");
    assert_eq!(uncertain.status, EffectStatus::Indeterminate);
    assert!(uncertain.requires_reconciliation());
    assert_eq!(store.list_turn_effects(turn.turn_id).unwrap().len(), 1);
}

#[test]
fn effect_journal_accepts_a_workflow_execution_scope_without_a_synthetic_turn() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/workflow-effect"))
        .expect("create thread");
    let flow_run_scope = Uuid::new_v4();
    let intent = EffectIntent {
        thread_id: thread.id,
        turn_id: flow_run_scope,
        agent_path: "/root/workflow-agent".to_string(),
        idempotency_key: format!("{flow_run_scope}/send/call-1"),
        kind: EffectKind::ToolCall,
        operation: "send_message".to_string(),
        input_hash: "workflow-input-v1".to_string(),
        input: serde_json::json!({ "message": "hello" }),
        side_effect_class: EffectSideEffectClass::External,
        idempotent: false,
    };

    let receipt = store
        .prepare_effect(&intent)
        .expect("prepare Workflow activity receipt");
    assert_eq!(receipt.turn_id, flow_run_scope);
    assert!(store.get_turn(flow_run_scope).unwrap().is_none());
    assert_eq!(store.list_turn_effects(flow_run_scope).unwrap().len(), 1);
}

#[test]
fn effect_journal_rejects_idempotency_key_reuse_with_new_input() {
    let store = SqliteSessionStore::open(":memory:").expect("open memory store");
    let thread = store
        .create_thread(None, PathBuf::from("C:/workspace/effect-conflict"))
        .expect("create thread");
    let turn = store
        .insert_turn(TurnRecord::running(thread.id, Uuid::new_v4()))
        .expect("insert turn");
    let mut intent = EffectIntent {
        thread_id: thread.id,
        turn_id: turn.turn_id,
        agent_path: "/root".to_string(),
        idempotency_key: "stable-call".to_string(),
        kind: EffectKind::ToolCall,
        operation: "write_file".to_string(),
        input_hash: "first".to_string(),
        input: serde_json::json!({ "path": "a.txt" }),
        side_effect_class: EffectSideEffectClass::Workspace,
        idempotent: false,
    };
    store.prepare_effect(&intent).expect("prepare first intent");
    intent.input_hash = "second".to_string();
    let error = store
        .prepare_effect(&intent)
        .expect_err("reject changed input under the same key");
    assert!(error.to_string().contains("reused with a different"));
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

    let interrupted = store.interrupt_active_turns().expect("recover turns");
    assert_eq!(interrupted.len(), 2);
    assert_eq!(
        interrupted
            .iter()
            .map(|turn| turn.turn_id)
            .collect::<Vec<_>>(),
        vec![running.turn_id, cancelling.turn_id]
    );
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
