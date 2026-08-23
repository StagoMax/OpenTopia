use super::flow_runtime_repository::attach_human_task_to_flow_run_conn;
use super::goal_event_repository::upsert_work_form_conn;
use super::sqlite_codec::{collect_rows, deserialize_json_column, invalid_column};
use super::SqliteSessionStore;
use crate::flow_runtime::FlowRunV1;
use crate::human_task::HumanTaskV1;
use crate::store_migrations;
use crate::work_form::{WorkForm, WorkFormStatus, WorkItemStatus};
use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex, MutexGuard,
};

const SQLITE_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const WAL_AUTOCHECKPOINT_PAGES: i64 = 4_096;

fn open_read_connection(path: &Path) -> anyhow::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("failed to open SQLite read connection {}", path.display()))?;
    connection.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    connection.execute_batch(
        r#"
        PRAGMA query_only = ON;
        PRAGMA foreign_keys = ON;
        "#,
    )?;
    Ok(connection)
}

struct CanonicalSchemaManifests {
    legacy: store_migrations::SchemaManifest,
    current: store_migrations::SchemaManifest,
}

fn canonical_schema_manifests() -> anyhow::Result<CanonicalSchemaManifests> {
    let reference = SqliteSessionStore {
        conn: Mutex::new(Connection::open_in_memory()?),
        read_connections: Vec::new(),
        next_read_connection: AtomicUsize::new(0),
    };
    reference.migrate_legacy_database()?;
    let mut conn = reference.conn.lock().expect("sqlite mutex poisoned");
    let legacy = store_migrations::inspect_schema(&conn)?;
    store_migrations::validate_frozen_legacy_manifest(&legacy)?;
    store_migrations::initialize_legacy_baseline(&mut conn)?;
    store_migrations::apply_pending_migrations(&mut conn)?;
    let current = store_migrations::inspect_schema(&conn)?;
    Ok(CanonicalSchemaManifests { legacy, current })
}

pub(super) fn recover_interrupted_runtime_state(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        r#"
        UPDATE flow_runs
        SET status = 'paused',
            revision = revision + 1,
            document_json = json_set(
                document_json,
                '$.status', 'paused',
                '$.error', 'server restarted at a node boundary; resume after inspection',
                '$.revision', revision + 1,
                '$.updatedAt', ?1
            ),
            updated_at = ?1
        WHERE status IN ('queued', 'running', 'pause_requested')
        "#,
        params![Utc::now().to_rfc3339()],
    )?;
    let has_human_tasks = conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'human_tasks'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if has_human_tasks {
        let interrupted_runs = {
            let mut stmt = conn.prepare(
                r#"
                SELECT document_json FROM flow_runs
                WHERE status = 'paused'
                  AND json_extract(document_json, '$.error') =
                      'server restarted at a node boundary; resume after inspection'
                "#,
            )?;
            let runs = collect_rows(stmt.query_map([], deserialize_json_column::<FlowRunV1>)?)?;
            runs
        };
        for mut run in interrupted_runs {
            if run.active_human_task_id.is_some() {
                continue;
            }
            let Some((node_run_id, node_id)) = run
                .node_runs
                .iter()
                .rev()
                .find(|node| {
                    matches!(
                        node.status,
                        crate::flow_runtime::FlowNodeRunStatusV1::Running
                            | crate::flow_runtime::FlowNodeRunStatusV1::Resuming
                    )
                })
                .map(|node| (node.id, node.node_id.clone()))
            else {
                continue;
            };
            let expected_run_revision = run.revision;
            let checkpoint_payload = run.active_checkpoint.as_ref().map(|checkpoint| {
                serde_json::json!({
                    "checkpointId": checkpoint.id,
                    "superstep": checkpoint.superstep,
                    "nodeIds": checkpoint.nodes.iter().map(|node| node.node_id.clone()).collect::<Vec<_>>(),
                    "completedPendingWrites": checkpoint.pending_writes.iter().filter(|write| write.result.is_some()).count(),
                    "pendingWriteCount": checkpoint.pending_writes.len(),
                })
            });
            let task = HumanTaskV1::flow_recovery(
                run.thread_id,
                run.id,
                Some(node_run_id),
                Some(node_id.clone()),
                node_run_id.to_string(),
                "服务在节点执行期间中断。请先核对外部系统是否已产生副作用，再决定重试或取消。",
                serde_json::json!({
                    "flowId": run.flow_id,
                    "flowVersion": run.flow_version,
                    "nodeId": node_id,
                    "sideEffectState": "unknown",
                    "resumeCommandId": run.pending_resume_command_id(),
                    "resumableContinuation": run.pending_resume_command_id().is_some(),
                    "checkpoint": checkpoint_payload,
                }),
            );
            run.active_human_task_id = Some(task.id);
            run.touch();
            attach_human_task_to_flow_run_conn(conn, &run, expected_run_revision, &task)?;
        }
        let legacy_approval_runs = {
            let mut stmt = conn
                .prepare("SELECT document_json FROM flow_runs WHERE status = 'waiting_approval'")?;
            let runs = collect_rows(stmt.query_map([], deserialize_json_column::<FlowRunV1>)?)?;
            runs
        };
        for mut run in legacy_approval_runs {
            if run.active_human_task_id.is_some() {
                continue;
            }
            let Some(node_run) = run
                .node_runs
                .iter()
                .rev()
                .find(|node| {
                    node.status == crate::flow_runtime::FlowNodeRunStatusV1::WaitingApproval
                })
                .cloned()
            else {
                continue;
            };
            let node_label = run
                .graph
                .nodes
                .iter()
                .find(|node| node.id == node_run.node_id)
                .map(|node| node.label.as_str())
                .unwrap_or(node_run.node_id.as_str());
            let expected_run_revision = run.revision;
            let task = HumanTaskV1::flow_approval(
                run.thread_id,
                run.id,
                node_run.id,
                node_run.node_id.clone(),
                node_label,
                serde_json::json!({
                    "flowId": run.flow_id,
                    "flowVersion": run.flow_version,
                    "nodeId": node_run.node_id,
                    "nodeLabel": node_label,
                    "input": node_run.input,
                }),
            );
            run.active_human_task_id = Some(task.id);
            run.touch();
            attach_human_task_to_flow_run_conn(conn, &run, expected_run_revision, &task)?;
        }
    }
    let recoverable_forms = {
        let mut stmt =
            conn.prepare("SELECT form_json FROM work_forms WHERE scope_kind = 'goal'")?;
        let forms = collect_rows(stmt.query_map([], |row| {
            let raw: String = row.get(0)?;
            serde_json::from_str::<WorkForm>(&raw)
                .map_err(|error| invalid_column(0, error.to_string()))
        })?)?;
        forms
    };
    for mut form in recoverable_forms {
        let mut interrupted = false;
        for item in &mut form.items {
            if item.status == WorkItemStatus::InProgress {
                item.status = WorkItemStatus::Blocked;
                item.note = Some("server restarted during task execution".to_string());
                interrupted = true;
            }
        }
        if !interrupted {
            continue;
        }
        form.status = WorkFormStatus::Blocked;
        form.updated_at = Utc::now();
        upsert_work_form_conn(conn, &form)?;
    }
    Ok(())
}

impl SqliteSessionStore {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if path != Path::new(":memory:") {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
        }

        let conn = Connection::open(path)
            .with_context(|| format!("failed to open sqlite db {}", path.display()))?;
        conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
        let mut store = Self {
            conn: Mutex::new(conn),
            read_connections: Vec::new(),
            next_read_connection: AtomicUsize::new(0),
        };
        store.migrate()?;
        if path != Path::new(":memory:") {
            {
                let conn = store.conn.lock().expect("sqlite mutex poisoned");
                // Keep automatic checkpoints off the high-frequency 4 MiB
                // default cadence. A larger passive window reduces how often
                // event commits inherit checkpoint work without weakening the
                // database's existing FULL durability policy.
                conn.pragma_update(None, "wal_autocheckpoint", WAL_AUTOCHECKPOINT_PAGES)?;
            }
            store.read_connections = (0..4)
                .map(|_| open_read_connection(path))
                .collect::<anyhow::Result<Vec<_>>>()?
                .into_iter()
                .map(Mutex::new)
                .collect();
        }
        Ok(store)
    }

    pub(super) fn read_connection(&self) -> MutexGuard<'_, Connection> {
        if self.read_connections.is_empty() {
            return self.conn.lock().expect("sqlite mutex poisoned");
        }
        let index =
            self.next_read_connection.fetch_add(1, Ordering::Relaxed) % self.read_connections.len();
        self.read_connections[index]
            .lock()
            .expect("sqlite read mutex poisoned")
    }

    pub(crate) fn with_collaboration_read<T>(
        &self,
        operation: impl FnOnce(&Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let connection = self.read_connection();
        operation(&connection)
    }

    pub(crate) fn with_collaboration_write<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let mut connection = self.conn.lock().expect("sqlite mutex poisoned");
        operation(&mut connection)
    }

    pub(super) fn migrate(&self) -> anyhow::Result<()> {
        let has_ledger = {
            let conn = self.conn.lock().expect("sqlite mutex poisoned");
            let has_ledger = store_migrations::has_migration_ledger(&conn)?;
            if has_ledger {
                store_migrations::validate_managed_database_before_migration(&conn)?;
            } else {
                store_migrations::preflight_unmanaged_database(&conn)?;
            }
            has_ledger
        };

        if !has_ledger {
            self.migrate_legacy_database()?;
        }

        let expected_schemas = canonical_schema_manifests()?;
        let mut conn = self.conn.lock().expect("sqlite mutex poisoned");
        if !has_ledger {
            store_migrations::validate_schema_manifest(&conn, &expected_schemas.legacy)?;
            store_migrations::validate_database_integrity(&conn)?;
            store_migrations::initialize_legacy_baseline(&mut conn)?;
        }
        store_migrations::apply_pending_migrations(&mut conn)?;
        store_migrations::verify_current_database(&conn, &expected_schemas.current)?;
        recover_interrupted_runtime_state(&conn)?;
        Ok(())
    }
}
