-- Ordinary plugins follow the Codex configuration model: enablement belongs
-- to the user/project configuration, never to an individual task. Capability
-- declarations are loaded from the active plugin manifest rather than copied
-- into a second mutable catalog.

DROP INDEX IF EXISTS idx_plugin_activations_scope;

CREATE TABLE plugin_activations_v32 (
    plugin_id TEXT NOT NULL,
    scope_type TEXT NOT NULL CHECK(scope_type IN ('global', 'workspace')),
    scope_id TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(plugin_id, scope_type, scope_id)
);

INSERT INTO plugin_activations_v32 (
    plugin_id, scope_type, scope_id, enabled, updated_at
)
SELECT plugin_id, scope_type, scope_id, enabled, updated_at
FROM plugin_activations
WHERE scope_type IN ('global', 'workspace');

DROP TABLE plugin_activations;
ALTER TABLE plugin_activations_v32 RENAME TO plugin_activations;

CREATE INDEX idx_plugin_activations_scope
    ON plugin_activations(scope_type, scope_id, plugin_id);

DROP INDEX IF EXISTS idx_thread_plugin_activations_thread;
DROP TABLE IF EXISTS thread_plugin_activations;

DROP INDEX IF EXISTS idx_plugin_contributions_plugin;
DROP TABLE IF EXISTS plugin_contributions;

-- Health is observational and contribution IDs change with the stable plugin
-- identity cutover. Runtimes repopulate it after they start.
DELETE FROM plugin_runtime_health;
