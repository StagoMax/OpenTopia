use super::sqlite_codec::collect_rows;
use super::sqlite_rows::{map_event, map_mcp_server, map_mcp_server_tool, map_thread_mcp_server};
use super::SqliteSessionStore;
use crate::mcp::{McpServerConfig, McpToolDescriptor, ThreadMcpServer};
use crate::model::AgentEvent;
use crate::settings::AppSettings;
use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use std::collections::HashMap;
use uuid::Uuid;

impl SqliteSessionStore {
    pub fn load_settings(
        &self,
        default_permission_mode: crate::policy::PermissionMode,
    ) -> anyhow::Result<AppSettings> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let settings_json: Option<String> = conn
            .query_row(
                "SELECT settings_json FROM app_settings WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match settings_json {
            Some(settings_json) => {
                let mut settings: AppSettings = serde_json::from_str(&settings_json)?;
                // Enterprise availability is a deployment boundary, not a
                // persisted preference that a client can turn on.
                settings.enterprise = crate::settings::EnterpriseSettings::from_env();
                if settings.providers.is_empty() {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&settings_json) {
                        if let Some(provider) = value.get("provider") {
                            if let Ok(p) = serde_json::from_value(provider.clone()) {
                                settings.providers = vec![p];
                            }
                        }
                    }
                    if settings.active_provider_id.is_empty() {
                        settings.active_provider_id = settings
                            .providers
                            .first()
                            .map(|p| p.id.clone())
                            .unwrap_or_default();
                    }
                }
                let migrated_parallel_tool_calls = !settings.parallel_tool_calls_migrated;
                let migrated_provider_axes = settings.providers.iter().any(|provider| {
                    provider.transport.is_none()
                        || provider.auth.is_none()
                        || provider.allowed_adapters.is_empty()
                });
                if migrated_parallel_tool_calls {
                    for provider in &mut settings.providers {
                        provider.parallel_tool_calls = true;
                    }
                    settings.parallel_tool_calls_migrated = true;
                }
                settings.touch();
                if migrated_parallel_tool_calls || migrated_provider_axes {
                    conn.execute(
                        "UPDATE app_settings SET settings_json = ?1, updated_at = ?2 WHERE id = 1",
                        params![
                            serde_json::to_string(&settings)?,
                            settings.updated_at.to_rfc3339()
                        ],
                    )?;
                }
                Ok(settings)
            }
            None => Ok(AppSettings::from_env(default_permission_mode)),
        }
    }

    pub fn save_settings(&self, mut settings: AppSettings) -> anyhow::Result<AppSettings> {
        settings.touch();
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO app_settings (id, settings_json, updated_at)
            VALUES (1, ?1, ?2)
            ON CONFLICT(id) DO UPDATE SET
                settings_json = excluded.settings_json,
                updated_at = excluded.updated_at
            "#,
            params![
                serde_json::to_string(&settings)?,
                settings.updated_at.to_rfc3339()
            ],
        )?;
        Ok(settings)
    }

    pub fn list_mcp_servers(&self) -> anyhow::Result<Vec<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT server_id, name, command, args_json, cwd, env_keys_json,
                   timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
            FROM mcp_servers
            ORDER BY name ASC
            "#,
        )?;
        let rows = stmt.query_map([], map_mcp_server)?;
        collect_rows(rows)
    }

    pub fn get_mcp_server(&self, server_id: Uuid) -> anyhow::Result<Option<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.query_row(
            r#"
            SELECT server_id, name, command, args_json, cwd, env_keys_json,
                   timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
            FROM mcp_servers
            WHERE server_id = ?1
            "#,
            params![server_id.to_string()],
            map_mcp_server,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn insert_mcp_server(&self, config: McpServerConfig) -> anyhow::Result<McpServerConfig> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO mcp_servers (
                server_id, name, command, args_json, cwd, env_keys_json,
                timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
            params![
                config.server_id.to_string(),
                &config.name,
                &config.command,
                serde_json::to_string(&config.args)?,
                config.cwd.as_ref().map(|path| path.display().to_string()),
                serde_json::to_string(&config.env_keys)?,
                config.timeout_ms as i64,
                config.enabled as i64,
                &config.plugin_id,
                &config.plugin_server_name,
                config.created_at.to_rfc3339(),
                config.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(config)
    }

    pub fn update_mcp_server(
        &self,
        config: McpServerConfig,
    ) -> anyhow::Result<Option<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let updated = conn.execute(
            r#"
            UPDATE mcp_servers
            SET name = ?1,
                command = ?2,
                args_json = ?3,
                cwd = ?4,
                env_keys_json = ?5,
                timeout_ms = ?6,
                enabled = ?7,
                plugin_id = ?8,
                plugin_server_name = ?9,
                updated_at = ?10
            WHERE server_id = ?11
            "#,
            params![
                &config.name,
                &config.command,
                serde_json::to_string(&config.args)?,
                config.cwd.as_ref().map(|path| path.display().to_string()),
                serde_json::to_string(&config.env_keys)?,
                config.timeout_ms as i64,
                config.enabled as i64,
                &config.plugin_id,
                &config.plugin_server_name,
                config.updated_at.to_rfc3339(),
                config.server_id.to_string(),
            ],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        Ok(Some(config))
    }

    pub fn delete_mcp_server(&self, server_id: Uuid) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let deleted = conn.execute(
            "DELETE FROM mcp_servers WHERE server_id = ?1",
            params![server_id.to_string()],
        )?;
        Ok(deleted > 0)
    }

    pub fn list_plugin_mcp_servers(&self, plugin_id: &str) -> anyhow::Result<Vec<McpServerConfig>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT server_id, name, command, args_json, cwd, env_keys_json,
                   timeout_ms, enabled, plugin_id, plugin_server_name, created_at, updated_at
            FROM mcp_servers
            WHERE plugin_id = ?1
            ORDER BY name ASC
            "#,
        )?;
        let rows = stmt.query_map(params![plugin_id], map_mcp_server)?;
        collect_rows(rows)
    }

    pub fn list_thread_mcp_servers(&self, thread_id: Uuid) -> anyhow::Result<Vec<ThreadMcpServer>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT thread_id, server_id, enabled, updated_at
            FROM thread_mcp_servers
            WHERE thread_id = ?1
            ORDER BY updated_at DESC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_thread_mcp_server)?;
        collect_rows(rows)
    }

    pub fn set_thread_mcp_server(
        &self,
        thread_id: Uuid,
        server_id: Uuid,
        enabled: bool,
    ) -> anyhow::Result<ThreadMcpServer> {
        let binding = ThreadMcpServer {
            thread_id,
            server_id,
            enabled,
            updated_at: Utc::now(),
        };
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO thread_mcp_servers (thread_id, server_id, enabled, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(thread_id, server_id) DO UPDATE SET
                enabled = excluded.enabled,
                updated_at = excluded.updated_at
            "#,
            params![
                binding.thread_id.to_string(),
                binding.server_id.to_string(),
                binding.enabled as i64,
                binding.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(binding)
    }

    pub fn list_thread_plugin_activations(
        &self,
        thread_id: Uuid,
    ) -> anyhow::Result<HashMap<String, bool>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT plugin_name, enabled
            FROM thread_plugin_activations
            WHERE thread_id = ?1
            ORDER BY plugin_name ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0))
        })?;
        let mut activations = HashMap::new();
        for row in rows {
            let (plugin_name, enabled) = row?;
            activations.insert(plugin_name, enabled);
        }
        Ok(activations)
    }

    pub fn set_thread_plugin_activation(
        &self,
        thread_id: Uuid,
        plugin_name: &str,
        enabled: bool,
    ) -> anyhow::Result<()> {
        let plugin_name = plugin_name.trim();
        anyhow::ensure!(!plugin_name.is_empty(), "plugin name cannot be empty");
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO thread_plugin_activations (thread_id, plugin_name, enabled, updated_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(thread_id, plugin_name) DO UPDATE SET
                enabled = excluded.enabled,
                updated_at = excluded.updated_at
            "#,
            params![
                thread_id.to_string(),
                plugin_name,
                enabled as i64,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Replaces the persisted tool catalog for one MCP server.
    ///
    /// The cache is a mirror of the server's last successful `tools/list`, so the whole
    /// catalog is rewritten in a single transaction rather than merged. Tools the server
    /// stopped advertising must not survive the rewrite.
    pub fn replace_mcp_server_tools(
        &self,
        server_id: Uuid,
        tools: &[McpToolDescriptor],
    ) -> anyhow::Result<()> {
        let updated_at = Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM mcp_server_tools WHERE server_id = ?1",
            params![server_id.to_string()],
        )?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO mcp_server_tools (
                    server_id, public_name, tool_name, description,
                    input_schema_json, annotations_json, meta_json,
                    permission_labels_json, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
            )?;
            for tool in tools {
                stmt.execute(params![
                    server_id.to_string(),
                    tool.public_name,
                    tool.tool_name,
                    tool.description,
                    serde_json::to_string(&tool.input_schema)?,
                    serde_json::to_string(&tool.annotations)?,
                    serde_json::to_string(&tool.meta)?,
                    serde_json::to_string(&tool.permission_labels)?,
                    updated_at,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_mcp_server_tools(&self, server_id: Uuid) -> anyhow::Result<Vec<McpToolDescriptor>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT server_id, public_name, tool_name, description,
                   input_schema_json, annotations_json, meta_json, permission_labels_json
            FROM mcp_server_tools
            WHERE server_id = ?1
            ORDER BY public_name ASC
            "#,
        )?;
        let rows = stmt.query_map(params![server_id.to_string()], map_mcp_server_tool)?;
        collect_rows(rows)
    }

    pub fn list_all_mcp_server_tools(&self) -> anyhow::Result<Vec<McpToolDescriptor>> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let mut stmt = conn.prepare(
            r#"
            SELECT server_id, public_name, tool_name, description,
                   input_schema_json, annotations_json, meta_json, permission_labels_json
            FROM mcp_server_tools
            ORDER BY public_name ASC
            "#,
        )?;
        let rows = stmt.query_map([], map_mcp_server_tool)?;
        collect_rows(rows)
    }

    /// Returns the event history needed by the conversation UI without the
    /// duplicated model/provider payloads retained for diagnostics and export.
    pub fn list_conversation_events(
        &self,
        thread_id: Uuid,
        after_seq: Option<i64>,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT id, thread_id, turn_id, seq, payload_json, created_at
            FROM conversation_events
            WHERE thread_id = ?1
              AND seq > ?2
            ORDER BY seq ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string(), after_seq.unwrap_or(0)],
            map_event,
        )?;
        collect_rows(rows)
    }

    /// Loads projected tool results for one turn without deserializing the
    /// complete (and potentially very large) conversation history.
    pub fn list_turn_tool_result_events(
        &self,
        thread_id: Uuid,
        turn_id: Uuid,
    ) -> anyhow::Result<Vec<AgentEvent>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT conversation_events.id, conversation_events.thread_id,
                   conversation_events.turn_id, conversation_events.seq,
                   conversation_events.payload_json, conversation_events.created_at
            FROM conversation_events
            INNER JOIN events ON events.id = conversation_events.id
            WHERE conversation_events.thread_id = ?1
              AND conversation_events.turn_id = ?2
              AND events.kind = 'tool_call_finished'
            ORDER BY conversation_events.seq ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![thread_id.to_string(), turn_id.to_string()],
            map_event,
        )?;
        collect_rows(rows)
    }

    /// Loads only event kinds used to calculate the context status panel.
    pub fn list_context_events(&self, thread_id: Uuid) -> anyhow::Result<Vec<AgentEvent>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT events.id, events.thread_id, events.turn_id, events.seq,
                   CASE events.kind
                       WHEN 'model_context_built'
                           THEN COALESCE(
                               conversation_events.payload_json,
                               json_remove(events.payload_json, '$.items')
                           )
                       ELSE events.payload_json
                   END,
                   events.created_at
            FROM events
            LEFT JOIN conversation_events ON conversation_events.id = events.id
            WHERE events.thread_id = ?1
              AND events.kind IN (
                  'model_context_built',
                  'provider_response_received',
                  'context_compacted',
                  'provider_context_state_invalidated',
                  'context_warning',
                  'token_usage'
              )
            ORDER BY events.seq ASC
            "#,
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], map_event)?;
        collect_rows(rows)
    }

    pub fn count_events_after(&self, thread_id: Uuid, after_seq: i64) -> anyhow::Result<usize> {
        let conn = self.read_connection();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM events WHERE thread_id = ?1 AND seq > ?2",
            params![thread_id.to_string(), after_seq],
            |row| row.get(0),
        )?;
        usize::try_from(count).context("event count exceeds usize range")
    }
}
