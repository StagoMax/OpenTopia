-- Pending events are immutable once accepted. Migration to a newer Flow
-- architecture marks an old event as superseded instead of rebinding its
-- frozen Deployment or deleting audit history.
ALTER TABLE workflow_trigger_invocations
    RENAME TO workflow_trigger_invocations_v29;

CREATE TABLE workflow_trigger_invocations (
    id TEXT PRIMARY KEY,
    release_id TEXT NOT NULL,
    trigger_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    deployment_id TEXT NOT NULL,
    flow_run_id TEXT,
    status TEXT NOT NULL CHECK(status IN (
        'accepted', 'started', 'failed', 'superseded'
    )),
    input_hash TEXT NOT NULL,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(release_id, idempotency_key),
    FOREIGN KEY(release_id) REFERENCES workflow_releases(id) ON DELETE CASCADE,
    FOREIGN KEY(deployment_id) REFERENCES workflow_deployments(id) ON DELETE RESTRICT,
    FOREIGN KEY(flow_run_id) REFERENCES flow_runs(id) ON DELETE SET NULL
);

INSERT INTO workflow_trigger_invocations (
    id, release_id, trigger_id, idempotency_key, deployment_id, flow_run_id,
    status, input_hash, document_json, created_at, updated_at
)
SELECT
    id, release_id, trigger_id, idempotency_key, deployment_id, flow_run_id,
    status, input_hash, document_json, created_at, updated_at
FROM workflow_trigger_invocations_v29;

DROP TABLE workflow_trigger_invocations_v29;

CREATE INDEX idx_workflow_trigger_invocations_release_updated
    ON workflow_trigger_invocations(release_id, updated_at DESC);

CREATE INDEX idx_workflow_trigger_invocations_status_updated
    ON workflow_trigger_invocations(status, updated_at DESC);

CREATE INDEX idx_workflow_trigger_invocations_trigger_created
    ON workflow_trigger_invocations(trigger_id, created_at DESC);
