CREATE TABLE integration_definitions (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    key TEXT NOT NULL COLLATE NOCASE UNIQUE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('mcp', 'oauth_api', 'database', 'local_app')),
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE connections (
    id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK(revision > 0),
    integration_definition_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN (
        'configured',
        'ready',
        'degraded',
        'reauth_required',
        'disabled'
    )),
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
    mcp_server_id TEXT,
    active_capability_revision INTEGER,
    document_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY(integration_definition_id) REFERENCES integration_definitions(id) ON DELETE RESTRICT,
    FOREIGN KEY(mcp_server_id) REFERENCES mcp_servers(server_id) ON DELETE CASCADE
);

-- A runtime belongs to one account-level Connection. Sharing a stdio process
-- between accounts would silently collapse their credential and tenant boundaries.
CREATE UNIQUE INDEX idx_connections_mcp_server
    ON connections(mcp_server_id)
    WHERE mcp_server_id IS NOT NULL;

CREATE INDEX idx_connections_integration_updated
    ON connections(integration_definition_id, updated_at DESC);

CREATE INDEX idx_connections_status_updated
    ON connections(status, updated_at DESC);

CREATE TABLE connection_capability_revisions (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    source TEXT NOT NULL CHECK(source IN ('mcp_tools_list', 'static')),
    content_hash TEXT NOT NULL,
    document_json TEXT NOT NULL,
    discovered_at TEXT NOT NULL,
    FOREIGN KEY(connection_id) REFERENCES connections(id) ON DELETE CASCADE,
    UNIQUE(connection_id, revision)
);

CREATE INDEX idx_connection_capability_revisions_discovered
    ON connection_capability_revisions(connection_id, discovered_at DESC);

-- Preserve all legacy MCP configurations while introducing the new account-level
-- control plane. A migrated server receives one catalog definition and one
-- Connection; future account Connections must bind their own MCP runtime.
INSERT INTO integration_definitions (
    id, revision, key, name, kind, enabled, document_json, created_at, updated_at
)
SELECT
    server_id,
    1,
    'legacy-mcp-' || server_id,
    name,
    'mcp',
    enabled,
    json_object(
        'schemaVersion', 1,
        'id', server_id,
        'revision', 1,
        'key', 'legacy-mcp-' || server_id,
        'name', name,
        'description', 'Migrated from the legacy MCP server catalog.',
        'kind', 'mcp',
        'authScheme', CASE WHEN json_array_length(env_keys_json) > 0 THEN 'external' ELSE 'none' END,
        'capabilityDiscovery', 'mcp_tools_list',
        'enabled', json(CASE WHEN enabled = 1 THEN 'true' ELSE 'false' END),
        'createdAt', created_at,
        'updatedAt', updated_at
    ),
    created_at,
    updated_at
FROM mcp_servers;

INSERT INTO connections (
    id, revision, integration_definition_id, status, enabled, mcp_server_id,
    active_capability_revision, document_json, created_at, updated_at
)
SELECT
    server_id,
    1,
    server_id,
    CASE WHEN enabled = 1 THEN 'configured' ELSE 'disabled' END,
    enabled,
    server_id,
    NULL,
    json_object(
        'schemaVersion', 1,
        'id', server_id,
        'revision', 1,
        'integrationDefinitionId', server_id,
        'name', name,
        'ownerType', 'personal',
        'environment', 'local',
        'enabled', json(CASE WHEN enabled = 1 THEN 'true' ELSE 'false' END),
        'status', CASE WHEN enabled = 1 THEN 'configured' ELSE 'disabled' END,
        'runtimeBinding', json_object('kind', 'mcp_server', 'serverId', server_id),
        'authContext', json_object(
            'verification', CASE
                WHEN json_array_length(env_keys_json) > 0 THEN 'legacy_unverified'
                ELSE 'not_required'
            END,
            'account', json_object('displayName', name),
            'grantedScopes', json_array()
        ),
        'createdAt', created_at,
        'updatedAt', updated_at
    ),
    created_at,
    updated_at
FROM mcp_servers;
