ALTER TABLE agent_mailbox_messages
    ADD COLUMN delivery_state TEXT NOT NULL DEFAULT 'pending'
        CHECK(delivery_state IN ('pending', 'delivered', 'acknowledged'));

ALTER TABLE agent_mailbox_messages
    ADD COLUMN delivered_at TEXT;

CREATE TABLE agent_events (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL CHECK(event_seq > 0),
    agent_thread_id TEXT NOT NULL,
    agent_turn_id TEXT NOT NULL,
    invocation_id INTEGER NOT NULL CHECK(invocation_id > 0),
    event_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    causation_id TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(session_id, event_seq),
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_thread_id) REFERENCES agent_threads(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_turn_id) REFERENCES agent_turns(id) ON DELETE CASCADE
);

CREATE INDEX idx_agent_events_thread_turn_seq
    ON agent_events(agent_thread_id, agent_turn_id, event_seq);

CREATE TABLE agent_provider_states (
    agent_thread_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    response_id TEXT NOT NULL,
    compatibility_hash TEXT NOT NULL,
    state_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(agent_thread_id, provider_id),
    FOREIGN KEY(agent_thread_id) REFERENCES agent_threads(id) ON DELETE CASCADE
);

DROP TABLE subagent_conversations;
DROP TABLE subagent_runs;
