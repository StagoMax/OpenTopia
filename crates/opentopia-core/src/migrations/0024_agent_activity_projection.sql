CREATE TABLE agent_activity_state (
    session_id TEXT NOT NULL,
    agent_thread_id TEXT NOT NULL,
    agent_turn_id TEXT NOT NULL,
    cursor INTEGER NOT NULL DEFAULT 0 CHECK(cursor >= 0),
    model_round INTEGER,
    round_boundary INTEGER,
    reasoning_tail TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL,
    PRIMARY KEY(agent_thread_id, agent_turn_id),
    FOREIGN KEY(session_id) REFERENCES agent_sessions(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_thread_id) REFERENCES agent_threads(id) ON DELETE CASCADE,
    FOREIGN KEY(agent_turn_id) REFERENCES agent_turns(id) ON DELETE CASCADE
);

CREATE INDEX idx_agent_activity_state_session_cursor
    ON agent_activity_state(session_id, cursor);

CREATE INDEX idx_agent_events_activity_visible
    ON agent_events(agent_thread_id, agent_turn_id, event_seq)
    WHERE event_kind IN (
        'model_context_built',
        'model_request',
        'tool_call_started',
        'tool_call_finished',
        'turn_suspended',
        'turn_awaiting_input',
        'error',
        'turn_started',
        'turn_finished',
        'turn_cancelled'
    );

CREATE INDEX idx_agent_events_reasoning_tail
    ON agent_events(agent_thread_id, agent_turn_id, event_seq)
    WHERE event_kind = 'reasoning_delta';

CREATE INDEX idx_agent_events_tool_results
    ON agent_events(agent_thread_id, agent_turn_id, event_seq)
    WHERE event_kind = 'tool_call_finished';

CREATE INDEX idx_agent_events_model_round
    ON agent_events(agent_thread_id, agent_turn_id, event_seq)
    WHERE event_kind IN (
        'model_context_built',
        'model_request',
        'provider_request_sent',
        'provider_request_retried',
        'provider_response_received'
    );
