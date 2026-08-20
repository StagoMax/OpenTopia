use super::sqlite_codec::{collect_rows, deserialize_json_column};
use super::{ConnectionStoreError, SqliteSessionStore};
use crate::connection::{
    ConnectionCapabilityRevisionV1, ConnectionStatusV1, ConnectionV1, IntegrationDefinitionV1,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

impl SqliteSessionStore {
    pub fn insert_integration_definition(
        &self,
        definition: &IntegrationDefinitionV1,
    ) -> anyhow::Result<IntegrationDefinitionV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM integration_definitions WHERE key = ?1 COLLATE NOCASE",
                params![definition.key],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_some() {
            return Err(
                ConnectionStoreError::DuplicateIntegrationKey(definition.key.clone()).into(),
            );
        }
        conn.execute(
            r#"
            INSERT INTO integration_definitions (
                id, revision, key, name, kind, enabled, document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                definition.id.to_string(),
                i64::from(definition.revision),
                definition.key,
                definition.name,
                definition.kind.as_str(),
                definition.enabled as i64,
                serde_json::to_string(definition)?,
                definition.created_at.to_rfc3339(),
                definition.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(definition.clone())
    }

    pub fn get_integration_definition(
        &self,
        definition_id: Uuid,
    ) -> anyhow::Result<Option<IntegrationDefinitionV1>> {
        let conn = self.read_connection();
        conn.query_row(
            "SELECT document_json FROM integration_definitions WHERE id = ?1",
            params![definition_id.to_string()],
            deserialize_json_column::<IntegrationDefinitionV1>,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_integration_definitions(&self) -> anyhow::Result<Vec<IntegrationDefinitionV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT document_json
            FROM integration_definitions
            ORDER BY name COLLATE NOCASE ASC, id ASC
            "#,
        )?;
        let rows = stmt.query_map([], deserialize_json_column::<IntegrationDefinitionV1>)?;
        collect_rows(rows)
    }

    pub fn update_integration_definition(
        &self,
        definition: &IntegrationDefinitionV1,
        expected_revision: u32,
    ) -> anyhow::Result<IntegrationDefinitionV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let conflicting_key: Option<String> = conn
            .query_row(
                r#"
                SELECT id FROM integration_definitions
                WHERE key = ?1 COLLATE NOCASE AND id <> ?2
                "#,
                params![definition.key, definition.id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if conflicting_key.is_some() {
            return Err(
                ConnectionStoreError::DuplicateIntegrationKey(definition.key.clone()).into(),
            );
        }
        let changed = conn.execute(
            r#"
            UPDATE integration_definitions
            SET revision = ?2, key = ?3, name = ?4, kind = ?5, enabled = ?6,
                document_json = ?7, updated_at = ?8
            WHERE id = ?1 AND revision = ?9
            "#,
            params![
                definition.id.to_string(),
                i64::from(definition.revision),
                definition.key,
                definition.name,
                definition.kind.as_str(),
                definition.enabled as i64,
                serde_json::to_string(definition)?,
                definition.updated_at.to_rfc3339(),
                i64::from(expected_revision),
            ],
        )?;
        if changed == 1 {
            return Ok(definition.clone());
        }
        Err(integration_update_error(&conn, definition.id)?)
    }

    pub fn delete_integration_definition(&self, definition_id: Uuid) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let deleted = conn.execute(
            "DELETE FROM integration_definitions WHERE id = ?1",
            params![definition_id.to_string()],
        )?;
        Ok(deleted == 1)
    }

    pub fn insert_connection(&self, connection: &ConnectionV1) -> anyhow::Result<ConnectionV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        ensure_runtime_unbound(&conn, connection.runtime_binding.mcp_server_id(), None)?;
        conn.execute(
            r#"
            INSERT INTO connections (
                id, revision, integration_definition_id, status, enabled, mcp_server_id,
                active_capability_revision, document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                connection.id.to_string(),
                i64::from(connection.revision),
                connection.integration_definition_id.to_string(),
                connection.status.as_str(),
                connection.enabled as i64,
                connection.runtime_binding.mcp_server_id().to_string(),
                connection.active_capability_revision.map(i64::from),
                serde_json::to_string(connection)?,
                connection.created_at.to_rfc3339(),
                connection.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(connection.clone())
    }

    pub fn get_connection(&self, connection_id: Uuid) -> anyhow::Result<Option<ConnectionV1>> {
        let conn = self.read_connection();
        conn.query_row(
            "SELECT document_json FROM connections WHERE id = ?1",
            params![connection_id.to_string()],
            deserialize_json_column::<ConnectionV1>,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn list_connections(
        &self,
        integration_definition_id: Option<Uuid>,
        status: Option<ConnectionStatusV1>,
    ) -> anyhow::Result<Vec<ConnectionV1>> {
        let conn = self.read_connection();
        let integration_definition_id = integration_definition_id.map(|id| id.to_string());
        let status = status.map(|status| status.as_str().to_string());
        let mut stmt = conn.prepare(
            r#"
            SELECT document_json
            FROM connections
            WHERE (?1 IS NULL OR integration_definition_id = ?1)
              AND (?2 IS NULL OR status = ?2)
            ORDER BY updated_at DESC, id ASC
            "#,
        )?;
        let rows = stmt.query_map(
            params![integration_definition_id, status],
            deserialize_json_column::<ConnectionV1>,
        )?;
        collect_rows(rows)
    }

    pub fn update_connection(
        &self,
        connection: &ConnectionV1,
        expected_revision: u32,
    ) -> anyhow::Result<ConnectionV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        ensure_runtime_unbound(
            &conn,
            connection.runtime_binding.mcp_server_id(),
            Some(connection.id),
        )?;
        update_connection_conn(&conn, connection, expected_revision)?;
        Ok(connection.clone())
    }

    pub fn list_connection_capability_revisions(
        &self,
        connection_id: Uuid,
    ) -> anyhow::Result<Vec<ConnectionCapabilityRevisionV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT document_json
            FROM connection_capability_revisions
            WHERE connection_id = ?1
            ORDER BY revision DESC
            "#,
        )?;
        let rows = stmt.query_map(
            params![connection_id.to_string()],
            deserialize_json_column::<ConnectionCapabilityRevisionV1>,
        )?;
        collect_rows(rows)
    }

    pub fn get_connection_capability_revision(
        &self,
        connection_id: Uuid,
        revision: u32,
    ) -> anyhow::Result<Option<ConnectionCapabilityRevisionV1>> {
        let conn = self.read_connection();
        conn.query_row(
            r#"
            SELECT document_json
            FROM connection_capability_revisions
            WHERE connection_id = ?1 AND revision = ?2
            "#,
            params![connection_id.to_string(), i64::from(revision)],
            deserialize_json_column::<ConnectionCapabilityRevisionV1>,
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn publish_connection_capability_revision(
        &self,
        connection: &ConnectionV1,
        expected_connection_revision: u32,
        capability_revision: &ConnectionCapabilityRevisionV1,
    ) -> anyhow::Result<(ConnectionV1, ConnectionCapabilityRevisionV1)> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        update_connection_conn(&tx, connection, expected_connection_revision)?;
        tx.execute(
            r#"
            INSERT INTO connection_capability_revisions (
                id, connection_id, revision, source, content_hash, document_json, discovered_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![
                capability_revision.id.to_string(),
                capability_revision.connection_id.to_string(),
                i64::from(capability_revision.revision),
                capability_revision.source.as_str(),
                capability_revision.content_hash,
                serde_json::to_string(capability_revision)?,
                capability_revision.discovered_at.to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok((connection.clone(), capability_revision.clone()))
    }
}

fn ensure_runtime_unbound(
    conn: &rusqlite::Connection,
    server_id: Uuid,
    allowed_connection_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM connections WHERE mcp_server_id = ?1",
            params![server_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if existing.as_deref() == allowed_connection_id.map(|id| id.to_string()).as_deref() {
        return Ok(());
    }
    if existing.is_some() {
        return Err(ConnectionStoreError::McpRuntimeAlreadyBound(server_id).into());
    }
    Ok(())
}

fn update_connection_conn(
    conn: &rusqlite::Connection,
    connection: &ConnectionV1,
    expected_revision: u32,
) -> anyhow::Result<()> {
    let changed = conn.execute(
        r#"
        UPDATE connections
        SET revision = ?2, integration_definition_id = ?3, status = ?4, enabled = ?5,
            mcp_server_id = ?6, active_capability_revision = ?7,
            document_json = ?8, updated_at = ?9
        WHERE id = ?1 AND revision = ?10
        "#,
        params![
            connection.id.to_string(),
            i64::from(connection.revision),
            connection.integration_definition_id.to_string(),
            connection.status.as_str(),
            connection.enabled as i64,
            connection.runtime_binding.mcp_server_id().to_string(),
            connection.active_capability_revision.map(i64::from),
            serde_json::to_string(connection)?,
            connection.updated_at.to_rfc3339(),
            i64::from(expected_revision),
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    Err(connection_update_error(conn, connection.id)?)
}

fn integration_update_error(
    conn: &rusqlite::Connection,
    definition_id: Uuid,
) -> anyhow::Result<anyhow::Error> {
    let current: Option<i64> = conn
        .query_row(
            "SELECT revision FROM integration_definitions WHERE id = ?1",
            params![definition_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match current {
        Some(revision) => ConnectionStoreError::IntegrationDefinitionRevisionConflict(
            u32::try_from(revision).unwrap_or(u32::MAX),
        )
        .into(),
        None => ConnectionStoreError::IntegrationDefinitionNotFound(definition_id).into(),
    })
}

fn connection_update_error(
    conn: &rusqlite::Connection,
    connection_id: Uuid,
) -> anyhow::Result<anyhow::Error> {
    let current: Option<i64> = conn
        .query_row(
            "SELECT revision FROM connections WHERE id = ?1",
            params![connection_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    Ok(match current {
        Some(revision) => ConnectionStoreError::ConnectionRevisionConflict(
            u32::try_from(revision).unwrap_or(u32::MAX),
        )
        .into(),
        None => ConnectionStoreError::ConnectionNotFound(connection_id).into(),
    })
}
