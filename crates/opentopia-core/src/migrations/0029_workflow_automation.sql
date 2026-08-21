-- Stable release channels keep external trigger identity separate from immutable
-- deployment snapshots, so canaries and rollback never mutate execution code.
CREATE TABLE workflow_releases (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    release_key TEXT NOT NULL UNIQUE,
    environment TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'disabled')),
    trigger_id TEXT UNIQUE,
    trigger_kind TEXT NOT NULL CHECK(trigger_kind IN (
        'manual', 'webhook', 'schedule', 'event_subscription'
    )),
    primary_deployment_id TEXT NOT NULL,
    canary_deployment_id TEXT,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
    FOREIGN KEY(primary_deployment_id) REFERENCES workflow_deployments(id) ON DELETE RESTRICT,
    FOREIGN KEY(canary_deployment_id) REFERENCES workflow_deployments(id) ON DELETE RESTRICT
);

CREATE INDEX idx_workflow_releases_status_trigger
    ON workflow_releases(status, trigger_kind, updated_at DESC);

CREATE INDEX idx_workflow_releases_environment_updated
    ON workflow_releases(environment, updated_at DESC);

-- The release/idempotency pair is the durable external-effect boundary. A
-- repeated request reuses the same selected deployment and Flow Run.
CREATE TABLE workflow_trigger_invocations (
    id TEXT PRIMARY KEY,
    release_id TEXT NOT NULL,
    trigger_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    deployment_id TEXT NOT NULL,
    flow_run_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('accepted', 'started', 'failed')),
    input_hash TEXT NOT NULL,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(release_id, idempotency_key),
    FOREIGN KEY(release_id) REFERENCES workflow_releases(id) ON DELETE CASCADE,
    FOREIGN KEY(deployment_id) REFERENCES workflow_deployments(id) ON DELETE RESTRICT,
    FOREIGN KEY(flow_run_id) REFERENCES flow_runs(id) ON DELETE SET NULL
);

CREATE INDEX idx_workflow_trigger_invocations_release_updated
    ON workflow_trigger_invocations(release_id, updated_at DESC);

CREATE INDEX idx_workflow_trigger_invocations_status_updated
    ON workflow_trigger_invocations(status, updated_at DESC);

CREATE INDEX idx_workflow_trigger_invocations_trigger_created
    ON workflow_trigger_invocations(trigger_id, created_at DESC);

-- A run has exactly one output delivery lifecycle. Attempts are updated with
-- optimistic concurrency and retain a stable idempotency key for providers.
CREATE TABLE workflow_delivery_receipts (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    run_id TEXT NOT NULL UNIQUE,
    deployment_id TEXT NOT NULL,
    output_kind TEXT NOT NULL CHECK(output_kind IN (
        'inbox', 'webhook', 'connection_operation', 'human_task'
    )),
    status TEXT NOT NULL CHECK(status IN (
        'pending', 'delivered', 'failed', 'waiting_human', 'cancelled'
    )),
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(run_id) REFERENCES flow_runs(id) ON DELETE CASCADE,
    FOREIGN KEY(deployment_id) REFERENCES workflow_deployments(id) ON DELETE RESTRICT
);

CREATE INDEX idx_workflow_delivery_receipts_status_updated
    ON workflow_delivery_receipts(status, updated_at DESC);

CREATE INDEX idx_workflow_delivery_receipts_deployment_updated
    ON workflow_delivery_receipts(deployment_id, updated_at DESC);

CREATE TABLE workflow_evaluations (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    deployment_id TEXT NOT NULL,
    evaluator TEXT NOT NULL,
    passed INTEGER NOT NULL CHECK(passed IN (0, 1)),
    score REAL NOT NULL CHECK(score >= 0.0 AND score <= 1.0),
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(run_id, evaluator),
    FOREIGN KEY(run_id) REFERENCES flow_runs(id) ON DELETE CASCADE,
    FOREIGN KEY(deployment_id) REFERENCES workflow_deployments(id) ON DELETE RESTRICT
);

CREATE INDEX idx_workflow_evaluations_deployment_created
    ON workflow_evaluations(deployment_id, created_at DESC);

-- Human Tasks are polymorphic in P5: a task can belong to a Flow execution or
-- to a DeliveryReceipt recovery/handoff. The application validates the typed
-- source; retaining a Flow-only foreign key here would make that impossible.
ALTER TABLE human_tasks RENAME TO human_tasks_v25;

CREATE TABLE human_tasks (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    thread_id TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN ('flow_run', 'delivery_receipt')),
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
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

INSERT INTO human_tasks (
    id, revision, thread_id, source_kind, source_id, source_node_run_id,
    task_type, status, document_json, created_at, updated_at, resolved_at
)
SELECT
    id, revision, thread_id, source_kind, source_id, source_node_run_id,
    task_type, status, document_json, created_at, updated_at, resolved_at
FROM human_tasks_v25;

DROP TABLE human_tasks_v25;

CREATE UNIQUE INDEX idx_human_tasks_active_source_boundary
    ON human_tasks(source_kind, source_id, source_node_run_id, task_type)
    WHERE status = 'pending';

CREATE INDEX idx_human_tasks_status_updated
    ON human_tasks(status, updated_at DESC);

CREATE INDEX idx_human_tasks_thread_status_updated
    ON human_tasks(thread_id, status, updated_at DESC);

CREATE INDEX idx_human_tasks_flow_run_status
    ON human_tasks(source_id, status, updated_at DESC)
    WHERE source_kind = 'flow_run';
