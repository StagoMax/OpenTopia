use super::agent_flow_repository::{
    insert_agent_template_version, query_agent_instance, query_agent_template_version,
    query_latest_published_agent_template,
};
use super::sqlite_codec::collect_rows;
use super::{AgentTemplateStoreError, SqliteSessionStore};
use crate::enterprise::{
    AgentInstanceStatusV1, AgentInstanceV1, AgentTemplateDiffV1, AgentTemplateError,
    AgentTemplateSpecV1, AgentTemplateStatusV1, AgentTemplateVersionV1,
};
use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, types::Type, OptionalExtension};
use serde_json::Value;
use uuid::Uuid;

impl SqliteSessionStore {
    pub fn create_agent_template_version(
        &self,
        template_id: String,
        name: String,
        owner: String,
        spec: AgentTemplateSpecV1,
    ) -> anyhow::Result<AgentTemplateVersionV1> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let existing: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT owner, archived_at FROM agent_templates WHERE template_id = ?1",
                params![&template_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((_existing_owner, Some(_))) => {
                return Err(AgentTemplateStoreError::TemplateArchived(template_id).into())
            }
            Some((existing_owner, None)) if existing_owner != owner => {
                return Err(AgentTemplateStoreError::OwnerMismatch.into())
            }
            Some(_) => {}
            None => {
                let now = Utc::now().to_rfc3339();
                tx.execute(
                    r#"
                    INSERT INTO agent_templates (
                        template_id, owner, name, created_at, archived_at
                    ) VALUES (?1, ?2, ?3, ?4, NULL)
                    "#,
                    params![&template_id, &owner, &name, now],
                )?;
            }
        }
        let next_version: i64 = tx.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM agent_template_versions WHERE template_id = ?1",
            params![&template_id],
            |row| row.get(0),
        )?;
        let next_version =
            u32::try_from(next_version).context("Agent template version overflow")?;
        let template =
            AgentTemplateVersionV1::new_draft(&template_id, next_version, &name, &owner, spec)?;
        tx.execute(
            "UPDATE agent_templates SET name = ?2 WHERE template_id = ?1",
            params![&template_id, &name],
        )?;
        insert_agent_template_version(&tx, &template)?;
        tx.commit()?;
        Ok(template)
    }

    pub fn list_agent_template_versions(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<AgentTemplateVersionV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            r#"
            SELECT agent_template_versions.document_json
            FROM agent_template_versions
            JOIN agent_templates USING (template_id)
            WHERE (?1 = 1 OR agent_templates.archived_at IS NULL)
            ORDER BY agent_template_versions.template_id ASC,
                     agent_template_versions.version DESC
            "#,
        )?;
        let rows = stmt.query_map(params![include_archived], |row| {
            let document: String = row.get(0)?;
            serde_json::from_str(&document).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })
        })?;
        collect_rows(rows)
    }

    pub fn get_agent_template_version(
        &self,
        template_id: &str,
        version: u32,
    ) -> anyhow::Result<Option<AgentTemplateVersionV1>> {
        let conn = self.read_connection();
        query_agent_template_version(&conn, template_id, version)
    }

    pub fn get_latest_published_agent_template(
        &self,
        template_id: &str,
    ) -> anyhow::Result<Option<AgentTemplateVersionV1>> {
        let conn = self.read_connection();
        query_latest_published_agent_template(&conn, template_id, None)
    }

    pub fn agent_template_is_archived(&self, template_id: &str) -> anyhow::Result<Option<bool>> {
        let conn = self.read_connection();
        conn.query_row(
            "SELECT archived_at IS NOT NULL FROM agent_templates WHERE template_id = ?1",
            params![template_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn publish_agent_template_version(
        &self,
        template_id: &str,
        version: u32,
        approved_by: &str,
        approve_capability_expansion: bool,
    ) -> anyhow::Result<(AgentTemplateVersionV1, AgentTemplateDiffV1)> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let current =
            query_agent_template_version(&tx, template_id, version)?.ok_or_else(|| {
                AgentTemplateStoreError::VersionNotFound {
                    template_id: template_id.to_string(),
                    version,
                }
            })?;
        let newer_published: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agent_template_versions WHERE template_id = ?1 AND status = 'published' AND version > ?2",
            params![template_id, i64::from(version)],
            |row| row.get(0),
        )?;
        if newer_published > 0 {
            return Err(AgentTemplateStoreError::StaleVersion.into());
        }
        let previous = query_latest_published_agent_template(&tx, template_id, Some(version))?;
        let (published, diff) =
            current.publish(approved_by, previous.as_ref(), approve_capability_expansion)?;
        let changed = tx.execute(
            r#"
            UPDATE agent_template_versions
            SET status = 'published', document_json = ?3,
                published_at = ?4, published_by = ?5
            WHERE template_id = ?1 AND version = ?2 AND status = 'draft'
            "#,
            params![
                template_id,
                i64::from(version),
                serde_json::to_string(&published)?,
                published.published_at.map(|value| value.to_rfc3339()),
                &published.published_by,
            ],
        )?;
        if changed != 1 {
            return Err(AgentTemplateError::VersionIsImmutable.into());
        }
        tx.commit()?;
        Ok((published, diff))
    }

    pub fn delete_agent_template_version(
        &self,
        template_id: &str,
        version: u32,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let status: Option<String> = tx
            .query_row(
                "SELECT status FROM agent_template_versions WHERE template_id = ?1 AND version = ?2",
                params![template_id, i64::from(version)],
                |row| row.get(0),
            )
            .optional()?;
        let Some(status) = status else {
            return Ok(false);
        };
        if status != AgentTemplateStatusV1::Draft.as_str() {
            return Err(AgentTemplateStoreError::PublishedVersionIsImmutable.into());
        }
        let instance_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agent_instances WHERE template_id = ?1 AND template_version = ?2",
            params![template_id, i64::from(version)],
            |row| row.get(0),
        )?;
        if instance_count > 0 {
            return Err(AgentTemplateStoreError::VersionInUse.into());
        }
        tx.execute(
            "DELETE FROM agent_template_versions WHERE template_id = ?1 AND version = ?2",
            params![template_id, i64::from(version)],
        )?;
        let remaining: i64 = tx.query_row(
            "SELECT COUNT(*) FROM agent_template_versions WHERE template_id = ?1",
            params![template_id],
            |row| row.get(0),
        )?;
        if remaining == 0 {
            tx.execute(
                "DELETE FROM agent_templates WHERE template_id = ?1",
                params![template_id],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn archive_agent_template(&self, template_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            "UPDATE agent_templates SET archived_at = ?2 WHERE template_id = ?1 AND archived_at IS NULL",
            params![template_id, Utc::now().to_rfc3339()],
        )?;
        Ok(changed == 1)
    }

    pub fn insert_agent_instance(&self, instance: &AgentInstanceV1) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO agent_instances (
                instance_id, template_id, template_version, thread_id,
                parent_instance_id, status, state_revision, document_json,
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            params![
                instance.id.to_string(),
                &instance.template_id,
                i64::from(instance.template_version),
                instance.thread_id.to_string(),
                instance.parent_instance_id.map(|id| id.to_string()),
                instance.status.as_str(),
                i64::try_from(instance.state_revision).context("Agent state revision overflow")?,
                serde_json::to_string(instance)?,
                instance.created_at.to_rfc3339(),
                instance.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn get_agent_instance(&self, id: Uuid) -> anyhow::Result<Option<AgentInstanceV1>> {
        let conn = self.read_connection();
        query_agent_instance(&conn, id)
    }

    pub fn list_thread_agent_instances(
        &self,
        thread_id: Uuid,
    ) -> anyhow::Result<Vec<AgentInstanceV1>> {
        let conn = self.read_connection();
        let mut stmt = conn.prepare(
            "SELECT document_json FROM agent_instances WHERE thread_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map(params![thread_id.to_string()], |row| {
            let document: String = row.get(0)?;
            serde_json::from_str(&document).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_agent_instances(
        &self,
        template_id: Option<&str>,
        status: Option<AgentInstanceStatusV1>,
        limit: u32,
    ) -> anyhow::Result<Vec<AgentInstanceV1>> {
        let conn = self.read_connection();
        let limit = i64::from(limit.clamp(1, 500));
        match (template_id, status) {
            (Some(template_id), Some(status)) => {
                let mut stmt = conn.prepare(
                    "SELECT document_json FROM agent_instances WHERE template_id = ?1 AND status = ?2 ORDER BY updated_at DESC LIMIT ?3",
                )?;
                let rows = stmt.query_map(
                    params![template_id, status.as_str(), limit],
                    deserialize_agent_instance,
                )?;
                collect_rows(rows)
            }
            (Some(template_id), None) => {
                let mut stmt = conn.prepare(
                    "SELECT document_json FROM agent_instances WHERE template_id = ?1 ORDER BY updated_at DESC LIMIT ?2",
                )?;
                let rows =
                    stmt.query_map(params![template_id, limit], deserialize_agent_instance)?;
                collect_rows(rows)
            }
            (None, Some(status)) => {
                let mut stmt = conn.prepare(
                    "SELECT document_json FROM agent_instances WHERE status = ?1 ORDER BY updated_at DESC LIMIT ?2",
                )?;
                let rows =
                    stmt.query_map(params![status.as_str(), limit], deserialize_agent_instance)?;
                collect_rows(rows)
            }
            (None, None) => {
                let mut stmt = conn.prepare(
                    "SELECT document_json FROM agent_instances ORDER BY updated_at DESC LIMIT ?1",
                )?;
                let rows = stmt.query_map(params![limit], deserialize_agent_instance)?;
                collect_rows(rows)
            }
        }
    }

    pub fn bind_thread_agent_instance(
        &self,
        thread_id: Uuid,
        instance_id: Uuid,
    ) -> anyhow::Result<AgentInstanceV1> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let instance = query_agent_instance(&tx, instance_id)?
            .ok_or(AgentTemplateStoreError::InstanceNotFound(instance_id))?;
        if instance.thread_id != thread_id {
            return Err(AgentTemplateStoreError::InstanceThreadMismatch.into());
        }
        if instance.parent_instance_id.is_some() || instance.status != AgentInstanceStatusV1::Active
        {
            return Err(AgentTemplateStoreError::InvalidThreadBinding.into());
        }
        tx.execute(
            r#"
            INSERT INTO thread_agent_bindings (thread_id, instance_id, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(thread_id) DO UPDATE SET
                instance_id = excluded.instance_id,
                updated_at = excluded.updated_at
            "#,
            params![
                thread_id.to_string(),
                instance_id.to_string(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok(instance)
    }

    pub fn get_bound_thread_agent_instance(
        &self,
        thread_id: Uuid,
    ) -> anyhow::Result<Option<AgentInstanceV1>> {
        let conn = self.read_connection();
        let document: Option<String> = conn
            .query_row(
                r#"
                SELECT agent_instances.document_json
                FROM thread_agent_bindings
                JOIN agent_instances USING (instance_id)
                WHERE thread_agent_bindings.thread_id = ?1
                "#,
                params![thread_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        document
            .map(|document| serde_json::from_str(&document).map_err(Into::into))
            .transpose()
    }

    pub fn update_agent_instance_state(
        &self,
        instance_id: Uuid,
        expected_revision: u64,
        state: Value,
    ) -> anyhow::Result<AgentInstanceV1> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let mut instance = query_agent_instance(&tx, instance_id)?
            .ok_or(AgentTemplateStoreError::InstanceNotFound(instance_id))?;
        if instance.state_revision != expected_revision {
            return Err(
                AgentTemplateStoreError::StateRevisionConflict(instance.state_revision).into(),
            );
        }
        let template =
            query_agent_template_version(&tx, &instance.template_id, instance.template_version)?
                .ok_or_else(|| AgentTemplateStoreError::VersionNotFound {
                    template_id: instance.template_id.clone(),
                    version: instance.template_version,
                })?;
        template.validate_state(&state)?;
        instance.state = state;
        instance.state_revision = instance.state_revision.saturating_add(1);
        instance.updated_at = Utc::now();
        let changed = tx.execute(
            r#"
            UPDATE agent_instances
            SET state_revision = ?2, document_json = ?3, updated_at = ?4
            WHERE instance_id = ?1 AND state_revision = ?5
            "#,
            params![
                instance_id.to_string(),
                i64::try_from(instance.state_revision).context("Agent state revision overflow")?,
                serde_json::to_string(&instance)?,
                instance.updated_at.to_rfc3339(),
                i64::try_from(expected_revision).context("Agent state revision overflow")?,
            ],
        )?;
        if changed != 1 {
            let current: i64 = tx.query_row(
                "SELECT state_revision FROM agent_instances WHERE instance_id = ?1",
                params![instance_id.to_string()],
                |row| row.get(0),
            )?;
            return Err(AgentTemplateStoreError::StateRevisionConflict(
                u64::try_from(current).unwrap_or(u64::MAX),
            )
            .into());
        }
        tx.commit()?;
        Ok(instance)
    }

    pub fn update_agent_instance_status(
        &self,
        instance_id: Uuid,
        status: AgentInstanceStatusV1,
    ) -> anyhow::Result<AgentInstanceV1> {
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        let tx = conn.transaction()?;
        let mut instance = query_agent_instance(&tx, instance_id)?
            .ok_or(AgentTemplateStoreError::InstanceNotFound(instance_id))?;
        instance.status = status;
        instance.updated_at = Utc::now();
        tx.execute(
            r#"
            UPDATE agent_instances
            SET status = ?2, document_json = ?3, updated_at = ?4
            WHERE instance_id = ?1
            "#,
            params![
                instance_id.to_string(),
                status.as_str(),
                serde_json::to_string(&instance)?,
                instance.updated_at.to_rfc3339(),
            ],
        )?;
        if status != AgentInstanceStatusV1::Active {
            tx.execute(
                "DELETE FROM thread_agent_bindings WHERE instance_id = ?1",
                params![instance_id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(instance)
    }
}

fn deserialize_agent_instance(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentInstanceV1> {
    let document: String = row.get(0)?;
    serde_json::from_str(&document)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error)))
}
