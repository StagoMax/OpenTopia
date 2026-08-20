-- Effect receipts belong to a durable execution scope. Before Workflow Agent
-- continuations existed every scope was a conversation Turn, so v19 encoded a
-- foreign key to turns. Flow runs use their own stable id and must not create a
-- synthetic active Turn that would interfere with conversation scheduling.
CREATE TABLE effect_journal_v28 (
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
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

INSERT INTO effect_journal_v28 (
    effect_id, thread_id, turn_id, agent_path, idempotency_key, kind,
    operation, input_hash, input_json, result_json, status,
    side_effect_class, idempotent, attempt, error, created_at, started_at,
    completed_at, updated_at
)
SELECT
    effect_id, thread_id, turn_id, agent_path, idempotency_key, kind,
    operation, input_hash, input_json, result_json, status,
    side_effect_class, idempotent, attempt, error, created_at, started_at,
    completed_at, updated_at
FROM effect_journal;

DROP TABLE effect_journal;
ALTER TABLE effect_journal_v28 RENAME TO effect_journal;

CREATE INDEX idx_effect_journal_turn
    ON effect_journal(turn_id, created_at);

CREATE INDEX idx_effect_journal_recovery
    ON effect_journal(status, updated_at);
