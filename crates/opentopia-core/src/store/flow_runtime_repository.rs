use super::project_repository::touch_thread;
use super::{FlowStoreError, HumanTaskStoreError};
use crate::flow_runtime::FlowRunV1;
use crate::human_task::HumanTaskV1;
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn update_flow_run_conn(
    conn: &Connection,
    run: &FlowRunV1,
    expected_revision: u32,
) -> anyhow::Result<()> {
    let changed = conn.execute(
        r#"
        UPDATE flow_runs
        SET revision = ?2, status = ?3, document_json = ?4,
            updated_at = ?5, completed_at = ?6
        WHERE id = ?1 AND revision = ?7
        "#,
        params![
            run.id.to_string(),
            i64::from(run.revision),
            run.status.as_str(),
            serde_json::to_string(run)?,
            run.updated_at.to_rfc3339(),
            run.completed_at.map(|value| value.to_rfc3339()),
            i64::from(expected_revision),
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    let current: Option<i64> = conn
        .query_row(
            "SELECT revision FROM flow_runs WHERE id = ?1",
            params![run.id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    match current {
        Some(revision) => Err(FlowStoreError::RunRevisionConflict(
            u32::try_from(revision).unwrap_or(u32::MAX),
        )
        .into()),
        None => Err(FlowStoreError::RunNotFound(run.id).into()),
    }
}

pub(super) fn attach_human_task_to_flow_run_conn(
    conn: &Connection,
    run: &FlowRunV1,
    expected_run_revision: u32,
    task: &HumanTaskV1,
) -> anyhow::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let transition = (|| -> anyhow::Result<()> {
        update_flow_run_conn(conn, run, expected_run_revision)?;
        insert_human_task_conn(conn, task)?;
        touch_thread(conn, run.thread_id)?;
        Ok(())
    })();
    match transition {
        Ok(()) => conn.execute_batch("COMMIT;").map_err(Into::into),
        Err(error) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

pub(super) fn insert_human_task_conn(conn: &Connection, task: &HumanTaskV1) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO human_tasks (
            id, revision, thread_id, source_kind, source_id, source_node_run_id,
            task_type, status, document_json, created_at, updated_at, resolved_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            task.id.to_string(),
            i64::from(task.revision),
            task.thread_id.to_string(),
            task.source_kind.as_str(),
            task.source_id.to_string(),
            task.source_node_run_id.map(|value| value.to_string()),
            task.task_type.as_str(),
            task.status.as_str(),
            serde_json::to_string(task)?,
            task.created_at.to_rfc3339(),
            task.updated_at.to_rfc3339(),
            task.resolved_at.map(|value| value.to_rfc3339()),
        ],
    )?;
    Ok(())
}

pub(super) fn update_human_task_conn(
    conn: &Connection,
    task: &HumanTaskV1,
    expected_revision: u32,
) -> anyhow::Result<()> {
    let changed = conn.execute(
        r#"
        UPDATE human_tasks
        SET revision = ?2, status = ?3, document_json = ?4,
            updated_at = ?5, resolved_at = ?6
        WHERE id = ?1 AND revision = ?7
        "#,
        params![
            task.id.to_string(),
            i64::from(task.revision),
            task.status.as_str(),
            serde_json::to_string(task)?,
            task.updated_at.to_rfc3339(),
            task.resolved_at.map(|value| value.to_rfc3339()),
            i64::from(expected_revision),
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    let current: Option<i64> = conn
        .query_row(
            "SELECT revision FROM human_tasks WHERE id = ?1",
            params![task.id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    match current {
        Some(revision) => Err(HumanTaskStoreError::RevisionConflict(
            u32::try_from(revision).unwrap_or(u32::MAX),
        )
        .into()),
        None => Err(HumanTaskStoreError::NotFound(task.id).into()),
    }
}
