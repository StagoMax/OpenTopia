CREATE TABLE agent_sessions (
    id TEXT PRIMARY KEY,
    user_task_id TEXT NOT NULL UNIQUE,
    policy_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    closed_at TEXT,
    FOREIGN KEY(user_task_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE TABLE agent_runtime_snapshots (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    parent_snapshot_id TEXT,
    content_hash TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(parent_snapshot_id) REFERENCES agent_runtime_snapshots(id)
);

CREATE TABLE agent_threads (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    parent_agent_thread_id TEXT,
    agent_path TEXT NOT NULL,
    task_name TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    runtime_snapshot_id TEXT NOT NULL,
    spawn_policy_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    archived_at TEXT,
    UNIQUE(session_id, agent_path),
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(parent_agent_thread_id) REFERENCES agent_threads(id),
    FOREIGN KEY(runtime_snapshot_id) REFERENCES agent_runtime_snapshots(id)
);

CREATE TABLE agent_turns (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_thread_id TEXT NOT NULL,
    requested_by_agent_thread_id TEXT,
    requested_by_turn_id TEXT,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    task_message TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'queued', 'running', 'waiting_approval', 'waiting_input',
        'waiting_action', 'completed', 'failed', 'cancelled', 'interrupted'
    )),
    invocation_id INTEGER NOT NULL CHECK(invocation_id > 0),
    outcome_ref TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    UNIQUE(agent_thread_id, sequence),
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_thread_id) REFERENCES agent_threads(id) ON DELETE CASCADE,
    FOREIGN KEY(requested_by_agent_thread_id) REFERENCES agent_threads(id),
    FOREIGN KEY(requested_by_turn_id) REFERENCES agent_turns(id)
);

CREATE TABLE agent_ledger_items (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_thread_id TEXT NOT NULL,
    agent_turn_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    item_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(agent_thread_id, sequence),
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_thread_id) REFERENCES agent_threads(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_turn_id) REFERENCES agent_turns(id) ON DELETE CASCADE
);

CREATE TABLE agent_mailbox_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    from_agent_thread_id TEXT NOT NULL,
    to_agent_thread_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('message', 'completion', 'needs_attention')),
    payload_json TEXT NOT NULL,
    causation_id TEXT,
    created_at TEXT NOT NULL,
    acknowledged_at TEXT,
    UNIQUE(session_id, sequence),
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(from_agent_thread_id) REFERENCES agent_threads(id),
    FOREIGN KEY(to_agent_thread_id) REFERENCES agent_threads(id)
);

CREATE INDEX idx_agent_threads_parent
    ON agent_threads(session_id, parent_agent_thread_id, created_at);

CREATE INDEX idx_agent_turns_agent_sequence
    ON agent_turns(agent_thread_id, sequence DESC);

CREATE UNIQUE INDEX idx_agent_turns_one_active
    ON agent_turns(agent_thread_id)
    WHERE status IN (
        'queued', 'running', 'waiting_approval', 'waiting_input', 'waiting_action'
    );

CREATE INDEX idx_agent_mailbox_pending
    ON agent_mailbox_messages(session_id, to_agent_thread_id, sequence)
    WHERE acknowledged_at IS NULL;

CREATE UNIQUE INDEX idx_agent_mailbox_causation
    ON agent_mailbox_messages(session_id, to_agent_thread_id, kind, causation_id)
    WHERE causation_id IS NOT NULL;
