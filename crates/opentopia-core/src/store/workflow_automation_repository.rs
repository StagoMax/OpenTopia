use super::{
    collect_rows, deserialize_json_column, SqliteSessionStore, WorkflowAutomationStoreError,
};
use crate::workflow_automation::{
    WorkflowDeliveryReceiptV1, WorkflowDeliveryStatusV1, WorkflowEvaluationV1,
    WorkflowReleaseStatusV1, WorkflowReleaseV1, WorkflowTriggerInvocationV1,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

impl SqliteSessionStore {
    pub fn insert_workflow_release(
        &self,
        release: &WorkflowReleaseV1,
    ) -> anyhow::Result<WorkflowReleaseV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO workflow_releases (
                id, revision, release_key, environment, thread_id, status,
                trigger_id, trigger_kind, primary_deployment_id,
                canary_deployment_id, document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                release.id.to_string(),
                i64::from(release.revision),
                &release.release_key,
                &release.environment,
                release.thread_id.to_string(),
                release.status.as_str(),
                release.trigger.trigger_id().map(|id| id.to_string()),
                release.trigger.kind_name(),
                release.primary_deployment_id.to_string(),
                release.canary_deployment_id.map(|id| id.to_string()),
                serde_json::to_string(release)?,
                release.created_at.to_rfc3339(),
                release.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(release.clone())
    }

    pub fn get_workflow_release(
        &self,
        release_id: Uuid,
    ) -> anyhow::Result<Option<WorkflowReleaseV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_releases WHERE id = ?1",
                params![release_id.to_string()],
                deserialize_json_column::<WorkflowReleaseV1>,
            )
            .optional()?)
    }

    pub fn get_workflow_release_by_trigger(
        &self,
        trigger_id: Uuid,
    ) -> anyhow::Result<Option<WorkflowReleaseV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_releases WHERE trigger_id = ?1",
                params![trigger_id.to_string()],
                deserialize_json_column::<WorkflowReleaseV1>,
            )
            .optional()?)
    }

    pub fn list_workflow_releases(
        &self,
        status: Option<WorkflowReleaseStatusV1>,
    ) -> anyhow::Result<Vec<WorkflowReleaseV1>> {
        let conn = self.read_connection();
        let mut statement = match status {
            Some(_) => conn.prepare(
                "SELECT document_json FROM workflow_releases WHERE status = ?1 ORDER BY updated_at DESC",
            )?,
            None => conn.prepare(
                "SELECT document_json FROM workflow_releases ORDER BY updated_at DESC",
            )?,
        };
        let rows = match status {
            Some(status) => statement.query_map(
                params![status.as_str()],
                deserialize_json_column::<WorkflowReleaseV1>,
            )?,
            None => statement.query_map([], deserialize_json_column::<WorkflowReleaseV1>)?,
        };
        collect_rows(rows)
    }

    pub fn update_workflow_release(
        &self,
        release: &WorkflowReleaseV1,
        expected_revision: u32,
    ) -> anyhow::Result<WorkflowReleaseV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE workflow_releases
            SET revision = ?2, status = ?3, trigger_id = ?4, trigger_kind = ?5,
                primary_deployment_id = ?6, canary_deployment_id = ?7,
                document_json = ?8, updated_at = ?9
            WHERE id = ?1 AND revision = ?10
            "#,
            params![
                release.id.to_string(),
                i64::from(release.revision),
                release.status.as_str(),
                release.trigger.trigger_id().map(|id| id.to_string()),
                release.trigger.kind_name(),
                release.primary_deployment_id.to_string(),
                release.canary_deployment_id.map(|id| id.to_string()),
                serde_json::to_string(release)?,
                release.updated_at.to_rfc3339(),
                i64::from(expected_revision),
            ],
        )?;
        if changed == 0 {
            let current = conn
                .query_row(
                    "SELECT revision FROM workflow_releases WHERE id = ?1",
                    params![release.id.to_string()],
                    |row| row.get::<_, u32>(0),
                )
                .optional()?;
            return Err(match current {
                Some(revision) => {
                    WorkflowAutomationStoreError::ReleaseRevisionConflict(revision).into()
                }
                None => WorkflowAutomationStoreError::ReleaseNotFound(release.id).into(),
            });
        }
        Ok(release.clone())
    }

    pub fn insert_workflow_trigger_invocation(
        &self,
        invocation: &WorkflowTriggerInvocationV1,
    ) -> anyhow::Result<WorkflowTriggerInvocationV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO workflow_trigger_invocations (
                id, release_id, trigger_id, idempotency_key, deployment_id,
                flow_run_id, status, input_hash, document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            params![
                invocation.id.to_string(),
                invocation.release_id.to_string(),
                invocation.trigger_id.to_string(),
                &invocation.idempotency_key,
                invocation.deployment_id.to_string(),
                invocation.flow_run_id.map(|id| id.to_string()),
                invocation.status.as_str(),
                &invocation.input_hash,
                serde_json::to_string(invocation)?,
                invocation.created_at.to_rfc3339(),
                invocation.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(invocation.clone())
    }

    pub fn get_workflow_trigger_invocation(
        &self,
        release_id: Uuid,
        idempotency_key: &str,
    ) -> anyhow::Result<Option<WorkflowTriggerInvocationV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_trigger_invocations WHERE release_id = ?1 AND idempotency_key = ?2",
                params![release_id.to_string(), idempotency_key],
                deserialize_json_column::<WorkflowTriggerInvocationV1>,
            )
            .optional()?)
    }

    pub fn get_workflow_trigger_invocation_by_id(
        &self,
        invocation_id: Uuid,
    ) -> anyhow::Result<Option<WorkflowTriggerInvocationV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_trigger_invocations WHERE id = ?1",
                params![invocation_id.to_string()],
                deserialize_json_column::<WorkflowTriggerInvocationV1>,
            )
            .optional()?)
    }

    pub fn list_workflow_trigger_invocations(
        &self,
        release_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<WorkflowTriggerInvocationV1>> {
        let conn = self.read_connection();
        let mut statement = match release_id {
            Some(_) => conn.prepare("SELECT document_json FROM workflow_trigger_invocations WHERE release_id = ?1 ORDER BY updated_at DESC LIMIT 500")?,
            None => conn.prepare("SELECT document_json FROM workflow_trigger_invocations ORDER BY updated_at DESC LIMIT 500")?,
        };
        let rows = match release_id {
            Some(id) => statement.query_map(
                params![id.to_string()],
                deserialize_json_column::<WorkflowTriggerInvocationV1>,
            )?,
            None => {
                statement.query_map([], deserialize_json_column::<WorkflowTriggerInvocationV1>)?
            }
        };
        collect_rows(rows)
    }

    pub fn count_recent_workflow_trigger_invocations(
        &self,
        trigger_id: Uuid,
        since: DateTime<Utc>,
    ) -> anyhow::Result<u32> {
        let conn = self.read_connection();
        let count = conn.query_row(
            "SELECT COUNT(*) FROM workflow_trigger_invocations WHERE trigger_id = ?1 AND created_at >= ?2",
            params![trigger_id.to_string(), since.to_rfc3339()],
            |row| row.get::<_, u32>(0),
        )?;
        Ok(count)
    }

    pub fn update_workflow_trigger_invocation(
        &self,
        invocation: &WorkflowTriggerInvocationV1,
    ) -> anyhow::Result<WorkflowTriggerInvocationV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE workflow_trigger_invocations
            SET flow_run_id = ?2, status = ?3, document_json = ?4, updated_at = ?5
            WHERE id = ?1
            "#,
            params![
                invocation.id.to_string(),
                invocation.flow_run_id.map(|id| id.to_string()),
                invocation.status.as_str(),
                serde_json::to_string(invocation)?,
                invocation.updated_at.to_rfc3339(),
            ],
        )?;
        anyhow::ensure!(changed == 1, "workflow trigger invocation not found");
        Ok(invocation.clone())
    }

    pub fn insert_workflow_delivery_receipt(
        &self,
        receipt: &WorkflowDeliveryReceiptV1,
    ) -> anyhow::Result<WorkflowDeliveryReceiptV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO workflow_delivery_receipts (
                id, revision, run_id, deployment_id, output_kind, status,
                document_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                receipt.id.to_string(),
                i64::from(receipt.revision),
                receipt.run_id.to_string(),
                receipt.deployment_id.to_string(),
                &receipt.output_kind,
                receipt.status.as_str(),
                serde_json::to_string(receipt)?,
                receipt.created_at.to_rfc3339(),
                receipt.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(receipt.clone())
    }

    pub fn get_workflow_delivery_receipt(
        &self,
        receipt_id: Uuid,
    ) -> anyhow::Result<Option<WorkflowDeliveryReceiptV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_delivery_receipts WHERE id = ?1",
                params![receipt_id.to_string()],
                deserialize_json_column::<WorkflowDeliveryReceiptV1>,
            )
            .optional()?)
    }

    pub fn get_workflow_delivery_receipt_for_run(
        &self,
        run_id: Uuid,
    ) -> anyhow::Result<Option<WorkflowDeliveryReceiptV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_delivery_receipts WHERE run_id = ?1",
                params![run_id.to_string()],
                deserialize_json_column::<WorkflowDeliveryReceiptV1>,
            )
            .optional()?)
    }

    pub fn list_workflow_delivery_receipts(
        &self,
        deployment_id: Option<Uuid>,
        status: Option<WorkflowDeliveryStatusV1>,
    ) -> anyhow::Result<Vec<WorkflowDeliveryReceiptV1>> {
        let conn = self.read_connection();
        let (sql, values): (&str, Vec<String>) = match (deployment_id, status) {
            (Some(deployment), Some(status)) => (
                "SELECT document_json FROM workflow_delivery_receipts WHERE deployment_id = ?1 AND status = ?2 ORDER BY updated_at DESC LIMIT 500",
                vec![deployment.to_string(), status.as_str().to_string()],
            ),
            (Some(deployment), None) => (
                "SELECT document_json FROM workflow_delivery_receipts WHERE deployment_id = ?1 ORDER BY updated_at DESC LIMIT 500",
                vec![deployment.to_string()],
            ),
            (None, Some(status)) => (
                "SELECT document_json FROM workflow_delivery_receipts WHERE status = ?1 ORDER BY updated_at DESC LIMIT 500",
                vec![status.as_str().to_string()],
            ),
            (None, None) => (
                "SELECT document_json FROM workflow_delivery_receipts ORDER BY updated_at DESC LIMIT 500",
                Vec::new(),
            ),
        };
        let mut statement = conn.prepare(sql)?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(values),
            deserialize_json_column::<WorkflowDeliveryReceiptV1>,
        )?;
        collect_rows(rows)
    }

    pub fn update_workflow_delivery_receipt(
        &self,
        receipt: &WorkflowDeliveryReceiptV1,
        expected_revision: u32,
    ) -> anyhow::Result<WorkflowDeliveryReceiptV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        let changed = conn.execute(
            r#"
            UPDATE workflow_delivery_receipts
            SET revision = ?2, status = ?3, document_json = ?4, updated_at = ?5
            WHERE id = ?1 AND revision = ?6
            "#,
            params![
                receipt.id.to_string(),
                i64::from(receipt.revision),
                receipt.status.as_str(),
                serde_json::to_string(receipt)?,
                receipt.updated_at.to_rfc3339(),
                i64::from(expected_revision),
            ],
        )?;
        if changed == 0 {
            let current = conn
                .query_row(
                    "SELECT revision FROM workflow_delivery_receipts WHERE id = ?1",
                    params![receipt.id.to_string()],
                    |row| row.get::<_, u32>(0),
                )
                .optional()?;
            return Err(match current {
                Some(revision) => {
                    WorkflowAutomationStoreError::DeliveryReceiptRevisionConflict(revision).into()
                }
                None => WorkflowAutomationStoreError::DeliveryReceiptNotFound(receipt.id).into(),
            });
        }
        Ok(receipt.clone())
    }

    pub fn insert_workflow_evaluation(
        &self,
        evaluation: &WorkflowEvaluationV1,
    ) -> anyhow::Result<WorkflowEvaluationV1> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        conn.execute(
            r#"
            INSERT INTO workflow_evaluations (
                id, run_id, deployment_id, evaluator, passed, score,
                document_json, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                evaluation.id.to_string(),
                evaluation.run_id.to_string(),
                evaluation.deployment_id.to_string(),
                &evaluation.evaluator,
                evaluation.passed,
                evaluation.score,
                serde_json::to_string(evaluation)?,
                evaluation.created_at.to_rfc3339(),
            ],
        )?;
        Ok(evaluation.clone())
    }

    pub fn get_workflow_evaluation(
        &self,
        run_id: Uuid,
        evaluator: &str,
    ) -> anyhow::Result<Option<WorkflowEvaluationV1>> {
        let conn = self.read_connection();
        Ok(conn
            .query_row(
                "SELECT document_json FROM workflow_evaluations WHERE run_id = ?1 AND evaluator = ?2",
                params![run_id.to_string(), evaluator],
                deserialize_json_column::<WorkflowEvaluationV1>,
            )
            .optional()?)
    }

    pub fn list_workflow_evaluations(
        &self,
        deployment_id: Option<Uuid>,
    ) -> anyhow::Result<Vec<WorkflowEvaluationV1>> {
        let conn = self.read_connection();
        let mut statement = match deployment_id {
            Some(_) => conn.prepare("SELECT document_json FROM workflow_evaluations WHERE deployment_id = ?1 ORDER BY created_at DESC LIMIT 1000")?,
            None => conn.prepare("SELECT document_json FROM workflow_evaluations ORDER BY created_at DESC LIMIT 1000")?,
        };
        let rows = match deployment_id {
            Some(id) => statement.query_map(
                params![id.to_string()],
                deserialize_json_column::<WorkflowEvaluationV1>,
            )?,
            None => statement.query_map([], deserialize_json_column::<WorkflowEvaluationV1>)?,
        };
        collect_rows(rows)
    }
}
