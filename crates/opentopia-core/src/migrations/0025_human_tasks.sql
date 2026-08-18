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

CREATE UNIQUE INDEX idx_human_tasks_active_source_boundary
    ON human_tasks(source_kind, source_id, source_node_run_id, task_type)
    WHERE status = 'pending';

CREATE INDEX idx_human_tasks_status_updated
    ON human_tasks(status, updated_at DESC);

CREATE INDEX idx_human_tasks_thread_status_updated
    ON human_tasks(thread_id, status, updated_at DESC);

CREATE INDEX idx_human_tasks_flow_run_status
    ON human_tasks(source_id, status, updated_at DESC);
