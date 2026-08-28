-- Flow is the only user-facing automation aggregate. Runtime revisions are
-- embedded in the active Flow and frozen again in every Flow Run.

DELETE FROM human_tasks
WHERE source_kind IN ('flow_run', 'delivery_receipt');

DROP TABLE workflow_evaluations;
DROP TABLE workflow_delivery_receipts;
DROP TABLE workflow_trigger_invocations;
DROP TABLE workflow_releases;
DROP TABLE workflow_deployments;

-- Runs from the removed Deployment/Release model contain the old serialized
-- contract. They are deliberately removed instead of retaining a compatibility
-- decoder; business demo cases are re-seeded through the new Flow API.
DELETE FROM flow_runs;

CREATE TABLE flows (
    flow_id TEXT PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    name TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'paused')),
    active_revision_id TEXT NOT NULL,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE INDEX idx_flows_status_updated
    ON flows(status, updated_at DESC);

CREATE INDEX idx_flows_thread_updated
    ON flows(thread_id, updated_at DESC);

CREATE TABLE flow_cases (
    id TEXT PRIMARY KEY,
    flow_id TEXT NOT NULL,
    trigger_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    flow_revision_id TEXT NOT NULL,
    flow_run_id TEXT,
    status TEXT NOT NULL CHECK(status IN (
        'accepted', 'started', 'failed', 'superseded'
    )),
    input_hash TEXT NOT NULL,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(flow_id, idempotency_key),
    FOREIGN KEY(flow_id) REFERENCES flows(flow_id) ON DELETE CASCADE,
    FOREIGN KEY(flow_run_id) REFERENCES flow_runs(id) ON DELETE SET NULL
);

CREATE INDEX idx_flow_cases_flow_updated
    ON flow_cases(flow_id, updated_at DESC);

CREATE INDEX idx_flow_cases_status_updated
    ON flow_cases(status, updated_at DESC);

CREATE INDEX idx_flow_cases_trigger_created
    ON flow_cases(trigger_id, created_at DESC);

CREATE TABLE flow_delivery_receipts (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    run_id TEXT NOT NULL UNIQUE,
    flow_revision_id TEXT NOT NULL,
    output_kind TEXT NOT NULL CHECK(output_kind IN (
        'inbox', 'webhook', 'connection_operation', 'human_task'
    )),
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'delivered', 'failed', 'waiting_human', 'cancelled'
    )),
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES flow_runs(id) ON DELETE CASCADE
);

CREATE INDEX idx_flow_delivery_receipts_status_updated
    ON flow_delivery_receipts(status, updated_at DESC);

CREATE INDEX idx_flow_delivery_receipts_revision_updated
    ON flow_delivery_receipts(flow_revision_id, updated_at DESC);

CREATE TABLE flow_evaluations (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    flow_revision_id TEXT NOT NULL,
    evaluator TEXT NOT NULL,
    passed INTEGER NOT NULL CHECK(passed IN (0, 1)),
    score REAL NOT NULL CHECK(score >= 0.0 AND score <= 1.0),
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(run_id, evaluator),
    FOREIGN KEY(run_id) REFERENCES flow_runs(id) ON DELETE CASCADE
);

CREATE INDEX idx_flow_evaluations_revision_created
    ON flow_evaluations(flow_revision_id, created_at DESC);
