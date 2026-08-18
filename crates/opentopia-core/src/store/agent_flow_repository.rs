use crate::enterprise::{AgentInstanceV1, AgentTemplateVersionV1};
use crate::flow::FlowDraftV1;
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

pub(super) fn insert_agent_template_version(
    conn: &Connection,
    template: &AgentTemplateVersionV1,
) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO agent_template_versions (
            template_id, version, status, content_hash, document_json,
            created_at, published_at, published_by
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            &template.template_id,
            i64::from(template.version),
            template.status.as_str(),
            &template.content_hash,
            serde_json::to_string(template)?,
            template.created_at.to_rfc3339(),
            template.published_at.map(|value| value.to_rfc3339()),
            &template.published_by,
        ],
    )?;
    Ok(())
}

pub(super) fn query_agent_template_version(
    conn: &Connection,
    template_id: &str,
    version: u32,
) -> anyhow::Result<Option<AgentTemplateVersionV1>> {
    let document: Option<String> = conn
        .query_row(
            "SELECT document_json FROM agent_template_versions WHERE template_id = ?1 AND version = ?2",
            params![template_id, i64::from(version)],
            |row| row.get(0),
        )
        .optional()?;
    document
        .map(|document| serde_json::from_str(&document).map_err(Into::into))
        .transpose()
}

pub(super) fn query_latest_published_agent_template(
    conn: &Connection,
    template_id: &str,
    before_version: Option<u32>,
) -> anyhow::Result<Option<AgentTemplateVersionV1>> {
    let document: Option<String> = conn
        .query_row(
            r#"
            SELECT document_json
            FROM agent_template_versions
            WHERE template_id = ?1 AND status = 'published'
              AND (?2 IS NULL OR version < ?2)
            ORDER BY version DESC
            LIMIT 1
            "#,
            params![template_id, before_version.map(i64::from)],
            |row| row.get(0),
        )
        .optional()?;
    document
        .map(|document| serde_json::from_str(&document).map_err(Into::into))
        .transpose()
}

pub(super) fn query_agent_instance(
    conn: &Connection,
    id: Uuid,
) -> anyhow::Result<Option<AgentInstanceV1>> {
    let document: Option<String> = conn
        .query_row(
            "SELECT document_json FROM agent_instances WHERE instance_id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    document
        .map(|document| serde_json::from_str(&document).map_err(Into::into))
        .transpose()
}

pub(super) fn query_flow_draft(conn: &Connection, id: Uuid) -> anyhow::Result<Option<FlowDraftV1>> {
    let document: Option<String> = conn
        .query_row(
            "SELECT document_json FROM flow_drafts WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    document
        .map(|document| serde_json::from_str(&document).map_err(Into::into))
        .transpose()
}
