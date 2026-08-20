CREATE TABLE workflow_deployments (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    name TEXT NOT NULL,
    environment TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'disabled')),
    flow_id TEXT NOT NULL,
    flow_version INTEGER NOT NULL CHECK(flow_version > 0),
    definition_id TEXT NOT NULL,
    snapshot_hash TEXT NOT NULL,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(definition_id) REFERENCES flow_definitions(id) ON DELETE RESTRICT
);

CREATE INDEX idx_workflow_deployments_flow_updated
    ON workflow_deployments(flow_id, flow_version, updated_at DESC);

CREATE INDEX idx_workflow_deployments_status_environment
    ON workflow_deployments(status, environment, updated_at DESC);
