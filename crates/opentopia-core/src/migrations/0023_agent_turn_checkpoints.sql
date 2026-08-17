CREATE TABLE agent_turn_checkpoints (
    agent_turn_id TEXT PRIMARY KEY,
    wait_kind TEXT NOT NULL CHECK(wait_kind IN ('approval', 'user_input', 'external_action')),
    continuation_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(agent_turn_id) REFERENCES agent_turns(id) ON DELETE CASCADE
);
