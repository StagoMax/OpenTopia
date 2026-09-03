use super::SqliteCollaborationRepository;
use crate::collaboration::{
    test_runtime_snapshot, AgentMailbox, AgentMailboxMessageKind, AgentSpawnPolicy,
    AgentThreadRecord, AgentTurnId, AgentTurnRecord, AgentTurnStatus, CollaborationRegistry,
    CollaborationSessionPolicy, CollaborationSessionRecord, CreateCollaborationSession,
    EnqueueAgentMessage, RuntimeSnapshotSeed, RuntimeWorkspaceModeV1, SpawnAgentThread,
};
use crate::model::AgentEventPayload;
use crate::store::{SessionStore, SqliteSessionStore};
use rusqlite::params;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

async fn fixture() -> (
    Arc<SqliteSessionStore>,
    SqliteCollaborationRepository,
    CollaborationSessionRecord,
    AgentThreadRecord,
    AgentTurnRecord,
) {
    let store = Arc::new(SqliteSessionStore::open(":memory:").unwrap());
    let user_thread = store
        .create_thread(None, PathBuf::from("C:/workspace/collaboration-sqlite"))
        .unwrap();
    let repository = SqliteCollaborationRepository::new(store.clone()).unwrap();
    let (session, root, root_turn) = repository
        .create_session(CreateCollaborationSession {
            user_task_id: user_thread.id,
            root_turn_id: AgentTurnId::new(),
            root_task_message: "root task".to_string(),
            root_agent_type: "default".to_string(),
            root_runtime_snapshot: RuntimeSnapshotSeed::new(
                None,
                test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated),
            ),
            session_policy: CollaborationSessionPolicy {
                max_agents: 8,
                max_active_runs: 4,
                max_depth: 2,
            },
            root_spawn_policy: AgentSpawnPolicy::allows_children(2, 4),
        })
        .await
        .unwrap();
    (store, repository, session, root, root_turn)
}

#[tokio::test]
async fn activity_event_batches_receive_contiguous_session_sequences() {
    let (_store, repository, session, root, root_turn) = fixture().await;
    let events = repository
        .append_activity_events(
            session.id,
            root.id,
            root_turn.id,
            root_turn.invocation_id,
            vec![
                AgentEventPayload::ReasoningDelta {
                    text: "one".to_string(),
                    provider_attempt: None,
                },
                AgentEventPayload::ModelDelta {
                    text: "two".to_string(),
                    provider_attempt: None,
                },
            ],
            None,
        )
        .expect("append activity batch");

    assert_eq!(
        events.iter().map(|event| event.seq).collect::<Vec<_>>(),
        [1, 2]
    );
    let listed = repository
        .list_activity_events(root.id, root_turn.id, None)
        .expect("list activity batch");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, events[0].id);
    assert_eq!(listed[1].id, events[1].id);
}

#[tokio::test]
async fn activity_summary_materializes_and_reads_a_bounded_reasoning_tail() {
    let (store, repository, session, root, root_turn) = fixture().await;
    let events = repository
        .append_activity_events(
            session.id,
            root.id,
            root_turn.id,
            root_turn.invocation_id,
            vec![
                AgentEventPayload::ReasoningDelta {
                    text: "one".to_string(),
                    provider_attempt: None,
                },
                AgentEventPayload::ReasoningDelta {
                    text: "two".to_string(),
                    provider_attempt: None,
                },
            ],
            None,
        )
        .expect("append reasoning events");
    store
        .with_collaboration_write(|connection| {
            connection.execute(
                "DELETE FROM agent_activity_state WHERE agent_turn_id = ?1",
                params![root_turn.id.to_string()],
            )?;
            Ok(())
        })
        .expect("remove eager activity state");

    let summary = repository
        .activity_summary(root.id, root_turn.id, None, 4)
        .expect("read activity summary");
    assert_eq!(summary.cursor, events[1].seq);
    assert_eq!(summary.reasoning_tail.as_deref(), Some("etwo"));

    let incremental = repository
        .activity_summary(root.id, root_turn.id, Some(events[0].seq), 16)
        .expect("read incremental activity summary");
    assert_eq!(incremental.reasoning_tail.as_deref(), Some("two"));

    let state_rows = store
        .with_collaboration_read(|connection| {
            connection
                .query_row(
                    "SELECT COUNT(*) FROM agent_activity_state WHERE agent_turn_id = ?1",
                    params![root_turn.id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(Into::into)
        })
        .expect("count materialized state rows");
    assert_eq!(state_rows, 1);
}

#[tokio::test]
async fn sqlite_registry_persists_recursive_identity_turns_and_snapshots() {
    let (_store, repository, session, root, root_turn) = fixture().await;
    let (child, child_turn) = repository
        .spawn_agent(SpawnAgentThread {
            parent_agent_thread_id: root.id,
            requested_by_turn_id: root_turn.id,
            task_name: "research".to_string(),
            agent_type: "explorer".to_string(),
            task_message: "inspect".to_string(),
            runtime_snapshot: RuntimeSnapshotSeed::new(
                Some(root.runtime_snapshot_id),
                test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
            ),
            spawn_policy: AgentSpawnPolicy::allows_children(2, 2),
        })
        .await
        .unwrap();
    let (grandchild, _) = repository
        .spawn_agent(SpawnAgentThread {
            parent_agent_thread_id: child.id,
            requested_by_turn_id: child_turn.id,
            task_name: "reviewer".to_string(),
            agent_type: "explorer".to_string(),
            task_message: "review".to_string(),
            runtime_snapshot: RuntimeSnapshotSeed::new(
                Some(child.runtime_snapshot_id),
                test_runtime_snapshot("explorer", RuntimeWorkspaceModeV1::SharedCoordinated),
            ),
            spawn_policy: AgentSpawnPolicy::disabled(2),
        })
        .await
        .unwrap();

    assert_eq!(grandchild.path.as_str(), "/root/research/reviewer");
    assert_eq!(repository.list_threads(session.id).await.unwrap().len(), 3);
    assert_eq!(
        repository
            .get_runtime_snapshot(child.runtime_snapshot_id)
            .await
            .unwrap()
            .parent_snapshot_id,
        Some(root.runtime_snapshot_id)
    );
}

#[tokio::test]
async fn root_followup_rotates_the_frozen_runtime_snapshot() {
    let (_store, repository, session, root, root_turn) = fixture().await;
    repository
        .transition_turn(root_turn.id, AgentTurnStatus::Running)
        .await
        .unwrap();
    repository
        .transition_turn(root_turn.id, AgentTurnStatus::Completed)
        .await
        .unwrap();

    let next_turn_id = AgentTurnId::new();
    let (updated_root, next_turn) = repository
        .create_root_followup_turn(
            session.id,
            next_turn_id,
            "next root turn",
            RuntimeSnapshotSeed::new(
                None,
                test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedReadOnly),
            ),
        )
        .unwrap();

    assert_ne!(updated_root.runtime_snapshot_id, root.runtime_snapshot_id);
    assert_eq!(next_turn.id, next_turn_id);
    assert_eq!(
        repository
            .get_runtime_snapshot(updated_root.runtime_snapshot_id)
            .await
            .unwrap()
            .parent_snapshot_id,
        Some(root.runtime_snapshot_id)
    );
    assert_eq!(
        repository
            .get_thread(root.id)
            .await
            .unwrap()
            .runtime_snapshot_id,
        updated_root.runtime_snapshot_id
    );
}

#[tokio::test]
async fn sqlite_mailbox_is_idempotent_and_acknowledged_transactionally() {
    let (_store, repository, session, root, root_turn) = fixture().await;
    let (child, _) = repository
        .spawn_agent(SpawnAgentThread {
            parent_agent_thread_id: root.id,
            requested_by_turn_id: root_turn.id,
            task_name: "worker".to_string(),
            agent_type: "worker".to_string(),
            task_message: "work".to_string(),
            runtime_snapshot: RuntimeSnapshotSeed::new(
                Some(root.runtime_snapshot_id),
                test_runtime_snapshot("worker", RuntimeWorkspaceModeV1::SharedCoordinated),
            ),
            spawn_policy: AgentSpawnPolicy::disabled(2),
        })
        .await
        .unwrap();
    let causation_id = Uuid::new_v4();
    let request = || EnqueueAgentMessage {
        session_id: session.id,
        from_agent_thread_id: root.id,
        to_agent_thread_id: child.id,
        kind: AgentMailboxMessageKind::Message,
        payload: json!({ "text": "hello" }),
        causation_id: Some(causation_id),
    };
    let first = repository.enqueue(request()).await.unwrap();
    let duplicate = repository.enqueue(request()).await.unwrap();
    assert_eq!(first.id, duplicate.id);

    repository.acknowledge(child.id, &[first.id]).await.unwrap();
    assert!(repository
        .snapshot(session.id, child.id, None, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn recoverable_turn_query_distinguishes_queued_work_from_interrupted_runs() {
    let (_store, repository, _session, root, root_turn) = fixture().await;
    repository
        .transition_turn(root_turn.id, AgentTurnStatus::Running)
        .await
        .unwrap();
    let (_child, child_turn) = repository
        .spawn_agent(SpawnAgentThread {
            parent_agent_thread_id: root.id,
            requested_by_turn_id: root_turn.id,
            task_name: "recoverable".to_string(),
            agent_type: "worker".to_string(),
            task_message: "resume after restart".to_string(),
            runtime_snapshot: RuntimeSnapshotSeed::new(
                Some(root.runtime_snapshot_id),
                test_runtime_snapshot("worker", RuntimeWorkspaceModeV1::SharedCoordinated),
            ),
            spawn_policy: AgentSpawnPolicy::disabled(2),
        })
        .await
        .unwrap();

    let recoverable = repository.list_recoverable_turns().unwrap();
    assert_eq!(
        recoverable
            .iter()
            .map(|turn| (turn.id, turn.status))
            .collect::<Vec<_>>(),
        [
            (root_turn.id, AgentTurnStatus::Running),
            (child_turn.id, AgentTurnStatus::Queued)
        ]
    );

    repository
        .record_turn_state(
            root_turn.id,
            AgentTurnStatus::Interrupted,
            &json!({ "reason": "simulated server restart" }),
        )
        .unwrap();
    let recoverable = repository.list_recoverable_turns().unwrap();
    assert_eq!(recoverable.len(), 1);
    assert_eq!(recoverable[0].id, child_turn.id);
    assert_eq!(recoverable[0].status, AgentTurnStatus::Queued);
}

#[tokio::test]
async fn sqlite_collaboration_state_survives_store_reopen() {
    let path =
        std::env::temp_dir().join(format!("opentopia-collaboration-{}.sqlite", Uuid::new_v4()));
    let (session_id, child_id, message_id) = {
        let store = Arc::new(SqliteSessionStore::open(&path).unwrap());
        let user_thread = store
            .create_thread(None, PathBuf::from("C:/workspace/collaboration-reopen"))
            .unwrap();
        let repository = SqliteCollaborationRepository::new(store.clone()).unwrap();
        let (session, root, root_turn) = repository
            .create_session(CreateCollaborationSession {
                user_task_id: user_thread.id,
                root_turn_id: AgentTurnId::new(),
                root_task_message: "root task".to_string(),
                root_agent_type: "default".to_string(),
                root_runtime_snapshot: RuntimeSnapshotSeed::new(
                    None,
                    test_runtime_snapshot("default", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                session_policy: CollaborationSessionPolicy {
                    max_agents: 4,
                    max_active_runs: 2,
                    max_depth: 1,
                },
                root_spawn_policy: AgentSpawnPolicy::allows_children(1, 2),
            })
            .await
            .unwrap();
        let (child, _) = repository
            .spawn_agent(SpawnAgentThread {
                parent_agent_thread_id: root.id,
                requested_by_turn_id: root_turn.id,
                task_name: "worker".to_string(),
                agent_type: "worker".to_string(),
                task_message: "work".to_string(),
                runtime_snapshot: RuntimeSnapshotSeed::new(
                    Some(root.runtime_snapshot_id),
                    test_runtime_snapshot("worker", RuntimeWorkspaceModeV1::SharedCoordinated),
                ),
                spawn_policy: AgentSpawnPolicy::disabled(1),
            })
            .await
            .unwrap();
        let message = repository
            .enqueue(EnqueueAgentMessage {
                session_id: session.id,
                from_agent_thread_id: root.id,
                to_agent_thread_id: child.id,
                kind: AgentMailboxMessageKind::Message,
                payload: json!({ "text": "persist me" }),
                causation_id: Some(Uuid::new_v4()),
            })
            .await
            .unwrap();
        (session.id, child.id, message.id)
    };

    {
        let reopened_store = Arc::new(SqliteSessionStore::open(&path).unwrap());
        let repository = SqliteCollaborationRepository::new(reopened_store).unwrap();
        assert_eq!(repository.get_thread(child_id).await.unwrap().id, child_id);
        let pending = repository
            .snapshot(session_id, child_id, None, 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, message_id);
    }

    let _ = std::fs::remove_file(path);
}
